//  ModelScene.swift
//  Reading a 3D file, and making what came back drawable.
//
//  Deliberately free of SwiftUI and of anything else in the app: this is the
//  part of the model pane that can be pointed at a file and asked what it got,
//  which is how `.dae` texture repair and FBX skinning were verified without
//  driving the running editor. `ModelViewer` is the pane; this is the file.

import AppKit
import GLTFKit2
import SceneKit

/// Nodes this app adds to somebody else's scene: the viewer camera, the
/// selection marker. They must not be counted, framed, listed or exported —
/// a polygon budget that included Suisei's own selection box would be a lie
/// about the file.
let suiseiInternalNodePrefix = "__suisei."

extension SCNNode {
    var isInternalToSuisei: Bool {
        (name ?? "").hasPrefix(suiseiInternalNodePrefix)
    }
}

extension SCNGeometry {
    /// Faces, whatever shape they are.
    ///
    /// `primitiveCount` counts triangles for `.triangles` and `.triangleStrip`
    /// and POLYGONS for `.polygon`, which is the number an artist quotes. A sum
    /// that only took `.triangles` reported an untriangulated mesh as having
    /// none — measured on `cube.obj`, which SceneKit hands back as one
    /// `.polygon` element of six, and which the Info panel called "Polygons: 0".
    ///
    /// Every element, because a mesh can carry more than one and a count that
    /// stopped at the first would under-report exactly the models with the
    /// most in them.
    var faceCount: Int {
        elements.reduce(0) { total, element in
            switch element.primitiveType {
            case .triangles, .triangleStrip, .polygon: return total + element.primitiveCount
            case .line, .point: return total
            @unknown default: return total
            }
        }
    }
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
    /// Faces. The number an artist asks for first — a budget is quoted in
    /// polygons, never in vertices.
    var polygons: Int
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
    var bones: Int
    var cameras: Int
    var lights: Int
    var particles: Int
    /// The model's bounding box in its own units.
    var extent: SIMD3<Float>
}

/// One load: the scene, what it is made of, and what to show if it failed.
struct ModelLoad {
    enum Result {
        case success(SCNScene)
        case failure(String)
    }

    let result: Result
    var stats: ModelStats?
    /// Texture files the material referenced and nothing could find. Named,
    /// because "why is my model grey" has exactly one answer and this is it.
    var missingTextures: [String] = []
    /// Textures that were named by an absolute path from another machine and
    /// found anyway, next to the model. Worth saying: it means the file is
    /// portable by luck, not by construction.
    var relocatedTextures = 0

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

        // Before anything is counted or drawn. A material that cannot resolve
        // its textures does not merely look wrong — under COLLADA's RGB_ZERO
        // transparency it renders NOTHING, and the counts below would then
        // describe a scene the user cannot see.
        let repair = ModelTextures.repair(scene, near: url)
        missingTextures = repair.missing
        relocatedTextures = repair.relocated

