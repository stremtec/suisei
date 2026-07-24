//! Tree-sitter syntax highlighting via **highlight queries** (`highlights.scm`).
//!
//! Tree-sitter columns are **byte** offsets; the editor uses **char** indices.
//! Query captures map to [`TokenKind`] through [`crate::highlight::from_capture`].

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor, Tree};

use crate::highlight::{self, TokenKind};

/// A parse kept for a file the user is not looking at right now.
///
/// Opening a file costs one cold parse; edits after that are incremental. The
/// project indexer pays that cost up front in the background so the FIRST touch
/// of a long file is not the one that stalls.
struct CachedParse {
    text: String,
    tree: Tree,
    ext: String,
}

/// One highlight span: (kind, start_col, end_col, row) — char columns, end exclusive.
pub type HlToken = (TokenKind, usize, usize, usize);

struct LangBundle {
    parser: Parser,
    query: Query,
}

pub struct SyntaxEngine {
    rust: Option<LangBundle>,
    python: Option<LangBundle>,
    javascript: Option<LangBundle>,
    typescript: Option<LangBundle>,
    tsx: Option<LangBundle>,
    c: Option<LangBundle>,
    go: Option<LangBundle>,
    bash: Option<LangBundle>,
    json: Option<LangBundle>,
    tree: Option<Tree>,
    last_ext: String,
    last_len: usize,
    last_fingerprint: u64,
    /// Text `tree` was parsed from — the reference for computing the edit
    /// descriptor that lets the NEXT parse be incremental.
    last_text: String,
    /// Query-based tokens (char columns). Covers `token_rows` only — the
    /// highlight query is limited to what the viewport can show, because
    /// rebuilding every token in the file was the whole remaining typing cost
    /// once parsing became incremental (2.65ms of 2.75ms at 6k lines).
    pub tokens: Vec<HlToken>,
    /// Row window `tokens` was built for (end exclusive).
    token_rows: std::ops::Range<usize>,
    /// Path the live tree belongs to — switching files must not diff against
    /// another file's text.
    last_path: String,
    /// Pre-parsed trees by path, oldest evicted first. Bounded: a tree runs a
    /// few MB, and holding every file a big project ever opened is how an
    /// editor quietly grows to a gigabyte.
    cache: std::collections::HashMap<String, CachedParse>,
    cache_order: std::collections::VecDeque<String>,
    pub active: bool,
}

impl Default for SyntaxEngine {
    fn default() -> Self {
        Self {
            rust: make_lang(
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY,
            ),
            python: make_lang(
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY,
            ),
            javascript: make_lang(
                tree_sitter_javascript::LANGUAGE.into(),
                tree_sitter_javascript::HIGHLIGHT_QUERY,
            ),
            typescript: make_lang(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            ),
            tsx: make_lang(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                // TSX uses the same highlights as TS + JSX patterns when available
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            ),
            c: make_lang(
                tree_sitter_c::LANGUAGE.into(),
                tree_sitter_c::HIGHLIGHT_QUERY,
            ),
            go: make_lang(tree_sitter_go::LANGUAGE.into(), tree_sitter_go::HIGHLIGHTS_QUERY),
            bash: make_lang(
                tree_sitter_bash::LANGUAGE.into(),
                tree_sitter_bash::HIGHLIGHT_QUERY,
            ),
            json: make_lang(
                tree_sitter_json::LANGUAGE.into(),
                tree_sitter_json::HIGHLIGHTS_QUERY,
            ),
            tree: None,
            last_ext: String::new(),
            last_len: 0,
            last_fingerprint: 0,
            last_text: String::new(),
            tokens: Vec::new(),
            token_rows: 0..0,
            last_path: String::new(),
            cache: std::collections::HashMap::new(),
            cache_order: std::collections::VecDeque::new(),
            active: false,
        }
    }
}

fn make_lang(language: Language, source: &str) -> Option<LangBundle> {
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return None;
    }
    let query = Query::new(&language, source).ok()?;
    Some(LangBundle { parser, query })
}

