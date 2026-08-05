# Suisei — GPU render architecture (Metal)

Code-verified 2026-08-03 on branch `devim-and-theme`. Host: Apple M4, 10 GPU
cores, Metal 4, macOS 26.4; deployment target `arm64-apple-macos26.0`, so every
device that can run Suisei has unified memory and Apple GPU family 9.

Two halves. **§1–§4 are the diagnosis** — why the app misses frames while the
machine sits idle, and why it gets worse with every tab and every split.
**§5–§13 are the design** — the render layer that replaces the CPU text path,
and the order to build it in.

> The premise of this document is that *underutilization is the defect*. The
> Rust core costs 0.011 ms per keystroke on a 6,000-line file and 0.001 ms per
> idle tick (`tests/keystroke_latency.rs`, `tests/tick_breakdown.rs`, release,
> measured today). The GPU draws nothing but a framebuffer blit. And the app
> still drops frames. Nothing here is a capacity problem; every number below is
> a **serialization** problem — one thread, one wall clock, one view tree.

---

## 0. Principle

> **A frame is a deadline, not a poll. Work that misses the deadline must move
> off the thread that owns it — onto the cores that are already idle.**

This is `SUISEI-EDIT-ARCHITECTURE.md`'s "로딩은 길어도, 편집은 최상" applied one
layer down. There it bought a thin keystroke hot path with an expensive cold
parse. Here it buys a thin *paint* hot path with an expensive cold **glyph
atlas prewarm**: pay once at file open, then every subsequent frame is a buffer
write and one draw call.

---

## 1. The frame clock is a wall clock

`EngineBridge.startTick()` (`EngineBridge.swift:938`):

```swift
tickTimer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { ... }
```

Every engine-driven visual change in Suisei is quantized to **50 ms — 20 fps on
a 120 Hz display**. Three separate defects follow:

1. **Rate.** A PTY write, an LSP diagnostic, a completion popup, a git refresh:
   all of them reach the screen at 20 fps regardless of what the display can do.
2. **Phase.** A `Timer` is not vsync-locked. It drifts against the refresh
   clock, so identical work lands at a different point in each frame and
   produces beat-frequency judder rather than a steady lag.
3. **Re-entry.** The tick runs *during* animations. See §3.

The only `CADisplayLink` in the tree is `AnimationTrace.swift:155`, which is
diagnostics and is off unless `SUISEI_ANIMATION_TRACE=1`. Nothing in the
shipping paint path is vsync-driven. `EditorCanvasView.startBracketFade`
(`EditorHost.swift:1507`) even builds a `Timer(timeInterval: 1.0/60.0)` — a
60 Hz wall clock on a 120 Hz panel, which is a guaranteed 2:1 beat.

---

## 2. The FFI moves ~730 KiB per refresh to deliver a few KiB of data

Struct sizes, measured by compiling `suisei_engine.h`:

| Snapshot | Size | Pulled by `refreshChrome()` |
|---|---:|---|
| `SuiseiTerminalSnapshot` | **300.0 KiB** | unconditionally |
| `SuiseiChromeSnapshot` | **181.1 KiB** | unconditionally |
| `SuiseiPreviewSnapshot` | 64.1 KiB | version-gated |
| `SuiseiGitWbSnapshot` | 55.5 KiB | unconditionally |
| `SuiseiDiagnosticsSnapshot` | 48.6 KiB | unconditionally |
| `SuiseiExplorerSnapshot` | 20.6 KiB | unconditionally |
| `SuiseiPaletteSnapshot` | 17.0 KiB | unconditionally |
| `SuiseiOutlineSnapshot` | 15.8 KiB | unconditionally |
| `SuiseiScmSnapshot` | 15.7 KiB | unconditionally |
| `SuiseiSettingsSnapshot` | 8.0 KiB | unconditionally |
| `SuiseiBandC` (per `pullBand`) | 107.5 KiB | per draw, ×N |

`refreshChrome()` (`EngineBridge.swift:4017`) calls `loadExplorer`,
`loadPalette`, `loadSearch`, `loadCompletions`, `loadTerminal`, `loadSettings`,
`loadScm`, `loadGitWb`, `loadTheme`, `loadOutline` and `refreshDiagnostics` on
**every** invocation and discards the result afterwards if the panel is closed
(`let paletteOut = palette.open ? palette : PaletteSnap.empty`). The gate is
applied after the copy, not before it.

Three findings that matter more than the totals:

**2.1 — 96.9% of the chrome snapshot is provably dead.**
`SuiseiChromeSnapshot.lines[SUISEI_MAX_LINES]` is `256 × 688 = 176,128` bytes of
the 185,440-byte struct. The face never decodes it. `EngineBridge.swift:3916`
says so in its own comment, and `decodeEditorLinesAndSplit` hard-codes
`let allLines: [EditorLine] = []`. The editor is a pull renderer; rows arrive
via `pullBand`. Meanwhile `ffi.rs:423` still does

```rust
std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiChromeSnapshot>());
```

— a 181 KiB memset — and Swift's `SuiseiChromeSnapshot()` zero-initializes
another 181 KiB before the call. **~362 KiB of memset per chrome pull, for the
~9 KiB that is actually read.**

**2.2 — the string decoder is quadratic-ish by construction.**
`readCString` (`EngineBridge.swift:4612`) appends one `CChar` at a time into a
growing Swift `Array`, then builds a `String` from it. It runs once per line in
every `pullBand`, and `pullBand` copies the 107.5 KiB `SuiseiBandC` **by value**
each call — `rows()` (`EditorHost.swift:787`) can call it several times to fill
one band. `String(cString:)` on the raw pointer is one call and does the same
job.

**2.3 — throughput.** With a terminal open the tick takes the `refreshChrome()`
branch (`EngineBridge.swift:952`). At 20 Hz that is roughly **14 MB/s of memset
and copy plus a few thousand `String` allocations per second**, sustained, to
paint a screen that mostly did not change.

**2.4 — but this is microseconds, not milliseconds.** Measured after the fact,
and worth stating plainly because it reorders the whole plan. On an M4:

| | |
|---|---:|
| zero-init the old 185,440-byte snapshot | 0.778 µs |
| full chrome pull + 16-tab decode, today | 0.541 µs |
| terminal snapshot pull (307,220 B) | 2.601 µs |
| terminal 48-row `String` decode | 1.976 µs |
| `open_panels` probe (4 B) | 0.001 µs |

Removing all of §2's waste saves on the order of **10 µs per refresh — 200 µs
per second at 20 Hz, about 0.02% of one core.** It is real waste and worth
deleting (memory bandwidth, allocator pressure, power), but it is **third-order
for latency**. The millisecond costs are §3 and §4, and they are three orders of
magnitude larger. Do not expect §2 to move a dropped frame.

