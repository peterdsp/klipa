//! Composition root.
//!
//! Wires the persistent store, the clipboard adapter, the watcher loop,
//! the global hotkey, the tray icon, and the Slint UI together.

// On a release Windows build, run as a GUI app so no console window
// flashes behind the menubar/tray UI.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod adapters;
mod app;
mod hotkey;
mod platform;
mod tray;

use adapters::{clipboard::ArboardClipboard, storage::SqliteStore};
use app::App;
use klipa_core::HistoryService;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Convert the process into a menubar-only accessory before any
    // window appears. No dock icon, no main menu — clipboard apps live
    // in the status bar, not the dock.
    platform::make_menubar_only();

    // ── Async runtime ──────────────────────────────────────────────────
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handle = runtime.handle().clone();

    // ── Domain service + persistence ───────────────────────────────────
    let history = handle.block_on(async {
        let store = Arc::new(SqliteStore::new().await.expect("storage init"));
        let clipboard = Arc::new(ArboardClipboard::new());
        // Unlimited history — usize::MAX means the cap-enforcement
        // branch in HistoryService::ingest never triggers. SQLite holds
        // everything; the in-memory mirror grows linearly with usage.
        let svc = Arc::new(HistoryService::new(store, clipboard, usize::MAX));
        svc.load().await.expect("history load");
        svc
    });

    // ── UI ─────────────────────────────────────────────────────────────
    let app = Rc::new(App::new(history.clone(), handle.clone()));
    app.install();

    // ── Clipboard watcher ──────────────────────────────────────────────
    // Flips the App's dirty flag when a new copy is ingested; the main
    // timer below picks it up and refreshes the model.
    {
        let svc = history.clone();
        let dirty = app.dirty_flag();
        handle.spawn(async move {
            adapters::watcher::run(svc, move || {
                dirty.store(true, std::sync::atomic::Ordering::Release);
            })
            .await;
        });
    }

    // Tray + hotkey must be created on the main thread (Cocoa/Win32),
    // before the Slint event loop owns it.
    let tray = tray::Tray::new();
    let hk = match hotkey::Hotkey::register_default() {
        Ok(h) => Some(h),
        Err(e) => {
            tracing::warn!(?e, "global hotkey registration failed");
            None
        }
    };

    // Drive tray + hotkey event draining from inside the Slint event loop
    // every 100ms. Keeps everything on the main thread, no extra channels.
    {
        let app_for_timer = app.clone();
        let hk_id = hk.as_ref().map(|h| h.id);
        let show_id = tray.show_id.clone();
        let quit_id = tray.quit_id.clone();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                // Tray menu events.
                for id in tray::poll_menu_events() {
                    if id == show_id {
                        app_for_timer.show_and_focus();
                    } else if id == quit_id {
                        slint::quit_event_loop().ok();
                    }
                }
                // Global hotkey events.
                for id in hotkey::poll_events() {
                    if Some(id) == hk_id {
                        app_for_timer.toggle();
                    }
                }
                // Refresh the model only if the watcher saw a change.
                app_for_timer.refresh_if_dirty();
            },
        );
        // Leak the timer so it lives for the whole program.
        Box::leak(Box::new(timer));
    }

    if let Err(e) = app.run() {
        tracing::error!(?e, "ui exited with error");
    }
}
