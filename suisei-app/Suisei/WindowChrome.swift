import AppKit
import SwiftUI

/// Settings / document window chrome — theme background without killing traffic lights.
enum WindowChrome {
    static let editorIdentifier = NSUserInterfaceItemIdentifier("suisei.window.editor")
    static let settingsIdentifier = NSUserInterfaceItemIdentifier("suisei.window.settings")
    static let gitWorkbenchIdentifier = NSUserInterfaceItemIdentifier("suisei.window.gitWorkbench")

    /// The corner radius macOS gives a window on this OS.
    ///
    /// The single place this number lives. It cannot be read back — the frame
    /// view reports `layer.cornerRadius == 0` here — so it is stated, and every
    /// surface that has to line up with a window corner derives from it:
    /// `ContentView.panelCornerRadius`, the Welcome window's own cut corner,
    /// and the resize HUD's mask. Those were three separate literals, and they
    /// had already drifted apart (12, 18, 12).
    ///
    /// If panels ever read tighter or rounder than the window behind them, this
    /// is the one value to change — everything else derives from it, so nothing
    /// else needs touching.
    ///
    /// MEASURED, not inferred. Captured a window against a near-black desktop
    /// and walked the top-left corner scanline by scanline: the inset reaches
    /// the straight edge after 31 native pixels on a 2x display, so the corner
    /// is ~16pt.
    ///
    /// The previous value here was 24, which I had back-solved from "the panel
    /// looked too tight" rather than measured — the same guessing this constant
    /// exists to stop. 16 is what the pixels say.
    static let windowCornerRadius: CGFloat = 16

    /// The appearance name a themed window should carry.
    ///
    /// There are FOUR of these, not two. `.aqua` / `.darkAqua` alone silently
    /// opt a window out of Increase Contrast, because the high-contrast
    /// variants are separate appearance names — name the plain one and every
    /// semantic colour resolved inside that window is pinned to its
    /// normal-contrast value for as long as the app runs.
    ///
    /// That matters here more than in a normal app: Suisei forces an appearance
    /// on every window (the chrome's light/dark is the THEME's, not the
    /// system's), so it does not get the system's choice by default and has to
    /// make the right one itself. Respecting the setting is a HIG requirement,
    /// and "semantic colours move with Increase Contrast" is the reason this
    /// app uses them at all.
    static func themedAppearanceName(light: Bool) -> NSAppearance.Name {
        let highContrast = NSWorkspace.shared
            .accessibilityDisplayShouldIncreaseContrast
        if light {
            return highContrast ? .accessibilityHighContrastAqua : .aqua
        }
        return highContrast ? .accessibilityHighContrastDarkAqua : .darkAqua
    }

    /// Apply appearance only. SwiftUI owns the Settings titlebar geometry.
    ///
    /// Moving or cloning the standard window buttons is appropriate for the
    /// editor's custom 48pt chrome, but not for a conventional Settings window.
    /// Keeping AppKit's real buttons in their native hierarchy preserves the
    /// system's placement, focus and accessibility behavior.
    static func applyThemedTitlebar(
        to window: NSWindow,
        background: NSColor,
        light: Bool,
        opaque: Bool = false
    ) {
        window.appearance = NSAppearance(named: themedAppearanceName(light: light))
        // The detail view paints its own semantic background. Keeping the
        // window itself transparent is what lets NavigationSplitView's native
        // sidebar material continue through the titlebar and blend like System
        // Settings; an opaque window flattened it into a mismatched dark slab.
        window.backgroundColor = opaque ? background : .clear
        window.isOpaque = opaque
        window.titlebarAppearsTransparent = true
        window.styleMask.insert([.titled, .closable, .miniaturizable, .resizable])
        window.titlebarSeparatorStyle = .none
        window.isMovableByWindowBackground = false
        if window.identifier == settingsIdentifier {
            // System Settings grows in height, not width. The sidebar +
            // grouped detail are composed for one column; stretching it
            // sideways just pads empty glass.
            let width: CGFloat = 780
            window.minSize = NSSize(width: width, height: 520)
            window.maxSize = NSSize(width: width, height: 12_000)
            if window.frame.width != width {
                var frame = window.frame
                frame.size.width = width
                window.setFrame(frame, display: true)
            }
        }

        for kind: NSWindow.ButtonType in [.closeButton, .miniaturizeButton, .zoomButton] {
            guard let button = window.standardWindowButton(kind) else { continue }
            button.isHidden = false
            button.alphaValue = 1
            button.isEnabled = true
        }

        if opaque {
            clearTitlebarMaterial(in: window)
            dumpTitlebar(window)
        }
    }

