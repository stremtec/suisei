//! The row the debugger is stopped on is marked in the EDITOR, not only in the
//! panel.
//!
//! Core had computed this all along and even wrote the accessor for the caller
//! that never arrived:
//!
//! ```text
//! /// Stopped line if the session is currently stopped in `path`.
//! pub fn current_line_for(&mut self, path: &str) -> Option<usize>
//! ```
//!
//! The value crossed the whole ABI too — `current_path`, `current_line` and
//! `has_location` are written into the snapshot and decoded into `DapSnap` on
//! the Swift side — and then nothing painted it. It died one line short of a
//! pixel.
//!
//! ```text
//! cargo test -p suisei-engine --test the_stopped_line_is_marked_in_the_editor
//! ```

use suisei_engine::Engine;
use suisei_engine::compositor::{DEBUG_STOPPED, build_editor_band};

const SRC: &str = "\
fn first() {}

fn second() {
    let x = 1;
}

fn third() {}
";

fn engine_at(path_name: &str) -> (Engine, String) {
    let dir = std::env::temp_dir().join("suisei_stopped_line_mark");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(path_name);
    std::fs::write(&path, SRC).expect("write source");

    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());
    engine.flush_syntax();
    let p = path.to_string_lossy().to_string();
    (engine, p)
}

/// Rows carrying the stopped flag, as BUFFER rows.
///
/// `EditorLineScene::line_no` is the display number — `row + 1` — while
/// `DapClient::current_line` is a buffer row. Converting here rather than
/// asserting in display numbers keeps the test speaking the same units as the
/// thing under test.
fn stopped_rows(engine: &Engine) -> Vec<usize> {
    let (lines, _) = build_editor_band(&engine.app, 0, 0, 32, 0, 200);
    lines
        .iter()
        .filter(|l| l.git_sign & DEBUG_STOPPED != 0)
        .map(|l| l.line_no as usize - 1)
        .collect()
}

#[test]
fn the_row_the_program_stopped_on_carries_the_flag_and_no_other_does() {
    let (mut engine, path) = engine_at("stopped.rs");
    assert!(
        stopped_rows(&engine).is_empty(),
        "nothing is marked before a session stops"
    );

    engine.app.dap.current_path = Some(path);
    engine.app.dap.current_line = Some(3);

    assert_eq!(
        stopped_rows(&engine),
        vec![3],
        "exactly the stopped row, and exactly once"
    );
}

/// A stop in ANOTHER file must not paint this one. The line number would
/// otherwise land on whatever happens to be at that row here, which is the
/// most confident possible way to be wrong.
#[test]
fn a_stop_in_a_different_file_marks_nothing_here() {
    let (mut engine, _) = engine_at("shown.rs");
    engine.app.dap.current_path = Some("/somewhere/else/other.rs".into());
    engine.app.dap.current_line = Some(3);

    assert!(stopped_rows(&engine).is_empty());
}

/// The flag shares a byte with the breakpoint bit and the git hunk bits, and
/// that byte has form: putting the staged flag on the breakpoint's bit once
/// drew a real breakpoint on every line of a staged hunk. A row that is both
/// stopped and breakpointed has to report both.
#[test]
fn a_breakpoint_and_a_stop_on_one_row_do_not_erase_each_other() {
    let (mut engine, path) = engine_at("both.rs");
    let _ = engine.app.dap.toggle_breakpoint(&path, 3);
    engine.app.dap.current_path = Some(path);
    engine.app.dap.current_line = Some(3);

    let (lines, _) = build_editor_band(&engine.app, 0, 0, 32, 0, 200);
    let row = lines
        .iter()
        .find(|l| l.line_no == 4)  // display number of buffer row 3
        .expect("the row is in the band");

    assert!(row.git_sign & DEBUG_STOPPED != 0, "stopped");
    assert!(row.git_sign & 0x40 != 0, "and breakpointed");
}
