import AppKit

// Which switch actually removes "Show Tab Bar" / "Show All Tabs" /
// "Merge All Windows" / "Move Tab to New Window" from the Window menu?
//   A: nothing            — baseline
//   B: allowsAutomaticWindowTabbing = false   (class-wide)
//   C: window.tabbingMode = .disallowed       (per window)
//   D: both

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

// A Window menu AppKit will inject its tabbing items into.
let mainMenu = NSMenu()
let windowItem = NSMenuItem()
let windowMenu = NSMenu(title: "Window")
windowItem.submenu = windowMenu
mainMenu.addItem(windowItem)
app.mainMenu = mainMenu
app.windowsMenu = windowMenu

func makeWindow(disallow: Bool) -> NSWindow {
    let w = NSWindow(
        contentRect: NSRect(x: -8000, y: -8000, width: 800, height: 600),
        styleMask: [.titled, .closable, .miniaturizable, .resizable],
        backing: .buffered,
        defer: false
    )
    if disallow { w.tabbingMode = .disallowed }
    w.contentView = NSView()
    w.orderFront(nil)
    return w
}

func tabbingItems() -> [String] {
    let names = ["Tab Bar", "All Tabs", "Merge All Windows", "Move Tab to New Window"]
    return (app.windowsMenu?.items ?? [])
        .map(\.title)
        .filter { t in names.contains { t.contains($0) } }
}

func run(_ label: String, classWide: Bool, perWindow: Bool) {
    NSWindow.allowsAutomaticWindowTabbing = !classWide
    let w = makeWindow(disallow: perWindow)
    RunLoop.main.run(until: Date().addingTimeInterval(0.5))
    // AppKit populates the Window menu lazily, on update.
    app.windowsMenu?.update()
    RunLoop.main.run(until: Date().addingTimeInterval(0.3))
    let found = tabbingItems()
    print(String(format: "  %-46@ allowsAutomaticWindowTabbing=%@  tabbingMode=%@",
                 label as NSString,
                 NSWindow.allowsAutomaticWindowTabbing ? "true " : "false",
                 w.tabbingMode == .disallowed ? "disallowed" : "automatic"))
    print("      window-menu tabbing items: \(found.isEmpty ? ["(none)"] : found)")
    w.orderOut(nil)
}

print("")
run("A  baseline", classWide: false, perWindow: false)
run("B  allowsAutomaticWindowTabbing = false", classWide: true, perWindow: false)
run("C  tabbingMode = .disallowed", classWide: false, perWindow: true)
run("D  both", classWide: true, perWindow: true)
print("")
exit(0)
