// Does the tab strip's × land where it is drawn?
//
// The question this project kept getting wrong by reasoning. Every previous
// answer — "the chip is too wide", "the catcher hangs off the padded box",
// "`navLiveWidth` is animating" — was a hypothesis that survived until it was
// measured. So this measures. It renders `TabStripHostView` into a bitmap
// twice, hovered and not, and takes the centroid of what CHANGED around the
// trailing slot: that is the × and nothing else. Then it compares that centroid
// to the rect `region(at:)` resolves from. Nothing here trusts the geometry; it
// reads the image.
//
// The differencing is not incidental. A plain alpha threshold caught the tail
// of the filename beside the glyph and reported the × 4.7–8.1pt left of where
// it is — a different distance per file, which is the only reason it was
// obvious the *measurement* was wrong rather than the drawing. Raising the
// threshold fixed three chips and left the ACTIVE one failing by 7.8pt,
// because an active title is `labelColor` at full alpha. Whatever the strip
// draws next, differencing keeps this honest.
//
//   swiftc -O scripts/tabstrip_host_probe.swift \
//     suisei-app/Suisei/TabStripLayout.swift \
//     suisei-app/Suisei/TabChipMetrics.swift \
//     suisei-app/Suisei/TabStripHost.swift \
//     -o /tmp/tabstrip-host-probe && /tmp/tabstrip-host-probe
//
// The `TabItem` below stands in for `EngineBridge.swift`'s, field for field, so
// the probe does not have to link the engine to ask a geometry question.

import AppKit

struct TabItem: Equatable, Identifiable {
    var id: Int
    var stableId: UInt64
    var title: String
    var dirty: Bool
    var active: Bool
    var group: UInt64 = 0
    var isLayout: Bool = false
    var isTerminal: Bool = false
    var deleted: Bool = false
}

// MARK: - Harness

var failures: [String] = []

func check(_ ok: Bool, _ message: @autoclosure () -> String) {
    if ok {
        print("  ok   \(message())")
    } else {
        print("  FAIL \(message())")
        failures.append(message())
    }
}

func near(_ a: CGFloat, _ b: CGFloat, _ tol: CGFloat) -> Bool { abs(a - b) <= tol }


func makeTabs(_ titles: [String], active: Int = 0) -> [TabItem] {
    titles.enumerated().map { i, t in
        TabItem(
            id: i, stableId: UInt64(i + 1), title: t,
            dirty: i == 1, active: i == active
        )
    }
}

/// A window + strip, sized like the real titlebar band.
func makeStrip(
    contentWidth: CGFloat, tabs: [TabItem],
    leading: CGFloat = 150, trailing: CGFloat = 150
) -> (NSWindow, TabStripHostView) {
    let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: contentWidth, height: 48),
        styleMask: [.titled, .closable, .resizable, .fullSizeContentView],
        backing: .buffered, defer: false
    )
    window.titlebarAppearsTransparent = true
    let view = TabStripHostView(
        frame: NSRect(x: 0, y: 0, width: contentWidth, height: 26)
    )
    view.tabs = tabs
    view.leadingInset = leading
    view.trailingInset = trailing
    window.contentView?.addSubview(view)
    return (window, view)
}

struct Shot {
    var alpha: [UInt8]
    var w: Int
    var h: Int
    var scale: Int
}

/// Render the strip on its own, on transparent black, so alpha IS ink.
func render(_ view: TabStripHostView) -> Shot? {
    guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else {
        return nil
    }
    rep.size = view.bounds.size
    view.cacheDisplay(in: view.bounds, to: rep)
    guard let data = rep.bitmapData else { return nil }
    let scale = max(1, rep.pixelsWide / Int(view.bounds.width.rounded()))
    var alpha = [UInt8](repeating: 0, count: rep.pixelsWide * rep.pixelsHigh)
    for y in 0..<rep.pixelsHigh {
        for x in 0..<rep.pixelsWide {
            alpha[y * rep.pixelsWide + x] = data[y * rep.bytesPerRow + x * 4 + 3]
        }
    }
    return Shot(alpha: alpha, w: rep.pixelsWide, h: rep.pixelsHigh, scale: scale)
}