impl SyntaxEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reparse (incrementally) and rebuild tokens for the whole file.
    pub fn parse(&mut self, text: &str, ext: Option<&str>) {
        self.parse_window(text, ext, None)
    }

    /// Parse for a known path, seeding from the pre-warmed cache when possible.
    ///
    /// Switching files used to mean a cold parse every time. If the indexer has
    /// already parsed this exact text, the tree is adopted and only the token
    /// query runs.
    pub fn parse_path(
        &mut self,
        path: &str,
        text: &str,
        ext: Option<&str>,
        rows: Option<std::ops::Range<usize>>,
    ) {
        if path != self.last_path {
            // Park the outgoing file so coming back is warm too.
            if !self.last_path.is_empty() && self.tree.is_some() {
                let tree = self.tree.clone();
                if let Some(tree) = tree {
                    let (p, t, e) =
                        (self.last_path.clone(), self.last_text.clone(), self.last_ext.clone());
                    self.store_cached(p, t, tree, e);
                }
            }
            // Adopt a pre-warmed tree when the text still matches.
            let hit = self
                .cache
                .get(path)
                .filter(|c| c.text == text)
                .map(|c| (c.tree.clone(), c.ext.clone()));
            match hit {
                Some((tree, ext_cached)) => {
                    self.tree = Some(tree);
                    self.last_text.clear();
                    self.last_text.push_str(text);
                    self.last_ext = ext_cached;
                    self.last_len = text.len();
                    self.last_fingerprint = fingerprint_text(text);
                    // Tokens belong to the viewport, so they are rebuilt below.
                    self.tokens.clear();
                    self.token_rows = 0..0;
                }
                None => {
                    self.tree = None;
                    self.last_text.clear();
                    self.tokens.clear();
                    self.token_rows = 0..0;
                }
            }
            self.last_path = path.to_string();
        }
        self.parse_window(text, ext, rows);
    }

    /// Parse `text` in the background and keep the tree for `path`.
    ///
    /// Deliberately does NOT disturb the live document: this runs while the
    /// user is editing something else.
    pub fn prewarm(&mut self, path: &str, text: &str, ext: Option<&str>) {
        if self.cache.contains_key(path) {
            return;
        }
        let Some(bundle) = self.bundle_for(ext) else { return };
        let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bundle.parser.parse(text, None)
        }));
        if let Ok(Some(tree)) = parsed {
            self.store_cached(
                path.to_string(),
                text.to_string(),
                tree,
                ext.unwrap_or("").to_string(),
            );
        }
    }

    /// How many files are pre-parsed right now.
    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }

    fn store_cached(&mut self, path: String, text: String, tree: Tree, ext: String) {
        const MAX_CACHED: usize = 48;
        if self.cache.insert(path.clone(), CachedParse { text, tree, ext }).is_none() {
            self.cache_order.push_back(path);
        }
        while self.cache_order.len() > MAX_CACHED {
            if let Some(old) = self.cache_order.pop_front() {
                self.cache.remove(&old);
            }
        }
    }

    fn bundle_for(&mut self, ext: Option<&str>) -> Option<&mut LangBundle> {
        match ext {
            Some("rs") => self.rust.as_mut(),
            Some("py" | "pyi") => self.python.as_mut(),
            Some("js" | "mjs" | "cjs") | Some("jsx") => self.javascript.as_mut(),
            Some("ts" | "mts" | "cts") => self.typescript.as_mut(),
            Some("tsx") => self.tsx.as_mut().or(self.typescript.as_mut()),
            Some("c" | "h") | Some("cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx") => self.c.as_mut(),
            Some("go") => self.go.as_mut(),
            Some("sh" | "bash" | "zsh") => self.bash.as_mut(),
            Some("json" | "jsonc") => self.json.as_mut(),
            _ => None,
        }
    }

    /// `rows` limits the highlight query to a row window — pass the viewport
    /// plus overscan. `None` highlights everything (TUI / tests).
    pub fn parse_window(
        &mut self,
        text: &str,
        ext: Option<&str>,
        rows: Option<std::ops::Range<usize>>,
    ) {
        let ext_str = ext.unwrap_or("");
        let fingerprint = fingerprint_text(text);

        // Skip full work when content unchanged
        let covered = match &rows {
            Some(r) => self.token_rows.start <= r.start && self.token_rows.end >= r.end,
            None => self.token_rows == (0..usize::MAX),
        };
        if self.active
            && self.last_ext == ext_str
            && self.last_len == text.len()
            && self.last_fingerprint == fingerprint
            && !self.tokens.is_empty()
            && covered
        {
            return;
        }

        // Reuse the previous tree when this is an edit to the SAME document in
        // the same language. `tree.edit()` tells tree-sitter which bytes moved,
        // turning an O(file) reparse into O(change) — the dominant cost on the
        // typing path. The old code dropped the tree every time because an
        // incremental parse WITHOUT this descriptor panics inside tree-sitter;
        // supplying it is the fix, and an unclear diff still falls back to a
        // full parse.
        let reuse = if self.last_ext == ext_str && !self.last_text.is_empty() {
            match (self.tree.take(), edit_between(&self.last_text, text)) {
                (Some(mut old), Some(edit)) => {
                    old.edit(&edit);
                    Some(old)
                }
                _ => None,
            }
        } else {
            None
        };

        let bundle = match ext {
            Some("rs") => self.rust.as_mut(),
            Some("py" | "pyi") => self.python.as_mut(),
            Some("js" | "mjs" | "cjs") => self.javascript.as_mut(),
            Some("jsx") => self.javascript.as_mut(),
            Some("ts" | "mts" | "cts") => self.typescript.as_mut(),
            Some("tsx") => self.tsx.as_mut().or(self.typescript.as_mut()),
            Some("c" | "h") => self.c.as_mut(),
            Some("cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx") => self.c.as_mut(),
            Some("go") => self.go.as_mut(),
            Some("sh" | "bash" | "zsh") => self.bash.as_mut(),
            Some("json" | "jsonc") => self.json.as_mut(),
            _ => {
                self.tokens.clear();
                self.tree = None;
                self.last_ext.clear();
                self.last_len = 0;
                self.last_fingerprint = 0;
                self.active = false;
                return;
            }
        };

        let Some(bundle) = bundle else {
            self.tokens.clear();
            self.active = false;
            return;
        };

        self.active = true;
        let len = text.len();

        // `reuse` (above) already took the tree and applied the edit; wrap ALL
        // ts calls in catch_unwind so a binding panic never kills the process.
        self.tree = None;
        self.tokens.clear();
        self.last_ext = ext_str.to_string();
        self.last_len = len;
        self.last_fingerprint = fingerprint;

        let source = text.as_bytes();
        let lines: Vec<&str> = text.split('\n').collect();
        let capture_names = bundle.query.capture_names().to_vec();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let Some(tree) = bundle.parser.parse(text, reuse.as_ref()) else {
                return None;
            };
            let root = tree.root_node();
            let mut cursor = QueryCursor::new();
            if let Some(r) = &rows {
                // Nodes that merely intersect the window still match, so a block
                // comment starting above the viewport keeps its colour.
                cursor.set_point_range(
                    tree_sitter::Point { row: r.start, column: 0 }
                        ..tree_sitter::Point { row: r.end, column: 0 },
                );
            }
            let mut tokens: Vec<HlToken> = Vec::new();
            let mut matches = cursor.matches(&bundle.query, root, source);
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    let name = capture_names
                        .get(cap.index as usize)
                        .copied()
                        .unwrap_or("");
                    let Some(kind) = highlight::from_capture(name) else {
                        continue;
                    };
                    let node = cap.node;
                    let end_byte = node.end_byte();
                    if end_byte > source.len() || node.start_byte() > end_byte {
                        continue;
                    }
                    push_node_tokens(node, &lines, kind, &mut tokens);
                }
            }
            tokens.sort_by_key(|(_, st, ed, row)| (*row, ed.saturating_sub(*st), *st));
            Some((tree, tokens))
        }));

        match result {
            Ok(Some((tree, tokens))) => {
                self.tree = Some(tree);
                self.tokens = tokens;
                self.token_rows = rows.clone().unwrap_or(0..usize::MAX);
                // Anchor for the next incremental parse.
                self.last_text.clear();
                self.last_text.push_str(text);
            }
            Ok(None) => {
                self.tree = None;
                self.tokens.clear();
                self.token_rows = 0..0;
                self.last_text.clear();
            }
            Err(_) => {
                // tree-sitter panicked — stay alive with no highlight, and drop
                // the incremental anchor so the next parse starts clean.
                self.tree = None;
                self.tokens.clear();
                self.token_rows = 0..0;
                self.last_text.clear();
                self.active = false;
            }
        }
    }

    /// Whether `tokens` already covers `rows` — lets the caller skip the
    /// O(file) `buffer.text()` join when nothing needs re-highlighting.
    pub fn covers_rows(&self, rows: &std::ops::Range<usize>) -> bool {
        self.active && self.token_rows.start <= rows.start && self.token_rows.end >= rows.end
    }

    /// Contiguous slice of highlight tokens on `row`. `tokens` is kept sorted by
    /// row (see the `sort_by_key` in `parse`), so this is an O(log n) binary
    /// search instead of the O(n) whole-file filter the renderer used to run for
    /// every visible row on every frame.
    pub fn tokens_for_row(&self, row: usize) -> &[HlToken] {
        let lo = self.tokens.partition_point(|t| t.3 < row);
        let hi = self.tokens.partition_point(|t| t.3 <= row);
        &self.tokens[lo..hi]
    }
}

