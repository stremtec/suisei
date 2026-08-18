//! ⌥-drag is a rectangle, and the rectangle has to be DRAWN.
//!
//! Two facts the gesture depends on, both easy to get wrong in a way that only
//! shows on ragged text:
//!
//!   · **The anchor is a screen column, not a `Position`.** A rectangle can
//!     start past the end of a short line — column 40 of a three-character row.
//!     `mouse.drag_anchor` keeps a `Position`, which clamps that to column 3 on
//!     the way in; a drag from there down to a long line would then take a
//!     block from column 3 and the user would watch it start in the wrong
//!     place.
//!
//!   · **Every row paints its own span.** The scene draws one selection span
//!     per line and took it from the PRIMARY. That was invisible until now
//!     because a ⌘-click multi-cursor set is all carets — a block is the first
//!     set where every selection has width.
//!
//! ```text
//! cargo test -p suisei-engine --test an_option_drag_takes_a_column
//! ```

use suisei_engine::Engine;
use suisei_engine::compositor::build_editor_band;

/// `name` per test: these run in parallel and a shared path is a race, not a
/// fixture — the first version of this file had one, and the test that asserted
/// against a short line was reading another test's document.
fn engine_with(name: &str, text: &str) -> Engine {
    let dir = std::env::temp_dir().join("suisei_block_selection");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.txt"));
    std::fs::write(&path, text).expect("write source");

    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());
    engine.flush_syntax();
    engine
}

/// (line number, selection span) for the whole document.
fn spans(engine: &Engine) -> Vec<(u32, Option<(u32, u32)>)> {
    let (lines, _) = build_editor_band(&engine.app, 0, 0, 32, 0, 200);
    lines
        .iter()
        .map(|l| {
            (
                l.line_no,
                l.sel_v0.and_then(|a| l.sel_v1.map(|b| (a, b))),
            )
        })
        .collect()
}

#[test]
fn a_drag_with_option_down_selects_a_rectangle() {
    let mut engine = engine_with("rect", "aaaaaaaa\nbbbbbbbb\ncccccccc\n");
    engine.block_click_at(0, 2);
    engine.block_drag_to(2, 5);

    let sels = engine.app.sel.all();
    assert_eq!(sels.len(), 3);
    for s in sels {
        assert_eq!((s.anchor.col, s.head.col), (2, 5));
    }
}

#[test]
fn the_rectangle_is_painted_on_every_row_not_just_the_pointer_s() {
    let mut engine = engine_with("painted", "aaaaaaaa\nbbbbbbbb\ncccccccc\n");
    engine.block_click_at(0, 2);
    engine.block_drag_to(2, 5);

    let painted = spans(&engine);
    for row in 1..=3u32 {
        let (_, span) = painted
            .iter()
            .find(|(n, _)| *n == row)
            .copied()
            .unwrap_or_else(|| panic!("line {row} missing from the band"));
        assert_eq!(span, Some((2, 5)), "line {row} paints the block");
    }
    // The row below the rectangle keeps its own answer.
    let below = painted.iter().find(|(n, _)| *n == 4).map(|(_, s)| *s);
    assert!(matches!(below, None | Some(None)), "line 4 is outside it");
}

#[test]
fn a_block_that_starts_past_a_short_line_keeps_its_column() {
    // The anchor row is three characters long and the press is at column 12.
    let mut engine = engine_with("ragged", "abc\nlong line of text here\nanother long line here\n");
    engine.block_click_at(0, 12);
    engine.block_drag_to(2, 16);

    let sels = engine.app.sel.all();
    assert_eq!(sels.len(), 3);
    // Row 0 clamps to its own end — it cannot reach column 12 — but the rows
    // that CAN reach it still start there. A `Position` anchor would have
    // clamped once, on the way in, and given every row column 3.
    assert_eq!((sels[0].anchor.col, sels[0].head.col), (3, 3));
    assert_eq!((sels[1].anchor.col, sels[1].head.col), (12, 16));
    assert_eq!((sels[2].anchor.col, sels[2].head.col), (12, 16));
}

#[test]
fn an_ordinary_drag_after_a_block_is_ordinary() {
    let mut engine = engine_with("ordinary", "aaaaaaaa\nbbbbbbbb\ncccccccc\n");
    engine.block_click_at(0, 2);
    engine.block_drag_to(2, 5);
    engine.mouse_up();

    engine.click_at(0, 1, false);
    engine.drag_to(1, 4);
    assert_eq!(engine.app.sel.len(), 1, "one selection, not a rectangle");
    assert_eq!(engine.app.sel.primary().anchor.row, 0);
    assert_eq!(engine.app.sel.primary().head.row, 1);
}

#[test]
fn a_block_drag_without_a_down_starts_one() {
    let mut engine = engine_with("nodown", "aaaaaaaa\nbbbbbbbb\n");
    // Autoscroll can deliver a move before the face has sent its press.
    engine.block_drag_to(1, 4);
    assert!(!engine.app.sel.all().is_empty());
}

#[test]
fn the_keyboard_grows_the_same_rectangle() {
    let mut engine = engine_with("keyboard", "aaaaaaaa\nbbbbbbbb\ncccccccc\n");
    engine.block_click_at(0, 2);
    engine.block_drag_to(0, 5);
    engine.mouse_up();

    engine.block_extend_rows(1);
    engine.block_extend_rows(1);
    let sels = engine.app.sel.all();
    assert_eq!(sels.len(), 3);
    for s in sels {
        assert_eq!((s.anchor.col, s.head.col), (2, 5));
    }
}

#[test]
fn typing_over_a_dragged_block_reaches_every_row() {
    let mut engine = engine_with("typing", "aaXXaa\nbbXXbb\nccXXcc\n");
    engine.block_click_at(0, 2);
    engine.block_drag_to(2, 4);
    engine.mouse_up();
    engine.app.gui_insert_text("--");

    assert_eq!(engine.app.buffer.line(0), "aa--aa");
    assert_eq!(engine.app.buffer.line(1), "bb--bb");
    assert_eq!(engine.app.buffer.line(2), "cc--cc");
}
