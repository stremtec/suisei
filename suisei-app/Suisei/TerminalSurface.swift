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
/// the input method and writes to its own PTY).
///
/// A protocol rather than an `is TerminalView` test at each site: the answer is
/// a property of being a terminal, and the next terminal surface should not
/// have to find every place that enumerates the current ones.
protocol TerminalKeySurface: AnyObject {}

extension TerminalView: TerminalKeySurface {}

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

    /// Take the keyboard on a click.
    ///
    /// AppKit does not do this for you — a window does not move the first
    /// responder just because a view was clicked; the view asks. `NSTextView`
    /// asks, the old `TermCanvas` asked, and SwiftTerm does not: upstream's
    /// sample apps put the terminal in a window where it is the initial first
    /// responder and never needs to. Here it is one surface among several, so
    /// without this a click selected text in a shell that could not be typed
    /// into.
    ///
    /// Before `super`, so the selection this click starts belongs to a view
    /// that already has the keyboard.
    override func mouseDown(with event: NSEvent) {
        if let window, window.firstResponder !== self {
            window.makeFirstResponder(self)
        }
        super.mouseDown(with: event)
    }
}

/// Which shell this is.
///
/// A pane's is named by the tab showing it — `BufferTab::id`, stable for that
/// tab's lifetime and never reused. The docked strip's shells have no tab at
/// all: they are a list of their own, so they get an ordinal from a counter
/// that also never repeats. Two namespaces in one enum rather than two
/// registries, because everything below this line treats them identically.
enum TerminalOwner: Hashable {
    case tab(UInt64)
    case dock(UInt64)

    /// The tab to report a title to, if any. The dock's chips carry their own
    /// titles on this side; core has no name for those shells.
    var tabId: UInt64? {
        if case .tab(let id) = self { return id }
        return nil
    }
}

/// One shell: the process, the view that draws it, and whoever owns both.
///
/// A class rather than a struct because it is a delegate and an owner of a
/// live process — and because the whole point is that it survives the SwiftUI
/// view that shows it.
final class TerminalSession: NSObject, LocalProcessTerminalViewDelegate {
    let owner: TerminalOwner
    let view: PaneTerminalView
    /// Reported by the shell (OSC 0/2). A pane's goes back to core for the tab
    /// chip; a dock session's is shown on its own chip from here. `nil` until
    /// the shell says something.
    @Published private(set) var title: String?
    /// The shell has ended. Its view stays — with whatever it printed last
    /// still on screen — until the tab or chip closes.
    private(set) var exited = false

    var onTitle: ((TerminalOwner, String?) -> Void)?
    var onExit: ((TerminalOwner) -> Void)?

    init(owner: TerminalOwner, cwd: String?, palette: TerminalPalette) {
        self.owner = owner
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
        onTitle?(owner, value)
    }

    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
        // OSC 7. Worth carrying into `BufferTab::terminal_cwd` one day, so a
        // restored shell reopens where the user left it rather than where they
        // started it — but that is core's field and a separate change.
    }

    func processTerminated(source: TerminalView, exitCode: Int32?) {
        guard !exited else { return }
        exited = true
        onExit?(owner)
    }
}

/// Every shell the face owns.
///
/// The registry exists because SwiftUI view structs are values that get
/// rebuilt constantly — a shell living in one would be forked again on every
/// re-render, and killed on every tab switch. What the user means by "my
/// terminal" outlives any view of it, so something else has to hold it.
///
/// It holds both kinds. A pane's shell is reaped when core says its tab is
/// gone; the docked strip's are a list this class owns outright, because core
/// has no idea they exist.
///
/// One registry for the app because there is one `EngineBridge` for the app,
/// and therefore one `BufferTab::id` space. A second engine would need a
/// second registry — the ids would collide, not merge.
final class TerminalSessions: ObservableObject {
    static let shared = TerminalSessions()

    private var sessions: [TerminalOwner: TerminalSession] = [:]

    /// The docked strip's sessions, in chip order, and which chip is showing.
    ///
    /// Published because the chip strip is SwiftUI and this is the only thing
    /// that knows how many shells there are. Core used to: an active
    /// `Terminal` plus a `parked_terminals` vector, with chip indices that
    /// skipped the active slot — arithmetic that existed only because the
    /// active session had to live in a particular field for core's key routing
    /// to find it. Nothing routes through core any more, so the list is a list.
    @Published private(set) var dock: [UInt64] = []
    @Published private(set) var dockActive: Int = 0
    private var nextDockId: UInt64 = 1

