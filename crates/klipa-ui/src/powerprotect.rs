//! App-side contract for the Mac App Store "Power Protect" companion.
//!
//! The sandboxed App Store build cannot change system sleep itself. It is NOT,
//! however, blocked by the sandbox from the direct build's `/var/run` socket:
//! a local app-sandbox probe on macOS 27.0 connected to the live 0666 helper
//! socket exactly as an unsandboxed process did, while the same sandbox denied
//! out-of-container user files (see `docs/mas-closed-lid-implementation.md`,
//! "Runtime evidence"). The companion route is chosen instead because the App
//! Store cannot ship or require that root `LaunchDaemon` (guideline 2.4.5), and
//! because a `pmset`-scoped sudoers rule is far less privilege than a root
//! daemon, NOT because of a technical sandbox block.
//! The only approvable route for bare-laptop closed-lid is
//! the off-store, user-installed Power Protect helper
//! (`packaging/macos/powerprotect/`): a toggle script in the user's
//! Application Scripts directory, authorized by a `pmset`-scoped sudoers rule,
//! plus a per-user LaunchAgent that owns timed restore. See
//! `docs/mas-closed-lid-implementation.md`.
//!
//! This module is the app's half of that contract, kept pure and unit-tested
//! so the Rust side and the shell watchdog cannot drift. It owns:
//!   * where the toggle script lives and whether it is installed,
//!   * parsing the toggle's `SleepDisabled=...` read-back (tri-state),
//!   * the shared session stamp the watchdog reads, which records the value
//!     observed BEFORE klipa engaged (so restore returns to the user's prior
//!     setting, never a blind zero) and the timed deadline,
//!   * the pure decisions: what to restore to, and what to do with a stranded
//!     session found at launch.
//!
//! Actually executing the toggle (via `NSUserUnixTask`) and the menu wiring
//! live in `powerprotect_run` and `tray`. A local app-sandbox probe has now
//! confirmed the runtime behavior `NSUserUnixTask` relies on: inside the
//! sandbox it runs the Application-Scripts toggle and captures its stdout, a
//! failing privileged call surfaces as an error (never a false "on"), the
//! app's `~/Library/Application Support` is redirected into its container while
//! the Application Scripts directory is the real path and read-only to the app
//! (so the watchdog, not the app, must own the shared stamp). A
//! profile-backed App Store build should re-confirm under its real
//! application-identifier container; see `docs/mas-closed-lid-implementation.md`.

// Items are wired into the menu and startup incrementally; until the UI
// integration lands, some are exercised only by tests. Keeps clippy green
// during the staged rollout without hiding real unused-code regressions
// elsewhere.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Bundle identifier, the Application Scripts subdirectory name.
const BUNDLE_ID: &str = "dev.peterdsp.klipa";
/// The toggle script file name (matches `packaging/macos/powerprotect/`).
const TOGGLE_SCRIPT: &str = "klipa-powerprotect";
/// The session stamp the watchdog reads (relative to the data dir). Plain
/// `key value` lines so both this module and the POSIX-sh watchdog parse it
/// without a JSON dependency.
const SESSION_FILE: &str = "powerprotect.session";
/// Current stamp schema. Bumped if the line format changes.
const STAMP_SCHEMA: u32 = 1;

/// The effective lid-close override state, parsed from the toggle's read-back.
/// Deliberately tri-state so "unreadable" is never mistaken for "off".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readback {
    /// `SleepDisabled=1`: the override is on.
    On,
    /// `SleepDisabled=0`: normal sleep.
    Off,
    /// Anything else (`?`, missing, malformed): not assertable.
    Unknown,
}

/// The toggle argument to reach a desired state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    On,
    Off,
}

impl Toggle {
    pub fn arg(self) -> &'static str {
        match self {
            Toggle::On => "on",
            Toggle::Off => "off",
        }
    }
}

/// The durable session stamp, shared byte-compatibly with the shell watchdog.
///
/// `prior` is the `disablesleep` value observed before klipa first engaged, so
/// restoration returns to exactly that (if the user already had it on, klipa
/// leaves it on; it never blindly forces 0). `deadline` is `None` for an
/// indefinite session (the watchdog never auto-restores those; the app owns
/// their end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStamp {
    pub active: bool,
    pub prior: Readback,
    pub deadline: Option<u64>,
}

