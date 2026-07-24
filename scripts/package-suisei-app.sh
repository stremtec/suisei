#!/usr/bin/env bash
# Build engine + Swift face into a real macOS .app (required for reliable launch).
# Incremental by default: only rebuilds what changed. Force full rebuild with SUISEI_FORCE=1.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

FORCE="${SUISEI_FORCE:-0}"
PROFILE="${SUISEI_PROFILE:-release}"

# Swift optimisation level. Measured on this tree (11 files, ~11.3k lines):
#   -O        3:59
#   -Onone    0:27   ← 9x
# 89% of a build is the optimiser, and it runs on ONE of this machine's ten
# cores. Do NOT reach for `-enable-batch-mode` to parallelise it: combined with
# `-import-objc-header` the frontends report success but emit no object files,
# and the build dies at link with "no such file or directory: ContentView-1.o".
# It looks 8% faster only because a build that never links is a build that did
# less work.
#
# `-O` stays the default because the editor canvas draws its text in Swift
# (EditorHost) — an unoptimised build is a bad place to judge input latency.
# Set SUISEI_FAST=1 for a throwaway UI-iteration build.
if [[ "${SUISEI_FAST:-0}" == "1" ]]; then
  SWIFT_OPT="-Onone"
else
  SWIFT_OPT="${SUISEI_OPT:--O}"
fi

STAGE="$ROOT/suisei-app/.engine"
BUILD="$ROOT/suisei-app/.build"
APP="$BUILD/Suisei.app"
CONTENTS="$APP/Contents"
MACOS="$CONTENTS/MacOS"
FW="$CONTENTS/Frameworks"
RES="$CONTENTS/Resources"

mkdir -p "$MACOS" "$FW" "$RES"

SWIFT_FILES=(
  "$ROOT/suisei-app/Suisei/SuiseiApp.swift"
  "$ROOT/suisei-app/Suisei/ContentView.swift"
  "$ROOT/suisei-app/Suisei/EngineBridge.swift"
  "$ROOT/suisei-app/Suisei/EditorHost.swift"
  "$ROOT/suisei-app/Suisei/GlassBackdrop.swift"
  "$ROOT/suisei-app/Suisei/WelcomeView.swift"
  "$ROOT/suisei-app/Suisei/ProjectTreeView.swift"
  "$ROOT/suisei-app/Suisei/ProjectIndex.swift"
  "$ROOT/suisei-app/Suisei/SettingsWindowView.swift"
  "$ROOT/suisei-app/Suisei/GlassChrome.swift"
  "$ROOT/suisei-app/Suisei/WindowChrome.swift"
)

need_engine=0
need_swift=0
need_icon=0

DYLIB_SRC="$ROOT/target/$PROFILE/libsuisei_engine.dylib"
DYLIB_STAGE="$STAGE/libsuisei_engine.dylib"
BIN="$MACOS/Suisei"
HDR="$ROOT/suisei-engine/include/suisei_engine.h"
OPT_STAMP="$BUILD/.swift-opt"

if [[ "$FORCE" == "1" ]]; then
  need_engine=1
  need_swift=1
  need_icon=1
  rm -rf "$APP"
  mkdir -p "$MACOS" "$FW" "$RES"
