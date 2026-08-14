//  TerminalSurface.swift
//  The terminal, as SwiftTerm's own AppKit view.
//
//  Suisei's terminal used to be a Rust emulator whose cell grid was re-encoded
//  to ANSI to cross the C ABI and re-parsed on this side. Four of the five
//  things wrong with it lived at that boundary rather than in the emulator:
//  rows truncated at a byte budget, a grid drawn with proportional text
//  measurement, a 307 KB snapshot per frame, and two scrollers over one
//  content. Moving to a view that owns its own emulator removes the boundary
//  rather than improving it.
//
//  What crosses the ABI now is only what core still owns: the tab a shell
//  belongs to, the directory it starts in, and — pushed the other way — the
//  title the shell announced. The bytes stay on this side.
//
//  See `third_party/SwiftTerm/VENDOR.md` for what is vendored and why.

import AppKit
import SwiftTerm
import SwiftUI

/// A view that IS a terminal, for the key monitor's benefit.
///
/// `EngineBridge` has to answer two questions about the first responder: may
/// ⌘-chords still reach the engine (yes — ⌘S must save the document behind the
/// split), and should plain keys be handed back to AppKit (yes — the view runs
/// the input method and writes to its own PTY). Both were asked as
/// `responder is TermCanvas`, which is a class, and there are two classes now.
///
/// A protocol rather than a second `is` test at each site: the answer is a
/// property of being a terminal, and the next terminal surface should not have
/// to find every place that enumerates the current ones.
protocol TerminalKeySurface: AnyObject {}

extension TerminalView: TerminalKeySurface {}
/// The docked shell (⌃T) still runs on core's emulator, and its canvas answers
/// the same two questions. Both conformances live here so the set is one list.
extension TermCanvas: TerminalKeySurface {}

/// How a pane terminal should look. Everything here is the editor's, so the
/// shell in a split does not read as a foreign application dropped into it.
struct TerminalPalette: Equatable {
    var background: NSColor
    var foreground: NSColor
    var font: NSFont

    /// The grid colours the face already computes for the docked shell, so the
    /// two terminals cannot drift apart while both exist.
    ///
    /// Qualified: SwiftTerm has a `Color` of its own — a 16-bit-per-channel
    /// terminal colour — and an unqualified one here means neither.
    init(background: SwiftUI.Color, foreground: SwiftUI.Color, fontSize: CGFloat) {
        // `getTerminalColor` reads the RGB components directly; a colour that
        // is not in an RGB space (a catalog colour, or a SwiftUI colour that
        // resolved to one) has no components to read.
        self.background = NSColor(background).usingColorSpace(.sRGB) ?? .black
        self.foreground = NSColor(foreground).usingColorSpace(.sRGB) ?? .white
        self.font = EditorMetrics.monospaced(fontSize, weight: .regular)
    }
}

/// `LocalProcessTerminalView` that reports when it takes the keyboard.
///
/// Focus has to travel in both directions. AppKit gives the view the first
/// responder on a click, but core decides which pane is focused — and if it
/// still believes a document pane is, ⌃⇧T closes the wrong shell and the split
/// draws its focus ring in the wrong column.
///
/// Hooked at `hasFocus` rather than at `mouseDown`: SwiftTerm sets it from
/// `becomeFirstResponder`, so tabbing into the view and a window becoming key
/// with the terminal already selected arrive here too, and a click is only the
/// most common of the three. (`becomeFirstResponder` itself is `public
/// override` in the vendored source — not `open` — so it cannot be overridden
/// from here. `hasFocus` can.)
final class PaneTerminalView: LocalProcessTerminalView {
    var onFocus: (() -> Void)?

    /// What we last told core. The setter runs on every focus change *and*
    /// whenever SwiftTerm refreshes the caret, and telling core to focus a pane
    /// republishes the chrome — so only real transitions are reported.
    private var reportedFocus = false

    override var hasFocus: Bool {
        get { super.hasFocus }
        set {
            super.hasFocus = newValue
            guard newValue != reportedFocus else { return }
            reportedFocus = newValue
            if newValue { onFocus?() }
        }
    }
}

