#!/usr/bin/env bash
# Build a Developer ID signed, notarized .pkg for DIRECT DOWNLOAD
# (outside the App Store). Drag-install to /Applications.
#
# Required env (your own Apple credentials — never commit these):
#   DEV_ID_APP   "Developer ID Application: Petros Dhespollari (TEAMID)"
#   DEV_ID_INST  "Developer ID Installer: Petros Dhespollari (TEAMID)"
# Optional, for notarization (recommended):
#   AC_PROFILE   notarytool keychain profile name, OR
#   AC_APPLE_ID / AC_TEAM_ID / AC_PASSWORD  (app-specific password)
#
# Output: dist/klipa-<version>-macos.pkg
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"

VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
APP="dist/klipa.app"
PKG="dist/klipa-${VERSION}-macos.pkg"

TARGET="${TARGET:-universal}" ./scripts/bundle-macos.sh

if [ -n "${DEV_ID_APP:-}" ]; then
  echo "==> codesign app (hardened runtime)"
  codesign --force --options runtime --timestamp \
    --entitlements packaging/macos/entitlements.plist \
    --sign "$DEV_ID_APP" "$APP"
  codesign --verify --strict --verbose=2 "$APP"
else
  echo "!! DEV_ID_APP unset — producing UNSIGNED app (Gatekeeper will block)." >&2
fi

echo "==> building component pkg"
mkdir -p dist
PKGROOT="$(mktemp -d)/root"; mkdir -p "$PKGROOT/Applications"
cp -R "$APP" "$PKGROOT/Applications/"

COMP="$(mktemp).pkg"
pkgbuild --root "$PKGROOT" --identifier dev.peterdsp.klipa \
  --version "$VERSION" --install-location / "$COMP"

if [ -n "${DEV_ID_INST:-}" ]; then
  productbuild --package "$COMP" --sign "$DEV_ID_INST" "$PKG"
else
  echo "!! DEV_ID_INST unset — producing UNSIGNED pkg." >&2
  productbuild --package "$COMP" "$PKG"
fi

if [ -n "${AC_PROFILE:-}" ] || [ -n "${AC_APPLE_ID:-}" ]; then
  echo "==> notarizing"
  if [ -n "${AC_PROFILE:-}" ]; then
    xcrun notarytool submit "$PKG" --keychain-profile "$AC_PROFILE" --wait
  else
    xcrun notarytool submit "$PKG" --apple-id "$AC_APPLE_ID" \
      --team-id "$AC_TEAM_ID" --password "$AC_PASSWORD" --wait
  fi
  xcrun stapler staple "$PKG"
fi

echo "Built $PKG"
