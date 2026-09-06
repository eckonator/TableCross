#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Installs TableCross from the unpacked Linux archive for the current user: programs to
# ~/.local/bin, menu entry and icon to ~/.local/share.
#   ./install.sh              install (run again to update)
#   ./install.sh --uninstall  remove again (settings stay)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS="$DATA/applications"
ICONS="$DATA/icons/hicolor"

refresh() {
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS" || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$ICONS" || true
}

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$BIN/tablecross" "$BIN/tablecross-cli" "$APPS/tablecross.desktop" "$ICONS/256x256/apps/tablecross.png"
  refresh
  echo "TableCross removed."
  exit 0
fi

mkdir -p "$BIN" "$APPS" "$ICONS/256x256/apps"
install -m755 "$HERE/tablecross" "$BIN/tablecross"
install -m755 "$HERE/tablecross-cli" "$BIN/tablecross-cli"
install -m644 "$HERE/tablecross.png" "$ICONS/256x256/apps/tablecross.png"
# Exec with the full path: ~/.local/bin is not in every desktop's search path
sed "s|^Exec=.*|Exec=\"$BIN/tablecross\"|" "$HERE/tablecross.desktop" > "$APPS/tablecross.desktop"
chmod 644 "$APPS/tablecross.desktop"
refresh

echo "TableCross installed: $BIN/tablecross"
echo "Start it from the application menu or with: tablecross"
echo "Allow UDP port 24800 in the firewall if one is active (for example: sudo ufw allow 24800/udp)."
