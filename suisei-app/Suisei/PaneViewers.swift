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
/// Every kind currently lands on `FilePlaceholderView`. That is not a stub: it
/// is the Xcode treatment — the file's real icon, what it is, how big, and the
/// things you can actually do with it — and it is the correct final answer for
/// `.binary`. Image, PDF and audio each replace their own branch here as their
/// surfaces land, and the app is usable at every step in between.
struct PaneViewer: View {
    let kind: PaneKind
    let path: String
    let palette: ViewerPalette

    var body: some View {
        switch kind {
        case .text, .terminal:
            // Not ours. The caller routes these; this is here so the switch is
            // total and a new kind is a compile error rather than a blank pane.
            Color.clear
        case .audio:
            AudioViewer(path: path, palette: palette)
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

// MARK: - Inspector

struct ViewerInfoRow: Identifiable {
    var id: String { label }
    let label: String
    let value: String
}

struct ViewerInfoSection: Identifiable {
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

/// What the file is, listed.
///
/// Apple's inspector shape — Preview's Info window, Music's Get Info: a
/// section title in small dim caps, then rows of a dim left label against a
/// right-aligned value in the document's ink. No separators, no boxes, no
/// alternating fill. The alignment does the work a rule would do, and the
/// panel stays quiet enough to sit beside the content without competing.
struct ViewerInspector: View {
    let sections: [ViewerInfoSection]
    let palette: ViewerPalette

    static let width: CGFloat = 230
    /// Below this the inspector is dropped rather than squeezed — two columns
    /// in a narrow split pane give neither one enough room.
    static let minPaneWidth: CGFloat = 620

    var body: some View {
        ScrollView(.vertical) {
            VStack(alignment: .leading, spacing: 18) {
                ForEach(sections) { section in
                    VStack(alignment: .leading, spacing: 6) {
                        Text(section.title.uppercased())
                            .font(.system(size: 9.5, weight: .semibold))
                            .tracking(0.6)
                            .foregroundStyle(palette.dim.opacity(0.75))
                        VStack(alignment: .leading, spacing: 4) {
                            ForEach(section.rows) { row in
                                if Self.isBlock(row.value) {
                                    // A credits list or a URL dump does not
                                    // belong in a right-aligned column — it
                                    // reads as ragged noise there. Same shape
                                    // as Get Info's Comments box: the label,
                                    // then the text under it, full width.
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(row.label)
                                            .font(.system(size: 11))
                                            .foregroundStyle(palette.dim)
                                        Text(row.value)
                                            .font(.system(size: 10.5))
                                            .foregroundStyle(palette.fg.opacity(0.9))
                                            .fixedSize(horizontal: false, vertical: true)
                                            .textSelection(.enabled)
                                    }
                                    .padding(.top, 2)
                                } else {
                                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                                        Text(row.label)
                                            .font(.system(size: 11))
                                            .foregroundStyle(palette.dim)
                                        Spacer(minLength: 6)
                                        Text(row.value)
                                            .font(.system(size: 11, weight: .medium))
                                            .foregroundStyle(palette.fg)
                                            .multilineTextAlignment(.trailing)
                                            // Wraps rather than truncates: a
                                            // long title is exactly what
                                            // someone opened this panel to read.
                                            .fixedSize(horizontal: false, vertical: true)
                                            .textSelection(.enabled)
                                    }
                                }
                            }
                        }
                    }
                }
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.never)
    }

    /// Too long or too many lines to sit in the value column.
    private static func isBlock(_ v: String) -> Bool {
        v.contains("\n") || v.count > 42
    }
}

// MARK: - Shared chrome

/// Preview.app's top strip: what the document is on the left, controls on the
/// right. One bar for both the image and the PDF surfaces, because in Preview
/// they are the same bar.
struct ViewerTopBar<Trailing: View>: View {
    let title: String
    let subtitle: String
    let palette: ViewerPalette
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(palette.fg)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.system(size: 10))
                        .foregroundStyle(palette.dim)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 12)
            trailing
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }
}

/// A plain icon control in the pane's own colours, sized for the top bar.
struct ViewerIconButton: View {
    let symbol: String
    var help: String = ""
    var active: Bool = false
    let palette: ViewerPalette
    let action: () -> Void

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(active ? palette.accent : palette.fg.opacity(0.85))
                .frame(width: 24, height: 22)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(active
                            ? palette.accent.opacity(0.16)
                            : palette.fg.opacity(hovering ? 0.10 : 0))
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(help)
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
