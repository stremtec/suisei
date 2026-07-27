import SwiftUI
import AppKit

/// Corner-radius scale. Radii were scattered across {0,4,6,8,10,12,18}, which
/// leaves no basis for the concentric rule — an inner element's radius has to
/// be derived from the surface it sits in, not picked.
enum Radius {
    /// List rows and chips sitting inside a panel.
    static let row: CGFloat = 6
    /// Buttons, fields, small controls.
    static let control: CGFloat = 8
    /// Cards, popovers, docked panels.
    static let panel: CGFloat = 12
    /// Floating glass surfaces (palette, HUD).
    static let floating: CGFloat = 18

    /// Concentric corners: an element inset by `gap` inside a surface of radius
    /// `outer`. Keeps the inner curve parallel to the outer one.
    static func inside(_ outer: CGFloat, gap: CGFloat) -> CGFloat {
        max(2, outer - gap)
    }
}




/// Production-oriented xei face: FrameDiff paint only; pointer lifecycle is editor-wide.
struct ContentView: View {
    @ObservedObject var engine: EngineBridge
    @FocusState private var focused: Bool
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow
    // Live panel sizes (@State). @AppStorage on every drag frame caused shake/ghosting.
    // Persist only when a resize gesture ends (see `persistPanelSizes`).
    @State private var navW: Double = 280
    @State private var termW: Double = 400
    @State private var debugAreaH: Double = 200
    @State private var inspectorW: Double = 240
    /// Xcode-like navigator mode (icon rail).
    @State private var navMode: NavMode = .project
    @State private var inspectorMode: InspectorMode = .outline
    @State private var inspectorFrom: Int = 0
    @State private var inspectorTo: Int = 0
    @State private var inspectorProgress: CGFloat = 1
    @State private var inspectorLiquid: Bool = false
    /// Find navigator query. Submitted on Return, never per keystroke — a
    /// project grep is far too expensive to run while someone is still typing.
    @State private var findQuery: String = ""
    /// Replacement text for Find navigator replace-one / replace-all.
    @State private var findReplace: String = ""
    /// Slot indices the selection indicator is travelling between, and how far
    /// along it is. Driven explicitly rather than by animating a position: the
    /// metaball needs to know where the pill came FROM, which a single
    /// interpolated coordinate has already forgotten.
    @State private var selectionFrom: Int = 0
    @State private var selectionTo: Int = 0
    @State private var selectionProgress: CGFloat = 1
    /// Shared geometry for the tab bar's active capsule.
    @Namespace private var tabPillSpace
    /// Tab currently held by a drag, nil otherwise.
    @State private var draggingTab: Int? = nil
    /// Chip rects in the strip's space — chips are as wide as their titles, so
    /// slots cannot be computed the way the rail's equal-width modes can.
    @State private var tabFrames: [Int: CGRect] = [:]
    static let tabStripSpace = "suisei.tabstrip"

    /// Which chip sits under `x`. Used for CLICKS, where "contains" is right.
    private func tabSlot(at x: CGFloat) -> Int? {
        tabFrames.first { $0.value.minX <= x && x <= $0.value.maxX }?.key
    }

    /// One-step neighbour swap while dragging, decided by the neighbour's
    /// MIDPOINT rather than its bounds.
    ///
    /// `tabSlot(at:)` swaps the moment the cursor touches a neighbour's edge —
    /// and the swap moves the chips, which puts the cursor back over the
    /// original, which swaps it straight back. That oscillation is the shake
    /// when a dragged tab sits ambiguously over another. Requiring the cursor
    /// to pass the neighbour's midpoint is stable: once the swap lands, the
    /// next midpoint is a whole chip away, so there is no return trip.
    ///
    /// One step at a time, so a fast drag walks the chips instead of teleporting
    /// past the ones it crossed.
    private func tabDragTarget(held: Int, x: CGFloat) -> Int? {
        if let next = tabFrames[held + 1], x > next.midX { return held + 1 }
        if held > 0, let prev = tabFrames[held - 1], x < prev.midX { return held - 1 }
        return nil
    }

    /// While the pill is in flight (click travel or drag) it turns to Liquid
    /// Glass and swells slightly; on arrival it sets back into solid accent.
    @State private var pillLiquid: Bool = false
    /// Live x of the pill's leading edge while the user drags it, nil otherwise.
    @State private var pillDragX: CGFloat? = nil
    /// Point-space override for `from` so a released drag animates home from
    /// wherever the finger left it, not from a slot boundary.
    @State private var pillFromXOverride: CGFloat? = nil
    /// Set while a drag commits `navMode` itself — stops `onChange(of: navMode)`
    /// from starting a second, competing travel.
    @State private var pillDragCommitting: Bool = false
    @State private var recents: [RecentItem] = RecentStore.load()
    /// Native sidebar visibility (mirrors `engine.uiNavVisible`).
    /// Find-bar caret blink driver.
    @State private var findCaretBlink = false
    /// Split divider drag override (committed to Core on release).
    @State private var liveSplitRatio: Double? = nil
    /// Minimap toggle (View menu).
    @AppStorage("suisei.minimap") private var minimapEnabled = true
    /// Live window-resize HUD (blur + dimensions).
    @State private var isLiveResizing = false
    @State private var liveResizeSize: CGSize = .zero
    /// Measured tab-chip row width (exact centering in the titlebar).
    @State private var tabStripContentWidth: CGFloat = 0
    @State private var tabStripHover = false
    @State private var plusBridge = PlusMenuBridge()
    /// Background code-file warm-up for the project's master directory.
    @StateObject private var projectIndex = ProjectIndex()
    /// Measured shell-chip row width — the header scroller hugs it until the
    /// chips outgrow the cap, so a single session sits right beside the "+".
    @State private var terminalChipsWidth: CGFloat = 0

    /// The right rail answers "what is THIS", about the one thing selected —
    /// as against the navigator's "where do I go". Xcode keeps the tab and
    /// shows a placeholder when an inspector does not apply rather than hiding
    /// it, and so do we: a rail whose tabs come and go cannot be learned.
    ///
    /// Outline is the deliberate exception to the rule. Every row IS a jump
    /// target, so by rights it belongs on the left — but it wants to be beside
    /// the code while you read, it already owns ⌥⌘0, and moving it left would
    /// make it fight the project tree for the same rail. Kept here knowingly.
    enum InspectorMode: String, CaseIterable {
        case outline, file, quickHelp
        var systemImage: String {
            switch self {
            case .outline: return "list.bullet.indent"
            case .file: return "doc"
            case .quickHelp: return "questionmark.circle"
            }
        }
        var title: String {
            switch self {
            case .outline: return "Outline"
            case .file: return "File"
            case .quickHelp: return "Quick Help"
            }
        }
    }

    /// A navigator mode is a LIST OF PLACES TO GO — every row is a jump target.
    /// Anything that merely toggles a panel is not a mode and does not belong
    /// here (the Debug Area toggle sits detached at the strip's trailing end).
    enum NavMode: String, CaseIterable {
        /// Xcode's order: Project · SCM · Find · Issues · Breakpoints
        case project, scm, find, issues, breakpoints
        var systemImage: String {
            switch self {
            case .project: return "folder"
            case .scm: return "arrow.triangle.branch"
            case .find: return "magnifyingglass"
            case .issues: return "exclamationmark.triangle"
            // Prefer widely available symbol — `breakpoint.fill` can render empty on some builds.
            case .breakpoints: return "bookmark.fill"
            }
        }
        var title: String {
            switch self {
            case .project: return "Project"
            case .scm: return "Source Control"
            case .find: return "Find"
            case .issues: return "Issues"
            case .breakpoints: return "Breakpoints"
            }
        }
    }

    // Live theme from Core (`engine.chrome.theme`) — updates when settings apply.
    private var theme: ThemeSnap { engine.chrome.theme }
    private var editorBg: Color { theme.color(theme.editorBg) }
    /// GUI contrast boost: TUI colors wash out on Retina; push fg/dim for readability.
    private var isLightTheme: Bool {
        let c = theme.editorBg
        let r = Double((c >> 16) & 0xFF)
        let g = Double((c >> 8) & 0xFF)
        let b = Double(c & 0xFF)
        return (0.299 * r + 0.587 * g + 0.114 * b) > 150
    }
    private var fg: Color {
        let base = theme.color(theme.fg)
        // Darker on light themes for Retina contrast.
        return isLightTheme ? mixColor(base, .black, 0.22) : base
    }
    private var dim: Color {
        let base = theme.color(theme.dim)
        return isLightTheme ? mixColor(base, .black, 0.30) : base.opacity(0.95)
    }

    private func mixColor(_ a: Color, _ b: Color, _ t: Double) -> Color {
        let nsA = NSColor(a).usingColorSpace(.sRGB) ?? NSColor(a)
        let nsB = NSColor(b).usingColorSpace(.sRGB) ?? NSColor(b)
        var ar: CGFloat = 0, ag: CGFloat = 0, ab: CGFloat = 0, aa: CGFloat = 0
        var br: CGFloat = 0, bg: CGFloat = 0, bb: CGFloat = 0, ba: CGFloat = 0
        nsA.getRed(&ar, green: &ag, blue: &ab, alpha: &aa)
        nsB.getRed(&br, green: &bg, blue: &bb, alpha: &ba)
        let u = CGFloat(t.clamped(to: 0...1))
        return Color(
            red: Double(ar + (br - ar) * u),
            green: Double(ag + (bg - ag) * u),
            blue: Double(ab + (bb - ab) * u)
        )
    }
    private var accent: Color { theme.color(theme.accent) }
    private var gutterFg: Color { isLightTheme ? Color.black.opacity(0.32) : dim.opacity(0.9) }
    /// Xcode-level current-line wash — barely visible, not a gray slab.
    private var cursorLineBg: Color {
        isLightTheme ? Color.black.opacity(0.035) : Color.white.opacity(0.055)
    }
    private var selBg: Color {
        theme.color(theme.selection).opacity(isLightTheme ? 0.45 : 0.55)
    }
    private var caretColor: Color { theme.color(theme.caret) }

