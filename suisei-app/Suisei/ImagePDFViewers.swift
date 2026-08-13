//  ImagePDFViewers.swift
//  Images and PDFs in a pane, shaped like Preview.app.
//
//  From Preview: the document on a neutral backdrop rather than on the app's
//  own background, a top strip naming what it is with the controls at the far
//  right, page thumbnails down the left of a PDF, and zoom that lands on
//  round numbers.
//
//  Neither surface is drawn by hand. A PDF is `PDFView` and `PDFThumbnailView`,
//  which is what Preview itself uses; an image is `NSImageView` inside a
//  magnifying `NSScrollView`. Reimplementing either would be a worse copy of
//  something already on the machine — the same argument as the binary tile
//  taking its icon from `NSWorkspace`.

import AppKit
import ImageIO
import PDFKit
import SwiftUI
import UniformTypeIdentifiers

// MARK: - Image

struct ImagePaneViewer: View {
    let path: String
    let palette: ViewerPalette

    @State private var image: NSImage?
    @State private var sections: [ViewerInfoSection] = []
    @State private var pixelSize: CGSize = .zero
    /// Nil means "fit" — the scroll view picks the magnification and keeps
    /// picking it as the pane resizes. A number means the user chose.
    @State private var zoom: CGFloat?
    @State private var liveZoom: CGFloat = 1
    @State private var showInspector = true

    private var url: URL { URL(fileURLWithPath: path) }

    var body: some View {
        GeometryReader { geo in
            let roomForInspector = geo.size.width >= ViewerInspector.minPaneWidth
            VStack(spacing: 0) {
                ViewerTopBar(label: dimensionLine, palette: palette) {
                    ViewerToolBarGroups {
                        zoomControls
                        if roomForInspector {
                            ViewerToolGroup {
                                ViewerIconButton(
                                    symbol: "sidebar.right", help: "정보",
                                    active: showInspector, palette: palette
                                ) { showInspector.toggle() }
                            }
                        }
                    }
                }
                Divider().overlay(palette.fg.opacity(0.10))
                HStack(spacing: 0) {
                    ZoomableImage(
                        image: image,
                        pixelSize: pixelSize,
                        zoom: $zoom,
                        liveZoom: $liveZoom,
                        palette: palette
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    if roomForInspector, showInspector, !sections.isEmpty {
                        Divider().overlay(palette.fg.opacity(0.10))
                        ViewerInspector(sections: sections, palette: palette)
                            .frame(width: ViewerInspector.width)
                    }
                }
            }
        }
        .background(palette.bg)
        .task(id: path) { await load() }
    }

    private var dimensionLine: String {
        guard pixelSize != .zero else { return "" }
        return "\(Int(pixelSize.width)) × \(Int(pixelSize.height))"
    }

    @ViewBuilder private var zoomControls: some View {
        ViewerToolGroup {
            ViewerIconButton(symbol: "minus.magnifyingglass", help: "축소", palette: palette) {
                zoom = max(0.05, (zoom ?? liveZoom) / 1.25)
            }
            // The live value, not the requested one: while fitting, the number
            // has to say what is actually on screen.
            Text("\(Int((liveZoom * 100).rounded()))%")
                .font(.system(size: 10.5, weight: .medium).monospacedDigit())
                .foregroundStyle(palette.fg.opacity(0.7))
                .frame(width: 40)
            ViewerIconButton(symbol: "plus.magnifyingglass", help: "확대", palette: palette) {
                zoom = min(32, (zoom ?? liveZoom) * 1.25)
            }
        }
        ViewerToolGroup {
            ViewerIconButton(
                symbol: "arrow.up.left.and.arrow.down.right",
                help: "화면에 맞추기", active: zoom == nil, palette: palette
            ) { zoom = nil }
            ViewerIconButton(symbol: "1.magnifyingglass", help: "실제 크기", palette: palette) {
                zoom = 1
            }
        }
    }

    private func load() async {
        // Reset first: without it, a switch between two images shows the old
        // one's dimensions against the new one's pixels until the read lands.
        image = nil
        sections = []
        pixelSize = .zero
        zoom = nil

        let url = self.url
        let loaded: (NSImage?, CGSize, [ViewerInfoSection]) = await Task.detached(
            priority: .userInitiated
        ) {
            let img = NSImage(contentsOf: url)
            let (size, info) = Self.probe(url)
            return (img, size, info)
        }.value
        image = loaded.0
        pixelSize = loaded.1
        sections = loaded.2
    }

    /// Everything `ImageIO` knows, without decoding the pixels.
    ///
    /// `NSImage.size` is in points and reflects the DPI, so a 300 dpi scan
    /// reports a third of its pixels — the inspector must not get its numbers
    /// from there.
    private nonisolated static func probe(_ url: URL) -> (CGSize, [ViewerInfoSection]) {
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
              let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil)
              as? [CFString: Any]
        else {
            return (.zero, [fileSection(url)])
        }
        let w = props[kCGImagePropertyPixelWidth] as? Int ?? 0
        let h = props[kCGImagePropertyPixelHeight] as? Int ?? 0
        let dpiX = props[kCGImagePropertyDPIWidth] as? Double
        let depth = props[kCGImagePropertyDepth] as? Int
        let model = (props[kCGImagePropertyColorModel] as? String)?
            .replacingOccurrences(of: "RGB", with: "RGB")
        let profile = props[kCGImagePropertyProfileName] as? String
        let alpha = props[kCGImagePropertyHasAlpha] as? Bool
        let orientation = props[kCGImagePropertyOrientation] as? Int
        let frames = CGImageSourceGetCount(src)

        var rows: [(String, String?)] = [
            ("Dimensions", w > 0 ? "\(w) × \(h)" : nil),
            ("Megapixels", w > 0 ? String(format: "%.1f MP", Double(w * h) / 1_000_000) : nil),
            ("Color Model", model),
            ("Profile", profile),
            ("Bit Depth", depth.map { "\($0)-bit" }),
            ("Alpha", alpha.map { $0 ? "Yes" : "No" }),
            ("Resolution", dpiX.map { "\(Int($0.rounded())) dpi" }),
        ]
        // Only worth a row when it says something: 1 is "as stored".
        if let o = orientation, o != 1 { rows.append(("Orientation", "\(o)")) }
        // An animated GIF or a multi-page TIFF.
        if frames > 1 { rows.append(("Frames", "\(frames)")) }

        return (CGSize(width: w, height: h), [
            ViewerInfoSection("Image", rows),
            fileSection(url),
        ])
    }

