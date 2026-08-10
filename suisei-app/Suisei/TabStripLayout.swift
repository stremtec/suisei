import CoreGraphics
import Foundation

/// Where every tab chip is, computed rather than measured.
///
/// The strip's geometry used to be four independent SwiftUI measurements —
/// each chip's rect in row space, each chip's rect in strip space, the row's
/// origin, the row's width — every one published by its own
/// `.onGeometryChange` on its own schedule. Nothing made them agree, and
/// SwiftUI offers no atomic "here is the layout now" snapshot, so a value read
/// during an event handler was a mixture of different frames' timestamps.
/// Combining two of them was a race, and it surfaced as: clicking a grouped
/// member navigating to a different file, a merged layout trapping the strip,
/// hover reporting the wrong chip, and stutter that grew with tab count and
/// with overflow.
///
/// Measured, with `SUISEI_TABLOG=1`, across two clicks of ONE double-click at a
/// stationary pointer: the row origin moved 187 → 14, a 173pt disagreement,
/// about one and a half chips. The arithmetic was never wrong; the inputs came
/// from different moments.
///
/// A chip's width is a pure function of its title, font and flags, so none of
/// this needs the view system. Given widths, everything else is arithmetic over
/// one array — and a hit test and a paint in the same frame read the same
/// numbers by construction. See `docs/SUISEI-TAB-STRIP-GEOMETRY.md`.
struct TabStripLayout: Equatable {
    struct Chip: Equatable {
        let stableId: UInt64
        /// Slot index — what every engine call takes.
        let slot: Int
        /// Leading edge in ROW space (0 = first chip's leading edge).
        let x: CGFloat
        let width: CGFloat
        /// The folded layout this chip belongs to, or 0.
        let group: UInt64

        var maxX: CGFloat { x + width }
    }

    /// Gap between chips. Matches the `HStack(spacing:)` the row is drawn with.
    static let gap: CGFloat = 4
    /// Width of the "+" slot.
    static let plusWidth: CGFloat = 22

    let chips: [Chip]
    /// Total width of the chip run, excluding any trailing gap.
    let contentWidth: CGFloat
    let viewportWidth: CGFloat
    /// The span the chip run may occupy: the viewport minus the "+" slot.
    ///
    /// Every question about where chips go is asked against this, never against
    /// `viewportWidth` — the difference between the two is the reserved slot,
    /// and any query that forgets it puts a chip under the button.
    let runWidth: CGFloat
    /// Leading edge of the row within the viewport, in STRIP space.
    ///
    /// Centred while the run fits; otherwise the negated scroll offset, so the
    /// row slides under a fixed viewport. This is the single number that used
    /// to be measured asynchronously and is now derived.
    let originX: CGFloat
    let overflow: Bool

    /// Build from the chips' own widths.
    ///
    /// `widthFor` is injected rather than called directly so this stays pure:
    /// tests supply fixed widths, the app supplies a cached CoreText
    /// measurement. It is the only thing here that needs a font.
    init(
        tabs: [(stableId: UInt64, group: UInt64)],
        viewportWidth: CGFloat,
        scrollOffset: CGFloat,
        widthFor: (Int) -> CGFloat
    ) {
        var built: [Chip] = []
        built.reserveCapacity(tabs.count)
        var pen: CGFloat = 0
        for (slot, tab) in tabs.enumerated() {
            let w = max(0, widthFor(slot))
            built.append(Chip(stableId: tab.stableId, slot: slot, x: pen, width: w, group: tab.group))
            pen += w + Self.gap
        }
        // `pen` carries one gap past the last chip; the run itself does not.
        let content = built.isEmpty ? 0 : pen - Self.gap

        chips = built
        contentWidth = content
        self.viewportWidth = viewportWidth
        // The run gets the viewport MINUS the "+" slot, in both regimes.
        //
        // The "+" rides the run's trailing edge, so it needs `plusWidth + gap`
        // after the last chip whether or not the run overflows. Sizing the run
        // to the whole viewport meant a full strip filled every point of it and
        // `plusX`'s clamp then parked the button back on top of the last chips
        // — the reported overlap. Reserving the slot up front is what makes
        // that clamp unreachable rather than merely survivable.
        let run = max(0, viewportWidth - (Self.plusWidth + Self.gap))
        runWidth = run
        overflow = content > run
        if overflow {
            // Clamp so the run cannot be scrolled past either end — the ends
            // are exactly where a measured layout used to drift.
            let maxScroll = content - run
            originX = -min(max(0, scrollOffset), maxScroll)
        } else {
            // Centred in the RUN's span, not the viewport's: the reserved "+"
            // slot is part of the strip, so centring against the full width
            // would push the chips half a slot left of where they read.
            originX = ((run - content) / 2).rounded()
        }
    }

