//! Amphetamine-style "keep awake" sessions.
//!
//! Prevents the machine from idle-sleeping for a chosen duration (or
//! indefinitely), using each OS's native mechanism:
//!
//! * **macOS**   - an IOKit power assertion (`IOPMAssertionCreateWithName`),
//!   the same public API the `caffeinate` tool wraps. Held in-process so
//!   it works inside the App Sandbox (no subprocess to spawn). The
//!   non-App-Store build can additionally keep the Mac awake with the lid
//!   closed by setting the system `disablesleep` flag via `pmset` behind
//!   the OS admin prompt; see `docs/lid-closed-keep-awake.md`.
//! * **Windows** - `SetThreadExecutionState` (a single Win32 call).
//! * **Linux**   - spawns `systemd-inhibit`, which holds an idle
//!   inhibitor for as long as its child process is alive.
//!
//! No extra dependency on any platform. The cross-platform session
//! bookkeeping (timer, display-sleep flag) lives here; the small
//! platform module below is the only OS-specific part.

use crate::clamshell::ClamshellStatus;
use std::time::{Duration, Instant};

/// A running (or stopped) keep-awake session.
pub struct KeepAwake {
    /// The live OS wake lock; `None` while idle. Dropping it releases.
    backend: Option<platform::Backend>,
    /// When a timed session ends; `None` while idle or indefinite.
    deadline: Option<Instant>,
    /// If true, the display may still sleep while the system stays awake.
    allow_display_sleep: bool,
    /// If true, sessions also keep the machine awake with the lid closed
    /// (macOS non-App-Store only). Ignored where unsupported.
    lid_closed: bool,
    /// Set when a lid-closed session was requested but could not actually
    /// engage: the system refused the `disablesleep` change (policy on a
    /// managed Mac) or the admin prompt was declined. Lets the UI say so
    /// instead of silently doing nothing, or worse, looking enabled.
    lid_closed_blocked: bool,
}

/// Snapshot of the session for rendering the menu.
pub struct AwakeView {
    pub active: bool,
    /// Human label for the active session, e.g. "Awake - 29m left".
    pub status: Option<String>,
    pub allow_display_sleep: bool,
    /// Whether new sessions keep the Mac awake with the lid closed.
    pub lid_closed: bool,
    /// Whether this build/platform can honor `lid_closed` at all. False
    /// everywhere except the non-App-Store macOS build, so the menu can
    /// hide the toggle where it would do nothing.
    pub lid_closed_supported: bool,
    /// Passwordless-helper state, for the "Enable passwordless mode" menu
    /// entries. `KeepAwake` doesn't own the helper, so `view` leaves these
    /// at their defaults and the composition root fills them in.
    /// `helper_installable` is true only when setup is actually offerable
    /// (macOS 13+, not yet registered), so the menu stays clean on older
    /// systems where only the admin-prompt path exists.
    pub helper_active: bool,
    pub helper_needs_approval: bool,
    pub helper_installable: bool,
    /// True when the last lid-closed request could not be applied (blocked
    /// by policy or declined), so the menu can say so honestly.
    pub lid_closed_blocked: bool,
    /// What happens if the lid closes now (external display / power).
    /// Sandbox-safe and shown in every build; filled in by the
    /// composition root, so `view` defaults it to `Hidden`.
    pub clamshell: ClamshellStatus,
}

impl KeepAwake {
    pub fn new() -> Self {
        Self {
            backend: None,
            deadline: None,
            allow_display_sleep: false,
            lid_closed: false,
            lid_closed_blocked: false,
        }
    }

    pub fn allow_display_sleep(&self) -> bool {
        self.allow_display_sleep
    }

    pub fn lid_closed(&self) -> bool {
        self.lid_closed
    }

    /// Flip the lid-closed preference. Restarts an active session so the
    /// new mode takes effect immediately: turning it on prompts for an
    /// admin password (the OS sets the `disablesleep` flag), turning it
    /// off prompts once more to restore normal sleep. A no-op where the
    /// feature is unsupported.
    pub fn set_lid_closed(&mut self, on: bool) {
        if !platform::LID_CLOSED_SUPPORTED || self.lid_closed == on {
            return;
        }
        self.lid_closed = on;
        if self.is_active() {
            let remaining = self.remaining();
            self.start(remaining);
        }
    }

