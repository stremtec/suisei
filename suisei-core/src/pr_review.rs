//! PR review surface — files + review comments via `gh`.
//!
//! All `gh` calls (network) run on background threads; the UI polls
//! [`PrReviewState::poll`] each frame, mirroring the git-workbench loader.
//! Per-file diffs are debounced so holding `j`/`k` spawns one fetch, not one
//! per keypress.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Selection must rest this long before the diff fetch fires.
const DIFF_DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone)]
pub struct PrFile {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone)]
pub struct PrComment {
    pub path: String,
    /// 1-based line in the new file when known
    pub line: Option<u32>,
    pub author: String,
    pub body: String,
    pub url: String,
}

/// Fetched PR header + file list (background-thread result).
#[derive(Debug, Clone)]
pub struct PrDetail {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub url: String,
    pub base: String,
    pub head: String,
    pub files: Vec<PrFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrReviewFocus {
    Files,
    Comments,
    Body,
}

impl PrReviewFocus {
    pub fn next(self) -> Self {
        match self {
            PrReviewFocus::Files => PrReviewFocus::Comments,
            PrReviewFocus::Comments => PrReviewFocus::Body,
            PrReviewFocus::Body => PrReviewFocus::Files,
        }
    }
}

#[derive(Debug)]
pub struct PrReviewState {
    pub open: bool,
    /// PR header/files still loading in the background.
    pub loading: bool,
    /// Git root captured at open — background fetches and file opens use this.
    pub root: Option<PathBuf>,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub url: String,
    pub base: String,
    pub head: String,
    pub files: Vec<PrFile>,
    pub comments: Vec<PrComment>,
    pub file_sel: usize,
    pub comment_sel: usize,
    pub body_scroll: usize,
    pub focus: PrReviewFocus,
    pub message: String,
    /// Fetch failure — shown inline in the panel.
    pub error: Option<String>,
    /// Cached `gh pr diff` for selected file (unified)
    pub file_diff: Vec<String>,
    pub diff_scroll: usize,

    detail_rx: Option<Receiver<Result<(PrDetail, Vec<PrComment>), String>>>,
    diff_rx: Option<Receiver<(u64, Vec<String>)>>,
    /// Generation tag — only the newest in-flight diff result is applied.
    diff_gen: u64,
    /// Set when the selection moved; fetch fires once it rests DIFF_DEBOUNCE.
    diff_pending_since: Option<Instant>,
}

impl Default for PrReviewState {
    fn default() -> Self {
        Self {
            open: false,
            loading: false,
            root: None,
            number: 0,
            title: String::new(),
            body: String::new(),
            author: String::new(),
            url: String::new(),
            base: String::new(),
            head: String::new(),
            files: Vec::new(),
            comments: Vec::new(),
            file_sel: 0,
            comment_sel: 0,
            body_scroll: 0,
            focus: PrReviewFocus::Files,
            message: String::new(),
            error: None,
            file_diff: Vec::new(),
            diff_scroll: 0,
            detail_rx: None,
            diff_rx: None,
            diff_gen: 0,
            diff_pending_since: None,
        }
    }
}

impl PrReviewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn close(&mut self) {
        // In-flight threads finish and their send fails silently.
        *self = Self::default();
    }

