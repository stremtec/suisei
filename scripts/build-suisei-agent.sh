#!/usr/bin/env bash
# Build the menu-bar agent (SuiseiDaemonAgent.app) — a headless LSUIElement
# SwiftUI app that shows the daemon's status in the menu bar. Standalone: it
# talks to the daemon over the Unix socket and links only system frameworks
# (no engine dylib, no bridging header).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AGENT="$ROOT/suisei-agent"
APP="$AGENT/.build/SuiseiDaemonAgent.app"
MACOS="$APP/Contents/MacOS"
RES="$APP/Contents/Resources"

export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
SDKROOT="${SDKROOT:-$DEVELOPER_DIR/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk}"
MACOS_TARGET="${MACOS_TARGET:-arm64-apple-macos26.0}"
MACOS_MIN="${MACOS_TARGET##*macos}"
OPT="${SUISEI_OPT:--O}"
[[ "${SUISEI_FAST:-0}" == "1" ]] && OPT="-Onone"

rm -rf "$APP"
mkdir -p "$MACOS" "$RES"

echo "→ swiftc → SuiseiDaemonAgent (sdk=$(basename "$SDKROOT"), $MACOS_TARGET, $OPT)"
swiftc "$OPT" \
  -sdk "$SDKROOT" \
  -target "$MACOS_TARGET" \
  -o "$MACOS/SuiseiDaemonAgent" \
  "$AGENT"/Sources/*.swift

cp -f "$AGENT/Info.plist" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy \
  -c "Set :LSMinimumSystemVersion $MACOS_MIN" \
  "$APP/Contents/Info.plist"
cp -f "$AGENT/Resources/StatusIcon.png" "$RES/StatusIcon.png"

# Ad-hoc sign so macOS lets it launch without a developer identity.
codesign --force --sign - "$APP" >/dev/null 2>&1 || true

echo "→ built $APP"
