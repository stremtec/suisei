//! Peek definition overlay (VS Code Alt+F12 style).

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct PeekState {
    pub open: bool,
    pub path: PathBuf,
    pub target_row: usize,
    pub target_col: usize,
    /// Lines loaded from file (or buffer) around the target.
    pub lines: Vec<String>,
    /// Absolute buffer row of `lines[0]`
    pub base_row: usize,
    pub scroll: usize,
}

impl Default for PeekState {
    fn default() -> Self {
        Self {
            open: false,
            path: PathBuf::new(),
            target_row: 0,
            target_col: 0,
            lines: Vec::new(),
            base_row: 0,
            scroll: 0,
        }
    }
}

impl PeekState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn close(&mut self) {
        self.open = false;
        self.lines.clear();
    }

    /// Load a window of source around `row` from disk (or `fallback_text` if path matches open buffer).
    pub fn open_at(
        &mut self,
        path: PathBuf,
        row: usize,
        col: usize,
        fallback_text: Option<&str>,
        context: usize,
    ) {
        let text = if let Some(t) = fallback_text {
            t.to_string()
        } else {
            std::fs::read_to_string(&path).unwrap_or_default()
        };
        let all: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        let n = all.len().max(1);
        let row = row.min(n.saturating_sub(1));
        let start = row.saturating_sub(context);
        let end = (row + context + 1).min(all.len());
        self.open = true;
        self.path = path;
        self.target_row = row;
        self.target_col = col;
        self.base_row = start;
        self.lines = all[start..end].to_vec();
        self.scroll = 0;
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let max = self.lines.len().saturating_sub(1);
        let cur = self.scroll as isize + delta;
        self.scroll = cur.clamp(0, max as isize) as usize;
    }
}