---

## 3. Why animations stutter — the exact mechanism

Suisei's animations are SwiftUI curves between 0.07 s and 0.4 s
(`EngineBridge.swift:2569-2581`, plus 32 `.animation(...)` sites in
`ContentView.swift`). At 120 Hz that is **8 to 48 frames**. During each one:

1. The 50 ms tick fires **1 to 8 times**.
2. Each firing calls `refreshEditorPaintOnly()` or `refreshChrome()`.
3. Both write `@Published` properties on `EngineBridge`.
4. `ContentView` holds `@ObservedObject var engine: EngineBridge`
   (`ContentView.swift:40`). It is a **5,616-line struct with 35 computed
   `some View` properties and 137 members**, all reachable from one `body`.
   Any publish re-evaluates all of it.
5. That happens on the main thread, inside the animation window.

The cost is already measured in the codebase. `EditorTickStore.swift:8-11`:

> "cheap unsplit (~0.04 ms) but **~20 ms once split**, because SwiftUI re-ran
> the split container's body for every pane on every key"

and `EngineBridge.swift:3929`:

> "`chrome publish` measured 0.04 ms unsplit → **~8 ms** the instant you split"

**8 ms is one entire 120 Hz frame.** A 0.3 s layout animation with split panes
and a visible terminal takes ~6 tick firings × ~8 ms — six dropped frames out
of thirty-six. That is precisely what stutter looks like.

The build system already reports the size of this tree from the other side:
`scripts/package-suisei-app.sh:12` records `swiftc -O` at **3:59** against
`-Onone` at **0:27**. Four minutes of optimizer work on one view struct is the
same fact as 8 ms of runtime diffing on it.

`EditorTickStore` was built to fix this *for typing* by moving per-keystroke
state onto a separate `ObservableObject`. It works, and it is the right idea.
But it is a point fix: every other publish still goes through the monolith.

And the worst case is structural. `refreshChromeWithTabMotion`
(`EngineBridge.swift:2610`) does:

```swift
withAnimation(animation) {
    refreshChrome()          // the ~730 KiB marshal, from §2
}
```

The animation's first frame carries the entire marshal, and the tick re-enters
the tree for every frame after it.

---

## 4. The face: CPU rasterization, and how it multiplies

### 4.1 Text is rasterized on the CPU, per line, per frame

`EditorCanvasView.draw` (`EditorHost.swift:891`) is CoreText into a
`CGContext`. Per visible line, per repaint:

- `visualToUTF16Map` (`EditorHost.swift:1270`) allocates a Swift `String` per
  *character* (`ns.substring(with: r)`) just to measure its width. It is called
  **twice per line** — once directly in `draw` for the find-highlight spans, and
  again inside `attributedLine`.
- `cacheKey(for:)` (`EditorHost.swift:881`) builds the CTLine cache key by
  `+=` string concatenation over every span, then hashes it. The key costs more
  than the lookup it guards.
- `CTLineGetOffsetForStringIndex` is called up to six times per line (find
  spans, selection, caret, bracket hints).

The codebase's own number, at `EditorHost.swift:1500`:

> "A full repaint measured **1.6 ms** in a release build"

1.6 ms is 19% of a 120 Hz frame for editor text alone, before SwiftUI,
before the marshal. `TermCanvas.draw` (`ContentView.swift:6977`) is the same
shape with an ANSI parse layered on top.

**And `visualToUTF16Map` was most of it.** Measured on a 60-row viewport of
mixed Latin/Korean source, per full repaint:

| `visualToUTF16Map`, 60 rows × 2 calls | |
|---|---:|
| original (`ns.substring` per character) | **1794 µs** |
| direct UTF-16 read, no memo | 999 µs |
| direct UTF-16 read + memo on line text | **11 µs** |

A 156× reduction, and on this content the original alone exceeded the whole
1.6 ms repaint figure — because that figure was measured on lighter text. Which
is the second finding here: **the cost scales with script.**
`rangeOfComposedCharacterSequence` plus a per-character `String` is far more
expensive for composed and multi-byte scripts than for ASCII, so the editor was
markedly slower on Korean than on English. For this project that is not an edge
case.

The CTLine cache key is the same defect, smaller: 53 µs → 8.8 µs per viewport.

And `grep -rn Metal suisei-app/` returns exactly one hit:

```
suisei-app/README.md:43: - Editor glyph blit (Metal) lands in S2; S1 is chrome snapshot only.
```

It never landed.

### 4.2 The scaling law — tabs × panes

The reported symptom is that lag grows with **open tab count** and with **split
depth**. `tests/scale_by_tabs_and_panes.rs` (added with this document) settles
where it comes from. Release build, M4:

```
=== suisei_engine_chrome, by open tab count ===
  tabs    chrome pull              per added tab
     1      0.0007ms                 0.00000ms
    64      0.0010ms                 0.00000ms

=== one repaint's band pulls, by pane count ===
 panes      all bands       per pane      chrome pull
     1      0.0006ms      0.0006ms        0.0009ms
     4      0.0021ms      0.0005ms        0.0008ms

=== idle tick, tabs × panes ===          (1/2/3/4 panes × 1/8/32/64 tabs)
            all sixteen cells: 0.0000ms
```

**The engine is flat.** 64 tabs cost 0.3 µs more than one. Four panes cost 2 µs
per frame at the FFI boundary. The idle tick is below measurement resolution
across the entire grid. Every millisecond the user feels is on the Swift side.

There, both axes are superlinear, for different reasons.

**Tabs — a genuine O(N²) per body evaluation.** The strip is a
`ScrollView(.horizontal) { HStack { ForEach(engine.chrome.tabs) } }`
(`ContentView.swift:1458-1464`). Four compounding facts:

1. It is a plain `HStack`, **not** `LazyHStack`. Every chip subtree is built and
   measured on every body evaluation, including the ones scrolled out of view.
2. Each chip carries `.onGeometryChange { … } action: { rememberTabFrame(tab, rect:) }`
   (`ContentView.swift:1504`). `rememberTabFrame` writes up to **four `@State`
   dictionaries** — `tabFrames`, `rememberedTabFrames`, and for grouped chips
   `layoutTransitionCenters` and `layoutTransitionWidths`. A `@State` write on
   `ContentView` invalidates *the whole 5,616-line body*, which rebuilds the
   strip, which re-measures every chip.
