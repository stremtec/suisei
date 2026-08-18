//! First-class GUI editing commands (P0.2).
//!
//! These are the semantic commands the Swift face calls in place of the old
//! synthetic vim `i`/`Esc` dance. They drive [`crate::selection::SelectionSet`]
//! and keep `buffer.cursor` in step with the primary head, so a caret is an
//! empty selection and Shift+Arrow extends by moving only the head.
//!
//! Motions reuse the buffer's own grapheme-aware movement (temporarily seating
//! the cursor at a given head, moving, reading the result back) so there is a
//! single tested definition of "one grapheme left" etc. Vertical motion is the
//! exception: it is `goal_x`-aware here, because `Buffer::move_up/down` clamp
//! the column and would forget the desired column after a short line.

use crate::app::App;
use crate::buffer::Position;
use crate::selection::Selection;

/// A cursor motion, independent of whether it moves or extends the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

impl Motion {
    /// Horizontal/word/line/doc motions define a fresh goal column; only
    /// vertical motion consults and preserves one.
    fn is_vertical(self) -> bool {
        matches!(
            self,
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown
        )
    }
}

impl App {
    /// Compute where `motion` lands starting from `from`, without disturbing
    /// the live cursor. Reuses the buffer's grapheme-aware movers so there is
    /// one definition of each motion.
    fn motion_target(&mut self, from: Position, motion: Motion) -> Position {
        // Vertical is handled by the caller (goal_x aware); this covers the
        // rest by seating the buffer cursor and reusing its tested logic.
        let saved = self.buffer.cursor;
        self.buffer.cursor = from;
        match motion {
            Motion::Left => self.buffer.move_left(),
            Motion::Right => self.buffer.move_right(),
            Motion::WordLeft => self.buffer.move_word_back(),
            Motion::WordRight => self.buffer.move_word_forward(),
            Motion::LineStart => self.buffer.move_to_line_start(),
            Motion::LineEnd => self.buffer.move_to_line_end(),
            Motion::DocStart => {
                self.buffer.cursor = Position::zero();
            }
            Motion::DocEnd => {
                let last = self.buffer.line_count().saturating_sub(1);
                let col = self.buffer.line(last).chars().count();
                self.buffer.cursor = Position::new(last, col);
            }
            // Vertical never reaches here (see `apply_motion`).
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown => {}
        }
        let target = self.buffer.cursor;
        self.buffer.cursor = saved;
        target
    }

    /// Vertical motion that preserves the goal column across short lines.
    /// `rows` is signed: negative moves up. Returns the new head and the goal
    /// column to carry forward.
    fn vertical_target(
        &self,
        from: Position,
        goal_x: Option<usize>,
        rows: isize,
    ) -> (Position, usize) {
        let goal = goal_x.unwrap_or(from.col);
        let last = self.buffer.line_count().saturating_sub(1);
        let row = if rows < 0 {
            from.row.saturating_sub(rows.unsigned_abs())
        } else {
            from.row.saturating_add(rows as usize).min(last)
        };
        let len = self.buffer.line(row).chars().count();
        (Position::new(row, goal.min(len)), goal)
    }

    /// Rows a Page motion travels: one screenful less an overlap line, so the
    /// line you were reading stays on screen. Falls back to a sane page when
    /// the viewport has not been sized yet (headless tests).
    fn page_rows(&self) -> usize {
        let h = self.grid_rows() as usize;
        if h > 1 { h - 1 } else { 20 }
    }

    /// Resolve a motion against the primary head, returning the new head and
    /// the goal column to store on the selection.
    fn apply_motion(&mut self, motion: Motion) -> (Position, Option<usize>) {
        let head = self.sel.primary().head;
        if motion.is_vertical() {
            let rows = match motion {
                Motion::Up => -1,
                Motion::Down => 1,
                Motion::PageUp => -(self.page_rows() as isize),
                _ => self.page_rows() as isize,
            };
            let (pos, goal) = self.vertical_target(head, self.sel.primary().goal_x, rows);
            (pos, Some(goal))
        } else {
            (self.motion_target(head, motion), None)
        }
    }

    /// Move the caret (no Shift): collapse any selection and move the head.
    /// From a non-empty selection, a plain Left/Right lands on the near/far
    /// edge without moving further — the macOS text-field convention.
    pub fn caret_move(&mut self, motion: Motion) {
        self.edit_run = crate::app::EditRun::None;
        // Any real caret motion dismisses the keyword popup — otherwise it
        // trails the cursor until Esc. List navigation goes through
        // `completion_move` (Up/Down are intercepted before this runs), so this
        // never swallows the popup's own arrow keys.
        self.completions.deactivate();
        let sel = self.sel.primary();
        if !sel.is_empty() && matches!(motion, Motion::Left | Motion::Right) {
            let collapsed = match motion {
                Motion::Left => sel.collapsed_to_start(),
                Motion::Right => sel.collapsed_to_end(),
                _ => unreachable!(),
            };
            self.sel.set_primary(collapsed);
            self.buffer.cursor = collapsed.head;
            return;
        }
        let (head, goal) = self.apply_motion(motion);
        self.sel.set_primary(Selection {
            anchor: head,
            head,
            goal_x: goal,
        });
        self.buffer.cursor = head;
    }

    /// Extend the selection (Shift): keep the anchor, move only the head.
    /// If there is no selection yet, the current head becomes the anchor.
    pub fn caret_extend(&mut self, motion: Motion) {
        self.edit_run = crate::app::EditRun::None;
        // Extending a selection moves away from the completion prefix — drop it.
        self.completions.deactivate();
        let sel = self.sel.primary();
        let (head, goal) = self.apply_motion(motion);
        self.sel.set_primary(Selection {
            anchor: sel.anchor,
            head,
            goal_x: goal,
        });
        self.buffer.cursor = head;
    }

    /// Place a single caret at `pos` (a plain click), discarding other cursors.
    pub fn caret_place(&mut self, pos: Position) {
        self.edit_run = crate::app::EditRun::None;
        // A click moves the caret off the completion prefix; drop the popup
        // rather than let it follow the new caret position.
        self.completions.deactivate();
        let clamped = self.clamp_to_document(pos);
        self.sel = crate::selection::SelectionSet::single(Selection::caret(clamped));
        self.buffer.cursor = clamped;
    }

    /// Extend the primary selection's head to `pos` (a drag, or Shift+Click).
    pub fn caret_drag_to(&mut self, pos: Position) {
        self.edit_run = crate::app::EditRun::None;
        let clamped = self.clamp_to_document(pos);
        let anchor = self.sel.primary().anchor;
        self.sel.set_primary(Selection::new(anchor, clamped));
        self.buffer.cursor = clamped;
    }

