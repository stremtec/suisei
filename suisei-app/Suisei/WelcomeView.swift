import SwiftUI
import AppKit
import QuartzCore

/// Xcode-style **launch sheet** (matches Xcode Welcome proportions & rhythm).
///
/// Layout (from Xcode 26 Welcome):
/// - Fixed rounded card, ~1.65∶1 aspect, continuous corners
/// - **50∶50** left / right split (no heavy divider — soft shade change only)
/// - Left: brand (icon + name + version) in upper half, action capsules lower
/// - Right: empty → “No Recent Projects” dead-center; else Recents list
/// - Top-leading `xmark.circle.fill` dismiss (not traffic lights)
struct WelcomeView: View {
    var onCreate: () -> Void
    var onOpen: () -> Void
    var onClone: () -> Void
    var onOpenRecent: (String) -> Void
    var onClose: () -> Void
    var recents: [RecentItem]

    /// Golden-ratio card (780 / φ ≈ 482) — balanced, Xcode-Welcome-like.
    static let windowSize = NSSize(width: 780, height: 482)
    static let cornerRadius: CGFloat = 16
    /// Hero icon — Xcode’s hammer sits ~128–136pt visual.
    private static let brandIconSize: CGFloat = 132

    // Xcode dark welcome: left slightly deeper, right a hair lighter.
    private let panelLeft = Color(red: 0.145, green: 0.145, blue: 0.150)
    private let panelRight = Color(red: 0.175, green: 0.175, blue: 0.182)
    private let buttonFill = Color.white.opacity(0.075)
    private let label = Color.white.opacity(0.94)
    private let muted = Color.white.opacity(0.40)

    @State private var appeared = false

    var body: some View {
        ZStack(alignment: .topLeading) {
            HStack(spacing: 0) {
                leftPane
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(
                        ZStack {
                            panelLeft
                            // Faint comet glow behind the brand (subtle, not a poster).
                            RadialGradient(
                                colors: [Color(red: 0.45, green: 0.55, blue: 0.95).opacity(0.16), .clear],
                                center: .init(x: 0.5, y: 0.30),
                                startRadius: 10,
                                endRadius: 260
                            )
                        }
                    )

                // Soft split only (Xcode: no hard 1px white rule).
                rightPane
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(panelRight)
            }

            Button(role: .cancel, action: onClose) {
                Image(systemName: "xmark.circle.fill")
                    .symbolRenderingMode(.hierarchical)
                    .font(.system(size: 14, weight: .regular))
                    .foregroundStyle(.secondary)
                    .opacity(0.85)
            }
            .buttonStyle(.plain)
            .help("Close")
            .accessibilityLabel("Close")
            .padding(.leading, 14)
            .padding(.top, 14)
        }
        .frame(width: Self.windowSize.width, height: Self.windowSize.height)
        .compositingGroup()
        .clipShape(RoundedRectangle(cornerRadius: Self.cornerRadius, style: .continuous))
        .gesture(WindowDragGesture())
        .preferredColorScheme(.dark)
        .background(WelcomeWindowChrome(cornerRadius: Self.cornerRadius))
        .onAppear {
            withAnimation(.smooth(duration: 0.45)) { appeared = true }
        }
    }

