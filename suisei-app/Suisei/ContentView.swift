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

private enum OverlayTextInput: Hashable {
    case find
    case palette
}

private enum GitWorkbenchCompactPage {
    case master
    case files
    case diff
}

private struct GitBranchRow: Identifiable {
    let id: Int
    let name: String
    let selected: Bool
    let current: Bool
    let remote: Bool
}




/// Production-oriented xei face: FrameDiff paint only; pointer lifecycle is editor-wide.
struct ContentView: View {
    @ObservedObject var engine: EngineBridge
    /// Observed separately from `engine` so the toolbar sees viewer state
    /// without the viewer state riding on the chrome's publish rate.
    @ObservedObject var viewerControls = EngineBridge.shared.viewerControls
    /// The window's shells. The docked strip's chip list lives here rather than
    /// in the engine — the processes are on this side now, and a list core
    /// cannot see is a list it cannot serve.
    @ObservedObject var shells = TerminalSessions.shared
    /// A player is a window session, not an audio pane. The pane is replaced
    /// whenever focus moves to another document; this object deliberately is
    /// not, so ordinary tab navigation does not stop the track or reset it.
    @StateObject private var audioPlayer = AudioPlayerModel()
    @FocusState private var focused: Bool
    @FocusState private var overlayTextInput: OverlayTextInput?
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow
    // Live panel sizes (@State). @AppStorage on every drag frame caused shake/ghosting.
    // Persist only when a resize gesture ends (see `persistPanelSizes`).
    @State private var navW: Double = 280
    /// The navigator width handed to the split view, frozen at launch.
    ///
    /// Separate from `navW` on purpose. `.navigationSplitViewColumnWidth` takes
    /// a value, not a binding, and re-reads it whenever that value changes — so
    /// pointing it at `navW` while `SplitColumnWidthReporter` writes `navW`
    /// from the splitter is a loop, with the drag on one side of it.
    @State private var navIdealWidth: CGFloat = 280
    /// The sidebar column's width RIGHT NOW, 0 while it is collapsed.
    ///
    /// Distinct from both of the above: `navW` is what to persist and
    /// `navIdealWidth` is what to launch with, while this one tracks the live
    /// splitter, mid-drag and mid-collapse. `topBar` uses it to keep the tab
    /// strip on the window's centreline rather than the detail column's.
    @State private var navLiveWidth: CGFloat = 280
    /// The sidebar width the TAB STRIP is allowed to see — the settled one.
    ///
    /// `navLiveWidth` is republished on every frame of an open/close animation
    /// and on every pixel of a splitter drag. Feeding that to the strip is
    /// rule H1's named failure (`SUISEI-TAB-STRIP-HOST.md`: "the strip's
    /// viewport width must not be read from a view that resizes when the
    /// sidebar does", and `navLiveWidth` is one of the four values listed
    /// there as already tried).
    ///
    /// It cannot simply be ignored either: a resized navigator has to move the
    /// corridor, or overflowing tabs end up underneath it. So the strip sees
    /// the width only once it has stopped moving — the animation produces one
    /// step at its end instead of twenty during it.
    @State private var navSettledWidth: CGFloat = 280
    @State private var navSettleTask: Task<Void, Never>?
    /// The last width the navigator actually settled at while open.
    ///
    /// The predictor for a reopen, and it has to be the number the reporter
    /// will publish when the animation ends — not `navW`, which is a saved
    /// preference and floored at 240. Predicting a value a point or two off
    /// meant the settle pass committed a second, slightly different width
    /// after the first travel had finished: opening animated twice, closing
    /// did not, because closing's target is exactly zero.
    @State private var navOpenWidth: CGFloat = 280
    /// The first report is committed without waiting: a restored split has to
    /// be respected before the first frame, or the opening tabs sit under a
    /// navigator wider than the bootstrap value.
    @State private var navSettledOnce = false
    @State private var termW: Double = 400
    @State private var debugAreaH: Double = 200
    @State private var inspectorW: Double = 240
    @State private var gitWorkbenchMasterW: Double = 336
    @State private var gitWorkbenchContextW: Double = 288
    @State private var gitWorkbenchCompactPage: GitWorkbenchCompactPage = .master
    @State private var gitWorkbenchContextVisible = false
    @State private var showGitNewBranchDialog = false
    @State private var gitNewBranchName = ""
    @State private var showGitBranchDeletionConfirmation = false
    @State private var pendingGitBranchDeletion = ""
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
    /// Tab currently held by a drag, nil otherwise.


    /// Width of the "+" button's slot.
    /// One shared height for jump bars and navigator selection chrome.
    static let editorHeaderHeight: CGFloat = 26



    /// Dispatch a chip click through the model (spec §3).
    ///
    /// Replaces `selectTabChip`'s three-way branch, whose
    /// `isLayoutDeskActive || editorSplit.isSplit` condition was a guess
    /// standing in for an undecided rule. The rule is decided now, so the
    /// branch is a lookup.
    private func applyStripClick(chip stableId: UInt64) {
        switch stripModel.click(chip: stableId) {
        case .focusDocument(let id):
            engine.gotoTabId(id)
        case .focusMember(let id, let group):
            // "Focus within the layout" has to say what happens when the
            // layout is not the arrangement currently on screen. I read the
            // spec's "arrangement unchanged" as plain `gotoTabId` and deleted
            // the branch below, calling it a guess. It was not a guess — it
            // carries two constraints, and dropping it broke both:
            //
            // * a parked layout must be INSTALLED, or its split never appears;
            // * installing sets `App::active_layout`, and `unfold_layout` bails
            //   on `active_layout == None` — so grouped ⇄ loose stopped working
            //   entirely (suisei-core/src/layouts.rs).
            //
            // The one case that must NOT activate is a free multi-pane split
            // the user built themselves: replacing it with the parked tree
            // discards their arrangement.
            if engine.activeLayoutId == group {
                // Already the arrangement on screen: move focus, touch nothing.
                engine.gotoTabId(id)
            } else if engine.editorSplit.isSplit && !engine.isLayoutDeskActive {
                // A split the user built themselves. Installing a parked tree
                // over it would discard their arrangement.
                engine.gotoTabId(id)
            } else {
                // Nothing of this layout's is on screen — including the case
                // where a DIFFERENT layout owns the desk. Install it.
                //
                // This used to read `isLayoutDeskActive || isSplit`, which
                // only asked whether SOME layout was active. With a merged
                // layout A on screen, clicking a member of a grouped layout
                // (B C) took the focus-in-place branch; core then saw a target
                // outside the active layout, parked A, and collapsed the desk
                // to one pane. So the first click showed B alone and the
                // SECOND click — with no layout active by then — finally
                // installed the split. Which layout is active is the question,
                // not whether one is.
                engine.activateLayout(group, focusDoc: id)
            }
        case .activateLayout(let id):
            engine.activateLayout(id)
        case nil:
            // A chip that is not on the strip — while merged, a member id lands
            // here. Opaque means opaque: do nothing rather than guess.
            break
        }
    }

