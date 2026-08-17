//! Logic View, L1: one function's control flow, read off the syntax tree.
//!
//! feature.txt ☐26 and `docs/SUISEI-LOGIC-VIEW-PLAN.md`. This is the first
//! stage and the one that answers the risk: **does a per-language table carry
//! a real grammar, or does every language need its own extractor?** Suisei
//! ships about thirty grammars, so the answer decides whether the feature is
//! affordable at all.
//!
//! What this is NOT, yet: the hierarchy (Project → Module → File), the runtime
//! overlay, or the AI's Logic Diff. It is one function, statically.
//!
//! ## Two rules that make it trustworthy
//!
//! **Every node names its exact source range.** A node that cannot say where
//! it came from cannot be clicked, cannot be stepped to, and cannot be edited
//! through — and it cannot be checked.
//!
//! **Anything the table does not name becomes [`LogicKind::Opaque`], never
//! nothing.** A flowchart with a missing branch is worse than no flowchart: it
//! is a confident lie about control flow, and someone will act on it. A
//! grammar this has never seen, a macro that expands to a loop, a construct
//! the table does not cover — all of them have to show up as "there is
//! something here I did not read".

use crate::lang::Lang;
use tree_sitter::Node;

/// What a node in the logic does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicKind {
    /// Where the function begins.
    Entry,
    /// An ordinary step: a binding, an assignment, an expression.
    Process,
    /// A branch. Its outgoing edges are labelled.
    Decision,
    /// A loop header. Has a back edge.
    Loop,
    /// Leaves the function — `return`, `?`, `throw`.
    Exit,
    /// Something the table does not name.
    ///
    /// Shown rather than dropped, always. See the module note.
    Opaque,
}

/// Why one node leads to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeLabel {
    /// Plain sequence.
    Next,
    /// The condition held.
    Yes,
    /// It did not.
    No,
    /// Round again.
    Back,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicNode {
    pub id: usize,
    pub kind: LogicKind,
    /// What to write in the box. Source text, trimmed to one line.
    pub label: String,
    /// 0-based, inclusive. The whole point of §"two rules".
    pub start_row: usize,
    pub end_row: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicEdge {
    pub from: usize,
    pub to: usize,
    pub label: EdgeLabel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogicGraph {
    pub nodes: Vec<LogicNode>,
    pub edges: Vec<LogicEdge>,
    /// The function this is of, for the header.
    pub name: String,
}

impl LogicGraph {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// The node kinds a grammar spells its control flow with.
///
/// **This is the bet.** If a table of node-kind names is enough, a new
/// language is an entry here rather than a module — the same shape that lets
/// one highlighter serve thirty grammars, where a capture name maps onto a
/// `TokenKind`.
///
/// Names come from each grammar's own `node-types.json`, so they are the
/// grammar's vocabulary and not an invented one.
#[derive(Debug, Clone, Copy)]
pub struct LogicGrammar {
    /// A function, method or closure — where a graph starts.
    pub function: &'static [&'static str],
    /// The name field to read off a function node.
    pub name_field: &'static str,
    /// Branches. Their condition is the first named child.
    pub decision: &'static [&'static str],
    /// Loops. Drawn with a back edge.
    pub loops: &'static [&'static str],
    /// Leaves the function.
    pub exit: &'static [&'static str],
    /// A container whose children are the real steps — walked THROUGH rather
    /// than drawn.
    pub block: &'static [&'static str],
    /// An ordinary step.
    pub process: &'static [&'static str],
    /// The branch taken when a decision holds, and the one when it does not.
    /// Field names on the decision node.
    pub consequence: &'static str,
    pub alternative: &'static str,
    /// The last expression of a body IS the return value.
    ///
    /// True of Rust and of nothing else in the table. Without it, `total` on
    /// the last line of a Rust function came out `Opaque` — which was honest
    /// (the table did not name it) and wrong (it is the return).
    pub tail_is_return: bool,
}

