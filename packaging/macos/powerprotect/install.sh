#!/bin/sh
# klipa Power Protect installer (off-store, user-run).
#
# This is NOT installed by the Mac App Store app and is NOT part of the MAS
# bundle. Apple does not allow a sandboxed app to install the privileged
# pieces below, so you install them yourself, once, with a single admin
# authorization. After this, the sandboxed klipa app can keep your Mac awake
# with the lid closed on a bare laptop (no external display), by running the
# toggle script this installs, which elevates only the two exact
# `pmset -a disablesleep 0|1` commands you authorize here.
#
# What it installs (all reversible with uninstall.sh):
#   1. ~/Library/Application Scripts/dev.peterdsp.klipa/klipa-powerprotect
#      (the toggle script the sandboxed app may execute)
#   2. ~/Library/Application Support/dev.peterdsp.klipa/klipa-powerprotect-watchdog
#      (restores a timed session at its deadline, even behind a shut lid)
#   3. /etc/sudoers.d/dev.peterdsp.klipa  (NOPASSWD for ONLY those two pmset
#      commands, for your user; validated with visudo before install)
#   4. ~/Library/LaunchAgents/dev.peterdsp.klipa.powerprotect.plist
#      (the per-user agent that runs the watchdog)
#
# Usage:  sh install.sh        (run it yourself; it will ask for your password)
set -eu

case "$(uname -s)" in
    Darwin) ;;
    *) echo "klipa Power Protect is macOS-only." >&2; exit 1 ;;
esac
if [ "$(id -u)" -eq 0 ]; then
    echo "Run this as your normal user (not with sudo); it will prompt when it needs admin." >&2
    exit 1
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
USER_NAME="$(id -un)"
APP_SCRIPTS="$HOME/Library/Application Scripts/dev.peterdsp.klipa"
SUPPORT="$HOME/Library/Application Support/dev.peterdsp.klipa"
AGENTS="$HOME/Library/LaunchAgents"
SUDOERS_DST="/etc/sudoers.d/dev.peterdsp.klipa"
AGENT_LABEL="dev.peterdsp.klipa.powerprotect"
AGENT_DST="$AGENTS/$AGENT_LABEL.plist"
TOGGLE_DST="$APP_SCRIPTS/klipa-powerprotect"
WATCHDOG_DST="$SUPPORT/klipa-powerprotect-watchdog"

echo "==> installing user scripts"
mkdir -p "$APP_SCRIPTS" "$SUPPORT" "$AGENTS"
install -m 0755 "$HERE/klipa-powerprotect" "$TOGGLE_DST"
install -m 0755 "$HERE/klipa-powerprotect-watchdog" "$WATCHDOG_DST"

echo "==> preparing the sudoers rule (scoped to exactly two pmset commands)"
TMP_SUDOERS="$(mktemp)"
trap 'rm -f "$TMP_SUDOERS"' EXIT
sed "s/__USER__/$USER_NAME/g" "$HERE/klipa.sudoers.in" > "$TMP_SUDOERS"
# Validate BEFORE touching the system file, so a bad rule can never wedge sudo.
if ! /usr/sbin/visudo -cf "$TMP_SUDOERS" >/dev/null 2>&1; then
    echo "!! generated sudoers rule failed validation; aborting (nothing installed to /etc)." >&2
    exit 1
fi

echo "==> installing the sudoers rule and the LaunchAgent (you will be asked for your password)"
TMP_AGENT="$(mktemp)"
sed "s#__WATCHDOG__#$WATCHDOG_DST#g" "$HERE/dev.peterdsp.klipa.powerprotect.plist.in" > "$TMP_AGENT"
install -m 0644 "$TMP_AGENT" "$AGENT_DST"
rm -f "$TMP_AGENT"

# One privileged step: place the validated sudoers file as root:wheel 0440.
sudo install -m 0440 -o root -g wheel "$TMP_SUDOERS" "$SUDOERS_DST"
# Re-validate the installed file in place (defense in depth).
sudo /usr/sbin/visudo -cf "$SUDOERS_DST" >/dev/null

echo "==> loading the LaunchAgent"
launchctl bootout "gui/$(id -u)/$AGENT_LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$AGENT_DST" 2>/dev/null || launchctl load "$AGENT_DST" 2>/dev/null || true

echo
echo "Installed. Quick self-check (no sleep change made):"
"$TOGGLE_DST" status || true
echo
echo "Done. In the klipa App Store app, closed-lid keep-awake on a bare laptop"
echo "is now available. Remove everything later with: sh uninstall.sh"
