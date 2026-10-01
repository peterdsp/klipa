//! Amphetamine-style "keep awake" sessions.
//!
//! Stops the machine idle-sleeping for a chosen duration, or
//! indefinitely, using each OS's native mechanism:
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
//! No extra dependency on any platform.
//!
//! # Shape of this module
//!
//! * [`AwakeMode`] and [`AwakeDuration`] are the domain: *what* a session
//!   asks the OS for, and *how long*. `AwakeDuration::Indefinite` is a
//!   real case with no duration inside it, so an indefinite session
//!   cannot grow a deadline, a timer, or a ceiling by accident.
//! * [`PowerSource`] is the single seam onto the OS power APIs, so the
//!   session logic below is testable without touching a real Mac's power
//!   state. [`platform::OsPower`] is the only production implementation.
//! * [`KeepAwake`] owns at most one [`WakeLock`] at a time. The lock is
//!   the assertion: creating one is the only way to get it, dropping it
//!   is the only way to release it, and there is exactly one slot for it.
//!   That makes a duplicate or an orphaned assertion unrepresentable.

use crate::clamshell::ClamshellStatus;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

// ── Domain ───────────────────────────────────────────────────────────

/// How long a keep-awake session runs.
///
/// `Indefinite` carries no duration at all. It is deliberately *not*
/// `For(a very large duration)`: there is no value to overflow, no
/// deadline to compute, no timer to fire, and no ceiling to hit. An
/// indefinite session ends only when the user stops it, klipa exits, the
/// machine restarts, or the OS invalidates the assertion.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AwakeDuration {
    /// Run until explicitly stopped. No expiry of any kind.
    Indefinite,
    /// Run for this long, then release on its own.
    For(Duration),
}

impl AwakeDuration {
    pub fn is_indefinite(self) -> bool {
        matches!(self, Self::Indefinite)
    }

    /// The instant this session ends, or `None` when indefinite. The only
    /// place a deadline is ever produced, so indefinite cannot acquire
    /// one behind our back.
    fn deadline(self, now: Instant) -> Option<Instant> {
        match self {
            Self::Indefinite => None,
            Self::For(d) => Some(now + d),
        }
    }
}

/// What a session asks the OS to hold. Exactly one is in effect at a
/// time: these are modes, not flags to be OR-ed together, so each one
/// requests only the behavior it actually needs.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwakeMode {
    /// Keep the screen and the machine awake.
    ///
    /// macOS: `kIOPMAssertionTypePreventUserIdleDisplaySleep`, on its own.
    /// Holding the display awake already holds the system awake, so
    /// adding a system assertion on top would be a second assertion for
    /// no extra behavior.
    #[default]
    ScreenAndSystem,
    /// Keep the machine awake and let the display turn off normally.
    /// Downloads, builds and servers keep running with a dark screen.
    ///
    /// macOS: `kIOPMAssertionTypePreventUserIdleSystemSleep`, on its own.
    SystemOnly,
    /// Keep running with the lid closed.
    ///
    /// The display is off behind a shut lid, so this holds the same
    /// system-only assertion as [`AwakeMode::SystemOnly`] and adds the
    /// one lever that actually changes lid-close behavior. See
    /// `docs/lid-closed-keep-awake.md`: a power assertion does **not**
    /// override the lid switch on any Mac, Intel or Apple Silicon, so
    /// this mode exists only where klipa can set the system
    /// `disablesleep` flag, and reports honestly everywhere else.
    LidClosed,
}

impl AwakeMode {
    /// Whether this mode lets the display power down on its own.
    ///
    /// The Windows backend maps straight off this (one execution-state
    /// flag); macOS picks a whole assertion type per mode instead, so
    /// there it only appears in the tests that pin the mapping down.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn allows_display_sleep(self) -> bool {
        matches!(self, Self::SystemOnly | Self::LidClosed)
    }
}

/// Why a lid-closed session could not actually engage. Lets the menu name
/// the real cause instead of one vague "blocked or declined" catch-all.
///
/// Off macOS these are never constructed (lid-closed is macOS-only), so the
/// variants read as dead code there; they are real where it matters.
#[cfg_attr(any(not(target_os = "macos"), feature = "mas"), allow(dead_code))]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum LidBlock {
    /// The admin password prompt was cancelled by the user.
    Declined,
    /// The change ran without error but the system never applied it: a
    /// managed Mac's power policy silently overrides `disablesleep`.
    Refused,
    /// The mechanism itself could not run (`osascript`/`pmset` missing or
    /// errored), so we never even reached the system.
    Unavailable,
}