/// Centroid of what CHANGED between two renders, inside a rect, in points.
///
/// A plain alpha threshold cannot isolate the ×. It is drawn at 0.90, the well
/// under it at 0.16, and an inactive title beside it at `secondaryLabelColor`'s
/// 0.55 — so 0.7 separates them. But an ACTIVE title is `labelColor`, which is
/// 1.0, and the search window catches its last glyph: that is the single
/// remaining failure this replaced, 7.8pt on the one active chip, and it was
/// the measurement, not the drawing.
///
/// Differencing hovered against unhovered leaves only what hovering changes,
/// which in this window is the × appearing (and, on a dirty chip, its dot
/// leaving — centred in the same slot, so it does not bias x). The 0.3 cut
/// drops the hover capsule (0.10) and the well (0.16) and keeps the glyph.
func changedCentroid(_ a: Shot, _ b: Shot, in rect: CGRect) -> CGPoint? {
    guard a.w == b.w, a.h == b.h else { return nil }
    let s = a.scale
    var sumX = 0.0, sumY = 0.0, n = 0.0
    let x0 = max(0, Int(rect.minX.rounded()) * s)
    let x1 = min(a.w, Int(rect.maxX.rounded()) * s)
    let y0 = max(0, Int(rect.minY.rounded()) * s)
    let y1 = min(a.h, Int(rect.maxY.rounded()) * s)
    guard x1 > x0, y1 > y0 else { return nil }
    for py in y0..<y1 {
        for px in x0..<x1 {
            let i = py * a.w + px
            let d = abs(Int(a.alpha[i]) - Int(b.alpha[i]))
            guard CGFloat(d) / 255 > 0.3 else { continue }
            sumX += Double(px); sumY += Double(py); n += 1
        }
    }
    guard n > 0 else { return nil }
    return CGPoint(
        x: CGFloat(sumX / n + 0.5) / CGFloat(s),
        y: CGFloat(sumY / n + 0.5) / CGFloat(s)
    )
}

/// Drive the real hover path rather than poking state: the × only draws when
/// the view believes the pointer is on that chip.
func hover(_ view: TabStripHostView, _ window: NSWindow, at local: CGPoint) {
    let inWindow = view.convert(local, to: nil)
    guard let e = NSEvent.mouseEvent(
        with: .mouseMoved, location: inWindow, modifierFlags: [],
        timestamp: ProcessInfo.processInfo.systemUptime,
        windowNumber: window.windowNumber, context: nil,
        eventNumber: 0, clickCount: 0, pressure: 0
    ) else { return }
    view.mouseMoved(with: e)
}


/// Wait out the insert animation before measuring.
///
/// A chip grows in over 0.14s — `scale(0.94 → 1)` about its own centre — and
/// while that runs the drawn × is up to 2pt inside the rect the hit test
/// answers from. That is a deliberate, bounded exception and this probe found
/// it the moment it was added, which is the point: the invariant this file
/// guards is that draw and hit agree AT REST. Transient decorative transforms
/// are allowed; a resting disagreement is the bug that produced this rewrite.
func settle() {
    Thread.sleep(forTimeInterval: 0.25)
}

/// Where the × is actually drawn for `chip`, measured off the pixels.
///
/// Hovers the chip's BODY, never the × itself, so nothing about where the
/// pointer was put can be what makes the answer agree.
func drawnCloseCentre(
    _ view: TabStripHostView, _ window: NSWindow,
    chip: TabStripLayout.Chip, frame f: TabStripHostView.Frame
) -> CGPoint? {
    hover(view, window, at: CGPoint(x: -50, y: -50))
    guard let quiet = render(view) else { return nil }
    hover(view, window, at: CGPoint(x: f.chipRect(chip).minX + 6, y: f.rowY + 12))
    guard let hovered = render(view) else { return nil }
    return changedCentroid(quiet, hovered, in: f.closeRect(chip).insetBy(dx: -10, dy: -6))
}

