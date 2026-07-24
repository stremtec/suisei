//! Built-in terminal emulator with a **real PTY**.
//!
//! Uses `portable-pty` so child processes (opencode, claude, vim, …) get a
//! genuine tty, correct `TIOCSWINSZ` on resize, and SIGWINCH. Pairs that with
//! UTF-8 / CSI / OSC parsing and an alternate screen buffer for full-screen TUIs.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use unicode_width::UnicodeWidthChar;

pub struct Terminal {
    pub open: bool,
    /// When true, terminal fills an editor slot (Ctrl+Shift+T)
    /// instead of the side panel (Ctrl+T).
    pub full_panel: bool,
    /// When split is active, full terminal is bound to this pane index only
    /// (`None` = fill the whole main editor area).
    pub pane_bound: Option<usize>,
    /// Ctrl+Shift+T created the split it lives in (single view before) —
    /// closing the terminal must collapse that split back too.
    pub owns_split: bool,
    /// Screen contents changed since the last `take_damage()` — lets faces
    /// repaint only when the PTY actually produced output instead of
    /// unconditionally every tick (that 20Hz republish made editor scrolling
    /// stutter whenever a terminal was open).
    damage: bool,
    rows: Vec<Vec<Cell>>,
    /// Saved primary buffer while alternate screen is active.
    saved_primary: Option<SavedScreen>,
    alt_screen: bool,
    cursor_row: usize,
    cursor_col: usize,
    saved_cursor: (usize, usize),
    cols: u16,
    rows_count: u16,
    scroll_offset: usize,
    /// Inner app enabled mouse reporting (CSI ?1000/1002/1003 h).
    mouse_reporting: bool,
    /// DECCKM — application cursor keys (arrows as ESC O A..D).
    app_cursor_keys: bool,
    /// Child enabled bracketed paste (DECSET 2004) — wrap forwarded pastes.
    bracketed_paste: bool,
    /// PTY master — kept alive for `resize` (TIOCSWINSZ + SIGWINCH).
    master: Option<Box<dyn MasterPty + Send>>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    writer: Option<Box<dyn Write + Send>>,
    rx: Option<Receiver<Vec<u8>>>,
    scrollback: Vec<Vec<Cell>>,
    fg: Color,
    bg: Color,
    bold: bool,
    reverse: bool,
    /// Incomplete UTF-8 / escape sequence from the previous poll chunk.
    pending: Vec<u8>,
    pub started: bool,
    /// Full/pane terminal: Esc asked once — wait for y/n before closing.
    pub close_confirm: bool,
}

struct SavedScreen {
    rows: Vec<Vec<Cell>>,
    cursor_row: usize,
    cursor_col: usize,
    scrollback: Vec<Vec<Cell>>,
}

#[derive(Clone)]
struct Cell {
    ch: char,
    fg: Option<Color>,
    bg: Option<Color>,
}

impl Cell {
    fn blank() -> Self {
        Self {
            ch: ' ',
            fg: None,
            bg: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Rgb(u8, u8, u8),
}

impl Color {
    fn to_ratatui(self) -> ratatui::style::Color {
        match self {
            // Default → pure black so agent TUIs don't sit on a grey "frame"
            Color::Default => ratatui::style::Color::Rgb(0, 0, 0),
            Color::Black => ratatui::style::Color::Rgb(0, 0, 0),
            Color::Red => ratatui::style::Color::Red,
            Color::Green => ratatui::style::Color::Green,
            Color::Yellow => ratatui::style::Color::Yellow,
            Color::Blue => ratatui::style::Color::Blue,
            Color::Magenta => ratatui::style::Color::Magenta,
            Color::Cyan => ratatui::style::Color::Cyan,
            Color::White => ratatui::style::Color::White,
            Color::BrightBlack => ratatui::style::Color::Gray,
            Color::BrightRed => ratatui::style::Color::LightRed,
            Color::BrightGreen => ratatui::style::Color::LightGreen,
            Color::BrightYellow => ratatui::style::Color::LightYellow,
            Color::BrightBlue => ratatui::style::Color::LightBlue,
            Color::BrightMagenta => ratatui::style::Color::LightMagenta,
            Color::BrightCyan => ratatui::style::Color::LightCyan,
            Color::BrightWhite => ratatui::style::Color::White,
            Color::Rgb(r, g, b) => ratatui::style::Color::Rgb(r, g, b),
        }
    }
}

fn blank_grid(cols: u16, rows: u16) -> Vec<Vec<Cell>> {
    vec![vec![Cell::blank(); cols as usize]; rows as usize]
}

impl Default for Terminal {
    fn default() -> Self {
        let (cols, rows) = (80, 24);
        Self {
            open: false,
            full_panel: false,
            pane_bound: None,
            owns_split: false,
            damage: false,
            rows: blank_grid(cols, rows),
            saved_primary: None,
            alt_screen: false,
            cursor_row: 0,
            cursor_col: 0,
            saved_cursor: (0, 0),
            cols,
            rows_count: rows,
            scroll_offset: 0,
            mouse_reporting: false,
            app_cursor_keys: false,
            bracketed_paste: false,
            master: None,
            child: None,
            writer: None,
            rx: None,
            scrollback: Vec::new(),
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            reverse: false,
            pending: Vec::new(),
            started: false,
            close_confirm: false,
        }
    }
}

impl Terminal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }
    pub fn rows_count(&self) -> u16 {
        self.rows_count
    }

