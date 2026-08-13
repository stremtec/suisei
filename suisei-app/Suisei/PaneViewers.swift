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
        case .image, .pdf, .audio, .binary:
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
