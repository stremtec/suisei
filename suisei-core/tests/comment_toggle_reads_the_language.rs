//! ⌘/ — the token comes from the language, and the column comes from the code.
//!
//! feature.txt P1: `highlight.rs` has known the line comment for twenty-five
//! languages since the highlighter was written, and nothing outside that file
//! could ask it. This is the control that asks.
//!
//! ```text
//! cargo test -p suisei-core --test comment_toggle_reads_the_language
//! ```

use suisei_core::app::App;
use suisei_core::buffer::Position;
use suisei_core::selection::{Selection, SelectionSet};

fn app_with(name: &str, text: &str) -> App {
    let dir = std::env::temp_dir().join("suisei_comment_toggle");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write");
    App::open_file(path.to_str().unwrap())
}

fn select(app: &mut App, from: (usize, usize), to: (usize, usize)) {
    app.sel = SelectionSet::single(Selection {
        anchor: Position { row: from.0, col: from.1 },
        head: Position { row: to.0, col: to.1 },
        goal_x: None,
    });
    app.buffer.cursor = app.sel.primary().head;
}

fn caret(app: &mut App, row: usize, col: usize) {
    select(app, (row, col), (row, col));
}

#[test]
fn one_line_goes_out_and_comes_back() {
    let mut app = app_with("one.rs", "fn main() {\n    let x = 1;\n}\n");
    caret(&mut app, 1, 8);
    assert!(app.toggle_line_comment());
    assert_eq!(app.buffer.line(1), "    // let x = 1;");

    assert!(app.toggle_line_comment());
    assert_eq!(app.buffer.line(1), "    let x = 1;", "and back exactly");
}

/// The token goes at the shallowest indentation in the block, not at column
/// zero. Jamming it against the left margin destroys the shape of the code it
/// is switching off, and the shape is how anyone reads what they did.
#[test]
fn the_token_lands_at_the_blocks_own_indentation() {
    const SRC: &str = "\
fn main() {
    if x {
        deeper();
    }
}
";
    let mut app = app_with("indent.rs", SRC);
    select(&mut app, (1, 0), (3, 5));
    app.toggle_line_comment();

    assert_eq!(app.buffer.line(1), "    // if x {");
    assert_eq!(app.buffer.line(2), "    //     deeper();", "the shape is intact");
    assert_eq!(app.buffer.line(3), "    // }");
}

/// Mixed goes the other way: a block half-commented by hand becomes wholly
/// commented, and pressing again restores it.
#[test]
fn a_half_commented_block_is_commented_rather_than_half_uncommented() {
    const SRC: &str = "\
// one
two
";
    let mut app = app_with("mixed.rs", SRC);
    select(&mut app, (0, 0), (1, 3));
    app.toggle_line_comment();
    assert_eq!(app.buffer.line(0), "// // one");
    assert_eq!(app.buffer.line(1), "// two");

    app.toggle_line_comment();
    assert_eq!(app.buffer.line(0), "// one", "exactly as it was");
    assert_eq!(app.buffer.line(1), "two");
}

/// A blank line inside a block gets nothing — a trailing `// ` on an empty
/// line is litter. A blank line on its own gets one, because pressing ⌘/ there
/// has only one meaning.
#[test]
fn blank_lines_are_left_alone_inside_a_block_and_served_on_their_own() {
    let mut app = app_with("blank.rs", "a();\n\nb();\n");
    select(&mut app, (0, 0), (2, 4));
    app.toggle_line_comment();
    assert_eq!(app.buffer.line(0), "// a();");
    assert_eq!(app.buffer.line(1), "", "nothing left behind here");
    assert_eq!(app.buffer.line(2), "// b();");

    let mut lone = app_with("lone.rs", "a();\n\nb();\n");
    caret(&mut lone, 1, 0);
    lone.toggle_line_comment();
    assert_eq!(lone.buffer.line(1), "// ", "a comment, started");
}

/// Dragging down to the start of a line selects the lines ABOVE it. Commenting
/// one the reader cannot see selected is a surprise.
#[test]
fn a_selection_ending_at_column_zero_does_not_take_that_line() {
    let mut app = app_with("edge.rs", "a();\nb();\nc();\n");
    select(&mut app, (0, 0), (2, 0));
    app.toggle_line_comment();
    assert_eq!(app.buffer.line(0), "// a();");
    assert_eq!(app.buffer.line(1), "// b();");
    assert_eq!(app.buffer.line(2), "c();", "untouched");
}

/// Every language the highlighter knows a token for, and an honest refusal for
/// the ones it does not.
#[test]
fn the_token_is_the_languages_own() {
    let mut py = app_with("x.py", "def f():\n    pass\n");
    caret(&mut py, 1, 0);
    py.toggle_line_comment();
    assert_eq!(py.buffer.line(1), "    # pass");

    let mut sh = app_with("x.sh", "echo hi\n");
    caret(&mut sh, 0, 0);
    sh.toggle_line_comment();
    assert_eq!(sh.buffer.line(0), "# echo hi");

    let mut lua = app_with("x.lua", "print(1)\n");
    caret(&mut lua, 0, 0);
    lua.toggle_line_comment();
    assert_eq!(lua.buffer.line(0), "-- print(1)");

    // JSON has no line comment. Inventing one produces a file that will not
    // parse, so it says no and changes nothing.
    let mut json = app_with("x.json", "{\n  \"a\": 1\n}\n");
    caret(&mut json, 1, 0);
    assert!(!json.toggle_line_comment());
    assert_eq!(json.buffer.line(1), "  \"a\": 1");
    assert!(json.message.contains("No line comment"), "{}", json.message);
}

/// One press, one undo. A toggle that took two undos to put back would be a
/// toggle nobody trusts.
#[test]
fn the_whole_toggle_is_one_undo() {
    let mut app = app_with("undo.rs", "a();\nb();\nc();\n");
    select(&mut app, (0, 0), (2, 4));
    app.toggle_line_comment();
    assert_eq!(app.buffer.line(0), "// a();");

    app.undo();
    assert_eq!(app.buffer.line(0), "a();");
    assert_eq!(app.buffer.line(1), "b();");
    assert_eq!(app.buffer.line(2), "c();");
}

/// Every caret gets its line, and a line touched by two of them is done once.
#[test]
fn multi_cursor_comments_each_line_once() {
    let mut app = app_with("multi.rs", "a();\nb();\nc();\n");
    app.sel = SelectionSet::carets(
        &[
            Position { row: 0, col: 1 },
            Position { row: 0, col: 3 },
            Position { row: 2, col: 0 },
        ],
        0,
    );
    app.toggle_line_comment();
    assert_eq!(app.buffer.line(0), "// a();", "not `// // a();`");
    assert_eq!(app.buffer.line(1), "b();", "nobody was on this line");
    assert_eq!(app.buffer.line(2), "// c();");
}
