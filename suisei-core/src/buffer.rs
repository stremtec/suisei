use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}

impl Position {
    #[allow(dead_code)]
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }

    pub fn zero() -> Self {
        Self { row: 0, col: 0 }
    }
}

/// Row-major document order: earlier row first, then earlier column. This is
/// what the `Selection` model needs to normalise anchor/head into a range and
/// to merge overlapping selections, so it lives on `Position` itself rather
/// than being re-derived at each call site.
impl PartialOrd for Position {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Position {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.row.cmp(&other.row).then(self.col.cmp(&other.col))
    }
}

#[derive(Clone)]
pub struct Buffer {
    lines: Vec<String>,
    pub cursor: Position,
    /// Bumped on every text mutation — frames re-parse/re-sync only on change.
    version: u64,
}

fn next_version() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            version: next_version(),
            cursor: Position::zero(),
        }
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_string(text: &str) -> Self {
        let lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
        Self {
            lines,
            cursor: Position::zero(),
            version: next_version(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, row: usize) -> &str {
        self.lines.get(row).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn cursor(&self) -> Position {
        self.cursor
    }

    pub fn current_line_len(&self) -> usize {
        self.line(self.cursor.row).chars().count()
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Move left by one Unicode grapheme cluster (not a scalar/code-unit step).
    pub fn move_left(&mut self) {
        if self.cursor.col > 0 {
            let line = self.line(self.cursor.row);
            self.cursor.col = grapheme_prev_col(line, self.cursor.col);
        }
    }

    /// Move right by one Unicode grapheme cluster.
    pub fn move_right(&mut self) {
        let line = self.line(self.cursor.row);
        let max = line.chars().count();
        if self.cursor.col < max {
            self.cursor.col = grapheme_next_col(line, self.cursor.col);
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.clamp_col();
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor.row < self.lines.len() - 1 {
            self.cursor.row += 1;
            self.clamp_col();
        }
    }

    pub fn move_to_line_start(&mut self) {
        self.cursor.col = 0;
    }

    pub fn move_to_line_end(&mut self) {
        self.cursor.col = self.current_line_len();
    }

    pub fn clamp_col(&mut self) {
        let max = self.current_line_len();
        if self.cursor.col > max {
            self.cursor.col = max;
        }
    }

    pub fn set_line(&mut self, row: usize, text: String) {
        self.touch();
        if row < self.lines.len() {
            self.lines[row] = text;
        }
    }

    pub fn buffer_col_to_screen_col(&self, row: usize, buf_col: usize) -> usize {
        let line = self.line(row);
        let mut visual = 0;
        for (i, ch) in line.chars().enumerate() {
            if i >= buf_col {
                return visual;
            }
            visual += if ch == '\t' {
                4 - (visual % 4)
            } else {
                ch.width().unwrap_or(1)
            };
        }
        visual
    }

    pub fn screen_col_to_buffer_col(&self, row: usize, screen_col: usize) -> usize {
        let line = self.line(row);
        let mut visual = 0;
        let mut buf_col = 0;
        for ch in line.chars() {
            let w = if ch == '\t' {
                4 - (visual % 4)
            } else {
                ch.width().unwrap_or(1)
            };
            if visual + w > screen_col {
                return buf_col;
            }
            visual += w;
            buf_col += 1;
        }
        buf_col
    }

    #[allow(dead_code)]
    pub fn append_to_line(&mut self, row: usize, text: &str) {
        self.touch();
        if row < self.lines.len() {
            self.lines[row].push_str(text);
        }
    }

    pub fn insert_line_at(&mut self, row: usize, line: String) {
        self.touch();
        self.lines.insert(row, line);
        self.cursor.row = row;
    }

    pub fn insert_char(&mut self, ch: char) {
        self.touch();
        let line = &mut self.lines[self.cursor.row];
        let byte_idx = char_to_byte(self.cursor.col, line);
        line.insert(byte_idx, ch);
        self.cursor.col += 1;
    }

    /// Insert multi-line text at the cursor (snippets / paste-like).
    ///
    /// Bulk splice rather than char-by-char: a per-char `insert_char` walks
    /// `char_to_byte` (O(col)) and memmoves the line tail (O(len)) on every
    /// character, making a single long paste O(n²). Splitting on `\n` and
    /// splicing whole segments keeps it O(n). Final cursor position matches the
    /// old char-by-char behaviour exactly.
    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.touch();
        let row = self.cursor.row;
        let line = &mut self.lines[row];
        let byte = char_to_byte(self.cursor.col, line);

        if !s.contains('\n') {
            // Single-line fast path: one splice into the current line.
            line.insert_str(byte, s);
            self.cursor.col += s.chars().count();
            return;
        }

        // Multi-line: split the current line at the cursor, weld the paste's
        // first segment onto the head and its last segment onto the tail.
        let tail: String = line[byte..].to_string();
        line.truncate(byte);

        let mut segs = s.split('\n');
        let first = segs.next().unwrap_or("");
        self.lines[row].push_str(first);

        let rest: Vec<&str> = segs.collect();
        let n = rest.len();
        let mut new_lines: Vec<String> = Vec::with_capacity(n);
        let mut last_col = 0usize;
        for (i, seg) in rest.iter().enumerate() {
            if i == n - 1 {
                last_col = seg.chars().count();
                let mut l = String::with_capacity(seg.len() + tail.len());
                l.push_str(seg);
                l.push_str(&tail);
                new_lines.push(l);
            } else {
                new_lines.push((*seg).to_string());
            }
        }
        let at = row + 1;
        self.lines.splice(at..at, new_lines);
        self.cursor.row = row + n;
        self.cursor.col = last_col;
    }

    /// Delete the half-open span `[start, end)` (document order) and leave the
    /// cursor at `start`. Returns the removed text. The foundation the GUI
    /// selection edits (`gui_insert_text`, `gui_delete_*`) build on — one
    /// range delete instead of the vim-mode-specific slicing.
    pub fn delete_range(&mut self, start: Position, end: Position) -> String {
        if start >= end {
            self.cursor = start;
            return String::new();
        }
        self.touch();
        let last = self.lines.len().saturating_sub(1);
        let s = Position::new(start.row.min(last), start.col);
        let e = Position::new(end.row.min(last), end.col);

        if s.row == e.row {
            let line = &mut self.lines[s.row];
            let from = char_to_byte(s.col.min(line.chars().count()), line);
            let to = char_to_byte(e.col.min(line.chars().count()), line);
            let removed = line[from..to].to_string();
            line.replace_range(from..to, "");
            self.cursor = s;
            return removed;
        }

        // Multi-row: head of the first line + tail of the last line survive.
        let head: String = {
            let line = &self.lines[s.row];
            let from = char_to_byte(s.col.min(line.chars().count()), line);
            line[..from].to_string()
        };
        let tail: String = {
            let line = &self.lines[e.row];
            let to = char_to_byte(e.col.min(line.chars().count()), line);
            line[to..].to_string()
        };
        // Collect what we remove for the return value (best-effort, newline-joined).
        let mut removed = String::new();
        {
            let line = &self.lines[s.row];
            let from = char_to_byte(s.col.min(line.chars().count()), line);
            removed.push_str(&line[from..]);
        }
        for row in (s.row + 1)..e.row {
            removed.push('\n');
            removed.push_str(&self.lines[row]);
        }
        removed.push('\n');
        {
            let line = &self.lines[e.row];
            let to = char_to_byte(e.col.min(line.chars().count()), line);
            removed.push_str(&line[..to]);
        }

        let mut merged = head;
        merged.push_str(&tail);
        self.lines.splice(s.row..=e.row, std::iter::once(merged));
        self.cursor = s;
        removed
    }

    pub fn insert_char_pair(&mut self, open: char, close: char) {
        self.touch();
        let line = &mut self.lines[self.cursor.row];
        let byte_idx = char_to_byte(self.cursor.col, line);
        line.insert(byte_idx, open);
        let byte_idx2 = char_to_byte(self.cursor.col + 1, line);
        line.insert(byte_idx2, close);
        self.cursor.col += 1;
    }

    pub fn char_after_cursor(&self) -> Option<char> {
        self.line(self.cursor.row).chars().nth(self.cursor.col)
    }

    pub fn char_before_cursor(&self) -> Option<char> {
        if self.cursor.col > 0 {
            self.line(self.cursor.row).chars().nth(self.cursor.col - 1)
        } else {
            None
        }
    }

    pub fn skip_char_if_match(&mut self, ch: char) -> bool {
        if self.char_after_cursor() == Some(ch) {
            self.cursor.col += 1;
            true
        } else {
            false
        }
    }

    pub fn delete_pair(&mut self, open: char, close: char) -> bool {
        self.touch();
        if self.char_before_cursor() == Some(open) && self.char_after_cursor() == Some(close) {
            let line_str = self.lines[self.cursor.row].clone();
            let open_byte = char_to_byte(self.cursor.col - 1, &line_str);
            let open_end = char_to_byte(self.cursor.col, &line_str);
            let close_byte = char_to_byte(self.cursor.col, &line_str);
            let close_end = char_to_byte(self.cursor.col + 1, &line_str);

            let line = &mut self.lines[self.cursor.row];
            line.drain(close_byte..close_end);
            line.drain(open_byte..open_end);
            self.cursor.col -= 1;
            true
        } else {
            false
        }
    }

    pub fn insert_newline(&mut self) {
        self.touch();
        let line = &mut self.lines[self.cursor.row];
        let byte_idx = char_to_byte(self.cursor.col, line);
        let after: String = line.drain(byte_idx..).collect();
        self.lines.insert(self.cursor.row + 1, after);
        self.cursor.row += 1;
        self.cursor.col = 0;
    }

    pub fn insert_newline_with_indent(&mut self, extra_indent: bool) {
        self.touch();
        let current_row = self.cursor.row;
        let indent = self.leading_indent(current_row);

        let line = &mut self.lines[current_row];
        let byte_idx = char_to_byte(self.cursor.col, line);
        let after: String = line.drain(byte_idx..).collect();

        let mut new_line = indent.clone();
        if extra_indent {
            new_line.push_str("    ");
        }
        new_line.push_str(&after);

        self.lines.insert(current_row + 1, new_line);
        self.cursor.row += 1;
        // cursor.col is a char index
        self.cursor.col = indent.chars().count() + if extra_indent { 4 } else { 0 };
    }

    /// Enter with GUI-editor semantics.
    ///
    /// The old path judged indentation from the whole line's END and appended
    /// whatever followed the caret straight after the indent, so `{|}` became
    /// `{` / `    |}` — the closer stuck to the caret. Decisions here are made
    /// from the text BEFORE the caret, and an opener immediately followed by
    /// its closer expands to three lines with the closer back at the original
    /// indent, which is what Xcode and VS Code do.
    pub fn insert_newline_smart(&mut self, indent_unit: &str) {
        self.touch();
        let row = self.cursor.row;
        // The indent to carry down — and NOT when the caret is still inside it.
        //
        // `after` is everything past the caret, so splitting a line inside its
        // leading whitespace puts the rest of that whitespace in `after`.
        // Prefixing `base` on top of it counted the same indent twice: Enter at
        // column 0 of "␣␣abc" produced an empty line and "␣␣␣␣abc", and every
        // Enter inside an indent grew it again. That is the reported "새로운
        // 줄인데 맨 앞에 한 칸이 빈다" — the space was real and the editor put
        // it there.
        //
        // Past the indent, carrying it is the whole point of auto-indent, so
        // that is exactly where it applies.
        let indent_end = self
            .line(row)
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let base = if self.cursor.col >= indent_end {
            self.leading_indent(row)
        } else {
            String::new()
        };

        let line = &mut self.lines[row];
        let byte_idx = char_to_byte(self.cursor.col, line);
        let after: String = line.drain(byte_idx..).collect();
        let before_trimmed = self.lines[row].trim_end().to_string();

        let opens = before_trimmed.ends_with('{')
            || before_trimmed.ends_with('[')
            || before_trimmed.ends_with('(')
            || before_trimmed.ends_with(':')
            || before_trimmed.ends_with("=>")
            || before_trimmed.ends_with("->");
        let tail = after.trim_start();
        let closes_immediately =
            tail.starts_with('}') || tail.starts_with(']') || tail.starts_with(')');

        if opens && closes_immediately {
            // {|}  →  {
            //             |
            //         }
            let mut inner = base.clone();
            inner.push_str(indent_unit);
            let closer = format!("{base}{tail}");
            self.lines.insert(row + 1, inner.clone());
            self.lines.insert(row + 2, closer);
            self.cursor.row = row + 1;
            self.cursor.col = inner.chars().count();
            return;
        }

        let mut new_line = base.clone();
        if opens {
            new_line.push_str(indent_unit);
        }
        let col = new_line.chars().count();
        new_line.push_str(&after);
        self.lines.insert(row + 1, new_line);
        self.cursor.row = row + 1;
        self.cursor.col = col;
    }

    /// Position of the bracket matching the one immediately before the caret.
    ///
    /// Mirrors Xcode: putting the caret after EITHER half of a pair points out
    /// the other half — after `{` it finds the `}`, after `}` it finds the `{`.
    /// Scanning is bounded because this runs on every compose; an unbalanced
    /// file must not turn it into a whole-document walk.
    pub fn matching_bracket_before_cursor(&self) -> Option<Position> {
        // Either side counts: the caret may sit before, on, or after the
        // delimiter and still be "at" it as far as the user is concerned.
        // Prefer the character behind the caret (Xcode's own bias), then the
        // one in front.
        if let Some(p) = self.match_for_char_before_at(self.cursor) {
            return Some(p);
        }
        // Caret sits just BEFORE a delimiter: shift one right and retry, so
        // clicking directly onto a bracket lights its partner too.
        //
        // The retry used to be `self.clone()` with the column bumped, which
        // copied every line in the document to move a cursor one character.
        // The compositor asks this question once per visible row, so a caret
        // parked in front of a bracket cloned the whole file 240 times per
        // draw — 17.5 ms at 6k lines, 43.6 ms at 20k, straight onto the main
        // thread ahead of CoreText. The scanners take the position instead.
        let ahead = self.char_after_cursor()?;
        if !matches!(ahead, '(' | '[' | '{' | ')' | ']' | '}') {
            return None;
        }
        self.match_for_char_before_at(Position::new(self.cursor.row, self.cursor.col + 1))
    }

    fn char_before(&self, at: Position) -> Option<char> {
        if at.col > 0 {
            self.line(at.row).chars().nth(at.col - 1)
        } else {
            None
        }
    }

    fn match_for_char_before_at(&self, at: Position) -> Option<Position> {
        match self.char_before(at)? {
            ')' | ']' | '}' => self.scan_back_for_opener(at),
            '(' | '[' | '{' => self.scan_forward_for_closer(at),
            _ => None,
        }
    }

    fn scan_forward_for_closer(&self, at: Position) -> Option<Position> {
        const MAX_ROWS: usize = 400;
        const MAX_CHARS: usize = 20_000;

        let opener = self.char_before(at)?;
        let closer = match opener {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            _ => return None,
        };

        let mut depth = 0usize;
        let mut scanned = 0usize;
        let first_row = at.row;
        let mut row = first_row;
        let mut col = at.col;

        loop {
            let chars: Vec<char> = self.line(row).chars().collect();
            let mut i = if row == first_row { col } else { 0 };
            while i < chars.len() {
                scanned += 1;
                if scanned > MAX_CHARS {
                    return None;
                }
                let ch = chars[i];
                if ch == opener {
                    depth += 1;
                } else if ch == closer {
                    if depth == 0 {
                        return Some(Position::new(row, i));
                    }
                    depth -= 1;
                }
                i += 1;
            }
            row += 1;
            if row >= self.lines.len() || row.saturating_sub(first_row) >= MAX_ROWS {
                return None;
            }
            col = 0;
        }
    }

    fn scan_back_for_opener(&self, at: Position) -> Option<Position> {
        const MAX_ROWS: usize = 400;
        const MAX_CHARS: usize = 20_000;

        let closer = self.char_before(at)?;
        let opener = match closer {
            ')' => '(',
            ']' => '[',
            '}' => '{',
            _ => return None,
        };

        let mut depth = 0usize;
        let mut scanned = 0usize;
        // Start just before the closer itself.
        let mut row = at.row;
        let mut col = at.col.saturating_sub(1);
        let first_row = row;

        loop {
            let chars: Vec<char> = self.line(row).chars().collect();
            let mut i = if row == first_row { col } else { chars.len() };
            while i > 0 {
                i -= 1;
                scanned += 1;
                if scanned > MAX_CHARS {
                    return None;
                }
                let ch = chars[i];
                if ch == closer {
                    depth += 1;
                } else if ch == opener {
                    if depth == 0 {
                        return Some(Position::new(row, i));
                    }
                    depth -= 1;
                }
            }
            if row == 0 || first_row.saturating_sub(row) >= MAX_ROWS {
                return None;
            }
            row -= 1;
            col = 0;
        }
    }

    pub fn leading_indent(&self, row: usize) -> String {
        let line = self.line(row);
        let indent_len = line.chars().take_while(|c| c.is_whitespace()).count();
        line.chars().take(indent_len).collect()
    }

    /// Delete the grapheme cluster immediately before the caret.
    pub fn backspace(&mut self) {
        self.touch();
        if self.cursor.col > 0 {
            let line = self.lines[self.cursor.row].clone();
            let prev = grapheme_prev_col(&line, self.cursor.col);
            let byte0 = char_to_byte(prev, &line);
            let byte1 = char_to_byte(self.cursor.col, &line);
            self.lines[self.cursor.row].drain(byte0..byte1);
            self.cursor.col = prev;
        } else if self.cursor.row > 0 {
            let moved_line = self.lines.remove(self.cursor.row);
            self.cursor.row -= 1;
            let prev_line_len = self.line(self.cursor.row).chars().count();
            let prev_line = &mut self.lines[self.cursor.row];
            prev_line.push_str(&moved_line);
            self.cursor.col = prev_line_len;
        }
    }

    /// Delete the grapheme cluster at/after the caret.
    pub fn delete_char_at_cursor(&mut self) {
        self.touch();
        let line = self.lines[self.cursor.row].clone();
        let line_len = line.chars().count();
        if self.cursor.col < line_len {
            let next = grapheme_next_col(&line, self.cursor.col);
            let byte0 = char_to_byte(self.cursor.col, &line);
            let byte1 = char_to_byte(next, &line);
            self.lines[self.cursor.row].drain(byte0..byte1);
        } else if self.cursor.row < self.lines.len() - 1 {
            let next_line = self.lines.remove(self.cursor.row + 1);
            self.lines[self.cursor.row].push_str(&next_line);
        }
    }

    pub fn delete_line(&mut self) -> String {
        self.touch();
        if self.lines.len() == 1 {
            let line = std::mem::take(&mut self.lines[0]);
            self.cursor.col = 0;
            return line;
        }
        let line = self.lines.remove(self.cursor.row);
        if self.cursor.row >= self.lines.len() {
            self.cursor.row = self.lines.len() - 1;
        }
        self.clamp_col();
        line
    }

    pub fn delete_word(&mut self) -> String {
        self.touch();
        let chars: Vec<char> = self.line(self.cursor.row).chars().collect();
        if self.cursor.col >= chars.len() {
            return String::new();
        }

        let start = self.cursor.col;
        let mut end = start;

        let class = char_class(chars[start]);
        while end < chars.len() && char_class(chars[end]) == class {
            end += 1;
        }

        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }

        let deleted: String = chars[start..end].iter().collect();
        let line = &mut self.lines[self.cursor.row];
        let start_byte = char_to_byte(start, line);
        let end_byte = char_to_byte(end, line);
        line.drain(start_byte..end_byte);
        self.clamp_col();
        deleted
    }

    pub fn paste_line_after(&mut self, text: &str) {
        self.touch();
        self.lines.insert(self.cursor.row + 1, text.to_string());
        self.cursor.row += 1;
        self.cursor.col = 0;
    }

    pub fn move_word_forward(&mut self) {
        let chars: Vec<char> = self.line(self.cursor.row).chars().collect();
        let mut pos = self.cursor.col;

        if pos >= chars.len() {
            if self.cursor.row < self.lines.len() - 1 {
                self.cursor.row += 1;
                self.cursor.col = 0;
                let new_chars: Vec<char> = self.line(self.cursor.row).chars().collect();
                let mut p = 0;
                while p < new_chars.len() && char_class(new_chars[p]) == CharClass::Whitespace {
                    p += 1;
                }
                self.cursor.col = p;
            }
            return;
        }

        let current_class = char_class(chars[pos]);
        while pos < chars.len() && char_class(chars[pos]) == current_class {
            pos += 1;
        }

        while pos < chars.len() && char_class(chars[pos]) == CharClass::Whitespace {
            pos += 1;
        }

        if pos >= chars.len() && self.cursor.row < self.lines.len() - 1 {
            self.cursor.row += 1;
            self.cursor.col = 0;
            let new_chars: Vec<char> = self.line(self.cursor.row).chars().collect();
            let mut p = 0;
            while p < new_chars.len() && char_class(new_chars[p]) == CharClass::Whitespace {
                p += 1;
            }
            self.cursor.col = p;
        } else {
            self.cursor.col = pos;
        }
    }

    pub fn move_word_back(&mut self) {
        if self.cursor.col == 0 {
            if self.cursor.row > 0 {
                self.cursor.row -= 1;
                self.cursor.col = self.current_line_len();
            }
            return;
        }

        let chars: Vec<char> = self.line(self.cursor.row).chars().collect();
        let mut pos = self.cursor.col;

        pos = pos.saturating_sub(1);

        while pos > 0 && char_class(chars[pos]) == CharClass::Whitespace {
            pos -= 1;
        }

        if pos == 0 && char_class(chars[0]) == CharClass::Whitespace {
            self.cursor.col = 0;
            return;
        }

        let target_class = char_class(chars[pos]);
        while pos > 0 && char_class(chars[pos - 1]) == target_class {
            pos -= 1;
        }

        self.cursor.col = pos;
    }

    /// Monotonic text version (constructor + every mutation get fresh values).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Bump the version counter — used by tests to simulate edit deltas
    /// without going through the full insert/delete path.
    #[doc(hidden)]
    pub fn set_version(&mut self, v: u64) {
        self.version = v;
    }

    /// Mark the text as changed.
    pub fn touch(&mut self) {
        self.version = next_version();
    }

    pub fn snapshot(&self) -> BufferSnapshot {
        BufferSnapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
            version: self.version,
        }
    }

    pub fn restore(&mut self, snapshot: &BufferSnapshot) {
        self.touch();
        self.lines = snapshot.lines.clone();
        self.cursor = snapshot.cursor;
    }

    // ── Document offsets (phase-1 Edit/Delta support) ──────────────────

    /// Absolute char offset from document start → position. Offsets past
    /// the end clamp to the document end. O(lines): the line index replaces
    /// this scan in the phase-2 document rewrite.
    pub fn offset_to_position(&self, offset: usize) -> Position {
        let mut remaining = offset;
        for (row, line) in self.lines.iter().enumerate() {
            let chars = line.chars().count();
            if remaining <= chars {
                return Position {
                    row,
                    col: remaining,
                };
            }
            remaining -= chars + 1; // +1: the line break
        }
        let row = self.lines.len().saturating_sub(1);
        Position {
            row,
            col: self.lines[row].chars().count(),
        }
    }

    /// Position → absolute char offset from document start (col clamped to
    /// the line length).
    pub fn position_to_offset(&self, pos: Position) -> usize {
        let mut off = 0;
        for (row, line) in self.lines.iter().enumerate() {
            if row == pos.row {
                return off + pos.col.min(line.chars().count());
            }
            off += line.chars().count() + 1;
        }
        off
    }

    /// Total document length in chars, line breaks included.
    pub fn len_chars(&self) -> usize {
        self.lines
            .iter()
            .map(|l| l.chars().count() + 1)
            .sum::<usize>()
            .saturating_sub(1)
    }

    /// Apply an edit atomically and return the delta between versions.
    /// Changes carry offsets in the CURRENT version and are applied
    /// back-to-front (largest offset first) so earlier offsets stay valid;
    /// the returned delta lists them in ascending document order with the
    /// ACTUAL removed text. The cursor is preserved — callers move it
    /// themselves. An empty edit does not bump the version.
    pub fn apply_edit(&mut self, edit: &crate::edit::Edit) -> crate::edit::Delta {
        use crate::edit::{Change, Delta};
        let version_before = self.version;
        if edit.changes.is_empty() {
            return Delta {
                version_before,
                version_after: version_before,
                changes: Vec::new(),
                cursor_before: self.cursor,
                cursor_after: self.cursor,
            };
        }
        let saved_cursor = self.cursor;
        let mut applied: Vec<Change> = Vec::with_capacity(edit.changes.len());
        let mut order: Vec<&Change> = edit.changes.iter().collect();
        order.sort_by(|a, b| b.start.cmp(&a.start));
        for c in order {
            let start = self.offset_to_position(c.start);
            let end = self.offset_to_position(c.start + c.old_len());
            let old = self.delete_range(start, end); // cursor lands at `start`
            if !c.new.is_empty() {
                self.insert_str(&c.new); // inserts at the cursor (= start)
            }
            applied.push(Change {
                start: c.start,
                old,
                new: c.new.clone(),
            });
        }
        applied.sort_by_key(|c| c.start);
        self.cursor = saved_cursor;
        // The primitives touch per change; one edit is one version.
        self.touch();
        Delta {
            version_before,
            version_after: self.version,
            changes: applied,
            // apply_edit preserves the cursor; callers that move it record
            // the pair through the undo stack's checkpoint diff instead.
            cursor_before: saved_cursor,
            cursor_after: saved_cursor,
        }
    }

    // ── In-line character search (char indices, UTF-8 safe) ──

    pub fn find_char_forward(&mut self, ch: char) {
        let chars: Vec<char> = self.line(self.cursor.row).chars().collect();
        if self.cursor.col + 1 >= chars.len() {
            return;
        }
        if let Some(rel) = chars[self.cursor.col + 1..].iter().position(|c| *c == ch) {
            self.cursor.col = self.cursor.col + 1 + rel;
        }
    }

    pub fn find_char_backward(&mut self, ch: char) {
        if self.cursor.col == 0 {
            return;
        }
        let chars: Vec<char> = self.line(self.cursor.row).chars().collect();
        let end = self.cursor.col.min(chars.len());
        if let Some(pos) = chars[..end].iter().rposition(|c| *c == ch) {
            self.cursor.col = pos;
        }
    }

    pub fn till_char_forward(&mut self, ch: char) {
        let chars: Vec<char> = self.line(self.cursor.row).chars().collect();
        if self.cursor.col + 1 >= chars.len() {
            return;
        }
        if let Some(rel) = chars[self.cursor.col + 1..].iter().position(|c| *c == ch) {
            if rel > 0 {
                self.cursor.col = self.cursor.col + rel;
            }
        }
    }

    pub fn till_char_backward(&mut self, ch: char) {
        if self.cursor.col <= 1 {
            return;
        }
        let chars: Vec<char> = self.line(self.cursor.row).chars().collect();
        let end = self.cursor.col.saturating_sub(1).min(chars.len());
        if let Some(pos) = chars[..end].iter().rposition(|c| *c == ch) {
            self.cursor.col = pos + 1;
        }
    }

    // ── Replace ────────────────────────────────────────

    pub fn replace_char(&mut self, ch: char) {
        self.touch();
        let line = &self.lines[self.cursor.row];
        let len = line.chars().count();
        if self.cursor.col >= len {
            return;
        }
        let start = char_to_byte(self.cursor.col, line);
        let end = char_to_byte(self.cursor.col + 1, line);
        let mut new_line = String::new();
        new_line.push_str(&line[..start]);
        new_line.push(ch);
        new_line.push_str(&line[end..]);
        self.lines[self.cursor.row] = new_line;
    }

    // ── Indent / Dedent ────────────────────────────────

    pub fn indent_line(&mut self) {
        self.touch();
        self.lines[self.cursor.row].insert_str(0, "    ");
        self.cursor.col += 4;
    }

    pub fn dedent_line(&mut self) {
        self.touch();
        let line = &self.lines[self.cursor.row];
        if line.starts_with("    ") {
            self.lines[self.cursor.row] = line[4..].to_string();
            self.cursor.col = self.cursor.col.saturating_sub(4);
        } else if line.starts_with(' ') {
            let spaces = line.chars().take_while(|c| *c == ' ').count().min(4);
            let byte = char_to_byte(spaces, line);
            self.lines[self.cursor.row] = line[byte..].to_string();
            self.cursor.col = self.cursor.col.saturating_sub(spaces);
        } else if line.starts_with('\t') {
            self.lines[self.cursor.row] = line[1..].to_string();
            self.cursor.col = self.cursor.col.saturating_sub(1);
        }
    }

    // ── Join lines ─────────────────────────────────────

    pub fn join_lines(&mut self) {
        self.touch();
        if self.cursor.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor.row + 1);
            let next_trim = next.trim_start();
            let current_len = self.lines[self.cursor.row].chars().count();
            let cur = &mut self.lines[self.cursor.row];
            if !cur.is_empty() && !cur.ends_with(' ') && !next_trim.is_empty() {
                cur.push(' ');
            }
            cur.push_str(next_trim);
            self.cursor.col = if next_trim.is_empty() {
                current_len
            } else if current_len > 0 {
                current_len // space is at current_len if we added one... simplified:
            } else {
                0
            };
            // Place cursor on the joining space (vim-like)
            if !next_trim.is_empty() && current_len > 0 {
                self.cursor.col = current_len; // on the space we may have inserted
            }
        }
    }

    // ── First non-blank ────────────────────────────────

    pub fn move_to_first_non_blank(&mut self) {
        let line = self.line(self.cursor.row);
        self.cursor.col = line
            .chars()
            .position(|c| c != ' ' && c != '\t')
            .unwrap_or(0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Word,
    Punctuation,
}

fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

#[derive(Clone)]
pub struct BufferSnapshot {
    lines: Vec<String>,
    cursor: Position,
    /// The document version this snapshot captured — consumers tag their
    /// derived results with it and drop anything stale.
    version: u64,
}

impl BufferSnapshot {
    pub fn lines(&self) -> &[String] {
        &self.lines
    }
    pub fn cursor(&self) -> Position {
        self.cursor
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn from_parts(lines: Vec<String>, cursor: Position) -> Self {
        Self {
            lines,
            cursor,
            version: next_version(),
        }
    }
}

fn char_to_byte(char_idx: usize, line: &str) -> usize {
    line.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

/// Char-index of the start of the grapheme cluster that ends at or before `col`.
/// If `col` is mid-cluster, snaps to that cluster's start.
pub fn grapheme_prev_col(line: &str, col: usize) -> usize {
    if col == 0 {
        return 0;
    }
    let mut starts = Vec::new();
    let mut char_i = 0usize;
    starts.push(0);
    for g in line.graphemes(true) {
        char_i += g.chars().count();
        starts.push(char_i);
    }
    // Largest start strictly less than col (or equal only if col lands mid-cluster: still previous).
    starts
        .into_iter()
        .filter(|&s| s < col)
        .next_back()
        .unwrap_or(0)
}

/// Char-index just past the grapheme cluster that contains or starts at `col`.
pub fn grapheme_next_col(line: &str, col: usize) -> usize {
    let max = line.chars().count();
    if col >= max {
        return max;
    }
    let mut char_i = 0usize;
    for g in line.graphemes(true) {
        let next = char_i + g.chars().count();
        if col < next {
            return next;
        }
        char_i = next;
    }
    max
}

/// True when `col` is not at a grapheme boundary (mid-cluster).
#[cfg(test)]
fn col_splits_grapheme(line: &str, col: usize) -> bool {
    if col == 0 || col >= line.chars().count() {
        return false;
    }
    let mut char_i = 0usize;
    for g in line.graphemes(true) {
        let next = char_i + g.chars().count();
        if char_i < col && col < next {
            return true;
        }
        char_i = next;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_buffer_has_one_empty_line() {
        let buf = Buffer::new();
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.line(0), "");
    }

    #[test]
    fn test_from_string_multiline() {
        let buf = Buffer::from_string("hello\nworld");
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.line(0), "hello");
        assert_eq!(buf.line(1), "world");
    }

    #[test]
    fn test_insert_char() {
        let mut buf = Buffer::new();
        buf.insert_char('h');
        buf.insert_char('i');
        assert_eq!(buf.line(0), "hi");
        assert_eq!(buf.cursor.col, 2);
    }

    #[test]
    fn test_insert_newline() {
        let mut buf = Buffer::from_string("hello");
        buf.cursor = Position::new(0, 2);
        buf.insert_newline();
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.line(0), "he");
        assert_eq!(buf.line(1), "llo");
        assert_eq!(buf.cursor, Position::new(1, 0));
    }

    #[test]
    fn test_backspace_merge_lines() {
        let mut buf = Buffer::from_string("he\nllo");
        buf.cursor = Position::new(1, 0);
        buf.backspace();
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.line(0), "hello");
        assert_eq!(buf.cursor, Position::new(0, 2));
    }

    #[test]
    fn test_move_left_right() {
        let mut buf = Buffer::from_string("abc");
        buf.cursor.col = 1;
        buf.move_left();
        assert_eq!(buf.cursor.col, 0);
        buf.move_left();
        assert_eq!(buf.cursor.col, 0);
        buf.move_right();
        assert_eq!(buf.cursor.col, 1);
    }

    #[test]
    fn test_move_up_down_clamps_col() {
        let mut buf = Buffer::from_string("long line\nx");
        buf.cursor = Position::new(0, 5);
        buf.move_down();
        assert_eq!(buf.cursor.row, 1);
        assert_eq!(buf.cursor.col, 1);
    }

    #[test]
    fn test_delete_char_at_cursor() {
        let mut buf = Buffer::from_string("abc");
        buf.cursor.col = 0;
        buf.delete_char_at_cursor();
        assert_eq!(buf.line(0), "bc");
    }

    #[test]
    fn test_unicode_insert_and_delete() {
        let mut buf = Buffer::new();
        buf.insert_char('ä');
        buf.insert_char('o');
        buf.insert_char('\u{3042}');
        assert_eq!(buf.line(0), "äoあ");
        assert_eq!(buf.cursor.col, 3);
        buf.backspace();
        assert_eq!(buf.line(0), "äo");
    }

    #[test]
    fn test_text_output() {
        let buf = Buffer::from_string("line1\nline2\nline3");
        assert_eq!(buf.text(), "line1\nline2\nline3");
    }

    #[test]
    fn test_delete_line() {
        let mut buf = Buffer::from_string("a\nb\nc");
        buf.cursor.row = 1;
        let deleted = buf.delete_line();
        assert_eq!(deleted, "b");
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.line(0), "a");
        assert_eq!(buf.line(1), "c");
    }

    #[test]
    fn test_delete_last_line() {
        let mut buf = Buffer::from_string("a\nb");
        buf.cursor.row = 1;
        let deleted = buf.delete_line();
        assert_eq!(deleted, "b");
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.line(0), "a");
        assert_eq!(buf.cursor.row, 0);
    }

    #[test]
    fn test_delete_only_line() {
        let mut buf = Buffer::from_string("hello");
        let deleted = buf.delete_line();
        assert_eq!(deleted, "hello");
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.line(0), "");
    }

    #[test]
    fn test_delete_word() {
        let mut buf = Buffer::from_string("hello world");
        buf.cursor.col = 0;
        let deleted = buf.delete_word();
        assert_eq!(deleted, "hello ");
        assert_eq!(buf.line(0), "world");
    }

    #[test]
    fn test_delete_word_punctuation() {
        let mut buf = Buffer::from_string("foo = bar");
        buf.cursor.col = 4;
        let deleted = buf.delete_word();
        assert_eq!(deleted, "= ");
        assert_eq!(buf.line(0), "foo bar");
    }

    #[test]
    fn test_move_word_forward() {
        let mut buf = Buffer::from_string("hello world foo");
        buf.cursor.col = 0;
        buf.move_word_forward();
        assert_eq!(buf.cursor.col, 6);
        buf.move_word_forward();
        assert_eq!(buf.cursor.col, 12);
    }

    #[test]
    fn test_move_word_back() {
        let mut buf = Buffer::from_string("hello world foo");
        buf.cursor.col = 12;
        buf.move_word_back();
        assert_eq!(buf.cursor.col, 6);
        buf.move_word_back();
        assert_eq!(buf.cursor.col, 0);
    }

    #[test]
    fn test_paste_line_after() {
        let mut buf = Buffer::from_string("a\nc");
        buf.paste_line_after("b");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.line(0), "a");
        assert_eq!(buf.line(1), "b");
        assert_eq!(buf.line(2), "c");
        assert_eq!(buf.cursor.row, 1);
    }

    #[test]
    fn test_screen_col_to_buffer_col() {
        let buf = Buffer::from_string("\thello");
        assert_eq!(buf.screen_col_to_buffer_col(0, 0), 0);
        assert_eq!(buf.screen_col_to_buffer_col(0, 1), 0);
        assert_eq!(buf.screen_col_to_buffer_col(0, 3), 0);
        assert_eq!(buf.screen_col_to_buffer_col(0, 4), 1);
        assert_eq!(buf.screen_col_to_buffer_col(0, 5), 2);
    }

    #[test]
    fn test_buffer_col_to_screen_col() {
        let buf = Buffer::from_string("\thello");
        assert_eq!(buf.buffer_col_to_screen_col(0, 0), 0);
        assert_eq!(buf.buffer_col_to_screen_col(0, 1), 4);
        assert_eq!(buf.buffer_col_to_screen_col(0, 5), 8);
    }

    #[test]
    fn test_col_roundtrip_tabs() {
        let buf = Buffer::from_string("\t\tfn main()");
        for bc in 0..=buf.line(0).chars().count() {
            let sc = buf.buffer_col_to_screen_col(0, bc);
            let back = buf.screen_col_to_buffer_col(0, sc);
            assert_eq!(back, bc, "roundtrip failed at buf_col={}", bc);
        }
    }

    #[test]
    fn test_col_roundtrip_spaces() {
        let buf = Buffer::from_string("        let x = 1;");
        for bc in 0..=buf.line(0).chars().count() {
            let sc = buf.buffer_col_to_screen_col(0, bc);
            let back = buf.screen_col_to_buffer_col(0, sc);
            assert_eq!(back, bc, "roundtrip failed at buf_col={}", bc);
        }
    }

    #[test]
    fn test_col_roundtrip_cjk() {
        let buf = Buffer::from_string("야르~");
        for bc in 0..=buf.line(0).chars().count() {
            let sc = buf.buffer_col_to_screen_col(0, bc);
            let back = buf.screen_col_to_buffer_col(0, sc);
            assert_eq!(back, bc, "roundtrip failed at buf_col={} for '야르~'", bc);
        }
    }

    #[test]
    fn test_cjk_width() {
        let buf = Buffer::from_string("a한b");
        assert_eq!(buf.buffer_col_to_screen_col(0, 0), 0); // 'a' at col 0
        assert_eq!(buf.buffer_col_to_screen_col(0, 1), 1); // '한' at col 1 → screen col 1
        assert_eq!(buf.buffer_col_to_screen_col(0, 2), 3); // 'b' at col 2 → screen col 3 (한=width 2)
    }
    #[test]
    fn test_col_roundtrip_mixed() {
        let buf = Buffer::from_string("  \t  hello\tworld");
        for bc in 0..=buf.line(0).chars().count() {
            let sc = buf.buffer_col_to_screen_col(0, bc);
            let back = buf.screen_col_to_buffer_col(0, sc);
            assert_eq!(back, bc, "roundtrip failed at buf_col={}", bc);
        }
    }

    #[test]
    fn test_snapshot_restore() {
        let mut buf = Buffer::from_string("hello");
        buf.cursor.col = 5;
        buf.insert_char('!');
        let snap = buf.snapshot();
        buf.insert_char('x');
        assert_eq!(buf.line(0), "hello!x");
        buf.restore(&snap);
        assert_eq!(buf.line(0), "hello!");
    }

    #[test]
    fn test_find_char_utf8() {
        let mut buf = Buffer::from_string("한a글b");
        buf.cursor = Position::new(0, 0);
        buf.find_char_forward('글');
        assert_eq!(buf.cursor.col, 2);
        buf.find_char_forward('b');
        assert_eq!(buf.cursor.col, 3);
        buf.find_char_backward('a');
        assert_eq!(buf.cursor.col, 1);
    }

    #[test]
    fn test_replace_char_utf8() {
        let mut buf = Buffer::from_string("한x글");
        buf.cursor = Position::new(0, 1);
        buf.replace_char('야');
        assert_eq!(buf.line(0), "한야글");
    }

    #[test]
    fn test_insert_str_single_line() {
        let mut buf = Buffer::from_string("HELLO");
        buf.cursor = Position::new(0, 2);
        buf.insert_str("xyz");
        assert_eq!(buf.line(0), "HExyzLLO");
        assert_eq!(buf.cursor, Position::new(0, 5));
    }

    #[test]
    fn test_insert_str_multi_line() {
        let mut buf = Buffer::from_string("HELLO");
        buf.cursor = Position::new(0, 2);
        buf.insert_str("a\nb\nc");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.line(0), "HEa");
        assert_eq!(buf.line(1), "b");
        assert_eq!(buf.line(2), "cLLO");
        assert_eq!(buf.cursor, Position::new(2, 1));
    }

    #[test]
    fn test_insert_str_matches_charwise() {
        // Batch insert_str must match the old char-by-char semantics exactly.
        let cases = ["", "abc", "a\n", "\nx", "one\ntwo\nthree", "한\n글", "\n\n"];
        for s in cases {
            let mut batch = Buffer::from_string("PREsuf");
            batch.cursor = Position::new(0, 3);
            batch.insert_str(s);

            let mut naive = Buffer::from_string("PREsuf");
            naive.cursor = Position::new(0, 3);
            for ch in s.chars() {
                if ch == '\n' {
                    naive.insert_newline();
                } else {
                    naive.insert_char(ch);
                }
            }
            assert_eq!(batch.text(), naive.text(), "text mismatch for {s:?}");
            assert_eq!(batch.cursor, naive.cursor, "cursor mismatch for {s:?}");
        }
    }

    #[test]
    fn test_join_lines() {
        let mut buf = Buffer::from_string("hello\nworld");
        buf.cursor = Position::new(0, 0);
        buf.join_lines();
        assert_eq!(buf.line(0), "hello world");
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn enter_between_a_pair_expands_to_three_lines() {
        let mut b = Buffer::from_string("fn main() {}");
        // caret between { and }
        b.cursor = Position::new(0, 11);
        b.insert_newline_smart("    ");
        assert_eq!(b.line(0), "fn main() {");
        assert_eq!(b.line(1), "    ");
        assert_eq!(b.line(2), "}");
        assert_eq!(
            b.cursor(),
            Position::new(1, 4),
            "caret sits on the blank line"
        );
    }

    #[test]
    fn enter_after_an_opener_indents_one_level() {
        let mut b = Buffer::from_string("    if x {");
        b.cursor = Position::new(0, 10);
        b.insert_newline_smart("    ");
        assert_eq!(b.line(1), "        ");
        assert_eq!(b.cursor(), Position::new(1, 8));
    }

    #[test]
    fn enter_mid_line_judges_by_text_before_the_caret() {
        // Whole line ends with `{`, but the caret is BEFORE it — the old code
        // looked at the line end and wrongly indented.
        let mut b = Buffer::from_string("let a = 1; foo {");
        b.cursor = Position::new(0, 10);
        b.insert_newline_smart("    ");
        assert_eq!(b.line(0), "let a = 1;");
        assert_eq!(b.line(1), " foo {");
        assert_eq!(b.cursor(), Position::new(1, 0), "no extra indent");
    }

    #[test]
    fn enter_keeps_plain_indentation() {
        let mut b = Buffer::from_string("        value");
        b.cursor = Position::new(0, 13);
        b.insert_newline_smart("    ");
        assert_eq!(b.line(1), "        ");
        assert_eq!(b.cursor(), Position::new(1, 8));
    }

    #[test]
    fn finds_matching_opener_across_nesting() {
        let mut b = Buffer::from_string("foo(bar(baz))");
        b.cursor = Position::new(0, 12); // just after the inner ')'
        assert_eq!(
            b.matching_bracket_before_cursor(),
            Some(Position::new(0, 7))
        );
        b.cursor = Position::new(0, 13); // just after the outer ')'
        assert_eq!(
            b.matching_bracket_before_cursor(),
            Some(Position::new(0, 3))
        );
    }

    #[test]
    fn finds_matching_opener_across_lines() {
        let mut b = Buffer::from_string("fn a() {\n    x;\n}");
        b.cursor = Position::new(2, 1); // just after the '}'
        assert_eq!(
            b.matching_bracket_before_cursor(),
            Some(Position::new(0, 7))
        );
    }

    #[test]
    fn no_match_when_unbalanced_or_not_a_closer() {
        let mut b = Buffer::from_string("value)");
        b.cursor = Position::new(0, 6);
        assert_eq!(
            b.matching_bracket_before_cursor(),
            None,
            "no opener to find"
        );

        let mut b2 = Buffer::from_string("value");
        b2.cursor = Position::new(0, 5);
        assert_eq!(b2.matching_bracket_before_cursor(), None, "not a closer");
    }

    #[test]
    fn finds_matching_closer_from_an_opener() {
        // Xcode highlights the other half from EITHER side.
        let mut b = Buffer::from_string("fn a() {\n    x;\n}");
        b.cursor = Position::new(0, 8); // just after the '{'
        assert_eq!(
            b.matching_bracket_before_cursor(),
            Some(Position::new(2, 0))
        );

        let mut c = Buffer::from_string("foo(bar(baz))");
        c.cursor = Position::new(0, 4); // just after the outer '('
        assert_eq!(
            c.matching_bracket_before_cursor(),
            Some(Position::new(0, 12))
        );
        c.cursor = Position::new(0, 8); // just after the inner '('
        assert_eq!(
            c.matching_bracket_before_cursor(),
            Some(Position::new(0, 11))
        );
    }

    #[test]
    fn no_forward_match_when_never_closed() {
        let mut b = Buffer::from_string("fn a() {\n    x;");
        b.cursor = Position::new(0, 8);
        assert_eq!(b.matching_bracket_before_cursor(), None);
    }

    #[test]
    fn move_right_left_never_splits_hangul_or_emoji_graphemes() {
        // Precomposed Hangul syllables + ZWJ family emoji (multi-codepoint grapheme).
        let text = "안녕👋a👨‍👩‍👧‍👦끝";
        let mut b = Buffer::from_string(text);
        b.cursor = Position::zero();
        let max = b.current_line_len();
        let mut seen = Vec::new();
        // Walk right across the whole line.
        loop {
            let col = b.cursor.col;
            assert!(
                !col_splits_grapheme(text, col),
                "caret col {col} splits a grapheme in {text:?}"
            );
            seen.push(col);
            if col >= max {
                break;
            }
            let before = col;
            b.move_right();
            assert!(b.cursor.col > before, "move_right must advance");
        }
        // Walk left back to start.
        loop {
            let col = b.cursor.col;
            assert!(!col_splits_grapheme(text, col), "leftward split at {col}");
            if col == 0 {
                break;
            }
            let before = col;
            b.move_left();
            assert!(b.cursor.col < before, "move_left must retreat");
        }
        assert_eq!(b.cursor.col, 0);
        // At least Hangul + emoji + letter should produce several stops.
        assert!(
            seen.len() >= 5,
            "expected multiple grapheme stops, got {seen:?}"
        );
    }

    #[test]
    fn backspace_deletes_whole_emoji_grapheme() {
        let mut b = Buffer::from_string("x👨‍👩‍👧‍👦y");
        // Caret on 'y' (after the family emoji grapheme).
        b.cursor = Position::zero();
        while b.cursor.col < b.current_line_len() && b.char_after_cursor() != Some('y') {
            b.move_right();
        }
        assert_eq!(b.char_after_cursor(), Some('y'));
        b.backspace();
        assert_eq!(
            b.line(0),
            "xy",
            "one backspace must remove the whole family emoji"
        );
    }

    #[test]
    fn delete_removes_hangul_syllable_as_one_unit() {
        let mut b = Buffer::from_string("한a");
        b.cursor = Position::zero();
        b.delete_char_at_cursor();
        assert_eq!(b.line(0), "a");
        assert_eq!(b.cursor.col, 0);
    }
}