    /// Make the titlebar band genuinely transparent, so the CONTENT shows
    /// through it.
    ///
    /// `titlebarAppearsTransparent` makes the titlebar transparent and leaves
    /// alone the vibrancy AppKit puts behind a toolbar. That material sat above
    /// the window's background, so the top band stayed a neutral grey while the
    /// document under it was the palette's — and the tab strip lives up there,
    /// which is why it read as "the tab bar is a different colour from the
    /// editor".
    ///
    /// The band is hidden, NOT repainted. Painting it one colour was the first
    /// attempt and it was worse: the titlebar spans the whole width, so a solid
    /// fill covered the navigator's material where it continues up through the
    /// titlebar — the full-height-sidebar arrangement this window is built on —
    /// and flattened the window into one slab. Transparent, each half of the
    /// band shows what is beneath it: the sidebar's material on the left, the
    /// editor's surface on the right, which is what Xcode looks like and what
    /// the arrangement was for.
    ///
    /// Found by class name rather than by index. That view tree is AppKit's and
    /// its shape changes between releases, so anything positional would be a
    /// silent no-op the next time it moves — and a no-op here looks exactly
    /// like the bug.
    private static func clearTitlebarMaterial(in window: NSWindow) {
        guard let frameView = window.contentView?.superview else { return }
        for view in frameView.subviews
        where String(describing: type(of: view)).contains("TitlebarContainer") {
            view.wantsLayer = true
            view.layer?.backgroundColor = NSColor.clear.cgColor
            // The effect view is a child of the titlebar view inside the
            // container. Hidden rather than removed: it belongs to AppKit,
            // which may re-lay it out, and a hidden view it still owns survives
            // that where a removed one would be recreated.
            hideEffectViews(in: view)
        }
    }

    /// Print the titlebar's view tree once, with what each layer is actually
    /// filled with. `SUISEI_DIAG=titlebar`.
    ///
    /// Three attempts at the top band have now been made from screenshots, and
    /// the thing that decides between them is not visible in one: whether the
    /// band is an effect view that was missed, a layer with a colour of its
    /// own, or the content showing through correctly and simply being a colour
    /// nobody expected. Those are three different fixes.
    private nonisolated(unsafe) static var dumpedTitlebar = false

    static func dumpTitlebar(_ window: NSWindow) {
        guard !dumpedTitlebar,
              ProcessInfo.processInfo.environment["SUISEI_DIAG"]?
                  .lowercased().contains("titlebar") == true,
              let frameView = window.contentView?.superview else { return }
        dumpedTitlebar = true

        func walk(_ v: NSView, _ depth: Int) {
            let name = String(describing: type(of: v))
            let pad = String(repeating: "  ", count: depth)
            var note = "frame=\(v.frame.integral)"
            if v.isHidden { note += " HIDDEN" }
            if let cg = v.layer?.backgroundColor,
               let c = NSColor(cgColor: cg)?.usingColorSpace(.sRGB) {
                note += String(
                    format: " layer=#%02X%02X%02X α%.2f",
                    Int(c.redComponent * 255), Int(c.greenComponent * 255),
                    Int(c.blueComponent * 255), c.alphaComponent
                )
            }
            if let e = v as? NSVisualEffectView {
                note += " EFFECT material=\(e.material.rawValue) state=\(e.state.rawValue)"
            }
            NSLog("[suisei/titlebar] \(pad)\(name) \(note)")
            for child in v.subviews { walk(child, depth + 1) }
        }

        NSLog("[suisei/titlebar] window bg=\(window.backgroundColor) opaque=\(window.isOpaque)")
        for v in frameView.subviews
        where String(describing: type(of: v)).contains("Titlebar") {
            walk(v, 0)
        }
    }

    private static func hideEffectViews(in view: NSView) {
        for child in view.subviews {
            if let effect = child as? NSVisualEffectView {
                effect.isHidden = true
            } else {
                hideEffectViews(in: child)
            }
        }
    }
}

/// Re-apply chrome when the hosting window appears.
struct ThemedWindowChrome: NSViewRepresentable {
    var background: NSColor
    var light: Bool
    var identifier: NSUserInterfaceItemIdentifier? = nil
    var opaque: Bool = false
    /// Smallest content the window may be dragged to.
    ///
    /// Set on the WINDOW rather than as a `frame(minWidth:)` on the content,
    /// and the Git workbench is why. A minimum expressed as a SwiftUI frame is
    /// a size the content is entitled to DEMAND, so opening the sidebar made
    /// the demand `sidebar + detailMinimum`; the split view grew past the
    /// window, centred itself with 144pt hanging off each side for the whole
    /// animation, and snapped back in the last two frames. A window minimum
    /// cannot do that — it bounds the window and the content gets what is
    /// there.
    var minContentSize: NSSize? = nil

    func makeNSView(context: Context) -> NSView {
        let v = NSView(frame: .zero)
        v.isHidden = true
        DispatchQueue.main.async { apply(v) }
        return v
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async { apply(nsView) }
    }

    private func apply(_ nsView: NSView) {
        guard let window = nsView.window else { return }
        if let identifier {
            window.identifier = identifier
        }
        if let minContentSize, window.contentMinSize != minContentSize {
            window.contentMinSize = minContentSize
        }
        WindowChrome.applyThemedTitlebar(
            to: window,
            background: background,
            light: light,
            opaque: opaque
        )
    }
}

