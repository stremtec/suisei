import SwiftUI
import AppKit

@main
struct SuiseiApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    /// Single Core face shared by welcome + editor + Settings.
    @StateObject private var engine = EngineBridge()

    init() {
        // Bring up the durable daemon (crash-safe state + LSP/DAP owner) and,
        // through it, the menu-bar status agent. Detached, so it survives us.
        DaemonLauncher.ensureRunning()
    }

    var body: some Scene {
        // MARK: Welcome (Xcode launch window)
        //
        // WWDC24 “Tailor macOS windows with SwiftUI”:
        //   .windowStyle(.plain)  → borderless — no traffic-light chrome strip
        //   .defaultLaunchBehavior(.presented)
        //   .restorationBehavior(.disabled)
        //
        // Close affordance is **not** the red traffic light. Apple’s own samples use
        // SF Symbol `xmark.circle.fill` with monochrome/secondary tint (see WWDC23
        // Inspectors sample / HIG close icons). That is what Xcode’s Welcome shows.
        //
        // Docs:
        //   https://developer.apple.com/videos/play/wwdc2024/10148/
        //   https://developer.apple.com/documentation/SwiftUI/Customizing-window-styles-and-state-restoration-behavior-in-macOS
        Window("Welcome", id: "welcome") {
            WelcomeSceneRoot(engine: engine)
        }
        .defaultSize(
            width: WelcomeView.windowSize.width,
            height: WelcomeView.windowSize.height
        )
        .windowResizability(.contentSize)
        .windowStyle(.plain)
        .windowBackgroundDragBehavior(.enabled)
        .defaultLaunchBehavior(.presented)
        .restorationBehavior(.disabled)
        .commandsRemoved()

        // Finder-shaped, independent Source Control window. A full workbench
        // has its own destinations, navigation history and multi-column detail;
        // embedding it in the editor made it an app-within-an-app and forced us
        // to fake titlebars, source lists and splitters.
        Window("Source Control", id: "git-workbench") {
            GitWorkbenchWindowView(engine: engine)
        }
        .defaultSize(width: 1040, height: 680)
        .defaultPosition(.center)
        .windowResizability(.contentMinSize)
        .restorationBehavior(.disabled)
        .commandsRemoved()

        // MARK: Editor shell
        WindowGroup("Suisei", id: "editor") {
            EditorSceneRoot(engine: engine)
        }
        .defaultSize(width: 1280, height: 820)
        // .hiddenTitleBar (NOT .titleBar): with .titleBar there is always a
        // system titlebar STRIP above the content — the navigator card can only
        // start below it, so the traffic lights + toggle float in a bare band
        // above the card instead of inside its top (the round-9 complaint).
        // Verified: .titleBar also re-asserts titlebarAppearsTransparent=false
        // behind AppKit's back, painting an opaque 32pt band over risen content.
        // .hiddenTitleBar keeps the lights, drops the strip, and lets the card
        // rise to the window edge and swallow them — Xcode 26 anatomy.
        .windowStyle(.hiddenTitleBar)
        .windowResizability(.contentMinSize)
        // Don't open an empty editor until the user leaves Welcome.
        .defaultLaunchBehavior(.suppressed)
        // No window restoration: the app has no document-session restore yet
        // (core `restore_session` is unwired), so SwiftUI would bring back
        // EMPTY untitled shells — and stack them launch over launch (two
        // editors on open, the traffic-light placement race lost in the
        // multi-window chaos). Welcome is disabled for the same reason.
        .restorationBehavior(.disabled)
        .commands {
            suiseiCommands
        }

        // Xcode-style Settings: independent window, sane fixed-ish size.
        Window("Settings", id: "settings") {
            SettingsWindowView(engine: engine)
        }
        .defaultSize(width: 780, height: 520)
        .defaultPosition(.center)
        .windowResizability(.contentMinSize)
        .restorationBehavior(.disabled)
        // Keep the standard titlebar. A root NavigationSplitView integrates
        // its source-list sidebar into this area and AppKit owns the traffic
        // lights; hiding the titlebar forced both back into custom geometry.
        .commandsRemoved()
    }

    @CommandsBuilder
    private var suiseiCommands: some Commands {
        // App-standard menu overrides live in a sub-builder: the top-level
        // commands builder caps at 10 children, and this app has more menus
        // than that (the 11th silently broke the build as "extra argument in
        // call" on the Terminal menu).
        standardCommands

        CommandGroup(after: .sidebar) {
            // Every panel toggle goes through `animatingPanels`. These used to
            // flip the flags raw, so a panel glided when its top-bar button was
            // clicked and snapped when the same action was keyed.
            Button(engine.uiNavVisible ? "Hide Navigator" : "Show Navigator") {
                engine.animatingPanels { engine.uiNavVisible.toggle() }
            }
            .keyboardShortcut("0", modifiers: .command)

            Button(engine.uiInspectorVisible ? "Hide Inspector" : "Show Inspector") {
                engine.animatingPanels { engine.uiInspectorVisible.toggle() }
            }
            .keyboardShortcut("0", modifiers: [.command, .option])

            Button(engine.uiDebugVisible ? "Hide Debug Area" : "Show Debug Area") {
                let next = !engine.uiDebugVisible
                engine.animatingPanels {
                    withAnimation(.spring(duration: 0.3, bounce: 0.12)) {
                        engine.setDebugArea(next)
                    }
                }
            }
            .keyboardShortcut("y", modifiers: [.command, .shift])

            Toggle("Minimap", isOn: minimapBinding)
        }

        // Workspace destinations. Keep this distinct from macOS' built-in
        // View menu so the menu bar never contains two adjacent “View” items.
        CommandMenu("Workspace") {
            Button("File Explorer") {
                NotificationCenter.default.post(name: .suiseiNavProject, object: nil)
                engine.animatingPanels { engine.uiNavVisible = true }
            }
            .keyboardShortcut("f", modifiers: .control)

            Button("Source Control") {
                // Load before the slide, never during it: these are engine
                // recomposes and chrome pulls, and on the animation's first
                // frame they are exactly what makes the panel hitch on open.
                engine.ensureScm()
                NotificationCenter.default.post(name: .suiseiNavScm, object: nil)
                engine.animatingPanels { engine.uiNavVisible = true }
            }
            .keyboardShortcut("g", modifiers: .control)

            Button("Find Navigator") {
                NotificationCenter.default.post(name: .suiseiNavFind, object: nil)
                engine.animatingPanels { engine.uiNavVisible = true }
            }

            Button("Breakpoints") {
                engine.refreshBreakpoints()
                NotificationCenter.default.post(name: .suiseiNavBreakpoints, object: nil)
                engine.animatingPanels { engine.uiNavVisible = true }
            }

            Divider()

            Button("Git Workbench") {
                NotificationCenter.default.post(name: .suiseiOpenGitWorkbenchWindow, object: nil)
            }
            .keyboardShortcut("g", modifiers: [.control, .shift])

            Button("Pretty Preview") {
                engine.togglePreview()
            }
            .keyboardShortcut("v", modifiers: [.command, .shift])
        }

        CommandMenu("Navigate") {
            Button("Go to File…") {
                engine.openFilePalette()
            }
            .keyboardShortcut("p", modifiers: .command)

            Button("Command Palette…") {
                engine.openCommandPalette()
            }
            .keyboardShortcut("p", modifiers: [.command, .shift])

            Divider()

            Button("Go to Definition") {
                engine.gotoDefinition()
            }
            .keyboardShortcut("j", modifiers: [.command, .control])

            Button("Find All References") {
                engine.requestReferences()
            }
            .keyboardShortcut("r", modifiers: [.command, .shift])

            Button("Rename Symbol…") {
                engine.promptRenameSymbol()
            }
            .keyboardShortcut("r", modifiers: [.command, .control])
        }

        // Editor — LSP / edit actions reachable from the menu bar.
        CommandMenu("Editor") {
            Button("Format Document") {
                engine.formatDocument()
            }
            .keyboardShortcut("i", modifiers: [.command, .shift])

            Button("Code Actions…") {
                engine.requestCodeActions()
            }
            .keyboardShortcut(".", modifiers: .command)

            Divider()

            Button("New Untitled Tab") {
                engine.openBlankTab()
            }

            Button("Next Tab") {
                engine.nextTab()
            }

            Button("Previous Tab") {
                engine.prevTab()
            }

            Divider()

            Button("Split Editor Right") {
                engine.splitEditorRight()
            }
            .disabled(engine.editorSplit.panes.count >= 4)

            Button("Split Editor Below") {
                engine.splitEditorBelow()
            }
            .disabled(engine.editorSplit.panes.count >= 4)

            Button("Focus Next Pane") {
                engine.focusNextPane()
            }
            .disabled(!engine.editorSplit.isSplit)

            Button("Close Focused Pane") {
                engine.closeFocusedPane()
            }
            .disabled(!engine.editorSplit.isSplit)

            Divider()

            Button("Save Split as Layout Tab") {
                engine.foldLayout()
            }
            .disabled(!engine.editorSplit.isSplit || engine.hasActiveLayout)

            Button("Unfold Active Layout") {
                engine.unfoldLayout()
            }
            .disabled(!engine.hasActiveLayout)

            Divider()

            Button("Larger Text") {
                engine.zoomFont(delta: 1)
            }
            .keyboardShortcut("+", modifiers: .command)

            Button("Smaller Text") {
                engine.zoomFont(delta: -1)
            }
            .keyboardShortcut("-", modifiers: .command)

            Button("Reset Text Size") {
                engine.resetFontZoom()
            }
            .keyboardShortcut("0", modifiers: [.command, .control])
        }

        // Terminal — shell surfaces (not only buried chords).
        CommandMenu("Terminal") {
            Button("Toggle Debug Area") {
                engine.setDebugArea(!engine.uiDebugVisible)
            }
            .keyboardShortcut("y", modifiers: [.command, .shift])

            Button("New Terminal Window") {
                engine.toggleTerminalFull()
            }
            .keyboardShortcut("t", modifiers: [.command, .shift])
        }
    }

    @CommandsBuilder
    private var standardCommands: some Commands {
        CommandGroup(replacing: .appInfo) {
            Button("About Suisei") {
                AboutPanelController.shared.show()
            }
        }

        CommandGroup(replacing: .appSettings) {
            Button("Settings…") {
                engine.openSettings()
                openSettingsWindow()
            }
            .keyboardShortcut(",", modifiers: .command)
        }

        CommandGroup(replacing: .newItem) {
            Button("New Untitled…") {
                engine.createNewProject()
            }
            .keyboardShortcut("n", modifiers: .command)

            Button("Open…") {
                engine.openProjectFolder()
            }
            .keyboardShortcut("o", modifiers: .command)

            Button("Clone Git Repository…") {
                engine.cloneGitRepository()
            }
        }

        CommandGroup(replacing: .saveItem) {
            Button("Save") {
                engine.save()
            }
            .keyboardShortcut("s", modifiers: .command)

            Button("Save As…") {
                engine.saveAsPanel()
            }
            .keyboardShortcut("s", modifiers: [.command, .shift])
        }

        CommandGroup(replacing: .undoRedo) {
            Button("Undo") { engine.undoCommand() }
                .keyboardShortcut("z", modifiers: .command)
            Button("Redo") { engine.redoCommand() }
                .keyboardShortcut("z", modifiers: [.command, .shift])
        }

        CommandGroup(replacing: .pasteboard) {
            Button("Cut") { engine.cutCommand() }
            .keyboardShortcut("x", modifiers: .command)
            Button("Copy") { engine.copyCommand() }
            .keyboardShortcut("c", modifiers: .command)
            Button("Paste") { engine.pasteCommand() }
            .keyboardShortcut("v", modifiers: .command)
            Button("Select All") { engine.selectAllCommand() }
                .keyboardShortcut("a", modifiers: .command)
        }

        CommandGroup(replacing: .textEditing) {
            Button("Find…") { engine.openFind() }
                .keyboardShortcut("f", modifiers: .command)
            Button("Find Next") { engine.findStep(forward: true) }
                .keyboardShortcut("g", modifiers: .command)
            Button("Find Previous") { engine.findStep(forward: false) }
                .keyboardShortcut("g", modifiers: [.command, .shift])
            Button("Find in Project…") {
                NotificationCenter.default.post(name: .suiseiNavFind, object: nil)
                engine.animatingPanels { engine.uiNavVisible = true }
            }
            .keyboardShortcut("f", modifiers: [.command, .shift])
        }
    }

    private var minimapBinding: Binding<Bool> {
        Binding(
            get: { UserDefaults.standard.object(forKey: "suisei.minimap") as? Bool ?? true },
            set: { UserDefaults.standard.set($0, forKey: "suisei.minimap") }
        )
    }

    private func openSettingsWindow() {
        NotificationCenter.default.post(name: .suiseiOpenSettingsWindow, object: nil)
    }
}

