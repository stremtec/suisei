//! A blank line inside a selection still shows that it is selected.
//!
//! Select-all over a file with blank lines between blocks left every blank line
//! looking untouched, so a fully-selected document read as a striped one. The
//! core was right all along: `selection_on_line` returns `0..1` for an empty
//! row — one cell, standing for the newline, which is genuinely what is
//! selected there. `build_editor_band` then clamped the span to the row's TEXT
//! width, which for a blank row is zero, and dropped it.
//!
//! The caret has the same need one column past the end and already gets the
//! allowance, with a comment explaining why. This is the selection's half of
//! it.
//!
//! ```text
//! cargo test -p suisei-engine --test a_blank_line_inside_a_selection_is_marked
//! ```

use suisei_engine::Engine;
use suisei_engine::compositor::build_editor_band;

const SRC: &str = "\
first = 1

middle = 2

last = 3
";

fn engine_with_blank_lines() -> Engine {
    let dir = std::env::temp_dir().join("suisei_blank_line_selection");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("blanks.toml");
    std::fs::write(&path, SRC).expect("write source");

    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());
    engine.flush_syntax();
    engine
}

/// (row index, has a selection span) for the whole document.
fn selected_rows(engine: &Engine) -> Vec<(usize, bool)> {
    let (lines, _) = build_editor_band(&engine.app, 0, 0, 32, 0);
    lines
        .iter()
        .map(|l| (l.line_no as usize, l.sel_v0.is_some()))
        .collect()
}

#[test]
fn every_row_of_a_selected_document_is_marked_including_the_blank_ones() {
    let mut engine = engine_with_blank_lines();
    engine.select_all();

    // The row AFTER the file's final newline is a real buffer row and is
    // genuinely NOT selected: `selected_range` is inclusive on both ends, so
    // select-all reports `(0,0)..(4,8)` — the last actual character — not
    // `(5,0)`, where no character exists. Asserting otherwise would be
    // asserting against the core's own convention rather than finding a bug.
    let last_selected = engine
        .app
        .selected_range()
        .expect("select-all leaves a range")
        .1
        .row;

    let unmarked: Vec<usize> = selected_rows(&engine)
        .into_iter()
        .filter(|(row, sel)| !*sel && *row <= last_selected + 1)
        .map(|(row, _)| row)
        .collect();
    assert!(
        unmarked.is_empty(),
        "select-all left rows {unmarked:?} unmarked; blank lines are inside the \
         selection and their newline is part of it"
    );
}

#[test]
fn a_blank_line_gets_exactly_one_cell() {
    let mut engine = engine_with_blank_lines();
    engine.select_all();

    let (lines, _) = build_editor_band(&engine.app, 0, 0, 32, 0);
    // Rows are 1-based in the scene; SRC's blank lines are the 2nd and 4th.
    let blank = lines
        .iter()
        .find(|l| l.line_no == 2)
        .expect("row 2 is in the band");
    assert_eq!(blank.text, "", "row 2 is the blank one");
    assert_eq!(
        (blank.sel_v0, blank.sel_v1),
        (Some(0), Some(1)),
        "one cell, standing for the newline — not zero, and not the width of \
         some other row"
    );
}

#[test]
fn a_row_outside_the_selection_stays_unmarked() {
    // The fix widens a clamp, so the thing to check is that it did not widen it
    // into rows that are not selected at all.
    let engine = engine_with_blank_lines();
    let marked: Vec<usize> = selected_rows(&engine)
        .into_iter()
        .filter(|(_, sel)| *sel)
        .map(|(row, _)| row)
        .collect();
    assert!(
        marked.is_empty(),
        "with no selection, rows {marked:?} claim one anyway"
    );
}
