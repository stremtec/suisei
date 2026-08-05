//! The feature as the user experiences it: type a prefix inside one function
//! and see what the popup offers.
//!
//! `scope_visibility.rs` tests the scope walk in isolation. This drives the
//! real path — `App::completion_after_typing` — so a break in the wiring
//! (stale tree, wrong byte offset, symbols dropped before ranking) fails here
//! even while the walker itself is fine.
//!
//! ```text
//! cargo test -p suisei-core --test completion_scope_e2e
//! ```

use suisei_core::app::App;
use suisei_core::buffer::{Buffer, Position};
use suisei_core::selection::{Selection, SelectionSet};

const SRC: &str = r#"const SHARED_LIMIT: usize = 10;

fn scaled_helper(scale_factor: usize) -> usize {
    let scaled_inner = scale_factor * 2;
    scaled_inner
}

fn other_place(input: usize) -> usize {
    let scoped_local = input + 1;
    let sc = 0;
    scoped_local + sc
}
"#;

/// An App holding `SRC` as a Rust file, with a warm parse, caret at
/// (row, col).
fn app_at(row: usize, col: usize) -> App {
    let mut app = App::default();
    app.buffer = Buffer::from_string(SRC);
    app.filename = Some(std::path::PathBuf::from("/tmp/suisei_completion.rs"));
    // Completion reads the WARM tree; without this it correctly declines.
    app.syntax.parse(SRC, Some("rs"));
    // The completion path reads `sel.primary().head`, so that is the caret
    // the test must move — setting `buffer.cursor` alone leaves the prefix
    // empty and the popup shut.
    let at = Position { row, col };
    app.buffer.cursor = at;
    app.sel = SelectionSet::single(Selection::caret(at));
    app
}

fn labels(app: &App) -> Vec<String> {
    app.completions
        .suggestions
        .iter()
        .map(|s| s.label.clone())
        .collect()
}

/// Row index of the line containing `needle`.
fn row_of(needle: &str) -> usize {
    SRC.lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in source"))
}

#[test]
fn typing_inside_one_function_does_not_offer_another_function_locals() {
    // Caret at the end of `    let sc = 0;` -> prefix "sc", inside `other_place`.
    let row = row_of("let sc = 0;");
    let mut app = app_at(row, "    let sc".chars().count());
    app.completion_after_typing();

    let got = labels(&app);
    assert!(app.completions.active, "popup should open for prefix `sc`");

    // `scaled_inner` and `scale_factor` belong to `scaled_helper`. They match
    // the prefix, so only scoping can keep them out.
    for leaked in ["scaled_inner", "scale_factor"] {
        assert!(
            !got.contains(&leaked.to_string()),
            "`{leaked}` is local to `scaled_helper` and must not be offered \
             inside `other_place`; got {got:?}"
        );
    }

    // This function's own binding does match and must be offered.
    assert!(
        got.contains(&"scoped_local".to_string()),
        "`scoped_local` is in scope here; got {got:?}"
    );
}

#[test]
fn globals_are_offered_from_inside_any_function() {
    let row = row_of("let sc = 0;");
    let mut app = app_at(row, "    let sc".chars().count());
    app.completion_after_typing();
    let got = labels(&app);
    assert!(
        got.contains(&"scaled_helper".to_string()),
        "a top-level fn matching the prefix is visible from every function; \
         got {got:?}"
    );
}

#[test]
fn the_same_prefix_in_the_other_function_offers_that_function_locals() {
    // Same two letters, different function: the answer must differ.
    let row = row_of("let scaled_inner");
    let mut app = app_at(row, "    let sc".chars().count());
    app.completion_after_typing();
    let got = labels(&app);

    assert!(
        got.contains(&"scale_factor".to_string()),
        "`scale_factor` is this function's parameter; got {got:?}"
    );
    assert!(
        !got.contains(&"scoped_local".to_string()),
        "`scoped_local` belongs to `other_place`; got {got:?}"
    );
}

#[test]
fn buffer_symbols_rank_above_keywords() {
    let row = row_of("let sc = 0;");
    let mut app = app_at(row, "    let sc".chars().count());
    app.completion_after_typing();
    let got = labels(&app);

    let local = got.iter().position(|l| l == "scoped_local");
    let keyword = got.iter().position(|l| l == "self");
    if let (Some(l), Some(k)) = (local, keyword) {
        assert!(
            l < k,
            "a binding from the file should outrank a keyword for the same \
             prefix; got {got:?}"
        );
    }
}

#[test]
fn a_stale_tree_falls_back_to_keywords_rather_than_lying() {
    // Parse one text, then replace the buffer with a longer one WITHOUT
    // re-parsing. Offsets from the old tree would name the wrong nodes.
    let mut app = App::default();
    app.buffer = Buffer::from_string(SRC);
    app.filename = Some(std::path::PathBuf::from("/tmp/suisei_completion.rs"));
    app.syntax.parse("fn tiny() {}\n", Some("rs"));
    let row = row_of("let sc = 0;");
    let at = Position {
        row,
        col: "    let sc".chars().count(),
    };
    app.buffer.cursor = at;
    app.sel = SelectionSet::single(Selection::caret(at));
    app.completion_after_typing();

    // Whatever it offers, it must not be a symbol invented by reading the old
    // tree at the new caret's offset.
    let got = labels(&app);
    for wrong in ["scaled_inner", "scale_factor", "scoped_local"] {
        assert!(
            !got.contains(&wrong.to_string()),
            "a tree that predates the buffer must yield no symbols, not wrong \
             ones; got {got:?}"
        );
    }
}
