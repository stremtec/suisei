# Suisei — outstanding work

> **Current implementation snapshot (code-verified 2026-07-30).** The dated
> investigation notes farther down are retained for their failure analysis,
> but their “currently/no/never” wording describes the state at the date of
> that section. This snapshot and the tests are authoritative for current
> implementation status.
>
> | | Theme | Why it ranks here |
> |---|---|---|
> | **P0** | Release gates · ABI contract | Rust tests, ABI layout tests, rustfmt, optimised app packaging, codesign and a non-interactive launch smoke pass. A human open/edit/save/reopen pass remains; the monolithic Swift `-O` compile is also unusually slow. |
> | **P0** | Crash durability | Atomic saves and the shadow WAL recover dirty named buffers. Untitled buffers are not journalled, and authoritative editor state has not moved into the daemon. |
> | **P1** | Document storage phase 2 | Central `Edit`/`Delta`, versioned snapshots, delta undo application, incremental LSP sync and the async syntax worker are done. `Buffer` text storage is still `Vec<String>`; a rope/piece tree plus line index remains. |
> | **P1** | Native IDE workflows | Search, diagnostics, format/rename/actions and partial UI exist; definition/hover/DAP need end-to-end workflows. |
> | **P2** | Terminal IME · extensions · multiwindow | Terminal protocol depth is substantially improved. Terminal-pane IME, an extension host and daemon-owned multiwindow document state remain. |
> | **P3** | Canvas, Metal glyph renderer, elaborate transitions | Optional visual/interaction bets, not independence blockers. |
>
> **Next architecture patch:** replace only the document storage/index behind
> the tested `Edit`/`Delta` boundary. Do not combine that migration with another
> selection, daemon or ABI rewrite.


Current snapshot as of 2026-07-30. Companion to `SUISEI-CORE-DESIGN.md` (the
original rewrite blueprint). Completed work is kept where it explains an
important implementation trap; open status is explicitly labelled.

