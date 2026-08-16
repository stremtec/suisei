//  ModelViewer.swift
//  A 3D asset in a pane.
//
//  feature.txt #8 — "게임 개발시에 쓰이는 여러가지 3d 모델들을 렌더링해서 볼 수
//  있게 해주는 기능". It is a viewer, not a modeller: the question a game
//  developer asks of a file tree is "is this the right mesh, and how big is
//  it", and that is answered by orbiting it and reading its counts.
//
//  Nothing here parses a model. SceneKit reads `.scn` and `.dae`, and defers
//  to Model I/O for USD, `.obj`, `.ply` and `.stl`; glTF and GLB come from the
//  vendored GLTFKit2, because macOS reads neither — measured, not assumed, and
//  written down in `third_party/GLTFKit2/VENDOR.md`. Either way the result is
//  an ordinary `SCNScene`, so the reader is the only thing that differs and
//  everything below this line is one path.
//
//  A format nothing can read is left as `Binary`, where the placeholder at
//  least names the file, rather than routed here to an empty stage. FBX is
//  that case: it needs Autodesk's own SDK, which is commercial and carries
//  redistribution terms.

import AppKit
import GLTFKit2
import SceneKit
import SwiftUI

/// The pane's contents: a scene, orbited.
struct ModelPaneViewer: View {
    let path: String
    let palette: ViewerPalette

    @State private var scene: SCNScene?
    @State private var failure: String?
    @State private var stats: ModelStats?
    @State private var command: ModelCommand?
    @State private var loading = true
    @State private var clips: [ModelClip] = []
    @State private var selectedClip: String = ""
    @State private var playing = false

    @ObservedObject private var controls = EngineBridge.shared.viewerControls

    var body: some View {
        ZStack {
            palette.bg
            if let scene {
                ModelStage(scene: scene, palette: palette, command: $command)
            } else if loading {
                ProgressView().controlSize(.small)
            } else {
                unreadable
            }

            if !clips.isEmpty, scene != nil {
                VStack {
                    Spacer()
                    ModelAnimationBar(
                        clips: clips,
                        selected: $selectedClip,
                        playing: $playing,
                        palette: palette
                    )
                    .padding(.bottom, 14)
                }
            }
        }
        .task(id: path) { await load() }
        .onAppear { claimToolbar() }
        .onDisappear { controls.release(.model) }
    }

    /// Why it will not open, in the file's own terms.
    ///
    /// A blank stage would be indistinguishable from a model that loaded and
    /// happens to be off camera, and those two need different reactions.
    private var unreadable: some View {
        VStack(spacing: 6) {
            Image(systemName: "cube.transparent")
                .font(.system(size: 26))
                .foregroundStyle(palette.dim)
            Text("Cannot read this model")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(palette.fg)
            if let failure {
                Text(failure)
                    .font(.system(size: 11))
                    .foregroundStyle(palette.dim)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 320)
            }
        }
    }

    private func claimToolbar() {
        controls.claim(.model, canZoom: true)
        controls.zoomLabel = ""
        controls.resetSymbol = "arrow.up.left.and.arrow.down.right"
        controls.resetHelp = "Frame the model"
        controls.perform = { cmd in
            switch cmd {
            case .zoomOut: command = .dolly(out: true)
            case .zoomIn: command = .dolly(out: false)
            case .reset: command = .frame
            }
        }
    }

    private func load() async {
        loading = true
        scene = nil
        failure = nil
        stats = nil
        clips = []
        selectedClip = ""
        playing = false
        controls.setSections([])

        let url = URL(fileURLWithPath: path)
        // Off the main thread: reading a mesh is arbitrary work on a file of
        // arbitrary size, and it has no business happening on the thread that
        // is drawing the pane. The same reason `PDFPaneViewer` detaches its
        // parse.
        let loaded = await Task.detached(priority: .userInitiated) {
            ModelLoad(url: url)
        }.value

        loading = false
        switch loaded.result {
        case .success(let s):
            scene = s
            stats = loaded.stats
            controls.setSections(loaded.sections)
            command = .frame
            clips = ModelLoad.clips(in: s.rootNode)
            // Paused on arrival. An asset that starts moving the instant it
            // opens is answering a question nobody asked — the first one is
            // "is this the right mesh", and a spinning one is harder to judge.
            for clip in clips { clip.player.stop() }
            selectedClip = clips.first?.id ?? ""
        case .failure(let message):
            failure = message
            controls.setSections([ViewerInfoSection.file(url)])
        }
    }
}

