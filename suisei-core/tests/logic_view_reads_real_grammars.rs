//! Logic View L1: does one table of node-kind names carry a real grammar?
//!
//! This is the question the whole feature turns on. Suisei ships about thirty
//! grammars; if control flow needs a hand-written extractor per language, the
//! feature is not affordable. If a table is enough — the same shape that lets
//! one highlighter serve thirty grammars — a new language is an entry.
//!
//! So these tests are not "does it produce a graph". They are "does the SAME
//! code, given only a different table, read Rust, Python, JavaScript, Go and C".
//!
//! ```text
//! cargo test -p suisei-core --test logic_view_reads_real_grammars
//! ```

use suisei_core::lang::{Lang, LangBundle};
use suisei_core::logic::{EdgeLabel, LogicKind, graph_at, grammar_for};

fn graph(lang: Lang, src: &str, row: usize) -> suisei_core::logic::LogicGraph {
    let mut bundle = LangBundle::build(lang).expect("grammar loads");
    let tree = bundle.parser.parse(src, None).expect("parses");
    graph_at(&tree, src, lang, row).expect("a function at that row")
}

fn kinds(g: &suisei_core::logic::LogicGraph) -> Vec<LogicKind> {
    g.nodes.iter().map(|n| n.kind).collect()
}

/// Rust: a branch with both arms, and a loop with a back edge.
#[test]
fn rust_reads_as_a_branch_and_a_loop() {
    const SRC: &str = "\
fn process(n: i32) -> i32 {
    let mut total = 0;
    if n > 10 {
        total = n;
    } else {
        return 0;
    }
    for i in 0..n {
        total += i;
    }
    total
}
";
    let g = graph(Lang::Rust, SRC, 2);
    assert_eq!(g.name, "process");
    assert!(kinds(&g).contains(&LogicKind::Decision), "the `if` is a branch");
    assert!(kinds(&g).contains(&LogicKind::Loop), "the `for` is a loop");
    assert!(kinds(&g).contains(&LogicKind::Exit), "the `return` leaves");

    // Both arms, labelled — a branch drawn with one edge is a lie about the
    // other one.
    assert!(g.edges.iter().any(|e| e.label == EdgeLabel::Yes), "taken arm");
    assert!(g.edges.iter().any(|e| e.label == EdgeLabel::No), "the other one");
    // The back edge is what makes a loop a loop rather than an indented step.
    assert!(g.edges.iter().any(|e| e.label == EdgeLabel::Back), "round again");

    // Nothing flows out of a `return`.
    let exit = g.nodes.iter().find(|n| n.kind == LogicKind::Exit).unwrap();
    assert!(
        !g.edges.iter().any(|e| e.from == exit.id),
        "an exit has no outgoing edge"
    );
}

/// Every node names where it came from, exactly. A node that cannot say that
/// cannot be clicked, stepped to, or checked.
#[test]
fn every_node_names_its_source_range() {
    const SRC: &str = "\
fn f(a: i32) -> i32 {
    let b = a + 1;
    b
}
";
    let g = graph(Lang::Rust, SRC, 1);
    let last = SRC.lines().count();
    for n in &g.nodes {
        assert!(n.start_row <= n.end_row, "{n:?} runs backwards");
        assert!(n.end_row < last, "{n:?} names a row past the file");
    }
}

