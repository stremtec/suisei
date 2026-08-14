import AppKit
import SwiftUI

// Does the toolbar's glass platter ANIMATE when its item set changes?
//
// The viewer's zoom group and info button come and go with an image or a PDF,
// and the platter jumps to its new width in one frame. This measures the four
// ways there are to change that width, sampling `NSToolbarPlatterView.frame`
// every frame for 0.6s after each toggle.
//
//   swiftc -O -parse-as-library scripts/toolbar_grow_probe.swift -o /tmp/p && /tmp/p
//
// MEASURED, macOS 26:
//
//   lever                                        distinct widths / 0.6s
//   ------------------------------------------   ----------------------
//   1  SwiftUI conditional ToolbarItem            1     75 → 227
//   2  one ToolbarItem, animated SwiftUI content  1    112 → 242
//   3  NSToolbar.insertItem / removeItem          1    104 → 248
//   4  AppKit view ramping intrinsicContentSize  17    112 → 121 → 157
//                                                      → 181 → 195 → 206
//
// So AppKit does not animate a platter when the ITEM SET changes — not from
// SwiftUI, and not through its own insert/remove API either. It does animate
// when an item's own intrinsic width changes, because that is an ordinary
// constraint-driven relayout.
//
// The catch, also measured: an item present at intrinsic width 0 still costs
// the platter 37pt (112 vs 75 for no item at all). Lever 4 therefore only
// removes the step if the trailing controls become ONE hosted item whose width
// ramps — which trades away the per-item treatment that is the reason
// `editorToolbar` uses real toolbar items at all. Left here as the record
// behind that trade-off rather than as something the app does.

enum Lever: Int {
    case conditionalItems = 1
    case animatedContent = 2
    case intrinsicRamp = 4

    /// Change to measure a different lever. Lever 3 is AppKit-only and lives
    /// in the comment above rather than in this SwiftUI harness.
    static let active: Lever = .intrinsicRamp
}

// MARK: - Lever 4's ramping view

final class RampView: NSView {
    var target: CGFloat = 0 { didSet { if target != oldValue { animate() } } }
    private var current: CGFloat = 0
    private var link: CADisplayLink?
    private var start: CFTimeInterval = 0
    private var from: CGFloat = 0

    override var intrinsicContentSize: NSSize { NSSize(width: current, height: 24) }

    private func animate() {
        from = current
        start = CACurrentMediaTime()
        if link == nil {
            let l = displayLink(target: self, selector: #selector(step))
            l.add(to: .main, forMode: .common)
            link = l
        }
        link?.isPaused = false
    }

    @objc private func step() {
        let t = min(1, (CACurrentMediaTime() - start) / 0.35)
        current = from + (target - from) * CGFloat(1 - pow(1 - t, 3))
        invalidateIntrinsicContentSize()
        if t >= 1 { link?.isPaused = true }
    }
}

struct WidthRamp: NSViewRepresentable {
    var wide: Bool
    func makeNSView(context: Context) -> RampView {
        let v = RampView()
        v.target = wide ? 130 : 0
        return v
    }
    func updateNSView(_ v: RampView, context: Context) { v.target = wide ? 130 : 0 }
}

// MARK: - Harness

struct Probe: View {
    @State private var extended = false

    var body: some View {
        Color.clear
            .frame(width: 700, height: 300)
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button {} label: { Image(systemName: "magnifyingglass") }
                }

                if Lever.active == .conditionalItems, extended {
                    ToolbarItemGroup(placement: .primaryAction) {
                        Button {} label: { Image(systemName: "minus.magnifyingglass") }
                        Text("100%").frame(minWidth: 38)
                        Button {} label: { Image(systemName: "plus.magnifyingglass") }
                    }
                    ToolbarItem(placement: .primaryAction) {
                        Button {} label: { Image(systemName: "info.circle") }
                    }
                }

                if Lever.active == .animatedContent {
                    ToolbarItem(placement: .primaryAction) {
                        HStack(spacing: 6) {
                            if extended {
                                Button {} label: { Image(systemName: "minus.magnifyingglass") }
                                Text("100%").frame(minWidth: 38)
                                Button {} label: { Image(systemName: "plus.magnifyingglass") }
                                Button {} label: { Image(systemName: "info.circle") }
                            }
                        }
                        .fixedSize()
                        .animation(.snappy(duration: 0.35), value: extended)
                    }
                }

                if Lever.active == .intrinsicRamp {
                    ToolbarItem(placement: .primaryAction) { WidthRamp(wide: extended) }
                }

                ToolbarItem(placement: .primaryAction) {
                    Button {} label: { Image(systemName: "sidebar.right") }
                }
            }
            .onAppear {
                Driver.shared.start { v in
                    withAnimation(.snappy(duration: 0.35)) { extended = v }
                }
            }
    }
}

func platterWidth(_ window: NSWindow) -> CGFloat {
    var w: CGFloat = 0
    func walk(_ v: NSView) {
        if String(describing: type(of: v)).contains("ToolbarPlatterView") {
            w = max(w, v.frame.width)
        }
        v.subviews.forEach(walk)
    }
    if let themeFrame = window.contentView?.superview { walk(themeFrame) }
    return w
}

final class Driver {
    static let shared = Driver()
    private var setExtended: ((Bool) -> Void)?

    func start(_ set: @escaping (Bool) -> Void) {
        guard setExtended == nil else { return }
        setExtended = set
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { self.phase(grow: true) }
    }

    private func phase(grow: Bool) {
        guard let window = NSApp.windows.first(where: { $0.toolbar != nil }) else {
            print("no toolbar window"); NSApp.terminate(nil); return
        }
        print("\n=== lever \(Lever.active.rawValue) · \(grow ? "GROW" : "SHRINK") ===")
        print(String(format: "before: %.1f", platterWidth(window)))
        setExtended?(grow)

        var widths: [CGFloat] = []
        var n = 0
        Timer.scheduledTimer(withTimeInterval: 1.0 / 60, repeats: true) { timer in
            n += 1
            widths.append(platterWidth(window))
            guard n >= 36 else { return }
            timer.invalidate()
            for (i, w) in widths.enumerated() where i % 3 == 0 {
                print(String(format: "  t=%.3f  %.1f", Double(i) / 60, w))
            }
            print("  distinct widths over 0.6s: \(Set(widths).count)")
            if grow {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { self.phase(grow: false) }
            } else {
                NSApp.terminate(nil)
            }
        }
    }
}

@main struct ProbeApp: App {
    @NSApplicationDelegateAdaptor(AD.self) var ad
    var body: some Scene { WindowGroup { Probe() } }
}

final class AD: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ n: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }
}
