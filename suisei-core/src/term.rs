//! Built-in terminal emulator with a **real PTY**.
//!
//! Uses `portable-pty` so child processes (opencode, claude, vim, …) get a
//! genuine tty, correct `TIOCSWINSZ` on resize, and SIGWINCH. Pairs that with
//! UTF-8 / CSI / OSC parsing and an alternate screen buffer for full-screen TUIs.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use unicode_width::UnicodeWidthChar;

pub struct Terminal {
    pub open: bool,
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
    /// DECSTBM scroll region, inclusive 0-based rows. Full grid when
    /// `top == 0 && bottom == rows - 1`; tmux/screen/vim set it constantly —
    /// ignoring it smeared their partial-screen redraws across the grid.
    scroll_top: usize,
    scroll_bottom: usize,
    /// Mouse tracking the inner app requested: 0 = off, 1 = press/release
    /// (?1000), 2 = + button-motion (?1002), 3 = any motion (?1003). Without
    /// this the face's mouse events never reach vim/htop/tmux.
    mouse_mode: u8,
    /// SGR extended mouse coordinates (?1006) — the only encoding that
    /// survives columns past 223; modern apps enable it alongside tracking.
    mouse_sgr: bool,
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
    /// Window title the shell reported last (OSC 0/2) — surfaced as the
    /// terminal tab's title so `make`, `vim file`, an ssh session each name
    /// their own tab instead of every shell reading "Terminal".
    pub title: Option<String>,
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
    /// SGR 1 at write time. Kept on the cell (not just parser state) so the
    /// SGR re-encoding can transmit it — `ls --color` and git diffs used to
    /// arrive flat because bold died with the parser run.
    bold: bool,
}

