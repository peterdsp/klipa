//! The override manager: the one component that owns the lid-closed
//! `disablesleep` transaction from acquisition through verified restoration.
//!
//! It is written over three small seams, [`System`] (the real power flag +
//! clock), [`JournalStore`] (the root-owned durable recovery record), so
//! the whole state machine, the interrupted-transaction recovery, and the
//! lease expiry can be exercised against fakes with no root, no `pmset`,
//! and no real time. Production wires in the OS-backed implementations in
//! `main.rs`.
//!
//! Invariants it enforces:
//!
//! * The durable journal is written *before* the flag is changed, and a
//!   write failure aborts rather than disabling sleep with no way back.
//! * A pre-existing override the daemon did not set is never cleared.
//! * The record is removed only after a restore is *verified*; an
//!   unconfirmed restore keeps it so recovery retries.
//! * Exactly one generation owns the override; a stale generation cannot
//!   disturb a newer session.
//! * The daemon owns expiry: a lapsed lease OR a reached session deadline
//!   restores on its own, independent of any client.

use klipa_ipc::{Effective, ErrorReason, Generation, Response, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};

/// Tri-state read of the system `disablesleep` flag. An unreadable system
/// is `Unknown`, never silently `Enabled`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SleepFlag {
    Disabled,
    Enabled,
    Unknown,
}

/// Parse the `SleepDisabled` value out of `pmset -g` output. Pure, so it
/// is tested directly; a missing or unexpected field is `Unknown`, never
/// silently `Enabled`.
pub fn parse_sleep_flag(pmset_g_stdout: &str) -> SleepFlag {
    for line in pmset_g_stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("SleepDisabled") {
            return match rest.split_whitespace().next() {
                Some("1") => SleepFlag::Disabled,
                Some("0") => SleepFlag::Enabled,
                _ => SleepFlag::Unknown,
            };
        }
    }
    SleepFlag::Unknown
}

/// Outcome of trying to change the flag, kept specific so the client can be
/// told why a change did not take.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SetOutcome {
    /// Changed and verified by readback.
    Applied,
    /// Ran, but the system never reflected the change (e.g. managed policy).
    Refused,
    /// The mechanism could not run at all.
    Unavailable,
}

/// The real power flag and wall clock. Production talks to `pmset`; tests
/// substitute an in-memory fake.
pub trait System {
    fn read_flag(&self) -> SleepFlag;
    /// Set the flag and verify it actually took before returning.
    fn set_flag(&self, on: bool) -> SetOutcome;
    /// Seconds since the Unix epoch. A monotonic-enough wall clock is fine
    /// here because deadlines are coarse (seconds) and must survive a
    /// daemon restart, which a process-local monotonic clock would not.
    fn now_epoch(&self) -> u64;
    /// The current boot session id, so a record from a prior boot (where a
    /// persisted `disablesleep` outlived its owning session) is told apart
    /// from one in this boot.
    fn boot_id(&self) -> String;
}

/// The durable, root-owned recovery record store.
pub trait JournalStore {
    fn load(&self) -> Option<Journal>;
    /// Persist atomically. `false` means it could not be written, which
    /// must abort an acquire rather than proceed unrecoverably.
    fn save(&self, journal: &Journal) -> bool;
    fn clear(&self);
}

/// Where a transaction is, so an interrupted one (daemon crash between
/// steps) is reconciled correctly on restart.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Journal written, flag not yet confirmed set.
    Arming,
    /// Flag confirmed set and owned.
    Active,
    /// Restore in progress / owed.
    Restoring,
}

/// The root-owned recovery journal. Schema-versioned, carries ownership,
/// boot identity, session generation, the exact prior value to restore to,
/// the transaction phase, and the lease/session deadlines, so any
/// interruption leaves enough to finish the job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub schema: u32,
    pub boot_id: String,
    pub generation: Generation,
    pub owner_uid: u32,
    /// `disablesleep` value before klipa changed it: `Some(true)` already
    /// disabled (not ours), `Some(false)` normal (ours to restore), `None`
    /// unreadable at the time (ambiguous ownership).
    pub prior_disabled: Option<bool>,
    pub phase: Phase,
    pub lease_deadline_epoch: u64,
    /// Absolute session deadline; `None` for an indefinite session.
    pub session_deadline_epoch: Option<u64>,
    pub set_at_epoch: u64,
}

