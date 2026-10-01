//! klipa privileged helper (macOS, direct-download build only).
//!
//! A minimal root daemon that exists for exactly one reason: to let the
//! non-sandboxed klipa app keep the Mac awake with the lid closed without
//! prompting for an admin password every time. Keeping the Mac awake with
//! the lid shut requires setting the system `disablesleep` power flag,
//! which needs root; an unprivileged app cannot do it directly.
//!
//! It is registered as a `LaunchDaemon` through `SMAppService` (the app
//! side), runs as root under launchd, and listens on a fixed local Unix
//! socket. The app connects and sends one of a tiny set of whitelisted
//! commands; anything else is rejected, so a compromise of the app can
//! only toggle system sleep, never run arbitrary code as root.
//!
//! Security note: the socket is created world-connectable (0666) so the
//! unprivileged app can reach it. That means any local process running as
//! any user can also ask to toggle system sleep. The capability is
//! deliberately limited to that single, benign power setting. Tightening
//! this to a code-signed XPC peer check is documented as future work in
//! `docs/lid-closed-keep-awake.md`.

// Everything real is macOS-only (Unix sockets + pmset). On other targets
// the binary still compiles (so `cargo build --workspace` is clean) but
// does nothing.
#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("klipa-helper is a macOS-only privileged helper; nothing to do here.");
}

/// The helper's entire command vocabulary. A closed whitelist is the
/// security boundary: the daemon can only ever toggle `disablesleep` or
/// answer a liveness ping, never anything an attacker might smuggle in.
/// Parsing is kept separate from acting so the whitelist is unit-tested
/// without touching `pmset` (and so CI exercises the helper logic, not
/// just that the daemon compiles).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
enum Request {
    /// `set 1`: disable system sleep (keep awake with the lid closed).
    Disable,
    /// `set 0`: restore normal sleep.
    Enable,
    /// `ping`: liveness check.
    Ping,
    /// Anything else, including an over-long or malformed line.
    Rejected,
}

/// Classify one request line. The input is already length-bounded by the
/// caller; an unrecognized command (or the sentinel used for an over-long
/// line) is `Rejected`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn classify(line: &str) -> Request {
    match line.trim() {
        "set 1" => Request::Disable,
        "set 0" => Request::Enable,
        "ping" => Request::Ping,
        _ => Request::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, Request};

    #[test]
    fn only_the_whitelist_is_accepted() {
        assert_eq!(classify("set 1"), Request::Disable);
        assert_eq!(classify("set 0"), Request::Enable);
        assert_eq!(classify("ping"), Request::Ping);
        // Surrounding whitespace / newline is tolerated.
        assert_eq!(classify("  set 1\n"), Request::Disable);
    }

    #[test]
    fn anything_outside_the_whitelist_is_rejected() {
        for bad in [
            "",
            "set",
            "set 2",
            "set 1; rm -rf /",
            "SET 1",
            "pmset -a disablesleep 1",
            "ping extra",
            "set 1 ",
        ] {
            // The trailing-space case is tolerated by `trim`, so check the
            // genuinely hostile ones are rejected.
            if bad.trim() == "set 1" {
                continue;
            }
            assert_eq!(classify(bad), Request::Rejected, "must reject {bad:?}");
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{classify, Request};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::time::Duration;

    /// Fixed socket path. `/var/run` is root-owned and cleared on boot;
    /// the daemon (RunAtLoad) recreates the socket each boot. Kept in sync
    /// by hand with `HELPER_SOCKET` in the app's `helper.rs`.
    const SOCKET_PATH: &str = "/var/run/dev.peterdsp.klipa.helper.sock";

    /// Upper bound on one request line. The whitelist commands are a
    /// handful of bytes; anything larger is hostile or broken, so the read
    /// is capped rather than left unbounded (a client that never sends a
    /// newline could otherwise grow the buffer without limit).
    const MAX_REQUEST_BYTES: u64 = 256;

    /// How long the daemon waits for a client to send its request before
    /// giving up on that connection. Bounds a stalled or silent client so
    /// it cannot hold the single-threaded accept loop, and therefore all
    /// power operations, open indefinitely.
    const READ_TIMEOUT: Duration = Duration::from_secs(5);

    pub fn run() {
        // Clear any stale socket left by an unclean previous exit, then
        // bind. If bind fails there is nothing useful we can do, so exit
        // non-zero and let launchd's KeepAlive relaunch us.
        let _ = std::fs::remove_file(SOCKET_PATH);
        let listener = match UnixListener::bind(SOCKET_PATH) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("klipa-helper: bind {SOCKET_PATH} failed: {e}");
                std::process::exit(1);
            }
        };
        // Let the unprivileged app connect. See the module security note.
        if let Ok(md) = std::fs::metadata(SOCKET_PATH) {
            let mut perms = md.permissions();
            perms.set_mode(0o666);
            let _ = std::fs::set_permissions(SOCKET_PATH, perms);
        }
        eprintln!("klipa-helper: listening on {SOCKET_PATH}");

        for conn in listener.incoming() {
            match conn {
                Ok(stream) => handle(stream),
                Err(e) => eprintln!("klipa-helper: accept failed: {e}"),
            }
        }
    }

    /// Read one newline-terminated command (bounded in both size and
    /// time), act on it, reply `ok`/`err`.
    fn handle(stream: UnixStream) {
        // Bound how long we wait for the request and how many bytes we
        // accept, so a client that connects and then stalls or floods
        // cannot wedge the single accept loop or exhaust memory.
        let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
        let _ = stream.set_write_timeout(Some(READ_TIMEOUT));

        let mut line = String::new();
        {
            let mut reader = BufReader::new((&stream).take(MAX_REQUEST_BYTES));
            if reader.read_line(&mut line).is_err() {
                return;
            }
        }
        // If we hit the cap without a newline the request was over-long:
        // treat it as rejected rather than acting on a truncated command.
        let request = if line.len() as u64 >= MAX_REQUEST_BYTES && !line.ends_with('\n') {
            Request::Rejected
        } else {
            classify(&line)
        };
        let ok = dispatch(request);
        let mut w = stream;
        let _ = w.write_all(if ok { b"ok\n" } else { b"err\n" });
    }

    /// Act on a classified request. The only side effect the daemon ever
    /// has is toggling `disablesleep`; everything else answers in-process.
    fn dispatch(request: Request) -> bool {
        match request {
            Request::Disable => pmset_disablesleep(true),
            Request::Enable => pmset_disablesleep(false),
            Request::Ping => true,
            Request::Rejected => {
                eprintln!("klipa-helper: rejected request");
                false
            }
        }
    }

    fn pmset_disablesleep(on: bool) -> bool {
        let val = if on { "1" } else { "0" };
        std::process::Command::new("/usr/bin/pmset")
            .arg("-a")
            .arg("disablesleep")
            .arg(val)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}
