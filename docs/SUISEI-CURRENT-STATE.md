# Suisei current state and independence roadmap

> **Historical baseline, code-verified 2026-07-23.** Several ordered patches in
> this document have since landed: modeless plural selections, native editing
> commands, a shadow WAL, central `Edit`/`Delta`, incremental LSP sync, an
> asynchronous syntax worker, stable document ids and a pixel-based core
> viewport. Use the dated current snapshot at the top of `SUISEI-TODO.md` for
> open status; retain this document for rationale and acceptance criteria.

This document turns the existing Swift face and Rust engine into a prioritised
path to an independently usable macOS IDE. It does not redefine the long-term
product vision in `SUISEI-PLAN.md`.

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
   - DONE `package-suisei-app.sh` dependency detection now scans
     `suisei-engine` **and** `suisei-core`, so a core-only edit can no longer
     package a stale dylib.
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
   *(Done 2026-07-24. Editing is fully modeless and multi-cursor carets now
   paint; only secondary-selection *fills* are deferred, pending ⌘-D.)*
   - DONE `Selection { anchor, head, goal_x }` + `SelectionSet` (`selection.rs`),
     exclusive, plural; `App.sel` is the single selection authority and
     `selected_range()` derives the legacy inclusive span from it.
   - DONE semantic edits on `app.sel` (`gui_edit.rs`): caret_move/extend,
     caret_place/drag/add, select_word/all, and gui_insert_text /
     gui_insert_newline / gui_delete_backward/forward — type-over-selection and
     multi-cursor edits, mode-independent. Undo coalesces a typing/delete run
     into one group (`App.edit_run`).
   - DONE the engine dispatch routes characters, backspace, delete, enter and
     the arrows straight to those commands; the synthetic `i`/`c`/`d` and the
     `mode_is_insert` fast-path gate are gone (`editor_accepts_text`). Mouse and
     Shift/Alt-arrow selection, drag, ⌘-click, and Esc→collapse are wired
     through Swift. vim keyboard commands (`:` `/` space-leader `i` hjkl) no
     longer exist in the GUI — a bare key just types. Core vim machinery stays
     as dormant dead code, deletable incrementally.
   - DONE multi-cursor carets paint. `App::secondary_caret_positions()` feeds
     every non-primary caret through the existing kind-250 span channel carrying
     UTF-16 offsets (like the bracket hint, kind 254), and the face positions it
     with CoreText and the primary caret’s cap-height geometry. No ABI change:
     the hard-coded `SuiseiEditorLineC` decoders (the offset trap) are untouched,
     avoiding a multi-span layout rewrite. Regression tests in `suisei-core`
     (`secondary_caret_positions`) and `suisei-engine` (compose asserts a
     kind-250 span at the right UTF-16 offset, incl. a CJK line).
   - REMAINING (small): secondary-selection *fills* (a distinct span kind + a
     pre-text draw pass) — dead until ⌘-D/add-next-occurrence creates non-empty
     secondary selections, so folded into that feature.
   - DONE 2026-07-25 — **the vim spine is out of the key path.** `Mode` was
     conflating vim editing states with GUI panel focus; it is now pure focus
     (`Mode::Editor` + the panels). Deleted: every vim key handler
     (`handle_normal`/`leader`/`operator_pending`/`pending`/`insert`/`visual`),
     macro record/playback, the `:` ex-command interpreter (`execute_xlc`) and
     the TUI-only surfaces (screensaver/bench/plugin-store/webview/ext-panel/
     rebase/PR-review). `dispatch.rs` 3,993 → 2,268 lines; `app.rs` 6,489 →
     ~5,700. The engine now *seals* the editor: a key the Selection-model
     tables decline is dropped, never handed to a command interpreter.
     The face stopped switching on a vim status badge — the engine sends the
     focus and Swift parses it once into a typed `Focus`.
     The second pass finished the job: the vim state fields
     (`visual_anchor`, `pending_*`, `count`, `marks`, `last_find`, `macros`,
     `which_key`) are gone from `App`, and `which_key.rs` / `macros.rs` /
     `substitute.rs` / `xlc.rs` / `ops.rs` are deleted. `nav.rs` kept only the
     jumplist (Back/Forward); `registers.rs` shrank to a clipboard-backed yank
     store; the one text-object the GUI needs is a local `word_range_at` in
     `gui_edit.rs`. The which-key and XLC FFI structs, their C decls and the
     Swift snapshots went with them, as did the `mode_normal/insert/visual`
     theme colours. Total: −5,600 lines.
   - FIXED 2026-07-25: the modeless edit path was gated on `explorer.open`,
     which in the GUI means "the docked navigator has entries", not "the
     explorer owns the keys". Opening any project therefore sent every
     keystroke to the vim command machine — typing did nothing and bare keys
     ran vim commands. Key routing now keys off `Mode` alone.
   - Gate: Shift+arrow, typing-over-selection, IME composition, drag, and
     ⌘-click multi-cursor caret paint ✅ without a mode-specific Swift workaround.