    /// Left half — brand upper-center, actions lower-center (Xcode rhythm).
    private var leftPane: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)
                .frame(height: 52)

            // Brand cluster (icon → title → version), horizontally centered.
            // The icns has generous internal margins — keep the gap tight.
            VStack(spacing: 0) {
                brandMark
                    .padding(.bottom, 2)

                Text("Suisei")
                    .font(.system(size: 26, weight: .semibold, design: .default))
                    .foregroundStyle(label)

                Text("Version \(EngineBridge.engineVersion)")
                    .font(.system(size: 12, weight: .regular, design: .default))
                    .foregroundStyle(muted)
                    .padding(.top, 5)
            }
            .frame(maxWidth: .infinity)
            .opacity(appeared ? 1 : 0)
            .offset(y: appeared ? 0 : 8)

            // Air between brand and actions — the key Xcode “feel”.
            Spacer(minLength: 36)

            VStack(spacing: 10) {
                welcomeButton(systemImage: "plus.square", title: "Create New File…", action: onCreate)
                welcomeButton(systemImage: "square.and.arrow.down.on.square", title: "Clone Git Repository…", action: onClone)
                welcomeButton(systemImage: "folder", title: "Open Existing Project…", action: onOpen)
            }
            // Inset so capsules don’t touch the split edge (Xcode ~40–48pt).
            .padding(.horizontal, 44)
            .padding(.bottom, 48)
            .opacity(appeared ? 1 : 0)
            .offset(y: appeared ? 0 : 10)
            .animation(.smooth(duration: 0.5).delay(0.08), value: appeared)
        }
    }

    @ViewBuilder
    private var brandMark: some View {
        if let ns = NSImage(named: "Suisei") ?? bundleIconImage() {
            Image(nsImage: ns)
                .resizable()
                .interpolation(.high)
                .aspectRatio(1, contentMode: .fit)
                .frame(width: Self.brandIconSize, height: Self.brandIconSize)
                .shadow(color: .black.opacity(0.40), radius: 18, y: 8)
        } else {
            Text("彗")
                .font(.system(size: 84, weight: .medium, design: .serif))
                .foregroundStyle(Color.white.opacity(0.95))
                .frame(width: Self.brandIconSize, height: Self.brandIconSize)
        }
    }

    private func bundleIconImage() -> NSImage? {
        guard let url = Bundle.main.url(forResource: "Suisei", withExtension: "icns")
                ?? Bundle.main.url(forResource: "Suisei", withExtension: "png")
        else { return nil }
        return NSImage(contentsOf: url)
    }

    /// Right half — empty state centered; grouped list when recents exist:
    /// project folders first (click to reveal their recent files), then loose files.
    @State private var expandedRecents: Set<String> = []

    private var recentFolders: [RecentItem] { recents.filter(\.isDir) }

    /// Recent files under `folder`, newest first (recents order).
    private func recentFiles(in folder: RecentItem) -> [RecentItem] {
        let prefix = folder.path.hasSuffix("/") ? folder.path : folder.path + "/"
        return recents.filter { !$0.isDir && $0.path.hasPrefix(prefix) }
    }

    /// Files not owned by any recent folder.
    private var looseRecentFiles: [RecentItem] {
        let prefixes = recentFolders.map { $0.path.hasSuffix("/") ? $0.path : $0.path + "/" }
        return recents.filter { item in
            !item.isDir && !prefixes.contains { item.path.hasPrefix($0) }
        }
    }

    private var rightPane: some View {
        Group {
            if recents.isEmpty {
                Text("No Recent Projects")
                    .font(.system(size: 13, weight: .regular, design: .default))
                    .foregroundStyle(muted)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                VStack(alignment: .leading, spacing: 0) {
                    Text("Recents")
                        .font(.system(size: 12, weight: .semibold, design: .default))
                        .foregroundStyle(muted)
                        .padding(.horizontal, 28)
                        .padding(.top, 28)
                        .padding(.bottom, 12)

                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 4) {
                            // ── Projects (folders) first
                            ForEach(recentFolders) { folder in
                                RecentFolderRow(
                                    item: folder,
                                    fileCount: recentFiles(in: folder).count,
                                    expanded: expandedRecents.contains(folder.path),
                                    label: label,
                                    muted: muted,
                                    onToggle: {
                                        withAnimation(.snappy(duration: 0.22)) {
                                            if expandedRecents.contains(folder.path) {
                                                expandedRecents.remove(folder.path)
                                            } else {
                                                expandedRecents.insert(folder.path)
                                            }
                                        }
                                    },
                                    onOpen: { onOpenRecent(folder.path) }
                                )
                                if expandedRecents.contains(folder.path) {
                                    let files = recentFiles(in: folder)
                                    if files.isEmpty {
                                        Text("No recent files in this project")
                                            .font(.system(size: 11))
                                            .foregroundStyle(muted.opacity(0.8))
                                            .padding(.leading, 46)
                                            .padding(.vertical, 4)
                                            .transition(.opacity)
                                    } else {
                                        ForEach(files) { file in
                                            RecentRow(
                                                item: file,
                                                label: label,
                                                muted: muted,
                                                indented: true
                                            ) {
                                                onOpenRecent(file.path)
                                            }
                                            .transition(.opacity.combined(with: .move(edge: .top)))
                                        }
                                    }
                                }
                            }

                            // ── Loose files (not under a recent project)
                            let loose = looseRecentFiles
                            if !loose.isEmpty {
                                if !recentFolders.isEmpty {
                                    Text("Files")
                                        .font(.system(size: 11, weight: .semibold))
                                        .foregroundStyle(muted.opacity(0.9))
                                        .padding(.horizontal, 10)
                                        .padding(.top, 10)
                                        .padding(.bottom, 2)
                                }
                                ForEach(loose) { item in
                                    RecentRow(item: item, label: label, muted: muted) {
                                        onOpenRecent(item.path)
                                    }
                                }
                            }
                        }
                        .padding(.horizontal, 18)
                        .padding(.bottom, 20)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }

    private func welcomeButton(
        systemImage: String,
        title: String,
        action: @escaping () -> Void
    ) -> some View {
        WelcomeActionButton(
            systemImage: systemImage,
            title: title,
            fill: buttonFill,
            label: label,
            action: action
        )
    }
}

/// Capsule action with an iOS-quality hover lift + press scale.
private struct WelcomeActionButton: View {
    var systemImage: String
    var title: String
    var fill: Color
    var label: Color
    var action: () -> Void
    @State private var hovering = false
    @State private var pressed = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 11) {
                Image(systemName: systemImage)
                    .font(.system(size: 13, weight: .regular))
                    .foregroundStyle(Color.white.opacity(hovering ? 0.92 : 0.70))
                    .frame(width: 18, alignment: .center)
                Text(title)
                    .font(.system(size: 13, weight: .regular, design: .default))
                    .foregroundStyle(label)
                Spacer(minLength: 0)
                Image(systemName: "chevron.right")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(Color.white.opacity(0.45))
                    .opacity(hovering ? 1 : 0)
                    .offset(x: hovering ? 0 : -4)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 11)
            .background(
                Capsule(style: .continuous)
                    .fill(Color.white.opacity(hovering ? 0.12 : 0.075))
            )
            .overlay(
                Capsule(style: .continuous)
                    .strokeBorder(Color.white.opacity(hovering ? 0.14 : 0.0), lineWidth: 1)
            )
            .scaleEffect(pressed ? 0.985 : 1)
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(.snappy(duration: 0.18), value: hovering)
        .animation(.snappy(duration: 0.12), value: pressed)
        .simultaneousGesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in pressed = true }
                .onEnded { _ in pressed = false }
        )
    }
}

