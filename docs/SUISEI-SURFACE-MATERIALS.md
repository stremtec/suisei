# Surfaces and materials: why the Git workbench is the reference

Written 2026-08-10, after four wrong attempts at "make the editor's material
match the workbench's". Each of those failed the same way — a change made from
a guess about where a material comes from, applied without reading the other
implementation first. This is that reading, plus the rule each choice comes
from, so the next attempt starts from facts.

---

## 1. What each window actually draws

Both inventories are complete — every `.background`, `.glassEffect` and effect
view in the file, not a sample.

### Git workbench — 16 surfaces

| source | count | where |
|---|---|---|
| `.windowBackgroundColor` | 4 | root, identityAndCommitRow, changesFilterBar, historyMaster |
| `.textBackgroundColor` | 5 | workspace, changesWorkspace, commitMessageEditor, repositoriesWorkspace, diffCanvas |
| `.controlBackgroundColor` | 4 | worktreeDiffCard, commitHeader, commitDiffCard ×2 |
| `.bar` (system material) | 1 | historyHeader |
| `.glassEffect(.clear.interactive())` | 1 | liquidSelectionPill |

Window: `ThemedWindowChrome(background: .windowBackgroundColor, opaque: true)`.

Structure: `NavigationSplitView { sidebar } detail: { workspace }`. The sidebar
is `VStack { rail; Divider; List(selection:).listStyle(.sidebar)
.scrollContentBackground(.hidden) }` and carries **no background of its own**.

Row-level hierarchy inside the diff is alpha over one base:
`labelColor.withAlphaComponent(0.018–0.045)`, `accent(0.08–0.13)`,
`green/red(0.09–0.16)`.

**Three rules hold without exception:**

1. Every surface is a **system semantic colour** or a **system material**. No
   authored colour anywhere.
2. There is exactly **one** glass, and it is **`.clear`, untinted**, on a small
   interactive pill.
3. Depth comes from **alpha washes over one base**, never from a second base
   colour.

### Editor — 62 surfaces

The same audit finds, besides the semantic colours it does use:

| breaks | where |
|---|---|
| `Glass.regular.tint(**white** 0.14 light / 0.06 dark)` | `SuiseiGlass.panel` — terminal panel, find bar, resize HUD |
| `Color.black.opacity(0.10 / 0.36)` | `GlassScrim` behind the palette |
| `.windowBackgroundColor.opacity(0.42 / 0.72)` | ContentView:3222 — a semantic colour made translucent |
| `Color.white.opacity(0.08)` | ContentView:4367, 4402 — raw white, not `labelColor` |
| `terminalGridBg` | theme-authored |

Structure: **no `NavigationSplitView` anywhere** — it appears only in comments.
The navigator is a hand-drawn floating card: `.background(editorBg)` +
`clipShape` + 1pt `separatorColor` stroke + shadow + offset-driven show/hide +
a manual resize grip. Its list is a `ScrollView` + `VStack`, not a `List`.

---

## 2. Why the workbench's structure is the right one

Not "because it looks nicer" — each of its three rules is a platform rule.

**Source lists are translucent, by specification.** Apple's Materials guidance
says source lists "have a translucent background that shows the desktop or
window behind them with vibrancy effects", and that vibrancy works "by pulling
color forward from behind the material". That is exactly what is visible in a
screenshot of the workbench: its sidebar takes a cast from the wallpaper. A flat
colour cannot do that, and neither can a `.withinWindow` blur — it has only the
window's own content to pull from.

**Content surfaces are not glass.** Apple's Liquid Glass guidance is explicit
that using the material "within the content layer adds unnecessary complexity",
and warns against "layering Liquid Glass elements", which "can lead to confused
hierarchies". The workbench's diff, lists and message field are all opaque
semantic colours; the one glass it has is a control, not a surface.

**Tint carries meaning or nothing.** "Use tint only to convey semantic meaning
such as a primary action or alert state, not as pure decoration." Every
`SuiseiGlass` tint was white — decoration by definition, since white says
nothing. `GlassBackdrop.swift` had already written the consequence three
functions below the offence: *"a heavy white wash turns the surface into frosted
plastic with no visible warp."* The widest surfaces carried the heaviest wash
(`panel`, 0.14).

**Semantic colours are not a style preference.** They resolve per appearance,
and they move with Increase Contrast and Reduce Transparency — which the HIG
requires be respected. An authored `rgb(27, 26, 24)` cannot.

