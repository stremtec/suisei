import SwiftUI
import AppKit

/// Settings — modern macOS System Settings look: icon-tile sidebar, grouped
/// forms, live theme preview. Values come from Core rows (single source).
struct SettingsWindowView: View {
    @ObservedObject var engine: EngineBridge
    @Environment(\.dismiss) private var dismiss
    @AppStorage("suisei.appearance") private var appearanceMode: String = "system"
    @State private var engineReferenceExpanded = false

    private var s: SettingsSnap { engine.chrome.settings }
    private var theme: ThemeSnap { engine.chrome.theme }

    private var isLightTheme: Bool {
        let c = theme.editorBg
        let r = Double((c >> 16) & 0xFF)
        let g = Double((c >> 8) & 0xFF)
        let b = Double(c & 0xFF)
        return (0.299 * r + 0.587 * g + 0.114 * b) > 150
    }

    private var preferredScheme: ColorScheme? {
        switch appearanceMode {
        case "light": return .light
        case "dark": return .dark
        default: return nil
        }
    }

    private struct Page: Identifiable {
        let id: Int
        let title: String
        let symbol: String
        let tint: Color
    }

    private let pages: [Page] = [
        Page(id: 1, title: "General", symbol: "gearshape.fill", tint: .gray),
        Page(id: 0, title: "About", symbol: "info.circle.fill", tint: .blue),
        Page(id: 2, title: "Extensions", symbol: "puzzlepiece.extension.fill", tint: .purple),
        Page(id: 3, title: "Shortcuts", symbol: "keyboard.fill", tint: .indigo),
    ]