pub const JOURNAL_SCHEMA: u32 = 1;

// ── Pure decisions (unit-tested without any OS) ──────────────────────

/// The effective state to report, from the raw flag and whether the daemon
/// currently owns an active override.
pub fn effective_state(flag: SleepFlag, owns_active: bool) -> Effective {
    match flag {
        SleepFlag::Enabled => Effective::Enabled,
        SleepFlag::Unknown => Effective::Unknown,
        SleepFlag::Disabled => {
            if owns_active {
                Effective::DisabledOwned
            } else {
                Effective::DisabledUnowned
            }
        }
    }
}

/// What a `Begin` for `req` should do given the current owner `cur`
/// (`cur == 0` means no owner).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BeginAction {
    /// Older than the current owner: reject.
    Stale,
    /// The same generation re-asking: idempotent, just refresh the lease.
    Idempotent,
    /// A newer generation: take ownership (replacing any older one).
    NewOwner,
}

pub fn begin_action(cur: Generation, req: Generation) -> BeginAction {
    if cur != 0 && req < cur {
        BeginAction::Stale
    } else if req == cur {
        BeginAction::Idempotent
    } else {
        BeginAction::NewOwner
    }
}

/// Whether a `Renew`/`End` for `req` is accepted given the current owner.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OwnerCheck {
    Stale,
    NotOwner,
    Owner,
}

pub fn owner_check(cur: Generation, req: Generation) -> OwnerCheck {
    if cur != 0 && req < cur {
        OwnerCheck::Stale
    } else if req == cur && cur != 0 {
        OwnerCheck::Owner
    } else {
        OwnerCheck::NotOwner
    }
}

/// The prior value to record when acquiring, preserving an existing
/// journal's value so a re-acquire never reclassifies ownership.
pub fn prior_to_record(existing: Option<Option<bool>>, flag: SleepFlag) -> Option<bool> {
    if let Some(prior) = existing {
        return prior;
    }
    match flag {
        SleepFlag::Disabled => Some(true),
        SleepFlag::Enabled => Some(false),
        SleepFlag::Unknown => None,
    }
}

/// What to do with the journal after a restore attempt.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RestoreAction {
    /// Prior was already disabled by something else: leave the flag, drop
    /// the record.
    LeaveFlag,
    /// Confirmed back to enabled: drop the record.
    Clear,
    /// Not confirmed: keep the record so recovery retries.
    Retain,
}

pub fn restore_action(prior_disabled: Option<bool>, readback: SleepFlag) -> RestoreAction {
    match prior_disabled {
        Some(true) => RestoreAction::LeaveFlag,
        Some(false) | None => match readback {
            SleepFlag::Enabled => RestoreAction::Clear,
            SleepFlag::Disabled | SleepFlag::Unknown => RestoreAction::Retain,
        },
    }
}

/// Whether the daemon should restore now, given the clock and the two
/// deadlines. Either a reached session deadline or a lapsed lease triggers
/// it; an indefinite session (no session deadline) is governed only by the
/// lease.
pub fn expiry_due(now_epoch: u64, lease_deadline: u64, session_deadline: Option<u64>) -> bool {
    now_epoch >= lease_deadline || session_deadline.is_some_and(|d| now_epoch >= d)
}

// ── The manager ──────────────────────────────────────────────────────

/// In-memory mirror of the active session. `generation == 0` means idle.
#[derive(Default)]
struct State {
    generation: Generation,
    owner_uid: u32,
    prior_disabled: Option<bool>,
    lease_deadline_epoch: u64,
    session_deadline_epoch: Option<u64>,
    active: bool,
}

pub struct OverrideManager<S: System, J: JournalStore> {
    sys: S,
    journal: J,
    state: State,
}

