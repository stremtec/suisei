import AppKit
import SwiftUI

// Faithful stand-in for ContentView.detailStack: does adding a .toolbar shift
// the top band down?

struct Marker: NSViewRepresentable {
    let label: String
    func makeNSView(context: Context) -> NSView {
        let v = NSView(frame: .zero)
        v.identifier = NSUserInterfaceItemIdentifier("probe.\(label)")
        return v
    }
    func updateNSView(_ v: NSView, context: Context) {}
}

struct DetailStack: View {
    var body: some View {
        ZStack(alignment: .top) {
            VStack(spacing: 0) {                       // status bar layer
                Spacer()
                Marker(label: "status").frame(height: 24)
            }
            VStack(spacing: 0) {                       // columns
                HStack(spacing: 0) {
                    VStack(spacing: 0) {
                        Spacer().frame(height: 48)
                        Marker(label: "editorCard")
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                        Spacer().frame(height: 24)
                    }
                }
            }
            Marker(label: "topBar")                    // titlebar row
                .frame(height: 48)
                .frame(maxWidth: .infinity)
                .zIndex(2)
        }
        .ignoresSafeArea(.container, edges: .top)
    }
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
            DetailStack()
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

    var hits: [(String, NSRect)] = []
    if let frame = window.contentView?.superview { find(frame, prefix: "probe.", into: &hits) }
    let winH = window.frame.height
    print("\n== \(name) ==  toolbar=\(window.toolbar != nil)")
    if let close = window.standardWindowButton(.closeButton) {
        let f = close.convert(close.bounds, to: nil)
        print(String(format: "  lights centre  %.1f from window top", winH - f.midY))
    }
    for (id, f) in hits.sorted(by: { $0.0 < $1.0 }) {
        print(String(format: "  %-11@ top %.1f  bottom %.1f  x %.0f w %.0f",
                     id.replacingOccurrences(of: "probe.", with: "") as NSString,
                     winH - f.maxY, winH - f.minY, f.origin.x, f.width))
    }
    window.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
run("A  no toolbar", withToolbar: false)
run("B  with toolbar", withToolbar: true)
exit(0)
