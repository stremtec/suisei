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

### 1.1 Panes address content by **index**, and indices move

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

### S1 · Stable buffer ids *(no visible change)*
Add `BufferId` to `BufferTab`, a `next_id` counter on `App`, and a
`buffer_index(BufferId)` lookup. Convert `Pane.tab_index` to
`Pane.buffer: BufferId`. Everything else keeps working through the lookup.

**Gate:** open four tabs, split, close the *first* tab. Both panes still show
what they showed. Today they shift by one. Regression test in
`suisei-core`.

### S2 · One source of truth for the focused pane
Move `scroll`/`hscroll`/`cursor` ownership entirely into the pane and make the
`App` fields read through. Delete both sync functions.

**Gate:** split, scroll one pane, switch focus twice, come back — position is
exactly where it was. Today this is the most common "unstable" report.

### S3 · Layout tree
Replace `kind` + flat `panes` + `ratio` with the `Layout` tree and per-child
weights. Keep the existing key bindings pointing at the new model.

**Gate:** vertical split, then horizontal-split one side; three dividers all
drag independently; `MAX_PANES` becomes a total-leaf cap rather than a
one-direction cap.

### S4 · `PaneContent`, and terminals with ids
Introduce `PaneContent`, move terminals into `terminals: HashMap<TerminalId,
Terminal>`, delete `full_panel` / `pane_bound` / `owns_split`.

**Gate:** two terminal panes open simultaneously — impossible today.

### S5 · J6 as specified
`convert_focused_pane_to_terminal()`:
1. Take the focused pane's `PaneContent::Document(id)`.
2. Ensure that document is present in the tab bar (it already is — the tab bar
   lists buffers, not panes), and make it the active tab if no other pane shows
   it.
3. Replace the pane's content with `PaneContent::Terminal(new)`.
4. No split is created and none is destroyed.

Closing that terminal pane restores the pane to the document it displaced, or
collapses the pane if the user closes the pane itself. `owns_split` is not
needed because no split was conjured.

**Gate:** with a 2×2 layout, convert one pane to a terminal and back; the other
three panes do not move, and the displaced file is reachable from the tab bar
throughout.

### S6 · Layout persistence (J7 depends on this)
With a `Layout` tree and stable ids, serialising a layout is straightforward,
which is what J7's "layout tab bar" needs. Not part of this plan beyond making
it possible.

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
