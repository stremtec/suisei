//  ModelWorkbench.swift
//  What the scene is made of, and changing it.
//
//  `ModelViewer` answers "is this the right mesh". This answers the questions
//  after that one — which node is that, what material is on it, where are the
//  cameras and lights, why is it black — and lets the answers be edited.
//
//  Xcode's SceneKit editor is the reference, and the shape is deliberately its
//  shape: an outline of the graph over an inspector that shows the sections the
//  selection actually has. What is NOT here is Xcode's manipulator gizmo — a
//  drag-in-the-viewport translate/rotate/scale widget is a large amount of
//  hit-testing and undo machinery for something the numeric transform fields do
//  exactly, and exactly is what a workbench beside a file tree is for.
//
//  Edits are held in memory and written out through Export. Nothing overwrites
//  the file that was opened: SceneKit can only WRITE `.scn`, `.dae` and USD,
//  and silently turning somebody's `.fbx` into something else on ⌘S would be
//  the wrong kind of helpful.

import AppKit
import SceneKit
import SwiftUI
import UniformTypeIdentifiers

// MARK: - The document

/// One open model: its scene, the selection over it, and whether it has been
/// changed.
///
/// `SCNNode` is a reference type that publishes nothing, so SwiftUI cannot
/// observe it. `revision` is the substitute — every edit bumps it, and every
/// inspector reads it, which is what makes a field redraw after the value
/// underneath it moves. `treeRevision` is bumped only by edits that change the
/// SHAPE of the graph, so dragging a slider does not rebuild the outline.
@MainActor
final class ModelDocument: ObservableObject {
    /// Hit-testing mask. The selection box is drawn into the scene, and
    /// without this the second click on an already-selected object hits the
    /// box instead of the object.
    static let highlightCategory = 1 << 30
    static let pickableCategory = 0x3FFF_FFFF

    @Published private(set) var scene: SCNScene?
    @Published var revision = 0
    @Published var treeRevision = 0
    @Published var dirty = false
    @Published var view = ModelViewOptions()
    @Published var clips: [ModelClip] = []
    @Published var selectedClip = ""
    @Published var playing = false
    @Published var selectedID: ObjectIdentifier?
    /// Set only when the workbench overrides it; otherwise the pane's palette
    /// decides, like every other viewer.
    @Published var backgroundOverride: Color?
    @Published var pointOfView: SCNNode?
    @Published var command: ModelCommand?

    private(set) var url: URL?
    private(set) var stats: ModelStats?
    private(set) var missingTextures: [String] = []
    private(set) var relocatedTextures = 0

    /// The camera the viewer orbits. Always ours, always in the scene, never
    /// written out — the file's own cameras are listed beside it and can be
    /// looked through, but orbiting one would edit the file.
    let viewerCamera: SCNNode = {
        let node = SCNNode()
        node.name = "\(suiseiInternalNodePrefix)viewer"
        let camera = SCNCamera()
        // Wide enough that a scene authored in centimetres does not clip, near
        // enough that one authored in metres does not vanish at close range.
        camera.zNear = 0.001
        camera.zFar = 1_000_000
        camera.wantsExposureAdaptation = false
        node.camera = camera
        node.categoryBitMask = ModelDocument.highlightCategory
        return node
    }()

    private var highlight: SCNNode?

    var selectedNode: SCNNode? {
        guard let id = selectedID else { return nil }
        return node(with: id)
    }

    // MARK: Lifecycle

    func clear() {
        clips.forEach { $0.stop() }
        scene = nil
        url = nil
        stats = nil
        missingTextures = []
        relocatedTextures = 0
        clips = []
        selectedClip = ""
        selectedID = nil
        playing = false
        dirty = false
        highlight = nil
        backgroundOverride = nil
        pointOfView = nil
        view = ModelViewOptions()
        revision &+= 1
        treeRevision &+= 1
    }

    func adopt(_ scene: SCNScene, url: URL, load: ModelLoad) {
        self.scene = scene
        self.url = url
        stats = load.stats
        missingTextures = load.missingTextures
        relocatedTextures = load.relocatedTextures
        // Physics starts stopped. A scene with dynamic bodies would otherwise
        // collapse into a heap the instant it is opened, and "is this the right
        // mesh" cannot be answered from a heap.
        scene.physicsWorld.speed = 0
        scene.rootNode.addChildNode(viewerCamera)
        pointOfView = viewerCamera
        clips = ModelLoad.clips(in: scene.rootNode)
        // Paused on arrival. An asset that starts moving the instant it opens
        // is answering a question nobody asked.
        clips.forEach { $0.stop() }
        selectedClip = clips.first?.id ?? ""
        revision &+= 1
        treeRevision &+= 1
    }

    /// A value changed. Redraw the inspectors and remember the file no longer
    /// matches what is on disk.
    func edited(structural: Bool = false) {
        dirty = true
        revision &+= 1
        if structural { treeRevision &+= 1 }
    }

    /// A binding that writes through to SceneKit and then says so.
    func bind<V>(_ get: @escaping () -> V, _ set: @escaping (V) -> Void) -> Binding<V> {
        Binding(get: get, set: { [weak self] v in set(v); self?.edited() })
    }

    // MARK: Selection

    func select(_ node: SCNNode?) {
        guard let node, !node.isInternalToSuisei else {
            selectedID = nil
            applyHighlight()
            return
        }
        selectedID = ObjectIdentifier(node)
        applyHighlight()
    }

    func node(with id: ObjectIdentifier) -> SCNNode? {
        guard let root = scene?.rootNode else { return nil }
        var found: SCNNode?
        func walk(_ n: SCNNode) {
            if found != nil { return }
            if ObjectIdentifier(n) == id { found = n; return }
            n.childNodes.forEach(walk)
        }
        walk(root)
        return found
    }