3. `rememberTabFrame` itself runs `engine.chrome.tabs.filter { … }` and
   `members.compactMap { tabFrames[$0.stableId] }` — **O(N) work inside a
   per-chip callback**, so one clean measurement pass is already O(N²) before
   any re-entry.
4. `tabStripPresentationKey` (`ContentView.swift:1335`) interpolates and joins a
   String over *every* tab. It is a computed property consumed by both
   `.animation(_:value:)` and `.onChange(of:)` (`ContentView.swift:1628-1630`),
   so it is built **twice per body evaluation** and compared O(N) each time. Its
   `onChange` then calls `pruneTabFrames`, which rewrites `tabFrames` wholesale
   — one more `@State` write, one more invalidation.

Each chip also mounts an `AnimationTraceProbe` (`ContentView.swift:1496`) — a
real `NSViewRepresentable` with a per-chip interpolated key string. The
recorder early-returns when `SUISEI_ANIMATION_TRACE` is unset, but
`makeNSView` still creates a layer-backed `NSView` per chip and `setKey` still
runs per update. The diagnostic is not free in the shipping build.

And all of it runs **20 times a second**, because any `@Published` write from
the tick invalidates the same body (§3).

**Panes — a large constant, multiplied.** Each pane is one `EditorHost` →
`NSScrollView` + `EditorCanvasView`. Per pane, per update:

- `updateNSView` (`EditorHost.swift:44-81`) builds **21 `NSColor(Color)`
  bridges** — one per theme token — on every call. The theme almost never
  changes; the conversion happens anyway.
- Every pane's `EditorHost` holds `@ObservedObject var editorTick`, the
  *shared* `EditorTickStore`. One `gen` bump updates **all P representables**,
  so a single keystroke is P × 21 colour bridges + P × `apply()` + P ×
  `needsDisplay`.
- P × a full CoreText repaint at the measured 1.6 ms each.
- P × `pullBand`, each zero-initializing a 107.5 KiB `SuiseiBandC` in Swift
  before Rust memsets the same 107.5 KiB again — 220 KiB of memset per pane per
  band fill, for rows the engine hands over in half a microsecond.
- P separate `ctCache` LRUs of 800 entries each, with **no sharing between panes
  showing the same file** — so a theme change costs P full cache rebuilds.

**Why they multiply rather than add.** Both live inside the same `body`. A
geometry callback from *one tab chip* invalidates `ContentView`, which
re-evaluates `splitEditorLayout` and reconstructs all P `EditorHost` values.
Conversely a tick that bumps `editorTick` for the panes also republishes
`chrome`, whose `tabs` array drives the O(N²) strip. N tabs and P panes are not
two independent costs; they are **one 5,616-line body that either can invalidate,
20 times a second**.

That is the whole scaling law: *the engine is O(1); the face is O(N² + P·k)
behind a single invalidation domain.*

### 4.3 What the running app actually reports

Everything above §4.2 was static reading plus headless benchmarks. This is the
packaged `-O` build under `SUISEI_PERF=1`, driven for ~90 s with **17 tabs open
and the editor split into 3 panes** — the case the user reports as worst. Ranked
by total time, which is the ranking that matters:

| label | total | mean | worst |
|---|---:|---:|---:|
| `EditorCanvasView.draw` | **192.2 ms** | 1.434 ms | **21.963 ms** |
| ↳ `ctLine total` | 84.7 ms | 0.037 ms | 14.687 ms |
| ↳ `CTLineDraw` | 57.0 ms | 0.025 ms | 2.918 ms |
| ↳ `gutter number` | 10.4 ms | 0.005 ms | 1.413 ms |
| `refreshEditorPaintOnly` | 49.5 ms | 0.869 ms | 15.973 ms |
| `refreshChrome` (full shell) | 20.4 ms | 0.786 ms | 2.604 ms |
| `chrome publish` | 19.9 ms | 0.350 ms | **15.533 ms** |
| `loadOutline` | 15.3 ms | 0.196 ms | 2.143 ms |

Three things fall out of this, and two of them revise what came before.

**1. The draw is 10× everything else combined**, and CoreText is 74% of the draw
(`ctLine` + `CTLineDraw` = 142 of 192 ms). That is the sustained cost, and it is
exactly what §9 exists to move onto the GPU. The 1.6 ms figure in
`EditorHost.swift:1500` was optimistic: mean is 1.434 ms but **worst is
21.963 ms — two and a half 120 Hz frames in a single draw.**

**2. `chrome publish` spikes to 15.533 ms.** That is one `chrome = next`
assignment (`EngineBridge.swift:3868`), and it confirms §3's mechanism with a
real number rather than a code comment. Note the shape: mean 0.350 ms, worst
15.533 ms. It is not a steady tax, it is an occasional stall — which is what
stutter *is*.

**3. The `ctLine` cache missed 26% of the time** (1,674 hits / 587 misses).
Cause: the cache was per-`EditorCanvasView`, and three panes were showing the
*same file*. A quarter of all CoreText shaping was re-deriving what the pane
next door had already computed. Fixed by making the shaped-line and
visual-map caches process-wide, keyed by a palette generation that is also
shared — a per-instance colour counter would have given identical colours
different keys and defeated it.

### 4.4 Scoreboard

| Resource | State | Should be |
|---|---|---|
| CPU | 1 thread doing everything visual; core is 0.011 ms/keystroke | 3–4 threads, engine off main |
| GPU (10 cores) | framebuffer blit of CPU-rasterized bitmaps | glyph atlas + instanced quads |
| Memory | ~1 MB of fixed structs that are mostly zeros | ~8 MB of warm atlas and instance buffers |
| Frame rate | 20 Hz poll, unlocked from vsync | on-damage present, vsync-locked, up to 120 |

---

# The design

## 5. Layering

The existing four layers stay. One is added, and the bridge between Compositor
and Renderer changes shape.

```
  Core (Rust)          suisei-core::App, dispatch          — unchanged
  Compositor (Rust)    scene.rs → FrameDiff                — emits packets, not Strings
  Bridge               shared arena + triple buffer        — replaces fixed C structs
  Shaper (Swift)       CoreText → glyph ids, cached        — NEW
  Renderer (Swift)     CAMetalLayer, atlas, instancing     — NEW (replaces CoreText draw)
```

Three rules govern the split:

1. **Rust never shapes text.** `suisei-engine`'s dependencies today are
   `suisei-core`, `suisei-daemon`, `unicode-width` — nothing else. That
   discipline is worth more than owning the shaper. Rust emits UTF-8 runs plus
   style spans; Swift shapes them with CoreText, which is also the only way to
   get the system's font cascade, IME behaviour and Korean/CJK fallback right.
