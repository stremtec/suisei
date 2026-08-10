//! Every grammar in the table loads, parses, and actually paints.
//!
//! A tree-sitter grammar can fail in three ways that all look like silence:
//!
//! 1. the generated parser speaks an ABI the linked runtime does not, so
//!    `set_language` returns `Err` and the file falls back to regex;
//! 2. the highlight query does not compile against that grammar, same result;
//! 3. both work, but every capture name in the query is one
//!    `highlight::from_capture` has never heard of, so the query matches and
//!    produces no spans.
//!
//! None of the three is a compile error. Adding twenty grammars without this
//! file would mean adding twenty places for a language to claim support and
//! deliver nothing — which is the exact failure the language audit found in the
//! shipped C++ support.
//!
//! ```text
//! cargo test -p suisei-core --test every_grammar_loads
//! ```

use std::collections::HashMap;

use suisei_core::highlight::from_capture;
use suisei_core::lang::{Lang, LangBundle};
use suisei_core::syntax::SyntaxEngine;

/// A few lines of each language, chosen to exercise the things a highlighter
/// must colour: a comment, a string, a number, a declaration, a call.
fn sample(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => "// c\nfn main() { let n: i32 = 1; println!(\"hi\"); }\n",
        Lang::Python => "# c\ndef main(n: int) -> int:\n    s = \"hi\"\n    return n + 1\n",
        Lang::JavaScript => "// c\nfunction main(n) { const s = \"hi\"; return n + 1; }\n",
        Lang::TypeScript => {
            "// c\nfunction main(n: number): number { const s: string = \"hi\"; return n + 1; }\n"
        }
        Lang::Tsx => "// c\nconst A = () => <div className=\"x\">{1}</div>;\n",
        Lang::C => {
            "/* c */\n#include <stdio.h>\nint main(int n) { char *s = \"hi\"; return n + 1; }\n"
        }
        Lang::Cpp => concat!(
            "// c\n#include <string>\n",
            "namespace app {\n",
            "template <typename T>\nclass Star {\npublic:\n  Star(T b) : brightness(b) {}\n",
            "  bool is_bright() const { return brightness > 1; }\n",
            "private:\n  T brightness;\n};\n}\n",
        ),
        Lang::Go => {
            "// c\npackage main\n\nfunc main(n int) int {\n\ts := \"hi\"\n\treturn n + 1\n}\n"
        }
        Lang::Bash => {
            "# c\nmain() {\n  local s=\"hi\"\n  if [ -n \"$s\" ]; then\n    echo \"$s\" 1\n  fi\n}\n"
        }
        Lang::Json => "{\n  \"a\": 1,\n  \"b\": \"hi\",\n  \"c\": [true, null]\n}\n",
        Lang::Java => concat!(
            "// c\nclass Star {\n  private int brightness = 1;\n",
            "  public boolean isBright(int limit) { String s = \"hi\"; return brightness > limit; }\n}\n",
        ),
        Lang::CSharp => concat!(
            "// c\nnamespace App {\n  class Star {\n    private int brightness = 1;\n",
            "    public bool IsBright(int limit) { string s = \"hi\"; return brightness > limit; }\n  }\n}\n",
        ),
        Lang::Ruby => {
            "# c\nclass Star\n  def is_bright(limit)\n    s = \"hi\"\n    @brightness > limit\n  end\nend\n"
        }
        Lang::Lua => "-- c\nlocal function main(n)\n  local s = \"hi\"\n  return n + 1\nend\n",
        Lang::Swift => concat!(
            "// c\nclass Star {\n  var brightness: Double = 1.0\n",
            "  func isBright(limit: Double) -> Bool { let s = \"hi\"; return brightness > limit }\n}\n",
        ),
        Lang::Php => {
            "<?php\n// c\nfunction main(int $n): int {\n  $s = \"hi\";\n  return $n + 1;\n}\n"
        }
        Lang::Zig => {
            "// c\nconst std = @import(\"std\");\n\npub fn main(n: i32) i32 {\n    return n + 1;\n}\n"
        }
        Lang::Scala => {
            "// c\nobject Main {\n  def main(n: Int): Int = {\n    val s = \"hi\"\n    n + 1\n  }\n}\n"
        }
        Lang::Haskell => "-- c\nmodule Main where\n\nmain :: Int -> Int\nmain n = n + 1\n",
        Lang::Elixir => {
            "# c\ndefmodule Main do\n  def main(n) do\n    s = \"hi\"\n    n + 1\n  end\nend\n"
        }
        Lang::Dart => {
            "// c\nclass Star {\n  double brightness = 1.0;\n  bool isBright(double limit) => brightness > limit;\n}\n"
        }
        Lang::ObjC => {
            "// c\n#import <Foundation/Foundation.h>\n\n@interface Star : NSObject\n- (BOOL)isBright:(double)limit;\n@end\n"
        }
        Lang::Html => {
            "<!-- c -->\n<html>\n  <body class=\"x\">\n    <p>hi</p>\n  </body>\n</html>\n"
        }
        Lang::Css => "/* c */\n.star {\n  color: #fff;\n  margin: 1px;\n}\n",
        Lang::Yaml => "# c\nname: suisei\nversion: 1\nlist:\n  - a\n  - b\n",
        Lang::Toml => "# c\n[package]\nname = \"suisei\"\nversion = 1\n",
        Lang::Markdown => "# Heading\n\nSome text.\n\n```rust\nfn main() {}\n```\n\n- a\n- b\n",
        Lang::Xml => {
            "<?xml version=\"1.0\"?>\n<!-- c -->\n<root a=\"1\">\n  <child>hi</child>\n</root>\n"
        }
        Lang::CMake => "# c\nproject(suisei)\n\nadd_library(core STATIC src/main.c)\n",
    }
}