/// Why a session failed to start. Separates a plain wake-lock failure
/// from a lid-closed failure so the menu can name each honestly.
///
/// Either way the session is *not* started: `KeepAwake` holds no lock and
/// reports `active == false`, so the UI never shows a session that the OS
/// declined.
#[cfg_attr(any(not(target_os = "macos"), feature = "mas"), allow(dead_code))]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EngageErr {
    /// The base OS wake lock (IOKit assertion / execution state /
    /// systemd inhibitor) could not be acquired.
    Assertion,
    /// The wake lock held, but the lid-closed change did not, for this
    /// reason. The lock was released again rather than left half-on.
    Lid(LidBlock),
}

/// The effective protection state shown in the menu, distinguishing a
/// preference from verified behavior. The non-lid modes only ever reach
/// `Inactive` or `Active` (an IOKit assertion either holds or it does not);
/// the richer states describe the lid-closed override, whose real state the
/// composition root reads from the helper (or the system flag) rather than
/// inferring from an in-memory session. The actual `disablesleep` read and
/// restore logic lives in `lid.rs`.
#[cfg_attr(any(not(target_os = "macos"), feature = "mas"), allow(dead_code))]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ProtectionState {
    /// No session, or nothing requested.
    Inactive,
    /// Requested; acquiring the override.
    Preparing,
    /// Requested; waiting for the user to approve the helper in Settings
    /// (or re-authorize after a cancelled prompt).
    AwaitingApproval,
    /// Acquired; confirming the system actually reflects it.
    Verifying,
    /// Verified: the system is genuinely held as requested.
    Active,
    /// Was active but the effective state slipped (e.g. the flag read back
    /// off); recovery is owed.
    DegradedRecovering,
    /// Ending: restoring the prior settings. Reserved for a future
    /// asynchronous restore; the current restore completes synchronously
    /// before the menu redraws, so this is defined for completeness of the
    /// model but not surfaced on its own.
    #[allow(dead_code)]
    Restoring,
    /// A restore could not be confirmed, or a hard failure needs the user's
    /// attention.
    RestorationFailed,
}

// ── The OS seam ──────────────────────────────────────────────────────

/// A live OS wake lock.
///
/// The assertion id (or child process, or execution-state flag) lives
/// inside the implementor and is released by its `Drop`. Nothing outside
/// can copy it, so it cannot be released twice or leaked past the value's
/// lifetime.
pub trait WakeLock {
    /// True if the OS released the lock on its own: only Linux's
    /// `systemd-inhibit` child can do that. Backends with no OS-side
    /// timer keep the default.
    fn finished(&mut self) -> bool {
        false
    }
}

/// The single place that talks to the OS power APIs.
///
/// Production is [`platform::OsPower`]; tests substitute a fake so the
/// session logic can be exercised without changing a real Mac's power
/// state.
pub trait PowerSource {
    /// Acquire a wake lock for `mode`. `duration` is passed through only
    /// for backends whose OS mechanism takes one (Linux's inhibitor
    /// child); macOS and Windows have no OS-side timer and ignore it,
    /// with `KeepAwake` driving expiry instead.
    fn engage(
        &self,
        mode: AwakeMode,
        duration: AwakeDuration,
    ) -> Result<Box<dyn WakeLock>, EngageErr>;

    /// Whether [`AwakeMode::LidClosed`] can do anything on this build and
    /// platform. False everywhere except the direct macOS build, so the
    /// menu can hide a control that would be a lie.
    fn supports_lid_closed(&self) -> bool {
        false
    }
}

// ── Session ──────────────────────────────────────────────────────────

/// The one running session. Owning the lock here, and nowhere else,
/// is what guarantees a single live assertion.
struct Session {
    /// Dropping this releases the OS assertion.
    lock: Box<dyn WakeLock>,
    mode: AwakeMode,
    duration: AwakeDuration,
    /// `Some` only for [`AwakeDuration::For`]. An indefinite session
    /// stores `None`, so `poll` has nothing to compare against and can
    /// never expire it.
    deadline: Option<Instant>,
}

/// A running (or stopped) keep-awake session.
pub struct KeepAwake {
    power: Box<dyn PowerSource>,
    /// The live session; `None` while idle.
    session: Option<Session>,
    /// The user's chosen mode. Survives a session ending, and is
    /// persisted across launches; the *session* is not.
    mode: AwakeMode,
    /// Why the last start attempt failed, if it did. Cleared whenever a
    /// session starts or is ended by the user.
    error: Option<EngageErr>,
}

