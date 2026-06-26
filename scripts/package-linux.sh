#!/usr/bin/env bash
# Build all Linux artifacts for klipa:
#   - dist/klipa-<v>-linux-x86_64.tar.gz   (portable, with install.sh)
#   - dist/klipa_<v>_amd64.deb             (cargo-deb)
#   - dist/klipa-<v>-1.x86_64.rpm          (cargo-generate-rpm)
#   - dist/klipa-<v>-x86_64.AppImage       (appimagetool)
#
# Each step is skipped (with a notice) if its tool is missing, so the
# script still produces what it can. Run from anywhere; cd's to root.
#
# Build deps (Debian/Ubuntu):
#   libfontconfig1-dev libxkbcommon-dev libgl1-mesa-dev libxcb1-dev \
#   libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"
have() { command -v "$1" >/dev/null 2>&1; }

VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
mkdir -p dist

echo "==> cargo build --release"
cargo build --release -p klipa-ui
BIN="target/release/klipa"

# ── staging tree (shared by tarball + AppImage) ───────────────────────
STAGE="$(mktemp -d)/klipa"
install -Dm755 "$BIN" "$STAGE/usr/bin/klipa"
install -Dm644 packaging/linux/klipa.desktop "$STAGE/usr/share/applications/klipa.desktop"
install -Dm644 LICENSE "$STAGE/usr/share/doc/klipa/LICENSE"
for s in 16 32 48 64 128 256 512; do
  install -Dm644 "packaging/icons/hicolor/${s}x${s}/apps/klipa.png" \
    "$STAGE/usr/share/icons/hicolor/${s}x${s}/apps/klipa.png"
done

# ── tarball ───────────────────────────────────────────────────────────
echo "==> tarball"
cat > "$STAGE/install.sh" <<'EOF'
#!/usr/bin/env sh
# Copy klipa into your prefix (default /usr/local). Run with sudo for /usr.
set -e
PREFIX="${PREFIX:-/usr/local}"
cp -av usr/bin/klipa "$PREFIX/bin/klipa"
mkdir -p "$PREFIX/share/applications" "$PREFIX/share/icons"
cp -av usr/share/applications/klipa.desktop "$PREFIX/share/applications/"
cp -av usr/share/icons/hicolor "$PREFIX/share/icons/"
echo "klipa installed to $PREFIX. Run 'klipa' or find it in your launcher."
EOF
chmod +x "$STAGE/install.sh"
tar -C "$(dirname "$STAGE")" -czf "dist/klipa-${VERSION}-linux-x86_64.tar.gz" klipa
echo "   dist/klipa-${VERSION}-linux-x86_64.tar.gz"

# ── .deb ──────────────────────────────────────────────────────────────
if have cargo-deb; then
  echo "==> .deb"
  cargo deb -p klipa-ui --no-build --output "dist/klipa_${VERSION}_amd64.deb"
else
  echo "!! cargo-deb missing (cargo install cargo-deb) — skipping .deb" >&2
fi

# ── .rpm ──────────────────────────────────────────────────────────────
if have cargo-generate-rpm; then
  echo "==> .rpm"
  cargo generate-rpm -p crates/klipa-ui --output "dist/klipa-${VERSION}-1.x86_64.rpm"
else
  echo "!! cargo-generate-rpm missing (cargo install cargo-generate-rpm) — skipping .rpm" >&2
fi

# ── AppImage ──────────────────────────────────────────────────────────
if have appimagetool; then
  echo "==> AppImage"
  APPDIR="$(mktemp -d)/klipa.AppDir"
  cp -a "$STAGE/usr" "$APPDIR/usr"
  ln -sf usr/bin/klipa "$APPDIR/AppRun"
  cp packaging/linux/klipa.desktop "$APPDIR/klipa.desktop"
  cp packaging/icons/hicolor/256x256/apps/klipa.png "$APPDIR/klipa.png"
  ARCH=x86_64 appimagetool "$APPDIR" "dist/klipa-${VERSION}-x86_64.AppImage"
else
  echo "!! appimagetool missing — skipping AppImage" >&2
fi

echo "Done. Artifacts in dist/:"
ls -1 dist/ | sed 's/^/   /'