else
  # Engine: any rust/header newer than staged dylib?
  if [[ ! -f "$DYLIB_STAGE" ]]; then
    need_engine=1
  else
    while IFS= read -r f; do
      if [[ "$f" -nt "$DYLIB_STAGE" ]]; then need_engine=1; break; fi
    done < <(find "$ROOT/suisei-engine" "$ROOT/suisei-core" -type f \( -name '*.rs' -o -name '*.toml' -o -name '*.h' \) 2>/dev/null)
  fi

  # Swift: sources or staged dylib/header newer than binary?
  if [[ ! -f "$BIN" ]]; then
    need_swift=1
  else
    for f in "${SWIFT_FILES[@]}" "$HDR" "$DYLIB_STAGE"; do
      if [[ -e "$f" && "$f" -nt "$BIN" ]]; then need_swift=1; break; fi
    done
  fi

  # Opt level changed since the last build? No mtime can see that, and a stale
  # -Onone binary passing itself off as a release is exactly the kind of trap
  # the embedded-dylib one already cost us an afternoon on.
  if [[ ! -f "$OPT_STAMP" || "$(cat "$OPT_STAMP" 2>/dev/null)" != "$SWIFT_OPT" ]]; then
    need_swift=1
  fi

  # Icon: rebuild if Composer package / script newer than bundled icns or Assets.car missing
  ICON_ICNS="$ROOT/suisei-app/Resources/Suisei.icns"
  ICON_CAR="$ROOT/suisei-app/Resources/Assets.car"
  ICON_SCRIPT="$ROOT/scripts/build-suisei-icon.sh"
  if [[ ! -f "$RES/Suisei.icns" || ! -f "$ICON_ICNS" ]]; then
    need_icon=1
  elif [[ ! -f "$ICON_CAR" || ! -f "$RES/Assets.car" ]]; then
    need_icon=1
  elif [[ -d "$HOME/Desktop/suiseiicon.icon" ]] && [[ "$HOME/Desktop/suiseiicon.icon/icon.json" -nt "$ICON_ICNS" ]]; then
    need_icon=1
  elif [[ -f "$ROOT/suisei-app/Resources/Suisei.icon/icon.json" && "$ROOT/suisei-app/Resources/Suisei.icon/icon.json" -nt "$ICON_ICNS" ]]; then
    need_icon=1
  elif [[ -f "$ICON_SCRIPT" && "$ICON_SCRIPT" -nt "$ICON_ICNS" ]]; then
    need_icon=1
  fi
fi

if [[ "$need_engine" == "1" ]]; then
  "$ROOT/scripts/build-suisei-engine.sh" "$PROFILE"
else
  echo "→ engine up-to-date (skip cargo)"
  # Keep stage header in sync, but only when content differs — a blind `cp`
  # bumps mtime and invalidates swiftc's bridging-header PCH mid-build.
  mkdir -p "$STAGE"
  if [[ ! -f "$STAGE/suisei_engine.h" ]] || ! cmp -s "$HDR" "$STAGE/suisei_engine.h"; then
    cp -f "$HDR" "$STAGE/suisei_engine.h"
  fi
  if [[ -f "$DYLIB_SRC" && ( ! -f "$DYLIB_STAGE" || "$DYLIB_SRC" -nt "$DYLIB_STAGE" ) ]]; then
    cp -f "$DYLIB_SRC" "$DYLIB_STAGE"
  fi
fi

export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
SDKROOT="${SDKROOT:-$DEVELOPER_DIR/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk}"
MACOS_TARGET="${MACOS_TARGET:-arm64-apple-macos26.0}"

if [[ "$need_swift" == "1" ]]; then
  echo "→ swiftc → Contents/MacOS/Suisei (sdk=$(basename "$SDKROOT"), $MACOS_TARGET, $SWIFT_OPT)"
  if [[ "$SWIFT_OPT" == "-Onone" ]]; then
    echo "   ⚠︎  UNOPTIMIZED build — fast to compile, slower at runtime."
    echo "      Never judge editor latency or ship a release from this."
  fi
  # Compile to temp then move — avoids partial binary on failure
  TMP_BIN=$(mktemp /tmp/Suisei.XXXXXX)
  swiftc "$SWIFT_OPT" \
    -parse-as-library \
    -sdk "$SDKROOT" \
    -target "$MACOS_TARGET" \
    "${SWIFT_FILES[@]}" \
    -framework SwiftUI \
    -framework AppKit \
    -framework Combine \
    -import-objc-header "$STAGE/suisei_engine.h" \
    -L "$STAGE" \
    -lsuisei_engine \
    -Xlinker -rpath -Xlinker @executable_path/../Frameworks \
    -o "$TMP_BIN"
  mv -f "$TMP_BIN" "$BIN"
  chmod +x "$BIN"
  printf '%s' "$SWIFT_OPT" > "$OPT_STAMP"

  echo "→ embed libsuisei_engine.dylib"
  cp -f "$STAGE/libsuisei_engine.dylib" "$FW/"
  if command -v install_name_tool >/dev/null; then
    install_name_tool -id "@rpath/libsuisei_engine.dylib" "$FW/libsuisei_engine.dylib" || true
    install_name_tool -change \
      "$STAGE/libsuisei_engine.dylib" \
      "@rpath/libsuisei_engine.dylib" \
      "$BIN" 2>/dev/null || true
    install_name_tool -add_rpath "@executable_path/../Frameworks" "$BIN" 2>/dev/null || true
  fi
