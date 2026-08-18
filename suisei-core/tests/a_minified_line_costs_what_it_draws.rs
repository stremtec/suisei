//! A line the editor can only show 480 columns of must only be highlighted for
//! 480 columns.
//!
//! Left over from the 8.5 MB HTML work, whose test says so in a comment: "a
//! single line this long in a `.js` file is a different problem — the
//! JavaScript grammar itself does not finish." It was not the grammar.
//!
//! ```text
//! 한 줄 10 KB    parse  13 ms
//! 한 줄 100 KB   parse  24 ms   query 384 ms   tokens 99,995
//! 한 줄 1 MB     30초 넘게 안 끝남
//! 같은 1 MB 를 4만 줄로  100 ms
//! ```
//!
//! Same bytes, 300× apart, so the cost was per LINE LENGTH. Two causes, both of
//! them the shape this codebase keeps meeting — work that outlives the drawing
//! it is for:
//!
//!   1. `byte_col_to_char_col` walked from the start of the line for EVERY
//!      capture, and a minified line is one capture per comma. O(line²).
//!   2. The query's point range `(row, 0)..(row, 0)` bounds rows and says
//!      nothing about columns, so tree-sitter walked all 8 MB to colour the
//!      480 bytes that get painted.
//!
//! Both now stop at `wrap::ROW_BYTES`, the same cut the renderer makes.
//!
//! ```text
//! cargo test -p suisei-core --test a_minified_line_costs_what_it_draws
//! ```

use std::time::{Duration, Instant};
use suisei_core::syntax::SyntaxEngine;
use suisei_core::wrap::ROW_BYTES;

/// `a.b(1),` repeated — a comma expression, which is what minifiers emit and
/// what gives the grammar one node per seven bytes.
fn minified(bytes: usize) -> String {
    "a.b(1),".repeat(bytes / 7)
}

/// The monster on row 0, three hundred ordinary rows under it.
fn file_with_a_minified_head(bytes: usize) -> String {
    let mut text = minified(bytes);
    text.push('\n');
    for i in 0..300 {
        text.push_str(&format!("const x{i} = {i};\n"));
    }
    text
}

/// The measurement that does not depend on this machine.
///
/// Both windows parse the same file, so both pay the same parse; only one of
/// them queries the minified row. A wall-clock threshold would be a guess, and
/// the earlier `typing_latency` failure is the lesson — subtracting two
/// separately-measured numbers swings on things that are not the code under
/// test. A RATIO between two runs over identical bytes does not.
#[test]
fn querying_the_minified_row_costs_no_more_than_not_querying_it() {
    let text = file_with_a_minified_head(100_000);

    let mut away = SyntaxEngine::new();
    let t = Instant::now();
    away.parse_window(&text, Some("js"), Some(250..300));
    let without = t.elapsed();

    let mut over = SyntaxEngine::new();
    let t = Instant::now();
    over.parse_window(&text, Some("js"), Some(0..50));
    let with = t.elapsed();

    // 14× before the fix at this size, and unbounded one decimal place up.
    assert!(
        with.as_secs_f64() < without.as_secs_f64() * 3.0 + 0.05,
        "the window holding the minified row took {with:?} against {without:?} \
         for the same file — the query is measuring past the cut again"
    );
}

/// The reported size, as a plain wall clock. This one never finished.
#[test]
fn a_megabyte_on_one_line_finishes() {
    let text = file_with_a_minified_head(1_000_000);
    let mut eng = SyntaxEngine::new();
    let t = Instant::now();
    eng.parse_window(&text, Some("js"), Some(0..200));
    let took = t.elapsed();
    assert!(
        took < Duration::from_secs(5),
        "a megabyte on one line took {took:?} — it is O(line²) again"
    );
}

/// Cheap is not the whole contract: what IS drawn still has to be coloured.
#[test]
fn the_drawn_head_of_a_minified_line_is_still_coloured() {
    let text = file_with_a_minified_head(100_000);
    let mut eng = SyntaxEngine::new();
    eng.parse_window(&text, Some("js"), Some(0..50));

    let head = eng.tokens_for_row(0);
    assert!(
        !head.is_empty(),
        "the first 480 columns are on screen and want their colours"
    );
    // And nothing is claimed for the part nobody can see. `ROW_BYTES + 1` is
    // the renderer's own bound: it takes that many characters before expanding
    // tabs and cutting.
    let past = head.iter().filter(|t| t.2 > ROW_BYTES + 1).count();
    assert_eq!(
        past, 0,
        "{past} tokens end past the cut, out of {}",
        head.len()
    );
}

/// The rows below it are ordinary rows and must be untouched — including the
/// LAST one in the window, which is where merging row ranges could lose a
/// newline and take a row with it.
#[test]
fn the_rows_under_it_are_highlighted_as_usual() {
    let text = file_with_a_minified_head(100_000);
    let mut eng = SyntaxEngine::new();
    eng.parse_window(&text, Some("js"), Some(1..40));

    for row in [1usize, 20, 39] {
        assert!(
            !eng.tokens_for_row(row).is_empty(),
            "row {row} is `const x… = …;` and got no tokens"
        );
    }
}

/// The cut is a rule about lines, not about minified files: a single enormous
/// STRING is the same shape and must not be walked either.
#[test]
fn one_enormous_string_literal_is_the_same_rule() {
    let mut text = String::from("const s = \"");
    text.push_str(&"x".repeat(2_000_000));
    text.push_str("\";\n");
    let mut eng = SyntaxEngine::new();
    let t = Instant::now();
    eng.parse_window(&text, Some("js"), Some(0..50));
    let took = t.elapsed();
    assert!(took < Duration::from_secs(5), "took {took:?}");

    let row = eng.tokens_for_row(0);
    assert!(!row.is_empty(), "`const` and the string's head are drawn");
    assert!(
        row.iter().all(|t| t.2 <= ROW_BYTES + 1),
        "a two-megabyte string still claimed a span past the cut"
    );
}
