# The tab strip: why its coordinates are unstable, and what to do instead

Written 2026-08-04, after a run of tab-strip defects that were each diagnosed,
patched, and followed by another one: clicking a grouped member navigating to a
different file, a merged layout trapping the whole strip, hover reporting the
wrong chip, and stutter that worsens with tab count and with overflow.

Those were treated as four bugs. They are one, and it is architectural.

---

## 1. The single cause

**The strip does not know where its chips are. It asks, and the answers arrive
at different times.**

Every geometric fact about the strip is a separate SwiftUI measurement, each
published by its own `.onGeometryChange`, each landing on its own schedule:

| what | where it comes from | consumers |
|---|---|---|
| each chip's rect (row space) | `.onGeometryChange` per chip | grouped band |
| each chip's rect (strip space) | `.onGeometryChange` per chip | hit-testing |
| the chip row's origin | `.onGeometryChange` on the row | *was* hit-testing |
| the chip row's width | `.onGeometryChange` on the row | "+" placement |

Nothing guarantees these agree. SwiftUI offers no atomic "here is the layout
now" snapshot — a value read during an event handler is a *mixture of different
frames' timestamps*. Combining two of them is a race, and every strip bug so far
has been that race surfacing somewhere new.

### The race, measured

`SUISEI_TABLOG=1`, two clicks of a single double-click, pointer stationary:

```
TABLOG x=61   rowOriginX=187   →  README.md   <<HIT
TABLOG x=234  rowOriginX=14    →  Cargo.lock  <<HIT
```

Raw pointer x was 248 both times. The row origin moved **187 → 14 between two
clicks of one gesture** — a 173pt disagreement, about one and a half chips. The
first click selected a chip the cursor was nowhere near.

That is the reported "hover A, get C", and it is not a logic error anywhere. The
arithmetic is correct; the *inputs* are from different moments.

### Why it gets worse with more tabs

Each chip owns 2 geometry callbacks, and each callback writes `@State`, and each
`@State` write invalidates `ContentView`'s body. The chip row is a plain
`HStack`, not a `LazyHStack`, so **every** chip is built and measured on **every**
layout pass, including chips scrolled out of sight.

So the per-pass cost is O(tabs) callbacks → O(tabs) invalidations → a body
rebuild each. The number of *opportunities* for two measurements to disagree
also grows with tab count. Both the stutter and the wrongness scale together,
which is exactly what the user reports.

### Why it gets worse when tabs are obscured

Overflow adds a scroll offset — a third asynchronous input, produced by
`ScrollView` internals nobody owns. While scrolling, the row origin changes
every frame, so the window in which the origin and the chip frames disagree is
no longer a transient after a layout change: it is *continuous*.

### Why integrating layouts is the worst case

Folding, merging and unfolding change the row's width abruptly — 93pt on merge
by the existing note in `ContentView`, 173–281pt observed for the origin. A
centred `ScrollView` re-centres in response. So the single moment when the
measurements disagree most is precisely the moment a layout integrates, which is
where the instability was reported.

---

## 2. Why the patches did not hold

Each fix targeted one consumer of the racing inputs:

| patch | what it fixed | why it did not end the class |
|---|---|---|
| `tabSlot` scans live tabs, narrowest wins | non-deterministic `Dictionary.first(where:)` | frames were still from mixed passes |
| chips publish a strip-space frame | removed the origin conversion | the frames themselves still race with the scroll offset |
| `rememberTabFrame` equality guard | fewer redundant writes | fewer races, not zero |
| `chipRowOrigin` as a reference box | removed a per-frame publish | the value was still a second timestamp |

The pattern is unmistakable in hindsight: every fix removed *one* of the
disagreeing inputs, and the defect reappeared through whichever one was left.

---

## 3. What correct implementations do

The editors whose tab strips feel solid do not measure their chrome. They
**compute** it.

- **VS Code** (`TabsTitleControl`): tab widths are computed in the model from
  label text and a fixed sizing policy; the scroll offset is explicit state the
  control owns. Layout is a pure function of (tabs, font, viewport, scroll).
- **Zed**: the whole UI is GPU-drawn from a computed layout tree; chrome
  geometry is never read back from a view system.
- **Terminal emulators** (Ghostty, Alacritty, kitty): everything derives from a
  computed cell grid. Nothing asks where a glyph "ended up".
- **AppKit** custom tab bars: an `NSView` subclass computes child frames in
  `layout()` and hit-tests against the frames it just assigned.

