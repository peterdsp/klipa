# Keep-awake release checklist

The release is intentionally NOT cut. This records the exact state, the
blocking gates, and the corrected process, so the owner can finish it when
the physical gate passes and signing/notarization credentials are available.

Status date: 2026-10-01.

## Candidate

- Version: `0.6.0` (a minor, because the helper/session contract was
  materially redesigned: authenticated versioned IPC, ownership lease,
  root-owned journal). Set in `Cargo.toml` `[workspace.package]` and
  propagated to `Cargo.lock`, the app/helper binaries, and the bundle
  `Info.plist` (templated by `bundle-macos.sh`) and the helper's embedded
  `Info.plist` (from `CARGO_PKG_VERSION`).
- Branch: `keep-awake-helper-ownership`, merged to `main` after CI passes.
- Package-manager manifests (Casks, bucket, winget, AUR) are NOT touched
  here; the hardened `release.yml` updates them only on the real
  `release: published` event, from published checksums, so `brew`/`scoop`
  never point at assets that do not exist yet.
- New runtime surface to validate at signing time: the helper now pins the
  caller Team ID (`KLIPA_TEAM_ID`, baked by `package-macos.sh` from the
  Developer ID identity) and exposes an authenticated socket protocol.

## Current state (2026-10-01): signed/notarized DRAFT staged

- PR #25 squash-merged to `main` (`b5e9a5b`); tag `v0.6.0` pushed; release
  workflow run 36884600655 fully green.
- The `v0.6.0` GitHub Release is a DRAFT with all platform artifacts +
  `SHA256SUMS.txt`. The macOS `.pkg`/`.zip` are Developer ID signed,
  notarized, stapled, and `spctl`-accepted (re-verified locally). The MAS
  build was uploaded to App Store Connect (processing, not public).
- NOT published. Public "Latest" is still v0.5.4. brew/scoop/winget and the
  website are untouched. Full evidence in `docs/keep-awake-verification.md`.

To finish, the owner: (1) runs the physical closed-lid gate on the failing
Mac (see the verification doc), (2) if it passes, publishes the draft
release (which fires the manifest/winget jobs from the published
checksums), and (3) updates the website download/support copy to match.

## App / helper compatibility

- The socket protocol is unchanged (`set 1` / `set 0` / `ping`), so a new
  app and the existing helper remain compatible. The helper now bounds
  request size/time but still speaks the same vocabulary.
- The on-disk recovery record changed from the legacy boolean
  `lid_awake.on` to `lid_awake.json`. Migration is handled: a leftover
  legacy marker is reconciled at startup as ambiguous ownership (restore to
  normal sleep when the flag is really set), so upgrading does not strand a
  prior override. Verify this on the real old-to-new upgrade path.

## Blocking gates (must all pass before publishing)

1. PHYSICAL closed-lid gate on the owner's failing Mac: NOT SATISFIED. See
   `docs/keep-awake-verification.md`. Owner action: run the listed cases on
   the real hardware with the packaged candidate and record pass/fail,
   durations, and artifact hashes. This cannot be delegated to CI or a VM.
2. Developer ID signing + notarization credentials: PRESENT as GitHub repo
   secrets (confirmed by name only, never read:
   `DEVELOPER_ID_CERTS_P12_BASE64`, `CERTS_P12_PASSWORD`,
   `CI_KEYCHAIN_PASSWORD`, `APPLE_ID`, `TEAM_ID`,
   `APPLE_APP_SPECIFIC_PASSWORD`, plus the MAS + ASC API-key secrets).
   VALIDITY is confirmed only by actually running the release workflow (a
   tag push stages a signed/notarized DRAFT; it does not publish). No
   signing material was accessed, printed, or committed. Local signing is
   not possible here (no Developer ID identity in the local keychain), so
   the signed/notarized candidate must come from the Actions runner.
3. App Store (MAS) upload: reported SEPARATELY. Missing MAS access does not
   block the direct-download release and must not be used to mark it
   incomplete. If MAS secrets exist, the `mas` job validates + uploads to
   App Store Connect; upload, processing, review, and public availability
   are distinct states to report individually.

## Corrected release process (implemented in `.github/workflows/release.yml`)

The workflow was hardened so a tag push can no longer auto-publish
unverified or unsigned artifacts:

- A `verify` job runs the required checks (build + clippy `-D warnings` +
  tests + the App Store feature-combo tests) on the EXACT candidate commit.
  The macOS/Windows/Linux/MAS build jobs now `needs: [verify]`, so a
  candidate that fails the gate never produces an artifact.
- The macOS Gatekeeper/nested-code check no longer swallows failure with
  `|| true`; when signing secrets are present it runs
  `pkgutil --check-signature` + `spctl --assess` and FAILS the job on a bad
  assessment.
- Pushing a tag stages a DRAFT GitHub Release (never public). A human must
  review signatures/notarization and the physical gate, then publish.
- Package-manager manifest updates (`managers`, `winget`) now run only on
  the real `release: published` event, from the published SHA256SUMS, so
  `brew`/`scoop` are never bumped to a not-yet-public version.

## Pre-publish verification steps (run against the final candidate)

Before publishing the draft:

- `codesign --verify --strict --deep` on the app, plus explicit nested
  verification of `Contents/MacOS/klipa-helper` and the embedded
  LaunchDaemon plist (`Contents/Library/LaunchDaemons/`).
- `pkgutil --check-signature` on the installer; `spctl --assess` on the
  installed app and installer; `xcrun stapler validate` for notarization
  stapling; confirm actual notarization status (not just that the command
  ran).
- Inspect the embedded helper's designated requirement, entitlements, and
  version; confirm app/helper identity/version agree.
- Validate BOTH channels: the `.pkg` installer and the separately
  distributed `-macos.zip` update.
- Confirm Windows/Linux artifacts still build and regress-check cleanly
  (CI covers this; the release build jobs rebuild per-OS).

## Post-publish verification (after the owner publishes)

- Download the published macOS artifact, verify its SHA256 against the
  staged hash, and smoke-test install / launch / helper status from that
  exact artifact.
- Confirm the in-app updater finds the new version and the website download
  links resolve.
- Confirm `managers`/`winget` updated manifests from the published
  checksums; report winget/AUR external submission states separately.

## Rollback

- If a release regresses: restore ONLY klipa-owned sleep changes via the
  recovery journal path (never remove a user's unrelated power settings),
  roll the published assets back to the previous version, and restore
  app/helper compatibility by reinstalling the prior signed bundle. Keep the
  previous bundle until the new app + helper are confirmed healthy.

## Not done (explicitly incomplete)

- The physical closed-lid gate on the owner's Mac (gate 1). This is the one
  hard blocker to publication.
- Public release NOT published, package managers NOT bumped, website NOT
  changed. A signed/notarized DRAFT staged by the tag push is the holding
  state until the owner confirms the physical test passed.
- The live `SecCode` audit-token path, the root daemon under launchd, and
  the socket round-trips are compile-verified with unit-tested logic but
  not runtime-verified here (device-gated).
- macOS 11/12 weak-linking/deployment-target confirmation (needs an 11/12
  machine); the helper is gated to macOS 13+ with an admin-prompt fallback
  below that.

The authenticated-IPC helper redesign, the verified UI state model, the
post-trial control reachability, and the updater handover ARE implemented
and merged (see the investigation doc "Update" section).
