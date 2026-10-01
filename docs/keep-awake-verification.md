# Keep-awake verification record

What has actually been verified, with what, and what remains an open gate.
Results are kept strictly separate from planned-but-unrun tests. Raw
machine diagnostics are deliberately NOT committed here.

Status date: 2026-10-01. Candidate commit: the keep-awake reliability
changes on top of `368cf36`. Version unchanged at 0.5.4 in-tree (no release
cut; see `docs/keep-awake-release-checklist.md`).

## Automated gates (run on the build host)

Host: model Mac17,3, arm64, macOS 27.0 (build 26A428), Rust 1.98.1.

| Gate | Command | Result |
|---|---|---|
| Build | `cargo build --workspace --all-targets --locked` | PASS |
| Tests | `cargo test --workspace --locked` | PASS (klipa-ui 42, klipa-helper 2, klipa-core 3, +doctests) |
| Clippy | `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| MAS+weather tests | `cargo test -p klipa-ui --no-default-features --features "mas weather" --locked` | PASS (34) |
| MAS tests | `cargo test -p klipa-ui --no-default-features --features mas --locked` | PASS (34) |
| No-default tests | `cargo test -p klipa-ui --no-default-features --locked` | PASS (36) |
| Clippy, each MAS combo | `cargo clippy -p klipa-ui --no-default-features [--features ...] --all-targets --locked -- -D warnings` | PASS |

New deterministic tests added this pass (injected inputs, no real power
state touched, no long sleeps):

- `parse_sleep_flag` reads the three states and treats a missing or
  unrecognized field as `Unknown`, never silently `Enabled`.
- `restore_step`: a pre-existing override klipa did not set is never reset;
  the recovery journal is cleared only on a confirmed `Enabled` readback;
  an unconfirmed or `Unknown` restore retains the journal.
- `prior_to_record`: an existing journal keeps ownership across a re-engage
  (mode/duration change); a fresh engage records the observed prior, with
  an unreadable prior left ambiguous.
- Helper `classify`: only `set 1` / `set 0` / `ping` are accepted;
  everything else (including shell-injection-looking and over-long input)
  is rejected.

### Pre-existing gate state (clearly separated from this pass)

- `cargo fmt --all -- --check` FAILS on `368cf36` and still reports drift in
  unrelated files (CI never ran fmt; this is rustfmt style drift, not a
  code defect). This pass did NOT mass-reformat the tree to keep the diff
  minimal; only `awake.rs` (the core file edited here) was left fully
  fmt-clean, and all newly added code is fmt-clean. To green this gate the
  owner can run a single `cargo fmt --all` normalization commit. Not done
  here by design.
- `cargo clippy -- -D warnings` FAILED on `368cf36` (three unrelated lints).
  Those are now fixed, so the lint gate is green and has been added to CI.

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
| Timed session, UI occupied | open menu / modal cannot delay helper expiry | UNRUN (depends on helper-owned expiry, not yet implemented) |
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
