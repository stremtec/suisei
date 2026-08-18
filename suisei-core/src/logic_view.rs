//! What a Logic View pane is looking at.
//!
//! [`crate::logic`] is the extractor — a tree in, rows out, no state. This is
//! the session behind one pane: which file, the rows as far as they have been
//! opened, what is selected, and the parse the rows were read from.
//!
//! ## Whose parse it uses
//!
//! Two callers, two answers, and the difference is which file is on screen.
//!
//! The right rail draws the FOCUSED document, and the syntax engine already
//! holds a live tree for exactly that, reparsed incrementally on every
//! keystroke. So the rail hands it over and this adopts it: the rail is always
//! visible, and building a parser and reparsing on every file switch — to
//! reach a tree that already exists — is the one cost that would make an
//! always-visible view not worth having.
//!
//! A Logic PANE draws whatever file it was opened on, which is usually not the
//! one the keyboard is in — that is the point of it. There is no live tree for
//! that file, so the session parses the text it was given and keeps it.
//!
//! Either way the tree and the text it was parsed from travel together: a tree
//! indexed against stale text names the wrong ranges.
//!
//! ## The expansion survives the rebuild
//!
//! An edit invalidates the tree, and rebuilding from scratch would close every
//! function the reader had opened — an edit is exactly the moment they are
//! reading. So a rebuild remembers what was open by NAME and opens it again.
//!
//! By name rather than by index: a line inserted above moves every row, and
//! index 7 after an edit is not the row that was open before it.

use crate::lang::{Lang, LangBundle};
use crate::logic::{self, LogicKind, LogicRow, LogicTree};
use std::path::{Path, PathBuf};

/// One pane's view of one file.
pub struct LogicSession {
    pub path: PathBuf,
    pub lang: Lang,
    /// The rows, as deep as they have been opened.
    pub tree: LogicTree,
    /// The row the reader is on. Also what "reveal in the editor" reveals.
    pub selected: usize,
    /// Why there is nothing to show, when there is nothing to show.
    ///
    /// A language with no table, a file that will not parse, a file with no
    /// functions in it: three different facts, and "empty" is the wrong answer
    /// to all three. Same rule as the model viewer's — "nothing here" and "I
    /// could not read this" need different reactions.
    pub note: Option<String>,
    /// The row the pointer is over, when the reader is asking about a branch.
    ///
    /// Separate from `selected` because it is a QUESTION, not a place: it
    /// lasts as long as the pointer does, and moving the pointer away must
    /// leave the reader exactly where they were.
    pub peek: Option<usize>,
    /// The function the CARET opened, by the row it starts on.
    ///
    /// Following the caret opens a function, and nothing ever closed one
    /// again — so reading through a file left every function it passed through
    /// open, and the rail ended up as the whole file flattened.
    auto_open: Option<usize>,
    /// Functions the READER has opened or closed by hand, by name.
    ///
    /// One rule, both directions: **once the reader touches a function, the
    /// caret stops managing it.** Opening one and having it shut behind you,
    /// or closing one and having it spring back because the caret is still
    /// inside, are the same fault — the view arguing with the person using it.
    ///
    /// By name so it survives an edit, which is when the tree is rebuilt.
    hand: Vec<String>,
    src: String,
    ts: Option<tree_sitter::Tree>,
}

impl LogicSession {
    /// Read `text` as `path`'s logic. Cheap: the outline is functions only.
    ///
    /// `ready` is a parse of that exact text when the caller already has one —
    /// the editor's live tree for the focused document. Without it this builds
    /// a parser and parses, which is the expensive path and the one the always
    /// visible right rail must not take on every file switch.
    pub fn open(path: &Path, text: &str, ready: Option<tree_sitter::Tree>) -> LogicSession {
        let mut s = LogicSession {
            path: path.to_path_buf(),
            lang: Lang::Rust,
            tree: LogicTree::default(),
            selected: 0,
            note: None,
            peek: None,
            auto_open: None,
            hand: Vec::new(),
            src: String::new(),
            ts: None,
        };
        s.rebuild(text, ready);
        s
    }