    /// A wireframe box on the current pick.
    ///
    /// Drawn depth-independent and last, so a selection inside a closed mesh is
    /// still visible — the point of the marker is to say "this one", and a
    /// marker you have to orbit to find does not say it.
    func applyHighlight() {
        highlight?.removeFromParentNode()
        highlight = nil
        guard let node = selectedNode, node !== scene?.rootNode else { return }

        let (lo, hi) = node.boundingBox
        var w = CGFloat(hi.x - lo.x)
        var h = CGFloat(hi.y - lo.y)
        var l = CGFloat(hi.z - lo.z)
        let degenerate = w < 1e-6 && h < 1e-6 && l < 1e-6
        if degenerate {
            // An empty, a camera or a light has no box. Give it a marker sized
            // against the scene so it is neither invisible nor the whole view.
            let extent = stats?.extent ?? SIMD3(repeating: 1)
            let s = CGFloat(max(max(extent.x, extent.y), max(extent.z, 0.001))) * 0.04
            w = s; h = s; l = s
        }
        let box = SCNBox(width: w, height: h, length: l, chamferRadius: 0)
        let material = SCNMaterial()
        material.lightingModel = .constant
        material.diffuse.contents = NSColor.controlAccentColor
        material.emission.contents = NSColor.controlAccentColor
        material.fillMode = .lines
        material.isDoubleSided = true
        material.writesToDepthBuffer = false
        material.readsFromDepthBuffer = false
        box.materials = [material]

        let marker = SCNNode(geometry: box)
        marker.name = "\(suiseiInternalNodePrefix)selection"
        marker.categoryBitMask = Self.highlightCategory
        marker.renderingOrder = 10_000
        if !degenerate {
            marker.position = SCNVector3((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, (lo.z + hi.z) / 2)
        }
        node.addChildNode(marker)
        highlight = marker
    }

    // MARK: Facts for the Info panel

    func infoSections() -> [ViewerInfoSection] {
        guard let url else { return [] }
        guard let scene else { return [ViewerInfoSection.file(url)] }
        // Recounted rather than remembered: the workbench can delete a mesh,
        // and a polygon count that still described the file on disk would be
        // the wrong answer to the only question this panel is asked.
        let c = ModelLoad.count(scene.rootNode)
        let box = ModelLoad.geometryBounds(scene.rootNode)
        let extent = SIMD3<Float>(
            Float(box.max.x - box.min.x),
            Float(box.max.y - box.min.y),
            Float(box.max.z - box.min.z)
        )
        let dims = NumberFormatter()
        dims.maximumFractionDigits = 2
        func size(_ v: Float) -> String { dims.string(from: NSNumber(value: v)) ?? "—" }

        // What a 3D file is asked about, in the order it is asked. The polygon
        // count leads because a budget is quoted in polygons; the attribute
        // line is next because "why is it flat" and "why is the texture
        // missing" are both answered by whether the mesh carries normals and
        // UVs at all.
        var sections: [ViewerInfoSection] = [
            ViewerInfoSection("Geometry", [
                ("Polygons", c.polygons.formatted()),
                ("Vertices", c.vertices.formatted()),
                ("Meshes", "\(c.meshes)"),
                ("Materials", "\(c.materials)"),
                ("Attributes", [
                    c.hasNormals ? "normals" : nil,
                    c.hasUVs ? "UVs" : nil,
                    c.skinned ? "skinned (\(c.bones) bones)" : nil,
                ].compactMap { $0 }.joined(separator: " · ")),
            ]),
            ViewerInfoSection("Scene", [
                // Suisei's own camera and selection box are filtered out by
                // `ModelCounts`, so these are the file's numbers.
                ("Nodes", "\(c.nodes)"),
                ("Depth", "\(c.depth)"),
                ("Animations", c.animations > 0 ? "\(c.animations)" : "none"),
                ("Cameras", c.cameras > 0 ? "\(c.cameras)" : nil),
                ("Lights", c.lights > 0 ? "\(c.lights)" : nil),
                ("Particles", c.particles > 0 ? "\(c.particles)" : nil),
            ]),
        ]
        // Only when there is something to say. A model whose textures all
        // resolved should not carry a row saying so.
        if !missingTextures.isEmpty || relocatedTextures > 0 {
            sections.append(ViewerInfoSection("Textures", [
                ("Missing", missingTextures.isEmpty ? nil : "\(missingTextures.count)"),
                ("Names", missingTextures.isEmpty
                    ? nil : missingTextures.prefix(4).joined(separator: ", ")),
                ("Relocated", relocatedTextures > 0 ? "\(relocatedTextures)" : nil),
            ]))
        }
        sections.append(ViewerInfoSection("Bounds", [
            ("Width", size(extent.x)),
            ("Height", size(extent.y)),
            ("Depth", size(extent.z)),
            // Its own units, said out loud. A model is authored in metres or
            // centimetres and the file rarely says which; a number with no unit
            // beside it invites the wrong one to be assumed.
            ("Units", "file units"),
        ]))
        sections.append(ViewerInfoSection.file(url))
        return sections
    }

    // MARK: Scene queries

    func nodes(where match: (SCNNode) -> Bool) -> [SCNNode] {
        guard let root = scene?.rootNode else { return [] }
        var out: [SCNNode] = []
        func walk(_ n: SCNNode) {
            if !n.isInternalToSuisei, match(n) { out.append(n) }
            n.childNodes.forEach(walk)
        }
        walk(root)
        return out
    }

    var authoredCameras: [SCNNode] { nodes { $0.camera != nil } }
    var lights: [SCNNode] { nodes { $0.light != nil } }
    var emitters: [SCNNode] { nodes { !($0.particleSystems ?? []).isEmpty } }

    // MARK: Structural edits

    func add(_ node: SCNNode, to parent: SCNNode? = nil) {
        let host = parent ?? selectedNode ?? scene?.rootNode
        host?.addChildNode(node)
        edited(structural: true)
        select(node)
    }

    func deleteSelection() {
        guard let node = selectedNode, node !== scene?.rootNode else { return }
        let parent = node.parent
        highlight?.removeFromParentNode()
        highlight = nil
        node.removeFromParentNode()
        edited(structural: true)
        select(parent === scene?.rootNode ? nil : parent)
    }

    func duplicateSelection() {
        guard let node = selectedNode, node !== scene?.rootNode,
              let parent = node.parent else { return }
        highlight?.removeFromParentNode()
        highlight = nil
        let copy = node.clone()
        copy.name = (node.name ?? "Node") + " copy"
        parent.addChildNode(copy)
        edited(structural: true)
        select(copy)
    }

    /// Put the viewer camera where the file's camera is, so the authored shot
    /// becomes a place to orbit from rather than a cage.
    func adoptCameraPosition(_ node: SCNNode) {
        viewerCamera.transform = node.presentation.worldTransform
        if let source = node.camera, let target = viewerCamera.camera {
            target.fieldOfView = source.fieldOfView
            target.usesOrthographicProjection = source.usesOrthographicProjection
            target.orthographicScale = source.orthographicScale
        }
        pointOfView = viewerCamera
        revision &+= 1
    }
}

// MARK: - The panel

struct ModelWorkbench: View {
    @ObservedObject var doc: ModelDocument
    let palette: ViewerPalette

    @State private var outlineHeight: CGFloat = 240
    @State private var exporting = false

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(palette.dim.opacity(0.18))
            ModelOutline(doc: doc, palette: palette)
                .frame(height: outlineHeight)
            ModelWorkbenchHDivider(height: $outlineHeight, palette: palette)
            ScrollView { inspector }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(palette.bg)
    }

