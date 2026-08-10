//! Reported: typing paints the text first and the colour one beat later.
//!
//! The parse runs on a worker, which is right — a COLD parse is 57 ms at 1,082
//! lines and 126 ms at 4,482. But `refresh_syntax` adopts finished frames
//! BEFORE it ships the new request, so a keystroke can never see the answer to
//! its own edit; the soonest it lands is the next 50 ms tick. Measured with
//! nothing in place, the painted tokens described buffer version 7 while the
//! buffer was at 26 — the tick is what makes nineteen stale versions look like
//! a single beat.
//!
//! Waiting for the incremental answer was tried and rejected on cost: 4.1 ms
//! (1k lines) to 6.7 ms (4.5k lines) per keystroke against 0.4 ms without, i.e.
//! half a frame on the hottest path. Instead the spans already on screen are
//! slid to match the edit, which is O(one row's spans) and measured at 0.22 ms
//! / 0.57 ms — the original cost. The worker's real answer replaces them a tick
//! later.
//!
//! `suisei-core/tests/optimistic_token_nudge.rs` pins the arithmetic. This pins
//! the wiring: that typing through the real keystroke path actually moves the
//! spans, without waiting.
//!
//! ```text
//! cargo test -p suisei-engine --test typing_paints_its_own_colours
//! ```

use suisei_engine::Engine;

const SRC: &str = "\
fn main() {
    let value = 1;
}
";

fn engine_on_a_warm_rust_file() -> Engine {
    let dir = std::env::temp_dir().join("suisei_typing_colour");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("colour_demo.rs");
    std::fs::write(&path, SRC).expect("write source");

    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());
    // Warm, as the app is once a file has been open a moment.
    engine.flush_syntax();
    engine
}

fn put_caret(engine: &mut Engine, row: usize, col: usize) {
    let at = suisei_core::buffer::Position { row, col };
    engine.app.buffer.cursor = at;
    engine.app.sel =
        suisei_core::selection::SelectionSet::single(suisei_core::selection::Selection::caret(at));
}

/// The span covering `col` on `row`, if any.
fn span_at(engine: &Engine, row: usize, col: usize) -> Option<(usize, usize)> {
    engine
        .app
        .syntax
        .tokens_for_row(row)
        .iter()
        .find(|t| t.1 <= col && col < t.2)
        .map(|t| (t.1, t.2))
}

/// The first span on `row`.
///
/// Taken from the tokens rather than named in the test: which identifiers a
/// grammar's `highlights.scm` captures is the grammar's business — Rust's does
/// not colour a plain `let` binding's name, so an earlier version of this test
/// failed in its SETUP rather than on the behaviour it meant to check.
fn first_span(engine: &Engine, row: usize) -> (usize, usize) {
    let t = engine
        .app
        .syntax
        .tokens_for_row(row)
        .first()
        .copied()
        .expect("this row has at least one highlighted span");
    (t.1, t.2)
}

#[test]
fn a_character_typed_at_the_end_of_a_word_is_coloured_with_it() {
    let mut engine = engine_on_a_warm_rust_file();
    let row = SRC.lines().position(|l| l.contains("let value")).unwrap();
    let before = first_span(&engine, row);

    // Caret at the very end of that span — which is what typing at the end of a
    // word is.
    put_caret(&mut engine, row, before.1);
    engine.gui_type_char('x');

    let after = span_at(&engine, row, before.1).expect(
        "the character just typed must be inside a span — uncoloured here is the \
         reported one-beat lag",
    );
    assert_eq!(
        after,
        (before.0, before.1 + 1),
        "the word being typed must have grown by exactly the new character"
    );
}

#[test]
fn every_span_after_the_caret_moves_with_its_text() {
    let mut engine = engine_on_a_warm_rust_file();
    let row = SRC.lines().position(|l| l.contains("let value")).unwrap();
    let before: Vec<(usize, usize)> = engine
        .app
        .syntax
        .tokens_for_row(row)
        .iter()
        .map(|t| (t.1, t.2))
        .collect();
    assert!(!before.is_empty(), "this row has spans to move");

    // At column 0, so every span on the row is strictly after the edit.
    put_caret(&mut engine, row, 0);
    engine.gui_type_char('q');

    let after: Vec<(usize, usize)> = engine
        .app
        .syntax
        .tokens_for_row(row)
        .iter()
        .map(|t| (t.1, t.2))
        .collect();
    let want: Vec<(usize, usize)> = before.iter().map(|(a, b)| (a + 1, b + 1)).collect();
    assert_eq!(
        after, want,
        "every span must slide by exactly the character inserted in front of it; \
         spans that stay put leave the colour attached to the wrong characters"
    );
}

#[test]
fn backspace_puts_the_spans_back() {
    let mut engine = engine_on_a_warm_rust_file();
    let row = SRC.lines().position(|l| l.contains("let value")).unwrap();
    let before = first_span(&engine, row);

    put_caret(&mut engine, row, before.1);
    engine.gui_type_char('x');
    engine.gui_delete_backward();

    assert_eq!(
        span_at(&engine, row, before.1 - 1),
        Some(before),
        "typing then deleting must leave the spans exactly as they were"
    );
}
