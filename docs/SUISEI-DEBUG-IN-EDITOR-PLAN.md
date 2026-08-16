# Debugging you can see in the editor

User's observation: most editors put everything in the debugger panel and leave
the editor unmarked, and that is the wrong split — "이게 에디터에 시각적으로
보여야 한다 생각함". Asked for a design that is native rather than a port of
VS Code's.

Written 2026-08-17 against `suisei-core/src/dap.rs`, `compositor/scene.rs`,
`EditorHost.swift` and `QuickHelpPopover.swift`.

---

## 0. What is there now

Measured, because half of this turns out to be built already.

| | state |
|---|---|
| Breakpoint in the gutter | **drawn** — sign bit `0x40`, a yellow chip behind the line number (`EditorHost.swift:1623`) |
| The stopped line | **not drawn anywhere** |
| Call stack, variables, console | panel only |
| Datatips (value on hover) | none |

And the stopped line is not missing for want of data. Core computes it, and it
wrote the accessor **for this exact caller**:

```rust
/// Stopped line if the session is currently stopped in `path`.
pub fn current_line_for(&mut self, path: &str) -> Option<usize>   // dap.rs:533
```

Nothing calls it. The value crosses the ABI too — `current_path`,
`current_line`, `has_location` are written into the snapshot
(`ffi.rs:4861`) and decoded into `DapSnap` on the Swift side
(`EngineBridge.swift:6623`) — and then nothing paints them.

So this is the same shape as the rest of this session, one step further along:
not "a fact core holds that never crosses the ABI", but a fact that crosses the
whole way and dies one line short of a pixel.

---

## 1. What Apple actually does, and the principle under it

Xcode, while stopped:

- a green band across the stopped line, and a **solid arrow** in the gutter;
- a **hollow arrow** when you select a parent frame in the call stack;
- **datatips**: hover a variable, get its value in a small popover with a
  disclosure triangle to walk into it;
- breakpoints as blue **pennants** — drag one off the gutter to delete it,
  ⌘-click to disable (it goes hollow);
- the **jump bar** naming the frame you are looking at.

What Xcode does *not* do is put the variables list in the editor. The principle
is worth stating because it decides everything below:

> **The editor answers "where am I". The panel answers "what is true".**

Position is spatial, and the editor is the only spatial surface in the app. A
value is not spatial — until you point at the thing it belongs to, which is
what a datatip is.

---

## 2. The design

### 2.1 The gutter carries meaning by SHAPE, not by hue

The git bar already owns colour in that strip: green added, red deleted,
filled/hollow for staged. So a debugger that spoke in colour would be arguing
with it — and colour alone fails Increase Contrast and every colour-blind user
anyway.

- **solid ▶** — the program is here.
- **hollow ▷** — a frame you selected that is not the top of the stack. This is
  the distinction only the editor can draw, and it is why "just show the
  stopped line" is not enough: while you walk the call stack, the editor should
  follow you, and it must be obvious that what you are looking at is *not*
  where execution is.
- **breakpoint** — the pennant, filled or hollow for enabled/disabled. The chip
  already exists; it becomes a shape with two states.

### 2.2 The stopped line is a band, and NOT the caret-line wash

Suisei already washes the caret's row (`current_line` theme token). The stopped
line must not reuse it: a caret line is where *you* are and a stopped line is
where the *program* is, and if the two look alike the editor is lying about
which one it is showing. They are also frequently the same row, and the band has
to survive that.

Full width, behind the text, and it stays when the editor loses focus — the
program is still stopped whether or not you are looking.

A new theme token `debug_stop`, appended (`ThemeToken`'s own rule: "append,
never insert", because the index is ABI).

### 2.3 Datatips are Quick Help with a different source

`QuickHelpPopover.swift` already exists: a themed card that measures its own
height and caps it, renders code spans in mono, and highlights the term it is
about. It was built for LSP hover on right-click.

While stopped, hovering an identifier asks **DAP `evaluate` in the selected
frame** instead of asking the language server for documentation. Same surface,
same theming, same measured card — a different provider.

This is the highest-value thing in this document and the cheapest, because the
hard part was built for something else. It also matches Xcode exactly: the
datatip's disclosure triangle walks into a struct, which the panel's variables
tree already knows how to do.

### 2.4 Reveal, do not merely mark

Stopping has to bring the line on screen. Xcode scrolls to it and pulses it
once. The precedent for the timing is already here: the bracket-match marker is
span kind 254 and its comment says the FACE owns the ~1s flash so that core
stays stateless. The stop pulse is the same arrangement.

Reduce Motion turns the pulse off; the band and the arrow are what carry the
information, and neither moves.

### 2.5 The jump bar names the frame

The breadcrumb is already there — `Users › asill › suisei › test.rs ›
goddddddd`. While stopped it should show the frame, and picking a frame in the
panel should move the breadcrumb, the hollow arrow and the scroll together.
One selection, three surfaces agreeing.

### 2.6 Inline values — last, and here is the honest cost

VS Code's grey `x = 5` at the end of the line. It is genuinely loved and it is
the most expensive thing here: an `evaluate` per variable per visible line, re-run
on **every step**, plus a new span kind and a place in the layout for text that
is not in the buffer. It also fights soft wrap.

Worth doing, worth doing last, and worth a budget before a line of it is
written.

---

## 3. What not to do

- **Do not move the variables tree into the editor.** The panel is right for it;
  §1's principle is the reason.
- **Do not distinguish anything by colour alone.** The gutter is already spoken
  for, and this is the surface Increase Contrast users need most.
- **Do not reuse the caret-line wash for the stopped line.** §2.2.
- **Do not evaluate on the tick.** A datatip is a request with a hover's
  latency budget, not a frame's. The DAP client is already polled; this joins
  that, it does not block it.

---

## 4. The part that will otherwise be forgotten

A stopped line that exists only as a coloured band is **invisible to
VoiceOver** — and the editor canvas has no accessibility at all today
(`EditorHost.swift`'s only mention is an `NSImage`'s
`accessibilityDescription: nil`; it is already an open item in feature.txt).

This is the one feature whose entire purpose is "the editor tells you where you
are", so shipping it as pixels only would be exactly the wrong feature to leave
silent. At minimum: announce the stop, and give the stopped row a label.

---

## 5. Staging

**E1 — the stopped line.** Band plus solid gutter arrow, driven by
`current_line_for`, which core already wrote for this. Reveal and pulse. This is
the whole of the user's complaint and it is mostly wiring something that already
computes.

**E2 — the selected frame.** Hollow arrow, jump bar, and one selection moving
the panel and the editor together.

**E3 — datatips.** Quick Help's card, DAP `evaluate` as the provider.

**E4 — the breakpoint gutter grows up.** Pennant with an enabled/disabled state,
drag off to delete, ⌘-click to disable, right-click for a condition. Core has
conditions and logpoints already (`condition_and_log_on_breakpoint`); nothing in
the face reaches them.

**E5 — inline values**, with the budget from §2.6 agreed first.

E1 is small and answers the question that was asked. E3 is the one that will
change how the debugger feels.
