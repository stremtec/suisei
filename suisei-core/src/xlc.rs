
pub struct Xlc {
    pub open: bool,
    pub input: String,
    pub output: Vec<String>,
    pub history: Vec<String>,
    pub history_index: usize,
    pub scroll_offset: usize,
}

impl Default for Xlc {
    fn default() -> Self {
        Self {
            open: false,
            input: String::new(),
            output: vec!["xei Line Command — :help · :screensaver · :pet".to_string()],
            history: Vec::new(),
            history_index: 0,
            scroll_offset: 0,
        }
    }
}

impl Xlc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_panel(&mut self, prompt: Option<&str>) {
        self.open = true;
        self.input.clear();
        self.history_index = self.history.len();
        self.scroll_offset = 0;
        if let Some(p) = prompt {
            self.input = p.to_string();
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.input.clear();
    }

    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn pop_char(&mut self) {
        self.input.pop();
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index > 0 {
            self.history_index -= 1;
            self.input = self.history[self.history_index].clone();
        }
    }

    pub fn history_down(&mut self) {
        if self.history_index < self.history.len() {
            self.history_index += 1;
            if self.history_index < self.history.len() {
                self.input = self.history[self.history_index].clone();
            } else {
                self.input.clear();
            }
        }
    }

    pub fn execute(&mut self) -> XlcCmd {
        let cmd = self.input.trim().to_string();
        if !cmd.is_empty() {
            self.history.push(cmd.clone());
        }
        self.history_index = self.history.len();

        self.output.push(format!("> {}", cmd));

        let result = parse_command(&cmd);
        self.input.clear();
        result
    }

