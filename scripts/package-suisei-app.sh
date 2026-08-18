#!/usr/bin/env bash
# Build engine + Swift face into a real macOS .app (required for reliable launch).
# Incremental by default: only rebuilds what changed. Force full rebuild with SUISEI_FORCE=1.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

FORCE="${SUISEI_FORCE:-0}"
PROFILE="${SUISEI_PROFILE:-release}"

# Swift optimisation level. Measured on the original tree (11 files, ~11.3k lines):
#   -O        3:59
#   -Onone    0:27   ← 9x
# Do NOT reach for `-enable-batch-mode` to parallelise single-file compilation:
# combined with
# `-import-objc-header` the frontends report success but emit no object files,
# and the build dies at link with "no such file or directory: ContentView-1.o".
# It looks 8% faster only because a build that never links is a build that did
# less work.
#
# Release instead uses Swift's supported whole-module mode. Without it the
# driver repeats optimisation for every primary file; after the native
# workbench grew the face to 22 files, EngineBridge.swift alone took 17 minutes
# and the driver then started over for ContentView.swift. WMO type-checks and
# optimises the module once, gives the optimiser cross-file visibility, and
# lets the LLVM backend use `-num-threads` safely. This is distinct from the
# broken batch-mode experiment above and matches SwiftPM's release defaults.
#
# `-O` stays the default because the editor canvas draws its text in Swift
# (EditorHost) — an unoptimised build is a bad place to judge input latency.
# Set SUISEI_FAST=1 for a throwaway UI-iteration build.
if [[ "${SUISEI_FAST:-0}" == "1" ]]; then
  SWIFT_OPT="-Onone"
  SWIFT_COMPILE_FLAGS=()
else
  SWIFT_OPT="${SUISEI_OPT:--O}"
  if [[ -n "${SUISEI_SWIFT_THREADS:-}" ]]; then
    SWIFT_THREADS="$SUISEI_SWIFT_THREADS"
  elif command -v sysctl >/dev/null 2>&1; then
    SWIFT_THREADS="$(sysctl -n hw.activecpu 2>/dev/null || echo 4)"
  else
    SWIFT_THREADS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
  fi
  SWIFT_COMPILE_FLAGS=(-whole-module-optimization -num-threads "$SWIFT_THREADS")
fi
# `${arr[*]}` on an EMPTY array is an unbound variable under `set -u` in the
# bash 3.2 macOS ships, so `SUISEI_FAST=1` — which is the branch that leaves
# this array empty — failed before it compiled anything. The documented
# fast-iteration build had never run.
SWIFT_BUILD_MODE="$SWIFT_OPT ${SWIFT_COMPILE_FLAGS[*]:-}"

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
  "$ROOT/suisei-app/Suisei/EditorTickStore.swift"
  "$ROOT/suisei-app/Suisei/EditorHost.swift"
  "$ROOT/suisei-app/Suisei/EditorDiagnostics.swift"
  "$ROOT/suisei-app/Suisei/QuickHelpPopover.swift"
  "$ROOT/suisei-app/Suisei/PaneViewers.swift"
  "$ROOT/suisei-app/Suisei/TerminalSurface.swift"
  "$ROOT/suisei-app/Suisei/AudioViewer.swift"
  "$ROOT/suisei-app/Suisei/ImagePDFViewers.swift"
  "$ROOT/suisei-app/Suisei/ModelScene.swift"
  "$ROOT/suisei-app/Suisei/ModelViewer.swift"
  "$ROOT/suisei-app/Suisei/ModelWorkbench.swift"
  "$ROOT/suisei-app/Suisei/FBXScene.swift"
  "$ROOT/suisei-app/Suisei/DebugPanel.swift"
  "$ROOT/suisei-app/Suisei/LogicView.swift"
  "$ROOT/suisei-app/Suisei/ProjectPane.swift"
  "$ROOT/suisei-app/Suisei/DatatipCard.swift"
  "$ROOT/suisei-app/Suisei/MetalTextRenderer.swift"
  "$ROOT/suisei-app/Suisei/TabStripLayout.swift"
  "$ROOT/suisei-app/Suisei/TabStripModel.swift"
  "$ROOT/suisei-app/Suisei/TabStripHost.swift"
  "$ROOT/suisei-app/Suisei/TabChipMetrics.swift"
  "$ROOT/suisei-app/Suisei/GlassBackdrop.swift"
  "$ROOT/suisei-app/Suisei/WelcomeView.swift"
  "$ROOT/suisei-app/Suisei/ProjectTreeView.swift"
  "$ROOT/suisei-app/Suisei/ProjectIndex.swift"
  "$ROOT/suisei-app/Suisei/SettingsWindowView.swift"
  "$ROOT/suisei-app/Suisei/GitHubAccount.swift"
  "$ROOT/suisei-app/Suisei/SoftwareUpdate.swift"
  "$ROOT/suisei-app/Suisei/GitWorkbenchWindowView.swift"
  "$ROOT/suisei-app/Suisei/AboutPanel.swift"
  "$ROOT/suisei-app/Suisei/GlassChrome.swift"
  "$ROOT/suisei-app/Suisei/WindowChrome.swift"
  "$ROOT/suisei-app/Suisei/DaemonLauncher.swift"
  "$ROOT/suisei-app/Suisei/PerfProbe.swift"
  "$ROOT/suisei-app/Suisei/AnimationTrace.swift"
  "$ROOT/suisei-app/Suisei/SidebarTrace.swift"
)