impl SessionStamp {
    /// Serialize to the plain-line format the watchdog reads:
    /// `schema N` / `active 0|1` / `prior 0|1|unknown` / `deadline EPOCH|none`.
    pub fn to_text(self) -> String {
        let prior = match self.prior {
            Readback::On => "1",
            Readback::Off => "0",
            Readback::Unknown => "unknown",
        };
        let deadline = match self.deadline {
            Some(d) => d.to_string(),
            None => "none".to_string(),
        };
        format!(
            "schema {STAMP_SCHEMA}\nactive {}\nprior {prior}\ndeadline {deadline}\n",
            u8::from(self.active),
        )
    }

    /// Parse the stamp. Unknown/missing fields are tolerated conservatively:
    /// a stamp that does not clearly say `active 1` is treated as inactive, and
    /// an unparseable prior is `Unknown` (so restore falls back to "off").
    pub fn parse(text: &str) -> Option<SessionStamp> {
        let mut active = false;
        let mut prior = Readback::Unknown;
        let mut deadline = None;
        let mut saw_any = false;
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(key), Some(val)) = (it.next(), it.next()) else {
                continue;
            };
            saw_any = true;
            match key {
                "active" => active = val == "1",
                "prior" => {
                    prior = match val {
                        "1" => Readback::On,
                        "0" => Readback::Off,
                        _ => Readback::Unknown,
                    }
                }
                "deadline" => {
                    deadline = if val == "none" {
                        None
                    } else {
                        val.chars()
                            .all(|c| c.is_ascii_digit())
                            .then(|| val.parse().ok())
                            .flatten()
                    }
                }
                _ => {}
            }
        }
        saw_any.then_some(SessionStamp {
            active,
            prior,
            deadline,
        })
    }
}

// ── Paths and presence ────────────────────────────────────────────────────

/// Absolute path to the user's Power Protect toggle script, if `HOME` is known.
pub fn toggle_script_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join("Library/Application Scripts")
            .join(BUNDLE_ID)
            .join(TOGGLE_SCRIPT),
    )
}

/// Whether the user has installed the Power Protect helper (the toggle script
/// exists). The app offers bare-laptop closed-lid only when this is true.
pub fn is_installed() -> bool {
    toggle_script_path().is_some_and(|p| p.exists())
}

/// Path of the session stamp the watchdog reads, in the shared data dir.
pub fn session_path() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join(SESSION_FILE))
}

// ── Read-back and pure decisions ──────────────────────────────────────────

/// Parse the single read-back line the toggle prints (`SleepDisabled=1`).
pub fn parse_readback(output: &str) -> Readback {
    for line in output.lines() {
        if let Some(v) = line.trim().strip_prefix("SleepDisabled=") {
            return match v.trim() {
                "1" => Readback::On,
                "0" => Readback::Off,
                _ => Readback::Unknown,
            };
        }
    }
    Readback::Unknown
}

/// What to toggle to RESTORE the prior setting. If the user already had the
/// override on before klipa engaged, restoration leaves it on; otherwise it is
/// turned off. An unknown prior is treated as off (the safe default: normal
/// sleep), matching the direct build's "never strand the override" bias only
/// in the direction of restoring sleep, never of forcing it on.
pub fn restore_toggle(prior: Readback) -> Toggle {
    match prior {
        Readback::On => Toggle::On,
        Readback::Off | Readback::Unknown => Toggle::Off,
    }
}

/// Whether klipa should even manage the override, given the prior state. If the
/// user already had `disablesleep` on, klipa does not own it: it must not later
/// turn it off. The caller still reports "stays awake", but marks it unowned.
pub fn klipa_owns(prior: Readback) -> bool {
    prior != Readback::On
}

/// What to do with a stamp found at app launch (no live in-memory session).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchAction {
    /// Nothing owed (no active stamp).
    None,
    /// A session was stranded (crash/force-quit/reboot): restore to `prior`.
    Restore(Toggle),
}

/// Reconcile a stamp at launch. Because the companion has no always-on daemon,
/// a stamp still marked active at a fresh launch means the previous run did not
/// cleanly end it; restore to the prior setting rather than leave a global
/// override stranded. Klipa-unowned overrides (prior On) are left as the user
/// had them.
pub fn launch_action(stamp: Option<&SessionStamp>) -> LaunchAction {
    match stamp {
        Some(s) if s.active && klipa_owns(s.prior) => {
            LaunchAction::Restore(restore_toggle(s.prior))
        }
        _ => LaunchAction::None,
    }
}

