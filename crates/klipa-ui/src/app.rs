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
use std::cell::RefCell;
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
    tokio: TokioHandle,
    ids: Ids,
}

impl App {
    pub fn new(history: Arc<HistoryService>, tokio: TokioHandle) -> Self {
        let window = KlipaWindow::new().expect("slint window");
        let model = Rc::new(VecModel::<HistoryRow>::from(vec![]));
        window.set_items(ModelRc::from(model.clone()));
        Self {
            window,
            history,
            model,
            tokio,
            ids: Rc::new(RefCell::new(vec![])),
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
        self.wire_clear();
        self.wire_key_pressed();
        self.refresh_blocking();
    }

    pub fn run(&self) -> Result<(), slint::PlatformError> {
        self.window.run()
    }

    // ── Public helpers used by main.rs (hotkey / tray)──────────────────

    pub fn show_and_focus(&self) {
        let win = self.window.window();
        win.show().ok();
    }

    pub fn hide(&self) {
        let win = self.window.window();
        win.hide().ok();
    }

    pub fn toggle(&self) {
        // Slint doesn't expose is-visible directly across all platforms,
        // so we shadow it via a property if we need it later. For now
        // each call is idempotent via the window's own state.
        let win = self.window.window();
        win.show().ok();
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
        self.window.on_row_activated(move |idx: i32| {
            let Some(id) = ids.borrow().get(idx as usize).copied() else {
                return;
            };
            let h = h.clone();
            t.spawn(async move {
                if let Err(e) = h.copy_to_clipboard(id).await {
                    tracing::warn!(?e, "copy failed");
                }
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

    fn wire_clear(&self) {
        let h = self.history.clone();
        let t = self.tokio.clone();
        let weak = self.window.as_weak();
        let model = self.model.clone();
        let ids = self.ids.clone();
        self.window.on_clear_clicked(move || {
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
        });
    }

    /// Richer key handling: arrow nav, Enter/Escape, plus Cmd/Ctrl-K
    /// for "clear search" and Cmd/Ctrl-Backspace for delete-selection.
    fn wire_key_pressed(&self) {
        let weak = self.window.as_weak();
        self.window.on_key_pressed(move |event| {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let total = win.get_items().row_count() as i32;
            let sel = win.get_selected_index();
            let mods = &event.modifiers;
            let primary = mods.control || mods.meta; // Ctrl on Win/Linux, Cmd on macOS

            // Modifier chords first (so Cmd-K doesn't also match the K key).
            if primary {
                match event.text.as_str() {
                    "k" | "K" => {
                        win.set_query(SharedString::from(""));
                        return;
                    }
                    "\u{8}" /* backspace */ => {
                        if sel >= 0 {
                            win.invoke_row_delete_requested(sel);
                        }
                        return;
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
    let rows: Vec<HistoryRow> = items
        .into_iter()
        .enumerate()
        .map(|(idx, it)| HistoryRow {
            id: SharedString::from(it.id.0.to_string()),
            title: SharedString::from(it.title),
            pin: SharedString::from(it.pin.unwrap_or_default()),
            is_selected: idx as i32 == select,
        })
        .collect();
    ids.replace(new_ids);
    model.set_vec(rows);
    if let Some(win) = weak.upgrade() {
        win.set_selected_index(select);
    }
}
