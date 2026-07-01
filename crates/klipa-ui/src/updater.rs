//! Best-effort "hey, there's a new version" check for the direct-
//! download builds (macOS .pkg, Windows setup.exe, Linux AppImage).
//!
//! Compiled out of the Mac App Store build - the App Store handles
//! updates for that one. Everything else pulls the latest release info
//! from GitHub once a day, and when it finds a newer version:
//!
//! 1. adds an "Update to vX.Y.Z" item to the tray menu, and
//! 2. on click, downloads the platform installer to a temp file and
//!    opens it with the OS default handler (Installer.app on macOS,
//!    the .exe setup on Windows, xdg-open on Linux).
//!
//! Zero binary cost: the fetch reuses [`crate::http::get`] (curl) and
//! nothing new is linked in. If the network is down, we silently skip;
//! if curl is missing the update item never appears.

#[cfg(not(feature = "mas"))]
pub use imp::UpdateState;

#[cfg(not(feature = "mas"))]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// One day between checks. That is plenty for a menu bar utility.
    const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
    const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
    const RELEASE_PAGE: &str = "https://github.com/peterdsp/klipa/releases/latest";

    /// Shared state between the main loop and the background checker.
    pub struct UpdateState {
        /// Confirmed newer version + optional direct installer URL.
        pending: Option<Pending>,
        /// When the last check kicked off (regardless of outcome).
        last_check: Option<Instant>,
        /// Background thread liveness guard so we never overlap checks.
        checking: Arc<AtomicBool>,
        /// Filled in by the background thread; drained by the main loop.
        inbox: Arc<Mutex<Option<Pending>>>,
    }

    #[derive(Clone)]
    struct Pending {
        version: String,
        installer_url: Option<String>,
    }

    impl UpdateState {
        pub fn new() -> Self {
            Self {
                pending: None,
                last_check: None,
                checking: Arc::new(AtomicBool::new(false)),
                inbox: Arc::new(Mutex::new(None)),
            }
        }

        /// Called on every event-loop tick. Kicks off a background
        /// check when the daily interval elapses. Also drains any
        /// result posted by an earlier check. Returns true if the menu
        /// should be rebuilt (i.e. we just learned about a new update).
        pub fn tick(&mut self) -> bool {
            let mut refreshed = false;
            // Drain any result the background thread posted.
            if let Ok(mut inbox) = self.inbox.lock() {
                if let Some(p) = inbox.take() {
                    // Only announce if it's actually newer than us.
                    if is_newer(&p.version, CURRENT_VERSION) {
                        self.pending = Some(p);
                        refreshed = true;
                    }
                }
            }
            // Time for a fresh check?
            let due = self
                .last_check
                .map(|t| t.elapsed() >= CHECK_INTERVAL)
                .unwrap_or(true);
            if due && !self.checking.load(Ordering::Acquire) {
                self.last_check = Some(Instant::now());
                self.spawn_check();
            }
            refreshed
        }

        /// Menu label if an update is pending, e.g. "Update to v0.3.0".
        pub fn menu_label(&self) -> Option<String> {
            self.pending.as_ref().map(|p| format!("Update to v{}", p.version))
        }

        /// Called when the user clicks the update menu item.
        pub fn trigger(&self) {
            let Some(p) = &self.pending else {
                return;
            };
            // Download the installer to a temp file and open it. If the
            // download fails, fall back to opening the release page in
            // the user's browser so they can install manually.
            let target = p
                .installer_url
                .as_deref()
                .and_then(download_to_temp)
                .unwrap_or_else(|| RELEASE_PAGE.to_string());
            open_native(&target);
        }

        fn spawn_check(&self) {
            let checking = self.checking.clone();
            let inbox = self.inbox.clone();
            checking.store(true, Ordering::Release);
            // Detached thread: cheap for a once-a-day job and lets the
            // UI ignore it entirely.
            std::thread::spawn(move || {
                if let Some(p) = do_check() {
                    if let Ok(mut slot) = inbox.lock() {
                        *slot = Some(p);
                    }
                }
                checking.store(false, Ordering::Release);
            });
        }
    }

    /// Ask GitHub for the latest release; parse the tag and the URL of
    /// the installer matching this platform.
    fn do_check() -> Option<Pending> {
        let body = crate::http::get(
            "https://api.github.com/repos/peterdsp/klipa/releases/latest",
            Duration::from_secs(10),
        )?;
        let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let tag = json.get("tag_name")?.as_str()?;
        let version = tag.trim_start_matches('v').to_string();
        let assets = json.get("assets")?.as_array()?;
        Some(Pending {
            version,
            installer_url: platform_asset(assets),
        })
    }

    /// Match this build's platform to the right release asset.
    fn platform_asset(assets: &[serde_json::Value]) -> Option<String> {
        // These substrings match what scripts/package-*.sh produce.
        let needle: &str = if cfg!(target_os = "macos") {
            "-macos.pkg"
        } else if cfg!(target_os = "windows") {
            "-windows-x64-setup.exe"
        } else if cfg!(target_os = "linux") {
            "-x86_64.AppImage"
        } else {
            return None;
        };
        for a in assets {
            let name = a.get("name").and_then(|v| v.as_str())?;
            if name.contains(needle) {
                return a
                    .get("browser_download_url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
        }
        None
    }

    /// Save a URL's body into a temp file named after the URL basename.
    fn download_to_temp(url: &str) -> Option<String> {
        let name = url.rsplit('/').next()?;
        // Only accept a sane filename to avoid weird tempdir paths.
        if name.is_empty() || name.len() > 128 {
            return None;
        }
        let body = crate::http::get(url, Duration::from_secs(120))?;
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, &body).ok()?;
        Some(path.to_string_lossy().to_string())
    }

    /// Open a file path or URL with the OS default handler.
    fn open_native(target: &str) {
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(target).spawn();
        }
        #[cfg(target_os = "windows")]
        {
            // `cmd /C start "" "..."` handles both URLs and file paths.
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", target])
                .spawn();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let _ = std::process::Command::new("xdg-open").arg(target).spawn();
        }
    }

    /// Semver-ish comparison for tags like "0.3.0". Extra dot-parts
    /// and non-numeric bits are ignored; missing parts count as 0.
    /// Returns true if `remote` > `local`.
    fn is_newer(remote: &str, local: &str) -> bool {
        triple(remote) > triple(local)
    }

    fn triple(s: &str) -> (u32, u32, u32) {
        let mut it = s.split('.');
        let p = |x: Option<&str>| x.and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        (p(it.next()), p(it.next()), p(it.next()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn is_newer_semver() {
            assert!(is_newer("0.3.0", "0.2.0"));
            assert!(is_newer("0.2.1", "0.2.0"));
            assert!(is_newer("1.0.0", "0.99.99"));
            assert!(!is_newer("0.2.0", "0.2.0"));
            assert!(!is_newer("0.2.0", "0.3.0"));
        }

        #[test]
        fn triple_handles_missing_parts() {
            assert_eq!(triple("1"), (1, 0, 0));
            assert_eq!(triple("1.2"), (1, 2, 0));
            assert_eq!(triple("1.2.3"), (1, 2, 3));
            assert_eq!(triple("bogus"), (0, 0, 0));
        }
    }
}

// ── No-op stub for the Mac App Store build ─────────────────────────
#[cfg(feature = "mas")]
pub struct UpdateState;

#[cfg(feature = "mas")]
impl UpdateState {
    pub fn new() -> Self {
        Self
    }
    pub fn tick(&mut self) -> bool {
        false
    }
    pub fn menu_label(&self) -> Option<String> {
        None
    }
    pub fn trigger(&self) {}
}

