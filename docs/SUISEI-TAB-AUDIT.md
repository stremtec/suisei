# Tabs and splits: full audit

Every tab- and split-related feature, walked one by one in the packaged app on
2026-07-27, with the cause named where it is broken. Written before starting
plan steps S2 and S3 ([`SUISEI-SPLIT-PLAN.md`](SUISEI-SPLIT-PLAN.md)) because
the user's "매우 불안정" covers more than those two steps address, and it was
worth knowing what is actually wrong before rebuilding the model.

**The headline result changed the plan.** Several of the worst symptoms are not
model bugs at all — they are in the face, and two of them are regressions from
the tab-drag work rather than long-standing gaps. S2 and S3 would not have
fixed them, so they come first and separately.

Legend: ✅ works · ❌ broken · ⚠️ works but by a mechanism that will not scale ·
— not tested.

---

## 1. Tab strip

| | Feature | | Notes |
|---|---|---|---|
| T1 | Click a chip → switch tab | ✅ | |
| T2 | Drag a chip → reorder | ✅ | J4 |
| T3 | Right-click → Close Tab / Close Other Tabs | ❌→✅ | **Fixed this session.** |
| T4 | `+` → New Tab / Next / Previous / Split menu | ❌ | see 1.1 |
| T5 | Gaps between chips are dead to clicks and to window-drag | ❌ | same cause as T4 |
| T6 | Active tab auto-scrolls into view | — | |
| T7 | Strip scrolls when tabs overflow | — | |
| T8 | Dirty dot, title, active capsule travel | ✅ | |

### 1.1 One cause behind T3, T4 and T5

`TabStripMouse` is an AppKit overlay that exists for one reason: this strip is
in the window's titlebar region, so AppKit consumes `mouseDown` to drag the
window before SwiftUI ever arbitrates, and only a view overriding
`mouseDownCanMoveWindow` gets the events at all. That part is correct and hard
won.

But it is attached with `.overlay` to the **whole `HStack`** — chips *and* the
`+` slot — and it hit-tests everything inside. Whatever it claims, the views
beneath never see:

* Right-clicks reached the overlay, which has no `menu(for:)`, so the strip's
  context menu silently stopped existing.
* Clicks on `+` reach the overlay too, and `mouseUp` only routes a click onward
  when `slotAt(x)` finds a chip under it. Over the `+` there is no chip, so the
  press is swallowed and the `Button` never fires.
* The same is true of the 4pt gaps between chips, which should fall through to
  the window-drag layer underneath.

The overlay should claim exactly what it needs — a left-button press **on a
chip** — and be transparent to everything else. Right-clicks are already fixed
that way; the rest is the same edit.

---

## 2. Splits

| | Feature | | Notes |
|---|---|---|---|
| P1 | Split Editor Right, 2 panes | ✅ | |
| P2 | Split Editor Right, **3+ panes** | ❌→✅ | see 2.1 — fixed twice: F2 stopped the clipping, S3 replaced the stopgap |
| P3 | Split Below while split vertically | ❌→✅ | was refused outright; the tree makes the four-pane `+` reachable |
| P4 | Divider drag, 2 panes | ✅ | |
| P5 | Divider drag, 3+ panes | ❌→✅ | N−1 independent dividers, from per-child weights |
| P6 | Focus Next Pane | ✅ | |
| P7 | Click a pane to focus it | ✅ | |
| P8 | Pane header `+` (open a file into this pane) | ✅ | focuses the pane, then opens the palette |
| P9 | Pane header `✕` (close focused pane) | — | |
| P10 | Per-pane scroll and cursor stay independent | ✅ | two panes; kept by hand, see 2.2 |
| P11 | Closing a tab leaves other panes alone | ✅ | fixed by S1 today |

### 2.1 Three panes: the first one is clipped off the window

Reproduced every time: split right twice, and the leftmost pane paints no
filename, no line numbers and no text — just the trailing `+` of its header.

It is not a paint bug. `ContentView.splitEditorLayout` computes exactly **two**
widths and gives the second to every pane after the first:

```swift
width: idx == 0 ? size.width * ratio : size.width * (1 - ratio)
```

