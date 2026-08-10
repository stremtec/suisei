# Language support

Audited 2026-08-08, implemented 2026-08-10. The starting complaint was "the
editing features are Rust-centric". They were not — they were **tiered**, and
the tier was set by one thing: whether the language had a tree-sitter grammar
linked. This patch moves twenty languages up a tier and removes the reason the
tiers could drift apart unnoticed.

---

## 1. Before and after

| capability | before | after |
|---|---|---|
| Fallback line highlighter (regex) | ~27 families | ~28 families (+ Objective-C) |
| Keyword completion | 15 spellings, hand-written | **every language the highlighter knows**, derived |
| **tree-sitter grammars** | **8** | **29** |
| **Scope-aware completion** | **6** | **16** |
| Language-server rows in Settings | 14 | 29 |

The 21 grammars added: C++, Java, C#, Ruby, Lua, Swift, PHP, Zig, Scala,
Haskell, Elixir, Dart, Objective-C, HTML, CSS, YAML, TOML, Markdown, XML,
CMake — plus TSX, which existed but shared TypeScript's incomplete query.

Scope-aware completion gained C++, Java, C#, Ruby, Lua, Swift, PHP, Zig, Dart
and Objective-C — 16 of the 19 languages here that have lexical bindings at
all.

### Rejected, with reasons

* **Kotlin** (`tree-sitter-kotlin-ng`) — ships a grammar but **no highlights
  query**, so it would parse and paint nothing. Keeps the regex fallback.
* **Dockerfile**, **Make** — still on tree-sitter 0.20's
  `fn language() -> Language`; that is a different type from 0.25's and does not
  type-check. Both are filename- rather than extension-identified anyway.
* **SQL** (0.0.2), **Vue** (0.0.3), **Nim** (0.1.0) — too early to trust; the
  regex fallback already covers them.

---

## 2. What was actually broken

Three bugs, all of the same shape: something claimed support and delivered
nothing, silently, because the failure looks exactly like a file that has no
grammar.

**C++ was parsed with the C grammar.** Classes, templates and namespaces were
unparsed. `ScopeLang::from_ext` did not know `cpp`/`hpp`/`cc`/`cxx`/`hh`/`hxx`
at all, so C++ files highlighted and then offered no buffer symbols — while the
LSP catalog advertised the entry as "C / C++". `pyi`, `mts` and `cts` were
missing the same way.

