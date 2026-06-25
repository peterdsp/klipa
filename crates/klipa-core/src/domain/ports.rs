//! Ports — traits the outer (adapter) layer must implement.
//!
//! The domain depends on these; nothing concrete lives here.

use crate::domain::entities::{HistoryItem, HistoryItemId, ItemContent};
use crate::Result;
use async_trait::async_trait;

/// Event emitted by [`ClipboardSource`] when the system clipboard changes.
#[derive(Debug, Clone)]
pub struct PasteboardEvent {
    pub contents: Vec<ItemContent>,
    pub source_application: Option<String>,
}

/// Watches the system clipboard and produces events. One implementation
/// per OS (arboard-based on Linux/Windows, NSPasteboard polling on macOS).
#[async_trait]
pub trait ClipboardSource: Send + Sync {
    /// Block-waits for the next clipboard change. Returns `Ok(None)` when
    /// the source has been shut down cleanly.
    async fn next(&self) -> Result<Option<PasteboardEvent>>;

    /// Write the given item back to the system clipboard.
    async fn write(&self, item: &HistoryItem) -> Result<()>;

    /// Clear the system clipboard.
    async fn clear(&self) -> Result<()>;
}

/// Durable storage for history items. Adapter typically wraps SQLite.
#[async_trait]
pub trait HistoryStore: Send + Sync {
    async fn load(&self) -> Result<Vec<HistoryItem>>;
    async fn insert(&self, item: &HistoryItem) -> Result<()>;
    async fn update(&self, item: &HistoryItem) -> Result<()>;
    async fn delete(&self, id: HistoryItemId) -> Result<()>;
    async fn clear_unpinned(&self) -> Result<()>;
    async fn clear_all(&self) -> Result<()>;
}
