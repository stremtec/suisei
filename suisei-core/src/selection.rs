//! GUI selection model — the foundation of Suisei's first-class editing.
//!
//! The core historically carried one `Buffer.cursor` plus a vim-style
//! `visual_anchor`/`Mode` pair whose selection is **inclusive** (the head sits
//! *on* a character). A GUI needs the opposite: an **exclusive** selection
//! whose head sits *between* characters, so that
//!
//!   * a caret is simply an empty selection (`anchor == head`);
//!   * Shift+Arrow extends by moving `head` while `anchor` stays put;
//!   * typing replaces exactly `[start, end)` with no off-by-one;
//!   * multiple selections are the same type in a `Vec`, so multi-cursor is
//!     inherent rather than bolted on.
//!
//! This module is pure geometry over `Position`. It does not touch buffer text
//! or movement — those need `&Buffer` context and are wired in a later patch.
//! Keeping it standalone is deliberate: it is fully unit-tested here before any
//! of the App state machine depends on it (delivery discipline in
//! `docs/SUISEI-CURRENT-STATE.md`).

use crate::buffer::Position;

/// A single selection. A caret is the empty case, `anchor == head`.
///
/// `anchor` is the fixed end (where the selection began); `head` is the moving
/// end (where the cursor visibly is). They are ordered by the *user's* gesture,
/// not by document order — `anchor` may come after `head` in the buffer when
/// the user selected backwards. Use [`Selection::range`] for the normalised
/// `[start, end)` span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Position,
    pub head: Position,
    /// Desired column for vertical movement, preserved across short lines so a
    /// caret returns to its original column after passing one. `None` until the
    /// first vertical move sets it. Not part of identity — two selections with
    /// the same anchor/head are equal regardless of `goal_x` for merge
    /// purposes; see [`Selection::same_span`].
    pub goal_x: Option<usize>,
}

impl Selection {
    /// An empty selection (a caret) at `pos`.
    pub fn caret(pos: Position) -> Self {
        Self {
            anchor: pos,
            head: pos,
            goal_x: None,
        }
    }

    /// A selection from `anchor` to `head` (either document order).
    pub fn new(anchor: Position, head: Position) -> Self {
        Self {
            anchor,
            head,
            goal_x: None,
        }
    }

    /// True when this is a caret (no selected text).
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Normalised span in document order: `start <= end`, half-open `[start,
    /// end)`. `start == end` for a caret.
    pub fn range(&self) -> (Position, Position) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The earlier of anchor/head in document order.
    pub fn start(&self) -> Position {
        self.range().0
    }

    /// The later of anchor/head in document order.
    pub fn end(&self) -> Position {
        self.range().1
    }

    /// Exclusive containment: a caret contains nothing, and `end` is never
    /// contained (the head boundary is not "inside" the span). This is what
    /// makes typing-over-selection replace exactly the visible highlight.
    pub fn contains(&self, pos: Position) -> bool {
        let (s, e) = self.range();
        pos >= s && pos < e
    }

    /// Collapse to a caret at the head (Arrow with no Shift after a selection).
    pub fn collapsed_to_head(&self) -> Self {
        Self {
            anchor: self.head,
            head: self.head,
            goal_x: self.goal_x,
        }
    }

    /// Collapse to a caret at the document-start edge (Left after a selection —
    /// the caret lands at the near edge, matching macOS text fields).
    pub fn collapsed_to_start(&self) -> Self {
        let s = self.start();
        Self {
            anchor: s,
            head: s,
            goal_x: None,
        }
    }

    /// Collapse to a caret at the document-end edge (Right after a selection).
    pub fn collapsed_to_end(&self) -> Self {
        let e = self.end();
        Self {
            anchor: e,
            head: e,
            goal_x: None,
        }
    }

    /// Move the head to `pos`, keeping the anchor — i.e. extend the selection
    /// (Shift+Arrow, Shift+Click). Clears `goal_x` unless the caller sets it.
    pub fn extended_to(&self, pos: Position) -> Self {
        Self {
            anchor: self.anchor,
            head: pos,
            goal_x: None,
        }
    }

    /// True when two selections cover the same anchor/head, ignoring `goal_x`.
    pub fn same_span(&self, other: &Self) -> bool {
        self.anchor == other.anchor && self.head == other.head
    }

    /// Do the two spans touch or overlap in document order? Adjacent caret and
    /// span (sharing an endpoint) count as overlapping so a merge collapses
    /// them — two carets at the same spot must not both survive.
    pub fn overlaps(&self, other: &Self) -> bool {
        let (a0, a1) = self.range();
        let (b0, b1) = other.range();
        a0 <= b1 && b0 <= a1
    }
}