impl<S: System, J: JournalStore> OverrideManager<S, J> {
    /// Build the manager and reconcile any leftover journal before it
    /// accepts work. This is the boot/restart recovery path.
    pub fn new(sys: S, journal: J) -> Self {
        let mut m = OverrideManager {
            sys,
            journal,
            state: State::default(),
        };
        m.reconcile_on_start();
        m
    }

    fn owns_active(&self) -> bool {
        self.state.active && self.state.generation != 0
    }

    fn ok_response(&self) -> Response {
        let now = self.sys.now_epoch();
        let lease_remaining = if self.owns_active() {
            Some(self.state.lease_deadline_epoch.saturating_sub(now))
        } else {
            None
        };
        Response {
            helper_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: PROTOCOL_VERSION,
            generation: if self.owns_active() {
                self.state.generation
            } else {
                0
            },
            effective: effective_state(self.sys.read_flag(), self.owns_active()),
            lease_remaining_secs: lease_remaining,
            error: None,
            detail: String::new(),
        }
    }

    fn err_response(&self, reason: ErrorReason, detail: &str) -> Response {
        let mut r = Response::error(reason, detail);
        // Even on error, report the real measured state and current owner.
        r.effective = effective_state(self.sys.read_flag(), self.owns_active());
        r.generation = if self.owns_active() {
            self.state.generation
        } else {
            0
        };
        r
    }

    /// Reconcile the durable journal at startup, before any client is
    /// served. Restores a prior-boot or interrupted override, and adopts a
    /// still-valid in-boot session so expiry keeps being enforced until the
    /// client reconnects or the lease lapses.
    fn reconcile_on_start(&mut self) {
        let Some(j) = self.journal.load() else {
            return;
        };
        if j.schema != JOURNAL_SCHEMA {
            // Unknown record: do not act on fields we may misread; restore
            // conservatively and clear.
            self.restore_and_reconcile(None);
            return;
        }
        let current_boot = self.sys.boot_id();
        let interrupted = j.phase != Phase::Active;
        if j.boot_id != current_boot || interrupted {
            // Prior boot (the owning session is gone; disablesleep may have
            // persisted) or an interrupted transaction: finish the restore.
            self.restore_and_reconcile(j.prior_disabled);
            return;
        }
        // Same boot, was active: adopt it so the lease/session expiry keeps
        // running. A client that is still alive will reconnect and renew;
        // a dead one lets the lease lapse and `tick` restores.
        self.state = State {
            generation: j.generation,
            owner_uid: j.owner_uid,
            prior_disabled: j.prior_disabled,
            lease_deadline_epoch: j.lease_deadline_epoch,
            session_deadline_epoch: j.session_deadline_epoch,
            active: true,
        };
    }

    /// Handle one request. The daemon calls this under its mutex.
    pub fn handle(&mut self, req: &klipa_ipc::Request, caller_uid: u32) -> Response {
        use klipa_ipc::Request;
        match *req {
            Request::Hello { protocol } => {
                if protocol != PROTOCOL_VERSION {
                    self.err_response(
                        ErrorReason::ProtocolMismatch,
                        &format!("helper speaks protocol {PROTOCOL_VERSION}"),
                    )
                } else {
                    self.ok_response()
                }
            }
            Request::Status => self.ok_response(),
            Request::Begin {
                generation,
                lease_secs,
                session_secs,
            } => self.begin(generation, lease_secs, session_secs, caller_uid),
            Request::Renew {
                generation,
                lease_secs,
                session_secs,
            } => self.renew(generation, lease_secs, session_secs, caller_uid),
            Request::End { generation } => self.end(generation, caller_uid),
        }
    }

