# Showing where a value lives, and when it moves

User's sketch: a `[` — but stretched down a long run of lines, so it is mostly
a vertical rule with a short cap at each end — marking how a value moves while
the debugger runs.

---

## 0. The shape already has a name here

Generally that is a **span bracket** (or range bracket) — a bracket whose job is
to say "all of this belongs together" rather than to enclose an expression.
Music notation calls the tall one joining staves a bracket for the same reason.

But this app already draws exactly it, and calls it something: the **capped
gutter bar**. `compositor/scene.rs`:

```text
0x10  first row of its hunk     → the bar caps here
0x20  last row of its hunk      → the bar caps here
```

and the face draws it with `runTopCap` / `flush(bottomCap:)`
(`EditorHost.swift`). So the git change bar IS the shape, and a value extent
would be the same vocabulary applied to a second thing — which is the argument
for doing it: the user already reads that mark as "this run, together".

---

## 1. "How a value moves" is two questions

They need different machinery and only one of them is a debugger question.

### 1.1 Where does it live? — static, and cheap

The declaration, every read, every write, and the last use. That is
`textDocument/documentHighlight`, which returns a **kind** per occurrence:
`Text`, `Read`, `Write`.

Core does not request it. It has `request_references`, which is the WRONG
request for this: references is a workspace-wide search built for "find every
caller", it costs a project scan, and it does not say read from write.
documentHighlight is one file, one round trip, and its whole purpose is
highlighting occurrences of the symbol under the caret.

So: bracket from the first occurrence to the last, cap at each end, and a
**tick on the writes**. Reads are where the value is used; writes are where it
moves. That distinction is the feature, and it comes free in the response.

### 1.2 When does it change? — runtime, and the real answer

A static bracket shows where a value *could* change. Only the debugger knows
where it *did*.

**DAP data breakpoints — watchpoints.** Measured on this machine's adapter:

```text
$ strings $(xcrun -f lldb-dap) | grep -i databreakpoint
dataBreakpointInfo
setDataBreakpoints
supportsDataBreakpoints
```

They are available and Suisei uses none of them. `dataBreakpointInfo` asks the
adapter whether a variable can be watched; `setDataBreakpoints` arms it; the
program then stops with `reason: "data breakpoint"` on the line that wrote to
it. That is literally "tell me when this value moves", and it is the one thing
in this document that cannot be approximated by reading the file.

(The same capability list also reports `supportsSetVariable`, which is the
panel being able to CHANGE a value while stopped. Not this feature, but it is
sitting there unused too.)

---

## 2. What makes it a debugger feature and not an editor one

An extent bracket on its own is a nice editor nicety. What makes it worth
drawing *while stopped* is that the stop line **cuts it in two**:

- above the stop — where this value has already been;
- below the stop — where it is going.

That is a fact only the debugger knows, it is spatial, and the editor is the
only spatial surface. It is the same argument
`SUISEI-DEBUG-IN-EDITOR-PLAN.md` makes for why the variables list stays in the
panel and a datatip does not: position belongs to the editor.

So the drawing is not one bar but a bar with a break in it, and the reader gets
past/future without a legend.

---

## 3. Where it goes, and the problem with that

The gutter is nearly full. Left to right today:

```text
[git stripe][ breakpoint chip + line number ][stop arrow][ code ]
```

and this session already shipped one collision — the stop arrow was placed in
the git stripe's lane and had to be moved into the text gap.

Three options, and the third is probably right:

1. **Its own gutter lane.** There is no room without widening the gutter for
   every file, including the ones never debugged.
2. **Reuse the git stripe.** No: a line that is both modified and inside the
   extent then has two meanings in six points, which is the collision that was
   just fixed.
3. **In the text, at the code's left edge.** A hairline rule just inside the
   text area with caps at the ends. It is about the CODE, not about the file's
   git state, so it belongs on the code's side of the gutter — and it costs the
   gutter nothing.

Option 3 also composes with the stop band, which starts at the same edge: the
rule sits over it and the break in the rule lands exactly on the band.

---

## 4. Costs, honestly

- **A round trip per symbol.** documentHighlight is per-file and fast, but it
  is still a request per caret move, and this session has spent a lot of time
  on O(file) work per keystroke. It has to be debounced and cached by
  (buffer version, symbol), and it must not run at all when the debugger is not
  stopped.
- **The extent can be enormous.** A `static` used across four hundred lines
  draws a bracket down the whole file, which is noise, not information. Cap the
  drawn extent at the visible band plus a margin, and let the caps fall off
  screen rather than compressing them.
- **Watchpoints are scarce hardware.** x86 and ARM give four; the adapter will
  refuse the fifth. `dataBreakpointInfo` says whether one can be set, so the UI
  must ask before offering, and say why when the answer is no. Silently doing
  nothing on the fifth watchpoint would be the worst version of this.

---

## 5. Staging

**V1 — the extent bracket.** documentHighlight in core, occurrences with their
Read/Write kind across the ABI, a capped rule at the code's left edge for the
symbol under the caret. No debugger involvement; useful on its own and it
proves the drawing.

**V2 — cut it at the stop.** While stopped, break the rule on the stop line so
past and future read apart. This is the part the user asked for.

**V3 — watchpoints.** `dataBreakpointInfo` + `setDataBreakpoints`, offered from
the same right-click that will grow breakpoint conditions in E4. A watched
value's ticks become live: the bracket then shows where it moved, not where it
might.

**V4 — set variable.** `supportsSetVariable` is already advertised. Editing a
value in the panel is a different feature that the same capability check
unlocks; noted here so the capability is not discovered twice.

V1 is the shape, V2 is the debugger, V3 is the answer to the question that was
actually asked.
