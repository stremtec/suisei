# Suisei (Swift face)

macOS Renderer for Suisei. **Rust owns Core · Compositor · Bridge** (`suisei-engine`).  
This app only paints `FrameDiff` / chrome snapshots and forwards input.

## Layout

```
suisei-app/
  Suisei/
    SuiseiApp.swift      # @main
    ContentView.swift    # glass hello chrome
    EngineBridge.swift   # C ABI → Swift
  README.md
```

## Run (correct way)

```bash
# From repo root — packages Suisei.app and launches it
./scripts/run-suisei-app.sh
```

Or:

```bash
./scripts/package-suisei-app.sh
open suisei-app/.build/Suisei.app
```

**Do not** double-click the bare binary `suisei-app/.build/Suisei` — it is not a macOS app bundle (dylib/rpath/activation will fail). Always use **`Suisei.app`**.

Engine-only:

```bash
./scripts/build-suisei-engine.sh release
```

## Rules

- Do **not** keep a document buffer in Swift.
- Do **not** implement Vim keys in Swift — only map `NSEvent` → `suisei_engine_dispatch_key`.
- Editor glyph blit (Metal) lands in S2; S1 is chrome snapshot only.
