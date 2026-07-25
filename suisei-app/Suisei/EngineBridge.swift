import Foundation
import Combine
import AppKit
import SwiftUI

enum SuiseiKey: UInt32 {
    case char_ = 1, enter = 2, esc = 3, backspace = 4, tab = 5, backtab = 6
    case delete = 7, left = 8, right = 9, up = 10, down = 11
    case home = 12, end = 13, pageUp = 14, pageDown = 15, f = 16
}

// Suisei is a plain GUI editor: Core stays modal internally (shared with the
// xei TUI), but the face never surfaces modes — typing always types, Esc only
// clears selection/overlays, and panels own their keys while focused.

struct SuiseiMod: OptionSet {
    let rawValue: UInt8
    static let shift = SuiseiMod(rawValue: 1)
    static let control = SuiseiMod(rawValue: 2)
    static let alt = SuiseiMod(rawValue: 4)
    static let superKey = SuiseiMod(rawValue: 8)
}

struct SyntaxSpan: Equatable {
    var start: UInt16
    var end: UInt16
    var kind: UInt8
}

struct EditorLine: Equatable, Identifiable {
    /// Stable identity — include wrap segment so soft-wrap rows don't collide.
    var id: String { "\(paneId)-\(lineNo)-\(gitSign)-\(text.prefix(24).hashValue)" }
    /// Which split pane this row belongs to (0 when unsplit).
    var paneId: UInt8
    var lineNo: UInt32
    var text: String
    var isCursor: Bool
    var caretVCol: UInt32
    /// Caret as a UTF-16 offset into `text`. `caretVCol` is the core's TERMINAL
    /// cell column (CJK = 2 cells); laying the caret out at `vcol * cellWidth`
    /// drifts right of the real glyphs on Hangul/Japanese lines, so the canvas
    /// resolves this offset against the drawn CTLine instead.
    var caretUTF16: UInt32
    var selV0: UInt32
    var selV1: UInt32
    /// Selection as UTF-16 offsets into `text` — laid out with real glyph
    /// advances, unlike the core's cell-grid `selV0/selV1`.
    var selU0: UInt32
    var selU1: UInt32
    /// Git gutter: low 7 bits 0 none / 1 add / 2 mod / 3 del; bit 0x80 = soft-wrap continuation.
    var gitSign: UInt8
    var spans: [SyntaxSpan]
    var hasSelection: Bool { selV0 != UInt32.max && selV1 != UInt32.max && selV1 > selV0 }
    var isWrapContinuation: Bool { (gitSign & 0x80) != 0 }
    /// DAP breakpoint on this buffer line (first visual segment only).
    var hasBreakpoint: Bool { (gitSign & 0x40) != 0 }
    var gitSignKind: UInt8 { gitSign & 0x3F }
}

/// One editor split surface (or the single full editor when unsplit).
struct EditorPaneSnap: Equatable, Identifiable {
    var id: Int
    var focused: Bool
    var tabIndex: Int
    var scroll: UInt32
    var hscroll: UInt32
    /// Total lines in this pane's buffer (scrollbar / clamp).
    var docLineCount: UInt32
    var lines: [EditorLine]
}

/// 0 = none, 1 = vertical (side-by-side), 2 = horizontal (stacked).
struct SplitSnap: Equatable {
    var kind: UInt8
    var ratio: Float
    var focus: Int
    var panes: [EditorPaneSnap]
    var isSplit: Bool { kind != 0 && panes.count >= 2 }
    static let empty = SplitSnap(kind: 0, ratio: 0.5, focus: 0, panes: [])
}

struct TabItem: Equatable, Identifiable {
    var id: Int
    var title: String
    var dirty: Bool
    var active: Bool
}

struct ExplorerEntry: Equatable, Identifiable {
    var id: Int
    var name: String
    var isDir: Bool
    var selected: Bool
}

struct ExplorerSnap: Equatable {
    var open: Bool
    var cwd: String
    var selected: Int
    var entries: [ExplorerEntry]
    static let empty = ExplorerSnap(open: false, cwd: "", selected: 0, entries: [])
}

struct OutlineItem: Equatable, Identifiable {
    var id: Int
    var name: String
    /// 1-based line
    var row: UInt32
    /// 0=header, 1=fn, 2=type, 3=other
    var kind: UInt8
    var depth: UInt8
}

struct BreakpointItem: Equatable, Identifiable {
    var id: String { "\(path)#\(line)" }
    var path: String
    var name: String
    /// 1-based line
    var line: UInt32
    var verified: Bool
    var condition: String
    var hasLog: Bool
}

struct DiagnosticItem: Equatable, Identifiable {
    var id: String { "\(row):\(col):\(message)" }
    /// 0-based
    var row: UInt32
    var col: UInt32
    /// 0 error · 1 warning · 2 info · 3 hint
    var severity: UInt8
    var message: String
}

struct SearchHitItem: Equatable, Identifiable {
    var id: String { "\(path)#\(row):\(col)" }
    var path: String
    /// 0-based
    var row: UInt32
    var col: UInt32
    var line: String
    var name: String { (path as NSString).lastPathComponent }
}

struct PreviewLineItem: Equatable, Identifiable {
    var id: Int
    var text: String
    var style: UInt8
}

struct PreviewSnap: Equatable {
    var open: Bool
    var kind: UInt8
    var scroll: UInt32
    var hscroll: UInt32
    var lines: [PreviewLineItem]
    static let empty = PreviewSnap(open: false, kind: 0, scroll: 0, hscroll: 0, lines: [])
    var kindLabel: String {
        switch kind {
        case 1: return "Markdown"
        case 2: return "JSON"
        case 3: return "Plain"
        case 4: return "Image"
        case 5: return "CSV"
        case 6: return "NumPy"
        case 7: return "Audio"
        default: return "Preview"
        }
    }
}


/// Which surface owns the keyboard, mirroring `suisei_core::app::Mode`.
///
/// The face used to decide this by substring-matching a vim status badge
/// (` NORMAL `, ` INSERT `, ` VISUAL `…) that the compositor emitted for the
/// status line. The engine now sends the focus itself; parse it once, here.
enum Focus: String {
    case editor = "EDITOR"
    case explorer = "EXPLORER"
    case terminal = "TERMINAL"
    case search = "SEARCH"
    case palette = "PALETTE"
    case scm = "SCM"
    case git = "GIT"
    case settings = "SETTINGS"
    case preview = "PREVIEW"
    case workspaceFind = "FIND"
    case debug = "DEBUG"
    case calls = "CALLS"
    case unknown = ""

    init(label: String) {
        self = Focus(rawValue: label.trimmingCharacters(in: .whitespaces).uppercased()) ?? .unknown
    }

    /// A panel that owns typed characters — neither the editor nor the PTY.
    var ownsTyping: Bool {
        switch self {
        case .editor, .terminal, .unknown: return false
        default: return true
        }
    }

    /// Key handling here mutates shell surfaces the light pull set never fetches.
    var wantsFullChrome: Bool {
        switch self {
        case .explorer, .scm, .git, .settings, .workspaceFind, .debug, .calls: return true
        default: return false
        }
    }
}

struct PaletteItem: Equatable, Identifiable {
    var id: Int
    var label: String
    var detail: String
    var selected: Bool
}

struct PaletteSnap: Equatable {
    var open: Bool
    var kind: String
    var query: String
    var items: [PaletteItem]
    static let empty = PaletteSnap(open: false, kind: "", query: "", items: [])
}

struct SearchSnap: Equatable {
    var open: Bool
    var forward: Bool
    var input: String
    var matchCount: UInt32
    var matchIndex: UInt32
    static let empty = SearchSnap(open: false, forward: true, input: "", matchCount: 0, matchIndex: 0)
}

struct HintRow: Equatable {
    var key: String
    var desc: String
}

struct CompRow: Equatable {
    var label: String
    var detail: String
}

struct CompletionsSnap: Equatable {
    var open: Bool
    var prefix: String
    var selected: Int
    var items: [CompRow]
    static let empty = CompletionsSnap(open: false, prefix: "", selected: 0, items: [])
}

struct TerminalSnap: Equatable {
    var open: Bool
    var fullPanel: Bool
    /// Split pane showing full terminal (`nil` = whole main or side panel).
    var paneBound: Int?
    var lines: [String]
    /// Shell cursor within `lines`. Never crossed the bridge before, which is
    /// why the terminal had no caret at all.
    var cursorRow: Int = 0
    var cursorCol: Int = 0
    static let empty = TerminalSnap(open: false, fullPanel: false, paneBound: nil, lines: [])

    /// Whether this editor split pane should paint the full-panel terminal.
    func isBoundToPane(_ paneId: Int) -> Bool {
        guard open, fullPanel else { return false }
        if let b = paneBound { return b == paneId }
        return true // whole-main fallback
    }
}

struct ScmEntryItem: Equatable, Identifiable {
    var id: Int
    var path: String
    var mark: String
    var staged: Bool
    var selected: Bool
}

struct ScmGraphItem: Equatable, Identifiable {
    var id: Int
    var line: String
    var selected: Bool
}

struct ScmSnap: Equatable {
    var open: Bool
    var branch: String
    var status: String
    var staged: [ScmEntryItem]
    var changes: [ScmEntryItem]
    var graph: [ScmGraphItem]
    static let empty = ScmSnap(
        open: false, branch: "", status: "", staged: [], changes: [], graph: []
    )
}

struct GitWbChipItem: Equatable, Identifiable {
    var id: Int
    var label: String
    var active: Bool
    var key: Int
}

struct GitWbSnap: Equatable {
    var open: Bool
    var docked: Bool
    var loading: Bool
    var tabIndex: Int
    var branch: String
    var message: String
    var chips: [GitWbChipItem]
    var colChanges: [String]
    var colLog: [String]
    var colFiles: [String]
    var special: [String]
    static let empty = GitWbSnap(
        open: false, docked: false, loading: false, tabIndex: 0,
        branch: "", message: "", chips: [],
        colChanges: [], colLog: [], colFiles: [], special: []
    )
}

struct SettingsRowItem: Equatable, Identifiable {
    var id: Int
    var label: String
    var value: String
    var isHeader: Bool
    var selected: Bool
}

struct SettingsSnap: Equatable {
    var open: Bool
    var dirty: Bool
    var pageIndex: Int
    var selected: Int
    var status: String
    var tabs: [String]
    var rows: [SettingsRowItem]
    static let empty = SettingsSnap(
        open: false, dirty: false, pageIndex: 0, selected: 0,
        status: "", tabs: [], rows: []
    )
}

/// Live theme from Core (0x00RRGGBB).
struct ThemeSnap: Equatable {
    var name: String
    var editorBg: UInt32
    var fg: UInt32
    var dim: UInt32
    var accent: UInt32
    var selection: UInt32
    var caret: UInt32
    var statusBg: UInt32
    var keyword: UInt32
    var string: UInt32
    var comment: UInt32
    var number: UInt32
    var typeName: UInt32
    var function: UInt32
    var macroName: UInt32
    var namespace: UInt32
    var parameter: UInt32
    var property: UInt32
    var constant: UInt32
    var operatorColor: UInt32
    var punctuation: UInt32

    static let empty = ThemeSnap(
        name: "ocean",
        editorBg: 0x0F111A, fg: 0xC8D2DC, dim: 0x525C72, accent: 0x6BB8C4,
        selection: 0x2A3A55, caret: 0xC8E08C, statusBg: 0x0A0C14,
        keyword: 0x00DCFF, string: 0x96E6B4, comment: 0x606C7A,
        number: 0xFFB482, typeName: 0x64C8FF, function: 0xFFDC78,
        macroName: 0xFD8F3F, namespace: 0x9EF1DD, parameter: 0xC8C8CD, property: 0x78C3B4, constant: 0xD0BF69, operatorColor: 0xDDDDDD, punctuation: 0x94949B
    )

    /// Theme colours arrive packed as `0xAARRGGBB`. Alpha is real: chrome
    /// tokens use it so separators composite over whatever is behind them,
    /// the way `NSColor.separatorColor` does, instead of baking an opaque grey.
    func color(_ packed: UInt32) -> Color {
        let a = Double((packed >> 24) & 0xFF) / 255.0
        let r = Double((packed >> 16) & 0xFF) / 255.0
        let g = Double((packed >> 8) & 0xFF) / 255.0
        let b = Double(packed & 0xFF) / 255.0
        return Color(red: r, green: g, blue: b).opacity(a)
    }
}

struct ChromeSnapshot: Equatable {
    var gen: UInt64
    var modeLabel: String
    var message: String
    var filename: String
    var breadcrumbs: String
    var dirty: Bool
    var welcome: Bool
    var explorerOpen: Bool
    var cursorRow: UInt32
    var cursorCol: UInt32
    var caretVCol: UInt32
    /// Why Core last moved the scroll: 0 none, 1 restore, 2 navigate, 3 caret.
    var scrollIntent: UInt8 = 0
    var lineCount: UInt32
    var scroll: UInt32
    var pct: UInt32
    var bufferVersion: UInt64
    var branch: String
    var tabs: [TabItem]
    /// Focused-pane lines (compat). Prefer `split` / `editorLines` for paint.
    var lines: [EditorLine]
    var split: SplitSnap
    var explorer: ExplorerSnap
    var palette: PaletteSnap
    var search: SearchSnap
    var completions: CompletionsSnap
    var terminal: TerminalSnap
    var settings: SettingsSnap
    var theme: ThemeSnap
    var scm: ScmSnap
    var gitWb: GitWbSnap
    var outline: [OutlineItem]

    static let empty = ChromeSnapshot(
        gen: 0, modeLabel: " … ", message: "loading", filename: "",
        breadcrumbs: "", dirty: false, welcome: true, explorerOpen: false,
        cursorRow: 1, cursorCol: 1, caretVCol: 0, lineCount: 1, scroll: 0,
        pct: 0, bufferVersion: 0, branch: "", tabs: [], lines: [],
        split: .empty,
        explorer: .empty, palette: .empty, search: .empty,
        completions: .empty, terminal: .empty,
        settings: .empty, theme: .empty, scm: .empty, gitWb: .empty,
        outline: []
    )
}

/// Geometry constants shared with editor paint (must match hit-test).
/// Font size is adjustable via Cmd+ / Cmd- (persisted).
enum EditorMetrics {
    /// Xcode-like line-number strip (digits + git stripe + pad) — not a wide slab.
    static let gutterBase: CGFloat = 34
    static let defaultFontSize: CGFloat = 14
    static let minFontSize: CGFloat = 10
    static let maxFontSize: CGFloat = 28
    static let linePad: CGFloat = 4
    /// Space between trailing line number and code (Cursor/VS Code–like air gap).
    static let gutterTextGap: CGFloat = 12
    static let gitStripeWidth: CGFloat = 3

    /// Live face font size — mutated by zoom; not a `let` so resize can recompute.
    static var fontSize: CGFloat = {
        let v = UserDefaults.standard.double(forKey: "suisei.fontSize")
        if v >= minFontSize && v <= maxFontSize { return CGFloat(v) }
        return defaultFontSize
    }()

    static var gutter: CGFloat {
        // Digits strip stays compact; air gap is gutterTextGap before code (not a wide slab).
        // At 14pt ≈ 36–40pt total with ~12pt gap.
        let digitsW = cellWidth * 3.4 + gitStripeWidth + 6
        let total = digitsW + gutterTextGap
        let scaled = (gutterBase + 4) * (fontSize / defaultFontSize)
        return min(44, max(32, max(total, scaled)))
    }

    static var cellWidth: CGFloat {
        let font = NSFont.monospacedSystemFont(ofSize: fontSize, weight: .medium)
        return max(7, ceil(("M" as NSString).size(withAttributes: [.font: font]).width))
    }

    static var lineHeight: CGFloat { fontSize + linePad * 2 }

    @discardableResult
    static func adjustFont(delta: CGFloat) -> CGFloat {
        fontSize = min(maxFontSize, max(minFontSize, fontSize + delta))
        UserDefaults.standard.set(Double(fontSize), forKey: "suisei.fontSize")
        return fontSize
    }

    static func resetFont() {
        fontSize = defaultFontSize
        UserDefaults.standard.set(Double(fontSize), forKey: "suisei.fontSize")
    }
}

