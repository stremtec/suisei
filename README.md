# Suisei (彗星)

A native macOS GUI code editor with Xcode-grade design and behaviour.

Swift face over a Rust engine: the UI is SwiftUI/AppKit, all editor logic lives
in `suisei-core` (headless) and is bridged to Swift through the `suisei-engine`
C ABI. No Electron, no web view for the editor itself.

```
suisei-app/     SwiftUI + AppKit face (the .app)
suisei-engine/  Rust cdylib — Core host, compositor, C ABI bridge
suisei-core/    Rust — buffers, syntax (tree-sitter), LSP/DAP, git, terminal
```

Suisei began as a GUI-first fork of the `xei` terminal editor and is now an
independent project. It shares no code with `xei` at build time.

## Build & run

```bash
# Fast, unoptimised build (UI iteration — do NOT judge input latency on it)
SUISEI_FAST=1 ./scripts/run-suisei-app.sh

# Optimised build (default; ~4 min, the real thing)
./scripts/run-suisei-app.sh
```

The Rust engine builds with plain `cargo build --release`. The packaging script
compiles the Swift face, embeds the engine dylib, and produces a signed `.app`
under `suisei-app/.build/`.

Requires: macOS 26+, Xcode toolchain (swiftc), a Rust toolchain.

## Documentation

- `docs/SUISEI-CURRENT-STATE.md` — where the project stands and the ordered path
  to an independently usable IDE.
- `docs/SUISEI-ARCHITECTURE-PLAN.md` — daemon, docking, resize, settings, theme,
  menus.
- `docs/SUISEI-TODO.md` — open bugs and the hard-won design notes (metaball
  chrome, Liquid Glass structure, resize coordinate spaces, build traps).
- `docs/SUISEI-TUI-RESIDUE.md` — what is still shaped like a terminal editor,
  measured, with the ordered plan to cut it.

## Status

Pre-release. `v0.1.0`, actively built. Editing, project navigation, terminal,
git, LSP completions/diagnostics/hover, and crash-recovery journaling work; the
selection model, daemon separation, and dockable panels are in progress.
