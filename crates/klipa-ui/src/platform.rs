//! Per-OS startup tweaks that keep klipa a true menubar app.

/// Make the running process a menubar accessory on macOS - no dock
/// icon, no main menu, the app exists only in the status bar.
///
/// On other OSes this is a no-op. Linux/Windows users will still see a
/// taskbar entry while the window is visible; hiding the window removes
/// it on Windows (because `tray-icon` reparents the window) and on
/// most Linux WMs.
#[cfg(target_os = "macos")]
pub fn make_menubar_only() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new()
        .expect("make_menubar_only must be called from the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

#[cfg(not(target_os = "macos"))]
pub fn make_menubar_only() {}
