//! Optional GitHub CLI (`gh`) integration — auth + PR/issue helpers.
//!
//! Auth is designed for a **TUI**: login never blocks the UI thread, device
//! codes are captured from `gh` output, and status is verified with `gh api user`.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhAuthState {
    /// `gh` binary not on PATH
    NotInstalled,
    /// Installed but not authenticated (or broken token)
    LoggedOut,
    /// Authenticated
    LoggedIn,
}

#[derive(Debug, Clone)]
pub struct GhAuthInfo {
    pub state: GhAuthState,
    /// e.g. github.com
    pub host: String,
    /// Active username when logged in
    pub user: String,
    /// protocol hint (https/ssh) if known
    pub protocol: String,
    /// OAuth scopes string when known
    pub scopes: String,
    /// keyring / env / etc.
    pub token_source: String,
    /// Raw status snippet for UI
    pub detail: String,
}

impl Default for GhAuthInfo {
    fn default() -> Self {
        Self {
            state: GhAuthState::NotInstalled,
            host: "github.com".into(),
            user: String::new(),
            protocol: String::new(),
            scopes: String::new(),
            token_source: String::new(),
            detail: String::new(),
        }
    }
}

/// Live browser/device login session (non-blocking).
#[derive(Debug)]
pub struct AuthLoginSession {
    pub rx: Receiver<AuthLoginEvent>,
    child: Arc<Mutex<Option<Child>>>,
    cancel: Arc<AtomicBool>,
    pub started: Instant,
    pub code: Option<String>,
    pub url: Option<String>,
    pub log: Vec<String>,
    /// True after we copied the code + opened the browser once.
    pub code_delivered: bool,
}

#[derive(Debug, Clone)]
pub enum AuthLoginEvent {
    /// Line of gh stdout/stderr for the log panel
    Log(String),
    /// One-time device code + verification URL
    DeviceCode { code: String, url: String },
    /// Process finished
    Done(Result<String, String>),
}