/// What the toolbar asked for. A value rather than a method call, because the
/// stage is an `NSViewRepresentable` and SwiftUI rebuilds its struct freely —
/// the same reason `PDFPaneViewer` sends its zoom this way.
enum ModelCommand: Equatable {
    case frame
    case dolly(out: Bool)
}

/// One animation the file carries.
///
/// Name and duration come straight from SceneKit, measured on a real animated
/// asset that ships with macOS (`B389_loop.dae`: one clip, "square_GEP-anim",
/// 6.00s). The player is the thing that plays it — held rather than looked up
/// again, because `animationPlayer(forKey:)` walks the node it belongs to.
struct ModelClip: Identifiable, Equatable {
    static func == (a: ModelClip, b: ModelClip) -> Bool { a.id == b.id }
    let id: String
    let name: String
    let duration: Double
    let player: SCNAnimationPlayer
}

/// Counts worth reading off a mesh before opening it in a real tool.
struct ModelStats: Equatable {
    var meshes: Int
    var vertices: Int
    var materials: Int
    var animations: Int
    /// The model's bounding box in its own units.
    var extent: SIMD3<Float>
}

/// One load: the scene, what it is made of, and what to show if it failed.
private struct ModelLoad {
    enum Result {
        case success(SCNScene)
        case failure(String)
    }

    let result: Result
    var stats: ModelStats?
    var sections: [ViewerInfoSection] = []

    init(url: URL) {
        guard FileManager.default.fileExists(atPath: url.path) else {
            result = .failure("The file is no longer there.")
            return
        }
        let scene: SCNScene
        do {
            scene = try ModelLoad.read(url)
        } catch {
            result = .failure(error.localizedDescription)
            return
        }

        let counted = ModelLoad.count(scene.rootNode)
        // A scene with no geometry is not a success, whatever SceneKit
        // returned. Measured: a malformed STL does not throw — it comes back
        // as an empty scene, and an empty stage is indistinguishable from a
        // model that loaded and is off camera. Those two need different
        // reactions from the user, so they get different screens.
        guard counted.meshes > 0 else {
            result = .failure(
                "The file opened but contains no geometry. It may be an "
                    + "unsupported variant of \(url.pathExtension.uppercased())."
            )
            return
        }
        result = .success(scene)
        let box = scene.rootNode.boundingBox
        let extent = SIMD3<Float>(
            Float(box.max.x - box.min.x),
            Float(box.max.y - box.min.y),
            Float(box.max.z - box.min.z)
        )
        let s = ModelStats(
            meshes: counted.meshes,
            vertices: counted.vertices,
            materials: counted.materials,
            animations: counted.animations,
            extent: extent
        )
        stats = s

        let dims = NumberFormatter()
        dims.maximumFractionDigits = 2
        func size(_ v: Float) -> String { dims.string(from: NSNumber(value: v)) ?? "—" }
        sections = [
            ViewerInfoSection("Model", [
                ("Meshes", "\(s.meshes)"),
                ("Vertices", s.vertices.formatted()),
                ("Materials", "\(s.materials)"),
                ("Animations", s.animations > 0 ? "\(s.animations)" : nil),
            ]),
            ViewerInfoSection("Bounds", [
                ("Width", size(extent.x)),
                ("Height", size(extent.y)),
                ("Depth", size(extent.z)),
            ]),
            ViewerInfoSection.file(url),
        ]
    }

