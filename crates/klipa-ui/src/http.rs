//! Ultra-thin HTTP client that shells out to the system's `curl`.
//!
//! klipa's identity is "small, fast, no bloat". Bundling `ureq` + `rustls`
//! for two occasional GETs (weather) and one occasional POST (license
//! verify) cost ~900 KB of binary size, which is a big price to pay for
//! network calls the average user makes zero times. `curl` is bundled
//! by every OS klipa targets - macOS, Windows 10 build 17063+ (2018),
//! and every desktop Linux - so this module is a rounding error and the
//! binary stays around 1.3 MB.
//!
//! If `curl` is missing at runtime, every call returns `None` and the
//! caller degrades gracefully (weather shows no temperature; license
//! stays in its current state). No panics, no user-visible errors.

#[cfg(any(feature = "weather", feature = "license", not(feature = "mas")))]
use std::process::{Command, Stdio};
#[cfg(any(feature = "weather", feature = "license", not(feature = "mas")))]
use std::time::Duration;
#[cfg(feature = "license")]
use std::io::Write;

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

/// POST a JSON body via stdin (so it never appears in the command line
/// nor a `ps` listing) and return the response body. Used to talk to the
/// Ko-fi license server's `/activate` endpoint, which reads JSON. Returns
/// the body even on HTTP 4xx (the server sends a meaningful JSON error,
/// e.g. "no license on file", alongside a 404), and `None` only when
/// there was no response at all (treated as a network failure upstream).
#[cfg(feature = "license")]
pub fn post_json(url: &str, json_body: &str, timeout: Duration) -> Option<Vec<u8>> {
    let mut child = Command::new("curl")
        .arg("-sSL")
        .arg("--max-time")
        .arg(timeout.as_secs().max(1).to_string())
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("--data-binary")
        .arg("@-") // read the body from stdin
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child
        .stdin
        .as_mut()?
        .write_all(json_body.as_bytes())
        .ok()?;
    let out = child.wait_with_output().ok()?;
    if out.stdout.is_empty() {
        None
    } else {
        Some(out.stdout)
    }
}