final class EngineBridge: ObservableObject {
    @Published private(set) var chrome: ChromeSnapshot = .empty
    /// Editor paint surface — updated on scroll without re-emitting full chrome shell.
    @Published private(set) var editorLines: [EditorLine] = []
    @Published private(set) var editorSplit: SplitSnap = .empty
    /// From Core only (`snap.scroll_frac`). Never a parallel face accumulator.
    @Published private(set) var editorScrollFrac: CGFloat = 0
    /// Horizontal pan (visual columns) when wrap is off.
    @Published private(set) var editorHScroll: UInt32 = 0
    @Published private(set) var wrapLines: Bool = true
    /// Pretty preview (Ctrl/Cmd+Shift+V).
    @Published private(set) var preview: PreviewSnap = .empty
    /// Bumped on font zoom so SwiftUI rebuilds line metrics / paint.
    @Published private(set) var fontGeneration: UInt64 = 0
    /// Breakpoints navigator list (from Core DAP store).
    @Published private(set) var breakpoints: [BreakpointItem] = []
    @Published private(set) var diagnostics: [DiagnosticItem] = []
    @Published private(set) var searchHits: [SearchHitItem] = []
    @Published private(set) var searchTruncated: Bool = false
    @Published private(set) var searchRunning: Bool = false
    /// Why a search produced nothing, when the reason is not "no matches".
    @Published private(set) var searchMessage: String = ""
    /// Find All References (LSP) — reuses the search-hit row shape. `active`
    /// drives the References view taking over the Find navigator; `ready`
    /// distinguishes "still waiting on the server" from "resolved, 0 refs".
    @Published private(set) var references: [SearchHitItem] = []
    @Published var referencesActive: Bool = false
    @Published private(set) var referencesReady: Bool = false
    @Published private(set) var referencesTruncated: Bool = false
    @Published private(set) var hoverText: String = ""
    private var hoverPoll: DispatchWorkItem?
    /// Discards results from a superseded query — the user types faster than a
    /// project grep finishes, and out-of-order replies would otherwise win.
    private var searchGeneration: UInt64 = 0
    /// Shared UI chrome toggles (menus + shell).
    @Published var uiNavVisible: Bool = true
    @Published var uiDebugVisible: Bool = false
    @Published var uiInspectorVisible: Bool = true

    // ── Shadow WAL crash recovery (D0) ──────────────────────────────────
    /// Pending recovery entries found on startup. Polled once after engine
    /// creation; the face presents a recovery sheet when non-empty.
    @Published private(set) var recoveryEntries: [RecoveryItem] = []
    @Published var recoverySheetShown: Bool = false

