//! Trial + license gate for the paid (non-App-Store) builds.
//!
//! klipa is free to use in full for [`TRIAL_DAYS`] days. After that the
//! app locks to a paywall until a license key (bought for €1.99) is
//! activated. The Mac App Store build is a paid app, so it carries no
//! licensing code at all - everything here is behind the `license`
//! feature, and a no-op stub stands in when the feature is off.
//!
//! Honesty note: klipa is open source (MIT). Client-side checks like
//! this only gate the *prebuilt* binaries - anyone can compile their
//! own. This is a nudge for honest users, not DRM.

/// What the rest of the app is allowed to do right now.
// In the App Store build (no `license` feature) only `Full` is ever
// constructed; the trial/paywall variants are intentionally unused.
#[cfg_attr(not(feature = "license"), allow(dead_code))]
pub enum Gate {
    /// Licensed, or feature compiled out: full functionality.
    Full,
    /// Inside the free trial; still full functionality.
    Trial { days_left: i64 },
    /// Trial elapsed and unlicensed: show the paywall.
    Locked,
}

impl Gate {
    pub fn is_locked(&self) -> bool {
        matches!(self, Gate::Locked)
    }
}

pub use imp::License;

// ── Real implementation (feature = "license") ────────────────────────
#[cfg(feature = "license")]
mod imp {
    use super::Gate;
    use crate::paths;
    use serde::{Deserialize, Serialize};
    use std::time::{Duration as StdDuration, Instant};
    use time::OffsetDateTime;

    /// Length of the free trial.
    const TRIAL_DAYS: i64 = 7;
    /// Shown on the unlock menu item. Override at build time with
    /// `KLIPA_PRICE=...` (the store charges in EUR by default).
    pub(super) const PRICE: &str = match option_env!("KLIPA_PRICE") {
        Some(v) => v,
        None => "\u{20ac}1.99",
    };

    /// Where buyers are sent to purchase. Override at build time with
    /// `KLIPA_PURCHASE_URL=...`.
    const PURCHASE_URL: &str = match option_env!("KLIPA_PURCHASE_URL") {
        Some(v) => v,
        None => "https://klipa.peterdsp.dev/buy",
    };
    /// Gumroad product id used to verify license keys. Override at build
    /// time with `KLIPA_GUMROAD_PRODUCT_ID=...`. Empty == not configured
    /// (keys can't be verified yet, but the trial still works).
    const GUMROAD_PRODUCT_ID: &str = match option_env!("KLIPA_GUMROAD_PRODUCT_ID") {
        Some(v) => v,
        None => "",
    };

    /// Persisted trial + license state (`license.json`).
    #[derive(Serialize, Deserialize, Default)]
    struct State {
        #[serde(with = "time::serde::rfc3339::option", default)]
        trial_started_at: Option<OffsetDateTime>,
        license_key: Option<String>,
        #[serde(with = "time::serde::rfc3339::option", default)]
        activated_at: Option<OffsetDateTime>,
        #[serde(with = "time::serde::rfc3339::option", default)]
        last_verified_at: Option<OffsetDateTime>,
    }

    pub struct License {
        state: State,
        /// Transient menu feedback (message + when it should disappear).
        transient: Option<(String, Instant)>,
    }

    impl License {
        /// Load (or start) the trial. Stamps the trial clock on first run.
        pub fn load() -> Self {
            let mut state: State = paths::license_file()
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            if state.trial_started_at.is_none() {
                state.trial_started_at = Some(OffsetDateTime::now_utc());
            }
            let me = Self {
                state,
                transient: None,
            };
            me.save();
            me
        }

        pub fn gate(&self) -> Gate {
            gate_at(
                self.state.activated_at.is_some(),
                self.state.trial_started_at,
                OffsetDateTime::now_utc(),
            )
        }

        pub fn price() -> &'static str {
            PRICE
        }

        /// Open the purchase page in the user's browser.
        pub fn open_purchase(&self) {
            open_url(PURCHASE_URL);
        }

