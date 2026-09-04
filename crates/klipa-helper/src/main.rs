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

#[cfg(target_os = "macos")]
mod macos {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};

    /// Fixed socket path. `/var/run` is root-owned and cleared on boot;
    /// the daemon (RunAtLoad) recreates the socket each boot. Kept in sync
    /// by hand with `HELPER_SOCKET` in the app's `helper.rs`.
    const SOCKET_PATH: &str = "/var/run/dev.peterdsp.klipa.helper.sock";

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

    /// Read one newline-terminated command, act on it, reply `ok`/`err`.
    fn handle(stream: UnixStream) {
        let mut line = String::new();
        {
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_err() {
                return;
            }
        }
        let ok = dispatch(line.trim());
        let mut w = stream;
        let _ = w.write_all(if ok { b"ok\n" } else { b"err\n" });
    }

    /// The entire command vocabulary. A closed whitelist is the security
    /// boundary: the daemon can only ever toggle `disablesleep` or answer
    /// a liveness ping, never anything an attacker might smuggle in.
    fn dispatch(cmd: &str) -> bool {
        match cmd {
            "set 1" => pmset_disablesleep(true),
            "set 0" => pmset_disablesleep(false),
            "ping" => true,
            other => {
                eprintln!("klipa-helper: rejected command {other:?}");
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
