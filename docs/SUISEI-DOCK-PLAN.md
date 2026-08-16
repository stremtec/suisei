# Docking, and tearing a pane into its own window

Plan for feature.txt **☐12** ("Docking 기능 구현"), whose whole spec was one
line. The user's clarification: docking **includes** pulling a pane out into an
independent window.

Written 2026-08-17 against `suisei-core/src/split.rs`, `layout_tab.rs`,
`session.rs`, `suisei-engine/src/ffi.rs` and the SwiftUI scene roots. Every
claim below was read out of the tree rather than remembered; where a comment in
the tree says otherwise, that is called out.

---

## 0. The fact that decides the architecture

`EngineBridge.shared` is a singleton holding one `suisei_engine_new()`
(`EngineBridge.swift:1239`, `:1514`), and `WindowGroup("Suisei", id: "editor")`
already exists (`SuiseiApp.swift:71`) — so macOS can open a second editor
window **today**. It would be a mirror: both windows are
`EditorSceneRoot(engine: engine)` over the same engine, so both show the same
`SplitState` — the same tree, the same focus, the same scroll.

Independent windows need independent desk state, and there is exactly one desk.
Everything else in this document follows from that.

---

## 1. Three architectures, and why one

**(A) One engine, N desks.** `App` grows a map from desk id to `SplitState`.
Buffers, LSP, git, DAP, terminals stay shared and single. Tearing out is moving
a `Pane` from one desk's tree into a new desk's tree.

**(B) N processes, one desk each, coordinated by the daemon.** The daemon's
per-client aggregation (`suisei-daemon/src/state.rs`) makes this look like the
intended shape. It is not one for a text editor: two processes means two copies
of every buffer, two rust-analyzer instances on the same crate graph, and the
same file open twice with no protocol to keep the two honest. The daemon
aggregates *projects*, which is a different question.

**(C) One desk; extra windows are views onto a subtree.** Cheapest and wrong. A
pane that is "torn out" while still in the parent's tree is not torn out —
closing the parent orphans it, and every operation that walks the tree has to
learn which parts are elsewhere.

**(A).** The rest of this section is why the tree is already shaped for it.

### 1.1 A pane addresses its document by stable id, not by position

```rust
pub struct Pane {
    pub id: PaneId,
    pub buffer: BufferId,   // stable — NOT an index
    pub scroll: usize,
    pub hscroll: usize,
    pub cursor: (usize, usize),
}
```

`split.rs:43`. This is the single most important precondition and it is already
paid for: `SUISEI-SPLIT-PLAN.md` §1.1 is the story of when it was `tab_index:
usize` and every tab close silently repointed every pane after it. **A pane can
move to any tree, in any window, and the document does not notice.**

### 1.2 Lifting a whole subtree out of the live desk is a solved operation

`LayoutTab` (`layout_tab.rs:37`) parks a `Layout` tree **and** the `Pane`s it
names, then puts them back — and its own comment says why it holds both: "each
carrying its document, scroll and cursor. Parked with the tree because they were
snapshotted together; restoring is a lookup, not a rebuild."

Tearing out is that operation with a different destination. It is not a new
capability; it is an existing one pointed somewhere else.

`LayoutTab.docs` also carries a warning worth reading before writing any of
this: it documents three defects that all came from consulting a *snapshot* of
which documents a layout holds while the layout was live. The same trap is
waiting for a desk — "which documents are in window 2" must be read off window
2's live panes, never off a remembered list.

### 1.3 Shells are already window-agnostic

`TerminalSessions.shared` (`TerminalSurface.swift:282`) keys live sessions by
`TerminalOwner` — `.tab(tabId)` or `.dock(id)`. **No pane. No window.** A shell
already survives being moved between panes, and would survive moving between
windows unchanged, because nothing in its key mentions where it is being drawn.

This is also the pattern the viewer panes need (see §2.5), and it is proof the
pattern works in this codebase.

### 1.4 Session restore is wired, and already saves the arrangement

`SuiseiApp.swift:95` says window restoration is off because "core
`restore_session` is unwired". **That comment is stale.**
`EngineBridge.init()` calls `suisei_engine_restore_session(engine)`
(`EngineBridge.swift:1517`), core saves through `session::save` (`app.rs:1099`),
and `Session` carries `split: Option<SessionSplit>` with a `SessionPane { tab,
scroll, … }` per pane (`session.rs:44`).

So restoring an arrangement is an existing, working mechanism. Restoring *N*
arrangements is one more field on `SessionPane`, not a new subsystem. That moves
restore from "the thing that turns docking from a week into three" to "a day at
the end", which is a large part of why (A) is affordable.

---

## 2. What has to change

In cost order.

### 2.1 The ABI addresses panes by visual index

```c
uint8_t  suisei_engine_pane_path(const SuiseiEngine*, uint32_t idx, ...);
uint64_t suisei_engine_pane_tab_id(const SuiseiEngine*, uint32_t idx);
void     suisei_engine_split_resize(SuiseiEngine*, uint32_t pane_a, uint32_t pane_b, float);
```

and the snapshot is a fixed `SuiseiPaneC panes[SUISEI_MAX_PANES]` with a
`pane_focus` byte (`suisei_engine.h:146-165`). Every one of those `uint32_t`s is
a **position in visual order** — `split.rs:158` says so outright: "the index the
compositor, the FFI and the face all speak."

With two desks, "pane 2" is ambiguous. This is precisely the disease core was
cured of in S1, still living one layer out.

Two ways out:

- **Add a desk to the address.** The snapshot gains a desk id; pane calls become
  `(desk, idx)`. Appending to a snapshot is the established discipline — the
  `model_bg` token did exactly that this week, and `ThemeToken`'s comment
  ("append, never insert") is the rule.
- **Address panes by `PaneId`.** The honest fix, and it deletes the ambiguity
  instead of qualifying it. Wider than docking strictly needs on day one.

Take the first for D2 and the second as its own cleanup, or the ABI churn lands
twice.

### 2.2 `MAX_PANES = 4` is app-wide

`split.rs:21` caps a `SplitState` at four leaves and the ABI mirrors it as a
fixed array. Four panes **per desk** is what a user expects; four **in total
across every window** is not. Once the snapshot is per desk (§2.1) this falls
out, but it is worth deciding on purpose rather than discovering.

### 2.3 Core must not learn what a window is

Core has no window notion at all today, and that is right. The unit should be a
**desk** — a layout root — and the face decides which desks are windows.

That keeps core testable without a windowing system, and it means a desk can
also become a `LayoutTab` (folded into the tab strip) rather than a window, with
no second concept. Docking and layout tabs then differ only in where the face
puts the desk.

### 2.4 One keyboard, N focused panes

`SplitState` holds `focus: PaneId` — per desk that is correct and stays. But
with N desks there are N focused panes and only ONE of them has the keyboard, so
there must also be an app-level **key desk**. Everything that says "the current
pane" — the status bar, ⌘S, the command palette, Quick Help — means *the key
desk's focused pane*.

This is worth stating loudly because this codebase has already shipped the bug
twice, from two directions, and both are written down:

- a terminal's keyboard claim that ran while the view was still out of the
  window and was never retried (`TerminalSurface.swift`), and
- gating the edit path on a panel's `open` flag instead of on the mode, which
  quietly sent typing to the vim machine.

A wrong answer here is a keystroke landing in the wrong window, which is the
worst failure this feature can have.

### 2.5 Viewer panes lose their state when the view moves

A model pane's `ModelDocument` is a `@StateObject` inside the pane's view
(`ModelViewer.swift:26`). Move that pane to another window and SwiftUI rebuilds
the view — the workbench's selection, its unexported edits and its camera are
gone. Same shape for the PDF's scroll position and the audio playhead.

The fix is the one already in the tree: a registry keyed by **tab id**, exactly
as `TerminalSessions` does it (§1.3). Viewer state is per-document, not
per-view, and it has been per-view only because nothing moved before.

Refusing to tear out a dirty viewer is the alternative, and it is worse — it
makes the feature conditional on which pane you grabbed.

---

## 3. The gesture

Docking has three verbs, and they should be one gesture:

1. Drag a tab chip **within its own strip** → reorder.
2. Drag it **onto another pane** → move the document there.
3. Drag it **out of any window** → a new window holding it.

One drag; the destination decides. That is what Xcode, VS Code, JetBrains and
every DAW do, and it is why none of them needs a "Detach Pane" command for the
user to find.

**The drop overlay.** Over a pane, five zones: four edge strips and a centre.
An edge splits that pane on that axis with the dragged document on that side;
the centre opens it into that pane. The overlay is the only new drawing this
needs, and it is what makes the two split axes discoverable without a menu.

**Outside any window** → a new desk, a new window at the cursor, sized to the
source pane.

**Reverse, symmetric.** Dragging a chip from a torn-out window back onto a pane
re-docks it; when a torn-out window's last chip leaves, the window closes. If
tearing out is a gesture and putting back is a menu, users stop tearing out.

**No drag exists yet.** The only `onDrag` in the face is the split divider
(`ContentView.swift:5842`); the tab strip has no drag source and no drop target.
This is genuinely new face work — which is why it is staged last, behind the
structural work that can be tested without it.

---

## 4. Staging

**D1 — Desks in core, one window.** `App.split: SplitState` becomes
`App.desks: { map, key }`. Every existing behaviour is desk 0. No UI, no ABI
change, no face change. Tests: a pane moved between desks keeps its buffer,
scroll and cursor; a desk whose last pane closes disappears; `MAX_PANES` is per
desk. This is where the model is proved, in a place with no windowing system in
it.

**D2 — The ABI takes a desk.** Snapshot per desk, pane calls `(desk, idx)`.
The face still shows one. Nothing visible changes, which is the point.

**D3 — A second window, no drag.** "Move Pane to New Window" in the pane header
menu opens a window on a new desk. **This is the whole feature minus the
gesture, and it ships on its own.** It also forces every hard question — desks,
ABI, key desk, viewer state — to be answered before any drag code exists.

**D4 — The drag.** Tab chips become drag sources, panes become drop targets with
the five-zone overlay, drop-outside makes a window. Re-dock and auto-close.

**D5 — Restore.** One desk id on `SessionPane`, and an arrangement of windows
comes back. Cheap because §1.4 already works. Also the moment to delete the
stale comment at `SuiseiApp.swift:95` and turn `.restorationBehavior` back on.

D3 is the honest MVP: it delivers "독립창으로 빼기" with a menu item and pays for
all the structure. D4 is what makes it feel like a real editor.

---

## 5. What not to do

- **Do not give core a `Window` type.** Desks, and the face maps them. §2.3.
- **Do not read "which documents are in that window" off a snapshot.** Read the
  live panes. `LayoutTab.docs` documents three defects that were all this.
- **Do not open a second `SuiseiEngine`.** That is (B) wearing (A)'s clothes:
  two buffer stores, two language servers, one file.
- **Do not ship the drag before the desk.** A drag over a mirrored window
  reorders a tree that another window is also showing, and the symptom appears
  in the window you were not looking at.