    /// Select the RECTANGLE between two cells — ⌥-drag, and ⌃⇧↑/↓.
    ///
    /// A block selection is not a new kind of selection. It is one ordinary
    /// selection per row, all on the same two columns, which is exactly what
    /// `SelectionSet::spans` already holds and what every multi-cursor edit
    /// already knows how to apply. Nothing downstream learns a new shape.
    ///
    /// The columns are **visual**, and that is the whole of the difficulty. A
    /// rectangle is a rectangle on the SCREEN; a tab is one character and eight
    /// columns, a CJK glyph is one character and two. Taking the block from
    /// character columns would leave it ragged over exactly the lines a column
    /// edit is for — a table, an indented block. So the caller passes what the
    /// pointer touched, and each row converts that back to its own characters.
    ///
    /// A row too short to reach the block gets a **caret at its end** rather
    /// than being skipped. That is what makes typing on a ragged block append
    /// to the short lines instead of silently missing them.
    pub fn select_block(
        &mut self,
        anchor_row: usize,
        anchor_vcol: usize,
        head_row: usize,
        head_vcol: usize,
    ) {
        self.edit_run = crate::app::EditRun::None;
        self.completions.deactivate();

        let last = self.buffer.line_count().saturating_sub(1);
        let (r0, r1) = (anchor_row.min(head_row).min(last), anchor_row.max(head_row).min(last));
        let (v0, v1) = (anchor_vcol.min(head_vcol), anchor_vcol.max(head_vcol));
        let tab = self.tab_width;

        let mut spans = Vec::with_capacity(r1 - r0 + 1);
        // The primary is the row the pointer is ON, so the caret the rest of
        // the editor follows is where the user's hand is.
        let mut primary = 0usize;
        for (i, row) in (r0..=r1).enumerate() {
            let a = self.buffer.screen_col_to_buffer_col(row, v0, tab);
            let b = self.buffer.screen_col_to_buffer_col(row, v1, tab);
            let len = self.buffer.line(row).chars().count();
            let (a, b) = (a.min(len), b.min(len));
            // Anchor and head keep the drag's DIRECTION, so shrinking the block
            // back the way it came removes what it added.
            let (anchor_col, head_col) = if head_vcol >= anchor_vcol { (a, b) } else { (b, a) };
            spans.push(Selection::new(
                Position::new(row, anchor_col),
                Position::new(row, head_col),
            ));
            if row == head_row.min(last) {
                primary = i;
            }
        }
        if spans.is_empty() {
            return;
        }
        self.buffer.cursor = spans[primary].head;
        self.sel = crate::selection::SelectionSet::spans(spans, primary);
    }

    /// The block the current selection describes, in visual columns, if it is
    /// one — `(anchor_row, anchor_vcol, head_row, head_vcol)`.
    ///
    /// What makes ⌃⇧↓ able to grow a block: the selection set does not record
    /// that it came from a rectangle, so the columns are read back off it.
    pub fn block_extent(&self) -> Option<(usize, usize, usize, usize)> {
        let all = self.sel.all();
        if all.len() < 2 {
            return None;
        }
        let tab = self.tab_width;
        let vcols = |s: &Selection| {
            (
                self.buffer.buffer_col_to_screen_col(s.anchor.row, s.anchor.col, tab),
                self.buffer.buffer_col_to_screen_col(s.head.row, s.head.col, tab),
            )
        };
        let (a0, h0) = vcols(&all[0]);
        // One row each, consecutive, same two columns — anything else is a
        // multi-cursor set the user built by hand, and growing THAT as a
        // rectangle would throw their carets away.
        for (i, s) in all.iter().enumerate() {
            if s.anchor.row != s.head.row || s.anchor.row != all[0].anchor.row + i {
                return None;
            }
            if vcols(s) != (a0, h0) {
                return None;
            }
        }
        let first = all[0].anchor.row;
        let last = all[all.len() - 1].anchor.row;
        let primary_row = self.sel.primary().head.row;
        // Which end the head is at decides which way ⌃⇧↓ grows.
        if primary_row == last {
            Some((first, a0, last, h0))
        } else {
            Some((last, a0, first, h0))
        }
    }

    /// ⌃⇧↑ / ⌃⇧↓ — start a block at the caret, or grow the one that is there.
    pub fn block_extend_rows(&mut self, delta: isize) {
        let last = self.buffer.line_count().saturating_sub(1);
        let (a_row, a_vcol, h_row, h_vcol) = match self.block_extent() {
            Some(b) => b,
            None => {
                let c = self.sel.primary();
                let tab = self.tab_width;
                (
                    c.anchor.row,
                    self.buffer.buffer_col_to_screen_col(c.anchor.row, c.anchor.col, tab),
                    c.head.row,
                    self.buffer.buffer_col_to_screen_col(c.head.row, c.head.col, tab),
                )
            }
        };
        let next = h_row as isize + delta;
        if next < 0 || next as usize > last {
            return;
        }
        self.select_block(a_row, a_vcol, next as usize, h_vcol);
    }

    /// Add a caret at `pos` (⌘-click multi-cursor).
    pub fn caret_add(&mut self, pos: Position) {
        self.edit_run = crate::app::EditRun::None;
        let clamped = self.clamp_to_document(pos);
        self.sel.add(Selection::caret(clamped));
        self.buffer.cursor = self.sel.primary().head;
    }

    /// Collapse to a single caret at the primary head (Escape / plain click on
    /// an existing selection).
    pub fn caret_collapse(&mut self) {
        self.sel.collapse_to_primary_caret();
        self.buffer.cursor = self.sel.primary().head;
    }

    /// Select the word under `pos` (double-click) into the GUI model.
    pub fn select_word_gui(&mut self, pos: Position) {
        let clamped = self.clamp_to_document(pos);
        let saved = self.buffer.cursor;
        self.buffer.cursor = clamped;
        let range = word_range_at(&self.buffer, clamped);
        self.buffer.cursor = saved;
        if let Some((start, end)) = range {
            // `end` is exclusive — exactly the GUI head.
            // anchor = word start, head = one past the word.
            self.sel = crate::selection::SelectionSet::single(Selection::new(start, end));
            self.buffer.cursor = end;
        } else {
            self.caret_place(clamped);
        }
    }

    /// Select the whole document.
    pub fn select_all_gui(&mut self) {
        let last = self.buffer.line_count().saturating_sub(1);
        let end = Position::new(last, self.buffer.line(last).chars().count());
        self.sel = crate::selection::SelectionSet::single(Selection::new(Position::zero(), end));
        self.buffer.cursor = end;
    }

    /// Clamp a position to a valid document location (row in range, col within
    /// that row's grapheme count).
    fn clamp_to_document(&self, pos: Position) -> Position {
        let last = self.buffer.line_count().saturating_sub(1);
        let row = pos.row.min(last);
        let col = pos.col.min(self.buffer.line(row).chars().count());
        Position::new(row, col)
    }

    // ── Semantic edits (mode-independent) ──────────────────────────────────
    //
    // These replace the vim insert/delete path for the GUI. Each maps over
    // ALL selections (multi-cursor is inherent) processing them last-first so
    // an earlier edit never shifts a not-yet-processed span, and re-seats every
    // caret afterwards. A non-empty selection is replaced (type-over). There is
    // no mode gate: typing always types.