/// Validate a proposed timed deadline against `now`. A deadline must be in the
/// future and within a sane bound (<= 365 days) so a corrupt or hostile value
/// cannot arm a nonsensical restore. Returns the clamped-valid deadline or
/// `None` (treat as indefinite / reject).
pub fn valid_deadline(deadline: u64, now: u64) -> Option<u64> {
    const MAX_AHEAD: u64 = 365 * 24 * 60 * 60;
    (deadline > now && deadline <= now.saturating_add(MAX_AHEAD)).then_some(deadline)
}

// ── Stamp persistence (atomic; tested against a temp dir) ─────────────────

/// Write the session stamp atomically (temp + rename) so the watchdog never
/// reads a half-written stamp.
pub fn write_stamp(stamp: &SessionStamp) -> bool {
    let Some(path) = session_path() else {
        return false;
    };
    write_stamp_at(&path, stamp)
}

/// Read the current stamp, if any.
pub fn read_stamp() -> Option<SessionStamp> {
    read_stamp_at(&session_path()?)
}

/// Clear the stamp (explicit clean end). Absent is success.
pub fn clear_stamp() -> bool {
    let Some(path) = session_path() else {
        return false;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

fn write_stamp_at(path: &Path, stamp: &SessionStamp) -> bool {
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    let tmp = path.with_extension("session.tmp");
    if std::fs::write(&tmp, stamp.to_text().as_bytes()).is_err() {
        return false;
    }
    std::fs::rename(&tmp, path).is_ok()
}

fn read_stamp_at(path: &Path) -> Option<SessionStamp> {
    let text = std::fs::read_to_string(path).ok()?;
    SessionStamp::parse(&text)
}

// ── Execution + session management (injected seams, fully testable) ───────

/// Runs the installed toggle script with `"on"`/`"off"`/`"status"` and returns
/// the parsed read-back. The production impl uses `NSUserUnixTask`
/// (`powerprotect_run`); tests inject a fake. `None` means the script could not
/// be run at all (not installed, or the sandbox/exec failed), which the caller
/// treats as "unavailable", never as "restored".
pub trait ToggleRunner {
    fn run(&self, arg: &str) -> Option<Readback>;
}

/// Where the session stamp lives. Real impl is the shared file; tests use an
/// in-memory store so the full session logic is provable without touching disk
/// or the real data dir.
pub trait StampStore {
    fn read(&self) -> Option<SessionStamp>;
    fn write(&self, stamp: &SessionStamp) -> bool;
    fn clear(&self) -> bool;
}

/// Production stamp store: the shared `powerprotect.session` file.
pub struct FileStampStore;

impl StampStore for FileStampStore {
    fn read(&self) -> Option<SessionStamp> {
        read_stamp()
    }
    fn write(&self, stamp: &SessionStamp) -> bool {
        write_stamp(stamp)
    }
    fn clear(&self) -> bool {
        clear_stamp()
    }
}

/// The outcome of engaging closed-lid keep-awake through the companion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngageOutcome {
    /// `disablesleep=1` confirmed and owned by klipa.
    Active,
    /// Already on before klipa engaged: honored, but not klipa's to clear.
    ActiveUnowned,
    /// The toggle ran but the read-back did not confirm the override.
    Failed,
    /// The toggle could not be run (helper not installed, or exec failed), or
    /// the stamp could not be persisted, so no override was claimed.
    Unavailable,
}

/// The outcome of ending (or retrying the end of) a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndOutcome {
    /// Restored to the prior setting and confirmed by read-back.
    Restored,
    /// Ran the restore but could not confirm it; the stamp is kept for retry.
    RestoreUnconfirmed,
    /// Could not run the toggle; the stamp is kept for retry.
    Unreachable,
}

/// Session manager over the two seams. Mirrors the direct build's
/// "record before you mutate, verify before you forget" discipline: the stamp
/// is written before the flag is turned on, and cleared only after a verified
/// restore.
pub struct SessionManager<R: ToggleRunner, S: StampStore> {
    runner: R,
    store: S,
}

impl<R: ToggleRunner, S: StampStore> SessionManager<R, S> {
    pub fn new(runner: R, store: S) -> Self {
        SessionManager { runner, store }
    }

