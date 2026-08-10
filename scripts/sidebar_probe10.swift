import AppKit
import SwiftUI

// The Git workbench's toolbar sits at the window's trailing edge. The editor's
// does not. Replicate the workbench's declaration exactly, then remove one
// factor at a time until the items move — that names the cause.
//
// Differences between the two, from the source:
//   workbench: .navigationTitle("Source Control") + .toolbarTitleDisplayMode(.inline)
//              window: titlebarAppearsTransparent, NO titleVisibility change,
//                      NO fullSizeContentView
//              last item is a 280pt search field
//   editor:    .toolbar(removing: .title)
//              window: + titleVisibility = .hidden, + fullSizeContentView
//              no wide item

struct Config {
    var name: String
    var title: Bool = true          // .navigationTitle + inline display mode
    var removeTitleItem: Bool = false
    var wideLastItem: Bool = true   // the 280pt search field
    var hideWindowTitle: Bool = false
    var fullSizeContent: Bool = false
}

struct Probe: View {
    let cfg: Config
    @State private var visibility: NavigationSplitViewVisibility = .all

    var body: some View {
        NavigationSplitView(columnVisibility: $visibility) {
            VStack(spacing: 0) {
                HStack { ForEach(0..<2, id: \.self) { _ in Image(systemName: "folder") } }
                    .frame(height: 28)
                Divider()
                List { ForEach(0..<6, id: \.self) { Text("row \($0)") } }
                    .listStyle(.sidebar)
                    .scrollContentBackground(.hidden)
            }
            .navigationSplitViewColumnWidth(min: 280, ideal: 294, max: 350)
        } detail: {
            Color.clear
        }
        .navigationSplitViewStyle(.balanced)
        .navigationTitle(cfg.title ? "Source Control" : "")
        .toolbarTitleDisplayMode(.inline)
        .modifier(RemoveTitle(on: cfg.removeTitleItem))
        .toolbar { bar }
    }

    @ToolbarContentBuilder private var bar: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            Button { } label: { Image(systemName: "square.and.pencil") }
        }
        ToolbarItem(placement: .primaryAction) {
            Button { } label: { Image(systemName: "folder") }
        }
        ToolbarItem(placement: .primaryAction) {
            Button { } label: { Image(systemName: "arrow.clockwise") }
        }
        if cfg.wideLastItem {
            ToolbarItem(placement: .primaryAction) {
                TextField("Filter", text: .constant(""))
                    .frame(width: 280, height: 30)
            }
        }
    }
}

struct RemoveTitle: ViewModifier {
    let on: Bool
    func body(content: Content) -> some View {
        if on { content.toolbar(removing: .title) } else { content }
    }
}

func scan(_ view: NSView, into out: inout [(String, NSRect)]) {
    let n = String(describing: type(of: view))
    if n == "NSToolbarItemViewer" || n == "NSToolbarPlatterView" {
        out.append((n, view.convert(view.bounds, to: nil)))
    }
    for sub in view.subviews { scan(sub, into: &out) }
}

func run(_ cfg: Config) {
    let controller = NSHostingController(rootView: Probe(cfg: cfg))
    let window = NSWindow(contentViewController: controller)
    window.setFrame(NSRect(x: -8000, y: -8000, width: 1280, height: 820), display: true)
    // applyThemedTitlebar, verbatim.
    window.backgroundColor = .windowBackgroundColor
    window.isOpaque = true
    window.titlebarAppearsTransparent = true
    window.styleMask.insert([.titled, .closable, .miniaturizable, .resizable])
    window.titlebarSeparatorStyle = .none
    window.isMovableByWindowBackground = false
    // The editor's extras.
    if cfg.hideWindowTitle { window.titleVisibility = .hidden }
    if cfg.fullSizeContent { window.styleMask.insert(.fullSizeContentView) }

    window.orderFront(nil)
    RunLoop.main.run(until: Date().addingTimeInterval(1.1))
    window.contentView?.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.5))

    var hits: [(String, NSRect)] = []
    if let frame = window.contentView?.superview { scan(frame, into: &hits) }
    let platter = hits.first { $0.0 == "NSToolbarPlatterView" }
    let rightmost = hits.map(\.1.maxX).max() ?? 0
    print(String(format: "  %-42@ platter %@   rightmost item ends at %.0f  (gap %.0f)",
                 cfg.name as NSString,
                 platter.map { String(format: "x %.0f … %.0f", $0.1.minX, $0.1.maxX) } ?? "none",
                 rightmost, 1280 - rightmost))
    print("       window.title = \"\(window.title)\"")
    window.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
print("\nwindow width 1280 — a trailing toolbar ends near 1280\n")
run(Config(name: "W   workbench, verbatim"))
run(Config(name: "W1  … minus the 280pt search field", wideLastItem: false))
run(Config(name: "W2  … minus navigationTitle", title: false))
run(Config(name: "W3  … plus toolbar(removing: .title)", removeTitleItem: true))
run(Config(name: "W4  … plus titleVisibility = .hidden", hideWindowTitle: true))
run(Config(name: "W5  … plus fullSizeContentView", fullSizeContent: true))
run(Config(name: "E   editor config (all three extras)",
           title: false, removeTitleItem: true, wideLastItem: false,
           hideWindowTitle: true, fullSizeContent: true))
// The proposed fix: empty title kept as the anchor, nothing removed or hidden.
run(Config(name: "F   editor + empty title, nothing removed",
           title: false, removeTitleItem: false, wideLastItem: false,
           hideWindowTitle: false, fullSizeContent: true))
print("")
exit(0)