    /// Resize the virtual screen **and** the real PTY (TIOCSWINSZ → SIGWINCH).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = cols.max(2);
        let rows = rows.max(2);
        if cols == self.cols && rows == self.rows_count {
            // Still push size to PTY in case we started before first paint
            // already matched — no-op when equal.
            return;
        }
        self.resize_grid(cols, rows);
        self.damage = true;
        if let Some(ref master) = self.master {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    fn resize_grid(&mut self, cols: u16, rows: u16) {
        let resize_buf = |grid: &mut Vec<Vec<Cell>>| {
            let mut new_rows = Vec::with_capacity(rows as usize);
            for r in 0..rows as usize {
                let mut row = if r < grid.len() {
                    let mut old = grid[r].clone();
                    old.resize(cols as usize, Cell::blank());
                    old.truncate(cols as usize);
                    old
                } else {
                    vec![Cell::blank(); cols as usize]
                };
                if row.len() != cols as usize {
                    row.resize(cols as usize, Cell::blank());
                }
                new_rows.push(row);
            }
            *grid = new_rows;
        };
        resize_buf(&mut self.rows);
        if let Some(ref mut saved) = self.saved_primary {
            resize_buf(&mut saved.rows);
            saved.cursor_row = saved.cursor_row.min(rows as usize - 1);
            saved.cursor_col = saved.cursor_col.min(cols as usize - 1);
        }
        self.cols = cols;
        self.rows_count = rows;
        self.cursor_row = self.cursor_row.min(rows as usize - 1);
        self.cursor_col = self.cursor_col.min(cols as usize - 1);
    }

    /// Open a real PTY and spawn the user shell at the current grid size.
    pub fn start(&mut self, anchor: Option<&PathBuf>) {
        if self.started {
            return;
        }

        let cwd = anchor
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let shell = std::env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(windows) {
                "powershell.exe".into()
            } else {
                "/bin/zsh".into()
            }
        });

        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows: self.rows_count,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(_) => {
                // Last-resort: stay closed rather than a broken half-state
                return;
            }
        };

        let mut cmd = CommandBuilder::new(&shell);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("COLUMNS", self.cols.to_string());
        cmd.env("LINES", self.rows_count.to_string());
        // Avoid inheriting host kitty/graphics state into nested agents
        cmd.env_remove("KITTY_WINDOW_ID");
        cmd.env_remove("WEZTERM_PANE");

        let child = match pair.slave.spawn_command(cmd) {
            Ok(c) => c,
            Err(_) => return,
        };

        let mut reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(_) => return,
        };
        let writer = match pair.master.take_writer() {
            Ok(w) => w,
            Err(_) => return,
        };

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        self.master = Some(pair.master);
        self.child = Some(child);
        self.writer = Some(writer);
        self.rx = Some(rx);
        self.open = true;
        self.started = true;
        self.pending.clear();
        self.alt_screen = false;
        self.saved_primary = None;
        self.rows = blank_grid(self.cols, self.rows_count);
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scrollback.clear();
        self.scroll_offset = 0;
        self.mouse_reporting = false;
        self.app_cursor_keys = false;
        self.bracketed_paste = false;
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.bold = false;
        self.reverse = false;
    }

    pub fn shutdown(&mut self) {
        // Drop writer first → EOF to slave
        self.writer = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.master = None;
        self.rx = None;
        self.started = false;
        self.open = false;
        self.full_panel = false;
        self.pane_bound = None;
        self.close_confirm = false;
        self.pending.clear();
        self.alt_screen = false;
        self.saved_primary = None;
        self.rows = blank_grid(self.cols, self.rows_count);
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scrollback.clear();
        self.scroll_offset = 0;
        self.mouse_reporting = false;
        self.app_cursor_keys = false;
        self.bracketed_paste = false;
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.bold = false;
        self.reverse = false;
    }

    pub fn write_input(&mut self, bytes: &[u8]) {
        // Typing snaps the view back to the live prompt.
        self.scroll_offset = 0;
        if let Some(ref mut w) = self.writer {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    /// Forward pasted text to the child. When the child enabled bracketed paste
    /// (DECSET 2004), wrap it in `\x1b[200~ … \x1b[201~` so TUIs (claude, vim,
    /// fish) treat it as one paste — not typed keys that auto-submit on newline.
    /// This is what makes Cmd+V and file drag-drop deliver a path/blob intact.
    pub fn paste_input(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.bracketed_paste {
            let mut buf = Vec::with_capacity(text.len() + 12);
            buf.extend_from_slice(b"\x1b[200~");
            buf.extend_from_slice(text.as_bytes());
            buf.extend_from_slice(b"\x1b[201~");
            self.write_input(&buf);
        } else {
            self.write_input(text.as_bytes());
        }
    }

    pub fn poll(&mut self) {
        let data = if let Some(ref rx) = self.rx {
            let mut all = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(part) => all.extend_from_slice(&part),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            if all.is_empty() {
                return;
            }
            all
        } else {
            return;
        };
        if std::env::var_os("SUISEI_TERM_DEBUG").is_some() {
            eprintln!(
                "[term-dbg] poll {}B cur=({},{}) grid={}x{}: {:?}",
                data.len(),
                self.cursor_row,
                self.cursor_col,
                self.cols,
                self.rows_count,
                String::from_utf8_lossy(&data[..data.len().min(120)])
            );
        }
        self.process_output(&data);
        self.damage = true;
    }

    /// True once if the screen changed since the last call.
    pub fn take_damage(&mut self) -> bool {
        std::mem::take(&mut self.damage)
    }

    fn process_output(&mut self, data: &[u8]) {
        self.pending.extend_from_slice(data);
        let buf = std::mem::take(&mut self.pending);
        let mut i = 0;
        while i < buf.len() {
            match self.try_consume(&buf, i) {
                Consume::Advanced(n) => i = n,
                Consume::NeedMore => {
                    self.pending = buf[i..].to_vec();
                    if self.pending.len() > 8192 {
                        self.pending.clear();
                    }
                    break;
                }
            }
        }
    }

    fn try_consume(&mut self, data: &[u8], i: usize) -> Consume {
        let b = data[i];
        if b == 0x1b {
            if i + 1 >= data.len() {
                return Consume::NeedMore;
            }
            let n = data[i + 1];
            match n {
                b'[' => return self.consume_csi(data, i + 2),
                b']' => return self.consume_osc(data, i + 2),
                b'P' | b'X' | b'^' | b'_' => return self.consume_string_seq(data, i + 2),
                b'\\' => return Consume::Advanced(i + 2),
                b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/' => {
                    if i + 2 >= data.len() {
                        return Consume::NeedMore;
                    }
                    return Consume::Advanced(i + 3);
                }
                b'7' => {
                    self.saved_cursor = (self.cursor_row, self.cursor_col);
                    return Consume::Advanced(i + 2);
                }
                b'8' => {
                    self.cursor_row = self.saved_cursor.0.min(self.rows_count as usize - 1);
                    self.cursor_col = self.saved_cursor.1.min(self.cols as usize - 1);
                    return Consume::Advanced(i + 2);
                }
                b'=' | b'>' | b'c' | b'M' | b'E' | b'D' | b'H' | b'Z' => {
                    return Consume::Advanced(i + 2);
                }
                _ if n >= 0x20 && n < 0x7f => return Consume::Advanced(i + 2),
                _ => return Consume::Advanced(i + 1),
            }
        }

        match b {
            b'\n' => {
                self.newline();
                Consume::Advanced(i + 1)
            }
            b'\r' => {
                self.cursor_col = 0;
                Consume::Advanced(i + 1)
            }
            0x08 | 0x7f => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
                Consume::Advanced(i + 1)
            }
            b'\t' => {
                let next = ((self.cursor_col / 8) + 1) * 8;
                while self.cursor_col < next && self.cursor_col < self.cols as usize {
                    self.write_char(' ');
                }
                Consume::Advanced(i + 1)
            }
            0x07 | 0x0e | 0x0f => Consume::Advanced(i + 1),
            b if b >= 0x80 => self.consume_utf8(data, i),
            b if b >= 0x20 => {
                self.write_char(b as char);
                Consume::Advanced(i + 1)
            }
            _ => Consume::Advanced(i + 1),
        }
    }

    fn consume_utf8(&mut self, data: &[u8], i: usize) -> Consume {
        let b0 = data[i];
        let need = if b0 & 0xE0 == 0xC0 {
            2
        } else if b0 & 0xF0 == 0xE0 {
            3
        } else if b0 & 0xF8 == 0xF0 {
            4
        } else {
            return Consume::Advanced(i + 1);
        };
        if i + need > data.len() {
            return Consume::NeedMore;
        }
        match std::str::from_utf8(&data[i..i + need]) {
            Ok(s) => {
                if let Some(ch) = s.chars().next() {
                    self.write_char(ch);
                }
                Consume::Advanced(i + need)
            }
            Err(_) => Consume::Advanced(i + 1),
        }
    }

    fn consume_csi(&mut self, data: &[u8], start: usize) -> Consume {
        let mut i = start;
        let mut private = None;
        if i < data.len() && matches!(data[i], b'?' | b'>' | b'=' | b'<') {
            private = Some(data[i] as char);
            i += 1;
        }
        let param_start = i;
        while i < data.len() {
            let b = data[i];
            if b.is_ascii_digit() || b == b';' || b == b':' || b == b' ' {
                i += 1;
                continue;
            }
            if (0x20..=0x2F).contains(&b) {
                i += 1;
                continue;
            }
            if (0x40..=0x7E).contains(&b) {
                let params = parse_csi_params(&data[param_start..i]);
                self.apply_csi(b as char, &params, private);
                return Consume::Advanced(i + 1);
            }
            return Consume::Advanced(i + 1);
        }
        Consume::NeedMore
    }

    fn consume_osc(&mut self, data: &[u8], start: usize) -> Consume {
        let mut i = start;
        while i < data.len() {
            if data[i] == 0x07 {
                return Consume::Advanced(i + 1);
            }
            if data[i] == 0x1b {
                if i + 1 >= data.len() {
                    return Consume::NeedMore;
                }
                if data[i + 1] == b'\\' {
                    return Consume::Advanced(i + 2);
                }
                return Consume::Advanced(i);
            }
            i += 1;
        }
        Consume::NeedMore
    }

    fn consume_string_seq(&mut self, data: &[u8], start: usize) -> Consume {
        self.consume_osc(data, start)
    }

    fn apply_csi(&mut self, cmd: char, nums: &[i32], private: Option<char>) {
        // Private modes: CSI ? … h/l  (alt screen, cursor, etc.)
        if private == Some('?') && (cmd == 'h' || cmd == 'l') {
            let enable = cmd == 'h';
            for &mode in nums {
                self.apply_private_mode(mode, enable);
            }
            return;
        }
        if private.is_some() {
            // Other private sequences — swallow
            return;
        }

        let n = |i: usize, d: i32| -> i32 {
            nums.get(i)
                .copied()
                .filter(|&v| v != 0)
                .unwrap_or(d)
        };
        let n0 = |i: usize, d: i32| -> i32 { nums.get(i).copied().unwrap_or(d) };

        match cmd {
            'A' => {
                self.cursor_row = self
                    .cursor_row
                    .saturating_sub(n(0, 1).max(1) as usize)
            }
            'B' => {
                self.cursor_row = (self.cursor_row + n(0, 1).max(1) as usize)
                    .min(self.rows_count as usize - 1)
            }
            'C' => {
                self.cursor_col = (self.cursor_col + n(0, 1).max(1) as usize)
                    .min(self.cols as usize - 1)
            }
            'D' => {
                self.cursor_col = self
                    .cursor_col
                    .saturating_sub(n(0, 1).max(1) as usize)
            }
            'E' => {
                self.cursor_row = (self.cursor_row + n(0, 1).max(1) as usize)
                    .min(self.rows_count as usize - 1);
                self.cursor_col = 0;
            }
            'F' => {
                self.cursor_row = self
                    .cursor_row
                    .saturating_sub(n(0, 1).max(1) as usize);
                self.cursor_col = 0;
            }
            'G' => {
                self.cursor_col = (n(0, 1).max(1) as usize - 1).min(self.cols as usize - 1);
            }
            'H' | 'f' => {
                self.cursor_row =
                    (n(0, 1).max(1) as usize - 1).min(self.rows_count as usize - 1);
                self.cursor_col =
                    (n(1, 1).max(1) as usize - 1).min(self.cols as usize - 1);
            }
            'd' => {
                self.cursor_row =
                    (n(0, 1).max(1) as usize - 1).min(self.rows_count as usize - 1);
            }
            'J' => self.erase_display(n0(0, 0)),
            'K' => self.erase_line(n0(0, 0)),
            'S' => {
                let n = n(0, 1).max(1) as usize;
                for _ in 0..n {
                    self.scroll_up_one();
                }
            }
            'T' => {
                let n = n(0, 1).max(1) as usize;
                for _ in 0..n {
                    self.scroll_down_one();
                }
            }
            '@' => {
                let n = n(0, 1).max(1) as usize;
                let r = self.cursor_row;
                let c = self.cursor_col;
                let row = &mut self.rows[r];
                for _ in 0..n {
                    if c < row.len() {
                        row.insert(c, Cell::blank());
                        if row.len() > self.cols as usize {
                            row.pop();
                        }
                    }
                }
            }
            'P' => {
                let n = n(0, 1).max(1) as usize;
                let r = self.cursor_row;
                let c = self.cursor_col;
                let row = &mut self.rows[r];
                for _ in 0..n {
                    if c < row.len() {
                        row.remove(c);
                        row.push(Cell::blank());
                    }
                }
            }
            'X' => {
                let n = n(0, 1).max(1) as usize;
                let r = self.cursor_row;
                for c in self.cursor_col..(self.cursor_col + n).min(self.cols as usize) {
                    self.rows[r][c] = Cell::blank();
                }
            }
            's' => self.saved_cursor = (self.cursor_row, self.cursor_col),
            'u' => {
                self.cursor_row = self.saved_cursor.0.min(self.rows_count as usize - 1);
                self.cursor_col = self.saved_cursor.1.min(self.cols as usize - 1);
            }
            'm' => self.apply_sgr(nums),
            'n' | 'r' | 't' => {}
            _ => {}
        }
    }

    fn apply_private_mode(&mut self, mode: i32, enable: bool) {
        match mode {
            // Alternate screen (xterm)
            47 | 1047 | 1049 => {
                if enable {
                    self.enter_alt_screen(mode == 1049 || mode == 1047);
                } else {
                    self.leave_alt_screen(mode == 1049 || mode == 1047);
                }
            }
            // DECCKM: arrows switch between CSI (\x1b[A) and SS3 (\x1bOA).
            1 => self.app_cursor_keys = enable,
            // The inner app asked for mouse events — the shell forwards wheel.
            1000 | 1002 | 1003 => self.mouse_reporting = enable,
            // Bracketed paste — remember so forwarded pastes get wrapped.
            2004 => self.bracketed_paste = enable,
            // Cursor visibility, SGR encoding, focus — ignore
            25 | 1006 | 1004 | 7 | 12 => {}
            _ => {}
        }
    }

    fn enter_alt_screen(&mut self, clear: bool) {
        if self.alt_screen {
            if clear {
                self.rows = blank_grid(self.cols, self.rows_count);
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            return;
        }
        self.saved_primary = Some(SavedScreen {
            rows: std::mem::replace(&mut self.rows, blank_grid(self.cols, self.rows_count)),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            scrollback: std::mem::take(&mut self.scrollback),
        });
        self.alt_screen = true;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        if !clear {
            // already blank
        }
    }

    fn leave_alt_screen(&mut self, _restore_cursor_style: bool) {
        if !self.alt_screen {
            return;
        }
        if let Some(saved) = self.saved_primary.take() {
            self.rows = saved.rows;
            self.cursor_row = saved.cursor_row.min(self.rows_count as usize - 1);
            self.cursor_col = saved.cursor_col.min(self.cols as usize - 1);
            self.scrollback = saved.scrollback;
        } else {
            self.rows = blank_grid(self.cols, self.rows_count);
            self.cursor_row = 0;
            self.cursor_col = 0;
        }
        self.alt_screen = false;
        self.scroll_offset = 0;
    }

    fn erase_display(&mut self, mode: i32) {
        if mode == 2 || mode == 3 {
            for row in &mut self.rows {
                for c in row.iter_mut() {
                    *c = Cell::blank();
                }
            }
            if mode == 2 {
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            if mode == 3 {
                self.scrollback.clear();
            }
        } else if mode == 0 {
            for c in self.cursor_col..self.cols as usize {
                self.rows[self.cursor_row][c] = Cell::blank();
            }
            for r in self.cursor_row + 1..self.rows_count as usize {
                for c in 0..self.cols as usize {
                    self.rows[r][c] = Cell::blank();
                }
            }
        } else if mode == 1 {
            for r in 0..self.cursor_row {
                for c in 0..self.cols as usize {
                    self.rows[r][c] = Cell::blank();
                }
            }
            for c in 0..=self.cursor_col.min(self.cols as usize - 1) {
                self.rows[self.cursor_row][c] = Cell::blank();
            }
        }
    }

    fn erase_line(&mut self, mode: i32) {
        let r = self.cursor_row;
        let range: Box<dyn Iterator<Item = usize>> = if mode == 0 {
            Box::new(self.cursor_col..self.cols as usize)
        } else if mode == 1 {
            Box::new(0..=self.cursor_col.min(self.cols as usize - 1))
        } else {
            Box::new(0..self.cols as usize)
        };
        for c in range {
            self.rows[r][c] = Cell::blank();
        }
    }

    fn scroll_up_one(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        if !self.alt_screen {
            self.scrollback.push(self.rows[0].clone());
            if self.scrollback.len() > 5000 {
                let drain = self.scrollback.len() - 5000;
                self.scrollback.drain(0..drain);
            }
        }
        self.rows.remove(0);
        self.rows
            .push(vec![Cell::blank(); self.cols as usize]);
    }

    fn scroll_down_one(&mut self) {
        self.rows
            .insert(0, vec![Cell::blank(); self.cols as usize]);
        if self.rows.len() > self.rows_count as usize {
            self.rows.pop();
        }
    }

    fn apply_sgr(&mut self, modes: &[i32]) {
        let modes: Vec<i32> = if modes.is_empty() {
            vec![0]
        } else {
            modes.to_vec()
        };
        let mut idx = 0;
        while idx < modes.len() {
            match modes[idx] {
                0 => {
                    self.fg = Color::Default;
                    self.bg = Color::Default;
                    self.bold = false;
                    self.reverse = false;
                }
                1 => self.bold = true,
                2 | 22 => self.bold = false,
                7 => self.reverse = true,
                27 => self.reverse = false,
                30..=37 => self.fg = ansi_to_color(modes[idx] - 30),
                38 => {
                    if let Some((c, skip)) = parse_ext_color(&modes[idx + 1..]) {
                        self.fg = c;
                        idx += skip;
                    }
                }
                39 => self.fg = Color::Default,
                40..=47 => self.bg = ansi_to_color(modes[idx] - 40),
                48 => {
                    if let Some((c, skip)) = parse_ext_color(&modes[idx + 1..]) {
                        self.bg = c;
                        idx += skip;
                    }
                }
                49 => self.bg = Color::Default,
                90..=97 => self.fg = bright_to_color(modes[idx] - 90),
                100..=107 => self.bg = bright_to_color(modes[idx] - 100),
                _ => {}
            }
            idx += 1;
        }
    }

    fn newline(&mut self) {
        if self.cursor_row + 1 >= self.rows_count as usize {
            self.scroll_up_one();
        } else {
            self.cursor_row += 1;
        }
        self.cursor_col = 0;
    }

    fn write_char(&mut self, ch: char) {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w == 0 {
            return;
        }
        if self.cursor_col + w > self.cols as usize {
            self.newline();
        }
        if self.cursor_row >= self.rows.len() || self.cursor_col >= self.cols as usize {
            return;
        }
        let (mut fg, mut bg) = (self.fg, self.bg);
        if self.reverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        let fg = if fg != Color::Default { Some(fg) } else { None };
        let bg = if bg != Color::Default { Some(bg) } else { None };
        self.rows[self.cursor_row][self.cursor_col] = Cell { ch, fg, bg };
        if w >= 2 && self.cursor_col + 1 < self.cols as usize {
            self.rows[self.cursor_row][self.cursor_col + 1] = Cell {
                ch: ' ',
                fg: None,
                bg,
            };
        }
        self.cursor_col += w;
    }

    /// Visible primary buffer rows as (char, fg, bg) with Default → black.
    /// Cells → spans, skipping the spacer cell after a wide (CJK) char so a
    /// 2-column glyph doesn't render as glyph + extra space.
    fn row_cells_to_spans(
        row: &[Cell],
        force_black_bg: bool,
    ) -> Vec<(String, Option<ratatui::style::Color>, Option<ratatui::style::Color>)> {
        // Run-group consecutive same-style cells: a typical row collapses from
        // ~200 one-char Strings to a handful of spans (this runs every frame).
        let mut out: Vec<(String, Option<ratatui::style::Color>, Option<ratatui::style::Color>)> =
            Vec::new();
        let mut i = 0;
        while i < row.len() {
            let cell = &row[i];
            let w = UnicodeWidthChar::width(cell.ch).unwrap_or(1).max(1);
            // Always resolve so empty cells paint pure black
            let fg = Some(cell.fg.unwrap_or(Color::Default).to_ratatui_fg());
            let bg = if force_black_bg {
                Some(Color::Default.to_ratatui())
            } else {
                Some(cell.bg.unwrap_or(Color::Default).to_ratatui())
            };
            match out.last_mut() {
                Some((run, rfg, rbg)) if *rfg == fg && *rbg == bg => run.push(cell.ch),
                _ => out.push((cell.ch.to_string(), fg, bg)),
            }
            i += w;
        }
        out
    }

    pub fn visible_rows(
        &self,
    ) -> Vec<Vec<(String, Option<ratatui::style::Color>, Option<ratatui::style::Color>)>> {
        self.rows
            .iter()
            .map(|row| Self::row_cells_to_spans(row, false))
            .collect()
    }

    /// Visible rows re-encoded with truecolor SGR for non-ratatui faces (Suisei).
    pub fn visible_rows_sgr(&self) -> Vec<String> {
        self.visible_rows()
            .into_iter()
            .map(|row| {
                let mut s = String::new();
                let mut last_fg: Option<(u8, u8, u8)> = None;
                let mut last_bg: Option<(u8, u8, u8)> = None;
                for (frag, fg, bg) in row {
                    let fgr = fg.and_then(ratatui_to_rgb);
                    let bgr = bg.and_then(ratatui_to_rgb);
                    if fgr != last_fg {
                        if let Some((r, g, b)) = fgr {
                            s.push_str(&format!("\u{1b}[38;2;{r};{g};{b}m"));
                        } else {
                            s.push_str("\u{1b}[39m");
                        }
                        last_fg = fgr;
                    }
                    if bgr != last_bg {
                        if let Some((r, g, b)) = bgr {
                            s.push_str(&format!("\u{1b}[48;2;{r};{g};{b}m"));
                        } else {
                            s.push_str("\u{1b}[49m");
                        }
                        last_bg = bgr;
                    }
                    s.push_str(&frag);
                }
                if last_fg.is_some() || last_bg.is_some() {
                    s.push_str("\u{1b}[0m");
                }
                let trimmed = s.trim_end().to_string();
                if trimmed.is_empty() {
                    " ".into()
                } else {
                    trimmed
                }
            })
            .collect()
    }

    /// Inner app (claude/vim/htop…) asked for mouse events — forward wheel.
    pub fn wants_mouse(&self) -> bool {
        self.mouse_reporting
    }

    /// Arrow-key bytes matching the inner app's cursor-key mode (DECCKM).
    pub fn arrow_seq(&self, dir: char) -> &'static [u8] {
        match (self.app_cursor_keys, dir) {
            (true, 'A') => b"\x1bOA",
            (true, 'B') => b"\x1bOB",
            (true, 'C') => b"\x1bOC",
            (true, 'D') => b"\x1bOD",
            (false, 'A') => b"\x1b[A",
            (false, 'B') => b"\x1b[B",
            (false, 'C') => b"\x1b[C",
            _ => b"\x1b[D",
        }
    }

    pub fn scroll(&self) -> usize {
        // Alt-screen TUIs shouldn't show scrollback
        if self.alt_screen {
            0
        } else {
            self.scroll_offset
        }
    }
    pub fn scroll_up(&mut self, a: usize) {
        if self.alt_screen {
            return;
        }
        self.scroll_offset = self
            .scroll_offset
            .saturating_add(a)
            .min(self.scrollback.len());
    }
    pub fn scroll_down(&mut self, a: usize) {
        if self.alt_screen {
            return;
        }
        self.scroll_offset = self.scroll_offset.saturating_sub(a);
    }
    pub fn scrollback_len(&self) -> usize {
        if self.alt_screen {
            0
        } else {
            self.scrollback.len()
        }
    }

    pub fn visible_scrollback(
        &self,
    ) -> Vec<Vec<(String, Option<ratatui::style::Color>, Option<ratatui::style::Color>)>> {
        if self.alt_screen {
            return Vec::new();
        }
        self.scrollback
            .iter()
            .map(|row| Self::row_cells_to_spans(row, true))
            .collect()
    }

    pub fn cursor_position(&self) -> (u16, u16) {
        (self.cursor_col as u16, self.cursor_row as u16)
    }

    pub fn is_alt_screen(&self) -> bool {
        self.alt_screen
    }
}

