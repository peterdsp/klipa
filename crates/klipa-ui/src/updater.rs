//! Best-effort "hey, there's a new version" check for the direct-
//! download builds (macOS .pkg, Windows setup.exe, Linux AppImage).
//!
//! Compiled out of the Mac App Store build - the App Store handles
//! updates for that one. Everything else pulls the latest release info
//! from GitHub once a day, and when it finds a newer version:
//!
//! 1. adds an "Update to vX.Y.Z" item to the tray menu, and
//! 2. on click:
//!    - macOS: downloads the notarized .zip and swaps `klipa.app` in
//!      place as the current user (no installer, no admin prompt), then
//!      relaunches on the new version. Falls back to the release page if
//!      the app directory isn't writable (e.g. a non-admin account).
//!    - Windows / Linux: downloads the platform installer and opens it
//!      with the OS default handler (setup .exe / xdg-open).
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
            self.pending
                .as_ref()
                .map(|p| format!("Update to v{}", p.version))
        }

        /// Called when the user clicks the update menu item.
        ///
        /// Runs entirely on a background thread so the ~seconds-long
        /// download never freezes the UI loop. On macOS it swaps the app
        /// bundle in place as the current user - no installer, no admin
        /// prompt - and relaunches klipa on the new version; the old
        /// process exits, so the stale "Update to ..." item disappears on
        /// its own. Everything else keeps the simple "download + open the
        /// installer" behavior.
        pub fn trigger(&self) {
            let Some(p) = &self.pending else {
                return;
            };
            let installer_url = p.installer_url.clone();
            std::thread::spawn(move || {
                let downloaded = installer_url.as_deref().and_then(download_to_temp);
                #[cfg(target_os = "macos")]
                match downloaded {
                    // The ordered update transaction succeeded -> relaunch on
                    // the new version (this exits the old process).
                    Some(zip) if apply_update(&zip) => relaunch(),
                    // Download failed, the candidate failed verification, the
                    // active session could not be confirmed restored, or the
                    // app directory was not writable: fall back to the release
                    // page so the user can update by hand (Gatekeeper-checked).
                    _ => open_native(RELEASE_PAGE),
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let target = downloaded.unwrap_or_else(|| RELEASE_PAGE.to_string());
                    open_native(&target);
                }
            });
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
        // macOS updates via the .zip (in-place bundle swap), not the
        // .pkg - the pkg is only for first installs.
        let needle: &str = if cfg!(target_os = "macos") {
            "-macos.zip"
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

    /// The running `.app` bundle, derived from the executable path
    /// (`<bundle>/Contents/MacOS/<bin>`). `None` if we're not running
    /// from a bundle (e.g. a bare `cargo run`), which skips the swap.
    #[cfg(target_os = "macos")]
    fn app_bundle_path() -> Option<std::path::PathBuf> {
        let exe = std::env::current_exe().ok()?;
        // MacOS -> Contents -> klipa.app
        let bundle = exe.parent()?.parent()?.parent()?;
        (bundle.extension()? == "app").then(|| bundle.to_path_buf())
    }

    /// The ordered steps of the update transaction, decided from the three
    /// checks that gate it. Pure, so the ordering rule (never replace the
    /// helper-containing bundle before the candidate is verified AND the
    /// active session is confirmed restored, and never discard the rollback
    /// before the installed app/helper is verified) is unit-tested.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    enum UpdatePlan {
        /// Stop before touching the installed bundle.
        AbortBeforeSwap,
        /// The swap happened but the installed app/helper failed verification:
        /// roll back to the retained previous bundle.
        RollbackAfterSwap,
        /// Everything healthy: commit and drop the rollback.
        Commit,
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn plan_update(candidate_ok: bool, session_restored: bool, installed_ok: bool) -> UpdatePlan {
        if !candidate_ok || !session_restored {
            return UpdatePlan::AbortBeforeSwap;
        }
        if !installed_ok {
            return UpdatePlan::RollbackAfterSwap;
        }
        UpdatePlan::Commit
    }

    /// Apply a downloaded update as an explicit, ordered transaction:
    ///
    /// 1. Extract the candidate into a staging dir on the same volume.
    /// 2. Verify the candidate's authenticated code identity (strict
    ///    signature + Gatekeeper/notarization + signed nested helper).
    /// 3. Coordinate the active session: an explicit, VERIFIED stop that
    ///    restores normal sleep BEFORE the helper-containing bundle is
    ///    replaced. A restore that cannot be confirmed aborts the update
    ///    rather than silently proceeding.
    /// 4. Replace the bundle, keeping the previous one as a rollback.
    /// 5. Verify the newly installed app/helper at its final path.
    /// 6. Only once healthy, drop the rollback; otherwise restore it.
    ///
    /// Returns true only when the new bundle is live and verified at the
    /// original path. Fails cleanly (leaving the old bundle intact) at every
    /// step, so the caller can fall back to a manual install.
    #[cfg(target_os = "macos")]
    fn apply_update(zip: &str) -> bool {
        use std::fs;
        let Some(bundle) = app_bundle_path() else {
            return false;
        };
        let Some(dir) = bundle.parent() else {
            return false;
        };
        let pid = std::process::id();
        let staging = dir.join(format!(".klipa-update-{pid}"));
        let old = dir.join(format!(".klipa-old-{pid}.app"));

        // Fresh staging dir; bail early if we can't write here at all.
        let _ = fs::remove_dir_all(&staging);
        if fs::create_dir_all(&staging).is_err() {
            return false;
        }
        // `ditto -x -k` unpacks the zip preserving the code signature,
        // notarization staple, and extended attributes. `--keepParent`
        // at pack time means the archive contains `klipa.app`.
        let extracted = staging.join("klipa.app");
        let unpacked = std::process::Command::new("ditto")
            .args(["-x", "-k", zip])
            .arg(&staging)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !unpacked || !extracted.exists() {
            let _ = fs::remove_dir_all(&staging);
            return false;
        }

        // Step 2: authenticated code identity of the candidate. A checksum
        // proves integrity, not authenticity, so this requires a strict
        // signature, a passing Gatekeeper assessment (notarization), and a
        // validly signed nested helper.
        let candidate_ok = verify_bundle(&extracted);

        // Step 3: verified stop of the active session BEFORE replacing the
        // bundle that carries the registered helper. If normal sleep cannot
        // be confirmed restored, do not proceed. (This whole module is the
        // non-App-Store build, so the lid coordinator is always present here.)
        let session_restored = crate::lid::end_verified();

        if plan_update(candidate_ok, session_restored, true) == UpdatePlan::AbortBeforeSwap {
            if !candidate_ok {
                tracing::warn!("update failed signature/notarization check; not installing");
            }
            if !session_restored {
                tracing::warn!(
                    "update aborted: could not confirm the active session restored; \
                     not replacing the helper bundle"
                );
            }
            let _ = fs::remove_dir_all(&staging);
            return false;
        }

        // Step 4: move the current bundle aside (kept as rollback), then move
        // the new one in. Same-directory renames are atomic.
        let _ = fs::remove_dir_all(&old);
        if fs::rename(&bundle, &old).is_err() {
            let _ = fs::remove_dir_all(&staging);
            return false;
        }
        if fs::rename(&extracted, &bundle).is_err() {
            // Put the old bundle back so we never leave the app missing.
            let _ = fs::rename(&old, &bundle);
            let _ = fs::remove_dir_all(&staging);
            return false;
        }

        // Step 5: verify the newly installed app/helper at its final path,
        // BEFORE discarding the rollback.
        let installed_ok = verify_bundle(&bundle);
        match plan_update(candidate_ok, session_restored, installed_ok) {
            UpdatePlan::Commit => {
                // Step 6: healthy, so it is safe to drop the rollback. A
                // failed cleanup here is non-fatal (the live binary keeps its
                // inode) but is logged, never silently ignored.
                if fs::remove_dir_all(&old).is_err() {
                    tracing::warn!("update healthy but could not remove the previous bundle");
                }
                let _ = fs::remove_dir_all(&staging);
                true
            }
            UpdatePlan::RollbackAfterSwap => {
                // The installed app/helper did not verify: restore the
                // retained previous bundle and report failure.
                let _ = fs::remove_dir_all(&bundle);
                let _ = fs::rename(&old, &bundle);
                let _ = fs::remove_dir_all(&staging);
                tracing::warn!(
                    "installed bundle failed verification; rolled back to the previous version"
                );
                false
            }
            // AbortBeforeSwap was already handled before any swap.
            UpdatePlan::AbortBeforeSwap => {
                let _ = fs::remove_dir_all(&staging);
                false
            }
        }
    }

    /// Whether a shell command exited 0.
    #[cfg(target_os = "macos")]
    fn run_ok(cmd: &str, args: &[&str]) -> bool {
        std::process::Command::new(cmd)
            .args(args)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Verify a downloaded `.app` is a strictly-signed, notarized klipa with
    /// a validly signed nested helper, before it is allowed to replace the
    /// installed bundle.
    #[cfg(target_os = "macos")]
    fn verify_bundle(app: &std::path::Path) -> bool {
        let Some(app_str) = app.to_str() else {
            return false;
        };
        let strict = run_ok(
            "codesign",
            &["--verify", "--strict", "--deep", "--verbose=2", app_str],
        );
        let gatekeeper = run_ok("spctl", &["--assess", "--type", "execute", app_str]);
        let helper = app.join("Contents/MacOS/klipa-helper");
        let helper_ok = match helper.to_str() {
            Some(h) if helper.exists() => run_ok("codesign", &["--verify", "--strict", h]),
            // No nested helper (should not happen for a direct build) is not
            // a failure of the app's own signature.
            _ => true,
        };
        strict && gatekeeper && helper_ok
    }

    /// Launch the freshly-installed bundle and exit this (old) process so the
    /// user lands on the new version. Called only after a successful
    /// `apply_update`, which has already performed the explicit, verified stop
    /// of the active session (restoring normal sleep and releasing the lid
    /// override) BEFORE the bundle was replaced. The relaunched app does not
    /// auto-resume the session (klipa never resurrects one), so the user
    /// restarts keep-awake deliberately after the update.
    #[cfg(target_os = "macos")]
    fn relaunch() -> ! {
        if let Some(bundle) = app_bundle_path() {
            let _ = std::process::Command::new("open")
                .arg("-n")
                .arg(&bundle)
                .spawn();
        }
        std::process::exit(0);
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

        #[test]
        fn update_never_swaps_before_candidate_and_session_are_good() {
            // The candidate must verify first.
            assert_eq!(
                plan_update(false, true, true),
                UpdatePlan::AbortBeforeSwap,
                "an unverified candidate is never installed"
            );
            // Regression for the ordering bug: the active session must be
            // confirmed restored BEFORE the helper-containing bundle is
            // replaced. A session that could not be restored aborts the
            // update rather than silently proceeding.
            assert_eq!(
                plan_update(true, false, true),
                UpdatePlan::AbortBeforeSwap,
                "an unrestored session blocks the swap"
            );
            assert_eq!(plan_update(false, false, true), UpdatePlan::AbortBeforeSwap);
        }

        #[test]
        fn update_keeps_rollback_until_the_installed_app_is_healthy() {
            // Swapped, but the installed app/helper did not verify: roll back
            // to the retained previous bundle instead of committing.
            assert_eq!(
                plan_update(true, true, false),
                UpdatePlan::RollbackAfterSwap
            );
            // All three checks pass: commit and drop the rollback.
            assert_eq!(plan_update(true, true, true), UpdatePlan::Commit);
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
