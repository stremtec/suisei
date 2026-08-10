import AppKit

// What exactly is NSTabBarNewTabButton made of?

final class TabHost: NSWindow {
    override func newWindowForTab(_ sender: Any?) {}
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
NSWindow.allowsAutomaticWindowTabbing = true

func makeWindow(_ title: String) -> NSWindow {
    let w = TabHost(
        contentRect: NSRect(x: -8000, y: -8000, width: 1000, height: 600),
        styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
        backing: .buffered, defer: false
    )
    w.title = title
    w.tabbingMode = .preferred
    w.tabbingIdentifier = "probe"
    w.contentView = NSView()
    return w
}

let first = makeWindow("a")
first.orderFront(nil)
for t in ["b", "c"] { first.addTabbedWindow(makeWindow(t), ordered: .above) }
RunLoop.main.run(until: Date().addingTimeInterval(0.8))
first.toggleTabBar(nil)
RunLoop.main.run(until: Date().addingTimeInterval(0.9))
first.contentView?.superview?.layoutSubtreeIfNeeded()
RunLoop.main.run(until: Date().addingTimeInterval(0.5))

func find(_ v: NSView, named: String) -> NSView? {
    if String(describing: type(of: v)) == named { return v }
    for s in v.subviews { if let f = find(s, named: named) { return f } }
    return nil
}

func hex(_ cg: CGColor?) -> String {
    guard let cg, let c = NSColor(cgColor: cg)?.usingColorSpace(.sRGB) else { return "-" }
    return String(format: "#%02X%02X%02X@%.2f", Int(c.redComponent * 255),
                  Int(c.greenComponent * 255), Int(c.blueComponent * 255), c.alphaComponent)
}

func dump(_ v: NSView, depth: Int) {
    let f = v.convert(v.bounds, to: nil)
    var s = String(format: "%@%-34@ (%.0f,%.0f %.0fx%.0f)",
                   String(repeating: "  ", count: depth),
                   String(describing: type(of: v)) as NSString, f.minX, f.minY, f.width, f.height)
    if let l = v.layer {
        s += " layer=\(type(of: l))"
        if l.cornerRadius > 0 { s += String(format: " r=%.1f", l.cornerRadius) }
        if l.backgroundColor != nil { s += " fill=\(hex(l.backgroundColor))" }
        if l.borderWidth > 0 { s += String(format: " border=%.1f", l.borderWidth) }
        if !(l.filters?.isEmpty ?? true) { s += " filters=\(l.filters!.count)" }
    }
    print(s)
    for sub in v.subviews { dump(sub, depth: depth + 1) }
}

guard let frame = first.contentView?.superview,
      let plus = find(frame, named: "NSTabBarNewTabButton") as? NSButton else {
    print("  new-tab button not found"); exit(0)
}

print("\n== NSTabBarNewTabButton ==")
print("  bezelStyle raw   : \(plus.bezelStyle.rawValue)")
print("  isBordered       : \(plus.isBordered)")
print("  bezelColor       : \(plus.bezelColor?.description ?? "nil")")
print("  contentTintColor : \(plus.contentTintColor?.description ?? "nil")")
print("  image            : \(plus.image?.name() ?? "nil")  symbol=\(plus.image?.isTemplate ?? false)")
print("  imagePosition    : \(plus.imagePosition.rawValue)")
print("  cell             : \(plus.cell.map { String(describing: type(of: $0)) } ?? "nil")")
if let c = plus.cell as? NSButtonCell {
    print("  cell.bezelStyle  : \(c.bezelStyle.rawValue)  isBordered=\(c.isBordered)")
    print("  cell.backgroundColor: \(c.backgroundColor?.description ?? "nil")")
    print("  cell.highlightsBy: \(c.highlightsBy.rawValue)  showsStateBy=\(c.showsStateBy.rawValue)")
}
print("  --- subtree ---")
dump(plus, depth: 1)

// Also: what is the parent trough, and where does the button sit relative to it?
if let track = find(frame, named: "NSTabBarTrackView") {
    let tf = track.convert(track.bounds, to: nil)
    let bf = plus.convert(plus.bounds, to: nil)
    print(String(format: "\n  track  x %.0f … %.0f", tf.minX, tf.maxX))
    print(String(format: "  plus   x %.0f … %.0f   gap after track = %.0f  window edge gap = %.0f",
                 bf.minX, bf.maxX, bf.minX - tf.maxX, 1000 - bf.maxX))
}
print("")
exit(0)
