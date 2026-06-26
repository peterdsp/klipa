//! HistoryStore adapter - a plain local file on the user's device.
//! No server, no network, nothing logged or uploaded: the entire
//! history lives in one file under the user's data dir and is
//! rewritten atomically on each change.

use async_trait::async_trait;
use klipa_core::{CoreError, HistoryItem, HistoryItemId, HistoryStore};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct JsonStore {
    path: PathBuf,
    items: Mutex<Vec<HistoryItem>>,
}

impl JsonStore {
    pub async fn new() -> klipa_core::Result<Self> {
        let path = data_file_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CoreError::Storage(e.to_string()))?;
        }
        let items = if path.exists() {
            let bytes = std::fs::read(&path).map_err(|e| CoreError::Storage(e.to_string()))?;
            // Tolerate an empty or corrupt file by starting fresh.
            serde_json::from_slice(&bytes).unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Self {
            path,
            items: Mutex::new(items),
        })
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

fn data_file_path() -> klipa_core::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "peterdsp", "klipa")
        .ok_or_else(|| CoreError::Storage("no project dir".into()))?;
    Ok(dirs.data_dir().join("history.json"))
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
        items.retain(|i| i.id != id);
        let snapshot = items.clone();
        drop(items);
        self.flush(&snapshot)
    }

    async fn clear_unpinned(&self) -> klipa_core::Result<()> {
        let mut items = self.lock();
        items.retain(|i| i.is_pinned());
        let snapshot = items.clone();
        drop(items);
        self.flush(&snapshot)
    }

    async fn clear_all(&self) -> klipa_core::Result<()> {
        let mut items = self.lock();
        items.clear();
        drop(items);
        self.flush(&[])
    }
}
