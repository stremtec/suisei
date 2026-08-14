import CoreGraphics
import Foundation

/// Standalone geometry regression test.
///
/// Run with:
///   swiftc Suisei/TabStripLayout.swift Tests/TabStripLayoutGeometryTests.swift \
///     -o /tmp/suisei-tab-strip-tests && /tmp/suisei-tab-strip-tests
@main
private enum TabStripLayoutGeometryTests {
    private static func require(
        _ condition: @autoclosure () -> Bool,
        _ message: String
    ) {
        guard condition() else {
            fputs("TabStripLayoutGeometryTests: \(message)\n", stderr)
            exit(1)
        }
    }

    static func main() {
        testCloseRectIsTheChipsOwnTrailingSlot()
        testEveryFilenameUsesItsOwnTrailingEdge()
        testCloseRectStaysInsideItsChip()
        testScrolledLayoutKeepsTheSameCentre()
        testViewportUsesTheActualSidebarBoundary()
        testViewportStopsBeforeTheNativeToolbar()
        testSidebarSweepDoesNotMoveTheRun()
        testANarrowCorridorStillPushesTheRunClear()
        testCorridorActuallyTravels()
        testRetargetingMidFlightLeavesFromWhereItIs()
        testTheToolbarEdgeTravelsWithTheRest()
        print("TabStripLayoutGeometryTests: passed")
    }

    /// Where the chip's `HStack` puts its trailing slot, computed the way the
    /// chip lays itself out: content inset by `horizontalPadding`, trailing
    /// slot flush against that inset edge.
    private static func drawnSlotLeadingEdge(chipWidth: CGFloat) -> CGFloat {
        chipWidth - TabChipBox.horizontalPadding - TabChipBox.trailingSlotWidth
    }

    /// The hit rect IS the drawn slot, grown by the hover slack — not a rect
    /// that happens to land near it.
    ///
    /// This is the invariant the strip keeps breaking. `closeSlot` used to
    /// hard-code `maxX - 24`, a hand-copy of `horizontalPadding +
    /// trailingSlotWidth`; changing the chip's padding moved the × and left the
    /// rect behind, and the symptom was always "누른 자리랑 그려진 자리가 다름".
    private static func testCloseRectIsTheChipsOwnTrailingSlot() {
        let widths: [CGFloat] = [62, 113, 219, 78]
        let layout = makeLayout(widths: widths, viewport: 700)
        let rowHeight = TabChipBox.height

        for chip in layout.chips {
            let rect = layout.closeRect(for: chip, rowHeight: rowHeight)
            let drawn = CGRect(
                x: drawnSlotLeadingEdge(chipWidth: chip.width) + chip.x,
                y: (rowHeight - TabChipBox.trailingSlotWidth) / 2,
                width: TabChipBox.trailingSlotWidth,
                height: TabChipBox.trailingSlotWidth
            )
            let slack = TabChipBox.closeHitInset
            require(
                rect == drawn.insetBy(dx: -slack, dy: -slack),
                "slot \(chip.slot): hit rect \(rect) is not the drawn slot "
                    + "\(drawn) grown by \(slack)"
            )
        }
    }

    /// Every chip's close rect is derived from its OWN trailing edge, at the
    /// centre of the slot its `HStack` draws there.
    private static func testEveryFilenameUsesItsOwnTrailingEdge() {
        let widths: [CGFloat] = [62, 113, 219, 78]
        let layout = makeLayout(widths: widths, viewport: 700)

        for (slot, chip) in layout.chips.enumerated() {
            let centre = CGPoint(
                x: layout.originX + chip.maxX - TabChipBox.closeSlotInset
                    + TabChipBox.trailingSlotWidth / 2,
                y: TabChipBox.height / 2
            )
            require(
                layout.closeSlot(at: centre, rowHeight: TabChipBox.height) == slot,
                "slot \(slot) did not answer at its own trailing edge"
            )
        }
    }

    /// The rect stays inside the chip, so a press can never close a neighbour.
    private static func testCloseRectStaysInsideItsChip() {
        let layout = makeLayout(widths: [120, 120], viewport: 600)
        let second = layout.chips[1]
        let bodyOfSecond = layout.originX + second.x + 1
        require(
            layout.closeSlot(at: CGPoint(x: bodyOfSecond, y: 12), rowHeight: 24)
                != layout.chips[0].slot,
            "a press on the next chip closed the previous tab"
        )
    }

