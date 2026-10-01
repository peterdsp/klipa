# Keep-awake verification record

What has actually been verified, with what, and what remains an open gate.
Results are kept strictly separate from planned-but-unrun tests. Raw
machine diagnostics are deliberately NOT committed here.

Status date: 2026-10-02. Candidate: version `0.6.1` in-tree (workspace
version in `Cargo.toml` + `Cargo.lock`), the keep-awake correctness fixes on
top of the merged 0.6.0 work. The 0.6.0 draft is superseded; see
`docs/keep-awake-release-checklist.md`.

## Automated gates (run on the build host, version 0.6.1)

Host: model Mac17,3, arm64, macOS 27.0 (build 26A428), Rust 1.98.1.

| Gate | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | PASS |
| Build | `cargo build --workspace --locked` | PASS |
| Tests | `cargo test --workspace --locked` | PASS (klipa-ui 49, klipa-helper 23, klipa-ipc 6, klipa-core 3, +doctests) |
| Clippy | `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| MAS+weather tests | `cargo test -p klipa-ui --no-default-features --features "mas weather" --locked` | PASS (29) |
| MAS tests | `cargo test -p klipa-ui --no-default-features --features mas --locked` | PASS (29) |
| No-default tests | `cargo test -p klipa-ui --no-default-features --locked` | PASS (43) |
| Clippy, each MAS combo | `cargo clippy -p klipa-ui --no-default-features [--features ...] --all-targets --locked -- -D warnings` | PASS |

The 0.6.1 fixes added regression tests: `helper.rs` shared-deadline update
(shorten/extend/timed<->indefinite/concurrent ordering) + `restore_confirmed`;
`lid.rs` restoration-failed stays visible after the session ends; `manager.rs`
autonomous retry of an unconfirmed restore, `End` reporting `RestoreUnconfirmed`,
and reassertion of a slipped override; `updater.rs` ordered-transaction
`plan_update` (no swap before candidate+session verified; rollback kept until
the installed app verifies).

Deterministic tests covering the section-7 matrix (injected clocks,
system, and journal seams; no real power state touched, no long sleeps):

- Protocol (`klipa-ipc`): request/response round-trips, oversized messages
  refused (not truncated), a line at the cap rejected on decode, garbage
  and unknown ops are clean decode errors, and extra fields cannot smuggle
  extra behavior.
- Helper state machine (`manager.rs`): begin writes the journal before
  setting; a failed set rolls back and names the reason; an unwritable
  journal refuses to disable sleep; a pre-existing override is never reset;
  end restores and clears only on confirmation; a stale generation cannot
  disturb a newer session; idempotent re-begin; a lapsed lease restores
  autonomously; the daemon owns timed-session expiry; renew extends the
  lease; an unconfirmed restore retains the record; startup reconciles
  interrupted-arming, prior-boot, and still valid sessions; another user
  cannot end an owner's session; tri-state `pmset` parsing.
- Helper auth (`auth.rs`): the pinned requirement includes identifier,
  Apple anchor, and Team ID; no Team ID fails closed.
- Lid coordinator (`lid.rs`): tri-state parse; restore never clears without
  confirmation; prior-value ownership; the 8-state protection classifier
  across the lifecycle.
- Session logic (`awake.rs`): the full existing suite (indefinite has no
  expiry, modes map to the right assertion, every replacement releases
  first, no transient gap on mode change, etc.) plus a live IOKit
  assert/release smoke test.
- Helper client (`helper.rs`): remaining-session countdown math.

### Formatting and lint gate history (resolved)

- `cargo fmt --all -- --check` FAILED on the `368cf36` baseline (rustfmt
  style drift in unrelated files, never run by CI). This was resolved by the
  `cargo fmt --all` normalization commit (`f65b933`) on the way to 0.6.0, so
  the tree is now fully fmt-clean and the gate PASSES; the 0.6.1 fixes keep
  it clean. The earlier note that the tree was deliberately left unformatted
  is superseded.
- `cargo clippy -- -D warnings` FAILED on `368cf36` (three unrelated lints).
  Those were fixed, the lint gate is green, and it is enforced in CI.

## Staged release candidate (0.6.0): signed, notarized, DRAFT only; SUPERSEDED

NOTE: the 0.6.0 draft below is retained as evidence that the signing /
notarization pipeline works end to end, but it is SUPERSEDED by 0.6.1 and
must not be published. Its hashes cover the buggy 0.6.0 build, not 0.6.1; a
fresh signed/notarized 0.6.1 candidate with its own hashes is produced by the
release workflow from the 0.6.1 tag, and this section will be replaced with
the 0.6.1 evidence once that run completes.

Candidate: tag `v0.6.0` on main commit `b5e9a5b` (PR #25 squash-merge).
Release workflow run 36884600655: `verify`, `macos`, `windows`, `linux`,
`mas`, and `release` all succeeded; `managers` and `winget` skipped (they
run only on `release: published`, so brew/scoop/winget were not touched and
`Casks/klipa.rb` + `bucket/klipa.json` remain at 0.5.4).

macOS signing/notarization evidence (from the run and re-verified locally
on macOS 27.0 against the downloaded draft asset):

- `notarytool`: "Current status: Accepted ... Processing complete" for both
  the `.pkg` and the update `.zip`; "The staple and validate action worked!"
- `pkgutil --check-signature klipa-0.6.0-macos.pkg`: "signed by a developer
  certificate issued by Apple for distribution", "Notarization: trusted by
  the Apple notary service", chain "Developer ID Installer: PETROS
  DHESPOLLARI (YTS4KJBX3P)".
- `spctl --assess --type install`: "accepted / source=Notarized Developer
  ID / origin=Developer ID Installer: PETROS DHESPOLLARI (YTS4KJBX3P)".
- Downloaded `.pkg` SHA256 matches `SHA256SUMS.txt`
  (`dab7fda9e51c7578ab34ff5c96a4a9418278a238ce5c400baf00742d8e9c0393`).
- The helper is signed with the hardened runtime inside-out before the app;
  the Team ID (`YTS4KJBX3P`) is baked into the helper for the caller check.

App Store: `mas` job validated and uploaded `klipa-0.6.0-mas.pkg` to App
Store Connect (Delivery UUID `6e01e24a-2507-4354-a0b7-4de59cf2db30`, "No
errors uploading"). That is an upload to processing, NOT public store
availability; review/release remain manual in App Store Connect.

Release state: the GitHub Release for `v0.6.0` is a DRAFT. The public
"Latest" release is still v0.5.4. Nothing was published, no package-manager
manifest was changed, and the website was not touched. This is the holding
state until the physical gate passes and the owner gives the go.

Draft asset checksums (`SHA256SUMS.txt`):

```
944d5fdda88429d7a353633b6c59362a13222db2b546d103264c1178c6251015  klipa-0.6.0-1.x86_64.rpm
13901e1e1518f56f51107a33349ecd4a6d8e1ed7a78dce9753c7ac32c6f8ee08  klipa-0.6.0-linux-x86_64.tar.gz
dab7fda9e51c7578ab34ff5c96a4a9418278a238ce5c400baf00742d8e9c0393  klipa-0.6.0-macos.pkg
586e9031b7452cd6929eeffd4b419daad36b5cf8d679b68a5e3c060a9868f4bf  klipa-0.6.0-macos.zip
c8f5872f7aac8ce86902e9c1cbc156b8921823d3e310eb138cec9aba3a50c2d4  klipa-0.6.0-windows-x64-setup.exe
871755b34988e4fd10b13fbc8361014b8ec0ed691e39db706bbd81d9c8ede15e  klipa-0.6.0-windows-x64.zip
e02bbd17dc9edcb26934ce0126c1c23429a42f8be140dc38177e425989eeb93f  klipa-0.6.0-x86_64.AppImage
1ad7a7f2e3dff960ef820e91229b88cc05a6073fe81cdfde7f88983c32133e88  klipa_0.6.0_amd64.deb
```

## Physical closed-lid gate: NOT SATISFIED (blocked on owner hardware)

None of the closed-lid behavior has been physically verified. The brief is
explicit that a VM, an injected lid event, a CI runner, a created
assertion, a screenshot, or `SleepDisabled=1` alone do NOT satisfy this
gate. It requires the owner's actual failing MacBook doing real lid-close
cycles with the packaged, signed candidate, and cannot be performed in this
environment.

The following cases are DEFINED and UNRUN. Each needs: exact hardware,
macOS build, artifact hash, helper version, starting `pmset -g` settings,
elapsed time, a local heartbeat/work recorder that takes no power
assertions, sleep/wake + lid logs, and a control run proving the machine
DOES sleep under the same idle conditions with klipa inactive.

| Case | Required evidence | Status |
|---|---|---|
| Open lid, short idle timeout, each ordinary mode | prevents the intended idle sleep past the baseline timeout | UNRUN |
| Display sleep + lock screen (system-only) | work continues screen-off; lock/unlock still works | UNRUN |
| Closed lid, no external display, AC | >= 30 min continuous work, no unintended system sleep | UNRUN |
| Closed lid, no external display, battery | >= 30 min where advertised, enough starting charge | UNRUN |
| Charger connect/disconnect while shut | >= 5 transitions each direction, no unintended sleep / misleading active state | UNRUN (highest-risk case per investigation) |
| Dock / external-display changes | protection + status reconcile | UNRUN |
| Repeated lid cycles | >= 10 open/close cycles, no loss of protection | UNRUN |
| Short timed session, lid shut | ends unattended; owned settings restore within stated tolerance | UNRUN |
| Timed session, UI occupied | open menu / modal cannot delay helper expiry | UNRUN on hardware (helper-owned expiry now implemented + unit-tested; needs physical confirmation) |
| Indefinite session | >= 2 h soak, no invented expiry, no accumulating assertions/processes | UNRUN |
| Duration/mode change while shut | no intermediate sleep gap; deadlines do not reset | UNRUN |
| Normal quit and force-quit | helper restores without relaunching klipa | UNRUN |
| Helper crash/restart + failed restore | recovery survives; UI does not declare premature success | UNRUN |
| Reboot/logout + approval changes | no stuck override; no silently resurrected session | UNRUN |
| Real update from the installed old version | new bundle/helper run; settings/history/license survive; no stranded override | UNRUN |
| Stop / restore / helper uninstall | original values + normal baseline sleep return; no orphan service/endpoint | UNRUN |

## Short test handoff for the owner

To run the physical gate on the failing Mac:

1. Record the environment first: `pmset -g`, `pmset -g custom`,
   `pmset -g assertions`, model, macOS build, charger/display/lid state.
   Keep these out of the repo.
2. Establish the control: with klipa NOT running, confirm the Mac sleeps on
   closed lid / idle under the same conditions. Without this control a
   "stayed awake" result proves nothing.
3. Install the packaged candidate (same signatures/paths users get), start
   a lid-closed session, and verify `pmset -g` shows `SleepDisabled 1` AND
   a local no-assertion heartbeat keeps advancing with the lid shut for the
   required duration. Watch the charger-transition case specifically.
4. For the timed case, confirm it ends unattended and `SleepDisabled`
   returns to `0` within tolerance, then that the Mac can sleep again.
5. Report model, macOS build, artifact SHA256, elapsed time, gaps vs the
   control, and pass/fail per case. Record dark-wake periods distinctly;
   intermittent wake is not uninterrupted work.

Until these are recorded as PASS on the real hardware, the closed-lid
objective is explicitly INCOMPLETE and must not be described as fixed.