    fileprivate nonisolated static func fileSection(_ url: URL) -> ViewerInfoSection {
        let v = try? url.resourceValues(forKeys: [
            .fileSizeKey, .contentTypeKey, .creationDateKey, .contentModificationDateKey,
        ])
        let df = DateFormatter()
        df.dateStyle = .medium
        df.timeStyle = .short
        return ViewerInfoSection("File", [
            ("Kind", v?.contentType?.localizedDescription),
            ("Size", v?.fileSize.map {
                ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .file)
            }),
            ("Created", v?.creationDate.map { df.string(from: $0) }),
            ("Modified", v?.contentModificationDate.map { df.string(from: $0) }),
        ])
    }
}

/// `NSScrollView`'s own magnification, which brings pinch-to-zoom, momentum
/// panning and the scroller behaviour with it — none of which is worth
/// rebuilding on top of a SwiftUI `Image`.
private struct ZoomableImage: NSViewRepresentable {
    let image: NSImage?
    /// The image's size in PIXELS, which is not `NSImage.size`.
    ///
    /// `NSImage.size` is in points and divides out the DPI: a 600×600 scan at
    /// 300 dpi reports 144×144. Laying the document view out at that size
    /// would draw the image at 24% while the toolbar said 100%, and "actual
    /// size" would be neither. One image pixel to one point is what 100% has
    /// to mean here.
    let pixelSize: CGSize
    @Binding var zoom: CGFloat?
    @Binding var liveZoom: CGFloat
    let palette: ViewerPalette

