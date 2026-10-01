# Keep awake: modes, indefinite sessions, and the lid

How klipa keeps a machine running, what "indefinitely" actually means
here, and why closing a MacBook lid is a different problem from every
other kind of sleep. This documents the feature implemented in
[`crates/klipa-ui/src/awake.rs`](../crates/klipa-ui/src/awake.rs).

## The three modes

Keeping a Mac awake is not one behavior, it is three, and conflating them
is how apps end up burning a display for a download. `AwakeMode` makes
them exclusive and names each one:

| Mode | Intent | macOS assertion type | Windows flags |
|---|---|---|---|
| `ScreenAndSystem` | keep the screen lit and the machine awake | `kIOPMAssertionTypePreventUserIdleDisplaySleep` | `ES_SYSTEM_REQUIRED \| ES_DISPLAY_REQUIRED` |
| `SystemOnly` | keep the machine awake, let the screen turn off | `kIOPMAssertionTypePreventUserIdleSystemSleep` | `ES_SYSTEM_REQUIRED` |
| `LidClosed` | keep running with the lid physically shut | `...PreventUserIdleSystemSleep` **plus** the system `disablesleep` flag | n/a |

Two rules fall out of this table:

- **One assertion per mode, never a blend.** `PreventUserIdleDisplaySleep`
  already implies the system stays awake, so `ScreenAndSystem` does *not*
  also take a system assertion; that would be a second assertion id to
  track for no extra behavior.
- **`LidClosed` lets the display sleep on purpose.** The panel is off
  behind a shut lid, so holding a display assertion there would only cost
  power in a machine that already cannot shed heat.

On Linux `systemd-inhibit --what=idle` blocks the entire idle path
(screen blank and auto-suspend together), so the first two modes are
indistinguishable there and `LidClosed` is not offered at all.

## "Indefinitely" means no timer

`AwakeDuration` has exactly two cases:

```rust
pub enum AwakeDuration {
    Indefinite,        // no duration inside it at all
    For(Duration),
}
```

`Indefinite` is deliberately not `For(some enormous duration)`. There is
no `Duration::MAX`, no hundred years, no `u64::MAX` seconds, and no timer
standing in for forever. The only place a deadline is ever computed is
`AwakeDuration::deadline`, which returns `None` for `Indefinite`, so an
indefinite session stores no deadline and `KeepAwake::poll` has nothing
to compare against. It ends when:

- the user picks **End current session**, or switches to a timed one,
- klipa quits (the assertion is released on the way out),
- the machine shuts down or restarts,
- or macOS invalidates the assertion.

The menu reflects that: an indefinite session reads `Awake indefinitely`
with no countdown, and `main.rs` skips the once-a-minute label refresh
for it entirely, because there is nothing to refresh. The custom-length
prompt caps its input at one year precisely so it can never become a back
door to a fake-infinite timer; asking for no limit is the `Indefinitely`
item's job.

## One assertion, one owner

`KeepAwake` holds at most one `Box<dyn WakeLock>`. That box *is* the
assertion: creating it is the only way to acquire one, dropping it is the
only way to release one, and there is exactly one slot for it. So:

- starting a session while one runs calls `end()` first, which drops the
  old lock and releases the old assertion before the new one is requested,
- a failed start leaves no session, no deadline, and no lock, and records
  the reason for the menu,
- quitting, expiring, and switching modes all funnel through the same
  drop.

Duplicate assertions and orphaned assertion ids are therefore not
representable rather than merely avoided. Every IOKit return code is
checked (`IOPMAssertionCreateWithName` and `IOPMAssertionRelease` both),
nothing is force-unwrapped, and a failure is logged and surfaced.

`PowerSource` is the single seam onto the OS power APIs, which is what
lets the session rules above be tested against a recording fake instead
of a real Mac's power state. See the tests at the bottom of `awake.rs`.

## Preferences persist, sessions do not