impl Color {
    fn to_ratatui_fg(self) -> ratatui::style::Color {
        match self {
            Color::Default => ratatui::style::Color::Rgb(200, 200, 200),
            other => other.to_ratatui(),
        }
    }
}

enum Consume {
    Advanced(usize),
    NeedMore,
}

fn parse_csi_params(bytes: &[u8]) -> Vec<i32> {
    let s = String::from_utf8_lossy(bytes);
    if s.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for part in s.split(';') {
        if part.is_empty() {
            out.push(0);
            continue;
        }
        if part.contains(':') {
            for sub in part.split(':') {
                out.push(sub.parse::<i32>().unwrap_or(0));
            }
        } else {
            out.push(part.parse::<i32>().unwrap_or(0));
        }
    }
    out
}

fn parse_ext_color(rest: &[i32]) -> Option<(Color, usize)> {
    if rest.is_empty() {
        return None;
    }
    match rest[0] {
        5 if rest.len() >= 2 => Some((index_to_color(rest[1]), 2)),
        2 if rest.len() >= 4 => {
            let r = rest[1].clamp(0, 255) as u8;
            let g = rest[2].clamp(0, 255) as u8;
            let b = rest[3].clamp(0, 255) as u8;
            Some((Color::Rgb(r, g, b), 4))
        }
        _ => Some((Color::Default, 1)),
    }
}