The common property is not the toolkit. It is that **one authority assigns
geometry and everyone else reads that same assignment** — so a hit test and a
paint can never be looking at different moments.

Suisei already applies this principle in the place it matters most. The editor
canvas is a *pull renderer*: it asks the engine for exactly the rows in the
dirty rect rather than being pushed a layout. And the Metal design's central
claim (`SUISEI-GPU-ARCHITECTURE.md` §9.1) is that a scroll should be *a uniform,
not a rebuild* — compute the offset, do not re-derive positions.

The tab strip is the one surface that does the opposite.

---

## 4. Proposed architecture: compute, don't measure

A chip's width is a pure function. Nothing about it needs the view system:

```
width(chip) = leadingPad
            + iconW
            + gap
            + ceil(textWidth(title, font))      // CTLineGetTypographicBounds
            + (showsTrailing ? gap + trailingW : 0)
            + trailingPad
```

`textWidth` is a CoreText measurement of a string — deterministic, cacheable by
`(title, font)`, and already needed for drawing. From it, everything follows by
arithmetic:

```
xs[0]      = 0
xs[i+1]    = xs[i] + width(i) + interChipGap
contentW   = xs[n]
overflow   = contentW > viewportW
originX    = overflow ? -scrollOffset : (viewportW - contentW) / 2
hitTest(p) = binarySearch(xs, p - originX)
plusX      = originX + contentW + gap
```

One source of truth: `xs` and `originX`, both computed synchronously from
`(tabs, font, viewportW, scrollOffset)`. A hit test and a paint in the same
frame read the same numbers by construction. There is no second timestamp to
disagree with, so the entire class of bug is gone rather than narrowed.

### What this deletes

- All four `.onGeometryChange` measurements in the strip
- `tabFrames`, `tabHitFrames`, `chipRowOrigin`, `tabStripContentWidth`
- `pruneTabFrames` and the stale-frame problem it exists to manage
- The `@State`-write-per-chip-per-pass invalidation storm

### What it makes possible

- **Overflow and scroll become owned state**, not `ScrollView` emergent
  behaviour — so "obscured" tabs are a clamp on `scrollOffset`, not a race.
- **The "+" is arithmetic** (`plusX` above), ending the overlay/offset attempts
  that have variously drifted, stranded it, or broken hit-testing.
- **The grouped band is arithmetic** — `xs[first] … xs[last+1]` — instead of a
  reduction over separately-measured member rects.
- **Layout transitions animate a number**, not a re-measurement. Merge changes
  `contentW`; interpolating it is a normal animation with no re-entrancy.
- **Cost becomes O(tabs) once per tab-set change**, not O(tabs) callbacks per
  layout pass. The stutter's growth with tab count goes away.

### Where the computation belongs

Two options, and the choice matters less than picking one:

1. **Face-side layout model** — a `TabStripLayout` struct rebuilt when
   `(tabs, font, viewport)` changes; chips drawn at assigned offsets. Smallest
   change, keeps SwiftUI as the renderer.
2. **Engine-side** — the compositor already emits `TabScene`; it could emit
   widths too. Consistent with the pull-renderer model, but the engine would
   need font metrics, which it deliberately does not have today.

**(1) is the right first step.** The engine has no business knowing about
CoreText, and the face already owns `EditorMetrics` for exactly this kind of
measurement.

### Risk

The strip's animations are the most hand-tuned code in the face — the grouped ⇄
unified morph, the travelling pill, drag-reorder hysteresis. Computed layout
*helps* them (an interpolable number instead of a re-measurement), but they must
be re-verified transition by transition, with `SUISEI_ANIMATION_TRACE=1` and the
existing `trace_analyze` harness rather than by eye.

---

## 5. Recommendation

Stop patching the hit-test path. The remaining open items — the "+" placement,
the hover mis-report — are both *symptoms of the same race*, and both disappear
under computed layout rather than needing their own fixes.

Do it in this order:

1. `TabStripLayout`: pure struct, pure function, unit-tested headlessly against
   known chip sets (widths, overflow, hit-test, band extents). No UI.
2. Swap hit-testing onto it, leaving rendering measured. Verify the four states
   (general / grouped / merged / detached) with `SUISEI_TABLOG=1`.
3. Swap rendering onto it and delete the measurements.
4. Re-verify the animations with the trace harness.

Step 1 is worth doing on its own: it is testable without a 12-minute app build,
which is the constraint that made every previous attempt expensive to verify.
