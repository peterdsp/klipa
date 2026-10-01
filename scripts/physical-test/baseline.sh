#!/usr/bin/env bash
# Capture a read-only power/hardware baseline for the keep-awake physical
# gate. Writes a timestamped file under the run directory. Changes nothing on
# the host. Keeps clipboard contents and identifying data out of the output
# (serial numbers and the computer name are redacted).
#
# Usage: scripts/physical-test/baseline.sh [run-dir]
#   run-dir defaults to ./keep-awake-run-<timestamp>

set -euo pipefail

RUN_DIR="${1:-./keep-awake-run-$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$RUN_DIR"
OUT="$RUN_DIR/baseline.txt"

redact() {
  # Drop lines that carry a serial number, UUID, or the machine name.
  sed -E \
    -e 's/(Serial Number[^:]*:).*/\1 [redacted]/I' \
    -e 's/(Hardware UUID:).*/\1 [redacted]/I' \
    -e 's/(Provisioning UDID:).*/\1 [redacted]/I'
}

{
  echo "# keep-awake baseline"
  echo "captured_at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "boot_session_uuid: $(sysctl -n kern.bootsessionuuid 2>/dev/null || echo unknown)"
  echo
  echo "## hardware"
  # Model id + chip only; no serial.
  sysctl -n hw.model 2>/dev/null | sed 's/^/model: /'
  uname -m | sed 's/^/arch: /'
  echo
  echo "## macOS"
  sw_vers | sed 's/^/  /'
  echo
  echo "## power settings (pmset -g)"
  pmset -g 2>&1 | sed 's/^/  /'
  echo
  echo "## custom power settings (pmset -g custom)"
  pmset -g custom 2>&1 | sed 's/^/  /'
  echo
  echo "## active power assertions (pmset -g assertions)"
  pmset -g assertions 2>&1 | sed 's/^/  /'
  echo
  echo "## battery / charger (pmset -g batt)"
  pmset -g batt 2>&1 | sed 's/^/  /'
  echo
  echo "## displays (physical vs virtual)"
  system_profiler SPDisplaysDataType 2>/dev/null | redact | sed 's/^/  /' | head -60
  echo
  echo "## other keep-awake inhibitors to rule out"
  echo "  (anything below with a PID that is not klipa/klipa-helper may mask a sleep failure)"
  pmset -g assertions 2>&1 | grep -iE "PreventUserIdle|PreventSystemSleep|caffeinate|amphetamine" | sed 's/^/  /' || true
  echo
  echo "## installed klipa"
  if [ -d /Applications/klipa.app ]; then
    /usr/bin/defaults read /Applications/klipa.app/Contents/Info CFBundleShortVersionString 2>/dev/null | sed 's/^/  version: /' || true
    codesign -dv --verbose=2 /Applications/klipa.app 2>&1 | grep -E "Identifier|TeamIdentifier|Authority" | sed 's/^/  /' || true
  else
    echo "  /Applications/klipa.app not present"
  fi
} | tee "$OUT"

echo
echo "baseline written to: $OUT"
echo "Keep this file private; it is not meant for the repo."
