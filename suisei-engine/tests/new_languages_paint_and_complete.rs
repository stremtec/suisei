//! The languages this patch added work through the ENGINE, not just through
//! `SyntaxEngine::parse`.
//!
//! Those are different paths and they have diverged before: the GUI parses on a
//! worker thread and adopts frames through `apply_frame`, which for the whole
//! life of the feature set `self.tree = None` — so `live_tree()` was always
//! `None` in the app and scope-aware completion silently returned keywords
//! only. It passed its tests because they called `parse()` directly, which is
//! the TUI path.
//!
//! So this opens a real file per language through the engine, the way the app
//! does, and asks the two questions a user would: is it coloured, and does
//! completion know what is in the file?
//!
//! ```text
//! cargo test -p suisei-engine --test new_languages_paint_and_complete
//! ```

use suisei_core::highlight::TokenKind;
use suisei_engine::Engine;

/// A file with a comment, a string, and declarations at two depths.
struct Case {
    ext: &'static str,
    src: &'static str,
    /// A local, visible only from inside the function that declares it.
    local: &'static str,
    /// A top-level declaration, visible from anywhere in the file.
    top: &'static str,
    /// The caret goes just after the LAST occurrence of this, which puts it
    /// inside the function body. Asking at end-of-file instead would ask the
    /// wrong question: a local is correctly invisible from the outermost scope,
    /// so that version of this test passed by agreeing with itself.
    caret_after: &'static str,
}

const CASES: &[Case] = &[
    Case {
        // The headline regression: the crate's TypeScript query is an overlay
        // with no comment, string or number rule at all.
        ext: "ts",
        src: "// a comment\nfunction main(limit: number): number {\n    const label = \"hi\";\n    return limit + 1;\n}\n",
        local: "label",
        top: "main",
        caret_after: "return limit + 1;",
    },
    Case {
        // Was parsed with the C grammar, so `class` and `namespace` were
        // invisible — and its query is a 70-line overlay on C's, so it had no
        // comment or string rule either.
        ext: "cpp",
        src: "// a comment\nnamespace app {\nclass Star {\npublic:\n    bool is_bright() const {\n        const char *label = \"hi\";\n        return true;\n    }\n};\n}\n",
        local: "label",
        top: "Star",
        caret_after: "return true;",
    },
    Case {
        ext: "java",
        src: "// a comment\nclass Star {\n    boolean isBright(int limit) {\n        String label = \"hi\";\n        return limit > 1;\n    }\n}\n",
        local: "label",
        top: "Star",
        caret_after: "return limit > 1;",
    },
    Case {
        ext: "rb",
        src: "# a comment\nclass Star\n  def is_bright(limit)\n    label = \"hi\"\n    lab\n  end\nend\n",
        local: "label",
        top: "Star",
        caret_after: "    lab",
    },
    Case {
        ext: "php",
        src: "<?php\n// a comment\nfunction is_bright($limit) {\n    $label = \"hi\";\n    return $limit > 1;\n}\n",
        local: "$label",
        top: "is_bright",
        caret_after: "return $limit > 1;",
    },
    Case {
        ext: "swift",
        src: "// a comment\nfunc isBright(limit: Int) -> Bool {\n    let label = \"hi\"\n    return limit > 1\n}\n",
        local: "label",
        top: "isBright",
        caret_after: "return limit > 1",
    },
    Case {
        ext: "lua",
        src: "-- a comment\nlocal function is_bright(limit)\n  local label = \"hi\"\n  return limit > 1\nend\n",
        local: "label",
        top: "is_bright",
        caret_after: "return limit > 1",
    },
    Case {
        ext: "cs",
        src: "// a comment\nclass Star {\n    bool IsBright(int limit) {\n        string label = \"hi\";\n        return limit > 1;\n    }\n}\n",
        local: "label",
        top: "Star",
        caret_after: "return limit > 1;",
    },
    Case {
        ext: "zig",
        src: "// a comment\nfn isBright(limit: i32) bool {\n    const label = \"hi\";\n    _ = label;\n    return limit > 1;\n}\n",
        local: "label",
        top: "isBright",
        caret_after: "return limit > 1;",
    },
    Case {
        ext: "dart",
        src: "// a comment\nbool isBright(int limit) {\n  var label = \"hi\";\n  return limit > 1;\n}\n",
        local: "label",
        top: "isBright",
        caret_after: "return limit > 1;",
    },
    Case {
        ext: "m",
        src: "// a comment\nBOOL isBright(int limit) {\n    const char *label = \"hi\";\n    return limit > 1;\n}\n",
        local: "label",
        top: "isBright",
        caret_after: "return limit > 1;",
    },
];