    /// Whether this session is still about `text`.
    pub fn is_current(&self, text: &str) -> bool {
        self.src == text
    }

    /// Re-read the file, keeping open what was open.
    pub fn refresh(&mut self, text: &str, ready: Option<tree_sitter::Tree>) {
        if self.is_current(text) {
            return;
        }
        let open: Vec<(String, usize)> = self
            .tree
            .rows
            .iter()
            .filter(|r| r.expanded)
            .map(|r| (r.label.clone(), r.depth))
            .collect();
        let selected = self.tree.rows.get(self.selected).map(|r| r.label.clone());
        self.rebuild(text, ready);
        self.reopen(&open);
        if let Some(label) = selected {
            if let Some(i) = self.tree.rows.iter().position(|r| r.label == label) {
                self.selected = i;
            }
        }
    }

    fn rebuild(&mut self, text: &str, ready: Option<tree_sitter::Tree>) {
        self.src = text.to_string();
        // `hand` is deliberately NOT cleared: it is by name, and an edit does
        // not change the reader's mind about which functions they are reading.
        self.auto_open = None;
        self.tree = LogicTree::default();
        self.ts = None;
        self.note = None;

        let ext = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        let Some(lang) = Lang::from_ext(ext) else {
            self.note = Some(format!("No logic table for .{ext}"));
            return;
        };
        self.lang = lang;
        if logic::grammar_for(lang).is_none() {
            self.note = Some(format!("{} has no control flow to read", lang.name()));
            return;
        }
        let ts = match ready {
            // Already parsed, by the editor, from this exact text. Adopting it
            // costs a refcount; the alternative is building a parser and
            // walking the file again to reach the same tree.
            Some(t) => t,
            None => {
                let Some(mut bundle) = LangBundle::build(lang) else {
                    self.note = Some("Grammar unavailable".into());
                    return;
                };
                let Some(t) = bundle.parser.parse(text, None) else {
                    self.note = Some("This file did not parse".into());
                    return;
                };
                t
            }
        };
        match logic::outline(&ts, text, lang) {
            Some(t) if !t.rows.is_empty() => self.tree = t,
            _ => self.note = Some("No functions in this file".into()),
        }
        self.ts = Some(ts);
        self.selected = self.selected.min(self.tree.rows.len().saturating_sub(1));
    }

    /// Open again what a rebuild closed, by name.
    ///
    /// Walks forward rather than iterating a snapshot of the rows: opening one
    /// inserts the rows under it, and those may be things that were open too.
    fn reopen(&mut self, open: &[(String, usize)]) {
        let mut i = 0;
        while i < self.tree.rows.len() {
            let row = &self.tree.rows[i];
            let key = (row.label.clone(), row.depth);
            if open.contains(&key) {
                self.expand(i);
            }
            i += 1;
        }
    }

    /// Open or close the row at `index`. Returns whether anything moved.
    ///
    /// A row the reader touches becomes THEIRS: the caret stops closing it,
    /// either way round. Opening it means they want it; closing it means they
    /// do not, and re-opening it behind them because the caret is still inside
    /// would be the view arguing.
    pub fn toggle(&mut self, index: usize) -> bool {
        if let Some(row) = self.tree.rows.get(index) {
            if row.kind == LogicKind::Entry && !self.hand.contains(&row.label) {
                self.hand.push(row.label.clone());
            }
            if Some(row.start_row) == self.auto_open {
                self.auto_open = None;
            }
        }
        match self.tree.rows.get(index) {
            Some(r) if r.expanded => logic::collapse(&mut self.tree, index),
            Some(_) => self.expand(index),
            None => false,
        }
    }

    fn expand(&mut self, index: usize) -> bool {
        let Some(ts) = self.ts.as_ref() else {
            return false;
        };
        logic::expand(&mut self.tree, index, ts, &self.src, self.lang)
    }

    pub fn rows(&self) -> &[LogicRow] {
        &self.tree.rows
    }

    /// The source line the selected row starts on.
    pub fn selected_line(&self) -> Option<usize> {
        self.tree.rows.get(self.selected).map(|r| r.start_row)
    }