    /// The pixel size when it is known, and the point size when it is not —
    /// a format `ImageIO` could not read still has to be drawn at some size.
    private var documentSize: CGSize {
        pixelSize.width > 0 && pixelSize.height > 0 ? pixelSize : (image?.size ?? .zero)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = CenteringScrollView()
        // Before `documentView`: swapping the clip view afterwards re-parents
        // the document and loses the scroll position.
        scroll.contentView = CenteringClipView()
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        scroll.allowsMagnification = true
        scroll.minMagnification = 0.05
        scroll.maxMagnification = 32
        scroll.drawsBackground = true
        scroll.borderType = .noBorder

        let iv = NSImageView()
        iv.imageScaling = .scaleProportionallyUpOrDown
        iv.animates = true
        // Preview shows transparency as a checkerboard rather than as the
        // window behind it, so a transparent PNG cannot be mistaken for a
        // white one on a light theme or a black one on a dark theme.
        iv.wantsLayer = true
        scroll.documentView = iv
        scroll.onMagnify = { [weak scroll] in
            guard let scroll else { return }
            context.coordinator.report(scroll.magnification)
        }
        context.coordinator.scroll = scroll
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let iv = scroll.documentView as? NSImageView else { return }
        let checker = NSColor(patternImage: Checkerboard.image(dark: palette.isDark))
        scroll.backgroundColor = NSColor(palette.bg).blended(withFraction: 0.5, of: .gray)
            ?? NSColor(palette.bg)

        if iv.image !== image || iv.frame.size != documentSize {
            iv.image = image
            iv.layer?.backgroundColor = image?.hasAlphaChannel == true
                ? checker.cgColor
                : nil
            // A new image starts fitted, whatever the last one was left at.
            iv.frame = NSRect(origin: .zero, size: documentSize)
            context.coordinator.lastRequested = nil
            fit(scroll, iv)
        }

        if zoom == nil {
            // Refit on every layout pass while in fit mode, so resizing the
            // pane keeps the image filling it.
            fit(scroll, iv)
        } else if let z = zoom, context.coordinator.lastRequested != z {
            context.coordinator.lastRequested = z
            scroll.setMagnification(z, centeredAt: visibleCentre(scroll))
            context.coordinator.report(z)
        }
    }

    private func visibleCentre(_ scroll: NSScrollView) -> NSPoint {
        let r = scroll.contentView.documentVisibleRect
        return NSPoint(x: r.midX, y: r.midY)
    }

    private func fit(_ scroll: NSScrollView, _ iv: NSImageView) {
        let size = documentSize
        guard size.width > 0, size.height > 0 else { return }
        let bounds = scroll.bounds.size
        guard bounds.width > 1, bounds.height > 1 else { return }
        let pad: CGFloat = 24
        let m = min(
            (bounds.width - pad) / size.width,
            (bounds.height - pad) / size.height
        )
        // Never enlarge to fit: Preview shows a 16×16 icon at 16×16, not
        // blown up to fill the window.
        let clamped = min(1, max(scroll.minMagnification, m))
        if abs(scroll.magnification - clamped) > 0.001 {
            scroll.magnification = clamped
        }
        Task { @MainActor in
            if abs(liveZoom - clamped) > 0.001 { liveZoom = clamped }
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator(liveZoom: $liveZoom) }

    /// Holds the binding, not the `ZoomableImage` that produced it.
    ///
    /// A coordinator is made once and kept; the struct around it is rebuilt on
    /// every update. Capturing the whole struct means reading last render's
    /// `palette` and `image` off a copy that has since been replaced — the
    /// binding is the only part that stays correct, so it is the only part
    /// worth keeping.
    final class Coordinator {
        weak var scroll: NSScrollView?
        var lastRequested: CGFloat?
        private let liveZoom: Binding<CGFloat>

        init(liveZoom: Binding<CGFloat>) { self.liveZoom = liveZoom }

        func report(_ m: CGFloat) {
            Task { @MainActor [liveZoom] in
                if abs(liveZoom.wrappedValue - m) > 0.001 { liveZoom.wrappedValue = m }
            }
        }
    }
}

private final class CenteringScrollView: NSScrollView {
    var onMagnify: (() -> Void)?

