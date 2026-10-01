//! macOS lid-closed override coordinator.
//!
//! The lid-closed mode needs the system `disablesleep` flag, which no
//! power assertion can replace. This module owns that override end to end,
//! choosing between two mechanisms and keeping the UI honest about which is
//! in effect:
//!
//! * **Helper (preferred).** When the privileged daemon is installed and
//!   approved, the override is set through the authenticated, versioned
//!   protocol in `helper.rs`. The daemon owns the recovery journal, the
//!   lease, and timed expiry, so a frozen or dead UI cannot strand it.
//! * **Admin prompt (fallback).** Before the helper is set up (or on macOS
//!   11/12, where `SMAppService` does not exist), the override is set via a
//!   one-off `osascript` admin prompt, with an app-side recovery journal so
//!   an unclean exit is still restored on the next launch.
//!
//! It is driven by explicit edges from the composition root
//! ([`LidOverride::begin`] / [`update_session`](LidOverride::update_session)
//! / [`end`](LidOverride::end)), never a per-tick retry, so a failed start
//! never becomes a prompt loop and a duration change never drops the
//! override (gap-free): the helper session is simply updated in place.
//!
//! Only the lid-capable macOS build ever engages this; the pure decision
//! helpers and the admin-prompt path still compile everywhere macOS does.

use crate::awake::{LidBlock, ProtectionState};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Which mechanism currently holds the override.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mechanism {
    Helper,
    Prompt,
}

/// Tri-state read of the system `disablesleep` flag (admin-prompt path).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SleepFlag {
    Disabled,
    Enabled,
    Unknown,
}

/// Parse the `SleepDisabled` value out of `pmset -g` output. Pure, tested.
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

/// What to do with the app-side journal after a restore attempt.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RestoreStep {
    LeaveFlag,
    Clear,
    Retain,
}

pub fn restore_step(prior_disabled: Option<bool>, readback: SleepFlag) -> RestoreStep {
    match prior_disabled {
        Some(true) => RestoreStep::LeaveFlag,
        Some(false) | None => match readback {
            SleepFlag::Enabled => RestoreStep::Clear,
            SleepFlag::Disabled | SleepFlag::Unknown => RestoreStep::Retain,
        },
    }
}

pub fn prior_to_record(existing: Option<Option<bool>>, readback: SleepFlag) -> Option<bool> {
    if let Some(prior) = existing {
        return prior;
    }
    match readback {
        SleepFlag::Disabled => Some(true),
        SleepFlag::Enabled => Some(false),
        SleepFlag::Unknown => None,
    }
}

/// Map a daemon error reason to the UI-facing lid block cause.
pub fn map_reason(reason: klipa_ipc::ErrorReason) -> LidBlock {
    use klipa_ipc::ErrorReason::*;
    match reason {
        SystemRefused => LidBlock::Refused,
        _ => LidBlock::Unavailable,
    }
}

/// The honest protection state for the lid-closed mode, from whether a
/// session is wanted, whether the override is engaged, the measured
/// effective state, and the helper's approval state. Pure, tested.
pub fn classify(
    desired: bool,
    engaged: bool,
    effective: Option<Effective>,
    helper_needs_approval: bool,
    last_error: Option<LidBlock>,
) -> ProtectionState {
    if !desired {
        return ProtectionState::Inactive;
    }
    if let Some(err) = last_error {
        return match err {
            // A cancelled or missing approval is recoverable by the user.
            LidBlock::Declined => ProtectionState::AwaitingApproval,
            LidBlock::Refused | LidBlock::Unavailable => ProtectionState::RestorationFailed,
        };
    }
    if helper_needs_approval && !engaged {
        return ProtectionState::AwaitingApproval;
    }
    if !engaged {
        return ProtectionState::Preparing;
    }
    match effective {
        Some(Effective::DisabledOwned) => ProtectionState::Active,
        Some(Effective::Enabled) => ProtectionState::DegradedRecovering,
        Some(Effective::DisabledUnowned) => ProtectionState::Active,
        Some(Effective::Unknown) | None => ProtectionState::Verifying,
    }
}

// Re-export the protocol's effective state so callers do not depend on
// klipa-ipc directly for this.
pub use klipa_ipc::Effective;

