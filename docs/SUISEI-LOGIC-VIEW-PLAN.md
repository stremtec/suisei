# Logic View

User's specification, in full, in feature.txt ☐26. In short: not a flowchart of
one file, but a hierarchical view of a whole program's logic — Project → Module
→ File → Function → Statement, collapsed by default, wired to the debugger so
it shows the path actually taken, and the surface an AI edits through.

This document is what the codebase can and cannot answer today, and what the
gap costs.

---

## 0. Four inputs. Three already exist.

The spec names its inputs: AST, control flow, call graph, symbol resolution.

| | state |
|---|---|
| **AST** | `syntax.rs::live_tree()` — a live tree-sitter tree per file, already reparsed incrementally on every edit |
| **Call graph** | `lsp.rs::request_call_hierarchy` with `CallItem` and `CallDirection` (incoming/outgoing) — already wired |
| **Symbols** | `documentSymbol` and `workspaceSymbol`, both requested today |
| **Control flow** | **nothing. Does not exist anywhere in the tree.** |

So this is one missing capability, not four — and the missing one is the hard
one.

### 0.1 And a surprise that matters

The Outline — the app's current structure view — is **not built from the AST**.
`compositor/scene.rs::build_outline` walks the buffer line by line looking for
`#` headings, `fn `, braces and indentation, with a `max_items` of 200. It is a
text scan.

That is worth knowing for two reasons. Logic View cannot be built on it: a
string scan cannot tell a call from a comment mentioning one. And building
Logic View properly gives the Outline a real foundation as a by-product — the
same node tree, read at a shallower depth.

---

## 1. The control-flow gap, and why it is not thirty extractors

tree-sitter gives a **concrete syntax tree**, not a control-flow graph. Turning
`if` / `match` / `loop` / `?` / `return` / `break` into decision nodes and edges
is a set of rules, and the rules are per grammar. Suisei ships about thirty.

Thirty hand-written CFG extractors is not a plan. The shape that scales is a
**table per language naming node kinds**:

```
rust:
  decision:  if_expression  match_expression  while_expression  loop_expression
  loop:      for_expression while_expression loop_expression
  call:      call_expression macro_invocation
  exit:      return_expression  try_expression  break_expression
  block:     block
```

A new language is then a table entry, not a module — the same shape the
highlighter already uses to map a grammar's captures onto `TokenKind`, and the
same reason it has thirty languages without thirty highlighters.

It will not be perfect for every grammar. That is fine, and §4 is how it stays
honest about it.

---

## 2. The hierarchy is the feature

The spec is emphatic that Logic View does not expand everything, and that is
not only a UI preference — it is what makes the thing affordable.

A whole-project CFG is not something to compute eagerly: it is every function
in every file, and it invalidates on every keystroke. Collapsed-by-default
means **the CFG is built for what is expanded and nothing else**, which turns
an intractable analysis into a per-function one on a tree that is already
parsed and already incremental.

So the collapse is load-bearing twice: it is what makes a hundred-file program
readable, and it is what makes it computable.

The levels map onto what exists:

- **Project / Module** — the file tree and `workspaceSymbol`.
- **File** — `documentSymbol`.
- **Function** — a node in the AST; the boundary at which a CFG is built.
- **Statement** — inside that CFG.

Crossing into another file is a node named for that file or module, and
expanding it is what triggers the next parse. Nothing below a collapsed node is
computed.

---

## 3. Runtime: what is free, what is not

This part is nearly done already, and the parts that are not have to be said
plainly rather than promised.

**Free, and already in the tree:**

- **The node executing now.** `dap.current_line` and `current_path` exist and
  are already painted in the editor; mapping a line to the node whose source
  range contains it is a lookup.
- **The path through the CALL graph.** The call stack is exact and already
  decoded — `frames`, with path and line each. Which functions we came through
  is not an inference.
- **Variables at a node.** The frame's scopes are already in the panel, and the
  datatip already resolves a single name inside a chosen frame.
- **A breakpoint on a node.** Node → source range → the breakpoint machinery,
  which now has conditions, log messages and an off switch.

**Not free:**

- **Which BRANCH was taken inside a function.** DAP does not report execution
  history. Three honest options, in increasing cost: infer it when the CFG has
  only one path from the function's entry to the stopped node (often true, and
  it must say "inferred" when it is); record it by stepping, which changes the
  program's timing; or instrument, which changes the program.