// `WindowIdentityProbe` is gone: it tagged a window WITHOUT applying titlebar
// policy, which was only needed while the editor styled its own chrome by hand.
// All three windows go through `ThemedWindowChrome` now, and that already sets
// the identifier — so there is one way to tag a window, not two.


// MARK: - Resize HUD (child window — covers the entire frame, lights included)

/// Frosted full-window overlay with live dimensions during window resize.
/// A child NSWindow is the only layer that can cover the traffic lights AND
/// still sample the window content behind it (its material blends behind-window,
/// so the parent's live pixels frost through — an in-window overlay host showed
/// a dead slab instead, because animating its alpha re-rendered the backdrop
/// offscreen where there is nothing to sample).
final class ResizeHudWindow {
    static let shared = ResizeHudWindow()
    private var hud: NSWindow?
    private weak var parent: NSWindow?
    /// Bumped on every show — a hide animation's completion from a PREVIOUS
    /// gesture must not tear down the HUD a newer show just put up.
    private var generation = 0

    private let model = ResizeHudModel()

    func show(over parent: NSWindow) {
        self.parent = parent
        generation += 1
        let w = hud ?? makeWindow()
        hud = w
        model.size = parent.frame.size
        w.setFrame(parent.frame, display: true)
        applyCornerMask(to: w, matching: parent)
        if w.parent == nil {
            parent.addChildWindow(w, ordered: .above)
        }
        w.alphaValue = 0
        w.orderFront(nil)
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.16
            w.animator().alphaValue = 1
        }
    }

    func update(over parent: NSWindow) {
        guard let hud else { return }
        hud.setFrame(parent.frame, display: true)
        // Re-read every frame: a drag that starts from a maximized window
        // begins square (radius 0) and gains rounded corners mid-gesture.
        applyCornerMask(to: hud, matching: parent)
        model.size = parent.frame.size
    }

    func hide() {
        guard let hud else { return }
        let gen = generation
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = 0.2
            hud.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            guard let self, self.generation == gen, let hud = self.hud else { return }
            hud.parent?.removeChildWindow(hud)
            hud.orderOut(nil)
        })
    }

    /// The child window is a plain rectangle: unmasked, its square corners paint
    /// over the parent's rounded ones for the whole drag. Clip it to match. The
    /// parent's frame view doesn't expose its radius as layer.cornerRadius on
    /// this OS (reads 0), so fall back to `WindowChrome.windowCornerRadius`; fullscreen
    /// windows are the only truly square case.
    private func applyCornerMask(to hud: NSWindow, matching parent: NSWindow) {
        guard let content = hud.contentView else { return }
        content.wantsLayer = true
        let parentRadius = parent.contentView?.superview?.layer?.cornerRadius ?? 0
        let radius: CGFloat
        if parent.styleMask.contains(.fullScreen) {
            radius = 0
        } else {
            radius = parentRadius > 0 ? parentRadius : WindowChrome.windowCornerRadius
        }
        content.layer?.cornerRadius = radius
        content.layer?.cornerCurve = .continuous
        content.layer?.masksToBounds = true
    }

    private func makeWindow() -> NSWindow {
        let w = NSWindow(
            contentRect: .zero,
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        w.isOpaque = false
        w.backgroundColor = .clear
        w.ignoresMouseEvents = true
        w.hasShadow = false
        w.contentView = NSHostingView(rootView: ResizeHudView(model: model))
        return w
    }
}

final class ResizeHudModel: ObservableObject {
    @Published var size: CGSize = .zero
}

/// Frost for the HUD child window. Must be a real NSVisualEffectView blending
/// BEHIND the (transparent) child window — the window server then blurs the
/// parent window's live pixels through it. A SwiftUI Material here has nothing
/// in its own window to sample and collapses into an opaque slab.
private struct HudBehindWindowBlur: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let v = NSVisualEffectView()
        v.blendingMode = .behindWindow
        v.material = .hudWindow
        v.state = .active
        return v
    }

    func updateNSView(_ v: NSVisualEffectView, context: Context) {}
}

struct ResizeHudView: View {
    @ObservedObject var model: ResizeHudModel
    private var size: CGSize { model.size }

    var body: some View {
        ZStack {
            HudBehindWindowBlur()
            VStack(spacing: 6) {
                Image(systemName: "arrow.up.left.and.arrow.down.right")
                    .font(.system(size: 20, weight: .medium))
                    .foregroundStyle(.secondary)
                Text("\(Int(size.width)) × \(Int(size.height))")
                    .font(.system(size: 26, weight: .semibold, design: .rounded))
                    .monospacedDigit()
                    .contentTransition(.numericText())
                    .animation(.snappy(duration: 0.18), value: size.width + size.height)
                Text("Suisei")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 20)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: Radius.floating, style: .continuous))
            .shadow(color: .black.opacity(0.25), radius: 20, y: 6)
        }
    }
}
