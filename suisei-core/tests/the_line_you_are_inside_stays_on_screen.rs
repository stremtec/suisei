//! Sticky scroll: the headers you are inside, pinned above the viewport.
//!
//! No second model of the document. Scope nesting is what `fold.rs` already
//! computes — a fold range IS "this header owns these lines" — so sticky scroll
//! is a query against the ranges, not a parallel outline that can disagree with
//! the fold triangles sitting in the same gutter.
//!
//! Two properties do the work:
//!
//!   · **A header is not its own ancestor.** If the top line of the viewport is
//!     `fn foo() {`, nothing is pinned — the answer is already on screen. The
//!     containment test is strict at the start and inclusive at the end.
//!   · **Truncation drops the INNERMOST.** With deeper nesting than room,
//!     `mod` + `impl` + `fn` tells you where you are; the three innermost `if`s
//!     do not.
//!
//! ```text
//! cargo test -p suisei-core --test the_line_you_are_inside_stays_on_screen
//! ```

use suisei_core::app::App;
use suisei_core::buffer::Buffer;

fn app_with(text: &str) -> App {
    let mut app = App::default();
    app.buffer = Buffer::from_string(text);
    app.tab_width = 4;
    app.rebuild_folds();
    app
}

/// Rust-ish nesting, four levels deep, with the row numbers that matter:
///
/// ```text
/// 0  mod outer {
/// 1      impl Thing {
/// 2          fn method(&self) {
/// 3              if cond {
/// 4                  deep();
/// 5              }
/// 6          }
/// 7      }
/// 8  }
/// ```
const NESTED: &str = "\
mod outer {
    impl Thing {
        fn method(&self) {
            if cond {
                deep();
            }
        }
    }
}
";

#[test]
fn the_top_of_the_file_has_nothing_above_it() {
    let app = app_with(NESTED);
    assert_eq!(app.sticky_headers(0, 5), Vec::<usize>::new());
}

#[test]
fn a_header_is_not_its_own_ancestor() {
    let app = app_with(NESTED);
    // Viewport starts ON `impl Thing {`. `mod outer {` is above it and pinned;
    // the `impl` line is drawn by the band itself and must not be pinned too,
    // or it appears twice.
    assert_eq!(app.sticky_headers(1, 5), vec![0]);
}

#[test]
fn every_enclosing_header_is_pinned_outermost_first() {
    let app = app_with(NESTED);
    // Row 4 (`deep();`) is inside all four.
    assert_eq!(app.sticky_headers(4, 8), vec![0, 1, 2, 3]);
}

#[test]
fn leaving_a_scope_unpins_its_header() {
    let app = app_with(NESTED);
    // Row 6 is `}` closing the fn — indent 8, so it is still inside `impl` and
    // `mod`, but the `if` body ended at row 4 and the fn body at row 5.
    let at_6 = app.sticky_headers(6, 8);
    assert!(!at_6.contains(&3), "the if is behind us: {at_6:?}");
    assert!(at_6.contains(&0), "still inside the mod: {at_6:?}");
}

#[test]
fn truncation_keeps_the_outermost() {
    let app = app_with(NESTED);
    // Room for two of four. `mod` + `impl` locates you; `if` alone would not.
    assert_eq!(app.sticky_headers(4, 2), vec![0, 1]);
}

#[test]
fn no_room_pins_nothing() {
    let app = app_with(NESTED);
    assert_eq!(app.sticky_headers(4, 0), Vec::<usize>::new());
}

#[test]
fn a_flat_file_pins_nothing() {
    let app = app_with("one\ntwo\nthree\nfour\n");
    for row in 0..4 {
        assert_eq!(app.sticky_headers(row, 5), Vec::<usize>::new(), "row {row}");
    }
}

#[test]
fn a_closed_fold_does_not_pin_the_rows_it_swallowed() {
    let mut app = app_with(NESTED);
    // Close the `impl`. Rows 2..=6 stop being drawn, so no row inside them can
    // be the top of the viewport — but the query is public and must not offer
    // a header that is not on screen either.
    app.folds.close_at(1);
    let pinned = app.sticky_headers(4, 8);
    assert!(
        pinned.iter().all(|r| !app.folds.is_hidden(*r)),
        "pinned a hidden header: {pinned:?}"
    );
}

#[test]
fn tabs_and_spaces_agree_about_depth() {
    // `line_indent` expands tabs with the tab width, so the same shape indented
    // either way has to produce the same nesting — otherwise sticky scroll is
    // right in one file and empty in the next for no visible reason.
    let spaces = app_with("fn a() {\n    if b {\n        c();\n    }\n}\n");
    let tabs = app_with("fn a() {\n\tif b {\n\t\tc();\n\t}\n}\n");
    assert_eq!(spaces.sticky_headers(2, 5), vec![0, 1]);
    assert_eq!(tabs.sticky_headers(2, 5), vec![0, 1]);
}

// ── The scan is keyed, because sticky asks every frame ──

#[test]
fn an_unchanged_document_is_not_rescanned() {
    let mut app = app_with(NESTED);
    // Stand in for the scan: hand-clear the ranges. A guarded refresh must NOT
    // put them back, because it must not have run at all.
    app.folds.ranges.clear();
    app.folds_refresh();
    assert!(
        app.folds.ranges.is_empty(),
        "folds_refresh rescanned a document that did not change"
    );
}

#[test]
fn an_edited_document_is_rescanned() {
    let mut app = app_with(NESTED);
    app.folds.ranges.clear();
    app.gui_insert_text("x");
    app.folds_refresh();
    assert!(!app.folds.ranges.is_empty(), "an edit must invalidate the scan");
}

#[test]
fn changing_the_tab_width_is_rescanned() {
    // The tab width is an INPUT to the scan, and it moves without the buffer
    // version moving. Keyed on the version alone, a document would keep the
    // nesting the old indent rule found until the next keystroke.
    let mut app = app_with("fn a() {\n\tif b {\n\t\tc();\n\t}\n}\n");
    app.folds.ranges.clear();
    app.tab_width = 8;
    app.folds_refresh();
    assert!(!app.folds.ranges.is_empty(), "tab width must invalidate the scan");
}
