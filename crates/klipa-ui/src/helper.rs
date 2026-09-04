//! App-side control of the privileged root helper (macOS direct build).
//!
//! This is the "Option B" upgrade over the plain admin-prompt path in
//! `awake.rs`: once the user installs and approves the helper, klipa can
//! keep the Mac awake with the lid closed (and restore sleep afterwards)
//! with **no password prompt at all**, because the root work happens in
//! the launchd-managed `klipa-helper` daemon.
//!
//! Two responsibilities live here:
//!   * lifecycle, register / unregister / status via `SMAppService`
//!     (the modern, admin-password-free approval flow, user toggles it in
//!     System Settings > Login Items);
//!   * transport, a one-line request over the helper's local Unix socket.
//!
//! Compiled only for the non-App-Store macOS build. The sandbox forbids
//! privileged helpers, so the App Store build never sees this module.

use smappservice_rs::{AppService, ServiceStatus, ServiceType};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::OnceLock;
use std::time::Duration;

/// LaunchDaemon plist bundled at `Contents/Library/LaunchDaemons/`.
const PLIST_NAME: &str = "dev.peterdsp.klipa.helper.plist";
/// Must match `SOCKET_PATH` in the `klipa-helper` crate.
const HELPER_SOCKET: &str = "/var/run/dev.peterdsp.klipa.helper.sock";

/// What the passwordless helper is currently doing, for the menu.
#[derive(Copy, Clone, PartialEq, Eq)]
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
/// helper is simply unavailable (Option A still works), and we must not
/// call into the missing framework. Cached, since the OS version is fixed
/// for the process lifetime.
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
/// approve it, open Login Items so they can flip it on. No admin password
/// is involved in this flow.
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

/// Toggle system sleep through the helper socket. Returns `true` only when
/// the daemon answered `ok`; `false` means "not available, caller should
/// fall back to the admin prompt" (daemon not installed/approved, not yet
/// listening, or it rejected the request).
pub fn set_disablesleep(on: bool) -> bool {
    send(if on { "set 1\n" } else { "set 0\n" })
}

fn send(cmd: &str) -> bool {
    let Ok(mut stream) = UnixStream::connect(HELPER_SOCKET) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    if stream.write_all(cmd.as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    let mut reader = BufReader::new(&stream);
    if reader.read_line(&mut reply).is_err() {
        return false;
    }
    reply.trim() == "ok"
}
