//! Incremental search — the find bar's model (A3-1 extraction).
//!
//! State and pure computation live here; the orchestration that touches the
//! buffer, scroll, jumps and status messages stays on `App` as thin wrappers.
//! `dispatch` and the compositor speak only to this struct's API — nobody
//! pokes the fields from the outside anymore (the key handler used to
//! re-implement match cycling inline; that copy is gone).

use crate::buffer::Position;

/// The find bar's whole state: committed pattern, live input, matches, and
/// the origins Esc restores.
#[derive(Debug, Clone)]
pub struct SearchState {
    /// Committed pattern — survives leaving Search mode, drives `n`/`N` and
    /// the persistent highlight.
    pub pattern: Option<String>,
    /// Live query while in Search mode (does not touch `pattern` until
    /// commit).
    pub input: String,
    /// Match positions in row order — [`SearchState::collect`] builds them
    /// sorted, so [`SearchState::row_slice`] binary-searches.
    pub matches: Vec<Position>,
    /// Index into `matches` of the match the caret sits on.
    pub current: usize,
    /// Cursor when the bar opened — Esc puts it back.
    pub origin: Option<Position>,
    /// Scroll when the bar opened — Esc puts it back.
    pub scroll_origin: usize,
    /// Pattern in force before this search session (restored on cancel).
    pattern_backup: Option<String>,
    /// `true` = forward `/`, `false` = reverse `?`. `n` follows this, `N`
    /// opposes it.
    pub forward: bool,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            pattern: None,
            input: String::new(),
            matches: Vec::new(),
            current: 0,
            origin: None,
            scroll_origin: 0,
            pattern_backup: None,
            forward: true,
        }
    }
}

impl SearchState {
    /// Open the find bar.
    pub fn begin(&mut self, forward: bool, cursor: Position, scroll: usize) {
        self.forward = forward;
        self.origin = Some(cursor);
        self.scroll_origin = scroll;
        self.pattern_backup = self.pattern.clone();
        self.input.clear();
    }

    /// Leave the bar, keeping the committed pattern (Enter).
    pub fn finish(&mut self) {
        self.input.clear();
        self.origin = None;
        self.pattern_backup = None;
    }

    /// Esc: give back the origins and the pre-search pattern. Returns them so
    /// the caller can restore the caret and rebuild matches for `n`/`N`
    /// without moving it.
    pub fn cancel(&mut self) -> (Option<Position>, usize, Option<String>) {
        let origin = self.origin.take();
        let scroll = self.scroll_origin;
        self.input.clear();
        self.pattern = self.pattern_backup.take();
        self.matches.clear();
        self.current = 0;
        (origin, scroll, self.pattern.clone())
    }

    /// Pattern used for highlighting — live input while in the bar, the
    /// committed pattern elsewhere.
    pub fn active_pattern(&self, in_search_mode: bool) -> Option<&str> {
        if in_search_mode {
            if self.input.is_empty() {
                None
            } else {
                Some(self.input.as_str())
            }
        } else {
            self.pattern.as_deref()
        }
    }

    pub fn pattern_len_chars(&self, in_search_mode: bool) -> usize {
        self.active_pattern(in_search_mode)
            .map(|p| p.chars().count())
            .unwrap_or(0)
    }

    /// Matches on `row` plus the global index of the first one, so callers
    /// can keep comparing against [`current`](Self::current). Binary search
    /// — `matches` is row-ordered.
    pub fn row_slice(&self, row: usize) -> (usize, &[Position]) {
        let lo = self.matches.partition_point(|p| p.row < row);
        let hi = self.matches.partition_point(|p| p.row <= row);
        (lo, &self.matches[lo..hi])
    }

    pub fn is_current_match(&self, row: usize, col: usize) -> bool {
        self.matches
            .get(self.current)
            .map(|p| p.row == row && p.col == col)
            .unwrap_or(false)
    }