2. **Swift never allocates per glyph per frame.** Shaping results are cached by
   `(bytes, style)`; the per-frame product is a flat instance buffer.
3. **SwiftUI never sits on the paint path.** It owns structural chrome only.

## 6. Frame clock — two clocks, not one

Replace the single 50 ms `Timer` with a producer and a presenter.

**Engine clock (producer).** The existing tick, moved off the main thread onto a
dedicated thread. It owns `App`. Its cadence stays whatever PTY/LSP need — 50 ms
is fine, and it can go event-driven later. It **never touches SwiftUI**. It
publishes a completed frame packet by swapping an index in a triple buffer.

**Present clock (consumer).** `CAMetalDisplayLink` bound to the window's screen,
vsync-locked. On each fire it reads the newest *complete* packet, and:

```swift
guard damage.isEmpty == false else { return }   // do not present an identical frame
```

Skip `present()` when nothing is dirty. That is how you get ~0% GPU at idle and
120 Hz during interaction from the same code — the alternative, a fixed-rate
render loop, would be the same mistake as the 50 ms timer with a nicer name.

```swift
let link = CAMetalDisplayLink(metalLayer: layer)
link.preferredFrameRateRange = CAFrameRateRange(minimum: 30, maximum: 120, preferred: 120)
link.delegate = self
link.add(to: .main, forMode: .common)
```

Triple buffering, no lock on the read side:

```rust
// engine thread
let slot = (published.load(Acquire) + 1) % 3;
write_packet(&mut arena[slot]);
published.store(slot, Release);   // publish is one atomic store
```

`CAMetalDisplayLink` fires with `targetTimestamp`, so the engine can also be
told how much time it has before the next deadline — useful later for
work-stealing an incremental syntax parse into the slack.

**This alone removes the §3 stutter**, because SwiftUI stops being re-entered by
the tick. It is worth doing before any shader is written.

## 7. The frame packet — what replaces the fixed structs

One shared arena per window, `MTLStorageModeShared`, so Rust writes into the
same physical pages the GPU reads. On Apple Silicon there is no staging copy and
no upload.

```
FrameHeader {
    u64 frame_id;
    u32 arena_len;
    u16 layer_count;
    u16 flags;
    DamageRect damage[8];      // 0 = full
}
Layer { u32 kind; u32 offset; u32 count; }   // kind: Rect | Run | Decor
```

Text is emitted as **runs**, not as 688-byte fixed line records:

```
RunSpan {          // 12 bytes
    u32 byte_off;  // into the packet's UTF-8 blob
    u16 byte_len;
    u16 style_id;  // index into the theme's style table
    u16 row;
    u16 col;
}
```

A 200×80 screen of real source is a few hundred spans — **~5 KiB** against
today's 181 KiB — and even the pathological one-span-per-cell case is 192 KiB
for content that today costs 181 KiB regardless of how little changed. The
UTF-8 blob is the visible text once, uncopied.

Backgrounds and decorations are the same idea:

```
RectInstance {     // 20 bytes
    f32 x, y, w, h;
    u16 style_id;
    u8  corner;    // SDF radius, 0 = square
    u8  kind;      // fill | stroke | selection | cursor-line | git-stripe
}
```

`RoundedRectangle(` appears 46 times in `ContentView.swift`. Every one of them
is one `RectInstance` with a nonzero `corner`, resolved by a signed-distance
fragment shader, in the same draw call as all the others.

### 7.1 Delete `lines[256]` first

Independent of everything else in this document: removing
`SuiseiChromeSnapshot.lines` takes the struct from **185,440 bytes to 9,312** —
181.1 KiB to 9.1 KiB, a 95% cut. It is unread (§2.1). This is one ABI field plus
the `ffi.rs` fill loop, and it is the single highest-yield edit in the repo.

## 8. Shaping and the glyph atlas

**Shaper.** A Swift actor, off the main thread:

```swift
struct ShapeKey: Hashable { let bytes: UInt64; let styleID: UInt16; let size: Float }
struct Shaped { let glyphs: [CGGlyph]; let advances: [Float]; let atlasSlots: [UInt16] }
```

`bytes` is a 64-bit hash of the run's UTF-8, accumulated without allocating —
this is what replaces `cacheKey`'s string concatenation. On a cache hit the
per-frame cost of a line is *zero shaping work*. During scrolling the hit rate
is ~100%; while typing, exactly one line misses.

Shaping itself stays CoreText (`CTLineCreateWithAttributedString` →
`CTRunGetGlyphs`) precisely because it is the only thing that gets the system
cascade list, Hangul jamo composition, and CJK fallback right. We keep the
correct typography and drop only the per-frame rasterization.

**Atlas.** One `MTLTexture`, `.r8Unorm`, 2048×2048, shelf-packed, with an LRU
for eviction. macOS has used grayscale antialiasing since Mojave, so a
single-channel alpha atlas is exactly what the platform renders — no subpixel
RGB triplets needed, one third the memory, and a trivial blend.

Capacity at 14 pt @2x, shelf-packed with a 1 px gutter: ~7,000 single-width
cells (≈17×34 px) or ~3,600 double-width ones (≈34×34). All 2,350 KS X 1001
Hangul syllables plus the full Latin set, punctuation and box drawing fit
simultaneously with room to spare. Rasterization is
`CTFontDrawGlyphs` into a `CGBitmapContext`, uploaded with
`replaceRegion(_:mipmapLevel:withBytes:bytesPerRow:)`, on the shaper thread.

Because the grid is monospaced, glyph origins snap to integer pixels — no
subpixel-position variants, which is why terminal renderers look crisp and why
the atlas stays small.

**Color glyphs** (emoji, `sbix`/`COLR` fonts) go in a second `.bgra8Unorm`
atlas, drawn in a second pass with a pipeline that skips the color multiply.

**Prewarm.** On file open, walk the buffer's distinct scalars and rasterize
their glyphs before the first paint. This is §0's principle: the cold cost is
paid where the user already expects to wait, and no frame afterwards ever
stalls on a glyph miss.

## 9. Render pipeline

One `CAMetalLayer` per editor surface:

```swift
layer.device = device
layer.pixelFormat = .bgra8Unorm_srgb     // hardware does linear-space blending
layer.framebufferOnly = true
layer.displaySyncEnabled = true
layer.maximumDrawableCount = 3
layer.contentsScale = window.backingScaleFactor
```

`.bgra8Unorm_srgb` matters: text antialiasing blended in sRGB space is the
classic "thin and washed out" look. Letting the hardware convert gives gamma-
correct coverage blending for free.

One render pass, four ordered draws:

