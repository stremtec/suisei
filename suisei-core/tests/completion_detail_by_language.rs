//! What the completion popup writes beside a name.
//!
//! The reported problem: every buffer symbol showed `fn`, so a list that
//! should read
//!
//! ```text
//!   new         fn
//!   is_bright   method
//!   brightness  f32
//! ```
//!
//! read `fn` four times. Two separate facts were missing — whether a function
//! takes a receiver, and what type a binding was declared with.
//!
//! Note what is deliberately NOT here: `brightness` and `name` are struct
//! FIELDS. They are not in lexical scope for a bare identifier — writing
//! `brightness` inside a method does not compile, you write `self.brightness`
//! — so offering them in this list would suggest code that does not build.
//! Member completion after `.` needs the receiver's TYPE, which a parse tree
//! does not carry; that is the language server's job, not this walk's.
//!
//! ```text
//! cargo test -p suisei-core --test completion_detail_by_language
//! ```

use suisei_core::lang::Lang;
use suisei_core::scope::{ScopeSymbol, SymbolKind, visible_at};
use tree_sitter::Parser;

fn symbols(src: &str, lang: Lang) -> Vec<ScopeSymbol> {
    let byte = src.find("⟨here⟩").expect("source must mark the caret");
    let clean = src.replace("⟨here⟩", "");
    let mut p = Parser::new();
    p.set_language(&lang.grammar().0).expect("grammar loads");
    let tree = p.parse(&clean, None).expect("parses");
    visible_at(&tree, &clean, byte, lang.scope().expect("has a scope walk"))
}

fn detail_of(syms: &[ScopeSymbol], name: &str) -> String {
    syms.iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| {
            panic!(
                "{name:?} is not visible; got {:?}",
                syms.iter().map(|s| &s.name).collect::<Vec<_>>()
            )
        })
        .detail()
}

/// The exact example from the report.
const RUST_STAR: &str = "\
struct Star {
    name: String,
    brightness: f32,
}

impl Star {
    fn new(name: String) -> Self {
        Star { name, brightness: 1.0 }
    }

    fn is_bright(&self) -> bool {
        let cutoff: f32 = 1.0;
        ⟨here⟩
    }
}
";

#[test]
fn a_receiver_is_what_makes_a_function_a_method() {
    let syms = symbols(RUST_STAR, Lang::Rust);
    assert_eq!(
        detail_of(&syms, "is_bright"),
        "method",
        "it takes &self, so it is a method"
    );
    assert_eq!(
        detail_of(&syms, "new"),
        "fn",
        "an associated function declared in the same impl is still a function; \
         calling it a method would be the same imprecision pointed the other way"
    );
}

#[test]
fn a_binding_shows_its_declared_type() {
    let syms = symbols(RUST_STAR, Lang::Rust);
    assert_eq!(
        detail_of(&syms, "cutoff"),
        "f32",
        "an annotated let must show the annotation, not the word `let`"
    );
}

#[test]
fn struct_fields_are_not_offered_as_bare_identifiers() {
    let syms = symbols(RUST_STAR, Lang::Rust);
    for field in ["brightness", "name"] {
        assert!(
            !syms.iter().any(|s| s.name == field),
            "{field:?} is a field, reachable only through `self.` — offering it \
             here suggests code that does not compile. Member completion is the \
             language server's job."
        );
    }
}

#[test]
fn an_unannotated_binding_falls_back_to_its_kind() {
    let src = "fn foo() {\n    let loose = 1;\n    ⟨here⟩\n}\n";
    let syms = symbols(src, Lang::Rust);
    assert_eq!(
        detail_of(&syms, "loose"),
        "let",
        "with no annotation to show, the column must say what kind of thing it is"
    );
}

#[test]
fn parameters_show_their_type_in_every_annotated_language() {
    let cases: &[(Lang, &str)] = &[
        (Lang::Rust, "fn foo(limit: i32) { ⟨here⟩ }\n"),
        (
            Lang::TypeScript,
            "function foo(limit: number): void { ⟨here⟩ }\n",
        ),
        (Lang::Go, "package main\nfunc foo(limit int) { ⟨here⟩ }\n"),
        (Lang::C, "void foo(int limit) { ⟨here⟩ }\n"),
        (Lang::Cpp, "void foo(int limit) { ⟨here⟩ }\n"),
        (
            Lang::Java,
            "class T {\n  void foo(int limit) { ⟨here⟩ }\n}\n",
        ),
        (
            Lang::CSharp,
            "class T {\n  void Foo(int limit) { ⟨here⟩ }\n}\n",
        ),
    ];
    let want = ["i32", "number", "int", "int", "int", "int", "int"];
    let mut failures = Vec::new();
    for ((lang, src), want) in cases.iter().zip(want) {
        let got = detail_of(&symbols(src, *lang), "limit");
        if got != want {
            failures.push(format!("  {}: got {got:?}, want {want:?}", lang.name()));
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn languages_without_annotations_still_say_something_useful() {
    // Python, JavaScript, Ruby and Lua mostly carry no types. The column must
    // not go blank — it falls back to the kind.
    let cases: &[(Lang, &str, &str, &str)] = &[
        (
            Lang::Python,
            "def foo(limit):\n    li⟨here⟩\n",
            "limit",
            "param",
        ),
        (
            Lang::JavaScript,
            "function foo(limit) { ⟨here⟩ }\n",
            "limit",
            "param",
        ),
        (
            Lang::Ruby,
            "def foo(limit)\n  li⟨here⟩\nend\n",
            "limit",
            "param",
        ),
        (
            Lang::Lua,
            "local function foo(limit)\n  li⟨here⟩\nend\n",
            "limit",
            "param",
        ),
    ];
    let mut failures = Vec::new();
    for (lang, src, name, want) in cases {
        let got = detail_of(&symbols(src, *lang), name);
        if got != *want {
            failures.push(format!("  {}: got {got:?}, want {want:?}", lang.name()));
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn a_grammar_with_its_own_method_node_says_method() {
    // Go, Java, C#, JS/TS and Ruby all spell a method as a distinct node, so
    // they need no receiver check — but the popup must still say `method`.
    let cases: &[(Lang, &str, &str)] = &[
        (
            Lang::Go,
            "package main\ntype S struct{}\nfunc (s S) Ping() {}\nfunc main() { ⟨here⟩ }\n",
            "Ping",
        ),
        (
            Lang::Ruby,
            "class S\n  def ping\n  end\n\n  def other\n    p⟨here⟩\n  end\nend\n",
            "ping",
        ),
        (
            Lang::JavaScript,
            "class S {\n  ping() {}\n  other() { ⟨here⟩ }\n}\n",
            "ping",
        ),
    ];
    let mut failures = Vec::new();
    for (lang, src, name) in cases {
        let syms = symbols(src, *lang);
        let found = syms.iter().find(|s| s.name == *name);
        match found {
            Some(s) if s.kind == SymbolKind::Method => {}
            Some(s) => failures.push(format!("  {}: {name} is {:?}", lang.name(), s.kind)),
            None => failures.push(format!(
                "  {}: {name} not visible ({:?})",
                lang.name(),
                syms.iter().map(|s| &s.name).collect::<Vec<_>>()
            )),
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}
