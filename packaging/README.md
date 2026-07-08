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
├── winget/       winget manifests (submit to microsoft/winget-pkgs)
├── aur/          AUR PKGBUILD + .SRCINFO (publish to aur.archlinux.org)
└── linux/        klipa.desktop  (+ deb/rpm metadata in crates/klipa-ui/Cargo.toml)
```

The Homebrew cask ([`../Casks/klipa.rb`](../Casks/klipa.rb)) and Scoop
manifest ([`../bucket/klipa.json`](../bucket/klipa.json)) live at the repo
root so **this repository doubles as the Homebrew tap and the Scoop
bucket** - no second repo to maintain. See *Package managers* below.

Icons are committed, but regenerate any time `assets/icon.svg` changes:

```bash
./scripts/make-icons.sh      # needs librsvg; macOS .icns also needs iconutil
```

---

## macOS - Developer ID `.pkg` (direct download)

A notarized `.pkg` users can download and install outside the App Store.
Requires a paid Apple Developer account and these certificates in your
login keychain (create them in *Certificates, IDs & Profiles*):

- **Developer ID Application**
- **Developer ID Installer**

```bash
export DEV_ID_APP="Developer ID Application: Petros Dhespollari (TEAMID)"
export DEV_ID_INST="Developer ID Installer: Petros Dhespollari (TEAMID)"
# Notarization (recommended) - app-specific password or a stored profile:
export AC_APPLE_ID="info@peterdsp.dev"
export AC_TEAM_ID="TEAMID"
export AC_PASSWORD="abcd-efgh-ijkl-mnop"     # app-specific password
#   or:  export AC_PROFILE="notary-profile"  # xcrun notarytool store-credentials

./scripts/package-macos.sh        # → dist/klipa-0.1.0-macos.pkg
```

Without the env vars the script still builds an **unsigned** `.pkg`
(handy for local testing; Gatekeeper will block it).

## macOS - Mac App Store `.pkg`

The App Store requires the **app sandbox**, an **Apple Distribution**
certificate, a **3rd Party Mac Developer Installer** certificate, an App
Store *provisioning profile*, and an app record in App Store Connect with
bundle id `dev.peterdsp.klipa`.

> The sandbox blocks frontmost-window inspection, so the MAS build is
> compiled with `--no-default-features --features mas`, which drops the
> `active-win-pos-rs` capture. Everything else (clipboard, menubar
> dropdown, local JSON history) works inside the sandbox.

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
privacy details - klipa collects nothing, so "Data Not Collected").

---

## Windows - `.exe` installer

Needs the **MSVC** Rust toolchain and **NSIS** (`makensis` on `PATH`).

```powershell
pwsh scripts/package-windows.ps1   # → dist/klipa-0.1.0-windows-x64-setup.exe
```

Optional Authenticode signing - set these before running and both
`klipa.exe` and the installer get signed with `signtool`:

```powershell
$env:WIN_CERT_PFX  = "C:\path\to\cert.pfx"
$env:WIN_CERT_PASS = "••••"
```

The app icon and version metadata are baked into `klipa.exe` at build
time by `build.rs` (via `winresource`, reading `packaging/icons/klipa.ico`).

---

## Linux - AppImage / deb / rpm / tarball

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

1. Add repository **secrets** for the signing you want (all optional -
   missing ones just produce unsigned artifacts):

   | Secret | Used for |
   |---|---|
   | `DEVELOPER_ID_CERTS_P12_BASE64`, `CERTS_P12_PASSWORD` | base64 of a `.p12` holding your Developer ID Application + Installer certs (direct-download `.pkg`) |
   | `CI_KEYCHAIN_PASSWORD` | password for the temporary CI keychain |
   | `AC_APPLE_ID`, `AC_TEAM_ID`, `AC_PASSWORD` | notarization of the direct-download `.pkg` |
   | `WINGET_TOKEN` | PAT to auto-submit winget updates (see above) |

   > Notarization only happens when a Developer ID **signed** pkg was
   > produced (i.e. the cert secrets are set). With no signing cert the
   > job ships an **unsigned** pkg and skips notarization.

   For the **Mac App Store** upload job (`mas`), add:

   | Secret | Used for |
   |---|---|
   | `MAS_CERTS_P12_BASE64`, `MAS_CERTS_P12_PASSWORD` | base64 of a `.p12` with **Apple Distribution** + **3rd Party Mac Developer Installer** certs |
   | `MAS_PROVISION_PROFILE_BASE64` | base64 of the App Store `.provisionprofile` |
   | `TEAMID` | your 10-char Apple Team ID |
   | `ASC_KEY_ID`, `ASC_ISSUER_ID`, `ASC_API_KEY_P8_BASE64` | App Store Connect **API key** (ASC -> Users and Access -> Integrations -> Keys) for upload |

   > The `mas` job builds the sandboxed App Store pkg and uploads it via
   > `xcrun altool`. It is skipped unless `MAS_CERTS_P12_BASE64` **and**
   > `ASC_API_KEY_P8_BASE64` are set, and it requires the **app record to
   > already exist** in App Store Connect (bundle id `dev.peterdsp.klipa`).
   > The App Store pkg is uploaded straight to ASC - it is *not* attached
   > to the public GitHub Release.

2. Tag and push:

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. The workflow builds macOS, Windows, and Linux installers, writes
   `SHA256SUMS.txt`, and publishes a GitHub Release. The website at
   <https://klipa.peterdsp.dev> picks up the new assets automatically.
   If the MAS secrets are present, the `mas` job also uploads the App
   Store build to App Store Connect (then submit it for review there).

4. The `managers` job then refreshes the Homebrew cask + Scoop manifest
   to the new version/checksums and commits them back to `main`, so
   `brew`/`scoop` users get the update with no extra work.

---

## Package managers

End-user install commands are in the [top-level README](../README.md#package-managers-cli).
This section is the **maintainer** side.

| Manager | Manifest | Hosting | Refresh |
|---|---|---|---|
| Homebrew (cask) | [`Casks/klipa.rb`](../Casks/klipa.rb) | this repo (tap) | auto (CI `managers` job) |
| Scoop | [`bucket/klipa.json`](../bucket/klipa.json) | this repo (bucket) | auto (CI `managers` job) |
| winget | [`winget/`](winget/) | microsoft/winget-pkgs | manual submit |
| AUR | [`aur/`](aur/) | aur.archlinux.org | manual push |

All five manifests are kept in sync by one script, which reads a
release's `SHA256SUMS.txt` and rewrites the version + per-artifact
checksum in each file:

```bash
# Defaults to the Cargo.toml version and that release's SHA256SUMS.txt.
./scripts/update-package-managers.sh                 # current version
./scripts/update-package-managers.sh 0.2.0           # explicit version
./scripts/update-package-managers.sh 0.2.0 ./out/SHA256SUMS.txt   # local sums
```

CI runs this automatically for the **Homebrew + Scoop** manifests (they
are served straight from this repo). **winget** and **AUR** live in
external repos, so after a release run the script, then submit:

**winget** - the **first** submission is manual with
[`wingetcreate`](https://github.com/microsoft/winget-create) (winget needs
an existing package before it can auto-update):

```bash
wingetcreate submit --token <gh-token> packaging/winget/
# or open a PR against microsoft/winget-pkgs with the three YAML files.
```

After that first PR is merged, the release workflow's `winget` job keeps
it updated automatically on every tag - set repo secret **`WINGET_TOKEN`**
to a classic PAT (scope `public_repo`) on your fork of
`microsoft/winget-pkgs`. Without the secret the job is skipped.

**AUR** - push to the `klipa-bin` package repo (one-time `git clone
ssh://aur@aur.archlinux.org/klipa-bin.git`):

