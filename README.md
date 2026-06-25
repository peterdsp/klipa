# klipa

> A small, fast, cross-platform clipboard manager.

Pure Rust, **no system WebView**, **no JavaScript runtime**. Runs on
Windows, Linux, and macOS in a single self-contained binary.

Sibling projects:
- [`clipb`](https://github.com/peterdsp/clipb) — macOS-native (Swift / SwiftUI / AppKit) clipboard manager.
- [`kujto`](https://github.com/peterdsp/kujto) — prior cross-platform exploration.

`klipa` does **not** share code with either; it shares behavior contracts
(search rules, sort rules, history capping, dedup).

## Memory budget (hard constraint)

`klipa` is built to never cross **100 MB RSS in any case, ever.**

| Component | Typical RSS |
|---|---|
| Rust process (core + adapters + tokio runtime) | ~10-15 MB |
| Slint UI (femtovg renderer, GPU-accelerated) | ~15-25 MB |
| GPU buffers (OpenGL / Metal / D3D) | ~5-10 MB |
| History data (200 items) | <1 MB |
| **Total steady-state** | **~30-50 MB** — headroom: 50-70 MB under cap |

If RSS climbs over 60 MB in steady state, that's a regression and a bug.

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

## Status

Functional scaffold. The tray icon ships a 1×1 transparent placeholder —
drop a real PNG and update `tray::default_icon()` before shipping.
Wayland users will see no global-hotkey support (security restriction).

## License

MIT
