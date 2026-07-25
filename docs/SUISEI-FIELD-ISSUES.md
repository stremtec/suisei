# Suisei field issues

Defects found by **using** the app, as opposed to the feature gaps in
`SUISEI-GAP.md` and the planned work in `SUISEI-TODO.md`. Every entry here was
observed in a running `.app`, so each one is reproducible and each one is a
regression against "this should feel like a native editor".

Status: `OPEN` · `CAUSE` (root cause found, not fixed) · `FIXED` (with commit).

Opened 2026-07-26 from a single usability pass.

---

## A. Editing performance — the loudest problem

### A1 · Large files are slow to edit; typing lags · OPEN
Typing latency grows with file size, and indexing finishing does not help.
Suspects, in the order they should be measured (**measure before touching
anything** — the last three "obvious" performance guesses in this project were
all wrong):
- `Vec<String>` buffer + full-text rebuilds. Every consumer that wants the
  document joins the lines again. `sync_lsp_document` sends **whole-file**
  `didChange` on a 3-tick throttle; on a large file that is a full `String`
  build plus JSON-escape per sync.
- Syntax re-parse per keystroke (`app.syntax`), no incremental edit path.
- `recompose()` cost: the engine tick chooses full vs light recompose, but
  chrome-dirty ticks force the full path.
- The journal (`journal.rs`) writes the whole dirty buffer on a 250 ms / 4 KiB
  policy — another full-text build.
This is the `SUISEI-CURRENT-STATE.md` P1.4 delta work arriving as a real
complaint. Gate: a 6,000-line file types at frame budget with the LSP live.

### A2 · Intermittent stalls ("자꾸 렉걸린다") · OPEN
Distinct from A1: hitches during ordinary use, not only in big files. Look for
synchronous work on the tick — git refresh, explorer refresh, project index,
`FileManager` enumeration on the main actor.

---

## B. Dirty-state lies

### B1 · A file shows dirty without being edited · OPEN
Opening a file and touching nothing raises the modified indicator. Find who
mutates the buffer (or bumps its version) on open: trailing-newline
normalisation, tab expansion, EOL translation, or a scroll/cursor write that
bumps `version()`.

### B2 · Undo back to the original state stays dirty · OPEN
`modified` is a latch, not a comparison. The fix is a saved-version watermark:
record `buffer.version()` at load/save and clear `modified` whenever the current
version equals it. Undo must restore the *version*, not just the text, or the
watermark never matches again.

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

### D1 · Horizontal scrolling is infinite · OPEN
The h-scroll has no content-width clamp, so a trackpad pan runs off into empty
space forever. `scroll_h_by` clamps only at 0 on the left.

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

### G1 · Moving a file *out* of a folder is painful · OPEN
Drag-and-drop only accepts a drop **on a directory row**, so moving a file up one
level means scrolling to the top-level directory row and dropping there. Needs
either a drop target for "the parent of this row" (drop on the gutter/whitespace
of a level), or a "Move to…" command in the context menu, or both.

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
