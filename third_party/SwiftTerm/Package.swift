// swift-tools-version:6.0
import PackageDescription

// Suisei's own manifest for a VENDORED SwiftTerm — not upstream's.
//
// Upstream's builds three executables and a fuzzer, pulls swift-argument-parser
// and swift-docc-plugin over the network, and generates its version constants
// with a build-tool plugin. None of that is wanted here: the app needs one
// static library, and a build that reaches the network is a build that can fail
// for reasons that have nothing to do with this repository.
//
// So the dependencies and the executables are gone, and the plugin's output —
// four version constants — is checked in as `SwiftTermBuildInfo.swift`,
// recording the commit this was taken from. See VENDOR.md.
let package = Package(
    name: "SwiftTerm",
    platforms: [.macOS(.v13)],
    products: [.library(name: "SwiftTerm", targets: ["SwiftTerm"])],
    targets: [
        .target(
            name: "SwiftTerm",
            path: "Sources/SwiftTerm",
            exclude: ["Mac/README.md"],
            resources: [.process("Apple/Metal/Shaders.metal")]
        )
    ],
    swiftLanguageModes: [.v5]
)
