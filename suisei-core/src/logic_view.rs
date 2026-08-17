//! What a Logic View pane is looking at.
//!
//! [`crate::logic`] is the extractor — a tree in, rows out, no state. This is
//! the session behind one pane: which file, the rows as far as they have been
//! opened, what is selected, and the parse the rows were read from.
//!
//! ## Why it holds its own parse
//!
//! The editor's live tree belongs to the buffer that is open in a pane, and a
//! Logic View pane is a pane — so the file it draws is very often NOT the one
//! the keyboard is in. Reading `live_tree()` would mean the view showing
//! whatever was last typed in, which is the opposite of "code here, logic
//! there, side by side".
//!
//! So a session parses the text it was given and keeps that tree. It is
//! rebuilt when the text changes, and never otherwise.
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
use crate::logic::{self, LogicRow, LogicTree};
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
    pub fn open(path: &Path, text: &str) -> LogicSession {
        let mut s = LogicSession {
            path: path.to_path_buf(),
            lang: Lang::Rust,
            tree: LogicTree::default(),
            selected: 0,
            note: None,
            src: String::new(),
            ts: None,
        };
        s.rebuild(text);
        s
    }

    /// Whether this session is still about `text`.
    pub fn is_current(&self, text: &str) -> bool {
        self.src == text
    }

    /// Re-read the file, keeping open what was open.
    pub fn refresh(&mut self, text: &str) {
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
        self.rebuild(text);
        self.reopen(&open);
        if let Some(label) = selected {
            if let Some(i) = self.tree.rows.iter().position(|r| r.label == label) {
                self.selected = i;
            }
        }
    }

    fn rebuild(&mut self, text: &str) {
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
        let Some(mut bundle) = LangBundle::build(lang) else {
            self.note = Some("Grammar unavailable".into());
            return;
        };
        let Some(ts) = bundle.parser.parse(text, None) else {
            self.note = Some("This file did not parse".into());
            return;
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
    pub fn get(&mut self, path: &Path, text: &str) -> &mut LogicSession {
        if let Some(i) = self.sessions.iter().position(|s| s.path == path) {
            self.sessions[i].refresh(text);
            return &mut self.sessions[i];
        }
        if self.sessions.len() >= MAX_SESSIONS {
            self.sessions.remove(0);
        }
        self.sessions.push(LogicSession::open(path, text));
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
