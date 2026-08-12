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
        testEveryFilenameUsesItsOwnTrailingEdge()
        testCloseRectStaysInsideItsChip()
        testScrolledLayoutKeepsTheSameCentre()
        print("TabStripLayoutGeometryTests: passed")
    }

    /// Every chip's close rect is derived from its OWN trailing edge, at the
    /// same `maxX - 24` the chip's `HStack` puts the slot at.
    private static func testEveryFilenameUsesItsOwnTrailingEdge() {
        let widths: [CGFloat] = [62, 113, 219, 78]
        let layout = makeLayout(widths: widths, viewport: 700)

        for (slot, chip) in layout.chips.enumerated() {
            let centre = CGPoint(
                x: layout.originX + chip.maxX - 17,   // maxX - 24 + 14/2
                y: 12
            )
            require(
                layout.closeSlot(at: centre, rowHeight: 24) == slot,
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
            let centre = CGPoint(x: layout.originX + chip.maxX - 17, y: 12)
            require(
                layout.closeSlot(at: centre, rowHeight: 24) == slot,
                "scrolled slot \(slot) did not preserve its close centre"
            )
        }
    }

    private static func makeLayout(
        widths: [CGFloat],
        viewport: CGFloat,
        scroll: CGFloat = 0
    ) -> TabStripLayout {
        TabStripLayout(
            tabs: widths.indices.map { (stableId: UInt64($0 + 1), group: 0) },
            viewportWidth: viewport,
            scrollOffset: scroll,
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
