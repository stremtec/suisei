//  ModelViewer.swift
//  A 3D asset in a pane.
//
//  feature.txt #8 — "게임 개발시에 쓰이는 여러가지 3d 모델들을 렌더링해서 볼 수
//  있게 해주는 기능". It opens as a viewer: the first question a game developer
//  asks of a file tree is "is this the right mesh, and how big is it", and that
//  is answered by orbiting it and reading its counts. `ModelWorkbench` is the
//  second question — what the scene is MADE of, and changing it.
//
//  Nothing here parses a model. SceneKit reads `.scn` and `.dae`, and defers
//  to Model I/O for USD, `.obj`, `.ply` and `.stl`; glTF and GLB come from the
//  vendored GLTFKit2, because macOS reads neither; FBX comes from the vendored
//  Assimp. All four readers hand back an ordinary `SCNScene`, so the reader is
//  the only thing that differs and everything below this line is one path.

import AppKit
import GLTFKit2
import SceneKit
import SwiftUI

/// The pane's contents: a scene, orbited, with a workbench beside it.
struct ModelPaneViewer: View {
    let path: String
    let palette: ViewerPalette

    @StateObject private var doc = ModelDocument()
    @State private var failure: String?
    @State private var loading = true
    @State private var workbench = false
    @State private var workbenchWidth: CGFloat = 300

    @ObservedObject private var controls = EngineBridge.shared.viewerControls

    var body: some View {
        ZStack {
            palette.bg
            HStack(spacing: 0) {
                stageColumn
                if workbench, doc.scene != nil {
                    ModelWorkbenchDivider(width: $workbenchWidth, palette: palette)
                    ModelWorkbench(doc: doc, palette: palette)
                        .frame(width: workbenchWidth)
                        .transition(.move(edge: .trailing))
                }
            }
        }
        .task(id: path) { await load() }
        .onAppear { claimToolbar() }
        .onDisappear { controls.release(.model) }
        // The Info panel is rebuilt on edits too, because "Polygons" is a fact
        // about the scene as it stands and the workbench can change the scene.
        .onChange(of: doc.revision) { _, _ in controls.setSections(doc.infoSections()) }
    }

