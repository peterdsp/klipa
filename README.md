# klipa

> A small, fast, cross-platform clipboard manager.

Pure Rust, **no system WebView**, **no JavaScript runtime**, **no GPU
renderer**. A ~1.3 MB self-contained binary on Windows, Linux, and macOS.

## Install

Downloads for every platform: **<https://klipa.peterdsp.dev>**

| Platform | Package |
|---|---|
| macOS 11+ | `.pkg` (universal, signed + notarized) - also on the Mac App Store |
| Windows 10/11 | `.exe` installer (64-bit) |
| Linux (x86-64) | `.AppImage`, `.deb`, `.rpm`, or portable `.tar.gz` |

All builds and `SHA256SUMS.txt` are attached to each
[GitHub release](https://github.com/peterdsp/klipa/releases). See
[`packaging/README.md`](packaging/README.md) for how the artifacts are
built and signed.

## Behavior

klipa is a **menubar-only app** - no dock icon, no taskbar entry, no
window. It lives as a clipboard glyph in the system status bar.

**Click the menubar icon** and your recent clipboard entries drop down.
Click an entry to copy it back, ready to paste. The bottom of the menu
has "Clear history" and "Quit klipa". That native dropdown is the whole
UI - no separate window, no GPU renderer.

History is stored in a **plain local file** under your data dir. It
never leaves your device - nothing is logged, uploaded, or sent
anywhere. klipa keeps the most recent 200 entries.

## Footprint

| | |
|---|---|
| Binary size | ~1.3 MB |
| Typical RSS | ~8-15 MB |
| History store | last 200 entries in a small JSON file |

No Electron, no browser engine, no GPU. The UI is a native menu, so
there is nothing to render.

## Architecture (Clean / Layered)

```
klipa/
├── crates/klipa-core/   ← Domain: entities, ports, use cases (pure Rust, no I/O, no OS)
│   └── src/{domain,usecases}
└── crates/klipa-ui/     ← Shell: tray UI + OS adapters
    └── src/
        ├── adapters/    clipboard / storage (JSON) / watcher  (impl core ports)
        ├── tray.rs      menubar icon + history dropdown (tray-icon + muda)
        ├── platform.rs  macOS menubar-accessory tweak
        └── main.rs      composition root + winit event loop
```

### Dependency direction

```
tray menu (main.rs) → HistoryService (core) → Ports (traits)
                                                   ↑ implemented by
                                       adapters/{clipboard, storage}
```

Inner layers never import outer layers. `klipa-core` has zero of:
`arboard`, `tokio` runtime, `tray-icon`, `winit`, `directories`.

## Features

- Cross-platform clipboard polling (`arboard`)
- History kept in a **single local file** on your device, last 200 entries
- Native **menubar dropdown** of recent copies; click to paste (`tray-icon`)
- Frontmost-app capture per OS via `active-win-pos-rs`
- macOS menubar accessory (no dock icon)

## Build & run

```bash
# Prerequisite: Rust toolchain (rustup default stable).
# Linux also needs: libfontconfig1-dev libxkbcommon-dev libgl1-mesa-dev \
#                   libgtk-3-dev libxdo-dev libayatana-appindicator3-dev \
#                   libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev

cargo run --release
cargo build --release       # → target/release/klipa
```

### Packaging

Per-platform installers are produced by the scripts in [`scripts/`](scripts/)
and, on a `v*` git tag, by the [release workflow](.github/workflows/release.yml):

```bash
./scripts/make-icons.sh        # regenerate icons from assets/icon.svg
./scripts/package-macos.sh     # macOS Developer ID .pkg (+ notarize)
./scripts/package-mas.sh       # Mac App Store .pkg
pwsh scripts/package-windows.ps1   # Windows .exe installer (NSIS)
./scripts/package-linux.sh     # .AppImage / .deb / .rpm / .tar.gz
```

See [`packaging/README.md`](packaging/README.md) for signing credentials
and Mac App Store submission steps.

## License

MIT
