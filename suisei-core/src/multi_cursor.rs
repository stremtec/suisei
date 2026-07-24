//! Multi-cursor (v1) — primary + extra carets for insert/edit.

use crate::buffer::{Buffer, Position};

/// Extra carets beyond `Buffer.cursor` (the primary).
#[derive(Debug, Clone, Default)]
pub struct MultiCursor {
    pub extras: Vec<Position>,
}

impl MultiCursor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.extras.clear();
    }

    pub fn is_active(&self) -> bool {
        !self.extras.is_empty()
    }

    pub fn count(&self, _primary: Position) -> usize {
        1 + self.extras.len()
    }

    /// All cursors including primary, sorted document order, deduped.
    pub fn all(&self, primary: Position) -> Vec<Position> {
        let mut v = vec![primary];
        v.extend(self.extras.iter().copied());
        v.sort_by(|a, b| a.row.cmp(&b.row).then(a.col.cmp(&b.col)));
        v.dedup();
        v
    }

    /// After an edit, replace set from new primary + extras (already sorted).
    pub fn set_from_all(&mut self, mut all: Vec<Position>) {
        all.sort_by(|a, b| a.row.cmp(&b.row).then(a.col.cmp(&b.col)));
        all.dedup();
        if all.is_empty() {
            self.extras.clear();
            return;
        }
        // Keep first as primary (caller assigns buffer.cursor)
        self.extras = all.into_iter().skip(1).collect();
    }

    pub fn add(&mut self, primary: Position, pos: Position) {
        if pos == primary {
            return;
        }
        if !self.extras.iter().any(|p| *p == pos) {
            self.extras.push(pos);
            self.extras
                .sort_by(|a, b| a.row.cmp(&b.row).then(a.col.cmp(&b.col)));
        }
    }

    pub fn remove_last(&mut self) -> bool {
        self.extras.pop().is_some()
    }

    /// Clamp every cursor to buffer bounds.
    pub fn clamp_all(&mut self, buf: &Buffer) {
        let max_row = buf.line_count().saturating_sub(1);
        for p in &mut self.extras {
            if p.row > max_row {
                p.row = max_row;
            }
            let max_col = buf.line(p.row).chars().count();
            if p.col > max_col {
                p.col = max_col;
            }
        }
        self.extras.retain(|p| p.row <= max_row);
    }
}

/// Word under cursor for multi-cursor "select next".
pub fn word_at(buf: &Buffer, pos: Position) -> Option<(Position, Position, String)> {
    let line = buf.line(pos.row);
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let col = pos.col.min(chars.len().saturating_sub(1).max(0));
    if col >= chars.len() && chars.is_empty() {
        return None;
    }
    let c = if col < chars.len() {
        chars[col]
    } else if col > 0 {
        chars[col - 1]
    } else {
        return None;
    };
    if !(c.is_alphanumeric() || c == '_') {
        return None;
    }
    let mut start = col.min(chars.len().saturating_sub(1));
    let mut end = start;
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    while end + 1 < chars.len() && (chars[end + 1].is_alphanumeric() || chars[end + 1] == '_') {
        end += 1;
    }
    let word: String = chars[start..=end].iter().collect();
    Some((
        Position {
            row: pos.row,
            col: start,
        },
        Position {
            row: pos.row,
            col: end + 1,
        },
        word,
    ))
}

/// Find next occurrence of `word` after `from` (exclusive start position).
pub fn find_next(buf: &Buffer, word: &str, from: Position) -> Option<Position> {
    if word.is_empty() {
        return None;
    }
    let n = buf.line_count();
    // Search current line after col, then following lines
    for row in from.row..n {
        let line = buf.line(row);
        let start_col = if row == from.row { from.col } else { 0 };
        let chars: Vec<char> = line.chars().collect();
        if start_col >= chars.len() {
            continue;
        }
        let s: String = chars[start_col..].iter().collect();
        if let Some(rel) = s.find(word) {
            // Verify word boundary-ish: check not mid-identifier for alphanumeric words
            let abs = start_col + rel;
            if is_word_match(&chars, abs, word) {
                return Some(Position {
                    row,
                    col: abs,
                });
            }
            // keep searching same line for next
            let mut search_from = abs + 1;
            while search_from < chars.len() {
                let rest: String = chars[search_from..].iter().collect();
                if let Some(r2) = rest.find(word) {
                    let abs2 = search_from + r2;
                    if is_word_match(&chars, abs2, word) {
                        return Some(Position {
                            row,
                            col: abs2,
                        });
                    }
                    search_from = abs2 + 1;
                } else {
                    break;
                }
            }
        }
    }
    // Wrap from top
    for row in 0..=from.row {
        let line = buf.line(row);
        let chars: Vec<char> = line.chars().collect();
        let limit = if row == from.row {
            from.col.min(chars.len())
        } else {
            chars.len()
        };
        let s: String = chars[..limit].iter().collect();
        if let Some(abs) = s.find(word) {
            if is_word_match(&chars, abs, word) {
                return Some(Position { row, col: abs });
            }
        }
    }
    None
}

fn is_word_match(chars: &[char], start: usize, word: &str) -> bool {
    let wchars: Vec<char> = word.chars().collect();
    if start + wchars.len() > chars.len() {
        return false;
    }
    if chars[start..start + wchars.len()] != wchars[..] {
        return false;
    }
    let before_ok = start == 0
        || !(chars[start - 1].is_alphanumeric() || chars[start - 1] == '_');
    let after = start + wchars.len();
    let after_ok = after >= chars.len()
        || !(chars[after].is_alphanumeric() || chars[after] == '_');
    before_ok && after_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    #[test]
    fn find_next_word() {
        let buf = Buffer::from_string("foo bar foo\nfoo");
        let p = find_next(&buf, "foo", Position { row: 0, col: 1 }).unwrap();
        assert_eq!(p, Position { row: 0, col: 8 });
    }
}