/// The table for `lang`, or `None` where the language has no logic to read —
/// JSON and Markdown have no control flow to be wrong about.
pub fn grammar_for(lang: Lang) -> Option<LogicGrammar> {
    Some(match lang {
        Lang::Rust => LogicGrammar {
            function: &["function_item", "closure_expression"],
            name_field: "name",
            decision: &["if_expression", "match_expression"],
            loops: &["for_expression", "while_expression", "loop_expression"],
            exit: &["return_expression", "break_expression", "continue_expression"],
            block: &["block", "expression_statement", "declaration_list"],
            // `compound_assignment_expr` is Rust's name for `+=`. It was
            // missing, so `total += i` drew as Opaque — found by dumping a
            // real graph rather than by an assertion, which is exactly what
            // L1 is for.
            process: &[
                "let_declaration",
                "assignment_expression",
                "compound_assignment_expr",
                "call_expression",
                "macro_invocation",
            ],
            consequence: "consequence",
            alternative: "alternative",
            tail_is_return: true,
        },
        Lang::Python => LogicGrammar {
            function: &["function_definition", "lambda"],
            name_field: "name",
            decision: &["if_statement", "match_statement", "try_statement"],
            loops: &["for_statement", "while_statement"],
            exit: &["return_statement", "break_statement", "continue_statement", "raise_statement"],
            block: &["block", "expression_statement"],
            process: &["assignment", "augmented_assignment", "call"],
            consequence: "consequence",
            alternative: "alternative",
            tail_is_return: false,
        },
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => LogicGrammar {
            function: &[
                "function_declaration",
                "function_expression",
                "arrow_function",
                "method_definition",
            ],
            name_field: "name",
            decision: &["if_statement", "switch_statement", "try_statement"],
            loops: &["for_statement", "for_in_statement", "while_statement", "do_statement"],
            exit: &["return_statement", "break_statement", "continue_statement", "throw_statement"],
            block: &["statement_block", "expression_statement"],
            process: &["lexical_declaration", "variable_declaration", "assignment_expression", "call_expression"],
            consequence: "consequence",
            alternative: "alternative",
            tail_is_return: false,
        },
        Lang::Go => LogicGrammar {
            function: &["function_declaration", "method_declaration", "func_literal"],
            name_field: "name",
            decision: &["if_statement", "type_switch_statement", "expression_switch_statement"],
            loops: &["for_statement"],
            exit: &["return_statement", "break_statement", "continue_statement"],
            block: &["block", "expression_statement"],
            process: &["short_var_declaration", "var_declaration", "assignment_statement", "call_expression"],
            consequence: "consequence",
            alternative: "alternative",
            tail_is_return: false,
        },
        Lang::C | Lang::Cpp => LogicGrammar {
            function: &["function_definition", "lambda_expression"],
            name_field: "declarator",
            decision: &["if_statement", "switch_statement"],
            loops: &["for_statement", "while_statement", "do_statement"],
            exit: &["return_statement", "break_statement", "continue_statement"],
            block: &["compound_statement", "expression_statement"],
            process: &["declaration", "assignment_expression", "call_expression"],
            consequence: "consequence",
            alternative: "alternative",
            tail_is_return: false,
        },
        // Not "no logic" — "no table yet". A language reaching this arm shows
        // nothing rather than something wrong, and adding it is an entry above.
        _ => return None,
    })
}

/// The innermost function node containing `row`, if any.
pub fn function_at<'t>(tree: &'t tree_sitter::Tree, g: &LogicGrammar, row: usize) -> Option<Node<'t>> {
    let mut cursor = tree.root_node().walk();
    let mut found: Option<Node<'t>> = None;
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if n.start_position().row > row || n.end_position().row < row {
            continue;
        }
        if g.function.contains(&n.kind()) {
            // Innermost wins: a closure inside a function is the thing the
            // caret is in.
            found = Some(match found {
                Some(prev) if prev.byte_range().len() <= n.byte_range().len() => prev,
                _ => n,
            });
        }
        for c in n.named_children(&mut cursor) {
            stack.push(c);
        }
    }
    found
}

/// Build the graph for the function containing `row`.
///
/// `src` must be the text the tree was parsed from — a tree indexed against
/// stale text names the wrong ranges, which is the same pairing `live_tree`
/// hands back together for the same reason.
pub fn graph_at(tree: &tree_sitter::Tree, src: &str, lang: Lang, row: usize) -> Option<LogicGraph> {
    let g = grammar_for(lang)?;
    let func = function_at(tree, &g, row)?;
    let mut b = Builder {
        g: &g,
        src,
        graph: LogicGraph {
            name: func
                .child_by_field_name(g.name_field)
                .and_then(|n| n.utf8_text(src.as_bytes()).ok())
                .unwrap_or("fn")
                .lines()
                .next()
                .unwrap_or("fn")
                .trim()
                .to_string(),
            ..Default::default()
        },
    };
    let entry = b.push(LogicKind::Entry, "Start".into(), func);
    // The body is the last block-shaped child — every grammar in the table
    // spells it differently ("body", "block", "compound_statement"), and
    // finding it by shape is one rule instead of a field name per language.
    let body = func
        .child_by_field_name("body")
        .or_else(|| last_block(func, &g))
        .unwrap_or(func);
    let tails = b.walk_block(body, &[entry]);
    let _ = tails;
    Some(b.graph)
}

