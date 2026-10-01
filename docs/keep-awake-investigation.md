# Keep-awake investigation: causes, mechanism, and decisions

Durable engineering record for the keep-awake reliability work, so the
investigation does not have to be repeated. It covers what was confirmed
in source, what the mechanism actually is, what was changed, and what
remains genuinely unproven because it needs physical hardware.

Status date: 2026-10-01. Baseline commit at the start of this pass:
`368cf36` (workspace version 0.5.4).

## Scope and honest framing

The reported failure is: a lid-closed keep-awake session does not reliably
keep the Mac running with the lid physically shut. This document records
causes that are demonstrable from the source and from read-only
diagnostics. It does NOT claim the closed-lid behavior is fixed on the
owner's failing Mac: that claim requires the physical closed-lid gate in
`docs/keep-awake-verification.md`, which needs the owner's hardware and
cannot be satisfied in this environment. See "What is not proven" below.

## Mechanism: what actually controls lid-closed sleep

Confirmed against the existing code, Apple documentation, and the behavior
every comparable tool relies on:

- An IOKit power assertion (`IOPMAssertionCreateWithName` with
  `PreventUserIdleDisplaySleep` or `PreventUserIdleSystemSleep`, the API
  `caffeinate` wraps) suppresses the idle-timeout sleep path only. Closing
  the lid is an explicit sleep request, not an idle timeout, so no public
  assertion overrides it. `PreventSystemSleep` is for holding the system
  through a specific operation (it does not defeat a lid close either) and
  is not a closed-lid solution.
- Apple's supported clamshell mode keeps a MacBook running with the lid
  shut only with an external display attached (plus AC on Intel; Apple
  Silicon can run clamshell on battery). The support note that documents
  the M3 closed-lid dual-display setup includes power; battery clamshell
  support is not implied by architecture alone. `clamshell.rs` already
  reads exactly display + power + architecture and reports this honestly.
- The one native lever that changes lid-close behavior beyond clamshell is
  the system `disablesleep` power setting (`pmset -a disablesleep 1`),
  which needs root. This is what Amphetamine, KeepingYouAwake's fork
  family, InsomniaX, etc. reach underneath. It is undocumented and is a
  real privilege escalation, not a trick.

Important caveat captured from the Amphetamine Power Protect project: on
Apple Silicon, closed-display operation is specifically fragile across
connecting/disconnecting external power. A one-time successful
`disablesleep=1` write is not evidence that the machine stays awake across
a later power-source transition with the lid shut. This is the single most
likely explanation for "it still fails in real use" and is exactly what the
physical gate's charger-transition cases exist to measure. It is NOT fixed
by writing the flag more often.

Read-only baseline captured on the build machine (model Mac17,3, arm64,
macOS 27.0 build 26A428) on 2026-10-01: `SleepDisabled 0` at rest; a
normal `PreventUserIdleSystemSleep` assertion from `powerd` is present when
the display is on. This machine is the development/build host; it is NOT
confirmed to be the reported failing device, and the app installed on any
given machine is not assumed to be built from this checkout.

## Concrete source findings (reconfirmed against `368cf36`)

The brief listed thirteen observations. Reconfirmed in the current
checkout, with the disposition of each in this pass:

1. `WakeLock::finished()` defaults to `false` on macOS; there is no
   assertion-health or override-health reconciliation after engagement, and
   a `Session` object is treated as proof of protection. CONFIRMED. Partly
   addressed: the override now has a verified, prior-value-aware restore
   path (below); a full live assertion/override health-reconcile loop and a
   verified-state field in the view remain open (see "Remaining work").
2. `sleep_currently_disabled()` collapsed command error, unreadable output,
   and a real `false` into one boolean, so "unknown" could be mistaken for
   "restored". CONFIRMED and FIXED: replaced by `read_sleep_flag()`
   returning a tri-state `SleepFlag { Disabled, Enabled, Unknown }` that
   requires a successful `pmset` exit and an explicitly parsed value. Pure
   parser `parse_sleep_flag` is unit-tested against real and malformed
   samples.
3. `Lock::drop()` and `recover_lid_closed()` ignored restoration failures
   and cleared the marker anyway, losing the evidence needed to retry.
   CONFIRMED and FIXED: restoration now reconciles against a verified
   readback and only clears the recovery record on a confirmed return to
   `Enabled`; a failed or unknown restore RETAINS the record so the next
   launch retries. Pure decision `restore_step` is unit-tested.
