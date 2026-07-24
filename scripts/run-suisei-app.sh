#!/usr/bin/env bash
# Package Suisei.app (incremental) and launch via `open`.
# Fast path: skips cargo/swiftc when sources are unchanged.
# Full rebuild: SUISEI_FORCE=1 ./scripts/run-suisei-app.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

"$ROOT/scripts/package-suisei-app.sh"

APP="$ROOT/suisei-app/.build/Suisei.app"
echo "→ open $APP"
# -n: new instance; shell returns after launch
exec open -n "$APP"
