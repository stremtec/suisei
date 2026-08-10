import AppKit
import SwiftUI

// Where does the macOS sidebar material actually come from?
// Four sidebar shapes, same NavigationSplitView. Dump every NSVisualEffectView
// AppKit produced, with its material, blending mode and frame.

struct CaseA: View {              // ScrollView + VStack  (what ProjectTreeView is)
    var body: some View {
        NavigationSplitView {
            ScrollView { VStack { ForEach(0..<8, id: \.self) { Text("row \($0)") } } }
        } detail: { Text("detail") }
    }
}

struct CaseB: View {              // List, default style inside a split view
    var body: some View {
        NavigationSplitView {
            List { ForEach(0..<8, id: \.self) { Text("row \($0)") } }
        } detail: { Text("detail") }
    }
}

struct CaseC: View {              // List + explicit .sidebar + hidden scroll bg
    var body: some View {
        NavigationSplitView {
            List { ForEach(0..<8, id: \.self) { Text("row \($0)") } }
                .listStyle(.sidebar)
                .scrollContentBackground(.hidden)
        } detail: { Text("detail") }
    }
}

struct CaseD: View {              // the Git workbench's exact shape
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

struct CaseE: View {              // ScrollView sidebar WITH an opaque card behind it
    var body: some View {
        NavigationSplitView {
            ScrollView { VStack { ForEach(0..<8, id: \.self) { Text("row \($0)") } } }
                .background(Color.black)
        } detail: { Text("detail") }
    }
}

func materialName(_ m: NSVisualEffectView.Material) -> String {
    switch m {
    case .titlebar: return "titlebar"
    case .selection: return "selection"
    case .menu: return "menu"
    case .popover: return "popover"
    case .sidebar: return "sidebar"
    case .headerView: return "headerView"
    case .sheet: return "sheet"
    case .windowBackground: return "windowBackground"
    case .hudWindow: return "hudWindow"
    case .fullScreenUI: return "fullScreenUI"
    case .toolTip: return "toolTip"
    case .contentBackground: return "contentBackground"
    case .underWindowBackground: return "underWindowBackground"
    case .underPageBackground: return "underPageBackground"
    default: return "raw(\(m.rawValue))"
    }
}

func dump(_ view: NSView, depth: Int, into out: inout [String]) {
    if let ve = view as? NSVisualEffectView {
        let f = view.convert(view.bounds, to: nil)
        out.append(String(
            format: "    NSVisualEffectView  material=%@  blending=%@  state=%d  frameInWindow=(%.0f,%.0f %.0fx%.0f)",
            materialName(ve.material),
            ve.blendingMode == .behindWindow ? "behindWindow" : "withinWindow",
            ve.state.rawValue,
            f.origin.x, f.origin.y, f.size.width, f.size.height
        ))
    }
    for sub in view.subviews { dump(sub, depth: depth + 1, into: &out) }
}

func probe(_ name: String, _ root: some View) {
    let host = NSHostingView(rootView: root)
    let window = NSWindow(
        contentRect: NSRect(x: -8000, y: -8000, width: 900, height: 600),
        styleMask: [.titled, .closable, .resizable, .fullSizeContentView],
        backing: .buffered,
        defer: false
    )
    window.contentView = host
    window.orderFront(nil)
    RunLoop.main.run(until: Date().addingTimeInterval(0.6))
    host.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.4))

    var out: [String] = []
    if let frame = window.contentView?.superview { dump(frame, depth: 0, into: &out) }
    print("\n== \(name) ==")
    print("  window.isOpaque=\(window.isOpaque)")
    if out.isEmpty { print("    (no NSVisualEffectView anywhere)") }
    for line in out { print(line) }
    window.orderOut(nil)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

probe("A  ScrollView+VStack sidebar", CaseA())
probe("B  List (default style)", CaseB())
probe("C  List .sidebar + hidden scroll bg", CaseC())
probe("D  workbench shape (VStack{rail,Divider,List})", CaseD())
probe("E  ScrollView sidebar + opaque .background(black)", CaseE())

// What the three semantic colours actually resolve to, both appearances.
for (label, name) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
    let ap = NSAppearance(named: name)!
    var line = "\n\(label): "
    for (n, c) in [("window", NSColor.windowBackgroundColor),
                   ("text", .textBackgroundColor),
                   ("control", .controlBackgroundColor),
                   ("underPageBackground", .underPageBackgroundColor)] {
        var hex = "?"
        ap.performAsCurrentDrawingAppearance {
            if let s = c.usingColorSpace(.sRGB) {
                hex = String(format: "#%02X%02X%02X",
                             Int(s.redComponent * 255), Int(s.greenComponent * 255), Int(s.blueComponent * 255))
            }
        }
        line += "\(n)=\(hex)  "
    }
    print(line)
}
print("")
exit(0)
