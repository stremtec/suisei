//! Find and replace, in the buffer on screen.
//!
//! feature.txt P1: replacing across the PROJECT has worked since
//! `workspace_search.rs` was written — the file in front of you was the one
//! place it could not be done. The two are not the same job either: that one
//! rewrites files on disk, and this goes through the buffer, so it lands in
//! undo, reaches the language server, and works on a document that has never
//! been saved.
//!
//! ```text
//! cargo test -p suisei-core --test replace_works_in_the_file_you_are_looking_at
//! ```

use suisei_core::app::App;
use suisei_core::buffer::Position;
use suisei_core::search::SearchState;

fn app_with(text: &str) -> App {
    let mut app = App::default();
    app.buffer = suisei_core::buffer::Buffer::from_string(text);
    app
}

fn find(app: &mut App, pattern: &str, with: &str) {
    app.search.pattern = Some(pattern.to_string());
    app.search.matches = SearchState::collect(app.buffer.lines(), pattern);
    app.search.current = 0;
    app.search.replace_input = with.to_string();
}

#[test]
fn one_match_is_replaced_and_the_caret_moves_to_the_next() {
    let mut app = app_with("alpha beta alpha\ngamma alpha\n");
    find(&mut app, "alpha", "OMEGA");

    assert!(app.replace_current());
    assert_eq!(app.buffer.line(0), "OMEGA beta alpha");
    // Standing on the next one, ready for another press.
    assert_eq!(app.search.matches.len(), 2, "two left");
    assert_eq!(app.buffer.cursor, Position { row: 0, col: 11 });

    assert!(app.replace_current());
    assert_eq!(app.buffer.line(0), "OMEGA beta OMEGA");
    assert_eq!(app.buffer.cursor.row, 1, "on to the next line");
}

#[test]
fn replace_all_does_the_whole_buffer_in_one_undo() {
    let mut app = app_with("a x a\nb a\n");
    find(&mut app, "a", "Z");
    assert_eq!(app.replace_all_in_buffer(), 3);
    assert_eq!(app.buffer.line(0), "Z x Z");
    assert_eq!(app.buffer.line(1), "b Z");

    app.undo();
    assert_eq!(app.buffer.line(0), "a x a", "one press, one undo");
    assert_eq!(app.buffer.line(1), "b a");
}

/// The collector allows overlapping matches on purpose, so `n` steps through
/// all of them. Replacing all of them would consume the same text twice.
#[test]
fn overlapping_matches_are_replaced_once_each() {
    let mut app = app_with("aaaa\n");
    find(&mut app, "aa", "-");
    assert_eq!(app.search.matches.len(), 3, "the collector finds three");
    assert_eq!(app.replace_all_in_buffer(), 2, "and two of them are real");
    assert_eq!(app.buffer.line(0), "--");
}

/// The search is smart-case: a lowercase query matches any case. What comes
/// OUT is what was typed as the replacement; what goes away is the text that
/// actually matched, not the query that found it.
#[test]
fn a_case_insensitive_match_replaces_the_text_that_was_there() {
    let mut app = app_with("Foo foo FOO\n");
    find(&mut app, "foo", "bar");
    assert_eq!(app.replace_all_in_buffer(), 3);
    assert_eq!(app.buffer.line(0), "bar bar bar");
}

/// A replacement of a different length moves everything after it on the line.
/// Applied last-to-first, so nothing has to be adjusted afterwards.
#[test]
fn a_longer_replacement_does_not_disturb_the_matches_before_it() {
    let mut app = app_with("x.x.x\n");
    find(&mut app, "x", "wide");
    assert_eq!(app.replace_all_in_buffer(), 3);
    assert_eq!(app.buffer.line(0), "wide.wide.wide");
}

/// Positions are stale the moment the text moves, and a stale list is worse
/// than none: `n` would step to a column that no longer holds a match, and the
/// highlight would be painted over ordinary text.
#[test]
fn the_match_list_is_rebuilt_against_the_new_text() {
    let mut app = app_with("one two one\n");
    find(&mut app, "one", "1");
    app.replace_current();
    assert_eq!(app.buffer.line(0), "1 two one");
    assert_eq!(app.search.matches, vec![Position { row: 0, col: 6 }]);
}

/// Replacing with nothing is a delete, and it is a thing people do.
#[test]
fn an_empty_replacement_deletes_the_match() {
    let mut app = app_with("keep DROP keep\n");
    find(&mut app, "DROP ", "");
    assert_eq!(app.replace_all_in_buffer(), 1);
    assert_eq!(app.buffer.line(0), "keep keep");
}

#[test]
fn nothing_to_replace_is_answered_rather_than_attempted() {
    let mut app = app_with("nothing here\n");
    assert!(!app.replace_current(), "no pattern at all");
    assert_eq!(app.replace_all_in_buffer(), 0);

    find(&mut app, "absent", "x");
    assert!(!app.replace_current(), "a pattern with no matches");
    assert_eq!(app.replace_all_in_buffer(), 0);
    assert_eq!(app.buffer.line(0), "nothing here");
}
