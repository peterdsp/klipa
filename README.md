# klipa

> A small, fast, cross-platform clipboard manager.

Pure Rust, **no system WebView**, **no JavaScript runtime**. Runs on
Windows, Linux, and macOS in a single self-contained binary.

Sibling projects:
- [`clipb`](https://github.com/peterdsp/clipb) — macOS-native (Swift / SwiftUI / AppKit) clipboard manager.
- [`kujto`](https://github.com/peterdsp/kujto) — prior cross-platform exploration.

`klipa` does **not** share code with either; it shares behavior contracts
(search rules, sort rules, history capping, dedup).

## Behavior

klipa is a **menubar-only app**. There is no dock icon, no taskbar
entry, no main menu. On launch the window stays hidden and the app
shows up as a clipboard glyph in the system status bar.

To open it:
- click the tray icon → "Show klipa"
- or press **Cmd+Shift+V** (macOS) / **Ctrl+Shift+V** (Win/Linux).

To dismiss it:
- press **Esc**, or
- press the global hotkey again.

History is **unlimited** — every copy is persisted to SQLite and kept
forever. The in-memory mirror grows linearly with usage; see below.

## Memory budget

Designed to sit far below **100 MB RSS** in normal use:

| Component | Typical RSS |
|---|---|
| Rust process (core + adapters + tokio runtime) | ~10-15 MB |
| Slint UI (femtovg renderer, GPU-accelerated) | ~15-25 MB |
| GPU buffers (OpenGL / Metal / D3D) | ~5-10 MB |
| History data (~200 bytes / item) | ~0.2 MB per 1,000 items |
| **Total at 1,000 items** | **~30-50 MB** |
| **Total at 100,000 items** | **~50-70 MB** |
| **Total at ~250,000 items** | nearing the 100 MB cap |

Since history is unlimited, RAM grows with use. At typical clipboard
volume (~50-200 items/day) you'll hit 100 MB after ~3 years. If that
becomes a concern, add a `clear_old` use case or a per-session cap.

## Architecture (Clean / Layered)

```
klipa/
├── crates/klipa-core/        ← Domain: entities, ports, use cases
│   └── src/                    pure Rust, no I/O, no Slint, no async runtime
│       ├── domain/
│       └── usecases/
└── crates/klipa-ui/          ← Shell: Slint UI + OS adapters
    ├── ui/main.slint           declarative UI markup
    └── src/
        ├── adapters/           clipboard / storage / watcher (impl core ports)
        ├── hotkey.rs           global-hotkey wrapper
        ├── tray.rs             tray-icon wrapper
        ├── app.rs              Slint ↔ HistoryService binding
        └── main.rs             composition root
```

### Dependency direction

```
Slint UI → app.rs → HistoryService (core) → Ports (traits)
                                                 ↑
                                       implemented by
                                                 │
                                  adapters/{clipboard, storage}.rs
```

Inner layers never import outer layers. `klipa-core/Cargo.toml` has zero
of: `slint`, `arboard`, `rusqlite`, `tokio` runtime, `directories`. The
compiler enforces the boundary — if a contributor tries to `use slint::`
inside `klipa-core`, `cargo check` fails.

## Features

- Cross-platform clipboard polling (`arboard`)
- SQLite-backed history (`rusqlite`), default cap of 200 items
- Global hotkey: **Cmd+Shift+V** (macOS) / **Ctrl+Shift+V** (Win/Linux)
- System tray with Show / Quit menu
- Frontmost-app capture per OS via `active-win-pos-rs`
- Keyboard nav: ↑/↓ to move, Enter to copy, Esc to clear/close,
  Cmd/Ctrl-K to clear search, Cmd/Ctrl-Backspace to delete selection
- Liquid-glass-style surface (translucent gradient + hairline border)

## Build & run

```bash
# Prerequisite: Rust toolchain (rustup default stable).
# Linux also needs: libfontconfig1-dev libxkbcommon-dev libgl1-mesa-dev \
#                   libxdo-dev (for arboard write on X11)

cargo run --release
cargo build --release       # → target/release/klipa
```

## License

MIT
