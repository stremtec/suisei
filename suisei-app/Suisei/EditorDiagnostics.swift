import Foundation

/// Opt-in tracing for "part of the editor did not draw" reports.
///
/// A region that renders and a region beside it that does not has three
/// candidate causes that all look identical on screen: the row band came back
/// short, the Metal instance buffers overflowed and dropped the tail, or the
/// Metal layer's frame did not cover the viewport. Guessing between them from a
/// screenshot is how a wrong fix gets shipped, so each one says so here.
///
/// Off unless `SUISEI_DIAG=band` (or `metal`, or `all`). Nothing is allocated
/// or formatted when off.
enum EditorDiagnostics {
    private static let modes: Set<String> = {
        let raw = ProcessInfo.processInfo.environment["SUISEI_DIAG"]?.lowercased() ?? ""
        return Set(raw.split(separator: ",").map(String.init))
    }()

    static let bandGaps = modes.contains("band") || modes.contains("all")
    static let metal = modes.contains("metal") || modes.contains("all")
    static let wrap = modes.contains("wrap") || modes.contains("all")
    static let ime = modes.contains("ime") || modes.contains("all")
    static let sidebar = modes.contains("sidebar") || modes.contains("all")

    /// Where the Git workbench's sidebar and its first row actually are, per
    /// layout pass.
    ///
    /// "It opens and about halfway it pops upward" has candidate causes that
    /// are indistinguishable on screen and need different fixes:
    ///
    /// * the LIST relaid out — a label unwrapped, a row changed height — and
    ///   everything below it moved. Row Y steps; the container does not.
    /// * the container's top inset changed, because AppKit flipped the titlebar
    ///   between the full-height-sidebar arrangement and the plain one partway
    ///   through the animation. Container Y or safe-top steps, and the row
    ///   follows it by the same amount.
    /// * the column is not sweeping at all but arriving in a couple of jumps.
    ///   Width steps.
    ///
    /// Printing width, the container's origin, the safe-area top and the first
    /// row's origin together makes those three different pictures rather than
    /// one shrug. Logged only when a value moves, since this runs on every
    /// layout pass of an animating view.
    private nonisolated(unsafe) static var lastSidebarSample = ""

    static func reportSidebar(
        width: CGFloat,
        originY: CGFloat,
        safeTop: CGFloat,
        rowY: CGFloat?
    ) {
        guard sidebar else { return }
        let line =
            "w=\(fmt(width)) y=\(fmt(originY)) safeTop=\(fmt(safeTop)) "
            + "row1=\(rowY.map(fmt) ?? "-")"
        guard line != lastSidebarSample else { return }
        lastSidebarSample = line
        NSLog("[suisei/sidebar] \(line)")
    }

    /// Every `NSTextInputClient` call, in order, with the state it left.
    ///
    /// "Backspace needs two presses" has several shapes that all look the same
    /// from a chair: the input method could be emptying the composition on the
    /// first press (so the second is the first real delete, and the behaviour
    /// is right but reads wrong), it could be calling `unmarkText` — which this
    /// client treats as *accept*, and would re-insert the text being deleted —
    /// or the key could be consumed without reaching the document at all.
    ///
    /// They are three different fixes and the difference is entirely in the
    /// order of these calls, which is not visible on screen.
    static func reportIME(_ call: String, _ detail: String, marked: String) {
        guard ime else { return }
        NSLog("[suisei/ime] \(call) \(detail)  marked=\(quoted(marked))")
    }

    private static func quoted(_ s: String) -> String {
        s.isEmpty ? "∅" : "\"\(s)\""
    }

    /// Where a wrapped row's right edge comes from.
    ///
    /// "It wraps with room to spare" has three candidate causes that look the
    /// same on screen: the pane is narrower than it appears, something is
    /// subtracted from it that is not really in the way, or the break rule
    /// gives up a column early. They are three different fixes, so each number
    /// that went into the answer is printed rather than inferred from a
    /// screenshot.
    ///
    /// Reported only when the width changes — this runs on every layout pass.
    static func reportWrap(
        pane: Int,
        clipWidth: CGFloat,
        gutter: CGFloat,
        rightInset: CGFloat,
        advance: CGFloat,
        wideRatio: UInt16,
        cols: Int
    ) {
        guard wrap else { return }
        let usable = clipWidth - gutter - rightInset
        NSLog(
            """
            [suisei/wrap] pane=\(pane) clip=\(fmt(clipWidth)) \
            − gutter \(fmt(gutter)) − inset \(fmt(rightInset)) = \(fmt(usable))pt \
            ÷ advance \(fmt(advance)) → \(cols) cols  wide=\(wideRatio) \
            (row paints \(fmt(CGFloat(cols) * advance))pt, \
            leaves \(fmt(usable - CGFloat(cols) * advance))pt)
            """
        )
    }

    private static func fmt(_ v: CGFloat) -> String {
        String(format: "%.1f", v)
    }

    /// Reported only when the slice fails to cover what the draw asked for —
    /// a full band is the normal case and would drown the interesting one.
    static func reportBand(
        pane: Int,
        want: ClosedRange<Int>,
        bandStart: Int,
        bandCount: Int,
        first: Int?,
        last: Int?,
        gotFirst: Int?,
        gotLast: Int?,
        gotCount: Int
    ) {
        let wantCount = want.upperBound - want.lowerBound + 1
        let covers = gotFirst == want.lowerBound && gotLast == want.upperBound
        guard !covers || gotCount < wantCount else { return }
        NSLog(
            """
            [suisei/band] pane=\(pane) want=\(want.lowerBound)…\(want.upperBound) \
            (\(wantCount) rows) got=\(gotFirst.map(String.init) ?? "-")…\
            \(gotLast.map(String.init) ?? "-") (\(gotCount) rows) \
            cache: start=\(bandStart) count=\(bandCount) \
            rows=\(first.map(String.init) ?? "-")…\(last.map(String.init) ?? "-")
            """
        )
    }

    /// The shared glyph atlas ran out of room. Said once, because once `full`
    /// is set it is never cleared and every frame after this one is affected.
    private nonisolated(unsafe) static var saidAtlasFull = false

    static func reportAtlasFull(row: Int, resident: Int) {
        guard !saidAtlasFull else { return }
        saidAtlasFull = true
        NSLog("[suisei/metal] glyph atlas FULL at row \(row) after \(resident) glyphs")
    }

    /// Reported when the Metal frame could not fit what it was given, or when
    /// the layer does not cover the viewport it was asked to fill.
    static func reportMetal(
        viewport: CGRect,
        bounds: CGRect,
        layerFrame: CGRect,
        rects: Int,
        rectCapacity: Int,
        glyphs: Int,
        glyphCapacity: Int
    ) {
        let overflowed = rects > rectCapacity || glyphs > glyphCapacity
        let uncovered = !layerFrame.contains(viewport)
        guard overflowed || uncovered else { return }
        NSLog(
            """
            [suisei/metal] viewport=\(viewport.integral) bounds=\(bounds.integral) \
            layer=\(layerFrame.integral) rects=\(rects)/\(rectCapacity) \
            glyphs=\(glyphs)/\(glyphCapacity) \
            \(overflowed ? "OVERFLOW " : "")\(uncovered ? "LAYER-SHORT" : "")
            """
        )
    }
}
