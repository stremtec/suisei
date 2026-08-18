//! The sticky band is built by the band's own assembler, and that is the point.
//!
//! A pinned `fn foo() {` and the same line scrolled into view have to be the
//! same picture — same syntax spans, same tab expansion, same truncation. A
//! renderer of its own for sticky scroll would be a second place the theme gets
//! applied, and it would drift the first time either side was touched.
//!
//! What a pinned row must NOT carry is the caret and the selection: the caret
//! is down in the document, and painting it on a pinned copy of its line shows
//! the user two carets.
//!
//! ```text
//! cargo test -p suisei-engine --test a_pinned_header_is_an_editor_row
//! ```

use suisei_engine::Engine;
use suisei_engine::compositor::{build_editor_band, build_sticky_band};

const NESTED: &str = "\
mod outer {
    impl Thing {
        fn method(&self) {
            let x = 1;
            let y = 2;
            let z = 3;
        }
    }
}
";

fn engine_with(name: &str, text: &str) -> Engine {
    let dir = std::env::temp_dir().join("suisei_sticky");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.rs"));
    std::fs::write(&path, text).expect("write source");

    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());
    engine.app.rebuild_folds();
    engine.flush_syntax();
    engine
}

#[test]
fn the_pinned_rows_are_the_enclosing_headers() {
    let engine = engine_with("headers", NESTED);
    // Viewport starts at row 4 (`let y = 2;`), inside all three.
    let sticky = build_sticky_band(&engine.app, 0, 4, 5, 200);
    let rows: Vec<u32> = sticky.iter().map(|l| l.line_no).collect();
    // `line_no` is 1-based, so rows 0/1/2 print as 1/2/3.
    assert_eq!(rows, vec![1, 2, 3]);
}

#[test]
fn a_pinned_row_carries_the_same_text_the_band_would_draw() {
    let engine = engine_with("sametext", NESTED);
    let sticky = build_sticky_band(&engine.app, 0, 4, 5, 200);
    let (band, _) = build_editor_band(&engine.app, 0, 0, 32, 0, 200);

    for pinned in &sticky {
        let in_band = band
            .iter()
            .find(|l| l.line_no == pinned.line_no)
            .unwrap_or_else(|| panic!("line {} missing from the band", pinned.line_no));
        assert_eq!(pinned.text, in_band.text, "line {}", pinned.line_no);
    }
}

#[test]
fn a_pinned_row_carries_the_same_syntax_spans() {
    // The reason sticky scroll goes through `build_lines_at` at all. If this
    // ever fails, the sticky band has grown a renderer of its own.
    let engine = engine_with("spans", NESTED);
    let sticky = build_sticky_band(&engine.app, 0, 4, 5, 200);
    let (band, _) = build_editor_band(&engine.app, 0, 0, 32, 0, 200);
    assert!(!sticky.is_empty());

    for pinned in &sticky {
        let in_band = band.iter().find(|l| l.line_no == pinned.line_no).expect("row");
        assert_eq!(
            pinned.spans.len(),
            in_band.spans.len(),
            "line {} highlights differently when pinned",
            pinned.line_no
        );
    }
}

#[test]
fn a_pinned_row_shows_no_caret_and_no_selection() {
    let mut engine = engine_with("nocaret", NESTED);
    // Put the caret ON a header, then scroll past it. The header is pinned and
    // the caret belongs to the copy in the document, not to the pinned one.
    engine.app.caret_place(suisei_core::buffer::Position::new(2, 8));
    let sticky = build_sticky_band(&engine.app, 0, 4, 5, 200);

    for line in &sticky {
        assert!(!line.is_cursor, "line {} pinned a caret", line.line_no);
        assert_eq!(line.sel_v0, None, "line {} pinned a selection", line.line_no);
        assert_eq!(line.sel_v1, None);
    }
}

#[test]
fn nothing_is_pinned_at_the_top_of_the_file() {
    let engine = engine_with("top", NESTED);
    assert!(build_sticky_band(&engine.app, 0, 0, 5, 200).is_empty());
}

#[test]
fn asking_for_no_rows_builds_nothing() {
    let engine = engine_with("zero", NESTED);
    assert!(build_sticky_band(&engine.app, 0, 4, 0, 200).is_empty());
}

#[test]
fn a_pane_the_desk_does_not_have_pins_nothing() {
    // Same rule the band follows: a pane that no longer exists is not a
    // request to be satisfied with some other pane's document.
    let engine = engine_with("nopane", NESTED);
    assert!(build_sticky_band(&engine.app, 7, 4, 5, 200).is_empty());
}

#[test]
fn the_pinned_rows_are_in_outermost_first_order() {
    let engine = engine_with("order", NESTED);
    let sticky = build_sticky_band(&engine.app, 0, 5, 5, 200);
    let rows: Vec<u32> = sticky.iter().map(|l| l.line_no).collect();
    let mut sorted = rows.clone();
    sorted.sort_unstable();
    assert_eq!(rows, sorted, "the face stacks these top-down");
}

#[test]
fn a_pinned_header_is_never_soft_wrapped_into_two_rows() {
    // A pinned header is ONE row. Wrapping it would push the document down by
    // an amount that changes as you scroll.
    let long = format!(
        "fn very_long_signature({}) {{\n    let x = 1;\n    let y = 2;\n}}\n",
        "argument: usize, ".repeat(40)
    );
    let engine = engine_with("wrapped", &long);
    let sticky = build_sticky_band(&engine.app, 0, 2, 5, 200);
    assert_eq!(sticky.len(), 1, "one header, one row");
}
