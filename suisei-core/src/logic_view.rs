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
    pub fn toggle(&mut self, index: usize) -> bool {
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
        if let Some(i) = logic::row_at(&self.tree, line) {
            let row = &self.tree.rows[i];
            if row.kind == LogicKind::Entry && row.expandable && !row.expanded {
                moved = self.expand(i);
            }
        }
        self.follow_source(line) || moved
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
            });
        }
        if !runtime_allowed {
            return out;
        }
        for &i in &rt.enclosing {
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