need_engine=0
need_swift=0
need_icon=0
need_swiftterm=0

DYLIB_SRC="$ROOT/target/$PROFILE/libsuisei_engine.dylib"
DYLIB_STAGE="$STAGE/libsuisei_engine.dylib"
BIN="$MACOS/Suisei"
HDR="$ROOT/suisei-engine/include/suisei_engine.h"
OPT_STAMP="$BUILD/.swift-opt"
# Vendored SwiftTerm — the terminal emulator and its AppKit view. Built once
# into a static library and linked like any other; see third_party/SwiftTerm.
# Vendored GLTFKit2 — the glTF reader. macOS cannot read glTF at all (measured:
# SCNScene refuses a minimal valid .gltf AND .glb, and there is no glTF
# framework on the system), and glTF is the game-asset pipeline's interchange
# format. It is the published binary rather than a source build; see
# third_party/GLTFKit2/VENDOR.md for why.
GLTF_DIR="$ROOT/third_party/GLTFKit2"

# Vendored Assimp — the FBX reader, static, FBX importer only. Nothing on the
# system reads FBX and Autodesk's own SDK is commercial; see
# third_party/Assimp/VENDOR.md. Its module map is upstream's own, which is why
# the module is `libassimp` and not a name of ours.
ASSIMP_DIR="$ROOT/third_party/Assimp"

ST_DIR="$ROOT/third_party/SwiftTerm"
ST_BUILD="$ST_DIR/.build/release"
ST_LIB="$STAGE/libSwiftTerm.a"

if [[ "$FORCE" == "1" ]]; then
  need_engine=1
  need_swift=1
  need_icon=1
  need_swiftterm=1
  rm -rf "$APP"
  mkdir -p "$MACOS" "$FW" "$RES"