    private var stageColumn: some View {
        // Ink for anything floating over the model, chosen from the stage
        // rather than from the theme — see `ViewerPalette.overStage`.
        let over = palette.overStage(doc.backgroundOverride)
        return ZStack {
            if let scene = doc.scene {
                ModelStage(doc: doc, scene: scene, palette: palette)
            } else if loading {
                ProgressView().controlSize(.small)
            } else {
                unreadable
            }

            if doc.scene != nil {
                VStack {
                    HStack(spacing: 8) {
                        Spacer()
                        ModelViewMenu(options: $doc.view, palette: over)
                        workbenchToggle(over)
                    }
                    .padding(.trailing, 12)
                    .padding(.top, 10)
                    Spacer()
                }

                if !doc.clips.isEmpty {
                    VStack {
                        Spacer()
                        ModelAnimationBar(
                            clips: doc.clips,
                            selected: $doc.selectedClip,
                            playing: $doc.playing,
                            palette: over
                        )
                        .padding(.bottom, 14)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func workbenchToggle(_ over: ViewerPalette) -> some View {
        Button {
            withAnimation(.easeOut(duration: 0.16)) { workbench.toggle() }
        } label: {
            Image(systemName: "sidebar.trailing")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(workbench ? palette.accent : over.fg)
                .frame(width: 26, height: 22)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 4)
        .padding(.vertical, 3)
        .glassEffect(.regular, in: Capsule())
        .help("Scene workbench")
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
        controls.perform = { [doc] cmd in
            switch cmd {
            case .zoomOut: doc.command = .dolly(out: true)
            case .zoomIn: doc.command = .dolly(out: false)
            case .reset: doc.command = .frame
            }
        }
    }

    private func load() async {
        loading = true
        failure = nil
        doc.clear()
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
        case .success(let scene):
            doc.adopt(scene, url: url, load: loaded)
            controls.setSections(doc.infoSections())
            doc.command = .frame
        case .failure(let message):
            failure = message
            controls.setSections([ViewerInfoSection.file(url)])
        }
    }
}

/// The drag handle between the stage and the workbench.
private struct ModelWorkbenchDivider: View {
    @Binding var width: CGFloat
    let palette: ViewerPalette
    @State private var start: CGFloat?

    var body: some View {
        Rectangle()
            .fill(palette.dim.opacity(0.18))
            .frame(width: 1)
            .overlay {
                Rectangle()
                    .fill(.clear)
                    .frame(width: 9)
                    .contentShape(Rectangle())
                    .onHover { NSCursor.resizeLeftRight.set(); if !$0 { NSCursor.arrow.set() } }
                    .gesture(
                        DragGesture(minimumDistance: 1)
                            .onChanged { g in
                                let base = start ?? width
                                if start == nil { start = width }
                                width = min(560, max(232, base - g.translation.width))
                            }
                            .onEnded { _ in start = nil }
                    )
            }
    }
}

/// What the toolbar asked for. A value rather than a method call, because the
/// stage is an `NSViewRepresentable` and SwiftUI rebuilds its struct freely —
/// the same reason `PDFPaneViewer` sends its zoom this way.
enum ModelCommand: Equatable {
    case frame
    case dolly(out: Bool)
    /// Frame the current selection rather than the whole scene.
    case frameSelection
}

// MARK: - The stage

/// SceneKit's view, with a camera that starts pointed at the model.
///
/// `allowsCameraControl` brings orbit, pan and pinch-zoom with it, which is
/// the whole interaction this viewer needs and none of which is worth
/// rebuilding on top of a gesture recogniser — the same trade `ImagePaneViewer`
/// makes with `NSScrollView`'s magnification.
struct ModelStage: NSViewRepresentable {
    @ObservedObject var doc: ModelDocument
    let scene: SCNScene
    let palette: ViewerPalette

    func makeNSView(context: Context) -> ModelStageView {
        let view = ModelStageView()
        view.allowsCameraControl = true
        view.autoenablesDefaultLighting = true
        view.defaultCameraController.interactionMode = .orbitTurntable
        view.defaultCameraController.inertiaEnabled = true
        view.antialiasingMode = .multisampling4X
        view.rendersContinuously = false
        view.onPick = { [weak doc] node in doc?.select(node) }
        return view
    }

    func updateNSView(_ view: ModelStageView, context: Context) {
        if view.scene !== scene {
            view.scene = scene
            context.coordinator.framed = false
        }
        view.backgroundColor = NSColor(doc.backgroundOverride ?? palette.stage)
        view.debugOptions = doc.view.debugOptions
        view.showsStatistics = doc.view.statistics
        view.autoenablesDefaultLighting = doc.view.defaultLighting
        // Looking through the file's own camera is looking through the file's
        // own camera: orbiting would move it, which is an edit the user did not
        // ask for. The workbench offers "Use as viewer position" for the case
        // where they did.
        let ownCamera = doc.pointOfView === doc.viewerCamera
        view.allowsCameraControl = ownCamera
        if view.pointOfView !== doc.pointOfView { view.pointOfView = doc.pointOfView }

        if !context.coordinator.framed {
            context.coordinator.framed = true
            frame(view)
        }
        guard let command = doc.command else { return }
        switch command {
        case .frame:
            frame(view)
        case .frameSelection:
            if let node = doc.selectedNode {
                view.defaultCameraController.frameNodes([node])
            } else {
                frame(view)
            }
        case .dolly(let out):
            view.defaultCameraController.dolly(
                by: out ? -2 : 2, onScreenPoint: .zero, viewport: view.bounds.size)
        }
        // Cleared on the next turn, not here: this runs inside a SwiftUI
        // update, and writing state from one is what the runtime warns about.
        DispatchQueue.main.async { [doc] in doc.command = nil }
    }

    /// Point the camera at whatever was loaded.
    ///
    /// Models arrive at wildly different scales — a USDZ authored in metres
    /// and an OBJ authored in centimetres differ by a hundred — so a fixed
    /// camera distance shows either a speck or the inside of a wall.
    /// `defaultCameraController` knows the scene's bounds and can do this
    /// exactly, PROVIDED it is given the meshes rather than the whole graph.
    private func frame(_ view: ModelStageView) {
        // Three-quarters, before the distance is computed. `frameNodes` moves
        // the camera and keeps its orientation, and the identity orientation
        // looks straight down −Z — which shows a floor-plane model exactly
        // edge-on, i.e. as nothing. Measured on `B389_loop.dae`, whose mesh is
        // a square lying in XZ.
        if doc.pointOfView === doc.viewerCamera {
            doc.viewerCamera.eulerAngles = SCNVector3(-CGFloat.pi / 7, CGFloat.pi / 5, 0)
        }
        view.defaultCameraController.frameNodes(ModelLoad.geometryNodes(scene.rootNode))
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        /// Whether this scene has been framed once. Re-framing on every update
        /// would fight the user's orbit.
        var framed = false
    }
}

/// The stage's view, which also answers "what did I just click on".
///
/// A workbench needs picking, and `allowsCameraControl` owns the mouse. The
/// two coexist by asking a different question of the same gesture: a press and
/// release that did not travel is a click, and anything else is an orbit. The
/// event is passed on either way, so the camera never notices.
final class ModelStageView: SCNView {
    var onPick: ((SCNNode?) -> Void)?
    private var pressedAt: NSPoint?

    override func mouseDown(with event: NSEvent) {
        pressedAt = convert(event.locationInWindow, from: nil)
        super.mouseDown(with: event)
    }

    override func mouseUp(with event: NSEvent) {
        super.mouseUp(with: event)
        guard let start = pressedAt else { return }
        pressedAt = nil
        let end = convert(event.locationInWindow, from: nil)
        guard abs(end.x - start.x) < 3, abs(end.y - start.y) < 3 else { return }
        let hits = hitTest(end, options: [
            .searchMode: SCNHitTestSearchMode.closest.rawValue,
            .ignoreHiddenNodes: true,
            // The selection box drawn on the current pick must not be pickable
            // itself, or the second click on a selected object hits the box.
            .categoryBitMask: ModelDocument.pickableCategory,
        ])
        onPick?(hits.first?.node)
    }
}

// MARK: - The animation transport

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

// MARK: - View options

/// What the stage is showing besides the model.
///
/// The names are SceneKit's own, read off `SCNSceneRenderer.h` rather than
/// from a tutorial, because that header is the authoritative list. All eleven
/// are here. Most mean nothing for a static mesh — and that is exactly why
/// they are grouped: the four an artist reaches for first are at the top, and
/// the ones that only light up when the scene has physics or lights or
/// constraints are behind a submenu that says so.
struct ModelViewOptions: Equatable {
    /// `SCNDebugOptionShowWireframe` — the mesh drawn OVER the shaded surface,
    /// which is what shows topology. `RenderAsWireframe` replaces the surface
    /// instead, and both are offered because they answer different questions.
    var wireframe = false
    var renderAsWireframe = false
    /// `SCNDebugOptionShowBoundingBoxes`.
    var boundingBoxes = false
    /// `SCNDebugOptionShowSkeletons` — nothing at all on an unskinned model,
    /// which is why the menu says what it is rather than hiding it.
    var skeletons = false
    var creases = false
    var cameras = false
    var lightInfluences = false
    var lightExtents = false
    var physicsShapes = false
    var physicsFields = false
    var constraints = false
    /// SceneKit's own frame/draw-call meter.
    var statistics = false
    /// Whether SceneKit supplies a light when the scene has none of its own.
    var defaultLighting = true

    var debugOptions: SCNDebugOptions {
        var out: SCNDebugOptions = []
        if wireframe { out.insert(.showWireframe) }
        if renderAsWireframe { out.insert(.renderAsWireframe) }
        if boundingBoxes { out.insert(.showBoundingBoxes) }
        if skeletons { out.insert(.showSkeletons) }
        if creases { out.insert(.showCreases) }
        if cameras { out.insert(.showCameras) }
        if lightInfluences { out.insert(.showLightInfluences) }
        if lightExtents { out.insert(.showLightExtents) }
        if physicsShapes { out.insert(.showPhysicsShapes) }
        if physicsFields { out.insert(.showPhysicsFields) }
        if constraints { out.insert(.showConstraints) }
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
            Toggle("Render as Wireframe", isOn: $options.renderAsWireframe)
            Toggle("Bounding Boxes", isOn: $options.boundingBoxes)
            Toggle("Skeleton", isOn: $options.skeletons)
            Divider()
            Toggle("Statistics", isOn: $options.statistics)
            Toggle("Default Lighting", isOn: $options.defaultLighting)
            Divider()
            Menu("Scene Overlays") {
                Toggle("Cameras", isOn: $options.cameras)
                Toggle("Light Influences", isOn: $options.lightInfluences)
                Toggle("Light Extents", isOn: $options.lightExtents)
                Toggle("Subdivision Creases", isOn: $options.creases)
                Toggle("Slider Constraints", isOn: $options.constraints)
                Divider()
                Toggle("Physics Shapes", isOn: $options.physicsShapes)
                Toggle("Physics Fields", isOn: $options.physicsFields)
            }
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