    fn begin(
        &mut self,
        gen: Generation,
        lease_secs: u64,
        session_secs: Option<u64>,
        caller_uid: u32,
    ) -> Response {
        let now = self.sys.now_epoch();
        match begin_action(self.state.generation, gen) {
            BeginAction::Stale => {
                self.err_response(ErrorReason::StaleGeneration, "a newer session owns the override")
            }
            BeginAction::Idempotent if self.owns_active() => {
                // Same generation re-confirming: just refresh deadlines.
                self.refresh_lease(now, lease_secs, session_secs);
                self.ok_response()
            }
            _ => {
                // New owner (or same generation that is not yet active):
                // acquire. Preserve an existing journal's prior value.
                let existing = self.journal.load().and_then(|j| {
                    (j.schema == JOURNAL_SCHEMA).then_some(j.prior_disabled)
                });
                let prior = prior_to_record(existing, self.sys.read_flag());
                let lease_deadline = now.saturating_add(lease_secs);
                let session_deadline = session_secs.map(|s| now.saturating_add(s));
                // Durable record BEFORE mutating the flag.
                let journal = Journal {
                    schema: JOURNAL_SCHEMA,
                    boot_id: self.sys.boot_id(),
                    generation: gen,
                    owner_uid: caller_uid,
                    prior_disabled: prior,
                    phase: Phase::Arming,
                    lease_deadline_epoch: lease_deadline,
                    session_deadline_epoch: session_deadline,
                    set_at_epoch: now,
                };
                if !self.journal.save(&journal) {
                    return self.err_response(
                        ErrorReason::JournalUnwritable,
                        "could not persist recovery record; refusing to disable sleep",
                    );
                }
                match self.sys.set_flag(true) {
                    SetOutcome::Applied => {
                        let active = Journal {
                            phase: Phase::Active,
                            ..journal
                        };
                        // Best effort: if this second write fails the flag
                        // is set with an Arming record, which recovery still
                        // restores safely.
                        let _ = self.journal.save(&active);
                        self.state = State {
                            generation: gen,
                            owner_uid: caller_uid,
                            prior_disabled: prior,
                            lease_deadline_epoch: lease_deadline,
                            session_deadline_epoch: session_deadline,
                            active: true,
                        };
                        self.ok_response()
                    }
                    SetOutcome::Refused => {
                        self.rollback(prior);
                        self.err_response(ErrorReason::SystemRefused, "system did not apply the change")
                    }
                    SetOutcome::Unavailable => {
                        self.rollback(prior);
                        self.err_response(ErrorReason::SystemUnavailable, "could not run the change")
                    }
                }
            }
        }
    }

    fn renew(
        &mut self,
        gen: Generation,
        lease_secs: u64,
        session_secs: Option<u64>,
        caller_uid: u32,
    ) -> Response {
        // A different user must never renew (or by extension prolong)
        // another user's owned session.
        if self.owns_active() && caller_uid != self.state.owner_uid {
            return self.err_response(ErrorReason::NotOwner, "another user owns the override");
        }
        match owner_check(self.state.generation, gen) {
            OwnerCheck::Owner if self.owns_active() => {
                let now = self.sys.now_epoch();
                self.refresh_lease(now, lease_secs, session_secs);
                self.ok_response()
            }
            OwnerCheck::Stale => {
                self.err_response(ErrorReason::StaleGeneration, "a newer session owns the override")
            }
            _ => self.err_response(ErrorReason::NotOwner, "no such active session to renew"),
        }
    }

    fn end(&mut self, gen: Generation, caller_uid: u32) -> Response {
        // Stopping one client's session must not erase another valid
        // owner's protection: only the owning user can end it.
        if self.owns_active() && caller_uid != self.state.owner_uid {
            return self.err_response(ErrorReason::NotOwner, "another user owns the override");
        }
        match owner_check(self.state.generation, gen) {
            OwnerCheck::Owner if self.owns_active() => {
                self.restore_and_reconcile(self.state.prior_disabled);
                self.ok_response()
            }
            OwnerCheck::Stale => {
                self.err_response(ErrorReason::StaleGeneration, "a newer session owns the override")
            }
            // Ending when we own nothing (or an older generation that is
            // already gone) is not an error: the desired end state is
            // already true.
            _ => self.ok_response(),
        }
    }

