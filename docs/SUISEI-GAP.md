# Suisei ↔ xei feature gap inventory

> **Code-verified status: 2026-07-23.** This is the implementation inventory,
> not the product plan. When this conflicts with an older planning document,
> code and this file win. Statuses: `missing` · `partial` · `done` · `planned`.

## Reality check

| Area | Current implementation | Status |
|---|---|---|
| App boundary | SwiftUI/AppKit face → fixed C ABI → Rust engine → `suisei-core` | **done** |
| Text ownership | Rust `App` owns every buffer; Swift has decoded paint snapshots only | **done** |
| Editor renderer | `NSScrollView` + custom CoreText `NSView`; native pull bands, not `TextEditor` | **done** |
| IME / CJK | `NSTextInputClient`; single scalar fast path and multi-scalar paste path | **done** |
| GUI editing policy | Core remains modal, but Swift keeps normal document typing in Insert mode | **done** |
| Syntax | tree-sitter spans and project prewarm; no semantic-token paint | **partial** |
| Persistence | atomic file save and session/undo facilities; no crash-safe GUI recovery | **partial** |
| Process model | Engine and all unsaved state live in the GUI process | **partial** |
| IDE surfaces | Navigator, SCM, Git workbench, preview, diagnostics/search/breakpoints are painted | **partial** |
| Extension webview / canvas / multiwindow state sharing | Not implemented | **missing** |

## Editor and input

| Feature | Suisei implementation | Status |
|---|---|---|
| Open / save / Save As | Native panels → FFI; `suisei-core` writes via temp + fsync + rename | **done** |
| Editing, undo/redo, clipboard | Shared Core state machine and standard macOS shortcuts | **done** |
| Grapheme-safe left/right/delete | `unicode-segmentation` in `Buffer` | **done** |
| Click, double-click, drag selection | CoreText UTF-16 hit testing → Core visual columns | **done** |
| Split editors | Up to four ABI panes; vertical/horizontal splits and focused pane | **done** |
| Scroll / minimap / font zoom | Native scroll ownership, Core synchronisation, fractional scroll, pull bands | **done** |
| Soft wrap | Core emits wrapped visual chunks; native clip/scroll mapping is still line-oriented | **partial** |
| Syntax / git gutter / breakpoints | Syntax spans, git stripes and clickable breakpoint markers; no fold glyphs | **partial** |
| Selection model | One Core cursor + Visual anchor; GUI works around it | **partial** |
| Multi-cursor / rectangular gestures | No first-class plural selections | **missing** |
| Hover, definition, rename, format, code actions | FFI exists for hover/format/rename/actions; definition is not exposed and UI affordances are incomplete | **partial** |

## Shell and IDE surfaces

| Feature | Suisei implementation | Status |
|---|---|---|
| Welcome / recents / project open / clone | Separate Welcome window and native panels | **done** |
| Tabs, breadcrumbs, status, palette, find | Painted Swift shell with Core snapshots | **done** |
| Project navigator | Swift recursive tree, filter, git marks and Core file open | **done** |
| Project prewarm | Async discovery, then one-file-at-a-time engine parse; progress state exists | **partial** |
| Terminal | Side/full panel, ANSI SGR paint, focused routing, multiple parked sessions | **partial** |
| SCM / Git workbench | Navigator SCM and dock/full workbench are painted from FFI snapshots | **partial** |
| Preview | Markdown/JSON/plain preview snapshot and panel | **partial** |
| Workspace search / diagnostics / breakpoints | Working FFI and navigator UI | **partial** |
| Settings / themes | Separate Settings window plus Core-backed live theme and persistence | **done** |
| DAP, call hierarchy, PR review | Core has state, but Suisei has no complete interactive surface | **missing** |
| Extension panel, plugin store, live WKWebView | Not wired into the face | **missing** |
| Dockable/tear-off panels, canvas documents | Planned only | **missing** |

## Known architectural limits

- `suisei-core` is a **fork** of `xei-core`, not a currently shared crate. Keep
  parity intentional; do not describe changes as automatically shared.
- `Buffer` remains `Vec<String>` and `buffer.text()` rebuilds the document.
  Parser/highlight work still has synchronous paths, so large-file typing has
  a structural latency risk.
- The FFI uses fixed-size C structs (for example, packed editor rows and
  string/span limits). The canvas mitigates this with exact-range pull bands,
  but ABI evolution requires coordinated Rust, C header, and Swift decoder
  changes.
- CoreText is the current glyph renderer. Metal is a possible replacement, not
  a shipped requirement.
- A GUI crash can lose unsaved work. The daemon/WAL design is not implemented.

## Verification baseline

- `cargo test -p suisei-engine`: **47 passed** (2026-07-23).
- `SUISEI_FAST=1 ./scripts/package-suisei-app.sh`: Swift app compiled, linked
  to `@rpath/libsuisei_engine.dylib`, and passed `codesign --verify`.
- `suisei-core` integration tests that write `~/.xei/undo` or bind localhost
  cannot pass in the restricted analysis sandbox. They must be run in a normal
  developer environment before release.

## Source of truth

- Current implementation: `suisei-app/`, `suisei-engine/`, `suisei-core/`
- Product/architecture target: `SUISEI-PLAN.md`, `SUISEI-ARCHITECTURE-PLAN.md`
- Ordered independence roadmap: `SUISEI-CURRENT-STATE.md`
