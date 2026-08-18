//! Tab management — the strip's state and everything that opens, closes,
//! moves or switches a tab (A3-2 extraction).
//!
//! [`TabStrip`] owns the documents in strip order and the id source; the
//! `impl App` block below is the orchestration — save/restore of the live
//! document, layout membership, LSP repoint — same domain, one file. The
//! rest of `App` speaks to tabs only through this module's API.

use crate::app::{App, BufferId, EMPTY_TEXT_HASH, EditRun, Mode, ScrollIntent, text_hash};
use crate::buffer::Buffer;
use crate::undo::UndoStack;
use std::path::PathBuf;
use std::{env, fs};

/// The document a fresh `App` starts with. Deliberately not `BufferId(0)` —
/// that value is the never-issued one, and a default-constructed `Pane` holds
/// it, so putting a real document there would make every unset pane silently
/// resolve to the first tab.
pub const FIRST_TAB_ID: BufferId = BufferId(1);

#[derive(Clone)]
pub struct BufferTab {
    /// Stable for this tab's lifetime and never reused.
    ///
    /// The face needs a list identity that does not move: with an index as its
    /// identity, dragging a tab left the identity list unchanged and only the
    /// titles swapped in place, so there was nothing for it to animate. Split
    /// panes address their document by this too — see [`BufferId`].
    pub id: BufferId,
    pub buffer: Buffer,
    pub filename: Option<PathBuf>,
    pub scroll: usize,
    pub modified: bool,
    /// Per-tab twin of [`App::saved_hash`], so switching tabs carries each
    /// document's on-disk fingerprint with it.
    pub saved_hash: u64,
    pub undo_stack: UndoStack,
    pub file_mtime: Option<std::time::SystemTime>,
    /// This tab is a terminal pane, and this id names its shell.
    ///
    /// The shell itself is the face's — SwiftTerm runs it in the view that
    /// draws it. `None` for ordinary document tabs, which is what makes this
    /// the "is a shell" axis of [`App::tab_kind`].
    pub terminal: Option<crate::split::TerminalId>,
    /// What that shell called itself (OSC 0/2), so the chip can say `make` or
    /// `vim README.md` instead of "Terminal".
    ///
    /// Reported over the ABI by the face, which is the only side that reads
    /// the escapes. It lived on the emulator that parsed it until there was no
    /// emulator; the tab is the thing the title is about.
    pub terminal_title: Option<String>,
    /// What kind of file this tab holds — decided once, when the tab is built
    /// from a path, because [`crate::media::classify_path`] can reach the disk
    /// and the face asks this question on every frame.
    ///
    /// Never `Terminal`: that axis is `terminal` above, and a fact with two
    /// owners is a fact that will disagree with itself. `App::tab_kind`
    /// composes the two.
    pub kind: crate::media::FileKind,
    /// Where a terminal tab's shell was started.
    ///
    /// The one thing about a shell worth carrying across a restart. The
    /// process cannot come back — it died with the machine's process table —
    /// but the directory it was working in is the whole of what makes the
    /// restored tab useful rather than merely present.
    pub terminal_cwd: Option<PathBuf>,
}

/// The tab strip's own state — the documents in strip order and the source
/// of their ids. Position helpers live here; anything touching the focused
/// pane, layouts, the LSP or the live document stays on `App`.
#[derive(Clone)]
pub struct TabStrip {
    pub buffers: Vec<BufferTab>,
    next_tab_id: u64,
}

impl TabStrip {
    /// A strip holding only the empty first tab.
    pub fn new() -> Self {
        Self::with_first(BufferTab {
            id: FIRST_TAB_ID,
            buffer: Buffer::new(),
            filename: None,
            scroll: 0,
            modified: false,
            saved_hash: EMPTY_TEXT_HASH,
            undo_stack: UndoStack::new(),
            file_mtime: None,
            terminal: None,
            terminal_title: None,
            kind: crate::media::FileKind::Text,
            terminal_cwd: None,
        })
    }

    /// A strip whose first tab is already built (opening with a file).
    pub fn with_first(first: BufferTab) -> Self {
        Self {
            buffers: vec![first],
            // Starts PAST `FIRST_TAB_ID`: the tab a fresh strip opens with
            // is not handed out by `next_id`.
            next_tab_id: FIRST_TAB_ID.0 + 1,
        }
    }

    /// Source of `BufferTab::id`. Monotonic; ids are never reused.
    pub fn next_id(&mut self) -> BufferId {
        let id = self.next_tab_id;
        self.next_tab_id = self
            .next_tab_id
            .checked_add(1)
            .expect("tab id space exhausted");
        BufferId(id)
    }