    /// Selection indices ordered latest-first by document position.
    fn selections_last_first(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.sel.len()).collect();
        order.sort_by(|&a, &b| self.sel.all()[b].start().cmp(&self.sel.all()[a].start()));
        order
    }

    /// Insert `text` at every caret, replacing any non-empty selection first.
    /// Bracket/quote auto-pairing at a single caret. Returns true when it took
    /// over the keystroke: an opener that inserted its matching closer (caret
    /// left between them), or a closer typed directly in front of its own match
    /// (the caret skips past it instead of doubling — the VS Code / CodeMirror
    /// convention). False falls through to a plain insert.
    fn try_auto_pair(&mut self, text: &str) -> bool {
        let mut it = text.chars();
        let (Some(ch), None) = (it.next(), it.next()) else {
            return false; // only single-scalar inserts pair
        };
        if self.sel.len() != 1 || !self.sel.primary().is_empty() {
            return false;
        }
        let caret = self.sel.primary().head;
        let line = self.buffer.line(caret.row);
        let next = line.chars().nth(caret.col);
        let prev = caret.col.checked_sub(1).and_then(|i| line.chars().nth(i));

        let closer_for = |c: char| match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            '\'' => Some('\''),
            '`' => Some('`'),
            _ => None,
        };
        let is_bracket_close = |c: char| matches!(c, ')' | ']' | '}');
        let is_quote = |c: char| matches!(c, '"' | '\'' | '`');

        // Type-over: a closer (or quote) typed right in front of the same glyph
        // steps past it rather than inserting a duplicate.
        if (is_bracket_close(ch) || is_quote(ch)) && next == Some(ch) {
            let to = Position::new(caret.row, caret.col + 1);
            self.sel = crate::selection::SelectionSet::single(Selection::caret(to));
            self.buffer.cursor = to;
            self.edit_run = crate::app::EditRun::None; // a move, not an edit
            return true;
        }

        // Auto-close an opener, but only where it reads as opening a new pair:
        // end of line, before whitespace, or before a closing bracket — never
        // mid-word. Quotes additionally never pair right after a word character
        // (English apostrophes: don't, it's).
        if let Some(close) = closer_for(ch) {
            let context_ok = match next {
                None => true,
                Some(n) => n.is_whitespace() || is_bracket_close(n) || is_quote(n),
            };
            let quote_ok = !is_quote(ch) || prev.map_or(true, |p| !p.is_alphanumeric());
            if context_ok && quote_ok {
                if self.edit_run != crate::app::EditRun::Insert {
                    self.push_undo();
                    self.edit_run = crate::app::EditRun::Insert;
                }
                self.buffer.cursor = caret;
                self.buffer.insert_char_pair(ch, close);
                let head = self.buffer.cursor;
                self.sel = crate::selection::SelectionSet::single(Selection::caret(head));
                return true;
            }
        }
        false
    }

    pub fn gui_insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.try_auto_pair(text) {
            return;
        }
        // Coalesce a typing run: snapshot once when the run starts, not per key.
        if self.edit_run != crate::app::EditRun::Insert {
            self.push_undo();
            self.edit_run = crate::app::EditRun::Insert;
        }
        let mut heads = vec![Position::zero(); self.sel.len()];
        for i in self.selections_last_first() {
            let s = self.sel.all()[i];
            let at = if s.is_empty() {
                s.head
            } else {
                self.buffer.delete_range(s.start(), s.end());
                s.start()
            };
            self.buffer.cursor = at;
            self.buffer.insert_str(text);
            heads[i] = self.buffer.cursor;
        }
        let primary = self.sel.primary_index();
        self.sel = crate::selection::SelectionSet::carets(&heads, primary);
        self.buffer.cursor = self.sel.primary().head;
    }

    /// Insert a smart-indented newline at every caret (Return).
    pub fn gui_insert_newline(&mut self, indent_unit: &str) {
        // Asked once, of the document — not per caret, and not of the buffer,
        // which has no idea what file it is.
        //
        // A file with no extension, or one no grammar claims, gets the indent:
        // an unknown file is more likely to be code than not, and the failure
        // is asymmetric — a missing indent in code is retyped every line, a
        // stray one in prose is deleted once.
        let auto_indent = self
            .file_extension()
            .and_then(|e| crate::lang::Lang::from_ext(&e))
            .map(|l| l.auto_indents())
            .unwrap_or(true);
        self.push_undo();
        self.edit_run = crate::app::EditRun::None;
        let mut heads = vec![Position::zero(); self.sel.len()];
        for i in self.selections_last_first() {
            let s = self.sel.all()[i];
            let at = if s.is_empty() {
                s.head
            } else {
                self.buffer.delete_range(s.start(), s.end());
                s.start()
            };
            self.buffer.cursor = at;
            self.buffer.insert_newline_smart(indent_unit, auto_indent);
            heads[i] = self.buffer.cursor;
        }
        let primary = self.sel.primary_index();
        self.sel = crate::selection::SelectionSet::carets(&heads, primary);
        self.buffer.cursor = self.sel.primary().head;
    }

    /// Backspace: delete the selection, or one grapheme before each caret.
    pub fn gui_delete_backward(&mut self) {
        // Auto-pair: backspacing between an empty pair — `(|)`, `"|"` — removes
        // BOTH the opener and its closer, the counterpart to auto-close.
        if self.sel.len() == 1 && self.sel.primary().is_empty() {
            let c = self.sel.primary().head;
            if c.col > 0 {
                let line = self.buffer.line(c.row);
                let prev = line.chars().nth(c.col - 1);
                let next = line.chars().nth(c.col);
                let empty_pair = matches!(
                    (prev, next),
                    (Some('('), Some(')'))
                        | (Some('['), Some(']'))
                        | (Some('{'), Some('}'))
                        | (Some('"'), Some('"'))
                        | (Some('\''), Some('\''))
                        | (Some('`'), Some('`'))
                );
                if empty_pair {
                    if self.edit_run != crate::app::EditRun::Delete {
                        self.push_undo();
                        self.edit_run = crate::app::EditRun::Delete;
                    }
                    let from = Position::new(c.row, c.col - 1);
                    self.buffer
                        .delete_range(from, Position::new(c.row, c.col + 1));
                    self.sel = crate::selection::SelectionSet::single(Selection::caret(from));
                    self.buffer.cursor = from;
                    return;
                }
            }
        }
        if self.edit_run != crate::app::EditRun::Delete {
            self.push_undo();
            self.edit_run = crate::app::EditRun::Delete;
        }
        let mut heads = vec![Position::zero(); self.sel.len()];
        for i in self.selections_last_first() {
            let s = self.sel.all()[i];
            let caret = if !s.is_empty() {
                self.buffer.delete_range(s.start(), s.end());
                s.start()
            } else {
                let h = s.head;
                if h.col > 0 {
                    let prev = crate::buffer::grapheme_prev_col(self.buffer.line(h.row), h.col);
                    self.buffer.delete_range(Position::new(h.row, prev), h);
                    Position::new(h.row, prev)
                } else if h.row > 0 {
                    let prev_len = self.buffer.line(h.row - 1).chars().count();
                    self.buffer
                        .delete_range(Position::new(h.row - 1, prev_len), h);
                    Position::new(h.row - 1, prev_len)
                } else {
                    h
                }
            };
            heads[i] = caret;
        }
        let primary = self.sel.primary_index();
        self.sel = crate::selection::SelectionSet::carets(&heads, primary);
        self.buffer.cursor = self.sel.primary().head;
    }

    /// Forward delete: delete the selection, or one grapheme after each caret.
    pub fn gui_delete_forward(&mut self) {
        if self.edit_run != crate::app::EditRun::Delete {
            self.push_undo();
            self.edit_run = crate::app::EditRun::Delete;
        }
        let mut heads = vec![Position::zero(); self.sel.len()];
        for i in self.selections_last_first() {
            let s = self.sel.all()[i];
            let caret = if !s.is_empty() {
                self.buffer.delete_range(s.start(), s.end());
                s.start()
            } else {
                let h = s.head;
                let len = self.buffer.line(h.row).chars().count();
                if h.col < len {
                    let next = crate::buffer::grapheme_next_col(self.buffer.line(h.row), h.col);
                    self.buffer.delete_range(h, Position::new(h.row, next));
                } else if h.row + 1 < self.buffer.line_count() {
                    // Join the next line up.
                    self.buffer.delete_range(h, Position::new(h.row + 1, 0));
                }
                h
            };
            heads[i] = caret;
        }
        let primary = self.sel.primary_index();
        self.sel = crate::selection::SelectionSet::carets(&heads, primary);
        self.buffer.cursor = self.sel.primary().head;
    }

    /// Collapse the GUI selection to a caret at the current buffer cursor.
    /// Used to keep `sel` coherent when a non-GUI path (the legacy dispatch,
    /// typing) moved `buffer.cursor` on its own.
    pub fn sync_sel_to_cursor(&mut self) {
        self.sel = crate::selection::SelectionSet::single(Selection::caret(self.buffer.cursor()));
    }

    /// The primary selection's normalised range, or `None` for a bare caret.
    /// Exclusive: `end` is the boundary past the last selected grapheme.
    pub fn gui_selection_range(&self) -> Option<(Position, Position)> {
        let s = self.sel.primary();
        if s.is_empty() { None } else { Some(s.range()) }
    }
}