> **Architecture plan (2026-07-21):** daemon separation, panel docking /
> stand-alone windows, resize stabilisation, Settings rework, dark-theme colour
> revision and menu-bar role division are specified in
> `SUISEI-ARCHITECTURE-PLAN.md`. That document supersedes this one for those six
> workstreams; this file remains the register of open bugs and deferred work.
>
> **Resolved:** `suisei-core::fs_atomic::atomic_write_file` now implements
> sibling tmp → write → fsync → rename, and `App::save_file()` uses it. The
> shadow WAL recovers dirty named buffers. The remaining durability gaps are
> untitled buffers and daemon ownership of the live editor state.
>
> **Resolved 2026-07-29 (technical-debt pass, "Section A"):**
> - Terminal: DECSTBM scroll regions + IND/NEL/RI index escapes; OSC 0/2
>   titles on terminal tabs; xterm mouse reporting (?1000/1002/1003, SGR
>   1006) end-to-end (face → FFI → PTY); UTF-8-safe ABI string truncation;
>   span cap now marker-priority (caret/diagnostics survive dense syntax);
>   tab overflow surfaces as "+N" instead of silent loss; `Terminal: Drop`
>   reaps PTYs.
> - FFI/bridge: header declarations moved inside the include guard /
>   `extern "C"`; duplicate prototypes removed; retired `split_kind` /
>   `split_ratio` (pads, geometry lives in per-pane rects); the triplicated
>   Swift tab decode is one `decodeTabs`; id counter `wrapping_add` →
>   `checked_add` (the never-reused contract can no longer silently wrap).
> - Session: files + cursors restore on launch, save on quit; the split
>   layout (tree tokens + per-pane viewports + focus) persists too.
> - CI: `.github/workflows/ci.yml` (Rust workspace tests + app packaging on
>   a normal macOS lane — the undo-spill/DAP-localhost tests the restricted
>   sandbox cannot run).
> - Glass: regular/clear mixing removed (Welcome), `interactive(false)` on
>   static panes, `GlassEffectContainer` fuses the Welcome panes, the
>   glassPanel double shadow is gone, find bar migrated off `regularMaterial`.
> - xei residue: the dead `extensions` cfg (9 build warnings) and the
>   desktop pet + `term_caps_summary` removed.
> - Traffic lights: window-top-anchored placement (independent of the async
>   titlebar growth) + a 20 Hz self-healing guard + didBecomeKey re-assert.
> - A1 (first four units): central `Edit`/`Delta` + versioned snapshots
>   (`edit.rs`, `Buffer::apply_edit`, versioned `BufferSnapshot`); undo is
>   delta-recorded and applies through `apply_edit` (no snapshot restore;
>   spill format is deltas); LSP didChange is incremental (line-diffed
>   ranges, UTF-16 positions, full-sync fallback); the selection model is
>   unified — `MultiCursor` is gone, Ctrl+D / Ctrl+Alt+↑↓ add real
>   selections to `sel`, so every edit path applies to all of them.
> - A1 (fifth unit): `current_buffer` — the positional index into
>   `buffers` — is gone. The active tab is DERIVED: `current_buffer()`
>   computes it from the focused pane's document id (S2: `App`'s live
>   fields ARE that document); a private `live_doc: BufferId` records
>   which document `App` holds across the one statement of a focus change
>   where the two differ. Every index bookkeeping died with the field:
>   the `-= 1` after closes, the remap after moves, the repoint-after-
>   regather. The focused pane's slot names the live document from the
>   moment it becomes active (it used to stay stale by design until the
>   next park — an invisible state the compositor papered over). Also
>   fixes a latent bug: `toggle_terminal_full` never saved the displaced
>   document's tab slot, so switching away from the terminal tab could
>   restore a stale copy.
> - A1 (sixth unit): parsing is off the keystroke path. A syntax worker
>   thread (`syntax_worker.rs`) owns the tree-sitter parsers; the engine
>   ships a text snapshot per buffer version (`try_send`, never blocks)
>   and adopts finished frames at the next recompose or tick. A typing
>   burst coalesces to its newest snapshot on the worker; while a parse
>   is in flight the stale tokens keep painting (shifted a column for a
>   frame or two — the standard async-highlight contract). The prewarm
>   cache moved to the worker with the trees; grammar bundles compile
>   lazily on first use (the main-thread engine never parses, so startup
>   pays for nothing); a same-text scroll re-runs the highlight query
>   alone instead of forcing a full reparse (`edit_between` is None for
>   equal texts).
> - A3: the god object is decomposed. `app.rs` went 6897 → 5442 lines;
>   five domain modules now carry the clusters: `search.rs` (SearchState
>   + smart-case collect / nearest / step / cycle — the key handler
>   stopped poking fields and re-implementing match cycling inline),
>   `tabs.rs` (a `TabStrip` owning `buffers` + the id source, plus the
>   whole open/close/move/goto + save/restore orchestration),
>   `layouts.rs` (fold/unfold/activate/membership), `panes.rs` (splits,
>   focus park-then-load, pane-terminal lifecycle), `dap_cmds.rs`
>   (breakpoints, launch/attach, stepping, stopped-location). State
>   structs where they were clean (SearchState, TabStrip), `impl App`
>   domain files where the orchestration dominates — cross-module
>   collaboration is `pub(crate)`, nothing wider.
> - A4: the daemon owns per-editor state. One entry per connected
>   client (keyed by connection, wire format unchanged); the snapshot
>   AGGREGATES live editors — LSP sessions sum, states take the best,
>   the project follows the latest report — and a disconnect drops its
>   editor from the very next query instead of ghosting for the whole
>   TTL. The TTL stays as a stall guard for wedged-but-connected
>   editors. Two open windows no longer play last-writer-wins over one
>   slot. (Full D1 — the daemon owning the language servers themselves
>   — remains future work; the bookkeeping it needs is now in place.)
> - A6: the viewport is pixel-based. The core owns the stage in POINTS
>   (`App.stage` — w/h/cell_px/cell_w/dpr); `resize_stage` is the one
>   production writer and the cell grid is DERIVED (`grid_cols` /
>   `grid_rows`, pure — the sanity clamp lives at the writer). The old
>   cell `EditorViewport` is gone along with its dozen defensive
>   re-syncs (`sync_viewport_to_app` survives only as a `#[cfg(test)]`
>   seam), and the dead `cell_px`/`cell_px_h` fields — which had no
>   writer anywhere and silently "defaulted" to 14/28 — now derive from
>   the stage × dpr, so the media preview scales against the real grid
>   instead of a lie. `compose` / `patch_chrome_editor_scroll` lost
>   their dead `shell` parameters.
>
> **Still open from that pass (the epics):** the phase-2 rope/piece-tree +
> line-index storage migration; daemon D1 language/debug/document ownership;
> true native soft-wrap layout; and further domain extraction from `App`.
> `current_buffer` index removal, parse-off-the-keystroke-path, the first
> `App` extraction pass and the pixel-based core viewport are complete.