    struct RecoveryItem: Identifiable, Equatable {
        var id: Int { index }
        var index: Int
        var path: String
        var name: String { (path as NSString).lastPathComponent }
    }
    /// Show/hide the Debug Area, spawning a session when opening into nothing.
    /// Lives here rather than in the view so the navigator strip's detached
    /// toggle and the View menu (⌘⇧Y) cannot drift apart — ⌘⇧Y is the only way
    /// in while the navigator is hidden, so it has to do the full job.
    func setDebugArea(_ on: Bool) {
        uiDebugVisible = on
        guard on else { return }
        if !chrome.terminal.open {
            dispatch(code: .char_, ch: UInt32(UnicodeScalar("t").value), mods: .control)
        }
        // Opening the panel means the shell gets the keyboard — that is what
        // opening a terminal is FOR. Core-side focus first…
        focusTerminal(true)
        // …then strip AppKit's field focus. The panel's layout pass hands
        // first-responder to the first text field it finds (the navigator's
        // Filter — the caret visibly landed there), and a focused field makes
        // the key monitor pass keys through to it instead of the shell.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            if let w = NSApp.keyWindow,
               w.firstResponder is NSText || w.firstResponder is NSTextField {
                w.makeFirstResponder(nil)
            }
        }
    }

    /// Last editor stage size — re-used after Cmd+/- so Core row count tracks line height.
    private var lastEditorSize: CGSize = .zero

    private var engine: OpaquePointer?
    private var keyMonitor: Any?
    /// Debounced catch-up refresh scheduled by the typing fast path.
    private var chromeSettleWork: DispatchWorkItem?
    /// Background indexer, so interaction can tell it to stand down.
    weak var projectIndex: ProjectIndex?
    /// Caret rect in SwiftUI window space (top-left origin), published by the
    /// canvas as it draws. Overlays that must sit AT the caret — completions,
    /// signature help — anchor to this instead of floating in a corner. Plain
    /// storage on purpose: it is read during a render the completion publish
    /// already triggered, so making it @Published would only add churn.
    var caretFrameInWindow: CGRect = .zero
    private var scrollMonitor: Any?
    private var installed = false
    private var pointerSession = false
    private var tickTimer: Timer?
    /// Last published frame_gen — skip full chrome rebuild when tick is a no-op.
    private var lastFrameGen: UInt64 = 0
    /// Throttle SwiftUI chrome publishes during mouse drag (engine still updates).
    private var lastDragPublish: CFTimeInterval = 0
    private var dragDirty = false
    /// Coalesce wheel deltas into one Core scroll + one line pull per runloop turn.
    private var scrollFlushScheduled = false
    private var pendingScrollFrac: Float = 0
    /// Debounce full chrome after rapid resize (GeometryReader thrash).
    private var resizeDebounceWork: DispatchWorkItem?
    private var resizePendingFull = false
    private var lastResizePush: CFTimeInterval = 0
    /// True during native window live-resize — the resize HUD blurs the content,
    /// so engine pushes are pointless until the gesture ends.
    var windowLiveResizing = false

    /// Apply the final editor size after a live window resize.
    func settleEditorResize() {
        resizePendingFull = false
        resizeDebounceWork?.cancel()
        lastResizePush = CACurrentMediaTime()
        pushEditorSize(dpr: 2)
    }
    /// Editor stage frame in **window** coordinates (for scroll routing).
    var editorWindowFrame: CGRect = .null
    /// Floating panel frames in window coords — scroll/hit pass-through.
    var explorerWindowFrame: CGRect = .null
    var terminalWindowFrame: CGRect = .null

    /// True while a modal/float owns chrome — editor must not place the caret.
    /// Settings is a separate window (does not block editor pointer).
    var floatingChromeBlocksEditor: Bool {
        chrome.palette.open
    }

    func bumpFontGeneration() {
        fontGeneration &+= 1
    }

    func closeSettings() {
        guard chrome.settings.open else { return }
        cancelPointerSession()
        // Persist draft before Esc dismiss (GUI has no "s to save" muscle memory).
        saveSettings()
        dispatch(code: .esc)
    }

    /// Write dirty draft to `~/.xei.toml` (no-op if clean / closed).
    func saveSettings() {
        guard let engine else { return }
        suisei_engine_settings_save(engine)
        refreshChrome()
    }

    /// Drop an in-flight drag without applying a phantom click (cursor stay put).
    func cancelPointerSession() {
        guard let engine else {
            pointerSession = false
            return
        }
        if pointerSession {
            suisei_engine_mouse_up(engine)
        }
        pointerSession = false
    }

    private var appearanceObserver: NSKeyValueObservation?

    init() {
        engine = suisei_engine_new()
        pushSystemAppearance()
        observeSystemAppearance()
        refreshChrome()
        checkRecovery()
    }

    deinit {
        tickTimer?.invalidate()
        appearanceObserver?.invalidate()
        if let keyMonitor { NSEvent.removeMonitor(keyMonitor) }
        if let scrollMonitor { NSEvent.removeMonitor(scrollMonitor) }
        if let engine { suisei_engine_free(engine) }
    }

    /// Follow macOS light/dark. Unless the user has pinned a theme, the engine
    /// picks the palette from this — the app used to read a fixed theme name
    /// out of a config file and stay light on a dark desktop.
    private func pushSystemAppearance() {
        guard let engine else { return }
        let match = NSApp.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua])
        suisei_engine_set_system_appearance(engine, match == .darkAqua ? 1 : 0)
        refreshChrome()
    }

    private func observeSystemAppearance() {
        appearanceObserver = NSApp.observe(\.effectiveAppearance) { [weak self] _, _ in
            DispatchQueue.main.async { self?.pushSystemAppearance() }
        }
    }

    func activateInput() {
        if !installed {
            installed = true
            installKeyMonitor()
            installScrollMonitor()
            startTick()
        }
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        refreshChrome()
        // Never force Insert on the welcome sheet — that can muddy cold-start state.
        if !chrome.welcome {
            ensureEditorFocus()
        }
    }

    // MARK: - Shadow WAL recovery

    /// Query the engine for pending crash-recovery entries on startup.
    /// If any exist, the Welcome view presents a recovery sheet.
    private func checkRecovery() {
        guard let engine else { return }
        let count = suisei_engine_recovery_count(engine)
        guard count > 0 else { return }
        var entries: [RecoveryItem] = []
        for i in 0..<Int(count) {
            var buf = [CChar](repeating: 0, count: 512)
            let ok = suisei_engine_recovery_path(engine, UInt32(i), &buf, UInt32(buf.count))
            if ok != 0 {
                let path = String(cString: buf)
                entries.append(RecoveryItem(index: i, path: path))
            }
        }
        recoveryEntries = entries
        recoverySheetShown = !entries.isEmpty
    }

    /// Accept recovery entry: open file, restore unsaved buffer, cursor, scroll.
    func acceptRecovery(_ item: RecoveryItem) {
        guard let engine else { return }
        let ok = suisei_engine_recovery_accept(engine, UInt32(item.index))
        guard ok != 0 else { return }
        recoveryEntries.removeAll { $0.index == item.index }
        // Re-index remaining entries (indices shift after accept/removal).
        reindexRecoveryEntries()
        recoverySheetShown = !recoveryEntries.isEmpty
        refreshChrome()
    }

    /// Discard recovery entry: delete the WAL file, user chose not to recover.
    func discardRecovery(_ item: RecoveryItem) {
        guard let engine else { return }
        suisei_engine_recovery_discard(engine, UInt32(item.index))
        recoveryEntries.removeAll { $0.index == item.index }
        reindexRecoveryEntries()
        recoverySheetShown = !recoveryEntries.isEmpty
    }

    /// Discard all remaining recovery entries.
    func discardAllRecovery() {
        guard let engine else { return }
        while !recoveryEntries.isEmpty {
            // Always discard index 0 — after each discard the list shifts down.
            suisei_engine_recovery_discard(engine, 0)
            recoveryEntries.removeFirst()
        }
        recoverySheetShown = false
    }

    /// After accepting/discarding, the journal's internal list shrinks.
    /// Remaining entries keep their original indices in the journal, but our
    /// Swift array may have gaps — rebuild from the engine.
    private func reindexRecoveryEntries() {
        guard let engine else { return }
        let count = suisei_engine_recovery_count(engine)
        var fresh: [RecoveryItem] = []
        for i in 0..<Int(count) {
            var buf = [CChar](repeating: 0, count: 512)
            let ok = suisei_engine_recovery_path(engine, UInt32(i), &buf, UInt32(buf.count))
            if ok != 0 {
                let path = String(cString: buf)
                fresh.append(RecoveryItem(index: i, path: path))
            }
        }
        recoveryEntries = fresh
    }

    private func startTick() {
        tickTimer?.invalidate()
        // 50ms is enough for PTY drain; face only paints when frame_gen advances.
        tickTimer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            guard let self, let engine = self.engine else { return }
            let gen = suisei_engine_tick(engine, 50)
            // Pick up an async references reply (one publish, then it stops).
            self.pollReferencesIfNeeded()
            // Never publish SwiftUI editor updates mid-gesture — the canvas
            // already merges paint windows itself; publishing re-enters
            // updateNSView while AppKit scrolls and shows as jitter.
            if self.isLiveScrolling { return }
            if gen != self.lastFrameGen {
                // Terminal / LSP noise: prefer light paint; full shell every ~0.5s max via gen.
                if self.chrome.terminal.open || self.chrome.gitWb.open {
                    self.refreshChrome()
                } else {
                    self.refreshEditorPaintOnly()
                }
            }
        }
        if let tickTimer {
            RunLoop.main.add(tickTimer, forMode: .common)
        }
    }

    /// Terminal owns the keyboard while Core routes keys to the PTY:
    /// Mode::Terminal (side/full focus), or the full-panel terminal bound to the
    /// focused split pane (core `terminal_window_focused` routes those too).
    var terminalOwnsKeys: Bool {
        let t = chrome.terminal
        guard t.open else { return false }
        if focus == .terminal { return true }
        if t.fullPanel {
            if let bound = t.paneBound {
                return !editorSplit.isSplit || editorSplit.focus == bound
            }
            return true
        }
        return false
    }

    func dispatch(code: SuiseiKey, ch: UInt32 = 0, fNum: UInt8 = 0, mods: SuiseiMod = []) {
        // Debug-strip terminal promises "Esc to leave" — release focus instead
        // of feeding Esc to the shell. (Pane terminals keep Esc for TUIs.)
        if code == .esc, terminalOwnsKeys, !chrome.terminal.fullPanel {
            focusTerminal(false)
            // Engine mode is now Normal (focus release) — enter Insert.
            if let engine { suisei_engine_gui_ensure_insert(engine) }
            refreshChrome()
            return
        }
        // GUI semantic Esc: collapse overlays/selection, land in Insert.
        // Panels (find bar, palette…) still get raw Esc to close themselves.
        if code == .esc, !terminalOwnsKeys, !panelOwnsTyping {
            if let engine {
                suisei_engine_gui_escape(engine)
                refreshEditorPaintOnly()
            }
            return
        }
        // GUI contract: printable keys always type. Skip when a terminal panel
        // owns input (PTY gets raw keys via Core) — panels own their own keys.
        if !terminalOwnsKeys {
            if prepareGuiDispatch(code: code, ch: ch, mods: mods) {
                return // consumed (semantic type / delete)
            }
        }
        dispatchRaw(code: code, ch: ch, fNum: fNum, mods: mods)
    }

    /// Core key without Face policy (used by ensureInsert / Esc chains).
    /// Typing fast path. Returns false when this key needs the full path.
    ///
    /// Measured: the whole Rust side (dispatch + recompose + compose) costs
    /// 0.001ms per key on an empty buffer, yet typing felt slow even there —
    /// because every keystroke pulled the chrome snapshot, decoded all editor
    /// lines into Swift structs, ran six more FFI probes and then assigned
    /// `@Published` state, re-evaluating the entire ContentView tree. None of
    /// that is needed to put a character on screen: the canvas pulls its own
    /// rows straight from the engine. So typing dispatches and repaints, and
    /// the rest of the UI (tab dirty dot, Ln/Col, outline) settles behind.
    func typeFast(ch: UInt32) -> Bool {
        guard let engine, !terminalOwnsKeys, !panelOwnsTyping else { return false }
        // Fast path whenever the editor owns the keys. Editing is modeless now
        // and the underlying insert REPLACES any selection, so a live drag
        // selection no longer needs the slow path — it is overwritten correctly.
        guard suisei_engine_editor_accepts_text(engine) != 0 else { return false }
        _ = suisei_engine_dispatch_key(engine, SuiseiKey.char_.rawValue, ch, 0, 0)
        // Autocomplete has to keep up WITH typing, not 120ms after it stops.
        // Probe cheaply and pull the popup only while there is one — this is
        // the single piece of chrome the fast path may not defer.
        if suisei_engine_completions_open(engine) != 0 || chrome.completions.open {
            let c = loadCompletions(engine)
            chrome.completions = c.open ? c : .empty
        }
        scheduleChromeSettle()
        return true
    }

    /// Coalesced catch-up for everything typing deliberately skipped.
    private func scheduleChromeSettle() {
        chromeSettleWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            self?.refreshChrome()
        }
        chromeSettleWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: work)
    }

    func dispatchRaw(code: SuiseiKey, ch: UInt32 = 0, fNum: UInt8 = 0, mods: SuiseiMod = []) {
        guard let engine else { return }
        // Chrome-owned modes keep shell surfaces (SCM list, git workbench, explorer
        // selection) live per keystroke — the light path never pulls those FFIs.
        let wasChromeMode = focus.wantsFullChrome
        _ = suisei_engine_dispatch_key(engine, code.rawValue, ch, fNum, mods.rawValue)
        // Terminal output is polled on tick — always refresh chrome while open.
        if chrome.terminal.open || wasChromeMode
            || Self.keyNeedsFullChrome(code: code, ch: ch, mods: mods)
        {
            refreshChrome()
        } else {
            refreshEditorPaintOnly()
        }
        if !wasChromeMode, focus.wantsFullChrome {
            // Key just entered a chrome mode the light path can't service.
            refreshChrome()
        }
    }

    /// Which surface owns the keyboard right now.
    var focus: Focus { Focus(label: chrome.modeLabel) }

    func insertChar(_ c: Character) {
        guard let scalar = c.unicodeScalars.first else { return }
        if !terminalOwnsKeys, !panelOwnsTyping, let engine {
            let t0 = DispatchTime.now().uptimeNanoseconds
            defer {
                PerfProbe.record(
                    "insertChar (whole keystroke)",
                    Double(DispatchTime.now().uptimeNanoseconds - t0) / 1_000_000
                )
            }
            // Semantic: mode transition + selection replace handled by engine.
            PerfProbe.measure("  engine_gui_type_char") {
                suisei_engine_gui_type_char(engine, scalar.value)
            }
            refreshEditorPaintOnly()
            scheduleChromeSettle()
        } else {
            dispatchRaw(code: .char_, ch: scalar.value)
        }
    }

    /// True while an overlay / panel surface owns typed characters (find bar,
    /// palette filter, git panes, settings, read-only preview…) — never force
    /// Insert then.
    private var panelOwnsTyping: Bool { focus.ownsTyping }

    /// Give the editor the keyboard back.
    ///
    /// There is no Insert mode to land in any more — the editor is one state
    /// and typing always types. All this does is release whichever panel or
    /// terminal currently owns keys. Terminal focus is released via
    /// `focus_terminal(false)` rather than Esc, because core's terminal Esc
    /// handler SHUTS the terminal down (TUI contract); we only want to unfocus.
    func ensureEditorFocus(replacingSelection: Bool = false) {
        guard let engine else { return }
        if focus == .editor { return }
        if focus == .terminal {
            focusTerminal(false)
        }
        suisei_engine_gui_ensure_insert(engine)
        refreshEditorPaintOnly()
    }

    /// GUI key prep. Returns `true` if the key was fully consumed (do not dispatch).
    ///
    /// Semantic commands (`gui_type_char`, `gui_delete_*`) handle mode transitions
    /// internally — no synthetic vim keystrokes cross the FFI boundary.
    @discardableResult
    private func prepareGuiDispatch(code: SuiseiKey, ch: UInt32, mods: SuiseiMod) -> Bool {
        // Esc: handled in dispatch() before this is called.
        if code == .esc { return false }
        // Panels (find bar, palette, git, settings…) own their keys — route raw.
        if panelOwnsTyping { return false }

        let chord = mods.contains(.control) || mods.contains(.superKey) || mods.contains(.alt)
        if chord {
            // Bare Ctrl+V: ensure Insert so Core reads it as clipboard paste,
            // never Visual-Block.
            if code == .char_,
               mods.contains(.control), !mods.contains(.superKey),
               ch == UInt32(UnicodeScalar("v").value) || ch == UInt32(UnicodeScalar("V").value)
            {
                ensureEditorFocus(replacingSelection: true)
            }
            return false
        }

        // Mac contract: Backspace / Delete removes the active selection.
        // Semantic command handles Visual → delete → Insert internally.
        if code == .backspace {
            if let engine {
                suisei_engine_gui_delete_backward(engine)
                refreshEditorPaintOnly()
                scheduleChromeSettle()
            }
            return true
        }
        if code == .delete {
            if let engine {
                suisei_engine_gui_delete_forward(engine)
                refreshEditorPaintOnly()
                scheduleChromeSettle()
            }
            return true
        }

        // Printable → semantic type (mode transition + selection replace internal).
        if code == .char_, ch >= 32, ch != 127 {
            if let engine {
                suisei_engine_gui_type_char(engine, ch)
                // Completions must keep up with typing.
                if suisei_engine_completions_open(engine) != 0 || chrome.completions.open {
                    let c = loadCompletions(engine)
                    chrome.completions = c.open ? c : .empty
                }
                refreshEditorPaintOnly()
                scheduleChromeSettle()
            }
            return true
        }
        return false
    }

    /// Keys that open/close panels or change shell-owned surfaces → full chrome.
    private static func keyNeedsFullChrome(code: SuiseiKey, ch: UInt32, mods: SuiseiMod) -> Bool {
        if code == .esc { return true }
        if code == .f { return true } // function keys often mode switches
        let hasChord = mods.contains(.control) || mods.contains(.superKey) || mods.contains(.alt)
        if hasChord {
            // Ctrl/Cmd chords usually open panels (f/g/p/t/,) or save — refresh shell.
            return true
        }
        return false
    }

    /// Resize from the **editor stage** GeometryReader (not the whole window).
    /// Panel drags and sidebar/inspector animations fire this per frame —
    /// throttle the engine push to ~30Hz with a trailing settle pass so the
    /// animation stays fluid (a 240-line recompose per frame reads as hitching).
    func resizeEditor(width: CGFloat, height: CGFloat, dpr: CGFloat = 2) {
        guard engine != nil, width > 80, height > 80 else { return }
        let w = width.rounded()
        let h = height.rounded()
        // 2pt threshold + quantize: kill sub-pixel GeometryReader thrash while dragging.
        if abs(w - lastEditorSize.width) < 2, abs(h - lastEditorSize.height) < 2 {
            return
        }
        lastEditorSize = CGSize(width: w, height: h)

        // Native window live-resize: the HUD blurs content — record only,
        // one push happens on gesture end (settleEditorResize).
        if windowLiveResizing { return }

        let now = CACurrentMediaTime()
        let due = now - lastResizePush >= (1.0 / 30.0)
        if due {
            lastResizePush = now
            pushEditorSize(dpr: dpr)
        }
        // Trailing settle: always apply the final size after the gesture/animation.
        resizePendingFull = true
        resizeDebounceWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.resizePendingFull else { return }
            self.resizePendingFull = false
            self.lastResizePush = CACurrentMediaTime()
            self.pushEditorSize(dpr: dpr)
        }
        resizeDebounceWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.14, execute: work)
    }

    private func pushEditorSize(dpr: CGFloat) {
        guard let engine else { return }
        suisei_engine_resize(
            engine,
            Float(lastEditorSize.width),
            Float(lastEditorSize.height),
            Float(EditorMetrics.lineHeight),
            Float(EditorMetrics.cellWidth),
            Float(dpr)
        )
        // Editor lines only — shell panels keep their last snapshot until drag settles.
        refreshEditorPaintOnly()
    }

    /// Cmd+ / Cmd- / Cmd+0 — zoom **editor canvas only** (persisted). Shell chrome is fixed.
    func zoomFont(delta: CGFloat) {
        EditorMetrics.adjustFont(delta: delta)
        fontGeneration &+= 1
        reapplyEditorMetrics()
    }

    func resetFontZoom() {
        EditorMetrics.resetFont()
        fontGeneration &+= 1
        reapplyEditorMetrics()
    }

    /// After font zoom, force Core viewport recompute even if pixel size is unchanged.
    private func reapplyEditorMetrics() {
        guard let engine else { return }
        let w = lastEditorSize.width
        let h = lastEditorSize.height
        guard w > 80, h > 80 else {
            refreshEditorPaintOnly()
            return
        }
        suisei_engine_resize(
            engine,
            Float(w),
            Float(h),
            Float(EditorMetrics.lineHeight),
            Float(EditorMetrics.cellWidth),
            2
        )
        refreshEditorPaintOnly()
    }

    @available(*, deprecated, message: "Use resizeEditor")
    func resize(width: CGFloat, height: CGFloat, cellPx: CGFloat = 15, dpr: CGFloat = 2) {
        resizeEditor(width: width, height: max(40, height - 80), dpr: dpr)
    }

    /// Integer scroll (PageUp / keys). Single source: Core. Clears residual.
    func scrollBy(_ delta: Int32) {
        guard let engine, delta != 0 else { return }
        suisei_engine_scroll(engine, delta)
        refreshChromeScrollOnly()
    }

    /// Snapshot for editor canvas hot path (scroll / paint without waiting on SwiftUI).
    struct EditorPaintSnap: Equatable {
        var lines: [EditorLine]
        var split: SplitSnap
        var scrollFrac: CGFloat
        var hscroll: UInt32
        var wrapLines: Bool
        var scroll: UInt32
        var lineCount: UInt32
        var frameGen: UInt64
    }

    /// Fractional scroll — Core owns residual; face only paints `snap.scroll_frac`.
    /// Positive = reveal content below. Always refresh lines+frac in the same turn.
    func scrollByFrac(_ delta: Float) {
        _ = scrollByFracPullingPaint(delta)
    }

    /// Absolute scroll for native NSScrollView (first visible line + hscroll columns).
    /// - publish: `false` during live scroll (canvas applies snap directly — no SwiftUI thrash);
    ///   `true` when settled (status line / pane chrome).
    @discardableResult
    func scrollToPullingPaint(line: UInt32, hscroll: UInt32, publish: Bool = false) -> EditorPaintSnap? {
        guard let engine else { return nil }
        suisei_engine_scroll_to(engine, line, hscroll)
        return pullEditorPaintSnap(publish: publish)
    }

    /// Horizontal pan (columns). No-op when soft-wrap is on.
    func scrollH(_ deltaCols: Int32) {
        guard let engine, deltaCols != 0 else { return }
        suisei_engine_scroll_h(engine, deltaCols)
        _ = pullEditorPaintSnap(publish: true)
        if focus == .preview || preview.open {
            refreshPreview()
        }
    }

    func refreshPreview() {
        guard let engine else {
            preview = .empty
            return
        }
        preview = loadPreview(engine)
        lastPreviewKey = previewKey(bufferVersion: chrome.bufferVersion)
    }

    /// (open, buffer version, hscroll) — preview pull cache key.
    private var lastPreviewKey: String = ""

    private func previewKey(bufferVersion: UInt64) -> String {
        "\(preview.open)|\(bufferVersion)|\(preview.hscroll)"
    }

    private func refreshPreviewIfNeeded(_ engine: OpaquePointer, bufferVersion: UInt64) {
        // Cheap single-chunk probe for open flag + pan state.
        var probe = SuiseiPreviewSnapshot()
        let ok = suisei_engine_preview(engine, 0, &probe)
        guard ok != 0, probe.open != 0 else {
            if preview != .empty { preview = .empty }
            lastPreviewKey = ""
            return
        }
        let key = "open|\(bufferVersion)|\(probe.hscroll)|\(probe.total)"
        if key == lastPreviewKey { return }
        let pv = loadPreview(engine)
        if pv != preview { preview = pv }
        lastPreviewKey = key
    }

    func togglePreview() {
        var m = SuiseiMod.superKey
        m.insert(.shift)
        dispatchRaw(code: .char_, ch: UInt32(UnicodeScalar("v").value), mods: m)
        refreshChrome()
        refreshPreview()
    }

    func toggleTerminalFull() {
        // Cmd/Ctrl+Shift+T — Core treats Super or Ctrl+Shift as cmd_like.
        // Use dispatchRaw so Hybrid never injects Insert around the chord.
        var m = SuiseiMod.superKey
        m.insert(.shift)
        dispatchRaw(code: .char_, ch: UInt32(UnicodeScalar("t").value), mods: m)
        refreshChrome()
        // Keep Core split focus on the terminal pane so keys hit the PTY.
        if chrome.terminal.open, chrome.terminal.fullPanel,
           let bound = chrome.terminal.paneBound, let engine
        {
            suisei_engine_focus_pane(engine, UInt32(bound))
            refreshChrome()
        }
        // Never leave the bottom Debug side-terminal open alongside full panel.
        if chrome.terminal.fullPanel {
            uiDebugVisible = false
        }
    }

    /// Core scroll + immediate paint snap (same call stack — no dual clock).
    /// EditorCanvasView applies the snap directly; SwiftUI is updated lightly for status/scrollbar.
    @discardableResult
    func scrollByFracPullingPaint(_ delta: Float) -> EditorPaintSnap? {
        guard let engine, delta != 0, delta.isFinite else { return nil }
        suisei_engine_scroll_frac(engine, delta)
        return pullEditorPaintSnap(publish: true)
    }

    /// Pull current editor lines + scroll residual without full shell FFI.
    @discardableResult
    func pullEditorPaintSnap(publish: Bool) -> EditorPaintSnap? {
        guard let engine else { return nil }
        var snap = SuiseiChromeSnapshot()
        guard suisei_engine_chrome(engine, &snap) != 0 else { return nil }
        let (lines, split) = decodeEditorLinesAndSplit(from: snap)
        let paint = EditorPaintSnap(
            lines: lines,
            split: split,
            scrollFrac: CGFloat(snap.scroll_frac),
            hscroll: snap.hscroll,
            wrapLines: snap.wrap_lines != 0,
            scroll: snap.scroll,
            lineCount: snap.line_count,
            frameGen: snap.frame_gen
        )
        if publish {
            applyEditorPaintSnap(paint, tabsFrom: snap)
        }
        return paint
    }

    private func applyEditorPaintSnap(_ paint: EditorPaintSnap, tabsFrom snap: SuiseiChromeSnapshot) {
        lastFrameGen = paint.frameGen
        var tabs: [TabItem] = []
        let tabCount = Int(snap.tab_count)
        withUnsafeBytes(of: snap.tab_titles) { titlesRaw in
            withUnsafeBytes(of: snap.tab_dirty) { dirtyRaw in
                let titleCap = Int(SUISEI_TITLE_CAP)
                for i in 0..<min(tabCount, Int(SUISEI_MAX_TABS)) {
                    let base = titlesRaw.baseAddress!.advanced(by: i * titleCap)
                    let title = String(cString: base.assumingMemoryBound(to: CChar.self))
                    tabs.append(TabItem(
                        id: i,
                        title: title.isEmpty ? "[No Name]" : title,
                        dirty: dirtyRaw[i] != 0,
                        active: i == Int(snap.tab_active)
                    ))
                }
            }
        }

        var txn = Transaction()
        txn.disablesAnimations = true
        withTransaction(txn) {
            if abs(paint.scrollFrac - editorScrollFrac) > 0.00005 {
                editorScrollFrac = paint.scrollFrac
            }
            if paint.hscroll != editorHScroll { editorHScroll = paint.hscroll }
            if paint.wrapLines != wrapLines { wrapLines = paint.wrapLines }
            if paint.lines != editorLines { editorLines = paint.lines }
            if paint.split != editorSplit { editorSplit = paint.split }
        }

        var next = chrome
        next.gen = paint.frameGen
        next.scroll = paint.scroll
        next.lineCount = paint.lineCount
        next.lines = paint.lines
        next.split = paint.split
        next.tabs = tabs
        next.modeLabel = cStringField(snap.mode_label)
        next.message = cStringField(snap.message)
        next.filename = cStringField(snap.filename)
        next.cursorRow = snap.cursor_row
        next.cursorCol = snap.cursor_col
        next.caretVCol = snap.caret_vcol
        next.scrollIntent = snap.scroll_intent
        next.pct = snap.pct
        next.bufferVersion = snap.buffer_version
        next.dirty = snap.dirty_buffer != 0
        if next != chrome { chrome = next }
    }

    /// Batch trackpad events to the next display frame (vsync when CADisplayLink available).
    func scrollByFracCoalesced(_ delta: Float) {
        guard delta != 0, delta.isFinite else { return }
        pendingScrollFrac += delta
        scheduleScrollFlush()
    }

    /// Called from EditorHost DisplayLink or fallback async.
    /// Returns paint snap when Core advanced (canvas applies immediately).
    @discardableResult
    func flushPendingScroll() -> EditorPaintSnap? {
        scrollFlushScheduled = false
        let d = pendingScrollFrac
        pendingScrollFrac = 0
        guard d != 0 else { return nil }
        return scrollByFracPullingPaint(d)
    }

    private func scheduleScrollFlush() {
        if scrollFlushScheduled { return }
        scrollFlushScheduled = true
        // Prefer AppKit view DisplayLink (set by EditorCanvasView); else next main turn.
        if scrollDisplayLinkActive {
            return
        }
        DispatchQueue.main.async { [weak self] in
            _ = self?.flushPendingScroll()
        }
    }

    /// Editor canvas owns a CADisplayLink; when active, scroll flushes on vsync.
    private(set) var scrollDisplayLinkActive = false

    func setScrollDisplayLinkActive(_ active: Bool) {
        scrollDisplayLinkActive = active
        if !active, scrollFlushScheduled, pendingScrollFrac != 0 {
            // Link stopped mid-gesture — flush remaining immediately.
            _ = flushPendingScroll()
        }
    }

    /// Live-scroll refcount from editor scroll views — while > 0, the tick loop
    /// must not publish SwiftUI editor updates (AppKit owns the pixels; a
    /// mid-gesture publish re-runs updateNSView and reads as jitter).
    private var liveScrollCount = 0

    func beginLiveScroll() {
        liveScrollCount += 1
    }

    func endLiveScroll() {
        liveScrollCount = max(0, liveScrollCount - 1)
    }

    var isLiveScrolling: Bool { liveScrollCount > 0 }

    /// Position-only Core sync while the clip scrolls (covered band, no pull).
    func scrollSync(line: UInt32, hscroll: UInt32) {
        guard let engine else { return }
        suisei_engine_scroll_sync(engine, line, hscroll)
    }

    func nextTab() {
        let tabs = chrome.tabs
        guard !tabs.isEmpty else { return }
        let cur = tabs.firstIndex(where: \.active) ?? 0
        gotoTab((cur + 1) % tabs.count)
    }

    func prevTab() {
        let tabs = chrome.tabs
        guard !tabs.isEmpty else { return }
        let cur = tabs.firstIndex(where: \.active) ?? 0
        gotoTab((cur - 1 + tabs.count) % tabs.count)
    }

    func focusNextPane() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_focus_next_pane(engine)
        refreshChrome()
    }

    func focusPane(_ index: Int) {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_focus_pane(engine, UInt32(index))
        refreshChrome()
    }

    func closeFocusedPane() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_close_focused_pane(engine)
        refreshChrome()
    }

    func openPath(_ path: String) {
        guard let engine else { return }
        var isDir: ObjCBool = false
        FileManager.default.fileExists(atPath: path, isDirectory: &isDir)
        if isDir.boolValue {
            setProjectRoot(path)
        } else if projectRoot.isEmpty {
            setProjectRoot((path as NSString).deletingLastPathComponent)
        }
        path.withCString { _ = suisei_engine_open_path(engine, $0) }
        // Seed docked Project tree (does not steal Mode::Explorer).
        suisei_engine_ensure_project_tree(engine)
        RecentStore.push(path: path)
        refreshChrome()
        resolveProjectRoot()
        // Hybrid: open file → ready to type (Mac contract).
        ensureEditorFocus()
    }

    /// Workspace root for hierarchical Project navigator (folder open / git root).
    @Published var projectRoot: String = ""

    /// Ensure file tree has entries for docked navigator without Mode::Explorer.
    func ensureProjectTree() {
        guard let engine else { return }
        suisei_engine_ensure_project_tree(engine)
        refreshChrome()
        resolveProjectRoot()
    }

    func ensureScm() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_ensure_scm(engine)
        refreshChrome()
    }

    func closeScm() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_close_scm(engine)
        refreshChrome()
    }

    /// Prefer explicit folder; else parent of current file; else seeded explorer.cwd.
    /// Never invent a root from bare cwd while still on Welcome (`open` often has cwd `/`).
    func resolveProjectRoot() {
        if chrome.welcome { return }
        if !projectRoot.isEmpty,
           FileManager.default.fileExists(atPath: projectRoot) {
            return
        }
        let file = chrome.filename
        if !file.isEmpty, file != "[No Name]", file != "Untitled" {
            var isDir: ObjCBool = false
            if FileManager.default.fileExists(atPath: file, isDirectory: &isDir), isDir.boolValue {
                projectRoot = file
            } else {
                projectRoot = (file as NSString).deletingLastPathComponent
            }
            return
        }
        let cwd = chrome.explorer.cwd
        // Only adopt explorer cwd when the tree was actually seeded — never bare `/`.
        if !cwd.isEmpty, !chrome.explorer.entries.isEmpty, cwd != "/" {
            var isDir: ObjCBool = false
            if FileManager.default.fileExists(atPath: cwd, isDirectory: &isDir), isDir.boolValue {
                projectRoot = cwd
            }
        }
    }

    func setProjectRoot(_ path: String) {
        var isDir: ObjCBool = false
        if FileManager.default.fileExists(atPath: path, isDirectory: &isDir), isDir.boolValue {
            projectRoot = path
        } else {
            projectRoot = (path as NSString).deletingLastPathComponent
        }
        ProjectTreeView.invalidateCache()
    }

    /// Leave welcome with a fresh blank buffer (still untitled).
    func createNewProject() {
        cancelPointerSession()
        openBlankTab()
        // With no project open there is nowhere sensible to put it, so ask.
        // This used to silently create `$TMPDIR/suisei-new/Untitled.txt` — a
        // file the user could not find and the system would eventually delete.
        if chrome.welcome {
            let panel = NSSavePanel()
            panel.nameFieldStringValue = "Untitled.txt"
            panel.canCreateDirectories = true
            panel.prompt = "Create"
            panel.message = "Where should the new file go?"
            guard panel.runModal() == .OK, let url = panel.url else { return }
            FileManager.default.createFile(atPath: url.path, contents: Data(), attributes: nil)
            openPath(url.path)
        }
    }

    // MARK: - File operations
    //
    // The filesystem call lives here, not in the core: Trash is an AppKit
    // service and drag payloads arrive as `NSItemProvider`. The core is then
    // TOLD, so open tabs, the active file and the language server follow the
    // path instead of pointing at something that no longer exists.

    /// Create an empty file, returning its path. Numbers the name on collision
    /// rather than overwriting.
    @discardableResult
    func createFile(in directory: String, named base: String = "Untitled.txt") -> String? {
        let url = Self.unusedURL(in: directory, base: base)
        guard FileManager.default.createFile(atPath: url.path, contents: Data()) else {
            presentFileError("Could not create \(url.lastPathComponent)")
            return nil
        }
        return url.path
    }

    @discardableResult
    func createFolder(in directory: String, named base: String = "untitled folder") -> String? {
        let url = Self.unusedURL(in: directory, base: base)
        do {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false)
            return url.path
        } catch {
            presentFileError("Could not create \(url.lastPathComponent): \(error.localizedDescription)")
            return nil
        }
    }

    /// Rename in place. Refuses to clobber an existing entry.
    @discardableResult
    func renamePath(_ path: String, to newName: String) -> String? {
        let src = URL(fileURLWithPath: path)
        let name = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, !name.contains("/") else { return nil }
        let dst = src.deletingLastPathComponent().appendingPathComponent(name)
        if dst.path == src.path { return path }
        guard !FileManager.default.fileExists(atPath: dst.path) else {
            presentFileError("“\(name)” already exists")
            return nil
        }
        do {
            try FileManager.default.moveItem(at: src, to: dst)
            notePathMoved(from: src.path, to: dst.path)
            return dst.path
        } catch {
            presentFileError("Could not rename: \(error.localizedDescription)")
            return nil
        }
    }

    /// Move `path` into `directory`. Returns the new path.
    @discardableResult
    func movePath(_ path: String, into directory: String, copy: Bool = false) -> String? {
        let src = URL(fileURLWithPath: path)
        let dst = URL(fileURLWithPath: directory).appendingPathComponent(src.lastPathComponent)
        guard Self.moveIsSane(from: src.path, to: dst.path) else { return nil }
        guard !FileManager.default.fileExists(atPath: dst.path) else {
            // Refuse rather than auto-rename: in an editor, silently making
            // "file 2.rs" is a worse surprise than being told no.
            presentFileError("“\(src.lastPathComponent)” already exists there")
            return nil
        }
        do {
            if copy {
                try FileManager.default.copyItem(at: src, to: dst)
            } else {
                try FileManager.default.moveItem(at: src, to: dst)
                notePathMoved(from: src.path, to: dst.path)
            }
            return dst.path
        } catch {
            presentFileError("Could not move: \(error.localizedDescription)")
            return nil
        }
    }

    /// Delete to the Trash — never `removeItem`. A tree row is one click away
    /// from a directory, and an editor has no business making that unrecoverable.
    func trashPath(_ path: String) {
        var resulting: NSURL?
        do {
            try FileManager.default.trashItem(
                at: URL(fileURLWithPath: path), resultingItemURL: &resulting
            )
            notePathMoved(from: path, to: resulting?.path ?? path)
        } catch {
            presentFileError("Could not move to Trash: \(error.localizedDescription)")
        }
    }

    /// A move is sane when it is not onto itself and not into its own subtree —
    /// dropping a folder inside itself detaches it from the tree entirely.
    static func moveIsSane(from src: String, to dst: String) -> Bool {
        if src == dst { return false }
        let srcDir = (src as NSString).standardizingPath
        let dstDir = ((dst as NSString).deletingLastPathComponent as NSString).standardizingPath
        if srcDir == dstDir { return false }            // already there
        return !(dstDir + "/").hasPrefix(srcDir + "/")  // into its own descendant
    }

    private static func unusedURL(in directory: String, base: String) -> URL {
        let dir = URL(fileURLWithPath: directory, isDirectory: true)
        let ext = (base as NSString).pathExtension
        let stem = (base as NSString).deletingPathExtension
        var candidate = dir.appendingPathComponent(base)
        var n = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            let name = ext.isEmpty ? "\(stem) \(n)" : "\(stem) \(n).\(ext)"
            candidate = dir.appendingPathComponent(name)
            n += 1
        }
        return candidate
    }

    private func notePathMoved(from old: String, to new: String) {
        guard let engine else { return }
        old.withCString { o in
            new.withCString { n in
                _ = suisei_engine_path_moved(engine, o, n)
            }
        }
        refreshChrome()
    }

    private func presentFileError(_ message: String) {
        let alert = NSAlert()
        alert.messageText = message
        alert.alertStyle = .warning
        alert.runModal()
    }

    func openProjectFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Open"
        panel.begin { [weak self] result in
            guard result == .OK, let url = panel.url else { return }
            DispatchQueue.main.async {
                self?.openPath(url.path)
                // If directory, also open explorer at that cwd via Core explorer after open
                if url.hasDirectoryPath {
                    self?.cancelPointerSession()
                    // Folder open already seeds Project tree via open_path; keep Normal focus.
                    self?.ensureProjectTree()
                }
            }
        }
    }

    func cloneGitRepository() {
        let alert = NSAlert()
        alert.messageText = "Clone Git Repository"
        alert.informativeText = "Enter a git URL (https://… or git@…)."
        alert.alertStyle = .informational
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 320, height: 24))
        field.placeholderString = "https://github.com/org/repo.git"
        alert.accessoryView = field
        alert.addButton(withTitle: "Clone")
        alert.addButton(withTitle: "Cancel")
        let response = alert.runModal()
        guard response == .alertFirstButtonReturn else { return }
        let url = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !url.isEmpty else { return }
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.prompt = "Clone To"
        panel.begin { [weak self] result in
            guard result == .OK, let dest = panel.url else { return }
            DispatchQueue.global(qos: .userInitiated).async {
                let task = Process()
                task.executableURL = URL(fileURLWithPath: "/usr/bin/git")
                task.arguments = ["clone", url]
                task.currentDirectoryURL = dest
                let err = Pipe()
                task.standardError = err
                try? task.run()
                task.waitUntilExit()
                DispatchQueue.main.async {
                    // Open cloned folder if present
                    let name = URL(string: url)?.deletingPathExtension().lastPathComponent
                        ?? (url as NSString).lastPathComponent
                        .replacingOccurrences(of: ".git", with: "")
                    let project = dest.appendingPathComponent(name)
                    if FileManager.default.fileExists(atPath: project.path) {
                        self?.openPath(project.path)
                        self?.toggleExplorer()
                    } else {
                        self?.openPath(dest.path)
                    }
                }
            }
        }
    }

    // MARK: - Pointer

    /// Begin press at editor-local point (viewport coords — legacy / hit_test path).
    func pointerDown(at point: CGPoint) {
        guard let engine else { return }
        guard !floatingChromeBlocksEditor else { return }
        guard NSEvent.pressedMouseButtons & (1 << 0) != 0 else { return }
        exitPanelFocusForEditorClick()
        var row: UInt32 = 0
        var col: UInt32 = 0
        guard hitTest(engine, point, &row, &col) else { return }
        suisei_engine_click(engine, row, col, 0)
        pointerSession = true
        refreshChrome()
        ensureEditorFocus()
    }

    /// Absolute buffer row/col from native document coordinates (NSScrollView canvas).
    /// (Called from the canvas's own mouseDown — the button state is implicit;
    /// a pressedMouseButtons guard here rejected fast/synthetic clicks.)
    /// Pointer addressed by UTF-16 offset — the face measures with CoreText,
    /// the core converts to its own cell columns (one width rule, one place).
    /// Pre-parse a file into the engine's syntax cache (project indexing).
    @discardableResult
    func prewarmFile(_ path: String) -> Bool {
        guard let engine else { return false }
        return path.withCString { suisei_engine_prewarm_file(engine, $0) != 0 }
    }

    func pointerDownUTF16(row: UInt32, utf16: UInt32) {
        guard let engine else { return }
        guard !floatingChromeBlocksEditor else { return }
        exitPanelFocusForEditorClick()
        suisei_engine_click_utf16(engine, row, utf16, 0)
        pointerSession = true
        refreshChrome()
        ensureEditorFocus()
    }

    func pointerDragUTF16(row: UInt32, utf16: UInt32) {
        guard let engine, pointerSession else { return }
        suisei_engine_drag_utf16(engine, row, utf16)
        // NO chrome pull here. `refreshEditorPaintOnly()` decodes every editor
        // line into Swift structs and writes @Published state, re-evaluating
        // the whole view tree — at autoscroll rate that is the stutter. The
        // canvas pulls its own rows straight from the engine, so it only needs
        // its cached band dropped; the shell catches up when the drag ends.
    }

    func pointerDownAbsolute(row: UInt32, col: UInt32) {
        guard let engine else { return }
        guard !floatingChromeBlocksEditor else { return }
        exitPanelFocusForEditorClick()
        suisei_engine_click(engine, row, col, 0)
        pointerSession = true
        refreshChrome()
        ensureEditorFocus()
    }

    /// Clicking the editor takes key focus back from terminal / find / palette.
    private func exitPanelFocusForEditorClick() {
        if focus != .editor {
            ensureEditorFocus()
        }
    }

    func pointerDrag(at point: CGPoint) {
        guard let engine else { return }
        guard !floatingChromeBlocksEditor else { return }
        guard NSEvent.pressedMouseButtons & (1 << 0) != 0 else { return }
        var row: UInt32 = 0
        var col: UInt32 = 0
        guard hitTest(engine, point, &row, &col) else { return }
        if !pointerSession {
            suisei_engine_click(engine, row, col, 0)
            pointerSession = true
        } else {
            suisei_engine_drag(engine, row, col)
        }
        let now = CACurrentMediaTime()
        if now - lastDragPublish >= (1.0 / 60.0) {
            lastDragPublish = now
            dragDirty = false
            refreshEditorPaintOnly()
        } else {
            dragDirty = true
        }
    }

    func pointerDragAbsolute(row: UInt32, col: UInt32) {
        guard let engine else { return }
        guard !floatingChromeBlocksEditor else { return }
        guard NSEvent.pressedMouseButtons & (1 << 0) != 0 else { return }
        if !pointerSession {
            suisei_engine_click(engine, row, col, 0)
            pointerSession = true
        } else {
            suisei_engine_drag(engine, row, col)
        }
        let now = CACurrentMediaTime()
        if now - lastDragPublish >= (1.0 / 60.0) {
            lastDragPublish = now
            dragDirty = false
            refreshEditorPaintOnly()
        } else {
            dragDirty = true
        }
    }

    /// Unified path for DragGesture.onChanged.
    func pointerMoved(at point: CGPoint) {
        guard NSEvent.pressedMouseButtons & (1 << 0) != 0 else { return }
        if pointerSession {
            pointerDrag(at: point)
        } else {
            pointerDown(at: point)
        }
    }

    func pointerUp() {
        guard let engine else { return }
        if pointerSession {
            suisei_engine_mouse_up(engine)
        }
        pointerSession = false
        dragDirty = false
        refreshChrome()
    }

    func toggleScm() {
        cancelPointerSession()
        dispatch(code: .char_, ch: UInt32(UnicodeScalar("g").value), mods: .control)
    }

    func toggleGitWorkbench() {
        cancelPointerSession()
        var m = SuiseiMod.control
        m.insert(.shift)
        dispatch(code: .char_, ch: UInt32(UnicodeScalar("g").value), mods: m)
    }

    func gitWbSetTab(_ index: Int) {
        guard let engine else { return }
        suisei_engine_git_wb_set_tab(engine, UInt32(index))
        refreshChrome()
    }

    func pointerDouble(at point: CGPoint) {
        guard let engine else { return }
        var row: UInt32 = 0
        var col: UInt32 = 0
        guard hitTest(engine, point, &row, &col) else { return }
        suisei_engine_click(engine, row, col, 1)
        pointerSession = false
        refreshChrome()
    }

    func pointerDoubleAbsolute(row: UInt32, col: UInt32) {
        guard let engine else { return }
        suisei_engine_click(engine, row, col, 1)
        pointerSession = false
        refreshChrome()
    }

    private func hitTest(
        _ engine: OpaquePointer,
        _ point: CGPoint,
        _ row: inout UInt32,
        _ col: inout UInt32
    ) -> Bool {
        suisei_engine_hit_test(
            engine,
            Float(point.x),
            Float(point.y),
            Float(EditorMetrics.gutter),
            Float(EditorMetrics.cellWidth),
            Float(EditorMetrics.lineHeight),
            &row,
            &col
        ) != 0
    }

    func save() {
        guard let engine else { return }
        // A buffer that has never been written has no location yet, so ask.
        // (It must not be given a fabricated one: a relative name saves against
        // the process cwd, and the project root can itself be unwritable.)
        let f = chrome.filename
        if f.isEmpty || f == "[No Name]" || !f.hasPrefix("/") {
            saveAsPanel()
            return
        }
        suisei_engine_save(engine)
        refreshChrome()
    }

    /// Engine/core version (single source: workspace Cargo version).
    static var engineVersion: String {
        guard let p = suisei_engine_version() else { return "" }
        return String(cString: p)
    }

    func saveAsPanel() {
        let panel = NSSavePanel()
        panel.canCreateDirectories = true
        // Open where the user is working rather than wherever the panel was
        // last: an untitled buffer almost always belongs in the project.
        let root = projectRoot.isEmpty ? chrome.explorer.cwd : projectRoot
        if !root.isEmpty, root != "/" {
            panel.directoryURL = URL(fileURLWithPath: root, isDirectory: true)
        }
        if chrome.filename.isEmpty || !chrome.filename.hasPrefix("/") {
            panel.nameFieldStringValue = "Untitled.txt"
        }
        panel.begin { [weak self] result in
            guard result == .OK, let url = panel.url, let self else { return }
            DispatchQueue.main.async {
                guard let engine = self.engine else { return }
                url.path.withCString { suisei_engine_save_as(engine, $0) }
                self.refreshChrome()
            }
        }
    }

    func openFilePanel() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.allowsMultipleSelection = false
        panel.begin { [weak self] result in
            guard result == .OK, let url = panel.url else { return }
            DispatchQueue.main.async { self?.openPath(url.path) }
        }
    }

    func explorerSelect(_ index: Int) {
        guard let engine else { return }
        suisei_engine_explorer_select(engine, UInt32(index))
        refreshChrome()
    }

    func explorerActivate(_ index: Int) {
        guard let engine else { return }
        suisei_engine_explorer_activate(engine, UInt32(index))
        refreshChrome()
    }

    func toggleExplorer() {
        // Ctrl+F — same as xei. Drop any drag so layout doesn't re-click line 1.
        cancelPointerSession()
        dispatch(code: .char_, ch: UInt32(UnicodeScalar("f").value), mods: .control)
    }

    // MARK: - GUI editor commands (menus + standard chords)

    func undo() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_undo(engine)
        refreshChrome()
        ensureEditorFocus()
    }

    func redo() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_redo(engine)
        refreshChrome()
        ensureEditorFocus()
    }

    func selectAll() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_select_all(engine)
        refreshEditorPaintOnly()
    }

    /// ⌘F — open the incremental find bar (typing goes to the pattern).
    func openFind() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_find_open(engine)
        refreshChrome()
    }

    /// ⌘G / ⌘⇧G — jump to next / previous match.
    func findStep(forward: Bool) {
        guard let engine else { return }
        suisei_engine_find_step(engine, forward ? 1 : 0)
        refreshEditorPaintOnly()
    }

    /// Close the find bar keeping the caret at the current match.
    func closeFind() {
        guard chrome.search.open else { return }
        dispatchRaw(code: .enter)
        refreshChrome()
        ensureEditorFocus()
    }

    /// Insert arbitrary text at the caret (drag-drop, programmatic paste).
    func pasteText(_ text: String) {
        guard let engine, !text.isEmpty else { return }
        text.withCString { suisei_engine_paste_text(engine, $0) }
        refreshChrome()
    }

    /// Pull renderer: exact rows `[start, start+max)` for a pane, synchronously.
    func pullBand(pane: Int, start: Int, max maxRows: Int) -> [EditorLine] {
        guard let engine, maxRows > 0 else { return [] }
        var band = SuiseiBandC()
        let ok = suisei_engine_editor_band(
            engine, UInt32(pane), UInt32(max(0, start)), UInt32(maxRows), &band
        )
        guard ok != 0 else { return [] }
        var out: [EditorLine] = []
        let n = Int(band.count)
        let stride = MemoryLayout<SuiseiEditorLineC>.stride
        let textCap = Int(SUISEI_LINE_CAP)
        let spanStride = MemoryLayout<SuiseiSpanC>.stride
        let spanBaseOff = 32 + textCap
        withUnsafeBytes(of: band.lines) { raw in
            for i in 0..<min(n, Int(SUISEI_BAND_MAX)) {
                let base = raw.baseAddress!.advanced(by: i * stride)
                let lineNo = base.load(as: UInt32.self)
                let isCursor = base.load(fromByteOffset: 4, as: UInt8.self) != 0
                let gitSign = base.load(fromByteOffset: 5, as: UInt8.self)
                let spanCount = Int(base.load(fromByteOffset: 6, as: UInt8.self))
                let caret = base.load(fromByteOffset: 8, as: UInt32.self)
                let caretU16 = base.load(fromByteOffset: 12, as: UInt32.self)
                let sel0 = base.load(fromByteOffset: 16, as: UInt32.self)
                let sel1 = base.load(fromByteOffset: 20, as: UInt32.self)
                let selU0 = base.load(fromByteOffset: 24, as: UInt32.self)
                let selU1 = base.load(fromByteOffset: 28, as: UInt32.self)
                let text = readCString(at: base, offset: 32, cap: textCap)
                var spans: [SyntaxSpan] = []
                let nSpan = min(spanCount, Int(SUISEI_MAX_SPANS))
                let spanBase = base.advanced(by: spanBaseOff)
                for j in 0..<nSpan {
                    let sp = spanBase.advanced(by: j * spanStride)
                    let s0 = sp.load(as: UInt16.self)
                    let s1 = sp.load(fromByteOffset: 2, as: UInt16.self)
                    let kind = sp.load(fromByteOffset: 4, as: UInt8.self)
                    if s1 > s0 {
                        spans.append(SyntaxSpan(start: s0, end: s1, kind: kind))
                    }
                }
                out.append(EditorLine(
                    paneId: UInt8(pane), lineNo: lineNo, text: text, isCursor: isCursor,
                    caretVCol: caret, caretUTF16: caretU16, selV0: sel0, selV1: sel1,
                    selU0: selU0, selU1: selU1,
                    gitSign: gitSign, spans: spans
                ))
            }
        }
        return out
    }

    /// Toggle a bookmark/breakpoint on a 1-based line (gutter click).
    func toggleBreakpointLine(_ line1based: UInt32) {
        guard let engine, line1based > 0 else { return }
        suisei_engine_toggle_breakpoint_line(engine, line1based)
        refreshBreakpoints()
        refreshEditorPaintOnly()
    }

    /// Live split-divider drag.
    func splitSetRatio(_ ratio: Double) {
        guard let engine else { return }
        suisei_engine_split_set_ratio(engine, Float(ratio))
        refreshEditorPaintOnly()
    }

    struct MinimapData: Equatable {
        var totalLines: Int
        var indent: [UInt8]
        var len: [UInt8]
        var flags: [UInt8]
    }

    private var minimapCacheVersion: UInt64 = .max
    private var minimapCache: MinimapData?

    /// Downsampled document overview (cached per buffer version).
    func minimapData() -> MinimapData? {
        guard let engine else { return nil }
        if chrome.bufferVersion == minimapCacheVersion, let cached = minimapCache {
            return cached
        }
        let t0 = DispatchTime.now().uptimeNanoseconds
        defer {
            PerfProbe.record(
                "minimapData (cache miss)",
                Double(DispatchTime.now().uptimeNanoseconds - t0) / 1_000_000
            )
        }
        var snap = SuiseiMinimapC()
        guard suisei_engine_minimap(engine, &snap) != 0 else { return nil }
        let n = Int(snap.buckets)
        var indent = [UInt8](repeating: 0, count: n)
        var len = [UInt8](repeating: 0, count: n)
        var flags = [UInt8](repeating: 0, count: n)
        withUnsafeBytes(of: snap.indent) { raw in
            for i in 0..<min(n, Int(SUISEI_MINIMAP_MAX)) { indent[i] = raw[i] }
        }
        withUnsafeBytes(of: snap.len) { raw in
            for i in 0..<min(n, Int(SUISEI_MINIMAP_MAX)) { len[i] = raw[i] }
        }
        withUnsafeBytes(of: snap.flags) { raw in
            for i in 0..<min(n, Int(SUISEI_MINIMAP_MAX)) { flags[i] = raw[i] }
        }
        let data = MinimapData(
            totalLines: Int(snap.total_lines),
            indent: indent,
            len: len,
            flags: flags
        )
        minimapCacheVersion = chrome.bufferVersion
        minimapCache = data
        return data
    }

    /// Width of the document in display columns — the horizontal scroll extent.
    /// A high-water mark on the engine side (see `App::content_width`), so it
    /// never shrinks under a scroll in progress.
    func contentCols() -> UInt32 {
        guard let engine else { return 0 }
        return suisei_engine_content_cols(engine)
    }

    /// Move the caret without a full pointer session (context-menu placement).
    func placeCaret(row: UInt32, col: UInt32) {
        guard let engine else { return }
        suisei_engine_click(engine, row, col, 0)
        suisei_engine_mouse_up(engine)
        refreshEditorPaintOnly()
    }

    /// Route keys to the PTY (clicking the terminal) or back to the editor.
    func focusTerminal(_ on: Bool) {
        guard let engine else { return }
        suisei_engine_focus_terminal(engine, on ? 1 : 0)
        refreshChrome()
    }

    // Multi-session shells (VS Code-style).
    var terminalSessionCount: Int {
        guard let engine else { return 1 }
        return max(1, Int(suisei_engine_terminal_sessions(engine)))
    }

    var terminalActiveSession: Int {
        guard let engine else { return 0 }
        return Int(suisei_engine_terminal_active_session(engine))
    }

    func terminalNewSession() {
        guard let engine else { return }
        suisei_engine_terminal_new_session(engine)
        refreshChrome()
    }

    func terminalSelectSession(_ i: Int) {
        guard let engine else { return }
        suisei_engine_terminal_select_session(engine, UInt32(i))
        refreshChrome()
    }

    func terminalCloseSession(_ i: Int) {
        guard let engine else { return }
        suisei_engine_terminal_close_session(engine, UInt32(i))
        refreshChrome()
    }

    /// Scroll the terminal panel through its scrollback; positive = older.
    func terminalScroll(_ rows: Int32) {
        guard let engine, rows != 0 else { return }
        suisei_engine_terminal_scroll(engine, rows)
        refreshChrome()
    }

    /// Size the PTY grid to the terminal panel (cells).
    func terminalResize(cols: Int, rows: Int) {
        guard let engine, cols > 0, rows > 0 else { return }
        suisei_engine_terminal_resize(engine, UInt32(cols), UInt32(rows))
    }

    /// Tell Core the face has acted on its scroll intent.
    func clearScrollIntent() {
        guard let engine else { return }
        suisei_engine_clear_scroll_intent(engine)
    }

    func gotoTab(_ index: Int) {
        guard let engine else { return }
        suisei_engine_goto_tab(engine, UInt32(index))
        refreshChrome()
    }

    func closeTab(_ index: Int) {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_close_tab(engine, UInt32(index))
        refreshChrome()
        ensureEditorFocus()
    }

    /// Xcode “+ → New Untitled Tab” — always stays in editor shell (never Welcome).
    func openBlankTab() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_open_blank_tab(engine)
        refreshChrome()
        // Ensure face left Welcome even if Core chrome lagged one frame.
        if chrome.welcome, chrome.tabs.count > 1 {
            // Force a full refresh after compositor welcome fix.
            refreshChrome()
        }
        ensureEditorFocus()
    }

    /// Xcode “+ → Editor Pane On Right”
    func splitEditorRight() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_split_vertical(engine)
        refreshChrome()
    }

    /// Xcode “+ → Editor Pane Below”
    func splitEditorBelow() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_split_horizontal(engine)
        refreshChrome()
    }

    /// Jump caret to 1-based line (outline / jump bar). The engine centers the
    /// viewport; EditorScrollView glides there via the follow-caret path.
    func gotoLine(_ line1based: UInt32) {
        guard let engine, line1based > 0 else { return }
        cancelPointerSession()
        suisei_engine_goto_line(engine, line1based)
        refreshChrome()
    }

    /// Reload breakpoints list from Core (navigator panel).
    // MARK: - Quick Help inspector

    /// Asks the language server about the symbol under the caret, then polls
    /// for the reply — hover is a round trip, so the answer is never ready in
    /// the same frame as the question.
    func refreshHover() {
        guard let engine else {
            hoverText = ""
            return
        }
        suisei_engine_request_hover(engine)
        readHoverText()
        // One catch-up read after the server has had a chance to answer.
        hoverPoll?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.readHoverText() }
        hoverPoll = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35, execute: work)
    }

    private func readHoverText() {
        guard let engine else { return }
        var buf = [CChar](repeating: 0, count: Int(SUISEI_HOVER_TEXT))
        let ok = suisei_engine_hover_text(engine, &buf, UInt32(SUISEI_HOVER_TEXT))
        hoverText = ok != 0 ? String(cString: buf) : ""
    }

    // MARK: - Issue navigator

    func refreshDiagnostics() {
        guard let engine else {
            diagnostics = []
            return
        }
        var snap = SuiseiDiagnosticsSnapshot()
        guard suisei_engine_diagnostics(engine, &snap) != 0 else {
            diagnostics = []
            return
        }
        let n = Int(snap.count)
        var out: [DiagnosticItem] = []
        out.reserveCapacity(n)
        withUnsafePointer(to: &snap.rows) { rows in
        withUnsafePointer(to: &snap.cols) { cols in
        withUnsafePointer(to: &snap.severities) { sev in
        withUnsafePointer(to: &snap.messages) { msgs in
            let r = UnsafeRawPointer(rows).assumingMemoryBound(to: UInt32.self)
            let c = UnsafeRawPointer(cols).assumingMemoryBound(to: UInt32.self)
            let s = UnsafeRawPointer(sev).assumingMemoryBound(to: UInt8.self)
            let m = UnsafeRawPointer(msgs).assumingMemoryBound(to: CChar.self)
            for i in 0..<n {
                out.append(DiagnosticItem(
                    row: r[i], col: c[i], severity: s[i],
                    message: String(cString: m + i * Int(SUISEI_DIAG_MSG))
                ))
            }
        }}}}
        diagnostics = out
    }

    // MARK: - Find navigator

    /// Roots a project-wide grep has any business walking. `/`, `/Users` and a
    /// bare home directory are not projects — they are the whole machine.
    private static func isSearchableRoot(_ path: String) -> Bool {
        let trimmed = path.hasSuffix("/") && path.count > 1
            ? String(path.dropLast()) : path
        let forbidden = [
            "", "/", "/Users", "/Applications", "/System", "/Library", "/Volumes",
            NSHomeDirectory(),
        ]
        return !forbidden.contains(trimmed)
    }

    /// Runs the project grep OFF the main thread. The FFI takes no engine
    /// pointer precisely so this is safe; the background indexer already
    /// showed what a long main-thread job does to interaction.
    func searchProject(_ pattern: String) {
        let root = projectRoot.isEmpty ? chrome.explorer.cwd : projectRoot
        guard !pattern.isEmpty, !root.isEmpty else {
            searchHits = []
            searchTruncated = false
            searchRunning = false
            searchMessage = ""
            return
        }
        // Refuse roots that are not a project. With no folder open the tree
        // sits at "/", and grepping that walks the entire disk — System,
        // Library, every mounted volume — for a query the user meant to scope
        // to their code.
        guard Self.isSearchableRoot(root) else {
            searchHits = []
            searchTruncated = false
            searchRunning = false
            searchMessage = "Open a project folder to search."
            return
        }
        searchMessage = ""
        searchGeneration &+= 1
        let generation = searchGeneration
        searchRunning = true
        DispatchQueue.global(qos: .userInitiated).async {
            // HEAP, not stack. This snapshot is ~228KB (300 × (512 + 240)) and
            // a global-queue thread only gets 512KB — declaring it as a local
            // blew the stack outright (EXC_BAD_ACCESS, "Thread stack size
            // exceeded") before a single result came back.
            let snap = UnsafeMutablePointer<SuiseiSearchHitsSnapshot>.allocate(capacity: 1)
            defer { snap.deallocate() }
            snap.initialize(to: SuiseiSearchHitsSnapshot())
            let ok = root.withCString { r in
                pattern.withCString { p in
                    suisei_engine_search_project(r, p, snap)
                }
            }
            var out: [SearchHitItem] = []
            var truncated = false
            if ok != 0 {
                truncated = snap.pointee.truncated != 0
                let n = Int(snap.pointee.count)
                out.reserveCapacity(n)
                let base = UnsafeRawPointer(snap)
                let rr = base.advanced(by: MemoryLayout<SuiseiSearchHitsSnapshot>
                    .offset(of: \.rows)!).assumingMemoryBound(to: UInt32.self)
                let cc = base.advanced(by: MemoryLayout<SuiseiSearchHitsSnapshot>
                    .offset(of: \.cols)!).assumingMemoryBound(to: UInt32.self)
                let pp = base.advanced(by: MemoryLayout<SuiseiSearchHitsSnapshot>
                    .offset(of: \.paths)!).assumingMemoryBound(to: CChar.self)
                let ll = base.advanced(by: MemoryLayout<SuiseiSearchHitsSnapshot>
                    .offset(of: \.lines)!).assumingMemoryBound(to: CChar.self)
                for i in 0..<n {
                    out.append(SearchHitItem(
                        path: String(cString: pp + i * Int(SUISEI_HIT_PATH)),
                        row: rr[i], col: cc[i],
                        line: String(cString: ll + i * Int(SUISEI_HIT_LINE))
                    ))
                }
            }
            let result = out
            let cut = truncated
            DispatchQueue.main.async { [weak self] in
                guard let self, generation == self.searchGeneration else { return }
                self.searchHits = result
                self.searchTruncated = cut
                self.searchRunning = false
            }
        }
    }

    func openSearchHit(_ hit: SearchHitItem) {
        openPath(hit.path)
        gotoLine(hit.row + 1)
    }

    // ── Find All References (LSP) ────────────────────────────────────────
    /// Ask the language server for every reference to the symbol under the
    /// cursor. The answer is asynchronous; `pollReferencesIfNeeded()` on the
    /// tick loop picks it up. Takes over the Find navigator until dismissed.
    func requestReferences() {
        guard let engine else { return }
        references = []
        referencesReady = false
        referencesTruncated = false
        referencesActive = true
        suisei_engine_request_references(engine)
    }

    /// Return the Find navigator to project search.
    func dismissReferences() {
        referencesActive = false
        references = []
        referencesReady = false
    }

    /// Cheap while the server is still thinking (`ready == 0`); decodes the
    /// result once and then stops polling.
    func pollReferencesIfNeeded() {
        guard referencesActive, !referencesReady, let engine else { return }
        let snap = UnsafeMutablePointer<SuiseiReferencesSnapshot>.allocate(capacity: 1)
        defer { snap.deallocate() }
        snap.initialize(to: SuiseiReferencesSnapshot())
        let ok = suisei_engine_references(engine, snap)
        guard ok != 0, snap.pointee.ready != 0 else { return }
        var out: [SearchHitItem] = []
        let n = Int(snap.pointee.count)
        out.reserveCapacity(n)
        let base = UnsafeRawPointer(snap)
        let rr = base.advanced(by: MemoryLayout<SuiseiReferencesSnapshot>
            .offset(of: \.rows)!).assumingMemoryBound(to: UInt32.self)
        let cc = base.advanced(by: MemoryLayout<SuiseiReferencesSnapshot>
            .offset(of: \.cols)!).assumingMemoryBound(to: UInt32.self)
        let pp = base.advanced(by: MemoryLayout<SuiseiReferencesSnapshot>
            .offset(of: \.paths)!).assumingMemoryBound(to: CChar.self)
        let ll = base.advanced(by: MemoryLayout<SuiseiReferencesSnapshot>
            .offset(of: \.lines)!).assumingMemoryBound(to: CChar.self)
        for i in 0..<n {
            out.append(SearchHitItem(
                path: String(cString: pp + i * Int(SUISEI_REF_PATH)),
                row: rr[i], col: cc[i],
                line: String(cString: ll + i * Int(SUISEI_REF_LINE))
            ))
        }
        references = out
        referencesTruncated = snap.pointee.truncated != 0
        referencesReady = true
    }

    /// Replace the first match of `query` on the hit's line (atomic write).
    @discardableResult
    func replaceSearchHit(_ hit: SearchHitItem, query: String, replace: String) -> Bool {
        guard !query.isEmpty else { return false }
        let ok = hit.path.withCString { p in
            query.withCString { q in
                replace.withCString { r in
                    suisei_engine_replace_in_file(p, hit.row, q, r) != 0
                }
            }
        }
        if ok {
            // Refresh open buffer if this path is current.
            if chrome.filename == hit.path || openPathMatches(hit.path) {
                openPath(hit.path)
            }
            searchProject(query)
        }
        return ok
    }

    /// Replace every occurrence of `query` across unique paths in current hits.
    @discardableResult
    func replaceAllSearchHits(query: String, replace: String) -> Int {
        guard !query.isEmpty else { return 0 }
        var total = 0
        var seen = Set<String>()
        for hit in searchHits {
            if seen.insert(hit.path).inserted {
                let n = hit.path.withCString { p in
                    query.withCString { q in
                        replace.withCString { r in
                            Int(suisei_engine_replace_all_in_file(p, q, r))
                        }
                    }
                }
                total += n
            }
        }
        if total > 0 {
            searchProject(query)
            refreshChrome()
        }
        return total
    }

    private func openPathMatches(_ path: String) -> Bool {
        chrome.filename == path
            || chrome.tabs.contains(where: { $0.title == (path as NSString).lastPathComponent })
    }

    // MARK: - LSP face surfaces

    func formatDocument() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_format_document(engine)
        refreshChrome()
    }

    func gotoDefinition() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_goto_definition(engine)
        refreshChrome()
        ensureEditorFocus()
    }

    func renameSymbol(_ newName: String) {
        guard let engine, !newName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        cancelPointerSession()
        newName.withCString { suisei_engine_rename_symbol(engine, $0) }
        refreshChrome()
    }

    func requestCodeActions() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_code_actions(engine)
        refreshChrome()
    }

    /// Prompt for a rename name (NSAlert) then call LSP rename.
    func promptRenameSymbol() {
        let alert = NSAlert()
        alert.messageText = "Rename Symbol"
        alert.informativeText = "Enter the new name for the symbol under the caret."
        alert.alertStyle = .informational
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.stringValue = ""
        alert.accessoryView = field
        alert.addButton(withTitle: "Rename")
        alert.addButton(withTitle: "Cancel")
        DispatchQueue.main.async {
            field.becomeFirstResponder()
        }
        if alert.runModal() == .alertFirstButtonReturn {
            renameSymbol(field.stringValue)
        }
    }

    func refreshBreakpoints() {
        guard let engine else {
            breakpoints = []
            return
        }
        breakpoints = loadBreakpoints(engine)
    }

    func gotoBreakpoint(_ item: BreakpointItem) {
        guard let engine else { return }
        cancelPointerSession()
        item.path.withCString { suisei_engine_goto_breakpoint(engine, $0, item.line) }
        refreshChrome()
        ensureEditorFocus()
    }

    func removeBreakpoint(_ item: BreakpointItem) {
        guard let engine else { return }
        cancelPointerSession()
        item.path.withCString { suisei_engine_remove_breakpoint(engine, $0, item.line) }
        refreshBreakpoints()
        refreshEditorPaintOnly()
    }

    /// F9 — toggle BP on current cursor line.
    func toggleBreakpointAtCursor() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_toggle_breakpoint_cursor(engine)
        refreshBreakpoints()
        refreshChrome()
    }

    func paletteSelect(_ index: Int) {
        guard let engine else { return }
        suisei_engine_palette_select(engine, UInt32(index))
        refreshChrome()
    }

    func paletteActivate(_ index: Int) {
        guard let engine else { return }
        suisei_engine_palette_activate(engine, UInt32(index))
        refreshChrome()
    }

    func openFilePalette() {
        dispatch(code: .char_, ch: UInt32(UnicodeScalar("p").value), mods: .superKey)
    }

    func openCommandPalette() {
        // Cmd+Shift+P
        var m = SuiseiMod.superKey
        m.insert(.shift)
        dispatch(code: .char_, ch: UInt32(UnicodeScalar("p").value), mods: m)
    }

    // MARK: - Keys

    /// True when key events belong to the editor shell (never alerts, open/save
    /// panels, the Settings / Welcome windows, or a focused native text field —
    /// e.g. the project-tree Filter — those keep native handling).
    private var editorOwnsKeyEvents: Bool {
        if NSApp.modalWindow != nil { return false }
        guard let key = NSApp.keyWindow else { return true }
        if key is NSPanel { return false }
        if key.title == "Settings" || key.title == "Welcome" { return false }
        if let responder = key.firstResponder,
           // Our own canvas is an NSTextInputClient now — it must stay
           // "editor-owned" so the ⌘-chords below still reach the engine.
           // Plain typing is handed back to it at the tail of the monitor.
           !(responder is EditorCanvasView),
           responder is NSTextView || responder is NSTextField
               || responder is NSTextInputClient
        {
            // Focused native/SwiftUI text input (tree filter, future fields).
            return false
        }
        return true
    }

    /// The editor canvas has focus, so plain keys must flow through its
    /// `NSTextInputClient` path (input method, standard key bindings) instead
    /// of being swallowed here.
    private var editorCanvasHasFocus: Bool {
        NSApp.keyWindow?.firstResponder is EditorCanvasView
    }

    private func installKeyMonitor() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            // Swallowing keys during a modal (clone-URL alert, save panel) made
            // those text fields untypable — pass through anything non-editor.
            guard self.editorOwnsKeyEvents else { return event }
            if event.modifierFlags.contains(.command) {
                let c = event.charactersIgnoringModifiers?.lowercased() ?? ""
                let hasCtrl = event.modifierFlags.contains(.control)
                let hasShift = event.modifierFlags.contains(.shift)
                // System/menu chords route natively (⌘N = New, ⌘0/⌥⌘0 = panels).
                if c == "q" || c == "h" || c == "m" || c == "n" { return event }
                // ⌘W closes the TAB (GUI-editor standard) — ⇧⌘W closes the window.
                if c == "w" {
                    if hasShift { return event }
                    if let active = self.chrome.tabs.first(where: \.active) {
                        self.closeTab(active.id)
                    }
                    return nil
                }
                if c == "o" { self.openProjectFolder(); return nil }
                if c == "s" {
                    // save() decides panel-vs-core (unnamed/relative buffers need Save As).
                    if hasShift { self.saveAsPanel() } else { self.save() }
                    return nil
                }
                // Standard editor chords (GUI contract — no vim needed).
                if c == "z", !self.terminalOwnsKeys {
                    if hasShift { self.redo() } else { self.undo() }
                    return nil
                }
                if c == "a", !self.terminalOwnsKeys {
                    self.selectAll()
                    return nil
                }
                if c == "f", !hasShift, !hasCtrl {
                    self.openFind()
                    return nil
                }
                if c == "g", !hasCtrl, !event.modifierFlags.contains(.option),
                   self.chrome.search.matchCount > 0 || self.chrome.search.open
                {
                    self.findStep(forward: !hasShift)
                    return nil
                }
                // Cmd+, → Settings (VS Code)
                if c == "," {
                    self.dispatch(
                        code: .char_,
                        ch: UInt32(UnicodeScalar(",").value),
                        mods: .superKey
                    )
                    return nil
                }
                // Cmd+ / Cmd= → zoom in · Cmd- → zoom out · Ctrl+Cmd+0 → reset.
                // Plain ⌘0 / ⌥⌘0 stay with the menu (navigator / inspector).
                if c == "=" || c == "+" {
                    self.zoomFont(delta: 1)
                    return nil
                }
                if c == "-" || c == "_" {
                    self.zoomFont(delta: -1)
                    return nil
                }
                if c == "0" {
                    if hasCtrl {
                        self.resetFontZoom()
                        return nil
                    }
                    return event
                }
            }
            // ⌃⇥ / ⌃⇧⇥ — cycle document tabs (standard macOS).
            if event.keyCode == 48, event.modifierFlags.contains(.control) {
                if event.modifierFlags.contains(.shift) {
                    self.prevTab()
                } else {
                    self.nextTab()
                }
                return nil
            }
            // Plain typing goes to the canvas's NSTextInputClient path so macOS
            // runs the input method (Hangul et al.) and resolves the standard
            // key bindings. Swallowing it here is what made IME impossible.
            if self.editorCanvasHasFocus { return event }
            self.handleNSEvent(event)
            return nil
        }
    }

    func openSettings() {
        cancelPointerSession()
        dispatch(code: .char_, ch: UInt32(UnicodeScalar(",").value), mods: .control)
    }

    func settingsSelect(_ row: Int) {
        guard let engine else { return }
        suisei_engine_settings_select(engine, UInt32(row))
        refreshChrome()
    }

    func settingsActivate(_ row: Int) {
        guard let engine else { return }
        suisei_engine_settings_activate(engine, UInt32(row))
        refreshChrome()
    }

    func settingsGotoPage(_ page: Int) {
        guard let engine else { return }
        suisei_engine_settings_goto_page(engine, UInt32(page))
        refreshChrome()
    }

    private func installScrollMonitor() {
        // Editor scroll is owned exclusively by EditorScrollView / NSScrollView panels.
        // Never dual-drive Core via scrollByFrac here — that desynced clip vs viewport.
        // Only block wheel during pointer-drag select.
        scrollMonitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
            guard let self else { return event }
            if self.pointerSession { return nil }
            return event
        }
    }

    func handleNSEvent(_ event: NSEvent) {
        var mods = SuiseiMod()
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if flags.contains(.shift) { mods.insert(.shift) }
        if flags.contains(.control) { mods.insert(.control) }
        if flags.contains(.option) { mods.insert(.alt) }
        if flags.contains(.command) { mods.insert(.superKey) }

        switch event.keyCode {
        case 53: dispatch(code: .esc, mods: mods); return
        case 36, 76: dispatch(code: .enter, mods: mods); return
        case 51: dispatch(code: .backspace, mods: mods); return
        case 117: dispatch(code: .delete, mods: mods); return
        case 48: dispatch(code: flags.contains(.shift) ? .backtab : .tab, mods: mods); return
        case 123: dispatch(code: .left, mods: mods); return
        case 124: dispatch(code: .right, mods: mods); return
        case 125: dispatch(code: .down, mods: mods); return
        case 126: dispatch(code: .up, mods: mods); return
        case 115: dispatch(code: .home, mods: mods); return
        case 119: dispatch(code: .end, mods: mods); return
        case 116: dispatch(code: .pageUp, mods: mods); return
        case 121: dispatch(code: .pageDown, mods: mods); return
        // F9 — toggle breakpoint (xei / VS Code convention)
        case 101:
            toggleBreakpointAtCursor()
            return
        default: break
        }

        if let ch = resolveCharacter(event: event) {
            // Explicit Mac chords (belt + suspenders for Super/Ctrl+Shift).
            let lower = ch.lowercased()
            if mods.contains(.shift), mods.contains(.superKey) || mods.contains(.control) {
                if lower == "v" {
                    // Super+Shift+V = preview; Ctrl+Shift+V also preview in xei (cmd_like).
                    // When terminal pane owns keys, Core pastes into PTY instead.
                    if !terminalOwnsKeys {
                        togglePreview()
                        return
                    }
                }
                if lower == "t" {
                    toggleTerminalFull()
                    return
                }
            }
            if mods.contains(.control) || mods.contains(.superKey) || mods.contains(.alt) {
                guard let scalar = ch.unicodeScalars.first else { return }
                dispatch(code: .char_, ch: scalar.value, mods: mods)
            } else if ch.isLetter {
                let lowerCh = Character(ch.lowercased())
                var m = mods
                if ch.isUppercase { m.insert(.shift) }
                guard let scalar = lowerCh.unicodeScalars.first else { return }
                dispatch(code: .char_, ch: scalar.value, mods: m)
            } else {
                insertChar(ch)
            }
        }
    }

    private func resolveCharacter(event: NSEvent) -> Character? {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if let s = event.charactersIgnoringModifiers {
            for u in s.unicodeScalars where u.value >= 32 && u.value != 127 {
                return Character(u)
            }
        }
        if let s = event.characters {
            for u in s.unicodeScalars where u.value >= 32 && u.value != 127 {
                return Character(u)
            }
        }
        return usAnsiChar(keyCode: event.keyCode, shift: flags.contains(.shift))
    }

    private func usAnsiChar(keyCode: UInt16, shift: Bool) -> Character? {
        let map: [UInt16: (Character, Character)] = [
            0: ("a", "A"), 1: ("s", "S"), 2: ("d", "D"), 3: ("f", "F"),
            4: ("h", "H"), 5: ("g", "G"), 6: ("z", "Z"), 7: ("x", "X"),
            8: ("c", "C"), 9: ("v", "V"), 11: ("b", "B"), 12: ("q", "Q"),
            13: ("w", "W"), 14: ("e", "E"), 15: ("r", "R"), 16: ("y", "Y"),
            17: ("t", "T"), 18: ("1", "!"), 19: ("2", "@"), 20: ("3", "#"),
            21: ("4", "$"), 22: ("6", "^"), 23: ("5", "%"), 24: ("=", "+"),
            25: ("9", "("), 26: ("7", "&"), 27: ("-", "_"), 28: ("8", "*"),
            29: ("0", ")"), 30: ("]", "}"), 31: ("o", "O"), 32: ("u", "U"),
            33: ("[", "{"), 34: ("i", "I"), 35: ("p", "P"), 37: ("l", "L"),
            38: ("j", "J"), 39: ("'", "\""), 40: ("k", "K"), 41: (";", ":"),
            42: ("\\", "|"), 43: (",", "<"), 44: ("/", "?"), 45: ("n", "N"),
            46: ("m", "M"), 47: (".", ">"), 49: (" ", " "), 50: ("`", "~"),
        ]
        guard let pair = map[keyCode] else { return nil }
        return shift ? pair.1 : pair.0
    }

    /// Scroll path: pull line window + Core residual together (atomic paint inputs).
    private func refreshChromeScrollOnly() {
        refreshEditorPaintOnly()
    }

    /// Light path: editor lines + status/caret/tabs/mode — **no** explorer/outline/scm/git/settings FFI.
    /// Used for typing, scroll, mid-resize. Full shell uses `refreshChrome()`.
    func refreshEditorPaintOnly() {
        let t0 = DispatchTime.now().uptimeNanoseconds
        defer {
            PerfProbe.record(
                "refreshEditorPaintOnly",
                Double(DispatchTime.now().uptimeNanoseconds - t0) / 1_000_000
            )
        }
        guard let engine else { return }
        var snap = PerfProbe.measure("  snapshot alloc (180KiB)") { SuiseiChromeSnapshot() }
        guard PerfProbe.measure("  suisei_engine_chrome", { suisei_engine_chrome(engine, &snap) }) != 0
        else { return }
        if snap.frame_gen == lastFrameGen { return }

        var tabs: [TabItem] = []
        let tabCount = Int(snap.tab_count)
        withUnsafeBytes(of: snap.tab_titles) { titlesRaw in
            withUnsafeBytes(of: snap.tab_dirty) { dirtyRaw in
                let titleCap = Int(SUISEI_TITLE_CAP)
                for i in 0..<min(tabCount, Int(SUISEI_MAX_TABS)) {
                    let base = titlesRaw.baseAddress!.advanced(by: i * titleCap)
                    let title = String(cString: base.assumingMemoryBound(to: CChar.self))
                    tabs.append(TabItem(
                        id: i,
                        title: title.isEmpty ? "[No Name]" : title,
                        dirty: dirtyRaw[i] != 0,
                        active: i == Int(snap.tab_active)
                    ))
                }
            }
        }

        let (lines, split) = PerfProbe.measure("  decodeEditorLinesAndSplit") {
            decodeEditorLinesAndSplit(from: snap)
        }
        lastFrameGen = snap.frame_gen
        let frac = CGFloat(snap.scroll_frac)

        let linesDiffer = PerfProbe.measure("  lines != editorLines") { lines != editorLines }
        PerfProbe.measure("  publish") {
            var txn = Transaction()
            txn.disablesAnimations = true
            withTransaction(txn) {
                if abs(frac - editorScrollFrac) > 0.0001 { editorScrollFrac = frac }
                if snap.hscroll != editorHScroll { editorHScroll = snap.hscroll }
                let wrap = snap.wrap_lines != 0
                if wrap != wrapLines { wrapLines = wrap }
                if linesDiffer { editorLines = lines }
                if split != editorSplit { editorSplit = split }
            }
        }
        // Preview can open/close without full shell rebuild — but only re-pull
        // when its inputs changed (a full 8k-line copy per keystroke flickered).
        refreshPreviewIfNeeded(engine, bufferVersion: snap.buffer_version)

        // Patch chrome in place — keep heavy surfaces (explorer/outline/scm) from previous full refresh.
        var next = chrome
        next.gen = snap.frame_gen
        next.modeLabel = cStringField(snap.mode_label)
        next.message = cStringField(snap.message)
        next.filename = cStringField(snap.filename)
        next.breadcrumbs = cStringField(snap.breadcrumbs)
        next.dirty = snap.dirty_buffer != 0
        next.welcome = snap.welcome != 0
        next.cursorRow = snap.cursor_row
        next.cursorCol = snap.cursor_col
        next.caretVCol = snap.caret_vcol
        next.lineCount = snap.line_count
        next.scroll = snap.scroll
        next.pct = snap.pct
        next.bufferVersion = snap.buffer_version
        next.tabs = tabs
        next.lines = lines
        next.split = split
        // Cheap open-flag probes only (empty payload when closed).
        let palette = loadPalette(engine)
        next.palette = palette.open ? palette : .empty
        let search = loadSearch(engine)
        next.search = search.open ? search : .empty
        let completions = loadCompletions(engine)
        next.completions = completions.open ? completions : .empty
        let terminal = loadTerminal(engine)
        next.terminal = terminal.open ? terminal : .empty
        // Outline is cheap to copy and the engine refreshes it on idle ticks —
        // keep it live on the light path too (typing never does a full pull).
        let outline = loadOutline(engine)
        if outline != next.outline { next.outline = outline }

        if next != chrome {
            chrome = next
        }
    }

    private func decodeEditorLinesAndSplit(from snap: SuiseiChromeSnapshot) -> ([EditorLine], SplitSnap) {
        var allLines: [EditorLine] = []
        let vis = Int(snap.visible_line_count)
        let stride = MemoryLayout<SuiseiEditorLineC>.stride
        let spanStride = MemoryLayout<SuiseiSpanC>.stride
        let textCap = Int(SUISEI_LINE_CAP)
        let spanBaseOff = 32 + textCap
        withUnsafeBytes(of: snap.lines) { raw in
            for i in 0..<min(vis, Int(SUISEI_MAX_LINES)) {
                let base = raw.baseAddress!.advanced(by: i * stride)
                let lineNo = base.load(as: UInt32.self)
                let isCursor = base.load(fromByteOffset: 4, as: UInt8.self) != 0
                let gitSign = base.load(fromByteOffset: 5, as: UInt8.self)
                let spanCount = Int(base.load(fromByteOffset: 6, as: UInt8.self))
                let caret = base.load(fromByteOffset: 8, as: UInt32.self)
                let caretU16 = base.load(fromByteOffset: 12, as: UInt32.self)
                let sel0 = base.load(fromByteOffset: 16, as: UInt32.self)
                let sel1 = base.load(fromByteOffset: 20, as: UInt32.self)
                let selU0 = base.load(fromByteOffset: 24, as: UInt32.self)
                let selU1 = base.load(fromByteOffset: 28, as: UInt32.self)
                let text = readCString(at: base, offset: 32, cap: textCap)
                var spans: [SyntaxSpan] = []
                let nSpan = min(spanCount, Int(SUISEI_MAX_SPANS))
                let spanBase = base.advanced(by: spanBaseOff)
                for j in 0..<nSpan {
                    let sp = spanBase.advanced(by: j * spanStride)
                    let start = sp.load(as: UInt16.self)
                    let end = sp.load(fromByteOffset: 2, as: UInt16.self)
                    let kind = sp.load(fromByteOffset: 4, as: UInt8.self)
                    if end > start {
                        spans.append(SyntaxSpan(start: start, end: end, kind: kind))
                    }
                }
                allLines.append(EditorLine(
                    paneId: 0, lineNo: lineNo, text: text, isCursor: isCursor,
                    caretVCol: caret, caretUTF16: caretU16, selV0: sel0, selV1: sel1,
                    selU0: selU0, selU1: selU1,
                    gitSign: gitSign, spans: spans
                ))
            }
        }

        // Split metadata (ABI fields added after tab titles — default 0 when old engine).
        let kind = snap.split_kind
        let paneCount = Int(snap.pane_count)
        let focus = Int(snap.pane_focus)
        let ratio = snap.split_ratio

        if kind == 0 || paneCount < 2 {
            let single = EditorPaneSnap(
                id: 0, focused: true, tabIndex: Int(snap.tab_active),
                scroll: snap.scroll, hscroll: snap.hscroll,
                docLineCount: snap.line_count, lines: allLines
            )
            // Stamp paneId 0 on all lines.
            return (allLines, SplitSnap(kind: 0, ratio: 0.5, focus: 0, panes: [single]))
        }

        // C fixed arrays import as tuples in Swift — walk via raw memory.
        // Layout must match SuiseiPaneC (tab, scroll, start, count, focused+pad, doc_line_count, hscroll).
        var panes: [EditorPaneSnap] = []
        var focusedLines: [EditorLine] = []
        let paneStride = MemoryLayout<SuiseiPaneC>.stride
        let nPanes = min(paneCount, Int(SUISEI_MAX_PANES))
        withUnsafeBytes(of: snap.panes) { raw in
            for pi in 0..<nPanes {
                let base = raw.baseAddress!.advanced(by: pi * paneStride)
                let tabIndex = base.load(as: UInt32.self)
                let scroll = base.load(fromByteOffset: 4, as: UInt32.self)
                let lineStart = base.load(fromByteOffset: 8, as: UInt32.self)
                let lineCount = base.load(fromByteOffset: 12, as: UInt32.self)
                let focusedFlag = base.load(fromByteOffset: 16, as: UInt8.self)
                // offset 20: doc_line_count, 24: hscroll (after 4 pad bytes at 16..19)
                let docLineCount = base.load(fromByteOffset: 20, as: UInt32.self)
                let hscroll = base.load(fromByteOffset: 24, as: UInt32.self)
                let start = Int(lineStart)
                let count = Int(lineCount)
                var paneLines: [EditorLine] = []
                for j in 0..<count {
                    let idx = start + j
                    guard idx < allLines.count else { break }
                    var line = allLines[idx]
                    line.paneId = UInt8(pi)
                    paneLines.append(line)
                }
                let isFocused = focusedFlag != 0 || pi == focus
                if isFocused {
                    focusedLines = paneLines
                }
                panes.append(EditorPaneSnap(
                    id: pi,
                    focused: isFocused,
                    tabIndex: Int(tabIndex),
                    scroll: scroll,
                    hscroll: hscroll,
                    docLineCount: max(1, docLineCount),
                    lines: paneLines
                ))
            }
        }
        // `lines` = focused pane only (never the packed multi-pane stream).
        return (
            focusedLines.isEmpty ? allLines : focusedLines,
            SplitSnap(kind: kind, ratio: ratio == 0 ? 0.5 : ratio, focus: focus, panes: panes)
        )
    }

    func refreshChrome() {
        let t0 = DispatchTime.now().uptimeNanoseconds
        defer {
            PerfProbe.record(
                "refreshChrome (full shell)",
                Double(DispatchTime.now().uptimeNanoseconds - t0) / 1_000_000
            )
        }
        guard let engine else { return }
        var snap = SuiseiChromeSnapshot()
        guard suisei_engine_chrome(engine, &snap) != 0 else { return }

        var tabs: [TabItem] = []
        let tabCount = Int(snap.tab_count)
        withUnsafeBytes(of: snap.tab_titles) { titlesRaw in
            withUnsafeBytes(of: snap.tab_dirty) { dirtyRaw in
                let titleCap = Int(SUISEI_TITLE_CAP)
                for i in 0..<min(tabCount, Int(SUISEI_MAX_TABS)) {
                    let base = titlesRaw.baseAddress!.advanced(by: i * titleCap)
                    let title = String(cString: base.assumingMemoryBound(to: CChar.self))
                    tabs.append(TabItem(
                        id: i,
                        title: title.isEmpty ? "[No Name]" : title,
                        dirty: dirtyRaw[i] != 0,
                        active: i == Int(snap.tab_active)
                    ))
                }
            }
        }

        let (lines, split) = decodeEditorLinesAndSplit(from: snap)
        if lines != editorLines { editorLines = lines }
        if split != editorSplit { editorSplit = split }
        // Always adopt Core residual (caret/goto clear it to 0).
        let frac = CGFloat(snap.scroll_frac)
        if abs(frac - editorScrollFrac) > 0.0001 {
            editorScrollFrac = frac
        }
        if snap.hscroll != editorHScroll { editorHScroll = snap.hscroll }
        let wrap = snap.wrap_lines != 0
        if wrap != wrapLines { wrapLines = wrap }
        preview = loadPreview(engine)

        // Always load explorer entries: docked Project navigator stays visible in Normal mode.
        // (TUI-style open flag only affects keyboard focus, not tree paint.)
        let explorer = loadExplorer(engine)
        let palette = loadPalette(engine)
        let paletteOut = palette.open ? palette : PaletteSnap.empty
        let search = loadSearch(engine)
        let searchOut = search.open ? search : SearchSnap.empty
        let completions = loadCompletions(engine)
        let compOut = completions.open ? completions : CompletionsSnap.empty
        let terminal = loadTerminal(engine)
        let termOut = terminal.open ? terminal : TerminalSnap.empty
        let settings = loadSettings(engine)
        let settingsOut = settings.open ? settings : SettingsSnap.empty
        // Always pull SCM / Git WB when Core marks them open (mode-independent docked UI).
        let scm = loadScm(engine)
        let scmOut = scm
        let gitWb = loadGitWb(engine)
        let gitWbOut = gitWb
        let theme = loadTheme(engine)
        let outline = loadOutline(engine)
        var branch = ""
        var statusEx = SuiseiStatusExtra()
        if suisei_engine_status_extra(engine, &statusEx) != 0 {
            branch = cStringField(statusEx.branch)
        }

        lastFrameGen = snap.frame_gen
        let next = ChromeSnapshot(
            gen: snap.frame_gen,
            modeLabel: cStringField(snap.mode_label),
            message: cStringField(snap.message),
            filename: cStringField(snap.filename),
            breadcrumbs: cStringField(snap.breadcrumbs),
            dirty: snap.dirty_buffer != 0,
            welcome: snap.welcome != 0,
            explorerOpen: snap.explorer_open != 0 || explorer.open,
            cursorRow: snap.cursor_row,
            cursorCol: snap.cursor_col,
            caretVCol: snap.caret_vcol,
            scrollIntent: snap.scroll_intent,
            lineCount: snap.line_count,
            scroll: snap.scroll,
            pct: snap.pct,
            bufferVersion: snap.buffer_version,
            branch: branch,
            tabs: tabs,
            lines: lines,
            split: split,
            explorer: explorer,
            palette: paletteOut,
            search: searchOut,
            completions: compOut,
            terminal: termOut,
            settings: settingsOut,
            theme: theme,
            scm: scmOut,
            gitWb: gitWbOut,
            outline: outline
        )
        // Equatable skip avoids SwiftUI thrash when nothing visual changed.
        if next != chrome {
            chrome = next
        }
    }

    private func loadScm(_ engine: OpaquePointer) -> ScmSnap {
        var snap = SuiseiScmSnapshot()
        guard suisei_engine_scm(engine, &snap) != 0 else { return .empty }
        // open==0 → empty payload (scene already cleared lists)
        if snap.open == 0 { return .empty }
        var staged: [ScmEntryItem] = []
        var changes: [ScmEntryItem] = []
        let total = Int(snap.staged_count + snap.change_count)
        withUnsafeBytes(of: snap.paths) { pathRaw in
            withUnsafeBytes(of: snap.marks) { markRaw in
                withUnsafeBytes(of: snap.staged_flags) { flagRaw in
                    let cap = Int(SUISEI_SCM_PATH)
                    for i in 0..<min(total, Int(SUISEI_MAX_SCM)) {
                        let pb = pathRaw.baseAddress!.advanced(by: i * cap)
                        let path = String(cString: pb.assumingMemoryBound(to: CChar.self))
                        let markByte = UInt8(bitPattern: CChar(markRaw[i]))
                        let mark = markByte == 0 ? "?" : String(UnicodeScalar(markByte))
                        let item = ScmEntryItem(
                            id: i,
                            path: path,
                            mark: mark,
                            staged: flagRaw[i] != 0,
                            selected: i == Int(snap.selected)
                        )
                        if flagRaw[i] != 0 {
                            staged.append(item)
                        } else {
                            changes.append(item)
                        }
                    }
                }
            }
        }
        var graph: [ScmGraphItem] = []
        let gn = Int(snap.graph_count)
        withUnsafeBytes(of: snap.graph_lines) { gRaw in
            withUnsafeBytes(of: snap.graph_selected) { selRaw in
                let gCap = Int(SUISEI_GRAPH_LINE)
                for i in 0..<min(gn, Int(SUISEI_MAX_SCM_GRAPH)) {
                    let gb = gRaw.baseAddress!.advanced(by: i * gCap)
                    graph.append(ScmGraphItem(
                        id: i,
                        line: String(cString: gb.assumingMemoryBound(to: CChar.self)),
                        selected: selRaw[i] != 0
                    ))
                }
            }
        }
        return ScmSnap(
            open: true,
            branch: cStringField(snap.branch),
            status: cStringField(snap.status),
            staged: staged,
            changes: changes,
            graph: graph
        )
    }

    private func loadGitWb(_ engine: OpaquePointer) -> GitWbSnap {
        var snap = SuiseiGitWbSnapshot()
        guard suisei_engine_git_wb(engine, &snap) != 0, snap.open != 0 else { return .empty }
        var chips: [GitWbChipItem] = []
        let cn = Int(snap.chip_count)
        withUnsafeBytes(of: snap.chip_labels) { labRaw in
            withUnsafeBytes(of: snap.chip_active) { actRaw in
                withUnsafeBytes(of: snap.chip_keys) { keyRaw in
                    for i in 0..<min(cn, Int(SUISEI_MAX_GIT_CHIPS)) {
                        let lb = labRaw.baseAddress!.advanced(by: i * 24)
                        chips.append(GitWbChipItem(
                            id: i,
                            label: String(cString: lb.assumingMemoryBound(to: CChar.self)),
                            active: actRaw[i] != 0,
                            key: Int(keyRaw[i])
                        ))
                    }
                }
            }
        }
        func loadCol(_ count: UInt32, _ field: UnsafeRawPointer) -> [String] {
            var out: [String] = []
            let n = Int(count)
            let cap = Int(SUISEI_GIT_WB_LINE)
            let colMax = Int(SUISEI_MAX_GIT_COL)
            for i in 0..<min(n, colMax) {
                let b = field.advanced(by: i * cap)
                out.append(String(cString: b.assumingMemoryBound(to: CChar.self)))
            }
            return out
        }
        var colChanges: [String] = []
        var colLog: [String] = []
        var colFiles: [String] = []
        var special: [String] = []
        withUnsafeBytes(of: snap.col_changes) { raw in
            colChanges = loadCol(snap.changes_count, raw.baseAddress!)
        }
        withUnsafeBytes(of: snap.col_log) { raw in
            colLog = loadCol(snap.log_count, raw.baseAddress!)
        }
        withUnsafeBytes(of: snap.col_files) { raw in
            colFiles = loadCol(snap.files_count, raw.baseAddress!)
        }
        withUnsafeBytes(of: snap.special) { raw in
            special = loadCol(snap.special_count, raw.baseAddress!)
        }
        return GitWbSnap(
            open: true,
            docked: snap.docked != 0,
            loading: snap.loading != 0,
            tabIndex: Int(snap.tab_index),
            branch: cStringField(snap.branch),
            message: cStringField(snap.message),
            chips: chips,
            colChanges: colChanges,
            colLog: colLog,
            colFiles: colFiles,
            special: special
        )
    }

    private func loadSettings(_ engine: OpaquePointer) -> SettingsSnap {
        var snap = SuiseiSettingsSnapshot()
        guard suisei_engine_settings(engine, &snap) != 0 else { return .empty }
        var tabs: [String] = []
        let tn = Int(snap.tab_count)
        withUnsafeBytes(of: snap.tabs) { raw in
            for i in 0..<min(tn, Int(SUISEI_MAX_SETTINGS_TABS)) {
                let base = raw.baseAddress!.advanced(by: i * 24)
                tabs.append(String(cString: base.assumingMemoryBound(to: CChar.self)))
            }
        }
        var rows: [SettingsRowItem] = []
        let rn = Int(snap.row_count)
        withUnsafeBytes(of: snap.row_labels) { labRaw in
            withUnsafeBytes(of: snap.row_values) { valRaw in
                withUnsafeBytes(of: snap.row_header) { hdrRaw in
                    withUnsafeBytes(of: snap.row_selected) { selRaw in
                        let lCap = Int(SUISEI_SETTINGS_LABEL)
                        let vCap = Int(SUISEI_SETTINGS_VALUE)
                        for i in 0..<min(rn, Int(SUISEI_MAX_SETTINGS_ROWS)) {
                            let lb = labRaw.baseAddress!.advanced(by: i * lCap)
                            let vb = valRaw.baseAddress!.advanced(by: i * vCap)
                            rows.append(SettingsRowItem(
                                id: i,
                                label: String(cString: lb.assumingMemoryBound(to: CChar.self)),
                                value: String(cString: vb.assumingMemoryBound(to: CChar.self)),
                                isHeader: hdrRaw[i] != 0,
                                selected: selRaw[i] != 0
                            ))
                        }
                    }
                }
            }
        }
        return SettingsSnap(
            open: snap.open != 0,
            dirty: snap.dirty != 0,
            pageIndex: Int(snap.page_index),
            selected: Int(snap.selected),
            status: cStringField(snap.status),
            tabs: tabs,
            rows: rows
        )
    }

    private func loadTheme(_ engine: OpaquePointer) -> ThemeSnap {
        var snap = SuiseiThemeSnapshot()
        guard suisei_engine_theme(engine, &snap) != 0 else { return .empty }
        return ThemeSnap(
            name: cStringField(snap.name),
            editorBg: snap.editor_bg,
            fg: snap.fg,
            dim: snap.dim,
            accent: snap.accent,
            selection: snap.selection,
            caret: snap.caret,
            statusBg: snap.status_bg,
            keyword: snap.keyword,
            string: snap.string_col,
            comment: snap.comment,
            number: snap.number,
            typeName: snap.type_name,
            function: snap.function,
            macroName: snap.macro_name,
            namespace: snap.namespace,
            parameter: snap.parameter,
            property: snap.property,
            constant: snap.constant,
            operatorColor: snap.operator,
            punctuation: snap.punctuation
        )
    }

    private func loadOutline(_ engine: OpaquePointer) -> [OutlineItem] {
        var snap = SuiseiOutlineSnapshot()
        guard suisei_engine_outline(engine, &snap) != 0 else { return [] }
        var items: [OutlineItem] = []
        let n = Int(snap.count)
        // Fixed C arrays import as tuples — use raw bytes for indexing.
        withUnsafeBytes(of: snap.names) { namesRaw in
            withUnsafeBytes(of: snap.rows) { rowsRaw in
                withUnsafeBytes(of: snap.kinds) { kindsRaw in
                    withUnsafeBytes(of: snap.depths) { depthsRaw in
                        let cap = Int(SUISEI_OUTLINE_NAME)
                        let rowPtr = rowsRaw.bindMemory(to: UInt32.self)
                        let kindPtr = kindsRaw.bindMemory(to: UInt8.self)
                        let depthPtr = depthsRaw.bindMemory(to: UInt8.self)
                        for i in 0..<min(n, Int(SUISEI_MAX_OUTLINE)) {
                            let base = namesRaw.baseAddress!.advanced(by: i * cap)
                            let name = String(cString: base.assumingMemoryBound(to: CChar.self))
                            items.append(OutlineItem(
                                id: i,
                                name: name,
                                row: rowPtr[i],
                                kind: kindPtr[i],
                                depth: depthPtr[i]
                            ))
                        }
                    }
                }
            }
        }
        return items
    }

    private func loadPreview(_ engine: OpaquePointer) -> PreviewSnap {
        // Page through chunks so long documents aren't truncated at ABI max.
        var lines: [PreviewLineItem] = []
        var kind: UInt8 = 0
        var scroll: UInt32 = 0
        var hscroll: UInt32 = 0
        var start: UInt32 = 0
        var open = false
        let lineCap = Int(SUISEI_PREVIEW_LINE)
        let maxPages = 80 // 80 × 128 = 10_240 lines safety cap

        for _ in 0..<maxPages {
            var snap = SuiseiPreviewSnapshot()
            guard suisei_engine_preview(engine, start, &snap) != 0 else { break }
            if snap.open == 0 {
                return .empty
            }
            open = true
            kind = snap.kind
            scroll = snap.scroll
            hscroll = snap.hscroll
            let n = min(Int(snap.count), Int(SUISEI_MAX_PREVIEW))
            if n == 0 { break }
            withUnsafeBytes(of: snap.lines) { linesRaw in
                withUnsafeBytes(of: snap.styles) { stylesRaw in
                    let stylePtr = stylesRaw.bindMemory(to: UInt8.self)
                    for i in 0..<n {
                        let base = linesRaw.baseAddress!.advanced(by: i * lineCap)
                        let text = String(cString: base.assumingMemoryBound(to: CChar.self))
                        lines.append(PreviewLineItem(id: lines.count, text: text, style: stylePtr[i]))
                    }
                }
            }
            let next = start + UInt32(n)
            if next >= snap.total || n < Int(SUISEI_MAX_PREVIEW) { break }
            start = next
        }
        guard open else { return .empty }
        return PreviewSnap(
            open: true,
            kind: kind,
            scroll: scroll,
            hscroll: hscroll,
            lines: lines
        )
    }

    private func loadBreakpoints(_ engine: OpaquePointer) -> [BreakpointItem] {
        var snap = SuiseiBreakpointSnapshot()
        guard suisei_engine_breakpoints(engine, &snap) != 0 else { return [] }
        var items: [BreakpointItem] = []
        let n = min(Int(snap.count), Int(SUISEI_MAX_BREAKPOINTS))
        withUnsafeBytes(of: snap.paths) { pathsRaw in
            withUnsafeBytes(of: snap.names) { namesRaw in
                withUnsafeBytes(of: snap.conditions) { condRaw in
                    withUnsafeBytes(of: snap.lines) { linesRaw in
                        withUnsafeBytes(of: snap.verified) { verRaw in
                            withUnsafeBytes(of: snap.has_condition) { hcRaw in
                                withUnsafeBytes(of: snap.has_log) { hlRaw in
                                    let pathCap = Int(SUISEI_PATH_CAP)
                                    let nameCap = Int(SUISEI_BP_NAME)
                                    let condCap = 96
                                    let linePtr = linesRaw.bindMemory(to: UInt32.self)
                                    let verPtr = verRaw.bindMemory(to: UInt8.self)
                                    let hcPtr = hcRaw.bindMemory(to: UInt8.self)
                                    let hlPtr = hlRaw.bindMemory(to: UInt8.self)
                                    for i in 0..<n {
                                        let path = String(
                                            cString: pathsRaw.baseAddress!
                                                .advanced(by: i * pathCap)
                                                .assumingMemoryBound(to: CChar.self)
                                        )
                                        let name = String(
                                            cString: namesRaw.baseAddress!
                                                .advanced(by: i * nameCap)
                                                .assumingMemoryBound(to: CChar.self)
                                        )
                                        let cond = String(
                                            cString: condRaw.baseAddress!
                                                .advanced(by: i * condCap)
                                                .assumingMemoryBound(to: CChar.self)
                                        )
                                        items.append(BreakpointItem(
                                            path: path,
                                            name: name.isEmpty ? (path as NSString).lastPathComponent : name,
                                            line: linePtr[i],
                                            verified: verPtr[i] != 0,
                                            condition: hcPtr[i] != 0 ? cond : "",
                                            hasLog: hlPtr[i] != 0
                                        ))
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        return items
    }

    private func loadExplorer(_ engine: OpaquePointer) -> ExplorerSnap {
        var snap = SuiseiExplorerSnapshot()
        guard suisei_engine_explorer(engine, &snap) != 0 else { return .empty }
        var entries: [ExplorerEntry] = []
        let n = Int(snap.count)
        withUnsafeBytes(of: snap.names) { namesRaw in
            withUnsafeBytes(of: snap.is_dir) { dirRaw in
                let nameCap = Int(SUISEI_EXPLORER_NAME)
                for i in 0..<min(n, Int(SUISEI_MAX_EXPLORER)) {
                    let base = namesRaw.baseAddress!.advanced(by: i * nameCap)
                    let name = String(cString: base.assumingMemoryBound(to: CChar.self))
                    entries.append(ExplorerEntry(
                        id: i,
                        name: name,
                        isDir: dirRaw[i] != 0,
                        selected: i == Int(snap.selected)
                    ))
                }
            }
        }
        return ExplorerSnap(
            open: snap.open != 0,
            cwd: cStringField(snap.cwd),
            selected: Int(snap.selected),
            entries: entries
        )
    }

    private func loadPalette(_ engine: OpaquePointer) -> PaletteSnap {
        var snap = SuiseiPaletteSnapshot()
        guard suisei_engine_palette(engine, &snap) != 0 else { return .empty }
        var items: [PaletteItem] = []
        let n = Int(snap.count)
        withUnsafeBytes(of: snap.labels) { labRaw in
            withUnsafeBytes(of: snap.details) { detRaw in
                let lCap = Int(SUISEI_PALETTE_LABEL)
                let dCap = Int(SUISEI_PALETTE_DETAIL)
                for i in 0..<min(n, Int(SUISEI_MAX_PALETTE)) {
                    let lb = labRaw.baseAddress!.advanced(by: i * lCap)
                    let db = detRaw.baseAddress!.advanced(by: i * dCap)
                    items.append(PaletteItem(
                        id: i,
                        label: String(cString: lb.assumingMemoryBound(to: CChar.self)),
                        detail: String(cString: db.assumingMemoryBound(to: CChar.self)),
                        selected: i == Int(snap.selected)
                    ))
                }
            }
        }
        return PaletteSnap(
            open: snap.open != 0,
            kind: cStringField(snap.kind),
            query: cStringField(snap.query),
            items: items
        )
    }

    private func loadSearch(_ engine: OpaquePointer) -> SearchSnap {
        var snap = SuiseiSearchSnapshot()
        guard suisei_engine_search(engine, &snap) != 0 else { return .empty }
        return SearchSnap(
            open: snap.open != 0,
            forward: snap.forward != 0,
            input: cStringField(snap.input),
            matchCount: snap.match_count,
            matchIndex: snap.match_index
        )
    }

    private func loadCompletions(_ engine: OpaquePointer) -> CompletionsSnap {
        var snap = SuiseiCompletionsSnapshot()
        guard suisei_engine_completions(engine, &snap) != 0 else { return .empty }
        var items: [CompRow] = []
        let n = Int(snap.count)
        withUnsafeBytes(of: snap.labels) { lRaw in
            withUnsafeBytes(of: snap.details) { dRaw in
                let cap = Int(SUISEI_COMP_LABEL)
                for i in 0..<min(n, Int(SUISEI_MAX_COMP)) {
                    let lb = lRaw.baseAddress!.advanced(by: i * cap)
                    let db = dRaw.baseAddress!.advanced(by: i * cap)
                    items.append(CompRow(
                        label: String(cString: lb.assumingMemoryBound(to: CChar.self)),
                        detail: String(cString: db.assumingMemoryBound(to: CChar.self))
                    ))
                }
            }
        }
        return CompletionsSnap(
            open: snap.open != 0,
            prefix: cStringField(snap.prefix),
            selected: Int(snap.selected),
            items: items
        )
    }

    private func loadTerminal(_ engine: OpaquePointer) -> TerminalSnap {
        var snap = SuiseiTerminalSnapshot()
        guard suisei_engine_terminal(engine, &snap) != 0 else { return .empty }
        var lines: [String] = []
        let n = Int(snap.count)
        withUnsafeBytes(of: snap.lines) { raw in
            let cap = Int(SUISEI_TERM_LINE)
            for i in 0..<min(n, Int(SUISEI_MAX_TERM_LINES)) {
                let b = raw.baseAddress!.advanced(by: i * cap)
                lines.append(String(cString: b.assumingMemoryBound(to: CChar.self)))
            }
        }
        let pane: Int? = snap.pane_bound == UInt32.max ? nil : Int(snap.pane_bound)
        return TerminalSnap(
            open: snap.open != 0,
            fullPanel: snap.full_panel != 0,
            paneBound: pane,
            lines: lines,
            cursorRow: Int(snap.cursor_row),
            cursorCol: Int(snap.cursor_col)
        )
    }
}

private func cStringField<T>(_ field: T) -> String {
    withUnsafeBytes(of: field) { raw in
        guard let base = raw.baseAddress else { return "" }
        return String(cString: base.assumingMemoryBound(to: CChar.self))
    }
}

private func readCString(at base: UnsafeRawPointer, offset: Int, cap: Int) -> String {
    let p = base.advanced(by: offset).assumingMemoryBound(to: CChar.self)
    var bytes: [CChar] = []
    for i in 0..<cap {
        let b = p[i]
        if b == 0 { break }
        bytes.append(b)
    }
    bytes.append(0)
    return String(cString: bytes)
}
