//! The scope rules, asserted for EVERY language that claims to support them.
//!
//! The suite was once Rust-weighted: one Python case and nothing for
//! JavaScript, TypeScript, C or Go, even though `ScopeLang` claimed all six. A
//! per-language table is the only way to know whether "supported" means the
//! rules hold or merely that `from_ext` returns `Some` — Python passed its one
//! test while offering no locals and no parameters at all, because the test
//! asserted an absence and an empty list satisfies it.
//!
//! The rules under test, one function each:
//!
//! 1. a function's own local is visible inside it
//! 2. another function's local is NOT
//! 3. a parameter is visible only inside its function
//! 4. a nested block's binding stays in that block
//! 5. top-level items are visible everywhere
//! 6. shadowing prefers the nearest binding
//!
//! Cases name a [`Lang`], not a grammar function: the grammar and the
//! `ScopeLang` both come from `crate::lang`, so a case cannot test one
//! language's tree with another language's rules.
//!
//! ```text
//! cargo test -p suisei-core --test scope_language_conformance
//! ```

use suisei_core::lang::Lang;
use suisei_core::scope::{ScopeLang, visible_at};
use tree_sitter::Parser;

fn parse(src: &str, lang: Lang) -> tree_sitter::Tree {
    let mut p = Parser::new();
    p.set_language(&lang.grammar().0)
        .unwrap_or_else(|e| panic!("{} grammar loads: {e}", lang.name()));
    p.parse(src, None)
        .unwrap_or_else(|| panic!("{} parses", lang.name()))
}

/// `⟨here⟩` marks the caret. Removed before parsing, so the source stays valid
/// — a marker left in place changes the tree that is being asked about.
fn at_marker(src: &str) -> (String, usize) {
    let byte = src.find("⟨here⟩").expect("source must mark the caret");
    (src.replace("⟨here⟩", ""), byte)
}

fn scope_of(lang: Lang) -> ScopeLang {
    lang.scope()
        .unwrap_or_else(|| panic!("{} claims scope support", lang.name()))
}

fn names(src: &str, lang: Lang) -> Vec<String> {
    let (clean, byte) = at_marker(src);
    let tree = parse(&clean, lang);
    visible_at(&tree, &clean, byte, scope_of(lang))
        .into_iter()
        .map(|s| s.name)
        .collect()
}

/// Every language's source for one rule, so a gap shows up as a named failure
/// rather than as an absent test.
struct Case {
    lang: Lang,
    src: &'static str,
}

