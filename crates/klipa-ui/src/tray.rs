//! Menubar/tray icon + the clipboard-history dropdown.
//!
//! The whole UI is this native menu: clicking the menubar icon drops
//! down the recent clipboard entries; clicking an entry copies it back
//! to the clipboard. No window, no GPU, no renderer - hence tiny.

use crate::adapters::clipboard::{decode_png, read_image_png};
use crate::awake::AwakeView;
use crate::license::Gate;
use crate::settings::MenubarDisplay;
use klipa_core::{HistoryItem, ItemKind};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;
use tray_icon::menu::{
    CheckMenuItem, Icon as MenuIcon, IconMenuItem, Menu, MenuEvent, MenuId, MenuItem,
    PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Edge length of the preview thumbnail shown next to image entries.
/// Big enough to recognise the image at a glance, no hover needed.
const THUMB: usize = 64;

/// Stable ids for the fixed actions in the menu.
pub const CLEAR_ID: &str = "__klipa_clear";
pub const QUIT_ID: &str = "__klipa_quit";
/// Keep-awake actions.
pub const AWAKE_END_ID: &str = "__klipa_awake_end";
pub const AWAKE_DISPLAY_ID: &str = "__klipa_awake_display";
/// Prefix for "start a session of N seconds" items; 0 = indefinitely.
pub const AWAKE_START_PREFIX: &str = "__klipa_awake_start:";
/// Open the purchase page / activate a license key.
pub const BUY_ID: &str = "__klipa_buy";
pub const ACTIVATE_ID: &str = "__klipa_activate";
/// Menu bar display presets.
pub const MENUBAR_ICON_ID: &str = "__klipa_menubar_icon";
pub const MENUBAR_DATE_ID: &str = "__klipa_menubar_date";
pub const MENUBAR_TEMP_ID: &str = "__klipa_menubar_temp";
pub const MENUBAR_BOTH_ID: &str = "__klipa_menubar_both";
/// Prefix for "show N entries in the dropdown" items.
pub const SHOW_COUNT_PREFIX: &str = "__klipa_show_count:";
/// Preset dropdown sizes offered in the "Show in dropdown" submenu.
const SHOW_COUNT_PRESETS: &[usize] = &[10, 25, 50, 100];

/// Parse the count payload of a `SHOW_COUNT_PREFIX` menu id.
pub fn parse_show_count(id: &str) -> Option<usize> {
    id.strip_prefix(SHOW_COUNT_PREFIX)?.parse().ok()
}
/// "A new version is available, click to download + open it".
pub const UPDATE_ID: &str = "__klipa_update";

/// Keep-awake session presets shown in the submenu: (label, duration).
/// `None` is an indefinite session.
const AWAKE_PRESETS: &[(&str, Option<Duration>)] = &[
    ("Indefinitely", None),
    ("5 minutes", Some(Duration::from_secs(5 * 60))),
    ("15 minutes", Some(Duration::from_secs(15 * 60))),
    ("30 minutes", Some(Duration::from_secs(30 * 60))),
    ("1 hour", Some(Duration::from_secs(60 * 60))),
    ("2 hours", Some(Duration::from_secs(2 * 60 * 60))),
    ("5 hours", Some(Duration::from_secs(5 * 60 * 60))),
];

/// Parse the seconds payload of an `AWAKE_START_PREFIX` menu id into a
/// session duration (`None` == indefinitely).
pub fn parse_awake_start(id: &str) -> Option<Option<Duration>> {
    let secs: u64 = id.strip_prefix(AWAKE_START_PREFIX)?.parse().ok()?;
    Some((secs > 0).then(|| Duration::from_secs(secs)))
}

/// Hard ceiling on dropdown entries regardless of the user's setting -
/// native menus get unwieldy and slow to build beyond this. The user's
/// configured "Show in dropdown" count is clamped to this.
const MAX_MENU_ITEMS_HARD: usize = 100;
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

    /// Swap the tray icon between the clipboard glyph and a transparent
    /// stand-in that occupies as little menu bar space as possible.
    /// Used when the user picks a date/temperature display: only the
    /// text should show, not "icon + text" side by side.
    pub fn set_icon_visible(&self, show_glyph: bool) {
        let icon = if show_glyph { clipboard_glyph() } else { blank_icon() };
        let _ = self.icon.set_icon(Some(icon));
        // `set_icon` re-sets the NSImage with template=false (hardcoded in the
        // tray-icon crate), which drops the template flag from `new()` and makes
        // the glyph render as fixed black — invisible on dark menu bars. Re-apply
        // template so macOS keeps tinting it (black on light, white on dark).
        #[cfg(target_os = "macos")]
        self.icon.set_icon_as_template(true);
    }

    /// Text shown next to the menubar icon (date, temperature, ...).
    /// Pass `None` for icon-only. macOS + Linux (ayatana-appindicator)
    /// render this next to the tray icon; on Windows the tray-icon
    /// crate treats it as a tooltip.
    pub fn set_title(&self, text: Option<&str>) {
        self.icon.set_title(text);
    }

    /// Rebuild the dropdown from the current history snapshot, the
    /// keep-awake session state, and the license gate. `notice` is an
    /// optional transient status line (e.g. activation feedback).
    pub fn set_menu(
        &self,
        items: &[HistoryItem],
        awake: &AwakeView,
        gate: &Gate,
        price: &str,
        notice: Option<&str>,
        menubar: MenubarDisplay,
        update: Option<&str>,
        dropdown_items: usize,
    ) {
        // Trial elapsed and unlicensed: show only the paywall.
        if gate.is_locked() {
            self.icon.set_menu(Some(Box::new(paywall_menu(price, notice))));
            return;
        }

        let shown = dropdown_items.clamp(1, MAX_MENU_ITEMS_HARD);
        let menu = Menu::new();
        if items.is_empty() {
            let _ = menu.append(&MenuItem::new("No clipboard history yet", false, None));
        } else {
            for it in items.iter().take(shown) {
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

        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&build_awake_submenu(awake));
        let _ = menu.append(&build_menubar_submenu(menubar));
        let _ = menu.append(&build_show_count_submenu(shown));

        // During the trial, surface the days left + unlock/activate.
        if let Gate::Trial { days_left } = gate {
            let _ = menu.append(&PredefinedMenuItem::separator());
            let plural = if *days_left == 1 { "day" } else { "days" };
            let _ = menu.append(&MenuItem::new(
                format!("Free trial: {days_left} {plural} left"),
                false,
                None,
            ));
            let _ = menu.append(&MenuItem::with_id(
                BUY_ID,
                format!("Unlock full version - {price}"),
                true,
                None,
            ));
            let _ = menu.append(&MenuItem::with_id(
                ACTIVATE_ID,
                "Activate (paste license key)",
                true,
                None,
            ));
            if let Some(msg) = notice {
                let _ = menu.append(&MenuItem::new(msg, false, None));
            }
        }

        let _ = menu.append(&PredefinedMenuItem::separator());
        if let Some(label) = update {
            // Show a triangle glyph so the update item is easy to spot
            // even when the menu is long.
            let _ = menu.append(&MenuItem::with_id(
                UPDATE_ID,
                format!("\u{25b2}  {label}"),
                true,
                None,
            ));
        }
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

/// The locked-state menu: trial over, history hidden, unlock + activate.
fn paywall_menu(price: &str, notice: Option<&str>) -> Menu {
    let menu = Menu::new();
    let _ = menu.append(&MenuItem::new("klipa - free trial ended", false, None));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(
        BUY_ID,
        format!("Unlock full version - {price}"),
        true,
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(
        ACTIVATE_ID,
        "Activate (paste license key)",
        true,
        None,
    ));
    if let Some(msg) = notice {
        let _ = menu.append(&MenuItem::new(msg, false, None));
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(QUIT_ID, "Quit klipa", true, None));
    menu
}

/// Build the "Keep awake" submenu: a status line when active, the
/// duration presets, the display-sleep toggle, and an end action.
fn build_awake_submenu(awake: &AwakeView) -> Submenu {
    let title = if awake.active { "Keep awake \u{25cf}" } else { "Keep awake" };
    let sub = Submenu::new(title, true);

    if let Some(status) = &awake.status {
        let _ = sub.append(&MenuItem::new(status, false, None));
        let _ = sub.append(&PredefinedMenuItem::separator());
    }

    for (label, dur) in AWAKE_PRESETS {
        let secs = dur.map(|d| d.as_secs()).unwrap_or(0);
        let id = format!("{AWAKE_START_PREFIX}{secs}");
        let _ = sub.append(&MenuItem::with_id(id, *label, true, None));
    }

    let _ = sub.append(&PredefinedMenuItem::separator());
    let _ = sub.append(&CheckMenuItem::with_id(
        AWAKE_DISPLAY_ID,
        "Allow display sleep",
        true,
        awake.allow_display_sleep,
        None,
    ));
    let _ = sub.append(&MenuItem::with_id(
        AWAKE_END_ID,
        "End current session",
        awake.active,
        None,
    ));
    sub
}

/// Build the "Menu bar" submenu with the four display presets. The
/// currently-selected preset is shown with a checkmark. Temperature
/// options are visible on every platform; they only make an HTTP call
/// once the user actually picks them, so the icon-only default (which
/// is unchanged) never touches the network.
fn build_menubar_submenu(current: MenubarDisplay) -> Submenu {
    let sub = Submenu::new("Menu bar", true);
    let add = |id: &str, label: &str, mode: MenubarDisplay| {
        let _ = sub.append(&CheckMenuItem::with_id(
            id,
            label,
            true,
            current == mode,
            None,
        ));
    };
    add(MENUBAR_ICON_ID, "Icon only", MenubarDisplay::IconOnly);
    add(MENUBAR_DATE_ID, "Date", MenubarDisplay::Date);
    add(MENUBAR_TEMP_ID, "Temperature", MenubarDisplay::Temperature);
    add(MENUBAR_BOTH_ID, "Date + Temperature", MenubarDisplay::Both);
    sub
}

/// Build the "Show in dropdown" submenu: how many clipboard entries the
/// dropdown lists when clicked. The current value is checkmarked.
fn build_show_count_submenu(current: usize) -> Submenu {
    let sub = Submenu::new("Show in dropdown", true);
    for &n in SHOW_COUNT_PRESETS {
        let id = format!("{SHOW_COUNT_PREFIX}{n}");
        let _ = sub.append(&CheckMenuItem::with_id(
            id,
            format!("{n} entries"),
            true,
            current == n,
            None,
        ));
    }
    sub
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

/// A 1x1 fully transparent icon. Used when the menu bar mode wants
/// only text (date, temperature, or both) and the clipboard glyph
/// would otherwise waste space next to it. The OS still reserves a
/// tiny slot for the icon, but nothing visible ends up there.
fn blank_icon() -> Icon {
    Icon::from_rgba(vec![0, 0, 0, 0], 1, 1).expect("blank tray icon")
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
