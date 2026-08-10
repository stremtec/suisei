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

Structure, *as audited*: **no `NavigationSplitView` anywhere** — it appeared
only in comments. The navigator was a hand-drawn floating card:
`.background(editorBg)` + `clipShape` + 1pt `separatorColor` stroke + shadow +
offset-driven show/hide + a manual resize grip.

That is what §5 Stage 1 replaced; the inventory above is the *before*.

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

**`NavigationSplitView` itself.** Measured, not inferred — an earlier revision
of this section said `List(.listStyle(.sidebar))`, reasoning from an Apple
forum thread rather than from the running app. That was wrong.

The probe: five sidebar shapes, one `NavigationSplitView` each, hosted in an
off-screen `NSWindow`, then the whole AppKit view tree dumped with class names,
frames, layer classes and every `NSVisualEffectView`'s material and blending
mode. What every case produces, identically:

```
_NSSplitViewItemViewWrapper (0,0 302x568)
    BackdropView (0,0 302x568)                 layer=CABackdropLayer filters=1
  NSContainerConcentricGlassEffectView (8,8 294x528)
        NSHostingView<ColumnView + NavigationPaneModifier<SidebarStyleContext>>
```

Three facts fall out of that:

1. The material is a **`CABackdropLayer` behind the whole column**, installed by
   the split view's first item. There is no `NSVisualEffectView` in the sidebar
   at all on macOS 26.
2. It appears with a plain `ScrollView` + `VStack` sidebar exactly as it does
   with `List(.listStyle(.sidebar))`, and exactly as it does with the
   workbench's `VStack { rail; Divider; List }`. **The list style governs row
   metrics, not the surface.** `ProjectTreeView` can keep its 800 lines of
   custom rows.
3. The system already insets the column's content by **8pt on every side** and
   rounds it concentrically — `NSContainerConcentricGlassEffectView`. The
   editor's hand-drawn floating card (6pt gap, derived corner radius, 1pt
   `separatorColor` stroke, shadow) was a reimplementation of something the OS
   now draws, and only the OS's copy sits on the backdrop.

So the card was never an alternative to the split view. It was an opaque
`.background(editorBg)` painted directly over the one surface the material lives
on — which is why all four attempts to add a material to it failed. Each added
a material *in front of* the thing that was covering it.

Corollary, also measured: an `NSVisualEffectView` added around the navigator
cannot substitute in either blending mode. It lands in front of the card's own
fill and inside its `clipShape`, so `.behindWindow` has nothing to sample and
`.withinWindow` is a flat tint. And window opacity is a red herring: the
workbench is `opaque: true` and has the material, because a `CABackdropLayer`
samples the layer tree, not the desktop.

Probe source: `sidebar_probe.swift` / `sidebar_probe2.swift`, run 2026-08-10.

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

## 5. The work

**Stage 1 — window and root. Done.**

* `SuiseiApp`: `.windowStyle(.hiddenTitleBar)` dropped. The premise in §4 is
  retired in the code, with the reason written where the old comment was.
* `ContentView.body`: `NavigationSplitView { sidebarColumn } detail:
  { detailStack }`, `.balanced`, chrome via `ThemedWindowChrome(background:
  .windowBackgroundColor, opaque: true)` — Source Control's, verbatim.
* The card treatment is gone: background, `clipShape`, stroke, shadow, the
  offset show/hide and its spring, and the manual `PanelResizeGrip`. Width is
  `.navigationSplitViewColumnWidth(min: 240, ideal:, max: 460)`; visibility is a
  `NavigationSplitViewVisibility` binding that reads and writes Core's
  `uiNavVisible`, so there is still exactly one authority for it.
* The traffic lights are AppKit's own. `StableTrafficLightOverlay` (cloned
  button cells, an Auto-Layout host on the frame view, a `drop` that subtracted
  where SwiftUI added, a re-install on every activation) is deleted along with
  `styleTrafficLights` and `applyTrafficLightInset`. So is
  `WindowIdentityProbe` — `ThemedWindowChrome` already tags the window.
* Three insets that existed only to route around a floating panel are gone:
  `editorCard`'s leading `contentInset`, `topBar`'s `navW + 16` reserve and its
  86pt lights zone, and `statusLine`'s `navW + panelGap + 7 + 12` padding. The
  palette's hand-measured `(navReserved - inspectorReserved) / 2` becomes
  `-inspectorReserved / 2`, because it now hangs off the detail column.
* New: `SplitColumnWidthReporter`, so a width dragged on the system splitter
  still survives relaunch. It only reads — it observes
  `NSSplitView.didResizeSubviewsNotification` and writes `navW`; the column's
  `ideal:` is frozen at launch (`navIdealWidth`) so the two cannot fight.

Net −231 lines.

**Stage 2 — sidebar content.** Now optional for the material (see §3), so it is
a row-conventions change, not a surface one: `ProjectTreeView`'s `ScrollView` +
`TreeRowStack` → `List` + `.listStyle(.sidebar)` +
`.scrollContentBackground(.hidden)` would buy standard sidebar row metrics,
inset selection capsules and system disclosure behaviour. The custom row
transitions, rename field and drag-and-drop do not survive a `List` unchanged,
which is the whole cost. Not started.

**Stage 3 — the remaining rule breaks. Done.** `Color.white.opacity` →
`Color.primary` / `separatorColor` in the palette; the translucent
`.windowBackgroundColor` → full opacity. `terminalGridBg` **stays**: terminal
palettes are deliberately theme-owned, and the grid is dark in both themes
because Core paints its default foreground as `rgb(200,200,200)` — a semantic
background would make a light theme's terminal a 1.35:1 contrast ratio.

**Also fixed, found while doing the above.** `resolve(_:light:)` and
`applyThemedTitlebar` both named `.aqua`/`.darkAqua` directly. Those are two of
four appearance names: naming the plain one pins every semantic colour inside
that window to its normal-contrast value, which silently opts the whole app out
of Increase Contrast — and "semantic colours move with the accessibility
settings" is §2's own argument for using them. Both now go through
`WindowChrome.themedAppearanceName(light:)`, and a
`accessibilityDisplayOptionsDidChangeNotification` observer re-applies.

**Restore point:** `5d5ff51`.

---

## Sources

* [Materials — Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/materials)
* [Meet Liquid Glass — WWDC25](https://developer.apple.com/videos/play/wwdc2025/219/)
* [On macOS, what is the appropriate way to disable the sidebar material in a NavigationSplitView? — Apple Developer Forums 798022](https://developer.apple.com/forums/thread/798022)
* [How to get translucent lists on macOS — Hacking with Swift](https://www.hackingwithswift.com/quick-start/swiftui/how-to-get-translucent-lists-on-macos)
