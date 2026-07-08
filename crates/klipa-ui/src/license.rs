//! Trial + license gate for the paid (non-App-Store) builds.
//!
//! klipa is free to use in full for [`TRIAL_DAYS`] days. After that the
//! app locks to a paywall until the buyer activates. Payment is handled
//! on Ko-fi; the self-hosted license server (see
//! `scripts/pi-license-server/`) emails each buyer an Ed25519-signed
//! license tied to the email they used at checkout. To activate, the
//! user copies that email to the clipboard and clicks Activate: the app
//! posts the email to the server, gets the signed license back, and
//! verifies the signature offline against the embedded public key.
//!
//! The Mac App Store build is a paid app, so it carries no licensing
//! code at all - everything here is behind the `license` feature, and a
//! no-op stub stands in when the feature is off.
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

    /// Product name the license server signs into each blob. Must match
    /// the server's product key for klipa.
    const PRODUCT: &str = "klipa";
    /// This build's version, used to enforce the license `min_version`.
    const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

    /// Ed25519 public key (base64, raw 32 bytes) paired with the license
    /// server's signing key. Public by nature - safe to embed. A license
    /// only validates if its signature checks out against this key, so a
    /// license minted for another product (different key) cannot unlock
    /// klipa.
    const LICENSE_PUBKEY_B64: &str = "jbSjJelSCv+gs0bXuaVnsKsu/IyhGGUrjJ+ProKrLPo=";

    /// Where the app posts `{email, product}` to fetch a signed license.
    /// Override at build time with `KLIPA_LICENSE_ENDPOINT=...`.
    const LICENSE_ENDPOINT: &str = match option_env!("KLIPA_LICENSE_ENDPOINT") {
        Some(v) => v,
        None => "https://licenses.peterdsp.dev/activate",
    };

    /// Where the *Unlock* button sends buyers. Override at build time
    /// with `KLIPA_PURCHASE_URL=...`.
    const PURCHASE_URL: &str = match option_env!("KLIPA_PURCHASE_URL") {
        Some(v) => v,
        None => "https://ko-fi.com/s/4e1cf2ac40",
    };

    /// Persisted trial + license state (`license.json`).
    #[derive(Serialize, Deserialize, Default)]
    struct State {
        #[serde(with = "time::serde::rfc3339::option", default)]
        trial_started_at: Option<OffsetDateTime>,
        /// The Ko-fi email this install is licensed to (lowercased).
        licensed_email: Option<String>,
        /// The signed license blob returned by the server, kept as proof
        /// and so activation survives offline restarts.
        license: Option<serde_json::Value>,
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
        /// Load (or start) the trial. Stamps the trial clock on first
        /// run, and mirrors that stamp into the OS keychain so a plain
        /// "uninstall the app and delete the data folder" no longer
        /// resets the 7 days: the next fresh install reads back the
        /// keychain value and honours the original trial start.
        pub fn load() -> Self {
            let mut state: State = paths::license_file()
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            // Merge in a keychain-backed trial start if it is older
            // than whatever is on disk. This is what stops reinstalls.
            let keyring_start = keyring_trial_start();
            state.trial_started_at = earliest(state.trial_started_at, keyring_start);
            if state.trial_started_at.is_none() {
                state.trial_started_at = Some(OffsetDateTime::now_utc());
            }
            // Push back to the keychain so a future reinstall picks it up.
            if let Some(t) = state.trial_started_at {
                write_keyring_trial_start(t);
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

        /// Activate using whatever text is on the clipboard - now the
        /// buyer's Ko-fi email, the natural gesture for a clipboard app:
        /// copy your email, click Activate. Fetches the signed license
        /// from the server and stores it on success.
        pub fn activate_from_clipboard(&mut self) {
            let text = arboard::Clipboard::new()
                .ok()
                .and_then(|mut c| c.get_text().ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if !looks_like_email(&text) {
                self.flash("Copy the email you used on Ko-fi, then Activate");
                return;
            }
            let email = text.to_lowercase();
            match request_license(&email) {
                (Verify::Valid, Some(blob)) => {
                    let now = OffsetDateTime::now_utc();
                    self.state.licensed_email = Some(email);
                    self.state.license = Some(blob);
                    self.state.activated_at = Some(now);
                    self.state.last_verified_at = Some(now);
                    self.save();
                    self.flash("Activated - thank you!");
                }
                (Verify::Invalid, _) => {
                    self.flash("No license found for that email - use your Ko-fi address")
                }
                (Verify::Network, _) => self.flash("Could not reach the license server"),
                (Verify::Valid, None) => self.flash("Could not reach the license server"),
            }
        }

        /// Re-check an activated license in the background now and then,
        /// so a refunded/revoked purchase eventually stops working.
        /// Best-effort: network failures keep the license (offline
        /// grace).
        pub fn reverify_if_stale(&mut self) {
            let (Some(email), Some(last)) =
                (self.state.licensed_email.clone(), self.state.last_verified_at)
            else {
                return;
            };
            if OffsetDateTime::now_utc() - last < time::Duration::days(14) {
                return;
            }
            match request_license(&email) {
                (Verify::Valid, _) => {
                    self.state.last_verified_at = Some(OffsetDateTime::now_utc());
                    self.save();
                }
                (Verify::Invalid, _) => {
                    // Revoked/refunded: drop the activation.
                    self.state.activated_at = None;
                    self.state.licensed_email = None;
                    self.state.license = None;
                    self.save();
                }
                // Network: keep the license as-is (offline grace).
                (Verify::Network, _) => {}
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

    /// Return the earlier of two optional timestamps. Used to prefer
    /// whichever trial-start stamp (file vs keychain) is oldest, so
    /// tampering with one always yields the honest date.
    fn earliest(
        a: Option<OffsetDateTime>,
        b: Option<OffsetDateTime>,
    ) -> Option<OffsetDateTime> {
        match (a, b) {
            (Some(x), Some(y)) => Some(if x < y { x } else { y }),
            (x, y) => x.or(y),
        }
    }

    const KEYRING_SERVICE: &str = "dev.peterdsp.klipa";
    const KEYRING_ACCOUNT: &str = "trial_started_at";

    /// Read the trial start from the OS keychain, if present.
    fn keyring_trial_start() -> Option<OffsetDateTime> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).ok()?;
        let s = entry.get_password().ok()?;
        OffsetDateTime::parse(&s, &time::format_description::well_known::Rfc3339).ok()
    }

    /// Write (or overwrite) the trial start in the OS keychain. Best
    /// effort: silently ignored if the platform keychain is unavailable
    /// (e.g. headless Linux with no Secret Service running).
    fn write_keyring_trial_start(t: OffsetDateTime) {
        let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) else {
            return;
        };
        if let Ok(s) = t.format(&time::format_description::well_known::Rfc3339) {
            let _ = entry.set_password(&s);
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
        Network,
    }

    /// Cheap sanity check before we bother the network: is this plausibly
    /// an email address (one `@`, a dot after it, no spaces)?
    fn looks_like_email(s: &str) -> bool {
        let s = s.trim();
        if s.is_empty() || s.contains(char::is_whitespace) {
            return false;
        }
        match s.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty() && domain.contains('.') && !domain.starts_with('.')
                    && !domain.ends_with('.')
            }
            None => false,
        }
    }

    /// Ask the license server for `email`'s signed license, then verify
    /// the signature offline. Returns `(Valid, blob)` only when the
    /// server returned a license whose signature checks out for this
    /// product and email.
    fn request_license(email: &str) -> (Verify, Option<serde_json::Value>) {
        let body = serde_json::json!({ "email": email, "product": PRODUCT }).to_string();
        let Some(resp) = crate::http::post_json(LICENSE_ENDPOINT, &body, StdDuration::from_secs(10))
        else {
            // No response body at all: treat as a network hiccup so an
            // existing license keeps working offline.
            return (Verify::Network, None);
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&resp) else {
            return (Verify::Invalid, None);
        };
        // A license blob carries a signature; an error response ("no
        // license on file", "invalid email", ...) does not.
        if json.get("signature").is_none() {
            return (Verify::Invalid, None);
        }
        if verify_blob(&json, email) {
            (Verify::Valid, Some(json))
        } else {
            (Verify::Invalid, None)
        }
    }

    /// Verify a signed license blob: the Ed25519 signature must check out
    /// against the embedded public key, the product must be klipa, the
    /// email must match, and this build must satisfy `min_version`.
    ///
    /// The signed message is the canonical JSON of the five fields with
    /// sorted keys and no whitespace - byte-identical to what the Python
    /// server signs (`json.dumps(..., separators=(",",":"), sort_keys=True)`).
    /// Values are ASCII in practice (emails, ISO timestamps, semver), so
    /// serde_json and Python produce the same bytes.
    fn verify_blob(blob: &serde_json::Value, expect_email: &str) -> bool {
        let field = |k: &str| blob.get(k).and_then(|v| v.as_str());
        let (
            Some(email),
            Some(issued_at),
            Some(order_id),
            Some(product),
            Some(min_version),
            Some(sig_b64),
        ) = (
            field("email"),
            field("issued_at"),
            field("order_id"),
            field("product"),
            field("min_version"),
            field("signature"),
        )
        else {
            return false;
        };

        if product != PRODUCT {
            return false;
        }
        if !email.eq_ignore_ascii_case(expect_email) {
            return false;
        }
        if !version_at_least(CURRENT_VERSION, min_version) {
            return false;
        }

        // Rebuild the canonical signed message (sorted keys, compact).
        let mut canonical = std::collections::BTreeMap::new();
        canonical.insert("email", email);
        canonical.insert("issued_at", issued_at);
        canonical.insert("min_version", min_version);
        canonical.insert("order_id", order_id);
        canonical.insert("product", product);
        let Ok(message) = serde_json::to_vec(&canonical) else {
            return false;
        };

        verify_signature(&message, sig_b64)
    }

    /// Ed25519 verify `message` against `signature_b64` using the
    /// embedded public key. Any decode/verify failure returns false.
    fn verify_signature(message: &[u8], signature_b64: &str) -> bool {
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD;
        let (Ok(pk_bytes), Ok(sig_bytes)) = (
            engine.decode(LICENSE_PUBKEY_B64),
            engine.decode(signature_b64),
        ) else {
            return false;
        };
        let (Ok(pk), Ok(sig)) = (
            ed25519_compact::PublicKey::from_slice(&pk_bytes),
            ed25519_compact::Signature::from_slice(&sig_bytes),
        ) else {
            return false;
        };
        pk.verify(message, &sig).is_ok()
    }

    /// True if `have` (dotted numeric version) is >= `need`. Missing or
    /// non-numeric components compare as 0. Keeps us off a semver dep.
    fn version_at_least(have: &str, need: &str) -> bool {
        let parse = |s: &str| -> Vec<u64> {
            s.split('.')
                .map(|p| p.trim().parse::<u64>().unwrap_or(0))
                .collect()
        };
        let (h, n) = (parse(have), parse(need));
        let len = h.len().max(n.len());
        for i in 0..len {
            let a = h.get(i).copied().unwrap_or(0);
            let b = n.get(i).copied().unwrap_or(0);
            if a != b {
                return a > b;
            }
        }
        true // equal
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

        #[test]
        fn version_compare() {
            assert!(version_at_least("0.4.4", "0.4.0"));
            assert!(version_at_least("0.4.0", "0.4.0"));
            assert!(version_at_least("1.0.0", "0.9.9"));
            assert!(!version_at_least("0.3.9", "0.4.0"));
            assert!(version_at_least("0.4", "0.4.0"));
        }

        #[test]
        fn email_shape() {
            assert!(looks_like_email("buyer@example.com"));
            assert!(looks_like_email("a.b+tag@sub.domain.co"));
            assert!(!looks_like_email("not-an-email"));
            assert!(!looks_like_email("two @spaces.com"));
            assert!(!looks_like_email("@example.com"));
            assert!(!looks_like_email("nope@nodot"));
        }

        // A license blob really signed by the klipa server's private key
        // (issued for buyer@example.com, min_version 0.4.0). Verifies the
        // Rust canonicalisation + Ed25519 path matches the Python signer.
        const GOOD_BLOB: &str = r#"{"email": "buyer@example.com", "issued_at": "2026-07-08T00:00:00Z", "order_id": "tx-test", "product": "klipa", "min_version": "0.4.0", "signature": "WoQAQr8JfMkyP6B4/8Kkome2s0I0uk6b06rGWYq4aAmRVBvfIq7zhZewvDQpPbBxkyXTxF7FyRWDbvtwnAhCDw=="}"#;

        #[test]
        fn real_signature_verifies_and_tamper_fails() {
            let blob: serde_json::Value = serde_json::from_str(GOOD_BLOB).unwrap();
            // Correct email + product + version -> valid.
            assert!(verify_blob(&blob, "buyer@example.com"));
            // Wrong email for the same signature -> rejected.
            assert!(!verify_blob(&blob, "someone@else.com"));
            // Tampered field breaks the signature.
            let mut tampered = blob.clone();
            tampered["order_id"] = serde_json::json!("hacked");
            assert!(!verify_blob(&tampered, "buyer@example.com"));
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