`settings.json` stores the selected `awake_mode` and the last custom
length. It does **not** store "a session was running". A power assertion
belongs to the process that created it and dies with it, so reconstructing
one at launch from a saved flag would be inventing a session the user
never started, and on the lid-closed path it would silently re-acquire
root. On relaunch klipa restores the preference and waits. (A saved
`LidClosed` preference loaded by a build that cannot honor it falls back
to the default rather than showing an inert selection.)

## The core problem with the lid

klipa's normal keep-awake holds an IOKit power assertion
(`IOPMAssertionCreateWithName`), the same public API that `caffeinate`
wraps. That assertion suppresses the **idle** sleep path: the timeout
that fires when nobody touches the machine.

Closing the lid is a different thing. It is not an idle timeout, it is an
**explicit** sleep request, and macOS honors it regardless of any power
assertion. This is why `caffeinate`, `ProcessInfo.beginActivity`,
`PreventUserIdleSystemSleep`, KeepingYouAwake, and klipa's plain
keep-awake all hold a Mac awake on an open desk but let it drop the
moment you fold it shut.

**Verified conclusion: no public, non-privileged API overrides the lid
switch, on Intel or Apple Silicon, on any current macOS.** The behavior
that *is* supported and documented is Apple's clamshell mode, and it
depends on the hardware and power situation:

| Configuration | Lid closed, no privileged change | Notes |
|---|---|---|
| MacBook, no external display | **sleeps** | on AC or battery, Intel or Apple Silicon; an assertion does not change this |
| MacBook + external display, Apple Silicon | **keeps running** | clamshell works on battery too |
| MacBook + external display, Intel, on AC | **keeps running** | Apple's documented clamshell requirement |
| MacBook + external display, Intel, on battery | **sleeps** | connect power to get clamshell |
| Desktop Mac | n/a | no lid to reason about |

Historically Apple's clamshell instructions also mentioned an external
keyboard or mouse; on current macOS an attached external display and
power are what the behavior actually tracks, which is why klipa detects
exactly those two and does not claim anything about input devices.

klipa encodes this table in
[`clamshell.rs`](../crates/klipa-ui/src/clamshell.rs) and shows the result
as one line in the Keep-awake submenu, in **every** build. That is the
honest answer to "will closing the lid work for me right now?".

The only native lever that changes lid-close behavior beyond that is the
system power setting `disablesleep`:

```bash
sudo pmset -a disablesleep 1   # stop sleeping, even with the lid closed
sudo pmset -a disablesleep 0   # restore normal behavior
```

Every third-party app that genuinely keeps a Mac awake with the lid
closed (Amphetamine, KeepAwake, AwakeToggle, InsomniaX) reaches this same
setting underneath. It works on Intel and Apple Silicon in practice, is
undocumented, and **requires root**. It is a real privilege escalation,
not a trick, which is why klipa puts it behind the OS's own admin
authentication and offers it only where the sandbox permits it.

## Why this is direct-download only

`pmset disablesleep` needs root, and klipa ships in two macOS variants
with different entitlements:

| Capability                         | Direct download (Developer ID, not sandboxed) | Mac App Store (`mas` feature, sandboxed) |
| ---------------------------------- | :--------------------------------------------: | :--------------------------------------: |
| IOPMAssertion idle keep-awake      | yes                                            | yes                                      |
| Run `pmset` / shell out            | yes                                            | no (sandbox blocks it)                   |
| Trigger the system admin prompt    | yes                                            | no                                       |
| Lid-closed keep-awake, net result  | **shippable**                                  | **impossible**                           |

So the feature is gated to the non-App-Store build. In code this is:

```rust
pub const LID_CLOSED_SUPPORTED: bool = !cfg!(feature = "mas");
```

In the sandboxed (`mas`) build the whole lid-closed path compiles to dead
runtime branches, and the tray never shows the toggle. `KeepAwake::view`
exposes `lid_closed_supported` so the menu hides a control that would do
nothing.

