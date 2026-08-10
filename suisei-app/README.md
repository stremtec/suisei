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
- Editor text uses CoreText by default so AppKit owns shaping, responsive
  scrolling and text overlays. The experimental atlas renderer is available
  only with `SUISEI_RENDERER=metal` for profiling and renderer development.
