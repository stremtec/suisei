//! Full **Git workbench** (Ctrl+Shift+G) — mini GitHub surface.
//!
//! - **Status** — working tree + sync + stash  
//! - **Branches** — list / checkout  
//! - **History** — commits as **list** or **graph** (`v` toggles)  
//! - **Commit** — message, files, stats  
//! - **Diff** — file unified diff  
//! - **PRs** — `gh pr list` / checkout (when authed)  
//! - **Auth** — built-in `gh auth` status / login / logout  

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

use crate::gh::{
    self, filter_issues, filter_prs, AuthLoginSession, GhAuthInfo, GhAuthState, IssueListState,
    IssueSummary, PrListState, PrSummary,
};
use crate::git_graph::{self, GraphRow};
use crate::git_ops::{self, BranchInfo, CommitDetail, CommitSummary, DiffLine};
use crate::scm::{parse_porcelain_entries, ScmEntry};

/// Background load target (network / gh / slow git).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitLoadTarget {
    Branches,
    PullRequests,
    Issues,
    Auth,
}

/// Right-click context menu on a commit (Log pane).
#[derive(Debug, Clone)]
pub struct GitContextMenu {
    pub x: u16,
    pub y: u16,
    pub sel: usize,
    pub commit_idx: usize,
    pub items: Vec<GitCtxItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitCtxItem {
    ShowFiles,
    CherryPick,
    Revert,
    CopyHash,
    BrowseOnGitHub,
}

impl GitCtxItem {
    pub fn label(self) -> &'static str {
        match self {
            GitCtxItem::ShowFiles => "Show files",
            GitCtxItem::CherryPick => "Cherry-pick",
            GitCtxItem::Revert => "Revert",
            GitCtxItem::CopyHash => "Copy commit hash",
            GitCtxItem::BrowseOnGitHub => "Browse on GitHub",
        }
    }
    pub fn key_hint(self) -> &'static str {
        match self {
            GitCtxItem::ShowFiles => "Enter",
            GitCtxItem::CherryPick => "C",
            GitCtxItem::Revert => "V",
            GitCtxItem::CopyHash => "y",
            GitCtxItem::BrowseOnGitHub => "o",
        }
    }
}

enum GitLoadResult {
    Branches(Result<Vec<BranchInfo>, String>),
    Prs(Result<Vec<PrSummary>, String>),
    Issues(Result<Vec<IssueSummary>, String>),
    Auth { info: GhAuthInfo, gh_ok: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitTab {
    Status,
    Branches,
    History,
    Commit,
    Diff,
    PullRequests,
    Issues,
    Auth,
    /// Stash list · apply / pop / drop
    Stash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryView {
    List,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitFocus {
    Tabs,
    List,
    Diff,
}

#[derive(Debug, Clone)]
pub enum DiffOrigin {
    Worktree { path: String, staged: bool },
    Commit { hash: String, path: String },
}

/// Background-able network git operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteAction {
    Fetch,
    Pull,
    PullRebase,
    Push,
}

#[derive(Debug)]
pub struct GitWorkbench {
    pub open: bool,
    pub from_scm: bool,
    pub tab: GitTab,
    pub focus: GitFocus,
    pub root: Option<PathBuf>,
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub staged: Vec<ScmEntry>,
    pub changes: Vec<ScmEntry>,
    pub selected: usize,
    pub branches: Vec<BranchInfo>,
    pub branch_sel: usize,
    // History
    pub commits: Vec<CommitSummary>,
    pub history_graph: Vec<GraphRow>,
    pub history_view: HistoryView,
    pub history_sel: usize,
    pub history_limit: usize,
    pub commit_detail: Option<CommitDetail>,
    pub commit_file_sel: usize,
    // Diff
    pub diff_path: Option<String>,
    pub diff_staged: bool,
    pub diff_lines: Vec<DiffLine>,
    pub diff_scroll: usize,
    pub diff_origin: Option<DiffOrigin>,
    // GitHub
    pub auth: GhAuthInfo,
    pub auth_action_sel: usize,
    /// Non-blocking browser login (Auth tab).
    pub auth_login: Option<AuthLoginSession>,
    pub prs: Vec<PrSummary>,
    pub pr_sel: usize,
    /// open / closed / merged / all
    pub pr_state: PrListState,
    /// Client filter query (empty = show all in current state)
    pub pr_filter: String,
    /// When true, typing goes into `pr_filter`
    pub pr_filter_mode: bool,
    /// Indices into `prs` after filter
    pub pr_filtered: Vec<usize>,
    pub issues: Vec<IssueSummary>,
    pub issue_sel: usize,
    pub issue_state: IssueListState,
    pub issue_filter: String,
    pub issue_filter_mode: bool,
    pub issue_filtered: Vec<usize>,
    pub issues_loaded: bool,
    pub message: Option<String>,
    pub error: Option<String>,
    pub gh_available: bool,
    /// Lazy-load flags — open path only fetches Status data (fast).
    pub branches_loaded: bool,
    pub history_loaded: bool,
    pub prs_loaded: bool,
    pub auth_loaded: bool,
    /// Inline input: create branch / rename / etc.
    pub input_mode: Option<InputMode>,
    pub input_buf: String,
    pub stashes: Vec<String>,
    pub stash_sel: usize,
    pub remotes: Vec<(String, String)>,
    /// Commit message draft (left pane, JetBrains-style).
    pub commit_buf: String,
    /// True while typing into `commit_buf`.
    pub commit_editing: bool,
    /// Which column has keyboard focus in the docked workbench.
    pub pane: GitPane,
    /// Background load in flight (Branches / PRs / Issues / Auth).
    pub loading: Option<GitLoadTarget>,
    loading_started: Option<Instant>,
    load_rx: Option<Receiver<GitLoadResult>>,
    /// Remote git op (push/pull/fetch) running on a background thread.
    action_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    pub action_label: Option<&'static str>,
    /// Finished action message for the app frontend to surface.
    app_msg: Option<String>,
    /// Right-click commit menu.
    pub ctx_menu: Option<GitContextMenu>,
}

/// JetBrains-style docked columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GitPane {
    /// Changes / staged list
    #[default]
    Changes,
    /// Commit log / graph
    Log,
    /// Selected commit files / detail
    Files,
}

/// Active text-input overlay inside the workbench.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    NewBranch,
    /// Confirm discard of selected file path
    ConfirmDiscard { path: String },
}

/// First History load size (was 120 + full graph — made open feel frozen).
pub const HISTORY_DEFAULT: usize = 60;
pub const HISTORY_MAX: usize = 1000;

impl Default for GitWorkbench {
    fn default() -> Self {
        Self {
            open: false,
            from_scm: false,
            tab: GitTab::Status,
            focus: GitFocus::List,
            root: None,
            branch: String::new(),
            ahead: 0,
            behind: 0,
            staged: Vec::new(),
            changes: Vec::new(),
            selected: 0,
            branches: Vec::new(),
            branch_sel: 0,
            commits: Vec::new(),
            history_graph: Vec::new(),
            history_view: HistoryView::List,
            history_sel: 0,
            history_limit: HISTORY_DEFAULT,
            commit_detail: None,
            commit_file_sel: 0,
            diff_path: None,
            diff_staged: false,
            diff_lines: Vec::new(),
            diff_scroll: 0,
            diff_origin: None,
            auth: GhAuthInfo::default(),
            auth_action_sel: 0,
            auth_login: None,
            prs: Vec::new(),
            pr_sel: 0,
            pr_state: PrListState::Open,
            pr_filter: String::new(),
            pr_filter_mode: false,
            pr_filtered: Vec::new(),
            issues: Vec::new(),
            issue_sel: 0,
            issue_state: IssueListState::Open,
            issue_filter: String::new(),
            issue_filter_mode: false,
            issue_filtered: Vec::new(),
            issues_loaded: false,
            message: None,
            error: None,
            gh_available: false,
            branches_loaded: false,
            history_loaded: false,
            prs_loaded: false,
            auth_loaded: false,
            input_mode: None,
            input_buf: String::new(),
            stashes: Vec::new(),
            stash_sel: 0,
            remotes: Vec::new(),
            commit_buf: String::new(),
            commit_editing: false,
            pane: GitPane::Changes,
            loading: None,
            loading_started: None,
            load_rx: None,
            action_rx: None,
            action_label: None,
            app_msg: None,
            ctx_menu: None,
        }
    }
}

impl GitWorkbench {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(&self) -> bool {
        self.open
    }

