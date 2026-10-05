# Keep-awake physical acceptance: one self-contained checklist

This is the one gate that cannot be satisfied in CI or a VM: proving a real
MacBook keeps working with the lid physically shut during a klipa keep-awake
session, and that normal sleep is restored afterward. It is the single blocker
to publishing the staged, signed direct v0.6.2 release.

You run six cases. Everything you type is below in full; you do not have to
read any other file, and you do not have to judge pass/fail or read any log.
Each case writes its evidence into its own `./keep-awake-run-<timestamp>/`
directory under `~/git/klipa`. When you are done, leave those directories in
place and tell the agent; the agent reads the evidence and fills the matrix.

## What these tests do and do not touch

- They exercise the **already-installed direct build** (`/Applications/klipa.app`,
  v0.6.2, with its `klipa-helper`). They only start/stop keep-awake sessions
  from the menu and, in the last case, toggle passwordless mode. They do **not**
  uninstall klipa, and they do **not** touch your license, data, or activation.
- They do **not** install or use the Mac App Store "Power Protect" companion
  (the sudoers rule / `powerprotect` scripts). That MAS candidate is kept
  entirely separate from this direct-build gate. Do not install it for these
  tests.
- The recorder takes **no** power assertions, so it cannot bias the result.

## One-time preparation (once, before the first case)

1. Make the scripts executable:
   ```sh
   cd ~/git/klipa
   chmod +x scripts/physical-test/*.sh
   ```
2. Confirm passwordless mode is on (needed for the timed-expiry case C5 to
   restore behind a shut lid): klipa menu > Keep awake > the submenu should read
   **"Passwordless mode: on"**. If it says off, choose "Enable passwordless mode
   (one-time setup)" and approve **klipa-helper** in System Settings > General >
   Login Items.

## Clear interfering keep-awake processes (mandatory before EACH case)

A "stayed awake" result means nothing if something other than klipa was holding
the Mac awake. Run this to list every process currently holding a
wake-preventing assertion:

```sh
pmset -g assertions | grep -iE "pid [0-9]+\(" | grep -iE "PreventUserIdleSystemSleep|PreventSystemSleep"
pgrep -xl caffeinate
```

Quit everything it lists **except** `klipa` / `klipa-helper`. In practice the
real offenders on this Mac are:

- **This Claude Code session and any other terminal AI agent (Codex, etc.)** —
  they spawn `caffeinate`, which disables idle sleep. You must quit the coding
  session to run these tests; that is why the agent cannot run them for you.
- **Xcode** when it is running or testing ("Xcode running tests").
- **Any media playing** in Safari/Chrome, Music, QuickTime, or a video call —
  pause or close it (media playback takes a sleep assertion).

Re-run the two commands above until the first prints nothing (no non-klipa
lines) and `pgrep` prints nothing. Keep normal security on (SIP, Gatekeeper,
FileVault as usual); do not disable them.

---

# The six cases

Do **M1 first** (it is the control). The other five can be done in any order and
across separate sittings, because each case is a self-contained run directory.

Each case follows the same shape:

```sh
cd ~/git/klipa
scripts/physical-test/baseline.sh            # mints a fresh run dir + records state
scripts/physical-test/recorder.sh "" 10 &    # heartbeat every 10s into that same dir
#   ... do the physical actions for the case ...
fg        # bring the recorder to the foreground
#   press Ctrl-C to stop it
scripts/physical-test/analyze.sh             # writes/﻿prints the analysis for that dir
#   (restore cases also run restore-check.sh, noted per case)
```

`baseline.sh` always creates a new `keep-awake-run-<timestamp>/`; the recorder,
analyze, and restore-check all attach to the newest one automatically, so you
never pass a path.

## M1 - Control run: the Mac DOES sleep with klipa OFF

Plain English: before trusting any "stayed awake" result, prove this Mac still
goes to sleep on its own when klipa is not running. If it does not sleep here,
something is still holding it awake and every other case would be meaningless.

- **Klipa state: OFF.** Quit klipa entirely (menu > Quit). Confirm it is gone.
- Set a short idle sleep so you are not waiting long: System Settings > Lock
  Screen (or Battery/Power) > turn the display/system sleep down to about
  2 minutes, or run `sudo pmset -a sleep 2` (you will be asked for your
  password). Remember to restore your normal value afterward.
