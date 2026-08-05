import AppKit
import Foundation
import QuartzCore
import SwiftUI

/// Frame-accurate, opt-in instrumentation for titlebar motion.
///
/// Computer-use screenshots deliberately wait for the UI to settle, which is
/// useful for interaction but cannot inspect a 200–300 ms transition.  This
/// recorder samples the views' model and Core Animation presentation geometry
/// on the window's own display link instead.  It is completely dormant unless
/// `SUISEI_ANIMATION_TRACE=1` is present in the process environment.
///
/// Optional knobs used by the animation lab:
///
/// - `SUISEI_ANIMATION_SLOWMO=8` multiplies structural animation durations.
/// - `SUISEI_ANIMATION_CAPTURE_PNG=1` writes the titlebar band for every sample.
/// - `SUISEI_ANIMATION_TRACE_DIR=/path` overrides the default `/tmp` output.
final class AnimationTraceRecorder: NSObject {
    static let shared = AnimationTraceRecorder()

    static let enabled = ProcessInfo.processInfo.environment["SUISEI_ANIMATION_TRACE"] == "1"
    static let capturePNG = ProcessInfo.processInfo.environment["SUISEI_ANIMATION_CAPTURE_PNG"] == "1"
    static let slowMotionMultiplier: Double = {
        guard let raw = ProcessInfo.processInfo.environment["SUISEI_ANIMATION_SLOWMO"],
              let value = Double(raw), value.isFinite
        else { return 1 }
        return min(20, max(1, value))
    }()

    private final class WeakProbe {
        weak var view: NSView?
        init(_ view: NSView) { self.view = view }
    }

    private struct ElementSample: Codable {
        var key: String
        var model: [Double]
        var presentation: [Double]
        var opacity: Double
        var scaleX: Double
        var scaleY: Double
        var hidden: Bool
    }

    private struct FrameSample: Codable {
        var frame: Int
        var milliseconds: Double
        var displayTimestamp: Double
        var elements: [ElementSample]
    }

    private struct TraceDocument: Codable {
        var kind: String
        var expectedDurationMs: Double
        var slowMotionMultiplier: Double
        var capturedPNG: Bool
        var frames: [FrameSample]
    }

    private final class Session {
        let token = UUID()
        let kind: String
        let started = DispatchTime.now().uptimeNanoseconds
        let expectedDuration: Double
        let directory: URL
        var frames: [FrameSample] = []
        var nextFrame = 0

        init(kind: String, expectedDuration: Double, directory: URL) {
            self.kind = kind
            self.expectedDuration = expectedDuration
            self.directory = directory
        }
    }

    private var probes: [String: WeakProbe] = [:]
    private var displayLink: CADisplayLink?
    private var session: Session?
    private var sessionSerial = 0
    private var endWork: DispatchWorkItem?

    private override init() {
        super.init()
    }

    func register(_ view: NSView, key: String) {
        guard Self.enabled else { return }
        probes[key] = WeakProbe(view)
        ensureDisplayLink(for: view)
    }

    func unregister(_ view: NSView, key: String) {
        guard probes[key]?.view === view else { return }
        probes.removeValue(forKey: key)
    }

    /// Start sampling before the model mutation that drives a transition.
    func begin(kind: String, expectedDuration: Double) {
        guard Self.enabled else { return }
        if session != nil { finish() }

        sessionSerial += 1
        let safeKind = kind.replacingOccurrences(
            of: "[^A-Za-z0-9._-]",
            with: "-",
            options: .regularExpression
        )
        let root: URL
        if let override = ProcessInfo.processInfo.environment["SUISEI_ANIMATION_TRACE_DIR"],
           !override.isEmpty {
            root = URL(fileURLWithPath: override, isDirectory: true)
        } else {
            root = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
                .appendingPathComponent("SuiseiAnimationTraces", isDirectory: true)
        }
        let stamp = String(format: "%06d", sessionSerial)
        let directory = root.appendingPathComponent("\(stamp)-\(safeKind)", isDirectory: true)
        do {
            try FileManager.default.createDirectory(
                at: directory,
                withIntermediateDirectories: true
            )
        } catch {
            FileHandle.standardError.write(Data("animation trace mkdir failed: \(error)\n".utf8))
            return
        }

        let active = Session(
            kind: kind,
            expectedDuration: expectedDuration,
            directory: directory
        )
        session = active
        // Display-link delivery starts on the next refresh. Capture the
        // currently rendered (pre-refresh) state synchronously so a structural
        // replacement cannot make frame zero look as if the transition began
        // at its destination.
        captureSample(displayTimestamp: CACurrentMediaTime())

        endWork?.cancel()
        let work = DispatchWorkItem { [weak self, weak active] in
            guard let self, let active, self.session === active else { return }
            self.finish()
        }
        endWork = work
        DispatchQueue.main.asyncAfter(
            deadline: .now() + expectedDuration + 0.20,
            execute: work
        )
    }

