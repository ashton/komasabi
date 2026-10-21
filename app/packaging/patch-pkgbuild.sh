#!/usr/bin/env bash
# Fixes up the PKGBUILD cargo-packager generates for its pacman package:
#   patch-pkgbuild.sh <PKGBUILD> <release-tag>
#
# - package() copies everything in $srcdir, which includes the source tarball
#   itself (makepkg links it there), so it would be installed at /. Copy only
#   the extracted usr/ tree.
# - The binary is already stripped, so skip makepkg's empty -debug package.
# - Download the tarball from the GitHub release, so the PKGBUILD works on its
#   own (e.g. as an AUR komasabi-bin package) instead of next to the tarball.
set -euo pipefail

pkgbuild=$1
tag=$2
repo=https://github.com/ashton/komasabi

sed -i \
  -e 's|cp -r "${srcdir}"/\* "${pkgdir}"/|cp -r "${srcdir}"/usr "${pkgdir}"/|' \
  -e 's|^options=(!lto)$|options=(!lto !debug)|' \
  -e "s|^source=(\"\\([^\"]*\\)\")\$|source=(\"\\1::$repo/releases/download/$tag/\\1\")|" \
  "$pkgbuild"

grep -q 'cp -r "${srcdir}"/usr' "$pkgbuild"
grep -q '^options=(!lto !debug)$' "$pkgbuild"
grep -q "^source=(\".*::$repo/releases/download/$tag/" "$pkgbuild"
