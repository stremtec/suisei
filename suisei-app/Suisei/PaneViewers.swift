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

// MARK: - Live reload marks

/// What a live reload did to a row.
enum LiveKind: UInt8 {
    case changed = 0
    case added = 1
    case removed = 2

    init(raw: UInt8) { self = LiveKind(rawValue: raw) ?? .changed }
}

/// The rows a live reload just replaced.
///
/// Its own `ObservableObject` for the reason `MenuState` is: this changes a
/// couple of times a minute at most, and putting it on the chrome would make
/// every reload republish the shell.
final class LiveMarks: ObservableObject {
    /// Row → what happened. Empty almost always.
    @Published private(set) var rows: [UInt32: LiveKind] = [:]
    /// Absolute path → when this process first saw it reload.
    ///
    /// Per file rather than per row, and it covers background tabs, which the
    /// row marks cannot: a row number means nothing for a buffer that is not
    /// on screen. This is what the project tree reads.
    @Published private(set) var files: [String: CFTimeInterval] = [:]
    /// When THIS process first saw each row marked.
    ///
    /// Kept here rather than taken from core for the same reason the
    /// breakpoint chips keep theirs: a row scrolled into view is not an
    /// arrival, and the fade should be the same length wherever the row was
    /// when the reload happened.
    private(set) var seenAt: [UInt32: CFTimeInterval] = [:]

    private var lastGen: UInt64 = 0
    /// Handed to whichever canvas is showing the live document, once. The
    /// slide belongs to a view, not to this list — but only this list knows
    /// where it should start.
    /// `rows` is signed: positive when lines arrived and the document has to
    /// make room, negative when they left and it has to close over the space.
    /// One value, because both are the same motion in opposite directions.
    private(set) var pendingShift: (below: Int, rows: Int)?

    func takePendingShift() -> (below: Int, rows: Int)? {
        defer { pendingShift = nil }
        return pendingShift
    }

    /// How much the document just grew or shrank, and when — for surfaces that
    /// have to move in step with the editor but are not the editor. Expires on
    /// its own; nothing consumes it.
    private(set) var growth: (rows: Int, start: CFTimeInterval)?

    static let shiftDuration: CFTimeInterval = 0.24

    /// 0…1 through the current grow/shrink, 1 when none is running.
    func growthProgress(now: CFTimeInterval = CACurrentMediaTime()) -> CGFloat {
        guard let g = growth else { return 1 }
        let t = min(1, max(0, (now - g.start) / Self.shiftDuration))
        return CGFloat(1 - pow(1 - t, 3))
    }

    /// 0 when the row is not flashing. Eased out — it leaves quickly and the
    /// tail is a whisper rather than a step to nothing.
    static let flashDuration: CFTimeInterval = 1.1
    /// Longer than a row's. A file nobody is looking at is worth pointing out
    /// for longer than one already on screen.
    static let fileFlashDuration: CFTimeInterval = 2.4

    func intensity(_ row: UInt32, now: CFTimeInterval = CACurrentMediaTime()) -> CGFloat {
        guard let start = seenAt[row] else { return 0 }
        let t = min(1, max(0, (now - start) / Self.flashDuration))
        return CGFloat(pow(1 - t, 2))
    }

    var isFlashing: Bool { !seenAt.isEmpty }

    /// 0 when this file is not flashing.
    func fileIntensity(_ path: String, now: CFTimeInterval = CACurrentMediaTime()) -> CGFloat {
        guard let start = files[path] else { return 0 }
        let t = min(1, max(0, (now - start) / Self.fileFlashDuration))
        return CGFloat(pow(1 - t, 2))
    }

    /// Pull only when core says the list moved. One `u64` read per tick
    /// otherwise, which is what makes this affordable at 120Hz.
    func poll(_ engine: OpaquePointer) {
        let gen = suisei_engine_live_gen(engine)
        if gen != lastGen {
            lastGen = gen
            let now = CACurrentMediaTime()

            var marks = [SuiseiLiveMarkC](repeating: SuiseiLiveMarkC(), count: 4096)
            let n = marks.withUnsafeMutableBufferPointer {
                suisei_engine_live_marks(engine, $0.baseAddress, UInt32($0.count))
            }
            var nextRows: [UInt32: LiveKind] = [:]
            var removedSpan: (row: Int, count: Int)?
            nextRows.reserveCapacity(Int(n))
            for i in 0..<Int(n) {
                let m = marks[i]
                nextRows[m.row] = LiveKind(raw: m.kind)
                if m.removed > 0 { removedSpan = (Int(m.row), Int(m.removed)) }
                if seenAt[m.row] == nil { seenAt[m.row] = now }
            }
            // How the document changed length, and from where. Added rows are
            // a contiguous run and the slide starts under them; a removal is
            // one mark carrying how many lines went.
            pendingShift = nil
            let added = nextRows.filter { $0.value == .added }.keys.sorted()
            if let first = added.first, let last = added.last,
               added.count == Int(last - first) + 1
            {
                pendingShift = (below: Int(last) + 1, rows: added.count)
            } else if let gone = removedSpan {
                pendingShift = (below: gone.row + 1, rows: -gone.count)
            }
            if let shift = pendingShift {
                growth = (rows: shift.rows, start: now)
            }
            rows = nextRows

            var buf = [CChar](repeating: 0, count: Int(SUISEI_LIVE_FILES_CAP))
            let count = buf.withUnsafeMutableBufferPointer {
                suisei_engine_live_files(engine, $0.baseAddress, UInt32($0.count))
            }
            var seenPaths: Set<String> = []
            if count > 0 {
                buf.withUnsafeBufferPointer { raw in
                    guard var p = raw.baseAddress else { return }
                    for _ in 0..<Int(count) {
                        let path = String(cString: p)
                        seenPaths.insert(path)
                        if files[path] == nil { files[path] = now }
                        p = p.advanced(by: strlen(p) + 1)
                    }
                }
            }
            // Core has forgotten the rest; so should we, or a path stays lit
            // until it happens to reload again.
            files = files.filter { seenPaths.contains($0.key) }
        }

        // Retire finished fades even when the list has not moved: the view
        // decides when a flash is over, and its length is not core's.
        let cutoff = CACurrentMediaTime()
        seenAt = seenAt.filter { cutoff - $0.value < Self.flashDuration }
        if let g = growth, cutoff - g.start >= Self.shiftDuration { growth = nil }
    }
}
