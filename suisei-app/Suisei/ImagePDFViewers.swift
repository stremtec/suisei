//  ImagePDFViewers.swift
//  Images and PDFs in a pane, shaped like Preview.app.
//
//  From Preview: the document alone on the pane, page thumbnails down the left
//  of a PDF, transparency as a checkerboard, and the zoom controls in the
//  WINDOW's toolbar rather than in a bar of our own — that last one being the
//  part a hand-drawn bar kept getting wrong. See `ViewerControls`.
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

    @ObservedObject private var controls = EngineBridge.shared.viewerControls

    private var url: URL { URL(fileURLWithPath: path) }

    /// Nothing but the picture, on the editor's own background.
    ///
    /// The controls are in the window's toolbar (`ViewerControls`), which is
    /// where Preview keeps them and the only place they can be real toolbar
    /// items. What is left here is what the screenshot actually shows: the
    /// image, centred, with nothing drawn around it.
    var body: some View {
        GeometryReader { geo in
            let roomForInspector = geo.size.width >= ViewerInspector.minPaneWidth
            HStack(spacing: 0) {
                ZoomableImage(
                    image: image,
                    pixelSize: pixelSize,
                    zoom: $zoom,
                    liveZoom: $liveZoom,
                    palette: palette
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                if roomForInspector, controls.infoOpen, !sections.isEmpty {
                    Divider().overlay(palette.fg.opacity(0.10))
                    ViewerInspector(sections: sections, palette: palette)
                        .frame(width: ViewerInspector.width)
                }
            }
        }
        .background(palette.bg)
        .task(id: path) { await load() }
        .onAppear { claimToolbar() }
        .onDisappear { controls.release(.image) }
        // Written from `.onChange`, never from `body`: publishing into an
        // object the toolbar observes while that toolbar is being evaluated is
        // how "Modifying state during view update" gets earned.
        .onChange(of: liveZoom) { _, _ in pushZoom() }
        .onChange(of: zoom) { _, _ in pushZoom() }
    }

    private func claimToolbar() {
        controls.claim(.image)
        controls.perform = { cmd in
            switch cmd {
            case .zoomOut: zoom = max(0.05, (zoom ?? liveZoom) / 1.25)
            case .zoomIn: zoom = min(32, (zoom ?? liveZoom) * 1.25)
            case .fit: zoom = nil
            case .actual: zoom = 1
            }
        }
        pushZoom()
    }

    private func pushZoom() {
        // The live value, not the requested one: while fitting, the number has
        // to say what is on screen. Rounded here so the toolbar republishes
        // only when a digit moves rather than on every frame of a pinch.
        let label = "\(Int((liveZoom * 100).rounded()))%"
        if controls.zoomLabel != label { controls.zoomLabel = label }
        if controls.fitted != (zoom == nil) { controls.fitted = zoom == nil }
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
        // A wheel or pinch zoom is the user choosing a magnification, so it has
        // to leave fit mode — otherwise the next layout pass refits and undoes
        // the gesture as it happens.
        scroll.onMagnify = { [weak scroll] in
            guard let scroll else { return }
            context.coordinator.adopt(scroll.magnification)
        }
        scroll.onDoubleClick = { [weak scroll] in
            guard let scroll else { return }
            context.coordinator.toggleFit(currently: scroll.magnification)
        }
        context.coordinator.scroll = scroll
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let iv = scroll.documentView as? NSImageView else { return }
        let checker = NSColor(patternImage: Checkerboard.image(dark: palette.isDark))
        // The editor's own background, not a neutral grey field. Preview can
        // afford its own backdrop because it owns the window; a pane sits
        // inside an editor and a second background is just a grey box in it.
        scroll.backgroundColor = NSColor(palette.bg)

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

    func makeCoordinator() -> Coordinator { Coordinator(zoom: $zoom, liveZoom: $liveZoom) }

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
        private let zoom: Binding<CGFloat?>
        private let liveZoom: Binding<CGFloat>

        init(zoom: Binding<CGFloat?>, liveZoom: Binding<CGFloat>) {
            self.zoom = zoom
            self.liveZoom = liveZoom
        }

        /// Report a magnification the view produced itself, without asking for
        /// it back — `lastRequested` is set so `updateNSView` does not then
        /// re-apply the same number and fight the gesture.
        func adopt(_ m: CGFloat) {
            lastRequested = m
            Task { @MainActor [zoom, liveZoom] in
                if abs(liveZoom.wrappedValue - m) > 0.001 { liveZoom.wrappedValue = m }
                if zoom.wrappedValue != m { zoom.wrappedValue = m }
            }
        }

        func report(_ m: CGFloat) {
            Task { @MainActor [liveZoom] in
                if abs(liveZoom.wrappedValue - m) > 0.001 { liveZoom.wrappedValue = m }
            }
        }

        /// Preview's double-click: fitted → actual size, anything else → fit.
        func toggleFit(currently: CGFloat) {
            Task { @MainActor [zoom] in
                zoom.wrappedValue = zoom.wrappedValue == nil ? 1 : nil
            }
        }
    }
}

