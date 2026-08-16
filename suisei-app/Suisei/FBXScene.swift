//  FBXScene.swift
//  Assimp's scene, as SceneKit's.
//
//  FBX is the one format in the viewer that nothing on the system reads and
//  that no Swift reader hands back an `SCNScene` for. Autodesk's own SDK is
//  commercial and carries redistribution terms; Assimp is BSD-3 and reads FBX
//  in both its binary and ASCII spellings, so it is the only way to open one
//  of these in a shipped, source-available app. See third_party/Assimp.
//
//  What Assimp gives back is its own scene graph, so this is the translation —
//  and it is the only place in the viewer that knows FBX exists at all. Above
//  it, an FBX pane is an `SCNScene` like every other.

import Foundation
import SceneKit
import libassimp

enum FBXScene {
    /// Read `url` and rebuild it as a SceneKit scene.
    static func read(_ url: URL) throws -> SCNScene {
        // Triangulate, because `SCNGeometryElement` has no quad primitive and
        // an FBX from a modelling package is quads more often than not.
        // Normals only where the file has none — `GenSmoothNormals` leaves
        // authored ones alone, and an artist's normals are a decision.
        // JoinIdenticalVertices because an FBX stores a vertex per face corner.
        // LimitBoneWeights because `SCNSkinner` takes a fixed number of
        // influences per vertex and Assimp's own cap is the four every engine
        // uses; letting it do the trimming keeps the renormalisation with the
        // library that knows the weights.
        let flags = aiProcess_Triangulate.rawValue
            | aiProcess_GenSmoothNormals.rawValue
            | aiProcess_JoinIdenticalVertices.rawValue
            | aiProcess_LimitBoneWeights.rawValue
            | aiProcess_ImproveCacheLocality.rawValue

        guard let raw = aiImportFile(url.path, UInt32(flags)) else {
            throw NSError(
                domain: "Suisei.FBX", code: 1,
                userInfo: [NSLocalizedDescriptionKey: String(cString: aiGetErrorString())]
            )
        }
        defer { aiReleaseImport(raw) }

        let asset = raw.pointee
        let materials = (0..<Int(asset.mNumMaterials)).map { i in
            material(asset.mMaterials[i])
        }
        let geometries = (0..<Int(asset.mNumMeshes)).map { i -> SCNGeometry in
            geometry(asset.mMeshes[i]!, materials: materials)
        }
        let skins = (0..<Int(asset.mNumMeshes)).map { i -> FBXSkin? in
            FBXSkin(asset.mMeshes[i]!)
        }

        let scene = SCNScene()
        if let root = asset.mRootNode {
            // Which node ended up with which mesh, recorded on the way down.
            // A skinner needs both the geometry and the bone NODES, and the
            // bone nodes do not exist until the whole tree has been built —
            // so binding is a second pass and this is what it needs to find
            // its way back.
            var placed: [(node: SCNNode, mesh: Int)] = []
            let tree = node(root, geometries: geometries, placed: &placed)
            scene.rootNode.addChildNode(tree)
            attachSkins(placed, skins: skins, tree: tree)
            attachAnimations(asset, to: tree)
        }
        return scene
    }

    // MARK: - Skinning