    fn refresh_lease(&mut self, now: u64, lease_secs: u64, session_secs: Option<u64>) {
        self.state.lease_deadline_epoch = now.saturating_add(lease_secs);
        self.state.session_deadline_epoch = session_secs.map(|s| now.saturating_add(s));
        if let Some(mut j) = self.journal.load() {
            j.lease_deadline_epoch = self.state.lease_deadline_epoch;
            j.session_deadline_epoch = self.state.session_deadline_epoch;
            j.phase = Phase::Active;
            let _ = self.journal.save(&j);
        }
    }

    /// The periodic monitor tick: restore autonomously when the lease has
    /// lapsed or the session deadline is reached. The daemon calls this on
    /// a timer thread, so expiry is owned by the daemon, not any client.
    /// Returns true if it restored.
    pub fn tick(&mut self) -> bool {
        if !self.owns_active() {
            return false;
        }
        let now = self.sys.now_epoch();
        if expiry_due(
            now,
            self.state.lease_deadline_epoch,
            self.state.session_deadline_epoch,
        ) {
            self.restore_and_reconcile(self.state.prior_disabled);
            true
        } else {
            false
        }
    }

    /// Roll back a failed acquire without re-prompting: only touch the flag
    /// if it somehow ended up set; otherwise just drop the record.
    fn rollback(&mut self, prior: Option<bool>) {
        match self.sys.read_flag() {
            SleepFlag::Enabled => {
                self.journal.clear();
                self.state = State::default();
            }
            SleepFlag::Disabled => self.restore_and_reconcile(prior),
            SleepFlag::Unknown => {
                // Ambiguous: keep the journal for startup recovery rather
                // than acting blind.
                self.state = State::default();
            }
        }
    }

