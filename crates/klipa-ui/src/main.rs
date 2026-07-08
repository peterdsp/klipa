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
mod updater;
mod weather;

use adapters::{clipboard::ArboardClipboard, storage::JsonStore};
use klipa_core::{HistoryItem, HistoryItemId, HistoryService};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

/// How often the loop wakes to drain menu clicks / refresh the menu.
const TICK: Duration = Duration::from_millis(200);
/// Where the first-run walkthrough lives.
const WELCOME_URL: &str = "https://klipa.peterdsp.dev/welcome.html";

struct Klipa {
    history: Arc<HistoryService>,
    rt: tokio::runtime::Handle,
    /// Set by the clipboard watcher when new history lands.
    dirty: Arc<AtomicBool>,
    tray: Option<tray::Tray>,
    /// Keep-awake session manager.
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
    /// Last menu bar title we actually drew (outer `None` = never drawn,
    /// inner `None` = icon-only). Lets the per-tick refresh skip the
    /// `set_title` call unless the date/temperature text really changed.
    menubar_title_shown: Option<Option<String>>,
    /// Once-a-day background check for a newer release (no-op in the
    /// App Store build - MAS handles updates itself).
    updater: updater::UpdateState,
    /// Hash of the last menu we actually drew. Rebuilds that would
    /// produce an identical menu are skipped, so background timers never
    /// tear down (and thus close) the dropdown while the user is
    /// browsing it - the menu only redraws when its contents change.
    menu_sig: Option<u64>,
}

impl Klipa {
    /// Redraw the tray menu, but only if its contents actually changed
    /// since the last draw. Reassigning the menu on macOS dismisses it
    /// if it is open, so skipping no-op redraws is what keeps the
    /// dropdown from closing under the user while background timers fire.
    fn rebuild_menu(&mut self) {
        let Some(tray) = &self.tray else { return };
        let items = self.rt.block_on(self.history.snapshot());
        let gate = self.license.gate();
        // The watcher depends on this even when the menu itself is
        // unchanged, so always keep it current.
        self.locked.store(gate.is_locked(), Ordering::Release);
        let awake = self.awake.view();
        let notice = self.license.transient_message();
        let update = self.updater.menu_label();

        // Signature of everything the menu renders. If it matches the
        // last drawn menu, there is nothing to redraw.
        let sig = menu_signature(
            &items,
            &awake,
            &gate,
            notice.as_deref(),
            self.settings.menubar_display,
            update.as_deref(),
            self.settings.dropdown_items,
        );
        if self.menu_sig == Some(sig) {
            return;
        }
        self.menu_sig = Some(sig);

        tray.set_menu(
            &items,
            &awake,
            &gate,
            license::License::price(),
            notice.as_deref(),
            self.settings.menubar_display,
            update.as_deref(),
            self.settings.dropdown_items,
        );
    }

    /// Refresh the tray title (date / temperature) and swap the glyph
    /// so the icon+text combo doesn't waste menu bar space. When any
    /// text mode is on, the clipboard icon is hidden and only the
    /// text shows.
    fn refresh_menubar_title(&mut self) {
        let Some(tray) = &self.tray else { return };
        let display = self.settings.menubar_display;
        // Non-blocking: reads the cached temperature and lets the weather
        // module refresh it on a worker thread. Cheap to call every tick,
        // so the title lands the moment a background fetch completes.
        let title = settings::menubar_title(display, &self.weather);
        if self.menubar_title_shown.as_ref() == Some(&title) {
            return;
        }
        tray.set_title(title.as_deref());
        tray.set_icon_visible(title.is_none());
        self.menubar_title_shown = Some(title);
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

    fn set_dropdown_items(&mut self, count: usize) {
        if self.settings.dropdown_items == count {
            return;
        }
        self.settings.dropdown_items = count;
        self.settings.save();
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
                other if tray::parse_show_count(other).is_some() => {
                    self.set_dropdown_items(tray::parse_show_count(other).unwrap());
                }
                tray::UPDATE_ID => self.updater.trigger(),
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

        // Refresh the menu bar title every tick. It's a no-op unless the
        // text changed, so this cheaply picks up the date rolling over at
        // midnight and any temperature the background weather fetch just
        // landed, without ever blocking the loop on the network.
        self.refresh_menubar_title();

        // Rebuild the menu the moment the background updater posts a
        // "new version available" result. Cheap: the check itself only
        // fires once every 24 h from inside `tick`.
        if self.updater.tick() {
            self.rebuild_menu();
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + TICK));
    }
}

/// Hash of everything the tray menu renders, so identical rebuilds can
/// be skipped (see `Klipa::rebuild_menu`). Cheap: one pass over the
/// entries the dropdown actually shows.
#[allow(clippy::too_many_arguments)]
fn menu_signature(
    items: &[HistoryItem],
    awake: &awake::AwakeView,
    gate: &license::Gate,
    notice: Option<&str>,
    display: settings::MenubarDisplay,
    update: Option<&str>,
    dropdown_items: usize,
) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    items.len().hash(&mut h);
    for it in items.iter().take(dropdown_items) {
        it.id.0.hash(&mut h);
        it.title.hash(&mut h);
    }
    awake.active.hash(&mut h);
    awake.allow_display_sleep.hash(&mut h);
    awake.status.hash(&mut h);
    match gate {
        license::Gate::Full => 0u8.hash(&mut h),
        license::Gate::Trial { days_left } => {
            1u8.hash(&mut h);
            days_left.hash(&mut h);
        }
        license::Gate::Locked => 2u8.hash(&mut h),
    }
    notice.hash(&mut h);
    (display as u8).hash(&mut h);
    update.hash(&mut h);
    dropdown_items.hash(&mut h);
    h.finish()
}

/// Open a URL with the OS default handler.
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
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

    // Load persisted settings first so we can seed HistoryService with
    // the user's chosen cap.
    let mut settings = settings::Settings::load();

    // Domain service backed by the local JSON file + arboard clipboard.
    let history = {
        let cap = settings.history_cap.max(1);
        handle.block_on(async {
            let store = Arc::new(JsonStore::new().await.expect("storage init"));
            let clipboard = Arc::new(ArboardClipboard::new());
            let svc = Arc::new(HistoryService::new(store, clipboard, cap));
            svc.load().await.expect("history load");
            svc
        })
    };

    // First-launch welcome: open the walkthrough in the user's default
    // browser and mark it done, so subsequent launches are silent. The
    // page explains the menu bar UI, the display modes, and the trial.
    if !settings.welcomed {
        open_url(WELCOME_URL);
        settings.welcomed = true;
        settings.save();
    }

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
        settings,
        weather: weather::WeatherState::new(),
        menubar_title_shown: None,
        updater: updater::UpdateState::new(),
        menu_sig: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        tracing::error!(?e, "event loop exited with error");
    }

    drop(runtime);
}
