//! The state behind a Logic View pane.
//!
//! `logic.rs` is the extractor and has no memory. This is the part that has
//! one — what file, what is open, what is selected — and the questions worth
//! asking of it are about what survives a change.
//!
//! ```text
//! cargo test -p suisei-core --test a_logic_view_is_a_session_over_a_file
//! ```

use std::path::Path;
use suisei_core::logic_view::{LogicSession, LogicViews};

const SRC: &str = "\
fn helper(x: i32) -> i32 {
    if x > 0 {
        return x;
    }
    0
}

fn main() {
    let a = helper(3);
    println!(\"{}\", a);
}
";

fn session() -> LogicSession {
    LogicSession::open(Path::new("/tmp/logic_probe.rs"), SRC, None)
}

#[test]
fn a_file_opens_as_its_functions_and_nothing_else() {
    let s = session();
    let labels: Vec<&str> = s.rows().iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, vec!["helper", "main"]);
    assert!(s.note.is_none());
}

/// An edit is exactly when someone is reading, so a rebuild must not close
/// what they had opened.
///
/// By NAME rather than by index: a line inserted above moves every row, and
/// index 7 after an edit is not the row that was open before it.
#[test]
fn an_edit_rebuilds_the_tree_and_keeps_open_what_was_open() {
    let mut s = session();
    s.toggle(0);
    let opened = s.rows().len();
    assert!(opened > 2, "helper's body is showing");

    // Two lines land above everything, so every row's line number moves.
    let edited = format!("// a note\n// and another\n{SRC}");
    s.refresh(&edited, None);

    assert_eq!(s.rows().len(), opened, "still open after the edit");
    let helper = s.rows().iter().find(|r| r.label == "helper").expect("still there");
    assert!(helper.expanded);
    assert_eq!(helper.start_row, 2, "and it moved down with the text");
}

/// Same text, no work. The view is asked for on every frame the pane draws.
#[test]
fn text_that_did_not_change_is_not_reparsed() {
    let mut s = session();
    s.toggle(0);
    let before = s.rows().len();
    s.refresh(SRC, None);
    assert_eq!(s.rows().len(), before, "the open function stayed open");
    assert!(s.is_current(SRC));
}

/// "Nothing here" and "I could not read this" need different reactions — the
/// same rule the model viewer arrived at from the other side.
#[test]
fn a_file_with_no_logic_says_so_rather_than_showing_an_empty_list() {
    let empty = LogicSession::open(Path::new("/tmp/logic_probe.rs"), "const A: i32 = 1;\n", None);
    assert!(empty.rows().is_empty());
    assert!(empty.note.is_some(), "it says why it is empty");

    let untabled = LogicSession::open(Path::new("/tmp/notes.md"), "# hello\n", None);
    assert!(untabled.note.is_some(), "Markdown has no control flow to be wrong about");
}

/// Clicking a line of source finds the row it belongs to — the containment
/// test the runtime overlay uses, read the other way round.
#[test]
fn the_view_can_follow_the_editor() {
    let mut s = session();
    s.toggle(0);
    assert!(s.follow_source(2), "`return x;` is inside helper");
    assert_eq!(s.rows()[s.selected].label, "return x");
    assert_eq!(s.selected_line(), Some(2));
    // Twice on the same line is not a move.
    assert!(!s.follow_source(2));
}

/// The rail sits beside the editor, so "the function you are reading" is the
/// one thing it can know without being asked — and it opens THAT function.
///
/// Only that one. Walking into every call from here would build the whole
/// file, which is the one thing the collapse exists to prevent.
#[test]
fn following_the_caret_opens_the_function_the_caret_is_in() {
    let mut s = session();
    assert_eq!(s.rows().len(), 2, "both functions closed to begin with");

    // Line 2 is `return x;`, inside `helper`.
    assert!(s.follow_caret(2));
    assert!(s.rows()[0].expanded, "helper opened");
    assert_eq!(s.rows()[s.selected].label, "return x", "and the row is the caret's");
    assert!(
        s.rows().iter().all(|r| r.label != "main" || !r.expanded),
        "main was not opened: nothing but the caret's function is built"
    );

    // Moving inside the same function does not rebuild it, it just moves.
    let opened = s.rows().len();
    s.follow_caret(4);
    assert_eq!(s.rows().len(), opened);
    assert_eq!(s.rows()[s.selected].label, "0");
}

/// Two Logic tabs are two sessions, and switching between them does not close
/// everything the reader had opened in the other.
#[test]
fn each_file_keeps_its_own_open_functions() {
    let mut views = LogicViews::default();
    let a = Path::new("/tmp/logic_a.rs");
    let b = Path::new("/tmp/logic_b.rs");

    views.get(a, SRC, None).toggle(0);
    let opened = views.get(a, SRC, None).rows().len();
    views.get(b, SRC, None);
    assert_eq!(views.get(a, SRC, None).rows().len(), opened, "a is as it was left");
    assert_eq!(views.get(b, SRC, None).rows().len(), 2, "b was never opened");

    views.forget(a);
    assert!(views.peek(a).is_none());
    assert!(views.peek(b).is_some());
}
