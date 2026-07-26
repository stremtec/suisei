# Suisei field issues

Defects found by **using** the app, as opposed to the feature gaps in
`SUISEI-GAP.md` and the planned work in `SUISEI-TODO.md`. Every entry here was
observed in a running `.app`, so each one is reproducible and each one is a
regression against "this should feel like a native editor".

Status: `OPEN` · `CAUSE` (root cause found, not fixed) · `FIXED` (with commit).

Opened 2026-07-26 from a single usability pass.

---

## A. Editing performance — the loudest problem

### A1 · Large files are slow to edit; typing lags · CAUSE FOUND, LARGELY FIXED

**Where it fundamentally goes.** Every measurement below is a **release** build,
a ~5,700-line file, ~59 visible lines. The Rust side is not involved: one
keystroke costs it 0.038 ms and the worst tick after an edit is 4.5 ms at
60,000 lines (`tick_breakdown`, `keystroke_latency`). Neither is the bridge —
the C ABI call is 0.006 ms, the 180 KiB snapshot allocation 0.001 ms, the line
decode 0.083 ms, the published-value compare 0.007 ms.

**It is all in `EditorCanvasView.draw`, and the reason is that the renderer had
no idea what changed.** Any repaint re-shaped and re-drew the entire viewport.

| per keystroke | before | after |
|---|---|---|
| `EditorCanvasView.draw` | **1.58–1.70 ms** | **0.40–0.48 ms** |
| lines shaped per repaint | 59 | 2 |
| gutter number, per line | 0.011 ms | 0.003 ms |

Three causes, all now fixed:

1. **The gutter had no cache.** Line numbers were drawn with
   `NSString.draw(at:withAttributes:)` — build a framesetter, shape the digits —
   once per visible line per repaint. 0.65 ms of the 1.6 ms, spent re-shaping
   digits that change only when the view scrolls. The *text* lines had a CTLine
   cache all along (it hits 3304 times out of 3305); the gutter did not. Now it
   does.
2. **The bracket-match flash repainted the whole viewport at 60 Hz.**
   `startBracketFade` set `needsDisplay = true` sixty times a second for 0.9 s.
   At 1.6 ms a repaint that is ~96 ms of main-thread work per second of flash —
   and writing code retriggers it on nearly every keystroke. It now invalidates
   only the rows the hint is painted into.
3. **`EditorMetrics.cellWidth` built a font and measured a string on every
   read**, and `gutter` reads it, and `draw` reads both. Cached per font size.
   (This one was also a crash — see the crash section.)

**Cleared of suspicion, so nobody re-suspects them:** the Rust engine, the C ABI
call, the 180 KiB snapshot copy, the line decode/compare/publish, the minimap
(its version cache holds — one miss per 25 keystrokes), and `cacheKey`'s string
building (0.001 ms/line; it looked expensive and is not).

**Still open.** `refreshEditorPaintOnly` shows an occasional 16–28 ms spike that
its instrumented children do not account for. The tail does five more FFI pulls
(palette, search, completions, terminal, outline) and a deep `Equatable` compare
of the whole chrome per keystroke; those measure at 0.02–0.03 ms each, so the
spike is elsewhere — most likely SwiftUI's own re-render after the `@Published`
assignment, which no probe on this side can see.

Gate: a 6,000-line file types at frame budget with the LSP live.

### A2 · Intermittent stalls ("자꾸 렉걸린다") · OPEN
Distinct from A1: hitches during ordinary use, not only in big files. The Rust
tick is ruled out (worst post-edit tick is 4.5 ms at 60,000 lines). Remaining:
synchronous work on the main actor — git refresh, explorer refresh, project
index, `FileManager` enumeration.

---

## B. Dirty-state lies

### B1 · A file shows dirty without being edited · FIXED (2026-07-26)
The trigger was never found, and chasing it was the wrong approach: `modified`
is set by `push_undo` on every edit and *nothing* ever put it back down, so any
path that touched the buffer and restored it — an abandoned IME composition, a
paste of text that was already there, a deletion of nothing — latched the flag
for the rest of the session. Auditing every such path is a losing game.

