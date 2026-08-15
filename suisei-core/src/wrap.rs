//! Soft wrap: how many rows a document occupies on screen, and which row is
//! which.
//!
//! # Why this is arithmetic and not layout
//!
//! Wrapping normally means asking the text engine where a line can break at a
//! given pixel width — `CTTypesetterSuggestLineBreak` and friends — because in
//! a proportional font the answer depends on the glyphs. Suisei's editor is
//! monospace by construction ([`crate::app`]'s metrics resolve JetBrains Mono
//! or the system fixed-pitch face), so every cell is one width and a break is
//! a division. That is what makes a whole-document wrap map affordable: no
//! shaping, no font, no per-line typesetter — one integer per line.
//!
//! The face still owns the pixel side. It knows the pane width, the cell width
//! and the gutter, so it decides **how many columns fit**; this decides what
//! that means for the document.
//!
//! # What the face needs, and why a prefix sum
//!
//! Three questions, all of them per-frame:
//!
//! * how tall is the document (the scroll extent);
//! * where does buffer row *N* start on screen (drawing, the caret, a jump);
//! * which buffer row is at screen row *V* (a click, the top of the viewport).
//!
//! Without wrapping all three are multiplication. With it they are a running
//! total over every line above, which is O(document) per question — so the
//! total is computed once into [`WrapMap::starts`] and the questions become a
//! lookup and a binary search.
//!
//! The map is rebuilt when the document changes or the width changes, and not
//! otherwise; see [`WrapMap::is_valid_for`].

/// One line's visual rows, and where they begin.
///
/// `starts[i]` is the first visual row of buffer row `i`; `starts[len]` is the
/// document's total height, which is why the vector is one longer than the
/// document. Storing the starts rather than the counts is what makes
/// [`WrapMap::visual_of`] a lookup instead of a sum.
#[derive(Clone, Debug, Default)]
pub struct WrapMap {
    starts: Vec<u32>,
    /// Columns this was built for. `0` means "not wrapping" — every line is
    /// one row, and the map is the identity.
    cols: u16,
    /// [`crate::buffer::Buffer::version`] this was built from.
    version: u64,
    tab_width: u16,
}

impl WrapMap {
    /// Whether this map still describes the document at this width.
    ///
    /// All three matter. The version alone misses a pane being resized; the
    /// width alone misses an edit; and the tab width changes how wide a line
    /// is without changing a byte of it.
    pub fn is_valid_for(&self, version: u64, cols: u16, tab_width: u16) -> bool {
        !self.starts.is_empty()
            && self.version == version
            && self.cols == cols
            && self.tab_width == tab_width
    }

    /// Build the map for `lines` at `cols` columns.
    ///
    /// `cols == 0` disables wrapping: every line is one row. That is the same
    /// shape as the wrapped map rather than a separate mode, so nothing above
    /// has to branch on whether wrapping is on.
    pub fn build(lines: &[String], version: u64, cols: u16, tab_width: u16) -> Self {
        let tab = tab_width.max(1) as usize;
        let mut starts = Vec::with_capacity(lines.len() + 1);
        let mut at = 0u32;
        for line in lines {
            starts.push(at);
            at = at.saturating_add(rows_for(line, cols, tab) as u32);
        }
        starts.push(at);
        Self {
            starts,
            cols,
            version,
            tab_width,
        }
    }

    /// Total visual rows in the document. Always at least 1: an empty document
    /// is one empty row, the one the caret sits on.
    pub fn total_rows(&self) -> u32 {
        self.starts.last().copied().unwrap_or(0).max(1)
    }