| # | Pass | Content | Draws |
|---|---|---|---|
| 1 | Rects | cursor line, selection, find wash, git stripes, panel fills | 1 (instanced) |
| 2 | Text | every glyph on screen | 1 (instanced) |
| 3 | Color text | emoji | 0–1 |
| 4 | Decor | carets, underlines, dividers, rounded borders (SDF) | 1 (instanced) |

**≤ 4 draw calls for the whole editor surface**, against thousands of CoreText
draws today.

No vertex buffer — quads come from `vertex_id`, instances from an
argument-buffer-addressed pointer. That is the Apple-GPU idiom and skips input
assembly entirely:

```metal
struct GlyphInstance {   // 12 bytes
    packed_half2 pos;
    ushort       slot;       // atlas cell
    uchar        fg, bg, flags, _pad;
};

vertex VOut glyph_vs(uint vid [[vertex_id]], uint iid [[instance_id]],
                     device const GlyphInstance* inst [[buffer(0)]],
                     constant Uniforms& u [[buffer(1)]]) {
    const float2 corner = float2((vid & 1), (vid >> 1));       // 0,0 1,0 0,1 1,1
    GlyphInstance g = inst[iid];
    float2 p = float2(g.pos) + corner * u.cell + u.scrollOffset;
    VOut o;
    o.position = float4(p * u.ndc + u.origin, 0, 1);
    o.uv       = atlasUV(g.slot, corner);
    o.fg       = u.palette[g.fg];
    return o;
}

fragment half4 glyph_fs(VOut in [[stage_in]],
                        texture2d<half> atlas [[texture(0)]]) {
    half a = atlas.sample(nearestSampler, in.uv).r;
    return half4(in.fg.rgb * in.fg.a * a, in.fg.a * a);        // premultiplied
}
```

Colors are palette indices, not per-instance RGBA: the theme has on the order of
20 token colors (`EditorCanvasView.Colors` has exactly 21 fields), so an 8-bit
index into a constant buffer is enough and keeps the instance at 12 bytes. A
theme change is one buffer write, not a cache invalidation — which incidentally
removes the whole reason `Colors` had to be `Equatable` to bust the CTLine cache
(`EditorHost.swift:516`).

### 9.1 Scrolling is a uniform, not a rebuild

`u.scrollOffset` is one `float2`. A scroll changes it and redraws; **nothing is
re-emitted, re-shaped or re-uploaded**. This is the single largest behavioural
difference from the current design, where a scroll invalidates a rect and
re-runs CoreText over every newly exposed line. It is also why the fat 3× row
overscan in `compose` (`scene.rs:573`) and the ±24-row band padding in `rows()`
stop being necessary.

### 9.2 Damage

Present only on damage. Sources: engine `frame_gen`, face-local scroll/caret/
IME state, atlas residency changes. Absent all three, the display link returns
without acquiring a drawable.

## 10. Threading

| Thread | Owns | Must not |
|---|---|---|
| Main | AppKit events, `NSTextInputClient`, SwiftUI structural chrome | block, marshal, rasterize |
| Engine | `suisei_core::App`, the tick, packet production | touch `@Published`, touch AppKit |
| Shaper | CoreText shaping, glyph rasterization, atlas upload | touch `App` |
| Present | display-link callback, command encoding | allocate |

Four threads doing real work on a 10-core machine, where there is one today.
IME stays on main — this is not negotiable, and §12 says why.

## 11. Migration — six stages, each shippable

The face is 350 KB of `ContentView.swift`. A rewrite will not happen. Every
stage below is independently revertible and independently measurable with the
existing `SUISEI_PERF=1` probe.

**G0 — cut the waste. No Metal.** Highest yield per line changed, and it makes
everything after it measurable. **Landed 2026-08-03**, except where noted:

- ✅ **Deleted `SuiseiChromeSnapshot.lines[256]`** — 185,440 → 9,312 bytes, a
  95% cut (§7.1). Rust memsets 9.1 KiB instead of 181 KiB, and Swift's
  `SuiseiChromeSnapshot()` zero-inits 9.1 KiB instead of 181 KiB, so the saving
  lands twice per pull. Guarded by two new tests in `tests/abi_layout.rs`
  (`chrome_snapshot_carries_no_line_payload`, plus a `pane_titles`-is-last
  assertion) so a bulk payload cannot creep back in.
  Measured on `scale_by_tabs_and_panes`: chrome pull **0.0007 ms → 0.0001 ms**
  at one tab, **0.0010 ms → 0.0004 ms** at 64.
- ✅ **Gated every panel pull behind one `u32`.** New
  `suisei_engine_open_panels()` returns a `SUISEI_PANEL_*` bitmask straight out
  of the last composed frame. `refreshChrome` asks it first and skips the copy
  for anything shut — terminal (300 KiB), git workbench (55 KiB), preview
  (64 KiB), explorer, palette, search, completions, settings, SCM, outline.
  The explorer bit deliberately means *"has entries"*, not *"owns the
  keyboard"*: the project navigator is docked and paints in Normal mode, so the
  focus flag would have blanked the tree.
  `refreshPreviewIfNeeded` gets the same gate — its "cheap single-chunk probe"
  was a 64 KiB struct pulled on the **light** path, every tick.
- ✅ **Diagnostics behind a fingerprint.** New
  `suisei_engine_diagnostics_fingerprint()` hashes rows, columns, severities
  *and* message text; `refreshDiagnostics` compares that `u64` before paying for
  a 48.6 KiB snapshot and a `String` per entry. Diagnostics move when a language
  server answers, not on the tick.
- ✅ **`readCString` → one allocation.** It appended a `CChar` at a time into a
  growing Swift `Array`, per row, for every band pull of every pane on every
  repaint. Now a bounded NUL scan plus `String(decoding:as:)`.
- ✅ **CTLine cache key → `UInt64`.** It built a `String` by interpolation and
  `+=` over every span, per visible line per repaint, to key a cache whose whole
  job is to avoid work. Now a `Hasher`. No weaker: the old key already carried
  `text.hashValue` rather than the text.
- ✅ **`visualToUTF16Map` no longer allocates per character.** It called
  `ns.substring(with:)` for every character just to read that character's first
  scalar — and ran twice per line per repaint (~12,000 string allocations for
  one frame of 60×100 code). Now it reads the UTF-16 unit directly (identical
  predicate: every high surrogate is above the 0x2E80 wide threshold, so astral
  characters still measure 2) and memoizes on a hash of the line text.
- ⏳ **Move the tick off the main thread** — deferred to G1 on purpose. Doing it
  here would mean two threads touching the engine pointer with no ownership
  story; it is correct only alongside the triple buffer in §6.

