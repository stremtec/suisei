//! The requested rule, as tests: a symbol declared inside function A must not
//! be offered while editing function C; a top-level symbol must be offered in
//! both.
//!
//! ```text
//! cargo test -p suisei-core --test scope_visibility
//! ```

use suisei_core::scope::{visible_at, ScopeLang, SymbolKind};
use tree_sitter::Parser;

fn parse(src: &str, lang: tree_sitter::Language) -> tree_sitter::Tree {
    let mut p = Parser::new();
    p.set_language(&lang).expect("grammar loads");
    p.parse(src, None).expect("parses")
}

/// Byte offset just after `needle`'s first occurrence — a stand-in for "the
/// caret is here".
fn caret(src: &str, needle: &str) -> usize {
    src.find(needle).unwrap_or_else(|| panic!("marker {needle:?} not in source")) + needle.len()
}

fn names_at(src: &str, marker: &str, lang: ScopeLang, ts: tree_sitter::Language) -> Vec<String> {
    let tree = parse(src, ts);
    visible_at(&tree, src, caret(src, marker), lang)
        .into_iter()
        .map(|s| s.name)
        .collect()
}

const RUST_SRC: &str = r#"
const LIMIT: usize = 10;

struct Config { size: usize }

fn helper(scale: usize) -> usize {
    let doubled = scale * 2;
    let a_only = doubled + 1;
    a_only
}

fn other(input: usize) -> usize {
    let c_only = input + 1;
    /*CARET_C*/
    c_only
}
"#;

#[test]
fn a_locals_are_invisible_from_another_function() {
    let names = names_at(
        RUST_SRC,
        "/*CARET_C*/",
        ScopeLang::Rust,
        tree_sitter_rust::LANGUAGE.into(),
    );

    // The whole point: nothing bound inside `helper` may appear here.
    for leaked in ["doubled", "a_only", "scale"] {
        assert!(
            !names.contains(&leaked.to_string()),
            "`{leaked}` is local to `helper` and must not be offered inside `other`; got {names:?}"
        );
    }

    // This function's own binding and parameter are in scope.
    for local in ["c_only", "input"] {
        assert!(
            names.contains(&local.to_string()),
            "`{local}` is local to `other` and must be offered; got {names:?}"
        );
    }
}

#[test]
fn top_level_items_are_visible_everywhere() {
    let names = names_at(
        RUST_SRC,
        "/*CARET_C*/",
        ScopeLang::Rust,
        tree_sitter_rust::LANGUAGE.into(),
    );
    for global in ["LIMIT", "Config", "helper", "other"] {
        assert!(
            names.contains(&global.to_string()),
            "`{global}` is top level and must be visible from any function; got {names:?}"
        );
    }
}

#[test]
fn a_function_sees_its_own_locals_and_not_its_siblings() {
    let src = RUST_SRC.replace("/*CARET_C*/", "").replace(
        "    a_only\n",
        "    /*CARET_A*/\n    a_only\n",
    );
    let names = names_at(
        &src,
        "/*CARET_A*/",
        ScopeLang::Rust,
        tree_sitter_rust::LANGUAGE.into(),
    );
    assert!(names.contains(&"doubled".to_string()), "got {names:?}");
    assert!(names.contains(&"scale".to_string()), "parameter; got {names:?}");
    assert!(
        !names.contains(&"c_only".to_string()),
        "`c_only` belongs to `other`; got {names:?}"
    );
}

#[test]
fn a_nested_block_keeps_its_bindings_to_itself() {
    let src = r#"
fn outer() {
    let visible = 1;
    if visible > 0 {
        let inner_only = 2;
        let _ = inner_only;
    }
    /*CARET*/
    let _ = visible;
}
"#;
    let names = names_at(
        src,
        "/*CARET*/",
        ScopeLang::Rust,
        tree_sitter_rust::LANGUAGE.into(),
    );
    assert!(names.contains(&"visible".to_string()), "got {names:?}");
    assert!(
        !names.contains(&"inner_only".to_string()),
        "a sibling block's binding must not escape it; got {names:?}"
    );
}

#[test]
fn kinds_and_global_flag_are_reported() {
    let tree = parse(RUST_SRC, tree_sitter_rust::LANGUAGE.into());
    let syms = visible_at(
        &tree,
        RUST_SRC,
        caret(RUST_SRC, "/*CARET_C*/"),
        ScopeLang::Rust,
    );
    let find = |n: &str| syms.iter().find(|s| s.name == n).cloned();

    let limit = find("LIMIT").expect("LIMIT visible");
    assert_eq!(limit.kind, SymbolKind::Constant);
    assert!(limit.global, "a top-level const is global");

    let local = find("c_only").expect("c_only visible");
    assert_eq!(local.kind, SymbolKind::Variable);
    assert!(!local.global, "a function-body binding is not global");

    let param = find("input").expect("input visible");
    assert_eq!(param.kind, SymbolKind::Parameter);
}

#[test]
fn python_locals_do_not_leak_between_functions() {
    let src = r#"
TOP = 1

def helper(scale):
    doubled = scale * 2
    return doubled

def other(value):
    mine = value + 1
    #CARET
    return mine
"#;
    let names = names_at(
        src,
        "#CARET",
        ScopeLang::Python,
        tree_sitter_python::LANGUAGE.into(),
    );
    assert!(names.contains(&"mine".to_string()), "got {names:?}");
    assert!(names.contains(&"TOP".to_string()), "module level; got {names:?}");
    assert!(names.contains(&"helper".to_string()), "module level; got {names:?}");
    assert!(
        !names.contains(&"doubled".to_string()),
        "`doubled` is local to `helper`; got {names:?}"
    );
}

#[test]
fn shadowing_prefers_the_nearest_binding() {
    let src = r#"
fn shadow() {
    let name = 1;
    {
        let name = 2;
        /*CARET*/
        let _ = name;
    }
}
"#;
    let tree = parse(src, tree_sitter_rust::LANGUAGE.into());
    let syms = visible_at(&tree, src, caret(src, "/*CARET*/"), ScopeLang::Rust);
    let hits: Vec<_> = syms.iter().filter(|s| s.name == "name").collect();
    assert_eq!(hits.len(), 1, "a shadowed name is offered once, not twice");
    assert_eq!(hits[0].depth, 0, "the innermost binding wins");
}
