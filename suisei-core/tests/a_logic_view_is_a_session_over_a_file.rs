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
use suisei_core::logic::LogicRuntime;
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

// ── What the editor draws ──────────────────────────────────────────────────

/// A guide runs at the node's OWN indentation, down what the node covers —
/// and a node that occupies one line has nothing to run down.
#[test]
fn the_selection_is_marked_as_a_run_at_its_own_column() {
    let mut s = session();
    s.toggle(0);
    let branch = s.rows().iter().position(|r| r.label == "x > 0").unwrap();
    s.selected = branch;

    let marks = s.marks(&LogicRuntime::default(), false, 4);
    assert_eq!(marks.len(), 1, "just the selection: nothing is running");
    let m = marks[0];
    assert!(m.selected && !m.runtime);
    assert_eq!((m.start_row, m.end_row), (1, 3), "the whole `if`");
    assert_eq!(m.col, 4, "the `if`'s own indentation, not its body's");

    // A single-line node still gets its band, and a column of zero says there
    // is nothing to run down.
    let step = s.rows().iter().position(|r| r.label == "return x").unwrap();
    s.selected = step;
    let one = s.marks(&LogicRuntime::default(), false, 4);
    assert_eq!(one[0].start_row, one[0].end_row);
    assert_eq!(one[0].col, 0);
}

/// The branches and loops the program is inside get an amber guide — but the
/// FUNCTION does not: a line beside every line of it says nothing.
#[test]
fn the_runtime_path_is_marked_but_not_the_whole_function() {
    let mut s = session();
    s.toggle(0);
    let rt = suisei_core::logic::runtime(
        &s.tree,
        "/tmp/logic_probe.rs",
        Some(("/tmp/logic_probe.rs", 2)),
        &[("helper", "/tmp/logic_probe.rs", 2)],
    );
    assert_eq!(rt.enclosing.len(), 2, "the function and the `if`");

    let marks = s.marks(&rt, true, 4);
    let runtime: Vec<_> = marks.iter().filter(|m| m.runtime).collect();
    assert_eq!(runtime.len(), 1, "the `if` only: {marks:?}");
    assert_eq!((runtime[0].start_row, runtime[0].end_row), (1, 3));
}

/// Closing the debug panel takes every runtime mark with it. The stop band and
/// the value bracket already learned this; a guide left behind would be the
/// same bug in another colour.
#[test]
fn closing_the_debug_panel_takes_the_runtime_marks_with_it() {
    let mut s = session();
    s.toggle(0);
    let rt = suisei_core::logic::runtime(
        &s.tree,
        "/tmp/logic_probe.rs",
        Some(("/tmp/logic_probe.rs", 2)),
        &[("helper", "/tmp/logic_probe.rs", 2)],
    );
    let marks = s.marks(&rt, false, 4);
    assert!(marks.iter().all(|m| !m.runtime), "nothing amber survives: {marks:?}");
}

/// A branch is the one thing you cannot read by looking at one line, so
/// pointing at one answers "what are the two ways out of here" on the code.
#[test]
fn peeking_a_branch_marks_both_of_its_arms() {
    let mut s = session();
    s.toggle(0);
    let branch = s.rows().iter().position(|r| r.label == "x > 0").unwrap();
    s.peek = Some(branch);

    let arms: Vec<_> = s
        .marks(&LogicRuntime::default(), false, 4)
        .into_iter()
        .filter(|m| m.arm.is_some())
        .collect();
    assert_eq!(arms.len(), 1, "this `if` has one arm written: {arms:?}");
    assert_eq!(arms[0].arm, Some(true), "and it is the one taken when it holds");
    assert_eq!(arms[0].start_row, 2, "`return x;`");

    // A peek is a question, not a place: withdrawing it leaves the selection.
    let was = s.selected;
    s.peek = None;
    assert!(s.marks(&LogicRuntime::default(), false, 4).iter().all(|m| m.arm.is_none()));
    assert_eq!(s.selected, was);
}

/// Only a branch has arms. Pointing at a step answers nothing rather than
/// answering about the wrong thing.
#[test]
fn peeking_something_that_is_not_a_branch_says_nothing() {
    let mut s = session();
    s.toggle(0);
    let step = s.rows().iter().position(|r| r.label == "return x").unwrap();
    s.peek = Some(step);
    assert!(s.marks(&LogicRuntime::default(), false, 4).iter().all(|m| m.arm.is_none()));
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
