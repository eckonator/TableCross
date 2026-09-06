#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Installs the latest TableCross into /Applications (macOS).
#
#   curl -fsSL https://raw.githubusercontent.com/eckonator/TableCross/main/scripts/install-macos.sh | bash
#
# What curl downloads carries no quarantine flag, so the app starts without a Gatekeeper prompt even
# though it is not notarized; the flag is removed anyway to be sure.
set -euo pipefail

REPO="eckonator/TableCross"
APP_NAME="TableCross.app"
# TC_INSTALL_DIR: other target (testing), TC_NO_OPEN=1: do not start it afterwards
DEST="${TC_INSTALL_DIR:-/Applications}"

# Everything in a function, so an interrupted download (curl | bash) runs nothing half-way
main() {
  say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
  fail() { printf '\033[31mError:\033[0m %s\n' "$*" >&2; exit 1; }

  [ "$(uname -s)" = "Darwin" ] || fail "This installer is for macOS. Other systems: https://github.com/$REPO/releases"

  if pgrep -f "$DEST/$APP_NAME/Contents/MacOS/TableCross" >/dev/null 2>&1; then
    fail "TableCross is running. Please quit it first (menu bar icon → Quit), then run this again."
  fi

  say "Looking up the latest release"
  local api url version
  api=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest") \
    || fail "Could not reach GitHub. Download manually: https://github.com/$REPO/releases"
  url=$(printf '%s' "$api" | grep -o '"browser_download_url": *"[^"]*-macOS\.zip"' | head -n1 | sed 's/.*"\(https[^"]*\)"/\1/')
  [ -n "$url" ] || fail "No macOS download found in the latest release: https://github.com/$REPO/releases"
  version=$(printf '%s' "$api" | grep -o '"tag_name": *"[^"]*"' | head -n1 | sed 's/.*"\([^"]*\)"$/\1/')

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT

  say "Downloading TableCross $version"
  curl -fL --progress-bar "$url" -o "$tmp/tablecross.zip" || fail "Download failed."
  ditto -x -k "$tmp/tablecross.zip" "$tmp/unpacked" || fail "Could not unpack the download."
  [ -d "$tmp/unpacked/$APP_NAME" ] || fail "The download does not contain $APP_NAME."

  local sudo=""
  if [ ! -w "$DEST" ] || { [ -e "$DEST/$APP_NAME" ] && [ ! -w "$DEST/$APP_NAME" ]; }; then
    say "Administrator password needed to write to $DEST"
    sudo="sudo"
  fi

  say "Installing to $DEST/$APP_NAME"
  $sudo rm -rf "$DEST/$APP_NAME"
  $sudo ditto "$tmp/unpacked/$APP_NAME" "$DEST/$APP_NAME"
  $sudo xattr -dr com.apple.quarantine "$DEST/$APP_NAME" 2>/dev/null || true

  if [ -n "${TC_NO_OPEN:-}" ]; then
    say "Done."
  else
    say "Done. Starting TableCross"
    open "$DEST/$APP_NAME"
  fi
}

main "$@"