## How klipa acquires root

There are two paths, and klipa ships both. The app always tries the
passwordless helper (Option B) first and falls back to the admin prompt
(Option A) when the helper is not set up. So the feature works out of the
box, and gets smoother if the user opts into the helper.

### Option A: admin prompt (always available)

klipa asks macOS to run `pmset` with administrator privileges through the
OS's own authentication dialog.

```rust
let script = format!(
    "do shell script \"/usr/bin/pmset -a disablesleep {val}\" \
     with administrator privileges"
);
Command::new("/usr/bin/osascript").arg("-e").arg(&script).status()
```

The user authenticates to the system, not to klipa. klipa never sees or
handles the password. There is no persistent component. The trade-off is
a password prompt each time a lid-closed session starts, and one more
when it ends.

### Option B: passwordless root helper (opt-in)

A small root daemon, `klipa-helper`, registered through `SMAppService`
(the modern replacement for `SMJobBless`). After a one-time setup the user
approves in System Settings > Login Items (no admin password), the app
toggles `disablesleep` with **no prompt at all**, because the root work
happens in the daemon.

- **Registration** ([`helper.rs`](../crates/klipa-ui/src/helper.rs)) uses
  the `smappservice-rs` wrapper over `SMAppService`:
  `AppService::new(ServiceType::Daemon { plist_name })` with `.register()`,
  `.status()`, `.unregister()`. The tray shows one of "Enable passwordless
  mode", "Approve ... in Settings", or "Turn off passwordless mode"
  depending on `status()`.
- **The daemon** ([`crates/klipa-helper`](../crates/klipa-helper)) runs as
  root under launchd (`RunAtLoad` + `KeepAlive`), listens on a fixed Unix
  socket (`/var/run/dev.peterdsp.klipa.helper.sock`), and speaks the
  versioned, structured protocol in [`crates/klipa-ipc`](../crates/klipa-ipc):
  a closed set of typed operations (`hello` / `status` / `begin` / `renew`
  / `end`), never a shell command, path, settings key, or environment, so a
  compromised app can only ask for one of those operations. Each request is
  bounded in size and time, so a stalled or flooding client cannot wedge
  the accept loop or exhaust memory. Crucially, the daemon (not the UI)
  **owns the override**: it writes a root-owned recovery journal before it
  changes the flag, verifies every restore, keeps the record until a
  restore is confirmed, and reconciles any leftover at startup. It holds the
  override under a renewable crash-safety **lease** plus a **session
  deadline** it owns, so a dead or frozen app releases the override on its
  own and timed sessions end unattended regardless of the UI. It never
  depends on Rust `Drop` (which cannot run after `SIGKILL` or power loss).
  See `manager.rs`.
- **Authenticated callers.** The socket is world-connectable, but that is
  not the trust boundary: every connection is validated by the peer's
  kernel audit token resolved to a `SecCode` and checked against a pinned
  designated requirement (klipa's identifier, an Apple anchor, and klipa's
  Team ID, baked in at build time from the signing identity). A process
  that is not klipa, or not signed by klipa's team, is rejected before any
  power operation. In an unsigned build no Team ID is baked, so the daemon
  fails closed and the app uses the admin-prompt path. See `auth.rs`.
- **Embedded `Info.plist` (required).** `SMAppService` only registers a
  daemon whose helper executable carries a bundle identifier, which for a
  plain (non-Xcode) binary means an `Info.plist` in the Mach-O
  `__TEXT,__info_plist` section. A stock Rust build emits none, so
  registration silently fails and no daemon is ever created (the app then
  falls back to Option A). [`crates/klipa-helper/build.rs`](../crates/klipa-helper/build.rs)
  generates a minimal `Info.plist` (bundle id `dev.peterdsp.klipa.helper`,
  version tracking the crate) and injects it with
  `-Wl,-sectcreate,__TEXT,__info_plist`. This also makes `codesign` stamp
  the binary with that identifier instead of the filename, matching the
  daemon's launchd `Label`. This was missing in 0.5.0 (passwordless mode
  never registered on device); fixed in 0.5.1.
