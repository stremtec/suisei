//! Scope-aware completion, driven the way the GUI drives it.
//!
//! Every existing test for this feature calls `app.syntax.parse(...)` first and
//! then asks for completions. That is the TUI path. The GUI never calls it: it
//! ships a text snapshot to the syntax worker and adopts the answer through
//! `apply_frame`, which used to end with
//!
//! ```text
//! self.tree = None;
//! self.last_text.clear();
//! ```
//!
//! so `live_tree()` was `None` for the entire life of the app,
//! `symbols_in_scope_at_caret` returned an empty vector on every keystroke, and
//! completion silently degraded to language keywords. The unit and e2e suites
//! all passed the whole time, because none of them went through the worker.
//!
//! This test goes through `Engine` — worker, frame adoption and all — so a
//! regression in the transport fails here even while the scope walker is fine.
//!
//! ```text
//! cargo test -p suisei-engine --test completion_through_the_gui_path
//! ```

use suisei_core::app::App;
use suisei_core::buffer::Position;
use suisei_core::selection::{Selection, SelectionSet};
use suisei_engine::Engine;

const SRC: &str = "\
const SHARED_LIMIT: usize = 10;

fn scaled_helper(scale_factor: usize) -> usize {
    let scaled_inner = scale_factor * 2;
    scaled_inner
}

fn other_place(input_value: usize) -> usize {
    let scoped_local = input_value + 1;
    scoped_local
}
";

/// An engine holding `SRC` as a real file, parsed the way the app parses —
/// through the worker, then flushed so the frame is adopted.
fn engine_with_source() -> (Engine, std::path::PathBuf) {
    let dir = std::env::temp_dir().join("suisei_gui_completion");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("scope_demo.rs");
    std::fs::write(&path, SRC).expect("write source");

    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = App::open_file(path.to_str().unwrap());
    // The worker is asynchronous; the GUI adopts frames on the next tick. This
    // is the same wait, made explicit.
    engine.flush_syntax();
    (engine, path)
}

fn labels(engine: &Engine) -> Vec<String> {
    engine
        .app
        .completions
        .suggestions
        .iter()
        .map(|s| s.label.clone())
        .collect()
}

/// Put the caret at the end of `line`'s content and type `prefix`, exactly as
/// keystrokes — which is what makes completion fire.
fn type_prefix(engine: &mut Engine, line_containing: &str, prefix: &str) {
    let row = SRC
        .lines()
        .position(|l| l.contains(line_containing))
        .expect("anchor line exists");
    let col = SRC.lines().nth(row).unwrap().chars().count();
    let at = Position { row, col };
    engine.app.buffer.cursor = at;
    engine.app.sel = SelectionSet::single(Selection::caret(at));

    // A fresh line, so the prefix is its own statement rather than appended to
    // existing code — then let the worker catch up. That mirrors the real
    // sequence: the tree trails the buffer by a frame, and by the time a couple
    // of characters of an identifier are typed it is current except for those
    // characters, on the same row.
    engine.app.gui_insert_newline("    ");
    engine.flush_syntax();

    for ch in prefix.chars() {
        engine.gui_type_char(ch);
    }
}

#[test]
fn the_live_tree_survives_a_worker_frame() {
    let (engine, _path) = engine_with_source();
    // The narrow fact the feature rests on, asserted on its own so a failure
    // here is not mistaken for a scope-walker bug.
    assert!(
        engine.app.syntax.live_tree().is_some(),
        "after adopting a worker frame the main thread must hold the parse — \
         without it every structural query silently returns nothing"
    );
    assert_eq!(
        engine.app.syntax.live_ext(),
        "rs",
        "and must know what language it parsed"
    );
}

#[test]
fn typing_in_one_function_offers_its_locals_not_the_others() {
    let (mut engine, _path) = engine_with_source();
    type_prefix(&mut engine, "let scoped_local", "sc");

    let got = labels(&engine);
    assert!(
        got.iter().any(|l| l == "scoped_local"),
        "this function's own local must be offered, got {got:?}"
    );
    // A top-level item, reachable from inside any function. `SHARED_LIMIT` is
    // also top-level but cannot appear here — the prefix is "sc" and it starts
    // "SH", which is the prefix filter doing its job, not a scope failure.
    assert!(
        got.iter().any(|l| l == "scaled_helper"),
        "a top-level item must be offered, got {got:?}"
    );
    assert!(
        !got.iter().any(|l| l == "scaled_inner"),
        "the OTHER function's local must not be offered, got {got:?}"
    );
}

#[test]
fn parameters_are_offered_inside_their_own_function_only() {
    let (mut engine, _path) = engine_with_source();
    type_prefix(&mut engine, "let scoped_local", "in");

    let got = labels(&engine);
    assert!(
        got.iter().any(|l| l == "input_value"),
        "this function's parameter must be offered, got {got:?}"
    );
    assert!(
        !got.iter().any(|l| l == "scale_factor"),
        "the other function's parameter must not be, got {got:?}"
    );
}
