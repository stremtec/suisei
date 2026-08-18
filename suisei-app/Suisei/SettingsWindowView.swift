import SwiftUI
import AppKit

/// Settings — modern macOS System Settings look: icon-tile sidebar, grouped
/// forms, live theme preview. Values come from Core rows (single source).
struct SettingsWindowView: View {
    @ObservedObject var engine: EngineBridge
    @ObservedObject private var accountStore = EngineBridge.shared.githubAccount
    @Environment(\.dismiss) private var dismiss
    @State private var engineReferenceExpanded = false
    @State private var searchText = ""
    /// Shortcuts' own filter. Separate from `searchText`, which filters the
    /// sidebar's pages — Xcode's Shortcuts pane has its own Filter field too.
    @State private var shortcutFilter = ""
    /// The catalogue, pulled. See `EngineBridge.keyBindings()` for why it is not
    /// on `chrome`.
    @State private var keyBindings: [KeyBindingItem] = []
    /// The command whose key field is armed, if any. One at a time — a second
    /// armed field would race the first for the key press.
    @State private var recordingCommand: String?
    /// Why the last chord was refused. Cleared the moment another is offered.
    @State private var shortcutError: String?
    /// Which syntax category the Themes page's editing row is pointed at.
    /// A key, not an index — see `selectedToken`.
    @State private var selectedTokenKey = "fg"
    @State private var savingTheme = false
    @State private var newThemeName = ""
    @State private var saveThemeError = ""
    @State private var themeToDelete: String?
    @State private var selectedPageID: PageID = .general
    @State private var pageHistory: [PageID] = [.general]
    @State private var historyIndex = 0
    @State private var confirmSignOut = false
    // Minimap. Face-side preferences, not Core rows: Core neither draws the
    // strip nor knows it exists. They take effect on change, like every
    // control in System Settings — the Apply bar below belongs to Core's
    // config file and stays away unless a Core row is dirty.
    @AppStorage("suisei.minimap") private var minimapEnabled = true
    @AppStorage("suisei.minimap.allPanes") private var minimapAllPanes = false
    @AppStorage("suisei.minimap.proportional") private var minimapProportional = false
    /// Line spacing and caret shape, stored as their raw values so the enum can
    /// gain a case without a defaults migration. `EditorMetrics` reads the same
    /// keys — these bindings exist to redraw the window, not to own the value.
    @AppStorage("suisei.lineSpacing") private var lineSpacing = "normal"
    @AppStorage("suisei.cursorStyle") private var cursorStyle = "bar"

    private static let sidebarWidth: CGFloat = 240

    private var s: SettingsSnap { engine.chrome.settings }
    private var theme: ThemeSnap { engine.chrome.theme }

    /// The highlight the user actually chose, not the system's.
    ///
    /// `Color.accentColor` is the SYSTEM accent and `.tint()` does not redirect
    /// it — so every ring and swatch here stayed blue while Appearance's own
    /// highlight setting said otherwise, which is a confusing thing for the
    /// window that sets it. `theme.accent` already carries the choice: Core
    /// builds the theme through `theme::with_highlight(t, cfg.highlight_color)`.
    private var liveAccent: Color { theme.color(theme.accent) }

    private var isLightTheme: Bool {
        let c = theme.editorBg
        let r = Double((c >> 16) & 0xFF)
        let g = Double((c >> 8) & 0xFF)
        let b = Double(c & 0xFF)
        return (0.299 * r + 0.587 * g + 0.114 * b) > 150
    }

    private var preferredScheme: ColorScheme? {
        switch appearanceMode {
        case "light": .light
        case "dark": .dark
        default: nil
        }
    }

    /// Which of the Color Scheme tiles is on — or `custom`, meaning none is.
    ///
    /// `config.theme` is one field with three meanings: `system`, the two
    /// built-in palettes `light`/`dark`, and any other catalogue name. Core's
    /// `value_index` for AppearanceMode folds that last case into 0, so pinning
    /// Ocean lit the **Automatic** tile — the window claimed to be following
    /// macOS while showing a palette macOS never asked for. The theme rows
    /// carry the truth (Core marks the active one with `●`), so ask them.
    private var appearanceMode: String {
        if pinnedThemeName != nil { return "custom" }
        guard let index = rows(.appearanceMode).first?.valueIndex else { return "system" }
        switch index {
        case 1: return "light"
        case 2: return "dark"
        default: return "system"
        }
    }

    /// The theme pinned right now, when it is not one the Color Scheme tiles
    /// already answer for.
    ///
    /// Asked of Core rather than recovered by looking for the `●` Core writes
    /// into a row's label. That marker only exists for catalogue rows, so with
    /// a user-made theme selected nothing was marked, this returned nil, and a
    /// Color Scheme tile lit up claiming to follow macOS.
    private var pinnedThemeName: String? {
        let current = engine.selectedTheme
        if current.isEmpty || current == "system" || current == "light" || current == "dark" {
            return nil
        }
        return current
    }

    private var glassStyle: String {
        rows(.glassStyle).first?.valueIndex == 1 ? "tinted" : "clear"
    }

    private enum PageID: String, Hashable {
        case accounts
        case accountProfile
        case accountSecurity
        case general
        case themes
        case editor
        case languageServers
        case sourceControl
        case extensions
        case shortcuts
        case softwareUpdate
        case softwareUpdateAutomatic
        case softwareUpdateBeta

        var isAccountFamily: Bool {
            switch self {
            case .accounts, .accountProfile, .accountSecurity: return true
            default: return false
            }
        }

        var isSoftwareUpdateFamily: Bool {
            switch self {
            case .softwareUpdate, .softwareUpdateAutomatic, .softwareUpdateBeta: return true
            default: return false
            }
        }
    }

    private struct Page: Identifiable {
        let id: PageID
        let title: String
        let symbol: String
        let searchTerms: String
        let corePage: Int
    }

    private struct PresentedSettingGroup: Identifiable {
        let id: String
        let title: String
        var rows: [SettingsRowItem]
    }

    private struct AccentPreset: Identifiable {
        let name: String
        let hex: String
        let color: Color
        var id: String { hex }
    }

    /// Pages are deliberately task-oriented rather than mirroring Core's four
    /// implementation pages. Several sidebar destinations share Core page 1,
    /// but present a much smaller, focused settings surface.
    private let pages: [Page] = [
        Page(
            id: .accounts, title: "GitHub Account", symbol: "person.crop.circle",
            searchTerms: "account github sign in profile avatar token", corePage: 1
        ),
        Page(
            id: .general, title: "General", symbol: "gearshape",
            searchTerms:
                "about version build appearance light dark automatic glass clipboard undo",
            corePage: 1
        ),
        Page(
            id: .themes, title: "Themes", symbol: "paintpalette",
            searchTerms: "theme palette syntax colour color accent highlight preview", corePage: 1
        ),
        Page(
            id: .editor, title: "Editor", symbol: "square.and.pencil",
            searchTerms:
                "editing wrap line numbers relative tab width clipboard undo minimap", corePage: 1
        ),
        Page(
            id: .languageServers, title: "Language Servers", symbol: "square.stack.3d.up",
            searchTerms: "lsp language server command completion diagnostics", corePage: 1
        ),
        Page(
            id: .sourceControl, title: "Source Control", symbol: "arrow.triangle.branch",
            searchTerms: "git scm repository workbench commit branch", corePage: 1
        ),
        Page(
            id: .extensions, title: "Extensions", symbol: "puzzlepiece.extension",
            searchTerms: "extension language syntax grammar vscode", corePage: 2
        ),
        Page(
            id: .shortcuts, title: "Shortcuts", symbol: "keyboard",
            searchTerms: "keyboard key binding command", corePage: 3
        ),
        Page(
            id: .softwareUpdate, title: "Software Update", symbol: "arrow.triangle.2.circlepath",
            searchTerms: "software update version release beta automatic install", corePage: 1
        ),
    ]