    /// Restore the owned override and reconcile the journal against a
    /// verified readback. Clears the record only on a confirmed restore.
    fn restore_and_reconcile(&mut self, prior: Option<bool>) {
        // Mark the phase so an interruption mid-restore is still recoverable.
        if let Some(mut j) = self.journal.load() {
            if j.phase != Phase::Restoring {
                j.phase = Phase::Restoring;
                let _ = self.journal.save(&j);
            }
        }
        if restore_action(prior, SleepFlag::Unknown) == RestoreAction::LeaveFlag {
            // Prior was already disabled by something else: not ours.
            self.journal.clear();
            self.state = State::default();
            return;
        }
        let _ = self.sys.set_flag(false);
        match restore_action(prior, self.sys.read_flag()) {
            RestoreAction::Clear | RestoreAction::LeaveFlag => {
                self.journal.clear();
                self.state = State::default();
            }
            RestoreAction::Retain => {
                // Keep the record; drop the in-memory active flag so we do
                // not claim ownership, but leave the journal for retry.
                self.state.active = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use klipa_ipc::Request;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct FakeSysInner {
        flag: SleepFlag,
        now: u64,
        boot: String,
        /// Forced outcome for the next set(true); None = behave normally.
        set_true_outcome: Option<SetOutcome>,
        /// Forced outcome for the next set(false); a non-`Applied` outcome
        /// leaves the flag `Unknown` to model an unreadable restore.
        set_false_outcome: Option<SetOutcome>,
    }

    #[derive(Clone)]
    struct FakeSys(Rc<RefCell<FakeSysInner>>);
    impl FakeSys {
        fn new() -> Self {
            FakeSys(Rc::new(RefCell::new(FakeSysInner {
                flag: SleepFlag::Enabled,
                now: 1000,
                boot: "boot-A".into(),
                set_true_outcome: None,
                set_false_outcome: None,
            })))
        }
        fn set_now(&self, n: u64) {
            self.0.borrow_mut().now = n;
        }
        fn flag(&self) -> SleepFlag {
            self.0.borrow().flag
        }
        fn force_next_set_true(&self, o: SetOutcome) {
            self.0.borrow_mut().set_true_outcome = Some(o);
        }
        fn force_next_set_false(&self, o: SetOutcome) {
            self.0.borrow_mut().set_false_outcome = Some(o);
        }
        fn set_boot(&self, b: &str) {
            self.0.borrow_mut().boot = b.into();
        }
    }
    impl System for FakeSys {
        fn read_flag(&self) -> SleepFlag {
            self.0.borrow().flag
        }
        fn set_flag(&self, on: bool) -> SetOutcome {
            let mut s = self.0.borrow_mut();
            if on {
                if let Some(o) = s.set_true_outcome.take() {
                    if o == SetOutcome::Applied {
                        s.flag = SleepFlag::Disabled;
                    }
                    return o;
                }
                s.flag = SleepFlag::Disabled;
            } else {
                if let Some(o) = s.set_false_outcome.take() {
                    s.flag = if o == SetOutcome::Applied {
                        SleepFlag::Enabled
                    } else {
                        SleepFlag::Unknown
                    };
                    return o;
                }
                s.flag = SleepFlag::Enabled;
            }
            SetOutcome::Applied
        }
        fn now_epoch(&self) -> u64 {
            self.0.borrow().now
        }
        fn boot_id(&self) -> String {
            self.0.borrow().boot.clone()
        }
    }

    #[derive(Clone, Default)]
    struct FakeJournal {
        rec: Rc<RefCell<Option<Journal>>>,
        /// When true, every save fails (unwritable storage).
        unwritable: Rc<RefCell<bool>>,
    }
    impl FakeJournal {
        fn preset(j: Journal) -> Self {
            FakeJournal {
                rec: Rc::new(RefCell::new(Some(j))),
                unwritable: Rc::new(RefCell::new(false)),
            }
        }
        fn present(&self) -> bool {
            self.rec.borrow().is_some()
        }
    }
    impl JournalStore for FakeJournal {
        fn load(&self) -> Option<Journal> {
            self.rec.borrow().clone()
        }
        fn save(&self, journal: &Journal) -> bool {
            if *self.unwritable.borrow() {
                return false;
            }
            *self.rec.borrow_mut() = Some(journal.clone());
            true
        }
        fn clear(&self) {
            *self.rec.borrow_mut() = None;
        }
    }

    fn mgr(sys: FakeSys, j: FakeJournal) -> OverrideManager<FakeSys, FakeJournal> {
        OverrideManager::new(sys, j)
    }

    fn begin(g: Generation, lease: u64, session: Option<u64>) -> Request {
        Request::Begin {
            generation: g,
            lease_secs: lease,
            session_secs: session,
        }
    }

    #[test]
    fn begin_writes_journal_before_setting_and_reports_owned() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        let r = m.handle(&begin(1, 90, Some(300)), 501);
        assert!(r.is_ok());
        assert_eq!(r.generation, 1);
        assert_eq!(r.effective, Effective::DisabledOwned);
        assert_eq!(sys.flag(), SleepFlag::Disabled);
        assert!(j.present(), "a durable record must exist while active");
    }

    #[test]
    fn a_failed_set_rolls_back_and_names_the_reason() {
        let sys = FakeSys::new();
        sys.force_next_set_true(SetOutcome::Refused);
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        let r = m.handle(&begin(1, 90, None), 501);
        assert_eq!(r.error, Some(ErrorReason::SystemRefused));
        assert_eq!(sys.flag(), SleepFlag::Enabled, "flag must not be left set");
        assert!(!j.present(), "no record should linger after a clean rollback");
    }

    #[test]
    fn an_unwritable_journal_refuses_to_disable_sleep() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        *j.unwritable.borrow_mut() = true;
        let mut m = mgr(sys.clone(), j.clone());
        let r = m.handle(&begin(1, 90, None), 501);
        assert_eq!(r.error, Some(ErrorReason::JournalUnwritable));
        assert_eq!(sys.flag(), SleepFlag::Enabled, "never disabled without a record");
    }

    #[test]
    fn a_preexisting_override_is_not_cleared_on_end() {
        // Flag already disabled by someone else before klipa.
        let sys = FakeSys::new();
        sys.0.borrow_mut().flag = SleepFlag::Disabled;
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        m.handle(&begin(1, 90, None), 501);
        // prior recorded as already-disabled; ending leaves the flag set.
        let r = m.handle(&Request::End { generation: 1 }, 501);
        assert!(r.is_ok());
        assert_eq!(sys.flag(), SleepFlag::Disabled, "someone else's override stays");
        assert!(!j.present());
    }

    #[test]
    fn end_restores_an_owned_override_and_clears_the_record() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        m.handle(&begin(1, 90, None), 501);
        let r = m.handle(&Request::End { generation: 1 }, 501);
        assert!(r.is_ok());
        assert_eq!(r.generation, 0);
        assert_eq!(sys.flag(), SleepFlag::Enabled);
        assert!(!j.present());
    }

