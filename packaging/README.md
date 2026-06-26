# Packaging klipa

Everything needed to turn `target/release/klipa` into a signed,
distributable installer on each platform. The scripts live in
[`../scripts/`](../scripts/); CI runs them on a `v*` tag via
[`../.github/workflows/release.yml`](../.github/workflows/release.yml).

```
packaging/
├── icons/        generated .icns / .ico / hicolor PNGs  (make-icons.sh)
├── macos/        Info.plist + entitlements (Developer ID + App Store)
├── windows/      NSIS installer script (klipa.nsi)
└── linux/        klipa.desktop  (+ deb/rpm metadata in crates/klipa-ui/Cargo.toml)
```

Icons are committed, but regenerate any time `assets/icon.svg` changes:

```bash
./scripts/make-icons.sh      # needs librsvg; macOS .icns also needs iconutil
```

---

## macOS — Developer ID `.pkg` (direct download)

A notarized `.pkg` users can download and install outside the App Store.
Requires a paid Apple Developer account and these certificates in your
login keychain (create them in *Certificates, IDs & Profiles*):

- **Developer ID Application**
- **Developer ID Installer**

```bash
export DEV_ID_APP="Developer ID Application: Petros Dhespollari (TEAMID)"
export DEV_ID_INST="Developer ID Installer: Petros Dhespollari (TEAMID)"
# Notarization (recommended) — app-specific password or a stored profile:
export AC_APPLE_ID="info@peterdsp.dev"
export AC_TEAM_ID="TEAMID"
export AC_PASSWORD="abcd-efgh-ijkl-mnop"     # app-specific password
#   or:  export AC_PROFILE="notary-profile"  # xcrun notarytool store-credentials

./scripts/package-macos.sh        # → dist/klipa-0.1.0-macos.pkg
```

Without the env vars the script still builds an **unsigned** `.pkg`
(handy for local testing; Gatekeeper will block it).

## macOS — Mac App Store `.pkg`

The App Store requires the **app sandbox**, an **Apple Distribution**
certificate, a **3rd Party Mac Developer Installer** certificate, an App
Store *provisioning profile*, and an app record in App Store Connect with
bundle id `dev.peterdsp.klipa`.

> The sandbox blocks frontmost-window inspection, so the MAS build is
> compiled with `--no-default-features --features mas`, which drops the
> `active-win-pos-rs` capture. Everything else (clipboard, global hotkey,
> SQLite history) works inside the sandbox.

```bash
export MAS_APP="Apple Distribution: Petros Dhespollari (TEAMID)"
export MAS_INST="3rd Party Mac Developer Installer: Petros Dhespollari (TEAMID)"
export TEAMID="TEAMID"
export PROFILE="/path/to/klipa_appstore.provisionprofile"

./scripts/package-mas.sh          # → dist/klipa-0.1.0-mas.pkg
```

Upload to App Store Connect with **Transporter.app**, or:

```bash
xcrun altool --upload-app -f dist/klipa-0.1.0-mas.pkg -t macos \
  --apiKey <KEY_ID> --apiIssuer <ISSUER_ID>
```

Then submit for review in App Store Connect (screenshots, description,
privacy details — klipa collects nothing, so "Data Not Collected").

---

## Windows — `.exe` installer

Needs the **MSVC** Rust toolchain and **NSIS** (`makensis` on `PATH`).

```powershell
pwsh scripts/package-windows.ps1   # → dist/klipa-0.1.0-windows-x64-setup.exe
```

Optional Authenticode signing — set these before running and both
`klipa.exe` and the installer get signed with `signtool`:

```powershell
$env:WIN_CERT_PFX  = "C:\path\to\cert.pfx"
$env:WIN_CERT_PASS = "••••"
```

The app icon and version metadata are baked into `klipa.exe` at build
time by `build.rs` (via `winresource`, reading `packaging/icons/klipa.ico`).

---

## Linux — AppImage / deb / rpm / tarball

Build dependencies (Debian/Ubuntu):

```bash
sudo apt-get install -y libfontconfig1-dev libxkbcommon-dev libgl1-mesa-dev \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
cargo install cargo-deb cargo-generate-rpm           # pure-Rust packagers
# AppImage:
sudo wget -O /usr/local/bin/appimagetool \
  https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
sudo chmod +x /usr/local/bin/appimagetool
```

```bash
./scripts/package-linux.sh        # builds whatever tooling is present:
#   dist/klipa-0.1.0-linux-x86_64.tar.gz
#   dist/klipa_0.1.0_amd64.deb
#   dist/klipa-0.1.0-1.x86_64.rpm
#   dist/klipa-0.1.0-x86_64.AppImage
```

The `.deb`/`.rpm` payloads are defined under `[package.metadata.deb]` and
`[package.metadata.generate-rpm]` in
[`../crates/klipa-ui/Cargo.toml`](../crates/klipa-ui/Cargo.toml).

---

## CI release (recommended)

1. Add repository **secrets** for the signing you want (all optional —
   missing ones just produce unsigned artifacts):

   | Secret | Used for |
   |---|---|
   | `MAC_CERT_P12`, `MAC_CERT_PASSWORD` | base64 of a `.p12` holding your Developer ID certs + key |
   | `DEV_ID_APP`, `DEV_ID_INST` | Developer ID identity names |
   | `AC_APPLE_ID`, `AC_TEAM_ID`, `AC_PASSWORD` | notarization |
   | `MAS_APP`, `MAS_INST`, `TEAMID` | Mac App Store `.pkg` |

2. Tag and push:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. The workflow builds macOS, Windows, and Linux installers, writes
   `SHA256SUMS.txt`, and publishes a GitHub Release. The website at
   <https://klipa.peterdsp.dev> picks up the new assets automatically.