// MARK: - Scene roots (window chrome via SwiftUI, not AppKit hacks)

/// Welcome launch window — plain chrome + system-symbol dismiss (Xcode pattern).
private struct WelcomeSceneRoot: View {
    @ObservedObject var engine: EngineBridge
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var recents: [RecentItem] = RecentStore.load()

    private let panelBg = Color(red: 0.13, green: 0.13, blue: 0.135)

    var body: some View {
        WelcomeView(
            onCreate: {
                engine.createNewProject()
                recents = RecentStore.load()
                promoteToEditor()
            },
            onOpen: {
                engine.openProjectFolder()
                recents = RecentStore.load()
                promoteToEditorIfLeftWelcome()
            },
            onClone: {
                engine.cloneGitRepository()
                recents = RecentStore.load()
                promoteToEditorIfLeftWelcome()
            },
            onOpenRecent: { path in
                var isDir: ObjCBool = false
                FileManager.default.fileExists(atPath: path, isDirectory: &isDir)
                if isDir.boolValue {
                    engine.setProjectRoot(path)
                }
                engine.openPath(path)
                recents = RecentStore.load()
                promoteToEditor()
            },
            onClose: {
                dismissWindow(id: "welcome")
                // No editor open → quit (launch sheet closed).
                DispatchQueue.main.async {
                    if !NSApp.windows.contains(where: { $0.isVisible && $0.title != "Welcome" }) {
                        NSApp.terminate(nil)
                    }
                }
            },
            recents: recents,
            // Launch warmup sequence (app-boot tier). Grammar warming is real
            // now — the rest of the pipeline (project index, LSP, git) attaches
            // at project-open per docs/SUISEI-EDIT-ARCHITECTURE.md §3. Engine
            // calls hop to the main actor so they serialise with the tick.
            bootStages: [
                BootStage(label: "Preparing editor") { Boot.warmEditorGlyphs() },
                BootStage(label: "Loading grammars") { await MainActor.run { engine.warmGrammars() } },
                BootStage(label: "Restoring session") { Boot.primeRecents() },
            ]
        )
        .frame(
            width: WelcomeView.windowSize.width,
            height: WelcomeView.windowSize.height
        )
        // Fixed panel — no stretch / no fullscreen elongation.
        .fixedSize()
        .windowResizeBehavior(.disabled)
        .windowFullScreenBehavior(.disabled)
        // Clear host so continuous corners + shadow show (content paints the fill).
        .containerBackground(.clear, for: .window)
        .preferredColorScheme(.dark)
        .onAppear {
            recents = RecentStore.load()
            engine.activateInput()
            // Re-apply AppKit chrome after the NSWindow exists (async host attach).
            DispatchQueue.main.async {
                WelcomeChromeApplier.applyToWelcomeWindows()
            }
            // Second pass once SwiftUI finishes laying out the hosting view.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                WelcomeChromeApplier.applyToWelcomeWindows()
            }
        }
        .onChange(of: engine.chrome.welcome) { _, welcome in
            if !welcome {
                promoteToEditor()
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiRecoveryAccepted)) { _ in
            promoteToEditor()
        }
        // ── Shadow WAL recovery sheet ──────────────────────────────────
        // Presented when the engine found unsaved work from a previous crash.
        // Each row shows the file path; Accept opens it with the recovered
        // buffer, Discard deletes the WAL entry permanently.
        // Presented only when there is something to recover.
        //
        // Observed at launch: the sheet came up reading "recovered unsaved work
        // in 0 files" over an empty list, with its own default action disabled.
        // `checkRecovery` sets the flag from a WAL entry count and then builds
        // the row list separately, so an entry whose path cannot be read leaves
        // the flag true and the list empty. Gating presentation on the list
        // itself makes the empty sheet unrepresentable rather than merely
        // unlikely.
        .sheet(
            isPresented: Binding(
                get: { engine.recoverySheetShown && !engine.recoveryEntries.isEmpty },
                set: { engine.recoverySheetShown = $0 }
            )
        ) {
            RecoverySheet(engine: engine)
        }
    }

    private func promoteToEditorIfLeftWelcome() {
        if !engine.chrome.welcome {
            promoteToEditor()
        }
    }

    private func promoteToEditor() {
        // Idempotent: the welcome→false flip can arrive more than once (an
        // editor opened early seeds the project, which ends welcome, which
        // fires this again), and each arrival used to open ANOTHER editor.
        let editorExists = NSApp.windows.contains {
            $0.identifier?.rawValue.hasPrefix("editor") ?? false
        }
        if !editorExists {
            openWindow(id: "editor")
        }
        dismissWindow(id: "welcome")
        DispatchQueue.main.async {
            SuiseiWindowLayout.apply(welcome: false, animate: true)
        }
    }
}

