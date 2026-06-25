//! App layer — binds `HistoryService` (domain) to the Slint UI.
//!
//! Responsibilities:
//! - load history on startup,
//! - refresh the Slint model whenever the service mutates,
//! - translate UI callbacks (clicks, key events) into use-case calls.
//!
//! Callback wiring is split into one `wire_*` fn per callback so each
//! handler stays inspectable on its own.

use klipa_core::{HistoryItem, HistoryItemId, HistoryService, SearchMode};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use tokio::runtime::Handle as TokioHandle;

slint::include_modules!();

/// Shared mutable list of ids in the same order as the visible rows.
/// Used to translate a row index from the UI back into a domain id.
type Ids = Rc<RefCell<Vec<HistoryItemId>>>;

pub struct App {
    window: KlipaWindow,
    history: Arc<HistoryService>,
    model: Rc<VecModel<HistoryRow>>,
    footer_model: Rc<VecModel<FooterAction>>,
    tokio: TokioHandle,
    ids: Ids,
    /// Shadow of the window's visible state — Slint's `Window` API
    /// doesn't expose `is_visible()` portably, so we mirror it here.
    /// Wrapped in `Rc` so internal closures (Esc key, etc.) can mutate.
    visible: Rc<Cell<bool>>,
}

impl App {
    pub fn new(history: Arc<HistoryService>, tokio: TokioHandle) -> Self {
        let window = KlipaWindow::new().expect("slint window");
        let model = Rc::new(VecModel::<HistoryRow>::from(vec![]));
        let footer_model = Rc::new(VecModel::<FooterAction>::from(default_footer_actions()));
        window.set_items(ModelRc::from(model.clone()));
        window.set_footer_actions(ModelRc::from(footer_model.clone()));
        Self {
            window,
            history,
            model,
            footer_model,
            tokio,
            ids: Rc::new(RefCell::new(vec![])),
            visible: Rc::new(Cell::new(false)),
        }
    }

    pub fn window(&self) -> &KlipaWindow {
        &self.window
    }

    /// Install all callbacks. Call once, before [`Self::run`].
    pub fn install(&self) {
        self.wire_query_changed();
        self.wire_row_clicked();
        self.wire_row_activated();
        self.wire_row_delete();
        self.wire_footer_clicked();
        self.wire_key_pressed();
        self.refresh_blocking();
    }

    /// Run the Slint event loop *without* showing the window. Menubar
    /// apps stay hidden until the tray icon or global hotkey summons
    /// them; `KlipaWindow::run()` would force an initial show, so we
    /// call the lower-level `run_event_loop()` instead.
    pub fn run(&self) -> Result<(), slint::PlatformError> {
        slint::run_event_loop()
    }

    // ── Public helpers used by main.rs (hotkey / tray)──────────────────

    pub fn show_and_focus(&self) {
        if !self.visible.get() {
            self.window.window().show().ok();
            self.visible.set(true);
        }
    }

    pub fn hide(&self) {
        if self.visible.get() {
            self.window.window().hide().ok();
            self.visible.set(false);
        }
    }

    pub fn toggle(&self) {
        if self.visible.get() {
            self.hide();
        } else {
            self.show_and_focus();
        }
    }

    pub fn refresh_async(&self) {
        let h = self.history.clone();
        let weak = self.window.as_weak();
        let model = self.model.clone();
        let ids = self.ids.clone();
        self.tokio.spawn(async move {
            let items = h.snapshot().await;
            let _ = slint::invoke_from_event_loop(move || {
                apply_items(&weak, &model, &ids, items, 0);
            });
        });
    }

    // ── Callback wiring (one fn per callback) ──────────────────────────

    fn wire_query_changed(&self) {
        let h = self.history.clone();
        let t = self.tokio.clone();
        let weak = self.window.as_weak();
        let model = self.model.clone();
        let ids = self.ids.clone();
        self.window.on_query_changed(move |q: SharedString| {
            let h = h.clone();
            let weak = weak.clone();
            let model = model.clone();
            let ids = ids.clone();
            t.spawn(async move {
                let items = if q.is_empty() {
                    h.snapshot().await
                } else {
                    h.query(&q, SearchMode::Mixed).await
                };
                let _ = slint::invoke_from_event_loop(move || {
                    apply_items(&weak, &model, &ids, items, 0);
                });
            });
        });
    }

