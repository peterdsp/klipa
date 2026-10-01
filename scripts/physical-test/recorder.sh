#!/usr/bin/env bash
# A local heartbeat/work-progress recorder for the keep-awake physical gate.
#
# It takes NO power assertions of its own (no caffeinate, no IOKit assertion,
# no pmset change), so it cannot itself keep the Mac awake and bias the test.
# It simply appends a timestamped line on a fixed interval. If the Mac sleeps
# (or this process is suspended) the wall-clock gap between consecutive lines
# jumps well past the interval, which analyze.sh detects. It also samples the
# charger state and the count of foreign wake-preventing assertions so a
# "stayed awake" result can be attributed correctly (real work vs dark wake vs
# a competing inhibitor).
#
# Usage: scripts/physical-test/recorder.sh [run-dir] [interval-seconds]
#   run-dir  defaults to the newest ./keep-awake-run-* (or a new one)
#   interval defaults to 10 seconds
#
# Stop with Ctrl-C (or `kill` the printed PID). Leave it running across the
# lid-closed / charger-transition actions.

set -euo pipefail

RUN_DIR="${1:-$(ls -dt ./keep-awake-run-* 2>/dev/null | head -1 || echo ./keep-awake-run-$(date +%Y%m%d-%H%M%S))}"
INTERVAL="${2:-10}"
mkdir -p "$RUN_DIR"
LOG="$RUN_DIR/heartbeat.tsv"

if [ ! -f "$LOG" ]; then
  printf 'epoch\tiso_time\tmono_uptime_s\tac_power\tforeign_assertions\tsleepdisabled\n' >"$LOG"
fi

echo "recorder PID $$ writing to $LOG every ${INTERVAL}s (no assertions taken)"
echo "leave this running; stop with Ctrl-C when the case is done"

while true; do
  EPOCH="$(date +%s)"
  ISO="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  # Monotonic-ish seconds since boot; does NOT advance while the Mac is asleep,
  # so a stall here next to an advancing wall clock is a sleep fingerprint.
  MONO="$(awk '{print $1}' /proc/uptime 2>/dev/null || sysctl -n kern.boottime 2>/dev/null | sed -E 's/.*sec = ([0-9]+).*/\1/' | awk -v now="$EPOCH" '{print now-$1}')"
  # AC vs battery without taking an assertion.
  if pmset -g batt 2>/dev/null | grep -q "AC Power"; then AC=1; else AC=0; fi
  # Count wake-preventing assertions NOT held by klipa, so a pass can be
  # attributed to klipa rather than a stray caffeinate/Amphetamine.
  FOREIGN="$(pmset -g assertions 2>/dev/null | grep -iE "PreventUserIdleSystemSleep|PreventSystemSleep" | grep -ivE "klipa" | grep -cvE "^\s*0\s" || true)"
  SD="$(pmset -g 2>/dev/null | awk '/SleepDisabled/{print $2; found=1} END{if(!found) print "?"}')"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$EPOCH" "$ISO" "$MONO" "$AC" "$FOREIGN" "$SD" >>"$LOG"
  sleep "$INTERVAL"
done
