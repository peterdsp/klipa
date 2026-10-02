# Keep-awake physical acceptance: owner checklist and results matrix

The one gate that cannot be satisfied in CI or a VM: proving a real MacBook
keeps working with the lid physically shut during a klipa keep-awake session,
and that normal sleep is restored afterward. This is the single blocker to
publishing the staged, signed v0.6.2 release.

What the agent already verified (no owner action): code gates (fmt, build,
test, clippy, MAS feature combos) pass on `main`; the v0.6.2 draft macOS
artifacts are Developer-ID signed, notarized, and their SHA256 match
`SHA256SUMS.txt`; the installed 0.6.2 app is signed/notarized and running;
the license is activated to info@peterdsp.dev; the passwordless helper daemon
is installed, running, version-matched, and correctly authenticating; the
test harness (baseline, recorder, analyze, restore-check) runs end to end.

What only the owner can do: physically close the lid, cycle the charger, and
reboot. Everything below is the exact sequence. Run it with the coding
session quit so no agent holds a sleep assertion. Send the agent the run
directories afterward; the agent inspects the evidence, not you.

## Pre-flight: clear contamination (mandatory, or results are meaningless)

A "stayed awake" result proves nothing if sleep was already disabled or
another process was holding the Mac awake.

1. End any active klipa session: klipa menu > Keep awake > "End current
   session" / "Restore normal sleep". Then confirm the override cleared:
   ```sh
   pmset -g | grep -i SleepDisabled      # must read: SleepDisabled  0
   ```
   (As of 2026-10-03 this currently reads 1, from an earlier session. It
   must be 0 before the control run.)
2. Quit every other sleep inhibitor on this Mac. The agent observed these
   holding assertions: Claude Code (spawns `caffeinate -i`), a Codex
   "Computer Use" agent (`SkyComputerUseService`), and Xcode ("Xcode running
   tests"). Quit them. Then confirm zero foreign inhibitors:
   ```sh
   pmset -g assertions | grep -iE "PreventUserIdleSystemSleep|PreventSystemSleep"
   # the only non-zero line may be powerd's "display is on"; it drops when the lid shuts
   pgrep -x caffeinate            # expect no output
   ```
3. Keep normal platform security on (SIP, Gatekeeper, FileVault as usual). Do
   not disable them for the test.

## The control run (do this first, it is mandatory)

Proves this Mac sleeps under the same idle conditions with klipa inactive.

```sh
cd ~/git/klipa
scripts/physical-test/baseline.sh                 # writes ./keep-awake-run-*/baseline.txt
scripts/physical-test/recorder.sh "" 10 &         # no assertions taken
# Quit klipa entirely. Set a short idle sleep (e.g. 2 min). Close the lid.
# Wait > that timeout. Reopen, bring the recorder to foreground, Ctrl-C.
scripts/physical-test/analyze.sh
```
Expected: a sleep-sized gap in the heartbeat (the Mac slept) and
`foreign assertions: 0`. If it did NOT sleep, an inhibitor is still active;
fix pre-flight and rerun. Without a passing control run, no case below counts.

## The cases

Start the recorder once per case and leave it running:
`scripts/physical-test/recorder.sh "" 10 &`. Start the stated klipa session
from the menu, do the physical actions, then `analyze.sh` (and
`restore-check.sh` for stop/expiry cases). For each, note: model, macOS
build, installed artifact SHA256, helper version (menu > Copy diagnostics),
starting `pmset -g`, elapsed, and pass/fail. Detailed steps per case are in
`scripts/physical-test/README.md`.

## Full acceptance matrix

Minimum to publish the direct release: M1, M2, C1, C3, C5, C8 (the core
"stays awake + restores" proof). The rest raise confidence and are tracked
honestly as UNTESTED until run, per the agreed matrix (not silently reduced
to three cases).

| # | Case | Priority | Status | Evidence |
|---|---|---|---|---|
| M1 | Control run: Mac sleeps with klipa OFF (lid shut) | must | UNTESTED | |
| C1 | Closed lid, no external display, AC, >= 30 min | must | UNTESTED | |
| C2 | Closed lid, no external display, battery, >= 30 min (charge > 50%) | should | UNTESTED | |
| C3 | Charger connect/disconnect while shut, >= 5 each way (highest risk) | must | UNTESTED | |
| C4 | Repeated lid cycles, >= 10 | should | UNTESTED | |
| C5 | Short timed session (3-5 min), lid shut, unattended expiry restores | must | UNTESTED | |
| C6 | Timed session with the menu held open past expiry (daemon owns expiry) | should | UNTESTED | |
| C7 | Indefinite session soak, >= 2 h, lid shut | should | UNTESTED | |
| C8 | Stop / restore from menu; then turn off passwordless mode | must | UNTESTED | |
| C9 | Session extension while shut (change duration mid-session) | should | UNTESTED | |
| C10 | Duration/mode change while shut updates in place (no protection drop) | should | UNTESTED | |
| C11 | Normal quit restores sleep | should | UNTESTED | |
| C12 | Force-quit: daemon restores within the lease (~1 min) without relaunch | should | UNTESTED | |
| C13 | Helper crash / failed restore is retried autonomously | nice | UNTESTED | |
| C14 | Reboot / logout during a session: reconciled on next boot | should | UNTESTED | |
| C15 | Real in-app update old->new while a session is active restores first | should | UNTESTED | |
| M2 | Battery closed-lid behavior recorded (pass or documented limitation) | must | UNTESTED | |

Mark each PASS / FAIL / UNTESTED. A case is PASS only when the heartbeat is
continuous while shut (no gap > ~2x interval), `SleepDisabled` held 1 for
keep-awake cases (or returned to 0 for restore cases), foreign assertions
stayed 0, and the `pmset -g log` sleep/wake evidence corroborates (no
DarkWake masquerading as work). Correlate the heartbeat gaps with the
`pmset -g log` section that `analyze.sh` prints.

## After you run it

Leave the `keep-awake-run-*` directories in `~/git/klipa` (they are
git-ignored and persist across quitting the coding session). Tell the agent
they are ready; the agent reads each run's `heartbeat.tsv`, `baseline.txt`,
and the analysis, correlates them with the sleep/wake log, fills this matrix
with PASS/FAIL and the evidence, and only then proceeds to publish. The agent
will not declare a pass it has not inspected.