        /// Try to activate using whatever text is on the clipboard - the
        /// natural gesture for a clipboard app: copy your key, click
        /// "Activate". Sets a transient menu message with the outcome.
        pub fn activate_from_clipboard(&mut self) {
            let key = arboard::Clipboard::new()
                .ok()
                .and_then(|mut c| c.get_text().ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if key.is_empty() {
                self.flash("Copy your license key first, then Activate");
                return;
            }
            match verify_key(&key) {
                Verify::Valid => {
                    let now = OffsetDateTime::now_utc();
                    self.state.license_key = Some(key);
                    self.state.activated_at = Some(now);
                    self.state.last_verified_at = Some(now);
                    self.save();
                    self.flash("Activated - thank you!");
                }
                Verify::Invalid => self.flash("That license key is not valid"),
                Verify::NotConfigured => self.flash("Licensing not configured yet"),
                Verify::Network => self.flash("Could not reach the license server"),
            }
        }

        /// Re-check an activated key in the background now and then, so a
        /// refunded/revoked key eventually stops working. Best-effort:
        /// network failures keep the license (offline grace).
        pub fn reverify_if_stale(&mut self) {
            let (Some(key), Some(last)) =
                (self.state.license_key.clone(), self.state.last_verified_at)
            else {
                return;
            };
            if OffsetDateTime::now_utc() - last < time::Duration::days(14) {
                return;
            }
            match verify_key(&key) {
                Verify::Valid => {
                    self.state.last_verified_at = Some(OffsetDateTime::now_utc());
                    self.save();
                }
                Verify::Invalid => {
                    // Revoked/refunded: drop the activation.
                    self.state.activated_at = None;
                    self.state.license_key = None;
                    self.save();
                }
                // Network / not-configured: keep the license as-is.
                _ => {}
            }
        }

        /// A short-lived status line for the menu, if one is pending.
        pub fn transient_message(&mut self) -> Option<String> {
            match &self.transient {
                Some((msg, until)) if Instant::now() < *until => Some(msg.clone()),
                Some(_) => {
                    self.transient = None;
                    None
                }
                None => None,
            }
        }

        fn flash(&mut self, msg: &str) {
            self.transient = Some((msg.to_string(), Instant::now() + StdDuration::from_secs(6)));
        }

        fn save(&self) {
            let Some(path) = paths::license_file() else {
                return;
            };
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_vec_pretty(&self.state) {
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, &json).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
        }
    }

    /// Pure trial/license decision, factored out so it can be tested
    /// without touching the clock or the filesystem.
    fn gate_at(licensed: bool, trial_started: Option<OffsetDateTime>, now: OffsetDateTime) -> Gate {
        if licensed {
            return Gate::Full;
        }
        let start = trial_started.unwrap_or(now);
        let end = start + time::Duration::days(TRIAL_DAYS);
        if now < end {
            let secs = (end - now).whole_seconds().max(0) as f64;
            Gate::Trial {
                days_left: (secs / 86_400.0).ceil() as i64,
            }
        } else {
            Gate::Locked
        }
    }

    enum Verify {
        Valid,
        Invalid,
        NotConfigured,
        Network,
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn licensed_is_always_full() {
            let now = OffsetDateTime::now_utc();
            // Even with a long-expired trial, a license means Full.
            let long_ago = now - time::Duration::days(99);
            assert!(matches!(gate_at(true, Some(long_ago), now), Gate::Full));
        }

        #[test]
        fn fresh_trial_has_seven_days() {
            let now = OffsetDateTime::now_utc();
            match gate_at(false, Some(now), now) {
                Gate::Trial { days_left } => assert_eq!(days_left, TRIAL_DAYS),
                g => panic!("expected Trial, got {:?}", DebugGate(&g)),
            }
        }