    /// Set by the face, so a session made deep inside a view update can still
    /// report a title or an exit without every call site threading the bridge
    /// through.
    weak var engine: EngineBridge?

    private init() {}

    // MARK: - Pane shells

    /// The session for a terminal tab, forking a shell the first time it is
    /// asked for.
    func session(for tabId: UInt64, cwd: String?, palette: TerminalPalette) -> TerminalSession? {
        // Tab 0 is "no tab" over the ABI. A shell keyed there would be adopted
        // by the next pane that also failed to identify itself.
        guard tabId != 0 else { return nil }
        return session(owner: .tab(tabId), cwd: cwd, palette: palette)
    }

    /// Whether a tab already has a shell — asked before making one, when the
    /// caller has no working directory in hand and does not want to fork by
    /// accident just to find out.
    func hasSession(for tabId: UInt64) -> Bool {
        sessions[.tab(tabId)] != nil
    }

    /// Kill every pane shell whose tab is gone.
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
        let gone = sessions.keys.filter { owner in
            guard let id = owner.tabId else { return false }  // dock: not core's
            return !isOpen(id)
        }
        for owner in gone {
            sessions.removeValue(forKey: owner)?.terminate()
        }
    }

    // MARK: - The docked strip

    /// The shell the dock is showing, opening the first one on demand.
    ///
    /// On demand, because ⌃T is "show the strip" and an empty strip is not a
    /// terminal. The alternative — spawning at launch — is a shell nobody asked
    /// for in every window.
    func dockSession(cwd: @autoclosure () -> String?, palette: TerminalPalette) -> TerminalSession? {
        if dock.isEmpty { openDockSession(cwd: cwd(), palette: palette) }
        guard dock.indices.contains(dockActive) else { return nil }
        return session(owner: .dock(dock[dockActive]), cwd: nil, palette: palette)
    }

    /// Add a session after the showing one and switch to it.
    func openDockSession(cwd: String?, palette: TerminalPalette) {
        let id = nextDockId
        nextDockId += 1
        _ = session(owner: .dock(id), cwd: cwd, palette: palette)
        let at = dock.isEmpty ? 0 : min(dockActive + 1, dock.count)
        dock.insert(id, at: at)
        dockActive = at
    }

    func selectDockSession(_ index: Int) {
        guard dock.indices.contains(index) else { return }
        dockActive = index
    }

    /// Close one chip. The strip promotes a neighbour; closing the last one
    /// leaves it empty, and the next ⌃T opens a fresh shell.
    func closeDockSession(_ index: Int) {
        guard dock.indices.contains(index) else { return }
        let id = dock.remove(at: index)
        sessions.removeValue(forKey: .dock(id))?.terminate()
        dockActive = min(dockActive, max(0, dock.count - 1))
    }

    /// The view of the chip that is showing, if a shell exists yet.
    ///
    /// So that focus can be handed to the docked shell from OUTSIDE a view
    /// update — see `EngineBridge.claimDockTerminalKeyboard`. Everything else
    /// here is asked for by a view that is about to draw it; this is asked for
    /// by the thing that just decided the shell owns the keyboard.
    var activeDockView: PaneTerminalView? {
        guard dock.indices.contains(dockActive) else { return nil }
        return sessions[.dock(dock[dockActive])]?.view
    }

    /// What a dock chip is called: the shell's own title, else its ordinal.
    func dockTitle(_ index: Int) -> String {
        guard dock.indices.contains(index) else { return "Shell" }
        if let t = sessions[.dock(dock[index])]?.title, !t.isEmpty { return t }
        return "Shell \(index + 1)"
    }

    // MARK: - Both

    /// Kill everything. The app is quitting.
    func closeAll() {
        for session in sessions.values { session.terminate() }
        sessions.removeAll()
        dock.removeAll()
        dockActive = 0
    }

    private func session(
        owner: TerminalOwner, cwd: String?, palette: TerminalPalette
    ) -> TerminalSession {
        if let live = sessions[owner] {
            live.apply(palette)
            return live
        }
        let session = TerminalSession(owner: owner, cwd: cwd, palette: palette)
        session.onTitle = { [weak self] owner, title in
            guard let self else { return }
            if let tabId = owner.tabId {
                self.engine?.setTerminalTitle(tabId: tabId, title: title)
            } else {
                // A dock chip's name lives here; nothing over the ABI has a
                // word for it. The strip is watching this object.
                self.objectWillChange.send()
            }
        }
        session.onExit = { [weak self] owner in
            // `exit` at the prompt closes the thing showing the shell, the way
            // it closes a window in every other terminal. Deferred because this
            // arrives on the process-reader's queue.
            DispatchQueue.main.async {
                guard let self else { return }
                switch owner {
                case .tab(let id):
                    self.engine?.closeTabId(id)
                case .dock(let id):
                    guard let at = self.dock.firstIndex(of: id) else { return }
                    self.closeDockSession(at)
                }
            }
        }
        sessions[owner] = session
        return session
    }
}

