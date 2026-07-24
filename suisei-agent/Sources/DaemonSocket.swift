import Foundation

/// Wire contract with `suisei-daemon` (see suisei-daemon/src/protocol.rs).
/// Frame: u32 len LE ‖ u16 opcode LE ‖ u16 version LE ‖ payload.
enum DaemonProto {
    static let version: UInt16 = 1
    static let maxFrame: UInt32 = 64 * 1024 * 1024

    enum Op: UInt16 {
        case hello = 1, helloAck = 2, helloNak = 3
        case ping = 4, pong = 5
        case statusRequest = 6, statusReport = 7
    }
}

/// Decoded `Status` (matches `Status` in protocol.rs).
struct DaemonStatus: Equatable {
    var lspSessions: UInt16 = 0
    var lspState: UInt8 = 0   // 0 none 1 starting 2 indexing 3 ready 4 error
    var dapState: UInt8 = 0   // 0 none 1 running 2 paused
    var health: UInt8 = 0     // 0 starting 1 healthy 2 degraded
    var uptimeSecs: UInt64 = 0
    var project: String = ""
}

/// Blocking Unix-socket client. One status poll = connect → Hello → Ack →
/// StatusRequest → StatusReport → close. Cheap enough to run every ~2s on a
/// background queue; a short-lived connection avoids any long-lived read loop.
enum DaemonSocket {
    /// Resolve the socket path the same way the daemon does.
    static func defaultPath() -> String {
        let env = ProcessInfo.processInfo.environment
        if let x = env["XDG_RUNTIME_DIR"], !x.isEmpty {
            return "\(x)/suisei/daemon.sock"
        }
        return NSHomeDirectory() + "/Library/Application Support/Suisei/daemon.sock"
    }

    static func fetchStatus(socketPath: String) -> DaemonStatus? {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return nil }
        defer { close(fd) }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let cap = MemoryLayout.size(ofValue: addr.sun_path)
        let pathC = Array(socketPath.utf8CString) // includes NUL
        guard pathC.count <= cap else { return nil } // sun_path overflow
        withUnsafeMutablePointer(to: &addr.sun_path) { raw in
            raw.withMemoryRebound(to: CChar.self, capacity: cap) { dst in
                for (i, b) in pathC.enumerated() { dst[i] = b }
            }
        }
        let connected = withUnsafePointer(to: &addr) { ap -> Int32 in
            ap.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                connect(fd, sa, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { return nil }

        guard writeFrame(fd, .hello, []),
              let ack = readFrame(fd), ack.op == .helloAck,
              writeFrame(fd, .statusRequest, []),
              let rep = readFrame(fd), rep.op == .statusReport
        else { return nil }
        return decodeStatus(rep.payload)
    }

    // ── framing ───────────────────────────────────────────────────────────────
    private static func le16(_ v: UInt16) -> [UInt8] { [UInt8(v & 0xff), UInt8(v >> 8)] }
    private static func le32(_ v: UInt32) -> [UInt8] {
        [UInt8(v & 0xff), UInt8((v >> 8) & 0xff), UInt8((v >> 16) & 0xff), UInt8((v >> 24) & 0xff)]
    }

    private static func writeFrame(_ fd: Int32, _ op: DaemonProto.Op, _ payload: [UInt8]) -> Bool {
        var body = le16(op.rawValue) + le16(DaemonProto.version) + payload
        var frame = le32(UInt32(body.count))
        frame.append(contentsOf: body)
        body.removeAll()
        return writeAll(fd, frame)
    }

    private static func readFrame(_ fd: Int32) -> (op: DaemonProto.Op, payload: [UInt8])? {
        guard let lenB = readN(fd, 4) else { return nil }
        let len = UInt32(lenB[0]) | (UInt32(lenB[1]) << 8) | (UInt32(lenB[2]) << 16) | (UInt32(lenB[3]) << 24)
        guard len >= 4, len <= DaemonProto.maxFrame, let body = readN(fd, Int(len)) else { return nil }
        let opRaw = UInt16(body[0]) | (UInt16(body[1]) << 8)
        guard let op = DaemonProto.Op(rawValue: opRaw) else { return nil }
        return (op, Array(body[4...]))
    }

    private static func writeAll(_ fd: Int32, _ bytes: [UInt8]) -> Bool {
        var off = 0
        return bytes.withUnsafeBytes { raw -> Bool in
            while off < bytes.count {
                let n = write(fd, raw.baseAddress!.advanced(by: off), bytes.count - off)
                if n <= 0 { return false }
                off += n
            }
            return true
        }
    }

    private static func readN(_ fd: Int32, _ count: Int) -> [UInt8]? {
        var buf = [UInt8](repeating: 0, count: count)
        var off = 0
        let ok = buf.withUnsafeMutableBytes { raw -> Bool in
            while off < count {
                let n = read(fd, raw.baseAddress!.advanced(by: off), count - off)
                if n <= 0 { return false }
                off += n
            }
            return true
        }
        return ok ? buf : nil
    }

    // ── Status payload (matches Status::encode) ────────────────────────────────
    private static func decodeStatus(_ p: [UInt8]) -> DaemonStatus? {
        guard p.count >= 16 + 512 else { return nil }
        func u16(_ i: Int) -> UInt16 { UInt16(p[i]) | (UInt16(p[i + 1]) << 8) }
        var uptime: UInt64 = 0
        for k in 0..<8 { uptime |= UInt64(p[8 + k]) << (8 * k) }
        let proj = p[16..<(16 + 512)]
        let end = proj.firstIndex(of: 0).map { $0 - 16 } ?? 512
        let project = String(decoding: p[16..<(16 + end)], as: UTF8.self)
        return DaemonStatus(
            lspSessions: u16(0),
            lspState: p[2], dapState: p[3], health: p[4],
            uptimeSecs: uptime, project: project
        )
    }
}
