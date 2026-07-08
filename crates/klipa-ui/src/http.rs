//! Ultra-thin HTTP client that shells out to the system's `curl`.
//!
//! klipa's identity is "small, fast, no bloat". Bundling `ureq` + `rustls`
//! for a couple of occasional GETs (weather, update check) would cost
//! ~900 KB of binary size, a big price for network calls the average
//! user makes zero times. `curl` is bundled by every OS klipa targets -
//! macOS, Windows 10 build 17063+ (2018), and every desktop Linux - so
//! this module is a rounding error and the binary stays tiny.
//!
//! If `curl` is missing at runtime, every call returns `None` and the
//! caller degrades gracefully (weather shows no temperature). No panics,
//! no user-visible errors. License activation does NOT use this module -
//! it verifies the signed `.klipa` file offline.

#[cfg(any(feature = "weather", not(feature = "mas")))]
use std::process::{Command, Stdio};
#[cfg(any(feature = "weather", not(feature = "mas")))]
use std::time::Duration;

/// GET a URL and return the response body. `None` on any failure.
/// Compiled in when the weather feature or a non-App-Store build wants
/// it (weather, updater). The strict "mas" build without weather has
/// no HTTP GET consumer and this function is dropped.
#[cfg(any(feature = "weather", not(feature = "mas")))]
pub fn get(url: &str, timeout: Duration) -> Option<Vec<u8>> {
    let out = Command::new("curl")
        .arg("-sSL") // silent, show errors, follow redirects
        .arg("--max-time")
        .arg(timeout.as_secs().max(1).to_string())
        .arg(url)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    Some(out.stdout)
}
