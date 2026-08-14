//  PaneViewers.swift
//  What goes in a pane when the pane is not a text editor.
//
//  A pane's `kind` (`PaneKind`, from `suisei_core::media::FileKind`) is decided
//  once in core, when the document is opened, and travels in the byte that used
//  to carry `is_terminal`. Everything here reads that one value and nothing
//  else — no second sniff, no extension matching in the face.
//
//  These viewers draw from the FILE, not from the buffer. A viewer pane's
//  buffer is deliberately empty: core refuses to decode a PNG into text, and
//  refuses to save text back over it. So `EditorPaneSnap.path` is the input,
//  and AppKit takes it from there.

import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Routes a non-text pane to its viewer.
///
/// `.binary` lands on `FilePlaceholderView`, which is not a fallback: it is
/// the Xcode treatment — the file's real icon, what it is, how big, and the
/// things you can actually do with it — and it is the whole correct answer for
/// a file with nothing to display.
struct PaneViewer: View {
    let kind: PaneKind
    let path: String
    let tabId: UInt64
    let palette: ViewerPalette
    let audioPlayer: AudioPlayerModel

    var body: some View {
        switch kind {
        case .text, .terminal:
            // Not ours. The caller routes these; this is here so the switch is
            // total and a new kind is a compile error rather than a blank pane.
            Color.clear
        case .audio:
            AudioViewer(path: path, tabId: tabId, palette: palette, model: audioPlayer)
        case .image:
            ImagePaneViewer(path: path, palette: palette)
        case .pdf:
            PDFPaneViewer(path: path, palette: palette)
        case .binary:
            FilePlaceholderView(path: path, kind: kind, palette: palette)
        }
    }
}

/// The colours a viewer needs, lifted out of `ContentView`'s theme so these
/// views can be previewed and reasoned about on their own.
struct ViewerPalette: Equatable {
    var fg: Color
    var dim: Color
    var accent: Color
    var bg: Color
}

// MARK: - Controls, hoisted to the window's toolbar

/// The focused viewer pane's controls, published where the WINDOW toolbar can
/// reach them.
///
/// A pane cannot have a toolbar. The look the user is after comes from macOS
/// wrapping a real `NSToolbarItem` in an `NSGlassEffectView` and grouping a
/// run of them into one platter, and nothing short of being a toolbar item
/// gets it — `editorToolbar` says exactly that, and it was measured.
///
/// So the controls go where they can be toolbar items. This is also where
/// Preview keeps them: its zoom buttons are in the window's toolbar, not
/// floating over the page. A hand-drawn bar inside the pane was reproducing
/// the wrong part of the screenshot.
///
/// Only the focused pane fills this in. Two viewer panes in a split have one
/// toolbar between them, which is the same answer every document app gives.
///
/// Not `@MainActor`-annotated, matching `MenuState`: `EngineBridge` builds its
/// small published objects in a synchronous initialiser, and every write here
/// comes from a view callback that is already on the main thread.
final class ViewerControls: ObservableObject {
    /// What the toolbar can ask for. Deliberately vague about what it MEANS —
    /// "reset" is fit for an image, fit for a PDF and default size for audio,
    /// and the toolbar has no business knowing which. The viewer that claimed
    /// the controls says what the button looks like.
    enum Command { case zoomIn, zoomOut, reset }

    /// Nil when nothing that owns these controls is on screen — the toolbar
    /// items are absent, not disabled.
    @Published var kind: PaneKind?
    /// Whether this viewer has anything to zoom. A binary tile does not.
    @Published var canZoom = false
    /// Already rounded, so a pinch only republishes when the number the user
    /// can read actually changes.
    @Published var zoomLabel = ""
    @Published var pageLabel = ""
    @Published var resetSymbol = "arrow.up.left.and.arrow.down.right"
    @Published var resetHelp = "화면에 맞추기"
    /// The facts the file inspector shows while this viewer is up.
    @Published var sections: [ViewerInfoSection] = []

