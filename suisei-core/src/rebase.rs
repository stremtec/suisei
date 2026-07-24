//! Interactive rebase planner — edit pick/squash/fix/drop then run `git rebase -i`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::git_ops::{self, CommitSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseAction {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl RebaseAction {
    pub fn label(self) -> &'static str {
        match self {
            RebaseAction::Pick => "pick",
            RebaseAction::Reword => "reword",
            RebaseAction::Edit => "edit",
            RebaseAction::Squash => "squash",
            RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        }
    }

    pub fn short(self) -> char {
        match self {
            RebaseAction::Pick => 'p',
            RebaseAction::Reword => 'r',
            RebaseAction::Edit => 'e',
            RebaseAction::Squash => 's',
            RebaseAction::Fixup => 'f',
            RebaseAction::Drop => 'd',
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            RebaseAction::Pick => RebaseAction::Reword,
            RebaseAction::Reword => RebaseAction::Edit,
            RebaseAction::Edit => RebaseAction::Squash,
            RebaseAction::Squash => RebaseAction::Fixup,
            RebaseAction::Fixup => RebaseAction::Drop,
            RebaseAction::Drop => RebaseAction::Pick,
        }
    }

    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'p' | 'P' => Some(RebaseAction::Pick),
            'r' | 'R' => Some(RebaseAction::Reword),
            'e' | 'E' => Some(RebaseAction::Edit),
            's' | 'S' => Some(RebaseAction::Squash),
            'f' | 'F' => Some(RebaseAction::Fixup),
            'd' | 'D' => Some(RebaseAction::Drop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RebaseEntry {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub action: RebaseAction,
}

#[derive(Debug, Clone)]
pub struct RebaseState {
    pub open: bool,
    pub root: PathBuf,
    /// Oldest first (rebase todo order).
    pub entries: Vec<RebaseEntry>,
    pub selected: usize,
    pub message: String,
    pub last_result: Option<String>,
}

impl Default for RebaseState {
    fn default() -> Self {
        Self {
            open: false,
            root: PathBuf::new(),
            entries: Vec::new(),
            selected: 0,
            message: String::new(),
            last_result: None,
        }
    }
}

impl RebaseState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn close(&mut self) {
        self.open = false;
        self.entries.clear();
        self.message.clear();
    }

    /// Open planner for the last `count` commits (newest first in log → reverse for todo).
    pub fn open_for(&mut self, root: &Path, count: usize) -> Result<(), String> {
        let n = count.clamp(2, 50);
        let commits = git_ops::list_commits(root, n, false)?;
        if commits.len() < 2 {
            return Err("Need at least 2 commits to rebase".into());
        }
        // list_commits is newest-first; rebase todo wants oldest-first
        let mut entries: Vec<RebaseEntry> = commits
            .into_iter()
            .take(n)
            .map(|c: CommitSummary| RebaseEntry {
                hash: c.hash,
                short: c.short,
                subject: c.subject,
                action: RebaseAction::Pick,
            })
            .collect();
        entries.reverse();
        self.root = root.to_path_buf();
        self.entries = entries;
        self.selected = 0;
        self.open = true;
        self.last_result = None;
        self.message = format!(
            "Interactive rebase · {} commits · Tab cycle · Enter run · Esc cancel",
            self.entries.len()
        );
        Ok(())
    }

    pub fn move_sel(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let n = self.entries.len() as isize;
        let cur = self.selected as isize + delta;
        self.selected = cur.rem_euclid(n) as usize;
    }

    pub fn cycle_action(&mut self) {
        if let Some(e) = self.entries.get_mut(self.selected) {
            e.action = e.action.cycle();
        }
    }

    pub fn set_action(&mut self, action: RebaseAction) {
        if let Some(e) = self.entries.get_mut(self.selected) {
            e.action = action;
        }
    }

    /// Move selected entry up/down in the todo list.
    pub fn move_entry(&mut self, delta: isize) {
        if self.entries.len() < 2 {
            return;
        }
        let i = self.selected;
        let j = (i as isize + delta).clamp(0, (self.entries.len() - 1) as isize) as usize;
        if i != j {
            self.entries.swap(i, j);
            self.selected = j;
        }
    }

    fn todo_text(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            if e.action == RebaseAction::Drop {
                continue;
            }
            out.push_str(&format!(
                "{} {} {}\n",
                e.action.label(),
                e.hash,
                e.subject
            ));
        }
        out
    }

    /// Execute `git rebase -i` with our sequence via a temporary sequence editor script.
    pub fn run(&mut self) -> Result<String, String> {
        if self.entries.is_empty() {
            return Err("Empty rebase plan".into());
        }
        if self
            .entries
            .iter()
            .all(|e| e.action == RebaseAction::Drop)
        {
            return Err("All commits marked drop — nothing to do".into());
        }

        let todo = self.todo_text();
        if todo.trim().is_empty() {
            return Err("Rebase todo is empty".into());
        }

        let tmp_dir = std::env::temp_dir().join(format!("xei-rebase-{}", std::process::id()));
        std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
        let todo_path = tmp_dir.join("git-rebase-todo");
        let script_path = tmp_dir.join("seq-editor.sh");
        std::fs::write(&todo_path, &todo).map_err(|e| e.to_string())?;

        // Sequence editor: copy our todo over the path git passes as $1
        let script = format!(
            "#!/bin/sh\ncp '{}' \"$1\"\n",
            todo_path.display()
        );
        std::fs::write(&script_path, script).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path)
                .map_err(|e| e.to_string())?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).map_err(|e| e.to_string())?;
        }

        // Rebase onto parent of oldest kept commit
        let base = self
            .entries
            .iter()
            .find(|e| e.action != RebaseAction::Drop)
            .map(|e| e.hash.clone())
            .ok_or_else(|| "No commits to rebase".to_string())?;

        // `git rebase -i <base>^` replays commits after base's parent
        let onto = format!("{base}^");
        let output = Command::new("git")
            .args(["rebase", "-i", &onto])
            .current_dir(&self.root)
            .env("GIT_SEQUENCE_EDITOR", &script_path)
            .env("GIT_EDITOR", "true") // skip reword/edit body prompts if any
            .output()
            .map_err(|e| format!("git rebase failed to start: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}").trim().to_string();

        // Cleanup temp (best-effort)
        let _ = std::fs::remove_dir_all(&tmp_dir);

        if output.status.success() {
            let msg = if combined.is_empty() {
                format!("✓ Rebase done ({} commits)", self.entries.len())
            } else {
                combined.lines().next().unwrap_or("✓ Rebase done").to_string()
            };
            self.last_result = Some(msg.clone());
            self.open = false;
            Ok(msg)
        } else {
            // Conflict or other failure — leave open so user can abort
            let msg = if combined.is_empty() {
                "Rebase failed — try :rebase-abort".into()
            } else {
                format!("Rebase issue: {}", combined.lines().next().unwrap_or("failed"))
            };
            self.last_result = Some(msg.clone());
            Err(msg)
        }
    }
}

