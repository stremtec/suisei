//! Folding: the part that was missing.
//!
//! `fold.rs` was a complete implementation — indent ranges, a closed set, an
//! O(1) hidden index, tests — and `App::fold_toggle` called into it. But
//! `is_hidden` was consulted in exactly ONE place in the whole workspace:
//! moving the caret out of the fold it had just closed. Nothing asked it what
//! to draw. So closing a fold set a status message, moved the caret, and left
//! every line on the screen.
//!
//! A feature that the source describes and the product does not do is worse
//! than one that is missing, because reading the code is how you find out.
//!
//! The fix is one fact: **a hidden row takes zero visual rows**, and it lives
//! in `WrapMap` — the single owner of buffer-row ↔ visual-row. Scroll extent,
//! scrollbar, hit-testing and the drawn band all read that map, so putting it
//! anywhere else would have let them disagree about how tall the file is.
//!
//! ```text
//! cargo test -p suisei-engine --test a_closed_fold_is_not_on_the_screen
//! ```

use suisei_engine::Engine;
use suisei_engine::compositor::build_editor_band;

const SRC: &str = "fn a() {\n    one;\n    two;\n    three;\n}\nfn b() {\n    four;\n}\n";

fn engine_with(name: &str, text: &str) -> Engine {
    let dir = std::env::temp_dir().join("suisei_fold_screen");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.rs"));
    std::fs::write(&path, text).expect("write source");
    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());
    engine.flush_syntax();
    engine
}

/// The line numbers the band actually draws.
fn drawn(engine: &Engine) -> Vec<u32> {
    let (lines, _) = build_editor_band(&engine.app, 0, 0, 64, 0, 200);
    lines.iter().map(|l| l.line_no).collect()
}

#[test]
fn closing_a_fold_takes_its_rows_off_the_screen() {
    let mut engine = engine_with("basic", SRC);
    assert_eq!(drawn(&engine), vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);

    engine.app.buffer.cursor.row = 0;
    engine.app.fold_toggle();

    // Rows 2, 3 and 4 are inside the fold. The header stays — it is what you
    // click to get them back.
    assert_eq!(drawn(&engine), vec![1, 5, 6, 7, 8, 9]);
}