/// Run `check` over every case, reporting ALL failures rather than the first —
/// otherwise fixing one language hides the next.
fn each(cases: &[Case], check: impl Fn(&[String]) -> Result<(), String>) {
    let mut failures = Vec::new();
    for case in cases {
        let got = names(case.src, case.lang);
        if let Err(why) = check(&got) {
            failures.push(format!(
                "  {}: {why}\n    visible: {got:?}",
                case.lang.name()
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

// ---------------------------------------------------------------- rule 1 & 2

#[test]
fn own_locals_visible_siblings_locals_not() {
    let cases = [
        Case {
            lang: Lang::Rust,
            src: "fn foo() { let local_a = 1; }\nfn bar() { let local_b = 2; ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Python,
            // Python's body must contain a statement, so the caret follows a
            // partial identifier rather than sitting on a blank line — which is
            // what completion actually sees anyway.
            src: "def foo():\n    local_a = 1\n\ndef bar():\n    local_b = 2\n    loc⟨here⟩\n",
        },
        Case {
            lang: Lang::JavaScript,
            src: "function foo() { let local_a = 1; }\nfunction bar() { let local_b = 2; ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::TypeScript,
            src: "function foo(): void { let local_a = 1; }\nfunction bar(): void { let local_b = 2; ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::C,
            src: "void foo(void) { int local_a = 1; }\nvoid bar(void) { int local_b = 2; ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Cpp,
            src: "void foo() { int local_a = 1; }\nvoid bar() { int local_b = 2; ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Go,
            src: "package main\nfunc foo() { local_a := 1 }\nfunc bar() { local_b := 2; ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Java,
            src: "class T {\n  void foo() { int local_a = 1; }\n  void bar() { int local_b = 2; ⟨here⟩ }\n}\n",
        },
        Case {
            lang: Lang::CSharp,
            src: "class T {\n  void Foo() { int local_a = 1; }\n  void Bar() { int local_b = 2; ⟨here⟩ }\n}\n",
        },
        Case {
            lang: Lang::Ruby,
            src: "def foo\n  local_a = 1\nend\n\ndef bar\n  local_b = 2\n  loc⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Lua,
            src: "local function foo()\n  local local_a = 1\nend\n\nlocal function bar()\n  local local_b = 2\n  loc⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Swift,
            src: "func foo() { let local_a = 1 }\nfunc bar() { let local_b = 2\n  loc⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Php,
            src: "<?php\nfunction foo() { $local_a = 1; }\nfunction bar() { $local_b = 2; ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Zig,
            src: "fn foo() void {\n    const local_a = 1;\n}\nfn bar() void {\n    const local_b = 2;\n    _ = local_b;\n    ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Dart,
            src: "void foo() { var local_a = 1; }\nvoid bar() { var local_b = 2; ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::ObjC,
            src: "void foo(void) { int local_a = 1; }\nvoid bar(void) { int local_b = 2; ⟨here⟩ }\n",
        },
    ];
    each(&cases, |got| {
        has_local(got, "local_b")?;
        lacks_local(got, "local_a")
    });
}

/// PHP variables carry their sigil (`$local_b`) because that is what the user
/// types and what must be inserted. Compare on the bare name so one table of
/// sources works for every language.
fn bare(name: &str) -> &str {
    name.strip_prefix('$').unwrap_or(name)
}

fn has_local(got: &[String], name: &str) -> Result<(), String> {
    if got.iter().any(|n| bare(n) == name) {
        Ok(())
    } else {
        Err(format!("expected {name:?} to be visible"))
    }
}

fn lacks_local(got: &[String], name: &str) -> Result<(), String> {
    if got.iter().any(|n| bare(n) == name) {
        Err(format!("{name:?} must NOT be visible here"))
    } else {
        Ok(())
    }
}

// -------------------------------------------------------------------- rule 3

#[test]
fn a_parameter_is_visible_only_inside_its_function() {
    let cases = [
        Case {
            lang: Lang::Rust,
            src: "fn foo(only_here: i32) {}\nfn bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Python,
            src: "def foo(only_here):\n    pass\n\ndef bar():\n    ⟨here⟩\n",
        },
        Case {
            lang: Lang::JavaScript,
            src: "function foo(only_here) {}\nfunction bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::TypeScript,
            src: "function foo(only_here: number): void {}\nfunction bar(): void { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::C,
            src: "void foo(int only_here) {}\nvoid bar(void) { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Cpp,
            src: "void foo(int only_here) {}\nvoid bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Go,
            src: "package main\nfunc foo(only_here int) {}\nfunc bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Java,
            src: "class T {\n  void foo(int only_here) {}\n  void bar() { ⟨here⟩ }\n}\n",
        },
        Case {
            lang: Lang::CSharp,
            src: "class T {\n  void Foo(int only_here) {}\n  void Bar() { ⟨here⟩ }\n}\n",
        },
        Case {
            lang: Lang::Ruby,
            src: "def foo(only_here)\n  nil\nend\n\ndef bar\n  ⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Lua,
            src: "local function foo(only_here)\nend\n\nlocal function bar()\n  ⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Swift,
            src: "func foo(only_here: Int) {}\nfunc bar() {\n  ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Php,
            src: "<?php\nfunction foo($only_here) {}\nfunction bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Zig,
            src: "fn foo(only_here: i32) void {}\nfn bar() void {\n    ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Dart,
            src: "void foo(int only_here) {}\nvoid bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::ObjC,
            src: "void foo(int only_here) {}\nvoid bar(void) { ⟨here⟩ }\n",
        },
    ];
    each(&cases, |got| lacks_local(got, "only_here"));
}

#[test]
fn a_parameter_is_visible_in_its_own_body() {
    let cases = [
        Case {
            lang: Lang::Rust,
            src: "fn foo(the_param: i32) { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Python,
            src: "def foo(the_param):\n    th⟨here⟩\n",
        },
        Case {
            lang: Lang::JavaScript,
            src: "function foo(the_param) { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::TypeScript,
            src: "function foo(the_param: number): void { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::C,
            src: "void foo(int the_param) { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Cpp,
            src: "void foo(int the_param) { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Go,
            src: "package main\nfunc foo(the_param int) { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Java,
            src: "class T {\n  void foo(int the_param) { ⟨here⟩ }\n}\n",
        },
        Case {
            lang: Lang::CSharp,
            src: "class T {\n  void Foo(int the_param) { ⟨here⟩ }\n}\n",
        },
        Case {
            lang: Lang::Ruby,
            src: "def foo(the_param)\n  th⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Lua,
            src: "local function foo(the_param)\n  th⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Swift,
            src: "func foo(the_param: Int) {\n  th⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Php,
            src: "<?php\nfunction foo($the_param) { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Zig,
            src: "fn foo(the_param: i32) void {\n    ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Dart,
            src: "void foo(int the_param) { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::ObjC,
            src: "void foo(int the_param) { ⟨here⟩ }\n",
        },
    ];
    each(&cases, |got| has_local(got, "the_param"));
}

// -------------------------------------------------------------------- rule 4

#[test]
fn a_nested_block_keeps_its_binding_to_itself() {
    let cases = [
        Case {
            lang: Lang::Rust,
            src: "fn foo() {\n    { let inner = 1; }\n    ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::JavaScript,
            src: "function foo() {\n    { let inner = 1; }\n    ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::TypeScript,
            src: "function foo(): void {\n    { let inner = 1; }\n    ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::C,
            src: "void foo(void) {\n    { int inner = 1; }\n    ⟨here⟩;\n}\n",
        },
        Case {
            lang: Lang::Cpp,
            src: "void foo() {\n    { int inner = 1; }\n    ⟨here⟩;\n}\n",
        },
        Case {
            lang: Lang::Go,
            src: "package main\nfunc foo() {\n    { inner := 1 }\n    ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Java,
            src: "class T {\n  void foo() {\n    { int inner = 1; }\n    ⟨here⟩;\n  }\n}\n",
        },
        Case {
            lang: Lang::CSharp,
            src: "class T {\n  void Foo() {\n    { int inner = 1; }\n    ⟨here⟩;\n  }\n}\n",
        },
        Case {
            lang: Lang::Lua,
            src: "local function foo()\n  do local inner = 1 end\n  ⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Dart,
            src: "void foo() {\n    { var inner = 1; }\n    ⟨here⟩;\n}\n",
        },
        Case {
            lang: Lang::ObjC,
            src: "void foo(void) {\n    { int inner = 1; }\n    ⟨here⟩;\n}\n",
        },
        // Python and Ruby have no block scope — a name bound in an `if` IS
        // visible after it. Asserting otherwise would be asserting a bug.
        // PHP is the same: `if` and `for` do not introduce a scope.
        // Swift's `do { }` does scope, but its grammar spells the body the same
        // as a function's, which this walk already covers via rule 1.
    ];
    each(&cases, |got| lacks_local(got, "inner"));
}

// -------------------------------------------------------------------- rule 5

#[test]
fn top_level_items_are_visible_from_inside_a_function() {
    let cases = [
        Case {
            lang: Lang::Rust,
            src: "const SHARED_LIMIT: i32 = 10;\nfn foo() {}\nfn bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Python,
            src: "SHARED_LIMIT = 10\n\ndef foo():\n    pass\n\ndef bar():\n    ⟨here⟩\n",
        },
        Case {
            lang: Lang::JavaScript,
            src: "const SHARED_LIMIT = 10;\nfunction foo() {}\nfunction bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::TypeScript,
            src: "const SHARED_LIMIT = 10;\nfunction foo(): void {}\nfunction bar(): void { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::C,
            src: "int SHARED_LIMIT = 10;\nvoid foo(void) {}\nvoid bar(void) { ⟨here⟩; }\n",
        },
        Case {
            lang: Lang::Cpp,
            src: "int SHARED_LIMIT = 10;\nvoid foo() {}\nvoid bar() { ⟨here⟩; }\n",
        },
        Case {
            lang: Lang::Go,
            src: "package main\nvar SHARED_LIMIT = 10\nfunc foo() {}\nfunc bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Java,
            src: "class T {\n  static int SHARED_LIMIT = 10;\n  void foo() {}\n  void bar() { ⟨here⟩; }\n}\n",
        },
        Case {
            lang: Lang::CSharp,
            src: "class T {\n  static int SHARED_LIMIT = 10;\n  void foo() {}\n  void bar() { ⟨here⟩; }\n}\n",
        },
        Case {
            lang: Lang::Ruby,
            src: "SHARED_LIMIT = 10\n\ndef foo\nend\n\ndef bar\n  ⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Lua,
            src: "local SHARED_LIMIT = 10\nlocal function foo() end\nlocal function bar()\n  ⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Swift,
            src: "let SHARED_LIMIT = 10\nfunc foo() {}\nfunc bar() {\n  ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Php,
            src: "<?php\n$SHARED_LIMIT = 10;\nfunction foo() {}\nfunction bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::Zig,
            src: "const SHARED_LIMIT = 10;\nfn foo() void {}\nfn bar() void {\n    ⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Dart,
            src: "var SHARED_LIMIT = 10;\nvoid foo() {}\nvoid bar() { ⟨here⟩ }\n",
        },
        Case {
            lang: Lang::ObjC,
            src: "int SHARED_LIMIT = 10;\nvoid foo(void) {}\nvoid bar(void) { ⟨here⟩; }\n",
        },
    ];
    each(&cases, |got| {
        has_local(got, "SHARED_LIMIT")?;
        has_local(got, "foo")?;
        has_local(got, "bar")
    });
}

// -------------------------------------------------------------------- rule 6

#[test]
fn shadowing_prefers_the_nearest_binding() {
    // Both bindings may be listed; what matters is that the INNER one is
    // offered first, since that is the one the name resolves to.
    let cases = [
        Case {
            lang: Lang::Rust,
            src: "fn foo() {\n    let dup = 1;\n    {\n        let dup = 2;\n        ⟨here⟩\n    }\n}\n",
        },
        Case {
            lang: Lang::JavaScript,
            src: "function foo() {\n    let dup = 1;\n    {\n        let dup = 2;\n        ⟨here⟩\n    }\n}\n",
        },
        Case {
            lang: Lang::C,
            src: "void foo(void) {\n    int dup = 1;\n    {\n        int dup = 2;\n        ⟨here⟩;\n    }\n}\n",
        },
        Case {
            lang: Lang::Cpp,
            src: "void foo() {\n    int dup = 1;\n    {\n        int dup = 2;\n        ⟨here⟩;\n    }\n}\n",
        },
        Case {
            lang: Lang::Go,
            src: "package main\nfunc foo() {\n    dup := 1\n    {\n        dup := 2\n        ⟨here⟩\n    }\n}\n",
        },
        Case {
            lang: Lang::Java,
            src: "class T {\n  void foo() {\n    int dup = 1;\n    {\n      int dup = 2;\n      ⟨here⟩;\n    }\n  }\n}\n",
        },
        Case {
            lang: Lang::Lua,
            src: "local function foo()\n  local dup = 1\n  do\n    local dup = 2\n    ⟨here⟩\n  end\nend\n",
        },
    ];
    let mut failures = Vec::new();
    for case in cases {
        let (clean, byte) = at_marker(case.src);
        let tree = parse(&clean, case.lang);
        let syms = visible_at(&tree, &clean, byte, scope_of(case.lang));
        let dups: Vec<_> = syms.iter().filter(|s| bare(&s.name) == "dup").collect();
        if dups.is_empty() {
            failures.push(format!(
                "  {}: \"dup\" not visible at all",
                case.lang.name()
            ));
            continue;
        }
        // Nearest first: depth ascending, and the first `dup` reached must be
        // the innermost one.
        if dups[0].depth != dups.iter().map(|s| s.depth).min().unwrap() {
            failures.push(format!(
                "  {}: nearest binding is not offered first (depths {:?})",
                case.lang.name(),
                dups.iter().map(|s| s.depth).collect::<Vec<_>>()
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

// ------------------------------------------------------- extension coverage

/// Every extension the parser builds a tree for either resolves to a
/// `ScopeLang` or belongs to a language that says, in `Lang::scope`, why it
/// does not.
///
/// This is the check that caught C++: `syntax.rs` parsed `cpp`/`hpp`/`cc`/
/// `cxx`/`hh`/`hxx` while `ScopeLang::from_ext` did not know those spellings,
/// so the files highlighted and silently offered no buffer symbols. `pyi`,
/// `mts` and `cts` were missing the same way. Both tables are now the one in
/// `crate::lang`, and this asserts the delegation actually holds end to end.
#[test]
fn every_parsed_extension_agrees_with_its_language() {
    let mut wrong = Vec::new();
    for lang in Lang::ALL {
        for ext in lang.extensions() {
            if ScopeLang::from_ext(ext) != lang.scope() {
                wrong.push(format!(
                    "  .{ext}: ScopeLang::from_ext -> {:?}, {} -> {:?}",
                    ScopeLang::from_ext(ext),
                    lang.name(),
                    lang.scope()
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}\n", wrong.join("\n"));
}

/// A language claiming scope support must actually produce symbols.
///
/// "Supported" once meant `from_ext` returned `Some`, and Python satisfied its
/// single test by returning nothing at all. Any `ScopeLang` that answers with
/// an empty list for an obviously-populated file is making the same claim.
#[test]
fn no_language_claims_scope_support_and_returns_nothing() {
    let mut empty = Vec::new();
    for lang in Lang::ALL {
        if lang.scope().is_none() {
            continue;
        }
        let src = match lang {
            Lang::Php => "<?php\nfunction the_fn($the_param) { $the_local = 1; }\n",
            Lang::Ruby => "def the_fn(the_param)\n  the_local = 1\nend\n",
            Lang::Lua => "local function the_fn(the_param)\n  local the_local = 1\nend\n",
            Lang::Swift => "func the_fn(the_param: Int) {\n  let the_local = 1\n}\n",
            Lang::Python => "def the_fn(the_param):\n    the_local = 1\n",
            Lang::Go => "package main\nfunc the_fn(the_param int) {\n\tthe_local := 1\n}\n",
            Lang::Java | Lang::CSharp => {
                "class T {\n  void the_fn(int the_param) { int the_local = 1; }\n}\n"
            }
            Lang::Rust => "fn the_fn(the_param: i32) { let the_local = 1; }\n",
            Lang::Zig => "fn the_fn(the_param: i32) void {\n    const the_local = 1;\n}\n",
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
                "function the_fn(the_param) { let the_local = 1; }\n"
            }
            _ => "void the_fn(int the_param) { int the_local = 1; }\n",
        };
        let tree = parse(src, *lang);
        // Just before the closing brace of the declaring construct — the
        // innermost scope. Not end-of-file: in Java and C# a method lives in a
        // class body and is genuinely NOT in scope at file level, so asking
        // there would be asserting the wrong rule rather than a missing one.
        let at = src.trim_end().len().saturating_sub(1);
        let syms = visible_at(&tree, src, at, lang.scope().expect("checked"));
        if !syms.iter().any(|s| bare(&s.name) == "the_fn") {
            empty.push(format!(
                "  {}: the function it declares is not visible at file scope \
                 (got {:?})",
                lang.name(),
                syms.iter().map(|s| &s.name).collect::<Vec<_>>()
            ));
        }
    }
    assert!(empty.is_empty(), "\n{}\n", empty.join("\n"));
}

// ------------------------------------------------------- caret at end of line

/// The caret sits BETWEEN characters, and completion fires with it at the end
/// of the prefix being typed. If that position is the end of a line, no node
/// covers the empty range and tree-sitter answers with the root — which strips
/// every enclosing scope.
///
/// Rust and C never showed it: a closing `}` keeps the caret inside the block's
/// span. Python has no such character, so this WAS the normal case there —
/// completion inside a function offered `def` names and nothing else.
#[test]
fn a_caret_at_end_of_line_still_sees_its_scope() {
    let cases = [
        Case {
            lang: Lang::Python,
            src: "def foo(the_param):\n    local_b = 2\n    loc⟨here⟩\n",
        },
        Case {
            lang: Lang::Go,
            src: "package main\nfunc foo(the_param int) {\n    local_b := 2\n    loc⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::JavaScript,
            src: "function foo(the_param) {\n    let local_b = 2;\n    loc⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Rust,
            src: "fn foo(the_param: i32) {\n    let local_b = 2;\n    loc⟨here⟩\n}\n",
        },
        Case {
            lang: Lang::Ruby,
            src: "def foo(the_param)\n  local_b = 2\n  loc⟨here⟩\nend\n",
        },
        Case {
            lang: Lang::Lua,
            src: "local function foo(the_param)\n  local local_b = 2\n  loc⟨here⟩\nend\n",
        },
    ];
    let mut failures = Vec::new();
    for case in cases {
        let got = names(case.src, case.lang);
        for want in ["the_param", "local_b", "foo"] {
            if !got.iter().any(|n| bare(n) == want) {
                failures.push(format!(
                    "  {}: {want:?} lost with the caret at end of line\n    visible: {got:?}",
                    case.lang.name()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}
