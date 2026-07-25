# TUI residue — what to cut, and in what order

**Measured 2026-07-25**, after the vim removal. `suisei-core` is a fork of a
terminal editor, and vim was only the loudest part of that inheritance. This
document inventories the *rest* — structures designed for a TUI, implementations
that are wrong for a GUI, and things that simply have no reason to exist in a
native macOS IDE — and orders the removal.

Every number here came from the tree, not from memory. Where a claim is a
judgement rather than a measurement it says so.

## Why this matters more than it looks

Three bugs this week all came from the same root, not from three separate
mistakes:

- Typing did nothing and `z` ran a fold command — the editor lived in vim's
  *command* mode because `Mode` conflated editing state with panel focus.
- rust-analyzer spawned and never indexed — the GUI had no per-frame pump,
  because in the TUI the main loop was the pump.
- The app stayed light on a dark desktop — the theme came from a config file
  shared with the TUI, and nothing read the system appearance.

The pattern: **a TUI assumption that is invisible until it produces a symptom
that looks unrelated to it.** That is the argument for cutting them out rather
than living with them.

## Inventory

### 1. Key ownership is a global mode, not a responder chain — 1,556 lines

`App::Mode` is a single global "who owns the keyboard", and `dispatch.rs` has
one key handler per surface:

| handler | lines |
|---|---|
| `handle_git_workbench` | 681 |
| `handle_scm` | 137 |
| `handle_settings` | 129 |
| `handle_preview` | 110 |
| `handle_debug` | 101 |
| `handle_workspace_search` | 100 |
| `handle_search_input` | 66 |
| `handle_call_hierarchy` | 44 |
| `handle_terminal` / `handle_explorer` / `handle_palette` | 26 / 23 / 22 |
| `handle_pane_terminal_window` (PTY raw routing — **keep**) | 117 |
| **total** | **1,556 of 2,249** |

The panels are *already* native SwiftUI views; only their keyboard handling is
modal. A GUI IDE lets AppKit's responder chain decide, and the core never has a
"mode". Every panel open/close currently has to remember to reset the mode by
hand — and a missed reset is exactly how the typing bug happened.

> Landmine, from `ContentView.swift:360`: a previous attempt at native focus
> "double-captured input and stole focus". `SUISEI-ARCHITECTURE-PLAN.md` §2.5
> records the same trap. This must be solved deliberately in the first panel
> converted, not discovered in the last.

### 2. The theme carries 63 fields; the GUI reads 13

`build_theme` maps exactly these: `editor_bg fg line_no accent selection_bg
cursor status_bg keyword string comment number type_name function`.

The other 50 (`explorer_bg`, `terminal_bg`, `panel_*`, `xlc_*`, `mode_*`,
`git_*_bg`, …) described *TUI widget* surfaces. The GUI paints those with
SwiftUI materials and system colours, so they are carried, serialised and
ignored. `xlc_*` and `mode_xlc` outlive the XLC console itself, which is gone.

### 3. `ratatui` is still a dependency of a GUI editor

Only `term.rs` needs it now, and only as a **colour enum**: ANSI cell attributes
are typed `ratatui::style::Color` and converted for the face. Replacing that
with the core's own `Rgba` (introduced with the theme fix) drops the dependency
outright.

*(Already cut: the theme's colour type, `highlight::style_for`,
`preview::to_ratatui_style` / `preview_line_to_ratatui` — all TUI-only.)*

### 4. The viewport is a terminal cell grid

```rust
pub struct EditorViewport { x: u16, y: u16, width: u16, height: u16, text_x: u16, text_y: u16 }
```

Character cells, not points. This is what blocks real soft wrap, variable line
height, and per-glyph hit testing, and it forces the compositor to translate
between cell columns and CoreText offsets on every band — the origin of the
"span kind 254 carries UTF-16 offsets, every other kind carries visual columns"
trap in `SUISEI-TODO.md`. `SUISEI-CURRENT-STATE.md` P1.5 already owns this.

### 5. Unsaved state still lives in `~/.xei`, shared with the TUI