**G0b — break the invalidation domain (§4.2).** Same stage, same idea, aimed at
the tabs × panes scaling rather than at bytes. None of it needs Metal.

Landed 2026-08-03:
- ✅ **`rememberTabFrame`'s group math hoisted out of the per-chip callback.**
  It derived its group's members with `engine.chrome.tabs.filter` *inside* the
  geometry callback, so measuring N chips walked the tab list N times and one
  clean layout pass was **O(N²)** in tab count before any re-entry. The index is
  now built once per pass in `documentTabStrip` and captured by the callbacks.
- ✅ **`tabStripPresentationKey` → `UInt64`.** It joined an interpolated
  `String` over every tab, and *two* modifiers consume it
  (`.animation(_:value:)` and `.onChange(of:)`), so a computed property built it
  twice per body evaluation and compared it O(N) each time.
- ✅ **`AnimationTraceProbe` compiles to nothing when tracing is off.** It was
  the representable itself, so all five mount sites — one of them *per tab chip*
  — built a layer-backed `NSView` and re-ran `setKey` on every update even with
  `SUISEI_ANIMATION_TRACE` unset. The recorder's guards made it inert, not
  absent.
- ✅ **Editor palette resolved once, not per pane.** `updateNSView` built 21
  `NSColor(Color)` bridges per pane per call, and every pane's `EditorHost`
  observes the *shared* `EditorTickStore`, so one tick updated all of them.
  Measured: 0.37 µs per plain conversion, 0.65 µs semantic, **7.8 µs per pane,
  30 µs for four panes** — every tick. Now a one-slot cache keyed by the source
  colours; panes after the first are hits.

- ✅ **Shaped-line caches shared across panes** (§4.3, finding 3). Verified in
  the packaged `-O` build, same scenario re-run (17 tabs, 3 panes, one file):

  | | before | after |
  |---|---:|---:|
  | `ctLine` miss rate, whole session | 26.0% | **17.7%** |
  | `ctLine` mean, steady-state scroll | 0.037 ms | **0.015 ms** |
  | `CTLineDraw` worst | 2.918 ms | **0.204 ms** |
  | `EditorCanvasView.draw` mean, steady-state | 1.434 ms | **1.195 ms** |

  The residual ~18% miss rate is the honest floor: scrolling exposes lines
  nobody has shaped yet, and those are genuine cold misses. What went away was
  the *duplicate* work — three panes shaping the same file three times.

  Whole-session totals are **not** quoted as a comparison: the two runs had
  different activity mixes (the baseline included tab-opening churn), so only
  rates, means and worst-cases are like-for-like.

  Correctness checked where the change actually risked something: switching the
  editor theme (Light → Monokai → Light) recolours all three panes, and the same
  line renders pixel-identically between panes. That is the path the shared
  palette generation had to get right — a per-instance colour counter would have
  given identical colours different cache keys.

**G0b-2 — the tab bar specifically.** Reported by the user as "horizontal
scrolling and animations in the tab bar feel extremely unnatural — lagging and
stuttering", *after* the aggregate ranking in §4.3 had already pushed the strip
down the list. Both things are true, and the lesson is worth keeping: **total
time across a session is not the same as felt latency.** The strip's cost is
small in aggregate because it is only being scrolled or animated for a few
seconds at a time — but during those seconds it is a per-frame cost, and that is
precisely when a user is looking straight at it.

Four causes, all found by reading the scroll path rather than the profile:

- ✅ **`chipRowOrigin` was `@State`.** It measures the chip row's origin *in the
  scroll view's coordinate space*, so it changes on **every frame of a
  horizontal scroll** — and every one of those frames wrote into `ContentView`'s
  5,616-line body, rebuilding all 17 chip subtrees, the layout band, and every
  editor pane. Its only readers are the three mouse-handler closures, which run
  on events, not during body evaluation. Now a reference box: the value stays
  live, the write invalidates nothing.
- ✅ **`tabStripContentWidth` was write-only dead state.** Fed by another
  `.onGeometryChange`, invalidating the body on every width change, and **never
  read anywhere**. Deleted. (The `.fixedSize` above it is what actually does the
  job its comment describes.)
- ✅ **`rememberTabFrame` wrote unconditionally.** `onGeometryChange` fires on
  any layout pass, not only one that moved the chip, and a `@State` dictionary
  write invalidates whether or not the value differs. Compare first.
- ✅ **The tick published straight through tab animations.** A structural motion
  runs 0.20–0.30 s and the 50 ms timer fires 4–6 times inside it; each publish
  re-entered the body while the chips were interpolating. `isLiveScrolling`
  already guards exactly this for scroll gestures, and `tabStructuralMotionActive`
  was already tracked for the auto-scroll guard — it just was not wired to the
  tick. It is now, with a catch-up `refreshChrome()` when the motion ends so a
  suppressed PTY or diagnostic update is not lost.

**And those four fixes did NOT resolve the reported symptom.** Recorded because
a negative result is worth more than a plausible story. Driving tab switches in
the packaged `-O` build afterwards:

```
refreshEditorPaintOnly   n=5   mean 2.639 ms   max 12.814 ms
  chrome publish         n=5   mean 2.571 ms   max 12.749 ms
```

A single `chrome = next` costs **12.7–15.5 ms** on a tab switch — and it lands
inside the 0.2 s `proxy.scrollTo` animation the strip runs when the active tab
moves, which is precisely when the user is looking at it. Four of those five
publishes were cheap; the expensive one is the publish where `tabs` changed.

The four fixes above were real defects (dead state, per-frame `@State` writes,
unguarded dictionary writes, tick publishes inside fold animations) but none of
them is this one. Note also that the `chipRowOrigin` fix is verified by reading
only: synthetic scroll events are not consumed by SwiftUI's `ScrollView`, so a
drag-scroll could not be driven from the test harness.

**Correction to an earlier claim in this document.** It said extracting the tab
strip into its own `View` was the fix for the publish cost. Worked through
properly, that is wrong on its own:

- If the extracted strip observes `engine`, a `chrome` publish invalidates the
  strip *and* `ContentView`. No gain.
- If it takes `tabs` by value, the strip still rebuilds on a tab switch —
  `tabs` genuinely changed.

The saving can only come from the **other** subtrees being skipped, and SwiftUI
may only skip a child whose inputs it can compare. Today every one of them is an
inline computed property in a single body, so all of them re-evaluate on every
publish. Extracting the strip alone buys close to nothing.

