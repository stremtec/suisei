import AppKit
import CoreText
import QuartzCore
import SwiftUI

// MARK: - SwiftUI bridge
//
// Pull-based renderer: NSScrollView owns pixels; `draw(_:)` asks the engine
// for exactly the rows in the dirty rect (synchronous in-proc FFI). There is
// no push/merge/coverage machinery left — Core is the single source and the
// canvas is a dumb, always-consistent blitter. Responsive-scrolling overdraw
// works for free (prepareContent just draws a larger rect).
struct EditorHost: NSViewRepresentable {
    var hScroll: UInt32
    var wrapLines: Bool
    var docScroll: UInt32
    var docLineCount: UInt32
    /// Bumped by the engine on every content/paint change (frame gen).
    var contentGen: UInt64
    /// Why Core last moved the scroll (0 none, 1 restore, 2 navigate, 3 caret).
    var scrollIntent: UInt8
    var editorBg: Color
    var fg: Color
    var dim: Color
    var accent: Color
    var selBg: Color
    var caretColor: Color
    var gutterFg: Color
    var cursorLineBg: Color
    var theme: ThemeSnap
    var engine: EngineBridge
    var paneIndex: Int
    var showFocusRing: Bool

    func makeNSView(context: Context) -> EditorScrollView {
        let v = EditorScrollView()
        v.engine = engine
        v.paneIndex = paneIndex
        v.setContentHuggingPriority(.defaultLow, for: .horizontal)
        v.setContentHuggingPriority(.defaultLow, for: .vertical)
        v.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        v.setContentCompressionResistancePriority(.defaultLow, for: .vertical)
        return v
    }

    func updateNSView(_ view: EditorScrollView, context: Context) {
        view.engine = engine
        view.paneIndex = paneIndex
        view.apply(
            hScroll: hScroll,
            wrapLines: wrapLines,
            docScroll: docScroll,
            docLineCount: docLineCount,
            contentGen: contentGen,
            scrollIntent: scrollIntent,
            showFocusRing: showFocusRing,
            colors: EditorCanvasView.Colors(
                bg: NSColor(editorBg),
                fg: NSColor(fg),
                dim: NSColor(dim),
                accent: NSColor(accent),
                sel: NSColor(selBg),
                caret: NSColor(caretColor),
                gutter: NSColor(gutterFg),
                cursorLine: NSColor(cursorLineBg),
                keyword: NSColor(theme.color(theme.keyword)),
                string: NSColor(theme.color(theme.string)),
                comment: NSColor(theme.color(theme.comment)),
                number: NSColor(theme.color(theme.number)),
                typeName: NSColor(theme.color(theme.typeName)),
                function: NSColor(theme.color(theme.function))
            )
        )
    }
}

// MARK: - NSScrollView

final class EditorScrollView: NSScrollView {
    let canvas = EditorCanvasView()
    weak var engine: EngineBridge? {
        didSet { canvas.engine = engine }
    }
    var paneIndex: Int = 0 {
        didSet { canvas.paneIndex = paneIndex }
    }

    // (Minimap blur machinery removed — the strip is an opaque SwiftUI panel
    // now. Backdrop sampling was structurally impossible here and the
    // snapshot-blur was retired by design.)