    /// Bind each skinned mesh to the bones that move it.
    ///
    /// Without this an FBX rig animates and the mesh does not: the bone nodes
    /// move, the vertices stay where they were authored, and the model appears
    /// to stand still inside a skeleton walking away from it. Measured on
    /// `animation_with_skeleton.fbx`, which came back `skinned = false`.
    private static func attachSkins(
        _ placed: [(node: SCNNode, mesh: Int)], skins: [FBXSkin?], tree: SCNNode
    ) {
        for (host, meshIndex) in placed {
            guard skins.indices.contains(meshIndex), let skin = skins[meshIndex],
                  let geometry = host.geometry
            else { continue }

            // Every bone must resolve. A skinner built with a placeholder for
            // a bone it could not find would deform toward the origin, which
            // looks like a torn mesh rather than like a missing bone.
            let bones = skin.boneNames.map { name -> SCNNode? in
                tree.name == name ? tree : tree.childNode(withName: name, recursively: true)
            }
            guard !bones.contains(where: { $0 == nil }) else { continue }

            let weights = SCNGeometrySource(
                data: skin.weights,
                semantic: .boneWeights,
                vectorCount: skin.vertexCount,
                usesFloatComponents: true,
                componentsPerVector: FBXSkin.influences,
                bytesPerComponent: MemoryLayout<Float32>.size,
                dataOffset: 0,
                dataStride: MemoryLayout<Float32>.size * FBXSkin.influences
            )
            let indices = SCNGeometrySource(
                data: skin.indices,
                semantic: .boneIndices,
                vectorCount: skin.vertexCount,
                usesFloatComponents: false,
                componentsPerVector: FBXSkin.influences,
                bytesPerComponent: MemoryLayout<UInt16>.size,
                dataOffset: 0,
                dataStride: MemoryLayout<UInt16>.size * FBXSkin.influences
            )

            let skinner = SCNSkinner(
                baseGeometry: geometry,
                bones: bones.compactMap { $0 },
                boneInverseBindTransforms: skin.offsets.map { NSValue(scnMatrix4: $0) },
                boneWeights: weights,
                boneIndices: indices
            )
            // The frame the bind pose was authored in. The imported root is
            // the only node guaranteed to be an ancestor of every bone.
            skinner.skeleton = tree
            host.skinner = skinner
        }
    }

    // MARK: - Animation

    /// Rebuild each `aiAnimation` as an `SCNAnimationPlayer` on the node it
    /// drives.
    ///
    /// Attached to the nodes rather than returned, because that is where the
    /// rest of the viewer already looks: `ModelLoad.clips(in:)` walks
    /// `animationKeys`, which is how a DAE's or a USD's clips are found. An FBX
    /// that hands its animations back through a side channel would need a
    /// second discovery path saying the same thing.
    ///
    /// Paused on arrival — `SCNAnimationPlayer` starts stopped, and the
    /// transport is what starts it.
    private static func attachAnimations(_ asset: aiScene, to root: SCNNode) {
        for a in 0..<Int(asset.mNumAnimations) {
            guard let anim = asset.mAnimations[a] else { continue }
            let clip = anim.pointee
            // Ticks are the file's own unit and the rate is optional; 25 is
            // Assimp's own fallback and the one FBX exporters assume.
            let rate = clip.mTicksPerSecond != 0 ? clip.mTicksPerSecond : 25
            let name = string(clip.mName).isEmpty ? "Take \(a + 1)" : string(clip.mName)

            for c in 0..<Int(clip.mNumChannels) {
                guard let ch = clip.mChannels[c] else { continue }
                let channel = ch.pointee
                let nodeName = string(channel.mNodeName)
                guard let target = root.childNode(withName: nodeName, recursively: true)
                        ?? (root.name == nodeName ? root : nil)
                else { continue }

                var parts: [CAAnimation] = []
                if channel.mNumPositionKeys > 0 {
                    var times: [NSNumber] = []
                    var values: [NSValue] = []
                    for k in 0..<Int(channel.mNumPositionKeys) {
                        let key = channel.mPositionKeys[k]
                        times.append(NSNumber(value: key.mTime / rate))
                        values.append(NSValue(scnVector3: SCNVector3(
                            CGFloat(key.mValue.x), CGFloat(key.mValue.y), CGFloat(key.mValue.z)
                        )))
                    }
                    parts.append(keyframes("position", times, values))
                }
                if channel.mNumRotationKeys > 0 {
                    var times: [NSNumber] = []
                    var values: [NSValue] = []
                    for k in 0..<Int(channel.mNumRotationKeys) {
                        let key = channel.mRotationKeys[k]
                        times.append(NSNumber(value: key.mTime / rate))
                        values.append(NSValue(scnVector4: SCNVector4(
                            CGFloat(key.mValue.x), CGFloat(key.mValue.y),
                            CGFloat(key.mValue.z), CGFloat(key.mValue.w)
                        )))
                    }
                    parts.append(keyframes("orientation", times, values))
                }
                if channel.mNumScalingKeys > 0 {
                    var times: [NSNumber] = []
                    var values: [NSValue] = []
                    for k in 0..<Int(channel.mNumScalingKeys) {
                        let key = channel.mScalingKeys[k]
                        times.append(NSNumber(value: key.mTime / rate))
                        values.append(NSValue(scnVector3: SCNVector3(
                            CGFloat(key.mValue.x), CGFloat(key.mValue.y), CGFloat(key.mValue.z)
                        )))
                    }
                    parts.append(keyframes("scale", times, values))
                }
                guard !parts.isEmpty else { continue }

                let group = CAAnimationGroup()
                group.animations = parts
                group.duration = clip.mDuration / rate
                group.repeatCount = .greatestFiniteMagnitude
                let player = SCNAnimationPlayer(animation: SCNAnimation(caAnimation: group))
                player.stop()
                target.addAnimationPlayer(player, forKey: name)
            }
        }
    }