    private var header: some View {
        HStack(spacing: 6) {
            Text(doc.url?.lastPathComponent ?? "Scene")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(palette.fg)
                .lineLimit(1)
                .truncationMode(.middle)
            if doc.dirty {
                Circle()
                    .fill(palette.accent)
                    .frame(width: 5, height: 5)
                    .help("Edited. Export to keep the changes.")
            }
            Spacer()
            Menu {
                Button("Export as SceneKit Scene…") { export(.scn) }
                Button("Export as COLLADA…") { export(.dae) }
                Button("Export as USDZ…") { export(.usdz) }
            } label: {
                Image(systemName: "square.and.arrow.up")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(palette.fg)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Export the edited scene")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
    }

    @ViewBuilder
    private var inspector: some View {
        // Read once so every section below is invalidated by an edit. SCNNode
        // publishes nothing; this is the subscription.
        let _ = doc.revision
        VStack(alignment: .leading, spacing: 0) {
            if let node = doc.selectedNode {
                ModelNodeInspector(doc: doc, node: node, palette: palette)
            } else {
                ModelSceneInspector(doc: doc, palette: palette)
            }
        }
        .padding(.bottom, 18)
    }

    private func export(_ format: ModelExportFormat) {
        guard let scene = doc.scene else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = [format.type].compactMap { $0 }
        panel.nameFieldStringValue =
            (doc.url?.deletingPathExtension().lastPathComponent ?? "scene") + "." + format.ext
        panel.canCreateDirectories = true
        panel.title = "Export Scene"
        guard panel.runModal() == .OK, let target = panel.url else { return }

        // Our own camera and the selection marker are not part of anybody's
        // model. Taken out for the write and put straight back, because the
        // user is still looking through one of them.
        let restore = ModelExport.detachInternals(scene)
        let ok = scene.write(to: target, options: nil, delegate: nil, progressHandler: nil)
        restore()
        if ok {
            doc.dirty = false
        } else {
            let alert = NSAlert()
            alert.messageText = "Could not export the scene"
            alert.informativeText =
                "SceneKit refused to write \(format.ext.uppercased()). "
                + "Try SceneKit Scene, which can hold anything the editor can make."
            alert.runModal()
        }
    }
}

enum ModelExportFormat {
    case scn, dae, usdz

    var ext: String {
        switch self {
        case .scn: return "scn"
        case .dae: return "dae"
        case .usdz: return "usdz"
        }
    }

    var type: UTType? {
        switch self {
        case .scn: return UTType("com.apple.scenekit.scene")
        case .dae: return UTType("org.khronos.collada.digital-asset-exchange")
        case .usdz: return UTType("com.pixar.universal-scene-description-mobile")
        }
    }
}

enum ModelExport {
    /// Lift Suisei's own nodes out of the graph and hand back the undo.
    static func detachInternals(_ scene: SCNScene) -> () -> Void {
        var removed: [(SCNNode, SCNNode)] = []
        func walk(_ n: SCNNode) {
            for child in n.childNodes {
                if child.isInternalToSuisei {
                    removed.append((child, n))
                } else {
                    walk(child)
                }
            }
        }
        walk(scene.rootNode)
        removed.forEach { $0.0.removeFromParentNode() }
        return {
            for (node, parent) in removed { parent.addChildNode(node) }
        }
    }
}

/// The drag handle between the outline and the inspector.
private struct ModelWorkbenchHDivider: View {
    @Binding var height: CGFloat
    let palette: ViewerPalette
    @State private var start: CGFloat?

    var body: some View {
        Rectangle()
            .fill(palette.dim.opacity(0.18))
            .frame(height: 1)
            .overlay {
                Rectangle()
                    .fill(.clear)
                    .frame(height: 9)
                    .contentShape(Rectangle())
                    .onHover { NSCursor.resizeUpDown.set(); if !$0 { NSCursor.arrow.set() } }
                    .gesture(
                        DragGesture(minimumDistance: 1)
                            .onChanged { g in
                                let base = start ?? height
                                if start == nil { start = height }
                                height = min(560, max(96, base + g.translation.height))
                            }
                            .onEnded { _ in start = nil }
                    )
            }
    }
}

// MARK: - Outline

/// The scene graph, as a tree you can pick from.
///
/// Built as values rather than handed `SCNNode`s directly: `List` wants
/// `Identifiable` and `Hashable`, and a class that mutates under SwiftUI is the
/// wrong thing to key a row on. The identity is the node's address, which is
/// stable for exactly as long as the node is in the scene.
struct ModelOutline: View {
    @ObservedObject var doc: ModelDocument
    let palette: ViewerPalette

    var body: some View {
        // Rebuilt when the graph's shape changes, not when a value does.
        let tree = ModelOutlineItem.build(doc.scene?.rootNode, revision: doc.treeRevision)
        VStack(spacing: 0) {
            List(selection: Binding(
                get: { doc.selectedID },
                set: { doc.select($0.flatMap { doc.node(with: $0) }) }
            )) {
                if let tree {
                    OutlineGroup(tree, children: \.children) { item in
                        ModelOutlineRow(item: item, palette: palette)
                            .tag(item.id)
                    }
                }
            }
            .listStyle(.sidebar)
            .scrollContentBackground(.hidden)
            .environment(\.defaultMinListRowHeight, 20)

            outlineToolbar
        }
    }

    private var outlineToolbar: some View {
        HStack(spacing: 2) {
            Menu {
                Button("Empty Node") { doc.add(named("Node")) }
                Divider()
                Button("Box") { doc.add(shape(SCNBox(width: 1, height: 1, length: 1, chamferRadius: 0), "Box")) }
                Button("Sphere") { doc.add(shape(SCNSphere(radius: 0.5), "Sphere")) }
                Button("Plane") { doc.add(shape(SCNPlane(width: 1, height: 1), "Plane")) }
                Divider()
                Button("Camera") {
                    let n = named("Camera")
                    n.camera = SCNCamera()
                    doc.add(n)
                }
                Menu("Light") {
                    ForEach(ModelLightKind.all, id: \.self) { kind in
                        Button(kind.label) {
                            let n = named(kind.label)
                            let light = SCNLight()
                            light.type = kind.type
                            light.intensity = 1000
                            n.light = light
                            doc.add(n)
                        }
                    }
                }
                Button("Particle System") {
                    let n = named("Particles")
                    n.addParticleSystem(ModelParticles.makeDefault())
                    doc.add(n)
                }
            } label: {
                Image(systemName: "plus")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Add a node")

            Button { doc.deleteSelection() } label: { Image(systemName: "minus") }
                .buttonStyle(.plain)
                .disabled(doc.selectedNode == nil)
                .help("Delete the selected node")

            Button { doc.duplicateSelection() } label: { Image(systemName: "plus.square.on.square") }
                .buttonStyle(.plain)
                .disabled(doc.selectedNode == nil)
                .help("Duplicate")

            Spacer()

            Button { doc.command = .frameSelection } label: {
                Image(systemName: "viewfinder")
            }
            .buttonStyle(.plain)
            .help("Frame the selection")
        }
        .font(.system(size: 11, weight: .medium))
        .foregroundStyle(palette.fg)
        .padding(.horizontal, 9)
        .padding(.vertical, 5)
        .background(palette.bg)
        .overlay(alignment: .top) {
            Rectangle().fill(palette.dim.opacity(0.15)).frame(height: 1)
        }
    }

    private func named(_ name: String) -> SCNNode {
        let n = SCNNode()
        n.name = name
        return n
    }

    private func shape(_ geometry: SCNGeometry, _ name: String) -> SCNNode {
        let n = SCNNode(geometry: geometry)
        n.name = name
        let m = SCNMaterial()
        m.lightingModel = .physicallyBased
        m.diffuse.contents = NSColor(white: 0.72, alpha: 1)
        m.roughness.contents = NSColor(white: 0.6, alpha: 1)
        geometry.materials = [m]
        return n
    }
}

/// One row's worth of tree.
struct ModelOutlineItem: Identifiable {
    let id: ObjectIdentifier
    let node: SCNNode
    let children: [ModelOutlineItem]?

    /// `revision` is unused inside and deliberately so: it is the argument
    /// that makes SwiftUI rebuild this when the graph's shape changes.
    static func build(_ root: SCNNode?, revision: Int) -> [ModelOutlineItem]? {
        guard let root else { return nil }
        func make(_ n: SCNNode) -> ModelOutlineItem? {
            guard !n.isInternalToSuisei else { return nil }
            let kids = n.childNodes.compactMap(make)
            return ModelOutlineItem(
                id: ObjectIdentifier(n),
                node: n,
                children: kids.isEmpty ? nil : kids
            )
        }
        // The root itself is not shown: a file's root is a container SceneKit
        // made up, not something the author placed, and a tree with one
        // permanent top row wastes a level of indentation on every other row.
        return root.childNodes.compactMap(make)
    }
}

struct ModelOutlineRow: View {
    let item: ModelOutlineItem
    let palette: ViewerPalette

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: symbol)
                .font(.system(size: 10))
                .foregroundStyle(item.node.isHidden ? palette.dim.opacity(0.5) : palette.accent)
                .frame(width: 13)
            Text(label)
                .font(.system(size: 11))
                .foregroundStyle(item.node.isHidden ? palette.dim : palette.fg)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 2)
            if item.node.isHidden {
                Image(systemName: "eye.slash")
                    .font(.system(size: 9))
                    .foregroundStyle(palette.dim)
            }
        }
        .padding(.vertical, 1)
    }

    private var label: String {
        if let name = item.node.name, !name.isEmpty { return name }
        if item.node.camera != nil { return "Camera" }
        if item.node.light != nil { return "Light" }
        if item.node.geometry != nil { return "Mesh" }
        return "Node"
    }

    private var symbol: String {
        if item.node.camera != nil { return "camera" }
        if item.node.light != nil { return "lightbulb" }
        if !(item.node.particleSystems ?? []).isEmpty { return "sparkles" }
        if item.node.skinner != nil { return "figure.walk" }
        if item.node.geometry != nil { return "cube" }
        if item.node.physicsBody != nil { return "atom" }
        return "square.dashed"
    }
}

// MARK: - Scene inspector (nothing selected)

struct ModelSceneInspector: View {
    @ObservedObject var doc: ModelDocument
    let palette: ViewerPalette

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ModelCamerasPanel(doc: doc, palette: palette)
            ModelLightsPanel(doc: doc, palette: palette)
            ModelEnvironmentPanel(doc: doc, palette: palette)
            ModelPhysicsWorldPanel(doc: doc, palette: palette)
            if !doc.missingTextures.isEmpty {
                ModelTexturesPanel(doc: doc, palette: palette)
            }
        }
    }
}

// MARK: - Node inspector