    #[test]
    fn a_stale_generation_cannot_disturb_a_newer_session() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        m.handle(&begin(5, 90, None), 501);
        // A delayed op from an older session.
        let r = m.handle(&Request::End { generation: 3 }, 501);
        assert_eq!(r.error, Some(ErrorReason::StaleGeneration));
        assert_eq!(sys.flag(), SleepFlag::Disabled, "newer session still protected");
        let r = m.handle(&begin(2, 90, None), 501);
        assert_eq!(r.error, Some(ErrorReason::StaleGeneration));
    }

    #[test]
    fn reselecting_the_same_generation_is_idempotent() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        m.handle(&begin(1, 90, None), 501);
        let r = m.handle(&begin(1, 120, None), 501);
        assert!(r.is_ok());
        assert_eq!(r.effective, Effective::DisabledOwned);
    }

    #[test]
    fn a_lapsed_lease_restores_autonomously() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        m.handle(&begin(1, 90, None), 501); // now=1000 -> lease@1090
        assert!(!m.tick(), "not due yet");
        sys.set_now(1091);
        assert!(m.tick(), "lease lapsed -> restore");
        assert_eq!(sys.flag(), SleepFlag::Enabled);
        assert!(!j.present());
    }

    #[test]
    fn the_daemon_owns_timed_session_expiry() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        // Session of 60s, lease of 300s: the session deadline is sooner and
        // the daemon honors it even though the lease is fine (UI could be
        // blocked on a modal and not renewing).
        m.handle(&begin(1, 300, Some(60)), 501); // now=1000 -> session@1060
        sys.set_now(1061);
        assert!(m.tick(), "session deadline reached -> restore");
        assert_eq!(sys.flag(), SleepFlag::Enabled);
    }

    #[test]
    fn renewing_extends_the_lease_so_a_healthy_session_persists() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        m.handle(&begin(1, 90, None), 501); // lease@1090
        sys.set_now(1080);
        m.handle(
            &Request::Renew {
                generation: 1,
                lease_secs: 90,
                session_secs: None,
            },
            501,
        ); // lease@1170
        sys.set_now(1095);
        assert!(!m.tick(), "renew pushed the lease out; not due");
        assert_eq!(sys.flag(), SleepFlag::Disabled);
    }

    #[test]
    fn a_restore_that_cannot_be_confirmed_retains_the_record() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        m.handle(&begin(1, 90, None), 501);
        // Lease lapses, but the restore cannot be confirmed (pmset runs yet
        // the readback is unreadable): the record must be kept for retry.
        sys.set_now(1091);
        sys.force_next_set_false(SetOutcome::Refused);
        let restored = m.tick();
        assert!(restored, "tick acted on the lapsed lease");
        assert!(j.present(), "an unconfirmed restore keeps the record for retry");
    }

    #[test]
    fn startup_reconciles_an_interrupted_arming_transaction() {
        let sys = FakeSys::new();
        sys.0.borrow_mut().flag = SleepFlag::Disabled; // flag was left set
        let j = FakeJournal::preset(Journal {
            schema: JOURNAL_SCHEMA,
            boot_id: "boot-A".into(),
            generation: 1,
            owner_uid: 501,
            prior_disabled: Some(false), // we owned it
            phase: Phase::Arming,        // interrupted before confirm
            lease_deadline_epoch: 2000,
            session_deadline_epoch: None,
            set_at_epoch: 1000,
        });
        let _m = mgr(sys.clone(), j.clone());
        // Interrupted arming => restore on start.
        assert_eq!(sys.flag(), SleepFlag::Enabled);
        assert!(!j.present());
    }

    #[test]
    fn startup_restores_an_override_left_from_a_prior_boot() {
        let sys = FakeSys::new();
        sys.0.borrow_mut().flag = SleepFlag::Disabled;
        sys.set_boot("boot-B"); // current boot differs from the record
        let j = FakeJournal::preset(Journal {
            schema: JOURNAL_SCHEMA,
            boot_id: "boot-A".into(),
            generation: 1,
            owner_uid: 501,
            prior_disabled: Some(false),
            phase: Phase::Active,
            lease_deadline_epoch: 2000,
            session_deadline_epoch: None,
            set_at_epoch: 1000,
        });
        let _m = mgr(sys.clone(), j.clone());
        assert_eq!(sys.flag(), SleepFlag::Enabled, "no auto-resume across reboot");
        assert!(!j.present());
    }

    #[test]
    fn startup_adopts_a_still_valid_in_boot_session() {
        let sys = FakeSys::new();
        sys.0.borrow_mut().flag = SleepFlag::Disabled;
        let j = FakeJournal::preset(Journal {
            schema: JOURNAL_SCHEMA,
            boot_id: "boot-A".into(),
            generation: 9,
            owner_uid: 501,
            prior_disabled: Some(false),
            phase: Phase::Active,
            lease_deadline_epoch: 2000, // now=1000, not lapsed
            session_deadline_epoch: None,
            set_at_epoch: 1000,
        });
        let mut m = mgr(sys.clone(), j.clone());
        // Adopted: still protected, and a Status reports owned by gen 9.
        let r = m.handle(&Request::Status, 501);
        assert_eq!(r.generation, 9);
        assert_eq!(r.effective, Effective::DisabledOwned);
        // And the adopted lease is still enforced: lapse it and tick.
        sys.set_now(2001);
        assert!(m.tick());
        assert_eq!(sys.flag(), SleepFlag::Enabled);
    }

    #[test]
    fn protocol_mismatch_is_reported() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys, j);
        let r = m.handle(&Request::Hello { protocol: 999 }, 501);
        assert_eq!(r.error, Some(ErrorReason::ProtocolMismatch));
    }

    #[test]
    fn another_user_cannot_end_an_owners_session() {
        let sys = FakeSys::new();
        let j = FakeJournal::default();
        let mut m = mgr(sys.clone(), j.clone());
        m.handle(&begin(1, 90, None), 501); // user A owns it
        let r = m.handle(&Request::End { generation: 1 }, 502); // user B
        assert_eq!(r.error, Some(ErrorReason::NotOwner));
        assert_eq!(sys.flag(), SleepFlag::Disabled, "owner's protection survives");
        // The real owner can still end it.
        let r = m.handle(&Request::End { generation: 1 }, 501);
        assert!(r.is_ok());
        assert_eq!(sys.flag(), SleepFlag::Enabled);
    }

    #[test]
    fn parse_sleep_flag_reads_the_three_states() {
        assert_eq!(parse_sleep_flag(" SleepDisabled\t\t0\n"), SleepFlag::Enabled);
        assert_eq!(parse_sleep_flag(" SleepDisabled          1\n"), SleepFlag::Disabled);
        assert_eq!(parse_sleep_flag(" hibernatemode 3\n"), SleepFlag::Unknown);
        assert_eq!(parse_sleep_flag("SleepDisabled  ?\n"), SleepFlag::Unknown);
        assert_eq!(parse_sleep_flag(""), SleepFlag::Unknown);
    }

    #[test]
    fn status_reports_an_unowned_external_override() {
        let sys = FakeSys::new();
        sys.0.borrow_mut().flag = SleepFlag::Disabled; // external
        let j = FakeJournal::default();
        let mut m = mgr(sys, j);
        let r = m.handle(&Request::Status, 501);
        assert_eq!(r.effective, Effective::DisabledUnowned);
        assert_eq!(r.generation, 0);
    }
}