    var body: some View {
        // Welcome is its own Window scene (SuiseiApp) with system chrome.
        // This view is the editor shell only.
        //
        // Xcode 26 anatomy: the navigator is a floating rounded card — same
        // language as the outline card, but full height, and tall enough that
        // the traffic lights + our toggle sit INSIDE its empty top area. The
        // card is hand-drawn: NavigationSplitView also renders a floating card
        // on this OS, but pins it below the titlebar safe area no matter what,
        // which strands the lights + toggle in a bare strip ABOVE the card
        // (the round-9/10 complaint). The scene's .hiddenTitleBar keeps the
        // lights while letting our card rise to the true window top.
        ZStack(alignment: .top) {
            // Bottom z-layer: the status bar spans the FULL window width. The
            // sidebar card (full height, drawn above) overlaps its left part,
            // so the bar's top line visibly passes UNDER the widget instead of
            // stopping at an arbitrary x (the recurring "라인이 끊김").
            VStack(spacing: 0) {
                Spacer()
                statusLine
            }

            VStack(spacing: 0) {
                HStack(spacing: 0) {
                    // No sidebar here: the navigator is not a column any
                    // more. The island spans from the window's left edge and
                    // the widget FLOATS over it (see the layer after this
                    // VStack) — only the editor's content steps aside for it.
                    detailColumn
                    // Hoisted OUT of `detailColumn` on purpose. Nested there it
                    // sat between that column's top band and its status-bar
                    // spacer, so it could never reach the window floor no
                    // matter how its corners or insets were adjusted — three
                    // rounds were spent on those before the level itself was
                    // suspected. Xcode's inspector is a full-height column and
                    // the status bar stops where it begins; this is that.
                    if outlineVisible {
                        inspectorColumn
                            .frame(width: CGFloat(inspectorW))
                            .frame(maxHeight: .infinity)
                            .contentShape(Rectangle())
                            // The grip OVERLAYS the seam instead of owning a
                            // layout slot. Even a 1pt slot showed the editor
                            // card's shadow through it as a dark sliver —
                            // exactly the line this arrangement exists to not
                            // have. As an overlay it floats above both sides
                            // and takes no width at all.
                            .overlay(alignment: .leading) {
                                PanelResizeGrip(
                                    size: $inspectorW, minS: 170, maxS: 380,
                                    axis: .horizontal, invert: true,
                                    fg: fg,
                                    onBegan: beginPanelLiveResize,
                                    onEnded: endPanelLiveResize
                                )
                            }
                            .zIndex(2)
                            .transition(.move(edge: .trailing).combined(with: .opacity))
                    }
                }
                .animation(.snappy(duration: 0.25), value: outlineVisible)
                .animation(nil, value: inspectorW)
            }

            // ── Floating navigator ──────────────────────────────────────────
            // The island passes cleanly beneath the widget and the widget
            // floats on that one continuous surface, separated by nothing but
            // its own shadow. This is also why the terminal↔sidebar metaball
            // bridge is gone: there is no shell channel left to bridge — the
            // ground between the two IS the island now.
            // ALWAYS PRESENT, moved by `offset`. Not `if` + `.transition`.
            //
            // The conditional-with-transition form would not animate its
            // REMOVAL, and five measured attempts failed to make it: explicit
            // `withAnimation` vs implicit `.animation(_:value:)`, the animation
            // on the outer container vs on the panel's own, adding `.zIndex`,
            // applying the transition before the full-width stretch instead of
            // after, and dropping the `windowLiveResizing` publish that lands
            // in the same update. Every one of them measured open ≈ 5–8 frames
            // of motion against close = 1 frame, i.e. the panel snapped shut.
            //
            // An offset is a plain animatable property, so both directions are
            // the same interpolation running in opposite senses and cannot
            // diverge. Keeping the panel mounted also means opening it no
            // longer re-runs `ProjectTreeView`'s `onAppear` rebuild.
            sidebarColumn
                .frame(width: CGFloat(navW))
                .frame(maxHeight: .infinity)
                .background(editorBg)
                .clipShape(RoundedRectangle(
                    cornerRadius: ContentView.panelCornerRadius, style: .continuous
                ))
                .overlay(
                    RoundedRectangle(
                        cornerRadius: ContentView.panelCornerRadius, style: .continuous
                    )
                    .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
                )
                // The resize grip rides the widget's own trailing edge now
                // that the sidebar owns no slot in the layout.
                // (240 floor: five modes plus the detached toggle need the
                // room — see `navStripIcon`.)
                .overlay(alignment: .trailing) {
                    PanelResizeGrip(
                        size: $navW, minS: 240, maxS: 460,
                        axis: .horizontal, invert: false,
                        fg: fg,
                        onBegan: beginPanelLiveResize,
                        onEnded: endPanelLiveResize
                    )
                }
                .shadow(
                    color: .black.opacity(isLightTheme ? 0.07 : 0.30),
                    radius: 9, y: 2
                )
                .padding(.leading, ContentView.panelGap)
                .padding(.vertical, ContentView.panelGap)
                // Far enough left to clear its own shadow as well as the card.
                .offset(x: engine.uiNavVisible ? 0 : -(CGFloat(navW) + 40))
                .opacity(engine.uiNavVisible ? 1 : 0)
                // Hidden means untouchable — an off-screen panel must not eat
                // clicks meant for the editor beneath it.
                .allowsHitTesting(engine.uiNavVisible)
                .frame(
                    maxWidth: .infinity, maxHeight: .infinity,
                    alignment: .leading
                )
                // Below `topBar` (2) so the sidebar toggle is never covered by
                // the panel it toggles.
                .zIndex(1)

            // Custom titlebar row: full window width → tabs stay WINDOW-centered
            // regardless of the sidebar, and everything here is SwiftUI content
            // (blurable, coverable — impossible with NSToolbar).
            topBar
                // Above the navigator (1) so the sidebar toggle is never
                // covered by the panel it toggles.
                .zIndex(2)
        }
        // THE INSPECTOR'S ANIMATION, verbatim — `.snappy(duration: 0.25)`
        // keyed on the visibility value, plus `nil` for the width so dragging a
        // resize grip never animates. The navigator was driven by an explicit
        // `withAnimation` transaction instead, which is what differed and what
        // stuttered.
        //
        // It sits HERE, on the container above both, rather than on the panel:
        // the navigator is a floating overlay, so the editor does not resize
        // around it — its content steps aside via `editorCard`'s leading
        // padding, in a different subtree. Only a modifier above both moves
        // them as one.
        //
        // The inspector's `.zIndex(2)` is deliberately NOT copied: it is
        // positional, not part of the animation, and in this ZStack it raised
        // the navigator above `topBar` and hid the sidebar toggle behind it.
        // `.snappy` is a spring WITH bounce (≈0.15). On the inspector's narrow
        // column that wobble is invisible; on a full-height panel it is not.
        // Explicit `bounce: 0` rather than `.smooth`, so the intent is in the
        // code and not in a preset that might change.
        .animation(.spring(duration: 0.25, bounce: 0), value: engine.uiNavVisible)
        // Everything starts at the true window top (no reserved titlebar strip):
        // the navigator card swallows the traffic lights + toggle, the top bar
        // row shares that height over the detail side.
        .ignoresSafeArea(.container, edges: .top)
        .overlay {
            // Settings is a separate Window — not an in-app overlay.
            if engine.chrome.palette.open {
                paletteOverlay
                    // Centre on the EDITOR, not the window. The overlay hangs
                    // off the root, so it centred on the window — and the two
                    // panels flanking the editor are not the same width, so
                    // window-centred is never editor-centred. Measured with
                    // both open: navigator edge at x=318, editor edge at
                    // x=1696, editor centre 1007 against a window centre of
                    // 1023.5 — the palette sat 16.5px right, which is exactly
                    // what it looked like.
                    .offset(x: (navReserved - inspectorReserved) / 2)
                    .zIndex(100)
                    // Removal is IMMEDIATE (.identity): the animated removal
                    // could wedge mid-transition, leaving an invisible view
                    // that swallowed every click/hover in the top band until
                    // some other state change re-evaluated the tree (the
                    // "Esc from palette kills the right-side buttons" bug).
                    .transition(.asymmetric(
                        insertion: .opacity.combined(with: .scale(scale: 0.98, anchor: .top)),
                        removal: .identity
                    ))
            }
            if engine.chrome.completions.open {
                completionsOverlay.zIndex(80)
            }
        }
        .animation(.snappy(duration: 0.22), value: engine.chrome.palette.open)
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willStartLiveResizeNotification)) { note in
            guard let w = note.object as? NSWindow, isEditorWindow(w) else { return }
            engine.windowLiveResizing = true
            ResizeHudWindow.shared.show(over: w)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didResizeNotification)) { note in
            guard let w = note.object as? NSWindow, isEditorWindow(w) else { return }
            // AppKit re-lays the standard buttons out on resize — re-assert.
            ContentView.applyTrafficLightInset(w)
            guard engine.windowLiveResizing else { return }
            ResizeHudWindow.shared.update(over: w)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didEndLiveResizeNotification)) { note in
            guard let w = note.object as? NSWindow, isEditorWindow(w) else { return }
            engine.windowLiveResizing = false
            ResizeHudWindow.shared.hide()
            engine.settleEditorResize()
        }
        // ONE backing surface for the entire window (Xcode: cards float on a
        // uniform base — any second base color shows up as a hard seam).
        .background(shellBase)
        .foregroundStyle(.primary)
        // Chrome UI uses a fixed size — Cmd+/- only zooms the editor canvas (EditorMetrics).
        .font(.system(size: 13, weight: .regular))
        .preferredColorScheme(isLightTheme ? .light : .dark)
        .frame(minWidth: 640, minHeight: 400)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // Keys route through the NSEvent monitor (EngineBridge); a container-level
        // .focusable()/.onKeyPress here double-captured input and stole focus
        // from native text fields (the tree Filter couldn't be typed into).
        .focused($focused)
        .onAppear {
            loadPanelSizes()
            focused = true
            engine.activateInput()
            recents = RecentStore.load()
            applyWindowAppearance()
            syncNavFromCore()
            SuiseiWindowLayout.apply(welcome: false, animate: false)
            engine.uiNavVisible = true
            navMode = .project
            engine.ensureProjectTree()
            engine.uiDebugVisible = false
            if !engine.chrome.welcome {
                engine.ensureEditorFocus()
            }
        }
        // Deferred to .default run-loop mode: the theme flips while the
        // Settings theme POPUP's menu-tracking session is still running, and
        // restyling windows mid-tracking threw a layout exception straight
        // into +[NSApplication _crashOnException:] (the "switch to Dark →
        // instant crash"). .default doesn't run until tracking ends.
        .onChange(of: isLightTheme) { _, _ in
            RunLoop.main.perform(inModes: [.default]) {
                applyWindowAppearance()
            }
        }
        .onChange(of: engine.referencesActive) { _, active in
            // "Find All References" surfaces its results in the Find navigator.
            if active {
                engine.uiNavVisible = true
                navMode = .find
            }
        }
        .onChange(of: navMode) { old, new in
            guard !pillDragCommitting,
                  let a = NavMode.allCases.firstIndex(of: old),
                  let b = NavMode.allCases.firstIndex(of: new), a != b else { return }
            selectionFrom = a
            selectionTo = b
            selectionProgress = 0
            // `settle`, not `morph`: `TravellingPill` clamps its progress to
            // 0…1, so a spring that overshoots past 1 spends its bounce doing
            // nothing on screen — the pill arrives early and then sits there
            // for the rest of the duration. A bounce-free curve uses the whole
            // travel, and the swell-and-return already supplies the character.
            withAnimation(.easeOut(duration: 0.12)) { pillLiquid = true }
            withAnimation(NavStrip.settle) {
                selectionProgress = 1
            } completion: {
                withAnimation(.easeOut(duration: 0.2)) { pillLiquid = false }
            }
        }
        .onChange(of: inspectorMode) { old, new in
            guard let a = InspectorMode.allCases.firstIndex(of: old),
                  let b = InspectorMode.allCases.firstIndex(of: new), a != b else { return }
            inspectorFrom = a
            inspectorTo = b
            inspectorProgress = 0
            withAnimation(.easeOut(duration: 0.12)) { inspectorLiquid = true }
            withAnimation(NavStrip.settle) {
                inspectorProgress = 1
            } completion: {
                withAnimation(.easeOut(duration: 0.2)) { inspectorLiquid = false }
            }
        }
        .onChange(of: engine.chrome.scm.open) { _, open in
            if open {
                navMode = .scm
                engine.uiNavVisible = true
            }
        }
        .onChange(of: engine.chrome.terminal.open) { _, open in
            // Side terminal (Ctrl+T) → Debug area. Full-panel (⌃⇧T) stays in the editor split.
            withAnimation(.snappy(duration: 0.28)) {
                if open && !engine.chrome.terminal.fullPanel {
                    engine.uiDebugVisible = true
                } else if !open {
                    engine.uiDebugVisible = false
                }
            }
        }
        .onChange(of: engine.chrome.terminal.fullPanel) { _, full in
            if full { engine.uiDebugVisible = false }
        }
        .onChange(of: engine.chrome.settings.open) { _, open in
            if open {
                openWindow(id: "settings")
            } else {
                dismissWindow(id: "settings")
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiOpenSettingsWindow)) { _ in
            if !engine.chrome.settings.open {
                engine.openSettings()
            }
            openWindow(id: "settings")
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiNavProject)) { _ in
            navMode = .project
            engine.uiNavVisible = true
            applyNavMode(.project)
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiNavScm)) { _ in
            navMode = .scm
            engine.uiNavVisible = true
            applyNavMode(.scm)
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiNavFind)) { _ in
            navMode = .find
            engine.uiNavVisible = true
            applyNavMode(.find)
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiNavBreakpoints)) { _ in
            navMode = .breakpoints
            engine.uiNavVisible = true
            applyNavMode(.breakpoints)
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiNewUntitledTab)) { _ in
            engine.openBlankTab()
            focused = true
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            // NEVER touch window styling from reactivation. Even the guarded
            // applyWindowAppearance re-froze the app here (verified twice):
            // AppKit resets the titlebar container + button frames on focus
            // return, so the "changed?" guards pass and the re-styling still
            // rebuilds the window bridge, killing NSHostingView hit-testing
            // (SwiftUI controls dead, AppKit canvas alive). Appearance sync
            // for every window happens in onAppear + onChange(isLightTheme);
            // the lights/container re-clamp rides didResizeNotification only.
            focused = true
            engine.activateInput()
        }
    }

    /// Width the navigator takes out of the window, 0 when hidden. Overlays
    /// that want to sit in the middle of the EDITOR offset by the difference
    /// between this and `inspectorReserved`.
    private var navReserved: CGFloat {
        engine.uiNavVisible ? CGFloat(navW) + ContentView.panelGap : 0
    }

    /// The inspector's equivalent.
    private var inspectorReserved: CGFloat {
        outlineVisible ? CGFloat(inspectorW) + ContentView.panelGap : 0
    }

    /// Outline shows unless the git workbench owns the editor slot.
    private var outlineVisible: Bool {
        engine.uiInspectorVisible && !engine.chrome.gitWb.open
    }

    /// Is this one of OUR document windows?
    ///
    /// Structural, not a title-string test. `w.title != "Settings"` was true of
    /// every AppKit auxiliary window as well — popovers, sheets, tooltips, the
    /// open panel, SwiftUI's own helper windows — and most of those carry no
    /// titlebar at all. `applyWindowAppearance` walks `NSApp.windows`, so it
    /// handed one to `styleTrafficLights`, whose first act is to read
    /// `titlebarAccessoryViewControllers`; that throws on a window without a
    /// titlebar, and AppKit escalates a throw during layout straight into
    /// `+[NSApplication _crashOnException:]`.
    ///
    /// That is the intermittent crash (report 2026-07-26 08:07): intermittent
    /// because it depends on which auxiliary windows happen to exist at the
    /// moment the appearance sync runs.
    private func isEditorWindow(_ w: NSWindow) -> Bool {
        guard w.styleMask.contains(.titled), !(w is NSPanel) else { return false }
        return w.title != "Settings" && w.title != "Welcome"
    }

    /// Run a panel show/hide animation without per-frame engine resizes —
    /// the 240-row recompose at 30Hz read as stutter, especially on big files.
    /// The motion itself lives on the bridge so the menu commands share it.
    private func animatePanels(_ body: () -> Void) {
        engine.animatingPanels(body)
    }

    private func syncNavFromCore() {
        if engine.chrome.scm.open {
            navMode = .scm
            engine.uiNavVisible = true
        }
    }

    /// Load panel sizes once; migrate old explorerW/scmW keys if needed.
    private func loadPanelSizes() {
        let d = UserDefaults.standard
        if d.object(forKey: "suisei.panel.navW") == nil {
            let e = d.double(forKey: "suisei.panel.explorerW")
            let s = d.double(forKey: "suisei.panel.scmW")
            if e >= 200 {
                navW = e
            } else if s >= 200 {
                navW = s
            }
            d.set(navW, forKey: "suisei.panel.navW")
        } else {
            // Floor matches `SidebarResizeStrip`'s minS — a width persisted
            // under the old 200 floor would otherwise load back and clip the
            // navigator strip.
            let v = d.double(forKey: "suisei.panel.navW")
            if v >= 240 { navW = v }
        }
        let t = d.double(forKey: "suisei.panel.termW")
        if t >= 200 { termW = t }
        // Key keeps its old name so a saved height survives the XLC removal.
        let x = d.double(forKey: "suisei.panel.xlcH")
        if x >= 100 { debugAreaH = x }
        let i = d.double(forKey: "suisei.panel.inspectorW")
        if i >= 140 { inspectorW = i }
    }

    private func persistPanelSizes() {
        let d = UserDefaults.standard
        d.set(navW, forKey: "suisei.panel.navW")
        d.set(termW, forKey: "suisei.panel.termW")
        d.set(debugAreaH, forKey: "suisei.panel.xlcH")
        d.set(inspectorW, forKey: "suisei.panel.inspectorW")
    }

    // MARK: - Xcode-like shell (flat panes, native sidebar/inspector)

    /// Chrome base slightly separated from the editor fill.
    private var shellBase: Color {
        isLightTheme
            ? mixColor(editorBg, .black, 0.03)
            : mixColor(editorBg, .black, 0.16)
    }

    /// Sidebar column — navigator strip on top, flat content below.
    /// NavigationSplitView hosts it full height into the titlebar (Xcode style):
    /// no background override here — the system sidebar material must run all
    /// the way to the top, with the traffic lights floating over it.
    private var sidebarColumn: some View {
        VStack(spacing: 0) {
            // Top band: traffic lights + toggle live here (Xcode 1st row).
            Spacer().frame(height: ContentView.topBandHeight)
            navigatorModeStrip
            dockedNavigator
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// Detail column: editor stage (+ outline card) + status line.
    private var detailColumn: some View {
        VStack(spacing: 0) {
            Spacer().frame(height: ContentView.topBandHeight) // top band (tabs row)
            editorCard
            // Status bar renders at the ROOT bottom layer (full width, under
            // the sidebar card) — reserve its height here.
            Spacer().frame(height: ContentView.statusBarHeight)
        }
    }

    /// Full-height right column. Only the top band is reserved — no status-bar
    /// spacer, because the bar stops before this column rather than running
    /// beneath it.
    private var inspectorColumn: some View {
        VStack(spacing: 0) {
            Spacer().frame(height: ContentView.topBandHeight)
            inspectorPanel
                .frame(maxHeight: .infinity)
        }
        // Opaque shell tone, and that tone is the ONLY separator against the
        // editor — Xcode draws no line here. Opacity also matters for its own
        // sake: it is what swallows the editor card's shadow on this side, and
        // it covers the status bar's strip so the column runs unbroken from
        // the very top of the window to its floor.
        .background(shellBase)
    }

    /// Editor and terminal as ONE surface —
    /// which is how Xcode draws them, and the only arrangement without seams.
    /// Three floating cards could be pushed flush and would still show a
    /// channel between them: each carried its own border and shadow, and two of
    /// those meeting is a groove. The corners are square along the bottom so
    /// the surface meets the status bar instead of hovering a `panelGap` above
    /// it, which is the gap under the terminal and the inspector alike.
    /// The editor island: editor + terminal as one card, FLOATING on the
    /// chrome with the same `panelGap` on all four sides — which is Xcode 26's
    /// actual grammar. Butting it flush against the inspector and the status
    /// bar was tried and looked severed: a white slab ending in a raw cut
    /// against flat gray, with its bottom shadow smeared across the bar
    /// because the shadow had nowhere to land. A uniform gap gives every edge
    /// the same soft boundary — gap, shadow, hairline — and the shadow falls
    /// into the gap instead of onto a neighbouring surface.
    private var editorCard: some View {
        let shape = RoundedRectangle(
            cornerRadius: ContentView.panelCornerRadius, style: .continuous
        )
        // The island starts at the WINDOW edge and passes beneath the floating
        // navigator; only the CONTENT steps aside.
        //
        // No extra breathing room beyond the panel's own width. The `+ 7` that
        // used to be here was SwiftUI PADDING, i.e. outside the editor canvas —
        // so the canvas could not paint it, and the cursor-line highlight
        // stopped 7pt short of the sidebar with bare card background showing
        // through (measured: sidebar border ends at x=316, highlight starts at
        // x=324). The navigator is a floating card with its own shadow; that
        // shadow is the separation, and it falls on content that now reaches.
        let contentInset = engine.uiNavVisible ? CGFloat(navW) : 0
        return editorIsolatedStage
            .padding(.leading, contentInset)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background {
                ZStack(alignment: .bottom) {
                    editorBg
                    // The terminal's tint band lives on the CARD, spanning the
                    // full island — under the navigator too. Painted on the
                    // inset content it stopped at the content's left edge, a
                    // vertical cut mid-surface. Its left fillet hides beneath
                    // the widget; the right one still sweeps the island wall.
                    if engine.uiDebugVisible {
                        dockedTerminalShape
                            .fill(terminalDockFill)
                            .overlay(
                                dockedTerminalShape.stroke(
                                    Color(nsColor: .separatorColor).opacity(0.6),
                                    lineWidth: 1
                                )
                            )
                            .overlay(alignment: .top) {
                                // Full-width rule under the 28pt terminal
                                // header — beneath the navigator included.
                                Rectangle()
                                    .fill(Color(nsColor: .separatorColor).opacity(0.6))
                                    .frame(height: 1)
                                    .offset(y: 28)
                            }
                            .frame(height: CGFloat(debugAreaH))
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    }
                }
            }
            .clipShape(shape)
            .overlay(shape.strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1))
            .shadow(color: .black.opacity(isLightTheme ? 0.07 : 0.30), radius: 9, y: 2)
            .padding(ContentView.panelGap)
    }

    /// The terminal's junction with the editor surface.
    private var dockedTerminalShape: DockedPanelShape {
        DockedPanelShape(fillet: ContentView.panelCornerRadius)
    }

    /// The terminal grid's own background — behind the glyphs AND behind the
    /// whole panel, so no sliver of another fill can show at its edges.
    private var terminalGridBg: Color {
        mixColor(editorBg, .black, isLightTheme ? 0.035 : 0.18)
    }

    /// The docked terminal region is filled with the GRID's own colour.
    ///
    /// It used to be its own barely-there tint, one shade off the grid. The
    /// grid view does not cover the dock shape exactly — measured 4pt short at
    /// the bottom in one layout and 27pt in another — and every point of that
    /// difference showed as a strip of a slightly different colour along the
    /// terminal's edge. One colour across the whole region makes the seam
    /// impossible instead of leaving it to arithmetic that has already been
    /// wrong twice.
    private var terminalDockFill: Color { terminalGridBg }

    private var floatingPanelBackground: some View {
        ZStack {
            editorBg.opacity(0.94)
            // Frosted lift without washing out theme colors.
            Rectangle()
                .fill(.ultraThinMaterial)
                .opacity(isLightTheme ? 0.55 : 0.35)
        }
    }

    /// Xcode 26 navigator row: the mode icons in a bordered group
    /// (segmented-control look), the Debug Area toggle at the trailing end.
    ///
    /// The chrome is ONE pill that splits (`SplitCapsule`), and it splits only
    /// while the Debug Area is open — so the framing itself reports state:
    /// merged means the terminal is closed, separated means it is running.
    /// A toggle is not a navigator mode, and the gap is what says so.
    private var navigatorModeStrip: some View {
        let separated = engine.uiDebugVisible
        let p: CGFloat = separated ? 1 : 0
        return GeometryReader { geo in
            // Every width is computed rather than left to `maxWidth: .infinity`:
            // the metaball behind the glyphs has to land on the SAME numbers, and
            // a flexible layout can only be asked where it ended up, not told.
            let inner = geo.size.width - NavStrip.inset * 2
            let modes = CGFloat(NavMode.allCases.count)
            let share = inner / (modes + 1)
            // Merged, the toggle is just another slot in the distribution —
            // otherwise the last gap reads wider than the rest and the button
            // looks exiled. Separated, it shrinks to a single glyph and the
            // modes take the space back.
            let toggleW = share + (NavStrip.iconW - share) * p
            let gapW = NavStrip.layoutGap * p
            let modesW = inner - gapW - toggleW
            // The metaball's leading half is NOT `modesW` — it has to SWALLOW
            // the trailing half while merged. Sized from the layout it overlaps
            // by only `2 * inset`, and two caps of radius `height/2` meeting on
            // a 4pt overlap pinch into a permanent waist: the pill looks
            // squeezed at rest, when it should read as one unbroken capsule.
            // Full width at p=0, retracting to meet `modesW` exactly at p=1.
            let retract = (NavStrip.layoutGap + NavStrip.iconW) * p

            HStack(spacing: 0) {
                // Icons SPREAD to fill the rail instead of packing left. Xcode's
                // navigator does this — compare its strip at minimum width against
                // a widened sidebar and the icon gaps grow, they don't leave dead
                // space at the trailing end.
                //
                // ZStack, not background, for the LIQUID state: per Apple's
                // Liquid Glass guidance the glass is a sized view in a
                // `GlassEffectContainer` — glass inside `.background` is the
                // documented anti-pattern, and it composited the lens ABOVE
                // these icons as a white blob. As the first sibling it renders
                // beneath the buttons and samples the rail.
                ZStack(alignment: .leading) {
                    // Glass BELOW the glyphs — settled after trying every
                    // layering. Above them, `.regular` frosts the icons to
                    // mush and `.clear` smears them at this 30pt scale; a rail
                    // where you cannot read the destinations is broken no
                    // matter how pretty. What sells the native look instead is
                    // the chip's own rim + the lift shadow + the interactive
                    // shimmer, all of which the glass carries itself. (The
                    // reference screenshot's drama is its PHOTO backdrop —
                    // over flat light chrome, Apple's own glass is exactly
                    // this quiet.)

                    HStack(spacing: 0) {
                    ForEach(NavMode.allCases, id: \.self) { mode in
                        Button {
                            // Re-clicking the selected mode is a NO-OP, not a
                            // collapse. Hiding the rail lives on ⌘0 and the
                            // top-bar toggle; a mode button that sometimes
                            // selects and sometimes closes punished the double
                            // click.
                            if !(engine.uiNavVisible && navMode == mode) {
                                navMode = mode
                                engine.uiNavVisible = true
                                applyNavMode(mode)
                            }
                            focused = true
                        } label: {
                            navStripIcon(
                                mode.systemImage,
                                lit: navSlotLit(mode, slotWidth: modesW / modes)
                            )
                        }
                        .buttonStyle(.plain)
                        .help(mode.title)
                        .frame(width: modesW / modes)
                    }
                    }
                    // The SOLID pill may stay a background — it is a plain
                    // fill, not glass. Hidden while the liquid is up.
                    .background {
                        let slot = modesW / modes
                        if engine.uiNavVisible {
                            // Hands over to the glass across the TRAVEL, not at
                            // its ends: solid at both endpoints, gone at
                            // mid-flight. Same geometry as the glass pill, so
                            // the two track each other exactly and there is
                            // never a size step to hide.
                            // `pillDragX` first, for BOTH ends: while the user
                            // is carrying the pill there is no travel, it just
                            // sits where the cursor put it. The glass overlay
                            // used to be the only thing honouring the drag, so
                            // removing the glass took the pill with it and a
                            // drag moved nothing at all.
                            TravellingPill(
                                progress: selectionProgress,
                                from: pillDragX ?? pillFromXOverride
                                    ?? CGFloat(selectionFrom) * slot,
                                to: pillDragX ?? CGFloat(selectionTo) * slot,
                                width: slot
                            )
                            .fill(Color.accentColor)
                        }
                    }

                }
                .frame(width: modesW)
                .animation(.easeOut(duration: 0.15), value: pillLiquid)
                // Drag the pill itself: the rail is a slider. Plain clicks
                // still reach the mode buttons (3pt threshold).
                .simultaneousGesture(
                    DragGesture(minimumDistance: 3, coordinateSpace: .local)
                        .onChanged { v in
                            guard engine.uiNavVisible, modes > 0 else { return }
                            let slot = modesW / modes
                            if pillDragX == nil {
                                withAnimation(.easeOut(duration: 0.12)) { pillLiquid = true }
                            }
                            pillDragX = min(max(v.location.x - slot / 2, 0), modesW - slot)
                        }
                        .onEnded { v in
                            guard engine.uiNavVisible, modes > 0 else { return }
                            let slot = modesW / modes
                            let idx = min(
                                NavMode.allCases.count - 1,
                                max(0, Int(v.location.x / slot))
                            )
                            let target = NavMode.allCases[idx]
                            pillFromXOverride = pillDragX
                            pillDragX = nil
                            selectionFrom = idx
                            selectionTo = idx
                            selectionProgress = 0
                            pillDragCommitting = true
                            if navMode != target {
                                navMode = target
                                applyNavMode(target)
                            }
                            pillDragCommitting = false
                            focused = true
                            withAnimation(NavStrip.settle) {
                                selectionProgress = 1
                            } completion: {
                                pillFromXOverride = nil
                                withAnimation(.easeOut(duration: 0.2)) { pillLiquid = false }
                            }
                        }
                )

                Color.clear.frame(width: gapW, height: 1)

                Button {
                    toggleDebugArea()
                } label: {
                    navStripToggleIcon("terminal", on: engine.uiDebugVisible)
                }
                .buttonStyle(.plain)
                .help("Debug Area · ⌘⇧Y")
                .frame(width: toggleW)
            }
            .padding(NavStrip.inset)
            .background {
                let pill = SplitCapsule(
                    leadingWidth: geo.size.width - retract,
                    trailingWidth: toggleW + NavStrip.inset * 2
                )
                pill
                    .fill(Color.primary.opacity(isLightTheme ? 0.035 : 0.06))
                    .overlay(
                        // `stroke`, not `strokeBorder` — see `SplitCapsule`.
                        pill.stroke(
                            Color(nsColor: .separatorColor).opacity(0.6), lineWidth: 1
                        )
                    )
                    // Separated from the tree by depth, not a hairline.
                    .shadow(color: .black.opacity(isLightTheme ? 0.10 : 0.35), radius: 6, y: 2)
            }
        }
        .frame(height: NavStrip.iconH + NavStrip.inset * 2)
        .animation(NavStrip.settle, value: separated)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }

    /// Geometry and motion for the navigator strip. Kept together because the
    /// icon layout and the metaball behind it must agree to the point — a
    /// mismatch shows up as glyphs sitting off-centre in their own chrome.
    private enum NavStrip {
        static let iconW: CGFloat = 28
        static let iconH: CGFloat = 24
        /// Breathing room between the icons and the pill's edge.
        static let inset: CGFloat = 2
        /// Gap in the LAYOUT once separated. The two halves each wrap their
        /// contents with `inset`, so the gap you actually see is this minus
        /// `2 * inset` — 12 here reads as 8 on screen.
        static let layoutGap: CGFloat = 12

        /// The Dynamic Island morph. Grounded in SwiftUI's own presets, read
        /// off `Spring` at runtime rather than guessed: `.smooth` is bounce 0,
        /// `.snappy` 0.15, `.bouncy` 0.30, and the community-converged Dynamic
        /// Island spring (response 0.4 / damping 0.6) is duration 0.4 with
        /// bounce 0.40. We sit just under it — the Island is a 126pt shape on a
        /// black bezel, this is a 28pt pill in a dense rail, and the full 0.40
        /// reads as jitter at that size.
        /// NO bounce, deliberately. The Dynamic Island reference is about the
        /// GOO — the neck that forms and snaps — and that is pure geometry,
        /// independent of the curve. The overshoot is separable and it costs
        /// more than it gives here: toggling the Debug Area resizes every slot
        /// in the rail, the selection indicator rides along, and a solid
        /// accent-coloured pill springing past its slot every single time the
        /// terminal opens reads as a glitch rather than as character.
        ///
        /// One curve for the whole strip, chrome included. The chrome IS the
        /// background of the icon row, so giving it a bouncier spring would let
        /// the capsule overshoot while the glyphs inside it did not — the pill
        /// and its contents visibly drifting apart mid-flight.
        ///
        /// Reference points, measured off SwiftUI's own presets rather than
        /// guessed: `.smooth` = duration 0.5 / bounce 0.0, `.snappy` =
        /// 0.5 / 0.15, `.bouncy` = 0.5 / 0.30, and the community-converged
        /// Dynamic Island spring (`response 0.4, dampingFraction 0.6`) converts
        /// to 0.4 / 0.40. Raise `bounce` here to put the overshoot back.
        static let settle: Animation = .spring(duration: 0.4, bounce: 0)
    }

    /// One navigator-strip glyph. The selection capsule FILLS the slot it was
    /// given rather than staying a fixed `iconW` — as the rail widens and the
    /// icons spread, the blue pill has to spread with them or it reads as a
    /// button that forgot to grow (Xcode's does grow). `iconW` survives only as
    /// the floor the toggle shrinks to once the pill splits.
    /// How lit a slot's icon is, 0 (unselected grey) … 1 (white on the pill).
    ///
    /// Tied to the pill's own travel, not to the click. Keying it off `navMode`
    /// flipped both icons the instant the button was pressed, and `navMode`
    /// changes one frame before the flight starts: the destination flashed
    /// white, dimmed for the whole journey, then snapped back. Now the origin
    /// hands its ink over as the pill leaves and the destination takes it as
    /// the pill arrives — which is only legible because the travelling pill
    private func navSlotLit(_ mode: NavMode, slotWidth: CGFloat) -> Double {
        guard engine.uiNavVisible,
              let slot = NavMode.allCases.firstIndex(of: mode) else { return 0 }
        // Dragging: light whatever the pill is ACTUALLY sitting over. Keyed off
        // the selection instead, the icon the pill was carried AWAY from kept
        // its white ink — white on the bare rail, so it simply vanished. The
        // pill covers `[dragX, dragX + slotWidth]`; overlap with this slot is
        // how lit it is, which also crossfades correctly mid-slot.
        if let dragX = pillDragX, slotWidth > 0 {
            let lo = max(dragX, CGFloat(slot) * slotWidth)
            let hi = min(dragX + slotWidth, CGFloat(slot + 1) * slotWidth)
            return Double(max(0, hi - lo) / slotWidth)
        }
        guard selectionTo != selectionFrom else {
            return navMode == mode ? 1 : 0
        }
        // Light whatever the pill is passing over, not only its endpoints: on a
        // two-slot jump the slot in the middle sat in unselected grey while
        // solid accent slid across it. `t` is the progress at which the pill is
        // centred on this slot; `reach` is one slot's worth of progress.
        let span = Double(selectionTo - selectionFrom)
        let t = (Double(slot) - Double(selectionFrom)) / span
        let reach = 1 / abs(span)
        return max(0, 1 - abs(Double(selectionProgress) - t) / reach)
    }

    private func navStripIcon(_ systemImage: String, lit: Double) -> some View {
        Image(systemName: systemImage)
            .font(.system(size: 12.5, weight: .medium))
            .foregroundStyle(Color.secondary.mix(with: .white, by: lit))
            .frame(maxWidth: .infinity)
            .frame(height: NavStrip.iconH)
            .contentShape(Rectangle())
    }

    /// The Debug Area toggle keeps its OWN fill: it is not part of the mode
    /// sequence, so the travelling indicator must never visit it.
    private func navStripToggleIcon(_ systemImage: String, on: Bool) -> some View {
        navStripIcon(systemImage, lit: on ? 1 : 0)
            .background(
                Capsule(style: .continuous).fill(on ? Color.accentColor : Color.clear)
            )
    }

    /// Center titlebar: all document tabs in a wheel/trackpad-scrollable strip
    /// (no "+n" truncation) + a "+" menu. The active tab auto-scrolls into view.
    private func documentTabStrip(maxWidth: CGFloat) -> some View {
        HStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 4) {
                        // Keyed by the DOCUMENT, not the slot. With the slot
                        // index as identity a reorder leaves the identity list
                        // unchanged and only the titles swap in place, so there
                        // is nothing for SwiftUI to move.
                        ForEach(engine.chrome.tabs, id: \.stableId) { tab in
                            ToolbarTabChip(
                                title: tab.title,
                                dirty: tab.dirty,
                                active: tab.active,
                                accent: Color.accentColor,
                                fg: Color.primary,
                                dim: Color.secondary,
                                isLight: isLightTheme,
                                tabId: Int(truncatingIfNeeded: tab.stableId),
                                pillSpace: tabPillSpace,
                                action: {
                                    focused = true
                                    engine.gotoTab(tab.id)
                                },
                                onClose: {
                                    focused = true
                                    engine.closeTab(tab.id)
                                }
                            )
                            .id(tab.stableId)
                            // Report this chip's slot so the drag below knows
                            // which one the cursor is over. Chip widths differ
                            // with the title, so slots cannot be computed.
                            .onGeometryChange(for: CGRect.self) { proxy in
                                proxy.frame(in: .named(Self.tabStripSpace))
                            } action: { rect in
                                tabFrames[tab.id] = rect
                            }
                            // Picked up: lifted, not merely dimmer. Without
                            // this a grabbed tab looked identical to a resting
                            // one and the reorder read as a teleport.
                            .scaleEffect(draggingTab == tab.id ? 1.06 : 1)
                            .opacity(draggingTab == tab.id ? 0.9 : 1)
                            .shadow(
                                color: .black.opacity(draggingTab == tab.id ? 0.28 : 0),
                                radius: draggingTab == tab.id ? 7 : 0,
                                y: draggingTab == tab.id ? 2 : 0
                            )
                            .zIndex(draggingTab == tab.id ? 1 : 0)
                            .animation(.snappy(duration: 0.16), value: draggingTab)
                            .contextMenu {
                                Button("Close Tab") { engine.closeTab(tab.id) }
                                Button("Close Other Tabs") {
                                    for other in engine.chrome.tabs.reversed() where other.id != tab.id {
                                        engine.closeTab(other.id)
                                    }
                                }
                            }
                        }
                    }
                    // ONE capsule for the whole strip, following the active
                    // chip's anchor. This is the only view holding both the
                    // chip it leaves and the chip it arrives at, which is why
                    // the travel is drawn — and animated — here.
                    .background {
                        if let activeId = engine.chrome.tabs
                            .first(where: \.active)
                            .map({ Int(truncatingIfNeeded: $0.stableId) }) {
                            Capsule(style: .continuous)
                                .fill(Color.primary.opacity(isLightTheme ? 0.10 : 0.14))
                                .matchedGeometryEffect(
                                    id: activeId, in: tabPillSpace, isSource: false
                                )
                        }
                    }
                    .padding(.horizontal, 2)
                    // Insert/remove AND reorder — the value is the ORDER of
                    // stable ids, which only now actually changes on a drag.
                    .animation(
                        .smooth(duration: 0.28),
                        value: engine.chrome.tabs.map(\.stableId)
                    )
                    // The strip is the only view holding BOTH the chip the
                    // active capsule leaves and the one it arrives at, so the
                    // travel has to be animated here.
                    .animation(
                        .snappy(duration: 0.22),
                        value: engine.chrome.tabs.first(where: \.active)?.id
                    )
                    // Take the chips' TRUE width, never the scroll frame's:
                    // without this the measurement can latch to the frame that
                    // it itself feeds, collapsing the strip to its floor and
                    // clipping the title down to a few characters.
                    .fixedSize(horizontal: true, vertical: false)
                    .onGeometryChange(for: CGFloat.self) { proxy in
                        proxy.size.width
                    } action: { w in
                        tabStripContentWidth = w
                    }
                }
                // Hug the measured chip row; scroll once it outgrows the space
                // actually available between the sidebar/toggle zone and the
                // trailing icons (a fixed 700 cap overflowed small windows —
                // tabs spilled across the sidebar). Before the first
                // measurement arrives, take the full allowance rather than the
                // floor — a 60pt strip reads as a broken tab bar.
                // Viewport pinned to the chips' own height so there is no
                // spare vertical room for a horizontal `ScrollView` to align
                // content within. (This was not the cause of the "+" sitting
                // low — that was type metrics, see `plusInkNudge` — but an
                // ambiguous viewport is worth removing anyway.)
                .frame(
                    width: tabStripContentWidth > 0
                        ? min(maxWidth, tabStripContentWidth)
                        : maxWidth,
                    height: Self.tabLabelFrameH
                )
                // Clipped ends melt away instead of hard-cutting.
                .mask(
                    HStack(spacing: 0) {
                        LinearGradient(
                            colors: [.clear, .black], startPoint: .leading, endPoint: .trailing
                        )
                        .frame(width: 14)
                        Rectangle().fill(Color.black)
                        LinearGradient(
                            colors: [.black, .clear], startPoint: .leading, endPoint: .trailing
                        )
                        .frame(width: 14)
                    }
                )
                .onChange(of: engine.chrome.tabs.firstIndex(where: \.active)) { _, idx in
                    guard let idx else { return }
                    withAnimation(.snappy(duration: 0.2)) {
                        proxy.scrollTo(idx, anchor: .center)
                    }
                }
            }

            // "+" holds a PERMANENT slot in the flow (stable position — the
            // overlay/offset variants kept drifting) but only materializes on
            // hover, per the user's call. The slot being real means no hover
            // boundary ever moves under the cursor.
            tabPlusMenu
                .fixedSize()
                .frame(width: 22, height: 26)
                .opacity(tabStripHover ? 1 : 0)
                .animation(.easeOut(duration: 0.12), value: tabStripHover)
        }
        // Fixed strip height keeps chips and "+" on one exact center line.
        .frame(height: 26)
        .contentShape(Rectangle())
        .coordinateSpace(name: Self.tabStripSpace)
        // GRAB AND MOVE, in AppKit — SwiftUI cannot win this one.
        //
        // This strip sits in the window's TITLEBAR REGION (the window is
        // `.fullSizeContentView` with a hidden titlebar). AppKit drags the
        // window from any view there whose `mouseDownCanMoveWindow` is true,
        // and it consumes the mouseDown BEFORE SwiftUI gesture arbitration
        // runs. Five SwiftUI gesture shapes were tried and none received a
        // single event; a synthetic drag on the strip dragged the window off
        // the screen instead. `.simultaneousGesture(DragGesture)` on a Button
        // is separately reported not to fire on macOS at all
        // (developer.apple.com/forums/thread/718959).
        //
        // An overlay that overrides `mouseDownCanMoveWindow` to false opts the
        // region out of titlebar dragging, and only then do mouse events
        // arrive. It therefore owns clicks too, so it routes them back.
        .overlay(
            TabStripMouse(
                slotAt: { x in tabSlot(at: x) },
                targetFor: { held, x in tabDragTarget(held: held, x: x) },
                onDrag: { held, to in
                    if engine.moveTab(from: held, to: to) { draggingTab = to }
                },
                onPick: { held in draggingTab = held },
                onClick: { slot in
                    focused = true
                    engine.gotoTab(slot)
                },
                onEnd: { draggingTab = nil }
            )
        )
        .zIndex(1)
        .onHover { tabStripHover = $0 }
    }

    /// Custom titlebar row: replaces NSToolbar so the sidebar reaches the
    /// traffic lights and the tab strip centers on the WINDOW — shrinking to
    /// the space left between the sidebar/toggle zone and the trailing icons.
    private var topBar: some View {
        GeometryReader { geo in
            // Window-centered while the tabs fit; once they overflow into a
            // scroll, the strip re-anchors after the sidebar and grows RIGHT
            // until just before the trailing icons — the "+" then lands out
            // by the outline header instead of mid-window (user spec).
            let leftReserve: CGFloat = engine.uiNavVisible ? CGFloat(navW) + 16 : 150
            let rightTight: CGFloat = 150
            let symCap = max(60, geo.size.width - 2 * max(leftReserve, 190)) - 22
            let wideCap = max(60, geo.size.width - leftReserve - rightTight) - 22
            let tabsOverflow = max(60, tabStripContentWidth) > symCap

            ZStack {
                // Empty areas drag the window; double-click zooms (titlebar
                // behaviors — the real titlebar container is clamped to the
                // lights zone so it can't swallow the buttons here).
                Color.clear
                    .contentShape(Rectangle())
                    .gesture(WindowDragGesture())
                    // INERT while the cursor is over the tab strip.
                    //
                    // `WindowDragGesture` drags the window at the AppKit level,
                    // and it won every press that began on a chip no matter
                    // what was tried above it — five gesture shapes and a
                    // zIndex, each one probe-confirmed to receive nothing.
                    // Ordering cannot help when the claimant is not competing
                    // in SwiftUI's arbitration; the layer has to stop taking
                    // hits where the tabs are. `tabStripHover` already tracks
                    // exactly that region.
                    .allowsHitTesting(!tabStripHover)
                    .zIndex(0)
                    .simultaneousGesture(
                        TapGesture(count: 2).onEnded {
                            NSApp.keyWindow?.performZoom(nil)
                        }
                    )

                // Tabs — true window center, faded at both clipped ends.
                // "+" hugs the last tab (8pt gap) and only materializes on
                // hover; it fades with a plain ease (a spring + scale moved
                // the hover boundary under the cursor — visible trembling)
                // and stays hit-testable at all times: yanking hit-testing
                // off a Menu mid-tracking wedges the app's event loop.
                if tabsOverflow {
                    HStack(spacing: 0) {
                        Spacer().frame(width: leftReserve)
                        documentTabStrip(maxWidth: wideCap)
                        Spacer(minLength: rightTight - 22)
                    }
                } else {
                    documentTabStrip(maxWidth: symCap)
                }

            HStack(spacing: 2) {
                // Toggle sits right AFTER the traffic lights (Xcode row 1) —
                // pinned to the card's top-right it crowded the corner.
                Spacer().frame(width: 86)
                ToolbarPlainIcon(
                    systemImage: "sidebar.left",
                    help: engine.uiNavVisible ? "Hide Navigator · ⌘0" : "Show Navigator · ⌘0",
                    active: false,
                    accent: Color.accentColor,
                    dim: Color.secondary,
                    iconSize: 15.5,
                    opticalNudgeX: 0.7
                ) {
                    // Content FIRST, animation second. `applyNavMode` runs a
                    // full engine recompose and a full chrome pull — measured
                    // at 13 ms — and running it after the toggle put all of
                    // that on the opening animation's first frame, which is
                    // where the panel visibly hitched on the way out.
                    if !engine.uiNavVisible { applyNavMode(navMode) }
                    animatePanels { engine.uiNavVisible.toggle() }
                    focused = true
                }
                Spacer()
                ToolbarPlainIcon(
                    systemImage: "magnifyingglass", help: "Go to File · ⌘P",
                    accent: Color.accentColor, dim: Color.secondary
                ) { engine.openFilePalette() }
                // No terminal toggle here — it lives detached at the trailing
                // end of the navigator strip. Two visible copies of one switch
                // is the thing that framing was meant to fix.
                ToolbarPlainIcon(
                    systemImage: "gearshape", help: "Settings · ⌘,",
                    accent: Color.accentColor, dim: Color.secondary
                ) {
                    engine.openSettings()
                    openWindow(id: "settings")
                }
                ToolbarPlainIcon(
                    systemImage: "sidebar.right", help: "Outline · ⌥⌘0",
                    active: engine.uiInspectorVisible,
                    accent: Color.accentColor, dim: Color.secondary,
                    opticalNudgeX: -0.6
                ) {
                    animatePanels { engine.uiInspectorVisible.toggle() }
                    focused = true
                }
            }
            .padding(.trailing, 10)
            // AppKit centres the traffic lights on the band; SwiftUI's centring
            // of this row lands 1 device px lower (measured: lights y-mid 235.5,
            // toggle glyph 236.5 at 2x). The system chrome is the anchor the eye
            // compares against, so the row moves to it — as one piece, keeping
            // the icons consistent with each other.
            .offset(y: -0.5)
            }
            .frame(width: geo.size.width, height: geo.size.height)
        }
        .frame(height: ContentView.topBandHeight)
        .frame(maxWidth: .infinity)
    }

    /// "+" tab/split menu. A plain BUTTON + manual NSMenu popup — SwiftUI's
    /// `Menu` (borderless popup-button control) lays its label out on AppKit's
    /// own baseline and the glyph permanently sat ~2pt high next to the tab
    /// chips no matter what frames wrapped it (the recurring "+가 위로 튐").
    /// A Button glyph centers exactly like every other chip.
    private var tabPlusMenu: some View {
        Button {
            plusBridge.engine = engine
            let menu = NSMenu()
            func add(_ title: String, _ sel: Selector) {
                let item = NSMenuItem(title: title, action: sel, keyEquivalent: "")
                item.target = plusBridge
                menu.addItem(item)
            }
            add("New Untitled Tab", #selector(PlusMenuBridge.newTab))
            add("Next Tab", #selector(PlusMenuBridge.nextTab))
            add("Previous Tab", #selector(PlusMenuBridge.prevTab))
            menu.addItem(.separator())
            add("Split Editor Right", #selector(PlusMenuBridge.splitRight))
            add("Split Editor Below", #selector(PlusMenuBridge.splitBelow))
            add("Focus Next Pane", #selector(PlusMenuBridge.focusNextPane))
            add("Close Focused Pane", #selector(PlusMenuBridge.closeFocusedPane))
            if let event = NSApp.currentEvent,
               let view = NSApp.keyWindow?.contentView
            {
                NSMenu.popUpContextMenu(menu, with: event, for: view)
            }
        } label: {
            // Literal text "+" — NOT an SF Symbol, NOT drawn shapes. Rendered
            // through the exact same text layout as the tab labels, so the
            // glyph rides the same baseline and lands at the tabs' OPTICAL
            // position on its own. Geometric centering (a symbol's alignment
            // rect, or crossed shapes) parks the glyph at the line-box center,
            // which sits ~1-2px ABOVE where text visually reads — that was the
            // persistent "+ too high". Text has no such offset to fight.
            Text("+")
                .font(.system(size: Self.plusPointSize, weight: .regular))
                .foregroundStyle(.secondary)
                .frame(width: 22, height: Self.plusFrameH)
                .offset(y: Self.plusInkNudge)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Tabs · ⌃⇥ cycle · split editors")
    }

    // Type metrics for the tab strip's "+". Kept next to the button they
    // correct, and DERIVED rather than eyeballed: the previous constant here
    // was hand-measured at one font size, in the wrong direction, and left the
    // glyph about 1.2pt low.
    private static let plusPointSize: CGFloat = 20
    /// Same box as a tab chip, so the two are centred by the same rule and only
    /// the glyph-ink difference below is left to correct.
    private static let plusFrameH: CGFloat = 24
    private static let tabLabelPointSize: CGFloat = 12
    private static let tabLabelFrameH: CGFloat = 24

    /// Nudge that lands the "+" on the tab labels' optical line.
    ///
    /// Three attempts, and the useful part is what each ruled out. Frame height
    /// is irrelevant: it cancels out of `(H - lineH)/2 + ascender - H/2`, so
    /// matching box sizes changed nothing. Deriving from the label's INK box
    /// (−0.50…−0.70pt) still read low — brackets and descenders stretch that box
    /// below where the eye puts the line. Deriving from the baseline–cap band
    /// (−1.57pt) overshot high.
    ///
    /// Those two bracket it. The optical centre of mixed-case text sits between
    /// its x-height and cap-height midpoints — the usual reference for centring
    /// a symbol against running text — and a "+" is drawn on the maths axis, so
    /// its own ink centre already is its optical centre. That derivation gives
    /// −1.03pt, which read very slightly high; the bracket is now
    /// −0.70 (low) … −1.03 (slightly high), and `opticalTrim` takes the
    /// remainder. It is the one number here that is observed rather than
    /// derived, which is why it is named and isolated instead of folded into
    /// the formula.
    private static let plusInkNudge: CGFloat = {
        /// How far below a centred `Text`'s frame centre its baseline sits.
        func baselineBelowCentre(_ size: CGFloat) -> CGFloat {
            let f = NSFont.systemFont(ofSize: size, weight: .regular)
            return f.ascender - (f.ascender - f.descender) / 2
        }
        let labelFont = NSFont.systemFont(ofSize: tabLabelPointSize, weight: .regular)
        let opticalBand = (labelFont.capHeight / 2 + labelFont.xHeight / 2) / 2
        let labelCentre = baselineBelowCentre(tabLabelPointSize) - opticalBand

        let plusFont = NSFont.systemFont(ofSize: plusPointSize, weight: .regular)
        let plusLine = CTLineCreateWithAttributedString(
            NSAttributedString(string: "+", attributes: [.font: plusFont])
        )
        let plusCentre = baselineBelowCentre(plusPointSize)
            - CTLineGetImageBounds(plusLine, nil).midY

        return labelCentre - plusCentre + opticalTrim
    }()

    /// Residual from eyeballing on a Retina display: the derived value sat a
    /// touch high. Positive moves the glyph down.
    private static let opticalTrim: CGFloat = 0.2

    private func applyNavMode(_ mode: NavMode) {
        switch mode {
        case .project:
            if engine.chrome.scm.open { engine.closeScm() }
            engine.ensureProjectTree()
        case .scm:
            engine.ensureScm()
        case .find:
            if engine.chrome.scm.open { engine.closeScm() }
        case .issues:
            if engine.chrome.scm.open { engine.closeScm() }
            engine.refreshDiagnostics()
        case .breakpoints:
            if engine.chrome.scm.open { engine.closeScm() }
            engine.refreshBreakpoints()
        }
    }

    /// Show/hide the Debug Area. The session-spawning half lives on the engine
    /// (`setDebugArea`) so the View menu shares it; this adds only the panel
    /// animation, which a menu item has no business carrying.
    private func toggleDebugArea() {
        let next = !engine.uiDebugVisible
        // The debug panel is an `if` + `.transition`, so it needs an explicit
        // transaction. The navigator does not — it is an animated `offset`.
        // This must NOT be an ancestor `.animation(value: uiDebugVisible)`:
        // that overrides `navigatorModeStrip`'s own
        // `.animation(NavStrip.settle, value: separated)` and the strip's
        // split-apart stopped animating entirely.
        animatePanels { withAnimation(NavStrip.settle) { engine.setDebugArea(next) } }
        // Only reclaim editor focus when CLOSING. On open the shell owns the
        // keyboard (setDebugArea), and `focused = true` here was part of the
        // long-standing focus bug — the root `.focused` container isn't itself
        // focusable, so SwiftUI handed focus to the first field it could find:
        // the navigator's Filter.
        if !next { focused = true }
    }

    @ViewBuilder
    private var dockedNavigator: some View {
        VStack(spacing: 0) {
            // Title row under mode strip (Xcode density)
            HStack(spacing: 6) {
                Text(navMode.title)
                    .font(.system(size: 11, weight: .semibold, design: .rounded))
                    .foregroundStyle(.primary)
                Spacer()
                if navMode == .project {
                    Button {
                        ProjectTreeView.invalidateCache()
                        engine.ensureProjectTree()
                        focused = true
                    } label: {
                        Image(systemName: "arrow.clockwise")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.secondary)
                            .padding(4)
                            .background(Circle().fill(Color.primary.opacity(0.08)))
                    }
                    .buttonStyle(.plain)
                    .help("Refresh")
                }
                if navMode == .breakpoints {
                    Button {
                        engine.refreshBreakpoints()
                        focused = true
                    } label: {
                        Image(systemName: "arrow.clockwise")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.secondary)
                            .padding(4)
                            .background(Circle().fill(Color.primary.opacity(0.08)))
                    }
                    .buttonStyle(.plain)
                    .help("Refresh breakpoints")
                    Button {
                        engine.toggleBreakpointAtCursor()
                        focused = true
                    } label: {
                        Image(systemName: "plus.circle")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.secondary)
                            .padding(4)
                            .background(Circle().fill(Color.primary.opacity(0.08)))
                    }
                    .buttonStyle(.plain)
                    .help("Toggle breakpoint at cursor · F9")
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)

            Group {
                switch navMode {
                case .project:
                    explorerPanelContent
                case .scm:
                    scmPanelContent
                case .find:
                    findPanelContent
                case .issues:
                    issuesPanelContent
                case .breakpoints:
                    breakpointsPanelContent
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            // Width is shared (`navW`); don’t animate content swap sideways.
            .transaction { $0.animation = nil }
        }
        .background(Color.clear)
    }

    /// Hierarchical Project navigator (Xcode-style).
    private var explorerPanelContent: some View {
        ProjectTreeView(
            index: {
                // The index parses through the engine; hand it over on first use.
                projectIndex.engine = engine
                engine.projectIndex = projectIndex
                return projectIndex
            }(),
            engine: engine,
            rootPath: engine.projectRoot.isEmpty ? engine.chrome.explorer.cwd : engine.projectRoot,
            accent: Color.accentColor,
            fg: Color.primary,
            dim: Color.secondary,
            editorBg: editorBg,
            onOpenFile: { path in
                focused = true
                engine.openPath(path)
                recents = RecentStore.load()
            },
            onRefresh: {
                ProjectTreeView.invalidateCache()
                engine.ensureProjectTree()
                focused = true
            }
        )
    }

    /// The Find navigator shows project grep, or — when a "Find All
    /// References" lookup is active — the LSP references for the symbol,
    /// Xcode-style (references land in the Find navigator).
    private var findPanelContent: some View {
        Group {
            if engine.referencesActive {
                referencesPanelContent
            } else {
                searchPanelContent
            }
        }
    }

    /// Project-wide grep. The core has had `search_project` all along; this is
    /// the surface it never had.
    private var searchPanelContent: some View {
        VStack(spacing: 0) {
            NavigatorSearchField(text: $findQuery) {
                engine.searchProject(findQuery)
                focused = true
            }
            .padding(.horizontal, 10)
            .padding(.bottom, 4)

            // Replace row — uses workspace_search::replace_in_file / replace_all.
            HStack(spacing: 6) {
                TextField("Replace…", text: $findReplace)
                    .textFieldStyle(.plain)
                    .font(.system(size: 12))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(
                        RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                            .fill(Color(nsColor: .controlBackgroundColor).opacity(0.55))
                    )
                Button("One") {
                    if let hit = engine.searchHits.first {
                        _ = engine.replaceSearchHit(hit, query: findQuery, replace: findReplace)
                    }
                    focused = true
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .disabled(findQuery.isEmpty || engine.searchHits.isEmpty)
                .help("Replace the first hit on its line")
                Button("All") {
                    _ = engine.replaceAllSearchHits(query: findQuery, replace: findReplace)
                    focused = true
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .disabled(findQuery.isEmpty || engine.searchHits.isEmpty)
                .help("Replace every match in files from the current hit list")
            }
            .padding(.horizontal, 10)
            .padding(.bottom, 6)

            if engine.searchRunning {
                navigatorPlaceholder("magnifyingglass", "Searching…", nil)
            } else if !engine.searchMessage.isEmpty {
                navigatorPlaceholder(
                    "folder.badge.questionmark", "No project open", engine.searchMessage
                )
            } else if engine.searchHits.isEmpty {
                navigatorPlaceholder(
                    "magnifyingglass",
                    findQuery.isEmpty ? "Search the project" : "No matches",
                    findQuery.isEmpty ? "Type a query and press Return." : nil
                )
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(engine.searchHits) { hit in
                            HStack(spacing: 4) {
                                Button {
                                    engine.openSearchHit(hit)
                                    focused = true
                                } label: {
                                    VStack(alignment: .leading, spacing: 2) {
                                        HStack(spacing: 6) {
                                            Text(hit.name)
                                                .font(.system(size: 12, weight: .medium))
                                                .foregroundStyle(.primary)
                                                .lineLimit(1)
                                            Text(":\(hit.row + 1)")
                                                .font(.system(size: 11, design: .monospaced))
                                                .foregroundStyle(.secondary)
                                            Spacer(minLength: 0)
                                        }
                                        Text(hit.line)
                                            .font(.system(size: 11, design: .monospaced))
                                            .foregroundStyle(.secondary)
                                            .lineLimit(1)
                                    }
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 4)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                Button {
                                    _ = engine.replaceSearchHit(hit, query: findQuery, replace: findReplace)
                                    focused = true
                                } label: {
                                    Image(systemName: "arrow.triangle.2.circlepath")
                                        .font(.system(size: 10, weight: .semibold))
                                        .foregroundStyle(.secondary)
                                        .frame(width: 22, height: 22)
                                        .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .help("Replace this hit")
                                .disabled(findQuery.isEmpty)
                            }
                        }
                        if engine.searchTruncated {
                            Text("Showing the first \(engine.searchHits.count) matches")
                                .font(.system(size: 10))
                                .foregroundStyle(.tertiary)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 6)
                        }
                    }
                    .padding(.horizontal, 4)
                }
            }
        }
    }

    /// References navigator — LSP "Find All References", shown in the Find
    /// slot. Header names the count and offers the way back to search; each
    /// row jumps to the usage (same styling as a search hit).
    private var referencesPanelContent: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: "link")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.secondary)
                Text("References")
                    .font(.system(size: 12, weight: .semibold))
                if engine.referencesReady {
                    Text("\(engine.references.count)")
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                Button {
                    engine.dismissReferences()
                    focused = true
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 12))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .help("Back to search")
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)

            if !engine.referencesReady {
                navigatorPlaceholder("link", "Finding references…", nil)
            } else if engine.references.isEmpty {
                navigatorPlaceholder(
                    "link", "No references",
                    "No references found — or no language server is running for this file."
                )
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(engine.references) { ref in
                            Button {
                                engine.openSearchHit(ref)
                                focused = true
                            } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    HStack(spacing: 6) {
                                        Text(ref.name)
                                            .font(.system(size: 12, weight: .medium))
                                            .foregroundStyle(.primary)
                                            .lineLimit(1)
                                        Text(":\(ref.row + 1)")
                                            .font(.system(size: 11, design: .monospaced))
                                            .foregroundStyle(.secondary)
                                        Spacer(minLength: 0)
                                    }
                                    Text(ref.line)
                                        .font(.system(size: 11, design: .monospaced))
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                }
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                        if engine.referencesTruncated {
                            Text("Showing the first \(engine.references.count) references")
                                .font(.system(size: 10))
                                .foregroundStyle(.tertiary)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 6)
                        }
                    }
                    .padding(.horizontal, 4)
                }
            }
        }
    }

    /// Issue navigator — LSP diagnostics for the open document.
    private var issuesPanelContent: some View {
        Group {
            if engine.diagnostics.isEmpty {
                navigatorPlaceholder(
                    "checkmark.circle", "No issues",
                    "Diagnostics for the open file appear here."
                )
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(engine.diagnostics) { d in
                            Button {
                                engine.gotoLine(d.row + 1)
                                focused = true
                            } label: {
                                HStack(alignment: .top, spacing: 8) {
                                    Image(systemName: diagnosticIcon(d.severity))
                                        .font(.system(size: 11, weight: .semibold))
                                        .foregroundStyle(diagnosticColor(d.severity))
                                        .frame(width: 14, height: 16)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(d.message)
                                            .font(.system(size: 11))
                                            .foregroundStyle(.primary)
                                            .lineLimit(3)
                                            .multilineTextAlignment(.leading)
                                        Text("Line \(d.row + 1)")
                                            .font(.system(size: 10, design: .monospaced))
                                            .foregroundStyle(.secondary)
                                    }
                                    Spacer(minLength: 0)
                                }
                                .padding(.horizontal, 8)
                                .padding(.vertical, 5)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.horizontal, 4)
                }
            }
        }
    }

    private func diagnosticIcon(_ severity: UInt8) -> String {
        switch severity {
        case 0: return "xmark.octagon.fill"
        case 1: return "exclamationmark.triangle.fill"
        case 2: return "info.circle.fill"
        default: return "lightbulb.fill"
        }
    }

    private func diagnosticColor(_ severity: UInt8) -> Color {
        switch severity {
        case 0: return .red
        case 1: return .orange
        case 2: return .blue
        default: return .secondary
        }
    }

    /// Shared empty state for the navigator panels — Xcode keeps the panel and
    /// explains itself rather than showing a blank rectangle.
    @ViewBuilder
    private func navigatorPlaceholder(
        _ symbol: String, _ title: String, _ detail: String?
    ) -> some View {
        VStack(spacing: 8) {
            Spacer(minLength: 20)
            Image(systemName: symbol)
                .font(.system(size: 26, weight: .light))
                .foregroundStyle(.tertiary)
            Text(title)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
            if let detail {
                Text(detail)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 16)
            }
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    /// Breakpoints list (navigator rail — not Find; search stays in the toolbar).
    private var breakpointsPanelContent: some View {
        Group {
            if engine.breakpoints.isEmpty {
                VStack(spacing: 10) {
                    Spacer(minLength: 20)
                    Image(systemName: "bookmark")
                        .font(.system(size: 28, weight: .light))
                        .foregroundStyle(.tertiary)
                    Text("No breakpoints")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.secondary)
                    Text("F9 or + toggles the cursor line.\nClick a row to jump.")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 16)
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(engine.breakpoints) { bp in
                            Button {
                                engine.gotoBreakpoint(bp)
                                focused = true
                            } label: {
                                HStack(alignment: .top, spacing: 8) {
                                    Image(systemName: bp.verified ? "bookmark.fill" : "bookmark")
                                        .font(.system(size: 11, weight: .semibold))
                                        .foregroundStyle(
                                            bp.verified
                                                ? Color.accentColor
                                                : Color.accentColor.opacity(0.55)
                                        )
                                        .frame(width: 14, height: 16)
                                    VStack(alignment: .leading, spacing: 2) {
                                        HStack(spacing: 6) {
                                            Text(bp.name)
                                                .font(.system(size: 12, weight: .medium))
                                                .foregroundStyle(.primary)
                                                .lineLimit(1)
                                            Text(":\(bp.line)")
                                                .font(.system(size: 11, design: .monospaced))
                                                .foregroundStyle(.secondary)
                                        }
                                        if !bp.condition.isEmpty {
                                            Text("if \(bp.condition)")
                                                .font(.system(size: 10, design: .monospaced))
                                                .foregroundStyle(accent.opacity(0.85))
                                                .lineLimit(1)
                                        } else if bp.hasLog {
                                            Text("logpoint")
                                                .font(.system(size: 10))
                                                .foregroundStyle(.secondary)
                                        } else {
                                            Text(bp.path)
                                                .font(.system(size: 10))
                                                .foregroundStyle(.tertiary)
                                                .lineLimit(1)
                                                .truncationMode(.middle)
                                        }
                                    }
                                    Spacer(minLength: 0)
                                    Button {
                                        engine.removeBreakpoint(bp)
                                    } label: {
                                        Image(systemName: "xmark")
                                            .font(.system(size: 9, weight: .bold))
                                            .foregroundStyle(.secondary)
                                            .padding(4)
                                    }
                                    .buttonStyle(.plain)
                                    .help("Remove breakpoint")
                                }
                                .padding(.horizontal, 10)
                                .padding(.vertical, 7)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .background(
                                RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                                    .fill(Color.clear)
                            )
                            .contextMenu {
                                Button("Jump to location") {
                                    engine.gotoBreakpoint(bp)
                                    focused = true
                                }
                                Button("Remove", role: .destructive) {
                                    engine.removeBreakpoint(bp)
                                }
                            }
                        }
                    }
                    .padding(.vertical, 4)
                }
            }
        }
        .onAppear { engine.refreshBreakpoints() }
    }

    private var scmPanelContent: some View {
        VStack(spacing: 0) {
            // Branch header
            HStack(spacing: 6) {
                Image(systemName: "arrow.triangle.branch")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(accent)
                Text(engine.chrome.scm.branch.isEmpty ? "No repository" : engine.chrome.scm.branch)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                Spacer()
                Button {
                    engine.ensureScm()
                    focused = true
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .help("Refresh")
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)

            if !engine.chrome.scm.status.isEmpty {
                Text(engine.chrome.scm.status)
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .padding(.horizontal, 10)
                    .padding(.bottom, 6)
            }

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    scmSection(title: "Staged Changes", rows: engine.chrome.scm.staged, empty: nil)
                    scmSection(
                        title: "Changes",
                        rows: engine.chrome.scm.changes,
                        empty: engine.chrome.scm.staged.isEmpty ? "No local changes" : nil
                    )

                    if !engine.chrome.scm.graph.isEmpty {
                        Text("HISTORY")
                            .font(.system(size: 10, weight: .bold))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 10)
                            .padding(.top, 12)
                            .padding(.bottom, 4)
                        ForEach(engine.chrome.scm.graph) { g in
                            Text(g.line)
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundStyle(g.selected ? accent : fg.opacity(0.88))
                                .lineLimit(1)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 2)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(
                                    RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                                        .fill(g.selected ? accent.opacity(0.12) : Color.clear)
                                )
                        }
                    }
                }
                .padding(.bottom, 8)
            }

            Button {
                engine.toggleGitWorkbench()
                focused = true
            } label: {
                HStack {
                    Image(systemName: "rectangle.split.3x1")
                        .font(.system(size: 11))
                    Text("Open Git Workbench")
                        .font(.system(size: 11, weight: .medium))
                    Spacer()
                }
                .foregroundStyle(accent)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
            .buttonStyle(.plain)
            .overlay(alignment: .top) {
                Rectangle().fill(Color(nsColor: .separatorColor)).frame(height: 1)
            }
        }
    }

    @ViewBuilder
    private func scmSection(title: String, rows: [ScmEntryItem], empty: String?) -> some View {
        if !rows.isEmpty || empty != nil {
            Text(title.uppercased())
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.top, 8)
                .padding(.bottom, 3)
            if rows.isEmpty, let empty {
                Text(empty)
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
            } else {
                ForEach(rows) { row in scmRow(row) }
            }
        }
    }

    /// Debug area — Xcode-style bottom console hosting the shell (no modes, no XLC).
    private var debugArea: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "terminal.fill")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(terminalFocused ? accent : dim)
                Text("Terminal")
                    .font(.system(size: 11, weight: .semibold, design: .rounded))
                    .foregroundStyle(terminalFocused ? fg : dim)
                if terminalFocused {
                    Text("keys → shell · Esc or click editor to leave")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .transition(.opacity)
                } else if engine.chrome.terminal.open {
                    Text("click to type")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .transition(.opacity)
                }
                Spacer()
                // Shell sessions (VS Code-style): chips + new-session.
                if engine.chrome.terminal.open {
                    HStack(spacing: 3) {
                        // Chips scroll once they outgrow the header; the "+"
                        // stays pinned outside the scroller so it never drifts
                        // off-screen as sessions pile up.
                        ScrollViewReader { proxy in
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: 3) {
                                    ForEach(0..<engine.terminalSessionCount, id: \.self) { i in
                                        terminalSessionChip(i).id(i)
                                    }
                                }
                                .padding(.horizontal, 1)
                                .onGeometryChange(for: CGFloat.self) { proxy in
                                    proxy.size.width
                                } action: { w in
                                    terminalChipsWidth = w
                                }
                            }
                            // Hug the chips; scroll only once they pass the cap.
                            .frame(width: min(260, max(1, terminalChipsWidth)))
                            .onChange(of: engine.terminalActiveSession) { _, i in
                                withAnimation(.snappy(duration: 0.2)) {
                                    proxy.scrollTo(i, anchor: .center)
                                }
                            }
                            .onChange(of: engine.terminalSessionCount) { _, n in
                                withAnimation(.snappy(duration: 0.2)) {
                                    proxy.scrollTo(max(0, n - 1), anchor: .trailing)
                                }
                            }
                        }
                        .fixedSize(horizontal: false, vertical: true)

                        HoverIconButton(
                            systemImage: "plus", help: "New Shell",
                            fg: Color.primary, dim: Color.secondary
                        ) {
                            engine.terminalNewSession()
                        }
                    }
                    .padding(.trailing, 4)
                }
                // Same component as the "+" beside it — a bespoke Button with
                // its own padding sized and centred differently, which is why
                // the two never lined up.
                HoverIconButton(
                    systemImage: "xmark", help: "Hide Debug Area",
                    fg: Color.primary, dim: Color.secondary
                ) {
                    withAnimation(.snappy(duration: 0.28)) {
                        engine.uiDebugVisible = false
                    }
                    // Only toggle side terminal — never Ctrl+T while full-panel is open
                    // (that would demote the editor-pane terminal into the debug strip).
                    if engine.chrome.terminal.open && !engine.chrome.terminal.fullPanel {
                        engine.dispatch(
                            code: .char_,
                            ch: UInt32(UnicodeScalar("t").value),
                            mods: .control
                        )
                    }
                    focused = true
                }
            }
            .frame(height: 28)
            .padding(.horizontal, 8)
            // The rule under this header is painted by the CARD band so it
            // spans the full island; drawn here it stopped at the content
            // inset — a hairline cut mid-air at the navigator's edge.
            .animation(.easeOut(duration: 0.15), value: terminalFocused)

            Group {
                if engine.chrome.terminal.open && !engine.chrome.terminal.fullPanel {
                    terminalPanelInner
                } else {
                    Button {
                        openDebugTerminal()
                    } label: {
                        VStack(spacing: 6) {
                            Image(systemName: "terminal")
                                .font(.system(size: 20, weight: .light))
                                .foregroundStyle(.tertiary)
                            Text("Open Terminal · ⌃T")
                                .font(.system(size: 11))
                                .foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        // Outer float chrome applied by parent (clip + material + shadow).
        .background(Color.clear)
    }

    private var terminalFocused: Bool {
        engine.focus == .terminal
    }

    private func terminalSessionChip(_ i: Int) -> some View {
        let active = i == engine.terminalActiveSession
        return Button {
            engine.terminalSelectSession(i)
            focused = true
        } label: {
            HStack(spacing: 4) {
                Image(systemName: "terminal")
                    .font(.system(size: 8, weight: .semibold))
                Text("zsh \(i + 1)")
                    .font(.system(size: 10, weight: active ? .semibold : .regular))
                if active, engine.terminalSessionCount > 1 {
                    Button {
                        engine.terminalCloseSession(i)
                    } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 7, weight: .bold))
                    }
                    .buttonStyle(.plain)
                    .help("Close Shell")
                }
            }
            .foregroundStyle(active ? Color.accentColor : Color.secondary)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(
                Capsule(style: .continuous)
                    .fill(active ? Color.accentColor.opacity(0.14) : Color.primary.opacity(0.05))
            )
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
    }

    private func openDebugTerminal() {
        engine.uiDebugVisible = true
        if !engine.chrome.terminal.open {
            engine.dispatch(code: .char_, ch: UInt32(UnicodeScalar("t").value), mods: .control)
        }
        focused = true
    }

    /// Terminal body (Debug strip): PTY grid sized to the panel, theme background,
    /// click-to-focus keys.
    private var terminalPanelInner: some View {
        GeometryReader { geo in
            TerminalGridView(
                lines: engine.chrome.terminal.lines,
                cursorRow: engine.chrome.terminal.cursorRow,
                cursorCol: engine.chrome.terminal.cursorCol,
                fontSize: 12,
                bg: NSColor(terminalGridBg),
                fg: NSColor(fg),
                onScrollback: { engine.terminalScroll($0) }
            )
            .frame(width: geo.size.width, height: geo.size.height)
            // The SAME fill behind the whole panel. The grid's canvas can land
            // a few points shy of the panel — measured 4pt at the bottom — and
            // the panel's own dock fill is a different tint, so that residue
            // read as a strip the terminal "did not reach". Painting one colour
            // behind both makes the seam impossible rather than arithmetically
            // avoided.
            .background(terminalGridBg)
            .contentShape(Rectangle())
            .onTapGesture {
                engine.focusTerminal(true)
                focused = true
            }
            .onAppear { reportTerminalCells(geo.size) }
            .onChange(of: geo.size) { _, s in reportTerminalCells(s) }
        }
    }

    /// Keep the PTY grid in sync with the visible panel (fixes mis-wrapped output).
    private func reportTerminalCells(_ size: CGSize) {
        let cell = max(6, ("M" as NSString).size(withAttributes: [
            .font: EditorMetrics.monospaced(12, weight: .regular)
        ]).width)
        let lineH: CGFloat = 12 + 4
        let cols = Int((size.width - 20) / cell)
        let rows = Int((size.height - 16) / lineH)
        engine.terminalResize(cols: cols, rows: rows)
    }

    /// Push theme appearance into AppKit windows so title bar / materials follow the theme.
    /// Editor windows drop the system titlebar entirely (custom top bar draws it):
    /// full-size content + hidden title = sidebar material up to the traffic lights.
    ///
    /// MUST stay idempotent — every assignment is guarded by a "changed?"
    /// check. Re-ASSIGNING NSApp.appearance / window styling with the same
    /// values on each app reactivation rebuilt the window's view bridge and
    /// killed NSHostingView's SwiftUI hit-testing (the focus-out/in freeze:
    /// every SwiftUI control dead, AppKit canvas still alive). With the guards
    /// it is safe to call from didBecomeActive, which is what keeps late-born
    /// windows (session restore, second editor window) in sync with the theme.
    private func applyWindowAppearance() {
        let name: NSAppearance.Name = isLightTheme ? .aqua : .darkAqua
        let appearance = NSAppearance(named: name)
        if NSApp.appearance?.name != name {
            NSApp.appearance = appearance
        }
        let bg = NSColor(shellBase)
        // Appearance is safe to push at any window; background and movability
        // are not — restyling an open panel or a popover is both wrong and, for
        // the titlebar work below, fatal. See `isEditorWindow`.
        for window in NSApp.windows where window.title != "Welcome" {
            if window.appearance?.name != name {
                window.appearance = appearance
            }
            guard isEditorWindow(window) else { continue }
            if window.backgroundColor != bg {
                window.backgroundColor = bg
            }
            window.isMovableByWindowBackground = false
            if !window.styleMask.contains(.fullSizeContentView) {
                window.styleMask.insert(.fullSizeContentView)
            }
            if window.titleVisibility != .hidden {
                window.titleVisibility = .hidden
            }
            if !window.titlebarAppearsTransparent {
                window.titlebarAppearsTransparent = true
            }
            if window.titlebarSeparatorStyle != .none {
                window.titlebarSeparatorStyle = .none
            }
            ContentView.styleTrafficLights(window)
        }
    }

    /// One corner radius for every floating panel (sidebar, editor island,
    /// outline card) — matches the window's own corner radius so the nested
    /// corners read as one system. The resize HUD mask uses the same value.
    static let panelCornerRadius: CGFloat = 12

    /// Gap between a panel and the window edge, and equally between the
    /// traffic lights and the panel's corner (the user's spec: the lights sit
    /// off the widget's corner by the same amount in BOTH axes).
    static let panelGap: CGFloat = 6
    static let lightsCornerGap: CGFloat = 11

    /// Height of the top band shared by lights, toggle, tabs and icons.
    /// Derived: lights center = panelGap + lightsCornerGap + 7 (half button).
    static let topBandHeight: CGFloat = 2 * (panelGap + lightsCornerGap + 7)

    /// Fixed status-bar height — it renders as the root's bottom z-layer and
    /// the detail column reserves exactly this much.
    static let statusBarHeight: CGFloat = 24

    /// Traffic lights inside the navigator card, Xcode style.
    ///
    /// There is no official "set the buttons' position" API. The sanctioned
    /// pieces: (1) a titlebar accessory grows the titlebar, which re-centers
    /// the buttons vertically (public API); (2) `standardWindowButton(_:)` is
    /// the public handle to the buttons — nudging their frames is the accepted
    /// pattern, but AppKit re-lays them out on every resize, so this must be
    /// re-asserted from the resize notifications (same finding as VS Code's
    /// Tahoe alignment issue).
    static func styleTrafficLights(_ window: NSWindow) {
        // Reading `titlebarAccessoryViewControllers` on a window without a
        // titlebar THROWS, and a throw during layout is a crash. The callers
        // filter, but this is the one place the AppKit contract lives.
        guard window.styleMask.contains(.titled) else { return }
        let spacerID = NSUserInterfaceItemIdentifier("suisei.lights.spacer")
        if !window.titlebarAccessoryViewControllers.contains(where: { $0.identifier == spacerID }) {
            let vc = NSTitlebarAccessoryViewController()
            vc.identifier = spacerID
            // Zero width — only the height matters (titlebar grows to fit,
            // dropping the lights to the card's comfortable top row).
            vc.view = NSView(frame: NSRect(x: 0, y: 0, width: 0, height: topBandHeight))
            vc.layoutAttribute = .right
            window.addTitlebarAccessoryViewController(vc)
        }
        applyTrafficLightInset(window)
    }

    /// Re-asserted after every resize: horizontal nudge for the lights AND the
    /// interaction clamp for the titlebar container.
    static func applyTrafficLightInset(_ window: NSWindow) {
        guard window.styleMask.contains(.titled) else { return }
        // Equal-axis offset from the card corner: the card sits panelGap off
        // the window, the lights sit lightsCornerGap off the card in BOTH x
        // and y (y comes from topBandHeight's derivation).
        let leading: CGFloat = ContentView.panelGap + ContentView.lightsCornerGap
        let kinds: [NSWindow.ButtonType] = [.closeButton, .miniaturizeButton, .zoomButton]
        let buttons = kinds.compactMap { window.standardWindowButton($0) }
        guard buttons.count == 3 else { return }
        let gap = max(6, buttons[1].frame.minX - buttons[0].frame.minX)
        for (i, b) in buttons.enumerated() {
            let x = leading + CGFloat(i) * gap
            // Vertical: pin the button CENTER to the band's center — the
            // exact line the toggle (and every top-bar control) centers on.
            // Titlebar views are unflipped: y counts from the bottom.
            let superH = b.superview?.frame.height ?? ContentView.topBandHeight
            let y = superH - ContentView.topBandHeight / 2 - b.frame.height / 2
            if abs(b.frame.origin.x - x) > 0.5 || abs(b.frame.origin.y - y) > 0.5 {
                b.setFrameOrigin(NSPoint(x: x, y: y))
            }
        }
        // The grown titlebar container is a raw AppKit sibling ABOVE the
        // content hosting view: left full-width it swallows every click and
        // hover in the whole top band — the toggle, tabs, "+", search,
        // terminal, gear and outline buttons all went dead ("거의 모든 버튼").
        // Clamp it to the lights zone; the SwiftUI top bar owns the rest
        // (WindowDragGesture + double-click zoom cover the titlebar behaviors).
        if let container = buttons[0].superview?.superview {
            let lightsZone: CGFloat = leading + gap * 2 + buttons[2].frame.width + 14
            // Height clamped to the BUTTON ROW, not the grown titlebar. At the
            // accessory's full 48pt the container's own edge showed as a pale
            // step beside the lights, ending exactly at `lightsZone` — the grey
            // slab between the traffic lights and the sidebar toggle. Sized to
            // the buttons, whatever it paints is behind them.
            let winH = window.frame.height
            let top = buttons.map { $0.frame.maxY }.max() ?? container.frame.height
            let bottom = buttons.map { $0.frame.minY }.min() ?? 0
            let h = max(1, top - bottom) + 2
            let y = winH - container.frame.height + bottom - 1
            let target = NSRect(x: 0, y: y, width: lightsZone, height: h)
            if container.frame != target {
                container.frame = target
            }
            // …and it must not PAINT. The titlebar the accessory grows draws
            // its own material, and clamped to the lights zone that material
            // reads as a pale ~85pt slab sitting between the traffic lights and
            // the sidebar toggle — a step with a visible edge at exactly
            // `lightsZone`. The navigator card behind it is the surface; the
            // titlebar has nothing to contribute but the buttons.
            // …and it must not PAINT. The container sits ABOVE our SwiftUI
            // hosting view in `NSThemeFrame`, and it carries a
            // `_NSTitlebarDecorationView` exactly its own size. Clamped to the
            // lights zone that decoration is a pale slab ~91pt wide sitting
            // between the traffic lights and the sidebar toggle, with a hard
            // edge where it stops. (`NSTitlebarBackgroundView` beside it is
            // already hidden — that one is not the culprit, and clearing the
            // container's own layer changes nothing because neither paints it.)
            //
            // Matched by name: the class is private, and a name that stops
            // matching means AppKit reorganised the titlebar, in which case the
            // slab is back and visible rather than silently mis-hidden.
            container.wantsLayer = true
            container.layer?.backgroundColor = NSColor.clear.cgColor
            for v in container.subviews
            where String(describing: type(of: v)).contains("TitlebarDecoration") {
                v.isHidden = true
            }
        }
    }

    /// Call at the start of any panel resize drag so the window never moves with the cursor.
    static func lockWindowBackgroundDrag() {
        for window in NSApp.windows {
            window.isMovableByWindowBackground = false
        }
    }

    /// Suppress engine recomposes during panel drag (same flag as window live-resize).
    private func beginPanelLiveResize() {
        engine.windowLiveResizing = true
        projectIndex.pause()
    }

    private func endPanelLiveResize() {
        engine.windowLiveResizing = false
        projectIndex.resume()
        engine.settleEditorResize()
        persistPanelSizes()
    }

    // MARK: - Liquid glass chrome

    /// Shared frosted Liquid Glass — blur without slab opacity.
    private func glassPanel<Content: View>(
        corner: CGFloat = 18,
        @ViewBuilder content: () -> Content
    ) -> some View {
        let shape = RoundedRectangle(cornerRadius: corner, style: .continuous)
        return content()
            .clipShape(shape)
            .glassEffect(SuiseiGlass.panel(light: isLightTheme), in: shape)
            .shadow(color: Color.black.opacity(isLightTheme ? 0.12 : 0.30), radius: 18, x: 0, y: 8)
    }

    // Legacy floating explorer/SCM cards removed — the navigator owns these
    // surfaces now (flat, full-height sidebar).

    private func scmRow(_ row: ScmEntryItem) -> some View {
        HoverRow(corner: 6) {
            HStack(spacing: 8) {
                Text(row.mark)
                    .font(.system(size: 11, weight: .bold, design: .monospaced))
                    .foregroundStyle(row.staged ? Color(nsColor: .systemGreen).opacity(0.9) : Color(nsColor: .systemOrange).opacity(0.9))
                    .frame(width: 16)
                Text(row.path)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
            .background(
                RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                    .fill(row.selected ? accent.opacity(0.16) : Color.clear)
            )
            .contentShape(Rectangle())
        }
        .padding(.horizontal, 4)
    }

    // MARK: - Git workbench (Ctrl+Shift+G)

    private var gitWorkbenchDocked: some View {
        VStack(spacing: 0) {
            // Top bar: branch + segmented tabs
            HStack(spacing: 10) {
                HStack(spacing: 6) {
                    Image(systemName: "arrow.triangle.branch")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(accent)
                    Text(engine.chrome.gitWb.branch.isEmpty ? "HEAD" : engine.chrome.gitWb.branch)
                        .font(.system(size: 12, weight: .semibold, design: .monospaced))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                }

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 4) {
                        ForEach(engine.chrome.gitWb.chips) { chip in
                            Button {
                                engine.gitWbSetTab(chip.key)
                                focused = true
                            } label: {
                                Text(chip.label)
                                    .font(.system(size: 11, weight: chip.active ? .semibold : .medium))
                                    .foregroundStyle(chip.active ? (isLightTheme ? Color.black.opacity(0.85) : Color.white) : dim)
                                    .padding(.horizontal, 11)
                                    .padding(.vertical, 5)
                                    .background(
                                        Capsule(style: .continuous)
                                            .fill(chip.active ? accent : fg.opacity(0.06))
                                    )
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }

                Spacer(minLength: 4)

                if engine.chrome.gitWb.loading {
                    ProgressView().controlSize(.mini)
                }

                Button {
                    engine.gitWbSetTab(engine.chrome.gitWb.chips.first(where: \.active)?.key ?? 1)
                    focused = true
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 24)
                }
                .buttonStyle(.plain)
                .help("Refresh")

                Button {
                    engine.toggleGitWorkbench()
                    focused = true
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 24)
                }
                .buttonStyle(.plain)
                .help("Close · Esc")
            }
            .padding(.horizontal, 12)
            .frame(height: 40)
            .overlay(alignment: .bottom) {
                Rectangle().fill(Color(nsColor: .separatorColor)).frame(height: 1)
            }

            // Body
            Group {
                if engine.chrome.gitWb.loading
                    && engine.chrome.gitWb.special.count <= 2
                    && engine.chrome.gitWb.colLog.count <= 1
                    && engine.chrome.gitWb.colChanges.count <= 1 {
                    gitEmptyState(
                        icon: "arrow.triangle.branch",
                        title: "Loading…",
                        detail: engine.chrome.gitWb.message.isEmpty
                            ? "Talking to git / gh…"
                            : engine.chrome.gitWb.message
                    )
                } else if engine.chrome.gitWb.docked {
                    GeometryReader { geo in
                        HStack(spacing: 0) {
                            gitRichColumn(
                                title: "Changes",
                                icon: "doc.badge.gearshape",
                                lines: engine.chrome.gitWb.colChanges
                            )
                            .frame(width: max(180, geo.size.width * 0.28))
                            Rectangle().fill(Color(nsColor: .separatorColor)).frame(width: 1)
                            gitRichColumn(
                                title: "History",
                                icon: "clock.arrow.circlepath",
                                lines: engine.chrome.gitWb.colLog
                            )
                            .frame(width: max(220, geo.size.width * 0.48))
                            Rectangle().fill(Color(nsColor: .separatorColor)).frame(width: 1)
                            gitRichColumn(
                                title: "Files",
                                icon: "doc.text",
                                lines: engine.chrome.gitWb.colFiles
                            )
                            .frame(maxWidth: .infinity)
                        }
                    }
                } else {
                    gitSpecialSurface
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            // Footer
            HStack(spacing: 8) {
                if engine.chrome.gitWb.loading {
                    ProgressView().controlSize(.mini)
                }
                Text(engine.chrome.gitWb.message.isEmpty
                    ? "Stage · commit · history · PRs via gh"
                    : engine.chrome.gitWb.message)
                    .font(.system(size: 11))
                    .foregroundStyle(engine.chrome.gitWb.loading ? accent : dim)
                    .lineLimit(1)
                Spacer()
            }
            .padding(.horizontal, 12)
            .frame(height: 26)
            .overlay(alignment: .top) {
                Rectangle().fill(Color(nsColor: .separatorColor)).frame(height: 1)
            }
        }
        .background(editorBg)
        .contentShape(Rectangle())
    }

    /// PRs / Issues / Branches / Auth — card list, not bare monospaced dump.
    private var gitSpecialSurface: some View {
        let lines = engine.chrome.gitWb.special
        let title = engine.chrome.gitWb.chips.first(where: \.active)?.label ?? "Git"
        let isEmptyList = lines.count <= 3 && lines.contains(where: {
            $0.localizedCaseInsensitiveContains("no open")
                || $0.localizedCaseInsensitiveContains("not logged")
                || $0.localizedCaseInsensitiveContains("missing")
                || $0.localizedCaseInsensitiveContains("empty")
        })

        return VStack(alignment: .leading, spacing: 0) {
            // Section header
            HStack {
                Text(title)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.primary)
                Spacer()
                Text("\(max(0, lines.filter { $0.hasPrefix("›") || $0.hasPrefix(" ") || $0.contains("#") }.count)) items")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .overlay(alignment: .bottom) {
                Rectangle().fill(Color(nsColor: .separatorColor)).frame(height: 1)
            }

            if lines.isEmpty {
                gitEmptyState(icon: "tray", title: "Nothing here", detail: "Refresh or switch tab")
            } else if isEmptyList {
                gitEmptyState(
                    icon: emptyIconForGitTab(title),
                    title: lines.first(where: { $0.localizedCaseInsensitiveContains("no open") || $0.localizedCaseInsensitiveContains("not found") }) ?? "No items",
                    detail: lines.dropFirst().prefix(3).joined(separator: "\n")
                )
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                            gitSpecialRow(line)
                        }
                    }
                    .padding(12)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func emptyIconForGitTab(_ title: String) -> String {
        let t = title.lowercased()
        if t.contains("pr") { return "arrow.triangle.pull" }
        if t.contains("issue") { return "exclamationmark.circle" }
        if t.contains("branch") { return "arrow.triangle.branch" }
        if t.contains("auth") { return "person.crop.circle.badge.key" }
        if t.contains("stash") { return "tray.full" }
        if t.contains("diff") { return "doc.plaintext" }
        return "tray"
    }

    @ViewBuilder
    private func gitSpecialRow(_ line: String) -> some View {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        let selected = line.contains("›") || line.hasPrefix("›")
        let isMeta = trimmed.hasPrefix("Pull Requests")
            || trimmed.hasPrefix("Issues")
            || trimmed.hasPrefix("GitHub")
            || trimmed.hasPrefix("Install")
            || trimmed.hasPrefix("Then:")
            || trimmed.hasPrefix("Auth tab")
            || trimmed.hasPrefix("Error:")
            || trimmed.hasPrefix("User:")
            || trimmed.hasPrefix("Not logged")
            || trimmed.hasPrefix("diff ·")
            || trimmed.hasPrefix("Fetching")

        if isMeta {
            Text(trimmed)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 8)
                .padding(.vertical, 2)
        } else {
            HoverRow(corner: 8) {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: gitRowIcon(trimmed))
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(selected ? accent : dim)
                        .frame(width: 18)
                    Text(trimmed.replacingOccurrences(of: "›", with: "").trimmingCharacters(in: .whitespaces))
                        .font(.system(size: 12, design: .default))
                        .foregroundStyle(selected ? fg : fg.opacity(0.92))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .lineLimit(2)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
                .background(
                    RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                        .fill(selected ? accent.opacity(0.14) : fg.opacity(0.04))
                )
                .overlay(
                    RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                        .stroke(Color(nsColor: .separatorColor).opacity(0.6), lineWidth: 1)
                )
                .contentShape(RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
            }
        }
    }

    private func gitRowIcon(_ line: String) -> String {
        if line.contains("#") { return "number" }
        if line.contains("*") { return "star.fill" }
        if line.hasPrefix("M ") || line.contains(" M ") { return "pencil" }
        if line.hasPrefix("A ") { return "plus.circle" }
        if line.hasPrefix("D ") { return "minus.circle" }
        if line.hasPrefix("?") { return "questionmark.circle" }
        return "circle.fill"
    }

    private func gitRichColumn(title: String, icon: String, lines: [String]) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: icon)
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(accent)
                Text(title)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.primary)
                Spacer()
                Text("\(max(0, lines.filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }.count))")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Color.primary.opacity(0.06)))
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .overlay(alignment: .bottom) {
                Rectangle().fill(Color(nsColor: .separatorColor)).frame(height: 1)
            }

            if lines.isEmpty || (lines.count == 1 && lines[0].localizedCaseInsensitiveContains("clean")) {
                gitEmptyState(
                    icon: icon,
                    title: title == "Changes" ? "Working tree clean" : "No items",
                    detail: title == "History" ? "Commits appear after load" : " "
                )
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 3) {
                        ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                            let cleaned = line
                                .replacingOccurrences(of: "▾ ", with: "")
                                .replacingOccurrences(of: "›", with: "")
                                .trimmingCharacters(in: .whitespaces)
                            let selected = line.contains("›")
                            let isHeader = cleaned.hasPrefix("Changes")
                                || cleaned.hasPrefix("Staged")
                                || cleaned.hasPrefix("Local")
                                || cleaned.hasPrefix("Log")
                                || cleaned.hasPrefix("Files")
                                || cleaned.hasPrefix("──")
                            if isHeader {
                                Text(cleaned)
                                    .font(.system(size: 10, weight: .bold))
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 12)
                                    .padding(.top, 8)
                                    .padding(.bottom, 2)
                            } else {
                                Button {
                                    NSPasteboard.general.clearContents()
                                    NSPasteboard.general.setString(cleaned, forType: .string)
                                    focused = true
                                } label: {
                                    HoverRow(corner: 6) {
                                        HStack(spacing: 8) {
                                            Circle()
                                                .fill(selected ? accent : gitStatusDotColor(cleaned))
                                                .frame(width: 5, height: 5)
                                            Text(cleaned)
                                                .font(.system(size: 11, design: .monospaced))
                                                .foregroundStyle(selected ? fg : fg.opacity(0.88))
                                                .lineLimit(1)
                                                .truncationMode(.middle)
                                            Spacer(minLength: 0)
                                        }
                                        .padding(.horizontal, 10)
                                        .padding(.vertical, 5)
                                        .background(
                                            RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                                                .fill(selected ? accent.opacity(0.12) : Color.clear)
                                        )
                                        .contentShape(Rectangle())
                                    }
                                }
                                .buttonStyle(.plain)
                                .help("Click to copy")
                                .padding(.horizontal, 6)
                            }
                        }
                    }
                    .padding(.vertical, 6)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(editorBg)
    }

    private func gitStatusDotColor(_ line: String) -> Color {
        let t = line.trimmingCharacters(in: .whitespaces)
        if t.hasPrefix("M") || t.contains(" modified") { return Color(nsColor: .systemOrange).opacity(0.85) }
        if t.hasPrefix("A") || t.contains(" new file") { return Color(nsColor: .systemGreen).opacity(0.85) }
        if t.hasPrefix("D") || t.contains(" deleted") { return Color(nsColor: .systemRed).opacity(0.85) }
        if t.hasPrefix("?") || t.hasPrefix("U") { return dim.opacity(0.55) }
        return dim.opacity(0.35)
    }

    private func gitEmptyState(icon: String, title: String, detail: String) -> some View {
        VStack(spacing: 12) {
            Spacer(minLength: 20)
            Image(systemName: icon)
                .font(.system(size: 28, weight: .light))
                .foregroundStyle(.secondary)
            Text(title)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(.primary)
                .multilineTextAlignment(.center)
            Text(detail)
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 24)
            Spacer(minLength: 20)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Find bar (⌘F — Xcode-style strip at the top of the editor)

    private var findBar: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.secondary)
            HStack(spacing: 2) {
                Text(engine.chrome.search.input.isEmpty ? "Find" : engine.chrome.search.input)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(engine.chrome.search.input.isEmpty ? dim.opacity(0.7) : fg)
                    .lineLimit(1)
                Rectangle()
                    .fill(accent)
                    .frame(width: 1.5, height: 13)
                    .opacity(findCaretBlink ? 1 : 0.15)
                    .animation(
                        .easeInOut(duration: 0.45).repeatForever(autoreverses: true),
                        value: findCaretBlink
                    )
                    .onAppear { findCaretBlink = true }
                    .onDisappear { findCaretBlink = false }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .frame(minWidth: 160, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                    .fill(fg.opacity(isLightTheme ? 0.05 : 0.08))
            )
            .overlay(
                RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                    .strokeBorder(accent.opacity(0.45), lineWidth: 1)
            )

            Text(
                engine.chrome.search.matchCount > 0
                    ? "\(engine.chrome.search.matchIndex + 1) of \(engine.chrome.search.matchCount)"
                    : (engine.chrome.search.input.isEmpty ? " " : "Not found")
            )
            .font(.system(size: 10, design: .rounded))
            .foregroundStyle(engine.chrome.search.matchCount > 0 ? Color.secondary : Color(nsColor: .systemOrange))
            .frame(minWidth: 64, alignment: .leading)

            HoverIconButton(systemImage: "chevron.left", help: "Previous · ⇧⌘G", fg: Color.primary, dim: Color.secondary) {
                engine.findStep(forward: false)
            }
            HoverIconButton(systemImage: "chevron.right", help: "Next · ⌘G", fg: Color.primary, dim: Color.secondary) {
                engine.findStep(forward: true)
            }
            HoverIconButton(systemImage: "xmark", help: "Done · Esc", fg: Color.primary, dim: Color.secondary) {
                engine.dispatch(code: .esc)
                focused = true
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: Radius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Radius.panel, style: .continuous)
                .strokeBorder(fg.opacity(0.10), lineWidth: 1)
        )
        .shadow(color: .black.opacity(isLightTheme ? 0.10 : 0.35), radius: 12, y: 4)
        .padding(.top, 8)
        .padding(.trailing, 12)
    }

    // MARK: - Palette overlay (Ctrl/Cmd+P)

    private var paletteOverlay: some View {
        ZStack {
            GlassScrim(lightChrome: isLightTheme)
                .ignoresSafeArea()
                .onTapGesture {
                    engine.dispatch(code: .esc)
                    focused = true
                }

            VStack {
                glassPanel(corner: 20) {
                    VStack(spacing: 0) {
                        HStack {
                            Text(engine.chrome.palette.kind.isEmpty ? "Palette" : engine.chrome.palette.kind)
                                .font(.system(size: 12, weight: .semibold, design: .rounded))
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text("Esc")
                                .font(.system(size: 10, weight: .medium, design: .rounded))
                                .foregroundStyle(.tertiary)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 3)
                                .background(Capsule().fill(Color.white.opacity(0.08)))
                        }
                        .padding(.horizontal, 16)
                        .padding(.top, 14)

                        HStack(spacing: 8) {
                            Image(systemName: "magnifyingglass")
                                .foregroundStyle(accent.opacity(0.9))
                            Text(engine.chrome.palette.query.isEmpty ? "type to filter…" : engine.chrome.palette.query)
                                .foregroundStyle(engine.chrome.palette.query.isEmpty ? dim : fg)
                            Spacer()
                        }
                        .font(.system(size: 15, design: .rounded))
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)

                        Rectangle()
                            .fill(Color.white.opacity(0.08))
                            .frame(height: 1)

                        ScrollView {
                            LazyVStack(alignment: .leading, spacing: 2) {
                                ForEach(engine.chrome.palette.items) { item in
                                    HoverRow(corner: 12) {
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(item.label)
                                                .font(.system(size: 13, design: .rounded))
                                                .foregroundStyle(.primary)
                                                .lineLimit(1)
                                            if !item.detail.isEmpty {
                                                Text(item.detail)
                                                    .font(.system(size: 10, design: .monospaced))
                                                    .foregroundStyle(.secondary)
                                                    .lineLimit(1)
                                                    .truncationMode(.middle)
                                            }
                                        }
                                        .padding(.horizontal, 12)
                                        .padding(.vertical, 9)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                        .background(
                                            RoundedRectangle(cornerRadius: Radius.panel, style: .continuous)
                                                .fill(item.selected ? Color.accentColor.opacity(0.22) : Color.clear)
                                        )
                                        .contentShape(RoundedRectangle(cornerRadius: Radius.panel, style: .continuous))
                                    }
                                    // GUI contract: one click opens (VS Code palette).
                                    .onTapGesture {
                                        engine.paletteActivate(item.id)
                                        focused = true
                                    }
                                }
                            }
                            .padding(8)
                        }
                        .frame(maxHeight: 340)
                    }
                }
                .frame(width: 540)
                .padding(.top, 72)
                Spacer()
            }
        }
    }

    // Which-key overlay removed — Suisei has no leader/prefix chords (GUI editor).

    // MARK: - Completions

    /// Xcode-style completion list: a tight list pinned UNDER the caret. The
    /// old version was a 340pt titled panel parked in the window's bottom-right
    /// corner, nowhere near the text being completed.
    private var completionsOverlay: some View {
        let items = engine.chrome.completions.items
        // Follow the editor's zoom (⌘+ / ⌘−) — a fixed-size list next to
        // zoomed-in code reads as a different app.
        let codeSize = EditorMetrics.fontSize
        let scale = codeSize / EditorMetrics.defaultFontSize
        let rowH = (codeSize + 7).rounded()
        let panelW = (300 * scale).rounded()
        let listH = min(CGFloat(max(1, items.count)) * rowH + 6 * scale, 260 * scale)
        // Concentric corners: the rows are capsules inset from the panel edge,
        // so the panel must be rowRadius + inset or the inside reads rounder
        // than the outside. Derived, so it stays right at every zoom level.
        let rowInset = 4 * scale
        let panelR = rowH / 2 + rowInset

        return GeometryReader { geo in
            // Measure the caret RELATIVE to this overlay rather than assuming
            // it shares the window's origin — guessing that offset put the
            // list a couple of lines below the caret.
            let origin = geo.frame(in: .global).origin
            let caretX = engine.caretFrameInWindow.minX - origin.x
            let caretTop = engine.caretFrameInWindow.minY - origin.y
            let caretBottom = engine.caretFrameInWindow.maxY - origin.y
            // Prefer directly below the caret; flip above when the line is near
            // the bottom, exactly like Xcode.
            let below = caretBottom + 2
            let fitsBelow = below + listH <= geo.size.height - 8
            let y = fitsBelow ? below : max(8, caretTop - listH - 2)
            let x = min(max(8, caretX - 2), max(8, geo.size.width - panelW - 8))

            ZStack(alignment: .topLeading) {
                Color.clear
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(items.enumerated()), id: \.offset) { i, item in
                                let picked = i == engine.chrome.completions.selected
                                let typed = engine.chrome.completions.prefix
                                HStack(spacing: 6) {
                                    // Bold the part already typed, like Xcode,
                                    // so the eye tracks what it is matching on.
                                    Group {
                                        if !typed.isEmpty, item.label.hasPrefix(typed) {
                                            Text(typed).fontWeight(.bold)
                                                + Text(item.label.dropFirst(typed.count))
                                        } else {
                                            Text(item.label)
                                        }
                                    }
                                    .font(.system(size: codeSize - 2, design: .monospaced))
                                    .foregroundStyle(picked ? Color.white : Color.primary)
                                    .lineLimit(1)

                                    Spacer(minLength: 12)
                                    if !item.detail.isEmpty {
                                        Text(item.detail)
                                            .font(.system(size: codeSize - 3))
                                            .foregroundStyle(
                                                picked ? Color.white.opacity(0.75) : Color.secondary
                                            )
                                            .lineLimit(1)
                                    }
                                }
                                .padding(.horizontal, 10 * scale)
                                .frame(height: rowH)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(
                                    Capsule(style: .continuous)
                                        .fill(picked ? accent : Color.clear)
                                )
                                .padding(.horizontal, rowInset)
                                .id(i)
                            }
                        }
                        .padding(.vertical, 4)
                    }
                    .frame(width: panelW, height: listH)
                    .background(
                        RoundedRectangle(cornerRadius: panelR, style: .continuous)
                            .fill(Color(nsColor: .windowBackgroundColor))
                    )
                    .clipShape(RoundedRectangle(cornerRadius: panelR, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: panelR, style: .continuous)
                            .strokeBorder(Color.primary.opacity(0.12), lineWidth: 0.6)
                    )
                    .shadow(color: .black.opacity(isLightTheme ? 0.16 : 0.45), radius: 10, y: 4)
                    .onChange(of: engine.chrome.completions.selected) { _, sel in
                        proxy.scrollTo(sel, anchor: .center)
                    }
                }
                .offset(x: x, y: y)
            }
        }
        .allowsHitTesting(false)
        .transition(.opacity)
    }

    // Floating terminal/XLC glass cards removed — Terminal lives in the Debug
    // area (or a split pane via ⌃⇧T); XLC is gone from the GUI entirely.

    // MARK: - Jump bar (Xcode path bar)

    private var jumpBar: some View {
        HStack(spacing: 0) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 2) {
                    let segs = jumpBarSegments
                    if segs.isEmpty {
                        Text(engine.chrome.filename.isEmpty ? "No Editor" : engine.chrome.filename)
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 8)
                    }
                    ForEach(Array(segs.enumerated()), id: \.offset) { idx, seg in
                        if idx > 0 {
                            Image(systemName: "chevron.forward")
                                .font(.system(size: 8, weight: .semibold))
                                .foregroundStyle(.tertiary)
                                .padding(.horizontal, 1)
                        }
                        JumpBarSegmentButton(
                            title: seg.title,
                            systemImage: seg.systemImage,
                            isFile: seg.isFile,
                            isLast: idx == segs.count - 1,
                            accent: accent,
                            fg: fg,
                            dim: dim
                        ) {
                            handleJumpSegment(seg)
                            focused = true
                        }
                        .help(seg.path)
                    }

                    // Symbols menu (current file outline)
                    if !engine.chrome.outline.isEmpty {
                        Image(systemName: "chevron.forward")
                            .font(.system(size: 8, weight: .semibold))
                            .foregroundStyle(.tertiary)
                            .padding(.horizontal, 1)
                        Menu {
                            ForEach(engine.chrome.outline) { item in
                                Button("\(item.name)  · L\(item.row)") {
                                    engine.gotoLine(item.row)
                                    focused = true
                                }
                            }
                        } label: {
                            HStack(spacing: 3) {
                                Image(systemName: "list.bullet.indent")
                                    .font(.system(size: 10))
                                Text(engine.chrome.outline.first?.name ?? "Symbols")
                                    .font(.system(size: 11))
                                    .lineLimit(1)
                            }
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 3)
                            .background(
                                RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                                    .fill(Color.primary.opacity(0.05))
                            )
                        }
                        .menuStyle(.borderlessButton)
                        .help("Jump to symbol")
                    }
                }
                .padding(.horizontal, 8)
            }
            Spacer(minLength: 0)
        }
        .frame(height: 26)
        .background(editorBg)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Color(nsColor: .separatorColor)).frame(height: 1)
        }
    }

    private struct JumpSeg: Identifiable {
        var id: String { path + title }
        var title: String
        var path: String
        var isFile: Bool
        var systemImage: String
    }

    private var jumpBarSegments: [JumpSeg] {
        let file = engine.chrome.filename
        guard !file.isEmpty, file != "[No Name]" else {
            let root = engine.projectRoot
            if root.isEmpty { return [] }
            return [JumpSeg(
                title: (root as NSString).lastPathComponent,
                path: root,
                isFile: false,
                systemImage: "folder.fill"
            )]
        }
        var stack: [String] = []
        for c in URL(fileURLWithPath: file).pathComponents where c != "/" {
            stack.append(c)
        }
        guard !stack.isEmpty else { return [] }
        let start = max(0, stack.count - 5)
        var parts: [JumpSeg] = []
        for i in start..<stack.count {
            let sub = stack[0...i].joined(separator: "/")
            let path = file.hasPrefix("/") ? "/" + sub : sub
            let isLast = i == stack.count - 1
            parts.append(JumpSeg(
                title: stack[i],
                path: path,
                isFile: isLast,
                systemImage: isLast ? fileJumpIcon(stack[i]) : "folder.fill"
            ))
        }
        return parts
    }

    private func fileJumpIcon(_ name: String) -> String {
        let ext = (name as NSString).pathExtension.lowercased()
        switch ext {
        case "md": return "doc.richtext"
        case "rs", "swift", "ts", "js": return "doc.text"
        default: return "doc"
        }
    }

    private func handleJumpSegment(_ seg: JumpSeg) {
        if seg.isFile {
            // Current file — no-op (or re-open)
            return
        }
        // Open folder in project tree / reveal
        var isDir: ObjCBool = false
        if FileManager.default.fileExists(atPath: seg.path, isDirectory: &isDir), isDir.boolValue {
            engine.setProjectRoot(seg.path)
            navMode = .project
            engine.uiNavVisible = true
            engine.ensureProjectTree()
        }
    }

    // MARK: - Inspector (outline)

    /// Outline — floating rounded card on the shell base (the design that read
    /// best next to the editor), resizable via its leading strip.
    private var inspectorPanel: some View {
        VStack(spacing: 0) {
            // No title row. The selected tab already says which inspector this
            // is, and Xcode's inspector has none — in a rail this narrow the
            // duplication costs 28pt before any content starts.
            inspectorModeStrip

            switch inspectorMode {
            case .file: fileInspectorContent
            case .quickHelp: quickHelpContent
            case .outline: outlineContent
            }
        }
        // Shares ONE surface with the editor, split by a hairline — which is
        // how Xcode draws it, and the only way to avoid a seam. Made flat and
        // full-bleed on its own it started 8pt above the editor card, because
        // the card carries `panelGap` vertically and this did not: a visible
        // step right where the two are supposed to meet. So it takes the same
        // insets and squares only the edge that butts against the editor,
        // whose trailing corners are squared to match (`editorStageShape`).
        // The navigator stays a separate floating widget on purpose; it is a
        // place you go, while this is a wall of facts about the editor and
        // belongs to it.
        // No border, no separator, no background of its own. The column
        // behind paints the shell tone, and the tonal step against the
        // editor's surface is the entire boundary — Xcode draws no line here,
        // and the 1pt hairline this used to carry was the very line the user
        // kept seeing.
    }

    /// Right rail tabs. Same capsule language as the navigator strip so the two
    /// sides read as one family — but no split and no detached toggle: nothing
    /// here is a panel switch, they are all inspectors on the same selection.
    private var inspectorModeStrip: some View {
        GeometryReader { geo in
            let slot = geo.size.width / CGFloat(InspectorMode.allCases.count)
            HStack(spacing: 0) {
                ForEach(Array(InspectorMode.allCases.enumerated()), id: \.element) { index, mode in
                    Button {
                        inspectorMode = mode
                        if mode == .quickHelp { engine.refreshHover() }
                        focused = true
                    } label: {
                        Image(systemName: mode.systemImage)
                            .font(.system(size: 11.5, weight: .medium))
                            .foregroundStyle(
                                inspectorMode == mode
                                    ? Color.white : Color.secondary
                            )
                            .frame(width: slot, height: 22)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help(mode.title)
                    // Hairline between segments, hidden next to the selected
                    // one — a rule running into the filled capsule reads as a
                    // scratch. Drawn as an overlay so it cannot take layout
                    // width and knock the slots out of step with the pill.
                    .overlay(alignment: .leading) {
                        if index > 0 {
                            Rectangle()
                                .fill(Color(nsColor: .separatorColor))
                                .frame(width: 1, height: 12)
                                .opacity(
                                    inspectorMode == mode
                                        || inspectorMode == InspectorMode.allCases[index - 1]
                                        ? 0 : 1
                                )
                        }
                    }
                }
            }
            // Same travelling indicator as the navigator rail, so switching
            // tabs on either side feels like the same control — liquid state
            // included, structured identically (solid behind, glass above).
            .background {
                TravellingPill(
                    progress: inspectorProgress,
                    from: CGFloat(inspectorFrom) * slot,
                    to: CGFloat(inspectorTo) * slot,
                    width: slot
                )
                .fill(Color.accentColor)
            }
            .background {
                // Behind the glyph row like the navigator's — but as a plain
                // sibling ZStack was not available here, `.background` with
                // the CONTAINER is acceptable: the anti-pattern is bare
                // glassEffect shapes in backgrounds, and the container is what
                // restores correct sampling.
            }
            .animation(.easeOut(duration: 0.15), value: inspectorLiquid)
        }
        .frame(height: 22)
        .padding(2)
        // Fill AND border in one background, the way the navigator rail does
        // it. As an `.overlay` the border drew ON TOP of the travelling pill,
        // and being a translucent `separatorColor` the pill showed straight
        // through it whenever it slid under the capsule's edge — visible only
        // mid-animation, which is why it survived so long.
        .background(
            Capsule(style: .continuous)
                .fill(Color.primary.opacity(isLightTheme ? 0.035 : 0.06))
                .overlay(
                    Capsule(style: .continuous)
                        .strokeBorder(
                            Color(nsColor: .separatorColor).opacity(0.6), lineWidth: 1
                        )
                )
        )
        .padding(.horizontal, 10)
        .padding(.top, 8)
        .padding(.bottom, 2)
    }

    private var outlineContent: some View {
        Group {
            if engine.chrome.outline.isEmpty {
                VStack(spacing: 6) {
                    Image(systemName: "list.bullet.indent")
                        .font(.system(size: 18))
                        .foregroundStyle(.tertiary)
                    Text("No symbols")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(engine.chrome.outline) { item in
                            Button {
                                engine.gotoLine(item.row)
                                focused = true
                            } label: {
                                HStack(spacing: 6) {
                                    Image(systemName: outlineIcon(item.kind))
                                        .font(.system(size: 10))
                                        .foregroundStyle(outlineColor(item.kind))
                                        .frame(width: 14)
                                    Text(item.name)
                                        .font(.system(size: 11))
                                        .foregroundStyle(.primary)
                                        .lineLimit(1)
                                    Spacer(minLength: 0)
                                    Text("\(item.row)")
                                        .font(.system(size: 9, design: .monospaced))
                                        .foregroundStyle(.tertiary)
                                }
                                .padding(.leading, 8 + CGFloat(item.depth) * 12)
                                .padding(.trailing, 8)
                                .padding(.vertical, 4)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(
                                    RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                                        .fill(
                                            engine.chrome.cursorRow == item.row
                                                ? Color.primary.opacity(isLightTheme ? 0.10 : 0.16)
                                                : Color.clear
                                        )
                                )
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.vertical, 4)
                    .padding(.horizontal, 4)
                }
            }
        }
    }

    /// File inspector. No core work at all — everything here is already on this
    /// side of the FFI, which is why it is the cheapest thing in the right rail.
    private var fileInspectorContent: some View {
        let path = engine.chrome.filename
        return Group {
            if path.isEmpty || path == "[No Name]" {
                navigatorPlaceholder("doc", "No file", "Open a file to inspect it.")
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        inspectorSection("Identity and Type")
                        inspectorRow("Name", (path as NSString).lastPathComponent)
                        inspectorRow("Type", fileKind(path))
                        inspectorRow("Location", (path as NSString).deletingLastPathComponent)
                        inspectorRow("Full Path", path, mono: true)

                        inspectorSection("Document")
                        inspectorRow("Lines", "\(engine.chrome.lineCount)")
                        inspectorRow("Size", fileSize(path) ?? "")

                        inspectorSection("Project Index")
                        inspectorRow(
                            "Parsed", projectIndex.isIndexed(path) ? "Yes" : "Not yet"
                        )
                    }
                    .padding(.bottom, 10)
                }
            }
        }
    }

    /// Quick Help — the LSP's own words about the symbol under the caret.
    private var quickHelpContent: some View {
        Group {
            if engine.hoverText.isEmpty {
                navigatorPlaceholder(
                    "questionmark.circle", "No description",
                    "Put the caret on a symbol, then reopen this tab."
                )
            } else {
                ScrollView {
                    Text(engine.hoverText)
                        .font(.system(size: 11))
                        .foregroundStyle(.primary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                }
            }
        }
    }

    /// Section heading, sitting at the panel's own margin rather than lining up
    /// with the value column — that offset is what makes Xcode's groups read as
    /// groups instead of as more rows.
    private func inspectorSection(_ title: String) -> some View {
        Text(title)
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(.primary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10)
            .padding(.top, 14)
            .padding(.bottom, 5)
    }

    /// One form row. The label is RIGHT-aligned in a fixed column so every
    /// value starts on the same x — that shared edge is the whole impression of
    /// an Xcode inspector, and left-aligning the labels loses it: the panel
    /// stops reading as a form and becomes a list. Read-only values still wear
    /// a field's background for the same reason.
    private func inspectorRow(
        _ label: String, _ value: String, mono: Bool = false
    ) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label)
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
                .frame(width: 62, alignment: .trailing)
                .lineLimit(1)
            Text(value.isEmpty ? "—" : value)
                .font(.system(size: 11, design: mono ? .monospaced : .default))
                .foregroundStyle(value.isEmpty ? .secondary : .primary)
                .textSelection(.enabled)
                .lineLimit(mono ? 3 : 1)
                .truncationMode(mono ? .middle : .tail)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(
                    RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                        .fill(Color.primary.opacity(isLightTheme ? 0.05 : 0.08))
                )
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 2)
    }

    private func fileKind(_ path: String) -> String {
        let ext = (path as NSString).pathExtension
        return ext.isEmpty ? "Plain text" : ext.uppercased() + " source"
    }

    private func fileSize(_ path: String) -> String? {
        guard let n = try? FileManager.default
            .attributesOfItem(atPath: path)[.size] as? Int64 else { return nil }
        return ByteCountFormatter.string(fromByteCount: n, countStyle: .file)
    }

    private func outlineIcon(_ kind: UInt8) -> String {
        switch kind {
        case 0: return "number"
        case 1: return "f.cursive"
        case 2: return "curlybraces"
        default: return "doc.text"
        }
    }

    private func outlineColor(_ kind: UInt8) -> Color {
        switch kind {
        case 0: return accent
        case 1: return Color(red: 0.45, green: 0.75, blue: 0.95)
        case 2: return Color(red: 0.85, green: 0.65, blue: 0.35)
        default: return dim
        }
    }

    // MARK: - Editor (Core scroll window — base layer only)

    /// Xcode 26 editor island: rounded card on the shell base hosting jump bar,
    /// editor surface, and the bottom Debug console; find bar floats top-trailing.
    private var editorIsolatedStage: some View {
        VStack(spacing: 0) {
            // Jump bar lives above the editor (Cursor/Xcode chrome).
            // Full-panel terminal is pane-local — keep jump bar when unsplit only if no term/preview.
            if !engine.editorSplit.isSplit, !engine.chrome.gitWb.open,
               !engine.preview.open,
               !(engine.chrome.terminal.open && engine.chrome.terminal.fullPanel)
            {
                jumpBar
            }
            Group {
                if engine.chrome.gitWb.open {
                    gitWorkbenchDocked
                } else if engine.preview.open {
                    previewPanel
                } else {
                    // Full-panel terminal paints inside the bound split pane (see editorColumn).
                    mainEditor
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            // Minimap lives at the island level (stable identity — no re-mount
            // with editor pane rebuilds; that was the layer flicker).
            .overlay(alignment: .trailing) {
                if minimapEnabled, !engine.editorSplit.isSplit,
                   !engine.chrome.gitWb.open, !engine.preview.open,
                   !(engine.chrome.terminal.open && engine.chrome.terminal.fullPanel)
                {
                    MinimapStrip(
                        engine: engine,
                        accent: accent,
                        fg: fg,
                        bg: editorBg,
                        isLight: isLightTheme
                    )
                    .frame(width: 62)
                    .transition(.opacity)
                }
            }
            // Top fade only once content actually slides under (the always-on
            // material veil looked like a blur glued to the jump bar).
            .overlay(alignment: .top) {
                if engine.chrome.scroll > 0 {
                    EdgeFade(color: editorBg, top: true)
                        .frame(height: 12)
                        .transition(.opacity)
                }
            }
            .overlay(alignment: .bottom) {
                EdgeFade(color: editorBg, top: false).frame(height: 12)
            }
            .animation(.easeOut(duration: 0.15), value: engine.chrome.scroll > 0)
            .overlay(alignment: .topTrailing) {
                if engine.chrome.search.open {
                    findBar
                        .transition(.move(edge: .top).combined(with: .opacity))
                }
            }
            .animation(.snappy(duration: 0.22), value: engine.chrome.search.open)

            // ── Debug area (Xcode bottom console) — rounded inset card.
            if engine.uiDebugVisible {
                VStack(spacing: 0) {
                    PanelResizeGrip(
                        size: $debugAreaH, minS: 120, maxS: 480,
                        axis: .vertical, invert: true,
                        fg: fg,
                        onBegan: beginPanelLiveResize,
                        onEnded: endPanelLiveResize
                    )
                    // Full editor width, ROUNDED top corners + upward shadow
                    // (a flat slab read as slapped-on): the card floats at the
                    // island's bottom, island clip rounds the bottom edge.
                    // Bare content: the tint band, fillets and hairline are
                    // painted by the CARD's background so they can span the
                    // full island (under the navigator included). Content
                    // here is inset with the rest of the stage.
                    debugArea
                        .frame(height: CGFloat(debugAreaH))
                }
                // No extra backing — the terminal card floats on the ROOT
                // shellBase like every other panel (a tinted wrapper drew a
                // visible rectangle behind the rounded card).
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        // Rounded island floating on the shell base — same panel language as
        // No opaque background of its own — the CARD paints the island's
        // surface (base + terminal band) and casts the shadow. The stage is
        // inset for the floating navigator, so an opaque fill here sat over
        // the card's full-width terminal band and cut the header off in a
        // white rectangle at the content edge.
    }


    /// Live window-resize HUD: frosted interior (titlebar included) + dimensions.
    private var resizeHud: some View {
        ZStack {
            Rectangle().fill(.ultraThinMaterial).ignoresSafeArea()
            VStack(spacing: 6) {
                Image(systemName: "arrow.up.left.and.arrow.down.right")
                    .font(.system(size: 20, weight: .medium))
                    .foregroundStyle(.secondary)
                Text("\(Int(liveResizeSize.width)) × \(Int(liveResizeSize.height))")
                    .font(.system(size: 26, weight: .semibold, design: .rounded))
                    .monospacedDigit()
                    .contentTransition(.numericText())
                Text("Suisei")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 20)
            .glassEffect(
                SuiseiGlass.panel(light: isLightTheme),
                in: RoundedRectangle(cornerRadius: Radius.floating, style: .continuous)
            )
            .shadow(color: .black.opacity(0.25), radius: 20, y: 6)
        }
        .transition(.opacity)
        .allowsHitTesting(false)
    }

    /// Pretty preview (Ctrl/Cmd+Shift+V) — Core-rendered Markdown/JSON lines.
    private var previewPanel: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "doc.richtext")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(accent)
                Text("Preview · \(engine.preview.kindLabel)")
                    .font(.system(size: 11, weight: .semibold, design: .rounded))
                    .foregroundStyle(.primary)
                if !engine.preview.lines.isEmpty {
                    Text("\(engine.preview.lines.count)")
                        .font(.system(size: 10, design: .rounded))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(Color.primary.opacity(0.08)))
                }
                Spacer()
                Text("Esc · ⌘⇧V")
                    .font(.system(size: 10, design: .rounded))
                    .foregroundStyle(.secondary)
                Button {
                    engine.togglePreview()
                    focused = true
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(.secondary)
                        .padding(4)
                }
                .buttonStyle(.plain)
                .help("Close preview")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .overlay(alignment: .bottom) {
                Rectangle().fill(Color(nsColor: .separatorColor)).frame(height: 1)
            }

            if engine.preview.lines.isEmpty {
                VStack(spacing: 10) {
                    Image(systemName: "doc.text.magnifyingglass")
                        .font(.system(size: 28, weight: .light))
                        .foregroundStyle(.tertiary)
                    Text("Nothing to preview")
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(.secondary)
                    Text("Open a Markdown or JSON file, then ⌘⇧V")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 4) {
                        ForEach(engine.preview.lines) { line in
                            previewLineView(line)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .textSelection(.enabled)
                        }
                    }
                    .frame(maxWidth: 820, alignment: .leading)
                    .padding(.horizontal, 28)
                    .padding(.vertical, 16)
                    .frame(maxWidth: .infinity, alignment: .center)
                }
            }
        }
        .background(editorBg)
        .onAppear { engine.refreshPreview() }
    }

    /// Renders a preview line; multi-span lines pack U+E000+style markers from Core.
    @ViewBuilder
    private func previewLineView(_ line: PreviewLineItem) -> some View {
        let runs = parsePreviewRuns(line.text, fallback: line.style)
        if isTableLike(line.text) {
            Text(line.text.unicodeScalars.filter { !(0xE000...0xE0FF).contains($0.value) }
                .reduce(into: "") { $0.unicodeScalars.append($1) })
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(fg.opacity(0.92))
                .lineLimit(1)
        } else if runs.isEmpty {
            Text(" ")
                .font(previewFont(line.style))
                .foregroundStyle(previewColor(line.style))
        } else if runs.count == 1 {
            let r = runs[0]
            Text(r.text.isEmpty ? " " : r.text)
                .font(previewFont(r.style))
                .foregroundStyle(previewColor(r.style))
        } else {
            // Multi-span: wrap runs in an HStack (avoids deprecated Text.+ on macOS 26).
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                ForEach(Array(runs.enumerated()), id: \.offset) { _, r in
                    Text(r.text.isEmpty ? " " : r.text)
                        .font(previewFont(r.style))
                        .foregroundStyle(previewColor(r.style))
                }
            }
        }
    }

    private struct PreviewRun {
        var text: String
        var style: UInt8
    }

    private func parsePreviewRuns(_ raw: String, fallback: UInt8) -> [PreviewRun] {
        guard !raw.isEmpty else { return [PreviewRun(text: " ", style: fallback)] }
        var runs: [PreviewRun] = []
        var curStyle = fallback
        var buf = String()
        var sawMarker = false
        for ch in raw {
            let v = ch.unicodeScalars.first?.value ?? 0
            if (0xE000...0xE0FF).contains(v) {
                sawMarker = true
                if !buf.isEmpty {
                    runs.append(PreviewRun(text: buf, style: curStyle))
                    buf = ""
                }
                curStyle = UInt8(v - 0xE000)
            } else {
                buf.append(ch)
            }
        }
        if !buf.isEmpty || runs.isEmpty {
            runs.append(PreviewRun(text: buf.isEmpty ? " " : buf, style: sawMarker ? curStyle : fallback))
        }
        return runs
    }

    /// Table / box-drawing lines must be monospaced or the pipes shear apart.
    private func isTableLike(_ text: String) -> Bool {
        var pipes = 0
        for ch in text {
            if ch == "|" || ch == "│" || ch == "┌" || ch == "└" || ch == "─" || ch == "├" {
                pipes += 1
                if pipes >= 2 { return true }
            }
        }
        return false
    }

    private func previewFont(_ style: UInt8) -> Font {
        switch style {
        case 1: return .system(size: 22, weight: .bold, design: .default)
        case 2: return .system(size: 18, weight: .semibold, design: .default)
        case 3: return .system(size: 15, weight: .semibold, design: .default)
        case 4: return .system(size: 13, weight: .semibold, design: .default)
        case 5: return .system(size: 13, weight: .bold, design: .default)
        case 6: return .system(size: 13, weight: .regular, design: .default).italic()
        case 7: return .system(size: 12, weight: .medium, design: .monospaced)
        case 12, 13, 14: return .system(size: 12, design: .monospaced)
        default: return .system(size: 13, design: .default)
        }
    }

    private func previewColor(_ style: UInt8) -> Color {
        switch style {
        case 1, 2, 3, 4, 5: return fg
        case 7: return theme.color(theme.string)
        case 8: return accent
        case 9: return dim
        case 11: return dim.opacity(0.75)
        case 12: return theme.color(theme.keyword)
        case 13: return theme.color(theme.string)
        case 14: return theme.color(theme.number)
        default: return fg.opacity(0.92)
        }
    }

    /// Ctrl/Cmd+Shift+T body — PTY output (header optional: split path bar already has one).
    /// Sizes the PTY grid to the visible pane and takes key focus on click.
    private func terminalPaneBody(showClose: Bool, showHeader: Bool = true) -> some View {
        VStack(spacing: 0) {
            if showHeader {
                HStack(spacing: 8) {
                    Image(systemName: "terminal.fill")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(accent)
                    Text("Terminal")
                        .font(.system(size: 11, weight: .semibold, design: .rounded))
                        .foregroundStyle(.primary)
                    if engine.chrome.terminal.lines.isEmpty {
                        Text("starting…")
                            .font(.system(size: 10, design: .rounded))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Text(terminalFocused ? "keys → shell" : "click to type · ⌃⇧T")
                        .font(.system(size: 10, design: .rounded))
                        .foregroundStyle(terminalFocused ? accent : dim)
                    if showClose {
                        HoverIconButton(systemImage: "xmark", help: "Close terminal", fg: Color.primary, dim: Color.secondary) {
                            engine.toggleTerminalFull()
                            focused = true
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .overlay(alignment: .bottom) {
                    Rectangle().fill(Color(nsColor: .separatorColor)).frame(height: 1)
                }
            }

            GeometryReader { geo in
                TerminalGridView(
                    lines: engine.chrome.terminal.lines,
                    cursorRow: engine.chrome.terminal.cursorRow,
                    cursorCol: engine.chrome.terminal.cursorCol,
                    fontSize: 12,
                    bg: NSColor(mixColor(editorBg, .black, isLightTheme ? 0.035 : 0.18)),
                    fg: NSColor(fg)
                )
                .frame(width: geo.size.width, height: geo.size.height)
                .onAppear { reportTerminalCells(geo.size) }
                .onChange(of: geo.size) { _, s in reportTerminalCells(s) }
            }
        }
        .background(editorBg)
        .contentShape(Rectangle())
        .onTapGesture {
            if let bound = engine.chrome.terminal.paneBound {
                engine.focusPane(bound)
            }
            engine.focusTerminal(true)
            focused = true
        }
    }

    private var mainEditor: some View {
        GeometryReader { geo in
            Group {
                if engine.editorSplit.isSplit {
                    splitEditorLayout(size: geo.size)
                } else if engine.chrome.terminal.open && engine.chrome.terminal.fullPanel {
                    // Unsplit whole-main terminal (rare — open path usually creates a split).
                    terminalPaneBody(showClose: true)
                } else {
                    editorSurface(
                        lines: engine.editorLines.isEmpty ? engine.chrome.lines : engine.editorLines,
                        size: geo.size,
                        paneIndex: 0,
                        showFocusRing: false
                    )
                }
            }
            // Pin NSViewRepresentable strictly inside the editor slot (never cover nav).
            .frame(width: geo.size.width, height: geo.size.height)
            .clipped()
            .id(engine.fontGeneration)
            .onAppear {
                engine.resizeEditor(width: geo.size.width, height: geo.size.height)
                // Ensure shell chrome (nav tree) is seeded after light-path sessions.
                if engine.uiNavVisible {
                    engine.ensureProjectTree()
                }
            }
            .onChange(of: geo.size) { _, newSize in
                guard newSize.width > 80, newSize.height > 80 else { return }
                engine.resizeEditor(width: newSize.width, height: newSize.height)
            }
            .background(WindowFrameReporter { engine.editorWindowFrame = $0 })
        }
        .layoutPriority(0)
    }

    /// Xcode assistant-style columns: each pane has its own path bar + buffer.
    /// Split ratio comes from Core; the divider drags live (local override,
    /// committed to Core on release).
    @ViewBuilder
    private func splitEditorLayout(size: CGSize) -> some View {
        let split = engine.editorSplit
        let panes = split.panes
        let pathH: CGFloat = 26
        let ratio = CGFloat(liveSplitRatio ?? Double(split.ratio == 0 ? 0.5 : split.ratio))
        if split.kind == 1 {
            HStack(spacing: 0) {
                ForEach(Array(panes.enumerated()), id: \.element.id) { idx, pane in
                    if idx > 0 {
                        SplitDivider(
                            vertical: true,
                            fg: fg,
                            accent: accent,
                            onDrag: { delta in
                                let base = liveSplitRatio ?? Double(ratio)
                                liveSplitRatio = min(0.85, max(0.15, base + Double(delta / max(1, size.width))))
                            },
                            onEnd: {
                                if let r = liveSplitRatio {
                                    engine.splitSetRatio(r)
                                }
                                liveSplitRatio = nil
                            }
                        )
                    }
                    editorColumn(
                        pane: pane,
                        contentSize: CGSize(
                            width: max(40, (idx == 0 ? size.width * ratio : size.width * (1 - ratio)) - 4),
                            height: max(40, size.height - pathH)
                        )
                    )
                    .frame(width: max(40, (idx == 0 ? size.width * ratio : size.width * (1 - ratio)) - (idx > 0 ? 7 : 0)))
                    .frame(maxHeight: .infinity)
                }
            }
        } else {
            VStack(spacing: 0) {
                ForEach(Array(panes.enumerated()), id: \.element.id) { idx, pane in
                    if idx > 0 {
                        SplitDivider(
                            vertical: false,
                            fg: fg,
                            accent: accent,
                            onDrag: { delta in
                                let base = liveSplitRatio ?? Double(ratio)
                                liveSplitRatio = min(0.85, max(0.15, base + Double(delta / max(1, size.height))))
                            },
                            onEnd: {
                                if let r = liveSplitRatio {
                                    engine.splitSetRatio(r)
                                }
                                liveSplitRatio = nil
                            }
                        )
                    }
                    editorColumn(
                        pane: pane,
                        contentSize: CGSize(
                            width: size.width,
                            height: max(40, (idx == 0 ? size.height * ratio : size.height * (1 - ratio)) - pathH)
                        )
                    )
                    .frame(height: max(40, (idx == 0 ? size.height * ratio : size.height * (1 - ratio)) - (idx > 0 ? 7 : 0)))
                    .frame(maxWidth: .infinity)
                }
            }
        }
    }

    /// One Xcode editor column: path bar (file for this pane) + text surface.
    /// When ⌃⇧T full terminal is bound to this pane, paint the PTY here instead.
    private func editorColumn(pane: EditorPaneSnap, contentSize: CGSize) -> some View {
        let termHere = engine.chrome.terminal.isBoundToPane(pane.id)
        return VStack(spacing: 0) {
            if termHere {
                // Single chrome bar (do not also paint terminalPaneBody header).
                HStack(spacing: 6) {
                    Image(systemName: "terminal.fill")
                        .font(.system(size: 10))
                        .foregroundStyle(pane.focused ? accent : dim)
                    Text("Terminal")
                        .font(.system(size: 11, weight: pane.focused ? .semibold : .regular))
                        .foregroundStyle(pane.focused ? fg : dim)
                    if engine.chrome.terminal.lines.isEmpty {
                        Text("starting…")
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 4)
                    Text("⌃⇧T · keys → shell")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                    Button {
                        engine.toggleTerminalFull()
                        focused = true
                    } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .help("Close terminal")
                }
                .padding(.horizontal, 10)
                .frame(height: 26)
                .background(editorBg.opacity(0.95))
                .overlay(alignment: .bottom) {
                    Rectangle().fill(Color(nsColor: .separatorColor)).frame(height: 1)
                }

                terminalPaneBody(showClose: false, showHeader: false)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                panePathBar(pane: pane)
                editorSurface(
                    lines: pane.lines,
                    size: contentSize,
                    paneIndex: pane.id,
                    showFocusRing: pane.focused
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(editorBg)
        .overlay(
            RoundedRectangle(cornerRadius: 0)
                .strokeBorder(pane.focused ? accent.opacity(0.45) : Color.clear, lineWidth: 1.5)
        )
        // Pane focus follows the canvas's own mouseDown (no tap recognizer —
        // it intercepted AppKit clicks).
    }

    /// Per-pane jump/path bar (like Xcode split editors).
    private func panePathBar(pane: EditorPaneSnap) -> some View {
        let tab = engine.chrome.tabs.first(where: { $0.id == pane.tabIndex })
        let title = tab?.title ?? "[No Name]"
        let dirty = tab?.dirty ?? false
        return HStack(spacing: 6) {
            Image(systemName: "doc.text.fill")
                .font(.system(size: 10))
                .foregroundStyle(pane.focused ? accent : dim)
            Text("\(title)\(dirty ? " ●" : "")")
                .font(.system(size: 11, weight: pane.focused ? .semibold : .regular))
                .foregroundStyle(pane.focused ? fg : dim)
                .lineLimit(1)
            if !engine.chrome.outline.isEmpty, pane.focused {
                Image(systemName: "chevron.forward")
                    .font(.system(size: 8, weight: .semibold))
                    .foregroundStyle(.tertiary)
                Menu {
                    ForEach(engine.chrome.outline) { item in
                        Button("\(item.name)  · L\(item.row)") {
                            engine.focusPane(pane.id)
                            engine.gotoLine(item.row)
                            focused = true
                        }
                    }
                } label: {
                    Text("No Selection")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                .menuStyle(.borderlessButton)
            }
            Spacer(minLength: 4)
            // Open another file into *this* pane (focus then palette).
            Button {
                engine.focusPane(pane.id)
                engine.openFilePalette()
                focused = true
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .frame(width: 20, height: 18)
            }
            .buttonStyle(.plain)
            .help("Open file in this editor")
            if pane.focused {
                Button {
                    engine.closeFocusedPane()
                    focused = true
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(.secondary)
                        .frame(width: 18, height: 18)
                }
                .buttonStyle(.plain)
                .help("Close focused pane")
            }
        }
        .padding(.horizontal, 8)
        .frame(height: 26)
        .background(pane.focused ? editorBg : shellBase.opacity(0.55))
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(pane.focused ? accent.opacity(0.35) : fg.opacity(0.08))
                .frame(height: pane.focused ? 1.5 : 1)
        }
    }

    private func splitDivider(vertical: Bool) -> some View {
        Rectangle()
            .fill(Color(nsColor: .separatorColor))
            .frame(width: vertical ? 1 : nil, height: vertical ? nil : 1)
            .frame(maxWidth: vertical ? 1 : .infinity, maxHeight: vertical ? .infinity : 1)
    }

    /// One pane (or the full editor when unsplit) — pull-based CoreText canvas.
    private func editorSurface(
        lines: [EditorLine],
        size: CGSize,
        paneIndex: Int,
        showFocusRing: Bool
    ) -> some View {
        let focusedPane = paneIndex == engine.editorSplit.focus || !engine.editorSplit.isSplit
        let pane: EditorPaneSnap? = {
            if engine.editorSplit.isSplit,
               paneIndex >= 0,
               paneIndex < engine.editorSplit.panes.count
            {
                return engine.editorSplit.panes[paneIndex]
            }
            return nil
        }()
        // Per-pane scroll / hscroll / doc length — never borrow the focused buffer's.
        let scroll: UInt32 = pane?.scroll ?? engine.chrome.scroll
        let hScroll: UInt32 = pane?.hscroll ?? (focusedPane ? engine.editorHScroll : 0)
        let docCount: UInt32 = pane?.docLineCount ?? engine.chrome.lineCount
        let _ = lines // pull renderer — rows come from the engine on draw
        return EditorHost(
            hScroll: hScroll,
            wrapLines: engine.wrapLines,
            docScroll: scroll,
            docLineCount: max(1, docCount),
            contentGen: engine.chrome.gen,
                    scrollIntent: engine.chrome.scrollIntent,
            editorBg: editorBg,
            fg: fg,
            dim: dim,
            accent: accent,
            selBg: selBg,
            caretColor: caretColor,
            gutterFg: gutterFg,
            cursorLineBg: cursorLineBg,
            theme: theme,
            engine: engine,
            paneIndex: paneIndex,
            showFocusRing: showFocusRing
        )
        // Stable identity per pane — do NOT include tab id (was recreating NSScrollView
        // and wiping native scroll state on every file switch).
        .id("editor-pane-\(paneIndex)")
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .frame(minWidth: size.width > 0 ? nil : size.width)
        // No container tap gesture here: SwiftUI's recognizer delayed/stole
        // mouseDown from the AppKit canvas (caret clicks never landed).
    }

    // MARK: - Status (flat Xcode-like; no editor "modes")

    private var statusLine: some View {
        HStack(spacing: 10) {
            HStack(spacing: 5) {
                Text(fileLeaf(engine.chrome.filename))
                    .font(.system(size: 11))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                if engine.chrome.dirty {
                    Circle()
                        .fill(Color(nsColor: .systemOrange).opacity(0.9))
                        .frame(width: 5, height: 5)
                        .transition(.scale.combined(with: .opacity))
                        .help("Unsaved changes")
                }
            }
            .animation(.snappy(duration: 0.18), value: engine.chrome.dirty)

            if terminalFocused {
                HStack(spacing: 4) {
                    Image(systemName: "keyboard")
                        .font(.system(size: 9, weight: .semibold))
                    Text("Terminal")
                }
                .font(.system(size: 10, weight: .medium, design: .rounded))
                .foregroundStyle(Color.accentColor)
                .padding(.horizontal, 7)
                .padding(.vertical, 2)
                .background(Capsule(style: .continuous).fill(Color.accentColor.opacity(0.12)))
                .transition(.opacity.combined(with: .scale(scale: 0.9)))
                .help("Keys go to the shell — Esc or click the editor to leave")
            }

            if !engine.chrome.message.isEmpty, !isNoiseMessage(engine.chrome.message) {
                Text(engine.chrome.message)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .transition(.opacity)
            }

            Spacer(minLength: 8)

            // Branch (when Core has SCM context)
            if !engine.chrome.branch.isEmpty {
                HStack(spacing: 3) {
                    Image(systemName: "arrow.triangle.branch")
                        .font(.system(size: 9, weight: .semibold))
                    Text(engine.chrome.branch)
                        .lineLimit(1)
                }
                .font(.system(size: 10, design: .rounded))
                .foregroundStyle(.secondary)
                .help("Git branch")
            }

            if !engine.wrapLines {
                Text("No Wrap")
                    .font(.system(size: 10, weight: .medium, design: .rounded))
                    .foregroundStyle(.secondary)
                    .help("Soft-wrap off — trackpad pans horizontally")
            }

            Text(String(format: "Ln %d, Col %d", engine.chrome.cursorRow, engine.chrome.cursorCol))
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)

            if engine.chrome.lineCount > 0 {
                Text("\(engine.chrome.pct)%")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .frame(minWidth: 28, alignment: .trailing)
                    .help("Scroll position")
            }
        }
        // Stops before the inspector instead of sliding under it — that column
        // owns its own floor now (Xcode does the same).
        .padding(
            .trailing,
            outlineVisible ? CGFloat(inspectorW) + 12 : 12
        )
        // The BAR spans the full window and slides under the sidebar card —
        // but its CONTENT must start where the card ends or the filename
        // hides behind the widget.
        .padding(
            .leading,
            engine.uiNavVisible
                ? CGFloat(navW) + ContentView.panelGap + 7 + 12
                : 12
        )
        .frame(height: ContentView.statusBarHeight)
        .frame(maxWidth: .infinity)
        // Shell tone, not `windowBackgroundColor` — the AppKit dynamic is a
        // visibly different tint from the shell every panel sits on, which
        // made the bar read as a stranger strip along the floor.
        .background(shellBase)
        // No separator: the island floats 6pt above with its own shadow, and a
        // full-width rule under it re-drew exactly the kind of hard line the
        // bar's shell tone exists to avoid.
        .animation(.easeOut(duration: 0.15), value: terminalFocused)
    }

    /// Vim-era chatter that shouldn't surface in a GUI status bar.
    private func isNoiseMessage(_ m: String) -> Bool {
        m.hasPrefix("--") || m.contains("same keys as xei") || m.hasPrefix("Search /")
            || m.hasPrefix("Search ?")
    }

    private func fileLeaf(_ path: String) -> String {
        if path.isEmpty || path == "[No Name]" { return "[No Name]" }
        return (path as NSString).lastPathComponent
    }

    private func handleKeyPress(_ press: KeyPress) {
        var mods = SuiseiMod()
        if press.modifiers.contains(.shift) { mods.insert(.shift) }
        if press.modifiers.contains(.control) { mods.insert(.control) }
        if press.modifiers.contains(.option) { mods.insert(.alt) }
        if press.modifiers.contains(.command) { mods.insert(.superKey) }

        if press.modifiers.contains(.command) {
            let c = press.characters.lowercased()
            if c == "o" { engine.openFilePanel(); return }
            if c == "s" {
                // engine.save() decides panel-vs-core (unnamed buffers need Save As).
                engine.save()
                return
            }
            if c == "z" {
                if press.modifiers.contains(.shift) { engine.redo() } else { engine.undo() }
                return
            }
            if c == "a" { engine.selectAll(); return }
            if c == "f", !press.modifiers.contains(.shift) { engine.openFind(); return }
        }

        switch press.key {
        case .escape: engine.dispatch(code: .esc, mods: mods)
        case .return: engine.dispatch(code: .enter, mods: mods)
        case .delete: engine.dispatch(code: .backspace, mods: mods)
        case .tab: engine.dispatch(code: .tab, mods: mods)
        case .leftArrow: engine.dispatch(code: .left, mods: mods)
        case .rightArrow: engine.dispatch(code: .right, mods: mods)
        case .upArrow: engine.dispatch(code: .up, mods: mods)
        case .downArrow: engine.dispatch(code: .down, mods: mods)
        default:
            if let ch = press.characters.first { engine.insertChar(ch) }
        }
    }
}

/// Lightweight frame report (no per-frame async thrash).
private struct WindowFrameReporter: View {
    var onChange: (CGRect) -> Void

    var body: some View {
        GeometryReader { geo in
            Color.clear
                .onAppear {
                    onChange(geo.frame(in: .global))
                }
        }
    }
}

/// Shared panel resize grip — navigator / inspector / terminal.
/// Global coordinates + 1pt quantize + disablesAnimations; no blueprint overlay.
private enum PanelResizeAxis { case horizontal, vertical }

private struct PanelResizeGrip: View {
    @Binding var size: Double
    var minS: Double
    var maxS: Double
    var axis: PanelResizeAxis
    /// When true, positive pointer Δ shrinks the panel (inspector left, terminal top).
    var invert: Bool
    var fg: Color
    var onBegan: (() -> Void)? = nil
    var onEnded: (() -> Void)? = nil

    @State private var dragBase: Double?
    @State private var lastPublished: Double = 0
    @State private var hovering = false

    private var showGrip: Bool { hovering || dragBase != nil }

    var body: some View {
        Group {
            if axis == .horizontal {
                horizontalBody
            } else {
                verticalBody
            }
        }
    }

    private var horizontalBody: some View {
        ZStack {
            Color.clear
            Capsule(style: .continuous)
                .fill(fg.opacity(showGrip ? 0.42 : 0))
                .frame(width: 4, height: 28)
                .animation(.easeOut(duration: 0.12), value: showGrip)
        }
        .frame(width: 7)
        .frame(maxHeight: .infinity)
        .contentShape(Rectangle())
        .frame(width: 1) // 1pt layout, 7pt hit
        .highPriorityGesture(dragGesture)
        .onHover(perform: hover)
        .help("Drag to resize")
    }

    private var verticalBody: some View {
        Rectangle()
            .fill(Color.primary.opacity(0.001))
            .frame(height: 14)
            .frame(maxWidth: .infinity)
            .overlay(
                Capsule(style: .continuous)
                    .fill(fg.opacity(showGrip ? 0.35 : 0))
                    .frame(width: 36, height: 4)
                    .animation(.easeOut(duration: 0.12), value: showGrip)
            )
            .contentShape(Rectangle())
            .highPriorityGesture(dragGesture)
            .onHover(perform: hover)
            .help("Drag to resize")
    }

    private var dragGesture: some Gesture {
        // GLOBAL — grip rides the edge it resizes; local feeds translation back (떨림).
        DragGesture(minimumDistance: 2, coordinateSpace: .global)
            .onChanged { v in
                ContentView.lockWindowBackgroundDrag()
                if dragBase == nil {
                    dragBase = size
                    lastPublished = size
                    onBegan?()
                }
                let base = dragBase ?? size
                let delta: Double = axis == .horizontal
                    ? Double(v.translation.width) * (invert ? -1 : 1)
                    : Double(v.translation.height) * (invert ? -1 : 1)
                let next = min(maxS, max(minS, base + delta)).rounded() // 1pt quantize
                guard abs(next - lastPublished) >= 1 else { return }
                lastPublished = next
                var txn = Transaction()
                txn.disablesAnimations = true
                withTransaction(txn) { size = next }
            }
            .onEnded { _ in
                dragBase = nil
                ContentView.lockWindowBackgroundDrag()
                onEnded?()
            }
    }

    private func hover(_ h: Bool) {
        hovering = h
        if h {
            if axis == .horizontal { NSCursor.resizeLeftRight.push() }
            else { NSCursor.resizeUpDown.push() }
        } else {
            NSCursor.pop()
        }
    }
}

/// Two equal-height capsules fused by a metaball neck.
///
/// This is an ANALYTIC outline, not a rendered one. The obvious approach is the
/// standard SwiftUI metaball (blur the shapes, cut the blurred alpha with
/// `alphaThreshold`), and it is the wrong tool here: thresholding is a raster
/// trick and these are vector controls that need a crisp 1pt border. Everything
/// downstream of that mismatch went wrong in turn — the threshold emits a
/// binary mask so edges aliased; deriving a border from two different cuts gave
/// two independently rasterised masks that would not line up, leaving a broken
/// transparent hairline around the shape; and compositing the border over the
/// fill made the outline visibly brighten wherever the shapes overlapped.
/// A Path has none of those failure modes.
///
///     body   = capsuleA ∪ capsuleB
///     bridge = sliver bounded by two fillet arcs tangent to the facing caps
///     result = body ∪ bridge
///
/// Fillet centres sit `k = √((r+R)² − (d/2)²)` off the axis. The neck vanishes
/// on its own once `R < gap/2` makes tangency impossible, and `k` reaches 0 at
/// exactly that moment — so it pinches to zero thickness before disappearing.
/// A pop is not merely unlikely here, it is geometrically impossible.
enum Metaball {
    /// Fillet radius while the caps still overlap, as a fraction of `r`.
    /// Bigger = more liquid, and the neck survives further into the travel.
    static let goo: CGFloat = 1.0

    /// `a` and `b` must share a height and a vertical centre.
    /// `breakGap` is the gap at which the neck is gone for good.
    static func path(_ a: CGRect, _ b: CGRect, breakGap: CGFloat) -> Path {
        let (left, right) = a.minX <= b.minX ? (a, b) : (b, a)
        guard left.height > 0, left.width > 0, right.width > 0 else { return Path() }
        let r = left.height / 2
        let cy = left.midY

        func capsule(_ rect: CGRect) -> CGPath {
            CGPath(roundedRect: rect, cornerWidth: r, cornerHeight: r, transform: nil)
        }
        var shape = capsule(left).union(capsule(right))

        // The two caps that face each other across the split.
        let c1 = left.maxX - r
        let c2 = right.minX + r
        let d = c2 - c1
        let gap = d - 2 * r
        guard d > 0, gap < breakGap else { return Path(shape) }

        let radius = goo * r * (1 - max(0, gap) / breakGap)
        let half = d / 2
        let kSq = (r + radius) * (r + radius) - half * half
        // `kSq <= 0` means the fillet cannot reach both caps — the neck is
        // already gone and there is nothing to add.
        guard radius > 0, kSq > 0 else { return Path(shape) }

        let k = kSq.squareRoot()
        let mid = (c1 + c2) / 2

        // Tangent points: where each cap circle meets its fillet. They sit along
        // the line joining the two centres, `r` out from the cap. Everything
        // else follows from these four points — and they are the reason this is
        // not built as `corridor − fillets`, which leaves the corridor's four
        // corners behind and squares the waist off into a bitten rectangle.
        func tangent(capX: CGFloat, filletY: CGFloat) -> CGPoint {
            let dx = mid - capX, dy = filletY - cy
            let len = (dx * dx + dy * dy).squareRoot()
            guard len > 0 else { return CGPoint(x: capX, y: cy) }
            return CGPoint(x: capX + r * dx / len, y: cy + r * dy / len)
        }
        func angle(_ p: CGPoint, _ centreY: CGFloat) -> CGFloat {
            atan2(p.y - centreY, p.x - mid)
        }

        let upperY = cy - k, lowerY = cy + k
        let u1 = tangent(capX: c1, filletY: upperY)
        let u2 = tangent(capX: c2, filletY: upperY)
        let l1 = tangent(capX: c1, filletY: lowerY)
        let l2 = tangent(capX: c2, filletY: lowerY)

        // One closed sliver: across the upper fillet, down the trailing cap,
        // back across the lower fillet. Unioning it in only ADDS the material
        // between the union's hard waist corner and the fillet arc — while the
        // caps overlap deeply the arcs fall inside the union and this is a
        // no-op, which is why the merged end of a travel needs no extra guard.
        let bridge = CGMutablePath()
        bridge.move(to: u1)
        bridge.addArc(
            center: CGPoint(x: mid, y: upperY), radius: radius,
            startAngle: angle(u1, upperY), endAngle: angle(u2, upperY), clockwise: true
        )
        bridge.addLine(to: l2)
        bridge.addArc(
            center: CGPoint(x: mid, y: lowerY), radius: radius,
            startAngle: angle(l2, lowerY), endAngle: angle(l1, lowerY), clockwise: true
        )
        bridge.closeSubpath()
        shape = shape.union(bridge)
        return Path(shape)
    }
}

/// The navigator strip's chrome: one pill that SPLITS in two.
///
/// Deliberately NOT `InsettableShape`: `strokeBorder` insets by half the line
/// width, and here an inset does not merely shrink the outline — it feeds
/// through `r`, the cap centres and `d`, so half a point can push the gap past
/// the break and delete the neck. Fill and border then draw two different
/// shapes and the goo appears with no outline around it. Stroking the same
/// uninset path keeps them identical by construction.
private struct SplitCapsule: Shape {
    /// Width of the leading half. While merged this spans the whole rect and
    /// swallows the trailing half, which is what makes the resting state read
    /// as one unbroken capsule.
    var leadingWidth: CGFloat
    /// Width of the trailing half.
    var trailingWidth: CGFloat

    /// Just under the 8pt resting gap so the halves finish visibly apart, which
    /// keeps the goo over ~75% of the travel instead of the last moment.
    private static let breakGap: CGFloat = 7

    /// Both widths travel together, so the pair IS the animatable state.
    var animatableData: AnimatablePair<CGFloat, CGFloat> {
        get { AnimatablePair(leadingWidth, trailingWidth) }
        set {
            leadingWidth = newValue.first
            trailingWidth = newValue.second
        }
    }

    func path(in rect: CGRect) -> Path {
        guard rect.width > 0, rect.height > 0 else { return Path() }
        let r = rect.height / 2
        let lead = min(max(leadingWidth, 2 * r), rect.width)
        let trail = min(max(trailingWidth, 2 * r), rect.width)

        // SAFETY VALVE. At rest the leading half spans everything, so the
        // answer is a plain capsule — say so outright instead of deriving it.
        // The resting state is what the user stares at all day and it must not
        // depend on cap arithmetic being right at every sidebar width; the
        // metaball only earns its keep once the halves actually part.
        // Continuous, too: as `lead` grows to the full width the union tends to
        // this same capsule, so engaging it costs no pop.
        if lead >= rect.width {
            return Path(roundedRect: rect, cornerRadius: r, style: .circular)
        }

        return Metaball.path(
            CGRect(x: rect.minX, y: rect.minY, width: lead, height: rect.height),
            CGRect(x: rect.maxX - trail, y: rect.minY, width: trail, height: rect.height),
            breakGap: Self.breakGap
        )
    }
}
/// Mouse handling for the tab strip, in AppKit.
///
/// Exists for one reason: `mouseDownCanMoveWindow`. The strip lives in the
/// titlebar region of a `.fullSizeContentView` window, and AppKit drags the
/// window from any view there that allows it — consuming the press before
/// SwiftUI ever arbitrates. Overriding it to `false` is what lets mouse events
/// reach anything at all here.
///
/// Because this overlay is then the hit view, it owns clicks as well as drags
/// and hands both back through closures.
private struct TabStripMouse: NSViewRepresentable {
    /// Which tab sits at an x in the strip's own coordinates (for clicks).
    var slotAt: (CGFloat) -> Int?
    /// The one-step swap target while dragging, or nil to stay put.
    var targetFor: (Int, CGFloat) -> Int?
    var onDrag: (Int, Int) -> Void
    var onPick: (Int) -> Void
    var onClick: (Int) -> Void
    var onEnd: () -> Void

    final class Catcher: NSView {
        var slotAt: ((CGFloat) -> Int?)?
        var targetFor: ((Int, CGFloat) -> Int?)?
        var onDrag: ((Int, Int) -> Void)?
        var onPick: ((Int) -> Void)?
        var onClick: ((Int) -> Void)?
        var onEnd: (() -> Void)?

        private var held: Int?
        private var startX: CGFloat = 0
        private var moved = false
        /// Where the last swap happened. The chip frames are re-measured
        /// asynchronously, so for a frame or two after a move they still
        /// describe the OLD order — acting on those is the other half of the
        /// shake. Requiring the cursor to travel before swapping again rides
        /// over that window without needing to know how long it is.
        private var lastSwapX: CGFloat?

        /// THE fix. Without it AppKit takes the press to drag the window and
        /// none of the handlers below ever run.
        override var mouseDownCanMoveWindow: Bool { false }
        /// Reorder a tab without having to focus the window first.
        override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
        override var isFlipped: Bool { true }

        private func x(of event: NSEvent) -> CGFloat {
            convert(event.locationInWindow, from: nil).x
        }

        override func mouseDown(with event: NSEvent) {
            startX = x(of: event)
            moved = false
            held = slotAt?(startX) ?? nil
        }

        override func mouseDragged(with event: NSEvent) { advance(to: x(of: event)) }

        /// Also treated as a drag while a press is live. Some event sources —
        /// synthetic input among them — post `mouseMoved` with the button held
        /// rather than `leftMouseDragged`, and a reorder that only listens for
        /// the latter silently does nothing for them.
        override func mouseMoved(with event: NSEvent) {
            guard held != nil else { return }
            advance(to: x(of: event))
        }

        private func advance(to cur: CGFloat) {
            if !moved, abs(cur - startX) > 3 {
                moved = true
                if let held { onPick?(held) }
            }
            guard moved, let from = held else { return }
            if let last = lastSwapX, abs(cur - last) < 6 { return }
            guard let to = targetFor?(from, cur), to != from else { return }
            onDrag?(from, to)
            held = to
            lastSwapX = cur
        }

        // `mouseMoved` needs a tracking area to be delivered at all.
        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            trackingAreas.forEach(removeTrackingArea)
            addTrackingArea(NSTrackingArea(
                rect: bounds,
                options: [.activeInKeyWindow, .mouseMoved, .inVisibleRect],
                owner: self
            ))
        }

        override func mouseUp(with event: NSEvent) {
            if !moved, let slot = slotAt?(x(of: event)) {
                onClick?(slot)
            }
            held = nil
            moved = false
            lastSwapX = nil
            onEnd?()
        }
    }

    func makeNSView(context: Context) -> Catcher {
        let v = Catcher()
        apply(to: v)
        return v
    }

    func updateNSView(_ v: Catcher, context: Context) { apply(to: v) }

    private func apply(to v: Catcher) {
        v.slotAt = slotAt
        v.targetFor = targetFor
        v.onDrag = onDrag
        v.onPick = onPick
        v.onClick = onClick
        v.onEnd = onEnd
    }
}

private struct TravellingPill: Shape {
    /// 0 = fully at `from`, 1 = fully at `to`.
    var progress: CGFloat
    var from: CGFloat
    var to: CGFloat
    var width: CGFloat

    /// Ceiling on the elongation, so a jump across the rail stretches into a
    /// lozenge rather than a bar spanning the whole strip.
    private static let maxStretch: CGFloat = 30

    /// Extra size at mid-flight, on top of the directional stretch: the pill
    /// swells as it leaves and settles back as it arrives.
    ///
    /// This lives on the SOLID pill now. It used to be a constant `+8` on the
    /// liquid-glass overlay, which meant the glass was permanently bigger than
    /// the pill it handed back to, and the hand-off was a crossfade — so the
    /// size never actually travelled, it stepped. `sin(πt)` is zero at both
    /// ends, and `TravellingPill` interpolates `progress` per frame, so the
    /// growth and the return are one continuous motion.
    private static let maxSwell: CGFloat = 6

    /// ALL FOUR values animate, not just `progress`. The slots resize whenever
    /// the Debug Area toggles, so `from`, `to` and `width` change too — and a
    /// Shape only interpolates what `animatableData` carries. Left out, they
    /// snapped to their new values on the first frame while the glyphs slid
    /// smoothly underneath, and the pill visibly came unstuck from the icons
    /// it is supposed to be sitting on.
    var animatableData: AnimatablePair<CGFloat, AnimatablePair<CGFloat, AnimatablePair<CGFloat, CGFloat>>> {
        get { AnimatablePair(progress, AnimatablePair(from, AnimatablePair(to, width))) }
        set {
            progress = newValue.first
            from = newValue.second.first
            to = newValue.second.second.first
            width = newValue.second.second.second
        }
    }

    /// Centre eases across; length swells and returns. `sin(πt)` is zero at
    /// BOTH ends by construction, so the endpoints are exactly a resting pill.
    /// Static so any overlay riding the pill uses the identical path —
    /// two copies of this math WILL drift.
    static func geometry(
        progress: CGFloat, from: CGFloat, to: CGFloat, width: CGFloat
    ) -> (centre: CGFloat, length: CGFloat) {
        let t = min(max(progress, 0), 1)
        let span = to - from
        let centre = from + span * (1 - pow(1 - t, 1.8)) + width / 2
        let stretch = min(abs(span), maxStretch) * sin(.pi * t)
        return (centre, width + stretch)
    }

    func path(in rect: CGRect) -> Path {
        guard rect.height > 0, width > 0 else { return Path() }
        let g = Self.geometry(progress: progress, from: from, to: to, width: width)
        let swell = Self.maxSwell * sin(.pi * min(max(progress, 0), 1))
        let h = rect.height + swell
        let w = g.length + swell
        return Path(
            roundedRect: CGRect(
                x: g.centre - w / 2, y: rect.minY - swell / 2,
                width: w, height: h
            ),
            cornerRadius: h / 2, style: .circular
        )
    }
}


/// Find navigator's query field. Same capsule language as the Project
/// navigator's filter bar so the two rails read as one family.
private struct NavigatorSearchField: View {
    @Binding var text: String
    var onSubmit: () -> Void

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
            TextField("Search project", text: $text)
                .textFieldStyle(.plain)
                .font(.system(size: 11))
                .onSubmit(onSubmit)
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 4)
        .background(Capsule(style: .continuous).fill(Color.primary.opacity(0.06)))
        .overlay(
            Capsule(style: .continuous)
                .strokeBorder(Color(nsColor: .separatorColor).opacity(0.5), lineWidth: 1)
        )
    }
}

/// A panel docked to the bottom of a surface, fused to it rather than laid on
/// top of it.
///
/// The corners run the OTHER way from a normal card: instead of convex corners
/// that pull the panel away from the walls and leave a notch of background in
/// each top corner, these are CONCAVE fillets sweeping up into the walls. That
/// is the same fillet the split pill uses for its neck, applied to a single
/// junction — the panel reads as grown from the surface, which is the whole
/// reason the terminal gets a metaball and the navigator does not.
///
///     convex (a slab dropped on)      concave (fused)
///       ╭──────────────╮                ╰──────────────╯
///       │              │                │              │
private struct DockedPanelShape: Shape {
    /// Radius of the concave sweep into the side walls.
    var fillet: CGFloat = 12

    func path(in rect: CGRect) -> Path {
        guard rect.width > 0, rect.height > 0 else { return Path() }
        // Never let the two fillets meet in the middle on a narrow panel.
        let f = min(fillet, rect.width / 2, rect.height)
        var p = Path()
        p.move(to: CGPoint(x: rect.minX, y: rect.minY - f))
        p.addArc(
            center: CGPoint(x: rect.minX + f, y: rect.minY - f), radius: f,
            startAngle: .degrees(180), endAngle: .degrees(90), clockwise: true
        )
        p.addLine(to: CGPoint(x: rect.maxX - f, y: rect.minY))
        p.addArc(
            center: CGPoint(x: rect.maxX - f, y: rect.minY - f), radius: f,
            startAngle: .degrees(90), endAngle: .degrees(0), clockwise: true
        )
        p.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        p.addLine(to: CGPoint(x: rect.minX, y: rect.maxY))
        p.closeSubpath()
        return p
    }
}

/// Real within-window blur — SwiftUI materials cannot sample AppKit siblings
/// (the editor canvas), NSVisualEffectView can.
///
/// `light` pins the material's appearance to the THEME (the chrome's source of
/// truth). Left to inherit, the effect view resolves its own appearance and can
/// disagree with the SwiftUI chrome around it — that was the "light theme but
/// pitch-black sidebar" mismatch.
struct WithinWindowBlur: NSViewRepresentable {
    var material: NSVisualEffectView.Material = .popover
    var light: Bool? = nil

    func makeNSView(context: Context) -> NSVisualEffectView {
        let v = NSVisualEffectView()
        v.blendingMode = .withinWindow
        v.material = material
        v.state = .active
        apply(v)
        return v
    }

    func updateNSView(_ v: NSVisualEffectView, context: Context) {
        v.material = material
        apply(v)
    }

    private func apply(_ v: NSVisualEffectView) {
        if let light {
            v.appearance = NSAppearance(named: light ? .aqua : .darkAqua)
        } else {
            v.appearance = nil
        }
    }
}

/// Soft fade where content slides under a pane edge — a plain editorBg
/// gradient (materials over the AppKit canvas are veils, not blurs).
struct EdgeFade: View {
    var color: Color
    var top: Bool

    var body: some View {
        LinearGradient(
            colors: top ? [color, color.opacity(0)] : [color.opacity(0), color],
            startPoint: .top,
            endPoint: .bottom
        )
        .allowsHitTesting(false)
    }
}

/// Editor split divider — hover grip, accent while dragging, live ratio.
private struct SplitDivider: View {
    var vertical: Bool
    var fg: Color
    var accent: Color
    var onDrag: (CGFloat) -> Void
    var onEnd: () -> Void
    @State private var hovering = false
    @State private var dragging = false
    @State private var lastTranslation: CGFloat = 0

    var body: some View {
        ZStack {
            Rectangle()
                .fill(fg.opacity(dragging ? 0.18 : (hovering ? 0.14 : 0.08)))
            Capsule(style: .continuous)
                .fill(dragging ? accent.opacity(0.9) : fg.opacity(hovering ? 0.45 : 0))
                .frame(
                    width: vertical ? 3 : 26,
                    height: vertical ? 26 : 3
                )
        }
        .frame(width: vertical ? 7 : nil, height: vertical ? nil : 7)
        .frame(maxWidth: vertical ? 7 : .infinity, maxHeight: vertical ? .infinity : 7)
        .contentShape(Rectangle())
        .animation(.snappy(duration: 0.15), value: hovering)
        .animation(.snappy(duration: 0.15), value: dragging)
        .highPriorityGesture(
            DragGesture(minimumDistance: 1, coordinateSpace: .global)
                .onChanged { v in
                    dragging = true
                    let t = vertical ? v.translation.width : v.translation.height
                    onDrag(t - lastTranslation)
                    lastTranslation = t
                }
                .onEnded { _ in
                    dragging = false
                    lastTranslation = 0
                    onEnd()
                }
        )
        .onHover { h in
            hovering = h
            if h {
                if vertical {
                    NSCursor.resizeLeftRight.push()
                } else {
                    NSCursor.resizeUpDown.push()
                }
            } else {
                NSCursor.pop()
            }
        }
    }
}

/// Xcode-style minimap: downsampled code bars + viewport indicator.
/// Click / drag glides the editor to that spot (via `.suiseiScrollToLine`).
/// Objective-C target for the "+" NSMenu items (plain Button + NSMenu keeps
/// the glyph geometry SwiftUI-exact; see tabPlusMenu).
final class PlusMenuBridge: NSObject {
    weak var engine: EngineBridge?

    @objc func newTab() { engine?.openBlankTab() }
    @objc func nextTab() { engine?.nextTab() }
    @objc func prevTab() { engine?.prevTab() }
    @objc func splitRight() { engine?.splitEditorRight() }
    @objc func splitBelow() { engine?.splitEditorBelow() }
    @objc func focusNextPane() { engine?.focusNextPane() }
    @objc func closeFocusedPane() { engine?.closeFocusedPane() }
}

struct MinimapStrip: View {
    @ObservedObject var engine: EngineBridge
    var accent: Color
    var fg: Color
    var bg: Color
    var isLight: Bool
    /// Live first-visible line — fed by the scroll view during gestures
    /// (chrome.scroll only updates on publishes, which live scroll suppresses).
    @State private var liveScrollLine: Int = -1

    /// Rows render at a FIXED density like Xcode's minimap — capped so a small
    /// file clusters tightly at the top instead of stretching bars across the
    /// whole strip ("띄엄띄엄" — read as broken rendering).
    private static let maxRowHeight: CGFloat = 2.6

    private func rowHeight(_ n: Int, stripHeight: CGFloat) -> CGFloat {
        min(Self.maxRowHeight, stripHeight / CGFloat(max(1, n)))
    }

    var body: some View {
        GeometryReader { geo in
            let data = engine.minimapData()
            ZStack(alignment: .topLeading) {
                // Bars redraw ONLY when the document data changes — redrawing
                // 2k rects on every scroll tick was the minimap stutter.
                MinimapBars(
                    data: data, accent: accent, fg: fg, isLight: isLight,
                    rowH: rowHeight(data?.len.count ?? 1, stripHeight: geo.size.height)
                )
                .equatable()

                // Viewport indicator — a cheap offset move at frame rate,
                // mapped over the RENDERED height (≠ strip height for small
                // files).
                if let data, data.totalLines > 0, !data.len.isEmpty {
                    let n = data.len.count
                    let rowH = rowHeight(n, stripHeight: geo.size.height)
                    let mapH = CGFloat(n) * rowH
                    let total = CGFloat(data.totalLines)
                    let visRows = geo.size.height / max(1, EditorMetrics.lineHeight)
                    let line = liveScrollLine >= 0 ? liveScrollLine : Int(engine.chrome.scroll)
                    let h = min(mapH, max(18, visRows / total * mapH))
                    let y0 = max(0, min(CGFloat(line) / total * mapH, mapH - h))
                    RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                        .fill(fg.opacity(isLight ? 0.08 : 0.10))
                        .overlay(
                            RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                                .strokeBorder(fg.opacity(0.18), lineWidth: 1)
                        )
                        .frame(width: geo.size.width - 2, height: h)
                        .offset(x: 1, y: y0)
                }
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { v in
                        jump(y: v.location.y, height: geo.size.height, data: data)
                    }
            )
            .onReceive(NotificationCenter.default.publisher(for: .suiseiEditorScrolled)) { note in
                if let line = note.userInfo?["line"] as? Int {
                    liveScrollLine = line
                }
            }
            // NO chrome.scroll sync here: on an outline jump core updates its
            // scroll to the DESTINATION first, which snapped the indicator to
            // the target and then re-animated it from the start ("튀었다가
            // 이동"). The live clip feed above covers every real movement.
        }
        // Opaque panel surface — by design (the blur experiment is retired).
        .background(bg)
        .overlay(alignment: .leading) {
            Rectangle().fill(Color(nsColor: .separatorColor)).frame(width: 1)
        }
    }

    /// Document bars only — Equatable so scroll-driven body updates skip the
    /// 2k-rect redraw entirely (the indicator lives in its own layer).
    private struct MinimapBars: View, Equatable {
        var data: EngineBridge.MinimapData?
        var accent: Color
        var fg: Color
        var isLight: Bool
        var rowH: CGFloat

        static func == (a: MinimapBars, b: MinimapBars) -> Bool {
            a.data == b.data && a.rowH == b.rowH && a.isLight == b.isLight
        }

        var body: some View {
            Canvas { ctx, size in
                let t0 = DispatchTime.now().uptimeNanoseconds
                defer {
                    PerfProbe.record(
                        "MinimapBars canvas fill",
                        Double(DispatchTime.now().uptimeNanoseconds - t0) / 1_000_000
                    )
                }
                guard let data, data.totalLines > 0, !data.len.isEmpty else { return }
                let n = data.len.count
                let barH = max(0.8, min(2.0, rowH * 0.62))
                let body = fg.opacity(isLight ? 0.26 : 0.30)
                for i in 0..<n {
                    let len = CGFloat(data.len[i])
                    if len == 0 { continue }
                    let x = 5 + CGFloat(data.indent[i]) * 0.32
                    let w = min(size.width - x - 4, len * 0.42)
                    if w <= 0.5 { continue }
                    let y = CGFloat(i) * rowH
                    let color = data.flags[i] == 1 ? accent.opacity(0.8) : body
                    ctx.fill(
                        Path(CGRect(x: x, y: y, width: w, height: barH)),
                        with: .color(color)
                    )
                }
            }
        }
    }

    private func jump(y: CGFloat, height: CGFloat, data: EngineBridge.MinimapData?) {
        guard let data, data.totalLines > 0, height > 0 else { return }
        // Same fixed-density mapping the renderer uses.
        let rowH = rowHeight(data.len.count, stripHeight: height)
        let mapH = max(1, CGFloat(data.len.count) * rowH)
        let frac = max(0, min(1, y / mapH))
        let line = Int(frac * CGFloat(data.totalLines))
        NotificationCenter.default.post(
            name: .suiseiScrollToLine,
            object: nil,
            userInfo: ["line": line]
        )
    }
}

// MARK: - Shared terminal / jump-bar helpers

/// PTY grid — native canvas, one uniform background, fixed row pitch.
/// (The SwiftUI LazyVStack version painted per-run background boxes with row
/// gaps: the "striped" terminal artifact.)
private struct TerminalGridView: NSViewRepresentable {
    var lines: [String]
    /// Shell cursor within `lines` — drawn as a block caret like every terminal.
    var cursorRow: Int = 0
    var cursorCol: Int = 0
    var fontSize: CGFloat
    var bg: NSColor
    var fg: NSColor
    /// Wheel past the ends of the live grid; positive = older output.
    var onScrollback: (Int32) -> Void = { _ in }

    func makeNSView(context: Context) -> TermScroll {
        TermScroll()
    }

    func updateNSView(_ view: TermScroll, context: Context) {
        view.cursorRow = cursorRow
        view.cursorCol = cursorCol
        view.onScrollback = onScrollback
        view.apply(lines: lines, fontSize: fontSize, bg: bg, fg: fg)
    }
}

private final class TermScroll: NSScrollView {
    var cursorRow: Int = 0 { didSet { canvas.setCursor(row: cursorRow, col: cursorCol) } }
    var cursorCol: Int = 0 { didSet { canvas.setCursor(row: cursorRow, col: cursorCol) } }
    let canvas = TermCanvas()
    /// Positive = reveal older output.
    var onScrollback: ((Int32) -> Void)?
    /// Sub-row remainder, so a slow trackpad drag eventually moves instead of
    /// rounding every delta away.
    private var scrollbackResidue: CGFloat = 0

    /// The grid this view holds is the LIVE SCREEN — core sizes the PTY to the
    /// panel, so there is usually nothing here to scroll natively at all. The
    /// 5,000 rows of history live on the other side of the ABI, and until this
    /// existed nothing could ask for them: scrolling the terminal panel did
    /// nothing whatsoever. Native scrolling still wins whenever there IS
    /// content to move; only the part that runs off an end is forwarded.
    override func scrollWheel(with event: NSEvent) {
        let dy = event.scrollingDeltaY
        guard dy != 0 else {
            super.scrollWheel(with: event)
            return
        }
        let scrollable = canvas.frame.height > contentView.bounds.height + 1
        let atTop = !scrollable || documentVisibleRect.minY <= 0.5
        let atBottom = !scrollable || documentVisibleRect.maxY >= canvas.frame.height - 0.5
        guard (dy > 0 && atTop) || (dy < 0 && atBottom) else {
            super.scrollWheel(with: event)
            return
        }
        // A notched wheel reports whole lines; a trackpad reports pixels.
        let perRow: CGFloat = event.hasPreciseScrollingDeltas ? 16 : 1
        scrollbackResidue += dy / perRow
        let rows = scrollbackResidue.rounded(.towardZero)
        guard rows != 0 else { return }
        scrollbackResidue -= rows
        onScrollback?(Int32(rows))
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        drawsBackground = false
        borderType = .noBorder
        hasVerticalScroller = true
        autohidesScrollers = true
        scrollerStyle = .overlay
        automaticallyAdjustsContentInsets = false
        contentInsets = .init()
        documentView = canvas
    }

    required init?(coder: NSCoder) { fatalError("unused") }

    override var isFlipped: Bool { true }

    /// Row count and metrics as of the last `apply`, so a frame change can
    /// re-fit without one.
    private var lastRowCount: Int = 1
    private var lastFontSize: CGFloat = 12

    /// Re-fit the grid whenever OUR frame changes.
    ///
    /// The canvas was sized only inside `apply`, which runs when SwiftUI hands
    /// down new lines — but a panel resize changes this view's frame through
    /// AppKit layout instead. The grid kept its old width, so the terminal's
    /// dark background stopped short of the new edge until the next output
    /// arrived.
    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        fitCanvasToBounds()
    }

    private func fitCanvasToBounds() {
        let lineH = lastFontSize + 5
        let h = max(bounds.height, CGFloat(lastRowCount) * lineH + 12)
        let w = max(bounds.width, 200)
        guard abs(canvas.frame.height - h) > 0.5 || abs(canvas.frame.width - w) > 0.5
        else { return }
        canvas.setFrameSize(NSSize(width: w, height: h))
        canvas.needsDisplay = true
    }

    func apply(lines: [String], fontSize: CGFloat, bg: NSColor, fg: NSColor) {
        // Core hands the WHOLE grid (empty rows as " "): untrimmed, a fresh
        // 44-row shell overflowed the panel and bottom-stick scrolled the
        // prompt half off the top. Trailing blanks carry no information.
        var lines = lines
        while lines.count > 1, lines.last == " " {
            lines.removeLast()
        }
        let lineH = fontSize + 5
        let wasAtBottom = documentVisibleRect.maxY >= canvas.frame.height - lineH * 2
        let changed = canvas.set(lines: lines, fontSize: fontSize, bg: bg, fg: fg)
        lastRowCount = lines.count
        lastFontSize = fontSize
        fitCanvasToBounds()
        if changed, wasAtBottom {
            let y = max(0, canvas.frame.height - contentView.bounds.height)
            contentView.setBoundsOrigin(NSPoint(x: 0, y: y))
            reflectScrolledClipView(contentView)
        }
    }
}

private final class TermCanvas: NSView {
    private var cursorRow = 0
    private var cursorCol = 0

    func setCursor(row: Int, col: Int) {
        guard row != cursorRow || col != cursorCol else { return }
        cursorRow = row
        cursorCol = col
        needsDisplay = true
    }

    private var lines: [String] = []
    private var fontSize: CGFloat = 12
    private var bg: NSColor = .black
    private var fg: NSColor = .white
    private var rowCache: [Int: CTLine] = [:]

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { false }

    /// Returns true when content changed (caller sticks to bottom).
    func set(lines: [String], fontSize: CGFloat, bg: NSColor, fg: NSColor) -> Bool {
        let changed = lines != self.lines || fontSize != self.fontSize
            || bg != self.bg || fg != self.fg
        guard changed else { return false }
        self.lines = lines
        self.fontSize = fontSize
        self.bg = bg
        self.fg = fg
        rowCache.removeAll(keepingCapacity: true)
        needsDisplay = true
        return true
    }

    override func draw(_ dirtyRect: NSRect) {
        bg.setFill()
        dirtyRect.fill()
        guard let cg = NSGraphicsContext.current?.cgContext else { return }
        let lineH = fontSize + 5
        let font = EditorMetrics.monospaced(fontSize, weight: .regular)
        let r0 = max(0, Int(floor(dirtyRect.minY / lineH)))
        let r1 = min(lines.count - 1, Int(ceil(dirtyRect.maxY / lineH)))
        guard r1 >= r0 else { return }
        for i in r0...r1 {
            let y = CGFloat(i) * lineH + 6
            let ct = row(i, font: font)
            cg.saveGState()
            cg.textMatrix = .identity
            cg.translateBy(x: 10, y: y + font.ascender)
            cg.scaleBy(x: 1, y: -1)
            CTLineDraw(ct, cg)
            cg.restoreGState()

            // Block caret, the way terminals draw it — measured against the
            // rendered line so wide glyphs land correctly.
            if i == cursorRow {
                let cellW = ("M" as NSString)
                    .size(withAttributes: [.font: font]).width
                let x = 10 + CTLineGetOffsetForStringIndex(ct, CFIndex(cursorCol), nil)
                let caret = CGRect(x: x, y: y, width: max(2, cellW), height: lineH - 2)
                fg.withAlphaComponent(0.55).setFill()
                caret.fill()
            }
        }
    }

    private func row(_ i: Int, font: NSFont) -> CTLine {
        if let cached = rowCache[i] { return cached }
        let runs = AnsiParser.parse(lines[i], defaultFg: Color(nsColor: fg))
        let out = NSMutableAttributedString()
        for run in runs {
            out.append(NSAttributedString(
                string: run.text,
                attributes: [
                    .font: font,
                    .foregroundColor: NSColor(run.fg),
                ]
            ))
        }
        if out.length == 0 {
            out.append(NSAttributedString(string: " ", attributes: [.font: font]))
        }
        let ct = CTLineCreateWithAttributedString(out)
        rowCache[i] = ct
        if rowCache.count > 400 { rowCache.removeAll(keepingCapacity: true) }
        return ct
    }
}

/// PTY / terminal log with stick-to-bottom + ANSI SGR (truecolor / 16-color).
private struct TerminalOutputView: View {
    var lines: [String]
    var fg: Color
    var dim: Color
    var fontSize: CGFloat
    var stickToBottom: Bool

    @State private var pinBottom = true

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    if lines.isEmpty {
                        Text(" ")
                            .font(.system(size: fontSize, design: .monospaced))
                            .id("term-empty")
                    }
                    ForEach(Array(lines.enumerated()), id: \.offset) { i, line in
                        ansiLineView(line)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)
                            .id(i)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
            }
            .onChange(of: lines.count) { _, n in
                guard stickToBottom, pinBottom, n > 0 else { return }
                proxy.scrollTo(n - 1, anchor: .bottom)
            }
            .onChange(of: lines) { _, newLines in
                guard stickToBottom, pinBottom, !newLines.isEmpty else { return }
                proxy.scrollTo(newLines.count - 1, anchor: .bottom)
            }
            .onAppear {
                if stickToBottom, !lines.isEmpty {
                    proxy.scrollTo(lines.count - 1, anchor: .bottom)
                }
            }
            .contextMenu {
                Button(pinBottom ? "Pause auto-scroll" : "Resume auto-scroll") {
                    pinBottom.toggle()
                }
            }
        }
    }

    @ViewBuilder
    private func ansiLineView(_ raw: String) -> some View {
        let runs = AnsiParser.parse(raw, defaultFg: fg)
        if runs.isEmpty {
            Text(" ")
                .font(.system(size: fontSize, design: .monospaced))
                .foregroundStyle(.primary)
        } else if runs.count == 1 {
            Text(runs[0].text.isEmpty ? " " : runs[0].text)
                .font(.system(size: fontSize, design: .monospaced))
                .foregroundStyle(runs[0].fg)
                .background(runs[0].bg.map { Rectangle().fill($0) })
        } else {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                ForEach(Array(runs.enumerated()), id: \.offset) { _, r in
                    Text(r.text.isEmpty ? " " : r.text)
                        .font(.system(size: fontSize, design: .monospaced))
                        .foregroundStyle(r.fg)
                        .background(r.bg.map { Rectangle().fill($0) })
                }
            }
        }
    }
}

