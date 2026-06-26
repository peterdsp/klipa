//! ClipboardSource adapter using `arboard`.

use async_trait::async_trait;
use klipa_core::{
    ClipboardSource, CoreError, HistoryItem, ItemContent, ItemKind, PasteboardEvent,
};
use std::sync::Mutex;
use tokio::sync::Notify;

pub struct ArboardClipboard {
    last_text: Mutex<Option<String>>,
    notify: Notify,
}

impl ArboardClipboard {
    pub fn new() -> Self {
        Self {
            last_text: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    pub fn poll_once(&self) -> Option<PasteboardEvent> {
        let mut cb = arboard::Clipboard::new().ok()?;
        let current = cb.get_text().ok()?;
        let mut last = self.last_text.lock().ok()?;
        if last.as_deref() == Some(current.as_str()) {
            return None;
        }
        *last = Some(current.clone());
        Some(PasteboardEvent {
            contents: vec![ItemContent {
                kind: ItemKind::Text,
                value: current,
            }],
            source_application: frontmost_app(),
        })
    }
}

/// Best-effort frontmost-application name (NSWorkspace on macOS,
/// GetForegroundWindow on Windows, _NET_ACTIVE_WINDOW on X11).
/// Returns None on Wayland and other unsupported environments.
///
/// Compiled out when the `frontmost` feature is off (e.g. the
/// sandboxed Mac App Store build), where it always returns None.
#[cfg(feature = "frontmost")]
fn frontmost_app() -> Option<String> {
    match active_win_pos_rs::get_active_window() {
        Ok(win) => {
            let name = win.app_name.trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        }
        Err(_) => None,
    }
}

#[cfg(not(feature = "frontmost"))]
fn frontmost_app() -> Option<String> {
    None
}

#[async_trait]
impl ClipboardSource for ArboardClipboard {
    async fn next(&self) -> klipa_core::Result<Option<PasteboardEvent>> {
        self.notify.notified().await;
        Ok(None)
    }

    async fn write(&self, item: &HistoryItem) -> klipa_core::Result<()> {
        let text = item
            .contents
            .iter()
            .find(|c| matches!(c.kind, ItemKind::Text))
            .map(|c| c.value.clone())
            .ok_or_else(|| CoreError::Invalid("no text content".into()))?;
        // arboard is synchronous and fast; callers drive this from the
        // UI event loop, so we run it inline rather than on a runtime.
        let mut cb = arboard::Clipboard::new().map_err(|e| CoreError::Clipboard(e.to_string()))?;
        // Record what we just wrote so the watcher's next poll doesn't
        // re-ingest it as a brand-new copy.
        if let Ok(mut last) = self.last_text.lock() {
            *last = Some(text.clone());
        }
        cb.set_text(text).map_err(|e| CoreError::Clipboard(e.to_string()))?;
        Ok(())
    }

    async fn clear(&self) -> klipa_core::Result<()> {
        let mut cb = arboard::Clipboard::new().map_err(|e| CoreError::Clipboard(e.to_string()))?;
        cb.clear().map_err(|e| CoreError::Clipboard(e.to_string()))?;
        Ok(())
    }
}