fn push_node_tokens(
    node: tree_sitter::Node,
    lines: &[&str],
    kind: TokenKind,
    tokens: &mut Vec<HlToken>,
) {
    let start = node.start_position();
    let end = node.end_position();

    // Safety: skip huge multi-line non-comment/string spans
    if start.row != end.row && !matches!(kind, TokenKind::Comment | TokenKind::String) {
        return;
    }

    if start.row == end.row {
        if let Some(line) = lines.get(start.row) {
            let scol = byte_col_to_char_col(line, start.column);
            let ecol = byte_col_to_char_col(line, end.column);
            if scol < ecol {
                tokens.push((kind, scol, ecol, start.row));
            }
        }
        return;
    }

    // Multi-line comments / strings
    if let Some(line) = lines.get(start.row) {
        let scol = byte_col_to_char_col(line, start.column);
        let ecol = line.chars().count();
        if scol < ecol {
            tokens.push((kind, scol, ecol, start.row));
        }
    }
    for row in start.row + 1..end.row {
        if let Some(line) = lines.get(row) {
            let ecol = line.chars().count();
            if ecol > 0 {
                tokens.push((kind, 0, ecol, row));
            }
        }
    }
    if let Some(line) = lines.get(end.row) {
        let ecol = byte_col_to_char_col(line, end.column);
        if ecol > 0 {
            tokens.push((kind, 0, ecol, end.row));
        }
    }
}