impl Cell {
    fn blank() -> Self {
        Self {
            ch: ' ',
            fg: None,
            bg: None,
            bold: false,
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
    /// The sRGB this ANSI colour paints as. `Default` is the caller's choice
    /// (see `to_rgba_fg`), so it returns `None` here.
    ///
    /// This replaced a two-hop `ANSI -> ratatui::style::Color -> rgb` walk; the
    /// palette below is the one that walk resolved to, unchanged.
    fn to_rgba(self) -> Option<crate::theme::Rgba> {
        use crate::theme::rgb;
        Some(match self {
            // Default -> pure black so agent TUIs don't sit on a grey "frame".
            Color::Default => rgb(0, 0, 0),
            Color::Black => rgb(0, 0, 0),
            Color::Red => rgb(205, 49, 49),
            Color::Green => rgb(13, 188, 121),
            Color::Yellow => rgb(229, 229, 16),
            Color::Blue => rgb(36, 114, 200),
            Color::Magenta => rgb(188, 63, 188),
            Color::Cyan => rgb(17, 168, 205),
            Color::White => rgb(229, 229, 229),
            Color::BrightBlack => rgb(102, 102, 102),
            Color::BrightRed => rgb(241, 76, 76),
            Color::BrightGreen => rgb(35, 209, 139),
            Color::BrightYellow => rgb(245, 245, 67),
            Color::BrightBlue => rgb(59, 142, 234),
            Color::BrightMagenta => rgb(214, 112, 214),
            Color::BrightCyan => rgb(41, 184, 219),
            Color::BrightWhite => rgb(255, 255, 255),
            Color::Rgb(r, g, b) => rgb(r, g, b),
        })
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
            scroll_top: 0,
            scroll_bottom: rows as usize - 1,
            mouse_mode: 0,
            mouse_sgr: false,
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
            title: None,
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // No orphaned PTYs: a dropped terminal kills its shell and reaps it.
        // Closing the master SIGHUPs the slave's session, but an explicit
        // kill covers hup-ignoring shells and reaps promptly — owners that
        // never call `shutdown` (a replaced session, a dropped state) must
        // not leak processes.
        self.writer = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.master = None;
        self.rx = None;
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
            // The PTY already has this size (openpty spawned with it, or an
            // earlier resize pushed it) — nothing to do.
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
        // Margins reference the old grid — back to the full grid.
        self.scroll_top = 0;
        self.scroll_bottom = rows as usize - 1;
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
        self.scroll_top = 0;
        self.scroll_bottom = self.rows_count as usize - 1;
        self.mouse_mode = 0;
        self.mouse_sgr = false;
        self.app_cursor_keys = false;
        self.bracketed_paste = false;
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.bold = false;
        self.reverse = false;
        self.title = None;
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
        self.close_confirm = false;
        self.pending.clear();
        self.alt_screen = false;
        self.saved_primary = None;
        self.rows = blank_grid(self.cols, self.rows_count);
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scrollback.clear();
        self.scroll_offset = 0;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows_count as usize - 1;
        self.mouse_mode = 0;
        self.mouse_sgr = false;
        self.app_cursor_keys = false;
        self.bracketed_paste = false;
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.bold = false;
        self.reverse = false;
        self.title = None;
    }

    pub fn write_input(&mut self, bytes: &[u8]) {
        // Typing snaps the view back to the live prompt.
        self.scroll_offset = 0;
        self.write_raw(bytes);
    }

    /// Write to the child without touching view state. Terminal-initiated
    /// replies (CPR, DA1) must not snap a scrolled-back view to the prompt.
    fn write_raw(&mut self, bytes: &[u8]) {
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
        // One tick's parsing budget. A flood (`yes | head -100000`) used to
        // drain and parse every queued chunk in one go, stalling the face's
        // main-thread timer for tens of ms; the rest now stays queued and the
        // damage flag keeps the next tick draining.
        const MAX_BYTES_PER_POLL: usize = 256 * 1024;
        let data = if let Some(ref rx) = self.rx {
            let mut all = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(part) => {
                        all.extend_from_slice(&part);
                        if all.len() >= MAX_BYTES_PER_POLL {
                            break;
                        }
                    }
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
                b'=' | b'>' | b'c' | b'H' | b'Z' => {
                    return Consume::Advanced(i + 2);
                }
                b'D' => {
                    // IND — index: down one, scrolling at the bottom margin.
                    self.index_down();
                    return Consume::Advanced(i + 2);
                }
                b'E' => {
                    // NEL — next line: index + carriage return.
                    self.index_down();
                    self.cursor_col = 0;
                    return Consume::Advanced(i + 2);
                }
                b'M' => {
                    // RI — reverse index: up one, scrolling at the top margin.
                    self.reverse_index();
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
            // OSC ends at BEL or ST (ESC \).
            let bel = data[i] == 0x07;
            let st = data[i] == 0x1b && i + 1 < data.len() && data[i + 1] == b'\\';
            if bel || st {
                self.apply_osc(&data[start..i]);
                return Consume::Advanced(if bel { i + 1 } else { i + 2 });
            }
            if data[i] == 0x1b {
                if i + 1 >= data.len() {
                    return Consume::NeedMore;
                }
                return Consume::Advanced(i);
            }
            i += 1;
        }
        Consume::NeedMore
    }

    /// OSC payload `code;text`. Only window titles matter here — 0 (icon +
    /// title) and 2 (title), which shells, vim and tmux all send. Everything
    /// else (7 = cwd, 52 = clipboard, …) is dropped.
    fn apply_osc(&mut self, payload: &[u8]) {
        let Ok(s) = std::str::from_utf8(payload) else {
            return;
        };
        let Some((code, rest)) = s.split_once(';') else {
            return;
        };
        if matches!(code, "0" | "2") {
            let title = rest.trim();
            self.title = if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            };
        }
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

        let n = |i: usize, d: i32| -> i32 { nums.get(i).copied().filter(|&v| v != 0).unwrap_or(d) };
        let n0 = |i: usize, d: i32| -> i32 { nums.get(i).copied().unwrap_or(d) };

        match cmd {
            'A' => self.cursor_row = self.cursor_row.saturating_sub(n(0, 1).max(1) as usize),
            'B' => {
                self.cursor_row =
                    (self.cursor_row + n(0, 1).max(1) as usize).min(self.rows_count as usize - 1)
            }
            'C' => {
                self.cursor_col =
                    (self.cursor_col + n(0, 1).max(1) as usize).min(self.cols as usize - 1)
            }
            'D' => self.cursor_col = self.cursor_col.saturating_sub(n(0, 1).max(1) as usize),
            'E' => {
                self.cursor_row =
                    (self.cursor_row + n(0, 1).max(1) as usize).min(self.rows_count as usize - 1);
                self.cursor_col = 0;
            }
            'F' => {
                self.cursor_row = self.cursor_row.saturating_sub(n(0, 1).max(1) as usize);
                self.cursor_col = 0;
            }
            'G' => {
                self.cursor_col = (n(0, 1).max(1) as usize - 1).min(self.cols as usize - 1);
            }
            'H' | 'f' => {
                self.cursor_row = (n(0, 1).max(1) as usize - 1).min(self.rows_count as usize - 1);
                self.cursor_col = (n(1, 1).max(1) as usize - 1).min(self.cols as usize - 1);
            }
            'd' => {
                self.cursor_row = (n(0, 1).max(1) as usize - 1).min(self.rows_count as usize - 1);
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
            'n' => {
                // DSR 6 → CPR: report the cursor position. Probing tools
                // (tmux startup, `tput u7`, zsh prompt widgets) stall or time
                // out on a terminal that never answers.
                if n0(0, 0) == 6 {
                    let reply = format!("\x1b[{};{}R", self.cursor_row + 1, self.cursor_col + 1);
                    self.write_raw(reply.as_bytes());
                }
            }
            'c' => {
                // DA1: identify as a VT220 with ANSI colour so startup probes
                // get an answer instead of a timeout.
                self.write_raw(b"\x1b[?62;22c");
            }
            'r' => {
                // DECSTBM — set top/bottom scroll margins (1-based,
                // inclusive). No args — or an invalid pair — resets to the
                // full grid. The cursor goes home, per the DEC spec.
                let top = n(0, 1).max(1) as usize - 1;
                let bottom = n(1, self.rows_count as i32).max(1) as usize - 1;
                if top < bottom && bottom < self.rows_count as usize {
                    self.scroll_top = top;
                    self.scroll_bottom = bottom;
                } else {
                    self.scroll_top = 0;
                    self.scroll_bottom = self.rows_count as usize - 1;
                }
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            't' => {}
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
            // The inner app asked for mouse events — the face forwards them.
            1000 => self.mouse_mode = if enable { 1 } else { 0 },
            1002 => self.mouse_mode = if enable { 2 } else { 0 },
            1003 => self.mouse_mode = if enable { 3 } else { 0 },
            // SGR extended coordinates for mouse reports.
            1006 => self.mouse_sgr = enable,
            // Bracketed paste — remember so forwarded pastes get wrapped.
            2004 => self.bracketed_paste = enable,
            // Cursor visibility, focus — ignore
            25 | 1004 | 7 | 12 => {}
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
        if self.region_active() {
            // Region scroll moves only the rows inside the margins — the
            // content above and below must not shift, and nothing goes to
            // scrollback (DEC: region scrolls are not history).
            let bottom = self.scroll_bottom.min(self.rows.len() - 1);
            let top = self.scroll_top.min(bottom);
            self.rows.remove(top);
            self.rows
                .insert(bottom, vec![Cell::blank(); self.cols as usize]);
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
        self.rows.push(vec![Cell::blank(); self.cols as usize]);
    }

    fn scroll_down_one(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        if self.region_active() {
            let bottom = self.scroll_bottom.min(self.rows.len() - 1);
            let top = self.scroll_top.min(bottom);
            self.rows.remove(bottom);
            self.rows
                .insert(top, vec![Cell::blank(); self.cols as usize]);
            return;
        }
        self.rows.insert(0, vec![Cell::blank(); self.cols as usize]);
        if self.rows.len() > self.rows_count as usize {
            self.rows.pop();
        }
    }

    /// True while DECSTBM margins are narrower than the full grid.
    fn region_active(&self) -> bool {
        self.scroll_top > 0 || self.scroll_bottom < self.rows_count as usize - 1
    }

    /// IND / NEL body: cursor down one line; at the bottom margin the scroll
    /// region (or the full grid) scrolls up instead, so the cursor never
    /// leaves the region. Column is preserved — NEL adds the column reset.
    fn index_down(&mut self) {
        let bottom = self.scroll_bottom.min(self.rows_count as usize - 1);
        if self.cursor_row == bottom {
            self.scroll_up_one();
        } else if self.cursor_row + 1 < self.rows_count as usize {
            self.cursor_row += 1;
        }
    }

    /// RI: cursor up one line; at the top margin the region scrolls down.
    fn reverse_index(&mut self) {
        if self.cursor_row == self.scroll_top {
            self.scroll_down_one();
        } else {
            self.cursor_row = self.cursor_row.saturating_sub(1);
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
        self.index_down();
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
        self.rows[self.cursor_row][self.cursor_col] = Cell {
            ch,
            fg,
            bg,
            bold: self.bold,
        };
        if w >= 2 && self.cursor_col + 1 < self.cols as usize {
            self.rows[self.cursor_row][self.cursor_col + 1] = Cell {
                ch: ' ',
                fg: None,
                bg,
                bold: false,
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
    ) -> Vec<(
        String,
        Option<crate::theme::Rgba>,
        Option<crate::theme::Rgba>,
    )> {
        // Run-group consecutive same-style cells: a typical row collapses from
        // ~200 one-char Strings to a handful of spans (this runs every frame).
        let mut out: Vec<(
            String,
            Option<crate::theme::Rgba>,
            Option<crate::theme::Rgba>,
        )> = Vec::new();
        let mut i = 0;
        while i < row.len() {
            let cell = &row[i];
            let w = UnicodeWidthChar::width(cell.ch).unwrap_or(1).max(1);
            // Always resolve so empty cells paint pure black
            let fg = cell.fg.unwrap_or(Color::Default).to_rgba_fg();
            let bg = if force_black_bg {
                Color::Default.to_rgba()
            } else {
                cell.bg.unwrap_or(Color::Default).to_rgba()
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
    ) -> Vec<
        Vec<(
            String,
            Option<crate::theme::Rgba>,
            Option<crate::theme::Rgba>,
        )>,
    > {
        self.rows
            .iter()
            .map(|row| Self::row_cells_to_spans(row, false))
            .collect()
    }

    /// The rows the panel should actually draw: the live grid, or a window into
    /// the scrollback when the user has scrolled up.
    ///
    /// [`Terminal::visible_rows`] is the live grid **alone**, and it is what the
    /// GUI scene has always sent — so `scroll_up` moved an offset nothing read
    /// and the panel's scrollback was unreachable. Scrolling up showed the same
    /// screen back.
    pub fn viewport_rows(
        &self,
    ) -> Vec<
        Vec<(
            String,
            Option<crate::theme::Rgba>,
            Option<crate::theme::Rgba>,
        )>,
    > {
        let offset = self.scroll();
        if offset == 0 {
            return self.visible_rows();
        }
        let height = self.rows.len();
        let from_scrollback = offset.min(self.scrollback.len());
        let start = self.scrollback.len() - from_scrollback;
        let mut out: Vec<_> = self.scrollback[start..]
            .iter()
            .take(height)
            .map(|row| Self::row_cells_to_spans(row, true))
            .collect();
        // Fill the rest of the window from the top of the live grid.
        let remaining = height.saturating_sub(out.len());
        out.extend(
            self.rows
                .iter()
                .take(remaining)
                .map(|row| Self::row_cells_to_spans(row, false)),
        );
        out
    }

    /// The rows to draw as raw cells, each flagged when it comes from the
    /// scrollback (scrollback rows suppress backgrounds — see `viewport_rows`).
    fn viewport_cell_rows(&self) -> Vec<(Vec<Cell>, bool)> {
        let offset = self.scroll();
        if offset == 0 {
            return self.rows.iter().map(|r| (r.clone(), false)).collect();
        }
        let height = self.rows.len();
        let from_scrollback = offset.min(self.scrollback.len());
        let start = self.scrollback.len() - from_scrollback;
        let mut out: Vec<(Vec<Cell>, bool)> = self.scrollback[start..]
            .iter()
            .take(height)
            .map(|r| (r.clone(), true))
            .collect();
        // Fill the rest of the window from the top of the live grid.
        let remaining = height.saturating_sub(out.len());
        out.extend(self.rows.iter().take(remaining).map(|r| (r.clone(), false)));
        out
    }

    /// Visible rows re-encoded as truecolor SGR strings.
    ///
    /// The face then parses those escapes back into colours — the panel
    /// round-trips ANSI *inside the process*. Handing the face `(text, fg, bg)`
    /// runs directly would drop an encode and a parse per row per frame; that
    /// is an FFI change, tracked separately in `SUISEI-TUI-RESIDUE.md`.
    pub fn visible_rows_sgr(&self) -> Vec<String> {
        // The ABI row cap is 1536 bytes in the engine (SUISEI_TERM_LINE):
        // dense truecolor output spends ~19 bytes per colour change and used
        // to be hard-truncated there — mid-escape and mid-UTF-8, which the
        // face rendered as garbage. Stop at the budget and drop the row's
        // tail; the face's parser is per-row, so an unfinished sequence is
        // simply ignored rather than misparsed.
        const ROW_BYTE_BUDGET: usize = 1400;
        self.viewport_cell_rows()
            .into_iter()
            .map(|(row, from_scrollback)| {
                let mut s = String::new();
                let mut last_fg: Option<(u8, u8, u8)> = None;
                let mut last_bg: Option<(u8, u8, u8)> = None;
                let mut last_bold = false;
                let mut bold_seen = false;
                let mut i = 0;
                while i < row.len() {
                    let cell = &row[i];
                    let w = UnicodeWidthChar::width(cell.ch).unwrap_or(1).max(1);
                    // Always resolve so empty cells paint the shell defaults.
                    let fg = cell.fg.unwrap_or(Color::Default).to_rgba_fg();
                    // A DEFAULT background emits a reset (`\e[49m`), never an
                    // explicit RGB. The face fills the whole grid with its own
                    // terminal background; a per-cell default-bg run would both
                    // override that with pure black AND, because the face's
                    // per-run rect stops ~2px short of the row pitch, draw a
                    // faint rule under every row (the reported "이상한 라인").
                    // Only a genuinely non-default cell background is painted;
                    // scrollback always shows on the terminal's own background.
                    let bg = if from_scrollback {
                        None
                    } else {
                        match cell.bg {
                            Some(c) if !matches!(c, Color::Default) => c.to_rgba(),
                            _ => None,
                        }
                    };
                    let fgr = fg.map(|c| (c.r, c.g, c.b));
                    let bgr = bg.map(|c| (c.r, c.g, c.b));
                    if cell.bold != last_bold {
                        if s.len() + 8 > ROW_BYTE_BUDGET {
                            break;
                        }
                        s.push_str(if cell.bold { "\u{1b}[1m" } else { "\u{1b}[22m" });
                        last_bold = cell.bold;
                        bold_seen = bold_seen || cell.bold;
                    }
                    if fgr != last_fg {
                        if s.len() + 20 > ROW_BYTE_BUDGET {
                            break;
                        }
                        if let Some((r, g, b)) = fgr {
                            s.push_str(&format!("\u{1b}[38;2;{r};{g};{b}m"));
                        } else {
                            s.push_str("\u{1b}[39m");
                        }
                        last_fg = fgr;
                    }
                    if bgr != last_bg {
                        if s.len() + 20 > ROW_BYTE_BUDGET {
                            break;
                        }
                        if let Some((r, g, b)) = bgr {
                            s.push_str(&format!("\u{1b}[48;2;{r};{g};{b}m"));
                        } else {
                            s.push_str("\u{1b}[49m");
                        }
                        last_bg = bgr;
                    }
                    if s.len() + cell.ch.len_utf8() > ROW_BYTE_BUDGET {
                        break;
                    }
                    s.push(cell.ch);
                    i += w;
                }
                if bold_seen || last_fg.is_some() || last_bg.is_some() {
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

    /// Inner app (claude/vim/htop…) asked for mouse events — forward them.
    pub fn wants_mouse(&self) -> bool {
        self.mouse_mode > 0
    }

    /// Report a mouse event to the inner app when it requested tracking
    /// (?1000/1002/1003). Coordinates are 1-based cells. `button`: 0 left,
    /// 1 middle, 2 right, 64 wheel-up, 65 wheel-down. Motion ORs 32 into the
    /// code per the protocol; ?1000 (clicks only) drops pure motion.
    pub fn mouse_report(&mut self, mut button: u8, x: u16, y: u16, pressed: bool, motion: bool) {
        if self.mouse_mode == 0 || (motion && self.mouse_mode == 1) {
            return;
        }
        if motion {
            button |= 32;
        }
        if self.mouse_sgr {
            let suffix = if pressed || motion { 'M' } else { 'm' };
            let report = format!("\x1b[<{button};{x};{y}{suffix}");
            self.write_input(report.as_bytes());
            return;
        }
        // Legacy X10 encoding: code + 32 as single bytes; coords cap at 223
        // (255 - 32) — apps that skip SGR accept that limit by convention.
        let code = if pressed || motion { button + 32 } else { 35 };
        let xb = (x.min(223) + 32) as u8;
        let yb = (y.min(223) + 32) as u8;
        self.write_input(&[0x1b, b'[', b'M', code, xb, yb]);
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
    ) -> Vec<
        Vec<(
            String,
            Option<crate::theme::Rgba>,
            Option<crate::theme::Rgba>,
        )>,
    > {
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
    /// Foreground variant: an unset foreground is light grey, not black.
    fn to_rgba_fg(self) -> Option<crate::theme::Rgba> {
        match self {
            Color::Default => Some(crate::theme::rgb(200, 200, 200)),
            other => other.to_rgba(),
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

    /// The GUI scene sends `visible_rows_sgr`, which returned the live grid
    /// alone — so `scroll_up` moved an offset nothing read and the panel's
    /// 5,000-row scrollback was unreachable. Scrolling up showed the same
    /// screen back.
    #[test]
    fn scrolling_up_shows_the_scrollback_not_the_same_screen() {
        let mut t = Terminal::new();
        t.resize(20, 3);
        // Push more rows than the grid holds, so the early ones must spill.
        for i in 0..8 {
            t.process_output(format!("line{i}\r\n").as_bytes());
        }
        let live = t.visible_rows_sgr();
        assert!(
            live.iter().any(|r| r.contains("line7")),
            "the newest output is on screen: {live:?}"
        );
        assert!(
            !live.iter().any(|r| r.contains("line0")),
            "the oldest output has scrolled off: {live:?}"
        );

        t.scroll_up(6);
        let back = t.visible_rows_sgr();
        assert_ne!(back, live, "scrolling up must change what is drawn");
        assert!(
            back.iter().any(|r| r.contains("line0")),
            "the oldest output is reachable again: {back:?}"
        );

        t.scroll_down(99);
        assert_eq!(
            t.visible_rows_sgr(),
            live,
            "scrolling back down returns to live"
        );
    }

    /// SGR 1 must survive the cell grid and reach the face's parser — bold
    /// used to die with the parser run, so `ls --color` rendered flat.
    #[test]
    fn bold_reaches_the_sgr_encoding() {
        let mut t = Terminal::new();
        t.process_output(b"\x1b[1mBOLD\x1b[0mplain");
        let row = &t.visible_rows_sgr()[0];
        assert!(row.contains("\u{1b}[1m"), "bold emitted: {row:?}");
        assert!(row.contains("BOLD"));
        assert!(row.contains("plain"));
    }

    /// A default background emits NO explicit `48;2;…` RGB — that was what
    /// painted every row pure black and left a faint rule at each row's foot
    /// (the face's per-run rect stopped short of the row pitch). A genuinely
    /// coloured background still encodes.
    #[test]
    fn default_background_emits_no_explicit_bg() {
        let mut t = Terminal::new();
        t.process_output(b"plain default text");
        let row = &t.visible_rows_sgr()[0];
        assert!(
            !row.contains("\u{1b}[48;2;"),
            "default bg must not paint an explicit RGB: {row:?}"
        );

        let mut t2 = Terminal::new();
        t2.process_output(b"\x1b[41mRED\x1b[0m"); // 41 = red background
        let row2 = &t2.visible_rows_sgr()[0];
        assert!(
            row2.contains("\u{1b}[48;2;"),
            "an explicit background still encodes: {row2:?}"
        );
    }

    /// A row whose colour changes every cell spends ~19 bytes of escape per
    /// character — the encoder must stop at its budget on a char boundary,
    /// never handing the face a row the 1536-byte ABI cap would truncate.
    #[test]
    fn dense_rows_stay_under_the_abi_cap() {
        let mut t = Terminal::new();
        let mut line = Vec::new();
        for i in 0..80u8 {
            line.extend_from_slice(
                format!(
                    "\x1b[38;2;{};{};{}mX",
                    i,
                    i.wrapping_mul(2),
                    i.wrapping_mul(3)
                )
                .as_bytes(),
            );
        }
        t.process_output(&line);
        for row in t.visible_rows_sgr() {
            assert!(row.len() <= 1536, "row {}B exceeds the ABI cap", row.len());
        }
    }

    fn row_text(t: &Terminal, r: usize) -> String {
        t.rows[r]
            .iter()
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// DECSTBM: scrolling inside the margins must not shift the rows outside
    /// them. tmux/screen/vim all repaint through regions; ignoring that
    /// smeared their output across the whole grid.
    #[test]
    fn scroll_region_keeps_rows_outside_it_still() {
        let mut t = Terminal::new();
        t.process_output(b"AAAA\nBBBB\nCCCC\nDDDD\nEEEE\x1b[H");
        // Margins = lines 2..4 (1-based) → rows 1..3 (0-based).
        t.process_output(b"\x1b[2;4r");
        // Cursor to the region's top line, then scroll it hard.
        t.process_output(b"\x1b[2;1H");
        t.process_output(b"1\n2\n3\n4\n5\n6");
        assert_eq!(row_text(&t, 0), "AAAA", "row above the region untouched");
        assert_eq!(row_text(&t, 4), "EEEE", "row below the region untouched");
    }

    /// RI at the top margin scrolls the region down instead of leaving the
    /// grid.
    #[test]
    fn reverse_index_scrolls_at_the_top_margin() {
        let mut t = Terminal::new();
        t.process_output(b"AAAA\nBBBB");
        t.process_output(b"\x1b[1;1H\x1bM");
        assert_eq!(row_text(&t, 0), "", "RI inserted a blank at the top");
        assert_eq!(row_text(&t, 1), "AAAA", "old top row pushed down");
    }

    /// IND at the bottom margin scrolls up; NEL also resets the column.
    #[test]
    fn index_and_next_line_honor_the_bottom_margin() {
        let mut t = Terminal::new();
        t.process_output(b"AAAA\nBBBB");
        // Cursor to the grid's bottom row (the bottom margin, no region).
        t.process_output(b"\x1b[24;1H\x1bD"); // IND → scroll up
        assert_eq!(row_text(&t, 0), "BBBB", "grid scrolled up");
        assert_eq!(row_text(&t, 1), "", "blank row below");
        t.process_output(b"XX\x1bE"); // NEL → down + col 0
        assert_eq!(t.cursor_col, 0, "NEL reset the column");
    }

    /// Resize and CSI r with bad args both reset the region to the full grid.
    #[test]
    fn region_resets_on_resize_and_bad_args() {
        let mut t = Terminal::new();
        t.process_output(b"\x1b[5;10r");
        assert!(t.region_active());
        t.process_output(b"\x1b[r");
        assert!(!t.region_active(), "CSI r with no args resets");
        t.process_output(b"\x1b[5;10r");
        t.resize(80, 30);
        assert!(!t.region_active(), "resize resets");
    }

    /// OSC 0/2 set the shell-reported title (BEL and ST terminators); an
    /// empty title clears it; other OSC codes (7 = cwd) are not titles.
    #[test]
    fn osc_0_and_2_set_the_title() {
        let mut t = Terminal::new();
        t.process_output(b"\x1b]2;build \xe2\x9c\x93\x07");
        assert_eq!(t.title.as_deref(), Some("build \u{2713}"), "OSC 2 via BEL");
        t.process_output(b"\x1b]0;user@host: ~\x1b\\");
        assert_eq!(t.title.as_deref(), Some("user@host: ~"), "OSC 0 via ST");
        t.process_output(b"\x1b]7;file:///some/path\x07");
        assert_eq!(
            t.title.as_deref(),
            Some("user@host: ~"),
            "OSC 7 is not a title"
        );
        t.process_output(b"\x1b]2;\x07");
        assert_eq!(t.title, None, "empty title clears");
    }

    /// Mouse tracking modes: off → no reports; ?1000 clicks-only; ?1002
    /// upgrades to button-motion; ?1006 flips on SGR encoding; disable turns
    /// everything off. (No PTY writer in a unit test, so observe the mode
    /// state that gates `mouse_report`.)
    #[test]
    fn mouse_reports_follow_the_tracking_mode() {
        let mut t = Terminal::new();
        assert!(!t.wants_mouse());
        t.process_output(b"\x1b[?1000h");
        assert!(t.wants_mouse());
        assert_eq!(t.mouse_mode, 1);
        t.process_output(b"\x1b[?1006h");
        assert!(t.mouse_sgr, "SGR encoding flag");
        t.process_output(b"\x1b[?1002h");
        assert_eq!(t.mouse_mode, 2, "1002 upgrades motion tracking");
        t.process_output(b"\x1b[?1000l");
        assert!(!t.wants_mouse(), "disable turns tracking off");
    }
}