    /// Scrolling moves the rect with the chip; nothing is cached.
    private static func testScrolledLayoutKeepsTheSameCentre() {
        let widths: [CGFloat] = [92, 167, 74, 205]
        let layout = makeLayout(widths: widths, viewport: 250, scroll: 121)
        require(layout.overflow, "fixture must overflow")

        for (slot, chip) in layout.chips.enumerated() {
            let centre = CGPoint(
                x: layout.originX + chip.maxX - TabChipBox.closeSlotInset
                    + TabChipBox.trailingSlotWidth / 2,
                y: TabChipBox.height / 2
            )
            require(
                layout.closeSlot(at: centre, rowHeight: TabChipBox.height) == slot,
                "scrolled slot \(slot) did not preserve its close centre"
            )
        }
    }

    /// A resized sidebar is an edge, not a hint. The old fixed 296pt inset put
    /// overflowing tabs underneath any navigator wider than that.
    private static func testViewportUsesTheActualSidebarBoundary() {
        let viewport = TabStripViewportGeometry.resolve(
            contentMinX: 0, contentMaxX: 1280,
            leadingInset: 376, trailingInset: 150,
            toolbarLeadingX: nil, trailingRunReserve: 34
        )
        require(viewport?.x == 376, "viewport ignored the live sidebar edge")
        require(viewport?.width == 754, "viewport width did not preserve both edges")
    }

    /// The toolbar is laid out by AppKit and can grow with viewer controls. Its
    /// measured leading edge wins over a guessed trailing inset, with room left
    /// for the strip's trailing + button.
    private static func testViewportStopsBeforeTheNativeToolbar() {
        let viewport = TabStripViewportGeometry.resolve(
            contentMinX: 0, contentMaxX: 1280,
            leadingInset: 296, trailingInset: 150,
            toolbarLeadingX: 760, trailingRunReserve: 30
        )
        require(viewport?.x == 296, "toolbar moved the leading boundary")
        require(viewport?.width == 434, "viewport did not stop before the toolbar")
        require(
            (viewport?.x ?? 0) + (viewport?.width ?? 0) + 30 == 760,
            "trailing + reserve overlaps the toolbar"
        )
    }

    /// THE regression. Rule H1: the run does not move because the sidebar did.
    ///
    /// Every width here is one frame of a sidebar open/close, and the corridor
    /// moves with each. While the run fits, its centre in WINDOW space must be
    /// the window's centre at every one of them — not the corridor's, which is
    /// what centring inside the viewport gave and what made the strip slide on
    /// every toggle.
    private static func testSidebarSweepDoesNotMoveTheRun() {
        let widths: [CGFloat] = [120, 140, 160]
        let windowWidth: CGFloat = 1280
        let windowCentre = windowWidth / 2
        var centres: [(centre: CGFloat, pinned: Bool)] = []

        for sidebar in stride(from: CGFloat(0), through: 460, by: 20) {
            guard let vp = TabStripViewportGeometry.resolve(
                contentMinX: 0, contentMaxX: windowWidth,
                leadingInset: sidebar > 1 ? sidebar + 8 : 150,
                trailingInset: 150,
                toolbarLeadingX: nil, trailingRunReserve: 34
            ) else { continue }
            let layout = makeLayout(
                widths: widths, viewport: vp.width,
                preferredCentre: windowCentre - vp.x
            )
            require(!layout.overflow, "fixture must fit at sidebar \(sidebar)")
            let pinned = layout.originX <= 0.5
                || layout.originX >= vp.width - layout.contentWidth - 0.5
            // Strip space → window space.
            centres.append((
                centre: vp.x + layout.originX + layout.contentWidth / 2,
                pinned: pinned
            ))
        }

        require(centres.count > 10, "sweep produced too few samples")
        // Every sample is either on the window's centreline, or pinned to a
        // corridor edge because centring would have crossed it. Nothing in
        // between: a sample that is off-centre WITHOUT the clamp engaged is
        // the run tracking the sidebar, which is the defect.
        for (centre, pinned) in centres {
            require(
                abs(centre - windowCentre) <= 1 || pinned,
                "the run moved with the sidebar: centre \(centre) vs window \(windowCentre)"
            )
        }
        require(
            centres.contains { abs($0.centre - windowCentre) <= 1 },
            "the sweep never reached the centred case"
        )
        require(
            centres.contains { $0.pinned },
            "the sweep never reached the clamped case — widen it"
        )
    }

    /// The other half of the same rule. Anchoring must never win over the
    /// corridor — a run that cannot be centred without crossing a boundary is
    /// pushed clear of it instead, which is what keeps tabs out from under a
    /// widened navigator.
    private static func testANarrowCorridorStillPushesTheRunClear() {
        let widths: [CGFloat] = [300, 300]
        guard let vp = TabStripViewportGeometry.resolve(
            contentMinX: 0, contentMaxX: 1280,
            leadingInset: 448, trailingInset: 150,
            toolbarLeadingX: nil, trailingRunReserve: 34
        ) else { return require(false, "corridor should exist") }
        let layout = makeLayout(
            widths: widths, viewport: vp.width, preferredCentre: 640 - vp.x
        )
        require(!layout.overflow, "fixture must still fit")
        require(layout.originX >= 0, "the run started before the corridor")
        require(
            layout.originX + layout.contentWidth <= vp.width + 0.5,
            "the run ran past the corridor"
        )
        // The window centre sits left of where this run can go, so the clamp
        // must have taken it — pinned to the sidebar edge, not centred.
        require(layout.originX == 0, "the clamp did not engage")
    }