struct ModelNodeInspector: View {
    @ObservedObject var doc: ModelDocument
    let node: SCNNode
    let palette: ViewerPalette

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            WBSection("Node", "square.dashed", palette: palette) {
                WBRow("Name", palette: palette) {
                    TextField("", text: doc.bind({ node.name ?? "" }, { node.name = $0 }))
                        .textFieldStyle(.roundedBorder)
                        .font(.system(size: 11))
                }
                WBRow("Visible", palette: palette) {
                    Toggle("", isOn: doc.bind({ !node.isHidden }, { node.isHidden = !$0 }))
                        .labelsHidden()
                        .toggleStyle(.switch)
                        .controlSize(.mini)
                }
                WBSlider("Opacity", palette: palette, range: 0...1,
                         value: doc.bind({ node.opacity }, { node.opacity = $0 }))
                WBRow("Shadow", palette: palette) {
                    Toggle("", isOn: doc.bind({ node.castsShadow }, { node.castsShadow = $0 }))
                        .labelsHidden().toggleStyle(.switch).controlSize(.mini)
                }
                WBRow("Order", palette: palette) {
                    WBNumberField(
                        value: doc.bind(
                            { CGFloat(node.renderingOrder) },
                            { node.renderingOrder = Int($0) }),
                        palette: palette)
                }
            }

            WBSection("Transform", "move.3d", palette: palette) {
                WBVector("Position", palette: palette,
                         x: doc.bind({ node.position.x }, { node.position.x = $0 }),
                         y: doc.bind({ node.position.y }, { node.position.y = $0 }),
                         z: doc.bind({ node.position.z }, { node.position.z = $0 }))
                // Degrees at the surface, radians underneath. SceneKit stores
                // radians and every artist thinks in degrees; converting here
                // is cheaper than a note explaining why 1.57 is a right angle.
                WBVector("Rotation", palette: palette, suffix: "°",
                         x: degrees(doc.bind({ node.eulerAngles.x }, { node.eulerAngles.x = $0 })),
                         y: degrees(doc.bind({ node.eulerAngles.y }, { node.eulerAngles.y = $0 })),
                         z: degrees(doc.bind({ node.eulerAngles.z }, { node.eulerAngles.z = $0 })))
                WBVector("Scale", palette: palette,
                         x: doc.bind({ node.scale.x }, { node.scale.x = $0 }),
                         y: doc.bind({ node.scale.y }, { node.scale.y = $0 }),
                         z: doc.bind({ node.scale.z }, { node.scale.z = $0 }))
                WBRow("", palette: palette) {
                    HStack(spacing: 6) {
                        Button("Reset") {
                            node.transform = SCNMatrix4Identity
                            doc.edited()
                            doc.applyHighlight()
                        }
                        Button("Frame") { doc.command = .frameSelection }
                        Spacer()
                    }
                    .font(.system(size: 10))
                    .buttonStyle(.bordered)
                    .controlSize(.mini)
                }
            }

            if let geometry = node.geometry {
                ModelGeometryPanel(doc: doc, node: node, geometry: geometry, palette: palette)
            }
            if let camera = node.camera {
                ModelCameraPanel(doc: doc, node: node, camera: camera, palette: palette)
            }
            if let light = node.light {
                ModelLightPanel(doc: doc, light: light, palette: palette)
            }
            if let systems = node.particleSystems, !systems.isEmpty {
                ForEach(Array(systems.enumerated()), id: \.offset) { _, system in
                    ModelParticlePanel(doc: doc, node: node, system: system, palette: palette)
                }
            }
            ModelNodePhysicsPanel(doc: doc, node: node, palette: palette)
            ModelActionsPanel(doc: doc, node: node, palette: palette)
        }
    }

    /// Radians in the model, degrees in the field.
    private func degrees(_ radians: Binding<CGFloat>) -> Binding<CGFloat> {
        Binding(
            get: { radians.wrappedValue * 180 / .pi },
            set: { radians.wrappedValue = $0 * .pi / 180 }
        )
    }
}

// MARK: - Geometry and materials

struct ModelGeometryPanel: View {
    @ObservedObject var doc: ModelDocument
    let node: SCNNode
    let geometry: SCNGeometry
    let palette: ViewerPalette
    @State private var materialIndex = 0

    var body: some View {
        WBSection("Geometry", "cube", palette: palette) {
            let verts = geometry.sources.first { $0.semantic == .vertex }?.vectorCount ?? 0
            WBFact("Polygons", geometry.faceCount.formatted(), palette: palette)
            WBFact("Vertices", verts.formatted(), palette: palette)
            WBFact("Materials", "\(geometry.materials.count)", palette: palette)
            if let skinner = node.skinner {
                WBFact("Bones", "\(skinner.bones.count)", palette: palette)
            }
            WBRow("Subdivision", palette: palette) {
                WBNumberField(
                    value: doc.bind(
                        { CGFloat(geometry.subdivisionLevel) },
                        { geometry.subdivisionLevel = max(0, min(4, Int($0))) }),
                    palette: palette)
            }
        }

        if !geometry.materials.isEmpty {
            WBSection("Material", "paintpalette", palette: palette) {
                if geometry.materials.count > 1 {
                    WBRow("Slot", palette: palette) {
                        Picker("", selection: $materialIndex) {
                            ForEach(Array(geometry.materials.enumerated()), id: \.offset) { i, m in
                                Text(m.name ?? "Material \(i + 1)").tag(i)
                            }
                        }
                        .labelsHidden().pickerStyle(.menu).font(.system(size: 11))
                    }
                }
                let index = min(materialIndex, geometry.materials.count - 1)
                ModelMaterialEditor(
                    doc: doc, material: geometry.materials[index], palette: palette)
            }
        }
    }
}

struct ModelMaterialEditor: View {
    @ObservedObject var doc: ModelDocument
    let material: SCNMaterial
    let palette: ViewerPalette

    private static let lightingModels: [(SCNMaterial.LightingModel, String)] = [
        (.physicallyBased, "Physically Based"),
        (.blinn, "Blinn"),
        (.phong, "Phong"),
        (.lambert, "Lambert"),
        (.constant, "Constant"),
        (.shadowOnly, "Shadow Only"),
    ]

    var body: some View {
        WBRow("Lighting", palette: palette) {
            Picker("", selection: doc.bind(
                { material.lightingModel }, { material.lightingModel = $0 })
            ) {
                ForEach(Self.lightingModels, id: \.0) { Text($0.1).tag($0.0) }
            }
            .labelsHidden().pickerStyle(.menu).font(.system(size: 11))
        }

        ModelChannelRow(doc: doc, material: material, slot: .diffuse, palette: palette)
        ModelChannelRow(doc: doc, material: material, slot: .emission, palette: palette)
        if material.lightingModel == .physicallyBased {
            ModelChannelRow(doc: doc, material: material, slot: .metalness, palette: palette)
            ModelChannelRow(doc: doc, material: material, slot: .roughness, palette: palette)
        } else {
            ModelChannelRow(doc: doc, material: material, slot: .specular, palette: palette)
        }
        ModelChannelRow(doc: doc, material: material, slot: .normal, palette: palette)

        WBSlider("Transparency", palette: palette, range: 0...1,
                 value: doc.bind({ material.transparency }, { material.transparency = $0 }))
        WBRow("Alpha from", palette: palette) {
            Picker("", selection: doc.bind(
                { material.transparencyMode }, { material.transparencyMode = $0 })
            ) {
                Text("Alpha channel").tag(SCNTransparencyMode.aOne)
                Text("RGB (COLLADA)").tag(SCNTransparencyMode.rgbZero)
                Text("Single layer").tag(SCNTransparencyMode.singleLayer)
                Text("Dual layer").tag(SCNTransparencyMode.dualLayer)
            }
            .labelsHidden().pickerStyle(.menu).font(.system(size: 11))
        }
        WBRow("Double-sided", palette: palette) {
            Toggle("", isOn: doc.bind({ material.isDoubleSided }, { material.isDoubleSided = $0 }))
                .labelsHidden().toggleStyle(.switch).controlSize(.mini)
        }
        WBRow("Fill", palette: palette) {
            Picker("", selection: doc.bind({ material.fillMode }, { material.fillMode = $0 })) {
                Text("Filled").tag(SCNFillMode.fill)
                Text("Lines").tag(SCNFillMode.lines)
            }
            .labelsHidden().pickerStyle(.segmented).font(.system(size: 10))
        }
    }
}

