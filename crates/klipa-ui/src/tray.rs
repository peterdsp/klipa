//! Menubar/tray icon + the clipboard-history dropdown.
//!
//! The whole UI is this native menu: clicking the menubar icon drops
//! down the recent clipboard entries; clicking an entry copies it back
//! to the clipboard. No window, no GPU, no renderer - hence tiny.

use crate::adapters::clipboard::{decode_png, read_image_png};
use klipa_core::{HistoryItem, ItemKind};
use std::cell::RefCell;
use std::collections::HashMap;
use tray_icon::menu::{
    Icon as MenuIcon, IconMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Edge length of the little preview icon shown next to image entries.
const THUMB: usize = 28;

/// Stable ids for the two fixed actions at the bottom of the menu.
pub const CLEAR_ID: &str = "__klipa_clear";
pub const QUIT_ID: &str = "__klipa_quit";

/// How many recent entries to show in the dropdown (native menus get
/// unwieldy beyond this; older items stay in history/the file).
const MAX_MENU_ITEMS: usize = 25;
const LABEL_MAX_CHARS: usize = 48;

pub struct Tray {
    icon: TrayIcon,
    /// Cache of generated thumbnails keyed by the image reference id,
    /// so we decode each image file at most once.
    thumbs: RefCell<HashMap<String, (Vec<u8>, u32, u32)>>,
}

impl Tray {
    pub fn new() -> Self {
        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(Menu::new()))
            .with_tooltip("klipa - clipboard history")
            .with_icon(clipboard_glyph());

        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_as_template(true);

        Self {
            icon: builder.build().expect("tray icon"),
            thumbs: RefCell::new(HashMap::new()),
        }
    }

    /// Rebuild the dropdown from the current history snapshot.
    pub fn set_history(&self, items: &[HistoryItem]) {
        let menu = Menu::new();
        if items.is_empty() {
            let _ = menu.append(&MenuItem::new("No clipboard history yet", false, None));
        } else {
            for it in items.iter().take(MAX_MENU_ITEMS) {
                let id = it.id.0.to_string();
                let label = menu_label(&it.title);
                match self.image_ref(it).and_then(|r| self.thumb_icon(r)) {
                    // Image entry -> show a small preview next to the label.
                    Some(icon) => {
                        let _ = menu.append(&IconMenuItem::with_id(id, label, true, Some(icon), None));
                    }
                    None => {
                        let _ = menu.append(&MenuItem::with_id(id, label, true, None));
                    }
                }
            }
        }
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id(CLEAR_ID, "Clear history", !items.is_empty(), None));
        let _ = menu.append(&MenuItem::with_id(QUIT_ID, "Quit klipa", true, None));
        self.icon.set_menu(Some(Box::new(menu)));
    }

    /// The image reference id of an item, if it is an image entry.
    fn image_ref<'a>(&self, item: &'a HistoryItem) -> Option<&'a str> {
        item.contents
            .iter()
            .find(|c| matches!(c.kind, ItemKind::Image))
            .map(|c| c.value.as_str())
    }

    /// Build (and cache) a small menu icon from an image entry.
    fn thumb_icon(&self, reference: &str) -> Option<MenuIcon> {
        if !self.thumbs.borrow().contains_key(reference) {
            let png = read_image_png(reference)?;
            let (w, h, rgba) = decode_png(&png)?;
            let thumb = downscale(w, h, &rgba, THUMB);
            self.thumbs
                .borrow_mut()
                .insert(reference.to_string(), (thumb, THUMB as u32, THUMB as u32));
        }
        let cache = self.thumbs.borrow();
        let (rgba, w, h) = cache.get(reference)?;
        MenuIcon::from_rgba(rgba.clone(), *w, *h).ok()
    }
}

/// Nearest-neighbour fit of `rgba` into a `size`x`size` transparent
/// canvas, preserving aspect ratio. Cheap and good enough for a glyph.
fn downscale(w: usize, h: usize, rgba: &[u8], size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size * size * 4];
    if w == 0 || h == 0 {
        return out;
    }
    let scale = (size as f32 / w as f32).min(size as f32 / h as f32);
    let tw = ((w as f32 * scale).round() as usize).max(1).min(size);
    let th = ((h as f32 * scale).round() as usize).max(1).min(size);
    let ox = (size - tw) / 2;
    let oy = (size - th) / 2;
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

/// Collapse whitespace to single spaces and truncate for the menu.
fn menu_label(title: &str) -> String {
    let collapsed = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > LABEL_MAX_CHARS {
        let head: String = collapsed.chars().take(LABEL_MAX_CHARS).collect();
        format!("{head}...")
    } else if collapsed.is_empty() {
        "(empty)".to_string()
    } else {
        collapsed
    }
}

/// Drain pending menu events. Returns the activated ids.
pub fn poll_menu_events() -> Vec<MenuId> {
    let receiver = MenuEvent::receiver();
    let mut out = vec![];
    while let Ok(ev) = receiver.try_recv() {
        out.push(ev.id);
    }
    out
}

/// Draw a 22x22 monochrome clipboard glyph in pure Rust (no asset).
/// On macOS it's used as a template image so the system tints it.
fn clipboard_glyph() -> Icon {
    const W: u32 = 22;
    const H: u32 = 22;
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    let mut set = |x: u32, y: u32, on: bool| {
        if x < W && y < H {
            let i = ((y * W + x) * 4) as usize;
            rgba[i + 3] = if on { 255 } else { 0 };
        }
    };
    for y in 4..20 {
        for x in 3..19 {
            let corner = (x == 3 || x == 18) && (y == 4 || y == 19);
            set(x, y, !corner);
        }
    }
    for y in 5..19 {
        for x in 4..18 {
            set(x, y, false);
        }
    }
    for y in 2..6 {
        for x in 8..14 {
            set(x, y, true);
        }
    }
    for y in 3..5 {
        for x in 9..13 {
            set(x, y, false);
        }
    }
    for line_y in [10u32, 13, 16] {
        for x in 6..16 {
            set(x, line_y, true);
        }
    }
    Icon::from_rgba(rgba, W, H).expect("tray icon")
}