/// The empty box a session's view is put into.
///
/// A container rather than the terminal itself, because the representable's
/// own view is created and destroyed by SwiftUI: the terminal belongs to the
/// session, a pane can be torn down and rebuilt while its shell keeps running,
/// and the same shell moves between hosts (pane ⇄ pane in a split, chip ⇄ chip
/// in the dock). None of that is expressible if the representable owns it.
///
/// Flipped so a subview pinned at the origin sits at the TOP, which is where a
/// terminal goes; an unflipped container puts the shell at the bottom of a tall
/// pane with the gap above it.
final class TerminalHostView: NSView {
    override var isFlipped: Bool { true }

    /// A terminal that should take the keyboard as soon as this box is in a
    /// window. See `claimKeyboard`.
    private weak var awaitingKeyboard: PaneTerminalView?

    /// Put `term` in this box, taking it off whatever box had it.
    ///
    /// Returns true when it actually moved, so the caller can decide about
    /// focus only on a real arrival.
    @discardableResult
    func mount(_ term: PaneTerminalView) -> Bool {
        // Whatever else is in this box comes OUT first.
        //
        // Without this the box accumulated shells. `mount` only ever added, so
        // switching chips left the previous shell's view still a subview, with
        // the new one drawn on top of it — and switching BACK hit the
        // `superview === self` early return, which reordered nothing. You were
        // then looking at the other shell: "shell 1에 codexx 쳐놓고 shell 2 열고
        // codexxxx 쓰고 shell 1 가니 codexxxx로 되어있음." Both processes were
        // alive and distinct the whole time; only one of them was visible, and
        // it was the wrong one.
        var displaced = false
        for sub in subviews where sub !== term {
            sub.removeFromSuperview()
            displaced = true
        }
        if term.superview === self {
            if term.frame != bounds { term.frame = bounds }
            // A chip switch that uncovered this shell IS an arrival as far as
            // the keyboard is concerned — the caller claims focus on a true.
            return displaced
        }
        term.removeFromSuperview()
        term.frame = bounds
        term.autoresizingMask = [.width, .height]
        addSubview(term)
        return true
    }

    /// Hand the keyboard over on the next pass.
    ///
    /// Deferred: this runs inside a SwiftUI view update, and the window may not
    /// even have the view yet. Re-checked on arrival because by then the layout
    /// may have moved on.
    ///
    /// **A missed claim has to be remembered, not dropped.** The caller asks
    /// once, at mount, and `mount` returns false ever after — so a claim that
    /// finds no window has no second chance from above. That is not a corner:
    /// the debug area opens inside `withAnimation`, and the first
    /// `updateNSView` of a transition runs while the representable's view is
    /// still off the window. The shell then sat there running, visible, and
    /// unfocused, with every keystroke going wherever the keyboard already was
    /// — the bug read as "the terminal from the sidebar cannot be typed into",
    /// and clicking it fixed it, which is what made it look intermittent.
    func claimKeyboard(for term: PaneTerminalView) {
        DispatchQueue.main.async { [weak self, weak term] in
            guard let self, let term, term.superview === self else { return }
            guard let window = term.window else {
                // Not on screen yet. `viewDidMoveToWindow` finishes this.
                self.awaitingKeyboard = term
                return
            }
            guard window.firstResponder !== term else { return }
            window.makeFirstResponder(term)
        }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let term = awaitingKeyboard, term.superview === self,
              let window else { return }
        awaitingKeyboard = nil
        if window.firstResponder !== term { window.makeFirstResponder(term) }
    }

    /// Give up the terminal without ending it.
    ///
    /// The host going away is a layout change — a split closing, a tab switch,
    /// the dock being hidden — not the shell ending. Sessions end in the
    /// registry, which knows the difference.
    static func release(_ container: NSView) {
        for sub in container.subviews where sub is PaneTerminalView {
            sub.removeFromSuperview()
        }
    }
}