- **Loop iteration counts.** There is no counter in DAP. A breakpoint's
  `hitCondition` is a filter, not a report. A logpoint on the loop header would
  give it at the cost of I/O per iteration.
- **Branches not executed.** This is the complement of the above and inherits
  the same limit: "not executed" is only knowable if "executed" is.

The spec asks for all of these. Two of the three are inference or
instrumentation, and the view must label which — a path drawn as fact when it
was guessed is the worst failure this feature can have.

---

## 4. The discipline that decides whether it is trustworthy

**Every node names its exact source range**, and

**anything the extractor does not understand appears as an opaque node** —
`⋯ 3 statements` — rather than being dropped.

A flowchart with a missing branch is worse than no flowchart. It is a confident
lie about control flow, and a person will act on it. A grammar this
extractor has never seen, a macro that expands to a loop, a language feature
the table does not cover: all of them have to show up as "there is something
here I did not read", visibly.

This is the same rule the model viewer arrived at from the other side — a
malformed STL that returns an empty scene must not be shown as an empty stage,
because "nothing here" and "I could not read this" need different reactions.

---

## 5. AI integration, and one inversion

The spec has the AI edit through the logic rather than through lines, showing a
Logic Diff first and touching source on approval.

The mapping node → source range makes that possible, and applying an approved
change is `edit.rs`'s existing range edits.

But the direction has to be inverted from the way the spec describes it:

> **Generate the source edit; RENDER it as logic.**

Not: generate a logic change and then re-derive source from it. Only the first
can guarantee that what the user approved is what lands — the second reviews an
abstraction and applies a translation of it, and a translation with any
ambiguity in it means the approved diff and the written diff are two different
things.

So the Logic Diff is a *view of a real patch*, produced by the same extractor
that draws the Logic View, on the before and after text.

---

## 6. Staging

**L1 — the node tree, one function.** The per-language kind table, a CFG for a
single function from the live tree, and a static view of it. No hierarchy, no
runtime, no AI. This is where it becomes clear whether the table approach
carries a real grammar, and it is cheap to abandon if it does not.

**L2 — the hierarchy and the collapse.** Project → Module → File → Function,
computing nothing below a collapsed node. This is the part that makes it a
product rather than a demo.

**L3 — the runtime overlay.** Current node, the call-stack path, variables at a
node, breakpoints from a node. All of it reads state that already exists; §3's
"not free" list stays out and is labelled as absent.

**L4 — inferred paths**, with "inferred" on the drawing wherever the CFG had
more than one way to reach the stopped node.

**L5 — Logic Diff**, per §5: patch first, rendered as logic.

L1 is the honest first question and it answers a real risk. Everything after it
depends on the answer.

---

## 6a. What it should LOOK like

Asked directly: how does this fit Suisei rather than fight it. The answer comes
out of what the app already draws, not out of what flowcharts usually look
like.

### It is not a canvas

The reflex is boxes and bezier edges on a pannable surface — draw.io, Figma,
every "code visualiser" screenshot. That is a different app's aesthetic and it
would be the only thing in Suisei that works that way: nothing else here pans,
nothing else is laid out by a graph algorithm, and nothing else asks the user
to find things by scrolling in two dimensions.

Suisei is a **document** app. Lists, rails, monospace, capped bars. And a
program's logic is *mostly a vertical sequence* — which is what a document is.

### The spine is the shape it already has

The git change bar and the value bracket are the same object: **a vertical rule
with caps, meaning "this run, together"**. The user already reads it that way.

So Logic View is a **spine with rows on it**:

```
  ┌  fn process
  │
  ├─ let mut total = 0
  │
  ├─ ◇ n > 10
  │  ├─ Yes ─ total = n
  │  └─ No  ─ return 0        ⤫
  │
  ├─ ↻ for i in 0..n
  │  └─ total += i
  │
  └─ total                    →
```

A branch splits the spine into two indented spines that rejoin; a loop is a
spine with a back mark; an exit ends its spine rather than rejoining. That is
the capped-bar vocabulary extended to a second subject, which is exactly the
argument for reusing it — a second visual language for "these belong together"
would be one more thing to learn.

### Indentation is the hierarchy

Project → Module → File → Function → Statement is disclosure, and the app has
that idiom twice already: the file tree and the model workbench's outline. A
closed node is `▸ [Authentication Module]`, and opening it indents its contents
under it. Nothing new to explain.