/// Snapshot of the session for rendering the menu.
pub struct AwakeView {
    pub active: bool,
    /// Runtime state, always present: "Inactive", "Awake indefinitely",
    /// or "Awake for 43m". Indefinite sessions never carry a countdown.
    pub status: String,
    /// What the active session actually holds, e.g.
    /// "Mac awake - display may sleep". `None` while idle.
    pub detail: Option<String>,
    /// True while an indefinite session is running, so the UI can say so
    /// rather than inferring it from a missing number.
    pub indefinite: bool,
    /// The selected mode (a preference; it outlives the session).
    pub mode: AwakeMode,
    /// Whether this build/platform can honor [`AwakeMode::LidClosed`] at
    /// all, so the menu can hide the option where it would do nothing.
    pub lid_closed_supported: bool,
    /// Why the last start attempt failed, if it did. `None` when the last
    /// request succeeded or none was made.
    pub error: Option<EngageErr>,
    /// Passwordless-helper state, for the "Enable passwordless mode" menu
    /// entries. `KeepAwake` doesn't own the helper, so `view` leaves these
    /// at their defaults and the composition root fills them in.
    /// `helper_installable` is true only when setup is actually offerable
    /// (macOS 13+, not yet registered), so the menu stays clean on older
    /// systems where only the admin-prompt path exists.
    pub helper_active: bool,
    pub helper_needs_approval: bool,
    pub helper_installable: bool,
    /// The last custom session length in minutes, for the "Custom..."
    /// menu label. A persisted preference the composition root owns, so
    /// `view` defaults it to `None`.
    pub custom_minutes: Option<u64>,
    /// What happens if the lid closes now (external display / power).
    /// Sandbox-safe and shown in every build; filled in by the
    /// composition root, so `view` defaults it to `Hidden`.
    pub clamshell: ClamshellStatus,
    /// The effective, verified protection state for the current mode, read
    /// from the real override/helper state by the composition root (which
    /// owns the lid coordinator), so `view` defaults it to `Inactive`.
    pub protection: ProtectionState,
}

impl KeepAwake {
    pub fn new() -> Self {
        Self::with_power(Box::new(platform::OsPower))
    }

    /// Build over an arbitrary [`PowerSource`]. Production goes through
    /// [`KeepAwake::new`]; tests pass a fake here.
    pub fn with_power(power: Box<dyn PowerSource>) -> Self {
        Self {
            power,
            session: None,
            mode: AwakeMode::default(),
            error: None,
        }
    }

    pub fn mode(&self) -> AwakeMode {
        self.mode
    }

    /// Adopt a persisted mode preference at launch **without** starting
    /// anything. Preference state and runtime assertion state are
    /// separate: klipa restores what you picked, it does not resurrect a
    /// session, because the IOKit assertion from the previous run died
    /// with that process.
    pub fn restore_mode(&mut self, mode: AwakeMode) {
        if mode == AwakeMode::LidClosed && !self.power.supports_lid_closed() {
            // Saved on a build that supports it, loaded on one that does
            // not. Fall back rather than showing an inert selection.
            self.mode = AwakeMode::default();
            return;
        }
        self.mode = mode;
    }

    /// Pick the mode.
    ///
    /// With a session running this restarts it under the new mode,
    /// preserving indefiniteness exactly (an indefinite session stays
    /// indefinite; a timed one keeps the time it has left). With no
    /// session running it starts an indefinite one, so picking a mode is
    /// the one-tap way to start keeping the machine awake, matching how
    /// the lid-closed switch has behaved since 0.5.4.
    ///
    /// Selecting the mode a running session already uses does nothing at
    /// all, so a menu rebuild or a repeated click never re-creates the
    /// assertion. With nothing running (including after a failed start,
    /// such as a cancelled admin prompt) selecting it tries again.
    pub fn set_mode(&mut self, mode: AwakeMode) {
        if mode == AwakeMode::LidClosed && !self.power.supports_lid_closed() {
            return;
        }
        if self.mode == mode && self.session.is_some() {
            return;
        }
        self.mode = mode;
        match self.remaining_request() {
            Some(request) => self.start(request),
            None => self.start(AwakeDuration::Indefinite),
        }
    }

    /// Begin a session, replacing any running one.
    ///
    /// The previous lock is released before the new one is requested (via
    /// `end`), so two assertions never coexist. If the OS refuses, no
    /// session is recorded and the reason is kept for the menu.
    pub fn start(&mut self, duration: AwakeDuration) {
        self.end();
        let mode = self.mode;
        match self.power.engage(mode, duration) {
            Ok(lock) => {
                self.session = Some(Session {
                    lock,
                    mode,
                    duration,
                    deadline: duration.deadline(Instant::now()),
                });
            }
            Err(err) => {
                // The OS declined. Stay honest: no session, no deadline,
                // and a reason the menu can show.
                tracing::warn!("keep-awake could not start");
                self.error = Some(err);
            }
        }
    }

    /// End any active session immediately, releasing the OS assertion.
    pub fn end(&mut self) {
        // Dropping the session drops the lock, which releases the OS
        // assertion (and restores system sleep for a lid-closed one).
        self.session = None;
        self.error = None;
    }

