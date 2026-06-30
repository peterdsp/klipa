#!/usr/bin/env bash
# Generate every platform icon from assets/icon.png (preferred) or
# assets/icon.svg (fallback).
#
#   macOS  : packaging/icons/klipa.icns
#   Windows: packaging/icons/klipa.ico
#   Linux  : packaging/icons/hicolor/<size>x<size>/apps/klipa.png
#   Tray   : packaging/icons/tray/<size>.png  (16/32 - currently unused at
#             runtime; the menubar template glyph is drawn in code)
#   Website: docs/assets/icon.png + favicon.png
#
# Requires one of these resamplers:
#   - sips        (built-in on macOS)
#   - magick/convert  (ImageMagick)
#   - rsvg-convert    (only works with SVG source)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PNG="$ROOT/assets/icon.png"
SVG="$ROOT/assets/icon.svg"
OUT="$ROOT/packaging/icons"
DOCS="$ROOT/docs/assets"
mkdir -p "$OUT" "$OUT/tray" "$DOCS"

have() { command -v "$1" >/dev/null 2>&1; }

# Pick a resampler and define a single `render <size> <out.png>` shim.
if [ -f "$PNG" ] && have sips; then
  SOURCE="png+sips"
  render() {
    # sips's -Z clamps the longest side; the icon is square so that's the size.
    sips -s format png -Z "$1" "$PNG" --out "$2" >/dev/null
  }
elif [ -f "$PNG" ] && have magick; then
  SOURCE="png+magick"
  render() { magick "$PNG" -resize "${1}x${1}" "$2"; }
elif [ -f "$PNG" ] && have convert; then
  SOURCE="png+convert"
  render() { convert "$PNG" -resize "${1}x${1}" "$2"; }
elif [ -f "$SVG" ] && have rsvg-convert; then
  SOURCE="svg+rsvg"
  render() { rsvg-convert -w "$1" -h "$1" "$SVG" -o "$2"; }
else
  echo "error: need assets/icon.png with sips/magick, or assets/icon.svg with rsvg-convert" >&2
  exit 1
fi
echo "==> using source: $SOURCE"

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

echo "==> Tray glyphs (legacy; runtime tray uses a code-drawn template)"
render 32 "$OUT/tray/32.png"
render 16 "$OUT/tray/16.png"

echo "==> Website (docs/assets)"
render 512 "$DOCS/icon.png"
render 256 "$DOCS/favicon.png"

echo "Done -> $OUT (+ $DOCS)"