/// One texture-bearing channel: a colour well when it holds a colour, and the
/// texture's name when it holds a texture.
///
/// Showing a colour well over a texture would be a control that lies — the
/// well would display grey for an image, and setting it would silently throw
/// the image away.
struct ModelChannelRow: View {
    @ObservedObject var doc: ModelDocument
    let material: SCNMaterial
    let slot: ModelTextureSlot
    let palette: ViewerPalette

    var body: some View {
        let property = slot.property(material)
        WBRow(slot.label, palette: palette) {
            HStack(spacing: 6) {
                if let texture = textureName(property.contents) {
                    Image(systemName: "photo")
                        .font(.system(size: 9))
                        .foregroundStyle(palette.dim)
                    Text(texture)
                        .font(.system(size: 10))
                        .foregroundStyle(palette.dim)
                        .lineLimit(1).truncationMode(.middle)
                    Spacer(minLength: 0)
                    Button("Clear") {
                        property.contents = slot.fallback
                        doc.edited()
                    }
                    .buttonStyle(.bordered).controlSize(.mini).font(.system(size: 9))
                } else {
                    ColorPicker("", selection: doc.bind(
                        { Self.color(property.contents) },
                        { property.contents = NSColor($0) }
                    ), supportsOpacity: true)
                    .labelsHidden()
                    .frame(width: 38)
                    Spacer(minLength: 0)
                    Button("Texture…") { chooseTexture(property) }
                        .buttonStyle(.bordered).controlSize(.mini).font(.system(size: 9))
                }
            }
        }
    }

    private func textureName(_ contents: Any?) -> String? {
        if let url = contents as? URL { return url.lastPathComponent }
        if contents is NSImage { return "image" }
        // `as? CGImage` is a bridging cast that always succeeds, so identity
        // has to be asked of Core Foundation directly.
        if let contents, CFGetTypeID(contents as CFTypeRef) == CGImage.typeID { return "image" }
        return nil
    }

    private func chooseTexture(_ property: SCNMaterialProperty) {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        property.contents = NSImage(contentsOf: url) ?? url as Any
        doc.edited()
    }

    static func color(_ contents: Any?) -> Color {
        if let c = contents as? NSColor { return Color(nsColor: c) }
        if let contents, CFGetTypeID(contents as CFTypeRef) == CGColor.typeID,
           let c = NSColor(cgColor: contents as! CGColor) {
            return Color(nsColor: c)
        }
        // A single number is a greyscale channel value — SceneKit accepts one
        // for metalness and roughness, and the well should show what it means.
        if let n = contents as? NSNumber { return Color(white: n.doubleValue) }
        return .gray
    }
}

// MARK: - Cameras

struct ModelCamerasPanel: View {
    @ObservedObject var doc: ModelDocument
    let palette: ViewerPalette

    var body: some View {
        WBSection("Cameras", "camera", palette: palette) {
            let authored = doc.authoredCameras
            ModelPickRow(
                title: "Viewer",
                subtitle: "orbit · pan · zoom",
                selected: doc.pointOfView === doc.viewerCamera,
                palette: palette
            ) {
                doc.pointOfView = doc.viewerCamera
            }
            ForEach(authored, id: \.self) { node in
                ModelPickRow(
                    title: node.name ?? "Camera",
                    subtitle: "authored · fixed",
                    selected: doc.pointOfView === node,
                    palette: palette
                ) {
                    doc.pointOfView = node
                } trailing: {
                    Button {
                        doc.adoptCameraPosition(node)
                    } label: {
                        Image(systemName: "arrow.down.left.arrow.up.right")
                            .font(.system(size: 9))
                    }
                    .buttonStyle(.borderless)
                    .help("Move the viewer camera here")
                }
            }
            if authored.isEmpty {
                WBNote("This file has no camera of its own.", palette: palette)
            } else {
                // Said out loud, because the stage stops orbiting and that
                // looks like a bug otherwise.
                WBNote(
                    "Looking through an authored camera is fixed — orbiting "
                        + "would move the file's camera.",
                    palette: palette)
            }
        }
    }
}

struct ModelCameraPanel: View {
    @ObservedObject var doc: ModelDocument
    let node: SCNNode
    let camera: SCNCamera
    let palette: ViewerPalette

    var body: some View {
        WBSection("Camera", "camera", palette: palette) {
            WBRow("Projection", palette: palette) {
                Picker("", selection: doc.bind(
                    { camera.usesOrthographicProjection },
                    { camera.usesOrthographicProjection = $0 })
                ) {
                    Text("Perspective").tag(false)
                    Text("Orthographic").tag(true)
                }
                .labelsHidden().pickerStyle(.segmented).font(.system(size: 10))
            }
            if camera.usesOrthographicProjection {
                WBRow("Scale", palette: palette) {
                    WBNumberField(
                        value: doc.bind(
                            { CGFloat(camera.orthographicScale) },
                            { camera.orthographicScale = Double($0) }),
                        palette: palette)
                }
            } else {
                WBSlider("Field of view", palette: palette, range: 1...160, suffix: "°",
                         value: doc.bind(
                            { CGFloat(camera.fieldOfView) },
                            { camera.fieldOfView = $0 }))
            }
            WBRow("Near", palette: palette) {
                WBNumberField(
                    value: doc.bind({ CGFloat(camera.zNear) }, { camera.zNear = Double($0) }),
                    palette: palette)
            }
            WBRow("Far", palette: palette) {
                WBNumberField(
                    value: doc.bind({ CGFloat(camera.zFar) }, { camera.zFar = Double($0) }),
                    palette: palette)
            }
            WBRow("HDR", palette: palette) {
                Toggle("", isOn: doc.bind({ camera.wantsHDR }, { camera.wantsHDR = $0 }))
                    .labelsHidden().toggleStyle(.switch).controlSize(.mini)
            }
            if camera.wantsHDR {
                WBSlider("Bloom", palette: palette, range: 0...1,
                         value: doc.bind(
                            { CGFloat(camera.bloomIntensity) },
                            { camera.bloomIntensity = $0 }))
            }
            WBRow("", palette: palette) {
                HStack {
                    Button("Look Through") { doc.pointOfView = node }
                    Button("Use Position") { doc.adoptCameraPosition(node) }
                    Spacer()
                }
                .font(.system(size: 10))
                .buttonStyle(.bordered).controlSize(.mini)
            }
        }
    }
}

// MARK: - Lights and environment

enum ModelLightKind: Hashable {
    case omni, directional, spot, ambient, area, probe

    static var all: [ModelLightKind] { [.omni, .directional, .spot, .ambient, .area] }

    var type: SCNLight.LightType {
        switch self {
        case .omni: return .omni
        case .directional: return .directional
        case .spot: return .spot
        case .ambient: return .ambient
        case .area: return .area
        case .probe: return .probe
        }
    }

    var label: String {
        switch self {
        case .omni: return "Omni"
        case .directional: return "Directional"
        case .spot: return "Spot"
        case .ambient: return "Ambient"
        case .area: return "Area"
        case .probe: return "Probe"
        }
    }

    static func of(_ type: SCNLight.LightType) -> ModelLightKind {
        all.first { $0.type == type } ?? .omni
    }
}

struct ModelLightsPanel: View {
    @ObservedObject var doc: ModelDocument
    let palette: ViewerPalette