    // MARK: - Queries
    //
    // All take STRIP-space coordinates, which is what the pointer arrives in.
    // There is no second space and no conversion, which is the point.

    /// Slot under `x`, or nil in a gap or past the ends.
    ///
    /// Chips are laid out left to right and never overlap, so this is a plain
    /// ordered scan with an early exit — no "narrowest match wins" tie-break,
    /// because with computed geometry there are no ties to break.
    func slot(at x: CGFloat) -> Int? {
        let local = x - originX
        if local < 0 { return nil }
        for chip in chips {
            if local < chip.x { return nil }        // fell in the gap before it
            if local <= chip.maxX { return chip.slot }
        }
        return nil
    }

    /// Slot whose close glyph is under `p`.
    func closeSlot(at p: CGPoint, rowHeight: CGFloat) -> Int? {
        let local = CGPoint(x: p.x - originX, y: p.y)
        for chip in chips {
            let glyph = CGRect(
                x: chip.maxX - 24, y: rowHeight / 2 - 7, width: 14, height: 14
            ).insetBy(dx: -3, dy: -3)
            if glyph.contains(local) { return chip.slot }
        }
        return nil
    }

    /// One-step neighbour swap while dragging, decided by the neighbour's
    /// MIDPOINT rather than its bounds — touching an edge would swap, and the
    /// swap moves the chips back under the cursor, which oscillates.
    func dragTarget(held: Int, x: CGFloat) -> Int? {
        let local = x - originX
        guard let current = chips.first(where: { $0.slot == held }) else { return nil }
        if let next = chips.first(where: { $0.slot == held + 1 }), local > next.x + next.width / 2 {
            return next.slot
        }
        if held > 0,
           let prev = chips.first(where: { $0.slot == held - 1 }),
           local < prev.x + prev.width / 2 {
            return prev.slot
        }
        _ = current
        return nil
    }

    /// Leading/trailing edges of a folded group's run, in STRIP space.
    ///
    /// Was a reduction over separately-measured member rects that could be from
    /// different passes — the "파란 알약이 오른쪽으로 튄다" overshoot. Now the
    /// first and last member's own computed edges.
    func bandExtent(group: UInt64) -> (minX: CGFloat, maxX: CGFloat)? {
        guard group != 0 else { return nil }
        let members = chips.filter { $0.group == group }
        guard let first = members.first, let last = members.last else { return nil }
        return (originX + first.x, originX + last.maxX)
    }

    /// Leading inset for the "+", which rides the run's trailing edge.
    ///
    /// The clamp is now a backstop rather than the common case: `runWidth`
    /// already keeps the run clear of this slot, so `trailing` cannot exceed
    /// `viewportWidth - plusWidth` by construction. It stays because a
    /// zero-width viewport still has to produce a number.
    var plusX: CGFloat {
        let trailing = originX + contentWidth + Self.gap
        return min(max(0, trailing), max(0, viewportWidth - Self.plusWidth))
    }

    /// Scroll offset that brings `slot` fully into view, or nil if it already
    /// is. Explicit, because "obscured" tabs should be a clamp we own rather
    /// than emergent `ScrollView` behaviour.
    func scrollToReveal(slot: Int, currentOffset: CGFloat) -> CGFloat? {
        guard overflow, let chip = chips.first(where: { $0.slot == slot }) else { return nil }
        let visibleMin = currentOffset
        let visibleMax = currentOffset + runWidth
        if chip.x >= visibleMin, chip.maxX <= visibleMax { return nil }
        let maxScroll = max(0, contentWidth - runWidth)
        // Centre it, which is what the auto-reveal has always aimed for.
        let centred = chip.x + chip.width / 2 - runWidth / 2
        return min(max(0, centred), maxScroll)
    }
}