    pub fn close(&mut self) {
        self.open = false;
        self.diff_path = None;
        self.diff_lines.clear();
        self.diff_origin = None;
        self.commit_detail = None;
        self.message = None;
        self.error = None;
        self.ctx_menu = None;
        self.commit_editing = false;
        // Cancel in-flight browser login so we don't leave a zombie gh process
        if let Some(s) = self.auth_login.take() {
            s.cancel();
        }
        // keep auth / lazy caches for next open
    }

    /// Open quickly: only working-tree status (no history/graph/gh network).
    /// Always re-resolves the git root from `hint` / cwd so project switches work.
    pub fn open_at(&mut self, hint: Option<&Path>, from_scm: bool) {
        self.open = true;
        self.from_scm = from_scm;
        self.tab = GitTab::Status;
        self.focus = GitFocus::List;
        self.diff_path = None;
        self.diff_lines.clear();
        self.diff_origin = None;
        self.commit_detail = None;
        // Drop project-scoped caches so a new folder cannot reuse old root/history.
        self.root = None;
        self.commits.clear();
        self.history_graph.clear();
        self.branches.clear();
        self.prs.clear();
        self.staged.clear();
        self.changes.clear();
        self.branch.clear();
        self.branches_loaded = false;
        self.history_loaded = false;
        self.prs_loaded = false;
        self.issues_loaded = false;
        self.pr_filter_mode = false;
        self.issue_filter_mode = false;
        self.input_mode = None;
        self.input_buf.clear();
        self.stashes.clear();
        self.remotes.clear();
        // Fast PATH check only — full `gh auth status` can hit the network.
        self.gh_available = gh::gh_installed();
        self.refresh_status(hint);
    }

    /// Ensure tab-specific data is loaded (lazy). Heavy tabs load in a
    /// background thread so the UI can animate a spinner.
    pub fn ensure_tab_data(&mut self) {
        match self.tab {
            GitTab::Branches if !self.branches_loaded => {
                self.start_load(GitLoadTarget::Branches);
            }
            GitTab::History | GitTab::Commit if !self.history_loaded => {
                if let Some(ref root) = self.root.clone() {
                    self.reload_history(&root);
                    self.history_loaded = true;
                }
            }
            GitTab::PullRequests if !self.prs_loaded => {
                self.start_load(GitLoadTarget::PullRequests);
            }
            GitTab::Issues if !self.issues_loaded => {
                self.start_load(GitLoadTarget::Issues);
            }
            GitTab::Auth if !self.auth_loaded => {
                self.start_load(GitLoadTarget::Auth);
            }
            GitTab::Stash => {
                self.load_stashes_and_remotes();
            }
            _ => {}
        }
    }

    /// Single-line braille tick (toolbar / status bar).
    pub fn spinner_frame(&self) -> &'static str {
        const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let i = self
            .loading_started
            .map(|t| (t.elapsed().as_millis() / 80) as usize)
            .unwrap_or(0);
        FRAMES[i % FRAMES.len()]
    }

    pub fn loading_label(&self) -> Option<&'static str> {
        match self.loading? {
            GitLoadTarget::Branches => Some("syncing branches"),
            GitLoadTarget::PullRequests => Some("syncing pull requests"),
            GitLoadTarget::Issues => Some("syncing issues"),
            GitLoadTarget::Auth => Some("refreshing GitHub account"),
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some() || self.action_rx.is_some()
    }

    /// Animation tick (120ms steps) for shade-block phase.
    pub fn loading_tick(&self) -> usize {
        self.loading_started
            .map(|t| (t.elapsed().as_millis() / 120) as usize)
            .unwrap_or(0)
    }