    var body: some View {
        WBSection("Lights", "lightbulb", palette: palette) {
            let lights = doc.lights
            ForEach(lights, id: \.self) { node in
                ModelPickRow(
                    title: node.name ?? "Light",
                    subtitle: node.light.map { ModelLightKind.of($0.type).label } ?? "",
                    selected: doc.selectedNode === node,
                    palette: palette
                ) {
                    doc.select(node)
                }
            }
            if lights.isEmpty {
                WBNote(
                    "No lights in the file. SceneKit is supplying one — turn "
                        + "off Default Lighting in the view menu to see the "
                        + "scene as authored.",
                    palette: palette)
            }
            WBRow("", palette: palette) {
                Menu {
                    ForEach(ModelLightKind.all, id: \.self) { kind in
                        Button(kind.label) { addLight(kind) }
                    }
                } label: {
                    Text("Add Light").font(.system(size: 10))
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }
        }
    }

    private func addLight(_ kind: ModelLightKind) {
        let node = SCNNode()
        node.name = "\(kind.label) Light"
        let light = SCNLight()
        light.type = kind.type
        light.intensity = kind == .ambient ? 200 : 1000
        light.castsShadow = kind == .directional || kind == .spot
        node.light = light
        // Above and in front of whatever is there, which is where a light put
        // at the origin is useless.
        let extent = doc.stats?.extent ?? SIMD3(repeating: 1)
        let reach = CGFloat(max(max(extent.x, extent.y), max(extent.z, 1))) * 1.5
        node.position = SCNVector3(reach, reach, reach)
        node.look(at: SCNVector3Zero)
        doc.add(node, to: doc.scene?.rootNode)
    }
}

struct ModelLightPanel: View {
    @ObservedObject var doc: ModelDocument
    let light: SCNLight
    let palette: ViewerPalette

    var body: some View {
        WBSection("Light", "lightbulb", palette: palette) {
            WBRow("Type", palette: palette) {
                Picker("", selection: doc.bind(
                    { ModelLightKind.of(light.type) },
                    { light.type = $0.type })
                ) {
                    ForEach(ModelLightKind.all, id: \.self) { Text($0.label).tag($0) }
                }
                .labelsHidden().pickerStyle(.menu).font(.system(size: 11))
            }
            WBRow("Colour", palette: palette) {
                ColorPicker("", selection: doc.bind(
                    { ModelChannelRow.color(light.color) },
                    { light.color = NSColor($0) }
                ), supportsOpacity: false)
                .labelsHidden().frame(width: 38)
            }
            WBRow("Intensity", palette: palette) {
                WBNumberField(
                    value: doc.bind(
                        { CGFloat(light.intensity) }, { light.intensity = $0 }),
                    palette: palette)
            }
            WBSlider("Temperature", palette: palette, range: 1000...10000, suffix: "K",
                     value: doc.bind(
                        { CGFloat(light.temperature) }, { light.temperature = $0 }))
            if light.type == .spot {
                WBSlider("Inner angle", palette: palette, range: 0...180, suffix: "°",
                         value: doc.bind(
                            { CGFloat(light.spotInnerAngle) }, { light.spotInnerAngle = $0 }))
                WBSlider("Outer angle", palette: palette, range: 0...180, suffix: "°",
                         value: doc.bind(
                            { CGFloat(light.spotOuterAngle) }, { light.spotOuterAngle = $0 }))
            }
            WBRow("Shadows", palette: palette) {
                Toggle("", isOn: doc.bind({ light.castsShadow }, { light.castsShadow = $0 }))
                    .labelsHidden().toggleStyle(.switch).controlSize(.mini)
            }
            if light.castsShadow {
                WBSlider("Softness", palette: palette, range: 0...16,
                         value: doc.bind(
                            { CGFloat(light.shadowRadius) }, { light.shadowRadius = $0 }))
                WBRow("Shadow", palette: palette) {
                    ColorPicker("", selection: doc.bind(
                        { ModelChannelRow.color(light.shadowColor) },
                        { light.shadowColor = NSColor($0) }
                    ), supportsOpacity: true)
                    .labelsHidden().frame(width: 38)
                }
            }
        }
    }
}

struct ModelEnvironmentPanel: View {
    @ObservedObject var doc: ModelDocument
    let palette: ViewerPalette

    var body: some View {
        WBSection("Environment", "globe", palette: palette) {
            WBRow("Default light", palette: palette) {
                Toggle("", isOn: Binding(
                    get: { doc.view.defaultLighting },
                    set: { doc.view.defaultLighting = $0 }))
                .labelsHidden().toggleStyle(.switch).controlSize(.mini)
            }
            WBRow("Background", palette: palette) {
                HStack(spacing: 6) {
                    ColorPicker("", selection: Binding(
                        // The theme's `model_bg`, which is white in every
                        // shipped palette — the stage does not follow the
                        // editor theme, and `ViewerPalette.stage` says why.
                        get: { doc.backgroundOverride ?? palette.stage },
                        set: { doc.backgroundOverride = $0 }
                    ), supportsOpacity: false)
                    .labelsHidden().frame(width: 38)
                    if doc.backgroundOverride != nil {
                        Button("Reset") { doc.backgroundOverride = nil }
                            .buttonStyle(.bordered).controlSize(.mini).font(.system(size: 9))
                    }
                    Spacer(minLength: 0)
                }
            }
            WBRow("Lighting", palette: palette) {
                HStack(spacing: 6) {
                    Button("Image…") { chooseEnvironment() }
                        .buttonStyle(.bordered).controlSize(.mini).font(.system(size: 9))
                    if doc.scene?.lightingEnvironment.contents != nil {
                        Button("Clear") {
                            doc.scene?.lightingEnvironment.contents = nil
                            doc.edited()
                        }
                        .buttonStyle(.bordered).controlSize(.mini).font(.system(size: 9))
                    }
                    Spacer(minLength: 0)
                }
            }
            WBNote(
                "An environment image lights physically-based materials the "
                    + "way a real room would. Any image works; an HDR panorama "
                    + "works best.",
                palette: palette)
        }
    }

    private func chooseEnvironment() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        guard panel.runModal() == .OK, let url = panel.url,
              let image = NSImage(contentsOf: url) else { return }
        doc.scene?.lightingEnvironment.contents = image
        doc.scene?.lightingEnvironment.intensity = 1
        doc.edited()
    }
}

struct ModelTexturesPanel: View {
    @ObservedObject var doc: ModelDocument
    let palette: ViewerPalette

    var body: some View {
        WBSection("Missing textures", "exclamationmark.triangle", palette: palette) {
            ForEach(doc.missingTextures, id: \.self) { name in
                Text(name)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(palette.dim)
                    .lineLimit(1).truncationMode(.middle)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12)
            }
            WBNote(
                "The file names these by absolute path and they are not on "
                    + "this machine. Suisei also looked next to the model and "
                    + "in textures/, maps/ and images/. Each channel is showing "
                    + "its neutral value instead — without that, a COLLADA "
                    + "RGB_ZERO transparency map would render the mesh invisible.",
                palette: palette)
        }
    }
}

// MARK: - Particles

enum ModelParticles {
    static func makeDefault() -> SCNParticleSystem {
        let system = SCNParticleSystem()
        system.birthRate = 120
        system.particleLifeSpan = 2
        system.particleSize = 0.08
        system.particleColor = .white
        system.emitterShape = SCNSphere(radius: 0.05)
        system.particleVelocity = 1.2
        system.spreadingAngle = 25
        system.blendMode = .additive
        return system
    }
}

struct ModelParticlePanel: View {
    @ObservedObject var doc: ModelDocument
    let node: SCNNode
    let system: SCNParticleSystem
    let palette: ViewerPalette