/// The word under `pos`, as an exclusive `[start, end)` span — the one piece of
/// the old vim text-object machinery the GUI needs (double-click, ⌘D).
/// Word class follows the editor convention: alphanumeric+`_` is one word, a
/// run of punctuation is another, whitespace is never selected.
fn word_range_at(buf: &crate::buffer::Buffer, pos: Position) -> Option<(Position, Position)> {
    let chars: Vec<char> = buf.line(pos.row).chars().collect();
    if chars.is_empty() {
        return None;
    }
    let col = pos.col.min(chars.len().saturating_sub(1));
    if chars[col].is_whitespace() {
        return None;
    }
    let word_char = |c: char| c.is_alphanumeric() || c == '_';
    let same_class = |a: char, b: char| word_char(a) == word_char(b) && !b.is_whitespace();
    let here = chars[col];
    let mut start = col;
    while start > 0 && same_class(here, chars[start - 1]) {
        start -= 1;
    }
    let mut end = col + 1;
    while end < chars.len() && same_class(here, chars[end]) {
        end += 1;
    }
    Some((Position::new(pos.row, start), Position::new(pos.row, end)))
}

impl App {
    /// The identifier being typed at the primary caret, if any.
    fn prefix_at_caret(&self) -> String {
        let c = self.sel.primary().head;
        let chars: Vec<char> = self.buffer.line(c.row).chars().collect();
        let mut start = c.col.min(chars.len());
        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        chars[start..c.col.min(chars.len())].iter().collect()
    }

    /// Keep the completion popup in step with what was just typed.
    ///
    /// Autocomplete has never actually worked in the GUI: the only thing that
    /// opened the popup was a `Ctrl+A` chord, because the typing trigger lived
    /// in the vim insert handler — a code path the GUI never reached, since it
    /// never entered vim's Insert mode. Typing has to drive it.
    ///
    /// Only a real identifier prefix opens it; anything else closes it, so
    /// punctuation and whitespace dismiss rather than leave a stale list.
    /// Multi-caret is deliberately excluded: one popup cannot describe several
    /// different prefixes.
    pub fn completion_after_typing(&mut self) {
        // Zero FIRST. These are read once per keystroke over FFI, and a key
        // that returns early — multi-caret, a prefix under two characters —
        // used to leave the previous walk's number sitting in the field to be
        // reported again. The log showed four samples agreeing to three
        // decimals, which is not a measurement, it is one measurement echoing.
        self.completions.last_scope_us = 0;
        let t0 = std::time::Instant::now();
        self.completion_after_typing_inner();
        self.completions.last_total_us = t0.elapsed().as_micros() as u32;
    }

    fn completion_after_typing_inner(&mut self) {
        if self.sel.len() > 1 {
            self.completions.deactivate();
            return;
        }
        let prefix = self.prefix_at_caret();
        if prefix.len() < Self::COMPLETION_MIN_PREFIX {
            self.completions.deactivate();
            return;
        }
        // Narrow BEFORE walking the scope. `refine` needs no symbol list, and
        // it is the overwhelmingly common case: once the popup is up, every
        // further character of the same identifier only narrows it.
        //
        // The walk used to run first, unconditionally, and its result was then
        // thrown away on exactly those keys — an O(file) scan to find the
        // caret's byte offset plus a full visibility walk, on every identifier
        // character. Measured at 28.5 ms per key at 20k lines and 181 ms at
        // 60k, before Swift or paint.
        if self.completions.active {
            self.completions.refine(&prefix);
            if self.completions.active {
                self.request_lsp_completions();
                return;
            }
            // `refine` only narrows; falling through re-widens, which is what
            // deleting back to a shorter prefix needs.
        }
        let ext = self.file_extension();
        let t_scope = std::time::Instant::now();
        let symbols = self.symbols_in_scope_at_caret(&prefix);
        self.completions.last_scope_us = t_scope.elapsed().as_micros() as u32;
        self.completions
            .activate_with(&prefix, ext.as_deref(), &symbols);
        self.request_lsp_completions();
    }

    /// Two characters, not one: a popup on every single letter is noise, and
    /// the suggestion list for a one-character prefix is never useful.
    const COMPLETION_MIN_PREFIX: usize = 2;

    /// Symbols lexically visible at the caret — see `crate::scope`.
    ///
    /// Reads the WARM tree rather than parsing: this runs on the typing path,
    /// where a cold parse would cost far more than the suggestions are worth.
    /// When the tree is stale relative to the buffer (mid-edit, before the next
    /// `parse`) the offsets would name the wrong nodes, so it returns nothing
    /// and completion falls back to keywords — a missing suggestion is a much
    /// smaller failure than a wrong one.
    ///
    /// Current file only. Symbols from other open files are deliberately out of
    /// scope here: the indexer has their trees, but "every identifier in the
    /// project" is a different feature with a different ranking problem, and it
    /// is not what lexical visibility means.
    ///
    /// `prefix` is passed down so the walk can skip building a symbol that
    /// cannot match it. `activate_with` still does the authoritative filtering
    /// — this only stops the list being 2,500 entries long on its way there.
    fn symbols_in_scope_at_caret(&mut self, prefix: &str) -> Vec<crate::scope::ScopeSymbol> {
        let Some(lang) = crate::scope::ScopeLang::from_ext(self.syntax.live_ext()) else {
            return Vec::new();
        };
        let tree_gen = self.syntax.live_tree_gen();
        // Disjoint fields: `syntax` is borrowed for the tree, `scope_cache`
        // mutably for the global list. Same `self`, different fields, so the
        // borrow checker allows both at once.
        let cache = &mut self.scope_cache;
        let Some((tree, text)) = self.syntax.live_tree() else {
            return Vec::new();
        };
        // Byte offset of the caret within the tree's own text.
        //
        // `self.sel.primary().head`, NOT `buffer.cursor` — this must be the
        // same caret `prefix_at_caret` used. Reading a different one would rank
        // the scope of one position against the prefix of another, which is a
        // wrong answer that still looks plausible.
        let cursor = self.sel.primary().head;
        // Through the cache's line index, not by walking the file. This used to
        // scan `text.lines()` from the top on every keystroke — with the caret
        // mid-file that is half the document per key, and it is half of what
        // the comment above measured at 28.5 ms. `None` is the same answer the
        // walk gave by falling off its end: the caret is past the tree's last
        // line, so the tree is behind the buffer and cannot place it.
        let Some(byte) = cache.byte_of(text, tree_gen, cursor.row, cursor.col) else {
            return Vec::new();
        };
        crate::scope::visible_at_cached(tree, text, byte, lang, cache, tree_gen, prefix)
    }