@main
enum Probe {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)

        print("\n1. window centring")
        do {
            let tabs = makeTabs(["main.rs", "Cargo.toml", "lib.rs"])
            // Same window, three different left keep-outs. The viewport narrows; the
            // run's centre must not move — that is the difference between "창 기준
            // 중앙" and the old "에디터 기준 중앙".
            var centres: [CGFloat] = []
            for leading in [150.0, 300.0, 420.0] as [CGFloat] {
                let (w, v) = makeStrip(contentWidth: 1200, tabs: tabs, leading: leading)
                guard let f = v.currentFrame() else {
                    check(false, "no frame at leading \(leading)"); continue
                }
                let runCentre = f.viewportX + f.originX + f.layout.contentWidth / 2
                centres.append(v.convert(CGPoint(x: runCentre, y: 0), to: nil).x)
                check(
                    !f.layout.overflow,
                    "leading \(Int(leading)): run fits (viewport \(Int(f.layout.viewportWidth)))"
                )
                _ = w
            }
            let contentCentre = 600.0
            for (i, c) in centres.enumerated() {
                check(
                    near(c, contentCentre, 1),
                    "run centre \(String(format: "%.1f", c)) == window centre 600 (case \(i))"
                )
            }
        }

        // MARK: - 2. The × is drawn where the hit rect is

        print("\n2. drawn × vs hit rect")
        do {
            let titles = ["a.rs", "medium_name.toml", "a_considerably_longer_file.swift", "z.md"]
            let tabs = makeTabs(titles, active: 2)
            let (window, view) = makeStrip(contentWidth: 1200, tabs: tabs)
            settle()
            guard let f = view.currentFrame() else { fatalError("no frame") }

            for chip in f.layout.chips {
                let expected = f.closeRect(chip)
                guard let centroid = drawnCloseCentre(
                    view, window, chip: chip, frame: f
                ) else { check(false, "slot \(chip.slot): no × ink found"); continue }

                check(
                    near(centroid.x, expected.midX, 1.0),
                    "slot \(chip.slot): drawn × x \(String(format: "%.2f", centroid.x)) "
                        + "vs rect \(String(format: "%.2f", expected.midX))"
                )
                check(
                    near(centroid.y, expected.midY, 1.0),
                    "slot \(chip.slot): drawn × y \(String(format: "%.2f", centroid.y)) "
                        + "vs rect \(String(format: "%.2f", expected.midY))"
                )
                check(
                    view.region(at: CGPoint(x: centroid.x, y: centroid.y))
                        == .close(slot: chip.slot),
                    "slot \(chip.slot): the pointer at the drawn × resolves to its own close"
                )
            }
        }

        // MARK: - 3. Regions do not overlap or leave holes inside a chip

        print("\n3. region resolution")
        do {
            let tabs = makeTabs(["one.rs", "two.rs", "three.rs"])
            let (_, view) = makeStrip(contentWidth: 1000, tabs: tabs)
            guard let f = view.currentFrame() else { fatalError("no frame") }

            for chip in f.layout.chips {
                let r = f.chipRect(chip)
                check(
                    view.region(at: CGPoint(x: r.minX + 2, y: r.midY)) == .chip(slot: chip.slot),
                    "slot \(chip.slot): leading edge is its own chip"
                )
                // One point inside the previous chip must never answer this one.
                if chip.slot > 0 {
                    check(
                        view.region(at: CGPoint(x: r.minX - 2, y: r.midY)) != .chip(slot: chip.slot),
                        "slot \(chip.slot): the gap before it is not part of it"
                    )
                }
            }
            check(view.region(at: CGPoint(x: 5, y: 13)) == .empty, "the left reserve is empty")
            check(
                view.region(at: f.plusRect.origin.applying(
                    CGAffineTransform(translationX: 8, y: 8)
                )) == .plus,
                "the + is its own region"
            )
        }

        // MARK: - 4. A press mid-move resolves against where the row IS
        //
        // The defect this whole rewrite is aimed at (H2). The old strip interpolated
        // `originX` inside a SwiftUI `Layout` while the hit test read the settled
        // value, so during every scroll and re-centre the two described different
        // strips. Here one function answers both, so moving the run must move the hit
        // rects with it — including part-way through.

        print("\n4. draw and hit move together")
        do {
            let titles = (1...14).map { "file_\($0).swift" }
            let tabs = makeTabs(titles)
            let (window, view) = makeStrip(contentWidth: 900, tabs: tabs)
            settle()
            guard let f0 = view.currentFrame() else { fatalError("no frame") }
            check(f0.layout.overflow, "fixture overflows (content \(Int(f0.layout.contentWidth)))")

            // Scroll by a real wheel event, then re-measure from scratch.
            guard let scroll = CGEvent(
                scrollWheelEvent2Source: nil, units: .pixel,
                wheelCount: 2, wheel1: 0, wheel2: -120, wheel3: 0
            )?.copy() else { fatalError("no scroll event") }
            scroll.location = CGPoint(x: 400, y: 13)
            if let e = NSEvent(cgEvent: scroll) { view.scrollWheel(with: e) }

            guard let f1 = view.currentFrame() else { fatalError("no frame after scroll") }
            check(f1.originX != f0.originX, "the run moved (\(Int(f0.originX)) → \(Int(f1.originX)))")

            var checked = 0
            for chip in f1.layout.chips {
                let expected = f1.closeRect(chip)
                // Only chips fully inside the viewport are rendered whole.
                let vp = CGRect(
                    x: f1.viewportX, y: 0,
                    width: f1.layout.viewportWidth, height: view.bounds.height
                )
                guard vp.insetBy(dx: 20, dy: 0).contains(expected) else { continue }
                guard let centroid = drawnCloseCentre(
                    view, window, chip: chip, frame: f1
                ) else { check(false, "scrolled slot \(chip.slot): no × ink"); continue }
                check(
                    near(centroid.x, expected.midX, 1.0),
                    "scrolled slot \(chip.slot): drawn × \(String(format: "%.2f", centroid.x)) "
                        + "vs rect \(String(format: "%.2f", expected.midX))"
                )
                check(
                    view.region(at: CGPoint(x: centroid.x, y: centroid.y))
                        == .close(slot: chip.slot),
                    "scrolled slot \(chip.slot): hit follows the drawn glyph"
                )
                checked += 1
            }
            check(checked >= 3, "measured \(checked) scrolled chips")
        }

        // MARK: - 5. The sidebar cannot move the strip
        //
        // H1. The old strip's viewport came from a `GeometryReader` that swept 1120 →
        // 820 → 1120 on every sidebar toggle, so every chip position and every × rect
        // swept with it. Here the width comes from the window, and the only thing a
        // keep-out can do is narrow the viewport.

        print("\n5. a keep-out narrows, it does not shift")
        do {
            let tabs = makeTabs(["main.rs", "Cargo.toml"])
            var xs: [CGFloat] = []
            for leading in stride(from: 150.0, through: 450.0, by: 30.0) {
                let (_, v) = makeStrip(contentWidth: 1200, tabs: tabs, leading: CGFloat(leading))
                guard let f = v.currentFrame(), let first = f.layout.chips.first else { continue }
                xs.append(v.convert(f.chipRect(first).origin, to: nil).x)
            }
            let spread = (xs.max() ?? 0) - (xs.min() ?? 0)
            check(
                spread <= 1,
                "first chip moved \(String(format: "%.2f", spread))pt across 11 keep-out widths"
            )
        }

        // MARK: -

        print("")
        if failures.isEmpty {
            print("tabstrip_host_probe: passed")
            exit(0)
        } else {
            print("tabstrip_host_probe: \(failures.count) FAILED")
            exit(1)
        }

    }
}
