# Suisei (彗星)

A native macOS code editor. Apple Silicon, macOS 26 and later.

A SwiftUI/AppKit face over a Rust engine. All editor logic — buffers,
selections, syntax, LSP, the debugger, git — lives in `suisei-core`, which is
headless and knows nothing about the screen. The face reaches it through a C
ABI in `suisei-engine` and draws what it is told.

```
suisei-core     the editor, headless and testable
suisei-engine   the C ABI the face calls, and the scene it builds
suisei-app      the Swift face: canvas, chrome, settings, viewers
suisei-daemon   background indexing
suisei-agent    the menu-bar item
```

## Building

Needs Rust and Xcode's Swift toolchain.

```sh
./scripts/release.sh              # → dist/Suisei-<version>.dmg
SUISEI_FAST=1 ./scripts/package-suisei-app.sh   # quick, unoptimised
cargo test -p suisei-core -p suisei-engine
```

## First launch

Suisei is not signed with an Apple Developer ID, so macOS refuses the first
launch and says it cannot be verified. That message means Apple has not been
paid to vouch for the build, not that anything is wrong with it. Open **System
Settings → Privacy & Security**, find the line about Suisei, and click **Open
Anyway**. Once, ever.

(From macOS Sequoia on, right-click → Open no longer works for this. The
Privacy & Security panel is the only route.)

## Updating

Because it is unsigned, a downloaded update would ask that question again every
time. So Suisei updates by **building the tagged release on your own machine** —
a locally built app is never quarantined, so macOS never asks. Settings →
Software Update clones the tag, checks it is the commit it was offered, builds
it, and exchanges the bundle at the next launch. The exchange is one atomic
rename: there is no instant where you have no editor, and the previous build
stays on disk as the way back.

It needs Rust and Swift installed, and it takes a while. Everything that can go
wrong leaves the app you already have exactly as it was.

## Licence

[Suisei Source License](LICENSE) — source-available, **not** open source. Free
for personal and non-commercial use; commercial use is a separate agreement.
See [EULA](EULA) and [CONTRIBUTING.md](CONTRIBUTING.md).