/// Main IDE window.
private struct EditorSceneRoot: View {
    @ObservedObject var engine: EngineBridge

    var body: some View {
        ContentView(engine: engine)
            .onAppear {
                engine.activateInput()
                // If Core is still on Welcome (edge case), open launch window instead.
                if engine.chrome.welcome {
                    // Keep editor suppressed path — user should use Welcome.
                } else {
                    SuiseiWindowLayout.apply(welcome: false, animate: false)
                }
            }
    }
}

extension Notification.Name {
    static let suiseiOpenSettingsWindow = Notification.Name("suisei.openSettingsWindow")
    static let suiseiOpenGitWorkbenchWindow = Notification.Name("suisei.openGitWorkbenchWindow")
    static let suiseiNavProject = Notification.Name("suisei.nav.project")
    static let suiseiNavScm = Notification.Name("suisei.nav.scm")
    static let suiseiNavFind = Notification.Name("suisei.nav.find")
    static let suiseiNavBreakpoints = Notification.Name("suisei.nav.breakpoints")
    static let suiseiNewUntitledTab = Notification.Name("suisei.newUntitledTab")
    /// Navigator title-row commands, executed inside `ProjectTreeView`
    /// (that is where the tree state — expansion, inline rename — lives).
    static let suiseiNavNewFile = Notification.Name("suisei.nav.newFile")
    static let suiseiNavNewFolder = Notification.Name("suisei.nav.newFolder")
    static let suiseiNavCollapseAll = Notification.Name("suisei.nav.collapseAll")
    static let suiseiRecoveryAccepted = Notification.Name("suisei.recoveryAccepted")
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        // Welcome window is presented by Scene.defaultLaunchBehavior(.presented).
        // Do not force AppKit traffic-light hacks here.
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

// MARK: - Shadow WAL recovery sheet

/// Crash-recovery sheet: lists unsaved buffers found in the WAL journal,
/// lets the user accept (open with recovered content) or discard each.
/// Built to macOS sheet conventions rather than to a private palette.
///
/// The previous version hard-coded a dark panel, wrote every foreground as
/// `.white.opacity(…)`, and then forced `.preferredColorScheme(.dark)` so the
/// numbers would hold — which meant it ignored the user's appearance entirely
/// and looked pasted-in on a light Mac. It also put TWO buttons on every row,
/// so a five-file recovery presented ten competing actions and no default, and
/// it was pinned to 460×320 whether it held one entry or twenty.
///
/// What a Mac sheet does instead: semantic colours so it follows the system;
/// one clear default action; destructive actions kept away from it; and a
/// height that follows the content between sensible bounds.
struct RecoverySheet: View {
    @ObservedObject var engine: EngineBridge