    var body: some View {
        WBSection("Particles", "sparkles", palette: palette) {
            WBRow("Birth rate", palette: palette) {
                WBNumberField(
                    value: doc.bind({ system.birthRate }, { system.birthRate = max(0, $0) }),
                    palette: palette)
            }
            WBRow("Life span", palette: palette) {
                WBNumberField(
                    value: doc.bind(
                        { system.particleLifeSpan }, { system.particleLifeSpan = max(0, $0) }),
                    palette: palette)
            }
            WBRow("Size", palette: palette) {
                WBNumberField(
                    value: doc.bind(
                        { system.particleSize }, { system.particleSize = max(0, $0) }),
                    palette: palette)
            }
            WBRow("Velocity", palette: palette) {
                WBNumberField(
                    value: doc.bind({ system.particleVelocity }, { system.particleVelocity = $0 }),
                    palette: palette)
            }
            WBSlider("Spread", palette: palette, range: 0...180, suffix: "°",
                     value: doc.bind({ system.spreadingAngle }, { system.spreadingAngle = $0 }))
            WBRow("Colour", palette: palette) {
                ColorPicker("", selection: doc.bind(
                    { Color(nsColor: system.particleColor) },
                    { system.particleColor = NSColor($0) }
                ), supportsOpacity: true)
                .labelsHidden().frame(width: 38)
            }
            WBRow("Blend", palette: palette) {
                Picker("", selection: doc.bind(
                    { system.blendMode }, { system.blendMode = $0 })
                ) {
                    Text("Additive").tag(SCNParticleBlendMode.additive)
                    Text("Alpha").tag(SCNParticleBlendMode.alpha)
                    Text("Multiply").tag(SCNParticleBlendMode.multiply)
                    Text("Screen").tag(SCNParticleBlendMode.screen)
                    Text("Subtract").tag(SCNParticleBlendMode.subtract)
                    Text("Replace").tag(SCNParticleBlendMode.replace)
                }
                .labelsHidden().pickerStyle(.menu).font(.system(size: 11))
            }
            WBRow("Gravity", palette: palette) {
                Toggle("", isOn: doc.bind(
                    { system.isAffectedByGravity }, { system.isAffectedByGravity = $0 }))
                .labelsHidden().toggleStyle(.switch).controlSize(.mini)
            }
            WBRow("Loops", palette: palette) {
                Toggle("", isOn: doc.bind({ system.loops }, { system.loops = $0 }))
                    .labelsHidden().toggleStyle(.switch).controlSize(.mini)
            }
            WBRow("", palette: palette) {
                HStack(spacing: 6) {
                    Button("Restart") { system.reset(); doc.edited() }
                    Button("Remove") {
                        node.removeParticleSystem(system)
                        doc.edited(structural: true)
                    }
                    Spacer()
                }
                .font(.system(size: 10))
                .buttonStyle(.bordered).controlSize(.mini)
            }
            WBNote(
                "Particles only move while the scene is simulating — turn on "
                    + "Simulate under Physics.",
                palette: palette)
        }
    }
}

// MARK: - Physics

struct ModelPhysicsWorldPanel: View {
    @ObservedObject var doc: ModelDocument
    let palette: ViewerPalette

    var body: some View {
        WBSection("Physics", "atom", palette: palette) {
            if let world = doc.scene?.physicsWorld {
                WBRow("Simulate", palette: palette) {
                    Toggle("", isOn: doc.bind(
                        { world.speed > 0 }, { world.speed = $0 ? 1 : 0 }))
                    .labelsHidden().toggleStyle(.switch).controlSize(.mini)
                }
                WBVector("Gravity", palette: palette,
                         x: doc.bind({ world.gravity.x }, { world.gravity.x = $0 }),
                         y: doc.bind({ world.gravity.y }, { world.gravity.y = $0 }),
                         z: doc.bind({ world.gravity.z }, { world.gravity.z = $0 }))
                WBRow("", palette: palette) {
                    HStack(spacing: 6) {
                        Button("Earth") {
                            world.gravity = SCNVector3(0, -9.8, 0)
                            doc.edited()
                        }
                        Button("Zero") {
                            world.gravity = SCNVector3Zero
                            doc.edited()
                        }
                        Spacer()
                    }
                    .font(.system(size: 10))
                    .buttonStyle(.bordered).controlSize(.mini)
                }
                WBNote(
                    "Simulation starts stopped. A scene of dynamic bodies "
                        + "collapses into a heap the moment it runs, and that "
                        + "is not a preview of the model.",
                    palette: palette)
            }
        }
    }
}

struct ModelNodePhysicsPanel: View {
    @ObservedObject var doc: ModelDocument
    let node: SCNNode
    let palette: ViewerPalette

    var body: some View {
        WBSection("Physics Body", "atom", palette: palette) {
            if let body = node.physicsBody {
                WBRow("Type", palette: palette) {
                    Picker("", selection: Binding(
                        get: { body.type },
                        set: { newType in
                            // `type` is get-only after construction, so the body
                            // is replaced rather than mutated.
                            node.physicsBody = SCNPhysicsBody(type: newType, shape: body.physicsShape)
                            doc.edited()
                        }
                    )) {
                        Text("Static").tag(SCNPhysicsBodyType.static)
                        Text("Dynamic").tag(SCNPhysicsBodyType.dynamic)
                        Text("Kinematic").tag(SCNPhysicsBodyType.kinematic)
                    }
                    .labelsHidden().pickerStyle(.menu).font(.system(size: 11))
                }
                WBRow("Mass", palette: palette) {
                    WBNumberField(
                        value: doc.bind({ body.mass }, { body.mass = max(0, $0) }),
                        palette: palette)
                }
                WBSlider("Bounce", palette: palette, range: 0...1,
                         value: doc.bind({ body.restitution }, { body.restitution = $0 }))
                WBSlider("Friction", palette: palette, range: 0...1,
                         value: doc.bind({ body.friction }, { body.friction = $0 }))
                WBSlider("Damping", palette: palette, range: 0...1,
                         value: doc.bind({ body.damping }, { body.damping = $0 }))
                WBRow("Gravity", palette: palette) {
                    Toggle("", isOn: doc.bind(
                        { body.isAffectedByGravity }, { body.isAffectedByGravity = $0 }))
                    .labelsHidden().toggleStyle(.switch).controlSize(.mini)
                }
                WBRow("", palette: palette) {
                    HStack {
                        Button("Remove") { node.physicsBody = nil; doc.edited() }
                        Spacer()
                    }
                    .font(.system(size: 10))
                    .buttonStyle(.bordered).controlSize(.mini)
                }
            } else {
                WBRow("", palette: palette) {
                    HStack(spacing: 6) {
                        ForEach(
                            [("Static", SCNPhysicsBodyType.static),
                             ("Dynamic", .dynamic),
                             ("Kinematic", .kinematic)], id: \.0
                        ) { label, type in
                            Button(label) { addBody(type) }
                        }
                        Spacer()
                    }
                    .font(.system(size: 10))
                    .buttonStyle(.bordered).controlSize(.mini)
                }
                WBNote("No physics body. Give it one to make it collide.", palette: palette)
            }
        }
    }

    private func addBody(_ type: SCNPhysicsBodyType) {
        // A convex hull for anything that moves, the exact mesh for anything
        // that does not: SceneKit will not run a dynamic body against a
        // concave triangle mesh, and silently getting no collisions is worse
        // than a hull that is slightly too fat.
        let options: [SCNPhysicsShape.Option: Any] = [
            .type: type == .static
                ? SCNPhysicsShape.ShapeType.concavePolyhedron
                : SCNPhysicsShape.ShapeType.convexHull
        ]
        let shape = node.geometry != nil ? SCNPhysicsShape(node: node, options: options) : nil
        node.physicsBody = SCNPhysicsBody(type: type, shape: shape)
        doc.edited()
    }
}

// MARK: - Actions

/// Xcode's action editor is a timeline that authors `SCNAction`s into a scene
/// file. This is the part of it a workbench needs: the standard motions,
/// applied to the selection, so "does this pivot look right" can be answered
/// without writing a test harness. They are runtime actions — they are not
/// written into the export, and the panel says so.
struct ModelActionsPanel: View {
    @ObservedObject var doc: ModelDocument
    let node: SCNNode
    let palette: ViewerPalette

    @State private var seconds: CGFloat = 2
    @State private var amount: CGFloat = 1

    var body: some View {
        WBSection("Actions", "play.circle", palette: palette) {
            WBRow("Duration", palette: palette) {
                WBNumberField(value: $seconds, palette: palette)
            }
            WBRow("Amount", palette: palette) {
                WBNumberField(value: $amount, palette: palette)
            }
            WBRow("", palette: palette) {
                VStack(alignment: .leading, spacing: 5) {
                    HStack(spacing: 6) {
                        Button("Spin") { run(spin) }
                        Button("Bob") { run(bob) }
                        Button("Pulse") { run(pulse) }
                    }
                    HStack(spacing: 6) {
                        Button("Fade") { run(fade) }
                        Button("Orbit") { run(orbit) }
                        Button("Stop") {
                            node.removeAllActions()
                            doc.edited()
                        }
                    }
                }
                .font(.system(size: 10))
                .buttonStyle(.bordered).controlSize(.mini)
            }
            WBNote(
                "Actions run in the viewer. They are motion you are trying "
                    + "out, not data — an export writes the scene, not what is "
                    + "currently moving in it.",
                palette: palette)
        }
    }