The flag is now self-correcting. The engine tick calls `App::recheck_modified`
(~1 s), which re-derives it from the text. Bounded three ways: only while the
flag is up (the latch is exact for clean → dirty, so a clean buffer is never
re-hashed), only when the text moved since the last check, and only on that
cadence. One 0.24 ms hash per second of active editing on a 60,000-line file.
This fixes the symptom for any cause, including ones not yet seen.

### B2 · Undo back to the original state stays dirty · FIXED (2026-07-26)
`modified` was a one-way latch: set by every edit, cleared only by a save. A
version watermark does not work — `Buffer::restore` calls `touch()`, so undo
produces a *fresh* version even when the text is identical. `App` now keeps
`saved_hash`, the fingerprint of the text as it stands on disk, recorded by
`mark_clean()` at load and after each save. Undo and redo call
`refresh_modified()`, which compares. That comparison is O(file), which is why
it is confined to undo/redo: typing can only ever make a document dirtier, so
the cheap latch stays on the hot path.

---

## C. Terminal panel

### C1 · Resizing the panel clips the terminal · FIXED (2026-07-26)
Not the propagation — the face does report the panel size. Two fixed caps:

* `SUISEI_TERM_LINE` was **256 bytes** per row, and rows are truecolor SGR
  strings, so one colour change costs up to 19 bytes on top of the character it
  colours. A wide `ls --color` or build log ran out of budget after roughly a
  dozen colour changes and the rest of the line was dropped. Now 1536.
* The scene gave the side-panel terminal **48 rows** and the full panel 120.
  Dragging the side panel taller than 48 rows drew rows the scene never sent.
  Both now use the snapshot's own ceiling, raised to 200.

Both are in `abi_layout.rs` now, so they cannot drift from the C header. While
there: the snapshot builder stopped zero-filling all 300 KiB per refresh — every
header field is assigned and a row only needs its first byte cleared to read as
an empty C string, so that is 200 stores instead of a 300 KiB memset.

### C2 · Terminal scrollback is clipped · FIXED (2026-07-26)
It was not clipped, it was **unreachable**. Core keeps 5,000 rows of scrollback
and a `scroll_offset`, and neither had a consumer: the scene sent
`visible_rows_sgr`, which returned the live grid alone, and there was no FFI
entry to move the offset and no gesture bound to one. Scrolling the panel
showed the same screen back.

`Terminal::viewport_rows` now windows over scrollback + live grid, and
`visible_rows_sgr` uses it. `suisei_engine_terminal_scroll` moves the offset.
The panel's `TermScroll` is an `NSScrollView`, so native scrolling still wins
whenever the grid has content to move; only the part that runs off an end is
forwarded to core. Verified in the running app: `seq 1 200` then scrolling up
walked 179 → 139 and back down to live.

### C3 · A new terminal opens scrolled to the bottom, hiding the greeting · FIXED (2026-07-26)
The PTY was spawned sized from the **editor viewport** (~40 rows) and then
resized a moment later to the panel's real height (~17). Shrinking a grid pushes
its top rows into scrollback, so the shell's greeting had already scrolled away
before the first paint. The engine now remembers the grid the face measures —
`terminal_resize` records it even while the panel is closed — and spawns at that
size, falling back to the viewport only when the face has not measured yet.

---

## D. Scrolling

### D1 · Horizontal scrolling is infinite · FIXED (2026-07-26)
Two halves, and the second was the treadmill. Core never clamped `hscroll` on
the right at all. Worse, the face sized its scroll canvas as
`max(400, hScroll + 160)` columns — **a width that grew with the scroll
position**, so every pan to the right made the document wider and the end
receded forever.

