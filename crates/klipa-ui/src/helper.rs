//! App-side control of the privileged root helper (macOS direct build).
//!
//! Two responsibilities:
//!   * **Lifecycle** via `SMAppService`: register / unregister / status
//!     (the admin-password-free approval flow the user toggles in System
//!     Settings > Login Items).
//!   * **Transport**: a client for the versioned [`klipa_ipc`] protocol over
//!     the helper's local Unix socket, plus a background renewer thread that
//!     keeps the ownership lease alive independent of the UI event loop.
//!
//! The daemon owns the override, the lease, the recovery journal, and timed
//! expiry (see the `klipa-helper` crate), so the client's job is only to
//! begin a session, keep its lease renewed while the session is healthy,
//! and end it. If this app freezes or dies, the renewer stops, the lease
//! lapses, and the daemon restores normal sleep on its own: a frozen UI can
//! never strand a global override.
//!
//! Compiled only for the non-App-Store macOS build.

use klipa_ipc::{decode_line, encode_line, Effective, ErrorReason, Request, Response};
use smappservice_rs::{AppService, ServiceStatus, ServiceType};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// LaunchDaemon plist bundled at `Contents/Library/LaunchDaemons/`.
const PLIST_NAME: &str = "dev.peterdsp.klipa.helper.plist";

/// Crash-safety lease length. The renewer refreshes well before this, so a
/// healthy session never lapses; a dead or frozen app lets it lapse and the
/// daemon restores within this bound.
const LEASE_SECS: u64 = 60;
/// How often the renewer refreshes the lease (comfortably under the lease).
const RENEW_INTERVAL: Duration = Duration::from_secs(20);
/// Bound on a single request/response exchange.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// What the passwordless helper is currently doing, for the menu.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Never registered, or the feature is unused. Offer one-time setup.
    NotInstalled,
    /// Registered but the user must flip it on in System Settings.
    NeedsApproval,
    /// Registered and enabled: toggles are passwordless.
    Active,
    /// This build ships no daemon to register (a bare `cargo run` with no
    /// app bundle), or the OS predates `SMAppService` (macOS 11/12). The
    /// passwordless helper genuinely cannot run here; the admin-prompt path
    /// still works. A `NotFound` status for a build that DOES ship the plist
    /// is treated as [`State::NotInstalled`] (registerable/repairable), not
    /// this.
    Unavailable,
}

fn service() -> AppService {
    AppService::new(ServiceType::Daemon {
        plist_name: PLIST_NAME,
    })
}

/// `SMAppService` exists only on macOS 13+. On 11/12 the passwordless
/// helper is simply unavailable (the admin-prompt path still works), and we
/// must not call into the missing framework. Cached, since the OS version
/// is fixed for the process lifetime.
pub fn available() -> bool {
    static AVAIL: OnceLock<bool> = OnceLock::new();
    *AVAIL.get_or_init(|| macos_major() >= 13)
}

fn macos_major() -> u32 {
    std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().split('.').next().map(str::to_string))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Derive the bundled LaunchDaemon plist path from the running executable
/// (`<bundle>/Contents/MacOS/<bin>` -> `<bundle>/Contents/Library/LaunchDaemons/<plist>`).
/// Pure, so the derivation is unit-tested without a real bundle.
fn daemon_plist_from_exe(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    // exe = <bundle>/Contents/MacOS/<bin>; parent().parent() = <bundle>/Contents
    let contents = exe.parent()?.parent()?;
    Some(contents.join("Library/LaunchDaemons").join(PLIST_NAME))
}

/// Whether this build actually ships the daemon plist in its bundle. A bare
/// `cargo run` (no .app) or a build without the bundled daemon returns false,
/// so we never offer to "enable" a helper that cannot possibly register.
fn bundled_plist_exists() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| daemon_plist_from_exe(&exe))
        .is_some_and(|p| p.exists())
}

/// Current registration state of the helper daemon.
pub fn state() -> State {
    if !available() {
        return State::Unavailable;
    }
    match service().status() {
        ServiceStatus::Enabled => State::Active,
        ServiceStatus::RequiresApproval => State::NeedsApproval,
        ServiceStatus::NotRegistered => State::NotInstalled,
        // `NotFound` means the OS has no usable record for the daemon: it was
        // never registered, or a stale/foreign registration (for example a
        // prior dev build registered under Xcode, or an upgrade over a
        // sideloaded build) shadows it. When this build actually ships the
        // daemon plist, `register()` is the documented recovery, so surface it
        // as installable/repairable rather than a dead "unavailable" with no
        // action. Only when the plist is genuinely absent (a bare dev build)
        // is it truly unavailable.
        ServiceStatus::NotFound => {
            if bundled_plist_exists() {
                State::NotInstalled
            } else {
                State::Unavailable
            }
        }
    }
}

