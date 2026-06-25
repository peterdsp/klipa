//! Global hotkey registration.
//!
//! Default chord: Cmd+Shift+V on macOS, Ctrl+Shift+V on Win/Linux.
//! Customisation hook is left as a TODO — wire to a settings file later.

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};

pub struct Hotkey {
    _mgr: GlobalHotKeyManager,
    pub id: u32,
}

impl Hotkey {
    pub fn register_default() -> Result<Self, global_hotkey::Error> {
        let mgr = GlobalHotKeyManager::new()?;
        #[cfg(target_os = "macos")]
        let mods = Modifiers::SUPER | Modifiers::SHIFT;
        #[cfg(not(target_os = "macos"))]
        let mods = Modifiers::CONTROL | Modifiers::SHIFT;

        let hk = HotKey::new(Some(mods), Code::KeyV);
        let id = hk.id();
        mgr.register(hk)?;
        Ok(Self { _mgr: mgr, id })
    }
}

/// Drain pending hotkey events. Returns the IDs that fired.
pub fn poll_events() -> Vec<u32> {
    let rx = GlobalHotKeyEvent::receiver();
    let mut out = vec![];
    while let Ok(ev) = rx.try_recv() {
        if ev.state == global_hotkey::HotKeyState::Pressed {
            out.push(ev.id);
        }
    }
    out
}
