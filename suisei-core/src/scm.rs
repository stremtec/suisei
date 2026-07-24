//! VS Code–style Source Control (SCM) panel state.
//!
//! Model mirrors VS Code's built-in Git provider:
//! - resource groups: **Staged** (index) + **Changes** (working tree)
//! - commit message input + Commit action
//! - pretty commit graph via [`crate::git_graph`]

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::git_graph::{self, GraphRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScmStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflict,
    TypeChange,
    Unknown,
}

impl ScmStatus {
    pub fn letter(self) -> char {
        match self {
            ScmStatus::Modified => 'M',
            ScmStatus::Added => 'A',
            ScmStatus::Deleted => 'D',
            ScmStatus::Renamed => 'R',
            ScmStatus::Untracked => 'U',
            ScmStatus::Conflict => 'C',
            ScmStatus::TypeChange => 'T',
            ScmStatus::Unknown => '?',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ScmStatus::Modified => "Modified",
            ScmStatus::Added => "Added",
            ScmStatus::Deleted => "Deleted",
            ScmStatus::Renamed => "Renamed",
            ScmStatus::Untracked => "Untracked",
            ScmStatus::Conflict => "Conflict",
            ScmStatus::TypeChange => "Type change",
            ScmStatus::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScmEntry {
    pub path: String,
    pub status: ScmStatus,
    /// true = index/staged group
    pub staged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScmFocus {
    /// Typing the commit message
    Message,
    /// "✓ Commit" action row
    CommitButton,
    /// File list (staged + changes)
    Changes,
    /// Recent graph
    Graph,
}

#[derive(Debug, Clone)]
pub struct ScmPanel {
    pub open: bool,
    pub message: String,
    pub focus: ScmFocus,
    /// Selected index in the flattened changes list (staged first, then unstaged)
    pub selected: usize,
    pub staged: Vec<ScmEntry>,
    pub changes: Vec<ScmEntry>,
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    /// Pretty commit graph rows (newest first)
    pub graph: Vec<GraphRow>,
    /// Selected commit in the graph (when focus is Graph)
    pub graph_selected: usize,
    /// How many commits to request from `git log` (grows with load-more)
    pub graph_limit: usize,
    pub root: Option<PathBuf>,
    pub error: Option<String>,
    pub last_result: Option<String>,
    /// Animation clock — armed on open/close, started by the first render so
    /// synchronous `git status` / `git log` refresh can't eat the window.
    pub opened_at: Option<std::time::Instant>,
    pub anim_pending: bool,
    /// Openness at phase start / end (0 = off-screen, 1 = fully visible).
    pub anim_from: f32,
    pub anim_to: f32,
    /// True while a close animation is playing (`open` stays true until done).
    pub closing: bool,
    /// Set for one frame when the close animation settles (app clears mode).
    pub just_closed: bool,
    /// Defer expensive graph layout until first paint (open feels instant).
    pub graph_pending: bool,
}

/// Slide animation length (ms) — open and close.
pub const SCM_ANIM_MS: u64 = 320;

/// Default commits for light SCM graph (keep small — workbench has full History).
pub const GRAPH_DEFAULT_LIMIT: usize = 40;
/// Upper cap so we don't hang on huge monorepos.
pub const GRAPH_MAX_LIMIT: usize = 2000;

impl Default for ScmPanel {
    fn default() -> Self {
        Self {
            open: false,
            message: String::new(),
            focus: ScmFocus::Message,
            selected: 0,
            staged: Vec::new(),
            changes: Vec::new(),
            branch: String::new(),
            ahead: 0,
            behind: 0,
            graph: Vec::new(),
            graph_selected: 0,
            graph_limit: GRAPH_DEFAULT_LIMIT,
            root: None,
            error: None,
            last_result: None,
            opened_at: None,
            anim_pending: false,
            anim_from: 0.0,
            anim_to: 1.0,
            closing: false,
            just_closed: false,
            graph_pending: false,
        }
    }
}

impl ScmPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the panel should be painted (open or mid close-anim).
    pub fn visible(&self) -> bool {
        self.open
    }

    pub fn is_animating(&self) -> bool {
        self.anim_pending
            || self.closing
            || self
                .opened_at
                .is_some_and(|t| t.elapsed().as_millis() < SCM_ANIM_MS as u128)
    }

    /// Instant hide (no animation) — used when switching to another overlay.
    pub fn close_immediate(&mut self) {
        self.open = false;
        self.closing = false;
        self.just_closed = false;
        self.error = None;
        self.opened_at = None;
        self.anim_pending = false;
        self.anim_from = 0.0;
        self.anim_to = 0.0;
    }

    /// Start slide-out close. Keeps `open` until the animation finishes.
    pub fn close(&mut self) {
        if !self.open || self.closing {
            return;
        }
        let current = self.snapshot_openness();
        self.closing = true;
        self.anim_from = current;
        self.anim_to = 0.0;
        self.anim_pending = true;
        self.opened_at = None;
    }

    /// Open and refresh against repo containing `hint_path` (or cwd).
    pub fn open_and_refresh(&mut self, hint_path: Option<&Path>) {
        self.open = true;
        self.closing = false;
        self.focus = ScmFocus::Message;
        // Arm open animation; clock starts at first rendered frame
        // (refresh below shells out to git and can take a long time).
        let from = if self.opened_at.is_some() {
            self.snapshot_openness()
        } else {
            0.0
        };
        self.anim_from = from;
        self.anim_to = 1.0;
        self.anim_pending = true;
        self.opened_at = None;
        // Status only on open — graph loads on first draw (see ensure_graph).
        self.refresh_status(hint_path);
        self.graph_pending = true;
        self.graph.clear();
    }

    /// Status/branch/changes only — no `git log` / graph layout.
    pub fn refresh_status(&mut self, hint_path: Option<&Path>) {
        self.error = None;
        self.staged.clear();
        self.changes.clear();
        self.branch.clear();
        self.ahead = 0;
        self.behind = 0;

        let root = find_git_root(hint_path);
        self.root = root.clone();
        let Some(root) = root else {
            self.error = Some("Not a git repository".into());
            return;
        };

        if let Some((branch, ahead, behind)) = parse_branch_status(&root) {
            self.branch = branch;
            self.ahead = ahead;
            self.behind = behind;
        }

        match run_git(&root, &["status", "--porcelain=v1", "-uall"]) {
            Ok(out) => self.refresh_entries_from_status(&out),
            Err(e) => {
                self.error = Some(e);
                return;
            }
        }
        self.clamp_selected();
    }

    /// Load graph if deferred (call from UI once per open).
    pub fn ensure_graph(&mut self) {
        if !self.graph_pending {
            return;
        }
        self.graph_pending = false;
        if let Some(ref root) = self.root.clone() {
            self.reload_graph(root);
        }
    }

    /// Linear **openness** 0.0..=1.0 (0 = off-screen, 1 = fully shown).
    /// Easing is applied in the UI. First call after arming starts the clock.
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
        let u = (t0.elapsed().as_millis() as f32 / SCM_ANIM_MS as f32).min(1.0);
        self.anim_from + (self.anim_to - self.anim_from) * u
    }

    fn tick_openness(&mut self) -> f32 {
        if self.anim_pending {
            self.anim_pending = false;
            self.opened_at = Some(std::time::Instant::now());
            return self.anim_from;
        }
        let Some(t0) = self.opened_at else {
            return if self.open && !self.closing {
                1.0
            } else {
                0.0
            };
        };
        let u = (t0.elapsed().as_millis() as f32 / SCM_ANIM_MS as f32).min(1.0);
        self.anim_from + (self.anim_to - self.anim_from) * u
    }

    fn finish_close(&mut self) {
        self.open = false;
        self.closing = false;
        self.just_closed = true;
        self.error = None;
        self.opened_at = None;
        self.anim_pending = false;
        self.anim_from = 0.0;
        self.anim_to = 0.0;
    }

    /// Returns true once when a close animation has settled.
    pub fn take_just_closed(&mut self) -> bool {
        if self.just_closed {
            self.just_closed = false;
            true
        } else {
            false
        }
    }

    pub fn total_files(&self) -> usize {
        self.staged.len() + self.changes.len()
    }

    pub fn entry_at(&self, idx: usize) -> Option<&ScmEntry> {
        if idx < self.staged.len() {
            self.staged.get(idx)
        } else {
            self.changes.get(idx - self.staged.len())
        }
    }

    pub fn clamp_selected(&mut self) {
        let n = self.total_files();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    pub fn move_sel(&mut self, delta: isize) {
        let n = self.total_files();
        if n == 0 {
            self.selected = 0;
            return;
        }
        let cur = self.selected as isize + delta;
        self.selected = cur.clamp(0, (n - 1) as isize) as usize;
    }

    pub fn refresh(&mut self, hint_path: Option<&Path>) {
        self.refresh_status(hint_path);
        // Only rebuild graph when the panel is open (status-bar refresh skips it).
        if self.open {
            if let Some(ref root) = self.root.clone() {
                self.reload_graph(root);
            }
            self.graph_pending = false;
        } else {
            self.graph.clear();
            self.graph_pending = false;
        }
    }

    fn reload_graph(&mut self, root: &Path) {
        // Pretty graph: topology + decorations + author/time
        // Use --all so gc-reachable tips on other branches still appear.
        // Avoid packing issues: plain `log` walks the commit graph (not only reflog).
        let limit = self.graph_limit.clamp(50, GRAPH_MAX_LIMIT).to_string();
        if let Ok(out) = run_git(
            root,
            &[
                "log",
                "--all",
                "--date-order",
                "-n",
                &limit,
                "--pretty=format:%H%x00%h%x00%P%x00%d%x00%s%x00%an%x00%ar",
            ],
        ) {
            self.graph = git_graph::build_graph(&out);
            if self.graph_selected >= self.graph.len() {
                self.graph_selected = self.graph.len().saturating_sub(1);
            }
        }
    }

    /// Fetch more history (double limit, capped). Call while graph is focused.
    pub fn load_more_graph(&mut self) -> Result<usize, String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let prev = self.graph.len();
        let next = (self.graph_limit.saturating_mul(2)).min(GRAPH_MAX_LIMIT);
        if next <= self.graph_limit && self.graph_limit >= GRAPH_MAX_LIMIT {
            return Ok(prev);
        }
        self.graph_limit = next.max(self.graph_limit + 100);
        self.reload_graph(&root);
        Ok(self.graph.len().saturating_sub(prev))
    }

    pub fn move_graph_sel(&mut self, delta: isize) {
        let n = self.graph.len();
        if n == 0 {
            self.graph_selected = 0;
            return;
        }
        let cur = self.graph_selected as isize + delta;
        self.graph_selected = cur.clamp(0, (n - 1) as isize) as usize;
        // Near the bottom → auto load more history
        if self.graph_selected + 5 >= n && self.graph_limit < GRAPH_MAX_LIMIT {
            let _ = self.load_more_graph();
        }
    }

    pub fn selected_graph_row(&self) -> Option<&GraphRow> {
        self.graph.get(self.graph_selected)
    }

    /// Stage the selected file (or all unstaged if none selected / with `all`).
    pub fn stage_selected(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        if let Some(e) = self.entry_at(self.selected).cloned() {
            if e.staged {
                // already staged — unstage
                run_git(&root, &["restore", "--staged", "--", &e.path])?;
            } else {
                run_git(&root, &["add", "--", &e.path])?;
            }
        }
        self.refresh(Some(&root));
        Ok(())
    }

    pub fn stage_all(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        run_git(&root, &["add", "-A"])?;
        self.refresh(Some(&root));
        self.last_result = Some("Staged all changes".into());
        Ok(())
    }

    pub fn unstage_all(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        run_git(&root, &["restore", "--staged", "."])?;
        self.refresh(Some(&root));
        self.last_result = Some("Unstaged all".into());
        Ok(())
    }

    pub fn discard_selected(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let e = self
            .entry_at(self.selected)
            .cloned()
            .ok_or_else(|| "No file selected".to_string())?;
        if e.staged {
            run_git(&root, &["restore", "--staged", "--", &e.path])?;
        }
        match e.status {
            ScmStatus::Untracked | ScmStatus::Added => {
                // remove untracked carefully
                let p = root.join(&e.path);
                if p.is_file() {
                    std::fs::remove_file(&p).map_err(|err| err.to_string())?;
                }
            }
            _ => {
                run_git(&root, &["restore", "--", &e.path])?;
            }
        }
        self.refresh(Some(&root));
        self.last_result = Some(format!("Discarded {}", e.path));
        Ok(())
    }

    /// Commit staged changes with current message. If nothing staged, stage all first (VS Code default often requires staged; we match VS Code "Commit" on staged only unless message + commit all shortcut).
    pub fn commit(&mut self, amend: bool) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let msg = self.message.trim().to_string();
        if msg.is_empty() && !amend {
            return Err("Commit message is empty".into());
        }
        if self.staged.is_empty() && !amend {
            // VS Code: commit only staged.
            if self.changes.is_empty() {
                return Err("No changes to commit".into());
            }
            return Err("No staged changes — press `a` to stage all, or Space on a file".into());
        }
        let out = if amend {
            if msg.is_empty() {
                run_git(&root, &["commit", "--amend", "--no-edit"])?
            } else {
                run_git(&root, &["commit", "--amend", "-m", &msg])?
            }
        } else {
            run_git(&root, &["commit", "-m", &msg])?
        };
        self.message.clear();
        self.refresh(Some(&root));
        let summary = out.lines().next().unwrap_or("Committed").to_string();
        self.last_result = Some(summary);
        Ok(())
    }

    pub fn cycle_focus(&mut self, forward: bool) {
        self.focus = if forward {
            match self.focus {
                ScmFocus::Message => ScmFocus::CommitButton,
                ScmFocus::CommitButton => ScmFocus::Changes,
                ScmFocus::Changes => ScmFocus::Graph,
                ScmFocus::Graph => ScmFocus::Message,
            }
        } else {
            match self.focus {
                ScmFocus::Message => ScmFocus::Graph,
                ScmFocus::CommitButton => ScmFocus::Message,
                ScmFocus::Changes => ScmFocus::CommitButton,
                ScmFocus::Graph => ScmFocus::Changes,
            }
        };
    }
}

fn find_git_root(hint: Option<&Path>) -> Option<PathBuf> {
    let start = hint
        .and_then(|p| {
            if p.is_file() {
                p.parent().map(|x| x.to_path_buf())
            } else {
                Some(p.to_path_buf())
            }
        })
        .or_else(|| std::env::current_dir().ok())?;

    let mut cur = start;
    for _ in 0..24 {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

fn run_git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git failed to start: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let err = err.trim();
        if err.is_empty() {
            return Err(format!("git {} failed", args.first().unwrap_or(&"")));
        }
        return Err(err.lines().next().unwrap_or("git error").to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_branch_status(root: &Path) -> Option<(String, u32, u32)> {
    // ## main...origin/main [ahead 1, behind 2]
    let out = run_git(root, &["status", "-sb"]).ok()?;
    let first = out.lines().next()?;
    let rest = first.strip_prefix("## ")?;
    let branch = rest
        .split(['.', ' ', '['])
        .next()
        .unwrap_or(rest)
        .to_string();
    let mut ahead = 0u32;
    let mut behind = 0u32;
    if let Some(idx) = rest.find('[') {
        let bracket = &rest[idx..];
        if let Some(a) = bracket.split("ahead ").nth(1) {
            ahead = a
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0);
        }
        if let Some(b) = bracket.split("behind ").nth(1) {
            behind = b
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0);
        }
    }
    Some((branch, ahead, behind))
}

fn status_from_code(c: char) -> ScmStatus {
    match c {
        'M' => ScmStatus::Modified,
        'A' => ScmStatus::Added,
        'D' => ScmStatus::Deleted,
        'R' => ScmStatus::Renamed,
        'C' => ScmStatus::Conflict, // also copy — treat as conflict-ish
        'U' => ScmStatus::Conflict,
        'T' => ScmStatus::TypeChange,
        '?' => ScmStatus::Untracked,
        _ => ScmStatus::Unknown,
    }
}

/// Parse porcelain line into 0–2 entries (staged and/or unstaged).
pub fn parse_porcelain_entries(line: &str) -> Vec<ScmEntry> {
    let mut out = Vec::new();
    if line.len() < 4 {
        return out;
    }
    let bytes = line.as_bytes();
    let x = bytes[0] as char;
    let y = bytes[1] as char;
    let path_part = match line.get(3..) {
        Some(p) => p.trim(),
        None => return out,
    };
    if path_part.is_empty() {
        return out;
    }
    let path = if let Some((_, new)) = path_part.split_once(" -> ") {
        new.to_string()
    } else {
        path_part.to_string()
    };

    if x == '?' && y == '?' {
        out.push(ScmEntry {
            path,
            status: ScmStatus::Untracked,
            staged: false,
        });
        return out;
    }
    if x == '!' {
        return out;
    }

    // Unmerged
    if matches!(x, 'U' | 'A' | 'D') && matches!(y, 'U' | 'A' | 'D') && (x == 'U' || y == 'U') {
        out.push(ScmEntry {
            path,
            status: ScmStatus::Conflict,
            staged: false,
        });
        return out;
    }

    if x != ' ' && x != '?' {
        out.push(ScmEntry {
            path: path.clone(),
            status: status_from_code(x),
            staged: true,
        });
    }
    if y != ' ' && y != '?' {
        out.push(ScmEntry {
            path,
            status: status_from_code(y),
            staged: false,
        });
    }
    out
}

impl ScmPanel {
    pub fn refresh_entries_from_status(&mut self, porcelain: &str) {
        self.staged.clear();
        self.changes.clear();
        for line in porcelain.lines() {
            for e in parse_porcelain_entries(line) {
                if e.staged {
                    self.staged.push(e);
                } else {
                    self.changes.push(e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_modified_worktree() {
        let e = parse_porcelain_entries(" M src/main.rs");
        assert_eq!(e.len(), 1);
        assert!(!e[0].staged);
        assert_eq!(e[0].status, ScmStatus::Modified);
        assert_eq!(e[0].path, "src/main.rs");
    }

    #[test]
    fn porcelain_staged_and_unstaged() {
        let e = parse_porcelain_entries("MM app.rs");
        assert_eq!(e.len(), 2);
        assert!(e[0].staged);
        assert!(!e[1].staged);
    }

    #[test]
    fn porcelain_untracked() {
        let e = parse_porcelain_entries("?? new.txt");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].status, ScmStatus::Untracked);
    }

    #[test]
    fn porcelain_renamed() {
        let e = parse_porcelain_entries("R  old.rs -> new.rs");
        assert_eq!(e.len(), 1);
        assert!(e[0].staged);
        assert_eq!(e[0].path, "new.rs");
        assert_eq!(e[0].status, ScmStatus::Renamed);
    }

    #[test]
    fn status_letters() {
        assert_eq!(ScmStatus::Modified.letter(), 'M');
        assert_eq!(ScmStatus::Untracked.letter(), 'U');
    }
}
