//! Column selection: what a rectangle means on a line that is not a grid.
//!
//! A block selection is one selection per row, all on the same two **visual**
//! columns. Visual is the whole difficulty: a tab is one character and up to
//! eight columns, `한` is one character and two. Taking the block from
//! character columns would leave it ragged over exactly the text a column edit
//! exists for — an indented block, a table.

use suisei_core::app::App;
use suisei_core::buffer::Buffer;
use suisei_core::selection::Selection;

fn app_with(lines: &[&str]) -> App {
    let mut app = App::default();
    app.buffer = Buffer::from_string(&lines.join("\n"));
    app.tab_width = 4;
    app
}

/// Every selection's columns, as (row, anchor_col, head_col).
fn cells(app: &App) -> Vec<(usize, usize, usize)> {
    app.sel
        .all()
        .iter()
        .map(|s: &Selection| (s.anchor.row, s.anchor.col, s.head.col))
        .collect()
}

#[test]
fn a_block_is_one_selection_per_row() {
    let mut app = app_with(&["abcdef", "ghijkl", "mnopqr"]);
    app.select_block(0, 1, 2, 4);
    // Three rows, same two columns, and nothing downstream has to learn a new
    // shape: this is the multi-cursor set that already exists.
    assert_eq!(cells(&app), vec![(0, 1, 4), (1, 1, 4), (2, 1, 4)]);
    assert!(app.sel.is_multi());
}

#[test]
fn a_tab_is_one_character_and_four_columns() {
    // Visual column 4 is `a` on the tabbed line and `e` on the spaced one; a
    // block taken from CHARACTER columns would take `\t` on one and `a` on the
    // other and the rectangle would not be one.
    let mut app = app_with(&["\tabcd", "    abcd"]);
    app.select_block(0, 4, 1, 6);
    assert_eq!(
        cells(&app),
        vec![(0, 1, 3), (1, 4, 6)],
        "same screen columns, different character columns"
    );
    // What the two rows actually hold is the same two glyphs.
    assert_eq!(&app.buffer.line(0)[1..3], "ab");
    assert_eq!(&app.buffer.line(1)[4..6], "ab");
}

#[test]
fn a_wide_glyph_is_one_character_and_two_columns() {
    let mut app = app_with(&["한글ab", "xyzab"]);
    // Columns 0..4 — the two CJK glyphs on the first row, four ASCII on the
    // second. Both are four cells wide on screen.
    app.select_block(0, 0, 1, 4);
    assert_eq!(cells(&app), vec![(0, 0, 2), (1, 0, 4)]);
}

#[test]
fn a_short_row_keeps_a_caret_at_its_end() {
    let mut app = app_with(&["long line here", "", "hi", "another long one"]);
    app.select_block(0, 8, 3, 12);
    let c = cells(&app);
    // The empty row and the two-character row cannot reach column 8, so they
    // clamp — but they are STILL IN THE SET. Skipping them is what would make
    // typing on a ragged block miss the short lines silently.
    assert_eq!(c.len(), 4);
    assert_eq!(c[1], (1, 0, 0), "empty row: a caret at its end");
    assert_eq!(c[2], (2, 2, 2), "two characters: a caret at its end");
    assert_eq!(c[0], (0, 8, 12));
}

#[test]
fn dragging_up_is_the_same_rectangle_as_dragging_down() {
    let mut app = app_with(&["abcdef", "ghijkl", "mnopqr"]);
    app.select_block(2, 4, 0, 1);
    // Same three rows and same two columns — only the direction differs, and
    // the direction is what lets shrinking the drag back remove what it added.
    let rows: Vec<usize> = app.sel.all().iter().map(|s| s.anchor.row).collect();
    assert_eq!(rows, vec![0, 1, 2]);
    for s in app.sel.all() {
        assert_eq!((s.anchor.col, s.head.col), (4, 1), "head trails the drag");
    }
}

#[test]
fn the_caret_lands_on_the_row_the_pointer_is_on() {
    let mut app = app_with(&["abcdef", "ghijkl", "mnopqr"]);
    app.select_block(0, 1, 2, 4);
    assert_eq!(app.buffer.cursor().row, 2);
    assert_eq!(app.sel.primary().head.row, 2);

    // Dragging upward puts it on the top row instead.
    app.select_block(2, 1, 0, 4);
    assert_eq!(app.sel.primary().head.row, 0);
}

#[test]
fn typing_over_a_block_replaces_every_row() {
    let mut app = app_with(&["aaXXbb", "ccXXdd", "eeXXff"]);
    app.select_block(0, 2, 2, 4);
    app.gui_insert_text("--");
    assert_eq!(app.buffer.line(0), "aa--bb");
    assert_eq!(app.buffer.line(1), "cc--dd");
    assert_eq!(app.buffer.line(2), "ee--ff");
}

