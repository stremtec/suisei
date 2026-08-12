import SwiftUI

/// The tab strip's geometry substrate.
///
/// The strip used to be a `ScrollView` wrapping an `HStack`, which *decided*
/// where chips go, surrounded by five measurements trying to *discover* what it
/// had decided: per-chip `tabFrames`, per-chip `tabHitFrames`, the row's
/// `chipRowOrigin`, `tabStripContentWidth`, and `TabStripLayout` computing the
/// same thing a sixth way for hit-testing. Every defect in this strip — a click
/// opening the wrong file, hover lighting a chip three places away, the "+"
/// stuck mid-row, chips untouchable while merged — was two of those disagreeing.
/// Fixing one made the others louder, because each fix removed a symptom and
/// left the cause.
///
/// Here the decision and the knowledge are the same object. `TabStripLayout`
/// computes every chip's position; this `Layout` *places* chips there rather
/// than proposing and measuring; and the hit test reads the same struct. Render
/// and hit-test cannot drift apart, not because they are kept in sync but
/// because there is only one of them.
///
/// What makes this possible at all: the viewport width is an INPUT
/// (`documentTabStrip(maxWidth:)` already had it), and chip widths are exact
/// (`TabChipMetrics`, verified against a hosted `HStack` at 1, 2, 5 and 10
/// chips). Given those two, `originX` and every chip's `x` are arithmetic. There
/// is nothing left to measure.
struct TabStripRow: Layout {
    /// Computed geometry for exactly the subviews passed in, in the same order.
    var layout: TabStripLayout
    /// Row height; chips are centred in it.
    var rowHeight: CGFloat

    /// Scroll and re-centre interpolate; chip insertion rides the transaction.
    var animatableData: CGFloat {
        get { layout.originX }
        set { originX = newValue }
    }
    /// Set by `animatableData` during interpolation; nil means use the computed
    /// value. Kept separate so an in-flight animation cannot be mistaken for
    /// the layout's own opinion about where the row sits.
    private var originX: CGFloat?

    init(layout: TabStripLayout, rowHeight: CGFloat) {
        self.layout = layout
        self.rowHeight = rowHeight
    }

    func sizeThatFits(
        proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) -> CGSize {
        // The row occupies the whole viewport, not just its chips. The chips
        // are positioned WITHIN it by `originX` — centred while they fit,
        // scrolled when they do not. A content-hugging row is what made the
        // strip jump 93pt on merge: the container resized under the content.
        CGSize(width: layout.viewportWidth, height: rowHeight)
    }

    func placeSubviews(
        in bounds: CGRect, proposal: ProposedViewSize,
        subviews: Subviews, cache: inout ()
    ) {
        let origin = originX ?? layout.originX
        for (i, sub) in subviews.enumerated() {
            guard i < layout.chips.count else {
                // More subviews than chips means the caller built the ForEach
                // from a different list than the layout. Park the extras
                // offscreen rather than piling them at x=0, where they would
                // be invisible in a screenshot but still hit-testable.
                sub.place(
                    at: CGPoint(x: bounds.minX - 10_000, y: bounds.midY),
                    anchor: .leading, proposal: .zero
                )
                continue
            }
            let chip = layout.chips[i]
            // The width is DICTATED, not proposed-and-accepted. A chip cannot
            // report a width the hit test does not know about, which is the
            // whole disagreement the old strip was built on.
            sub.place(
                at: CGPoint(x: bounds.minX + origin + chip.x, y: bounds.midY),
                anchor: .leading,
                proposal: ProposedViewSize(width: chip.width, height: rowHeight)
            )
        }
    }
}

/// Scroll offset for the chip run, owned rather than observed.
///
/// A `ScrollView` keeps this privately and reports it back asynchronously —
/// that report is the `chipRowOrigin` whose one-pass lag produced the 187→14
/// race. Holding it here makes it available to the layout in the same pass it
/// changes, so nothing has to guess where the row currently is.
@MainActor
final class TabStripScroll: ObservableObject {
    /// Points scrolled from the leading edge. Clamped by `TabStripLayout`
    /// against the live content width, so it can never address past the ends.
    @Published var offset: CGFloat = 0

    /// Advance on a horizontal wheel/trackpad gesture.
    ///
    /// Returns whether the strip actually moved, so the caller can let an
    /// unconsumed scroll fall through instead of swallowing it at the ends.
    func scroll(by delta: CGFloat, layout: TabStripLayout) -> Bool {
        guard layout.overflow else { return false }
        let maxScroll = max(0, layout.contentWidth - layout.viewportWidth)
        let next = min(max(0, offset - delta), maxScroll)
        guard next != offset else { return false }
        offset = next
        return true
    }

    /// Bring a slot fully into view, if it is not already.
    ///
    /// `scrollToReveal` returns nil when the chip is already whole on screen,
    /// which is the common case — so this writes nothing on most selections and
    /// the strip does not re-centre itself under the pointer.
    func reveal(slot: Int, layout: TabStripLayout) {
        guard let next = layout.scrollToReveal(slot: slot, currentOffset: offset),
              next != offset else { return }
        offset = next
    }
}
