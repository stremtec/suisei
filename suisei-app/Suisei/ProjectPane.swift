//  ProjectPane.swift
//  `project.suiseiprj`, as a screen rather than as raw JSON.
//
//  feature.txt ☐22, and it carries ☐"per-project settings" with it: a settings
//  screen with nothing on it is not worth opening, and both are the same file
//  with the same writer.
//
//  Three rules this pane obeys, and the first one is why it exists at all:
//
//  · **It never writes text.** A viewer pane's buffer is empty on purpose —
//    that is what stops ⌘S writing an empty document over a PNG, and this file
//    is committed to a repository where the same mistake would be worse. Every
//    change here asks core to write the project, and core owns the format.
//  · **Absence is not an opinion.** A setting the project does not mention is
//    inherited, and the screen says so in the control rather than showing a
//    default that looks decided.
//  · **The raw JSON stays reachable.** A screen cannot know every key a future
//    version will add, and someone will need to fix one by hand.
//
//  The widgets are the 3D workbench's — `WBSection`, `WBRow` — because the app
//  already has this shape twice and a third dialect would be one more thing to
//  learn.

import AppKit
import SwiftUI

struct ProjectSnap: Equatable {
    var ok = false
    var root = ""
    var name = ""
    var projectId = ""
    var schema: UInt32 = 1
    /// `nil` = the project does not set one, so the global setting stands.
    var tabWidth: Int?
    var lspServers: [(String, String)] = []

    static func == (a: ProjectSnap, b: ProjectSnap) -> Bool {
        a.ok == b.ok && a.root == b.root && a.name == b.name
            && a.projectId == b.projectId && a.schema == b.schema
            && a.tabWidth == b.tabWidth
            && a.lspServers.map(\.0) == b.lspServers.map(\.0)
            && a.lspServers.map(\.1) == b.lspServers.map(\.1)
    }
}

struct ProjectPaneViewer: View {
    let path: String
    let palette: ViewerPalette
    @ObservedObject private var engine = EngineBridge.shared

    @State private var snap = ProjectSnap()
    @State private var name = ""
    @State private var newLang = ""
    @State private var newCmd = ""

    private static let widths = [0, 2, 4, 8]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                header
                identity
                editorSettings
                languageServers
                escapeHatch
            }
            .padding(.vertical, 16)
            .frame(maxWidth: 620, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(palette.bg)
        .onAppear(perform: reload)
        // The file can be edited in another pane, or by a pull. Re-read when
        // anything in the app moves rather than trusting a snapshot taken once.
        .onChange(of: engine.chrome.filename) { _, _ in reload() }
        .onChange(of: engine.chrome.message) { _, _ in reload() }
    }

    private func reload() {
        snap = engine.loadProject()
        name = snap.name
    }

    private var header: some View {
        HStack(spacing: 10) {
            Image(systemName: "shippingbox")
                .font(.system(size: 22, weight: .light))
                .foregroundStyle(palette.accent)
            VStack(alignment: .leading, spacing: 2) {
                Text(snap.ok ? snap.name : "Not a project")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(palette.fg)
                Text(snap.ok ? snap.root : path)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(palette.dim)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
    }

    private var identity: some View {
        WBSection("Identity", "tag", palette: palette) {
            WBRow("Name", palette: palette) {
                TextField("", text: $name)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 11))
                    .frame(maxWidth: 260)
                    .onSubmit { engine.projectSetName(name); reload() }
            }
            WBRow("ID", palette: palette) {
                HStack(spacing: 6) {
                    // Read-only, and said so rather than shown in a field
                    // nobody may type in: the id survives a rename and a move,
                    // and anything that remembers this project remembers it.
                    Text(snap.projectId.isEmpty ? "—" : snap.projectId)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(palette.dim)
                        .textSelection(.enabled)
                    Button {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(snap.projectId, forType: .string)
                    } label: {
                        Image(systemName: "doc.on.doc").font(.system(size: 9))
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(palette.dim)
                    .help("Copy")
                }
            }
            WBRow("Schema", palette: palette) {
                Text("\(snap.schema)")
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(palette.dim)
            }
        }
    }

    private var editorSettings: some View {
        WBSection("Editor", "text.alignleft", palette: palette) {
            WBRow("Indent", palette: palette) {
                VStack(alignment: .leading, spacing: 4) {
                    Picker("", selection: Binding(
                        get: { snap.tabWidth ?? 0 },
                        set: { engine.projectSetTabWidth($0); reload() }
                    )) {
                        // "Inherit" is a real choice and comes first: a
                        // project that has no opinion about indentation must
                        // be able to say so, and saying nothing is how.
                        Text("Inherit").tag(0)
                        ForEach(Self.widths.dropFirst(), id: \.self) { w in
                            Text("\(w) spaces").tag(w)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(maxWidth: 280)
                    Text(snap.tabWidth == nil
                         ? "Everyone keeps their own setting."
                         : "Everyone who clones this repository indents by \(snap.tabWidth!).")
                        .font(.system(size: 10))
                        .foregroundStyle(palette.dim)
                }
            }
        }
    }

    private var languageServers: some View {
        WBSection("Language Servers", "gearshape.2", palette: palette) {
            VStack(alignment: .leading, spacing: 6) {
                if snap.lspServers.isEmpty {
                    Text("None. Each person's own settings decide.")
                        .font(.system(size: 10))
                        .foregroundStyle(palette.dim)
                        .padding(.horizontal, 10)
                }
                ForEach(snap.lspServers, id: \.0) { lang, cmd in
                    WBRow(lang, palette: palette) {
                        HStack(spacing: 6) {
                            Text(cmd)
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundStyle(palette.fg.opacity(0.85))
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Spacer(minLength: 0)
                            Button {
                                engine.projectSetLsp(lang, "")
                                reload()
                            } label: {
                                Image(systemName: "minus.circle").font(.system(size: 10))
                            }
                            .buttonStyle(.plain)
                            .foregroundStyle(palette.dim)
                            .help("Remove")
                        }
                    }
                }
                HStack(spacing: 6) {
                    TextField("language id", text: $newLang)
                        .textFieldStyle(.roundedBorder)
                        .font(.system(size: 10))
                        .frame(width: 110)
                    TextField("command", text: $newCmd)
                        .textFieldStyle(.roundedBorder)
                        .font(.system(size: 10, design: .monospaced))
                    Button("Add") {
                        engine.projectSetLsp(newLang, newCmd)
                        newLang = ""
                        newCmd = ""
                        reload()
                    }
                    .controlSize(.small)
                    .disabled(newLang.trimmingCharacters(in: .whitespaces).isEmpty
                              || newCmd.trimmingCharacters(in: .whitespaces).isEmpty)
                }
                .padding(.horizontal, 10)
                .padding(.top, 2)
            }
        }
    }

    /// A screen cannot know every key a later version will add, and someone
    /// will need to fix one by hand. So the file itself stays one click away.
    private var escapeHatch: some View {
        HStack {
            Spacer()
            Button("View as JSON") {
                engine.openProjectAsText(path)
            }
            .controlSize(.small)
            .help("Open the file itself, in the editor")
        }
        .padding(.horizontal, 16)
    }
}
