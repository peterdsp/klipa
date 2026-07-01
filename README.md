<p align="center">
  <img src="packaging/icons/klipa.png" alt="klipa" width="160" height="160" />
</p>

# klipa

> A small, fast, cross-platform clipboard manager.

Pure Rust, **no system WebView**, **no JavaScript runtime**, **no GPU
renderer**. A ~1.6 MB self-contained binary on Windows, Linux, and macOS.

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

### Package managers (CLI)

**macOS - Homebrew** (this repo is the tap):

```bash
brew tap peterdsp/klipa https://github.com/peterdsp/klipa
brew install --cask klipa
```

**Windows - winget** or **Scoop** (this repo is the bucket):

```powershell
winget install dev.peterdsp.klipa
# or
scoop bucket add klipa https://github.com/peterdsp/klipa
scoop install klipa
```

**Linux - Arch (AUR)** or **build with cargo**:

```bash
yay -S klipa-bin          # any AUR helper
# or, on any distro with a Rust toolchain:
cargo install --git https://github.com/peterdsp/klipa klipa-ui
```

The Homebrew cask and Scoop manifest are auto-bumped on each release;
winget and AUR are submitted per release (see
[`packaging/README.md`](packaging/README.md)).

### Pricing

klipa is **free in full for 7 days**, then **€1.99 once** to keep using
it - a single license key, no subscription. After the trial the menubar
dropdown shows an *Unlock full version* item; buy a key, copy it, and
click *Activate (paste license key)*. The key is verified once online and
then works offline.

The **Mac App Store** build is a normal paid app - the store handles
payment, so it has no trial or license key (that whole mechanism is
compiled out). klipa is MIT-licensed, so the paywall only applies to the
prebuilt binaries; you can always build it yourself from source.

## Behavior

klipa is a **menubar-only app** - no dock icon, no taskbar entry, no
window. It lives as a clipboard glyph in the system status bar.

**Click the menubar icon** and your recent clipboard entries drop down.
Click an entry to copy it back, ready to paste. Below the history the
menu has "Clear history", a **Keep awake** submenu, "Hide menubar icon",
and "Quit klipa". That native dropdown is the whole UI - no separate
window, no GPU renderer.

History is stored in a **plain local file** under your data dir. It
never leaves your device - nothing is logged, uploaded, or sent
anywhere. klipa keeps the most recent 200 entries.

### Keep awake

An Amphetamine-style keep-awake session stops your machine from idle
sleeping. Open **Keep awake** and pick a duration - *Indefinitely*, or
5 / 15 / 30 minutes, 1 / 2 / 5 hours - and klipa holds the system awake
until the timer elapses or you choose **End current session**. Toggle
**Allow display sleep** to let the screen sleep while the system stays
awake (macOS/Windows).

It uses each OS's native mechanism, with no extra dependency:

| Platform | Mechanism |
|---|---|
| macOS | built-in `caffeinate` tool |
| Windows | `SetThreadExecutionState` (Win32) |
| Linux | `systemd-inhibit` idle inhibitor (needs systemd-logind) |

On Linux the idle inhibitor covers the whole idle path (screen blank +
auto-suspend together), so **Allow display sleep** has no separate
effect there.

### Hide menubar icon

**Hide menubar icon** removes klipa from the status bar while it keeps
running and watching the clipboard. Relaunch klipa to bring the icon
back.

### Menu bar display

Open **Menu bar** and pick what appears next to the tray icon:

- **Icon only** (default) - nothing beyond the little clipboard glyph.
- **Date** - short local date, e.g. `Wed 30`.
- **Temperature** - current temperature at your location, e.g. `22°`.
- **Date + Temperature** - both, e.g. `Wed 30  22°`.

Your choice is saved in `settings.json` next to `history.json`.

> **Network note.** The **Icon only** and **Date** modes make **zero
> network calls** - klipa's core promise ("local-only, no network") is
> unchanged. Only the two temperature modes reach the internet: coarse
> location from your IP via ip-api.com (cached 24 h) and the temperature
> from open-meteo.com (cached 10 min). Both are free, keyless, and
> ~6 requests/hour while the mode is on.

On macOS and Linux (ayatana-appindicator) the text renders **next to**
the icon in the menu bar. On Windows the tray backend does not display
title text, so the value shows in the tooltip instead.

## Footprint

| | |
|---|---|
| Binary size | ~1.6 MB (no bundled TLS or HTTP client) |
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
        ├── awake.rs     keep-awake sessions (caffeinate/Win32/systemd-inhibit)
        ├── license.rs   7-day trial + €1.99 unlock (off in the App Store build)
        ├── settings.rs  persistent user prefs (menu bar display mode)
        ├── weather.rs   opt-in IP location + open-meteo temperature
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
- **Keep-awake sessions** - timed or indefinite, native on macOS / Windows / Linux
- **Hide menubar icon** while klipa keeps running in the background
- **Menu bar display**: icon only (default), date, temperature, or both
- **7-day free trial**, then a one-time **€1.99** unlock (App Store build excluded)
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
./scripts/make-icons.sh        # regenerate icons from assets/icon.png
./scripts/package-macos.sh     # macOS Developer ID .pkg (+ notarize)
./scripts/package-mas.sh       # Mac App Store .pkg
pwsh scripts/package-windows.ps1   # Windows .exe installer (NSIS)
./scripts/package-linux.sh     # .AppImage / .deb / .rpm / .tar.gz
```

See [`packaging/README.md`](packaging/README.md) for signing credentials
and Mac App Store submission steps.

## License

MIT