```bash
cp packaging/aur/PKGBUILD packaging/aur/.SRCINFO /path/to/aur/klipa-bin/
cd /path/to/aur/klipa-bin && git commit -am "klipa 0.2.0" && git push
```

> `.SRCINFO` must match `PKGBUILD`; regenerate it in a checkout with
> `makepkg --printsrcinfo > .SRCINFO` if you hand-edit the PKGBUILD.

---

## Licensing / paywall (non-App-Store builds)

The paid gate lives in [`../crates/klipa-ui/src/license.rs`](../crates/klipa-ui/src/license.rs)
behind the `license` cargo feature (on by default, **off** for the Mac App
Store build via `--no-default-features --features mas`). It is a 7-day
free trial, then a one-time **€1.99** unlock.

**How it works.** Payment is on **Ko-fi**. A self-hosted license server
(the shared multi-product service in `../scripts/pi-license-server/`)
receives the Ko-fi webhook, signs an **Ed25519** license tied to the
buyer's email, and emails it (inline text + a `.klipa` attachment). To
activate, the buyer copies the license contents to the clipboard and
clicks *Activate*: klipa verifies the signature **offline** against the
public key baked into `license.rs`. No network at activation.

Activation deliberately requires the **signed file**, not just an email:
the server's `/activate` email-lookup is disabled for klipa
(`email_activation=False`), so knowing a buyer's address is not enough to
unlock - you need the license they were emailed. Since a valid license is
a permanent signed file, there is no online re-verification (a refund
can't be revoked remotely) - an accepted tradeoff for a €1.99 open-source
"honest nudge" gate.

Two **build-time** settings are baked in via `option_env!` (not secret -
they ship in the binary; both optional since the source defaults are
correct):

| Repo variable | Meaning | Default if unset |
|---|---|---|
| `KLIPA_PURCHASE_URL` | where *Unlock* sends the buyer | `https://ko-fi.com/s/4e1cf2ac40` |

Set them as GitHub **Actions variables** (Settings -> Secrets and
variables -> Actions -> *Variables*); the macOS / Windows / Linux build
jobs already read `KLIPA_PURCHASE_URL`.

**Server setup:** see `../scripts/pi-license-server/README.md`. The public
signing key is the `LICENSE_PUBKEY_B64` constant in `license.rs`; the
private key lives only on the Pi. To move to a different store, point the
webhook at the server and adjust the item match.

To build a licensed binary locally for testing:

```bash
cargo build --release                                      # license on (default)
cargo build --release --no-default-features --features mas # App Store: no gate
```

The trial state lives in `license.json` in the user's data dir; deleting
it restarts the trial (expected - this is a nudge, not DRM).
