import AppKit

// What is a native window tab MADE of? Materials, layer fills, radii — for the
// selected tab, the unselected ones, the bar behind them, and the +/x buttons.

final class TabHost: NSWindow {
    override func newWindowForTab(_ sender: Any?) {}   // makes AppKit show the "+"
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
NSWindow.allowsAutomaticWindowTabbing = true

func makeWindow(_ title: String) -> NSWindow {
    let w = TabHost(
        contentRect: NSRect(x: -8000, y: -8000, width: 1000, height: 600),
        styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
        backing: .buffered,
        defer: false
    )
    w.title = title
    w.tabbingMode = .preferred
    w.tabbingIdentifier = "probe"
    w.contentView = NSView()
    return w
}

let first = makeWindow("README.md")
first.orderFront(nil)
for t in ["ContentView.swift", "notes.txt"] {
    first.addTabbedWindow(makeWindow(t), ordered: .above)
}
RunLoop.main.run(until: Date().addingTimeInterval(0.8))
first.toggleTabBar(nil)
RunLoop.main.run(until: Date().addingTimeInterval(0.9))
first.contentView?.superview?.layoutSubtreeIfNeeded()
RunLoop.main.run(until: Date().addingTimeInterval(0.6))

func materialName(_ m: NSVisualEffectView.Material) -> String {
    switch m {
    case .titlebar: return "titlebar"; case .selection: return "selection"
    case .menu: return "menu"; case .popover: return "popover"
    case .sidebar: return "sidebar"; case .headerView: return "headerView"
    case .sheet: return "sheet"; case .windowBackground: return "windowBackground"
    case .hudWindow: return "hudWindow"; case .fullScreenUI: return "fullScreenUI"
    case .toolTip: return "toolTip"; case .contentBackground: return "contentBackground"
    case .underWindowBackground: return "underWindowBackground"
    case .underPageBackground: return "underPageBackground"
    default: return "raw(\(m.rawValue))"
    }
}

func hex(_ cg: CGColor?) -> String {
    guard let cg, let c = NSColor(cgColor: cg)?.usingColorSpace(.sRGB) else { return "-" }
    return String(format: "#%02X%02X%02X @%.3f",
                  Int(c.redComponent * 255), Int(c.greenComponent * 255),
                  Int(c.blueComponent * 255), c.alphaComponent)
}

func walk(_ v: NSView, depth: Int, into out: inout [String]) {
    let n = String(describing: type(of: v))
    let interesting = n.localizedCaseInsensitiveContains("tab")
        || n.contains("Glass") || n.contains("Backdrop") || n.contains("VisualEffect")
        || v is NSButton
    if interesting {
        let f = v.convert(v.bounds, to: nil)
        var s = String(format: "%-32@ (%.0f,%.0f %.0fx%.0f)",
                       n as NSString, f.minX, f.minY, f.width, f.height)
        if let ve = v as? NSVisualEffectView {
            s += "  MATERIAL=\(materialName(ve.material)) "
            s += ve.blendingMode == .behindWindow ? "behind" : "within"
            s += ve.isEmphasized ? " emphasized" : ""
        }
        if let l = v.layer {
            s += "  layer=\(type(of: l))"
            if l.cornerRadius > 0 { s += String(format: " r=%.1f", l.cornerRadius) }
            if l.backgroundColor != nil { s += "  fill=\(hex(l.backgroundColor))" }
            if l.borderWidth > 0 { s += String(format: " border=%.1f %@", l.borderWidth, hex(l.borderColor) as NSString) }
            if !(l.filters?.isEmpty ?? true) { s += " filters=\(l.filters!.count)" }
            if l.shadowOpacity > 0 { s += String(format: " shadow=%.2f r%.1f", l.shadowOpacity, l.shadowRadius) }
        }
        if let b = v as? NSButton {
            s += "  BUTTON img=\(b.image?.name() ?? "-") bezel=\(b.bezelStyle.rawValue) bordered=\(b.isBordered)"
        }
        out.append(String(repeating: "  ", count: depth) + s)
    }
    for s in v.subviews { walk(s, depth: depth + 1, into: &out) }
}

var out: [String] = []
if let frame = first.contentView?.superview { walk(frame, depth: 0, into: &out) }
print("\n== native tab bar materials (dark = \(NSApp.effectiveAppearance.name.rawValue)) ==")
for line in out { print("  " + line) }
print("\n  selected tab is the FIRST window in the group: \(first.title)")
print("")
exit(0)