`App::content_cols()` is now the single answer, exposed as
`suisei_engine_content_cols` and used by both `set_hscroll` and the canvas
sizing. It is a **high-water mark**: raised by the widest line currently on
screen, never lowered, reset when the document changes. Rescanning the whole
file per keystroke would be O(file) on the typing path, and letting the extent
shrink as short lines scrolled into view would resize the scroller thumb under
the user's hand. Widths are display columns — tabs to the next stop, CJK two
cells — so the clamp lands on the last glyph rather than near it.

---

## E. Animation and motion

### E1 · Closing the find bar with ✕ animates wrong · OPEN
(Esc not closing it at all is a separate known bug — see H1.)

### E2 · Tab bar switching feels wrong · OPEN
No motion continuity between the outgoing and incoming tab.

### E3 · Left sidebar open/close animation is wrong · FIXED (2026-07-26)
It was not wrong, it was **absent** — depending on how you asked. The top bar's
toggle wrapped the flag flip in `animatePanels` (a private helper in
`ContentView`); the menu commands ⌘0 / ⌥⌘0 / ⇧⌘Y and every "reveal this
navigator" menu item flipped the same flags raw. Frame capture: one changed
frame for ⌘0 against eight for the button.

The motion now lives on `EngineBridge.animatingPanels`, so every entry point
shares it — including the Inspector and Debug Area, which had the same split.
Re-captured after: ⌘0 glides across eight frames like the button.

### E4 · The travelling pill's *colour* transition is awkward while it moves ·
FIXED (2026-07-26)
Frame capture settled it in one look. Through the flight the pill went
**accent → pale → pure white → pale → accent**: while travelling, the solid
accent pill is hidden and a `LiquidGlassPill` drawn instead, and that glass was
applied to `Color.clear`. Untinted glass over the rail's light chrome is white,
so the selection appeared to vanish and a white smear crossed the strip.

The glass now carries `.regular.tint(.accentColor)`, so the indicator keeps its
identity for the whole journey. Two follow-ons fell out of that:

* The icon ink now keys off the pill's *travel* rather than the click.
  `navMode` changes one frame before the flight begins, so the destination icon
  flashed white, dimmed for the whole journey, then snapped back. The origin now
  hands its ink over as the pill leaves and the destination takes it as the pill
  arrives — legible only because the pill is tinted.
* A slot the pill merely *crosses* on a two-slot jump used to sit in unselected
  grey while solid accent slid across it; lighting is now a triangular window
  over the whole span, not just its endpoints.

Capture method, for the next one of these: `screencapture -x -R <x,y,w,h>` in a
loop (~76 ms/frame) while clicking, then diff consecutive frames to find the
transition. Region coordinates are points; the computer-use screenshot is scaled
(1389 px wide for a 2048-point display, ×1.474).

---

## F. Layout and proportion

### F1 · The Welcome screen's proportions are poor · OPEN
Measured against Xcode 26.6's welcome window (reference screenshots, 2026-07-26):
Xcode splits ~60/40 between the icon column and the Recents list and puts a hard
vertical divider between them; Suisei splits ~54/46 with no divider, a smaller
icon relative to the panel, and Recents rendered as one fat rounded card instead
of a plain list of rows.

---

## G. File tree ergonomics

### G1 · Moving a file *out* of a folder is painful · FIXED (2026-07-26)
Both halves, as the entry suggested.

**Drop on a file row now lands in that file's folder.** Previously only
directory rows accepted a drop, on the reasoning that a file row would have to
guess between "into its folder" and "replace it" — but replace is never a
sensible file-tree drop, and Finder resolves it the same way. Moving something
up one level is now "drop it on any sibling". The highlight follows the real
destination, so the enclosing folder lights up rather than the row under the
cursor. *Not verified by automation:* synthetic mouse events do not reliably
open a SwiftUI drag session, so this half was reasoned and compiled, not driven.

**"Move to…" in the context menu** opens a destination picker rooted at the
file's own folder. Reaches folders that are collapsed, scrolled away, or outside
the project — none of which a drag can. Verified end to end in the running app:
the file moved on disk, the tree refreshed, and the open tab's breadcrumb
followed it (`App::path_moved`).

