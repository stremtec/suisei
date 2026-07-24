//! Git gutter signs from `git diff` (working tree vs HEAD) + optional blame.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitSign {
    /// New line in working tree
    Added,
    /// Changed line
    Modified,
    /// Deletion adjacent to this line (show marker on the following line)
    Deleted,
}

#[derive(Debug, Clone, Default)]
pub struct BlameLine {
    /// Short author (truncated)
    pub author: String,
    /// 7-char commit hash
    pub hash: String,
    /// Short date YYYY-MM-DD if known
    pub date: String,
}

/// Full blame column width when open (cells).
pub const BLAME_PANEL_WIDTH: u16 = 28;
/// Slide-open / slide-close duration (ms).
pub const BLAME_ANIM_MS: u64 = 300;

#[derive(Debug, Clone)]
pub struct GitBlame {
    /// 0-based line → blame info
    pub lines: HashMap<usize, BlameLine>,
    pub path: String,
    /// Panel open (or closing animation in progress).
    pub open: bool,
    pub available: bool,
    /// Legacy inline mode (`gb` line suffix) — kept for optional use.
    pub enabled: bool,
    // ── open animation (SCM-style openness) ──
    pub closing: bool,
    pub anim_from: f32,
    pub anim_to: f32,
    pub anim_pending: bool,
    pub opened_at: Option<std::time::Instant>,
}

impl Default for GitBlame {
    fn default() -> Self {
        Self {
            lines: HashMap::new(),
            path: String::new(),
            open: false,
            available: false,
            enabled: false,
            closing: false,
            anim_from: 0.0,
            anim_to: 0.0,
            anim_pending: false,
            opened_at: None,
        }
    }
}

impl GitBlame {
    pub fn clear(&mut self) {
        self.lines.clear();
        self.path.clear();
        self.available = false;
        self.enabled = false;
        self.open = false;
        self.closing = false;
        self.opened_at = None;
        self.anim_pending = false;
    }

    /// Whether the blame column should take layout space (open or animating).
    pub fn visible(&self) -> bool {
        self.open || self.closing
    }

    /// Linear openness 0..=1 for UI easing.
    pub fn anim_progress(&mut self) -> f32 {
        let v = self.tick_openness();
        if self.closing && v <= 0.001 {
            self.finish_close();
        }
        v
    }

    fn snapshot_openness(&self) -> f32 {
        if self.anim_pending {
            return self.anim_from;
        }
        let Some(t0) = self.opened_at else {
            return if self.open && !self.closing {
                1.0
            } else {
                0.0
            };
        };
        let u = (t0.elapsed().as_millis() as f32 / BLAME_ANIM_MS as f32).min(1.0);
        self.anim_from + (self.anim_to - self.anim_from) * u
    }

    fn tick_openness(&mut self) -> f32 {
        if self.anim_pending {
            self.anim_pending = false;
            self.opened_at = Some(std::time::Instant::now());
            return self.anim_from;
        }
        self.snapshot_openness()
    }

    fn finish_close(&mut self) {
        self.open = false;
        self.closing = false;
        self.enabled = false;
        self.opened_at = None;
        self.anim_pending = false;
        self.anim_from = 0.0;
        self.anim_to = 0.0;
    }

    /// Open panel with slide-in (loads blame for `path`).
    pub fn open_panel(&mut self, path: &str) -> String {
        // Optimistic open — `git blame` runs on a background thread and the
        // panel fills (or closes with a message) when the result lands.
        self.path = path.to_string();
        let from = if self.open || self.closing {
            self.snapshot_openness()
        } else {
            0.0
        };
        self.open = true;
        self.closing = false;
        self.enabled = true;
        self.anim_from = from;
        self.anim_to = 1.0;
        self.anim_pending = true;
        self.opened_at = None;
        if self.lines.is_empty() {
            "Blame · loading… · Ctrl+B close".into()
        } else {
            format!("Blame · {} lines · Ctrl+B close", self.lines.len())
        }
    }

    /// Slide-out close.
    pub fn close_panel(&mut self) {
        if !self.open || self.closing {
            if !self.open {
                self.enabled = false;
            }
            return;
        }
        let cur = self.snapshot_openness();
        self.closing = true;
        self.anim_from = cur;
        self.anim_to = 0.0;
        self.anim_pending = true;
        self.opened_at = None;
    }

    pub fn toggle_panel(&mut self, path: &str) -> String {
        if self.open && !self.closing {
            self.close_panel();
            "Blame closing…".into()
        } else if self.closing {
            // reopen mid-close
            self.open_panel(path)
        } else {
            self.open_panel(path)
        }
    }