    override func magnify(with event: NSEvent) {
        super.magnify(with: event)
        onMagnify?()
    }
}

/// Keeps the document centred when it is smaller than the clip view, which is
/// the normal state for anything not zoomed past the pane.
///
/// The centring has to happen HERE and not by moving the document view's
/// frame. A scroll view positions its document by the clip view's bounds
/// origin, and `constrainBoundsRect` is asked for that origin on every scroll,
/// every magnification and every resize — so a frame nudged into place in
/// `layout()` is immediately overruled, and the document ends up wherever the
/// default clamp puts it. In unflipped coordinates that is the bottom, which
/// is exactly where the image was.
private final class CenteringClipView: NSClipView {
    override func constrainBoundsRect(_ proposedBounds: NSRect) -> NSRect {
        var rect = super.constrainBoundsRect(proposedBounds)
        guard let doc = documentView else { return rect }
        // `rect` is already in magnified coordinates; `doc.frame` is not
        // scaled, and comparing them is what decides "is there slack".
        if rect.width > doc.frame.width {
            rect.origin.x = (doc.frame.width - rect.width) / 2
        }
        if rect.height > doc.frame.height {
            rect.origin.y = (doc.frame.height - rect.height) / 2
        }
        return rect
    }
}

/// The transparency checkerboard, built once per shade.
private enum Checkerboard {
    nonisolated(unsafe) private static var cache: [Bool: NSImage] = [:]

    static func image(dark: Bool) -> NSImage {
        if let hit = cache[dark] { return hit }
        let s: CGFloat = 10
        let img = NSImage(size: NSSize(width: s * 2, height: s * 2))
        img.lockFocus()
        let light = dark ? NSColor(white: 0.24, alpha: 1) : NSColor(white: 1.00, alpha: 1)
        let shade = dark ? NSColor(white: 0.19, alpha: 1) : NSColor(white: 0.90, alpha: 1)
        light.setFill()
        NSRect(x: 0, y: 0, width: s * 2, height: s * 2).fill()
        shade.setFill()
        NSRect(x: 0, y: 0, width: s, height: s).fill()
        NSRect(x: s, y: s, width: s, height: s).fill()
        img.unlockFocus()
        cache[dark] = img
        return img
    }
}

// MARK: - PDF

struct PDFPaneViewer: View {
    let path: String
    let palette: ViewerPalette

    @State private var document: PDFDocument?
    @State private var sections: [ViewerInfoSection] = []
    @State private var page = 1
    @State private var pageCount = 0
    @State private var showThumbnails = true
    @State private var showInspector = false
    @State private var zoomCommand: PDFZoomCommand?

    private var url: URL { URL(fileURLWithPath: path) }