    /// A journey has to have a length.
    ///
    /// Twice now the corridor's `from` was captured AFTER the destination had
    /// been written, so it travelled from the target to the target: the code
    /// looked animated, compiled, and produced a hard cut. Nothing but an
    /// assertion catches that.
    private static func testCorridorActuallyTravels() {
        let travel = TabStripCorridorTravel(
            fromLeading: 288, fromTrailing: 150, fromToolbar: 900,
            start: 0, duration: 0.25
        )
        let atStart = travel.edges(
            at: 0, settledLeading: 150, settledTrailing: 150, settledToolbar: 900
        )
        require(abs(atStart.leading - 288) < 0.01, "did not leave from where it was")

        let midway = travel.edges(
            at: 0.125, settledLeading: 150, settledTrailing: 150, settledToolbar: 900
        )
        require(
            midway.leading < 288 - 1 && midway.leading > 150 + 1,
            "the corridor teleported instead of travelling: \(midway.leading)"
        )

        let done = travel.edges(
            at: 0.25, settledLeading: 150, settledTrailing: 150, settledToolbar: 900
        )
        require(abs(done.leading - 150) < 0.01, "did not arrive")
        require(travel.isFinished(at: 0.25), "finished but not reported so")
        require(!travel.isFinished(at: 0.2), "reported finished early")
    }

    /// The destination is read at sample time, not frozen into the journey, so
    /// a boundary that moves mid-flight is followed rather than fought — and a
    /// new journey started from the live value picks the run up where it is.
    private static func testRetargetingMidFlightLeavesFromWhereItIs() {
        let first = TabStripCorridorTravel(
            fromLeading: 288, fromTrailing: 150, fromToolbar: nil,
            start: 0, duration: 0.25
        )
        let live = first.edges(
            at: 0.1, settledLeading: 150, settledTrailing: 150, settledToolbar: nil
        )
        let second = TabStripCorridorTravel(
            fromLeading: live.leading, fromTrailing: live.trailing,
            fromToolbar: live.toolbar, start: 0.1, duration: 0.25
        )
        let resumed = second.edges(
            at: 0.1, settledLeading: 288, settledTrailing: 150, settledToolbar: nil
        )
        require(
            abs(resumed.leading - live.leading) < 0.01,
            "the retarget jumped instead of continuing from \(live.leading)"
        )
    }

    /// The toolbar's edge grows and shrinks with the viewer controls, so it is
    /// a moving boundary like the others — except the very first measurement,
    /// which has no departure point and must land.
    private static func testTheToolbarEdgeTravelsWithTheRest() {
        let travel = TabStripCorridorTravel(
            fromLeading: 150, fromTrailing: 150, fromToolbar: 1000,
            start: 0, duration: 0.25
        )
        let midway = travel.edges(
            at: 0.125, settledLeading: 150, settledTrailing: 150, settledToolbar: 800
        )
        require(
            (midway.toolbar ?? 0) < 1000 - 1 && (midway.toolbar ?? 0) > 800 + 1,
            "the toolbar edge snapped: \(String(describing: midway.toolbar))"
        )

        let firstEver = TabStripCorridorTravel(
            fromLeading: 150, fromTrailing: 150, fromToolbar: nil,
            start: 0, duration: 0.25
        )
        let landed = firstEver.edges(
            at: 0.125, settledLeading: 150, settledTrailing: 150, settledToolbar: 800
        )
        require(landed.toolbar == 800, "a first measurement should land, not slide")
    }

    private static func makeLayout(
        widths: [CGFloat],
        viewport: CGFloat,
        scroll: CGFloat = 0,
        preferredCentre: CGFloat? = nil
    ) -> TabStripLayout {
        TabStripLayout(
            tabs: widths.indices.map { (stableId: UInt64($0 + 1), group: 0) },
            viewportWidth: viewport,
            scrollOffset: scroll,
            preferredCentre: preferredCentre,
            widthFor: { widths[$0] }
        )
    }

    private static func circleSamples(
        centre: CGPoint,
        radius: CGFloat
    ) -> [CGPoint] {
        stride(from: 0.0, to: Double.pi * 2, by: Double.pi / 4).map { angle in
            CGPoint(
                x: centre.x + radius * CGFloat(cos(angle)),
                y: centre.y + radius * CGFloat(sin(angle))
            )
        }
    }
}
