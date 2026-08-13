//! Tree-sitter syntax highlighting via **highlight queries** (`highlights.scm`).
//!
//! Tree-sitter columns are **byte** offsets; the editor uses **char** indices.
//! Query captures map to [`TokenKind`] through [`crate::highlight::from_capture`].

use streaming_iterator::StreamingIterator;
use tree_sitter::{QueryCursor, Tree};

use crate::highlight::{self, TokenKind};
use crate::lang::{Grammars, Lang, LangBundle};

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

pub struct SyntaxEngine {
    /// Every grammar, behind one lookup. See `crate::lang` for why this is a
    /// field of its own rather than a method on the engine.
    grammars: Grammars,
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
            // Grammars compile their highlight queries on first use (A1-6):
            // the main-thread engine only ADOPTS worker frames and may never
            // parse at all, so compiling every query at startup would be waste
            // — and at 29 grammars it would be a very expensive waste.
            grammars: Grammars::default(),
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

impl SyntaxEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reparse (incrementally) and rebuild tokens for the whole file.
    pub fn parse(&mut self, text: &str, ext: Option<&str>) {
        self.parse_window(text, ext, None)
    }

    /// The live tree, and the exact text it was parsed from.
    ///
    /// Both, together, or neither: a tree indexed against stale text yields
    /// byte offsets that name the wrong identifiers. Completion's scope walk
    /// (`crate::scope`) is the caller — it needs a parse that is already warm,
    /// because re-parsing on every keystroke to answer "what is in scope" would
    /// cost more than the suggestion is worth.
    pub fn live_tree(&self) -> Option<(&Tree, &str)> {
        self.tree.as_ref().map(|t| (t, self.last_text.as_str()))
    }

    /// Extension the live tree was parsed as.
    pub fn live_ext(&self) -> &str {
        &self.last_ext
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
                    let (p, t, e) = (
                        self.last_path.clone(),
                        self.last_text.clone(),
                        self.last_ext.clone(),
                    );
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
        let Some(bundle) = self.bundle_for(ext) else {
            return;
        };
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

    /// How many grammars are compiled right now. Used by the tests that pin
    /// what a first parse is allowed to cost.
    pub fn grammars_loaded(&self) -> usize {
        self.grammars.loaded_count()
    }

    fn store_cached(&mut self, path: String, text: String, tree: Tree, ext: String) {
        const MAX_CACHED: usize = 48;
        if self
            .cache
            .insert(path.clone(), CachedParse { text, tree, ext })
            .is_none()
        {
            self.cache_order.push_back(path);
        }
        while self.cache_order.len() > MAX_CACHED {
            if let Some(old) = self.cache_order.pop_front() {
                self.cache.remove(&old);
            }
        }
    }

    /// Eagerly build every language's parser + highlight query. Grammars are
    /// otherwise lazy — the first file of a type pays a cold parser+`Query`
    /// build (query compilation is the slow part). The boot pipeline warms
    /// them off the worker thread so the first highlight of the first file
    /// opened — of any language — is instant. Idempotent: `Grammars::get`
    /// no-ops once a slot is filled.
    pub fn warm_all(&mut self) {
        for lang in Lang::ALL {
            self.warm_one(*lang);
        }
    }

    /// Build one grammar. The worker drains the table a language at a time on
    /// idle turns rather than calling `warm_all`, because the whole table is
    /// 780 ms and a parse waiting behind it is a file with no colours.
    pub fn warm_one(&mut self, lang: Lang) {
        let _ = self.grammars.get(lang);
    }

    fn bundle_for(&mut self, ext: Option<&str>) -> Option<&mut LangBundle> {
        self.grammars.for_ext(ext)
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

        // Same bytes as the tree on hand — only the WINDOW moved (a scroll).
        // Re-run the query alone; reparsing identical text is pure waste
        // (`edit_between` returns None for equal texts, which would force a
        // full parse instead).
        if self.active && self.last_ext == ext_str && self.last_fingerprint == fingerprint {
            if let Some(tree) = self.tree.clone() {
                if let Some(bundle) = self.bundle_for(ext) {
                    let requery = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        query_tree(bundle, &tree, text, rows.clone())
                    }));
                    if let Ok(tokens) = requery {
                        self.tokens = tokens;
                        self.token_rows = rows.unwrap_or(0..usize::MAX);
                        return;
                    }
                }
            }
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

