import AppKit
import SwiftUI

// Does a .navigation toolbar item land OVER the sidebar (where the system's
// sidebar toggle was), or after it in the detail's leading area?

struct Probe: View {
    @State private var v: NavigationSplitViewVisibility = .all
    var body: some View {
        NavigationSplitView(columnVisibility: $v) {
            List { ForEach(0..<5, id: \.self) { Text("row \($0)") } }
                .listStyle(.sidebar)
                .navigationSplitViewColumnWidth(min: 240, ideal: 300, max: 460)
                // Declared ON THE COLUMN, so it joins the column's own section.
                .toolbar {
                    ToolbarItem(placement: .automatic) {
                        Button { } label: { Image(systemName: "list.bullet") }
                    }
                }
        } detail: { Color.clear }
        .navigationSplitViewStyle(.balanced)
        .navigationTitle("")
        .toolbar(removing: .sidebarToggle)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { } label: { Image(systemName: "magnifyingglass") }
            }
        }
    }
}

func scan(_ v: NSView, into out: inout [(String, NSRect)]) {
    let n = String(describing: type(of: v))
    if n == "NSToolbarItemViewer" || n == "NSToolbarPlatterView" {
        out.append((n, v.convert(v.bounds, to: nil)))
    }
    for s in v.subviews { scan(s, into: &out) }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let c = NSHostingController(rootView: Probe())
let w = NSWindow(contentViewController: c)
w.setFrame(NSRect(x: -8000, y: -8000, width: 1280, height: 820), display: true)
w.styleMask.insert([.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView])
w.titlebarAppearsTransparent = true
w.titlebarSeparatorStyle = .none
w.orderFront(nil)
RunLoop.main.run(until: Date().addingTimeInterval(1.2))
w.contentView?.layoutSubtreeIfNeeded()
RunLoop.main.run(until: Date().addingTimeInterval(0.5))
var out: [(String, NSRect)] = []
if let f = w.contentView?.superview { scan(f, into: &out) }
print("\n  sidebar column is 0 … 300; window 1280")
for (n, f) in out.sorted(by: { $0.1.minX < $1.1.minX }) {
    print(String(format: "  %-20@ x %.0f … %.0f  %@", n as NSString, f.minX, f.maxX,
                 (f.minX < 300 ? "OVER THE SIDEBAR" : "in the detail area") as NSString))
}
print("")
exit(0)