    /// Flip the display-sleep preference. Restarts an active session so
    /// the new flag takes effect immediately.
    pub fn set_allow_display_sleep(&mut self, allow: bool) {
        if self.allow_display_sleep == allow {
            return;
        }
        self.allow_display_sleep = allow;
        if self.is_active() {
            let remaining = self.remaining();
            self.start(remaining);
        }
    }

    /// Begin a session. `duration == None` means indefinitely.
    pub fn start(&mut self, duration: Option<Duration>) {
        self.end();
        self.deadline = duration.map(|d| Instant::now() + d);
        self.backend =
            platform::Backend::engage(duration, self.allow_display_sleep, self.lid_closed);
        if self.backend.is_none() {
            // Engaging the OS lock failed; don't pretend we're awake.
            self.deadline = None;
            // If this was a lid-closed request, the failure is the system
            // refusing the `disablesleep` change (or a declined prompt), not
            // a plain assertion failure. Remember it so the menu can be
            // honest rather than showing an enabled session that isn't real.
            self.lid_closed_blocked = self.lid_closed;
        }
    }

    /// End any active session immediately.
    pub fn end(&mut self) {
        // Dropping the backend releases the OS wake lock.
        self.backend = None;
        self.deadline = None;
        self.lid_closed_blocked = false;
    }

    /// Reap a session whose timer elapsed (or whose helper process
    /// exited). Returns true if the active state changed, so the caller
    /// can refresh the menu.
    pub fn poll(&mut self) -> bool {
        if self.backend.is_none() {
            return false;
        }
        // A helper-process backend finished on its own (Linux's
        // `systemd-inhibit sleep`); macOS/Windows report false here and
        // are ended by the deadline below.
        if self.backend.as_mut().is_some_and(|b| b.finished()) {
            self.end();
            return true;
        }
        // Or our own timer elapsed (covers backends without a built-in
        // timer, like the Windows execution-state flag).
        if let Some(deadline) = self.deadline {
            if Instant::now() >= deadline {
                self.end();
                return true;
            }
        }
        false
    }

    fn is_active(&self) -> bool {
        self.backend.is_some()
    }

    /// Time left in a timed session; `None` when idle or indefinite.
    fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|d| d.saturating_duration_since(Instant::now()))
    }

    pub fn view(&self) -> AwakeView {
        let status = self.is_active().then(|| {
            let base = match self.remaining() {
                Some(left) => format!("Awake - {} left", fmt_remaining(left)),
                None => "Awake - indefinitely".to_string(),
            };
            // Surface lid-closed mode in the status so the user can tell,
            // at a glance, that this session survives closing the lid.
            if self.lid_closed {
                format!("{base} - lid closed")
            } else {
                base
            }
        });
        AwakeView {
            active: self.is_active(),
            status,
            allow_display_sleep: self.allow_display_sleep,
            lid_closed: self.lid_closed,
            lid_closed_supported: platform::LID_CLOSED_SUPPORTED,
            lid_closed_blocked: self.lid_closed_blocked,
            // Filled in by the composition root, which owns helper state.
            helper_active: false,
            helper_needs_approval: false,
            helper_installable: false,
            clamshell: ClamshellStatus::Hidden,
        }
    }
}

/// Restore normal sleep if a previous "keep awake with lid closed"
/// session left the system `disablesleep` flag set after an unclean exit
/// (crash, force-quit, power loss). Call once at startup. No-op except on
/// the non-App-Store macOS build, and even there it only prompts when the
/// flag is genuinely still set.
pub fn recover_lid_closed() {
    platform::recover_lid_closed();
}

/// Compact "1h05m" / "9m" / "<1m" label for the remaining time.
fn fmt_remaining(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        "<1m".to_string()
    }
}

// ── Platform backends ────────────────────────────────────────────────
// Each `Backend` holds the OS wake lock; `Drop` releases it. `engage`
// returns `None` if the lock could not be acquired. `finished` reports
// whether the OS released the lock on its own (timed helper exited).