    /// Kick off a background fetch of the PR. Errors only for fast
    /// preconditions; fetch results (or errors) arrive via [`poll`].
    pub fn open_pr(&mut self, root: &Path, number: u64) -> Result<(), String> {
        if !crate::gh::gh_installed() {
            return Err("gh not installed".into());
        }
        *self = Self::default();
        self.open = true;
        self.loading = true;
        self.root = Some(root.to_path_buf());
        self.number = number;
        self.message = format!("⏳ Loading PR #{number}…");
        let (tx, rx) = mpsc::channel();
        self.detail_rx = Some(rx);
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let result = fetch_pr_detail(&root, number).map(|detail| {
                let comments = fetch_pr_review_comments(&root, number).unwrap_or_default();
                (detail, comments)
            });
            let _ = tx.send(result);
        });
        Ok(())
    }

    /// Drain background results + fire debounced diff fetches.
    /// Call once per frame while open. Returns true when state changed.
    pub fn poll(&mut self) -> bool {
        if !self.open {
            return false;
        }
        let mut changed = false;

        if let Some(rx) = self.detail_rx.take() {
            match rx.try_recv() {
                Ok(Ok((detail, comments))) => {
                    self.apply_detail(detail, comments);
                    changed = true;
                }
                Ok(Err(e)) => {
                    self.loading = false;
                    self.message = e.clone();
                    self.error = Some(e);
                    changed = true;
                }
                Err(TryRecvError::Empty) => self.detail_rx = Some(rx),
                Err(TryRecvError::Disconnected) => {
                    self.loading = false;
                    changed = true;
                }
            }
        }

        if let Some(since) = self.diff_pending_since {
            if since.elapsed() >= DIFF_DEBOUNCE {
                self.diff_pending_since = None;
                self.spawn_diff_fetch();
            }
        }

        if let Some(rx) = self.diff_rx.take() {
            match rx.try_recv() {
                Ok((generation, lines)) => {
                    if generation == self.diff_gen {
                        self.file_diff = lines;
                        changed = true;
                    }
                    // Stale result — a newer fetch is (or was) in flight.
                }
                Err(TryRecvError::Empty) => self.diff_rx = Some(rx),
                Err(TryRecvError::Disconnected) => changed = true,
            }
        }
        changed
    }

    fn apply_detail(&mut self, d: PrDetail, comments: Vec<PrComment>) {
        self.loading = false;
        self.number = d.number;
        self.title = d.title;
        self.body = d.body;
        self.author = d.author;
        self.url = d.url;
        self.base = d.base;
        self.head = d.head;
        self.files = d.files;
        self.comments = comments;
        self.file_sel = 0;
        self.comment_sel = 0;
        self.message = format!(
            "PR #{} · {} file(s) · {} comment(s)",
            self.number,
            self.files.len(),
            self.comments.len()
        );
        self.schedule_diff();
    }

    /// Mouse: jump straight to a file row (schedules its diff).
    pub fn select_file(&mut self, idx: usize) {
        if idx >= self.files.len() {
            return;
        }
        self.focus = PrReviewFocus::Files;
        self.file_sel = idx;
        self.diff_scroll = 0;
        self.schedule_diff();
    }

    /// Mouse: jump straight to a comment row.
    pub fn select_comment(&mut self, idx: usize) {
        if idx >= self.comments.len() {
            return;
        }
        self.focus = PrReviewFocus::Comments;
        self.comment_sel = idx;
    }

    pub fn move_sel(&mut self, delta: isize) {
        match self.focus {
            PrReviewFocus::Files => {
                if self.files.is_empty() {
                    return;
                }
                let n = self.files.len() as isize;
                self.file_sel = (self.file_sel as isize + delta).rem_euclid(n) as usize;
                self.diff_scroll = 0;
                self.schedule_diff();
            }
            PrReviewFocus::Comments => {
                if self.comments.is_empty() {
                    return;
                }
                let n = self.comments.len() as isize;
                self.comment_sel = (self.comment_sel as isize + delta).rem_euclid(n) as usize;
            }
            PrReviewFocus::Body => {
                let max = self.body.lines().count().saturating_sub(1);
                let cur = self.body_scroll as isize + delta;
                self.body_scroll = cur.clamp(0, max as isize) as usize;
            }
        }
    }

    fn schedule_diff(&mut self) {
        self.file_diff = vec!["(loading diff…)".into()];
        self.diff_pending_since = Some(Instant::now());
    }

    fn spawn_diff_fetch(&mut self) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let Some(file) = self.files.get(self.file_sel) else {
            self.file_diff.clear();
            return;
        };
        self.diff_gen = self.diff_gen.wrapping_add(1);
        let generation = self.diff_gen;
        let number = self.number;
        let path = file.path.clone();
        let (tx, rx) = mpsc::channel();
        self.diff_rx = Some(rx);
        std::thread::spawn(move || {
            let lines = fetch_pr_file_diff(&root, number, &path)
                .unwrap_or_else(|e| vec![format!("(diff unavailable: {e})")]);
            let _ = tx.send((generation, lines));
        });
    }

    pub fn selected_file_path(&self) -> Option<&str> {
        self.files.get(self.file_sel).map(|f| f.path.as_str())
    }

    pub fn comments_for_selected_file(&self) -> Vec<&PrComment> {
        let Some(path) = self.selected_file_path() else {
            return Vec::new();
        };
        // Both `gh pr view --json files` and the REST comments API return
        // repo-relative paths, so exact equality is the correct match.
        self.comments.iter().filter(|c| c.path == path).collect()
    }
}

