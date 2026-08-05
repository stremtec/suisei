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

/// Opt-in trace for tab-strip hit-testing. Off unless `SUISEI_TABLOG=1`.
///
/// Exists because two reported strip bugs — clicking chip A switching to chip
/// C, and a detached layout member becoming unreachable — are both "the pointer
/// resolved to the wrong tab", and every attempt to fix them by reasoning about
/// coordinate spaces has been wrong. This prints the actual arithmetic: the
/// probe x, the row origin it was corrected by, and every frame on file with
/// its bounds, next to the live tab list. Whatever is wrong (stale rect,
/// constant offset, missing key) is then visible rather than inferred.
enum TabLog {
    static let enabled = ProcessInfo.processInfo.environment["SUISEI_TABLOG"] == "1"

    /// Which chip SwiftUI's OWN hit-testing thinks the cursor is over.
    ///
    /// Separate from `dump` because hover and click travel different paths:
    /// clicks go through `TabStripMouse` -> `tabSlot` -> the measured frames,
    /// hover goes through SwiftUI's hit-testing on the chip itself. A report
    /// that hovering chip A highlights chip C can therefore be true while the
    /// click log looks perfect, and only this line can show it.
    static func hover(_ title: String, entered: Bool) {
        guard enabled, entered else { return }
        FileHandle.standardError.write(Data("TABHOVER \(title)\n".utf8))
    }

    /// Computed layout vs the frames SwiftUI actually reported.
    ///
    /// The point of `TabStripLayout` is to replace four asynchronous
    /// measurements with one derivation — but only if the derived widths match
    /// what is drawn. A formula off by a few points would mis-aim every click,
    /// which is worse than the race it replaces. So both run and this prints
    /// the disagreement until the numbers say the swap is safe.
    ///
    /// Compared RELATIVE to the first chip, deliberately. The computed x is in
    /// the row's own space and the measured x is in the strip's, so absolute
    /// values differ by the row origin — the very quantity this work exists to
    /// stop depending on. Offsets from the first chip cancel it and isolate the
    /// only thing in question: whether widths and gaps accumulate correctly.
    static func compare(computed: [(slot: Int, x: CGFloat, w: CGFloat)],
                        measured: [(slot: Int, x: CGFloat, w: CGFloat)]) {
        guard enabled, let c0 = computed.first, let m0 = measured.first else { return }
        var worstDrift: CGFloat = 0
        var worstW: CGFloat = 0
        var rows: [String] = []
        for c in computed {
            guard let m = measured.first(where: { $0.slot == c.slot }) else {
                rows.append("\(c.slot):MEASURED-MISSING")
                continue
            }
            // Accumulated position error: a 1pt width error compounds along the
            // run, so the LAST chip is where a bad formula shows up worst.
            let drift = (c.x - c0.x) - (m.x - m0.x)
            let dw = c.w - m.w
            worstDrift = max(worstDrift, abs(drift))
            worstW = max(worstW, abs(dw))
            if abs(drift) > 0.5 || abs(dw) > 0.5 {
                rows.append(String(format: "%d:drift=%+.1f dw=%+.1f", c.slot, drift, dw))
            }
        }
        let verdict = (worstDrift <= 0.5 && worstW <= 0.5) ? "AGREE" : "DISAGREE"
        var line = String(
            format: "TABGEOM %@ worst drift=%.1f dw=%.1f  n=%d\n",
            verdict, worstDrift, worstW, computed.count
        )
        if !rows.isEmpty {
            line += "  " + rows.joined(separator: "  ") + "\n"
        }
        FileHandle.standardError.write(Data(line.utf8))
    }

    static func dump(
        x: CGFloat,
        origin: CGPoint,
        frames: [UInt64: CGRect],
        tabs: [TabItem]
    ) {
        guard enabled else { return }
        let live = tabs.map { t -> String in
            let mark = t.isLayout ? "L" : (t.group != 0 ? "g" : "-")
            let f = frames[t.stableId]
            let box = f.map { "[\(Int($0.minX))…\(Int($0.maxX))]" } ?? "[NO FRAME]"
            let hit = f.map { $0.minX <= x && x <= $0.maxX } ?? false
            return "\(t.id)\(mark):\(t.title.prefix(14))\(box)\(hit ? "<<HIT" : "")"
        }.joined(separator: "  ")
        let orphans = frames.keys
            .filter { key in !tabs.contains { $0.stableId == key } }
            .sorted()
            .map(String.init)
            .joined(separator: ",")
        var line = "TABLOG x=\(Int(x)) rowOriginX=\(Int(origin.x))\n"
        line += "  live: \(live)\n"
        if !orphans.isEmpty {
            line += "  orphaned frames (no live tab): \(orphans)\n"
        }
        FileHandle.standardError.write(Data(line.utf8))
    }
}