/// Register the daemon (one-time setup). If macOS then wants the user to
/// approve it, open Login Items so they can flip it on.
pub fn install() {
    if !available() {
        return;
    }
    let svc = service();
    match svc.register() {
        Ok(()) => tracing::info!("klipa helper registered"),
        Err(e) => {
            // A first register can fail when a stale or foreign registration
            // (for example a prior dev build registered under Xcode) shadows
            // the daemon, which the OS reports as NotFound. Repair it: drop
            // the stale record, then register fresh.
            tracing::warn!(error = ?e, "klipa helper register failed; repairing via unregister + register");
            let _ = svc.unregister();
            match svc.register() {
                Ok(()) => tracing::info!("klipa helper registered after repair"),
                Err(e2) => tracing::warn!(error = ?e2, "klipa helper repair register failed"),
            }
        }
    }
    if svc.status() == ServiceStatus::RequiresApproval {
        AppService::open_system_settings_login_items();
    }
}

/// Unregister the daemon, back to plain admin-prompt (Option A) behavior.
pub fn remove() {
    if let Err(e) = service().unregister() {
        tracing::warn!(error = ?e, "klipa helper unregister failed");
    }
}

// ── Protocol client ───────────────────────────────────────────────────

/// Monotonic generation source. Seeded from the wall clock so generations
/// keep increasing across app restarts within a boot, which the daemon's
/// stale-reply check relies on.
fn next_generation() -> u64 {
    static GEN: OnceLock<AtomicU64> = OnceLock::new();
    let counter = GEN.get_or_init(|| {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1);
        AtomicU64::new(seed.max(1))
    });
    counter.fetch_add(1, Ordering::SeqCst)
}

/// Send one request and read one response over a fresh connection. `None`
/// means the daemon could not be reached (not installed/approved/listening);
/// the caller falls back to the admin prompt.
fn request(req: &Request) -> Option<Response> {
    let stream = UnixStream::connect(klipa_ipc::SOCKET_PATH).ok()?;
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let bytes = encode_line(req).ok()?;
    {
        let mut w = &stream;
        w.write_all(&bytes).ok()?;
    }
    let mut line = Vec::new();
    let mut reader = BufReader::new((&stream).take(klipa_ipc::MAX_FRAME_BYTES as u64));
    reader.read_until(b'\n', &mut line).ok()?;
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    decode_line(&line).ok()
}

/// The most recent effective state the daemon reported, updated on every
/// exchange so the UI can read it cheaply without a socket round-trip on the
/// event-loop thread.
fn last_effective_slot() -> &'static Mutex<Option<Effective>> {
    static E: OnceLock<Mutex<Option<Effective>>> = OnceLock::new();
    E.get_or_init(|| Mutex::new(None))
}

fn record_effective(resp: &Response) {
    if let Ok(mut slot) = last_effective_slot().lock() {
        *slot = Some(resp.effective);
    }
}

/// The last effective state the daemon reported, or `None` if we have not
/// heard from it.
pub fn last_effective() -> Option<Effective> {
    last_effective_slot().lock().ok().and_then(|g| *g)
}

/// Read-only effective state from the daemon, if reachable.
pub fn status() -> Option<Response> {
    let resp = request(&Request::Status)?;
    record_effective(&resp);
    Some(resp)
}

/// The authoritative session deadline, shared between the renewer thread and
/// `update_session`. Both read and write this one cell, so an in-place
/// duration change takes effect on the very next renewal instead of being
/// clobbered by a stale copy the thread captured when it started.
///
/// Holds an absolute deadline in epoch seconds, or `None` for an indefinite
/// session. Cheap to clone (it is an `Arc`), and every clone sees the same
/// value.
#[derive(Clone, Default)]
struct SharedDeadline(Arc<Mutex<Option<u64>>>);

impl SharedDeadline {
    fn new(deadline: Option<u64>) -> Self {
        SharedDeadline(Arc::new(Mutex::new(deadline)))
    }

    /// Overwrite the authoritative deadline. The next renewal (thread or
    /// immediate) will carry this value.
    fn set(&self, deadline: Option<u64>) {
        if let Ok(mut g) = self.0.lock() {
            *g = deadline;
        }
    }

