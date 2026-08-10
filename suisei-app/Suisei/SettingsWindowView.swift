import SwiftUI
import AppKit

/// Settings — modern macOS System Settings look: icon-tile sidebar, grouped
/// forms, live theme preview. Values come from Core rows (single source).
struct SettingsWindowView: View {
    @ObservedObject var engine: EngineBridge
    @Environment(\.dismiss) private var dismiss
    @State private var engineReferenceExpanded = false
    @State private var searchText = ""
    @State private var selectedPageID: PageID = .general
    @State private var pageHistory: [PageID] = [.general]
    @State private var historyIndex = 0

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

    private var appearanceMode: String {
        guard let index = rows(.appearanceMode).first?.valueIndex else { return "system" }
        switch index {
        case 1: return "light"
        case 2: return "dark"
        default: return "system"
        }
    }

    private var glassStyle: String {
        rows(.glassStyle).first?.valueIndex == 1 ? "tinted" : "clear"
    }

    private enum PageID: String, Hashable {
        case general
        case accounts
        case appearance
        case editor
        case languageServers
        case sourceControl
        case extensions
        case shortcuts
        case softwareUpdate
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
            id: .general, title: "General", symbol: "gearshape",
            searchTerms: "overview settings preferences", corePage: 1
        ),
        Page(
            id: .accounts, title: "Accounts", symbol: "at",
            searchTerms: "account google sign in sync", corePage: 1
        ),
        Page(
            id: .appearance, title: "Appearance", symbol: "paintbrush",
            searchTerms: "theme light dark auto color", corePage: 1
        ),
        Page(
            id: .editor, title: "Editor", symbol: "square.and.pencil",
            searchTerms: "editing wrap line tab gpu clipboard undo", corePage: 1
        ),
        Page(
            id: .languageServers, title: "Language Servers", symbol: "square.stack.3d.up",
            searchTerms: "lsp language server command", corePage: 1
        ),
        Page(
            id: .sourceControl, title: "Source Control", symbol: "arrow.triangle.branch",
            searchTerms: "git scm repository workbench", corePage: 1
        ),
        Page(
            id: .extensions, title: "Extensions", symbol: "puzzlepiece.extension",
            searchTerms: "extension language syntax grammar", corePage: 2
        ),
        Page(
            id: .shortcuts, title: "Shortcuts", symbol: "keyboard",
            searchTerms: "keyboard key binding command", corePage: 3
        ),
        Page(
            id: .softwareUpdate, title: "Software Update", symbol: "arrow.triangle.2.circlepath",
            searchTerms: "software update version release", corePage: 1
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
        pages.first(where: { $0.id == selectedPageID }) ?? pages[0]
    }

