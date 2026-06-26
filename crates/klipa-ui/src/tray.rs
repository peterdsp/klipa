//! Menubar/tray icon + the clipboard-history dropdown.
//!
//! The whole UI is this native menu: clicking the menubar icon drops
//! down the recent clipboard entries; clicking an entry copies it back
//! to the clipboard. No window, no GPU, no renderer - hence tiny.

use klipa_core::HistoryItem;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Stable ids for the two fixed actions at the bottom of the menu.
pub const CLEAR_ID: &str = "__klipa_clear";
pub const QUIT_ID: &str = "__klipa_quit";

/// How many recent entries to show in the dropdown (native menus get
/// unwieldy beyond this; older items stay in history/the file).
const MAX_MENU_ITEMS: usize = 25;
const LABEL_MAX_CHARS: usize = 48;

pub struct Tray {
    icon: TrayIcon,
}

impl Tray {
    pub fn new() -> Self {
        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(build_menu(&[])))
            .with_tooltip("klipa - clipboard history")
            .with_icon(clipboard_glyph());

        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_as_template(true);

        Self {
            icon: builder.build().expect("tray icon"),
        }
    }

    /// Rebuild the dropdown from the current history snapshot.
    pub fn set_history(&self, items: &[HistoryItem]) {
        self.icon.set_menu(Some(Box::new(build_menu(items))));
    }
}

fn build_menu(items: &[HistoryItem]) -> Menu {
    let menu = Menu::new();
    if items.is_empty() {
        let _ = menu.append(&MenuItem::new("No clipboard history yet", false, None));
    } else {
        for it in items.iter().take(MAX_MENU_ITEMS) {
            // Item id = the history UUID, so a click maps straight back.
            let item = MenuItem::with_id(it.id.0.to_string(), menu_label(&it.title), true, None);
            let _ = menu.append(&item);
        }
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(
        CLEAR_ID,
        "Clear history",
        !items.is_empty(),
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(QUIT_ID, "Quit klipa", true, None));
    menu
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
