//! Vim-style operators × motions × text objects.
//!
//! Ranges are **inclusive** start and **exclusive** end in (row, col) space,
//! except `linewise` ranges which cover whole lines.

use crate::buffer::{Buffer, Position};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordForward,
    WordBack,
    WordEnd,
    LineStart,
    LineEnd,
    FirstNonBlank,
    /// Inclusive find char on line
    FindForward(char),
    FindBackward(char),
    TillForward(char),
    TillBackward(char),
    /// Down/up linewise (for dj/dk)
    LineDown,
    LineUp,
    /// Entire current line (dd / yy / cc)
    WholeLine,
    /// To end of buffer (dG)
    BufferEnd,
    /// To start of buffer (dgg) — handled as linewise from top
    BufferStart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextObject {
    InnerWord,
    AWord,
    InnerQuote(char),
    AQuote(char),
    InnerBracket(char, char), // open, close
    ABracket(char, char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditRange {
    pub start: Position,
    pub end: Position,
    pub linewise: bool,
}

impl EditRange {
    pub fn new(start: Position, end: Position, linewise: bool) -> Self {
        let (start, end) = order(start, end);
        Self {
            start,
            end,
            linewise,
        }
    }
}

fn order(a: Position, b: Position) -> (Position, Position) {
    if a.row < b.row || (a.row == b.row && a.col <= b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

/// Motion from cursor, applied `count` times. Returns exclusive end of the span
/// from original cursor (or linewise range).
pub fn range_for_motion(buf: &Buffer, motion: Motion, count: usize) -> EditRange {
    let count = count.max(1);
    let start = buf.cursor();
    let mut tmp = buf.clone();
    tmp.cursor = start;

    match motion {
        Motion::WholeLine => {
            let row = start.row;
            let end_row = (row + count - 1).min(buf.line_count().saturating_sub(1));
            return EditRange {
                start: Position::new(row, 0),
                end: Position::new(end_row, buf.line(end_row).chars().count()),
                linewise: true,
            };
        }
        Motion::LineDown => {
            let end_row = (start.row + count).min(buf.line_count().saturating_sub(1));
            return EditRange {
                start: Position::new(start.row, 0),
                end: Position::new(end_row, buf.line(end_row).chars().count()),
                linewise: true,
            };
        }
        Motion::LineUp => {
            let end_row = start.row.saturating_sub(count);
            return EditRange {
                start: Position::new(end_row, 0),
                end: Position::new(start.row, buf.line(start.row).chars().count()),
                linewise: true,
            };
        }
        Motion::BufferEnd => {
            let last = buf.line_count().saturating_sub(1);
            return EditRange {
                start: Position::new(start.row, 0),
                end: Position::new(last, buf.line(last).chars().count()),
                linewise: true,
            };
        }
        Motion::BufferStart => {
            return EditRange {
                start: Position::new(0, 0),
                end: Position::new(start.row, buf.line(start.row).chars().count()),
                linewise: true,
            };
        }
        _ => {}
    }

    for _ in 0..count {
        match motion {
            Motion::Left => tmp.move_left(),
            Motion::Right => tmp.move_right(),
            Motion::Up => tmp.move_up(),
            Motion::Down => tmp.move_down(),
            Motion::WordForward => tmp.move_word_forward(),
            Motion::WordBack => tmp.move_word_back(),
            Motion::WordEnd => move_word_end(&mut tmp),
            Motion::LineStart => tmp.move_to_line_start(),
            Motion::LineEnd => tmp.move_to_line_end(),
            Motion::FirstNonBlank => tmp.move_to_first_non_blank(),
            Motion::FindForward(c) => tmp.find_char_forward(c),
            Motion::FindBackward(c) => tmp.find_char_backward(c),
            Motion::TillForward(c) => tmp.till_char_forward(c),
            Motion::TillBackward(c) => tmp.till_char_backward(c),
            _ => {}
        }
    }

    let end = tmp.cursor();
    // Inclusive find: include the found character for d/f style
    let end = match motion {
        Motion::FindForward(_) | Motion::WordEnd => Position::new(end.row, end.col + 1),
        Motion::FindBackward(_) => {
            // range from end to start+1
            return EditRange::new(end, Position::new(start.row, start.col + 1), false);
        }
        _ => end,
    };

    // For backward motions, order handles it
    if end.row < start.row || (end.row == start.row && end.col < start.col) {
        EditRange::new(end, start, false)
    } else {
        EditRange {
            start,
            end,
            linewise: false,
        }
    }
}

fn move_word_end(buf: &mut Buffer) {
    let chars: Vec<char> = buf.line(buf.cursor.row).chars().collect();
    let mut i = buf.cursor.col;
    if i >= chars.len() {
        if buf.cursor.row + 1 < buf.line_count() {
            buf.cursor.row += 1;
            buf.cursor.col = 0;
            move_word_end(buf);
        }
        return;
    }
    // If on whitespace, skip to next word first
    if chars[i].is_whitespace() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            buf.cursor.col = chars.len();
            return;
        }
    } else if i + 1 < chars.len() {
        // leave current end-of-word: step forward once if middle of word
        let class = word_class(chars[i]);
        if i + 1 < chars.len() && word_class(chars[i + 1]) == class {
            i += 1;
        } else {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
        }
    } else {
        i = chars.len().saturating_sub(1);
        buf.cursor.col = i;
        return;
    }
    if i >= chars.len() {
        buf.cursor.col = chars.len().saturating_sub(1);
        return;
    }
    let class = word_class(chars[i]);
    while i + 1 < chars.len() && word_class(chars[i + 1]) == class {
        i += 1;
    }
    buf.cursor.col = i;
}

#[derive(Clone, Copy, PartialEq)]
enum WClass {
    Space,
    Word,
    Punct,
}

fn word_class(c: char) -> WClass {
    if c.is_whitespace() {
        WClass::Space
    } else if c.is_alphanumeric() || c == '_' {
        WClass::Word
    } else {
        WClass::Punct
    }
}

pub fn range_for_textobject(buf: &Buffer, obj: TextObject) -> Option<EditRange> {
    let cur = buf.cursor();
    match obj {
        TextObject::InnerWord | TextObject::AWord => {
            let line = buf.line(cur.row);
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() {
                return None;
            }
            let col = cur.col.min(chars.len().saturating_sub(1));
            if chars[col].is_whitespace() {
                if obj == TextObject::InnerWord {
                    return None;
                }
                // aW on space: include surrounding spaces? keep simple: expand spaces
                let mut s = col;
                let mut e = col;
                while s > 0 && chars[s - 1].is_whitespace() {
                    s -= 1;
                }
                while e < chars.len() && chars[e].is_whitespace() {
                    e += 1;
                }
                return Some(EditRange {
                    start: Position::new(cur.row, s),
                    end: Position::new(cur.row, e),
                    linewise: false,
                });
            }
            let class = word_class(chars[col]);
            let mut s = col;
            let mut e = col;
            while s > 0 && word_class(chars[s - 1]) == class {
                s -= 1;
            }
            while e < chars.len() && word_class(chars[e]) == class {
                e += 1;
            }
            if obj == TextObject::AWord {
                // include trailing whitespace
                while e < chars.len() && chars[e].is_whitespace() {
                    e += 1;
                }
                if e == col + 1 || (s < e && e == s) {
                    // leading whitespace if no trailing
                    while s > 0 && chars[s - 1].is_whitespace() {
                        s -= 1;
                    }
                }
            }
            Some(EditRange {
                start: Position::new(cur.row, s),
                end: Position::new(cur.row, e),
                linewise: false,
            })
        }
        TextObject::InnerQuote(q) | TextObject::AQuote(q) => {
            let line = buf.line(cur.row);
            let chars: Vec<char> = line.chars().collect();
            // find quotes on this line
            let mut opens = Vec::new();
            for (i, &c) in chars.iter().enumerate() {
                if c == q {
                    opens.push(i);
                }
            }
            if opens.len() < 2 {
                return None;
            }
            // pair surrounding cursor
            let mut best: Option<(usize, usize)> = None;
            for pair in opens.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                let a = pair[0];
                let b = pair[1];
                if cur.col >= a && cur.col <= b {
                    best = Some((a, b));
                    break;
                }
            }
            // if cursor not between, take nearest pair after cursor or first
            let (a, b) = best.or_else(|| {
                opens.chunks(2).find_map(|p| {
                    if p.len() == 2 {
                        Some((p[0], p[1]))
                    } else {
                        None
                    }
                })
            })?;
            if matches!(obj, TextObject::InnerQuote(_)) {
                Some(EditRange {
                    start: Position::new(cur.row, a + 1),
                    end: Position::new(cur.row, b),
                    linewise: false,
                })
            } else {
                Some(EditRange {
                    start: Position::new(cur.row, a),
                    end: Position::new(cur.row, b + 1),
                    linewise: false,
                })
            }
        }
        TextObject::InnerBracket(open, close) | TextObject::ABracket(open, close) => {
            find_bracket_range(buf, cur, open, close, matches!(obj, TextObject::ABracket(_, _)))
        }
    }
}

fn find_bracket_range(
    buf: &Buffer,
    cur: Position,
    open: char,
    close: char,
    around: bool,
) -> Option<EditRange> {
    // Search backward for open with depth, then forward for close.
    let mut depth = 0i32;
    let mut open_pos: Option<Position> = None;

    // scan from cursor backward through buffer
    let mut r = cur.row as isize;
    let mut started = false;
    while r >= 0 {
        let row = r as usize;
        let chars: Vec<char> = buf.line(row).chars().collect();
        let mut c_idx = if row == cur.row && !started {
            started = true;
            cur.col.min(chars.len())
        } else {
            chars.len()
        };
        while c_idx > 0 {
            c_idx -= 1;
            let ch = chars[c_idx];
            if ch == close {
                depth += 1;
            } else if ch == open {
                if depth == 0 {
                    open_pos = Some(Position::new(row, c_idx));
                    break;
                }
                depth -= 1;
            }
        }
        if open_pos.is_some() {
            break;
        }
        r -= 1;
    }
    let op = open_pos?;

    // forward for matching close
    depth = 0;
    let mut close_pos: Option<Position> = None;
    for row in op.row..buf.line_count() {
        let chars: Vec<char> = buf.line(row).chars().collect();
        let start_col = if row == op.row { op.col + 1 } else { 0 };
        for (c_idx, &ch) in chars.iter().enumerate().skip(start_col) {
            if ch == open {
                depth += 1;
            } else if ch == close {
                if depth == 0 {
                    close_pos = Some(Position::new(row, c_idx));
                    break;
                }
                depth -= 1;
            }
        }
        if close_pos.is_some() {
            break;
        }
    }
    let cp = close_pos?;

    if around {
        Some(EditRange {
            start: op,
            end: Position::new(cp.row, cp.col + 1),
            linewise: false,
        })
    } else {
        Some(EditRange {
            start: Position::new(op.row, op.col + 1),
            end: cp,
            linewise: false,
        })
    }
}

/// Extract text for a range (exclusive end for charwise).
pub fn extract_text(buf: &Buffer, range: EditRange) -> String {
    if range.linewise {
        let mut lines = Vec::new();
        for row in range.start.row..=range.end.row.min(buf.line_count().saturating_sub(1)) {
            lines.push(buf.line(row).to_string());
        }
        return lines.join("\n");
    }
    if range.start.row == range.end.row {
        let chars: Vec<char> = buf.line(range.start.row).chars().collect();
        let s = range.start.col.min(chars.len());
        let e = range.end.col.min(chars.len());
        if s >= e {
            return String::new();
        }
        return chars[s..e].iter().collect();
    }
    let mut out = String::new();
    let first: Vec<char> = buf.line(range.start.row).chars().collect();
    let s = range.start.col.min(first.len());
    out.extend(first[s..].iter());
    out.push('\n');
    for row in (range.start.row + 1)..range.end.row {
        out.push_str(buf.line(row));
        out.push('\n');
    }
    let last: Vec<char> = buf.line(range.end.row).chars().collect();
    let e = range.end.col.min(last.len());
    out.extend(last[..e].iter());
    out
}

/// Delete range from buffer; returns deleted text. Cursor moves to start.
pub fn delete_range(buf: &mut Buffer, range: EditRange) -> String {
    let text = extract_text(buf, range);
    if range.linewise {
        let start = range.start.row;
        let end = range.end.row.min(buf.line_count().saturating_sub(1));
        if start == 0 && end + 1 >= buf.line_count() {
            // entire buffer
            *buf = Buffer::new();
            return text;
        }
        for _ in start..=end {
            if buf.line_count() == 1 {
                buf.set_line(0, String::new());
                break;
            }
            buf.cursor.row = start.min(buf.line_count().saturating_sub(1));
            let _ = buf.delete_line();
        }
        buf.cursor.row = start.min(buf.line_count().saturating_sub(1));
        buf.cursor.col = 0;
        return text;
    }

    if range.start.row == range.end.row {
        let line = buf.line(range.start.row);
        let chars: Vec<char> = line.chars().collect();
        let s = range.start.col.min(chars.len());
        let e = range.end.col.min(chars.len());
        let new_line: String = chars[..s].iter().chain(chars[e..].iter()).collect();
        buf.set_line(range.start.row, new_line);
        buf.cursor = Position::new(range.start.row, s);
        return text;
    }

    let first_chars: Vec<char> = buf.line(range.start.row).chars().collect();
    let last_chars: Vec<char> = buf.line(range.end.row).chars().collect();
    let prefix: String = first_chars.iter().take(range.start.col).collect();
    let suffix: String = last_chars.iter().skip(range.end.col).collect();
    let merged = prefix + &suffix;

    // delete lines from end down to start+1
    for row in (range.start.row + 1..=range.end.row).rev() {
        if row < buf.line_count() {
            buf.cursor.row = row;
            let _ = buf.delete_line();
        }
    }
    buf.set_line(range.start.row, merged);
    buf.cursor = Position::new(range.start.row, range.start.col);
    buf.clamp_col();
    text
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LastChange {
    Operator {
        op: Operator,
        motion: Motion,
        count: usize,
    },
    TextObject {
        op: Operator,
        obj: TextObject,
        count: usize,
    },
    DeleteChar {
        count: usize,
    },
    ReplaceChar {
        ch: char,
    },
}

pub fn parse_textobject(mod_char: char, obj_char: char) -> Option<TextObject> {
    let inner = mod_char == 'i';
    let around = mod_char == 'a';
    if !inner && !around {
        return None;
    }
    match obj_char {
        'w' => Some(if inner {
            TextObject::InnerWord
        } else {
            TextObject::AWord
        }),
        '"' | '\'' | '`' => Some(if inner {
            TextObject::InnerQuote(obj_char)
        } else {
            TextObject::AQuote(obj_char)
        }),
        '(' | ')' | 'b' => Some(if inner {
            TextObject::InnerBracket('(', ')')
        } else {
            TextObject::ABracket('(', ')')
        }),
        '[' | ']' => Some(if inner {
            TextObject::InnerBracket('[', ']')
        } else {
            TextObject::ABracket('[', ']')
        }),
        '{' | '}' | 'B' => Some(if inner {
            TextObject::InnerBracket('{', '}')
        } else {
            TextObject::ABracket('{', '}')
        }),
        '<' | '>' => Some(if inner {
            TextObject::InnerBracket('<', '>')
        } else {
            TextObject::ABracket('<', '>')
        }),
        _ => None,
    }
}

pub fn motion_from_char(c: char) -> Option<Motion> {
    Some(match c {
        'h' => Motion::Left,
        'l' => Motion::Right,
        'j' => Motion::LineDown,
        'k' => Motion::LineUp,
        'w' => Motion::WordForward,
        'b' => Motion::WordBack,
        'e' => Motion::WordEnd,
        '0' => Motion::LineStart,
        '$' => Motion::LineEnd,
        '^' => Motion::FirstNonBlank,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diw_deletes_word() {
        let mut buf = Buffer::from_string("hello world");
        buf.cursor = Position::new(0, 1);
        let range = range_for_textobject(&buf, TextObject::InnerWord).unwrap();
        let t = delete_range(&mut buf, range);
        assert_eq!(t, "hello");
        assert_eq!(buf.line(0), " world");
    }

    #[test]
    fn di_quote() {
        let mut buf = Buffer::from_string(r#"say "hi" now"#);
        buf.cursor = Position::new(0, 5); // inside quotes
        let range = range_for_textobject(&buf, TextObject::InnerQuote('"')).unwrap();
        let t = delete_range(&mut buf, range);
        assert_eq!(t, "hi");
        assert_eq!(buf.line(0), r#"say "" now"#);
    }

    #[test]
    fn dw_motion() {
        let mut buf = Buffer::from_string("foo bar");
        buf.cursor = Position::new(0, 0);
        let range = range_for_motion(&buf, Motion::WordForward, 1);
        let t = delete_range(&mut buf, range);
        assert_eq!(t, "foo ");
        assert_eq!(buf.line(0), "bar");
    }

    #[test]
    fn dd_linewise() {
        let mut buf = Buffer::from_string("a\nb\nc");
        buf.cursor = Position::new(1, 0);
        let range = range_for_motion(&buf, Motion::WholeLine, 1);
        let t = delete_range(&mut buf, range);
        assert_eq!(t, "b");
        assert_eq!(buf.text(), "a\nc");
    }

    #[test]
    fn dib_parens() {
        let mut buf = Buffer::from_string("x(hello)y");
        buf.cursor = Position::new(0, 3);
        let range = range_for_textobject(&buf, TextObject::InnerBracket('(', ')')).unwrap();
        let t = delete_range(&mut buf, range);
        assert_eq!(t, "hello");
        assert_eq!(buf.line(0), "x()y");
    }
}
