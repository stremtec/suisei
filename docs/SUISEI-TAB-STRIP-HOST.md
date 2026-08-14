# The tab strip: where it lives

Written 2026-08-12, after two sessions in which every tab-strip defect turned
out to be in the same unspecified layer.

Companions, both still accurate and both still implemented:

- `SUISEI-TAB-STRIP-BEHAVIOUR.md` — the **model**: what the strip is a view of,
  what a click means, what close means. Implemented by `TabStripModel`.
- `SUISEI-TAB-STRIP-GEOMETRY.md` — the **geometry**: chip positions are computed
  from `(tabs, font, viewport, scrollOffset)`, never measured. Implemented by
  `TabStripLayout` + `TabChipMetrics`, with headless tests.

Those two layers each got a document, then a rewrite, and they have not produced
a defect since. This document is the third layer, which never got either:

> **The host.** Where the strip's viewport width comes from, who owns a mouse
> press inside it, and how the row moves when its position changes.

Every open defect is in that layer. They are not separate bugs; they are three
unstated rules.

---

## 1. The three defects, and the rule each one is missing

### D1 — the strip trembles, and the hit lands left of the glyph

**Measured**, by making the app log its own strip geometry: on every sidebar
toggle `topBarBody`'s `geo.size.width` sweeps 1120 → 820 → 1120. Not the
sidebar's width — the *whole row's* reported width, inside a `GeometryReader`
that sits in the animating hierarchy.

Everything downstream is arithmetic on that number:

```
geo.size.width  →  wideCap  →  viewport  →  originX  →  chip.x  →  close rect
```

so the entire strip — chips, band, pill, "+", and the × hit rect — is recomputed
on every frame of a sidebar animation. That is the tremble, and it is why a
press during (or just after) the animation lands somewhere the glyph is not.

Four fixes were tried against this and all four were misdiagnoses:
`navLiveWidth`, `navW`, `navIdealWidth`, and moving `topBar` out of the root
`ZStack` into an `.overlay`. The last one was verified against the log and did
**not** stop the sweep, which also disproves "a `ZStack` takes its size from its
largest child".

> **Missing rule H1.** The strip's viewport width must not be read from a view
> that resizes when the sidebar does.

### D2 — draw and hit disagree while the row moves

`TabStripRow` is a SwiftUI `Layout` with:

```swift
var animatableData: CGFloat {
    get { layout.originX }
    set { originX = newValue }        // private override
}
```

so during any animated change SwiftUI re-runs `placeSubviews` per frame with an
**interpolated** `originX`. The hit test is `TabStripMouse`, an AppKit view whose
closures call `layout.closeSlot(...)` — and that `layout` is the one captured at
body-evaluation time, holding the **final** `originX`.

The drawn row and the hit rects are therefore in different places for the whole
duration of every scroll, re-centre, merge and sidebar toggle. Nothing connects
them; there is no mechanism by which they could agree.

*(Reasoned from the code, not measured. It is consistent with the reported
symptom — "히트 위치가 렌더 위치보다 왼쪽" — but it has not been isolated on its
own, and it should be, before anything is built on top of it.)*

> **Missing rule H2.** The thing that paints a chip and the thing that hit-tests
> it must read the same `originX` at the same instant.

### D3 — the "+" cannot be clicked

The strip sits in the window's titlebar region (`.fullSizeContentView`, hidden
titlebar). AppKit drags the window from any view there whose
`mouseDownCanMoveWindow` is true, and it consumes the press **before** SwiftUI
gesture arbitration runs. So `TabStripMouse.Catcher` sets
`mouseDownCanMoveWindow = false` and claims presses — and its `hitTest` returns
`self` only where `slotAt` finds a chip, precisely so the `+` and the gaps fall
through.

They fall through to a `Color.clear` carrying a `WindowDragGesture`, which is
switched off by `.allowsHitTesting(!tabStripHover)`. Over the `+`,
`tabStripHover` is true, so that layer declines too — and the SwiftUI `Button`
underneath the catcher never receives anything.

Three participants (`Catcher`, the drag layer, the `Button`) each decide
independently whether a press is theirs. Every combination has been tried;
"tried five gesture shapes and a `zIndex`, each probe-confirmed to receive
nothing" is in the source as a comment.

> **Missing rule H3.** Exactly one object decides who owns a press inside the
> strip's rect. Everything in the strip — chips, ×, "+", gaps — is a rect that
> object owns and dispatches.