#[test]
fn a_zero_width_block_is_a_column_of_carets() {
    let mut app = app_with(&["aaa", "bbb", "ccc"]);
    app.select_block(0, 2, 2, 2);
    assert_eq!(app.sel.len(), 3);
    assert!(app.sel.all().iter().all(|s| s.is_empty()));
    app.gui_insert_text("!");
    assert_eq!(app.buffer.line(0), "aa!a");
    assert_eq!(app.buffer.line(1), "bb!b");
    assert_eq!(app.buffer.line(2), "cc!c");
}

#[test]
fn the_block_does_not_run_off_the_document() {
    let mut app = app_with(&["one", "two"]);
    app.select_block(0, 0, 40, 2);
    let rows: Vec<usize> = app.sel.all().iter().map(|s| s.anchor.row).collect();
    assert_eq!(rows, vec![0, 1]);
}

// ── Growing one with the keyboard ──

#[test]
fn extending_reads_the_rectangle_back_off_the_selections() {
    let mut app = app_with(&["abcdef", "ghijkl", "mnopqr", "stuvwx"]);
    app.select_block(0, 1, 1, 4);
    // The set does not record that it came from a rectangle, so ⌃⇧↓ has to
    // recover the columns from what is there.
    assert_eq!(app.block_extent(), Some((0, 1, 1, 4)));
    app.block_extend_rows(1);
    assert_eq!(cells(&app), vec![(0, 1, 4), (1, 1, 4), (2, 1, 4)]);
}

#[test]
fn extending_from_a_plain_caret_starts_a_block() {
    let mut app = app_with(&["abcdef", "ghijkl"]);
    app.caret_place(suisei_core::buffer::Position::new(0, 3));
    app.block_extend_rows(1);
    assert_eq!(cells(&app), vec![(0, 3, 3), (1, 3, 3)]);
}

#[test]
fn extending_back_the_way_it_came_shrinks_it() {
    let mut app = app_with(&["abcdef", "ghijkl", "mnopqr"]);
    app.select_block(0, 1, 2, 4);
    app.block_extend_rows(-1);
    let rows: Vec<usize> = app.sel.all().iter().map(|s| s.anchor.row).collect();
    assert_eq!(rows, vec![0, 1]);
}

#[test]
fn extending_stops_at_the_edges() {
    let mut app = app_with(&["abc", "def"]);
    app.select_block(0, 0, 1, 2);
    app.block_extend_rows(1);
    let rows: Vec<usize> = app.sel.all().iter().map(|s| s.anchor.row).collect();
    assert_eq!(rows, vec![0, 1], "the last row is the last row");

    app.select_block(1, 0, 0, 2);
    app.block_extend_rows(-1);
    let rows: Vec<usize> = app.sel.all().iter().map(|s| s.anchor.row).collect();
    assert_eq!(rows, vec![0, 1]);
}

#[test]
fn a_hand_built_multi_cursor_is_not_a_rectangle() {
    let mut app = app_with(&["abcdef", "ghijkl", "mnopqr"]);
    app.caret_place(suisei_core::buffer::Position::new(0, 1));
    app.caret_add(suisei_core::buffer::Position::new(2, 5));
    // Rows 0 and 2 with different columns. Growing THAT as a rectangle would
    // throw the user's carets away, so `block_extent` refuses to see one.
    assert_eq!(app.block_extent(), None);
}

// ── What the row actually paints ──

#[test]
fn every_row_of_a_block_paints_its_own_span() {
    let mut app = app_with(&["aaaaaa", "bbbbbb", "cccccc"]);
    app.select_block(0, 1, 2, 4);
    // `selected_range` answers for the primary alone; before block selection
    // there was nothing else to answer for, because a ⌘-click set is all
    // carets. A block is the first set where every selection has WIDTH.
    for row in 0..3 {
        let (s, e) = app
            .selection_on_row(row)
            .unwrap_or_else(|| panic!("row {row} paints nothing"));
        assert_eq!((s.row, s.col), (row, 1));
        assert_eq!((e.row, e.col), (row, 3), "inclusive end");
    }
}

#[test]
fn a_row_outside_the_block_paints_nothing() {
    let mut app = app_with(&["aaaaaa", "bbbbbb", "cccccc"]);
    app.select_block(0, 1, 1, 4);
    assert!(app.selection_on_row(2).is_none());
}

#[test]
fn a_single_selection_still_answers_from_the_primary() {
    let mut app = app_with(&["hello world"]);
    app.caret_place(suisei_core::buffer::Position::new(0, 0));
    app.caret_drag_to(suisei_core::buffer::Position::new(0, 5));
    assert_eq!(app.selection_on_row(0), app.selected_range());
    assert!(app.selection_on_row(0).is_some());
}
