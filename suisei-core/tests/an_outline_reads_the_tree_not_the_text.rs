//! U5: the outline comes off the syntax tree, like everything else does.
//!
//! It used to be a line-by-line string match — `trimmed.strip_prefix("fn ")`
//! and sixteen friends — which is wrong in ways a parser cannot be:
//!
//!   · a `class` named in a COMMENT was a symbol;
//!   · a signature broken across lines was invisible, because the line the
//!     declaration starts on begins with an argument;
//!   · nesting was not modelled at all, so a method and its class were
//!     siblings;
//!   · and it stopped at 200 items, because a scan of every line in a big file
//!     had no cheaper way to bound itself.
//!
//! The tree already exists — the logic graph is built from it — so this is one
//! source of truth reused, not a second parser.
//!
//! ```text
//! cargo test -p suisei-core --test an_outline_reads_the_tree_not_the_text
//! ```

use suisei_core::lang::Lang;
use suisei_core::logic;

/// Parse `src` and read its outline, or fail the test saying why.
fn outline(src: &str, lang: Lang) -> Vec<logic::OutlineRow> {
    let mut parser = tree_sitter::Parser::new();
    let (language, _) = lang.grammar();
    parser.set_language(&language).expect("set language");
    let tree = parser.parse(src, None).expect("parse");
    logic::file_outline(&tree, src, lang).expect("a logic table for this language")
}

fn names(rows: &[logic::OutlineRow]) -> Vec<&str> {
    rows.iter().map(|r| r.name.as_str()).collect()
}

#[test]
fn a_declaration_in_a_comment_is_not_a_symbol() {
    // The headline failure of the string match. Both lines below begin with
    // exactly the text it looked for.
    let src = "\
// fn ghost_one() is described here
/* class GhostTwo — also only a comment */
fn real() {}
";
    assert_eq!(names(&outline(src, Lang::Rust)), vec!["real"]);
}

#[test]
fn a_declaration_inside_a_string_is_not_a_symbol() {
    let src = "\
fn real() {
    let sample = \"fn ghost() {}\";
    let _ = sample;
}
";
    assert_eq!(names(&outline(src, Lang::Rust)), vec!["real"]);
}

#[test]
fn a_signature_broken_across_lines_is_still_found() {
    // The line this declaration STARTS on is `fn`, but the old scan matched on
    // a trimmed line and this one keeps working; the case that broke it is the
    // argument-first continuation, which a tree does not care about at all.
    let src = "\
fn wrapped(
    first: usize,
    second: usize,
) -> usize {
    first + second
}
";
    let rows = outline(src, Lang::Rust);
    assert_eq!(names(&rows), vec!["wrapped"]);
    assert_eq!(rows[0].row, 0, "reported on the line it starts on");
}

#[test]
fn a_method_is_nested_under_the_type_that_owns_it() {
    let src = "\
struct Thing {
    field: usize,
}

impl Thing {
    fn method(&self) {}
}
";
    let rows = outline(src, Lang::Rust);
    let method = rows.iter().find(|r| r.name == "method").expect("method");
    let thing = rows.iter().find(|r| r.kind == 2).expect("a container");
    assert!(method.depth > thing.depth, "{rows:#?}");
}

#[test]
fn a_type_and_a_function_are_told_apart() {
    let src = "struct Alpha;\nfn beta() {}\n";
    let rows = outline(src, Lang::Rust);
    assert_eq!(rows.iter().find(|r| r.name == "Alpha").map(|r| r.kind), Some(2));
    assert_eq!(rows.iter().find(|r| r.name == "beta").map(|r| r.kind), Some(1));
}

#[test]
fn an_impl_block_is_named_for_its_type_not_for_the_word_fn() {
    // `impl` has no `name` field. Falling through to the function label's
    // default would have listed every impl in a file as "fn".
    let src = "struct Thing;\nimpl Thing {\n    fn a(&self) {}\n}\n";
    let rows = outline(src, Lang::Rust);
    assert!(
        rows.iter().all(|r| r.name != "fn"),
        "an unnamed row got the placeholder: {rows:#?}"
    );
    assert!(rows.iter().filter(|r| r.name == "Thing").count() >= 2, "{rows:#?}");
}

#[test]
fn rows_come_back_in_document_order() {
    let src = "fn c() {}\nstruct B;\nfn a() {}\n";
    let rows = outline(src, Lang::Rust);
    let mut sorted = rows.clone();
    sorted.sort_by_key(|r| r.row);
    assert_eq!(rows, sorted);
}

#[test]
fn there_is_no_two_hundred_item_cap() {
    // The old scan stopped at 200 and a file with more declarations simply
    // ended, with nothing saying so.
    let src: String = (0..250).map(|i| format!("fn f{i}() {{}}\n")).collect();
    let rows = outline(&src, Lang::Rust);
    assert_eq!(rows.len(), 250, "got {}", rows.len());
}

#[test]
fn python_classes_hold_their_methods() {
    let src = "\
class Thing:
    def method(self):
        pass

def free():
    pass
";
    let rows = outline(src, Lang::Python);
    let method = rows.iter().find(|r| r.name == "method").expect("method");
    let free = rows.iter().find(|r| r.name == "free").expect("free");
    assert!(method.depth > free.depth, "{rows:#?}");
}

#[test]
fn a_language_with_no_table_declines_rather_than_guessing() {
    // JSON has no control flow and no declarations. The caller keeps its own
    // handling — Markdown headings are not declarations either.
    let mut parser = tree_sitter::Parser::new();
    let (language, _) = Lang::Json.grammar();
    parser.set_language(&language).expect("set language");
    let src = "{\"a\": 1}";
    let tree = parser.parse(src, None).expect("parse");
    assert!(logic::file_outline(&tree, src, Lang::Json).is_none());
}