/// Project folder row — click expands its recent files; the trailing button opens it.
private struct RecentFolderRow: View {
    var item: RecentItem
    var fileCount: Int
    var expanded: Bool
    var label: Color
    var muted: Color
    var onToggle: () -> Void
    var onOpen: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: onToggle) {
            HStack(spacing: 10) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(muted)
                    .rotationEffect(.degrees(expanded ? 90 : 0))
                    .frame(width: 10)
                Image(systemName: "folder.fill")
                    .foregroundStyle(Color(red: 0.42, green: 0.62, blue: 0.98).opacity(hovering || expanded ? 1 : 0.8))
                    .frame(width: 18)
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(.system(size: 13, weight: .medium, design: .default))
                        .foregroundStyle(label)
                        .lineLimit(1)
                    Text(item.subtitle)
                        .font(.system(size: 11, design: .default))
                        .foregroundStyle(muted)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 0)
                if fileCount > 0, !hovering {
                    Text("\(fileCount)")
                        .font(.system(size: 10, weight: .medium, design: .rounded))
                        .foregroundStyle(muted)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(Color.white.opacity(0.07)))
                }
                // Open the project itself (row click only expands).
                Button(action: onOpen) {
                    Image(systemName: "arrow.forward.circle.fill")
                        .symbolRenderingMode(.hierarchical)
                        .font(.system(size: 15))
                        .foregroundStyle(Color.white.opacity(0.60))
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .help("Open Project")
                .opacity(hovering ? 1 : 0)
                .offset(x: hovering ? 0 : -6)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                    .fill(hovering ? Color.white.opacity(0.10) : Color.white.opacity(0.05))
            )
            .contentShape(RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(.snappy(duration: 0.16), value: hovering)
        .animation(.snappy(duration: 0.18), value: expanded)
    }
}