**Alpha over one base is how AppKit separates chrome from content in dark
mode.** Measured on this machine, macOS 26 dark resolves
`windowBackgroundColor`, `textBackgroundColor` and `controlBackgroundColor` to
the *same* `#1E1E1E` (light: all `#FFFFFF`). So a design that expects those
three to differ has no hierarchy at all. The workbench does not expect it: it
layers `labelColor` at 1.8–4.5% instead.

---

## 3. Where the sidebar material actually comes from

This is the fact four attempts got wrong, so it is stated plainly:

**`List(.listStyle(.sidebar))`.** Not `NavigationSplitView`, which only makes
`.sidebar` the *default* list style for its first column.

The evidence is an Apple developer-forum thread asking the opposite question —
how to *remove* the sidebar material. The answer is to change the list style;
and when the asker replies that their sidebar is "a `ScrollView` with a `VStack`"
rather than a List, the thread concludes there is no API for it, and
`.scrollContentBackground(.hidden)` does not apply.

`ProjectTreeView` is that `ScrollView` + `VStack`. That, and nothing else, is
why the editor's navigator has no material.

Two corollaries, both measured here:

* An `NSVisualEffectView` added *around* the navigator cannot substitute. It
  lands in front of the card's own opaque `.background(editorBg)` and inside its
  `clipShape`, so `.behindWindow` has nothing to sample and `.withinWindow` is a
  flat tint. Both were tried.
* Forcing the editor window opaque makes it worse, not better: `.behindWindow`
  needs a window that is not opaque. The workbench is opaque *and* translucent
  where it matters because the effect view punches through for its own region.

---

## 4. The premise that has to be retired

`SuiseiApp.swift` justifies `.windowStyle(.hiddenTitleBar)`, and therefore the
whole hand-drawn card, like this:

> with `.titleBar` there is always a system titlebar STRIP above the content —
> the navigator card can only start below it, so the traffic lights + toggle
> float in a bare band above the card

The workbench disproves it in the same app. `Window("Source Control")` declares
**no** `windowStyle`, so it gets the default `.titleBar`, and
`applyThemedTitlebar` sets `titlebarAppearsTransparent = true`. Its
`NavigationSplitView` sidebar rises through the titlebar area and the traffic
lights float over it — which is the Xcode anatomy the editor's comment says it
is chasing.

---

## 5. The gap, as work

Ordered so each stage is buildable on its own.

**Stage 1 — window and root.** Drop `.windowStyle(.hiddenTitleBar)`. Apply
`ThemedWindowChrome(background: .windowBackgroundColor, light:, identifier:
.editorIdentifier, opaque: true)`. Replace the `ZStack`/`HStack` root with
`NavigationSplitView { sidebarColumn } detail: { detailColumn }`. Remove the
card treatment — background, clip, stroke, shadow, offset show/hide, manual
resize grip — for `.navigationSplitViewColumnWidth` and a
`NavigationSplitViewVisibility` binding. Move `topBar` into the detail column.

This stage carries the risk. `ContentView.body` lines 342–580 also host the
palette's editor-centred offset (measured: navigator edge 318, editor edge 1696,
editor centre 1007 vs window centre 1023.5), the panel spring documented as five
failed alternatives, the live-resize HUD, and focus reclamation. All of it has
to move with the structure.

**Stage 2 — sidebar content.** `ProjectTreeView`'s `ScrollView` + `TreeRowStack`
becomes a `List` with `.listStyle(.sidebar)` and
`.scrollContentBackground(.hidden)`. This is the stage that delivers the
material. The custom row transitions do not survive a List unchanged.

**Stage 3 — the remaining rule breaks.** `Color.white.opacity` → `Color.primary`
(= `labelColor`); the translucent `.windowBackgroundColor` at 3222 → full
opacity; `terminalGridBg` → semantic, unless terminal palettes are deliberately
theme-owned (they are: leave it, and say so here).

Already done: the white glass tints are gone (`4373153`), and the effect view
that could not work has been removed.

**Restore point:** `08658cd`.

---

## Sources

* [Materials — Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/materials)
* [Meet Liquid Glass — WWDC25](https://developer.apple.com/videos/play/wwdc2025/219/)
* [On macOS, what is the appropriate way to disable the sidebar material in a NavigationSplitView? — Apple Developer Forums 798022](https://developer.apple.com/forums/thread/798022)
* [How to get translucent lists on macOS — Hacking with Swift](https://www.hackingwithswift.com/quick-start/swiftui/how-to-get-translucent-lists-on-macos)
