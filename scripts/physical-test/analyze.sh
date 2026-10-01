#!/usr/bin/env bash
# Analyze a recorder log plus the system sleep/wake log for one keep-awake
# case. Read-only. Reports the largest heartbeat gap (a gap well past the
# recording interval means the Mac slept or the recorder was suspended),
# distinguishes dark wake from full wake using the unified log, and summarizes
# charger transitions seen during the run.
#
# Usage: scripts/physical-test/analyze.sh [run-dir] [interval-seconds]

set -euo pipefail

RUN_DIR="${1:-$(ls -dt ./keep-awake-run-* 2>/dev/null | head -1)}"
INTERVAL="${2:-10}"
LOG="$RUN_DIR/heartbeat.tsv"

if [ ! -f "$LOG" ]; then
  echo "no heartbeat log at $LOG" >&2
  exit 1
fi

echo "# analysis for $RUN_DIR"
echo

# --- heartbeat gaps -----------------------------------------------------
echo "## heartbeat continuity (interval ${INTERVAL}s)"
awk -v interval="$INTERVAL" '
  NR==1 { next }                     # header
  { if (prev != "") {
      gap = $1 - prev
      if (gap > maxgap) { maxgap = gap; at = prevt }
      if (gap > interval * 2) {
        printf "  gap %ds at %s -> %s\n", gap, prevt, $2
        gaps++
      }
      total += gap; n++
    }
    prev = $1; prevt = $2
  }
  END {
    printf "  samples: %d\n", n+1
    if (n > 0) printf "  largest gap: %ds (near %s)\n", maxgap, at
    printf "  suspicious gaps (> %ds): %d\n", interval*2, gaps+0
    if (gaps+0 == 0) print "  VERDICT: continuous; no sleep-sized gap in the heartbeat"
    else print "  VERDICT: at least one sleep-sized gap; the Mac likely slept (see sleep/wake log below)"
  }
' "$LOG"
echo

# --- charger transitions seen -------------------------------------------
echo "## charger transitions during the run (from the heartbeat ac_power column)"
awk -F'\t' '
  NR==1 { next }
  { if (prev != "" && $4 != prev) printf "  %s AC=%s -> %s\n", $2, prev, $4; prev = $4 }
' "$LOG" || true
echo

# --- SleepDisabled during the run ---------------------------------------
echo "## SleepDisabled flag across the run"
awk -F'\t' 'NR==1{next} {c[$6]++} END{for (k in c) printf "  SleepDisabled=%s : %d samples\n", k, c[k]}' "$LOG"
echo

# --- foreign assertions (bias check) ------------------------------------
echo "## foreign wake-preventing assertions (should be 0 for an attributable pass)"
awk -F'\t' 'NR==1{next} {if ($5+0 > max) max=$5} END{printf "  max foreign assertions seen: %d\n", max+0; if (max+0>0) print "  WARNING: a non-klipa inhibitor was active; a pass is not attributable to klipa"}' "$LOG"
echo

# --- system sleep/wake log (bounded window) -----------------------------
echo "## system sleep/wake events (pmset -g log, last 40 relevant lines)"
pmset -g log 2>/dev/null \
  | grep -iE "Sleep |Wake |DarkWake|Entering Sleep|Lid" \
  | tail -40 | sed 's/^/  /' || echo "  (pmset -g log unavailable)"
echo
echo "Note: DarkWake periods are NOT uninterrupted work. If the heartbeat"
echo "shows gaps that line up with DarkWake/Wake entries, the case FAILED."