- Steps:
  ```sh
  cd ~/git/klipa
  scripts/physical-test/baseline.sh
  scripts/physical-test/recorder.sh "" 10 &
  ```
  Close the lid. Wait at least 5 minutes past the idle timeout (so ~7-10 min).
  Open the lid, `fg`, press Ctrl-C, then:
  ```sh
  scripts/physical-test/analyze.sh
  ```
- Recovery: restore your normal sleep timeout (`sudo pmset -a sleep <your value>`
  or via System Settings).
- Evidence saved: `keep-awake-run-*/baseline.txt`, `heartbeat.tsv`. (The agent
  expects a sleep-sized gap here; that is the good result for the control.)

## C1 - Closed lid, no external display, on AC, >= 30 min

Plain English: the core promise. On wall power with the lid shut and no external
display, a klipa keep-awake session should keep the Mac fully awake the whole
time.

- **Klipa state: RUNNING, with an active lid-closed session.**
- Plug in AC. Disconnect any external display or dock.
- Steps:
  ```sh
  cd ~/git/klipa
  scripts/physical-test/baseline.sh
  scripts/physical-test/recorder.sh "" 10 &
  ```
  klipa menu > Keep awake > **"Keep running with lid closed"**, then pick the
  **1 hour** preset. Close the lid. Wait **at least 30 minutes**. Open the lid,
  `fg`, Ctrl-C, then:
  ```sh
  scripts/physical-test/analyze.sh
  ```
- Recovery: klipa menu > Keep awake > **"End current session"** so the override
  clears before the next case.
- Evidence saved: that run dir's `baseline.txt`, `heartbeat.tsv`.

## C3 - Charger connect/disconnect while shut (>= 5 each way) - highest risk

Plain English: power-source changes are the most common cause of a Mac dropping
to sleep behind a shut lid. This cycles the charger repeatedly and checks
protection never lapsed.

- **Klipa state: RUNNING, with an active lid-closed session.**
- Steps:
  ```sh
  cd ~/git/klipa
  scripts/physical-test/baseline.sh
  scripts/physical-test/recorder.sh "" 10 &
  ```
  Start the same lid-closed session (Keep awake > "Keep running with lid
  closed" > 1 hour). Close the lid. With the lid **shut** the whole time:
  unplug AC, wait ~1 min; plug AC, wait ~1 min. Repeat until you have done at
  least **5 unplugs and 5 plugs**. Open the lid, `fg`, Ctrl-C, then:
  ```sh
  scripts/physical-test/analyze.sh
  ```
- Recovery: klipa menu > Keep awake > "End current session".
- Evidence saved: that run dir's `baseline.txt`, `heartbeat.tsv` (the heartbeat
  records each charger transition for the agent to correlate).

## C5 - Short timed session restores itself at expiry (unattended, lid shut)

Plain English: if you set a timed session and walk away with the lid shut, it
must end on time and restore normal sleep **by itself**, with no password prompt
(which could never be answered behind a closed lid).

- **Klipa state: RUNNING, with an active short TIMED session.**
- Confirm passwordless mode is on (see preparation).
- Steps:
  ```sh
  cd ~/git/klipa
  scripts/physical-test/baseline.sh
  scripts/physical-test/recorder.sh "" 10 &
  ```
  klipa menu > Keep awake > "Keep running with lid closed" > **Custom**, enter
  **4 minutes**. Close the lid. Wait past the duration plus ~1 minute (so about
  6 minutes). Open the lid, `fg`, Ctrl-C, then:
  ```sh
  scripts/physical-test/analyze.sh
  scripts/physical-test/restore-check.sh
  ```
- Recovery: none needed if restore-check says it restored; if it did not,
  klipa menu > Keep awake > "Restore normal sleep".
- Evidence saved: that run dir's `baseline.txt`, `heartbeat.tsv`, plus the
  restore-check output on screen (also fine to leave as is).

## C8 - Stop / restore from the menu, then turn passwordless mode off

Plain English: ending a session from the menu must put sleep back exactly as it
was, and turning off passwordless mode must leave no helper override behind.

- **Klipa state: RUNNING; you will start a session, then stop it.**
- Steps:
  ```sh
  cd ~/git/klipa
  scripts/physical-test/baseline.sh
  scripts/physical-test/recorder.sh "" 10 &
  ```
  Start a lid-closed session (Keep awake > "Keep running with lid closed" >
  1 hour). You can leave the lid open for this one. Then, from the menu:
  **"End current session"** (or "Restore normal sleep"), then **"Turn off
  passwordless mode"**. `fg`, Ctrl-C, then:
  ```sh
  scripts/physical-test/analyze.sh
  scripts/physical-test/restore-check.sh
  ```