/// The coordinator. Holds which mechanism (if any) currently owns the
/// override and the last specific failure, so the menu can report it.
#[derive(Default)]
pub struct LidOverride {
    engaged: Option<Mechanism>,
    last_error: Option<LidBlock>,
}

impl LidOverride {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_engaged(&self) -> bool {
        self.engaged.is_some()
    }

    pub fn last_error(&self) -> Option<LidBlock> {
        self.last_error
    }

    /// The measured effective state (helper path reports it; the prompt
    /// path reads the flag directly).
    pub fn effective(&self) -> Option<Effective> {
        match self.engaged {
            Some(Mechanism::Helper) => crate::helper::last_effective(),
            Some(Mechanism::Prompt) => Some(match read_sleep_flag() {
                SleepFlag::Disabled => Effective::DisabledOwned,
                SleepFlag::Enabled => Effective::Enabled,
                SleepFlag::Unknown => Effective::Unknown,
            }),
            None => None,
        }
    }

    /// Begin the override for a session with `session_secs` remaining
    /// (`None` = indefinite). Prefers the helper; falls back to the admin
    /// prompt only when the daemon is unreachable (not when it explicitly
    /// refused, since the prompt would hit the same wall). A no-op if
    /// already engaged.
    pub fn begin(&mut self, session_secs: Option<u64>) {
        if self.engaged.is_some() {
            self.update_session(session_secs);
            return;
        }
        self.last_error = None;
        match crate::helper::engage(session_secs) {
            crate::helper::Engage::Active => {
                self.engaged = Some(Mechanism::Helper);
                return;
            }
            crate::helper::Engage::Failed(reason) => {
                self.last_error = Some(map_reason(reason));
                return;
            }
            crate::helper::Engage::Unreachable => {} // fall back to prompt
        }
        match prompt_begin() {
            Ok(()) => {
                self.engaged = Some(Mechanism::Prompt);
                self.last_error = None;
            }
            Err(block) => self.last_error = Some(block),
        }
    }

    /// Update the remaining session time in place, without retiring the
    /// override (gap-free duration change). Only meaningful on the helper
    /// path; the prompt path's flag is sticky and needs nothing.
    pub fn update_session(&mut self, session_secs: Option<u64>) {
        if let Some(Mechanism::Helper) = self.engaged {
            crate::helper::update_session(session_secs);
        }
    }

    /// End the override and restore normal sleep.
    pub fn end(&mut self) {
        match self.engaged.take() {
            Some(Mechanism::Helper) => crate::helper::disengage(),
            Some(Mechanism::Prompt) => prompt_restore(),
            None => {}
        }
        self.last_error = None;
    }

    /// The protection state to show, given whether a lid session is wanted
    /// and the helper's approval state.
    pub fn protection(&self, desired: bool, helper_needs_approval: bool) -> ProtectionState {
        classify(
            desired,
            self.is_engaged(),
            self.effective(),
            helper_needs_approval,
            self.last_error,
        )
    }
}

// ── Process-global coordinator + the free functions the root calls ────

use std::sync::{Mutex, OnceLock};

fn global() -> &'static Mutex<LidOverride> {
    static L: OnceLock<Mutex<LidOverride>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(LidOverride::new()))
}

/// Begin (or update) the lid override for a session with `session_secs`
/// remaining. Safe to call on a false->true edge.
pub fn begin(session_secs: Option<u64>) {
    if let Ok(mut g) = global().lock() {
        g.begin(session_secs);
    }
}

/// Update the remaining session time in place (gap-free duration change).
pub fn update_session(session_secs: Option<u64>) {
    if let Ok(mut g) = global().lock() {
        g.update_session(session_secs);
    }
}

/// End the lid override and restore normal sleep.
pub fn end() {
    if let Ok(mut g) = global().lock() {
        g.end();
    }
}

pub fn is_engaged() -> bool {
    global().lock().map(|g| g.is_engaged()).unwrap_or(false)
}

/// The protection state to render, given whether a lid session is wanted
/// and the helper's approval state.
pub fn protection(desired: bool, helper_needs_approval: bool) -> ProtectionState {
    global()
        .lock()
        .map(|g| g.protection(desired, helper_needs_approval))
        .unwrap_or(ProtectionState::Inactive)
}

pub fn last_error() -> Option<LidBlock> {
    global().lock().ok().and_then(|g| g.last_error())
}

