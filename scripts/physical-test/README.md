# Keep-awake physical test harness

A reproducible harness for the one gate that cannot be satisfied in CI or a
VM: proving a real MacBook keeps working with the lid physically closed during
a klipa keep-awake session, and that normal sleep is restored afterward.

Everything here is read-only except the klipa session you start from the menu.
The recorder takes **no** power assertions, so it cannot bias the result.

## What you run

| Script | Purpose |
|---|---|
| `baseline.sh` | Capture power/hardware state before testing (read-only). |
| `recorder.sh` | Append a timestamped heartbeat every N seconds, taking no assertions. Leave it running during a case. |
| `analyze.sh` | Report the largest heartbeat gap, dark-wake vs full-wake, charger transitions, and bias checks. |
| `restore-check.sh` | Confirm `SleepDisabled` returned to baseline and no restore is owed. |

All four write into a single run directory (`./keep-awake-run-<timestamp>`),
which stays out of the repo. Do not commit these run files.

## One-time setup

```sh
chmod +x scripts/physical-test/*.sh
```

Install the signed candidate (the exact bytes users get, not a `cargo run`
build). See "Installation" below. Then enable the passwordless helper so timed
closed-lid sessions can expire unattended:

- klipa menu > Keep awake > "Enable passwordless mode (one-time setup)", then
  approve **klipa-helper** in System Settings > General > Login Items (toggle
  on). The menu should then read "Passwordless mode: on".

Timed closed-lid sessions are only reliable unattended with the helper
approved. Without it, restoration at expiry would need an admin prompt that
cannot appear behind a shut lid, and klipa instead restores on next launch.

## The control run (do this first, it is mandatory)

A "stayed awake" result proves nothing unless the same Mac sleeps under the
same idle conditions with klipa inactive.

```sh
scripts/physical-test/baseline.sh                 # writes ./keep-awake-run-*/baseline.txt
scripts/physical-test/recorder.sh "" 10 &         # start the recorder
# Quit klipa entirely. Set a short idle sleep, close the lid, wait > that timeout.
# Reopen, stop the recorder (fg then Ctrl-C), then:
scripts/physical-test/analyze.sh
```

Expect a sleep-sized gap in the heartbeat (the Mac slept). If it did not sleep,
some other inhibitor is active (see the foreign-assertions line) and must be
removed before the real cases mean anything.

## The cases

Start the recorder once and leave it running across a case:

```sh
scripts/physical-test/recorder.sh "" 10 &
```

For each case, start the stated klipa session from the menu, perform the
physical actions, then run `analyze.sh` and (for stop/expiry cases)
`restore-check.sh`. Record for each: model, macOS build, the installed
artifact SHA256, helper version (from Copy diagnostics), starting `pmset -g`,
elapsed time, and pass/fail.

1. **Closed lid, no external display, AC (>= 30 min)**
   - Plug in AC. No external display or dock.
   - Menu > Keep awake > "Keep running with lid closed", then pick the 1 hour
     preset (or Custom 40 min).
   - Close the lid. Wait 30+ minutes. Open the lid.
   - PASS if `analyze.sh` shows no sleep-sized gap and the heartbeat advanced
     the whole time (and foreign assertions stayed 0).

2. **Closed lid, no external display, battery (>= 30 min)**
   - Only if battery closed-lid is advertised and charge is comfortable
     (> ~50%). Do not drain to an emergency level.
   - Same as case 1 but on battery. Record starting and ending charge.

3. **Charger connect/disconnect while shut (>= 5 each direction)** - highest risk
   - Start a lid-closed session, close the lid.
   - Unplug AC, wait ~1 min; plug AC, wait ~1 min. Repeat so you have at least
     5 unplug and 5 plug events. Keep the lid shut throughout.
   - Open the lid, stop the recorder, run `analyze.sh`.
   - PASS if no sleep-sized gap bracketed any transition, the charger
     transitions appear in the analysis, and `SleepDisabled` stayed 1.

4. **Repeated lid cycles (>= 10)**
   - Start a lid-closed session. Close and open the lid 10+ times with short
     waits. PASS if protection never dropped (SleepDisabled stayed 1,
     heartbeat continuous while shut).

5. **Short timed session, lid shut (unattended expiry)**
   - Start a lid-closed session with a 3-5 minute Custom duration.
   - Close the lid. Wait past the duration + ~1 min. Open the lid.
   - Run `restore-check.sh`: PASS if `SleepDisabled` returned to baseline with
     no admin prompt having appeared, and no owed restore remains.

6. **Timed session with the menu held open**
   - Start a short timed lid session, open the Keep awake submenu and keep it
     open past expiry. PASS if it still expires on time (the daemon owns
     expiry; an open menu must not delay it) and restores.

7. **Normal quit and force-quit**
   - Start a lid-closed session, then (a) Quit klipa from the menu, and in a
     separate run (b) force-quit it (Activity Monitor > Force Quit).
   - After each, run `restore-check.sh`: PASS if normal sleep is restored
     without relaunching klipa (for force-quit, the daemon restores within the
     lease window, about a minute).

8. **Stop / restore / helper uninstall**
   - End the session from the menu ("End current session" / "Restore normal
     sleep"), then "Turn off passwordless mode". Run `restore-check.sh`: PASS
     if the baseline value returned and no orphan helper/socket remains.

## Reading a result

- A heartbeat gap larger than ~2x the interval means the Mac slept or the
  recorder was suspended: that is a FAIL for a "stayed awake" case.
- `DarkWake` in the sleep/wake log is not uninterrupted work. If gaps line up
  with DarkWake/Wake entries, the case failed.
- If the foreign-assertions count was ever > 0, a non-klipa inhibitor was
  active and the pass is not attributable to klipa. Remove it and rerun.
- Do not rely on an SSH session or continuous input as the "proof" it stayed
  up: that input can itself prevent sleep.

## Installation of the signed candidate

See the repository release checklist. In short: download the published (or
staged draft) `klipa-<version>-macos.pkg`, verify its SHA256 against
`SHA256SUMS.txt`, confirm `spctl --assess --type install` accepts it and
`pkgutil --check-signature` shows Developer ID + notarized, then install it to
`/Applications/klipa.app`. Confirm the version and signature with:

```sh
defaults read /Applications/klipa.app/Contents/Info CFBundleShortVersionString
codesign --verify --strict --deep --verbose=2 /Applications/klipa.app
codesign --verify --strict /Applications/klipa.app/Contents/MacOS/klipa-helper
```
