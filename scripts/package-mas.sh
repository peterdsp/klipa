#!/usr/bin/env bash
# Build a Mac App Store .pkg, ready to upload via Transporter / altool.
#
# Required env (from your Apple Developer account):
#   MAS_APP   "Apple Distribution: Petros Dhespollari (TEAMID)"
#   MAS_INST  "3rd Party Mac Developer Installer: Petros Dhespollari (TEAMID)"
#   TEAMID    your 10-char Apple Team ID
#   PROFILE   path to the App Store provisioning profile (.provisionprofile)
#
# Output: dist/klipa-<version>-mas.pkg  (upload, do NOT install locally)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"

: "${MAS_APP:?set MAS_APP signing identity}"
: "${MAS_INST:?set MAS_INST signing identity}"
: "${TEAMID:?set TEAMID}"

VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
APP="dist/klipa.app"
PKG="dist/klipa-${VERSION}-mas.pkg"
ENT="$(mktemp).mas.entitlements"
sed "s/TEAMID/${TEAMID}/g" packaging/macos/entitlements.mas.plist > "$ENT"

# `mas` compiles out sandbox-incompatible frontmost-window capture (and
# the licensing gate, since the store handles payment). `weather` stays
# on so the App Store build can still show temperature in the menu bar
# when the user opts in - the store permits outbound HTTP.
TARGET="universal" FEATURES="mas weather" NO_DEFAULT="1" ./scripts/bundle-macos.sh

if [ -n "${PROFILE:-}" ]; then
  cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"
fi

echo "==> codesign for App Store"
codesign --force --options runtime --timestamp \
  --entitlements "$ENT" --sign "$MAS_APP" "$APP"
codesign --verify --strict --verbose=2 "$APP"

echo "==> productbuild (App Store component)"
mkdir -p dist
productbuild --component "$APP" /Applications \
  --sign "$MAS_INST" "$PKG"

echo "Built $PKG"
echo "Upload with: xcrun altool --upload-app -f '$PKG' -t macos --apiKey <KEY> --apiIssuer <ISSUER>"
echo "        or:  Transporter.app"