/// The byte range that differs between `old` and `new`, as tree-sitter's edit
/// descriptor. Found by a common prefix/suffix scan: ordinary typing changes a
/// couple of bytes, so this is a raw memcmp-speed walk versus a full reparse.
/// Returns `None` when the texts are equal or the diff is not expressible,
/// which makes the caller fall back to a full parse.
fn edit_between(old: &str, new: &str) -> Option<tree_sitter::InputEdit> {
    if old == new {
        return None;
    }
    let (ob, nb) = (old.as_bytes(), new.as_bytes());

    let max_pre = ob.len().min(nb.len());
    let mut pre = 0;
    while pre < max_pre && ob[pre] == nb[pre] {
        pre += 1;
    }
    // Back off to a boundary that is valid in BOTH strings — splitting a
    // multi-byte char would hand tree-sitter a nonsense range.
    while pre > 0 && (!old.is_char_boundary(pre) || !new.is_char_boundary(pre)) {
        pre -= 1;
    }

    let max_suf = max_pre - pre;
    let mut suf = 0;
    while suf < max_suf && ob[ob.len() - 1 - suf] == nb[nb.len() - 1 - suf] {
        suf += 1;
    }
    while suf > 0
        && (!old.is_char_boundary(ob.len() - suf) || !new.is_char_boundary(nb.len() - suf))
    {
        suf -= 1;
    }

    let start = pre;
    let old_end = ob.len() - suf;
    let new_end = nb.len() - suf;
    if start > old_end || start > new_end {
        return None;
    }

    Some(tree_sitter::InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point_at(old, start),
        old_end_position: point_at(old, old_end),
        new_end_position: point_at(new, new_end),
    })
}