So the fix is **subdivision of the body into `Equatable`-comparable child
views** — strip, navigator, inspector, status bar, editor area — not a single
extraction. And part of the 12.7 ms is legitimate: on a tab switch the file
really did change, so the outline and the panes genuinely must rebuild. How much
is legitimate is the open question, and it decides whether subdivision is worth
12 ms or 3 ms.

**The instrumented answer — and the tab strip is not the culprit.** Rather than
start a 650-line refactor on the reasoning above, one build split the publish
and timed every major subtree's construction. Packaged `-O`, 13 tabs, 2 panes,
driven by tab switches:

```
WHO OWNS THE PUBLISH COST
  chrome value copy (no publish)   0.000 ms     <- the struct is free
  chrome deep compare              0.001 ms
  chrome publish            mean 0.770, worst 4.408 ms

WHICH SUBTREE REBUILDS EXPENSIVELY
  body.detailColumn     15.1 ms total   worst 10.114 ms
    body.editorCard     13.7 ms total   worst  9.134 ms   <- nested in detailColumn
  body.statusLine        5.2 ms total   worst  0.488 ms
  body.sidebarColumn     3.8 ms total   worst  2.663 ms
  body.dockedNavigator   2.7 ms total   worst  1.816 ms
  body.inspectorColumn   2.7 ms total   worst  0.999 ms
  body.topBar            0.2 ms total   worst  0.105 ms   <- the TAB STRIP
```

Two conclusions, both of which redirect the work:

**1. The cost is invalidation, not the value.** Assigning the identical
`ChromeSnapshot` to a plain local — same retain/release traffic over every array
and `String` it owns, no publisher attached — measures **0.000 ms**. So
shrinking or boxing the payload would buy nothing; the whole cost is SwiftUI
reaching every observer. Subdivision is the right direction, confirmed.

**2. The tab strip is the CHEAPEST subtree in the window.** `topBar` — chips,
band, pill, the lot — costs **0.2 ms total and 0.105 ms worst**, two orders of
magnitude below `editorCard`'s 9.1 ms. The tab bar stutter is not produced by
the tab bar. It is produced by the **editor panes rebuilding**, which happens to
land inside the 0.2 s `scrollTo` animation the strip runs on a tab switch — so
the hitch is *visible* in the strip while being *caused* somewhere else.

This is why the strip extraction was not worth doing: it would have moved the
cheapest thing in the tree. Measuring first cost one build and saved a 650-line
refactor that could not have helped.

Note the shape of `editorCard`: mean 0.232 ms, worst 9.134 ms. It is not a
sustained tax, it is a rare spike — so the target is whatever makes that
occasional rebuild expensive (a new file entering a pane: `EditorHost`
recreation, `NSScrollView` setup, minimap rebuild), not the steady-state path.

Still open — and this is the one that matters:
- ⏳ **Extract the tab strip into its own `View` + store.** `tabFrames`,
  `rememberedTabFrames`, `layoutTransitionCenters` and `layoutTransitionWidths`
  are `@State` on `ContentView`, so a chip's geometry callback invalidates the
  whole 5,616-line body — including `splitEditorLayout`, which then reconstructs
  every pane. **The fix requires the strip to be a separate `View` that owns the
  state**: moving the state alone does not help, because anything `ContentView`
  observes invalidates `ContentView`.

  The measured `chrome publish` spike (15.5 ms, §4.3) sharpens *why*. That is a
  single `@Published` assignment, and the body it invalidates is one indivisible
  unit — 5,616 lines with **no comparison boundary anywhere inside it**, so
  SwiftUI has nothing it is allowed to skip. Every publish reconstructs all 17
  chip subtrees, re-runs `layoutGroupRuns`' grouping and sort, and rebuilds
  every pane, whether or not any of it changed.

  So the goal is not merely "isolate the geometry writes" — it is **subdivision
  into `Equatable`-comparable child views**, so a publish that did not touch
  `tabs` skips the strip entirely. The strip is the first and highest-value cut
  because its construction is O(tabs) and it rebuilds on every publish. The same
  treatment then applies to the navigator, inspector and status bar.

  It is ~500 lines (`documentTabStrip` plus `layoutGroupRuns`,
  `layoutShapeRuns`, `layoutGroupContainer`, `tabSlot`, `tabDragTarget`,
  `tabCloseSlot`, `rememberTabFrame`, `pruneTabFrames`,
  `tabPresentationTransition`) of the most heavily hand-tuned code in the face —
  the grouped ⇄ unified morph, the pill travel, the drag-reorder hysteresis.
  **`suisei-app` has no automated tests at all**, so the only verification
  available for it is launching the app and watching the transitions. Do not
  land this one blind.
- ⏳ **`HStack` → `LazyHStack` in the strip.** Deferred deliberately, not
  forgotten: with a lazy stack the off-screen chips never report geometry, so
  `rememberTabFrame`'s `frames.count == members.count` guard silently stops
  updating group centres and the grouped band loses its anchor. It needs the
  band to derive its extent from something other than per-chip measurement
  first.

**What G0 actually bought — measured, not projected.**

| | before | after |
|---|---:|---:|
| `visualToUTF16Map`, per 60-row repaint | 1794 µs | **11 µs** |
| CTLine cache key, per 60-row repaint | 53 µs | 8.8 µs |
| chrome snapshot | 185,440 B | **9,312 B** |
| chrome pull (Rust side), 64 tabs | 0.0010 ms | 0.0004 ms |
| whole marshal + panel gating, per refresh | — | ~10 µs saved |

Read that ordering carefully, because it is not what §2 implies. **The draw-path
allocation churn was the millisecond-scale item (~1.8 ms per repaint) and the
FFI marshal was not (~10 µs per refresh).** §2's waste was worth deleting on its
own terms — bandwidth, allocator pressure, power — but anyone expecting it to
recover a dropped frame will be disappointed.

The two costs still standing are the ones that dominate:

- **~8 ms** of SwiftUI body re-evaluation per publish when split (§3) — the
  single largest number anywhere in this document, and untouched by G0. **G0b**
  is what addresses it.
- **CoreText rasterization** itself (§4.1), now that the map is not paying for
  it. That is G2's job, and the reason this document exists.

So: *G0 is necessary groundwork and a real win on CJK-heavy files, but the
stutter the user reports is §3, and the fix for §3 is G0b.* Measure with many
tabs open **and** split.

**G1 — frame clock.** `CAMetalDisplayLink` presents; the engine tick becomes a
producer behind a triple buffer. Still CoreText. This decouples animation from
the poll and is the load-bearing change for §3.