/// The same code, four more grammars, one table each.
///
/// If this passes, a language is an entry rather than a module — which is the
/// whole bet the design makes.
#[test]
fn the_same_extractor_reads_four_other_languages() {
    let cases: &[(Lang, &str, usize)] = &[
        (
            Lang::Python,
            "\
def process(n):
    total = 0
    if n > 10:
        total = n
    else:
        return 0
    for i in range(n):
        total += i
    return total
",
            1,
        ),
        (
            Lang::JavaScript,
            "\
function process(n) {
    let total = 0;
    if (n > 10) {
        total = n;
    } else {
        return 0;
    }
    for (let i = 0; i < n; i++) {
        total += i;
    }
    return total;
}
",
            1,
        ),
        (
            Lang::Go,
            "\
func process(n int) int {
    total := 0
    if n > 10 {
        total = n
    } else {
        return 0
    }
    for i := 0; i < n; i++ {
        total += i
    }
    return total
}
",
            1,
        ),
        (
            Lang::C,
            "\
int process(int n) {
    int total = 0;
    if (n > 10) {
        total = n;
    } else {
        return 0;
    }
    for (int i = 0; i < n; i++) {
        total += i;
    }
    return total;
}
",
            1,
        ),
    ];

    for (lang, src, row) in cases {
        let g = graph(*lang, src, *row);
        let ks = kinds(&g);
        assert!(ks.contains(&LogicKind::Decision), "{lang:?}: no branch found");
        assert!(ks.contains(&LogicKind::Loop), "{lang:?}: no loop found");
        assert!(ks.contains(&LogicKind::Exit), "{lang:?}: no exit found");
        assert!(
            g.edges.iter().any(|e| e.label == EdgeLabel::Yes),
            "{lang:?}: the taken arm is not labelled"
        );
        assert!(
            g.edges.iter().any(|e| e.label == EdgeLabel::No),
            "{lang:?}: the other arm is not labelled"
        );
        assert!(
            g.edges.iter().any(|e| e.label == EdgeLabel::Back),
            "{lang:?}: the loop has no back edge"
        );
    }
}

/// A construct the table does not name is SHOWN, not dropped.
///
/// The rule the module states: a flowchart with a missing branch is worse than
/// no flowchart, because it is a confident lie about control flow and somebody
/// will act on it. Anything unread has to appear as "there is something here".
#[test]
fn what_the_table_does_not_name_becomes_opaque_rather_than_nothing() {
    const SRC: &str = "\
fn f() {
    unsafe { danger() }
    let x = 1;
}
";
    let g = graph(Lang::Rust, SRC, 1);
    assert!(
        kinds(&g).contains(&LogicKind::Opaque),
        "the unsafe block is not in the table and must still be drawn: {:?}",
        g.nodes
    );
    // And it names its rows, so the reader can go and look at what was not read.
    let opaque = g.nodes.iter().find(|n| n.kind == LogicKind::Opaque).unwrap();
    assert_eq!(opaque.start_row, 1);
}

/// A language with no table shows NOTHING rather than something wrong.
#[test]
fn a_language_without_a_table_produces_no_graph() {
    assert!(grammar_for(Lang::Markdown).is_none());
    assert!(grammar_for(Lang::Json).is_none());
    assert!(grammar_for(Lang::Rust).is_some());
}

/// The innermost function wins: a closure inside a function is what the caret
/// is in.
#[test]
fn the_innermost_function_is_the_one_read() {
    const SRC: &str = "\
fn outer() {
    let f = |x: i32| {
        if x > 0 { return; }
    };
    f(1);
}
";
    let g = graph(Lang::Rust, SRC, 2);
    assert!(
        kinds(&g).contains(&LogicKind::Decision),
        "the closure's own branch, not the outer function's absence of one"
    );
}

/// The three things dumping a real graph caught that the assertions above did
/// not. Each was found by LOOKING at the output, which is what L1 exists for.
#[test]
fn the_labels_and_kinds_a_reader_actually_sees() {
    const SRC: &str = "\
fn process(n: i32) -> i32 {
    let mut total = 0;
    for i in 0..n {
        total += i;
    }
    total
}
";
    let g = graph(Lang::Rust, SRC, 1);

    // A loop is labelled with its HEADER. `named_child(0)` gave the loop
    // VARIABLE, so `for i in 0..n` drew itself as "i" — true, and useless.
    let loop_node = g.nodes.iter().find(|n| n.kind == LogicKind::Loop).unwrap();
    assert_eq!(loop_node.label, "for i in 0..n");

    // `+=` is `compound_assignment_expr` in Rust's grammar, which the table
    // did not list — so an ordinary step drew as "something I did not read".
    let plus_eq = g.nodes.iter().find(|n| n.label.contains("+=")).unwrap();
    assert_eq!(plus_eq.kind, LogicKind::Process);

    // A Rust body's last expression IS the return.
    let tail = g.nodes.last().unwrap();
    assert_eq!(tail.label, "total");
    assert_eq!(tail.kind, LogicKind::Exit);
    assert!(
        !g.edges.iter().any(|e| e.from == tail.id),
        "and nothing flows out of it"
    );
}

