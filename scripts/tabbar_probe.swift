import AppKit

// What does the real macOS window tab bar measure? Build one with three tabs
// and read the geometry off it instead of recalling it.

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
NSWindow.allowsAutomaticWindowTabbing = true

func makeWindow(_ title: String) -> NSWindow {
    let w = NSWindow(
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
if first.tabbedWindows?.isEmpty ?? true { print("  (no tab group formed)") }
first.toggleTabBar(nil)
RunLoop.main.run(until: Date().addingTimeInterval(0.8))
first.contentView?.superview?.layoutSubtreeIfNeeded()
RunLoop.main.run(until: Date().addingTimeInterval(0.5))

func describe(_ v: NSView) -> String {
    let f = v.convert(v.bounds, to: nil)
    var s = String(format: "%-34@ (%.0f,%.0f  %.0f x %.0f)",
                   String(describing: type(of: v)) as NSString,
                   f.minX, f.minY, f.width, f.height)
    if let l = v.layer, l.cornerRadius > 0 {
        s += String(format: "  radius %.1f", l.cornerRadius)
    }
    if let b = v as? NSButton {
        s += "  button[\(b.image?.name() ?? b.title)] bezel=\(b.bezelStyle.rawValue)"
    }
    if let c = v as? NSTextField {
        s += "  text[\(c.stringValue)] font=\(c.font?.pointSize ?? 0)"
    }
    return s
}

func walk(_ v: NSView, depth: Int, into out: inout [String]) {
    let n = String(describing: type(of: v))
    if n.localizedCaseInsensitiveContains("tab") || v is NSButton || v is NSTextField {
        out.append(String(repeating: "  ", count: depth) + describe(v))
    }
    for s in v.subviews { walk(s, depth: depth + 1, into: &out) }
}

var out: [String] = []
if let frame = first.contentView?.superview { walk(frame, depth: 0, into: &out) }
print("\n== native window tab bar, 3 tabs, window 1000pt wide ==")
print("  tabbedWindows: \(first.tabbedWindows?.count ?? 0)")
for line in out { print("  " + line) }
print("")
exit(0)