/// Minimal CSI SGR parser for terminal face paint (`\x1b[…m`).
private enum AnsiParser {
    struct Run {
        var text: String
        var fg: Color
        var bg: Color?
    }

    static func parse(_ raw: String, defaultFg: Color) -> [Run] {
        guard raw.contains("\u{1b}") else {
            return [Run(text: raw.isEmpty ? " " : raw, fg: defaultFg.opacity(0.92), bg: nil)]
        }
        var runs: [Run] = []
        var fg = defaultFg.opacity(0.92)
        var bg: Color? = nil
        var buf = ""
        var i = raw.startIndex
        while i < raw.endIndex {
            if raw[i] == "\u{1b}", raw.index(after: i) < raw.endIndex, raw[raw.index(after: i)] == "[" {
                if !buf.isEmpty {
                    runs.append(Run(text: buf, fg: fg, bg: bg))
                    buf = ""
                }
                var j = raw.index(i, offsetBy: 2)
                var params = ""
                while j < raw.endIndex {
                    let ch = raw[j]
                    if ch == "m" {
                        applySgr(params, fg: &fg, bg: &bg, defaultFg: defaultFg)
                        j = raw.index(after: j)
                        break
                    }
                    if ch.isNumber || ch == ";" {
                        params.append(ch)
                        j = raw.index(after: j)
                    } else {
                        // Unknown CSI — skip until letter
                        while j < raw.endIndex, raw[j].isASCII, raw[j].isLetter == false {
                            j = raw.index(after: j)
                        }
                        if j < raw.endIndex { j = raw.index(after: j) }
                        break
                    }
                }
                i = j
            } else {
                buf.append(raw[i])
                i = raw.index(after: i)
            }
        }
        if !buf.isEmpty {
            runs.append(Run(text: buf, fg: fg, bg: bg))
        }
        return runs.isEmpty ? [Run(text: " ", fg: defaultFg.opacity(0.92), bg: nil)] : runs
    }