### Type and colour come from the app

- **Labels are source text**, so JetBrains Mono, which ships. A paraphrase
  would be a second thing that can disagree with the code, and the code is what
  the reader is being helped to understand.
- **Section headers** uppercase, 9pt, tracked — `WBSection`'s, which is already
  the app's word for "this is a section".
- **Structure** in the theme's `fg`/`dim`. **Opaque** nodes in `dim` with `⋯`,
  visibly less certain than the rest, because they are.
- **Runtime in amber** — the debugger's colour, the one the stop band, the
  breakpoint chip, the datatip and the inline values all now share. A node lit
  in Logic View and a line lit in the editor are then obviously the same fact,
  which is the entire point of wiring the two together.

### Where it lives

A **pane**, like the model viewer — not the right inspector and not the bottom
dock.

The inspector is where the Outline lives and Logic View is "the outline, but of
logic", which argues for it. But the inspector is a column: a branch needs
width, and this is a surface you *work in* rather than glance at. The bottom
dock is the debugger's and this is not only for debugging.

A pane also gets the pairing for free: Logic View in one split, code in the
other, clicking a node moves the other pane's caret. Which is the whole
interaction — and it is why every node carries its exact source range.

### The one thing to resist

Auto-layout. The moment nodes are positioned by an algorithm, the same function
looks different after an edit, and a reader loses the map they had built. Rows
in source order, indentation for nesting, and nothing moves that the code did
not move.

---

## 6b. Second placement: the right rail, and marks in the editor

§6a put this in a pane and argued the inspector was too narrow. The user wants
the right rail, and wants Logic View wired into the editor the way the debugger
now is. Both are right, and the width objection has an answer §6a missed.

### What §6a got wrong

The objection was "a branch needs width". That is true **only if the column has
to show the code**. In a pane it does — the code is not on screen, so the view
must reproduce it, and reproducing indented source in 240pt is hopeless.

In the right rail the editor is showing the same file **at the same time**. So
the column does not carry the text at all: it carries the SHAPE, and the editor
carries the TEXT. That is the division of labour the debugger already uses —
the panel holds the tree of frames and variables, the editor holds the marks —
and it makes the narrow column *better* than the wide pane rather than a
compromise: two surfaces each doing the thing they are good at, instead of one
surface doing both badly.

Everything else in §6a stands: the spine, source text for labels, shape before
colour, amber for runtime, and no auto-layout.

### The column

Compact, no title row (the mode strip already says which inspector this is —
`inspectorPanel` makes that rule and it is right), rows at 22pt, depth indent
10pt, and a right-aligned line number in 9pt mono like the Outline's, which
also gives the eye a stable right edge.

```
▾ ƒ  process(n)              12
  ◇  n > 10                  14
  │ Y  total = n             15
  │ N  ↩ 0                   17
  ↻  for i in 0..n           19
  │    total += i            20
  ⤺  total                   22
▸ ƒ  main()                  26
```

Labels are source text, truncated at the tail — the full line is one glance to
the left, which is the entire argument for this placement. `Y`/`N` instead of
`YES`/`NO`: at this width a two-letter chip costs a word of label.

What the column does NOT get: the file name, the language, a "stopped here"
banner. Those are either in the editor already or in the debug panel already,
and a rail this narrow cannot afford to say anything twice.

### The editor side — five marks, and who owns which lane

"Integrated like the debugger" means marks on the code itself. The debugger
draws five: the stop band, the breakpoint chips, the value bracket, the inline
values, the datatip. Logic View's five, and the lane rules that keep them from
fighting:

1. **The selected node's body, as a guide.** A hairline down the node's own
   indentation column, spanning its rows, with a small foot. Accent, not amber.
2. **The selected node's first row**, faintly banded in the accent — the same
   "you are pointing at this" the rail's selection means.
3. **While stopped: the path you are inside.** Nested amber guides for every
   branch and loop body the stop is within. This is the one genuinely new fact
   in the editor — a debugger shows the line, never "you are inside this loop
   inside this else" — and it is EXACT, read off ranges, not inferred.
4. **On hover of a decision row**, both arms tinted for as long as the pointer
   is there: consequence and alternative, so a branch can be read without
   counting braces. Transient, so it can afford to be loud.
