#!/usr/bin/env bash
# Confirm that ending / expiring a keep-awake session restored normal sleep.
# Read-only. Compares the current SleepDisabled flag against the baseline and
# reports whether ordinary sleep is possible again. It does NOT force sleep.
#
# Usage: scripts/physical-test/restore-check.sh [run-dir]

set -euo pipefail

RUN_DIR="${1:-$(ls -dt ./keep-awake-run-* 2>/dev/null | head -1)}"
BASE="$RUN_DIR/baseline.txt"

now_flag() { pmset -g 2>/dev/null | awk '/SleepDisabled/{print $2; found=1} END{if(!found) print "?"}'; }

CUR="$(now_flag)"
echo "# restore check for $RUN_DIR"
echo "current SleepDisabled: $CUR"

if [ -f "$BASE" ]; then
  BASE_FLAG="$(grep -E "SleepDisabled" "$BASE" | head -1 | awk '{print $2}')"
  echo "baseline SleepDisabled: ${BASE_FLAG:-unknown}"
  if [ "$CUR" = "${BASE_FLAG:-0}" ]; then
    echo "VERDICT: restored to the baseline value"
  else
    echo "VERDICT: NOT restored to baseline (owned override may still be in effect)"
  fi
else
  echo "no baseline file; judging against the normal default (0 = sleep allowed)"
  [ "$CUR" = "0" ] && echo "VERDICT: sleep is allowed again" || echo "VERDICT: sleep still disabled"
fi

echo
echo "klipa-helper recovery journal (should be absent after a clean restore):"
if [ -e "/Library/Application Support/dev.peterdsp.klipa.helper/recovery.json" ]; then
  echo "  PRESENT: a restore is still owed or in progress (the daemon retries on its own)"
else
  echo "  absent: no owed restore"
fi