    /// Reap a session whose timer elapsed (or whose helper process
    /// exited). Returns true if the active state changed, so the caller
    /// can refresh the menu.
    ///
    /// An indefinite session has no deadline, so the only thing that can
    /// end it here is the OS releasing the lock itself.
    pub fn poll(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        // A helper-process backend finished on its own (Linux's
        // `systemd-inhibit sleep`); macOS/Windows report false here and
        // are ended by the deadline below.
        if session.lock.finished() {
            self.end();
            return true;
        }
        // Or our own timer elapsed. `deadline` is `None` for an
        // indefinite session, so this branch cannot fire for one.
        if session.deadline.is_some_and(|d| Instant::now() >= d) {
            self.end();
            return true;
        }
        false
    }

    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }

    /// True while an indefinite session is running. Lets the event loop
    /// skip the countdown refresh without building a whole `AwakeView`
    /// every tick.
    pub fn is_indefinite(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.duration.is_indefinite())
    }

    /// What to ask for when restarting the running session under a new
    /// mode: `Indefinite` stays `Indefinite`, a timed session keeps only
    /// the time it has left. `None` when nothing is running. Also used by
    /// the composition root to drive the lid override's remaining session
    /// time (so the daemon owns the correct expiry).
    pub fn remaining_request(&self) -> Option<AwakeDuration> {
        let session = self.session.as_ref()?;
        Some(match session.duration {
            AwakeDuration::Indefinite => AwakeDuration::Indefinite,
            AwakeDuration::For(_) => AwakeDuration::For(
                session
                    .deadline
                    .map(|d| d.saturating_duration_since(Instant::now()))
                    .unwrap_or_default(),
            ),
        })
    }

    pub fn view(&self) -> AwakeView {
        let indefinite = self.is_indefinite();
        let status = match self.session.as_ref() {
            None => "Inactive".to_string(),
            // No countdown here, and nothing to count down to: an
            // indefinite session holds no deadline.
            Some(s) if s.duration.is_indefinite() => "Awake indefinitely".to_string(),
            Some(s) => {
                let left = s
                    .deadline
                    .map(|d| d.saturating_duration_since(Instant::now()))
                    .unwrap_or_default();
                format!("Awake for {}", fmt_remaining(left))
            }
        };
        let detail = self.session.as_ref().map(|s| {
            match s.mode {
                AwakeMode::ScreenAndSystem => "Screen and system awake",
                AwakeMode::SystemOnly => "System awake - display may sleep",
                AwakeMode::LidClosed => "System awake - lid closed, display off",
            }
            .to_string()
        });
        AwakeView {
            active: self.is_active(),
            status,
            detail,
            indefinite,
            mode: self.mode,
            lid_closed_supported: self.power.supports_lid_closed(),
            error: self.error,
            // Filled in by the composition root, which owns helper state.
            helper_active: false,
            helper_needs_approval: false,
            helper_installable: false,
            custom_minutes: None,
            clamshell: ClamshellStatus::Hidden,
            protection: if self.is_active() {
                ProtectionState::Active
            } else {
                ProtectionState::Inactive
            },
        }
    }
}

