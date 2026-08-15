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
        cols: Int
    ) {
        guard wrap else { return }
        let usable = clipWidth - gutter - rightInset
        NSLog(
            """
            [suisei/wrap] pane=\(pane) clip=\(fmt(clipWidth)) \
            − gutter \(fmt(gutter)) − inset \(fmt(rightInset)) = \(fmt(usable))pt \
            ÷ advance \(fmt(advance)) → \(cols) cols \
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
