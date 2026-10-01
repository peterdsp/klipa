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
    /// The OS can't find the bundled daemon (unsigned/dev build, or the
    /// app was moved). Treated like "not installed" for the menu.
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

/// Current registration state of the helper daemon.
pub fn state() -> State {
    if !available() {
        return State::Unavailable;
    }
    match service().status() {
        ServiceStatus::Enabled => State::Active,
        ServiceStatus::RequiresApproval => State::NeedsApproval,
        ServiceStatus::NotRegistered => State::NotInstalled,
        ServiceStatus::NotFound => State::Unavailable,
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
        Err(e) => tracing::warn!(error = ?e, "klipa helper register failed"),
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

/// The live renewer: the generation it owns and a stop flag. One at a time.
struct Renewer {
    generation: u64,
    /// Absolute session deadline (epoch secs), or `None` for indefinite.
    session_deadline: Option<u64>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

static RENEWER: OnceLock<Mutex<Option<Renewer>>> = OnceLock::new();

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
    start_renewer(generation, session_secs);
    Engage::Active
}

/// Update the remaining session time on the running override (after a
/// duration change) without retiring it.
pub fn update_session(session_secs: Option<u64>) {
    let slot = renewer_slot();
    if let Ok(mut guard) = slot.lock() {
        if let Some(r) = guard.as_mut() {
            r.session_deadline = session_secs.map(|s| now_epoch().saturating_add(s));
            let generation = r.generation;
            let deadline = r.session_deadline;
            // Push the new remaining time to the daemon immediately.
            drop(guard);
            let _ = request(&Request::Renew {
                generation,
                lease_secs: LEASE_SECS,
                session_secs: remaining_session(deadline, now_epoch()),
            });
        }
    }
}

/// End the override and stop the renewer. Idempotent.
pub fn disengage() {
    let generation = {
        let slot = renewer_slot();
        let mut guard = match slot.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match guard.take() {
            Some(r) => {
                r.stop.store(true, Ordering::SeqCst);
                r.generation
            }
            None => return,
        }
    };
    if let Some(resp) = request(&Request::End { generation }) {
        record_effective(&resp);
    }
}

fn start_renewer(generation: u64, session_secs: Option<u64>) {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let session_deadline = session_secs.map(|s| now_epoch().saturating_add(s));
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
                session_deadline,
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
            let session = remaining_session(session_deadline, now_epoch());
            let resp = request(&Request::Renew {
                generation,
                lease_secs: LEASE_SECS,
                session_secs: session,
            });
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
    use super::remaining_session;

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
}