    private var selectedPage: Binding<PageID?> {
        Binding(
            get: { selectedPageID },
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
        .toolbarTitleDisplayMode(.inlineLarge)
        .toolbar {
            ToolbarItem(placement: .navigation) {
                historyControl
            }
        }
        // Keep the split view responsible for the entire resizable window so
        // the sidebar material and the detail background both reach the bottom.
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .frame(minWidth: 740, minHeight: 480)
        .preferredColorScheme(preferredScheme)
        .tint(theme.color(theme.accent))
        .accentColor(theme.color(theme.accent))
        .background(
            ThemedWindowChrome(
                background: NSColor.windowBackgroundColor,
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
            ForEach(visiblePages) { page in
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
        .toolbar {
            Rectangle()
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

    private func sidebarIcon(for page: Page, size: CGFloat) -> some View {
        Image(systemName: page.symbol)
            .symbolRenderingMode(.monochrome)
            .font(.system(size: size * 0.68, weight: .regular))
            .foregroundStyle(.secondary)
            .frame(width: size, height: size)
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
        guard let page = pages.first(where: { $0.id == id }) else { return }

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
            // No `ScrollView` around this: `.formStyle(.grouped)` already
            // scrolls, and nesting the two gave the page a second scroller and
            // its own inset that never lined up with the group insets.
            //
            // The big inline page title is gone too — the window title carries
            // it now, which is where System Settings puts it, and it stops the
            // heading from scrolling away from the groups it names.
            Form {
                switch selectedPageID {
                case .general: generalSections
                case .accounts: accountSections
                case .appearance: appearanceSections
                case .editor: editorSections
                case .languageServers: languageServerSections
                case .sourceControl: sourceControlSections
                case .extensions: extensionsSections
                case .shortcuts: helpSections
                case .softwareUpdate: softwareUpdateSections
                }
            }
            .formStyle(.grouped)
            .animation(nil, value: selectedPageID)

            // Apply bar, only where something can be applied.
            //
            // System Settings has no Save at all — it commits as you type. This
            // window still needs an explicit commit (Core owns the rows and
            // writing on each keystroke would rewrite the config file per
            // character), so the affordance stays; what changes is that it now
            // reads as a sheet's action bar at the bottom rather than as a
            // toolbar button beside a title, and the "Unsaved" badge became the
            // plain sentence it always meant.
            if s.dirty {
                Divider()
                HStack(spacing: 10) {
                    Image(systemName: "pencil.circle.fill")
                        .foregroundStyle(.orange)
                    Text("You have unsaved changes.")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Button("Save") { engine.saveSettings() }
                        .keyboardShortcut("s", modifiers: .command)
                }
                .animation(.snappy(duration: 0.2), value: s.dirty)
                .padding(.horizontal, 20)
                .padding(.vertical, 12)
                .background(.bar)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private func retheme() {
        let light = preferredScheme == .light || (preferredScheme == nil && isLightTheme)
        DispatchQueue.main.async {
            for w in NSApp.windows where w.identifier == WindowChrome.settingsIdentifier {
                WindowChrome.applyThemedTitlebar(
                    to: w,
                    background: NSColor.windowBackgroundColor,
                    light: light
                )
            }
        }
    }

    @ViewBuilder private var accountSections: some View {
        Section {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "person.crop.circle")
                    .font(.system(size: 28, weight: .regular))
                    .foregroundStyle(.secondary)
                    .frame(width: 34)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Sign in to a Google Account")
                        .fontWeight(.medium)
                    Text("Account sync is planned but is not available in this build.")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 4)
        }

        Section {
            LabeledContent("Status", value: "Not Connected")
        } footer: {
            Text("Suisei does not store placeholder credentials or make an account request from this page.")
        }
    }

    @ViewBuilder private var softwareUpdateSections: some View {
        Section {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "arrow.triangle.2.circlepath")
                    .font(.system(size: 24, weight: .regular))
                    .foregroundStyle(.secondary)
                    .frame(width: 34)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Software Update")
                        .fontWeight(.medium)
                    Text("Suisei can check for a newer release in the background at launch.")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 4)
        }

        Section {
            LabeledContent("Current Version", value: EngineBridge.engineVersion)
            LabeledContent("Release Channel", value: "Development")
            if let updateCheck = rows(.updateCheck).first {
                settingControl(updateCheck)
            }
        } footer: {
            Text("Automatic checks are throttled and do not block editor startup.")
        }
    }

    // MARK: General

    @ViewBuilder private var generalSections: some View {
        Section {
            appearanceModeSelector
        } header: {
            Text("Appearance")
        }

        ForEach(presentedGroups(on: .general)) { group in
            Section(group.title) {
                ForEach(group.rows) { row in
                    settingControl(row)
                }
            }
        }
    }

    @ViewBuilder private var appearanceSections: some View {
        Section {
            appearanceModeSelector
            glassStyleSelector
        } header: {
            Text("Appearance")
        } footer: {
            Text("Automatic follows macOS. Liquid Glass changes floating editor controls without changing syntax colours.")
        }

        if let highlight = rows(.highlightColor).first {
            Section {
                accentColorSelector(highlight)
                customAccentColorSelector(highlight)
            } header: {
                Text("Theme")
            } footer: {
                Text("Default follows the selected palette. A custom accent changes selections, focus, links, and active controls.")
            }
        }
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

    @ViewBuilder private var editorSections: some View {
        ForEach(presentedGroups(on: .editor)) { group in
            Section(group.title) {
                ForEach(group.rows) { row in
                    settingControl(row)
                }
            }
        }

        let advancedRows = presentedRows(on: .editor, advanced: true)
        if !advancedRows.isEmpty {
            Section {
                DisclosureGroup("Terminal Compatibility") {
                    VStack(spacing: 0) {
                        ForEach(Array(advancedRows.enumerated()), id: \.element.id) { index, row in
                            settingControl(row)
                                .padding(.vertical, 7)
                            if index < advancedRows.count - 1 {
                                Divider()
                            }
                        }
                    }
                    .padding(.top, 4)
                }
            } header: {
                Text("Advanced")
            } footer: {
                Text("These options affect terminal frontends. The native Mac editor does not require them.")
            }
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

    /// Live swatch strip of the active theme (bg / fg / accent / syntax hues).
    private var themePreviewCard: some View {
        Group {
            RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                .fill(theme.color(theme.editorBg))
                .overlay(
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(spacing: 4) {
                            Text("func").foregroundStyle(theme.color(theme.keyword))
                            Text("render()").foregroundStyle(theme.color(theme.function))
                        }
                        HStack(spacing: 4) {
                            Text("let").foregroundStyle(theme.color(theme.keyword))
                            Text("title =").foregroundStyle(theme.color(theme.fg))
                            Text("\"suisei\"").foregroundStyle(theme.color(theme.string))
                        }
                        HStack(spacing: 4) {
                            Text("// comet engine").foregroundStyle(theme.color(theme.comment))
                        }
                    }
                    .font(.system(size: 11, design: .monospaced))
                    .padding(10),
                    alignment: .topLeading
                )
                .overlay(
                    RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                        .strokeBorder(Color.primary.opacity(0.12), lineWidth: 1)
                )
                .frame(height: 76)

        }
        .padding(.vertical, 4)
    }

    /// Swatches, on their own line under the sample.
    ///
    /// They used to be a right-hand column beside it, which squeezed four
    /// 9-point labels into whatever width the sample left over — the labels
    /// ended up jammed against the window edge and too small to read. Laid out
    /// horizontally they get the full width and a legible size, and the code
    /// sample gets the whole card.
    private var themeSwatches: some View {
        HStack(spacing: 16) {
            ForEach(
                [("Accent", theme.accent), ("Text", theme.fg),
                 ("Selection", theme.selection), ("Caret", theme.caret)],
                id: \.0
            ) { name, packed in
                HStack(spacing: 5) {
                    Circle()
                        .fill(theme.color(packed))
                        .frame(width: 10, height: 10)
                        .overlay(Circle().strokeBorder(Color.primary.opacity(0.15), lineWidth: 0.5))
                    Text(name)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
        }
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

            }
            .fixedSize(horizontal: true, vertical: false)
        }
        .padding(.vertical, 5)
    }

    private func customAccentColorSelector(_ row: SettingsRowItem) -> some View {
        HStack(alignment: .center, spacing: 16) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Custom Accent Color")
                Text("Choose any sRGB colour or return to the palette default.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 16)
            Button("Default") {
                engine.settingsSetHighlightColor("default")
            }
            .buttonStyle(.borderless)
            .disabled(row.value.caseInsensitiveCompare("default") == .orderedSame)
            CompactColorWell(color: bindHighlightColor(row))
                .frame(width: 44, height: 22)
                .help("Choose Custom Accent Color")
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

    @ViewBuilder private var helpSections: some View {
        Section {
            shortcutGrid([
                ("⌘S", "Save"),
                ("⌘P", "Open file"),
                ("⇧⌘P", "Command palette"),
                ("⌘F", "Find in file"),
                ("⌘G / ⇧⌘G", "Next / previous match"),
                ("⇧⌘F", "Find in project"),
                ("⌘Z / ⇧⌘Z", "Undo / redo"),
                ("⌃⇥ / ⌃⇧⇥", "Next / previous tab"),
            ])
        } header: {
            Text("Editing")
        }
        Section {
            shortcutGrid([
                ("⌘0", "Toggle navigator"),
                ("⌥⌘0", "Toggle inspector"),
                ("⇧⌘Y", "Toggle debug area"),
                ("⌃T", "Terminal in debug area"),
                ("⌃⇧T", "Terminal in editor pane"),
                ("⇧⌘V", "Pretty preview"),
                ("⌃G / ⌃⇧G", "Source control / Git workbench"),
                ("⌘,", "Settings"),
            ])
        } header: {
            Text("Panels")
        }
        Section {
            DisclosureGroup(isExpanded: $engineReferenceExpanded) {
                LazyVStack(spacing: 0) {
                    ForEach(s.rows.filter { !$0.isHeader }.prefix(24)) { row in
                        shortcutRow(row.label, row.value)
                    }
                }
                .padding(.top, 6)
            } label: {
                HStack {
                    Text("Show engine commands")
                    Spacer()
                    Text("\(min(24, s.rows.filter { !$0.isHeader }.count))")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        } header: {
            Text("Engine Reference")
        } footer: {
            Text("Full Suisei keybinding reference — advanced users can drive Core directly.")
        }
    }

    private func shortcutRow(_ keys: String, _ what: String) -> some View {
        HStack(spacing: 8) {
            Text(keys)
                .font(.system(size: 12, design: .monospaced))
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(
                    RoundedRectangle(cornerRadius: Radius.row, style: .continuous)
                        .fill(Color.primary.opacity(0.06))
                )
            Text(what)
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 0)
        }
        .frame(minHeight: 30)
    }

    private func shortcutGrid(_ rows: [(String, String)]) -> some View {
        LazyVGrid(
            columns: [
                GridItem(.flexible(), spacing: 12),
                GridItem(.flexible(), spacing: 12),
            ],
            alignment: .leading,
            spacing: 0
        ) {
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                shortcutRow(row.0, row.1)
            }
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

/// AppKit's minimal color well is the same compact circular control used in
/// System Settings. SwiftUI's `ColorPicker` expands into a wide capsule on
/// macOS 26, which breaks a horizontal palette even when its host is framed.
private struct CompactColorWell: NSViewRepresentable {
    @Binding var color: Color

    func makeCoordinator() -> Coordinator {
        Coordinator(color: $color)
    }

    func makeNSView(context: Context) -> CircularColorWellHost {
        let host = CircularColorWellHost()
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

    override var intrinsicContentSize: NSSize { NSSize(width: 22, height: 22) }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.cornerRadius = 11
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
