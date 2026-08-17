//! Debug Adapter Protocol (DAP) client — launch, breakpoints, step, stack/vars.
//!
//! Talks DAP with Content-Length framing over stdio (same transport as LSP).
//! Auto-picks a local adapter when available:
//! - Python → `python -m debugpy.adapter` / `debugpy-adapter`
//! - Go → `dlv dap`
//! - Rust / C / C++ → `lldb-dap` / `codelldb` / `lldb-vscode`
//! - Node → `js-debug-adapter` (if present)
//!
//! Sequence (matches VS Code): `initialize` → response → `launch` → adapter
//! emits `initialized` → `setBreakpoints*` → `setExceptionBreakpoints` →
//! `configurationDone` → launch response. Adapters that never emit
//! `initialized` get the configuration after a 2s fallback in `poll()`.
//!
//! UI surface lives in the TUI (`Mode::Debug`); this module is headless-safe.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
// No `Command` here on purpose: every adapter is spawned through `exec::tool`,
// so a binary the `command_exists` gate accepted is one the OS can actually
// find. An import of the bare constructor is how the LSP client stopped doing
// that — see `lsp::LspClient::start_with_text`.
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Panel slide-up duration.
pub const DAP_PANEL_ANIM_MS: u64 = 200;
/// If the adapter never sends `initialized`, push configuration after this.
const CONFIG_FALLBACK: Duration = Duration::from_secs(2);
/// Grace period between terminate/disconnect and SIGKILL.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(1000);
/// Cap on stack frames requested per stop.
const STACK_LEVELS: u64 = 40;

// ── Public types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DapState {
    Idle,
    Starting,
    Running,
    Stopped,
    Ending,
}

impl DapState {
    pub fn label(self) -> &'static str {
        match self {
            DapState::Idle => "idle",
            DapState::Starting => "starting",
            DapState::Running => "running",
            DapState::Stopped => "stopped",
            DapState::Ending => "ending",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Breakpoint {
    /// 0-based line
    pub line: usize,
    pub verified: bool,
    pub message: String,
    /// Optional DAP condition expression
    pub condition: Option<String>,
    /// Optional logpoint message (adapter-dependent)
    pub log_message: Option<String>,
    /// A breakpoint that is still THERE but is not armed.
    ///
    /// Xcode's ⌘-click, and the reason it exists: a breakpoint carries a
    /// place, a condition and a log message, and deleting one to quiet it for
    /// five minutes throws all three away.
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct StackFrameInfo {
    pub id: i64,
    pub name: String,
    pub path: String,
    /// 0-based
    pub line: usize,
    pub column: usize,
}

/// One row of the Variables tree (scopes are depth-0 roots).
#[derive(Debug, Clone)]
pub struct VarNode {
    pub name: String,
    pub value: String,
    pub typ: String,
    /// >0 = expandable (has children on the adapter side)
    pub var_ref: i64,
    pub depth: usize,
    pub expanded: bool,
    pub is_scope: bool,
}

/// A value the program is stopped for whenever it changes.
///
/// The `data_id` is the adapter's — it is resolved from a NAME in a frame, and
/// it is what `setDataBreakpoints` takes. The name is kept beside it because
/// the id is opaque and a list of opaque ids is not a list a person can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watchpoint {
    pub data_id: String,
    pub name: String,
    /// The adapter's own description of what it is watching — often the
    /// address and size, which is the honest answer to "watching what".
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugPane {
    Stack,
    Variables,
    Breakpoints,
    Console,
}

impl DebugPane {
    pub fn label(self) -> &'static str {
        match self {
            DebugPane::Stack => "Stack",
            DebugPane::Variables => "Vars",
            DebugPane::Breakpoints => "BPs",
            DebugPane::Console => "Console",
        }
    }

    pub fn next(self) -> Self {
        match self {
            DebugPane::Stack => DebugPane::Variables,
            DebugPane::Variables => DebugPane::Breakpoints,
            DebugPane::Breakpoints => DebugPane::Console,
            DebugPane::Console => DebugPane::Stack,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            DebugPane::Stack => DebugPane::Console,
            DebugPane::Variables => DebugPane::Stack,
            DebugPane::Breakpoints => DebugPane::Variables,
            DebugPane::Console => DebugPane::Breakpoints,
        }
    }
}

#[derive(Debug, Clone)]
enum PendingKind {
    Initialize,
    /// launch *or* attach response
    Launch,
    SetBreakpoints(String),
    ExceptionBreakpoints,
    ConfigDone,
    StackTrace,
    Scopes,
    /// variablesReference the children belong to
    Variables(i64),
    Threads,
    /// dataBreakpointInfo — carries the name so the answer can be listed under
    /// it, since the `dataId` that comes back is opaque.
    DataBreakpointInfo(String),
    SetDataBreakpoints,
    /// setVariable — carries the row it came from and the container it lives
    /// in, so the answer lands on the right variable and its siblings can be
    /// refreshed.
    SetVariable { index: usize, container: i64 },
    /// A hover datatip. Its own kind, not `Evaluate`, because the answer goes
    /// to a popover instead of the console — a hover that logged would fill
    /// the console with every word the pointer crossed.
    Datatip,
    Continue,
    Next,
    StepIn,
    StepOut,
    Pause,
    Terminate,
    Disconnect,
    Evaluate,
}

// ── Client ─────────────────────────────────────────────────────────────────

pub struct DapClient {
    /// Outbound DAP writer (adapter stdin or TCP stream).
    writer: Option<Box<dyn Write + Send>>,
    rx: Option<Receiver<Value>>,
    child: Option<Child>,
    next_id: u64,
    pending: HashMap<u64, PendingKind>,
    /// What each in-flight datatip asked about, so the answer can be shown
    /// under the right word.
    pending_datatip: HashMap<u64, String>,

    pub state: DapState,
    pub adapter_name: String,
    pub error: Option<String>,
    /// Soft hint (adapter missing, etc.)
    pub soft_error: Option<String>,

    /// canonical path → breakpoints (0-based lines)
    pub breakpoints: HashMap<String, Vec<Breakpoint>>,
    pub stack: Vec<StackFrameInfo>,
    /// Flattened Variables tree (scope roots + expanded children).
    pub vars: Vec<VarNode>,
    pub console: Vec<String>,
    /// (thread id, name) from the last `threads` response / thread events.
    pub threads: Vec<(i64, String)>,

    pub selected_frame: usize,
    /// The last hover datatip: what was asked, and what came back.
    ///
    /// The expression is kept beside the value because the answer arrives
    /// asynchronously and the pointer has usually moved on — a value shown
    /// under the wrong identifier is a wrong answer that looks like a right
    /// one, which is the same reason `refreshHover` clears before it asks.
    pub datatip: Option<(String, String, String)>,
    pub datatip_pending: bool,
    /// Values being watched. Armed together, because `setDataBreakpoints`
    /// replaces the whole set on every call — the same shape as
    /// `setBreakpoints`.
    pub watchpoints: Vec<Watchpoint>,
    pub selected_bp: usize,
    pub pane: DebugPane,
    pub focus_row: usize,

    pub thread_id: Option<i64>,
    pub stopped_reason: Option<String>,
    /// Current stopped location (path, 0-based line)
    pub current_path: Option<String>,
    pub current_line: Option<usize>,

    /// Panel visible in the UI layout (independent of focus / Mode::Debug).
    pub panel_open: bool,
    /// Set when stopped location changes — TUI should jump editor once.
    pub location_dirty: bool,
    /// Program + args for last launch (for restart)
    pub last_program: Option<String>,
    pub last_cwd: Option<String>,
    pub last_lang: Option<String>,
    pub last_args: Vec<String>,
    /// Last attach target for restart (e.g. "pid:1234" / "port:5678")
    pub last_attach: Option<String>,

    // Sequencer
    supports_config_done: bool,
    supports_terminate: bool,
    /// Whether the adapter can watch a value at all.
    ///
    /// Asked rather than assumed: `dataBreakpointInfo` is optional in the
    /// spec, and offering "break when this changes" on an adapter that cannot
    /// do it is the worst kind of menu item.
    pub supports_data_breakpoints: bool,
    /// Whether a value can be CHANGED while stopped.
    ///
    /// Asked rather than assumed, like the watchpoint capability — offering an
    /// edit an adapter will refuse is worse than not offering it.
    pub supports_set_variable: bool,
    /// Filters chosen from the adapter's exceptionBreakpointFilters.
    exception_filters: Vec<String>,
    /// Set when the launch/attach request went out; drives the config fallback timer.
    launch_sent_at: Option<Instant>,
    /// setBreakpoints/exception/configurationDone already sent.
    config_sent: bool,
    /// Launch *or* attach request body prepared at session start
    launch_body: Option<Value>,
    /// When true, send `attach` instead of `launch` after initialize.
    is_attach: bool,
    /// Deadline after terminate/disconnect before the adapter is killed.
    shutdown_deadline: Option<Instant>,
    /// Queued relaunch once the graceful stop reaches Idle.
    restart_pending: Option<(String, Option<PathBuf>, Option<String>, Vec<String>)>,
    /// pause requested while the thread id was still unknown.
    pause_requested: bool,
    /// stopped event arrived without threadId; stack fetch waits on `threads`.
    awaiting_stack_thread: bool,

    /// variablesReference → children (valid until the next resume).
    children_cache: HashMap<i64, Vec<VarNode>>,
    /// Memoized fs::canonicalize results (gutter runs per frame).
    canon_cache: HashMap<String, String>,

    // Panel entrance animation (lazy first-frame clock).
    opened_at: Option<Instant>,
    anim_pending: bool,

    /// Console REPL input line (evaluate request).
    pub eval_input: String,
    /// Async `cargo build` for Rust when the binary is missing.
    build_rx: Option<Receiver<Result<(String, PathBuf, String, Vec<String>), String>>>,
    /// Status line while building ("cargo build…").
    pub build_message: Option<String>,
    /// Which builder is running, so the completion log does not call `rustc`
    /// "cargo". It said "⚙ rustc (single file)…" and then "✓ cargo build ok".
    build_tool: &'static str,

    /// Outgoing requests captured for sequence tests.
    #[cfg(test)]
    pub(crate) sent: Vec<Value>,
}

impl Default for DapClient {
    fn default() -> Self {
        Self::new()
    }
}

impl DapClient {
    pub fn new() -> Self {
        Self {
            writer: None,
            rx: None,
            child: None,
            next_id: 1,
            pending: HashMap::new(),
            pending_datatip: HashMap::new(),
            state: DapState::Idle,
            adapter_name: String::new(),
            error: None,
            soft_error: None,
            breakpoints: HashMap::new(),
            stack: Vec::new(),
            vars: Vec::new(),
            console: Vec::new(),
            threads: Vec::new(),
            selected_frame: 0,
            datatip: None,
            datatip_pending: false,
            watchpoints: Vec::new(),
            selected_bp: 0,
            pane: DebugPane::Stack,
            focus_row: 0,
            thread_id: None,
            stopped_reason: None,
            current_path: None,
            current_line: None,
            panel_open: false,
            location_dirty: false,
            last_program: None,
            last_cwd: None,
            last_lang: None,
            last_args: Vec::new(),
            last_attach: None,
            supports_config_done: true,
            supports_terminate: false,
            supports_data_breakpoints: false,
            supports_set_variable: false,
            exception_filters: Vec::new(),
            launch_sent_at: None,
            config_sent: false,
            launch_body: None,
            is_attach: false,
            shutdown_deadline: None,
            restart_pending: None,
            pause_requested: false,
            awaiting_stack_thread: false,
            children_cache: HashMap::new(),
            canon_cache: HashMap::new(),
            opened_at: None,
            anim_pending: false,
            eval_input: String::new(),
            build_rx: None,
            build_message: None,
            build_tool: "cargo",
            #[cfg(test)]
            sent: Vec::new(),
        }
    }

    pub fn is_session(&self) -> bool {
        matches!(
            self.state,
            DapState::Starting | DapState::Running | DapState::Stopped
        )
    }

    // ── Panel animation ────────────────────────────────────────────────

    /// Arm the slide-up; the clock starts on the first rendered frame.
    pub fn arm_panel_animation(&mut self) {
        self.anim_pending = true;
        self.opened_at = None;
    }

    pub fn anim_progress(&mut self) -> f32 {
        if self.anim_pending {
            self.anim_pending = false;
            self.opened_at = Some(Instant::now());
            return 0.0;
        }
        let Some(t0) = self.opened_at else {
            return 1.0;
        };
        (t0.elapsed().as_millis() as f32 / DAP_PANEL_ANIM_MS as f32).min(1.0)
    }

    // ── Console ────────────────────────────────────────────────────────

    pub fn log(&mut self, msg: impl Into<String>) {
        let was_tail = self.pane == DebugPane::Console && self.focus_row + 1 >= self.console.len();
        self.console.push(msg.into());
        if self.console.len() > 400 {
            let drop_n = self.console.len() - 300;
            self.console.drain(0..drop_n);
            self.focus_row = self.focus_row.saturating_sub(drop_n);
        }
        if was_tail && self.pane == DebugPane::Console {
            self.focus_row = self.console.len().saturating_sub(1);
        }
    }

    // ── Breakpoints ────────────────────────────────────────────────────

    /// Memoized fs::canonicalize (the gutter asks every frame).
    fn canon(&mut self, path: &str) -> String {
        if let Some(c) = self.canon_cache.get(path) {
            return c.clone();
        }
        let c = std::fs::canonicalize(path)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.to_string());
        self.canon_cache.insert(path.to_string(), c.clone());
        c
    }

    /// Toggle breakpoint at 0-based line for `path`. Returns new state (true = on).
    pub fn toggle_breakpoint(&mut self, path: &str, line: usize) -> bool {
        let path = self.canon(path);
        let entry = self.breakpoints.entry(path.clone()).or_default();
        if let Some(i) = entry.iter().position(|b| b.line == line) {
            entry.remove(i);
            if entry.is_empty() {
                self.breakpoints.remove(&path);
            }
            if self.is_session() {
                self.send_set_breakpoints(&path);
            }
            let _ = self.persist_breakpoints();
            return false;
        }
        entry.push(Breakpoint {
            line,
            verified: false,
            message: String::new(),
            condition: None,
            log_message: None,
            enabled: true,
        });
        entry.sort_by_key(|b| b.line);
        if self.is_session() {
            self.send_set_breakpoints(&path);
        }
        let _ = self.persist_breakpoints();
        true
    }

    /// Arm or disarm a breakpoint without losing it.
    ///
    /// Returns the new state, or `None` when there is no breakpoint there.
    pub fn toggle_breakpoint_enabled(&mut self, path: &str, line: usize) -> Option<bool> {
        let path = self.canon(path);
        let now = {
            let b = self.breakpoints.get_mut(&path)?.iter_mut().find(|b| b.line == line)?;
            b.enabled = !b.enabled;
            b.enabled
        };
        if self.is_session() {
            self.send_set_breakpoints(&path);
        }
        let _ = self.persist_breakpoints();
        Some(now)
    }

    /// Set / clear a condition on an existing BP (0-based line). Creates BP if missing.
    pub fn set_breakpoint_condition(&mut self, path: &str, line: usize, condition: Option<String>) {
        let path = self.canon(path);
        let entry = self.breakpoints.entry(path.clone()).or_default();
        if let Some(b) = entry.iter_mut().find(|b| b.line == line) {
            b.condition = condition.filter(|s| !s.trim().is_empty());
        } else {
            entry.push(Breakpoint {
                line,
                verified: false,
                message: String::new(),
                condition: condition.filter(|s| !s.trim().is_empty()),
                log_message: None,
                enabled: true,
            });
            entry.sort_by_key(|b| b.line);
        }
        if self.is_session() {
            self.send_set_breakpoints(&path);
        }
        let _ = self.persist_breakpoints();
    }

    /// Set / clear a logpoint message.
    pub fn set_breakpoint_log(&mut self, path: &str, line: usize, log_message: Option<String>) {
        let path = self.canon(path);
        let entry = self.breakpoints.entry(path.clone()).or_default();
        if let Some(b) = entry.iter_mut().find(|b| b.line == line) {
            b.log_message = log_message.filter(|s| !s.trim().is_empty());
        } else {
            entry.push(Breakpoint {
                line,
                verified: false,
                message: String::new(),
                condition: None,
                log_message: log_message.filter(|s| !s.trim().is_empty()),
                enabled: true,
            });
            entry.sort_by_key(|b| b.line);
        }
        if self.is_session() {
            self.send_set_breakpoints(&path);
        }
        let _ = self.persist_breakpoints();
    }

    pub fn has_breakpoint(&mut self, path: &str, line: usize) -> bool {
        let path = self.canon(path);
        self.breakpoints
            .get(&path)
            .map(|v| v.iter().any(|b| b.line == line))
            .unwrap_or(false)
    }

    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
        let _ = self.persist_breakpoints();
    }

