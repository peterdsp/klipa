//! klipa privileged helper (macOS, direct-download build only).
//!
//! A minimal root daemon whose one job is to own the lid-closed override:
//! setting the system `disablesleep` power flag (which needs root) on
//! request, and, crucially, restoring it reliably even if the app crashes,
//! freezes, or is force-quit. Keeping a Mac awake with the lid shut
//! requires that flag; an unprivileged app cannot set it directly.
//!
//! It is registered as a `LaunchDaemon` through `SMAppService` (the app
//! side), runs as root under launchd (`RunAtLoad` + `KeepAlive`), and
//! listens on a fixed local Unix socket. Three things make it safe:
//!
//! * **Authenticated callers.** Every connection is validated by the
//!   peer's code signature (audit token -> `SecCode` -> pinned requirement:
//!   klipa's identifier, Apple anchor, klipa's Team ID). See `auth.rs`.
//! * **A closed, versioned protocol.** The client sends only the typed
//!   operations in `klipa-ipc` (hello/status/begin/renew/end), never a
//!   shell command, path, settings key, or environment. Requests are
//!   bounded in size and time.
//! * **Daemon-owned ownership, lease, and recovery.** The daemon, not the
//!   UI, owns timed expiry and a crash-safety lease, writes a root-owned
//!   recovery journal before mutating the flag, verifies every restore, and
//!   reconciles the journal at startup. See `manager.rs`. It never depends
//!   on Rust `Drop`, which cannot run after `SIGKILL` or power loss.

#[cfg(target_os = "macos")]
mod auth;
#[cfg(target_os = "macos")]
mod manager;

// Everything real is macOS-only (Unix sockets + pmset + Security). On other
// targets the binary still compiles (so `cargo build --workspace` is clean)
// but does nothing.
#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("klipa-helper is a macOS-only privileged helper; nothing to do here.");
}

#[cfg(target_os = "macos")]
mod macos {
    use crate::auth::authenticate_peer;
    use crate::manager::{Journal, JournalStore, OverrideManager, SetOutcome, SleepFlag, System};
    use klipa_ipc::{decode_line, encode_line, ErrorReason, Request, Response, MAX_FRAME_BYTES};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use klipa_ipc::SOCKET_PATH;

    /// Per-connection read timeout: a client that connects and then stalls
    /// cannot hold the accept loop (and therefore all power operations).
    const READ_TIMEOUT: Duration = Duration::from_secs(5);
    /// How often the monitor thread checks for lease/session expiry.
    const MONITOR_INTERVAL: Duration = Duration::from_secs(1);

    // ── Production OS backends ─────────────────────────────────────────

    /// The real power flag + clock, via `pmset` and the system.
    struct OsSystem;

    impl System for OsSystem {
        fn read_flag(&self) -> SleepFlag {
            match std::process::Command::new("/usr/bin/pmset")
                .arg("-g")
                .output()
            {
                Ok(out) if out.status.success() => {
                    crate::manager::parse_sleep_flag(&String::from_utf8_lossy(&out.stdout))
                }
                _ => SleepFlag::Unknown,
            }
        }