/// An ordered set of selections with a designated primary.
///
/// Invariant: after any mutating operation the selections are sorted by
/// `start()` and no two overlap (overlapping ones are merged, keeping the
/// primary's head where possible). There is always at least one selection, so
/// `primary()` never panics — a document always has a caret.
#[derive(Clone, Debug)]
pub struct SelectionSet {
    selections: Vec<Selection>,
    /// Index into `selections` of the primary (the one the viewport follows and
    /// that reports as *the* cursor across the FFI).
    primary: usize,
}

impl SelectionSet {
    /// A fresh set with a single caret at the origin.
    pub fn new() -> Self {
        Self {
            selections: vec![Selection::caret(Position::zero())],
            primary: 0,
        }
    }

    /// A set with a single selection.
    pub fn single(sel: Selection) -> Self {
        Self {
            selections: vec![sel],
            primary: 0,
        }
    }

    /// Rebuild from a list of caret positions (one per prior selection), keeping
    /// the caret at index `primary` as primary. Used after a multi-cursor edit
    /// re-seats every caret.
    pub fn carets(heads: &[Position], primary: usize) -> Self {
        if heads.is_empty() {
            return Self::new();
        }
        let mut set = Self {
            selections: heads.iter().map(|&h| Selection::caret(h)).collect(),
            primary: primary.min(heads.len() - 1),
        };
        set.normalise_keeping_primary();
        set
    }

    /// Index of the primary within the current (sorted) selection list.
    pub fn primary_index(&self) -> usize {
        self.primary
    }

    pub fn all(&self) -> &[Selection] {
        &self.selections
    }

    pub fn len(&self) -> usize {
        self.selections.len()
    }

    pub fn is_multi(&self) -> bool {
        self.selections.len() > 1
    }

    /// The primary selection — the one the caret/viewport tracks.
    pub fn primary(&self) -> Selection {
        self.selections[self.primary]
    }

    /// Replace the primary selection, then restore the sorted/non-overlapping
    /// invariant. The primary index follows its selection through the re-sort.
    pub fn set_primary(&mut self, sel: Selection) {
        self.selections[self.primary] = sel;
        self.normalise_keeping_primary();
    }

    /// Collapse to a single caret at the primary's head — Escape, or a plain
    /// click. Discards every other selection.
    pub fn collapse_to_primary_caret(&mut self) {
        let head = self.primary().head;
        self.selections = vec![Selection::caret(head)];
        self.primary = 0;
    }

    /// Add a selection (⌘-click / ⌥-drag adds a cursor) and make it primary.
    /// Overlaps with existing selections are merged.
    pub fn add(&mut self, sel: Selection) {
        self.selections.push(sel);
        self.primary = self.selections.len() - 1;
        self.normalise_keeping_primary();
    }

    /// Apply `f` to every selection (an edit or a movement maps over all
    /// cursors), then re-establish the invariant.
    pub fn map(&mut self, mut f: impl FnMut(Selection) -> Selection) {
        for s in &mut self.selections {
            *s = f(*s);
        }
        self.normalise_keeping_primary();
    }

    /// Sort by start, merge overlaps, and keep `primary` pointing at whatever
    /// selection currently holds the primary head. Merged selections adopt the
    /// widest span; if the primary is absorbed, the absorbing selection becomes
    /// primary so the viewport does not jump to an unrelated cursor.
    fn normalise_keeping_primary(&mut self) {
        let primary_head = self.selections[self.primary].head;

        self.selections.sort_by_key(|s| s.start());

        let mut merged: Vec<Selection> = Vec::with_capacity(self.selections.len());
        for s in std::mem::take(&mut self.selections) {
            match merged.last_mut() {
                Some(prev) if prev.overlaps(&s) => {
                    // Union the spans. Keep the later head so a forward-growing
                    // multi-selection reads naturally.
                    let start = prev.start().min(s.start());
                    let end = prev.end().max(s.end());
                    *prev = Selection {
                        anchor: start,
                        head: end,
                        goal_x: s.goal_x,
                    };
                }
                _ => merged.push(s),
            }
        }
        self.selections = merged;

        // Re-locate the primary: the selection now covering the old primary
        // head, else the nearest by start, else clamp.
        self.primary = self
            .selections
            .iter()
            .position(|s| s.head == primary_head || s.contains(primary_head))
            .unwrap_or(0)
            .min(self.selections.len().saturating_sub(1));
    }
}

