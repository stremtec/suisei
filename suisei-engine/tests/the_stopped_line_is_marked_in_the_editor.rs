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
use suisei_engine::compositor::{DEBUG_FRAME, DEBUG_STOPPED, build_editor_band};

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
        .filter(|l| l.debug_sign & DEBUG_STOPPED != 0)
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

    assert!(row.debug_sign & DEBUG_STOPPED != 0, "stopped");
    assert!(row.git_sign & 0x40 != 0, "and breakpointed");
}

/// Reading a caller must not move where the program is.
///
/// `select_frame` used to write the chosen frame into `current_path` /
/// `current_line`, so clicking the second frame of a recursion made the editor
/// claim execution had moved there — and the real stop became unfindable. The
/// stop stays solid; the frame being read is marked hollow, separately.
#[test]
fn selecting_a_caller_marks_it_without_moving_the_stop() {
    let (mut engine, path) = engine_at("frames.rs");
    engine.app.dap.current_path = Some(path.clone());
    engine.app.dap.current_line = Some(3);
    engine.app.dap.stack = vec![
        frame(1, &path, 3),
        frame(2, &path, 6),
    ];
    engine.app.dap.selected_frame = 1;

    let (lines, _) = build_editor_band(&engine.app, 0, 0, 32, 0, 200);
    let mark = |row: u32| {
        lines
            .iter()
            .find(|l| l.line_no == row + 1)
            .map(|l| l.debug_sign)
            .unwrap_or(0)
    };

    assert!(mark(3) & DEBUG_STOPPED != 0, "the stop is still the stop");
    assert!(mark(3) & DEBUG_FRAME == 0, "and is not also drawn as a frame");
    assert!(mark(6) & DEBUG_FRAME != 0, "the caller is marked");
    assert!(mark(6) & DEBUG_STOPPED == 0, "but not as execution");
}

/// Selecting the TOP frame — which is what a fresh stop does — must not draw a
/// hollow arrow underneath its own solid one.
#[test]
fn the_top_frame_is_not_drawn_twice() {
    let (mut engine, path) = engine_at("top.rs");
    engine.app.dap.current_path = Some(path.clone());
    engine.app.dap.current_line = Some(3);
    engine.app.dap.stack = vec![frame(1, &path, 3)];
    engine.app.dap.selected_frame = 0;

    let (lines, _) = build_editor_band(&engine.app, 0, 0, 32, 0, 200);
    let row = lines.iter().find(|l| l.line_no == 4).expect("in band");
    assert!(row.debug_sign & DEBUG_STOPPED != 0);
    assert!(row.debug_sign & DEBUG_FRAME == 0);
}

fn frame(id: i64, path: &str, line: usize) -> suisei_core::dap::StackFrameInfo {
    suisei_core::dap::StackFrameInfo {
        id,
        name: format!("f{id}"),
        path: path.to_string(),
        line,
        column: 0,
    }
}

/// The value extent: a capped run from the symbol's first occurrence to its
/// last, with the writes ticked.
///
/// The kind is the whole feature. A read is where a value is used; a write is
/// where it moves, and only `documentHighlight` carries that — which is why
/// core asks it rather than `references`, whose answer cannot tell them apart.
#[test]
fn the_value_extent_caps_its_ends_and_ticks_its_writes() {
    use suisei_core::lsp::SymbolOccurrence;
    use suisei_engine::compositor::{VALUE_EXTENT, VALUE_FIRST, VALUE_LAST, VALUE_WRITE};

    let (mut engine, _) = engine_at("extent.rs");
    engine.app.lsp.highlights = vec![
        SymbolOccurrence { row: 2, write: true },
        SymbolOccurrence { row: 4, write: false },
        SymbolOccurrence { row: 6, write: true },
    ];

    let (lines, _) = build_editor_band(&engine.app, 0, 0, 32, 0, 200);
    let sign = |row: u32| {
        lines
            .iter()
            .find(|l| l.line_no == row + 1)
            .map(|l| l.debug_sign)
            .unwrap_or(0)
    };

    assert!(sign(1) & VALUE_EXTENT == 0, "before the first occurrence");
    assert!(sign(2) & VALUE_FIRST != 0, "capped at the top");
    assert!(sign(2) & VALUE_WRITE != 0, "and the declaration is a write");
    // The rule runs THROUGH rows the symbol does not appear on — the extent is
    // where the value lives, not a list of the lines that mention it.
    assert!(sign(3) & VALUE_EXTENT != 0, "the run is continuous");
    assert!(sign(3) & VALUE_WRITE == 0, "but nothing moves here");
    assert!(sign(4) & VALUE_EXTENT != 0);
    assert!(sign(4) & VALUE_WRITE == 0, "a read is not a write");
    assert!(sign(6) & VALUE_LAST != 0, "capped at the bottom");
    assert!(sign(7) & VALUE_EXTENT == 0, "after the last occurrence");
}

/// An extent that spans the whole file is noise, not information. Saying
/// nothing beats drawing a rule down every screen of a `static` used in four
/// hundred places.
#[test]
fn an_extent_longer_than_the_bound_is_not_drawn() {
    use suisei_core::lsp::SymbolOccurrence;
    use suisei_engine::compositor::VALUE_EXTENT;

    let (mut engine, _) = engine_at("huge.rs");
    engine.app.lsp.highlights = vec![
        SymbolOccurrence { row: 0, write: true },
        SymbolOccurrence { row: 5_000, write: false },
    ];

    let (lines, _) = build_editor_band(&engine.app, 0, 0, 32, 0, 200);
    assert!(lines.iter().all(|l| l.debug_sign & VALUE_EXTENT == 0));
}
