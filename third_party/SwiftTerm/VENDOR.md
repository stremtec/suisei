# SwiftTerm — vendored

    upstream  https://github.com/migueldeicaza/SwiftTerm
    commit    105299618ef6de56974c9d502d4346c5a74b68ce
    date      2026-08-13
    licence   MIT — see LICENSE, which ships in the app bundle

## Why vendored rather than a package dependency

Suisei's app is compiled by a single `swiftc` invocation
(`scripts/package-suisei-app.sh`), not by SwiftPM, so there is no manifest for a
dependency to hang off. A pinned copy also means the build does not reach the
network and cannot change under a `swift build` on a different day.

## What was changed from upstream

Nothing in `Sources/SwiftTerm`. Only the manifest is ours:

* upstream's three executables, the fuzzer and the benchmarks are dropped, along
  with their `swift-argument-parser` / `swift-docc-plugin` dependencies — the app
  needs one library;
* the `SwiftTermBuildInfoPlugin` is dropped and its output, four version
  constants, is checked in as `Sources/SwiftTerm/SwiftTermBuildInfo.swift` with
  the commit above baked in;
* `Documentation.docc` removed.

## Updating

Re-clone upstream at the new commit, copy `Sources/SwiftTerm` and `LICENSE` over,
regenerate `SwiftTermBuildInfo.swift` (or hand-edit the four constants), and
update the commit above.
