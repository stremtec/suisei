//! Painted spans follow a single-line edit immediately, so a just-typed
//! character is coloured now instead of one tick from now.
//!
//! The parse is async and stays async — a COLD parse is 57 ms at 1,082 lines
//! and 126 ms at 4,482, which is why the worker exists. Waiting even for the
//! INCREMENTAL answer was measured at 4.1–6.7 ms per keystroke against 0.4 ms
//! without, i.e. half a frame on the hottest path in the app. So the spans
//! already on screen are slid to match the edit and the worker's real answer
//! replaces them a tick later.
//!
//! What this pins is the arithmetic of that slide. The interesting case is the
//! third: a span ending exactly AT the caret must stretch, because that is what
//! typing at the end of a word is.
//!
//! ```text
//! cargo test -p suisei-core --test optimistic_token_nudge
//! ```

use suisei_core::highlight::TokenKind;
use suisei_core::syntax::SyntaxEngine;

/// `let value = 1;` — spans for `let` and `value` on row 0, plus a decoy on
/// row 1 that must never move.
fn engine_with_spans() -> SyntaxEngine {
    let mut e = SyntaxEngine::new();
    e.tokens = vec![
        (TokenKind::Keyword, 0, 3, 0),  // let
        (TokenKind::Variable, 4, 9, 0), // value
        (TokenKind::Number, 12, 13, 0), // 1
        (TokenKind::Keyword, 0, 2, 1),  // another row
    ];
    e
}

fn row(e: &SyntaxEngine, row: usize) -> Vec<(usize, usize)> {
    e.tokens_for_row(row).iter().map(|t| (t.1, t.2)).collect()
}

#[test]
fn typing_at_the_end_of_a_word_extends_that_word() {
    let mut e = engine_with_spans();
    // Caret right after `value` (col 9) — the span ends exactly there.
    e.nudge_for_insert(0, 9, 1);
    assert_eq!(
        row(&e, 0),
        vec![(0, 3), (4, 10), (13, 14)],
        "the word being typed must grow to include the new character, and \
         everything after it must shift"
    );
}

#[test]
fn typing_inside_a_word_extends_it() {
    let mut e = engine_with_spans();
    e.nudge_for_insert(0, 6, 1); // inside `value`
    assert_eq!(row(&e, 0), vec![(0, 3), (4, 10), (13, 14)]);
}

#[test]
fn typing_before_a_span_shifts_it_without_stretching() {
    let mut e = engine_with_spans();
    e.nudge_for_insert(0, 4, 1); // exactly at `value`'s start
    assert_eq!(
        row(&e, 0),
        vec![(0, 3), (5, 10), (13, 14)],
        "a span starting at the caret moves whole; it does not adopt the \
         character typed in front of it"
    );
}

#[test]
fn other_rows_are_untouched() {
    let mut e = engine_with_spans();
    e.nudge_for_insert(0, 9, 1);
    assert_eq!(
        row(&e, 1),
        vec![(0, 2)],
        "an edit on row 0 cannot move row 1"
    );
}

#[test]
fn deleting_reverses_the_slide() {
    let mut e = engine_with_spans();
    e.nudge_for_insert(0, 9, 1);
    e.nudge_for_delete(0, 10, 1);
    assert_eq!(
        row(&e, 0),
        vec![(0, 3), (4, 9), (12, 13)],
        "backspacing the character just typed must return the spans exactly"
    );
}

#[test]
fn a_span_a_deletion_empties_is_dropped() {
    let mut e = SyntaxEngine::new();
    e.tokens = vec![
        (TokenKind::Variable, 4, 5, 0), // one character wide
        (TokenKind::Number, 8, 9, 0),
    ];
    e.nudge_for_delete(0, 5, 1); // removes exactly that character
    assert_eq!(
        row(&e, 0),
        vec![(7, 8)],
        "a span with nothing left in it must go, not linger as zero width"
    );
}

#[test]
fn a_zero_width_edit_changes_nothing() {
    let mut e = engine_with_spans();
    let before = row(&e, 0);
    e.nudge_for_insert(0, 5, 0);
    e.nudge_for_delete(0, 5, 0);
    assert_eq!(row(&e, 0), before);
}