    /// Set by the viewer that currently owns the toolbar. Held as a closure
    /// rather than as more `@Published` state so a button press does not have
    /// to round-trip through a value the view then has to notice and clear.
    var perform: ((Command) -> Void)?

    func claim(_ kind: PaneKind, canZoom: Bool) {
        if self.kind != kind { self.kind = kind }
        if self.canZoom != canZoom { self.canZoom = canZoom }
    }

    /// Give the toolbar back. Guarded on the kind so a pane that is going away
    /// cannot clear the controls of one that has just taken over.
    func release(_ kind: PaneKind) {
        guard self.kind == kind else { return }
        self.kind = nil
        canZoom = false
        zoomLabel = ""
        pageLabel = ""
        sections = []
        perform = nil
    }

    func setSections(_ next: [ViewerInfoSection]) {
        // Cheap identity check — these are rebuilt on load, not per frame, and
        // the inspector republishing on an unchanged list is pure waste.
        if sections.count != next.count
            || zip(sections, next).contains(where: { $0.title != $1.title
                || $0.rows.count != $1.rows.count })
        {
            sections = next
            return
        }
        if zip(sections, next).contains(where: { a, b in
            zip(a.rows, b.rows).contains { $0.label != $1.label || $0.value != $1.value }
        }) {
            sections = next
        }
    }
}

// MARK: - Inspector

struct ViewerInfoRow: Identifiable, Equatable {
    var id: String { label }
    let label: String
    let value: String
}

struct ViewerInfoSection: Identifiable, Equatable {
    var id: String { title }
    let title: String
    let rows: [ViewerInfoRow]

    /// Skips rows with nothing in them, so a caller can list everything it
    /// might know without guarding each line.
    init(_ title: String, _ rows: [(String, String?)]) {
        self.title = title
        self.rows = rows.compactMap { label, value in
            guard let value, !value.isEmpty else { return nil }
            return ViewerInfoRow(label: label, value: value)
        }
    }
}

// MARK: - The Xcode treatment

/// A file the editor will not open as text: its icon, its identity, and the
/// two or three things that are actually useful to do with it.
///
/// The icon is `NSWorkspace.icon(forFile:)` — the same one Finder draws, which
/// means a Quick Look thumbnail for anything macOS can preview and the generic
/// rounded document tile (with the extension written across it) for anything
/// else. Drawing our own tile would have been a worse copy of a picture the
/// system already has.
struct FilePlaceholderView: View {
    let path: String
    let kind: PaneKind
    let palette: ViewerPalette

    @State private var icon: NSImage?
    @State private var typeName: String = ""
    @State private var sizeText: String = ""
    @State private var isExecutable = false
    @ObservedObject private var controls = EngineBridge.shared.viewerControls

    private var url: URL { URL(fileURLWithPath: path) }