        let counted = ModelLoad.count(scene.rootNode)
        // A scene with no geometry is not a success, whatever SceneKit
        // returned. Measured: a malformed STL does not throw — it comes back
        // as an empty scene, and an empty stage is indistinguishable from a
        // model that loaded and is off camera. Those two need different
        // reactions from the user, so they get different screens.
        //
        // A particle system is the one scene with nothing to count that is
        // still worth showing, so it is allowed through explicitly.
        guard counted.meshes > 0 || counted.particles > 0 else {
            result = .failure(
                "The file opened but contains no geometry. It may be an "
                    + "unsupported variant of \(url.pathExtension.uppercased())."
            )
            return
        }
        result = .success(scene)
        let box = ModelLoad.geometryBounds(scene.rootNode)
        let extent = SIMD3<Float>(
            Float(box.max.x - box.min.x),
            Float(box.max.y - box.min.y),
            Float(box.max.z - box.min.z)
        )
        stats = ModelStats(
            meshes: counted.meshes,
            vertices: counted.vertices,
            polygons: counted.polygons,
            materials: counted.materials,
            animations: counted.animations,
            nodes: counted.nodes,
            depth: counted.depth,
            hasNormals: counted.hasNormals,
            hasUVs: counted.hasUVs,
            skinned: counted.skinned,
            bones: counted.bones,
            cameras: counted.cameras,
            lights: counted.lights,
            particles: counted.particles,
            extent: extent
        )
    }

    /// Whichever reader knows this extension.
    ///
    /// Four readers, one result. Each hands back an ordinary `SCNScene`, so
    /// this is the only place that has to know there is more than one — and it
    /// is here rather than in the view because "what can open this file" is a
    /// fact about the file, not about the pane.
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
        case "scnp":
            return try particleScene(url)
        default:
            // `checkConsistency` off deliberately: it rejects files that other
            // tools open, and the alternative to a slightly odd scene here is
            // no scene at all.
            return try SCNScene(url: url, options: [.checkConsistency: false])
        }
    }

    /// A bare particle system, given a scene to live in.
    ///
    /// `.scnp` is a keyed archive of one `SCNParticleSystem` and nothing else —
    /// no node, no scene — so the pane has to supply the rest. Secure coding is
    /// off because the archive legitimately carries images and colours whose
    /// classes are not knowable up front, and this is a file the user asked to
    /// open.
    private static func particleScene(_ url: URL) throws -> SCNScene {
        let data = try Data(contentsOf: url)
        let unarchiver = try NSKeyedUnarchiver(forReadingFrom: data)
        unarchiver.requiresSecureCoding = false
        guard let system = unarchiver.decodeObject(
            forKey: NSKeyedArchiveRootObjectKey
        ) as? SCNParticleSystem else {
            throw NSError(
                domain: "Suisei.Model", code: 2,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        "The file is not a SceneKit particle system."
                ]
            )
        }
        let scene = SCNScene()
        let node = SCNNode()
        node.name = url.deletingPathExtension().lastPathComponent
        node.addParticleSystem(system)
        scene.rootNode.addChildNode(node)
        return scene
    }

    /// The bounds of the GEOMETRY, which is not the bounds of the graph.
    ///
    /// Measured on `B389_loop.dae`: the mesh spans about one unit and the file
    /// also carries a camera rig fifty-four units away. `frameNodes` on the
    /// root therefore framed a sphere dominated by an empty node, and the mesh
    /// came out twelve pixels wide in the middle of an apparently blank stage.
    /// Cameras and lights are not the model.
    static func geometryNodes(_ root: SCNNode) -> [SCNNode] {
        var out: [SCNNode] = []
        func walk(_ n: SCNNode) {
            guard !n.isInternalToSuisei else { return }
            if n.geometry != nil { out.append(n) }
            n.childNodes.forEach(walk)
        }
        walk(root)
        // Nothing to frame but particles: frame their emitters instead, or the
        // camera controller is handed an empty list and does nothing at all.
        if out.isEmpty {
            func emitters(_ n: SCNNode) {
                guard !n.isInternalToSuisei else { return }
                if !(n.particleSystems ?? []).isEmpty { out.append(n) }
                n.childNodes.forEach(emitters)
            }
            emitters(root)
        }
        return out.isEmpty ? [root] : out
    }

    /// The union of every mesh's world-space box.
    static func geometryBounds(_ root: SCNNode) -> (min: SCNVector3, max: SCNVector3) {
        var lo = SIMD3<Float>(repeating: .greatestFiniteMagnitude)
        var hi = SIMD3<Float>(repeating: -.greatestFiniteMagnitude)
        var any = false
        for node in geometryNodes(root) where node.geometry != nil {
            let (bmin, bmax) = node.boundingBox
            // All eight corners, because a rotated box's extremes are not its
            // two transformed corners.
            for xi in 0...1 {
                for yi in 0...1 {
                    for zi in 0...1 {
                        let local = SCNVector3(
                            xi == 0 ? bmin.x : bmax.x,
                            yi == 0 ? bmin.y : bmax.y,
                            zi == 0 ? bmin.z : bmax.z
                        )
                        let w = node.convertPosition(local, to: nil)
                        let v = SIMD3<Float>(Float(w.x), Float(w.y), Float(w.z))
                        lo = simd_min(lo, v)
                        hi = simd_max(hi, v)
                        any = true
                    }
                }
            }
        }
        guard any else { return (SCNVector3Zero, SCNVector3Zero) }
        return (
            SCNVector3(CGFloat(lo.x), CGFloat(lo.y), CGFloat(lo.z)),
            SCNVector3(CGFloat(hi.x), CGFloat(hi.y), CGFloat(hi.z))
        )
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
            guard !node.isInternalToSuisei else { return }
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
    /// One walk rather than eight: a scene graph can be deep, and asking it the
    /// same question eight times is eight traversals for one answer.
    static func count(_ root: SCNNode) -> ModelCounts {
        var c = ModelCounts()
        // Materials are shared between nodes far more often than not, so they
        // are counted by identity — a cube with one material on six faces has
        // one material, not six.
        var materials: Set<ObjectIdentifier> = []

        func walk(_ node: SCNNode, _ level: Int) {
            guard !node.isInternalToSuisei else { return }
            c.nodes += 1
            c.depth = max(c.depth, level)
            if let geometry = node.geometry {
                c.meshes += 1
                for source in geometry.sources {
                    switch source.semantic {
                    case .vertex: c.vertices += source.vectorCount
                    case .normal: c.hasNormals = true
                    case .texcoord: c.hasUVs = true
                    default: break
                    }
                }
                c.polygons += geometry.faceCount
                for material in geometry.materials {
                    materials.insert(ObjectIdentifier(material))
                }
            }
            if let skinner = node.skinner {
                c.skinned = true
                c.bones = max(c.bones, skinner.bones.count)
            }
            if node.camera != nil { c.cameras += 1 }
            if node.light != nil { c.lights += 1 }
            c.particles += (node.particleSystems ?? []).count
            c.animations += node.animationKeys.count
            for child in node.childNodes { walk(child, level + 1) }
        }
        walk(root, 0)
        c.materials = materials.count
        return c
    }
}

