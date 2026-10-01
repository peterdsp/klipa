# klipa 0.6.0

Keep-awake reliability and a hardened privileged helper.

## Keep awake

- The lid-closed override is now owned end to end by the privileged helper,
  not the app. The helper writes a durable, root-owned recovery record
  before it changes any system setting, verifies every restore, and
  reconciles leftovers at startup, so a crash, force-quit, or power loss can
  no longer leave a Mac unable to sleep.
- A crash-safety lease plus a helper-owned session deadline mean a frozen or
  closed app releases the override on its own, and timed sessions end
  unattended. The app only renews a healthy session; it never has to be
  alive for the override to be undone.
- The app and helper now speak a versioned, structured protocol. The helper
  authenticates every caller by code signature (the connecting process must
  be klipa, signed by klipa's team) and accepts only a closed set of typed
  operations, never arbitrary commands.
- The Keep-awake menu reports the real, verified protection state
  (preparing, awaiting approval, active, recovering, needs attention) rather
  than assuming a request succeeded. End session / restore normal sleep and
  a Copy diagnostics action stay reachable even after the trial ends.
- Duration and mode changes update a running lid session in place instead of
  briefly dropping protection. The in-app updater restores normal sleep
  before it relaunches.

## Compatibility

- Clipboard history, search, menu-bar display, licensing, and the
  Windows/Linux builds are unchanged.
- The passwordless helper requires macOS 13 or later; on macOS 11 and 12,
  lid-closed mode uses the one-time admin-password prompt instead, and
  ordinary keep-awake works as before.

## A note on closed-lid behavior

Keeping a Mac awake with the lid physically shut depends on the hardware,
the power source, and macOS itself. klipa reports honestly what closing the
lid will do on your machine, and names when it cannot guarantee it (for
example a managed Mac whose power policy overrides the setting). Closing a
running Mac's lid with no way to shed heat is a real cost: prefer a bounded
session on the charger.