    private static func applySgr(_ params: String, fg: inout Color, bg: inout Color?, defaultFg: Color) {
        let parts = params.isEmpty ? ["0"] : params.split(separator: ";").map(String.init)
        var i = 0
        while i < parts.count {
            let n = Int(parts[i]) ?? 0
            switch n {
            case 0:
                fg = defaultFg.opacity(0.92)
                bg = nil
            case 39:
                fg = defaultFg.opacity(0.92)
            case 49:
                bg = nil
            case 30...37:
                fg = basicColor(n - 30, bright: false)
            case 90...97:
                fg = basicColor(n - 90, bright: true)
            case 40...47:
                bg = basicColor(n - 40, bright: false).opacity(0.35)
            case 100...107:
                bg = basicColor(n - 100, bright: true).opacity(0.35)
            case 38:
                if i + 1 < parts.count, parts[i + 1] == "2", i + 4 < parts.count {
                    let r = Double(Int(parts[i + 2]) ?? 200) / 255
                    let g = Double(Int(parts[i + 3]) ?? 200) / 255
                    let b = Double(Int(parts[i + 4]) ?? 200) / 255
                    fg = Color(red: r, green: g, blue: b)
                    i += 4
                } else if i + 1 < parts.count, parts[i + 1] == "5", i + 2 < parts.count {
                    fg = index256(Int(parts[i + 2]) ?? 7)
                    i += 2
                }
            case 48:
                if i + 1 < parts.count, parts[i + 1] == "2", i + 4 < parts.count {
                    let r = Double(Int(parts[i + 2]) ?? 0) / 255
                    let g = Double(Int(parts[i + 3]) ?? 0) / 255
                    let b = Double(Int(parts[i + 4]) ?? 0) / 255
                    bg = Color(red: r, green: g, blue: b).opacity(0.45)
                    i += 4
                } else if i + 1 < parts.count, parts[i + 1] == "5", i + 2 < parts.count {
                    bg = index256(Int(parts[i + 2]) ?? 0).opacity(0.35)
                    i += 2
                }
            default:
                break
            }
            i += 1
        }
    }