#[test]
fn opening_it_again_puts_them_back() {
    let mut engine = engine_with("reopen", SRC);
    engine.app.buffer.cursor.row = 0;
    engine.app.fold_toggle();
    assert_eq!(drawn(&engine).len(), 6);
    engine.app.fold_toggle();
    assert_eq!(drawn(&engine), vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn the_document_gets_shorter_not_just_the_drawing() {
    // The whole reason this lives in `WrapMap`: the scroll extent has to agree
    // with the band. If only the renderer skipped rows, the scrollbar would
    // still describe the unfolded file and the last page would scroll into
    // nothing.
    let mut engine = engine_with("extent", SRC);
    let before = engine.wrap_total_rows(0, 0, 200);
    engine.app.buffer.cursor.row = 0;
    engine.app.fold_toggle();
    let after = engine.wrap_total_rows(0, 0, 200);
    assert_eq!(before - after, 3, "three rows folded away");
}

#[test]
fn a_fold_change_invalidates_a_cache_keyed_on_the_document() {
    // Closing a fold does not change a byte, so the buffer version does not
    // move. Without the fold generation in the validity key the map would keep
    // describing the unfolded document — and it would be RIGHT to, by its own
    // key. This is the test that would have caught that.
    let mut engine = engine_with("cachekey", SRC);
    let version_before = engine.app.buffer.version();
    let tall = engine.wrap_total_rows(0, 0, 200);
    engine.app.buffer.cursor.row = 0;
    engine.app.fold_toggle();
    assert_eq!(
        engine.app.buffer.version(),
        version_before,
        "folding must not pretend the document changed"
    );
    assert!(engine.wrap_total_rows(0, 0, 200) < tall, "map went stale");
}

#[test]
fn clicking_where_a_row_is_drawn_lands_on_that_row() {
    // Hit-testing reads the same map. With rows 2..4 gone, the second drawn
    // row is buffer row 5 — and a click there must not land on row 2.
    let mut engine = engine_with("hit", SRC);
    engine.app.buffer.cursor.row = 0;
    engine.app.fold_toggle();
    let (row, _) = engine.wrap_buffer_at(0, 0, 200, 1);
    assert_eq!(row, 4, "0-based: the brace that closes fn a");
}

#[test]
fn the_header_says_it_is_closed_and_how_much_it_holds() {
    let mut engine = engine_with("marker", SRC);
    let (lines, _) = build_editor_band(&engine.app, 0, 0, 64, 0, 200);
    assert_eq!(lines[0].fold, 1, "an open fold starts on line 1");

    engine.app.buffer.cursor.row = 0;
    engine.app.fold_toggle();
    let (lines, _) = build_editor_band(&engine.app, 0, 0, 64, 0, 200);
    assert_eq!(lines[0].fold, 2, "now closed");
    assert_eq!(lines[0].fold_lines, 3, "three lines under it");
}

#[test]
fn a_row_that_cannot_fold_carries_no_marker() {
    let engine = engine_with("plain", SRC);
    let (lines, _) = build_editor_band(&engine.app, 0, 0, 64, 0, 200);
    let body = lines.iter().find(|l| l.line_no == 2).expect("line 2");
    assert_eq!(body.fold, 0);
}

#[test]
fn folding_everything_still_leaves_a_document() {
    let mut engine = engine_with("all", SRC);
    engine.app.folds.close_all();
    let d = drawn(&engine);
    assert!(!d.is_empty(), "a fully folded file is still a file");
    assert!(engine.wrap_total_rows(0, 0, 200) >= 1);
}

#[test]
fn a_giant_closed_fold_is_stepped_over_in_one_hop() {
    // Not a timing test — a behaviour one. The band must reach the rows BELOW
    // a huge closed fold within its budget. Walking hidden rows one at a time
    // would spend the whole band inside the fold and draw nothing after it.
    let mut body = String::from("fn big() {\n");
    for i in 0..5000 {
        body.push_str(&format!("    let x{i} = {i};\n"));
    }
    body.push_str("}\nfn after() {\n    done;\n}\n");
    let mut engine = engine_with("giant", &body);
    engine.app.buffer.cursor.row = 0;
    engine.app.fold_toggle();

    let d = drawn(&engine);
    assert!(
        d.contains(&5003),
        "the line after a 5000-line closed fold must be drawn; got {:?}",
        &d[..d.len().min(8)]
    );
}

// ── The commands the gutter and the menu call ──

#[test]
fn the_gutter_folds_the_row_it_was_clicked_on_not_the_caret_s() {
    let mut engine = engine_with("gutter", SRC);
    // Caret on line 7, click the triangle on line 1.
    engine.app.buffer.cursor.row = 6;
    engine.fold_toggle_row(0);
    assert_eq!(drawn(&engine), vec![1, 5, 6, 7, 8, 9]);
    assert_eq!(engine.app.buffer.cursor.row, 6, "the caret did not move");
}

#[test]
fn a_caret_inside_a_fold_that_closes_comes_up_to_the_header() {
    // Otherwise the caret sits on a line nobody can see and the next keystroke
    // edits text off the screen.
    let mut engine = engine_with("caret", SRC);
    engine.app.buffer.cursor.row = 2;
    engine.fold_toggle_row(0);
    assert_eq!(engine.app.buffer.cursor.row, 0);
    assert!(!engine.app.folds.is_hidden(engine.app.buffer.cursor.row));
    assert_eq!(engine.app.sel.len(), 1, "and the selection follows it");
}

#[test]
fn fold_all_then_unfold_all() {
    let mut engine = engine_with("all_cmd", SRC);
    engine.fold_all(true);
    let folded = drawn(&engine);
    assert!(folded.len() < 9, "something folded: {folded:?}");
    engine.fold_all(false);
    assert_eq!(drawn(&engine), vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn the_gutter_only_folds_rows_that_have_a_triangle() {
    // Where this parts company with vim's `za`. `FoldState::toggle` falls back
    // to the ENCLOSING block on a body row, which is right for a key pressed
    // with the caret inside one — but a triangle is drawn on the header and
    // nowhere else, so a click on line 2 folding line 1's block would be the
    // gesture disagreeing with what it was aimed at.
    let mut engine = engine_with("noop", SRC);
    let before = drawn(&engine);
    engine.fold_toggle_row(1);
    assert_eq!(drawn(&engine), before, "line 2 has no triangle");
}

#[test]
fn the_keyboard_still_folds_the_block_the_caret_is_in() {
    // And the vim behaviour is kept where it belongs.
    let mut engine = engine_with("za", SRC);
    engine.app.buffer.cursor.row = 2;
    engine.app.fold_toggle();
    assert_eq!(drawn(&engine), vec![1, 5, 6, 7, 8, 9]);
}

#[test]
fn a_row_past_the_end_is_clamped_not_a_panic() {
    let mut engine = engine_with("clamp", SRC);
    engine.fold_toggle_row(9_999);
    assert!(!drawn(&engine).is_empty());
}
