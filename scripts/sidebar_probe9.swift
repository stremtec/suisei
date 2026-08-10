import AppKit
import SwiftUI

// Can a flexible space be pushed into SwiftUI's NSToolbar from AppKit, and does
// it actually move the items? Declarative ToolbarSpacer measured correct twice
// and did not hold in the app, so this tests the mechanism instead of the hint.

struct Probe: View {
    @State private var visibility: NavigationSplitViewVisibility = .all
    var body: some View {
        NavigationSplitView(columnVisibility: $visibility) {
            VStack(spacing: 0) {
                HStack { ForEach(0..<5, id: \.self) { _ in Image(systemName: "folder") } }
                    .frame(height: 40)
                List { ForEach(0..<6, id: \.self) { Text("row \($0)") } }
                    .listStyle(.sidebar)
                    .scrollContentBackground(.hidden)
            }
            .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 460)
        } detail: {
            ZStack(alignment: .top) {
                VStack(spacing: 0) { Spacer(); Color.clear.frame(height: 24) }
                Color.clear.frame(height: 48).frame(maxWidth: .infinity).zIndex(2)
            }
            .ignoresSafeArea(.container, edges: .top)
        }
        .navigationSplitViewStyle(.balanced)
        .toolbar(removing: .title)
        .toolbar {
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
        .frame(minWidth: 640, minHeight: 400)
        .preferredColorScheme(.dark)
        .background(Color(nsColor: .windowBackgroundColor).ignoresSafeArea())
    }
}

func scan(_ view: NSView, into out: inout [(String, NSRect)]) {
    let n = String(describing: type(of: view))
    if n == "NSToolbarItemViewer" || n == "NSToolbarPlatterView" {
        out.append((n, view.convert(view.bounds, to: nil)))
    }
    for sub in view.subviews { scan(sub, into: &out) }
}

func report(_ label: String, _ window: NSWindow) {
    var hits: [(String, NSRect)] = []
    if let frame = window.contentView?.superview { scan(frame, into: &hits) }
    print("  -- \(label) --")
    print("     items: \(window.toolbar?.items.map(\.itemIdentifier.rawValue) ?? [])")
    for (n, f) in hits.sorted(by: { $0.1.minX < $1.1.minX }) {
        print(String(format: "     %-20@ x %.0f … %.0f   (trailing gap %.0f)",
                     n as NSString, f.minX, f.maxX, 1280 - f.maxX))
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

let controller = NSHostingController(rootView: Probe())
let window = NSWindow(contentViewController: controller)
window.setFrame(NSRect(x: -8000, y: -8000, width: 1280, height: 820), display: true)
window.styleMask.insert([.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView])
window.titlebarAppearsTransparent = true
window.titleVisibility = .hidden
window.titlebarSeparatorStyle = .none
window.isOpaque = true
window.orderFront(nil)
RunLoop.main.run(until: Date().addingTimeInterval(1.2))
window.contentView?.layoutSubtreeIfNeeded()
RunLoop.main.run(until: Date().addingTimeInterval(0.6))

print("\n== SwiftUI toolbar, untouched ==")
report("before", window)

// Insert a flexible space at the front, the way NSToolbar itself allows.
if let toolbar = window.toolbar {
    let ids = toolbar.items.map(\.itemIdentifier)
    print("\n  inserting .flexibleSpace at 0 (current first = \(ids.first?.rawValue ?? "nil"))")
    toolbar.insertItem(withItemIdentifier: .flexibleSpace, at: 0)
}
RunLoop.main.run(until: Date().addingTimeInterval(0.6))
window.contentView?.layoutSubtreeIfNeeded()
RunLoop.main.run(until: Date().addingTimeInterval(0.4))
report("after flexibleSpace at 0", window)

// And again, to prove idempotence guarding is needed.
if let toolbar = window.toolbar {
    print("\n  identifiers now: \(toolbar.items.map(\.itemIdentifier.rawValue))")
}
exit(0)
