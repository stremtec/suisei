import AppKit
import SwiftUI

/// Replacing the app with the build that was staged for this launch.
///
/// Suisei is not signed, so a downloaded binary would put the receiver through
/// System Settings → Privacy & Security on *every* update. An update built on
/// the machine that will run it never carries `com.apple.quarantine`, so
/// Gatekeeper never asks. The cost is that updating is a build, and a build
/// cannot happen while you are using the editor it replaces — so it happens in
/// the background, and the result is applied here, at the next launch.
///
/// **This runs before any window exists.** The exchange itself is one atomic
/// rename and takes milliseconds; what takes a moment is relaunching. A panel
/// makes that legible — an app that vanishes and comes back with no
/// explanation reads as a crash.
enum SoftwareUpdateApply {
    static var currentVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
    }

    /// The bundle to replace: the one this process is running out of.
    ///
    /// Not a guess at `/Applications/Suisei.app`. Someone running a copy from
    /// their Downloads folder should have THAT copy updated, and replacing a
    /// different bundle than the one they launched would leave them looking at
    /// the old version wondering why nothing changed.
    static var appPath: String { Bundle.main.bundlePath }

    /// Apply a staged update, if one is waiting. Returns true when the app is
    /// about to relaunch and the caller should stop setting itself up.
    @discardableResult
    static func applyIfPending() -> Bool {
        var buf = [CChar](repeating: 0, count: 64)
        let n = currentVersion.withCString { cur in
            suisei_engine_update_pending(cur, &buf, 64)
        }
        guard n > 0 else { return false }
        let version = String(cString: buf)

        let panel = progressPanel(version: version)
        panel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        // One turn of the run loop so the panel actually draws before the
        // synchronous swap below.
        RunLoop.current.run(until: Date().addingTimeInterval(0.05))

        var err = [CChar](repeating: 0, count: 512)
        let rc = currentVersion.withCString { cur in
            appPath.withCString { app in
                suisei_engine_update_apply(cur, app, &err, 512)
            }
        }
        panel.close()

        guard rc == 0 else {
            // The installed app is untouched — the swap is atomic, so a failure
            // means it did not happen rather than that it half did. Say so and
            // carry on into the version they already have.
            let message = String(cString: err)
            let alert = NSAlert()
            alert.messageText = "Suisei could not finish updating"
            alert.informativeText = message.isEmpty
                ? "The update was not applied. You are still running \(currentVersion)."
                : "\(message)\n\nYou are still running \(currentVersion)."
            alert.alertStyle = .warning
            alert.runModal()
            return false
        }

        relaunch()
        return true
    }

    private static func progressPanel(version: String) -> NSWindow {
        let w = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 360, height: 110),
            styleMask: [.titled, .fullSizeContentView],
            backing: .buffered, defer: false
        )
        w.titlebarAppearsTransparent = true
        w.titleVisibility = .hidden
        w.isMovableByWindowBackground = true
        w.center()
        w.contentView = NSHostingView(
            rootView: VStack(spacing: 10) {
                ProgressView().controlSize(.small)
                Text("Updating to \(version)…")
                    .font(.system(size: 13))
                Text("Suisei will restart.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            .frame(width: 360, height: 110)
        )
        return w
    }

    /// Start the new bundle and let this process go.
    ///
    /// `open` rather than exec: this process is still running the OLD binary
    /// out of the old inode, which the swap left alive. Only a fresh launch
    /// picks up what is now at the path.
    private static func relaunch() {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        task.arguments = ["-n", appPath]
        try? task.run()
        NSApp.terminate(nil)
    }
}