- **Bundling and signing.** The daemon binary ships at
  `Contents/MacOS/klipa-helper` and its plist at
  `Contents/Library/LaunchDaemons/dev.peterdsp.klipa.helper.plist`, both
  signed with the same Team ID as the app (inside-out signing in
  [`package-macos.sh`](../scripts/package-macos.sh)). SMAppService
  registration only works on a properly signed and notarized app, so the
  helper is unavailable in unsigned local dev builds (the app then simply
  uses Option A).

Transport is a local Unix socket rather than XPC. XPC from Rust needs C
blocks and a large amount of unsafe libxpc FFI; a `std` Unix socket plus an
audit-token `SecCode` check gives the equivalent caller-identity guarantee
(the connecting process must be klipa, signed by klipa's team) with far
less unsafe surface. The socket's file mode is not relied on for security:
an unauthorized peer is rejected by the signature check, not by the mode.

## User flow

1. Open the tray, `Settings -> Keep awake`.
2. (Optional, recommended) click **"Enable passwordless mode (one-time
   setup)"**, then approve klipa in System Settings > Login Items. After
   this, lid-closed sessions never prompt again.
3. Pick the mode **"Keep running with lid closed (runs hot)"**. Since
   0.5.4 this is a one-tap switch: with no session running it starts an
   indefinite one straight away, and during a session it moves that
   session over, keeping a timed one's remaining time and an indefinite
   one indefinite. If passwordless mode is on the Mac silently starts
   staying awake with the lid shut; otherwise macOS shows its admin
   prompt once.
4. Pick a different duration at any point, including **Indefinitely** or
   **Custom...**.
5. The submenu reads, on its own lines:

   ```
   Awake indefinitely
   System awake - lid closed, display off
   Lid closed: kept awake by klipa
   ```

   or, for a timed session, `Awake for 1h30m` on the first line.

If the admin prompt is cancelled, or a managed Mac's power policy
overrides the change, no session starts at all and the menu names the
reason instead of showing a mode that would die the moment the lid shut.

## Safety and correctness

A closed lid has nowhere to dump heat. A Mac running full tilt inside a
closed lid, or a bag, will get hot and drain the battery flat. The
feature is built with that firmly in mind:

- **The label names the cost.** The menu item literally says "runs hot"
  so the trade-off is visible before the user commits.
- **`disablesleep` is sticky.** Unlike a power assertion, it survives the
  app quitting, crashing, or the machine rebooting. If klipa set it and
  never cleared it, the Mac would never sleep again. The revert path is
  therefore load-bearing, not a nicety, and is handled three ways:
  1. **Normal end** (timer expiry, "End current session", switching to
     another mode, or quitting klipa): `Drop` on the lock restores the
     prior `disablesleep` value (silently via the helper, or via one admin
     prompt in Option A) and reconciles the recovery journal against a
     verified readback.
  2. **Clean quit:** the Quit handler ends the session *before* exiting
     the event loop, so any Option A revert prompt lands while the app is
     still alive rather than during teardown.
  3. **Unclean exit** (crash, force-quit, power loss): a recovery journal
     (`lid_awake.json`, see [`paths::lid_awake_journal`](../crates/klipa-ui/src/paths.rs))
     is written *before* the flag is changed. It records the exact
     `disablesleep` value observed beforehand and the boot session, so the
     restore returns to that prior value rather than blindly to zero (an
     existing override klipa did not set is left alone). On the next
     launch, `awake::recover_lid_closed()` reconciles it: it restores only
     if the system is genuinely still `SleepDisabled`, verifies the result
     with a tri-state read, and clears the journal only once the restore is
     confirmed. A restore that cannot be confirmed keeps the journal so the
     next launch retries rather than losing the evidence. A pre-0.5.5
     boolean `lid_awake.on` marker is migrated as ambiguous ownership (no
     fabricated prior value). The read distinguishes enabled, disabled, and
     unreadable, so an unknown state is never mistaken for a restored one.