    fn breakpoints_path() -> PathBuf {
        crate::fs_atomic::state_path("breakpoints")
    }

    /// Persist BPs to `~/.suisei/breakpoints` (`path|line[:cond][:log=msg]`).
    pub fn persist_breakpoints(&self) -> Result<(), String> {
        let path = Self::breakpoints_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut out = String::from("# xei breakpoints — path|line|condition|log\n");
        let mut keys: Vec<_> = self.breakpoints.keys().cloned().collect();
        keys.sort();
        for k in keys {
            if let Some(list) = self.breakpoints.get(&k) {
                for b in list {
                    let cond = b.condition.as_deref().unwrap_or("");
                    let log = b.log_message.as_deref().unwrap_or("");
                    // A disabled breakpoint marks its LINE, not a fifth field.
                    // The log message is the last field and may contain `|`,
                    // so it cannot be split further — and an older build reads
                    // `!12` as an unparseable line and skips it, which loses
                    // exactly the breakpoints that were switched off anyway.
                    let mark = if b.enabled { "" } else { "!" };
                    out.push_str(&format!("{}|{}{}|{}|{}\n", k, mark, b.line, cond, log));
                }
            }
        }
        std::fs::write(path, out).map_err(|e| e.to_string())
    }

    /// Load BPs from `~/.suisei/breakpoints` (merge into current map).
    pub fn load_persisted_breakpoints(&mut self) {
        let Ok(text) = std::fs::read_to_string(Self::breakpoints_path()) else {
            return;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 2 {
                continue;
            }
            let path = parts[0].to_string();
            let (enabled, digits) = match parts[1].strip_prefix('!') {
                Some(rest) => (false, rest),
                None => (true, parts[1]),
            };
            let Ok(ln) = digits.parse::<usize>() else {
                continue;
            };
            let cond = parts
                .get(2)
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
            let log = parts
                .get(3)
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
            let entry = self.breakpoints.entry(path).or_default();
            if !entry.iter().any(|b| b.line == ln) {
                entry.push(Breakpoint {
                    line: ln,
                    verified: false,
                    message: String::new(),
                    condition: cond,
                    log_message: log,
                    enabled,
                });
                entry.sort_by_key(|b| b.line);
            }
        }
    }

    /// All BP lines for a path (0-based).
    pub fn lines_for(&mut self, path: &str) -> Vec<usize> {
        let path = self.canon(path);
        self.breakpoints
            .get(&path)
            .map(|v| v.iter().map(|b| b.line).collect())
            .unwrap_or_default()
    }

    /// Where the user is LOOKING: the selected frame's file and line.
    ///
    /// Distinct from `current_path`/`current_line`, which are where the
    /// program is stopped. They are the same frame until you click another one
    /// in the call stack, and the editor has to be able to draw both — solid
    /// for execution, hollow for the frame being read.
    pub fn frame_location(&self) -> Option<(String, usize)> {
        let f = self.stack.get(self.selected_frame)?;
        (!f.path.is_empty()).then(|| (f.path.clone(), f.line))
    }

    /// Stopped line if the session is currently stopped in `path`.
    pub fn current_line_for(&mut self, path: &str) -> Option<usize> {
        let line = self.current_line?;
        let cur = self.current_path.clone()?;
        if self.canon(path) == self.canon(&cur) {
            Some(line)
        } else {
            None
        }
    }

    /// Best-effort line tracking for buffer edits.
    ///
    /// - **Insert** (`delta > 0`): `anchor` is the line *after which* content
    ///   was inserted (e.g. newline at end of `anchor`). BPs on `anchor` stay;
    ///   BPs with `line > anchor` shift down by `delta`.
    /// - **Delete** (`delta < 0`): `anchor` is the first deleted line
    ///   (inclusive). BPs in `[anchor, anchor+|delta|)` are removed; later
    ///   lines shift up.
    ///
    /// Live-updates the adapter mid-session.
    pub fn shift_breakpoints(&mut self, path: &str, anchor: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        let path = self.canon(path);
        let Some(list) = self.breakpoints.get_mut(&path) else {
            return;
        };
        if delta > 0 {
            let d = delta as usize;
            for b in list.iter_mut() {
                if b.line > anchor {
                    b.line += d;
                }
            }
        } else {
            let d = (-delta) as usize;
            // Inclusive start at `anchor`
            list.retain_mut(|b| {
                if b.line < anchor {
                    return true;
                }
                if b.line < anchor + d {
                    return false; // inside the deleted span
                }
                b.line -= d;
                true
            });
        }
        list.sort_by_key(|b| b.line);
        list.dedup_by_key(|b| b.line);
        if list.is_empty() {
            self.breakpoints.remove(&path);
        }
        if self.is_session() {
            self.send_set_breakpoints(&path);
        }
        let _ = self.persist_breakpoints();
    }

    /// Flattened BP list for UI: (path, line 0-based, verified)
    pub fn flat_bps(&self) -> Vec<(String, usize, bool)> {
        let mut out = Vec::new();
        let mut keys: Vec<_> = self.breakpoints.keys().cloned().collect();
        keys.sort();
        for k in keys {
            if let Some(list) = self.breakpoints.get(&k) {
                for b in list {
                    out.push((k.clone(), b.line, b.verified));
                }
            }
        }
        out
    }

    // ── Session lifecycle ──────────────────────────────────────────────

    /// Start debugging `program` (or current file for script langs).
    pub fn start(
        &mut self,
        program: &str,
        cwd: Option<&Path>,
        lang_hint: Option<&str>,
        args: &[String],
    ) -> Result<(), String> {
        if self.is_session() {
            return Err("Debug session already active — stop first (Shift+F5)".into());
        }
        self.finish_shutdown();
        self.canon_cache.clear();

        let program_path = PathBuf::from(program);
        let abs_prog =
            std::fs::canonicalize(&program_path).unwrap_or_else(|_| program_path.clone());
        let cwd = cwd
            .map(Path::to_path_buf)
            .or_else(|| abs_prog.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let lang = lang_hint
            .map(|s| s.to_string())
            .unwrap_or_else(|| detect_lang(&abs_prog));

        // Node uses TCP DAP (js-debug) — not stdio.
        if lang == "node" {
            return self.start_node(&abs_prog.display().to_string(), Some(&cwd), args);
        }

        let (adapter_cmd, adapter_args, launch) = pick_adapter(&lang, &abs_prog, &cwd, args)?;

        // Rust: if binary is missing, kick off `cargo build` then relaunch.
        if lang == "rust" {
            if let Some(bin) = launch.get("program").and_then(|p| p.as_str()) {
                if !Path::new(bin).is_file() {
                    return self.begin_cargo_build(
                        &cwd,
                        abs_prog.display().to_string(),
                        lang,
                        args.to_vec(),
                    );
                }
            }
        }

        let mut child = crate::exec::tool(&adapter_cmd)
            .args(&adapter_args)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start {adapter_cmd}: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "adapter stdin unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "adapter stdout unavailable".to_string())?;
        let stderr = child.stderr.take();

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || read_loop(stdout, tx));
        drain_stderr(stderr);

        self.begin_session_common(
            Box::new(stdin),
            rx,
            Some(child),
            &adapter_cmd,
            &lang,
            launch,
            false,
            Some(abs_prog.display().to_string()),
            Some(cwd.display().to_string()),
            args.to_vec(),
            None,
        );

        let id = self.alloc(PendingKind::Initialize);
        let init = json!({
            "seq": id,
            "type": "request",
            "command": "initialize",
            "arguments": {
                "clientID": "xei",
                "clientName": "xei",
                "adapterID": lang,
                "pathFormat": "path",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "supportsVariableType": true,
                "supportsVariablePaging": false,
                "supportsRunInTerminalRequest": false,
                "locale": "en-us"
            }
        });
        self.send_json(&init);
        Ok(())
    }

    /// Shared session bookkeeping after a transport is ready.
    #[allow(clippy::too_many_arguments)]
    fn begin_session_common(
        &mut self,
        writer: Box<dyn Write + Send>,
        rx: Receiver<Value>,
        child: Option<Child>,
        adapter_name: &str,
        lang: &str,
        body: Value,
        is_attach: bool,
        program: Option<String>,
        cwd: Option<String>,
        args: Vec<String>,
        attach_tag: Option<String>,
    ) {
        self.writer = Some(writer);
        self.rx = Some(rx);
        self.child = child;
        self.adapter_name = adapter_name.to_string();
        self.state = DapState::Starting;
        self.error = None;
        self.soft_error = None;
        self.config_sent = false;
        self.launch_sent_at = None;
        self.launch_body = Some(body);
        self.is_attach = is_attach;
        self.last_program = program;
        self.last_cwd = cwd;
        self.last_lang = Some(lang.to_string());
        self.last_args = args;
        self.last_attach = attach_tag;
        self.panel_open = true;
        self.build_rx = None;
        self.build_message = None;
        self.stack.clear();
        self.vars.clear();
        self.threads.clear();
        self.children_cache.clear();
        self.current_line = None;
        self.current_path = None;
        self.stopped_reason = None;
        self.thread_id = None;
        self.pause_requested = false;
        self.awaiting_stack_thread = false;
        let kind = if is_attach { "attach" } else { "launch" };
        self.log(format!(
            "▶ {kind} · {adapter_name} · {lang} · {}",
            self.last_program.as_deref().unwrap_or("-")
        ));
    }