With three panes and `ratio = 0.5` that asks for `0.5W` three times — 150% of
the space. The `HStack` overflows and centres, so the surplus is clipped evenly
off both ends, and the first pane loses its left edge, which is where its title
and gutter live. The trailing `+` survives because it is right-aligned.

The arithmetic matches the pixels exactly. Editor area ≈ 974pt, so each pane
asks for 487pt, total 1461pt, overflow 487pt, 243pt clipped per side:

| | asked | measured on screen |
|---|---|---|
| pane 1 | 487 | 245 (x 215→460) |
| pane 2 | 487 | 488 (x 462→950) |
| pane 3 | 487 | 238 (x 952→1190) |

This is §1.4 of the split plan ("one `ratio` for any number of panes") showing
up in the **face** rather than in core. Worth stating plainly because the plan
located that flaw in `SplitState` and would have left this side of it standing.

### 2.2 The pane state duplication is real but currently invisible

P10 passes, so the `sync_split_from_active` / `apply_focused_pane` pair is
holding for the paths exercised here. That does not make it safe — it is the
hand-maintained invariant S2 deletes — but it means S2 is a *robustness* step,
not a fix for a symptom that is currently visible. Ordering it after the face
repairs costs nothing.

---

## 3. Adjacent, found while auditing

| | | | |
|---|---|---|---|
| X1 | An overlay opened while the tree filter has focus is **deaf** | ❌→✅ | see 3.1 — **fixed this session** |
| X2 | Esc does not close the palette / find bar (H1) | ? | see 3.2 — **not established**, and my test method cannot decide it |
| X3 | The file palette lists from `/` when the project root is `/` | ⚠️ | Usability, out of scope here. |

### 3.1 A palette that cannot hear

Opening the file palette while the project tree's Filter field still held first
responder produced a palette that was visible and completely deaf. Measured
directly: with the palette open, typing `s1_b` filtered the **project tree**
behind it and left the palette's own filter empty.

`editorOwnsKeyEvents` stands down whenever an `NSTextField`/`NSTextView` is the
window's first responder, which is correct — otherwise the tree filter could
not be typed into. What was missing is that opening an engine-owned overlay has
to *take the keyboard back*. `reclaimKeyboardFromTextFields()` now does that,
driven off the engine's own `palette.open` / `search.open` flags so every route
in is covered rather than the one call site that happened to be tested.

Verified after the fix: same setup, typing `s1_b` filters the palette to
`s1_b.txt` and the tree filter keeps its own text.

### 3.2 Esc: the tool was the bug, not necessarily the app

Escape *appeared* not to close the palette. It does not close it under
synthetic input — but neither does anything else. An `NSMenu` was left open,
sent the same synthetic Escape, and stayed open; a real Escape always closes an
`NSMenu`. So the synthetic Escape never reaches the application at all, and
**every "Esc does nothing" observation in this session is an artefact of the
test method.**

What is established, by a test rather than by clicking
(`esc_closes_the_file_palette` in `suisei-engine`): core routes Esc to
`handle_palette`, the engine front-end does not intercept it, and the palette
closes. So if Esc really is dead in the user's hands, the fault is in Swift key
delivery — but *whether* it is dead is not something this pass could determine.
H1 stays open and needs a human at the keyboard.

---

## 4. Revised order of work

The audit moves three cheap, high-visibility repairs ahead of the model work,
and demotes one step that turned out not to be causing visible harm.

### Phase 0 — face repairs, no model change

* **F1 · The strip overlay claims only chips.** Fixes T4 and T5, and completes
  the T3 fix. `hitTest` returns `nil` unless the point is over a chip and the
  event is a plain left press.
* **F2 · Equal pane widths for n > 2.** Fixes P2. Deliberately an interim fix:
  divide the space by pane count when there are more than two, keep `ratio` for
  the two-pane case. S3 replaces it with real per-child weights, and this is
  the shape that makes P5 fixable at all.
* **F3 · An opening overlay reclaims the keyboard.** Fixes X1. Does *not*
  address X2, which turned out not to be a established finding at all.