    /// Whichever reader knows this extension.
    ///
    /// Two readers, one result. GLTFKit2 hands back an ordinary `SCNScene`, so
    /// this is the only place that has to know there are two — and it is here
    /// rather than in the view because "what can open this file" is a fact
    /// about the file, not about the pane.
    private static func read(_ url: URL) throws -> SCNScene {
        switch url.pathExtension.lowercased() {
        case "gltf", "glb":
            let asset = try GLTFAsset(url: url)
            let source = GLTFSCNSceneSource(asset: asset)
            guard let scene = source.defaultScene else {
                throw NSError(
                    domain: "Suisei.Model", code: 1,
                    userInfo: [
                        NSLocalizedDescriptionKey:
                            "The glTF opened but names no default scene."
                    ]
                )
            }
            return scene
        default:
            // `checkConsistency` off deliberately: it rejects files that other
            // tools open, and the alternative to a slightly odd scene here is
            // no scene at all.
            return try SCNScene(url: url, options: [.checkConsistency: false])
        }
    }

    /// Every animation in the scene, with the player that runs it.
    ///
    /// A clip's key is unique within its node but not within the file, so the
    /// identity carries the node's name too — two meshes each animating
    /// "Take 001" is the ordinary case in an exported rig, and a picker whose
    /// entries collide picks the wrong one.
    static func clips(in root: SCNNode) -> [ModelClip] {
        var out: [ModelClip] = []
        func walk(_ node: SCNNode) {
            for key in node.animationKeys {
                guard let player = node.animationPlayer(forKey: key) else { continue }
                out.append(ModelClip(
                    id: "\(node.name ?? "node")/\(key)",
                    name: key,
                    duration: player.animation.duration,
                    player: player
                ))
            }
            node.childNodes.forEach(walk)
        }
        walk(root)
        return out
    }

    /// Walk the scene once for everything worth counting.
    ///
    /// One walk rather than four: a scene graph can be deep, and asking it the
    /// same question four times is four traversals for one answer.
    private static func count(
        _ root: SCNNode
    ) -> (meshes: Int, vertices: Int, materials: Int, animations: Int) {
        var meshes = 0
        var vertices = 0
        var animations = 0
        // Materials are shared between nodes far more often than not, so they
        // are counted by identity — a cube with one material on six faces has
        // one material, not six.
        var materials: Set<ObjectIdentifier> = []

        func walk(_ node: SCNNode) {
            if let geometry = node.geometry {
                meshes += 1
                for source in geometry.sources where source.semantic == .vertex {
                    vertices += source.vectorCount
                }
                for material in geometry.materials {
                    materials.insert(ObjectIdentifier(material))
                }
            }
            animations += node.animationKeys.count
            for child in node.childNodes { walk(child) }
        }
        walk(root)
        return (meshes, vertices, materials.count, animations)
    }
}

/// SceneKit's view, with a camera that starts pointed at the model.
///
/// `allowsCameraControl` brings orbit, pan and pinch-zoom with it, which is
/// the whole interaction this viewer needs and none of which is worth
/// rebuilding on top of a gesture recogniser — the same trade `ImagePaneViewer`
/// makes with `NSScrollView`'s magnification.
private struct ModelStage: NSViewRepresentable {
    let scene: SCNScene
    let palette: ViewerPalette
    @Binding var command: ModelCommand?

    func makeNSView(context: Context) -> SCNView {
        let view = SCNView()
        view.allowsCameraControl = true
        view.autoenablesDefaultLighting = true
        // An asset with no lights of its own is otherwise a silhouette, and a
        // silhouette is not a preview. Omni rather than ambient so the form
        // reads: flat light makes a sphere and a disc look the same.
        view.defaultCameraController.interactionMode = .orbitTurntable
        view.defaultCameraController.inertiaEnabled = true
        view.antialiasingMode = .multisampling4X
        view.rendersContinuously = false
        return view
    }

    func updateNSView(_ view: SCNView, context: Context) {
        if view.scene !== scene {
            view.scene = scene
            context.coordinator.framed = false
        }
        view.backgroundColor = NSColor(palette.bg)

        if !context.coordinator.framed {
            context.coordinator.framed = true
            frame(view)
        }
        guard let command else { return }
        switch command {
        case .frame:
            frame(view)
        case .dolly(let out):
            view.defaultCameraController.dolly(by: out ? -2 : 2, onScreenPoint: .zero, viewport: view.bounds.size)
        }
        // Cleared on the next turn, not here: this runs inside a SwiftUI
        // update, and writing state from one is what the runtime warns about.
        DispatchQueue.main.async { self.command = nil }
    }

