import Foundation

/// Starts the Suisei daemon on app launch, **detached** so it outlives the app
/// (macOS reparents a spawned child to launchd on app exit — we never wait on
/// or kill it). The daemon in turn launches the menu-bar agent. Idempotent: if
/// the socket already accepts a connection, the daemon is up and we do nothing.
///
/// This is the pragmatic form of arch-plan §1.5's launchd LaunchAgent — good
/// enough for "open the app → daemon appears; close the app → daemon stays".
enum DaemonLauncher {
    static func ensureRunning() {
        let sock = socketPath()
        if isDaemonUp(sock) { return }
        guard let bin = daemonBinaryPath() else {
            NSLog("Suisei: daemon binary not found — menu-bar status unavailable")
            return
        }
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: bin)
        var env = ProcessInfo.processInfo.environment
        if let agent = agentAppPath() { env["SUISEI_AGENT_APP"] = agent }
        proc.environment = env
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        do {
            try proc.run()
            NSLog("Suisei: spawned daemon at \(bin)")
        } catch {
            NSLog("Suisei: failed to start daemon: \(error)")
        }
        // Intentionally do not retain, wait on, or terminate `proc`: the daemon
        // must survive this app.
    }

    static func socketPath() -> String {
        let env = ProcessInfo.processInfo.environment
        if let x = env["XDG_RUNTIME_DIR"], !x.isEmpty { return "\(x)/suisei/daemon.sock" }
        return NSHomeDirectory() + "/Library/Application Support/Suisei/daemon.sock"
    }

    /// Bundled helper: `Suisei.app/Contents/Helpers/suisei-daemon`; `$SUISEI_DAEMON_BIN`
    /// overrides for development (point it at `target/debug/suisei-daemon`).
    private static func daemonBinaryPath() -> String? {
        if let env = ProcessInfo.processInfo.environment["SUISEI_DAEMON_BIN"],
           FileManager.default.isExecutableFile(atPath: env) {
            return env
        }
        let bundled = Bundle.main.bundleURL
            .appendingPathComponent("Contents/Helpers/suisei-daemon").path
        return FileManager.default.isExecutableFile(atPath: bundled) ? bundled : nil
    }

    private static func agentAppPath() -> String? {
        if let env = ProcessInfo.processInfo.environment["SUISEI_AGENT_APP"] { return env }
        let bundled = Bundle.main.bundleURL
            .appendingPathComponent("Contents/Helpers/SuiseiDaemonAgent.app").path
        return FileManager.default.fileExists(atPath: bundled) ? bundled : nil
    }

    private static func isDaemonUp(_ path: String) -> Bool {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let cap = MemoryLayout.size(ofValue: addr.sun_path)
        let pathC = Array(path.utf8CString)
        guard pathC.count <= cap else { return false }
        withUnsafeMutablePointer(to: &addr.sun_path) { raw in
            raw.withMemoryRebound(to: CChar.self, capacity: cap) { dst in
                for (i, b) in pathC.enumerated() { dst[i] = b }
            }
        }
        return withUnsafePointer(to: &addr) { ap in
            ap.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                connect(fd, sa, socklen_t(MemoryLayout<sockaddr_un>.size)) == 0
            }
        }
    }
}
