#!/usr/bin/env bash
# Refresh every package-manager manifest (Homebrew cask, Scoop, winget,
# AUR) to a given klipa version + the SHA-256 of its release artifacts.
#
#   scripts/update-package-managers.sh [VERSION] [SHA256SUMS]
#
# VERSION     defaults to the workspace version in Cargo.toml.
# SHA256SUMS  a local path or URL to the release's SHA256SUMS.txt;
#             defaults to the file attached to the GitHub release for
#             that version. (That file is produced by the release job.)
#
# The manifests embed the version in their download URLs, so this simply
# rewrites the old version -> new version in each file and swaps in each
# artifact's checksum. Commit the result (CI does this automatically).
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="peterdsp/klipa"
VERSION="${1:-$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
SUMS_SRC="${2:-https://github.com/$REPO/releases/download/v$VERSION/SHA256SUMS.txt}"

echo "==> klipa $VERSION (sums: $SUMS_SRC)"

# Load SHA256SUMS into a "<sha>  <file>" blob.
if [ -f "$SUMS_SRC" ]; then
  SUMS="$(cat "$SUMS_SRC")"
else
  SUMS="$(curl -fsSL "$SUMS_SRC")"
fi

# sha_of <artifact-filename> -> the 64-hex checksum from SHA256SUMS.txt.
sha_of() {
  local f="$1" sha
  sha="$(printf '%s\n' "$SUMS" | awk -v f="$1" '$2==f || $2=="*"f {print $1; exit}')"
  if [ -z "$sha" ]; then
    echo "!! no checksum for $f in SHA256SUMS.txt" >&2
    exit 1
  fi
  printf '%s' "$sha"
}

PKG_SHA="$(sha_of "klipa-$VERSION-macos.pkg")"
ZIP_SHA="$(sha_of "klipa-$VERSION-windows-x64.zip")"
EXE_SHA="$(sha_of "klipa-$VERSION-windows-x64-setup.exe")"
TGZ_SHA="$(sha_of "klipa-$VERSION-linux-x86_64.tar.gz")"

# bump <file> <new-sha>: rewrite the old semver to $VERSION everywhere
# (covers version fields and download URLs) and the placeholder/old
# checksum to <new-sha>. Portable in-place edit (BSD + GNU sed).
bump() {
  local file="$1" sha="$2" old esc_old
  old="$(grep -oE '[0-9]+\.[0-9]+\.[0-9]+' "$file" | head -1)"
  esc_old="$(printf '%s' "$old" | sed 's/\./\\./g')"
  sed -e "s/$esc_old/$VERSION/g" \
      -e "s/[0-9a-f]\{64\}/$sha/g" \
      "$file" > "$file.tmp"
  mv "$file.tmp" "$file"
  echo "   updated $file"
}

bump Casks/klipa.rb "$PKG_SHA"
bump bucket/klipa.json "$ZIP_SHA"
bump packaging/winget/dev.peterdsp.klipa.installer.yaml "$EXE_SHA"
bump packaging/aur/PKGBUILD "$TGZ_SHA"
bump packaging/aur/.SRCINFO "$TGZ_SHA"

# These winget files carry the version but no checksum (pass a dummy sha
# that won't match the 64-hex pattern in them).
sed -i.bak -E "s/(PackageVersion: ).*/\1$VERSION/" \
  packaging/winget/dev.peterdsp.klipa.yaml \
  packaging/winget/dev.peterdsp.klipa.locale.en-US.yaml
rm -f packaging/winget/*.bak
echo "   updated winget version + locale manifests"

echo "Done."
