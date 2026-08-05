//! Central edit/delta types — the artifact every downstream consumer (undo,
//! LSP sync, incremental parse) consumes instead of re-deriving from full
//! text. Phase 1 of the CORE-DESIGN document rewrite: the types plus the
//! atomic apply. The rope/line-index migration is a later phase, so offsets
//! here are absolute CHAR offsets (the buffer's native column unit), not
//! bytes.

/// One replacement: the char range `[start, start + old.chars().count())`
/// becomes `new`. `old` is the text that range held — captured on apply, so
/// a change carries its own inverse description and undo needs no snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Absolute char offset from document start.
    pub start: usize,
    /// The text that was replaced ("" = pure insert).
    pub old: String,
    /// The replacement ("" = pure delete).
    pub new: String,
}

impl Change {
    pub fn insert(start: usize, new: impl Into<String>) -> Self {
        Self {
            start,
            old: String::new(),
            new: new.into(),
        }
    }

    pub fn delete(start: usize, old: impl Into<String>) -> Self {
        Self {
            start,
            old: old.into(),
            new: String::new(),
        }
    }

    pub fn replace(start: usize, old: impl Into<String>, new: impl Into<String>) -> Self {
        Self {
            start,
            old: old.into(),
            new: new.into(),
        }
    }

    pub fn old_len(&self) -> usize {
        self.old.chars().count()
    }

    pub fn new_len(&self) -> usize {
        self.new.chars().count()
    }

    /// The change that reverses this one (new ↔ old, same start — valid in
    /// the document version this change PRODUCED).
    pub fn inverse(&self) -> Change {
        Change {
            start: self.start,
            old: self.new.clone(),
            new: self.old.clone(),
        }
    }
}

/// An atomic group of changes against one document version. Changes carry
/// offsets in that version; `Buffer::apply_edit` applies them back-to-front
/// so earlier offsets stay valid. Ascending, non-overlapping order is the
/// convention `Delta` upholds.
#[derive(Clone, Debug, Default)]
pub struct Edit {
    pub changes: Vec<Change>,
}

impl Edit {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn single(change: Change) -> Self {
        Self {
            changes: vec![change],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// What changed between two document versions — what undo records, LSP
/// would sync, and an incremental parser would consume. Changes are in
/// ascending document order, offsets relative to `version_before`. The
/// cursors let undo/redo restore the caret without a snapshot.
#[derive(Clone, Debug)]
pub struct Delta {
    pub version_before: u64,
    pub version_after: u64,
    pub changes: Vec<Change>,
    pub cursor_before: crate::buffer::Position,
    pub cursor_after: crate::buffer::Position,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The edit that undoes this delta: each change inverted and remapped
    /// into the AFTER document's offsets, ordered back-to-front so the
    /// result applies as a valid `Edit` against `version_after`.
    pub fn inverse(&self) -> Edit {
        let mut out = Vec::with_capacity(self.changes.len());
        // Accumulated length shift of the changes BEFORE this one: an
        // inverse change sits at its original start plus the net length
        // delta of everything earlier in the document.
        let mut shift: isize = 0;
        for c in &self.changes {
            out.push(Change {
                start: (c.start as isize + shift) as usize,
                old: c.new.clone(),
                new: c.old.clone(),
            });
            shift += c.new_len() as isize - c.old_len() as isize;
        }
        out.reverse();
        Edit { changes: out }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    fn text_of(b: &Buffer) -> String {
        b.text()
    }

    #[test]
    fn offset_position_roundtrip() {
        let mut b = Buffer::from_string("ab\ncde\nf");
        // Document offsets: a0 b1 \n2 c3 d4 e5 \n6 f7
        assert_eq!(
            b.offset_to_position(0),
            crate::buffer::Position { row: 0, col: 0 }
        );
        assert_eq!(
            b.offset_to_position(2),
            crate::buffer::Position { row: 0, col: 2 }
        );
        assert_eq!(
            b.offset_to_position(3),
            crate::buffer::Position { row: 1, col: 0 }
        );
        assert_eq!(
            b.offset_to_position(7),
            crate::buffer::Position { row: 2, col: 0 }
        );
        for off in 0..=b.len_chars() {
            let p = b.offset_to_position(off);
            assert_eq!(b.position_to_offset(p), off, "roundtrip at {off}");
        }
        b.insert_str("한글"); // at (0,0): line0 = "한글ab"
        // Char offsets, not bytes: "한글" is 2 chars (6 bytes); offset 2 is
        // right after it, still on row 0.
        let off = b.position_to_offset(crate::buffer::Position { row: 0, col: 2 });
        assert_eq!(off, 2);
        assert_eq!(
            b.offset_to_position(off),
            crate::buffer::Position { row: 0, col: 2 }
        );
    }

    #[test]
    fn apply_edit_insert_delete_replace() {
        let mut b = Buffer::from_string("hello world");
        let v0 = b.version();
        let d = b.apply_edit(&Edit::single(Change::insert(5, " cruel")));
        assert_eq!(text_of(&b), "hello cruel world");
        assert_eq!(d.version_before, v0);
        assert!(d.version_after > v0, "one version bump");
        assert_eq!(d.changes[0].old, "");
        assert_eq!(d.changes[0].new, " cruel");

        let d2 = b.apply_edit(&Edit::single(Change::delete(5, " cruel")));
        assert_eq!(text_of(&b), "hello world");
        assert_eq!(d2.changes[0].old, " cruel");

        let d3 = b.apply_edit(&Edit::single(Change::replace(0, "hello", "goodbye")));
        assert_eq!(text_of(&b), "goodbye world");
        assert_eq!(d3.changes[0].old, "hello");
        assert_eq!(d3.changes[0].new, "goodbye");
    }

    #[test]
    fn multi_change_edit_applies_back_to_front() {
        let mut b = Buffer::from_string("abcdef");
        // Two inserts against the SAME version's offsets — the later offset
        // must not be disturbed by the earlier insert.
        let edit = Edit {
            changes: vec![Change::insert(1, "1"), Change::insert(4, "4")],
        };
        let d = b.apply_edit(&edit);
        assert_eq!(text_of(&b), "a1bcd4ef");
        assert_eq!(d.changes.len(), 2);
        assert_eq!(d.changes[0].start, 1, "delta stays in ascending order");
        assert_eq!(d.changes[1].start, 4);
    }

    #[test]
    fn delta_inverse_restores_the_text() {
        let mut b = Buffer::from_string("the quick fox");
        let original = text_of(&b);
        let d = b.apply_edit(&Edit {
            changes: vec![
                Change::replace(4, "quick", "slow"),
                Change::insert(13, " jumps"),
            ],
        });
        assert_eq!(text_of(&b), "the slow fox jumps");
        b.apply_edit(&d.inverse());
        assert_eq!(text_of(&b), original, "inverse restores exactly");
    }

    #[test]
    fn empty_edit_does_not_bump_the_version() {
        let mut b = Buffer::from_string("x");
        let v = b.version();
        let d = b.apply_edit(&Edit::new());
        assert!(d.is_empty());
        assert_eq!(b.version(), v, "no bump without changes");
    }

    #[test]
    fn multiline_change_roundtrips() {
        let mut b = Buffer::from_string("line1\nline2\nline3");
        let original = text_of(&b);
        // Replace "line2" (offset 6..11) with two lines.
        let d = b.apply_edit(&Edit::single(Change::replace(6, "line2", "a\nb")));
        assert_eq!(text_of(&b), "line1\na\nb\nline3");
        b.apply_edit(&d.inverse());
        assert_eq!(text_of(&b), original);
    }
}
