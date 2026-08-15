//! Which language a file is — decided **once**, here.
//!
//! Before this module the same question was answered by two `match` arms in
//! `syntax.rs` (one for `bundle_for`, one inlined in `parse_window` to dodge a
//! borrow), plus a third in `scope.rs`. They had already drifted: `syntax.rs`
//! parsed `cpp`/`hpp`/`cc`/`cxx`/`hh`/`hxx` and `scope.rs` did not, so C++ files
//! highlighted and then silently offered no buffer symbols. `pyi`, `mts` and
//! `cts` were missing the same way. Two authorities for one fact is how that
//! happens, and going from 8 grammars to 28 would have doubled the surface.
//!
//! So: [`Lang`] owns the extension table, the grammar, the highlight query and
//! the link to [`crate::scope::ScopeLang`]. `scope.rs` and `syntax.rs` both ask
//! this module. `tests/every_grammar_loads.rs` walks [`Lang::ALL`] and asserts
//! each one loads, compiles its query and actually paints something — a grammar
//! that fails any of those is removed from the table rather than left to
//! degrade quietly into the regex fallback.
//!
//! The extensions are listed per language and `from_ext` scans them, rather
//! than a separate lookup table that could disagree with the list.

use tree_sitter::{Language, Parser, Query};

use crate::scope::ScopeLang;

/// A language Suisei has a tree-sitter grammar for.
///
/// Everything NOT here still opens, still highlights through the regex
/// fallback in `highlight.rs`, and still gets keyword completion. What it does
/// not get is a parse tree — so no query-driven highlighting and no
/// scope-aware completion. That is the whole difference between the tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    C,
    Cpp,
    Go,
    Bash,
    Json,
    Java,
    CSharp,
    Ruby,
    Lua,
    Swift,
    Php,
    Zig,
    Scala,
    Haskell,
    Elixir,
    Dart,
    ObjC,
    Html,
    Css,
    Yaml,
    Toml,
    Markdown,
    Xml,
    CMake,
}

