import SwiftUI

/// Polls the daemon's status on a background queue and republishes it on the
/// main actor for the menu-bar UI. A failed poll means the daemon is offline —
/// the socket connect simply fails, no exception.
@MainActor
final class DaemonClient: ObservableObject {
    @Published private(set) var connected = false
    @Published private(set) var status = DaemonStatus()
    /// Wall-clock of the last successful poll (drives a stale indicator).
    @Published private(set) var lastSeen: Date?

    private let socketPath = DaemonSocket.defaultPath()
    private var timer: Timer?

    init() {
        poll()
        timer = Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
    }

    deinit { timer?.invalidate() }

    /// Path shown in the UI so the user can see where the daemon lives.
    var socketDisplayPath: String { socketPath }

    /// Stop the daemon, then quit the agent. Because the daemon supervises the
    /// agent, quitting the agent alone would just get it respawned — so the
    /// off-switch stops the daemon first.
    func quit() {
        let path = socketPath
        DispatchQueue.global(qos: .userInitiated).async {
            DaemonSocket.sendShutdown(socketPath: path)
            DispatchQueue.main.async { NSApplication.shared.terminate(nil) }
        }
    }

    func poll() {
        let path = socketPath
        DispatchQueue.global(qos: .utility).async {
            let fetched = DaemonSocket.fetchStatus(socketPath: path)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if let s = fetched {
                    self.status = s
                    self.connected = true
                    self.lastSeen = Date()
                } else {
                    self.connected = false
                }
            }
        }
    }
}
