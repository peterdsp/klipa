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

echo "==> assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp packaging/macos/Info.plist          "$APP/Contents/Info.plist"
cp packaging/icons/klipa.icns          "$APP/Contents/Resources/klipa.icns"
cp "$BIN"                              "$APP/Contents/MacOS/klipa"
chmod +x "$APP/Contents/MacOS/klipa"
printf 'APPL????' > "$APP/Contents/PkgInfo"

echo "Built $APP"