/// tree-sitter `Point` for a byte offset. `column` is a BYTE offset within the
/// row (tree-sitter's convention), not a char index.
fn point_at(text: &str, byte: usize) -> tree_sitter::Point {
    let b = byte.min(text.len());
    let upto = &text.as_bytes()[..b];
    let row = upto.iter().filter(|&&c| c == b'\n').count();
    let line_start = upto.iter().rposition(|&c| c == b'\n').map(|i| i + 1).unwrap_or(0);
    tree_sitter::Point { row, column: b - line_start }
}

fn byte_col_to_char_col(line: &str, byte_col: usize) -> usize {
    if byte_col == 0 {
        return 0;
    }
    if byte_col >= line.len() {
        return line.chars().count();
    }
    let mut idx = byte_col;
    while idx > 0 && !line.is_char_boundary(idx) {
        idx -= 1;
    }
    line.get(..idx).map(|s| s.chars().count()).unwrap_or(0)
}

/// Full-content FNV-1a fingerprint (skip re-query only when bytes are identical).
fn fingerprint_text(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in text.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    // Mix length so empty vs non-empty always differ
    for b in text.len().to_le_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_to_char_ascii() {
        assert_eq!(byte_col_to_char_col("hello", 0), 0);
        assert_eq!(byte_col_to_char_col("hello", 5), 5);
    }

    #[test]
    fn byte_to_char_cjk() {
        let line = "a한b";
        assert_eq!(byte_col_to_char_col(line, 0), 0);
        assert_eq!(byte_col_to_char_col(line, 1), 1);
        assert_eq!(byte_col_to_char_col(line, 4), 2);
    }

    #[test]
    fn parse_rust_produces_tokens() {
        let mut eng = SyntaxEngine::new();
        eng.parse("fn main() { let x = 1; }", Some("rs"));
        assert!(eng.active);
        assert!(!eng.tokens.is_empty(), "expected query tokens");
    }

    #[test]
    fn rust_does_not_paint_whole_function_as_one_token() {
        let mut eng = SyntaxEngine::new();
        let src = "fn main() {\n    let x = 42;\n    let s = \"hi\";\n}\n";
        eng.parse(src, Some("rs"));
        let first_line_len = src.lines().next().unwrap().chars().count();
        let paints_whole_line = eng.tokens.iter().any(|(k, st, ed, row)| {
            *row == 0 && *st == 0 && *ed >= first_line_len && matches!(k, TokenKind::Keyword)
        });
        assert!(
            !paints_whole_line,
            "keyword token painted entire first line: {:?}",
            eng.tokens
        );
        let has_number = eng
            .tokens
            .iter()
            .any(|(k, _, _, _)| matches!(k, TokenKind::Number));
        let has_string = eng
            .tokens
            .iter()
            .any(|(k, _, _, _)| matches!(k, TokenKind::String));
        assert!(
            has_number || has_string,
            "expected number/string tokens, got {:?}",
            eng.tokens
        );
    }

    #[test]
    fn rust_highlights_fn_and_function_name() {
        let mut eng = SyntaxEngine::new();
        eng.parse("fn main() {}", Some("rs"));
        assert!(eng.active);
        assert!(!eng.tokens.is_empty());
    }

    #[test]
    fn python_query_active() {
        let mut eng = SyntaxEngine::new();
        eng.parse("def foo(x):\n    return x + 1\n", Some("py"));
        assert!(eng.active);
        assert!(!eng.tokens.is_empty());
    }

    #[test]
    fn go_and_json_query_active() {
        let mut eng = SyntaxEngine::new();
        eng.parse("package main\nfunc Hello() {}\n", Some("go"));
        assert!(eng.active);
        assert!(!eng.tokens.is_empty());
        eng.parse(r#"{"a": 1, "b": "x"}"#, Some("json"));
        assert!(eng.active);
        assert!(!eng.tokens.is_empty());
    }

    #[test]
    fn skip_reparse_when_unchanged() {
        let mut eng = SyntaxEngine::new();
        eng.parse("fn a() {}", Some("rs"));
        let n = eng.tokens.len();
        eng.parse("fn a() {}", Some("rs"));
        assert_eq!(eng.tokens.len(), n);
    }

    #[test]
    fn rapid_edits_do_not_panic() {
        // Regression: incremental parse without tree.edit() panicked with
        // "range start index N out of range for slice of length M".
        let mut eng = SyntaxEngine::new();
        let mut src = String::from("fn main() {\n    let x = 1;\n}\n");
        eng.parse(&src, Some("rs"));
        for i in 0..80 {
            src.insert(src.len().saturating_sub(2), char::from(b'a' + (i % 26) as u8));
            eng.parse(&src, Some("rs"));
            // delete a char near the middle
            if src.len() > 10 {
                let mid = src.len() / 2;
                if src.is_char_boundary(mid) {
                    src.remove(mid);
                }
                eng.parse(&src, Some("rs"));
            }
        }
        assert!(eng.active || eng.tokens.is_empty());
    }

    #[test]
    fn switch_language_and_edit() {
        let mut eng = SyntaxEngine::new();
        eng.parse("fn foo() {}", Some("rs"));
        eng.parse("def foo():\n  pass\n", Some("py"));
        eng.parse("fn bar() { let y = 2; }", Some("rs"));
        assert!(eng.tokens.iter().any(|t| t.0 == TokenKind::Keyword || t.0 == TokenKind::Function));
    }

    /// A wrong edit descriptor does not panic — it silently produces a WRONG
    /// tree. So incremental output must be byte-identical to a cold full parse.
    #[test]
    fn incremental_parse_matches_full_parse() {
        let base = "fn main() {\n    let x = 1;\n    println!(\"hi\");\n}\n";
        let mut inc = SyntaxEngine::new();
        inc.parse(base, Some("rs"));

        // Edits at the front, middle and end, plus a deletion and a multi-byte
        // insert — each one reuses the tree from the previous round.
        let mut text = base.to_string();
        let steps: Vec<Box<dyn Fn(&mut String)>> = vec![
            Box::new(|t: &mut String| t.insert_str(0, "// lead\n")),
            Box::new(|t: &mut String| {
                let at = t.find("let x").unwrap();
                t.insert_str(at, "let y = 2; ");
            }),
            Box::new(|t: &mut String| t.push_str("fn tail() {}\n")),
            Box::new(|t: &mut String| {
                let at = t.find("let y = 2; ").unwrap();
                t.replace_range(at..at + "let y = 2; ".len(), "");
            }),
            Box::new(|t: &mut String| {
                let at = t.find("\"hi\"").unwrap();
                t.replace_range(at..at + 4, "\"안녕 🌏\"");
            }),
        ];

        for step in steps {
            step(&mut text);
            inc.parse(&text, Some("rs"));

            let mut full = SyntaxEngine::new();
            full.parse(&text, Some("rs"));

            assert_eq!(
                inc.tokens, full.tokens,
                "incremental tokens diverged from a full parse after edit; text was:\n{text}"
            );
        }
    }
}
