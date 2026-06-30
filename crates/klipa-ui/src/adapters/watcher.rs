//! Clipboard watcher - polls the OS clipboard and feeds events into
//! [`HistoryService::ingest`]. Notifies the UI via the provided callback.

use super::clipboard::ArboardClipboard;
use klipa_core::HistoryService;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// `locked` is set by the main loop when the trial has lapsed and the
/// app is unlicensed; while locked we stop recording new clipboard
/// entries (the paywall is showing instead of history).
pub async fn run<F: Fn() + Send + Sync + 'static>(
    history: Arc<HistoryService>,
    locked: Arc<AtomicBool>,
    on_change: F,
) {
    let cb = ArboardClipboard::new();
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    loop {
        interval.tick().await;
        if locked.load(Ordering::Acquire) {
            continue;
        }
        if let Some(ev) = cb.poll_once() {
            match history.ingest(ev).await {
                Ok(_) => on_change(),
                Err(e) => tracing::warn!(?e, "ingest failed"),
            }
        }
    }
}
