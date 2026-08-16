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
    @State private var view = ModelViewOptions()
    @State private var clips: [ModelClip] = []
    @State private var selectedClip: String = ""
    @State private var playing = false

    @ObservedObject private var controls = EngineBridge.shared.viewerControls

    var body: some View {
        ZStack {
            palette.bg
            if let scene {
                ModelStage(scene: scene, palette: palette, command: $command, options: view)
            } else if loading {
                ProgressView().controlSize(.small)
            } else {
                unreadable
            }

            if scene != nil {
                VStack {
                    HStack {
                        Spacer()
                        ModelViewMenu(options: $view, palette: palette)
                            .padding(.trailing, 12)
                            .padding(.top, 10)
                    }
                    Spacer()
                }
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
            for clip in clips { clip.stop() }
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
/// 6.00s).
///
/// **Players, plural.** A rigged animation is one clip with a channel per
/// bone, and each channel becomes a player on the node it drives. Keyed by
/// player, `animation_with_skeleton.fbx` came back as fifteen clips all called
/// "Armature|ArmatureAction" — a picker with fifteen identical rows, of which
/// choosing any one moved a single bone. A clip is a NAME, and playing it
/// plays everything under it.
struct ModelClip: Identifiable, Equatable {
    static func == (a: ModelClip, b: ModelClip) -> Bool { a.id == b.id }
    let id: String
    let name: String
    let duration: Double
    let players: [SCNAnimationPlayer]

    func play() { players.forEach { $0.paused = false; $0.play() } }
    func pause() { players.forEach { $0.paused = true } }
    func stop() { players.forEach { $0.stop() } }
    func setSpeed(_ rate: CGFloat) { players.forEach { $0.speed = rate } }
}

/// Counts worth reading off a mesh before opening it in a real tool.
struct ModelStats: Equatable {
    var meshes: Int
    var vertices: Int
    /// Triangles. The number an artist asks for first — a budget is quoted in
    /// polygons, never in vertices, and after triangulation these are the same
    /// question.
    var triangles: Int
    var materials: Int
    var animations: Int
    /// Nodes in the graph, and how deep it goes. A model that is one mesh in
    /// forty nested transforms behaves differently from one that is forty
    /// meshes side by side, and nothing else in the panel would say so.
    var nodes: Int
    var depth: Int
    var hasNormals: Bool
    var hasUVs: Bool
    var skinned: Bool
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
            triangles: counted.triangles,
            materials: counted.materials,
            animations: counted.animations,
            nodes: counted.nodes,
            depth: counted.depth,
            hasNormals: counted.hasNormals,
            hasUVs: counted.hasUVs,
            skinned: counted.skinned,
            extent: extent
        )
        stats = s

        let dims = NumberFormatter()
        dims.maximumFractionDigits = 2
        func size(_ v: Float) -> String { dims.string(from: NSNumber(value: v)) ?? "—" }
        // What a 3D file is asked about, in the order it is asked. The polygon
        // count leads because a budget is quoted in polygons; the attribute
        // line is next because "why is it flat" and "why is the texture
        // missing" are both answered by whether the mesh carries normals and
        // UVs at all.
        sections = [
            ViewerInfoSection("Geometry", [
                ("Polygons", s.triangles.formatted()),
                ("Vertices", s.vertices.formatted()),
                ("Meshes", "\(s.meshes)"),
                ("Materials", "\(s.materials)"),
                ("Attributes", [
                    s.hasNormals ? "normals" : nil,
                    s.hasUVs ? "UVs" : nil,
                    s.skinned ? "skinned" : nil,
                ].compactMap { $0 }.joined(separator: " · ")),
            ]),
            ViewerInfoSection("Scene", [
                ("Nodes", "\(s.nodes)"),
                ("Depth", "\(s.depth)"),
                ("Animations", s.animations > 0 ? "\(s.animations)" : "none"),
            ]),
            ViewerInfoSection("Bounds", [
                ("Width", size(extent.x)),
                ("Height", size(extent.y)),
                ("Depth", size(extent.z)),
                // Its own units, said out loud. A model is authored in metres
                // or centimetres and the file rarely says which; a number with
                // no unit beside it invites the wrong one to be assumed.
                ("Units", "file units"),
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
        case "fbx":
            return try FBXScene.read(url)
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

    /// Every animation in the scene, grouped by name.
    ///
    /// By NAME, not by player: a rigged clip has a channel per bone and each
    /// becomes a player on its own node. Measured on
    /// `animation_with_skeleton.fbx` — fifteen players, one name — so keying
    /// by player produced fifteen identical picker rows, each moving one bone.
    ///
    /// The duration is the longest channel's. They should agree, and when they
    /// do not the clip lasts as long as its slowest part.
    static func clips(in root: SCNNode) -> [ModelClip] {
        var byName: [String: [SCNAnimationPlayer]] = [:]
        var order: [String] = []
        func walk(_ node: SCNNode) {
            for key in node.animationKeys {
                guard let player = node.animationPlayer(forKey: key) else { continue }
                if byName[key] == nil { order.append(key) }
                byName[key, default: []].append(player)
            }
            node.childNodes.forEach(walk)
        }
        walk(root)
        return order.map { name in
            let players = byName[name] ?? []
            return ModelClip(
                id: name,
                name: name,
                duration: players.map(\.animation.duration).max() ?? 0,
                players: players
            )
        }
    }

    /// Walk the scene once for everything worth counting.
    ///
    /// One walk rather than four: a scene graph can be deep, and asking it the
    /// same question four times is four traversals for one answer.
    private static func count(
        _ root: SCNNode
    ) -> (
        meshes: Int, vertices: Int, triangles: Int, materials: Int, animations: Int,
        nodes: Int, depth: Int, hasNormals: Bool, hasUVs: Bool, skinned: Bool
    ) {
        var meshes = 0
        var vertices = 0
        var triangles = 0
        var animations = 0
        var nodes = 0
        var depth = 0
        var hasNormals = false
        var hasUVs = false
        var skinned = false
        // Materials are shared between nodes far more often than not, so they
        // are counted by identity — a cube with one material on six faces has
        // one material, not six.
        var materials: Set<ObjectIdentifier> = []

        func walk(_ node: SCNNode, _ level: Int) {
            nodes += 1
            depth = max(depth, level)
            if let geometry = node.geometry {
                meshes += 1
                for source in geometry.sources {
                    switch source.semantic {
                    case .vertex: vertices += source.vectorCount
                    case .normal: hasNormals = true
                    case .texcoord: hasUVs = true
                    default: break
                    }
                }
                // Every element, because a mesh can carry more than one and a
                // count that stopped at the first would under-report exactly
                // the models with the most in them.
                for element in geometry.elements where element.primitiveType == .triangles {
                    triangles += element.primitiveCount
                }
                for material in geometry.materials {
                    materials.insert(ObjectIdentifier(material))
                }
            }
            if node.skinner != nil { skinned = true }
            animations += node.animationKeys.count
            for child in node.childNodes { walk(child, level + 1) }
        }
        walk(root, 0)
        return (
            meshes, vertices, triangles, materials.count, animations,
            nodes, depth, hasNormals, hasUVs, skinned
        )
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
    let options: ModelViewOptions

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
        view.debugOptions = options.debugOptions
        view.showsStatistics = options.statistics

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
                if playing { clip.play() } else { clip.pause() }
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
                clip.stop()
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
                    clips.first { $0.id == old }?.stop()
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
                    clip?.setSpeed(CGFloat(rate))
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

/// What the stage is showing besides the model.
///
/// The names are SceneKit's own — read off `SCNSceneRenderer.h` rather than
/// from a tutorial, because that header is the authoritative list and it has
/// eleven entries, most of which mean nothing for a static asset (physics
/// fields, light extents, slider constraints). These four are the ones a
/// person looking at a mesh asks for, and they are the ones Xcode's own scene
/// editor puts on its toolbar.
struct ModelViewOptions: Equatable {
    /// `SCNDebugOptionShowWireframe` — the mesh drawn OVER the shaded surface,
    /// which is what shows topology. `RenderAsWireframe` replaces the surface
    /// instead, and then a dense mesh is a grey blob.
    var wireframe = false
    /// `SCNDebugOptionShowBoundingBoxes`.
    var boundingBoxes = false
    /// `SCNDebugOptionShowSkeletons` — nothing at all on an unskinned model,
    /// which is why the menu says what it is rather than hiding it.
    var skeletons = false
    /// SceneKit's own frame/draw-call meter.
    var statistics = false

    var debugOptions: SCNDebugOptions {
        var out: SCNDebugOptions = []
        if wireframe { out.insert(.showWireframe) }
        if boundingBoxes { out.insert(.showBoundingBoxes) }
        if skeletons { out.insert(.showSkeletons) }
        return out
    }
}

/// The view toggles, as a menu in the stage's corner.
///
/// Not in the window toolbar with zoom and reset: those act on the model and
/// this changes how it is drawn, and the toolbar is shared with every other
/// viewer. A corner menu also keeps them out of the way of the thing they are
/// about, which is the whole stage.
struct ModelViewMenu: View {
    @Binding var options: ModelViewOptions
    let palette: ViewerPalette

    var body: some View {
        Menu {
            Toggle("Wireframe", isOn: $options.wireframe)
            Toggle("Bounding Boxes", isOn: $options.boundingBoxes)
            Toggle("Skeleton", isOn: $options.skeletons)
            Divider()
            Toggle("Statistics", isOn: $options.statistics)
        } label: {
            Image(systemName: "square.3.layers.3d")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(palette.fg)
                .frame(width: 26, height: 22)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .padding(.horizontal, 4)
        .padding(.vertical, 3)
        .glassEffect(.regular, in: Capsule())
        .help("View options")
    }
}