    /// Engage the override. `requested_deadline` is `Some(epoch)` for a timed
    /// session (validated), `None` for indefinite. The prior value is read once
    /// and preserved across re-engage, so changing duration never loses the
    /// true pre-klipa setting.
    pub fn engage(&self, now: u64, requested_deadline: Option<u64>) -> EngageOutcome {
        let prior = match self.store.read() {
            Some(s) if s.active => s.prior, // keep the original prior across re-engage
            _ => match self.runner.run("status") {
                Some(rb) => rb,
                None => return EngageOutcome::Unavailable,
            },
        };
        let deadline = requested_deadline.and_then(|d| valid_deadline(d, now));
        // Record BEFORE mutating, and refuse if the record cannot be written
        // (never strand a global override with no way to recover it).
        let stamp = SessionStamp {
            active: true,
            prior,
            deadline,
        };
        if !self.store.write(&stamp) {
            return EngageOutcome::Unavailable;
        }
        match self.runner.run("on") {
            None => EngageOutcome::Unavailable,
            Some(Readback::On) if klipa_owns(prior) => EngageOutcome::Active,
            Some(Readback::On) => EngageOutcome::ActiveUnowned,
            Some(_) => EngageOutcome::Failed,
        }
    }

    /// Update the remaining time on a running session in place (duration change
    /// or extension). Returns whether a live session was updated.
    pub fn extend(&self, now: u64, new_deadline: Option<u64>) -> bool {
        let Some(mut stamp) = self.store.read() else {
            return false;
        };
        if !stamp.active {
            return false;
        }
        stamp.deadline = new_deadline.and_then(|d| valid_deadline(d, now));
        self.store.write(&stamp)
    }

    /// End the session: restore the prior setting and clear the stamp only once
    /// a read-back confirms it. An unconfirmed or unreachable restore keeps the
    /// stamp so the next launch (or the watchdog) retries.
    pub fn end(&self) -> EndOutcome {
        let prior = self.store.read().map_or(Readback::Unknown, |s| s.prior);
        let target = restore_toggle(prior);
        match self.runner.run(target.arg()) {
            None => EndOutcome::Unreachable,
            Some(got) if got == desired_readback(target) => {
                self.store.clear();
                EndOutcome::Restored
            }
            Some(_) => EndOutcome::RestoreUnconfirmed,
        }
    }

    /// Whether a timed session is now past its deadline (for the app to end it
    /// while running; the watchdog owns it when the app is gone).
    pub fn due_for_expiry(&self, now: u64) -> bool {
        matches!(self.store.read(), Some(s) if s.active && s.deadline.is_some_and(|d| now >= d))
    }

    /// Reconcile at launch: restore a stranded owned session, or clear a
    /// leftover inactive stamp. Returns what it decided, for logging/UI.
    pub fn reconcile_at_launch(&self) -> LaunchAction {
        let stamp = self.store.read();
        let action = launch_action(stamp.as_ref());
        match action {
            LaunchAction::Restore(target) => {
                if let Some(got) = self.runner.run(target.arg()) {
                    if got == desired_readback(target) {
                        self.store.clear();
                    }
                }
            }
            LaunchAction::None => {
                // Clear only a leftover INACTIVE stamp. An active-but-unowned
                // stamp (the user had the override on before klipa) is left
                // alone: klipa never owned that flag and must not forget or
                // touch it.
                if matches!(&stamp, Some(s) if !s.active) {
                    self.store.clear();
                }
            }
        }
        action
    }
}