    /// Rotating tips under the loading logo (changes every ~2.4s).
    pub fn loading_tip(&self) -> &'static str {
        const TIPS: &[&str] = &[
            "tip · Space stages a file · c commits with message",
            "tip · Ctrl+Shift+T opens a terminal window",
            "tip · Tab cycles Changes · Log · Files panes",
            "tip · Right-click a commit for cherry-pick / copy hash",
            "tip · Enter opens diff · Esc goes back",
            "tip · 1–8 jump surfaces · r refreshes this tab",
            "tip · v toggles graph / list in Log",
            "tip · f fetch · p pull · u push when ready",
            "tip · Ctrl+, settings · Help for full shortcuts",
            "tip · Ctrl+W v/s split · w cycles panes",
        ];
        // Stable tip for first ~2.4s of a load, then rotate
        let base = self
            .loading_started
            .map(|t| t.elapsed().as_millis() as usize / 2400)
            .unwrap_or(0);
        // Mix with label so different tabs don't always show the same first tip
        let salt = match self.loading {
            Some(GitLoadTarget::Branches) => 0,
            Some(GitLoadTarget::PullRequests) => 1,
            Some(GitLoadTarget::Issues) => 2,
            Some(GitLoadTarget::Auth) => 3,
            None => 0,
        };
        TIPS[(base + salt) % TIPS.len()]
    }

    fn start_load(&mut self, target: GitLoadTarget) {
        if self.loading.is_some() {
            return;
        }
        // Already have data
        match target {
            GitLoadTarget::Branches if self.branches_loaded => return,
            GitLoadTarget::PullRequests if self.prs_loaded => return,
            GitLoadTarget::Issues if self.issues_loaded => return,
            GitLoadTarget::Auth if self.auth_loaded => return,
            _ => {}
        }
        let root = self.root.clone();
        let pr_state = self.pr_state;
        let issue_state = self.issue_state;
        let (tx, rx) = mpsc::channel();
        self.load_rx = Some(rx);
        self.loading = Some(target);
        self.loading_started = Some(Instant::now());
        self.message = Some(match target {
            GitLoadTarget::Branches => "Syncing branches…".into(),
            GitLoadTarget::PullRequests => "Syncing pull requests…".into(),
            GitLoadTarget::Issues => "Syncing issues…".into(),
            GitLoadTarget::Auth => "Refreshing GitHub account…".into(),
        });

        thread::spawn(move || {
            let result = match target {
                GitLoadTarget::Branches => {
                    let r = root
                        .as_ref()
                        .ok_or_else(|| "No git root".to_string())
                        .and_then(|p| git_ops::list_branches(p));
                    GitLoadResult::Branches(r)
                }
                GitLoadTarget::PullRequests => {
                    let gh_ok = gh::gh_installed();
                    if !gh_ok {
                        let _ = tx.send(GitLoadResult::Prs(Err(
                            "gh CLI not installed".into(),
                        )));
                        return;
                    }
                    let auth = gh::auth_status();
                    if auth.state != GhAuthState::LoggedIn {
                        let _ = tx.send(GitLoadResult::Auth {
                            info: auth,
                            gh_ok: true,
                        });
                        // Also signal empty PRs path via Auth first — apply_load
                        // will set auth then user can login. Send empty prs too.
                        let _ = tx.send(GitLoadResult::Prs(Ok(Vec::new())));
                        return;
                    }
                    let r = root
                        .as_ref()
                        .ok_or_else(|| "No git root".to_string())
                        .and_then(|p| gh::list_prs(p, 40, pr_state));
                    // Ensure auth is marked loaded
                    let _ = tx.send(GitLoadResult::Auth {
                        info: auth,
                        gh_ok: true,
                    });
                    GitLoadResult::Prs(r)
                }
                GitLoadTarget::Issues => {
                    let gh_ok = gh::gh_installed();
                    if !gh_ok {
                        let _ = tx.send(GitLoadResult::Issues(Err(
                            "gh CLI not installed".into(),
                        )));
                        return;
                    }
                    let auth = gh::auth_status();
                    if auth.state != GhAuthState::LoggedIn {
                        let _ = tx.send(GitLoadResult::Auth {
                            info: auth,
                            gh_ok: true,
                        });
                        let _ = tx.send(GitLoadResult::Issues(Ok(Vec::new())));
                        return;
                    }
                    let r = root
                        .as_ref()
                        .ok_or_else(|| "No git root".to_string())
                        .and_then(|p| gh::list_issues(p, 40, issue_state));
                    let _ = tx.send(GitLoadResult::Auth {
                        info: auth,
                        gh_ok: true,
                    });
                    GitLoadResult::Issues(r)
                }
                GitLoadTarget::Auth => {
                    let gh_ok = gh::gh_installed();
                    let info = if gh_ok {
                        gh::auth_status()
                    } else {
                        GhAuthInfo::default()
                    };
                    GitLoadResult::Auth { info, gh_ok }
                }
            };
            let _ = tx.send(result);
        });
    }

    /// Poll background loads. Call once per frame from the main loop.
    /// Returns true if state changed (needs redraw attention).
    pub fn poll_loading(&mut self) -> bool {
        let mut changed = self.poll_auth_login();
        // Remote action completion (push/pull/fetch)
        if let Some(rx) = self.action_rx.take() {
            match rx.try_recv() {
                Ok(result) => {
                    self.action_label = None;
                    let msg = match result {
                        Ok(m) => m.lines().next().unwrap_or("ok").to_string(),
                        Err(e) => e.lines().next().unwrap_or("git action failed").to_string(),
                    };
                    self.message = Some(msg.clone());
                    self.app_msg = Some(msg);
                    let root = self.root.clone();
                    self.refresh(root.as_deref());
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => self.action_rx = Some(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.action_label = None;
                }
            }
        }
        let Some(rx) = self.load_rx.take() else {
            return changed;
        };
        let mut disconnected = false;
        let mut batch = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(result) => batch.push(result),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        for result in batch {
            self.apply_load_result(result);
            changed = true;
        }
        // Clear loading flag once the primary target is satisfied
        let done = match self.loading {
            Some(GitLoadTarget::Branches) => self.branches_loaded,
            Some(GitLoadTarget::PullRequests) => self.prs_loaded,
            Some(GitLoadTarget::Issues) => self.issues_loaded,
            Some(GitLoadTarget::Auth) => self.auth_loaded,
            None => true,
        };
        if done || disconnected {
            self.loading = None;
            self.loading_started = None;
            // drop rx
        } else {
            // Keep waiting for more results
            self.load_rx = Some(rx);
        }
        changed
    }

    fn apply_load_result(&mut self, result: GitLoadResult) {
        match result {
            GitLoadResult::Branches(Ok(b)) => {
                self.branches = b;
                if self.branch_sel >= self.branches.len() {
                    self.branch_sel = 0;
                }
                if let Some(i) = self.branches.iter().position(|x| x.current) {
                    self.branch_sel = i;
                }
                self.branches_loaded = true;
                self.message = Some(format!("{} branches", self.branches.len()));
            }
            GitLoadResult::Branches(Err(e)) => {
                self.branches_loaded = true;
                self.error = Some(e);
            }
            GitLoadResult::Prs(Ok(p)) => {
                self.prs = p;
                self.refilter_prs();
                self.prs_loaded = true;
                self.message = Some(format!(
                    "PRs · {} · {} shown",
                    self.pr_state.label(),
                    self.pr_filtered.len()
                ));
            }
            GitLoadResult::Prs(Err(e)) => {
                self.prs_loaded = true;
                self.message = Some(e);
            }
            GitLoadResult::Issues(Ok(items)) => {
                self.issues = items;
                self.refilter_issues();
                self.issues_loaded = true;
                self.message = Some(format!(
                    "Issues · {} · {} shown",
                    self.issue_state.label(),
                    self.issue_filtered.len()
                ));
            }
            GitLoadResult::Issues(Err(e)) => {
                self.issues_loaded = true;
                self.message = Some(e);
            }
            GitLoadResult::Auth { info, gh_ok } => {
                self.auth = info;
                self.gh_available = gh_ok;
                self.auth_loaded = true;
                self.auth_action_sel = 0;
                // Always surface result after auth refresh (spinner was showing)
                self.message = Some(self.auth.detail.clone());
                self.error = None;
            }
        }
    }

    /// Open a context menu for the commit at `commit_idx` near screen (x,y).
    pub fn open_commit_ctx(&mut self, x: u16, y: u16, commit_idx: usize) {
        let n = self.commits.len().max(self.history_graph.len());
        if commit_idx >= n {
            return;
        }
        self.history_sel = commit_idx;
        self.pane = GitPane::Log;
        self.ctx_menu = Some(GitContextMenu {
            x,
            y,
            sel: 0,
            commit_idx,
            items: vec![
                GitCtxItem::ShowFiles,
                GitCtxItem::CherryPick,
                GitCtxItem::Revert,
                GitCtxItem::CopyHash,
                GitCtxItem::BrowseOnGitHub,
            ],
        });
    }

    pub fn close_ctx_menu(&mut self) {
        self.ctx_menu = None;
    }

    /// Run the selected context-menu action. Returns a status message.
    pub fn run_ctx_action(&mut self) -> Result<String, String> {
        let menu = self.ctx_menu.clone().ok_or_else(|| "No menu".to_string())?;
        let item = *menu
            .items
            .get(menu.sel)
            .ok_or_else(|| "No item".to_string())?;
        self.history_sel = menu.commit_idx;
        self.ctx_menu = None;
        match item {
            GitCtxItem::ShowFiles => {
                self.focus_files_pane()?;
                Ok(self
                    .message
                    .clone()
                    .unwrap_or_else(|| "Files".into()))
            }
            GitCtxItem::CherryPick => {
                self.cherry_pick_selected()?;
                Ok(self
                    .message
                    .clone()
                    .unwrap_or_else(|| "Cherry-picked".into()))
            }
            GitCtxItem::Revert => {
                self.revert_selected()?;
                Ok(self.message.clone().unwrap_or_else(|| "Reverted".into()))
            }
            GitCtxItem::CopyHash => {
                let h = self
                    .copy_commit_hash()
                    .ok_or_else(|| "No commit".to_string())?;
                Ok(format!("Copied {h}"))
            }
            GitCtxItem::BrowseOnGitHub => {
                self.browse_commit()?;
                Ok(self
                    .message
                    .clone()
                    .unwrap_or_else(|| "Opened browser".into()))
            }
        }
    }

    fn load_branches(&mut self) {
        let Some(ref root) = self.root.clone() else {
            return;
        };
        match git_ops::list_branches(root) {
            Ok(b) => {
                self.branches = b;
                if self.branch_sel >= self.branches.len() {
                    self.branch_sel = 0;
                }
                if let Some(i) = self.branches.iter().position(|x| x.current) {
                    self.branch_sel = i;
                }
                self.branches_loaded = true;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Lightweight: branch tip + porcelain only (what Status tab needs).
    ///
    /// Root is always re-discovered from the current file / cwd. Never prefer a
    /// stale `self.root` from a previous project (that broke Ctrl+Shift+G after
    /// switching folders while Ctrl+G still worked).
    pub fn refresh_status(&mut self, hint: Option<&Path>) {
        self.error = None;
        let discovered = git_ops::find_git_root(hint).or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|cwd| git_ops::find_git_root(Some(&cwd)))
        });
        if self.root.as_ref() != discovered.as_ref() {
            // Project changed — invalidate lazy tabs
            self.branches_loaded = false;
            self.history_loaded = false;
            self.prs_loaded = false;
            self.issues_loaded = false;
            self.commits.clear();
            self.history_graph.clear();
            self.branches.clear();
            self.prs.clear();
            self.issues.clear();
            self.pr_filtered.clear();
            self.issue_filtered.clear();
            self.commit_detail = None;
            self.diff_path = None;
            self.diff_lines.clear();
            self.diff_origin = None;
            self.stashes.clear();
            self.remotes.clear();
        }
        self.root = discovered.clone();
        let Some(root) = discovered else {
            self.error = Some("Not a git repository".into());
            self.staged.clear();
            self.changes.clear();
            self.branch.clear();
            return;
        };

        self.branch = git_ops::current_branch(&root);
        if let Ok(sb) = git_ops::run_git(&root, &["status", "-sb"]) {
            if let Some((b, a, be)) = parse_sb_branch(&sb) {
                if !b.is_empty() {
                    self.branch = b;
                }
                self.ahead = a;
                self.behind = be;
            }
        }

        self.staged.clear();
        self.changes.clear();
        if let Ok(out) = git_ops::run_git(&root, &["status", "--porcelain=v1", "-uall"]) {
            for line in out.lines() {
                for e in parse_porcelain_entries(line) {
                    if e.staged {
                        self.staged.push(e);
                    } else {
                        self.changes.push(e);
                    }
                }
            }
        }
        self.clamp_selected();
        // cheap metadata for Status footer
        self.stashes = git_ops::stash_list(&root).unwrap_or_default();
        self.remotes = git_ops::remotes(&root).unwrap_or_default();
    }

    pub fn total_files(&self) -> usize {
        self.staged.len() + self.changes.len()
    }

    pub fn entry_at(&self, idx: usize) -> Option<&ScmEntry> {
        if idx < self.staged.len() {
            self.staged.get(idx)
        } else {
            self.changes.get(idx.saturating_sub(self.staged.len()))
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

    pub fn auth_actions(&self) -> Vec<&'static str> {
        if self.auth_login.is_some() {
            return vec!["Cancel login"];
        }
        match self.auth.state {
            GhAuthState::NotInstalled => {
                vec![
                    "Refresh status",
                    "Install docs (cli.github.com)",
                    "Copy install: brew install gh",
                ]
            }
            GhAuthState::LoggedOut => {
                vec![
                    "Sign in with browser",
                    "Refresh status",
                    "Configure git credentials",
                ]
            }
            GhAuthState::LoggedIn => {
                vec![
                    "Refresh status",
                    "Sign out",
                    "Configure git credentials",
                    "Open GitHub in browser",
                ]
            }
        }
    }

    /// Drain browser-login progress; call every frame.
    pub fn poll_auth_login(&mut self) -> bool {
        let Some(session) = self.auth_login.as_mut() else {
            return false;
        };
        if let Some(result) = session.poll() {
            self.auth_login = None;
            match result {
                Ok(msg) => {
                    self.message = Some(msg);
                    self.error = None;
                    // Background verify — shows spinner, no freeze
                    self.refresh_auth();
                }
                Err(e) => {
                    self.message = Some(e.clone());
                    self.error = Some(e);
                    self.refresh_auth();
                }
            }
            return true;
        }

        // First time we see a device code: copy to clipboard + open browser + message.
        if !session.code_delivered {
            if let Some(code) = session.code.clone() {
                let url = session
                    .url
                    .clone()
                    .unwrap_or_else(|| "https://github.com/login/device".into());
                let copied = crate::clipboard::copy(&code);
                let opened = gh::open_in_browser(&url).is_ok();
                session.code_delivered = true;
                self.message = Some(format!(
                    "Code {code} {} · browser {} · paste at github.com/login/device",
                    if copied { "copied" } else { "copy failed" },
                    if opened { "opened" } else { "open failed" },
                ));
                self.error = None;
            }
        }

        true // keep redrawing for spinner / code
    }

    pub fn start_browser_login(&mut self) -> Result<(), String> {
        if self.auth_login.is_some() {
            return Err("Login already in progress".into());
        }
        if !gh::gh_installed() {
            return Err("gh CLI not installed".into());
        }
        let session = gh::auth_login_web_start()?;
        // Eagerly open the device page so the user isn't waiting on gh's delay.
        let _ = gh::open_in_browser("https://github.com/login/device");
        self.message = Some(
            "Browser opened · waiting for one-time code (will auto-copy)…".into(),
        );
        self.error = None;
        self.auth_login = Some(session);
        Ok(())
    }

    pub fn cancel_browser_login(&mut self) {
        if let Some(s) = self.auth_login.take() {
            s.cancel();
            self.message = Some("Login cancelled".into());
        }
    }

    pub fn move_sel(&mut self, delta: isize) {
        // Docked 3-pane view: j/k follow active column
        if matches!(
            self.tab,
            GitTab::Status | GitTab::History | GitTab::Commit
        ) {
            match self.pane {
                GitPane::Changes => {
                    let n = self.total_files();
                    if n == 0 {
                        return;
                    }
                    let cur = self.selected as isize + delta;
                    self.selected = cur.clamp(0, (n - 1) as isize) as usize;
                    return;
                }
                GitPane::Log => {
                    let n = self.commits.len().max(self.history_graph.len());
                    if n == 0 {
                        return;
                    }
                    let cur = self.history_sel as isize + delta;
                    self.history_sel = cur.clamp(0, (n - 1) as isize) as usize;
                    if self.history_sel + 8 >= n && self.history_limit < HISTORY_MAX {
                        let _ = self.load_more_history();
                    }
                    // Live-load right-pane detail without thrashing tab/focus.
                    let _ = self.load_selected_commit_detail();
                    return;
                }
                GitPane::Files => {
                    let n = self
                        .commit_detail
                        .as_ref()
                        .map(|d| d.files.len())
                        .unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    let cur = self.commit_file_sel as isize + delta;
                    self.commit_file_sel = cur.clamp(0, (n - 1) as isize) as usize;
                    return;
                }
            }
        }
        match self.tab {
            GitTab::Status => {
                let n = self.total_files();
                if n == 0 {
                    return;
                }
                let cur = self.selected as isize + delta;
                self.selected = cur.clamp(0, (n - 1) as isize) as usize;
            }
            GitTab::Branches => {
                let n = self.branches.len();
                if n == 0 {
                    return;
                }
                let cur = self.branch_sel as isize + delta;
                self.branch_sel = cur.clamp(0, (n - 1) as isize) as usize;
            }
            GitTab::History => {
                let n = self.commits.len().max(self.history_graph.len());
                if n == 0 {
                    return;
                }
                let cur = self.history_sel as isize + delta;
                self.history_sel = cur.clamp(0, (n - 1) as isize) as usize;
                if self.history_sel + 8 >= n && self.history_limit < HISTORY_MAX {
                    let _ = self.load_more_history();
                }
            }
            GitTab::Commit => {
                let n = self
                    .commit_detail
                    .as_ref()
                    .map(|d| d.files.len())
                    .unwrap_or(0);
                if n == 0 {
                    return;
                }
                let cur = self.commit_file_sel as isize + delta;
                self.commit_file_sel = cur.clamp(0, (n - 1) as isize) as usize;
            }
            GitTab::Diff => {
                let max = self.diff_lines.len().saturating_sub(1);
                let cur = self.diff_scroll as isize + delta;
                self.diff_scroll = cur.clamp(0, max as isize) as usize;
            }
            GitTab::PullRequests => {
                if self.pr_filter_mode {
                    return;
                }
                let n = self.pr_filtered.len();
                if n == 0 {
                    return;
                }
                let cur = self.pr_sel as isize + delta;
                self.pr_sel = cur.clamp(0, (n - 1) as isize) as usize;
            }
            GitTab::Issues => {
                if self.issue_filter_mode {
                    return;
                }
                let n = self.issue_filtered.len();
                if n == 0 {
                    return;
                }
                let cur = self.issue_sel as isize + delta;
                self.issue_sel = cur.clamp(0, (n - 1) as isize) as usize;
            }
            GitTab::Auth => {
                let n = self.auth_actions().len();
                if n == 0 {
                    return;
                }
                let cur = self.auth_action_sel as isize + delta;
                self.auth_action_sel = cur.clamp(0, (n - 1) as isize) as usize;
            }
            GitTab::Stash => {
                let n = self.stashes.len();
                if n == 0 {
                    return;
                }
                let cur = self.stash_sel as isize + delta;
                self.stash_sel = cur.clamp(0, (n - 1) as isize) as usize;
            }
        }
    }

    pub fn toggle_history_view(&mut self) {
        self.history_view = match self.history_view {
            HistoryView::List => HistoryView::Graph,
            HistoryView::Graph => HistoryView::List,
        };
        if self.history_view == HistoryView::Graph {
            if let Some(ref root) = self.root.clone() {
                if self.history_graph.is_empty() && self.history_loaded {
                    self.reload_history_graph(root);
                } else if !self.history_loaded {
                    self.reload_history(root);
                    self.history_loaded = true;
                }
            }
        }
        self.message = Some(match self.history_view {
            HistoryView::List => "History: list view".into(),
            HistoryView::Graph => "History: graph view".into(),
        });
    }

    pub fn next_tab(&mut self) {
        self.tab = match self.tab {
            GitTab::Status => GitTab::Branches,
            GitTab::Branches => GitTab::History,
            GitTab::History => {
                if self.commit_detail.is_some() {
                    GitTab::Commit
                } else if self.diff_path.is_some() {
                    GitTab::Diff
                } else {
                    GitTab::PullRequests
                }
            }
            GitTab::Commit => {
                if self.diff_path.is_some() {
                    GitTab::Diff
                } else {
                    GitTab::PullRequests
                }
            }
            GitTab::Diff => GitTab::PullRequests,
            GitTab::PullRequests => GitTab::Issues,
            GitTab::Issues => GitTab::Auth,
            GitTab::Auth => GitTab::Stash,
            GitTab::Stash => GitTab::Status,
        };
        self.focus = GitFocus::List;
        self.exit_filter_modes();
        self.ensure_tab_data();
    }

    pub fn prev_tab(&mut self) {
        self.tab = match self.tab {
            GitTab::Status => GitTab::Stash,
            GitTab::Branches => GitTab::Status,
            GitTab::History => GitTab::Branches,
            GitTab::Commit => GitTab::History,
            GitTab::Diff => {
                if self.commit_detail.is_some() {
                    GitTab::Commit
                } else {
                    GitTab::History
                }
            }
            GitTab::PullRequests => {
                if self.diff_path.is_some() {
                    GitTab::Diff
                } else if self.commit_detail.is_some() {
                    GitTab::Commit
                } else {
                    GitTab::History
                }
            }
            GitTab::Issues => GitTab::PullRequests,
            GitTab::Auth => GitTab::Issues,
            GitTab::Stash => GitTab::Auth,
        };
        self.exit_filter_modes();
        self.ensure_tab_data();
    }

    fn exit_filter_modes(&mut self) {
        self.pr_filter_mode = false;
        self.issue_filter_mode = false;
    }

    pub fn cycle_pr_state(&mut self, forward: bool) {
        self.pr_state = if forward {
            self.pr_state.next()
        } else {
            self.pr_state.prev()
        };
        self.prs_loaded = false;
        self.pr_sel = 0;
        self.prs.clear();
        self.pr_filtered.clear();
        // Drop any in-flight load so we can restart for the new state
        self.loading = None;
        self.load_rx = None;
        self.start_load(GitLoadTarget::PullRequests);
    }

    pub fn cycle_issue_state(&mut self) {
        self.issue_state = self.issue_state.next();
        self.issues_loaded = false;
        self.issue_sel = 0;
        self.issues.clear();
        self.issue_filtered.clear();
        self.loading = None;
        self.load_rx = None;
        self.start_load(GitLoadTarget::Issues);
    }

    pub fn begin_pr_filter(&mut self) {
        self.pr_filter_mode = true;
        self.message = Some("Filter PRs…  Enter apply · Esc cancel".into());
    }

    pub fn begin_issue_filter(&mut self) {
        self.issue_filter_mode = true;
        self.message = Some("Filter issues…  Enter apply · Esc cancel".into());
    }

    pub fn refilter_prs(&mut self) {
        self.pr_filtered = filter_prs(&self.prs, &self.pr_filter);
        if self.pr_sel >= self.pr_filtered.len() {
            self.pr_sel = self.pr_filtered.len().saturating_sub(1);
        }
    }

    pub fn refilter_issues(&mut self) {
        self.issue_filtered = filter_issues(&self.issues, &self.issue_filter);
        if self.issue_sel >= self.issue_filtered.len() {
            self.issue_sel = self.issue_filtered.len().saturating_sub(1);
        }
    }

    pub fn selected_pr(&self) -> Option<&PrSummary> {
        let idx = *self.pr_filtered.get(self.pr_sel)?;
        self.prs.get(idx)
    }

    pub fn selected_issue(&self) -> Option<&IssueSummary> {
        let idx = *self.issue_filtered.get(self.issue_sel)?;
        self.issues.get(idx)
    }

    pub fn go_back(&mut self) -> bool {
        match self.tab {
            GitTab::Diff => {
                match &self.diff_origin {
                    Some(DiffOrigin::Commit { .. }) if self.commit_detail.is_some() => {
                        // Back to docked Files column (not a separate Commit tab)
                        self.tab = GitTab::History;
                        self.pane = GitPane::Files;
                        self.focus = GitFocus::List;
                    }
                    Some(DiffOrigin::Worktree { .. }) => {
                        self.tab = GitTab::Status;
                        self.pane = GitPane::Changes;
                        self.focus = GitFocus::List;
                    }
                    _ => {
                        self.tab = GitTab::History;
                        self.pane = GitPane::Log;
                    }
                }
                true
            }
            GitTab::Commit => {
                self.tab = GitTab::History;
                self.pane = GitPane::Log;
                self.focus = GitFocus::List;
                true
            }
            _ => false,
        }
    }

    /// Refresh GitHub auth **in the background** (never block the UI).
    /// Shows the workbench loading spinner until `gh auth status` / API returns.
    pub fn refresh_auth(&mut self) {
        if self.auth_login.is_some() {
            self.message = Some("Finish or cancel browser login first".into());
            return;
        }
        // Fast local check only — full status hits the network.
        self.gh_available = gh::gh_installed();
        if !self.gh_available {
            self.auth = GhAuthInfo {
                state: GhAuthState::NotInstalled,
                detail: "gh CLI not installed".into(),
                ..Default::default()
            };
            self.auth_loaded = true;
            self.message = Some(self.auth.detail.clone());
            return;
        }
        // Drop any in-flight load so we can restart Auth refresh.
        self.loading = None;
        self.load_rx = None;
        self.loading_started = None;
        self.auth_loaded = false;
        self.message = Some("Refreshing GitHub account…".into());
        self.start_load(GitLoadTarget::Auth);
    }

    /// Full refresh of **current tab only** (r key). Avoids redoing everything.
    pub fn refresh(&mut self, hint: Option<&Path>) {
        self.refresh_status(hint);
        // Invalidate lazy caches so ensure reloads for heavy tabs
        self.branches_loaded = false;
        self.history_loaded = false;
        self.prs_loaded = false;
        self.issues_loaded = false;
        self.auth_loaded = false;
        self.loading = None;
        self.load_rx = None;
        match self.tab {
            GitTab::Branches => self.start_load(GitLoadTarget::Branches),
            GitTab::History | GitTab::Commit => {
                if let Some(ref root) = self.root.clone() {
                    self.reload_history(root);
                    self.history_loaded = true;
                }
            }
            GitTab::PullRequests => self.start_load(GitLoadTarget::PullRequests),
            GitTab::Issues => self.start_load(GitLoadTarget::Issues),
            GitTab::Auth => self.start_load(GitLoadTarget::Auth),
            GitTab::Stash => self.load_stashes_and_remotes(),
            GitTab::Diff => {
                if let Some(origin) = self.diff_origin.clone() {
                    match origin {
                        DiffOrigin::Worktree { path, staged } => {
                            let _ = self.load_diff(&path, staged);
                        }
                        DiffOrigin::Commit { hash, path } => {
                            let _ = self.load_commit_file_diff(&hash, &path);
                        }
                    }
                }
            }
            GitTab::Status => {}
        }
        if let Some(ref d) = self.commit_detail.clone() {
            if matches!(self.tab, GitTab::Commit) {
                let hash = d.hash.clone();
                let _ = self.open_commit(&hash);
            }
        }
    }

    pub fn ensure_history(&mut self) {
        if self.history_loaded {
            return;
        }
        if let Some(root) = self.root.clone() {
            self.reload_history(&root);
            self.history_loaded = true;
        }
    }

    pub fn reload_history(&mut self, root: &Path) {
        match git_ops::list_commits(root, self.history_limit, true) {
            Ok(c) => {
                self.commits = c;
                if self.history_sel >= self.commits.len() {
                    self.history_sel = self.commits.len().saturating_sub(1);
                }
            }
            Err(e) => self.error = Some(e),
        }
        // Graph only if user is in graph view (saves CPU when list-only)
        if self.history_view == HistoryView::Graph {
            self.reload_history_graph(root);
        } else {
            self.history_graph.clear();
        }
    }

    fn reload_history_graph(&mut self, root: &Path) {
        let limit = self.history_limit.clamp(20, HISTORY_MAX).to_string();
        if let Ok(out) = git_ops::run_git(
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
            self.history_graph = git_graph::build_graph(&out);
        } else {
            self.history_graph.clear();
        }
    }

    pub fn load_more_history(&mut self) -> Result<usize, String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let prev = self.commits.len();
        let next = (self.history_limit.saturating_mul(2)).min(HISTORY_MAX);
        if next <= self.history_limit {
            return Ok(0);
        }
        self.history_limit = next;
        self.reload_history(&root);
        Ok(self.commits.len().saturating_sub(prev))
    }

    pub fn open_selected_commit(&mut self) -> Result<(), String> {
        let hash = if self.history_view == HistoryView::Graph {
            self.history_graph
                .get(self.history_sel)
                .map(|r| r.hash.clone())
        } else {
            self.commits.get(self.history_sel).map(|c| c.hash.clone())
        }
        .ok_or_else(|| "No commit selected".to_string())?;
        self.open_commit(&hash)
    }

    /// Load commit detail into the right dock column without changing tabs.
    /// Used for j/k on Log and for key `4` (Files pane focus).
    pub fn load_selected_commit_detail(&mut self) -> Result<(), String> {
        let hash = if self.history_view == HistoryView::Graph {
            self.history_graph
                .get(self.history_sel)
                .map(|r| r.hash.clone())
        } else {
            self.commits.get(self.history_sel).map(|c| c.hash.clone())
        }
        .ok_or_else(|| "No commit selected".to_string())?;
        // Skip reload if already showing this commit
        if self
            .commit_detail
            .as_ref()
            .is_some_and(|d| d.hash == hash)
        {
            return Ok(());
        }
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let detail = git_ops::commit_detail(&root, &hash)?;
        self.commit_file_sel = 0;
        self.commit_detail = Some(detail);
        Ok(())
    }

    /// Focus the Files column (docked right pane). Ensures history + detail.
    pub fn focus_files_pane(&mut self) -> Result<(), String> {
        self.ensure_history();
        self.load_selected_commit_detail()?;
        // Stay on a docked tab (Status or History), never leave Commit as a
        // "special" surface — docked layout uses pane focus instead.
        if !matches!(self.tab, GitTab::Status | GitTab::History | GitTab::Commit) {
            self.tab = GitTab::History;
        } else if matches!(self.tab, GitTab::Commit) {
            self.tab = GitTab::History;
        }
        self.pane = GitPane::Files;
        self.focus = GitFocus::List;
        let short = self
            .commit_detail
            .as_ref()
            .map(|d| d.hash[..7.min(d.hash.len())].to_string())
            .unwrap_or_default();
        self.message = Some(format!("Files · {short}"));
        Ok(())
    }

    /// Open Diff for the context under the active docked pane.
    pub fn open_context_diff(&mut self) -> Result<(), String> {
        match self.pane {
            GitPane::Files => {
                if self.commit_detail.is_none() {
                    self.load_selected_commit_detail()?;
                }
                self.open_selected_commit_file_diff()
            }
            GitPane::Changes => self.open_selected_diff(),
            GitPane::Log => {
                // Diff of first file in selected commit, if any
                self.load_selected_commit_detail()?;
                self.pane = GitPane::Files;
                self.open_selected_commit_file_diff()
            }
        }
    }

    pub fn open_commit(&mut self, hash: &str) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let detail = git_ops::commit_detail(&root, hash)?;
        self.commit_file_sel = 0;
        self.commit_detail = Some(detail);
        // Prefer docked Files pane over a separate Commit "tab surface"
        self.tab = GitTab::History;
        self.pane = GitPane::Files;
        self.focus = GitFocus::List;
        self.message = Some(format!("Commit {}", &hash[..7.min(hash.len())]));
        Ok(())
    }

    pub fn open_selected_commit_file_diff(&mut self) -> Result<(), String> {
        let detail = self
            .commit_detail
            .as_ref()
            .ok_or_else(|| "No commit open".to_string())?;
        let hash = detail.hash.clone();
        let path = detail
            .files
            .get(self.commit_file_sel)
            .map(|f| f.path.clone())
            .ok_or_else(|| "No file selected".to_string())?;
        self.load_commit_file_diff(&hash, &path)
    }

    pub fn load_commit_file_diff(&mut self, hash: &str, path: &str) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        self.diff_lines = git_ops::commit_file_diff(&root, hash, path)?;
        self.diff_path = Some(path.to_string());
        self.diff_staged = false;
        self.diff_scroll = 0;
        self.diff_origin = Some(DiffOrigin::Commit {
            hash: hash.to_string(),
            path: path.to_string(),
        });
        self.tab = GitTab::Diff;
        self.focus = GitFocus::Diff;
        Ok(())
    }

    pub fn load_diff(&mut self, path: &str, staged: bool) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        self.diff_lines = git_ops::file_diff(&root, path, staged)?;
        self.diff_path = Some(path.to_string());
        self.diff_staged = staged;
        self.diff_scroll = 0;
        self.diff_origin = Some(DiffOrigin::Worktree {
            path: path.to_string(),
            staged,
        });
        self.tab = GitTab::Diff;
        self.focus = GitFocus::Diff;
        Ok(())
    }

    pub fn open_selected_diff(&mut self) -> Result<(), String> {
        let e = self
            .entry_at(self.selected)
            .cloned()
            .ok_or_else(|| "No file selected".to_string())?;
        self.load_diff(&e.path, e.staged)
    }

    pub fn stage_selected(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let e = self
            .entry_at(self.selected)
            .cloned()
            .ok_or_else(|| "No file selected".to_string())?;
        if e.staged {
            git_ops::run_git(&root, &["restore", "--staged", "--", &e.path])?;
            self.message = Some(format!("Unstaged {}", e.path));
        } else {
            git_ops::run_git(&root, &["add", "--", &e.path])?;
            self.message = Some(format!("Staged {}", e.path));
        }
        self.refresh_status(Some(&root));
        Ok(())
    }

    /// Commit with left-pane message buffer (JetBrains-style).
    pub fn commit_with_buf(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let msg = self.commit_buf.trim().to_string();
        if msg.is_empty() {
            return Err("Empty commit message".into());
        }
        if self.staged.is_empty() {
            if self.changes.is_empty() {
                return Err("No changes to commit".into());
            }
            // Stage all then commit (common IDE default)
            git_ops::run_git(&root, &["add", "-A"])?;
        }
        git_ops::run_git(&root, &["commit", "-m", &msg])?;
        self.commit_buf.clear();
        self.message = Some("Committed".into());
        self.refresh_status(Some(&root));
        // Refresh history if already loaded
        if self.history_loaded {
            if let Some(root) = self.root.clone() {
                self.reload_history(&root);
            }
        }
        Ok(())
    }

    pub fn cycle_pane(&mut self) {
        self.pane = match self.pane {
            GitPane::Changes => GitPane::Log,
            GitPane::Log => GitPane::Files,
            GitPane::Files => GitPane::Changes,
        };
    }

    pub fn stage_all(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let msg = git_ops::stage_all(&root)?;
        self.message = Some(msg);
        self.refresh_status(Some(&root));
        Ok(())
    }

    pub fn unstage_all(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let msg = git_ops::unstage_all(&root)?;
        self.message = Some(msg);
        self.refresh_status(Some(&root));
        Ok(())
    }

    pub fn begin_discard_selected(&mut self) -> Result<(), String> {
        let e = self
            .entry_at(self.selected)
            .cloned()
            .ok_or_else(|| "No file selected".to_string())?;
        self.input_mode = Some(InputMode::ConfirmDiscard { path: e.path });
        self.input_buf.clear();
        self.message = Some("Discard? Enter=yes  Esc=no".into());
        Ok(())
    }

    pub fn confirm_discard(&mut self) -> Result<(), String> {
        let path = match &self.input_mode {
            Some(InputMode::ConfirmDiscard { path }) => path.clone(),
            _ => return Err("No discard pending".into()),
        };
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let msg = git_ops::discard_file(&root, &path)?;
        self.input_mode = None;
        self.message = Some(msg);
        self.refresh_status(Some(&root));
        Ok(())
    }

    pub fn begin_new_branch(&mut self) {
        self.input_mode = Some(InputMode::NewBranch);
        self.input_buf.clear();
        self.message = Some("New branch name:".into());
    }

    pub fn submit_input(&mut self) -> Result<(), String> {
        match self.input_mode.clone() {
            Some(InputMode::NewBranch) => {
                let name = self.input_buf.trim().to_string();
                if name.is_empty() {
                    return Err("Empty branch name".into());
                }
                let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
                let msg = git_ops::create_branch(&root, &name)?;
                self.input_mode = None;
                self.input_buf.clear();
                self.message = Some(msg);
                self.branches_loaded = false;
                self.load_branches();
                self.refresh_status(Some(&root));
                self.tab = GitTab::Branches;
                Ok(())
            }
            Some(InputMode::ConfirmDiscard { .. }) => self.confirm_discard(),
            None => Ok(()),
        }
    }

    pub fn cancel_input(&mut self) {
        self.input_mode = None;
        self.input_buf.clear();
        self.message = Some("Cancelled".into());
    }

    pub fn delete_selected_branch(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let b = self
            .branches
            .get(self.branch_sel)
            .cloned()
            .ok_or_else(|| "No branch selected".to_string())?;
        if b.current {
            return Err("Cannot delete current branch".into());
        }
        if b.remote {
            return Err("Use git push to delete remotes (local only here)".into());
        }
        let msg = git_ops::delete_branch(&root, &b.name, false)?;
        self.message = Some(msg);
        self.branches_loaded = false;
        self.load_branches();
        Ok(())
    }

    pub fn cherry_pick_selected(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let hash = self
            .copy_commit_hash()
            .ok_or_else(|| "No commit selected".to_string())?;
        let msg = git_ops::cherry_pick(&root, &hash)?;
        self.message = Some(msg);
        self.refresh_status(Some(&root));
        self.history_loaded = false;
        Ok(())
    }

    pub fn revert_selected(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let hash = self
            .copy_commit_hash()
            .ok_or_else(|| "No commit selected".to_string())?;
        let msg = git_ops::revert_commit(&root, &hash)?;
        self.message = Some(msg);
        self.refresh_status(Some(&root));
        self.history_loaded = false;
        Ok(())
    }

    pub fn load_stashes_and_remotes(&mut self) {
        if let Some(ref root) = self.root {
            self.stashes = git_ops::stash_list(root).unwrap_or_default();
            self.remotes = git_ops::remotes(root).unwrap_or_default();
        }
    }

    /// Network git ops run on a background thread — the toolbar spinner plays
    /// while they fly and the result lands via `poll_loading`.
    pub fn remote_action(&mut self, action: RemoteAction) -> String {
        if self.action_rx.is_some() {
            return "⟳ a git action is already running…".into();
        }
        let Some(root) = self.root.clone() else {
            return "No git root".into();
        };
        let (label, f): (&'static str, fn(&std::path::Path) -> Result<String, String>) =
            match action {
                RemoteAction::Fetch => ("Fetching", |r| git_ops::fetch(r)),
                RemoteAction::Pull => ("Pulling", |r| git_ops::pull(r)),
                RemoteAction::PullRebase => ("Pull-rebasing", |r| git_ops::pull_rebase(r)),
                RemoteAction::Push => ("Pushing", |r| git_ops::push(r)),
            };
        self.action_label = Some(label);
        let (tx, rx) = std::sync::mpsc::channel();
        self.action_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(f(&root));
        });
        format!("⟳ {label}…")
    }

    /// Finished remote-action message for the app status line.
    pub fn take_app_message(&mut self) -> Option<String> {
        self.app_msg.take()
    }

    pub fn checkout_selected_branch(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let name = self
            .branches
            .get(self.branch_sel)
            .map(|b| b.name.clone())
            .ok_or_else(|| "No branch selected".to_string())?;
        let msg = git_ops::checkout_branch(&root, &name)?;
        self.message = Some(msg);
        self.refresh(Some(&root));
        Ok(())
    }

    pub fn stash(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let msg = git_ops::stash_push(&root)?;
        self.message = Some(msg);
        self.refresh(Some(&root));
        self.load_stashes_and_remotes();
        Ok(())
    }

    pub fn stash_pop(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let msg = git_ops::stash_pop(&root)?;
        self.message = Some(msg);
        self.refresh(Some(&root));
        self.load_stashes_and_remotes();
        Ok(())
    }

    pub fn stash_apply_selected(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        if self.stashes.is_empty() {
            return Err("No stashes".into());
        }
        let idx = self.stash_sel.min(self.stashes.len() - 1);
        let msg = git_ops::stash_apply(&root, idx)?;
        self.message = Some(msg);
        self.refresh(Some(&root));
        self.load_stashes_and_remotes();
        Ok(())
    }

    pub fn stash_drop_selected(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        if self.stashes.is_empty() {
            return Err("No stashes".into());
        }
        let idx = self.stash_sel.min(self.stashes.len() - 1);
        let msg = git_ops::stash_drop(&root, idx)?;
        self.message = Some(msg);
        self.load_stashes_and_remotes();
        if self.stash_sel >= self.stashes.len() {
            self.stash_sel = self.stashes.len().saturating_sub(1);
        }
        Ok(())
    }

    pub fn stash_show_selected(&mut self) -> Result<String, String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        if self.stashes.is_empty() {
            return Err("No stashes".into());
        }
        let idx = self.stash_sel.min(self.stashes.len() - 1);
        git_ops::stash_show(&root, idx)
    }

    pub fn reload_prs(&mut self) {
        self.prs.clear();
        self.pr_filtered.clear();
        if !self.auth_loaded {
            if self.gh_available {
                self.refresh_auth();
            } else {
                return;
            }
        }
        if self.auth.state != GhAuthState::LoggedIn {
            return;
        }
        let Some(ref root) = self.root else {
            return;
        };
        match gh::list_prs(root, 40, self.pr_state) {
            Ok(p) => {
                self.prs = p;
                self.refilter_prs();
                self.message = Some(format!(
                    "PRs · {} · {} shown",
                    self.pr_state.label(),
                    self.pr_filtered.len()
                ));
            }
            Err(e) => {
                self.message = Some(e);
            }
        }
    }

    pub fn reload_issues(&mut self) {
        self.issues.clear();
        self.issue_filtered.clear();
        if !self.auth_loaded {
            if self.gh_available {
                self.refresh_auth();
            } else {
                return;
            }
        }
        if self.auth.state != GhAuthState::LoggedIn {
            return;
        }
        let Some(ref root) = self.root else {
            return;
        };
        match gh::list_issues(root, 40, self.issue_state) {
            Ok(items) => {
                self.issues = items;
                self.refilter_issues();
                self.message = Some(format!(
                    "Issues · {} · {} shown",
                    self.issue_state.label(),
                    self.issue_filtered.len()
                ));
            }
            Err(e) => self.message = Some(e),
        }
    }

    pub fn checkout_selected_pr(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let num = self
            .selected_pr()
            .map(|p| p.number)
            .ok_or_else(|| "No PR selected".to_string())?;
        let msg = gh::pr_checkout(&root, num)?;
        self.message = Some(msg);
        self.refresh_status(Some(&root));
        Ok(())
    }

    pub fn merge_selected_pr(&mut self, method: &str) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let num = self
            .selected_pr()
            .map(|p| p.number)
            .ok_or_else(|| "No PR selected".to_string())?;
        let msg = gh::pr_merge(&root, num, method)?;
        self.message = Some(msg);
        self.prs_loaded = false;
        self.reload_prs();
        self.prs_loaded = true;
        Ok(())
    }

    pub fn run_auth_action(&mut self) -> Result<(), String> {
        // Cancel in-progress login takes priority
        if self.auth_login.is_some() {
            self.cancel_browser_login();
            return Ok(());
        }
        match self.auth.state {
            GhAuthState::NotInstalled => match self.auth_action_sel {
                0 => {
                    // Async — spinner via start_load
                    self.refresh_auth();
                }
                1 => {
                    self.message = Some(gh::open_gh_install_docs()?);
                }
                2 => {
                    let _ = crate::clipboard::copy("brew install gh");
                    self.message = Some("Copied: brew install gh".into());
                }
                _ => {}
            },
            GhAuthState::LoggedOut => match self.auth_action_sel {
                0 => self.start_browser_login()?,
                1 => {
                    self.refresh_auth();
                }
                2 => {
                    let msg = gh::auth_setup_git()?;
                    self.message = Some(msg);
                }
                _ => {}
            },
            GhAuthState::LoggedIn => match self.auth_action_sel {
                0 => {
                    self.refresh_auth();
                }
                1 => {
                    let msg = gh::auth_logout()?;
                    // Optimistic UI update — no network freeze
                    self.auth = GhAuthInfo {
                        state: GhAuthState::LoggedOut,
                        host: "github.com".into(),
                        detail: msg.clone(),
                        ..Default::default()
                    };
                    self.auth_loaded = true;
                    self.gh_available = true;
                    self.message = Some(msg);
                }
                2 => {
                    let msg = gh::auth_setup_git()?;
                    self.message = Some(msg);
                }
                3 => {
                    if let Some(ref root) = self.root {
                        let msg = gh::browse(root, None)?;
                        self.message = Some(msg);
                    } else {
                        let _ = std::process::Command::new("open")
                            .arg("https://github.com")
                            .status();
                        self.message = Some("Opened github.com".into());
                    }
                }
                _ => {}
            },
        }
        Ok(())
    }

    pub fn browse_repo(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let msg = gh::browse(&root, None)?;
        self.message = Some(msg);
        Ok(())
    }

    pub fn browse_commit(&mut self) -> Result<(), String> {
        let hash = self
            .commit_detail
            .as_ref()
            .map(|d| d.hash.clone())
            .or_else(|| {
                if self.history_view == HistoryView::Graph {
                    self.history_graph
                        .get(self.history_sel)
                        .map(|r| r.hash.clone())
                } else {
                    self.commits.get(self.history_sel).map(|c| c.hash.clone())
                }
            })
            .ok_or_else(|| "No commit".to_string())?;
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        let msg = gh::browse(&root, Some(&hash))?;
        self.message = Some(msg);
        Ok(())
    }

    pub fn copy_commit_hash(&self) -> Option<String> {
        self.commit_detail
            .as_ref()
            .map(|d| d.hash.clone())
            .or_else(|| {
                if self.history_view == HistoryView::Graph {
                    self.history_graph
                        .get(self.history_sel)
                        .map(|r| r.hash.clone())
                } else {
                    self.commits.get(self.history_sel).map(|c| c.hash.clone())
                }
            })
    }

    /// Create PR from current branch (title = last commit subject).
    pub fn create_pr_from_head(&mut self) -> Result<(), String> {
        let root = self.root.clone().ok_or_else(|| "No git root".to_string())?;
        if self.auth.state != GhAuthState::LoggedIn {
            return Err("Login in Auth tab first".into());
        }
        let title = self
            .commits
            .first()
            .map(|c| c.subject.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Update {}", self.branch));
        let body = format!("Created from xei Git workbench on branch `{}`.", self.branch);
        let msg = gh::pr_create(&root, &title, &body)?;
        self.message = Some(msg);
        self.prs_loaded = false;
        self.pr_state = PrListState::Open;
        self.reload_prs();
        self.prs_loaded = true;
        self.tab = GitTab::PullRequests;
        Ok(())
    }
}

fn parse_sb_branch(sb: &str) -> Option<(String, u32, u32)> {
    let first = sb.lines().next()?;
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
