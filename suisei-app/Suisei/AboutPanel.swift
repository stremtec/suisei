import AppKit
import SwiftUI

/// Owns the single app-level About panel.
///
/// This is intentionally not a Settings destination. AppKit owns the window
/// chrome and lifecycle while SwiftUI supplies only the branded content, which
/// matches the architecture used by Xcode's About window without relying on a
/// private framework or recreating traffic lights.
@MainActor
final class AboutPanelController {
    static let shared = AboutPanelController()

    private var panel: NSPanel?

    private init() {}

    func show() {
        if panel == nil {
            panel = makePanel()
        }

        guard let panel else { return }
        panel.center()
        panel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    private func makePanel() -> NSPanel {
        let size = NSSize(width: 530, height: 212)
        let panel = NSPanel(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )

        panel.title = "About Suisei"
        panel.identifier = NSUserInterfaceItemIdentifier("suisei.window.about")
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.isReleasedWhenClosed = false
        panel.isOpaque = true
        panel.backgroundColor = .windowBackgroundColor
        panel.contentMinSize = size
        panel.contentMaxSize = size
        panel.standardWindowButton(.miniaturizeButton)?.isEnabled = false
        panel.standardWindowButton(.zoomButton)?.isEnabled = false
        panel.contentView = NSHostingView(rootView: AboutPanelView())

        return panel
    }

    static func openLicense() {
        guard let url = Bundle.main.url(forResource: "LICENSE", withExtension: nil) else {
            NSSound.beep()
            return
        }
        NSWorkspace.shared.open(url)
    }
}

private struct AboutPanelView: View {
    private var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "—"
    }

    private var build: String? {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String
    }

    private var versionLine: String {
        guard let build, !build.isEmpty else { return "Version \(version)" }
        return "Version \(version) (\(build))"
    }

    private var copyright: String {
        Bundle.main.object(forInfoDictionaryKey: "NSHumanReadableCopyright") as? String
            ?? "Copyright © 2026 Stremtec. All rights reserved."
    }

    var body: some View {
        HStack(alignment: .top, spacing: 34) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .interpolation(.high)
                .aspectRatio(1, contentMode: .fit)
                .frame(width: 104, height: 104)
                .shadow(color: .black.opacity(0.18), radius: 5, y: 2)
                .padding(.top, 31)

            VStack(alignment: .leading, spacing: 0) {
                Text("Suisei")
                    .font(.system(size: 34, weight: .regular))

                Text(versionLine)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .padding(.top, 1)

                Spacer(minLength: 20)

                Text(copyright)
                    .font(.system(size: 9.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)

                Spacer(minLength: 12)

                HStack {
                    Spacer(minLength: 0)
                    Button("License Agreement") {
                        AboutPanelController.openLicense()
                    }
                    .controlSize(.regular)
                }
            }
            .padding(.top, 20)
            .padding(.bottom, 14)
        }
        .padding(.leading, 44)
        .padding(.trailing, 20)
        .frame(width: 530, height: 212)
        .background(Color(nsColor: .windowBackgroundColor))
    }
}