    private static func basicColor(_ i: Int, bright: Bool) -> Color {
        let table: [(Double, Double, Double)] = [
            (0.0, 0.0, 0.0),
            (0.80, 0.19, 0.19),
            (0.05, 0.74, 0.47),
            (0.90, 0.90, 0.06),
            (0.14, 0.45, 0.78),
            (0.74, 0.25, 0.74),
            (0.07, 0.66, 0.80),
            (0.90, 0.90, 0.90),
        ]
        let brightTable: [(Double, Double, Double)] = [
            (0.40, 0.40, 0.40),
            (0.95, 0.30, 0.30),
            (0.14, 0.82, 0.55),
            (0.96, 0.96, 0.26),
            (0.23, 0.56, 0.92),
            (0.84, 0.44, 0.84),
            (0.16, 0.72, 0.86),
            (1.0, 1.0, 1.0),
        ]
        let t = bright ? brightTable : table
        let c = t[max(0, min(7, i))]
        return Color(red: c.0, green: c.1, blue: c.2)
    }

    private static func index256(_ i: Int) -> Color {
        if i < 16 {
            return basicColor(i % 8, bright: i >= 8)
        }
        if i <= 231 {
            let n = i - 16
            let r = Double((n / 36) % 6) / 5.0
            let g = Double((n / 6) % 6) / 5.0
            let b = Double(n % 6) / 5.0
            return Color(red: r, green: g, blue: b)
        }
        let v = Double((i - 232) * 10 + 8) / 255.0
        return Color(white: min(1, v))
    }
}