    /// Per-row destructive action, confirmed before it runs. Discarding a
    /// recovery deletes the only copy of that work.
    @State private var pendingDiscardAll = false

    private var entries: [EngineBridge.RecoveryItem] { engine.recoveryEntries }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            list
            Divider()
            footer
        }
        .frame(width: 480)
        // Follows the content instead of being pinned: one recovered file no
        // longer sits in a half-empty panel.
        .frame(minHeight: 240, maxHeight: 460)
        .background(.regularMaterial)
        .confirmationDialog(
            "Discard all recovered changes?",
            isPresented: $pendingDiscardAll
        ) {
            Button("Discard All", role: .destructive) {
                engine.discardAllRecovery()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This permanently deletes the unsaved work in all \(entries.count) files. It cannot be undone.")
        }
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: "clock.arrow.circlepath")
                .symbolRenderingMode(.hierarchical)
                .font(.system(size: 26, weight: .regular))
                // Semantic, so it stays legible in both appearances.
                .foregroundStyle(Color.accentColor)
                .frame(width: 30)
            VStack(alignment: .leading, spacing: 4) {
                Text("Unsaved Changes Found")
                    .font(.system(size: 13, weight: .semibold))
                Text(entries.count == 1
                     ? "Suisei recovered unsaved work in 1 file from a previous session."
                     : "Suisei recovered unsaved work in \(entries.count) files from a previous session.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 20)
        .padding(.top, 20)
        .padding(.bottom, 16)
    }

    private var list: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                let items = entries
                ForEach(items.indices, id: \.self) { index in
                    row(items[index])
                    if index < items.count - 1 {
                        Divider().padding(.leading, 42)
                    }
                }
            }
        }
        // Zebra striping and per-row chrome both went; the list reads as one
        // surface, which is what makes the file names scannable.
        .background(Color(nsColor: .textBackgroundColor).opacity(0.5))
    }

    private func row(_ item: EngineBridge.RecoveryItem) -> some View {
        HStack(spacing: 10) {
            Image(systemName: "doc.text")
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .frame(width: 18)
            VStack(alignment: .leading, spacing: 1) {
                Text(item.name)
                    .font(.system(size: 12, weight: .medium))
                Text(item.path)
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 8)
            // One action per row, and it is the non-destructive one. Discarding
            // a single file is rare enough to live behind the ⌫ affordance
            // rather than competing with Recover on every line.
            Button("Recover") { engine.acceptRecovery(item) }
                .controlSize(.small)
            Button {
                engine.discardRecovery(item)
            } label: {
                Image(systemName: "trash")
            }
            .controlSize(.small)
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
            .help("Discard this file's recovered changes")
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 8)
    }

    private var footer: some View {
        HStack(spacing: 12) {
            Button("Discard All…") { pendingDiscardAll = true }
            Spacer()
            Button("Later") { engine.recoverySheetShown = false }
                .keyboardShortcut(.cancelAction)
            // The default action, so Return does the safe thing.
            Button("Recover All") {
                for item in entries { engine.acceptRecovery(item) }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(entries.isEmpty)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
    }
}