    private(set) var isUserScrolling = false
    private var suppressPush = false
    private var lastDocLineCount: UInt32 = 0
    private var lastWrap = true
    private var lastContentGen: UInt64 = 0
    private var lastHCols: Int = 0
    /// Last position-only Core sync (covered scrolling).
    private var lastPosSyncTime: CFTimeInterval = 0

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        configure()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        configure()
    }

    private func configure() {
        drawsBackground = false
        borderType = .noBorder
        hasVerticalScroller = true
        hasHorizontalScroller = true
        autohidesScrollers = true
        scrollerStyle = .overlay
        horizontalScrollElasticity = .allowed
        verticalScrollElasticity = .allowed
        usesPredominantAxisScrolling = true
        wantsLayer = true
        contentView.wantsLayer = true
        contentView.postsBoundsChangedNotifications = true
        automaticallyAdjustsContentInsets = false
        contentInsets = .init()
        scrollerInsets = .init()
        documentView = canvas
        canvas.scrollView = self

        NotificationCenter.default.addObserver(
            self,
            selector: #selector(clipBoundsChanged(_:)),
            name: NSView.boundsDidChangeNotification,
            object: contentView
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(liveScrollWillStart(_:)),
            name: NSScrollView.willStartLiveScrollNotification,
            object: self
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(liveScrollDidEnd(_:)),
            name: NSScrollView.didEndLiveScrollNotification,
            object: self
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(externalScrollRequest(_:)),
            name: .suiseiScrollToLine,
            object: nil
        )
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
        if isUserScrolling { engine?.endLiveScroll() }
    }

    override var isFlipped: Bool { true }
    override class var isCompatibleWithResponsiveScrolling: Bool { true }


    @objc private func liveScrollWillStart(_ note: Notification) {
        if !isUserScrolling { engine?.beginLiveScroll() }
        isUserScrolling = true
        canvas.isLiveScrolling = true
    }

    @objc private func liveScrollDidEnd(_ note: Notification) {
        if isUserScrolling { engine?.endLiveScroll() }
        isUserScrolling = false
        canvas.isLiveScrolling = false
        canvas.cancelWheelSettleTimer()
        syncCorePosition(force: true)
    }

    /// Minimap / palette "reveal line" requests (focused pane only).
    @objc private func externalScrollRequest(_ note: Notification) {
        guard let line = note.userInfo?["line"] as? Int else { return }
        if let engine, engine.editorSplit.isSplit, paneIndex != engine.editorSplit.focus {
            return
        }
        scrollToLineAnimated(line, center: true)
    }

    func apply(
        hScroll: UInt32,
        wrapLines: Bool,
        docScroll: UInt32,
        docLineCount: UInt32,
        contentGen: UInt64,
        scrollIntent: UInt8,
        showFocusRing: Bool,
        colors: EditorCanvasView.Colors
    ) {
        let lineH = EditorMetrics.lineHeight
        let cell = EditorMetrics.cellWidth
        let count = max(1, Int(docLineCount))

        hasHorizontalScroller = !wrapLines
        horizontalScrollElasticity = wrapLines ? .none : .allowed

        let docH = max(bounds.height, CGFloat(count) * lineH + 8)
        var docW = max(bounds.width, 1)
        if !wrapLines {
            // Generous fixed budget — precise max width isn't known without a scan.
            let cols = max(400, Int(hScroll) + 160)
            docW = max(docW, CGFloat(cols) * cell + EditorMetrics.gutter + 32)
        }
        let newSize = NSSize(width: docW, height: docH)
        if abs(canvas.frame.width - newSize.width) > 0.5
            || abs(canvas.frame.height - newSize.height) > 0.5
        {
            // Resizing the canvas clamps the clip origin, which fires
            // `clipBoundsChanged` and pushes that clamped position back into
            // Core — wiping the scroll it had just restored for this tab. The
            // push has to stay muted across the resize; the clip is positioned
            // deliberately below.
            suppressPush = true
            canvas.setFrameSize(newSize)
            suppressPush = false
        }

        let docChanged = docLineCount != lastDocLineCount || wrapLines != lastWrap
        lastDocLineCount = docLineCount
        lastWrap = wrapLines

        canvas.setChrome(
            showFocusRing: showFocusRing,
            colors: colors,
            wrapLines: wrapLines,
            docLineCount: docLineCount
        )

        // Engine content advanced → drop the row cache and repaint the viewport.
        if contentGen != lastContentGen || docChanged {
            lastContentGen = contentGen
            canvas.noteContentChanged()
        }

        // Core states WHY it moved the scroll; the face just obeys. Guessing
        // from `abs(coreLine - clipLine)` could never separate "restore a tab"
        // (instant) from "jump to a symbol" (animate) — any threshold got one
        // of them wrong.
        let coreLine = Int(docScroll)
        let clipLine = Int(floor(documentVisibleRect.minY / max(1, lineH)))
        if !isUserScrolling, !canvas.isLiveScrolling, !canvas.isTrackingDrag,
           scrollIntent != 0
        {
            switch scrollIntent {
            case 2:  // navigate — outline, goto, search hit
                if coreLine != clipLine { scrollToLineAnimated(coreLine, center: false) }
            default: // restore / caret — be where you belong at once
                if coreLine != clipLine {
                    setClipTo(line: coreLine, hCols: wrapLines ? 0 : Int(hScroll))
                }
            }
            engine?.clearScrollIntent()
        } else if docChanged {
            setClipTo(line: coreLine, hCols: wrapLines ? 0 : Int(hScroll))
        }
    }

    private func setClipTo(line: Int, hCols: Int) {
        let lineH = EditorMetrics.lineHeight
        let cell = EditorMetrics.cellWidth
        suppressPush = true
        let wantY = CGFloat(line) * lineH
        let wantX = CGFloat(hCols) * cell
        let maxY = max(0, canvas.frame.height - contentView.bounds.height)
        let maxX = max(0, canvas.frame.width - contentView.bounds.width)
        contentView.setBoundsOrigin(NSPoint(
            x: min(max(0, wantX), maxX),
            y: min(max(0, wantY), maxY)
        ))
        reflectScrolledClipView(contentView)
        suppressPush = false
        // Announce it. `suppressPush` only mutes the write BACK to Core — the
        // minimap still has to learn where the clip went, or a tab restore
        // leaves its indicator parked at the previous file's position.
        NotificationCenter.default.post(
            name: .suiseiEditorScrolled, object: nil,
            userInfo: ["line": max(0, line)]
        )
    }

    /// Smooth ease-in-out glide to a buffer line (outline / minimap / goto).
    func scrollToLineAnimated(_ line: Int, center: Bool) {
        let lineH = EditorMetrics.lineHeight
        let visRows = Int(contentView.bounds.height / max(1, lineH))
        let target = center ? max(0, line - visRows / 2) : line
        let maxY = max(0, canvas.frame.height - contentView.bounds.height)
        let wantY = min(max(0, CGFloat(target) * lineH), maxY)
        let current = contentView.bounds.origin.y
        guard abs(wantY - current) > 0.5 else { return }
        suppressPush = true
        NSAnimationContext.runAnimationGroup({ ctx in
            // Distance-aware duration, decelerating curve — iOS feel.
            let dist = abs(wantY - current)
            ctx.duration = min(0.45, 0.18 + Double(dist) / 6000.0)
            ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            ctx.allowsImplicitAnimation = true
            contentView.animator().setBoundsOrigin(
                NSPoint(x: contentView.bounds.origin.x, y: wantY)
            )
            self.reflectScrolledClipView(self.contentView)
        }, completionHandler: { [weak self] in
            self?.suppressPush = false
            self?.syncCorePosition(force: true)
        })
    }

    /// Last line published to the minimap (dedup for per-frame posts).
    private var lastMinimapLine = -1

    @objc private func clipBoundsChanged(_ note: Notification) {
        postLiveMinimapLine()
        guard !suppressPush else { return }
        syncCorePosition(force: false)
    }

    /// Minimap indicator feed at FRAME rate — the 30Hz core-sync throttle made
    /// the indicator visibly step ("버벅"). Cheap: one int compare per event.
    private func postLiveMinimapLine() {
        let lineH = max(1, EditorMetrics.lineHeight)
        let v0 = max(0, Int(floor(documentVisibleRect.minY / lineH)))
        guard v0 != lastMinimapLine else { return }
        lastMinimapLine = v0
        if engine?.editorSplit.isSplit != true || paneIndex == engine?.editorSplit.focus {
            NotificationCenter.default.post(
                name: .suiseiEditorScrolled,
                object: nil,
                userInfo: ["line": v0]
            )
        }
    }

    /// Position-only Core sync — throttled ~30Hz; never recomposes.
    private func syncCorePosition(force: Bool) {
        let now = CACurrentMediaTime()
        if !force, now - lastPosSyncTime < (1.0 / 30.0) { return }
        lastPosSyncTime = now
        let lineH = max(1, EditorMetrics.lineHeight)
        let cell = max(1, EditorMetrics.cellWidth)
        let v0 = max(0, Int(floor(documentVisibleRect.minY / lineH)))
        let hCols = canvas.wrapLines
            ? 0
            : max(0, Int(floor(documentVisibleRect.minX / cell)))
        lastHCols = hCols
        if engine?.editorSplit.isSplit != true || paneIndex == engine?.editorSplit.focus {
            engine?.scrollSync(line: UInt32(v0), hscroll: UInt32(hCols))
        }
    }
}

extension Notification.Name {
    static let suiseiScrollToLine = Notification.Name("suisei.scrollToLine")
    /// Posted (throttled) as the clip scrolls — live feed for the minimap.
    static let suiseiEditorScrolled = Notification.Name("suisei.editorScrolled")
}

