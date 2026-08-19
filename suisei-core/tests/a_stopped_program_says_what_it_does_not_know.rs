//! L4: which nodes ran, and which ones only might have.
//!
//! A debugger stopped at a line knows **where the program is, not how it got
//! there**. The call stack is recorded; the branch history is not. So the only
//! thing the editor can assert about the route is dominance:
//!
//!   · a node on EVERY path from the entry to the stopped node ran — there was
//!     no way to be here without having been there;
//!   · a node on SOME path might have run;
//!   · a node from which no path reaches the stopped node did not.
//!
//! Drawing the second the same as the first is the failure this exists to
//! prevent. A reader who sees a highlighted `else` arm and takes it for fact
//! goes looking for the bug in the wrong half of the function.
//!
//! ```text
//! cargo test -p suisei-core --test a_stopped_program_says_what_it_does_not_know
//! ```

use suisei_core::logic::{Certainty, EdgeLabel, LogicEdge, LogicGraph, LogicKind, LogicNode};

fn node(id: usize, label: &str) -> LogicNode {
    LogicNode {
        id,
        kind: LogicKind::Process,
        label: label.into(),
        start_row: id,
        end_row: id,
    }
}

fn edge(from: usize, to: usize) -> LogicEdge {
    LogicEdge { from, to, label: EdgeLabel::Next }
}

/// ```text
///        0 entry
///        │
///        1 if
///       ╱ ╲
///      2   3      ← the two arms
///       ╲ ╱
///        4 join
/// ```
fn diamond() -> LogicGraph {
    LogicGraph {
        nodes: (0..5).map(|i| node(i, &format!("n{i}"))).collect(),
        edges: vec![
            edge(0, 1),
            edge(1, 2),
            edge(1, 3),
            edge(2, 4),
            edge(3, 4),
        ],
        name: "f".into(),
    }
}

fn at(graph: &LogicGraph, stopped: usize) -> Vec<(usize, Certainty)> {
    let mut v = suisei_core::logic::certainty(graph, stopped);
    v.sort_by_key(|(id, _)| *id);
    v
}

#[test]
fn everything_before_a_branch_is_certain() {
    // Stopped in the left arm. The entry and the `if` had to run.
    let c = at(&diamond(), 2);
    assert_eq!(c[0].1, Certainty::Certain, "entry");
    assert_eq!(c[1].1, Certainty::Certain, "the branch itself");
    assert_eq!(c[2].1, Certainty::Certain, "the arm we are standing in");
}

#[test]
fn the_arm_we_are_not_in_did_not_run() {
    let c = at(&diamond(), 2);
    assert_eq!(c[3].1, Certainty::Unreached, "the other arm");
}

#[test]
fn what_comes_after_has_not_run_yet() {
    let c = at(&diamond(), 2);
    assert_eq!(c[4].1, Certainty::Unreached, "the join is still ahead");
}

#[test]
fn past_a_join_neither_arm_is_certain() {
    // THE case L4 exists for. Stopped at the join, both arms could have got us
    // here and the debugger cannot say which — so neither may be drawn as fact.
    let c = at(&diamond(), 4);
    assert_eq!(c[2].1, Certainty::Inferred, "left arm");
    assert_eq!(c[3].1, Certainty::Inferred, "right arm");
    // And the things that were not in doubt stay out of doubt.
    assert_eq!(c[0].1, Certainty::Certain, "entry");
    assert_eq!(c[1].1, Certainty::Certain, "the branch");
    assert_eq!(c[4].1, Certainty::Certain, "where we are standing");
}

#[test]
fn a_node_is_certain_of_itself() {
    for stop in 0..5 {
        let c = at(&diamond(), stop);
        assert_eq!(c[stop].1, Certainty::Certain, "stopped at {stop}");
    }
}

#[test]
fn a_straight_line_is_all_certain_behind_and_all_unreached_ahead() {
    let graph = LogicGraph {
        nodes: (0..4).map(|i| node(i, "s")).collect(),
        edges: vec![edge(0, 1), edge(1, 2), edge(2, 3)],
        name: "f".into(),
    };
    let c = at(&graph, 2);
    assert_eq!(
        c.iter().map(|(_, x)| *x).collect::<Vec<_>>(),
        vec![
            Certainty::Certain,
            Certainty::Certain,
            Certainty::Certain,
            Certainty::Unreached,
        ]
    );
}

#[test]
fn a_loop_body_is_inferred_from_after_the_loop() {
    // ```text
    // 0 → 1 (head) → 2 (body) → 1
    //          ↓
    //          3 (after)
    // ```
    // Stopped after the loop: the body may have run zero times. Saying it ran
    // would be a guess dressed as a fact.
    let graph = LogicGraph {
        nodes: (0..4).map(|i| node(i, "l")).collect(),
        edges: vec![edge(0, 1), edge(1, 2), edge(2, 1), edge(1, 3)],
        name: "f".into(),
    };
    let c = at(&graph, 3);
    assert_eq!(c[1].1, Certainty::Certain, "the loop head is unavoidable");
    assert_eq!(c[2].1, Certainty::Inferred, "the body may never have run");
}

#[test]
fn an_empty_graph_and_an_unknown_node_answer_nothing() {
    assert!(suisei_core::logic::certainty(&LogicGraph::default(), 0).is_empty());
    assert!(
        suisei_core::logic::certainty(&diamond(), 99).is_empty(),
        "a node the graph does not have is not a stop location"
    );
}

#[test]
fn a_node_unreachable_from_the_entry_is_never_certain() {
    // An orphan with an edge INTO the stopped node. It reaches the stop, so it
    // is not `Unreached` — but it cannot be `Certain`, because nothing reaches
    // IT from the entry, so the program cannot have come through it.
    let mut graph = diamond();
    graph.nodes.push(node(9, "orphan"));
    graph.edges.push(edge(9, 4));
    let c = at(&graph, 4);
    let orphan = c.iter().find(|(id, _)| *id == 9).expect("orphan");
    assert_ne!(orphan.1, Certainty::Certain, "{c:?}");
}
