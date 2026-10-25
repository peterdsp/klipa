//! ClipboardSource adapter using `arboard`. Captures both text and
//! images (e.g. screenshots). Images are stored PNG-compressed so the
//! local history file stays small.

use crate::paths;
use async_trait::async_trait;
use base64::Engine as _;
use klipa_core::{ClipboardSource, CoreError, HistoryItem, ItemContent, ItemKind, PasteboardEvent};
use std::borrow::Cow;
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::sync::Mutex;
use tokio::sync::Notify;

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
            let fingerprint = hash(&img.bytes);
            let sig = format!("i:{}x{}:{:016x}", img.width, img.height, fingerprint);
            if self.is_new(&sig) {
                // Reference id is derived from the image content, so copying
                // the same image again yields the same id. That lets history
                // dedupe it (bubbling the existing entry to the top) instead
                // of stacking duplicates, and reuses the on-disk PNG.
                let id = image_ref_id(img.width, img.height, fingerprint);
                if let Some(path) = paths::image_path(&id) {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    // Only encode + write when we haven't stored this image
                    // before; a re-copy of a known image skips the disk work.
                    let ready = path.exists()
                        || encode_png(img.width, img.height, &img.bytes)
                            .map(|png| std::fs::write(&path, &png).is_ok())
                            .unwrap_or(false);
                    if ready {
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

/// Content-derived reference id for an image entry. Identical images
/// (same dimensions + content fingerprint) always map to the same id,
/// which is what lets history dedupe re-copied images instead of
/// stacking duplicates.
fn image_ref_id(width: usize, height: usize, fingerprint: u64) -> String {
    format!("img-{width}x{height}-{fingerprint:016x}")
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

/// Build a `size`x`size` RGBA thumbnail for an image entry without ever
/// holding the full picture in memory.
///
/// A retina screenshot decodes to tens of megabytes of RGBA (3644x2372
/// is 34 MB), and decoding one per visible menu entry was what pushed
/// this process into the hundreds of megabytes. Here the PNG is read a
/// row at a time straight off disk and sampled into the thumbnail as it
/// goes, so peak cost is one source row (tens of kilobytes) plus the
/// small output canvas, whatever the image's size.
pub fn decode_png_thumb(reference: &str, size: usize) -> Option<Vec<u8>> {
    if let Some(path) = paths::image_path(reference) {
        if path.exists() {
            if let Some(thumb) = File::open(&path)
                .ok()
                .and_then(|f| thumb_streamed(BufReader::new(f), size))
            {
                return Some(thumb);
            }
            // Interlaced or otherwise unusual PNG: rows do not arrive in
            // top-to-bottom order, so fall back to a full decode.
            let bytes = std::fs::read(&path).ok()?;
            let (w, h, rgba) = decode_png(&bytes)?;
            return Some(downscale(w, h, &rgba, size));
        }
    }
    // Legacy entries inlined base64 PNG directly in the value.
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(reference.as_bytes())
        .ok()?;
    if let Some(thumb) = thumb_streamed(Cursor::new(&bytes[..]), size) {
        return Some(thumb);
    }
    let (w, h, rgba) = decode_png(&bytes)?;
    Some(downscale(w, h, &rgba, size))
}

/// Row-by-row nearest-neighbour thumbnail. Returns None when the PNG
/// cannot be sampled in a single forward pass (interlaced, or a colour
/// type the expansion did not normalise), leaving the caller to decide.
fn thumb_streamed<R: Read>(source: R, size: usize) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(source);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;

    let (w, h, interlaced) = {
        let info = reader.info();
        (info.width as usize, info.height as usize, info.interlaced)
    };
    if interlaced {
        return None;
    }
    let bpp = match reader.output_color_type().0 {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return None,
    };

    let mut out = vec![0u8; size * size * 4];
    if w == 0 || h == 0 {
        return Some(out);
    }
    let (tw, th, ox, oy) = thumb_fit(w, h, size);

    // Target row ty samples source row ty * h / th, the same mapping the
    // whole-image path uses, so thumbnails look identical either way.
    let mut ty = 0usize;
    let mut y = 0usize;
    while ty < th {
        let row = match reader.next_row() {
            Ok(Some(row)) => row,
            _ => break,
        };
        let data = row.data();
        while ty < th && ty * h / th == y {
            for tx in 0..tw {
                let si = (tx * w / tw) * bpp;
                let di = ((oy + ty) * size + ox + tx) * 4;
                if si + bpp <= data.len() && di + 4 <= out.len() {
                    let px = &data[si..si + bpp];
                    let (r, g, b, a) = match bpp {
                        4 => (px[0], px[1], px[2], px[3]),
                        3 => (px[0], px[1], px[2], 255),
                        2 => (px[0], px[0], px[0], px[1]),
                        _ => (px[0], px[0], px[0], 255),
                    };
                    out[di] = r;
                    out[di + 1] = g;
                    out[di + 2] = b;
                    out[di + 3] = a;
                }
            }
            ty += 1;
        }
        y += 1;
    }
    Some(out)
}

/// Target size and centring offsets for fitting `w`x`h` into `size`.
fn thumb_fit(w: usize, h: usize, size: usize) -> (usize, usize, usize, usize) {
    let scale = (size as f32 / w as f32).min(size as f32 / h as f32);
    let tw = ((w as f32 * scale).round() as usize).max(1).min(size);
    let th = ((h as f32 * scale).round() as usize).max(1).min(size);
    ((tw), (th), (size - tw) / 2, (size - th) / 2)
}

/// Nearest-neighbour fit of a whole decoded image into a `size`x`size`
/// transparent canvas. Only the interlaced fallback needs this.
fn downscale(w: usize, h: usize, rgba: &[u8], size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size * size * 4];
    if w == 0 || h == 0 {
        return out;
    }
    let (tw, th, ox, oy) = thumb_fit(w, h, size);
    for ty in 0..th {
        let sy = ty * h / th;
        for tx in 0..tw {
            let sx = tx * w / tw;
            let si = (sy * w + sx) * 4;
            let di = ((oy + ty) * size + ox + tx) * 4;
            if si + 4 <= rgba.len() && di + 4 <= out.len() {
                out[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
            }
        }
    }
    out
}

/// Hand pages freed by a large transient decode back to the OS.
///
/// Pasting an image still has to materialise it in full for the system
/// clipboard. Without this the allocator keeps those pages, and the
/// process footprint stays at its worst-case size for the rest of the
/// session.
#[cfg(target_os = "macos")]
pub fn release_reclaimed_memory() {
    extern "C" {
        /// A null zone scavenges every zone.
        fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal_size: usize) -> usize;
    }
    unsafe {
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn release_reclaimed_memory() {}

#[async_trait]
impl ClipboardSource for ArboardClipboard {
    async fn next(&self) -> klipa_core::Result<Option<PasteboardEvent>> {
        self.notify.notified().await;
        Ok(None)
    }

    async fn write(&self, item: &HistoryItem) -> klipa_core::Result<()> {
        let mut cb = arboard::Clipboard::new().map_err(|e| CoreError::Clipboard(e.to_string()))?;

        // Image item -> load the stored PNG and set it on the clipboard.
        if let Some(c) = item
            .contents
            .iter()
            .find(|c| matches!(c.kind, ItemKind::Image))
        {
            let png = read_image_png(&c.value)
                .ok_or_else(|| CoreError::Clipboard("image not found".into()))?;
            let (width, height, rgba) =
                decode_png(&png).ok_or_else(|| CoreError::Clipboard("bad image data".into()))?;
            let result = cb
                .set_image(arboard::ImageData {
                    width,
                    height,
                    bytes: Cow::Owned(rgba),
                })
                .map_err(|e| CoreError::Clipboard(e.to_string()));
            drop(png);
            release_reclaimed_memory();
            result?;
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
        let mut cb = arboard::Clipboard::new().map_err(|e| CoreError::Clipboard(e.to_string()))?;
        cb.clear()
            .map_err(|e| CoreError::Clipboard(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_share_a_ref_id() {
        // Same image copied twice -> same id, so history dedupes it.
        let bytes = [1u8, 2, 3, 4, 5];
        let a = image_ref_id(100, 200, hash(&bytes));
        let b = image_ref_id(100, 200, hash(&bytes));
        assert_eq!(a, b);
    }

    #[test]
    fn streamed_thumbnail_matches_a_whole_image_decode() {
        // A gradient big enough to exercise the row mapping.
        let (w, h) = (97usize, 61usize);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                rgba[i] = (x % 256) as u8;
                rgba[i + 1] = (y % 256) as u8;
                rgba[i + 2] = ((x + y) % 256) as u8;
                rgba[i + 3] = 255;
            }
        }
        let png = encode_png(w, h, &rgba).expect("encode");

        let streamed = thumb_streamed(Cursor::new(&png[..]), 64).expect("streamed");
        let (dw, dh, decoded) = decode_png(&png).expect("decode");
        let whole = downscale(dw, dh, &decoded, 64);

        assert_eq!(streamed, whole);
    }

    #[test]
    fn different_images_get_different_ref_ids() {
        // Different content (or different size) must not collide.
        let one = image_ref_id(100, 200, hash(&[1u8, 2, 3]));
        let two = image_ref_id(100, 200, hash(&[9u8, 9, 9]));
        let three = image_ref_id(101, 200, hash(&[1u8, 2, 3]));
        assert_ne!(one, two);
        assert_ne!(one, three);
    }
}