/// Preview's interaction set, which a bare `NSScrollView` does not have.
///
/// A scroll view scrolls, and that is all it does with a mouse: the wheel
/// moved the picture and dragging did nothing at all, which is the "이동시키는건
/// 더 이상함" — there was no panning to be odd about, only its absence. What
/// Preview does, and what this does now:
///
/// * **drag** pans, with the closed hand that says so
/// * **⌘ + wheel** zooms about the pointer, so the thing under the cursor
///   stays under it
/// * **wheel** alone scrolls, unchanged
/// * **double-click** toggles fit and actual size
///
/// Zooming about the pointer rather than about the centre is the part that
/// makes wheel-zoom feel right; centred zoom is what made it feel wrong.
private final class CenteringScrollView: NSScrollView {
    var onMagnify: (() -> Void)?
    var onDoubleClick: (() -> Void)?

    private var panning = false

    override func magnify(with event: NSEvent) {
        super.magnify(with: event)
        onMagnify?()
    }

    override func scrollWheel(with event: NSEvent) {
        guard event.modifierFlags.contains(.command) else {
            super.scrollWheel(with: event)
            return
        }
        let step = event.hasPreciseScrollingDeltas
            ? event.scrollingDeltaY / 180
            : event.scrollingDeltaY / 20
        let next = min(maxMagnification, max(minMagnification, magnification * (1 + step)))
        guard abs(next - magnification) > 0.0001 else { return }
        let at = contentView.convert(event.locationInWindow, from: nil)
        setMagnification(next, centeredAt: at)
        onMagnify?()
    }

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            onDoubleClick?()
            return
        }
        // Only pan when there is somewhere to pan to. Dragging an image that
        // already fits should do nothing, not jitter against the clamp.
        let doc = documentView?.frame.size ?? .zero
        let visible = contentView.documentVisibleRect.size
        panning = doc.width > visible.width + 1 || doc.height > visible.height + 1
        if panning { NSCursor.closedHand.push() } else { super.mouseDown(with: event) }
    }

    override func mouseDragged(with event: NSEvent) {
        guard panning else { return super.mouseDragged(with: event) }
        var origin = contentView.bounds.origin
        origin.x -= event.deltaX / magnification
        // The clip view is unflipped, so dragging the picture down means
        // moving the viewport up.
        origin.y += (isFlipped ? -event.deltaY : event.deltaY) / magnification
        contentView.setBoundsOrigin(contentView.constrainBoundsRect(
            NSRect(origin: origin, size: contentView.bounds.size)
        ).origin)
        reflectScrolledClipView(contentView)
    }

    override func mouseUp(with event: NSEvent) {
        if panning {
            panning = false
            NSCursor.pop()
        } else {
            super.mouseUp(with: event)
        }
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
    @State private var zoomCommand: PDFZoomCommand?

    @ObservedObject private var controls = EngineBridge.shared.viewerControls

    private var url: URL { URL(fileURLWithPath: path) }

    var body: some View {
        GeometryReader { geo in
            let wide = geo.size.width >= 520
            let roomForInspector = geo.size.width >= ViewerInspector.minPaneWidth
            HStack(spacing: 0) {
                PDFSurface(
                    document: document,
                    palette: palette,
                    showThumbnails: wide,
                    zoomCommand: $zoomCommand,
                    page: $page
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                if roomForInspector, controls.infoOpen, !sections.isEmpty {
                    Divider().overlay(palette.fg.opacity(0.10))
                    ViewerInspector(sections: sections, palette: palette)
                        .frame(width: ViewerInspector.width)
                }
            }
        }
        .background(palette.bg)
        .task(id: path) { await load() }
        .onAppear { claimToolbar() }
        .onDisappear { controls.release(.pdf) }
        .onChange(of: page) { _, _ in pushPage() }
        .onChange(of: pageCount) { _, _ in pushPage() }
    }

    private func claimToolbar() {
        controls.claim(.pdf)
        controls.fitted = true
        controls.zoomLabel = ""
        controls.perform = { cmd in
            switch cmd {
            case .zoomOut: zoomCommand = .out
            case .zoomIn: zoomCommand = .in
            case .fit, .actual: zoomCommand = .fit
            }
        }
        pushPage()
    }

    private func pushPage() {
        let label = pageCount > 0 ? "\(page) / \(pageCount)" : ""
        if controls.pageLabel != label { controls.pageLabel = label }
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
        pdf.backgroundColor = NSColor(palette.bg)
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