    fn request_lsp_completions(&mut self) {
        if !self.lsp.server_running {
            return;
        }
        self.sync_lsp_document();
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            return;
        };
        let c = self.buffer.cursor();
        self.lsp.request_completion(&path, c.row, c.col);
    }

    /// Insert the selected suggestion, replacing the typed prefix.
    /// Returns false when nothing was open to accept.
    pub fn completion_accept(&mut self) -> bool {
        if !self.completions.active {
            return false;
        }
        let Some(text) = self
            .completions
            .selected_suggestion()
            .map(|s| s.insert_text.clone())
        else {
            self.completions.deactivate();
            return false;
        };
        let prefix_len = self.prefix_at_caret().chars().count();
        self.completions.deactivate();
        // Delete what was typed, then insert the full suggestion — one undo
        // group, so a rejected completion comes back in a single step.
        for _ in 0..prefix_len {
            self.gui_delete_backward();
        }
        self.gui_insert_text(&text);
        true
    }

    pub fn completion_move(&mut self, forward: bool) -> bool {
        if !self.completions.active {
            return false;
        }
        if forward {
            self.completions.next();
        } else {
            self.completions.prev();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    fn app_with(text: &str) -> App {
        let mut app = App::new();
        app.buffer = Buffer::from_string(text);
        app.caret_place(Position::zero());
        app
    }

    #[test]
    fn auto_pair_closes_a_bracket_and_leaves_caret_inside() {
        let mut app = app_with("");
        app.gui_insert_text("(");
        assert_eq!(app.buffer.text(), "()");
        assert_eq!(app.sel.primary().head, Position::new(0, 1), "caret between");
    }

    #[test]
    fn auto_pair_quotes_and_braces() {
        let mut app = app_with("");
        app.gui_insert_text("\"");
        app.gui_insert_text("{");
        assert_eq!(app.buffer.text(), "\"{}\"");
        assert_eq!(app.sel.primary().head, Position::new(0, 2));
    }

    #[test]
    fn typing_the_closer_skips_over_the_auto_inserted_one() {
        let mut app = app_with("");
        app.gui_insert_text("("); // -> "(|)"
        app.gui_insert_text(")"); // type-over, not a second ")"
        assert_eq!(app.buffer.text(), "()");
        assert_eq!(
            app.sel.primary().head,
            Position::new(0, 2),
            "caret past closer"
        );
    }

    #[test]
    fn backspace_between_an_empty_pair_removes_both() {
        let mut app = app_with("");
        app.gui_insert_text("("); // "(|)"
        app.gui_delete_backward();
        assert_eq!(app.buffer.text(), "");
        assert_eq!(app.sel.primary().head, Position::new(0, 0));
    }

    #[test]
    fn opener_before_a_word_does_not_pair() {
        // Typing "(" right before existing text must not inject a stray ")".
        let mut app = app_with("word");
        app.caret_place(Position::zero());
        app.gui_insert_text("(");
        assert_eq!(app.buffer.text(), "(word");
    }

    #[test]
    fn apostrophe_after_a_letter_does_not_pair() {
        let mut app = app_with("dont");
        app.caret_place(Position::new(0, 3)); // don|t
        app.gui_insert_text("'");
        assert_eq!(app.buffer.text(), "don't", "no stray closing quote");
    }

    #[test]
    fn caret_move_right_is_grapheme_aware() {
        let mut app = app_with("héllo"); // é is one grapheme
        app.caret_move(Motion::Right);
        app.caret_move(Motion::Right);
        assert_eq!(app.sel.primary().head, Position::new(0, 2));
        assert!(app.sel.primary().is_empty());
    }

    #[test]
    fn shift_right_extends_selection_keeping_anchor() {
        let mut app = app_with("hello world");
        app.caret_extend(Motion::Right);
        app.caret_extend(Motion::Right);
        let s = app.sel.primary();
        assert_eq!(s.anchor, Position::new(0, 0));
        assert_eq!(s.head, Position::new(0, 2));
        assert_eq!(
            app.gui_selection_range(),
            Some((Position::new(0, 0), Position::new(0, 2)))
        );
    }

    #[test]
    fn plain_left_collapses_selection_to_near_edge() {
        let mut app = app_with("hello");
        app.caret_extend(Motion::Right);
        app.caret_extend(Motion::Right);
        app.caret_extend(Motion::Right); // selection cols 0..3
        app.caret_move(Motion::Left); // collapse to start, not 2
        assert!(app.sel.primary().is_empty());
        assert_eq!(app.sel.primary().head, Position::new(0, 0));
    }

    #[test]
    fn vertical_move_preserves_goal_column_across_short_line() {
        // Long, short, long — moving down then down should return to col 8.
        let mut app = app_with("0123456789\nab\n0123456789");
        app.caret_place(Position::new(0, 8));
        app.caret_move(Motion::Down); // onto "ab" (len 2) → col clamps to 2
        assert_eq!(app.sel.primary().head, Position::new(1, 2));
        app.caret_move(Motion::Down); // back onto long line → col restores to 8
        assert_eq!(app.sel.primary().head, Position::new(2, 8));
    }

    #[test]
    fn word_motion_extends() {
        let mut app = app_with("foo bar baz");
        app.caret_extend(Motion::WordRight);
        let s = app.sel.primary();
        assert_eq!(s.anchor, Position::new(0, 0));
        assert!(s.head.col >= 3); // past "foo"
    }

    #[test]
    fn drag_sets_anchor_then_head() {
        let mut app = app_with("hello world");
        app.caret_place(Position::new(0, 2));
        app.caret_drag_to(Position::new(0, 7));
        let s = app.sel.primary();
        assert_eq!(s.anchor, Position::new(0, 2));
        assert_eq!(s.head, Position::new(0, 7));
    }

    #[test]
    fn cmd_click_adds_cursor() {
        let mut app = app_with("line0\nline1\nline2");
        app.caret_place(Position::new(0, 0));
        app.caret_add(Position::new(2, 0));
        assert_eq!(app.sel.len(), 2);
        assert!(app.sel.is_multi());
    }

    #[test]
    fn secondary_carets_exclude_the_primary() {
        // The render paints the primary through caret_*/sel_*; the extras come
        // from secondary_caret_positions(). Together they must cover every head
        // exactly once, with no double-count of the primary.
        let mut app = app_with("line0\nline1\nline2");
        app.caret_place(Position::new(0, 0));
        app.caret_add(Position::new(2, 3)); // this one becomes primary
        let secondaries = app.secondary_caret_positions();
        assert_eq!(secondaries.len(), 1);
        assert_eq!(secondaries[0], Position::new(0, 0));
        // Primary head + secondaries == the full set of caret heads.
        let mut heads = secondaries;
        heads.push(app.sel.primary().head);
        heads.sort();
        assert_eq!(heads, vec![Position::new(0, 0), Position::new(2, 3)]);
    }

    #[test]
    fn secondary_carets_empty_for_single_cursor() {
        let mut app = app_with("solo");
        app.caret_place(Position::new(0, 2));
        assert!(app.secondary_caret_positions().is_empty());
    }

    #[test]
    fn select_all_spans_document() {
        let mut app = app_with("ab\ncde");
        app.select_all_gui();
        assert_eq!(
            app.gui_selection_range(),
            Some((Position::new(0, 0), Position::new(1, 3)))
        );
    }

    #[test]
    fn clicks_clamp_out_of_range_positions() {
        let mut app = app_with("hi");
        app.caret_place(Position::new(99, 99));
        assert_eq!(app.sel.primary().head, Position::new(0, 2));
    }

    #[test]
    fn gui_selection_feeds_copy_and_selected_range() {
        // The whole point of the single-source bridge: a GUI selection made by
        // the caret commands is what copy yanks, with no vim Visual mode.
        let mut app = app_with("hello world");
        app.caret_place(Position::new(0, 0));
        for _ in 0..5 {
            app.caret_extend(Motion::Right); // select "hello"
        }
        assert!(app.has_selection());
        app.clipboard_copy();
        assert_eq!(app.yank_buffer.as_deref(), Some("hello"));
        // selected_range is the inclusive bridge: [0,0]..[0,4] for "hello".
        assert_eq!(
            app.selected_range(),
            Some((Position::new(0, 0), Position::new(0, 4)))
        );
    }

    #[test]
    fn unicode_gui_selection_copies_exact_text() {
        let mut app = app_with("a한글🙂b");
        app.caret_place(Position::new(0, 1));
        app.caret_drag_to(Position::new(0, 4));
        app.clipboard_copy();
        assert_eq!(app.yank_buffer.as_deref(), Some("한글🙂"));
        assert_eq!(
            app.selected_range(),
            Some((Position::new(0, 1), Position::new(0, 3)))
        );
    }

    #[test]
    fn caret_with_no_selection_has_no_range() {
        let mut app = app_with("hello");
        app.caret_place(Position::new(0, 3));
        assert!(!app.has_selection());
        assert_eq!(app.selected_range(), None);
    }

    #[test]
    fn insert_at_caret_types_text() {
        let mut app = app_with("helloworld");
        app.caret_place(Position::new(0, 5));
        app.gui_insert_text(" ");
        assert_eq!(app.buffer.text(), "hello world");
        assert_eq!(app.sel.primary().head, Position::new(0, 6));
        assert!(app.sel.primary().is_empty());
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut app = app_with("hello world");
        // select "hello"
        app.caret_place(Position::new(0, 0));
        for _ in 0..5 {
            app.caret_extend(Motion::Right);
        }
        app.gui_insert_text("HI");
        assert_eq!(app.buffer.text(), "HI world");
        assert!(app.sel.primary().is_empty());
        assert_eq!(app.sel.primary().head, Position::new(0, 2));
    }

    #[test]
    fn backspace_deletes_grapheme_or_selection() {
        let mut app = app_with("héllo"); // é one grapheme
        app.caret_place(Position::new(0, 2)); // after é
        app.gui_delete_backward();
        assert_eq!(app.buffer.text(), "hllo");
        // now delete a selection
        app.caret_place(Position::new(0, 0));
        app.caret_extend(Motion::Right);
        app.caret_extend(Motion::Right);
        app.gui_delete_backward();
        assert_eq!(app.buffer.text(), "lo");
    }

    #[test]
    fn backspace_at_line_start_joins_previous() {
        let mut app = app_with("ab\ncd");
        app.caret_place(Position::new(1, 0));
        app.gui_delete_backward();
        assert_eq!(app.buffer.text(), "abcd");
        assert_eq!(app.sel.primary().head, Position::new(0, 2));
    }

    #[test]
    fn forward_delete_removes_next_or_joins() {
        let mut app = app_with("ab\ncd");
        app.caret_place(Position::new(0, 2)); // end of "ab"
        app.gui_delete_forward(); // join next line
        assert_eq!(app.buffer.text(), "abcd");
        app.caret_place(Position::new(0, 0));
        app.gui_delete_forward(); // delete 'a'
        assert_eq!(app.buffer.text(), "bcd");
    }

    /// Auto-indent carries the line's indent DOWN. It must not also leave it
    /// behind on the text that moved.
    ///
    /// `after` — everything past the caret — already contains whatever leading
    /// whitespace the caret has not passed. Prefixing the indent on top of that
    /// counted it twice: Enter at column 0 of an indented line produced an
    /// empty line and a line indented twice over, and every Enter inside an
    /// indent grew it again. Reported as "새로운 줄인데 맨 앞에 한 칸이 빈다";
    /// the space was real, and the editor put it there.
    #[test]
    fn enter_inside_an_indent_does_not_duplicate_it() {
        // At column 0: the whole line moves down unchanged.
        let mut app = app_with("  abc");
        app.caret_place(Position::new(0, 0));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(0), "");
        assert_eq!(app.buffer.line(1), "  abc", "indent not doubled");

        // Halfway through the indent: the whitespace is split, not multiplied.
        let mut app = app_with("  abc");
        app.caret_place(Position::new(0, 1));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(0), " ");
        assert_eq!(app.buffer.line(1), " abc");

        // A line that is only whitespace, split in the middle: still two
        // spaces in total, not four.
        let mut app = app_with("    ");
        app.caret_place(Position::new(0, 2));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(0), "  ");
        assert_eq!(app.buffer.line(1), "  ");
    }

    /// And past the indent it still does its job — that is the whole point.
    #[test]
    fn enter_past_the_indent_carries_it_down() {
        let mut app = app_with("  abc");
        app.caret_place(Position::new(0, 5));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(1), "  ", "new line starts at the indent");
        assert_eq!(app.buffer.cursor().col, 2, "caret after it");

        // Splitting after the indent keeps the tail at the indent.
        let mut app = app_with("  abcdef");
        app.caret_place(Position::new(0, 5));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(0), "  abc");
        assert_eq!(app.buffer.line(1), "  def");

        // An opener still adds one unit on top.
        let mut app = app_with("  if x {");
        app.caret_place(Position::new(0, 8));
        app.gui_insert_newline("  ");
        assert_eq!(app.buffer.line(1), "    ", "indent + one unit");
    }

    /// Auto-indent is a CODE affordance, and asks the language.
    ///
    /// README.md line 10 is a wrapped Markdown bullet's continuation, indented
    /// two spaces to keep it under the bullet. Enter at its end used to hand
    /// you those two spaces on a line that is a new thought — reported as
    /// "엔터하면 라인 앞에 탭처럼 스페이스가 하나 생김".
    #[test]
    fn markdown_does_not_carry_an_indent_down() {
        let line = "  final development snapshot (e.g. `2026dev`) when the ";

        let mut app = app_with(line);
        app.filename = Some(std::path::PathBuf::from("/tmp/README.md"));
        let end = app.buffer.line(0).chars().count();
        app.caret_place(Position::new(0, end));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(1), "", "prose starts at column 1");

        // The same text in a code file still does.
        let mut app = app_with(line);
        app.filename = Some(std::path::PathBuf::from("/tmp/a.rs"));
        app.caret_place(Position::new(0, end));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(1), "  ", "code keeps its depth");
    }

    /// An opener is a block in code and a sentence in prose, so the bonus
    /// answers to the same question the indent does.
    #[test]
    fn markdown_does_not_open_a_block_on_a_colon() {
        let mut app = app_with("Options:");
        app.filename = Some(std::path::PathBuf::from("/tmp/notes.md"));
        app.caret_place(Position::new(0, 8));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(1), "");

        let mut app = app_with("def f():");
        app.filename = Some(std::path::PathBuf::from("/tmp/a.py"));
        app.caret_place(Position::new(0, 8));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(1), "    ", "python opens a block");
    }

    /// A file nothing claims gets the indent. An unknown file is more likely
    /// code than not, and the failure is asymmetric: a missing indent in code
    /// is retyped every line, a stray one in prose is deleted once.
    #[test]
    fn an_unknown_file_still_indents() {
        let mut app = app_with("  x");
        app.filename = Some(std::path::PathBuf::from("/tmp/thing.wat"));
        app.caret_place(Position::new(0, 3));
        app.gui_insert_newline("  ");
        assert_eq!(app.buffer.line(1), "  ");

        let mut app = app_with("  x");
        app.filename = None;
        app.caret_place(Position::new(0, 3));
        app.gui_insert_newline("  ");
        assert_eq!(app.buffer.line(1), "  ");
    }

    #[test]
    fn newline_splits_the_line() {
        let mut app = app_with("abcd");
        app.caret_place(Position::new(0, 2));
        app.gui_insert_newline("    ");
        assert_eq!(app.buffer.line(0), "ab");
        assert_eq!(app.buffer.line(1), "cd");
        assert_eq!(app.sel.primary().head.row, 1);
    }

    #[test]
    fn multi_cursor_insert_types_at_every_caret() {
        let mut app = app_with("a\nb\nc");
        app.caret_place(Position::new(0, 1));
        app.caret_add(Position::new(1, 1));
        app.caret_add(Position::new(2, 1));
        assert_eq!(app.sel.len(), 3);
        app.gui_insert_text("!");
        assert_eq!(app.buffer.text(), "a!\nb!\nc!");
        assert_eq!(app.sel.len(), 3, "still three carets");
    }

    #[test]
    fn multi_cursor_backspace_deletes_at_every_caret() {
        let mut app = app_with("aX\nbX\ncX");
        app.caret_place(Position::new(0, 2));
        app.caret_add(Position::new(1, 2));
        app.caret_add(Position::new(2, 2));
        app.gui_delete_backward();
        assert_eq!(app.buffer.text(), "a\nb\nc");
    }

    #[test]
    fn typing_run_undoes_as_one_group() {
        let mut app = app_with("");
        app.caret_place(Position::new(0, 0));
        for c in "hello".chars() {
            app.gui_insert_text(&c.to_string());
        }
        assert_eq!(app.buffer.text(), "hello");
        app.undo(); // one undo reverts the whole coalesced run
        assert_eq!(app.buffer.text(), "");
    }

    #[test]
    fn caret_move_breaks_the_undo_run() {
        let mut app = app_with("");
        app.caret_place(Position::new(0, 0));
        app.gui_insert_text("a");
        app.caret_move(Motion::Right); // boundary (no-op move still resets run)
        app.gui_insert_text("b");
        assert_eq!(app.buffer.text(), "ab");
        app.undo(); // reverts only "b"
        assert_eq!(app.buffer.text(), "a");
    }

    #[test]
    fn buffer_cursor_mirrors_primary_head() {
        let mut app = app_with("hello");
        app.caret_move(Motion::Right);
        app.caret_extend(Motion::Right);
        assert_eq!(app.buffer.cursor(), app.sel.primary().head);
    }

    #[test]
    fn undo_resyncs_gui_caret_before_cjk_paste() {
        let mut app = app_with("middle 한국어 insertion");
        app.caret_place(Position::new(0, 7));

        app.gui_insert_text("xptmxm");
        assert_eq!(app.buffer.line(0), "middle xptmxm한국어 insertion");
        app.undo();
        assert_eq!(app.buffer.cursor(), Position::new(0, 7));

        app.gui_insert_text("테스트");
        assert_eq!(app.buffer.line(0), "middle 테스트한국어 insertion");
        assert_eq!(app.buffer.cursor(), Position::new(0, 10));
        assert_eq!(app.sel.primary().head, Position::new(0, 10));
    }

    #[test]
    fn redo_resyncs_gui_caret_before_typing() {
        let mut app = app_with("abc");
        app.caret_place(Position::new(0, 1));
        app.gui_insert_text("X");
        app.undo();
        app.redo();

        app.gui_insert_text("Y");
        assert_eq!(app.buffer.line(0), "aXYbc");
        assert_eq!(app.buffer.cursor(), Position::new(0, 3));
        assert_eq!(app.sel.primary().head, Position::new(0, 3));
    }
}