## Open bugs (user-reported)

1. **Terminal focus — FIXED 2026-07-21.** Pinned down at last: toggling the
   Debug Area open put the caret in the navigator's FILTER field instead of the
   shell. Two mechanisms, both in the face: `focused = true` targets the root
   `.focused` container, which is not itself focusable, so SwiftUI handed focus
   to the first field it could find — the Filter; and the panel's layout pass
   did the same via AppKit first-responder assignment. Fix in
   `setDebugArea(_:)`: `focusTerminal(true)` on open (core enters TERM, the key
   monitor routes to the shell), strip AppKit field focus 0.1s later
   (`makeFirstResponder(nil)` when a text field holds it), and the toggle only
   reclaims editor focus when CLOSING. Verified end-to-end: typed text lands at
   the shell prompt with no caret in the Filter.
2. **Folder expand animation does not run.** Rows appear instantly. Attaching a
   `.transition` to rows in the flat `ForEach` never fires — three attempts
   failed (stagger, group move, non-lazy `VStack`). The tree renders a *flat
   array* that changes wholesale, which SwiftUI does not animate reliably.
   **Fix = rewrite the navigator's rendering on `OutlineGroup` /
   `DisclosureGroup`**, which animates disclosure natively (that IS Xcode's
   behaviour). Data, filter, git marks and index marks can stay as they are.
3. **Tab close → content switch is abrupt.** The chip animation is done; the
   editor body still swaps instantly. Needs a crossfade in the canvas.

## 기능 추가 속히 요망

1. **편집기 경로 헤더에 상시 pane 분할/닫기 컨트롤 — IMPLEMENTED,
   runtime verification pending (2026-07-30).**
   - 대상은 전역 문서 탭 바가 아니라, 현재 파일 경로가 보이는 각 editor
     pane의 breadcrumb/jump-bar 헤더다.
   - 오른쪽 끝에 `split ▾`와 `×`를 둔다. hover 전용으로 숨기지 않는다.
   - `split ▾`는 항상 표시하고, 클릭 시 `위로 분할` / `아래로 분할`
     menu 또는 popover를 연다.
   - `×`는 pane이 2개 이상일 때만 항상 표시한다. 단일 pane에서는
     disabled 상태나 빈 공간도 남기지 않고 완전히 숨긴다.
   - 두 동작 모두 클릭한 헤더의 stable pane ID를 대상으로 해야 한다.
     `×`는 pane만 닫고 문서 tab/buffer는 닫지 않는다.
   - 최대 4-pane에서는 split 명령을 미리 disabled하고 이유를 표시한다.
   - 상세 인터랙션, 상태 보존 규칙, 접근성, DoD는
     [`SUISEI-PLAN.md`](SUISEI-PLAN.md) §6.1.1을 단일 명세로 따른다.
2. **Full Git Workbench master–detail 재설계 — PHASE 2 IMPLEMENTED,
   runtime/layout-scoped persistence verification pending (2026-07-30).**
   - `Status / Log / Branches / Files / Diff / PRs / Issues / Auth / Stash`
     9개 동급 탭을 작업 위계에 맞게 재편한다.
   - 1차 mode는 `Changes / History / Branches / Stashes`, GitHub 기능은
     `PRs / Issues` 그룹으로 분리하며 `Auth`는 settings/overflow로 옮긴다.
   - `Files / Diff`는 전역 mode가 아니라 선택된 file/commit의 detail이다.
   - 고정 `28% / 48% / rest`를 제거하고 draggable
     `master 320–360 / detail flex / context optional 260–320` 구조를 쓴다.
   - 행의 primary click은 선택+detail 열기다. 문자열 복사는 context
     menu로 이동한다.
   - Full Workbench 동안 Project Navigator는 임시로 접고 종료 시 이전
     visibility를 복구한다. 전용 26pt footer는 제거한다.
   - `<800pt`는 master→files/diff push navigation, `800–1199pt`는
     master+detail와 trailing context drawer, `≥1200pt`는
     master+detail+context를 사용한다.
   - master/context divider는 7pt hit target으로 drag 가능하고 전역
     preference에는 저장된다. layout tab별 폭 복원은 아직 남아 있다.
   - 반응형 계약과 mode별 화면 구성은
     [`SUISEI-SWISS-GRID-AUDIT.md`](SUISEI-SWISS-GRID-AUDIT.md) §12,
     발견 이슈는 [`../Report.md`](../Report.md) SUI-035를 따른다.

