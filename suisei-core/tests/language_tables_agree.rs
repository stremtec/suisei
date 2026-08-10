//! The other tables that answer "what language is this file?" agree with the
//! language table.
//!
//! There is one fact — a file's language — and historically five places
//! decided it: the two `match` arms in `syntax.rs`, `scope.rs`, `highlight.rs`,
//! and three in `lsp.rs`. They drifted, and every drift is silent: the file
//! opens, so nothing looks broken. C++ highlighted and offered no symbols for
//! as long as the feature existed, because two of those tables disagreed.
//!
//! `crate::lang` is now the one that decides, and the rest normalise through
//! it. This asserts the normalisation actually reaches them, which is the part
//! a refactor can quietly get wrong.
//!
//! ```text
//! cargo test -p suisei-core --test language_tables_agree
//! ```

use suisei_core::highlight::rules_for_ext;
use suisei_core::lang::Lang;
use suisei_core::lsp::ext_to_lang_key;

#[test]
fn every_extension_resolves_to_an_lsp_language_key() {
    // A language server keyed off the extension is how "go to definition" and
    // diagnostics start at all. An extension the language table knows and this
    // one does not is a file that highlights and then behaves as unsupported.
    let mut missing = Vec::new();
    for lang in Lang::ALL {
        for ext in lang.extensions() {
            if ext_to_lang_key(ext).is_none() {
                missing.push(format!("  .{ext} ({})", lang.name()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "\nparsed by a grammar but unknown to `lsp::ext_to_lang_key`:\n{}\n",
        missing.join("\n")
    );
}

#[test]
fn every_language_key_is_configurable_in_settings() {
    // `config::lsp_lang_catalog` is what the Settings pane lists, so a key
    // missing from it is a language whose server the user can neither see nor
    // change — while `lsp::default_server_for_ext` starts one anyway.
    let catalog = suisei_core::config::lsp_lang_catalog();
    let mut hidden = Vec::new();
    for lang in Lang::ALL {
        let Some(key) = ext_to_lang_key(lang.extensions()[0]) else {
            continue;
        };
        if !catalog.iter().any(|(k, _, _)| *k == key) {
            hidden.push(format!("  {key} ({})", lang.name()));
        }
    }
    assert!(
        hidden.is_empty(),
        "\nlanguages with a server but no row in Settings:\n{}\n",
        hidden.join("\n")
    );
}

#[test]
fn every_code_language_has_keywords_to_complete() {
    // Keyword completion is the second half of the popup — buffer symbols
    // first, keywords after. Java, C#, Ruby, Lua, Swift, PHP, Zig, Scala,
    // Haskell, Elixir and Dart had no entry in `completion.rs` at all and so
    // offered none, while `highlight.rs` had held their keyword lists the whole
    // time for colouring. One table now; this checks it reaches every language
    // that has code in it.
    let mut bare = Vec::new();
    for lang in Lang::ALL {
        if lang.scope().is_none() {
            // Data and markup languages: no reserved words to offer.
            continue;
        }
        for ext in lang.extensions() {
            if rules_for_ext(Some(ext)).keywords.is_empty() {
                bare.push(format!("  .{ext} ({})", lang.name()));
            }
        }
    }
    assert!(
        bare.is_empty(),
        "\nlanguages with a scope walk but no keywords to complete:\n{}\n",
        bare.join("\n")
    );
}

#[test]
fn an_alternate_spelling_behaves_like_the_common_one() {
    // The spellings the language table added — `c++`, `hxx`, `rake`, `csx`,
    // `phtml`, `sbt`, `ksh`, `pyw` — must not be second-class. Each has to
    // reach the same language server key and the same keyword set as the
    // spelling everyone writes.
    let pairs = [
        ("c++", "cpp"),
        ("hxx", "cpp"),
        ("ipp", "cpp"),
        ("rake", "rb"),
        ("gemspec", "rb"),
        ("csx", "cs"),
        ("phtml", "php"),
        ("sbt", "scala"),
        ("ksh", "sh"),
        ("pyw", "py"),
        ("xhtml", "html"),
        ("markdown", "md"),
    ];
    let mut wrong = Vec::new();
    for (alt, common) in pairs {
        if Lang::from_ext(alt) != Lang::from_ext(common) {
            wrong.push(format!(
                "  .{alt} is {:?}, .{common} is {:?}",
                Lang::from_ext(alt).map(Lang::name),
                Lang::from_ext(common).map(Lang::name)
            ));
            continue;
        }
        if ext_to_lang_key(alt) != ext_to_lang_key(common) {
            wrong.push(format!(
                "  .{alt} -> LSP {:?}, .{common} -> {:?}",
                ext_to_lang_key(alt),
                ext_to_lang_key(common)
            ));
        }
        if rules_for_ext(Some(alt)).keywords != rules_for_ext(Some(common)).keywords {
            wrong.push(format!("  .{alt} and .{common} have different keywords"));
        }
    }
    assert!(wrong.is_empty(), "\n{}\n", wrong.join("\n"));
}
