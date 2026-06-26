//! Clipboard watcher - polls the OS clipboard and feeds events into
//! [`HistoryService::ingest`]. Notifies the UI via the provided callback.

use super::clipboard::ArboardClipboard;
use klipa_core::HistoryService;
use std::sync::Arc;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub async fn run<F: Fn() + Send + Sync + 'static>(history: Arc<HistoryService>, on_change: F) {
    let cb = ArboardClipboard::new();
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    loop {
        interval.tick().await;
        if let Some(ev) = cb.poll_once() {
            match history.ingest(ev).await {
                Ok(_) => on_change(),
                Err(e) => tracing::warn!(?e, "ingest failed"),
            }
        }
    }
}