fn last_block<'t>(n: Node<'t>, g: &LogicGrammar) -> Option<Node<'t>> {
    let mut cursor = n.walk();
    n.named_children(&mut cursor)
        .filter(|c| g.block.contains(&c.kind()))
        .last()
}

/// Whether a node kind is a statement rather than an expression.
///
/// Rust's tail rule is about EXPRESSIONS: `let x = 1;` at the end of a body
/// returns nothing, and `x` does. Every grammar in the table names its
/// statements with the suffix, which is a convention rather than a guarantee —
/// so this is a heuristic, and being wrong makes a node a Process instead of
/// an Exit rather than inventing a branch.
fn is_statement(kind: &str) -> bool {
    kind.ends_with("_statement") || kind.ends_with("_declaration") || kind.ends_with("_item")
}

struct Builder<'a> {
    g: &'a LogicGrammar,
    src: &'a str,
    graph: LogicGraph,
}

impl Builder<'_> {
    fn push(&mut self, kind: LogicKind, label: String, n: Node<'_>) -> usize {
        let id = self.graph.nodes.len();
        self.graph.nodes.push(LogicNode {
            id,
            kind,
            label,
            start_row: n.start_position().row,
            end_row: n.end_position().row,
        });
        id
    }

    fn link(&mut self, from: &[usize], to: usize, label: EdgeLabel) {
        for &f in from {
            self.graph.edges.push(LogicEdge { from: f, to, label });
        }
    }

    /// One line of source for a node's box.
    ///
    /// The SOURCE, not a paraphrase. A generated description would be a second
    /// thing that can disagree with the code; the code is what the reader is
    /// being helped to understand.
    fn label(&self, n: Node<'_>) -> String {
        let text = n.utf8_text(self.src.as_bytes()).unwrap_or("");
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.chars().count() <= 48 {
            return flat;
        }
        let cut: String = flat.chars().take(47).collect();
        format!("{cut}…")
    }

    /// The text of `n` up to where `body` starts — a loop's or branch's header.
    fn header(&self, n: Node<'_>, body: Node<'_>) -> String {
        let start = n.start_byte();
        let end = body.start_byte().max(start).min(self.src.len());
        let text = self.src.get(start..end).unwrap_or("");
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let flat = flat.trim_end_matches(['{', ':', '(']).trim().to_string();
        if flat.chars().count() <= 48 {
            return flat;
        }
        let cut: String = flat.chars().take(47).collect();
        format!("{cut}…")
    }

    /// Walk a block's statements in order, threading the live ends through.
    ///
    /// Returns the ends that fall out of the bottom — empty when every path
    /// left through a `return`, which is how a function with no fallthrough
    /// says so.
    fn walk_block(&mut self, block: Node<'_>, incoming: &[usize]) -> Vec<usize> {
        let mut cursor = block.walk();
        let kids: Vec<Node<'_>> = block.named_children(&mut cursor).collect();
        let last = kids.len().saturating_sub(1);
        let mut live: Vec<usize> = incoming.to_vec();
        for (i, child) in kids.iter().enumerate() {
            live = self.walk_stmt(*child, &live);
            // In a language where the last expression IS the return, say so.
            // Only when the walk drew ONE plain node for it — a tail that is
            // itself a branch already ends in its own arms' exits.
            if self.g.tail_is_return && i == last && live.len() == 1 {
                let id = live[0];
                let n = &mut self.graph.nodes[id];
                if matches!(n.kind, LogicKind::Process | LogicKind::Opaque)
                    && !is_statement(child.kind())
                {
                    n.kind = LogicKind::Exit;
                    live = Vec::new();
                }
            }
        }
        live
    }

    fn walk_stmt(&mut self, n: Node<'_>, incoming: &[usize]) -> Vec<usize> {
        let kind = n.kind();
        let g = self.g;

        if g.decision.contains(&kind) {
            let cond = n
                .child_by_field_name("condition")
                .or_else(|| n.named_child(0))
                .unwrap_or(n);
            let id = self.push(LogicKind::Decision, self.label(cond), n);
            self.link(incoming, id, EdgeLabel::Next);

            let mut tails = Vec::new();
            match n.child_by_field_name(g.consequence) {
                Some(yes) => tails.extend(self.branch(yes, id, EdgeLabel::Yes)),
                // A decision whose taken branch cannot be found still HAS one:
                // the reader must not be shown a branch that goes nowhere.
                None => tails.push(id),
            }
            match n.child_by_field_name(g.alternative) {
                Some(no) => tails.extend(self.branch(no, id, EdgeLabel::No)),
                None => tails.push(id),
            }
            return tails;
        }

        if g.loops.contains(&kind) {
            let body = n
                .child_by_field_name("body")
                .or_else(|| last_block(n, g))
                .unwrap_or(n);
            // Everything before the body. `named_child(0)` gave the loop
            // VARIABLE — `for i in 0..n` labelled itself "i" — and a
            // condition field does not exist on a `for`. The header is the
            // part that says what the loop does, and it is whatever precedes
            // the body whatever the grammar calls its pieces.
            let id = self.push(LogicKind::Loop, self.header(n, body), n);
            self.link(incoming, id, EdgeLabel::Next);
            let ends = self.walk_block(body, &[id]);
            // Round again. The back edge is what makes a loop a loop rather
            // than a step that happens to be indented.
            self.link(&ends, id, EdgeLabel::Back);
            return vec![id];
        }

        if g.exit.contains(&kind) {
            let id = self.push(LogicKind::Exit, self.label(n), n);
            self.link(incoming, id, EdgeLabel::Next);
            // Nothing flows out of an exit.
            return Vec::new();
        }

        if g.block.contains(&kind) {
            // A container, not a step. `expression_statement` wraps the thing
            // that matters in most of these grammars, so walking through it
            // rather than drawing it is what keeps `foo();` one box.
            let mut cursor = n.walk();
            let inner: Vec<Node<'_>> = n.named_children(&mut cursor).collect();
            if inner.len() == 1 {
                return self.walk_stmt(inner[0], incoming);
            }
            return self.walk_block(n, incoming);
        }

        let kind_out = if g.process.contains(&kind) {
            LogicKind::Process
        } else {
            // The rule from the module note: named, or visibly not.
            LogicKind::Opaque
        };
        let id = self.push(kind_out, self.label(n), n);
        self.link(incoming, id, EdgeLabel::Next);
        vec![id]
    }

    fn branch(&mut self, n: Node<'_>, from: usize, label: EdgeLabel) -> Vec<usize> {
        // The edge carries the label; the first node of the branch receives it.
        let before = self.graph.nodes.len();
        let ends = self.walk_block(n, &[from]);
        // Re-label the edges that left the decision into this branch.
        for e in self.graph.edges.iter_mut() {
            if e.from == from && e.to >= before && e.label == EdgeLabel::Next {
                e.label = label;
            }
        }
        ends
    }
}

