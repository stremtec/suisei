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
    wide_ratio: u16,
}

impl WrapMap {
    /// Whether this map still describes the document at this width.
    ///
    /// All three matter. The version alone misses a pane being resized; the
    /// width alone misses an edit; and the tab width changes how wide a line
    /// is without changing a byte of it.
    pub fn is_valid_for(
        &self,
        version: u64,
        cols: u16,
        tab_width: u16,
        wide_ratio: u16,
    ) -> bool {
        !self.starts.is_empty()
            && self.version == version
            && self.cols == cols
            && self.tab_width == tab_width
            && self.wide_ratio == wide_ratio
    }

    /// Build the map for `lines` at `cols` columns.
    ///
    /// `cols == 0` disables wrapping: every line is one row. That is the same
    /// shape as the wrapped map rather than a separate mode, so nothing above
    /// has to branch on whether wrapping is on.
    pub fn build(
        lines: &[String],
        version: u64,
        cols: u16,
        tab_width: u16,
        wide_ratio: u16,
    ) -> Self {
        let tab = tab_width.max(1) as usize;
        let mut starts = Vec::with_capacity(lines.len() + 1);
        let mut at = 0u32;
        for line in lines {
            starts.push(at);
            at = at.saturating_add(rows_for(line, cols, tab, wide_ratio) as u32);
        }
        starts.push(at);
        Self {
            starts,
            cols,
            version,
            tab_width,
            wide_ratio,
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
///
/// **A walk, not a division.** `ceil(width / cols)` is wrong wherever a glyph
/// is wider than one cell, because a double-width glyph that will not fit
/// starts the next row rather than being cut in half. Four Hangul syllables at
/// three columns are FOUR rows — two cells, break, two cells, break — and the
/// division says three. The renderer breaks greedily
/// ([`visual_chunks`]); a count that disagreed with it would put every row
/// below the line in the wrong place.
fn rows_for(line: &str, cols: u16, tab_width: usize, wide_ratio: u16) -> usize {
    if cols == 0 {
        return 1;
    }
    let width = cols as u32 * CELL;
    let tab = tab_width.max(1);
    let mut rows = 1usize;
    let mut row_w = 0u32;
    let mut col = 0usize;
    for ch in line.chars() {
        // A tab is cells, not a glyph: expanded it is N spaces, and a break
        // can land between any two of them. Walking it as one advance of N
        // would refuse to break inside a tab and overflow the row instead —
        // and the renderer, which chunks text that has already been expanded,
        // breaks inside it.
        let cells: usize = if ch == '\t' { tab - (col % tab) } else { 0 };
        if ch == '\t' {
            for _ in 0..cells {
                if row_w + CELL > width {
                    rows += 1;
                    row_w = 0;
                }
                row_w += CELL;
            }
            col += cells;
            continue;
        }
        let w = char_width(ch, wide_ratio);
        if row_w > 0 && row_w + w > width {
            rows += 1;
            row_w = 0;
        }
        row_w += w;
        col += char_cells(ch);
    }
    rows
}

fn char_cells(ch: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    ch.width().unwrap_or(0)
}

/// One narrow cell, in the hundredths this module measures wrapping in.
pub const CELL: u32 = 100;

/// How wide a "two-cell" glyph really paints, in hundredths of a narrow cell.
///
/// The cell model says CJK is two cells, and in a terminal that is true because
/// the terminal draws on a grid. The EDITOR does not: it lays text out with
/// real CoreText advances, which is why the caret is carried as a UTF-16 offset
/// rather than a cell column. Measured with the shipped font at size 12, `한`
/// advances 10.380pt against a narrow cell's 7.200 — **1.44 cells, not 2**.
///
/// Wrapping at two cells therefore budgeted 39% more width than a Hangul line
/// paints, and a Korean paragraph broke with a quarter of the pane still empty.
/// The face measures the real ratio for the font in use and pushes it down;
/// this default is the terminal-true value for anything that never does.
pub const WIDE_TWO_CELLS: u16 = 200;

/// Width of one character in hundredths of a cell.
fn char_width(ch: char, wide_ratio: u16) -> u32 {
    match char_cells(ch) {
        0 => 0,
        1 => CELL,
        _ => u32::from(wide_ratio.max(1)),
    }
}

/// Tabs as the spaces they paint as.
///
/// The renderer works in expanded coordinates — syntax spans, the caret column
/// and the selection are all reported against this string — so wrapping has to
/// see the same text. Lived in the engine's scene builder with the tab width
/// hardcoded to 4, which is its own quiet bug: a document set to eight-column
/// tabs was measured at four everywhere the expansion was used.
pub fn expand_tabs(s: &str, tab_width: usize) -> String {
    let tab = tab_width.max(1);
    let mut out = String::with_capacity(s.len());
    let mut col = 0usize;
    for ch in s.chars() {
        if ch == '\t' {
            let n = tab - (col % tab);
            for _ in 0..n {
                out.push(' ');
            }
            col += n;
        } else {
            out.push(ch);
            col += char_cells(ch);
        }
    }
    out
}

/// Split already-expanded text into the rows it wraps to, each with the
/// display column it starts at.
///
/// The one place a line is broken. [`rows_for`] counts what this produces;
/// they are tested against each other, because a count and a split that
/// disagree put every row below the line somewhere it is not drawn.
pub fn visual_chunks(text: &str, cols: u16, wide_ratio: u16) -> Vec<(u32, String)> {
    if cols == 0 {
        return vec![(0, text.to_string())];
    }
    let width = cols as u32 * CELL;
    let mut out = Vec::new();
    let mut col: u32 = 0;
    let mut seg_start_col: u32 = 0;
    let mut seg = String::new();
    let mut seg_w = 0u32;
    for ch in text.chars() {
        let w = char_width(ch, wide_ratio).max(CELL);
        if seg_w > 0 && seg_w + w > width {
            out.push((seg_start_col, std::mem::take(&mut seg)));
            seg_start_col = col;
            seg_w = 0;
        }
        seg.push(ch);
        seg_w += w;
        // The reported column stays CELLS: `base_col` rebases syntax spans and
        // the caret, which are cell coordinates. Only the break decision is
        // measured in hundredths.
        col += char_cells(ch).max(1) as u32;
    }
    if !seg.is_empty() || out.is_empty() {
        out.push((seg_start_col, seg));
    }
    out
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
        let m = WrapMap::build(&d, 1, 0, 4, WIDE_TWO_CELLS);
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
        let m = WrapMap::build(&doc(&["", "", ""]), 1, 10, 4, WIDE_TWO_CELLS);
        assert_eq!(m.total_rows(), 3);
        assert_eq!(m.visual_of(2), 2);
    }

    /// A line exactly as wide as the pane is ONE row. The break goes after the
    /// last cell that fits; a trailing empty segment would be a blank row the
    /// document does not contain.
    #[test]
    fn an_exact_fit_does_not_spill_a_blank_row() {
        let m = WrapMap::build(&doc(&["abcdefghij"]), 1, 10, 4, WIDE_TWO_CELLS);
        assert_eq!(m.rows_of(0), 1);
        assert_eq!(m.total_rows(), 1);

        let m = WrapMap::build(&doc(&["abcdefghijk"]), 1, 10, 4, WIDE_TWO_CELLS);
        assert_eq!(m.rows_of(0), 2, "one cell over is two rows");
    }

    /// The prefix sum and its inverse have to agree, at every row and at every
    /// segment — this is the pair the whole map exists to keep honest.
    #[test]
    fn visual_and_buffer_are_inverses() {
        let d = doc(&["a", &"b".repeat(25), "", &"c".repeat(10), "d"]);
        let m = WrapMap::build(&d, 1, 10, 4, WIDE_TWO_CELLS);
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
        let m = WrapMap::build(&doc(&["\t\t\t\t"]), 1, 10, 4, WIDE_TWO_CELLS);
        assert_eq!(m.rows_of(0), 2);
    }

    /// Hangul and CJK take two cells, so a line of them wraps at half the
    /// character count. Getting this wrong is the difference between a wrap
    /// point and a clipped glyph.
    #[test]
    fn wide_glyphs_take_two_cells() {
        assert_eq!(display_columns("한글", 4), 4);
        let m = WrapMap::build(&doc(&["한글한글한글"]), 1, 10, 4, WIDE_TWO_CELLS);
        assert_eq!(m.rows_of(0), 2, "12 columns at width 10");
    }

    /// The tab width is part of the key: the same bytes are a different number
    /// of rows at a different tab stop, and nothing else would notice.
    #[test]
    fn validity_covers_width_version_and_tab_stop() {
        let m = WrapMap::build(&doc(&["\t\ta"]), 7, 10, 4, WIDE_TWO_CELLS);
        assert!(m.is_valid_for(7, 10, 4, WIDE_TWO_CELLS));
        assert!(!m.is_valid_for(8, 10, 4, WIDE_TWO_CELLS), "an edit");
        assert!(!m.is_valid_for(7, 12, 4, WIDE_TWO_CELLS), "a resize");
        assert!(!m.is_valid_for(7, 10, 8, WIDE_TWO_CELLS), "a tab-width change");
        assert!(
            !WrapMap::default().is_valid_for(0, 0, 4, WIDE_TWO_CELLS),
            "a map that was never built describes nothing"
        );
    }

    /// A row index past the end clamps instead of panicking: the face pulls
    /// bands asynchronously and can ask about a row an edit has just removed.
    #[test]
    fn out_of_range_rows_clamp() {
        let m = WrapMap::build(&doc(&["a", "b"]), 1, 10, 4, WIDE_TWO_CELLS);
        assert_eq!(m.visual_of(99), m.total_rows());
        assert_eq!(m.rows_of(99), 1);
        assert_eq!(m.buffer_at(999), (1, 0));
    }

    /// Segments carve the line into equal column ranges, and the unwrapped map
    /// says "all of it".
    #[test]
    fn segments_carve_the_line_into_columns() {
        let m = WrapMap::build(&doc(&[&"x".repeat(25)]), 1, 10, 4, WIDE_TWO_CELLS);
        assert_eq!(m.segment_columns(0), (0, 10));
        assert_eq!(m.segment_columns(1), (10, 20));
        assert_eq!(m.segment_columns(2), (20, 30));
        let flat = WrapMap::build(&doc(&["x"]), 1, 0, 4, WIDE_TWO_CELLS);
        assert_eq!(flat.segment_columns(0), (0, usize::MAX));
    }

    /// **The count and the split must agree, always.**
    ///
    /// `WrapMap` says how tall a line is; `visual_chunks` says what its rows
    /// contain. They are separate walks over the same rule, and if they ever
    /// disagree the document is a different height than it draws — every row
    /// below the line lands where nothing is painted, and every click below it
    /// resolves to the wrong line. This is the test that keeps them one rule.
    ///
    /// The cases are the ones a division gets wrong: wide glyphs that cannot
    /// straddle a boundary, and tabs, which expand to cells that can.
    #[test]
    fn the_row_count_and_the_split_agree() {
        let cases: &[&str] = &[
            "",
            "a",
            "abcdefghij",
            "abcdefghijk",
            &"x".repeat(97),
            "한한한한",
            "한한한한한",
            "a한한",
            "한a한a",
            "\t",
            "\t\t\t\t",
            "\tabc\tdef",
            "a\t한\tb",
            "  \t한글 코드 \t끝",
        ];
        for tab in [2usize, 4, 8] {
            for cols in 1u16..=20 {
                for raw in cases {
                    let counted = rows_for(raw, cols, tab, WIDE_TWO_CELLS);
                    let split = visual_chunks(&expand_tabs(raw, tab), cols, WIDE_TWO_CELLS).len();
                    assert_eq!(
                        counted, split,
                        "line {raw:?} at {cols} cols, tab {tab}: \
                         counted {counted} rows, split into {split}"
                    );
                }
            }
        }
    }

    /// The case that made this one rule instead of two: four double-width
    /// glyphs at three columns are four rows, and `ceil(8 / 3)` is three.
    #[test]
    fn a_wide_glyph_starts_a_row_rather_than_being_cut() {
        assert_eq!(display_columns("한한한한", 4), 8);
        assert_eq!(rows_for("한한한한", 3, 4, WIDE_TWO_CELLS), 4);
        assert_eq!(visual_chunks("한한한한", 3, WIDE_TWO_CELLS).len(), 4);
        assert_ne!(8usize.div_ceil(3), 4, "the division this replaced");
    }

    /// Tabs expand before they wrap, so a break can land inside one.
    #[test]
    fn a_tab_can_be_broken_across_rows() {
        // One tab at width 8 is eight cells; at three columns that is three
        // rows, which only happens if the tab is cells rather than a glyph.
        assert_eq!(expand_tabs("\t", 8), " ".repeat(8));
        assert_eq!(rows_for("\t", 3, 8, WIDE_TWO_CELLS), 3);
        assert_eq!(visual_chunks(&expand_tabs("\t", 8), 3, WIDE_TWO_CELLS).len(), 3);
    }

    /// The tab width reaches the expansion. It used to be hardcoded to 4 in the
    /// engine's copy, so an eight-column document was measured at four.
    #[test]
    fn the_tab_width_is_honoured() {
        assert_eq!(expand_tabs("\ta", 2), "  a");
        assert_eq!(expand_tabs("\ta", 8), "        a");
        assert_eq!(display_columns("\ta", 8), 9);
    }

    /// A wide glyph is as wide as it PAINTS, not as wide as the cell model
    /// says.
    ///
    /// The editor lays text out with real CoreText advances — that is why the
    /// caret crosses as a UTF-16 offset and not a cell column — and with the
    /// shipped font `한` advances 1.44 narrow cells, not 2. Budgeting it at 2
    /// gave a Korean paragraph 39% more width than it paints, so it broke with
    /// a quarter of the pane still empty.
    ///
    /// The columns REPORTED stay cells, because `base_col` rebases syntax
    /// spans and the caret and those are cell coordinates. Only the break
    /// decision is measured in hundredths.
    #[test]
    fn a_wide_glyph_is_budgeted_at_what_it_paints() {
        // Ten columns of budget. At two cells each, five syllables fill it.
        assert_eq!(rows_for("한한한한한", 10, 4, 200), 1);
        assert_eq!(rows_for("한한한한한한", 10, 4, 200), 2);

        // At the measured 1.44, six fit in the same ten columns — 6 × 144 =
        // 864 of the 1000 hundredths, and a seventh would be 1008.
        assert_eq!(rows_for("한한한한한한", 10, 4, 144), 1);
        assert_eq!(rows_for("한한한한한한한", 10, 4, 144), 2);

        // And the split agrees with the count at the measured ratio too —
        // the property the whole module rests on, not just at the default.
        for wide in [100u16, 144, 175, 200, 260] {
            for cols in 1u16..=16 {
                for raw in ["한한한한한한한한", "a한b한c한", "한a", "\t한\t한"] {
                    assert_eq!(
                        rows_for(raw, cols, 4, wide),
                        visual_chunks(&expand_tabs(raw, 4), cols, wide).len(),
                        "{raw:?} at {cols} cols, wide {wide}"
                    );
                }
            }
        }
    }

    /// The reported column of each chunk stays in CELLS whatever the ratio —
    /// spans and the caret are rebased against it.
    #[test]
    fn chunk_columns_are_cells_not_hundredths() {
        let narrow = visual_chunks("한한한한", 2, 200);
        assert_eq!(narrow.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 2, 4, 6]);
        // A different ratio changes WHERE it breaks, never the units.
        let wide = visual_chunks("한한한한", 4, 144);
        assert_eq!(wide[0].0, 0);
        assert!(wide.iter().all(|c| c.0 % 2 == 0), "cell columns: {wide:?}");
    }

    /// An empty document is one row, because the caret is somewhere.
    #[test]
    fn an_empty_document_is_one_row() {
        assert_eq!(WrapMap::build(&[], 1, 10, 4, WIDE_TWO_CELLS).total_rows(), 1);
        assert_eq!(WrapMap::build(&doc(&[""]), 1, 10, 4, WIDE_TWO_CELLS).total_rows(), 1);
    }
}
