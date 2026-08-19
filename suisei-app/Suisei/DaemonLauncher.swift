import Foundation

/// Starts the Suisei daemon on app launch, **detached** so it outlives the app
/// (macOS reparents a spawned child to launchd on app exit — we never wait on
/// or kill it). The daemon in turn launches the menu-bar agent. Idempotent
/// because the DAEMON makes it so — it probes the socket and stands down when
/// one of its own version is already serving. See `ensureRunning`.
///
/// This is the pragmatic form of arch-plan §1.5's launchd LaunchAgent — good
/// enough for "open the app → daemon appears; close the app → daemon stays".
enum DaemonLauncher {
    static func ensureRunning() {
        // **Always spawn it. The daemon decides.**
        //
        // This used to skip the spawn whenever something ACCEPTED a connection
        // on the socket, which answers the wrong question: "is a daemon
        // running" rather than "is a daemon that can serve THIS app running".
        // After an update the old daemon is still there and still accepting, so
        // the new one was never started — and the old one refuses the editor's
        // handshake with a version Nak, so nothing could report. The app looked
        // like it had started its daemon (it had, weeks ago) and the menu bar
        // showed no LSP and no DAP.
        //
        // The daemon knows how to answer this properly: it probes the socket,
        // stands down when a daemon of its own version is serving, and asks an
        // older one to quit before taking over. One place owns the decision,
        // and it is the place that can read the version off the wire. The cost
        // is a process that exits immediately on most launches.
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
}
