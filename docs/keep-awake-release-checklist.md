# Keep-awake release checklist

The release is intentionally NOT cut. This records the exact state, the
blocking gates, and the corrected process, so the owner can finish it when
the physical gate passes and signing/notarization credentials are available.

Status date: 2026-10-01.

## Candidate

- Base commit: `368cf36` (version 0.5.4) plus the keep-awake reliability
  changes from this pass.
- In-tree version: still `0.5.4`. Deliberately NOT bumped, and manifests
  (Casks, bucket, winget, AUR) deliberately NOT touched, because bumping
  them would point `brew`/`scoop` users at release assets that do not
  exist yet. Recommended next version when the gates pass: `0.5.5`
  (reliability/restore-safety changes; no user-facing contract change that
  forces a minor). If the authenticated-IPC helper redesign in the
  investigation doc lands first, cut a minor (`0.6.0`) instead.

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
2. Developer ID signing + notarization credentials: AVAILABILITY NOT
   CONFIRMED in this environment. No signing material was accessed, printed,
   or committed. Owner action: confirm the `DEVELOPER_ID_CERTS_P12_BASE64`,
   `CERTS_P12_PASSWORD`, `CI_KEYCHAIN_PASSWORD`, `APPLE_ID`, `TEAM_ID`,
   `APPLE_APP_SPECIFIC_PASSWORD` secrets exist and are valid for the public
   direct-download channel. Unsigned/ad-hoc artifacts may only be staged as
   clearly-identified developer builds, never published to the stable
   channel.
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

- No tag pushed, no release published, no manifest bumped.
- No signed/notarized candidate produced here (credentials unconfirmed;
  none accessed).
- The authenticated-IPC helper redesign, UI state model, post-trial control
  reachability, and updater handover remain open (see the investigation
  doc). These should be weighed before deciding the final version number.
