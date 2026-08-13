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
    /// Kind alone: 0 none, 1 added, 2 modified, 3 deleted.
    ///
    /// `& 0x03`, not `& 0x3F`. The wider mask let the hunk flags through into
    /// the value every `switch` compares against, so a staged or capped row
    /// matched no case at all. See `GIT_*` in `compositor/scene.rs` for the
    /// whole byte.
    var gitSignKind: UInt8 { gitSign & 0x03 }
    /// The hunk this row belongs to is staged — draw the bar filled.
    var gitHunkStaged: Bool { (gitSign & 0x08) != 0 }
    /// This row is the first / last of its hunk, so the bar caps here.
    var gitHunkFirst: Bool { (gitSign & 0x10) != 0 }
    var gitHunkLast: Bool { (gitSign & 0x20) != 0 }
}

/// One editor split surface (or the single full editor when unsplit).
struct EditorPaneSnap: Equatable, Identifiable {
    var id: Int
    var focused: Bool
    var tabIndex: Int
    /// Actual document title supplied per pane. A visible unified layout chip
    /// is not a reliable lookup key for the buffers inside that layout.
    var title: String = "[No Name]"
    var scroll: UInt32
    var hscroll: UInt32
    /// Total lines in this pane's buffer (scrollbar / clamp).
    var docLineCount: UInt32
    var lines: [EditorLine]
    /// Normalised rect within the editor area, straight from core's layout
    /// tree. The face used to re-derive geometry from `kind` + `ratio`, which
    /// can only describe two panes — three asked for 150% of the width and the
    /// first pane got clipped off-screen.
    var rect: CGRect = CGRect(x: 0, y: 0, width: 1, height: 1)
    /// This pane runs its own shell.
    var isTerminal: Bool = false
    /// That shell's content generation — bumped when its grid changes, so the
    /// face skips re-pulling a ~300 KiB snapshot it already has.
    var termGen: UInt16 = 0
    /// That shell's rows and caret. Pulled per pane: terminal panes are
    /// separate processes, and one shared snapshot made them all show the same
    /// session.
    var termLines: [String] = []
    var termCursorRow: Int = 0
    var termCursorCol: Int = 0
}

/// Split state for the editor island. The shape lives entirely in the
/// per-pane rects; `isSplit` is just "more than one pane". (The old
/// `kind`/`ratio` pair could only describe two panes and was retired with
/// the layout tree — the ABI bytes survive as pads.)
struct SplitSnap: Equatable {
    var focus: Int
    var panes: [EditorPaneSnap]
    var isSplit: Bool { panes.count >= 2 }
    static let empty = SplitSnap(focus: 0, panes: [])
}

struct TabItem: Equatable, Identifiable {
    /// Slot index — what every engine call takes.
    var id: Int
    /// `BufferTab::id`. Stable across a reorder, so it is what the tab strip
    /// uses as list identity: with the slot index there, dragging a tab left
    /// the identity list unchanged and only the titles swapped in place, and
    /// SwiftUI had nothing to animate.
    var stableId: UInt64
    var title: String
    var dirty: Bool
    var active: Bool
    /// The folded layout this chip belongs to, or 0.
    ///
    /// Consecutive chips sharing a non-zero value are one layout drawn in the
    /// **grouped** shape — the documents keep their chips and the strip draws
    /// one rounded container around the run. A **unified** layout is instead a
    /// single chip with `isLayout` set.
    var group: UInt64 = 0
    var isLayout: Bool = false
    /// This tab is a terminal — a shell runs in it.
    var isTerminal: Bool = false
    /// The document's file was deleted on disk — the chip flags it (a Save
    /// re-creates the file) so editing a vanished path is never silent.
    var deleted: Bool = false
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

/// The DOCKED terminal (⌃T). Pane terminals are separate processes pulled
/// per pane by `attachPaneTerminals` — the old `fullPanel`/`paneBound` flags
/// were the single-shared-terminal model and never carried truth for panes
/// (the dock snapshot hardcodes them off), so every branch reading them was
/// dead. Removed 2026-07-29.
struct TerminalSnap: Equatable {
    var open: Bool
    var lines: [String]
    /// Shell cursor within `lines`. Never crossed the bridge before, which is
    /// why the terminal had no caret at all.
    var cursorRow: Int = 0
    var cursorCol: Int = 0
    static let empty = TerminalSnap(open: false, lines: [])
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

struct GitWorktreeItem: Equatable, Identifiable {
    var id: Int
    var path: String
    var status: String
    var staged: Bool
    var selected: Bool
}

struct GitHistoryItem: Equatable, Identifiable {
    var id: Int
    var hash: String
    var shortHash: String
    var subject: String
    var author: String
    var when: String
    var selected: Bool
}

struct GitBranchItem: Equatable, Identifiable {
    var id: Int
    var name: String
    var upstream: String
    var current: Bool
    var remote: Bool
    var selected: Bool
}

struct GitCommitFileItem: Equatable, Identifiable {
    var id: Int
    var path: String
    var status: String
    var insertions: Int
    var deletions: Int
    var selected: Bool
}

struct GitCommitDetailSnap: Equatable {
    var hash: String
    var shortHash: String
    var subject: String
    var author: String
    var email: String
    var date: String
    var body: String
    var insertions: Int
    var deletions: Int
}

struct GitRemoteItem: Equatable, Identifiable {
    var id: Int
    var name: String
    var url: String
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
    var rootPath: String
    var repositoryName: String
    var authorName: String
    var authorEmail: String
    var worktree: [GitWorktreeItem]
    var history: [GitHistoryItem]
    var branches: [GitBranchItem]
    var commitFiles: [GitCommitFileItem]
    var commitDetail: GitCommitDetailSnap?
    var stashes: [String]
    var remotes: [GitRemoteItem]
    static let empty = GitWbSnap(
        open: false, docked: false, loading: false, tabIndex: 0,
        branch: "", message: "", chips: [],
        colChanges: [], colLog: [], colFiles: [], special: [],
        rootPath: "", repositoryName: "", authorName: "", authorEmail: "",
        worktree: [], history: [], branches: [], commitFiles: [],
        commitDetail: nil, stashes: [], remotes: []
    )
}

/// Independent publication domain for the native Source Control window.
///
/// The editor shell and workbench used to observe the same `EngineBridge`.
/// A diff/status update therefore rebuilt both large SwiftUI trees, while an
/// unrelated editor/LSP frame copied and decoded the workbench snapshot. Keep
/// the native window on its own object so each surface invalidates only itself.
final class GitWorkbenchStore: ObservableObject {
    @Published private(set) var snapshot: GitWbSnap = .empty
    @Published private(set) var theme: ThemeSnap = .empty

    func publish(snapshot next: GitWbSnap) {
        if next != snapshot { snapshot = next }
    }

    func publish(theme next: ThemeSnap) {
        if next != theme { theme = next }
    }
}

/// What a settings row IS, carried from Core rather than guessed from its text.
///
/// Every control in Settings used to be recovered by matching the DISPLAY
/// LABEL — `label == "LSP enabled"`, `label.hasPrefix("LSP ·")`,
/// `label.contains("●")`, a hard-coded list of five toggle names. Core has had
/// this type all along (`SettingRow`); it was flattened to strings at the FFI
/// boundary and reconstructed here by pattern-matching, so renaming a label
/// silently broke a control and three rows were never listed at all.
///
/// Raw values are ABI — they match `SettingRow::kind`. Append only.
enum SettingKind: UInt32 {
    /// Prose with no setting behind it (About, Extensions, Shortcuts rows).
    case none = 0
    case themeHeader = 1
    case theme = 2
    case editorHeader = 3
    case tabWidth = 4
    case relativeNumber = 5
    case wrapLines = 6
    case undoCaching = 7
    case clipboardSync = 8
    case gpuAcc = 9
    case gpuGraphics = 10
    case gpuHyperlinks = 11
    case keyHints = 12
    case lspHeader = 13
    case lspEnabled = 14
    case lspLang = 15
    case gitHeader = 16
    case openWorkbench = 17
    case openScm = 18
    case highlightColor = 19
    case updateCheck = 20
    case appearanceMode = 21
    case glassStyle = 22
}

/// Native layout metadata supplied by Core. Raw values are the append-only ABI
/// in `SettingSurfacePage::code` and `SettingControl::code`.
enum SettingSurfacePage: UInt32 {
    case none = 0
    case general = 1
    case appearance = 2
    case editor = 3
    case languageServers = 4
    case sourceControl = 5
}

enum SettingControlKind: UInt32 {
    case none = 0
    case toggle = 1
    case menu = 2
    case segmented = 3
    case action = 4
    case color = 5
}

struct SettingsRowItem: Equatable, Identifiable {
    var id: Int
    var label: String
    var value: String
    var isHeader: Bool
    var selected: Bool
    /// Identity from Core. `.none` for prose rows.
    var kind: SettingKind = .none
    /// Which theme / which language, for the indexed kinds.
    var payload: Int = 0
    var page: SettingSurfacePage = .none
    var group: String = ""
    var control: SettingControlKind = .none
    var detail: String = ""
    var options: [String] = []
    var valueIndex: Int = 0
    var advanced = false
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
    /// Tabs past the ABI cap (SUISEI_MAX_TABS) that couldn't be shipped —
    /// the strip shows "+N" instead of them silently vanishing.
    var tabsOverflow: Int = 0
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
    /// Width of the gutter's change bar.
    ///
    /// Wide enough to be HOLLOW: an unstaged hunk is drawn as an outline, and
    /// at the old 3pt the two 1.5pt edges met and the bar read as a thin solid
    /// line whatever its state.
    static let gitStripeWidth: CGFloat = 6

    /// Live face font size — mutated by zoom; not a `let` so resize can recompute.
    static var fontSize: CGFloat = {
        let v = UserDefaults.standard.double(forKey: "suisei.fontSize")
        if v >= minFontSize && v <= maxFontSize { return CGFloat(v) }
        return defaultFontSize
    }()

    /// Digits the widest line number in view needs.
    ///
    /// The gutter used to be sized for a flat 3.4 digits and then CAPPED at
    /// 44pt, so a four-digit file had nowhere to put its numbers: the drawing
    /// clamped them to x=4, on top of the change bar, and the breakpoint chip
    /// — sized from the same too-small span — cut the third digit off. Both
    /// reported bugs are that one constant.
    static var lineNumberDigits: Int = 3 {
        didSet { lineNumberDigits = max(3, min(9, lineNumberDigits)) }
    }

    static var gutter: CGFloat {
        // Digits strip stays compact; air gap is gutterTextGap before code (not
        // a wide slab). The 0.4 is the half-digit of air the numbers sit in.
        //
        // NO upper cap. One existed and it is what clipped four-digit files;
        // the width is derived from what has to fit, so a ceiling on it can
        // only ever mean "and then do not fit".
        let digitsW =
            cellWidth * (CGFloat(lineNumberDigits) + 0.4) + gitStripeWidth + 6
        let total = digitsW + gutterTextGap
        let scaled = (gutterBase + 4) * (fontSize / defaultFontSize)
        return max(32, max(total, scaled))
    }

    /// How many digits `count` needs.
    static func digits(for count: UInt32) -> Int {
        var n = max(1, count)
        var d = 0
        while n > 0 {
            d += 1
            n /= 10
        }
        return d
    }

    /// The editor's monospaced font, cached, and never nil.
    ///
    /// **JetBrains Mono**, bundled in `Resources/Fonts` (OFL). It falls back to
    /// the system monospaced face (SF Mono) whenever the bundled font has not
    /// registered yet — `ATSApplicationFontsPath` covers packaged launches and
    /// `WelcomeFonts.registerIfNeeded()` covers direct-binary dev runs, but the
    /// first paint can still beat either, so the fallback keeps the grid honest
    /// until it lands.
    ///
    /// `NSFont.monospacedSystemFont(ofSize:weight:)` is imported into Swift as
    /// **non-optional**, but the AppKit call behind it can return nil under
    /// font-server pressure. Swift carries that nil along and it only detonates
    /// much later, when CoreText copies it into an attributes dictionary —
    /// `NSInvalidArgumentException … attempt to insert nil object from
    /// objects[0]`, the stack pointing at whoever was drawing rather than at
    /// the font. Caching also removes the reason it was ever likely: this used
    /// to run on every access, and `cellWidth`/`gutter` read it several times
    /// per draw.
    static func monospaced(_ size: CGFloat, weight: NSFont.Weight) -> NSFont {
        let key = FontKey(size: size, weight: weight.rawValue)
        if let hit = fontCache[key] { return hit }
        WelcomeFonts.registerIfNeeded()
        var font = NSFont(name: jetBrainsMonoName(weight), size: size)
            ?? NSFont.monospacedSystemFont(ofSize: size, weight: weight)
        if unsafeBitCast(font, to: UnsafeRawPointer?.self) == nil {
            font = NSFont.userFixedPitchFont(ofSize: size) ?? NSFont.systemFont(ofSize: size)
        }
        fontCache[key] = font
        return font
    }