    private func run(_ make: () -> SCNAction) {
        node.removeAllActions()
        node.runAction(.repeatForever(make()))
        doc.edited()
    }

    private var duration: TimeInterval { TimeInterval(max(0.05, seconds)) }

    private func spin() -> SCNAction {
        .rotateBy(x: 0, y: .pi * 2, z: 0, duration: duration)
    }

    private func bob() -> SCNAction {
        let up = SCNAction.moveBy(x: 0, y: amount, z: 0, duration: duration / 2)
        up.timingMode = .easeInEaseOut
        return .sequence([up, up.reversed()])
    }

    private func pulse() -> SCNAction {
        let grow = SCNAction.scale(by: 1 + max(0.01, amount) * 0.25, duration: duration / 2)
        grow.timingMode = .easeInEaseOut
        return .sequence([grow, grow.reversed()])
    }

    private func fade() -> SCNAction {
        .sequence([
            .fadeOut(duration: duration / 2),
            .fadeIn(duration: duration / 2),
        ])
    }

    private func orbit() -> SCNAction {
        // Around the world origin rather than its own: a turntable of the
        // whole model is what "orbit" means to somebody looking at a scene.
        .customAction(duration: duration) { node, elapsed in
            let t = CGFloat(elapsed / CGFloat(self.duration)) * .pi * 2
            let r = max(0.01, self.amount)
            node.position = SCNVector3(cos(t) * r, node.position.y, sin(t) * r)
        }
    }
}

// MARK: - Small parts

/// A titled, collapsible group. Collapsed state lives per-title so opening
/// "Transform" on one node leaves it open on the next.
struct WBSection<Content: View>: View {
    let title: String
    let symbol: String
    let palette: ViewerPalette
    @ViewBuilder let content: Content
    @State private var open = true

    init(
        _ title: String, _ symbol: String, palette: ViewerPalette,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.symbol = symbol
        self.palette = palette
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeOut(duration: 0.12)) { open.toggle() }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 8, weight: .bold))
                        .rotationEffect(.degrees(open ? 90 : 0))
                        .foregroundStyle(palette.dim)
                    Image(systemName: symbol)
                        .font(.system(size: 9))
                        .foregroundStyle(palette.dim)
                    Text(title.uppercased())
                        .font(.system(size: 9, weight: .semibold))
                        .tracking(0.5)
                        .foregroundStyle(palette.dim)
                    Spacer()
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if open {
                VStack(alignment: .leading, spacing: 4) { content }
                    .padding(.bottom, 8)
            }
            Rectangle().fill(palette.dim.opacity(0.12)).frame(height: 1)
        }
    }
}

/// Label on the left, control on the right, at one width for the whole panel.
struct WBRow<Content: View>: View {
    let label: String
    let palette: ViewerPalette
    @ViewBuilder let content: Content

    init(_ label: String, palette: ViewerPalette, @ViewBuilder content: () -> Content) {
        self.label = label
        self.palette = palette
        self.content = content()
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label)
                .font(.system(size: 10))
                .foregroundStyle(palette.dim)
                .frame(width: 74, alignment: .trailing)
            content
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 10)
    }
}

/// A row that only states something.
struct WBFact: View {
    let label: String
    let value: String
    let palette: ViewerPalette

    init(_ label: String, _ value: String, palette: ViewerPalette) {
        self.label = label
        self.value = value
        self.palette = palette
    }

    var body: some View {
        WBRow(label, palette: palette) {
            Text(value)
                .font(.system(size: 10, weight: .medium).monospacedDigit())
                .foregroundStyle(palette.fg)
        }
    }
}

struct WBNote: View {
    let text: String
    let palette: ViewerPalette

    init(_ text: String, palette: ViewerPalette) {
        self.text = text
        self.palette = palette
    }

    var body: some View {
        Text(text)
            .font(.system(size: 9.5))
            .foregroundStyle(palette.dim.opacity(0.85))
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 12)
            .padding(.top, 2)
    }
}

/// A number you can type or drag.
struct WBNumberField: View {
    @Binding var value: CGFloat
    let palette: ViewerPalette
    var suffix: String = ""

    @State private var text = ""
    @State private var editing = false
    @FocusState private var focused: Bool

    var body: some View {
        TextField("", text: $text)
            .textFieldStyle(.roundedBorder)
            .font(.system(size: 10).monospacedDigit())
            .multilineTextAlignment(.trailing)
            .focused($focused)
            .onSubmit(commit)
            .onChange(of: focused) { _, now in
                editing = now
                if now { text = Self.format(value) } else { commit() }
            }
            // While the field is focused the user's half-typed text is the
            // truth; outside it the model is. Overwriting mid-edit is what
            // makes a field impossible to type a minus sign into.
            .onChange(of: value) { _, now in
                if !editing { text = Self.format(now) }
            }
            .onAppear { text = Self.format(value) }
    }

    private func commit() {
        let cleaned = text.replacingOccurrences(of: suffix, with: "")
            .trimmingCharacters(in: .whitespaces)
        if let parsed = Double(cleaned) { value = CGFloat(parsed) }
        text = Self.format(value)
    }

    static func format(_ v: CGFloat) -> String {
        if v == v.rounded(), abs(v) < 1e9 { return String(Int(v)) }
        return String(format: "%.4g", Double(v))
    }
}

/// Three numbers that mean one thing.
struct WBVector: View {
    let label: String
    let palette: ViewerPalette
    var suffix: String = ""
    @Binding var x: CGFloat
    @Binding var y: CGFloat
    @Binding var z: CGFloat

    init(
        _ label: String, palette: ViewerPalette, suffix: String = "",
        x: Binding<CGFloat>, y: Binding<CGFloat>, z: Binding<CGFloat>
    ) {
        self.label = label
        self.palette = palette
        self.suffix = suffix
        _x = x
        _y = y
        _z = z
    }

    var body: some View {
        WBRow(label, palette: palette) {
            HStack(spacing: 3) {
                WBNumberField(value: $x, palette: palette, suffix: suffix)
                WBNumberField(value: $y, palette: palette, suffix: suffix)
                WBNumberField(value: $z, palette: palette, suffix: suffix)
            }
        }
    }
}

/// A number with a range, which is a slider with the number beside it.
struct WBSlider: View {
    let label: String
    let palette: ViewerPalette
    let range: ClosedRange<CGFloat>
    var suffix: String = ""
    @Binding var value: CGFloat

    init(
        _ label: String, palette: ViewerPalette, range: ClosedRange<CGFloat>,
        suffix: String = "", value: Binding<CGFloat>
    ) {
        self.label = label
        self.palette = palette
        self.range = range
        self.suffix = suffix
        _value = value
    }

    var body: some View {
        WBRow(label, palette: palette) {
            HStack(spacing: 6) {
                Slider(value: $value, in: range)
                    .controlSize(.mini)
                Text(WBNumberField.format(value) + suffix)
                    .font(.system(size: 9).monospacedDigit())
                    .foregroundStyle(palette.dim)
                    .frame(width: 34, alignment: .trailing)
            }
        }
    }
}

/// A row in a list you pick from — a camera, a light.
struct ModelPickRow<Trailing: View>: View {
    let title: String
    let subtitle: String
    let selected: Bool
    let palette: ViewerPalette
    let action: () -> Void
    @ViewBuilder let trailing: Trailing

    init(
        title: String, subtitle: String, selected: Bool, palette: ViewerPalette,
        action: @escaping () -> Void,
        @ViewBuilder trailing: () -> Trailing = { EmptyView() }
    ) {
        self.title = title
        self.subtitle = subtitle
        self.selected = selected
        self.palette = palette
        self.action = action
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: 6) {
            Button(action: action) {
                HStack(spacing: 6) {
                    Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                        .font(.system(size: 10))
                        .foregroundStyle(selected ? palette.accent : palette.dim)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(title)
                            .font(.system(size: 11))
                            .foregroundStyle(palette.fg)
                            .lineLimit(1)
                        if !subtitle.isEmpty {
                            Text(subtitle)
                                .font(.system(size: 9))
                                .foregroundStyle(palette.dim)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            trailing
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 2)
    }
}