- **No half-on sessions.** If the user cancels the admin prompt, `engage`
  releases the IOKit assertion and reports failure, so klipa never claims
  a lid-closed session that would silently die the moment the lid shuts.
- **Display is allowed to sleep.** With the lid closed the panel is off
  anyway, so a lid-closed session uses `PreventUserIdleSystemSleep` and
  lets the display power down.
- **No session is resurrected at launch.** The mode is a persisted
  preference; the session is not. klipa never re-acquires root on startup
  because of something saved in `settings.json`.

## Known limitations

- **Without the helper, ending a lid-closed session prompts once more.**
  In Option A, restoring sleep needs root, so a timed lid-closed session
  reaching its deadline asks for the admin password and the Mac stays
  awake until that is confirmed. **The passwordless helper (Option B)
  removes this**: with it installed, timed lid-closed sessions end fully
  unattended, so "sleep at the time I set" works hands-off. Plain (non-lid)
  timed sessions are unaffected either way.
- **Helper caller check.** The helper validates each caller by code
  signature (audit token to `SecCode`, pinned to klipa's identifier and
  Team ID), so a non-klipa process cannot toggle system sleep even though
  the socket is world-connectable. This is verified on signed builds; an
  unsigned build has no Team ID to pin and fails closed.
- **Undocumented behavior.** `disablesleep` is not a documented API. A
  future macOS could change lid-close behavior. The idle-only keep-awake,
  which uses supported public API, remains the always-available baseline.
- **"Custom..." is macOS-only.** It is an `NSAlert`, the one modal a
  windowless tray app can raise natively. Windows and Linux would need a
  GUI toolkit klipa does not carry, so the item is hidden there rather
  than shown dead. The fixed presets and **Indefinitely** work everywhere.
- **Power source.** klipa uses `pmset -a` (all sources) so the feature is
  not silently a no-op on battery. Running it on battery with the lid
  closed is exactly the highest-risk mode. Prefer a bounded session on
  charger.

## Sandbox-safe clamshell status (every build, App Store included)

Overriding lid-close sleep needs root, so it is direct-download only. But a
sandboxed app can still *read* whether closing the lid will keep the Mac
running, and tell the user honestly. That is the one App-Store-compliant
thing to do here, and klipa ships it in **every** build.

[`clamshell.rs`](../crates/klipa-ui/src/clamshell.rs) reads, via public
CoreGraphics and IOKit calls (all allowed in the App Sandbox, no
entitlement):

- whether an **external display** is attached (`CGGetActiveDisplayList` +
  `CGDisplayIsBuiltin`),
- the **power source** (`IOPSGetProvidingPowerSourceType`),
- whether the Mac is **Apple Silicon** (`sysctlbyname("hw.optional.arm64")`,
  correct even under Rosetta).

A MacBook runs with the lid closed only in Apple's supported clamshell
mode: an external display attached, plus AC power on Intel (Apple Silicon
can run clamshell on battery). The Keep-awake submenu shows one honest
line reflecting that:

