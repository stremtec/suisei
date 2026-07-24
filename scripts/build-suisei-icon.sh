#!/usr/bin/env bash
# Compile Icon Composer `.icon` the official way (actool → Assets.car + .icns).
#
# Official docs:
#   https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer
#
# Without Xcode project, still use actool on the .icon package directly:
#   actool Suisei.icon --compile OUT --app-icon Suisei --platform macosx ...
# That produces:
#   - Assets.car  (layered Liquid Glass stacks: default / dark / tinted)
#   - Suisei.icns (flattened fallback for older macOS)
#
# Do NOT re-rasterize glass with PIL — the system renders layers from Assets.car.
#
# Source priority:
#   1) $SUISEI_ICON_SRC or ~/Desktop/suiseiicon.icon
#   2) suisei-app/Resources/Suisei.icon
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RES="$ROOT/suisei-app/Resources"
ICON_PKG="$RES/Suisei.icon"
DESKTOP_ICON="${SUISEI_ICON_SRC:-$HOME/Desktop/suiseiicon.icon}"

mkdir -p "$RES"

if [[ -d "$DESKTOP_ICON" && -f "$DESKTOP_ICON/icon.json" ]]; then
  echo "→ sync Icon Composer package from $DESKTOP_ICON"
  rm -rf "$ICON_PKG"
  # actool uses the package basename as the icon name → must be Suisei.icon
  cp -R "$DESKTOP_ICON" "$ICON_PKG"
fi

if [[ ! -d "$ICON_PKG" || ! -f "$ICON_PKG/icon.json" ]]; then
  echo "error: missing $ICON_PKG/icon.json" >&2
  exit 1
fi

# Ensure folder is named Suisei.icon (CFBundleIconName = Suisei)
if [[ "$(basename "$ICON_PKG")" != "Suisei.icon" ]]; then
  echo "error: icon package must be named Suisei.icon, got $(basename "$ICON_PKG")" >&2
  exit 1
fi

OUT="$(mktemp -d /tmp/suisei-actoolXXXX)"
PLIST="$OUT/partial.plist"
echo "→ actool compile Suisei.icon (Liquid Glass stacks + fallback icns)"
set +e
xcrun actool "$ICON_PKG" \
  --compile "$OUT" \
  --output-format human-readable-text \
  --notices --warnings --errors \
  --output-partial-info-plist "$PLIST" \
  --app-icon Suisei \
  --include-all-app-icons \
  --enable-on-demand-resources NO \
  --development-region en \
  --target-device mac \
  --minimum-deployment-target 26.0 \
  --platform macosx \
  >"$OUT/actool.log" 2>&1
STATUS=$?
set -e

if [[ "$STATUS" -ne 0 ]] || [[ ! -f "$OUT/Assets.car" ]]; then
  echo "error: actool failed (status=$STATUS)" >&2
  tail -50 "$OUT/actool.log" >&2 || true
  rm -rf "$OUT"
  exit 1
fi

cp -f "$OUT/Assets.car" "$RES/Assets.car"
if [[ -f "$OUT/Suisei.icns" ]]; then
  cp -f "$OUT/Suisei.icns" "$RES/Suisei.icns"
fi

# Optional marketing stills (not used for Dock on macOS 26 — system renders stacks)
if [[ -f "$ROOT/scripts/render_suisei_icon.py" ]]; then
  echo "→ optional flat masters for docs/README (not Dock source of truth)"
  python3 "$ROOT/scripts/render_suisei_icon.py" "$ICON_PKG" "$RES" 2>/dev/null || true
fi

# Summary
echo "→ installed:"
ls -la "$RES/Assets.car" "$RES/Suisei.icns" 2>/dev/null || true
if command -v assetutil >/dev/null; then
  assetutil -I "$RES/Assets.car" 2>/dev/null | python3 -c '
import sys, json
d = json.load(sys.stdin)
stacks = [i for i in d if isinstance(i, dict) and i.get("AssetType") == "IconImageStack"]
groups = [i for i in d if isinstance(i, dict) and i.get("AssetType") == "IconGroup"]
print(f"  IconImageStack: {len(stacks)}  IconGroup: {len(groups)}")
for i in stacks:
    print(f"    stack appearance={i.get(\"Appearance\")}")
' 2>/dev/null || true
fi
if [[ -f "$PLIST" ]]; then
  echo "→ actool partial plist keys:"
  plutil -p "$PLIST" 2>/dev/null || cat "$PLIST"
fi

rm -rf "$OUT"
echo "done. Bundle CFBundleIconName / CFBundleIconFile should be: Suisei"
