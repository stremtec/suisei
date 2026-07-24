# Suisei current state and independence roadmap

**Code-verified: 2026-07-23.** This document turns the existing Swift face and
Rust engine into a prioritised path to an independently usable macOS IDE. It
does not redefine the long-term product vision in `SUISEI-PLAN.md`.

## What “independent GUI IDE” means

Suisei is independent when a user can install and use the `.app` without the
xei TUI, recover after GUI failure, edit Unicode text correctly, work in a
project, and use core IDE features through native surfaces. It does **not**
require Metal, canvas mode, dockable panels, or full VS Code extension parity.

Today the runtime is already independent of the TUI: Swift only calls the
`suisei-engine` C ABI and the engine depends on `suisei-core`. However,
`suisei-core` is a fork rather than a shared crate, and all unsaved state still
lives inside the GUI process.

```text
today:     Suisei.app (Swift face + Rust Engine + unsaved App state)
target:    Suisei.app (native client) ←IPC→ suisei-daemon (durable App state)
                                                   ↓
                                             WAL / files / recovery
```

## Ordered patches

### P0 — correctness and release gates

1. **Make the current build and tests release-reliable.**
   - Fix `package-suisei-app.sh` dependency detection: it watches `xei-core`
     but omits `suisei-core`, so a core-only edit can package a stale dylib.
   - Fix `ProjectIndex` Swift-concurrency warnings before Swift 6 promotes them
     to errors: do filesystem enumeration off actor without actor-isolated
     static reads, and replace the async `FileManager.DirectoryEnumerator`
     iteration with a Sendable-safe producer.
   - Add a normal-environment CI lane for the undo spill/cache and DAP
     localhost tests; the restricted sandbox cannot validate them.
   - Add an ABI layout/version test covering the Rust struct, C header, and
     Swift decoder. FFI field changes currently have three coupled consumers.
   - Gate: release build, signed `.app` launch smoke test, engine tests, core
     tests, and an open/edit/save/reopen smoke test pass on macOS.

2. **Replace the GUI’s Vim-mode emulation with first-class editing commands.**
   - Keep the Core command state machine, but expose semantic commands for
     insert, move, delete, select, marked text, and pointer selection.
   - Remove the face’s synthetic `i` / `Esc` policy from normal typing and
     mouse flows. It is a compatibility layer, not a sound document model.
   - Introduce `Selection { anchor, head, goal_x }` and plural selections;
     map shift-arrows, word/line selection, option-drag and command-click to
     it. A caret is an empty selection.
   - Gate: Shift+arrow, typing-over-selection, IME composition, drag, and
     multi-cursor work without a mode-specific Swift workaround.

3. **Make unsaved work durable before expanding the feature surface.**
   - Implement Architecture Plan D0 first: append edit deltas to a shadow WAL
     with a 250 ms / 4 KiB fsync policy, periodic snapshots, and recovery UI.
   - Then move authoritative `App` ownership behind a local Unix-socket daemon
     (D1). Keep a protocol version in every message; do not reuse silent
     fixed-offset decoding across processes without a version handshake.
   - Gate: force-killing the GUI restores unsaved text, cursor and scroll;
     killing during save leaves the original file valid.

### P1 — latency and IDE-grade daily work

4. **Introduce document deltas and remove synchronous whole-file work.**
   - First add a central `Edit`/`Delta` and versioned immutable snapshot API;
     consumers must stop independently rebuilding full text.
   - Move parse/highlight/LSP/index consumers to a background, cancellable
     pipeline which publishes only results for the current document version.
   - Replace `Vec<String>` with a rope/piece tree plus line index only after
     the delta boundary is tested. Do not combine storage rewrite, selection
     rewrite and daemon migration in one patch.
   - Gate: 6,000-line typing remains below the frame budget with parser/index
     activity; stale results are rejected by version.

5. **Finish the native editor viewport.**
   - Make the scroll model visual-row/pixel based so soft-wrapped lines,
     selection geometry, hit testing and minimap share one mapping.
   - Preserve exact-range CoreText pull rendering; it is the right current
     performance shape. Metal is optional later, not a prerequisite.
   - Complete fold and breakpoint gutter semantics and semantic-token paint.
   - Gate: wrapped long lines scroll and select correctly, including CJK and
     tabs, with no cursor jump or duplicate row identity.

6. **Turn existing Core capability into complete IDE workflows.**
   - Finish Go to Definition, hover, code actions, rename, format, diagnostics
     navigation, workspace search/replace, and semantic-token presentation.
   - Ship an intentional DAP surface: launch/attach, breakpoint lifecycle,
     stack/scopes/variables, debug console and stop/continue. A snapshot alone
     is not a debugger workflow.
   - Harden project root handling and show/cancel index progress; keep all
     filesystem/git work off the main actor.
   - Gate: a Rust/Swift project can be opened, searched, diagnosed, navigated,
     formatted and debugged entirely from Suisei.

### P2 — product completeness after the daily editor is trustworthy

7. **Terminal and shell polish.**
   - Preserve PTY focus deterministically across navigator, split and window
     changes; complete ANSI attributes/links/selection and terminal resize
     tests. Avoid more global key monitors for local focus problems.

8. **Extension host and webview islands.**
   - Connect `xei-ext-host` to a scoped Suisei extension surface, then provide
     a sandboxed `WKWebView` island. Keep extension rendering out of editor
     typing and document ownership paths.

9. **Workspace ergonomics.**
   - Persist project/layout state, then add dock/tear-off panels using a
     serialised Swift layout tree. Multiwindow needs daemon-backed shared state
     first; do not duplicate an `EngineBridge` and hope state stays coherent.

10. **Optional visual bets.**
    - Infinite canvas, Metal glyph backend and elaborate glass transitions are
      P3 experiments. They must not delay correctness, recovery, latency or
      native IDE workflows.

## Explicit non-priorities

- Rewriting the editor in Swift/`NSTextStorage` or `TextEditor`.
- Moving render layout details into Core.
- A large FFI redesign without ABI versioning/tests.
- Treating existing xei surfaces as “done” merely because Core has their data.

## Delivery discipline

Each patch must name its Core state owner, FFI/API contract, Swift surface,
versioning/migration plan, and a regression test. Update `SUISEI-GAP.md` only
after that end-to-end path works.
