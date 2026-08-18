//! What an assistive client asks the editor, and what it must be told.
//!
//! feature.txt P2: the canvas draws its text with Metal, so AppKit has no idea
//! what is in it — Welcome, Settings and Account were readable and the BODY of
//! the application was one blank rectangle.
//!
//! A text area answers in CHARACTER offsets over the whole document. These
//! tests are about the arithmetic that turns a buffer into that, because it is
//! the part that is silently wrong in the presence of one multi-byte character
//! or one empty line.
//!
//! ```text
//! cargo test -p suisei-core --test the_editor_can_be_read_by_a_screen_reader
//! ```

use suisei_core::app::App;
use suisei_core::buffer::{Buffer, Position};

/// The same arithmetic the FFI performs, over a buffer.
fn char_count(app: &App) -> usize {
    let lines = app.buffer.lines();
    lines.iter().map(|l| l.chars().count()).sum::<usize>() + lines.len().saturating_sub(1)
}

fn offset_of_row(app: &App, row: usize) -> usize {
    let lines = app.buffer.lines();
    let upto = row.min(lines.len());
    let sum: usize = lines[..upto].iter().map(|l| l.chars().count() + 1).sum();
    // No newline after the last line: a row past the end is the END of the
    // document, not one character beyond it.
    if upto == lines.len() { sum.saturating_sub(1) } else { sum }
}

fn row_of_offset(app: &App, offset: usize) -> usize {
    let lines = app.buffer.lines();
    let mut seen = 0usize;
    for (row, line) in lines.iter().enumerate() {
        let end = seen + line.chars().count();
        if offset <= end {
            return row;
        }
        seen = end + 1;
    }
    lines.len().saturating_sub(1)
}

fn app_with(text: &str) -> App {
    let mut app = App::default();
    app.buffer = Buffer::from_string(text);
    app
}

/// The newline between two lines is a character, and the one after the last
/// line is not. Getting this wrong shifts every offset in the document by one
/// per line — the reader lands a line further off with every step down.
#[test]
fn the_document_is_counted_the_way_a_text_area_counts() {
    let app = app_with("ab\ncd\n");
    // "ab" + newline + "cd" + the trailing empty line the final newline makes.
    assert_eq!(app.buffer.line_count(), 3);
    assert_eq!(char_count(&app), 2 + 1 + 2 + 1 + 0);

    assert_eq!(offset_of_row(&app, 0), 0);
    assert_eq!(offset_of_row(&app, 1), 3);
    assert_eq!(offset_of_row(&app, 2), 6);
}

/// CHARACTERS, not bytes. A screen reader asked for offset 3 of a Korean line
/// and a byte-counting editor would hand it the middle of a code point.
#[test]
fn offsets_are_characters_and_not_bytes() {
    let app = app_with("한글\nabc\n");
    assert_eq!(offset_of_row(&app, 1), 3, "two characters and a newline");
    assert_eq!(row_of_offset(&app, 0), 0);
    assert_eq!(row_of_offset(&app, 2), 0, "still on the first line");
    assert_eq!(row_of_offset(&app, 3), 1, "the newline belongs to the line above");
    assert_eq!(row_of_offset(&app, 6), 1);
}

/// An empty line is a real line with a real offset, and a caret can be on it.
#[test]
fn an_empty_line_is_not_skipped() {
    let app = app_with("a\n\nb\n");
    assert_eq!(app.buffer.line_count(), 4);
    assert_eq!(offset_of_row(&app, 1), 2);
    assert_eq!(offset_of_row(&app, 2), 3);
    assert_eq!(row_of_offset(&app, 2), 1, "the empty line");
}

/// Past the end is the end. A client that asks about an offset the document no
/// longer has must be told where the document stops, not crashed into.
#[test]
fn an_offset_past_the_end_lands_on_the_last_line() {
    let app = app_with("only\n");
    assert_eq!(row_of_offset(&app, 9_999), app.buffer.line_count() - 1);
    assert_eq!(offset_of_row(&app, 9_999), char_count(&app));
}

/// The caret's offset is what the reader is told, so it has to agree with the
/// row arithmetic in both directions.
#[test]
fn the_caret_offset_round_trips() {
    let mut app = app_with("first\nsecond\nthird\n");
    app.buffer.cursor = Position { row: 1, col: 3 };
    let at = offset_of_row(&app, 1) + 3;
    assert_eq!(at, 9);
    assert_eq!(row_of_offset(&app, at), 1);
}
