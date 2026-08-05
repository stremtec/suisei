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
    /// This tab is a terminal pane. The shell lives in `App::pane_terminals`
    /// keyed by this id. `None` for ordinary document tabs.
    pub terminal: Option<crate::split::TerminalId>,
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
        });
        self.split.focused_pane_mut().buffer = tab_id;
        self.restore_state_from_tab();
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
            let in_layout = self
                .layouts
                .iter()
                .any(|l| l.id == lid && l.holds(target));
            if in_layout {
                // Same arrangement, different member.
                if let Some(pidx) = self.split.panes.iter().position(|p| p.buffer == target)
                {
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
                .find(|l| l.holds(t.id))
                .map(|l| l.id)
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
            self.modified = tab.modified;
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
        }
    }
    pub fn open_new_tab(&mut self, path: &str) {
        // The focused pane's document BEFORE this open. Opening replaces what
        // the focused pane shows (S2: `App` IS the focused pane), so an active
        // layout has to swap this document out of its membership for the one
        // being opened — else the new file lands as a loose chip outside the
        // group while the displaced one lingers inside it, shown by no pane.
        let replacing = self.current_buffer_id();
        self.save_state_to_tab();

        let pathbuf = PathBuf::from(path);
        let abs_path = if pathbuf.is_absolute() {
            pathbuf
        } else {
            env::current_dir().unwrap_or_default().join(&pathbuf)
        };

        let existing = self
            .tabs
            .buffers
            .iter()
            .find(|t| t.filename.as_ref() == Some(&abs_path))
            .map(|t| t.id);
        if let Some(id) = existing {
            self.split.focused_pane_mut().buffer = id;
            self.restore_state_from_tab();
            self.swap_focused_doc_in_active_layout(replacing, self.current_buffer_id());
            self.lsp_restart_for_current();
            self.refresh_git();
            self.message = format!("Switched to: {}", abs_path.display());
            return;
        }

        let content = fs::read_to_string(&abs_path).unwrap_or_default();
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
        });
        self.split.focused_pane_mut().buffer = tab_id;
        self.restore_state_from_tab();
        self.swap_focused_doc_in_active_layout(replacing, self.current_buffer_id());
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
        // End the shell the tab hosted.
        if let Some(mut t) = self.pane_terminals.remove(&tid) {
            t.shutdown();
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
        self.swap_focused_doc_in_active_layout(closed, replaced);
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
        // If the closing tab is a terminal, end its shell.
        if let Some(tab) = self.tabs.buffers.get(idx) {
            if let Some(tid) = tab.terminal {
                if let Some(mut t) = self.pane_terminals.remove(&tid) {
                    t.shutdown();
                }
                if self.pane_close_confirm == Some(tid) {
                    self.pane_close_confirm = None;
                }
            }
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
        // If the closing tab is a terminal, end its shell.
        if let Some(tab) = self.tabs.buffers.get(self.current_buffer()) {
            if let Some(tid) = tab.terminal {
                if let Some(mut t) = self.pane_terminals.remove(&tid) {
                    t.shutdown();
                }
                if self.pane_close_confirm == Some(tid) {
                    self.pane_close_confirm = None;
                }
            }
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
    }
}
