#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds dist/TableCross.app (app, system helper and command line tool inside).
# Usage: scripts/bundle-macos.sh   (TC_SKIP_BUILD=1 uses binaries already in target/release)
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP="dist/TableCross.app"

if [ -z "${TC_SKIP_BUILD:-}" ]; then
  cargo build --release -p tablecross -p tablecross-helper -p tablecross-cli
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/tablecross "$APP/Contents/MacOS/TableCross"
# The app installs the helper from right next to itself
cp target/release/tablecross-helper "$APP/Contents/MacOS/tablecross-helper"
cp target/release/tablecross-cli "$APP/Contents/MacOS/tablecross-cli"
cp assets/TableCross.icns "$APP/Contents/Resources/TableCross.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>TableCross</string>
  <key>CFBundleDisplayName</key><string>TableCross</string>
  <key>CFBundleIdentifier</key><string>de.eckonator.tablecross</string>
  <key>CFBundleExecutable</key><string>TableCross</string>
  <key>CFBundleIconFile</key><string>TableCross</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>NSHumanReadableCopyright</key><string>© 2026 Markus Eckert · GPL-3.0-or-later</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <!-- Menu bar app: no Dock icon, not in ⌘Tab -->
  <key>LSUIElement</key><true/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>TableCross connects to your other computers on the local network to share mouse, keyboard and clipboard.</string>
  <key>NSBonjourServices</key>
  <array><string>_tablecross._udp</string></array>
</dict>
</plist>
PLIST

# Ad-hoc signature, so Apple silicon Macs start it without a developer account
codesign --force --sign - "$APP/Contents/MacOS/tablecross-helper" >/dev/null 2>&1
codesign --force --sign - "$APP/Contents/MacOS/tablecross-cli" >/dev/null 2>&1
codesign --force --sign - --identifier de.eckonator.tablecross "$APP" >/dev/null 2>&1
echo "Done: $APP"