    private static func keyframes(
        _ path: String, _ times: [NSNumber], _ values: [NSValue]
    ) -> CAKeyframeAnimation {
        let a = CAKeyframeAnimation(keyPath: path)
        a.values = values
        // Normalised to the clip, which is what `keyTimes` means — the raw
        // seconds would put every key at time 1 and the model would snap.
        let end = times.last?.doubleValue ?? 1
        a.keyTimes = end > 0 ? times.map { NSNumber(value: $0.doubleValue / end) } : times
        a.duration = end
        return a
    }

    // MARK: - Geometry

    private static func geometry(
        _ meshPtr: UnsafeMutablePointer<aiMesh>, materials: [SCNMaterial]
    ) -> SCNGeometry {
        let mesh = meshPtr.pointee
        let count = Int(mesh.mNumVertices)

        var sources: [SCNGeometrySource] = [
            source(from: mesh.mVertices, count: count, semantic: .vertex)
        ]
        if let normals = mesh.mNormals {
            sources.append(source(from: normals, count: count, semantic: .normal))
        }
        // Channel 0 only. Assimp carries eight UV sets and SceneKit's material
        // model reads one; a second set with nothing bound to it is bytes.
        if let uv = mesh.mTextureCoords.0 {
            // Still aiVector3D — the third component is unused for a 2D set,
            // so the stride steps over it rather than the data being repacked.
            sources.append(
                SCNGeometrySource(
                    data: Data(bytes: uv, count: count * MemoryLayout<aiVector3D>.stride),
                    semantic: .texcoord,
                    vectorCount: count,
                    usesFloatComponents: true,
                    componentsPerVector: 2,
                    bytesPerComponent: MemoryLayout<Float>.size,
                    dataOffset: 0,
                    dataStride: MemoryLayout<aiVector3D>.stride
                )
            )
        }

        var indices: [UInt32] = []
        indices.reserveCapacity(Int(mesh.mNumFaces) * 3)
        for f in 0..<Int(mesh.mNumFaces) {
            let face = mesh.mFaces[f]
            // Triangulate ran, so this is three — but a degenerate face can
            // survive it, and a two-index "triangle" would read past its own
            // array.
            guard face.mNumIndices == 3 else { continue }
            for i in 0..<3 { indices.append(face.mIndices[Int(i)]) }
        }

        let element = SCNGeometryElement(
            data: Data(bytes: indices, count: indices.count * MemoryLayout<UInt32>.size),
            primitiveType: .triangles,
            primitiveCount: indices.count / 3,
            bytesPerIndex: MemoryLayout<UInt32>.size
        )

        let geometry = SCNGeometry(sources: sources, elements: [element])
        geometry.name = string(mesh.mName)
        let slot = Int(mesh.mMaterialIndex)
        if materials.indices.contains(slot) {
            geometry.materials = [materials[slot]]
        }
        return geometry
    }