## Sidebars — the Xcode split (direction settled 2026-07-20)

The rule the whole layout follows:

> **Left = "where do I go"** — every row is a jump target.
> **Right = "what is this"** — facts about the one selected thing.

Done: the navigator strip's chrome is ONE pill that splits (`SplitCapsule`),
and it splits only while the Debug Area is open — so the framing reports state:
merged means the terminal is closed, separated means it is running. The gap is
what says "a toggle is not a mode". `NavMode.debug` is gone (it rendered the
project tree and merely toggled a panel), and the duplicate top-bar terminal
icon with it.

Two details worth not rediscovering:

- **The metaball is an ANALYTIC path, not a rendered one** (`SplitCapsule`).
  The standard SwiftUI recipe — blur the halves, cut the blurred alpha with
  `alphaThreshold` — was tried first and is the wrong tool: it is a raster trick
  and this is a vector control needing a crisp 1pt border. Every symptom that
  followed traced back to that one mismatch, and each was fixed in isolation
  before the real cause was named:
    - the threshold emits a BINARY mask, so edges aliased and a blur had to be
      added back to fake antialiasing;
    - the border had to come from two different cuts, which are two
      independently rasterised hard masks that do not line up — measured, 53 of
      832 columns carried an alpha dip up to 64/255, a broken transparent
      hairline circling the shape;
    - compositing the border over the fill made the outline visibly brighten
      wherever the halves overlapped.
  The Path has none of these: `fill` + `stroke` give an exact, uniform,
  antialiased border, and a sweep of the whole travel measures 0 alpha dips.
- **The outline is assembled with boolean ops on cap circles.** `body = capsuleA
  ∪ capsuleB`, then a bridge sliver bounded by two fillet arcs tangent to the
  facing caps. Fillet centres sit `k = √((r+R)² − (d/2)²)` off the axis; the
  neck vanishes on its own when `R < gap/2` makes tangency impossible, and `k`
  reaches 0 at exactly that moment, so the neck pinches to zero thickness before
  disappearing — a pop is not merely unlikely, it is geometrically impossible.
- **Do not bound that bridge with a full-height corridor.** `corridor − fillets`
  looks equivalent and is not: the corridor's four corners survive the
  subtraction and square off the waist, so the goo reads as a rectangle with
  circular bites taken out of it. The tangent points are needed.
- **`SplitCapsule` is deliberately not `InsettableShape`.** `strokeBorder` insets
  by half the line width, and here an inset feeds through `r`, the cap centres
  and `d` — half a point can push `gap` past the break and delete the neck. Fill
  and border then draw different shapes and the goo appears with no outline
  around it. Stroke the same uninset path instead.
- **The leading half must SWALLOW the trailing one while merged.** Sized from
  the layout (`modesW + 2 * inset`) the two overlap by only 4pt, and two caps of
  radius `height/2` meeting on a 4pt overlap pinch into a permanent waist — the
  pill looks squeezed at rest. It has to span the full width at `p == 0` and
  retract to meet `modesW` exactly at `p == 1`.
- **Spring values, measured not guessed.** SwiftUI's own presets, read off
  `Spring` at runtime: `.smooth` = duration 0.5 / bounce 0.0, `.snappy` =
  0.5 / 0.15, `.bouncy` = 0.5 / 0.30. The community-converged Dynamic Island
  spring (`response 0.4, dampingFraction 0.6`) converts to duration 0.4,
  bounce 0.40. `NavStrip.morph` sits just under at 0.4 / 0.34: the Island is a
  126pt shape on a black bezel, this is a 28pt pill in a dense rail, and the
  full 0.40 reads as jitter at that size.

