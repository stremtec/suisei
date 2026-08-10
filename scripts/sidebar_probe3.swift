import AppKit
import SwiftUI

// Where does sidebar content actually land, measured from the WINDOW top?
// Same window configuration the editor now has.

struct Marker: NSViewRepresentable {
    let label: String
    func makeNSView(context: Context) -> NSView {
        let v = NSView(frame: .zero)
        v.identifier = NSUserInterfaceItemIdentifier("probe.\(label)")
        return v
    }
    func updateNSView(_ v: NSView, context: Context) {}
}

struct Probe: View {
    let ignoreTop: Bool
    let topSpacer: CGFloat

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            VStack(spacing: 0) {
                Marker(label: "detailTop").frame(height: 1)
                Spacer()
            }
            .ignoresSafeArea(.container, edges: .top)
        }
        .navigationSplitViewStyle(.balanced)
    }

    @ViewBuilder private var sidebar: some View {
        let content = VStack(spacing: 0) {
            Spacer().frame(height: topSpacer)
            Marker(label: "strip").frame(height: 40)
            Spacer()
        }
        .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 460)

        if ignoreTop {
            content.ignoresSafeArea(.container, edges: .top)
        } else {
            content
        }
    }
}

func find(_ view: NSView, prefix: String, into out: inout [(String, NSRect)]) {
    if let id = view.identifier?.rawValue, id.hasPrefix(prefix) {
        out.append((id, view.convert(view.bounds, to: nil)))
    }
    for sub in view.subviews { find(sub, prefix: prefix, into: &out) }
}

func run(_ name: String, ignoreTop: Bool, topSpacer: CGFloat) {
    let controller = NSHostingController(rootView: Probe(ignoreTop: ignoreTop, topSpacer: topSpacer))
    let window = NSWindow(contentViewController: controller)
    window.setFrame(NSRect(x: -8000, y: -8000, width: 1000, height: 700), display: true)
    window.styleMask.insert([.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView])
    window.titlebarAppearsTransparent = true
    window.titleVisibility = .hidden
    window.titlebarSeparatorStyle = .none
    window.isOpaque = true
    window.backgroundColor = .windowBackgroundColor
    window.orderFront(nil)
    RunLoop.main.run(until: Date().addingTimeInterval(0.9))
    window.contentView?.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.4))

    var hits: [(String, NSRect)] = []
    if let frame = window.contentView?.superview { find(frame, prefix: "probe.", into: &hits) }

    // Window coordinates are bottom-up; report distance from the WINDOW TOP.
    let winH = window.frame.height
    print("\n== \(name) ==  window height \(Int(winH))")
    if let title = window.standardWindowButton(.closeButton) {
        let f = title.convert(title.bounds, to: nil)
        print(String(format: "  close button      top-inset %.1f  (centre %.1f)",
                     winH - f.maxY, winH - f.midY))
    }
    for (id, f) in hits {
        print(String(format: "  %-18@ top-inset %.1f  height %.1f  x %.1f width %.1f",
                     id.replacingOccurrences(of: "probe.", with: "") as NSString,
                     winH - f.maxY, f.height, f.origin.x, f.width))
    }
    if hits.isEmpty { print("  (markers not found)") }
    window.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
run("A  sidebar respects safe area, 35pt spacer", ignoreTop: false, topSpacer: 35)
run("B  sidebar respects safe area, 0pt spacer", ignoreTop: false, topSpacer: 0)
run("C  sidebar ignores top safe area, 35pt spacer", ignoreTop: true, topSpacer: 35)
run("D  sidebar ignores top safe area, 0pt spacer", ignoreTop: true, topSpacer: 0)
exit(0)
