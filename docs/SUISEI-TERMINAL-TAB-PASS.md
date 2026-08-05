# Pane terminals and the tab strip: audit, fixes, and what stays open

Code-verified 2026-07-29, on branch `devim-and-theme`. A full-stack pass over
the pane terminal feature (core `term.rs` / `app.rs`, engine `runtime.rs` /
`ffi.rs`, face `ContentView.swift` / `EngineBridge.swift`) and the tab strip's
layout-group machinery. Every fix below is regression-tested where the layer
is testable; the face changes compile and package, and their motion needs a
human at the screen.

## Fixed — pane terminals

### Critical
1. **Pane PTYs were never resized.** `terminal_resize` only ever touched the
   dock; pane shells spawned at a viewport guess and kept it forever — output
   wrapped at the wrong column, divider drags and window resizes never
   reflowed, vim/htop drew garbled. Worse, every pane's face measurement was
   reported INTO the dock's PTY. Now: `suisei_engine_terminal_resize_pane` +
   `reportTerminalCells(_:pane:)` routes each pane's geometry to its own
   shell (`runtime.rs`, `ContentView.swift`). Tests: `pane_terminal_resize_reaches_the_pane_pty`.
2. **Esc never reached a pane shell.** The face gated its "Esc leaves the
   terminal" branch on `chrome.terminal.fullPanel` — a field of the DOCK
   snapshot, always false for panes — so Esc died in a no-op and vim/less/nano
   were unusable in panes. The branch now fires only for the dock
   (`chrome.terminal.open && focus == .terminal`); panes keep Esc for TUIs.
3. **⌘V pasted into the dock (or nowhere).** `paste_clipboard_to_terminal`
   targeted `app.terminal` unconditionally; with the dock closed the paste
   vanished. Both the key path and the engine's `paste_text` (IME commits,
   drops) now resolve the focused pane shell first, then the dock, then the
   editor.
4. **Close-confirm state was shared with the dock.** One `close_confirm` flag
   on the dock `Terminal` gated every pane shell: pane B answered pane A's
   dialog, B's keys blackholed while A's prompt was up, `y` killed whichever
   shell was focused at confirm time, and a latch surviving ⌘W killed the
   NEXT terminal on a stale `y`. Now per-shell: `App.pane_close_confirm:
   Option<TerminalId>`, validated at confirm, cleared on every close path.
   Tests: `close_confirm_belongs_to_the_pane_that_asked`,
   `closing_a_terminal_tab_clears_its_close_confirm`.

### High
5. **Backgrounds never painted.** `TermCanvas` built attributed strings with
   foreground only — reverse-video and coloured-bg cells (vim status lines,
   man headers, selections) drew black-on-black. The canvas now fills per-run
   background rects measured off the CTLine; the parser's existing bg
   (dimmed 0.35/0.45) is what it paints.
6. **Row pitch disagreed (16 vs 17).** The face reported 16pt rows but painted
   at 17 (`fontSize + 5`), so the PTY got one more row than the grid could
   show — a full screen lost its top row to stick-to-bottom. One constant
   (17) in `reportTerminalCells`.
7. **The pane header ✕ hit the focused pane.** It called `toggleTerminalFull`
   (which acts on the focused pane) without focusing its own — clicking B's ✕
   killed A's shell, or converted A into a terminal. The button now focuses
   its pane first.
8. *(folded into 3 — IME/drop paste.)*
9. **No wheel scrollback in panes.** Pane grids were built without an
   `onScrollback`; the wheel only moved the (desynced, see 6) canvas. Now
   `suisei_engine_terminal_scroll_pane` + wiring; PageUp still works too.
   Test: `pane_terminal_scroll_moves_only_that_pane`.

### Medium
11. **DSR/CPR and DA1 went unanswered.** Probing tools (tmux startup, `tput
    u7`, zsh widgets) stalled on a silent terminal. `CSI 6 n` now replies
    `CSI row;col R`; `CSI c` answers DA1 (`term.rs`). Replies use a new
    `write_raw` that does not snap a scrolled-back view to the prompt.
12. **Output floods parsed unbounded.** `yes | head -100000` drained and
    parsed every queued chunk in one tick, stalling the face's main thread.
    `poll` now caps at 256 KiB per tick; the rest stays queued and the damage
    flag keeps the next tick draining.
