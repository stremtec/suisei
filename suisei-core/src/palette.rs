//! VS Code-style command / file / problems palette.

use std::path::{Path, PathBuf};

use crate::lsp::{Diagnostic, DiagnosticSeverity};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteKind {
    Files,
    Commands,
    Problems,
    /// Document / workspace symbols
    Symbols,
    /// LSP code actions / quick fixes
    CodeActions,
}

#[derive(Clone, Debug)]
pub enum PaletteAction {
    OpenFile(PathBuf),
    /// Built-in command id
    Command(&'static str),
    Goto {
        row: usize,
        col: usize,
    },
    /// Multi-file location jump (search, symbols, refs)
    GotoFile {
        path: PathBuf,
        row: usize,
        col: usize,
    },
    /// Index into App.code_action_bank
    CodeAction(usize),
}

#[derive(Clone, Debug)]
pub struct PaletteItem {
    pub label: String,
    pub detail: String,
    pub action: PaletteAction,
}

#[derive(Clone, Debug)]
pub struct Palette {
    pub open: bool,
    pub kind: PaletteKind,
    pub query: String,
    pub items: Vec<PaletteItem>,
    /// Indices into `items` after filter
    pub filtered: Vec<usize>,
    pub selected: usize,
    /// Animation clock — armed on open, started by the first render so the
    /// synchronous file walk can't eat the window.
    pub opened_at: Option<std::time::Instant>,
    pub anim_pending: bool,
}

/// Expand-in animation length (ms).
pub const PALETTE_ANIM_MS: u64 = 160;

impl Default for Palette {
    fn default() -> Self {
        Self {
            open: false,
            kind: PaletteKind::Commands,
            query: String::new(),
            items: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            opened_at: None,
            anim_pending: false,
        }
    }
}

impl Palette {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.items.clear();
        self.filtered.clear();
        self.selected = 0;
        self.opened_at = None;
        self.anim_pending = false;
    }

    /// Linear expand progress in 0.0..=1.0 (easing happens in the UI).
    /// The clock starts on the first call after opening — the first frame.
    pub fn anim_progress(&mut self) -> f32 {
        if self.anim_pending {
            self.anim_pending = false;
            self.opened_at = Some(std::time::Instant::now());
            return 0.0;
        }
        let Some(t0) = self.opened_at else {
            return 1.0;
        };
        (t0.elapsed().as_millis() as f32 / PALETTE_ANIM_MS as f32).min(1.0)
    }

    fn arm_animation(&mut self) {
        self.anim_pending = true;
        self.opened_at = None;
    }

    pub fn open_commands(&mut self) {
        self.open = true;
        self.kind = PaletteKind::Commands;
        self.query.clear();
        self.arm_animation();
        self.items = builtin_commands();
        self.refilter();
    }

    pub fn open_files(&mut self, root: &Path) {
        self.open = true;
        self.kind = PaletteKind::Files;
        self.query.clear();
        self.arm_animation();
        self.items = collect_files(root, 400);
        self.refilter();
    }

    pub fn open_problems(&mut self, diags: &[Diagnostic]) {
        self.open = true;
        self.kind = PaletteKind::Problems;
        self.query.clear();
        self.arm_animation();
        self.items = diags
            .iter()
            .map(|d| {
                let sev = match d.severity {
                    DiagnosticSeverity::Error => "E",
                    DiagnosticSeverity::Warning => "W",
                    DiagnosticSeverity::Info => "I",
                    DiagnosticSeverity::Hint => "H",
                };
                PaletteItem {
                    label: format!("L{}:{}  [{}] {}", d.row + 1, d.col_start + 1, sev, d.message),
                    detail: String::new(),
                    action: PaletteAction::Goto {
                        row: d.row,
                        col: d.col_start,
                    },
                }
            })
            .collect();
        if self.items.is_empty() {
            self.items.push(PaletteItem {
                label: "No diagnostics".into(),
                detail: String::new(),
                action: PaletteAction::Command("noop"),
            });
        }
        self.refilter();
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.refilter();
    }