---

## I. Crashes

### I1 · Uncaught exception during window styling · FIXED (2026-07-26)
Crash report 2026-07-26 08:07, `EXC_BREAKPOINT` via
`+[NSApplication _crashOnException:]` inside `_NSViewLayout`. Symbolicated:
`ContentView.styleTrafficLights` → `-[NSWindow titlebarAccessoryViewControllers]`
→ `objc_exception_throw`.

`isEditorWindow` was a title-string test — `w.title != "Settings" && w.title !=
"Welcome"` — which is true of every AppKit auxiliary window as well: popovers,
sheets, tooltips, the open panel, SwiftUI's own helpers. Most carry no titlebar,
and reading `titlebarAccessoryViewControllers` on such a window throws. It is
intermittent because it depends on which auxiliary windows exist at the moment
the appearance sync runs. The test is structural now (`.titled`, not an
`NSPanel`), and both titlebar helpers refuse an untitled window themselves.

### I2 · Nil font reaching CoreText · FIXED (2026-07-26)
Caught live under a typing load: `NSInvalidArgumentException … attempt to insert
nil object from objects[0]`, thrown from `CTLineCreateWithAttributedString`
under `EditorMetrics.cellWidth`.

`NSFont.monospacedSystemFont(ofSize:weight:)` is imported into Swift as
**non-optional**, but the AppKit call behind it can return nil when the font
cannot be created. Swift carries that nil along as an ordinary reference; it
detonates much later, when CoreText copies it into an attributes dictionary,
with a stack pointing at whoever happened to be drawing rather than at the font.
`cellWidth` called it on every read — several times per draw — so the exposure
was continuous. It is now created once per size through
`EditorMetrics.monospaced`, which checks for the bridged nil and falls back.

---

## J. Second usability pass (2026-07-26, evening)

### J1 · The inspector toggle's rounded background is off-centre · FIXED (2026-07-26)
`ToolbarPlainIcon` applied `opticalNudgeX` to the **glyph** and drew the hover
capsule on the **frame**, so the two disagreed by exactly the nudge. The
inspector toggle carries `-0.6`, and 0.6pt of disagreement is what reads as the
capsule being shoved right. The nudge now sits after the capsule, so glyph and
capsule move together and the pair is optically centred in its slot; the row
rhythm shifts by the same sub-point, which nothing can see.

### J2 · The command palette is off-centre · FIXED (2026-07-26)
The overlay hangs off the root view, so it centred on the **window** — and the
two panels flanking the editor are not the same width, so window-centred is
never editor-centred. Measured with both open: navigator edge at x=318,
inspector edge at x=1757, editor centre 1037.5 against a window centre of
1023.5. It now offsets by `(navReserved - inspectorReserved) / 2`; re-measured
after, the panel centres on 1036.

Note for the next overlay: the editor's right bound is the **inspector's** edge
(1757), not the minimap's (1696). Measuring against the minimap gives an editor
centre of 1007 and sends you the wrong way by 30px.

### J3 · No motion when switching tabs · FIXED (2026-07-26)
Each chip drew its own capsule, so a switch cross-faded two of them in place.
There is now **one** capsule for the whole strip, matched to per-chip anchors
via `matchedGeometryEffect`, animated on the strip — the only view holding both
the chip it leaves and the chip it arrives at.

Worth knowing for the next one of these: a capsule *per chip* each claiming the
same id as a source renders **twice** mid-transition (both chips look
highlighted for a frame). Caught in a mid-flight capture on the first attempt.
One consumer (`isSource: false`) against many anchors (`isSource: true`) is the
construction that cannot do that.

### J4 · Tabs cannot be reordered · **HALF DONE** — the move works, the grab does not

**Done and tested:** `App::move_tab(from, to)` carries every index that points
into `buffers` with the move — the active tab and every split pane's
`tab_index`. Panes address their document by position
(`SUISEI-SPLIT-PLAN.md` §1.1), so a reorder that moved only the vector would
leave each pane showing whatever slid into its slot. Two regression tests cover
both directions, the pane remap and the no-ops. Exposed as
`suisei_engine_move_tab` / `EngineBridge.moveTab`.