else
  echo "→ swift up-to-date (skip swiftc)"
  # Still refresh dylib in Frameworks if engine rebuilt
  if [[ "$need_engine" == "1" || ! -f "$FW/libsuisei_engine.dylib" ]]; then
    cp -f "$STAGE/libsuisei_engine.dylib" "$FW/"
    if command -v install_name_tool >/dev/null; then
      install_name_tool -id "@rpath/libsuisei_engine.dylib" "$FW/libsuisei_engine.dylib" || true
    fi
  fi
fi

# Info.plist — always rewrite (tiny). Version = workspace Cargo version (single source).
SUISEI_VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
SUISEI_VERSION="${SUISEI_VERSION:-0.0.0}"
cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>Suisei</string>
  <key>CFBundleIdentifier</key>
  <string>com.stremtec.suisei</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Suisei</string>
  <key>CFBundleDisplayName</key>
  <string>Suisei</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${SUISEI_VERSION}</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>14.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
  <key>CFBundleIconFile</key>
  <string>Suisei</string>
  <key>CFBundleIconName</key>
  <string>Suisei</string>
</dict>
</plist>
PLIST

ICON_SRC_DIR="$ROOT/suisei-app/Resources"
if [[ "$need_icon" == "1" || ! -f "$RES/Suisei.icns" ]]; then
  "$ROOT/scripts/build-suisei-icon.sh"
else
  echo "→ icon up-to-date (skip)"
fi
# Official Icon Composer pipeline: Assets.car (Liquid Glass stacks) + Suisei.icns fallback.
# CFBundleIconName / CFBundleIconFile must both be "Suisei" (see Info.plist above).
if [[ -f "$ICON_SRC_DIR/Assets.car" ]]; then
  cp -f "$ICON_SRC_DIR/Assets.car" "$RES/Assets.car"
fi
if [[ -f "$ICON_SRC_DIR/Suisei.icns" ]]; then
  cp -f "$ICON_SRC_DIR/Suisei.icns" "$RES/Suisei.icns"
fi
# Optional flat masters / package for reference (not Dock source of truth on macOS 26+)
for f in Suisei.png Suisei-dark.png Suisei-mono.png; do
  [[ -f "$ICON_SRC_DIR/$f" ]] && cp -f "$ICON_SRC_DIR/$f" "$RES/$f" || true
done
if [[ -d "$ICON_SRC_DIR/Suisei.icon" ]]; then
  rm -rf "$RES/Suisei.icon"
  cp -R "$ICON_SRC_DIR/Suisei.icon" "$RES/Suisei.icon"
fi

echo -n "APPL????" > "$CONTENTS/PkgInfo"

# Sign only when binary/dylib changed (slow otherwise)
if [[ "$need_swift" == "1" || "$need_engine" == "1" || "$need_icon" == "1" ]]; then
  if command -v codesign >/dev/null; then
    codesign --force --deep --sign - "$APP" 2>/dev/null || true
  fi
  touch "$APP"
fi

echo "→ packaged $APP  (engine=$need_engine swift=$need_swift icon=$need_icon)"
if [[ "$need_swift" == "1" ]]; then
  otool -L "$BIN" | head -6
fi
echo "done."