// MARK: - Document canvas (pull renderer)

final class EditorCanvasView: NSView {
    struct Colors {
        var bg: NSColor
        var fg: NSColor
        var dim: NSColor
        var accent: NSColor
        var sel: NSColor
        var caret: NSColor
        var gutter: NSColor
        var cursorLine: NSColor
        var keyword: NSColor
        var string: NSColor
        var comment: NSColor
        var number: NSColor
        var typeName: NSColor
        var function: NSColor
    }

    weak var engine: EngineBridge?
    weak var scrollView: EditorScrollView?
    var paneIndex: Int = 0

    private(set) var wrapLines: Bool = true
    private(set) var docLineCount: UInt32 = 1
    private(set) var showFocusRing: Bool = false
    var colors = Colors(
        bg: .textBackgroundColor, fg: .labelColor, dim: .secondaryLabelColor,
        accent: .controlAccentColor, sel: .selectedTextBackgroundColor,
        caret: .textColor, gutter: .tertiaryLabelColor, cursorLine: .quaternaryLabelColor,
        keyword: .systemCyan, string: .systemGreen, comment: .systemGray,
        number: .systemOrange, typeName: .systemTeal, function: .systemYellow
    )

    var isLiveScrolling = false
    private var wheelLiveEndWork: DispatchWorkItem?
    /// True while the mouse-tracking loop owns scrolling. `autoscroll(with:)`
    /// does NOT post live-scroll notifications, so without this the scroll
    /// view's follow-caret correction treats a drag-scroll as "the user isn't
    /// scrolling" and yanks the clip back to Core's line — the two fight, which
    /// is what made every autoscroll implementation feel broken.
    private(set) var isTrackingDrag = false
    private var tracking = false
    /// The pointer actually moved during this gesture — click vs drag.
    private var dragMoved = false
    /// Bracket hint: which match is showing, and since when. Xcode shows this
    /// as a brief flash rather than a persistent highlight, so the timing lives
    /// here — the core just reports the position (span kind 254).
    private var bracketKey: String = ""
    private var bracketShownAt: CFTimeInterval = 0
    /// Repaints the flash while it fades. Without frames the alpha is only
    /// evaluated once, so the hint appeared and then vanished in one step —
    /// a blink, not a fade.
    private var bracketFadeTimer: Timer?
    private static let bracketFlashDuration: CFTimeInterval = 0.9
    private static let bracketFadeTail: CFTimeInterval = 0.35
    private static let bracketPopDuration: CFTimeInterval = 0.26
    private var ctCache: [String: CTLine] = [:]
    private var ctCacheFontSize: CGFloat = 0
    private var colorGen: UInt64 = 0

    /// Row cache: contiguous band pulled from the engine (0-based start).
    private var bandStart: Int = 0
    private var bandRows: [EditorLine] = []
    private var bookmarkImage: NSImage?

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override class var isCompatibleWithResponsiveScrolling: Bool { true }

    // MARK: - AppKit text input state
    //
    // Typing used to be intercepted by a global NSEvent keyDown monitor, which
    // bypasses the input method entirely — that is why Hangul (and dead keys,
    // option-accents, the emoji picker) could never work. Keys now go through
    // `interpretKeyEvents`, so macOS itself runs the input method and hands us
    // committed text via `insertText` and editing intent via
    // `doCommandBySelector` — including the user's own key-binding overrides.