/// Capture names a highlight query may emit that Suisei deliberately paints
/// nothing for.
///
/// The point of the list is that it is short and explicit. A capture NOT here
/// and not understood by `from_capture` is a gap in the highlighter, and the
/// test says which grammar produced it — that is how "refine the highlighting"
/// becomes a finite, checkable job rather than an opinion.
const IGNORED_CAPTURES: &[&str] = &[
    // Editor concerns that are not colour.
    "spell",
    "nospell",
    "conceal",
    "none",
    // A bare `@text` run is body text; painting it would spend a span to
    // produce the default foreground colour.
    "text",
    // Query plumbing, never a span on its own.
    "local.scope",
    "local.definition",
    "local.reference",
    "injection.content",
    "injection.language",
    // The region an injected language occupies — Suisei parses one grammar per
    // document, so there is nothing to hand it to.
    "embedded",
    // A parse error. Painting it red on every keystroke of half-typed code is
    // noise, not information.
    "error",
];

/// Captures whose name begins with `_` are a query-authoring convention:
/// scratch bindings that exist so a `#eq?` or `#match?` predicate can refer to
/// them. They are not meant to paint, and every grammar spells its own
/// (`@_name`, `@_op`, `@_selector`, `@__name__`).
fn is_internal_capture(name: &str) -> bool {
    name.starts_with('_')
}

fn bundles() -> Vec<(Lang, LangBundle)> {
    let mut out = Vec::new();
    let mut failed = Vec::new();
    for lang in Lang::ALL {
        match LangBundle::try_build(*lang) {
            Ok(b) => out.push((*lang, b)),
            Err(why) => failed.push(format!("  {:<12} {why}", lang.name())),
        }
    }
    assert!(
        failed.is_empty(),
        "\ngrammar or highlight query failed to build:\n{}\n",
        failed.join("\n")
    );
    out
}

#[test]
fn every_grammar_builds() {
    assert_eq!(
        bundles().len(),
        Lang::ALL.len(),
        "every language in the table must build"
    );
}

/// Kinds a language's sample must actually produce.
///
/// A span count alone is too weak to catch the interesting failure. The
/// TypeScript query shipped by the crate is a 35-line OVERLAY — types,
/// parameters and the TS-only keywords — with no rule for a comment, a string
/// or a number, because upstream layers it on the JavaScript query. Used alone
/// it still produced plenty of spans, so a count check passed while every `.ts`
/// file in the editor had uncoloured comments and strings for as long as the
/// feature had existed. Naming the kinds is what finds that.
fn must_paint(lang: Lang) -> &'static [suisei_core::highlight::TokenKind] {
    use suisei_core::highlight::TokenKind::{Comment, Keyword, Number, String as Str};
    match lang {
        // No comment syntax at all.
        Lang::Json => &[Str, Number],
        // The block grammar has no comment either; a fenced code block is what
        // it must at least find.
        Lang::Markdown => &[Str],
        // No string literal in the sample, but keywords and a comment.
        Lang::Css | Lang::CMake | Lang::Yaml | Lang::ObjC | Lang::Dart | Lang::Haskell => {
            &[Comment]
        }
        // TOML has no keywords — it is keys, values and tables.
        Lang::Toml => &[Comment, Str],
        Lang::Xml | Lang::Html => &[Comment, Str],
        _ => &[Comment, Str, Keyword],
    }
}