// ── L2: the hierarchy, and the collapse ────────────────────────────────────
//
// The collapse is load-bearing twice. It is what makes a hundred-file program
// readable, and it is what makes it COMPUTABLE: a whole-project graph is every
// function in every file and it invalidates on every keystroke. Computing
// nothing below a closed node turns that into a per-function job on a tree
// that is already parsed and already incremental.
//
// So `outline` builds the shallowest thing that is useful — the file's
// functions, closed — and `expand` is the only thing that ever builds a graph.

/// A row of the hierarchy. Flattened with a depth, the same shape the
/// Variables tree uses, and for the same reason: a list is what a view draws
/// and a depth is what makes it a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicRow {
    pub kind: LogicKind,
    pub label: String,
    pub start_row: usize,
    pub end_row: usize,
    pub depth: usize,
    /// There is something inside this that has not been built.
    pub expandable: bool,
    pub expanded: bool,
    /// For a call that resolves: the row its target function starts on.
    ///
    /// Same file only, at this stage. Crossing a file needs a definition
    /// lookup, which is the same mechanism with a round trip in front of it.
    pub target_row: Option<usize>,
}

/// The file's logic, as deep as it has been opened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogicTree {
    pub rows: Vec<LogicRow>,
}

/// Every function in the file, closed.
///
/// Nothing below this is computed. That is the point: opening one function is
/// what costs, and a file of four hundred functions costs four hundred rows
/// rather than four hundred graphs.
pub fn outline(tree: &tree_sitter::Tree, src: &str, lang: Lang) -> Option<LogicTree> {
    let g = grammar_for(lang)?;
    let mut rows = Vec::new();
    let mut cursor = tree.root_node().walk();
    let mut stack: Vec<Node<'_>> = tree.root_node().named_children(&mut cursor).collect();
    stack.reverse();
    while let Some(n) = stack.pop() {
        if g.function.contains(&n.kind()) {
            rows.push(LogicRow {
                kind: LogicKind::Entry,
                label: function_label(n, &g, src),
                start_row: n.start_position().row,
                end_row: n.end_position().row,
                depth: 0,
                expandable: true,
                expanded: false,
                target_row: None,
            });
            // Not into a function: its body is what expanding it is FOR.
            continue;
        }
        let mut c = n.walk();
        let kids: Vec<Node<'_>> = n.named_children(&mut c).collect();
        for k in kids.into_iter().rev() {
            stack.push(k);
        }
    }
    rows.sort_by_key(|r| r.start_row);
    Some(LogicTree { rows })
}

