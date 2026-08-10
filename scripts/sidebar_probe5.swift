import AppKit
import SwiftUI

// With a real NSToolbar present, who receives a click in the middle of the
// top band — where the editor's tab strip lives?

final class TabProbeView: NSView {
    override func hitTest(_ point: NSPoint) -> NSView? {
        let hit = super.hitTest(point)
        return hit
    }
}

struct TabMarker: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let v = TabProbeView(frame: .zero)
        v.identifier = NSUserInterfaceItemIdentifier("probe.tabstrip")
        return v
    }
    func updateNSView(_ v: NSView, context: Context) {}
}

struct Probe: View {
    let withToolbar: Bool
    var body: some View {
        NavigationSplitView {
            Color.clear.navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 460)
        } detail: {
            ZStack(alignment: .top) {
                Color.clear
                TabMarker().frame(width: 300, height: 48)
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

func find(_ view: NSView, id: String) -> NSView? {
    if view.identifier?.rawValue == id { return view }
    for sub in view.subviews { if let f = find(sub, id: id) { return f } }
    return nil
}

func chain(_ view: NSView?) -> String {
    var names: [String] = []
    var v = view
    while let cur = v, names.count < 6 {
        names.append(String(describing: type(of: cur)))
        v = cur.superview
    }
    return names.joined(separator: " ← ")
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
    window.orderFront(nil)
    RunLoop.main.run(until: Date().addingTimeInterval(1.0))
    window.contentView?.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.5))

    print("\n== \(name) ==  toolbar=\(window.toolbar != nil)")
    guard let frameView = window.contentView?.superview else { return }
    guard let marker = find(frameView, id: "probe.tabstrip") else {
        print("  marker not found"); return
    }
    let mf = marker.convert(marker.bounds, to: nil)
    print(String(format: "  tab marker in window coords: (%.0f,%.0f %.0fx%.0f)",
                 mf.origin.x, mf.origin.y, mf.width, mf.height))

    // Sample the marker's own centre, in window coordinates.
    let p = NSPoint(x: mf.midX, y: mf.midY)
    let hit = frameView.hitTest(frameView.convert(p, from: nil))
    print("  hitTest at marker centre → \(chain(hit))")
    let reachesContent = hit === marker || hit?.isDescendant(of: window.contentView!) == true
    print("  reaches the CONTENT view: \(reachesContent)")
    window.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
run("A  no toolbar", withToolbar: false)
run("B  with toolbar", withToolbar: true)
exit(0)