    /// Index of the match a fresh search from `from` lands on: the first at
    /// or after it (forward) / the last at or before it (reverse), wrapping
    /// to the far end when nothing qualifies. `None` when there are no
    /// matches at all.
    pub fn nearest(&self, from: Position, forward: bool) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        if forward {
            Some(
                self.matches
                    .iter()
                    .position(|p| p.row > from.row || (p.row == from.row && p.col >= from.col))
                    .unwrap_or(0),
            )
        } else {
            Some(
                self.matches
                    .iter()
                    .rposition(|p| p.row < from.row || (p.row == from.row && p.col <= from.col))
                    .unwrap_or(self.matches.len() - 1),
            )
        }
    }

    /// Index of the next match STRICTLY past `cur`, wrapping at the ends.
    /// The bool says whether the step wrapped — the face shows "search hit
    /// BOTTOM, continuing at TOP".
    pub fn step(&self, cur: Position, forward: bool) -> Option<(usize, bool)> {
        if self.matches.is_empty() {
            return None;
        }
        let idx = if forward {
            self.matches
                .iter()
                .position(|p| p.row > cur.row || (p.row == cur.row && p.col > cur.col))
                .unwrap_or(0)
        } else {
            self.matches
                .iter()
                .rposition(|p| p.row < cur.row || (p.row == cur.row && p.col < cur.col))
                .unwrap_or(self.matches.len() - 1)
        };
        let pos = self.matches[idx];
        let wrapped = if forward {
            idx == 0 && (pos.row < cur.row || (pos.row == cur.row && pos.col <= cur.col))
        } else {
            idx == self.matches.len() - 1
                && (pos.row > cur.row || (pos.row == cur.row && pos.col >= cur.col))
        };
        Some((idx, wrapped))
    }

    /// Cycle the live match while still typing (↑↓ in the find bar). Returns
    /// the caret position to jump to.
    pub fn cycle(&mut self, forward: bool) -> Option<Position> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = if forward {
            (self.current + 1) % self.matches.len()
        } else if self.current == 0 {
            self.matches.len() - 1
        } else {
            self.current - 1
        };
        Some(self.matches[self.current])
    }

    /// All matches of `pattern` in `lines`, row order. Smart case: an
    /// all-lowercase pattern matches case-insensitively; any uppercase makes
    /// it exact. Overlapping matches count (vim default).
    pub fn collect(lines: &[String], pattern: &str) -> Vec<Position> {
        let mut out = Vec::new();
        if pattern.is_empty() {
            return out;
        }
        let smart_case = !pattern.chars().any(|c| c.is_uppercase());
        let pat_lower = if smart_case {
            pattern.to_lowercase()
        } else {
            String::new()
        };

        for (row, line) in lines.iter().enumerate() {
            if smart_case {
                // Case-insensitive: walk char-by-char comparing lowered windows.
                let line_chars: Vec<char> = line.chars().collect();
                let pat_chars: Vec<char> = pat_lower.chars().collect();
                if pat_chars.is_empty() {
                    continue;
                }
                let plen = pat_chars.len();
                if line_chars.len() < plen {
                    continue;
                }
                let line_lower: Vec<char> = line_chars
                    .iter()
                    .map(|c| c.to_lowercase().next().unwrap_or(*c))
                    .collect();
                let mut i = 0;
                while i + plen <= line_lower.len() {
                    if line_lower[i..i + plen] == pat_chars[..] {
                        out.push(Position::new(row, i));
                    }
                    i += 1; // overlapping allowed (vim default for most)
                }
            } else {
                let mut search_from = 0usize;
                while search_from <= line.len() {
                    if let Some(byte_rel) = line[search_from..].find(pattern) {
                        let byte_abs = search_from + byte_rel;
                        let col = line[..byte_abs].chars().count();
                        out.push(Position::new(row, col));
                        search_from = byte_abs + pattern.len().max(1);
                    } else {
                        break;
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn collect_is_smart_case_and_overlapping() {
        let text = lines(&["Foo foo foo", "nothing", "fooFOO"]);
        // All-lowercase pattern → case-insensitive.
        let m = SearchState::collect(&text, "foo");
        assert_eq!(
            m.len(),
            5,
            "Foo + foo + foo on row 0, foo + FOO-part on row 2"
        );
        assert_eq!(m[0], Position::new(0, 0));
        // Uppercase in pattern → byte-exact ("FOO" is NOT "Foo").
        let m = SearchState::collect(&text, "Foo");
        assert_eq!(m, vec![Position::new(0, 0)]);
        // Overlaps count: "aa" in "aaa" matches twice.
        let m = SearchState::collect(&lines(&["aaa"]), "aa");
        assert_eq!(m, vec![Position::new(0, 0), Position::new(0, 1)]);
    }

    #[test]
    fn step_wraps_and_reports_the_wrap() {
        let mut s = SearchState::default();
        s.matches = vec![
            Position::new(0, 0),
            Position::new(1, 0),
            Position::new(2, 0),
        ];
        let (idx, wrapped) = s.step(Position::new(1, 0), true).unwrap();
        assert_eq!((idx, wrapped), (2, false));
        // Past the last → wraps to the first.
        let (idx, wrapped) = s.step(Position::new(2, 0), true).unwrap();
        assert_eq!((idx, wrapped), (0, true));
        // Before the first, backwards → wraps to the last.
        let (idx, wrapped) = s.step(Position::new(0, 0), false).unwrap();
        assert_eq!((idx, wrapped), (2, true));
    }

    #[test]
    fn nearest_respects_direction_and_inclusivity() {
        let mut s = SearchState::default();
        s.matches = vec![Position::new(0, 5), Position::new(1, 3)];
        // Forward includes the match AT the origin.
        assert_eq!(s.nearest(Position::new(0, 5), true), Some(0));
        assert_eq!(s.nearest(Position::new(0, 6), true), Some(1));
        // Reverse includes it too.
        assert_eq!(s.nearest(Position::new(1, 3), false), Some(1));
        assert_eq!(s.nearest(Position::new(1, 2), false), Some(0));
        // Nothing past → wraps to the far end.
        assert_eq!(s.nearest(Position::new(9, 0), true), Some(0));
        s.matches.clear();
        assert_eq!(s.nearest(Position::new(0, 0), true), None);
    }

    #[test]
    fn cancel_restores_the_pre_search_pattern() {
        let mut s = SearchState::default();
        s.pattern = Some("old".into());
        s.begin(true, Position::new(3, 4), 42);
        s.pattern = Some("new".into());
        let (origin, scroll, restored) = s.cancel();
        assert_eq!(origin, Some(Position::new(3, 4)));
        assert_eq!(scroll, 42);
        assert_eq!(restored.as_deref(), Some("old"));
        assert_eq!(s.pattern.as_deref(), Some("old"));
        assert!(s.input.is_empty() && s.matches.is_empty() && s.current == 0);
    }

    #[test]
    fn cycle_walks_the_live_matches() {
        let mut s = SearchState::default();
        assert_eq!(s.cycle(true), None, "no matches, no cycle");
        s.matches = vec![Position::new(0, 0), Position::new(1, 0)];
        s.current = 0;
        assert_eq!(s.cycle(true), Some(Position::new(1, 0)));
        assert_eq!(s.cycle(true), Some(Position::new(0, 0)), "wraps forward");
        assert_eq!(s.cycle(false), Some(Position::new(1, 0)), "wraps backward");
    }
}