    pub fn add_output(&mut self, msg: &str) {
        self.output.push(msg.to_string());
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self
            .scroll_offset
            .saturating_add(amount)
            .min(self.output.len().saturating_sub(1));
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    #[allow(dead_code)]
    pub fn scroll_reset(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = self.output.len().saturating_sub(1);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }
}

#[derive(Debug)]
pub enum XlcCmd {
    None,
    Save,
    SaveAs(String),
    SaveAndQuit,
    Quit,
    ForceQuit,
    Open(String),
    Move(String),
    Rename(String),
    DeleteFile,
    Pwd,
    Ls,
    Help,
    Search(String),
    Theme(String),
    BufDelete,
    LspStart(String),
    GotoLine(usize),
    Problems,
    /// `:s/.../` or `:%s/.../`
    Substitute(String),
    /// `:Rename new_name` LSP rename
    LspRename(String),
    /// `:preview` pretty document view
    Preview,
    /// `:gh-login` / `:gha` GitHub auth helpers
    GhLogin,
    GhLogout,
    GhStatus,
    GitWorkbench,
    Settings,
    /// `:screensaver` / `:xeifetch` — themed splash + clock + weather
    Screensaver,
    /// `:pet [path]` — load desktop pet GIF (empty = status)
    Pet(String),
    /// DAP debug
    DapStart,
    DapStop,
    DapLaunch(String),
    DapBreakpoint,
    DapPanel,
    DapCondition(String),
    DapLogpoint(String),
    DapConfig(Option<String>),
    DapEval(String),
    DapAttach(String),
    /// Interactive rebase last N commits
    Rebase(usize),
    RebaseAbort,
    RebaseContinue,
    CallHierarchy,
    CodeLens,
    PrReview(u64),
    HooksReload,
    /// `:mbb` — open a fresh blank tab (welcome screen)
    BlankTab,
    /// `:update` — self-update to the latest release
    Update,
    /// `:bench` — run the in-editor self-benchmark
    Bench,
    /// `:status` — toggle the live CPU/MEM/GPU status readout
    StatusMetrics,
    /// `:exttest` — Spike 0: invoke the bundled VSCode test extension
    ExtTest,
    /// `:extapi` — Spike 2: show the extension's vscode.* API-usage histogram
    ExtApi,
    /// `:plugins [install <id>]` — minimal plugin store (Open VSX)
    Plugins(String),
    /// `:webview` — render + show the latest extension webview
    Webview,
}

fn parse_command(input: &str) -> XlcCmd {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

    match cmd {
        "w" | "save" => {
            if arg.is_empty() {
                XlcCmd::Save
            } else {
                XlcCmd::SaveAs(arg.to_string())
            }
        }
        "q" | "quit" => XlcCmd::Quit,
        "q!" | "quit!" => XlcCmd::ForceQuit,
        "wq" | "x" => XlcCmd::SaveAndQuit,
        "e" | "open" if !arg.is_empty() => XlcCmd::Open(arg.to_string()),
        "mv" | "move" if !arg.is_empty() => XlcCmd::Move(arg.to_string()),
        "rename" if !arg.is_empty() => XlcCmd::Rename(arg.to_string()),
        "rm" => XlcCmd::DeleteFile,
        "pwd" => XlcCmd::Pwd,
        "ls" => XlcCmd::Ls,
        "help" | "h" | "?" => XlcCmd::Help,
        "find" | "/" if !arg.is_empty() => XlcCmd::Search(arg.to_string()),
        "theme" => XlcCmd::Theme(arg.to_string()),
        "bd" => XlcCmd::BufDelete,
        "problems" | "diag" => XlcCmd::Problems,
        "preview" | "Preview" => XlcCmd::Preview,
        "gh-login" | "gha" | "ghlogin" => XlcCmd::GhLogin,
        "gh-logout" | "ghlogout" => XlcCmd::GhLogout,
        "gh-status" | "ghstatus" => XlcCmd::GhStatus,
        "git" | "Git" => XlcCmd::GitWorkbench,
        "settings" | "set" | "Settings" => XlcCmd::Settings,
        "screensaver" | "xeifetch" | "fetch" | "ss" => XlcCmd::Screensaver,
        "pet" => XlcCmd::Pet(arg.to_string()),
        "dap" | "debug" if arg.is_empty() => XlcCmd::DapPanel,
        "dap" | "debug" if arg == "start" || arg == "run" => XlcCmd::DapStart,
        "dap" | "debug" if arg == "stop" => XlcCmd::DapStop,
        "dap" | "debug" if arg == "bp" || arg == "break" => XlcCmd::DapBreakpoint,
        "DapLaunch" | "dap-launch" | "launch" if !arg.is_empty() => {
            XlcCmd::DapLaunch(arg.to_string())
        }
        "bp" | "break" if arg.is_empty() => XlcCmd::DapBreakpoint,
        "bp" | "break" if arg.starts_with("if ") => {
            XlcCmd::DapCondition(arg.trim_start_matches("if ").trim().to_string())
        }
        "bp" | "break" if arg.starts_with("log ") => {
            XlcCmd::DapLogpoint(arg.trim_start_matches("log ").trim().to_string())
        }
        "bp" | "break" => XlcCmd::DapBreakpoint,
        "DapConfig" | "dap-config" | "launch-config" if arg.is_empty() => XlcCmd::DapConfig(None),
        "DapConfig" | "dap-config" | "launch-config" => {
            XlcCmd::DapConfig(Some(arg.to_string()))
        }
        "eval" | "DapEval" if !arg.is_empty() => XlcCmd::DapEval(arg.to_string()),
        "DapAttach" | "dap-attach" | "attach" if !arg.is_empty() => {
            XlcCmd::DapAttach(arg.to_string())
        }
        "rebase" => {
            let n = if arg.is_empty() {
                5
            } else {
                arg.parse().unwrap_or(5)
            };
            XlcCmd::Rebase(n)
        }
        "rebase-abort" | "rebase!" => XlcCmd::RebaseAbort,
        "rebase-continue" | "rebase-cont" => XlcCmd::RebaseContinue,
        "calls" | "callers" | "hierarchy" => XlcCmd::CallHierarchy,
        "codelens" | "lens" => XlcCmd::CodeLens,
        "pr" | "review" if !arg.is_empty() => {
            let n = arg.trim_start_matches('#').parse().unwrap_or(0);
            XlcCmd::PrReview(n)
        }
        "hooks" | "hooks-reload" => XlcCmd::HooksReload,
        "update" | "upgrade" => XlcCmd::Update,
        "bench" | "benchmark" => XlcCmd::Bench,
        "status" | "stats" | "metrics" => XlcCmd::StatusMetrics,
        "exttest" | "ext" => XlcCmd::ExtTest,
        "extapi" | "ext-api" => XlcCmd::ExtApi,
        "plugins" | "store" | "plugin" => XlcCmd::Plugins(arg.to_string()),
        "webview" | "wv" => XlcCmd::Webview,
        "mbb" | "new" | "blank" => XlcCmd::BlankTab,
        "lsp" | "LspStart" if !arg.is_empty() => XlcCmd::LspStart(arg.to_string()),
        "Rename" | "rename" if !arg.is_empty() => XlcCmd::LspRename(arg.to_string()),
        other if !other.is_empty() && other.chars().all(|c| c.is_ascii_digit()) => {
            match other.parse::<usize>() {
                Ok(n) => XlcCmd::GotoLine(n),
                Err(_) => XlcCmd::None,
            }
        }
        // `:s/...` or `:%s/...`
        other if other.starts_with("%s") || (other.starts_with('s') && other.len() > 1) => {
            XlcCmd::Substitute(other.to_string())
        }
        _ => XlcCmd::None,
    }
}

// UndoStack moved to crate::undo (delta-based, SSD spill).

