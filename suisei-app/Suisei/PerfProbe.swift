import Foundation

/// Opt-in timing for the face's hot paths. Off unless `SUISEI_PERF=1` is in the
/// environment, and when off `measure` is a direct call with no clock reads —
/// so this can live in the shipping build.
///
/// It exists because guessing has a bad record here: the Rust tick and
/// `dispatch_key` both measure well under a frame even on a 60,000-line file
/// (`tick_breakdown`, `keystroke_latency`), so the typing latency users report
/// has to be on this side of the C ABI. Numbers, not hunches.
///
/// ```text
/// SUISEI_PERF=1 suisei-app/.build/Suisei.app/Contents/MacOS/Suisei
/// ```
enum PerfProbe {
    static let enabled = ProcessInfo.processInfo.environment["SUISEI_PERF"] == "1"

    /// How often the accumulated numbers are dumped to stderr.
    private static let reportInterval: TimeInterval = 2.0

    private struct Bucket {
        var count = 0
        var total = 0.0
        var worst = 0.0
        mutating func add(_ ms: Double) {
            count += 1
            total += ms
            worst = max(worst, ms)
        }
    }

    private static var buckets: [String: Bucket] = [:]
    private static var lastReport = Date()

    /// Time `body` under `label`. Returns whatever `body` returns.
    @inline(__always)
    static func measure<T>(_ label: String, _ body: () -> T) -> T {
        guard enabled else { return body() }
        let t = DispatchTime.now().uptimeNanoseconds
        let out = body()
        record(label, Double(DispatchTime.now().uptimeNanoseconds - t) / 1_000_000)
        return out
    }

    /// Record a duration measured elsewhere.
    static func record(_ label: String, _ ms: Double) {
        guard enabled else { return }
        buckets[label, default: Bucket()].add(ms)
        if Date().timeIntervalSince(lastReport) >= reportInterval {
            report()
        }
    }

    private static func report() {
        lastReport = Date()
        guard !buckets.isEmpty else { return }
        var out = "── suisei perf (last \(Int(reportInterval))s) ──\n"
        for (label, b) in buckets.sorted(by: { $0.value.total > $1.value.total }) {
            out += String(
                format: "  %-34s n=%-5d mean=%7.3fms  max=%7.3fms  total=%8.2fms\n",
                (label as NSString).utf8String!, b.count, b.total / Double(b.count), b.worst, b.total
            )
        }
        FileHandle.standardError.write(Data(out.utf8))
        buckets.removeAll(keepingCapacity: true)
    }
}