3. **Make unsaved work durable before expanding the feature surface.**
   *(D0 done 2026-07-24 — the crash-safety gate below is met. D1 daemon is a
   separate, larger step, deferred pending an explicit decision.)*
   - DONE Architecture Plan D0 (shadow WAL): `suisei-engine/src/journal.rs`
     flushes the dirty buffer on the tick loop under a 250 ms / 4 KiB policy;
     both the WAL and the file save use `fs_atomic::atomic_write_file`
     (tmp → fsync → rename). Recovery scan/count/path/accept/discard cross the
     C ABI (`recovery_ffi.rs`, 9 tests) and the face shows a recovery sheet on
     startup. Accept restores text, cursor **and** scroll (clamped). The journal
     lives in Suisei's own `~/.suisei/journal`, not `~/.xei`.
   - REMAINING D1 (daemon): move authoritative `App` ownership behind a local
     Unix-socket daemon so GUI crashes cannot lose in-flight state at all. Keep
     a protocol version in every message; do not reuse silent fixed-offset
     decoding across processes without a version handshake. Per the Architecture
     Plan, D0 alone already solves crash loss, so D1 is an isolation/multiwindow
     investment, not a prerequisite for this gate.
   - Gate: force-killing the GUI restores unsaved text, cursor and scroll (✅ via
     D0); killing during save leaves the original file valid (✅ atomic save).

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
   - DONE (2026-07-25) the GUI now *pumps* the language services at all. The
     engine tick never called `LspClient::poll`, and that drain is what parses
     the `initialize` reply and then sends `initialized` + `didOpen` — so every
     spawned rust-analyzer stayed un-handshaked and idle, `server_running` was
     permanently false, and every LSP-backed surface (references, hover,
     definition, rename, format, diagnostics) resolved empty. The xei TUI drove
     this from its main loop; the GUI never had an equivalent.
     `App::poll_language_services` (`pump.rs`) is that equivalent, plus the
     throttled post-edit `didChange` the GUI edit path never sent.
   - DONE (2026-07-26) the daemon reports **real** state. Its `DaemonState`
     setters had no production caller, so the menu-bar agent drew
     `LSP none · DAP none · Project none` in every session — and nothing else
     could fill them, because the daemon owns no language server (that is D1).
     A `ReportStatus` opcode (client → daemon, same fixed `Status` layout, no
     reply) is the missing writer; the engine tick builds the snapshot from
     live `App` state and a background thread pushes it, never blocking the
     tick. Reported fields expire after 12 s so a killed editor stops claiming
     a live server. "Indexing" was separately unobservable: the client never
     advertised `window.workDoneProgress`, so rust-analyzer sent no
     `$/progress` at all — now it does, and `LspClient::is_busy()` brackets the
     window.
   - Finish Go to Definition, hover, code actions, rename, format, diagnostics
     navigation, workspace search/replace, and semantic-token presentation —
     now against a language server that actually answers.
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