/// Restore normal sleep if a previous "keep awake with lid closed"
/// session left the system `disablesleep` flag set after an unclean exit
/// (crash, force-quit, power loss). Call once at startup.
///
/// The admin-prompt (Option A) path is reconciled here from the app-side
/// journal (see `lid.rs`). The helper path's override is reconciled by the
/// privileged daemon itself, which owns its own root journal and lease, so
/// the app does not (and cannot) touch it. No-op off macOS.
pub fn recover_lid_closed() {
    #[cfg(all(target_os = "macos", not(feature = "mas")))]
    crate::lid::recover();
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
// Each platform module exposes `OsPower`, the production `PowerSource`,
// whose `Lock` holds the OS wake lock and releases it on `Drop`.

/// macOS: keep awake via an IOKit power-management assertion. This is the
/// public API that `caffeinate` itself wraps, called in-process so the idle
/// keep-awake works inside the App Sandbox with no subprocess and no
/// entitlements. The assertion is held for the life of the `Lock` and
/// released on `Drop`. There is no OS-side timer (the simple assertion API
/// has none), so timed sessions are ended by `KeepAwake::poll` via the
/// deadline, exactly like the Windows backend.
///
/// Lid-closed mode takes the same system assertion as `SystemOnly` here;
/// the privileged `disablesleep` override it also needs is owned separately
/// by the lid coordinator (`lid.rs`) and the root daemon, not by this lock.
/// Keeping the override out of the lock is what lets a duration or mode
/// change update it in place instead of dropping and re-taking it.
#[cfg(target_os = "macos")]
mod platform {
    use super::{AwakeDuration, AwakeMode, EngageErr, WakeLock};
    use std::ffi::c_void;

    /// The production [`super::PowerSource`].
    pub struct OsPower;

    /// A live keep-awake lock: the IOKit assertion, released on `Drop`.
    pub struct Lock {
        assertion: u32,
    }

    /// Only the non-App-Store build can honor lid-closed mode: it needs a
    /// privileged `disablesleep` change the App Sandbox forbids. In the
    /// sandboxed (`mas`) build this is `false` and the lid-closed path is
    /// inert (the menu hides the control).
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
        unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) }
    }

    /// The single IOKit assertion type each mode needs, and no more.
    ///
    /// `kIOPMAssertionTypePreventUserIdleDisplaySleep` already implies the
    /// system stays awake, so `ScreenAndSystem` does not also take a system
    /// assertion. The other two modes want the display free to power down,
    /// so they take only `kIOPMAssertionTypePreventUserIdleSystemSleep`.
    fn assertion_type(mode: AwakeMode) -> &'static str {
        match mode {
            AwakeMode::ScreenAndSystem => "PreventUserIdleDisplaySleep",
            AwakeMode::SystemOnly | AwakeMode::LidClosed => "PreventUserIdleSystemSleep",
        }
    }

    impl super::PowerSource for OsPower {
        fn engage(
            &self,
            mode: AwakeMode,
            _duration: AwakeDuration,
        ) -> Result<Box<dyn WakeLock>, EngageErr> {
            let type_str = cfstr(assertion_type(mode));
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
                tracing::warn!("could not build the assertion CFStrings");
                return Err(EngageErr::Assertion);
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
                return Err(EngageErr::Assertion);
            }
            Ok(Box::new(Lock { assertion: id }))
        }

        fn supports_lid_closed(&self) -> bool {
            LID_CLOSED_SUPPORTED
        }
    }

    // No OS timer; the deadline in `KeepAwake` drives ending, so the default
    // `finished` (always false) is correct here.
    impl WakeLock for Lock {}

    impl Drop for Lock {
        fn drop(&mut self) {
            // SAFETY: releases the assertion we created in `engage`.
            let rc = unsafe { IOPMAssertionRelease(self.assertion) };
            if rc != 0 {
                tracing::warn!(rc, "IOPMAssertionRelease failed");
            }
        }
    }
}

/// Linux / other Unix: keep awake via `systemd-inhibit`, which holds an
/// idle inhibitor for as long as the command it runs stays alive. We run
/// `sleep` for the session length (or `infinity`, the shell's own
/// no-deadline form, for an indefinite session) and kill it to release
/// early. Requires systemd-logind, present on most desktop distros.
///
/// The display/system distinction can't be honored separately here: the
/// idle inhibitor blocks the whole idle path (screen blank + auto-suspend).
#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::{AwakeDuration, AwakeMode, EngageErr, WakeLock};
    use std::process::{Child, Command, Stdio};

    pub struct OsPower;
    pub struct Lock(Child);

    impl super::PowerSource for OsPower {
        fn engage(
            &self,
            _mode: AwakeMode,
            duration: AwakeDuration,
        ) -> Result<Box<dyn WakeLock>, EngageErr> {
            let sleep_arg = match duration {
                AwakeDuration::For(d) => d.as_secs().max(1).to_string(),
                // `sleep infinity` never returns, so the inhibitor is
                // held until we kill the child. No large number involved.
                AwakeDuration::Indefinite => "infinity".to_string(),
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
                Ok(child) => Ok(Box::new(Lock(child))),
                Err(e) => {
                    tracing::warn!(?e, "failed to start systemd-inhibit (is systemd present?)");
                    Err(EngageErr::Assertion)
                }
            }
        }
    }

    impl WakeLock for Lock {
        fn finished(&mut self) -> bool {
            matches!(self.0.try_wait(), Ok(Some(_)))
        }
    }

    impl Drop for Lock {
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
    use super::{AwakeDuration, AwakeMode, EngageErr, WakeLock};

    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(es_flags: u32) -> u32;
    }

    pub struct OsPower;
    pub struct Lock;

    impl super::PowerSource for OsPower {
        fn engage(
            &self,
            mode: AwakeMode,
            _duration: AwakeDuration,
        ) -> Result<Box<dyn WakeLock>, EngageErr> {
            // Same split as macOS: the display flag is added only by the
            // mode that actually wants the screen kept on.
            let mut flags = ES_CONTINUOUS | ES_SYSTEM_REQUIRED;
            if !mode.allows_display_sleep() {
                flags |= ES_DISPLAY_REQUIRED;
            }
            // SAFETY: documented Win32 call; sets the calling thread's
            // execution state and returns the previous state (0 on error).
            let previous = unsafe { SetThreadExecutionState(flags) };
            if previous == 0 {
                tracing::warn!("SetThreadExecutionState failed");
                return Err(EngageErr::Assertion);
            }
            Ok(Box::new(Lock))
        }
    }

    // No OS timer; the deadline in `KeepAwake` drives ending.
    impl WakeLock for Lock {}

    impl Drop for Lock {
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
    use super::{AwakeDuration, AwakeMode, EngageErr, WakeLock};

    pub struct OsPower;
    pub struct Lock;

    impl super::PowerSource for OsPower {
        fn engage(
            &self,
            _mode: AwakeMode,
            _duration: AwakeDuration,
        ) -> Result<Box<dyn WakeLock>, EngageErr> {
            tracing::info!("keep-awake is not enforced on this platform");
            Ok(Box::new(Lock))
        }
    }

    impl WakeLock for Lock {}
}