    pub fn move_down(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    pub fn move_up(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected == 0 {
                self.selected = self.filtered.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    pub fn selected_action(&self) -> Option<&PaletteAction> {
        let idx = *self.filtered.get(self.selected)?;
        Some(&self.items[idx].action)
    }

    pub fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        if q.is_empty() {
            self.filtered = (0..self.items.len()).collect();
        } else {
            let mut scored: Vec<(i32, usize)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, it)| {
                    let hay = format!("{} {}", it.label, it.detail).to_lowercase();
                    fuzzy_score(&hay, &q).map(|s| (s, i))
                })
                .collect();
            // Best first; original order breaks ties so the list is stable.
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        }
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn open_symbols(&mut self, items: Vec<PaletteItem>) {
        self.open = true;
        self.kind = PaletteKind::Symbols;
        self.query.clear();
        self.arm_animation();
        self.items = items;
        if self.items.is_empty() {
            self.items.push(PaletteItem {
                label: "No symbols".into(),
                detail: "LSP may not support documentSymbol".into(),
                action: PaletteAction::Command("noop"),
            });
        }
        self.refilter();
    }

    pub fn open_code_actions(&mut self, items: Vec<PaletteItem>) {
        self.open = true;
        self.kind = PaletteKind::CodeActions;
        self.query.clear();
        self.arm_animation();
        self.items = items;
        if self.items.is_empty() {
            self.items.push(PaletteItem {
                label: "No code actions".into(),
                detail: String::new(),
                action: PaletteAction::Command("noop"),
            });
        }
        self.refilter();
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            PaletteKind::Files => " Open file ",
            PaletteKind::Commands => " Commands ",
            PaletteKind::Problems => " Problems ",
            PaletteKind::Symbols => " Symbols ",
            PaletteKind::CodeActions => " Code actions ",
        }
    }
}

/// Score a subsequence match, or `None` when `needle` does not fit in `hay`.
/// Higher is better.
///
/// A bare subsequence test — which this was — matches nearly every long
/// absolute path, and with no score the results stayed in filesystem-walk
/// order. Since the face only renders the first 40, the file you wanted could
/// be match #500 and simply never appear. Scoring is what makes "type three
/// letters, hit Enter" work.
fn fuzzy_score(hay: &str, needle: &str) -> Option<i32> {
    let h: Vec<char> = hay.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() {
        return Some(0);
    }
    // Where the basename starts — a hit there is worth far more than one
    // buried in a directory component.
    let base = hay.rfind('/').map(|i| hay[..i].chars().count() + 1).unwrap_or(0);

    let mut score = 0i32;
    let mut hi = 0usize;
    let mut prev_hit: Option<usize> = None;
    for &nc in &n {
        let start = hi;
        loop {
            if hi >= h.len() {
                return None;
            }
            if h[hi] == nc {
                break;
            }
            hi += 1;
        }
        // Consecutive characters are the strongest signal.
        if prev_hit == Some(hi.wrapping_sub(1)) {
            score += 15;
        }
        // Start of a path/word component.
        let boundary = hi == 0 || matches!(h[hi - 1], '/' | '_' | '-' | '.' | ' ');
        if boundary {
            score += 10;
        }
        if hi >= base {
            score += 12;
        }
        // Prefer hits that did not need a long skip.
        score -= ((hi - start) as i32).min(10);
        prev_hit = Some(hi);
        hi += 1;
    }
    // Shorter candidates win ties: `lsp.rs` over `.../vendor/lsp.rs`.
    score -= (h.len() as i32) / 24;
    Some(score)
}

