//! Tray icon + menu. Cross-platform via the `tray-icon` crate.
//!
//! The tray is created on the main thread (Cocoa/Win32 requirement).
//! Menu events are polled inside the Slint event loop on a timer.

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

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("klipa — clipboard history")
            .with_icon(default_icon())
            .build()
            .expect("tray icon");

        Self {
            _icon: icon,
            show_id,
            quit_id,
        }
    }
}

/// Tray icon. The PNG is embedded into the binary at build time so we
/// never have to ship a sidecar asset file.
fn default_icon() -> Icon {
    const BYTES: &[u8] = include_bytes!("../../../assets/tray.png");
    let img = image::load_from_memory_with_format(BYTES, image::ImageFormat::Png)
        .expect("tray.png decode")
        .to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).expect("tray icon")
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