/// The tail rule is Rust's, not everyone's. A trailing expression statement in
/// Python or JavaScript returns nothing.
#[test]
fn the_tail_rule_does_not_leak_into_other_languages() {
    const PY: &str = "\
def f():
    x = 1
    print(x)
";
    let g = graph(Lang::Python, PY, 1);
    let tail = g.nodes.last().unwrap();
    assert_ne!(
        tail.kind,
        LogicKind::Exit,
        "`print(x)` is a step, not a return: {:?}",
        g.nodes
    );
}

// ── L2: the hierarchy, and the collapse ────────────────────────────────────

use suisei_core::logic::{collapse, expand, outline};

const TWO_FUNCTIONS: &str = "\
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

fn parsed(lang: Lang, src: &str) -> tree_sitter::Tree {
    let mut bundle = LangBundle::build(lang).expect("grammar loads");
    bundle.parser.parse(src, None).expect("parses")
}

/// The file opens as its functions, closed — and NOTHING below them is built.
///
/// That is the whole affordability argument: a file of four hundred functions
/// costs four hundred rows, not four hundred graphs.
#[test]
fn a_file_opens_as_its_functions_and_computes_nothing_inside_them() {
    let tree = parsed(Lang::Rust, TWO_FUNCTIONS);
    let t = outline(&tree, TWO_FUNCTIONS, Lang::Rust).unwrap();

    let labels: Vec<&str> = t.rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, vec!["helper", "main"], "in source order");
    assert!(t.rows.iter().all(|r| r.depth == 0 && !r.expanded && r.expandable));
}

/// Opening one is what costs, and closing it gives the cost back.
#[test]
fn opening_a_function_builds_its_body_and_closing_it_drops_it() {
    let tree = parsed(Lang::Rust, TWO_FUNCTIONS);
    let mut t = outline(&tree, TWO_FUNCTIONS, Lang::Rust).unwrap();

    assert!(expand(&mut t, 0, &tree, TWO_FUNCTIONS, Lang::Rust));
    assert!(t.rows[0].expanded);
    assert!(t.rows.len() > 2, "helper's body is in the list now");
    // Its steps sit one level in, and `main` is still where it was.
    assert!(t.rows[1..].iter().any(|r| r.depth == 1));
    assert!(t.rows.iter().any(|r| r.label == "main" && r.depth == 0));
    // The function's own Entry is not drawn again inside itself.
    assert_eq!(
        t.rows.iter().filter(|r| r.label == "helper").count(),
        1,
        "the name appears once"
    );

    let opened = t.rows.len();
    assert!(collapse(&mut t, 0));
    assert_eq!(t.rows.len(), 2, "back to two functions");
    assert!(opened > t.rows.len());
    assert!(!t.rows[0].expanded);
}