impl Default for SelectionSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    #[test]
    fn caret_is_empty_and_contains_nothing() {
        let c = Selection::caret(p(2, 5));
        assert!(c.is_empty());
        assert_eq!(c.range(), (p(2, 5), p(2, 5)));
        assert!(!c.contains(p(2, 5)));
    }

    #[test]
    fn range_normalises_backwards_selection() {
        let backward = Selection::new(p(3, 4), p(1, 2)); // anchor after head
        assert_eq!(backward.range(), (p(1, 2), p(3, 4)));
        assert_eq!(backward.start(), p(1, 2));
        assert_eq!(backward.end(), p(3, 4));
    }

    #[test]
    fn contains_is_exclusive_of_end() {
        let s = Selection::new(p(0, 1), p(0, 4)); // covers cols 1,2,3
        assert!(!s.contains(p(0, 0)));
        assert!(s.contains(p(0, 1)));
        assert!(s.contains(p(0, 3)));
        assert!(!s.contains(p(0, 4))); // end boundary excluded
    }

    #[test]
    fn extend_moves_head_keeps_anchor() {
        let s = Selection::caret(p(0, 2)).extended_to(p(0, 6));
        assert_eq!(s.anchor, p(0, 2));
        assert_eq!(s.head, p(0, 6));
        assert!(!s.is_empty());
    }

    #[test]
    fn collapse_edges_pick_the_right_side() {
        let backward = Selection::new(p(0, 5), p(0, 1));
        assert_eq!(backward.collapsed_to_head().head, p(0, 1));
        assert_eq!(backward.collapsed_to_start().head, p(0, 1));
        assert_eq!(backward.collapsed_to_end().head, p(0, 5));
    }

    #[test]
    fn set_merges_overlapping_selections() {
        let mut set = SelectionSet::single(Selection::new(p(0, 0), p(0, 5)));
        set.add(Selection::new(p(0, 3), p(0, 9))); // overlaps → merge
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().range(), (p(0, 0), p(0, 9)));
    }

    #[test]
    fn set_keeps_disjoint_selections_sorted() {
        let mut set = SelectionSet::single(Selection::caret(p(5, 0)));
        set.add(Selection::caret(p(1, 0)));
        set.add(Selection::caret(p(3, 0)));
        let starts: Vec<_> = set.all().iter().map(|s| s.start()).collect();
        assert_eq!(starts, vec![p(1, 0), p(3, 0), p(5, 0)]);
    }

    #[test]
    fn adjacent_carets_merge_to_one() {
        let mut set = SelectionSet::single(Selection::caret(p(2, 2)));
        set.add(Selection::caret(p(2, 2))); // same spot
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn collapse_to_primary_discards_others() {
        let mut set = SelectionSet::single(Selection::caret(p(0, 0)));
        set.add(Selection::caret(p(4, 0)));
        assert_eq!(set.len(), 2);
        set.collapse_to_primary_caret();
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().head, p(4, 0)); // the added one was primary
    }

    #[test]
    fn map_applies_to_every_selection() {
        let mut set = SelectionSet::single(Selection::caret(p(0, 0)));
        set.add(Selection::caret(p(2, 0)));
        // Extend every caret one column right.
        set.map(|s| {
            let h = Position::new(s.head.row, s.head.col + 3);
            s.extended_to(h)
        });
        for s in set.all() {
            assert_eq!(s.head.col, s.anchor.col + 3);
        }
    }

    #[test]
    fn primary_follows_its_head_through_normalisation() {
        let mut set = SelectionSet::single(Selection::caret(p(9, 0)));
        set.add(Selection::caret(p(1, 0))); // primary now the (1,0) one
        assert_eq!(set.primary().head, p(1, 0));
        // Adding an earlier disjoint selection re-sorts but primary stays put.
        set.selections_push_for_test(Selection::caret(p(0, 0)));
        assert_eq!(set.primary().head, p(1, 0));
    }
}

#[cfg(test)]
impl SelectionSet {
    /// Test-only: push without changing which head is primary, then normalise.
    fn selections_push_for_test(&mut self, sel: Selection) {
        let keep = self.primary().head;
        self.selections.push(sel);
        // Restore primary to the kept head after re-sort.
        self.normalise_keeping_primary();
        self.primary = self
            .selections
            .iter()
            .position(|s| s.head == keep)
            .unwrap_or(self.primary);
    }
}