fn function_label(n: Node<'_>, g: &LogicGrammar, src: &str) -> String {
    n.child_by_field_name(g.name_field)
        .and_then(|x| x.utf8_text(src.as_bytes()).ok())
        .unwrap_or("fn")
        .lines()
        .next()
        .unwrap_or("fn")
        .trim()
        .to_string()
}

/// Open the row at `index`, building what is inside it.
///
/// The ONLY thing that builds a graph. Returns false when there was nothing to
/// open — a row already open, a call that resolves nowhere, an index past the
/// end.
pub fn expand(t: &mut LogicTree, index: usize, tree: &tree_sitter::Tree, src: &str, lang: Lang) -> bool {
    let Some(row) = t.rows.get(index) else { return false };
    if row.expanded || !row.expandable {
        return false;
    }
    let Some(g) = grammar_for(lang) else { return false };
    // A call opens where its target is DEFINED, so the body that appears is
    // the callee's — which is what "expand [Authentication Module]" means.
    let at = row.target_row.unwrap_or(row.start_row);
    let Some(graph) = graph_at(tree, src, lang, at) else { return false };

    let depth = t.rows[index].depth + 1;
    let functions = function_starts(tree, &g, src);
    let inserted: Vec<LogicRow> = graph
        .nodes
        .iter()
        // The graph's own Entry is this row; drawing it again would be the
        // function's name twice, once inside itself.
        .filter(|n| n.kind != LogicKind::Entry)
        .map(|n| {
            let target = call_target(&n.label, &functions);
            LogicRow {
                kind: n.kind,
                label: n.label.clone(),
                start_row: n.start_row,
                end_row: n.end_row,
                depth,
                expandable: target.is_some(),
                expanded: false,
                target_row: target,
            }
        })
        .collect();
    t.rows[index].expanded = true;
    let after = index + 1;
    t.rows.splice(after..after, inserted);
    true
}

/// Close the row at `index`, dropping everything under it.
pub fn collapse(t: &mut LogicTree, index: usize) -> bool {
    let Some(depth) = t.rows.get(index).map(|r| r.depth) else { return false };
    if !t.rows[index].expanded {
        return false;
    }
    let mut end = index + 1;
    while end < t.rows.len() && t.rows[end].depth > depth {
        end += 1;
    }
    t.rows.drain(index + 1..end);
    t.rows[index].expanded = false;
    true
}

/// Every function in the file, by name, with the row it starts on.
fn function_starts(tree: &tree_sitter::Tree, g: &LogicGrammar, src: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut cursor = tree.root_node().walk();
    let mut stack: Vec<Node<'_>> = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if g.function.contains(&n.kind()) {
            let name = function_label(n, g, src);
            if !name.is_empty() {
                out.push((name, n.start_position().row));
            }
        }
        for c in n.named_children(&mut cursor) {
            stack.push(c);
        }
    }
    out
}

/// The function a step calls, if it is one in this file.
///
/// By NAME, off the label, which is source text. That is a deliberate limit
/// and it is why this only claims same-file: a name is not a resolution, and
/// two functions can share one. Crossing a file — and being right about which
/// `new` was meant — is the language server's job, and it is the same
/// mechanism with a round trip in front of it.
fn call_target(label: &str, functions: &[(String, usize)]) -> Option<usize> {
    // The identifier immediately before the first `(`.
    //
    // Not "everything before the `(`": a step's label is the whole statement,
    // so `let a = helper(3);` would have resolved as `let a = helper`. Walking
    // back over identifier characters from the paren is what picks the name
    // out of `let a = helper(`, `self.helper(` and `foo::bar(` alike.
    let open = label.find('(')?;
    let head = &label[..open];
    let name = head
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map(|(i, _)| &head[i..])?
        .trim();
    if name.is_empty() {
        return None;
    }
    let mut hits = functions.iter().filter(|(n, _)| n == name);
    let first = hits.next()?;
    // Ambiguous is not resolved. Opening the wrong `new` would be a confident
    // lie of exactly the kind the Opaque rule exists to prevent.
    if hits.next().is_some() {
        return None;
    }
    Some(first.1)
}
