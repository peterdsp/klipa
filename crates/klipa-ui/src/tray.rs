//! Tray icon + menu. Cross-platform via the `tray-icon` crate.
//!
//! The icon is procedurally drawn — no bundled asset file. On macOS it's
//! flagged as a template so the system auto-tints it for light/dark mode,
//! matching native menubar icons.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    _icon: TrayIcon,
    pub show_id: tray_icon::menu::MenuId,
    pub quit_id: tray_icon::menu::MenuId,
}

impl Tray {
    pub fn new() -> Self {
        let menu = Menu::new();
        let show = MenuItem::new("Show klipa", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let show_id = show.id().clone();
        let quit_id = quit.id().clone();
        menu.append(&show).ok();
        menu.append(&quit).ok();

        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("klipa — clipboard history")
            .with_icon(clipboard_glyph());

        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_as_template(true);

        let icon = builder.build().expect("tray icon");

        Self {
            _icon: icon,
            show_id,
            quit_id,
        }
    }
}

/// Draw a 22×22 monochrome clipboard glyph in pure Rust.
///
/// Shape: rectangular body outline with a small clip at the top and
/// three horizontal text lines inside. All pixels are pure black with
/// alpha — when used as a macOS template image the OS recolours it
/// (white in dark menubar, black in light menubar) automatically.
fn clipboard_glyph() -> Icon {
    const W: u32 = 22;
    const H: u32 = 22;
    let mut rgba = vec![0u8; (W * H * 4) as usize];

    let mut set = |x: u32, y: u32, on: bool| {
        if x < W && y < H {
            let i = ((y * W + x) * 4) as usize;
            rgba[i] = 0;
            rgba[i + 1] = 0;
            rgba[i + 2] = 0;
            rgba[i + 3] = if on { 255 } else { 0 };
        }
    };

    // Body outline: 16×16 rectangle inset by 3px, with corners rounded
    // by one pixel.
    for y in 4..20 {
        for x in 3..19 {
            let on_corner = (x == 3 || x == 18) && (y == 4 || y == 19);
            set(x, y, !on_corner);
        }
    }
    // Hollow the body interior so we're left with a 1px outline.
    for y in 5..19 {
        for x in 4..18 {
            set(x, y, false);
        }
    }

    // Clip at top — 6×4 rectangle straddling the body's top edge.
    for y in 2..6 {
        for x in 8..14 {
            set(x, y, true);
        }
    }
    // Hollow the clip interior so it's also outline-only.
    for y in 3..5 {
        for x in 9..13 {
            set(x, y, false);
        }
    }

    // Three "text" lines inside the body.
    for line_y in [10u32, 13, 16] {
        for x in 6..16 {
            set(x, line_y, true);
        }
    }

    Icon::from_rgba(rgba, W, H).expect("tray icon")
}

/// Drain pending menu events. Returns the IDs that were activated.
pub fn poll_menu_events() -> Vec<tray_icon::menu::MenuId> {
    let receiver = MenuEvent::receiver();
    let mut out = vec![];
    while let Ok(ev) = receiver.try_recv() {
        out.push(ev.id);
    }
    out
}
