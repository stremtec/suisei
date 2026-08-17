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

## 7. What this is not

Not a flowchart generator. The spec says so and it is worth keeping in front:
the value is in the hierarchy and the runtime link, and a tool that draws one
pretty function and cannot open a project is the thing this is explicitly not.
