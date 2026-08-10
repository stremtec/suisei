//! Nested highlight captures resolve to non-overlapping spans, inner wins.
//!
//! Captures nest because they are tree nodes — an escape sequence inside a
//! string, a bracket inside a type argument list. The face paints spans in
//! array order with `addAttributes`, so the LAST span applied is the colour
//! that shows. The tokens used to be sorted by width ascending, which put the
//! widest span last: every precise capture was overwritten by the vaguer one
//! containing it, and an escape sequence could never look different from its
//! string.
//!
//! This pins the arithmetic of the fix. `every_grammar_loads.rs` pins that
//! real grammars still paint through it.
//!
//! ```text
//! cargo test -p suisei-core --test overlapping_captures
//! ```

use suisei_core::highlight::TokenKind;
use suisei_core::syntax::flatten_overlaps;

/// (kind, start, end, row) with a readable kind, since the assertions are
//  about geometry and which kind survived, not about colour.
fn spans(v: &[(TokenKind, usize, usize, usize)]) -> Vec<(TokenKind, usize, usize, usize)> {
    flatten_overlaps(v.to_vec())
}

#[test]
fn an_inner_span_splits_the_one_around_it() {
    // `"a\nb"` — the string spans 0..6, the escape 2..4.
    let got = spans(&[(TokenKind::String, 0, 6, 0), (TokenKind::Operator, 2, 4, 0)]);
    assert_eq!(
        got,
        vec![
            (TokenKind::String, 0, 2, 0),
            (TokenKind::Operator, 2, 4, 0),
            (TokenKind::String, 4, 6, 0),
        ],
        "the enclosing span must yield the middle to the nested one and resume \
         after it"
    );
}

#[test]
fn an_inner_span_at_the_start_leaves_no_empty_fragment() {
    let got = spans(&[(TokenKind::String, 0, 6, 0), (TokenKind::Operator, 0, 2, 0)]);
    assert_eq!(
        got,
        vec![(TokenKind::Operator, 0, 2, 0), (TokenKind::String, 2, 6, 0)],
        "a zero-width leading fragment must not be emitted"
    );
}

#[test]
fn identical_ranges_let_the_later_capture_win() {
    // This is the contract `lang.rs` composes queries against: the
    // language-specific overlay is concatenated last, so where it captures the
    // same node as the base query it is the one that paints.
    let got = spans(&[
        (TokenKind::Variable, 3, 8, 0),
        (TokenKind::TypeName, 3, 8, 0),
    ]);
    assert_eq!(
        got,
        vec![(TokenKind::TypeName, 3, 8, 0)],
        "the last capture pushed for a range is the one that survives"
    );
}

#[test]
fn siblings_are_left_alone() {
    let got = spans(&[
        (TokenKind::Keyword, 0, 3, 0),
        (TokenKind::Variable, 4, 9, 0),
        (TokenKind::Number, 12, 13, 0),
    ]);
    assert_eq!(
        got,
        vec![
            (TokenKind::Keyword, 0, 3, 0),
            (TokenKind::Variable, 4, 9, 0),
            (TokenKind::Number, 12, 13, 0),
        ]
    );
}

#[test]
fn rows_do_not_bleed_into_each_other() {
    let got = spans(&[
        (TokenKind::Comment, 0, 20, 0),
        (TokenKind::Keyword, 0, 3, 1),
        (TokenKind::String, 4, 9, 1),
    ]);
    assert_eq!(
        got,
        vec![
            (TokenKind::Comment, 0, 20, 0),
            (TokenKind::Keyword, 0, 3, 1),
            (TokenKind::String, 4, 9, 1),
        ],
        "a wide span on row 0 must not be treated as enclosing row 1"
    );
}

#[test]
fn three_levels_deep_still_resolves() {
    let got = spans(&[
        (TokenKind::Comment, 0, 30, 0),
        (TokenKind::String, 5, 20, 0),
        (TokenKind::Number, 10, 12, 0),
    ]);
    assert_eq!(
        got,
        vec![
            (TokenKind::Comment, 0, 5, 0),
            (TokenKind::String, 5, 10, 0),
            (TokenKind::Number, 10, 12, 0),
            (TokenKind::String, 12, 20, 0),
            (TokenKind::Comment, 20, 30, 0),
        ]
    );
}

#[test]
fn the_result_is_sorted_by_row_then_column() {
    // `tokens_for_row` binary-searches on row, and the face takes the first 32
    // spans of a line — both are wrong if the order is not this.
    let got = spans(&[
        (TokenKind::String, 4, 30, 2),
        (TokenKind::Number, 10, 12, 2),
        (TokenKind::Keyword, 0, 3, 1),
        (TokenKind::Comment, 0, 8, 0),
    ]);
    let keys: Vec<(usize, usize)> = got.iter().map(|t| (t.3, t.1)).collect();
    let mut want = keys.clone();
    want.sort();
    assert_eq!(keys, want, "spans must come out ordered by (row, start)");
}

#[test]
fn nothing_overlaps_afterwards() {
    let got = spans(&[
        (TokenKind::Comment, 0, 30, 0),
        (TokenKind::String, 5, 20, 0),
        (TokenKind::Number, 10, 12, 0),
        (TokenKind::Operator, 12, 14, 0),
        (TokenKind::Variable, 25, 28, 0),
    ]);
    for pair in got.windows(2) {
        assert!(
            pair[0].3 != pair[1].3 || pair[0].2 <= pair[1].1,
            "spans {:?} and {:?} overlap; the face would paint one over the other",
            pair[0],
            pair[1]
        );
    }
}
