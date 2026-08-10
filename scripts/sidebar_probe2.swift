import AppKit
import SwiftUI

// Full view + layer tree, so the sidebar region can be compared shape by shape.

struct CaseA: View {
    var body: some View {
        NavigationSplitView {
            ScrollView { VStack { ForEach(0..<8, id: \.self) { Text("row \($0)") } } }
        } detail: { Text("detail") }
    }
}

struct CaseD: View {
    var body: some View {
        NavigationSplitView {
            VStack(spacing: 0) {
                Text("rail").frame(height: 28)
                Divider()
                List { ForEach(0..<8, id: \.self) { Text("row \($0)") } }
                    .listStyle(.sidebar)
                    .scrollContentBackground(.hidden)
            }
            .navigationSplitViewColumnWidth(min: 280, ideal: 294, max: 350)
        } detail: { Text("detail") }
    }
}

func walk(_ view: NSView, depth: Int, into out: inout [String]) {
    let f = view.convert(view.bounds, to: nil)
    let pad = String(repeating: "  ", count: depth)
    var extra = ""
    if let ve = view as? NSVisualEffectView {
        extra = "  [material=\(ve.material.rawValue) blending=\(ve.blendingMode == .behindWindow ? "behind" : "within") emph=\(ve.isEmphasized)]"
    }
    if let sv = view as? NSSplitView {
        extra += "  [splitView vertical=\(sv.isVertical)]"
    }
    var layerInfo = ""
    if let l = view.layer {
        layerInfo = " layer=\(type(of: l))"
        if let bg = l.backgroundColor, let c = NSColor(cgColor: bg)?.usingColorSpace(.sRGB) {
            layerInfo += String(format: " bg=#%02X%02X%02X@%.2f",
                                Int(c.redComponent * 255), Int(c.greenComponent * 255),
                                Int(c.blueComponent * 255), c.alphaComponent)
        }
        if !(l.filters?.isEmpty ?? true) { layerInfo += " filters=\(l.filters!.count)" }
        if !(l.backgroundFilters?.isEmpty ?? true) { layerInfo += " bgFilters=\(l.backgroundFilters!.count)" }
    }
    out.append(String(format: "%@%@ (%.0f,%.0f %.0fx%.0f)%@%@",
                      pad, String(describing: type(of: view)),
                      f.origin.x, f.origin.y, f.size.width, f.size.height, extra, layerInfo))
    for sub in view.subviews { walk(sub, depth: depth + 1, into: &out) }
}

func controllerTree(_ vc: NSViewController, depth: Int, into out: inout [String]) {
    let pad = String(repeating: "  ", count: depth)
    var extra = ""
    if let split = vc as? NSSplitViewController {
        extra = "  items=[" + split.splitViewItems.map { item in
            "behavior=\(item.behavior.rawValue) collapsed=\(item.isCollapsed)"
        }.joined(separator: " | ") + "]"
    }
    out.append("\(pad)\(type(of: vc))\(extra)")
    for child in vc.children { controllerTree(child, depth: depth + 1, into: &out) }
}

func probe(_ name: String, _ root: some View) {
    let controller = NSHostingController(rootView: root)
    let window = NSWindow(contentViewController: controller)
    window.setFrame(NSRect(x: -8000, y: -8000, width: 900, height: 600), display: true)
    window.styleMask.insert(.fullSizeContentView)
    window.titlebarAppearsTransparent = true
    window.orderFront(nil)
    RunLoop.main.run(until: Date().addingTimeInterval(0.8))
    window.contentView?.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.4))

    print("\n======== \(name) ========")
    var vcs: [String] = []
    if let root = window.contentViewController { controllerTree(root, depth: 1, into: &vcs) }
    print("-- view controllers --")
    for l in vcs { print(l) }

    var out: [String] = []
    if let frame = window.contentView?.superview { walk(frame, depth: 1, into: &out) }
    print("-- views (left 320pt only) --")
    for l in out where l.contains("NSVisualEffect") || l.contains("SplitView")
        || l.contains("Sidebar") || l.contains("Backdrop") || l.contains("Glass")
        || l.contains("bgFilters") || l.contains("bg=#") {
        print(l)
    }
    window.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
probe("A  ScrollView+VStack sidebar", CaseA())
probe("D  workbench shape", CaseD())
exit(0)
