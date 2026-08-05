# Rebuilding editor splits and the terminal pane

Plan for field issues **J5** (editor split is unstable) and **J6** ("turn this
pane into a terminal" is unstable). Both are called out as needing a rewrite
rather than repair, and reading the current code agrees: they are unstable for
*structural* reasons, and each individual bug fixed in place would be replaced
by another.

Written 2026-07-26 against `suisei-core/src/split.rs` (280 lines) and the
terminal-pane code in `app.rs`.

---

## 1. Why it is unstable — the actual mechanisms

### 1.1 Panes address content by **index**, and indices move — *fixed in S1*

```rust
pub struct Pane {
    pub tab_index: usize,   // ← index into App.buffers
    ...
}
```

Every pane points at a tab by position. Closing a tab, opening one, or (once
J4 lands) **reordering the tab bar** shifts every index after it, and every
pane holding one of those indices is now showing a different file. There is no
repair for this short of changing what a pane holds.

The same pattern appears again in the terminal:

```rust
pub terminal.pane_bound: Option<usize>   // ← index into split.panes
```

and it already has hand-written patch-up code for the case where a pane
disappears (`app.rs`: `Some(b - 1)`), which is the tell — the model is being
corrected after the fact instead of being correct.

### 1.2 The focused pane's state lives in **two places**

`App` holds `buffer`, `scroll`, `cursor`, `filename` for the active document.
`split.panes[focus]` *also* holds `scroll`, `hscroll`, `cursor`. They are kept
in step by hand through `sync_split_from_active()` and
`sync_active_from_split()`, called at every site that might have changed
either. Any path that forgets one leaves the two disagreeing, and the symptom
appears later and somewhere else — a pane that scrolls back to a stale
position, a cursor that jumps on focus change.

### 1.3 One `kind` for the whole layout, so no mixed splits

```rust
pub kind: SplitKind,        // None | Vertical | Horizontal — for ALL panes
pub panes: Vec<Pane>,       // flat
```

`SplitAdd::MixedKind` exists purely to refuse "split this pane the other way".
A flat vector cannot express it. Every real editor layout is a tree.

### 1.4 One `ratio` for any number of panes

```rust
/// Divider position for the 2-pane case (drag-resize); ≥3 panes are equal.
pub ratio: f32,
```

Three panes cannot be resized at all. There is one divider position and there
are two dividers.

### 1.5 The terminal's location is encoded in four fields that can disagree

`open`, `full_panel`, `pane_bound`, `owns_split` — plus `Mode::Terminal` — all
describe "where is the terminal and who owns it". Nothing enforces a
consistent combination, and `owns_split` is the terminal storing a fact about
*why the split exists* so it can undo it later.

There can also only ever be **one** pane-bound terminal, because `App.terminal`
is a single `Terminal`.

### 1.6 J6 is not implemented as specified

The requested behaviour is: **convert the focused pane into a terminal, and
send the document that was there back to the tab bar.** What
`open_terminal_window` actually does is *create a new split* and put a terminal
in the new pane. The document is never displaced, nothing returns to the tab
bar, and if the user was already split it behaves differently again. This is
not a bug to fix; the feature does not exist yet.

---

## 2. Target model

One change underpins everything: **a pane holds content, not an index, and
content is addressed by a stable id.**

```rust
/// Stable for the lifetime of the document/terminal. Never reused.
pub struct BufferId(u64);
pub struct TerminalId(u64);

pub enum PaneContent {
    Document(BufferId),
    Terminal(TerminalId),
}
```

and the layout becomes a tree:

```rust
pub enum Layout {
    Leaf(PaneId),
    Split { axis: Axis, children: Vec<Layout>, weights: Vec<f32> },
}
```

* `weights` per child replaces the single `ratio`, so any number of dividers is
  draggable.
* `axis` per node replaces the single `kind`, so mixed layouts fall out for
  free and `SplitAdd::MixedKind` is deleted.
* `PaneContent::Terminal` makes "this pane is a terminal" a *state of the pane*
  rather than four fields on `App.terminal`. `full_panel`, `pane_bound` and
  `owns_split` all go.
* Multiple terminals become possible because terminals live in a map keyed by
  `TerminalId`, exactly like buffers.

And the focused pane's state stops being duplicated: `App.buffer`/`scroll`/
`cursor` become *accessors* that read through to the focused pane's document,
not copies of it. `sync_split_from_active` / `sync_active_from_split` are
deleted rather than made more careful.

---

## 3. Ordered steps, each shippable

Nothing here is a big-bang rewrite; each step leaves the app working.

### S1 · Stable buffer ids — **DONE**
`BufferId` on `BufferTab`, a `next_tab_id` counter on `App`, and a
`buffer_index(BufferId)` lookup. `Pane.tab_index` became `Pane.buffer:
BufferId`; the paint path resolves id → index at its boundary, so the C ABI
(`SuiseiPaneC.tab_index`) is unchanged.

Two things fell out that the step did not predict:

* `SplitState::clamp_tabs` is gone. It clamped *out-of-range* pane indices,
  which is why closing tab 0 of four was silent — the indices stayed in range
  and simply named different files. Its replacement, `repoint(gone, adopt)`,
  handles the one case ids actually have: the document a pane was showing is
  closed. It resets that pane's scroll and cursor too, since those were
  coordinates in a file that no longer exists.
* `Engine::close_tab` ended with `sync_focused_pane_tab()`, which pushed the
  newly active document *into* the focused pane — so closing any tab dragged
  the focused pane along even after the ids landed. It now calls
  `apply_focused_pane()`: the pane keeps its document and the editor follows
  it, which is also the direction S2 takes.

**Gate — met.** Four tabs, split, close the first: both panes still show what
they showed. Covered by `closing_a_tab_leaves_the_other_panes_on_their_own_documents`
(core) and `closing_a_tab_leaves_split_panes_painting_their_own_files`
(engine, through the real paint path), and confirmed by hand in the packaged
app.

`App::current_buffer` is still a position. That is deliberate: it is repaired
synchronously by the one function that reshuffles the vector, and S2 removes it
outright by making the focused pane the owner.

### S2 · One source of truth for the focused pane — **DONE**
Landed the other way round from the sketch above, and better for it. Rather
than making `App`'s fields read through to the pane (83 call sites), **`App`
*is* the focused pane** and the slots hold the others. The focused slot is
stale by design, so nothing needs syncing while the user works; park and load
happen in `focus_pane_to`, the only way focus changes. All twenty-odd
`sync_*` calls are gone, as are three of the four functions.

This is the rule the compositor already followed for the *document* — it reads
`current_buffer` for the focused pane and the slot only for the others. The
duplication was the anomaly.

Writing the gate turned up the bug behind the actual complaint: restoring a
pane finished with `update_scroll()`, which re-derives scroll from the **caret**.
The wheel does not move the caret, so a pane scrolled with the wheel snapped
back to the top the instant focus returned. That is "a pane that scrolls back
to a stale position", verbatim.

**Gate — met.** `a_pane_keeps_its_viewport_across_focus_changes`, verified to
fail against the old code (0 instead of 40).

### S3 · Layout tree — **DONE**
`kind` + `ratio` are gone. `SplitState` holds a `Layout` tree with a per-child
weight list; `panes` survives as a flat list kept in **visual order**, derived
from the tree, so index-addressed callers (compositor, FFI pane array, face)
needed no rework.

The face no longer derives geometry at all. Core computes a normalised `Rect`
per pane and ships it in `SuiseiPaneC` (`rect_x/y/w/h`, +16 bytes per pane —
the ABI guard moved with it). The face places panes absolutely by those rects
and finds dividers by looking for panes whose rects share an edge, which gives
N−1 independent dividers for free.

Three things fell out that the step did not predict:

* **Row budgets are per pane now.** The old code picked one `rows_each` for the
  whole layout from `kind` — full height for a vertical split, `rows/n` for a
  horizontal one. A tree has both at once, so one number cannot describe it.
* **Repeated same-axis splits equalise.** Halving only the focused pane gives
  1/2, 1/4, 1/8, 1/8 for four "split right"s, which is not what anyone means by
  splitting four ways. New siblings redistribute evenly, vim-style.
* **`park_focused_pane` had to stop skipping the unsplit case.** With the tree
  there is always exactly one pane, and the first split copies the focused
  pane's slot into the new pane — so a slot that was never parked handed the
  fresh pane an empty document.

`focus_dir` is genuinely directional now: it picks the nearest pane that lies
that way and overlaps perpendicular, instead of stepping ±1 along the single
axis and refusing the other two keys.

**Gate — met**, in the running app: the four-pane `+` (impossible before —
`SplitAdd::MixedKind` refused it outright), and the left column's divider
dragged from y=424 to y=600 while the right column's stayed at 424.
`MAX_PANES` is now a total-leaf cap.

### S4 · `PaneContent` — **DONE**, with one part deliberately deferred
`PaneContent::{Document(BufferId), Terminal { restore }}` on the pane, and
`full_panel` / `pane_bound` / `owns_split` deleted. "Where is the terminal" is
now one fact in one place — the layout tree — and the two values the face still
asks for (`full_panel`, `pane_bound`) are **computed** from it, so the C ABI and
the face were untouched.

`close_split`'s `Some(b - 1)` arm is gone. It shifted `pane_bound` down when a
lower pane closed, which §1.1 called out as the tell: a model being corrected
after the fact instead of being right. A pane that closes takes its terminal
with it; there are no indices to repair.

**Multiple concurrent terminals — also done**, after being deferred once on
the argument that "nobody has asked for it". That was wrong, and it was wrong
in a way worth recording: with one shared `App.terminal`, a second terminal
pane was a second *view* of the same session, and converting a second pane
**moved** the shell rather than starting one. The user's report was "두개
에디터 전부 각각 터미널 띄우는게 작동 안함 … 터미널이 연동되는것같은 오류" —
both symptoms are that single shared PTY, so the deferral did not scope the
work down, it left the feature broken.

`PaneContent::Terminal` now carries a `TerminalId`, and `App.pane_terminals`
holds one `Terminal` per pane. `App.terminal` is the **docked** shell (⌃T)
only. Every pane shell is polled on tick, so a build in one keeps running while
you work in another. The face pulls rows per pane through
`suisei_engine_terminal_for_pane`; the pane's `is_terminal` flag rides in a
byte that was already padding, so `SuiseiPaneC` did not change size.

**Gate — met.** `two_terminal_panes_are_two_shells` (engine), and by hand: two
panes, `echo LEFT_ONLY` in one and `echo RIGHT_ONLY` in the other, each showing
only its own output.

### S5 · J6 as specified — **DONE**
`convert_focused_pane_to_terminal()`:
1. Take the focused pane's `PaneContent::Document(id)`.
2. Ensure that document is present in the tab bar (today it already is — the
   tab bar lists buffers, not panes; once S6 lands that stops being automatic,
   see §S6), and make it the active tab if no other pane shows it.
3. Replace the pane's content with `PaneContent::Terminal(new)`.
4. No split is created and none is destroyed.

Closing that terminal pane restores the pane to the document it displaced, or
collapses the pane if the user closes the pane itself. `owns_split` is not
needed because no split was conjured.

`toggle_terminal_full` is now exactly this. It converts the focused pane in
place: no split is created, none is destroyed, and the displaced document stays
open — the strip lists buffers, not panes, so it is already there. It is made
the active tab so it is obvious where it went.

**Gate — met.** `terminal_takes_over_the_focused_pane_and_leaves_the_file_reachable`
(engine) pins the pane count, which pane runs the terminal, that the other pane
is untouched, that the document is still open, and that toggling restores.
Confirmed by hand: with a 2-pane split, ⌃⇧T turned the focused pane into a
Terminal, the other pane kept its file, the tab strip kept the document, and
the pane count never changed.

**Product invariant — the `Terminal` tab is intentional.** Converting the
focused pane also creates a `Terminal` entry in the top tab strip. That entry
is the terminal leaf's stable identity, not an accidental document and not a
request for a separate macOS window. It must participate in S6 layout groups
so a layout containing terminal panes can fold, switch away, restore and
unfold through the same tab model as a document-only layout. Do not “fix” this
by hiding or suppressing the Terminal tab.

**Not verified: the shell inside the pane is usable.** The PTY starts and the
engine's snapshot carries the prompt (checked in-process), but the grid paints
no text in the running app and keystrokes land in the project tree's filter —
see `SUISEI-TAB-AUDIT.md` §3.3. Both are terminal-subsystem faults, on code
paths this step did not touch, and both reproduce with a single unsplit pane.

### S6 · Layout tabs (J7) — **DONE**

A fast upward flick over the tab strip folds whatever the editor is showing
into one entry. The four transitions land as designed: the **fold is silent**
(the layout tab is active, so nothing on screen changes), **leaving clears the
desk** (the editor comes down to that one document while the arrangement waits
in its tab), **coming back restores it** including each pane's scroll, and a
fast flick down **unfolds**.

`LayoutTab` parks S3's tree *and its panes* — snapshotted together, so
restoring is a lookup rather than a rebuild. That is the payoff for building
this on S3: a layout tab needs no state of its own.

**Two strip shapes**, switchable from the tab's right-click menu:

* **Grouped** — the documents keep their chips and the strip draws one rounded
  grey container around the run, so you can still see what is in there.
  Clicking any member chip restores the arrangement: the group *is* the layout.
* **Unified** — a single chip carrying the layout's name.

Both are expressed with two fields per chip (`tab_groups`, `tab_is_layout`)
rather than a second concept in the strip, so the drag, the travelling capsule
and the context menu all keep working unchanged.

Three things this turned up:

* **The gesture had to be claimed at hit-test time.** The strip's AppKit
  overlay is transparent to everything that is not a left press (that is what
  restored the `+` and the context menu), so scroll events never reached it.
  It now claims *only* a dominantly-vertical fast flick — which means ordinary
  horizontal tab scrolling falls through by never being claimed, instead of
  being forwarded afterwards and hoping.
* **The velocity floor's unit depends on the device.** A trackpad reports
  points and drifts; a wheel reports detents, and a points floor is
  unreachable for it — so a mouse could not fold at all.
* **`scrollingDeltaY` is content-space** and its sign flips with the "natural
  scrolling" preference, which would make the gesture mean the opposite thing
  on half the machines. `isDirectionInvertedFromDevice` turns the delta back
  into an intent.

**Gate — met**, by test (`folding_parks_the_arrangement_and_leaving_clears_the_desk`,
plus refusal on a single pane and the style toggle) and by hand: fold a
two-pane split, switch to another tab and watch the editor come down to one
pane, click a member chip and get the arrangement back.

---

## 4. What this costs, honestly

S1 and S2 are mechanical and low-risk, and between them they remove the two
mechanisms behind most of the reported instability — index invalidation and
duplicated pane state. **They are worth doing even if the rest is deferred.**

S3 is the real work: the layout tree touches the compositor's pane geometry,
the FFI `SuiseiPaneC` array, and the Swift `splitEditorLayout`. It is the step
that needs its own careful pass, and `SUISEI_MAX_PANES` in the ABI header is
part of it.

S4–S5 are small once S1–S3 exist, and impossible before them.

The UI complaint ("UI도 구리고") is deliberately not addressed here — divider
styling, pane headers and focus affordance should be designed against the tree
model once it exists, not retrofitted to the flat one.