    /// Position of `id` in the strip, or `None` if that document is closed.
    pub fn index(&self, id: BufferId) -> Option<usize> {
        self.buffers.iter().position(|t| t.id == id)
    }

    /// The active tab's position — where the focused pane's document sits.
    /// A document no pane shows (mid-switch, or closed out from under the
    /// pane) falls back to the first tab: a pane must render *something*.
    pub fn current(&self, focused_doc: BufferId) -> usize {
        self.index(focused_doc).unwrap_or(0)
    }
}

impl Default for TabStrip {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn open_blank_tab(&mut self) {
        // A new untitled tab LEAVES an active layout, exactly as switching to a
        // tab outside it does: park the arrangement, then clear the desk down
        // to the new document.
        //
        // It used to just point the focused pane at the new buffer, which made
        // the new document part of the arrangement — and a unified layout draws
        // one chip for the whole arrangement, so the tab the user had just
        // asked for had no chip at all. Pressing "+" appeared to do nothing,
        // and a member had quietly been displaced to make room for it.
        //
        // The arrangement is not lost: it is a tab, and clicking it brings the
        // whole desk back. That is what folding is for.
        let left_layout = self.active_layout.take().is_some_and(|id| {
            self.park_layout(id);
            true
        });
        self.save_state_to_tab();
        let buffer = Buffer::new();
        let mut undo = UndoStack::new();
        undo.push(buffer.snapshot());
        let tab_id = self.take_tab_id();
        self.tabs.buffers.push(crate::BufferTab {
            id: tab_id,
            buffer,
            filename: None,
            scroll: 0,
            modified: false,
            saved_hash: EMPTY_TEXT_HASH,
            undo_stack: undo,
            file_mtime: None,
            terminal: None,
            terminal_title: None,
            kind: crate::media::FileKind::Text,
            terminal_cwd: None,
        });
        self.split.focused_pane_mut().buffer = tab_id;
        self.restore_state_from_tab();
        // Clear the desk, the same "leave" a tab switch out of a layout does.
        // Only when one was parked above — a free split the user built is
        // theirs and a new tab must not collapse it.
        if left_layout {
            self.split.collapse_to(tab_id);
        }
        self.refresh_git();
        self.mode = Mode::Editor;
        self.message = "New tab · i insert · Ctrl+P files · :e <file>".into();
    }
    /// Switch to tab index if it exists (0-based).
    ///
    /// Layout-aware:
    /// * **Active layout + target is a member** → stay on the desk; focus the
    ///   pane that already shows that document (or load it into the focused
    ///   pane). Never park/collapse — collapsing here made ⌃⇥ / chip switches
    ///   between group members clear the split, after which a later free
    ///   split + tab change looked like "layout save" again when a member
    ///   click re-activated the parked tree.
    /// * **Active layout + target outside** → park, switch, collapse to one
    ///   document (the designed leave).
    /// * **No active layout** → only the focused pane's document changes; a
    ///   free multi-pane split is preserved.
    pub fn goto_tab(&mut self, idx: usize) {
        if idx >= self.tabs.buffers.len() {
            self.message = format!("No tab {}", idx + 1);
            return;
        }
        let target = self.tabs.buffers[idx].id;

        if let Some(lid) = self.active_layout {
            let in_layout = self.layout_holds(lid, target);
            if in_layout {
                // Same arrangement, different member.
                if let Some(pidx) = self.split.panes.iter().position(|p| p.buffer == target) {
                    if pidx != self.split.focus_index() {
                        self.park_focused_pane();
                        self.split.set_focus(pidx);
                        self.load_focused_pane();
                    }
                } else {
                    // Member not currently on a pane (was swapped out) — put it
                    // on the focused pane without leaving the layout.
                    self.park_focused_pane();
                    self.save_state_to_tab();
                    self.split.focused_pane_mut().buffer = target;
                    self.restore_state_from_tab();
                }
                self.message = format!("Tab {}", idx + 1);
                return;
            }
            // Outside the active layout — leave: park, then clear the desk.
            let leaving = self.active_layout.take();
            if let Some(id) = leaving {
                self.park_layout(id);
            }
            self.save_state_to_tab();
            self.split.focused_pane_mut().buffer = target;
            self.restore_state_from_tab();
            let doc = self.current_buffer_id();
            self.split.collapse_to(doc);
            self.message = format!("Tab {}", idx + 1);
            return;
        }

        // Free desk (no active layout): change only the focused pane.
        self.save_state_to_tab();
        self.split.focused_pane_mut().buffer = target;
        self.restore_state_from_tab();
        self.message = format!("Tab {}", idx + 1);
    }
    /// Activate the tab holding `id` (a chip click).
    pub fn goto_tab_id(&mut self, id: BufferId) {
        if let Some(idx) = self.buffer_index(id) {
            self.goto_tab(idx);
        }
    }
    /// Close the tab holding `id` (a chip's ✕ / Close Tab).
    pub fn close_tab_id(&mut self, id: BufferId) {
        if let Some(idx) = self.buffer_index(id) {
            self.close_tab_at(idx);
        }
    }
    /// Reorder: move the tab holding `from` onto the strip position of `to`.
    /// Buffer order and strip order agree on the relative order of visible
    /// documents, so moving between the two ids is the same move in both.
    pub fn move_tab_ids(&mut self, from: BufferId, to: BufferId) -> bool {
        let (Some(f), Some(t)) = (self.buffer_index(from), self.buffer_index(to)) else {
            return false;
        };
        self.move_tab(f, t)
    }
    pub(crate) fn take_tab_id(&mut self) -> BufferId {
        self.tabs.next_id()
    }
    /// Position of `id` in `buffers`, or `None` if that document is closed.
    pub fn buffer_index(&self, id: BufferId) -> Option<usize> {
        self.tabs.index(id)
    }
    /// The active tab's position in `buffers` — derived, never stored.
    ///
    /// The focused pane names the active document (S2: `App`'s live fields
    /// ARE that document), so the active tab is wherever the focused pane's
    /// document sits in the strip. Reorders and closes shift the vector and
    /// this follows by itself — the old stored index needed a `-= 1` after
    /// every remove and a remap after every move; none of that survives.
    pub fn current_buffer(&self) -> usize {
        self.tabs.current(self.split.focused_pane().buffer)
    }
    /// Stable handle for the active document.
    pub fn current_buffer_id(&self) -> BufferId {
        self.tabs
            .buffers
            .get(self.current_buffer())
            .map(|t| t.id)
            .unwrap_or_default()
    }
    /// Which tab a pane is showing, falling back to the active one when its
    /// document has been closed. Panes must always render *something*, so the
    /// paint path wants this; anything asking "is this pane's document X?"
    /// wants [`App::buffer_index`] and its honest `None`.
    pub fn pane_tab(&self, pane: &crate::split::Pane) -> usize {
        self.buffer_index(pane.buffer)
            .unwrap_or(self.current_buffer())
    }
    /// Move the tab at `from` to sit at `to`, as a tab-bar drag does.
    ///
    /// Panes need no repair here: they hold a [`BufferId`], and reordering the
    /// vector does not change any document's identity. The active tab is
    /// derived from the focused pane's id, so it follows the move by itself.
    ///
    /// Returns false when the move is a no-op or out of range, or when it
    /// would break a folded layout's group — dragging an outside tab into a
    /// group's run (or a member out of it) is refused, so the grouped strip
    /// shape always wraps its members alone.
    pub fn move_tab(&mut self, from: usize, to: usize) -> bool {
        let n = self.tabs.buffers.len();
        if from >= n || to >= n || from == to {
            return false;
        }
        if !self.move_keeps_groups_contiguous(from, to) {
            return false;
        }
        // The active tab's state lives in `App` until it is parked, so park it
        // before the vector moves underneath it.
        self.save_state_to_tab();
        let tab = self.tabs.buffers.remove(from);
        self.tabs.buffers.insert(to, tab);

        // Reload through the derived position: the slot moved, and this
        // re-reads it through the focused pane's document at its new index.
        self.restore_state_from_tab();
        true
    }
    /// Would moving the tab at `from` to `to` leave every folded layout's
    /// members as one contiguous run in the strip?
    ///
    /// A grouped layout paints one rounded container from its first member's
    /// left edge to its last member's right edge; a non-member sitting inside
    /// that span is swallowed visually. So a reorder is only legal when it
    /// keeps each group's members adjacent — which also refuses dragging an
    /// outside tab into a group and dragging a member out of one.
    fn move_keeps_groups_contiguous(&self, from: usize, to: usize) -> bool {
        if self.layouts.is_empty() {
            return true;
        }
        let group_of = |t: &BufferTab| -> u64 {
            self.layouts
                .iter()
                .map(|l| l.id)
                .find(|id| self.layout_holds(*id, t.id))
                .unwrap_or(0)
        };
        let mut order: Vec<u64> = self.tabs.buffers.iter().map(group_of).collect();
        let g = order.remove(from);
        order.insert(to, g);
        // A group is contiguous iff, once seen, it never reappears after a
        // different group has intervened.
        let mut seen = std::collections::HashSet::new();
        let mut prev: u64 = 0;
        for &grp in &order {
            if grp != 0 && grp != prev && seen.contains(&grp) {
                return false;
            }
            if grp != 0 {
                seen.insert(grp);
            }
            prev = grp;
        }
        true
    }
    pub fn save_state_to_tab(&mut self) {
        // Viewer tabs use the same buffer-shaped slot as text documents, but
        // that empty buffer is only compatibility state. A historical or
        // stray edit flag must never survive tab parking as if an MP3/PDF/
        // image had editable bytes waiting to be saved.
        if self.live_tab_kind().is_viewer() {
            self.modified = false;
        }
        if let Some(idx) = self.buffer_index(self.live_doc) {
            let tab = &mut self.tabs.buffers[idx];
            tab.buffer = self.buffer.clone();
            tab.filename = self.filename.clone();
            tab.scroll = self.scroll;
            tab.modified = self.modified;
            tab.saved_hash = self.saved_hash;
            tab.undo_stack = self.undo_stack.clone();
            tab.file_mtime = self.file_mtime;
        }
    }
    pub fn restore_state_from_tab(&mut self) {
        self.scroll_intent = ScrollIntent::Restore;
        if let Some(tab) = self.tabs.buffers.get(self.current_buffer()).cloned() {
            self.live_doc = tab.id;
            self.buffer = tab.buffer;
            self.filename = tab.filename;
            self.scroll = tab.scroll;
            self.modified = if tab.kind.is_viewer() {
                false
            } else {
                tab.modified
            };
            self.saved_hash = tab.saved_hash;
            self.content_width = 0; // different document, different extent
            self.undo_stack = tab.undo_stack;
            self.file_mtime = tab.file_mtime;
            // Deletion is a property of the restored document, not global UI
            // state. Re-derive it on every tab switch so a vanished inactive
            // file cannot borrow the previous tab's flag (or lose its own).
            self.file_deleted = self.file_mtime.is_some()
                && self
                    .filename
                    .as_ref()
                    .is_some_and(|path| std::fs::metadata(path).is_err());
            // GUI selection is ephemeral across tabs (interim): collapse to a
            // caret at the restored cursor. The cursor itself rides in the
            // buffer clone, so it is already correct.
            self.sel = crate::selection::SelectionSet::single(crate::selection::Selection::caret(
                self.buffer.cursor(),
            ));
            self.edit_run = EditRun::None;
            self.git_follow_live_document();
        }
    }

