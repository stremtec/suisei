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
    var wrapLines: Bool
    /// Per-keystroke caret/scroll comes through THIS, observed independently of
    /// the enclosing view tree — so a keystroke updates only this pane's clip
    /// (`updateNSView`), never the split container or tab strip. hScroll,
    /// docScroll, docLineCount, contentGen and scrollIntent all read from it.
    @ObservedObject var editorTick: EditorTickStore
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
    /// Points along this pane's right edge that something is drawn over —
    /// today, the minimap. The strip is a SwiftUI overlay, so AppKit's idea of
    /// what is visible includes the band underneath it: `scrollToVisible`
    /// happily declared a caret "shown" while the minimap was covering it.
    var rightInset: CGFloat = 0
    /// Gutter counts from the caret rather than from 1.
    var relativeNumber: Bool = false

    /// Last resolved palette, keyed by the SwiftUI colours it came from.
    ///
    /// `NSColor(Color)` measures 0.37–0.65 µs and this builds 21 of them. Every
    /// pane's `EditorHost` observes the SHARED `EditorTickStore`, so one tick
    /// updates all of them: 4 panes × 21 = 84 conversions, ~30 µs, twenty times
    /// a second — to re-derive a palette that only changes when the theme does.
    ///
    /// One slot is enough: every pane in a window resolves the same palette, so
    /// the panes after the first are all hits.
    private struct PaletteKey: Equatable {
        var editorBg: Color, fg: Color, dim: Color, accent: Color, selBg: Color
        var caretColor: Color, gutterFg: Color, cursorLineBg: Color
        var theme: ThemeSnap
    }
    nonisolated(unsafe) private static var paletteKey: PaletteKey?
    nonisolated(unsafe) private static var paletteValue: EditorCanvasView.Colors?

    @MainActor
    private static func resolvedColors(
        editorBg: Color, fg: Color, dim: Color, accent: Color, selBg: Color,
        caretColor: Color, gutterFg: Color, cursorLineBg: Color, theme: ThemeSnap
    ) -> EditorCanvasView.Colors {
        let key = PaletteKey(
            editorBg: editorBg, fg: fg, dim: dim, accent: accent, selBg: selBg,
            caretColor: caretColor, gutterFg: gutterFg, cursorLineBg: cursorLineBg,
            theme: theme
        )
        if key == paletteKey, let cached = paletteValue { return cached }
        let colors = EditorCanvasView.Colors(
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
            function: NSColor(theme.color(theme.function)),
            macroName: NSColor(theme.color(theme.macroName)),
            namespace: NSColor(theme.color(theme.namespace)),
            parameter: NSColor(theme.color(theme.parameter)),
            property: NSColor(theme.color(theme.property)),
            constant: NSColor(theme.color(theme.constant)),
            operatorColor: NSColor(theme.color(theme.operatorColor)),
            punctuation: NSColor(theme.color(theme.punctuation)),
            gitChange: .systemBlue,
            gitDelete: .systemRed,
            removedBg: NSColor.systemYellow.withAlphaComponent(0.10),
            removedFg: NSColor(fg).withAlphaComponent(0.82),
            removedEdge: NSColor.systemYellow.withAlphaComponent(0.28),
            breakpoint: .systemYellow,
            breakpointInk: .black,
            debugStop: NSColor(theme.color(theme.debugStop)),
            debugStopInk: .systemGreen,
            liveFlash: NSColor(accent).withAlphaComponent(0.22),
            bracketFill: EditorCanvasView.bracketYellow,
            bracketInk: .black
        )
        paletteKey = key
        paletteValue = colors
        return colors
    }

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

    /// What the selected completion would add, for the inline grey preview.
    ///
    /// The remainder only — the popup's label minus what has already been
    /// typed — so the preview reads as a continuation of the caret rather than
    /// a duplicate of the prefix. Matched case-insensitively because the list
    /// is: typing `def` offers `Definition`, and the ghost has to be `inition`,
    /// not nothing.
    ///
    /// Focused pane only. Under a split every pane draws its own caret, and a
    /// ghost on the others would be a suggestion for a caret that is not there.
    static func ghostSuffix(_ engine: EngineBridge, paneIndex: Int) -> String {
        let split = engine.editorSplit
        guard !split.isSplit || paneIndex == split.focus else { return "" }
        let c = engine.chrome.completions
        guard c.open, c.selected >= 0, c.selected < c.items.count else { return "" }
        let label = c.items[c.selected].label
        guard label.count > c.prefix.count,
              label.lowercased().hasPrefix(c.prefix.lowercased()) else { return "" }
        return String(label.dropFirst(c.prefix.count))
    }

    func updateNSView(_ view: EditorScrollView, context: Context) {
        view.engine = engine
        view.paneIndex = paneIndex
        let colors = Self.resolvedColors(
            editorBg: editorBg, fg: fg, dim: dim, accent: accent, selBg: selBg,
            caretColor: caretColor, gutterFg: gutterFg, cursorLineBg: cursorLineBg,
            theme: theme
        )
        view.canvas.ghostSuffix = Self.ghostSuffix(engine, paneIndex: paneIndex)
        view.rightInset = rightInset
        let tick = editorTick.tick(for: paneIndex)
        view.apply(
            hScroll: tick.hscroll,
            wrapLines: wrapLines,
            relativeNumber: relativeNumber,
            docScroll: tick.scroll,
            docLineCount: tick.docLineCount,
            contentGen: editorTick.gen,
            scrollIntent: editorTick.scrollIntent,
            showFocusRing: showFocusRing,
            colors: colors
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
    /// The line height this clip was last positioned at.
    ///
    /// ⌘+ / ⌘− changes it, and a clip's origin is in POINTS. Nothing else in
    /// `apply` notices: the document's line count has not changed and no scroll
    /// intent is set, so the point offset survives a change of the very unit it
    /// was measured in and lands on a different line — further down on ⌘−,
    /// where the lines just got shorter.
    ///
    /// It is also why the minimap "did not show" the jump: Core's `scroll` is a
    /// LINE and it did not move, so the indicator was right and the viewer was
    /// wrong. Which in turn is why jumping from the outline afterwards
    /// misbehaved — it computes against a base the view no longer agrees with.
    private var lastLineHeight: CGFloat = 0
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

    /// Points of this pane's right edge covered by an overlay (the minimap).
    var rightInset: CGFloat = 0

    /// How many columns a wrapped row may use in THIS pane.
    ///
    /// The pane's own width, minus the gutter, minus whatever is drawn over the
    /// right edge, minus a column of breathing room so a full row does not sit
    /// flush against the minimap. Zero when wrapping is off, which every layer
    /// below reads as "one row per line".
    ///
    /// Core used to guess this from `app.grid_cols()` — the whole editor's
    /// columns — so a wrapped line in a split pane broke past its own edge.
    private func wrapColumns(wrapLines: Bool) -> Int {
        guard wrapLines else { return 0 }
        // `textAdvance`, not `cellWidth`: the latter is the gutter's layout
        // quantum, measured at `.medium` and rounded UP to a whole point. The
        // text draws at `.regular` with real advances, and dividing a pane by
        // the rounded-up number rounds the column count DOWN — five columns of
        // a 46-column row at size 12, left empty at the right edge.
        let advance = max(1, EditorMetrics.textAdvance)
        // `rightInset` already holds the row off whatever covers the edge, so
        // there is no second column of margin to take here.
        let usable = contentView.bounds.width
            - EditorMetrics.gutter
            - rightInset
        // A pane too narrow to hold anything still has to wrap somewhere, or
        // `WrapMap` divides by a width of zero rows.
        return max(8, Int(floor(usable / advance)))
    }

    /// Wrap width as of the last sync — the canvas's copy is the same number,
    /// kept here so `fitCanvasToBounds` can ask whether it is wrapping without
    /// reaching through.
    private var lastWrapCols: Int = 0

    /// Document width in columns as of the last `apply`.
    private var lastContentCols: Int = 0

    /// Re-fit the canvas whenever OUR frame changes.
    ///
    /// The canvas used to be sized only inside `apply`, which runs when SwiftUI
    /// re-renders the representable — and a panel slide changes this view's
    /// frame through AppKit layout, not through the representable's inputs. So
    /// the canvas kept its old width for the whole animation, and everything
    /// drawn to `bounds.width` (the cursor-line highlight above all) stopped
    /// short of the new edge until some later event happened to re-apply.
    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        // The wrap width is a function of this frame, so it has to be
        // recomputed HERE and not only in `apply`. A panel slide changes the
        // frame through AppKit layout; `apply` runs on a SwiftUI publish, which
        // arrives when the animation ends. So the text went on wrapping at the
        // old width for the whole slide and snapped at the end of it.
        syncWrapColumns()
        fitCanvasToBounds()
    }

    /// Recompute the wrap width from the pane as it is now, and tell the canvas
    /// if it moved. Cheap: a divide and a compare unless it changed.
    private func syncWrapColumns() {
        let cols = wrapColumns(wrapLines: lastWrap)
        guard cols != canvas.wrapCols else { return }
        EditorDiagnostics.reportWrap(
            pane: paneIndex,
            clipWidth: contentView.bounds.width,
            gutter: EditorMetrics.gutter,
            rightInset: rightInset,
            advance: EditorMetrics.textAdvance,
            wideRatio: EditorMetrics.wideGlyphRatio,
            cols: cols
        )
        canvas.setWrapCols(cols)
        lastWrapCols = cols
        // Different chunking: the band in hand is the old shape.
        canvas.noteContentChanged()
    }

    /// Re-size the document after the canvas changed how many rows it draws.
    func refitCanvas() { fitCanvasToBounds() }

    /// Where a buffer row is drawn, from the canvas that decides it.
    ///
    /// Forwarded rather than reimplemented: scrolling to a line has to land on
    /// the same y the line is painted at, and two copies of that arithmetic is
    /// how every other pair in this codebase came to disagree.
    private func visualY(_ bufferRow: Int) -> CGFloat { canvas.visualY(bufferRow) }

    /// Size the document view to the greater of the visible area and the
    /// content. Face-only — no engine round trip, so it can run per frame.
    private func fitCanvasToBounds() {
        let lineH = EditorMetrics.lineHeight
        let cell = EditorMetrics.cellWidth
        // Plus any rows an expanded change has put on screen: they take height
        // like every other row, and a document sized without them cannot be
        // scrolled to its own end.
        // Visual rows, not buffer lines: a wrapped document is taller than its
        // line count and could not otherwise be scrolled to its own end.
        let count = canvas.totalVisualRows() + canvas.extraVisualRows
        let docH = max(bounds.height, CGFloat(count) * lineH + 8)
        // The engine owns the extent. The old budget was
        // `max(400, hScroll + 160)` — a width that GREW WITH THE SCROLL
        // POSITION, so every pan to the right made the document wider and the
        // pan could never reach an end.
        //
        // Conditional on wrapping again, now that wrapping wraps. It was made
        // unconditional while the renderer still discarded continuation rows,
        // because a document sized to the viewport then had nowhere to scroll
        // and the text past the right edge was neither folded nor reachable.
        // Folded text has nothing to pan to, so a canvas wider than the pane
        // would be empty space with a scroller attached to it.
        var docW = max(bounds.width, 1)
        if lastWrapCols == 0 {
            docW = max(docW, CGFloat(lastContentCols) * cell + EditorMetrics.gutter + 32)
        }
        let newSize = NSSize(width: docW, height: docH)
        guard abs(canvas.frame.width - newSize.width) > 0.5
            || abs(canvas.frame.height - newSize.height) > 0.5
        else { return }
        // Resizing the canvas clamps the clip origin, which fires
        // `clipBoundsChanged` and pushes that clamped position back into Core —
        // wiping the scroll it had just restored for this tab. The push has to
        // stay muted across the resize.
        suppressPush = true
        canvas.setFrameSize(newSize)
        suppressPush = false
        canvas.needsDisplay = true
    }


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
        // A minimap names the pane it belongs to, because with a strip in every
        // pane the sender is no longer necessarily the focused one. Everything
        // else — outline, goto, a search hit — names no pane and means the
        // pane the user is in.
        if let target = note.userInfo?["pane"] as? Int, target >= 0 {
            guard target == paneIndex else { return }
        } else if let engine, engine.editorSplit.isSplit,
                  paneIndex != engine.editorSplit.focus {
            return
        }
        scrollToLineAnimated(line, center: true)
    }

    func apply(
        hScroll: UInt32,
        wrapLines: Bool,
        relativeNumber: Bool,
        docScroll: UInt32,
        docLineCount: UInt32,
        contentGen: UInt64,
        scrollIntent: UInt8,
        showFocusRing: Bool,
        colors: EditorCanvasView.Colors
    ) {
        let _applyT0 = DispatchTime.now().uptimeNanoseconds
        defer {
            PerfProbe.record(
                "apply(pane) updateNSView",
                Double(DispatchTime.now().uptimeNanoseconds - _applyT0) / 1_000_000
            )
        }
        let lineH = EditorMetrics.lineHeight

        // Keep the AppKit first responder on the ENGINE-focused pane. Focus can
        // move by keyboard (⌃W) or a SwiftUI overlay tap — paths that call
        // `focusPane` but never reassign the responder — which left IME marked
        // text landing in a stale pane's canvas (usually pane 0, the leftmost:
        // the composing글자 flickered there instead of where you were typing).
        // Reclaim ONLY from another editor canvas, never from a text field
        // (the find bar / project filter), and never when already correct.
        if let win = window, let engine,
           engine.editorSplit.isSplit,
           paneIndex == engine.editorSplit.focus,
           win.firstResponder is EditorCanvasView,
           win.firstResponder !== canvas {
            win.makeFirstResponder(canvas)
        }

        // ONE number, before anything reads it. The band is chunked at this
        // width, the wrap map is built at this width, and the canvas is sized
        // from that map — a row's contents and the count of rows computed at
        // two different widths would put every line below the first wrap
        // somewhere it is not drawn. Assigned ahead of `fitCanvasToBounds`
        // below, which asks the map how tall the document is.
        // Captured before `lastWrap` moves: `docChanged` below compares against
        // it, and assigning first made "wrap was just toggled" invisible — the
        // one moment every row's position changes.
        let wrapChanged = wrapLines != lastWrap
        lastWrap = wrapLines
        syncWrapColumns()

        // A wrapped document has nothing to pan to, but an unwrapped one does
        // and the scroller is how you know. Both were switched off with wrap
        // for years while nothing wrapped, which is how the setting came to
        // hide text instead of folding it.
        hasHorizontalScroller = !wrapLines
        horizontalScrollElasticity = wrapLines ? .none : .allowed

        // A zoom moves every row without changing a single one of them, so it
        // has to be re-anchored the same way a document change is: put Core's
        // line back under the top of the clip.
        let metricsChanged = lastLineHeight != 0 && lineH != lastLineHeight
        lastLineHeight = lineH
        let docChanged = docLineCount != lastDocLineCount || wrapChanged || metricsChanged
        lastDocLineCount = docLineCount
        // Only the focused pane may ask, because `contentCols()` can only
        // answer for one document: it measures `App::buffer` — the LIVE one —
        // over `App::scroll`, and `restore_state_from_tab` resets its
        // high-water mark on every tab switch precisely because a different
        // document has a different extent.
        //
        // Every pane was asking, so every pane sized ITS canvas to the FOCUSED
        // pane's document. Moving focus between a wide file and a narrow one
        // resized both canvases, and resizing a canvas clamps its clip origin:
        // the horizontal position of both panes stepped sideways on every
        // focus change, and a narrow file grew a horizontal scrollbar it had no
        // use for. An unfocused pane keeps the extent it measured for its own
        // document, which is the only extent it was ever entitled to.
        //
        // Same guard as `revealCaret`, `syncCorePosition` and
        // `postLiveMinimapLine` below, for the same reason.
        if engine?.editorSplit.isSplit != true || paneIndex == engine?.editorSplit.focus {
            lastContentCols = Int(engine?.contentCols() ?? 0)
        }
        fitCanvasToBounds()

        canvas.setChrome(
            showFocusRing: showFocusRing,
            colors: colors,
            wrapLines: wrapLines,
            relativeNumber: relativeNumber,
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
        //
        // …and it is the FOCUSED pane that obeys. `scrollIntent` is one field on
        // `App`: it describes what core just did to the document it is holding,
        // which is the focused pane's. Every pane read it, so focusing pane B
        // told pane A to restore a position it had not left.
        //
        // That is visible exactly once, and only from the bottom of a document.
        // A clip resting at the end sits at `docHeight - viewportHeight`, which
        // is not a whole number of lines; core stores the scroll as a line
        // index, `floor`ed. Restoring it multiplies that integer back out and
        // the view rises by the lost remainder — after which the clip IS on a
        // line boundary, `coreLine == clipLine`, and nothing moves again. "튄
        // 이후로 안 건들면 안 움직임."
        //
        // A pane that is not focused and whose document did change still lands
        // correctly: it falls to the `docChanged` branch below, which is what
        // that branch is for.
        let focusedPane = engine?.editorSplit.isSplit != true
            || paneIndex == engine?.editorSplit.focus
        let coreLine = Int(docScroll)
        let clipLine = Int(floor(documentVisibleRect.minY / max(1, lineH)))
        if focusedPane, !isUserScrolling, !canvas.isLiveScrolling, !canvas.isTrackingDrag,
           scrollIntent != 0
        {
            switch scrollIntent {
            case 2:  // navigate — outline, goto, search hit
                if coreLine != clipLine { scrollToLineAnimated(coreLine, center: false) }
            case 3:  // caret — reveal it with the MINIMUM scroll, judged against
                     // the pane's real clip bounds. Core only guarantees the
                     // caret line is IN the snapshot (rough follow + overscan);
                     // the exact placement is done here so it is right in any
                     // split pane size, on both axes, glyph-accurate and
                     // immediate — no cell-grid estimate, no hscroll round trip.
                revealCaret()
            default: // restore — place the saved tab position exactly, at once.
                if coreLine != clipLine {
                    setClipTo(line: coreLine, hCols: Int(hScroll))
                }
            }
            engine?.clearScrollIntent()
        } else if docChanged {
            setClipTo(line: coreLine, hCols: Int(hScroll))
        }
    }

    private func setClipTo(line: Int, hCols: Int) {
        let lineH = EditorMetrics.lineHeight
        let cell = EditorMetrics.cellWidth
        suppressPush = true
        let wantY = visualY(line)
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

    /// Scroll the minimum amount to reveal the caret, measured against the
    /// pane's ACTUAL clip bounds — the native `scrollToVisible` contract. This
    /// replaced a core-estimated cell-grid follow that broke on unequal split
    /// panes (the estimate never matched the real pane) and lagged horizontally
    /// (every column change round-tripped through core's `hscroll`).
    ///
    /// `y` is exact from the caret row. `x` is the glyph-accurate caret x from
    /// the last paint (≤ one frame stale on the same line — the padding covers
    /// it); on a fresh row a cell estimate is enough to bring the row on screen,
    /// and the next paint refines it. Focused pane only — the caret is its own.
    func revealCaret() {
        if let engine, engine.editorSplit.isSplit, paneIndex != engine.editorSplit.focus {
            return
        }
        guard let engine else { return }
        let lineH = EditorMetrics.lineHeight
        let cell = max(1, EditorMetrics.cellWidth)
        // Pulled, not read off `chrome`: the typing fast path publishes no
        // chrome, so the snapshot's caret is from before this run of
        // keystrokes. Scrolling to it put the view where the caret used to be.
        let (row, vcol) = engine.caretRowVCol()
        // The caret's own segment, not the line's first row: on a wrapped line
        // those are different rows, and revealing the top of a line whose
        // caret is four rows down scrolls to the wrong place.
        let y = visualY(row) + CGFloat(canvas.caretSegment(row: row)) * lineH
        // Real-time glyph x against the same CTLine the draw uses. Falls back
        // to a cell estimate only if the caret line can't be fetched.
        let x: CGFloat = canvas.liveCaretGlyphX(row: row)
            ?? (EditorMetrics.gutter + CGFloat(vcol) * cell)
        // What has to be on screen, not just the caret.
        //
        // DOWN a whole line past the caret's own. Typing `{` gives you `{}`
        // with the caret between them, and Enter turns that into three lines
        // with the caret on the middle one — so revealing only the caret line
        // leaves the closing brace off the bottom, which is the one thing you
        // wanted to see move. One line of lead is also what an editor wants in
        // general: the next line is where you are about to be.
        //
        // RIGHT past the minimap, plus four columns. The strip is an overlay,
        // so the scroll view counts the band under it as visible and would
        // stop scrolling with the caret hidden behind the thumbnail.
        //
        // LEFT to the very edge once the caret is near the start of a line.
        // Press Enter at the end of a long line that had you scrolled right and
        // the caret lands in column 0 — but two columns of lead is satisfied by
        // a clip still parked mid-gutter, so the view stopped with the line
        // numbers half cut off. Within a few columns of the text's start there
        // is nothing to the left worth keeping off screen, so ask for x = 0 and
        // get the whole gutter back.
        //
        // Otherwise two columns of lead: behind the caret there is nothing you
        // are about to need, only what you can already read.
        let nearLineStart = x <= EditorMetrics.gutter + cell * 4
        let left = nearLineStart ? 0 : max(0, x - cell * 2)
        let rect = CGRect(
            x: left,
            y: y - lineH * 0.5,
            width: (x - left) + rightInset + cell * 4,
            height: lineH * 2.5
        )
        canvas.scrollToVisible(rect)
        // Core paints the visible-line window from `app.scroll`; tell it where
        // the clip actually landed so the snapshot keeps containing the caret
        // line and the next keystroke follows from the right base.
        syncCorePosition(force: true)
    }

    /// Smooth ease-in-out glide to a buffer line (outline / minimap / goto).
    func scrollToLineAnimated(_ line: Int, center: Bool) {
        let lineH = EditorMetrics.lineHeight
        let visRows = Int(contentView.bounds.height / max(1, lineH))
        let target = center ? max(0, line - visRows / 2) : line
        let maxY = max(0, canvas.frame.height - contentView.bounds.height)
        let wantY = min(max(0, visualY(target)), maxY)
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
        // CoreText participates in AppKit responsive scrolling and must not be
        // force-invalidated for every fractional clip movement. Metal does not,
        // so only its explicit opt-in path needs a viewport submission here.
        if RendererChoice.useMetal {
            canvas.viewportDidScroll()
        }
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
        let hCols = max(0, Int(floor(documentVisibleRect.minX / cell)))
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

/// LRU with batched eviction. Replaces the old all-or-nothing caches: those
/// dropped EVERY entry once a cap was crossed, so scrolling a long file past
/// the cap re-shaped every visible line on each pass — measured thrash. The
/// batched eviction drops the oldest eighth at a time, amortized O(1) per
/// insert, and the working set survives the cap.
final class LRUCache<Key: Hashable, Value> {
    private let capacity: Int
    private var entries: [Key: (value: Value, stamp: UInt64)] = [:]
    private var clock: UInt64 = 0

    var count: Int { entries.count }

    init(capacity: Int) {
        self.capacity = capacity
    }

    subscript(key: Key) -> Value? {
        get {
            guard let e = entries[key] else { return nil }
            clock &+= 1
            entries[key] = (e.value, clock)
            return e.value
        }
        set {
            clock &+= 1
            if let v = newValue {
                entries[key] = (v, clock)
                if entries.count > capacity { evictOldest() }
            } else {
                entries.removeValue(forKey: key)
            }
        }
    }

    func removeAll(keepingCapacity: Bool = true) {
        entries.removeAll(keepingCapacity: keepingCapacity)
    }

    private func evictOldest() {
        let keep = capacity - max(1, capacity / 8)
        guard entries.count > keep else { return }
        let oldest = entries
            .sorted { $0.value.stamp < $1.value.stamp }
            .prefix(entries.count - keep)
            .map { $0.key }
        for key in oldest {
            entries.removeValue(forKey: key)
        }
    }
}

/// The squiggle drawn under a find match, in place of a box behind it.
///
/// A wash behind the glyphs competes with the syntax colours it covers and
/// hides the selection when the two overlap. An underline marks the same
/// span while leaving the text exactly as it was — which is what a spell
/// checker has always done, and for the same reason.
///
/// Defined once as a function of x so the two renderers cannot disagree:
/// CoreText strokes it as a path, Metal — which has only quads — samples
/// the same curve into 1pt columns.
enum FindSquiggle {
    /// Points per full wave. Short enough to read as a squiggle on a
    /// three-character match, long enough not to alias into a blur.
    static let period: CGFloat = 4.5
    static let amplitude: CGFloat = 0.9

    static func offset(at x: CGFloat) -> CGFloat {
        sin(x / period * 2 * .pi) * amplitude
    }

    /// Distance from the line box's TOP to the wave's centreline. The view
    /// is flipped, so this grows downward.
    ///
    /// Sits at the very bottom of the line box and overhangs it slightly. Any
    /// higher and the crests reach into the descenders — the wave was clipping
    /// the tails of g, j, p, q and y, which is the one thing an underline is
    /// supposed not to do. The overhang lands in the leading above the next
    /// row, where there are no glyphs.
    static func centreY(lineTop y: CGFloat, lineHeight: CGFloat) -> CGFloat {
        y + lineHeight - 1.2
    }

    static func thickness(current: Bool) -> CGFloat { current ? 1.8 : 1.3 }

    /// The current match is the one ⌘G is on; the rest are context.
    ///
    /// Fixed sRGB rather than `systemYellow` for the reason the bracket flash
    /// is — that one is dynamic and AppKit resolves it to a different hue per
    /// appearance, so a mark that means the same thing in both themes would
    /// not have been the same colour in both.
    static let ink = NSColor(srgbRed: 1.0, green: 0.80, blue: 0.20, alpha: 1)

    static func color(current: Bool) -> NSColor {
        ink.withAlphaComponent(current ? 1.0 : 0.6)
    }
}

final class EditorCanvasView: NSView {
    /// Equatable so a theme change to ANY token invalidates the CTLine cache.
    /// The hand-written comparison this replaced stopped at `function`, so
    /// recoloring only macroName/namespace/parameter/property/constant/
    /// operator/punctuation left stale glyphs on screen until something else
    /// bumped the cache.
    struct Colors: Equatable {
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
        var macroName: NSColor
        var namespace: NSColor
        var parameter: NSColor
        var property: NSColor
        var constant: NSColor
        var operatorColor: NSColor
        var punctuation: NSColor
        /// The gutter's "changed since HEAD" bar. Its own entry rather than
        /// `accent`: the theme accent moves with the user's colour scheme and
        /// this needs to stay the one colour that means "uncommitted", the way
        /// Xcode's does.
        var gitChange: NSColor
        /// Lines were removed here. The one change with no evidence in the
        /// text, so it gets its own colour — see `gitColor`.
        var gitDelete: NSColor
        /// The band an expanded change's replaced text is drawn on, its ink,
        /// and the hairline closing it. Warm rather than red: this text is not
        /// an error, it is history. Faint rather than loud: the live code above
        /// and below it is still the thing being read.
        var removedBg: NSColor
        var removedFg: NSColor
        var removedEdge: NSColor
        /// The breakpoint chip behind a line number, and the ink that reads on
        /// it. Yellow because it is a stop, not a change — nothing else in this
        /// gutter is yellow.
        var breakpoint: NSColor
        var breakpointInk: NSColor
        /// The row the program is stopped on, and the arrow that points at it.
        ///
        /// A band rather than the caret's wash, and green rather than the
        /// gutter's other colours: the git bar already spends red and blue in
        /// that strip, and the arrow is a SHAPE so it stays legible under
        /// Increase Contrast and to a colour-blind reader who sees the band
        /// and the caret row as the same grey.
        var debugStop: NSColor
        var debugStopInk: NSColor
        /// The wash over a row a live reload just replaced. The accent, faint
        /// — this is a notice that something arrived, not an error and not a
        /// change the reader made. It fades to nothing within about a second,
        /// so it is the only colour here whose job is to leave.
        var liveFlash: NSColor
        /// The flash behind a matching delimiter, and the ink redrawn on it.
        ///
        /// A fixed sRGB value and OPAQUE, in both themes.
        ///
        /// Two things had made it differ. It was `systemYellow`, which is a
        /// dynamic colour AppKit resolves to a different hue per appearance.
        /// And it was translucent, so even one hue at one alpha could not come
        /// out the same: 55% yellow over white and 55% over near-black are not
        /// the same colour, and tuning the alpha per theme to compensate only
        /// made the difference deliberate instead of removing it.
        ///
        /// Opaque is the only way two themes get the same pixels. The flash
        /// means the same thing in both, so it looks the same in both. The
        /// fade still runs through alpha — a fade has to — but it ends here.
        var bracketFill: NSColor
        var bracketInk: NSColor
    }

    weak var engine: EngineBridge?
    weak var scrollView: EditorScrollView?
    var paneIndex: Int = 0

    private(set) var wrapLines: Bool = true
    /// Gutter counts from the caret line instead of from 1.
    private(set) var relativeNumber: Bool = false
    func setWrapCols(_ cols: Int) { wrapCols = cols }

    /// Columns a wrapped row may use, or 0 when not wrapping.
    ///
    /// The face's number, because only the face knows the pane's width in
    /// points, the cell width, the gutter and what covers the right edge. It is
    /// the SAME number the band is pulled with and the wrap map is built with —
    /// a row count and a row's contents computed at two widths would place
    /// every line below the first wrap somewhere it is not drawn.
    private(set) var wrapCols: Int = 0
    /// Inline preview of the selected completion, drawn after the caret.
    /// Empty whenever the popup is closed or this pane is not the focused one.
    var ghostSuffix: String = "" {
        didSet { if ghostSuffix != oldValue { needsDisplay = true } }
    }

    private(set) var docLineCount: UInt32 = 1
    private(set) var showFocusRing: Bool = false
    /// Where the last right-click landed, and what identifier was under it.
    ///
    /// Captured in `menu(for:)` because that is the last moment the event
    /// exists: by the time an item is chosen the pointer has moved to the menu
    /// and the click is long gone, and both the anchor and the symbol are
    /// about where the question was asked.
    private var contextMenuPoint: CGPoint = .zero
    private var contextMenuSymbol: String = ""
    /// Held so a second Quick Help replaces the first rather than stacking.
    private var quickHelpPopover: NSPopover?
    private var datatipPopover: NSPopover?
    private var datatipWork: DispatchWorkItem?
    private var datatipAsk: DispatchWorkItem?
    private var datatipSymbol: String?
    var colors = Colors(
        bg: .textBackgroundColor, fg: .labelColor, dim: .secondaryLabelColor,
        accent: .controlAccentColor, sel: .selectedTextBackgroundColor,
        caret: .textColor, gutter: .tertiaryLabelColor, cursorLine: .quaternaryLabelColor,
        keyword: .systemCyan, string: .systemGreen, comment: .systemGray,
        number: .systemOrange, typeName: .systemTeal, function: .systemYellow,
        macroName: .systemPink, namespace: .systemMint, parameter: .labelColor,
        property: .systemTeal, constant: .systemOrange, operatorColor: .labelColor,
        punctuation: .secondaryLabelColor, gitChange: .systemBlue,
        gitDelete: .systemRed,
        removedBg: NSColor.systemYellow.withAlphaComponent(0.10),
        removedFg: NSColor.labelColor.withAlphaComponent(0.82),
        removedEdge: NSColor.systemYellow.withAlphaComponent(0.28),
        breakpoint: .systemYellow, breakpointInk: .black,
        debugStop: NSColor.systemGreen.withAlphaComponent(0.18),
        debugStopInk: .systemGreen,
        liveFlash: NSColor.systemBlue.withAlphaComponent(0.22),
        bracketFill: bracketYellow,
        bracketInk: .black
    )

    /// One yellow for the bracket flash, in both themes. Opaque — see
    /// `Colors.bracketFill`.
    static let bracketYellow = NSColor(
        srgbRed: 0.99, green: 0.79, blue: 0.22, alpha: 1
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
    // Shaped-line caches, shared by EVERY canvas in the process.
    //
    // They used to be per-instance, and panes do not have private content:
    // splitting a window shows the SAME document in two or three panes, and
    // each one shaped every visible line from scratch into its own 800-entry
    // cache. Measured in the packaged app with three panes on one file, the
    // `ctLine` miss rate was **26%** — a quarter of all CoreText shaping was
    // re-deriving what the pane next door had already computed.
    //
    // Sharing is only correct because the key covers everything that varies:
    // line text, spans, and the palette generation below. All panes in a
    // window resolve the same palette (`EditorHost.resolvedColors`), so the
    // generation is shared too — a per-instance counter would have given the
    // same colours different keys and defeated the whole thing.
    //
    // Main-thread only: `draw(_:)` and `apply(...)` are both AppKit callbacks.
    nonisolated(unsafe) private static let ctCache =
        LRUCache<UInt64, CTLine>(capacity: 2400)
    /// Visual-column → UTF-16 offset maps, keyed by a hash of the line text.
    ///
    /// Building one costs a `rangeOfComposedCharacterSequence` per character,
    /// and the draw needs it twice per line (once for find spans, once for
    /// syntax colouring). Scrolling redraws the same text over and over, so
    /// this is a near-100% hit rate for the cost of one string hash.
    nonisolated(unsafe) private static let vmapCache =
        LRUCache<UInt64, [Int]>(capacity: 2400)
    /// Palette identity behind the shared cache, and the colours that produced
    /// it. Bumped once per real theme change, by whichever canvas notices first.
    nonisolated(unsafe) private static var sharedColors: Colors?
    nonisolated(unsafe) private static var sharedColorGen: UInt64 = 0
    nonisolated(unsafe) private static var sharedFontSize: CGFloat = 0

    private var ctCache: LRUCache<UInt64, CTLine> { Self.ctCache }
    private var vmapCache: LRUCache<UInt64, [Int]> { Self.vmapCache }
    private var colorGen: UInt64 { Self.sharedColorGen }

    /// Row cache: contiguous band pulled from the engine (0-based start).
    private var bandStart: Int = 0
    /// Last BUFFER row the cache actually holds, 0-based; -1 when empty.
    ///
    /// Not derivable from `bandRows.count`, and that mattered: in wrap mode one
    /// buffer row becomes several segment entries, so the count overstates how
    /// far the band reaches. Measured on a wrapped Korean document, the cache
    /// held 117 entries covering rows 108…191 — 84 rows — while the coverage
    /// test read it as reaching row 225 and never re-pulled. Rows past 191 had
    /// no data and simply did not draw.
    private var bandEnd: Int = -1
    private var bandRows: [EditorLine] = []
    private lazy var metalRenderer: MetalTextRenderer? = {
        RendererChoice.useMetal ? MetalTextRenderer() : nil
    }()
    private let metalLayer = CAMetalLayer()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        if RendererChoice.useMetal { wantsLayer = true }
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        if RendererChoice.useMetal { wantsLayer = true }
    }

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
    /// Selection inside the marked string, in UTF-16 units. Korean IMEs move
    /// this range while recomposing a syllable; always reporting "at end" made
    /// AppKit replace the wrong portion when composition happened mid-line.
    private var markedSelectionRange = NSRange(location: 0, length: 0)
    /// Absolute UTF-16 document offset where this composition began.
    private var markedAnchorUTF16 = 0
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
    /// `discardMarkedText()` calls back through `NSTextInputClient` on some
    /// input methods. While a document action resolves the visible
    /// composition itself, suppress those callbacks so the same syllable
    /// cannot be inserted twice.
    private var resolvingMarkedTextForDocumentAction = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        installMetalLayerIfNeeded()
        // Claim focus on launch. Otherwise SwiftUI hands first responder to the
        // first focusable view — the project-tree Filter field — so the first
        // thing typed goes into the filter instead of the document.
        //
        // Against the ENGINE's focused pane, not pane 0.
        //
        // `viewDidMoveToWindow` fires whenever a canvas is added to a window,
        // not only at launch — installing a split creates fresh canvases and
        // runs it again. Hard-coded to pane 0 it then claimed the responder a
        // runloop turn later regardless of which pane the layout actually
        // focuses, and `EditorScrollView.apply` — which keeps the responder on
        // the engine-focused pane — took it straight back. Focus ping-ponged
        // through pane 0, redrawing its focus ring each way, which is why the
        // LEFT pane alone flickered on a switch into a split while the right
        // one sat still. `apply`'s own comment already named pane 0 as the
        // pane that wrongly ended up holding it.
        //
        // Deferring to the engine keeps the launch behaviour (one pane, focus
        // 0) and stops the fight: only the pane the engine focuses claims, and
        // that is the pane `apply` agrees with.
        let wantsFocus = engine?.editorSplit.focus ?? 0
        guard paneIndex == wantsFocus, let win = window else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, self.window === win else { return }
            if !(win.firstResponder is EditorCanvasView) {
                win.makeFirstResponder(self)
            }
        }
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        guard RendererChoice.useMetal else { return }
        configureMetalLayer()
        needsDisplay = true
    }

    override func keyDown(with event: NSEvent) {
        currentKeyEvent = event
        inputHandledKey = false
        EditorDiagnostics.reportIME(
            "keyDown", "code=\(event.keyCode) chars=\(event.characters ?? "")",
            marked: markedText
        )
        defer { currentKeyEvent = nil }
        interpretKeyEvents([event])
    }

    /// Commit text into the document. Single characters keep the core's insert
    /// path (auto-pairing, auto-indent); multi-char commits — IME output, emoji
    /// picker, pasted runs — go in verbatim.
    fileprivate func commitText(_ text: String) {
        guard !text.isEmpty, let engine else { return }
        // A new edit is a new bracket encounter. Fade-timer repaints do not
        // pass here, so a visible match still pulses exactly once.
        bracketKey = ""
        markedText = ""
        markedSelectionRange = NSRange(location: 0, length: 0)
        markedAnchorUTF16 = 0
        // Option dead keys can be cancelled before a base letter arrives. In
        // that state AppKit may hand us only a zero-width combining scalar;
        // inserting it creates an invisible source character and diagnostics
        // such as an unexplained Unicode code at an apparently empty column.
        // A real composed character contains a base scalar and is preserved.
        let scalarsForValidation = Array(text.unicodeScalars)
        let orphanedMark = scalarsForValidation.allSatisfy { scalar in
            switch scalar.properties.generalCategory {
            case .nonspacingMark, .spacingMark, .enclosingMark: return true
            default: return false
            }
        }
        let forbiddenControl = scalarsForValidation.contains { scalar in
            scalar.properties.generalCategory == .control
                && scalar.value != 0x09
                && scalar.value != 0x0A
                && scalar.value != 0x0D
        }
        guard !orphanedMark, !forbiddenControl else {
            needsDisplay = true
            return
        }
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
                // …and scroll to the caret ourselves, for the same reason. The
                // slow path gets this from `EditorScrollView.apply` reading
                // `scrollIntent == .caret` — but `apply` only runs on a SwiftUI
                // publish, and this path deliberately does not publish. Chrome
                // settles 120 ms after the LAST keystroke and the timer is reset
                // by each one, so while the user typed continuously the view
                // never followed the caret at all: type past the right edge, or
                // add lines at the bottom, and the text simply left the screen.
                (enclosingScrollView as? EditorScrollView)?.revealCaret()
                return
            }
            engine.dispatch(code: .char_, ch: scalar.value, mods: [])
        } else {
            engine.pasteText(text)
        }
        needsDisplay = true
    }

    /// Make the text currently visible at the IME caret part of the document
    /// before Save / Save As snapshots Core.
    ///
    /// Marked text intentionally lives only in this AppKit view while the IME
    /// owns it. A menu command can therefore reach `suisei_engine_save`
    /// without the final Hangul/Japanese composition ever entering Core. The
    /// disk then trails the screen by one syllable. Resolve the input context,
    /// insert the exact visible marked string once, and only then let the
    /// document action continue.
    @discardableResult
    func commitMarkedTextForDocumentAction() -> Bool {
        guard !markedText.isEmpty, engine != nil else { return false }
        let pending = markedText
        resolvingMarkedTextForDocumentAction = true
        inputContext?.discardMarkedText()
        resolvingMarkedTextForDocumentAction = false
        markedText = ""
        commitText(pending)
        noteContentChanged()
        return true
    }

    fileprivate func fallbackToLegacyKeyPath() {
        guard let e = currentKeyEvent else { return }
        bracketKey = ""
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

    /// Set when the engine changed under pixels AppKit had already drawn ahead
    /// of the viewport. Consumed by `prepareContent(in:)`.
    private var overdrawPredatesChange = false

    /// AppKit's responsive scrolling draws BEYOND the viewport so a scroll has
    /// pixels ready — the engine's overscan exists to feed exactly that (see
    /// the `rows` computation in `scene.rs`, citing WWDC 2013-215). Those rows
    /// were drawn from the state BEFORE the last change, and invalidating only
    /// the viewport leaves them on the layer to be revealed by the next scroll.
    ///
    /// Select-all is where it shows: rows on screen repaint highlighted, then
    /// scrolling uncovers bands drawn before the selection existed — measured,
    /// with lines 20–24 selected, 26–30 not, and 32–35 selected again, because
    /// only some tiles were redrawn. Every whole-document change has that
    /// shape: a theme switch, a font size, a syntax frame landing.
    ///
    /// Assigning `preparedContentRect` from `noteContentChanged` was tried
    /// first and only partly worked — AppKit re-prepared some bands and left
    /// others. Redrawing here does work, because this is the moment AppKit
    /// itself has decided it wants that area. The cost lands on the first
    /// scroll after a change rather than on every keystroke, which is the one
    /// path in this view that cannot afford a three-viewport repaint.
    override func prepareContent(in rect: NSRect) {
        if overdrawPredatesChange {
            overdrawPredatesChange = false
            setNeedsDisplay(rect)
        }
        super.prepareContent(in: rect)
    }

    /// Engine content changed — drop cached rows and repaint the viewport.
    func noteContentChanged() {
        bandRows.removeAll(keepingCapacity: true)
        bandEnd = -1
        overdrawPredatesChange = true
        closeRevealIfItsChangeIsGone()
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
        relativeNumber: Bool,
        docLineCount: UInt32
    ) {
        var repaint = false
        // Per-instance: THIS canvas needs a repaint.
        if !Self.colorsEqual(self.colors, colors) {
            repaint = true
        }
        // Process-wide: the shared cache turns over once per real theme change,
        // not once per pane. Whichever canvas sees the new palette first bumps
        // the generation; the others then compare equal and leave it alone —
        // otherwise pane 2 would flush what pane 1 had just repopulated.
        if Self.sharedColors == nil || !Self.colorsEqual(Self.sharedColors!, colors) {
            Self.sharedColors = colors
            Self.sharedColorGen &+= 1
            Self.ctCache.removeAll(keepingCapacity: true)
        }
        if self.showFocusRing != showFocusRing { repaint = true }
        self.showFocusRing = showFocusRing
        self.colors = colors
        self.wrapLines = wrapLines
        // Every number in the gutter changes when this flips, and again on
        // every caret move while it is on — the cached CTLines are keyed by the
        // number they draw, so they stay valid; only the repaint is needed.
        if self.relativeNumber != relativeNumber {
            self.relativeNumber = relativeNumber
            repaint = true
        }
        self.docLineCount = max(1, docLineCount)
        let fontSize = EditorMetrics.fontSize
        if fontSize != Self.sharedFontSize {
            Self.sharedFontSize = fontSize
            Self.ctCache.removeAll(keepingCapacity: true)
            // The visual→UTF-16 map is tab-expansion and character width only,
            // so it survives a font-size change. The shaped lines do not.
            repaint = true
        }
        if repaint { noteContentChanged() }
    }

    /// Rows `[r0, r1]` from cache, pulling from the engine when uncovered.
    private func rows(_ r0: Int, _ r1: Int) -> ArraySlice<EditorLine> {
        let start = max(0, r0)
        let end = max(start, min(r1, Int(docLineCount) - 1))
        // Against the rows the band really holds, not against how many entries
        // it took to hold them — see `bandEnd`.
        let covered = !bandRows.isEmpty && start >= bandStart && end <= bandEnd
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
                    max: min(160, want1 - cursor + 1),
                    wrapCols: wrapCols
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
            bandEnd = pulled.last.map { Int($0.lineNo) - 1 } ?? (want0 - 1)
        }
        // Slice by lineNo bounds (wrap rows share lineNo with their primary).
        let lo = bandRows.firstIndex { Int($0.lineNo) - 1 >= start } ?? bandRows.endIndex
        let hi = bandRows.lastIndex { Int($0.lineNo) - 1 <= end }.map { $0 + 1 } ?? lo
        let slice = bandRows[lo..<hi]
        if EditorDiagnostics.bandGaps {
            EditorDiagnostics.reportBand(
                pane: paneIndex,
                want: start...end,
                bandStart: bandStart,
                bandCount: bandRows.count,
                first: bandRows.first.map { Int($0.lineNo) - 1 },
                last: bandRows.last.map { Int($0.lineNo) - 1 },
                gotFirst: slice.first.map { Int($0.lineNo) - 1 },
                gotLast: slice.last.map { Int($0.lineNo) - 1 },
                gotCount: slice.count
            )
        }
        return slice
    }

    /// Real-time glyph x of the caret in canvas coordinates, from the CURRENT
    /// snapshot — not the last paint. It lays the caret against the SAME CTLine
    /// the draw uses (real glyph advances, so CJK is exact and never drifts off
    /// the cell grid), a cache hit whenever the line was just painted, so the
    /// caret reveal never lags a frame. Nil when the caret's line can't be
    /// fetched — the caller then falls back to a cell estimate to bring the row
    /// on screen and the next pass refines it.
    ///
    /// The row is passed in rather than read from `chrome.cursorRow`: that
    /// snapshot is stale for the whole of a continuous typing run.
    func liveCaretGlyphX(row: Int) -> CGFloat? {
        guard engine != nil else { return nil }
        let band = rows(row, row)
        // The row core marked as the caret's — with a wrapped line that is the
        // SEGMENT the caret is on, and its `caretUTF16` is relative to that
        // segment's text. Falling back to the line's first row is only for the
        // case where the band has no caret in it at all.
        guard let line = band.first(where: { Int($0.lineNo) - 1 == row && $0.isCursor })
            ?? band.first(where: { Int($0.lineNo) - 1 == row })
        else { return nil }
        let font = EditorMetrics.monospaced(EditorMetrics.fontSize, weight: .regular)
        let ct = ctLine(for: line, font: font)
        let maxIdx = (line.text as NSString).length
        let idx = CFIndex(min(max(0, Int(line.caretUTF16)), maxIdx))
        return EditorMetrics.gutter + CTLineGetOffsetForStringIndex(ct, idx, nil)
    }

    private static func colorsEqual(_ a: Colors, _ b: Colors) -> Bool {
        a == b
    }

    /// Line numbers, laid out once each.
    ///
    /// They were drawn with `NSString.draw(at:withAttributes:)` — which builds
    /// a framesetter and shapes the glyphs from scratch — once per visible line
    /// per repaint. Measured in a release build that was 0.011 ms × 59 lines =
    /// **0.65 ms of the draw's 1.6 ms**, spent re-shaping digits that change
    /// only when the view scrolls. The text lines had a cache all along; the
    /// gutter did not.
    private struct GutterKey: Hashable {
        let number: UInt32
        let isCursor: Bool
        let onBreakpoint: Bool
        let colorGen: UInt64
    }
    private let gutterCache = LRUCache<GutterKey, (line: CTLine, width: CGFloat)>(capacity: 4000)

    /// What the gutter prints for a row.
    ///
    /// Absolute unless relative numbering is on, and then still absolute on the
    /// caret's own row: that is where you are, and "0" is not an answer to
    /// where you are. Every other row shows its distance, which is the number
    /// you would type after a motion.
    private func gutterNumber(for line: EditorLine) -> UInt32 {
        // A wrapped line has ONE number, on its first row. The rows after it
        // are the same line, and repeating the number down the gutter says
        // there are three line 26s — which is what it looked like.
        if line.isWrapContinuation { return 0 }
        guard relativeNumber, !line.isCursor else { return line.lineNo }
        let caret = caretLineNo
        guard caret > 0 else { return line.lineNo }
        return UInt32(abs(Int(line.lineNo) - Int(caret)))
    }

    /// 1-based caret row. Pulled live for the same reason the reveal is: the
    /// typing fast path publishes no chrome, and a gutter counting from a stale
    /// caret would be off by the length of the run you just typed.
    private var caretLineNo: UInt32 {
        guard relativeNumber, let engine else { return 0 }
        return UInt32(engine.caretRowVCol().row + 1)
    }

    private func gutterLine(
        _ number: UInt32, isCursor: Bool, font: NSFont,
        onBreakpoint: Bool = false
    ) -> (line: CTLine, width: CGFloat) {
        // 0 is not a line: it is `gutterNumber`'s way of saying this row
        // continues the one above and owns no number.
        if number == 0 {
            return (CTLineCreateWithAttributedString(NSAttributedString(string: "")), 0)
        }
        let key = GutterKey(
            number: number, isCursor: isCursor,
            onBreakpoint: onBreakpoint, colorGen: colorGen
        )
        if let hit = gutterCache[key] { return hit }
        // On the chip the number is read against yellow, so it takes the ink
        // that goes with it rather than the gutter's usual grey.
        let attrs: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: onBreakpoint
                ? colors.breakpointInk
                : (isCursor
                    ? colors.accent.withAlphaComponent(0.92) : colors.gutter),
        ]
        let ct = CTLineCreateWithAttributedString(
            NSAttributedString(string: "\(number)", attributes: attrs)
        )
        let width = CGFloat(CTLineGetTypographicBounds(ct, nil, nil, nil))
        let entry = (line: ct, width: width)
        gutterCache[key] = entry
        return entry
    }

    /// Width of a line number, for placing the breakpoint chip around it.
    private func gutterNumberWidth(_ number: UInt32, font: NSFont) -> CGFloat {
        gutterLine(number, isCursor: false, font: font).width
    }

    /// Cache identity for one rendered line, as a 64-bit hash.
    ///
    /// This used to build a `String`: interpolate lineNo / length / text hash /
    /// colorGen, then `+=` a fragment for every span, and use THAT as the
    /// dictionary key. Several heap allocations per visible line per repaint —
    /// to look up a cache whose entire job is to avoid work. Hashing allocates
    /// nothing.
    ///
    /// No weaker than what it replaces: the old key already carried
    /// `text.hashValue` rather than the text, so its collision domain was the
    /// same. `ctCache` is cleared outright on a colour or font-size change, so
    /// `colorGen` here is belt-and-braces.
    private func cacheKey(for line: EditorLine) -> UInt64 {
        var h = Hasher()
        // NOT `line.lineNo`. `attributedLine` reads the text and the spans and
        // nothing else, so a shaped line does not depend on which row it sits
        // at — two identical lines render identically. Keying on the row number
        // meant one Return invalidated every cached line below the caret,
        // which is precisely the edit that needs the cache most: measured 311
        // misses against 504 hits while typing.
        h.combine(line.text)
        h.combine(colorGen)
        for s in line.spans {
            h.combine(s.start)
            h.combine(s.end)
            h.combine(s.kind)
        }
        return UInt64(bitPattern: Int64(h.finalize()))
    }

    // MARK: - Draw

    private func installMetalLayerIfNeeded() {
        guard RendererChoice.useMetal, metalRenderer != nil, let root = layer else { return }
        if metalLayer.superlayer !== root {
            metalLayer.removeFromSuperlayer()
            root.addSublayer(metalLayer)
        }
        configureMetalLayer()
    }

    private func configureMetalLayer() {
        guard RendererChoice.useMetal, let device = MetalTextRenderer.device else { return }
        metalLayer.device = device
        metalLayer.pixelFormat = .bgra8Unorm
        metalLayer.framebufferOnly = true
        metalLayer.displaySyncEnabled = true
        metalLayer.maximumDrawableCount = 3
        metalLayer.isOpaque = true
        metalLayer.contentsScale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
    }

    /// Builds one complete visible frame for the instanced Metal renderer.
    /// CoreText remains the shaper; all glyph fallback and real advances are
    /// therefore identical to the CPU path below. Returning false makes the
    /// same draw immediately fall back to CoreText.
    private func renderMetalViewport() -> Bool {
        guard RendererChoice.useMetal, let renderer = metalRenderer else { return false }
        installMetalLayerIfNeeded()
        guard metalLayer.superlayer != nil else { return false }

        let viewport = (scrollView?.documentVisibleRect ?? visibleRect).intersection(bounds)
        guard !viewport.isNull, viewport.width > 0, viewport.height > 0 else { return false }
        let scale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        metalLayer.frame = viewport
        metalLayer.contentsScale = scale
        metalLayer.drawableSize = CGSize(
            width: max(1, ceil(viewport.width * scale)),
            height: max(1, ceil(viewport.height * scale))
        )
        CATransaction.commit()

        let lineH = EditorMetrics.lineHeight
        let gutter = EditorMetrics.gutter
        let fontSize = EditorMetrics.fontSize
        let cell = EditorMetrics.cellWidth
        let font = EditorMetrics.monospaced(fontSize, weight: .regular)
        let ascent = font.ascender
        let gap = EditorMetrics.gutterTextGap
        // The band is chosen in VISUAL rows and pulled in buffer rows: an
        // expanded change shifts everything below it, so the visible span is
        // not the buffer span.
        let r0 = nearestBufferRow(atY: viewport.minY)
        let r1 = max(r0, nearestBufferRow(atY: viewport.maxY))
        let band = rows(r0, r1)

        renderer.beginFrame()
        syncBreakpointAnimations(band)
        syncLiveFlashes()
        for (r, c) in hunkHoverRects(lineH: lineH) { renderer.addRect(r, c) }
        for (r, c) in gitBars(band, lineH: lineH) { renderer.addRect(r, c) }
        bracketRects.removeAll(keepingCapacity: true)
        // Every row the band holds, continuations included.
        //
        // This used to be `where !line.isWrapContinuation`, with the rows after
        // the first collapsed into a `⋯` at the right edge. Core has always
        // split a wrapped line into a row per segment; the renderer drew the
        // first one and dropped the rest on the floor, so turning Wrap Lines on
        // did not wrap a long line — it hid everything past the first screenful
        // of it. `segment` is counted here rather than carried over the ABI:
        // the band arrives in order, so consecutive rows sharing a `lineNo` are
        // that line's segments in sequence.
        var segment = 0
        var previousLineNo: UInt32 = .max
        for line in band {
            let baseRow = max(0, Int(line.lineNo) - 1)
            segment = line.lineNo == previousLineNo ? segment + 1 : 0
            previousLineNo = line.lineNo
            let y = visualY(baseRow) + CGFloat(segment) * lineH
            if y + lineH < viewport.minY || y > viewport.maxY { continue }
            let rowRect = CGRect(x: viewport.minX, y: y, width: viewport.width, height: lineH)

            if line.isCursor { renderer.addRect(rowRect, colors.cursorLine) }
            // After the caret wash on purpose: the two are frequently the same
            // row, and the one that has to win is the one saying where the
            // PROGRAM is.
            if line.isStoppedLine || line.isFrameLine {
                // The frame you are READING gets a fainter band and a hollow
                // arrow. Same information, said quieter, because the loud one
                // has to stay reserved for where execution actually is.
                let solid = line.isStoppedLine
                renderer.addRect(
                    Self.stopBandRect(rowRect),
                    solid ? colors.debugStop : colors.debugStop.withAlphaComponent(
                        colors.debugStop.alphaComponent * 0.45)
                )
                let bars = solid
                    ? Self.stopArrowBars(atY: y, lineH: lineH)
                    : Self.frameArrowBars(atY: y, lineH: lineH)
                for bar in bars { renderer.addRect(bar, colors.debugStopInk) }
            }
            let flash = liveFlash(line.lineNo)
            if flash > 0.01 {
                let ink = liveFlashColor(line.lineNo)
                renderer.addRect(
                    rowRect, ink.withAlphaComponent(ink.alphaComponent * flash)
                )
            }
            let bpPhase = breakpointPhase(line)
            if bpPhase > 0.001 {
                // Same chip the CoreText path draws, squared off: this
                // renderer has rects only, and a 4pt radius on a 15pt-wide
                // chip is the part it can afford to lose.
                renderer.addRect(
                    Self.breakpointChip(
                        atY: y, lineH: lineH,
                        numberWidth: gutterNumberWidth(gutterNumber(for: line), font: font),
                        phase: bpPhase
                    ),
                    colors.breakpoint.withAlphaComponent(bpPhase)
                )
            }

            if EditorDiagnostics.metal, renderer.atlas.isFull {
                EditorDiagnostics.reportAtlasFull(
                    row: Int(line.lineNo),
                    resident: renderer.atlas.residentCount
                )
            }
            let gutterEntry = gutterLine(
                gutterNumber(for: line), isCursor: line.isCursor, font: font
            )
            guard renderer.addLine(
                gutterEntry.line,
                origin: CGPoint(x: max(4, gutter - gap - gutterEntry.width), y: y + (lineH - fontSize) * 0.5 - 1 + ascent),
                fallbackColor: line.isCursor ? colors.accent : colors.gutter,
                scale: scale
            ) else { return false }

            let textY = y + (lineH - fontSize) * 0.5 - 1
            let ct = ctLine(for: line, font: font)
            var displayCT = ct
            var compositionCaretUTF16: Int?
            var compositionStartUTF16: Int?
            if line.isCursor, !markedText.isEmpty {
                let composed = NSMutableAttributedString(attributedString: attributedLine(line, font: font))
                let at = min(max(0, Int(line.caretUTF16)), composed.length)
                composed.insert(
                    NSAttributedString(
                        string: markedText,
                        attributes: [.font: font, .foregroundColor: colors.fg]
                    ),
                    at: at
                )
                displayCT = CTLineCreateWithAttributedString(composed)
                compositionStartUTF16 = at
                compositionCaretUTF16 = at + markedText.utf16.count
            }

            let visualMap = visualToUTF16Map(line.text)
            for span in line.spans where span.kind == 248 || span.kind == 249 {
                let v0 = Int(span.start)
                let v1 = Int(span.end)
                guard v1 > v0, v0 < visualMap.count else { continue }
                let u0 = visualMap[v0]
                let u1 = v1 < visualMap.count ? visualMap[v1] : (line.text as NSString).length
                guard u1 > u0 else { continue }
                let x0 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(u0), nil)
                let x1 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(u1), nil)
                let current = span.kind == 249
                // Metal draws quads only, so the squiggle is sampled into 1pt
                // columns off the same curve the CoreText path strokes.
                let cy = FindSquiggle.centreY(lineTop: y, lineHeight: lineH)
                let thick = FindSquiggle.thickness(current: current)
                let ink = FindSquiggle.color(current: current)
                var px = x0
                while px < x1 {
                    let w = min(1, x1 - px)
                    renderer.addRect(
                        CGRect(
                            x: px,
                            y: cy + FindSquiggle.offset(at: px - x0) - thick / 2,
                            width: w, height: thick
                        ),
                        ink
                    )
                    px += 1
                }
            }

            if line.hasSelection {
                let x0 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(line.selU0), nil)
                let x1 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(line.selU1), nil)
                renderer.addRect(
                    CGRect(x: x0, y: y, width: max(cell, x1 - x0), height: lineH),
                    colors.sel
                )
            }

            for span in line.spans where span.kind == 254 {
                let key = "\(line.lineNo):\(span.start)"
                let expired = CACurrentMediaTime() - bracketShownAt >= Self.bracketFlashDuration
                if key != bracketKey || (!viewport.intersects(rowRect) && expired) {
                    bracketKey = key
                    bracketShownAt = CACurrentMediaTime()
                    startBracketFade()
                }
                let age = CACurrentMediaTime() - bracketShownAt
                guard age < Self.bracketFlashDuration else { continue }
                let x0 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(span.start), nil)
                let x1 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(span.end), nil)
                let raw = min(1.0, max(0.0, (Self.bracketFlashDuration - age) / Self.bracketFadeTail))
                let fade = raw * raw * (3 - 2 * raw)
                renderer.addRect(
                    CGRect(x: x0 - 1, y: y + 1, width: max(cell, x1 - x0) + 2, height: lineH - 2),
                    colors.bracketFill.withAlphaComponent(fade)
                )
                bracketRects.append(rowRect)
            }

            guard renderer.addLine(
                displayCT,
                origin: CGPoint(x: gutter, y: textY + ascent),
                fallbackColor: colors.fg,
                scale: scale
            ) else { return false }

            if let compositionStartUTF16, let compositionCaretUTF16 {
                let x0 = gutter + CTLineGetOffsetForStringIndex(displayCT, CFIndex(compositionStartUTF16), nil)
                let x1 = gutter + CTLineGetOffsetForStringIndex(displayCT, CFIndex(compositionCaretUTF16), nil)
                renderer.addRect(CGRect(x: x0, y: y + lineH - 2, width: max(1, x1 - x0), height: 1), colors.fg)
            }


            if line.isCursor {
                let caretIndex = compositionCaretUTF16 ?? Int(line.caretUTF16)
                let caretX = gutter + CTLineGetOffsetForStringIndex(displayCT, CFIndex(caretIndex), nil)
                let baseline = textY + font.ascender
                let capTop = baseline - font.capHeight
                let descBottom = baseline - font.descender
                let caretRect = EditorMetrics.caretRect(
                    x: caretX,
                    capTop: capTop,
                    descBottom: descBottom,
                    advance: CTLineGetOffsetForStringIndex(displayCT, CFIndex(caretIndex + 1), nil)
                        - (caretX - gutter)
                )
                lastCaretRect = caretRect
                if let win = window,
                   engine?.editorSplit.isSplit != true || paneIndex == engine?.editorSplit.focus {
                    let inWindow = convert(caretRect, to: nil)
                    let height = win.contentView?.bounds.height ?? win.frame.height
                    engine?.caretFrameInWindow = CGRect(
                        x: inWindow.minX, y: height - inWindow.maxY,
                        width: inWindow.width, height: inWindow.height
                    )
                }
                renderer.addRect(caretRect, colors.caret)
            }

            for span in line.spans {
                if span.kind == 250 {
                    let x = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(span.start), nil)
                    let baseline = textY + font.ascender
                    let capTop = baseline - font.capHeight
                    let descBottom = baseline - font.descender
                    renderer.addRect(
                        CGRect(x: x, y: (capTop - 1).rounded(), width: 2, height: (descBottom - capTop + 2).rounded()),
                        colors.caret.withAlphaComponent(0.85)
                    )
                } else if span.kind >= 251 && span.kind <= 253 {
                    let x0 = gutter + CGFloat(span.start) * cell
                    let width = max(cell, CGFloat(span.end - span.start) * cell)
                    let color: NSColor = span.kind == 251 ? .systemRed : (span.kind == 252 ? .systemOrange : .systemBlue)
                    var x = x0
                    var up = false
                    while x < x0 + width {
                        renderer.addRect(
                            CGRect(x: x, y: y + lineH - (up ? 3 : 2), width: min(2, x0 + width - x), height: 1),
                            color.withAlphaComponent(0.85)
                        )
                        x += 2
                        up.toggle()
                    }
                }
            }
        }

        if EditorDiagnostics.metal {
            EditorDiagnostics.reportMetal(
                viewport: viewport,
                bounds: bounds,
                layerFrame: metalLayer.frame,
                rects: renderer.rectCount,
                rectCapacity: renderer.rectCapacity,
                glyphs: renderer.glyphCount,
                glyphCapacity: renderer.glyphCapacity
            )
        }
        let presented = renderer.present(
            to: metalLayer,
            size: viewport.size,
            scroll: viewport.origin,
            background: colors.bg
        )
        metalLayer.isHidden = !presented
        return presented
    }

    /// Called directly by `EditorScrollView` while its clip view moves. Metal
    /// does not participate in AppKit's responsive-scroll redraw callbacks, so
    /// relying on `draw(_:)` alone can freeze the texture until scrolling ends.
    func viewportDidScroll() {
        guard RendererChoice.useMetal else { return }
        if renderMetalViewport() { return }
        metalLayer.isHidden = true
        setNeedsDisplay(visibleRect)
    }

    override func draw(_ dirtyRect: NSRect) {
        let t0 = DispatchTime.now().uptimeNanoseconds
        defer {
            PerfProbe.record(
                "EditorCanvasView.draw",
                Double(DispatchTime.now().uptimeNanoseconds - t0) / 1_000_000
            )
        }
        if renderMetalViewport() { return }
        metalLayer.isHidden = true
        colors.bg.setFill()
        dirtyRect.fill()

        let lineH = EditorMetrics.lineHeight
        let gutter = EditorMetrics.gutter
        let fontSize = EditorMetrics.fontSize
        let cell = EditorMetrics.cellWidth
        let font = EditorMetrics.monospaced(fontSize, weight: .regular)
        let ascent = font.ascender
        let gap = EditorMetrics.gutterTextGap
        guard let cg = NSGraphicsContext.current?.cgContext else { return }

        let r0 = nearestBufferRow(atY: dirtyRect.minY)
        let r1 = max(r0, nearestBufferRow(atY: dirtyRect.maxY))
        let band = rows(r0, r1)
        // Rebuilt below by whichever rows carry a hint this pass.
        bracketRects.removeAll(keepingCapacity: true)

        PerfProbe.measure("   draw: gutter decorations") {
            syncBreakpointAnimations(band)
            syncLiveFlashes()
            for (r, c) in hunkHoverRects(lineH: lineH) {
                c.setFill()
                r.fill()
            }
            drawShownChange(lineH: lineH, font: font, cg: cg)
            for (r, c) in gitBars(band, lineH: lineH) {
                c.setFill()
                r.fill()
            }
        }
        let rowLoopStart = DispatchTime.now().uptimeNanoseconds
        // Every row, continuations included — see the note in the Metal path.
        var segment = 0
        var previousLineNo: UInt32 = .max
        for line in band {
            let baseRow = max(0, Int(line.lineNo) - 1)
            segment = line.lineNo == previousLineNo ? segment + 1 : 0
            previousLineNo = line.lineNo
            let y = visualY(baseRow) + CGFloat(segment) * lineH
            if y + lineH < dirtyRect.minY || y > dirtyRect.maxY { continue }

            let rowRect = CGRect(x: 0, y: y, width: bounds.width, height: lineH)
            // A row still arriving is drawn only as far as the gap has opened.
            //
            // Without this the rows below — which are mid-slide and therefore
            // still overlapping it — draw their text through this one, and the
            // two are legible at once. The clip is what makes the new line
            // look revealed rather than superimposed.
            var clipped = false
            if let o = liveOpening, o.rows > 0,
               baseRow >= o.below - o.rows, baseRow < o.below {
                let top = visualY(o.below - o.rows)
                let open = CGFloat(o.rows) * lineH * liveOpenProgress
                cg.saveGState()
                cg.clip(to: CGRect(x: 0, y: top, width: bounds.width, height: open))
                clipped = true
            }
            defer { if clipped { cg.restoreGState() } }

            if line.isCursor {
                colors.cursorLine.setFill()
                rowRect.fill()
            }
            if line.isStoppedLine || line.isFrameLine {
                let solid = line.isStoppedLine
                (solid
                    ? colors.debugStop
                    : colors.debugStop.withAlphaComponent(
                        colors.debugStop.alphaComponent * 0.45)).setFill()
                Self.stopBandRect(rowRect).fill()
                let arrow = Self.stopArrowPath(atY: y, lineH: lineH)
                colors.debugStopInk.setFill()
                if solid {
                    arrow.fill()
                } else {
                    // Hollow: the same outline, stroked. A different SHAPE
                    // would be a second thing to learn; the same shape emptied
                    // out reads as "this one, but not the live one".
                    arrow.lineWidth = 1.5
                    colors.debugStopInk.setStroke()
                    arrow.stroke()
                }
            }
            // A row someone else just wrote. Behind the text, ahead of nothing
            // else: it has to be legible under the glyphs, and it fades out.
            let flash = liveFlash(line.lineNo)
            if flash > 0.01 {
                let ink = liveFlashColor(line.lineNo)
                ink.withAlphaComponent(ink.alphaComponent * flash).setFill()
                rowRect.fill()
            }


            PerfProbe.measure("    gutter number") {
                let ln = gutterLine(
                    gutterNumber(for: line), isCursor: line.isCursor, font: font,
                    onBreakpoint: breakpointPhase(line) > 0.5
                )
                let phase = breakpointPhase(line)
                if phase > 0.001 {
                    let chip = Self.breakpointChip(
                        atY: y, lineH: lineH, numberWidth: ln.width, phase: phase
                    )
                    colors.breakpoint.withAlphaComponent(phase).setFill()
                    NSBezierPath(
                        roundedRect: chip,
                        xRadius: Self.breakpointChipRadius * phase,
                        yRadius: Self.breakpointChipRadius * phase
                    ).fill()
                }
                cg.saveGState()
                cg.textMatrix = .identity
                cg.translateBy(
                    x: max(4, gutter - gap - ln.width),
                    y: y + (lineH - fontSize) * 0.5 - 1 + ascent
                )
                cg.scaleBy(x: 1, y: -1)
                CTLineDraw(ln.line, cg)
                cg.restoreGState()
            }

            cg.saveGState()
            cg.clip(to: CGRect(x: gutter, y: y, width: max(0, bounds.width - gutter), height: lineH))

            // Built before the selection fill: both the highlight and the caret
            // are positioned by measuring THIS line, not the core's cell grid.
            let textY = y + (lineH - fontSize) * 0.5 - 1
            let ct = PerfProbe.measure("   ctLine total") { ctLine(for: line, font: font) }
            var displayCT = ct
            var compositionCaretUTF16: Int?
            if line.isCursor, !markedText.isEmpty {
                // Insert the provisional composition into a display-only
                // attributed line. Painting it over the existing suffix made
                // mid-line Hangul look corrupted because both glyph runs
                // occupied the same pixels; this shifts the suffix exactly as
                // a native text editor does without mutating Core prematurely.
                let composed = NSMutableAttributedString(
                    attributedString: attributedLine(line, font: font)
                )
                let at = min(max(0, Int(line.caretUTF16)), composed.length)
                composed.insert(
                    NSAttributedString(
                        string: markedText,
                        attributes: [
                            .font: font,
                            .foregroundColor: colors.fg,
                            .underlineStyle: NSUnderlineStyle.single.rawValue,
                        ]
                    ),
                    at: at
                )
                displayCT = CTLineCreateWithAttributedString(composed)
                compositionCaretUTF16 = at + markedText.utf16.count
            }

            // Find results: a yellow squiggle under every match, brighter and
            // thicker under the current one. These spans are display columns,
            // so resolve through the same visual→UTF-16 map used by syntax
            // before asking CoreText for real glyph positions.
            let visualMap = visualToUTF16Map(line.text)
            for sp in line.spans where sp.kind == 248 || sp.kind == 249 {
                let v0 = Int(sp.start)
                let v1 = Int(sp.end)
                guard v1 > v0, v0 < visualMap.count else { continue }
                let u0 = visualMap[v0]
                let u1 = v1 < visualMap.count ? visualMap[v1] : (line.text as NSString).length
                guard u1 > u0 else { continue }
                let x0 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(u0), nil)
                let x1 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(u1), nil)
                let current = sp.kind == 249
                let cy = FindSquiggle.centreY(lineTop: y, lineHeight: lineH)
                let wave = NSBezierPath()
                // Half-point steps: smooth enough to read as a curve at any
                // scale, and simpler than fitting béziers to a sine.
                var px = x0
                wave.move(to: CGPoint(x: px, y: cy + FindSquiggle.offset(at: 0)))
                while px < x1 {
                    px = min(px + 0.5, x1)
                    wave.line(to: CGPoint(x: px, y: cy + FindSquiggle.offset(at: px - x0)))
                }
                wave.lineWidth = FindSquiggle.thickness(current: current)
                wave.lineCapStyle = .round
                wave.lineJoinStyle = .round
                FindSquiggle.color(current: current).setStroke()
                wave.stroke()
            }

            if line.hasSelection {
                // Same reason as the caret: measure against the drawn line so
                // the highlight tracks CJK glyphs instead of the cell grid.
                let x0 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(line.selU0), nil)
                let x1 = gutter + CTLineGetOffsetForStringIndex(ct, CFIndex(line.selU1), nil)
                let w = max(cell, x1 - x0)
                colors.sel.setFill()
                CGRect(x: x0, y: y, width: w, height: lineH).fill()
            }

            PerfProbe.measure("    CTLineDraw") {
                cg.saveGState()
                cg.textMatrix = .identity
                cg.translateBy(x: gutter, y: textY + ascent)
                cg.scaleBy(x: 1, y: -1)
                CTLineDraw(displayCT, cg)
                cg.restoreGState()
            }

            // Soft-wrap tail exists but can't stack in the absolute row model —
            // show a clipped-content marker at the right edge.

            if line.isCursor {
                // Resolve the caret against the DRAWN line, not the core's
                // terminal cell grid: CJK counts as 2 cells there, but CoreText
                // lays glyphs out by their real advances, so `vcol * cell` sat
                // far right of the text on Hangul/Japanese lines (and dragged
                // the IME composition along with it).
                let caretIndex = compositionCaretUTF16 ?? Int(line.caretUTF16)
                let caretX = gutter + CTLineGetOffsetForStringIndex(
                    displayCT, CFIndex(caretIndex), nil
                )

                // Span the band the LETTERS occupy — cap height down to the
                // descender — not the full em box. `ascender` carries accent
                // clearance well above the capitals (~3.4px at 14pt), so a
                // caret drawn to it visibly pokes over the text.
                let baseline = textY + font.ascender
                let capTop = baseline - font.capHeight
                let descBottom = baseline - font.descender
                let caretRect = EditorMetrics.caretRect(
                    x: caretX,
                    capTop: capTop,
                    descBottom: descBottom,
                    advance: CTLineGetOffsetForStringIndex(displayCT, CFIndex(caretIndex + 1), nil)
                        - (caretX - gutter)
                )
                // Remember it for `firstRect(forCharacterRange:)` — the input
                // method places its candidate window against this.
                lastCaretRect = caretRect
                // …and hand it to SwiftUI (top-left origin) so caret-anchored
                // overlays like the completion popup can find it. ONLY the
                // focused pane may publish this: under a split every pane draws
                // its own caret, and the last one to draw used to win, so the
                // completion popup jumped to whichever pane repainted last
                // instead of the one being typed in.
                if let win = window,
                   engine?.editorSplit.isSplit != true
                    || paneIndex == engine?.editorSplit.focus {
                    let inWindow = convert(caretRect, to: nil)
                    let h = win.contentView?.bounds.height ?? win.frame.height
                    engine?.caretFrameInWindow = CGRect(
                        x: inWindow.minX, y: h - inWindow.maxY,
                        width: inWindow.width, height: inWindow.height
                    )
                }
                colors.caret.setFill()
                caretRect.fill()

                // Xcode-style inline preview: the rest of the selected
                // suggestion, faint, starting where the caret is. Drawn AFTER
                // the caret so the bar stays crisp over it, and clipped to the
                // pane so a long symbol cannot run out past the edge.
                //
                // No layout is re-derived for it — `caretX` and `baseline` are
                // the ones the caret just used, so the preview cannot drift
                // from the caret the way a separately-measured overlay would.
                if !ghostSuffix.isEmpty, compositionCaretUTF16 == nil {
                    let ghost = NSAttributedString(
                        string: ghostSuffix,
                        attributes: [
                            .font: font,
                            .foregroundColor: colors.fg.withAlphaComponent(0.38),
                        ]
                    )
                    cg.saveGState()
                    cg.clip(to: CGRect(
                        x: caretX, y: capTop - 1,
                        width: max(0, bounds.width - caretX - 4),
                        height: descBottom - capTop + 2
                    ))
                    ghost.draw(at: CGPoint(x: caretX, y: textY))
                    cg.restoreGState()
                }
            }
            for sp in line.spans where sp.kind == 254 {
                // Matching bracket: a visible match flashes once. If CoreText
                // is drawing an off-viewport row during responsive-scroll
                // prefetch, keep the old pulse loop alive so the hint is still
                // breathing when that distant match enters the user's view.
                let key = "\(line.lineNo):\(sp.start)"
                let targetVisible = (scrollView?.documentVisibleRect ?? visibleRect)
                    .intersects(rowRect)
                let expired = CACurrentMediaTime() - bracketShownAt >= Self.bracketFlashDuration
                if key != bracketKey || (!targetVisible && expired) {
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
                // The fade timer repaints exactly this, and nothing else.
                bracketRects.append(rowRect)
                // Grow about the centre so the pop does not drift sideways.
                let box = base.insetBy(
                    dx: -base.width * (scale - 1) / 2,
                    dy: -base.height * (scale - 1) / 2
                )
                colors.bracketFill.withAlphaComponent(fade).setFill()
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
                            .font: EditorMetrics.monospaced(fontSize, weight: .bold),
                            .foregroundColor: colors.bracketInk
                                .withAlphaComponent(fade),
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
        PerfProbe.record(
            "   draw: row loop",
            Double(DispatchTime.now().uptimeNanoseconds - rowLoopStart) / 1_000_000
        )

        // Do not draw a full rectangular focus ring around a pane. Split
        // dividers already define its bounds, while the focused pane header
        // carries an accent rule, icon tint and semibold title. The additional
        // blue rectangle made the editor read like a selected web card and
        // doubled into a thick band where it met a divider.
        _ = showFocusRing
    }

    /// Bookmark-style breakpoint marker in the gutter (SF Symbol, accent tint).

    private func ctLine(for line: EditorLine, font: NSFont) -> CTLine {
        let key = PerfProbe.measure("    cacheKey") { cacheKey(for: line) }
        if let cached = ctCache[key] {
            PerfProbe.record("    ctLine HIT", 0)
            return cached
        }
        PerfProbe.record("    ctLine MISS", 0)
        let attr = attributedLine(line, font: font)
        let ct = CTLineCreateWithAttributedString(attr)
        ctCache[key] = ct
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
        for sp in line.spans where sp.kind < 248 {
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

    /// Visual column → UTF-16 offset, memoized on the line's text.
    ///
    /// The core reports spans in visual columns (tab-expanded, CJK counted as
    /// two cells); CoreText wants UTF-16 offsets. This is the bridge, and the
    /// draw needs it for every visible line.
    ///
    /// It used to allocate a Swift `String` PER CHARACTER — `ns.substring(with:
    /// r)`, purely to look at that character's first scalar — and it ran twice
    /// per line per repaint. On a 60-row viewport of 100-column code that is
    /// ~12,000 string allocations for a single frame.
    private func visualToUTF16Map(_ s: String) -> [Int] {
        var h = Hasher()
        h.combine(s)
        let key = UInt64(bitPattern: Int64(h.finalize()))
        if let cached = vmapCache[key] { return cached }
        let map = buildVisualToUTF16Map(s)
        vmapCache[key] = map
        return map
    }

    private func buildVisualToUTF16Map(_ s: String) -> [Int] {
        let ns = s as NSString
        var map: [Int] = []
        map.reserveCapacity(ns.length + 1)
        var i = 0
        var col = 0
        while i < ns.length {
            while map.count <= col { map.append(i) }
            let r = ns.rangeOfComposedCharacterSequence(at: i)
            // Width decided from the sequence's FIRST UTF-16 unit, which is
            // exactly what the substring version asked for — and identical for
            // astral characters too: every high surrogate (0xD800…0xDBFF) is
            // above the 0x2E80 wide threshold, so an emoji still measures 2.
            let unit = ns.character(at: i)
            let w: Int
            if unit == 0x09 {
                w = 4 - (col % 4)
            } else if unit > 0x2E80 {
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
        // 7-14 used to fall through to plain foreground: the tokenizer
        // classified macros, namespaces, properties and the rest, the engine
        // sent the kind, the theme had a colour for each — and the face painted
        // them all as body text.
        case 7: return colors.macroName
        case 8: return colors.namespace
        case 9: return colors.parameter
        case 10: return colors.property
        case 11: return colors.constant
        case 13: return colors.operatorColor
        case 14: return colors.punctuation
        default: return colors.fg
        }
    }

    /// Every change bar in `band`, as coloured rects.
    ///
    /// A SEPARATE pass, because a bar is a hunk-shaped object and the row loop
    /// is row-shaped. Drawing it per row put a cap on every line, so a run of
    /// changed lines came out as a column of little boxes rather than one
    /// change — and both row loops skip rows outside the dirty rect, so a run
    /// cannot be accumulated inside them anyway.
    ///
    /// Rects because that is the only primitive BOTH renderers have. A stroked
    /// path would have to exist twice, once per renderer, and could then
    /// differ; from one rect list they cannot.
    /// One device pixel in points. The change bars' rounded ends are built
    /// column by column and have to land on the pixel grid — see `capRects`.
    private var devicePixel: CGFloat {
        1 / (window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2)
    }

    private func gitBars<Band: Sequence<EditorLine>>(
        _ band: Band, lineH: CGFloat
    ) -> [(CGRect, NSColor)] {
        var out: [(CGRect, NSColor)] = []
        var runStart: CGFloat?
        var runEnd: CGFloat = 0
        var runTopCap = false
        var runKind: UInt8 = 0
        var runStaged = false
        var lastRow = -2

        func flush(bottomCap: Bool) {
            guard let start = runStart else { return }
            let color = gitColor(runKind)
            // A deletion is a pill like any other, one line tall — the row the
            // removed text used to sit above. It was briefly a small wedge, on
            // the reasoning that a deletion occupies no line of its own; but a
            // wedge answers nothing about staging and reads as neither an
            // addition nor a change. One shape, one rule.
            // Hovered when the run lies inside the hunk the pointer is over.
            // Compared by rows rather than by identity because a hunk can be
            // split into several runs here — `signs_from_hunks` marks the
            // modified head and the added tail of one change differently.
            let target = hoveredHunk ?? fadingHunk
            let inHovered = target.map { h in
                start >= visualY(h.first) - 0.5
                    && runEnd <= (visualY(h.last) + lineH) + 0.5
            } ?? false
            for r in Self.barRects(
                top: start, bottom: runEnd,
                topCap: runTopCap, bottomCap: bottomCap,
                staged: runStaged, grown: inHovered ? hoverEase : 0,
                pixel: devicePixel
            ) {
                out.append((r, color))
            }
            runStart = nil
        }

        for line in band where !line.isWrapContinuation {
            let kind = line.gitSignKind
            let row = max(0, Int(line.lineNo) - 1)
            guard kind != 0 else {
                flush(bottomCap: false)
                lastRow = row
                continue
            }
            let y = visualY(row)
            // A run is a HUNK, not a colour.
            //
            // It used to break wherever the sign kind changed, and one hunk
            // can carry two: `signs_from_hunks` overrides the last row of a
            // hunk that removed more lines than it added to Deleted, so a
            // six-lines-into-two edit comes out `Modified, Deleted`. That
            // split one change into two bars with a flat butt-joint between
            // them — one capsule's round end, a seam, then another's — which
            // reads as the bar being cut in half.
            //
            // Only a new hunk starts a new run now. Core already marks that
            // row, and it is the only boundary that means anything to a reader.
            let sameRun = runStart != nil
                && row == lastRow + 1
                && !line.gitHunkFirst
                && line.gitHunkStaged == runStaged
            if !sameRun {
                flush(bottomCap: false)
                // A run that begins where a change is expanded starts at the
                // TOP of the opened block: the removed lines are part of the
                // same change and the bar has to say so.
                //
                // Only the START moves. Lifting `y` itself and then taking
                // `runEnd = y + lineH` from it put the run's bottom a block
                // above the row it belongs to, so the bar came out one line
                // tall at the top of the block — "hunk가 한줄밖에 안떠".
                var top = y
                if let e = shownChange, row == e.insertAt {
                    top -= insertedHeight(above: row)
                }
                runStart = top
                runTopCap = line.gitHunkFirst
                runKind = kind
                runStaged = line.gitHunkStaged
            } else {
                // The strongest kind in the run wins, and the kinds are
                // numbered in that order: added, modified, deleted. Core's own
                // rule, said once more here — "a hunk that removed more lines
                // than it added is a DELETION as far as the reader is
                // concerned, whatever else it also did". One change, one
                // colour.
                runKind = max(runKind, kind)
            }
            runEnd = y + lineH
            lastRow = row
            if line.gitHunkLast { flush(bottomCap: true) }
        }
        // A hunk running past the bottom of the band gets no cap there, which
        // is right: its end is off screen.
        flush(bottomCap: false)
        return out
    }

    /// A horizontal slice of a bar, `hw` wide either side of the centre.
    ///
    /// The ONE place hollow-vs-filled is decided, so every shape the gutter
    /// draws answers staging the same way. A slice thinner than the stroke has
    /// no inside to leave open and is drawn solid whatever the state.
    private static func barSlice(
        y: CGFloat, height: CGFloat, halfWidth hw: CGFloat, staged: Bool
    ) -> [CGRect] {
        guard height > 0, hw > 0 else { return [] }
        // Centre is FIXED, so a hovered bar grows about its own axis instead of
        // sliding sideways as it thickens.
        let cx = gitBarCentreX
        let t = EditorCanvasView.gitBarStroke
        if staged || hw <= t {
            return [CGRect(x: cx - hw, y: y, width: hw * 2, height: height)]
        }
        return [
            CGRect(x: cx - hw, y: y, width: t, height: height),
            CGRect(x: cx + hw - t, y: y, width: t, height: height),
        ]
    }

    /// One bar: a capsule, hollow while the hunk is unstaged and solid once it
    /// is staged.
    ///
    /// A cap is drawn only where the hunk actually ends. Where the run leaves
    /// the visible band the edge is left open, so a long change reads as
    /// continuing rather than as stopping at the viewport.
    private static func barRects(
        top: CGFloat, bottom: CGFloat,
        topCap: Bool, bottomCap: Bool, staged: Bool, grown: CGFloat = 0,
        pixel: CGFloat = 0.5
    ) -> [CGRect] {
        // A hovered hunk thickens. Xcode does this, and it is the affordance
        // that says the bar is a control and not a decoration — the pointer is
        // already in its column, about to press it. `grown` is 0…1 so the swell
        // is animated rather than stepped.
        let ease = max(0, min(1, grown))
        let w = EditorMetrics.gitStripeWidth
            + EditorCanvasView.gitBarHoverGrowth * ease
        let r = w / 2   // a true capsule

        // The swell is UNIFORM — the outline moves outward by the same amount
        // in every direction, which is what a shape growing looks like.
        // Widening alone reads as a sideways stretch: the ends stay on exactly
        // the rows they sat on, so only the middle appears to move.
        //
        // The vertical half is an inset that OPENS rather than an overhang
        // that grows. Bulging past the hunk's rows made the bar say something
        // false about which lines changed, and it disagreed with the hover
        // wash, which is exactly those rows — visible as soon as staging fills
        // the bar in and the overhang is solid ink past the highlighted band.
        // So the resting bar is held a little inside its rows and hover
        // returns it to them: the same 1pt of travel per end, anchored to the
        // truth instead of past it.
        //
        // Only a capped end moves. An end that runs off the band has no end to
        // inset; the bar simply continues.
        let inset = EditorCanvasView.gitBarHoverGrowth / 2 * (1 - ease)
        let top = topCap ? top + inset : top
        let bottom = bottomCap ? bottom - inset : bottom

        var out: [CGRect] = []
        let bodyTop = top + (topCap ? r : 0)
        let bodyBottom = bottom - (bottomCap ? r : 0)
        out += barSlice(
            y: bodyTop, height: bodyBottom - bodyTop, halfWidth: r, staged: staged
        )

        if topCap {
            out += capRects(centreY: top + r, r: r, up: true, staged: staged, pixel: pixel)
        }
        if bottomCap {
            out += capRects(
                centreY: bottom - r, r: r, up: false, staged: staged, pixel: pixel
            )
        }
        return out
    }

    /// One rounded end, sliced into COLUMNS.
    ///
    /// It was sliced into rows, and that is why a hovered bar looked squared
    /// off: the topmost row of a dome spans the chord at that depth, so the
    /// apex came out as one flat horizontal run — the two side strokes meeting
    /// in a straight line — and the run got longer as the bar thickened,
    /// because the chord grows with the radius. Round, and reading as square.
    ///
    /// A dome is steep in x at the apex and shallow at the shoulders, so
    /// columns put the unavoidable flat parts on the shoulders, where the
    /// curve is nearly flat anyway, and let the apex taper.
    ///
    /// Rects rather than a path because the Metal renderer takes nothing else,
    /// and both renderers have to agree about the shape.
    /// One rounded end, sliced into COLUMNS on the pixel grid.
    ///
    /// It was sliced into rows, and that is why a hovered bar looked squared
    /// off: the topmost row of a dome spans the chord at that depth, so the
    /// apex came out as one flat horizontal run — the two side strokes meeting
    /// in a straight line — and the run got longer as the bar thickened.
    ///
    /// A dome is steep in x at the apex and shallow at the shoulders, so
    /// columns put the unavoidable flat parts on the shoulders, where the
    /// curve is nearly flat anyway, and let the apex taper.
    ///
    /// Rects rather than a path because the Metal renderer takes nothing else,
    /// and both renderers have to agree about the shape.
    private static func capRects(
        centreY cy: CGFloat, r: CGFloat, up: Bool, staged: Bool, pixel: CGFloat
    ) -> [CGRect] {
        let cx = gitBarCentreX
        let t = EditorCanvasView.gitBarStroke
        // One column per DEVICE PIXEL, on the pixel grid.
        //
        // The columns were a fixed 0.25pt, which at 2x is half a pixel: every
        // pixel in the cap was shared by two of them, and two partial
        // coverages of an opaque colour do not composite to full — so the caps
        // came out uniformly paler than the body, and the bar looked like it
        // changed colour at each end. Landing every boundary on a pixel edge
        // means no horizontal coverage is ever split. The curve is carried by
        // the column HEIGHTS, which stay fractional and antialias as they
        // should.
        let px = max(0.1, pixel)
        var x = ((cx - r) / px).rounded() * px
        let end = cx + r
        var out: [CGRect] = []
        while x < end - 0.0001 {
            let w = min(px, end - x)
            let dx = x + w / 2 - cx
            let outer = (max(0, r * r - dx * dx)).squareRoot()
            // The hollow's inner edge is the same arc, one stroke smaller. Past
            // where that circle ends, the stroke is all there is and the column
            // runs to the cap's base — which is what closes the tip.
            let inner = r - t
            let hole = abs(dx) < inner ? (inner * inner - dx * dx).squareRoot() : 0
            let depth = staged ? outer : outer - hole
            guard depth > 0.01 else { x += w; continue }
            out.append(CGRect(
                x: x, y: up ? cy - outer : cy + outer - depth,
                width: w, height: depth
            ))
            x += w
        }
        return out
    }

    /// The bar's axis. Fixed, from the RESTING width, so a hover thickens it
    /// about its own centre instead of sliding it sideways.
    private static var gitBarCentreX: CGFloat {
        EditorCanvasView.gitBarX + EditorMetrics.gitStripeWidth / 2
    }

    /// The menu a change bar opens.
    ///
    /// Built where it is used, through a block-holding target, rather than as a
    /// bridge with one `@objc` per entry. Held on the view for as long as it
    /// can be open — a released target silently disables every item.
    private var hunkMenuTarget: TabStripMenuTarget?

    private func showHunkMenu(
        _ h: (first: Int, last: Int, staged: Bool, kind: UInt8), at p: CGPoint
    ) {
        guard let engine else { return }
        let line = UInt32(h.first + 1)
        let target = TabStripMenuTarget()
        hunkMenuTarget = target
        let menu = NSMenu()

        // "Show Change" first, as in the screenshot, and only where there IS
        // something to show — a pure addition replaced nothing.
        if let removed = engine.removedTextForHunk(atLine: line), !removed.isEmpty {
            let showing = shownChange?.insertAt == h.first
            menu.addItem(target.item(
                showing ? "Hide Change" : "Show Change",
                symbol: showing ? "eye.slash" : "eye",
                key: "\r", modifiers: [.command]
            ) { [weak self] in
                self?.toggleShownChange(h)
            })
            menu.addItem(.separator())
        }

        // Staged and unstaged are opposites, so only one of them is ever an
        // action: offering both means one of them is always a no-op, and a
        // menu item that reports "Not staged" is a worse answer than an item
        // that was not there.
        if h.staged {
            menu.addItem(target.item(
                "Unstage Change", symbol: "minus.circle"
            ) { [weak engine] in
                engine?.applyGutterHunk(line1based: line, action: 1)
            })
        } else {
            menu.addItem(target.item(
                "Stage Change", symbol: "plus.circle"
            ) { [weak engine] in
                engine?.applyGutterHunk(line1based: line, action: 0)
            })
        }

        menu.addItem(target.item(
            "Discard Change", symbol: "arrow.uturn.backward"
        ) { [weak engine] in
            engine?.applyGutterHunk(line1based: line, action: 2)
        })

        // What the change replaced, on the clipboard. Those lines are in no
        // buffer and cannot be selected, so without this there is no way to
        // get them back out of the gutter.
        if let removed = engine.removedTextForHunk(atLine: line),
           !removed.isEmpty
        {
            menu.addItem(.separator())
            menu.addItem(target.item(
                "Copy Original", symbol: "doc.on.doc"
            ) {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(removed, forType: .string)
            })
        }

        // To the LEFT of the bar, at the pressed row.
        //
        // `popUp(positioning:at:in:)` puts the menu's LEADING edge at the
        // point, so anchoring at the bar sent it rightwards across the code it
        // is describing. Subtracting its own width puts its trailing edge on
        // the bar instead, which is where the screenshot has it and which
        // leaves the change itself visible while the menu is open. AppKit still
        // nudges it back on screen if the window is against the left edge.
        menu.popUp(
            positioning: nil,
            at: CGPoint(
                x: Self.gitBarX - menu.size.width - 4,
                y: p.y.rounded()
            ),
            in: self
        )
    }

    // MARK: - Breakpoint chip: appearance

    /// Rows whose chip is mid-transition, and when it started.
    ///
    /// Keyed by line number rather than by index, so scrolling does not
    /// reassign an animation to a different line.
    private var bpAnim: [UInt32: (start: CFTimeInterval, appearing: Bool)] = [:]
    /// What the last band said, so a flip can be told from a first sighting.
    private var bpSeen: [UInt32: Bool] = [:]
    private var bpAnimTimer: Timer?
    private var liveFlashTimer: Timer?

    /// Notice chips arriving and leaving in this band.
    ///
    /// A row scrolled into view for the first time is NOT an arrival — it was
    /// already there. Only a row this canvas has seen before and whose state
    /// flipped gets an animation; otherwise every scroll would replay every
    /// chip on screen.
    /// Keep repainting while any row is flashing.
    ///
    /// Which rows, and when each was first seen, belong to `LiveMarks` — the
    /// same list the minimap reads, because the minimap has to show changes
    /// that are off screen and a per-row flag can only describe the band. This
    /// view only has to keep drawing while the fade runs.
    private func syncLiveFlashes() {
        syncLiveShift()
        guard engine?.live.isFlashing == true else { return }
        startLiveFlashTimer()
    }

    /// 0 once the flash is over.
    private func liveFlash(_ lineNo: UInt32) -> CGFloat {
        engine?.live.intensity(lineNo) ?? 0
    }

    /// Blue arrived, red left. The same two colours the gutter already uses
    /// for the same two facts, so a reader does not have to learn a second
    /// vocabulary for changes they did not make.
    private func liveFlashColor(_ lineNo: UInt32) -> NSColor {
        switch engine?.live.rows[lineNo] {
        case .added: return colors.gitChange.withAlphaComponent(0.26)
        case .removed: return colors.gitDelete.withAlphaComponent(0.26)
        default: return colors.liveFlash
        }
    }

    private func startLiveFlashTimer() {
        guard liveFlashTimer == nil else { return }
        let t = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] timer in
            guard let self else { timer.invalidate(); return }
            self.needsDisplay = true
            if self.engine?.live.isFlashing != true {
                timer.invalidate()
                self.liveFlashTimer = nil
            }
        }
        RunLoop.main.add(t, forMode: .common)
        liveFlashTimer = t
    }

    private func syncBreakpointAnimations<Band: Sequence<EditorLine>>(_ band: Band) {
        let now = CACurrentMediaTime()
        var started = false
        for line in band where !line.isWrapContinuation {
            let on = line.hasBreakpoint
            if let was = bpSeen[line.lineNo], was != on {
                bpAnim[line.lineNo] = (now, on)
                started = true
            }
            bpSeen[line.lineNo] = on
        }
        bpAnim = bpAnim.filter { now - $0.value.start < Self.bpAnimDuration }
        if started { startBreakpointAnimation() }
    }

    /// 0 = no chip, 1 = fully drawn. Eased, and derived from the clock rather
    /// than stepped, so a dropped frame costs a frame and not a wrong size.
    private func breakpointPhase(_ line: EditorLine) -> CGFloat {
        // A breakpoint belongs to the LINE, and the line's chip is on the row
        // that carries its number. `bpAnim` is keyed by `lineNo`, which every
        // segment of a wrapped line shares — so while the animation ran, all
        // of them drew a chip and the numberless rows below appeared to blink
        // one into existence too. Core already withholds `hasBreakpoint` from
        // continuations; this is the same rule for the animating case.
        if line.isWrapContinuation { return 0 }
        guard let a = bpAnim[line.lineNo] else {
            return line.hasBreakpoint ? 1 : 0
        }
        let t = min(1, max(0, (CACurrentMediaTime() - a.start) / Self.bpAnimDuration))
        let eased = CGFloat(1 - pow(1 - t, 3))
        return a.appearing ? eased : 1 - eased
    }

    private func startBreakpointAnimation() {
        guard bpAnimTimer == nil else { return }
        let t = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] timer in
            guard let self else { timer.invalidate(); return }
            self.needsDisplay = true
            if self.bpAnim.isEmpty {
                timer.invalidate()
                self.bpAnimTimer = nil
                self.needsDisplay = true
            }
        }
        RunLoop.main.add(t, forMode: .common)
        bpAnimTimer = t
    }

    static let bpAnimDuration: CFTimeInterval = 0.16

    /// The breakpoint chip: a rounded rect wrapping the LINE NUMBER.
    ///
    /// Breakpoints used to be a bookmark glyph at x=4, inside the change bar's
    /// own column — so a click meant for a hunk toggled a breakpoint, and the
    /// two markers sat on top of each other. They are separate targets now:
    /// the bar is pressed for its hunk, the number is pressed for its
    /// breakpoint, and neither can be hit by accident.
    /// The band is over the TEXT, not over the gutter.
    ///
    /// Washing the whole row put a green tint under the line number, the change
    /// bar and the amber breakpoint chip — three colours inside fifteen points,
    /// none of them legible, which is most of what read as "UI가 좀 구린디".
    /// The gutter already has its own vocabulary; the band belongs to the code.
    static func stopBandRect(_ row: CGRect) -> CGRect {
        // Starts at the code, which is now also where the arrow stops — the
        // band must not wash over the mark that points into it.
        let left = max(row.minX, EditorMetrics.gutter)
        return CGRect(x: left, y: row.minY, width: max(0, row.maxX - left), height: row.height)
    }

    /// The instruction pointer: a right-pointing triangle in the air gap
    /// between the line number and the code.
    ///
    /// A SHAPE and not a colour, deliberately. The gutter already spends its
    /// colours on git — added, deleted, staged — so a debugger arguing in hue
    /// would be a fourth meaning for the same pixels, and colour alone is what
    /// Increase Contrast and a colour-blind reader cannot use.
    ///
    /// **In the text gap, not in the change bar's lane.** It started at
    /// `gitBarX`, which is exactly where the git stripe is drawn, so a line
    /// that was both modified and stopped had the arrow sitting on top of its
    /// hunk bar — two marks fighting for six points. `gutterTextGap` is the
    /// twelve points of air the layout already leaves between the number and
    /// the code, it belongs to nothing else, and an arrow there points AT the
    /// line rather than merely being beside it.
    static func stopArrowFrame(atY y: CGFloat, lineH: CGFloat) -> CGRect {
        let h = min(lineH - 4, 10)
        let w = h * 0.7
        // Right-aligned in the gap, two points clear of the text.
        let right = EditorMetrics.gutter - 2
        return CGRect(
            x: right - w, y: (y + (lineH - h) / 2).rounded(),
            width: w, height: h
        )
    }

    static func stopArrowPath(atY y: CGFloat, lineH: CGFloat) -> NSBezierPath {
        let f = stopArrowFrame(atY: y, lineH: lineH)
        let path = NSBezierPath()
        path.move(to: CGPoint(x: f.minX, y: f.minY))
        path.line(to: CGPoint(x: f.maxX, y: f.midY))
        path.line(to: CGPoint(x: f.minX, y: f.maxY))
        path.close()
        return path
    }

    /// The hollow arrow, for the Metal path: the outline as four thin bars.
    ///
    /// A stroked triangle is not available where only rectangles are, so the
    /// outline is drawn as its edges — a left bar for the back, and a stepped
    /// diagonal for each of the two sides.
    static func frameArrowBars(atY y: CGFloat, lineH: CGFloat) -> [CGRect] {
        let f = stopArrowFrame(atY: y, lineH: lineH)
        let t: CGFloat = 1.5
        var bars: [CGRect] = [
            CGRect(x: f.minX, y: f.minY, width: t, height: f.height)
        ]
        let steps = max(3, Int(f.height / 2))
        let dy = f.height / CGFloat(steps * 2)
        let dx = f.width / CGFloat(steps)
        for i in 0..<steps {
            let x = f.minX + dx * CGFloat(i)
            bars.append(CGRect(x: x, y: f.minY + dy * CGFloat(i), width: dx + t, height: t))
            bars.append(CGRect(x: x, y: f.maxY - dy * CGFloat(i) - t, width: dx + t, height: t))
        }
        return bars
    }

    /// The same arrow for the Metal path, which has rectangles and nothing
    /// else. Horizontal bars of shrinking width — at eleven points that is six
    /// of them, and the staircase reads as a triangle at any sane font size.
    static func stopArrowBars(atY y: CGFloat, lineH: CGFloat) -> [CGRect] {
        let f = stopArrowFrame(atY: y, lineH: lineH)
        let steps = max(3, Int(f.height / 2))
        let step = f.height / CGFloat(steps * 2)
        var bars: [CGRect] = []
        bars.reserveCapacity(steps)
        for i in 0..<steps {
            let inset = step * CGFloat(i)
            let width = f.width * (1 - CGFloat(i) / CGFloat(steps))
            bars.append(CGRect(
                x: f.minX, y: f.minY + inset,
                width: max(1, width), height: max(1, f.height - inset * 2)
            ))
        }
        return bars
    }

    static func breakpointChip(
        atY y: CGFloat, lineH: CGFloat, numberWidth: CGFloat,
        phase: CGFloat = 1
    ) -> CGRect {
        let right = EditorMetrics.gutter - EditorMetrics.gutterTextGap
        let pad = breakpointChipPadding
        let left = max(gitBarZone, right - numberWidth - pad)
        let h = min(lineH - 2, EditorMetrics.lineHeight - 2)
        let full = CGRect(
            x: left, y: (y + (lineH - h) / 2).rounded(),
            width: max(0, right + pad - left), height: h
        )
        guard phase < 1 else { return full }
        // Grows out of its own centre. A chip that simply appears at full size
        // reads as a glitch — "너무 뙄 생김".
        let k = 0.55 + 0.45 * max(0, min(1, phase))
        return full.insetBy(
            dx: full.width * (1 - k) / 2, dy: full.height * (1 - k) / 2
        )
    }

    static let breakpointChipPadding: CGFloat = 4
    static let breakpointChipRadius: CGFloat = 4

    /// Leading inset of the change bar inside the gutter.
    static let gitBarX: CGFloat = 2
    /// Outline thickness. Enough to read at a glance without closing the gap
    /// it exists to show.
    static let gitBarStroke: CGFloat = 1.5
    /// Vertical resolution of the rounded ends.
    static let gitBarCapStep: CGFloat = 0.25
    /// How much wider a hovered hunk's bar is.
    static let gitBarHoverGrowth: CGFloat = 2
    /// The rules closing the top and bottom of a hovered hunk.
    static let gitHoverRule: CGFloat = 1

    /// Additions and modifications share one colour, as Xcode's do: which of
    /// the two it is can be read off the text, and two colours down the gutter
    /// read as two kinds of warning.
    ///
    /// A deletion does not share it. Xcode distinguishes that one by SHAPE — a
    /// small triangle rather than a bar — and this gutter cannot, because a
    /// deletion is drawn as an ordinary one-line pill here. With the shape
    /// carrying nothing, colour is the only axis left, and a deletion is the
    /// one change with no evidence in the text at all: the pill IS the whole
    /// report. Blue made it indistinguishable from the line above it having
    /// been added.
    private func gitColor(_ kind: UInt8) -> NSColor {
        switch kind {
        case 1, 2: return colors.gitChange
        case 3: return colors.gitDelete
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

    // MARK: - Show Change: rows the buffer does not have

    /// The one change whose replaced text is on screen, if any.
    ///
    /// ONE at a time, deliberately. Several would make the row mapping a scan
    /// instead of a subtraction, and it is read on every hit test, every caret
    /// placement and every drawn row.
    private var shownChange: (insertAt: Int, lines: [String], staged: Bool)?
    /// When the reveal started, and whether it is closing.
    private var revealStart: CFTimeInterval = 0
    private var revealClosing = false
    private var revealTimer: Timer?

    static let revealDuration: CFTimeInterval = 0.20

    /// How far open the expanded change is, 0…1. Eased.
    ///
    /// Derived from the clock, like every other motion in this view: a dropped
    /// frame costs a frame and never a wrong height, which matters more here
    /// than anywhere else because this number also decides where every line
    /// below it is drawn AND hit-tested.
    private var revealProgress: CGFloat {
        guard shownChange != nil else { return 0 }
        let t = min(1, max(0, (CACurrentMediaTime() - revealStart) / Self.revealDuration))
        let eased = CGFloat(1 - pow(1 - t, 3))
        return revealClosing ? 1 - eased : eased
    }

    /// Points of inserted space above a buffer row.
    private func insertedHeight(above bufferRow: Int) -> CGFloat {
        var h: CGFloat = 0
        if let e = shownChange, bufferRow >= e.insertAt {
            h += CGFloat(e.lines.count) * EditorMetrics.lineHeight * revealProgress
        }
        // A live insertion still opening. NEGATIVE: the rows below are drawn
        // above their final places and settle down into them, so the document
        // appears to make room rather than to have always had it.
        //
        // Composed here rather than as a second offset elsewhere, because
        // `visualY` and `bufferRow(atY:)` both go through this one function
        // and that is the entire reason the reveal has never disagreed with a
        // hit test. A second place that shifts rows would be a second place to
        // keep in step.
        if let o = liveOpening, bufferRow >= o.below {
            // Signed: negative while lines arrive (the rows below start high
            // and settle down), positive while they leave (the rows below
            // start low and rise into the space). One term, both directions.
            h -= CGFloat(o.rows) * EditorMetrics.lineHeight * (1 - liveOpenProgress)
        }
        return h
    }

    /// The live document's shift, if one is running and this pane is showing
    /// that document.
    ///
    /// Read from `LiveMarks`, not stored: the value belongs to the reload, not
    /// to a view, and every pane has to agree about it. Gated on focus because
    /// the row numbers are the LIVE document's — applying them to a pane
    /// showing another file would slide the wrong text.
    private var liveOpening: (below: Int, rows: Int)? {
        guard let live = engine?.live, let s = live.shift else { return nil }
        if let split = engine?.editorSplit, split.isSplit, paneIndex != split.focus {
            return nil
        }
        return (below: s.below, rows: s.rows)
    }

    private var liveOpenProgress: CGFloat {
        engine?.live.shiftProgress() ?? 1
    }

    private var liveOpenTimer: Timer?

    /// Keep drawing while a shift runs. No state of its own — the value and
    /// the clock are both `LiveMarks`'s.
    private func syncLiveShift() {
        guard engine?.live.isShifting == true, liveOpenTimer == nil else { return }
        scrollView?.refitCanvas()
        let t = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] timer in
            guard let self else { timer.invalidate(); return }
            self.scrollView?.refitCanvas()
            self.needsDisplay = true
            if self.engine?.live.isShifting != true {
                timer.invalidate()
                self.liveOpenTimer = nil
                self.needsDisplay = true
            }
        }
        RunLoop.main.add(t, forMode: .common)
        liveOpenTimer = t
    }

    /// Where a buffer row is drawn.
    ///
    /// THE mapping, in POINTS rather than in row indices — the reveal animates,
    /// so during it the offset is a fraction of a row and an integer mapping
    /// could not express where anything is. Everything that turns a row into a
    /// y goes through this, and everything that turns a y back into a row goes
    /// through `bufferRow(atY:)`. With nothing expanded both are the identity.
    func visualY(_ bufferRow: Int) -> CGFloat {
        CGFloat(visualRowOf(bufferRow)) * EditorMetrics.lineHeight
            + insertedHeight(above: bufferRow)
    }

    /// First VISUAL row of a buffer row — the identity when not wrapping.
    ///
    /// This is the whole of the row-model change. `visualY` multiplied the
    /// buffer row by the line height, which is only its position while one
    /// buffer row is one screen row; with wrapping the answer is a running
    /// total over every line above, which core keeps as a prefix sum.
    func visualRowOf(_ bufferRow: Int) -> Int {
        guard wrapCols > 0, let engine else { return max(0, bufferRow) }
        return engine.wrapVisualOf(pane: paneIndex, cols: wrapCols, row: max(0, bufferRow))
    }

    /// Total visual rows — the document's height in rows.
    func totalVisualRows() -> Int {
        guard wrapCols > 0, let engine else { return max(1, Int(docLineCount)) }
        return max(1, engine.wrapTotalRows(pane: paneIndex, cols: wrapCols))
    }

    /// The buffer row drawn at a y, and nil inside the inserted block — those
    /// rows belong to no line and must never resolve to one, or a click on
    /// deleted text would move the caret into live code.
    func bufferRow(atY y: CGFloat) -> Int? {
        let lineH = max(1, EditorMetrics.lineHeight)
        guard let e = shownChange, revealProgress > 0 else {
            return bufferRowOfVisual(Int(floor(y / lineH)))
        }
        let top = visualY(e.insertAt)
        let openH = CGFloat(e.lines.count) * lineH * revealProgress
        if y < top { return bufferRowOfVisual(Int(floor(y / lineH))) }
        if y < top + openH { return nil }
        return bufferRowOfVisual(Int(floor((y - openH) / lineH)))
    }

    /// The buffer row a visual row belongs to — the identity when not wrapping.
    func bufferRowOfVisual(_ visualRow: Int) -> Int {
        let v = max(0, visualRow)
        guard wrapCols > 0, let engine else { return v }
        return engine.wrapBufferAt(pane: paneIndex, cols: wrapCols, visualRow: v).row
    }

    /// Which segment of a line the caret sits on, from the band core marked.
    /// 0 when not wrapping, or when the band does not carry the caret.
    func caretSegment(row: Int) -> Int {
        guard wrapCols > 0 else { return 0 }
        let ofRow = rows(row, row).filter { Int($0.lineNo) - 1 == row }
        return ofRow.firstIndex(where: \.isCursor) ?? 0
    }

    /// Which segment of its line is drawn at a visual row. 0 when not wrapping.
    func segmentOfVisual(_ visualRow: Int) -> Int {
        guard wrapCols > 0, let engine else { return 0 }
        return engine.wrapBufferAt(
            pane: paneIndex, cols: wrapCols, visualRow: max(0, visualRow)
        ).segment
    }

    /// Nearest buffer row to a y — for gestures that must land somewhere even
    /// when they start inside the inserted block.
    func nearestBufferRow(atY y: CGFloat) -> Int {
        if let r = bufferRow(atY: y) { return r }
        return shownChange?.insertAt ?? 0
    }

    /// The inserted block's rect, or nil when nothing is open.
    private func revealRect() -> CGRect? {
        guard let e = shownChange else { return nil }
        let p = revealProgress
        guard p > 0.001 else { return nil }
        let lineH = EditorMetrics.lineHeight
        return CGRect(
            x: 0, y: CGFloat(e.insertAt) * lineH,
            width: bounds.width,
            height: CGFloat(e.lines.count) * lineH * p
        )
    }

    /// Rows on screen that no buffer line owns. Read by the scroll view when it
    /// sizes the document, which is the other place row count means height.
    /// Rows on screen that no buffer line owns.
    ///
    /// The reveal's phantom rows, plus — while a REMOVAL is closing — the rows
    /// that are gone. The document has already shrunk by then, but the rows
    /// below are still being drawn that far down as they rise into the space,
    /// and a canvas sized to the new content clips them away. That is why a
    /// deletion had no animation and simply vanished: the motion was drawn
    /// outside the view.
    ///
    /// Insertions need nothing here — the document grew first, so the room is
    /// already there.
    var extraVisualRows: Int {
        var n = shownChange?.lines.count ?? 0
        if let o = liveOpening, o.rows < 0, liveOpenProgress < 1 {
            n += -o.rows
        }
        return n
    }

    /// Drop the reveal when the change it describes has stopped existing.
    ///
    /// `shownChange` is a claim about the buffer: N rows that HEAD has here and
    /// this document does not. Discarding that change makes the claim false in
    /// the worst possible way — HEAD's lines become the buffer's lines, so the
    /// reveal goes on inserting a phantom copy of text that is now really
    /// there. The leftover rows are the visible half of it; the other half is
    /// that `visualY` and `bufferRow(atY:)` stay shifted by N, so every row
    /// below the discarded change is drawn and clicked N rows out of place.
    ///
    /// Closing it from the Discard menu item would have fixed the reported
    /// case and left the rest: a discard on a hunk ABOVE this one, an edit that
    /// merges it away, an undo, or a tab switch — after which the reveal would
    /// show one file's deleted lines inside another. So the check is on the
    /// claim, not on the actions that can break it.
    ///
    /// Verified rather than closed outright, because content advances on every
    /// keystroke and caret move and nearly all of those leave the change where
    /// it was. `hunk_at` is a scan of the gutter's own small Vec — no `git`
    /// process — and this runs only while something is revealed.
    private func closeRevealIfItsChangeIsGone() {
        guard let e = shownChange, !revealClosing else { return }
        let live = engine?.removedTextForHunk(atLine: UInt32(e.insertAt + 1))
        if live == e.lines.joined(separator: "\n") { return }
        // No closing animation: the rows it was showing are, in the discard
        // case, now real rows in the buffer at the same place. Shrinking a
        // phantom copy of them over the top reads as a glitch, where dropping
        // it is almost seamless.
        revealTimer?.invalidate()
        revealTimer = nil
        shownChange = nil
        revealClosing = false
        scrollView?.refitCanvas()
        needsDisplay = true
    }

    private func toggleShownChange(
        _ h: (first: Int, last: Int, staged: Bool, kind: UInt8)
    ) {
        if shownChange?.insertAt == h.first, !revealClosing {
            // Kept alive through the close so the block can shrink; cleared by
            // the timer once it has.
            revealClosing = true
            revealStart = CACurrentMediaTime()
            startRevealAnimation()
            return
        }
        guard let text = engine?.removedTextForHunk(atLine: UInt32(h.first + 1)),
              !text.isEmpty
        else { return }
        do {
            shownChange = (
                insertAt: h.first,
                lines: text.components(separatedBy: "\n"),
                staged: h.staged
            )
            revealClosing = false
            revealStart = CACurrentMediaTime()
        }
        startRevealAnimation()
    }

    private func startRevealAnimation() {
        revealTimer?.invalidate()
        scrollView?.refitCanvas()
        needsDisplay = true
        let t = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] timer in
            guard let self else { timer.invalidate(); return }
            // The document's height follows the reveal, or the scroller and the
            // content disagree about how far there is to go for its duration.
            self.scrollView?.refitCanvas()
            self.needsDisplay = true
            let done = self.revealClosing
                ? self.revealProgress <= 0
                : self.revealProgress >= 1
            guard done else { return }
            timer.invalidate()
            self.revealTimer = nil
            if self.revealClosing {
                self.shownChange = nil
                self.revealClosing = false
            }
            self.scrollView?.refitCanvas()
            self.needsDisplay = true
        }
        RunLoop.main.add(t, forMode: .common)
        revealTimer = t
    }

    /// The lines an expanded change replaced, in the rows made for them.
    ///
    /// Sits on a warm wash and reads as CODE — `fg`, the document's own ink, at
    /// the document's own x. It was `secondaryLabelColor` with a "−" in the
    /// gutter and a rule under it, which made a block of dimmed, marked-up text
    /// that looked like a diff pasted into the editor rather than like the file
    /// as it used to be. The screenshot has neither: the removed rows simply
    /// have no line number, and that absence is the marker.
    private func drawShownChange(lineH: CGFloat, font: NSFont, cg: CGContext) {
        guard let e = shownChange, let box = revealRect() else { return }
        let top = box.minY
        colors.removedBg.setFill()
        box.fill()

        // One hairline where the old text ends and the surviving text begins.
        // Nothing at the top: the code above it is genuinely what precedes it.
        colors.removedEdge.setFill()
        CGRect(x: 0, y: box.maxY - 1, width: bounds.width, height: 1).fill()

        let attrs: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: colors.removedFg,
        ]
        let markAttrs: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: colors.removedFg.withAlphaComponent(0.55),
        ]
        cg.saveGState()
        cg.clip(to: box)
        for (i, text) in e.lines.enumerated() {
            let y = top + CGFloat(i) * lineH
            let baseline = y + (lineH - font.ascender + font.descender) / 2
            // A "−" where the line number would be. These rows have no number
            // because they have no line, and the mark says which absence it is.
            let mark = NSAttributedString(string: "−", attributes: markAttrs)
            mark.draw(at: CGPoint(
                x: EditorMetrics.gutter - EditorMetrics.gutterTextGap
                    - mark.size().width,
                y: baseline
            ))
            NSAttributedString(string: text, attributes: attrs).draw(
                at: CGPoint(x: EditorMetrics.gutter, y: baseline)
            )
        }
        cg.restoreGState()
    }

    // MARK: - Gutter change bars: hover

    /// The hunk the pointer is over, in rows, or nil.
    ///
    /// Held rather than recomputed at paint time because the wash has to be
    /// drawn for rows the pointer is NOT on — a hunk is highlighted whole.
    private var hoveredHunk: (first: Int, last: Int, staged: Bool, kind: UInt8)?
    /// When the current hover began, for the grow-in. Zero while nothing is
    /// hovered; the fade-out runs from `hoverLeftAt` instead.
    private var hoverEnteredAt: CFTimeInterval = 0
    private var hoverLeftAt: CFTimeInterval = 0
    private var hoverAnimTimer: Timer?
    /// The hunk being faded out. Without it the region vanishes the instant the
    /// pointer leaves and only the bar animates, which reads as a glitch.
    private var fadingHunk: (first: Int, last: Int, staged: Bool, kind: UInt8)?

    /// How far into the gutter a press or a hover belongs to the change bar
    /// rather than to the breakpoint column behind it.
    static var gitBarZone: CGFloat {
        gitBarX + EditorMetrics.gitStripeWidth + 4
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for a in trackingAreas where a.owner === self { removeTrackingArea(a) }
        addTrackingArea(NSTrackingArea(
            rect: .zero,
            options: [
                .activeInKeyWindow, .mouseMoved, .mouseEnteredAndExited,
                .inVisibleRect,
            ],
            owner: self
        ))
    }

    override func mouseMoved(with event: NSEvent) {
        super.mouseMoved(with: event)
        let p = convert(event.locationInWindow, from: nil)
        updateHunkHover(p)
        updateDatatipHover(p)
    }

    override func mouseExited(with event: NSEvent) {
        super.mouseExited(with: event)
        setHoveredHunk(nil)
        cancelDatatip()
    }

    // MARK: - Datatips

    /// Point at a variable while stopped and it says what it is worth.
    ///
    /// Only while STOPPED: a running program has no frame to evaluate in, and
    /// a datatip over a program that is not paused would be asking a question
    /// that has no answer yet. Xcode behaves the same way, for the same reason.
    ///
    /// The dwell is what keeps this from being a nuisance. Moving the pointer
    /// across a line crosses a dozen identifiers, and a popover that opened on
    /// each of them would be a strobe — so nothing is asked until the pointer
    /// has been still on one word.
    private func updateDatatipHover(_ p: CGPoint) {
        guard let engine, engine.dap.state == .stopped else {
            cancelDatatip()
            return
        }
        // Gutter presses belong to the change bar and the breakpoint column.
        guard p.x > EditorMetrics.gutter else {
            cancelDatatip()
            return
        }
        let symbol = symbolUnderPointer(p)
        guard let symbol, !symbol.isEmpty else {
            cancelDatatip()
            return
        }
        // Already on this one: leave it alone. Re-asking on every mouse move
        // within a word would restart the popover under the pointer.
        if symbol == datatipSymbol { return }

        datatipAsk?.cancel()
        datatipWork?.cancel()
        datatipSymbol = symbol
        let point = p

        // ASK early, SHOW later — two clocks, because they are two different
        // decisions. Asking is invisible and cheap, so it can happen almost at
        // once; showing is the commitment and needs the dwell that stops a
        // pointer crossing a line from strobing popovers.
        //
        // With one clock the delays ADDED: the card opened after the dwell and
        // only then went to the adapter, so what the user waited through was
        // dwell + round-trip and it read as "팝오버가 넘 늦게뜸". Now the
        // answer is usually already in by the time the card appears.
        let ask = DispatchWorkItem { [weak self] in
            guard let self, let engine = self.engine, engine.dap.state == .stopped else { return }
            engine.requestDatatip(symbol)
        }
        datatipAsk = ask
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.datatipAskDelay, execute: ask)

        let work = DispatchWorkItem { [weak self] in
            self?.showDatatip(symbol, at: point)
        }
        datatipWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.datatipDwell, execute: work)
    }

    /// Long enough not to fire on a pointer that is merely passing through,
    /// short enough that the answer is in flight before the card is due.
    private static let datatipAskDelay: TimeInterval = 0.10
    /// When the card appears. Xcode's is about this; past ~0.4s a datatip stops
    /// feeling like an answer and starts feeling like a wait.
    private static let datatipDwell: TimeInterval = 0.28

    private func showDatatip(_ symbol: String, at p: CGPoint) {
        guard let engine, engine.dap.state == .stopped else { return }
        // Normally the ask already went out on the earlier clock; this covers
        // the case where it was cancelled or the state changed under it.
        if engine.datatip == nil, !engine.datatipPending {
            engine.requestDatatip(symbol)
        }

        let popover = NSPopover()
        popover.behavior = .transient
        popover.animates = false
        let host = NSHostingController(rootView: DatatipCard(engine: engine, symbol: symbol))
        // The card opens on the dwell and fills in when the adapter answers, so
        // its size is not known when it appears — the same reason Quick Help
        // asks for this.
        host.sizingOptions = [.preferredContentSize]
        popover.contentViewController = host
        let anchor = NSRect(x: p.x - 1, y: p.y - 1, width: 2, height: 2)
        datatipPopover?.close()
        datatipPopover = popover
        popover.show(relativeTo: anchor, of: self, preferredEdge: .maxY)
    }

    private func cancelDatatip() {
        datatipAsk?.cancel()
        datatipAsk = nil
        datatipWork?.cancel()
        datatipWork = nil
        guard datatipSymbol != nil else { return }
        datatipSymbol = nil
        datatipPopover?.close()
        datatipPopover = nil
        engine?.clearDatatip()
    }

    private func updateHunkHover(_ p: CGPoint) {
        guard p.x >= 0, p.x <= Self.gitBarZone else {
            setHoveredHunk(nil)
            return
        }
        let lineH = max(1, EditorMetrics.lineHeight)
        setHoveredHunk(hunkExtent(atRow: nearestBufferRow(atY: p.y)))
    }

    private func setHoveredHunk(
        _ next: (first: Int, last: Int, staged: Bool, kind: UInt8)?
    ) {
        let same = hoveredHunk?.first == next?.first
            && hoveredHunk?.last == next?.last
            && hoveredHunk?.staged == next?.staged
            && hoveredHunk?.kind == next?.kind
        guard !same else { return }
        let now = CACurrentMediaTime()
        if next != nil {
            // Moving straight from one hunk to another restarts the grow so
            // the new region arrives rather than inheriting the old one's age.
            hoverEnteredAt = now
            hoverLeftAt = 0
        } else {
            hoverLeftAt = now
        }
        if next == nil { fadingHunk = hoveredHunk } else { fadingHunk = nil }
        hoveredHunk = next
        toolTip = next.map { $0.staged ? "Staged change" : "Unstaged change" }
        startHoverAnimation()
    }

    /// 0 while resting, 1 while fully hovered. Eased.
    ///
    /// Derived from the clock rather than stepped into storage, for the same
    /// reason the tab strip's origin is: a dropped frame then costs a frame and
    /// not a wrong value, and the last frame of the fade lands exactly on 0
    /// whether or not a timer fired for it.
    private var hoverProgress: CGFloat {
        let now = CACurrentMediaTime()
        let d = Self.hoverGrowDuration
        if hoveredHunk != nil {
            return CGFloat(min(1, max(0, (now - hoverEnteredAt) / d)))
        }
        guard hoverLeftAt > 0 else { return 0 }
        return CGFloat(1 - min(1, max(0, (now - hoverLeftAt) / d)))
    }

    /// Eased, so the bar swells out and settles rather than ramping linearly.
    private var hoverEase: CGFloat {
        let t = hoverProgress
        return 1 - pow(1 - t, 3)
    }

    private func startHoverAnimation() {
        hoverAnimTimer?.invalidate()
        needsDisplay = true
        let t = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] timer in
            guard let self else { timer.invalidate(); return }
            self.needsDisplay = true
            let settled = self.hoveredHunk != nil
                ? self.hoverProgress >= 1
                : self.hoverProgress <= 0
            if settled {
                timer.invalidate()
                self.hoverAnimTimer = nil
                self.needsDisplay = true
            }
        }
        RunLoop.main.add(t, forMode: .common)
        hoverAnimTimer = t
    }

    static let hoverGrowDuration: CFTimeInterval = 0.13

    /// The rows of the hunk covering `row`, walking outward from it.
    ///
    /// Derived from the per-row hunk-boundary bits rather than from a hunk list
    /// the face keeps its own copy of — the scene already marks each run's
    /// first and last row, and a second list would be one more thing to hold in
    /// step with the first.
    private func hunkExtent(
        atRow row: Int
    ) -> (first: Int, last: Int, staged: Bool, kind: UInt8)? {
        guard row >= 0, row < Int(docLineCount) else { return nil }
        guard let here = changedLine(at: row) else { return nil }

        // Walk out to the hunk's own ends. Stop AT the boundary row, having
        // included it — the previous version tested the row above and broke
        // before stepping onto it, so the first line of every multi-line hunk
        // was left out. That is one off-by-one and it produced both reported
        // symptoms: the region started a line low, and the bar-thickening test
        // then compared the run against a `first` the run began above, so the
        // bar stayed thin unless the pointer was on that first line.
        var first = row
        while first > 0, changedLine(at: first)?.gitHunkFirst != true {
            guard changedLine(at: first - 1) != nil else { break }
            first -= 1
        }
        var last = row
        let maxRow = Int(docLineCount) - 1
        while last < maxRow, changedLine(at: last)?.gitHunkLast != true {
            guard changedLine(at: last + 1) != nil else { break }
            last += 1
        }
        return (first, last, here.gitHunkStaged, here.gitSignKind)
    }

    /// The line at `row`, if it carries a change.
    private func changedLine(at row: Int) -> EditorLine? {
        guard row >= 0, row < Int(docLineCount) else { return nil }
        return rows(row, row)
            .first { Int($0.lineNo) - 1 == row && $0.gitSignKind != 0 }
    }

    /// The wash behind a hovered hunk, and the rules that close it.
    ///
    /// The wash covers the whole change, not the row under the pointer,
    /// because the change is what the menu on it will act upon. The rules are
    /// what make a run of lines read as ONE region with a start and an end
    /// rather than as a stretch of tinted rows — Xcode draws them, and without
    /// them a hunk that runs off the top of the viewport looks the same as one
    /// that begins there.
    private func hunkHoverRects(lineH: CGFloat) -> [(CGRect, NSColor)] {
        // Held through the fade-out, when `hoveredHunk` is already nil.
        guard let h = hoveredHunk ?? fadingHunk else { return [] }
        let e = hoverEase
        guard e > 0.001 else { return [] }
        // The region covers the opened block too — it is the same change.
        var top = visualY(h.first)
        if let e = shownChange, e.insertAt == h.first {
            top -= insertedHeight(above: h.first)
        }
        let bottom = (visualY(h.last) + lineH)
        let rule = Self.gitHoverRule
        // The region takes the CHANGE's colour, not one fixed blue. A deletion
        // is red in the gutter and was washing blue, which said the region and
        // the bar were about different things.
        let base = gitColor(h.kind)
        return [
            (
                CGRect(x: 0, y: top, width: bounds.width, height: bottom - top),
                base.withAlphaComponent(Self.hoverWashAlpha * e)
            ),
            (
                CGRect(x: 0, y: top, width: bounds.width, height: rule),
                base.withAlphaComponent(Self.hoverEdgeAlpha * e)
            ),
            (
                CGRect(x: 0, y: bottom - rule, width: bounds.width, height: rule),
                base.withAlphaComponent(Self.hoverEdgeAlpha * e)
            ),
        ]
    }

    static let hoverWashAlpha: CGFloat = 0.15
    static let hoverEdgeAlpha: CGFloat = 0.60

    override func mouseDown(with event: NSEvent) {
        bracketKey = ""
        // Commit any in-progress composition BEFORE the click moves the caret.
        // Otherwise the uncommitted marked text (e.g. Hangul just typed after a
        // ')') stays live and re-anchors to the new caret — it appears to
        // "follow the click" instead of staying where it was composed.
        commitMarkedTextForDocumentAction()
        window?.makeFirstResponder(self)
        guard let engine else { return }
        // Whatever part of this pane was hit, the click is a statement about
        // THIS pane. Core has to agree before anything below runs, because
        // everything below acts on the live document: `toggleBreakpointLine`
        // sets a breakpoint in it, and the hunk menu shows and discards its
        // changes.
        //
        // This used to sit after both gutter branches, and both of them
        // `return`. So the focus moved for a click on the text and not for a
        // click on the gutter: clicking line 1's number in an unfocused pane
        // put a breakpoint on line 1 of the pane you were already in, and a
        // click on its change bar offered you the other file's hunk — with
        // Discard on the menu.
        if engine.editorSplit.isSplit, paneIndex != engine.editorSplit.focus {
            engine.focusPane(paneIndex)
        }
        let p = convert(event.locationInWindow, from: nil)
        // The gutter has two targets, and they used to be one.
        //
        // A press anywhere left of the text toggled a breakpoint, including on
        // the change bar — so pressing a hunk would have set a breakpoint every
        // time. The bar's column belongs to the hunk; the line NUMBER belongs
        // to the breakpoint.
        if p.x <= Self.gitBarZone {
            // The change bar's own column. Deliberately handled here rather
            // than falling through — falling through is what set a breakpoint
            // on every press meant for a hunk.
            if let h = hunkExtent(atRow: nearestBufferRow(atY: p.y)) {
                showHunkMenu(h, at: p)
            }
            return
        }
        if p.x < EditorMetrics.gutter - EditorMetrics.gutterTextGap * 0.5 {
            // An inserted line has no buffer row and therefore no breakpoint.
            guard let row = bufferRow(atY: p.y) else { return }
            // Neither does a wrapped line's continuation. Its gutter is empty
            // BECAUSE it owns no line — clicking there set a breakpoint on the
            // line above, from a row that shows no number to say so.
            let rowPitch = max(1, EditorMetrics.lineHeight)
            guard segmentOfVisual(Int(floor(p.y / rowPitch))) == 0 else { return }
            engine.toggleBreakpointLine(UInt32(row) + 1)
            return
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
        // The next drag starts from a standing start, not from wherever the
        // last one had ramped to.
        autoscrollLeftViewAt = 0
    }

    /// When the pointer last left the viewport during a drag. Zero while it is
    /// inside, so the time ramp below starts over on every excursion.
    private var autoscrollLeftViewAt: CFTimeInterval = 0

    /// One frame of drag-scrolling.
    ///
    /// `autoscroll(with:)` scrolls PROPORTIONALLY to how far outside the view
    /// the pointer is — fine for easing a few points past the edge, but flick
    /// the mouse well below the window and each call jumps a long way, so at
    /// tick rate it staircases instead of gliding. So: ramp, but cap the
    /// per-frame step.
    ///
    /// The ramp used to span six lines of overshoot, which the pointer clears
    /// almost the instant it leaves the view — so the scroll reached its cap
    /// immediately and then ran at one fixed speed no matter what the hand did.
    /// That reads as no acceleration at all, which is what it was.
    ///
    /// Two ramps now, because a drag expresses intent two ways:
    ///
    /// * **how far out** — over 40% of the viewport, so pulling further really
    ///   is faster all the way rather than only for the first few lines;
    /// * **how long held** — ×1 → ×1.8 over 1.5 s, the way every native list
    ///   behaves when you park the pointer at the edge and wait.
    ///
    /// At the edge that is ~0.3 lines a frame (≈14 lines/s at the 45 Hz
    /// tracking rate); held far out for a second and a half, ~5.9 (≈265/s).
    /// Both ends are smoothstepped, so there is no step change to feel.
    private func autoscrollStep(toward point: CGPoint) {
        guard let clip = scrollView?.contentView else { return }
        let visible = visibleRect
        var overshoot: CGFloat = 0
        if point.y < visible.minY { overshoot = point.y - visible.minY }
        else if point.y > visible.maxY { overshoot = point.y - visible.maxY }
        guard overshoot != 0 else {
            autoscrollLeftViewAt = 0
            return
        }

        let now = CACurrentMediaTime()
        if autoscrollLeftViewAt == 0 { autoscrollLeftViewAt = now }

        let lineH = EditorMetrics.lineHeight
        let span = max(lineH * 6, visible.height * 0.4)
        let ramp = min(abs(overshoot) / span, 1)
        let eased = ramp * ramp * (3 - 2 * ramp)          // smoothstep
        let held = min((now - autoscrollLeftViewAt) / 1.5, 1)
        let sustain = 1 + 0.8 * (held * held * (3 - 2 * held))
        let step = (lineH * 0.3 + lineH * 3.0 * eased) * sustain
            * (overshoot < 0 ? -1 : 1)

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


    /// Repaint the bracket hint's own rows for the length of its flash.
    ///
    /// This used to set `needsDisplay = true` — the WHOLE viewport — sixty
    /// times a second for 0.9 s. A full repaint measured 1.6 ms in a release
    /// build, so every matched bracket cost ~96 ms of main-thread work per
    /// second of flash, and writing code retriggers it on nearly every
    /// keystroke. The hint covers two cells.
    private func startBracketFade() {
        bracketFadeTimer?.invalidate()
        let started = bracketShownAt
        let t = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] timer in
            guard let self else { timer.invalidate(); return }
            self.invalidateBracketRows()
            if CACurrentMediaTime() - started >= Self.bracketFlashDuration {
                timer.invalidate()
                self.bracketFadeTimer = nil
                // One last pass with the hint expired, so it clears.
                self.invalidateBracketRows()
            }
        }
        RunLoop.main.add(t, forMode: .common)
        bracketFadeTimer = t
    }

    /// Rows a bracket hint was painted into on the last draw. Empty before the
    /// first paint, which is the one case that still needs the whole view.
    private var bracketRects: [CGRect] = []

    private func invalidateBracketRows() {
        guard !bracketRects.isEmpty else {
            needsDisplay = true
            return
        }
        for r in bracketRects {
            setNeedsDisplay(r)
        }
    }





    // MARK: - Context menu (right-click — standard GUI editing)

    override func menu(for event: NSEvent) -> NSMenu? {
        guard let engine else { return nil }
        // Was a vim-Visual probe, which the GUI never entered — Cut/Copy were
        // therefore always disabled. The painted band knows the truth.
        let selectionActive = bandRows.contains { $0.hasSelection }
        // Where the question was asked. The popover anchors here, and the
        // symbol is read from here, so both have to survive until the menu item
        // is chosen — by then the event is long gone.
        let p = convert(event.locationInWindow, from: nil)
        contextMenuPoint = p
        contextMenuSymbol = symbolUnderPointer(p) ?? ""
        // Click outside a selection moves the caret there first (Xcode behavior).
        //
        // By the SAME hit test the symbol above was read with. This used to use
        // the cell grid, which floors `(x - gutter) / cellWidth` and lands
        // elsewhere wherever a glyph is not one cell wide — so Quick Help named
        // the word under the pointer in its title and asked the language server
        // about whatever the caret hit, and on a line with CJK those are two
        // different symbols.
        if !selectionActive {
            if let (row, u16) = absoluteHitUTF16(p) {
                engine.placeCaretUTF16(row: row, utf16: u16)
            } else {
                let (row, col) = absoluteHit(p)
                engine.placeCaret(row: row, col: col)
            }
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

        // First, because it is the reason for right-clicking a name rather
        // than for right-clicking the editor.
        //
        // Present and disabled when the click was not on an identifier, rather
        // than absent: an item that comes and goes makes the user hunt for it,
        // and "there is nothing here to describe" is the answer they were
        // after anyway.
        let help = item(
            contextMenuSymbol.isEmpty
                ? "Quick Help"
                : "Quick Help for “\(contextMenuSymbol)”",
            #selector(ctxQuickHelp(_:))
        )
        help.image = NSImage(
            systemSymbolName: "info.circle", accessibilityDescription: nil
        )
        help.isEnabled = !contextMenuSymbol.isEmpty
        menu.addItem(help)
        menu.addItem(.separator())

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

    /// Ask about the symbol that was right-clicked, and answer beside it.
    ///
    /// The caret is already on it — `menu(for:)` moved it there — so the
    /// engine's existing hover request is asking about the right thing. What
    /// was missing was anywhere to put the answer at the moment of asking: the
    /// only caller of `refreshHover` was switching to the inspector's Quick
    /// Help tab, which is why that tab's empty state told the user to reopen
    /// it.
    @objc private func ctxQuickHelp(_ sender: Any?) {
        guard let engine else { return }
        engine.refreshHover()

        let popover = NSPopover()
        // Transient: it goes away on the next click anywhere, like every other
        // informational popover on the system. It has nothing to confirm.
        popover.behavior = .transient
        let host = NSHostingController(
            rootView: QuickHelpCard(engine: engine, symbol: contextMenuSymbol)
        )
        // The card opens on the click and fills in when the server answers, so
        // its size is not known when the popover appears — it starts as one
        // line of "Looking up…" and becomes a page. Without this the popover
        // keeps whichever size it was born at, and the answer arrives inside a
        // box built for the spinner.
        host.sizingOptions = [.preferredContentSize]
        popover.contentViewController = host
        // A point has no edges to hang off, so the anchor is a two-point box
        // around the click. Bigger and the card visibly floats away from the
        // word it is about.
        let anchor = NSRect(
            x: contextMenuPoint.x - 1, y: contextMenuPoint.y - 1, width: 2, height: 2
        )
        quickHelpPopover?.close()
        quickHelpPopover = popover
        popover.show(relativeTo: anchor, of: self, preferredEdge: .maxY)
    }

    /// The identifier under `p`, or nil when there is not one there.
    ///
    /// Read off the DRAWN line rather than asked of core, because the drawn
    /// line is what the pointer was over — the same reason the hit test itself
    /// works in UTF-16 offsets into that text. Tabs being expanded in it does
    /// not matter here: a tab is not part of a word either way.
    private func symbolUnderPointer(_ p: CGPoint) -> String? {
        guard let (row, u16) = absoluteHitUTF16(p) else { return nil }
        // The row's segments concatenate to the line, wrapped or not.
        let line = rows(Int(row), Int(row))
            .filter { Int($0.lineNo) - 1 == Int(row) }
            .map(\.text)
            .joined()
        return Self.identifier(in: line, atUTF16: Int(u16))
    }

    /// The word around `idx`.
    ///
    /// Letters, digits and underscore — every language Suisei parses spells
    /// identifiers out of those, and asking core which language this is to
    /// refine it would buy nothing the language server does not already decide
    /// for itself when it answers.
    ///
    /// A click one position past the end of a word still means that word,
    /// which is where the pointer lands when you click the right half of the
    /// last letter.
    static func identifier(in line: String, atUTF16 idx: Int) -> String? {
        let ns = line as NSString
        guard ns.length > 0 else { return nil }
        func isWord(_ at: Int) -> Bool {
            // A lone surrogate is half a character and cannot be classified.
            // Identifiers outside the BMP are rare enough that treating one as
            // a boundary loses nothing worth the complication.
            guard let scalar = UnicodeScalar(ns.character(at: at)) else { return false }
            let c = Character(scalar)
            return c.isLetter || c.isNumber || c == "_"
        }
        var i = min(max(0, idx), ns.length - 1)
        if !isWord(i), i > 0, isWord(i - 1) { i -= 1 }
        guard isWord(i) else { return nil }
        var start = i
        var end = i
        while start > 0, isWord(start - 1) { start -= 1 }
        while end + 1 < ns.length, isWord(end + 1) { end += 1 }
        return ns.substring(with: NSRange(location: start, length: end - start + 1))
    }

    private func absoluteHit(_ docPoint: CGPoint) -> (UInt32, UInt32) {
        let lineH = max(1, EditorMetrics.lineHeight)
        let cell = max(1, EditorMetrics.cellWidth)
        let gutter = EditorMetrics.gutter
        let row = UInt32(nearestBufferRow(atY: docPoint.y))
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
        let row = nearestBufferRow(atY: docPoint.y)
        // WHICH SEGMENT of the line was clicked, not just which line. A click
        // on the third screen row of a wrapped line resolves inside that row's
        // text, and the offset it produces is relative to that chunk — so the
        // chunks above it on the same line have to be added back before core,
        // which walks the whole line, is told a position.
        let segment = max(0, Int(floor(docPoint.y / lineH)) - visualRowOf(row))
        let band = rows(row, row)
        let ofRow = band.filter { Int($0.lineNo) - 1 == row }
        guard !ofRow.isEmpty else { return nil }
        let at = min(segment, ofRow.count - 1)
        let line = ofRow[at]
        // UTF-16 length of every chunk before this one. The chunks concatenate
        // to the line, so their lengths sum to the offset this one starts at.
        let base = ofRow[0..<at].reduce(0) { $0 + ($1.text as NSString).length }
        let font = EditorMetrics.monospaced(EditorMetrics.fontSize, weight: .regular)
        let ct = ctLine(for: line, font: font)
        let x = docPoint.x - EditorMetrics.gutter
        let idx = CTLineGetStringIndexForPosition(ct, CGPoint(x: max(0, x), y: 0))
        guard idx != kCFNotFound else { return nil }
        return (UInt32(row), UInt32(max(0, base + Int(idx))))
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
        let hCols = max(0, Int(floor(documentVisibleRect.minX / cell)))
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
        guard !resolvingMarkedTextForDocumentAction else { return }
        EditorDiagnostics.reportIME(
            "insertText", "\(asString(string))", marked: markedText
        )
        inputHandledKey = true
        commitText(asString(string))
    }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        guard !resolvingMarkedTextForDocumentAction else { return }
        EditorDiagnostics.reportIME(
            "setMarkedText", "\(asString(string))", marked: markedText
        )
        inputHandledKey = true
        let beginningComposition = markedText.isEmpty
        if beginningComposition {
            markedAnchorUTF16 = replacementRange.location == NSNotFound
                ? (engine?.caretUTF16Offset() ?? 0)
                : replacementRange.location
        }
        markedText = asString(string)
        let count = markedText.utf16.count
        let location = min(max(0, selectedRange.location), count)
        let length = min(max(0, selectedRange.length), count - location)
        markedSelectionRange = NSRange(location: location, length: length)
        // Repaint ONLY the caret line, not the whole view. A Korean syllable
        // fires several `setMarkedText` calls (one per jamo), and a full
        // `needsDisplay` repainted the entire visible band (~40 rows) each time
        // — that per-jamo full repaint is why CJK input felt laggy next to
        // English (one commit, one repaint). The composition only ever affects
        // the caret's own line (its suffix shifts); clearing the whole line
        // width also erases a longer previous composition when it shrinks.
        //
        // Both numbers below were wrong, and either one puts the dirty rect on
        // the wrong rows — so the composing jamo is painted and never
        // composited, which looks exactly like the key not registering.
        //
        // The ROW came from `chrome.cursorRow`, a snapshot the typing fast path
        // deliberately never publishes: after any run of Latin typing it names
        // wherever the caret was before that run. Pulled live now, like
        // `revealCaret`.
        //
        // The Y came from `row * lineHeight`, which is only the row's position
        // when nothing above it is expanded. `visualY` is the arithmetic the
        // draw itself uses; a shown change or a live-reload insertion above the
        // caret shifted every row below it and this rect stayed put.
        let lineH = EditorMetrics.lineHeight
        let row = engine?.caretRowVCol().row ?? 0
        let band = wrapLines ? lineH * 6 : lineH * 2
        setNeedsDisplay(CGRect(
            x: 0, y: max(0, visualY(row) - 1),
            width: bounds.width, height: band + 2
        ))
    }

    func unmarkText() {
        EditorDiagnostics.reportIME("unmarkText", "→ commits", marked: markedText)
        guard !markedText.isEmpty else { return }
        let pending = markedText
        markedText = ""
        markedSelectionRange = NSRange(location: 0, length: 0)
        markedAnchorUTF16 = 0
        if resolvingMarkedTextForDocumentAction {
            needsDisplay = true
            return
        }
        // NSTextInputClient's unmark means "accept this composition". Because
        // marked text is drawn provisionally rather than inserted into Core,
        // clearing it without this commit silently loses the last syllable.
        commitText(pending)
        noteContentChanged()
    }

    func hasMarkedText() -> Bool { !markedText.isEmpty }

    /// Synthetic ranges: the document's UTF-16 offsets are not exposed by the
    /// core yet. The input method only needs these to stay self-consistent
    /// while composing, which they do.
    func markedRange() -> NSRange {
        markedText.isEmpty
            ? NSRange(location: NSNotFound, length: 0)
            : NSRange(location: markedAnchorUTF16, length: markedText.utf16.count)
    }

    func selectedRange() -> NSRange {
        markedText.isEmpty
            ? NSRange(location: engine?.caretUTF16Offset() ?? 0, length: 0)
            : NSRange(
                location: markedAnchorUTF16 + markedSelectionRange.location,
                length: markedSelectionRange.length
            )
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
    /// Commands that REMOVE text.
    ///
    /// An input method commits text; it never deletes any. So a commit cannot
    /// stand in for one of these, however much it "handled" the key.
    private static let deletionCommands: Set<Selector> = [
        #selector(NSStandardKeyBindingResponding.deleteBackward(_:)),
        #selector(NSStandardKeyBindingResponding.deleteForward(_:)),
        #selector(NSStandardKeyBindingResponding.deleteWordBackward(_:)),
        #selector(NSStandardKeyBindingResponding.deleteWordForward(_:)),
        #selector(NSStandardKeyBindingResponding.deleteToBeginningOfLine(_:)),
        #selector(NSStandardKeyBindingResponding.deleteToEndOfLine(_:)),
        #selector(NSStandardKeyBindingResponding.deleteToBeginningOfParagraph(_:)),
        #selector(NSStandardKeyBindingResponding.deleteToEndOfParagraph(_:)),
    ]

    override func doCommand(by selector: Selector) {
        EditorDiagnostics.reportIME(
            "doCommand", "\(selector) handled=\(inputHandledKey)", marked: markedText
        )
        // `inputHandledKey` means the input method turned this key into TEXT,
        // and the guard exists so the key's raw meaning is not applied on top —
        // Enter confirming a candidate must not also insert a newline.
        //
        // It was applied to every command, including the ones that delete.
        // Backspace on a live composition, traced with `SUISEI_DIAG=ime`:
        //
        //     keyDown code=51        marked="ㅇ"
        //     setMarkedText ㅇ       marked="ㅇ"
        //     insertText ㅇ          marked="ㅇ"   ← commits the jamo
        //     doCommand deleteBackward: handled=true
        //
        // The Korean input method commits the composing jamo and THEN asks for
        // the delete, and both were wanted: the commit makes the jamo real, the
        // delete takes a character away. Swallowing the delete left the press
        // doing nothing you could see — the character it removed was the one it
        // had just made real — so it took two presses to remove one. That is
        // the reported "백스페이스를 두 번 눌러야 지워짐".
        //
        // Deletions only, because that is what the trace shows. A move after a
        // commit is the same shape and may well have the same bug, but nobody
        // has reproduced it and `SUISEI_DIAG=ime` is how they would.
        if inputHandledKey, !Self.deletionCommands.contains(selector) { return }
        if !markedText.isEmpty {
            // Navigation/Return after composition applies after the composed
            // text, never underneath a floating marked string.
            let pending = markedText
            markedText = ""
            markedSelectionRange = NSRange(location: 0, length: 0)
            markedAnchorUTF16 = 0
            commitText(pending)
        }
        fallbackToLegacyKeyPath()
    }
}