#[test]
fn every_grammar_paints_its_sample() {
    let mut engine = SyntaxEngine::new();
    let mut thin = Vec::new();
    for lang in Lang::ALL {
        let ext = lang.extensions()[0];
        engine.parse(sample(*lang), Some(ext));
        let kinds: Vec<_> = engine.tokens.iter().map(|t| t.0).collect();
        let missing: Vec<_> = must_paint(*lang)
            .iter()
            .filter(|k| !kinds.contains(k))
            .map(|k| format!("{k:?}"))
            .collect();
        if !missing.is_empty() {
            thin.push(format!(
                "  {:<12} .{ext}: no {} among {} spans (active={})",
                lang.name(),
                missing.join(", "),
                kinds.len(),
                engine.active
            ));
        }
    }
    assert!(
        thin.is_empty(),
        "\ngrammars whose query never reaches the painter for a kind the sample \
         plainly contains — usually a capture name `highlight::from_capture` \
         drops, or a query used without the base layer it overlays:\n{}\n",
        thin.join("\n")
    );
}

#[test]
fn every_capture_name_is_understood() {
    let mut unknown: HashMap<String, Vec<&str>> = HashMap::new();
    for (lang, bundle) in bundles() {
        for name in bundle.query.capture_names() {
            if IGNORED_CAPTURES.contains(name)
                || is_internal_capture(name)
                || from_capture(name).is_some()
            {
                continue;
            }
            unknown
                .entry((*name).to_string())
                .or_default()
                .push(lang.name());
        }
    }
    let mut lines: Vec<String> = unknown
        .iter()
        .map(|(cap, langs)| format!("  @{cap:<28} {}", langs.join(", ")))
        .collect();
    lines.sort();
    assert!(
        lines.is_empty(),
        "\ncapture names no `TokenKind` answers for — add each to \
         `highlight::from_capture`, or to IGNORED_CAPTURES if it should paint \
         nothing:\n{}\n",
        lines.join("\n")
    );
}

/// Not a check — a tool. Prints each sample's parse tree so the node kinds in
/// `scope.rs`'s rule tables can be read off the grammar instead of guessed.
/// Guessing them is how a language ends up claiming scope support and returning
/// an empty list.
///
/// ```text
/// cargo test -p suisei-core --test every_grammar_loads -- --ignored --nocapture dump
/// ```
#[test]
#[ignore = "diagnostic tool, not an assertion"]
fn dump_sample_parse_trees() {
    let only = std::env::var("SUISEI_DUMP_LANG").unwrap_or_default();
    for lang in Lang::ALL {
        if !only.is_empty() && !lang.name().eq_ignore_ascii_case(&only) {
            continue;
        }
        let Some(mut b) = LangBundle::build(*lang) else {
            continue;
        };
        let src = sample(*lang);
        let Some(tree) = b.parser.parse(src, None) else {
            continue;
        };
        println!(
            "\n===== {} =====\n{}",
            lang.name(),
            tree.root_node().to_sexp()
        );
    }
}

#[test]
fn a_first_parse_builds_only_the_grammar_it_needs() {
    // Compiling all 29 highlight queries takes 780 ms — Haskell alone is
    // 186 ms. The worker used to do that in one call, BEFORE the first parse
    // in the same burst, which at eight grammars was unnoticeable and at
    // twenty-nine would have left the first file opened without colours for
    // most of a second. It now drains the table one grammar per idle turn, so
    // what a first paint costs is this: one.
    let mut engine = SyntaxEngine::new();
    assert_eq!(engine.grammars_loaded(), 0, "nothing built before a parse");
    engine.parse(sample(Lang::Rust), Some("rs"));
    assert_eq!(
        engine.grammars_loaded(),
        1,
        "parsing one file must not drag the whole table in with it"
    );
    assert!(!engine.tokens.is_empty(), "and it must actually have painted");
}

#[test]
fn no_extension_is_claimed_twice() {
    let mut owner: HashMap<&str, Lang> = HashMap::new();
    let mut clashes = Vec::new();
    for lang in Lang::ALL {
        for ext in lang.extensions() {
            if let Some(prev) = owner.insert(ext, *lang) {
                clashes.push(format!("  .{ext}: {} and {}", prev.name(), lang.name()));
            }
        }
    }
    assert!(
        clashes.is_empty(),
        "\ntwo languages claim the same extension, so which one wins depends on \
         the order of Lang::ALL:\n{}\n",
        clashes.join("\n")
    );
}

#[test]
fn every_extension_finds_its_language() {
    let mut wrong = Vec::new();
    for lang in Lang::ALL {
        for ext in lang.extensions() {
            if Lang::from_ext(ext) != Some(*lang) {
                wrong.push(format!(
                    "  .{ext} -> {:?}, expected {}",
                    Lang::from_ext(ext).map(Lang::name),
                    lang.name()
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}\n", wrong.join("\n"));
}