**The selection indicator is ONE travelling shape** (`TravellingPill`), not a
background on each icon — per-icon fills can only cross-fade, while a single
shape can be somewhere in between. Two copies of the same pill run from the old
slot to the new one on different easings, so they start coincident, pull apart
mid-flight, and re-merge on arrival: a metaball whose endpoints are guaranteed
to be one clean capsule with no special-casing. Adjacent slots overlap
throughout and read as a STRETCH; a jump across the rail shows a real neck.

- **Sizing the halves instead (shrink the source, grow the target) is worse.** A
  capsule narrower than it is tall is degenerate, and it would have to pop out of
  existence at the end.
- **The lag must be capped or the pill tears in two.** Left to the easings the
  copies separate by `span · lag`, which a one-slot hop survives and a
  whole-rail jump does not — two floating pills read as a bug, not as liquid.
- **Do not cap it at `breakGap`.** That parameter only says where the fillet
  radius reaches zero; the neck actually dies earlier, as soon as `R < gap/2`
  makes tangency impossible — around `gap == r` at goo 1. Clamping to `breakGap`
  lands exactly on the break and it still tears. Measured with the cap at
  `0.75·r`: 0 breaks at every distance, neck 23.5pt (1 slot), 13.5pt (2), 6.5pt
  (4) on a 24pt pill.
- **Measure the neck between the cap centres, not over the whole shape.** The
  outer end caps taper to a point, so a naive minimum reports 0.5pt and sends
  you tuning a problem that is not there.

**Icons fill the rail, they do not pack left.** Compare Xcode's navigator at
minimum width against a widened sidebar: the icon gaps grow, they never leave
dead space at the trailing end. Two consequences that are easy to miss —

- **While merged, the toggle is just another slot in the distribution.** Give
  the modes `maxWidth: .infinity` and then append the toggle and the last gap
  comes out wider than the rest, which reads as the button being exiled.
- **The selection capsule fills its slot** rather than staying a fixed 28pt.
  Xcode's grows with the rail; a fixed pill reads as a button that forgot to.

Every width is computed from a `GeometryReader` rather than left to the layout,
because the metaball behind the glyphs has to land on the *same* numbers — a
flexible layout can only be asked where it ended up, not told.

Left rail is now Project · Source Control · Find · Issues · Breakpoints, plus
the detached Debug Area toggle. The two new ones were pure FFI plumbing — the
algorithms were already in the core:

17. **Find navigator** — DONE. `suisei_engine_search_project` wraps
    `workspace_search::search_project`. It takes **no engine pointer** so Swift
    can run it off the main thread; routing it through the engine would either
    freeze the UI or need locking the rest of the ABI does not have. Two traps
    it cost on the way in:
    - **The snapshot is 228,008 bytes** (300 × (512 + 240)) and a global-queue
      thread gets a 512KB stack. Declaring it as a local crashed the app
      outright — `EXC_BAD_ACCESS`, "Thread stack size exceeded", before a single
      result returned. It has to be heap-allocated.
    - **Never slice a `&str` by byte index to truncate.** `&line[..n]` panics
      the moment `n` lands inside a multi-byte character, and a panic across the
      FFI takes the app with it. `write_cstr` already caps the copy safely.
    - **Guard the root.** With no folder open the tree sits at `/`, and grepping
      that walks System, Library and every mounted volume. `isSearchableRoot`
      refuses `/`, `/Users`, `~` and friends.
18. **Issue navigator** — DONE. `suisei_engine_diagnostics` exposes
    `App.lsp.diagnostics`, which the core had carried all along; only line spans
    (kinds 251-253) ever reached the face.

Still to do:

19. **Right rail gets a mode strip** mirroring the left: Outline · File ·
    Quick Help. File inspector needs no core work at all (path, size, line
    count from `ProjectIndex`, git state from `chrome.scm`, index mark);
    Quick Help is one hover FFI. Keep the tabs visible and show a placeholder
    when they don't apply — that is what Xcode's "Not Applicable" is doing.

**Width is the constraint that bites.** The strip must survive the minimum
sidebar, so icons are 28pt, not 30, and `minS` is 240 (was 200):
`6×28 + 5 spacing + 4 padding + 8 gap + 32 toggle + 20 strip padding = 237`.
Adding a seventh mode means shrinking icons or raising `minS` again — do the
arithmetic before adding one, or the rail clips silently at small widths.
`loadPanelSizes`' floor must move with `minS` or a stale persisted width
reloads under it.