    /// Point the git gutter at whatever document is now live.
    ///
    /// The gutter describes exactly one file, and until now nothing enforced
    /// that. Fifteen paths restore a tab; seven called `refresh_git` after and
    /// eight did not — including `goto_tab`, which is what a tab-chip click
    /// runs. So editing A and then clicking B left A's hunks in `self.git`,
    /// and the gutter drew them against whatever rows B happened to have:
    /// changes appearing on a file that has none.
    ///
    /// Clearing first matters as much as refreshing. `refresh_git` is
    /// asynchronous — it hands the diff to a thread — so a refresh alone would
    /// still show the old file's bars for every frame until the result lands.
    ///
    /// Cheap when nothing moved: a switch back to the same document, or a pane
    /// focus change that lands on the same file, compares two strings and
    /// returns.
    fn git_follow_live_document(&mut self) {
        let now = self
            .filename
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if self.git.path == now {
            return;
        }
        self.git.clear();
        self.blame.clear();
        self.refresh_git();
    }
    /// Open the Logic View of the focused document, as a tab of its own.
    ///
    /// A tab rather than a panel: it lands in a pane, so splitting puts the
    /// code and its logic side by side, and every piece of tab machinery —
    /// close, reorder, focus, the tab strip — already works on it. That is the
    /// same reason a terminal is a tab, and this borrows the shape whole.
    ///
    /// Opening it twice focuses the one that is open. The view is OF a file,
    /// so two of them would be two copies of one fact.
    pub fn open_logic_view(&mut self) {
        let Some(path) = self.filename.clone() else {
            self.message = "Logic View needs a saved file".into();
            return;
        };
        if crate::logic::grammar_for_path(&path).is_none() {
            self.message = format!(
                "No logic to read in {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            return;
        }
        self.save_state_to_tab();
        let existing = self
            .tabs
            .buffers
            .iter()
            .find(|t| t.kind == crate::media::FileKind::Logic && t.filename.as_ref() == Some(&path))
            .map(|t| t.id);
        if let Some(id) = existing {
            self.split.focused_pane_mut().buffer = id;
            self.restore_state_from_tab();
            return;
        }
        let tab_id = self.take_tab_id();
        self.tabs.buffers.push(BufferTab {
            id: tab_id,
            // Empty on purpose, like every other viewer: there is no text to
            // show, the view draws from the path, and an empty buffer is the
            // one thing that cannot be edited into a corrupted file.
            buffer: Buffer::new(),
            filename: Some(path.clone()),
            scroll: 0,
            modified: false,
            saved_hash: EMPTY_TEXT_HASH,
            undo_stack: UndoStack::new(),
            file_mtime: None,
            terminal: None,
            terminal_title: None,
            kind: crate::media::FileKind::Logic,
            terminal_cwd: None,
        });
        self.split.focused_pane_mut().buffer = tab_id;
        self.restore_state_from_tab();
        self.message = format!("Logic View: {}", path.display());
    }

    /// The session for `path`, built or refreshed against the live text.
    ///
    /// Hands over the editor's own tree when this is the focused document and
    /// that tree was parsed from exactly this text. The right rail is always
    /// visible, so without this every file switch pays a second full parse —
    /// and a second parser build — on the main thread, for a tree the syntax
    /// engine has already produced and keeps incrementally up to date.
    pub fn logic_session(&mut self, path: &std::path::Path) -> &mut crate::logic_view::LogicSession {
        let text = self.logic_source(path);
        let ready = if self.filename.as_deref() == Some(path)
            && !self.live_tab_kind().is_viewer()
        {
            self.syntax
                .live_tree()
                .filter(|(_, parsed)| *parsed == text)
                .map(|(tree, _)| tree.clone())
        } else {
            None
        };
        self.logic_views.get(path, &text, ready)
    }

    /// A cheap stand-in for "has this file changed", for the pane's poll.
    ///
    /// The Logic pane asks whether anything moved on every tick, and the
    /// honest answer — reparse and compare — is a full copy of the buffer per
    /// frame per pane. A version counter and a length say the same thing for
    /// this purpose: they move when the text moves.
    pub fn logic_source_stamp(&self, path: &std::path::Path) -> u64 {
        if self.filename.as_deref() == Some(path)
            && self.live_tab_kind() != crate::media::FileKind::Logic
        {
            return self.buffer.version() ^ (self.buffer.line_count() as u64) << 40;
        }
        for tab in &self.tabs.buffers {
            if tab.kind != crate::media::FileKind::Logic && tab.filename.as_deref() == Some(path) {
                return tab.buffer.version() ^ (tab.buffer.line_count() as u64) << 40;
            }
        }
        // Not open: the file on disk, by size and when it was written.
        let Ok(meta) = std::fs::metadata(path) else {
            return 0;
        };
        let secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        meta.len() ^ secs << 24
    }

    /// Take the reader to the source a row came from.
    ///
    /// The pairing this view exists for: logic on one side, code on the other.
    /// So the code goes where the code already is — the pane that is showing
    /// this file if there is one, another pane if there is not, and only this
    /// pane when there is no other. Opening the source over the view the user
    /// clicked in would be answering "show me this" by hiding the question.
    pub fn reveal_logic_row(&mut self, path: &std::path::Path, line: usize) {
        let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let showing = |app: &Self, buffer: crate::tabs::BufferId| {
            app.tabs
                .buffers
                .iter()
                .find(|t| t.id == buffer)
                .is_some_and(|t| {
                    t.kind != crate::media::FileKind::Logic
                        && t.filename.as_ref().is_some_and(|f| {
                            std::fs::canonicalize(f).unwrap_or_else(|_| f.clone()) == target
                        })
                })
        };
        let here = self.split.focus_index();
        let already = (0..self.split.panes.len())
            .find(|&i| showing(self, self.split.panes[i].buffer));
        match already {
            Some(i) => self.focus_pane_to(i),
            None => {
                if let Some(other) = (0..self.split.panes.len()).find(|&i| i != here) {
                    self.focus_pane_to(other);
                }
                self.open_new_tab(&path.to_string_lossy());
            }
        }
        if self.buffer.line_count() == 0 {
            return;
        }
        self.buffer.cursor.row = line.min(self.buffer.line_count().saturating_sub(1));
        self.buffer.move_to_line_start();
        self.update_scroll();
    }

    /// The text a Logic View of `path` should read.
    ///
    /// The open buffer when there is one, including edits that have not been
    /// saved — reading the disk beside an editor showing something else is the
    /// one thing a side-by-side view must not do. The disk otherwise, because
    /// the file being viewed need not be open at all.
    pub fn logic_source(&self, path: &std::path::Path) -> String {
        if self.filename.as_deref() == Some(path) && self.live_tab_kind() != crate::media::FileKind::Logic {
            return self.buffer.text();
        }
        for tab in &self.tabs.buffers {
            if tab.kind != crate::media::FileKind::Logic && tab.filename.as_deref() == Some(path) {
                return tab.buffer.text();
            }
        }
        std::fs::read_to_string(path).unwrap_or_default()
    }

    pub fn open_new_tab(&mut self, path: &str) {
        // No "displaced document" to record. Opening replaces what the focused
        // pane shows (S2: `App` IS the focused pane), and an active layout's
        // membership IS its panes — so the opened file joins it and the
        // displaced one leaves, both by having happened. This used to hand a
        // before/after pair to `swap_focused_doc_in_active_layout`, which
        // looked the old one up in a stored member list and did nothing at all
        // when the focused pane had been split off after the fold.
        self.save_state_to_tab();

        let pathbuf = PathBuf::from(path);
        let abs_path = if pathbuf.is_absolute() {
            pathbuf
        } else {
            env::current_dir().unwrap_or_default().join(&pathbuf)
        };

        // A toolbar reopen can hand this function a standardised absolute URL
        // while the original explorer open retained a lexical `..`, `.`, or a
        // symlink spelling. PathBuf equality treats those as different files
        // and used to create a second tab for the track already playing.
        // Identity is the filesystem's canonical path when available; retain
        // `abs_path` itself for display and for files that do not exist yet.
        let requested_identity = fs::canonicalize(&abs_path).unwrap_or_else(|_| abs_path.clone());
        let existing = self
            .tabs
            .buffers
            .iter()
            .find(|tab| {
                // A Logic View tab carries the SOURCE file's path — it is a
                // view of that file, not that file. Without this, opening
                // `foo.rs` while its logic is open focused the logic pane and
                // the text never appeared.
                tab.kind != crate::media::FileKind::Logic
                    && tab.filename.as_ref().is_some_and(|open_path| {
                        fs::canonicalize(open_path).unwrap_or_else(|_| open_path.clone())
                            == requested_identity
                    })
            })
            .map(|t| t.id);
        if let Some(id) = existing {
            self.split.focused_pane_mut().buffer = id;
            self.restore_state_from_tab();
            self.regather_active_layout();
            self.lsp_restart_for_current();
            self.refresh_git();
            self.message = format!("Switched to: {}", abs_path.display());
            return;
        }

        // The same `read_to_string().unwrap_or_default()` that `App::open_file`
        // was fixed for — and this is the path the explorer actually uses, so
        // it is the one that mattered. A PNG failed UTF-8, became an empty
        // document, and ⌘S wrote the emptiness back over the file.
        let raw = fs::read(&abs_path).ok();
        let kind = crate::media::classify_bytes(&abs_path, raw.as_deref());
        let content = match raw {
            Some(b) if !kind.is_viewer() => String::from_utf8_lossy(&b).into_owned(),
            // A viewer's document stays empty on purpose: there is no text to
            // show, the viewer draws from the path, and an empty buffer is the
            // one thing that cannot be edited into a corrupted file.
            _ => String::new(),
        };
        let buffer = Buffer::from_string(&content);
        let mtime = std::fs::metadata(&abs_path)
            .ok()
            .and_then(|m| m.modified().ok());
        let mut undo = UndoStack::new();
        undo.push(buffer.snapshot());
        undo.attach_file(&abs_path, self.undo_caching, &content);

        let tab_id = self.take_tab_id();
        self.tabs.buffers.push(BufferTab {
            id: tab_id,
            buffer,
            filename: Some(abs_path.clone()),
            scroll: 0,
            modified: false,
            saved_hash: text_hash(&content),
            undo_stack: undo,
            file_mtime: mtime,
            terminal: None,
            terminal_title: None,
            kind,
            terminal_cwd: None,
        });
        self.split.focused_pane_mut().buffer = tab_id;
        self.restore_state_from_tab();
        self.regather_active_layout();
        let text = self.buffer.text();
        self.lsp
            .auto_start_with_text(&abs_path.display().to_string(), Some(&text));
        self.lsp_synced_path = Some(abs_path.clone());
        self.lsp_synced_hash = text_hash(&text);
        self.refresh_git();
        self.message = format!("Opened: {}", abs_path.display());
        self.fire_hook(crate::hooks::HookEvent::Open);
    }
    pub fn next_tab(&mut self) {
        if self.tabs.buffers.len() < 2 {
            return;
        }
        self.save_state_to_tab();
        let next = (self.current_buffer() + 1) % self.tabs.buffers.len();
        self.split.focused_pane_mut().buffer = self.tabs.buffers[next].id;
        self.restore_state_from_tab();
        self.lsp_restart_for_current();
        self.refresh_git();
    }
    pub fn prev_tab(&mut self) {
        if self.tabs.buffers.len() < 2 {
            return;
        }
        self.save_state_to_tab();
        let cur = self.current_buffer();
        let prev = if cur == 0 {
            self.tabs.buffers.len() - 1
        } else {
            cur - 1
        };
        self.split.focused_pane_mut().buffer = self.tabs.buffers[prev].id;
        self.restore_state_from_tab();
        self.lsp_restart_for_current();
        self.refresh_git();
    }
    /// Close the tab at `idx`, leaving the editor showing whatever it was
    /// showing — unless that is the document being closed.
    ///
    /// The tab strip's close button used to route through
    /// `goto_tab(idx); close_current_tab()`, which had to make the doomed tab
    /// active first and then put the editor back afterwards. Every attempt at
    /// "putting it back" was a guess, because by then the information had been
    /// overwritten. Not moving in the first place is the fix.
    /// Closing a terminal tab that took over a split pane (⌃⇧T): restore the
    /// document the shell displaced back into that pane and keep the split,
    /// instead of retiring the pane. Returns true when it handled the close.
    ///
    /// This is the counterpart to the swap `toggle_terminal_full` does on open.
    /// Without it, closing the shell fell to the generic tab close, which drops
    /// the pane showing the closed document — so a two-pane split collapsed to
    /// one, losing the arrangement the user was working in.
    fn close_terminal_restoring_pane(&mut self, closed: BufferId) -> bool {
        if !self.split.is_split() {
            return false;
        }
        let Some(&replaced) = self.terminal_replaced.get(&closed) else {
            return false;
        };
        // `closed` must really be a terminal tab and the remembered document
        // must still be open; otherwise fall back to the generic close.
        let tid = self
            .tabs
            .buffers
            .iter()
            .find(|t| t.id == closed)
            .and_then(|t| t.terminal);
        let Some(tid) = tid else {
            self.terminal_replaced.remove(&closed);
            return false;
        };
        if replaced == closed || self.buffer_index(replaced).is_none() {
            self.terminal_replaced.remove(&closed);
            return false;
        }
        if self.pane_close_confirm == Some(tid) {
            self.pane_close_confirm = None;
        }
        self.terminal_replaced.remove(&closed);
        // Point every pane showing the terminal back at the displaced document,
        // so the split survives: the generic retire only removes panes that
        // still name the closed id, and now none do.
        for p in self.split.panes.iter_mut() {
            if p.buffer == closed {
                p.buffer = replaced;
            }
        }
        // Restore layout membership (terminal → displaced document), drop the
        // terminal tab from the strip, then load the restored document into the
        // focused pane and re-point language/git services at it.
        self.regather_active_layout();
        if let Some(at) = self.buffer_index(closed) {
            self.tabs.buffers.remove(at);
        }
        self.restore_state_from_tab();
        self.lsp_restart_for_current();
        self.refresh_git();
        if let Some(id) = self.active_layout {
            self.park_layout(id);
        }
        self.message = "Terminal closed · previous tab restored".into();
        true
    }

    pub fn close_tab_at(&mut self, idx: usize) {
        let n = self.tabs.buffers.len();
        if idx >= n {
            return;
        }
        let closed = self.tabs.buffers[idx].id;
        if self.close_terminal_restoring_pane(closed) {
            return;
        }
        if idx == self.current_buffer() || n <= 1 {
            self.close_current_tab();
            return;
        }
        // Same bookkeeping `close_current_tab` does for the tab it drops.
        if let Some(tab) = self.tabs.buffers.get_mut(idx) {
            if tab.filename.is_some() {
                let text = tab.buffer.text();
                tab.undo_stack.finish(self.undo_caching, &text);
            }
        }
        // Clear a close-confirm dialog that was about this shell. Ending the
        // process itself is the face's — it reaps by asking which tabs remain.
        if let Some(tab) = self.tabs.buffers.get(idx)
            && tab.terminal.is_some()
            && self.pane_close_confirm == tab.terminal
        {
            self.pane_close_confirm = None;
        }
        let closed = self.tabs.buffers[idx].id;
        // Drop the closed document from any layout's membership.
        self.remove_doc_from_layouts(closed);
        self.tabs.buffers.remove(idx);
        // The active tab is derived from the focused pane's document id —
        // untouched by this close when that pane was not the one showing
        // `closed`. Closing a tab must also remove the pane(s) that were
        // showing it: repointing left a ghost pane inside a layout group
        // that still claimed a document that no longer exists.
        self.retire_panes_of_closed_doc(closed);
        self.message = String::from("Buffer closed");
    }
    pub fn close_current_tab(&mut self) {
        // A terminal tab that took over a split pane restores its displaced
        // document into the pane and keeps the split (⌃⇧T close, or the tab ✕).
        let closed_id = self.current_buffer_id();
        if self.close_terminal_restoring_pane(closed_id) {
            return;
        }
        // Persist or discard the closing buffer's history (undo_caching).
        self.save_state_to_tab();
        let closing = self.current_buffer();
        if let Some(tab) = self.tabs.buffers.get_mut(closing) {
            if tab.filename.is_some() {
                let text = tab.buffer.text();
                tab.undo_stack.finish(self.undo_caching, &text);
            }
        }
        let closed = self.current_buffer_id();
        // Same as `close_tab_at`: only the dialog is ours to clear.
        if let Some(tab) = self.tabs.buffers.get(self.current_buffer())
            && tab.terminal.is_some()
            && self.pane_close_confirm == tab.terminal
        {
            self.pane_close_confirm = None;
        }
        // Drop the closed document from any layout's membership.
        self.remove_doc_from_layouts(closed);
        if self.tabs.buffers.len() <= 1 {
            self.lsp.shutdown();
            self.buffer = Buffer::new();
            self.filename = None;
            self.scroll = 0;
            self.modified = false;
            self.saved_hash = EMPTY_TEXT_HASH;
            self.undo_stack = UndoStack::new();
            self.undo_stack.push(self.buffer.snapshot());
            self.file_mtime = None;
            let fresh_id = self.take_tab_id();
            self.tabs.buffers[0] = BufferTab {
                id: fresh_id,
                buffer: self.buffer.clone(),
                filename: None,
                scroll: 0,
                modified: false,
                saved_hash: EMPTY_TEXT_HASH,
                undo_stack: self.undo_stack.clone(),
                file_mtime: None,
                terminal: None,
                terminal_title: None,
                kind: crate::media::FileKind::Text,
                terminal_cwd: None,
            };
            // The slot survives but the document in it does not, so the id
            // changed and any pane still naming the old one has to follow.
            self.split.repoint(closed, fresh_id);
            // `App` holds the fresh document now — say so, or the next save
            // would chase the closed id and land nowhere.
            self.live_doc = fresh_id;
            return;
        }

        let at = self.current_buffer();
        self.tabs.buffers.remove(at);
        // Prefer removing the pane(s) that showed this document over
        // repointing them at a neighbour. In a layout group the pane *is*
        // the member; leaving a repointed ghost kept the split shape while
        // the strip lost the chip. Only the last remaining pane is repointed
        // (a single view must show something).
        self.retire_panes_of_closed_doc(closed);
        // Focus may have moved to a survivor pane — load it.
        self.restore_state_from_tab();
        // Re-point the LSP at the newly current tab (same language → reuse the
        // running server; different → restart). The old unconditional shutdown
        // left the surviving tabs with no LSP at all.
        self.lsp_restart_for_current();
        self.refresh_git();
        self.message = String::from("Buffer closed");
    }

    /// After a document leaves `buffers`, drop every split pane that was
    /// showing it. If a single pane still names the closed id (the unsplit
    /// case, or the last pane of a multi-close), repoint it at a surviving
    /// tab. Syncs the active layout's parked tree when one is live.
    fn retire_panes_of_closed_doc(&mut self, closed: BufferId) {
        if self.split.is_split() {
            let _ = self.split.remove_panes_showing(closed);
        }
        // Last pane (or any leftover) still pointing at the closed doc.
        // `current_buffer_id` resolves through the focused pane, which may
        // still name `closed` — fall back to the first surviving tab.
        if self.split.panes.iter().any(|p| p.buffer == closed) {
            let adopt = self
                .tabs
                .buffers
                .iter()
                .map(|t| t.id)
                .find(|id| *id != closed)
                .unwrap_or(closed);
            if adopt != closed {
                self.split.repoint(closed, adopt);
            }
        }
        if let Some(id) = self.active_layout {
            if self.split.is_split() {
                self.park_layout(id);
            }
        }
        // The panes are final now, which is the only point at which "is this
        // still an arrangement" has an answer.
        self.dissolve_degenerate_layouts();
    }
}