4. The override always wrote `disablesleep` back to zero rather than the
   observed prior value, and the marker was a write-ignoring boolean.
   CONFIRMED and FIXED: a JSON recovery journal records the prior value
   observed before klipa changed it; restore returns to that value and
   never blindly resets a pre-existing `disablesleep=1` that klipa did not
   set. The journal is written BEFORE the flag is mutated, and engage
   refuses if the durable record cannot be written.
5. The root helper accepts unauthenticated commands on a mode-0666 socket,
   one connection at a time, with an unbounded `read_line` and no server
   read deadline; no session ownership, lease, journal, or autonomous
   recovery. CONFIRMED. Partly addressed: the helper now bounds each
   request in size (256 bytes) and time (5s read/write deadline), and its
   command parsing is factored into a unit-tested whitelist, so a stalled
   or flooding client can no longer wedge the accept loop or smuggle a
   truncated command. NOT yet addressed: caller authentication (audit
   token / code-signature / Team ID), session ownership and lease, and a
   root-owned durable journal inside the daemon. See "Remaining work": this
   is the large XPC-grade redesign the brief describes and is not completed
   in this pass.
6. Helper `Active` is derived from `SMAppService` registration, not from
   authenticated liveness or a verified operation. CONFIRMED, NOT changed
   in this pass (depends on the authenticated-IPC redesign in 5).