/// One shell: the process, the view that draws it, and the tab that owns both.
///
/// A class rather than a struct because it is a delegate and an owner of a
/// live process — and because the whole point is that it survives the SwiftUI
/// view that shows it.
final class TerminalSession: NSObject, LocalProcessTerminalViewDelegate {
    /// `BufferTab::id`. Stable for the tab's lifetime and never reused, which
    /// is exactly the lifetime this session wants.
    let tabId: UInt64
    let view: PaneTerminalView
    /// Reported by the shell (OSC 0/2) and pushed back to core, which puts it
    /// on the tab chip. `nil` until the shell says something.
    private(set) var title: String?
    /// The shell has ended. Its view stays — with whatever it printed last
    /// still on screen — until the tab closes.
    private(set) var exited = false

    var onTitle: ((UInt64, String?) -> Void)?
    var onExit: ((UInt64) -> Void)?

    init(tabId: UInt64, cwd: String?, palette: TerminalPalette) {
        self.tabId = tabId
        view = PaneTerminalView(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        super.init()
        view.processDelegate = self
        apply(palette)
        // Terminal.app's default. With Option as Meta, a Korean or European
        // keyboard loses the characters that live on Option — and macOS users
        // reach for ⌥← / ⌥→ for word movement, which the shell gets either way.
        view.optionAsMetaKey = false
        view.scrollerStyle = .overlay
        start(cwd: cwd)
    }

    /// Spawn the user's login shell.
    ///
    /// `execName` with a leading dash is what makes it a **login** shell, and
    /// that is not cosmetic: an app launched from Finder inherits Finder's
    /// environment, so without `/etc/zprofile` running `path_helper` the shell
    /// would have no `/usr/local/bin`, no Homebrew, and none of the version
    /// managers that hang off a login profile.
    private func start(cwd: String?) {
        let shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        let name = (shell as NSString).lastPathComponent
        // A directory that has since been deleted or renamed would fail the
        // spawn outright; the home directory is a shell that works.
        let dir = cwd.flatMap { path -> String? in
            var isDir: ObjCBool = false
            let ok = FileManager.default.fileExists(atPath: path, isDirectory: &isDir)
            return ok && isDir.boolValue ? path : nil
        }
        view.startProcess(
            executable: shell,
            environment: Self.environment(),
            execName: "-\(name)",
            currentDirectory: dir ?? NSHomeDirectory()
        )
    }

    /// The app's own environment plus what a terminal owes its child.
    ///
    /// SwiftTerm's `getEnvironmentVariables` builds a minimal one from scratch
    /// and hardcodes `LANG=en_US.UTF-8`. Inheriting instead keeps whatever the
    /// user's session actually has — including a `LANG` that is not English.
    private static func environment() -> [String] {
        var env = ProcessInfo.processInfo.environment
        env["TERM"] = "xterm-256color"
        env["COLORTERM"] = "truecolor"
        // A shell with no locale at all runs in `C`, where a multi-byte
        // character is a sequence of bytes: Hangul in a prompt, in a filename
        // or in `ls` output comes back as mojibake. An app launched from
        // Finder has no `LANG` — one launched from a terminal inherits one —
        // so this is the difference between the two launch paths, not a
        // preference.
        if env["LANG"] == nil, env["LC_ALL"] == nil {
            env["LANG"] = "en_US.UTF-8"
        }
        return env.map { "\($0.key)=\($0.value)" }
    }

    func apply(_ palette: TerminalPalette) {
        if view.font != palette.font { view.font = palette.font }
        if view.nativeBackgroundColor != palette.background {
            view.nativeBackgroundColor = palette.background
        }
        if view.nativeForegroundColor != palette.foreground {
            view.nativeForegroundColor = palette.foreground
        }
        // The caret has to match the grid it sits on, not the editor beside
        // it — the same mistake the old canvas made, where a light theme gave
        // the dark terminal a near-black cursor nobody could see.
        view.caretColor = palette.foreground
        view.caretTextColor = palette.background
    }

    /// End the shell. Called when the tab closes, and by `deinit` for anything
    /// that drops a session without going through the registry.
    func terminate() {
        guard !exited else { return }
        exited = true
        view.processDelegate = nil
        view.terminate()
    }

    deinit {
        // Not `terminate()` — that touches `view`, and by here the only thing
        // guaranteed is that nothing else holds this session. The process must
        // still die: a shell outliving its window is a leaked process the user
        // can only find in Activity Monitor.
        if !exited { view.terminate() }
    }

    // MARK: - LocalProcessTerminalViewDelegate

    func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {
        // SwiftTerm sizes its own PTY. Core used to be told, because core held
        // the grid; it holds nothing now, so there is nobody to tell.
    }

    func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        let next = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let value = next.isEmpty ? nil : next
        guard value != self.title else { return }
        self.title = value
        onTitle?(tabId, value)
    }

    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
        // OSC 7. Worth carrying into `BufferTab::terminal_cwd` one day, so a
        // restored shell reopens where the user left it rather than where they
        // started it — but that is core's field and a separate change.
    }

    func processTerminated(source: TerminalView, exitCode: Int32?) {
        guard !exited else { return }
        exited = true
        onExit?(tabId)
    }
}