**TypeScript had no coloured comments, strings or numbers.** The crate's
`highlights.scm` is 35 lines — types, parameters, the TS-only keywords — because
upstream layers it *over* the JavaScript query. Suisei used it alone. The same
trap held for **C++** (a 70-line overlay on C's) and for **TSX**, which also
needs the JSX query. Queries are now composed, overlay last.

**Objective-C's query has no `@comment` or `@string` rule at all** — its only
`@string` is `(platform)`. That is a hole upstream, not an overlay, so Suisei
supplies the two rules.

**The caret's scope was resolved with too narrow a retry.** A caret sits
*between* characters, and an empty range at a node's end boundary is contained
by neither the token before it nor the statement nor the block — only by
something wider. There was already a retry for that, but it fired only when the
first attempt collapsed all the way to the root. In Lua a caret at the end of
`return limit > 1` resolves to the *function* — two scopes, so the guard was
satisfied — while the block holding every local in the body was skipped. Every
local was missing and the guard could not tell. Both resolutions are now tried
and the one that sees more scopes wins. Found by
`new_languages_paint_and_complete`, not by any per-language table.

Plus one painting bug the new grammars surfaced: **nested captures were resolved
backwards.** The face paints spans in array order and the last one wins; tokens
were sorted narrowest-first, so the *widest*, least specific span was applied
last and overwrote every precise one inside it. An escape sequence could never
differ in colour from its string.

---

## 3. The shape of the fix

### One table, not five

`suisei-core/src/lang.rs` owns the extension table, the grammar, the highlight
query and the link to `ScopeLang`. Everything else normalises through it:

* `syntax.rs` had the language table **twice** — once in `bundle_for`, once
  inlined in `parse_window` to dodge a borrow. Both are gone; `Grammars` is a
  field, so borrowing it leaves the rest of the engine writable, which is what
  the duplication existed to work around.
* `scope.rs::ScopeLang::from_ext` delegates.
* `highlight.rs::rules_for_ext` and `lsp.rs` normalise the extension first, so
  the eleven spellings the table added (`c++`, `hxx`, `ipp`, `rake`, `gemspec`,
  `csx`, `phtml`, `sbt`, `ksh`, `pyw`, `xhtml`) behave like the common ones
  instead of falling through as unknown.

Going from 8 grammars to 29 with five tables would have meant five places for a
language to be half-supported.

### Languages as data

`scope.rs` was a hand-written `match` arm per language. At six that reads fine;
at sixteen it stops being reviewable, because every arm repeats the same few
shapes. They are now `Bind` — *take this node's `name` field*, *walk this
pattern for identifiers*, *unwrap a C declarator*, *this bare identifier is a
parameter because of its parent*, *the first bare identifier child* (Zig,
Objective-C), *the nearest `name` above the body* (Dart buries it two levels
down) — and each language is a table of `DeclRule`
plus a list of scope node kinds. A reviewer checks it against the grammar's
node-types without reading any control flow.

### Keywords derived, not duplicated

`completion.rs` kept its own keyword lists for 15 spellings; `highlight.rs` had
held keyword tables for ~27 languages all along, for colouring. Same fact, two
tables — and Java, C#, Ruby, Lua, Swift, PHP, Zig, Scala, Haskell, Elixir and
Dart were in neither, so they offered no keywords at all. Completion now reads
the highlighter's ruleset. The hand-written arms stay only where their
per-keyword descriptions ("immutable binding") beat anything derivable.

---

## 4. Completion detail

Requested: `is_bright  method` / `new  fn` / `brightness  f32` rather than `fn`
against everything. Two facts were missing, and they are now separate:

* **whether a function takes a receiver.** *That*, not the enclosing type, is
  what makes a function a method — `Star::new` is declared inside `impl Star`
  and is still an associated function. Grammars that already spell a method as
  its own node (`method_definition`, `method_declaration`) say so in their
  table; Rust, Python and PHP are decided by the receiver.
* **the declared type.** Shown verbatim when the grammar has one (Rust, TS, Go,
  C, C++, Java, C#), falling back to the kind label where a language mostly does
  not annotate (Python, JS, Ruby, Lua) — so the column is never blank.

**Fields are deliberately still not offered.** `brightness` and `name` are not
in lexical scope for a bare identifier: writing `brightness` inside a method
does not compile. Member completion after `.` needs the receiver's TYPE, which a
parse tree does not carry — that is the language server's job.

---

## 5. Cost

Measured on this machine, release build:

* compiling all 29 highlight queries: **640–780 ms**, Haskell alone 186 ms;
* resident memory for holding them: **+68 MB**.

The worker used to warm every grammar in one call **before** the first parse in
the same burst. At eight grammars that was invisible; at twenty-nine it would
have left the first file opened without colours for most of a second. It now
drains the table **one grammar per idle turn**, so a first paint costs exactly
one grammar build — pinned by `a_first_parse_builds_only_the_grammar_it_needs`.

The +68 MB is a deliberate trade in the direction already asked for: memory
spent so the first open of any language is instant.

---

## 6. What keeps it honest

Adding a grammar can fail three ways that all look like silence: the parser
speaks an ABI the runtime refuses (this is real — eight of these grammars
declare ABI 15, which is why the runtime moved from tree-sitter 0.24 to 0.25),
the query does not compile, or the query's capture names are ones
`from_capture` drops. None is a compile error.

| test | what it would catch |
|---|---|
| `every_grammar_loads::every_grammar_builds` | ABI refusal, query that will not compile — with the reason |
| `every_grammar_loads::every_grammar_paints_its_sample` | a query that runs and reaches the painter for nothing; **named kinds**, not a span count, because a count check passed for TypeScript for as long as the feature existed |
| `every_grammar_loads::every_capture_name_is_understood` | a capture no `TokenKind` answers for — this is how "refine the highlighting" became a finite job |
| `every_grammar_loads::a_first_parse_builds_only_the_grammar_it_needs` | the warm-up creeping back in front of the first paint |
| `overlapping_captures` (8) | nested spans resolving backwards again |
| `scope_language_conformance` (9 × up to 16 languages) | the six scope rules, per language |
| `scope_language_conformance::no_language_claims_scope_support_and_returns_nothing` | "supported" meaning `from_ext` returns `Some` |
| `language_tables_agree` (4) | the extension tables drifting apart again |
| `completion_detail_by_language` (7) | the detail column going back to `fn` for everything |
| `suisei-engine/new_languages_paint_and_complete` (2) | the GUI path diverging from the TUI one — it opens a real file per language through the engine and asks both questions, which is how the caret-retry bug above was found |

`SUISEI_DUMP_LANG=Lua cargo test -p suisei-core --test every_grammar_loads --
--ignored --nocapture dump` prints a sample's parse tree, which is how the rule
tables were written rather than guessed.

---

## 7. Still open

* **Member completion** after `.` — needs the receiver's type. Route to LSP.
* **Scope walks** for Scala, Haskell and Elixir. All three bind names in ways
  the walk has no vocabulary for — Scala's `val` patterns and givens, Haskell's
  equations and guards, Elixir's pattern-matching `=`. They have precise
  highlighting, keyword completion and a language server; `Lang::scope` returns
  `None` with the reason on the arm. Each needs its own rules and a row per
  rule in the conformance suite — claiming support without that is the failure
  this whole patch exists to prevent.
* **Markdown inline spans** (emphasis, links) — the crate ships a second
  grammar for them and Suisei parses one tree per document. Needs injection
  support in the pipeline.
* **Filename-identified languages** — `Makefile`, `Dockerfile`, `CMakeLists.txt`
  have no extension, so nothing resolves them. `Lang::from_filename` would be a
  small addition once something needs it.
* **Kotlin** — would need a hand-written highlights query.