private struct RecentRow: View {
    var item: RecentItem
    var label: Color
    var muted: Color
    var indented: Bool = false
    var action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: item.isDir ? "folder.fill" : "doc.text")
                    .foregroundStyle(
                        item.isDir
                            ? Color(red: 0.42, green: 0.62, blue: 0.98).opacity(hovering ? 1 : 0.8)
                            : Color.white.opacity(hovering ? 0.70 : 0.50)
                    )
                    .frame(width: 18)
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(.system(size: indented ? 12 : 13, weight: .medium, design: .default))
                        .foregroundStyle(label)
                        .lineLimit(1)
                    if !indented {
                        Text(item.subtitle)
                            .font(.system(size: 11, design: .default))
                            .foregroundStyle(muted)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Spacer(minLength: 0)
                Image(systemName: "arrow.forward.circle.fill")
                    .symbolRenderingMode(.hierarchical)
                    .font(.system(size: 14))
                    .foregroundStyle(Color.white.opacity(0.55))
                    .opacity(hovering ? 1 : 0)
                    .offset(x: hovering ? 0 : -6)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, indented ? 5 : 8)
            .padding(.leading, indented ? 28 : 0)
            .background(
                RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                    .fill(hovering ? Color.white.opacity(0.10) : Color.white.opacity(0.05))
            )
            .contentShape(RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(.snappy(duration: 0.16), value: hovering)
    }
}

struct RecentItem: Identifiable, Equatable, Codable {
    var id: String { path }
    var path: String
    var title: String
    var subtitle: String
    var isDir: Bool
}

enum RecentStore {
    private static let key = "suisei.recents"

    static func load() -> [RecentItem] {
        guard let data = UserDefaults.standard.data(forKey: key),
              let items = try? JSONDecoder().decode([RecentItem].self, from: data)
        else { return [] }
        return items
    }

    static func push(path: String) {
        var items = load()
        items.removeAll { $0.path == path }
        let url = URL(fileURLWithPath: path)
        let isDir = (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
        let title = url.lastPathComponent
        let parent = url.deletingLastPathComponent().path
        items.insert(
            RecentItem(path: path, title: title, subtitle: parent, isDir: isDir),
            at: 0
        )
        if items.count > 12 { items = Array(items.prefix(12)) }
        if let data = try? JSONEncoder().encode(items) {
            UserDefaults.standard.set(data, forKey: key)
        }
    }
}

// MARK: - Plain-window AppKit chrome (rounded + draggable)

/// Shared AppKit apply path for the Welcome plain window.
enum WelcomeChromeApplier {
    static func apply(to window: NSWindow, cornerRadius: CGFloat) {
        // 1) Transparent host so continuous corners show the desktop, not a square slab.
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = true
        window.isMovable = true
        window.isMovableByWindowBackground = true

        // 2) Continuous corner mask on the window content view (the exterior shape).
        if let cv = window.contentView {
            cv.wantsLayer = true
            cv.layer?.cornerRadius = cornerRadius
            cv.layer?.cornerCurve = .continuous
            cv.layer?.masksToBounds = true
            // Solid fill under clipped content so corners don't flash through wrong color.
            cv.layer?.backgroundColor = NSColor(
                red: 0.145, green: 0.145, blue: 0.150, alpha: 1
            ).cgColor
        }

        // Also mask the immediate hosting subview (SwiftUI often draws on a child).
        if let host = window.contentView?.subviews.first {
            host.wantsLayer = true
            host.layer?.cornerRadius = cornerRadius
            host.layer?.cornerCurve = .continuous
            host.layer?.masksToBounds = true
        }

        // 3) Fixed compact size — prevent content-size stretch.
        let size = WelcomeView.windowSize
        let content = window.contentLayoutRect.size
        if abs(content.width - size.width) > 0.5 || abs(content.height - size.height) > 0.5 {
            window.setContentSize(size)
        }
        var frameSize = size
        let deltaW = window.frame.width - window.contentLayoutRect.width
        let deltaH = window.frame.height - window.contentLayoutRect.height
        frameSize.width += max(0, deltaW)
        frameSize.height += max(0, deltaH)
        window.minSize = frameSize
        window.maxSize = frameSize

        // 4) Shadow must recompute after radius/opaque changes or it stays rectangular.
        window.invalidateShadow()
    }