fn builtin_commands() -> Vec<PaletteItem> {
    [
        ("Save file", "w", "save"),
        ("Save and quit", "wq", "wq"),
        ("Quit", "q", "quit"),
        ("Force quit", "q!", "quit!"),
        ("Toggle explorer", "Ctrl+F", "explorer"),
        ("Toggle side terminal", "Ctrl+T", "terminal"),
        ("Pane / full terminal", "Ctrl+Shift+T", "terminal_full"),
        ("Source Control", "Ctrl+G", "scm"),
        ("Git workbench", "Ctrl+Shift+G", "git"),
        ("Settings", "Ctrl+,", "settings"),
        ("Preview document", "Ctrl+Shift+V", "preview"),
        ("Command panel (XLC)", ":", "xlc"),
        ("Next tab", "gt", "tab_next"),
        ("Previous tab", "gT", "tab_prev"),
        ("Close tab", ":bd", "tab_close"),
        ("Theme: ocean", "", "theme:ocean"),
        ("Theme: monokai", "", "theme:monokai"),
        ("Theme: nord", "", "theme:nord"),
        ("Theme: gruvbox", "", "theme:gruvbox"),
        ("Theme: sakura", "", "theme:sakura"),
        ("Show problems", "", "problems"),
        ("Go to definition", "gd", "lsp_def"),
        ("Peek definition", "gp", "lsp_peek"),
        ("Format document", "Ctrl+Shift+I", "format"),
        ("Code actions / Quick fix", "Ctrl+.", "code_action"),
        ("Document symbols", "gO / Ctrl+Shift+O", "symbols"),
        ("Workspace symbols", "", "workspace_symbols"),
        ("Find in files", "Ctrl+Shift+F", "workspace_find"),
        ("Split vertical", "Ctrl+W v", "split_v"),
        ("Split horizontal", "Ctrl+W s", "split_h"),
        ("Find files", "Ctrl+P", "files"),
        ("Help", ":help", "help"),
    ]
    .into_iter()
    .map(|(label, detail, id)| PaletteItem {
        label: label.into(),
        detail: detail.into(),
        action: PaletteAction::Command(id),
    })
    .collect()
}

fn collect_files(root: &Path, max: usize) -> Vec<PaletteItem> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let skip = ["target", "node_modules", ".git", "dist", "build", ".xei"];

    while let Some(dir) = stack.pop() {
        if out.len() >= max {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if out.len() >= max {
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && name != ".env" {
                continue;
            }
            if path.is_dir() {
                if skip.iter().any(|s| *s == name) {
                    continue;
                }
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| path.display().to_string());
                out.push(PaletteItem {
                    label: rel,
                    detail: String::new(),
                    action: PaletteAction::OpenFile(path),
                });
            }
        }
    }
    out.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_subsequence() {
        assert!(fuzzy_score("src/main.rs", "smr").is_some());
        assert!(fuzzy_score("src/main.rs", "xyz").is_none());
    }

    #[test]
    fn filter_commands() {
        let mut p = Palette::new();
        p.open_commands();
        p.query = "save".into();
        p.refilter();
        assert!(!p.filtered.is_empty());
    }
}

#[cfg(test)]
mod fuzzy_tests {
    use super::*;

    fn item(label: &str) -> PaletteItem {
        PaletteItem {
            label: label.into(),
            detail: String::new(),
            action: PaletteAction::Command("noop"),
        }
    }

    fn ranked(files: &[&str], query: &str) -> Vec<String> {
        let mut p = Palette::new();
        p.items = files.iter().map(|f| item(f)).collect();
        p.query = query.into();
        p.refilter();
        p.filtered.iter().map(|&i| p.items[i].label.clone()).collect()
    }

    /// The bug this replaces: an unranked subsequence test left the wanted file
    /// wherever the filesystem walk happened to put it, and the face only draws
    /// the first 40.
    #[test]
    fn the_real_file_outranks_incidental_path_matches() {
        let out = ranked(
            &[
                "var/tmp/sysdiagnose_2026_macOS_gu_i_edit_junk/acdi.log",
                "suisei-core/src/gui_edit.rs",
                "var/folders/g/u/i/e/d/i/t/cache",
            ],
            "gui_edit",
        );
        assert_eq!(out.first().map(String::as_str), Some("suisei-core/src/gui_edit.rs"));
    }

    #[test]
    fn a_basename_hit_beats_a_directory_hit() {
        let out = ranked(&["lsp/src/other.rs", "core/src/lsp.rs"], "lsp");
        assert_eq!(out.first().map(String::as_str), Some("core/src/lsp.rs"));
    }

    #[test]
    fn consecutive_characters_win_over_scattered_ones() {
        let out = ranked(&["a/p/p/l/e/x.rs", "src/apple.rs"], "apple");
        assert_eq!(out.first().map(String::as_str), Some("src/apple.rs"));
    }

    #[test]
    fn a_needle_that_does_not_fit_is_dropped() {
        assert!(ranked(&["src/main.rs"], "zzz").is_empty());
    }

    #[test]
    fn an_empty_query_keeps_everything_in_order() {
        assert_eq!(ranked(&["b.rs", "a.rs"], ""), vec!["b.rs", "a.rs"]);
    }
}