14. **Dense rows truncated mid-escape.** Truecolor output spends ~19 bytes per
    colour change and outran the 1536-byte ABI cap, which then cut mid-escape
    or mid-UTF-8 into garbage. The encoder stops at a 1400-byte budget on a
    char boundary; the face parser is per-row, so an unfinished sequence is
    ignored rather than misparsed. Test: `dense_rows_stay_under_the_abi_cap`.
15. **Bold died with the parser run.** Core tracked SGR 1 but cells dropped
    it; `ls --color` and git diffs rendered flat. Cells now carry `bold`,
    `visible_rows_sgr` emits `1`/`22`, and the face draws bold runs with the
    semibold face. Test: `bold_reaches_the_sgr_encoding`.
17. **Caret drawn over scrollback.** Snapshots always sent the live cursor
    while the viewport showed history — a block caret sat on the wrong row of
    old output. `terminal_for_pane` sends `u16::MAX` while `scroll() > 0` and
    the canvas suppresses it.
19. **Dock open stole keys while face indicators disagreed.** Opening the
    dock forces `Mode::Terminal` (keys → dock) but the face checked the pane
    first and insisted the pane owned them. `terminalOwnsKeys` now checks the
    dock first; Ctrl+Tab yields to a focused pane shell.
21. **Pane headers always said "click to type".** The hint derived from the
    dock's mode; it now reflects whether that pane actually owns the keyboard.
24. **Failed PTY spawn left a permanent dead tab.** `start()` failed silently
    and the tab + map entry were inserted anyway — "starting…" forever.
    `toggle_terminal_full` now rolls back and reports.
- **Snapshot churn.** Every keystroke re-pulled ~300 KiB per idle terminal
  pane (`visible_rows_sgr` + decode). Panes now ship a wrapping `term_gen`
  (reused pad bytes at offset 18 — ABI stride unchanged, tripwire updated)
  and the face reuses its grid when the generation and tab index are
  unchanged.

## Fixed — tab strip and layout groups

- **Slot ≠ index addressing.** Strip slots stop being buffer indices the
  moment a folded layout gathers members (grouped) or hides them (unified):
  every slot after the group named a DIFFERENT document than the same buffer
  index — clicks, closes, reorders and ⌃⇥ cycling all hit the wrong tab, and
  the unified chip's ✕ closed the rightmost document (the slot clamped). The
  face now addresses chips by `stableId` through new id-based entries
  (`goto_tab_id`, `close_tab_id`, `move_tab_ids`, `drop_layout`); the engine
  translates. Tests: `id_addressed_tab_ops_hit_the_named_tab`,
  `id_addressed_tab_ops_survive_a_folded_group`.
- **Zombie layouts.** Nothing dissolved a layout whose documents were closed;
  invisible in grouped style, unkillable in unified. Layouts below two
  documents are now dropped on membership change, and "Close Tab" on a layout
  chip has a real verb (`drop_layout` — documents stay open). Tests:
  `a_layout_dissolves_when_it_drops_below_two_documents`,
  `dropping_a_layout_keeps_its_documents`.
- **Group container defects.** +2pt rightward shift (origin measured after
  the padding its backgrounds ignore — measurement moved before it); radius
  10→12 to match the 24pt chip capsules; margins 7/7→8/8; the active capsule
  now draws OVER the container (it was behind, so an active member just
  deepened the band); `tabFrames` pruned on chip removal (stale slots leaked
  into hit-testing and the container span); lone-member containers filtered
  in the face as well as core; auto-scroll-to-active targeted the slot index
  while chips are `.id(stableId)` — it never matched.