/// A terminal pane: SwiftTerm's view, held by the registry, shown here.
struct TerminalPaneSurface: NSViewRepresentable {
    /// `BufferTab::id` of the terminal tab this pane shows.
    let tabId: UInt64
    let palette: TerminalPalette
    /// Core's pane index, so a click can move core's focus to match AppKit's.
    let paneIndex: Int
    let engine: EngineBridge

    func makeNSView(context: Context) -> TerminalHostView {
        let host = TerminalHostView()
        host.autoresizesSubviews = true
        return host
    }

    func updateNSView(_ host: TerminalHostView, context: Context) {
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
        guard host.mount(term) else { return }
        // A pane that appears because the user asked for a terminal should be
        // typable without a second click — but ONLY when core already considers
        // this pane focused. Mounting also happens for reasons the user did not
        // ask for (a font change rebuilds every representable, a split
        // rearranges), and grabbing the keyboard then would take it out of the
        // document they are typing in.
        if engine.editorSplit.focus == paneIndex { host.claimKeyboard(for: term) }
    }

    static func dismantleNSView(_ host: TerminalHostView, coordinator: ()) {
        TerminalHostView.release(host)
    }
}

/// The docked strip's shell (⌃T) — whichever chip is selected.
///
/// The dock's sessions are the registry's own list, not core's, so this asks
/// for one by nothing at all: "the one showing". Switching chips swaps which
/// view is mounted here and leaves both processes running.
struct TerminalDockSurface: NSViewRepresentable {
    let palette: TerminalPalette
    let engine: EngineBridge
    /// Bumped by the chip strip so SwiftUI re-runs `updateNSView` on a switch.
    let activeChip: Int

    func makeNSView(context: Context) -> TerminalHostView {
        let host = TerminalHostView()
        host.autoresizesSubviews = true
        return host
    }

    func updateNSView(_ host: TerminalHostView, context: Context) {
        let sessions = TerminalSessions.shared
        sessions.engine = engine
        guard let session = sessions.dockSession(
            cwd: engine.dockTerminalCwd(), palette: palette
        ) else { return }
        let term = session.view
        term.onFocus = { [weak engine] in engine?.focusTerminal(true) }
        guard host.mount(term) else {
            // Already mounted, so this is not an arrival — and **that is not
            // the same as focused.** The claim above is one-shot: `mount`
            // returns false ever after, so whatever happens to the first
            // responder later, nothing here asked for it again.
            //
            // Something does happen to it. `EditorCanvasView` claims the
            // keyboard from `viewDidMoveToWindow`, which fires whenever a
            // canvas is added to the window — a split, a font change, the
            // debug area opening under it. One steal is permanent: the shell
            // was drawn, was selected, the status line said `keys → shell`,
            // and every keystroke went into the document. That is the whole of
            // "이젠 도킹 터미널에 입력이 안됨", and it is why the previous
            // condition here (`firstResponder === window`, i.e. take it only
            // from NOBODY) could never recover from it.
            //
            // Core's focus is the authority, so this takes the keyboard back
            // from the EDITOR — but never from a field the user deliberately
            // typed into. The find bar, the tree filter and the palette are
            // native text controls precisely so AppKit owns their editing;
            // yanking the keyboard out of one mid-word would be this fix
            // becoming the next bug.
            if engine.focus == .terminal, let window = term.window,
               let responder = window.firstResponder,
               !(responder is TerminalKeySurface),
               !(responder is NSTextView), !(responder is NSTextField)
            {
                window.makeFirstResponder(term)
            }
            return
        }
        // Unlike a pane, the dock only ever appears because the user asked for
        // it — ⌃T, the Debug-area button, or a chip — so it always takes the
        // keyboard on arrival.
        host.claimKeyboard(for: term)
    }

    static func dismantleNSView(_ host: TerminalHostView, coordinator: ()) {
        TerminalHostView.release(host)
    }
}

/// Ends the shells of tabs that have closed.
///
/// A view modifier for the same reason `AudioTabLifetimeModifier` is one: the
/// signal is the chrome republish, which every close route already performs,
/// and `ContentView.body` is at Swift's type-checking limit without it.
///
/// Dock sessions are deliberately not reaped here. They belong to no tab, and
/// hiding the strip is not closing them — that is what ⌃T means now.
struct TerminalTabLifetimeModifier: ViewModifier {
    let engine: EngineBridge

    func body(content: Content) -> some View {
        content.onReceive(engine.$chrome) { _ in
            TerminalSessions.shared.engine = engine
            TerminalSessions.shared.reap { engine.tabIdIsOpen($0) }
        }
    }
}