    private static func source(
        from p: UnsafeMutablePointer<aiVector3D>, count: Int, semantic: SCNGeometrySource.Semantic
    ) -> SCNGeometrySource {
        SCNGeometrySource(
            data: Data(bytes: p, count: count * MemoryLayout<aiVector3D>.stride),
            semantic: semantic,
            vectorCount: count,
            usesFloatComponents: true,
            componentsPerVector: 3,
            bytesPerComponent: MemoryLayout<Float>.size,
            dataOffset: 0,
            dataStride: MemoryLayout<aiVector3D>.stride
        )
    }

    // MARK: - Materials

    /// Colour and name only.
    ///
    /// Textures are deliberately not resolved. An FBX references them by paths
    /// that were valid on the machine that exported it, and chasing those would
    /// mean reading files from wherever the string happens to point — a viewer
    /// opening arbitrary paths named inside a document it was handed. What the
    /// viewer is for is the shape, and the shape reads from lighting.
    private static func material(_ ptr: UnsafeMutablePointer<aiMaterial>?) -> SCNMaterial {
        let m = SCNMaterial()
        m.lightingModel = .physicallyBased
        m.isDoubleSided = true
        guard let ptr else { return m }

        // The key spelled out. Assimp's `AI_MATKEY_*` are multi-token macros —
        // `AI_MATKEY_NAME` is `"?mat.name", 0, 0` — so they do not import into
        // Swift at all, and the three arguments are written here instead.
        var name = aiString()
        if aiGetMaterialString(ptr, "?mat.name", 0, 0, &name) == aiReturn_SUCCESS {
            m.name = string(name)
        }
        var colour = aiColor4D()
        if aiGetMaterialColor(ptr, "$clr.diffuse", 0, 0, &colour) == aiReturn_SUCCESS {
            m.diffuse.contents = NSColor(
                srgbRed: CGFloat(colour.r), green: CGFloat(colour.g),
                blue: CGFloat(colour.b), alpha: CGFloat(colour.a)
            )
        }
        return m
    }

    // MARK: - Node tree

    private static func node(
        _ ptr: UnsafeMutablePointer<aiNode>, geometries: [SCNGeometry],
        placed: inout [(node: SCNNode, mesh: Int)]
    ) -> SCNNode {
        let a = ptr.pointee
        let out = SCNNode()
        out.name = string(a.mName)
        out.transform = transform(a.mTransformation)

        // A node with several meshes becomes a node with several children:
        // SCNNode holds one geometry, and merging them would lose the
        // material boundary that made them separate meshes.
        let ids = (0..<Int(a.mNumMeshes)).map { Int(a.mMeshes[$0]) }
        if ids.count == 1, geometries.indices.contains(ids[0]) {
            out.geometry = geometries[ids[0]]
            placed.append((out, ids[0]))
        } else {
            for id in ids where geometries.indices.contains(id) {
                let child = SCNNode(geometry: geometries[id])
                child.name = geometries[id].name
                out.addChildNode(child)
                placed.append((child, id))
            }
        }
        for c in 0..<Int(a.mNumChildren) {
            out.addChildNode(node(a.mChildren[c]!, geometries: geometries, placed: &placed))
        }
        return out
    }

    /// Assimp's matrix is row-major; SceneKit's is column-major. The
    /// transposition IS the conversion — copied straight across, every model
    /// arrives with its rotation and translation swapped into each other.
    private static func transform(_ m: aiMatrix4x4) -> SCNMatrix4 {
        SCNMatrix4(
            m11: CGFloat(m.a1), m12: CGFloat(m.b1), m13: CGFloat(m.c1), m14: CGFloat(m.d1),
            m21: CGFloat(m.a2), m22: CGFloat(m.b2), m23: CGFloat(m.c2), m24: CGFloat(m.d2),
            m31: CGFloat(m.a3), m32: CGFloat(m.b3), m33: CGFloat(m.c3), m34: CGFloat(m.d3),
            m41: CGFloat(m.a4), m42: CGFloat(m.b4), m43: CGFloat(m.c4), m44: CGFloat(m.d4)
        )
    }

