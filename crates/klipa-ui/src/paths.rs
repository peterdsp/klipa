//! Where klipa keeps its data on disk (one place, used by the storage
//! adapter, the clipboard adapter, and the tray thumbnails).

use std::path::PathBuf;

pub fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "peterdsp", "klipa")
        .map(|d| d.data_dir().to_path_buf())
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