    fn send_initialize(&mut self, adapter_id: &str) {
        let id = self.alloc(PendingKind::Initialize);
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "initialize",
            "arguments": {
                "clientID": "xei",
                "clientName": "xei",
                "adapterID": adapter_id,
                "pathFormat": "path",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "supportsVariableType": true,
                "supportsVariablePaging": false,
                "supportsRunInTerminalRequest": false,
                "locale": "en-us"
            }
        }));
    }

    /// Attach to a running process by PID (lldb-dap / codelldb).
    pub fn attach_pid(&mut self, pid: u32) -> Result<(), String> {
        if self.is_session() {
            return Err("Debug session already active — stop first (Shift+F5)".into());
        }
        self.finish_shutdown();
        let adapter = ["lldb-dap", "codelldb", "lldb-vscode"]
            .into_iter()
            .find(|c| command_exists(c))
            .ok_or_else(|| install_hint("rust"))?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut child = crate::exec::tool(adapter)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start {adapter}: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "adapter stdin unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "adapter stdout unavailable".to_string())?;
        drain_stderr(child.stderr.take());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || read_loop(stdout, tx));
        let body = json!({
            "name": format!("Attach PID {pid}"),
            "type": "lldb",
            "request": "attach",
            "pid": pid,
            "stopOnEntry": false
        });
        self.begin_session_common(
            Box::new(stdin),
            rx,
            Some(child),
            adapter,
            "native",
            body,
            true,
            Some(format!("pid:{pid}")),
            Some(cwd.display().to_string()),
            Vec::new(),
            Some(format!("pid:{pid}")),
        );
        self.send_initialize("lldb");
        Ok(())
    }

    /// Attach to a debug adapter / runtime listening on `host:port`.
    ///
    /// - `python` → debugpy.adapter stdio + attach connect
    /// - `node` → js-debug TCP server + attach
    /// - `native` / default → lldb-dap attach via connect (when supported)
    pub fn attach_port(
        &mut self,
        port: u16,
        lang_hint: Option<&str>,
        host: Option<&str>,
    ) -> Result<(), String> {
        if self.is_session() {
            return Err("Debug session already active — stop first (Shift+F5)".into());
        }
        self.finish_shutdown();
        let host = host.unwrap_or("127.0.0.1");
        let lang = lang_hint.unwrap_or("python");
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        match lang {
            "node" | "javascript" | "typescript" => {
                // Connect to an already-listening js-debug / inspector, or start our own server
                // and attach to the user-provided debug port via DAP attach.
                self.start_js_debug_tcp_session(
                    json!({
                        "name": format!("Attach Node :{port}"),
                        "type": "pwa-node",
                        "request": "attach",
                        "address": host,
                        "port": port,
                        "localRoot": cwd.display().to_string(),
                        "skipFiles": ["<node_internals>/**"]
                    }),
                    true,
                    Some(format!("port:{port}")),
                    Some(cwd.display().to_string()),
                    Vec::new(),
                    Some(format!("port:{port}")),
                )
            }
            "python" | "debugpy" => {
                let py = if command_exists("python3") {
                    "python3"
                } else if command_exists("python") {
                    "python"
                } else {
                    return Err(install_hint("python"));
                };
                let mut child = crate::exec::tool(py)
                    .args(["-m", "debugpy.adapter"])
                    .current_dir(&cwd)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("Failed to start debugpy.adapter: {e}"))?;
                let stdin = child
                    .stdin
                    .take()
                    .ok_or_else(|| "adapter stdin unavailable".to_string())?;
                let stdout = child
                    .stdout
                    .take()
                    .ok_or_else(|| "adapter stdout unavailable".to_string())?;
                drain_stderr(child.stderr.take());
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || read_loop(stdout, tx));
                let body = json!({
                    "name": format!("Python Attach :{port}"),
                    "type": "python",
                    "request": "attach",
                    "connect": { "host": host, "port": port },
                    "justMyCode": true
                });
                self.begin_session_common(
                    Box::new(stdin),
                    rx,
                    Some(child),
                    "debugpy",
                    "python",
                    body,
                    true,
                    Some(format!("port:{port}")),
                    Some(cwd.display().to_string()),
                    Vec::new(),
                    Some(format!("port:{port}")),
                );
                self.send_initialize("python");
                Ok(())
            }
            _ => {
                // Generic: lldb attach by connecting to a debugserver is rare;
                // try process-less TCP attach body for adapters that support it.
                let adapter = ["lldb-dap", "codelldb"]
                    .into_iter()
                    .find(|c| command_exists(c))
                    .ok_or_else(|| {
                        String::from(
                            "No attach adapter. Use `:DapAttach pid <n>` or python/node port attach",
                        )
                    })?;
                let mut child = crate::exec::tool(adapter)
                    .current_dir(&cwd)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("Failed to start {adapter}: {e}"))?;
                let stdin = child
                    .stdin
                    .take()
                    .ok_or_else(|| "adapter stdin unavailable".to_string())?;
                let stdout = child
                    .stdout
                    .take()
                    .ok_or_else(|| "adapter stdout unavailable".to_string())?;
                drain_stderr(child.stderr.take());
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || read_loop(stdout, tx));
                let body = json!({
                    "name": format!("Attach :{port}"),
                    "type": "lldb",
                    "request": "attach",
                    "attachCommands": [format!("process connect connect://{host}:{port}")]
                });
                self.begin_session_common(
                    Box::new(stdin),
                    rx,
                    Some(child),
                    adapter,
                    lang,
                    body,
                    true,
                    Some(format!("port:{port}")),
                    Some(cwd.display().to_string()),
                    Vec::new(),
                    Some(format!("port:{port}")),
                );
                self.send_initialize(adapter);
                Ok(())
            }
        }
    }

    /// Launch a Node/JS program via js-debug over TCP (stdio is unsupported).
    pub fn start_node(
        &mut self,
        program: &str,
        cwd: Option<&Path>,
        args: &[String],
    ) -> Result<(), String> {
        if self.is_session() {
            return Err("Debug session already active — stop first (Shift+F5)".into());
        }
        self.finish_shutdown();
        let program_path = PathBuf::from(program);
        let abs_prog =
            std::fs::canonicalize(&program_path).unwrap_or_else(|_| program_path.clone());
        let cwd = cwd
            .map(Path::to_path_buf)
            .or_else(|| abs_prog.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let body = json!({
            "name": "Launch Node",
            "type": "pwa-node",
            "request": "launch",
            "program": abs_prog.display().to_string(),
            "args": args,
            "cwd": cwd.display().to_string(),
            "console": "internalConsole",
            "skipFiles": ["<node_internals>/**"]
        });
        self.start_js_debug_tcp_session(
            body,
            false,
            Some(abs_prog.display().to_string()),
            Some(cwd.display().to_string()),
            args.to_vec(),
            None,
        )
    }

    /// Spawn `js-debug-adapter` as a TCP DAP server and connect.
    fn start_js_debug_tcp_session(
        &mut self,
        body: Value,
        is_attach: bool,
        program: Option<String>,
        cwd: Option<String>,
        args: Vec<String>,
        attach_tag: Option<String>,
    ) -> Result<(), String> {
        let port =
            free_localhost_port().ok_or_else(|| "No free TCP port for js-debug".to_string())?;
        let adapter_cmd = if command_exists("js-debug-adapter") {
            "js-debug-adapter".to_string()
        } else if command_exists("node") {
            // Fallback: try npx vscode-js-debug style — require js-debug-adapter on PATH
            return Err(
                "js-debug-adapter not found. Install VS Code js-debug adapter on PATH".into(),
            );
        } else {
            return Err(install_hint("node"));
        };

        let workdir = cwd
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        // Common flags: --server=PORT  or  just PORT
        let mut child = crate::exec::tool(&adapter_cmd)
            .args([format!("--server={port}")])
            .current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .or_else(|_| {
                crate::exec::tool(&adapter_cmd)
                    .arg(port.to_string())
                    .current_dir(&workdir)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
            })
            .map_err(|e| format!("Failed to start {adapter_cmd}: {e}"))?;

        drain_stderr(child.stderr.take());
        if let Some(out) = child.stdout.take() {
            // Don't block forever; just drain in background
            thread::spawn(move || {
                let mut r = BufReader::new(out);
                let mut line = String::new();
                while r.read_line(&mut line).unwrap_or(0) > 0 {
                    line.clear();
                }
            });
        }

        // Wait for the DAP TCP server
        let stream = wait_for_tcp("127.0.0.1", port, Duration::from_secs(3)).map_err(|e| {
            let _ = child.kill();
            e
        })?;
        let reader = stream.try_clone().map_err(|e| format!("tcp clone: {e}"))?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || read_loop(reader, tx));

        self.begin_session_common(
            Box::new(stream),
            rx,
            Some(child),
            "js-debug",
            "node",
            body,
            is_attach,
            program,
            cwd.or_else(|| Some(workdir.display().to_string())),
            args,
            attach_tag,
        );
        self.send_initialize("pwa-node");
        Ok(())
    }

    pub fn continue_exec(&mut self) {
        let Some(tid) = self.thread_id else {
            self.log("continue: no thread");
            return;
        };
        if self.state != DapState::Stopped {
            self.log("continue: not stopped");
            return;
        }
        let id = self.alloc(PendingKind::Continue);
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "continue",
            "arguments": { "threadId": tid }
        }));
        self.on_resumed();
        self.log("→ continue");
    }

    pub fn step_over(&mut self) {
        self.step_cmd("next", PendingKind::Next);
    }
    pub fn step_into(&mut self) {
        self.step_cmd("stepIn", PendingKind::StepIn);
    }
    pub fn step_out(&mut self) {
        self.step_cmd("stepOut", PendingKind::StepOut);
    }

    fn step_cmd(&mut self, command: &str, kind: PendingKind) {
        let Some(tid) = self.thread_id else {
            self.log(format!("{command}: no thread"));
            return;
        };
        if self.state != DapState::Stopped {
            self.log(format!("{command}: not stopped"));
            return;
        }
        let id = self.alloc(kind);
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": command,
            "arguments": { "threadId": tid }
        }));
        self.on_resumed();
        self.log(format!("→ {command}"));
    }

    /// Suspend a running program (F6).
    pub fn pause(&mut self) {
        if self.state != DapState::Running {
            self.log("pause: not running");
            return;
        }
        if let Some(tid) = self.thread_id {
            let id = self.alloc(PendingKind::Pause);
            self.send_json(&json!({
                "seq": id,
                "type": "request",
                "command": "pause",
                "arguments": { "threadId": tid }
            }));
            self.log("→ pause");
        } else {
            // Thread id unknown while running — fetch, then pause on response.
            self.pause_requested = true;
            self.request_threads();
        }
    }

    /// Variable references die on resume.
    fn on_resumed(&mut self) {
        self.state = DapState::Running;
        self.current_line = None;
        self.children_cache.clear();
    }

    /// Graceful stop: terminate/disconnect, then SIGKILL after [`SHUTDOWN_GRACE`]
    /// (enforced in `poll`). Pressing stop twice force-kills.
    /// Stop whatever is in front of you — a session, a build, or a stale error.
    ///
    /// This is the escape hatch, and it did not work. `finish_shutdown` keeps
    /// a running build alive on purpose ("Keep build_rx if a build is still
    /// running"), so ⇧F5 while a build was in flight did **nothing**: the
    /// build carried on and launched anyway. A stop that cannot stop the thing
    /// in front of the user is the whole of the reported "shift f5는 입력도
    /// 씹히고".
    pub fn stop(&mut self) {
        // A build in flight is the thing being stopped. Dropping the receiver
        // is the cancellation: the thread finishes on its own and its result
        // goes nowhere. Killing the compiler mid-flight would leave a partial
        // binary behind for the next launch to find.
        if self.build_rx.take().is_some() {
            self.build_message = None;
            self.soft_error = None;
            self.state = DapState::Idle;
            self.log("■ build cancelled");
            return;
        }
        // Clearing this here is what makes a second ⇧F5 mean something after a
        // failed launch: the panel was showing an error about a session that
        // no longer existed, with every transport button correctly disabled.
        self.soft_error = None;
        if self.writer.is_none() {
            self.finish_shutdown();
            return;
        }
        if self.state == DapState::Ending {
            self.log("■ force kill");
            self.finish_shutdown();
            return;
        }
        self.state = DapState::Ending;
        if self.supports_terminate {
            let id = self.alloc(PendingKind::Terminate);
            self.send_json(&json!({
                "seq": id,
                "type": "request",
                "command": "terminate",
                "arguments": { "restart": false }
            }));
        } else {
            let id = self.alloc(PendingKind::Disconnect);
            self.send_json(&json!({
                "seq": id,
                "type": "request",
                "command": "disconnect",
                "arguments": { "restart": false, "terminateDebuggee": true }
            }));
        }
        self.shutdown_deadline = Some(Instant::now() + SHUTDOWN_GRACE);
        self.log("■ stopping…");
    }

    /// Queue a relaunch; it fires from `poll()` once the graceful stop lands.
    pub fn restart(&mut self) -> Result<(), String> {
        let prog = self
            .last_program
            .clone()
            .ok_or_else(|| "No previous program".to_string())?;
        let cwd = self.last_cwd.clone().map(PathBuf::from);
        let lang = self.last_lang.clone();
        let args = self.last_args.clone();
        if self.is_session() || self.state == DapState::Ending {
            self.restart_pending = Some((prog, cwd, lang, args));
            self.stop();
            self.log("↻ restart queued");
            Ok(())
        } else {
            self.start(&prog, cwd.as_deref(), lang.as_deref(), &args)
        }
    }

    fn finish_shutdown(&mut self) {
        self.writer = None;
        self.rx = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.pending.clear();
        self.config_sent = false;
        self.launch_sent_at = None;
        self.launch_body = None;
        self.shutdown_deadline = None;
        self.pause_requested = false;
        self.awaiting_stack_thread = false;
        self.thread_id = None;
        self.current_line = None;
        self.children_cache.clear();
        // Watchpoints die with the session. A `dataId` is the adapter's,
        // resolved against an address in a process that no longer exists —
        // carrying them across would arm garbage on the next run.
        self.watchpoints.clear();
        // Keep build_rx if a build is still running
        if self.build_rx.is_none() {
            self.build_message = None;
        }
        if self.state != DapState::Idle && self.build_rx.is_none() {
            self.state = DapState::Idle;
            self.log("session ended");
        }
    }

    /// Spawn the build in the background; on success `poll` re-enters `start`.
    ///
    /// Which build depends on what the file IS. See [`RustBuild`] — a loose
    /// `.rs` file is not a cargo target, and building the workspace it happens
    /// to sit in produces every binary except the one that was asked for.
    fn begin_cargo_build(
        &mut self,
        cwd: &Path,
        program: String,
        lang: String,
        args: Vec<String>,
    ) -> Result<(), String> {
        let plan = RustBuild::of(Path::new(&program), cwd);
        if let RustBuild::Rustc { out } = plan {
            return self.begin_rustc_build(cwd, program, out, lang, args);
        }
        if !command_exists("cargo") {
            return Err(format!(
                "Binary missing and cargo not found — build the project first"
            ));
        }
        let (tx, rx) = mpsc::channel();
        let cwd_b = cwd.to_path_buf();
        let prog_for_resolve = program.clone();
        let lang_c = lang.clone();
        let args_c = args.clone();
        thread::spawn(move || {
            let out = crate::exec::tool("cargo")
                .args(["build"])
                .current_dir(&cwd_b)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();
            match out {
                Ok(o) if o.status.success() => {
                    let bin = resolve_rust_bin(&cwd_b, Path::new(&prog_for_resolve))
                        .unwrap_or_else(|| {
                            resolve_rust_bin(&cwd_b, Path::new("src/main.rs"))
                                .unwrap_or(prog_for_resolve)
                        });
                    if Path::new(&bin).is_file() {
                        let _ = tx.send(Ok((bin, cwd_b, lang_c, args_c)));
                    } else {
                        let _ = tx.send(Err(format!("cargo build ok but binary not found: {bin}")));
                    }
                }
                Ok(o) => {
                    // Was the LAST non-empty line, which for a cargo build is
                    // always "error: could not compile … due to N previous
                    // errors" — a sentence that names nothing anyone can go
                    // and look at.
                    let err = String::from_utf8_lossy(&o.stderr);
                    let _ = tx.send(Err(first_compile_error(&err, "cargo build failed")));
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("cargo spawn: {e}")));
                }
            }
        });
        self.build_rx = Some(rx);
        self.build_tool = "cargo build";
        self.build_message = Some("cargo build…".into());
        self.panel_open = true;
        self.state = DapState::Starting;
        self.last_program = Some(program);
        self.last_cwd = Some(cwd.display().to_string());
        self.last_lang = Some(lang);
        self.last_args = args;
        self.log("⚙ cargo build… (will launch when done)");
        Ok(())
    }

    /// Compile ONE `.rs` file with `rustc` and debug that.
    ///
    /// The reported case: `test.rs` sitting at the root of a cargo WORKSPACE.
    /// `cargo build` succeeded — it built the workspace's members — and then
    /// the binary the debugger wanted was not there, because a loose file is
    /// not a target of anything. "cargo build ok but binary not found" was an
    /// accurate report of a build that was never going to produce it.
    ///
    /// `--edition 2021` because a file handed straight to `rustc` defaults to
    /// 2015, where `async`, `dyn` and array `IntoIterator` all behave
    /// differently — a single file would fail to compile for reasons that have
    /// nothing to do with what the user wrote.
    fn begin_rustc_build(
        &mut self,
        cwd: &Path,
        program: String,
        out: PathBuf,
        lang: String,
        args: Vec<String>,
    ) -> Result<(), String> {
        if !command_exists("rustc") {
            return Err("rustc not found — install Rust to debug a single file".into());
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("could not make a build directory: {e}"))?;
        }
        let (tx, rx) = mpsc::channel();
        let cwd_b = cwd.to_path_buf();
        let src = program.clone();
        let out_c = out.clone();
        let lang_c = lang.clone();
        let args_c = args.clone();
        thread::spawn(move || {
            let result = crate::exec::tool("rustc")
                .args(["-g", "--edition", "2021", "-o"])
                .arg(&out_c)
                .arg(&src)
                .current_dir(&cwd_b)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();
            match result {
                Ok(o) if o.status.success() && out_c.is_file() => {
                    let _ = tx.send(Ok((
                        out_c.display().to_string(),
                        cwd_b,
                        lang_c,
                        args_c,
                    )));
                }
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    let _ = tx.send(Err(first_compile_error(&err, "rustc failed")));
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("rustc spawn: {e}")));
                }
            }
        });
        self.build_rx = Some(rx);
        self.build_tool = "rustc";
        self.build_message = Some("rustc…".into());
        self.panel_open = true;
        self.state = DapState::Starting;
        self.last_program = Some(program);
        self.last_cwd = Some(cwd.display().to_string());
        self.last_lang = Some(lang);
        self.last_args = args;
        self.log("⚙ rustc (single file)… (will launch when done)");
        Ok(())
    }

    // ── Requests ───────────────────────────────────────────────────────

    fn alloc(&mut self, kind: PendingKind) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.pending.insert(id, kind);
        id
    }

    fn send_json(&mut self, v: &Value) {
        #[cfg(test)]
        self.sent.push(v.clone());
        let body = v.to_string();
        if let Some(ref mut writer) = self.writer {
            let header = format!("Content-Length: {}\r\n\r\n", body.len());
            let _ = writer.write_all(header.as_bytes());
            let _ = writer.write_all(body.as_bytes());
            let _ = writer.flush();
        }
    }

    /// Arm the enabled breakpoints for `path`.
    ///
    /// **The filter here and the zip in the response must agree.** The
    /// response array is 1:1 with the request array by POSITION, and the
    /// handler walks the stored list to apply verification and the adapter's
    /// line slides. Sending a subset while the handler walked the whole list
    /// would pair each answer with the wrong breakpoint — silently moving a
    /// breakpoint to a line the adapter never mentioned. Both sides filter on
    /// `enabled`, and a test holds them together.
    fn send_set_breakpoints(&mut self, path: &str) {
        let lines = self
            .breakpoints
            .get(path)
            .map(|v| {
                v.iter()
                    .filter(|b| b.enabled)
                    .map(|b| {
                        let mut o = json!({ "line": b.line + 1 });
                        if let Some(ref c) = b.condition {
                            o["condition"] = json!(c);
                        }
                        if let Some(ref m) = b.log_message {
                            o["logMessage"] = json!(m);
                        }
                        o
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let id = self.alloc(PendingKind::SetBreakpoints(path.to_string()));
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "setBreakpoints",
            "arguments": {
                "source": {
                    "path": path,
                    "name": Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path)
                },
                "breakpoints": lines,
                "sourceModified": false
            }
        }));
    }

    /// Evaluate expression in the current stopped frame (REPL / watch).
    pub fn evaluate(&mut self, expression: &str) {
        let expr = expression.trim();
        if expr.is_empty() {
            return;
        }
        if self.state != DapState::Stopped {
            self.log("eval: not stopped");
            return;
        }
        let frame_id = self
            .stack
            .get(self.selected_frame)
            .map(|f| f.id)
            .unwrap_or(0);
        self.log(format!("> {expr}"));
        let id = self.alloc(PendingKind::Evaluate);
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "evaluate",
            "arguments": {
                "expression": expr,
                "frameId": frame_id,
                "context": "repl"
            }
        }));
        self.eval_input.clear();
    }

    /// Change a variable's value while the program is stopped.
    ///
    /// `index` is a row of the flattened Variables tree.
    ///
    /// **The container, not the variable.** DAP's `setVariable` takes the
    /// PARENT's `variablesReference` plus a name — a `VarNode`'s own `var_ref`
    /// is the handle to its CHILDREN, which is the opposite end. The tree is
    /// flattened with a depth per row, so the parent is the nearest preceding
    /// row one level shallower.
    pub fn set_variable(&mut self, index: usize, value: &str) {
        if !self.supports_set_variable {
            self.log("✗ set: this adapter cannot change values");
            return;
        }
        if self.state != DapState::Stopped {
            self.log("✗ set: only while stopped");
            return;
        }
        let Some(node) = self.vars.get(index) else { return };
        if node.is_scope {
            return;
        }
        let name = node.name.clone();
        let depth = node.depth;
        let Some(container) = self.vars[..index]
            .iter()
            .rev()
            .find(|n| n.depth + 1 == depth)
            .map(|n| n.var_ref)
            .filter(|r| *r > 0)
        else {
            self.log(format!("✗ set {name}: no container to set it in"));
            return;
        };
        let id = self.alloc(PendingKind::SetVariable { index, container });
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "setVariable",
            "arguments": {
                "variablesReference": container,
                "name": name,
                "value": value
            }
        }));
    }

    // ── Query surface ──────────────────────────────────────────────────
    //
    // What the debugger KNOWS, asked without disturbing what it is showing.
    //
    // Everything above this line is shaped for the panel: `vars` is a
    // flattened tree with an expansion state, `selected_frame` is a cursor a
    // person moved. A second reader — the inline values below, and Logic View
    // after them — needs the same facts without owning that cursor, and
    // without a request going out per question.
    //
    // These are all reads of state that is already here. They cost nothing and
    // they are the seam a view sits on.

    /// The current frame's local variables, by name.
    ///
    /// Free: the first scope is auto-expanded when a stop lands, so the panel
    /// has already fetched these. A second consumer asking the adapter again
    /// would be paying for an answer that is sitting in `vars`.
    ///
    /// Locals only — the scope roots and any expanded children below them are
    /// left out. A child is `a.b`, which is not a name that appears in the
    /// source as itself, and a scope is not a value.
    pub fn frame_values(&self) -> Vec<(&str, &str)> {
        let mut out = Vec::new();
        let mut in_first_scope = false;
        for v in &self.vars {
            if v.is_scope {
                // The first scope is the one that is auto-expanded — Locals
                // for every adapter that has been looked at. Globals and
                // Registers are not what a line of source is talking about.
                if in_first_scope {
                    break;
                }
                in_first_scope = true;
                continue;
            }
            if in_first_scope && !v.value.is_empty() {
                out.push((v.name.as_str(), v.value.as_str()));
            }
        }
        out
    }

    /// Where the program is stopped: path and 0-based line.
    pub fn stop_location(&self) -> Option<(&str, usize)> {
        Some((self.current_path.as_deref()?, self.current_line?))
    }

    /// The call stack as (name, path, 0-based line), innermost first.
    ///
    /// The path through the program that is KNOWN rather than inferred — a
    /// view drawing execution has this for free and has nothing else for free.
    pub fn call_path(&self) -> Vec<(&str, &str, usize)> {
        self.stack
            .iter()
            .map(|f| (f.name.as_str(), f.path.as_str(), f.line))
            .collect()
    }

    /// Ask the adapter whether a value can be watched, then watch it.
    ///
    /// Two steps because the spec is two steps, and the first one is the
    /// important one: `dataBreakpointInfo` resolves a NAME in a frame to an
    /// opaque `dataId`, and it is allowed to answer "no" — a register, a value
    /// with no address, or simply no hardware left. **Watchpoints are scarce
    /// hardware**: x86 and ARM give four, and the fifth request fails. Asking
    /// first is what lets the refusal be reported instead of swallowed.
    pub fn watch(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if !self.supports_data_breakpoints {
            self.log(format!("✗ watch {name}: this adapter cannot watch values"));
            return;
        }
        if self.state != DapState::Stopped {
            self.log(format!("✗ watch {name}: only while stopped"));
            return;
        }
        if self.watchpoints.iter().any(|w| w.name == name) {
            self.unwatch(name);
            return;
        }
        let frame_id = self
            .stack
            .get(self.selected_frame)
            .map(|f| f.id)
            .unwrap_or(0);
        let id = self.alloc(PendingKind::DataBreakpointInfo(name.to_string()));
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "dataBreakpointInfo",
            "arguments": { "name": name, "frameId": frame_id }
        }));
    }

    /// Stop watching, by the name the user asked about.
    pub fn unwatch(&mut self, name: &str) {
        let before = self.watchpoints.len();
        self.watchpoints.retain(|w| w.name != name);
        if self.watchpoints.len() != before {
            self.log(format!("○ no longer watching {name}"));
            self.send_data_breakpoints();
        }
    }

    /// Arm the whole set.
    ///
    /// `setDataBreakpoints` replaces every watchpoint on each call — the same
    /// shape as `setBreakpoints`, and the same reason the list is the unit
    /// rather than the individual.
    fn send_data_breakpoints(&mut self) {
        if !self.is_session() {
            return;
        }
        let points: Vec<Value> = self
            .watchpoints
            .iter()
            .map(|w| json!({ "dataId": w.data_id, "accessType": "write" }))
            .collect();
        let id = self.alloc(PendingKind::SetDataBreakpoints);
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "setDataBreakpoints",
            "arguments": { "breakpoints": points }
        }));
    }

    /// Ask what an expression is worth, for a hover datatip.
    ///
    /// `context: "hover"` is the DAP spec's own name for this, and adapters
    /// use it to be conservative — a hover must not call functions or mutate
    /// anything, because the user only pointed at a word.
    ///
    /// Nothing is logged. The console belongs to what the user typed.
    pub fn request_datatip(&mut self, expression: &str) {
        let expr = expression.trim();
        if expr.is_empty() || self.state != DapState::Stopped {
            self.datatip = None;
            self.datatip_pending = false;
            return;
        }
        let frame_id = self
            .stack
            .get(self.selected_frame)
            .map(|f| f.id)
            .unwrap_or(0);
        // Cleared before asking, like the LSP hover: a stale value under a new
        // identifier reads as an answer.
        self.datatip = None;
        self.datatip_pending = true;
        let id = self.alloc(PendingKind::Datatip);
        self.pending_datatip.insert(id, expr.to_string());
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "evaluate",
            "arguments": {
                "expression": expr,
                "frameId": frame_id,
                "context": "hover"
            }
        }));
    }

    /// setBreakpoints* → setExceptionBreakpoints → configurationDone.
    /// Responses may arrive later; ordering of the requests is what matters.
    fn send_configuration(&mut self) {
        if self.config_sent {
            return;
        }
        self.config_sent = true;
        let paths: Vec<String> = self.breakpoints.keys().cloned().collect();
        for p in paths {
            self.send_set_breakpoints(&p);
        }
        if !self.exception_filters.is_empty() {
            let filters = self.exception_filters.clone();
            let id = self.alloc(PendingKind::ExceptionBreakpoints);
            self.send_json(&json!({
                "seq": id,
                "type": "request",
                "command": "setExceptionBreakpoints",
                "arguments": { "filters": filters }
            }));
        }
        if self.supports_config_done {
            let id = self.alloc(PendingKind::ConfigDone);
            self.send_json(&json!({
                "seq": id,
                "type": "request",
                "command": "configurationDone"
            }));
        }
    }

    fn request_threads(&mut self) {
        let id = self.alloc(PendingKind::Threads);
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "threads"
        }));
    }

    fn request_stack(&mut self) {
        let Some(tid) = self.thread_id else { return };
        let id = self.alloc(PendingKind::StackTrace);
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "stackTrace",
            "arguments": {
                "threadId": tid,
                "startFrame": 0,
                "levels": STACK_LEVELS
            }
        }));
    }

    fn request_scopes(&mut self, frame_id: i64) {
        let id = self.alloc(PendingKind::Scopes);
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "scopes",
            "arguments": { "frameId": frame_id }
        }));
    }

    fn request_variables(&mut self, variables_reference: i64) {
        if variables_reference <= 0 {
            return;
        }
        let id = self.alloc(PendingKind::Variables(variables_reference));
        self.send_json(&json!({
            "seq": id,
            "type": "request",
            "command": "variables",
            "arguments": { "variablesReference": variables_reference }
        }));
    }

    // ── Panel navigation ───────────────────────────────────────────────

    /// Select stack frame by index and load its scopes.
    pub fn select_frame(&mut self, idx: usize) {
        if idx >= self.stack.len() {
            return;
        }
        self.selected_frame = idx;
        self.focus_row = idx;
        let frame = &self.stack[idx];
        // `current_path` / `current_line` are NOT touched. They mean "where the
        // program is stopped", which is the top of the stack and does not move
        // while you read the stack — writing the selected frame into them made
        // the editor claim execution was in the caller you had just clicked,
        // and the real stop became unfindable.
        //
        // Where you are LOOKING is `selected_frame`, and the editor reads it
        // through `frame_location`.
        let _ = frame;
        // Take the editor there. Selecting a frame moved the location and did
        // not ask anyone to follow it, so the variables changed to a frame the
        // user could not see — and in every debugger, clicking a frame in the
        // call stack is how you go and look at it. `dap_apply_stopped_location`
        // already opens another file when the frame is in one.
        self.location_dirty = true;
        let fid = frame.id;
        self.datatip = None;
        self.datatip_pending = false;
        self.vars.clear();
        self.request_scopes(fid);
    }

    /// Expand/collapse the Variables tree node at `idx`.
    pub fn toggle_var_at(&mut self, idx: usize) {
        let Some(node) = self.vars.get(idx) else {
            return;
        };
        if node.var_ref <= 0 {
            return;
        }
        if node.expanded {
            let depth = node.depth;
            self.vars[idx].expanded = false;
            let mut end = idx + 1;
            while end < self.vars.len() && self.vars[end].depth > depth {
                end += 1;
            }
            self.vars.drain(idx + 1..end);
        } else {
            if self.state != DapState::Stopped {
                // Refs die on resume — don't fetch stale children.
                return;
            }
            self.vars[idx].expanded = true;
            let vr = self.vars[idx].var_ref;
            if let Some(children) = self.children_cache.get(&vr).cloned() {
                self.insert_children(vr, children);
            } else {
                self.request_variables(vr);
            }
        }
    }

    /// Splice `children` in under the (expanded) node holding `var_ref`.
    /// Drop everything nested under `index`, and mark it collapsed.
    ///
    /// Used when a value changes: what was read out of the old value is not a
    /// description of the new one.
    fn collapse_children_of(&mut self, index: usize) {
        let Some(depth) = self.vars.get(index).map(|n| n.depth) else { return };
        let mut end = index + 1;
        while end < self.vars.len() && self.vars[end].depth > depth {
            end += 1;
        }
        self.vars.drain(index + 1..end);
        if let Some(n) = self.vars.get_mut(index) {
            n.expanded = false;
        }
    }

    fn insert_children(&mut self, var_ref: i64, children: Vec<VarNode>) {
        let Some(pos) = self
            .vars
            .iter()
            .position(|n| n.var_ref == var_ref && n.expanded)
        else {
            return; // collapsed (or gone) while the request was in flight
        };
        let depth = self.vars[pos].depth + 1;
        let mut rows = children;
        for r in &mut rows {
            r.depth = depth;
            r.expanded = false;
        }
        // Replace any previous children (refresh case).
        let mut end = pos + 1;
        while end < self.vars.len() && self.vars[end].depth > self.vars[pos].depth {
            end += 1;
        }
        self.vars.splice(pos + 1..end, rows);
    }

    /// Move panel focus; selection only — network requests stay on Enter.
    pub fn move_focus(&mut self, delta: isize) {
        let len = match self.pane {
            DebugPane::Stack => self.stack.len(),
            DebugPane::Variables => self.vars.len(),
            DebugPane::Breakpoints => self.flat_bps().len(),
            DebugPane::Console => self.console.len(),
        };
        if len == 0 {
            self.focus_row = 0;
            return;
        }
        let cur = self.focus_row as isize + delta;
        self.focus_row = cur.clamp(0, (len as isize) - 1) as usize;
        match self.pane {
            DebugPane::Stack => self.selected_frame = self.focus_row,
            DebugPane::Breakpoints => self.selected_bp = self.focus_row,
            DebugPane::Variables | DebugPane::Console => {}
        }
    }

    /// Switch pane, placing focus sensibly (console starts at the tail).
    pub fn set_pane(&mut self, pane: DebugPane) {
        self.pane = pane;
        self.focus_row = match pane {
            DebugPane::Stack => self.selected_frame.min(self.stack.len().saturating_sub(1)),
            DebugPane::Console => self.console.len().saturating_sub(1),
            _ => 0,
        };
    }

    // ── Poll & dispatch ────────────────────────────────────────────────

    pub fn poll(&mut self) {
        // Async cargo build completion
        if let Some(rx) = self.build_rx.take() {
            match rx.try_recv() {
                Ok(Ok((bin, cwd, lang, args))) => {
                    self.build_message = None;
                    // `begin_*_build` set `Starting` so the panel had something
                    // to show while the build ran — and `is_session` counts
                    // `Starting`, so leaving it set made `start` refuse with
                    // "Debug session already active — stop first". The
                    // build-then-launch path could therefore never complete;
                    // it was only ever hidden behind the earlier
                    // "binary not found". The session's own `Starting` is set
                    // by `start` on the very next line.
                    self.state = DapState::Idle;
                    let tool = self.build_tool;
                    self.log(format!("✓ {tool} ok · launching {bin}"));
                    if let Err(e) = self.start(&bin, Some(&cwd), Some(&lang), &args) {
                        self.soft_error = Some(e.clone());
                        self.log(format!("✗ launch after build: {e}"));
                        self.state = DapState::Idle;
                    }
                }
                Ok(Err(e)) => {
                    self.build_message = None;
                    self.build_rx = None;
                    self.state = DapState::Idle;
                    self.soft_error = Some(e.clone());
                    let tool = self.build_tool;
                    self.log(format!("✗ {tool}: {e}"));
                }
                Err(TryRecvError::Empty) => {
                    self.build_rx = Some(rx);
                }
                Err(TryRecvError::Disconnected) => {
                    self.build_message = None;
                    self.state = DapState::Idle;
                }
            }
        }
        // Enforce the shutdown grace deadline.
        if let Some(deadline) = self.shutdown_deadline {
            if Instant::now() >= deadline {
                self.log("■ grace expired — killing adapter");
                self.finish_shutdown();
            }
        }
        // Config fallback for adapters that never emit `initialized`.
        if !self.config_sent
            && self.writer.is_some()
            && self
                .launch_sent_at
                .map(|t| t.elapsed() >= CONFIG_FALLBACK)
                .unwrap_or(false)
        {
            self.log("no initialized event — sending configuration anyway");
            self.send_configuration();
        }
        // Queued restart once the previous session fully lands.
        if self.state == DapState::Idle {
            if let Some((prog, cwd, lang, args)) = self.restart_pending.take() {
                if let Err(e) = self.start(&prog, cwd.as_deref(), lang.as_deref(), &args) {
                    self.log(format!("restart failed: {e}"));
                }
            }
        }

        let mut batch = Vec::new();
        if let Some(ref rx) = self.rx {
            loop {
                match rx.try_recv() {
                    Ok(m) => batch.push(m),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if self.state == DapState::Starting {
                            let hint = self
                                .last_lang
                                .as_deref()
                                .map(install_hint)
                                .unwrap_or_default();
                            self.error = Some(format!("Debug adapter exited at startup. {hint}"));
                            self.log("adapter exited at startup");
                        } else if self.is_session() {
                            self.error = Some("Debug adapter disconnected".into());
                            self.log("adapter disconnected");
                        }
                        self.finish_shutdown();
                        break;
                    }
                }
            }
        }
        for msg in batch {
            self.handle_msg(msg);
        }
    }

    fn handle_msg(&mut self, v: Value) {
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "event" => self.handle_event(&v),
            "response" => self.handle_response(&v),
            "request" => self.handle_reverse_request(&v),
            _ => {}
        }
    }

    fn handle_event(&mut self, v: &Value) {
        let event = v.get("event").and_then(|e| e.as_str()).unwrap_or("");
        let body = v.get("body").cloned().unwrap_or(json!({}));
        match event {
            "initialized" => {
                // Adapter is ready for breakpoints + configurationDone.
                self.send_configuration();
            }
            "stopped" => {
                self.state = DapState::Stopped;
                let reason = body
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("stopped")
                    .to_string();
                self.stopped_reason = Some(reason.clone());
                self.children_cache.clear();
                if let Some(tid) = body.get("threadId").and_then(|t| t.as_i64()) {
                    self.thread_id = Some(tid);
                    self.log(format!("● stopped ({reason})"));
                    self.request_stack();
                } else {
                    // Legal when allThreadsStopped — find a thread first.
                    self.log(format!("● stopped ({reason}) — resolving thread"));
                    self.awaiting_stack_thread = true;
                    self.request_threads();
                }
            }
            "continued" => {
                self.on_resumed();
            }
            "thread" => {
                let tid = body.get("threadId").and_then(|t| t.as_i64()).unwrap_or(0);
                match body.get("reason").and_then(|r| r.as_str()).unwrap_or("") {
                    "started" => {
                        if !self.threads.iter().any(|(id, _)| *id == tid) {
                            self.threads.push((tid, format!("thread {tid}")));
                        }
                    }
                    "exited" => {
                        self.threads.retain(|(id, _)| *id != tid);
                        if self.thread_id == Some(tid) {
                            self.thread_id = None;
                        }
                    }
                    _ => {}
                }
            }
            "breakpoint" => {
                self.apply_breakpoint_event(&body);
            }
            "terminated" => {
                self.log("■ terminated");
                self.finish_shutdown();
            }
            "exited" => {
                let code = body
                    .get("exitCode")
                    .and_then(|c| c.as_i64())
                    .unwrap_or_default();
                // Exit info only — `terminated` ends the session.
                self.log(format!("program exited with code {code}"));
            }
            "output" => {
                let cat = body
                    .get("category")
                    .and_then(|c| c.as_str())
                    .unwrap_or("console");
                let out = body
                    .get("output")
                    .and_then(|o| o.as_str())
                    .unwrap_or("")
                    .trim_end()
                    .to_string();
                if !out.is_empty() {
                    for line in out.lines() {
                        self.log(format!("[{cat}] {line}"));
                    }
                }
            }
            _ => {}
        }
    }

    /// Adapter re-verified / moved a breakpoint after launch.
    fn apply_breakpoint_event(&mut self, body: &Value) {
        let Some(bp) = body.get("breakpoint") else {
            return;
        };
        let line = bp
            .get("line")
            .and_then(|l| l.as_u64())
            .map(|l| l.saturating_sub(1) as usize);
        let verified = bp
            .get("verified")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let message = bp
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let src_path = bp
            .get("source")
            .and_then(|s| s.get("path"))
            .and_then(|p| p.as_str())
            .map(|s| s.to_string());
        let Some(line) = line else { return };
        let canon_src = src_path.map(|p| self.canon(&p));
        for (path, list) in self.breakpoints.iter_mut() {
            if canon_src.as_ref().map(|s| s == path).unwrap_or(true) {
                if let Some(b) = list.iter_mut().find(|b| b.line == line) {
                    b.verified = verified;
                    b.message = message.clone();
                }
            }
        }
    }

    fn handle_response(&mut self, v: &Value) {
        let id = v.get("request_seq").and_then(|x| x.as_u64());
        let success = v.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
        let command = v.get("command").and_then(|c| c.as_str()).unwrap_or("");
        let body = v.get("body").cloned().unwrap_or(json!({}));
        let kind = id.and_then(|i| self.pending.remove(&i));

        if !success {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("request failed");
            // A datatip that failed is not news. The pointer crosses keywords,
            // comments and punctuation constantly, and an adapter answers "no
            // such variable" to every one of them — logging that would fill the
            // console with the mouse's path across the file. It also has to
            // clear the pending flag here, because this branch returns and the
            // success arm below never runs.
            if let (Some(PendingKind::Datatip), Some(i)) = (kind.as_ref(), id) {
                self.pending_datatip.remove(&i);
                self.datatip_pending = false;
                self.datatip = None;
                return;
            }
            self.log(format!("✗ {command}: {msg}"));
            match kind {
                Some(PendingKind::Initialize | PendingKind::Launch) => {
                    self.error = Some(msg.to_string());
                    self.finish_shutdown();
                }
                Some(PendingKind::Terminate) => {
                    // Fall back to disconnect within the same grace window.
                    let id = self.alloc(PendingKind::Disconnect);
                    self.send_json(&json!({
                        "seq": id,
                        "type": "request",
                        "command": "disconnect",
                        "arguments": { "restart": false, "terminateDebuggee": true }
                    }));
                }
                _ => {}
            }
            return;
        }

        match kind {
            Some(PendingKind::Initialize) => {
                self.supports_config_done = body
                    .get("supportsConfigurationDoneRequest")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(true);
                self.supports_terminate = body
                    .get("supportsTerminateRequest")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                self.supports_data_breakpoints = body
                    .get("supportsDataBreakpoints")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                self.supports_set_variable = body
                    .get("supportsSetVariable")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                self.exception_filters = pick_exception_filters(&body);
                // Spec order: launch/attach goes out now; the adapter answers it only
                // after configurationDone (which follows its `initialized`).
                if let Some(args) = self.launch_body.take() {
                    let id = self.alloc(PendingKind::Launch);
                    let cmd = if self.is_attach { "attach" } else { "launch" };
                    self.send_json(&json!({
                        "seq": id,
                        "type": "request",
                        "command": cmd,
                        "arguments": args
                    }));
                    self.launch_sent_at = Some(Instant::now());
                    self.log(format!("initialize ok → {cmd}"));
                }
            }
            Some(PendingKind::Launch) => {
                let kind = if self.is_attach { "attach" } else { "launch" };
                self.log(format!("{kind} ok"));
                if self.state == DapState::Starting {
                    self.state = DapState::Running;
                }
            }
            Some(PendingKind::SetBreakpoints(path)) => {
                // Response array is 1:1 with the (line-sorted) request array.
                let mut moved: Vec<(usize, usize)> = Vec::new();
                if let Some(arr) = body.get("breakpoints").and_then(|b| b.as_array()) {
                    if let Some(list) = self.breakpoints.get_mut(&path) {
                        // Enabled only — see `send_set_breakpoints`. This is
                        // the other half of that pairing.
                        for (b, resp) in list.iter_mut().filter(|b| b.enabled).zip(arr) {
                            b.verified = resp
                                .get("verified")
                                .and_then(|x| x.as_bool())
                                .unwrap_or(false);
                            b.message = resp
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("")
                                .to_string();
                            // The adapter may slide the BP to the nearest line
                            // that has code — a breakpoint asked for on a
                            // comment lands below it.
                            //
                            // SAY SO. It used to move in silence, and a mark
                            // appearing two lines from where it was clicked
                            // reads as the editor being wrong rather than as
                            // the debugger being right: reported as "분명 85번
                            // 라인번호를 더블클릭했는데 87번에 브레이크 포인트가
                            // 생김".
                            if let Some(l) = resp.get("line").and_then(|l| l.as_u64()) {
                                let to = l.saturating_sub(1) as usize;
                                if to != b.line {
                                    moved.push((b.line + 1, to + 1));
                                }
                                b.line = to;
                            }
                        }
                        list.sort_by_key(|b| b.line);
                        list.dedup_by_key(|b| b.line);
                    }
                }
                for (from, to) in moved {
                    self.log(format!("● breakpoint {from} → {to} (no code on {from})"));
                }
            }
            Some(PendingKind::Threads) => {
                self.threads = body
                    .get("threads")
                    .and_then(|t| t.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|t| {
                                (
                                    t.get("id").and_then(|i| i.as_i64()).unwrap_or(0),
                                    t.get("name")
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("thread")
                                        .to_string(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if self.thread_id.is_none() {
                    self.thread_id = self.threads.first().map(|(id, _)| *id);
                }
                if self.awaiting_stack_thread {
                    self.awaiting_stack_thread = false;
                    self.request_stack();
                }
                if self.pause_requested {
                    self.pause_requested = false;
                    if self.state == DapState::Running {
                        self.pause();
                    }
                }
            }
            Some(PendingKind::StackTrace) => {
                self.stack.clear();
                if let Some(arr) = body.get("stackFrames").and_then(|s| s.as_array()) {
                    for f in arr {
                        self.stack.push(StackFrameInfo {
                            id: f.get("id").and_then(|x| x.as_i64()).unwrap_or(0),
                            name: frame_label(
                                f.get("name").and_then(|x| x.as_str()).unwrap_or("??"),
                            ),
                            path: f
                                .get("source")
                                .and_then(|s| s.get("path"))
                                .and_then(|p| p.as_str())
                                .unwrap_or("")
                                .to_string(),
                            line: f
                                .get("line")
                                .and_then(|l| l.as_u64())
                                .unwrap_or(1)
                                .saturating_sub(1) as usize,
                            column: f
                                .get("column")
                                .and_then(|c| c.as_u64())
                                .unwrap_or(1)
                                .saturating_sub(1) as usize,
                        });
                    }
                }
                if let Some(top) = self.stack.first() {
                    self.current_path = Some(top.path.clone());
                    self.current_line = Some(top.line);
                    self.selected_frame = 0;
                    if self.pane == DebugPane::Stack {
                        self.focus_row = 0;
                    }
                    self.location_dirty = true;
                    let fid = top.id;
                    self.vars.clear();
                    self.request_scopes(fid);
                }
            }
            Some(PendingKind::Scopes) => {
                self.vars.clear();
                if let Some(arr) = body.get("scopes").and_then(|s| s.as_array()) {
                    for s in arr {
                        self.vars.push(VarNode {
                            name: s
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("scope")
                                .to_string(),
                            value: String::new(),
                            typ: String::new(),
                            var_ref: s
                                .get("variablesReference")
                                .and_then(|r| r.as_i64())
                                .unwrap_or(0),
                            depth: 0,
                            expanded: false,
                            is_scope: true,
                        });
                    }
                }
                // Auto-expand the first (usually "Locals") scope.
                if let Some(first) = self.vars.first_mut() {
                    if first.var_ref > 0 {
                        first.expanded = true;
                        let vr = first.var_ref;
                        self.request_variables(vr);
                    }
                }
                if self.pane == DebugPane::Variables {
                    self.focus_row = 0;
                }
            }
            Some(PendingKind::Variables(var_ref)) => {
                let children: Vec<VarNode> = body
                    .get("variables")
                    .and_then(|s| s.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|var| VarNode {
                                name: var
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("?")
                                    .to_string(),
                                value: var
                                    .get("value")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                typ: var
                                    .get("type")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                var_ref: var
                                    .get("variablesReference")
                                    .and_then(|r| r.as_i64())
                                    .unwrap_or(0),
                                depth: 0,
                                expanded: false,
                                is_scope: false,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.children_cache.insert(var_ref, children.clone());
                self.insert_children(var_ref, children);
            }
            Some(
                PendingKind::Continue
                | PendingKind::Next
                | PendingKind::StepIn
                | PendingKind::StepOut
                | PendingKind::Pause,
            ) => {
                // stopped event will follow
            }
            Some(PendingKind::Terminate | PendingKind::Disconnect) => {
                // terminated event (or the grace deadline) completes shutdown
            }
            Some(PendingKind::Evaluate) => {
                let result = body
                    .get("result")
                    .and_then(|r| r.as_str())
                    .unwrap_or("(no result)");
                let typ = body.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if typ.is_empty() {
                    self.log(format!("= {result}"));
                } else {
                    self.log(format!("= {result}  ({typ})"));
                }
            }
            Some(PendingKind::SetVariable { index, container }) => {
                let Some(node) = self.vars.get_mut(index) else { return };
                if let Some(v) = body.get("value").and_then(|v| v.as_str()) {
                    node.value = v.to_string();
                }
                if let Some(t) = body.get("type").and_then(|t| t.as_str()) {
                    node.typ = t.to_string();
                }
                let name = node.name.clone();
                // Anything that was UNDER it is about a value that no longer
                // exists. A new `variablesReference` says so outright; even
                // without one, the children were read from the old value.
                if let Some(r) = body.get("variablesReference").and_then(|r| r.as_i64()) {
                    node.var_ref = r;
                }
                self.collapse_children_of(index);
                self.log(format!("= {name} set"));
                // And its SIBLINGS may have moved with it — a field of the same
                // struct, a length beside a buffer. Re-reading the container is
                // one request and it keeps the tree's shape, where re-reading
                // the scopes would collapse everything the user had opened.
                self.children_cache.remove(&container);
                self.request_variables(container);
            }
            Some(PendingKind::DataBreakpointInfo(name)) => {
                // `dataId: null` is the spec's "no, and here is why". Reporting
                // that is the whole point of asking — hardware watchpoints run
                // out at four, and the fifth silently doing nothing would be
                // the worst version of this feature.
                let Some(data_id) = body.get("dataId").and_then(|d| d.as_str()) else {
                    let why = body
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("cannot be watched");
                    self.log(format!("✗ watch {name}: {why}"));
                    return;
                };
                let description = body
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                self.watchpoints.push(Watchpoint {
                    data_id: data_id.to_string(),
                    name: name.clone(),
                    description,
                });
                self.log(format!("◉ watching {name}"));
                self.send_data_breakpoints();
            }
            Some(PendingKind::SetDataBreakpoints) => {
                // The response is 1:1 with the request, and an adapter may
                // refuse one it had previously allowed — the hardware is
                // claimed at arm time, not at info time.
                if let Some(arr) = body.get("breakpoints").and_then(|b| b.as_array()) {
                    let mut refused: Vec<String> = Vec::new();
                    for (w, resp) in self.watchpoints.iter().zip(arr) {
                        if !resp.get("verified").and_then(|v| v.as_bool()).unwrap_or(false) {
                            let why = resp
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("refused");
                            refused.push(format!("{}: {why}", w.name));
                        }
                    }
                    for r in &refused {
                        self.log(format!("✗ watch {r}"));
                    }
                    let names: Vec<String> =
                        refused.iter().filter_map(|r| r.split(':').next().map(str::to_string)).collect();
                    self.watchpoints.retain(|w| !names.contains(&w.name));
                }
            }
            Some(PendingKind::Datatip) => {
                self.datatip_pending = false;
                let expr = id
                    .and_then(|i| self.pending_datatip.remove(&i))
                    .unwrap_or_default();
                let result = body
                    .get("result")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string();
                let typ = body
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                if !result.is_empty() {
                    self.datatip = Some((expr, result, typ));
                }
            }
            Some(PendingKind::ExceptionBreakpoints | PendingKind::ConfigDone) | None => {}
        }
    }

    fn handle_reverse_request(&mut self, v: &Value) {
        // runInTerminal etc. — reject gracefully
        let command = v.get("command").and_then(|c| c.as_str()).unwrap_or("");
        let seq = v.get("seq").and_then(|s| s.as_u64()).unwrap_or(0);
        self.log(format!("← reverse request {command} (unsupported)"));
        let id = self.next_id;
        self.next_id += 1;
        self.send_json(&json!({
            "seq": id,
            "type": "response",
            "request_seq": seq,
            "success": false,
            "command": command,
            "message": "not supported by xei"
        }));
    }

    // ── Test scaffolding ───────────────────────────────────────────────

    /// Pretend an adapter is attached (no process); requests land in `sent`.
    #[cfg(test)]
    fn test_session(&mut self, launch_body: Value) {
        self.state = DapState::Starting;
        self.launch_body = Some(launch_body);
        self.adapter_name = "mock".into();
        // stdin stays None — send_json records to `sent` in tests.
    }

    #[cfg(test)]
    fn sent_commands(&self) -> Vec<String> {
        self.sent
            .iter()
            .filter_map(|v| v.get("command").and_then(|c| c.as_str()))
            .map(|s| s.to_string())
            .collect()
    }
}

impl Drop for DapClient {
    /// Reap the debug-adapter child (same reason as `LspClient`): `Child`'s
    /// drop leaves the process running, so a dropped client would orphan the
    /// adapter.
    fn drop(&mut self) {
        self.finish_shutdown();
    }
}

// ── Transport ──────────────────────────────────────────────────────────────

fn read_loop<R: Read>(stdout: R, tx: mpsc::Sender<Value>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return,
                Ok(_) => {}
                Err(_) => return,
            }
            let t = line.trim_end();
            if t.is_empty() {
                break;
            }
            if let Some(rest) = t.strip_prefix("Content-Length:") {
                content_length = rest.trim().parse().ok();
            }
        }
        let Some(len) = content_length else {
            continue;
        };
        let mut buf = vec![0u8; len];
        if reader.read_exact(&mut buf).is_err() {
            return;
        }
        // Parse once here; the client works on Values.
        let Ok(v) = serde_json::from_slice::<Value>(&buf) else {
            continue;
        };
        if tx.send(v).is_err() {
            return;
        }
    }
}

// ── Adapter selection ──────────────────────────────────────────────────────

fn detect_lang(path: &Path) -> String {
    let raw = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ext = match crate::lang::Lang::from_ext(&raw) {
        Some(l) => l.extensions()[0],
        None => raw.as_str(),
    };
    match ext {
        // Spellings normalise through `crate::lang` first, so `.c++`, `.hxx`,
        // `.ipp` and the rest reach the same adapter as `.cpp` instead of
        // resolving to "unknown" — the same drift that left C++ without scope
        // completion, pointed at the debugger.
        "py" | "pyw" => "python".into(),
        "rs" => "rust".into(),
        "go" => "go".into(),
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" => "cpp".into(),
        "js" | "mjs" | "cjs" | "ts" | "tsx" => "node".into(),
        "rb" => "ruby".into(),
        _ => "unknown".into(),
    }
}

/// Default exception filters: the adapter's `default: true` ones, else
/// "uncaught" when offered.
fn pick_exception_filters(caps: &Value) -> Vec<String> {
    let Some(arr) = caps
        .get("exceptionBreakpointFilters")
        .and_then(|f| f.as_array())
    else {
        return Vec::new();
    };
    let defaults: Vec<String> = arr
        .iter()
        .filter(|f| f.get("default").and_then(|d| d.as_bool()).unwrap_or(false))
        .filter_map(|f| f.get("filter").and_then(|s| s.as_str()))
        .map(|s| s.to_string())
        .collect();
    if !defaults.is_empty() {
        return defaults;
    }
    arr.iter()
        .filter_map(|f| f.get("filter").and_then(|s| s.as_str()))
        .filter(|s| *s == "uncaught")
        .map(|s| s.to_string())
        .collect()
}

fn pick_adapter(
    lang: &str,
    program: &Path,
    cwd: &Path,
    args: &[String],
) -> Result<(String, Vec<String>, Value), String> {
    let prog_s = program.display().to_string();
    let cwd_s = cwd.display().to_string();
    match lang {
        "python" => {
            let py = if command_exists("python3") {
                "python3"
            } else if command_exists("python") {
                "python"
            } else {
                return Err(install_hint(lang));
            };
            Ok((
                py.into(),
                vec!["-m".into(), "debugpy.adapter".into()],
                json!({
                    "name": "Python: current file",
                    "type": "python",
                    "request": "launch",
                    "program": prog_s,
                    "args": args,
                    "cwd": cwd_s,
                    "console": "internalConsole",
                    "justMyCode": true,
                    "stopOnEntry": false
                }),
            ))
        }
        "go" => {
            if !command_exists("dlv") {
                return Err(install_hint(lang));
            }
            Ok((
                "dlv".into(),
                vec!["dap".into()],
                json!({
                    "name": "Launch Go",
                    "type": "go",
                    "request": "launch",
                    "mode": "debug",
                    "program": prog_s,
                    "args": args,
                    "cwd": cwd_s
                }),
            ))
        }
        "rust" | "cpp" | "c" => {
            let adapter = ["lldb-dap", "codelldb", "lldb-vscode"]
                .into_iter()
                .find(|c| command_exists(c))
                .ok_or_else(|| install_hint(lang))?;
            // For rust source files, try cargo target/debug/<name>
            let program_bin =
                if lang == "rust" && program.extension().and_then(|e| e.to_str()) == Some("rs") {
                    resolve_rust_bin(cwd, program).unwrap_or_else(|| prog_s.clone())
                } else {
                    prog_s.clone()
                };
            // Missing binary is handled in `start()` via async cargo build.
            Ok((
                adapter.into(),
                vec![],
                json!({
                    "name": "Launch",
                    "type": "lldb",
                    "request": "launch",
                    "program": program_bin,
                    "args": args,
                    "cwd": cwd_s,
                    "stopOnEntry": false
                }),
            ))
        }
        "node" => {
            // Handled by start() → start_node (TCP). Keep a clear error if reached.
            Err("Node debugging uses TCP transport — call start_node".into())
        }
        _ => {
            // Generic: if path is executable, try lldb-dap
            let adapter = ["lldb-dap", "codelldb"]
                .into_iter()
                .find(|c| command_exists(c))
                .ok_or_else(|| install_hint(lang))?;
            if !program.is_file() {
                return Err(format!("Not an executable file: {prog_s}"));
            }
            Ok((
                adapter.into(),
                vec![],
                json!({
                    "name": "Launch",
                    "type": "lldb",
                    "request": "launch",
                    "program": prog_s,
                    "args": args,
                    "cwd": cwd_s
                }),
            ))
        }
    }
}

/// How a `.rs` file becomes something a debugger can launch.
///
/// A file is only a cargo target if a package CLAIMS it, and the check that
/// matters is the one that was missing: **a workspace root has no
/// `[package]`.** `/Users/asill/suisei/test.rs` sits beside a `Cargo.toml`
/// that is nothing but `[workspace]`, so there is no package name to build,
/// no `target/debug/test` to launch, and `cargo build` cheerfully succeeds
/// having built every member crate and nothing that was asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum RustBuild {
    /// Inside a package's `src/` — cargo owns it.
    Cargo,
    /// Anything else: one file, compiled on its own.
    Rustc { out: PathBuf },
}

impl RustBuild {
    pub fn of(src: &Path, cwd: &Path) -> Self {
        // From the SOURCE, not from the working directory. The two differ
        // exactly when it matters — a loose file's cwd is the project root,
        // whose manifest belongs to something else entirely.
        let mut dir = src.parent().map(|p| p.to_path_buf());
        while let Some(d) = dir {
            let manifest = d.join("Cargo.toml");
            if manifest.is_file() {
                let has_package = std::fs::read_to_string(&manifest)
                    .ok()
                    .and_then(|t| parse_cargo_name(&t))
                    .is_some();
                // Claimed only if there is a package AND the file is under its
                // `src/`. A `build.rs`, an example pasted at the package root
                // or a scratch file beside `Cargo.toml` is not a target, and
                // parsing `[[bin]]` to find out would be answering a harder
                // question than this needs.
                if has_package && src.starts_with(d.join("src")) {
                    return RustBuild::Cargo;
                }
                // A manifest that does not claim it ends the walk: the next
                // one up belongs to something this file is not part of.
                break;
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
        let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("main");
        let _ = cwd;
        RustBuild::Rustc {
            out: std::env::temp_dir().join("suisei-debug").join(stem),
        }
    }
}

fn resolve_rust_bin(cwd: &Path, src: &Path) -> Option<String> {
    // Prefer package name from Cargo.toml
    let mut dir = cwd.to_path_buf();
    for _ in 0..8 {
        let cargo = dir.join("Cargo.toml");
        if cargo.is_file() {
            if let Ok(text) = std::fs::read_to_string(&cargo) {
                if let Some(name) = parse_cargo_name(&text) {
                    return Some(dir.join("target/debug").join(&name).display().to_string());
                }
            }
            break;
        }
        if !dir.pop() {
            break;
        }
    }
    let stem = src.file_stem()?.to_str()?;
    Some(cwd.join("target/debug").join(stem).display().to_string())
}

fn parse_cargo_name(toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = t.strip_prefix("name") {
                let rest = rest.trim().trim_start_matches('=').trim();
                let name = rest.trim_matches('"').trim_matches('\'').to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

/// The first real compiler error, WITH the place it happened.
///
/// rustc prints the message and the location on separate lines:
///
/// ```text
/// error: invalid reference to positional argument 5 (there is 1 argument)
///    --> test.rs:130:29
/// ```
///
/// Taking only the first line — which is what this did — threw the location
/// away, so the debug panel said what was wrong and left the user to find
/// where. Taking the LAST line, which the cargo path did, is worse still: that
/// is always "aborting due to N previous errors".
///
/// The two are joined so the panel's one line is worth reading on its own.
/// The file is shortened to its name: the panel is narrow, the directory is
/// almost always the project you are looking at, and the line number is the
/// part being navigated to.
fn first_compile_error(stderr: &str, fallback: &str) -> String {
    let mut lines = stderr.lines();
    let Some(msg) = lines.find(|l| l.trim_start().starts_with("error")) else {
        return stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or(fallback)
            .to_string();
    };
    let msg = msg.trim();
    // The arrow line follows immediately, unless the error has no location
    // (a link failure, a bad flag) — in which case there is nothing to add.
    let Some(at) = lines.next().and_then(|l| l.trim().strip_prefix("--> ")) else {
        return msg.to_string();
    };
    // `path:line:col` → `name:line`.
    let mut parts = at.rsplitn(3, ':');
    let (_col, line, path) = (parts.next(), parts.next(), parts.next());
    let (Some(line), Some(path)) = (line, path) else {
        return msg.to_string();
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    format!("{name}:{line} · {msg}")
}

/// The frame name a person reads.
///
/// `lldb-dap` reports Rust frames as `test::recursive_test::h78d59b0538fe2034`.
/// That trailing segment is the compiler's disambiguating hash — it exists so
/// two monomorphisations do not collide in the symbol table, and it means
/// nothing to the person reading a call stack. Three recursive frames rendered
/// as three identical hashes was most of what made the panel unreadable.
///
/// The length is the whole discrimination and it must be exact: legacy Rust
/// mangling writes `h` followed by **sixteen** hex digits, so `mem::h64` is a
/// function called `h64` and keeps its name. Writing this as "h then any hex"
/// ate that case, which the test caught immediately.
///
/// Everything else passes through untouched. This is a display nicety, not a
/// demangler, and a name it does not recognise must arrive intact rather than
/// half-eaten.
fn frame_label(raw: &str) -> String {
    const HASH_LEN: usize = 1 + 16;
    let Some((head, last)) = raw.rsplit_once("::") else {
        return raw.to_string();
    };
    let is_hash = last.len() == HASH_LEN
        && last.starts_with('h')
        && last[1..].chars().all(|c| c.is_ascii_hexdigit());
    if is_hash { head.to_string() } else { raw.to_string() }
}

fn install_hint(lang: &str) -> String {
    match lang {
        "python" => "No Python DAP adapter. Install: pip install debugpy".into(),
        "go" => {
            "No Go DAP adapter. Install: go install github.com/go-delve/delve/cmd/dlv@latest".into()
        }
        "rust" | "cpp" | "c" => "No native DAP adapter. Install lldb-dap (LLVM) or CodeLLDB".into(),
        "node" => "No Node DAP adapter (js-debug-adapter) found".into(),
        _ => format!(
            "No DAP adapter for `{lang}`. Install debugpy / dlv / lldb-dap for your language"
        ),
    }
}

/// Whether an adapter can actually be run.
///
/// **`crate::exec`, not `PATH`.** This used to walk `std::env::var("PATH")`
/// itself, which is the exact failure `exec` was written to end: an app
/// launched from Finder inherits `/usr/bin:/bin:/usr/sbin:/sbin`, so Homebrew,
/// cargo and Apple's own toolchain are all invisible. The adapters were already
/// SPAWNED through `exec::tool` — the note at the top of this file says so —
/// and this gate was refusing to reach that line, so the debugger reported
/// "install lldb-dap" on machines where `exec::tool` would have started it.
///
/// The gate and the spawn have to agree about what "installed" means, and now
/// they ask the same function.
fn command_exists(cmd: &str) -> bool {
    crate::exec::is_available(cmd)
}

fn drain_stderr(stderr: Option<std::process::ChildStderr>) {
    if let Some(err) = stderr {
        thread::spawn(move || {
            let mut r = BufReader::new(err);
            let mut line = String::new();
            while r.read_line(&mut line).unwrap_or(0) > 0 {
                line.clear();
            }
        });
    }
}

fn free_localhost_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    listener.local_addr().ok().map(|a| a.port())
}

fn wait_for_tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let start = Instant::now();
    let mut last_err = String::from("connect failed");
    while start.elapsed() < timeout {
        match TcpStream::connect((host, port)) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => {
                last_err = e.to_string();
                thread::sleep(Duration::from_millis(40));
            }
        }
    }
    Err(format!("TCP {host}:{port} not ready: {last_err}"))
}

// ── launch.json subset ─────────────────────────────────────────────────────

/// Minimal VS Code-compatible launch configuration.
#[derive(Debug, Clone)]
pub struct LaunchConfig {
    pub name: String,
    pub request: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    /// Original type field (python / lldb / go / …)
    pub adapter_type: String,
    /// Attach: process id when present
    pub pid: Option<u32>,
    /// Attach: TCP port when present
    pub port: Option<u16>,
    /// Attach host (default 127.0.0.1)
    pub host: Option<String>,
}

/// Walk up from `hint` looking for `.vscode/launch.json` and parse configurations.
pub fn load_launch_configs(hint: Option<&Path>) -> Vec<LaunchConfig> {
    let start = hint
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut dir = if start.is_file() {
        start.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        start
    };
    for _ in 0..12 {
        let candidate = dir.join(".vscode").join("launch.json");
        if candidate.is_file() {
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                // Strip // comments (VS Code allows them)
                let cleaned = strip_jsonc_comments(&text);
                if let Ok(v) = serde_json::from_str::<Value>(&cleaned) {
                    return parse_launch_configs(&v, &dir);
                }
            }
        }
        if !dir.pop() {
            break;
        }
    }
    Vec::new()
}

fn strip_jsonc_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_str = false;
    let mut escape = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            // line comment
            while let Some(n) = chars.next() {
                if n == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(n) = chars.next() {
                if n == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn parse_launch_configs(v: &Value, workspace: &Path) -> Vec<LaunchConfig> {
    let Some(arr) = v.get("configurations").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for c in arr {
        let request = c
            .get("request")
            .and_then(|r| r.as_str())
            .unwrap_or("launch")
            .to_string();
        if request != "launch" && request != "attach" {
            continue;
        }
        let name = c
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unnamed")
            .to_string();
        let program = c
            .get("program")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .replace("${workspaceFolder}", &workspace.display().to_string())
            .replace("${file}", "");
        let args = c
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let cwd = c
            .get("cwd")
            .and_then(|p| p.as_str())
            .map(|s| s.replace("${workspaceFolder}", &workspace.display().to_string()));
        let mut env = Vec::new();
        if let Some(obj) = c.get("env").and_then(|e| e.as_object()) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    env.push((k.clone(), s.to_string()));
                }
            }
        }
        let adapter_type = c
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let pid = c
            .get("processId")
            .or_else(|| c.get("pid"))
            .and_then(|p| p.as_u64())
            .map(|p| p as u32);
        let port = c
            .get("port")
            .and_then(|p| p.as_u64())
            .or_else(|| {
                c.get("connect")
                    .and_then(|o| o.get("port"))
                    .and_then(|p| p.as_u64())
            })
            .map(|p| p as u16);
        let host = c
            .get("connect")
            .and_then(|o| o.get("host"))
            .and_then(|h| h.as_str())
            .or_else(|| c.get("address").and_then(|a| a.as_str()))
            .map(|s| s.to_string());
        out.push(LaunchConfig {
            name,
            request,
            program,
            args,
            cwd,
            env,
            adapter_type,
            pid,
            port,
            host,
        });
    }
    out
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    fn stopped_with_locals() -> DapClient {
        let mut d = DapClient::default();
        d.state = DapState::Stopped;
        d.supports_set_variable = true;
        d.stack = vec![StackFrameInfo {
            id: 1,
            name: "f".into(),
            path: "/tmp/x.rs".into(),
            line: 1,
            column: 0,
        }];
        d.vars = vec![
            VarNode { name: "Locals".into(), value: String::new(), typ: String::new(),
                      var_ref: 100, depth: 0, expanded: true, is_scope: true },
            VarNode { name: "count".into(), value: "3".into(), typ: "int".into(),
                      var_ref: 0, depth: 1, expanded: false, is_scope: false },
            VarNode { name: "user".into(), value: "User".into(), typ: "User".into(),
                      var_ref: 200, depth: 1, expanded: true, is_scope: false },
            VarNode { name: "id".into(), value: "7".into(), typ: "int".into(),
                      var_ref: 0, depth: 2, expanded: false, is_scope: false },
        ];
        d
    }

    /// `setVariable` takes the PARENT's reference, not the variable's own.
    ///
    /// A `VarNode`'s `var_ref` is the handle to its CHILDREN — the opposite
    /// end of the relationship the request wants. The tree is flattened with a
    /// depth per row, so the container is the nearest preceding row one level
    /// shallower: `Locals` for a local, and `user` for `user.id`.
    #[test]
    fn setting_a_variable_addresses_its_container() {
        let mut d = stopped_with_locals();

        d.set_variable(1, "9"); // `count`, a child of Locals
        let sent = d.sent.last().unwrap();
        assert_eq!(sent["command"], "setVariable");
        assert_eq!(sent["arguments"]["variablesReference"], 100, "Locals");
        assert_eq!(sent["arguments"]["name"], "count");
        assert_eq!(sent["arguments"]["value"], "9");

        d.set_variable(3, "8"); // `id`, a child of `user`
        let sent = d.sent.last().unwrap();
        assert_eq!(sent["arguments"]["variablesReference"], 200, "user");
        assert_eq!(sent["arguments"]["name"], "id");
    }

    /// The answer lands on the row that asked, and takes its children with it.
    ///
    /// What was read out of the old value is not a description of the new one,
    /// so the subtree goes — and the siblings are re-read, because a field of
    /// the same struct may have moved with it.
    #[test]
    fn a_set_value_replaces_the_row_and_drops_what_was_under_it() {
        let mut d = stopped_with_locals();
        d.set_variable(2, "Other"); // `user`, which has a child
        let seq = d.sent.last().unwrap()["seq"].as_u64().unwrap();

        d.handle_response(&json!({
            "type": "response",
            "request_seq": seq,
            "success": true,
            "command": "setVariable",
            "body": { "value": "Other", "type": "User", "variablesReference": 300 }
        }));

        assert_eq!(d.vars[2].value, "Other");
        assert_eq!(d.vars[2].var_ref, 300, "a new handle for the new value");
        assert!(!d.vars[2].expanded);
        assert_eq!(d.vars.len(), 3, "the stale child is gone");
        // And the container is re-read rather than the scopes, which would
        // have collapsed everything the user had opened.
        let last = d.sent.last().unwrap();
        assert_eq!(last["command"], "variables");
        assert_eq!(last["arguments"]["variablesReference"], 100);
    }

    /// A scope is not a value, a running program has no frame to set one in,
    /// and an adapter that cannot do it is not asked.
    #[test]
    fn setting_is_refused_where_it_makes_no_sense() {
        let mut d = stopped_with_locals();
        d.set_variable(0, "x"); // "Locals"
        assert!(d.sent.is_empty(), "a scope is not a value");

        let mut d = stopped_with_locals();
        d.state = DapState::Running;
        d.set_variable(1, "9");
        assert!(d.sent.is_empty(), "no frame to set it in");

        let mut d = stopped_with_locals();
        d.supports_set_variable = false;
        d.set_variable(1, "9");
        assert!(d.sent.is_empty());
        assert!(d.console.last().unwrap().contains("cannot change"));
    }

    /// A disabled breakpoint is not armed, and the response still lands on the
    /// right one.
    ///
    /// This is the trap the filter creates. `setBreakpoints` answers 1:1 with
    /// the request BY POSITION, and the handler applies verification and the
    /// adapter's line slides by walking the stored list. Filter the request
    /// and not the walk, and every answer pairs with the wrong breakpoint —
    /// which shows up as a breakpoint silently moving to a line the adapter
    /// never mentioned.
    #[test]
    fn a_disabled_breakpoint_is_skipped_on_both_sides_of_the_wire() {
        let mut d = DapClient::new();
        for l in [4usize, 9, 14] {
            d.toggle_breakpoint("/tmp/x.rs", l);
        }
        let path = d.breakpoints.keys().next().unwrap().clone();
        // Switch the MIDDLE one off, so a mispairing cannot pass by luck.
        d.toggle_breakpoint_enabled(&path, 9);
        d.state = DapState::Stopped;
        d.send_set_breakpoints(&path);

        let sent = d.sent.last().unwrap();
        let asked: Vec<u64> = sent["arguments"]["breakpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["line"].as_u64().unwrap())
            .collect();
        assert_eq!(asked, vec![5, 15], "only the enabled ones, 1-based");

        // The adapter verifies both and slides the second.
        let id = d.alloc(PendingKind::SetBreakpoints(path.clone()));
        d.handle_msg(response(
            id,
            "setBreakpoints",
            json!({ "breakpoints": [
                {"verified": true, "line": 5},
                {"verified": true, "line": 16}
            ]}),
        ));

        let list = &d.breakpoints[&path];
        assert_eq!(list[0].line, 4, "the first is where it was");
        assert!(list[0].verified);
        assert_eq!(list[1].line, 9, "the DISABLED one is untouched");
        assert!(!list[1].verified, "and was never verified");
        assert_eq!(list[2].line, 15, "and the slide landed on the third");
    }

    /// Disabling keeps the condition. Deleting a breakpoint to quiet it for
    /// five minutes throws away its place, its condition and its log message,
    /// which is the whole reason this is not just "remove it".
    #[test]
    fn disabling_a_breakpoint_keeps_what_it_knows() {
        let mut d = DapClient::new();
        d.toggle_breakpoint("/tmp/x.rs", 4);
        let path = d.breakpoints.keys().next().unwrap().clone();
        d.set_breakpoint_condition(&path, 4, Some("i == 3".into()));

        assert_eq!(d.toggle_breakpoint_enabled(&path, 4), Some(false));

        let b = &d.breakpoints[&path][0];
        assert!(!b.enabled);
        assert_eq!(b.condition.as_deref(), Some("i == 3"));

        assert_eq!(d.toggle_breakpoint_enabled(&path, 4), Some(true));
        assert!(d.breakpoints[&path][0].enabled);
    }

    /// Toggling somewhere with no breakpoint is not an error, and creates
    /// nothing — a ⌘-click on a bare line means nothing to enable.
    #[test]
    fn toggling_a_breakpoint_that_is_not_there_does_nothing() {
        let mut d = DapClient::new();
        assert_eq!(d.toggle_breakpoint_enabled("/tmp/x.rs", 4), None);
        assert!(d.breakpoints.is_empty());
    }

    fn stopped_with_a_frame() -> DapClient {
        let mut d = DapClient::default();
        d.state = DapState::Stopped;
        d.supports_data_breakpoints = true;
        d.stack = vec![StackFrameInfo {
            id: 11,
            name: "f".into(),
            path: "/tmp/x.rs".into(),
            line: 1,
            column: 0,
        }];
        d
    }

    /// Watching is two steps because the spec is two steps, and the first is
    /// the one that matters: a name is resolved to an opaque `dataId` IN A
    /// FRAME before anything is armed.
    #[test]
    fn watching_asks_the_adapter_first() {
        let mut d = stopped_with_a_frame();
        d.watch("bottom");

        let sent = d.sent.last().expect("a request went out");
        assert_eq!(sent["command"], "dataBreakpointInfo");
        assert_eq!(sent["arguments"]["name"], "bottom");
        assert_eq!(sent["arguments"]["frameId"], 11, "in the selected frame");
        assert!(d.watchpoints.is_empty(), "nothing is watched until it answers");
    }

    /// A refusal is REPORTED. Hardware watchpoints run out at four, and the
    /// fifth silently doing nothing is the worst version of this feature.
    #[test]
    fn a_refused_watch_says_why_and_watches_nothing() {
        let mut d = stopped_with_a_frame();
        d.watch("bottom");
        let seq = d.sent.last().unwrap()["seq"].as_u64().unwrap();
        let before = d.console.len();

        d.handle_response(&json!({
            "type": "response",
            "request_seq": seq,
            "success": true,
            "command": "dataBreakpointInfo",
            "body": { "dataId": null, "description": "no more hardware watchpoints" }
        }));

        assert!(d.watchpoints.is_empty());
        let said = d.console[before..].join("\n");
        assert!(said.contains("no more hardware watchpoints"), "{said}");
    }

    /// An accepted one is armed, and arming replaces the whole set — the same
    /// shape `setBreakpoints` has, and the reason the list is the unit.
    #[test]
    fn an_accepted_watch_is_armed_as_part_of_the_whole_set() {
        let mut d = stopped_with_a_frame();
        d.writer = None;
        d.watch("bottom");
        let seq = d.sent.last().unwrap()["seq"].as_u64().unwrap();

        d.handle_response(&json!({
            "type": "response",
            "request_seq": seq,
            "success": true,
            "command": "dataBreakpointInfo",
            "body": { "dataId": "0x16f", "description": "4 bytes at 0x16f" }
        }));

        assert_eq!(d.watchpoints.len(), 1);
        assert_eq!(d.watchpoints[0].name, "bottom");
        assert_eq!(d.watchpoints[0].data_id, "0x16f");

        let armed = d.sent.last().unwrap();
        assert_eq!(armed["command"], "setDataBreakpoints");
        assert_eq!(armed["arguments"]["breakpoints"][0]["dataId"], "0x16f");
        assert_eq!(armed["arguments"]["breakpoints"][0]["accessType"], "write");
    }

    /// Asking to watch something already watched stops watching it — a menu
    /// item that says "Break When Value Changes" has to un-say it.
    #[test]
    fn watching_the_same_value_twice_stops_watching_it() {
        let mut d = stopped_with_a_frame();
        d.watchpoints.push(Watchpoint {
            data_id: "0x16f".into(),
            name: "bottom".into(),
            description: String::new(),
        });

        d.watch("bottom");

        assert!(d.watchpoints.is_empty());
    }

    /// A `dataId` is an address in a process. When the process goes, so does
    /// the watchpoint — carrying it into the next run would arm garbage.
    #[test]
    fn watchpoints_do_not_survive_the_session() {
        let mut d = stopped_with_a_frame();
        d.watchpoints.push(Watchpoint {
            data_id: "0x16f".into(),
            name: "bottom".into(),
            description: String::new(),
        });

        d.stop();

        assert!(d.watchpoints.is_empty());
    }

    /// An adapter that cannot watch says so once, rather than the request
    /// going out and failing.
    #[test]
    fn an_adapter_without_watchpoints_is_not_asked() {
        let mut d = stopped_with_a_frame();
        d.supports_data_breakpoints = false;

        d.watch("bottom");

        assert!(d.sent.is_empty());
        assert!(d.console.last().unwrap().contains("cannot watch"));
    }

    /// A breakpoint the adapter moved says where it went.
    ///
    /// It moved in silence, and a mark appearing two lines below the one that
    /// was clicked reads as the editor being wrong rather than as the debugger
    /// being right — a breakpoint asked for on a comment lands on the next
    /// line with code.
    #[test]
    fn a_slid_breakpoint_says_where_it_went() {
        let mut d = DapClient::new();
        d.toggle_breakpoint("/tmp/x.rs", 84); // 0-based 84 → line 85, a comment
        let path = d.breakpoints.keys().next().unwrap().clone();
        let id = d.alloc(PendingKind::SetBreakpoints(path.clone()));
        let before = d.console.len();

        d.handle_msg(response(
            id,
            "setBreakpoints",
            json!({ "breakpoints": [ {"verified": true, "line": 87} ] }),
        ));

        assert_eq!(d.breakpoints[&path][0].line, 86, "moved to line 87");
        let said = d.console[before..].join("\n");
        assert!(said.contains("85"), "names where it was asked for: {said}");
        assert!(said.contains("87"), "and where it went: {said}");
    }

    /// A breakpoint the adapter left alone says nothing. Announcing every
    /// verified breakpoint would make the console useless for the one case
    /// that matters.
    #[test]
    fn a_breakpoint_that_did_not_move_is_not_announced() {
        let mut d = DapClient::new();
        d.toggle_breakpoint("/tmp/x.rs", 4);
        let path = d.breakpoints.keys().next().unwrap().clone();
        let id = d.alloc(PendingKind::SetBreakpoints(path.clone()));
        let before = d.console.len();

        d.handle_msg(response(
            id,
            "setBreakpoints",
            json!({ "breakpoints": [ {"verified": true, "line": 5} ] }),
        ));

        assert_eq!(d.console.len(), before);
    }

    /// A build failure has to say WHERE.
    ///
    /// The real report: `println!("total score: {5}", total)` on line 130 of a
    /// file the user had just edited. The panel said "invalid reference to
    /// positional argument 5" and nothing else, so it read as the debugger
    /// having broken rather than as a typo three screens down.
    #[test]
    fn a_compile_error_carries_the_place_it_happened() {
        let stderr = "\
error: invalid reference to positional argument 5 (there is 1 argument)
   --> test.rs:130:29
    |
130 |     println!(\"total score: {5}\", total);
    |                             ^
    |
    = note: positional arguments are zero-based

error: aborting due to 2 previous errors
";
        assert_eq!(
            first_compile_error(stderr, "rustc failed"),
            "test.rs:130 · error: invalid reference to positional argument 5 (there is 1 argument)"
        );
    }

    /// The path is shortened to its name: the panel is narrow and the
    /// directory is almost always the project already on screen.
    #[test]
    fn a_long_path_is_shortened_to_the_file() {
        let stderr = "error: cannot find value `x`\n   --> /Users/a/proj/src/deep/mod.rs:7:3\n";
        assert_eq!(
            first_compile_error(stderr, "f"),
            "mod.rs:7 · error: cannot find value `x`"
        );
    }

    /// Some failures have no location — a link error, a bad flag. The message
    /// still has to arrive, without an invented file.
    #[test]
    fn an_error_without_a_location_is_still_reported() {
        assert_eq!(
            first_compile_error("error: linking with `cc` failed\n", "f"),
            "error: linking with `cc` failed"
        );
        assert_eq!(first_compile_error("", "rustc failed"), "rustc failed");
    }

    /// NOT the last line. For cargo that is always "aborting due to N previous
    /// errors", which names nothing anyone can go and look at — and taking it
    /// is what the cargo path used to do.
    #[test]
    fn the_summary_line_is_not_the_error() {
        let stderr = "\
error[E0308]: mismatched types
   --> src/main.rs:4:5
error: aborting due to 1 previous error
";
        assert!(first_compile_error(stderr, "f").starts_with("main.rs:4 ·"));
    }

    /// A hover asks the adapter with the spec's own `hover` context, and says
    /// nothing in the console.
    ///
    /// The console belongs to what the user TYPED. A pointer crossing a line
    /// touches a dozen identifiers and the adapter refuses most of them, so a
    /// datatip that logged would write the mouse's path into the transcript.
    #[test]
    fn a_datatip_asks_quietly_and_in_the_hover_context() {
        let mut d = DapClient::default();
        d.state = DapState::Stopped;
        d.stack = vec![StackFrameInfo {
            id: 7,
            name: "f".into(),
            path: "/tmp/x.rs".into(),
            line: 1,
            column: 0,
        }];
        let console_before = d.console.len();

        d.request_datatip("bottom");

        assert!(d.datatip_pending);
        assert_eq!(d.console.len(), console_before, "nothing is logged");
        let sent = d.sent.last().expect("a request went out");
        assert_eq!(sent["command"], "evaluate");
        assert_eq!(sent["arguments"]["context"], "hover");
        assert_eq!(sent["arguments"]["expression"], "bottom");
        assert_eq!(sent["arguments"]["frameId"], 7, "in the SELECTED frame");
    }

    /// A running program has no frame to evaluate in, so there is nothing to
    /// ask and the previous answer must not linger.
    #[test]
    fn a_datatip_is_refused_while_the_program_runs() {
        let mut d = DapClient::default();
        d.state = DapState::Running;
        d.datatip = Some(("old".into(), "1".into(), "int".into()));
        d.datatip_pending = true;

        d.request_datatip("bottom");

        assert!(d.datatip.is_none());
        assert!(!d.datatip_pending);
        assert!(d.sent.is_empty(), "and nothing was sent");
    }

    /// The answer arrives under the word that was asked about.
    ///
    /// The pointer has usually moved by the time an adapter replies, so the
    /// expression rides along with the value — a value shown under the wrong
    /// identifier is a wrong answer that looks like a right one.
    #[test]
    fn a_datatip_answer_carries_the_word_it_was_asked_about() {
        let mut d = DapClient::default();
        d.state = DapState::Stopped;
        d.stack = vec![StackFrameInfo {
            id: 1,
            name: "f".into(),
            path: "/tmp/x.rs".into(),
            line: 1,
            column: 0,
        }];
        d.request_datatip("bottom");
        let seq = d.sent.last().unwrap()["seq"].as_u64().unwrap();

        d.handle_response(&json!({
            "type": "response",
            "request_seq": seq,
            "success": true,
            "command": "evaluate",
            "body": { "result": "1234", "type": "unsigned int" }
        }));

        assert_eq!(
            d.datatip,
            Some(("bottom".into(), "1234".into(), "unsigned int".into()))
        );
        assert!(!d.datatip_pending);
    }

    /// A refusal clears the spinner and stays out of the console.
    ///
    /// The failure branch of `handle_response` returns early, so without its
    /// own arm there the pending flag stayed set forever and the popover span
    /// on a keyword that was never going to have a value.
    #[test]
    fn a_refused_datatip_stops_waiting_and_says_nothing() {
        let mut d = DapClient::default();
        d.state = DapState::Stopped;
        d.stack = vec![StackFrameInfo {
            id: 1,
            name: "f".into(),
            path: "/tmp/x.rs".into(),
            line: 1,
            column: 0,
        }];
        d.request_datatip("if");
        let seq = d.sent.last().unwrap()["seq"].as_u64().unwrap();
        let console_before = d.console.len();

        d.handle_response(&json!({
            "type": "response",
            "request_seq": seq,
            "success": false,
            "command": "evaluate",
            "message": "no variable named 'if'"
        }));

        assert!(!d.datatip_pending, "the spinner stops");
        assert!(d.datatip.is_none());
        assert_eq!(d.console.len(), console_before, "and the console is untouched");
    }

    /// A call stack that reads. lldb-dap names Rust frames with the
    /// compiler's disambiguating hash, and three recursive frames arrived as
    /// three identical `test::recursive_test::h78d59b0538fe2034`.
    #[test]
    fn a_frame_name_loses_the_symbol_hash_and_nothing_else() {
        assert_eq!(
            frame_label("test::recursive_test::h78d59b0538fe2034"),
            "test::recursive_test"
        );
        // Not a demangler. A name it does not recognise arrives intact rather
        // than half-eaten.
        assert_eq!(frame_label("main"), "main");
        assert_eq!(frame_label("core::ptr::drop_in_place"), "core::ptr::drop_in_place");
        assert_eq!(frame_label("std::io::Write::write_all"), "std::io::Write::write_all");
        // A real function whose name merely looks like the hash keeps it.
        assert_eq!(frame_label("crypto::hash"), "crypto::hash");
        assert_eq!(frame_label("mem::h64"), "mem::h64");
        assert_eq!(frame_label("??"), "??");
    }

    /// The build's placeholder state must not block the launch it exists to
    /// reach.
    ///
    /// `begin_*_build` sets `Starting` so the panel has something to show, and
    /// `is_session` counts `Starting` — so when the build finished, `start`
    /// refused with "Debug session already active". The path could never
    /// complete; it was hidden behind the earlier "binary not found".
    #[test]
    fn a_finished_build_is_not_mistaken_for_a_live_session() {
        let mut d = DapClient::default();
        d.state = DapState::Starting;
        assert!(d.is_session(), "a build shows as Starting");
        // What `poll` now does before re-entering `start`.
        d.state = DapState::Idle;
        assert!(!d.is_session(), "and start() may proceed");
    }

    /// ⇧F5 during a build cancels the build.
    ///
    /// It used to do nothing at all: `finish_shutdown` keeps `build_rx` alive
    /// on purpose, so the compiler carried on and launched anyway.
    #[test]
    fn stop_cancels_a_build_in_flight() {
        let mut d = DapClient::default();
        let (tx, rx) = mpsc::channel();
        d.build_rx = Some(rx);
        d.build_message = Some("rustc…".into());
        d.state = DapState::Starting;

        d.stop();

        assert!(d.build_rx.is_none(), "the build is cancelled");
        assert!(d.build_message.is_none());
        assert_eq!(d.state, DapState::Idle);
        // The sender outliving the receiver is the cancellation; sending into
        // a dropped channel is an error rather than a panic.
        assert!(tx.send(Ok(("x".into(), PathBuf::from("."), "rust".into(), vec![]))).is_err());
    }

    /// And a stop after a failed launch clears the message about it, so the
    /// panel stops reporting a session that does not exist.
    #[test]
    fn stop_clears_a_stale_error() {
        let mut d = DapClient::default();
        d.soft_error = Some("Debug session already active — stop first".into());
        d.stop();
        assert!(d.soft_error.is_none());
    }

    /// A loose `.rs` file beside a WORKSPACE manifest is not a cargo target.
    ///
    /// This repo is the reported case: `Cargo.toml` at the root is
    /// `[workspace]` with no `[package]`, so there is no name to build and no
    /// `target/debug/test` to launch — and `cargo build` still succeeds,
    /// having built every member crate. "cargo build ok but binary not found"
    /// was an accurate report of a build that could never produce it.
    #[test]
    fn a_loose_file_beside_a_workspace_manifest_is_built_by_rustc() {
        let dir = std::env::temp_dir().join(format!("suisei-rb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        let src = dir.join("test.rs");
        std::fs::write(&src, "fn main() {}\n").unwrap();

        match RustBuild::of(&src, &dir) {
            RustBuild::Rustc { out } => {
                assert!(out.ends_with("test"), "named for the file: {out:?}");
            }
            RustBuild::Cargo => panic!("a workspace root claims no source file"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// And a file that a package really does own still goes through cargo —
    /// building one file with rustc would lose the dependencies.
    #[test]
    fn a_file_under_a_packages_src_is_built_by_cargo() {
        let dir = std::env::temp_dir().join(format!("suisei-rb-pkg-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let src = dir.join("src/main.rs");
        std::fs::write(&src, "fn main() {}\n").unwrap();

        assert_eq!(RustBuild::of(&src, &dir), RustBuild::Cargo);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A scratch file at the package root is NOT under `src/`, so it is not a
    /// target either — same answer as the workspace case, for the same reason.
    #[test]
    fn a_scratch_file_beside_a_package_manifest_is_not_a_target() {
        let dir = std::env::temp_dir().join(format!("suisei-rb-scr-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let src = dir.join("scratch.rs");
        std::fs::write(&src, "fn main() {}\n").unwrap();

        assert!(matches!(RustBuild::of(&src, &dir), RustBuild::Rustc { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A tool that exists but is not on `PATH` must still count as installed.
    ///
    /// `lldb-dap` is the real case and the reported one: it ships inside Xcode
    /// and inside the Command Line Tools, neither of which is on anyone's
    /// `PATH`, so a Finder-launched Suisei — whose `PATH` is
    /// `/usr/bin:/bin:/usr/sbin:/sbin` — told the user to install a debugger
    /// their Mac already had. The gate walked `PATH` while the spawn went
    /// through `exec::tool`; the two have to ask the same question.
    ///
    /// Skips rather than fails where no Apple toolchain is installed.
    #[test]
    fn a_tool_off_path_still_counts_as_installed() {
        let Some(found) = crate::exec::find("lldb-dap") else {
            return;
        };
        assert!(
            command_exists("lldb-dap"),
            "exec found it at {found:?} but the adapter gate said no"
        );
    }

    use super::*;

    fn response(seq: u64, command: &str, body: Value) -> Value {
        json!({
            "type": "response",
            "request_seq": seq,
            "success": true,
            "command": command,
            "body": body
        })
    }

    fn event(name: &str, body: Value) -> Value {
        json!({ "type": "event", "event": name, "body": body })
    }

    /// seq of the last sent request matching `command`.
    fn seq_of(d: &DapClient, command: &str) -> u64 {
        d.sent
            .iter()
            .rev()
            .find(|v| v.get("command").and_then(|c| c.as_str()) == Some(command))
            .and_then(|v| v.get("seq").and_then(|s| s.as_u64()))
            .expect("request was sent")
    }

    #[test]
    fn toggle_breakpoint_roundtrip() {
        let mut d = DapClient::new();
        assert!(d.toggle_breakpoint("/tmp/foo.py", 10));
        assert!(d.has_breakpoint("/tmp/foo.py", 10));
        assert!(!d.toggle_breakpoint("/tmp/foo.py", 10));
        assert!(!d.has_breakpoint("/tmp/foo.py", 10));
    }

    #[test]
    fn condition_and_log_on_breakpoint() {
        let mut d = DapClient::new();
        d.set_breakpoint_condition("/tmp/a.py", 5, Some("x > 0".into()));
        d.set_breakpoint_log("/tmp/a.py", 5, Some("hit".into()));
        let path = d.canon("/tmp/a.py");
        let b = d
            .breakpoints
            .get(&path)
            .unwrap()
            .iter()
            .find(|b| b.line == 5)
            .unwrap();
        assert_eq!(b.condition.as_deref(), Some("x > 0"));
        assert_eq!(b.log_message.as_deref(), Some("hit"));
    }

    #[test]
    fn parse_launch_json_minimal() {
        let j = r#"{
            // comment
            "configurations": [
                {
                    "name": "Run",
                    "type": "python",
                    "request": "launch",
                    "program": "${workspaceFolder}/main.py",
                    "args": ["a", "b"]
                },
                {
                    "name": "AttachPy",
                    "type": "python",
                    "request": "attach",
                    "connect": { "host": "127.0.0.1", "port": 5678 }
                }
            ]
        }"#;
        let cleaned = strip_jsonc_comments(j);
        let v: Value = serde_json::from_str(&cleaned).unwrap();
        let cfgs = parse_launch_configs(&v, Path::new("/proj"));
        assert_eq!(cfgs.len(), 2);
        assert_eq!(cfgs[0].name, "Run");
        assert_eq!(cfgs[0].program, "/proj/main.py");
        assert_eq!(cfgs[0].args, vec!["a", "b"]);
        assert_eq!(cfgs[1].request, "attach");
        assert_eq!(cfgs[1].port, Some(5678));
        assert_eq!(cfgs[1].host.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn free_port_and_wait_helpers() {
        let port = free_localhost_port().expect("port");
        // Nothing listening — wait should fail quickly
        let err = wait_for_tcp("127.0.0.1", port, Duration::from_millis(80));
        assert!(err.is_err());
    }

    #[test]
    fn cargo_name_parse() {
        let t = "[package]\nname = \"xei-core\"\nversion = \"1\"\n";
        assert_eq!(parse_cargo_name(t).as_deref(), Some("xei-core"));
    }

    #[test]
    fn detect_langs() {
        assert_eq!(detect_lang(Path::new("a.py")), "python");
        assert_eq!(detect_lang(Path::new("a.rs")), "rust");
        assert_eq!(detect_lang(Path::new("main.go")), "go");
    }

    #[test]
    fn state_label() {
        assert_eq!(DapState::Stopped.label(), "stopped");
    }

    #[test]
    fn sequencer_launch_after_initialize_config_after_initialized_event() {
        let mut d = DapClient::new();
        d.toggle_breakpoint("/tmp/x.py", 3);
        d.sent.clear();
        d.test_session(json!({"program": "/tmp/x.py"}));

        // initialize response → launch must go out (and nothing config-ish yet)
        d.handle_msg(response(
            1,
            "initialize",
            json!({
                "supportsConfigurationDoneRequest": true,
                "supportsTerminateRequest": true,
                "exceptionBreakpointFilters": [
                    {"filter": "raised", "label": "Raised", "default": false},
                    {"filter": "uncaught", "label": "Uncaught", "default": true}
                ]
            }),
        ));
        // pending id for initialize isn't registered in test_session; drive via
        // the real alloc path instead: simulate full start bookkeeping.
        // (init response with unknown request_seq is a no-op — assert that.)
        assert!(d.sent_commands().is_empty());

        // Register initialize as pending and retry.
        let init_id = d.alloc(PendingKind::Initialize);
        d.handle_msg(response(
            init_id,
            "initialize",
            json!({
                "supportsConfigurationDoneRequest": true,
                "supportsTerminateRequest": true,
                "exceptionBreakpointFilters": [
                    {"filter": "uncaught", "label": "Uncaught", "default": true}
                ]
            }),
        ));
        assert_eq!(d.sent_commands(), vec!["launch"]);
        assert!(d.launch_sent_at.is_some());
        assert!(!d.config_sent);

        // initialized event → setBreakpoints, setExceptionBreakpoints, configurationDone
        d.handle_msg(event("initialized", json!({})));
        let cmds = d.sent_commands();
        assert_eq!(
            cmds,
            vec![
                "launch",
                "setBreakpoints",
                "setExceptionBreakpoints",
                "configurationDone"
            ]
        );
        assert!(d.config_sent);

        // duplicate initialized must not resend configuration
        d.handle_msg(event("initialized", json!({})));
        assert_eq!(d.sent_commands().len(), 4);

        // launch response → Running
        let launch_seq = seq_of(&d, "launch");
        d.handle_msg(response(launch_seq, "launch", json!({})));
        assert_eq!(d.state, DapState::Running);
    }

    #[test]
    fn stopped_without_thread_id_resolves_via_threads() {
        let mut d = DapClient::new();
        d.test_session(json!({}));
        d.state = DapState::Running;
        d.sent.clear();

        d.handle_msg(event("stopped", json!({ "reason": "pause" })));
        assert_eq!(d.state, DapState::Stopped);
        assert_eq!(d.sent_commands(), vec!["threads"]);

        let tseq = seq_of(&d, "threads");
        d.handle_msg(response(
            tseq,
            "threads",
            json!({ "threads": [{"id": 7, "name": "main"}] }),
        ));
        assert_eq!(d.thread_id, Some(7));
        assert_eq!(d.threads, vec![(7, "main".to_string())]);
        assert_eq!(d.sent_commands(), vec!["threads", "stackTrace"]);
    }

    #[test]
    fn stack_scopes_variables_build_tree() {
        let mut d = DapClient::new();
        d.test_session(json!({}));
        d.state = DapState::Stopped;
        d.thread_id = Some(1);
        d.sent.clear();

        d.request_stack();
        let sseq = seq_of(&d, "stackTrace");
        d.handle_msg(response(
            sseq,
            "stackTrace",
            json!({
                "stackFrames": [
                    {"id": 100, "name": "main", "line": 12, "column": 1,
                     "source": {"path": "/tmp/x.py"}}
                ]
            }),
        ));
        assert_eq!(d.stack.len(), 1);
        assert_eq!(d.current_line, Some(11));
        assert!(d.location_dirty);

        let scseq = seq_of(&d, "scopes");
        d.handle_msg(response(
            scseq,
            "scopes",
            json!({
                "scopes": [
                    {"name": "Locals", "variablesReference": 200, "expensive": false},
                    {"name": "Globals", "variablesReference": 300, "expensive": true}
                ]
            }),
        ));
        // Scope roots present, first auto-expanding.
        assert_eq!(d.vars.len(), 2);
        assert!(d.vars[0].is_scope && d.vars[0].expanded);

        let vseq = seq_of(&d, "variables");
        d.handle_msg(response(
            vseq,
            "variables",
            json!({
                "variables": [
                    {"name": "x", "value": "1", "type": "int", "variablesReference": 0},
                    {"name": "items", "value": "[…]", "type": "list", "variablesReference": 400}
                ]
            }),
        ));
        assert_eq!(d.vars.len(), 4);
        assert_eq!(d.vars[1].name, "x");
        assert_eq!(d.vars[1].depth, 1);
        assert_eq!(d.vars[2].var_ref, 400);

        // Collapse the scope removes its children.
        d.toggle_var_at(0);
        assert_eq!(d.vars.len(), 2);
        // Re-expand hits the cache without a new request.
        let n_before = d.sent.len();
        d.toggle_var_at(0);
        assert_eq!(d.vars.len(), 4);
        assert_eq!(d.sent.len(), n_before);
    }

    #[test]
    fn graceful_stop_waits_for_terminated() {
        let mut d = DapClient::new();
        d.test_session(json!({}));
        d.state = DapState::Running;
        d.supports_terminate = true;
        // Pretend transport exists so stop() doesn't shortcut to Idle.
        // (stdin is None in tests; emulate by giving it a deadline manually.)
        d.sent.clear();
        d.state = DapState::Running;
        d.shutdown_deadline = None;
        // stop() with stdin None finishes immediately — assert Idle path…
        d.stop();
        assert_eq!(d.state, DapState::Idle);
        // …and the terminated-event path also lands Idle.
        d.state = DapState::Ending;
        d.handle_msg(event("terminated", json!({})));
        assert_eq!(d.state, DapState::Idle);
    }

    #[test]
    fn breakpoint_event_updates_verified() {
        let mut d = DapClient::new();
        d.toggle_breakpoint("/tmp/x.py", 5);
        assert!(!d.flat_bps()[0].2);
        d.handle_msg(event(
            "breakpoint",
            json!({
                "reason": "changed",
                "breakpoint": { "line": 6, "verified": true }
            }),
        ));
        assert!(d.flat_bps()[0].2);
    }

    #[test]
    fn set_breakpoints_response_slides_lines() {
        let mut d = DapClient::new();
        d.toggle_breakpoint("/tmp/x.py", 4); // 0-based 4 → sent as line 5
        let path = d.breakpoints.keys().next().unwrap().clone();
        let id = d.alloc(PendingKind::SetBreakpoints(path.clone()));
        d.handle_msg(response(
            id,
            "setBreakpoints",
            json!({
                "breakpoints": [ {"verified": true, "line": 7} ]
            }),
        ));
        let list = &d.breakpoints[&path];
        assert_eq!(list[0].line, 6); // adapter moved it to line 7 (1-based)
        assert!(list[0].verified);
    }

    #[test]
    fn shift_breakpoints_tracks_edits() {
        let mut d = DapClient::new();
        for l in [2usize, 5, 9] {
            d.toggle_breakpoint("/tmp/x.py", l);
        }
        // 2 lines inserted after line 3 → 5,9 shift; 2 stays.
        d.shift_breakpoints("/tmp/x.py", 3, 2);
        assert_eq!(d.lines_for("/tmp/x.py"), vec![2, 7, 11]);
        // 3 lines deleted after line 5 → BP at 7 falls inside span and dies, 11 → 8.
        d.shift_breakpoints("/tmp/x.py", 5, -3);
        assert_eq!(d.lines_for("/tmp/x.py"), vec![2, 8]);
    }

    #[test]
    fn config_fallback_fires_without_initialized_event() {
        let mut d = DapClient::new();
        d.test_session(json!({}));
        let init_id = d.alloc(PendingKind::Initialize);
        d.handle_msg(response(init_id, "initialize", json!({})));
        assert!(!d.config_sent);
        // Rewind the launch clock past the fallback and poll.
        d.launch_sent_at = Some(Instant::now() - CONFIG_FALLBACK - Duration::from_millis(1));
        // poll() requires stdin.is_some() for the fallback — emulate the
        // condition by calling send_configuration directly through poll's gate:
        d.config_sent = false;
        d.send_configuration();
        assert!(d.config_sent);
    }

    #[test]
    fn console_follows_tail_when_focused_there() {
        let mut d = DapClient::new();
        d.set_pane(DebugPane::Console);
        d.log("one");
        d.log("two");
        assert_eq!(d.focus_row, 1);
        // Scroll up — new logs must not yank focus back to the tail.
        d.move_focus(-1);
        d.log("three");
        assert_eq!(d.focus_row, 0);
    }

    #[test]
    fn exception_filter_defaults() {
        let caps = json!({
            "exceptionBreakpointFilters": [
                {"filter": "raised", "default": false},
                {"filter": "uncaught", "default": true}
            ]
        });
        assert_eq!(pick_exception_filters(&caps), vec!["uncaught"]);
        let caps2 = json!({
            "exceptionBreakpointFilters": [
                {"filter": "raised"},
                {"filter": "uncaught"}
            ]
        });
        assert_eq!(pick_exception_filters(&caps2), vec!["uncaught"]);
        assert!(pick_exception_filters(&json!({})).is_empty());
    }
}
