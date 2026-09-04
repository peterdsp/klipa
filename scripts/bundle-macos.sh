#!/usr/bin/env bash
# Build the klipa release binary and assemble klipa.app.
#
#   TARGET   cargo target triple (default: host). Pass
#            "universal" to build x86_64 + arm64 and lipo them.
#   FEATURES extra cargo features (e.g. "mas" for the App Store build).
#
# Output: dist/klipa.app
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET="${TARGET:-}"
FEATURES="${FEATURES:-}"
NO_DEFAULT="${NO_DEFAULT:-}"
APP="dist/klipa.app"
FEAT_FLAG=()
[ -n "$FEATURES" ] && FEAT_FLAG=(--features "$FEATURES")
[ -n "$NO_DEFAULT" ] && FEAT_FLAG+=(--no-default-features)

build_one() { cargo build --release -p klipa-ui --target "$1" ${FEAT_FLAG[@]+"${FEAT_FLAG[@]}"}; }
# The privileged helper is a separate, feature-less crate; never pass the
# app's feature flags (e.g. `mas`) to it.
build_one_helper() { cargo build --release -p klipa-helper --target "$1"; }

# Ship the privileged helper only in the direct-download build. The
# sandboxed App Store build (FEATURES contains `mas`) must not bundle a
# privileged helper: the sandbox forbids it and review would reject it.
case " $FEATURES " in
  *" mas "*) INCLUDE_HELPER=0 ;;
  *)         INCLUDE_HELPER=1 ;;
esac

echo "==> building binary"
if [ "$TARGET" = "universal" ]; then
  rustup target add x86_64-apple-darwin aarch64-apple-darwin >/dev/null 2>&1 || true
  build_one x86_64-apple-darwin
  build_one aarch64-apple-darwin
  mkdir -p target/universal/release
  lipo -create -output target/universal/release/klipa \
    target/x86_64-apple-darwin/release/klipa \
    target/aarch64-apple-darwin/release/klipa
  BIN="target/universal/release/klipa"
elif [ -n "$TARGET" ]; then
  build_one "$TARGET"; BIN="target/$TARGET/release/klipa"
else
  cargo build --release -p klipa-ui ${FEAT_FLAG[@]+"${FEAT_FLAG[@]}"}; BIN="target/release/klipa"
fi

VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"

echo "==> assembling $APP (version $VERSION)"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# Template the workspace version into CFBundleShortVersionString + CFBundleVersion
# so the committed Info.plist doesn't have to be kept in sync by hand.
sed -E \
    -e '/<key>CFBundleShortVersionString<\/key>/,/<\/string>/ s|<string>[^<]*</string>|<string>'"$VERSION"'</string>|' \
    -e '/<key>CFBundleVersion<\/key>/,/<\/string>/ s|<string>[^<]*</string>|<string>'"$VERSION"'</string>|' \
    packaging/macos/Info.plist > "$APP/Contents/Info.plist"
cp packaging/icons/klipa.icns          "$APP/Contents/Resources/klipa.icns"
cp "$BIN"                              "$APP/Contents/MacOS/klipa"
chmod +x "$APP/Contents/MacOS/klipa"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# ── Privileged helper (Option B: passwordless lid-closed keep-awake) ──
if [ "$INCLUDE_HELPER" = 1 ]; then
  echo "==> building privileged helper"
  if [ "$TARGET" = "universal" ]; then
    build_one_helper x86_64-apple-darwin
    build_one_helper aarch64-apple-darwin
    mkdir -p target/universal/release
    lipo -create -output target/universal/release/klipa-helper \
      target/x86_64-apple-darwin/release/klipa-helper \
      target/aarch64-apple-darwin/release/klipa-helper
    HELPER_BIN="target/universal/release/klipa-helper"
  elif [ -n "$TARGET" ]; then
    build_one_helper "$TARGET"; HELPER_BIN="target/$TARGET/release/klipa-helper"
  else
    cargo build --release -p klipa-helper; HELPER_BIN="target/release/klipa-helper"
  fi
  mkdir -p "$APP/Contents/Library/LaunchDaemons"
  cp "$HELPER_BIN" "$APP/Contents/MacOS/klipa-helper"
  chmod +x "$APP/Contents/MacOS/klipa-helper"
  cp packaging/macos/dev.peterdsp.klipa.helper.plist \
     "$APP/Contents/Library/LaunchDaemons/dev.peterdsp.klipa.helper.plist"
fi

echo "Built $APP"