// ── Admin-prompt (Option A) mechanism + app-side recovery journal ─────

/// Read the `disablesleep` flag (no privileges needed).
pub fn read_sleep_flag() -> SleepFlag {
    match std::process::Command::new("/usr/bin/pmset")
        .arg("-g")
        .output()
    {
        Ok(out) if out.status.success() => parse_sleep_flag(&String::from_utf8_lossy(&out.stdout)),
        _ => SleepFlag::Unknown,
    }
}

fn wait_for_sleep_flag(want: SleepFlag) -> bool {
    for _ in 0..20 {
        if read_sleep_flag() == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    read_sleep_flag() == want
}

/// Flip `disablesleep` through the OS admin-authentication dialog. klipa
/// never sees the password.
fn set_disablesleep_prompt(on: bool) -> PromptResult {
    let val = if on { "1" } else { "0" };
    let script = format!(
        "do shell script \"/usr/bin/pmset -a disablesleep {val}\" \
         with administrator privileges"
    );
    match std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
    {
        Ok(out) if out.status.success() => PromptResult::Ok,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("-128") || stderr.contains("User canceled") {
                PromptResult::Declined
            } else {
                tracing::warn!(%stderr, "pmset via osascript failed");
                PromptResult::Unavailable
            }
        }
        Err(e) => {
            tracing::warn!(?e, "pmset via osascript failed");
            PromptResult::Unavailable
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum PromptResult {
    Ok,
    Declined,
    Unavailable,
}

/// Begin the override via the admin prompt, recording a durable app-side
/// journal before mutating the flag.
fn prompt_begin() -> Result<(), LidBlock> {
    let existing = read_journal().map(|j| j.prior_disabled);
    let prior = prior_to_record(existing, read_sleep_flag());
    if !write_journal(prior) {
        return Err(LidBlock::Unavailable);
    }
    let prompt = set_disablesleep_prompt(true);
    if wait_for_sleep_flag(SleepFlag::Disabled) {
        return Ok(());
    }
    // Did not take: roll back the record (without re-prompting) and report.
    rollback_prompt(prior);
    Err(match prompt {
        PromptResult::Declined => LidBlock::Declined,
        PromptResult::Unavailable => LidBlock::Unavailable,
        PromptResult::Ok => LidBlock::Refused,
    })
}

/// Restore the admin-prompt override and reconcile the app journal.
fn prompt_restore() {
    let prior = read_journal()
        .map(|j| j.prior_disabled)
        .unwrap_or(Some(false));
    restore_owned(prior);
}

fn rollback_prompt(prior: Option<bool>) {
    match read_sleep_flag() {
        SleepFlag::Enabled => {
            remove_journal();
            remove_legacy_marker();
        }
        SleepFlag::Disabled => restore_owned(prior),
        SleepFlag::Unknown => {
            tracing::warn!("lid engage failed with unreadable sleep state; keeping journal");
        }
    }
}

fn restore_owned(prior_disabled: Option<bool>) {
    if restore_step(prior_disabled, SleepFlag::Unknown) == RestoreStep::LeaveFlag {
        remove_journal();
        remove_legacy_marker();
        return;
    }
    let _ = set_disablesleep_prompt(false);
    match restore_step(prior_disabled, read_sleep_flag()) {
        RestoreStep::Clear | RestoreStep::LeaveFlag => {
            remove_journal();
            remove_legacy_marker();
        }
        RestoreStep::Retain => {
            tracing::warn!("could not confirm sleep restore; keeping journal for retry");
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Journal {
    schema: u32,
    prior_disabled: Option<bool>,
    boot_id: String,
}

const JOURNAL_SCHEMA: u32 = 1;

fn boot_id() -> String {
    std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.bootsessionuuid"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn write_journal(prior_disabled: Option<bool>) -> bool {
    let Some(path) = crate::paths::lid_awake_journal() else {
        return false;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let record = Journal {
        schema: JOURNAL_SCHEMA,
        prior_disabled,
        boot_id: boot_id(),
    };
    match serde_json::to_vec(&record) {
        Ok(bytes) => std::fs::write(&path, bytes).is_ok(),
        Err(_) => false,
    }
}

fn read_journal() -> Option<Journal> {
    let path = crate::paths::lid_awake_journal()?;
    let bytes = std::fs::read(&path).ok()?;
    let record: Journal = serde_json::from_slice(&bytes).ok()?;
    (record.schema == JOURNAL_SCHEMA).then_some(record)
}

fn remove_journal() {
    if let Some(path) = crate::paths::lid_awake_journal() {
        let _ = std::fs::remove_file(path);
    }
}

fn remove_legacy_marker() {
    if let Some(path) = crate::paths::lid_awake_marker() {
        let _ = std::fs::remove_file(path);
    }
}

/// Restore any leftover admin-prompt override at startup: the app journal
/// from this build, or a legacy `lid_awake.on` marker from klipa <= 0.5.4.
/// The helper path's root journal is reconciled by the daemon itself.
pub fn recover() {
    if let Some(record) = read_journal() {
        match read_sleep_flag() {
            SleepFlag::Enabled => {
                remove_journal();
                remove_legacy_marker();
            }
            SleepFlag::Disabled | SleepFlag::Unknown => {
                tracing::warn!("admin-prompt lid override outlived its session; restoring");
                restore_owned(record.prior_disabled);
            }
        }
        return;
    }
    if crate::paths::lid_awake_marker().is_some_and(|p| p.exists()) {
        match read_sleep_flag() {
            SleepFlag::Enabled => remove_legacy_marker(),
            SleepFlag::Disabled | SleepFlag::Unknown => {
                tracing::warn!("legacy lid-awake marker present; restoring normal sleep");
                restore_owned(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sleep_flag_reads_the_three_states() {
        assert_eq!(
            parse_sleep_flag(" SleepDisabled\t\t0\n"),
            SleepFlag::Enabled
        );
        assert_eq!(
            parse_sleep_flag(" SleepDisabled   1\n"),
            SleepFlag::Disabled
        );
        assert_eq!(parse_sleep_flag(" hibernatemode 3\n"), SleepFlag::Unknown);
        assert_eq!(parse_sleep_flag("SleepDisabled ?\n"), SleepFlag::Unknown);
        assert_eq!(parse_sleep_flag(""), SleepFlag::Unknown);
    }

    #[test]
    fn restore_step_never_clears_without_confirmation() {
        for rb in [SleepFlag::Enabled, SleepFlag::Disabled, SleepFlag::Unknown] {
            assert_eq!(restore_step(Some(true), rb), RestoreStep::LeaveFlag);
        }
        assert_eq!(
            restore_step(Some(false), SleepFlag::Enabled),
            RestoreStep::Clear
        );
        assert_eq!(
            restore_step(Some(false), SleepFlag::Disabled),
            RestoreStep::Retain
        );
        assert_eq!(restore_step(None, SleepFlag::Unknown), RestoreStep::Retain);
    }

    #[test]
    fn prior_to_record_prefers_an_existing_journal() {
        assert_eq!(
            prior_to_record(Some(Some(false)), SleepFlag::Disabled),
            Some(false)
        );
        assert_eq!(prior_to_record(None, SleepFlag::Enabled), Some(false));
        assert_eq!(prior_to_record(None, SleepFlag::Unknown), None);
    }

    #[test]
    fn protection_classifies_the_lifecycle() {
        // Not wanted.
        assert_eq!(
            classify(false, false, None, false, None),
            ProtectionState::Inactive
        );
        // Wanted, waiting on approval.
        assert_eq!(
            classify(true, false, None, true, None),
            ProtectionState::AwaitingApproval
        );
        // Wanted, engaging.
        assert_eq!(
            classify(true, false, None, false, None),
            ProtectionState::Preparing
        );
        // Engaged and verified.
        assert_eq!(
            classify(true, true, Some(Effective::DisabledOwned), false, None),
            ProtectionState::Active
        );
        // Engaged but the flag is somehow off again: degraded.
        assert_eq!(
            classify(true, true, Some(Effective::Enabled), false, None),
            ProtectionState::DegradedRecovering
        );
        // A hard failure surfaces as restoration-failed / needs attention.
        assert_eq!(
            classify(true, false, None, false, Some(LidBlock::Refused)),
            ProtectionState::RestorationFailed
        );
        assert_eq!(
            classify(true, false, None, false, Some(LidBlock::Declined)),
            ProtectionState::AwaitingApproval
        );
    }
}