/// The read-back that confirms a toggle reached its target.
fn desired_readback(target: Toggle) -> Readback {
    match target {
        Toggle::On => Readback::On,
        Toggle::Off => Readback::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn readback_is_tri_state() {
        assert_eq!(parse_readback("SleepDisabled=1"), Readback::On);
        assert_eq!(parse_readback("SleepDisabled=0"), Readback::Off);
        assert_eq!(parse_readback("SleepDisabled=?"), Readback::Unknown);
        assert_eq!(parse_readback(""), Readback::Unknown);
        assert_eq!(parse_readback("noise\nSleepDisabled=1\n"), Readback::On);
        assert_eq!(parse_readback("  SleepDisabled=1  "), Readback::On);
    }

    #[test]
    fn toggle_path_is_the_application_scripts_location() {
        std::env::set_var("HOME", "/Users/tester");
        assert_eq!(
            toggle_script_path(),
            Some(PathBuf::from(
                "/Users/tester/Library/Application Scripts/dev.peterdsp.klipa/klipa-powerprotect"
            ))
        );
    }

    #[test]
    fn restore_preserves_a_user_preexisting_override() {
        // User already had disablesleep on: klipa does not own it and restore
        // must leave it on, never force it off.
        assert!(!klipa_owns(Readback::On));
        assert_eq!(restore_toggle(Readback::On), Toggle::On);
        // Normal case: klipa owns it, restore turns it off.
        assert!(klipa_owns(Readback::Off));
        assert_eq!(restore_toggle(Readback::Off), Toggle::Off);
        // Unknown prior: restore toward normal sleep, and do not claim ownership
        // of a flag we could not read as off.
        assert_eq!(restore_toggle(Readback::Unknown), Toggle::Off);
    }

    #[test]
    fn launch_reconciles_a_stranded_session_only_when_owned() {
        let owned_active = SessionStamp {
            active: true,
            prior: Readback::Off,
            deadline: Some(10),
        };
        assert_eq!(
            launch_action(Some(&owned_active)),
            LaunchAction::Restore(Toggle::Off),
            "a stranded owned session restores on next launch"
        );
        let inactive = SessionStamp {
            active: false,
            prior: Readback::Off,
            deadline: None,
        };
        assert_eq!(launch_action(Some(&inactive)), LaunchAction::None);
        let unowned = SessionStamp {
            active: true,
            prior: Readback::On,
            deadline: None,
        };
        assert_eq!(
            launch_action(Some(&unowned)),
            LaunchAction::None,
            "a user-preexisting override is left as the user had it"
        );
        assert_eq!(launch_action(None), LaunchAction::None);
    }

    #[test]
    fn deadline_validation_rejects_past_and_absurd() {
        let now = 1_000_000;
        assert_eq!(valid_deadline(now + 300, now), Some(now + 300));
        assert_eq!(valid_deadline(now, now), None, "not strictly future");
        assert_eq!(valid_deadline(now - 1, now), None, "past");
        assert_eq!(
            valid_deadline(now + 400 * 24 * 3600, now),
            None,
            "beyond the 365-day bound"
        );
    }

    #[test]
    fn stamp_roundtrips_and_matches_the_watchdog_line_format() {
        let timed = SessionStamp {
            active: true,
            prior: Readback::Off,
            deadline: Some(1_790_000_600),
        };
        let text = timed.to_text();
        // Exact lines the sh watchdog parses with awk.
        assert_eq!(
            text, "schema 1\nactive 1\nprior 0\ndeadline 1790000600\n",
            "byte-stable format shared with the watchdog"
        );
        assert_eq!(SessionStamp::parse(&text), Some(timed));

        let indefinite = SessionStamp {
            active: true,
            prior: Readback::On,
            deadline: None,
        };
        assert_eq!(
            indefinite.to_text(),
            "schema 1\nactive 1\nprior 1\ndeadline none\n"
        );
        assert_eq!(SessionStamp::parse(&indefinite.to_text()), Some(indefinite));

        // Conservative parsing: empty is None; a stamp without `active 1` is
        // inactive; a garbage deadline is treated as indefinite (no auto-restore).
        assert_eq!(SessionStamp::parse(""), None);
        let got = SessionStamp::parse("schema 1\nprior 0\ndeadline notanumber\n").unwrap();
        assert!(!got.active);
        assert_eq!(got.deadline, None);
    }

    #[test]
    fn stamp_file_roundtrips_atomically() {
        let dir = std::env::temp_dir().join(format!("klipa-pp-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(SESSION_FILE);

        assert_eq!(read_stamp_at(&path), None);
        let stamp = SessionStamp {
            active: true,
            prior: Readback::Off,
            deadline: Some(1_790_000_000),
        };
        assert!(write_stamp_at(&path, &stamp));
        assert_eq!(read_stamp_at(&path), Some(stamp));
        // Overwrite (extend / re-arm).
        let extended = SessionStamp {
            deadline: Some(1_790_000_900),
            ..stamp
        };
        assert!(write_stamp_at(&path, &extended));
        assert_eq!(read_stamp_at(&path), Some(extended));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Session manager (over fake seams) ─────────────────────────────────

    /// A stateful fake toggle: tracks the current flag, records calls, and can
    /// simulate "cannot run" and "ran but did not take".
    struct FakeRunner {
        state: RefCell<Readback>,
        calls: RefCell<Vec<String>>,
        unreachable: bool,
        ignore_writes: bool,
    }
    impl FakeRunner {
        fn new(initial: Readback) -> Self {
            FakeRunner {
                state: RefCell::new(initial),
                calls: RefCell::new(Vec::new()),
                unreachable: false,
                ignore_writes: false,
            }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }
    impl ToggleRunner for FakeRunner {
        fn run(&self, arg: &str) -> Option<Readback> {
            self.calls.borrow_mut().push(arg.to_string());
            if self.unreachable {
                return None;
            }
            if !self.ignore_writes {
                match arg {
                    "on" => *self.state.borrow_mut() = Readback::On,
                    "off" => *self.state.borrow_mut() = Readback::Off,
                    _ => {}
                }
            }
            Some(*self.state.borrow())
        }
    }

    struct MemStore {
        slot: RefCell<Option<SessionStamp>>,
        writable: bool,
    }
    impl MemStore {
        fn empty() -> Self {
            MemStore {
                slot: RefCell::new(None),
                writable: true,
            }
        }
        fn with(stamp: SessionStamp) -> Self {
            MemStore {
                slot: RefCell::new(Some(stamp)),
                writable: true,
            }
        }
    }
    impl StampStore for MemStore {
        fn read(&self) -> Option<SessionStamp> {
            *self.slot.borrow()
        }
        fn write(&self, stamp: &SessionStamp) -> bool {
            if self.writable {
                *self.slot.borrow_mut() = Some(*stamp);
                true
            } else {
                false
            }
        }
        fn clear(&self) -> bool {
            *self.slot.borrow_mut() = None;
            true
        }
    }

    #[test]
    fn engage_records_before_mutating_then_confirms_active() {
        let mgr = SessionManager::new(FakeRunner::new(Readback::Off), MemStore::empty());
        let out = mgr.engage(1_000, Some(1_300));
        assert_eq!(out, EngageOutcome::Active);
        // Read prior via status, then turn on.
        assert_eq!(mgr.runner.calls(), vec!["status", "on"]);
        let s = mgr.store.read().unwrap();
        assert!(s.active && s.prior == Readback::Off && s.deadline == Some(1_300));
    }

    #[test]
    fn engage_preserves_the_true_prior_across_reengage() {
        let mgr = SessionManager::new(FakeRunner::new(Readback::Off), MemStore::empty());
        assert_eq!(mgr.engage(1_000, Some(1_300)), EngageOutcome::Active);
        // Duration change: re-engage. Even though the flag is now On, the prior
        // recorded must stay the original Off (not re-read as On).
        assert_eq!(mgr.engage(1_050, Some(1_900)), EngageOutcome::Active);
        let s = mgr.store.read().unwrap();
        assert_eq!(s.prior, Readback::Off, "original prior preserved");
        assert_eq!(s.deadline, Some(1_900));
        // Second engage did not re-read status (used the existing stamp's prior).
        assert_eq!(mgr.runner.calls(), vec!["status", "on", "on"]);
    }

    #[test]
    fn engage_on_a_user_preexisting_override_is_unowned() {
        let mgr = SessionManager::new(FakeRunner::new(Readback::On), MemStore::empty());
        assert_eq!(mgr.engage(1_000, None), EngageOutcome::ActiveUnowned);
        assert_eq!(mgr.store.read().unwrap().prior, Readback::On);
    }

    #[test]
    fn engage_is_unavailable_when_runner_down_or_stamp_unwritable() {
        let mut r = FakeRunner::new(Readback::Off);
        r.unreachable = true;
        let mgr = SessionManager::new(r, MemStore::empty());
        assert_eq!(mgr.engage(1_000, None), EngageOutcome::Unavailable);
        assert!(mgr.store.read().is_none(), "no override claimed");

        let store = MemStore {
            slot: RefCell::new(None),
            writable: false,
        };
        let mgr2 = SessionManager::new(FakeRunner::new(Readback::Off), store);
        assert_eq!(
            mgr2.engage(1_000, None),
            EngageOutcome::Unavailable,
            "refuses if the stamp cannot be recorded"
        );
    }

    #[test]
    fn end_restores_off_and_clears_when_owned() {
        let mgr = SessionManager::new(FakeRunner::new(Readback::Off), MemStore::empty());
        mgr.engage(1_000, Some(1_300));
        assert_eq!(mgr.end(), EndOutcome::Restored);
        assert!(mgr.store.read().is_none());
        assert_eq!(*mgr.runner.state.borrow(), Readback::Off);
    }

    #[test]
    fn end_on_unowned_leaves_it_on_and_clears_tracking() {
        let mgr = SessionManager::new(FakeRunner::new(Readback::On), MemStore::empty());
        mgr.engage(1_000, None); // unowned
        assert_eq!(mgr.end(), EndOutcome::Restored);
        assert_eq!(
            *mgr.runner.state.borrow(),
            Readback::On,
            "the user's pre-existing override is left on"
        );
        assert!(mgr.store.read().is_none());
    }

    #[test]
    fn end_keeps_the_stamp_when_restore_is_unconfirmed_or_unreachable() {
        // Unconfirmed: toggle runs but the flag does not change.
        let mut r = FakeRunner::new(Readback::On);
        r.ignore_writes = true;
        let mgr = SessionManager::new(
            r,
            MemStore::with(SessionStamp {
                active: true,
                prior: Readback::Off,
                deadline: Some(5),
            }),
        );
        assert_eq!(mgr.end(), EndOutcome::RestoreUnconfirmed);
        assert!(mgr.store.read().is_some(), "kept for retry");

        // Unreachable: toggle cannot run.
        let mut r2 = FakeRunner::new(Readback::On);
        r2.unreachable = true;
        let mgr2 = SessionManager::new(
            r2,
            MemStore::with(SessionStamp {
                active: true,
                prior: Readback::Off,
                deadline: None,
            }),
        );
        assert_eq!(mgr2.end(), EndOutcome::Unreachable);
        assert!(mgr2.store.read().is_some());
    }

    #[test]
    fn extend_updates_a_live_session_only() {
        let mgr = SessionManager::new(FakeRunner::new(Readback::Off), MemStore::empty());
        mgr.engage(1_000, Some(1_300));
        assert!(mgr.extend(1_100, Some(2_000)));
        assert_eq!(mgr.store.read().unwrap().deadline, Some(2_000));
        // Timed -> indefinite.
        assert!(mgr.extend(1_100, None));
        assert_eq!(mgr.store.read().unwrap().deadline, None);
        // No live session: nothing to extend.
        let idle = SessionManager::new(FakeRunner::new(Readback::Off), MemStore::empty());
        assert!(!idle.extend(1_100, Some(2_000)));
    }

    #[test]
    fn due_for_expiry_tracks_the_deadline() {
        let mgr = SessionManager::new(
            FakeRunner::new(Readback::Off),
            MemStore::with(SessionStamp {
                active: true,
                prior: Readback::Off,
                deadline: Some(1_500),
            }),
        );
        assert!(!mgr.due_for_expiry(1_499));
        assert!(mgr.due_for_expiry(1_500));
        assert!(mgr.due_for_expiry(1_600));
    }

    #[test]
    fn reconcile_restores_a_stranded_owned_session() {
        let mgr = SessionManager::new(
            FakeRunner::new(Readback::On),
            MemStore::with(SessionStamp {
                active: true,
                prior: Readback::Off,
                deadline: Some(10),
            }),
        );
        assert_eq!(
            mgr.reconcile_at_launch(),
            LaunchAction::Restore(Toggle::Off)
        );
        assert!(
            mgr.store.read().is_none(),
            "cleared after a confirmed restore"
        );
        assert_eq!(*mgr.runner.state.borrow(), Readback::Off);
    }

    #[test]
    fn reconcile_clears_inactive_but_leaves_unowned_active() {
        // Inactive leftover is cleared.
        let mgr = SessionManager::new(
            FakeRunner::new(Readback::Off),
            MemStore::with(SessionStamp {
                active: false,
                prior: Readback::Off,
                deadline: None,
            }),
        );
        assert_eq!(mgr.reconcile_at_launch(), LaunchAction::None);
        assert!(mgr.store.read().is_none());

        // Active-but-unowned is left untouched (klipa never owned that flag).
        let unowned = SessionManager::new(
            FakeRunner::new(Readback::On),
            MemStore::with(SessionStamp {
                active: true,
                prior: Readback::On,
                deadline: None,
            }),
        );
        assert_eq!(unowned.reconcile_at_launch(), LaunchAction::None);
        assert!(unowned.store.read().is_some(), "unowned session left as is");
    }
}
