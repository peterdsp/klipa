#!/usr/bin/env bash
# Generate all platform icon formats from assets/icon.svg.
#
#   macOS : packaging/icons/klipa.icns
#   Windows: packaging/icons/klipa.ico
#   Linux : packaging/icons/hicolor/<size>x<size>/apps/klipa.png
#   Tray  : packaging/icons/tray/<size>.png  (16/32/template)
#
# Requires: rsvg-convert (librsvg). macOS .icns also needs iconutil.
# Falls back gracefully when a tool is missing.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SVG="$ROOT/assets/icon.svg"
OUT="$ROOT/packaging/icons"
mkdir -p "$OUT" "$OUT/tray"

have() { command -v "$1" >/dev/null 2>&1; }

if ! have rsvg-convert; then
  echo "error: rsvg-convert not found (brew install librsvg / apt install librsvg2-bin)" >&2
  exit 1
fi

render() { rsvg-convert -w "$1" -h "$1" "$SVG" -o "$2"; }

echo "==> Linux hicolor PNGs"
for s in 16 24 32 48 64 128 256 512; do
  d="$OUT/hicolor/${s}x${s}/apps"
  mkdir -p "$d"
  render "$s" "$d/klipa.png"
done
cp "$OUT/hicolor/512x512/apps/klipa.png" "$OUT/klipa.png"

echo "==> macOS .icns"
ICONSET="$(mktemp -d)/klipa.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  render "$s"        "$ICONSET/icon_${s}x${s}.png"
  render "$((s*2))"  "$ICONSET/icon_${s}x${s}@2x.png"
done
if have iconutil; then
  iconutil -c icns "$ICONSET" -o "$OUT/klipa.icns"
else
  echo "   (iconutil missing - skipping .icns; build on macOS)" >&2
fi

echo "==> Windows .ico"
TMPICO="$(mktemp -d)"
for s in 16 24 32 48 64 128 256; do render "$s" "$TMPICO/$s.png"; done
if have magick;   then magick "$TMPICO"/{16,24,32,48,64,128,256}.png "$OUT/klipa.ico"
elif have convert; then convert "$TMPICO"/{16,24,32,48,64,128,256}.png "$OUT/klipa.ico"
elif have python3; then python3 "$ROOT/scripts/png2ico.py" "$OUT/klipa.ico" "$TMPICO"/{16,24,32,48,64,128,256}.png
else echo "   (no ico packer - skipping .ico)" >&2
fi

echo "==> Tray glyphs"
render 32 "$OUT/tray/32.png"
render 16 "$OUT/tray/16.png"

echo "Done -> $OUT"