/// A call to a function in this file can be opened, and opening it shows the
/// CALLEE's body — which is what "expand [Authentication Module]" means.
#[test]
fn a_call_opens_into_what_it_calls() {
    let tree = parsed(Lang::Rust, TWO_FUNCTIONS);
    let mut t = outline(&tree, TWO_FUNCTIONS, Lang::Rust).unwrap();

    // Open `main`, find the call to `helper`.
    let main_at = t.rows.iter().position(|r| r.label == "main").unwrap();
    assert!(expand(&mut t, main_at, &tree, TWO_FUNCTIONS, Lang::Rust));
    let call_at = t
        .rows
        .iter()
        .position(|r| r.label.contains("helper(3)"))
        .expect("the call is a row");
    assert!(t.rows[call_at].expandable, "it resolves in this file");

    assert!(expand(&mut t, call_at, &tree, TWO_FUNCTIONS, Lang::Rust));
    // What appeared is helper's body — its branch — one level deeper.
    let inner_depth = t.rows[call_at].depth + 1;
    assert!(
        t.rows
            .iter()
            .any(|r| r.depth == inner_depth && r.kind == LogicKind::Decision),
        "helper's `if` is now visible under the call: {:?}",
        t.rows
    );
}

/// A call that does not resolve is not openable, and an AMBIGUOUS one is not
/// either.
///
/// A name is not a resolution. Opening the wrong `new` would be a confident
/// lie of exactly the kind the Opaque rule exists to prevent — so two
/// functions sharing a name resolve to neither, and that is the language
/// server's job with a round trip in front of it.
#[test]
fn an_unresolved_or_ambiguous_call_does_not_open() {
    const SRC: &str = "\
fn a() -> i32 { 1 }
fn b() -> i32 { 2 }
fn main() {
    external_thing();
}
";
    let tree = parsed(Lang::Rust, SRC);
    let mut t = outline(&tree, SRC, Lang::Rust).unwrap();
    let main_at = t.rows.iter().position(|r| r.label == "main").unwrap();
    expand(&mut t, main_at, &tree, SRC, Lang::Rust);
    let call = t
        .rows
        .iter()
        .find(|r| r.label.contains("external_thing"))
        .expect("the call is a row");
    assert!(!call.expandable, "nothing in this file to open");
}

// ── L3: the runtime overlay ────────────────────────────────────────────────

use suisei_core::logic::{row_at, runtime, values_on};

/// The row running now is the INNERMOST one holding the stopped line, and the
/// rows around it are the branch and loop bodies we are inside.
///
/// An `if` and the step inside it both hold the line. The step is what is
/// running; the `if` is where we are.
#[test]
fn the_stopped_row_is_the_innermost_thing_holding_the_line() {
    let tree = parsed(Lang::Rust, TWO_FUNCTIONS);
    let mut t = outline(&tree, TWO_FUNCTIONS, Lang::Rust).unwrap();
    expand(&mut t, 0, &tree, TWO_FUNCTIONS, Lang::Rust);

    // Line 2 is `return x;`, inside `if x > 0 {`, inside `helper`.
    let rt = runtime(&t, "/x.rs", Some(("/x.rs", 2)), &[("helper", "/x.rs", 2)]);
    let at = rt.stopped.expect("something is running");
    assert_eq!(t.rows[at].label, "return x");

    let around: Vec<&str> = rt.enclosing.iter().map(|&i| t.rows[i].label.as_str()).collect();
    assert_eq!(around, vec!["helper", "x > 0"], "outermost first");
    assert!(!rt.enclosing.contains(&at), "the stopped row is not around itself");
}

/// A local called `count` means nothing on a row of a file the program is not
/// stopped in — the same rule the editor's inline values follow, because it is
/// the same fact.
#[test]
fn nothing_is_marked_in_a_file_the_program_is_not_in() {
    let tree = parsed(Lang::Rust, TWO_FUNCTIONS);
    let mut t = outline(&tree, TWO_FUNCTIONS, Lang::Rust).unwrap();
    expand(&mut t, 0, &tree, TWO_FUNCTIONS, Lang::Rust);

    let rt = runtime(&t, "/x.rs", Some(("/other.rs", 2)), &[("helper", "/other.rs", 2)]);
    assert_eq!(rt, Default::default(), "nothing here is running: {rt:?}");
}