/// macOS: keep awake via an IOKit power-management assertion. This is the
/// public API that `caffeinate` itself wraps, called in-process so the
/// idle keep-awake works inside the App Sandbox with no subprocess and no
/// entitlements. The assertion is held for the life of the `Backend` and
/// released on `Drop`. There is no OS-side timer (the simple assertion API
/// has none), so timed sessions are ended by `KeepAwake::poll` via the
/// deadline, exactly like the Windows backend.
///
/// Lid-closed mode is layered on top in the non-App-Store build only: an
/// assertion cannot override an explicit lid-close sleep, so the system
/// `disablesleep` flag is set via `pmset` behind the OS admin prompt (see
/// `set_disablesleep`). That path does spawn a subprocess and needs root,
/// which is exactly why it is unavailable under the sandbox.
#[cfg(target_os = "macos")]
mod platform {
    use super::Duration;
    use std::ffi::c_void;

    /// A live keep-awake lock: the IOKit assertion, plus whether this
    /// session also set the system `disablesleep` flag (lid-closed mode)
    /// so `Drop` knows to restore it.
    pub struct Backend {
        assertion: u32,
        disabled_sleep: bool,
    }

    /// Only the non-App-Store build can honor lid-closed mode: it needs
    /// to run `pmset` as root, which the App Sandbox forbids. In the
    /// sandboxed (`mas`) build this is `false` and the whole lid-closed
    /// path compiles to dead runtime branches.
    pub const LID_CLOSED_SUPPORTED: bool = !cfg!(feature = "mas");