    pub fn line_count(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    /// First visual row of a buffer row. Clamped, so a stale row index from a
    /// face that has not caught up yet lands at the end rather than panicking.
    pub fn visual_of(&self, buffer_row: usize) -> u32 {
        match self.starts.get(buffer_row) {
            Some(v) => *v,
            None => self.starts.last().copied().unwrap_or(0),
        }
    }

    /// How many visual rows a buffer row takes.
    pub fn rows_of(&self, buffer_row: usize) -> u32 {
        let a = self.visual_of(buffer_row);
        let b = self
            .starts
            .get(buffer_row + 1)
            .copied()
            .unwrap_or_else(|| self.starts.last().copied().unwrap_or(0));
        b.saturating_sub(a).max(1)
    }

    /// The buffer row drawn at a visual row, and which segment of it.
    ///
    /// Binary search rather than a second array indexed by visual row: that
    /// array would be as long as the *wrapped* document, which at a narrow
    /// width is several times the line count, and it would answer the same
    /// question this does in the same asymptotic time the face cares about
    /// (once per frame, not once per row).
    pub fn buffer_at(&self, visual_row: u32) -> (usize, u32) {
        if self.starts.len() < 2 {
            return (0, 0);
        }
        // `partition_point` gives the first index whose start is > the target;
        // the row we want is the one before it.
        let idx = self.starts.partition_point(|&s| s <= visual_row);
        let row = idx.saturating_sub(1).min(self.line_count().saturating_sub(1));
        // The SEGMENT clamps too, not just the row. Past the end of the
        // document `partition_point` lands on the sentinel and the subtraction
        // below measures from the last line's start — a visual row 999 in a
        // two-line document reported segment 998 of line 1. Both halves of the
        // answer have to be a place that exists.
        let segment = visual_row
            .saturating_sub(self.visual_of(row))
            .min(self.rows_of(row).saturating_sub(1));
        (row, segment)
    }

    /// Column range of one segment of a line: `[first, last)` in display
    /// columns. Unwrapped, or the last segment, extends to the line's end.
    pub fn segment_columns(&self, segment: u32) -> (usize, usize) {
        if self.cols == 0 {
            return (0, usize::MAX);
        }
        let w = self.cols as usize;
        let first = segment as usize * w;
        (first, first + w)
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }
}

/// Visual rows one line occupies at `cols` columns.
///
/// An empty line is one row, not zero — it is a row you can put the caret on.
/// A line exactly `cols` wide is also one row: the break belongs *after* the
/// last cell that fits, and a trailing empty segment is a blank row the
/// document does not contain.
fn rows_for(line: &str, cols: u16, tab_width: usize) -> usize {
    if cols == 0 {
        return 1;
    }
    let w = display_columns(line, tab_width);
    if w == 0 {
        return 1;
    }
    w.div_ceil(cols as usize)
}

/// Display columns a line occupies: tabs advance to the next stop, wide glyphs
/// (CJK, emoji) take two cells.
///
/// The same rule [`crate::app`] measures content width with. Two copies of this
/// would let the wrap map and the horizontal extent disagree about where a line
/// ends, which is exactly the kind of pair that goes wrong quietly.
pub fn display_columns(line: &str, tab_width: usize) -> usize {
    use unicode_width::UnicodeWidthChar;
    let tab = tab_width.max(1);
    let mut col = 0usize;
    for ch in line.chars() {
        col += if ch == '\t' {
            tab - (col % tab)
        } else {
            ch.width().unwrap_or(0)
        };
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    /// `cols == 0` is the identity: the face asks for this when wrapping is
    /// off, so everything above can use one code path.
    #[test]
    fn no_wrap_is_one_row_per_line() {
        let d = doc(&["", "short", &"x".repeat(500)]);
        let m = WrapMap::build(&d, 1, 0, 4);
        assert_eq!(m.total_rows(), 3);
        for row in 0..3 {
            assert_eq!(m.rows_of(row), 1);
            assert_eq!(m.visual_of(row), row as u32);
            assert_eq!(m.buffer_at(row as u32), (row, 0));
        }
    }

    /// An empty line is a row you can put the caret on, not zero rows.
    #[test]
    fn an_empty_line_still_occupies_a_row() {
        let m = WrapMap::build(&doc(&["", "", ""]), 1, 10, 4);
        assert_eq!(m.total_rows(), 3);
        assert_eq!(m.visual_of(2), 2);
    }

    /// A line exactly as wide as the pane is ONE row. The break goes after the
    /// last cell that fits; a trailing empty segment would be a blank row the
    /// document does not contain.
    #[test]
    fn an_exact_fit_does_not_spill_a_blank_row() {
        let m = WrapMap::build(&doc(&["abcdefghij"]), 1, 10, 4);
        assert_eq!(m.rows_of(0), 1);
        assert_eq!(m.total_rows(), 1);

        let m = WrapMap::build(&doc(&["abcdefghijk"]), 1, 10, 4);
        assert_eq!(m.rows_of(0), 2, "one cell over is two rows");
    }

    /// The prefix sum and its inverse have to agree, at every row and at every
    /// segment — this is the pair the whole map exists to keep honest.
    #[test]
    fn visual_and_buffer_are_inverses() {
        let d = doc(&["a", &"b".repeat(25), "", &"c".repeat(10), "d"]);
        let m = WrapMap::build(&d, 1, 10, 4);
        // 1 + 3 + 1 + 1 + 1
        assert_eq!(m.total_rows(), 7);
        assert_eq!(m.visual_of(0), 0);
        assert_eq!(m.visual_of(1), 1);
        assert_eq!(m.visual_of(2), 4);
        assert_eq!(m.visual_of(3), 5);
        assert_eq!(m.visual_of(4), 6);

        for row in 0..d.len() {
            let first = m.visual_of(row);
            for seg in 0..m.rows_of(row) {
                assert_eq!(
                    m.buffer_at(first + seg),
                    (row, seg),
                    "row {row} segment {seg}"
                );
            }
        }
    }

    /// A tab is a stop, not a character — a line of four tabs at width 4 is
    /// sixteen columns and wraps at ten.
    #[test]
    fn tabs_are_measured_as_stops() {
        assert_eq!(display_columns("\t", 4), 4);
        assert_eq!(display_columns("a\t", 4), 4, "advance TO the stop");
        assert_eq!(display_columns("\t\t\t\t", 4), 16);
        let m = WrapMap::build(&doc(&["\t\t\t\t"]), 1, 10, 4);
        assert_eq!(m.rows_of(0), 2);
    }

    /// Hangul and CJK take two cells, so a line of them wraps at half the
    /// character count. Getting this wrong is the difference between a wrap
    /// point and a clipped glyph.
    #[test]
    fn wide_glyphs_take_two_cells() {
        assert_eq!(display_columns("한글", 4), 4);
        let m = WrapMap::build(&doc(&["한글한글한글"]), 1, 10, 4);
        assert_eq!(m.rows_of(0), 2, "12 columns at width 10");
    }

    /// The tab width is part of the key: the same bytes are a different number
    /// of rows at a different tab stop, and nothing else would notice.
    #[test]
    fn validity_covers_width_version_and_tab_stop() {
        let m = WrapMap::build(&doc(&["\t\ta"]), 7, 10, 4);
        assert!(m.is_valid_for(7, 10, 4));
        assert!(!m.is_valid_for(8, 10, 4), "an edit");
        assert!(!m.is_valid_for(7, 12, 4), "a resize");
        assert!(!m.is_valid_for(7, 10, 8), "a tab-width change");
        assert!(
            !WrapMap::default().is_valid_for(0, 0, 4),
            "a map that was never built describes nothing"
        );
    }

    /// A row index past the end clamps instead of panicking: the face pulls
    /// bands asynchronously and can ask about a row an edit has just removed.
    #[test]
    fn out_of_range_rows_clamp() {
        let m = WrapMap::build(&doc(&["a", "b"]), 1, 10, 4);
        assert_eq!(m.visual_of(99), m.total_rows());
        assert_eq!(m.rows_of(99), 1);
        assert_eq!(m.buffer_at(999), (1, 0));
    }

    /// Segments carve the line into equal column ranges, and the unwrapped map
    /// says "all of it".
    #[test]
    fn segments_carve_the_line_into_columns() {
        let m = WrapMap::build(&doc(&[&"x".repeat(25)]), 1, 10, 4);
        assert_eq!(m.segment_columns(0), (0, 10));
        assert_eq!(m.segment_columns(1), (10, 20));
        assert_eq!(m.segment_columns(2), (20, 30));
        let flat = WrapMap::build(&doc(&["x"]), 1, 0, 4);
        assert_eq!(flat.segment_columns(0), (0, usize::MAX));
    }

    /// An empty document is one row, because the caret is somewhere.
    #[test]
    fn an_empty_document_is_one_row() {
        assert_eq!(WrapMap::build(&[], 1, 10, 4).total_rows(), 1);
        assert_eq!(WrapMap::build(&doc(&[""]), 1, 10, 4).total_rows(), 1);
    }
}
