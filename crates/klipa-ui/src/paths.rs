//! Where klipa keeps its data on disk (one place, used by the storage
//! adapter, the clipboard adapter, and the tray thumbnails).

use std::path::PathBuf;

pub fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "peterdsp", "klipa").map(|d| d.data_dir().to_path_buf())
}

/// The single local history file (text + image references).
pub fn history_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("history.json"))
}

/// Directory holding the full PNGs for image entries (kept out of the
/// history file so it - and memory - stays tiny).
pub fn images_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("images"))
}

/// Full path to one image entry's PNG, by its reference id.
pub fn image_path(id: &str) -> Option<PathBuf> {
    images_dir().map(|d| d.join(format!("{id}.png")))
}

/// Trial + license state for the paid (non-App-Store) build.
#[cfg_attr(not(feature = "license"), allow(dead_code))]
pub fn license_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("license.json"))
}

/// User preferences (menu bar display, weather location, ...).
pub fn settings_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("settings.json"))
}

/// Legacy sentinel (klipa <= 0.5.4): a bare boolean file written while a
/// lid-closed session had `disablesleep` set. It carries no snapshot of
/// the value to restore to, so on upgrade it is migrated as an
/// ambiguous-ownership recovery rather than trusted for a prior value.
/// macOS (non-App-Store) build only; harmless to compute elsewhere.
#[cfg_attr(any(not(target_os = "macos"), feature = "mas"), allow(dead_code))]
pub fn lid_awake_marker() -> Option<PathBuf> {
    data_dir().map(|d| d.join("lid_awake.on"))
}

/// Recovery journal for a lid-closed override. Because `disablesleep`
/// outlives a crash, a force-quit, or a reboot, this records the value
/// observed before klipa changed it (so the exact prior value is
/// restored, not a blind zero) and the boot session it belongs to. Its
/// presence at startup means a restore may still be owed, and it is
/// removed only after a restore is verified. macOS (non-App-Store) build
/// only; harmless to compute elsewhere.
#[cfg_attr(any(not(target_os = "macos"), feature = "mas"), allow(dead_code))]
pub fn lid_awake_journal() -> Option<PathBuf> {
    data_dir().map(|d| d.join("lid_awake.json"))
}