    private var visiblePages: [Page] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return pages }
        return pages.filter {
            $0.title.localizedCaseInsensitiveContains(query)
                || $0.searchTerms.localizedCaseInsensitiveContains(query)
        }
    }

    /// Page selection as a binding, so the sidebar can be a real `List`.
    ///
    /// The selection still lives in Core (`settingsGotoPage`); this only adapts
    /// it to the shape `List(selection:)` wants. Rows used to be hand-rolled
    /// `Button`s, which meant no keyboard navigation, no focus ring, and a
    /// selection drawn as a 14%-accent wash instead of the filled capsule every
    /// other Mac sidebar uses.
    private var currentPage: Page {
        switch selectedPageID {
        case .accountProfile:
            return Page(
                id: .accountProfile, title: "Profile", symbol: "person.crop.circle",
                searchTerms: "", corePage: 1
            )
        case .accountSecurity:
            return Page(
                id: .accountSecurity, title: "Sign-In & Security", symbol: "lock",
                searchTerms: "", corePage: 1
            )
        case .softwareUpdateAutomatic:
            return Page(
                id: .softwareUpdateAutomatic, title: "Automatic Updates",
                symbol: "arrow.triangle.2.circlepath", searchTerms: "", corePage: 1
            )
        case .softwareUpdateBeta:
            return Page(
                id: .softwareUpdateBeta, title: "Beta Updates",
                symbol: "hammer", searchTerms: "", corePage: 1
            )
        default:
            return pages.first(where: { $0.id == selectedPageID }) ?? pages[0]
        }
    }

    private var selectedPage: Binding<PageID?> {
        Binding(
            get: {
                if selectedPageID.isAccountFamily { return .accounts }
                if selectedPageID.isSoftwareUpdateFamily { return .softwareUpdate }
                return selectedPageID
            },
            set: { if let id = $0 { navigate(to: id) } }
        )
    }

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            settingsDetail
        }
        .navigationTitle(currentPage.title)
        .confirmationDialog(
            "Sign out of GitHub?",
            isPresented: $confirmSignOut,
            titleVisibility: .visible
        ) {
            Button("Sign Out", role: .destructive) { engine.githubSignOut() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Suisei will stop using this GitHub identity. Local repositories stay on disk.")
        }
        .toolbarTitleDisplayMode(.inlineLarge)
        .toolbar {
            ToolbarItem(placement: .navigation) {
                historyControl
            }
        }
        // Keep the split view responsible for the entire resizable window so
        // the sidebar material and the detail background both reach the bottom.
        .frame(minWidth: 780, idealWidth: 780, maxWidth: 780, minHeight: 520)
        .preferredColorScheme(preferredScheme)
        .tint(theme.color(theme.accent))
        .accentColor(theme.color(theme.accent))
        .background(
            ThemedWindowChrome(
                background: NSColor(theme.windowBg),
                light: preferredScheme == .light || (preferredScheme == nil && isLightTheme),
                identifier: WindowChrome.settingsIdentifier
            )
        )
        .onAppear {
            // A `Window` scene may build this view while its Core model is
            // still closed. In that case `settingsGotoPage` intentionally
            // ignores the request; the real open transition below will sync
            // the page once the model is ready.
            if s.open {
                navigate(to: selectedPageID, recordingHistory: false)
            }
            retheme()
        }
        .onChange(of: engine.chrome.settings.open) { _, open in
            if open {
                // Scene creation and Core opening are independent. Always
                // re-publish the visible native destination at the moment the
                // Core panel becomes available, otherwise a pre-created
                // Settings scene can remain backed by About/empty rows.
                navigate(to: selectedPageID, recordingHistory: false)
                retheme()
            } else {
                dismiss()
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { note in
            guard let window = note.object as? NSWindow,
                  window.identifier == WindowChrome.settingsIdentifier else { return }
            if engine.chrome.settings.open { engine.closeSettings() }
        }
        .onChange(of: appearanceMode) { _, _ in retheme() }
    }

    /// A native source-list sidebar. The `List` owns the sidebar material;
    /// adding a second effect view inside the column creates an inset card
    /// below the titlebar safe area instead of one full-height surface.
    private var sidebar: some View {
        List(selection: selectedPage) {
            if visiblePages.contains(where: { $0.id == .accounts }) {
                Section {
                    accountSidebarRow.tag(PageID.accounts)
                }
            }
            ForEach(visiblePages.filter { $0.id != .accounts }) { page in
                sidebarRow(page).tag(page.id)
            }
        }
        .listStyle(.sidebar)
        // macOS 26's native pinned-bar path: the list is inset at rest, then
        // scrolls below the search field with the same soft edge blur used by
        // System Settings. This replaces the old fake spacer row and manual
        // rectangular overlay, both of which left a visible horizontal seam.
        .scrollEdgeEffectStyle(.soft, for: .top)
        .safeAreaBar(edge: .top, spacing: 0) {
            sidebarSearchField
        }
        .navigationSplitViewColumnWidth(
            min: Self.sidebarWidth,
            ideal: Self.sidebarWidth,
            max: Self.sidebarWidth
        )
        .toolbar(removing: .sidebarToggle)
        // A toolbar participant makes SwiftUI use the modern full-height
        // sidebar/titlebar arrangement on macOS 26. It is intentionally
        // invisible and carries no interaction or accessibility surface.
        //
        // Sized, because a bare `Rectangle` has no intrinsic size and expands
        // to whatever it is offered — and in a `NavigationSplitView` this
        // shares the LEADING toolbar region with the root's
        // `.navigation`-placed history control. A participant that only has
        // to exist should not also be claiming that region's width.
        .toolbar {
            Rectangle()
                .frame(width: 1, height: 1)
                .hidden()
                .accessibilityHidden(true)
        }
    }

    /// System Settings keeps search inside the source-list column, below the
    /// traffic lights. `searchable` is intentionally not used here: in a
    /// two-column settings window SwiftUI promotes it to the detail toolbar,
    /// even when its requested placement is `.sidebar`.
    private var sidebarSearchField: some View {
        NativeSidebarSearchField(text: $searchText)
        .frame(height: 28)
        .padding(.horizontal, 10)
        .padding(.top, 8)
        .padding(.bottom, 10)
    }

    /// One sidebar row. Hoisted out of the `List` builder — inline, the tile's
    /// modifier chain pushed the whole split view past what the type-checker
    /// will solve in reasonable time.
    private func sidebarRow(_ page: Page) -> some View {
        Label {
            Text(page.title)
        } icon: {
            sidebarIcon(for: page, size: 20)
        }
    }

    /// System Settings puts the signed-in identity first: a circular photo,
    /// the person's name, and a small "GitHub Account" caption. Signed out
    /// keeps the same row so the destination does not jump.
    private var accountSidebarRow: some View {
        let snap = accountStore.snap
        return HStack(spacing: 8) {
            GitHubAvatarView(image: accountStore.avatar, size: 36, signedIn: snap.isSignedIn)
            VStack(alignment: .leading, spacing: 1) {
                Text(snap.isSignedIn ? snap.displayName : "GitHub Account")
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                Text(snap.isSignedIn ? "GitHub Account" : (snap.isMissingCLI ? "Not Installed" : "Sign In"))
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 4)
    }

    /// A flat monochrome glyph — Xcode's Settings sidebar, not System
    /// Settings'.
    ///
    /// These were briefly filled, tinted squircles, which is the System
    /// Settings idiom. Suisei is an editor and sits beside Xcode, whose
    /// Settings sidebar is outline symbols in one colour with a filled accent
    /// capsule marking the selection. Copying the wrong Apple window is still
    /// copying the wrong window.
    private func sidebarIcon(for page: Page, size: CGFloat) -> some View {
        Image(systemName: page.symbol)
            .symbolRenderingMode(.monochrome)
            .font(.system(size: size * 0.68, weight: .regular))
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }

    private var historyControl: some View {
        ControlGroup {
            Button {
                historyIndex -= 1
                navigate(to: pageHistory[historyIndex], recordingHistory: false)
            } label: {
                Image(systemName: "chevron.backward")
            }
            .disabled(historyIndex == 0)
            .help("Back")

            Button {
                historyIndex += 1
                navigate(to: pageHistory[historyIndex], recordingHistory: false)
            } label: {
                Image(systemName: "chevron.forward")
            }
            .disabled(historyIndex >= pageHistory.count - 1)
            .help("Forward")
        }
        .controlGroupStyle(.navigation)
        .accessibilityElement(children: .contain)
    }

    private func navigate(to id: PageID, recordingHistory: Bool = true) {
        let page: Page
        switch id {
        case .accountProfile:
            page = Page(
                id: .accountProfile, title: "Profile", symbol: "person.crop.circle",
                searchTerms: "", corePage: 1
            )
        case .accountSecurity:
            page = Page(
                id: .accountSecurity, title: "Sign-In & Security", symbol: "lock",
                searchTerms: "", corePage: 1
            )
        case .softwareUpdateAutomatic:
            page = Page(
                id: .softwareUpdateAutomatic, title: "Automatic Updates",
                symbol: "arrow.triangle.2.circlepath", searchTerms: "", corePage: 1
            )
        case .softwareUpdateBeta:
            page = Page(
                id: .softwareUpdateBeta, title: "Beta Updates",
                symbol: "hammer", searchTerms: "", corePage: 1
            )
        default:
            guard let listed = pages.first(where: { $0.id == id }) else { return }
            page = listed
        }

        if recordingHistory, id != selectedPageID {
            pageHistory = Array(pageHistory.prefix(historyIndex + 1))
            pageHistory.append(id)
            historyIndex = pageHistory.count - 1
        }

        selectedPageID = id
        // Always publish the requested Core page. During a newly-created scene
        // `s` can still be the previous/empty snapshot for one SwiftUI pass;
        // using that stale page index as a guard left the native shell visible
        // but backed by About/empty rows. The Core operation is idempotent and
        // is the correct authority for this transition.
        engine.settingsGotoPage(page.corePage)
        if id.isAccountFamily {
            engine.refreshGitHubAccount()
        }
    }

    /// Rows of one kind, in Core's order.
    private func rows(_ kind: SettingKind) -> [SettingsRowItem] {
        s.rows.filter { $0.kind == kind }
    }

    private func presentedRows(
        on page: SettingSurfacePage,
        advanced: Bool? = nil
    ) -> [SettingsRowItem] {
        s.rows.filter { row in
            guard !row.isHeader, row.control != .none, row.page == page else { return false }
            return advanced.map { row.advanced == $0 } ?? true
        }
    }

    private func presentedGroups(
        on page: SettingSurfacePage,
        advanced: Bool = false
    ) -> [PresentedSettingGroup] {
        var groups: [PresentedSettingGroup] = []
        for row in presentedRows(on: page, advanced: advanced) {
            if let index = groups.firstIndex(where: { $0.title == row.group }) {
                groups[index].rows.append(row)
            } else {
                groups.append(PresentedSettingGroup(
                    id: "\(page.rawValue):\(row.group):\(advanced)",
                    title: row.group,
                    rows: [row]
                ))
            }
        }
        return groups
    }

    // MARK: - Detail

    private var settingsDetail: some View {
        VStack(spacing: 0) {
            // Account pages own their Form so the Apple Account hero can sit
            // above the grouped rows. Everything else stays a single grouped
            // Form — nesting a ScrollView around it used to grow a second
            // scroller whose insets never lined up with the groups.
            Group {
                switch selectedPageID {
                case .softwareUpdate:
                    SoftwareUpdatePage(
                        store: EngineBridge.shared.softwareUpdate,
                        automaticUpdates: rows(.updateCheck).first,
                        onOpenAutomatic: { navigate(to: .softwareUpdateAutomatic) },
                        onOpenBeta: { navigate(to: .softwareUpdateBeta) },
                        onCheckNow: { engine.checkForSoftwareUpdate() },
                        onInfo: { engine.openSoftwareUpdateNotes() }
                    )
                case .softwareUpdateAutomatic:
                    SoftwareUpdateAutomaticPage(
                        automaticUpdates: rows(.updateCheck).first,
                        onSetAutomatic: { on in
                            if let row = rows(.updateCheck).first {
                                engine.settingsSetValue(row.id, value: on ? 1 : 0)
                            }
                        }
                    )
                case .softwareUpdateBeta:
                    SoftwareUpdateBetaPage(store: EngineBridge.shared.softwareUpdate)
                case .accounts:
                    GitHubAccountRootPage(
                        store: accountStore,
                        accent: liveAccent,
                        onOpenProfile: { navigate(to: .accountProfile) },
                        onOpenSecurity: { navigate(to: .accountSecurity) },
                        onSignIn: { engine.githubSignIn() },
                        onCancel: { engine.githubCancelSignIn() },
                        onSignOut: { confirmSignOut = true },
                        onRefresh: { engine.refreshGitHubAccount() },
                        onInstall: { engine.githubInstallDocs() },
                        onOpenGitHub: { engine.githubOpenProfile() },
                        onHelp: { engine.githubInstallDocs() }
                    )
                case .accountProfile:
                    GitHubAccountProfilePage(
                        store: accountStore,
                        onOpenGitHub: { engine.githubOpenProfile() }
                    )
                case .accountSecurity:
                    GitHubAccountSecurityPage(
                        store: accountStore,
                        onSetupGit: { engine.githubSetupGit() },
                        onRefresh: { engine.refreshGitHubAccount() }
                    )
                default:
                    Form {
                        switch selectedPageID {
                        case .general: generalSections
                        case .themes: themeSections
                        case .editor: editorSections
                        case .languageServers: languageServerSections
                        case .sourceControl: sourceControlSections
                        case .extensions: extensionsSections
                        case .shortcuts: helpSections
                        default: EmptyView()
                        }
                    }
                    .formStyle(.grouped)
                }
            }
            .animation(nil, value: selectedPageID)
        }
        .background(theme.windowBg)
        .task(id: settingsFingerprint) { await commitWhenSettled() }
        // Pulled on arrival and after any change core made — including a reset
        // from another window. Leaving the page disarms whatever was recording,
        // so a field cannot sit waiting for a key nobody is going to press.
        .task(id: engine.keymapGeneration) { keyBindings = engine.keyBindings() }
        .onChange(of: selectedPageID) { _, _ in
            recordingCommand = nil
            shortcutError = nil
        }
    }

    /// Everything the user could have just changed, as one comparable value.
    ///
    /// `dirty` alone cannot drive the debounce: it goes true on the first
    /// change and stays true, so a timer keyed on it would fire once and then
    /// ignore every later edit until a save reset it.
    private var settingsFingerprint: String {
        // Theme colours are in here on purpose. Core applies a colour edit to
        // the draft but deliberately does NOT write the file — a colour well
        // emits continuously while dragged. Those edits are not rows, so
        // without them the fingerprint would not move and the debounce below
        // would never fire: every colour change would be lost on quit.
        //
        // The mask alone is not enough. Dragging a well from red to green
        // leaves the same bit set, so only the resulting COLOURS distinguish
        // one value from the next.
        var out = "\(s.dirty ? 1 : 0):\(engine.themeOverrideMask)"
        for token in EngineBridge.themeTokens {
            out += ":\(color(ofToken: token.key))"
        }
        for row in s.rows {
            out += ";\(row.id):\(row.valueIndex):\(row.value)"
        }
        return out
    }

    /// Commit once the changes stop, instead of asking the user to.
    ///
    /// System Settings has no Save button; this window had one, and a bar
    /// across the bottom saying "You have unsaved changes." The reason was
    /// real — Core owns the rows, and writing on every change would rewrite
    /// `~/.suisei.toml` per keystroke — but the reason argues for a debounce,
    /// not for a button. `task(id:)` cancels and restarts whenever the
    /// fingerprint moves, so dragging through the colour wheel or arrowing
    /// down the theme menu writes the settled value exactly once.
    ///
    /// Nothing waits on this: values already take effect live off the draft.
    /// The write is only what makes them survive a relaunch.
    private func commitWhenSettled() async {
        guard s.dirty else { return }
        try? await Task.sleep(for: .milliseconds(500))
        guard !Task.isCancelled, s.dirty else { return }
        engine.saveSettings()
    }

    private func retheme() {
        let light = preferredScheme == .light || (preferredScheme == nil && isLightTheme)
        DispatchQueue.main.async {
            for w in NSApp.windows where w.identifier == WindowChrome.settingsIdentifier {
                WindowChrome.applyThemedTitlebar(
                    to: w,
                    background: NSColor(theme.windowBg),
                    light: light
                )
            }
        }
    }

    // Account pages live in GitHubAccount.swift — they are a System Settings
    // identity surface, not another Core settings row list.

    // MARK: General

    /// General: how the app looks and behaves, the way Xcode's General does.
    ///
    /// It leads with the Appearance tiles — that is the first thing on Xcode's
    /// General — then the app's own behaviour. It does **not** carry Software
    /// Update: that has its own sidebar destination, and a row here would be
    /// the same question answered twice. Xcode does not put updates in
    /// Settings at all.
    ///
    /// What it used to hold: a duplicate of Appearance's colour-scheme tiles,
    /// then Tab Width, Line Numbers and Line Wrapping under "Editor Defaults"
    /// while a page named Editor sat below it in the sidebar.
    @ViewBuilder private var generalSections: some View {
        Section {
            HStack(spacing: 14) {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: 52, height: 52)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Suisei")
                        .font(.system(size: 15, weight: .semibold))
                    Text(SuiseiBuild.installedName)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                    Text("Engine \(EngineBridge.engineVersion)")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 6)
        }

        Section {
            appearanceModeSelector
            glassStyleSelector
        } header: {
            Text("Appearance")
        } footer: {
            Text("Automatic follows macOS. A theme pins one palette regardless — see Themes.")
        }

        // Core's remaining General groups, minus the one drawn above. The
        // Appearance rows are Segmented controls in Core, and rendering them
        // generically here would put a second, plainer copy of the tiles
        // underneath the tiles.
        ForEach(presentedGroups(on: .general).filter { $0.title != "Appearance" }) { group in
            Section(group.title) {
                ForEach(group.rows) { row in
                    settingControl(row)
                }
            }
        }
    }


    /// Themes, shaped like Xcode's: the picker, the live preview, then every
    /// colour with a well beside it.
    ///
    /// The page used to show twenty colours and let you change one. The palette
    /// layer refused the rest and argued it — letting each colour drift
    /// independently is how unreadable themes get made. The reason was right
    /// and the conclusion was too strong, so Core now allows the edits and
    /// measures the result: a row whose contrast against the editor background
    /// falls below the readable floor says so, instead of being disallowed.
    @ViewBuilder private var themeSections: some View {
        Section {
            themeSelector
        }

        // Xcode's shape exactly: the categories are a SELECTABLE list, and one
        // row underneath edits whichever is selected. Twenty rows each carrying
        // their own well — which is what this was — turns a preview you read
        // into a form you scan, and the preview is the part that tells you
        // whether an edit worked.
        Section {
            syntaxList
            selectedTokenRow
        } header: {
            Text("Source Editor")
        } footer: {
            if let ratio = selectedContrast, ratio < 3.0 {
                Label(
                    "This colour is hard to read on the editor background. \(Self.readableFloorText)",
                    systemImage: "exclamationmark.triangle.fill"
                )
                .foregroundStyle(.orange)
            } else if let worst = worstContrast {
                Label(
                    "\(worst.label) is hard to read on this background (\(contrastText(worst.ratio))).",
                    systemImage: "exclamationmark.triangle.fill"
                )
                .foregroundStyle(.orange)
            }
        }

        // Xcode's two popups, in Xcode's place: between the editing row and the
        // surface wells. Both are face-side preferences like the font size —
        // Core neither measures a line nor draws a caret.
        Section {
            Picker("Line Spacing", selection: $lineSpacing) {
                ForEach(EditorMetrics.LineSpacing.allCases, id: \.rawValue) { option in
                    Text(option.label).tag(option.rawValue)
                }
            }
            Picker("Cursor Style", selection: $cursorStyle) {
                ForEach(EditorMetrics.CursorStyle.allCases, id: \.rawValue) { option in
                    Text(option.label).tag(option.rawValue)
                }
            }
        }
        .onChange(of: lineSpacing) { _, _ in engine.relayoutEditors() }
        .onChange(of: cursorStyle) { _, _ in engine.relayoutEditors() }

        Section {
            ForEach(surfaceTokens) { token in
                surfaceRow(token)
            }
        }

        if let highlight = rows(.highlightColor).first {
            Section {
                accentColorSelector(highlight)
                    .disabled(pinnedThemeName != nil)
            } header: {
                Text("Highlight")
            } footer: {
                // Disabled rather than hidden, and the reason is stated. A
                // control that vanishes is a puzzle; a greyed one with a
                // sentence next to it is an answer.
                Text(pinnedThemeName == nil
                    ? "Unlike the Accent well above, this also re-derives everything downstream of it — selection, search, and the text drawn on accent."
                    : "\(themeDisplayName(activePaletteName)) brings its own accent, so this does not apply. It tints Light and Dark, which are deliberately neutral. To change this theme's accent, use the Accent well above.")
            }
        }

        themeManagementSection
    }

    /// The token the editing row below the list is currently pointed at.
    ///
    /// Stored by KEY, not by index: indices are Core's ABI and a future
    /// appended token would move them, which would silently repoint a stored
    /// selection at a different colour.
    private var selectedToken: EngineBridge.ThemeTokenInfo? {
        inkTokens.first { $0.key == selectedTokenKey } ?? inkTokens.first
    }

    private var selectedContrast: Double? {
        selectedToken.flatMap(contrastAgainstEditorBackground)
    }

    /// The categories, painted in their own colours on the theme's own
    /// background, and selectable.
    ///
    /// The selection band is the theme's `selection` colour rather than the
    /// system's — the same thing Xcode does, and it means the list previews
    /// one more colour for free.
    private var syntaxList: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(inkTokens) { token in
                    let selected = token.key == selectedToken?.key
                    Text(token.label)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(theme.color(color(ofToken: token.key)))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .background(selected ? theme.color(theme.selection) : .clear)
                        .contentShape(Rectangle())
                        .onTapGesture { selectedTokenKey = token.key }
                }
            }
            .padding(.vertical, 6)
        }
        .frame(height: 190)
        .background(theme.color(theme.editorBg))
        .clipShape(RoundedRectangle(cornerRadius: Radius.row, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                .strokeBorder(Color.primary.opacity(0.12), lineWidth: 1)
        )
        .padding(.vertical, 4)
    }

    /// The one row that edits whatever the list has selected.
    ///
    /// Xcode shows the font here. Suisei has no per-category font — the editor
    /// font is one setting for the whole document — so the slot carries the
    /// contrast reading instead, which is the number that actually decides
    /// whether the colour you are about to pick is usable.
    @ViewBuilder private var selectedTokenRow: some View {
        if let token = selectedToken {
            LabeledContent {
                HStack(spacing: 10) {
                    if let ratio = contrastAgainstEditorBackground(token) {
                        Text(contrastText(ratio))
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(ratio < 3.0 ? .orange : .secondary)
                            .help("Contrast against the editor background. \(Self.readableFloorText)")
                    }
                    if isOverridden(token) {
                        Button("Reset") {
                            engine.settingsSetThemeToken(token.index, "default")
                        }
                        .buttonStyle(.borderless)
                        .controlSize(.small)
                    }
                    colorWell(for: token)
                }
            } label: {
                Text(token.label)
            }
        }
    }

    private func surfaceRow(_ token: EngineBridge.ThemeTokenInfo) -> some View {
        LabeledContent {
            HStack(spacing: 10) {
                if isOverridden(token) {
                    Button("Reset") { engine.settingsSetThemeToken(token.index, "default") }
                        .buttonStyle(.borderless)
                        .controlSize(.small)
                }
                colorWell(for: token)
            }
        } label: {
            Text(token.label)
        }
    }

    private func colorWell(for token: EngineBridge.ThemeTokenInfo) -> some View {
        let packed = color(ofToken: token.key)
        return CompactColorWell(
            color: Binding(
                get: { theme.color(packed) },
                set: { engine.settingsSetThemeToken(token.index, hexString(for: $0)) }
            ),
            pill: true
        )
        .frame(width: 38, height: 20)
        .help("Change \(token.label)")
    }

    private static let readableFloorText =
        "3:1 is the readable floor for code; 4.5:1 is the standard for body text."

    /// Display order, which is NOT Core's order.
    ///
    /// Core's token indices are ABI, so a colour added later goes on the end of
    /// its enum even when it reads as belonging in the middle — `current_line`
    /// next to `line_no`, `invisibles` among the ink. Sorting for the eye is
    /// this side's job, and keeping the two orders separate is what lets Core
    /// append without the page rearranging itself.
    ///
    /// Any key Core grows that is not named here still appears, at the end of
    /// the ink list: a new colour must never be invisible just because this
    /// list was not updated.
    private static let inkOrder: [String] = [
        "fg", "comment", "string", "number", "keyword", "type_name", "function",
        "macro_name", "namespace", "parameter", "property", "constant",
        "operator", "punctuation", "line_no", "invisibles",
    ]

    private static let surfaceOrder: [String] = [
        "editor_bg", "current_line", "selection_bg", "cursor", "status_bg", "accent",
    ]

    private var inkTokens: [EngineBridge.ThemeTokenInfo] {
        ordered(Self.inkOrder, fallbackForUnknown: true)
    }

    private var surfaceTokens: [EngineBridge.ThemeTokenInfo] {
        ordered(Self.surfaceOrder, fallbackForUnknown: false)
    }

    private func ordered(
        _ keys: [String],
        fallbackForUnknown: Bool
    ) -> [EngineBridge.ThemeTokenInfo] {
        let rank = Dictionary(uniqueKeysWithValues: keys.enumerated().map { ($1, $0) })
        var listed = EngineBridge.themeTokens
            .filter { rank[$0.key] != nil }
            .sorted { rank[$0.key]! < rank[$1.key]! }
        if fallbackForUnknown {
            let known = Set(Self.inkOrder).union(Self.surfaceOrder)
            listed += EngineBridge.themeTokens.filter { !known.contains($0.key) }
        }
        return listed
    }

    /// Surfaces are not ink on the editor background, so a contrast reading
    /// against it would mean nothing for them.
    private static let surfaceKeys: Set<String> = Set(surfaceOrder)

    /// The live colour of a token, from the snapshot the editor is painting
    /// with. Core owns the names and the order; this is the one mapping the
    /// face has to keep, because `ThemeSnap` is a Swift type Core cannot name.
    private func color(ofToken key: String) -> UInt32 {
        switch key {
        case "fg": theme.fg
        case "comment": theme.comment
        case "string": theme.string
        case "number": theme.number
        case "keyword": theme.keyword
        case "type_name": theme.typeName
        case "function": theme.function
        case "macro_name": theme.macroName
        case "namespace": theme.namespace
        case "parameter": theme.parameter
        case "property": theme.property
        case "constant": theme.constant
        case "operator": theme.operatorColor
        case "punctuation": theme.punctuation
        case "line_no": theme.dim
        case "current_line": theme.currentLine
        case "invisibles": theme.invisibles
        case "editor_bg": theme.editorBg
        case "selection_bg": theme.selection
        case "cursor": theme.caret
        case "status_bg": theme.statusBg
        case "accent": theme.accent
        default: theme.fg
        }
    }

    private var activePaletteName: String {
        pinnedThemeName ?? (isLightTheme ? "light" : "dark")
    }

    private var overrideCount: Int {
        engine.themeOverrideMask.nonzeroBitCount
    }

    private func isOverridden(_ token: EngineBridge.ThemeTokenInfo) -> Bool {
        engine.themeOverrideMask & (1 << UInt32(token.index)) != 0
    }

    /// How this token reads on the editor background — `nil` for the tokens
    /// that are not ink on it, where the number would mean nothing.
    private func contrastAgainstEditorBackground(
        _ token: EngineBridge.ThemeTokenInfo
    ) -> Double? {
        guard !Self.surfaceKeys.contains(token.key) else { return nil }
        return Self.contrast(color(ofToken: token.key), theme.editorBg)
    }

    private var worstContrast: (label: String, ratio: Double)? {
        inkTokens
            .compactMap { token -> (String, Double)? in
                guard let ratio = contrastAgainstEditorBackground(token), ratio < 3.0 else {
                    return nil
                }
                return (token.label, ratio)
            }
            .min { $0.1 < $1.1 }
            .map { (label: $0.0, ratio: $0.1) }
    }

    private func contrastText(_ ratio: Double) -> String {
        String(format: "%.1f:1", ratio)
    }

    /// WCAG relative contrast — the same formula `theme::contrast_ratio` uses
    /// in Core. It is duplicated rather than crossed the ABI because the face
    /// needs it per row per redraw and it is eight lines of arithmetic; the
    /// Rust side has the test that pins the numbers.
    private static func contrast(_ a: UInt32, _ b: UInt32) -> Double {
        let la = luminance(a)
        let lb = luminance(b)
        let (hi, lo) = la > lb ? (la, lb) : (lb, la)
        return (hi + 0.05) / (lo + 0.05)
    }

    private static func luminance(_ packed: UInt32) -> Double {
        let channel = { (v: UInt32) -> Double in
            let value = Double(v) / 255.0
            return value <= 0.04045 ? value / 12.92 : pow((value + 0.055) / 1.055, 2.4)
        }
        return channel((packed >> 16) & 0xFF) * 0.2126
            + channel((packed >> 8) & 0xFF) * 0.7152
            + channel(packed & 0xFF) * 0.0722
    }

    private var appearanceModeSelector: some View {
        HStack(alignment: .top, spacing: 24) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Color Scheme")
                Text("Choose the editor’s light or dark appearance.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            HStack(spacing: 10) {
                appearanceTile("system", "Automatic", preview: .automatic)
                appearanceTile("light", "Light", preview: .light)
                appearanceTile("dark", "Dark", preview: .dark)
            }
            .fixedSize(horizontal: true, vertical: false)
        }
        .padding(.vertical, 4)
    }

    private var glassStyleSelector: some View {
        HStack(alignment: .top, spacing: 24) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Liquid Glass")
                Text("Choose the clarity of floating editor controls.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            HStack(spacing: 10) {
                glassStyleTile("clear", "Clear", tinted: false)
                glassStyleTile("tinted", "Tinted", tinted: true)
            }
            .fixedSize(horizontal: true, vertical: false)
        }
        .padding(.vertical, 4)
    }

    /// Everything about how the editor treats text, on the page called Editor.
    ///
    /// The minimap is deliberately spliced in directly after **Display** rather
    /// than trailing all the Core groups: it answers the same question those
    /// rows do — what you see beside the text — and it was reading as an
    /// afterthought below Editing. It is a face-side preference, so Core cannot
    /// order it; this is the one place that knows both.
    ///
    /// The "Advanced ▸ Terminal Compatibility" disclosure that used to close
    /// this page is gone. Its own footer said "The native Mac editor does not
    /// require them" — three switches for a terminal frontend that does not
    /// exist. They are still stored, just not offered.
    @ViewBuilder private var editorSections: some View {
        let groups = presentedGroups(on: .editor)
        ForEach(groups) { group in
            Section(group.title) {
                ForEach(group.rows) { row in
                    settingControl(row)
                }
            }
            if group.title == "Display" {
                minimapSection
            }
        }
        // If Core ever stops emitting a Display group, the minimap must still
        // be reachable. A setting you cannot find is the bug this page is for.
        if !groups.contains(where: { $0.title == "Display" }) {
            minimapSection
        }
    }

    /// The minimap's three questions, in the window that answers questions.
    ///
    /// Also in View ▸ Minimap, because that is where you reach for it mid-edit.
    /// Both surfaces read and write the same `UserDefaults` keys, so neither is
    /// a copy of the other's state — there is one answer and two ways to say it.
    ///
    /// The two shape controls are disabled rather than hidden while the minimap
    /// is off: hiding them would make the section change height on a toggle,
    /// and a setting you cannot find is worse than one you cannot currently
    /// use.
    @ViewBuilder private var minimapSection: some View {
        Section {
            Toggle("Show Minimap", isOn: $minimapEnabled)

            Picker("Show In", selection: $minimapAllPanes) {
                Text("Focused Pane Only").tag(false)
                Text("All Panes").tag(true)
            }
            .disabled(!minimapEnabled)

            Picker("Width", selection: $minimapProportional) {
                Text("Fixed").tag(false)
                Text("Proportional to Pane").tag(true)
            }
            .disabled(!minimapEnabled)
        } header: {
            Text("Minimap")
        } footer: {
            Text(
                minimapProportional
                    ? "The strip is 12% of its pane, between 44 and 62 points."
                    : "The strip is 62 points wide in every pane — its widest."
            )
        }
    }

    @ViewBuilder private var languageServerSections: some View {
        let primaryRows = presentedRows(on: .languageServers)
            .filter { $0.kind == .lspEnabled }
        Section {
            ForEach(primaryRows) { row in
                settingControl(row)
            }
        } header: {
            Text("Language Servers")
        } footer: {
            Text("Provide completion, navigation, diagnostics, and refactoring for supported languages.")
        }

        // The server list is its own group and collapsed by default: fourteen
        // languages is a long list to walk past every time you open General,
        // and only the ones you have edited are usually worth seeing.
        Section {
            DisclosureGroup("Configured Servers") {
                ForEach(rows(.lspLang)) { row in
                    settingControl(row)
                }
            }
        }
    }

    @ViewBuilder private var sourceControlSections: some View {
        ForEach(presentedGroups(on: .sourceControl)) { group in
            Section(group.title) {
                ForEach(group.rows) { row in
                    settingControl(row)
                }
            }
        }
    }

    /// The palette catalogue, which had no control at all.
    ///
    /// Core ships fifteen themes and a `SettingRow::Theme(i)` for each, and no
    /// page rendered any of them — Appearance hand-built its two sections and
    /// never asked Core for its rows. The catalogue was reachable only by
    /// hand-editing `~/.suisei.toml`.
    ///
    /// `light` and `dark` are left out on purpose: `config.theme` is ONE field
    /// holding either `system`, one of those two, or a catalogue name, so the
    /// Color Scheme tiles above already answer for them. Listing them twice
    /// would be two controls silently overwriting each other. "Match Color
    /// Scheme" hands the field back to the tiles.
    private var themeSelector: some View {
        let catalogue = engine.themeCatalogue
        let builtIns = catalogue.filter {
            !$0.isCustom && $0.name != "light" && $0.name != "dark"
        }
        let customs = catalogue.filter(\.isCustom)
        return Picker("Theme", selection: Binding<String>(
            get: { engine.selectedTheme },
            set: { engine.settingsSelectTheme($0) }
        )) {
            Text("Match Color Scheme").tag("system")
            if !customs.isEmpty {
                Divider()
                ForEach(customs) { choice in
                    Text(choice.label).tag(choice.name)
                }
            }
            Divider()
            ForEach(builtIns) { choice in
                Text(themeDisplayName(choice.label)).tag(choice.name)
            }
        }
    }

    /// The theme in use is one the user made, so it can be deleted.
    private var selectedCustomTheme: EngineBridge.ThemeChoice? {
        let current = engine.selectedTheme
        return engine.themeCatalogue.first { $0.isCustom && $0.name == current }
    }

    /// Save-as and delete.
    ///
    /// Save-as is offered only once something has been changed: with no edits
    /// it would make a theme identical to the one it came from, under a second
    /// name, which is a way to accumulate duplicates and not a feature.
    @ViewBuilder private var themeManagementSection: some View {
        Section {
            Button("Save as New Theme…") { beginSaveTheme() }
                .disabled(engine.themeOverrideMask == 0)

            Button("Reset \(themeDisplayName(activePaletteName)) to Its Original Colours") {
                engine.settingsResetThemeTokens()
            }
            .disabled(engine.themeOverrideMask == 0)

            if let custom = selectedCustomTheme {
                Button("Delete “\(custom.label)”", role: .destructive) {
                    themeToDelete = custom.name
                }
            }
        } footer: {
            Text(saveThemeFooter)
        }
        .alert("Save as New Theme", isPresented: $savingTheme) {
            TextField("Name", text: $newThemeName)
            Button("Save") { commitSaveTheme() }
                .disabled(newThemeName.trimmingCharacters(in: .whitespaces).isEmpty)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(saveThemeError.isEmpty
                ? "Your changes are kept under this name. \(themeDisplayName(activePaletteName)) goes back to its original colours."
                : saveThemeError)
        }
        .confirmationDialog(
            "Delete “\(themeToDelete ?? "")”?",
            isPresented: Binding(get: { themeToDelete != nil }, set: { if !$0 { themeToDelete = nil } }),
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                if let name = themeToDelete { engine.settingsDeleteTheme(name) }
                themeToDelete = nil
            }
            Button("Cancel", role: .cancel) { themeToDelete = nil }
        } message: {
            Text("Its colours are removed. The palette it was built on is unaffected.")
        }
    }

    private var saveThemeFooter: String {
        if overrideCount == 0 {
            return "No colours changed on this theme."
        }
        let plural = overrideCount == 1 ? "" : "s"
        return "\(overrideCount) colour\(plural) changed. Edits are kept per theme, so switching themes does not carry them over."
    }

    private func beginSaveTheme() {
        saveThemeError = ""
        newThemeName = "\(themeDisplayName(activePaletteName)) Copy"
        savingTheme = true
    }

    /// Core owns the refusal — blank, already taken, or shadowing a built-in.
    /// Re-opening the sheet with the reason beats a silent no-op.
    private func commitSaveTheme() {
        if engine.settingsSaveThemeAs(newThemeName) == nil {
            saveThemeError = "That name is already taken, or belongs to a built-in theme."
            savingTheme = true
        } else {
            saveThemeError = ""
        }
    }

    /// `mono_dark` → "Mono Dark". Core's names are config keys, not labels.
    private func themeDisplayName(_ raw: String) -> String {
        raw.split(separator: "_")
            .map { $0.prefix(1).uppercased() + $0.dropFirst() }
            .joined(separator: " ")
    }

    private var accentPresets: [AccentPreset] {
        [
            AccentPreset(name: "Blue", hex: "#0A84FF", color: Color(nsColor: .systemBlue)),
            AccentPreset(name: "Purple", hex: "#BF5AF2", color: Color(nsColor: .systemPurple)),
            AccentPreset(name: "Pink", hex: "#FF375F", color: Color(nsColor: .systemPink)),
            AccentPreset(name: "Red", hex: "#FF453A", color: Color(nsColor: .systemRed)),
            AccentPreset(name: "Orange", hex: "#FF9F0A", color: Color(nsColor: .systemOrange)),
            AccentPreset(name: "Yellow", hex: "#FFD60A", color: Color(nsColor: .systemYellow)),
            AccentPreset(name: "Green", hex: "#30D158", color: Color(nsColor: .systemGreen)),
            AccentPreset(name: "Gray", hex: "#98989D", color: Color(nsColor: .systemGray)),
        ]
    }

    private func accentColorSelector(_ row: SettingsRowItem) -> some View {
        let selected = row.value.uppercased()
        return HStack(alignment: .center, spacing: 20) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Accent Color")
                Text("Selections, focus, links, and active controls.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            HStack(spacing: 8) {
                Button { engine.settingsSetHighlightColor("default") } label: {
                    Circle()
                        .fill(
                            AngularGradient(
                                colors: [.red, .orange, .yellow, .green, .blue, .purple, .red],
                                center: .center
                            )
                        )
                        .frame(width: 22, height: 22)
                        .overlay(Circle().strokeBorder(Color.white.opacity(0.38), lineWidth: 0.5))
                        .overlay(
                            Circle()
                                .strokeBorder(liveAccent, lineWidth: 2)
                                .padding(-3)
                                .opacity(selected == "DEFAULT" ? 1 : 0)
                        )
                }
                .buttonStyle(.plain)
                .help("Default")

                ForEach(accentPresets) { preset in
                    Button { engine.settingsSetHighlightColor(preset.hex) } label: {
                        Circle()
                            .fill(preset.color)
                            .frame(width: 22, height: 22)
                            .overlay(Circle().strokeBorder(Color.white.opacity(0.22), lineWidth: 0.5))
                            .overlay(
                                Circle()
                                    .strokeBorder(liveAccent, lineWidth: 2)
                                    .padding(-3)
                                    .opacity(selected == preset.hex ? 1 : 0)
                            )
                    }
                    .buttonStyle(.plain)
                    .help(preset.name)
                }

                // Custom sits at the END of the same strip, which is where
                // System Settings puts it — one row, one question. It used to
                // be a second row ("Custom Accent Color") carrying its own
                // Default button, so the accent was asked twice and "Default"
                // was offered twice: once as the multicolour circle that opens
                // this strip, once as a text button below it.
                CompactColorWell(color: bindHighlightColor(row))
                    .frame(width: 26, height: 22)
                    .help("Custom…")
            }
            .fixedSize(horizontal: true, vertical: false)
        }
        .padding(.vertical, 5)
    }

    private func glassStyleTile(_ key: String, _ title: String, tinted: Bool) -> some View {
        let on = glassStyle == key
        let shape = RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
        let base = tinted
            ? [
                Color(red: 0.23, green: 0.14, blue: 0.49),
                Color(red: 0.18, green: 0.10, blue: 0.39),
                Color(red: 0.11, green: 0.07, blue: 0.25),
            ]
            : [
                Color(red: 0.26, green: 0.18, blue: 0.88),
                Color(red: 0.31, green: 0.11, blue: 0.72),
                Color(red: 0.14, green: 0.07, blue: 0.42),
            ]
        return Button { chooseGlassStyle(key) } label: {
            VStack(spacing: 6) {
                ZStack {
                    LinearGradient(
                        colors: base,
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )

                    RadialGradient(
                        colors: [
                            Color(red: 0.24, green: 0.55, blue: 1.0)
                                .opacity(tinted ? 0.12 : 0.42),
                            .clear,
                        ],
                        center: .topLeading,
                        startRadius: 0,
                        endRadius: 62
                    )

                    Capsule()
                        .fill(
                            LinearGradient(
                                colors: [
                                    Color(red: 0.10, green: 0.18, blue: 0.55)
                                        .opacity(tinted ? 0.72 : 0.52),
                                    Color(red: 0.025, green: 0.035, blue: 0.16)
                                        .opacity(tinted ? 0.88 : 0.76),
                                    Color(red: 0.15, green: 0.10, blue: 0.42)
                                        .opacity(tinted ? 0.72 : 0.52),
                                ],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                        )
                        .overlay(
                            Capsule()
                                .strokeBorder(
                                    LinearGradient(
                                        colors: [
                                            Color(red: 0.28, green: 0.62, blue: 1.0)
                                                .opacity(tinted ? 0.18 : 0.62),
                                            Color.white.opacity(tinted ? 0.06 : 0.18),
                                            .clear,
                                        ],
                                        startPoint: .topLeading,
                                        endPoint: .bottomTrailing
                                    ),
                                    lineWidth: 0.9
                                )
                        )
                        .frame(width: 104, height: 27)
                        .rotationEffect(.degrees(-20))
                        .offset(x: 13, y: 7)
                        .shadow(
                            color: Color.blue.opacity(tinted ? 0.08 : 0.24),
                            radius: 4,
                            x: -2,
                            y: -2
                        )

                    Capsule()
                        .fill(
                            LinearGradient(
                                colors: [
                                    Color.white.opacity(tinted ? 0.04 : 0.19),
                                    Color.blue.opacity(tinted ? 0.03 : 0.12),
                                    .clear,
                                ],
                                startPoint: .leading,
                                endPoint: .trailing
                            )
                        )
                        .frame(width: 76, height: 6)
                        .rotationEffect(.degrees(-20))
                        .offset(x: 4, y: -2)
                        .blur(radius: 0.7)
                }
                .frame(width: 82, height: 50)
                .clipShape(shape)
                .overlay(
                    shape.strokeBorder(
                        Color.white.opacity(tinted ? 0.10 : 0.22),
                        lineWidth: 0.6
                    )
                )
                .overlay(
                    shape
                        .strokeBorder(on ? liveAccent : Color.secondary.opacity(0.3),
                                      lineWidth: on ? 2.5 : 1)
                )
                .scaleEffect(on ? 1.0 : 0.97)

                Text(title)
                    .font(.system(size: 11, weight: on ? .medium : .regular))
                    .foregroundStyle(on ? .primary : .secondary)
            }
            .frame(width: 88)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .animation(.snappy(duration: 0.18), value: on)
    }

    private enum PreviewKind { case automatic, light, dark }

    private struct PreviewPalette {
        let sidebar: Color
        let toolbar: Color
        let editor: Color
        let line: Color
        let mutedLine: Color
    }

    private func appearanceTile(_ key: String, _ title: String, preview: PreviewKind) -> some View {
        let on = appearanceMode == key
        return Button { chooseAppearance(key) } label: {
            VStack(spacing: 6) {
                appearancePreview(preview)
                .overlay(
                    RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                        .strokeBorder(on ? liveAccent : Color.secondary.opacity(0.3),
                                      lineWidth: on ? 2.5 : 1)
                )
                .scaleEffect(on ? 1.0 : 0.97)
                Text(title)
                    .font(.system(size: 11, weight: on ? .medium : .regular))
                    .foregroundStyle(on ? .primary : .secondary)
            }
            .frame(width: 88)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .animation(.snappy(duration: 0.18), value: on)
    }

    /// A tiny but structurally faithful editor window. The former previews
    /// were three empty gradients with traffic lights pasted on top; at this
    /// scale that reads as placeholder art. Xcode's previews remain legible
    /// because their navigator, toolbar, selection and editor are visible.
    private func appearancePreview(_ kind: PreviewKind) -> some View {
        let palette = previewPalette(kind)

        return ZStack(alignment: .topLeading) {
            HStack(spacing: 0) {
                ZStack(alignment: .topLeading) {
                    Rectangle().fill(palette.sidebar)

                    VStack(alignment: .leading, spacing: 2.5) {
                        RoundedRectangle(cornerRadius: 1.5, style: .continuous)
                            .fill(liveAccent.opacity(0.85))
                            .frame(width: 17, height: 6)
                        ForEach([13.0, 16.0, 11.0], id: \.self) { width in
                            Capsule()
                                .fill(palette.mutedLine)
                                .frame(width: width, height: 2)
                        }
                    }
                    .padding(.top, 17)
                    .padding(.leading, 4)
                }
                .frame(width: 25)

                VStack(spacing: 0) {
                    Rectangle()
                        .fill(palette.toolbar)
                        .frame(height: 13)

                    ZStack(alignment: .topLeading) {
                        Rectangle().fill(palette.editor)

                        VStack(alignment: .leading, spacing: 3) {
                            RoundedRectangle(cornerRadius: 1, style: .continuous)
                                .fill(liveAccent.opacity(0.82))
                                .frame(width: 24, height: 3)
                            RoundedRectangle(cornerRadius: 1, style: .continuous)
                                .fill(palette.line)
                                .frame(width: 37, height: 3)
                            RoundedRectangle(cornerRadius: 1, style: .continuous)
                                .fill(palette.mutedLine)
                                .frame(width: 29, height: 3)
                        }
                        .padding(.top, 7)
                        .padding(.leading, 7)
                    }
                }
            }

            HStack(spacing: 2.5) {
                Circle().fill(Color(red: 1.0, green: 0.32, blue: 0.31))
                Circle().fill(Color(red: 1.0, green: 0.73, blue: 0.18))
                Circle().fill(Color(red: 0.18, green: 0.78, blue: 0.35))
            }
            .frame(width: 18, height: 5)
            .padding(.top, 5)
            .padding(.leading, 4)
        }
        .frame(width: 82, height: 50)
        .clipShape(RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
    }

    private func previewPalette(_ kind: PreviewKind) -> PreviewPalette {
        switch kind {
        case .automatic:
            return PreviewPalette(
                sidebar: Color(white: 0.84),
                toolbar: Color(white: 0.92),
                editor: Color(white: 0.11),
                line: Color(white: 0.80),
                mutedLine: Color(white: 0.48)
            )
        case .light:
            return PreviewPalette(
                sidebar: Color(white: 0.86),
                toolbar: Color(white: 0.95),
                editor: Color(white: 0.99),
                line: Color(white: 0.34),
                mutedLine: Color(white: 0.66)
            )
        case .dark:
            return PreviewPalette(
                sidebar: Color(white: 0.16),
                toolbar: Color(white: 0.22),
                editor: Color(white: 0.10),
                line: Color(white: 0.78),
                mutedLine: Color(white: 0.38)
            )
        }
    }

    private func chooseAppearance(_ key: String) {
        guard let value = ["system": 0, "light": 1, "dark": 2][key],
              let row = rows(.appearanceMode).first else { return }
        engine.settingsSetValue(row.id, value: value)
    }

    private func chooseGlassStyle(_ key: String) {
        guard let value = ["clear": 0, "tinted": 1][key],
              let row = rows(.glassStyle).first else { return }
        engine.settingsSetValue(row.id, value: value)
    }

    // MARK: Other pages

    @ViewBuilder private var extensionsSections: some View {
        Section {
            ForEach(s.rows.filter { !$0.isHeader }) { row in
                Button {
                    engine.settingsSelect(row.id)
                    engine.settingsActivate(row.id)
                } label: {
                    LabeledContent(clean(row.label), value: row.value)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        } footer: {
            Text("VS Code-compatible extensions run through the shared Suisei host.")
        }
    }

    /// Xcode's Shortcuts pane: a filter, then the commands grouped by where
    /// they live, each with its key equivalent right-aligned.
    ///
    /// This was a two-column `LazyVGrid` of hand-drawn key chips — a shape no
    /// Mac settings window uses, and one that put the KEY first and the command
    /// second, so the column you would scan for ("Save") was the one you could
    /// not scan. Xcode leads with the command and right-aligns the keys; that
    /// is also the only order that stays aligned when the key strings differ
    /// in width.
    @ViewBuilder private var helpSections: some View {
        let engineRows = s.rows.filter { !$0.isHeader }
        let bindings = keyBindings.filter { shortcutMatches($0.title, $0.chord) }
        let groups = orderedGroups(of: bindings)

        Section {
            TextField("Filter", text: $shortcutFilter)
                .textFieldStyle(.roundedBorder)
            if let shortcutError {
                Label(shortcutError, systemImage: "exclamationmark.triangle.fill")
                    .font(.subheadline)
                    .foregroundStyle(Color(nsColor: .systemYellow))
            }
        }

        ForEach(groups, id: \.self) { group in
            Section(group) {
                ForEach(bindings.filter { $0.group == group }) { item in
                    shortcutRow(item)
                }
            }
        }

        if keyBindings.contains(where: \.customised) {
            Section {
                Button("Restore All Defaults") {
                    engine.resetKeyBindings()
                    reloadKeyBindings()
                }
            } footer: {
                Text("Only the shortcuts you changed are stored; everything else follows Suisei's defaults.")
            }
        }

        // Core's own binding table. Reference only — these are the modal engine
        // keys, they COMPOSE (`d`,`i`,`w`), and "what does `diw` rebind to" is a
        // different feature with a different answer. Capped, and the cap is
        // stated rather than silently swallowing the tail: a list that stops
        // without saying so reads as a complete list.
        let engineMatches = engineRows.filter { shortcutMatches($0.label, $0.value) }
        if !engineMatches.isEmpty {
            Section {
                DisclosureGroup(isExpanded: $engineReferenceExpanded) {
                    ForEach(engineMatches.prefix(Self.engineCommandLimit)) { row in
                        LabeledContent(clean(row.label)) {
                            Text(row.value)
                                .font(.system(size: 12, design: .monospaced))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                    }
                } label: {
                    LabeledContent("Engine Commands", value: "\(engineMatches.count)")
                }
            } header: {
                Text("Advanced")
            } footer: {
                Text(
                    engineMatches.count > Self.engineCommandLimit
                        ? "Showing the first \(Self.engineCommandLimit) of \(engineMatches.count) engine bindings. These are modal editor keys and are not rebindable here."
                        : "Suisei's engine bindings — modal editor keys, not rebindable here."
                )
            }
        }
    }

    /// Groups in the order core lists them, not alphabetically: the catalogue's
    /// order is a judgement about what a reader looks for first.
    private func orderedGroups(of items: [KeyBindingItem]) -> [String] {
        var seen: [String] = []
        for i in items where !seen.contains(i.group) { seen.append(i.group) }
        return seen
    }

    private static let engineCommandLimit = 24

    /// Command on the left, its key on the right — the order every Mac shortcut
    /// list uses, the menu bar included. The key is a BUTTON now: click it and
    /// the next chord replaces it.
    @ViewBuilder
    private func shortcutRow(_ item: KeyBindingItem) -> some View {
        let isRecording = recordingCommand == item.id
        LabeledContent(item.title) {
            HStack(spacing: 6) {
                if item.customised, !isRecording {
                    // Only where there is something to undo.
                    Button {
                        engine.setKeyBinding(id: item.id, chord: nil)
                        reloadKeyBindings()
                    } label: {
                        Image(systemName: "arrow.uturn.backward")
                            .font(.caption)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .help("Back to \(item.defaultChord)")
                }
                Button {
                    shortcutError = nil
                    recordingCommand = isRecording ? nil : item.id
                } label: {
                    Text(isRecording ? "Press a key…" : item.chord)
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(isRecording ? Color.accentColor : .secondary)
                        .lineLimit(1)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 3)
                        .background(
                            RoundedRectangle(cornerRadius: 5, style: .continuous)
                                .fill(Color.primary.opacity(isRecording ? 0.10 : 0.06))
                        )
                        .overlay(
                            RoundedRectangle(cornerRadius: 5, style: .continuous)
                                .strokeBorder(
                                    isRecording ? Color.accentColor : .clear,
                                    lineWidth: 1
                                )
                        )
                }
                .buttonStyle(.plain)
                .overlay {
                    // Armed only for the row being recorded, so ⌘S still saves
                    // everywhere else in this window.
                    if isRecording {
                        ShortcutRecorder(
                            recording: Binding(
                                get: { recordingCommand == item.id },
                                set: { if !$0 { recordingCommand = nil } }
                            ),
                            onChord: { apply($0, to: item) }
                        )
                        .allowsHitTesting(false)
                    }
                }
            }
        }
        .help(item.customised ? "Suisei's default is \(item.defaultChord)" : "")
    }

    /// Take a recorded chord, but say what is wrong before taking it.
    ///
    /// Two menu items on one key equivalent is a state AppKit resolves by
    /// picking one — silently, and not necessarily the one just set. So the
    /// clash is refused and named, rather than stored and discovered later as
    /// "the shortcut stopped working".
    private func apply(_ chord: String, to item: KeyBindingItem) {
        if let other = engine.keyBindingConflict(id: item.id, chord: chord) {
            shortcutError = "\(chord) is already \(other)."
            return
        }
        guard engine.setKeyBinding(id: item.id, chord: chord) else {
            shortcutError = "\(chord) cannot be a menu shortcut. Use ⌘ or ⌃ — ⌥ and a letter types a character."
            return
        }
        shortcutError = nil
        reloadKeyBindings()
    }

    private func reloadKeyBindings() {
        keyBindings = engine.keyBindings()
    }

    private func shortcutMatches(_ command: String, _ keys: String) -> Bool {
        let query = shortcutFilter.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return true }
        return command.localizedCaseInsensitiveContains(query)
            || keys.localizedCaseInsensitiveContains(query)
    }

    /// Command on the left, key equivalent right-aligned — the order every Mac
    /// shortcut list uses, including the menu bar itself.
    private func shortcutRow(_ command: String, _ keys: String) -> some View {
        LabeledContent(command) {
            Text(keys)
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }



    // MARK: Core-described controls

    @ViewBuilder
    private func settingControl(_ row: SettingsRowItem) -> some View {
        switch row.control {
        case .toggle:
            Toggle(isOn: bindToggle(row)) {
                settingLabel(row)
            }
            .accessibilityLabel(row.label)
            .accessibilityHint(row.detail)
        case .menu:
            HStack(alignment: .center, spacing: 16) {
                settingLabel(row)
                Spacer(minLength: 12)
                if row.options.isEmpty {
                    Text(row.value)
                        .foregroundStyle(.secondary)
                } else {
                    Picker("", selection: bindOption(row)) {
                        ForEach(Array(row.options.enumerated()), id: \.offset) { index, option in
                            Text(option)
                                .tag(index)
                                .disabled(
                                    row.kind == .lspLang
                                        && index == 2
                                        && row.valueIndex != 2
                                )
                        }
                    }
                    .labelsHidden()
                    .fixedSize()
                    .accessibilityLabel(row.label)
                    .accessibilityHint(row.detail)
                }
            }
        case .segmented:
            HStack(alignment: .center, spacing: 16) {
                settingLabel(row)
                Spacer(minLength: 12)
                Picker("", selection: bindOption(row)) {
                    ForEach(Array(row.options.enumerated()), id: \.offset) { index, option in
                        Text(option).tag(index)
                    }
                }
                .labelsHidden()
                .pickerStyle(.segmented)
                .fixedSize()
                .accessibilityLabel(row.label)
                .accessibilityHint(row.detail)
            }
        case .action:
            Button {
                engine.settingsSelect(row.id)
                engine.settingsActivate(row.id)
            } label: {
                HStack(spacing: 12) {
                    settingLabel(row)
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(row.label)
            .accessibilityHint(row.detail)
        case .color:
            HStack(alignment: .center, spacing: 12) {
                settingLabel(row)
                Spacer(minLength: 12)
                Button("Default") {
                    engine.settingsSetHighlightColor("default")
                }
                .buttonStyle(.borderless)
                .disabled(row.value.caseInsensitiveCompare("default") == .orderedSame)
                ColorPicker(
                    "Highlight Color",
                    selection: bindHighlightColor(row),
                    supportsOpacity: false
                )
                .labelsHidden()
                .accessibilityHint(row.detail)
            }
        case .none:
            EmptyView()
        }
    }

    private func settingLabel(_ row: SettingsRowItem) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(row.label)
            if !row.detail.isEmpty {
                Text(row.detail)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    // MARK: Bindings

    private func bindToggle(_ row: SettingsRowItem) -> Binding<Bool> {
        Binding(
            get: { row.valueIndex != 0 },
            set: { enabled in
                engine.settingsSetValue(row.id, value: enabled ? 1 : 0)
            }
        )
    }

    private func bindOption(_ row: SettingsRowItem) -> Binding<Int> {
        Binding(
            get: { row.valueIndex },
            set: { option in
                engine.settingsSetValue(row.id, value: option)
            }
        )
    }

    private func bindHighlightColor(_ row: SettingsRowItem) -> Binding<Color> {
        Binding(
            get: { theme.color(theme.accent) },
            set: { engine.settingsSetHighlightColor(hexString(for: $0)) }
        )
    }

    private func hexString(for color: Color) -> String {
        let source = NSColor(color)
        let converted = source.usingColorSpace(.sRGB) ?? source
        var red: CGFloat = 0
        var green: CGFloat = 0
        var blue: CGFloat = 0
        var alpha: CGFloat = 0
        converted.getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        return String(
            format: "#%02X%02X%02X",
            Int((red * 255).rounded()),
            Int((green * 255).rounded()),
            Int((blue * 255).rounded())
        )
    }

    private func clean(_ label: String) -> String {
        label
            .replacingOccurrences(of: "●", with: "")
            .trimmingCharacters(in: .whitespaces)
    }
}

/// A row that pushes to another page, drawn the way Xcode's Settings draws one.
///
/// Xcode uses two different icon treatments and it is worth being precise about
/// which is which. Its **sidebar** is flat monochrome outline symbols in the
/// label's own colour. Its **in-page navigation rows** — Editing ▸ Display,
/// Completion, Indentation — are tinted squircles with a white glyph and a
/// trailing chevron. Suisei briefly had the squircles in the sidebar, which is
/// the System Settings idiom, on an editor that sits next to Xcode.
struct SettingsNavigationRow: View {
    var symbol: String
    var tint: Color
    var title: String
    var value: String = ""
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .fill(tint.gradient)
                    .frame(width: 20, height: 20)
                    .overlay(
                        Image(systemName: symbol)
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(.white)
                    )
                Text(title).foregroundStyle(.primary)
                Spacer(minLength: 8)
                if !value.isEmpty {
                    Text(value)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityValue(value)
    }
}

/// AppKit's minimal color well is the same compact circular control used in
/// System Settings. SwiftUI's `ColorPicker` expands into a wide capsule on
/// macOS 26, which breaks a horizontal palette even when its host is framed.
private struct CompactColorWell: NSViewRepresentable {
    @Binding var color: Color
    /// Pill rather than circle. Xcode's Themes page uses wide rounded wells;
    /// the accent strip on this page uses circles, because there it is one
    /// swatch among nine and a circle reads as a choice in a row of choices.
    var pill = false

    func makeCoordinator() -> Coordinator {
        Coordinator(color: $color)
    }

    func makeNSView(context: Context) -> CircularColorWellHost {
        let host = CircularColorWellHost(pill: pill)
        let well = host.well
        well.supportsAlpha = false
        well.target = context.coordinator
        well.action = #selector(Coordinator.colorChanged(_:))
        well.color = NSColor(color)
        return host
    }

    func updateNSView(_ host: CircularColorWellHost, context: Context) {
        context.coordinator.color = $color
        let well = host.well
        let next = NSColor(color).usingColorSpace(.sRGB) ?? NSColor(color)
        let current = well.color.usingColorSpace(.sRGB) ?? well.color
        if next != current { well.color = next }
    }

    final class Coordinator: NSObject {
        var color: Binding<Color>

        init(color: Binding<Color>) {
            self.color = color
        }

        @objc func colorChanged(_ sender: NSColorWell) {
            color.wrappedValue = Color(nsColor: sender.color)
        }
    }
}

private final class CircularColorWellHost: NSView {
    let well = NSColorWell(style: .minimal)
    private let pill: Bool

    override var intrinsicContentSize: NSSize {
        pill ? NSSize(width: 38, height: 20) : NSSize(width: 22, height: 22)
    }

    init(pill: Bool) {
        self.pill = pill
        super.init(frame: .zero)
        wantsLayer = true
        // The host does the clipping, so the shape is ours regardless of how
        // AppKit chooses to draw a minimal well at this size.
        layer?.cornerRadius = pill ? 10 : 11
        layer?.masksToBounds = true
        well.frame = bounds
        well.autoresizingMask = [.width, .height]
        addSubview(well)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}

/// Use the same AppKit control as System Settings instead of rebuilding its
/// icon, bezel, clear button and focus behavior from independent SwiftUI views.
/// Its host remains transparent, so source-list rows can scroll beneath the
/// rounded search bezel without exposing a rectangular header boundary.
private struct NativeSidebarSearchField: NSViewRepresentable {
    @Binding var text: String

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeNSView(context: Context) -> NSSearchField {
        let field = NSSearchField()
        field.placeholderString = "Search"
        field.controlSize = .regular
        field.font = .systemFont(ofSize: 13)
        field.sendsWholeSearchString = false
        field.sendsSearchStringImmediately = true
        field.target = context.coordinator
        field.action = #selector(Coordinator.searchChanged(_:))
        field.setAccessibilityLabel("Search Settings")
        return field
    }

    func updateNSView(_ nsView: NSSearchField, context: Context) {
        context.coordinator.parent = self
        if nsView.stringValue != text {
            nsView.stringValue = text
        }
    }

    final class Coordinator: NSObject {
        var parent: NativeSidebarSearchField

        init(parent: NativeSidebarSearchField) {
            self.parent = parent
        }

        @objc func searchChanged(_ sender: NSSearchField) {
            parent.text = sender.stringValue
        }
    }
}
