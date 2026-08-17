//! A Logic View tab carries the SOURCE file's path, and everything that keys
//! on a path has to know the difference.
//!
//! That is the whole hazard of making it a tab: the tab machinery is what
//! gives it panes, splits, focus and close for free, and it is also what will
//! happily treat it as the file it names.
//!
//! ```text
//! cargo test -p suisei-core --test a_logic_tab_is_a_view_not_the_file
//! ```

use suisei_core::app::App;
use suisei_core::media::FileKind;

const SRC: &str = "\
fn helper(x: i32) -> i32 {
    if x > 0 {
        return x;
    }
    0
}

fn main() {
    let a = helper(3);
}
";

fn fixture(name: &str) -> String {
    let dir = std::env::temp_dir().join("suisei_logic_tab");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(name);
    std::fs::write(&path, SRC).expect("write source");
    path.to_string_lossy().to_string()
}

#[test]
fn opening_the_view_leaves_the_source_tab_alone() {
    let path = fixture("pair.rs");
    let mut app = App::open_file(&path);
    app.open_logic_view();

    assert_eq!(app.live_tab_kind(), FileKind::Logic);
    assert_eq!(app.tabs.buffers.len(), 2, "the source tab is still there");
    // The view's buffer is empty on purpose — there is no text to show, and an
    // empty buffer is the one thing that cannot be edited into a corrupt file.
    assert_eq!(app.buffer.text().trim(), "");
    assert_eq!(app.filename.as_deref().map(|p| p.to_string_lossy().to_string()), Some(path.clone()));
}

/// The bug this guard exists for: the tab dedupe matches on filename, and a
/// Logic tab carries the source's. Opening `foo.rs` used to focus the Logic
/// pane and the text never appeared.
#[test]
fn opening_the_source_again_does_not_land_on_the_view() {
    let path = fixture("again.rs");
    let mut app = App::open_file(&path);
    app.open_logic_view();
    assert_eq!(app.live_tab_kind(), FileKind::Logic);

    app.open_new_tab(&path);
    assert_eq!(app.live_tab_kind(), FileKind::Text, "the text, not the view");
    assert!(app.buffer.text().contains("fn helper"));
}

/// Twice is once. The view is OF a file, so two of them would be two copies of
/// one fact.
#[test]
fn asking_twice_focuses_the_one_that_is_open() {
    let path = fixture("twice.rs");
    let mut app = App::open_file(&path);
    app.open_logic_view();
    let n = app.tabs.buffers.len();
    app.open_logic_view();
    assert_eq!(app.tabs.buffers.len(), n);
}

/// A file with no logic table gets a message, not an empty view of nothing.
#[test]
fn a_file_with_no_table_is_refused_rather_than_opened_empty() {
    let dir = std::env::temp_dir().join("suisei_logic_tab");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("notes.md");
    std::fs::write(&path, "# hello\n").expect("write");
    let mut app = App::open_file(&path.to_string_lossy());
    app.open_logic_view();
    assert_eq!(app.live_tab_kind(), FileKind::Text, "no view was opened");
    assert!(app.message.contains("No logic"), "and it said why: {}", app.message);
}

/// The view reads the OPEN BUFFER, unsaved edits and all. Reading the disk
/// beside an editor showing something else is the one thing a side-by-side
/// view must not do.
#[test]
fn the_view_reads_what_the_editor_has_not_saved_yet() {
    let path = fixture("unsaved.rs");
    let mut app = App::open_file(&path);
    // A third function, typed and not saved.
    app.buffer.cursor.row = app.buffer.line_count().saturating_sub(1);
    for line in ["", "fn typed_just_now() {}"] {
        app.buffer.insert_newline();
        for c in line.chars() {
            app.buffer.insert_char(c);
        }
    }
    let text = app.logic_source(std::path::Path::new(&path));
    assert!(text.contains("typed_just_now"), "the buffer, not the disk");

    let session = app.logic_session(std::path::Path::new(&path));
    assert!(
        session.rows().iter().any(|r| r.label == "typed_just_now"),
        "and the view shows it: {:?}",
        session.rows().iter().map(|r| r.label.clone()).collect::<Vec<_>>()
    );
}

/// Clicking a row takes the reader to the code — in a pane that is not the one
/// they clicked in, when there is one.
#[test]
fn revealing_a_row_moves_the_editor_to_that_line() {
    let path = fixture("reveal.rs");
    let mut app = App::open_file(&path);
    app.open_logic_view();

    let p = std::path::PathBuf::from(&path);
    let row = {
        let session = app.logic_session(&p);
        session.toggle(0);
        session
            .rows()
            .iter()
            .position(|r| r.label == "return x")
            .expect("helper's return is a row")
    };
    let line = {
        let session = app.logic_session(&p);
        session.selected = row;
        session.selected_line().expect("it names its source range")
    };
    app.reveal_logic_row(&p, line);

    assert_eq!(app.live_tab_kind(), FileKind::Text, "the code, not the view");
    assert_eq!(app.buffer.cursor.row, 2);
}