    // kIOPMAssertionLevelOn.
    const ASSERTION_LEVEL_ON: u32 = 255;
    // kCFStringEncodingUTF8.
    const UTF8: u32 = 0x0800_0100;
    // kIOReturnSuccess.
    const IO_SUCCESS: i32 = 0;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            num_bytes: isize,
            encoding: u32,
            is_external_representation: u8,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: *const c_void,
            assertion_level: u32,
            assertion_name: *const c_void,
            assertion_id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(assertion_id: u32) -> i32;
    }

    /// Build a CFString from a Rust `&str`. Caller must `CFRelease` it.
    /// Returns null on failure.
    fn cfstr(s: &str) -> *const c_void {
        // SAFETY: valid pointer + length; UTF-8 is a supported encoding.
        unsafe {
            CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0)
        }
    }

    /// Flip the system-wide `disablesleep` power flag. Runs `pmset` as
    /// root, the only lever that keeps the machine awake when the lid is
    /// physically closed (a lid close is an explicit sleep request that no
    /// power assertion overrides). Returns whether the change applied.
    ///
    /// Prefers the installed root helper (Option B): when it is registered,
    /// approved, and listening, the toggle is passwordless. Otherwise falls
    /// back to a one-off admin prompt (Option A), so the feature still
    /// works before, or without ever, setting the helper up.
    fn set_disablesleep(on: bool) -> bool {
        // Attempt the change: the passwordless helper first (when present),
        // else the admin prompt.
        #[cfg(not(feature = "mas"))]
        {
            if !crate::helper::set_disablesleep(on) {
                set_disablesleep_prompt(on);
            }
        }
        #[cfg(feature = "mas")]
        {
            set_disablesleep_prompt(on);
        }
        // Trust the real system state, not the exit code. A managed Mac can
        // accept the admin auth (or restrict `osascript`'s privileged exec)
        // so that `pmset` reports success yet `disablesleep` never actually
        // changes. Report success only if the flag really flipped, so the
        // caller never claims a lid-closed session that isn't in effect.
        sleep_currently_disabled() == on
    }

    /// Option A path: flip the flag through the standard macOS
    /// admin-authentication dialog. `osascript ... with administrator
    /// privileges` shows the OS's own password prompt: the user
    /// authenticates to the system, not to klipa, and we never see or
    /// handle the password.
    fn set_disablesleep_prompt(on: bool) -> bool {
        let val = if on { "1" } else { "0" };
        // `-a`: apply on both battery and charger, so the feature is not
        // silently a no-op on battery. Heat/battery cost is surfaced in
        // the UI/docs, mirroring how Amphetamine gates closed-lid mode.
        let script = format!(
            "do shell script \"/usr/bin/pmset -a disablesleep {val}\" \
             with administrator privileges"
        );
        match std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(&script)
            .status()
        {
            Ok(status) => status.success(),
            Err(e) => {
                tracing::warn!(?e, "pmset via osascript failed");
                false
            }
        }
    }

    /// Read (no privileges needed) whether the system currently has sleep
    /// disabled, so recovery only prompts when the flag is really stuck.
    fn sleep_currently_disabled() -> bool {
        std::process::Command::new("/usr/bin/pmset")
            .arg("-g")
            .output()
            .map(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .any(|l| l.contains("SleepDisabled") && l.trim_end().ends_with('1'))
            })
            .unwrap_or(false)
    }

    /// Write or clear the on-disk sentinel that records "we have sleep
    /// disabled". Written the instant we set the flag so an unclean exit
    /// is recoverable on the next launch.
    fn set_marker(on: bool) {
        let Some(path) = crate::paths::lid_awake_marker() else {
            return;
        };
        if on {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, b"1");
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }

    fn marker_present() -> bool {
        crate::paths::lid_awake_marker()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// See `super::recover_lid_closed`.
    pub fn recover_lid_closed() {
        if !LID_CLOSED_SUPPORTED || !marker_present() {
            return;
        }
        if sleep_currently_disabled() {
            tracing::warn!("system sleep left disabled after unclean exit; restoring");
            set_disablesleep(false);
        }
        set_marker(false);
    }

    impl Backend {
        pub fn engage(
            _duration: Option<Duration>,
            allow_display_sleep: bool,
            lid_closed: bool,
        ) -> Option<Self> {
            let lid = lid_closed && LID_CLOSED_SUPPORTED;
            // With the lid shut the panel is off anyway, so a lid-closed
            // session always lets the display sleep. Otherwise:
            // PreventUserIdleDisplaySleep keeps the display (and therefore
            // the system) awake; PreventUserIdleSystemSleep keeps the
            // system awake but lets the display sleep.
            let assertion_type = if allow_display_sleep || lid {
                "PreventUserIdleSystemSleep"
            } else {
                "PreventUserIdleDisplaySleep"
            };
            let type_str = cfstr(assertion_type);
            let name_str = cfstr("klipa keep awake");
            if type_str.is_null() || name_str.is_null() {
                // SAFETY: each pointer is either a valid CFString or null.
                unsafe {
                    if !type_str.is_null() {
                        CFRelease(type_str);
                    }
                    if !name_str.is_null() {
                        CFRelease(name_str);
                    }
                }
                return None;
            }
            let mut id: u32 = 0;
            // SAFETY: both CFStrings are valid; `id` is a valid out-pointer.
            let rc = unsafe {
                IOPMAssertionCreateWithName(type_str, ASSERTION_LEVEL_ON, name_str, &mut id)
            };
            // SAFETY: the CFStrings we created are copied by the call above;
            // release our references now.
            unsafe {
                CFRelease(type_str);
                CFRelease(name_str);
            }
            if rc != IO_SUCCESS {
                tracing::warn!(rc, "IOPMAssertionCreateWithName failed");
                return None;
            }

            // Lid-closed mode: set the root power flag. If the user
            // cancels the admin prompt (or it fails), don't start a
            // half-on session that only survives an open lid, release the
            // assertion and report failure so the UI stays honest.
            let mut disabled_sleep = false;
            if lid {
                if set_disablesleep(true) {
                    set_marker(true);
                    disabled_sleep = true;
                } else {
                    // SAFETY: releasing the assertion we just created.
                    unsafe {
                        IOPMAssertionRelease(id);
                    }
                    return None;
                }
            }

            Some(Self {
                assertion: id,
                disabled_sleep,
            })
        }

        pub fn finished(&mut self) -> bool {
            // No OS timer; the deadline in KeepAwake drives ending.
            false
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            // SAFETY: releases the assertion we created in `engage`.
            unsafe {
                IOPMAssertionRelease(self.assertion);
            }
            // Restore normal sleep if we disabled it. This prompts for the
            // admin password once more; `main` ends the session before
            // quitting so the prompt lands while the app is still alive,
            // and the on-disk marker covers any exit that skips this.
            if self.disabled_sleep {
                set_disablesleep(false);
                set_marker(false);
            }
        }
    }
}