/// One traversal's worth of answers.
struct ModelCounts {
    var meshes = 0
    var vertices = 0
    var polygons = 0
    var materials = 0
    var animations = 0
    var nodes = 0
    var depth = 0
    var bones = 0
    var cameras = 0
    var lights = 0
    var particles = 0
    var hasNormals = false
    var hasUVs = false
    var skinned = false
}

// MARK: - Textures

/// Making a material's texture references resolve, or saying they do not.
///
/// **Why this exists.** `perf-fixtures/animated.dae` rendered as an empty
/// stage. The geometry was intact — 8281 vertices, 16200 triangles, indices in
/// range — and rebuilding the same sources into a fresh `SCNGeometry` with a
/// fresh material drew it perfectly. The material was the whole defect: both
/// its `diffuse` and its `transparent` channel pointed at
/// `/job/comms/…/loremipsum.png`, an absolute path on somebody else's render
/// farm, and its `transparencyMode` was `.rgbZero`. An unreadable transparent
/// channel under RGB_ZERO samples as black, black means fully transparent, and
/// the mesh disappears completely.
///
/// That is not a quirk of one file. Absolute texture paths are what every DCC
/// tool writes by default, so every model that arrives from another machine
/// hits this — silently, and looking exactly like a broken viewer.
enum ModelTextures {
    struct Outcome {
        var missing: [String] = []
        var relocated = 0
    }

    /// Where a texture might actually be, given where the model is.
    private static let searchDirs = [
        "", "textures", "Textures", "texture", "Texture",
        "maps", "Maps", "images", "Images", "tex", "materials", "Materials",
    ]