        #[test]
        fn trial_counts_down_and_then_locks() {
            let start = OffsetDateTime::now_utc();
            // 6.5 days in -> still in trial, 1 day left (ceil).
            let mid = start + time::Duration::hours(6 * 24 + 12);
            assert!(matches!(
                gate_at(false, Some(start), mid),
                Gate::Trial { days_left: 1 }
            ));
            // Exactly at the boundary and beyond -> Locked.
            let end = start + time::Duration::days(TRIAL_DAYS);
            assert!(matches!(gate_at(false, Some(start), end), Gate::Locked));
            assert!(matches!(
                gate_at(false, Some(start), end + time::Duration::days(1)),
                Gate::Locked
            ));
        }

        // Small helper so panics print which variant we got.
        struct DebugGate<'a>(&'a Gate);
        impl std::fmt::Debug for DebugGate<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self.0 {
                    Gate::Full => write!(f, "Full"),
                    Gate::Trial { days_left } => write!(f, "Trial({days_left})"),
                    Gate::Locked => write!(f, "Locked"),
                }
            }
        }
    }

    /// Verify a license key against Gumroad's license API. Swap this one
    /// function to use a different store (Lemon Squeezy, Polar, ...).
    ///
    /// Gumroad has used both `product_id` (current) and `product_permalink`
    /// (legacy, e.g. "klipa") as the identifier, and which one their API
    /// accepts has varied. We try the modern field first and fall back to
    /// the permalink, so whichever value `KLIPA_GUMROAD_PRODUCT_ID` holds
    /// will verify.
    fn verify_key(key: &str) -> Verify {
        if GUMROAD_PRODUCT_ID.is_empty() {
            tracing::warn!("KLIPA_GUMROAD_PRODUCT_ID not set at build time; cannot verify keys");
            return Verify::NotConfigured;
        }
        match verify_with(key, "product_id") {
            // Only an outright "invalid" is worth a permalink retry; a
            // network error should stay a network error (offline grace).
            Verify::Invalid => verify_with(key, "product_permalink"),
            other => other,
        }
    }

    /// One verify attempt using the given identifier field name.
    fn verify_with(key: &str, id_field: &str) -> Verify {
        let resp = ureq::post("https://api.gumroad.com/v2/licenses/verify")
            .timeout(StdDuration::from_secs(10))
            .send_form(&[
                (id_field, GUMROAD_PRODUCT_ID),
                ("license_key", key),
                ("increment_uses_count", "false"),
            ]);
        let json: serde_json::Value = match resp {
            Ok(r) => match r.into_json() {
                Ok(j) => j,
                Err(_) => return Verify::Network,
            },
            // Gumroad answers an invalid key with HTTP 404 + success:false.
            Err(ureq::Error::Status(_, r)) => match r.into_json() {
                Ok(j) => j,
                Err(_) => return Verify::Invalid,
            },
            Err(_) => return Verify::Network,
        };
        let ok = json.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
        let refunded = json
            .get("purchase")
            .and_then(|p| p.get("refunded"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if ok && !refunded {
            Verify::Valid
        } else {
            Verify::Invalid
        }
    }

    /// Open a URL in the default browser, per OS.
    fn open_url(url: &str) {
        #[cfg(target_os = "macos")]
        let (cmd, args): (&str, &[&str]) = ("open", &[]);
        #[cfg(target_os = "windows")]
        let (cmd, args): (&str, &[&str]) = ("cmd", &["/C", "start", ""]);
        #[cfg(all(unix, not(target_os = "macos")))]
        let (cmd, args): (&str, &[&str]) = ("xdg-open", &[]);
        let _ = std::process::Command::new(cmd).args(args).arg(url).spawn();
    }
}

// ── No-op stub (Mac App Store / feature off) ─────────────────────────
#[cfg(not(feature = "license"))]
mod imp {
    use super::Gate;

    /// Stub: there is no trial or paywall in this build.
    pub struct License;

    impl License {
        pub fn load() -> Self {
            License
        }
        pub fn gate(&self) -> Gate {
            Gate::Full
        }
        pub fn price() -> &'static str {
            ""
        }
        pub fn open_purchase(&self) {}
        pub fn activate_from_clipboard(&mut self) {}
        pub fn reverify_if_stale(&mut self) {}
        pub fn transient_message(&mut self) -> Option<String> {
            None
        }
    }
}
