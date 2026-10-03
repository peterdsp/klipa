//! App-side contract for the Mac App Store "Power Protect" companion.
//!
//! The sandboxed App Store build cannot change system sleep itself, and it
//! cannot reach the direct build's privileged daemon (the sandbox blocks the
//! `/var/run` socket). The only approvable route for bare-laptop closed-lid is
//! the off-store, user-installed Power Protect helper
//! (`packaging/macos/powerprotect/`): a toggle script in the user's
//! Application Scripts directory, authorized by a `pmset`-scoped sudoers rule,
//! plus a per-user LaunchAgent that owns timed restore. See
//! `docs/mas-closed-lid-implementation.md`.
//!
//! This module is the app's half of that contract, kept pure and unit-tested
//! so the Rust side and the shell watchdog cannot drift:
//!   * where the toggle script lives and whether it is installed,
//!   * how to parse the toggle's `SleepDisabled=...` read-back,
//!   * the `powerprotect.deadline` file the watchdog reads to own timed
//!     restore (write the same integer epoch the watchdog expects).
//!
//! Executing the toggle (via `NSUserUnixTask`) and wiring this into the menu
//! are the next increment; they need a real MAS-signed build to verify, so
//! they are intentionally not here yet. Until then these items are not called
//! from the UI.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Bundle identifier, the Application Scripts subdirectory name.
const BUNDLE_ID: &str = "dev.peterdsp.klipa";
/// The toggle script file name (matches `packaging/macos/powerprotect/`).
const TOGGLE_SCRIPT: &str = "klipa-powerprotect";
/// The deadline file the watchdog reads (relative to the data dir).
const DEADLINE_FILE: &str = "powerprotect.deadline";

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

/// Absolute path to the user's Power Protect toggle script, if `HOME` is known.
/// `~/Library/Application Scripts/dev.peterdsp.klipa/klipa-powerprotect` is the
/// one location a sandboxed app may execute a user script from.
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
/// exists). The app offers bare-laptop closed-lid only when this is true, and
/// otherwise points the user at the off-store installer rather than showing a
/// control that would do nothing.
pub fn is_installed() -> bool {
    toggle_script_path().is_some_and(|p| p.exists())
}

/// Parse the single read-back line the toggle prints (`SleepDisabled=1`).
/// Pure, so the app and the shell stay in lockstep.
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

/// Path of the deadline file the watchdog reads, in the shared data dir.
pub fn deadline_path() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join(DEADLINE_FILE))
}

/// Arm (or re-arm) unattended timed restore: write the absolute restore time
/// (epoch seconds) where the watchdog expects it. Returns whether it was
/// written. Indefinite sessions must NOT call this (they are ended explicitly);
/// use [`disarm`] on stop.
pub fn arm_timed(deadline_epoch: u64) -> bool {
    let Some(path) = deadline_path() else {
        return false;
    };
    write_deadline_at(&path, deadline_epoch)
}

/// Clear any armed timed restore (on explicit stop, or when switching to an
/// indefinite session). Absent file is success.
pub fn disarm() -> bool {
    let Some(path) = deadline_path() else {
        return false;
    };
    remove_deadline_at(&path)
}

/// The currently armed deadline, if any. Lets the app show remaining time and
/// reconcile with the watchdog.
pub fn armed_deadline() -> Option<u64> {
    read_deadline_at(&deadline_path()?)
}

// ── Pure file helpers (tested without the real data dir) ──────────────────

/// Write the epoch as plain ASCII digits the watchdog reads with
/// `tr -dc '0-9'`. Atomic via a temp file + rename so the watchdog never reads
/// a half-written value.
fn write_deadline_at(path: &Path, deadline_epoch: u64) -> bool {
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    let tmp = path.with_extension("deadline.tmp");
    if std::fs::write(&tmp, deadline_epoch.to_string().as_bytes()).is_err() {
        return false;
    }
    std::fs::rename(&tmp, path).is_ok()
}

/// Remove the deadline file. Absent is success; only a real removal failure is
/// a failure.
fn remove_deadline_at(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Read the armed deadline, tolerating trailing whitespace/newline exactly as
/// the watchdog's `tr -dc '0-9'` does (digits only).
fn read_deadline_at(path: &Path) -> Option<u64> {
    let raw = std::fs::read_to_string(path).ok()?;
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readback_is_tri_state() {
        assert_eq!(parse_readback("SleepDisabled=1"), Readback::On);
        assert_eq!(parse_readback("SleepDisabled=0"), Readback::Off);
        assert_eq!(parse_readback("SleepDisabled=?"), Readback::Unknown);
        assert_eq!(parse_readback(""), Readback::Unknown);
        assert_eq!(parse_readback("unrelated noise\n"), Readback::Unknown);
        // Real-world: a leading status line then the read-back.
        assert_eq!(
            parse_readback("some pmset chatter\nSleepDisabled=1\n"),
            Readback::On
        );
        // Whitespace tolerance.
        assert_eq!(parse_readback("  SleepDisabled=1  "), Readback::On);
    }

    #[test]
    fn toggle_path_is_the_application_scripts_location() {
        // Set HOME deterministically for the derivation.
        let got = {
            std::env::set_var("HOME", "/Users/tester");
            toggle_script_path()
        };
        assert_eq!(
            got,
            Some(PathBuf::from(
                "/Users/tester/Library/Application Scripts/dev.peterdsp.klipa/klipa-powerprotect"
            ))
        );
    }

    #[test]
    fn deadline_roundtrips_and_matches_the_watchdog_format() {
        let dir = std::env::temp_dir().join(format!("klipa-pp-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(DEADLINE_FILE);

        // Absent reads as None; removing an absent file is success.
        assert_eq!(read_deadline_at(&path), None);
        assert!(remove_deadline_at(&path));

        // Written as plain digits (what the watchdog's `tr -dc '0-9'` reads).
        assert!(write_deadline_at(&path, 1_790_000_000));
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk, "1790000000", "plain ASCII digits, no newline");
        assert_eq!(read_deadline_at(&path), Some(1_790_000_000));

        // Re-arm (extend) overwrites.
        assert!(write_deadline_at(&path, 1_790_000_600));
        assert_eq!(read_deadline_at(&path), Some(1_790_000_600));

        // Disarm removes it; reading then yields None.
        assert!(remove_deadline_at(&path));
        assert_eq!(read_deadline_at(&path), None);

        // A clobbered file with stray bytes still parses the digits (matches
        // the watchdog's digit-only extraction), empty parses as None.
        std::fs::write(&path, "  1790000777\n").unwrap();
        assert_eq!(read_deadline_at(&path), Some(1_790_000_777));
        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(read_deadline_at(&path), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