    /// Point the camera at whatever was loaded.
    ///
    /// Models arrive at wildly different scales — a USDZ authored in metres
    /// and an OBJ authored in centimetres differ by a hundred — so a fixed
    /// camera distance shows either a speck or the inside of a wall.
    /// `defaultCameraController` knows the scene's bounds and can do this
    /// exactly.
    private func frame(_ view: SCNView) {
        view.defaultCameraController.frameNodes([view.scene?.rootNode].compactMap { $0 })
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        /// Whether this scene has been framed once. Re-framing on every update
        /// would fight the user's orbit.
        var framed = false
    }
}

/// The animation transport.
///
/// Deliberately the audio viewer's card: a rounded platter floating over the
/// content rather than a bar welded to the pane's edge, made of the toolbar's
/// own material through `glassEffect` rather than of a fill chosen to look
/// like it. `AudioViewer.transportCard` says why at length, and a second
/// answer to the same question would be a second thing to keep in step.
///
/// What differs is what a model has that a sound file does not: more than one
/// clip. So the play control gains a picker, and it appears only when there is
/// something to pick — one clip is not a choice.
struct ModelAnimationBar: View {
    let clips: [ModelClip]
    @Binding var selected: String
    @Binding var playing: Bool
    let palette: ViewerPalette

    private var clip: ModelClip? {
        clips.first { $0.id == selected } ?? clips.first
    }

    var body: some View {
        HStack(spacing: 14) {
            Button {
                guard let clip else { return }
                playing.toggle()
                if playing {
                    clip.player.play()
                } else {
                    clip.player.paused = true
                }
            } label: {
                Image(systemName: playing ? "pause.fill" : "play.fill")
                    .font(.system(size: 19, weight: .medium))
                    .foregroundStyle(palette.fg)
                    .frame(width: 26, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Button {
                guard let clip else { return }
                clip.player.stop()
                playing = false
            } label: {
                Image(systemName: "stop.fill")
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(palette.fg)
                    .frame(width: 20, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Rewind to the start")

            if clips.count > 1 {
                Picker("", selection: $selected) {
                    ForEach(clips) { c in
                        Text(c.name).tag(c.id)
                    }
                }
                .labelsHidden()
                .pickerStyle(.menu)
                .frame(maxWidth: 180)
                .onChange(of: selected) { old, _ in
                    // Switching clips stops the one that was running. Two
                    // animations on one rig at once is not a preview of either.
                    clips.first { $0.id == old }?.player.stop()
                    playing = false
                }
            } else if let clip {
                Text(clip.name)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(palette.fg)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 180)
            }

            // Duration, not a playhead. `SCNAnimationPlayer` exposes play,
            // pause, stop and speed, and no position to read or write — so a
            // scrubbing bar here would be a control that cannot do what its
            // shape promises. The number is what can be said truthfully.
            if let clip {
                Text(Self.clock(clip.duration))
                    .font(.system(size: 11, weight: .medium).monospacedDigit())
                    .foregroundStyle(palette.dim)
                    .fixedSize()
            }

            speedControl
        }
        // Wider than it looks like it needs: a capsule's end is a half-circle,
        // so at this height the first 28pt of each side is curve.
        .padding(.horizontal, 28)
        .padding(.vertical, 12)
        .glassEffect(.regular, in: Capsule())
    }

    /// Speed, which is the one thing about playback SceneKit does let you move.
    private var speedControl: some View {
        Menu {
            ForEach([0.25, 0.5, 1.0, 2.0], id: \.self) { rate in
                Button(rate == 1 ? "Normal" : "\(rate)×") {
                    clip?.player.speed = CGFloat(rate)
                }
            }
        } label: {
            Image(systemName: "gauge.with.dots.needle.33percent")
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(palette.dim)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .frame(width: 20)
        .help("Playback speed")
    }

    static func clock(_ s: Double) -> String {
        guard s.isFinite, s > 0 else { return "—" }
        let total = Int(s.rounded())
        return String(format: "%d:%02d", total / 60, total % 60)
    }
}