    /// In-progress input-method text. Lives ONLY here: composing text is not
    /// part of the document until the input method commits it.
    var markedText: String = ""
    /// Caret rect in view coords, captured while drawing — the input method
    /// needs it in screen coords to place the candidate window.
    var lastCaretRect: CGRect = .zero
    /// The key event being interpreted, so selectors we have not mapped yet can
    /// still fall back to the existing NSEvent path instead of being dropped.
    private var currentKeyEvent: NSEvent?
    /// Set when the input method consumed the current key (committed text or
    /// updated the composition). Replaying the raw event on top of that
    /// applies it twice — Enter confirming a candidate AND inserting a
    /// newline, which is what made the caret jump after Japanese input.
    fileprivate var inputHandledKey = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // Claim focus on launch. Otherwise SwiftUI hands first responder to the
        // first focusable view — the project-tree Filter field — so the first
        // thing typed goes into the filter instead of the document.
        guard paneIndex == 0, let win = window else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, self.window === win else { return }
            if !(win.firstResponder is EditorCanvasView) {
                win.makeFirstResponder(self)
            }
        }
    }

    override func keyDown(with event: NSEvent) {
        currentKeyEvent = event
        inputHandledKey = false
        defer { currentKeyEvent = nil }
        interpretKeyEvents([event])
    }

    /// Commit text into the document. Single characters keep the core's insert
    /// path (auto-pairing, auto-indent); multi-char commits — IME output, emoji
    /// picker, pasted runs — go in verbatim.
    fileprivate func commitText(_ text: String) {
        guard !text.isEmpty, let engine else { return }
        markedText = ""
        // Count SCALARS, not graphemes: one grapheme can be several scalars
        // (decomposed Hangul, ZWJ emoji), and sending only `.first` silently
        // dropped the rest — that is what mangled Korean input. Anything that
        // is not exactly one scalar goes in whole.
        let scalars = text.unicodeScalars
        if scalars.count == 1, let scalar = scalars.first,
           scalar.value != 0x0A, scalar.value != 0x0D
        {
            if engine.typeFast(ch: scalar.value) {
                // Fast path took it: drop the cached rows and repaint straight
                // from the engine, without waiting on a SwiftUI publish cycle.
                noteContentChanged()
                return
            }
            engine.dispatch(code: .char_, ch: scalar.value, mods: [])
        } else {
            engine.pasteText(text)
        }
        needsDisplay = true
    }

    fileprivate func fallbackToLegacyKeyPath() {
        guard let e = currentKeyEvent else { return }
        engine?.handleNSEvent(e)
        // Caret-only moves (arrows, Home/End) do not change the document, so
        // nothing else drops the cached row band — the canvas would keep
        // painting the old caret and the move looked like it was swallowed
        // until a later key finally forced a repaint.
        noteContentChanged()
    }

    func cancelWheelSettleTimer() {
        wheelLiveEndWork?.cancel()
        wheelLiveEndWork = nil
    }

    /// Engine content changed — drop cached rows and repaint the viewport.
    func noteContentChanged() {
        bandRows.removeAll(keepingCapacity: true)
        if let sv = scrollView {
            let pad = EditorMetrics.lineHeight * 4
            setNeedsDisplay(sv.documentVisibleRect.insetBy(dx: 0, dy: -pad))
        } else {
            needsDisplay = true
        }
    }

    func setChrome(
        showFocusRing: Bool,
        colors: Colors,
        wrapLines: Bool,
        docLineCount: UInt32
    ) {
        var repaint = false
        if !Self.colorsEqual(self.colors, colors) {
            colorGen &+= 1
            ctCache.removeAll(keepingCapacity: true)
            bookmarkImage = nil
            repaint = true
        }
        if self.showFocusRing != showFocusRing { repaint = true }
        self.showFocusRing = showFocusRing
        self.colors = colors
        self.wrapLines = wrapLines
        self.docLineCount = max(1, docLineCount)
        let fontSize = EditorMetrics.fontSize
        if fontSize != ctCacheFontSize {
            ctCacheFontSize = fontSize
            ctCache.removeAll(keepingCapacity: true)
            repaint = true
        }
        if repaint { noteContentChanged() }
    }

    /// Rows `[r0, r1]` from cache, pulling from the engine when uncovered.
    private func rows(_ r0: Int, _ r1: Int) -> ArraySlice<EditorLine> {
        let start = max(0, r0)
        let end = max(start, min(r1, Int(docLineCount) - 1))
        let covered = !bandRows.isEmpty
            && start >= bandStart
            && end < bandStart + bandRows.count
        if !covered {
            // Pull a padded band so small scrolls stay cache-hits.
            let pad = 24
            let want0 = max(0, start - pad)
            let want1 = end + pad
            var pulled: [EditorLine] = []
            var cursor = want0
            while cursor <= want1, let engine {
                let chunk = engine.pullBand(
                    pane: paneIndex,
                    start: cursor,
                    max: min(160, want1 - cursor + 1)
                )
                if chunk.isEmpty { break }
                pulled.append(contentsOf: chunk)
                // Wrap mode emits extra segment rows; advance by buffer rows consumed.
                let lastNo = chunk.last.map { Int($0.lineNo) } ?? cursor
                cursor = max(cursor + 1, lastNo + 1)
                if chunk.count < 4 { break }
            }
            bandStart = want0
            bandRows = pulled
        }
        // Slice by lineNo bounds (wrap rows share lineNo with their primary).
        let lo = bandRows.firstIndex { Int($0.lineNo) - 1 >= start } ?? bandRows.endIndex
        let hi = bandRows.lastIndex { Int($0.lineNo) - 1 <= end }.map { $0 + 1 } ?? lo
        return bandRows[lo..<hi]
    }

    private static func colorsEqual(_ a: Colors, _ b: Colors) -> Bool {
        a.bg == b.bg && a.fg == b.fg && a.dim == b.dim && a.accent == b.accent
            && a.sel == b.sel && a.caret == b.caret && a.gutter == b.gutter
            && a.cursorLine == b.cursorLine && a.keyword == b.keyword
            && a.string == b.string && a.comment == b.comment && a.number == b.number
            && a.typeName == b.typeName && a.function == b.function
    }

    private func cacheKey(for line: EditorLine) -> String {
        var sp = "\(line.lineNo)|\(line.text.count)|\(line.text.hashValue)|\(colorGen)"
        for s in line.spans {
            sp += "|\(s.start)-\(s.end):\(s.kind)"
        }
        return sp
    }

    // MARK: - Draw

    override func draw(_ dirtyRect: NSRect) {
        colors.bg.setFill()
        dirtyRect.fill()

        let lineH = EditorMetrics.lineHeight
        let gutter = EditorMetrics.gutter
        let fontSize = EditorMetrics.fontSize
        let cell = EditorMetrics.cellWidth
        let font = NSFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
        let ascent = font.ascender
        let gap = EditorMetrics.gutterTextGap
        guard let cg = NSGraphicsContext.current?.cgContext else { return }

        let r0 = max(0, Int(floor(dirtyRect.minY / lineH)))
        let r1 = max(r0, Int(ceil(dirtyRect.maxY / lineH)))
        let band = rows(r0, r1)

        // Wrapped primaries in this band (tails are clipped, marked with ⋯).
        var wrapped: Set<UInt32> = []
        if wrapLines {
            for line in band where line.isWrapContinuation {
                wrapped.insert(line.lineNo)
            }
        }

        for line in band {
            if line.isWrapContinuation { continue }
            let baseRow = max(0, Int(line.lineNo) - 1)
            let y = CGFloat(baseRow) * lineH
            if y + lineH < dirtyRect.minY || y > dirtyRect.maxY { continue }

            let rowRect = CGRect(x: 0, y: y, width: bounds.width, height: lineH)
            if line.isCursor {
                colors.cursorLine.setFill()
                rowRect.fill()
            }

            let gitKind = line.gitSignKind
            if gitKind != 0 {
                gitColor(gitKind).setFill()
                CGRect(x: 2, y: y + 2, width: EditorMetrics.gitStripeWidth, height: lineH - 4).fill()
            }
            if line.hasBreakpoint {
                drawBookmark(at: y, lineH: lineH)
            }

            let lnAttrs: [NSAttributedString.Key: Any] = [
                .font: font,
                .foregroundColor: line.isCursor
                    ? colors.accent.withAlphaComponent(0.92) : colors.gutter,
            ]
            let lnStr = "\(line.lineNo)" as NSString
            let lnSize = lnStr.size(withAttributes: lnAttrs)
            lnStr.draw(
                at: CGPoint(x: max(4, gutter - gap - lnSize.width), y: y + (lineH - fontSize) * 0.5 - 1),
                withAttributes: lnAttrs
            )

            cg.saveGState()
            cg.clip(to: CGRect(x: gutter, y: y, width: max(0, bounds.width - gutter), height: lineH))

            // Built before the selection fill: both the highlight and the caret
            // are positioned by measuring THIS line, not the core's cell grid.
            let textY = y + (lineH - fontSize) * 0.5 - 1
            let ct = ctLine(for: line, font: font)

            if line.hasSelection {
                // Same reason as the caret: measure against the drawn line so
                // the highlight tracks CJK glyphs instead of the cell grid.
                let x0 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(line.selU0), nil)
                let x1 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(line.selU1), nil)
                let w = max(cell, x1 - x0)
                colors.sel.setFill()
                CGRect(x: x0, y: y, width: w, height: lineH).fill()
            }

            cg.saveGState()
            cg.textMatrix = .identity
            cg.translateBy(x: gutter, y: textY + ascent)
            cg.scaleBy(x: 1, y: -1)
            CTLineDraw(ct, cg)
            cg.restoreGState()

            // Soft-wrap tail exists but can't stack in the absolute row model —
            // show a clipped-content marker at the right edge.
            if wrapLines, wrapped.contains(line.lineNo) {
                let marker = "⋯" as NSString
                let mAttrs: [NSAttributedString.Key: Any] = [
                    .font: font,
                    .foregroundColor: colors.dim.withAlphaComponent(0.85),
                ]
                let mSize = marker.size(withAttributes: mAttrs)
                marker.draw(
                    at: CGPoint(x: bounds.width - mSize.width - 6, y: textY),
                    withAttributes: mAttrs
                )
            }

            if line.isCursor {
                // Resolve the caret against the DRAWN line, not the core's
                // terminal cell grid: CJK counts as 2 cells there, but CoreText
                // lays glyphs out by their real advances, so `vcol * cell` sat
                // far right of the text on Hangul/Japanese lines (and dragged
                // the IME composition along with it).
                let cx = gutter + CTLineGetOffsetForStringIndex(
                    ct, CFIndex(line.caretUTF16), nil
                )
                var caretX = cx

                // Composing text (Hangul jamo mid-syllable, dead keys) is drawn
                // at the caret with the standard underline until the input
                // method commits it — the caret then sits AFTER it, the way
                // every native text view behaves.
                if !markedText.isEmpty {
                    let mAttrs: [NSAttributedString.Key: Any] = [
                        .font: font,
                        .foregroundColor: colors.fg,
                        .underlineStyle: NSUnderlineStyle.single.rawValue,
                    ]
                    let s = markedText as NSString
                    s.draw(at: CGPoint(x: cx, y: textY), withAttributes: mAttrs)
                    caretX = cx + s.size(withAttributes: mAttrs).width
                }

                // Span the band the LETTERS occupy — cap height down to the
                // descender — not the full em box. `ascender` carries accent
                // clearance well above the capitals (~3.4px at 14pt), so a
                // caret drawn to it visibly pokes over the text.
                let baseline = textY + font.ascender
                let capTop = baseline - font.capHeight
                let descBottom = baseline - font.descender
                let caretRect = CGRect(
                    x: caretX,
                    y: (capTop - 1).rounded(),
                    width: 2,
                    height: (descBottom - capTop + 2).rounded()
                )
                // Remember it for `firstRect(forCharacterRange:)` — the input
                // method places its candidate window against this.
                lastCaretRect = caretRect
                // …and hand it to SwiftUI (top-left origin) so caret-anchored
                // overlays like the completion popup can find it.
                if let win = window {
                    let inWindow = convert(caretRect, to: nil)
                    let h = win.contentView?.bounds.height ?? win.frame.height
                    engine?.caretFrameInWindow = CGRect(
                        x: inWindow.minX, y: h - inWindow.maxY,
                        width: inWindow.width, height: inWindow.height
                    )
                }
                colors.caret.setFill()
                caretRect.fill()
            }
            for sp in line.spans where sp.kind == 254 {
                // Matching bracket: flash, then fade out and stop drawing.
                let key = "\(line.lineNo):\(sp.start)"
                // Retrigger on the SAME pair once its previous flash is over —
                // keying on position alone meant a pair could only ever animate
                // once per session.
                let expired = CACurrentMediaTime() - bracketShownAt >= Self.bracketFlashDuration
                if key != bracketKey || expired {
                    bracketKey = key
                    bracketShownAt = CACurrentMediaTime()
                    startBracketFade()
                }
                let age = CACurrentMediaTime() - bracketShownAt
                guard age < Self.bracketFlashDuration else { continue }

                // Pop, settle, then fade — Xcode's hint springs out slightly
                // oversized before it calms down, which is what makes it read
                // as "here" rather than as a static swatch.
                let popT = min(1.0, age / Self.bracketPopDuration)
                let pop = sin(Double.pi * popT)          // 0 → 1 → 0 bump
                let scale = 1.0 + 0.22 * pop

                let raw = min(1.0, max(0.0,
                    (Self.bracketFlashDuration - age) / Self.bracketFadeTail))
                // Smoothstep — a linear ramp reads as a mechanical wipe.
                let fade = raw * raw * (3 - 2 * raw)

                let x0 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(sp.start), nil)
                let x1 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(sp.end), nil)
                let base = CGRect(
                    x: x0 - 1, y: y + 1,
                    width: max(cell, x1 - x0) + 2, height: lineH - 2
                )
                // Grow about the centre so the pop does not drift sideways.
                let box = base.insetBy(
                    dx: -base.width * (scale - 1) / 2,
                    dy: -base.height * (scale - 1) / 2
                )
                NSColor.systemYellow.withAlphaComponent(0.55 * fade).setFill()
                NSBezierPath(roundedRect: box, xRadius: 4, yRadius: 4).fill()

                // Redraw the delimiter itself bold and dark on top: the fill
                // alone leaves the glyph in its normal syntax colour, which is
                // hard to pick out against yellow.
                let ns = line.text as NSString
                let range = NSRange(location: Int(sp.start), length: Int(sp.end - sp.start))
                if range.location >= 0, range.location + range.length <= ns.length {
                    ns.substring(with: range).draw(
                        at: CGPoint(x: x0, y: textY),
                        withAttributes: [
                            .font: NSFont.monospacedSystemFont(ofSize: fontSize, weight: .bold),
                            .foregroundColor: NSColor.black.withAlphaComponent(fade),
                        ]
                    )
                }
            }
            for sp in line.spans {
                if sp.kind == 250 {
                    // Extra caret (GUI multi-cursor). sp.start is a UTF-16 offset,
                    // not a cell column — resolve it against the DRAWN line so it
                    // tracks CJK glyphs exactly like the primary caret, instead of
                    // sitting at vcol*cell far to the right on Hangul/CJK lines.
                    let cx = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(sp.start), nil)
                    // Same cap-height geometry as the primary caret above, so a
                    // secondary caret is visually indistinguishable from it.
                    let baseline = textY + font.ascender
                    let capTop = baseline - font.capHeight
                    let descBottom = baseline - font.descender
                    let caretRect = CGRect(
                        x: cx,
                        y: (capTop - 1).rounded(),
                        width: 2,
                        height: (descBottom - capTop + 2).rounded()
                    )
                    colors.caret.withAlphaComponent(0.85).setFill()
                    caretRect.fill()
                } else if sp.kind >= 251 && sp.kind <= 253 {
                    let x0 = gutter + CGFloat(sp.start) * cell
                    let w = max(cell, CGFloat(sp.end - sp.start) * cell)
                    let col: NSColor = sp.kind == 251 ? .systemRed : (sp.kind == 252 ? .systemOrange : .systemBlue)
                    col.withAlphaComponent(0.85).setStroke()
                    let path = NSBezierPath()
                    let yy = y + lineH - 2
                    path.move(to: CGPoint(x: x0, y: yy))
                    var x = x0
                    var up = true
                    while x < x0 + w {
                        path.line(to: CGPoint(x: x + 2, y: yy + (up ? -1.5 : 0)))
                        x += 2
                        up.toggle()
                    }
                    path.lineWidth = 1
                    path.stroke()
                }
            }
            cg.restoreGState()
        }

        if showFocusRing {
            colors.accent.withAlphaComponent(0.22).setStroke()
            cg.setLineWidth(1)
            cg.stroke(bounds.insetBy(dx: 0.5, dy: 0.5))
        }
    }

    /// Bookmark-style breakpoint marker in the gutter (SF Symbol, accent tint).
    private func drawBookmark(at y: CGFloat, lineH: CGFloat) {
        if bookmarkImage == nil {
            let cfg = NSImage.SymbolConfiguration(pointSize: 10, weight: .semibold)
            bookmarkImage = NSImage(
                systemSymbolName: "bookmark.fill",
                accessibilityDescription: "Breakpoint"
            )?
            .withSymbolConfiguration(cfg)
        }
        guard let img = bookmarkImage else { return }
        let size = img.size
        let rect = CGRect(
            x: 4,
            y: y + (lineH - size.height) * 0.5,
            width: size.width,
            height: size.height
        )
        // Tint via template drawing.
        NSGraphicsContext.current?.saveGraphicsState()
        colors.accent.set()
        img.isTemplate = true
        img.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 0.95)
        NSGraphicsContext.current?.restoreGraphicsState()
    }

    private func ctLine(for line: EditorLine, font: NSFont) -> CTLine {
        let key = cacheKey(for: line)
        if let cached = ctCache[key] { return cached }
        let attr = attributedLine(line, font: font)
        let ct = CTLineCreateWithAttributedString(attr)
        ctCache[key] = ct
        if ctCache.count > 800 {
            ctCache.removeAll(keepingCapacity: true)
            ctCache[key] = ct
        }
        return ct
    }

    private func attributedLine(_ line: EditorLine, font: NSFont) -> NSAttributedString {
        let raw = line.text.isEmpty ? " " : line.text
        let ns = raw as NSString
        let out = NSMutableAttributedString(
            string: raw,
            attributes: [.font: font, .foregroundColor: colors.fg]
        )
        let map = visualToUTF16Map(raw)
        for sp in line.spans where sp.kind < 250 {
            let v0 = Int(sp.start)
            let v1 = Int(sp.end)
            guard v1 > v0, v0 < map.count else { continue }
            let u0 = map[v0]
            let u1 = v1 < map.count ? map[v1] : ns.length
            guard u1 > u0 else { continue }
            let nsRange = NSRange(location: u0, length: min(u1, ns.length) - u0)
            guard nsRange.location + nsRange.length <= ns.length else { continue }
            out.addAttributes([.foregroundColor: colorForKind(sp.kind)], range: nsRange)
        }
        return out
    }

    private func visualToUTF16Map(_ s: String) -> [Int] {
        var map: [Int] = []
        let ns = s as NSString
        var i = 0
        var col = 0
        while i < ns.length {
            while map.count <= col { map.append(i) }
            let r = ns.rangeOfComposedCharacterSequence(at: i)
            let ch = ns.substring(with: r)
            let w: Int
            if ch == "\t" {
                w = 4 - (col % 4)
            } else if let scalar = ch.unicodeScalars.first, scalar.value > 0x2E80 {
                w = 2
            } else {
                w = 1
            }
            col += w
            i = r.location + r.length
        }
        while map.count <= col { map.append(i) }
        return map
    }

    private func colorForKind(_ kind: UInt8) -> NSColor {
        switch kind {
        case 1: return colors.keyword
        case 2: return colors.string
        case 3: return colors.comment
        case 4: return colors.number
        case 5: return colors.typeName
        case 6: return colors.function
        default: return colors.fg
        }
    }

    private func gitColor(_ sign: UInt8) -> NSColor {
        switch sign {
        case 1: return .systemGreen
        case 2: return .systemOrange
        case 3: return .systemRed
        default: return .clear
        }
    }

    // MARK: - Mouse

    override func scrollWheel(with event: NSEvent) {
        // ⌘ + wheel/pinch — zoom the editor text (Cmd+= / Cmd+- equivalents).
        if event.modifierFlags.contains(.command) {
            let dy = event.scrollingDeltaY
            if abs(dy) > 0.1 {
                engine?.zoomFont(delta: dy > 0 ? 1 : -1)
            }
            return
        }
        isLiveScrolling = true
        scrollView?.markUserScrollingFromWheel()
        wheelLiveEndWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            self?.isLiveScrolling = false
            self?.scrollView?.endWheelUserScrolling()
        }
        wheelLiveEndWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6, execute: work)
        super.scrollWheel(with: event)
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        guard let engine else { return }
        let p = convert(event.locationInWindow, from: nil)
        // Gutter click → toggle bookmark/breakpoint on that line.
        if p.x < EditorMetrics.gutter - EditorMetrics.gutterTextGap * 0.5 {
            let row = UInt32(max(0, Int(floor(p.y / max(1, EditorMetrics.lineHeight)))))
            engine.toggleBreakpointLine(row + 1)
            return
        }
        if engine.editorSplit.isSplit, paneIndex != engine.editorSplit.focus {
            engine.focusPane(paneIndex)
        }
        tracking = true
        isTrackingDrag = true
        dragMoved = false
        if let (row, u16) = absoluteHitUTF16(p) {
            engine.pointerDownUTF16(row: row, utf16: u16)
        } else {
            let (row, col) = absoluteHit(p)
            engine.pointerDownAbsolute(row: row, col: col)
        }
        // Insert mode is settled on mouse UP, once we know whether this was a
        // click or a drag. Scheduling it here raced the gesture: on a quick
        // drag the async block ran after the button was already released and
        // forced Insert, collapsing the selection that had just been made.
        trackDrag()
    }

    /// Cocoa's mouse-tracking loop, per Apple's Event Handling Guide.
    ///
    /// A drag-event-driven loop "is driven only as long as the user actually
    /// moves the mouse … it won't work to cause continual scrolling if the user
    /// presses the mouse button but never moves the mouse itself." That is
    /// exactly what made the earlier versions stutter and stall — the pointer
    /// usually ends up parked outside the window during a drag-scroll, where no
    /// further drag events arrive. The documented fix is PERIODIC events in the
    /// tracking mask, which is what this does.
    private func trackDrag() {
        guard let win = window else { return }
        NSEvent.startPeriodicEvents(afterDelay: 0.06, withPeriod: 1.0 / 45.0)
        engine?.projectIndex?.pause()
        defer {
            NSEvent.stopPeriodicEvents()
            engine?.projectIndex?.resume()
        }

        var tracking = true
        while tracking, let e = win.nextEvent(
            matching: [.leftMouseDragged, .leftMouseUp, .periodic],
            until: .distantFuture,
            inMode: .eventTracking,
            dequeue: true
        ) {
            switch e.type {
            case .leftMouseDragged:
                dragMoved = true
                extendSelection(to: convert(e.locationInWindow, from: nil))
                autoscrollStep(toward: convert(e.locationInWindow, from: nil))

            case .periodic:
                // Held still — usually outside the window. Synthesise an event
                // at the LIVE pointer so AppKit's own proportional autoscroll
                // does the scrolling; hand-rolled speed curves never matched it.
                let p = convert(win.mouseLocationOutsideOfEventStream, from: nil)
                autoscrollStep(toward: p)
                extendSelection(to: p)

            case .leftMouseUp:
                tracking = false
                finishDrag(with: e)

            default:
                break
            }
        }
        self.tracking = false
        isTrackingDrag = false
    }

    /// One frame of drag-scrolling.
    ///
    /// `autoscroll(with:)` scrolls PROPORTIONALLY to how far outside the view
    /// the pointer is — fine for easing a few points past the edge, but flick
    /// the mouse well below the window and each call jumps a long way, so at
    /// tick rate it staircases instead of gliding. This ramps with distance but
    /// caps the per-frame step, which is what keeps a fling smooth.
    private func autoscrollStep(toward point: CGPoint) {
        guard let clip = scrollView?.contentView else { return }
        let visible = visibleRect
        var overshoot: CGFloat = 0
        if point.y < visible.minY { overshoot = point.y - visible.minY }
        else if point.y > visible.maxY { overshoot = point.y - visible.maxY }
        guard overshoot != 0 else { return }

        let lineH = EditorMetrics.lineHeight
        // Ramp over the first ~6 lines of overshoot, then hold at the cap.
        let ramp = min(abs(overshoot) / (lineH * 6), 1)
        let eased = ramp * ramp * (3 - 2 * ramp)          // smoothstep
        let step = (lineH * 0.35 + lineH * 2.0 * eased) * (overshoot < 0 ? -1 : 1)

        let maxY = max(0, frame.height - visible.height)
        let newY = min(max(0, visible.origin.y + step), maxY)
        guard abs(newY - visible.origin.y) > 0.01 else { return }
        clip.scroll(to: CGPoint(x: visible.origin.x, y: newY))
        scrollView?.reflectScrolledClipView(clip)
    }

    private func extendSelection(to point: CGPoint) {
        guard let engine else { return }
        // Clamp into the visible band so the hit test resolves against a row
        // that actually exists after autoscrolling.
        let v = visibleRect
        let clamped = CGPoint(x: point.x, y: min(max(point.y, v.minY + 1), v.maxY - 1))
        if let (row, u16) = absoluteHitUTF16(clamped) {
            engine.pointerDragUTF16(row: row, utf16: u16)
        } else {
            let (row, col) = absoluteHit(clamped)
            engine.pointerDragAbsolute(row: row, col: col)
        }
        // Repaint straight from the engine — no SwiftUI round trip.
        noteContentChanged()
    }

    private func finishDrag(with event: NSEvent) {
        guard let engine else { return }
        if event.clickCount >= 2 {
            let p = convert(event.locationInWindow, from: nil)
            let (row, col) = absoluteHit(p)
            engine.pointerDoubleAbsolute(row: row, col: col)
        }
        engine.pointerUp()
        engine.refreshChrome()
        // A plain click places the caret and resumes typing; a drag leaves its
        // selection alone (typing over it replaces it).
        if !dragMoved, event.clickCount == 1 {
            engine.ensureEditorFocus()
        }
        dragMoved = false
    }


    private func startBracketFade() {
        bracketFadeTimer?.invalidate()
        let started = bracketShownAt
        let t = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] timer in
            guard let self else { timer.invalidate(); return }
            self.needsDisplay = true
            if CACurrentMediaTime() - started >= Self.bracketFlashDuration {
                timer.invalidate()
                self.bracketFadeTimer = nil
                // One last pass with the hint expired, so it clears.
                self.needsDisplay = true
            }
        }
        RunLoop.main.add(t, forMode: .common)
        bracketFadeTimer = t
    }





    // MARK: - Context menu (right-click — standard GUI editing)

    override func menu(for event: NSEvent) -> NSMenu? {
        guard let engine else { return nil }
        // Was a vim-Visual probe, which the GUI never entered — Cut/Copy were
        // therefore always disabled. The painted band knows the truth.
        let selectionActive = bandRows.contains { $0.hasSelection }
        // Click outside a selection moves the caret there first (Xcode behavior).
        if !selectionActive {
            let p = convert(event.locationInWindow, from: nil)
            let (row, col) = absoluteHit(p)
            engine.placeCaret(row: row, col: col)
        }

        let menu = NSMenu()
        func item(_ title: String, _ action: Selector, key: String = "") -> NSMenuItem {
            let it = NSMenuItem(title: title, action: action, keyEquivalent: key)
            it.target = self
            return it
        }
        let cut = item("Cut", #selector(ctxCut(_:)), key: "x")
        let copy = item("Copy", #selector(ctxCopy(_:)), key: "c")
        cut.isEnabled = selectionActive
        copy.isEnabled = selectionActive
        menu.autoenablesItems = false
        menu.addItem(cut)
        menu.addItem(copy)
        menu.addItem(item("Paste", #selector(ctxPaste(_:)), key: "v"))
        menu.addItem(item("Select All", #selector(ctxSelectAll(_:)), key: "a"))
        menu.addItem(.separator())
        menu.addItem(item("Toggle Bookmark", #selector(ctxToggleBreakpoint(_:))))
        let file = engine.chrome.filename
        if file.hasPrefix("/") {
            menu.addItem(.separator())
            menu.addItem(item("Reveal in Finder", #selector(ctxReveal(_:))))
        }
        return menu
    }

    @objc private func ctxCut(_ sender: Any?) {
        engine?.dispatch(code: .char_, ch: UInt32(UnicodeScalar("x").value), mods: .superKey)
    }

    @objc private func ctxCopy(_ sender: Any?) {
        engine?.dispatch(code: .char_, ch: UInt32(UnicodeScalar("c").value), mods: .superKey)
    }

    @objc private func ctxPaste(_ sender: Any?) {
        engine?.dispatch(code: .char_, ch: UInt32(UnicodeScalar("v").value), mods: .superKey)
    }

    @objc private func ctxSelectAll(_ sender: Any?) {
        engine?.selectAll()
    }

    @objc private func ctxToggleBreakpoint(_ sender: Any?) {
        engine?.toggleBreakpointAtCursor()
    }

    @objc private func ctxReveal(_ sender: Any?) {
        guard let file = engine?.chrome.filename, file.hasPrefix("/") else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: file)])
    }

    private func absoluteHit(_ docPoint: CGPoint) -> (UInt32, UInt32) {
        let lineH = max(1, EditorMetrics.lineHeight)
        let cell = max(1, EditorMetrics.cellWidth)
        let gutter = EditorMetrics.gutter
        let row = UInt32(max(0, floor(docPoint.y / lineH)))
        let col = UInt32(max(0, floor((docPoint.x - gutter) / cell)))
        return (row, col)
    }

    /// UTF-16 offset under the pointer, measured against the DRAWN line.
    ///
    /// The cell-grid version floored `(x - gutter) / cellWidth`, so clicking the
    /// right half of a glyph still landed the caret before it, and CJK — two
    /// cells wide in the core, one glyph on screen — drifted badly.
    /// `CTLineGetStringIndexForPosition` snaps to the nearest boundary and uses
    /// the real advances.
    private func absoluteHitUTF16(_ docPoint: CGPoint) -> (UInt32, UInt32)? {
        let lineH = max(1, EditorMetrics.lineHeight)
        let row = Int(max(0, floor(docPoint.y / lineH)))
        let band = rows(row, row)
        guard let line = band.first(where: { Int($0.lineNo) - 1 == row && !$0.isWrapContinuation })
        else { return nil }
        let font = NSFont.monospacedSystemFont(ofSize: EditorMetrics.fontSize, weight: .regular)
        let ct = ctLine(for: line, font: font)
        let x = docPoint.x - EditorMetrics.gutter
        let idx = CTLineGetStringIndexForPosition(ct, CGPoint(x: max(0, x), y: 0))
        guard idx != kCFNotFound else { return nil }
        return (UInt32(row), UInt32(max(0, idx)))
    }
}

extension EditorScrollView {
    fileprivate func markUserScrollingFromWheel() {
        if !isUserScrolling { engine?.beginLiveScroll() }
        isUserScrolling = true
    }

    fileprivate func endWheelUserScrolling() {
        guard isUserScrolling else { return }
        engine?.endLiveScroll()
        isUserScrolling = false
        syncCorePositionPublic()
    }

    fileprivate func syncCorePositionPublic() {
        // Re-sync once the wheel settles (throttle-free).
        let lineH = max(1, EditorMetrics.lineHeight)
        let cell = max(1, EditorMetrics.cellWidth)
        let v0 = max(0, Int(floor(documentVisibleRect.minY / lineH)))
        let hCols = canvas.wrapLines
            ? 0
            : max(0, Int(floor(documentVisibleRect.minX / cell)))
        if engine?.editorSplit.isSplit != true || paneIndex == engine?.editorSplit.focus {
            engine?.scrollSync(line: UInt32(v0), hscroll: UInt32(hCols))
        }
    }
}

// MARK: - NSTextInputClient
//
// The macOS text input contract (NSTextInputClient.h, @required). Adopting it
// is the ONLY way to get input-method text — Hangul, Japanese, dead keys,
// option-accents, the emoji picker — into the editor. It also makes AppKit
// translate key events into standard editing selectors via
// `NSStandardKeyBindingResponding`, honouring the user's own
// ~/Library/KeyBindings overrides.
extension EditorCanvasView: NSTextInputClient {
    private func asString(_ any: Any) -> String {
        if let s = any as? String { return s }
        if let a = any as? NSAttributedString { return a.string }
        return ""
    }

    func insertText(_ string: Any, replacementRange: NSRange) {
        inputHandledKey = true
        commitText(asString(string))
    }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        inputHandledKey = true
        markedText = asString(string)
        needsDisplay = true
    }

    func unmarkText() {
        markedText = ""
        needsDisplay = true
    }

    func hasMarkedText() -> Bool { !markedText.isEmpty }

    /// Synthetic ranges: the document's UTF-16 offsets are not exposed by the
    /// core yet. The input method only needs these to stay self-consistent
    /// while composing, which they do.
    func markedRange() -> NSRange {
        markedText.isEmpty
            ? NSRange(location: NSNotFound, length: 0)
            : NSRange(location: 0, length: markedText.utf16.count)
    }

    func selectedRange() -> NSRange {
        NSRange(location: markedText.utf16.count, length: 0)
    }

    func attributedSubstring(
        forProposedRange range: NSRange, actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        nil
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }

    /// Where the candidate window goes — the caret, in screen coordinates.
    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        let inWindow = convert(lastCaretRect, to: nil)
        return window?.convertToScreen(inWindow) ?? inWindow
    }

    func characterIndex(for point: NSPoint) -> Int { NSNotFound }

    /// Editing intent, already resolved by AppKit from the key event. Selectors
    /// we have not mapped to a semantic core command yet fall through to the
    /// existing NSEvent path, so nothing regresses during the migration.
    override func doCommand(by selector: Selector) {
        // The input method already turned this key into text or composition;
        // replaying the raw event would apply it a second time.
        if inputHandledKey { return }
        fallbackToLegacyKeyPath()
    }
}