    var body: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)
            iconTile
            Text(url.lastPathComponent)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(palette.fg)
                .lineLimit(1)
                .truncationMode(.middle)
                .padding(.top, 18)
            Text(subtitle)
                .font(.system(size: 11))
                .foregroundStyle(palette.dim)
                .padding(.top, 3)
            actions
                .padding(.top, 22)
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(40)
        .background(palette.bg)
        // Re-read when the pane is pointed at a different file. Without the
        // id, SwiftUI reuses the view and the second binary you open wears the
        // first one's icon.
        .task(id: path) { load() }
        // No zoom — a tile has one size — but it has as much to say in the
        // File tab as anything else does.
        .onAppear { controls.claim(kind, canZoom: false) }
        .onDisappear { controls.release(kind) }
    }

    private var iconTile: some View {
        Group {
            if let icon {
                Image(nsImage: icon)
                    .resizable()
                    .interpolation(.high)
                    .aspectRatio(contentMode: .fit)
            } else {
                // Only visible for the instant before `load()` runs, and only
                // for a file the workspace has no icon for at all.
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(palette.dim.opacity(0.12))
            }
        }
        .frame(width: 128, height: 128)
        .shadow(color: .black.opacity(0.18), radius: 8, y: 3)
    }

    private var subtitle: String {
        [typeName, sizeText].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    @ViewBuilder private var actions: some View {
        HStack(spacing: 10) {
            // Xcode's own button for an executable, and it does what Xcode's
            // does: hands the file to Terminal, which runs it. Deliberately
            // absent for everything else — "run this" is not an offer to make
            // about a file that was never going to be run.
            if isExecutable {
                ViewerButton(title: "Open with Terminal", systemImage: "terminal", prominent: true,
                             palette: palette) {
                    let terminal = URL(fileURLWithPath: "/System/Applications/Utilities/Terminal.app")
                    NSWorkspace.shared.open([url], withApplicationAt: terminal,
                                            configuration: NSWorkspace.OpenConfiguration())
                }
            } else {
                ViewerButton(title: "Open with Default App", systemImage: "arrow.up.forward.app",
                             prominent: true, palette: palette) {
                    NSWorkspace.shared.open(url)
                }
            }
            ViewerButton(title: "Reveal in Finder", systemImage: "folder", prominent: false,
                         palette: palette) {
                NSWorkspace.shared.activateFileViewerSelecting([url])
            }
        }
    }

    private func load() {
        guard !path.isEmpty else { return }
        // `icon(forFile:)` is synchronous and hits the icon services cache; at
        // 128pt it is a lookup, not a render. The size has to be set on the
        // NSImage or SwiftUI scales the 32pt representation up into mush.
        let img = NSWorkspace.shared.icon(forFile: path)
        img.size = NSSize(width: 256, height: 256)
        icon = img

        let values = try? url.resourceValues(forKeys: [.contentTypeKey, .fileSizeKey])
        typeName = values?.contentType?.localizedDescription
            ?? UTType(filenameExtension: url.pathExtension)?.localizedDescription
            ?? kind.viewerNoun
        if let bytes = values?.fileSize {
            sizeText = ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
        }
        isExecutable = FileManager.default.isExecutableFile(atPath: path)
            && !url.hasDirectoryPath

        let extra = try? url.resourceValues(forKeys: [
            .creationDateKey, .contentModificationDateKey,
        ])
        let df = DateFormatter()
        df.dateStyle = .medium
        df.timeStyle = .short
        controls.setSections([
            ViewerInfoSection("File", [
                ("Name", url.lastPathComponent),
                ("Kind", typeName),
                ("Size", sizeText),
                ("Executable", isExecutable ? "Yes" : nil),
                ("Created", extra?.creationDate.map { df.string(from: $0) }),
                ("Modified", extra?.contentModificationDate.map { df.string(from: $0) }),
            ]),
            ViewerInfoSection("Location", [
                ("Where", url.deletingLastPathComponent().path),
            ]),
        ])
    }
}

/// A capsule button in the pane's own colours — `.borderedProminent` uses the
/// system accent, which is the one colour on screen that has nothing to do
/// with the user's chosen theme.
private struct ViewerButton: View {
    let title: String
    let systemImage: String
    let prominent: Bool
    let palette: ViewerPalette
    let action: () -> Void

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: systemImage)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(prominent ? Color.white : palette.fg)
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
                .background(
                    Capsule().fill(
                        prominent
                            ? palette.accent.opacity(hovering ? 1.0 : 0.88)
                            : palette.fg.opacity(hovering ? 0.16 : 0.10)
                    )
                )
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

extension PaneKind {
    /// A fallback name, used only when the system has no localized description
    /// for the file's type — which happens for a file with no extension and no
    /// recognisable magic, i.e. exactly the compiled-binary case.
    var viewerNoun: String {
        switch self {
        case .text: return "Text File"
        case .terminal: return "Terminal"
        case .image: return "Image"
        case .pdf: return "PDF Document"
        case .audio: return "Audio"
        case .binary: return "Binary File"
        }
    }
}