Each is independently verifiable in the running app, which is how they were
found — and all three are done and verified there.

### Phase 1 — S2, one owner for the focused pane — **DONE**
Demoted below Phase 0 because P10 showed no visible fault. Writing its gate
found one anyway: restoring a pane re-derived scroll from the caret, so any
pane scrolled with the wheel snapped to the top when focus came back.

### Phase 2 — S3, the layout tree — **DONE**
Both audit-derived jobs landed. `splitEditorLayout` no longer computes geometry
at all — core ships a normalised rect per pane and the face places them
absolutely, finding dividers where rects share an edge. P5 was the gate and it
is met.

### 3.3 The terminal: four faults, all now fixed

Found while verifying J6. None were caused by the split work — all reproduce
with a single unsplit pane — but they made the terminal unusable, so they are
recorded together.

**Contrast.** `terminalGridBg` was the editor background nudged 3.5% toward
black, which in the light theme is very nearly white, while core paints the
shell's default foreground as rgb(200,200,200): a contrast ratio of about
1.35:1. The terminal rendered perfectly and was invisible. The caret had the
same problem from the other side — it was drawn in the *editor's* foreground,
near-black, which is why the shell appeared to have no cursor. Both now use a
terminal palette: dark grid, light ink, in either theme.

**Scrolled past the prompt.** The trailing-blank trim tested `row == " "`,
which was true back when core sent bare spaces for empty rows. Core wraps every
row in colour escapes now, so nothing was ever trimmed: a fresh shell handed
over ~40 rows of decorated blanks, the canvas grew to fit them all, and
stick-to-bottom scrolled straight past the one row with the prompt. The blank
test now strips escapes first, and stick-to-bottom requires the view to have
been scrollable at all — it was trivially "at the bottom" on the very first
apply, when the canvas still had zero height.

**Keyboard.** Two separate holds. Clicking gave the terminal focus in the
engine but never took the window's first responder, so keystrokes kept landing
in the project tree's filter (§3.1 again); `TermCanvas` now accepts and claims
first responder itself. And the face's `terminalOwnsKeys` only recognised the
*docked* shell's mode, so with a terminal pane focused printable keys went down
the editor's insert path — the pane received Enter (not printable, so it fell
through to the raw dispatch) and nothing else, which looked like a shell that
answered every keystroke with a bare new prompt.

**Case.** `resolveCharacter` preferred `charactersIgnoringModifiers`, the
UNSHIFTED key — right for chords (⇧⌘T is "t"), wrong for typing, so no capital
could be produced on any surface that uses the key monitor. The editor escaped
it only because its canvas takes the `NSTextInputClient` path. Core then had to
learn to re-apply the SHIFT modifier when writing a letter to a PTY, since its
key model carries case as a modifier rather than in the character.

### 3.4 The docked terminal (⌃T)

Three faults, two of them mine from §3.3's fix.

**The button was a keystroke.** "Open Terminal · ⌃T" and the dock's ✕ did not
call anything — they synthesised a ⌃T key event. That worked only while nothing
else claimed the key, and a focused terminal pane claims it correctly (⌃T is a
readline binding), so the control silently stopped working and the panel sat on
an empty state you could not dismiss. `suisei_engine_toggle_terminal_dock` now
exists and the controls call it. A button should call the thing it names.

**Empty dock unreadable** *(regression from the dark grid)*. `terminalDockFill`
is the grid's own colour, deliberately — the grid does not cover the dock shape
exactly, and every point of difference used to show as a seam. Once the grid
went genuinely dark, the empty state's `.tertiary` prompt was near-black on
near-black. The fill now only wears the grid's colour when a grid is there.

**Header unreadable while running** *(same cause, other half)*. The header sits
on that one-colour fill, so with a shell running its theme-derived ink was
dark-on-dark. It now follows the fill rather than the editor theme, which keeps
the seam guarantee intact instead of trading it away.

### Still open
`terminal.pane_bound` is a pane **index** into a list whose order the tree can
change — the last positional handle in this area, and S4's job. J6 (pane →
terminal) and J7 (layout tabs) stay where the plan puts them.
