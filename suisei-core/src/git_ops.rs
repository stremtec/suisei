//! Shared git CLI helpers for SCM + Git workbench.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub current: bool,
    pub remote: bool,
    /// ahead/behind vs upstream when known
    pub upstream: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// Full raw line from `git diff` (includes leading `+`/`-`/` ` when applicable).
    pub text: String,
    /// Old-file line number (left gutter). `None` for headers / pure adds.
    pub old_no: Option<u32>,
    /// New-file line number (right gutter). `None` for headers / pure deletes.
    pub new_no: Option<u32>,
}

impl DiffLine {
    pub fn new(kind: DiffLineKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            old_no: None,
            new_no: None,
        }
    }

    /// Content without the leading diff marker (`+`/`-`/` `).
    pub fn content(&self) -> &str {
        match self.kind {
            DiffLineKind::Add | DiffLineKind::Del | DiffLineKind::Context => {
                self.text.get(1..).unwrap_or(self.text.as_str())
            }
            _ => self.text.as_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Header,
    Hunk,
    Add,
    Del,
    Context,
    Meta,
}

pub fn find_git_root(hint: Option<&Path>) -> Option<PathBuf> {
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

pub fn run_git(root: &Path, args: &[&str]) -> Result<String, String> {
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

/// Best-effort; stderr on failure still returned as Err.
pub fn run_git_ok(root: &Path, args: &[&str]) -> Result<String, String> {
    run_git(root, args)
}

pub fn list_branches(root: &Path) -> Result<Vec<BranchInfo>, String> {
    // Local branches
    let local = run_git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short)%00%(upstream:short)%00%(HEAD)",
            "refs/heads",
        ],
    )?;
    let mut out = Vec::new();
    for line in local.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\0').collect();
        let name = parts.first().copied().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let upstream = parts
            .get(1)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let current = parts.get(2).map(|s| *s == "*").unwrap_or(false);
        out.push(BranchInfo {
            name,
            current,
            remote: false,
            upstream,
        });
    }

    // Remote branches (no current)
    if let Ok(remote) = run_git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/remotes",
        ],
    ) {
        for line in remote.lines() {
            let name = line.trim();
            if name.is_empty() || name.ends_with("/HEAD") {
                continue;
            }
            // skip if already have local with same short name? keep remotes as remote/foo
            if out.iter().any(|b| b.name == name) {
                continue;
            }
            out.push(BranchInfo {
                name: name.to_string(),
                current: false,
                remote: true,
                upstream: None,
            });
        }
    }

    // Current first, then local, then remote
    out.sort_by(|a, b| {
        b.current
            .cmp(&a.current)
            .then_with(|| a.remote.cmp(&b.remote))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

pub fn checkout_branch(root: &Path, name: &str) -> Result<String, String> {
    // Remote-only: checkout -b local --track remote
    if name.contains('/') && name.starts_with("origin/") {
        let local = name.trim_start_matches("origin/");
        // try create tracking branch
        match run_git(root, &["checkout", "-B", local, "--track", name]) {
            Ok(o) => return Ok(o.lines().next().unwrap_or("Checked out").to_string()),
            Err(_) => {
                // already exists
                return run_git(root, &["checkout", local])
                    .map(|o| o.lines().next().unwrap_or("Checked out").to_string());
            }
        }
    }
    run_git(root, &["checkout", name])
        .map(|o| o.lines().next().unwrap_or("Checked out").to_string())
}

pub fn create_branch(root: &Path, name: &str) -> Result<String, String> {
    run_git(root, &["checkout", "-b", name])
        .map(|o| o.lines().next().unwrap_or("Created branch").to_string())
}

pub fn delete_branch(root: &Path, name: &str, force: bool) -> Result<String, String> {
    let flag = if force { "-D" } else { "-d" };
    run_git(root, &["branch", flag, name])
        .map(|_| format!("Deleted branch {name}"))
}

pub fn stage_all(root: &Path) -> Result<String, String> {
    run_git(root, &["add", "-A"]).map(|_| "Staged all changes".into())
}

pub fn unstage_all(root: &Path) -> Result<String, String> {
    run_git(root, &["restore", "--staged", "."]).map(|_| "Unstaged all".into())
}

pub fn discard_file(root: &Path, path: &str) -> Result<String, String> {
    // Untracked: remove file; tracked: restore from HEAD
    let status = run_git(root, &["status", "--porcelain", "--", path])?;
    let line = status.lines().next().unwrap_or("");
    if line.starts_with("??") {
        let p = root.join(path);
        if p.is_file() {
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
        } else if p.is_dir() {
            std::fs::remove_dir_all(&p).map_err(|e| e.to_string())?;
        }
        return Ok(format!("Removed untracked {path}"));
    }
    // unstage then restore worktree
    let _ = run_git(root, &["restore", "--staged", "--", path]);
    run_git(root, &["restore", "--", path]).map(|_| format!("Discarded {path}"))
}

pub fn cherry_pick(root: &Path, hash: &str) -> Result<String, String> {
    run_git(root, &["cherry-pick", hash]).map(|o| {
        o.lines()
            .next()
            .unwrap_or("Cherry-picked")
            .to_string()
    })
}

pub fn revert_commit(root: &Path, hash: &str) -> Result<String, String> {
    run_git(root, &["revert", "--no-edit", hash]).map(|o| {
        o.lines()
            .next()
            .unwrap_or("Reverted")
            .to_string()
    })
}

pub fn pull_rebase(root: &Path) -> Result<String, String> {
    run_git(root, &["pull", "--rebase"]).map(|o| {
        let t = o.trim();
        if t.is_empty() {
            "Pulled (rebase)".into()
        } else {
            t.lines().next().unwrap_or("Pulled (rebase)").to_string()
        }
    })
}

pub fn stash_list(root: &Path) -> Result<Vec<String>, String> {
    let out = run_git(root, &["stash", "list"])?;
    Ok(out
        .lines()
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .collect())
}

pub fn remotes(root: &Path) -> Result<Vec<(String, String)>, String> {
    let out = run_git(root, &["remote", "-v"])?;
    let mut v = Vec::new();
    for line in out.lines() {
        let mut parts = line.split_whitespace();
        let name = parts.next().unwrap_or("").to_string();
        let url = parts.next().unwrap_or("").to_string();
        let kind = parts.next().unwrap_or("");
        if kind.contains("fetch") && !name.is_empty() {
            v.push((name, url));
        }
    }
    Ok(v)
}

pub fn log_file(root: &Path, path: &str, limit: usize) -> Result<Vec<CommitSummary>, String> {
    let n = limit.clamp(5, 100).to_string();
    let out = run_git(
        root,
        &[
            "log",
            "-n",
            &n,
            "--pretty=format:%H%x00%h%x00%s%x00%an%x00%ae%x00%ar%x00%P",
            "--",
            path,
        ],
    )?;
    let mut commits = Vec::new();
    for line in out.lines() {
        if line.is_empty() {
            continue;
        }
        let p: Vec<&str> = line.split('\0').collect();
        if p.len() < 6 {
            continue;
        }
        commits.push(CommitSummary {
            hash: p[0].to_string(),
            short: p[1].to_string(),
            subject: p[2].to_string(),
            author: p[3].to_string(),
            email: p[4].to_string(),
            when: p[5].to_string(),
            parents: p
                .get(6)
                .unwrap_or(&"")
                .split_whitespace()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect(),
        });
    }
    Ok(commits)
}

pub fn fetch(root: &Path) -> Result<String, String> {
    run_git(root, &["fetch", "--all", "--prune"])
        .map(|_| "Fetched".into())
}

pub fn pull(root: &Path) -> Result<String, String> {
    run_git(root, &["pull", "--ff-only"])
        .or_else(|_| run_git(root, &["pull"]))
        .map(|o| {
            let t = o.trim();
            if t.is_empty() {
                "Pulled".into()
            } else {
                t.lines().next().unwrap_or("Pulled").to_string()
            }
        })
}

pub fn push(root: &Path) -> Result<String, String> {
    run_git(root, &["push"]).map(|o| {
        let t = o.trim();
        if t.is_empty() {
            // push often writes to stderr even on success — try -u
            "Pushed".into()
        } else {
            t.lines().next().unwrap_or("Pushed").to_string()
        }
    }).or_else(|e| {
        // first push may need -u
        if e.contains("no upstream") || e.contains("has no upstream") {
            run_git(root, &["push", "-u", "origin", "HEAD"]).map(|_| "Pushed (set upstream)".into())
        } else {
            // git push writes progress to stderr; re-run capturing both
            let output = Command::new("git")
                .args(["push"])
                .current_dir(root)
                .output()
                .map_err(|err| format!("git push: {err}"))?;
            if output.status.success() {
                Ok("Pushed".into())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(err.lines().next().unwrap_or("push failed").to_string())
            }
        }
    })
}

pub fn file_diff(root: &Path, path: &str, staged: bool) -> Result<Vec<DiffLine>, String> {
    let args: Vec<&str> = if staged {
        vec!["diff", "--no-color", "--cached", "--", path]
    } else {
        vec!["diff", "--no-color", "HEAD", "--", path]
    };
    // Untracked: no HEAD diff — show as all adds via /dev/null
    let out = match run_git(root, &args) {
        Ok(o) if !o.trim().is_empty() => o,
        _ => {
            // try unstaged only
            let o2 = run_git(root, &["diff", "--no-color", "--", path]).unwrap_or_default();
            if o2.trim().is_empty() {
                // untracked file
                let full = root.join(path);
                if full.is_file() {
                    let content = std::fs::read_to_string(&full).unwrap_or_default();
                    let mut lines = vec![DiffLine::new(
                        DiffLineKind::Header,
                        format!("diff -- untracked a/{path} b/{path}"),
                    )];
                    let mut n = 1u32;
                    for l in content.lines() {
                        lines.push(DiffLine {
                            kind: DiffLineKind::Add,
                            text: format!("+{l}"),
                            old_no: None,
                            new_no: Some(n),
                        });
                        n += 1;
                    }
                    if lines.len() == 1 {
                        lines.push(DiffLine::new(DiffLineKind::Meta, "(empty file)"));
                    }
                    return Ok(lines);
                }
                return Ok(vec![DiffLine::new(DiffLineKind::Meta, "No diff")]);
            }
            o2
        }
    };
    Ok(parse_diff(&out))
}

/// Parse unified diff text and attach old/new line numbers from hunk headers.
pub fn parse_diff(text: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let mut old_ln: u32 = 0;
    let mut new_ln: u32 = 0;
    for line in text.lines() {
        if line.starts_with("diff ") || line.starts_with("index ") {
            out.push(DiffLine::new(DiffLineKind::Header, line));
        } else if line.starts_with("@@") {
            // @@ -old_start,old_count +new_start,new_count @@
            if let Some((o, n)) = parse_hunk_starts(line) {
                old_ln = o;
                new_ln = n;
            }
            out.push(DiffLine::new(DiffLineKind::Hunk, line));
        } else if line.starts_with('+') && !line.starts_with("+++") {
            out.push(DiffLine {
                kind: DiffLineKind::Add,
                text: line.to_string(),
                old_no: None,
                new_no: Some(new_ln),
            });
            new_ln = new_ln.saturating_add(1);
        } else if line.starts_with('-') && !line.starts_with("---") {
            out.push(DiffLine {
                kind: DiffLineKind::Del,
                text: line.to_string(),
                old_no: Some(old_ln),
                new_no: None,
            });
            old_ln = old_ln.saturating_add(1);
        } else if line.starts_with("+++") || line.starts_with("---") {
            out.push(DiffLine::new(DiffLineKind::Meta, line));
        } else if line.starts_with(' ') || (line.is_empty() && (old_ln > 0 || new_ln > 0)) {
            // Context line (leading space) or blank context after a hunk.
            let old_no = if old_ln > 0 { Some(old_ln) } else { None };
            let new_no = if new_ln > 0 { Some(new_ln) } else { None };
            if old_ln > 0 {
                old_ln = old_ln.saturating_add(1);
            }
            if new_ln > 0 {
                new_ln = new_ln.saturating_add(1);
            }
            out.push(DiffLine {
                kind: DiffLineKind::Context,
                text: if line.is_empty() {
                    " ".into()
                } else {
                    line.to_string()
                },
                old_no,
                new_no,
            });
        } else {
            out.push(DiffLine::new(DiffLineKind::Meta, line));
        }
    }
    if out.is_empty() {
        out.push(DiffLine::new(DiffLineKind::Meta, "No changes"));
    }
    out
}

/// Extract old/new start line numbers from a `@@ -a,b +c,d @@` header.
fn parse_hunk_starts(hunk: &str) -> Option<(u32, u32)> {
    // Find "-N" and "+M"
    let mut old = None;
    let mut new = None;
    for part in hunk.split_whitespace() {
        if let Some(rest) = part.strip_prefix('-') {
            let num = rest.split(',').next()?.parse::<u32>().ok()?;
            old = Some(num.max(1));
        } else if let Some(rest) = part.strip_prefix('+') {
            if rest.starts_with('+') {
                continue; // +++
            }
            let num = rest.split(',').next()?.parse::<u32>().ok()?;
            new = Some(num.max(1));
        }
    }
    Some((old?, new?))
}

pub fn stash_push(root: &Path) -> Result<String, String> {
    run_git(root, &["stash", "push", "-u"]).map(|o| {
        o.lines()
            .next()
            .unwrap_or("Stashed")
            .to_string()
    })
}

pub fn stash_pop(root: &Path) -> Result<String, String> {
    run_git(root, &["stash", "pop"]).map(|o| {
        o.lines()
            .next()
            .unwrap_or("Stash applied")
            .to_string()
    })
}

pub fn stash_apply(root: &Path, index: usize) -> Result<String, String> {
    let refname = format!("stash@{{{index}}}");
    run_git(root, &["stash", "apply", &refname]).map(|o| {
        o.lines()
            .next()
            .unwrap_or("Stash applied")
            .to_string()
    })
}

pub fn stash_drop(root: &Path, index: usize) -> Result<String, String> {
    let refname = format!("stash@{{{index}}}");
    run_git(root, &["stash", "drop", &refname]).map(|o| {
        o.lines()
            .next()
            .unwrap_or("Stash dropped")
            .to_string()
    })
}

pub fn stash_show(root: &Path, index: usize) -> Result<String, String> {
    let refname = format!("stash@{{{index}}}");
    run_git(root, &["stash", "show", "-p", "--stat", &refname])
}

pub fn current_branch(root: &Path) -> String {
    run_git(root, &["branch", "--show-current"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

// ── Commit history (GitHub-style) ───────────────────────

#[derive(Debug, Clone)]
pub struct CommitSummary {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    pub email: String,
    pub when: String,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CommitFileChange {
    pub path: String,
    /// A/M/D/R/C/T/?
    pub status: char,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone)]
pub struct CommitDetail {
    pub hash: String,
    pub short: String,
    pub subject: String,
    /// Full body (may be empty)
    pub body: String,
    pub author: String,
    pub email: String,
    pub date: String,
    pub files: Vec<CommitFileChange>,
    pub insertions: u32,
    pub deletions: u32,
}

/// Newest-first commit list (`git log --all` when `all` is true).
pub fn list_commits(root: &Path, limit: usize, all: bool) -> Result<Vec<CommitSummary>, String> {
    let n = limit.clamp(20, 2000).to_string();
    let mut args = vec![
        "log",
        "-n",
        n.as_str(),
        "--pretty=format:%H%x00%h%x00%s%x00%an%x00%ae%x00%ar%x00%P",
    ];
    if all {
        args.insert(1, "--all");
    }
    let out = run_git(root, &args)?;
    let mut commits = Vec::new();
    for line in out.lines() {
        if line.is_empty() {
            continue;
        }
        let p: Vec<&str> = line.split('\0').collect();
        if p.len() < 6 {
            continue;
        }
        let parents = p
            .get(6)
            .unwrap_or(&"")
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        commits.push(CommitSummary {
            hash: p[0].to_string(),
            short: p[1].to_string(),
            subject: p[2].to_string(),
            author: p[3].to_string(),
            email: p[4].to_string(),
            when: p[5].to_string(),
            parents,
        });
    }
    Ok(commits)
}

/// Message + file list + numstat for one commit (GitHub commit page).
pub fn commit_detail(root: &Path, hash: &str) -> Result<CommitDetail, String> {
    // Metadata: subject, body, author, date
    let meta = run_git(
        root,
        &[
            "show",
            "-s",
            "--format=%H%x00%h%x00%s%x00%b%x00%an%x00%ae%x00%aI",
            hash,
        ],
    )?;
    // git show -s with body can be multi-line; use null-separated carefully.
    // Body may contain newlines but not NULs. Format uses %x00 between fields;
    // body is field 3 and can have newlines until next field... actually
    // pretty format with %b then %x00 is tricky with newlines.
    // Safer: separate calls.
    let head = run_git(
        root,
        &[
            "show",
            "-s",
            "--format=%H%x00%h%x00%s%x00%an%x00%ae%x00%aI%x00%P",
            hash,
        ],
    )?;
    let line = head.lines().next().unwrap_or("");
    let p: Vec<&str> = line.split('\0').collect();
    let full = p.first().unwrap_or(&hash).to_string();
    let short = p.get(1).unwrap_or(&"").to_string();
    let subject = p.get(2).unwrap_or(&"").to_string();
    let author = p.get(3).unwrap_or(&"").to_string();
    let email = p.get(4).unwrap_or(&"").to_string();
    let date = p.get(5).unwrap_or(&"").to_string();

    let body = run_git(root, &["log", "-1", "--format=%b", hash])
        .map(|s| s.trim_end().to_string())
        .unwrap_or_default();
    let _ = meta;

    // name-status
    let ns = run_git(root, &["show", "--name-status", "--format=", hash]).unwrap_or_default();
    // numstat
    let num = run_git(root, &["show", "--numstat", "--format=", hash]).unwrap_or_default();

    let mut stats: std::collections::HashMap<String, (u32, u32)> = std::collections::HashMap::new();
    for line in num.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let ins: u32 = parts[0].parse().unwrap_or(0);
        let del: u32 = parts[1].parse().unwrap_or(0);
        let path = parts[2].to_string();
        // renames: old => new
        let path = if let Some((_, n)) = path.split_once(" => ") {
            n.to_string()
        } else {
            path
        };
        stats.insert(path, (ins, del));
    }

    let mut files = Vec::new();
    let mut total_ins = 0u32;
    let mut total_del = 0u32;
    for line in ns.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let st = parts.next().unwrap_or("M");
        let path_raw = parts.next().unwrap_or("").trim();
        if path_raw.is_empty() {
            continue;
        }
        let status = st.chars().next().unwrap_or('M');
        let path = if let Some((_, n)) = path_raw.split_once(" => ") {
            n.to_string()
        } else if let Some((_, n)) = path_raw.split_once('\t') {
            // R100\told\tnew
            n.to_string()
        } else {
            path_raw.to_string()
        };
        // name-status for rename: R100\told\tnew
        let path = {
            let bits: Vec<&str> = line.split('\t').collect();
            if bits.len() >= 3 {
                bits[2].to_string()
            } else if bits.len() == 2 {
                bits[1].to_string()
            } else {
                path
            }
        };
        let (ins, del) = stats.get(&path).copied().unwrap_or((0, 0));
        total_ins += ins;
        total_del += del;
        files.push(CommitFileChange {
            path,
            status,
            insertions: ins,
            deletions: del,
        });
    }

    Ok(CommitDetail {
        hash: full,
        short,
        subject,
        body,
        author,
        email,
        date,
        files,
        insertions: total_ins,
        deletions: total_del,
    })
}

/// Diff of one file in a commit vs its first parent.
pub fn commit_file_diff(root: &Path, hash: &str, path: &str) -> Result<Vec<DiffLine>, String> {
    let out = run_git(
        root,
        &["show", "--no-color", "--format=", hash, "--", path],
    )?;
    if out.trim().is_empty() {
        return Ok(vec![DiffLine::new(
            DiffLineKind::Meta,
            "No diff for this file",
        )]);
    }
    Ok(parse_diff(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_diff_kinds() {
        let d = parse_diff(
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n context\n",
        );
        assert!(d.iter().any(|l| l.kind == DiffLineKind::Add));
        assert!(d.iter().any(|l| l.kind == DiffLineKind::Del));
        assert!(d.iter().any(|l| l.kind == DiffLineKind::Hunk));
    }

    #[test]
    fn parse_diff_line_numbers() {
        let d = parse_diff(
            "@@ -10,3 +20,4 @@ fn foo\n context a\n-removed\n+added1\n+added2\n context b\n",
        );
        let del = d.iter().find(|l| l.kind == DiffLineKind::Del).unwrap();
        assert_eq!(del.old_no, Some(11));
        assert_eq!(del.new_no, None);
        let adds: Vec<_> = d.iter().filter(|l| l.kind == DiffLineKind::Add).collect();
        assert_eq!(adds[0].new_no, Some(21));
        assert_eq!(adds[1].new_no, Some(22));
        let ctx: Vec<_> = d
            .iter()
            .filter(|l| l.kind == DiffLineKind::Context)
            .collect();
        assert_eq!(ctx[0].old_no, Some(10));
        assert_eq!(ctx[0].new_no, Some(20));
    }
}
