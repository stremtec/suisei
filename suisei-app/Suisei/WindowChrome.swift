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
        window.appearance = NSAppearance(named: light ? .aqua : .darkAqua)
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

        for kind: NSWindow.ButtonType in [.closeButton, .miniaturizeButton, .zoomButton] {
            guard let button = window.standardWindowButton(kind) else { continue }
            button.isHidden = false
            button.alphaValue = 1
            button.isEnabled = true
        }
    }
}

/// Re-apply chrome when the hosting window appears.
struct ThemedWindowChrome: NSViewRepresentable {
    var background: NSColor
    var light: Bool
    var identifier: NSUserInterfaceItemIdentifier? = nil
    var opaque: Bool = false

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
        WindowChrome.applyThemedTitlebar(
            to: window,
            background: background,
            light: light,
            opaque: opaque
        )
    }
}

/// Tags a SwiftUI window without applying titlebar policy. The editor and
/// Settings deliberately use different chrome, so title strings and broad
/// `NSApp.windows` heuristics are not safe window-role identifiers.
struct WindowIdentityProbe: NSViewRepresentable {
    var identifier: NSUserInterfaceItemIdentifier
    var onResolved: (NSWindow) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        view.isHidden = true
        DispatchQueue.main.async { resolve(view) }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        guard nsView.window?.identifier != identifier else { return }
        DispatchQueue.main.async { resolve(nsView) }
    }

    private func resolve(_ view: NSView) {
        guard let window = view.window else { return }
        window.identifier = identifier
        onResolved(window)
    }
}


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
