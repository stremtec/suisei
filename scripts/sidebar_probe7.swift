import AppKit
import SwiftUI

// Where do toolbar items actually land horizontally? Probe 4 showed them at
// x≈292–416 in a 1000pt window — packed against the sidebar, not the trailing
// edge. Which declaration puts them top-RIGHT?

enum Variant: String, CaseIterable {
    case primaryActionNoTitle   = "A  .primaryAction, title removed (current)"
    case primaryActionWithTitle = "B  .primaryAction, navigationTitle kept"
    case itemGroup              = "C  ToolbarItemGroup(.primaryAction)"
    case flexibleSpacer         = "D  ToolbarSpacer(.flexible) then items"
    case confirmationAction     = "E  .confirmationAction"
}

struct Probe: View {
    let variant: Variant

    var body: some View {
        NavigationSplitView {
            Color.clear.navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 460)
        } detail: {
            Color.clear
        }
        .navigationSplitViewStyle(.balanced)
        .navigationTitle(variant == .primaryActionWithTitle ? "Suisei" : "")
        .modifier(TitleStrip(strip: variant != .primaryActionWithTitle))
        .toolbar { bar }
    }

    @ToolbarContentBuilder private var bar: some ToolbarContent {
        switch variant {
        case .primaryActionNoTitle, .primaryActionWithTitle:
            ToolbarItem(placement: .primaryAction) { icon("magnifyingglass") }
            ToolbarItem(placement: .primaryAction) { icon("gearshape") }
            ToolbarItem(placement: .primaryAction) { icon("sidebar.right") }
        case .itemGroup:
            ToolbarItemGroup(placement: .primaryAction) {
                icon("magnifyingglass"); icon("gearshape"); icon("sidebar.right")
            }
        case .flexibleSpacer:
            ToolbarSpacer(.flexible)
            ToolbarItem(placement: .primaryAction) { icon("magnifyingglass") }
            ToolbarItem(placement: .primaryAction) { icon("gearshape") }
            ToolbarItem(placement: .primaryAction) { icon("sidebar.right") }
        case .confirmationAction:
            ToolbarItem(placement: .confirmationAction) { icon("magnifyingglass") }
            ToolbarItem(placement: .confirmationAction) { icon("gearshape") }
            ToolbarItem(placement: .confirmationAction) { icon("sidebar.right") }
        }
    }

    private func icon(_ name: String) -> some View {
        Button { } label: { Image(systemName: name) }
    }
}

struct TitleStrip: ViewModifier {
    let strip: Bool
    func body(content: Content) -> some View {
        if strip { content.toolbar(removing: .title) } else { content }
    }
}

func scanToolbar(_ view: NSView, into out: inout [(String, NSRect)]) {
    let n = String(describing: type(of: view))
    if n == "NSToolbarItemViewer" || n == "NSToolbarPlatterView" {
        out.append((n, view.convert(view.bounds, to: nil)))
    }
    for sub in view.subviews { scanToolbar(sub, into: &out) }
}

func run(_ variant: Variant) {
    let controller = NSHostingController(rootView: Probe(variant: variant))
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
    if let frame = window.contentView?.superview { scanToolbar(frame, into: &hits) }
    print("\n== \(variant.rawValue) ==  window width 1000")
    for (n, f) in hits.sorted(by: { $0.1.minX < $1.1.minX }) {
        print(String(format: "  %-22@ x %.0f … %.0f   (trailing gap %.0f)",
                     n as NSString, f.minX, f.maxX, 1000 - f.maxX))
    }
    if hits.isEmpty { print("  (no toolbar item views)") }
    window.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
for v in Variant.allCases { run(v) }
exit(0)