fn fetch_pr_detail(root: &Path, number: u64) -> Result<PrDetail, String> {
    let n = number.to_string();
    let out = Command::new("gh")
        .args([
            "pr",
            "view",
            &n,
            "--json",
            "number,title,body,author,url,baseRefName,headRefName,files",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("gh pr view failed").into());
    }
    parse_pr_view_json(&String::from_utf8_lossy(&out.stdout))
}

fn parse_pr_view_json(text: &str) -> Result<PrDetail, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("PR json: {e}"))?;
    let number = v.get("number").and_then(|n| n.as_u64()).unwrap_or(0);
    let title = v
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let body = v
        .get("body")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let author = v
        .get("author")
        .and_then(|a| a.get("login").or_else(|| a.get("name")))
        .and_then(|s| s.as_str())
        .unwrap_or("?")
        .to_string();
    let url = v
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let base = v
        .get("baseRefName")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let head = v
        .get("headRefName")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let mut files = Vec::new();
    if let Some(arr) = v.get("files").and_then(|f| f.as_array()) {
        for f in arr {
            let path = f
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            if path.is_empty() {
                continue;
            }
            files.push(PrFile {
                path,
                additions: f.get("additions").and_then(|a| a.as_u64()).unwrap_or(0) as u32,
                deletions: f.get("deletions").and_then(|a| a.as_u64()).unwrap_or(0) as u32,
            });
        }
    }
    Ok(PrDetail {
        number,
        title,
        body,
        author,
        url,
        base,
        head,
        files,
    })
}

/// Undo jq `@tsv` escaping (`\t` `\n` `\r` `\\`) back to real characters.
fn tsv_unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn fetch_pr_review_comments(root: &Path, number: u64) -> Result<Vec<PrComment>, String> {
    // REST via gh: pulls/{n}/comments
    let n = number.to_string();
    let out = Command::new("gh")
        .args([
            "api",
            &format!("repos/{{owner}}/{{repo}}/pulls/{n}/comments"),
            "--paginate",
            "-q",
            ".[] | [.path, (.line // .original_line // 0|tostring), .user.login, .body, .html_url] | @tsv",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        // Fallback without jq filter
        return fetch_pr_comments_json(root, number);
    }
    let mut comments = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.splitn(5, '\t').collect();
        if parts.len() < 4 {
            continue;
        }
        let line_n: u32 = parts[1].parse().unwrap_or(0);
        comments.push(PrComment {
            path: tsv_unescape(parts[0]),
            line: if line_n > 0 { Some(line_n) } else { None },
            author: tsv_unescape(parts[2]),
            body: tsv_unescape(parts[3]),
            url: tsv_unescape(parts.get(4).unwrap_or(&"")),
        });
    }
    Ok(comments)
}