- **Animations (new).** Fold: the container grows in (`.snappy(0.22)`,
  scale 0.94 → 1 + opacity), keyed on the measured runs because a fold flips
  `group` without changing a single stableId. Unfold: asymmetric removal
  (scale 0.97 out). Merge (grouped ⇄ unified): container and unified chip
  share one `matchedGeometryEffect` id (the layout's, in a NEW namespace —
  `tabPillSpace` would collide with the active capsule's ids), so the style
  toggle folds the container into the chip in place under the strip's
  existing `.smooth(0.28)` transaction; the unified chip emits at its first
  member's strip position (`build_tabs`) so nothing flies across the strip.
  The unified chip gets its own resting affordance (`square.on.square` glyph,
  grey capsule) — the shape the morph travels to.
- **Unified chip placement.** `build_tabs` emitted unified chips at the
  strip's END; they now sit at the first member's position (the merge morph's
  precondition). Test: `unified_chip_sits_at_its_first_members_position`.

## Open — listed, not implemented

Terminal emulator gaps (each is a feature, not a bug in existing behavior):
- **Scroll regions** — `CSI r` (DECSTBM) and `ESC M/D/E` index escapes are
  no-ops; tmux/screen and partial-screen scrollers smear (`term.rs:703`).
- **IME in terminal panes** — `TermCanvas` is a plain `NSView`, not an
  `NSTextInputClient`; Hangul/Japanese composition into a shell arrives as
  raw Latin. The editor canvas shows the pattern to copy.
- **Mouse reporting** — `wants_mouse()` never crosses the ABI; vim/htop wheel
  and click are dead in dock and panes alike.
- **OSC 0/2 titles** — swallowed, so terminal tabs are all named "Terminal".
- **Confirm on ⌘W/✕** — a running shell dies without the dialog the ⌃⇧W
  prompt promises; route the other close paths through
  `request_close_pane_terminal` when the child has a foreground job.
- **No `Drop` on `Terminal`** — orphan cleanup relies on kernel SIGHUP.

Tab strip, known small rough edges:
- `TabItem.isTerminal` is decoded in three places and read in none.
- The chip's `action:` duplicates the overlay's click routing; dead for mice,
  live for accessibility — the two copies must change together.
- `Int(truncatingIfNeeded: stableId)` for matchedGeometry ids is safe only
  while ids stay small (one monotonic counter).
- Fold is a silent no-op on a single pane; nothing surfaces the refusal.
- The 0.6s flick debounce swallows a second deliberate flick.

Fixed in a follow-up sweep the same day:
- **⌃⇥ stalled on unified chips** — cycling targeted the chip's id, which is
  a layout id with no buffer; `nextTab`/`prevTab` now activate the layout
  when the target chip `isLayout`.
- **`colorsEqual` compared 14 of 21 fields** — recoloring only
  macroName/namespace/parameter/property/constant/operator/punctuation left
  stale glyphs until something else bumped the cache. `Colors` is now
  `Equatable` and the comparison is `==`, so it cannot drift again.
- **`fullPanel`/`paneBound` vestige deleted** — the single-shared-terminal
  model's flags were hardcoded false in the dock snapshot, so every branch
  reading them (five in `ContentView`, two in `toggleTerminalFull`,
  `TerminalSnap.isBoundToPane`) was dead. Gone from the face; the ABI fields
  stay for stride.
- **Dock/pane separation pinned** — `dock_resize_and_scroll_leave_pane_terminals_alone`
  asserts dock controls never touch a pane shell's grid or scroll.

Wider editor debts observed in passing (pre-existing, out of this pass's
scope — see `SUISEI-CURRENT-STATE.md` P1 for the ordered versions):
- `ctCache`/`gutterCache` flush all-or-nothing at their caps; large-file
  scrolling thrashes them.
- The 256-line FFI budget is shared across panes — a later pane can receive
  zero lines; the 24-span cap can push caret/diagnostic markers out.
- `current_buffer` is still a position; `restore_session` has no callers
  (the session file is written, never read); no layout/tab state survives a
  restart (P2-9); `split_kind`/`split_ratio` are vestigial but still shipped.
- Tab decode is triplicated in `EngineBridge` (three verbatim loops).

## Verification

- `cargo test --workspace`: 289 core + 83 engine unit + 6 ABI + 9 recovery +
  7 journal + 1 daemon, all green. New tests pin every core/engine fix above.
- `SUISEI_FAST=1 ./scripts/package-suisei-app.sh`: packages clean (the
  face-only warnings predate this pass).
- **Needs a human at the screen:** the three group transitions (fold flick,
  unfold, style toggle morph), pane resize reflow with vim/htop, Esc in a
  pane TUI, ⌘V, wheel scrollback, and reverse-video paint — motion and PTY
  behavior are not assertable from here.