/// `owner` names the calling test, and that is not decoration: cargo runs the
/// tests in this file on separate threads, and `fs::write` truncates before it
/// writes. Sharing one path meant one test could read the other's file
/// mid-truncation and see a short one — which showed up as a caret anchor that
/// was "not in the file". A directory per test removes the race rather than
/// making it rarer.
fn engine_on(owner: &str, ext: &str, src: &str) -> Engine {
    let dir = std::env::temp_dir()
        .join("suisei_new_languages")
        .join(owner);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("sample.{ext}"));
    std::fs::write(&path, src).expect("write source");

    let mut engine = Engine::new();
    engine.resize(1600.0, 1000.0, 18.0, 8.0, 2.0);
    engine.app = suisei_core::app::App::open_file(path.to_str().unwrap());
    // Warm, as the app is once a file has been open a moment.
    engine.flush_syntax();
    engine
}

#[test]
fn each_new_language_colours_its_comments_and_strings() {
    const OWNER: &str = "colours";
    let mut failures = Vec::new();
    for case in CASES {
        let engine = engine_on(OWNER, case.ext, case.src);
        let kinds: Vec<TokenKind> = engine.app.syntax.tokens.iter().map(|t| t.0).collect();
        for want in [TokenKind::Comment, TokenKind::String] {
            if !kinds.contains(&want) {
                failures.push(format!(
                    "  .{}: no {want:?} among {} spans — the file has one plainly \
                     visible in the source",
                    case.ext,
                    kinds.len()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn each_new_language_offers_a_symbol_from_its_own_buffer() {
    const OWNER: &str = "symbols";
    let mut failures = Vec::new();
    for case in CASES {
        let engine = engine_on(OWNER, case.ext, case.src);
        let Some((tree, text)) = engine.app.syntax.live_tree() else {
            failures.push(format!(
                "  .{}: no live tree — the worker frame did not carry its parse, \
                 which is what makes completion fall back to keywords",
                case.ext
            ));
            continue;
        };
        let Some(lang) = suisei_core::scope::ScopeLang::from_ext(engine.app.syntax.live_ext())
        else {
            failures.push(format!(
                "  .{}: parsed as {:?}, which resolves to no scope language",
                case.ext,
                engine.app.syntax.live_ext()
            ));
            continue;
        };
        // Inside the function body, which is where completion actually fires.
        let Some(found) = text.rfind(case.caret_after) else {
            failures.push(format!("  .{}: caret anchor not in the file", case.ext));
            continue;
        };
        let at = found + case.caret_after.len();
        let syms = suisei_core::scope::visible_at(tree, text, at, lang);
        let names: Vec<&String> = syms.iter().map(|s| &s.name).collect();
        for want in [case.local, case.top] {
            if !syms.iter().any(|s| s.name == want) {
                failures.push(format!(
                    "  .{}: {want:?} not offered inside the body; got {names:?}",
                    case.ext
                ));
            }
        }

        // And the local must NOT leak out to file scope — a list that offers
        // everything is as wrong as one that offers nothing.
        let outer = suisei_core::scope::visible_at(tree, text, text.len(), lang);
        if outer.iter().any(|s| s.name == case.local) {
            failures.push(format!(
                "  .{}: {:?} is visible at file scope, but it is declared inside \
                 a function",
                case.ext, case.local
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}