**G2 — Metal editor canvas.** Replace **only** `EditorCanvasView` with
`MetalCanvasView: NSView` hosting a `CAMetalLayer`. It is already a
self-contained `NSView` inside an `NSScrollView`, so the swap is local. Keep the
CoreText path alive behind `SUISEI_RENDERER` for A/B and rollback.

**G2-1 — renderer, atlas and pipelines. Landed 2026-08-03.**
`suisei-app/Suisei/MetalTextRenderer.swift`: `MTLDevice`, a 2048² `.r8Unorm`
shelf-packed glyph atlas rasterized with `CTFontDrawGlyphs`, and two instanced
pipelines (rects, glyphs) whose shaders compile at runtime via
`makeLibrary(source:)` — so the flat `swiftc` packaging script needs no metal
build step. Neither pipeline binds a vertex buffer; quads come from `vertex_id`
and instances from a device pointer.

Verified offscreen against real pixels before touching the editor — Latin,
Korean, Japanese and CJK all render through the run's own fallback font, with
background rects compositing correctly beneath. Two bugs the picture caught that
review would not have:

- The clip transform was written as `(p·scale + origin)` with the result
  negated. That maps y=0 to −1 and puts the entire viewport outside the
  frustum. Rects and glyphs both vanished; the fix is that the y term
  *subtracts* from the origin.
- The atlas scratch context is reused and grows to the largest glyph, so it is
  usually bigger than the glyph being drawn. A `CGBitmapContext`'s memory row 0
  is the image's TOP while its y axis counts UP from the bottom, so drawing at
  y≈0 put the ink in the LAST rows of memory while the upload read the FIRST
  ones. Every glyph reported "resident" and the screen stayed empty. The V-flip
  in the UVs was compensating for a flip that does not exist once the glyph is
  placed against the top of the buffer.

Measured, 60 rows of mixed Latin/Korean source, 5,534 glyphs, 1200×1200 @2x:

| | ms/frame |
|---|---:|
| CoreText `CTLineDraw` ×60, lines already cached | **1.788** |
| Metal, build instances (main thread) | **0.459** |
| Metal, build + encode + wait for GPU | 0.954 |
| Metal, scroll only (one uniform changes) | 0.442 |

**3.9× less main-thread work per repaint.** Read the third row carefully: it
waits on a completion semaphore, so it includes a command-buffer round trip the
real app never pays on the main thread — the scroll-only row shows that round
trip is ~0.44 ms by itself, with no CPU build at all. The number that belongs in
a frame budget is **0.459 ms**.

The CoreText baseline here (1.788 ms) independently corroborates the 1.6 ms in
`EditorHost.swift:1500`, on heavier content.

**G2-2 — wire it into the canvas.** Not started. This is where
`NSTextInputClient` must survive intact (§12), and where selection, caret, find
spans, git stripes and the gutter become `RectInstance`s.

**G3 — Metal terminal canvas.** `TermCanvas` has the same shape and benefits
more, because PTY damage is continuous. Its ANSI runs map directly onto
`RunSpan` + `style_id`.

**G4 — shared-memory packets.** Replace the fixed-struct FFI with the arena
(§7). Only after G2/G3 have proven the renderer — the ABI change is the most
invasive step and the least urgent.

**G5 — chrome into the render layer.** Move dividers, panel borders, rounded
containers and the tab strip's geometry into the decor pass, shrinking the
SwiftUI tree that §3 blames. Do this **only if** G0–G3 leave measurable cost.
Do not touch `NSVisualEffectView` (10 sites) — Liquid Glass is system-optimized
and reimplementing it in a shader is a downgrade.

## 12. Risks

**IME is the one that can sink this.** `EditorCanvasView` implements
`NSTextInputClient`, and `terminal-must-be-nstextinputclient` is already a
recorded lesson in this project. `MetalCanvasView` **must** implement the full
protocol — `insertText`, `setMarkedText`, `firstRect(forCharacterRange:)` in
screen coordinates, `attributedSubstring`. The marked-text underline
(`EditorHost.swift:968-991`) becomes a `RunSpan` with an underline flag plus a
decor instance; the caret rect is already tracked as `lastCaretRect`. Budget
real time for Hangul mid-line composition and the candidate window's placement.

**Accessibility.** A Metal layer is opaque to VoiceOver — but so is the current
`CTLineDraw` canvas, so this is a pre-existing gap, not a regression. If it is
ever addressed it will be through an `NSAccessibilityElement` tree over the
engine's line model, which is renderer-independent.

**Display and scale changes.** Handle `viewDidChangeBackingProperties`
(`contentsScale`, `drawableSize`, atlas re-raster at the new scale) and
`NSWindow.didChangeScreenNotification` (rebind the display link, new refresh
rate). Dragging between a 60 Hz external panel and the built-in 120 Hz display
must not wedge the presenter.

**Font fallback.** The atlas is keyed by `(CTFont, CGGlyph, size)`, never by
character — the cascade is resolved during shaping, so a run that falls back to
a CJK face is simply a run with a different font key.

**Memory grows, deliberately.** ~4 MB alpha atlas + ~2 MB color atlas + ~1 MB of
triple-buffered instance data. That is a good trade against ~1 MB of fixed
structs that are mostly zeros, and it is the point: warm data in memory is what
buys a thin frame.

## 13. Definition of done

Measured with `SUISEI_PERF=1`, plus a Metal capture and an Instruments Display
trace, on the 60,000-line fixture already used by `tick_breakdown`:

| Metric | Today | Target |
|---|---:|---:|
| Editor canvas repaint (CPU) | 1.6 ms | < 0.3 ms |
| Editor canvas (GPU, 1600×1000) | n/a | < 0.5 ms |
| Draw calls, editor surface | thousands | ≤ 4 |
| Chrome marshal per refresh | ~730 KiB | < 16 KiB |
| Chrome publish, split panes | ~8 ms | < 0.5 ms |
| Body evaluations per tab-strip layout pass | O(N) | 1 |
| `NSColor` bridges per tick, 4 panes | 84 | 0 (cached) |
| Frame cost slope, 1 → 32 tabs | superlinear | flat, matching the engine |
| Frame cost slope, 1 → 4 panes | superlinear | linear, ≤ 4× the 1-pane cost |
| Dropped frames, 0.3 s layout animation (split + terminal) | ~6 | 0 |
| Sustained scroll, 60,000-line file | judder at 20 Hz republish | 120 fps |
| GPU utilization at idle | ~0% | ~0%, with **zero presents** |
| Threads doing frame work | 1 | 4 |

The idle row is not a typo. The goal is not to keep the GPU busy; it is to make
the GPU capable of absorbing a frame's entire work in under a millisecond, so
that the frame is never late — and then to not ask it for frames nobody needs.