    /// PostScript name of the nearest bundled JetBrains Mono weight. Only three
    /// weights ship (Regular/Medium/Bold); anything semibold-or-heavier maps to
    /// Bold, medium maps to Medium, the rest to Regular. All weights share one
    /// advance width, so `cellWidth` is stable across them.
    private static func jetBrainsMonoName(_ weight: NSFont.Weight) -> String {
        if weight.rawValue >= NSFont.Weight.semibold.rawValue { return "JetBrainsMono-Bold" }
        if weight.rawValue >= NSFont.Weight.medium.rawValue { return "JetBrainsMono-Medium" }
        return "JetBrainsMono-Regular"
    }

    private struct FontKey: Hashable {
        let size: CGFloat
        let weight: CGFloat
    }
    private static var fontCache: [FontKey: NSFont] = [:]
    private static var cellWidthCache: [CGFloat: CGFloat] = [:]

    /// Width of one monospaced cell. Measured once per font size — this was a
    /// computed property that built a font and ran `sizeWithAttributes` on
    /// every read, and `gutter` reads it, and `draw` reads both.
    static var cellWidth: CGFloat {
        if let hit = cellWidthCache[fontSize] { return hit }
        let font = monospaced(fontSize, weight: .medium)
        let w = max(7, ceil(("M" as NSString).size(withAttributes: [.font: font]).width))
        cellWidthCache[fontSize] = w
        return w
    }

    static var lineHeight: CGFloat { fontSize + linePad * 2 }

    @discardableResult
    static func adjustFont(delta: CGFloat) -> CGFloat {
        fontSize = min(maxFontSize, max(minFontSize, fontSize + delta))
        // Keyed by size, so nothing is stale — but a zoom run would otherwise
        // grow the caches one entry per step.
        cellWidthCache.removeAll(keepingCapacity: true)
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
    let gitWorkbenchStore = GitWorkbenchStore()
    /// Lightweight bridge used only to present/dismiss the independent Window
    /// scene. Workbench rows themselves live in `gitWorkbenchStore`.
    @Published private(set) var gitWorkbenchWindowOpen = false
    /// Per-keystroke caret/scroll, on a SEPARATE object so publishing it does
    /// not re-evaluate the whole ContentView tree (the split container reads
    /// only structural `editorSplit`; each pane's canvas observes THIS). See
    /// `EditorTickStore`.
    let editorTick = EditorTickStore()
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
    /// True while the tab strip is changing its structure (close/reorder or a
    /// layout presentation step). The strip uses this to suppress its normal
    /// active-tab auto-centering: scrolling the viewport while chips are also
    /// being inserted, removed, or reordered creates a second coordinate
    /// system and is the source of the visible two-stage lurch.
    @Published private(set) var tabStructuralMotionActive: Bool = false
    /// Current structural verb, read by the strip to keep layout labels out of
    /// the parent HStack transaction during grouped ⇄ unified replacement.
    private(set) var tabStructuralKind: String = ""
    private var tabStructuralMotionToken: UInt64 = 0

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
    /// The full diff is intentionally outside the fixed chrome snapshot. Keep
    /// one decoded copy and refresh it only when Core's generation changes.
    private var gitDiffGeneration = UInt64.max
    private var gitDiffLinesCache: [String] = []
    /// Independent Core generation for the structured workbench model.
    private var gitWbGeneration = UInt64.max
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
    /// Run a docked-panel show/hide with the panel motion, from **any** entry
    /// point.
    ///
    /// This lived privately in `ContentView`, so the top bar's toggles glided
    /// and the menu commands (⌘0, ⌥⌘0) snapped — the same panel behaved
    /// differently depending on how you asked for it. On the bridge, every
    /// caller gets the motion, including the next one added.
    ///
    /// `windowLiveResizing` is held across the animation because the editor is
    /// an `NSViewRepresentable`: without it, each of the ~18 animation frames
    /// pushes a resize into the engine and recomposes.
    func animatingPanels(_ body: () -> Void) {
        PerfProbe.record("panel toggle started", 0)
        windowLiveResizing = true
        // NO `withAnimation` here. The motion is an implicit
        // `.animation(_:value:)` on the container that holds both the panel and
        // the content that steps aside for it — the way the inspector has
        // always done it. Driving the same change from an explicit transaction
        // as well gave the navigator two animators for one value, and it
        // stuttered; the inspector, with only the implicit one, never did.
        body()
        // CANCELLABLE, and that is the whole point. Toggling again inside the
        // settle window used to let the PREVIOUS toggle's timer fire in the
        // middle of the new animation: it cleared `windowLiveResizing`, which
        // re-enabled the per-frame engine resize this flag exists to suppress,
        // and then pushed a full resize on top of it. Toggling repeatedly piled
        // the timers up — one stomp each, mid-flight. That was the stutter.
        panelSettleWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            PerfProbe.record("panel settle RAN", 0)
            self.windowLiveResizing = false
            self.settleEditorResize()
        }
        panelSettleWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.36, execute: work)
    }

    /// Pending tail of the last panel animation; only the newest may run.
    private var panelSettleWork: DispatchWorkItem?

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
        // Previous session's files + cursors, if any — landing named buffers
        // flips the welcome rule, so Welcome yields to the restored editor.
        suisei_engine_restore_session(engine)
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
        if let engine {
            // Last write wins: persist open files + cursors for next launch.
            suisei_engine_save_session(engine)
            suisei_engine_free(engine)
        }
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
        setProjectRoot(Self.workspaceRoot(containing: item.path))
        NotificationCenter.default.post(name: .suiseiRecoveryAccepted, object: nil)
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

    /// Milliseconds for the engine, clamped.
    ///
    /// A tick after the app was suspended reports an enormous gap; the engine
    /// should see a plausible frame rather than an hour. (`ContentView` has a
    /// `clamped(to:)` of its own but it is fileprivate to that file.)
    private static func clampedMs(_ v: Double) -> UInt32 {
        UInt32(min(max(v.rounded(), 1), 250))
    }

    /// Heartbeat interval.
    ///
    /// Was 50 ms — a 20 Hz update loop on a 60–120 Hz display. That is the
    /// single biggest reason the app can feel a beat behind while using about a
    /// tenth of one core: it is not slow, it is mostly asleep, and every visible
    /// change is quantised to 50 ms whatever the display can do.
    ///
    /// Raising it is affordable because the tick is gated, not because it is
    /// cheap to do more work: `suisei_engine_tick` measures under a microsecond
    /// even at 64 tabs, and the SwiftUI publish below still only runs when
    /// `frame_gen` actually advanced. An idle app at 120 Hz does 120 sub-
    /// microsecond FFI calls a second and publishes nothing.
    static let tickInterval: TimeInterval = 1.0 / 120.0

