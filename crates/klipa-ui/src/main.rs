//! Composition root.
//!
//! Wires the persistent store, the clipboard adapter, the watcher loop,
//! the global hotkey, the tray icon, and the Slint UI together.

mod adapters;
mod app;
mod hotkey;
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
        let svc = Arc::new(HistoryService::new(store, clipboard, 200));
        svc.load().await.expect("history load");
        svc
    });

    // ── Clipboard watcher ──────────────────────────────────────────────
    {
        let svc = history.clone();
        handle.spawn(async move {
            adapters::watcher::run(svc, || {
                // Nudge the Slint loop so the App's refresh path runs.
                let _ = slint::invoke_from_event_loop(|| {});
            })
            .await;
        });
    }

    // ── UI ─────────────────────────────────────────────────────────────
    let app = Rc::new(App::new(history, handle));
    app.install();

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
                // Watcher nudge → refresh model.
                app_for_timer.refresh_async();
            },
        );
        // Leak the timer so it lives for the whole program.
        Box::leak(Box::new(timer));
    }

    if let Err(e) = app.run() {
        tracing::error!(?e, "ui exited with error");
    }
}
