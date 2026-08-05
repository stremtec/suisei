# suisei-core — editing engine design

> **Historical rewrite blueprint.** The diagnosis below records the pre-native
> editing baseline and is not a description of the current tree. First-class
> plural selections, retained `goal_x`, modeless semantic commands, central
> `Edit`/`Delta`, delta-based undo application, incremental LSP sync and the
> asynchronous syntax worker are implemented. The `Document` rope/piece-tree
> and line-index storage described below is still the next storage phase:
> current `Buffer` text remains `Vec<String>`.

Blueprint for rewriting the editing core of `suisei-core` (forked from
`xei-core`, which is TUI-shaped). This remains a target design; the
implementation baseline was checked on 2026-07-23.

## Why a rewrite

Measured, release build, `tests/syntax_typing_perf.rs`:

| file | per keystroke | p95 |
|---|---|---|
| 500 lines | 3.4 ms | 3.8 ms |
| 3000 lines | 7.2 ms | 7.4 ms |
| **6000 lines** | **18.3 ms** | **23.3 ms** |

A 60 fps frame is 16.7 ms, and this runs *before* layout and paint. The cause is
structural, not a slow function:

- `runtime.rs :: dispatch_key()` → `recompose()` runs **synchronously** on the
  keystroke, and inside it:
  - `buffer.text()` = `lines.join("\n")` — rebuilds the whole document as one
    `String`, every keystroke
  - `fingerprint_text()` — FNV hash over **every byte**
  - `syntax.parse()` sets `self.tree = None` and **full-reparses**; the comment
    admits why (“incremental parse without edit panics”), i.e. incremental
    parsing was disabled rather than fixed
  - the highlight query then walks the whole tree and rebuilds **all** tokens
- `Buffer` is `Vec<String>` with a **single** `cursor: Position`; there is no
  selection type at all (selection exists only as `App.visual_anchor` gated by
  `Mode::Visual`)
- movement and deletion now use `unicode-segmentation`, so the caret does not
  split Hangul syllables or emoji. This does **not** provide first-class ranges,
  plural selections, word-boundary semantics or retained vertical `goal_x`.
- `Mode` conflates *editing mode* with *which panel owns the keyboard*
  (`Mode::` appears 125× in dispatch.rs, 102× in app.rs)

## The one principle

> **An edit never waits for derived state.**

Typing must be O(size of the edit). Parsing, highlighting, indexing and LSP are
*consumers* of edits that run behind, on immutable snapshots, and publish
results when ready. The renderer draws with the newest results it has and never
blocks. This is the difference between “fast enough” and “instant”.

## Architecture

### 1. Document — the text storage

```
Document {
    text:       Rope,          // structural sharing → O(1) snapshots
    version:    u64,
    line_index: LineIndex,     // offset ↔ (row, col) in O(log n), updated incrementally
}
```

- Rope (or piece tree) replaces `Vec<String>`. Kills `buffer.text()`: consumers
  take a **snapshot** instead of a rebuilt `String`.
- Snapshots are cheap and immutable → background work holds one safely while
  editing continues. This is what makes the async pipeline possible.
- All positions are byte offsets; `(row, col)` is derived through `LineIndex`.
- Movement and deletion operate on **grapheme clusters** (UAX #29), not scalars.

### 2. Selection — first-class, plural

```
Selection { anchor: usize, head: usize, goal_x: Option<f32> }
Document  { …, selections: Vec<Selection>, marked: Option<Range<usize>> }
```

- A caret is an empty selection. There is no separate “cursor”.
- Every edit applies to **all** selections, so multi-cursor is inherent rather
  than bolted on (today `MultiCursor` stores extras beside a “primary”).
- `goal_x` fixes vertical movement: today `move_up/down` call `clamp_col()` and
  the column is lost permanently.
- `marked` is the IME composing range (AppKit `setMarkedText`).

### 3. Edit — the central artifact

```
Edit  { changes: Vec<(Range<usize>, String)> }   // applied atomically
Delta { version_before, version_after, changes }  // what everyone downstream consumes
```

Today nothing produces a delta, so every consumer re-derives from full text —
that is the origin of all the O(document) work. With a delta:

- syntax: `tree.edit(&InputEdit)` + reparse with the old tree → O(change)
- highlighting: re-query only `tree.changed_ranges()`, patch the token list
- LSP: incremental `textDocument/didChange` instead of full-text sync
- undo: record the delta and its inverse — no full-document snapshots
  (`Buffer::snapshot()` currently clones every line)
- selections: map through the delta instead of clamping

### 4. Pipeline — what runs where

```
key ──▶ AppKit (NSTextInputClient)          [done]
     ──▶ command (move/delete/insert/select)
     ──▶ apply Edit to Document              ← MUST be microseconds
     ──▶ publish version, return, paint

            └─ background, on a snapshot ─▶ parse ─▶ tokens ─▶ publish(version)
                                          ─▶ LSP didChange
                                          ─▶ outline / index
```

The renderer paints with the newest published tokens; a slightly stale
highlight for one frame is invisible, a 18 ms stall is not.

### 5. Viewport

- Highlight queries limited to the visible range + overscan (tokens are only
  ever consumed per row, via `tokens_for_row` — nothing needs whole-file tokens).
- Long term, replace the terminal cell grid (`cols = css_w / cell_w`,
  `text_x = 5`) with a pixel viewport; that is what unblocks real soft wrap and
  variable line heights.

### 6. Commands, not keystrokes

AppKit's `NSStandardKeyBindingResponding` is a 3-axis product:

**direction × granularity (grapheme | word | line | paragraph | document) × extend**

So the core exposes `move`, `delete`, `select` over that product plus
`insert`/`set_marked`, and the face is a thin selector→command table. Adopting
`NSTextInputClient` already removed the synthetic-vim-key steering
(`ensureInsertMode` sending `i`), which is what broke bracket auto-pairing.

## Order of work

Each phase must be verifiable on its own; the app stays usable throughout
(old path kept until the new one passes).

1. **Latency, contained** — incremental parse via an edit descriptor, and move
   parsing off the keystroke path onto a snapshot + debounce.
   *Exit: < 1 ms per keystroke at 6000 lines, measured with the existing bench.*
2. **Document** — rope + line index + `Edit`/`Delta`; `buffer.text()` gone.
   *Exit: typing cost flat as file size grows.*
3. **Selection** — plural selections, grapheme movement, `goal_x`.
   *Exit: drag, shift-arrows, double/triple click, multi-cursor all work.*
4. **Consumers on deltas** — undo without snapshots, LSP incremental sync,
   token patching over `changed_ranges`.
5. **Viewport-limited highlighting**, then the pixel viewport.
6. **Mode decoupling** — editing state separate from panel focus.

## Non-goals

- Not moving text ownership into `NSTextStorage`/`NSTextView`: Suisei's face is
  a custom renderer over the core scene model, so that would rewrite the face,
  not the core, and would lose native multi-cursor.
- Not touching `xei-core`; the TUI keeps it unchanged.
