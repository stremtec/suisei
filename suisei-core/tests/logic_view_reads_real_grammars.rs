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
