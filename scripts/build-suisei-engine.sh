#!/usr/bin/env bash
# Build libsuisei_engine.dylib for the Swift face.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PROFILE="${1:-release}"
echo "→ cargo build -p suisei-engine --$PROFILE"
cargo build -p suisei-engine "--$PROFILE"

OUT="$ROOT/target/$PROFILE"
echo "→ dylib: $OUT/libsuisei_engine.dylib"
ls -la "$OUT"/libsuisei_engine.* 2>/dev/null || ls -la "$OUT"/libsuisei_engine.dylib

# Stage for Swift link
STAGE="$ROOT/suisei-app/.engine"
mkdir -p "$STAGE"
cp -f "$OUT/libsuisei_engine.dylib" "$STAGE/"
cp -f "$ROOT/suisei-engine/include/suisei_engine.h" "$STAGE/"
# install_name for local load
if command -v install_name_tool >/dev/null; then
  install_name_tool -id "@rpath/libsuisei_engine.dylib" "$STAGE/libsuisei_engine.dylib" || true
fi
echo "→ staged in $STAGE"
echo "done."