    /// `aiString` is a length and a fixed 1024-byte buffer, which Swift imports
    /// as a tuple of that many `CChar`s. Read through its own storage rather
    /// than by naming members.
    static func string(_ s: aiString) -> String {
        var value = s
        return withUnsafeBytes(of: &value.data) { raw in
            let bytes = raw.bindMemory(to: UInt8.self)
            let n = min(Int(value.length), bytes.count)
            return String(decoding: bytes[0..<n], as: UTF8.self)
        }
    }

    /// Assimp's `aiMatrix4x4` as SceneKit's, for callers outside this type.
    static func matrix(_ m: aiMatrix4x4) -> SCNMatrix4 { transform(m) }
}

// MARK: - One mesh's skin weights

/// Assimp stores skinning the way a modeller thinks about it — a bone, and the
/// vertices it pulls — and SceneKit wants it the way a GPU does: for each
/// vertex, which bones and how much. This is the transposition, done once at
/// import.
struct FBXSkin {
    /// Influences per vertex. Four is what `aiProcess_LimitBoneWeights` trims
    /// to by default and what every real-time skinning path uses.
    static let influences = 4

    let boneNames: [String]
    let offsets: [SCNMatrix4]
    let vertexCount: Int
    /// `Float32 × 4` per vertex, summing to one wherever the vertex is bound.
    let weights: Data
    /// `UInt16 × 4` per vertex, indexing `boneNames`.
    let indices: Data

    init?(_ meshPtr: UnsafeMutablePointer<aiMesh>) {
        let mesh = meshPtr.pointee
        let boneCount = Int(mesh.mNumBones)
        guard boneCount > 0, mesh.mBones != nil else { return nil }
        // `boneIndices` is 16-bit here, so a rig past this could only be
        // written by truncating bone numbers into each other. Refusing is the
        // honest answer; nothing in a game asset comes close.
        guard boneCount <= Int(UInt16.max) else { return nil }

        let count = Int(mesh.mNumVertices)
        vertexCount = count

        var names: [String] = []
        var binds: [SCNMatrix4] = []
        names.reserveCapacity(boneCount)
        binds.reserveCapacity(boneCount)

        var w = [Float32](repeating: 0, count: count * Self.influences)
        var idx = [UInt16](repeating: 0, count: count * Self.influences)
        // How many slots of each vertex are already spoken for.
        var filled = [UInt8](repeating: 0, count: count)

        for b in 0..<boneCount {
            guard let bonePtr = mesh.mBones[b] else { return nil }
            let bone = bonePtr.pointee
            names.append(FBXScene.string(bone.mName))
            // The inverse bind: the transform that takes a vertex from mesh
            // space into this bone's space at the pose the mesh was authored
            // in. SceneKit calls it `boneInverseBindTransforms` and Assimp
            // calls it the offset matrix; they are the same matrix.
            binds.append(FBXScene.matrix(bone.mOffsetMatrix))

            for k in 0..<Int(bone.mNumWeights) {
                let entry = bone.mWeights[k]
                let v = Int(entry.mVertexId)
                guard v < count else { continue }
                let slot = Int(filled[v])
                // LimitBoneWeights ran, so this should not trip. If a file
                // slips past it, the extra influence is dropped rather than
                // written over somebody else's vertex.
                guard slot < Self.influences else { continue }
                w[v * Self.influences + slot] = entry.mWeight
                idx[v * Self.influences + slot] = UInt16(b)
                filled[v] = UInt8(slot + 1)
            }
        }

        // A vertex nothing claimed would collapse to the origin under
        // skinning, because a zero weight set means "no bone" to the shader
        // and mesh-space zero to the maths. Bind it rigidly to bone 0 instead,
        // which leaves it exactly where the file put it.
        for v in 0..<count where filled[v] == 0 {
            w[v * Self.influences] = 1
            idx[v * Self.influences] = 0
        }

        boneNames = names
        offsets = binds
        weights = w.withUnsafeBufferPointer { Data(buffer: $0) }
        indices = idx.withUnsafeBufferPointer { Data(buffer: $0) }
    }
}