5. **Right-click a line → Reveal in Logic**, which selects that row. The
   editor's own way of asking the question the rail answers.

**The lane rules.** `gutterTextGap` — the twelve points between the number and
the code — belongs to the value bracket, which lives there because a mark at a
fixed x cannot follow indentation. Logic's guides have the opposite property:
a node's body has ONE indentation, its own, so the guide belongs *in the text
column at that indent* and moves with the code by construction. Nothing of
Logic's goes in the gutter lane, so the two never collide.

Colour: **amber is the debugger's**, and Logic borrows it only for runtime
facts, which ARE the debugger's facts. Structure is `dim`. Selection is the
accent. Git keeps green/red/blue in the gutter. One family per subject, still.

### Both directions, because it is one selection

- **Caret → rail.** Moving the caret selects the row whose range holds it, and
  reveals it if it is inside something closed. `logic::row_at` already answers
  this; the Outline's version is `cursorRow == item.row`, an exact-match test
  that lights nothing for 95% of lines, and this is the containment answer it
  should have had.
- **Rail → caret.** Clicking a row moves the caret, in the pane that is already
  showing the file. `reveal_logic_row` already does this.

### The ABI route, which is forced

`SuiseiEditorLineC.debug_sign` is a byte and all eight bits are spent
(stopped, frame, extent, first, last, write, breakpoint-disabled,
breakpoint-decorated). So the marks do NOT ride the line struct.

They come from a pull over the visible band — `logic_marks(first, count)` —
the same shape `inline_values(first, count)` already uses for the editor's
value annotations, and for the same reason: it costs nothing when the overlay
is off, and it needs no ABI surgery. Each entry is a row, a flag byte, and the
column to draw the guide at, computed in core from the text (a body's minimum
indentation) so the face does no analysis.

### What happens to the pane

It stays, demoted. It is the same rows, the same row view and the same
snapshot, so it costs one thin wrapper — and a wide read of a long function is
a real thing to want. It loses ⌃⌘L to the rail and moves to "Open in a Pane"
inside the rail. If nobody opens it in a month of use, delete it and keep
`FileKind::Logic = 7` reserved.

### Outline and Logic are two modes, for now

They overlap and they are not the same list: the Outline covers types,
constants and Markdown headings, and Logic covers control flow. The tempting
merge — one mode where expanding a function reveals its logic — conflates
"where things are" with "what happens", and would make the fast flat list pay
for the deep one.

Keep them separate, and take the by-product §0.1 promised anyway: the Outline
is a line-by-line text scan with a 200-item cap, and the same tree Logic reads
is a real foundation for it. That is a separate change and it stands on its
own.

### Cost, and the two ways to get it wrong

The rail is ALWAYS visible, which the pane was not. Two consequences:

- **Do not parse twice.** For the focused document the syntax engine already
  holds a live, incrementally-reparsed tree. The session must use it and keep
  its own parse only for a file that is not the focused one — otherwise every
  file switch pays a second full parse on the main thread.
- **Do not compute when nobody is looking.** The rail has four modes and three
  of them are not this one. Everything here is gated on the mode being visible,
  the same way the debug panel gates its own marks on `dap.panel_open`.

### Staging

**U1 — the mode.** Logic as a fourth inspector mode, drawing the column from
the snapshot that already exists. Caret → rail follow. Nothing in the editor
yet. This is the placement decision, testable in an afternoon.

**U2 — selection in the editor.** The `logic_marks` pull, the selected node's
guide and band. Rail → caret already works. This is where the two surfaces
start behaving like one.

**U3 — the stopped path.** Nested amber guides for the branch and loop bodies
the stop is inside. Reads `LogicRuntime`, which is already built and tested.

**U4 — the branch peek.** Hover a decision, both arms tint. Transient.

**U5 — the Outline's foundation.** Replace the text scan with the same tree.
Independent of the rest, and it removes a 200-item cap and a class of wrong
answers (a call in a comment).

### What would change my mind

If the column at 200pt turns out to need the code after all — if reading the
shape without the text is not actually possible while the eye is on the editor
— then the division of labour is wrong and the pane was right. U1 is deliberately
the cheapest possible test of exactly that, and it comes before any editor work.

---

## 7. What this is not

Not a flowchart generator. The spec says so and it is worth keeping in front:
the value is in the hierarchy and the runtime link, and a tool that draws one
pretty function and cannot open a project is the thing this is explicitly not.
