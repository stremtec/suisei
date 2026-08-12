//! Panes — splits, focus, and the terminals that live in panes (A3-4
//! extraction).
//!
//! The split tree itself is [`crate::split::SplitState`]; this file is
//! everything `App` does with it: opening/closing splits, moving focus
//! (park-then-load, the single moment the two copies of a pane's viewport
//! have to agree), and the pane-terminal lifecycle — spawn, title, close
//! confirmation, full-screen toggle.

use crate::app::{App, BufferId, EMPTY_TEXT_HASH, Mode};
use crate::buffer::Buffer;
use crate::tabs::BufferTab;
use crate::term::Terminal;
use crate::undo::UndoStack;

impl App {
    pub fn toggle_terminal_side(&mut self) {
        if self.terminal.open {
            self.terminal.open = false;
            self.terminal.shutdown();
            self.mode = Mode::Editor;
        } else {
            self.terminal.close_confirm = false;
            self.terminal.open = true;
            self.terminal.start(self.filename.as_ref());
            self.mode = Mode::Terminal;
        }
    }
    /// The shell running in the pane at `idx`, if that pane shows a terminal tab.
    pub fn pane_terminal(&self, idx: usize) -> Option<&Terminal> {
        let buf_id = self.split.panes.get(idx)?.buffer;
        let tab = self.tabs.buffers.iter().find(|t| t.id == buf_id)?;
        let tid = tab.terminal?;
        self.pane_terminals.get(&tid)
    }
    pub fn pane_terminal_mut(&mut self, idx: usize) -> Option<&mut Terminal> {
        let buf_id = self.split.panes.get(idx)?.buffer;
        let tab = self.tabs.buffers.iter().find(|t| t.id == buf_id)?;
        let tid = tab.terminal?;
        self.pane_terminals.get_mut(&tid)
    }
    /// The shell the keyboard is pointed at — the focused pane's, if it shows
    /// a terminal tab.
    pub fn focused_pane_terminal_mut(&mut self) -> Option<&mut Terminal> {
        let idx = self.split.focus_index();
        self.pane_terminal_mut(idx)
    }
    /// Whether the focused pane currently shows a terminal tab.
    pub fn terminal_window_focused(&self) -> bool {
        let buf_id = match self.split.panes.get(self.split.focus_index()) {
            Some(p) => p.buffer,
            None => return false,
        };
        self.is_terminal_tab(buf_id)
    }
    /// Whether the tab identified by `id` is a terminal tab.
    pub fn is_terminal_tab(&self, id: BufferId) -> bool {
        self.tabs
            .buffers
            .iter()
            .find(|t| t.id == id)
            .is_some_and(|t| t.terminal.is_some())
    }
    /// The title the shell reported (OSC 0/2) for a terminal tab, if any —
    /// the tab strip shows it in place of the generic "Terminal".
    pub fn terminal_title(&self, tid: crate::split::TerminalId) -> Option<&str> {
        self.pane_terminals
            .get(&tid)
            .and_then(|t| t.title.as_deref())
    }
    /// Ctrl+Shift+T — open a terminal as a **tab** in the focused pane.
    ///
    /// A terminal is just a [`BufferTab`] whose `terminal` field is set; the
    /// pane shows it like any other document. Pressing ⌃⇧T again while a
    /// terminal tab is focused closes it (the tab closes, the shell ends).
    pub fn toggle_terminal_full(&mut self) {
        // Second press on a terminal tab closes it.
        if self.terminal_window_focused() {
            self.close_current_tab();
            return;
        }
        // The focused pane's document BEFORE this open — an active layout has
        // to swap it out for the terminal tab, exactly like `open_new_tab`.
        let replacing = self.current_buffer_id();
        self.park_focused_pane();
        // Park the displaced document into its tab slot too — the pane slot
        // holds no content, and without this the tab strip would restore a
        // stale copy once the terminal tab is switched away from.
        self.save_state_to_tab();

        // Spawn a shell of its own.
        let cols = if self.split.is_split() {
            (self.grid_cols() / 2).max(40)
        } else {
            self.grid_cols().max(40)
        };
        let rows = self.grid_rows().max(24);
        let anchor = self.terminal_working_directory().join(".suisei-terminal");
        let mut term = Terminal::new();
        term.open = true;
        term.close_confirm = false;
        term.resize(cols, rows);
        term.start(Some(&anchor));
        if !term.started {
            // No half-state: report the failure and leave the pane as it was.
            self.message = "Terminal: failed to spawn shell (PTY)".into();
            return;
        }

        let tid = crate::split::TerminalId(self.next_terminal_id);
        self.next_terminal_id = self
            .next_terminal_id
            .checked_add(1)
            .expect("terminal id space exhausted");
        self.pane_terminals.insert(tid, term);

        // Create a tab for the terminal and switch to it.
        let tab_id = self.take_tab_id();
        self.tabs.buffers.push(BufferTab {
            id: tab_id,
            buffer: Buffer::new(),
            filename: None,
            scroll: 0,
            modified: false,
            saved_hash: EMPTY_TEXT_HASH,
            undo_stack: UndoStack::new(),
            file_mtime: None,
            terminal: Some(tid),
        });
        // Point the focused pane at the new terminal tab BEFORE the restore:
        // the active tab is derived from the pane, so this is what makes
        // the restore load the terminal tab.
        self.split.focused_pane_mut().buffer = tab_id;
        self.restore_state_from_tab();
        // The pane now shows the terminal tab, so the active layout already
        // contains it — membership is the panes. Only the strip order needs a
        // nudge to keep the group's run contiguous.
        self.regather_active_layout();
        // Remember what this shell displaced, so closing its tab restores that
        // document into the pane and keeps the split (see
        // `close_terminal_restoring_pane`). Only meaningful while split — a
        // single view has nothing to keep — but recording it always keeps the
        // close path simple.
        self.terminal_replaced.insert(tab_id, replacing);

        // Stay in Editor mode — the terminal is a tab, not a mode.
        if matches!(self.mode, Mode::Terminal | Mode::Editor) {
            self.mode = Mode::Editor;
        }
        self.message = "Terminal tab · keys → shell · ⌃⇧T close · ^W w other pane".into();
    }
    /// The focused pane's shell id, when it shows a terminal tab.
    fn focused_pane_terminal_id(&self) -> Option<crate::split::TerminalId> {
        let buf_id = self.split.panes.get(self.split.focus_index())?.buffer;
        self.tabs.buffers.iter().find(|t| t.id == buf_id)?.terminal
    }
    pub fn request_close_pane_terminal(&mut self) {
        let Some(tid) = self.focused_pane_terminal_id() else {
            return;
        };
        if self.pane_close_confirm == Some(tid) {
            self.pane_close_confirm = None;
            self.message = "Close cancelled".into();
            return;
        }
        self.pane_close_confirm = Some(tid);
        self.message = "Close terminal?  [y]es  /  [n]o  ·  Ctrl+Shift+W cancel".into();
    }
    pub fn confirm_close_pane_terminal(&mut self, yes: bool) {
        let Some(tid) = self.pane_close_confirm.take() else {
            return;
        };
        if !yes {
            self.message = "Close cancelled".into();
            return;
        }
        // Close the shell the dialog was raised for — focus may have moved
        // since, and `y` must not kill whichever terminal happens to sit
        // under the caret now.
        if let Some(idx) = self
            .tabs
            .buffers
            .iter()
            .position(|t| t.terminal == Some(tid))
        {
            self.close_tab_at(idx);
        }
        if matches!(self.mode, Mode::Terminal) {
            self.mode = Mode::Editor;
        }
        self.message = "Terminal closed".into();
    }
    pub fn split_vertical(&mut self) {
        self.open_split_kind(crate::split::Axis::Col, "Vertical");
    }
    pub fn split_horizontal(&mut self) {
        self.open_split_kind(crate::split::Axis::Row, "Horizontal");
    }
    pub fn split_above(&mut self) {
        self.open_split_kind_before(crate::split::Axis::Row, "Split above");
    }
    pub fn split_left(&mut self) {
        self.open_split_kind_before(crate::split::Axis::Col, "Split left");
    }
    fn open_split_kind(&mut self, axis: crate::split::Axis, label: &str) {
        use crate::split::SplitAdd;
        self.save_state_to_tab();
        // Park first: `split_focused` copies the focused pane's slot into the
        // new pane, and the focused slot is stale until parked (see S2).
        self.park_focused_pane();
        let r = self.split.split_focused(axis);
        self.message = match r {
            SplitAdd::Opened => {
                format!("{label} split · Ctrl+W w cycle · Ctrl+W q close")
            }
            SplitAdd::Added => format!(
                "Pane added ({}) · Ctrl+W w cycle · Ctrl+W q close",
                self.split.pane_count()
            ),
            SplitAdd::Full => format!("Max {} panes", crate::split::MAX_PANES),
        };
    }
    fn open_split_kind_before(&mut self, axis: crate::split::Axis, label: &str) {
        use crate::split::SplitAdd;
        self.save_state_to_tab();
        self.park_focused_pane();
        let r = self.split.split_focused_before(axis);
        self.message = match r {
            SplitAdd::Opened => {
                format!("{label} · Ctrl+W w cycle · Ctrl+W q close")
            }
            SplitAdd::Added => format!(
                "{label} ({}) · Ctrl+W w cycle · Ctrl+W q close",
                self.split.pane_count()
            ),
            SplitAdd::Full => format!("Max {} panes", crate::split::MAX_PANES),
        };
    }
    /// Vim `C-w q`: close the *focused* pane; neighbors survive (the split
    /// collapses once one pane remains).
    ///
    /// The closed pane's document stays open as a tab (header × is not a tab
    /// close), but it **leaves any layout group** it was a member of — a
    /// group is the on-screen arrangement, and a document that no pane of
    /// that arrangement shows is not in the group any more.
    pub fn close_split(&mut self) {
        if !self.split.is_split() {
            return;
        }
        // If the focused pane shows a terminal tab, end the shell.
        let buf_id = self.split.focused_pane().buffer;
        if let Some(tab) = self.tabs.buffers.iter().find(|t| t.id == buf_id) {
            if let Some(tid) = tab.terminal {
                if let Some(mut t) = self.pane_terminals.remove(&tid) {
                    t.shutdown();
                }
                if self.pane_close_confirm == Some(tid) {
                    self.pane_close_confirm = None;
                }
            }
        }
        // Closing the pane itself (not the tab) discards the shell — there is
        // no pane left to restore the displaced document into, so drop the
        // remembered mapping rather than leak it.
        self.terminal_replaced.remove(&buf_id);
        // Park the closing pane's document into its tab slot while the live
        // copy still names it — the tab stays open when its pane closes, and
        // after `remove_focused` the active tab derives from the survivor.
        self.save_state_to_tab();
        let survivor = self.split.remove_focused();
        // Eject from layout membership when no remaining pane shows this doc.
        // The tab itself stays; only the group bond breaks.
        if !self.split.panes.iter().any(|p| p.buffer == buf_id) {
            self.remove_doc_from_layouts(buf_id);
        }
        // Keep the active layout's parked tree honest while it still exists.
        if let Some(id) = self.active_layout {
            if self.split.is_split() {
                self.park_layout(id);
            }
        }
        // Two panes left out of three is still an arrangement; one distinct
        // document across them is not. Asked here, of the panes that remain.
        self.dissolve_degenerate_layouts();
        if self.split.is_split() {
            self.load_focused_pane();
            self.message = format!("Pane closed · {} left", self.split.pane_count());
            return;
        }
        // Collapsed to a single view: adopt the survivor snapshot.
        if let Some(p) = survivor {
            if self.buffer_index(p.buffer).is_some() && p.buffer != self.live_doc {
                // The survivor shows a document `App` is not holding. The one
                // pane IS the survivor now, so the derived active tab already
                // names its document — just load (the save happened before
                // `remove_focused`).
                self.restore_state_from_tab();
                self.lsp_restart_for_current();
                self.refresh_git();
            }
            let max_row = self.buffer.line_count().saturating_sub(1);
            self.buffer.cursor.row = p.cursor.0.min(max_row);
            self.buffer.cursor.col = p.cursor.1;
            self.buffer.clamp_col();
            self.scroll = p.scroll.min(max_row);
            // No `update_scroll()` — same reason as `load_focused_pane`: it
            // re-derives scroll from the caret and would throw away the
            // survivor's viewport.
        }
        self.message = String::from("Pane closed");
    }
    /// Vim `C-w h/j/k/l`: directional focus along the split axis (steps one
    /// pane per press; works for any pane count).
    pub fn focus_dir(&mut self, dir: char) {
        if !self.split.is_split() {
            return;
        }
        // Genuinely directional now. This used to step +1/-1 along the single
        // split axis and refuse the other two keys outright, because there was
        // only ever one axis. With a tree the layout is two-dimensional, so
        // pick the nearest pane that actually lies that way and overlaps on
        // the perpendicular axis.
        let rects = self.split.rects();
        let here = self.split.focus_index();
        let Some(from) = rects.get(here).copied() else {
            return;
        };
        let (cx, cy) = (from.x + from.w / 2.0, from.y + from.h / 2.0);
        let mut best: Option<(f32, usize)> = None;
        for (i, r) in rects.iter().enumerate() {
            if i == here {
                continue;
            }
            let (rx, ry) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let (ok, dist) = match dir {
                'h' => (rx < cx, cx - rx),
                'l' => (rx > cx, rx - cx),
                'k' => (ry < cy, cy - ry),
                'j' => (ry > cy, ry - cy),
                _ => return,
            };
            // Must share some extent perpendicular to the move, or "left" can
            // land on a pane that is really diagonally opposite.
            let overlaps = match dir {
                'h' | 'l' => r.y < from.y + from.h && from.y < r.y + r.h,
                _ => r.x < from.x + from.w && from.x < r.x + r.w,
            };
            if ok && overlaps && best.map_or(true, |(d, _)| dist < d) {
                best = Some((dist, i));
            }
        }
        let Some((_, next)) = best else { return };
        self.focus_pane_to(next);
        self.message = format!("Pane {}", next + 1);
    }
    pub fn focus_other_pane(&mut self) {
        if !self.split.is_split() {
            return;
        }
        let n = self.split.panes.len();
        let cur = self.split.focus_index();
        self.focus_pane_to((cur + 1) % n);
        self.message = format!("Pane {}", self.split.focus_index() + 1);
    }
    pub fn focus_pane(&mut self, idx: usize) {
        if !self.split.is_split() {
            return;
        }
        self.focus_pane_to(idx);
    }
    /// Park the focused pane's viewport into its slot.
    ///
    /// **The focused pane's slot is stale by design.** `App` holds the live
    /// document, scroll, hscroll and cursor for whichever pane has focus; the
    /// slots hold the *other* panes. Nothing needs syncing while the user
    /// works, because there is only ever one authority at a time.
    ///
    /// This is not how it used to be. Both copies were meant to be live at
    /// once, kept in step by four functions —`sync_split_from_active`,
    /// `sync_focused_pane_viewport`, `sync_focused_pane_tab` and the write
    /// buried in `update_scroll` — called from twenty-odd sites, each of which
    /// had to remember. Every forgotten call left the two disagreeing, and the
    /// symptom surfaced later and somewhere else: a pane that scrolled back to
    /// a stale position, a cursor that jumped on focus change.
    ///
    /// The compositor already worked this way for the *document* — it reads
    /// `current_buffer` for the focused pane and the slot only for the others
    /// (`scene.rs`). This extends that rule to the rest of the pane state.
    pub(crate) fn park_focused_pane(&mut self) {
        // Unconditional, even with a single pane. There is always exactly one
        // focused pane now, and the first split copies the focused pane's slot
        // into the new one — so a slot that was never parked would hand the
        // fresh pane an empty document and a cursor at the origin.
        let cur = (self.buffer.cursor.row, self.buffer.cursor.col);
        let id = self.current_buffer_id();
        let p = self.split.focused_pane_mut();
        p.buffer = id;
        p.scroll = self.scroll;
        p.hscroll = self.hscroll;
        p.cursor = cur;
    }
    /// Load the focused pane's slot into `App`, the inverse of
    /// [`App::park_focused_pane`].
    pub(crate) fn load_focused_pane(&mut self) {
        if !self.split.is_split() {
            return;
        }
        let pane = self.split.focused_pane().clone();
        if self.buffer_index(pane.buffer).is_some() && pane.buffer != self.live_doc {
            // `App` still holds the previously focused document; the caller
            // parked it into its tab slot before moving focus, so all that is
            // left is to load the focused pane's document (the derived active
            // tab already names it).
            self.restore_state_from_tab();
            self.lsp_restart_for_current();
            self.refresh_git();
        }
        // Clamped — the buffer may have changed underneath this pane.
        let max_row = self.buffer.line_count().saturating_sub(1);
        self.buffer.cursor.row = pane.cursor.0.min(max_row);
        self.buffer.cursor.col = pane.cursor.1;
        self.buffer.clamp_col();
        self.scroll = pane.scroll.min(max_row);
        self.hscroll = if self.wrap_lines { 0 } else { pane.hscroll };
        // NO `update_scroll()`. It re-derives the scroll from the CARET, so a
        // pane scrolled away from its cursor — scrolled with the wheel, which
        // does not move the caret — snapped straight back to the top the
        // moment focus returned to it. That *was* the "pane scrolls back to a
        // stale position" report. The pane's parked scroll is authoritative;
        // the caret being off-view is exactly the state the user left.
        //
        // `Engine::goto_tab` learned the same lesson for tabs already.
    }
    /// Move focus to `idx`. **The only way focus changes.**
    ///
    /// Park then load, in that order, in one place. The whole point of S2 is
    /// that this is the single moment where the two copies of a pane's
    /// viewport have to agree, so it is the only code that has to be right.
    pub fn focus_pane_to(&mut self, idx: usize) {
        if !self.split.is_split() {
            return;
        }
        let idx = idx.min(self.split.panes.len() - 1);
        if idx == self.split.focus_index() {
            return;
        }
        self.park_focused_pane();
        // Save BEFORE focus moves: once it has, the active tab is derived
        // from the new pane and the save would land on the wrong document.
        self.save_state_to_tab();
        self.split.set_focus(idx);
        self.load_focused_pane();
    }
}