    private func ensureDisplayLink(for view: NSView) {
        guard displayLink == nil, let window = view.window else { return }
        let link = window.displayLink(target: self, selector: #selector(displayLinkFired(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(
            minimum: 60,
            maximum: 120,
            preferred: 120
        )
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    @objc private func displayLinkFired(_ link: CADisplayLink) {
        captureSample(displayTimestamp: link.timestamp)
    }

    private func captureSample(displayTimestamp: CFTimeInterval) {
        guard let session else { return }
        probes = probes.filter { $0.value.view != nil }

        let now = DispatchTime.now().uptimeNanoseconds
        let elapsed = Double(now - session.started) / 1_000_000
        var elements: [ElementSample] = []
        elements.reserveCapacity(probes.count)

        for (key, weakProbe) in probes.sorted(by: { $0.key < $1.key }) {
            guard let view = weakProbe.view,
                  let root = view.window?.contentView
            else { continue }
            let model = view.convert(view.bounds, to: root)
            let layer = view.layer
            let presentationLayer = layer?.presentation()
            let rootLayer = root.layer?.presentation() ?? root.layer
            let presentation: CGRect
            if let presentationLayer, let rootLayer {
                presentation = presentationLayer.convert(presentationLayer.bounds, to: rootLayer)
            } else {
                presentation = model
            }
            let transform = presentationLayer?.transform ?? layer?.transform ?? CATransform3DIdentity
            elements.append(ElementSample(
                key: key,
                model: rectArray(model),
                presentation: rectArray(presentation),
                opacity: Double(presentationLayer?.opacity ?? layer?.opacity ?? 1),
                scaleX: hypot(Double(transform.m11), Double(transform.m12)),
                scaleY: hypot(Double(transform.m21), Double(transform.m22)),
                hidden: view.isHidden
            ))
        }

        let frame = session.nextFrame
        session.nextFrame += 1
        session.frames.append(FrameSample(
            frame: frame,
            milliseconds: elapsed,
            displayTimestamp: displayTimestamp,
            elements: elements
        ))

        if Self.capturePNG {
            captureTitlebar(frame: frame, milliseconds: elapsed, into: session.directory)
        }
    }

    private func captureTitlebar(frame: Int, milliseconds: Double, into directory: URL) {
        guard let strip = probes["tab-strip"]?.view,
              let root = strip.window?.contentView
        else { return }
        let stripRect = strip.convert(strip.bounds, to: root)
        let wanted = CGRect(
            x: root.bounds.minX,
            y: max(root.bounds.minY, stripRect.minY - 14),
            width: root.bounds.width,
            height: min(root.bounds.height, stripRect.height + 28)
        ).intersection(root.bounds)
        guard wanted.width > 0, wanted.height > 0,
              let bitmap = root.bitmapImageRepForCachingDisplay(in: wanted)
        else { return }
        root.cacheDisplay(in: wanted, to: bitmap)
        guard let png = bitmap.representation(using: .png, properties: [:]) else { return }
        let name = String(format: "frame-%04d-%07.2fms.png", frame, milliseconds)
        try? png.write(to: directory.appendingPathComponent(name), options: .atomic)
    }

    private func finish() {
        guard let session else { return }
        endWork?.cancel()
        endWork = nil
        self.session = nil

        let document = TraceDocument(
            kind: session.kind,
            expectedDurationMs: session.expectedDuration * 1000,
            slowMotionMultiplier: Self.slowMotionMultiplier,
            capturedPNG: Self.capturePNG,
            frames: session.frames
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        do {
            let data = try encoder.encode(document)
            try data.write(
                to: session.directory.appendingPathComponent("trace.json"),
                options: .atomic
            )
            let line = "animation trace \(session.kind): \(session.frames.count) frames → \(session.directory.path)\n"
            FileHandle.standardError.write(Data(line.utf8))
        } catch {
            FileHandle.standardError.write(Data("animation trace write failed: \(error)\n".utf8))
        }
    }

    private func rectArray(_ rect: CGRect) -> [Double] {
        [Double(rect.minX), Double(rect.minY), Double(rect.width), Double(rect.height)]
    }
}

/// Zero-cost when tracing is disabled; otherwise registers the exact SwiftUI
/// layout box as an AppKit view whose presentation layer can be sampled.
///
/// "Zero-cost" is now literal. This used to be the representable itself, so
/// every mount built a layer-backed `NSView` and re-ran `setKey` on every
/// update even with `SUISEI_ANIMATION_TRACE` unset — and one of the five mount
/// sites is *per tab chip*, inside the strip that already re-evaluates more
/// than it should. The recorder's own guards made it inert, not absent.
struct AnimationTraceProbe: View {
    let key: String

    var body: some View {
        if AnimationTraceRecorder.enabled {
            AnimationTraceProbeRepresentable(key: key)
        }
    }
}

private struct AnimationTraceProbeRepresentable: NSViewRepresentable {
    let key: String

    func makeNSView(context: Context) -> AnimationTraceProbeView {
        AnimationTraceProbeView(key: key)
    }

    func updateNSView(_ view: AnimationTraceProbeView, context: Context) {
        view.setKey(key)
    }
}

final class AnimationTraceProbeView: NSView {
    private var traceKey: String

    init(key: String) {
        traceKey = key
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = NSColor.clear.cgColor
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func setKey(_ key: String) {
        guard key != traceKey else { return }
        AnimationTraceRecorder.shared.unregister(self, key: traceKey)
        traceKey = key
        AnimationTraceRecorder.shared.register(self, key: traceKey)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil {
            AnimationTraceRecorder.shared.unregister(self, key: traceKey)
        } else {
            AnimationTraceRecorder.shared.register(self, key: traceKey)
        }
    }

    deinit {
        AnimationTraceRecorder.shared.unregister(self, key: traceKey)
    }
}
