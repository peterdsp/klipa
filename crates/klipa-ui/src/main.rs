//! Composition root.
//!
//! klipa is an ultralight, menubar-only clipboard manager: the entire
//! UI is the tray dropdown (see `tray.rs`). This wires the local JSON
//! store, the clipboard adapter + watcher, and a minimal winit event
//! loop that pumps the OS run loop and the tray menu - no window, no
//! GPU renderer, nothing logged or uploaded.

// Release Windows build runs as a GUI app (no console window).
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod adapters;
mod awake;
mod http;
mod license;
mod paths;
mod platform;
mod settings;
mod tray;
mod weather;

use adapters::{clipboard::ArboardClipboard, storage::JsonStore};
use klipa_core::{HistoryItemId, HistoryService};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

/// Keep at most this many entries (newest first). Plenty for a
/// menubar dropdown; the file stays small.
const HISTORY_CAP: usize = 200;
/// How often the loop wakes to drain menu clicks / refresh the menu.
const TICK: Duration = Duration::from_millis(200);

struct Klipa {
    history: Arc<HistoryService>,
    rt: tokio::runtime::Handle,
    /// Set by the clipboard watcher when new history lands.
    dirty: Arc<AtomicBool>,
    tray: Option<tray::Tray>,
    /// Amphetamine-style keep-awake session manager.
    awake: awake::KeepAwake,
    /// Last time the keep-awake countdown label was refreshed.
    awake_refreshed: Instant,
    /// Trial + license gate (no-op in the App Store build).
    license: license::License,
    /// Mirrors the locked state so the watcher can stop recording.
    locked: Arc<AtomicBool>,
    /// Last time the trial/license state was reflected in the menu.
    license_refreshed: Instant,
    /// User preferences (currently: menu bar display).
    settings: settings::Settings,
    /// Cache for the temperature backing the "weather" menu bar modes.
    weather: weather::WeatherState,
    /// Last time the menu bar title (date/temp) was refreshed.
    menubar_refreshed: Instant,
}

impl Klipa {
    fn rebuild_menu(&mut self) {
        if let Some(tray) = &self.tray {
            let items = self.rt.block_on(self.history.snapshot());
            let gate = self.license.gate();
            self.locked.store(gate.is_locked(), Ordering::Release);
            let notice = self.license.transient_message();
            tray.set_menu(
                &items,
                &self.awake.view(),
                &gate,
                license::License::price(),
                notice.as_deref(),
                self.settings.menubar_display,
            );
        }
    }

    /// Refresh the tray title (date / temperature). Runs the network
    /// call on the tokio blocking pool so the event loop never stalls.
    fn refresh_menubar_title(&mut self) {
        let Some(tray) = &self.tray else { return };
        let display = self.settings.menubar_display;
        let title = settings::menubar_title(display, &mut self.weather);
        tray.set_title(title.as_deref());
        self.menubar_refreshed = Instant::now();
    }

    fn set_menubar_display(&mut self, mode: settings::MenubarDisplay) {
        if self.settings.menubar_display == mode {
            return;
        }
        self.settings.menubar_display = mode;
        self.settings.save();
        self.refresh_menubar_title();
        self.rebuild_menu();
    }
}

