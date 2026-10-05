#!/bin/sh
# Remove everything install.sh added, and make sure sleep is restored.
set -eu

case "$(uname -s)" in Darwin) ;; *) echo "macOS-only." >&2; exit 1 ;; esac
if [ "$(id -u)" -eq 0 ]; then
    echo "Run as your normal user; it will prompt for admin when needed." >&2
    exit 1
fi

APP_SCRIPTS="$HOME/Library/Application Scripts/dev.peterdsp.klipa"
SUPPORT="$HOME/Library/Application Support/dev.peterdsp.klipa"
AGENT_LABEL="dev.peterdsp.klipa.powerprotect"
AGENT_DST="$HOME/Library/LaunchAgents/$AGENT_LABEL.plist"
SUDOERS_DST="/etc/sudoers.d/dev.peterdsp.klipa"
TOGGLE="$APP_SCRIPTS/klipa-powerprotect"

echo "==> restoring normal sleep"
if [ -x "$TOGGLE" ]; then
    "$TOGGLE" off >/dev/null 2>&1 || true
fi

echo "==> unloading the LaunchAgent"
launchctl bootout "gui/$(id -u)/$AGENT_LABEL" 2>/dev/null || launchctl unload "$AGENT_DST" 2>/dev/null || true
rm -f "$AGENT_DST"

echo "==> removing user scripts and session state"
rm -f "$TOGGLE" "$SUPPORT/klipa-powerprotect-watchdog" "$SUPPORT/powerprotect.session"
rmdir "$APP_SCRIPTS" 2>/dev/null || true

echo "==> removing the sudoers rule (you will be asked for your password)"
sudo rm -f "$SUDOERS_DST"

echo "Done. Power Protect removed; normal sleep restored."