- Recovery: none; this case is itself the restore. If you want passwordless mode
  back on for later, re-enable it from the menu.
- Evidence saved: that run dir's files plus the restore-check output.

## M2 - Battery closed-lid behavior recorded (pass or honest limitation)

Plain English: repeat the core test on battery and simply record what happens.
On Apple Silicon a bare-laptop lid-closed session on battery may legitimately
not stay awake; the goal is to record the real behavior, not to force a pass.

- **Klipa state: RUNNING, with an active lid-closed session.**
- Only do this if the battery is comfortable (above ~50%). Do not drain to an
  emergency level.
- Unplug AC. Steps:
  ```sh
  cd ~/git/klipa
  scripts/physical-test/baseline.sh
  scripts/physical-test/recorder.sh "" 10 &
  ```
  Start the lid-closed session (Keep awake > "Keep running with lid closed" >
  1 hour). Note the starting battery percentage. Close the lid. Wait **at least
  30 minutes**. Open the lid, note the ending percentage, `fg`, Ctrl-C, then:
  ```sh
  scripts/physical-test/analyze.sh
  ```
- Recovery: klipa menu > Keep awake > "End current session". Plug AC back in.
- Evidence saved: that run dir's `baseline.txt`, `heartbeat.tsv`; also tell the
  agent the start/end battery percentages.

---

## When you are done

Leave every `keep-awake-run-*` directory in `~/git/klipa` (they are git-ignored
and survive quitting the coding session). Reopen Claude Code and say the runs
are ready. The agent reads each run's `baseline.txt`, `heartbeat.tsv`, and the
analysis, correlates them with the system sleep/wake log, fills the matrix below
with PASS/FAIL and the evidence, and only then proceeds toward publishing. The
agent will not declare a pass it has not inspected, and you are not asked to
interpret any log yourself.

## Full acceptance matrix

Minimum to publish the direct release: M1, C1, C3, C5, C8, M2 (the core
"stays awake + restores" proof). The rest raise confidence and stay honestly
UNTESTED until run.

| # | Case | Priority | Klipa | Status | Evidence |
|---|---|---|---|---|---|
| M1 | Control run: Mac sleeps with klipa OFF (lid shut) | must | OFF | UNTESTED | |
| C1 | Closed lid, no external display, AC, >= 30 min | must | session | UNTESTED | |
| C2 | Closed lid, no external display, battery, >= 30 min (charge > 50%) | should | session | UNTESTED | |
| C3 | Charger connect/disconnect while shut, >= 5 each way (highest risk) | must | session | UNTESTED | |
| C4 | Repeated lid cycles, >= 10 | should | session | UNTESTED | |
| C5 | Short timed session (3-5 min), lid shut, unattended expiry restores | must | timed session | UNTESTED | |
| C6 | Timed session with the menu held open past expiry | should | timed session | UNTESTED | |
| C7 | Indefinite session soak, >= 2 h, lid shut | should | session | UNTESTED | |
| C8 | Stop / restore from menu; then turn off passwordless mode | must | start+stop | UNTESTED | |
| C9 | Session extension while shut (change duration mid-session) | should | session | UNTESTED | |
| C10 | Duration/mode change while shut updates in place | should | session | UNTESTED | |
| C11 | Normal quit restores sleep | should | session+quit | UNTESTED | |
| C12 | Force-quit: daemon restores within the lease (~1 min) | should | session+kill | UNTESTED | |
| C13 | Helper crash / failed restore is retried autonomously | nice | session | UNTESTED | |
| C14 | Reboot / logout during a session: reconciled on next boot | should | session | UNTESTED | |
| C15 | Real in-app update old->new while a session is active restores first | should | session | UNTESTED | |
| M2 | Battery closed-lid behavior recorded (pass or documented limitation) | must | session | UNTESTED | |

A case is PASS only when the heartbeat is continuous while shut (no gap larger
than about twice the interval), `SleepDisabled` held 1 for keep-awake cases (or
returned to the baseline for restore cases), foreign assertions stayed 0, and
the `pmset -g log` sleep/wake evidence corroborates. The agent checks all of
this; you only run the cases and keep the directories.