- `Lid closed: stays awake (external display)`
- `Lid closed: connect power to stay awake` (Intel, external display, on battery)
- `Lid closed: Mac will sleep` (no external display)
- and, in the direct build while a lid-closed session is active,
  `Lid closed: kept awake by klipa` (klipa's override wins over the OS default).

On a desktop Mac (no built-in display) or off macOS, no line is shown.

This is why the App Store build is not left with nothing: it cannot force
lid-closed operation (no sandboxed app can, the only lever is a private
Apple-only entitlement or root), but it can always tell the truth about
what the lid will do. See the fuller investigation for the exact API
boundary.

## Files touched

- [`crates/klipa-ui/src/awake.rs`](../crates/klipa-ui/src/awake.rs): the
  `AwakeMode` / `AwakeDuration` domain, the `PowerSource` seam and its
  per-OS backends, the session lifecycle, the `pmset` toggle
  (helper-first, prompt fallback), the marker, `recover_lid_closed`, and
  the unit tests.
- [`crates/klipa-ui/src/helper.rs`](../crates/klipa-ui/src/helper.rs):
  app-side `SMAppService` register/status/unregister and the socket client.
- [`crates/klipa-helper`](../crates/klipa-helper): the root daemon.
- [`packaging/macos/dev.peterdsp.klipa.helper.plist`](../packaging/macos/dev.peterdsp.klipa.helper.plist):
  the LaunchDaemon plist bundled in the app.
- [`crates/klipa-ui/src/clamshell.rs`](../crates/klipa-ui/src/clamshell.rs):
  sandbox-safe lid-close outlook detection (all builds, App Store included).
- [`crates/klipa-ui/src/prompt.rs`](../crates/klipa-ui/src/prompt.rs): the
  `NSAlert` prompt behind **Custom...**, plus its input validation.
- [`crates/klipa-ui/src/settings.rs`](../crates/klipa-ui/src/settings.rs):
  the persisted `awake_mode` / `awake_custom_minutes` preferences.
- [`crates/klipa-ui/src/paths.rs`](../crates/klipa-ui/src/paths.rs): the
  `lid_awake_marker` sentinel path.
- [`crates/klipa-ui/src/tray.rs`](../crates/klipa-ui/src/tray.rs): the
  mode radio group, the duration presets, the status/detail lines and the
  passwordless-helper menu items.
- [`crates/klipa-ui/src/main.rs`](../crates/klipa-ui/src/main.rs): the menu
  handlers, helper-state wiring, preference restore, startup recovery, and
  revert-before-quit.
- [`scripts/bundle-macos.sh`](../scripts/bundle-macos.sh) and
  [`scripts/package-macos.sh`](../scripts/package-macos.sh): build, embed,
  and sign the helper (direct build only).

## Manual test checklist

Build, sign, and notarize the direct (non-`mas`) variant, install it to
`/Applications`, and run it. On a MacBook:

1. Passwordless setup: click "Enable passwordless mode", approve klipa in
   System Settings > Login Items, confirm the menu now shows "Passwordless
   mode: on" and `sudo launchctl print system/dev.peterdsp.klipa.helper`
   lists the daemon.
2. Pick the lid-closed mode, then a short (5 minute) session. With the
   helper on there should be **no** password prompt.
3. Confirm `pmset -g | grep SleepDisabled` shows `1`.
4. Close the lid, confirm the machine keeps running (e.g. it still answers
   on the network, or an ongoing task keeps progressing).
5. Let the timer expire and confirm `SleepDisabled` returns to `0` with no
   prompt (this is the Option B win).
6. Turn off passwordless mode, repeat step 2, and confirm the admin prompt
   now appears (Option A fallback still works).
7. Crash test: start a session, `kill -9` klipa, relaunch, confirm
   `recover_lid_closed()` restores `SleepDisabled` to `0` (silently with
   the helper on).
8. Build with `--no-default-features --features mas` and confirm the
   lid-closed mode and helper items are absent, the clamshell outlook line
   is still shown, and the bundle contains no `klipa-helper` or
   LaunchDaemon plist.
9. Indefinite: pick **Indefinitely**, confirm the menu reads
   `Awake indefinitely` with no countdown and `pmset -g assertions` lists
   one klipa assertion. Leave it running, then **End current session** and
   confirm the assertion disappears immediately.
10. Modes: switch between **Keep screen & Mac awake** and **Keep Mac
    awake, allow screen off** and confirm `pmset -g assertions` shows
    exactly one klipa assertion at a time, of the matching type, never two.