    /// Legacy inline toggle (`gb`) — same panel for consistency.
    pub fn toggle(&mut self, path: &str) -> String {
        self.toggle_panel(path)
    }

    /// Sync wrapper (hot paths use App's async channel + `compute_blame`).
    pub fn refresh(&mut self, path: &str) {
        let (available, lines) = compute_blame(path);
        self.path = path.to_string();
        self.available = available;
        self.lines = lines;
    }

    pub fn at(&self, row: usize) -> Option<&BlameLine> {
        if self.enabled || self.open {
            self.lines.get(&row)
        } else {
            None
        }
    }
}

/// Fixed **flame** palette — independent of editor theme.

/// Blocking blame computation (runs on a background thread for the panel).
pub fn compute_blame(path: &str) -> (bool, HashMap<usize, BlameLine>) {
    let mut lines = HashMap::new();
    if path.is_empty() {
        return (false, lines);
    }
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf());
    let Some(parent) = abs.parent() else {
        return (false, lines);
    };
    let output = Command::new("git")
        .args([
            "blame",
            "--line-porcelain",
            "--",
            abs.to_str().unwrap_or(path),
        ])
        .current_dir(parent)
        .output();
    let Ok(out) = output else {
        return (false, lines);
    };
    if !out.status.success() {
        return (false, lines);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_blame_porcelain(&text, &mut lines);
    (!lines.is_empty(), lines)
}

pub fn flame_color_for(key: &str) -> (u8, u8, u8) {
    // Warm fire: deep red → orange → gold → ember
    const FLAME: &[(u8, u8, u8)] = &[
        (255, 48, 20),   // core red
        (255, 90, 25),   // orange-red
        (255, 130, 30),  // orange
        (255, 170, 40),  // amber
        (255, 200, 55),  // gold
        (255, 110, 45),  // ember
        (255, 70, 35),   // flame edge
        (255, 150, 60),  // bright orange
    ];
    let h = key
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(33).wrapping_add(b as u32));
    FLAME[(h as usize) % FLAME.len()]
}

/// Column width for current anim openness (eased in UI).
pub fn blame_width_for_openness(t: f32) -> u16 {
    let t = t.clamp(0.0, 1.0);
    ((BLAME_PANEL_WIDTH as f32) * t).round() as u16
}

/// Parse `git blame --line-porcelain` into per-line info.
pub fn parse_blame_porcelain(text: &str, out: &mut HashMap<usize, BlameLine>) {
    let mut hash = String::new();
    let mut author = String::new();
    let mut date = String::new();
    let mut line_no: Option<usize> = None; // 0-based final line

    for line in text.lines() {
        if line.len() >= 40 && line.as_bytes().get(0).is_some_and(|b| b.is_ascii_hexdigit()) {
            // header: hash orig final [group]
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                hash = parts[0].chars().take(7).collect();
                if let Ok(n) = parts[2].parse::<usize>() {
                    line_no = Some(n.saturating_sub(1));
                }
            }
            author.clear();
            date.clear();
        } else if let Some(a) = line.strip_prefix("author ") {
            author = a.chars().take(12).collect();
        } else if let Some(t) = line.strip_prefix("author-time ") {
            // unix timestamp → rough date via optional; keep raw short
            if let Ok(secs) = t.parse::<i64>() {
                // minimal YYYY without chrono: leave short stamp
                date = format!("{secs}");
                // Prefer author-mail skip; use time only if no better
            }
        } else if let Some(d) = line.strip_prefix("author-time ") {
            let _ = d;
        } else if line.starts_with('\t') {
            if let Some(row) = line_no {
                let auth = if author.is_empty() {
                    "?".into()
                } else {
                    author.clone()
                };
                out.insert(
                    row,
                    BlameLine {
                        author: auth,
                        hash: hash.clone(),
                        date: date.clone(),
                    },
                );
            }
            line_no = None;
        }
    }
}


/// Blocking gutter computation (runs on a background thread).
pub fn compute_gutter(path: &str) -> (bool, HashMap<usize, GitSign>) {
    let mut signs = HashMap::new();
    if path.is_empty() {
        return (false, signs);
    }
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf());
    let Some(parent) = abs.parent() else {
        return (false, signs);
    };
    let output = Command::new("git")
        .args([
            "diff",
            "HEAD",
            "--no-color",
            "-U0",
            "--",
            abs.to_str().unwrap_or(path),
        ])
        .current_dir(parent)
        .output();
    let Ok(out) = output else {
        return (false, signs);
    };
    if !out.status.success() && out.stdout.is_empty() {
        return (false, signs);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_diff_hunks(&text, &mut signs);
    (true, signs)
}

