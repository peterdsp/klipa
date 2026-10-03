# klipa Power Protect (Mac App Store closed-lid companion)

This directory holds the **off-store** helper that lets the sandboxed Mac App
Store build of klipa keep a MacBook awake with the lid closed on a bare laptop
(no external display). It is a candidate architecture under proof; see
`docs/mas-closed-lid-feasibility.md` and `docs/mas-closed-lid-implementation.md`.

## Why it is separate from the app

Apple's App Sandbox and App Store Review Guideline 2.4.5 forbid a MAS app from
installing code in shared locations, downloading code to add functionality, or
escalating to root. Keeping a Mac awake with the lid shut and no external
display requires the root-only `pmset -a disablesleep 1`. So the privileged
pieces cannot ship in the App Store app; the user installs them once, off
store, exactly as Amphetamine's "Power Protect" does. The sandboxed app then
runs the installed toggle script through `NSUserUnixTask` (the one sanctioned
way a sandboxed app may execute a user script), from
`~/Library/Application Scripts/dev.peterdsp.klipa/`.

The direct-download (Developer ID) build does NOT need this: it keeps the lid
closed via its own privileged `klipa-helper` daemon and is the more robust
channel. Power Protect exists only for users who get klipa from the App Store.

## Files

| File | Installs to | Purpose |
|---|---|---|
| `klipa-powerprotect` | `~/Library/Application Scripts/dev.peterdsp.klipa/` | Toggle script the sandboxed app runs (`on`/`off`/`status`); elevates only two exact `pmset` commands and reports read-back |
| `klipa.sudoers.in` | `/etc/sudoers.d/dev.peterdsp.klipa` (0440 root:wheel) | NOPASSWD rule scoped to exactly `pmset -a disablesleep 0\|1`, validated with `visudo` before install |
| `klipa-powerprotect-watchdog` | `~/Library/Application Support/dev.peterdsp.klipa/` | Owns unattended TIMED restore behind a shut lid (reads the app's `powerprotect.deadline`) |
| `dev.peterdsp.klipa.powerprotect.plist.in` | `~/Library/LaunchAgents/` | Per-user agent that runs the watchdog (RunAtLoad + 30s) |
| `install.sh` / `uninstall.sh` | user-run | One admin authorization; fully reversible |

## Security properties

- Privilege is a single sudoers rule scoped to two exact commands, nothing
  else. No root daemon. The script runs as the user and elevates only those
  two `pmset` calls.
- The user performs the install and the one admin authorization; the app never
  installs or escalates anything.
- `install.sh` validates the sudoers rule with `visudo -cf` before and after
  placing it, so a malformed rule can never wedge `sudo`.
- `uninstall.sh` restores normal sleep and removes every file.

## Honest limitations vs the direct build

The direct build's daemon owns a crash-safety lease, a root recovery journal,
and autonomous retry. This companion's guarantees are weaker: timed restore is
owned by the user LaunchAgent (robust during an active session), and
crash/reboot restore is best-effort on next agent load. Described as such in
the app and in App Review notes, never equated with the direct build.

## Status

Not yet verified end to end: the `NSUserUnixTask` invocation and the sandbox
boundary need a real MAS-signed build (owner Apple account), and the on-device
toggle/restore cases need a clean environment (so the direct helper and any
residual override do not contaminate the result). The sudoers rule validates
with `visudo` and all scripts pass `sh -n`; the on-device and sandbox proofs
are the remaining work.
