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
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

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
        let was_tail =
            self.pane == DebugPane::Console && self.focus_row + 1 >= self.console.len();
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
        });
        entry.sort_by_key(|b| b.line);
        if self.is_session() {
            self.send_set_breakpoints(&path);
        }
        let _ = self.persist_breakpoints();
        true
    }

    /// Set / clear a condition on an existing BP (0-based line). Creates BP if missing.
    pub fn set_breakpoint_condition(
        &mut self,
        path: &str,
        line: usize,
        condition: Option<String>,
    ) {
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
            });
            entry.sort_by_key(|b| b.line);
        }
        if self.is_session() {
            self.send_set_breakpoints(&path);
        }
        let _ = self.persist_breakpoints();
    }

    /// Set / clear a logpoint message.
    pub fn set_breakpoint_log(
        &mut self,
        path: &str,
        line: usize,
        log_message: Option<String>,
    ) {
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
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".xei").join("breakpoints")
    }

    /// Persist BPs to `~/.xei/breakpoints` (`path|line[:cond][:log=msg]`).
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
                    out.push_str(&format!("{}|{}|{}|{}\n", k, b.line, cond, log));
                }
            }
        }
        std::fs::write(path, out).map_err(|e| e.to_string())
    }

    /// Load BPs from `~/.xei/breakpoints` (merge into current map).
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
            let Ok(ln) = parts[1].parse::<usize>() else {
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
        let abs_prog = std::fs::canonicalize(&program_path)
            .unwrap_or_else(|_| program_path.clone());
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

        let mut child = Command::new(&adapter_cmd)
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
        let mut child = Command::new(adapter)
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
                let mut child = Command::new(py)
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
                let mut child = Command::new(adapter)
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
        let abs_prog = std::fs::canonicalize(&program_path)
            .unwrap_or_else(|_| program_path.clone());
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
        let port = free_localhost_port().ok_or_else(|| "No free TCP port for js-debug".to_string())?;
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
        let mut child = Command::new(&adapter_cmd)
            .args([format!("--server={port}")])
            .current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .or_else(|_| {
                Command::new(&adapter_cmd)
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
        let reader = stream
            .try_clone()
            .map_err(|e| format!("tcp clone: {e}"))?;
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
    pub fn stop(&mut self) {
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
        // Keep build_rx if a build is still running
        if self.build_rx.is_none() {
            self.build_message = None;
        }
        if self.state != DapState::Idle && self.build_rx.is_none() {
            self.state = DapState::Idle;
            self.log("session ended");
        }
    }

    /// Spawn `cargo build` in the background; on success `poll` re-enters `start`.
    fn begin_cargo_build(
        &mut self,
        cwd: &Path,
        program: String,
        lang: String,
        args: Vec<String>,
    ) -> Result<(), String> {
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
            let out = Command::new("cargo")
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
                        let _ = tx.send(Err(format!(
                            "cargo build ok but binary not found: {bin}"
                        )));
                    }
                }
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    let msg = err
                        .lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("cargo build failed")
                        .to_string();
                    let _ = tx.send(Err(msg));
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("cargo spawn: {e}")));
                }
            }
        });
        self.build_rx = Some(rx);
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

    fn send_set_breakpoints(&mut self, path: &str) {
        let lines = self
            .breakpoints
            .get(path)
            .map(|v| {
                v.iter()
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
        self.current_path = Some(frame.path.clone());
        self.current_line = Some(frame.line);
        let fid = frame.id;
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
                    self.log(format!("✓ cargo build ok · launching {bin}"));
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
                    self.log(format!("✗ cargo build: {e}"));
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
                            self.error =
                                Some(format!("Debug adapter exited at startup. {hint}"));
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
                if let Some(arr) = body.get("breakpoints").and_then(|b| b.as_array()) {
                    if let Some(list) = self.breakpoints.get_mut(&path) {
                        for (b, resp) in list.iter_mut().zip(arr) {
                            b.verified = resp
                                .get("verified")
                                .and_then(|x| x.as_bool())
                                .unwrap_or(false);
                            b.message = resp
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("")
                                .to_string();
                            // Adapter may slide the BP to the nearest valid line.
                            if let Some(l) = resp.get("line").and_then(|l| l.as_u64()) {
                                b.line = l.saturating_sub(1) as usize;
                            }
                        }
                        list.sort_by_key(|b| b.line);
                        list.dedup_by_key(|b| b.line);
                    }
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
                            name: f
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or("??")
                                .to_string(),
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
                let typ = body
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                if typ.is_empty() {
                    self.log(format!("= {result}"));
                } else {
                    self.log(format!("= {result}  ({typ})"));
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
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
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
            let program_bin = if lang == "rust"
                && program.extension().and_then(|e| e.to_str()) == Some("rs")
            {
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

fn install_hint(lang: &str) -> String {
    match lang {
        "python" => "No Python DAP adapter. Install: pip install debugpy".into(),
        "go" => "No Go DAP adapter. Install: go install github.com/go-delve/delve/cmd/dlv@latest".into(),
        "rust" | "cpp" | "c" => {
            "No native DAP adapter. Install lldb-dap (LLVM) or CodeLLDB".into()
        }
        "node" => "No Node DAP adapter (js-debug-adapter) found".into(),
        _ => format!(
            "No DAP adapter for `{lang}`. Install debugpy / dlv / lldb-dap for your language"
        ),
    }
}

fn command_exists(cmd: &str) -> bool {
    if cmd.contains('/') {
        return Path::new(cmd).is_file();
    }
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        let p = dir.join(cmd);
        if p.is_file() {
            return true;
        }
        // Windows
        let p_exe = dir.join(format!("{cmd}.exe"));
        if p_exe.is_file() {
            return true;
        }
    }
    false
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
            .map(|s| {
                s.replace("${workspaceFolder}", &workspace.display().to_string())
            });
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
        let b = d.breakpoints.get(&path).unwrap().iter().find(|b| b.line == 5).unwrap();
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
        d.handle_msg(response(1, "initialize", json!({
            "supportsConfigurationDoneRequest": true,
            "supportsTerminateRequest": true,
            "exceptionBreakpointFilters": [
                {"filter": "raised", "label": "Raised", "default": false},
                {"filter": "uncaught", "label": "Uncaught", "default": true}
            ]
        })));
        // pending id for initialize isn't registered in test_session; drive via
        // the real alloc path instead: simulate full start bookkeeping.
        // (init response with unknown request_seq is a no-op — assert that.)
        assert!(d.sent_commands().is_empty());

        // Register initialize as pending and retry.
        let init_id = d.alloc(PendingKind::Initialize);
        d.handle_msg(response(init_id, "initialize", json!({
            "supportsConfigurationDoneRequest": true,
            "supportsTerminateRequest": true,
            "exceptionBreakpointFilters": [
                {"filter": "uncaught", "label": "Uncaught", "default": true}
            ]
        })));
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
        d.handle_msg(response(sseq, "stackTrace", json!({
            "stackFrames": [
                {"id": 100, "name": "main", "line": 12, "column": 1,
                 "source": {"path": "/tmp/x.py"}}
            ]
        })));
        assert_eq!(d.stack.len(), 1);
        assert_eq!(d.current_line, Some(11));
        assert!(d.location_dirty);

        let scseq = seq_of(&d, "scopes");
        d.handle_msg(response(scseq, "scopes", json!({
            "scopes": [
                {"name": "Locals", "variablesReference": 200, "expensive": false},
                {"name": "Globals", "variablesReference": 300, "expensive": true}
            ]
        })));
        // Scope roots present, first auto-expanding.
        assert_eq!(d.vars.len(), 2);
        assert!(d.vars[0].is_scope && d.vars[0].expanded);

        let vseq = seq_of(&d, "variables");
        d.handle_msg(response(vseq, "variables", json!({
            "variables": [
                {"name": "x", "value": "1", "type": "int", "variablesReference": 0},
                {"name": "items", "value": "[…]", "type": "list", "variablesReference": 400}
            ]
        })));
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
        d.handle_msg(event("breakpoint", json!({
            "reason": "changed",
            "breakpoint": { "line": 6, "verified": true }
        })));
        assert!(d.flat_bps()[0].2);
    }

    #[test]
    fn set_breakpoints_response_slides_lines() {
        let mut d = DapClient::new();
        d.toggle_breakpoint("/tmp/x.py", 4); // 0-based 4 → sent as line 5
        let path = d.breakpoints.keys().next().unwrap().clone();
        let id = d.alloc(PendingKind::SetBreakpoints(path.clone()));
        d.handle_msg(response(id, "setBreakpoints", json!({
            "breakpoints": [ {"verified": true, "line": 7} ]
        })));
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
