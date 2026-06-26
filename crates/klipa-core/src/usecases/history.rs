//! HistoryService - orchestrates a [`HistoryStore`] and a [`ClipboardSource`].
//!
//! Mirrors the responsibilities of `History.swift`: ingest new copies,
//! dedupe against the in-memory mirror, cap to a size limit, expose
//! the filtered/sorted view to callers.

use crate::domain::entities::{HistoryItem, HistoryItemId};
use crate::domain::ports::{ClipboardSource, HistoryStore, PasteboardEvent};
use crate::usecases::search::{SearchMode, Searcher};
use crate::usecases::sorter::Sorter;
use crate::Result;
use async_lock::RwLock;
use std::sync::Arc;

pub struct HistoryService {
    store: Arc<dyn HistoryStore>,
    clipboard: Arc<dyn ClipboardSource>,
    state: RwLock<HistoryState>,
    searcher: Searcher,
    sorter: Sorter,
}

#[derive(Default)]
struct HistoryState {
    all: Vec<HistoryItem>,
    max_size: usize,
}

impl HistoryService {
    pub fn new(
        store: Arc<dyn HistoryStore>,
        clipboard: Arc<dyn ClipboardSource>,
        max_size: usize,
    ) -> Self {
        Self {
            store,
            clipboard,
            state: RwLock::new(HistoryState {
                max_size,
                ..Default::default()
            }),
            searcher: Searcher::default(),
            sorter: Sorter::default(),
        }
    }

    /// Load from persistence into the in-memory mirror.
    pub async fn load(&self) -> Result<()> {
        let mut items = self.store.load().await?;
        self.sorter.sort(&mut items);
        let mut s = self.state.write().await;
        s.all = items;
        Ok(())
    }

    /// Apply a clipboard event: dedupe, persist, cap.
    pub async fn ingest(&self, ev: PasteboardEvent) -> Result<HistoryItem> {
        let item = HistoryItem::new(ev.contents, ev.source_application);

        let mut s = self.state.write().await;
        if let Some(existing_idx) = s.all.iter().position(|x| x.matches(&item)) {
            let mut existing = s.all.remove(existing_idx);
            existing.number_of_copies += 1;
            existing.last_copied_at = item.last_copied_at;
            self.store.update(&existing).await?;
            s.all.insert(0, existing.clone());
            return Ok(existing);
        }

        self.store.insert(&item).await?;
        s.all.insert(0, item.clone());

        // Enforce size cap (only unpinned count toward the limit).
        let cap = s.max_size;
        let mut unpinned_count = s.all.iter().filter(|i| !i.is_pinned()).count();
        if unpinned_count > cap {
            let mut to_remove: Vec<HistoryItemId> = vec![];
            // Walk from the tail (oldest) dropping unpinned until we fit.
            for it in s.all.iter().rev() {
                if !it.is_pinned() {
                    to_remove.push(it.id);
                    unpinned_count -= 1;
                    if unpinned_count <= cap {
                        break;
                    }
                }
            }
            s.all.retain(|i| !to_remove.contains(&i.id));
            drop(s);
            for id in to_remove {
                let _ = self.store.delete(id).await;
            }
            return Ok(item);
        }
        Ok(item)
    }

    pub async fn delete(&self, id: HistoryItemId) -> Result<()> {
        self.store.delete(id).await?;
        let mut s = self.state.write().await;
        s.all.retain(|i| i.id != id);
        Ok(())
    }

    pub async fn clear_unpinned(&self) -> Result<()> {
        self.store.clear_unpinned().await?;
        let mut s = self.state.write().await;
        s.all.retain(|i| i.is_pinned());
        Ok(())
    }

    pub async fn copy_to_clipboard(&self, id: HistoryItemId) -> Result<()> {
        let s = self.state.read().await;
        let item = s
            .all
            .iter()
            .find(|i| i.id == id)
            .ok_or(crate::CoreError::NotFound)?
            .clone();
        drop(s);
        self.clipboard.write(&item).await
    }

    /// Snapshot of all items, sorted with the configured sorter.
    pub async fn snapshot(&self) -> Vec<HistoryItem> {
        let s = self.state.read().await;
        s.all.clone()
    }

    /// Search the in-memory mirror.
    pub async fn query(&self, q: &str, mode: SearchMode) -> Vec<HistoryItem> {
        let s = self.state.read().await;
        self.searcher
            .search(q, &s.all, mode)
            .into_iter()
            .map(|r| r.item.clone())
            .collect()
    }
}
