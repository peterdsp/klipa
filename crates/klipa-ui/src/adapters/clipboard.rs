//! ClipboardSource adapter using `arboard`. Captures both text and
//! images (e.g. screenshots). Images are stored PNG-compressed so the
//! local history file stays small.

use crate::paths;
use async_trait::async_trait;
use base64::Engine as _;
use klipa_core::{
    ClipboardSource, CoreError, HistoryItem, ItemContent, ItemKind, PasteboardEvent,
};
use std::borrow::Cow;
use std::io::Cursor;
use std::sync::Mutex;
use tokio::sync::Notify;
use uuid::Uuid;

pub struct ArboardClipboard {
    /// Signature of the last thing we saw, so we don't re-capture the
    /// same clipboard contents on every poll. Text -> the text itself;
    /// image -> dimensions + a cheap content hash.
    last_sig: Mutex<Option<String>>,
    notify: Notify,
}

impl ArboardClipboard {
    pub fn new() -> Self {
        Self {
            last_sig: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    pub fn poll_once(&self) -> Option<PasteboardEvent> {
        let mut cb = arboard::Clipboard::new().ok()?;

        // Prefer text. Most copies are text and it's cheap to read.
        if let Ok(text) = cb.get_text() {
            if !text.is_empty() {
                let sig = format!("t:{text}");
                if self.is_new(&sig) {
                    return Some(PasteboardEvent {
                        contents: vec![ItemContent {
                            kind: ItemKind::Text,
                            value: text,
                        }],
                        source_application: frontmost_app(),
                    });
                }
                return None;
            }
        }

        // No text -> try an image (screenshots, copied pictures).
        if let Ok(img) = cb.get_image() {
            let sig = format!("i:{}x{}:{:016x}", img.width, img.height, hash(&img.bytes));
            if self.is_new(&sig) {
                if let Some(png) = encode_png(img.width, img.height, &img.bytes) {
                    // Write the full PNG to disk and keep only a short
                    // reference id in history, so memory/history stay tiny.
                    let id = Uuid::new_v4().to_string();
                    if let Some(path) = paths::image_path(&id) {
                        if let Some(dir) = path.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        if std::fs::write(&path, &png).is_ok() {
                            return Some(PasteboardEvent {
                                // Leading text label = the readable menu
                                // line; the image ref is what gets pasted.
                                contents: vec![
                                    ItemContent {
                                        kind: ItemKind::Text,
                                        value: format!("[Image {}x{}]", img.width, img.height),
                                    },
                                    ItemContent {
                                        kind: ItemKind::Image,
                                        value: id,
                                    },
                                ],
                                source_application: frontmost_app(),
                            });
                        }
                    }
                }
            }
        }
        None
    }

    /// Returns true (and records `sig`) if it differs from the last.
    fn is_new(&self, sig: &str) -> bool {
        let mut last = self.last_sig.lock().unwrap_or_else(|p| p.into_inner());
        if last.as_deref() == Some(sig) {
            return false;
        }
        *last = Some(sig.to_string());
        true
    }
}

/// Best-effort frontmost-application name. None when the `frontmost`
/// feature is off (e.g. the sandboxed App Store build).
#[cfg(feature = "frontmost")]
fn frontmost_app() -> Option<String> {
    match active_win_pos_rs::get_active_window() {
        Ok(win) => {
            let name = win.app_name.trim();
            (!name.is_empty()).then(|| name.to_string())
        }
        Err(_) => None,
    }
}

#[cfg(not(feature = "frontmost"))]
fn frontmost_app() -> Option<String> {
    None
}

/// FNV-1a hash - cheap content fingerprint for image dedup.
fn hash(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn encode_png(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, width as u32, height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(buf)
}

/// Load an image entry's PNG bytes. New entries reference a file on
/// disk; legacy entries inlined base64 PNG directly in the value.
pub fn read_image_png(value: &str) -> Option<Vec<u8>> {
    if let Some(path) = paths::image_path(value) {
        if let Ok(bytes) = std::fs::read(&path) {
            return Some(bytes);
        }
    }
    base64::engine::general_purpose::STANDARD
        .decode(value.as_bytes())
        .ok()
}

pub fn decode_png(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width as usize, info.height as usize, buf))
}

#[async_trait]
impl ClipboardSource for ArboardClipboard {
    async fn next(&self) -> klipa_core::Result<Option<PasteboardEvent>> {
        self.notify.notified().await;
        Ok(None)
    }

    async fn write(&self, item: &HistoryItem) -> klipa_core::Result<()> {
        let mut cb =
            arboard::Clipboard::new().map_err(|e| CoreError::Clipboard(e.to_string()))?;

        // Image item -> load the stored PNG and set it on the clipboard.
        if let Some(c) = item.contents.iter().find(|c| matches!(c.kind, ItemKind::Image)) {
            let png = read_image_png(&c.value)
                .ok_or_else(|| CoreError::Clipboard("image not found".into()))?;
            let (width, height, rgba) =
                decode_png(&png).ok_or_else(|| CoreError::Clipboard("bad image data".into()))?;
            cb.set_image(arboard::ImageData {
                width,
                height,
                bytes: Cow::Owned(rgba),
            })
            .map_err(|e| CoreError::Clipboard(e.to_string()))?;
            return Ok(());
        }

        // Text item.
        let text = item
            .contents
            .iter()
            .find(|c| matches!(c.kind, ItemKind::Text))
            .map(|c| c.value.clone())
            .ok_or_else(|| CoreError::Invalid("no writable content".into()))?;
        cb.set_text(text)
            .map_err(|e| CoreError::Clipboard(e.to_string()))?;
        Ok(())
    }

    async fn clear(&self) -> klipa_core::Result<()> {
        let mut cb =
            arboard::Clipboard::new().map_err(|e| CoreError::Clipboard(e.to_string()))?;
        cb.clear().map_err(|e| CoreError::Clipboard(e.to_string()))?;
        Ok(())
    }
}