fn fetch_pr_comments_json(root: &Path, number: u64) -> Result<Vec<PrComment>, String> {
    let n = number.to_string();
    let out = Command::new("gh")
        .args([
            "api",
            &format!("repos/{{owner}}/{{repo}}/pulls/{n}/comments"),
            "--paginate",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap_or(serde_json::json!([]));
    let arr = v.as_array().cloned().unwrap_or_default();
    let mut comments = Vec::new();
    for c in arr {
        let path = c
            .get("path")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        let line = c
            .get("line")
            .or_else(|| c.get("original_line"))
            .and_then(|l| l.as_u64())
            .map(|l| l as u32);
        let author = c
            .get("user")
            .and_then(|u| u.get("login"))
            .and_then(|s| s.as_str())
            .unwrap_or("?")
            .to_string();
        let body = c
            .get("body")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string();
        let url = c
            .get("html_url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        if !path.is_empty() {
            comments.push(PrComment {
                path,
                line,
                author,
                body,
                url,
            });
        }
    }
    Ok(comments)
}

fn fetch_pr_file_diff(root: &Path, number: u64, path: &str) -> Result<Vec<String>, String> {
    let n = number.to_string();
    let out = Command::new("gh")
        .args(["pr", "diff", &n, "--", path])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        // full diff fallback — filter client-side
        let out2 = Command::new("gh")
            .args(["pr", "diff", &n])
            .current_dir(root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| e.to_string())?;
        if !out2.status.success() {
            return Err("gh pr diff failed".into());
        }
        return Ok(filter_diff_for_path(
            &String::from_utf8_lossy(&out2.stdout),
            path,
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect())
}

fn filter_diff_for_path(full: &str, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut keep = false;
    let a_tok = format!("a/{path}");
    let b_tok = format!("b/{path}");
    for line in full.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // Exact token match — `a.rs` must not match `xa.rs`.
            keep = rest.split_whitespace().any(|t| t == a_tok || t == b_tok);
        }
        if keep {
            out.push(line.to_string());
        }
    }
    if out.is_empty() {
        out.push(format!("(no diff hunks for {path})"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_pr() {
        let j = r#"{
            "number": 12,
            "title": "Add feature",
            "body": "hello",
            "author": {"login": "alice"},
            "url": "https://example.com",
            "baseRefName": "main",
            "headRefName": "feat",
            "files": [{"path": "src/a.rs", "additions": 3, "deletions": 1}]
        }"#;
        let p = parse_pr_view_json(j).unwrap();
        assert_eq!(p.number, 12);
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.author, "alice");
    }

    #[test]
    fn diff_filter_matches_exact_tokens_only() {
        let full = "diff --git a/xa.rs b/xa.rs\n+++ b/xa.rs\n+wrong\n\
diff --git a/a.rs b/a.rs\n+++ b/a.rs\n+right\n";
        let out = filter_diff_for_path(full, "a.rs");
        assert!(out.iter().any(|l| l == "+right"));
        assert!(!out.iter().any(|l| l == "+wrong"));
    }

    #[test]
    fn tsv_unescape_roundtrip() {
        assert_eq!(tsv_unescape("plain"), "plain");
        assert_eq!(tsv_unescape(r"a\nb\tc"), "a\nb\tc");
        assert_eq!(tsv_unescape(r"back\\slash"), r"back\slash");
        assert_eq!(tsv_unescape(r"lone\q"), r"lone\q");
    }

    #[test]
    fn comments_match_exact_path_only() {
        let mut s = PrReviewState::default();
        s.files = vec![PrFile {
            path: "a.rs".into(),
            additions: 0,
            deletions: 0,
        }];
        s.comments = vec![
            PrComment {
                path: "a.rs".into(),
                line: Some(1),
                author: "x".into(),
                body: String::new(),
                url: String::new(),
            },
            PrComment {
                path: "lib/a.rs".into(),
                line: Some(2),
                author: "y".into(),
                body: String::new(),
                url: String::new(),
            },
        ];
        let matched = s.comments_for_selected_file();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].path, "a.rs");
    }

    #[test]
    fn move_sel_schedules_debounced_diff() {
        let mut s = PrReviewState::default();
        s.open = true;
        s.root = Some(PathBuf::from("."));
        s.files = vec![
            PrFile {
                path: "a.rs".into(),
                additions: 1,
                deletions: 0,
            },
            PrFile {
                path: "b.rs".into(),
                additions: 2,
                deletions: 0,
            },
        ];
        s.move_sel(1);
        assert_eq!(s.file_sel, 1);
        assert!(s.diff_pending_since.is_some());
        assert_eq!(s.file_diff, vec!["(loading diff…)".to_string()]);
        // Within the debounce window poll() must not fire the fetch yet.
        s.poll();
        assert!(s.diff_pending_since.is_some());
    }
}
