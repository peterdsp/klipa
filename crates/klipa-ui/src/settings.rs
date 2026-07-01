//! Persistent user preferences (currently just the menu bar display).
//!
//! Kept intentionally tiny: one JSON file next to history.json. New
//! fields should default to today's behavior so upgrading never breaks
//! anyone's setup.

use crate::paths;
use crate::weather::WeatherState;
use serde::{Deserialize, Serialize};
use time::{format_description::FormatItem, macros::format_description, OffsetDateTime};

/// What to show next to the menu bar icon.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MenubarDisplay {
    /// Just the monochrome clipboard glyph (backwards-compatible default).
    #[default]
    IconOnly,
    /// Today's date (e.g. "Wed 30").
    Date,
    /// Current temperature at the user's location (e.g. "22°").
    Temperature,
    /// Date and temperature (e.g. "Wed 30  22°").
    Both,
}

impl MenubarDisplay {
    pub fn needs_weather(self) -> bool {
        matches!(self, Self::Temperature | Self::Both)
    }
    pub fn needs_date(self) -> bool {
        matches!(self, Self::Date | Self::Both)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub menubar_display: MenubarDisplay,
}

impl Settings {
    /// Read settings.json, or return defaults if it's missing / invalid.
    pub fn load() -> Self {
        paths::settings_file()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Best-effort atomic save. Failures are logged and ignored - a
    /// missing settings file just falls back to defaults on next launch.
    pub fn save(&self) {
        let Some(path) = paths::settings_file() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(json) = serde_json::to_vec_pretty(self) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, &json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Compose the text shown next to the menu bar icon, based on the
/// user's display preference. Returns `None` for `IconOnly`.
///
/// `weather` is only queried when the display mode actually wants a
/// temperature, so a user in "Date" mode never hits the network.
pub fn menubar_title(display: MenubarDisplay, weather: &mut WeatherState) -> Option<String> {
    if matches!(display, MenubarDisplay::IconOnly) {
        return None;
    }
    let date = display.needs_date().then(format_date);
    let temp = weather.label(display.needs_weather());
    match (date, temp) {
        (Some(d), Some(t)) => Some(format!("{d}  {t}")),
        (Some(d), None) => Some(d),
        (None, Some(t)) => Some(t),
        (None, None) => None,
    }
}

/// Local-time short date like "Wed 30".
fn format_date() -> String {
    const FMT: &[FormatItem<'_>] =
        format_description!("[weekday repr:short] [day padding:none]");
    // Prefer the local timezone; fall back to UTC only if the platform
    // does not expose it (some sandboxed environments strip it).
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(FMT).unwrap_or_default()
}