#[derive(Debug, Default, Clone)]
pub struct GitGutter {
    /// 0-based buffer line → sign
    pub signs: HashMap<usize, GitSign>,
    pub path: String,
    pub available: bool,
}

impl GitGutter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.signs.clear();
        self.path.clear();
        self.available = false;
    }

    /// Sync wrapper (hot paths use App's async channel + `compute_gutter`).
    pub fn refresh(&mut self, path: &str) {
        let (available, signs) = compute_gutter(path);
        self.path = path.to_string();
        self.available = available;
        self.signs = signs;
    }

    pub fn sign_at(&self, row: usize) -> Option<GitSign> {
        self.signs.get(&row).copied()
    }
}

/// Format blame for a narrow gutter: `ab  a1b2c3d`
pub fn format_blame_gutter(b: &BlameLine, width: usize) -> String {
    let s = format!("{:<8} {}", b.author.chars().take(8).collect::<String>(), b.hash);
    s.chars().take(width).collect()
}

/// Parse unified diff hunks (`@@ -old,oc +new,nc @@`) into line signs.
pub fn parse_diff_hunks(diff: &str, signs: &mut HashMap<usize, GitSign>) {
    for line in diff.lines() {
        if !line.starts_with("@@") {
            continue;
        }
        // @@ -l,s +l,s @@
        let Some(rest) = line.strip_prefix("@@") else {
            continue;
        };
        let parts: Vec<&str> = rest.split_whitespace().collect();
        // expect at least -old +new
        let mut old_count = 1i64;
        let mut new_start = 0i64;
        let mut new_count = 1i64;
        for p in parts {
            if let Some(spec) = p.strip_prefix('-') {
                let (_s, c) = parse_hunk_spec(spec);
                old_count = c;
            } else if let Some(spec) = p.strip_prefix('+') {
                let (s, c) = parse_hunk_spec(spec);
                new_start = s;
                new_count = c;
            }
        }

        // 1-based → 0-based for new file lines
        if old_count == 0 && new_count > 0 {
            // pure addition
            for i in 0..new_count as usize {
                let row = (new_start - 1).max(0) as usize + i;
                signs.insert(row, GitSign::Added);
            }
        } else if new_count == 0 && old_count > 0 {
            // pure deletion — mark the line after the deletion point (or previous)
            let row = if new_start > 0 {
                (new_start as usize).saturating_sub(1)
            } else {
                0
            };
            signs.entry(row).or_insert(GitSign::Deleted);
        } else {
            // modification / mix
            let n = new_count.max(0) as usize;
            let o = old_count.max(0) as usize;
            let base = (new_start - 1).max(0) as usize;
            for i in 0..n {
                let row = base + i;
                if i < o {
                    signs.insert(row, GitSign::Modified);
                } else {
                    signs.insert(row, GitSign::Added);
                }
            }
            if o > n {
                signs.entry(base.saturating_add(n.saturating_sub(1)).max(0))
                    .or_insert(GitSign::Deleted);
            }
        }
    }
}

fn parse_hunk_spec(spec: &str) -> (i64, i64) {
    // "10" or "10,3"
    if let Some((a, b)) = spec.split_once(',') {
        let s = a.parse().unwrap_or(0);
        let c = b.parse().unwrap_or(1);
        (s, c)
    } else {
        let s = spec.parse().unwrap_or(0);
        (s, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_added_lines() {
        let mut m = HashMap::new();
        parse_diff_hunks("@@ -5,0 +6,2 @@\n+a\n+b\n", &mut m);
        assert_eq!(m.get(&5), Some(&GitSign::Added));
        assert_eq!(m.get(&6), Some(&GitSign::Added));
    }

    #[test]
    fn parse_modified_line() {
        let mut m = HashMap::new();
        parse_diff_hunks("@@ -10,1 +10,1 @@\n-old\n+new\n", &mut m);
        assert_eq!(m.get(&9), Some(&GitSign::Modified));
    }

    #[test]
    fn parse_deleted() {
        let mut m = HashMap::new();
        parse_diff_hunks("@@ -3,2 +3,0 @@\n-a\n-b\n", &mut m);
        assert!(m.values().any(|s| *s == GitSign::Deleted));
    }
}