---

## 2. What the strip does today

The inventory a rewrite is checked against. Everything here currently works
unless marked **[broken]**.

### 2.1 Chip content

| part | rule |
|---|---|
| icon | `doc.text.fill`; `square.on.square` when merged; `exclamationmark.triangle.fill` (orange) when deleted on disk |
| title | 12pt system, **`.semibold` when active** — so chip widths depend on the selection; struck through in orange when deleted |
| trailing slot | 14pt square: dirty ● (6pt orange) when `dirty && !hovered`; close × (8pt bold in a 14pt circle) when hovered |
| box | `HStack(spacing: 5)`, `.padding(.horizontal, 10)`, `.frame(height: 24)` — all four now from `TabChipBox` |
| ink in a layout group | black (0.72 for dim) instead of accent/primary/secondary |

### 2.2 Chip states

| state | drawing |
|---|---|
| active | a single capsule the strip draws, `primary.opacity(0.10 / 0.14)`, travelling between chips |
| hovered, not active | capsule `primary.opacity(0.06 / 0.10)`, or `black.opacity(0.08)` inside a group |
| dragging | scale 1.06, opacity 0.9, shadow (r 7, y 2), on top |
| pressed | scale 0.96 |

### 2.3 Furniture

- **Group band** — one `RoundedRectangle(12)` per grouped run, systemBlue fill
  0.22/0.32 with a 0.45/0.55 stroke, from the run's own computed extent.
- **"+"** — 22×26, visible only while the pointer is in the strip, parked at
  `layout.plusX` (run trailing edge + 4pt gap, clamped to `viewport − 22`).
  Menu: New Untitled Tab · Next Tab · Previous Tab — Split Editor Right · Split
  Editor Below · Focus Next Pane · Close Focused Pane. **[broken: D3]**
- **Overflow counter** — `+N` at `plusX + 22` when the engine reports more tabs
  than the ABI cap carries.
- **Edge fade** — 14pt linear-gradient mask at each end; chips rest inside it.
- **Band** — the row is 26pt inside a 48pt titlebar band, dropped 2pt.

### 2.4 Input

| gesture | result |
|---|---|
| click a chip | `TabStripModel.click` → focus document / focus member / activate layout |
| click × | `TabStripModel.close` → close document / drop layout |
| double-click a grouped chip | `toggleLayoutStyle(group)` |
| drag > 3pt | pick up; swap with a neighbour once past its **midpoint**; 6pt travel debounce between swaps; moves by stable id, not slot |
| hover | one lookup, published only when the slot changes |
| horizontal scroll | trackpad `deltaX`; a plain wheel's `deltaY` × 24; shift-wheel. Unconsumed at either end falls through |
| fast vertical flick | `|dy| > |dx| × 1.5` and ≥ 6pt (trackpad) or ≥ 1 detent (wheel) → fold up / down one step. One step per physical gesture; the momentum tail is locked out |
| right-click / ctrl-click | context menu: [Show Layout as Group ⇄ Merge Layout into One Tab] · Unfold Layout — Close Tab · Close Other Tabs |
| press on empty strip | drag the window; double-click zooms |
| active tab changes | scroll to centre it, if it is not already fully visible and no structural motion is running |

**[broken]** trackpad vertical delta also drives the strip sideways — a plain
vertical scroll over the strip scrolls it horizontally.

**[broken]** the close glyph's hover releases on small vertical drift: the
tracking area is the catcher's 24pt bounds inside a 26pt row.

### 2.5 Motion

| what | curve |
|---|---|
| structural change (`tabStripPresentationKey`) | `engine.tabStructuralAnimation` |
| merged-layout chip in | fade + scale 0.96, `easeOut 0.10` delayed 0.07 |
| group member in / out | bare fade, `easeOut 0.10` / `0.07` |
| loose document in / out | fade + scale 0.94 |
| hover capsule | `.snappy(0.16)` |
| press | `.snappy(0.12)` |
| ● ⇄ × | `.easeInOut(0.14)` |
| "+" fade / move | `.easeOut(0.12)` / `.snappy(0.22)` |
| auto-reveal scroll | `.snappy(0.2)` |

---

## 3. Rules the host must obey

**H1 — the run is anchored to the window, not to the corridor.**
The strip is centred on the **window**: `contentLayoutRect.midX`, which does not
move when the sidebar opens. This is the behaviour asked for directly — "탭 바가
사이드바가 열리든 뭐든 창 기준 중앙" — and it supersedes
`SUISEI-TAB-STRIP-GEOMETRY.md` §4's `originX = (viewportW − contentW) / 2`,
which centres on the viewport.

