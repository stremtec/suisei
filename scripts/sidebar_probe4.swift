import AppKit
import SwiftUI

// Does adding a .toolbar move the sidebar's content down?
// Same window configuration the editor has, with and without toolbar items.

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
    let withToolbar: Bool

    var body: some View {
        NavigationSplitView {
            VStack(spacing: 0) {
                Marker(label: "strip").frame(height: 40)
                Spacer()
            }
            .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 460)
        } detail: {
            VStack(spacing: 0) {
                Marker(label: "detailTop").frame(height: 1)
                Spacer()
            }
            .ignoresSafeArea(.container, edges: .top)
        }
        .navigationSplitViewStyle(.balanced)
        .toolbar(removing: .title)
        .toolbar {
            if withToolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button { } label: { Image(systemName: "magnifyingglass") }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button { } label: { Image(systemName: "gearshape") }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button { } label: { Image(systemName: "sidebar.right") }
                }
            }
        }
    }
}

func find(_ view: NSView, prefix: String, into out: inout [(String, NSRect)]) {
    if let id = view.identifier?.rawValue, id.hasPrefix(prefix) {
        out.append((id, view.convert(view.bounds, to: nil)))
    }
    for sub in view.subviews { find(sub, prefix: prefix, into: &out) }
}

func findToolbarItems(_ view: NSView, depth: Int, into out: inout [String]) {
    let n = String(describing: type(of: view))
    if n.contains("Toolbar") || n.contains("Glass") {
        let f = view.convert(view.bounds, to: nil)
        out.append(String(format: "%@%@ (%.0f,%.0f %.0fx%.0f)",
                          String(repeating: "  ", count: depth), n,
                          f.origin.x, f.origin.y, f.width, f.height))
    }
    for sub in view.subviews { findToolbarItems(sub, depth: depth + 1, into: &out) }
}

func run(_ name: String, withToolbar: Bool) {
    let controller = NSHostingController(rootView: Probe(withToolbar: withToolbar))
    let window = NSWindow(contentViewController: controller)
    window.setFrame(NSRect(x: -8000, y: -8000, width: 1000, height: 700), display: true)
    window.styleMask.insert([.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView])
    window.titlebarAppearsTransparent = true
    window.titleVisibility = .hidden
    window.titlebarSeparatorStyle = .none
    window.isOpaque = true
    window.backgroundColor = .windowBackgroundColor
    window.orderFront(nil)
    RunLoop.main.run(until: Date().addingTimeInterval(1.0))
    window.contentView?.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.5))

    var hits: [(String, NSRect)] = []
    if let frame = window.contentView?.superview { find(frame, prefix: "probe.", into: &hits) }
    let winH = window.frame.height
    print("\n== \(name) ==")
    print("  toolbar present: \(window.toolbar != nil)  style=\(window.toolbarStyle.rawValue)")
    if let close = window.standardWindowButton(.closeButton) {
        let f = close.convert(close.bounds, to: nil)
        print(String(format: "  lights centre    %.1f from window top", winH - f.midY))
    }
    for (id, f) in hits.sorted(by: { $0.0 < $1.0 }) {
        print(String(format: "  %-12@ top-inset %.1f  x %.1f",
                     id.replacingOccurrences(of: "probe.", with: "") as NSString,
                     winH - f.maxY, f.origin.x))
    }
    var tb: [String] = []
    if let frame = window.contentView?.superview { findToolbarItems(frame, depth: 0, into: &tb) }
    for line in tb.prefix(14) { print("    " + line) }
    window.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
run("A  no toolbar items", withToolbar: false)
run("B  three primaryAction items", withToolbar: true)
exit(0)