pub fn rebase_abort(root: &Path) -> Result<String, String> {
    git_ops::run_git(root, &["rebase", "--abort"]).map(|s| {
        let t = s.trim();
        if t.is_empty() {
            "Rebase aborted".into()
        } else {
            t.to_string()
        }
    })
}

pub fn rebase_continue(root: &Path) -> Result<String, String> {
    git_ops::run_git(root, &["rebase", "--continue"]).map(|s| {
        let t = s.trim();
        if t.is_empty() {
            "Rebase continued".into()
        } else {
            t.to_string()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_actions() {
        let mut a = RebaseAction::Pick;
        for _ in 0..6 {
            a = a.cycle();
        }
        assert_eq!(a, RebaseAction::Pick);
    }

    #[test]
    fn todo_skips_drop() {
        let mut s = RebaseState::new();
        s.entries = vec![
            RebaseEntry {
                hash: "aaa".into(),
                short: "aaa".into(),
                subject: "one".into(),
                action: RebaseAction::Pick,
            },
            RebaseEntry {
                hash: "bbb".into(),
                short: "bbb".into(),
                subject: "two".into(),
                action: RebaseAction::Drop,
            },
            RebaseEntry {
                hash: "ccc".into(),
                short: "ccc".into(),
                subject: "three".into(),
                action: RebaseAction::Squash,
            },
        ];
        let t = s.todo_text();
        assert!(t.contains("pick aaa"));
        assert!(!t.contains("bbb"));
        assert!(t.contains("squash ccc"));
    }
}