impl ApplicationHandler for Klipa {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        if self.tray.is_none() {
            // macOS: drop the Dock icon / main menu now that the run
            // loop exists. No-op elsewhere.
            platform::make_menubar_only();
            self.tray = Some(tray::Tray::new());
            self.rebuild_menu();
            self.refresh_menubar_title();
            tracing::info!("klipa ready - menubar icon active");
        }
    }

    // No windows, so nothing to handle here.
    fn window_event(&mut self, _e: &ActiveEventLoop, _id: WindowId, _ev: WindowEvent) {}

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        for id in tray::poll_menu_events() {
            match id.as_ref() {
                tray::QUIT_ID => {
                    event_loop.exit();
                    return;
                }
                tray::CLEAR_ID => {
                    let _ = self.rt.block_on(self.history.clear_unpinned());
                    self.rebuild_menu();
                }
                tray::HIDE_ICON_ID => {
                    if let Some(tray) = &self.tray {
                        tray.set_visible(false);
                    }
                }
                tray::BUY_ID => {
                    self.license.open_purchase();
                }
                tray::ACTIVATE_ID => {
                    self.license.activate_from_clipboard();
                    self.rebuild_menu();
                }
                tray::MENUBAR_ICON_ID => self.set_menubar_display(settings::MenubarDisplay::IconOnly),
                tray::MENUBAR_DATE_ID => self.set_menubar_display(settings::MenubarDisplay::Date),
                tray::MENUBAR_TEMP_ID => self.set_menubar_display(settings::MenubarDisplay::Temperature),
                tray::MENUBAR_BOTH_ID => self.set_menubar_display(settings::MenubarDisplay::Both),
                tray::AWAKE_END_ID => {
                    self.awake.end();
                    self.rebuild_menu();
                }
                tray::AWAKE_DISPLAY_ID => {
                    let next = !self.awake.allow_display_sleep();
                    self.awake.set_allow_display_sleep(next);
                    self.rebuild_menu();
                }
                other if tray::parse_awake_start(other).is_some() => {
                    let dur = tray::parse_awake_start(other).unwrap();
                    self.awake.start(dur);
                    self.rebuild_menu();
                }
                other => {
                    if let Ok(uuid) = uuid::Uuid::parse_str(other) {
                        if let Err(e) = self
                            .rt
                            .block_on(self.history.copy_to_clipboard(HistoryItemId(uuid)))
                        {
                            tracing::warn!(?e, "copy failed");
                        }
                    }
                }
            }
        }

        // Reap a keep-awake session whose timer elapsed, and otherwise
        // keep the "X left" countdown label roughly fresh by rebuilding
        // about once a minute while a session is running.
        let awake_ended = self.awake.poll();
        let awake_stale = self.awake.view().active
            && self.awake_refreshed.elapsed() >= Duration::from_secs(60);
        if awake_ended || awake_stale {
            self.awake_refreshed = Instant::now();
            self.rebuild_menu();
        }

        if self.dirty.swap(false, Ordering::AcqRel) {
            self.rebuild_menu();
        }

        // Periodically reflect the trial clock: the countdown ticks down,
        // and the menu flips to the paywall the moment the trial lapses
        // (and clears any expired activation notice).
        if self.license_refreshed.elapsed() >= Duration::from_secs(20) {
            self.license_refreshed = Instant::now();
            self.rebuild_menu();
        }

        // Refresh the menu bar title once a minute: the date rolls over
        // at midnight, and the temperature cache re-fetches on its own
        // TTL from inside the WeatherState.
        if self.menubar_refreshed.elapsed() >= Duration::from_secs(60) {
            self.refresh_menubar_title();
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + TICK));
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handle = runtime.handle().clone();

    // Domain service backed by the local JSON file + arboard clipboard.
    let history = handle.block_on(async {
        let store = Arc::new(JsonStore::new().await.expect("storage init"));
        let clipboard = Arc::new(ArboardClipboard::new());
        let svc = Arc::new(HistoryService::new(store, clipboard, HISTORY_CAP));
        svc.load().await.expect("history load");
        svc
    });

    // Trial + license gate. Stamps the trial clock on first run, and
    // re-checks an activated key in the background if it's gone stale.
    let mut license = license::License::load();
    license.reverify_if_stale();
    let locked = Arc::new(AtomicBool::new(license.gate().is_locked()));

    // Clipboard watcher: flips `dirty` so the loop refreshes the menu.
    // It stops recording while `locked` is set (trial lapsed).
    let dirty = Arc::new(AtomicBool::new(false));
    {
        let svc = history.clone();
        let d = dirty.clone();
        let lk = locked.clone();
        handle.spawn(async move {
            adapters::watcher::run(svc, lk, move || d.store(true, Ordering::Release)).await;
        });
    }

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = Klipa {
        history,
        rt: handle,
        dirty,
        tray: None,
        awake: awake::KeepAwake::new(),
        awake_refreshed: Instant::now(),
        license,
        locked,
        license_refreshed: Instant::now(),
        settings: settings::Settings::load(),
        weather: weather::WeatherState::new(),
        menubar_refreshed: Instant::now(),
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        tracing::error!(?e, "event loop exited with error");
    }

    drop(runtime);
}
