#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Packs dist/TableCross.app into a disk image with a link to Applications (drag to install).
# Usage: scripts/make-dmg.sh [target.dmg]   (run scripts/bundle-macos.sh first)
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT="${1:-dist/TableCross-$VERSION-macOS.dmg}"
APP="dist/TableCross.app"
[ -d "$APP" ] || { echo "Missing: $APP (run scripts/bundle-macos.sh first)" >&2; exit 1; }

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/TableCross.app"
ln -s /Applications "$STAGE/Applications"
cp README.md LICENSE "$STAGE/"
rm -f "$OUT"
hdiutil create -quiet -volname "TableCross $VERSION" -srcfolder "$STAGE" -format UDZO "$OUT"
echo "Done: $OUT"