impl AuthLoginSession {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Ok(mut g) = self.child.lock() {
            if let Some(mut c) = g.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    /// Drain pending events into session fields. Returns true if finished.
    pub fn poll(&mut self) -> Option<Result<String, String>> {
        loop {
            match self.rx.try_recv() {
                Ok(AuthLoginEvent::Log(line)) => {
                    if self.log.len() < 40 {
                        self.log.push(line);
                    }
                }
                Ok(AuthLoginEvent::DeviceCode { code, url }) => {
                    self.code = Some(code);
                    self.url = Some(url);
                }
                Ok(AuthLoginEvent::Done(r)) => return Some(r),
                Err(std::sync::mpsc::TryRecvError::Empty) => return None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Some(Err("login process ended unexpectedly".into()));
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PrSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub head_ref: String,
    pub base_ref: String,
    pub is_draft: bool,
    pub state: String,
    pub url: String,
    pub updated_at: String,
}

pub fn gh_installed() -> bool {
    Command::new("which")
        .arg("gh")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Probe auth state. Prefers JSON hosts, then plain status, then `gh api user`.
pub fn auth_status() -> GhAuthInfo {
    if !gh_installed() {
        return GhAuthInfo {
            state: GhAuthState::NotInstalled,
            detail: "gh CLI not installed".into(),
            ..Default::default()
        };
    }

    // 1) JSON (gh ≥ 2.x) — always exit 0 with --json unless fatal
    if let Ok(out) = Command::new("gh")
        .args(["auth", "status", "--json", "hosts", "-h", "github.com"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(info) = parse_auth_json(&text) {
            // Double-check live token if JSON says success but we want certainty
            if info.state == GhAuthState::LoggedIn {
                if let Some(api_user) = api_login() {
                    let mut i = info;
                    if i.user.is_empty() {
                        i.user = api_user;
                    }
                    i.detail = format!("Signed in as {} · {}", i.user, i.host);
                    return i;
                }
            }
            return info;
        }
        // empty hosts object
        if text.contains("\"hosts\"") && (text.contains("{}") || text.contains("[]")) {
            return GhAuthInfo {
                state: GhAuthState::LoggedOut,
                detail: "Not signed in to GitHub".into(),
                ..Default::default()
            };
        }
    }

    // 2) Plain text fallback
    if let Ok(out) = Command::new("gh")
        .args(["auth", "status", "-h", "github.com"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let text = if !stdout.trim().is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        let info = parse_auth_plain(&text, out.status.success());
        if info.state == GhAuthState::LoggedIn {
            return info;
        }
        // 3) Gold standard: API call
        if let Some(user) = api_login() {
            return GhAuthInfo {
                state: GhAuthState::LoggedIn,
                host: "github.com".into(),
                user: user.clone(),
                protocol: "https".into(),
                detail: format!("Signed in as {user} · github.com"),
                ..Default::default()
            };
        }
        return info;
    }

    GhAuthInfo {
        state: GhAuthState::LoggedOut,
        detail: "Not signed in to GitHub".into(),
        ..Default::default()
    }
}

/// Active login via API (most reliable).
fn api_login() -> Option<String> {
    let out = Command::new("gh")
        .args(["api", "user", "-q", ".login"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let u = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if u.is_empty() || u.contains(' ') {
        None
    } else {
        Some(u)
    }
}

fn parse_auth_json(text: &str) -> Option<GhAuthInfo> {
    // Shape: {"hosts":{"github.com":[{"login":"u","state":"success","active":true,...}]}}
    if text.trim().is_empty() {
        return None;
    }
    // No accounts
    if text.contains("\"hosts\":{}") || text.contains("\"hosts\": {}") {
        return Some(GhAuthInfo {
            state: GhAuthState::LoggedOut,
            detail: "Not signed in to GitHub".into(),
            ..Default::default()
        });
    }

    let login = json_str_val(text, "login").filter(|u| !u.contains('.') && u != "github.com");
    let host = json_str_val(text, "host").unwrap_or_else(|| "github.com".into());
    let protocol = json_str_val(text, "gitProtocol").unwrap_or_default();
    let scopes = json_str_val(text, "scopes").unwrap_or_default();
    let token_source = json_str_val(text, "tokenSource").unwrap_or_default();
    let state_s = json_str_val(text, "state").unwrap_or_default();

    if let Some(user) = login {
        let ok = state_s.is_empty() || state_s == "success";
        if ok {
            return Some(GhAuthInfo {
                state: GhAuthState::LoggedIn,
                host: host.clone(),
                user: user.clone(),
                protocol,
                scopes,
                token_source,
                detail: format!("Signed in as {user} · {host}"),
            });
        }
    }

    // hosts present but no usable login
    if text.contains("\"hosts\"") {
        return Some(GhAuthInfo {
            state: GhAuthState::LoggedOut,
            host,
            detail: "Not signed in to GitHub".into(),
            ..Default::default()
        });
    }
    None
}

fn json_str_val(body: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let i = body.find(&pat)?;
    let rest = &body[i + pat.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parse_auth_plain(text: &str, success: bool) -> GhAuthInfo {
    let mut user = String::new();
    let mut host = "github.com".to_string();
    let mut protocol = String::new();
    let mut scopes = String::new();
    let mut token_source = String::new();

    for line in text.lines() {
        let l = line.trim();
        // "✓ Logged in to github.com account USER (keyring)"
        if let Some(rest) = l.split("account ").nth(1) {
            let u = rest.split_whitespace().next().unwrap_or("");
            if !u.is_empty() {
                user = u.trim_matches(|c| c == '(' || c == ')').to_string();
            }
            if l.contains("keyring") {
                token_source = "keyring".into();
            }
        }
        if l.contains("Logged in to ") {
            if let Some(h) = l.split("Logged in to ").nth(1) {
                let h = h.split_whitespace().next().unwrap_or("github.com");
                if h.contains('.') {
                    host = h.to_string();
                }
            }
        }
        if let Some(rest) = l.strip_prefix("- Git operations protocol:") {
            protocol = rest.trim().to_string();
        }
        if let Some(rest) = l.strip_prefix("- Token scopes:") {
            scopes = rest.trim().trim_matches('\'').replace("', '", ", ");
        }
    }

    if !user.is_empty() || (success && text.to_ascii_lowercase().contains("logged in")) {
        if user.is_empty() {
            user = api_login().unwrap_or_else(|| "user".into());
        }
        return GhAuthInfo {
            state: GhAuthState::LoggedIn,
            host: host.clone(),
            user: user.clone(),
            protocol,
            scopes,
            token_source,
            detail: format!("Signed in as {user} · {host}"),
        };
    }

    GhAuthInfo {
        state: GhAuthState::LoggedOut,
        host,
        detail: "Not signed in to GitHub".into(),
        ..Default::default()
    }
}

/// Start **non-blocking** browser login. UI should poll `session.poll()`.
pub fn auth_login_web_start() -> Result<AuthLoginSession, String> {
    if !gh_installed() {
        return Err("gh CLI not installed — brew install gh".into());
    }

    let (tx, rx) = mpsc::channel();
    let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let cancel = Arc::new(AtomicBool::new(false));
    let child_slot2 = Arc::clone(&child_slot);
    let cancel2 = Arc::clone(&cancel);

    thread::spawn(move || {
        let _ = tx.send(AuthLoginEvent::Log(
            "Starting gh auth login --web …".into(),
        ));

        let mut child = match Command::new("gh")
            .args([
                "auth",
                "login",
                "--hostname",
                "github.com",
                "--git-protocol",
                "https",
                "--web",
                // Copy device code to system clipboard when gh prints it
                "--clipboard",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(AuthLoginEvent::Done(Err(format!("spawn failed: {e}"))));
                return;
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        if let Ok(mut g) = child_slot2.lock() {
            *g = Some(child);
        }

        let tx2 = tx.clone();
        let reader = thread::spawn(move || {
            if let Some(out) = stdout {
                pump_login_output(out, &tx2);
            }
        });
        let tx3 = tx.clone();
        let reader_err = thread::spawn(move || {
            if let Some(err) = stderr {
                pump_login_output(err, &tx3);
            }
        });

        // Wait for process (or cancel)
        loop {
            if cancel2.load(Ordering::SeqCst) {
                if let Ok(mut g) = child_slot2.lock() {
                    if let Some(mut c) = g.take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                }
                let _ = tx.send(AuthLoginEvent::Done(Err("Login cancelled".into())));
                let _ = reader.join();
                let _ = reader_err.join();
                return;
            }
            let finished = {
                let mut g = child_slot2.lock().ok();
                if let Some(ref mut guard) = g {
                    if let Some(ref mut c) = **guard {
                        match c.try_wait() {
                            Ok(Some(status)) => Some(status.success()),
                            Ok(None) => None,
                            Err(_) => Some(false),
                        }
                    } else {
                        Some(false)
                    }
                } else {
                    Some(false)
                }
            };
            if let Some(ok) = finished {
                let _ = reader.join();
                let _ = reader_err.join();
                // Prefer live API check over exit code (gh may exit 0 mid-flow)
                thread::sleep(std::time::Duration::from_millis(400));
                let info = auth_status();
                if info.state == GhAuthState::LoggedIn {
                    let _ = tx.send(AuthLoginEvent::Done(Ok(format!(
                        "Signed in as {} · {}",
                        info.user, info.host
                    ))));
                } else if ok {
                    // process ok but not yet visible — one more check
                    thread::sleep(std::time::Duration::from_millis(800));
                    let info2 = auth_status();
                    if info2.state == GhAuthState::LoggedIn {
                        let _ = tx.send(AuthLoginEvent::Done(Ok(format!(
                            "Signed in as {} · {}",
                            info2.user, info2.host
                        ))));
                    } else {
                        let _ = tx.send(AuthLoginEvent::Done(Err(
                            "Login finished but still not signed in — try Refresh".into(),
                        )));
                    }
                } else {
                    let _ = tx.send(AuthLoginEvent::Done(Err(
                        "Login failed or was cancelled in the browser".into(),
                    )));
                }
                if let Ok(mut g) = child_slot2.lock() {
                    *g = None;
                }
                return;
            }
            thread::sleep(std::time::Duration::from_millis(120));
        }
    });

    Ok(AuthLoginSession {
        rx,
        child: child_slot,
        cancel,
        started: Instant::now(),
        code: None,
        url: None,
        log: Vec::new(),
        code_delivered: false,
    })
}

fn pump_login_output(stream: impl std::io::Read, tx: &Sender<AuthLoginEvent>) {
    let reader = BufReader::new(stream);
    for line in reader.lines().flatten() {
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        // Detect device code patterns from gh
        if let Some((code, url)) = extract_device_code(&trimmed) {
            let _ = tx.send(AuthLoginEvent::DeviceCode { code, url });
        } else if let Some(code) = extract_code_only(&trimmed) {
            let _ = tx.send(AuthLoginEvent::DeviceCode {
                code,
                url: "https://github.com/login/device".into(),
            });
        }
        let _ = tx.send(AuthLoginEvent::Log(trimmed));
    }
}

fn extract_device_code(line: &str) -> Option<(String, String)> {
    // "! First copy your one-time code: ABCD-1234"
    let code = extract_code_only(line)?;
    let url = if line.contains("http") {
        line.split_whitespace()
            .find(|w| w.starts_with("http"))
            .unwrap_or("https://github.com/login/device")
            .trim_matches(|c| c == '.' || c == ')' || c == '(')
            .to_string()
    } else {
        "https://github.com/login/device".into()
    };
    Some((code, url))
}

fn extract_code_only(line: &str) -> Option<String> {
    // Match XXXX-XXXX style one-time codes
    let lower = line.to_ascii_lowercase();
    if !(lower.contains("one-time code")
        || lower.contains("one time code")
        || lower.contains("code:"))
    {
        // still scan for pattern
    }
    for word in line.split_whitespace() {
        let w = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
        let parts: Vec<_> = w.split('-').collect();
        if parts.len() == 2
            && parts[0].len() >= 4
            && parts[1].len() >= 4
            && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_alphanumeric()))
        {
            return Some(w.to_ascii_uppercase());
        }
    }
    None
}

/// Blocking login (legacy / XLC). Prefer [`auth_login_web_start`] in the TUI.
pub fn auth_login_web() -> Result<String, String> {
    let mut session = auth_login_web_start()?;
    loop {
        if let Some(r) = session.poll() {
            return r;
        }
        thread::sleep(std::time::Duration::from_millis(80));
    }
}

/// Logout active account on github.com
pub fn auth_logout() -> Result<String, String> {
    if !gh_installed() {
        return Err("gh not installed".into());
    }
    let out = Command::new("gh")
        .args(["auth", "logout", "--hostname", "github.com"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok("Signed out of github.com".into());
    }
    let out2 = Command::new("gh")
        .args(["auth", "logout"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if out2.status.success() {
        Ok("Signed out".into())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(if err.trim().is_empty() {
            "Logout failed".into()
        } else {
            err.trim().to_string()
        })
    }
}

pub fn auth_setup_git() -> Result<String, String> {
    if !gh_installed() {
        return Err("gh not installed".into());
    }
    let out = Command::new("gh")
        .args(["auth", "setup-git"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok("Git credential helper configured via gh".into())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(if err.trim().is_empty() {
            "gh auth setup-git failed".into()
        } else {
            err.trim().to_string()
        })
    }
}

/// Open install docs in the system browser.
pub fn open_gh_install_docs() -> Result<String, String> {
    open_url("https://cli.github.com")
}

/// Open any URL in the default browser (macOS/Linux/Windows).
pub fn open_in_browser(url: &str) -> Result<String, String> {
    open_url(url)
}

fn open_url(url: &str) -> Result<String, String> {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", url]).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    }
    .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(format!("Opened {url}"))
    } else {
        Err(format!("Could not open {url}"))
    }
}

/// PR list state filter (maps to `gh pr list --state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrListState {
    #[default]
    Open,
    Closed,
    Merged,
    All,
}

impl PrListState {
    pub fn as_gh(self) -> &'static str {
        match self {
            PrListState::Open => "open",
            PrListState::Closed => "closed",
            PrListState::Merged => "merged",
            PrListState::All => "all",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PrListState::Open => "open",
            PrListState::Closed => "closed",
            PrListState::Merged => "merged",
            PrListState::All => "all",
        }
    }

    pub fn next(self) -> Self {
        match self {
            PrListState::Open => PrListState::Closed,
            PrListState::Closed => PrListState::Merged,
            PrListState::Merged => PrListState::All,
            PrListState::All => PrListState::Open,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            PrListState::Open => PrListState::All,
            PrListState::Closed => PrListState::Open,
            PrListState::Merged => PrListState::Closed,
            PrListState::All => PrListState::Merged,
        }
    }
}

/// List PRs for the current repo (requires auth).
pub fn list_prs(
    root: &Path,
    limit: usize,
    state: PrListState,
) -> Result<Vec<PrSummary>, String> {
    if !gh_installed() {
        return Err("gh not installed".into());
    }
    let lim = limit.clamp(5, 80).to_string();
    let out = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            state.as_gh(),
            "--limit",
            &lim,
            "--json",
            "number,title,author,headRefName,baseRefName,isDraft,state,url,updatedAt",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("gh pr list failed").to_string());
    }
    Ok(parse_pr_list_json(&String::from_utf8_lossy(&out.stdout)))
}

/// Client-side filter on number / title / author / head / base.
pub fn filter_prs(prs: &[PrSummary], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..prs.len()).collect();
    }
    prs.iter()
        .enumerate()
        .filter(|(_, p)| {
            p.title.to_lowercase().contains(&q)
                || p.author.to_lowercase().contains(&q)
                || p.head_ref.to_lowercase().contains(&q)
                || p.base_ref.to_lowercase().contains(&q)
                || p.number.to_string().contains(&q)
                || format!("#{}", p.number).contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

#[derive(Debug, Clone)]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: String,
    pub labels: String,
    pub url: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IssueListState {
    #[default]
    Open,
    Closed,
    All,
}

impl IssueListState {
    pub fn as_gh(self) -> &'static str {
        match self {
            IssueListState::Open => "open",
            IssueListState::Closed => "closed",
            IssueListState::All => "all",
        }
    }

    pub fn label(self) -> &'static str {
        self.as_gh()
    }

    pub fn next(self) -> Self {
        match self {
            IssueListState::Open => IssueListState::Closed,
            IssueListState::Closed => IssueListState::All,
            IssueListState::All => IssueListState::Open,
        }
    }
}

pub fn list_issues(
    root: &Path,
    limit: usize,
    state: IssueListState,
) -> Result<Vec<IssueSummary>, String> {
    if !gh_installed() {
        return Err("gh not installed".into());
    }
    let lim = limit.clamp(5, 80).to_string();
    let out = Command::new("gh")
        .args([
            "issue",
            "list",
            "--state",
            state.as_gh(),
            "--limit",
            &lim,
            "--json",
            "number,title,author,state,labels,url,updatedAt",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("gh issue list failed").to_string());
    }
    Ok(parse_issue_list_json(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_issue_list_json(text: &str) -> Vec<IssueSummary> {
    let mut out = Vec::new();
    let trimmed = text.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.is_empty() {
        return out;
    }
    for chunk in trimmed.split("},{") {
        let c = chunk.trim_matches(|ch| ch == '{' || ch == '}');
        let number = extract_u64(c, "\"number\":").unwrap_or(0);
        let title = extract_str(c, "\"title\":").unwrap_or_default();
        let author = extract_str(c, "\"login\":").unwrap_or_default();
        let state = extract_str(c, "\"state\":").unwrap_or_else(|| "OPEN".into());
        let url = extract_str(c, "\"url\":").unwrap_or_default();
        let updated_at = extract_str(c, "\"updatedAt\":").unwrap_or_default();
        // labels: collect "name" fields crudely
        let mut labels = Vec::new();
        let mut search = c;
        while let Some(i) = search.find("\"name\":\"") {
            let rest = &search[i + 8..];
            if let Some(end) = rest.find('"') {
                labels.push(rest[..end].to_string());
                search = &rest[end + 1..];
            } else {
                break;
            }
        }
        // first "name" might be author in nested objects — prefer labels after "labels"
        let labels = if let Some(li) = c.find("\"labels\"") {
            let sub = &c[li..];
            let mut ls = Vec::new();
            let mut s = sub;
            while let Some(i) = s.find("\"name\":\"") {
                let rest = &s[i + 8..];
                if let Some(end) = rest.find('"') {
                    ls.push(rest[..end].to_string());
                    s = &rest[end + 1..];
                } else {
                    break;
                }
            }
            ls.join(", ")
        } else {
            labels.join(", ")
        };
        if number > 0 {
            out.push(IssueSummary {
                number,
                title,
                author,
                state,
                labels,
                url,
                updated_at,
            });
        }
    }
    out
}

pub fn filter_issues(items: &[IssueSummary], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..items.len()).collect();
    }
    items
        .iter()
        .enumerate()
        .filter(|(_, it)| {
            it.title.to_lowercase().contains(&q)
                || it.author.to_lowercase().contains(&q)
                || it.labels.to_lowercase().contains(&q)
                || it.number.to_string().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

pub fn pr_merge(root: &Path, number: u64, method: &str) -> Result<String, String> {
    // method: merge | squash | rebase
    let n = number.to_string();
    let out = Command::new("gh")
        .args(["pr", "merge", &n, &format!("--{method}"), "--delete-branch=false"])
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(format!("Merged PR #{number} ({method})"))
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().next().unwrap_or("pr merge failed").to_string())
    }
}

fn parse_pr_list_json(text: &str) -> Vec<PrSummary> {
    // Split objects roughly by },{
    let mut prs = Vec::new();
    let trimmed = text.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.is_empty() {
        return prs;
    }
    for chunk in trimmed.split("},{") {
        let c = chunk.trim_matches(|ch| ch == '{' || ch == '}');
        let number = extract_u64(c, "\"number\":").unwrap_or(0);
        let title = extract_str(c, "\"title\":").unwrap_or_default();
        let head_ref = extract_str(c, "\"headRefName\":").unwrap_or_default();
        let base_ref = extract_str(c, "\"baseRefName\":").unwrap_or_default();
        let url = extract_str(c, "\"url\":").unwrap_or_default();
        let state = extract_str(c, "\"state\":").unwrap_or_else(|| "OPEN".into());
        let updated_at = extract_str(c, "\"updatedAt\":").unwrap_or_default();
        let is_draft = c.contains("\"isDraft\":true");
        // author: {"login":"x"}
        let author = extract_str(c, "\"login\":").unwrap_or_default();
        if number > 0 {
            prs.push(PrSummary {
                number,
                title,
                author,
                head_ref,
                base_ref,
                is_draft,
                state,
                url,
                updated_at,
            });
        }
    }
    prs
}

pub fn pr_checkout(root: &Path, number: u64) -> Result<String, String> {
    let n = number.to_string();
    let out = Command::new("gh")
        .args(["pr", "checkout", &n])
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(format!("Checked out PR #{number}"))
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().next().unwrap_or("pr checkout failed").to_string())
    }
}

pub fn pr_create(root: &Path, title: &str, body: &str) -> Result<String, String> {
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "create", "--title", title, "--body", body])
        .current_dir(root);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Ok(if url.is_empty() {
            "PR created".into()
        } else {
            url
        })
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().next().unwrap_or("pr create failed").to_string())
    }
}

pub fn browse(root: &Path, target: Option<&str>) -> Result<String, String> {
    let mut args = vec!["browse".to_string()];
    if let Some(t) = target {
        args.push(t.to_string());
    }
    let status = Command::new("gh")
        .args(&args)
        .current_dir(root)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok("Opened in browser".into())
    } else {
        Err("gh browse failed".into())
    }
}

fn extract_str(text: &str, key: &str) -> Option<String> {
    let i = text.find(key)?;
    let rest = text[i + key.len()..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let rest = &rest[1..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else if c == '"' {
            break;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

fn extract_u64(text: &str, key: &str) -> Option<u64> {
    let i = text.find(key)?;
    let rest = text[i + key.len()..].trim_start();
    let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    n.parse().ok()
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    #[test]
    fn parse_hosts_json_logged_in() {
        let j = r#"{"hosts":{"github.com":[{"state":"success","active":true,"host":"github.com","login":"stremtec","tokenSource":"keyring","scopes":"gist, read:org, repo","gitProtocol":"https"}]}}"#;
        let info = parse_auth_json(j).expect("parse");
        assert_eq!(info.state, GhAuthState::LoggedIn);
        assert_eq!(info.user, "stremtec");
        assert_eq!(info.protocol, "https");
        assert!(info.scopes.contains("repo"));
    }

    #[test]
    fn parse_hosts_json_empty() {
        let j = r#"{"hosts":{}}"#;
        let info = parse_auth_json(j).expect("parse");
        assert_eq!(info.state, GhAuthState::LoggedOut);
    }

    #[test]
    fn extract_device_code_line() {
        let (c, u) = extract_device_code(
            "! First copy your one-time code: ABCD-1234",
        )
        .expect("code");
        assert_eq!(c, "ABCD-1234");
        assert!(u.contains("github.com"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_json_minimal() {
        let j = r#"[{"number":12,"title":"Fix bug","author":{"login":"alice"},"headRefName":"fix","baseRefName":"main","isDraft":false,"state":"OPEN","url":"https://example.com","updatedAt":"2024-01-01"}]"#;
        let prs = parse_pr_list_json(j);
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 12);
        assert_eq!(prs[0].title, "Fix bug");
        assert_eq!(prs[0].author, "alice");
    }

    #[test]
    fn filter_prs_by_title() {
        let prs = parse_pr_list_json(
            r#"[{"number":1,"title":"Alpha","author":{"login":"a"},"headRefName":"a","baseRefName":"main","isDraft":false,"state":"OPEN","url":"u","updatedAt":"t"},{"number":2,"title":"Beta fix","author":{"login":"b"},"headRefName":"b","baseRefName":"main","isDraft":false,"state":"OPEN","url":"u","updatedAt":"t"}]"#,
        );
        let idx = filter_prs(&prs, "beta");
        assert_eq!(idx, vec![1]);
        assert_eq!(filter_prs(&prs, "#1"), vec![0]);
    }

    #[test]
    fn pr_state_cycle() {
        assert_eq!(PrListState::Open.next(), PrListState::Closed);
        assert_eq!(PrListState::Merged.next(), PrListState::All);
        assert_eq!(PrListState::All.next(), PrListState::Open);
    }

    #[test]
    fn extract_helpers() {
        assert_eq!(
            extract_str(r#"{"title":"Hello \"x\""}"#, "\"title\":").as_deref(),
            Some("Hello \"x\"")
        );
    }
}