else
  # SwiftTerm: any vendored source newer than the staged archive? It is pinned
  # and changes only when someone re-vendors it, so this is nearly always a
  # no-op — but a stale archive next to updated sources is a silent wrong build.
  if [[ ! -f "$ST_LIB" ]]; then
    need_swiftterm=1
  else
    while IFS= read -r f; do
      if [[ "$f" -nt "$ST_LIB" ]]; then need_swiftterm=1; break; fi
    done < <(find "$ST_DIR/Sources" "$ST_DIR/Package.swift" -type f 2>/dev/null)
  fi

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
    for f in "${SWIFT_FILES[@]}" "$HDR" "$DYLIB_STAGE" "$ST_LIB" \
             "$GLTF_DIR/GLTFKit2.framework/Versions/A/GLTFKit2" \
             "$ASSIMP_DIR/libassimp.a"; do
      if [[ -e "$f" && "$f" -nt "$BIN" ]]; then need_swift=1; break; fi
    done
  fi

  # Opt level changed since the last build? No mtime can see that, and a stale
  # -Onone binary passing itself off as a release is exactly the kind of trap
  # the embedded-dylib one already cost us an afternoon on.
  if [[ ! -f "$OPT_STAMP" || "$(cat "$OPT_STAMP" 2>/dev/null)" != "$SWIFT_BUILD_MODE" ]]; then
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
  elif [[ -d "$HOME/Desktop/suisei2.icon" ]] && [[ "$HOME/Desktop/suisei2.icon/icon.json" -nt "$ICON_ICNS" ]]; then
    need_icon=1
  elif [[ -f "$ROOT/suisei-app/Resources/Suisei.icon/icon.json" && "$ROOT/suisei-app/Resources/Suisei.icon/icon.json" -nt "$ICON_ICNS" ]]; then
    need_icon=1
  elif [[ -f "$ICON_SCRIPT" && "$ICON_SCRIPT" -nt "$ICON_ICNS" ]]; then
    need_icon=1
  fi
fi