    fn get(&self) -> Option<u64> {
        self.0.lock().ok().and_then(|g| *g)
    }

    /// Remaining session seconds to send on a renewal at `now`, from the
    /// current authoritative deadline.
    fn remaining(&self, now: u64) -> Option<u64> {
        remaining_session(self.get(), now)
    }
}

/// Build the `Renew` request to send now, from the authoritative shared
/// deadline. Separated so the "always send the current deadline, never a
/// stale captured copy" rule is unit-tested without a socket.
fn renew_request(generation: u64, deadline: &SharedDeadline, now: u64) -> Request {
    Request::Renew {
        generation,
        lease_secs: LEASE_SECS,
        session_secs: deadline.remaining(now),
    }
}

/// The live renewer: the generation it owns, the shared authoritative
/// deadline, and a stop flag. One at a time.
struct Renewer {
    generation: u64,
    /// Shared with the spawned renewer thread, so `update_session` can change
    /// the remaining time in place and the thread picks it up on its next
    /// tick rather than continuing to send the deadline it started with.
    deadline: SharedDeadline,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

static RENEWER: OnceLock<Mutex<Option<Renewer>>> = OnceLock::new();

/// The generation of the most recent engage, kept so `disengage` can re-send
/// `End` (and retry a restore) even after the renewer has been torn down. `0`
/// means nothing is engaged or owed.
static LAST_GENERATION: AtomicU64 = AtomicU64::new(0);

fn renewer_slot() -> &'static Mutex<Option<Renewer>> {
    RENEWER.get_or_init(|| Mutex::new(None))
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Remaining session seconds from an absolute deadline, or `None` for an
/// indefinite session. Pure, so the countdown math is unit-tested.
pub fn remaining_session(deadline: Option<u64>, now: u64) -> Option<u64> {
    deadline.map(|d| d.saturating_sub(now))
}

/// The outcome of asking the helper to begin (or re-confirm) the override.
pub enum Engage {
    /// The daemon confirmed the override is held and owned by klipa.
    Active,
    /// The daemon answered, but did not (yet) hold the override; the reason
    /// lets the menu show something honest.
    Failed(ErrorReason),
    /// The daemon could not be reached: the caller should fall back to the
    /// admin-prompt path.
    Unreachable,
}

/// Begin (or re-confirm) a lid-closed override through the daemon, owned by
/// a fresh generation, and start the background renewer. `session_secs` is
/// the remaining session time (`None` for indefinite); the daemon owns that
/// expiry independent of this UI.
pub fn engage(session_secs: Option<u64>) -> Engage {
    let generation = next_generation();
    let begin = Request::Begin {
        generation,
        lease_secs: LEASE_SECS,
        session_secs,
    };
    let Some(resp) = request(&begin) else {
        return Engage::Unreachable;
    };
    if let Some(reason) = resp.error {
        return Engage::Failed(reason);
    }
    record_effective(&resp);
    if resp.effective != Effective::DisabledOwned {
        // The daemon answered OK but the system is not actually holding the
        // override (e.g. a race or an unreadable state): do not claim it.
        return Engage::Failed(ErrorReason::SystemRefused);
    }
    LAST_GENERATION.store(generation, Ordering::SeqCst);
    start_renewer(generation, session_secs);
    Engage::Active
}

/// Update the remaining session time on the running override (after a
/// duration change) without retiring it. Writes the new deadline into the
/// authoritative shared cell so the renewer thread carries it from its next
/// tick onward, and pushes it to the daemon immediately for responsiveness.
pub fn update_session(session_secs: Option<u64>) {
    if let Some((generation, deadline)) = set_session_in_slot(session_secs, now_epoch()) {
        // Push the new remaining time to the daemon immediately. The renewer
        // reads the same shared deadline, so even if this request is lost the
        // next renewal still carries the updated value (not a stale one).
        let _ = request(&renew_request(generation, &deadline, now_epoch()));
    }
}

/// Write the new deadline into the live renewer's shared cell and hand back
/// its generation and a clone of that same cell. Separated from the socket
/// I/O so the "update mutates the cell the renewer thread reads" wiring is
/// unit-tested. `None` if there is no live renewer.
fn set_session_in_slot(session_secs: Option<u64>, now: u64) -> Option<(u64, SharedDeadline)> {
    let slot = renewer_slot();
    let mut guard = slot.lock().ok()?;
    let r = guard.as_mut()?;
    r.deadline.set(session_secs.map(|s| now.saturating_add(s)));
    Some((r.generation, r.deadline.clone()))
}

/// The outcome of ending (or retrying the end of) the override.
pub enum Disengage {
    /// Normal sleep is confirmed restored (or there was nothing to restore).
    Confirmed,
    /// The daemon ran the restore but could not confirm it. It keeps the
    /// recovery record and retries on its own; the caller should keep showing
    /// a restoration-failed state and an accessible retry.
    Unconfirmed,
    /// The daemon could not be reached to confirm the restore.
    Unreachable,
}

/// Whether an `End` response confirms the restore completed. A plain OK, or a
/// stale/not-owner rejection (we no longer own the flag, so nothing is owed by
/// us), counts as confirmed; any other error does not.
pub fn restore_confirmed(resp: &Response) -> bool {
    match resp.error {
        None => true,
        Some(ErrorReason::StaleGeneration) | Some(ErrorReason::NotOwner) => true,
        Some(_) => false,
    }
}

/// The generation of the last engage, or `0` if nothing is engaged or owed.
pub fn last_generation() -> u64 {
    LAST_GENERATION.load(Ordering::SeqCst)
}

/// End the override and stop the renewer, reporting whether normal sleep was
/// verified restored. Idempotent, and safe to call again as a retry: it
/// re-sends `End` for the last generation even after the renewer is gone, so a
/// previously-unconfirmed restore can be retried from the UI. On a confirmed
/// restore it forgets the generation so a further call is a no-op.
pub fn disengage() -> Disengage {
    // Stop the renewer if one is live; we are ending, so the lease should be
    // allowed to lapse too (defense in depth behind the explicit End).
    {
        let slot = renewer_slot();
        if let Ok(mut guard) = slot.lock() {
            if let Some(r) = guard.take() {
                r.stop.store(true, Ordering::SeqCst);
            }
        }
    }
    let generation = LAST_GENERATION.load(Ordering::SeqCst);
    if generation == 0 {
        return Disengage::Confirmed; // nothing was ever engaged
    }
    match request(&Request::End { generation }) {
        Some(resp) => {
            record_effective(&resp);
            if restore_confirmed(&resp) {
                LAST_GENERATION.store(0, Ordering::SeqCst);
                Disengage::Confirmed
            } else {
                Disengage::Unconfirmed
            }
        }
        None => Disengage::Unreachable,
    }
}

fn start_renewer(generation: u64, session_secs: Option<u64>) {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let deadline = SharedDeadline::new(session_secs.map(|s| now_epoch().saturating_add(s)));
    // The thread holds its own clone of the SAME shared cell as the renewer
    // stored in the slot, so `update_session` mutating it is visible here.
    let thread_deadline = deadline.clone();
    {
        let slot = renewer_slot();
        if let Ok(mut guard) = slot.lock() {
            // Replace any previous renewer (its stop flag is dropped, but
            // we set it so its thread exits).
            if let Some(old) = guard.take() {
                old.stop.store(true, Ordering::SeqCst);
            }
            *guard = Some(Renewer {
                generation,
                deadline,
                stop: stop.clone(),
            });
        }
    }
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(RENEW_INTERVAL);
            if stop.load(Ordering::SeqCst) {
                break;
            }
            // Read the authoritative deadline fresh each tick so a duration
            // change made via `update_session` is honored immediately rather
            // than overwritten with the value captured at start.
            let resp = request(&renew_request(generation, &thread_deadline, now_epoch()));
            // If the daemon no longer recognizes us (it expired the session
            // on its own, or a newer generation took over), stop renewing.
            match resp {
                Some(r) if r.error.is_some() => {
                    record_effective(&r);
                    break;
                }
                Some(r) => record_effective(&r),
                None => {} // transient unreachable: keep trying until stop
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_session_counts_down_and_floors_at_zero() {
        assert_eq!(remaining_session(Some(1100), 1000), Some(100));
        assert_eq!(
            remaining_session(Some(1000), 1100),
            Some(0),
            "never negative"
        );
        assert_eq!(remaining_session(None, 1000), None, "indefinite stays None");
    }

    /// The session_secs a renewal would carry, for a given shared deadline.
    fn carried(deadline: &SharedDeadline, now: u64) -> Option<u64> {
        match renew_request(1, deadline, now) {
            Request::Renew { session_secs, .. } => session_secs,
            _ => unreachable!("renew_request builds a Renew"),
        }
    }

    #[test]
    fn an_in_place_update_is_honored_by_the_renewers_shared_deadline() {
        // Regression for the stale-deadline bug: the renewer thread must read
        // the authoritative deadline, so a duration change is not reverted on
        // the next lease renewal.
        let now = 1_000;
        let shared = SharedDeadline::new(Some(now + 300));
        // The thread holds a clone of the SAME cell start_renewer stored.
        let thread_view = shared.clone();
        assert_eq!(carried(&thread_view, now), Some(300));

        // Extend.
        shared.set(Some(now + 900));
        assert_eq!(carried(&thread_view, now), Some(900), "extend is seen");

        // Shorten.
        shared.set(Some(now + 60));
        assert_eq!(carried(&thread_view, now), Some(60), "shorten is seen");

        // Timed -> indefinite.
        shared.set(None);
        assert_eq!(
            carried(&thread_view, now),
            None,
            "timed->indefinite is seen"
        );

        // Indefinite -> timed.
        shared.set(Some(now + 120));
        assert_eq!(
            carried(&thread_view, now),
            Some(120),
            "indefinite->timed is seen"
        );
    }

    #[test]
    fn a_renewal_always_carries_the_latest_committed_deadline() {
        // Concurrent renewal/update ordering: a renewal built before an update
        // carries the old value, one built after carries the new, and because
        // the thread re-reads every tick it never sends a value older than the
        // last committed update.
        let now = 2_000;
        let shared = SharedDeadline::new(Some(now + 100));
        let before = carried(&shared, now);
        assert_eq!(before, Some(100));
        shared.set(Some(now + 500));
        let after = carried(&shared, now);
        assert_eq!(after, Some(500));
        assert!(
            after > before,
            "the later read reflects the committed update"
        );
    }

    #[test]
    fn update_mutates_the_cell_the_renewer_thread_reads() {
        // Wire the slot exactly as start_renewer does, capture the thread's
        // clone, then run the update path's slot mutation and confirm the
        // thread's clone sees the new deadline. (Serial: the sole test that
        // touches the process-global renewer slot.)
        let now = 5_000;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let deadline = SharedDeadline::new(Some(now + 100));
        let thread_view = deadline.clone();
        {
            let mut guard = renewer_slot().lock().unwrap();
            *guard = Some(Renewer {
                generation: 7,
                deadline,
                stop: stop.clone(),
            });
        }

        let (generation, handed_back) =
            set_session_in_slot(Some(600), now).expect("a live renewer");
        assert_eq!(generation, 7);
        assert_eq!(
            carried(&thread_view, now),
            Some(600),
            "the renewer thread's own clone sees the update"
        );
        assert_eq!(carried(&handed_back, now), Some(600));

        // Clean up the global so other tests see no live renewer.
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        *renewer_slot().lock().unwrap() = None;
    }

    #[test]
    fn daemon_plist_path_is_derived_from_the_bundle_layout() {
        use std::path::{Path, PathBuf};
        let exe = Path::new("/Applications/klipa.app/Contents/MacOS/klipa");
        assert_eq!(
            daemon_plist_from_exe(exe),
            Some(PathBuf::from(
                "/Applications/klipa.app/Contents/Library/LaunchDaemons/dev.peterdsp.klipa.helper.plist"
            ))
        );
        // A bare binary with no bundle above it yields no plist path, so the
        // "repairable" branch never fires for a non-bundled dev build.
        assert_eq!(daemon_plist_from_exe(Path::new("klipa")), None);
    }

    #[test]
    fn restore_confirmed_reads_the_end_response() {
        let ok = Response {
            helper_version: "x".into(),
            protocol: 1,
            generation: 0,
            effective: Effective::Enabled,
            lease_remaining_secs: None,
            error: None,
            detail: String::new(),
        };
        assert!(restore_confirmed(&ok), "a plain OK confirms the restore");

        let owed = Response::error(ErrorReason::RestoreUnconfirmed, "retrying");
        assert!(
            !restore_confirmed(&owed),
            "an unconfirmed restore is not done"
        );

        // We no longer own the flag: nothing is owed by us.
        let stale = Response::error(ErrorReason::StaleGeneration, "newer owns it");
        assert!(restore_confirmed(&stale));
        let not_owner = Response::error(ErrorReason::NotOwner, "another user");
        assert!(restore_confirmed(&not_owner));
    }
}
