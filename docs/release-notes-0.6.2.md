# klipa 0.6.2

The 0.6.x series makes "keep awake with the lid closed" reliable and
honest, and hardens how the privileged part of it is owned and recovered.

## Keep awake with the lid closed

- The lid-closed override (`disablesleep`) is owned end to end by the
  privileged helper. It writes a durable, root-owned recovery record before
  it changes any setting, verifies every restore by reading the flag back,
  and reconciles anything left over at startup.
- A crash-safety lease plus a helper-owned session deadline mean a frozen,
  force-quit, or crashed app cannot strand a global sleep override: the
  helper restores normal sleep on its own. Timed closed-lid sessions end
  unattended because the daemon, not the menu, owns expiry.
- The keep-awake menu reports real, verified state: preparing, awaiting
  approval, active, recovering, needs attention. "End session / restore
  normal sleep" and "Copy diagnostics" stay reachable even after the trial
  ends, so an active session or an owed restore is never trapped.
- Changing the duration or mode of a running lid-closed session updates it
  in place instead of briefly dropping protection. The in-app updater
  restores normal sleep before it relaunches.

## What closing the lid actually does

klipa reports honestly what will happen if you close the lid now:

- With an external display attached (clamshell), the Mac stays awake with no
  special privilege, on AC on Intel and on battery on Apple Silicon.
- On a bare laptop (no external display), keep-awake with the lid closed
  needs a system-level override that requires administrator approval; klipa
  names when it cannot guarantee it (for example on a managed Mac).

## Fixes since 0.6.0

- 0.6.1: four correctness fixes in the helper session contract (stale lease
  renewal clobbering an in-place duration change; End clearing an error
  without a verified restore; a failed restore stranded until restart; the
  updater replacing the bundle before coordinating the session), each with a
  regression test, plus reasserting an override that a power-source
  transition cleared.
- 0.6.2: a `NotFound` helper status (seen after running under Xcode or
  upgrading over a sideloaded build) is now treated as registerable and
  self-heals via unregister + register, instead of a dead end where
  passwordless mode could never be enabled.

## Compatibility

Clipboard history, search, the menu-bar clock, and licensing are unchanged,
as are the Windows and Linux builds. The passwordless helper needs macOS 13
or later; on 11 and 12 the lid-closed override uses a one-time admin-password
prompt, and ordinary keep-awake works as before.

## A note on the Mac App Store build

The App Store build is sandboxed and cannot run the in-app privileged
override; it still reports clamshell state honestly and keeps the Mac awake
in the supported clamshell configuration. See
`docs/mas-closed-lid-feasibility.md` for what closed-lid support on the App
Store requires.