> **Amended.** H1 first said the sidebar's width is "an input to *nothing*
> here", and the strip took a constant keep-out per sidebar state. That is too
> strong, and it cost the two complaints the constant could not answer: a
> 146pt hole beside a closed navigator, and overflowing tabs underneath a
> widened one. The sidebar's width IS an input — to the **corridor**, the span
> in which chips may be drawn. It is not an input to the **anchor**.
>
> `TabStripLayout` takes a `preferredCentre` and clamps it to the corridor. A
> run that fits sits on the window's centreline no matter what either boundary
> does; only a corridor too narrow to hold a centred run pushes it clear, which
> is exactly the case where a boundary must win. `testSidebarSweepDoesNotMoveTheRun`
> asserts both halves across a full open/close sweep.
>
> And the corridor reads the **settled** sidebar width, never the live one.
> `SplitColumnWidthReporter` republishes on every frame of the animation and
> every pixel of a splitter drag; `ContentView.settleNavWidth` waits 120ms of
> quiet before committing, so a toggle produces one step at its end instead of
> twenty during it. Feeding `navLiveWidth` straight through — the failure this
> rule was written for, and one of the four values named below as already
> tried — put the sweep back in June's shape.

**H2 — one `originX`, one instant.**
Whatever moves the row must be readable by the hit test at event time. Either
nothing interpolates privately (the row is always placed at `layout.originX`,
and smooth motion comes from animating the *input*, so every frame's layout is
the truth for both), or the painter publishes the origin it actually used and
the hit test reads that. Not both, and never neither.

**H3 — one press owner.**
One object hit-tests the strip's whole rect and dispatches: chip, ×, "+", gap,
empty. No SwiftUI `Button` inside the strip, no second gesture layer, no
`allowsHitTesting` toggled by hover state.

**H4 — the × rect is derived, not restated.** *(done — `TabChipBox`)*
`closeSlot` used to hard-code `maxX − 24`, a hand-copy of `horizontalPadding +
trailingSlotWidth`. Changing the chip's padding moved the drawn × and left the
hit rect behind. The box is stated once and the chip, the width and the hit rect
all read it; `TabStripLayoutGeometryTests` asserts the hit rect **is** the drawn
slot grown by the hover slack.

---

## 4. Two routes

### Route A — replace the host, keep the model and the geometry

Delete `TabStripRow`, `TabStripMouse` and the `topBar` width arithmetic. One
`NSView` draws the chips *and* hit-tests them, using `TabStripLayout` for both,
with its own animation clock. Width from `NSWindow`.

- H1, H2, H3 all fall out of the structure rather than being maintained.
- Chip drawing moves to CoreText + CALayer; transitions become `CAAnimation`.
  The context menu gets *easier* (`menu(for:)`).
- ~700–900 lines new, ~600 deleted.
- The hand-tuned transitions in §2.5 have to be rebuilt and re-verified with
  `SUISEI_ANIMATION_TRACE=1`. That is the real cost and it is not small.

### Route B — fix the three rules in place

1. **H1**: take the width from `NSWindow.frame.width` via a tiny observer, drop
   the `GeometryReader`. Centre the run on the window, offset into the strip's
   own space.
2. **H2**: delete `TabStripRow.animatableData`; drive `tabScroll.offset` with an
   owned animator so the body re-evaluates per frame and both sides read the
   same layout every frame.
3. **H3**: give `Catcher` the "+" rect and the plus action; drop the SwiftUI
   `Button` and the `allowsHitTesting(!tabStripHover)` dance.

- ~150–250 lines changed, nothing re-tuned. Every existing animation survives.
- Each step is independently verifiable, and step 2 can be tested on its own
  before anything is built on it.
- Leaves the strip a SwiftUI/AppKit hybrid — H2 and H3 stay rules someone has to
  keep, rather than facts the structure guarantees.

**Recommendation: B, in that order, one commit each.** Not because A is wrong —
A is the better end state — but because D2 is still *reasoned*, not measured.
B's step 2 is the cheapest possible experiment that confirms or kills it, and if
it is wrong, A would have been 900 lines spent on the wrong cause. Two sessions
were lost to exactly that mistake. Measure first, then decide whether A is still
worth its price.
