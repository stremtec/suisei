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

### C1 · Resizing the panel clips the terminal · OPEN
The PTY is resized from the editor viewport (`ensure_terminal_started` uses
`app.viewport`), and the panel's own height is not propagated back on drag, so
rows fall outside the drawn area.

### C2 · Terminal scrollback is clipped · OPEN
Same root suspicion as C1: the visible row window and the PTY's row count
disagree.

### C3 · A new terminal opens scrolled to the bottom, hiding the greeting · OPEN
The shell's startup banner is already scrolled past before the first paint.

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

### E3 · Left sidebar open/close animation is wrong · OPEN
Both directions. Compare against the inspector, whose pill work already landed.

### E4 · The travelling pill's *colour* transition is awkward while it moves ·
OPEN
Both sidebars. The geometry now animates (that was the earlier inspector-border
fix); the fill/label colour still cuts rather than crossfades.

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