    private func startTick() {
        tickTimer?.invalidate()
        var lastTickAt = CACurrentMediaTime()
        tickTimer = Timer.scheduledTimer(
            withTimeInterval: Self.tickInterval, repeats: true
        ) { [weak self] _ in
            guard let self, let engine = self.engine else { return }
            // Real elapsed time, not the nominal interval. The engine uses this
            // to drive time-based work, and handing it a constant 50 while
            // firing at another rate would make every duration it computes
            // wrong by that ratio.
            let now = CACurrentMediaTime()
            let dt = Self.clampedMs((now - lastTickAt) * 1000)
            lastTickAt = now
            let gen = suisei_engine_tick(engine, dt)
            // Pick up an async references reply (one publish, then it stops).
            self.pollReferencesIfNeeded()
            // Source Control has its own generation and ObservableObject. This
            // cheap probe is safe at display cadence and performs no snapshot
            // copy when Git state did not change.
            self.refreshGitWorkbenchIfNeeded()
            // Never publish SwiftUI editor updates mid-gesture — the canvas
            // already merges paint windows itself; publishing re-enters
            // updateNSView while AppKit scrolls and shows as jitter.
            if self.isLiveScrolling { return }
            // Same reason, different gesture. A structural tab animation —
            // fold, unfold, reorder, close — runs 0.20–0.30 s, and this timer
            // fires 4 to 6 times inside it. Every one of those publishes
            // re-enters ContentView's body WHILE the chips are interpolating,
            // and the strip visibly hitches. `tabStructuralMotionActive` was
            // already tracked for the auto-scroll guard; it belongs here too.
            //
            // The engine keeps ticking throughout (the PTY still drains, the
            // LSP still answers) — only the SwiftUI publish waits, and the
            // catch-up refresh when motion ends picks up whatever landed.
            if self.tabStructuralMotionActive { return }
            if gen != self.lastFrameGen {
                // Terminal / LSP noise: prefer light paint; full shell every ~0.5s max via gen.
                if self.chrome.terminal.open {
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
        // Docked shell (⌃T) first: opening the dock forces Core's
        // `Mode::Terminal` and takes the keyboard even while a terminal pane
        // is focused — checking the pane first made the face insist the pane
        // owned keys the engine was already routing to the dock.
        if chrome.terminal.open, focus == .terminal {
            return true
        }
        // A focused terminal PANE owns typing even though the engine stays in
        // `Mode::Editor` — that mode belongs to the docked shell. Without this
        // the face took printable keys down the editor's insert path, so a
        // terminal pane received Enter (not printable, so it fell through to
        // the raw dispatch) and nothing else: pressing keys produced a bare
        // new prompt and the characters went into the document.
        if editorSplit.panes.indices.contains(editorSplit.focus),
           editorSplit.panes[editorSplit.focus].isTerminal
        {
            return true
        }
        return false
    }

    func dispatch(code: SuiseiKey, ch: UInt32 = 0, fNum: UInt8 = 0, mods: SuiseiMod = []) {
        // Debug-strip terminal promises "Esc to leave" — release focus instead
        // of feeding Esc to the shell. Pane terminals keep Esc for their TUIs
        // (vim insert, less, nano): the dock's Mode::Terminal is the only case
        // where "leave" means anything. The old gate read `fullPanel` off the
        // DOCK snapshot — always false for panes — and swallowed Esc into a
        // no-op, so vim was unusable in terminal panes.
        if code == .esc, chrome.terminal.open, focus == .terminal {
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
        // Stay on the semantic GUI path. Raw key dispatch happens to type most
        // characters too, but it also carries shortcut/modifier policy and was
        // able to diverge from IME commits and programmatic insertion. One edit
        // primitive now owns selection replacement, auto-pairs and Unicode.
        suisei_engine_gui_type_char(engine, ch)
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

    /// Absolute UTF-16 caret offset for AppKit's text-input coordinate space.
    func caretUTF16Offset() -> Int {
        guard let engine else { return 0 }
        return Int(clamping: suisei_engine_caret_utf16_offset(engine))
    }

    /// Coalesced catch-up for everything typing deliberately skipped.
    private func scheduleChromeSettle() {
        chromeSettleWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            // CLEARED before the refresh, not after and not never: a completed
            // `DispatchWorkItem` still reports `isCancelled == false`, so a
            // pending-check that asked it would answer "still typing" forever
            // and the paint path would stop publishing chrome for good.
            self?.chromeSettleWork = nil
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

    /// A non-published twin of `chrome`.
    ///
    /// Two jobs, and the second is why it is not behind `SUISEI_PERF`. It was
    /// added to measure the cost of releasing the previous snapshot apart from
    /// the cost of telling SwiftUI — assigning to it is byte-for-byte the work
    /// `chrome = next` does, minus the publisher. That measurement said the
    /// publisher is the entire cost.
    ///
    /// Now it also absorbs the writes a skipped mid-burst publish would have
    /// made, so the value is not lost: the settle at the end of the burst does
    /// a full `refreshChrome`, which rebuilds from the engine regardless.
    private var chromeShadow: ChromeSnapshot = .empty

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
            scheduleChromeSettle()
            refreshEditorPaintOnly()
        } else {
            dispatchRaw(code: .char_, ch: scalar.value)
        }
    }

    /// True while an overlay / panel surface owns typed characters (find bar,
    /// palette filter, git panes, settings, read-only preview…) — never force
    /// Insert then.
    private var panelOwnsTyping: Bool { focus.ownsTyping }

    /// Take the keyboard back from a native text field.
    ///
    /// `editorOwnsKeyEvents` deliberately stands down while an `NSTextField` or
    /// `NSTextView` is the window's first responder — otherwise the project
    /// tree's filter could not be typed into. The gap is what happens when an
    /// engine-owned overlay opens *while* that field still holds focus: the
    /// palette appears on screen and is completely deaf. Its filter gets
    /// nothing, Esc gets nothing, and the keystrokes go on landing in the tree
    /// filter behind it. Measured exactly that way — typing "s1_b" with the
    /// file palette open filtered the project tree instead.
    ///
    /// So opening one has to reclaim the responder. `nil` hands it to the
    /// window, which is enough for `editorOwnsKeyEvents` to say yes.
    func reclaimKeyboardFromTextFields() {
        guard let win = NSApp.keyWindow ?? NSApp.mainWindow,
              let responder = win.firstResponder,
              !(responder is EditorCanvasView),
              responder is NSTextView || responder is NSTextField
                  || responder is NSTextInputClient
        else { return }
        win.makeFirstResponder(nil)
    }

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
                // Armed BEFORE the paint refresh, which reads it: scheduled
                // after, the first key of every burst still paid the 30ms
                // publish and only its successors skipped.
                scheduleChromeSettle()
                refreshEditorPaintOnly()
            }
            return true
        }
        if code == .delete {
            if let engine {
                suisei_engine_gui_delete_forward(engine)
                scheduleChromeSettle()
                refreshEditorPaintOnly()
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
                scheduleChromeSettle()
                refreshEditorPaintOnly()
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
        // Ask the four-byte panel mask before touching the snapshot. What is
        // called a "cheap single-chunk probe" below is a 64 KiB struct: Swift
        // zero-fills it, the FFI memsets it again, and all we wanted was three
        // fields. On the light path that ran every tick.
        guard suisei_engine_open_panels(engine) & SUISEI_PANEL_PREVIEW != 0 else {
            if preview != .empty { preview = .empty }
            lastPreviewKey = ""
            return
        }
        // Open: now the pan/extent probe is worth its bytes.
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

    /// Pretty document preview.
    ///
    /// Calls the thing it names. This used to dispatch ⇧⌘V, and that chord is
    /// "pretty preview" only while the editor holds focus — a focused terminal
    /// pane claims it as "paste the clipboard into the shell", so the menu item
    /// could paste into a running process. Same lesson as
    /// `toggleTerminalDock`'s.
    func togglePreview() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_toggle_preview(engine)
        refreshChrome()
        refreshPreview()
    }

    /// Open a full terminal TAB, or close it when one is already focused.
    ///
    /// Named for what Core does (`toggle_terminal_full` parks the pane, spawns
    /// a shell and gives it a tab). It used to dispatch ⇧⌘T, which the terminal
    /// pane handles on a different branch from the editor.
    func toggleTerminalTab() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_toggle_terminal_tab(engine)
        refreshChrome()
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
        let (lines, split) = decodeEditorLinesAndSplit(from: snap, engine: engine)
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
        let tabs = decodeTabs(from: snap)

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
        EditorMetrics.lineNumberDigits = EditorMetrics.digits(for: paint.lineCount)
        next.lines = paint.lines
        next.split = paint.split
        next.tabs = tabs
        next.tabsOverflow = max(0, Int(snap.tab_count) - tabs.count)
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
        stepTab(tabs[(cur + 1) % tabs.count])
    }

    func prevTab() {
        let tabs = chrome.tabs
        guard !tabs.isEmpty else { return }
        let cur = tabs.firstIndex(where: \.active) ?? 0
        stepTab(tabs[(cur - 1 + tabs.count) % tabs.count])
    }

    /// ⌃⇥ target: unified layout chip activates the layout; a parked layout
    /// member on a single-pane desk restores the arrangement; everything else
    /// goes through `gotoTabId` (which keeps an active layout's multi-pane
    /// desk when the target is a member, and keeps a free multi-pane split
    /// when there is no active layout).
    private func stepTab(_ target: TabItem) {
        if target.isLayout {
            activateLayout(target.group)
        } else if target.group != 0, !isLayoutDeskActive, !editorSplit.isSplit {
            activateLayout(target.group, focusDoc: target.stableId)
        } else {
            gotoTabId(target.stableId)
        }
    }

    /// Chip click routing shared by the SwiftUI action (a11y) and the AppKit
    /// titlebar overlay (real mouse). See `documentTabStrip`.
    func selectTabChip(_ tab: TabItem) {
        if tab.isLayout {
            activateLayout(tab.group)
        } else if tab.group != 0, !tab.active {
            // Parked-layout member:
            // * desk already owns this layout → focus member in-place (goto)
            // * free multi-pane split → do NOT activate (would clobber it with
            //   the parked tree and re-arm layout-leave collapse)
            // * single pane → restore the parked arrangement
            if isLayoutDeskActive || editorSplit.isSplit {
                gotoTabId(tab.stableId)
            } else {
                activateLayout(tab.group, focusDoc: tab.stableId)
            }
        } else {
            gotoTabId(tab.stableId)
        }
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

    @discardableResult
    func openPath(_ path: String) -> Bool {
        guard let engine else { return false }
        var isDir: ObjCBool = false
        FileManager.default.fileExists(atPath: path, isDirectory: &isDir)
        if isDir.boolValue {
            let result = path.withCString {
                suisei_engine_switch_project(engine, $0)
            }
            if result == 2 {
                presentFileError(
                    "This project has unsaved tabs. Save or close them before switching projects."
                )
                return false
            }
            guard result == 1 else {
                presentFileError("Could not open project: \(path)")
                return false
            }
            setProjectRoot(path)
        } else if projectRoot.isEmpty || !Self.path(path, belongsTo: projectRoot) {
            setProjectRoot(Self.workspaceRoot(containing: path))
            path.withCString { _ = suisei_engine_open_path(engine, $0) }
        } else {
            path.withCString { _ = suisei_engine_open_path(engine, $0) }
        }
        // Seed docked Project tree (does not steal Mode::Explorer).
        suisei_engine_ensure_project_tree(engine)
        RecentStore.push(path: path)
        refreshChrome()
        resolveProjectRoot()
        // Hybrid: open file → ready to type (Mac contract).
        ensureEditorFocus()
        return true
    }

    /// Recover the workspace identity when a recent item is a child file.
    /// Stopping at its immediate `src` directory shrank the Project navigator;
    /// walk upward to the nearest project marker first.
    private static func workspaceRoot(containing file: String) -> String {
        let manager = FileManager.default
        var directory = (file as NSString).deletingLastPathComponent
        let markers = [".git", "Cargo.toml", "package.json", "go.mod", "pyproject.toml"]
        while !directory.isEmpty, directory != "/" {
            if markers.contains(where: {
                manager.fileExists(atPath: (directory as NSString).appendingPathComponent($0))
            }) {
                return directory
            }
            let parent = (directory as NSString).deletingLastPathComponent
            if parent == directory { break }
            directory = parent
        }
        return (file as NSString).deletingLastPathComponent
    }

    private static func path(_ path: String, belongsTo root: String) -> Bool {
        let normalizedPath = (path as NSString).standardizingPath
        let normalizedRoot = (root as NSString).standardizingPath
        return normalizedPath == normalizedRoot
            || normalizedPath.hasPrefix(normalizedRoot + "/")
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
                guard self?.openPath(url.path) == true else { return }
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

    /// Boot pipeline: warm every language grammar on the syntax worker so the
    /// first file opened highlights with no cold parser/query build.
    /// Non-blocking — the worker warms while the launch splash is up.
    func warmGrammars() {
        guard let engine else { return }
        suisei_engine_warm_grammars(engine)
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

    func scmSelect(_ row: Int) {
        guard let engine else { return }
        suisei_engine_scm_select(engine, UInt32(row))
        refreshChrome()
    }

    func scmActivate(_ row: Int) {
        guard let engine else { return }
        suisei_engine_scm_activate(engine, UInt32(row))
        refreshChrome()
    }

    func scmToggleStage(_ row: Int) {
        guard let engine else { return }
        suisei_engine_scm_toggle_stage(engine, UInt32(row))
        refreshChrome()
    }

    func toggleGitWorkbench() {
        cancelPointerSession()
        var m = SuiseiMod.control
        m.insert(.shift)
        dispatch(code: .char_, ch: UInt32(UnicodeScalar("g").value), mods: m)
        refreshGitWorkbenchIfNeeded()
    }

    func openGitWorkbenchWindow() {
        guard let engine else { return }
        cancelPointerSession()
        if gitWorkbenchStore.snapshot.open {
            suisei_engine_git_wb_focus_window(engine)
        } else {
            suisei_engine_git_wb_open_window(engine)
        }
        refreshGitWorkbenchIfNeeded()
    }

    func focusGitWorkbenchWindow() {
        guard let engine else { return }
        suisei_engine_git_wb_focus_window(engine)
        refreshGitWorkbenchIfNeeded()
    }

    func closeGitWorkbenchWindow() {
        guard let engine else { return }
        suisei_engine_git_wb_close_window(engine)
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbSetTab(_ index: Int) {
        guard let engine else { return }
        suisei_engine_git_wb_set_tab(engine, UInt32(index))
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbSelectChange(_ row: Int) {
        guard let engine else { return }
        suisei_engine_git_wb_select_change(engine, UInt32(row))
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbSelectHistory(_ row: Int) {
        guard let engine else { return }
        suisei_engine_git_wb_select_history(engine, UInt32(row))
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbSelectCommitFile(_ row: Int) {
        guard let engine else { return }
        suisei_engine_git_wb_select_commit_file(engine, UInt32(row))
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbSelectSpecial(_ row: Int) {
        guard let engine else { return }
        suisei_engine_git_wb_select_special(engine, UInt32(row))
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbSelectBranchHistory(_ row: Int) {
        guard let engine else { return }
        suisei_engine_git_wb_select_branch_history(engine, UInt32(row))
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbRefreshWindow() {
        guard let engine else { return }
        suisei_engine_git_wb_refresh_window(engine)
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbToggleStage(_ row: Int) {
        guard let engine else { return }
        suisei_engine_git_wb_toggle_stage(engine, UInt32(row))
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbStageAll() {
        guard let engine else { return }
        suisei_engine_git_wb_stage_all(engine)
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbUnstageAll() {
        guard let engine else { return }
        suisei_engine_git_wb_unstage_all(engine)
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbCommit(message: String, amend: Bool) {
        guard let engine else { return }
        message.withCString {
            suisei_engine_git_wb_commit(engine, $0, amend ? 1 : 0)
        }
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbStash() {
        guard let engine else { return }
        suisei_engine_git_wb_stash(engine)
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbDiscardChange(_ row: Int) {
        guard let engine else { return }
        suisei_engine_git_wb_discard_change(engine, UInt32(row))
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbCheckoutSelectedBranch() {
        guard let engine else { return }
        suisei_engine_git_wb_checkout_selected_branch(engine)
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbCreateBranch(_ name: String) {
        guard let engine else { return }
        name.withCString { suisei_engine_git_wb_create_branch(engine, $0) }
        refreshGitWorkbenchIfNeeded()
    }

    func gitWbDeleteSelectedBranch() {
        guard let engine else { return }
        suisei_engine_git_wb_delete_selected_branch(engine)
        refreshGitWorkbenchIfNeeded()
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
        commitPendingEditorComposition()
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
        commitPendingEditorComposition()
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

    /// Save is initiated by the menu/local key monitor, before AppKit naturally
    /// ends the current IME composition. Commit what the focused canvas is
    /// visibly showing so Core and the file snapshot cannot lose the last
    /// marked syllable.
    private func commitPendingEditorComposition() {
        guard let canvas = NSApp.keyWindow?.firstResponder as? EditorCanvasView else {
            return
        }
        _ = canvas.commitMarkedTextForDocumentAction()
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

    /// App menu commands still fire while a SwiftUI/AppKit text field owns the
    /// first responder. Routing those actions unconditionally to Core made
    /// ⌘A select the document behind the Project Filter and made Cut/Copy/
    /// Paste/Undo equally unsafe. Preserve the native responder chain whenever
    /// a real text control is focused; use Core only for the editor canvas.
    func cutCommand() {
        if sendNativeTextAction(#selector(NSText.cut(_:))) { return }
        dispatch(
            code: .char_,
            ch: UInt32(UnicodeScalar("x").value),
            mods: .superKey
        )
    }

    func copyCommand() {
        if sendNativeTextAction(#selector(NSText.copy(_:))) { return }
        dispatch(
            code: .char_,
            ch: UInt32(UnicodeScalar("c").value),
            mods: .superKey
        )
    }

    func pasteCommand() {
        if sendNativeTextAction(#selector(NSText.paste(_:))) { return }
        dispatch(
            code: .char_,
            ch: UInt32(UnicodeScalar("v").value),
            mods: .superKey
        )
    }

    func selectAllCommand() {
        if sendNativeTextAction(#selector(NSText.selectAll(_:))) { return }
        selectAll()
    }

    func undoCommand() {
        if nativeTextControlHasFocus {
            NSApp.keyWindow?.undoManager?.undo()
            return
        }
        undo()
    }

    func redoCommand() {
        if nativeTextControlHasFocus {
            NSApp.keyWindow?.undoManager?.redo()
            return
        }
        redo()
    }

    @discardableResult
    private func sendNativeTextAction(_ action: Selector) -> Bool {
        guard nativeTextControlHasFocus else { return false }
        return NSApp.sendAction(action, to: nil, from: nil)
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

    func setFindInput(_ input: String) {
        guard let engine, input != chrome.search.input else { return }
        input.withCString { suisei_engine_find_set_input(engine, $0) }
        refreshEditorPaintOnly()
    }

    func setPaletteQuery(_ query: String) {
        guard let engine, query != chrome.palette.query else { return }
        query.withCString { suisei_engine_palette_set_query(engine, $0) }
        refreshEditorPaintOnly()
    }

    /// Close the find bar keeping the caret at the current match.
    func closeFind() {
        guard let engine, chrome.search.open else { return }
        // Commit BEFORE releasing the native field's first-responder. The old
        // order let the editor cancel Search during SwiftUI focus propagation,
        // then delivered this Return to the document as a newline.
        suisei_engine_find_accept(engine)
        refreshChrome()
        NSApp.keyWindow?.makeFirstResponder(nil)
        ensureEditorFocus()
    }

    /// Cancel the find bar and restore its opening caret/scroll position.
    func cancelFind() {
        guard let engine, chrome.search.open else { return }
        suisei_engine_find_cancel(engine)
        refreshChrome()
        NSApp.keyWindow?.makeFirstResponder(nil)
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
    /// Stage or discard the one change covering a line.
    ///
    /// Addressed by LINE, not by hunk index — the caller is a gutter click and
    /// a line is what a click has. An index would be a second name for the
    /// same change, stale the moment the file is re-diffed.
    /// `action`: 0 stage, 1 unstage, 2 discard — the engine's own encoding, so
    /// the two sides cannot drift into different orders.
    @discardableResult
    func applyGutterHunk(line1based: UInt32, action: UInt8) -> Bool {
        guard let engine, line1based > 0 else { return false }
        let rc = suisei_engine_apply_hunk(engine, line1based, action)
        // A discard rewrites the file, so the text and not only the gutter has
        // to come back.
        refreshChrome()
        refreshEditorPaintOnly()
        return rc == 0
    }

    /// What the change on this line replaced, or nil when it replaced nothing.
    ///
    /// Sized by asking first: the engine answers the required length for a
    /// zero capacity, so the buffer is never a guess that truncates someone's
    /// deleted code.
    func removedTextForHunk(atLine line1based: UInt32) -> String? {
        guard let engine, line1based > 0 else { return nil }
        let needed = suisei_engine_hunk_removed_text(engine, line1based, nil, 0)
        guard needed > 1 else { return nil }
        var buf = [CChar](repeating: 0, count: Int(needed))
        let wrote = buf.withUnsafeMutableBufferPointer {
            suisei_engine_hunk_removed_text(
                engine, line1based, $0.baseAddress, UInt64($0.count)
            )
        }
        guard wrote == needed else { return nil }
        return String(cString: buf)
    }

    func toggleBreakpointLine(_ line1based: UInt32) {
        guard let engine, line1based > 0 else { return }
        suisei_engine_toggle_breakpoint_line(engine, line1based)
        refreshBreakpoints()
        refreshEditorPaintOnly()
    }

    /// Live split-divider drag.
    /// Drag the divider between two panes.
    ///
    /// `delta` is a fraction of the whole editor along that divider's axis.
    /// This replaced `splitSetRatio`, which addressed the single `ratio` that
    /// the entire layout shared — with three panes there are two dividers and
    /// both moved together.
    func splitResize(_ a: Int, _ b: Int, delta: Double) {
        guard let engine else { return }
        suisei_engine_split_resize(engine, UInt32(a), UInt32(b), Float(delta))
        // Chrome, not paint-only: the pane rects the layout is drawn from live
        // in the chrome snapshot, so a paint-only refresh moved the divider in
        // core and left the face drawing the old geometry.
        refreshChrome()
    }

    struct MinimapData: Equatable {
        var totalLines: Int
        var indent: [UInt8]
        var len: [UInt8]
        var flags: [UInt8]
    }

    private var minimapCacheVersion: UInt64 = .max
    private var minimapCacheTabIndex: Int = -1
    private var minimapCache: MinimapData?

    /// Downsampled document overview (cached per buffer version).
    func minimapData() -> MinimapData? {
        guard let engine else { return nil }
        let focusedTab = editorSplit.panes.indices.contains(editorSplit.focus)
            ? editorSplit.panes[editorSplit.focus].tabIndex
            : chrome.tabs.first(where: \.active)?.id ?? -1
        if chrome.bufferVersion == minimapCacheVersion,
           focusedTab == minimapCacheTabIndex,
           let cached = minimapCache {
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
        minimapCacheTabIndex = focusedTab
        minimapCache = data
        return data
    }

    /// Reorder the tab bar. Core moves the active index and every split pane
    /// with it, so this is safe while a document is open in more than one pane.
    @discardableResult
    func moveTab(from: Int, to: Int) -> Bool {
        guard let engine, from != to, from >= 0, to >= 0 else { return false }
        let moved = suisei_engine_move_tab(engine, UInt32(from), UInt32(to)) != 0
        if moved { refreshChrome() }
        return moved
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
    /// Point the keyboard at a terminal **pane**.
    ///
    /// Deliberately not `focusTerminal(true)`: that enters `Mode::Terminal`,
    /// which is the *docked* shell's mode, and it routed every keystroke to
    /// the dock instead of the pane the user clicked. A pane terminal stays in
    /// `Mode::Editor` — core sees the focused pane is a terminal and hands it
    /// the keys.
    // MARK: - Layout tabs (J7) — sequential presentation animation

    /// How long each structural step is expected to take.
    ///
    /// DURATIONS ONLY. There used to be a matching `Animation` per verb, handed
    /// to a `withAnimation` around the snapshot swap; the strip that needed
    /// those curves is AppKit now and eases itself, and the transaction only
    /// reached things that wanted no animation at all. What is left is how long
    /// to hold the auto-centre suppression and what to tell the trace recorder.
    private static let animationSlowmo = AnimationTraceRecorder.slowMotionMultiplier
    private static let tabReorderDuration = 0.24 * animationSlowmo
    private static let tabCloseDuration = 0.20 * animationSlowmo
    private static let layoutGatherDuration = 0.30 * animationSlowmo
    private static let layoutContainerDuration = 0.28 * animationSlowmo
    private static let layoutMergeDuration = 0.30 * animationSlowmo

    /// Run one structural tab-strip transaction and keep the viewport pinned
    /// until it has settled. Rapid input retargets the same motion window: an
    /// older completion may never clear a newer transition's lock.
    private func refreshChromeWithTabMotion(
        kind: String,
        duration: TimeInterval
    ) {
        tabStructuralMotionToken &+= 1
        let token = tabStructuralMotionToken
        tabStructuralKind = kind
        tabStructuralMotionActive = true
        AnimationTraceRecorder.shared.begin(kind: kind, expectedDuration: duration)
        // NO `withAnimation` here, and this is the point of the function now.
        //
        // It used to wrap the snapshot swap in an explicit transaction, for a
        // reason its own comment stated plainly: a value-scoped `.animation` on
        // the strip "does not reach the ScrollView's internal HStack relayout",
        // so without it the chips to the right of a merging group teleported
        // into the freed space. There is no ScrollView and no HStack. The strip
        // is `TabStripHostView`, which computes its own geometry and eases its
        // own origin off a display link — nothing in it reads a SwiftUI
        // transaction at all.
        //
        // What the transaction still reached was everything ELSE `refreshChrome`
        // touches, and `editorSplit` is in that set. Switching a document tab
        // for a two-pane layout tab installs the split AND removes the jump bar
        // above the editor, and the transaction turned that 26pt height change
        // into a 230ms animation on the OUTGOING full-width editor — which sat
        // on top of the freshly installed panes for the whole curve. Measured,
        // one switch:
        //
        //     43.4ms  pane 0  0×0 → 353×709       (the split arrives)
        //     44.7ms  pane 1  0×0 → 353×709
        //     89.5ms  pane 0  708×710.4 → 708×712.1
        //      …18 steps, width pinned at the full 708…
        //    322.8ms  pane 0  708×734.9 → 708×736.0
        //
        // That is the judder on the left pane. A structural change to the
        // editor's arrangement should land in one frame — `splitEditorLayout`
        // already says so with `.animation(nil, value: paneStructureKey)`, and
        // this was overruling it from outside.
        refreshChrome()
        DispatchQueue.main.asyncAfter(deadline: .now() + duration + 0.06) { [weak self] in
            guard let self, self.tabStructuralMotionToken == token else { return }
            self.tabStructuralMotionActive = false
            self.tabStructuralKind = ""
            // NO catch-up refresh here, deliberately.
            //
            // An earlier revision called `refreshChrome()` at this point to
            // flush whatever the engine changed while publishes were
            // suppressed. That was wrong three ways: it is **redundant**
            // (`lastFrameGen` is not advanced during suppression, so the very
            // next 50 ms tick sees the gap and refreshes through the normal
            // path); it is **expensive** (measured 6.1 ms mean, 20.2 ms worst
            // on a layout switch — not the ~0.3 ms the comment claimed); and
            // worst of all it is **unanimated**, so every change that accrued
            // during the motion snapped into place the instant the curve
            // ended. That is the "sometimes it just jumps with no animation"
            // the user reported.
        }
    }

    /// Fold the editor's arrangement into a layout tab. Returns false when
    /// there is nothing to fold (a single pane is not an arrangement).
    @discardableResult
    func foldLayout() -> Bool {
        guard let engine else { return false }
        let ok = suisei_engine_fold_layout(engine) != 0
        if ok {
            refreshChromeWithTabMotion(
                kind: "layout-loose-to-grouped",
                duration: Self.layoutGatherDuration
            )
        }
        return ok
    }

    /// Unfold the ACTIVE layout — the tab you are in, never the one under the
    /// pointer. A layout that detonates because the pointer was passing over
    /// it on the way somewhere else is worse than no unfold at all.
    @discardableResult
    func unfoldLayout() -> Bool {
        guard let engine else { return false }
        let ok = suisei_engine_unfold_layout(engine) != 0
        if ok {
            refreshChromeWithTabMotion(
                kind: "layout-grouped-to-loose",
                duration: Self.layoutContainerDuration
            )
        }
        return ok
    }

    func activateLayout(_ id: UInt64, focusDoc: UInt64 = 0) {
        guard let engine else { return }
        if suisei_engine_activate_layout(engine, id, focusDoc) != 0 {
            refreshChromeWithTabMotion(
                kind: "layout-activate",
                duration: Self.layoutGatherDuration
            )
        }
    }

    /// Switch a layout between the grouped and unified strip shapes.
    func toggleLayoutStyle(_ id: UInt64) {
        guard let engine else { return }
        if suisei_engine_toggle_layout_style(engine, id) != 0 {
            let wasUnified = chrome.tabs.contains { $0.group == id && $0.isLayout }
            refreshChromeWithTabMotion(
                kind: wasUnified
                    ? "layout-unified-to-grouped"
                    : "layout-grouped-to-unified",
                duration: Self.layoutMergeDuration
            )
        }
    }

    /// One deliberate upward tab-strip flick advances exactly one stage:
    /// loose split → grouped layout → unified layout.
    ///
    /// 순차적 에니메이션: 각 단계가 0.1s 간격으로 겹치며 시작된다.
    /// 일반→그룹: ① 칩들이 연속 run으로 슬라이드(gather) → 0.1s → ② 컨테이너 fade-in
    /// 그룹→통합: ③ 멤버 칩들이 수렴하며 사라짐(merge) → 0.1s → ④ 빈 공간 reclaim
    @discardableResult
    func advanceLayoutPresentation() -> Bool {
        // 그룹→통합: merge 애니메이션으로 칩들이 수렴. 0.1s 후 뒤 탭들이
        // 빈 공간을 채우며 슬라이드하는 것은 SwiftUI가 unified 칩 하나만
        // 남은 상태에서 자연스럽게 처리한다.
        if let grouped = chrome.tabs.first(where: {
            $0.active && $0.group != 0 && !$0.isLayout
        }) {
            toggleLayoutStyle(grouped.group)
            return true
        }
        // A unified layout is already the most compact stage.
        if chrome.tabs.contains(where: { $0.active && $0.isLayout }) {
            return false
        }
        // 일반→그룹: fold_layout이 buffers 순서를 재정렬(gather)하고
        // LayoutTab을 생성(style=Grouped). 칩들이 연속 run으로 슬라이드하고,
        // 0.1s 후 tabFrames가 갱신되면 컨테이너가 자연스럽게 등장한다.
        return foldLayout()
    }

    /// One deliberate downward tab-strip flick reverses exactly one stage:
    /// unified layout → grouped layout → loose split.
    ///
    /// 통합→그룹: ④⁻¹ 통합 칩이 벌어지며 멤버 칩들이 펼쳐짐 → 0.1s → ③⁻¹ 정착
    /// 그룹→일반: ②⁻¹ 컨테이너 fade-out → 0.1s → ①⁻¹ 칩들이 원래 위치로
    @discardableResult
    func retreatLayoutPresentation() -> Bool {
        if let unified = chrome.tabs.first(where: { $0.active && $0.isLayout }) {
            toggleLayoutStyle(unified.group)
            return true
        }
        if chrome.tabs.contains(where: {
            $0.active && $0.group != 0 && !$0.isLayout
        }) {
            return unfoldLayout()
        }
        return false
    }

    /// Whether a folded layout currently **owns the desk** (`App::active_layout`).
    ///
    /// Distinct from "some chip has a non-zero group": a parked layout still
    /// tags its members, but the desk is free until the layout is activated.
    var isLayoutDeskActive: Bool { activeLayoutId != 0 }

    /// WHICH layout owns the desk, or 0 for none.
    ///
    /// The id, not just the fact. "Some layout is active" is not enough to
    /// decide what a click on a grouped member means — the member may belong to
    /// a DIFFERENT layout than the one on screen, and then focusing in place
    /// leaves the arrangement it asked for uninstalled.
    var activeLayoutId: UInt64 {
        guard let engine else { return 0 }
        return suisei_engine_active_layout_id(engine)
    }

    /// Whether a folded layout is currently on screen (desk-active or a
    /// group-member chip is focused). Prefer `isLayoutDeskActive` when the
    /// question is park/collapse behaviour.
    var hasActiveLayout: Bool {
        isLayoutDeskActive || chrome.tabs.contains { $0.group != 0 && $0.active }
    }

    /// Open or close the docked terminal (⌃T).
    ///
    /// Calls the engine directly. The dock's button and its ✕ used to
    /// synthesise a ⌃T keystroke, which a focused terminal pane now eats as a
    /// control byte — so the button did nothing and the panel sat on an empty
    /// state you could not dismiss.
    func toggleTerminalDock() {
        guard let engine else { return }
        suisei_engine_toggle_terminal_dock(engine)
        refreshChrome()
    }

    func focusTerminalPane(_ index: Int) {
        reclaimKeyboardFromTextFields()
        focusPane(index)
    }

    func focusTerminal(_ on: Bool) {
        guard let engine else { return }
        // Same trap as the palette: giving the terminal focus in the *engine*
        // does nothing about the window's first responder, so with the project
        // tree's filter still focused the shell was handed the keyboard in
        // name only and every keystroke went on landing in that filter.
        if on { reclaimKeyboardFromTextFields() }
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

    /// Raw keyboard text into the focused terminal's PTY (IME-committed
    /// Hangul/CJK, typed characters). The terminal input view routes
    /// `insertText` here — NOT `pasteText`, which the shell would treat as a
    /// bracketed paste. Repaints the shell at once.
    func terminalInput(_ text: String) {
        guard let engine, !text.isEmpty else { return }
        text.withCString { suisei_engine_terminal_input(engine, $0) }
        refreshChrome()
    }

    /// Scroll the terminal panel through its scrollback; positive = older.
    func terminalScroll(_ rows: Int32) {
        guard let engine, rows != 0 else { return }
        suisei_engine_terminal_scroll(engine, rows)
        refreshChrome()
    }

    /// Size the PTY grid to the terminal panel (cells).
    ///
    /// The resize is DEBOUNCED to the drag's settle, not applied per step. A
    /// live drag crosses many cell boundaries; resizing the PTY on each one
    /// sent a burst of `SIGWINCH`es, and — crucially — a drag that dips to the
    /// minimum width made the shell reflow (wrap) its output at that tiny width.
    /// Widening back cannot un-wrap it, so the grid came back "clipped and
    /// stuck". Applying exactly one resize at the FINAL size skips every
    /// intermediate width, so the shell only ever reflows to where the panel
    /// actually settles.
    func terminalResize(cols: Int, rows: Int) {
        guard cols > 0, rows > 0 else { return }
        pendingTerminalResize = (cols, rows, nil)
        scheduleTerminalResize()
    }

    /// Size a docked/pane PTY once the resize drag settles. `pane == nil` is the
    /// docked shell (`chrome.terminal`); a pane index routes to that pane's PTY
    /// and repaints through the split path instead.
    private var pendingTerminalResize: (cols: Int, rows: Int, pane: Int?)?
    private var terminalResizeWork: DispatchWorkItem?
    private func scheduleTerminalResize() {
        terminalResizeWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self, let engine = self.engine,
                  let (cols, rows, pane) = self.pendingTerminalResize else { return }
            if let pane {
                suisei_engine_terminal_resize_pane(engine, UInt32(pane), UInt32(cols), UInt32(rows))
                self.refreshEditorPaintOnly()
            } else {
                suisei_engine_terminal_resize(engine, UInt32(cols), UInt32(rows))
                self.refreshChrome()
            }
        }
        terminalResizeWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.08, execute: work)
    }

    /// Size a PANE terminal's own PTY (cells). Pane shells are separate
    /// processes — the docked `terminalResize` never touched them, so they
    /// kept their spawn-time guess and output wrapped at the wrong column.
    func terminalResizePane(_ pane: Int, cols: Int, rows: Int) {
        guard cols > 0, rows > 0 else { return }
        // Debounced to settle, same as the docked shell (see `terminalResize`).
        pendingTerminalResize = (cols, rows, pane)
        scheduleTerminalResize()
    }

    /// Scroll a pane terminal through its scrollback; positive = older.
    func terminalScrollPane(_ pane: Int, _ rows: Int32) {
        guard let engine, rows != 0 else { return }
        suisei_engine_terminal_scroll_pane(engine, UInt32(pane), rows)
        refreshChrome()
    }

    /// Forward a mouse event to a terminal's inner app (xterm tracking).
    /// pane = -1 targets the dock. True when the shell consumed the event
    /// (the caller should not also act on it — e.g. wheel → scrollback).
    func terminalMouse(pane: Int32, button: UInt8, x: Int32, y: Int32, pressed: Bool, motion: Bool) -> Bool {
        guard let engine else { return false }
        let p: UInt32 = pane < 0 ? 0xFFFF : UInt32(pane)
        return suisei_engine_terminal_mouse(
            engine, p, button, UInt16(x), UInt16(y),
            pressed ? 1 : 0, motion ? 1 : 0
        ) != 0
    }

    /// Tell Core the face has acted on its scroll intent.
    func clearScrollIntent() {
        guard let engine else { return }
        suisei_engine_clear_scroll_intent(engine)
    }

    func gotoTab(_ index: Int) {
        guard let engine else { return }
        suisei_engine_goto_tab(engine, UInt32(index))
        withAnimation(.snappy(duration: 0.22 * Self.animationSlowmo)) {
            refreshChrome()
        }
    }

    func closeTab(_ index: Int) {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_close_tab(engine, UInt32(index))
        refreshChromeWithTabMotion(
            kind: "tab-close-index",
            duration: Self.tabCloseDuration
        )
        ensureEditorFocus()
    }

    // Stable-id tab addressing. Strip slots diverge from buffer indices the
    // moment a folded layout gathers its members into a run (grouped) or
    // hides them behind one chip (unified) — every slot after the group then
    // names a different document than the same buffer index, so chips are
    // addressed by `stableId` and the engine translates.

    /// Switch tabs. No animation, deliberately.
    ///
    /// This used to wrap `refreshChrome()` in `withAnimation(.snappy(0.22))`,
    /// which glided the selection capsule across to the clicked chip — and
    /// animated everything else the refresh republished along with it.
    /// Switching tabs is a jump, not a journey: the content under the strip is
    /// replaced outright, so a capsule travelling to catch up arrives after the
    /// thing it is meant to be pointing at.
    ///
    /// Structural motion is unaffected. Opening and closing tabs still animate,
    /// through `refreshChromeWithTabMotion` and the strip's own motion, both
    /// keyed off the tab SET rather than the
    /// selection. ⌃⇥ / ⌃⇧⇥ inherit this, since `stepTab` routes here.
    func gotoTabId(_ stableId: UInt64) {
        guard let engine else { return }
        suisei_engine_goto_tab_id(engine, stableId)
        refreshChrome()
    }

    func closeTabId(_ stableId: UInt64) {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_close_tab_id(engine, stableId)
        refreshChromeWithTabMotion(
            kind: "tab-close-\(stableId)",
            duration: Self.tabCloseDuration
        )
        ensureEditorFocus()
    }

    func moveTabIds(from: UInt64, to: UInt64) -> Bool {
        guard let engine, from != to else { return false }
        let moved = suisei_engine_move_tab_ids(engine, from, to) != 0
        if moved {
            refreshChromeWithTabMotion(
                kind: "tab-reorder-\(from)-to-\(to)",
                duration: Self.tabReorderDuration
            )
        }
        return moved
    }

    /// "Close Tab" on a layout chip: the layout entry goes, its documents
    /// stay open as loose tabs.
    func dropLayout(_ id: UInt64) {
        guard let engine else { return }
        if suisei_engine_drop_layout(engine, id) != 0 {
            refreshChromeWithTabMotion(
                kind: "layout-drop-\(id)",
                duration: Self.layoutContainerDuration
            )
        }
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

    /// Pane header menu → place a sibling directly left of the chosen pane.
    func splitEditorLeft() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_split_left(engine)
        refreshChrome()
    }

    /// Xcode “+ → Editor Pane Below”
    func splitEditorBelow() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_split_horizontal(engine)
        refreshChrome()
    }

    /// Pane header menu → place a sibling directly above the chosen pane.
    func splitEditorAbove() {
        guard let engine else { return }
        cancelPointerSession()
        suisei_engine_split_above(engine)
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

    /// Fingerprint of the diagnostic set behind `diagnostics`, so an unchanged
    /// set costs one `u64` instead of a 48.6 KiB copy and a `String` per entry.
    private var lastDiagnosticsFingerprint: UInt64 = 0

    func refreshDiagnostics() {
        guard let engine else {
            diagnostics = []
            lastDiagnosticsFingerprint = 0
            return
        }
        // The snapshot is 48.6 KiB and this runs on every full refresh — but
        // diagnostics only move when a language server answers. Ask the cheap
        // question first.
        // The fingerprint is the single source of truth for "did it change".
        // It starts at 0 and so does `diagnostics`, so the two cannot drift.
        let fingerprint = suisei_engine_diagnostics_fingerprint(engine)
        if fingerprint == lastDiagnosticsFingerprint { return }
        lastDiagnosticsFingerprint = fingerprint
        if fingerprint == 0 {
            diagnostics = []
            return
        }
        var snap = SuiseiDiagnosticsSnapshot()
        guard suisei_engine_diagnostics(engine, &snap) != 0 else {
            diagnostics = []
            lastDiagnosticsFingerprint = 0
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
        // Every request, including Clear, invalidates an older async result.
        // Otherwise a grep already in flight can repopulate the list after
        // the field was emptied.
        searchGeneration &+= 1
        let generation = searchGeneration
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
        if key.identifier == WindowChrome.settingsIdentifier
            || key.title == "Settings" || key.title == "Welcome"
        {
            return false
        }
        if key.identifier == WindowChrome.gitWorkbenchIdentifier { return false }
        if let responder = key.firstResponder,
           // Our own canvas is an NSTextInputClient now — it must stay
           // "editor-owned" so the ⌘-chords below still reach the engine.
           // Plain typing is handed back to it at the tail of the monitor.
           !(responder is EditorCanvasView),
           // The terminal grid is likewise an NSTextInputClient: keep it
           // editor-owned so ⌘W/⌘S still work, and plain keys are handed to its
           // keyDown at the monitor tail (terminalCanvasHasFocus).
           !(responder is TermCanvas),
           responder is NSTextView || responder is NSTextField
               || responder is NSTextInputClient
        {
            // Focused native/SwiftUI text input (tree filter, future fields).
            return false
        }
        return true
    }

    private var nativeTextControlHasFocus: Bool {
        guard let responder = NSApp.keyWindow?.firstResponder,
              !(responder is EditorCanvasView)
        else { return false }
        return responder is NSTextView || responder is NSTextField
            || responder is NSTextInputClient
    }

    /// The editor canvas has focus, so plain keys must flow through its
    /// `NSTextInputClient` path (input method, standard key bindings) instead
    /// of being swallowed here.
    private var editorCanvasHasFocus: Bool {
        NSApp.keyWindow?.firstResponder is EditorCanvasView
    }

    /// The terminal grid holds the keyboard. Like the editor canvas it is now an
    /// `NSTextInputClient`, so the monitor hands plain keys back to its `keyDown`
    /// (which runs the input method for Hangul/CJK) instead of routing them raw.
    var terminalCanvasHasFocus: Bool {
        NSApp.keyWindow?.firstResponder is TermCanvas
    }

    private func installKeyMonitor() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            let plainPanelKey = !flags.contains(.command)
                && !flags.contains(.control)
                && !flags.contains(.option)

            // The full workbench is a native auxiliary window now. Intercept
            // its global chord before Core can replace the editor with the old
            // docked mode; the editor scene owns `openWindow` and brings the
            // existing window forward when it is already open.
            if flags.contains(.control), flags.contains(.shift),
               !flags.contains(.command), !flags.contains(.option),
               event.charactersIgnoringModifiers?.lowercased() == "g"
            {
                NotificationCenter.default.post(name: .suiseiOpenGitWorkbenchWindow, object: nil)
                return nil
            }

            // Find and Palette use native text fields so IME, selection and
            // editing shortcuts are AppKit-owned. Their navigation keys still
            // belong to the surrounding transient surface.
            if self.nativeTextControlHasFocus, plainPanelKey,
               self.chrome.palette.open
            {
                switch event.keyCode {
                case 53:
                    NSApp.keyWindow?.makeFirstResponder(nil)
                    self.dispatch(code: .esc)
                    self.ensureEditorFocus()
                    return nil
                case 36, 76:
                    NSApp.keyWindow?.makeFirstResponder(nil)
                    self.dispatch(code: .enter)
                    self.ensureEditorFocus()
                    return nil
                case 125:
                    self.dispatch(code: .down)
                    return nil
                case 126:
                    self.dispatch(code: .up)
                    return nil
                default:
                    break
                }
            }
            if self.nativeTextControlHasFocus, plainPanelKey,
               self.chrome.search.open
            {
                switch event.keyCode {
                case 53:
                    self.cancelFind()
                    return nil
                case 36, 76:
                    self.closeFind()
                    return nil
                case 125:
                    self.findStep(forward: true)
                    return nil
                case 126:
                    self.findStep(forward: false)
                    return nil
                default:
                    break
                }
            }
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
                // By stableId: under a layout group slot indices diverge from
                // buffer indices, and `closeTab(slot)` closes the wrong document.
                if c == "w" {
                    if hasShift { return event }
                    if let active = self.chrome.tabs.first(where: \.active) {
                        if active.isLayout {
                            self.dropLayout(active.group)
                        } else {
                            self.closeTabId(active.stableId)
                        }
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
            // ⌃⇥ / ⌃⇧⇥ — cycle document tabs (standard macOS). A focused pane
            // shell gets the key instead: stealing it there is the surprising
            // direction (the dock, which owns ⌃⇥ by convention, keeps it).
            if event.keyCode == 48, event.modifierFlags.contains(.control) {
                if self.terminalOwnsKeys, self.focus != .terminal {
                    return event
                }
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
            // Same for the terminal grid: it is an NSTextInputClient too now, so
            // hand plain keys to its keyDown (which composes Hangul and routes
            // control/non-text keys to the PTY) rather than feeding the PTY raw.
            if self.terminalCanvasHasFocus { return event }
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

    func settingsSetValue(_ row: Int, value: Int) {
        guard let engine else { return }
        suisei_engine_settings_set_value(engine, UInt32(row), UInt32(max(0, value)))
        refreshChrome()
    }

    func settingsSetHighlightColor(_ value: String) {
        guard let engine else { return }
        value.withCString { suisei_engine_settings_set_highlight_color(engine, $0) }
        refreshChrome()
    }

    var glassStyle: SuiseiGlassStyle {
        guard let engine else { return .clear }
        return SuiseiGlassStyle(rawValue: suisei_engine_glass_style(engine)) ?? .clear
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
                    toggleTerminalTab()
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
        let chording = flags.contains(.command)
            || flags.contains(.control)
            || flags.contains(.option)
        // `charactersIgnoringModifiers` is the UNSHIFTED key. That is exactly
        // what a chord wants — ⇧⌘T is "t" — and exactly wrong for typing,
        // where it turns every capital into lower case. It was preferred
        // unconditionally, so `echo HELLO` reached the shell as `echo hello`
        // and typing into the palette could not produce a capital either. The
        // editor escaped it only because its canvas takes the
        // `NSTextInputClient` path instead of this one.
        let order: [String?] = chording
            ? [event.charactersIgnoringModifiers, event.characters]
            : [event.characters, event.charactersIgnoringModifiers]
        for s in order.compactMap({ $0 }) {
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

        let tabs = decodeTabs(from: snap)

        let (lines, split) = PerfProbe.measure("  decodeEditorLinesAndSplit") {
            decodeEditorLinesAndSplit(from: snap, engine: engine)
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
        EditorMetrics.lineNumberDigits = EditorMetrics.digits(for: snap.line_count)
        next.scroll = snap.scroll
        next.pct = snap.pct
        next.bufferVersion = snap.buffer_version
        next.tabs = tabs
        next.tabsOverflow = max(0, Int(snap.tab_count) - tabs.count)
        next.lines = lines
        next.split = split
        // Cheap open-flag probes only (empty payload when closed).
        PerfProbe.measure("  loadPalette") {
            let palette = loadPalette(engine)
            next.palette = palette.open ? palette : .empty
        }
        PerfProbe.measure("  loadSearch") {
            let search = loadSearch(engine)
            next.search = search.open ? search : .empty
        }
        PerfProbe.measure("  loadCompletions") {
            let completions = loadCompletions(engine)
            next.completions = completions.open ? completions : .empty
        }
        // The docked terminal snapshot is 300 KiB, and pulling it cost ~3.7ms
        // EVERY keystroke — even with no terminal open. Skip it unless the dock
        // is actually open. The light path is only entered when the shell
        // surface did NOT change, and opening/closing the dock IS a shell
        // change (full path), so last frame's flag is accurate here. Pane
        // terminals are pulled separately by `attachPaneTerminals`.
        if chrome.terminal.open {
            PerfProbe.measure("  loadTerminal (300KiB)") {
                let terminal = loadTerminal(engine)
                next.terminal = terminal.open ? terminal : .empty
            }
        }
        // Outline is cheap to copy and the engine refreshes it on idle ticks —
        // keep it live on the light path too (typing never does a full pull).
        PerfProbe.measure("  loadOutline") {
            let outline = loadOutline(engine)
            if outline != next.outline { next.outline = outline }
        }

        // Publish only what a keystroke does NOT change.
        //
        // Measured: `chrome publish` is 16 ms mean and 57 ms worst, and
        // `chrome copy+free (no publish)` beside it is 0.001 ms — the entire
        // cost is `objectWillChange` reaching every observer of this object,
        // which is ContentView's whole body. Twice a frame's budget to say the
        // cursor column moved by one.
        //
        // The first attempt gated that on "is a settle pending", and the trace
        // says why that was the wrong shape: a block where every one of
        // sixteen paint refreshes published anyway. Ten callers reach this
        // function and most of them schedule no settle, so the guard answered
        // for the caller rather than for the change.
        //
        // Gating on the CHANGE has no such hole. Ln/Col, the caret column, the
        // line count, the scroll and the buffer version move on every
        // keystroke and are read by the status line, which the settle
        // refreshes 0.12s after the last key. Carried over from the current
        // value, they cannot make `next` differ — so a keystroke that touches
        // nothing else publishes nothing at all, whoever called.
        let volatilePerKeystroke = (
            cursorRow: next.cursorRow, cursorCol: next.cursorCol,
            caretVCol: next.caretVCol, lineCount: next.lineCount,
            scroll: next.scroll, pct: next.pct,
            bufferVersion: next.bufferVersion, dirty: next.dirty
        )
        next.cursorRow = chrome.cursorRow
        next.cursorCol = chrome.cursorCol
        next.caretVCol = chrome.caretVCol
        next.lineCount = chrome.lineCount
        next.scroll = chrome.scroll
        next.pct = chrome.pct
        next.bufferVersion = chrome.bufferVersion
        next.dirty = chrome.dirty

        let differs = PerfProbe.measure("  chrome deep compare") { next != chrome }
        if differs {
            // Something real changed, so the volatile fields ride along with
            // it — they are current and the publish is already being paid for.
            next.cursorRow = volatilePerKeystroke.cursorRow
            next.cursorCol = volatilePerKeystroke.cursorCol
            next.caretVCol = volatilePerKeystroke.caretVCol
            next.lineCount = volatilePerKeystroke.lineCount
            next.scroll = volatilePerKeystroke.scroll
            next.pct = volatilePerKeystroke.pct
            next.bufferVersion = volatilePerKeystroke.bufferVersion
            next.dirty = volatilePerKeystroke.dirty
        }
        if differs {
            // Kept from the investigation, because it is what settled which of
            // two candidates owns the cost. `chromeShadow` holds the previous
            // snapshot and nothing else does, so assigning to it copies in AND
            // frees the old one — the identical work, minus the publisher.
            //
            // An earlier version used `var scratch = chrome; scratch = next`,
            // which leaves `chrome` holding the old value, so nothing is ever
            // deallocated: it measured 0.000 ms whatever the payload cost, and
            // "so the rest must be SwiftUI" did not follow from it. With the
            // release actually happening it still reads 0.001 ms, and the
            // conclusion finally does.
            if PerfProbe.enabled {
                PerfProbe.measure("  chrome copy+free (no publish)") {
                    chromeShadow = next
                }
            }
            PerfProbe.measure("  chrome publish") { chrome = next }
        }
        // Per-keystroke caret/scroll goes to the isolated tick store LAST, so
        // `chrome` is already current when a pane's `updateNSView` fires from it.
        PerfProbe.measure("  editorTick publish") {
            publishEditorTick(from: snap, split: split)
        }
    }

    /// Fill the isolated per-keystroke store. Kept off `chrome`/`editorSplit` so
    /// its publish reaches only the pane canvases, not the whole view tree.
    private func publishEditorTick(from snap: SuiseiChromeSnapshot, split: SplitSnap) {
        let t = editorTick
        if t.gen != snap.frame_gen { t.gen = snap.frame_gen }
        if t.scrollIntent != snap.scroll_intent { t.scrollIntent = snap.scroll_intent }
        let frac = CGFloat(snap.scroll_frac)
        if abs(t.scrollFrac - frac) > 0.0001 { t.scrollFrac = frac }
        let panes: [EditorTickStore.PaneTick] = split.isSplit
            ? split.panes.map {
                .init(scroll: $0.scroll, hscroll: $0.hscroll, docLineCount: $0.docLineCount)
            }
            : [.init(scroll: snap.scroll, hscroll: snap.hscroll, docLineCount: snap.line_count)]
        if panes != t.panes { t.panes = panes }
    }

    /// Tab-block decode of the chrome snapshot — the single source of truth
    /// for the raw-array walk. Three verbatim copies used to exist (paint
    /// path, light refresh, full refresh); a new tab field had to land in
    /// all three, and missing one desynced the strip on exactly that path.
    private func decodeTabs(from snap: SuiseiChromeSnapshot) -> [TabItem] {
        var tabs: [TabItem] = []
        let tabCount = Int(snap.tab_count)
        withUnsafeBytes(of: snap.tab_titles) { titlesRaw in
            withUnsafeBytes(of: snap.tab_dirty) { dirtyRaw in
                let ids = withUnsafeBytes(of: snap.tab_ids) { Array($0.bindMemory(to: UInt64.self)) }
                let groups = withUnsafeBytes(of: snap.tab_groups) { Array($0.bindMemory(to: UInt64.self)) }
                let isLayout = withUnsafeBytes(of: snap.tab_is_layout) { Array($0.bindMemory(to: UInt8.self)) }
                let isTerminal = withUnsafeBytes(of: snap.tab_is_terminal) { Array($0.bindMemory(to: UInt8.self)) }
                let titleCap = Int(SUISEI_TITLE_CAP)
                for i in 0..<min(tabCount, Int(SUISEI_MAX_TABS)) {
                    let base = titlesRaw.baseAddress!.advanced(by: i * titleCap)
                    let title = String(cString: base.assumingMemoryBound(to: CChar.self))
                    // bit 0 = dirty, bit 1 = deleted-on-disk (packed in one byte).
                    let flags = dirtyRaw[i]
                    tabs.append(TabItem(
                        id: i,
                        stableId: ids[i],
                        title: title.isEmpty ? "[No Name]" : title,
                        dirty: flags & 1 != 0,
                        active: i == Int(snap.tab_active),
                        group: groups[i],
                        isLayout: isLayout[i] != 0,
                        isTerminal: isTerminal[i] != 0,
                        deleted: flags & 2 != 0
                    ))
                }
            }
        }
        return tabs
    }

    private func decodeEditorLinesAndSplit(
        from snap: SuiseiChromeSnapshot,
        engine: OpaquePointer
    ) -> ([EditorLine], SplitSnap) {
        func paneTitle(_ pane: Int) -> String {
            withUnsafeBytes(of: snap.pane_titles) { raw in
                let base = raw.baseAddress!.advanced(
                    by: pane * Int(SUISEI_TITLE_CAP)
                )
                let value = String(cString: base.assumingMemoryBound(to: CChar.self))
                return value.isEmpty ? "[No Name]" : value
            }
        }

        // The packed line stream is DELIBERATELY not decoded.
        //
        // The editor is a *pull* renderer: each `EditorHost` canvas pulls its
        // own rows from the engine (`build_editor_band`) on draw — see the
        // `let _ = lines` in `editorSurface`. The snapshot's packed lines, the
        // per-pane `lines`, and `editorLines` are never read for rendering
        // (minimap and preview pull separately too).
        //
        // Decoding them anyway cost: up to `SUISEI_MAX_LINES` `EditorLine`
        // allocations — a decoded `text` String and a spans array each — EVERY
        // keystroke, times the pane count. Worse, carrying them inside
        // `editorSplit`/`chrome.split` made those values differ on every
        // keystroke, so publishing `chrome` re-diffed and churned the whole
        // split each time (`chrome publish` measured 0.04ms unsplit → ~8ms the
        // instant you split). Leaving the arrays empty makes `editorSplit`
        // change only on a *structural* edit, so typing no longer re-lays the
        // split. Redraw is driven by `contentGen`, not by these lines.
        let allLines: [EditorLine] = []

        // Split metadata (ABI fields added after tab titles — default 0 when old engine).
        let paneCount = Int(snap.pane_count)
        let focus = Int(snap.pane_focus)

        if paneCount < 2 {
            var single = EditorPaneSnap(
                id: 0, focused: true, tabIndex: Int(snap.tab_active),
                title: paneTitle(0),
                scroll: snap.scroll, hscroll: snap.hscroll,
                docLineCount: snap.line_count, lines: allLines
            )
            // The unsplit pane is synthesised by the FFI rather than walked out
            // of the pane array, but it can still be a terminal.
            single.isTerminal = withUnsafeBytes(of: snap.panes) { raw in
                raw.baseAddress!.load(fromByteOffset: 17, as: UInt8.self) != 0
            }
            // Stamp paneId 0 on all lines.
            return (
                allLines,
                SplitSnap(focus: 0,
                          panes: attachPaneTerminals(engine, [single]))
            )
        }

        // C fixed arrays import as tuples in Swift — walk via raw memory.
        // Layout must match SuiseiPaneC (tab, scroll, start, count, focused+pad, doc_line_count, hscroll).
        var panes: [EditorPaneSnap] = []
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
                // offset 17: is_terminal (a former pad byte).
                let isTerm = base.load(fromByteOffset: 17, as: UInt8.self) != 0
                // offset 18: term_gen (u16) — pane shell content generation.
                let termGen = base.load(fromByteOffset: 18, as: UInt16.self)
                // offset 20: doc_line_count, 24: hscroll (after 4 pad bytes at 16..19)
                let docLineCount = base.load(fromByteOffset: 20, as: UInt32.self)
                let hscroll = base.load(fromByteOffset: 24, as: UInt32.self)
                // offsets 28..40: the pane's normalised rect.
                let rx = base.load(fromByteOffset: 28, as: Float.self)
                let ry = base.load(fromByteOffset: 32, as: Float.self)
                let rw = base.load(fromByteOffset: 36, as: Float.self)
                let rh = base.load(fromByteOffset: 40, as: Float.self)
                _ = lineStart
                _ = lineCount
                // Per-pane lines intentionally empty — pull renderer, see above.
                let paneLines: [EditorLine] = []
                let isFocused = focusedFlag != 0 || pi == focus
                panes.append(EditorPaneSnap(
                    id: pi,
                    focused: isFocused,
                    tabIndex: Int(tabIndex),
                    title: paneTitle(pi),
                    scroll: scroll,
                    hscroll: hscroll,
                    docLineCount: max(1, docLineCount),
                    lines: paneLines,
                    rect: CGRect(
                        x: CGFloat(rx), y: CGFloat(ry),
                        width: CGFloat(rw), height: CGFloat(rh)
                    ),
                    isTerminal: isTerm,
                    termGen: termGen
                ))
            }
        }
        // `lines` stays empty — the panes pull their own rows on draw.
        return (
            allLines,
            SplitSnap(
                focus: focus,
                panes: attachPaneTerminals(engine, panes)
            )
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

        let tabs = PerfProbe.measure("  decodeTabs") { decodeTabs(from: snap) }

        let (lines, split) = PerfProbe.measure("  decodeEditorLinesAndSplit(full)") {
            decodeEditorLinesAndSplit(from: snap, engine: engine)
        }
        // These are FIVE separate `@Published` properties on this object, and
        // each assignment is its own invalidation of everything observing
        // `EngineBridge` — i.e. ContentView's whole body — before `chrome =
        // next` further down has even run. `editorSplit` in particular is
        // guaranteed to change on a layout-tab switch, because that is exactly
        // what a layout does: rearrange the panes.
        //
        // Measured as one block because that is how they are paid.
        PerfProbe.measure("  editor @Published block") {
            if lines != editorLines { editorLines = lines }
            PerfProbe.measure("    editorSplit publish") {
                if split != editorSplit { editorSplit = split }
            }
            // Always adopt Core residual (caret/goto clear it to 0).
            let frac = CGFloat(snap.scroll_frac)
            if abs(frac - editorScrollFrac) > 0.0001 {
                editorScrollFrac = frac
            }
            if snap.hscroll != editorHScroll { editorHScroll = snap.hscroll }
            let wrap = snap.wrap_lines != 0
            if wrap != wrapLines { wrapLines = wrap }
        }
        // Which panels are actually open — ONE u32, answered out of the last
        // composed frame.
        //
        // Every `loadX` below copies a fixed-size C struct: the terminal's is
        // 300 KiB, the git workbench's 55 KiB, preview 64 KiB, diagnostics 49
        // KiB. This used to pull all of them unconditionally and only then
        // check `.open`, discarding the copy when the panel was shut — roughly
        // 730 KiB of memset and copy per refresh, twenty times a second, for a
        // window showing none of them. Asking first costs four bytes.
        let panels = PerfProbe.measure("  open_panels") { suisei_engine_open_panels(engine) }
        func open(_ bit: UInt32) -> Bool { panels & bit != 0 }

        // `preview` is @Published: assigning fires the publisher whether or not
        // the value changed, so compare first. The old code assigned every
        // refresh unconditionally.
        let previewOut = open(SUISEI_PANEL_PREVIEW)
            ? PerfProbe.measure("  loadPreview") { loadPreview(engine) }
            : PreviewSnap.empty
        if previewOut != preview { preview = previewOut }

        // Explorer is the docked Project navigator: it paints its entries in
        // Normal mode too, so the engine sets this bit whenever it HAS entries,
        // not merely when it owns the keyboard.
        let explorer = open(SUISEI_PANEL_EXPLORER)
            ? PerfProbe.measure("  loadExplorer") { loadExplorer(engine) }
            : ExplorerSnap.empty
        let paletteOut = open(SUISEI_PANEL_PALETTE)
            ? PerfProbe.measure("  loadPalette") { loadPalette(engine) }
            : PaletteSnap.empty
        let searchOut = open(SUISEI_PANEL_SEARCH)
            ? PerfProbe.measure("  loadSearch") { loadSearch(engine) }
            : SearchSnap.empty
        let compOut = open(SUISEI_PANEL_COMPLETIONS)
            ? PerfProbe.measure("  loadCompletions") { loadCompletions(engine) }
            : CompletionsSnap.empty
        let termOut = open(SUISEI_PANEL_TERMINAL)
            ? PerfProbe.measure("  loadTerminal") { loadTerminal(engine) }
            : TerminalSnap.empty
        let settingsOut = open(SUISEI_PANEL_SETTINGS)
            ? PerfProbe.measure("  loadSettings") { loadSettings(engine) }
            : SettingsSnap.empty
        let scmOut = open(SUISEI_PANEL_SCM)
            ? PerfProbe.measure("  loadScm") { loadScm(engine) }
            : ScmSnap.empty
        // Theme is 112 bytes and every surface reads it — always.
        let theme = PerfProbe.measure("  loadTheme") { loadTheme(engine) }
        gitWorkbenchStore.publish(theme: theme)
        let outline = open(SUISEI_PANEL_OUTLINE)
            ? PerfProbe.measure("  loadOutline") { loadOutline(engine) }
            : []
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
            // The independent native workbench has its own publication store.
            // Keeping this empty prevents a full editor-shell refresh from
            // comparing/copying Git arrays that the shell never renders.
            gitWb: .empty,
            outline: outline
        )
        // Equatable skip avoids SwiftUI thrash when nothing visual changed.
        let chromeChanged = PerfProbe.measure("  chrome != compare") { next != chrome }
        if chromeChanged {
            PerfProbe.measure("  publish chrome (SwiftUI)") { chrome = next }
        }
        // Diagnostics use a separate bounded FFI snapshot. Core asks for a
        // full refresh whenever its revision changes; adopt the list in the
        // same transaction so an Issues panel opened during LSP indexing does
        // not stay frozen at its initial "No issues".
        PerfProbe.measure("  refreshDiagnostics") { refreshDiagnostics() }
        PerfProbe.measure("  editorTick publish (full)") {
            publishEditorTick(from: snap, split: split)
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

    /// Pull the native Source Control model only when Core says that model
    /// changed. This is intentionally separate from `refreshChrome()`: the
    /// workbench window is independent and neither editor paint nor LSP noise
    /// should copy its large snapshot or invalidate its SwiftUI tree.
    private func refreshGitWorkbenchIfNeeded() {
        guard let engine else { return }
        let generation = suisei_engine_git_wb_generation(engine)
        guard generation != gitWbGeneration else { return }
        gitWbGeneration = generation

        let next = PerfProbe.measure("loadGitWb (dedicated generation)") {
            loadGitWb(engine)
        }
        gitWorkbenchStore.publish(snapshot: next)
        if gitWorkbenchWindowOpen != next.open {
            gitWorkbenchWindowOpen = next.open
        }
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

        if let header = special.first(where: { $0.hasPrefix("diff ·") }) {
            let generation = suisei_engine_git_wb_diff_generation(engine)
            if generation != gitDiffGeneration {
                gitDiffGeneration = generation
                gitDiffLinesCache.removeAll(keepingCapacity: true)

                let byteCount = suisei_engine_git_wb_diff_byte_count(engine)
                if byteCount > 0, byteCount <= UInt64(Int.max) {
                    var bytes = [CChar](repeating: 0, count: Int(byteCount))
                    let copied = bytes.withUnsafeMutableBufferPointer { buffer in
                        suisei_engine_git_wb_diff_copy(engine, buffer.baseAddress, byteCount)
                    }
                    if copied == byteCount {
                        var start = 0
                        let end = Int(copied)
                        while start < end {
                            var stop = start
                            while stop < end, bytes[stop] != 0 { stop += 1 }
                            let line = bytes[start..<stop].map { UInt8(bitPattern: $0) }
                            gitDiffLinesCache.append(String(decoding: line, as: UTF8.self))
                            start = stop + 1
                        }
                    }
                }
            }
            special = [header] + gitDiffLinesCache
        }

        func loadFixedStrings(
            _ count: Int,
            _ field: UnsafeRawBufferPointer,
            stride: Int
        ) -> [String] {
            guard let base = field.baseAddress else { return [] }
            return (0..<count).map { index in
                String(cString: base.advanced(by: index * stride).assumingMemoryBound(to: CChar.self))
            }
        }

        let worktreeCount = min(Int(snap.worktree_count), Int(SUISEI_MAX_GIT_WORKTREE))
        let worktreePaths = withUnsafeBytes(of: snap.worktree_paths) {
            loadFixedStrings(worktreeCount, $0, stride: Int(SUISEI_GIT_PATH))
        }
        let worktreeStages = withUnsafeBytes(of: snap.worktree_staged) {
            Array($0.prefix(worktreeCount))
        }
        let worktreeStatuses = withUnsafeBytes(of: snap.worktree_status) {
            Array($0.prefix(worktreeCount))
        }
        let worktree = (0..<worktreeCount).map { index in
            let byte = worktreeStatuses[index]
            return GitWorktreeItem(
                id: index,
                path: worktreePaths[index],
                status: byte == 0 ? "?" : String(UnicodeScalar(byte)),
                staged: worktreeStages[index] != 0,
                selected: index == Int(snap.selected_change)
            )
        }

        let historyCount = min(Int(snap.history_count), Int(SUISEI_MAX_GIT_HISTORY))
        let historyHashes = withUnsafeBytes(of: snap.history_hashes) {
            loadFixedStrings(historyCount, $0, stride: 48)
        }
        let historyShorts = withUnsafeBytes(of: snap.history_shorts) {
            loadFixedStrings(historyCount, $0, stride: 16)
        }
        let historySubjects = withUnsafeBytes(of: snap.history_subjects) {
            loadFixedStrings(historyCount, $0, stride: Int(SUISEI_GIT_SUBJECT))
        }
        let historyAuthors = withUnsafeBytes(of: snap.history_authors) {
            loadFixedStrings(historyCount, $0, stride: Int(SUISEI_GIT_AUTHOR))
        }
        let historyWhens = withUnsafeBytes(of: snap.history_whens) {
            loadFixedStrings(historyCount, $0, stride: 64)
        }
        let history = (0..<historyCount).map { index in
            GitHistoryItem(
                id: index,
                hash: historyHashes[index],
                shortHash: historyShorts[index],
                subject: historySubjects[index],
                author: historyAuthors[index],
                when: historyWhens[index],
                selected: index == Int(snap.history_selected)
            )
        }

        let branchCount = min(Int(snap.branch_count), Int(SUISEI_MAX_GIT_BRANCHES))
        let branchNames = withUnsafeBytes(of: snap.branch_names) {
            loadFixedStrings(branchCount, $0, stride: Int(SUISEI_GIT_PATH))
        }
        let branchUpstreams = withUnsafeBytes(of: snap.branch_upstreams) {
            loadFixedStrings(branchCount, $0, stride: Int(SUISEI_GIT_PATH))
        }
        let branchCurrent = withUnsafeBytes(of: snap.branch_current) {
            Array($0.prefix(branchCount))
        }
        let branchRemote = withUnsafeBytes(of: snap.branch_remote) {
            Array($0.prefix(branchCount))
        }
        let branches = (0..<branchCount).map { index in
            GitBranchItem(
                id: index,
                name: branchNames[index],
                upstream: branchUpstreams[index],
                current: branchCurrent[index] != 0,
                remote: branchRemote[index] != 0,
                selected: index == Int(snap.branch_selected)
            )
        }

        let commitFileCount = min(Int(snap.commit_file_count), Int(SUISEI_MAX_GIT_FILES))
        let commitFilePaths = withUnsafeBytes(of: snap.commit_file_paths) {
            loadFixedStrings(commitFileCount, $0, stride: Int(SUISEI_GIT_PATH))
        }
        let commitFileStatuses = withUnsafeBytes(of: snap.commit_file_status) {
            Array($0.prefix(commitFileCount))
        }
        let commitFileInsertions = withUnsafeBytes(of: snap.commit_file_insertions) {
            Array($0.bindMemory(to: UInt32.self).prefix(commitFileCount))
        }
        let commitFileDeletions = withUnsafeBytes(of: snap.commit_file_deletions) {
            Array($0.bindMemory(to: UInt32.self).prefix(commitFileCount))
        }
        let commitFiles = (0..<commitFileCount).map { index in
            let byte = commitFileStatuses[index]
            return GitCommitFileItem(
                id: index,
                path: commitFilePaths[index],
                status: byte == 0 ? "?" : String(UnicodeScalar(byte)),
                insertions: Int(commitFileInsertions[index]),
                deletions: Int(commitFileDeletions[index]),
                selected: index == Int(snap.commit_file_selected)
            )
        }

        let commitDetail: GitCommitDetailSnap? = snap.commit_detail_valid == 0 ? nil :
            GitCommitDetailSnap(
                hash: cStringField(snap.detail_hash),
                shortHash: cStringField(snap.detail_short),
                subject: cStringField(snap.detail_subject),
                author: cStringField(snap.detail_author),
                email: cStringField(snap.detail_email),
                date: cStringField(snap.detail_date),
                body: cStringField(snap.detail_body),
                insertions: Int(snap.detail_insertions),
                deletions: Int(snap.detail_deletions)
            )

        let stashCount = min(Int(snap.stash_count), Int(SUISEI_MAX_GIT_STASHES))
        let stashes = withUnsafeBytes(of: snap.stashes) {
            loadFixedStrings(stashCount, $0, stride: Int(SUISEI_GIT_WB_LINE))
        }
        let remoteCount = min(Int(snap.remote_count), Int(SUISEI_MAX_GIT_REMOTES))
        let remoteNames = withUnsafeBytes(of: snap.remote_names) {
            loadFixedStrings(remoteCount, $0, stride: Int(SUISEI_GIT_AUTHOR))
        }
        let remoteURLs = withUnsafeBytes(of: snap.remote_urls) {
            loadFixedStrings(remoteCount, $0, stride: Int(SUISEI_GIT_PATH))
        }
        let remotes = (0..<remoteCount).map { index in
            GitRemoteItem(id: index, name: remoteNames[index], url: remoteURLs[index])
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
            special: special,
            rootPath: cStringField(snap.root_path),
            repositoryName: cStringField(snap.repository_name),
            authorName: cStringField(snap.author_name),
            authorEmail: cStringField(snap.author_email),
            worktree: worktree,
            history: history,
            branches: branches,
            commitFiles: commitFiles,
            commitDetail: commitDetail,
            stashes: stashes,
            remotes: remotes
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
        let rn = min(Int(snap.row_count), Int(SUISEI_MAX_SETTINGS_ROWS))
        let labels = withUnsafeBytes(of: snap.row_labels) { raw in
            (0..<rn).map { readCString(at: raw.baseAddress!, offset: $0 * Int(SUISEI_SETTINGS_LABEL), cap: Int(SUISEI_SETTINGS_LABEL)) }
        }
        let values = withUnsafeBytes(of: snap.row_values) { raw in
            (0..<rn).map { readCString(at: raw.baseAddress!, offset: $0 * Int(SUISEI_SETTINGS_VALUE), cap: Int(SUISEI_SETTINGS_VALUE)) }
        }
        let groups = withUnsafeBytes(of: snap.row_groups) { raw in
            (0..<rn).map { readCString(at: raw.baseAddress!, offset: $0 * Int(SUISEI_SETTINGS_GROUP), cap: Int(SUISEI_SETTINGS_GROUP)) }
        }
        let details = withUnsafeBytes(of: snap.row_details) { raw in
            (0..<rn).map { readCString(at: raw.baseAddress!, offset: $0 * Int(SUISEI_SETTINGS_DETAIL), cap: Int(SUISEI_SETTINGS_DETAIL)) }
        }
        let optionStrings = withUnsafeBytes(of: snap.row_options) { raw in
            (0..<rn).map { readCString(at: raw.baseAddress!, offset: $0 * Int(SUISEI_SETTINGS_OPTIONS), cap: Int(SUISEI_SETTINGS_OPTIONS)) }
        }
        let kinds = withUnsafeBytes(of: snap.row_kind) { Array($0.bindMemory(to: UInt32.self)) }
        let payloads = withUnsafeBytes(of: snap.row_payload) { Array($0.bindMemory(to: UInt32.self)) }
        let pages = withUnsafeBytes(of: snap.row_page) { Array($0.bindMemory(to: UInt32.self)) }
        let controls = withUnsafeBytes(of: snap.row_control) { Array($0.bindMemory(to: UInt32.self)) }
        let valueIndices = withUnsafeBytes(of: snap.row_value_index) { Array($0.bindMemory(to: UInt32.self)) }
        let headers = withUnsafeBytes(of: snap.row_header) { Array($0.bindMemory(to: UInt8.self)) }
        let selected = withUnsafeBytes(of: snap.row_selected) { Array($0.bindMemory(to: UInt8.self)) }
        let advanced = withUnsafeBytes(of: snap.row_advanced) { Array($0.bindMemory(to: UInt8.self)) }

        for i in 0..<rn {
            rows.append(SettingsRowItem(
                id: i,
                label: labels[i],
                value: values[i],
                isHeader: headers[i] != 0,
                selected: selected[i] != 0,
                kind: SettingKind(rawValue: kinds[i]) ?? .none,
                payload: Int(payloads[i]),
                page: SettingSurfacePage(rawValue: pages[i]) ?? .none,
                group: groups[i],
                control: SettingControlKind(rawValue: controls[i]) ?? .none,
                detail: details[i],
                options: optionStrings[i].split(separator: "|").map(String.init),
                valueIndex: Int(valueIndices[i]),
                advanced: advanced[i] != 0
            ))
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

    /// Fill in each terminal pane's own rows.
    ///
    /// Only terminal panes are pulled, so this is usually zero work and at most
    /// four. It cannot come from the chrome's single terminal snapshot: these
    /// are separate processes, and sharing one snapshot is precisely what made
    /// two terminal panes mirror each other.
    private func attachPaneTerminals(_ engine: OpaquePointer, _ panes: [EditorPaneSnap])
        -> [EditorPaneSnap]
    {
        guard panes.contains(where: \.isTerminal) else { return panes }
        // The grid in hand (last publish) is reused when the shell's content
        // generation is unchanged — each pull decodes ~300 KiB, which used to
        // run per keystroke per idle terminal. A tabIndex mismatch forces a
        // pull, so a reshuffled split can never inherit another shell's grid.
        let prev = editorSplit.panes
        var out = panes
        for i in out.indices where out[i].isTerminal {
            if prev.indices.contains(i), !prev[i].termLines.isEmpty,
               prev[i].tabIndex == out[i].tabIndex, prev[i].termGen == out[i].termGen
            {
                out[i].termLines = prev[i].termLines
                out[i].termCursorRow = prev[i].termCursorRow
                out[i].termCursorCol = prev[i].termCursorCol
                continue
            }
            var snap = SuiseiTerminalSnapshot()
            guard suisei_engine_terminal_for_pane(engine, UInt32(i), &snap) != 0 else { continue }
            var lines: [String] = []
            let n = Int(snap.count)
            withUnsafeBytes(of: snap.lines) { raw in
                let cap = Int(SUISEI_TERM_LINE)
                for j in 0..<min(n, Int(SUISEI_MAX_TERM_LINES)) {
                    let b = raw.baseAddress!.advanced(by: j * cap)
                    lines.append(String(cString: b.assumingMemoryBound(to: CChar.self)))
                }
            }
            out[i].termLines = lines
            out[i].termCursorRow = Int(snap.cursor_row)
            out[i].termCursorCol = Int(snap.cursor_col)
        }
        return out
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
        _ = pane // ABI carries a pane binding; the dock has no use for it.
        return TerminalSnap(
            open: snap.open != 0,
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

/// Decode one fixed-capacity C string field out of a packed struct.
///
/// This used to append a `CChar` at a time into a growing Swift `Array` and
/// then build a `String` from it — two heap allocations plus the array's
/// reallocation ladder, per line, and `pullBand` calls it for every row of
/// every band, on every repaint of every pane.
///
/// `String(decoding:as:)` is one allocation over the bytes we already have. It
/// is also more forgiving than `String(cString:)`: it needs no NUL terminator
/// (we bound the scan ourselves) and repairs invalid UTF-8 instead of trapping,
/// which matters because the engine truncates these fields at a byte cap.
private func readCString(at base: UnsafeRawPointer, offset: Int, cap: Int) -> String {
    let p = base.advanced(by: offset).assumingMemoryBound(to: UInt8.self)
    let bytes = UnsafeBufferPointer(start: p, count: cap)
    let n = bytes.firstIndex(of: 0) ?? cap
    if n == 0 { return "" }
    return String(decoding: UnsafeBufferPointer(start: p, count: n), as: UTF8.self)
}