/// Small icon button with an iOS-quality hover state (used by the find bar etc.).
struct HoverIconButton: View {
    var systemImage: String
    var help: String
    var fg: Color
    var dim: Color
    var action: () -> Void
    @State private var hovering = false
    @State private var pressed = false

    var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(hovering ? fg : dim)
                // Measured: at 10pt semibold these SF Symbols render 1 device px
                // BELOW the row's text baseline centre (header label + session
                // chip both sat at 684.5, the glyphs at 685.5 on a 2x display).
                // Shift the glyph only — the frame and hover capsule stay put.
                .offset(y: -0.5)
                .frame(width: 22, height: 20)
                .background(
                    Capsule(style: .continuous)
                        .fill(fg.opacity(hovering ? 0.10 : 0))
                )
                .scaleEffect(pressed ? 0.92 : 1)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
        .onHover { hovering = $0 }
        .animation(.snappy(duration: 0.15), value: hovering)
        .animation(.snappy(duration: 0.12), value: pressed)
        .simultaneousGesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in pressed = true }
                .onEnded { _ in pressed = false }
        )
    }
}

private struct JumpBarSegmentButton: View {
    var title: String
    var systemImage: String
    var isFile: Bool
    var isLast: Bool
    var accent: Color
    var fg: Color
    var dim: Color
    var action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                Image(systemName: systemImage)
                    .font(.system(size: 10))
                    .foregroundStyle(isFile ? accent : dim)
                Text(title)
                    .font(.system(size: 11, weight: isLast ? .semibold : .regular))
                    .foregroundStyle(isLast ? fg : dim)
                    .lineLimit(1)
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(
                Capsule(style: .continuous)
                    .fill(hovering ? fg.opacity(0.10) : Color.clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(.easeOut(duration: 0.1), value: hovering)
    }
}

private extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}