**Known hole:** the strip lives inside `if engine.uiNavVisible`, so hiding the
navigator hides the Debug Area toggle too; ⌘⇧Y is then the only way in. That is
why `setDebugArea` lives on `EngineBridge` rather than the view — the menu path
and the button path must not drift. Revisit only if it annoys in practice; a
second visible copy is the thing this change removed.

## Right rail layout — RESOLVED 2026-07-21 (kept for the traps)

Xcode's inspector is a FULL-HEIGHT column: it reaches up into the toolbar band
(the sidebar-toggle button lives inside it) and down past the status bar to the
window floor. The status bar spans only the editor's width and stops where the
inspector begins.

Ours is nested inside `detailColumn`, which sandwiches it between
`topBandHeight` and a `statusBarHeight` spacer, while `statusLine` is drawn at
the ROOT across the full width — so it runs underneath the inspector. **No
amount of corner-squaring or padding can make it meet the bottom bar from
there.** Three rounds were spent adjusting insets and radii before the level
itself was suspected; the giveaway was in the reference screenshot the whole
time.

    now                                     wanted
    HStack { sidebar, detailColumn }        HStack { sidebar, detailColumn,
      detailColumn = VStack {                       inspectorColumn }
        topBand, [editor | inspector],       detailColumn    = VStack { topBand,
        statusReserve }                                        editorCard,
    statusLine: full width                                     statusReserve }
                                             inspectorColumn = VStack { topBand,
                                                               inspector }
                                                               ← no bottom reserve
                                             statusLine: stops before inspector

Fixed by hoisting `inspectorColumn` to the root HStack as a full-height
column. Four traps surfaced on the way, each of which re-created "the line"
after the structure itself was already right:

- **The editor card's `strokeBorder` runs all the way round**, so its trailing
  edge IS a line at the seam. Masked away (`.mask` with a 1pt clear trailing
  column) while the inspector is open — Xcode separates the two by tone alone.
- **Even a 1pt layout slot for the resize grip shows the card's shadow** as a
  dark sliver between the surfaces. The grip is now an `.overlay` on the
  inspector's leading edge and owns no width at all.
- **The inspector column must be OPAQUE** (`shellBase`): transparency let the
  card's trailing shadow and the status bar's strip show through from behind.
  Opacity is also what lets the tone run unbroken from window top to floor.
- **`windowBackgroundColor` is not `shellBase`.** The AppKit dynamic is a
  visibly different tint, which made the status bar read as a stranger strip.
- The card's trailing corner squares off and its trailing `panelGap` drops to 0
  while the inspector is open; both were leaving shell showing through.

Done already and worth keeping: editor + terminal + inspector share ONE card
(`editorCard`) rather than carrying a border and shadow each — two of those
meeting is a groove no gap of zero can close.

**Final layout grammar (settled 2026-07-21): the island passes BENEATH the
floating navigator.** The editor+terminal island starts at the window's left
edge; the navigator widget floats over it, separated by nothing but its own
shadow; only the editor's CONTENT is inset (`navW + 7`) to stay clear of the
widget. The status bar keeps the shell tone and lost its top separator — the
island's shadow is the divider.