7. `KeepAwake::start()` calls `end()` before acquiring the replacement, so
   a duration/mode change briefly drops protection. CONFIRMED, NOT changed.
   For the non-privileged assertion modes this is a sub-millisecond gap and
   is covered by tests asserting exactly one assertion at a time; for the
   lid-closed override the new journal preserves ownership across a
   re-engage (`prior_to_record` keeps an existing journal's prior value),
   but a demonstrably gap-free transaction for the override itself is still
   open.
8. Privileged subprocesses, verification polling, and `Drop` release run on
   the menu event-loop thread; expiry depends on that loop. CONFIRMED, NOT
   changed. Moving blocking IPC/subprocess/poll off the main thread remains
   open and is called out as a correctness item for the helper redesign.
9. The updater swaps the bundle and calls `process::exit(0)` from a
   background thread, bypassing destructors and the `awake.end()` path.
   CONFIRMED, NOT changed. The on-disk recovery journal now at least makes
   a bypassed restore recoverable on next launch, but the explicit
   update/handover sequence in the brief's section 8 is not implemented.
10. `clamshell.rs` infers laptop/desktop from the active built-in display
    and assumes Apple Silicon + external display implies battery clamshell
    readiness. CONFIRMED as a known modeling limitation; the module already
    presents clamshell as a prediction and distinguishes power source. A
    fuller "uncertain" presentation is open.
11. The tray labels an override rejection as a managed-power-policy issue.
    PARTIALLY addressed in prior code via `LidBlock::{Declined, Refused,
    Unavailable}`; the `Refused` path is now only reached after a tri-state
    readback fails to confirm the change, which is more accurate than
    before, though "refused" still cannot positively prove MDM.
12. The trial-expired menu omits keep-awake controls. CONFIRMED present in
    `tray.rs` (locked gate shows only the paywall). NOT changed in this
    pass; flagged as a real gap: an active session or a pending restoration
    can become unreachable from the menu when the license locks. See
    "Remaining work".
13. The release workflow permitted unsigned public artifacts, swallowed the
    Gatekeeper check with `|| true`, and did not gate publish on CI for the
    exact commit. CONFIRMED and FIXED: see "Release workflow" below.

## What changed in this pass (all verifiable without hardware)

- Tri-state `disablesleep` read (`SleepFlag`) with a pure, unit-tested
  parser; no more "unknown == restored".
- A durable, prior-value-aware recovery journal (`paths::lid_awake_journal`,
  `lid_awake.json`): schema, the prior `disablesleep` value, and the boot
  session id. Written before mutation; engage refuses if it cannot be
  written; removed only after a verified restore. Legacy `lid_awake.on`
  markers are migrated as ambiguous ownership (no fabricated prior value).
- Verified restoration in both `Lock::drop` and `recover_lid_closed`, with
  the record retained on any unconfirmed restore so a stuck override is
  never silently forgotten. Pre-existing overrides klipa did not set are
  left alone instead of being reset to zero.
- Helper hardening: bounded request size and read/write deadlines, plus a
  unit-tested command whitelist (`classify`).
- Release/CI workflow hardening (below).
- Three pre-existing `clippy -D warnings` failures fixed so the lint gate
  can run clean (unrelated to keep-awake: `search.rs` derive/sort,
  `tray.rs` type alias + arg-count allow, `license.rs` item ordering).

## Mechanism decision record

Candidate approaches evaluated:

- Public idle assertions only: insufficient for closed lid by construction
  (does not defeat an explicit lid-close sleep). Kept as the always-on
  baseline for the two non-lid modes and the App Store build.
- Clamshell (external display + power): supported and already detected and
  reported honestly; not a substitute for lid-closed-without-monitor.
- Privileged `disablesleep` via a root helper (current approach): the only
  mechanism that changes lid-close behavior without an external display,
  runs under SIP/Gatekeeper/normal boot security, and does not make users
  install a competing utility. Chosen. Its residual risks (undocumented
  behavior, Apple Silicon power-transition fragility, restore safety) are
  the focus of the changes above and the physical gate.

Decision: keep the privileged-`disablesleep` mechanism, but make its
restore safe, prior-value-aware, verified, and durably recoverable, and
keep the override out of the sandboxed App Store binary. No global security
change, no kernel patching, no SIP/Gatekeeper weakening.

## What is NOT proven (requires the owner's hardware)

- That `disablesleep=1` keeps the owner's specific failing Mac awake with
  the lid shut and no external display, for the required durations.
- That protection survives the charger connect/disconnect transition class
  on that Apple Silicon machine (the most likely real-world failure).
- That timed closed-lid sessions end unattended and restore within
  tolerance on that machine.

These cannot be simulated. `SleepDisabled=1`, a created assertion, a VM, or
a CI run do NOT satisfy them. The required evidence and exact steps are in
`docs/keep-awake-verification.md`.

## Update 2026-10-01: deep redesign completed (version 0.6.0)

The section-4 items deferred in the first pass are now implemented, on the
`keep-awake-helper-ownership` branch, targeting version 0.6.0:

- **Shared versioned protocol** (`crates/klipa-ipc`): a closed set of typed
  operations (hello/status/begin/renew/end), effective-state and error
  enums, bounded newline-framed JSON. No shell, path, settings key, or env
  can be expressed on the wire. (6 unit tests.)
- **Authenticated helper IPC** (`crates/klipa-helper/src/auth.rs`): every
  connection is validated by the peer's kernel audit token resolved to a
  `SecCode` and checked against a pinned designated requirement (klipa
  identifier + Apple anchor + Team ID baked at build time from the signing
  identity). Fails closed when unsigned. The requirement builder is unit
  tested; the live `SecCode` path is device-gated FFI (see below).
- **Daemon ownership + lease + journal** (`crates/klipa-helper/src/manager.rs`):
  one component owns the `disablesleep` transaction. A monotonic generation
  stops stale replies; a renewable crash-safety lease plus a daemon-owned
  session deadline mean a dead or frozen UI cannot strand the override and
  the daemon (not the UI) owns timed expiry; a root-owned journal (schema,
  boot id, generation, owner uid, prior value, phase, deadlines) is written
  before the flag changes and cleared only after a verified restore; startup
  reconciles interrupted, prior-boot, and still valid sessions before
  accepting work; another user cannot end an owner's session. It never
  relies on Rust `Drop`. (20 unit tests over injected system/journal seams.)
- **Verified UI state model** (`ProtectionState` in `awake.rs`, computed in
  `lid.rs`): inactive / preparing / awaiting approval / verifying / active /
  degraded-recovering / restoring / restoration-failed, driven by the real
  effective readback from the helper, not in-memory session presence.
- **Gap-free transitions + off-thread work**: the override lives in the
  `lid.rs` coordinator driven by explicit session edges from the
  composition root, not in the assertion `Lock`/`Drop`. A duration change
  updates the override in place; the lease renewer runs on its own thread,
  not the menu loop. (Findings 7, 8.)
- **Trial-lock controls** (finding 12): the paywall menu keeps an
  always-reachable End keep-awake / restore and Copy diagnostics.
- **Updater handover** (finding 9): `relaunch()` reconciles owned power
  (ends the override, restoring sleep) before the detached `process::exit`.

### Still not proven / genuinely remaining

- The physical closed-lid gate (owner hardware). Unchanged: see
  `docs/keep-awake-verification.md`.
- The live `SecCode` audit-token validation, the real root daemon behavior
  under launchd, and the socket round-trips are COMPILE-verified and their
  pure logic is unit-tested, but are not runtime-verified here (they need
  the signed app + root daemon on a real Mac). Labeled device-gated.
- macOS 11/12 weak-linking: `SMAppService` is 13+; the helper path is gated
  on `>= 13` and 11/12 falls back to the admin prompt, but that the signed
  binary actually loads on 11/12 (weak-link/deployment target) is not
  verified without an 11/12 machine. Deployment target and `LSMinimumSystemVersion`
  should be confirmed there before claiming 11/12 support for ordinary
  keep-awake.
- Signed/notarized artifact: produced only by the release workflow (the
  secrets exist); staged as a draft, not published, pending the physical
  gate and owner go.

## Update 2026-10-02: 0.6.0 candidate reopened; 0.6.1 correctness fixes

The 0.6.0 draft candidate was re-reviewed against the executable code (not
its own documentation). Four real implementation gaps were found and fixed,
each with a regression test. Because these change the candidate, the version
is advanced to 0.6.1; the 0.6.0 draft and its signing evidence do NOT cover
this build.

1. **Stale renewal clobbered an in-place duration change** (`helper.rs`).
   The lease renewer thread captured the session deadline it started with,
   while `update_session` mutated a different copy, so ~20 s after a duration
   change the renewer overwrote the daemon's deadline with the old value.
   Fixed with a single authoritative `SharedDeadline` (an `Arc<Mutex>`) read
   fresh on every renewal and written by `update_session`. Tests cover
   shorten, extend, timed->indefinite, indefinite->timed, and that the
   renewer's own clone sees an update (concurrent ordering).

2. **End cleared the error without a verified restore** (`lid.rs`).
   `end()` set `last_error = None` and dropped the mechanism regardless of
   whether sleep was actually restored, and `helper::disengage()` returned
   nothing to check. Now `disengage()` returns Confirmed / Unconfirmed /
   Unreachable (from the daemon's `End` effective state), `end()` keeps a
   restoration-failed error until a readback confirms, and `classify()`
   surfaces `RestorationFailed` even after the session is no longer desired,
   so the state and its retry control stay visible. The menu's End action is
   relabelled "Restore normal sleep" and stays reachable in that state.

3. **A failed restore was stranded until a restart** (`manager.rs`). An
   unconfirmed restore set `active = false`, and `tick()` skipped inactive
   sessions, so the owed restore was never retried while the daemon ran. Now
   an owed restore is recorded with a bounded backoff and `tick()` retries it
   autonomously until confirmed; `End` reports `RestoreUnconfirmed` instead
   of a false success, and an explicit client retry also re-runs it. Tested
   as transient failure then recovery with no app/daemon restart.

4. **Updater replaced the bundle before coordinating the session**
   (`updater.rs`). The bundle swap (including deleting the old bundle) ran
   before `relaunch()` attempted any power-state cleanup, so the active
   session was not stopped before the helper-containing bundle was replaced,
   and the rollback was discarded before the new app/helper was verified.
   Replaced with an explicit ordered transaction (`apply_update`): verify the
   candidate's code identity, perform a VERIFIED stop of the session
   (`lid::end_verified`) BEFORE any swap, replace the bundle keeping the old
   one as rollback, verify the installed app/helper at its final path, and
   only then drop the rollback; a failed restore aborts before the swap and a
   failed install verification rolls back. The ordering is unit-tested via a
   pure `plan_update`.

Additional hardening from the section-5 review (charger/power transitions):
`tick()` now reasserts an owned override that has slipped off (flag read back
enabled while a session is active), so a power-source transition that clears
`disablesleep` is repaired within the monitor interval rather than silently
losing protection. This is defense in depth; the charger-transition case
still requires the physical gate to confirm end to end.

### Reviewed section items with honest limitations (not silently "done")

- **Clock**: the daemon uses the wall-clock epoch for lease/session
  deadlines on purpose, so they survive a daemon restart (a process-local
  monotonic clock would not), discriminating boots by `kern.bootsessionuuid`.
  A backward wall-clock adjustment can extend a lease, but the lease is a
  short (60 s) crash-safety bound the client renews, so the exposure is
  bounded. Deadline math is checked (`saturating_add`/`saturating_sub`).
- **Synchronous UI**: expiry and lease renewal are off the menu thread (the
  daemon owns expiry; the renewer runs on its own thread). Start/stop are
  single bounded calls (5 s IO timeout). The admin-prompt fallback's
  readback and the diagnostics round-trip are bounded and user-initiated.
- **Helper readiness vs registration**: the lid protection state is driven
  by verified effective readback from the daemon, not registration. The
  "Passwordless mode: on" label is still derived from `SMAppService`
  registration status; it reflects approval, not a live round-trip, which is
  an acceptable, low-stakes nuance documented here rather than hidden.
- **Admin-prompt unattended expiry (limitation)**: a TIMED closed-lid
  session restored without the passwordless helper would need a fresh admin
  prompt at expiry, which cannot happen behind a shut lid; the app-side
  journal then restores on the next launch. Unattended timed closed-lid
  expiry is therefore only reliable with the helper approved. This is a
  limitation to disclose, not a solved case, and the helper is the fix.