// ── ⌘/ ─────────────────────────────────────────────────────────────────────

impl App {
    /// Comment or uncomment every line the selection touches.
    ///
    /// The token comes from `highlight::rules_for_ext`, which has known it for
    /// twenty-five languages since the highlighter was written and had no way
    /// to be asked.
    ///
    /// Returns false when this language has no line comment — JSON has none,
    /// and inventing one produces a file that will not parse.
    pub fn toggle_line_comment(&mut self) -> bool {
        let rules = crate::highlight::rules_for_ext(self.file_extension().as_deref());
        let Some(token) = rules.line_comment() else {
            self.message = "No line comment in this file type".into();
            return false;
        };
        let rows = self.comment_rows();
        if rows.is_empty() {
            return false;
        }

        // Uncomment only when EVERY line that could be commented already is.
        // Mixed goes the other way — so a block half-commented by hand becomes
        // wholly commented, and pressing again restores it.
        let mut any_content = false;
        let all_commented = rows.iter().all(|&row| {
            let line = self.buffer.line(row);
            if line.trim().is_empty() {
                return true;
            }
            any_content = true;
            line.trim_start().starts_with(token)
        });

        self.push_undo();
        self.edit_run = crate::app::EditRun::None;
        if all_commented && any_content {
            for &row in &rows {
                self.uncomment_row(row, token);
            }
        } else {
            // At the SHALLOWEST indentation in the block, not at column zero.
            // A comment token jammed against the left margin destroys the
            // shape of the code it is commenting out, and the shape is how
            // anyone reads what they just switched off.
            let col = rows
                .iter()
                .map(|&row| self.buffer.line(row))
                .filter(|line| !line.trim().is_empty())
                .map(|line| visual_indent(&line, self.tab_width))
                .min()
                .unwrap_or(0);
            let alone = rows.len() == 1;
            for &row in &rows {
                self.comment_row(row, token, col, alone);
            }
        }
        self.sync_after_comment(&rows);
        true
    }

