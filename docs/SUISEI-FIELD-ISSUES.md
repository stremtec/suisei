# Suisei field issues

Defects found by **using** the app, as opposed to the feature gaps in
`SUISEI-GAP.md` and the planned work in `SUISEI-TODO.md`. Every entry here was
observed in a running `.app`, so each one is reproducible and each one is a
regression against "this should feel like a native editor".

Status: `OPEN` · `CAUSE` (root cause found, not fixed) · `FIXED` (with commit).

Opened 2026-07-26 from a single usability pass.

---

## A. Editing performance — the loudest problem

### A1 · Large files are slow to edit; typing lags · CAUSE (partly)

**Measured 2026-07-26. Most of the obvious suspects are innocent** — recorded
here so nobody re-suspects them.

Rust side (`tests/tick_breakdown.rs`, `tests/keystroke_latency.rs`, release):

| | 2k lines | 20k | 60k |
|---|---|---|---|
| `dispatch_key` (one keystroke, 6k-line file) | — | 0.038 ms | — |
| `buffer.text()` | 0.008 ms | 0.077 ms | 0.238 ms |
| `build_outline` | 0.364 ms | 0.731 ms | 0.727 ms |
| idle tick | 0.008 ms | 0.094 ms | 0.239 ms |
| worst tick after an edit | 0.623 ms | 2.066 ms | 4.514 ms |

Face side (`PerfProbe`, **-Onone debug** build so these are inflated, 5,700-line
file, per keystroke):

| | mean | max |
|---|---|---|
| `EditorCanvasView.draw` | 1.64 ms | 2.47 ms |
| `refreshEditorPaintOnly` | 1.69 ms | 1.78 ms |
| ⤷ `decodeEditorLinesAndSplit` | 0.82 ms | 0.91 ms |
| ⤷ `suisei_engine_chrome` (the FFI call) | 0.006 ms | |
| ⤷ 180 KiB snapshot alloc | 0.014 ms | |
| ⤷ `lines != editorLines` compare | 0.024 ms | |
| ⤷ `@Published` assignment | 0.027 ms | |
| `refreshChrome` (full shell) | 1.74 ms | 6.23 ms |
| `minimapData` | **1 miss per 25 keystrokes** | |

Cleared of suspicion: the Rust engine (two orders of magnitude under budget),
the C ABI call, the 180 KiB snapshot copy, the line decode/compare/publish, and
the minimap (its version cache holds).

**Found and fixed:** the shadow-WAL call site built the entire document on
*every* tick — 20×/second, dirty or clean — for a value the 250 ms/4 KiB policy
throws away most of the time. Now a closure, called only on an actual flush.

**Still open — the real remainder.** Nothing measured above scales the way the
complaint does, and two things are still unmeasured:
1. **SwiftUI's own re-render** after `@Published chrome` fires. The probe stops
   at the assignment; body evaluation, layout and diffing of the whole shell
   (tab bar, breadcrumb, outline list, status bar, project tree) happen after.
   `refreshChrome` runs 120 ms after every typing pause via `scheduleChromeSettle`.
2. **A release-build number.** Everything above on the face side is `-Onone`;
   real latency needs `./scripts/package-suisei-app.sh` without `SUISEI_FAST`.
Next step: instrument SwiftUI body evaluation (or Instruments' SwiftUI template)
against a release build, and re-measure with rust-analyzer live — the user notes
the lag persists after indexing completes, which points at the diagnostics /
semantic-token republish path rather than at indexing itself.

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
