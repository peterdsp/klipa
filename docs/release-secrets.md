# Release secrets and one-time setup

klipa ships through two macOS channels plus Windows and Linux. All signing
and store credentials live only in GitHub Actions Secrets (write-only) and in
your local backup of the signing material. This doc is the runbook to
recreate everything from scratch if you ever lose that backup: revoke, reissue,
re-upload the base64, done.

The single workflow is [`.github/workflows/release.yml`](../.github/workflows/release.yml).
Tagging `vX.Y.Z` on `main` fans out to the `macos`, `windows`, `linux`, `mas`,
`release`, `managers`, and `winget` jobs.

## Channel 1: Mac App Store (job `mas`)

Sandboxed `.pkg` built by `scripts/package-mas.sh`, uploaded to App Store
Connect via `xcrun altool` and an ASC API key. Independent of the direct
download; the App Store build has the `license` cargo feature compiled OUT
because the store handles paid unlock instead.

### Certificates and profile

1. In [Apple Developer > Certificates](https://developer.apple.com/account/resources/certificates/list),
   create both:
   - **Apple Distribution** (signs the .app)
   - **3rd Party Mac Developer Installer** (signs the .pkg)
2. Export the pair from Keychain Access as one `.p12` with a strong password.
3. In [Apple Developer > Profiles](https://developer.apple.com/account/resources/profiles/list),
   create a Mac App Store provisioning profile for bundle id
   `dev.peterdsp.klipa` and download the `.provisionprofile`.
4. Base64-encode both for GitHub Secrets:

```
base64 -i klipa-mas-certs.p12          -o klipa-mas-certs.p12.b64
base64 -i klipa_mas.provisionprofile   -o klipa_mas.provisionprofile.b64
```

The distribution certs + provisioning profile can be created through the App
Store Connect API instead of the portal UI (avoids session logouts). The
current App ID is `6786092508`, bundle id `dev.peterdsp.klipa`, Team id
`YTS4KJBX3P`.

### App Store Connect API key

1. In [App Store Connect > Users and Access > Integrations > Keys](https://appstoreconnect.apple.com/access/api),
   create an API key with the **App Manager** role.
2. Download the `.p8` file. Apple only lets you download it ONCE, ever.
3. Note the Issuer ID and the Key ID.
4. Base64-encode the `.p8` for the secret:

```
base64 -i AuthKey_XXXXXXXX.p8 -o AuthKey_XXXXXXXX.p8.b64
```

### GitHub secrets for the `mas` job

| Name | Value |
| --- | --- |
| `MAS_CERTS_P12_BASE64` | Base64 of the Apple Distribution + 3rd Party Installer `.p12` |
| `MAS_CERTS_P12_PASSWORD` | Password for the `.p12` |
| `MAS_PROVISION_PROFILE_BASE64` | Base64 of the `.provisionprofile` |
| `TEAMID` | 10-character Apple Team ID (e.g. `YTS4KJBX3P`) |
| `ASC_KEY_ID` | Key ID from ASC (e.g. `B3BD3SK79A`) |
| `ASC_ISSUER_ID` | Issuer ID from ASC |
| `ASC_API_KEY_P8_BASE64` | Base64 of the `AuthKey_*.p8` |
| `CI_KEYCHAIN_PASSWORD` | Any strong password, used only inside the runner |

Upload via CLI (never paste base64 into the browser):

```
gh secret set MAS_CERTS_P12_BASE64          < klipa-mas-certs.p12.b64
gh secret set MAS_PROVISION_PROFILE_BASE64  < klipa_mas.provisionprofile.b64
gh secret set ASC_API_KEY_P8_BASE64         < AuthKey_XXXXXXXX.p8.b64
gh secret set MAS_CERTS_P12_PASSWORD
gh secret set TEAMID
gh secret set ASC_KEY_ID
gh secret set ASC_ISSUER_ID
gh secret set CI_KEYCHAIN_PASSWORD
```

## Channel 2: Direct download (job `macos`)

Regular Developer ID signed `.pkg` on the GitHub Release, plus mirrored via
the in-repo Homebrew cask (`Casks/klipa.rb`) and Scoop bucket
(`bucket/klipa.json`). Built by `scripts/package-macos.sh`. If the Developer
ID secrets below are not set, the job still builds an ad-hoc unsigned `.pkg`
and skips notarization; Gatekeeper warns on first launch but nothing breaks.

### Certificates

1. Create both in Apple Developer:
   - **Developer ID Application** (signs the .app)
   - **Developer ID Installer** (signs the .pkg)
2. Export both together as one `.p12` (via Keychain Access, select both, right
   click, Export 2 items). Strong password.
3. Base64-encode.

### Notarization

Create an app-specific password at [appleid.apple.com](https://appleid.apple.com)
under Sign-In and Security > App-Specific Passwords. Notarization uses this
password plus your Apple ID plus the Team ID.

### GitHub secrets for the `macos` job

| Name | Value |
| --- | --- |
| `DEVELOPER_ID_CERTS_P12_BASE64` | Base64 of the Developer ID Application + Installer `.p12` |
| `CERTS_P12_PASSWORD` | Password for the `.p12` |
| `APPLE_ID` | The Apple ID email used for notarization |
| `TEAM_ID` | 10-character Apple Team ID (distinct secret from `TEAMID` above) |
| `APPLE_APP_SPECIFIC_PASSWORD` | The app-specific password |
| `CI_KEYCHAIN_PASSWORD` | Reused across `macos` + `mas` jobs |

## Channel 3: Windows winget (job `winget`)

Opens a PR to `microsoft/winget-pkgs` when a tag ships. Skipped if the token
is not set. The first-ever winget submission must be done manually; see
`packaging/README.md`.

| Name | Value |
| --- | --- |
| `WINGET_TOKEN` | Classic PAT with `public_repo` scope that can push to your fork of `winget-pkgs` |

## Build-time variables (not secrets)

Baked into the binary at compile time for the license gate. Set in
[Actions > Variables](https://github.com/peterdsp/klipa/settings/variables/actions),
not Secrets, because they are baked into the shipped binary anyway.

| Name | Value |
| --- | --- |
| `KLIPA_GUMROAD_PRODUCT_ID` | Product id used by the license verify call |
| `KLIPA_PURCHASE_URL` | Public buy URL surfaced in the trial-expired dialog |

## Tagging a release

```
git tag v0.4.4 && git push origin v0.4.4   # ships everything the secrets allow
```

You can also `gh workflow run release.yml` (workflow_dispatch) to build all
channels without cutting a public Release; the `release` / `managers` /
`winget` jobs skip when there is no tag, so this is how you push a MAS-only
build to App Store Connect between public releases.

## Backup of the raw signing material

GitHub Secrets are write-only. Once you paste a value in you can never read it
back, only overwrite it. That means the local copies of the `.p12`, `.p8`,
`.provisionprofile`, and their passwords are the only way you can rotate to a
new CI, use them on another Mac, or debug signing locally.

Do not check them into the repo (encrypted or otherwise) - this repo is public
and any pushed ciphertext is permanent.

Store the folder in a durable location:

- Password manager attachment (1Password, Bitwarden), or
- Encrypted DMG in iCloud Drive:
  `hdiutil create -encryption AES-256 -stdinpass -size 20m -fs APFS -volname klipa-signing klipa-signing.dmg`

Local backup is currently at `~/Documents/klipa-mas-signing/`.

## What each stored file is

Inside the signing folder:

| File | Purpose | Recoverable? |
| --- | --- | --- |
| `AuthKey_*.p8` | ASC API private key | No, Apple only serves it once |
| `klipa-dist.key` | Private key paired with the Apple Distribution cert | No, must reissue the cert if lost |
| `klipa-mas-certs.p12` | `.key` + certs bundled with a password | Rebuildable from `.key` + `.cer` |
| `p12-password.txt` | Password for the `.p12` | No, `.p12` is unopenable without it |
| `apple-distribution.cer` | Public Apple Distribution cert | Redownloadable from Apple Developer |
| `mac-installer.cer` | Public 3rd Party Installer cert | Redownloadable from Apple Developer |
| `klipa_mas.provisionprofile` | MAS provisioning profile | Regeneratable in Apple Developer |
| `klipa-dist.csr` | The CSR used to request the cert | Useless once the cert has been issued |