// ── Tests ────────────────────────────────────────────────────────────
//
// The whole point of the `PowerSource` seam: these exercise the session
// rules (indefinite has no expiry, modes map to the right assertion,
// every replacement releases first) against a recording fake, so nothing
// here touches a real machine's power state.
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Log {
        /// Every successful `engage`, in order.
        engaged: Vec<(AwakeMode, AwakeDuration)>,
        /// How many locks have been dropped (i.e. assertions released).
        released: usize,
        /// When set, every `engage` fails with this instead.
        fail: Option<EngageErr>,
    }

    struct FakeLock(Rc<RefCell<Log>>);
    impl WakeLock for FakeLock {}
    impl Drop for FakeLock {
        fn drop(&mut self) {
            self.0.borrow_mut().released += 1;
        }
    }

    struct FakePower {
        log: Rc<RefCell<Log>>,
        lid_supported: bool,
    }

    impl PowerSource for FakePower {
        fn engage(
            &self,
            mode: AwakeMode,
            duration: AwakeDuration,
        ) -> Result<Box<dyn WakeLock>, EngageErr> {
            let fail = self.log.borrow().fail;
            if let Some(err) = fail {
                return Err(err);
            }
            self.log.borrow_mut().engaged.push((mode, duration));
            Ok(Box::new(FakeLock(self.log.clone())))
        }

        fn supports_lid_closed(&self) -> bool {
            self.lid_supported
        }
    }

    /// A session manager over a recording fake. `lid` says whether the
    /// fake platform claims lid-closed support.
    fn harness(lid: bool) -> (KeepAwake, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let power = FakePower {
            log: log.clone(),
            lid_supported: lid,
        };
        (KeepAwake::with_power(Box::new(power)), log)
    }

    fn engaged(log: &Rc<RefCell<Log>>) -> Vec<(AwakeMode, AwakeDuration)> {
        log.borrow().engaged.clone()
    }

    const HOUR: Duration = Duration::from_secs(3600);

    #[test]
    fn indefinite_starts_and_is_marked_indefinite() {
        let (mut awake, log) = harness(false);
        awake.start(AwakeDuration::Indefinite);
        assert!(awake.is_active());
        assert_eq!(
            engaged(&log),
            vec![(AwakeMode::ScreenAndSystem, AwakeDuration::Indefinite)]
        );
        let view = awake.view();
        assert!(view.active);
        assert!(view.indefinite);
        assert_eq!(view.status, "Awake indefinitely");
    }

    #[test]
    fn indefinite_creates_no_expiry() {
        let (mut awake, _log) = harness(false);
        awake.start(AwakeDuration::Indefinite);
        // The single source of expiry is the session deadline, and an
        // indefinite session simply does not have one.
        assert!(awake.session.as_ref().unwrap().deadline.is_none());
        assert!(AwakeDuration::Indefinite.deadline(Instant::now()).is_none());
    }

    #[test]
    fn indefinite_never_expires_on_poll() {
        let (mut awake, _log) = harness(false);
        awake.start(AwakeDuration::Indefinite);
        for _ in 0..1000 {
            assert!(!awake.poll(), "an indefinite session must never expire");
        }
        assert!(awake.is_active());
    }

    #[test]
    fn stopping_indefinite_releases_the_assertion() {
        let (mut awake, log) = harness(false);
        awake.start(AwakeDuration::Indefinite);
        assert_eq!(log.borrow().released, 0);
        awake.end();
        assert!(!awake.is_active());
        assert_eq!(log.borrow().released, 1);
        assert_eq!(awake.view().status, "Inactive");
    }

    #[test]
    fn timed_session_still_expires() {
        let (mut awake, log) = harness(false);
        // A zero-length session is already past its deadline.
        awake.start(AwakeDuration::For(Duration::ZERO));
        assert!(awake.is_active());
        assert!(awake.poll(), "an elapsed timed session must be reaped");
        assert!(!awake.is_active());
        assert_eq!(log.borrow().released, 1);
        // And a long one does not.
        awake.start(AwakeDuration::For(HOUR));
        assert!(!awake.poll());
        assert!(awake.is_active());
    }

    #[test]
    fn timed_to_indefinite_drops_the_deadline() {
        let (mut awake, log) = harness(false);
        awake.start(AwakeDuration::For(HOUR));
        assert!(awake.session.as_ref().unwrap().deadline.is_some());
        awake.start(AwakeDuration::Indefinite);
        assert!(awake.session.as_ref().unwrap().deadline.is_none());
        assert!(awake.view().indefinite);
        // The first assertion was released before the second was taken.
        assert_eq!(log.borrow().released, 1);
        assert_eq!(engaged(&log).len(), 2);
    }

    #[test]
    fn indefinite_to_timed_sets_a_deadline() {
        let (mut awake, log) = harness(false);
        awake.start(AwakeDuration::Indefinite);
        awake.start(AwakeDuration::For(HOUR));
        assert!(awake.session.as_ref().unwrap().deadline.is_some());
        assert!(!awake.view().indefinite);
        assert!(awake.view().status.starts_with("Awake for "));
        assert_eq!(log.borrow().released, 1);
    }

    #[test]
    fn screen_awake_mode_requests_screen_and_system() {
        let (mut awake, log) = harness(false);
        awake.set_mode(AwakeMode::ScreenAndSystem);
        awake.start(AwakeDuration::Indefinite);
        assert_eq!(engaged(&log).last().unwrap().0, AwakeMode::ScreenAndSystem);
        assert!(!AwakeMode::ScreenAndSystem.allows_display_sleep());
        assert_eq!(awake.view().detail.unwrap(), "Screen and system awake");
    }

    #[test]
    fn allow_display_sleep_mode_requests_system_only() {
        let (mut awake, log) = harness(false);
        awake.set_mode(AwakeMode::SystemOnly);
        assert_eq!(engaged(&log).last().unwrap().0, AwakeMode::SystemOnly);
        assert!(AwakeMode::SystemOnly.allows_display_sleep());
        assert_eq!(
            awake.view().detail.unwrap(),
            "System awake - display may sleep"
        );
    }

    #[test]
    fn lid_closed_mode_requests_system_only_plus_the_lid_lever() {
        let (mut awake, log) = harness(true);
        awake.set_mode(AwakeMode::LidClosed);
        assert_eq!(engaged(&log).last().unwrap().0, AwakeMode::LidClosed);
        // Behind a shut lid the panel is off anyway, so this mode lets
        // the display sleep rather than burning it.
        assert!(AwakeMode::LidClosed.allows_display_sleep());
    }

    #[test]
    fn lid_closed_mode_is_refused_where_unsupported() {
        let (mut awake, log) = harness(false);
        awake.set_mode(AwakeMode::LidClosed);
        assert_eq!(awake.mode(), AwakeMode::ScreenAndSystem);
        assert!(!awake.is_active());
        assert!(engaged(&log).is_empty());
        assert!(!awake.view().lid_closed_supported);
    }

    #[test]
    fn switching_mode_releases_the_previous_assertion_and_keeps_indefiniteness() {
        let (mut awake, log) = harness(false);
        awake.start(AwakeDuration::Indefinite);
        awake.set_mode(AwakeMode::SystemOnly);
        assert_eq!(log.borrow().released, 1, "old assertion must be released");
        assert_eq!(
            engaged(&log),
            vec![
                (AwakeMode::ScreenAndSystem, AwakeDuration::Indefinite),
                (AwakeMode::SystemOnly, AwakeDuration::Indefinite),
            ],
            "an indefinite session stays indefinite across a mode change"
        );
        assert!(awake.view().indefinite);
    }

    #[test]
    fn switching_mode_keeps_a_timed_session_timed() {
        let (mut awake, log) = harness(false);
        awake.start(AwakeDuration::For(HOUR));
        awake.set_mode(AwakeMode::SystemOnly);
        let last = *engaged(&log).last().unwrap();
        assert_eq!(last.0, AwakeMode::SystemOnly);
        match last.1 {
            // It keeps only the time it has left, never a fresh hour and
            // never an accidental promotion to indefinite.
            AwakeDuration::For(d) => assert!(d <= HOUR && d > HOUR - Duration::from_secs(5)),
            AwakeDuration::Indefinite => panic!("timed session became indefinite"),
        }
    }

    #[test]
    fn reselecting_the_same_mode_does_not_recreate_the_assertion() {
        let (mut awake, log) = harness(false);
        awake.start(AwakeDuration::Indefinite);
        for _ in 0..10 {
            awake.set_mode(AwakeMode::ScreenAndSystem);
        }
        assert_eq!(engaged(&log).len(), 1);
        assert_eq!(log.borrow().released, 0);
    }

    #[test]
    fn assertion_failure_does_not_mark_the_session_active() {
        let (mut awake, log) = harness(false);
        log.borrow_mut().fail = Some(EngageErr::Assertion);
        awake.start(AwakeDuration::Indefinite);
        assert!(!awake.is_active());
        let view = awake.view();
        assert!(!view.active);
        assert!(!view.indefinite);
        assert_eq!(view.status, "Inactive");
        assert_eq!(view.error, Some(EngageErr::Assertion));
        assert!(view.detail.is_none());
    }

    #[test]
    fn lid_failure_rolls_back_and_names_the_reason() {
        let (mut awake, log) = harness(true);
        log.borrow_mut().fail = Some(EngageErr::Lid(LidBlock::Declined));
        awake.set_mode(AwakeMode::LidClosed);
        assert!(!awake.is_active());
        assert_eq!(awake.view().error, Some(EngageErr::Lid(LidBlock::Declined)));
    }

    #[test]
    fn reselecting_a_mode_that_failed_to_engage_retries() {
        let (mut awake, log) = harness(true);
        log.borrow_mut().fail = Some(EngageErr::Lid(LidBlock::Declined));
        awake.set_mode(AwakeMode::LidClosed);
        assert!(!awake.is_active());
        // The user clicks it again and accepts the prompt this time.
        log.borrow_mut().fail = None;
        awake.set_mode(AwakeMode::LidClosed);
        assert!(awake.is_active());
        assert_eq!(awake.view().error, None);
        assert_eq!(
            engaged(&log),
            vec![(AwakeMode::LidClosed, AwakeDuration::Indefinite)]
        );
    }

    #[test]
    fn a_failed_start_releases_the_session_it_replaced() {
        let (mut awake, log) = harness(false);
        awake.start(AwakeDuration::Indefinite);
        log.borrow_mut().fail = Some(EngageErr::Assertion);
        awake.start(AwakeDuration::For(HOUR));
        assert!(!awake.is_active());
        assert_eq!(log.borrow().released, 1, "no assertion may be left behind");
    }

    #[test]
    fn indefinite_status_carries_no_countdown() {
        let (mut awake, _log) = harness(false);
        awake.start(AwakeDuration::Indefinite);
        let status = awake.view().status;
        assert_eq!(status, "Awake indefinitely");
        assert!(
            !status.chars().any(|c| c.is_ascii_digit()),
            "an indefinite session must never render a number"
        );
    }

    #[test]
    fn restoring_a_preference_never_starts_a_session() {
        let (mut awake, log) = harness(true);
        awake.restore_mode(AwakeMode::LidClosed);
        assert_eq!(awake.mode(), AwakeMode::LidClosed);
        assert!(!awake.is_active(), "relaunch must not resurrect a session");
        assert!(engaged(&log).is_empty());
    }

    #[test]
    fn restoring_an_unsupported_preference_falls_back() {
        let (mut awake, _log) = harness(false);
        awake.restore_mode(AwakeMode::LidClosed);
        assert_eq!(awake.mode(), AwakeMode::ScreenAndSystem);
    }

    #[test]
    fn picking_a_mode_while_idle_starts_an_indefinite_session() {
        let (mut awake, log) = harness(false);
        awake.set_mode(AwakeMode::SystemOnly);
        assert!(awake.is_active());
        assert_eq!(
            engaged(&log),
            vec![(AwakeMode::SystemOnly, AwakeDuration::Indefinite)]
        );
    }

    /// The real macOS backend, not the fake: both non-privileged modes
    /// must actually get an IOKit assertion, and dropping it must release
    /// it. Safe to run anywhere: it needs no privileges, touches no
    /// persistent power setting, and holds the assertion for microseconds.
    #[cfg(target_os = "macos")]
    #[test]
    fn real_iokit_assertions_are_created_and_released() {
        let power = platform::OsPower;
        for mode in [AwakeMode::ScreenAndSystem, AwakeMode::SystemOnly] {
            let lock = power
                .engage(mode, AwakeDuration::Indefinite)
                .unwrap_or_else(|_| panic!("IOKit refused an assertion for {mode:?}"));
            drop(lock);
        }
    }

    #[test]
    fn remaining_label_is_compact() {
        assert_eq!(fmt_remaining(Duration::from_secs(43 * 60)), "43m");
        assert_eq!(fmt_remaining(Duration::from_secs(3900)), "1h05m");
        assert_eq!(fmt_remaining(Duration::from_secs(30)), "<1m");
    }

    // The disablesleep read + restore decisions moved to `lid.rs` (the lid
    // override coordinator) along with the rest of the privileged path, and
    // are unit-tested there.
}