    /// Every buffer row any caret or selection touches, once each, in order.
    ///
    /// A selection that ENDS at column zero does not include that last row:
    /// dragging down to the start of a line selects the lines above it, and
    /// commenting one the user cannot see selected is a surprise.
    fn comment_rows(&self) -> Vec<usize> {
        let last = self.buffer.line_count().saturating_sub(1);
        let mut rows: Vec<usize> = Vec::new();
        for sel in self.sel.all() {
            let (start, end) = (sel.start(), sel.end());
            let stop = if end.row > start.row && end.col == 0 {
                end.row - 1
            } else {
                end.row
            };
            for row in start.row..=stop.min(last) {
                if !rows.contains(&row) {
                    rows.push(row);
                }
            }
        }
        rows.sort_unstable();
        rows
    }

    fn comment_row(&mut self, row: usize, token: &str, col: usize, alone: bool) {
        let line = self.buffer.line(row);
        // A blank line inside a block gets nothing — a trailing `// ` on an
        // empty line is litter. A blank line ON ITS OWN does get one: pressing
        // ⌘/ on an empty line means "start a comment here", and that is the
        // only reading of it.
        if line.trim().is_empty() && !alone {
            return;
        }
        let at = char_col_for_visual(&line, col, self.tab_width);
        self.buffer.cursor = Position { row, col: at };
        self.buffer.insert_str(&format!("{token} "));
    }

