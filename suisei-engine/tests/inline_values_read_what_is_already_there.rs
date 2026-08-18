//! Inline values: `x = 5` at the end of a line, at no cost.
//!
//! The plan warned that this would be "an evaluate per variable per visible
//! line, re-run on every step". It is not, for the case that matters: a stop
//! auto-expands the first scope so the panel can show Locals, which means the
//! frame's locals are ALREADY fetched. A line of source mostly mentions
//! locals, so the annotation is a lookup rather than a request.
//!
//! ```text
//! cargo test -p suisei-engine --test inline_values_read_what_is_already_there
//! ```

use suisei_core::dap::{DapState, StackFrameInfo, VarNode};
use suisei_engine::Engine;

const SRC: &str = "\
fn main() {
    let count = 3;
    let account = 99;
    println!(\"{}\", count);
    let other = 1;
}
";

fn stopped_engine(named: &str) -> (Engine, String) {
    let dir = std::env::temp_dir().join("suisei_inline_values");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(named);
    std::fs::write(&path, SRC).expect("write source");

    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());
    engine.flush_syntax();

    let p = path.to_string_lossy().to_string();
    let dap = &mut engine.app.dap;
    dap.panel_open = true;
    dap.state = DapState::Stopped;
    dap.current_path = Some(p.clone());
    dap.current_line = Some(3);
    dap.stack = vec![StackFrameInfo {
        id: 1,
        name: "main".into(),
        path: p.clone(),
        line: 3,
        column: 0,
    }];
    // What a stop leaves behind for the panel: the scope root, auto-expanded,
    // with its children already fetched.
    dap.vars = vec![
        scope("Locals"),
        local("count", "3"),
        local("account", "99"),
    ];
    (engine, p)
}

fn scope(name: &str) -> VarNode {
    VarNode {
        name: name.into(),
        value: String::new(),
        typ: String::new(),
        var_ref: 1,
        depth: 0,
        expanded: true,
        is_scope: true,
    }
}

fn local(name: &str, value: &str) -> VarNode {
    VarNode {
        name: name.into(),
        value: value.into(),
        typ: "int".into(),
        var_ref: 0,
        depth: 1,
        expanded: false,
        is_scope: false,
    }
}

#[test]
fn a_row_is_annotated_with_the_locals_it_names() {
    let (engine, _) = stopped_engine("named.rs");
    let got = engine.inline_values(0, 6);
    let by_row: std::collections::HashMap<u32, String> = got.into_iter().collect();

    // `let count = 3;` names one local.
    assert_eq!(by_row.get(&1).map(String::as_str), Some("count = 3"));
    // `let account = 99;` names the other — and NOT `count`, which it merely
    // contains. A row annotated with a variable that is not on it is read as a
    // fact about that row.
    assert_eq!(by_row.get(&2).map(String::as_str), Some("account = 99"));
    // A USE counts, not only a declaration — the value at the moment the line
    // runs is what the annotation is for.
    assert_eq!(by_row.get(&3).map(String::as_str), Some("count = 3"));
    // A row that mentions nothing in scope gets nothing.
    assert!(by_row.get(&4).is_none(), "`let other = 1;` names no local");
    assert!(by_row.get(&0).is_none(), "`fn main() {{` names none either");
}

/// And not once the panel is closed.
///
/// The values are the debugger talking, and a reader who closed the panel is
/// not debugging. This was missing: closing the debug area left `count = 3`
/// sitting at the end of every line it had annotated, and the only thing that
/// had ever removed them was the session ending.
#[test]
fn nothing_is_annotated_once_the_panel_is_closed() {
    let (mut engine, _) = stopped_engine("closed.rs");
    assert!(!engine.inline_values(0, 6).is_empty(), "shown while it is open");
    engine.app.dap.panel_open = false;
    assert!(engine.inline_values(0, 6).is_empty(), "and gone when it is not");
}

/// Not while running, and not in another file. A local called `count` means
/// nothing on a line of a file the program is not stopped in.
#[test]
fn nothing_is_annotated_away_from_the_stop() {
    let (mut engine, _) = stopped_engine("elsewhere.rs");
    engine.app.dap.current_path = Some("/somewhere/else/other.rs".into());
    assert!(engine.inline_values(0, 6).is_empty(), "another file");

    let (mut engine, _) = stopped_engine("running.rs");
    engine.app.dap.state = DapState::Running;
    assert!(engine.inline_values(0, 6).is_empty(), "not stopped");
}

/// Only the rows asked for. The band is a window and this follows it.
#[test]
fn only_the_rows_in_the_window_are_asked_about() {
    let (engine, _) = stopped_engine("window.rs");
    let rows: Vec<u32> = engine.inline_values(2, 2).into_iter().map(|(r, _)| r).collect();
    assert_eq!(rows, vec![2, 3], "rows 2 and 3, and row 1 is outside the window");
}
