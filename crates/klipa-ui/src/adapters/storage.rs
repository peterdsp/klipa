//! HistoryStore adapter - a plain local file on the user's device.
//! No server, no network, nothing logged or uploaded: the entire
//! history lives in one file under the user's data dir and is
//! rewritten atomically on each change.

use crate::paths;
use async_trait::async_trait;
use base64::Engine as _;
use klipa_core::{CoreError, HistoryItem, HistoryItemId, HistoryStore, ItemKind};
use std::path::PathBuf;
use std::sync::Mutex;
use uuid::Uuid;

/// Delete the on-disk PNG backing any image content of `item`.
fn remove_image_files(item: &HistoryItem) {
    for c in &item.contents {
        if matches!(c.kind, ItemKind::Image) {
            if let Some(path) = paths::image_path(&c.value) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Convert any image content that still holds inline base64 bytes into
/// an on-disk PNG referenced by a short id. Returns true if anything
/// changed (so the caller knows to re-write the history file).
fn migrate_inline_images(items: &mut [HistoryItem]) -> bool {
    let mut changed = false;
    for item in items.iter_mut() {
        for c in item.contents.iter_mut() {
            if !matches!(c.kind, ItemKind::Image) {
                continue;
            }
            // Already a file reference?
            if paths::image_path(&c.value).map(|p| p.exists()).unwrap_or(false) {
                continue;
            }
            // Otherwise try to treat the value as inline base64 PNG.
            if let Ok(png) = base64::engine::general_purpose::STANDARD.decode(c.value.as_bytes()) {
                let id = Uuid::new_v4().to_string();
                if let Some(path) = paths::image_path(&id) {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if std::fs::write(&path, &png).is_ok() {
                        c.value = id;
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

pub struct JsonStore {
    path: PathBuf,
    items: Mutex<Vec<HistoryItem>>,
}

impl JsonStore {
    pub async fn new() -> klipa_core::Result<Self> {
        let path =
            paths::history_file().ok_or_else(|| CoreError::Storage("no data dir".into()))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CoreError::Storage(e.to_string()))?;
        }
        let mut items: Vec<HistoryItem> = if path.exists() {
            let bytes = std::fs::read(&path).map_err(|e| CoreError::Storage(e.to_string()))?;
            // Tolerate an empty or corrupt file by starting fresh.
            serde_json::from_slice(&bytes).unwrap_or_default()
        } else {
            Vec::new()
        };
        let store = Self {
            path,
            items: Mutex::new(Vec::new()),
        };
        // One-time migration: older versions inlined image bytes (base64)
        // in the history file, which bloated memory. Move any such image
        // out to its own file and keep only a small reference.
        if migrate_inline_images(&mut items) {
            store.flush(&items)?;
        }
        *store.lock() = items;
        Ok(store)
    }

    /// Serialize the whole list and replace the file atomically
    /// (write to a sibling temp file, then rename).
    fn flush(&self, items: &[HistoryItem]) -> klipa_core::Result<()> {
        let json = serde_json::to_vec(items).map_err(|e| CoreError::Storage(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &json).map_err(|e| CoreError::Storage(e.to_string()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<HistoryItem>> {
        self.items.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[async_trait]
impl HistoryStore for JsonStore {
    async fn load(&self) -> klipa_core::Result<Vec<HistoryItem>> {
        Ok(self.lock().clone())
    }

    async fn insert(&self, item: &HistoryItem) -> klipa_core::Result<()> {
        let mut items = self.lock();
        items.retain(|i| i.id != item.id);
        items.insert(0, item.clone());
        let snapshot = items.clone();
        drop(items);
        self.flush(&snapshot)
    }

    async fn update(&self, item: &HistoryItem) -> klipa_core::Result<()> {
        let mut items = self.lock();
        if let Some(slot) = items.iter_mut().find(|i| i.id == item.id) {
            *slot = item.clone();
        } else {
            items.insert(0, item.clone());
        }
        let snapshot = items.clone();
        drop(items);
        self.flush(&snapshot)
    }

    async fn delete(&self, id: HistoryItemId) -> klipa_core::Result<()> {
        let mut items = self.lock();
        if let Some(it) = items.iter().find(|i| i.id == id) {
            remove_image_files(it);
        }
        items.retain(|i| i.id != id);
        let snapshot = items.clone();
        drop(items);
        self.flush(&snapshot)
    }

    async fn clear_unpinned(&self) -> klipa_core::Result<()> {
        let mut items = self.lock();
        items.iter().filter(|i| !i.is_pinned()).for_each(remove_image_files);
        items.retain(|i| i.is_pinned());
        let snapshot = items.clone();
        drop(items);
        self.flush(&snapshot)
    }

    async fn clear_all(&self) -> klipa_core::Result<()> {
        let mut items = self.lock();
        items.iter().for_each(remove_image_files);
        items.clear();
        drop(items);
        self.flush(&[])
    }
}