if [[ "$need_swiftterm" == "1" ]]; then
  echo "→ SwiftTerm (vendored, pinned — see third_party/SwiftTerm/VENDOR.md)"
  ( cd "$ST_DIR" && swift build -c release --disable-automatic-resolution >/dev/null )
  mkdir -p "$STAGE"
  # SwiftPM builds a library target to objects and links them into products; it
  # emits no archive of its own, and `swiftc` needs one to link against.
  libtool -static -o "$ST_LIB" "$ST_BUILD"/SwiftTerm.build/*.o 2>/dev/null
  # The Metal shaders travel as a SwiftPM resource bundle. The GPU renderer
  # probes for it by name and falls through when it is absent, so this is not
  # load-bearing — but shipping it is what lets that path work at all.
  if [[ -d "$ST_BUILD/SwiftTerm_SwiftTerm.bundle" ]]; then
    rm -rf "$RES/SwiftTerm_SwiftTerm.bundle"
    cp -R "$ST_BUILD/SwiftTerm_SwiftTerm.bundle" "$RES/"
  fi
else
  echo "→ SwiftTerm up-to-date (skip)"
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
MACOS_MIN="${MACOS_TARGET##*macos}"

if [[ "$need_swift" == "1" ]]; then
  echo "→ swiftc → Contents/MacOS/Suisei (sdk=$(basename "$SDKROOT"), $MACOS_TARGET, $SWIFT_BUILD_MODE)"
  if [[ "$SWIFT_OPT" == "-Onone" ]]; then
    echo "   ⚠︎  UNOPTIMIZED build — fast to compile, slower at runtime."
    echo "      Never judge editor latency or ship a release from this."
  fi
  # Compile to temp then move — avoids partial binary on failure
  TMP_BIN=$(mktemp /tmp/Suisei.XXXXXX)
  swiftc "$SWIFT_OPT" ${SWIFT_COMPILE_FLAGS[@]+"${SWIFT_COMPILE_FLAGS[@]}"} \
    -parse-as-library \
    -sdk "$SDKROOT" \
    -target "$MACOS_TARGET" \
    "${SWIFT_FILES[@]}" \
    -framework SwiftUI \
    -framework AppKit \
    -framework Combine \
    -F "$GLTF_DIR" \
    -framework GLTFKit2 \
    -Xcc -I"$ASSIMP_DIR/include" \
    -Xcc -fmodule-map-file="$ASSIMP_DIR/include/assimp/module.modulemap" \
    "$ASSIMP_DIR/libassimp.a" \
    -lz \
    -lc++ \
    -import-objc-header "$STAGE/suisei_engine.h" \
    -I "$ST_BUILD/Modules" \
    "$ST_LIB" \
    -L "$STAGE" \
    -lsuisei_engine \
    -Xlinker -rpath -Xlinker @executable_path/../Frameworks \
    -o "$TMP_BIN"
  mv -f "$TMP_BIN" "$BIN"
  chmod +x "$BIN"
  printf '%s' "$SWIFT_BUILD_MODE" > "$OPT_STAMP"

  echo "→ embed GLTFKit2.framework"
  rm -rf "$FW/GLTFKit2.framework"
  cp -R "$GLTF_DIR/GLTFKit2.framework" "$FW/"

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
SUISEI_GIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo 416ad08)"
SUISEI_BUILD_NAME="Suisei2026dev${SUISEI_GIT}"
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
  <key>NSHumanReadableCopyright</key>
  <string>Copyright © 2026 Stremtec. All rights reserved.</string>
  <key>CFBundleVersion</key>
  <string>${SUISEI_BUILD_NAME}</string>
  <key>SuiseiBuildName</key>
  <string>${SUISEI_BUILD_NAME}</string>
  <key>LSMinimumSystemVersion</key>
  <string>${MACOS_MIN}</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
  <key>CFBundleIconFile</key>
  <string>Suisei</string>
  <key>CFBundleIconName</key>
  <string>Suisei</string>
  <!-- Bundled display fonts (Welcome wordmark: Milker). -->
  <key>ATSApplicationFontsPath</key>
  <string>Fonts</string>
  <!-- project.suiseiprj gets its own icon in Finder.

       Declared as an EXPORTED type because Suisei defines this format; an
       imported declaration is for reading somebody else's. The identifier is
       reverse-DNS and permanent — Launch Services caches it, and changing it
       later orphans every marker already on disk.

       It conforms to public.json, which is true (the file is JSON) and is what
       lets Quick Look preview one without Suisei installed. -->
  <key>UTExportedTypeDeclarations</key>
  <array>
    <dict>
      <key>UTTypeIdentifier</key>
      <string>com.stemtec.suisei.project</string>
      <key>UTTypeDescription</key>
      <string>Suisei Project</string>
      <key>UTTypeConformsTo</key>
      <array>
        <string>public.json</string>
        <string>public.data</string>
      </array>
      <key>UTTypeIconFile</key>
      <string>SuiseiProject</string>
      <key>UTTypeTagSpecification</key>
      <dict>
        <key>public.filename-extension</key>
        <array>
          <string>suiseiprj</string>
        </array>
      </dict>
    </dict>
  </array>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key>
      <string>Suisei Project</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>com.stemtec.suisei.project</string>
      </array>
      <key>CFBundleTypeIconFile</key>
      <string>SuiseiProject</string>
      <!-- Editor, not Viewer: opening one opens the project it marks. -->
      <key>CFBundleTypeRole</key>
      <string>Editor</string>
      <key>LSHandlerRank</key>
      <string>Owner</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Presented from the native About panel. Keep the shipped text identical to
# the repository license rather than maintaining a second UI-only copy.
cp -f "$ROOT/LICENSE" "$RES/LICENSE"
# SwiftTerm is MIT and its notice has to travel with the binary that contains
# it. Named for what it covers, beside our own licence rather than replacing it.
cp -f "$ROOT/third_party/SwiftTerm/LICENSE" "$RES/LICENSE-SwiftTerm"
# GLTFKit2 is MIT for the same reason.
cp -f "$ROOT/third_party/GLTFKit2/LICENSE" "$RES/LICENSE-GLTFKit2"
# Assimp is BSD-3 and its notice has to travel too.
cp -f "$ROOT/third_party/Assimp/LICENSE" "$RES/LICENSE-Assimp"

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
# Document icon for project.suiseiprj. Rendered from the app icon package's own
# knot, so the document and the app cannot drift apart.
PRJ_ICNS="$ROOT/suisei-app/Resources/SuiseiProject.icns"
PRJ_SVG="$ROOT/suisei-app/Resources/SuiseiProject.svg"
PRJ_RENDER="$ROOT/scripts/render_project_icon.swift"
if [[ ! -f "$PRJ_ICNS" || "$PRJ_SVG" -nt "$PRJ_ICNS" || "$PRJ_RENDER" -nt "$PRJ_ICNS" ]]; then
  PRJ_SET="$(mktemp -d)/SuiseiProject.iconset"
  if swift "$PRJ_RENDER" "$PRJ_SVG" "$PRJ_SET" >/dev/null 2>&1; then
    iconutil -c icns "$PRJ_SET" -o "$PRJ_ICNS" || true
  fi
  rm -rf "$PRJ_SET"
fi
[[ -f "$PRJ_ICNS" ]] && cp -f "$PRJ_ICNS" "$RES/SuiseiProject.icns"

if [[ -f "$ICON_SRC_DIR/Suisei.icns" ]]; then
  cp -f "$ICON_SRC_DIR/Suisei.icns" "$RES/Suisei.icns"
fi
# Optional flat masters / package for reference (not Dock source of truth on macOS 26+)
for f in Suisei.png Suisei-dark.png Suisei-mono.png WelcomeHero.jpg; do
  [[ -f "$ICON_SRC_DIR/$f" ]] && cp -f "$ICON_SRC_DIR/$f" "$RES/$f" || true
done
# Bundled fonts (Welcome wordmark etc.).
if [[ -d "$ICON_SRC_DIR/Fonts" ]]; then
  rm -rf "$RES/Fonts"
  mkdir -p "$RES/Fonts"
  find "$ICON_SRC_DIR/Fonts" -maxdepth 1 -type f \( -iname '*.otf' -o -iname '*.ttf' -o -iname '*.ttc' \) \
    -exec cp -f {} "$RES/Fonts/" \;
  echo "→ Fonts: $(ls "$RES/Fonts" 2>/dev/null | wc -l | tr -d ' ') files"
fi
# Launch-art rotation pool (one random image per app start).
if [[ -d "$ICON_SRC_DIR/WelcomeHeroes" ]]; then
  rm -rf "$RES/WelcomeHeroes"
  mkdir -p "$RES/WelcomeHeroes"
  # Only ship real image files (skip .DS_Store / notes).
  find "$ICON_SRC_DIR/WelcomeHeroes" -maxdepth 1 -type f \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' \) \
    -exec cp -f {} "$RES/WelcomeHeroes/" \;
  echo "→ WelcomeHeroes: $(ls "$RES/WelcomeHeroes" | wc -l | tr -d ' ') images"
fi
if [[ -d "$ICON_SRC_DIR/Suisei.icon" ]]; then
  rm -rf "$RES/Suisei.icon"
  cp -R "$ICON_SRC_DIR/Suisei.icon" "$RES/Suisei.icon"
fi

echo -n "APPL????" > "$CONTENTS/PkgInfo"

# ── Helpers: the durable daemon + its menu-bar agent ────────────────────────
# Suisei.app spawns the daemon (detached) on launch; the daemon launches the
# agent. Both live in Contents/Helpers so the app can find them by bundle path,
# and `codesign --deep` below signs them in place.
HELPERS="$CONTENTS/Helpers"
mkdir -p "$HELPERS"
echo "→ build + bundle daemon"
cargo build --release -p suisei-daemon --manifest-path "$ROOT/Cargo.toml" >/dev/null 2>&1 \
  && cp -f "$ROOT/target/release/suisei-daemon" "$HELPERS/suisei-daemon" \
  || echo "  ⚠︎ daemon build failed — status agent will be unavailable"
echo "→ build + bundle menu-bar agent"
if "$ROOT/scripts/build-suisei-agent.sh" >/dev/null 2>&1; then
  rm -rf "$HELPERS/SuiseiDaemonAgent.app"
  cp -R "$ROOT/suisei-agent/.build/SuiseiDaemonAgent.app" "$HELPERS/SuiseiDaemonAgent.app"
else
  echo "  ⚠︎ agent build failed — menu-bar status will be unavailable"
fi

# Info.plist, PkgInfo, resources and helpers are refreshed even on an
# incremental build. Any one of those writes invalidates the outer bundle
# signature, so signing cannot be conditional on the main binary/dylib.
if command -v codesign >/dev/null; then
  codesign --force --deep --sign - "$APP" 2>/dev/null || true
fi
touch "$APP"

echo "→ packaged $APP  (engine=$need_engine swift=$need_swift icon=$need_icon)"
if [[ "$need_swift" == "1" ]]; then
  otool -L "$BIN" | head -6
fi
echo "done."
