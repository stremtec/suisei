//! Indent-based code folding.

use std::collections::{HashMap, HashSet};

/// Inclusive line range that can collapse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldRange {
    pub start: usize,
    pub end: usize, // inclusive
}

#[derive(Debug, Clone, Default)]
pub struct FoldState {
    /// All detected fold ranges (start → end).
    pub ranges: Vec<FoldRange>,
    /// Start lines of currently closed folds.
    pub closed: HashSet<usize>,
    /// Derived O(1) lookup: fold-start row → widest range starting there.
    /// Rebuilt whenever `ranges` changes; keeps `fold_at`/`closed_count` off the
    /// per-row-per-frame linear scan that dominated large-file rendering.
    starts: HashMap<usize, FoldRange>,
    /// Derived O(1) lookup: every row currently hidden inside a closed fold.
    /// Recomputed only when `closed` or `ranges` changes.
    hidden: HashSet<usize>,
}

impl FoldState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.ranges.clear();
        self.closed.clear();
        self.starts.clear();
        self.hidden.clear();
    }

    /// Rebuild the `starts` index from `ranges` (keeps the widest fold per row).
    fn reindex_starts(&mut self) {
        self.starts.clear();
        for r in &self.ranges {
            self.starts
                .entry(r.start)
                .and_modify(|cur| {
                    if r.end > cur.end {
                        *cur = *r;
                    }
                })
                .or_insert(*r);
        }
    }

    /// Recompute the `hidden` set from the closed folds. O(closed × span) once
    /// per fold-state change instead of O(ranges) per row per frame.
    fn recompute_hidden(&mut self) {
        self.hidden.clear();
        for start in &self.closed {
            if let Some(r) = self.starts.get(start) {
                for row in (r.start + 1)..=r.end {
                    self.hidden.insert(row);
                }
            }
        }
    }

    /// Rebuild indent folds from buffer lines.
    pub fn rebuild(&mut self, lines: &[String], tab_width: usize) {
        let old_closed = self.closed.clone();
        self.ranges.clear();
        self.closed.clear();

        let n = lines.len();
        if n < 2 {
            return;
        }
        let indents: Vec<usize> = lines.iter().map(|l| line_indent(l, tab_width)).collect();

        for i in 0..n.saturating_sub(1) {
            // Skip blank lines as fold starts
            if lines[i].trim().is_empty() {
                continue;
            }
            let base = indents[i];
            // Look for a block that starts with increased indent after this line
            let mut j = i + 1;
            while j < n && lines[j].trim().is_empty() {
                j += 1;
            }
            if j >= n || indents[j] <= base {
                continue;
            }
            // Extend while indent > base (or blank)
            let mut end = j;
            let mut k = j;
            while k < n {
                if lines[k].trim().is_empty() {
                    k += 1;
                    continue;
                }
                if indents[k] > base {
                    end = k;
                    k += 1;
                } else {
                    break;
                }
            }
            if end > i {
                self.ranges.push(FoldRange {
                    start: i,
                    end,
                });
            }
        }

        // Restore closed state for ranges that still exist
        for r in &self.ranges {
            if old_closed.contains(&r.start) {
                self.closed.insert(r.start);
            }
        }
        self.reindex_starts();
        self.recompute_hidden();
    }

    pub fn fold_at(&self, row: usize) -> Option<FoldRange> {
        self.starts.get(&row).copied()
    }

    pub fn is_closed(&self, start: usize) -> bool {
        self.closed.contains(&start)
    }

    /// True if `row` is hidden inside a closed fold (not the header line).
    pub fn is_hidden(&self, row: usize) -> bool {
        self.hidden.contains(&row)
    }

    pub fn toggle(&mut self, row: usize) -> Option<&'static str> {
        // Prefer fold starting at row; else enclosing fold start
        let start = if self.starts.contains_key(&row) {
            row
        } else {
            self.ranges
                .iter()
                .filter(|r| row > r.start && row <= r.end)
                .max_by_key(|r| r.start)
                .map(|r| r.start)?
        };
        let msg = if self.closed.contains(&start) {
            self.closed.remove(&start);
            Some("opened fold")
        } else if self.starts.contains_key(&start) {
            self.closed.insert(start);
            Some("closed fold")
        } else {
            None
        };
        if msg.is_some() {
            self.recompute_hidden();
        }
        msg
    }

    pub fn close_at(&mut self, row: usize) -> bool {
        if !self.starts.contains_key(&row) {
            return false;
        }
        self.closed.insert(row);
        self.recompute_hidden();
        true
    }

    pub fn open_at(&mut self, row: usize) -> bool {
        let removed = self.closed.remove(&row);
        if removed {
            self.recompute_hidden();
        }
        removed
    }

    pub fn close_all(&mut self) {
        for r in &self.ranges {
            self.closed.insert(r.start);
        }
        self.recompute_hidden();
    }

    pub fn open_all(&mut self) {
        self.closed.clear();
        self.hidden.clear();
    }

    /// Lines hidden under a closed fold starting at `start`.
    pub fn closed_count(&self, start: usize) -> usize {
        self.fold_at(start)
            .filter(|_| self.is_closed(start))
            .map(|r| r.end - r.start)
            .unwrap_or(0)
    }
}

fn line_indent(line: &str, tab_width: usize) -> usize {
    let mut n = 0;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += tab_width,
            _ => break,
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indent_fold_fn_body() {
        let lines = vec![
            "fn main() {".into(),
            "    let x = 1;".into(),
            "    let y = 2;".into(),
            "}".into(),
        ];
        let mut f = FoldState::new();
        f.rebuild(&lines, 4);
        assert!(!f.ranges.is_empty());
        let r = f.fold_at(0).expect("fold on fn line");
        assert_eq!(r.start, 0);
        assert!(r.end >= 2);
        f.toggle(0);
        assert!(f.is_hidden(1));
        assert!(!f.is_hidden(0));
    }

    #[test]
    fn close_all_open_all_updates_hidden_index() {
        let lines = vec![
            "fn a() {".into(),
            "    let x = 1;".into(),
            "}".into(),
            "fn b() {".into(),
            "    let y = 2;".into(),
            "}".into(),
        ];
        let mut f = FoldState::new();
        f.rebuild(&lines, 4);
        f.close_all();
        assert!(f.is_hidden(1));
        assert!(f.is_hidden(4));
        assert!(!f.is_hidden(0));
        assert!(!f.is_hidden(3));
        f.open_all();
        assert!(!f.is_hidden(1));
        assert!(!f.is_hidden(4));
        // open_at only recomputes when something was actually closed
        f.close_at(0);
        assert!(f.is_hidden(1));
        f.open_at(0);
        assert!(!f.is_hidden(1));
    }
}
