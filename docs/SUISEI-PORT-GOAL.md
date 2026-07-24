# Goal: Port every xei surface to Suisei

> **Historical port matrix; reconciled 2026-07-23.** Use `SUISEI-GAP.md` for
> the code-verified feature inventory and `SUISEI-CURRENT-STATE.md` for the
> ordered work. This document is retained for the original xei-parity scope.

> **Active goal** — execute phase by phase; never claim “done” until Phase D.  
> **Strategy:** gap-list driven — Core state → Compositor FrameDiff → Swift paint/input.  
> **Never:** reimplement Vim/buffer in Swift.  
> **Track:** `docs/SUISEI-GAP.md` (status) + this file (master matrix + phases).

## Definition of done (per surface)

1. Core owns the feature (or a small API in `suisei-core`; parity with
   `xei-core` is explicit because it is currently forked).  
2. `suisei-engine` exposes it in compose/FFI.  
3. Swift paints it from snapshot only.  
4. Engine unit test on real `Engine` / `App::dispatch` path.  
5. Gap row flips to `done` / `partial` honestly.

## Launch (always)

```bash
./scripts/run-suisei-app.sh
# → packages suisei-app/.build/Suisei.app and open(1)
```

Bare `suisei-app/.build/Suisei` is **not** supported.

## Master matrix

| ID | Surface | Suisei | Priority |
|----|---------|--------|----------|
| E1 | Editor text + modes | **done** | P0 |
| E2 | Click / drag select | **done** | P0 |
| E3 | Gutter git/fold/BP | **partial** (git stripe + breakpoint click; no fold glyph) | P1 |
| E4 | Soft-wrap + syntax | **partial** (syntax spans + visual chunks; scroll mapping incomplete) | P1 |
| E5 | Completions popup | **partial** | P1 |
| E6 | Multi-cursor paint | missing | P2 |
| C1 | Tab bar + click | **done** | P0 |
| C2 | Breadcrumbs | partial | P2 |
| C3 | Welcome | **done** | P0 |
| C4 | Status badges | **partial** (+branch) | P1 |
| C5 | Explorer | **done** (Swift tree, filter, git/index marks; polish remains) | P1 |
| C6 | Ext panel | missing | P2 |
| C7 | Terminal | **partial** (text PTY) | P0 |
| C8 | XLC | **partial** | P0 |
| C9 | Search bar | **done** | P0 |
| C10 | Palette | **done** | P0 |
| C11 | Which-key | **partial** | P1 |
| C12 | Settings + theme | **partial** (panel + live theme) | P1 |
| C13 | SCM panel | **partial** (status/graph snapshot and navigator) | P1 |
| C14 | Git workbench | **partial** (tabs/snapshots; richer operations remain) | P1 |
| C15 | Preview | **partial** (Markdown/JSON/plain panel) | P1 |
| C16 | Workspace search | **partial** (search/replace navigator) | P1 |
| C17 | DAP | missing | P1 |
| C18–C25 | Calls/PR/plugins/webview/context | missing | P2 |
| L1 | `.app` launch | **done** | P0 |
| U1 | Glass / blink caret | partial unlock | P1 |

## Phases

### Phase A — shell
- [x] L1 package + open  
- [x] E1/E2 editor + drag  
- [x] C1/C5/C8/C9/C10 tabs, explorer, XLC, search, palette  

### Phase B — daily driver (active)
- [x] C11 which-key popup (Core pending_hints)  
- [x] E5 completions popup (Core completions)  
- [x] C4 richer status (+ git branch when known)  
- [x] C7 terminal panel (plain-text PTY rows; color later)  
- [x] E4 syntax spans (tree-sitter via Core; paint coalesced runs)  
- [x] E3 git gutter stripe (added/mod/del from `app.git`)  
- [x] Render jank fix (no per-char views; tick paint only on frame_gen)  
- [x] Natural trackpad scroll (pixel accumulate → whole lines)  
- [ ] C7 terminal: full color + input focus polish  

### Phase C — IDE depth (next)
- [x] C12 settings panel (Ctrl+, / Cmd+, / ⚙) + live theme paint  
- [x] Liquid glass = macOS 26 `glassEffect(.regular)` (blur, light tint)  
- [x] C13 SCM panel (Ctrl+G) — staged/changes + commit graph  
- [x] C14 Git workbench face (Ctrl+Shift+G) — tabs Status/Branches/History/…  
- [x] Panel resize (explorer/SCM/terminal/XLC grips, persisted)
- [x] C16 workspace search / replace navigator
- [ ] C17 DAP  
- [ ] E4 soft-wrap (remaining)  
- [ ] E3 fold / breakpoint glyphs  

### Phase D — extensions & unlocks
- [ ] C21/C22 plugins + webview  
- [ ] Canvas mode  

## Rules

1. One surface per PR-sized chunk; test before next.  
2. Update this matrix + `SUISEI-GAP.md` when status changes.  
3. Prefer `App::dispatch` / existing methods over new face logic.
