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

## Remaining work (scoped, not done in this pass)

Honestly out of scope for what was completed here, and required before the
closed-lid objective can be called fully fixed:

- Authenticated, bounded, versioned helper IPC (audit token / code
  signature / Team ID), session ownership + renewable lease, and a
  root-owned durable journal inside the daemon, with launchd-based
  autonomous recovery. (Brief sections 4 and parts of 5/6.)
- A richer session state model in the UI (preparing / awaiting approval /
  verifying / active / degraded / restoring / restoration-failed) driven by
  a verified effective-state read, not an in-memory session. (Brief 3, 6.)
- Keep-awake + restore controls reachable after trial lock. (Finding 12.)
- Gap-free override transaction on mode/duration change, and moving
  blocking IPC/subprocess/`Drop` restore off the event-loop thread.
  (Findings 7, 8.)
- Explicit updater handover sequence that reconciles owned power state
  before bundle replacement/relaunch. (Finding 9, brief 8.)