    static func repair(_ scene: SCNScene, near url: URL) -> Outcome {
        var out = Outcome()
        var seen: Set<String> = []
        var visited: Set<ObjectIdentifier> = []

        func fix(_ material: SCNMaterial) {
            guard visited.insert(ObjectIdentifier(material)).inserted else { return }
            for slot in ModelTextureSlot.all {
                let property = slot.property(material)
                guard let ref = property.contents as? URL,
                      !FileManager.default.fileExists(atPath: ref.path)
                else { continue }
                if let found = relocate(ref, near: url) {
                    property.contents = found
                    out.relocated += 1
                    continue
                }
                if seen.insert(ref.lastPathComponent).inserted {
                    out.missing.append(ref.lastPathComponent)
                }
                property.contents = slot.fallback
                // The load-bearing line. Leaving `.rgbZero` on a channel with
                // nothing to read erases the mesh; `.aOne` is SceneKit's own
                // default and means "use the alpha", which a flat colour has.
                if slot == .transparent { material.transparencyMode = .aOne }
            }
        }

        func walk(_ node: SCNNode) {
            node.geometry?.materials.forEach(fix)
            node.childNodes.forEach(walk)
        }
        walk(scene.rootNode)
        return out
    }

    /// The same file name, somewhere sane relative to the model.
    ///
    /// Exporters write the path they saw at export time; what survives the
    /// trip is the file's NAME and its position relative to the model. Trying
    /// that is what every other 3D tool does before giving up.
    private static func relocate(_ ref: URL, near model: URL) -> URL? {
        let dir = model.deletingLastPathComponent()
        let name = ref.lastPathComponent
        guard !name.isEmpty else { return nil }
        for sub in searchDirs {
            let candidate = sub.isEmpty
                ? dir.appendingPathComponent(name)
                : dir.appendingPathComponent(sub).appendingPathComponent(name)
            if FileManager.default.fileExists(atPath: candidate.path) { return candidate }
        }
        return nil
    }
}

/// One texture-bearing channel of a material, and what it should say when the
/// texture is gone.
///
/// The fallbacks are the identity value for each channel, not a guess: a
/// missing normal map is flat (0.5, 0.5, 1), a missing roughness map is
/// mid-rough, a missing occlusion map occludes nothing. Substituting mid-grey
/// everywhere would make a metallic model look sandblasted.
enum ModelTextureSlot: String, CaseIterable {
    case diffuse, transparent, normal, metalness, roughness
    case emission, specular, ambientOcclusion, selfIllumination, displacement, multiply

    static var all: [ModelTextureSlot] { allCases }

    func property(_ m: SCNMaterial) -> SCNMaterialProperty {
        switch self {
        case .diffuse: return m.diffuse
        case .transparent: return m.transparent
        case .normal: return m.normal
        case .metalness: return m.metalness
        case .roughness: return m.roughness
        case .emission: return m.emission
        case .specular: return m.specular
        case .ambientOcclusion: return m.ambientOcclusion
        case .selfIllumination: return m.selfIllumination
        case .displacement: return m.displacement
        case .multiply: return m.multiply
        }
    }

    var fallback: NSColor {
        switch self {
        case .diffuse: return NSColor(white: 0.72, alpha: 1)
        case .transparent, .ambientOcclusion, .multiply: return .white
        case .normal: return NSColor(red: 0.5, green: 0.5, blue: 1, alpha: 1)
        case .metalness, .emission, .selfIllumination, .displacement: return .black
        case .roughness: return NSColor(white: 0.6, alpha: 1)
        case .specular: return NSColor(white: 0.2, alpha: 1)
        }
    }

    var label: String {
        switch self {
        case .ambientOcclusion: return "Occlusion"
        case .selfIllumination: return "Self-illumination"
        default: return rawValue.capitalized
        }
    }
}