    /// Where to draw the guide for the row at `index`, as a visual column.
    ///
    /// The node's OWN indentation, not its body's. A block's last line is its
    /// closing brace, which sits at the node's indent and not the body's, so
    /// "the minimum indent inside" is the head's anyway — and measuring the
    /// head is one line instead of all of them, and cannot be thrown by a
    /// continuation line that happens to be outdented.
    ///
    /// `None` for a node that occupies one line: there is nothing to run down.
    pub fn guide_col(&self, index: usize, tab_width: usize) -> Option<usize> {
        let row = self.tree.rows.get(index)?;
        if row.end_row <= row.start_row {
            return None;
        }
        let line = self.src.split('\n').nth(row.start_row)?;
        let mut col = 0usize;
        for c in line.chars() {
            match c {
                ' ' => col += 1,
                '\t' => col += tab_width - (col % tab_width.max(1)),
                _ => break,
            }
        }
        Some(col)
    }

    /// Point the view at the row a source line belongs to, opening nothing.
    ///
    /// The other direction of the same containment test the runtime overlay
    /// uses: source and logic are two views of one range.
    pub fn follow_source(&mut self, line: usize) -> bool {
        match logic::row_at(&self.tree, line) {
            Some(i) if i != self.selected => {
                self.selected = i;
                true
            }
            _ => false,
        }
    }

    /// Follow the caret, opening the function it is in.
    ///
    /// The rail is beside the editor and the caret is always somewhere, so
    /// "the function you are reading" is the one thing the view can know
    /// without being asked. It opens THAT function and nothing else — walking
    /// into every call from here would build the whole file, which is the one
    /// thing the collapse exists to prevent.
    pub fn follow_caret(&mut self, line: usize) -> bool {
        let mut moved = false;
        let here = self.function_at(line);
        if here != self.auto_open {
            // Close the one the caret opened before this. Only that one: a
            // function the reader opened is not the caret's to close.
            if let Some(prev) = self.auto_open.take() {
                if let Some(j) = self.function_row(prev) {
                    if self.tree.rows[j].expanded {
                        moved |= logic::collapse(&mut self.tree, j);
                    }
                }
            }
            // Found again after the collapse: closing a function above this
            // one moves every index below it.
            if let Some(start) = here {
                if let Some(i) = self.function_row(start) {
                    let theirs = self.hand.contains(&self.tree.rows[i].label);
                    if !theirs && !self.tree.rows[i].expanded && self.expand(i) {
                        self.auto_open = Some(start);
                        moved = true;
                    }
                }
            }
        }
        self.follow_source(line) || moved
    }

    /// The row a function starts on, as an index. Identity is the START ROW
    /// rather than the index: indices move when anything above them opens.
    fn function_row(&self, start_row: usize) -> Option<usize> {
        self.tree
            .rows
            .iter()
            .position(|r| r.kind == LogicKind::Entry && r.start_row == start_row)
    }

    /// Which function holds `line`, by the row it starts on.
    fn function_at(&self, line: usize) -> Option<usize> {
        self.tree
            .rows
            .iter()
            .filter(|r| r.kind == LogicKind::Entry && line >= r.start_row && line <= r.end_row)
            .map(|r| r.start_row)
            .max()
    }
}

/// A run of source rows for the EDITOR to mark.
///
/// Runs, not rows. A guide down a block is one object, and the face clips it
/// to whatever band it is painting — handing over one entry per visible row
/// would rebuild that object inside a row loop, which is how the value
/// bracket came out in pieces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicMark {
    pub start_row: usize,
    pub end_row: usize,
    /// Visual column for the guide: the node's own indentation.
    pub col: usize,
    /// The reader is pointing at this.
    pub selected: bool,
    /// The program is stopped inside this.
    pub runtime: bool,
    /// One arm of a branch the pointer is over: `Some(true)` when the branch
    /// held, `Some(false)` when it did not.
    pub arm: Option<bool>,
}

