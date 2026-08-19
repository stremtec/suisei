//! L5: a Logic Diff is a view of a REAL patch, not a patch derived from a view.
//!
//! §5 of the plan inverts the obvious direction on purpose:
//!
//! > **Generate the source edit; RENDER it as logic.**
//!
//! Deriving source from an approved logic change reviews an abstraction and
//! then applies a *translation* of it — and any ambiguity in that translation
//! means the diff the user approved and the diff that lands are two different
//! things. So the diff runs the same extractor over the before and after TEXT,
//! and whatever it shows, the patch is already the thing that will be written.
//!
//! ```text
//! cargo test -p suisei-core --test a_patch_can_be_read_as_logic
//! ```

use suisei_core::lang::Lang;
use suisei_core::logic::{self, LogicChange, LogicGraph};

/// The logic of the first function in `src` — the extractor the view uses.
fn graph(src: &str) -> LogicGraph {
    let mut parser = tree_sitter::Parser::new();
    let (language, _) = Lang::Rust.grammar();
    parser.set_language(&language).expect("set language");
    let tree = parser.parse(src, None).expect("parse");
    logic::graph_at(&tree, src, Lang::Rust, 1).expect("a graph")
}

fn changes(before: &str, after: &str) -> Vec<LogicChange> {
    logic::diff(&graph(before), &graph(after))
}

#[test]
fn an_unchanged_function_has_no_logic_diff() {
    let src = "fn f() {\n    let a = 1;\n    let b = 2;\n    g(a, b);\n}\n";
    assert!(changes(src, src).is_empty(), "{:?}", changes(src, src));
}

#[test]
fn a_reformat_that_changes_no_logic_shows_no_logic() {
    // The property that makes a Logic Diff worth reading: whitespace and
    // line breaks are not logic, so they do not appear as logic.
    let before = "fn f() {\n    let a = 1;\n    g(a);\n}\n";
    let after = "fn f() {\n    let a = 1;\n\n\n    g(a);\n}\n";
    let d = changes(before, after);
    assert!(
        d.iter().all(|c| matches!(c, LogicChange::Moved { .. })),
        "a reformat produced added/removed steps: {d:?}"
    );
}

#[test]
fn an_added_step_reads_as_added() {
    let before = "fn f() {\n    let a = 1;\n    g(a);\n}\n";
    let after = "fn f() {\n    let a = 1;\n    let b = 2;\n    g(a);\n}\n";
    let d = changes(before, after);
    assert!(
        d.iter().any(|c| matches!(c, LogicChange::Added(_))),
        "no addition reported: {d:?}"
    );
    assert!(
        !d.iter().any(|c| matches!(c, LogicChange::Removed(_))),
        "an insertion reported a removal: {d:?}"
    );
}

#[test]
fn a_deleted_step_reads_as_removed() {
    let before = "fn f() {\n    let a = 1;\n    let b = 2;\n    g(a);\n}\n";
    let after = "fn f() {\n    let a = 1;\n    g(a);\n}\n";
    let d = changes(before, after);
    assert!(
        d.iter().any(|c| matches!(c, LogicChange::Removed(_))),
        "no removal reported: {d:?}"
    );
    assert!(
        !d.iter().any(|c| matches!(c, LogicChange::Added(_))),
        "a deletion reported an addition: {d:?}"
    );
}

#[test]
fn a_new_branch_reads_as_added_control_flow() {
    let before = "fn f(x: i32) {\n    g(x);\n}\n";
    let after = "fn f(x: i32) {\n    if x > 0 {\n        g(x);\n    }\n}\n";
    let d = changes(before, after);
    assert!(
        d.iter().any(|c| matches!(c, LogicChange::Added(_))),
        "wrapping a call in a branch added nothing: {d:?}"
    );
}

#[test]
fn two_identical_steps_pair_with_the_right_partners() {
    // The reason matching is a longest common subsequence and not a first-hit
    // lookup: a function with the same call twice would otherwise pair both
    // befores with the same after and report a phantom removal.
    let before = "fn f() {\n    g();\n    g();\n}\n";
    let after = "fn f() {\n    g();\n    h();\n    g();\n}\n";
    let d = changes(before, after);
    assert_eq!(
        d.iter().filter(|c| matches!(c, LogicChange::Removed(_))).count(),
        0,
        "inserting between two identical calls reported a removal: {d:?}"
    );
    assert_eq!(
        d.iter().filter(|c| matches!(c, LogicChange::Added(_))).count(),
        1,
        "{d:?}"
    );
}

#[test]
fn an_empty_side_is_all_of_the_other_side() {
    let g = graph("fn f() {\n    let a = 1;\n    h(a);\n}\n");
    let empty = LogicGraph::default();

    let added = logic::diff(&empty, &g);
    assert_eq!(added.len(), g.nodes.len());
    assert!(added.iter().all(|c| matches!(c, LogicChange::Added(_))));

    let removed = logic::diff(&g, &empty);
    assert_eq!(removed.len(), g.nodes.len());
    assert!(removed.iter().all(|c| matches!(c, LogicChange::Removed(_))));
}

#[test]
fn two_empty_graphs_are_no_change() {
    assert!(logic::diff(&LogicGraph::default(), &LogicGraph::default()).is_empty());
}
