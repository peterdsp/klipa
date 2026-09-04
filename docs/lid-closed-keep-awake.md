# Keep awake with the lid closed

How klipa keeps a Mac running with the lid physically shut, why it works
the way it does, and the safety rules around it. This documents the
feature implemented in [`crates/klipa-ui/src/awake.rs`](../crates/klipa-ui/src/awake.rs).

## The core problem

klipa's normal keep-awake holds an IOKit power assertion
(`IOPMAssertionCreateWithName`), the same public API that `caffeinate`
wraps. That assertion suppresses the **idle** sleep path: the timeout
that fires when nobody touches the machine.

Closing the lid is a different thing. It is not an idle timeout, it is an
**explicit** sleep request, and macOS honors it regardless of any power
assertion. This is why `caffeinate`, KeepingYouAwake, and klipa's plain
keep-awake all hold a Mac awake on an open desk but let it drop the
moment you fold it shut.

The only native lever that changes lid-close behavior is the system power
setting `disablesleep`:

```bash
sudo pmset -a disablesleep 1   # stop sleeping, even with the lid closed
sudo pmset -a disablesleep 0   # restore normal behavior
```

Every third-party app that genuinely keeps a Mac awake with the lid
closed (Amphetamine, KeepAwake, AwakeToggle, InsomniaX) reaches this same
setting underneath. It works on Intel and Apple Silicon in practice, is
undocumented, and requires root.

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
- **The daemon** ([`crates/klipa-helper`](../crates/klipa-helper)) is
  dependency-free pure `std`. It runs as root under launchd
  (`RunAtLoad` + `KeepAlive`), listens on a fixed Unix socket
  (`/var/run/dev.peterdsp.klipa.helper.sock`), and accepts a closed
  whitelist of commands: `set 1`, `set 0`, `ping`. Anything else is
  rejected, so a compromise of the app can only toggle system sleep, never
  run arbitrary code as root. This matches the chosen scope: the helper
  does the power toggle and nothing else.
- **Bundling and signing.** The daemon binary ships at
  `Contents/MacOS/klipa-helper` and its plist at
  `Contents/Library/LaunchDaemons/dev.peterdsp.klipa.helper.plist`, both
  signed with the same Team ID as the app (inside-out signing in
  [`package-macos.sh`](../scripts/package-macos.sh)). SMAppService
  registration only works on a properly signed and notarized app, so the
  helper is unavailable in unsigned local dev builds (the app then simply
  uses Option A).

Transport is a local Unix socket rather than XPC. XPC from Rust needs C
blocks and a large amount of unsafe libxpc FFI; a `std` Unix socket is
robust and auditable. The cost is a weaker peer check: the socket is
created `0666`, so any local process running as the user could also ask to
toggle system sleep. The capability is deliberately limited to that one
benign power setting. Hardening to a code-signed XPC peer check is the
natural future step if that residual risk ever matters.

## User flow

1. Open the tray, `Settings -> Keep awake`.
2. (Optional, recommended) click **"Enable passwordless mode (one-time
   setup)"**, then approve klipa in System Settings > Login Items. After
   this, lid-closed toggles never prompt again.
3. Check **"Stay awake with lid closed (runs hot)"**. This only stores the
   preference, no prompt yet.
4. Pick a duration (or "Indefinitely"). If passwordless mode is on the Mac
   silently starts staying awake with the lid shut; otherwise macOS shows
   its admin prompt once.
5. The session status reads e.g. `Awake - 1h30m left - lid closed`.

Toggling the checkbox while a session is already running restarts the
session so the new mode takes effect immediately.

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
  1. **Normal end** (timer expiry, "End current session", unchecking the
     box, or quitting klipa): `Drop` on the backend runs
     `pmset disablesleep 0` (silently via the helper, or via one admin
     prompt in Option A).
  2. **Clean quit:** the Quit handler ends the session *before* exiting
     the event loop, so any Option A revert prompt lands while the app is
     still alive rather than during teardown.
  3. **Unclean exit** (crash, force-quit, power loss): a sentinel file
     (`lid_awake.on`, see [`paths::lid_awake_marker`](../crates/klipa-ui/src/paths.rs))
     is written the instant the flag is set. On the next launch,
     `awake::recover_lid_closed()` sees the marker and, only if the system
     is genuinely still `SleepDisabled`, restores normal sleep. If a
     reboot already cleared the flag, it just deletes the marker with no
     prompt.
- **No half-on sessions.** If the user cancels the admin prompt, `engage`
  releases the IOKit assertion and reports failure, so klipa never claims
  a lid-closed session that would silently die the moment the lid shuts.
- **Display is allowed to sleep.** With the lid closed the panel is off
  anyway, so a lid-closed session uses `PreventUserIdleSystemSleep` and
  lets the display power down.

## Known limitations

- **Without the helper, ending a lid-closed session prompts once more.**
  In Option A, restoring sleep needs root, so a timed lid-closed session
  reaching its deadline asks for the admin password and the Mac stays
  awake until that is confirmed. **The passwordless helper (Option B)
  removes this**: with it installed, timed lid-closed sessions end fully
  unattended, so "sleep at the time I set" works hands-off. Plain (non-lid)
  timed sessions are unaffected either way.
- **Helper socket peer check.** The helper socket is `0666`, so any local
  process running as the user could ask to toggle system sleep. The
  capability is limited to that one benign setting; a code-signed XPC peer
  check would tighten it further.
- **Undocumented behavior.** `disablesleep` is not a documented API. A
  future macOS could change lid-close behavior. The idle-only keep-awake,
  which uses supported public API, remains the always-available baseline.
- **Power source.** klipa uses `pmset -a` (all sources) so the feature is
  not silently a no-op on battery. Running it on battery with the lid
  closed is exactly the highest-risk mode. Prefer a bounded session on
  charger.

## Files touched

- [`crates/klipa-ui/src/awake.rs`](../crates/klipa-ui/src/awake.rs): the
  lid-closed backend, the `pmset` toggle (helper-first, prompt fallback),
  the marker, and `recover_lid_closed`.
- [`crates/klipa-ui/src/helper.rs`](../crates/klipa-ui/src/helper.rs):
  app-side `SMAppService` register/status/unregister and the socket client.
- [`crates/klipa-helper`](../crates/klipa-helper): the root daemon.
- [`packaging/macos/dev.peterdsp.klipa.helper.plist`](../packaging/macos/dev.peterdsp.klipa.helper.plist):
  the LaunchDaemon plist bundled in the app.
- [`crates/klipa-ui/src/paths.rs`](../crates/klipa-ui/src/paths.rs): the
  `lid_awake_marker` sentinel path.
- [`crates/klipa-ui/src/tray.rs`](../crates/klipa-ui/src/tray.rs): the
  `AWAKE_LID_ID` toggle and the passwordless-helper menu items.
- [`crates/klipa-ui/src/main.rs`](../crates/klipa-ui/src/main.rs): the menu
  handlers, helper-state wiring, startup recovery, and revert-before-quit.
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
2. Enable lid-closed, start a short (5 minute) session. With the helper on
   there should be **no** password prompt.
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
   lid-closed toggle and helper items are absent, and the bundle contains
   no `klipa-helper` or LaunchDaemon plist.