    /// Dispatch a close through the model (spec §5).
    private func applyStripClose(chip stableId: UInt64) {
        switch stripModel.close(chip: stableId) {
        case .closeDocument(let id):
            engine.closeTabId(id)
        case .dropLayout(let id):
            engine.dropLayout(id)
        case nil:
            break
        }
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
    /// Split divider drag override (committed to Core on release).
    /// Minimap toggle (View menu).
    @AppStorage("suisei.minimap") private var minimapEnabled = true
    /// Show a minimap in every split pane, not only the focused one.
    ///
    /// Off by default because the minimap follows the focused pane, and one
    /// strip that moves is less to read than four that do not. On, it is a
    /// per-pane overview — closer to how VS Code behaves in a split.
    @AppStorage("suisei.minimap.allPanes") private var minimapAllPanes = false
    /// Scale the strip with the pane instead of pinning it to 62pt.
    ///
    /// A fixed strip is a fixed fraction of a wide window and half a narrow
    /// pane. Proportional keeps the ratio; the clamp keeps it legible at one
    /// end and out of the way at the other.
    @AppStorage("suisei.minimap.proportional") private var minimapProportional = false
    /// Editor area in points, from the one `GeometryReader` that already
    /// measures it. A proportional minimap needs its pane's width, and a
    /// pane's width is this times `pane.rect.width` — exact, and cheaper than
    /// a second reader per pane.
    @State private var editorAreaSize: CGSize = .zero
    /// Live window-resize HUD (blur + dimensions).
    @State private var isLiveResizing = false
    @State private var liveResizeSize: CGSize = .zero
    /// Measured tab-chip row width (exact centering in the titlebar).
    /// Slot under the pointer, from the SAME hit test clicks use.
    ///
    /// Not each chip's own `.onHover`: that is SwiftUI's hit-testing, a second
    /// authority that disagreed with the click path — hovering one chip
    /// highlighted and selected another. Spec §4 requires one lookup.
    /// `NavigationSplitView` cannot reliably reverse a column-removal
    /// transition in place. A second toggle during the 0.25s flight used to
    /// leave its internal splitter at an intermediate width while
    /// `uiNavVisible` already said the column was open. Finish the current
    /// native transition, then apply only the latest requested destination.
    @State private var navigatorTransitionInFlight = false
    @State private var queuedNavigatorVisibility: Bool?
    @State private var navigatorTransitionGeneration: UInt64 = 0
    /// Horizontal scroll position of the chip run.
    ///
    /// A `StateObject`, not `@State`: this changes on every wheel tick, and as
    /// a published property on a small object it invalidates only what reads
    /// it. It replaces the `ScrollView` that used to own this privately and
    /// report it back a pass late as `chipRowOrigin`.
    /// The strip's entry model — see docs/SUISEI-TAB-STRIP-BEHAVIOUR.md.
    private var stripModel: TabStripModel { TabStripModel(tabs: engine.chrome.tabs) }
    /// Retains the strip's menu blocks for as long as a menu can be open.
    @State private var stripMenus = TabStripMenuTarget()
    /// Background code-file warm-up for the project's master directory.
    @StateObject private var projectIndex = ProjectIndex()
    /// Measured shell-chip row width — the header scroller hugs it until the
    /// chips outgrow the cap, so a single session sits right beside the "+".
    @State private var terminalChipsWidth: CGFloat = 0
    @State private var debugTab: DebugAreaTab = .terminal

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
        /// Logic sits next to Outline on purpose: they answer neighbouring
        /// questions — where things are, and what happens — and the rail is
        /// where you look while reading the code either way.
        case outline, logic, file, quickHelp
        var systemImage: String {
            switch self {
            case .outline: return "list.bullet.indent"
            // A record: the groove and the label at the middle. NOT a branch
            // glyph — the navigator's Source Control tab is already a branch,
            // and two rails on two sides of one window showing the same
            // symbol for different things is the rail saying nothing.
            case .logic: return "smallcircle.filled.circle"
            case .file: return "doc"
            case .quickHelp: return "questionmark.circle"
            }
        }
        var title: String {
            switch self {
            case .outline: return "Outline"
            case .logic: return "Logic"
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

    /// The editor's own surface — the palette's.
    ///
    /// This was `.textBackgroundColor`, the system's content surface, resolved
    /// against the theme's light/dark. The argument for it was that AppKit
    /// tracks Increase Contrast and an authored constant cannot, and that
    /// taking the surface from the platform makes the editor and the workbench
    /// one app. Both true, and both beside the point once the palette is
    /// something other than Light or Dark: Catppuccin authors #1E1E2E, and
    /// under that rule the editor painted the system's grey while the syntax
    /// on top of it was Catppuccin's. Choosing a theme changed the words and
    /// not the page.
    ///
    /// Light and Dark still look like the platform, because their `editor_bg`
    /// was authored to. The difference is that now that is a property of those
    /// two palettes rather than a rule imposed on all thirteen.
    private var editorBg: Color { theme.color(theme.editorBg) }

    private static func resolve(_ color: NSColor, light: Bool) -> NSColor {
        // Through `themedAppearanceName`, so a resolved surface still tracks
        // Increase Contrast. Naming `.aqua`/`.darkAqua` here froze every
        // resolved colour at its normal-contrast value — which would have made
        // "we use semantic colours so they follow the accessibility settings"
        // untrue of the surfaces that go through this function.
        let appearance = NSAppearance(
            named: WindowChrome.themedAppearanceName(light: light)
        )
        var out = color
        appearance?.performAsCurrentDrawingAppearance {
            out = color.usingColorSpace(.sRGB) ?? color
        }
        return out
    }

    /// GUI contrast boost: TUI colors wash out on Retina; push fg/dim for readability.
    private var isLightTheme: Bool { theme.isLight }
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

    /// Text that stays readable on a filled `accent`.
    ///
    /// The accent is the user's, and Appearance offers Yellow — white on
    /// `#FFD60A` is barely a colour difference. Same rule the Git workbench
    /// already uses for its accent-filled chips.
    private var accentForeground: Color {
        let packed = theme.accent
        let r = Double((packed >> 16) & 0xFF) / 255.0
        let g = Double((packed >> 8) & 0xFF) / 255.0
        let b = Double(packed & 0xFF) / 255.0
        return (0.2126 * r + 0.7152 * g + 0.0722 * b) > 0.58 ? .black : .white
    }

    /// The theme, handed to the non-text viewers, which live outside this file
    /// and so cannot reach `theme` themselves.
    private var viewerPalette: ViewerPalette {
        ViewerPalette(
            fg: fg, dim: dim, accent: accent, bg: editorBg,
            stage: theme.color(theme.modelBg))
    }

    /// The File tab is on screen right now — so the ⓘ button is showing
    /// something, and its next press closes it.
    private var showingFileInspector: Bool {
        engine.uiInspectorVisible && inspectorMode == .file
    }

    /// The compact transport belongs in the trailing native toolbar only when
    /// the full transport is not already visible in an editor pane.
    private var showsCompactNowPlaying: Bool {
        guard audioPlayer.engaged, !audioPlayer.sourcePath.isEmpty else { return false }
        guard !engine.preview.open else { return true }
        let source = URL(fileURLWithPath: audioPlayer.sourcePath).standardizedFileURL.path
        return !engine.editorSplit.panes.contains {
            $0.kind == .audio
                && URL(fileURLWithPath: $0.path).standardizedFileURL.path == source
        }
    }

    /// Whether the unsplit island gets a minimap.
    ///
    /// The minimap is a picture of a text document's shape. A pane holding a
    /// shell, an image, a PDF or an audio file has no such shape — and the
    /// strip was drawn over them regardless, because it lives at the island
    /// level where it never had to know what the pane was showing. On the
    /// audio pane it landed straight across the inspector and cut every value
    /// in half.
    ///
    /// The split path never had this: there the minimap hangs off
    /// `editorSurface`, which only exists in the text branch. This is the same
    /// condition, said where the island can hear it.
    private var islandShowsMinimap: Bool {
        guard minimapEnabled, !engine.editorSplit.isSplit, !engine.preview.open else {
            return false
        }
        // No pane at all is the empty editor, which is still text.
        return (engine.editorSplit.panes.first?.kind ?? .text) == .text
    }
    /// The minimap's width when it is not scaled to the pane. Also its ceiling
    /// when it is: proportional makes the strip narrower in a narrow pane, and
    /// never wider than the fixed setting would have been. Fixed is the widest
    /// the minimap gets.
    private static let minimapFixedWidth: CGFloat = 62
    /// Floor of the proportional strip. Below this the thumbnail stops
    /// resembling the code it is a picture of.
    private static let minimapMinWidth: CGFloat = 44

    /// How wide the minimap is over a pane this wide.
    private func minimapWidth(paneWidth: CGFloat) -> CGFloat {
        guard minimapProportional, paneWidth > 0 else { return Self.minimapFixedWidth }
        return min(Self.minimapFixedWidth, max(Self.minimapMinWidth, paneWidth * 0.12))
    }

    /// Points of a pane's right edge the minimap is drawn over, so the caret
    /// reveal can scroll past it. Zero when this pane has no strip.
    ///
    /// The minimap is an overlay: it does not take the space, it covers it. So
    /// nothing in AppKit's geometry knows the caret can be behind it, and
    /// `scrollToVisible` has to be told.
    private func minimapInset(for pane: EditorPaneSnap) -> CGFloat {
        guard minimapEnabled, pane.kind == .text,
              pane.focused || minimapAllPanes
        else { return 0 }
        return minimapWidth(paneWidth: editorAreaSize.width * pane.rect.width)
    }
    private var gutterFg: Color { isLightTheme ? Color.black.opacity(0.32) : dim.opacity(0.9) }
    /// The theme's own current-line wash.
    ///
    /// This band was already being drawn; its colour was a hard-coded black or
    /// white at 3.5%/5.5%, so every one of the fifteen palettes got the same
    /// grey wash and the theme had no say. Core carries `current_line` now, so
    /// the palette decides and the Themes page can edit it — the same shape as
    /// `relative_number` and the wrap ratio before it: the fact existed and had
    /// never crossed the ABI.
    private var cursorLineBg: Color { theme.color(theme.currentLine) }
    private var selBg: Color {
        theme.color(theme.selection).opacity(isLightTheme ? 0.45 : 0.55)
    }
    private var caretColor: Color { theme.color(theme.caret) }

    var body: some View {
        // Welcome is its own Window scene (SuiseiApp) with system chrome.
        // This view is the editor shell only.
        //
        // Source Control's structure, adopted for a measured reason rather than
        // an aesthetic one.
        //
        // On macOS 26 the sidebar's material is produced by
        // `NavigationSplitView` itself. Dumping the AppKit tree of a live split
        // view (probe, 2026-08-10) shows the first column wrapped in a
        // `BackdropView` backed by a `CABackdropLayer`, and inside that an
        // `NSContainerConcentricGlassEffectView` inset 8pt on every side. The
        // system already draws the floating, rounded, inset card this file used
        // to draw by hand — and only the system's copy sits on the backdrop.
        //
        // So the hand-drawn card was never an alternative to the split view. It
        // was an opaque `.background(editorBg)` painted directly over the one
        // surface the material lives on, which is why four attempts to give it
        // the workbench's material failed: each added a material in FRONT of
        // the thing that was covering it.
        //
        // The same probe retires the other half of the old story. The backdrop
        // appears with a plain `ScrollView` sidebar exactly as it does with
        // `List(.listStyle(.sidebar))` — that list style governs row metrics,
        // not the surface. `ProjectTreeView` keeps its custom rows and still
        // gets the material.
        ZStack(alignment: .top) {
            NavigationSplitView(columnVisibility: navigatorVisibility) {
                sidebarColumn
            } detail: {
                detailStack
            }

            // Keep the document strip in the window root coordinate space.
            // `4373153` owned it here; moving it into the split view's detail
            // host in `d471ba5` was the first commit where the drawn chips and
            // the AppKit event catcher acquired different origins. The native
            // split sidebar remains intact, but tab placement and pointer input
            // once again share the window-level host that made the stable
            // version work.
            topBar
                .ignoresSafeArea(.container, edges: .top)
                .zIndex(2)

            // The palette centres on the WINDOW, which is why it is here and
            // not on `detailStack`.
            //
            // It used to be an overlay on the detail column, so "centred" meant
            // centred on the editor — right of the navigator, and then shoved
            // another `inspectorReserved / 2` sideways by hand. Two corrections
            // for one number, and neither of them the window. Spotlight, Open
            // Quickly and every palette on this platform centre on the window;
            // at the root that is what centring already means, with nothing to
            // correct.
            if engine.chrome.palette.open {
                paletteOverlay
                    .ignoresSafeArea()
                    .zIndex(100)
                    // Removal is IMMEDIATE (.identity): an animated removal
                    // could wedge mid-transition, leaving an invisible view
                    // that swallowed every click and hover in the top band
                    // until some other state change re-evaluated the tree (the
                    // "Esc from palette kills the right-side buttons" bug).
                    .transition(.asymmetric(
                        insertion: .opacity.combined(
                            with: .scale(scale: 0.94, anchor: .center)
                        ),
                        removal: .identity
                    ))
            }
        }
        // A short spring with a little bounce, which is what a panel arriving
        // over a scrim does on this platform. The previous curve was a plain
        // 0.22s ease on a scale of 0.98 — two percent, under an opaque glass
        // panel, and the reported result was that there was no animation at
        // all. There was; it could not be seen.
        .animation(.snappy(duration: 0.26, extraBounce: 0.08),
                   value: engine.chrome.palette.open)
        // Keep the split geometry stable while the navigator opens and closes.
        // The editor's *backing plane* continues below the native sidebar; its
        // usable content still starts at the splitter, just as it does in
        // Xcode. Overlay-style column sizing moved the titlebar, inspector and
        // editor viewport as a unit and broke their established alignment.
        .navigationSplitViewStyle(.balanced)
        // The navigator's show/hide motion.
        //
        // I removed this when the root became a split view, on the belief that
        // a column animates its own collapse. It does not, when the visibility
        // change arrives from a binding written outside a transaction — so the
        // sidebar snapped.
        //
        // An IMPLICIT animation keyed on the value, which is the shape
        // `animatingPanels` argues for and the inspector has always used: it
        // deliberately runs no `withAnimation`, because an explicit transaction
        // plus this modifier is two animators for one value, and that is what
        // used to make the navigator stutter where the inspector never did.
        .animation(.snappy(duration: 0.25), value: engine.uiNavVisible)
        // The strip learns where the sidebar is GOING, at the moment it is
        // told to go there.
        //
        // Waiting for `SplitColumnWidthReporter` to settle means waiting for
        // the animation to finish and then some, so the strip started moving
        // only once the sidebar had stopped — two runs in sequence, and the
        // second one arriving as a jump. The destination is known at the
        // toggle: shut, or the navigator's own width. Committing it here lets
        // the two move together, and it is still ONE change to the input
        // rather than one per frame.
        .onChange(of: engine.uiNavVisible) { _, visible in
            navSettleTask?.cancel()
            navSettledOnce = true
            // The measured width, not the saved preference — see `navOpenWidth`.
            let target: CGFloat = visible
                ? (navOpenWidth > 1 ? navOpenWidth : CGFloat(navW))
                : 0
            if abs(navSettledWidth - target) > 0.5 { navSettledWidth = target }
        }
        // An EMPTY title, not a removed one. This is the whole reason the
        // toolbar items sit at the trailing edge — see `editorToolbar`.
        //
        // The title item is what anchors a split view's `.primaryAction` group
        // to the right. Take it away and the group collapses leftward. Both
        // ways of taking it away do it: `.toolbar(removing: .title)` and
        // `window.titleVisibility = .hidden` each move the glass platter from
        // x 878…1270 to x 98…490 in a 1280pt window, measured one factor at a
        // time against Source Control's exact declaration
        // (`scripts/sidebar_probe10.swift`). This file had BOTH.
        //
        // An empty string keeps the anchor and draws nothing, which is what
        // this row needs: Source Control wants its title visible, and this
        // window has a tab strip running through that space.
        .navigationTitle("")
        // The generated toggle follows the split divider. The fixed titlebar
        // slot below uses SwiftUI's native glass button instead, immediately
        // after the traffic lights.
        .toolbar(removing: .sidebarToggle)
        .toolbar { editorToolbar }
        .frame(minWidth: 640, minHeight: 400)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .foregroundStyle(.primary)
        .tint(accent)
        // Chrome UI uses a fixed size — Cmd+/- only zooms the editor canvas (EditorMetrics).
        .font(.system(size: 13, weight: .regular))
        .preferredColorScheme(isLightTheme ? .light : .dark)
        // Keys route through the NSEvent monitor (EngineBridge); a container-level
        // .focusable()/.onKeyPress here double-captured input and stole focus
        // from native text fields (the tree Filter couldn't be typed into).
        .focused($focused)
        // The editor is the window-wide backing surface. The navigator's
        // native material is layered over it, so its backdrop samples the same
        // editor plane that continues into the detail column. Painting the
        // chrome colour here made the navigator and renderer two unrelated
        // islands even when their frames touched.
        .background(editorBg.ignoresSafeArea())
        // Source Control's chrome, verbatim. It re-applies on every SwiftUI
        // update, which is what stops AppKit from quietly restoring an opaque
        // titlebar band, and it un-hides the REAL traffic lights — the split
        // view runs the sidebar up under them.
        .background(
            ThemedWindowChrome(
                // The palette's floor. The navigator's material is translucent
                // and samples what is behind it, so this is the colour the
                // sidebar ends up being — set it to the editor's surface and
                // the sidebar stops being a distinct surface at all.
                //
                // The tab bar matching the editor is not this window's
                // background's job: the titlebar is transparent and shows the
                // content under it (`clearTitlebarMaterial`), so the band over
                // the editor is the editor and the band over the navigator is
                // the navigator.
                background: NSColor(theme.windowBg),
                light: isLightTheme,
                identifier: WindowChrome.editorIdentifier,
                opaque: true
            )
        )
        .background(EditorWindowChrome())
        .background(EditorGeneratedSidebarTogglePruner())
        .background(
            EditorNavigatorToolbarAccessory(isVisible: engine.uiNavVisible) {
                toggleNavigatorVisibility()
            }
        )
    }

    /// The document controls, as real toolbar items — Source Control's toolbar,
    /// same shape: plain `Button { Image(systemName:) }` with a `.help`, and no
    /// styling of our own anywhere.
    ///
    /// That absence is the point. These three were hand-drawn `ToolbarPlainIcon`
    /// buttons in the titlebar row, and they could not look like the workbench's
    /// because the workbench's look is not something it draws — macOS 26 wraps
    /// toolbar items in an `NSToolbarPlatterView` holding an `NSGlassEffectView`
    /// and groups them into one capsule. Measured
    /// (`scripts/sidebar_probe4.swift`): a 112×36 platter for three items.
    /// Nothing short of being a toolbar item gets that.
    ///
    /// Three things had to be true before this was safe to adopt, and all three
    /// were measured rather than assumed (`sidebar_probe5/6.swift`):
    ///
    /// * the toolbar does NOT swallow clicks over the tab strip — `hitTest` at
    ///   the strip's centre reaches the SwiftUI content view with the toolbar
    ///   present, exactly as without it;
    /// * it does not move `topBar`, which stays at 0–48 either way;
    /// * it moves the traffic lights from 16pt to 26pt below the window top —
    ///   which is `topBandHeight / 2 + titlebarDrop`, i.e. the tab row's own
    ///   optical centreline. The lights and the tabs now share a line for the
    ///   first time without anyone positioning them.
    ///
    /// The tab strip itself stays SwiftUI content, for the reason it always
    /// has: it must be blurable and coverable, which a toolbar item is not.
    @ToolbarContentBuilder
    private var editorToolbar: some ToolbarContent {

        if showsCompactNowPlaying {
            ToolbarItemGroup(placement: .primaryAction) {
                Button { audioPlayer.skip(-10) } label: {
                    Image(systemName: "gobackward.10")
                }
                .help("10초 뒤로")

                Button { audioPlayer.toggle() } label: {
                    Image(systemName: audioPlayer.playing ? "pause.fill" : "play.fill")
                }
                .help(audioPlayer.playing ? "일시정지" : "재생")

                Button { audioPlayer.skip(10) } label: {
                    Image(systemName: "goforward.10")
                }
                .help("10초 앞으로")

                CompactNowPlayingIdentity(model: audioPlayer) {
                    _ = engine.openPath(audioPlayer.sourcePath)
                    focused = true
                }

                Button { audioPlayer.muted.toggle() } label: {
                    Image(systemName: audioPlayer.muted ? "speaker.slash.fill" : "speaker.wave.2.fill")
                }
                .help(audioPlayer.muted ? "음소거 해제" : "음소거")
            }
        }

        // The focused viewer pane's controls, when there is one.
        //
        // These are here rather than drawn inside the pane because this is the
        // only place they can be real toolbar items, and being one is the
        // whole of the look — see the paragraph above about
        // `NSToolbarPlatterView`. It is also where Preview keeps the same
        // buttons: its zoom controls are in the window's toolbar, not floating
        // over the page. `ToolbarItemGroup` puts the run in one platter, which
        // is the grouping a hand-built bar was imitating.
        if viewerControls.kind != nil {
            if viewerControls.canZoom {
                ToolbarItemGroup(placement: .primaryAction) {
                    Button {
                        viewerControls.perform?(.zoomOut)
                    } label: {
                        Image(systemName: "minus.magnifyingglass")
                    }
                    .help("축소")

                    if !viewerControls.zoomLabel.isEmpty {
                        Text(viewerControls.zoomLabel)
                            .font(.system(size: 11, weight: .medium).monospacedDigit())
                            .foregroundStyle(.secondary)
                            .frame(minWidth: 38)
                    }

                    Button {
                        viewerControls.perform?(.zoomIn)
                    } label: {
                        Image(systemName: "plus.magnifyingglass")
                    }
                    .help("확대")

                    // What this button MEANS is the viewer's business — fit
                    // for an image, fit for a PDF, default size for audio —
                    // so it supplies the glyph too.
                    Button {
                        viewerControls.perform?(.reset)
                    } label: {
                        Image(systemName: viewerControls.resetSymbol)
                    }
                    .help(viewerControls.resetHelp)
                }
            }

            if !viewerControls.pageLabel.isEmpty {
                ToolbarItem(placement: .primaryAction) {
                    Text(viewerControls.pageLabel)
                        .font(.system(size: 11, weight: .medium).monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }

            // Reveals the facts in the inspector that already exists rather
            // than in a column of the viewer's own.
            //
            // Three states, not two. Shut → open it on the File tab. Open on
            // some other tab → switch, and do NOT close: the panel is showing
            // something else, so the press means "show me the file". Open and
            // already on File → close, because at that point the press cannot
            // mean anything else.
            ToolbarItem(placement: .primaryAction) {
                Button {
                    if engine.uiInspectorVisible, inspectorMode == .file {
                        animatePanels { engine.uiInspectorVisible = false }
                    } else {
                        if !engine.uiInspectorVisible {
                            animatePanels { engine.uiInspectorVisible = true }
                        }
                        inspectorMode = .file
                    }
                    focused = true
                } label: {
                    Image(systemName: showingFileInspector ? "info.circle.fill" : "info.circle")
                }
                .help(showingFileInspector ? "파일 정보 닫기" : "파일 정보")
            }
        }

        // Nothing here places these at the trailing edge, and nothing needs to.
        // `.navigationTitle("")` on the split view does it, by keeping the
        // title item that anchors the `.primaryAction` group — see there.
        //
        // Four earlier attempts assumed the placement had to be declared, and
        // all four were wrong in the same way: `.primaryAction`,
        // `.confirmationAction` and `ToolbarItemGroup` all landed at x 298…410
        // in a 1280pt window, and `ToolbarSpacer(.flexible)` measured correct
        // twice (x 1158…1270, once in a probe mirroring this view's whole
        // modifier chain) while doing nothing at all in the app. None of that
        // was a placement problem. The items were packed left because the
        // window had lost its title item, and this file had removed it twice
        // over — `.toolbar(removing: .title)` and `titleVisibility = .hidden`.
        ToolbarItem(placement: .primaryAction) {
            Button {
                engine.openFilePalette()
            } label: {
                Image(systemName: "magnifyingglass")
            }
            .help("Go to File · ⌘P")
        }

        ToolbarItem(placement: .primaryAction) {
            Button {
                engine.openSettings()
                openWindow(id: "settings")
            } label: {
                Image(systemName: "gearshape")
            }
            .help("Settings · ⌘,")
        }

        // A Button, not a Toggle — deliberately, and this is the one place in
        // the toolbar that does NOT take the system's default.
        //
        // A `Toggle(.button)` draws the accent-filled selected state whenever
        // the inspector is open, which reads as "this control is on" rather
        // than "this control opens a panel". Its counterpart across the window,
        // the split view's own sidebar toggle, is a plain button and stays
        // unfilled at every state — so a filled one here would make the two
        // sides of the same gesture disagree.
        //
        // The state is still visible, on the thing the state belongs to: the
        // inspector column is either there or it is not.
        ToolbarItem(placement: .primaryAction) {
            Button {
                animatePanels { engine.uiInspectorVisible.toggle() }
                focused = true
            } label: {
                Image(systemName: "sidebar.right")
            }
            .help("Outline · ⌥⌘0")
        }
    }

    /// One authority for "is the navigator showing" — Core's flag — restated in
    /// the split view's own vocabulary. A second, SwiftUI-owned visibility
    /// state that could disagree with `uiNavVisible` is exactly the shape of
    /// defect this file keeps hitting, so there isn't one.
    private var navigatorVisibility: Binding<NavigationSplitViewVisibility> {
        Binding(
            get: { engine.uiNavVisible ? .all : .detailOnly },
            set: { visibility in
                let visible = visibility != .detailOnly
                requestNavigatorVisibility(visible)
            }
        )
    }

    /// Toggle against the latest requested destination, not merely the model's
    /// already-written Bool. That makes three rapid clicks close→open→close
    /// instead of treating both later clicks as duplicate requests to open.
    private func toggleNavigatorVisibility() {
        let latest = queuedNavigatorVisibility ?? engine.uiNavVisible
        requestNavigatorVisibility(!latest)
    }

    private func requestNavigatorVisibility(_ visible: Bool) {
        if navigatorTransitionInFlight {
            queuedNavigatorVisibility = visible
            return
        }
        guard visible != engine.uiNavVisible else { return }

        // Content FIRST, animation second. `applyNavMode` runs a full engine
        // recompose and chrome pull; doing it after the visibility write hitches
        // the opening animation's first frame.
        if visible { applyNavMode(navMode) }

        navigatorTransitionInFlight = true
        navigatorTransitionGeneration &+= 1
        let generation = navigatorTransitionGeneration
        engine.animatingPanels { engine.uiNavVisible = visible }
        focused = true

        // The visible curve is 0.25s, but AppKit does not commit the removed
        // split column at the curve's nominal end. EngineBridge deliberately
        // keeps its resize transaction open through 0.36s for that tail. Wait
        // for the same settle boundary plus one display frame before asking
        // NavigationSplitView to travel in the other direction; reopening at
        // 0.28s could resurrect a logically-visible column at 3pt wide.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.38) {
            guard generation == navigatorTransitionGeneration else { return }
            navigatorTransitionInFlight = false
            let queued = queuedNavigatorVisibility
            queuedNavigatorVisibility = nil
            guard let queued, queued != engine.uiNavVisible else { return }
            requestNavigatorVisibility(queued)
        }
    }

    /// Everything right of the navigator: the titlebar row, the editor island,
    /// the inspector, the status bar, and the overlays that must centre on the
    /// EDITOR rather than on the window.
    ///
    /// Centring is why the overlays live here. The palette used to hang off the
    /// window and correct itself by `(navReserved - inspectorReserved) / 2`,
    /// hand-measured against both panels. Inside the detail column that
    /// arithmetic is just the column's geometry, so only the inspector — still
    /// ours, still inside this column — needs compensating.
    private var detailStack: some View {
        ZStack(alignment: .top) {
            // Bottom z-layer: the status bar spans the detail column edge to
            // edge and the inspector's opaque column covers its right end, so
            // the bar reads as one unbroken floor (the recurring "라인이 끊김").
            // It stops at the sidebar because the sidebar is a real column now
            // — Xcode's does the same.
            VStack(spacing: 0) {
                Spacer()
                statusLine
            }

            VStack(spacing: 0) {
                HStack(spacing: 0) {
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

        }
        // The row starts at the true window top. The titlebar is transparent
        // and its only occupant — the traffic lights — sits over the sidebar.
        .ignoresSafeArea(.container, edges: .top)
        .overlay {
            // Completions stay HERE. They are anchored to the caret, so the
            // editor column's coordinate space is the one they want. The
            // palette is not anchored to anything in this column and has moved
            // to the window root — see `body`.
            if engine.chrome.completions.open {
                completionsOverlay.zIndex(80)
            }
        }
        // An overlay that owns its keys has to actually be given them. Opening
        // one while the project tree's filter still holds first responder left
        // it deaf — see `reclaimKeyboardFromTextFields`. Keyed off the engine's
        // own open flags so every route in is covered (⌘P, the pane header +,
        // the menus), not just the call site that happened to be tested.
        .onChange(of: engine.chrome.palette.open) { _, open in
            if open { engine.reclaimKeyboardFromTextFields() }
        }
        .onChange(of: engine.chrome.search.open) { _, open in
            if open { engine.reclaimKeyboardFromTextFields() }
        }
        .onChange(of: engine.gitWorkbenchWindowOpen) { _, open in
            if open {
                openWindow(id: "git-workbench")
            } else {
                dismissWindow(id: "git-workbench")
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willStartLiveResizeNotification)) { note in
            guard let w = note.object as? NSWindow, isEditorWindow(w) else { return }
            engine.windowLiveResizing = true
            ResizeHudWindow.shared.show(over: w)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didResizeNotification)) { note in
            guard let w = note.object as? NSWindow, isEditorWindow(w) else { return }
            guard engine.windowLiveResizing else { return }
            ResizeHudWindow.shared.update(over: w)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { note in
            guard let w = note.object as? NSWindow, isEditorWindow(w) else { return }
            // Nothing to re-place: the traffic lights are AppKit's own, in
            // AppKit's own position, exactly as in Source Control.
            //
            // The Git model may stay open in its own window. Returning to an
            // editor window must nevertheless return keyboard ownership to the
            // editor instead of leaving Core in GitWorkbench mode.
            engine.ensureEditorFocus()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didEndLiveResizeNotification)) { note in
            guard let w = note.object as? NSWindow, isEditorWindow(w) else { return }
            engine.windowLiveResizing = false
            ResizeHudWindow.shared.hide()
            engine.settleEditorResize()
        }
        // Environment, sizing and the window surfaces all live on the root, so
        // the SIDEBAR inherits them too — see `body`.
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
        .modifier(AudioTabLifetimeModifier(engine: engine, player: audioPlayer))
        .modifier(TerminalTabLifetimeModifier(engine: engine))
        // Increase Contrast / Reduce Transparency change which appearance the
        // windows should carry, and this app pins that appearance rather than
        // inheriting it — so nothing else would notice. Same deferral as the
        // theme flip, for the same reason.
        .onReceive(
            NSWorkspace.shared.notificationCenter.publisher(
                for: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification
            )
        ) { _ in
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
        // The debugger's panel is on screen, told to core from ONE place.
        //
        // `dapSetPanel` used to be called only from the tab button, so it
        // tracked WHICH TAB rather than whether the dock was open — and
        // closing the dock with its ✕ left core believing the panel was up.
        // Everything gated on that flag stayed drawn: the stop band, the value
        // bracket, the hover value. Reported as "디버그 패널 닫아도 다 사라지지
        // 않음".
        //
        // Two inputs, one answer, and an observer on each so no path can
        // forget to make the call.
        .onChange(of: engine.dap.session) { _, live in
            // A session starting brings the debugger forward, and its ending
            // leaves it where it is. Pressing Start and getting the terminal
            // is the panel not answering what was asked; being thrown back to
            // the terminal the moment a program exits is the panel deciding
            // you are finished reading.
            guard live else { return }
            debugTab = .debug
            if !engine.uiDebugVisible {
                animatePanels { withAnimation(NavStrip.settle) { engine.uiDebugVisible = true } }
            }
        }
        // A build starting brings its panel forward, the way a debug session
        // does. Through the state rather than from the menu item, so every way
        // of starting one — menu, key, panel button — lands the same.
        .onChange(of: engine.build.state) { _, state in
            guard state == .running else { return }
            debugTab = .build
            if !engine.uiDebugVisible {
                animatePanels { withAnimation(NavStrip.settle) { engine.uiDebugVisible = true } }
            }
        }
        .onChange(of: engine.chrome.terminal.open) { _, open in
            // Docked terminal (⌃T) → Debug area. Pane terminals (⌃⇧T) are
            // pane-local content and never touch the debug strip.
            withAnimation(.snappy(duration: 0.28)) {
                if open {
                    // Bring the TERMINAL tab forward — the mirror of what a
                    // starting debug session does above, and the line that was
                    // missing. `debugTab` sticks at `.debug` once a session has
                    // run, so a shell opened afterwards started, took the
                    // keyboard and put the dock up STILL SHOWING THE DEBUGGER.
                    // Every keystroke went to a terminal that was not on
                    // screen: "사이드바에서 여는 터미널 여전히 입력 안됨".
                    debugTab = .terminal
                    engine.uiDebugVisible = true
                } else {
                    engine.uiDebugVisible = false
                }
            }
        }
        .onChange(of: engine.chrome.settings.open) { _, open in
            if open {
                openWindow(id: "settings")
            } else {
                dismissWindow(id: "settings")
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiOpenSettingsWindow)) { _ in
            // The command/toolbar sender already opened Core. This notification
            // only presents (or raises) the independent native window.
            openWindow(id: "settings")
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiOpenGitWorkbenchWindow)) { _ in
            engine.openGitWorkbenchWindow()
            openWindow(id: "git-workbench")
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiRevealLogicInspector)) { _ in
            // The File inspector's three states, and for the same reasons.
            // Shut → open on Logic. Open on something else → switch, do not
            // close: the press means "show me the logic". Open and already on
            // Logic → close, because at that point it cannot mean anything else.
            if engine.uiInspectorVisible, inspectorMode == .logic {
                animatePanels { engine.uiInspectorVisible = false }
            } else {
                if !engine.uiInspectorVisible {
                    animatePanels { engine.uiInspectorVisible = true }
                }
                inspectorMode = .logic
            }
            focused = true
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

    /// Width the inspector takes out of the detail column, 0 when hidden.
    ///
    /// The navigator's counterpart is gone: it is a split view column now, so
    /// nothing inside the detail has to know how wide it is, or whether it is
    /// there at all.
    private var inspectorReserved: CGFloat {
        outlineVisible ? CGFloat(inspectorW) + ContentView.panelGap : 0
    }

    private var outlineVisible: Bool {
        engine.uiInspectorVisible
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
        w.identifier == WindowChrome.editorIdentifier
    }

    /// Run a panel show/hide animation without per-frame engine resizes —
    /// the 240-row recompose at 30Hz read as stutter, especially on big files.
    /// The motion itself lives on the bridge so the menu commands share it.
    private func animatePanels(_ body: () -> Void) {
        engine.animatingPanels(body)
    }

    /// Let the strip see a sidebar width only after the sidebar has stopped.
    ///
    /// Each report restarts the wait, so a 0.25s open/close — which publishes
    /// a width every frame, and flips through the collapsed sentinel partway —
    /// commits exactly once, at the end. A splitter drag likewise commits when
    /// the pointer rests rather than on every pixel.
    ///
    /// 120ms: comfortably longer than a frame and shorter than any pause a
    /// user would read as the strip failing to follow.
    private func settleNavWidth(_ live: CGFloat) {
        navSettleTask?.cancel()
        guard navSettledOnce else {
            navSettledOnce = true
            commitNavWidth(live)
            return
        }
        guard abs(navSettledWidth - live) > 0.5 else {
            // Already where the strip thinks it is — nothing to move. Still
            // worth remembering, so the next reopen predicts this exact
            // number and needs no correcting step after it arrives.
            if live > 1 { navOpenWidth = live }
            return
        }
        navSettleTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 120_000_000)
            guard !Task.isCancelled else { return }
            if abs(navSettledWidth - live) > 0.5 { commitNavWidth(live) }
        }
    }

    private func commitNavWidth(_ live: CGFloat) {
        navSettledWidth = live
        if live > 1 { navOpenWidth = live }
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
            // Floor matches the split view column's own `min:` — a width
            // persisted under the old 200 floor would otherwise load back and
            // clip the navigator strip.
            let v = d.double(forKey: "suisei.panel.navW")
            if v >= 240 { navW = v }
        }
        // Read once, here, and never again: see `navIdealWidth`.
        navIdealWidth = CGFloat(navW)
        let t = d.double(forKey: "suisei.panel.termW")
        if t >= 200 { termW = t }
        // Key keeps its old name so a saved height survives the XLC removal.
        let x = d.double(forKey: "suisei.panel.xlcH")
        if x >= 100 { debugAreaH = x }
        let i = d.double(forKey: "suisei.panel.inspectorW")
        if i >= 140 { inspectorW = i }
        let gitMaster = d.double(forKey: "suisei.gitWorkbench.masterW")
        if gitMaster >= 280 { gitWorkbenchMasterW = min(480, gitMaster) }
        let gitContext = d.double(forKey: "suisei.gitWorkbench.contextW")
        if gitContext >= 260 { gitWorkbenchContextW = min(320, gitContext) }
    }

    private func persistPanelSizes() {
        let d = UserDefaults.standard
        d.set(navW, forKey: "suisei.panel.navW")
        d.set(termW, forKey: "suisei.panel.termW")
        d.set(debugAreaH, forKey: "suisei.panel.xlcH")
        d.set(inspectorW, forKey: "suisei.panel.inspectorW")
        d.set(gitWorkbenchMasterW, forKey: "suisei.gitWorkbench.masterW")
        d.set(gitWorkbenchContextW, forKey: "suisei.gitWorkbench.contextW")
    }

    // MARK: - Xcode-like shell (flat panes, native sidebar/inspector)

    /// Chrome base slightly separated from the editor fill.
    /// Everything around the editor — the same surface the Git workbench's
    /// window uses.
    ///
    /// It was `editorBg` darkened by 16%, so the shell sat further from the
    /// content than macOS puts it and the window read as higher contrast than a
    /// native one. Then it was `.windowBackgroundColor`, which fixed the
    /// direction — AppKit's chrome recedes by nearing the mid grey, not by
    /// going blacker — but answered with the SYSTEM's grey, so choosing
    /// Catppuccin repainted the code and left the frame around it macOS-grey.
    ///
    /// It is the palette's own floor now. Every theme authors one; `bg` was
    /// carried by all thirteen and read by nothing, which is precisely why the
    /// theme stopped at the text.
    private var shellBase: Color { theme.windowBg }

    /// Sidebar column — navigator strip on top, flat content below.
    ///
    /// **It must not paint a background.** The split view has already put a
    /// `CABackdropLayer` behind this column and an inset concentric-glass
    /// container around it; any opaque fill here covers both, which is what the
    /// hand-drawn card did for months. `dockedNavigator` and its rows follow
    /// the same rule — colour comes from `Color.primary` washes, never a second
    /// base.
    ///
    /// No `NSVisualEffectView` either, in any blending mode. Two were tried and
    /// both landed IN FRONT of the surface they were meant to be: a material
    /// added inside a column that already has one is the confused-hierarchy
    /// case the Liquid Glass guidance names.
    private var sidebarColumn: some View {
        PerfProbe.measure("  body.sidebarColumn") { sidebarColumnBody }
    }

    @ViewBuilder
    private var sidebarColumnBody: some View {
        VStack(spacing: 0) {
            // No top spacer. The column's safe area already clears the titlebar
            // row — the traffic lights AND the split view's own sidebar toggle
            // live there — and the strip carries its own 5pt beat below that.
            //
            // MEASURED (`scripts/sidebar_probe3.swift`, four configurations, in
            // a window styled exactly like this one): sidebar content with no
            // spacer starts at 42pt, and at 52pt once the window carries a
            // toolbar (`sidebar_probe6.swift`) — the safe area tracks the
            // titlebar row's real height on its own. The 35pt spacer that used
            // to be here came from the hand-drawn card, which had no safe area
            // to inherit; it pushed the strip to 77pt in the first build.
            navigatorModeStrip
            dockedNavigator
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        // Ideal, not a binding: `.navigationSplitViewColumnWidth` is consulted
        // when the column is first laid out, so feeding it a value the user is
        // actively dragging would fight the splitter. `navIdealWidth` is
        // therefore frozen at launch and `SplitColumnWidthReporter` writes the
        // dragged width back to `navW` for the NEXT launch.
        // (240 floor: five modes plus the detached toggle need the room —
        // see `navStripIcon`.)
        .navigationSplitViewColumnWidth(
            min: 240, ideal: navIdealWidth, max: 460
        )
        // The reporter reads the actual NSSplitView geometry. In prominent
        // detail mode the editor itself stays window-wide, but tabs and other
        // interactive content still need the live sidebar boundary so none of
        // them is left half-covered during a resize or collapse animation.
        .background {
            SplitColumnWidthReporter { width in
                let live = CGFloat(width)
                if abs(navLiveWidth - live) > 0.5 {
                    navLiveWidth = live
                }
                if live >= 240, abs(navW - Double(live)) > 0.5 {
                    navW = Double(live)
                }
                settleNavWidth(live)
            }
        }
    }

    /// Detail column: editor stage (+ outline card) + status line.
    private var detailColumn: some View {
        PerfProbe.measure("  body.detailColumn") { detailColumnBody }
    }

    @ViewBuilder
    private var detailColumnBody: some View {
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
        PerfProbe.measure("  body.inspectorColumn") { inspectorColumnBody }
    }

    @ViewBuilder
    private var inspectorColumnBody: some View {
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

    /// Editor and terminal as one surface.
    ///
    /// Only the leading edge differs from the old floating card: it is square
    /// and flush with the split boundary, while the window-wide `editorBg`
    /// below the native sidebar supplies the continuation underneath it. The
    /// established top, bottom and inspector-side spacing stays untouched.
    private var editorCard: some View {
        PerfProbe.measure("  body.editorCard") { editorCardBody }
    }

    @ViewBuilder
    private var editorCardBody: some View {
        let shape = UnevenRoundedRectangle(
            topLeadingRadius: 0,
            bottomLeadingRadius: 0,
            bottomTrailingRadius: ContentView.panelCornerRadius,
            topTrailingRadius: ContentView.panelCornerRadius,
            style: .continuous
        )
        return editorIsolatedStage
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background {
                ZStack(alignment: .bottom) {
                    editorBg
                    // The terminal's tint band lives on the CARD, spanning the
                    // full island, so both its fillets sweep the island walls.
                    if engine.uiDebugVisible {
                        dockedTerminalShape
                            .fill(terminalDockFill)
                            .overlay(
                                dockedTerminalShape.stroke(
                                    theme.separator.opacity(0.6),
                                    lineWidth: 1
                                )
                            )
                            .overlay(alignment: .top) {
                                // Full-width rule under the 28pt terminal
                                // header — beneath the navigator included.
                                Rectangle()
                                    .fill(theme.separator.opacity(0.6))
                                    .frame(height: 1)
                                    .offset(y: 28)
                            }
                            .frame(height: CGFloat(debugAreaH))
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    }
                }
            }
            .clipShape(shape)
            .overlay(shape.strokeBorder(theme.separator, lineWidth: 1))
            .shadow(color: theme.shadowInk.opacity(isLightTheme ? 0.13 : 0.34), radius: 5, y: 1)
            .padding(.vertical, ContentView.panelGap)
            .padding(.trailing, ContentView.panelGap)
    }

    /// The terminal's junction with the editor surface.
    private var dockedTerminalShape: DockedPanelShape {
        DockedPanelShape(fillet: ContentView.panelCornerRadius)
    }

    /// The terminal grid's own background — behind the glyphs AND behind the
    /// whole panel, so no sliver of another fill can show at its edges.
    /// The terminal grid is dark in **both** themes.
    ///
    /// It used to be the editor background nudged 3.5% toward black, which in
    /// the light theme is very nearly white — and core paints the shell's
    /// default foreground as rgb(200,200,200). Light grey on near-white is a
    /// contrast ratio of about 1.35:1, so the terminal rendered perfectly and
    /// could not be seen at all. The parser's SGR backgrounds are tints
    /// (`.opacity(0.45)`) meant to sit *over* a dark grid, which is the other
    /// half of the same assumption.
    ///
    /// A dark terminal inside a light editor is the norm — Xcode, VS Code and
    /// iTerm all do it — so this fixes the contrast without inventing a
    /// light-terminal palette core does not emit.
    private var terminalGridBg: Color {
        isLightTheme
            ? Color(red: 0.10, green: 0.11, blue: 0.13)
            : mixColor(editorBg, .black, 0.18)
    }

    /// Default ink for the terminal grid — and the colour of its block caret.
    ///
    /// This used to be the *editor's* foreground, which in the light theme is
    /// near-black: a dark caret on a dark terminal, which is why the shell
    /// appeared to have no cursor at all. It has to match the grid it sits on,
    /// not the editor beside it.
    private var terminalGridFg: Color {
        Color(red: 0.85, green: 0.86, blue: 0.88)
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
    private var terminalDockFill: Color {
        // Only wear the grid's colour when a grid is actually there. An empty
        // dock painted terminal-dark left its "Open Terminal · ⌃T" prompt as
        // `.tertiary` ink on near-black — legible in the light theme it was
        // designed against, invisible once the grid went dark.
        showingTerminalGrid ? terminalGridBg : editorBg
    }

    /// Whether the dock is CURRENTLY showing a terminal grid.
    ///
    /// Not `terminal.open`, which only says a shell exists somewhere. The dock
    /// has two tabs and they want opposite colours: the grid is deliberately
    /// dark even in a light theme (Xcode, VS Code and iTerm all do that), while
    /// the debugger is an ordinary panel whose ink comes from the editor
    /// palette. Asking whether a shell exists painted the whole dock
    /// terminal-dark while the Debug tab was showing, so the debugger drew its
    /// near-black editor ink on near-black — reported as "라이트모드인데
    /// 이거 왜이래".
    ///
    /// Same shape as two focus bugs already fixed in this app: gating on
    /// whether a thing EXISTS rather than on whether it is the thing in front.
    private var showingTerminalGrid: Bool {
        debugTab == .terminal && engine.chrome.terminal.open
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
                            reclaimEditorFocus()
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
                            .fill(accent)
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
                            reclaimEditorFocus()
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
                            theme.separator.opacity(0.6), lineWidth: 1
                        )
                    )
                    // Separated from the tree by depth, not a hairline.
                    .shadow(color: theme.shadowInk.opacity(isLightTheme ? 0.14 : 0.36), radius: 4, y: 1)
            }
        }
        .frame(height: NavStrip.iconH + NavStrip.inset * 2)
        .animation(NavStrip.settle, value: separated)
        .padding(.horizontal, 10)
        // The 26pt selection capsule and the editor's 26pt jump bar start at
        // the same height. Both are measured from the WINDOW top:
        //
        //   jump bar top = topBandHeight (48) + panelGap (6)          = 54
        //   strip top    = the column's own safe-area inset            = 52
        //   + NavStrip.inset, which is the capsule's offset in the strip = 54
        //
        // so the capsule needs nothing above it. The `.padding(.top, 5)` and
        // `.offset(y: 2)` that used to be here were tuned against the
        // hand-drawn card, whose content began at the window top because it had
        // no safe area to inherit; carried onto a real column they stacked on
        // top of the system's own 52pt and pushed the capsule 7pt low.
        //
        // The bottom padding absorbs both, so the strip keeps its 40pt slot and
        // nothing below it — Project, the tree — moves at all.
        .padding(.bottom, 10)
    }

    /// Geometry and motion for the navigator strip. Kept together because the
    /// icon layout and the metaball behind it must agree to the point — a
    /// mismatch shows up as glyphs sitting off-centre in their own chrome.
    private enum NavStrip {
        static let iconW: CGFloat = 28
        static let iconH: CGFloat = ContentView.editorHeaderHeight
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



    /// One folded layout's container behind its run of chips. Its horizontal
    /// bounds are EXACTLY the measured run: the old `+16` expansion crossed
    /// the 4pt inter-tab gap by 4pt on either side and visibly overlapped
    /// unrelated neighbors.
    ///
    /// Grouped → unified: the container must *collapse into* the unified chip,
    /// not pop out while members separately fade. Removal uses a stronger
    /// shrink toward center so it reads as the same shape becoming the chip
    /// (the chip itself inserts with a matching grow). Insertion stays a
    /// soft gather for fold.
    /// Grouped-layout band: Apple system blue wash (same family as light-mode
    /// selection highlights) so a folded run reads clearly against the titlebar.
    private var layoutGroupFill: Color {
        Color(nsColor: .systemBlue).opacity(isLightTheme ? 0.22 : 0.32)
    }
    private var layoutGroupStroke: Color {
        Color(nsColor: .systemBlue).opacity(isLightTheme ? 0.45 : 0.55)
    }

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
                Capsule(style: .continuous).fill(on ? accent : Color.clear)
            )
    }


    /// Custom titlebar row: replaces NSToolbar so the sidebar reaches the
    /// traffic lights and the tab strip centers on the WINDOW — shrinking to
    /// the space left between the sidebar/toggle zone and the trailing icons.
    private var topBar: some View {
        PerfProbe.measure("  body.topBar") { topBarBody }
    }

    @ViewBuilder
    private var topBarBody: some View {
        // No `GeometryReader`, and no reserve arithmetic on a measured width.
        //
        // Everything that used to be here — a viewport computed from
        // `geo.size.width`, a `Spacer` sized by the sidebar, and a `Color.clear`
        // window-drag layer switched off by hover — is gone into
        // `TabStripHostView`, which reads its viewport from the window and
        // routes its own presses. `geo.size.width` swept 1120 → 820 → 1120 on
        // every sidebar toggle and carried the whole strip with it; asking the
        // window instead is the only way to stop asking a moving thing. See
        // docs/SUISEI-TAB-STRIP-HOST.md.
        //
        // No control cluster here either. The navigator toggle and the three
        // document controls are NATIVE toolbar items — the sidebar column's own
        // for the toggle, `editorToolbar` for the rest.
        TabStripHost(
            tabs: engine.chrome.tabs,
            overflowCount: engine.chrome.tabsOverflow,
            palette: stripPalette,
            // The split view already reports its real AppKit column width. Use
            // that edge instead of the old 296pt default, which was too wide for
            // one window and underneath a resized navigator in another.
            // The SETTLED width, never the live one — see `navSettledWidth`.
            leadingInset: navSettledWidth > 1
                ? navSettledWidth + ContentView.stripSidebarGap
                : ContentView.stripReserveCollapsedLeading,
            // Fallback only. TabStripHost reads the real visible NSToolbar item
            // frames and stops before whichever audio/image/PDF run AppKit laid
            // out; no control width is guessed here.
            trailingInset: ContentView.stripReserveTrailing,
            // The 48pt Swiss-grid band, dropped by the shared amount so the
            // chips sit on the same line as the trailing toolbar items.
            rowDrop: ContentView.titlebarDrop,
            bandHeight: ContentView.topBandHeight,
            activeSlot: engine.chrome.tabs.first(where: \.active)?.id,
            actions: stripActions
        )
        .frame(height: ContentView.topBandHeight)
        .frame(maxWidth: .infinity)
        // Inert: the real strip is a subview of the window's content view, not
        // of this tree. Left hit-testable and it would be one more claimant on
        // the band, which is the defect it was moved out to escape.
        .allowsHitTesting(false)
    }

    /// Window-space fallback keep-outs. An open navigator supplies its measured
    /// boundary; a closed one still clears the traffic lights and fixed toggle.
    static let stripReserveCollapsedLeading: CGFloat = 150
    /// Small visual breath after the native sidebar divider — not another panel
    /// width. Eight points matches the shell's compact titlebar rhythm.
    static let stripSidebarGap: CGFloat = 8
    static let stripReserveTrailing: CGFloat = 150

    private var stripPalette: TabStripPalette {
        TabStripPalette(
            isLight: isLightTheme,
            // The THEME's accent, not `Color.accentColor` — that is the system
            // accent and `.tint` does not redirect it.
            accent: NSColor(accent),
            fg: NSColor(Color.primary),
            dim: NSColor(Color.secondary),
            groupFill: NSColor(layoutGroupFill),
            groupStroke: NSColor(layoutGroupStroke),
            activeFill: NSColor(
                Color.primary.opacity(isLightTheme ? 0.10 : 0.14)
            ),
            hoverFill: NSColor(
                Color.primary.opacity(isLightTheme ? 0.06 : 0.10)
            ),
            hoverFillInGroup: NSColor(Color.black.opacity(0.08)),
            closeWell: NSColor(
                Color.primary.opacity(isLightTheme ? 0.10 : 0.16)
            )
        )
    }

    /// Everything the strip can do. One dispatch table, because the view has
    /// one press router — the three-way disagreement over who owned a click is
    /// the defect this replaced.
    private var stripActions: TabStripActions {
        let tabs = engine.chrome.tabs
        return TabStripActions(
            click: { slot in
                focused = true
                guard let tab = tabs.first(where: { $0.id == slot }) else {
                    engine.gotoTab(slot)
                    return
                }
                applyStripClick(chip: tab.stableId)
            },
            doubleClick: { slot in
                focused = true
                // An alternate grouped ⇄ unified gesture, routed through the
                // same coordinator as a one-step vertical scroll so it cannot
                // snap independently.
                guard let tab = tabs.first(where: { $0.id == slot }),
                      tab.group != 0
                else { return }
                engine.toggleLayoutStyle(tab.group)
            },
            close: { slot in
                guard let tab = tabs.first(where: { $0.id == slot }) else { return }
                focused = true
                applyStripClose(chip: tab.stableId)
            },
            reorder: { held, to in
                // By stable id: under a folded group the slots no longer line
                // up with buffer indices, so a slot-based move would carry the
                // wrong document.
                guard let fromId = tabs.first(where: { $0.id == held })?.stableId,
                      let toId = tabs.first(where: { $0.id == to })?.stableId
                else { return }
                _ = engine.moveTabIds(from: fromId, to: toId)
            },
            foldUp: { _ = engine.advanceLayoutPresentation() },
            foldDown: { _ = engine.retreatLayoutPresentation() },
            plusMenu: { view, rect, event in
                stripMenus.menu([
                    ("New Untitled Tab", { engine.openBlankTab() }),
                    ("Next Tab", { engine.nextTab() }),
                    ("Previous Tab", { engine.prevTab() }),
                    nil,
                    ("Split Editor Right", { engine.splitEditorRight() }),
                    ("Split Editor Below", { engine.splitEditorBelow() }),
                    ("Focus Next Pane", { engine.focusNextPane() }),
                    ("Close Focused Pane", { engine.closeFocusedPane() }),
                ])
                // Under the button, not at the strip's origin — the strip is
                // the width of the window.
                .popUp(
                    positioning: nil,
                    at: CGPoint(x: rect.minX, y: rect.maxY + 2),
                    in: view
                )
                _ = event
            },
            contextMenu: { view, slot, event in
                guard let tab = tabs.first(where: { $0.id == slot }) else { return }
                var entries: [(String, () -> Void)?] = []
                if tab.group != 0 {
                    entries.append((
                        tab.isLayout
                            ? "Show Layout as Group"
                            : "Merge Layout into One Tab",
                        { engine.toggleLayoutStyle(tab.group) }
                    ))
                    entries.append(("Unfold Layout", { _ = engine.unfoldLayout() }))
                    entries.append(nil)
                }
                entries.append(("Close Tab", { applyStripClose(chip: tab.stableId) }))
                entries.append(("Close Other Tabs", {
                    for other in tabs.reversed() where other.stableId != tab.stableId {
                        applyStripClose(chip: other.stableId)
                    }
                }))
                NSMenu.popUpContextMenu(
                    stripMenus.menu(entries), with: event, for: view
                )
            }
        )
    }


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

    /// Put the keyboard back in the editor shell after a navigator click —
    /// unless a shell has it.
    ///
    /// `focused` drives the root container, which is not itself focusable, so
    /// SwiftUI hands the keyboard to the first field it can find (the
    /// navigator's Filter). Doing that to a window with a running terminal
    /// reads as the terminal going dead: it is on screen, it has a caret, and
    /// it receives nothing. Switching navigator modes is a statement about
    /// which list is shown, not about who owns the keyboard.
    private func reclaimEditorFocus() {
        guard !engine.terminalOwnsKeys else { return }
        focused = true
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
        PerfProbe.measure("  body.dockedNavigator") { dockedNavigatorBody }
    }

    @ViewBuilder
    private var dockedNavigatorBody: some View {
        VStack(spacing: 0) {
            // Title row under mode strip (Xcode density)
            HStack(spacing: 6) {
                Text(navMode.title)
                    .font(.system(size: 11, weight: .semibold, design: .rounded))
                    .foregroundStyle(.primary)
                    .padding(.leading, navMode == .project ? 2 : 0)
                Spacer()
                if navMode == .project {
                    // Navigator actions on the section header's right end —
                    // the same ToolbarPlainIcon as the window topBar, 12 pt.
                    // New file / folder / collapse execute inside the tree
                    // (that is where the rename + expansion state lives);
                    // refresh is the header's own action, as before.
                    ToolbarPlainIcon(
                        systemImage: "doc.badge.plus", help: "New File in Folder",
                        accent: accent, dim: Color.secondary,
                        iconSize: 11.5, opticalNudgeY: 1.5
                    ) {
                        NotificationCenter.default.post(name: .suiseiNavNewFile, object: nil)
                    }
                    ToolbarPlainIcon(
                        systemImage: "folder.badge.plus", help: "New Folder",
                        accent: accent, dim: Color.secondary,
                        iconSize: 12.5, opticalNudgeY: 0.5
                    ) {
                        NotificationCenter.default.post(name: .suiseiNavNewFolder, object: nil)
                    }
                    ToolbarPlainIcon(
                        systemImage: "arrow.clockwise", help: "Refresh",
                        accent: accent, dim: Color.secondary,
                        iconSize: 13.5, opticalNudgeY: 1
                    ) {
                        ProjectTreeView.invalidateCache()
                        engine.ensureProjectTree()
                        focused = true
                    }
                    ToolbarPlainIcon(
                        // `rectangle.compress.vertical` paints much taller than
                        // its nominal point size. 12pt matches the measured
                        // visual mass of the adjacent action glyphs.
                        systemImage: "rectangle.compress.vertical", help: "Collapse All",
                        accent: accent, dim: Color.secondary,
                        iconSize: 12, opticalNudgeY: 1
                    ) {
                        NotificationCenter.default.post(name: .suiseiNavCollapseAll, object: nil)
                    }
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
            accent: accent,
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
            NavigatorSearchField(
                text: $findQuery,
                onSubmit: {
                    engine.searchProject(findQuery)
                    focused = true
                },
                onClear: {
                    engine.searchProject("")
                }
            )
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
                            .fill(theme.panelSurface.opacity(0.55))
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
                                                ? accent
                                                : accent.opacity(0.55)
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
                        let unpushed = engine.chrome.scm.graph.filter(\.unpushed).count
                        HStack(spacing: 6) {
                            Text("History")
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(.secondary)
                            Spacer(minLength: 0)
                            // The other two sections count their rows here.
                            // Counting commits would say the graph limit, which
                            // is a setting; what the column is scanned for is
                            // how much is still local.
                            if unpushed > 0 {
                                Text("\(unpushed) unpushed")
                                    .font(.system(size: 10, weight: .medium).monospacedDigit())
                                    .foregroundStyle(accent)
                            }
                        }
                        .padding(.horizontal, 10)
                        .padding(.top, 12)
                        .padding(.bottom, 4)
                        ForEach(engine.chrome.scm.graph) { g in
                            scmHistoryRow(g)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 4)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(
                                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                                        .fill(g.selected ? accent.opacity(0.12) : Color.clear)
                                )
                        }
                    }
                }
                .padding(.bottom, 8)
            }

            Button {
                NotificationCenter.default.post(name: .suiseiOpenGitWorkbenchWindow, object: nil)
            } label: {
                HStack {
                    Image(systemName: "macwindow")
                        .font(.system(size: 11))
                    Text("Open Source Control Window")
                        .font(.system(size: 11, weight: .medium))
                    Spacer()
                }
                .foregroundStyle(accent)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
            .buttonStyle(.plain)
            .overlay(alignment: .top) {
                Rectangle().fill(theme.separator).frame(height: 1)
            }
        }
    }