        // No grammar for this extension: the file is not tier 1, so drop any
        // state left from the previous document and let the regex fallback in
        // `highlight.rs` take it. Distinct from "grammar exists but failed to
        // build", handled just below — that one keeps `last_ext` so the next
        // parse of the same file does not look like a language switch.
        if Lang::from_ext(ext.unwrap_or("")).is_none() {
            self.tokens.clear();
            self.token_rows = 0..0;
            self.tree = None;
            self.last_ext.clear();
            self.last_len = 0;
            self.last_fingerprint = 0;
            self.active = false;
            return;
        }

        // Borrows `self.grammars` alone, which is what lets the rest of the
        // engine stay writable below. That borrow is the entire reason this
        // table used to be written out twice.
        let Some(bundle) = self.grammars.for_ext(ext) else {
            self.tokens.clear();
            self.token_rows = 0..0;
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

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let Some(tree) = bundle.parser.parse(text, reuse.as_ref()) else {
                return None;
            };
            let tokens = query_tree(bundle, &tree, text, rows.clone());
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
    /// Slide the painted spans on `row` to match a single-line insertion, so a
    /// just-typed character is coloured NOW instead of one tick from now.
    ///
    /// The real answer still comes from the worker; this only keeps what is
    /// already on screen honest until it lands. Waiting for the parse instead
    /// was measured at 4.1 ms (1k lines) to 6.7 ms (4.5k lines) per keystroke
    /// against 0.4 ms without — half a frame, on the hottest path in the app.
    /// This costs a pass over one row's spans.
    ///
    /// Three cases, and the third is the one that matters:
    ///
    /// * a span entirely after the caret — shift it right;
    /// * a span containing the caret — stretch it;
    /// * a span ENDING exactly at the caret — stretch it too, because that is
    ///   what typing at the end of a word is, and the new character belongs to
    ///   the word being typed.
    ///
    /// Wrong only at a boundary where the edit changes what the token IS — say
    /// typing `"` mid-identifier — and then only until the parse lands.
    pub fn nudge_for_insert(&mut self, row: usize, col: usize, chars: usize) {
        if chars == 0 {
            return;
        }
        for t in &mut self.tokens {
            if t.3 != row {
                continue;
            }
            if t.1 >= col {
                t.1 += chars;
                t.2 += chars;
            } else if t.2 >= col {
                t.2 += chars;
            }
        }
    }

    /// A line was SPLIT at `row`/`col`: everything below moves down one row.
    ///
    /// The column nudges above cover an edit inside one line, which is every
    /// keystroke except the one that changes how many lines there are. Return
    /// was not covered, so after it every token below the caret still named the
    /// row it used to be on — the stale colours the paint path is designed to
    /// keep going were suddenly the wrong stale colours, and the file appeared
    /// to lose its highlighting until the worker's next frame landed.
    ///
    /// The split row's own tokens are divided at `col`: what was left of the
    /// break stays, what was right of it moves to the new row and back to
    /// column zero.
    pub fn nudge_for_split(&mut self, row: usize, col: usize) {
        for t in &mut self.tokens {
            if t.3 > row {
                t.3 += 1;
                continue;
            }
            if t.3 != row {
                continue;
            }
            if t.1 >= col {
                // Wholly after the break: it belongs to the new row.
                t.1 -= col;
                t.2 -= col;
                t.3 += 1;
            } else if t.2 > col {
                // Straddles it. Truncate here rather than splitting in two:
                // this is a one-frame approximation, and the worker's answer
                // is already on its way.
                t.2 = col;
            }
        }
    }

    /// Two lines were JOINED — `row + 1` merged onto the end of `row` at `col`.
    /// The inverse of [`Self::nudge_for_split`], for Backspace at a line start.
    pub fn nudge_for_join(&mut self, row: usize, col: usize) {
        for t in &mut self.tokens {
            if t.3 == row + 1 {
                t.3 = row;
                t.1 += col;
                t.2 += col;
            } else if t.3 > row + 1 {
                t.3 -= 1;
            }
        }
    }

    /// The same, for a single-line deletion of `chars` ending at `col`.
    ///
    /// Spans that the deletion empties are dropped rather than left as
    /// zero-width, which would paint nothing but still cost a span.
    pub fn nudge_for_delete(&mut self, row: usize, col: usize, chars: usize) {
        if chars == 0 {
            return;
        }
        let start = col.saturating_sub(chars);
        let clamp = |v: usize| -> usize {
            if v <= start {
                v
            } else if v >= col {
                v - chars
            } else {
                start
            }
        };
        for t in &mut self.tokens {
            if t.3 != row {
                continue;
            }
            t.1 = clamp(t.1);
            t.2 = clamp(t.2);
        }
        self.tokens.retain(|t| t.3 != row || t.2 > t.1);
    }

    pub fn tokens_for_row(&self, row: usize) -> &[HlToken] {
        let lo = self.tokens.partition_point(|t| t.3 < row);
        let hi = self.tokens.partition_point(|t| t.3 <= row);
        &self.tokens[lo..hi]
    }

    /// Adopt a frame parsed by the background worker (A1-6). This engine
    /// instance holds what the renderer consumes; the tree and the
    /// incremental anchor live on the worker's twin.
    /// Returns true when the frame changes what gets PAINTED (tokens or
    /// `active`) — adopting an empty frame over an empty state (untitled
    /// document, first answer) must not schedule a redraw that draws
    /// nothing.
    /// Adopt a worker frame: its tokens AND its parse.
    ///
    /// This used to end with `self.tree = None; self.last_text.clear()`, on the
    /// reasoning that the tree lived on the worker and the main thread only
    /// painted tokens. True for highlighting — and fatal for anything else that
    /// asks about structure. `live_tree()` was therefore `None` for the entire
    /// life of the GUI, so scope-aware completion returned an empty symbol list
    /// on every keystroke and quietly degraded to keywords. It passed its tests
    /// because they call `parse()` directly, which is the TUI path.
    ///
    /// The tree now travels with the tokens. Callers that have no tree to give
    /// still clear it, so a frame can never leave a tree that disagrees with
    /// the text beside it.
    pub fn apply_frame(
        &mut self,
        path: String,
        window: std::ops::Range<usize>,
        tokens: Vec<HlToken>,
        active: bool,
        tree: Option<Tree>,
        text: String,
        ext: String,
    ) -> bool {
        let changed = self.tokens != tokens || self.active != active;
        self.last_path = path;
        self.tokens = tokens;
        self.token_rows = window;
        self.active = active;
        // Tree and text are one fact. Byte offsets from `visible_at` index into
        // `last_text`, so a tree without its own text would resolve carets
        // against a different document.
        match tree {
            Some(t) => {
                self.tree = Some(t);
                self.last_text = text;
                self.last_ext = ext;
            }
            None => {
                self.tree = None;
                self.last_text.clear();
            }
        }
        changed
    }

    /// Drop painted tokens without a replacement — a document switch, where
    /// another file's colours would lie about this text until the worker's
    /// first frame lands.
    pub fn clear_tokens(&mut self) {
        self.tokens.clear();
        self.token_rows = 0..0;
        self.active = false;
        self.last_path.clear();
    }

    /// Path the painted tokens belong to ("" once cleared / never painted).
    pub fn applied_path(&self) -> &str {
        &self.last_path
    }
}

/// Run the highlight query over an already-parsed tree, limited to `rows`.
///
/// Nodes that merely intersect the window still match, so a block comment
/// starting above the viewport keeps its colour. Shared by the parse path
/// and the scroll re-query (same tree, new window — no reparse).
fn query_tree(
    bundle: &LangBundle,
    tree: &Tree,
    text: &str,
    rows: Option<std::ops::Range<usize>>,
) -> Vec<HlToken> {
    let source = text.as_bytes();
    let lines: Vec<&str> = text.split('\n').collect();
    let capture_names = bundle.query.capture_names().to_vec();
    let root = tree.root_node();
    let mut cursor = QueryCursor::new();
    if let Some(r) = &rows {
        cursor.set_point_range(
            tree_sitter::Point {
                row: r.start,
                column: 0,
            }..tree_sitter::Point {
                row: r.end,
                column: 0,
            },
        );
    }
    let mut tokens: Vec<HlToken> = Vec::new();
    let mut matches = cursor.matches(&bundle.query, root, source);
    while let Some(m) = matches.next() {
        for cap in m.captures {
            let name = capture_names.get(cap.index as usize).copied().unwrap_or("");
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
    flatten_overlaps(tokens)
}

/// Turn overlapping captures into non-overlapping spans, letting the INNER one
/// win.
///
/// Captures nest, because they are tree nodes: an escape sequence sits inside a
/// string, a type argument's brackets sit inside a type. The face paints spans
/// in array order with `addAttributes`, so whichever span is applied LAST is
/// the colour that shows. The old ordering was by width ascending, which meant
/// the widest — least specific — span was applied last and overwrote every
/// precise one inside it. Escapes never showed a different colour from their
/// string; nothing did.
///
/// Two things depend on this being resolved here rather than at paint time:
/// the face's `.take(32)` per row now keeps the leftmost 32 real spans instead
/// of the 32 narrowest fragments, and composing two highlight queries (see
/// `lang.rs`) becomes well defined — for the same range the later pattern wins,
/// which is why the language-specific overlay is concatenated last.
pub fn flatten_overlaps(mut tokens: Vec<HlToken>) -> Vec<HlToken> {
    if tokens.len() < 2 {
        return tokens;
    }
    // Pre-order within a row: outer before inner. For two captures on the exact
    // same range the sort is a tie, and a stable sort keeps the order they were
    // pushed in — so the later pattern ends up "inside" the earlier one and
    // wins, which is the composition rule `lang.rs` relies on.
    tokens.sort_by(|a, b| (a.3, a.1).cmp(&(b.3, b.1)).then(b.2.cmp(&a.2)));

    let mut out: Vec<HlToken> = Vec::with_capacity(tokens.len());
    // (kind, cursor, end, row) — `cursor` is the next column of this span not
    // yet covered by something nested inside it.
    let mut stack: Vec<HlToken> = Vec::new();
    let close = |stack: &mut Vec<HlToken>, out: &mut Vec<HlToken>, upto: Option<(usize, usize)>| {
        while let Some(top) = stack.last() {
            let done = match upto {
                Some((row, col)) => top.3 != row || top.2 <= col,
                None => true,
            };
            if !done {
                break;
            }
            let top = stack.pop().expect("just peeked");
            if top.1 < top.2 {
                out.push(top);
            }
        }
    };

    for t in tokens {
        close(&mut stack, &mut out, Some((t.3, t.1)));
        if let Some(top) = stack.last_mut() {
            if top.1 < t.1 {
                out.push((top.0, top.1, t.1, top.3));
            }
            // Resume the enclosing span after the nested one. Clamped both
            // ways: a capture that starts before the cursor or runs past the
            // parent's end must not move it backwards or beyond.
            top.1 = top.1.max(t.2).min(top.2);
        }
        stack.push(t);
    }
    close(&mut stack, &mut out, None);

    out.sort_by_key(|t| (t.3, t.1));
    out
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
    let line_start = upto
        .iter()
        .rposition(|&c| c == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    tree_sitter::Point {
        row,
        column: b - line_start,
    }
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
mod nudge_tests {
    use super::*;
    use crate::highlight::TokenKind;

    fn engine_with(tokens: Vec<HlToken>) -> SyntaxEngine {
        let mut e = SyntaxEngine::new();
        e.tokens = tokens;
        e
    }

    #[test]
    fn a_split_moves_everything_below_down_one_row() {
        // (kind, start, end, row)
        let mut e = engine_with(vec![
            (TokenKind::Keyword, 0, 3, 5),
            (TokenKind::Function, 0, 4, 9),
        ]);
        e.nudge_for_split(5, 3);
        assert_eq!(e.tokens[1].3, 10, "the row below followed the new line");
    }

    #[test]
    fn a_split_divides_the_row_it_happens_on() {
        let mut e = engine_with(vec![
            (TokenKind::Keyword, 0, 3, 5),   // before the break
            (TokenKind::Function, 6, 9, 5),     // after it
            (TokenKind::String, 2, 8, 5),    // straddling it
        ]);
        e.nudge_for_split(5, 4);
        assert_eq!(e.tokens[0], (TokenKind::Keyword, 0, 3, 5), "stays put");
        assert_eq!(
            e.tokens[1],
            (TokenKind::Function, 2, 5, 6),
            "moves to the new row and back to column zero"
        );
        assert_eq!(
            e.tokens[2],
            (TokenKind::String, 2, 4, 5),
            "truncated at the break rather than split in two"
        );
    }

    #[test]
    fn a_join_is_the_inverse_of_a_split() {
        let original = vec![
            (TokenKind::Keyword, 0, 3, 5),
            (TokenKind::Function, 6, 9, 5),
            (TokenKind::Comment, 0, 2, 8),
        ];
        let mut e = engine_with(original.clone());
        e.nudge_for_split(5, 4);
        e.nudge_for_join(5, 4);
        assert_eq!(
            e.tokens, original,
            "split then join at the same point returns every token"
        );
    }
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
            src.insert(
                src.len().saturating_sub(2),
                char::from(b'a' + (i % 26) as u8),
            );
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
        assert!(
            eng.tokens
                .iter()
                .any(|t| t.0 == TokenKind::Keyword || t.0 == TokenKind::Function)
        );
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
