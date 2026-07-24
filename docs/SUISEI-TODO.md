# Suisei — outstanding work

> **Priority assessment (code-reviewed 2026-07-23).** Ordered by what stops
> Suisei being a trustworthy independent GUI editor first, an IDE second, and
> a polished product third. The full ordered roadmap is now
> `SUISEI-CURRENT-STATE.md`; this file retains detailed implementation traps.
>
> | | Theme | Why it ranks here |
> |---|---|---|
> | **P0** | Release gates · Swift 6 concurrency · ABI contract | The current app builds, but build/test reliability and the three-way ABI contract need a macOS CI gate. |
> | **P0** | First-class selections and native editing commands | Grapheme movement is now implemented, but selections still piggyback on Vim Visual state and Swift synthesises `i`/`Esc`. |
> | **P0** | WAL recovery → daemon | Atomic file writes are done; unsaved GUI-process state still disappears on crash. |
> | **P1** | Edit/Delta + async document consumers | `Vec<String>`, whole-text reconstruction and synchronous parser/highlight work remain the large-file latency risk. |
> | **P1** | Native IDE workflows | Search, diagnostics, format/rename/actions and partial UI exist; definition/hover/DAP need end-to-end workflows. |
> | **P2** | Terminal, extensions, docking/multiwindow | Important product depth, but only after editing, recovery and latency are dependable. |
> | **P3** | Canvas, Metal glyph renderer, elaborate transitions | Optional visual/interaction bets, not independence blockers. |
>
> **If only one product patch gets done next: the selection/command model (P0).**
> Run the release/ABI gate in parallel; then it unblocks shift-selection,
> multi-cursor, correct native text input and removal of render-layer workarounds.


State as of 2026-07-23. Companion to `SUISEI-CORE-DESIGN.md` (the rewrite
blueprint). Everything here is either reported-and-unfixed or deliberately
deferred; finished work is not listed.

> **Architecture plan (2026-07-21):** daemon separation, panel docking /
> stand-alone windows, resize stabilisation, Settings rework, dark-theme colour
> revision and menu-bar role division are specified in
> `SUISEI-ARCHITECTURE-PLAN.md`. That document supersedes this one for those six
> workstreams; this file remains the register of open bugs and deferred work.
>
> **Resolved:** `suisei-core::fs_atomic::atomic_write_file` now implements
> sibling tmp → write → fsync → rename, and `App::save_file()` uses it. The
> remaining durability gap is recovery of unsaved GUI-process state.

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

The core keeps vim semantics; the GUI needs its own. Currently patched at the
render layer only (`drawn_caret_col` in `scene.rs`).

4. **No first-class selection model.** `Buffer` has one `cursor`; selection exists only as
   `App.visual_anchor` gated by `Mode::Visual`, and it is **inclusive** while a
   GUI needs **exclusive**. Consequences: shift+arrow does nothing (AppKit sends
   `moveLeftAndModifySelection:`, nothing can receive it), and moving the cursor
   to fix the caret would widen the selection by one character.
   Target: `Selection { anchor, head, goal_x }`, `selections: Vec<Selection>`,
   every edit applied to all of them (multi-cursor becomes inherent).
5. **Grapheme movement and deletion are implemented.** `Buffer` now uses
   `unicode-segmentation` for left/right/backspace/delete, including Hangul and
   emoji. This is not a reason to defer the selection model: its range and
   vertical-motion semantics are still separate work.
6. **No goal column** — `move_up/down` still call `clamp_col()`, so the column is lost
   permanently when passing a short line.
7. **Word boundaries are an ASCII heuristic** (`char_class`), not UAX #29.
8. GUI selection gestures beyond drag: ⌥-drag block select, ⌘-click multi-cursor.

## Performance / architecture

9. **Phase 2 — Document rewrite.** `buffer.text()` (`lines.join`) and
   `fingerprint_text()` are still O(file) on every keystroke. Rope + line index
   + `Edit`/`Delta` removes both and unlocks incremental LSP sync.
10. **Undo still snapshots.** `push_undo()` → `buffer.snapshot()` clones every
    line. Should record edit deltas instead.
11. **Viewport is a terminal cell grid.** Blocks real soft-wrap and variable line
    heights; replace with a pixel viewport.

## LSP / DAP — capability exists, surface does not

12. The core already implements definition, peek, hover, references, rename,
    formatting, code action, code lens, inlay hints, call hierarchy, workspace
    symbols and semantic tokens. The face now has FFI entry points for hover,
    diagnostics, format, rename and code actions, plus workspace search. The
    unfinished work is a coherent UI flow (especially definition/hover) and
    semantic-token paint, not a blank FFI boundary. Suggested order: go-to-
    definition → hover → code actions → rename/format.
13. **DAP has Core state and breakpoint FFI, but no complete GUI workflow.**
    Launch/attach, stack, scopes, variables, console, and continue/stop remain
    unreachable as a dependable IDE surface.
14. **Completion data is too thin** for an Xcode-grade list:
    `Suggestion { label, detail, insert_text }` — no kind (icons), no
    documentation, no sort/filter score, no snippet placeholders, no trigger
    characters.

## Indexing (partly done)

15. Parse-tree prewarming discovers supported files asynchronously and warms
    one at a time, largest first. `total` / `done` / `isRunning` are published
    but not yet surfaced as progress UI; parsing still crosses the main actor
    and needs an off-main snapshot pipeline plus on-disk warm cache.
16. Tree filter now searches **only already-expanded directories** — a deliberate
    trade to kill the freeze (a cold recursive scan from `/` walked System and
    Library). Project-wide search belongs in ⌘P / workspace symbols instead.

## Traps that cost hours — do not relearn

- **`package-suisei-app.sh` currently watches `xei-core`, not `suisei-core`.**
  A core-only edit can therefore package an old dylib and make a fix appear to
  do nothing. Repair the dependency scan; until then force
  `build-suisei-engine.sh` after core-only edits and check the embedded dylib's
  timestamp.
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