    fn uncomment_row(&mut self, row: usize, token: &str) {
        let line = self.buffer.line(row);
        let trimmed = line.trim_start();
        if !trimmed.starts_with(token) {
            return;
        }
        // CHARACTERS, not bytes: `Position::col` is a char column everywhere
        // in this codebase, and `line.len() - trimmed.len()` is a byte count
        // that agrees with it only while the indentation is ASCII. A no-break
        // space in the indent would have put the cut in the wrong place.
        let indent = line.chars().count() - trimmed.chars().count();
        // The token, and ONE space after it if the comment was written the way
        // this writes them. Two spaces are the reader's, not ours.
        let mut width = token.chars().count();
        if trimmed[token.len()..].starts_with(' ') {
            width += 1;
        }
        self.buffer.delete_range(
            Position { row, col: indent },
            Position {
                row,
                col: indent + width,
            },
        );
    }

    /// Put the caret back where the reader left it, allowing for what moved.
    fn sync_after_comment(&mut self, rows: &[usize]) {
        let mut out: Vec<Selection> = Vec::new();
        for sel in self.sel.all() {
            let fix = |p: Position| -> Position {
                let len = self.buffer.line(p.row).len();
                Position {
                    row: p.row.min(self.buffer.line_count().saturating_sub(1)),
                    col: p.col.min(len),
                }
            };
            out.push(Selection {
                anchor: fix(sel.anchor),
                head: fix(sel.head),
                goal_x: None,
            });
        }
        let primary = self.sel.primary_index();
        self.sel = crate::selection::SelectionSet::spans(out, primary);
        self.buffer.cursor = self.sel.primary().head;
        let _ = rows;
    }
}

/// How far into the line the first non-space character sits, in visual columns.
fn visual_indent(line: &str, tab_width: usize) -> usize {
    let mut col = 0;
    for c in line.chars() {
        match c {
            ' ' => col += 1,
            '\t' => col += tab_width - (col % tab_width.max(1)),
            _ => break,
        }
    }
    col
}

/// The CHAR column a visual column lands on, clamped to the line's own indent.
///
/// Chars rather than bytes because that is what `Position::col` is. The two
/// agree for ASCII indentation, which is nearly all of it — and "nearly" is
/// how a tab-vs-space bug hides for a year.
fn char_col_for_visual(line: &str, want: usize, tab_width: usize) -> usize {
    let mut col = 0;
    for (i, c) in line.chars().enumerate() {
        if col >= want {
            return i;
        }
        match c {
            ' ' => col += 1,
            '\t' => col += tab_width - (col % tab_width.max(1)),
            // Past the indentation: a line shallower than the block's minimum
            // takes the token at its own start rather than inside its text.
            _ => return i,
        }
    }
    line.chars().count()
}

// ── Find and replace, in the file on screen ────────────────────────────────
//
// Replacing across the project has worked since `workspace_search.rs` was
// written. The file in front of you was the one place it could not be done,
// which is backwards — and the two are not the same job: that one rewrites
// files on disk, and this one goes through the buffer, so it lands in undo,
// reaches the language server, and can be done to a document that has never
// been saved.

impl App {
    /// Replace the match the caret is on, and move to the next.
    ///
    /// Returns false when there is nothing to replace.
    pub fn replace_current(&mut self) -> bool {
        let Some(pattern) = self.search.pattern.clone().filter(|p| !p.is_empty()) else {
            return false;
        };
        let with = self.search.replace_input.clone();
        let Some(&at) = self.search.matches.get(self.search.current) else {
            return false;
        };
        self.push_undo();
        self.edit_run = crate::app::EditRun::None;
        self.splice_match(at, &pattern, &with);
        self.recollect_matches(at, &with);
        true
    }

    /// Replace every match in this buffer, as one edit.
    ///
    /// Returns how many. Overlapping matches count once: searching `aa` in
    /// `aaaa` finds three (the collector allows overlap on purpose, so `n`
    /// steps through them), and replacing all three would consume text twice.
    pub fn replace_all_in_buffer(&mut self) -> usize {
        let Some(pattern) = self.search.pattern.clone().filter(|p| !p.is_empty()) else {
            return 0;
        };
        let with = self.search.replace_input.clone();
        let width = pattern.chars().count();
        let mut accepted: Vec<Position> = Vec::new();
        for &m in &self.search.matches {
            match accepted.last() {
                Some(prev) if prev.row == m.row && m.col < prev.col + width => continue,
                _ => accepted.push(m),
            }
        }
        if accepted.is_empty() {
            return 0;
        }
        self.push_undo();
        self.edit_run = crate::app::EditRun::None;
        // Last to first: a replacement of a different length moves everything
        // after it on that line, and nothing before it.
        for &at in accepted.iter().rev() {
            self.splice_match(at, &pattern, &with);
        }
        let n = accepted.len();
        let last = *accepted.last().expect("checked non-empty");
        self.recollect_matches(last, &with);
        self.message = format!("Replaced {n}");
        n
    }

    /// Swap one occurrence for the replacement, keeping the text that is there.
    ///
    /// The pattern's LENGTH is what is removed, not the pattern itself: the
    /// search is smart-case, so `foo` matches `Foo`, and cutting by length is
    /// what removes the text that actually matched rather than the query that
    /// found it.
    fn splice_match(&mut self, at: Position, pattern: &str, with: &str) {
        let width = pattern.chars().count();
        let end = Position {
            row: at.row,
            col: at.col + width,
        };
        self.buffer.delete_range(at, end);
        self.buffer.cursor = at;
        if !with.is_empty() {
            self.buffer.insert_str(with);
        }
    }

    /// Rebuild the match list and land on the next one after the edit.
    ///
    /// Positions are stale the moment the text moves, and a stale list is
    /// worse than no list: `n` would walk to a column that no longer holds a
    /// match and the highlight would be drawn over ordinary text.
    fn recollect_matches(&mut self, edited: Position, with: &str) {
        let Some(pattern) = self.search.pattern.clone() else {
            return;
        };
        self.search.matches =
            crate::search::SearchState::collect(self.buffer.lines(), &pattern);
        let after = Position {
            row: edited.row,
            col: edited.col + with.chars().count(),
        };
        self.search.current = self
            .search
            .matches
            .iter()
            .position(|m| *m >= after)
            .unwrap_or(0);
        if let Some(&next) = self.search.matches.get(self.search.current) {
            self.buffer.cursor = next;
            self.sel = crate::selection::SelectionSet::single(Selection::caret(next));
        } else {
            self.buffer.cursor = after;
            self.sel = crate::selection::SelectionSet::single(Selection::caret(after));
        }
        self.update_scroll();
    }
}