/// Every shell the face owns, keyed by the tab showing it.
///
/// The registry exists because SwiftUI view structs are values that get
/// rebuilt constantly — a shell living in one would be forked again on every
/// re-render, and killed on every tab switch. What the user means by "my
/// terminal" is tied to the tab, so that is what holds it.
///
/// Not an `ObservableObject`: nothing here drives a SwiftUI update. The view
/// it hands out is an AppKit view that paints itself.
///
/// One registry for the app because there is one `EngineBridge` for the app,
/// and therefore one `BufferTab::id` space. A second engine would need a
/// second registry — the ids would collide, not merge.
final class TerminalSessions {
    static let shared = TerminalSessions()

    private var sessions: [UInt64: TerminalSession] = [:]

    /// Set by the face once, so a session made deep inside a view update can
    /// still report a title or an exit without every call site threading the
    /// bridge through.
    weak var engine: EngineBridge?

    private init() {}

    /// The session for a tab, forking a shell the first time it is asked for.
    func session(for tabId: UInt64, cwd: String?, palette: TerminalPalette) -> TerminalSession? {
        // Tab 0 is "no tab" over the ABI. A shell keyed there would be adopted
        // by the next pane that also failed to identify itself.
        guard tabId != 0 else { return nil }
        if let live = sessions[tabId] {
            live.apply(palette)
            return live
        }
        let session = TerminalSession(tabId: tabId, cwd: cwd, palette: palette)
        session.onTitle = { [weak self] id, title in
            self?.engine?.setTerminalTitle(tabId: id, title: title)
        }
        session.onExit = { [weak self] id in
            // `exit` at the prompt should close the tab, the way it closes a
            // window in every other terminal. Deferred because this arrives on
            // the process-reader's queue, and closing a tab republishes the
            // chrome.
            DispatchQueue.main.async { self?.engine?.closeTabId(id) }
        }
        sessions[tabId] = session
        return session
    }

    /// Whether a tab already has a shell — asked before making one, when the
    /// caller has no palette and does not want to fork by accident.
    func hasSession(for tabId: UInt64) -> Bool {
        sessions[tabId] != nil
    }

    /// Kill every shell whose tab is gone.
    ///
    /// Driven from the chrome republish rather than from a close button: every
    /// route that can remove a document — the tab ×, ⌘W, a palette command,
    /// replacing the project, closing the window — republishes, and none of
    /// them knows about this. Wiring the shell to one particular button is how
    /// the other five routes leak a process.
    ///
    /// The question is put to core one id at a time rather than answered from
    /// the published tab list. That list is clamped to `SUISEI_MAX_TABS` (64) —
    /// it carries the true count but not the entries — so a terminal sitting
    /// past the cap is absent from it while very much open, and reaping from
    /// absence would kill a running shell for the crime of being the 65th tab.
    func reap(isOpen: (UInt64) -> Bool) {
        // The list first, then the removals. `sessions.keys` is a view onto the
        // dictionary, not a copy of it.
        let gone = sessions.keys.filter { !isOpen($0) }
        for id in gone {
            sessions.removeValue(forKey: id)?.terminate()
        }
    }