    var body: some View {
        GeometryReader { geo in
            let wide = geo.size.width >= 520
            let roomForInspector = geo.size.width >= ViewerInspector.minPaneWidth
            VStack(spacing: 0) {
                ViewerTopBar(
                    label: pageCount > 0 ? "\(page) / \(pageCount) 페이지" : "",
                    palette: palette
                ) {
                    ViewerToolBarGroups {
                        ViewerToolGroup {
                            ViewerIconButton(
                                symbol: "minus.magnifyingglass", help: "축소", palette: palette
                            ) { zoomCommand = .out }
                            ViewerIconButton(
                                symbol: "plus.magnifyingglass", help: "확대", palette: palette
                            ) { zoomCommand = .in }
                            ViewerIconButton(
                                symbol: "arrow.up.left.and.arrow.down.right",
                                help: "화면에 맞추기", palette: palette
                            ) { zoomCommand = .fit }
                        }
                        // The two panel toggles together, one on each side of
                        // the document, the way the window's own are.
                        if wide || roomForInspector {
                            ViewerToolGroup {
                                if wide {
                                    ViewerIconButton(
                                        symbol: "sidebar.leading", help: "축소판",
                                        active: showThumbnails, palette: palette
                                    ) { showThumbnails.toggle() }
                                }
                                if roomForInspector {
                                    ViewerIconButton(
                                        symbol: "sidebar.right", help: "정보",
                                        active: showInspector, palette: palette
                                    ) { showInspector.toggle() }
                                }
                            }
                        }
                    }
                }
                Divider().overlay(palette.fg.opacity(0.10))
                HStack(spacing: 0) {
                    PDFSurface(
                        document: document,
                        palette: palette,
                        showThumbnails: wide && showThumbnails,
                        zoomCommand: $zoomCommand,
                        page: $page
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    if roomForInspector, showInspector, !sections.isEmpty {
                        Divider().overlay(palette.fg.opacity(0.10))
                        ViewerInspector(sections: sections, palette: palette)
                            .frame(width: ViewerInspector.width)
                    }
                }
            }
        }
        .background(palette.bg)
        .task(id: path) { await load() }
    }

    private func load() async {
        document = nil
        sections = []
        page = 1
        pageCount = 0

        let url = self.url
        // Parsing a large PDF is not instant, and it has no business happening
        // on the thread that is drawing the pane.
        let loaded: (PDFDocument?, [ViewerInfoSection]) = await Task.detached(
            priority: .userInitiated
        ) {
            guard let doc = PDFDocument(url: url) else {
                return (nil, [ImagePaneViewer.fileSection(url)])
            }
            return (doc, Self.describe(doc, url: url))
        }.value
        document = loaded.0
        pageCount = loaded.0?.pageCount ?? 0
        sections = loaded.1
    }

    private nonisolated static func describe(
        _ doc: PDFDocument, url: URL
    ) -> [ViewerInfoSection] {
        let attrs = doc.documentAttributes ?? [:]
        func attr(_ k: PDFDocumentAttribute) -> String? {
            attrs[k] as? String
        }
        let df = DateFormatter()
        df.dateStyle = .medium
        df.timeStyle = .short

        var docRows: [(String, String?)] = [
            ("Pages", "\(doc.pageCount)"),
            ("Title", attr(.titleAttribute)),
            ("Author", attr(.authorAttribute)),
            ("Subject", attr(.subjectAttribute)),
            ("Creator", attr(.creatorAttribute)),
            ("Producer", attr(.producerAttribute)),
        ]
        if let d = attrs[PDFDocumentAttribute.creationDateAttribute] as? Date {
            docRows.append(("Created", df.string(from: d)))
        }
        if let d = attrs[PDFDocumentAttribute.modificationDateAttribute] as? Date {
            docRows.append(("Modified", df.string(from: d)))
        }
        // The first page's size in points, which is what "is this A4 or
        // Letter" comes down to.
        if let first = doc.page(at: 0) {
            let b = first.bounds(for: .mediaBox)
            docRows.append(("Page Size", String(
                format: "%.0f × %.0f pt", b.width, b.height
            )))
        }
        docRows.append(("Encrypted", doc.isEncrypted ? "Yes" : nil))
        docRows.append(("Locked", doc.isLocked ? "Yes" : nil))

        return [
            ViewerInfoSection("Document", docRows),
            ImagePaneViewer.fileSection(url),
        ]
    }
}

enum PDFZoomCommand { case `in`, out, fit }

/// `PDFView` plus `PDFThumbnailView` — the two views Preview is built out of.
private struct PDFSurface: NSViewRepresentable {
    let document: PDFDocument?
    let palette: ViewerPalette
    let showThumbnails: Bool
    @Binding var zoomCommand: PDFZoomCommand?
    @Binding var page: Int