impl Lang {
    /// Every language in the table. Walked by the conformance tests and by
    /// `SyntaxEngine::warm_all`, so a language added below is covered by both
    /// without a second list to remember.
    pub const ALL: &'static [Lang] = &[
        Lang::Rust,
        Lang::Python,
        Lang::JavaScript,
        Lang::TypeScript,
        Lang::Tsx,
        Lang::C,
        Lang::Cpp,
        Lang::Go,
        Lang::Bash,
        Lang::Json,
        Lang::Java,
        Lang::CSharp,
        Lang::Ruby,
        Lang::Lua,
        Lang::Swift,
        Lang::Php,
        Lang::Zig,
        Lang::Scala,
        Lang::Haskell,
        Lang::Elixir,
        Lang::Dart,
        Lang::ObjC,
        Lang::Html,
        Lang::Css,
        Lang::Yaml,
        Lang::Toml,
        Lang::Markdown,
        Lang::Xml,
        Lang::CMake,
    ];

    /// Human name, for the status line and for test failure messages.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "Rust",
            Lang::Python => "Python",
            Lang::JavaScript => "JavaScript",
            Lang::TypeScript => "TypeScript",
            Lang::Tsx => "TSX",
            Lang::C => "C",
            Lang::Cpp => "C++",
            Lang::Go => "Go",
            Lang::Bash => "Shell",
            Lang::Json => "JSON",
            Lang::Java => "Java",
            Lang::CSharp => "C#",
            Lang::Ruby => "Ruby",
            Lang::Lua => "Lua",
            Lang::Swift => "Swift",
            Lang::Php => "PHP",
            Lang::Zig => "Zig",
            Lang::Scala => "Scala",
            Lang::Haskell => "Haskell",
            Lang::Elixir => "Elixir",
            Lang::Dart => "Dart",
            Lang::ObjC => "Objective-C",
            Lang::Html => "HTML",
            Lang::Css => "CSS",
            Lang::Yaml => "YAML",
            Lang::Toml => "TOML",
            Lang::Markdown => "Markdown",
            Lang::Xml => "XML",
            Lang::CMake => "CMake",
        }
    }

    /// Extensions this language claims, lowercase, without the dot.
    ///
    /// No extension may appear twice across the table — pinned by
    /// `no_extension_is_claimed_twice`.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Lang::Rust => &["rs"],
            Lang::Python => &["py", "pyi", "pyw"],
            Lang::JavaScript => &["js", "jsx", "mjs", "cjs"],
            Lang::TypeScript => &["ts", "mts", "cts"],
            Lang::Tsx => &["tsx"],
            // `.h` stays C. Telling a C++ header from a C one needs the project,
            // not the name; guessing C++ would mis-parse every C header, and
            // guessing C is what the editor already did.
            Lang::C => &["c", "h"],
            Lang::Cpp => &[
                "cpp", "cxx", "cc", "c++", "hpp", "hxx", "hh", "h++", "ipp", "tpp", "inl",
            ],
            Lang::Go => &["go"],
            Lang::Bash => &["sh", "bash", "zsh", "ksh"],
            Lang::Json => &["json", "jsonc"],
            Lang::Java => &["java"],
            Lang::CSharp => &["cs", "csx"],
            Lang::Ruby => &["rb", "rake", "gemspec", "ru"],
            Lang::Lua => &["lua"],
            Lang::Swift => &["swift"],
            Lang::Php => &["php", "phtml"],
            Lang::Zig => &["zig"],
            Lang::Scala => &["scala", "sc", "sbt"],
            // `.lhs` is literate Haskell — a different surface syntax that this
            // grammar does not parse, so it keeps the regex fallback.
            Lang::Haskell => &["hs"],
            Lang::Elixir => &["ex", "exs"],
            Lang::Dart => &["dart"],
            // `.m` is also MATLAB. In a tree that has an editor's worth of
            // Swift and headers beside it, Objective-C is the better guess; a
            // MATLAB file gets a wrong parse, not a crash, and still highlights.
            Lang::ObjC => &["m", "mm"],
            Lang::Html => &["html", "htm", "xhtml"],
            // `scss`/`less` are supersets this grammar rejects — they keep the
            // regex fallback, which handles them adequately.
            Lang::Css => &["css"],
            Lang::Yaml => &["yaml", "yml"],
            Lang::Toml => &["toml"],
            // `.mdx` is Markdown with JSX embedded; the block grammar mis-reads
            // the JSX, so it stays on the fallback.
            Lang::Markdown => &["md", "markdown"],
            Lang::Xml => &["xml", "xsd", "xsl", "xslt", "svg", "plist"],
            Lang::CMake => &["cmake"],
        }
    }

    /// The language claiming `ext`, which must already be lowercase.
    pub fn from_ext(ext: &str) -> Option<Lang> {
        Lang::ALL
            .iter()
            .copied()
            .find(|l| l.extensions().contains(&ext))
    }

    /// The grammar and its highlight query source.
    ///
    /// Built on demand — constructing a `Language` is cheap, compiling a
    /// `Query` is not, which is why `Grammars` below caches the compiled pair
    /// rather than this.
    ///
    /// Some queries are **composed**, and that is not a nicety. The crate's
    /// TypeScript `highlights.scm` is 35 lines that add types, parameters and
    /// the TS-only keywords — it has no rule for a comment, a string, a number
    /// or a function, because upstream layers it over the JavaScript query.
    /// Suisei was using it alone, so every `.ts` and `.tsx` file in the editor
    /// had uncoloured comments and strings. The composition order matters:
    /// the overlay goes LAST, because where two patterns capture the same node
    /// the later one is the one that paints (see `syntax.rs::flatten_overlaps`).
    pub fn grammar(self) -> (Language, String) {
        let (lang, query): (Language, String) = match self {
            Lang::Rust => (
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Python => (
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::JavaScript => (
                tree_sitter_javascript::LANGUAGE.into(),
                [
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                ]
                .join("\n"),
            ),
            Lang::TypeScript => (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                [
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                ]
                .join("\n"),
            ),
            Lang::Tsx => (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                [
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                ]
                .join("\n"),
            ),
            Lang::C => (
                tree_sitter_c::LANGUAGE.into(),
                tree_sitter_c::HIGHLIGHT_QUERY.to_string(),
            ),
            // Same overlay trap as TypeScript: the C++ query is 70 lines of
            // C++-only constructs with no comment and no ordinary string rule,
            // because it is written to sit on top of C's.
            Lang::Cpp => (
                tree_sitter_cpp::LANGUAGE.into(),
                [
                    tree_sitter_c::HIGHLIGHT_QUERY,
                    tree_sitter_cpp::HIGHLIGHT_QUERY,
                ]
                .join("\n"),
            ),
            Lang::Go => (
                tree_sitter_go::LANGUAGE.into(),
                tree_sitter_go::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Bash => (
                tree_sitter_bash::LANGUAGE.into(),
                tree_sitter_bash::HIGHLIGHT_QUERY.to_string(),
            ),
            Lang::Json => (
                tree_sitter_json::LANGUAGE.into(),
                tree_sitter_json::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Java => (
                tree_sitter_java::LANGUAGE.into(),
                tree_sitter_java::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::CSharp => (
                tree_sitter_c_sharp::LANGUAGE.into(),
                tree_sitter_c_sharp::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Ruby => (
                tree_sitter_ruby::LANGUAGE.into(),
                tree_sitter_ruby::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Lua => (
                tree_sitter_lua::LANGUAGE.into(),
                tree_sitter_lua::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Swift => (
                tree_sitter_swift::LANGUAGE.into(),
                tree_sitter_swift::HIGHLIGHTS_QUERY.to_string(),
            ),
            // `LANGUAGE_PHP` is the one that understands `<?php` delimiters;
            // `LANGUAGE_PHP_ONLY` assumes the file is already inside a tag.
            Lang::Php => (
                tree_sitter_php::LANGUAGE_PHP.into(),
                tree_sitter_php::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Zig => (
                tree_sitter_zig::LANGUAGE.into(),
                tree_sitter_zig::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Scala => (
                tree_sitter_scala::LANGUAGE.into(),
                tree_sitter_scala::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Haskell => (
                tree_sitter_haskell::LANGUAGE.into(),
                tree_sitter_haskell::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Elixir => (
                tree_sitter_elixir::LANGUAGE.into(),
                tree_sitter_elixir::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Dart => (
                tree_sitter_dart::LANGUAGE.into(),
                tree_sitter_dart::HIGHLIGHTS_QUERY.to_string(),
            ),
            // The Objective-C query is 216 lines and has no rule for a comment
            // or an ordinary string literal at all — its only `@string` is
            // `(platform)`. That is a hole upstream, not an overlay, so Suisei
            // supplies the two rules rather than shipping a language whose
            // comments are body text. Both node kinds are the C ones the
            // grammar inherits; if that ever stops being true, `Query::new`
            // fails and `every_grammar_loads` says so.
            Lang::ObjC => (
                tree_sitter_objc::LANGUAGE.into(),
                [
                    tree_sitter_objc::HIGHLIGHTS_QUERY,
                    "(comment) @comment\n(string_literal) @string\n",
                ]
                .join("\n"),
            ),
            Lang::Html => (
                tree_sitter_html::LANGUAGE.into(),
                tree_sitter_html::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Css => (
                tree_sitter_css::LANGUAGE.into(),
                tree_sitter_css::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Yaml => (
                tree_sitter_yaml::LANGUAGE.into(),
                tree_sitter_yaml::HIGHLIGHTS_QUERY.to_string(),
            ),
            Lang::Toml => (
                tree_sitter_toml_ng::LANGUAGE.into(),
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY.to_string(),
            ),
            // Markdown ships two grammars: block structure and inline spans.
            // Suisei's pipeline is one tree per document, so this is the block
            // one — headings, fences, lists, quotes. Inline emphasis and links
            // stay uncoloured until the parser can host injections.
            Lang::Markdown => (
                tree_sitter_md::LANGUAGE.into(),
                tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.to_string(),
            ),
            Lang::Xml => (
                tree_sitter_xml::LANGUAGE_XML.into(),
                tree_sitter_xml::XML_HIGHLIGHT_QUERY.to_string(),
            ),
            Lang::CMake => (
                tree_sitter_cmake::LANGUAGE.into(),
                tree_sitter_cmake::HIGHLIGHTS_QUERY.to_string(),
            ),
        };
        (lang, query)
    }

    /// Whether Enter should carry the current line's indentation down.
    ///
    /// Auto-indent is a CODE affordance. In a language with nesting, the line
    /// you are starting almost always belongs at the depth of the one you just
    /// left, and typing the indent by hand every time is the thing the editor
    /// is for.
    ///
    /// In prose it is the opposite. A wrapped Markdown bullet's continuation is
    /// indented two spaces to keep it under the bullet — press Enter at its end
    /// and you get two spaces you did not ask for, on a line that is a new
    /// thought rather than more of the same one. Reported against README.md
    /// line 10, which is exactly that shape.
    ///
    /// Markup that nests structurally — HTML, XML — keeps it: their indent
    /// means depth, the same as code. Markdown's does not; it means "this is
    /// still the previous bullet".
    pub fn auto_indents(self) -> bool {
        !matches!(self, Lang::Markdown)
    }

    /// How `scope.rs` should walk this language's tree, if it can.
    ///
    /// `None` is a statement, not a gap: a language returns it when lexical
    /// scope completion is either meaningless (JSON, YAML, TOML, Markdown, XML,
    /// HTML, CSS, CMake — no bindings to resolve) or not yet written. The
    /// reason is on each arm so the next reader does not have to guess which.
    pub fn scope(self) -> Option<ScopeLang> {
        match self {
            Lang::Rust => Some(ScopeLang::Rust),
            Lang::Python => Some(ScopeLang::Python),
            Lang::JavaScript => Some(ScopeLang::JavaScript),
            Lang::TypeScript | Lang::Tsx => Some(ScopeLang::TypeScript),
            Lang::C => Some(ScopeLang::C),
            Lang::Cpp => Some(ScopeLang::Cpp),
            Lang::Go => Some(ScopeLang::Go),
            Lang::Java => Some(ScopeLang::Java),
            Lang::CSharp => Some(ScopeLang::CSharp),
            Lang::Ruby => Some(ScopeLang::Ruby),
            Lang::Lua => Some(ScopeLang::Lua),
            Lang::Swift => Some(ScopeLang::Swift),
            Lang::Php => Some(ScopeLang::Php),
            Lang::Zig => Some(ScopeLang::Zig),
            Lang::Dart => Some(ScopeLang::Dart),
            Lang::ObjC => Some(ScopeLang::ObjC),
            // Precise highlighting, no scope walk yet. All three bind names in
            // ways the walk has no vocabulary for — Scala's `val` patterns and
            // givens, Haskell's equations and guards, Elixir's pattern-matching
            // `=`. Each needs its own rules and a row per rule in
            // `tests/scope_language_conformance.rs`; claiming support without
            // that is exactly the silent-empty-list failure this table exists
            // to prevent.
            Lang::Scala | Lang::Haskell | Lang::Elixir => None,
            // Nothing to resolve: no lexical bindings in the language at all.
            Lang::Bash
            | Lang::Json
            | Lang::Html
            | Lang::Css
            | Lang::Yaml
            | Lang::Toml
            | Lang::Markdown
            | Lang::Xml
            | Lang::CMake => None,
        }
    }
}

/// A grammar's parser and its compiled highlight query.
pub struct LangBundle {
    pub parser: Parser,
    pub query: Query,
}

impl LangBundle {
    /// Build the pair, or say why not.
    ///
    /// The reason matters and is not otherwise recoverable: a grammar the
    /// runtime cannot load (its generated parser speaks a newer ABI) and a
    /// query that will not compile are different problems with different
    /// fixes, and both present at runtime as a file that simply is not
    /// highlighted. `tests/every_grammar_loads.rs` prints this.
    pub fn try_build(lang: Lang) -> Result<LangBundle, String> {
        let (language, source) = lang.grammar();
        let mut parser = Parser::new();
        if let Err(e) = parser.set_language(&language) {
            return Err(format!("grammar will not load: {e}"));
        }
        let query =
            Query::new(&language, &source).map_err(|e| format!("highlight query: {e:?}"))?;
        Ok(LangBundle { parser, query })
    }

    pub fn build(lang: Lang) -> Option<LangBundle> {
        Self::try_build(lang).ok()
    }
}

/// Compiled grammars, built on first use and kept.
///
/// A separate struct rather than fields on `SyntaxEngine` for one concrete
/// reason: a method taking `&mut self` on the engine borrows the WHOLE engine,
/// which is why the language table used to be inlined into `parse_window` a
/// second time. Borrowing one field leaves the rest of the engine writable, so
/// there can be a single lookup.
#[derive(Default)]
pub struct Grammars {
    /// `None` records a build that failed, so a broken grammar is attempted
    /// once rather than on every keystroke.
    loaded: std::collections::HashMap<Lang, Option<LangBundle>>,
}

impl Grammars {
    pub fn get(&mut self, lang: Lang) -> Option<&mut LangBundle> {
        self.loaded
            .entry(lang)
            .or_insert_with(|| LangBundle::build(lang))
            .as_mut()
    }

    pub fn for_ext(&mut self, ext: Option<&str>) -> Option<&mut LangBundle> {
        self.get(Lang::from_ext(ext?)?)
    }

    /// How many grammars are compiled right now — used by the boot path's
    /// warm-up assertions.
    pub fn loaded_count(&self) -> usize {
        self.loaded.values().filter(|b| b.is_some()).count()
    }
}