/// Linux / other Unix: keep awake via `systemd-inhibit`, which holds an
/// idle inhibitor for as long as the command it runs stays alive. We run
/// `sleep` for the session length (or `infinity`) and kill it to release
/// early. Requires systemd-logind, present on most desktop distros.
///
/// `allow_display_sleep` can't be honored separately here: the idle
/// inhibitor blocks the whole idle path (screen blank + auto-suspend).
#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::Duration;
    use std::process::{Child, Command, Stdio};

    pub struct Backend(Child);

    /// systemd-inhibit only blocks idle suspend; there is no portable
    /// "stay on with the lid closed" equivalent, so the feature is hidden
    /// here.
    pub const LID_CLOSED_SUPPORTED: bool = false;

    /// No lid-closed flag is ever set off macOS, so nothing to recover.
    pub fn recover_lid_closed() {}

    impl Backend {
        pub fn engage(
            duration: Option<Duration>,
            _allow_display_sleep: bool,
            _lid_closed: bool,
        ) -> Option<Self> {
            let sleep_arg = match duration {
                Some(d) => d.as_secs().max(1).to_string(),
                None => "infinity".to_string(),
            };
            let mut cmd = Command::new("systemd-inhibit");
            cmd.arg("--what=idle")
                .arg("--who=klipa")
                .arg("--why=klipa keep awake")
                .arg("--mode=block")
                .arg("sleep")
                .arg(sleep_arg)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            match cmd.spawn() {
                Ok(child) => Some(Self(child)),
                Err(e) => {
                    tracing::warn!(?e, "failed to start systemd-inhibit (is systemd present?)");
                    None
                }
            }
        }

        pub fn finished(&mut self) -> bool {
            matches!(self.0.try_wait(), Ok(Some(_)))
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Windows: keep awake via `SetThreadExecutionState`. This sets a flag on
/// the calling thread that persists until cleared or the thread exits, so
/// it must run on klipa's long-lived main thread (it does - every call
/// goes through the event loop). There is no built-in timer, so timed
/// sessions are ended by `KeepAwake::poll` via the deadline.
#[cfg(target_os = "windows")]
mod platform {
    use super::Duration;

    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(es_flags: u32) -> u32;
    }

    pub struct Backend;

    /// Windows has no lid-closed keep-awake equivalent klipa exposes.
    pub const LID_CLOSED_SUPPORTED: bool = false;

    /// No lid-closed flag is ever set off macOS, so nothing to recover.
    pub fn recover_lid_closed() {}

    impl Backend {
        pub fn engage(
            _duration: Option<Duration>,
            allow_display_sleep: bool,
            _lid_closed: bool,
        ) -> Option<Self> {
            let mut flags = ES_CONTINUOUS | ES_SYSTEM_REQUIRED;
            if !allow_display_sleep {
                flags |= ES_DISPLAY_REQUIRED;
            }
            // SAFETY: documented Win32 call; sets the calling thread's
            // execution state and returns the previous state (0 on error).
            let previous = unsafe { SetThreadExecutionState(flags) };
            if previous == 0 {
                tracing::warn!("SetThreadExecutionState failed");
                return None;
            }
            Some(Self)
        }

        pub fn finished(&mut self) -> bool {
            // No OS timer; the deadline in KeepAwake drives ending.
            false
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            // SAFETY: clears the keep-awake flags on the same thread.
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        }
    }
}

/// Any other target (e.g. wasm): track session state but make no OS
/// assertion. The timer still works via the deadline.
#[cfg(not(any(unix, windows)))]
mod platform {
    use super::Duration;

    pub struct Backend;

    /// Unknown target: nothing is enforced, so nothing is supported.
    pub const LID_CLOSED_SUPPORTED: bool = false;

    /// No lid-closed flag is ever set off macOS, so nothing to recover.
    pub fn recover_lid_closed() {}

    impl Backend {
        pub fn engage(
            _duration: Option<Duration>,
            _allow_display_sleep: bool,
            _lid_closed: bool,
        ) -> Option<Self> {
            tracing::info!("keep-awake is not enforced on this platform");
            Some(Self)
        }

        pub fn finished(&mut self) -> bool {
            false
        }
    }
}
