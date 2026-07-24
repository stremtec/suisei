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
        matches!(self, Motion::Up | Motion::Down)
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
            Motion::Up | Motion::Down => {}
        }
        let target = self.buffer.cursor;
        self.buffer.cursor = saved;
        target
    }

    /// Vertical motion that preserves the goal column across short lines.
    /// Returns the new head and the goal column to carry forward.
    fn vertical_target(&self, from: Position, goal_x: Option<usize>, up: bool) -> (Position, usize) {
        let goal = goal_x.unwrap_or(from.col);
        let row = if up {
            from.row.saturating_sub(1)
        } else {
            (from.row + 1).min(self.buffer.line_count().saturating_sub(1))
        };
        let len = self.buffer.line(row).chars().count();
        (Position::new(row, goal.min(len)), goal)
    }

    /// Resolve a motion against the primary head, returning the new head and
    /// the goal column to store on the selection.
    fn apply_motion(&mut self, motion: Motion) -> (Position, Option<usize>) {
        let head = self.sel.primary().head;
        if motion.is_vertical() {
            let (pos, goal) =
                self.vertical_target(head, self.sel.primary().goal_x, motion == Motion::Up);
            (pos, Some(goal))
        } else {
            (self.motion_target(head, motion), None)
        }
    }

    /// Move the caret (no Shift): collapse any selection and move the head.
    /// From a non-empty selection, a plain Left/Right lands on the near/far
    /// edge without moving further — the macOS text-field convention.
    pub fn caret_move(&mut self, motion: Motion) {
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
        self.sel.set_primary(Selection { anchor: head, head, goal_x: goal });
        self.buffer.cursor = head;
    }

    /// Extend the selection (Shift): keep the anchor, move only the head.
    /// If there is no selection yet, the current head becomes the anchor.
    pub fn caret_extend(&mut self, motion: Motion) {
        let sel = self.sel.primary();
        let (head, goal) = self.apply_motion(motion);
        self.sel.set_primary(Selection { anchor: sel.anchor, head, goal_x: goal });
        self.buffer.cursor = head;
    }

    /// Place a single caret at `pos` (a plain click), discarding other cursors.
    pub fn caret_place(&mut self, pos: Position) {
        let clamped = self.clamp_to_document(pos);
        self.sel = crate::selection::SelectionSet::single(Selection::caret(clamped));
        self.buffer.cursor = clamped;
    }

    /// Extend the primary selection's head to `pos` (a drag, or Shift+Click).
    pub fn caret_drag_to(&mut self, pos: Position) {
        let clamped = self.clamp_to_document(pos);
        let anchor = self.sel.primary().anchor;
        self.sel.set_primary(Selection::new(anchor, clamped));
        self.buffer.cursor = clamped;
    }

    /// Add a caret at `pos` (⌘-click multi-cursor).
    pub fn caret_add(&mut self, pos: Position) {
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
        let range = crate::ops::range_for_textobject(&self.buffer, crate::ops::TextObject::InnerWord);
        self.buffer.cursor = saved;
        if let Some(r) = range {
            // `range_for_textobject` end is exclusive already — exactly the GUI
            // head. anchor = word start, head = one past the word.
            self.sel = crate::selection::SelectionSet::single(Selection::new(r.start, r.end));
            self.buffer.cursor = r.end;
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
    pub fn gui_insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.push_undo();
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
        self.push_undo();
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
            self.buffer.insert_newline_smart(indent_unit);
            heads[i] = self.buffer.cursor;
        }
        let primary = self.sel.primary_index();
        self.sel = crate::selection::SelectionSet::carets(&heads, primary);
        self.buffer.cursor = self.sel.primary().head;
    }

    /// Backspace: delete the selection, or one grapheme before each caret.
    pub fn gui_delete_backward(&mut self) {
        self.push_undo();
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
        self.push_undo();
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
        if s.is_empty() {
            None
        } else {
            Some(s.range())
        }
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
        assert_eq!(app.gui_selection_range(), Some((Position::new(0, 0), Position::new(0, 2))));
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
        assert!(!matches!(app.mode, crate::app::Mode::Visual));
        app.clipboard_copy();
        assert_eq!(app.yank_buffer.as_deref(), Some("hello"));
        // selected_range is the inclusive bridge: [0,0]..[0,4] for "hello".
        assert_eq!(
            app.selected_range(),
            Some((Position::new(0, 0), Position::new(0, 4)))
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
    fn buffer_cursor_mirrors_primary_head() {
        let mut app = app_with("hello");
        app.caret_move(Motion::Right);
        app.caret_extend(Motion::Right);
        assert_eq!(app.buffer.cursor(), app.sel.primary().head);
    }
}
