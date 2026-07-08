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
//! The network is never touched on the caller's thread: `label` only
//! reads the shared cache and, when it is stale, kicks off a detached
//! worker thread to refresh it. That keeps the menu bar responsive even
//! on a slow or unreachable network, where each HTTP call can sit until
//! its timeout.
//!
//! When the `weather` feature is off (e.g. a minimal build) the whole
//! module compiles to a no-op that always returns `None`.

#[cfg(feature = "weather")]
use std::sync::Arc;

/// Handle to the temperature cache, refreshed off the main thread.
pub struct WeatherState {
    #[cfg(feature = "weather")]
    shared: Arc<imp::Shared>,
}

impl WeatherState {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "weather")]
            shared: Arc::new(imp::Shared::default()),
        }
    }

    /// Return the current temperature label ("22°") from cache without
    /// ever blocking the caller. When the cached value is stale (or
    /// absent) this spawns a background refresh and returns the last
    /// known value in the meantime - `None` until the first fetch lands.
    /// Cheap enough to call on every event-loop tick.
    ///
    /// `_enabled` = whether the user's display mode wants weather. When
    /// false we neither read nor fetch.
    pub fn label(&self, _enabled: bool) -> Option<String> {
        #[cfg(feature = "weather")]
        {
            if !_enabled {
                return None;
            }
            imp::current_label(&self.shared)
        }
        #[cfg(not(feature = "weather"))]
        {
            let _ = _enabled;
            None
        }
    }
}

#[cfg(feature = "weather")]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// How long we wait for a single HTTP response. Kept short so a slow
    /// network never keeps a worker thread (or the retry cadence) stuck.
    const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

    const LOCATION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
    const TEMP_TTL: Duration = Duration::from_secs(10 * 60);
    /// Minimum spacing between fetch attempts, so a persistently failing
    /// network (or a machine without `curl`) is retried at a sane pace
    /// instead of once per event-loop tick.
    const RETRY_MIN: Duration = Duration::from_secs(60);

    #[derive(Default)]
    struct Cache {
        location: Option<((f64, f64), Instant)>,
        temperature: Option<(i16, Instant)>,
        /// When the last fetch was *started*, to rate-limit retries.
        last_attempt: Option<Instant>,
    }

    #[derive(Default)]
    pub(super) struct Shared {
        cache: Mutex<Cache>,
        /// True while a background fetch is in flight, so we never spawn
        /// two at once.
        fetching: AtomicBool,
    }

    /// Read the cached temperature label, spawning a background refresh
    /// when the value is stale and none is already running. Never blocks
    /// on the network.
    pub(super) fn current_label(shared: &Arc<Shared>) -> Option<String> {
        let mut cache = shared.cache.lock().unwrap();

        let fresh = cache
            .temperature
            .is_some_and(|(_, at)| at.elapsed() < TEMP_TTL);

        if !fresh
            && !shared.fetching.load(Ordering::Acquire)
            && cache.last_attempt.is_none_or(|a| a.elapsed() >= RETRY_MIN)
        {
            cache.last_attempt = Some(Instant::now());
            shared.fetching.store(true, Ordering::Release);
            spawn_refresh(Arc::clone(shared));
        }

        cache.temperature.map(|(t, _)| format!("{t}\u{00b0}"))
    }

    /// Refresh the location (if its 24 h TTL lapsed) then the
    /// temperature on a detached worker thread. Network calls run
    /// without the cache lock held; only the quick writes take it.
    fn spawn_refresh(shared: Arc<Shared>) {
        std::thread::spawn(move || {
            let temp = resolve_location(&shared).and_then(|(lat, lon)| fetch_temperature(lat, lon));
            if let Some(t) = temp {
                if let Ok(mut cache) = shared.cache.lock() {
                    cache.temperature = Some((t, Instant::now()));
                }
            }
            shared.fetching.store(false, Ordering::Release);
        });
    }

    fn resolve_location(shared: &Arc<Shared>) -> Option<(f64, f64)> {
        if let Ok(cache) = shared.cache.lock() {
            if let Some((coords, at)) = cache.location {
                if at.elapsed() < LOCATION_TTL {
                    return Some(coords);
                }
            }
        }
        let coords = ip_geolocate()?;
        if let Ok(mut cache) = shared.cache.lock() {
            cache.location = Some((coords, Instant::now()));
        }
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