**Not done:** getting a mouse drag on a tab chip to *reach* anything. Four
approaches, each one built and driven against the running app, each one
measured rather than assumed:

| approach | result |
|---|---|
| `.onDrag` / `.onDrop` (system drag session) | never starts — the chip's own `simultaneousGesture(DragGesture(minimumDistance: 0))` pre-empts the press |
| per-chip `DragGesture` + `simultaneousGesture` | chip is a `Button`; its recogniser claims the movement |
| row-level `highPriorityGesture` | `onChanged` **never fires** (probe-confirmed). Dragging moved the **window** — the top bar has a full-bleed `WindowDragGesture` layer under the strip |
| AppKit `NSView` overlay, `hitTest` + `acceptsFirstMouse` | `mouseDown` **never fires** (probe-confirmed) |

The third row is the strongest evidence: the press is being claimed by the top
bar's `WindowDragGesture`, which is a full-bleed `Color.clear` layer behind the
tabs in the top bar's `ZStack`.

Next things to try, in order:
1. **Stop that layer being full-bleed.** Give the window drag only the regions
   that should drag the window, so the strip's area is not over it at all.
   This addresses the measured cause directly.
2. Check whether the strip's `.mask(...)` is breaking hit-testing for anything
   other than the Buttons — a `.contentShape(Rectangle())` after the mask would
   rule it in or out.

### J5 · Editor split is unstable · PLANNED — **priority 2**
### J6 · "Turn this pane into a terminal" is unstable · PLANNED
Both need rebuilding rather than repair. Causes and an ordered plan are in
[`SUISEI-SPLIT-PLAN.md`](SUISEI-SPLIT-PLAN.md). The short version: panes address
content by **index** (`Pane.tab_index`, `terminal.pane_bound`) so closing or
reordering anything silently repoints them; the focused pane's scroll/cursor
exists in two places kept in step by hand; one `kind` and one `ratio` serve the
whole layout, so mixed splits are refused outright and three panes cannot be
resized; and the terminal's location is spread across four fields that can
disagree. J6 as specified — convert the focused pane, displace its document to
the tab bar — is not implemented at all; the current code opens a *new split*
instead.

### J7 · No editor layout save / restore · OPEN
VS Code-like tab behaviour is missing. The intended shape is specific: a fast
scroll up or down near the tab bar reveals a **separate layout tab bar**, so
layouts are switchable the way documents are. Design first, then build — this
is a feature, not a defect.

---

## H. Known-deferred, still open

### H1 · Esc does not close the find bar · OPEN
The ✕ button works, so this is Swift key delivery, not engine routing.

### H2 · `open -a Suisei <file>` does not open the document · OPEN
No `application(_:open:)` / Apple-event handling.

### H3 · The project tree's `+` leaves focus in the filter field · OPEN

---

## Working rule for this list

These are mostly *feel* bugs, and feel bugs have a bad history in this project of
being "fixed" by the third guess. For each one: reproduce in the running app,
find the mechanism, state the mechanism in the commit, then fix. Frame-by-frame
capture beat reasoning twice already (the folder-expand animation, the `+`
optical centre) — reach for it early.

For anything that feels *slow*, measure both sides before touching code:

```bash
cargo test -p suisei-engine --release --test tick_breakdown -- --ignored --nocapture
```

```bash
cargo test -p suisei-engine --release --test keystroke_latency -- --ignored --nocapture
```

```bash
SUISEI_PERF=1 ./suisei-app/.build/Suisei.app/Contents/MacOS/Suisei
```

The last one prints a per-label mean/max/total to stderr every 2 s (see
`PerfProbe.swift`); launching the binary directly rather than via `open` is what
puts stderr on the terminal. It is compiled into every build and costs nothing
when the variable is unset. A1 above is what that tooling found — and what it
cleared.