    fn wire_row_clicked(&self) {
        let weak = self.window.as_weak();
        self.window.on_row_clicked(move |idx: i32| {
            if let Some(win) = weak.upgrade() {
                win.set_selected_index(idx);
            }
        });
    }

    fn wire_row_activated(&self) {
        let h = self.history.clone();
        let t = self.tokio.clone();
        let ids = self.ids.clone();
        let visible = self.visible.clone();
        let weak = self.window.as_weak();
        self.window.on_row_activated(move |idx: i32| {
            let Some(id) = ids.borrow().get(idx as usize).copied() else {
                return;
            };
            let h = h.clone();
            // Auto-hide the window after activating an item, matching
            // the macOS clipb UX.
            let weak = weak.clone();
            let visible = visible.clone();
            t.spawn(async move {
                if let Err(e) = h.copy_to_clipboard(id).await {
                    tracing::warn!(?e, "copy failed");
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        win.window().hide().ok();
                        visible.set(false);
                    }
                });
            });
        });
    }

    fn wire_row_delete(&self) {
        let h = self.history.clone();
        let t = self.tokio.clone();
        let weak = self.window.as_weak();
        let model = self.model.clone();
        let ids = self.ids.clone();
        self.window.on_row_delete_requested(move |idx: i32| {
            let Some(id) = ids.borrow().get(idx as usize).copied() else {
                return;
            };
            let h = h.clone();
            let weak = weak.clone();
            let model = model.clone();
            let ids = ids.clone();
            t.spawn(async move {
                let _ = h.delete(id).await;
                let items = h.snapshot().await;
                let _ = slint::invoke_from_event_loop(move || {
                    apply_items(&weak, &model, &ids, items, 0);
                });
            });
        });
    }

    /// Dispatch on the action-id from the footer button that was clicked.
    /// Adding a new footer action = add to [`default_footer_actions`] +
    /// add an arm here.
    fn wire_footer_clicked(&self) {
        let h = self.history.clone();
        let t = self.tokio.clone();
        let weak = self.window.as_weak();
        let model = self.model.clone();
        let ids = self.ids.clone();
        self.window.on_footer_clicked(move |action_id: SharedString| {
            match action_id.as_str() {
                "clear" => {
                    let h = h.clone();
                    let weak = weak.clone();
                    let model = model.clone();
                    let ids = ids.clone();
                    t.spawn(async move {
                        let _ = h.clear_unpinned().await;
                        let items = h.snapshot().await;
                        let _ = slint::invoke_from_event_loop(move || {
                            apply_items(&weak, &model, &ids, items, 0);
                        });
                    });
                }
                "prefs" => {
                    // TODO: settings window. For now just log.
                    tracing::info!("preferences not implemented yet");
                }
                "about" => {
                    tracing::info!("klipa — cross-platform clipboard manager");
                }
                "quit" => {
                    slint::quit_event_loop().ok();
                }
                other => tracing::warn!(?other, "unknown footer action"),
            }
        });
    }

    /// Key handling:
    /// - ↑ / ↓                 — move selection
    /// - Enter                  — activate (copy) selected row
    /// - Esc                    — clear query, else hide window
    /// - Cmd/Ctrl + 1…9         — activate that row directly
    /// - Cmd/Ctrl + K           — clear search
    /// - Cmd/Ctrl + Backspace   — delete selection
    /// - Cmd/Ctrl + Q           — quit
    /// - Cmd/Ctrl + ,           — open preferences (placeholder)
    fn wire_key_pressed(&self) {
        let weak = self.window.as_weak();
        let visible = self.visible.clone();
        self.window.on_key_pressed(move |event| {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let total = win.get_items().row_count() as i32;
            let sel = win.get_selected_index();
            let mods = &event.modifiers;
            let primary = mods.control || mods.meta;

            // Modifier chords first so plain text fall-throughs don't fire.
            if primary {
                match event.text.as_str() {
                    "k" | "K" => {
                        win.set_query(SharedString::from(""));
                        return;
                    }
                    "q" | "Q" => {
                        slint::quit_event_loop().ok();
                        return;
                    }
                    "," => {
                        win.invoke_footer_clicked(SharedString::from("prefs"));
                        return;
                    }
                    "\u{8}" => {
                        if sel >= 0 {
                            win.invoke_row_delete_requested(sel);
                        }
                        return;
                    }
                    digit if digit.len() == 1 => {
                        if let Some(d) = digit.chars().next().and_then(|c| c.to_digit(10)) {
                            if (1..=9).contains(&d) {
                                let idx = (d as i32) - 1;
                                if idx < total {
                                    win.invoke_row_activated(idx);
                                }
                                return;
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Bare keys.
            match event.text.as_str() {
                t if t == slint::platform::Key::DownArrow.as_str() => {
                    if total > 0 {
                        win.set_selected_index((sel + 1).min(total - 1).max(0));
                    }
                }
                t if t == slint::platform::Key::UpArrow.as_str() => {
                    if total > 0 {
                        win.set_selected_index((sel - 1).max(0));
                    }
                }
                t if t == slint::platform::Key::Return.as_str() => {
                    if sel >= 0 {
                        win.invoke_row_activated(sel);
                    }
                }
                t if t == slint::platform::Key::Escape.as_str() => {
                    if win.get_query().is_empty() {
                        win.window().hide().ok();
                        visible.set(false);
                    } else {
                        win.set_query(SharedString::from(""));
                    }
                }
                _ => {}
            }
        });
    }

    fn refresh_blocking(&self) {
        let items = self.tokio.block_on(self.history.snapshot());
        apply_items(&self.window.as_weak(), &self.model, &self.ids, items, 0);
    }
}

/// Static footer-action list, rendered below the history list.
/// Mirrors the macOS clipb menu (Clear / Preferences / About / Quit).
fn default_footer_actions() -> Vec<FooterAction> {
    let cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
    vec![
        FooterAction {
            label: SharedString::from("Clear"),
            shortcut_label: SharedString::from(format!("⌥{cmd}⌫")),
            action_id: SharedString::from("clear"),
            is_selected: false,
        },
        FooterAction {
            label: SharedString::from("Preferences…"),
            shortcut_label: SharedString::from(format!("{cmd},")),
            action_id: SharedString::from("prefs"),
            is_selected: false,
        },
        FooterAction {
            label: SharedString::from("About"),
            shortcut_label: SharedString::from(""),
            action_id: SharedString::from("about"),
            is_selected: false,
        },
        FooterAction {
            label: SharedString::from("Quit"),
            shortcut_label: SharedString::from(format!("{cmd}Q")),
            action_id: SharedString::from("quit"),
            is_selected: false,
        },
    ]
}

/// Replace the visible model with `items` and select index `select`.
/// Lives at module scope so each callback fn can call it without
/// cloning the App handle.
fn apply_items(
    weak: &slint::Weak<KlipaWindow>,
    model: &Rc<VecModel<HistoryRow>>,
    ids: &Ids,
    items: Vec<HistoryItem>,
    select: i32,
) {
    let new_ids: Vec<HistoryItemId> = items.iter().map(|i| i.id).collect();
    let cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
    let rows: Vec<HistoryRow> = items
        .into_iter()
        .enumerate()
        .map(|(idx, it)| {
            let shortcut_label = if idx < 9 {
                SharedString::from(format!("{cmd}{}", idx + 1))
            } else {
                SharedString::from("")
            };
            HistoryRow {
                id: SharedString::from(it.id.0.to_string()),
                title: SharedString::from(it.title),
                pin: SharedString::from(it.pin.unwrap_or_default()),
                shortcut_label,
                is_selected: idx as i32 == select,
            }
        })
        .collect();
    ids.replace(new_ids);
    model.set_vec(rows);
    if let Some(win) = weak.upgrade() {
        win.set_selected_index(select);
    }
}