impl LogicSession {
    /// What the editor should draw: the selection, then the branches and loops
    /// the program is inside, outermost first.
    ///
    /// `runtime_allowed` is the debugger's panel. Closing it has to take every
    /// runtime mark with it — the stop band and the value bracket already
    /// learned that, and a guide left behind would be the same bug in another
    /// colour.
    pub fn marks(
        &self,
        rt: &crate::logic::LogicRuntime,
        runtime_allowed: bool,
        tab_width: usize,
    ) -> Vec<LogicMark> {
        let mut out = Vec::new();
        if let Some(row) = self.tree.rows.get(self.selected) {
            out.push(LogicMark {
                start_row: row.start_row,
                end_row: row.end_row,
                col: self.guide_col(self.selected, tab_width).unwrap_or(0),
                selected: true,
                runtime: false,
                arm: None,
            });
        }
        // The panel gate is the RUNTIME marks' gate and nothing else's. It was
        // an early return, which took the branch peek with it — a question
        // about a branch has nothing to do with whether a debugger is open.
        for &i in rt.enclosing.iter().filter(|_| runtime_allowed) {
            let Some(row) = self.tree.rows.get(i) else { continue };
            // Not the function itself. "You are inside `process`" is what the
            // whole view already says, and a guide down a whole function is a
            // line beside every line of it.
            if row.kind == LogicKind::Entry {
                continue;
            }
            out.push(LogicMark {
                start_row: row.start_row,
                end_row: row.end_row,
                col: self.guide_col(i, tab_width).unwrap_or(0),
                selected: false,
                runtime: true,
                arm: None,
            });
        }
        out.extend(self.arms());
        out
    }

    /// Both arms of the branch the pointer is over.
    ///
    /// A branch is the one thing in a file you cannot read by looking at one
    /// line, and this is the question "what are the two ways out of here"
    /// answered on the code itself. Transient, because it is a question.
    fn arms(&self) -> Vec<LogicMark> {
        let Some(at) = self.peek else { return Vec::new() };
        let Some(head) = self.tree.rows.get(at) else { return Vec::new() };
        if !matches!(head.kind, LogicKind::Decision) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut arm: Option<bool> = None;
        for row in self.tree.rows.iter().skip(at + 1) {
            // Out of the branch entirely: what follows it is not an arm of it.
            if row.depth <= head.depth {
                break;
            }
            // The label the graph put on the edge that reaches this row is
            // what says which arm it is — and a row deeper than an arm's own
            // level inherits the arm it is inside.
            match row.edge {
                crate::logic::EdgeLabel::Yes => arm = Some(true),
                crate::logic::EdgeLabel::No => arm = Some(false),
                _ => {}
            }
            let Some(which) = arm else { continue };
            out.push(LogicMark {
                start_row: row.start_row,
                end_row: row.end_row,
                col: 0,
                selected: false,
                runtime: false,
                arm: Some(which),
            });
        }
        out
    }
}

/// The sessions the open Logic View panes are using.
///
/// A handful, keyed by path, because switching between two Logic tabs and
/// losing every open function on the way is not a thing to ship. Oldest out
/// first past the cap — a reader is not working in nine files at once, and a
/// dropped session costs one reparse.
#[derive(Default)]
pub struct LogicViews {
    sessions: Vec<LogicSession>,
}

const MAX_SESSIONS: usize = 8;

impl LogicViews {
    /// The session for `path`, built or refreshed against `text`.
    ///
    /// `ready` is a parse of that exact text when the caller has one — see
    /// [`LogicSession::open`].
    pub fn get(
        &mut self,
        path: &Path,
        text: &str,
        ready: Option<tree_sitter::Tree>,
    ) -> &mut LogicSession {
        if let Some(i) = self.sessions.iter().position(|s| s.path == path) {
            self.sessions[i].refresh(text, ready);
            return &mut self.sessions[i];
        }
        if self.sessions.len() >= MAX_SESSIONS {
            self.sessions.remove(0);
        }
        self.sessions.push(LogicSession::open(path, text, ready));
        self.sessions.last_mut().expect("just pushed")
    }

    /// The session for `path` if there is one, without building anything.
    pub fn peek(&self, path: &Path) -> Option<&LogicSession> {
        self.sessions.iter().find(|s| s.path == path)
    }

    pub fn forget(&mut self, path: &Path) {
        self.sessions.retain(|s| s.path != path);
    }
}