    /// Best-effort find of the Welcome plain window (never the editor shell).
    static func applyToWelcomeWindows(cornerRadius: CGFloat = WelcomeView.cornerRadius) {
        for window in NSApp.windows {
            let id = window.identifier?.rawValue ?? ""
            let titledWelcome = window.title == "Welcome"
            let idWelcome = id == "welcome"
            let plainSmall = !window.styleMask.contains(.titled)
                && window.isVisible
                && window.frame.width < 900
                && window.frame.height < 560
            if titledWelcome || idWelcome || plainSmall {
                apply(to: window, cornerRadius: cornerRadius)
            }
        }
    }
}

/// `.windowStyle(.plain)` strips system chrome: square exterior, no background-drag.
/// Re-apply continuous corners, clear opacity, and movable background the AppKit way.
private struct WelcomeWindowChrome: NSViewRepresentable {
    var cornerRadius: CGFloat

    func makeNSView(context: Context) -> WelcomeChromeProbe {
        let v = WelcomeChromeProbe(cornerRadius: cornerRadius)
        // Zero size, *not* hidden — hidden probes often never attach to a window.
        v.frame = .zero
        return v
    }

    func updateNSView(_ nsView: WelcomeChromeProbe, context: Context) {
        nsView.cornerRadius = cornerRadius
        nsView.applyChrome()
    }
}

/// Probe that configures its host `NSWindow` once attached (and on layout).
final class WelcomeChromeProbe: NSView {
    var cornerRadius: CGFloat

    init(cornerRadius: CGFloat) {
        self.cornerRadius = cornerRadius
        super.init(frame: .zero)
        isHidden = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// Empty areas of the panel can drag the window (buttons still receive hits above).
    override var mouseDownCanMoveWindow: Bool { true }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        applyChrome()
    }

    override func viewDidMoveToSuperview() {
        super.viewDidMoveToSuperview()
        applyChrome()
    }

    override func layout() {
        super.layout()
        applyChrome()
    }

    func applyChrome() {
        guard let window else { return }
        WelcomeChromeApplier.apply(to: window, cornerRadius: cornerRadius)
    }

    override func mouseDown(with event: NSEvent) {
        // Fallback drag if background-drag flag is ignored by the hosting stack.
        window?.performDrag(with: event)
    }
}

// MARK: - Window sizing (welcome compact → editor expanded)

enum SuiseiWindowLayout {
    static let welcomeSize = WelcomeView.windowSize
    static let editorSize = NSSize(width: 1280, height: 820)
    static let editorMinSize = NSSize(width: 900, height: 560)

    /// Resize + center the **editor** window after leaving Welcome.
    /// Welcome chrome is owned by SwiftUI scene modifiers (not AppKit button hiding).
    static func apply(welcome: Bool, animate: Bool = true) {
        // Prefer the editor WindowGroup window when leaving welcome.
        let window = NSApp.windows.first(where: {
            $0.isVisible && $0.title != "Settings" && $0.title != "Welcome"
        })
            ?? NSApp.windows.first(where: { $0.isVisible || $0.isMainWindow })
            ?? NSApp.mainWindow
            ?? NSApp.windows.first
        guard let window else { return }

        if welcome {
            // Welcome Window scene sizes itself via .windowResizability(.contentSize).
            return
        }

        let target = editorSize
        window.minSize = editorMinSize
        window.maxSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )
        window.isMovableByWindowBackground = false
        window.isOpaque = true
        window.titlebarAppearsTransparent = true

        // Already editor-sized (SwiftUI defaultSize) → re-framing here is what
        // made freshly opened windows visibly snap sideways. Leave it alone.
        if abs(window.frame.width - target.width) < 80,
           abs(window.frame.height - target.height) < 80
        {
            return
        }

        let screen = window.screen ?? NSScreen.main
        let visible = screen?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1400, height: 900)
        var frame = window.frame
        frame.size = target
        frame.origin.x = visible.midX - target.width / 2
        frame.origin.y = visible.midY - target.height / 2
        frame.origin.x = max(visible.minX + 20, min(frame.origin.x, visible.maxX - target.width - 20))
        frame.origin.y = max(visible.minY + 20, min(frame.origin.y, visible.maxY - target.height - 20))

        if animate {
            NSAnimationContext.runAnimationGroup { ctx in
                ctx.duration = 0.28
                ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                window.animator().setFrame(frame, display: true)
            }
        } else {
            window.setFrame(frame, display: true)
        }
    }
}