fn ansi_to_color(i: i32) -> Color {
    match i {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::White,
        _ => Color::Default,
    }
}
fn bright_to_color(i: i32) -> Color {
    match i {
        0 => Color::BrightBlack,
        1 => Color::BrightRed,
        2 => Color::BrightGreen,
        3 => Color::BrightYellow,
        4 => Color::BrightBlue,
        5 => Color::BrightMagenta,
        6 => Color::BrightCyan,
        7 => Color::BrightWhite,
        _ => Color::Default,
    }
}
fn index_to_color(i: i32) -> Color {
    match i {
        0..=7 => ansi_to_color(i),
        8..=15 => bright_to_color(i - 8),
        16..=231 => {
            let n = i - 16;
            let r = ((n / 36) % 6) * 51;
            let g = ((n / 6) % 6) * 51;
            let b = (n % 6) * 51;
            Color::Rgb(r as u8, g as u8, b as u8)
        }
        232..=255 => {
            let v = ((i - 232) * 10 + 8).clamp(0, 255) as u8;
            Color::Rgb(v, v, v)
        }
        _ => Color::Default,
    }
}

fn ratatui_to_rgb(c: ratatui::style::Color) -> Option<(u8, u8, u8)> {
    use ratatui::style::Color::*;
    Some(match c {
        Reset => return None,
        Black => (0, 0, 0),
        Red => (205, 49, 49),
        Green => (13, 188, 121),
        Yellow => (229, 229, 16),
        Blue => (36, 114, 200),
        Magenta => (188, 63, 188),
        Cyan => (17, 168, 205),
        Gray => (180, 180, 180),
        DarkGray => (102, 102, 102),
        LightRed => (241, 76, 76),
        LightGreen => (35, 209, 139),
        LightYellow => (245, 245, 67),
        LightBlue => (59, 142, 234),
        LightMagenta => (214, 112, 214),
        LightCyan => (41, 184, 219),
        White => (229, 229, 229),
        Rgb(r, g, b) => (r, g, b),
        Indexed(i) => match index_to_color(i as i32) {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Black | Color::Default => (0, 0, 0),
            Color::Red => (205, 49, 49),
            Color::Green => (13, 188, 121),
            Color::Yellow => (229, 229, 16),
            Color::Blue => (36, 114, 200),
            Color::Magenta => (188, 63, 188),
            Color::Cyan => (17, 168, 205),
            Color::White => (229, 229, 229),
            Color::BrightBlack => (102, 102, 102),
            Color::BrightRed => (241, 76, 76),
            Color::BrightGreen => (35, 209, 139),
            Color::BrightYellow => (245, 245, 67),
            Color::BrightBlue => (59, 142, 234),
            Color::BrightMagenta => (214, 112, 214),
            Color::BrightCyan => (41, 184, 219),
            Color::BrightWhite => (255, 255, 255),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_box_drawing_not_mojibake() {
        let mut t = Terminal::new();
        t.process_output(&[0xe2, 0x94, 0x80]);
        assert_eq!(t.rows[0][0].ch, '─');
    }

    #[test]
    fn osc_title_is_swallowed() {
        let mut t = Terminal::new();
        let mut seq = b"\x1b]0;hello\x07".to_vec();
        seq.extend_from_slice(b"ok");
        t.process_output(&seq);
        assert_eq!(t.rows[0][0].ch, 'o');
        assert_eq!(t.rows[0][1].ch, 'k');
    }

    #[test]
    fn incomplete_utf8_held_across_chunks() {
        let mut t = Terminal::new();
        t.process_output(&[0xe2]);
        t.process_output(&[0x94, 0x80]);
        assert_eq!(t.rows[0][0].ch, '─');
    }

    #[test]
    fn alt_screen_enter_leave() {
        let mut t = Terminal::new();
        t.process_output(b"hello");
        assert_eq!(t.rows[0][0].ch, 'h');
        // CSI ? 1049 h
        t.process_output(b"\x1b[?1049h");
        assert!(t.alt_screen);
        assert_eq!(t.rows[0][0].ch, ' ');
        t.process_output(b"alt");
        assert_eq!(t.rows[0][0].ch, 'a');
        // CSI ? 1049 l
        t.process_output(b"\x1b[?1049l");
        assert!(!t.alt_screen);
        assert_eq!(t.rows[0][0].ch, 'h');
    }

    #[test]
    fn cup_and_clear() {
        let mut t = Terminal::new();
        t.process_output(b"\x1b[10;5H*");
        assert_eq!(t.cursor_row, 9);
        assert_eq!(t.cursor_col, 5); // after writing *
        assert_eq!(t.rows[9][4].ch, '*');
        t.process_output(b"\x1b[2J");
        assert_eq!(t.rows[9][4].ch, ' ');
        assert_eq!(t.cursor_row, 0);
    }
}
