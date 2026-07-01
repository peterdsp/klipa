//! Best-effort current temperature for the menu bar display.
//!
//! Only reached when the user picks a temperature-showing menu bar
//! mode; the default (icon only) never hits the network. Two public
//! endpoints are used, both free and keyless:
//!
//! * `ip-api.com/json`         - approximate lat/lon from the caller's IP
//! * `api.open-meteo.com`      - the current temperature at those coords
//!
//! Location is cached for 24 h; temperature for 10 min, so a running
//! app makes about 6 requests per hour when this mode is enabled.
//!
//! When the `weather` feature is off (e.g. a minimal build) the whole
//! module compiles to a no-op that always returns `None`.

#[cfg(feature = "weather")]
use std::time::{Duration, Instant};

/// Fetched-and-cached current temperature (integer °C).
pub struct WeatherState {
    #[cfg(feature = "weather")]
    cache: imp::Cache,
}

impl WeatherState {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "weather")]
            cache: imp::Cache::default(),
        }
    }

    /// Return the current temperature label ("22°"), refreshing the
    /// cache if it is stale. Non-blocking-ish: the HTTP calls each have
    /// a short timeout and are made on the caller's thread, so this
    /// runs on the tokio blocking pool via `spawn_blocking`.
    ///
    /// `_enabled` = whether the user's display mode wants weather.
    /// When false we do not touch the cache at all.
    pub fn label(&mut self, _enabled: bool) -> Option<String> {
        #[cfg(feature = "weather")]
        {
            if !_enabled {
                return None;
            }
            let temp = imp::current_temperature_c(&mut self.cache)?;
            Some(format!("{}\u{00b0}", temp))
        }
        #[cfg(not(feature = "weather"))]
        {
            None
        }
    }
}

/// How long we wait for a single HTTP response. Kept short so a slow
/// network never freezes the menu bar refresh tick.
#[cfg(feature = "weather")]
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(feature = "weather")]
mod imp {
    use super::*;

    const LOCATION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
    const TEMP_TTL: Duration = Duration::from_secs(10 * 60);

    #[derive(Default)]
    pub(super) struct Cache {
        location: Option<((f64, f64), Instant)>,
        temperature: Option<(i16, Instant)>,
    }

    /// Return a rounded-to-int Celsius temperature, refreshing either
    /// leg of the cache when it goes stale.
    pub(super) fn current_temperature_c(cache: &mut Cache) -> Option<i16> {
        if let Some((t, at)) = cache.temperature {
            if at.elapsed() < TEMP_TTL {
                return Some(t);
            }
        }
        let (lat, lon) = resolve_location(cache)?;
        let t = fetch_temperature(lat, lon)?;
        cache.temperature = Some((t, Instant::now()));
        Some(t)
    }

    fn resolve_location(cache: &mut Cache) -> Option<(f64, f64)> {
        if let Some((coords, at)) = cache.location {
            if at.elapsed() < LOCATION_TTL {
                return Some(coords);
            }
        }
        let coords = ip_geolocate()?;
        cache.location = Some((coords, Instant::now()));
        Some(coords)
    }

    /// Ask ip-api.com where the caller's IP lives. Free, no key, no
    /// account. Returns None on any failure.
    fn ip_geolocate() -> Option<(f64, f64)> {
        let body = crate::http::get(
            "http://ip-api.com/json/?fields=status,lat,lon",
            HTTP_TIMEOUT,
        )?;
        let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
        if json.get("status").and_then(|v| v.as_str()) != Some("success") {
            return None;
        }
        let lat = json.get("lat")?.as_f64()?;
        let lon = json.get("lon")?.as_f64()?;
        Some((lat, lon))
    }

    /// Ask Open-Meteo for the current temperature at (lat, lon).
    /// Free, no key. Response shape: {"current_weather":{"temperature":22.5}}.
    fn fetch_temperature(lat: f64, lon: f64) -> Option<i16> {
        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current_weather=true"
        );
        let body = crate::http::get(&url, HTTP_TIMEOUT)?;
        let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let t = json
            .get("current_weather")?
            .get("temperature")?
            .as_f64()?;
        Some(t.round() as i16)
    }
}
