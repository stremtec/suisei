import AppKit
import SwiftUI

// Faithful mirror of ContentView's actual chain: a sidebar with real content
// (so the automatic sidebar toggle exists), a ZStack detail that ignores the
// top safe area, and the same modifier order after the split view.

struct Probe: View {
    let useSpacer: Bool
    let removeSidebarToggle: Bool
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
                VStack(spacing: 0) { HStack(spacing: 0) { Color.clear } }
                Color.clear.frame(height: 48).frame(maxWidth: .infinity).zIndex(2)
            }
            .ignoresSafeArea(.container, edges: .top)
        }
        .navigationSplitViewStyle(.balanced)
        .toolbar(removing: .title)
        .modifier(SidebarToggleStrip(strip: removeSidebarToggle))
        .toolbar { bar }
        .frame(minWidth: 640, minHeight: 400)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .foregroundStyle(.primary)
        .tint(.blue)
        .font(.system(size: 13, weight: .regular))
        .preferredColorScheme(.dark)
        .background(Color(nsColor: .windowBackgroundColor).ignoresSafeArea())
    }

    @ToolbarContentBuilder private var bar: some ToolbarContent {
        if useSpacer { ToolbarSpacer(.flexible) }
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

struct SidebarToggleStrip: ViewModifier {
    let strip: Bool
    func body(content: Content) -> some View {
        if strip { content.toolbar(removing: .sidebarToggle) } else { content }
    }
}

func scan(_ view: NSView, into out: inout [(String, NSRect)]) {
    let n = String(describing: type(of: view))
    if n == "NSToolbarItemViewer" || n == "NSToolbarPlatterView" || n == "NSToolbarView" {
        out.append((n, view.convert(view.bounds, to: nil)))
    }
    for sub in view.subviews { scan(sub, into: &out) }
}

func run(_ name: String, useSpacer: Bool, removeSidebarToggle: Bool = false) {
    let controller = NSHostingController(
        rootView: Probe(useSpacer: useSpacer, removeSidebarToggle: removeSidebarToggle)
    )
    let window = NSWindow(contentViewController: controller)
    window.setFrame(NSRect(x: -8000, y: -8000, width: 1280, height: 820), display: true)
    window.styleMask.insert([.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView])
    window.titlebarAppearsTransparent = true
    window.titleVisibility = .hidden
    window.titlebarSeparatorStyle = .none
    window.isOpaque = true
    window.backgroundColor = .windowBackgroundColor
    window.orderFront(nil)
    RunLoop.main.run(until: Date().addingTimeInterval(1.2))
    window.contentView?.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.6))

    var hits: [(String, NSRect)] = []
    if let frame = window.contentView?.superview { scan(frame, into: &hits) }
    print("\n== \(name) ==  window width 1280")
    for (n, f) in hits.sorted(by: { $0.1.minX < $1.1.minX }) {
        print(String(format: "  %-21@ x %.0f … %.0f   (trailing gap %.0f)",
                     n as NSString, f.minX, f.maxX, 1280 - f.maxX))
    }
    window.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
run("A  no spacer", useSpacer: false)
run("B  ToolbarSpacer(.flexible)", useSpacer: true)
run("C  spacer + sidebarToggle removed", useSpacer: true, removeSidebarToggle: true)
exit(0)