    /// Kill everything. The app is quitting.
    func closeAll() {
        for session in sessions.values { session.terminate() }
        sessions.removeAll()
    }
}

/// A terminal pane: SwiftTerm's view, held by the registry, shown here.
///
/// `makeNSView` returns an empty container rather than the terminal itself.
/// The terminal belongs to the session, one pane can be torn down and rebuilt
/// while its shell keeps running, and the same shell can move between panes in
/// a split — none of which is expressible if the representable's own view is
/// the terminal.
struct TerminalPaneSurface: NSViewRepresentable {
    /// `BufferTab::id` of the terminal tab this pane shows.
    let tabId: UInt64
    let palette: TerminalPalette
    /// Core's pane index, so a click can move core's focus to match AppKit's.
    let paneIndex: Int
    let engine: EngineBridge

    func makeNSView(context: Context) -> NSView {
        let container = FlippedContainer()
        container.autoresizesSubviews = true
        return container
    }

    func updateNSView(_ container: NSView, context: Context) {
        let sessions = TerminalSessions.shared
        sessions.engine = engine
        // The working directory is asked for only when a shell is about to be
        // forked — once per terminal in the window's life. That is why it is a
        // pull and not a field on the pane snapshot: a `terminalCwd` there
        // would be copied on every frame in service of one call.
        let cwd = sessions.hasSession(for: tabId) ? nil : engine.paneTerminalCwd(paneIndex)
        guard let session = sessions.session(
            for: tabId, cwd: cwd, palette: palette
        ) else { return }
        let term = session.view
        term.onFocus = { [weak engine] in engine?.focusTerminalPane(paneIndex) }
        if term.superview !== container {
            // Re-parenting rather than adding: in a split, the same tab can be
            // shown by a pane that is being built while the pane that had it is
            // still being torn down.
            term.removeFromSuperview()
            term.frame = container.bounds
            term.autoresizingMask = [.width, .height]
            container.addSubview(term)
            // A pane that appears because the user asked for a terminal should
            // be typable without a second click — but ONLY when core already
            // considers this pane focused. Re-parenting also happens for
            // reasons the user did not ask for (a font change rebuilds every
            // representable, a split rearranges), and grabbing the keyboard
            // then would take it out of the document they are typing in.
            if engine.editorSplit.focus == paneIndex {
                DispatchQueue.main.async {
                    guard term.superview === container, let window = term.window,
                          window.firstResponder !== term
                    else { return }
                    window.makeFirstResponder(term)
                }
            }
        } else if term.frame != container.bounds {
            term.frame = container.bounds
        }
    }

    static func dismantleNSView(_ container: NSView, coordinator: ()) {
        // The terminal leaves with the container, but it is NOT terminated:
        // the pane going away is a layout change (a split closing, a tab
        // switch), not the shell ending. `reap` is what ends shells, and it
        // asks core which tabs are still open.
        for sub in container.subviews where sub is PaneTerminalView {
            sub.removeFromSuperview()
        }
    }

    /// Flipped so a subview pinned at the origin sits at the TOP, which is
    /// where a terminal goes. An unflipped container puts the shell at the
    /// bottom of a tall pane with the gap above it.
    private final class FlippedContainer: NSView {
        override var isFlipped: Bool { true }
    }
}

/// Ends the shells of tabs that have closed.
///
/// A view modifier for the same reason `AudioTabLifetimeModifier` is one: the
/// signal is the chrome republish, which every close route already performs,
/// and `ContentView.body` is at Swift's type-checking limit without it.
struct TerminalTabLifetimeModifier: ViewModifier {
    let engine: EngineBridge

    func body(content: Content) -> some View {
        content.onReceive(engine.$chrome) { _ in
            TerminalSessions.shared.engine = engine
            TerminalSessions.shared.reap { engine.tabIdIsOpen($0) }
        }
    }
}
