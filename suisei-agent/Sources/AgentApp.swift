import SwiftUI

/// The Suisei daemon's menu-bar presence (arch plan §1.5). A headless
/// `LSUIElement` agent: no Dock icon, no window — just the blackhole glyph in
/// the menu bar, and a status popover on click. It owns no state; it subscribes
/// to the daemon over the Unix socket.
@main
struct SuiseiDaemonAgentApp: App {
    @StateObject private var client = DaemonClient()

    var body: some Scene {
        MenuBarExtra {
            DaemonStatusView(client: client)
        } label: {
            Image(nsImage: Self.menuBarIcon)
        }
        .menuBarExtraStyle(.window) // rich popover, not a plain menu
    }

    /// The blackhole silhouette as a template image — macOS tints it to match
    /// the menu bar (black in light, white in dark) from its alpha alone.
    static let menuBarIcon: NSImage = {
        let path = Bundle.main.path(forResource: "StatusIcon", ofType: "png")
        let img = (path.flatMap { NSImage(contentsOfFile: $0) }) ?? NSImage()
        img.isTemplate = true
        // Standard menu-bar size: an 18pt frame. The PNG carries ~18% padding
        // around the shape so the glyph sits at ~15pt with breathing room, like
        // the neighbouring status items (not edge-to-edge, not a speck).
        img.size = NSSize(width: 18, height: 18)
        return img
    }()
}

struct DaemonStatusView: View {
    @ObservedObject var client: DaemonClient

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider().padding(.vertical, 8)

            if client.connected {
                VStack(alignment: .leading, spacing: 9) {
                    row("LSP", lsp.text, lsp.color, systemImage: "chevron.left.forwardslash.chevron.right")
                    row("DAP", dap.text, dap.color, systemImage: "ladybug")
                    row("Project", projectText, .primary, systemImage: "folder")
                    row("Stability", health.text, health.color, systemImage: "waveform.path.ecg")
                }
            } else {
                offline
            }

            Divider().padding(.vertical, 8)
            footer
        }
        .padding(14)
        .frame(width: 288)
    }

    // ── sections ────────────────────────────────────────────────────────────
    private var header: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(client.connected ? Color.green : Color.secondary)
                .frame(width: 8, height: 8)
            Text("Suisei Daemon").font(.system(size: 13, weight: .semibold))
            Spacer()
            Text(client.connected ? "connected" : "offline")
                .font(.system(size: 11)).foregroundStyle(.secondary)
        }
    }

    private var offline: some View {
        HStack(spacing: 8) {
            Image(systemName: "bolt.horizontal.circle")
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text("Daemon not running").font(.system(size: 12, weight: .medium))
                Text("Start it with the app, or `cargo run -p suisei-daemon`.")
                    .font(.system(size: 11)).foregroundStyle(.secondary)
            }
        }
    }

    private var footer: some View {
        HStack {
            Text(client.connected ? "uptime \(uptimeText)" : "—")
                .font(.system(size: 11)).foregroundStyle(.secondary)
            Spacer()
            // Stops the daemon too — the daemon supervises this agent, so
            // quitting the agent alone would just respawn it.
            Button(client.connected ? "Quit Daemon" : "Quit Agent") {
                if client.connected { client.quit() } else { NSApp.terminate(nil) }
            }
            .controlSize(.small)
        }
    }

    private func row(_ label: String, _ value: String, _ color: Color, systemImage: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: systemImage)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .frame(width: 16)
            Text(label)
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .frame(width: 66, alignment: .leading)
            Circle().fill(color).frame(width: 7, height: 7)
            Text(value)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 0)
        }
    }

    // ── derived display ──────────────────────────────────────────────────────
    private var lsp: (text: String, color: Color) {
        switch client.status.lspState {
        case 1: return ("starting", .orange)
        case 2: return ("indexing (\(client.status.lspSessions))", .orange)
        case 3: return ("ready (\(client.status.lspSessions))", .green)
        case 4: return ("error", .red)
        default: return ("none", .secondary)
        }
    }
    private var dap: (text: String, color: Color) {
        switch client.status.dapState {
        case 1: return ("running", .green)
        case 2: return ("paused", .orange)
        default: return ("none", .secondary)
        }
    }
    private var health: (text: String, color: Color) {
        switch client.status.health {
        case 1: return ("healthy", .green)
        case 2: return ("degraded", .orange)
        default: return ("starting", .secondary)
        }
    }
    private var projectText: String {
        let p = client.status.project
        return p.isEmpty ? "none" : (p as NSString).lastPathComponent
    }
    private var uptimeText: String {
        let s = client.status.uptimeSecs
        if s < 60 { return "\(s)s" }
        if s < 3600 { return "\(s / 60)m" }
        return "\(s / 3600)h \((s % 3600) / 60)m"
    }
}