    func makeNSView(context: Context) -> NSSplitView {
        let split = NSSplitView()
        split.isVertical = true
        split.dividerStyle = .thin

        let thumbs = PDFThumbnailView()
        thumbs.thumbnailSize = NSSize(width: 72, height: 92)
        thumbs.backgroundColor = .clear
        thumbs.setFrameSize(NSSize(width: 116, height: 100))

        let pdf = PDFView()
        pdf.autoScales = true
        pdf.displayMode = .singlePageContinuous
        pdf.displayDirection = .vertical
        pdf.displaysPageBreaks = true
        thumbs.pdfView = pdf

        split.addArrangedSubview(thumbs)
        split.addArrangedSubview(pdf)
        // The thumbnail column holds its width and the page area takes the
        // slack. `NSSplitView` gives extra space to the LOWEST holding
        // priority, so a sidebar has to sit above the content, not below it.
        split.setHoldingPriority(NSLayoutConstraint.Priority(260), forSubviewAt: 0)
        split.setHoldingPriority(NSLayoutConstraint.Priority(250), forSubviewAt: 1)

        context.coordinator.pdf = pdf
        context.coordinator.thumbs = thumbs
        NotificationCenter.default.addObserver(
            context.coordinator,
            selector: #selector(Coordinator.pageChanged),
            name: .PDFViewPageChanged,
            object: pdf
        )
        return split
    }

    func updateNSView(_ split: NSSplitView, context: Context) {
        guard let pdf = context.coordinator.pdf,
              let thumbs = context.coordinator.thumbs else { return }
        // Preview puts the page on a neutral field, a shade off the app's own
        // background, so the white of the paper reads as paper.
        let backdrop = NSColor(palette.bg).blended(withFraction: 0.5, of: .gray)
            ?? NSColor(palette.bg)
        pdf.backgroundColor = backdrop
        // `thumbs` IS `arrangedSubviews[0]`; hiding it is what collapses the
        // column, and `NSSplitView` redistributes on the next layout pass.
        if thumbs.isHidden == showThumbnails {
            thumbs.isHidden = !showThumbnails
            split.adjustSubviews()
        }

        if pdf.document !== document {
            pdf.document = document
            pdf.autoScales = true
            context.coordinator.push(page: 1, to: $page)
        }

        if let cmd = zoomCommand {
            switch cmd {
            case .in: pdf.zoomIn(nil)
            case .out: pdf.zoomOut(nil)
            case .fit: pdf.autoScales = true
            }
            // Zooming by hand ends auto-scaling, or the next layout pass undoes
            // what the button just did.
            if cmd != .fit { pdf.autoScales = false }
            Task { @MainActor in zoomCommand = nil }
        }
    }

    static func dismantleNSView(_ nsView: NSSplitView, coordinator: Coordinator) {
        NotificationCenter.default.removeObserver(coordinator)
    }

    func makeCoordinator() -> Coordinator { Coordinator(page: $page) }

    final class Coordinator: NSObject {
        weak var pdf: PDFView?
        weak var thumbs: PDFThumbnailView?
        private let pageBinding: Binding<Int>

        init(page: Binding<Int>) { self.pageBinding = page }

        func push(page: Int, to binding: Binding<Int>) {
            Task { @MainActor in
                if binding.wrappedValue != page { binding.wrappedValue = page }
            }
        }

        @objc func pageChanged() {
            guard let pdf, let current = pdf.currentPage,
                  let index = pdf.document?.index(for: current) else { return }
            push(page: index + 1, to: pageBinding)
        }
    }
}

// MARK: - Small helpers

extension ViewerPalette {
    /// Whether the pane's background is dark, which the checkerboard needs to
    /// know so a transparent image does not sit on a white grid in a dark
    /// theme.
    var isDark: Bool {
        let c = NSColor(bg).usingColorSpace(.sRGB) ?? .black
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        c.getRed(&r, green: &g, blue: &b, alpha: &a)
        return (0.299 * r + 0.587 * g + 0.114 * b) < 0.55
    }
}

extension NSImage {
    /// Whether any representation carries alpha — the checkerboard is only
    /// drawn for images that can actually show it.
    var hasAlphaChannel: Bool {
        representations.contains { ($0 as? NSBitmapImageRep)?.hasAlpha ?? false }
    }
}