        fn set_flag(&self, on: bool) -> SetOutcome {
            let val = if on { "1" } else { "0" };
            let ran = std::process::Command::new("/usr/bin/pmset")
                .arg("-a")
                .arg("disablesleep")
                .arg(val)
                .status();
            match ran {
                Ok(s) if s.success() => {}
                Ok(_) | Err(_) => return SetOutcome::Unavailable,
            }
            // Verify against the real system state (powerd applies the
            // change asynchronously), not the exit code.
            let want = if on {
                SleepFlag::Disabled
            } else {
                SleepFlag::Enabled
            };
            for _ in 0..20 {
                if self.read_flag() == want {
                    return SetOutcome::Applied;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if self.read_flag() == want {
                SetOutcome::Applied
            } else {
                SetOutcome::Refused
            }
        }

        fn now_epoch(&self) -> u64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        }

        fn boot_id(&self) -> String {
            std::process::Command::new("/usr/sbin/sysctl")
                .args(["-n", "kern.bootsessionuuid"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        }
    }

    /// The root-owned recovery journal on disk. Lives under a root-only
    /// directory that persists across reboot (so does `disablesleep`), so a
    /// crash or power loss always leaves a record to reconcile.
    struct FileJournal;

    const JOURNAL_DIR: &str = "/Library/Application Support/dev.peterdsp.klipa.helper";

    impl FileJournal {
        fn path() -> std::path::PathBuf {
            std::path::Path::new(JOURNAL_DIR).join("recovery.json")
        }
    }

    impl JournalStore for FileJournal {
        fn load(&self) -> Option<Journal> {
            let bytes = std::fs::read(Self::path()).ok()?;
            serde_json::from_slice(&bytes).ok()
        }

        fn save(&self, journal: &Journal) -> bool {
            let dir = std::path::Path::new(JOURNAL_DIR);
            if std::fs::create_dir_all(dir).is_err() {
                return false;
            }
            // Root-only directory.
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            let Ok(bytes) = serde_json::to_vec(journal) else {
                return false;
            };
            // Write to a temp file then rename, so a crash mid-write never
            // leaves a half-written record.
            let tmp = Self::path().with_extension("json.tmp");
            if std::fs::write(&tmp, &bytes).is_err() {
                return false;
            }
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            std::fs::rename(&tmp, Self::path()).is_ok()
        }

        fn clear(&self) {
            let _ = std::fs::remove_file(Self::path());
        }
    }

    // ── Daemon ─────────────────────────────────────────────────────────

    type Manager = OverrideManager<OsSystem, FileJournal>;

    pub fn run() {
        // Build the manager, which reconciles any leftover journal before
        // we accept a single connection.
        let manager: Arc<Mutex<Manager>> =
            Arc::new(Mutex::new(OverrideManager::new(OsSystem, FileJournal)));

        // Autonomous expiry: the daemon owns timed/lease expiry on its own
        // thread, so a dead or frozen UI cannot strand a global override.
        {
            let manager = manager.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(MONITOR_INTERVAL);
                if let Ok(mut m) = manager.lock() {
                    if m.tick() {
                        eprintln!("klipa-helper: lease/session expired; restored normal sleep");
                    }
                }
            });
        }

        // Clear any stale socket from an unclean previous exit, then bind.
        let _ = std::fs::remove_file(SOCKET_PATH);
        let listener = match UnixListener::bind(SOCKET_PATH) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("klipa-helper: bind {SOCKET_PATH} failed: {e}");
                std::process::exit(1);
            }
        };
        // Let the unprivileged app connect; the per-connection code-signature
        // check, not the file mode, is the trust boundary.
        if let Ok(md) = std::fs::metadata(SOCKET_PATH) {
            let mut perms = md.permissions();
            perms.set_mode(0o666);
            let _ = std::fs::set_permissions(SOCKET_PATH, perms);
        }
        eprintln!("klipa-helper: listening on {SOCKET_PATH}");

        for conn in listener.incoming() {
            match conn {
                Ok(stream) => handle(stream, &manager),
                Err(e) => eprintln!("klipa-helper: accept failed: {e}"),
            }
        }
    }

    /// Validate the caller, read one bounded request, dispatch it under the
    /// lock, and reply. One connection, one request/response.
    fn handle(stream: UnixStream, manager: &Arc<Mutex<Manager>>) {
        let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
        let _ = stream.set_write_timeout(Some(READ_TIMEOUT));

        // Authenticate by code signature before doing anything else.
        let uid = match authenticate_peer(stream.as_raw_fd()) {
            Ok(uid) => uid,
            Err(reason) => {
                reply(&stream, &Response::error(reason, "caller not authorized"));
                return;
            }
        };

        // Read one newline-terminated request, bounded in size.
        let mut line = Vec::new();
        {
            let mut reader = BufReader::new((&stream).take(MAX_FRAME_BYTES as u64));
            if reader.read_until(b'\n', &mut line).is_err() {
                return;
            }
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let request: Request = match decode_line(&line) {
            Ok(r) => r,
            Err(_) => {
                reply(
                    &stream,
                    &Response::error(ErrorReason::Malformed, "unparseable or oversized request"),
                );
                return;
            }
        };

        let response = match manager.lock() {
            Ok(mut m) => m.handle(&request, uid),
            Err(_) => Response::error(ErrorReason::SystemUnavailable, "helper busy"),
        };
        reply(&stream, &response);
    }

    fn reply(mut stream: &UnixStream, response: &Response) {
        if let Ok(bytes) = encode_line(response) {
            let _ = stream.write_all(&bytes);
        }
    }
}