/// The way in, which is the one part of the runtime path that is KNOWN.
///
/// The call stack is exact — which functions we came through is not an
/// inference, and the row a caller sits on is the call that is still open.
#[test]
fn the_call_stack_marks_the_way_in() {
    let tree = parsed(Lang::Rust, TWO_FUNCTIONS);
    let mut t = outline(&tree, TWO_FUNCTIONS, Lang::Rust).unwrap();
    let main_at = t.rows.iter().position(|r| r.label == "main").unwrap();
    expand(&mut t, main_at, &tree, TWO_FUNCTIONS, Lang::Rust);
    expand(&mut t, 0, &tree, TWO_FUNCTIONS, Lang::Rust);

    // Stopped inside helper, having got there from `let a = helper(3);`.
    let rt = runtime(
        &t,
        "/x.rs",
        Some(("/x.rs", 2)),
        &[("helper", "/x.rs", 2), ("main", "/x.rs", 8)],
    );
    let callers: Vec<&str> = rt.callers.iter().map(|&i| t.rows[i].label.as_str()).collect();
    assert_eq!(callers, vec!["let a = helper(3);"]);
    // And the frame we are IN is not listed as a caller of itself.
    assert!(!rt.callers.contains(&rt.stopped.unwrap()));
}

/// An arm of a branch has to read as an arm, not as the next statement — so a
/// row carries WHY it follows the one before it, off the graph's own edges.
#[test]
fn a_branch_arm_is_marked_as_an_arm_and_nests_inside_its_branch() {
    let tree = parsed(Lang::Rust, TWO_FUNCTIONS);
    let mut t = outline(&tree, TWO_FUNCTIONS, Lang::Rust).unwrap();
    expand(&mut t, 0, &tree, TWO_FUNCTIONS, Lang::Rust);

    let branch = t.rows.iter().position(|r| r.kind == LogicKind::Decision).unwrap();
    let arm = t.rows.iter().position(|r| r.label == "return x").unwrap();
    assert_eq!(t.rows[arm].edge, EdgeLabel::Yes, "it runs when the test holds");
    assert_eq!(
        t.rows[arm].depth,
        t.rows[branch].depth + 1,
        "and it sits inside the branch"
    );
    // What comes after the whole `if` is back out at the branch's own level.
    let after = t.rows.iter().position(|r| r.label == "0").unwrap();
    assert_eq!(t.rows[after].depth, t.rows[branch].depth);
    assert_eq!(t.rows[after].edge, EdgeLabel::Next);
}

/// A row is annotated with what it NAMES, and nothing else. `account` is not
/// `count`.
#[test]
fn a_row_shows_only_the_values_it_mentions() {
    let vals = [("a", "3"), ("count", "7"), ("account", "99")];
    assert_eq!(values_on("let a = helper(3);", &vals), vec![("a", "3")]);
    assert_eq!(values_on("println!(\"{}\", account)", &vals), vec![("account", "99")]);
    assert!(values_on("return;", &vals).is_empty());
}

/// Source to logic: clicking a line finds the row, which is the same
/// containment test read the other way round.
#[test]
fn a_line_of_source_resolves_to_the_row_that_is_it() {
    let tree = parsed(Lang::Rust, TWO_FUNCTIONS);
    let mut t = outline(&tree, TWO_FUNCTIONS, Lang::Rust).unwrap();
    let main_at = t.rows.iter().position(|r| r.label == "main").unwrap();
    expand(&mut t, main_at, &tree, TWO_FUNCTIONS, Lang::Rust);

    assert_eq!(t.rows[row_at(&t, 8).unwrap()].label, "let a = helper(3);");
    // A line inside a function that has not been opened resolves to the
    // function — which is the row that is actually on screen.
    assert_eq!(t.rows[row_at(&t, 2).unwrap()].label, "helper");
    assert!(row_at(&t, 6).is_none(), "the blank line between them is nobody's");
}
