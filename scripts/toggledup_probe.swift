import AppKit
import SwiftUI

// Two toggles appeared. What items does the toolbar actually contain, and does
// .toolbar(removing: .sidebarToggle) take effect where it is written?

struct Probe: View {
    let removeOnSplit: Bool
    let removeOnColumn: Bool
    @State private var v: NavigationSplitViewVisibility = .all

    var body: some View {
        NavigationSplitView(columnVisibility: $v) {
            VStack(spacing: 0) {
                Color.clear.frame(height: 40)
                List { ForEach(0..<5, id: \.self) { Text("row \($0)") } }
                    .listStyle(.sidebar)
            }
            .navigationSplitViewColumnWidth(min: 240, ideal: 300, max: 460)
            .modifier(Strip(on: removeOnColumn))
            .toolbar {
                ToolbarItem(placement: .automatic) {
                    Button { } label: { Image(systemName: "list.bullet") }
                }
            }
        } detail: { Color.clear }
        .navigationSplitViewStyle(.balanced)
        .navigationTitle("")
        .modifier(Strip(on: removeOnSplit))
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
    }
}

struct Strip: ViewModifier {
    let on: Bool
    func body(content: Content) -> some View {
        if on { content.toolbar(removing: .sidebarToggle) } else { content }
    }
}

func scan(_ v: NSView, into out: inout [NSRect]) {
    if String(describing: type(of: v)) == "NSToolbarItemViewer" {
        out.append(v.convert(v.bounds, to: nil))
    }
    for s in v.subviews { scan(s, into: &out) }
}

func run(_ label: String, split: Bool, column: Bool, width: CGFloat = 1280) {
    let c = NSHostingController(rootView: Probe(removeOnSplit: split, removeOnColumn: column))
    let w = NSWindow(contentViewController: c)
    w.setFrame(NSRect(x: -8000, y: -8000, width: width, height: 820), display: true)
    w.styleMask.insert([.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView])
    w.titlebarAppearsTransparent = true
    w.titlebarSeparatorStyle = .none
    w.orderFront(nil)
    RunLoop.main.run(until: Date().addingTimeInterval(1.1))
    w.contentView?.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.5))
    var out: [NSRect] = []
    if let f = w.contentView?.superview { scan(f, into: &out) }
    let overSidebar = out.filter { $0.minX < 300 }.sorted { $0.minX < $1.minX }
    print("\n  \(label)")
    print("     items over the sidebar: \(overSidebar.count)")
    for f in overSidebar { print(String(format: "       x %.0f … %.0f", f.minX, f.maxX)) }
    print("     identifiers: \(w.toolbar?.items.map(\.itemIdentifier.rawValue) ?? [])")
    w.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
run("1280pt wide", split: true, column: false, width: 1280)
run(" 900pt wide", split: true, column: false, width: 900)
run(" 700pt wide", split: true, column: false, width: 700)
run(" 620pt wide", split: true, column: false, width: 620)
print("")
exit(0)