    var body: some View {
        HSplitView {
            settingsSidebar
                .frame(minWidth: 176, idealWidth: 188, maxWidth: 208)

            settingsDetail
                .frame(minWidth: 536, maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(minWidth: 760, minHeight: 520)
        .preferredColorScheme(preferredScheme)
        .background(
            ThemedWindowChrome(
                background: NSColor.windowBackgroundColor,
                light: preferredScheme == .light || (preferredScheme == nil && isLightTheme),
                identifier: WindowChrome.settingsIdentifier
            )
        )
        .onAppear {
            if !engine.chrome.settings.open { engine.openSettings() }
            // Land on General (not About) like Xcode.
            if s.pageIndex == 0 { engine.settingsGotoPage(1) }
            retheme()
        }
        .onDisappear {
            if engine.chrome.settings.open { engine.closeSettings() }
        }
        .onChange(of: engine.chrome.settings.open) { _, open in
            if !open { dismiss() }
        }
        .onChange(of: appearanceMode) { _, _ in retheme() }
    }

    // MARK: - Sidebar (System Settings style)

    private var settingsSidebar: some View {
        VStack(spacing: 0) {
            // App identity header
            HStack(spacing: 10) {
                if let ns = NSImage(named: "Suisei") {
                    Image(nsImage: ns)
                        .resizable()
                        .interpolation(.high)
                        .aspectRatio(1, contentMode: .fit)
                        .frame(width: 36, height: 36)
                        .shadow(color: .black.opacity(0.22), radius: 4, y: 1)
                } else {
                    Image(systemName: "sparkles")
                        .font(.system(size: 24))
                        .foregroundStyle(.secondary)
                        .frame(width: 36, height: 36)
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text("Suisei")
                        .font(.system(size: 13, weight: .semibold))
                    Text("Version \(EngineBridge.engineVersion)")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 14)
            .padding(.top, 12)
            .padding(.bottom, 10)

            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(pages) { page in
                        Button {
                            engine.settingsGotoPage(page.id)
                        } label: {
                            HStack(spacing: 8) {
                                Image(systemName: page.symbol)
                                    .font(.system(size: 11, weight: .semibold))
                                    .foregroundStyle(.white)
                                    .frame(width: 22, height: 22)
                                    .background(
                                        RoundedRectangle(cornerRadius: 4, style: .continuous)
                                            .fill(page.tint.gradient)
                                    )
                                Text(page.title)
                                    .font(.system(size: 13))
                                Spacer(minLength: 0)
                            }
                            .padding(.horizontal, 8)
                            .frame(height: 32)
                            .background(
                                RoundedRectangle(cornerRadius: 4, style: .continuous)
                                    .fill(
                                        s.pageIndex == page.id
                                            ? Color.accentColor.opacity(0.14)
                                            : Color.clear
                                    )
                            )
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(page.title)
                        .accessibilityValue(s.pageIndex == page.id ? "Selected" : "")
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
            }
        }
        .background(Color(nsColor: .controlBackgroundColor))
    }

    // MARK: - Detail

    private var settingsDetail: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Text(pages.first(where: { $0.id == s.pageIndex })?.title ?? "Settings")
                    .font(.title2.weight(.semibold))
                if s.dirty {
                    Text("Unsaved")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.orange)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(Color.orange.opacity(0.15)))
                        .transition(.opacity.combined(with: .scale(scale: 0.9)))
                } else if !s.status.isEmpty {
                    Text(s.status)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .transition(.opacity)
                }
                Spacer()
                // About, Extensions and Shortcuts are read-only surfaces.
                // Showing a permanently disabled Save button on every page
                // implied that those pages contained unsaved controls.
                if s.pageIndex == 1 {
                    Button("Save") {
                        engine.saveSettings()
                    }
                    .keyboardShortcut("s", modifiers: .command)
                    .disabled(!s.dirty)
                    .controlSize(.small)
                }
            }
            .animation(.snappy(duration: 0.2), value: s.dirty)
            .padding(.horizontal, 24)
            .padding(.top, 16)
            .padding(.bottom, 8)

            ScrollView {
                Form {
                    switch s.pageIndex {
                    case 0: aboutSections
                    case 2: extensionsSections
                    case 3: helpSections
                    default: generalSections
                    }
                }
                .formStyle(.grouped)
                .padding(.horizontal, 12)
                .padding(.bottom, 24)
                .frame(maxWidth: 720, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .center)
                .animation(nil, value: s.pageIndex)
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

    // MARK: About

    @ViewBuilder private var aboutSections: some View {
        Section {
            HStack(spacing: 16) {
                if let ns = NSImage(named: "Suisei") {
                    Image(nsImage: ns)
                        .resizable()
                        .interpolation(.high)
                        .aspectRatio(1, contentMode: .fit)
                        .frame(width: 64, height: 64)
                        .shadow(color: .black.opacity(0.25), radius: 8, y: 3)
                } else {
                    Image(systemName: "app.dashed")
                        .font(.system(size: 40))
                        .foregroundStyle(.secondary)
                        .frame(width: 64, height: 64)
                }
                VStack(alignment: .leading, spacing: 3) {
                    Text("Suisei").font(.title3.weight(.semibold))
                    Text("Version \(EngineBridge.engineVersion)").foregroundStyle(.secondary)
                    Text("Native macOS editor powered by the Suisei engine")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 6)
        }
        Section("Details") {
            LabeledContent("Engine", value: "suisei-core \(EngineBridge.engineVersion)")
            LabeledContent("Theme", value: theme.name)
            LabeledContent("Config", value: "~/.suisei.toml")
        }
    }

    // MARK: General

    @ViewBuilder private var generalSections: some View {
        Section {
            HStack(spacing: 10) {
                appearanceTile("system", "Auto", preview: .system)
                appearanceTile("light", "Light", preview: .light)
                appearanceTile("dark", "Dark", preview: .dark)
            }
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.vertical, 6)
        } header: {
            Text("Window Appearance")
        } footer: {
            Text("Affects window chrome and materials. The editor colors follow the theme below.")
        }

        Section {
            Picker("Theme", selection: themePicker) {
                ForEach(themeNames, id: \.self) { name in
                    Text(displayThemeName(name)).tag(name)
                }
            }
            .pickerStyle(.menu)

            themePreviewCard
        } header: {
            Text("Editor Theme")
        } footer: {
            Text("“Light” and “Dark” are the production defaults; the rest match the Suisei terminal themes.")
        }

        Section("Editor") {
            LazyVGrid(
                columns: [
                    GridItem(.flexible(), spacing: 8),
                    GridItem(.flexible(), spacing: 8),
                ],
                spacing: 8
            ) {
                ForEach(editorToggles) { row in
                    settingsToggleTile(row)
                }
            }
            if let tab = s.rows.first(where: { $0.label == "Tab width" }) {
                Picker("Tab width", selection: bindTabWidth(tab)) {
                    Text("2").tag(2)
                    Text("4").tag(4)
                    Text("8").tag(8)
                }
                .pickerStyle(.segmented)
            }
        }

        Section("Language Servers") {
            ForEach(s.rows.filter { $0.label == "LSP enabled" }) { row in
                Toggle("Enable LSP", isOn: bindToggle(row))
                    .toggleStyle(.switch)
                    .controlSize(.small)
            }
            LazyVGrid(
                columns: [
                    GridItem(.flexible(), spacing: 8),
                    GridItem(.flexible(), spacing: 8),
                    GridItem(.flexible(), spacing: 8),
                ],
                spacing: 6
            ) {
                ForEach(s.rows.filter { $0.label.hasPrefix("LSP ·") }) { row in
                    languageServerTile(row)
                }
            }
        }

        Section("Source Control") {
            HStack(spacing: 8) {
                ForEach(s.rows.filter {
                    $0.label.localizedCaseInsensitiveContains("workbench")
                        || $0.label.localizedCaseInsensitiveContains("SCM")
                }) { row in
                    Button {
                        engine.settingsSelect(row.id)
                        engine.settingsActivate(row.id)
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: row.label.localizedCaseInsensitiveContains("workbench")
                                  ? "arrow.triangle.branch" : "sidebar.left")
                                .foregroundStyle(Color.accentColor)
                            Text(clean(row.label))
                                .lineLimit(1)
                            Spacer(minLength: 4)
                            Image(systemName: "arrow.up.right")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        .padding(.horizontal, 10)
                        .frame(maxWidth: .infinity, minHeight: 32)
                        .background(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .fill(Color.primary.opacity(0.045))
                        )
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    /// Live swatch strip of the active theme (bg / fg / accent / syntax hues).
    private var themePreviewCard: some View {
        HStack(spacing: 0) {
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

            VStack(alignment: .trailing, spacing: 4) {
                ForEach(
                    [("Accent", theme.accent), ("Text", theme.fg),
                     ("Selection", theme.selection), ("Caret", theme.caret)],
                    id: \.0
                ) { name, packed in
                    HStack(spacing: 6) {
                        Text(name)
                            .font(.system(size: 9))
                            .foregroundStyle(.secondary)
                        Circle()
                            .fill(theme.color(packed))
                            .frame(width: 10, height: 10)
                            .overlay(Circle().strokeBorder(Color.primary.opacity(0.15), lineWidth: 0.5))
                    }
                }
            }
            .padding(.leading, 14)
        }
        .padding(.vertical, 4)
    }

    private func displayThemeName(_ raw: String) -> String {
        switch raw {
        case "light": return "Light (Default)"
        case "dark": return "Dark (Default)"
        default: return raw.prefix(1).uppercased() + raw.dropFirst()
        }
    }

    private enum PreviewKind { case system, light, dark }

    private func appearanceTile(_ key: String, _ title: String, preview: PreviewKind) -> some View {
        let on = appearanceMode == key
        return Button { appearanceMode = key } label: {
            VStack(spacing: 6) {
                ZStack(alignment: .topLeading) {
                    RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                        .fill(previewFill(preview))
                .frame(width: 82, height: 50)
                    HStack(spacing: 3) {
                        Circle().fill(.red.opacity(0.9)).frame(width: 6, height: 6)
                        Circle().fill(.yellow.opacity(0.9)).frame(width: 6, height: 6)
                        Circle().fill(.green.opacity(0.9)).frame(width: 6, height: 6)
                    }
                    .padding(7)
                }
                .overlay(
                    RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                        .strokeBorder(on ? Color.accentColor : Color.secondary.opacity(0.3),
                                      lineWidth: on ? 2.5 : 1)
                )
                .scaleEffect(on ? 1.0 : 0.97)
                Text(title).font(.caption).foregroundStyle(on ? .primary : .secondary)
            }
            .frame(width: 88)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .animation(.snappy(duration: 0.18), value: on)
    }

    private func previewFill(_ k: PreviewKind) -> some ShapeStyle {
        switch k {
        case .light:
            return AnyShapeStyle(LinearGradient(colors: [Color(white: 0.97), Color(white: 0.90)], startPoint: .top, endPoint: .bottom))
        case .dark:
            return AnyShapeStyle(LinearGradient(colors: [Color(white: 0.24), Color(white: 0.12)], startPoint: .top, endPoint: .bottom))
        case .system:
            return AnyShapeStyle(LinearGradient(colors: [Color(white: 0.93), Color(white: 0.16)], startPoint: .leading, endPoint: .trailing))
        }
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

    private func settingsToggleTile(_ row: SettingsRowItem) -> some View {
        HStack(spacing: 8) {
            Text(clean(row.label))
                .font(.system(size: 12))
                .lineLimit(1)
            Spacer(minLength: 4)
            Toggle("", isOn: bindToggle(row))
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
        }
        .padding(.horizontal, 10)
        .frame(minHeight: 34)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(Color.primary.opacity(0.045))
        )
    }

    private func languageServerTile(_ row: SettingsRowItem) -> some View {
        let language = clean(row.label)
            .replacingOccurrences(of: "LSP ·", with: "")
            .trimmingCharacters(in: .whitespaces)
        return HStack(spacing: 6) {
            Circle()
                .fill(row.value == "default" ? Color.secondary.opacity(0.45) : Color.accentColor)
                .frame(width: 5, height: 5)
            Text(language)
                .font(.system(size: 11))
                .lineLimit(1)
            Spacer(minLength: 2)
            Text(row.value)
                .font(.system(size: 9, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .padding(.horizontal, 8)
        .frame(minHeight: 28)
        .background(
            RoundedRectangle(cornerRadius: 5, style: .continuous)
                .fill(Color.primary.opacity(0.035))
        )
    }

    // MARK: Bindings

    private var themeNames: [String] {
        s.rows
            .filter { $0.value == "Enter to apply" || $0.label.contains("●") || $0.label.hasPrefix(" ") }
            .map { clean($0.label) }
            .filter { !$0.isEmpty }
    }

    private var themePicker: Binding<String> {
        Binding(
            get: { theme.name },
            set: { name in
                if let row = s.rows.first(where: { clean($0.label) == name }) {
                    engine.settingsSelect(row.id)
                    engine.settingsActivate(row.id)
                }
            }
        )
    }

    private var editorToggles: [SettingsRowItem] {
        let labels = [
            "Relative number", "Wrap lines", "Undo caching", "Clipboard sync", "Key hints",
        ]
        return s.rows.filter { labels.contains($0.label) }
    }

    private func bindToggle(_ row: SettingsRowItem) -> Binding<Bool> {
        Binding(
            get: { row.value == "on" },
            set: { _ in
                engine.settingsSelect(row.id)
                engine.settingsActivate(row.id)
            }
        )
    }

    private func bindTabWidth(_ row: SettingsRowItem) -> Binding<Int> {
        Binding(
            get: { Int(row.value) ?? 4 },
            set: { _ in
                engine.settingsSelect(row.id)
                engine.settingsActivate(row.id)
            }
        )
    }

    private func clean(_ label: String) -> String {
        label
            .replacingOccurrences(of: "●", with: "")
            .trimmingCharacters(in: .whitespaces)
    }
}