A terminal↔navigator metaball bridge across the shell channel was built first
and then CUT: with the island under the widget there is no channel left to
bridge. (Its one lesson: a neck must be the SURFACE colour — painted in the
terminal tint, −3.5% vs the channel's −3%, it was simply invisible.) The
metaball survives where it earns its keep: `SplitCapsule` on the navigator
rail, and `DockedPanelShape`'s concave fillets where the terminal band meets
the island walls — that band is painted by the CARD's background so it spans
the full island, under the navigator included; painted on the inset content it
stopped mid-surface in a vertical cut.

## Editing model (Phase 3 of the design doc)

The GUI editing path is now modeless. `Mode` identifies the keyboard-focus
surface, while `SelectionSet` is the selection authority and semantic commands
perform text edits.

4. **First-class plural selections are implemented.** `Selection {
   anchor, head, goal_x }` is exclusive, edits apply to every selection, and
   Shift movement, ⌘-click and add-next-occurrence use the same model. The
   remaining paint gap is the fill for non-primary non-empty selections.
5. **Grapheme movement and deletion are implemented.** `Buffer` uses
   `unicode-segmentation` for left/right/backspace/delete, including Hangul and
   emoji.
6. **Retained vertical goal columns are implemented** in the GUI selection
   movement path.
7. **Word boundaries are an ASCII heuristic** (`char_class`), not UAX #29.
8. **Still open:** ⌥-drag block selection. ⌘-click multi-cursor is implemented.

## Performance / architecture

9. **Document storage phase 2 remains.** `Edit`/`Delta` and incremental LSP are
   implemented, but `buffer.text()` still joins `Vec<String>` and offset
   conversion scans lines. Replace the storage with a rope/piece tree and an
   incremental line index behind the existing edit boundary.
10. **Undo stores and replays deltas, but checkpoints still snapshot.**
    History entries and spill files no longer retain full buffers; the current
    checkpoint-diff producer still calls `buffer.snapshot()` and clones lines.
    Native edit paths should eventually feed their produced deltas directly.
11. **The core viewport is pixel-based.** The AppKit canvas still maps one
    document line to one absolute row, so true stacked soft-wrap and variable
    line heights remain open.

## LSP / DAP — capability exists, surface does not

12. The core already implements definition, peek, hover, references, rename,
    formatting, code action, code lens, inlay hints, call hierarchy, workspace
    symbols and semantic tokens. The face now has FFI entry points for hover,
    diagnostics, format, rename and code actions, plus workspace search. The
    unfinished work is a coherent UI flow (especially definition/hover) and
    semantic-token paint, not a blank FFI boundary. Suggested order: go-to-
    definition → hover → code actions → rename/format.
    *(2026-07-25: none of it could work before — the GUI never pumped the LSP.
    See the "an async client is dead until something drains it" trap below.
    The pump also applies definition jumps, workspace edits and code actions,
    so those FFI entry points now complete a round trip.)*
13. **DAP has Core state and breakpoint FFI, but no complete GUI workflow.**
    Launch/attach, stack, scopes, variables, console, and continue/stop remain
    unreachable as a dependable IDE surface.
14. **Auto-pairing and completion-accept never worked in the GUI.** Both lived
    in the vim insert-mode handler, which the GUI never reached (it stayed in
    vim Normal). Removing that handler in the 2026-07-25 vim sweep deleted the
    dead implementations; the popup still opens (Ctrl+A) with no way to accept
    it. Re-implement on the Selection-model edit path, not by reviving a mode.
15. **Completion data is too thin** for an Xcode-grade list:
    `Suggestion { label, detail, insert_text }` — no kind (icons), no
    documentation, no sort/filter score, no snippet placeholders, no trigger
    characters.

## Indexing (partly done)

15. Parse-tree prewarming and normal parse/highlight work run in the syntax
    worker, coalescing bursts and rejecting stale versions. `total` / `done` /
    `isRunning` are not yet surfaced as progress UI; persistent warm-cache and
    incremental token patching remain.
16. Tree filter now searches **only already-expanded directories** — a deliberate
    trade to kill the freeze (a cold recursive scan from `/` walked System and
    Library). Project-wide search belongs in ⌘P / workspace symbols instead.

## Traps that cost hours — do not relearn

- **Traffic lights drop 16pt below their design point after certain titlebar
  relayouts.** Two-layer disease, fixed 2026-07-29. Layer 1 (the original):
  `applyTrafficLightInset` computed y from the button superview's height, but
  the titlebar accessory that grows the titlebar lands asynchronously — the
  inset fired while the titlebar was still its default 16pt and parked the
  buttons 16pt low until the next resize. Fixed: the inset anchors to the
  WINDOW top (superview origin converted to window space), correct before and
  after the growth, and runs immediately when the accessory is added. Layer 2
  (the residual): AppKit re-tiles the standard buttons on titlebar relayouts
  we neither control nor can enumerate — measured on macOS 26: focus return
  and hide/unhide do NOT move them and the accessory growth never even fires,
  yet specific flows still dropped them, healed only by a manual resize (its
  didResize re-assert). Fixed with a self-healing guard instead of trigger
  whack-a-mole: the 20 Hz tick compares the close button's real position
  against the design point (`panelGap + lightsCornerGap` off the top-left on
  both axes) and re-asserts on >0.5 pt drift (`TrafficLightGuard`), plus an
  immediate frame-only re-assert on `didBecomeKey`. The re-assert is frames
  only — the didBecomeActive RESTYLING freeze below is a different mechanism
  and must not be "fixed" by re-styling on reactivation.
- **A sheet squares the corners of a plain (borderless) window.** macOS 26's
  sheet machinery inserts an `NSSheetEffectDimmingView` as a SIBLING of the
  content view — both hang off the frame view (`NSNextStepFrame`) — so a
  corner mask on the content view alone never clips the dim, and the square
  dim paints over the transparent corner arcs while the sheet is up
  (recovery sheet on Welcome, fixed 2026-07-29). Mask the FRAME view
  (`contentView.superview`) as well; the Welcome probe re-applies on layout,
  which heals an AppKit reset. Verified by dumping the hierarchy with the
  sheet attached — `screencapture` is not an option (no screen-recording
  permission in this environment).
- **Key routing must be gated on `Mode`, never on a panel's `open` flag.**
  `explorer.open` means the docked Project navigator *has entries*
  (`suisei_engine_open_path` sets it on every open, deliberately without taking
  focus); keyboard focus is `Mode::Explorer`. `try_gui_edit` /
  `try_gui_navigation` gated on `explorer.open`, so from the moment a project
  was opened every keystroke fell past the modeless edit path into the vim
  command machine — typing did nothing and `z` opened the vim fold prefix.
  Fixed 2026-07-25. The failure is silent: the editor looks normal and just
  stops taking text. Tell-tale: a vim which-key hint in the status line.
- **An async client is dead until something drains it.** `LspClient`/`DapClient`
  write to a child's stdin and a reader thread fills a channel; `poll()` is the
  drain — and for the LSP it is *also* what sends `initialized` + `didOpen`
  after the `initialize` reply. The GUI engine tick never called it, so every
  rust-analyzer it spawned answered `initialize` and then sat at ~10 MB RSS
  forever while every LSP surface returned empty. Fixed 2026-07-25 by
  `App::poll_language_services` (`suisei-core/src/pump.rs`), called from
  `Engine::tick`. Symptom to recognise next time: **the server process exists
  and its RSS never climbs.**
- ~~`package-suisei-app.sh` watches `xei-core`, not `suisei-core`.~~ Fixed —
  the dependency scan covers both `suisei-engine` and `suisei-core`.
- **The Swift row decoder reads `SuiseiEditorLineC` by hard-coded byte offsets.**
  Adding a field means shifting `sel_v0/sel_v1/sel_u0/sel_u1/text/spanBaseOff` in
  BOTH decoders.
- **Span kind 254 (bracket hint) carries UTF-16 offsets**, unlike every other
  span, which carries visual columns.
- **`Terminal::cursor_position()` returns `(col, row)`** despite the name.
- **The Swift build is 4 minutes, and 89% of it is the optimiser.** Measured on
  this tree: `-O` 3:59, `-Onone` 0:27. It runs on ONE of ten cores, and there is
  no incremental step — a one-line edit to `ContentView.swift` recompiles all 11
  files. `SUISEI_FAST=1 ./scripts/package-suisei-app.sh` gives the 27s build; it
  stamps `.build/.swift-opt` so flipping the flag back forces a rebuild rather
  than leaving an unoptimised binary posing as a release. Never judge editor
  latency on it — the canvas draws its text in Swift.
- **Do not add `-enable-batch-mode` to parallelise that.** With
  `-import-objc-header` the frontends report success, emit no object files, and
  the build dies at link with `no such file or directory: ContentView-1.o`. It
  benchmarks 8% faster purely because a build that never links did less work —
  check that the binary actually exists before believing any build timing.
- **Measure before optimising.** The drag-scroll stutter took five wrong fixes;
  one measurement (`work` 0.3ms vs `gap` 56–77ms) showed instantly that our code
  was never the bottleneck — the main thread was starved by the indexer.
  Benchmarks that exist: `suisei-core/tests/syntax_typing_perf.rs`,
  `suisei-engine/tests/keystroke_latency.rs` (both `--ignored`).