    /// One commit, laid out instead of printed.
    ///
    /// It was `Text(g.line)` — the string Core pre-joins as
    /// `"{strip} {short}  {subject}  {when}"` — set in 10pt monospace, one line,
    /// 2pt of padding. That is a log dump wearing a sidebar, and it could not
    /// have been anything else: the FFI flattened the commit into that string
    /// and the face had nothing else to work with. Core has carried `short`,
    /// `subject`, `when`, `refs` and a lane colour the whole time.
    ///
    /// The graph strip is deliberately NOT drawn here. It only reads as a graph
    /// when every row is monospaced and aligned, and a 240pt navigator cannot
    /// give it that and the subject too — the workbench window is where a graph
    /// belongs. What this column is for is "what landed recently", which is the
    /// subject, and enough identity to act on it.
    private func scmHistoryRow(_ g: ScmGraphItem) -> some View {
        HStack(spacing: 7) {
            Circle()
                .fill(scmLaneColor(g.color))
                .frame(width: 6, height: 6)
            VStack(alignment: .leading, spacing: 1) {
                Text(g.subject.isEmpty ? g.line : g.subject)
                    .font(.system(size: 11.5))
                    .foregroundStyle(g.selected ? accentForeground : .primary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                HStack(spacing: 5) {
                    Text(g.shortHash)
                        .font(.system(size: 9.5, design: .monospaced))
                    if !g.when.isEmpty {
                        Text(g.when).font(.system(size: 9.5))
                    }
                    if !g.refs.isEmpty {
                        Text(g.refs)
                            .font(.system(size: 9))
                            .lineLimit(1)
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(
                                Capsule().fill(accent.opacity(g.selected ? 0.30 : 0.16))
                            )
                    }
                }
                .foregroundStyle(
                    g.selected ? accentForeground.opacity(0.75) : Color.secondary
                )
                .lineLimit(1)
            }
            Spacer(minLength: 0)
            if g.unpushed { scmUnpushedBadge(selected: g.selected) }
        }
    }

    /// Xcode's `U`: this commit is on HEAD and not on its upstream yet.
    ///
    /// Trailing, so the badges line up in one column and the list can be
    /// scanned for "what have I not pushed" without reading a single subject.
    ///
    /// Core marks the rows; the face does not count them. The graph walk is
    /// `git log --all`, so other branches' tips sit between HEAD's commits in
    /// date order — badging the top `ahead` rows would have marked whichever
    /// commits were most recent, on any branch.
    private func scmUnpushedBadge(selected: Bool) -> some View {
        Text("U")
            .font(.system(size: 9, weight: .bold, design: .rounded))
            .foregroundStyle(selected ? accentForeground : accent)
            .frame(width: 14, height: 14)
            .background(
                Circle().fill(accent.opacity(selected ? 0.30 : 0.16))
            )
            .help("Not pushed to the upstream yet")
    }

    /// The graph walker's lane colour, so a branch keeps one hue down the
    /// column. Falls back to the accent for an index we do not have a hue for.
    private func scmLaneColor(_ index: UInt8) -> Color {
        let lanes: [Color] = [
            accent,
            theme.successColor,
            theme.warningColor,
            theme.dangerColor,
            theme.color(theme.macroName),
            theme.color(theme.typeName),
        ]
        return lanes[Int(index) % lanes.count]
    }

    /// `M`, `A`, `D`… in a chip whose colour says staged or not.
    private func scmBadge(_ row: ScmEntryItem) -> some View {
        let mark = row.mark.trimmingCharacters(in: .whitespaces)
        let ink = row.staged ? theme.successColor : theme.warningColor
        return Text(mark.isEmpty ? "•" : mark)
            .font(.system(size: 9, weight: .bold, design: .monospaced))
            .foregroundStyle(ink)
            .frame(width: 15, height: 15)
            .background(
                RoundedRectangle(cornerRadius: 3.5, style: .continuous)
                    .fill(ink.opacity(0.16))
            )
    }

    private func scmFileName(_ path: String) -> String {
        (path as NSString).lastPathComponent
    }

    /// The folder, or nothing when the file sits at the repository root —
    /// "." beside a filename is noise pretending to be information.
    private func scmFolder(_ path: String) -> String? {
        let parent = (path as NSString).deletingLastPathComponent
        return parent.isEmpty || parent == "." ? nil : parent
    }

    private func scmFileSymbol(_ path: String) -> String {
        switch (path as NSString).pathExtension.lowercased() {
        case "swift": "swift"
        case "rs", "c", "cpp", "h", "hpp", "go", "java", "kt", "cs": "chevron.left.forwardslash.chevron.right"
        case "js", "ts", "jsx", "tsx", "py", "rb", "sh", "lua": "curlybraces"
        case "json", "toml", "yaml", "yml", "plist", "xml": "list.bullet.rectangle"
        case "md", "txt", "rst": "doc.text"
        case "png", "jpg", "jpeg", "gif", "svg", "pdf": "photo"
        default: "doc"
        }
    }

    @ViewBuilder
    private func scmSection(title: String, rows: [ScmEntryItem], empty: String?) -> some View {
        if !rows.isEmpty || empty != nil {
            HStack(spacing: 6) {
                Text(title)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
                if !rows.isEmpty {
                    Text("\(rows.count)")
                        .font(.system(size: 10, weight: .medium).monospacedDigit())
                        .foregroundStyle(.tertiary)
                }
            }
            // Sentence case, not SHOUTED. Xcode's source list labels its
            // groups the way the rest of the system does; an all-caps 10pt
            // header is a VS Code idiom and it was the loudest text in a
            // column whose job is to be scanned past.
            .padding(.horizontal, 10)
            .padding(.top, 10)
            .padding(.bottom, 4)
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

    /// Ink for the dock's header row.
    ///
    /// The dock paints ONE colour across its whole region — deliberately, so a
    /// grid that falls a few points short can never show a seam — and that
    /// colour is the terminal's dark one while the terminal tab is SHOWING.
    /// The header sits on top of it, so its ink has to follow the fill instead
    /// of the editor theme, or it is dark-on-dark. See `showingTerminalGrid`
    /// for why "showing" and not "open".
    private var dockHeaderFg: Color {
        showingTerminalGrid ? terminalGridFg : fg
    }

    private var dockHeaderDim: Color {
        showingTerminalGrid ? terminalGridFg.opacity(0.55) : dim
    }

    /// Debug area — Xcode-style bottom console hosting the shell (no modes, no XLC).
    private var debugArea: some View {
        debugAreaBody
            // Here rather than on the editor shell's chain: that one is at the
            // type-checker's limit and one more modifier tips it over, which is
            // the same wall `sidebarRow` documents. This is also the view the
            // flag is ABOUT.
            //
            // The id is the TAB, so `task` fires on appear and on every
            // switch — exactly when core needs telling. It used to be
            // `uiDebugVisible && debugTab == .debug`, which is a boolean that
            // does not move when the reader goes from Terminal to Build: two
            // different tabs, one `false`, and core never heard about the
            // second one. The `uiDebugVisible` half is the bridge's, which is
            // where it has to be — this view is inside `if uiDebugVisible`.
            .task(id: debugTab) {
                syncDebugPanel()
            }
    }

    private var debugAreaBody: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                // The bottom area holds two things now, so it says which one
                // it is showing. Xcode's arrangement: the debugger and the
                // console share the floor and neither owns it.
                debugAreaTabs
                if debugTab == .terminal, terminalFocused {
                    Text("keys → shell · Esc or click editor to leave")
                        .font(.system(size: 10))
                        .foregroundStyle(dockHeaderDim)
                        .transition(.opacity)
                } else if debugTab == .terminal, engine.chrome.terminal.open {
                    Text("click to type")
                        .font(.system(size: 10))
                        .foregroundStyle(dockHeaderDim)
                        .transition(.opacity)
                }
                Spacer()
                // Shell sessions (VS Code-style): chips + new-session.
                if debugTab == .terminal, engine.chrome.terminal.open {
                    HStack(spacing: 3) {
                        // Chips scroll once they outgrow the header; the "+"
                        // stays pinned outside the scroller so it never drifts
                        // off-screen as sessions pile up.
                        ScrollViewReader { proxy in
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: 3) {
                                    ForEach(0..<shells.dock.count, id: \.self) { i in
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
                            .onChange(of: shells.dockActive) { _, i in
                                withAnimation(.snappy(duration: 0.2)) {
                                    proxy.scrollTo(i, anchor: .center)
                                }
                            }
                            .onChange(of: shells.dock.count) { _, n in
                                withAnimation(.snappy(duration: 0.2)) {
                                    proxy.scrollTo(max(0, n - 1), anchor: .trailing)
                                }
                            }
                        }
                        .fixedSize(horizontal: false, vertical: true)

                        HoverIconButton(
                            systemImage: "plus", help: "New Shell",
                            fg: dockHeaderFg, dim: dockHeaderDim
                        ) {
                            shells.openDockSession(
                                cwd: engine.dockTerminalCwd(), palette: terminalPalette
                            )
                            engine.focusTerminal(true)
                        }
                    }
                    .padding(.trailing, 4)
                }
                // Same component as the "+" beside it — a bespoke Button with
                // its own padding sized and centred differently, which is why
                // the two never lined up.
                HoverIconButton(
                    systemImage: "xmark", help: "Hide Debug Area",
                    fg: dockHeaderFg, dim: dockHeaderDim
                ) {
                    withAnimation(.snappy(duration: 0.28)) {
                        engine.uiDebugVisible = false
                    }
                    // Closes the DOCK only. Pane terminals are separate
                    // processes and are not affected.
                    if engine.chrome.terminal.open {
                        engine.toggleTerminalDock()
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
                if debugTab == .debug {
                    DebugPanelView(
                        engine: engine, accent: accent, fg: fg, dim: dim,
                        separator: theme.separator
                    )
                } else if debugTab == .build {
                    BuildPanelView(
                        engine: engine, accent: accent, fg: fg, dim: dim,
                        separator: theme.separator
                    )
                } else if engine.chrome.terminal.open {
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

    /// Which third of the bottom area is showing.
    ///
    /// Build joined Terminal and Debug because it is the same KIND of thing —
    /// a program ran and said something — and putting its output anywhere else
    /// would have meant a second place to look for what a program said.
    enum DebugAreaTab: String, CaseIterable {
        case terminal, debug, build

        var title: String {
            switch self {
            case .terminal: return "Terminal"
            case .debug: return "Debug"
            case .build: return "Build"
            }
        }

        var symbol: String {
            switch self {
            case .terminal: return "terminal.fill"
            case .debug: return "ladybug.fill"
            case .build: return "hammer.fill"
            }
        }
    }

    /// Two words and two glyphs, not a segmented control.
    ///
    /// The header is 28pt and already carries the shell chips, a "+" and a
    /// close button; a bordered segmented control at that height is taller
    /// than the row it sits in. This is the same treatment the navigator's
    /// mode strip uses — the lit one is the one you are looking at.
    private var debugAreaTabs: some View {
        HStack(spacing: 2) {
            ForEach(DebugAreaTab.allCases, id: \.self) { tab in
                let on = debugTab == tab
                Button {
                    // Just the tab. `syncDebugPanel` tells core, from the one
                    // place that watches BOTH inputs — see it for why this
                    // used to set the flag here and was wrong.
                    debugTab = tab
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: tab.symbol)
                            .font(.system(size: 9.5, weight: .semibold))
                        Text(tab.title)
                            .font(.system(size: 11, weight: .semibold, design: .rounded))
                    }
                    .foregroundStyle(on ? dockHeaderFg : dockHeaderDim)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 3)
                    .background(
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .fill(on ? Color.primary.opacity(0.10) : .clear)
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func terminalSessionChip(_ i: Int) -> some View {
        let active = i == shells.dockActive
        return Button {
            shells.selectDockSession(i)
            engine.focusTerminal(true)
        } label: {
            HStack(spacing: 4) {
                Image(systemName: "terminal")
                    .font(.system(size: 8, weight: .semibold))
                // The shell's own name once it reports one (OSC 0/2), so a
                // running `make` or an ssh session says so on its chip.
                Text(shells.dockTitle(i))
                    .font(.system(size: 10, weight: active ? .semibold : .regular))
                    .lineLimit(1)
                if active, shells.dock.count > 1 {
                    Button {
                        shells.closeDockSession(i)
                    } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 7, weight: .bold))
                    }
                    .buttonStyle(.plain)
                    .help("Close Shell")
                }
            }
            .foregroundStyle(active ? accent : Color.secondary)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(
                Capsule(style: .continuous)
                    .fill(active ? accent.opacity(0.14) : Color.primary.opacity(0.05))
            )
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
    }

    /// The "Open Terminal · ⌃T" prompt, when the debug area is up with no shell.
    ///
    /// Through `setDebugArea` rather than around it. This used to open the dock
    /// by hand — `uiDebugVisible` plus a toggle — which skipped the two things
    /// that make an opened shell typable: core's own focus, and stripping the
    /// keyboard from whatever field had it. Then it set `focused = true`, and
    /// the root container `focused` drives is not itself focusable, so SwiftUI
    /// handed the keyboard to the first field it could find — the navigator's
    /// Filter. Exactly the bug the tap handler below documents, still live at
    /// this one call site.
    /// Hand the tab half of the answer to the bridge, which owns the other
    /// half and does the telling.
    ///
    /// Not `dapSetPanel` from here any more. This runs on a view inside
    /// `if uiDebugVisible { … }`, so it can say "the debugger is showing" and
    /// can never say the opposite — the dock closing takes this view with it.
    private func syncDebugPanel() {
        engine.debugTabIsDebugger = (debugTab == .debug)
        engine.debugTabIsBuild = (debugTab == .build)
    }

    private func openDebugTerminal() {
        // Set here as well as in the `terminal.open` handler, because a shell
        // that is ALREADY running does not change that flag — so opening the
        // terminal while the debugger tab was showing published nothing and
        // the tab never moved.
        debugTab = .terminal
        engine.setDebugArea(true)
    }

    /// Terminal body (Debug strip) — the selected shell, drawn by SwiftTerm.
    private var terminalPanelInner: some View {
        TerminalDockSurface(
            palette: terminalPalette,
            engine: engine,
            activeChip: shells.dockActive
        )
        // The SAME fill behind the whole panel. The view can land a few points
        // shy of it — measured 4pt at the bottom — and the panel's own dock
        // fill is a different tint, so that residue read as a strip the
        // terminal "did not reach". Painting one colour behind both makes the
        // seam impossible rather than arithmetically avoided.
        .background(terminalGridBg)
        .contentShape(Rectangle())
        .onTapGesture {
            engine.focusTerminal(true)
            // RELEASE the editor's SwiftUI focus — `focused` drives the editor
            // canvas's first responder. Setting it true here (the old code)
            // re-focused the EDITOR the instant you clicked the terminal, so
            // the input method composed Hangul in the document instead of the
            // shell. The terminal claims the responder itself.
            focused = false
        }
    }

    /// One palette for every shell in the window, docked or in a pane.
    private var terminalPalette: TerminalPalette {
        TerminalPalette(
            background: terminalGridBg, foreground: terminalGridFg, fontSize: 12
        )
    }

    /// Push theme appearance into AppKit windows so titlebar / materials follow
    /// the theme. Editor windows keep the system titlebar — transparent, title
    /// hidden — and the split view's sidebar runs up through it with AppKit's
    /// own traffic lights floating over it, exactly as Source Control does.
    ///
    /// MUST stay idempotent — every assignment is guarded by a "changed?"
    /// check. Re-ASSIGNING NSApp.appearance / window styling with the same
    /// values on each app reactivation rebuilt the window's view bridge and
    /// killed NSHostingView's SwiftUI hit-testing (the focus-out/in freeze:
    /// every SwiftUI control dead, AppKit canvas still alive). With the guards
    /// it is safe to call from didBecomeActive, which is what keeps late-born
    /// windows (session restore, second editor window) in sync with the theme.
    private func applyWindowAppearance() {
        let name = WindowChrome.themedAppearanceName(light: isLightTheme)
        let appearance = NSAppearance(named: name)
        if NSApp.appearance?.name != name {
            NSApp.appearance = appearance
        }
        // Appearance is safe to push at any window; window styling is not —
        // restyling an open panel or a popover is both wrong and, for titlebar
        // work, fatal. See `isEditorWindow`.
        //
        // Everything the editor window needs beyond appearance — background,
        // opacity, transparent titlebar, separator style, movability, visible
        // standard buttons — is `ThemedWindowChrome`'s job, exactly as in
        // Source Control, and it re-applies on every SwiftUI update rather than
        // once per theme change. The only thing left here is the window title,
        // which this window has no place to draw.
        for window in NSApp.windows where window.title != "Welcome" {
            if window.appearance?.name != name {
                window.appearance = appearance
            }
            // Nothing else: the editor window's own geometry is
            // `EditorWindowChrome`'s, which reads its window directly instead
            // of matching an identifier this function may run before anyone
            // has set.
        }
    }

    /// One corner radius for every floating panel (sidebar, editor island,
    /// outline card) — matches the window's own corner radius so the nested
    /// corners read as one system. The resize HUD mask uses the same value.
    ///
    /// Derived, not written twice. Nothing in this app rounds the editor window
    /// — macOS draws that corner — so this constant is a MIRROR of a system
    /// value, and it had drifted: three places each claimed to know the window
    /// radius and two of them said 12 while `WelcomeView` (the one window whose
    /// corner this app actually cuts itself, so the one number that had to look
    /// right next to real windows) said 18. On this OS the window corner is the
    /// rounder one, which is why the sidebar read tighter than the background
    /// behind it.
    ///
    /// Same shape of defect as the tab strip's: several independent authorities
    /// for one fact, silently disagreeing. There is one now.
    ///
    /// `window - gap`, not `window`. A card inset by `panelGap` keeps a uniform
    /// gap only if its corner is that much tighter; give it the window's own
    /// radius and the two curves converge at the corner and the gap pinches
    /// shut. (I set this to the window radius first, which is the same mistake
    /// in the opposite direction from the 12 it replaced.)
    static let panelCornerRadius: CGFloat =
        Radius.inside(WindowChrome.windowCornerRadius, gap: panelGap)

    /// Gap between a panel and the window edge.
    static let panelGap: CGFloat = 6

    /// The editor island starts below this band, and the tab strip sits on its
    /// 24pt optical centreline.
    ///
    /// It no longer positions anything but the tabs. The traffic lights and the
    /// document controls are AppKit's own, in AppKit's own titlebar row over
    /// the sidebar column — and with a toolbar present AppKit puts the lights
    /// at 26pt from the window top, which is `topBandHeight / 2 + titlebarDrop`
    /// exactly. The row that used to be aligned by hand now agrees with the
    /// system by arithmetic (measured, `scripts/sidebar_probe6.swift`).
    static let topBandHeight: CGFloat = 48

    /// How far the titlebar row sits below that centreline. One constant for
    /// the tab strip and the trailing icons, because they read as one row and
    /// any drift between them is immediately visible.
    static let titlebarDrop: CGFloat = 2

    /// Fixed status-bar height — it renders as the root's bottom z-layer and
    /// the detail column reserves exactly this much.
    static let statusBarHeight: CGFloat = 24

    // The editor's traffic lights are AppKit's own now.
    //
    // `styleTrafficLights` / `applyTrafficLightInset` / the whole
    // `StableTrafficLightOverlay` — hidden standard buttons, cloned button
    // cells, an Auto-Layout host pinned to the frame view, a `drop` that had to
    // SUBTRACT where SwiftUI adds, and a re-install on every window activation
    // — all existed to put lights inside a hand-drawn card. A
    // `NavigationSplitView` sidebar rises under the real titlebar on its own,
    // so the real buttons land over the sidebar with no help, which is what
    // Source Control has always done.
    //
    // Nothing replaces them: `ThemedWindowChrome` un-hides the standard
    // buttons and AppKit owns their geometry.

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
        // No manual shadow: system glass casts its own context-aware one
        // (adapts to the content beneath). The extra shadow this used to
        // add doubled it and never adapted.
        return content()
            .clipShape(shape)
            .glassEffect(SuiseiGlass.panel(light: isLightTheme, style: engine.glassStyle), in: shape)
    }

    // Legacy floating explorer/SCM cards removed — the navigator owns these
    // surfaces now (flat, full-height sidebar).

    private func scmRow(_ row: ScmEntryItem) -> some View {
        Button {
            engine.scmSelect(row.id)
            focused = true
        } label: {
            HoverRow(corner: 4) {
                HStack(spacing: 7) {
                    Image(systemName: scmFileSymbol(row.path))
                        .font(.system(size: 11))
                        .foregroundStyle(row.selected ? accentForeground : .secondary)
                        .frame(width: 15)
                    // The NAME first and the folder after it, quietly. It was
                    // the whole relative path in monospace, middle-truncated,
                    // which in a 240pt column turns `src/…/main.rs` into a
                    // puzzle — the filename is what you are looking for and it
                    // was the part being elided.
                    Text(scmFileName(row.path))
                        .font(.system(size: 11.5))
                        .foregroundStyle(row.selected ? accentForeground : .primary)
                        .lineLimit(1)
                    if let folder = scmFolder(row.path) {
                        Text(folder)
                            .font(.system(size: 10))
                            .foregroundStyle(
                                row.selected ? accentForeground.opacity(0.7) : .secondary
                            )
                            .lineLimit(1)
                            .truncationMode(.head)
                            .layoutPriority(-1)
                    }
                    Spacer(minLength: 4)
                    // A letter in a chip on the right, the way every source
                    // list on this platform marks a row's state — not a bare
                    // glyph in the leading column competing with the icon.
                    scmBadge(row)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(
                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                        .fill(row.selected ? accent.opacity(0.16) : Color.clear)
                )
                .contentShape(Rectangle())
            }
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 4)
        .simultaneousGesture(
            TapGesture(count: 2).onEnded {
                engine.scmActivate(row.id)
                focused = true
            }
        )
        .contextMenu {
            Button("Open File") {
                engine.scmActivate(row.id)
                focused = true
            }
            Button(row.staged ? "Unstage" : "Stage") {
                engine.scmToggleStage(row.id)
                focused = true
            }
        }
    }

    // MARK: - Git workbench (Ctrl+Shift+G)

    private var gitWorkbenchDocked: some View {
        HStack(spacing: 0) {
            gitWorkbenchSourceList
                .frame(width: 184)

            gitWorkbenchDivider

            VStack(spacing: 0) {
                gitWorkbenchDetailHeader
                gitWorkbenchDetailBody
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(editorBg)
        .contentShape(Rectangle())
        .alert("New Branch", isPresented: $showGitNewBranchDialog) {
            TextField("Branch name", text: $gitNewBranchName)
            Button("Cancel", role: .cancel) {}
            Button("Create") {
                engine.gitWbCreateBranch(gitNewBranchName)
                gitNewBranchName = ""
                focused = true
            }
            .disabled(gitNewBranchName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        } message: {
            Text("Create and check out a branch from the current HEAD.")
        }
        .confirmationDialog(
            "Delete “\(pendingGitBranchDeletion)”?",
            isPresented: $showGitBranchDeletionConfirmation,
            titleVisibility: .visible
        ) {
            Button("Delete Branch", role: .destructive) {
                engine.gitWbDeleteSelectedBranch()
                pendingGitBranchDeletion = ""
                focused = true
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Only the selected local branch will be deleted. This cannot be undone.")
        }
    }

    private var gitWorkbenchSourceList: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 7) {
                Label("Source Control", systemImage: "externaldrive.connected.to.line.below")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.primary)

                Label(
                    engine.chrome.gitWb.branch.isEmpty ? "Detached HEAD" : engine.chrome.gitWb.branch,
                    systemImage: "arrow.triangle.branch"
                )
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            }
            .padding(.horizontal, 12)
            .frame(height: 58, alignment: .leading)

            Rectangle()
                .fill(theme.separator)
                .frame(height: 1)

            ScrollView {
                VStack(alignment: .leading, spacing: 3) {
                    gitWorkbenchSourceSection("WORKSPACE")
                    gitWorkbenchSourceDestination("Status", label: "Changes")
                    gitWorkbenchSourceDestination("Log", label: "History")

                    gitWorkbenchSourceSection("REPOSITORY")
                        .padding(.top, 8)
                    gitWorkbenchSourceDestination("Branches")
                    gitWorkbenchSourceDestination("Stash", label: "Stashes")

                    gitWorkbenchSourceSection("GITHUB")
                        .padding(.top, 8)
                    gitWorkbenchSourceDestination("PRs", label: "Pull Requests")
                    gitWorkbenchSourceDestination("Issues")
                    gitWorkbenchSourceDestination("Auth", label: "Account")
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 9)
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
        // Full opacity. A semantic colour made translucent is no longer the
        // semantic colour — it is that colour mixed with whatever happens to be
        // behind it, so it stops tracking appearance and Increase Contrast.
        // Depth here comes from the `Color.primary` washes on the rows.
        .background(theme.windowBg)
    }

    private func gitWorkbenchSourceSection(_ title: String) -> some View {
        Text(title)
            .font(.system(size: 9, weight: .semibold))
            .foregroundStyle(.tertiary)
            .tracking(0.35)
            .padding(.horizontal, 7)
            .frame(height: 18, alignment: .bottomLeading)
    }

    @ViewBuilder
    private func gitWorkbenchSourceDestination(_ name: String, label: String? = nil) -> some View {
        if let chip = engine.chrome.gitWb.chips.first(where: { $0.label == name }) {
            Button {
                gitWorkbenchCompactPage = .master
                gitWorkbenchContextVisible = false
                engine.gitWbSetTab(chip.key)
                focused = true
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: gitWorkbenchDestinationSymbol(name))
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(chip.active ? Color.white : .secondary)
                        .frame(width: 15)
                    Text(label ?? gitWorkbenchDestinationTitle(name))
                        .font(.system(size: 11, weight: chip.active ? .semibold : .regular))
                        .foregroundStyle(chip.active ? Color.white : .primary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 8)
                .frame(height: 27)
                .background(
                    RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                        .fill(chip.active ? accent : Color.clear)
                )
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private var gitWorkbenchDetailHeader: some View {
        HStack(spacing: 9) {
            Image(systemName: gitWorkbenchDestinationSymbol(activeGitWorkbenchDestination))
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)

            VStack(alignment: .leading, spacing: 1) {
                Text(gitWorkbenchDestinationTitle(activeGitWorkbenchDestination))
                    .font(.system(size: 12, weight: .semibold))
                Text(gitWorkbenchDetailSubtitle)
                    .font(.system(size: 9.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            if activeGitWorkbenchDestination == "Branches" {
                Button {
                    gitNewBranchName = ""
                    showGitNewBranchDialog = true
                } label: {
                    Label("New Branch", systemImage: "plus")
                        .font(.system(size: 10.5, weight: .medium))
                }
                .buttonStyle(.borderless)

                if let branch = selectedGitBranch, !branch.current {
                    Button("Check Out") {
                        engine.gitWbCheckoutSelectedBranch()
                        focused = true
                    }
                    .font(.system(size: 10.5, weight: .medium))
                    .buttonStyle(.borderless)
                }
            }

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
                    .frame(width: 26, height: 24)
            }
            .buttonStyle(.plain)
            .help("Refresh")

            Button {
                gitWorkbenchCompactPage = .master
                gitWorkbenchContextVisible = false
                engine.toggleGitWorkbench()
                focused = true
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 26, height: 24)
            }
            .buttonStyle(.plain)
            .help("Close · Esc")
        }
        .padding(.horizontal, 12)
        .frame(height: 44)
        .overlay(alignment: .bottom) {
            Rectangle().fill(theme.separator).frame(height: 1)
        }
    }

    private var gitWorkbenchDetailSubtitle: String {
        switch activeGitWorkbenchDestination {
        case "Status": return "Review and stage changes in the working tree"
        case "Log": return "Browse commits and inspect changed files"
        case "Branches": return "Select a branch, then check it out explicitly"
        case "Stash": return "Saved working tree changes"
        case "PRs": return "Pull requests for this repository"
        case "Issues": return "Issues for this repository"
        case "Auth": return "GitHub CLI account and authentication"
        default: return engine.chrome.gitWb.message
        }
    }

    @ViewBuilder
    private var gitWorkbenchDetailBody: some View {
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
                let contextWidth = min(max(CGFloat(gitWorkbenchContextW), 260), 320)
                let maximumMaster = geo.size.width >= 1_200
                    ? max(280, geo.size.width - contextWidth - 522)
                    : max(280, geo.size.width - 521)
                let masterWidth = min(
                    max(CGFloat(gitWorkbenchMasterW), 280),
                    min(480, maximumMaster)
                )
                let isHistory = activeGitWorkbenchDestination == "Log"

                if geo.size.width < 800 {
                    gitWorkbenchCompactBody(isHistory: isHistory)
                } else if isHistory {
                    HStack(spacing: 0) {
                        gitRichColumn(
                            title: "History",
                            icon: "clock.arrow.circlepath",
                            lines: engine.chrome.gitWb.colLog,
                            kind: .history
                        ) {
                            engine.gitWbSelectHistory($0)
                            if geo.size.width < 1_200 {
                                gitWorkbenchContextVisible = true
                            }
                        }
                        .frame(width: masterWidth)

                        gitWorkbenchResizeDivider(
                            size: $gitWorkbenchMasterW,
                            min: 280,
                            max: 480,
                            invert: false
                        )

                        gitDiffDetail(
                            lines: engine.chrome.gitWb.special,
                            showFilesAction: geo.size.width < 1_200
                                ? { gitWorkbenchContextVisible.toggle() }
                                : nil
                        )
                        .frame(maxWidth: .infinity)
                        .overlay(alignment: .trailing) {
                            if geo.size.width < 1_200, gitWorkbenchContextVisible {
                                HStack(spacing: 0) {
                                    gitWorkbenchResizeDivider(
                                        size: $gitWorkbenchContextW,
                                        min: 260,
                                        max: 320,
                                        invert: true
                                    )
                                    gitRichColumn(
                                        title: "Changed Files",
                                        icon: "doc.text",
                                        lines: engine.chrome.gitWb.colFiles,
                                        kind: .files
                                    ) {
                                        engine.gitWbSelectCommitFile($0)
                                        gitWorkbenchContextVisible = false
                                    }
                                    .frame(width: contextWidth)
                                }
                                .background(editorBg)
                                .shadow(color: theme.shadowInk.opacity(0.20), radius: 12, x: -4)
                                .transition(.move(edge: .trailing).combined(with: .opacity))
                            }
                        }

                        if geo.size.width >= 1_200 {
                            gitWorkbenchResizeDivider(
                                size: $gitWorkbenchContextW,
                                min: 260,
                                max: 320,
                                invert: true
                            )
                            gitRichColumn(
                                title: "Changed Files",
                                icon: "doc.text",
                                lines: engine.chrome.gitWb.colFiles,
                                kind: .files
                            ) { engine.gitWbSelectCommitFile($0) }
                            .frame(width: contextWidth)
                        }
                    }
                    .animation(.easeOut(duration: 0.16), value: gitWorkbenchContextVisible)
                } else {
                    HStack(spacing: 0) {
                        gitRichColumn(
                            title: "Changes",
                            icon: "doc.badge.gearshape",
                            lines: engine.chrome.gitWb.colChanges,
                            kind: .changes
                        ) { engine.gitWbSelectChange($0) }
                        .frame(width: masterWidth)

                        gitWorkbenchResizeDivider(
                            size: $gitWorkbenchMasterW,
                            min: 280,
                            max: 480,
                            invert: false
                        )

                        gitDiffDetail(lines: engine.chrome.gitWb.special)
                            .frame(maxWidth: .infinity)
                    }
                }
            }
        } else if activeGitWorkbenchDestination == "Branches" {
            gitBranchesSurface
        } else {
            gitSpecialSurface
        }
    }

    private var activeGitWorkbenchDestination: String {
        engine.chrome.gitWb.chips.first(where: \.active)?.label ?? "Status"
    }

    private func gitWorkbenchDestinationTitle(_ raw: String) -> String {
        raw == "Log" ? "History" : (raw == "Stash" ? "Stashes" : raw)
    }

    private func gitWorkbenchDestinationSymbol(_ raw: String) -> String {
        switch raw {
        case "Status": return "arrow.triangle.2.circlepath"
        case "Log": return "clock.arrow.circlepath"
        case "Branches": return "arrow.triangle.branch"
        case "Stash": return "tray.full"
        case "PRs": return "arrow.triangle.pull"
        case "Issues": return "exclamationmark.circle"
        case "Auth": return "person.crop.circle"
        default: return "point.3.connected.trianglepath.dotted"
        }
    }

    private var gitWorkbenchDivider: some View {
        Rectangle()
            .fill(theme.separator)
            .frame(width: 1)
    }

    private func gitWorkbenchResizeDivider(
        size: Binding<Double>,
        min: Double,
        max: Double,
        invert: Bool
    ) -> some View {
        ZStack {
            gitWorkbenchDivider
            PanelResizeGrip(
                size: size,
                minS: min,
                maxS: max,
                axis: .horizontal,
                invert: invert,
                fg: fg,
                onEnded: persistPanelSizes
            )
        }
        .frame(width: 1)
        .zIndex(2)
    }

    @ViewBuilder
    private func gitWorkbenchCompactBody(isHistory: Bool) -> some View {
        switch gitWorkbenchCompactPage {
        case .master:
            gitRichColumn(
                title: isHistory ? "History" : "Changes",
                icon: isHistory ? "clock.arrow.circlepath" : "doc.badge.gearshape",
                lines: isHistory
                    ? engine.chrome.gitWb.colLog
                    : engine.chrome.gitWb.colChanges,
                kind: isHistory ? .history : .changes
            ) {
                if isHistory {
                    engine.gitWbSelectHistory($0)
                    gitWorkbenchCompactPage = .files
                } else {
                    engine.gitWbSelectChange($0)
                    gitWorkbenchCompactPage = .diff
                }
            }

        case .files:
            VStack(spacing: 0) {
                gitWorkbenchCompactBackBar(title: "History") {
                    gitWorkbenchCompactPage = .master
                }
                gitRichColumn(
                    title: "Changed Files",
                    icon: "doc.text",
                    lines: engine.chrome.gitWb.colFiles,
                    kind: .files
                ) {
                    engine.gitWbSelectCommitFile($0)
                    gitWorkbenchCompactPage = .diff
                }
            }

        case .diff:
            VStack(spacing: 0) {
                gitWorkbenchCompactBackBar(title: isHistory ? "Changed Files" : "Changes") {
                    gitWorkbenchCompactPage = isHistory ? .files : .master
                }
                gitDiffDetail(lines: engine.chrome.gitWb.special)
            }
        }
    }

    private func gitWorkbenchCompactBackBar(
        title: String,
        action: @escaping () -> Void
    ) -> some View {
        HStack(spacing: 8) {
            Button(action: action) {
                Label("Back", systemImage: "chevron.backward")
                    .font(.system(size: 11, weight: .semibold))
            }
            .buttonStyle(.plain)
            .foregroundStyle(accent)
            Spacer()
            Text(title)
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 12)
        .frame(height: 32)
        .overlay(alignment: .bottom) {
            Rectangle().fill(theme.separator).frame(height: 1)
        }
    }

    private var gitBranchRows: [GitBranchRow] {
        engine.chrome.gitWb.special.enumerated().compactMap { index, line in
            let characters = Array(line)
            guard characters.count >= 3 else { return nil }

            let selected = characters[0] == "›"
            let current = characters[1] == "*"
            let hasScopeMarker = characters.count >= 4
                && (characters[2] == "L" || characters[2] == "R")
                && characters[3] == " "
            let remote = hasScopeMarker
                ? characters[2] == "R"
                : String(characters.dropFirst(2)).trimmingCharacters(in: .whitespaces).hasPrefix("origin/")
            let markerWidth = hasScopeMarker ? 4 : 2
            let name = String(characters.dropFirst(markerWidth))
                .trimmingCharacters(in: .whitespaces)
            guard !name.isEmpty, !name.hasPrefix("(") else { return nil }

            return GitBranchRow(
                id: index,
                name: name,
                selected: selected,
                current: current,
                remote: remote
            )
        }
    }

    private var selectedGitBranch: GitBranchRow? {
        gitBranchRows.first(where: \.selected) ?? gitBranchRows.first(where: \.current)
    }

    private var gitBranchesSurface: some View {
        let local = gitBranchRows.filter { !$0.remote }
        let remote = gitBranchRows.filter(\.remote)

        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Text("Branches")
                    .font(.system(size: 12, weight: .semibold))
                Spacer()
                Text("\(gitBranchRows.count)")
                    .font(.system(size: 10).monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 14)
            .frame(height: 34)
            .overlay(alignment: .bottom) {
                Rectangle().fill(theme.separator).frame(height: 1)
            }

            if gitBranchRows.isEmpty {
                gitEmptyState(
                    icon: "arrow.triangle.branch",
                    title: "No branches",
                    detail: engine.chrome.gitWb.message.isEmpty
                        ? "Refresh the repository to load branches."
                        : engine.chrome.gitWb.message
                )
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 2) {
                        gitBranchGroup("LOCAL", rows: local)
                        if !remote.isEmpty {
                            gitBranchGroup("REMOTES", rows: remote)
                                .padding(.top, 9)
                        }
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder
    private func gitBranchGroup(_ title: String, rows: [GitBranchRow]) -> some View {
        if !rows.isEmpty {
            Text(title)
                .font(.system(size: 9, weight: .semibold))
                .foregroundStyle(.tertiary)
                .tracking(0.35)
                .padding(.horizontal, 8)
                .frame(height: 20, alignment: .bottomLeading)

            ForEach(rows) { branch in
                gitBranchRow(branch)
            }
        }
    }

    private func gitBranchRow(_ branch: GitBranchRow) -> some View {
        HoverRow(corner: Radius.row) {
            Button {
                engine.gitWbSelectSpecial(branch.id)
                focused = true
            } label: {
                HStack(spacing: 9) {
                    Image(systemName: branch.current
                        ? "checkmark.circle.fill"
                        : (branch.remote ? "icloud" : "arrow.triangle.branch"))
                        .font(.system(size: 11, weight: branch.current ? .semibold : .regular))
                        .foregroundStyle(branch.current ? accent : .secondary)
                        .frame(width: 16)

                    Text(branch.name)
                        .font(.system(size: 11.5, weight: branch.current ? .semibold : .regular))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .truncationMode(.middle)

                    Spacer(minLength: 8)

                    if branch.current {
                        Text("Current")
                            .font(.system(size: 9.5))
                            .foregroundStyle(.secondary)
                    } else if branch.remote {
                        Text("Remote")
                            .font(.system(size: 9.5))
                            .foregroundStyle(.tertiary)
                    }
                }
                .padding(.horizontal, 10)
                .frame(height: 30)
                .background(
                    RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                        .fill(branch.selected ? accent.opacity(isLightTheme ? 0.14 : 0.20) : Color.clear)
                )
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .contextMenu {
            Button("Check Out") {
                engine.gitWbSelectSpecial(branch.id)
                engine.gitWbCheckoutSelectedBranch()
                focused = true
            }
            .disabled(branch.current)

            if !branch.remote {
                Divider()
                Button("Delete Branch", role: .destructive) {
                    engine.gitWbSelectSpecial(branch.id)
                    pendingGitBranchDeletion = branch.name
                    showGitBranchDeletionConfirmation = true
                }
                .disabled(branch.current)
            }
        }
        .simultaneousGesture(
            TapGesture(count: 2).onEnded {
                guard !branch.current else { return }
                engine.gitWbSelectSpecial(branch.id)
                engine.gitWbCheckoutSelectedBranch()
                focused = true
            }
        )
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
                Rectangle().fill(theme.separator).frame(height: 1)
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
                        ForEach(Array(lines.enumerated()), id: \.offset) { offset, line in
                            gitSpecialRow(
                                line,
                                selectionIndex: gitSpecialSelectionIndex(lines: lines, offset: offset)
                            )
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
    private func gitSpecialRow(_ line: String, selectionIndex: Int?) -> some View {
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

        if isMeta || selectionIndex == nil {
            Text(trimmed)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 8)
                .padding(.vertical, 2)
        } else if let selectionIndex {
            HoverRow(corner: 4) {
                Button {
                    engine.gitWbSelectSpecial(selectionIndex)
                } label: {
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
                        Rectangle()
                            .fill(selected ? accent.opacity(0.10) : Color.clear)
                    )
                    .overlay(alignment: .leading) {
                        Rectangle()
                            .fill(selected ? accent : Color.clear)
                            .frame(width: 2)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(trimmed.replacingOccurrences(of: "›", with: ""))
            }
        }
    }

    private func gitSpecialSelectionIndex(lines: [String], offset: Int) -> Int? {
        guard lines.indices.contains(offset) else { return nil }
        let tab = engine.chrome.gitWb.tabIndex

        func isSelectable(_ line: String) -> Bool {
            let trimmed = line
                .trimmingCharacters(in: .whitespaces)
                .replacingOccurrences(of: "›", with: "")
                .trimmingCharacters(in: .whitespaces)
            switch tab {
            case 2, 8:
                return !trimmed.hasPrefix("(") && !trimmed.isEmpty
            case 5, 6:
                return trimmed.hasPrefix("#")
            default:
                return false
            }
        }

        guard isSelectable(lines[offset]) else { return nil }
        return lines[..<offset].filter(isSelectable).count
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

    private enum GitWorkbenchListKind {
        case changes
        case history
        case files
    }

    private func gitRichColumn(
        title: String,
        icon: String,
        lines: [String],
        kind: GitWorkbenchListKind,
        onSelect: @escaping (Int) -> Void
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: icon)
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(accent)
                Text(title)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.primary)
                Spacer()
                let shown = gitWorkbenchSelectableCount(lines: lines, kind: kind)
                let total = gitWorkbenchAdvertisedCount(lines: lines, kind: kind)
                Text(total > shown ? "\(shown)/\(total)" : "\(shown)")
                    .font(.system(size: 10).monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .frame(height: 32)
            .overlay(alignment: .bottom) {
                Rectangle().fill(theme.separator).frame(height: 1)
            }

            if lines.isEmpty || (lines.count == 1 && lines[0].localizedCaseInsensitiveContains("clean")) {
                gitEmptyState(
                    icon: icon,
                    title: title == "Changes" ? "Working tree clean" : "No items",
                    detail: title == "History" ? "Commits appear after load" : " "
                )
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(lines.enumerated()), id: \.offset) { offset, line in
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
                            // The column header already says “Changes”; the
                            // engine's first wrapper line repeated the same
                            // title immediately underneath it. Keep staged /
                            // local groups, omit only that duplicate wrapper.
                            if kind == .changes && cleaned.hasPrefix("Changes") {
                                EmptyView()
                            } else if isHeader {
                                Text(cleaned)
                                    .font(.system(size: 10, weight: .bold))
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 12)
                                    .padding(.top, 8)
                                    .padding(.bottom, 2)
                            } else if let selection = gitWorkbenchSelectionIndex(
                                lines: lines,
                                offset: offset,
                                kind: kind
                            ) {
                                Button {
                                    onSelect(selection)
                                    focused = true
                                } label: {
                                    gitWorkbenchListRow(
                                        cleaned: cleaned,
                                        selected: selected,
                                        kind: kind
                                    )
                                }
                                .buttonStyle(.plain)
                                .padding(.horizontal, 4)
                            } else {
                                Text(cleaned)
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 12)
                                    .padding(.vertical, 5)
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

    private func gitWorkbenchListRow(
        cleaned: String,
        selected: Bool,
        kind: GitWorkbenchListKind
    ) -> some View {
        let status = gitStatusCode(cleaned, kind: kind)
        let title = status == nil ? cleaned : String(cleaned.dropFirst(2))
        return HoverRow(corner: Radius.row) {
            HStack(spacing: 8) {
                Image(systemName: kind == .history ? "point.3.connected.trianglepath.dotted" : "doc.text")
                    .font(.system(size: 11, weight: .regular))
                    .foregroundStyle(selected ? accent : .secondary)
                    .frame(width: 14)
                Text(title)
                    .font(.system(size: 11, design: kind == .history ? .monospaced : .default))
                    .foregroundStyle(selected ? fg : fg.opacity(0.88))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
                if let status {
                    Text(status)
                        .font(.system(size: 9, weight: .semibold, design: .monospaced))
                        .foregroundStyle(gitStatusDotColor(cleaned))
                        .padding(.horizontal, 5)
                        .frame(height: 16)
                        .background(
                            RoundedRectangle(cornerRadius: 4, style: .continuous)
                                .fill(gitStatusDotColor(cleaned).opacity(0.12))
                        )
                }
            }
            .padding(.horizontal, 10)
            .frame(height: 27)
            .background(
                RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                    .fill(selected ? accent.opacity(isLightTheme ? 0.14 : 0.18) : Color.clear)
            )
            .contentShape(Rectangle())
        }
    }

    private func gitStatusCode(
        _ line: String,
        kind: GitWorkbenchListKind
    ) -> String? {
        guard kind == .changes, line.count >= 2 else { return nil }
        let chars = Array(line)
        guard chars[1].isWhitespace, "MADRU?".contains(chars[0]) else { return nil }
        return String(chars[0])
    }

    private func gitWorkbenchSelectionIndex(
        lines: [String],
        offset: Int,
        kind: GitWorkbenchListKind
    ) -> Int? {
        guard lines.indices.contains(offset) else { return nil }
        var selection = 0
        for index in 0...offset {
            let cleaned = lines[index]
                .replacingOccurrences(of: "▾ ", with: "")
                .replacingOccurrences(of: "›", with: "")
                .trimmingCharacters(in: .whitespaces)
            let selectable: Bool
            switch kind {
            case .changes:
                let first = cleaned.first
                let second = cleaned.dropFirst().first
                selectable = first.map { "MADRU CT?".contains($0) } == true
                    && second?.isWhitespace == true
            case .history:
                selectable = index > 0
                    && !cleaned.hasPrefix("(")
                    && !cleaned.localizedCaseInsensitiveContains("loading history")
            case .files:
                selectable = index > 1
                    && !cleaned.hasPrefix("(")
            }
            if selectable {
                if index == offset { return selection }
                selection += 1
            }
        }
        return nil
    }

    private func gitWorkbenchSelectableCount(
        lines: [String],
        kind: GitWorkbenchListKind
    ) -> Int {
        lines.indices.reduce(into: 0) { count, offset in
            if gitWorkbenchSelectionIndex(lines: lines, offset: offset, kind: kind) != nil {
                count += 1
            }
        }
    }

    /// The engine deliberately caps the visible Status column so an enormous
    /// worktree does not flood the fixed-size FFI snapshot. Surface that as
    /// “shown/total” instead of presenting two apparently contradictory
    /// counts (for example 50 in the header and 58 in the group label).
    private func gitWorkbenchAdvertisedCount(
        lines: [String],
        kind: GitWorkbenchListKind
    ) -> Int {
        let shown = gitWorkbenchSelectableCount(lines: lines, kind: kind)
        guard kind == .changes,
              let wrapper = lines.first(where: {
                  $0.replacingOccurrences(of: "▾", with: "")
                      .trimmingCharacters(in: .whitespaces)
                      .hasPrefix("Changes")
              }),
              let total = wrapper.split(whereSeparator: { !$0.isNumber }).last
                  .flatMap({ Int($0) })
        else { return shown }
        return max(shown, total)
    }

    private func gitDiffDetail(
        lines: [String],
        showFilesAction: (() -> Void)? = nil
    ) -> some View {
        let hasHeader = lines.first?.localizedCaseInsensitiveContains("diff ·") == true
        let title = hasHeader
            ? lines[0].replacingOccurrences(of: "diff ·", with: "").trimmingCharacters(in: .whitespaces)
            : "Diff"
        let body = hasHeader ? Array(lines.dropFirst()) : lines

        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "doc.badge.ellipsis")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(accent)
                Text(title.isEmpty ? "Diff" : title)
                    .font(.system(size: 11, weight: .semibold, design: .monospaced))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
                if let showFilesAction {
                    Button(action: showFilesAction) {
                        Label("Files", systemImage: "sidebar.trailing")
                            .font(.system(size: 10, weight: .medium))
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .help("Show changed files")
                }
            }
            .padding(.horizontal, 12)
            .frame(height: 32)
            .overlay(alignment: .bottom) {
                Rectangle().fill(theme.separator).frame(height: 1)
            }

            if body.isEmpty {
                gitEmptyState(
                    icon: "doc.text.magnifyingglass",
                    title: "Select a file",
                    detail: "Its changes will appear here"
                )
            } else {
                GeometryReader { viewport in
                    ScrollView([.horizontal, .vertical]) {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(body.enumerated()), id: \.offset) { _, line in
                                Text(line.isEmpty ? " " : line)
                                    .font(.system(size: 11, design: .monospaced))
                                    .foregroundStyle(gitDiffLineColor(line))
                                    .padding(.horizontal, 12)
                                    .frame(minHeight: 20, alignment: .leading)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .background(gitDiffLineBackground(line))
                            }
                        }
                        // A two-axis ScrollView otherwise sizes to the diff's
                        // intrinsic text width and centres that narrow column.
                        // Give it the viewport as a floor so code and hunk
                        // backgrounds begin at the detail pane's leading edge.
                        .frame(
                            minWidth: viewport.size.width,
                            alignment: .topLeading
                        )
                        .padding(.vertical, 6)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(editorBg)
    }

    private func gitDiffLineColor(_ line: String) -> Color {
        if line.hasPrefix("@@") { return accent }
        if line.hasPrefix("+") && !line.hasPrefix("+++") {
            return Color(nsColor: .systemGreen)
        }
        if line.hasPrefix("-") && !line.hasPrefix("---") {
            return Color(nsColor: .systemRed)
        }
        return fg.opacity(0.86)
    }

    private func gitDiffLineBackground(_ line: String) -> Color {
        if line.hasPrefix("+") && !line.hasPrefix("+++") {
            return Color(nsColor: .systemGreen).opacity(0.07)
        }
        if line.hasPrefix("-") && !line.hasPrefix("---") {
            return Color(nsColor: .systemRed).opacity(0.07)
        }
        return Color.clear
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

    /// The find bar, and — when asked for — the replace row under it.
    ///
    /// Replacing across the project has worked since the workspace search was
    /// written; the file in front of you was the one place it could not be
    /// done. The row is disclosed rather than always present: most finds are
    /// finds, and a field nobody is using is a field in the way.
    private var findBar: some View {
        VStack(alignment: .leading, spacing: 6) {
            findRow
            if engine.chrome.search.replaceOpen {
                replaceRow
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .frame(minWidth: 420, idealWidth: 500, maxWidth: 560)
        .glassEffect(
            SuiseiGlass.chrome(light: isLightTheme, style: engine.glassStyle),
            in: RoundedRectangle(cornerRadius: 18, style: .continuous)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .strokeBorder(fg.opacity(0.08), lineWidth: 0.5)
        )
        .shadow(color: theme.shadowInk.opacity(isLightTheme ? 0.12 : 0.32), radius: 6, y: 2)
        .padding(.top, 8)
        .padding(.trailing, 12)
        .animation(.snappy(duration: 0.18), value: engine.chrome.search.replaceOpen)
    }

    private var replaceRow: some View {
        HStack(spacing: 8) {
            NativeReplaceField(
                text: Binding(
                    get: { engine.chrome.search.replaceInput },
                    set: { engine.setReplaceInput($0) }
                ),
                onCommit: { engine.replaceCurrent() }
            )
            .frame(minWidth: 230, maxWidth: .infinity)
            .frame(height: 24)

            Button("Replace") { engine.replaceCurrent() }
                .controlSize(.small)
                .disabled(engine.chrome.search.matchCount == 0)
                .help("Replace this one and move on")

            Button("All") { engine.replaceAll() }
                .controlSize(.small)
                .disabled(engine.chrome.search.matchCount == 0)
                .help("Replace every match in this file · one undo")
        }
        // Lined up with the field above rather than with the bar: the two
        // fields are one control and their left edges have to agree.
        .padding(.leading, 24)
    }

    private var findRow: some View {
        HStack(spacing: 8) {
            // The disclosure, first, so the two fields sit in one column.
            Button {
                engine.setReplaceOpen(!engine.chrome.search.replaceOpen)
            } label: {
                Image(systemName: engine.chrome.search.replaceOpen
                    ? "chevron.down" : "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .frame(width: 16, height: 16)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Replace · ⌥⌘F")

            NativeFindSearchField(
                text: Binding(
                    get: { engine.chrome.search.input },
                    set: { engine.setFindInput($0) }
                )
            )
            .focused($overlayTextInput, equals: .find)
            .frame(minWidth: 230, maxWidth: .infinity)
            .frame(height: 24)
            .onAppear {
                DispatchQueue.main.async {
                    overlayTextInput = .find
                }
            }
            .onDisappear {
                if overlayTextInput == .find {
                    overlayTextInput = nil
                }
            }

            Text(
                engine.chrome.search.matchCount > 0
                    ? "\(engine.chrome.search.matchIndex + 1) of \(engine.chrome.search.matchCount)"
                    : (engine.chrome.search.input.isEmpty ? "0 results" : "No results")
            )
            .font(.system(size: 11))
            .monospacedDigit()
            .foregroundStyle(
                engine.chrome.search.matchCount > 0
                    ? Color.secondary : Color(nsColor: .systemOrange)
            )
            .frame(minWidth: 58, alignment: .trailing)

            ControlGroup {
                Button {
                    engine.findStep(forward: false)
                } label: {
                    Image(systemName: "chevron.up")
                }
                .help("Previous · ⇧⌘G")

                Button {
                    engine.findStep(forward: true)
                } label: {
                    Image(systemName: "chevron.down")
                }
                .help("Next · ⌘G")
            }
            .controlGroupStyle(.navigation)
            .controlSize(.small)
            .disabled(engine.chrome.search.matchCount == 0)

            Button("Done") {
                // Commit while the find field still owns first responder. If
                // focus is released first, the editor cancels Search and this
                // same Return is reinterpreted as a document newline.
                engine.closeFind()
                focused = true
            }
            .controlSize(.small)
            .keyboardShortcut(.return, modifiers: [])
            .help("Done · Return")
        }
    }

    // MARK: - Palette overlay (Ctrl/Cmd+P)

    private var paletteOverlay: some View {
        GeometryReader { geo in
            ZStack(alignment: .top) {
                GlassScrim(lightChrome: isLightTheme)
                    .ignoresSafeArea()
                    .onTapGesture {
                        engine.dispatch(code: .esc)
                        focused = true
                    }

                palettePanel
                    .frame(width: ContentView.paletteWidth)
                    // TOP-anchored, not centred vertically. The panel grows and
                    // shrinks as the list filters, and a vertically centred
                    // panel moves under the pointer on every keystroke — the
                    // field would drift while being typed into. Spotlight pins
                    // its field and lets the list fall out of it; this is that.
                    //
                    // Proportional rather than a fixed 72pt, which read as
                    // "stuck to the top" on a tall window and crowded a short
                    // one. Clamped at both ends so neither extreme is silly.
                    .padding(.top, min(200, max(64, geo.size.height * 0.16)))
                    .frame(maxWidth: .infinity, alignment: .center)
            }
        }
    }

    /// Roughly Open Quickly's. Wide enough for a path with a filename on the
    /// end, narrow enough not to read as a sheet.
    private static let paletteWidth: CGFloat = 560

    /// A field, a hairline, a list — Open Quickly's shape.
    ///
    /// What was here instead: a header bar carrying the palette's kind on the
    /// left and a fake "Esc" key capsule on the right, above a field set in SF
    /// **Rounded**. Neither Spotlight nor Open Quickly has a header, and macOS
    /// does not set chrome in Rounded — it is the system font for watchOS and
    /// for playful app content, so a rounded palette reads as a different
    /// platform before you have read a word of it. The kind is worth keeping,
    /// so it became the field's prompt, which is where a Mac says what a search
    /// field searches.
    private var palettePanel: some View {
        glassPanel(corner: 20) {
            VStack(spacing: 0) {
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 15))
                        .foregroundStyle(.secondary)
                    TextField(
                        palettePrompt,
                        text: Binding(
                            get: { engine.chrome.palette.query },
                            set: { engine.setPaletteQuery($0) }
                        )
                    )
                    .textFieldStyle(.plain)
                    .foregroundStyle(fg)
                    .focused($overlayTextInput, equals: .palette)
                    Spacer(minLength: 0)
                }
                .font(.system(size: 15))
                .padding(.horizontal, 16)
                .padding(.vertical, 13)
                .onAppear {
                    DispatchQueue.main.async { overlayTextInput = .palette }
                }
                .onDisappear {
                    if overlayTextInput == .palette { overlayTextInput = nil }
                }

                Rectangle()
                    .fill(theme.separator)
                    .frame(height: 1)

                paletteList
            }
        }
    }

    /// What this palette is for, said once, where a Mac says it.
    private var palettePrompt: String {
        let kind = engine.chrome.palette.kind
        return kind.isEmpty ? "Search" : kind
    }

    /// The results, with the keyboard selection kept on screen.
    ///
    /// The list had no `ScrollViewReader`, so arrowing past the last visible
    /// row moved a selection nobody could see — the list only followed if the
    /// pointer happened to scroll it. Every palette on this platform keeps its
    /// selection visible; that is the behaviour, not a nicety.
    private var paletteList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    ForEach(engine.chrome.palette.items) { item in
                        paletteRow(item).id(item.id)
                    }
                }
                .padding(8)
            }
            .frame(maxHeight: 340)
            .onChange(of: engine.chrome.palette.items.first(where: \.selected)?.id) {
                _, id in
                guard let id else { return }
                proxy.scrollTo(id, anchor: .center)
            }
        }
    }

    @ViewBuilder
    private func paletteRow(_ item: PaletteItem) -> some View {
        // A FILLED selection, the way AppKit draws a selected source-list or
        // completion row. It was a 14%-accent wash behind a 2pt accent bar down
        // the leading edge — a VS Code idiom, and next to a real macOS list the
        // difference is the first thing that reads as wrong.
        let selected = item.selected
        Button {
            engine.paletteActivate(item.id)
            focused = true
        } label: {
            // `HoverRow` still wraps it, so an unselected row lights under the
            // pointer as every list on this platform does. Under the selected
            // row's opaque fill the wash is invisible, so it needs no branch.
            HoverRow(corner: 6) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(item.label)
                        .font(.system(size: 13))
                        .foregroundStyle(selected ? accentForeground : Color.primary)
                        .lineLimit(1)
                    if !item.detail.isEmpty {
                        Text(item.detail)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(
                                selected
                                    ? accentForeground.opacity(0.75) : Color.secondary
                            )
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(selected ? accent : Color.clear)
                )
                .contentShape(Rectangle())
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(item.label)
        .accessibilityHint(item.detail.isEmpty ? "Activate command" : item.detail)
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
                                            HStack(spacing: 0) {
                                                Text(typed).fontWeight(.bold)
                                                Text(String(item.label.dropFirst(typed.count)))
                                            }
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
                            .fill(theme.windowBg)
                    )
                    .clipShape(RoundedRectangle(cornerRadius: panelR, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: panelR, style: .continuous)
                            .strokeBorder(Color.primary.opacity(0.12), lineWidth: 0.6)
                    )
                    .shadow(color: theme.shadowInk.opacity(isLightTheme ? 0.20 : 0.58), radius: 10, y: 4)
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

            Menu {
                Button("Split Left") {
                    engine.splitEditorLeft()
                    focused = true
                }
                .disabled(engine.editorSplit.panes.count >= 4)

                Button("Split Right") {
                    engine.splitEditorRight()
                    focused = true
                }
                .disabled(engine.editorSplit.panes.count >= 4)

                Divider()

                Button("Split Above") {
                    engine.splitEditorAbove()
                    focused = true
                }
                .disabled(engine.editorSplit.panes.count >= 4)

                Button("Split Below") {
                    engine.splitEditorBelow()
                    focused = true
                }
                .disabled(engine.editorSplit.panes.count >= 4)
            } label: {
                Image(systemName: "rectangle.split.1x2")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.secondary)
                    .frame(width: 24, height: 24)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Split editor")
            .accessibilityLabel("Split editor")
            .padding(.trailing, 4)
        }
        .frame(height: ContentView.editorHeaderHeight)
        .background(editorBg)
        .overlay(alignment: .bottom) {
            Rectangle().fill(theme.separator).frame(height: 1)
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
            case .logic: logicInspectorContent
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
                                .fill(theme.separator)
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
                .fill(accent)
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
                            theme.separator.opacity(0.6), lineWidth: 1
                        )
                )
        )
        .padding(.horizontal, 10)
        .padding(.top, 8)
        .padding(.bottom, 2)
    }

    /// Logic — the shape of the file, beside the code that is the text of it.
    ///
    /// Nothing of the file's identity is repeated here: the editor one column
    /// left is already showing which file this is, and a rail this narrow
    /// cannot afford to say anything twice.
    private var logicInspectorContent: some View {
        LogicRail(
            rawPath: engine.chrome.filename,
            palette: viewerPalette,
            cursorRow: engine.chrome.cursorRow
        )
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
                                    RoundedRectangle(cornerRadius: 4, style: .continuous)
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
            // A viewer pane's facts are not the text editor's facts: an image
            // has no line count and a PNG's "Type" is not what someone opened
            // this tab to read. The viewer publishes its own, and they are
            // drawn in this panel's rows so the rail looks like itself.
            if !viewerControls.sections.isEmpty {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(viewerControls.sections) { section in
                            inspectorSection(section.title)
                            ForEach(section.rows) { row in
                                inspectorRow(row.label, row.value)
                            }
                        }
                    }
                    .padding(.bottom, 10)
                }
            } else if path.isEmpty || path == "[No Name]" {
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
                // Was "Put the caret on a symbol, then reopen this tab" — a
                // feature explaining its own wiring, because switching to this
                // tab was the only thing that ever asked. Right-clicking a name
                // asks now, and answers next to the name.
                navigatorPlaceholder(
                    "questionmark.circle", "No description",
                    "Right-click a symbol in the editor and choose Quick Help."
                )
            } else {
                ScrollView {
                    // The same renderer as the popover. This printed the raw
                    // markdown too — fences, rules and link syntax — which
                    // nobody noticed while the only answers reaching it were
                    // one-line signatures.
                    QuickHelpBody(markdown: engine.hoverText)
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
            if !engine.editorSplit.isSplit, !engine.preview.open {
                jumpBar
            }
            Group {
                if engine.preview.open {
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
                if islandShowsMinimap {
                    MinimapStrip(
                        engine: engine,
                        accent: accent,
                        fg: fg,
                        bg: editorBg,
                        isLight: isLightTheme
                    )
                    .frame(width: minimapWidth(paneWidth: editorAreaSize.width))
                    .transition(.opacity)
                }
            }
            // Two 12pt `EdgeFade`s used to sit here, one at each edge, ramping
            // `editorBg` over the text so a line sliding under the island's
            // boundary dissolved instead of being cut. They are gone: the thing
            // they softened is a hard edge that reads perfectly well, and what
            // they actually did was wash out the top and bottom lines of every
            // file you read. "에디터 내부로 들어오는 블러 싹 지우자. 이쁘지도
            // 않고 에디팅경험도 별로."
            //
            // `islandEdgeIsEditorBackground` went with them — it existed to
            // keep the bottom fade off a terminal pane, and there is no fade to
            // keep off anything now.
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
                SuiseiGlass.panel(light: isLightTheme, style: engine.glassStyle),
                in: RoundedRectangle(cornerRadius: Radius.floating, style: .continuous)
            )
            .shadow(color: theme.shadowInk.opacity(0.34), radius: 20, y: 6)
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
                Rectangle().fill(theme.separator).frame(height: 1)
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

    /// ⌃⇧T body — this pane's own shell (header optional: the split's path bar
    /// already has one).
    ///
    /// Every terminal in the window is a separate process. Painting them all
    /// from one `chrome.terminal` snapshot, which is what the face used to do,
    /// made them views of a single session.
    private func terminalPaneBody(
        showClose: Bool,
        showHeader: Bool = true,
        pane: EditorPaneSnap
    ) -> some View {
        let idx = pane.id
        let ownsKeys = engine.editorSplit.focus == idx && engine.terminalOwnsKeys
        return VStack(spacing: 0) {
            if showHeader {
                HStack(spacing: 8) {
                    Image(systemName: "terminal.fill")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(accent)
                    Text(pane.title.isEmpty ? "Terminal" : pane.title)
                        .font(.system(size: 11, weight: .semibold, design: .rounded))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Spacer()
                    Text(ownsKeys ? "keys → shell" : "click to type · ⌃⇧T")
                        .font(.system(size: 10, design: .rounded))
                        .foregroundStyle(ownsKeys ? accent : dim)
                    if showClose {
                        HoverIconButton(systemImage: "xmark", help: "Close terminal", fg: Color.primary, dim: Color.secondary) {
                            // Close THIS pane's shell — ⌃⇧T acts on the focused
                            // pane, so focus must follow the click first.
                            engine.focusPane(idx)
                            engine.toggleTerminalTab()
                            focused = true
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .overlay(alignment: .bottom) {
                    Rectangle().fill(theme.separator).frame(height: 1)
                }
            }

            // SwiftTerm owns the shell, the emulator, the scrollback and its
            // own scroller. There is no grid to size, nothing to report back,
            // and no wheel to forward: everything the old `TerminalGridView`
            // did across the ABI happens inside this view.
            TerminalPaneSurface(
                tabId: pane.tabStableId,
                palette: terminalPalette,
                paneIndex: idx,
                engine: engine
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(terminalGridBg)
        .contentShape(Rectangle())
        // NOTE: no `focused = true` here. That drives the root container's
        // `@FocusState`, which takes the window's first responder straight back
        // off the terminal — undoing the click that just gave it the keyboard.
        // The terminal claims the responder itself in `mouseDown`.
        .onTapGesture { engine.focusTerminalPane(idx) }
    }

    private var mainEditor: some View {
        GeometryReader { geo in
            Group {
                if engine.editorSplit.isSplit {
                    splitEditorLayout(size: geo.size)
                } else if let only = engine.editorSplit.panes.first, only.isTerminal {
                    // Unsplit, and the single pane was converted to a terminal.
                    terminalPaneBody(showClose: true, pane: only)
                } else if let only = engine.editorSplit.panes.first, only.kind.isViewer {
                    // Unsplit, and the document is not text — an image, a PDF,
                    // audio, or something with no text in it at all.
                    PaneViewer(
                        kind: only.kind, path: only.path,
                        tabId: only.tabStableId,
                        palette: viewerPalette, audioPlayer: audioPlayer
                    )
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
                editorAreaSize = geo.size
                engine.resizeEditor(width: geo.size.width, height: geo.size.height)
                // Ensure shell chrome (nav tree) is seeded after light-path sessions.
                if engine.uiNavVisible {
                    engine.ensureProjectTree()
                }
            }
            .onChange(of: geo.size) { _, newSize in
                editorAreaSize = newSize
                guard newSize.width > 80, newSize.height > 80 else { return }
                engine.resizeEditor(width: newSize.width, height: newSize.height)
            }
            .background(WindowFrameReporter { engine.editorWindowFrame = $0 })
        }
        .layoutPriority(0)
    }

    /// A divider between two panes, derived from their rects sharing an edge.
    struct PaneSeam: Identifiable {
        var a: Int
        var b: Int
        /// True when the seam is vertical (the panes sit side by side).
        var vertical: Bool
        /// Position and extent of the seam in normalised editor coordinates.
        var at: CGFloat
        var from: CGFloat
        var to: CGFloat
        var id: String { "\(a)-\(b)-\(vertical)" }
    }

    /// Find every draggable seam in the current layout.
    ///
    /// Two panes share a seam when one's trailing edge is the other's leading
    /// edge and they overlap on the perpendicular axis. Deriving them from the
    /// rects means the face never has to know the tree — and it gets N-1
    /// independent dividers for free, where the old `ratio` gave one for the
    /// whole layout no matter how many panes there were.
    private func paneSeams(_ panes: [EditorPaneSnap]) -> [PaneSeam] {
        var out: [PaneSeam] = []
        let eps: CGFloat = 0.001
        for (i, p) in panes.enumerated() {
            for (j, q) in panes.enumerated() where i != j {
                // q immediately right of p
                if abs(p.rect.maxX - q.rect.minX) < eps {
                    let lo = max(p.rect.minY, q.rect.minY)
                    let hi = min(p.rect.maxY, q.rect.maxY)
                    if hi - lo > eps {
                        out.append(PaneSeam(a: i, b: j, vertical: true,
                                            at: q.rect.minX, from: lo, to: hi))
                    }
                }
                // q immediately below p
                if abs(p.rect.maxY - q.rect.minY) < eps {
                    let lo = max(p.rect.minX, q.rect.minX)
                    let hi = min(p.rect.maxX, q.rect.maxX)
                    if hi - lo > eps {
                        out.append(PaneSeam(a: i, b: j, vertical: false,
                                            at: q.rect.minY, from: lo, to: hi))
                    }
                }
            }
        }
        return out
    }

    /// Panes placed by the rects core computed from its layout tree.
    ///
    /// Absolute placement, not an `HStack`/`VStack`. Stacks could only express
    /// one axis for the whole layout, which is why the four-pane `+` was
    /// unreachable and why three panes overflowed: the old code asked for
    /// `ratio` and `1 - ratio` and handed the second to every pane after the
    /// first. The tree already knows the answer; this just draws it.
    @ViewBuilder
    private func splitEditorLayout(size: CGSize) -> some View {
        let panes = engine.editorSplit.panes
        let pathH = ContentView.editorHeaderHeight
        return ZStack(alignment: .topLeading) {
            ForEach(Array(panes.enumerated()), id: \.element.id) { _, pane in
                let w = max(40, size.width * pane.rect.width - 1)
                let h = max(40, size.height * pane.rect.height - 1)
                editorColumn(
                    pane: pane,
                    contentSize: CGSize(width: max(40, w - 4), height: max(40, h - pathH))
                )
                .frame(width: w, height: h)
                .offset(x: size.width * pane.rect.minX, y: size.height * pane.rect.minY)
            }
            ForEach(paneSeams(panes)) { seam in
                SplitDivider(
                    vertical: seam.vertical,
                    fg: fg,
                    accent: accent,
                    onDrag: { delta in
                        let axis = seam.vertical ? size.width : size.height
                        engine.splitResize(seam.a, seam.b, delta: Double(delta / max(1, axis)))
                    },
                    onEnd: {}
                )
                .frame(
                    width: seam.vertical ? 7 : size.width * (seam.to - seam.from),
                    height: seam.vertical ? size.height * (seam.to - seam.from) : 7
                )
                .offset(
                    x: seam.vertical
                        ? size.width * seam.at - 3.5
                        : size.width * seam.from,
                    y: seam.vertical
                        ? size.height * seam.from
                        : size.height * seam.at - 3.5
                )
            }
        }
        .frame(width: size.width, height: size.height, alignment: .topLeading)
        // Pane STRUCTURE lands in one frame. Pane SIZE still animates.
        //
        // Two different changes reach this view and they want opposite
        // treatment:
        //
        // * a tab switch that collapses a split changes what the ENGINE says
        //   the panes are — their count and their rect fractions — while the
        //   container's size holds still. That should be instant; animated, the
        //   departing pane lingers and the survivor is caught mid-grow, which
        //   is a one-step action played as a little movie.
        // * opening or closing a panel changes the CONTAINER's size while the
        //   engine's pane fractions hold still. That should follow the panel.
        //
        // A blanket `.transaction { $0.animation = nil }` was tried and killed
        // both, so a split desk snapped to full width the instant a panel
        // closed while an unsplit one — a different branch, untouched — glided.
        // Keying the suppression to the structure alone separates them: the key
        // moves only when the engine's own description of the panes does.
        .animation(nil, value: paneStructureKey(panes))
    }

    /// What the ENGINE says the panes are, as one comparable value.
    ///
    /// Ids and rect fractions — deliberately not the pixel sizes, which move
    /// whenever the window or a panel does. This changes when the arrangement
    /// changes and holds still when only the space around it does.
    private func paneStructureKey(_ panes: [EditorPaneSnap]) -> String {
        panes
            .map { p in
                let r = p.rect
                return "\(p.id):\(r.minX),\(r.minY),\(r.width),\(r.height)"
            }
            .joined(separator: "|")
    }

    /// One Xcode editor column: path bar (file for this pane) + text surface.
    /// When ⌃⇧T full terminal is bound to this pane, paint the PTY here instead.
    private func editorColumn(pane: EditorPaneSnap, contentSize: CGSize) -> some View {
        let termHere = pane.isTerminal
        return VStack(spacing: 0) {
            if termHere {
                // Single chrome bar (do not also paint terminalPaneBody header).
                HStack(spacing: 6) {
                    Image(systemName: "terminal.fill")
                        .font(.system(size: 10))
                        .foregroundStyle(pane.focused ? accent : dim)
                    Text(pane.title.isEmpty ? "Terminal" : pane.title)
                        .font(.system(size: 11, weight: pane.focused ? .semibold : .regular))
                        .foregroundStyle(pane.focused ? fg : dim)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    Text(pane.focused && engine.terminalOwnsKeys ? "keys → shell" : "click to type · ⌃⇧T")
                        .font(.system(size: 10))
                        .foregroundStyle(pane.focused && engine.terminalOwnsKeys ? accent : Color.secondary)
                    paneSplitMenu(pane: pane)
                    Button {
                        // Close THIS pane's shell — ⌃⇧T acts on the focused
                        // pane, so without this the button killed (or created)
                        // a terminal in whichever pane happened to have focus.
                        engine.focusPane(pane.id)
                        engine.toggleTerminalTab()
                        focused = true
                    } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(.secondary)
                            // Same 24pt target as the split menu beside it —
                            // a bare glyph next to a framed one reads as two
                            // unrelated controls, and is harder to hit.
                            .frame(width: 24, height: 24)
                    }
                    .buttonStyle(.plain)
                    .help("Close terminal")
                }
                .padding(.horizontal, 10)
                .frame(height: ContentView.editorHeaderHeight)
                .background(editorBg.opacity(0.95))
                .overlay(alignment: .bottom) {
                    Rectangle()
                        .fill(pane.focused ? accent.opacity(0.5) : theme.separator)
                        .frame(height: pane.focused ? 2 : 1)
                }

                terminalPaneBody(showClose: false, showHeader: false, pane: pane)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if pane.kind.isViewer {
                // The path bar stays: a viewer pane is still a document in a
                // split, and the breadcrumb is how you know which one.
                panePathBar(pane: pane)
                PaneViewer(
                    kind: pane.kind, path: pane.path,
                    tabId: pane.tabStableId,
                    palette: viewerPalette, audioPlayer: audioPlayer
                )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    // Clicking a viewer moves core's focus, the way clicking a
                    // canvas or a shell already does. Without it the header
                    // accent, the status bar and ⌘W all went on describing the
                    // pane the user had just clicked away from.
                    .overlay {
                        PaneClickReporter {
                            guard engine.editorSplit.focus != pane.id else { return }
                            engine.focusPane(pane.id)
                        }
                    }
            } else {
                panePathBar(pane: pane)
                editorSurface(
                    lines: pane.lines,
                    size: contentSize,
                    paneIndex: pane.id,
                    showFocusRing: pane.focused
                )
                .overlay(alignment: .trailing) {
                    if minimapEnabled, pane.focused || minimapAllPanes {
                        MinimapStrip(
                            engine: engine,
                            paneIndex: pane.id,
                            accent: accent,
                            fg: fg,
                            bg: editorBg,
                            isLight: isLightTheme
                        )
                        // The pane's own width, not the editor's: a strip
                        // proportional to the whole area would be the same
                        // width in a pane half the size.
                        .frame(width: minimapWidth(
                            paneWidth: contentSize.width * pane.rect.width
                        ))
                        .transition(.opacity)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(editorBg)
        // Pane focus follows the canvas's own mouseDown (no tap recognizer —
        // it intercepted AppKit clicks).
    }

    /// The glyph for a pane's header, by what the pane holds.
    ///
    /// A TEXT pane defers to the file's own glyph — the same `FileSymbol`
    /// table the tree and the tab strip read — so a split of `main.rs` and
    /// `notes.md` is two different glyphs here too. The other kinds answer
    /// for themselves: there is one image viewer and it looks like one.
    private func paneHeaderSymbol(_ kind: PaneKind, title: String = "") -> String {
        switch kind {
        case .text: return title.isEmpty ? "doc.text.fill" : FileSymbol.symbol(for: title)
        case .terminal: return "terminal.fill"
        case .image: return "photo.fill"
        case .pdf: return "doc.richtext.fill"
        case .audio: return "waveform"
        case .model: return "cube.fill"
        case .logic: return "smallcircle.filled.circle"
        case .project: return "shippingbox"
        case .binary: return "doc.fill"
        }
    }

    /// Per-pane jump/path bar (like Xcode split editors).
    private func panePathBar(pane: EditorPaneSnap) -> some View {
        let bufferTab = engine.chrome.tabs.first(where: {
            !$0.isLayout && $0.title == pane.title
        })
        let title = pane.title
        let dirty = pane.focused ? engine.chrome.dirty : (bufferTab?.dirty ?? false)
        return HStack(spacing: 6) {
            // The pane's own kind. Every pane's header claimed `doc.text.fill`,
            // so a split of a PDF and a PNG showed two identical text-document
            // glyphs — the row meant to tell you which pane you are in.
            Image(systemName: paneHeaderSymbol(pane.kind, title: title))
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
            paneSplitMenu(pane: pane)

            Button {
                engine.focusPane(pane.id)
                engine.closeFocusedPane()
                focused = true
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.secondary)
                    .frame(width: 24, height: 24)
            }
            .buttonStyle(.plain)
            .help("Close pane")
        }
        .padding(.horizontal, 8)
        .frame(height: ContentView.editorHeaderHeight)
        .background(pane.focused ? editorBg : shellBase.opacity(0.55))
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(pane.focused ? accent.opacity(0.35) : fg.opacity(0.08))
                .frame(height: pane.focused ? 1.5 : 1)
        }
    }

    /// Split-this-pane menu, for whatever kind of pane is asking.
    ///
    /// Shared rather than copied into each header: splitting is a property of
    /// being a pane, not of showing a document. A terminal pane had no way to
    /// split from its own header at all — the four commands were only ever in
    /// the path bar, which a shell does not have.
    ///
    /// Every item focuses the pane first. ⌃W-family commands act on the focused
    /// pane, so without it the menu split whichever pane happened to have the
    /// keyboard rather than the one whose header was clicked.
    private func paneSplitMenu(pane: EditorPaneSnap) -> some View {
        let full = engine.editorSplit.panes.count >= 4
        return Menu {
            Button("Split Left") {
                engine.focusPane(pane.id)
                engine.splitEditorLeft()
                focused = true
            }
            .disabled(full)

            Button("Split Right") {
                engine.focusPane(pane.id)
                engine.splitEditorRight()
                focused = true
            }
            .disabled(full)

            Divider()

            Button("Split Above") {
                engine.focusPane(pane.id)
                engine.splitEditorAbove()
                focused = true
            }
            .disabled(full)

            Button("Split Below") {
                engine.focusPane(pane.id)
                engine.splitEditorBelow()
                focused = true
            }
            .disabled(full)

            if full {
                Divider()
                Text("Maximum 4 panes")
            }
        } label: {
            Image(systemName: "rectangle.split.1x2")
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: 24, height: 24)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Split editor")
    }

    private func splitDivider(vertical: Bool) -> some View {
        Rectangle()
            .fill(theme.separator)
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
        // Per-keystroke scroll / hscroll / doc length now flow through
        // `editorTick` (observed by EditorHost), NOT captured here — so a
        // keystroke does not re-run this builder or the split container.
        let _ = lines // pull renderer — rows come from the engine on draw
        return EditorHost(
            wrapLines: engine.wrapLines,
            editorTick: engine.editorTick,
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
            showFocusRing: showFocusRing,
            // The unsplit editor has no `EditorPaneSnap` here; its strip is the
            // island's and covers the whole width.
            rightInset: pane.map(minimapInset(for:))
                ?? (islandShowsMinimap
                    ? minimapWidth(paneWidth: editorAreaSize.width)
                    : 0),
            relativeNumber: engine.relativeNumber
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
        PerfProbe.measure("  body.statusLine") { statusLineBody }
    }

    /// What the focused pane actually is. `.text` when there is no split to
    /// ask, which is what an editor with one document is.
    private var focusedPaneKind: PaneKind {
        let split = engine.editorSplit
        guard split.panes.indices.contains(split.focus) else { return .text }
        return split.panes[split.focus].kind
    }

    /// What a non-text pane can truthfully say.
    ///
    /// `viewerControls` is claimed by whichever viewer is on screen, so a split
    /// showing two of them has one set of controls between them. Its page and
    /// zoom are only read when it belongs to the same kind as the focused pane
    /// — the guard `ViewerControls.release` uses, for the same reason. The kind
    /// name is always true and needs nobody's permission.
    private var viewerStatusText: String {
        let kind = focusedPaneKind
        var parts = [kind.statusName]
        if viewerControls.kind == kind {
            if !viewerControls.pageLabel.isEmpty { parts.append(viewerControls.pageLabel) }
            if !viewerControls.zoomLabel.isEmpty { parts.append(viewerControls.zoomLabel) }
        }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private var statusLineBody: some View {
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
                .foregroundStyle(accent)
                .padding(.horizontal, 7)
                .padding(.vertical, 2)
                .background(Capsule(style: .continuous).fill(accent.opacity(0.12)))
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

            // The right end used to be unconditional, so an image pane reported
            // "No Wrap · Ln 1, Col 1 · 0%" — three facts about a document that
            // has no lines to be on, no wrapping to be off, and no line to be a
            // percentage through. Core has named the pane's kind the whole
            // time; the bar never asked.
            switch focusedPaneKind {
            case .text:
                if !engine.wrapLines {
                    Text("No Wrap")
                        .font(.system(size: 10, weight: .medium, design: .rounded))
                        .foregroundStyle(.secondary)
                        .help("Soft-wrap off — trackpad pans horizontally")
                }

                Text(String(
                    format: "Ln %d, Col %d",
                    engine.chrome.cursorRow, engine.chrome.cursorCol
                ))
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)

                if engine.chrome.lineCount > 0 {
                    Text("\(engine.chrome.pct)%")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .frame(minWidth: 28, alignment: .trailing)
                        .help("Scroll position")
                }

            case .terminal:
                // The focus chip on the left already says Terminal, and a shell
                // has no position this bar could report — the grid scrolls
                // itself and the caret belongs to the program running in it.
                EmptyView()

            default:
                Text(viewerStatusText)
                    .font(.system(size: 10, design: .rounded))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        // Stops before the inspector instead of sliding under it — that column
        // owns its own floor now (Xcode does the same).
        .padding(
            .trailing,
            outlineVisible ? CGFloat(inspectorW) + 12 : 12
        )
        // A plain inset. The bar used to span the whole window and slide under
        // the floating navigator, so its CONTENT had to be pushed past a panel
        // it could not see. It stops at the sidebar column now, like Xcode's.
        .padding(.leading, 12)
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
/// Shared selection geometry for Suisei's draggable mode rails. The editor
/// navigator and independent workbench both use the same motion contract.
struct TravellingPill: Shape {
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
    var onClear: () -> Void

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
                    onClear()
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

/// AppKit's real single-line search control. Xcode's find panel uses this
/// control rather than a plain text field with a separately drawn icon and
/// bezel, so the clear button, focus ring, text metrics and accessibility all
/// follow the current macOS appearance automatically.
/// The replace field.
///
/// A plain `NSTextField` rather than a `NSSearchField`: this one is not a
/// search, it has no magnifier and no history, and Return in it means "replace
/// this one" rather than "find again".
private struct NativeReplaceField: NSViewRepresentable {
    @Binding var text: String
    let onCommit: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(parent: self) }

    func makeNSView(context: Context) -> NSTextField {
        let field = NSTextField()
        field.placeholderString = "Replace"
        field.controlSize = .small
        field.font = .systemFont(ofSize: 12)
        field.bezelStyle = .roundedBezel
        field.isBordered = true
        field.delegate = context.coordinator
        field.target = context.coordinator
        field.action = #selector(Coordinator.commit(_:))
        field.setAccessibilityLabel("Replace")
        return field
    }

    func updateNSView(_ nsView: NSTextField, context: Context) {
        context.coordinator.parent = self
        if nsView.stringValue != text {
            nsView.stringValue = text
        }
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: NativeReplaceField
        init(parent: NativeReplaceField) { self.parent = parent }

        func controlTextDidChange(_ note: Notification) {
            guard let field = note.object as? NSTextField else { return }
            parent.text = field.stringValue
        }

        @objc func commit(_ sender: NSTextField) {
            parent.text = sender.stringValue
            parent.onCommit()
        }
    }
}

private struct NativeFindSearchField: NSViewRepresentable {
    @Binding var text: String

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeNSView(context: Context) -> NSSearchField {
        let field = NSSearchField()
        field.placeholderString = "Find"
        field.controlSize = .small
        field.font = .systemFont(ofSize: 12)
        field.sendsWholeSearchString = false
        field.sendsSearchStringImmediately = true
        field.target = context.coordinator
        field.action = #selector(Coordinator.searchChanged(_:))
        field.setAccessibilityLabel("Find")
        return field
    }

    func updateNSView(_ nsView: NSSearchField, context: Context) {
        context.coordinator.parent = self
        if nsView.stringValue != text {
            nsView.stringValue = text
        }
    }

    final class Coordinator: NSObject {
        var parent: NativeFindSearchField

        init(parent: NativeFindSearchField) {
            self.parent = parent
        }

        @objc func searchChanged(_ sender: NSSearchField) {
            parent.text = sender.stringValue
        }
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
    /// Sample the DESKTOP rather than the window's own content.
    ///
    /// This is the difference the sidebar's colour turns on. Measured against
    /// Xcode: its sidebar reads neutral `#F5F5F5` over a black desktop and
    /// takes on a blue cast over a purple wallpaper — because a real Mac
    /// sidebar blends behind-window and samples what is behind it. A flat
    /// colour, or `.withinWindow`, can never do that.
    ///
    /// Behind-window blending only shows through a window that is actually
    /// transparent there, so this travels with the window being non-opaque.
    var behindWindow: Bool = false

    func makeNSView(context: Context) -> NSVisualEffectView {
        let v = NSVisualEffectView()
        v.blendingMode = behindWindow ? .behindWindow : .withinWindow
        v.material = material
        v.state = .active
        apply(v)
        return v
    }

    func updateNSView(_ v: NSVisualEffectView, context: Context) {
        v.material = material
        v.blendingMode = behindWindow ? .behindWindow : .withinWindow
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
            // The 7pt strip is the drag TARGET, not a visible bar — idle it is
            // clear, so no pale slab sits between the panes. A wide tint only
            // appears while hovering or dragging.
            Rectangle()
                .fill(
                    dragging ? fg.opacity(0.14)
                        : (hovering ? fg.opacity(0.08) : .clear)
                )
            // A crisp 1px seam marks the split at rest — Xcode's separator, not
            // a bar. Both panes share the editor colour, so without this the
            // boundary would vanish.
            Rectangle()
                .fill(fg.opacity(0.09))
                .frame(width: vertical ? 1 : nil, height: vertical ? nil : 1)
            Capsule(style: .continuous)
                .fill(dragging ? accent.opacity(0.9) : fg.opacity(hovering ? 0.4 : 0))
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
struct MinimapStrip: View {
    @ObservedObject var engine: EngineBridge
    /// Observed separately, like everywhere else these small objects are used:
    /// a reload landing must not republish the shell to reach the minimap.
    @ObservedObject private var live = EngineBridge.shared.live
    /// Per-pane scroll, for the viewport box. The notification feed below only
    /// speaks for the focused pane.
    @ObservedObject private var tick = EngineBridge.shared.editorTick
    /// Which pane this strip summarises; `-1` is the unsplit editor.
    ///
    /// Everything here used to be the LIVE document's — the bars, the viewport
    /// box, the jump target — which is correct while only the focused pane has
    /// a strip and wrong the moment they all do: pane A drew pane B's file,
    /// with pane B's scroll position boxed on it.
    var paneIndex: Int = -1
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

    /// Whether this strip belongs to the pane holding the keyboard. The
    /// unsplit editor is one pane and always does.
    private var isFocusedPane: Bool {
        paneIndex < 0 || paneIndex == engine.editorSplit.focus
    }

    var body: some View {
        GeometryReader { geo in
            let data = engine.minimapData(pane: paneIndex)
            ZStack(alignment: .topLeading) {
                // Bars redraw ONLY when the document data changes — redrawing
                // 2k rects on every scroll tick was the minimap stutter.
                // The map grows and shrinks WITH the editor.
                //
                // The bars are already the new document — the data refreshes
                // on the reload — so what was missing was the motion, not the
                // content: the column snapped to its new length while the
                // rows beside it were still sliding. Scaled about the top,
                // which is where a document grows from.
                //
                // A scale rather than interpolating the bar list: the bars are
                // discrete and there is no half a row, so stretching the
                // column is both the honest picture and the cheap one.
                TimelineView(.animation(paused: !live.isShifting)) { _ in
                    let p = live.shiftProgress()
                    let grew = CGFloat(live.shift?.rows ?? 0)
                    let n = CGFloat(max(1, data?.len.count ?? 1))
                    // Where the column started: this many rows fewer (or more,
                    // for a removal) than it has now.
                    let from = max(0.05, (n - grew) / n)
                    MinimapBars(
                        data: data, accent: accent, fg: fg, isLight: isLight,
                        rowH: rowHeight(data?.len.count ?? 1, stripHeight: geo.size.height)
                    )
                    .equatable()
                    .scaleEffect(
                        x: 1, y: live.isShifting ? from + (1 - from) * p : 1,
                        anchor: .top
                    )
                }

                // Viewport indicator — a cheap offset move at frame rate,
                // mapped over the RENDERED height (≠ strip height for small
                // files).
                if let data, data.totalLines > 0, !data.len.isEmpty {
                    let n = data.len.count
                    let rowH = rowHeight(n, stripHeight: geo.size.height)
                    let mapH = CGFloat(n) * rowH
                    let total = CGFloat(data.totalLines)
                    let visRows = geo.size.height / max(1, EditorMetrics.lineHeight)
                    // The live feed is posted only by the focused pane's clip,
                    // so an unfocused strip reads its own pane's scroll from
                    // the per-keystroke store instead of inheriting a position
                    // from the pane the user is actually in.
                    let line = isFocusedPane
                        ? (liveScrollLine >= 0 ? liveScrollLine : Int(engine.chrome.scroll))
                        : Int(tick.tick(for: paneIndex).scroll)
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

                    // Where a live reload landed — the whole point of putting
                    // it here is the part that is OFF SCREEN. The editor's
                    // flash can only speak for rows in the band; this speaks
                    // for the file.
                    //
                    // Shaped like the viewport indicator on purpose: the two
                    // say the same kind of thing ("this region, right now")
                    // and a second visual language for that would be noise.
                    if !live.rows.isEmpty {
                        // The fade is read from a clock, so this has to be
                        // re-evaluated per frame while it runs. Present only
                        // while there is something to fade — a `TimelineView`
                        // left standing would drive the minimap at frame rate
                        // for the whole session.
                        TimelineView(.animation) { _ in
                            ForEach(liveBoxes(mapH: mapH, total: total), id: \.id) { box in
                                RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                                    .fill(box.color.opacity(0.20 * box.fade))
                                    .overlay(
                                        RoundedRectangle(
                                            cornerRadius: Radius.row, style: .continuous
                                        )
                                        .strokeBorder(
                                            box.color.opacity(0.75 * box.fade), lineWidth: 1
                                        )
                                    )
                                    .frame(width: geo.size.width - 2, height: box.height)
                                    .offset(x: 1, y: box.y)
                                    .allowsHitTesting(false)
                            }
                        }
                    }
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
                guard isFocusedPane, let line = note.userInfo?["line"] as? Int else { return }
                liveScrollLine = line
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

    private struct LiveBox: Identifiable {
        let id: Int
        let y: CGFloat
        let height: CGFloat
        let color: Color
        let fade: CGFloat
    }

    /// Contiguous runs of marked rows, as boxes over the map.
    ///
    /// Runs rather than one box per row: a reload usually replaces a band, and
    /// twenty separate 1pt boxes read as noise where one box reads as a place.
    /// A box is never thinner than 3pt — a single changed line in a 4,000 line
    /// file would otherwise be invisible, which is exactly the case this
    /// exists for.
    private func liveBoxes(mapH: CGFloat, total: CGFloat) -> [LiveBox] {
        let marks = engine.live.rows
        guard !marks.isEmpty, total > 0 else { return [] }
        let rows = marks.keys.sorted()
        var out: [LiveBox] = []
        var runStart = rows[0]
        var prev = rows[0]

        func flush(_ last: UInt32) {
            let kind = marks[runStart] ?? .changed
            let top = CGFloat(runStart) / total * mapH
            let bottom = CGFloat(last + 1) / total * mapH
            out.append(LiveBox(
                id: Int(runStart),
                y: max(0, min(top, mapH - 3)),
                height: max(3, bottom - top),
                color: kind == .removed ? Color(nsColor: .systemRed) : accent,
                fade: engine.live.intensity(runStart)
            ))
        }

        for row in rows.dropFirst() {
            if row != prev + 1 {
                flush(prev)
                runStart = row
            }
            prev = row
        }
        flush(prev)
        return out.filter { $0.fade > 0.01 }
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
        // Focus first when the strip belongs to another pane. Only the focused
        // pane reports its clip position back to core, so scrolling an
        // unfocused one would leave core's idea of that document's scroll
        // behind — and the next structural update would snap it back. Focusing
        // is also what the click means: you pointed at that file.
        if !isFocusedPane { engine.focusPane(paneIndex) }
        NotificationCenter.default.post(
            name: .suiseiScrollToLine,
            object: nil,
            userInfo: ["line": line, "pane": paneIndex]
        )
    }
}

// The docked terminal's renderer used to live here: `TerminalGridView`, the
// `TermScroll` that forwarded wheel events past its own ends into core's
// scrollback, the `TermCanvas` that drew a cell grid and implemented
// `NSTextInputClient` so Hangul could be composed into a PTY, and an SGR
// parser to turn core's re-encoded rows back into colours. About 750 lines,
// all of it a second emulator's worth of work done to display the first one.
// SwiftTerm draws its own cells.


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

/// Keeps the window-owned player alive across pane replacement, but not across
/// closing its owning document tab. Isolated from `ContentView.body` because
/// that modifier chain is already near Swift's type-checking complexity limit.
private struct AudioTabLifetimeModifier: ViewModifier {
    let engine: EngineBridge
    @ObservedObject var player: AudioPlayerModel

    func body(content: Content) -> some View {
        // Every route that can mutate the open-document set already republishes
        // chrome: tab ×, Cmd-W, palette commands and project replacement alike.
        // Checking the stable id here is both broader and safer than wiring the
        // player to one particular close button implementation.
        content.onReceive(engine.$chrome) { _ in
            if player.sourceTabId == 0, !player.sourcePath.isEmpty,
               let pane = engine.editorSplit.panes.first(where: {
                   $0.kind == .audio && Self.sameFile($0.path, player.sourcePath)
               })
            {
                player.adoptTabIdIfMissing(pane.tabStableId)
            }
            let id = player.sourceTabId
            guard id != 0, !engine.tabIdIsOpen(id)
            else { return }
            player.close()
        }
    }

    private static func sameFile(_ lhs: String, _ rhs: String) -> Bool {
        URL(fileURLWithPath: lhs).standardizedFileURL.path
            == URL(fileURLWithPath: rhs).standardizedFileURL.path
    }
}

/// The two window bits that are the EDITOR's alone, applied to its own window.
///
/// Full-size content is what lets the split view's sidebar rise THROUGH the
/// transparent titlebar with AppKit's traffic lights floating over it. Without
/// it the content view starts below the titlebar, `detailStack`'s
/// `.ignoresSafeArea(.top)` has nothing to ignore, and the window grows a 28pt
/// empty strip above the tab row. Hidden title, because this window has nowhere
/// to draw one.
///
/// Settings deliberately gets neither — SwiftUI owns its titlebar geometry —
/// which is why these are here and not in `applyThemedTitlebar`.
///
/// A representable rather than a branch inside `applyWindowAppearance`: that
/// function finds its windows by `identifier`, and the identifier is set
/// asynchronously by `ThemedWindowChrome`, so on a cold launch it could run
/// against a window that matched nothing yet. This reads `nsView.window`.
///
/// Guarded, like everything else that touches window style here: re-assigning
/// identical values on reactivation rebuilds the window's view bridge and kills
/// NSHostingView hit-testing.
private struct EditorWindowChrome: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        view.isHidden = true
        DispatchQueue.main.async { apply(view) }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async { apply(nsView) }
    }

    private func apply(_ view: NSView) {
        guard let window = view.window else { return }
        if !window.styleMask.contains(.fullSizeContentView) {
            window.styleMask.insert(.fullSizeContentView)
        }
        // Belt and braces with `allowsAutomaticWindowTabbing = false` in the
        // app delegate. That flag is class-wide and set once at launch; this is
        // the editor window saying it for itself, on every update, because the
        // editor is the only WindowGroup here — the one scene that can produce
        // a second window for AppKit to want to tab together.
        if window.tabbingMode != .disallowed {
            window.tabbingMode = .disallowed
        }
        // The traffic lights are NOT repositioned here. There used to be a 2pt
        // optical nudge, and it could not be made to hold: it was right at
        // launch and gone forever after the first sidebar toggle. Logged, one
        // toggle:
        //
        //     light apply        cur 19.00 → target 17.00
        //       wrote 17.00 → frame now 17.00   constraints 4
        //     light frameChanged cur 19.00 → target 17.00
        //       wrote 17.00 → frame now 19.00   constraints 4
        //
        // The observer fired, the baseline survived, the target was right, and
        // the write simply did not take: collapsing the navigator brings
        // `NSTitlebarView` under Auto Layout, and a frame written into a view
        // the layout engine owns is reverted before the next line runs. There
        // is no version of this that holds.
        //
        // It was also correcting the wrong way. `topBandHeight`'s own note
        // records the measurement (`scripts/sidebar_probe6.swift`): with a
        // toolbar present AppKit puts the lights at 26pt from the window top,
        // which is `topBandHeight / 2 + titlebarDrop` exactly — the tab row's
        // centreline. Native is already aligned; the nudge was pushing them
        // 2pt off it.
        //
        // If this row ever needs to move again, move `titlebarDrop`, which is
        // our own drawing. Nothing here writes AppKit geometry now, which is
        // the same reason `SplitColumnWidthReporter` only reads.
        // No `titleVisibility = .hidden` here, deliberately. It hides the title
        // by removing the toolbar's title ITEM, and that item is what anchors
        // the `.primaryAction` group to the trailing edge — hiding it moved the
        // controls to x 98…490 in a 1280pt window
        // (`scripts/sidebar_probe10.swift`). `.navigationTitle("")` on the
        // split view draws nothing and keeps the anchor.
        //
        // The flexible space this used to insert into SwiftUI's NSToolbar is
        // gone with it: it was a workaround for a placement problem that turned
        // out to be this line.
    }

}

/// A fixed titlebar slot for the navigator control.
///
/// The accessory owns placement only: unlike `.navigation`, this slot is
/// relative to the window and therefore never follows the sidebar divider.
/// Its button deliberately has no resting platter; it matches the trailing
/// toolbar symbol at rest and reveals its target only while hovering.
private struct EditorNavigatorToolbarAccessory: NSViewRepresentable {
    var isVisible: Bool
    var action: () -> Void

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        view.isHidden = true
        DispatchQueue.main.async { install(from: view) }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async { install(from: nsView) }
    }

    private func install(from view: NSView) {
        guard let window = view.window else { return }
        let controller: EditorNavigatorAccessoryController
        if let existing = window.titlebarAccessoryViewControllers
            .compactMap({ $0 as? EditorNavigatorAccessoryController })
            .first
        {
            controller = existing
        } else {
            controller = EditorNavigatorAccessoryController()
            window.addTitlebarAccessoryViewController(controller)
        }
        controller.update(isVisible: isVisible, action: action)
    }
}

private struct EditorNavigatorButton: View {
    var isVisible: Bool
    var action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: "sidebar.leading")
                // This accessory does not inherit `ToolbarItem`'s symbol
                // environment, so its default glyph is visibly smaller than
                // the trailing `sidebar.right`. Match that toolbar glyph's
                // optical size without changing the right-hand control.
                .font(.system(size: 16.5, weight: .regular))
                .foregroundStyle(.primary)
                .frame(width: 28, height: 28)
                .background {
                    Circle()
                        .fill(Color.primary.opacity(hovering ? 0.10 : 0))
                }
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .help(isVisible ? "Hide Navigator · ⌘0" : "Show Navigator · ⌘0")
        .accessibilityLabel("Navigators")
        .frame(width: 36, height: 36)
        .onHover { hovering = $0 }
        .animation(.easeOut(duration: 0.10), value: hovering)
    }
}

private final class EditorNavigatorAccessoryController: NSTitlebarAccessoryViewController {
    private let host = EditorNavigatorAccessoryHost()

    init() {
        super.init(nibName: nil, bundle: nil)
        layoutAttribute = .left
        view = host
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(isVisible: Bool, action: @escaping () -> Void) {
        host.hosting.rootView = EditorNavigatorButton(
            isVisible: isVisible,
            action: action
        )
    }
}

private final class EditorNavigatorAccessoryHost: NSView {
    let hosting = NSHostingView(
        rootView: EditorNavigatorButton(isVisible: true, action: {})
    )

    init() {
        super.init(frame: NSRect(x: 0, y: 0, width: 36, height: 52))
        hosting.frame = NSRect(x: 0, y: 8, width: 36, height: 36)
        addSubview(hosting)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func layout() {
        super.layout()
        var centreY = bounds.midY
        if let zoom = window?.standardWindowButton(.zoomButton) {
            let zoomInWindow = zoom.convert(zoom.bounds, to: nil)
            centreY = convert(NSPoint(x: 0, y: zoomInWindow.midY), from: nil).y
        }
        hosting.frame = NSRect(x: 0, y: centreY - 18, width: 36, height: 36)
    }
}

/// Removes the divider-relative toggle that `NavigationSplitView` can recreate
/// after `.toolbar(removing: .sidebarToggle)` during reconciliation.
private struct EditorGeneratedSidebarTogglePruner: NSViewRepresentable {
    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        view.isHidden = true
        context.coordinator.attach(to: view)
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.attach(to: nsView)
    }

    final class Coordinator {
        private weak var toolbar: NSToolbar?
        private var willAddObserver: NSObjectProtocol?
        private var didRemoveObserver: NSObjectProtocol?
        private var normalizing = false

        deinit {
            if let willAddObserver { NotificationCenter.default.removeObserver(willAddObserver) }
            if let didRemoveObserver { NotificationCenter.default.removeObserver(didRemoveObserver) }
        }

        func attach(to view: NSView) {
            DispatchQueue.main.async { [weak self, weak view] in
                guard let self, let nextToolbar = view?.window?.toolbar else { return }
                if toolbar !== nextToolbar {
                    stopObserving()
                    toolbar = nextToolbar
                    let centre = NotificationCenter.default
                    willAddObserver = centre.addObserver(
                        forName: NSToolbar.willAddItemNotification,
                        object: nextToolbar,
                        queue: .main
                    ) { [weak self] note in
                        // BEFORE the add, so the item never gets a frame on
                        // screen. `willAddItem` fires while the item is not yet
                        // in `toolbar.items`, so it cannot be removed here —
                        // only deferred, and that deferral is one whole frame:
                        // switching a layout tab against a document tab
                        // reinstalls the split view, SwiftUI regenerates the
                        // toggle, and it flashed beside the divider before the
                        // next run loop took it away.
                        //
                        // It carries the item, though, and a hidden item is
                        // laid out at zero size. The removal below still
                        // happens; this only makes it invisible while it
                        // exists.
                        if let item = note.userInfo?["item"] as? NSToolbarItem,
                           Coordinator.isGeneratedSidebarToggle(item.itemIdentifier)
                        {
                            item.isHidden = true
                            item.isEnabled = false
                        }
                        self?.normalizeOnNextRunLoop()
                    }
                    didRemoveObserver = centre.addObserver(
                        forName: NSToolbar.didRemoveItemNotification,
                        object: nextToolbar,
                        queue: .main
                    ) { [weak self] _ in self?.normalizeOnNextRunLoop() }
                }
                normalize()
            }
        }

        private func stopObserving() {
            let centre = NotificationCenter.default
            if let willAddObserver { centre.removeObserver(willAddObserver) }
            if let didRemoveObserver { centre.removeObserver(didRemoveObserver) }
            willAddObserver = nil
            didRemoveObserver = nil
        }

        private func normalizeOnNextRunLoop() {
            DispatchQueue.main.async { [weak self] in self?.normalize() }
        }

        /// Either form of the divider-relative system item.
        static func isGeneratedSidebarToggle(
            _ identifier: NSToolbarItem.Identifier
        ) -> Bool {
            if identifier == .toggleSidebar { return true }
            let name = identifier.rawValue.lowercased()
            return name.contains("navigationsplitview")
                && name.contains("togglesidebar")
        }

        private func normalize() {
            guard !normalizing, let toolbar else { return }
            normalizing = true
            defer { normalizing = false }

            // The visible accessory is not part of `toolbar.items`.
            for index in toolbar.items.indices.reversed()
            where Coordinator.isGeneratedSidebarToggle(
                toolbar.items[index].itemIdentifier
            ) {
                toolbar.removeItem(at: index)
            }
        }
    }
}

/// The sidebar's dragged width, reported back so it survives relaunch.
///
/// The split view owns the navigator's width now, and `PanelResizeGrip` — which
/// used to own it and wrote straight into `navW` — went with the floating card.
/// Without this the column would silently reset to its `ideal:` on every
/// launch: a capability the editor had, quietly lost.
///
/// It only reads. It walks up to the enclosing `NSSplitView` and reports the
/// first pane's width whenever AppKit finishes laying the split out. Nothing
/// here writes AppKit geometry, so there is no frame to race — which is the
/// whole reason the traffic-light overlay it replaced had to exist.
struct SplitColumnWidthReporter: NSViewRepresentable {
    var onWidth: (Double) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        view.isHidden = true
        context.coordinator.attach(to: view, report: onWidth)
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.attach(to: nsView, report: onWidth)
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        private weak var splitView: NSSplitView?
        private var report: ((Double) -> Void)?
        private var observer: NSObjectProtocol?

        func attach(to view: NSView, report: @escaping (Double) -> Void) {
            self.report = report
            guard splitView == nil else { return }
            // Deferred: at `makeNSView` time the view has no ancestors to walk.
            DispatchQueue.main.async { [weak self, weak view] in
                guard let self, let view, self.splitView == nil else { return }
                var candidate: NSView? = view.superview
                while let next = candidate, !(next is NSSplitView) {
                    candidate = next.superview
                }
                guard let split = candidate as? NSSplitView else { return }
                self.splitView = split
                self.observer = NotificationCenter.default.addObserver(
                    forName: NSSplitView.didResizeSubviewsNotification,
                    object: split,
                    queue: .main
                ) { [weak self] _ in self?.publishWidth() }
                // A restored split does not necessarily resize after this
                // observer attaches. Publish its laid-out width now; otherwise
                // the titlebar can retain the 280pt bootstrap value forever and
                // let the first tabs sit under a wider navigator.
                publishWidth()
            }
        }

        private func publishWidth() {
            guard let split = splitView,
                  let first = split.arrangedSubviews.first
            else { return }
            // A collapsed pane keeps its old frame width, so the width alone
            // cannot say "the sidebar is shut". Ask the split view and preserve
            // zero as a real boundary state.
            let collapsed = split.isSubviewCollapsed(first) || first.isHidden
            report?(collapsed ? 0 : Double(first.frame.width))
        }

        deinit {
            if let observer { NotificationCenter.default.removeObserver(observer) }
        }
    }
}

private extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}
