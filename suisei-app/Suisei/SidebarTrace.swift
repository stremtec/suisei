import AppKit
import SwiftUI

/// Frame-accurate geometry for the Git workbench's sidebar transition.
///
/// The first attempt at this measured the wrong layer. A `GeometryReader` in
/// the sidebar's background reports what SwiftUI LAID OUT, and the trace it
/// produced showed the column's width never moving at all:
///
///     w=280.0 y=52.0 safeTop=44.0 row1=103.0
///     w=280.0 y=44.0 safeTop=44.0 row1=103.0
///
/// That is not a broken sidebar, it is a probe watching the wrong thing.
/// `NavigationSplitView` is `NSSplitViewController` underneath, and collapsing
/// a pane is an AppKit animation on the split view's own layers. SwiftUI sees
/// the start state and the end state; the frames in between belong to Core
/// Animation and never reach a `GeometryReader` at all.
///
/// So this samples `layer.presentation()` — what is actually on screen this
/// refresh — for the split view's arranged subviews and for the sidebar's own
/// host. A pop partway through has to appear in one of those as a step between
/// consecutive frames, and whichever one steps is the one to fix.
///
/// Entirely inert unless `SUISEI_DIAG=sidebar`.
struct SidebarPresentationTrace: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        EditorDiagnostics.sidebar ? SidebarTraceView() : NSView()
    }

    func updateNSView(_ nsView: NSView, context: Context) {}
}

private final class SidebarTraceView: NSView {
    private var link: CADisplayLink?
    private var last = ""
    /// Frames whose geometry matched the one before it. Reported with the next
    /// change so a step's duration is visible: "held for 7 frames, then moved
    /// 8pt" is a different bug from "moved 1pt every frame".
    private var still = 0

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        link?.invalidate()
        link = nil
        guard let window else { return }
        let created = window.displayLink(target: self, selector: #selector(tick))
        created.preferredFrameRateRange = CAFrameRateRange(
            minimum: 60,
            maximum: 120,
            preferred: 120
        )
        created.add(to: .main, forMode: .common)
        link = created
    }

    deinit {
        link?.invalidate()
    }

    /// Nearest enclosing split view. Walking up rather than being handed it:
    /// the view hierarchy between a SwiftUI background and AppKit's split view
    /// is SwiftUI's business and it changes between releases.
    private var splitView: NSSplitView? {
        var v: NSView? = superview
        while let current = v {
            if let split = current as? NSSplitView { return split }
            v = current.superview
        }
        return nil
    }

    @objc private func tick() {
        guard let split = splitView else { return }

        // The presentation layer is what the compositor is showing right now.
        // `frame` would report the model value, which is where the animation
        // is going, not where it is.
        func shown(_ view: NSView) -> CGRect {
            (view.layer?.presentation()?.frame).map { view.superview?.layer?.convert($0, to: nil) ?? $0 }
                ?? view.frame
        }

        var parts: [String] = []
        for (i, pane) in split.arrangedSubviews.enumerated() {
            let f = shown(pane)
            parts.append(
                "pane\(i)=[x\(fmt(f.minX)) y\(fmt(f.minY)) w\(fmt(f.width)) h\(fmt(f.height))]"
            )
        }
        let mine = shown(self)
        parts.append("host=[y\(fmt(mine.minY)) h\(fmt(mine.height))]")

        let line = parts.joined(separator: " ")
        guard line != last else {
            still += 1
            return
        }
        let held = still
        still = 0
        last = line
        NSLog("[suisei/sidebar] \(held > 0 ? "(+\(held) still) " : "")\(line)")
    }

    private func fmt(_ v: CGFloat) -> String {
        String(format: "%.0f", v)
    }
}