`breakpoints`, `session`, `undo`, `update_check`, `extensions` — plus
`hooks.toml`. The journal was moved to `~/.suisei/journal` when it was found to
collide; config moved to `~/.suisei.toml` when a `theme = "light"` left there by
the TUI made the GUI ignore dark mode. **The same class of bug is still live for
the five directories above**: two editors writing one breakpoint file, one undo
spill directory keyed by path hash, one session.

### 6. Surfaces the GUI cannot reach at all

Never referenced by the compositor, the FFI, or Swift:

| module | lines |
|---|---|
| `pr_review.rs` | 680 |
| `screensaver.rs` | 514 |
| `rebase.rs` | 339 |
| `plugin_store.rs` | 220 |
| `snippets.rs` | 182 |

Their `Mode` variants and key handlers were deleted with vim; the modules
remain. `peek.rs` (75) and `hooks.rs` (405) reach `App` but never the face.

### 7. Smaller things that are simply wrong for a GUI

- **`set_cursor_esc`** printed an OSC 12 escape to stdout to recolour the
  *terminal's* cursor. In an `.app` that is stdout garbage. *(cut)*
- **`color_u32`** recovered RGB by `format!("{c:?}")` and parsing the Debug
  string back — once per colour per frame. *(cut)*
- **7 `print!`/`eprintln!` sites** remain in core paths.
- **The Welcome window** hard-coded `.preferredColorScheme(.dark)` and its own
  grey literals, with the Recents column *brighter* than the content — the
  inverse of every macOS sidebar. *(cut)*

## Plan

Ordered by risk, not by size: each step must leave the app shippable, and the
first step of each group establishes the pattern the rest follow.

### R1 — Finish the state split from xei — **DONE 2026-07-25**
Move `session`, `undo`, `breakpoints`, `update_check`, `hooks.toml` to
`~/.suisei/`, adopting the existing files once, exactly as config did.
**Gate:** run both editors on one file; neither sees the other's breakpoints,
undo spill or session.

### R2 — Delete the unreachable surfaces — **DONE 2026-07-25**, 1,997 lines
`pr_review`, `screensaver`, `rebase`, `plugin_store`, `snippets` and the `App`
fields feeding them, plus the orphaned webview/ext-panel state.
*Correction to the table above: `git_graph.rs` is NOT unreachable — the initial
probe searched scene.rs for the module name, but the SCM graph reaches the face
as `GraphRow`, via `git_workbench` and `scm`. It stays.*

### R3 — Key ownership → responder chain *(the big one, 1,556 lines)*
One panel at a time; each step converts a panel to native focus + semantic
action FFI, then deletes its `handle_*` and its `Mode` variant.
`Palette (22)` → `Search (66)` → `Explorer + CallHierarchy (67)` →
`Settings + Preview + Debug (340)` → `SCM + GitWorkbench (818)` → delete `Mode`,
leaving `terminal_focused: bool`.
**Gate per panel:** the panel takes and releases focus by click and by Esc, and
the editor still types while it is open. Solve the double-capture trap in the
first one.

### R4 — Theme surface to what the GUI uses
Cut the 50 unread fields; keep the 13 plus whatever new chrome tokens the face
actually consumes. **Gate:** all themes render identically before/after.

### R5 — Drop `ratatui`
Retype `term.rs` cell colours as `Rgba`. **Gate:** ANSI colours, bold/dim and
256-colour output identical in the terminal panel; `ratatui` out of
`Cargo.toml`.

### R6 — Pixel viewport *(largest; already P1.5)*
`EditorViewport` in points, not cells. Unlocks soft wrap, variable line height,
and removes the cell↔UTF-16 translation trap. Do **not** combine with R3.

## Explicitly not in scope

- Rewriting `Buffer` (`Vec<String>` → rope). Real, tracked as P1.4; independent
  of everything here.
- The daemon migration (`SUISEI-CURRENT-STATE.md` P0.3 D1).
- `dap.rs` (2,975 lines): large, but it is a real debug-adapter client, not TUI
  residue. Its missing piece is a GUI surface, not a removal.
