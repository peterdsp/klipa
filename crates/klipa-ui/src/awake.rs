//! Amphetamine-style "keep awake" sessions.
//!
//! Prevents the machine from idle-sleeping for a chosen duration (or
//! indefinitely), using each OS's native mechanism:
//!
//! * **macOS**   - an IOKit power assertion (`IOPMAssertionCreateWithName`),
//!   the same public API the `caffeinate` tool wraps. Held in-process so
//!   it works inside the App Sandbox (no subprocess to spawn).
//! * **Windows** - `SetThreadExecutionState` (a single Win32 call).
//! * **Linux**   - spawns `systemd-inhibit`, which holds an idle
//!   inhibitor for as long as its child process is alive.
//!
//! No extra dependency on any platform. The cross-platform session
//! bookkeeping (timer, display-sleep flag) lives here; the small
//! platform module below is the only OS-specific part.

use std::time::{Duration, Instant};

/// A running (or stopped) keep-awake session.
pub struct KeepAwake {
    /// The live OS wake lock; `None` while idle. Dropping it releases.
    backend: Option<platform::Backend>,
    /// When a timed session ends; `None` while idle or indefinite.
    deadline: Option<Instant>,
    /// If true, the display may still sleep while the system stays awake.
    allow_display_sleep: bool,
}

/// Snapshot of the session for rendering the menu.
pub struct AwakeView {
    pub active: bool,
    /// Human label for the active session, e.g. "Awake - 29m left".
    pub status: Option<String>,
    pub allow_display_sleep: bool,
}

impl KeepAwake {
    pub fn new() -> Self {
        Self {
            backend: None,
            deadline: None,
            allow_display_sleep: false,
        }
    }

    pub fn allow_display_sleep(&self) -> bool {
        self.allow_display_sleep
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
        self.backend = platform::Backend::engage(duration, self.allow_display_sleep);
        if self.backend.is_none() {
            // Engaging the OS lock failed; don't pretend we're awake.
            self.deadline = None;
        }
    }

    /// End any active session immediately.
    pub fn end(&mut self) {
        // Dropping the backend releases the OS wake lock.
        self.backend = None;
        self.deadline = None;
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
        AwakeView {
            active: self.is_active(),
            status: if self.is_active() {
                Some(match self.remaining() {
                    Some(left) => format!("Awake - {} left", fmt_remaining(left)),
                    None => "Awake - indefinitely".to_string(),
                })
            } else {
                None
            },
            allow_display_sleep: self.allow_display_sleep,
        }
    }
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
/// public API that `caffeinate` itself wraps, called in-process so it
/// works inside the App Sandbox with no subprocess and no entitlements.
/// The assertion is held for the life of the `Backend` and released on
/// `Drop`. There is no OS-side timer (the simple assertion API has none),
/// so timed sessions are ended by `KeepAwake::poll` via the deadline,
/// exactly like the Windows backend.
#[cfg(target_os = "macos")]
mod platform {
    use super::Duration;
    use std::ffi::c_void;

    /// `IOPMAssertionID` returned by the create call; passed back to
    /// release. 0 is never a valid live assertion.
    pub struct Backend(u32);

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

    impl Backend {
        pub fn engage(_duration: Option<Duration>, allow_display_sleep: bool) -> Option<Self> {
            // PreventUserIdleDisplaySleep keeps the display (and therefore
            // the system) awake; PreventUserIdleSystemSleep keeps the
            // system awake but lets the display sleep.
            let assertion_type = if allow_display_sleep {
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
            if rc == IO_SUCCESS {
                Some(Self(id))
            } else {
                tracing::warn!(rc, "IOPMAssertionCreateWithName failed");
                None
            }
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
                IOPMAssertionRelease(self.0);
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

    impl Backend {
        pub fn engage(duration: Option<Duration>, _allow_display_sleep: bool) -> Option<Self> {
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

    impl Backend {
        pub fn engage(_duration: Option<Duration>, allow_display_sleep: bool) -> Option<Self> {
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

    impl Backend {
        pub fn engage(_duration: Option<Duration>, _allow_display_sleep: bool) -> Option<Self> {
            tracing::info!("keep-awake is not enforced on this platform");
            Some(Self)
        }

        pub fn finished(&mut self) -> bool {
            false
        }
    }
}
