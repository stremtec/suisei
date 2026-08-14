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

## What Suisei uses

`suisei-app/Suisei/TerminalSurface.swift` is the whole of it. Terminal *panes*
run on `LocalProcessTerminalView`; the docked shell (⌃T) still runs on core's
own emulator in `suisei-core/src/term.rs`, so the two can be compared side by
side before the old one goes.

Two upstream declarations the port leans on, worth checking on an update
because a change to either is a compile error here and not obviously about us:

* `TerminalView.hasFocus` is `open`, which is the only hook for "the terminal
  took the keyboard" — `becomeFirstResponder` next to it is `public override`
  and cannot be overridden outside the module. See `PaneTerminalView`.
* `LocalProcessTerminalView.startProcess(executable:args:environment:execName:
  currentDirectory:)` — `execName` is what makes the shell a *login* shell,
  without which an app launched from Finder hands its child no real `PATH`.

Also relevant: SwiftTerm exports a type named `Color` (a 16-bit-per-channel
terminal colour). Any file importing both SwiftTerm and SwiftUI must qualify.

## Updating

Re-clone upstream at the new commit, copy `Sources/SwiftTerm` and `LICENSE` over,
regenerate `SwiftTermBuildInfo.swift` (or hand-edit the four constants), and
update the commit above.
