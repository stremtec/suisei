# GLTFKit2 — vendored

    upstream  https://github.com/warrenm/GLTFKit2
    release   0.5.15
    artefact  GLTFKit2.xcframework.zip, macOS slice only
    sha256    9d0c338282acce4986494aa02a5f1495278f56c60d43f31453fefea6875b4928
              (the zip, matching the checksum in upstream's own Package.swift)
    licence   MIT — see LICENSE, which ships in the app bundle

## Why at all

macOS cannot read glTF. Measured rather than assumed: a minimal valid glTF 2.0
triangle, written out as both `.gltf` and `.glb`, is refused by `SCNScene(url:)`
with "could not be opened", and there is no glTF framework in
`/System/Library/Frameworks` — Model I/O, which SceneKit defers to, does not
know the format. glTF is the interchange format of the game-asset pipeline, so
a 3D viewer that cannot open a `.glb` has a hole in the middle of it.

The same triangle opens through `GLTFAsset` + `GLTFSCNSceneSource`, which hands
back an ordinary `SCNScene` — so this is a reader, not a second rendering path.
Everything downstream of the load is the SceneKit code that was already there.

## Why the built framework rather than the source

Upstream's own `Package.swift` distributes exactly this: a `binaryTarget`
pointing at this release's xcframework, pinned by the checksum above. Building
from source instead would pull in libktx (35 MB of CMake) and a C++
translation unit for meshopt, into a build that is one `swiftc` invocation and
has no CMake in it.

Only the macOS slice is kept. The full xcframework is 87 MB across iOS,
tvOS, visionOS, Catalyst and their simulators; the macOS framework is 5.5 MB
and carries both architectures.

## What was changed from upstream

Nothing. The framework is the published binary, and the licence is upstream's.
