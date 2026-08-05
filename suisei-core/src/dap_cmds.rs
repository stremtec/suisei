//! Debugging (DAP) — breakpoints, launch/attach, stepping, and the
//! stopped-location jump (A3-5 extraction).
//!
//! The protocol client is [`crate::dap::DapClient`] (`App::dap`); this file
//! is everything `App` does on top of it: resolving the program under the
//! cursor, configs from `.vscode/launch.json`, and driving the caret when
//! the debugger stops.

use crate::app::{App, Mode};
use std::path::{Path, PathBuf};

impl App {
    /// F9 — toggle breakpoint on cursor line.
    pub fn dap_toggle_breakpoint(&mut self) {
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            self.message = "No file for breakpoint".into();
            return;
        };
        let line = self.buffer.cursor.row;
        let on = self.dap.toggle_breakpoint(&path, line);
        self.message = if on {
            format!("● Breakpoint L{}", line + 1)
        } else {
            format!("○ Cleared BP L{}", line + 1)
        };
    }
    /// F5 — start or continue.
    pub fn dap_start_or_continue(&mut self) {
        use crate::dap::DapState;
        match self.dap.state {
            DapState::Stopped => {
                self.dap.continue_exec();
                self.message = "→ continue".into();
            }
            DapState::Running | DapState::Starting => {
                self.message = format!("DAP {}", self.dap.state.label());
            }
            DapState::Idle | DapState::Ending => {
                let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
                    self.message = "Open a file to debug".into();
                    return;
                };
                let cwd = self
                    .filename
                    .as_ref()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()));
                let ext = self.file_extension();
                let lang = ext.as_deref().map(|e| match e {
                    "py" | "pyw" => "python",
                    "rs" => "rust",
                    "go" => "go",
                    "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" => "cpp",
                    "js" | "mjs" | "cjs" | "ts" | "tsx" => "node",
                    _ => "unknown",
                });
                let was_closed = !self.dap.panel_open;
                match self.dap.start(&path, cwd.as_deref(), lang, &[]) {
                    Ok(()) => {
                        if was_closed {
                            self.dap.arm_panel_animation();
                        }
                        self.mode = Mode::Debug;
                        self.message = format!(
                            "▶ DAP {} · {}",
                            self.dap.adapter_name,
                            self.dap.last_program.as_deref().unwrap_or(&path)
                        );
                    }
                    Err(e) => {
                        self.message = e;
                    }
                }
            }
        }
    }
    /// Launch a program (XLC `:DapLaunch <path> [args…]`).
    pub fn dap_launch_program(&mut self, program_line: &str) {
        let mut parts = program_line.split_whitespace();
        let Some(program) = parts.next() else {
            self.message = "DapLaunch: missing program".into();
            return;
        };
        let args: Vec<String> = parts.map(|s| s.to_string()).collect();
        let cwd = Path::new(program)
            .parent()
            .map(|p| p.to_path_buf())
            .or_else(|| {
                self.filename
                    .as_ref()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            });
        let was_closed = !self.dap.panel_open;
        match self.dap.start(program, cwd.as_deref(), None, &args) {
            Ok(()) => {
                if was_closed {
                    self.dap.arm_panel_animation();
                }
                self.mode = Mode::Debug;
                self.message = format!("▶ DAP launch {program_line}");
            }
            Err(e) => self.message = e,
        }
    }
    /// F6 — suspend a running program.
    pub fn dap_pause(&mut self) {
        self.dap.pause();
        self.message = "⏸ pause requested".into();
    }
    /// Evaluate expression in the stopped frame (Console REPL).
    pub fn dap_evaluate(&mut self, expr: &str) {
        self.dap.evaluate(expr);
        self.message = format!("eval: {expr}");
    }
    /// `:bp if <expr>` — conditional breakpoint on cursor line.
    pub fn dap_set_condition(&mut self, condition: &str) {
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            self.message = "No file for breakpoint".into();
            return;
        };
        let line = self.buffer.cursor.row;
        let cond = condition.trim();
        if cond.is_empty() {
            self.dap.set_breakpoint_condition(&path, line, None);
            self.message = format!("○ condition cleared L{}", line + 1);
        } else {
            self.dap
                .set_breakpoint_condition(&path, line, Some(cond.to_string()));
            self.message = format!("● L{} if {cond}", line + 1);
        }
    }
    /// `:bp log <msg>` — logpoint on cursor line.
    pub fn dap_set_logpoint(&mut self, msg: &str) {
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            self.message = "No file for logpoint".into();
            return;
        };
        let line = self.buffer.cursor.row;
        let m = msg.trim();
        if m.is_empty() {
            self.dap.set_breakpoint_log(&path, line, None);
            self.message = format!("○ logpoint cleared L{}", line + 1);
        } else {
            self.dap
                .set_breakpoint_log(&path, line, Some(m.to_string()));
            self.message = format!("● L{} log {m}", line + 1);
        }
    }
    /// Launch using a named config from `.vscode/launch.json`.
    pub fn dap_launch_config(&mut self, name: Option<&str>) {
        let hint = self.filename.as_deref();
        let configs = crate::dap::load_launch_configs(hint);
        if configs.is_empty() {
            self.message = "No .vscode/launch.json configurations found".into();
            return;
        }
        let cfg = if let Some(n) = name {
            configs.iter().find(|c| c.name == n)
        } else {
            configs.first()
        };
        let Some(cfg) = cfg else {
            let names: Vec<_> = configs.iter().map(|c| c.name.as_str()).collect();
            self.message = format!("Unknown config. Available: {}", names.join(", "));
            return;
        };
        let was_closed = !self.dap.panel_open;
        let result = if cfg.request == "attach" {
            // Prefer port from env-less configs: look for numeric in program or name
            // launch.json attach often has "port" field — re-parse via args empty + name
            self.dap_attach_from_config(cfg)
        } else {
            if cfg.program.is_empty() {
                self.message = format!("Config '{}' has no program", cfg.name);
                return;
            }
            let cwd = cfg.cwd.as_ref().map(PathBuf::from).or_else(|| {
                self.filename
                    .as_ref()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            });
            let lang = match cfg.adapter_type.as_str() {
                "python" | "debugpy" => Some("python"),
                "go" | "delve" => Some("go"),
                "lldb" | "cppdbg" | "codelldb" => Some("rust"),
                "node" | "pwa-node" => Some("node"),
                _ => None,
            };
            self.dap
                .start(&cfg.program, cwd.as_deref(), lang, &cfg.args)
        };
        match result {
            Ok(()) => {
                if was_closed {
                    self.dap.arm_panel_animation();
                }
                self.mode = Mode::Debug;
                self.message = format!("▶ launch.json · {}", cfg.name);
            }
            Err(e) => self.message = e,
        }
    }
    fn dap_attach_from_config(&mut self, cfg: &crate::dap::LaunchConfig) -> Result<(), String> {
        let lang = match cfg.adapter_type.as_str() {
            "python" | "debugpy" => Some("python"),
            "node" | "pwa-node" => Some("node"),
            "lldb" | "cppdbg" | "codelldb" => Some("native"),
            other if !other.is_empty() => Some(other),
            _ => None,
        };
        if let Some(pid) = cfg.pid {
            return self.dap.attach_pid(pid);
        }
        if let Some(port) = cfg.port {
            return self.dap.attach_port(port, lang, cfg.host.as_deref());
        }
        // Heuristic fallback: program field as port or pid
        if let Some(port) = cfg
            .program
            .parse::<u16>()
            .ok()
            .or_else(|| cfg.program.rsplit(':').next().and_then(|s| s.parse().ok()))
        {
            let host = if cfg.program.contains(':') {
                cfg.program.split(':').next()
            } else {
                None
            };
            return self.dap.attach_port(port, lang, host);
        }
        if let Ok(pid) = cfg.program.parse::<u32>() {
            return self.dap.attach_pid(pid);
        }
        Err(format!(
            "Attach config '{}' needs port, processId/pid, or program=port|pid",
            cfg.name
        ))
    }
    /// `:DapAttach pid <n>` or `:DapAttach port <n> [lang]`
    pub fn dap_attach(&mut self, spec: &str) {
        let parts: Vec<&str> = spec.split_whitespace().collect();
        if parts.is_empty() {
            self.message = "Usage: DapAttach pid <n> | DapAttach port <n> [python|node]".into();
            return;
        }
        let was_closed = !self.dap.panel_open;
        let result = match parts[0] {
            "pid" => {
                let Some(pid) = parts.get(1).and_then(|s| s.parse::<u32>().ok()) else {
                    self.message = "Usage: DapAttach pid <n>".into();
                    return;
                };
                self.dap.attach_pid(pid)
            }
            "port" => {
                let Some(port) = parts.get(1).and_then(|s| s.parse::<u16>().ok()) else {
                    self.message = "Usage: DapAttach port <n> [python|node]".into();
                    return;
                };
                let lang = parts.get(2).copied();
                self.dap.attach_port(port, lang, None)
            }
            // Bare number: prefer port if ≤65535, else pid
            n if n.parse::<u32>().is_ok() => {
                let num: u32 = n.parse().unwrap();
                if num <= 65535 {
                    self.dap.attach_port(num as u16, Some("python"), None)
                } else {
                    self.dap.attach_pid(num)
                }
            }
            _ => {
                self.message = "Usage: DapAttach pid <n> | DapAttach port <n> [lang]".into();
                return;
            }
        };
        match result {
            Ok(()) => {
                if was_closed {
                    self.dap.arm_panel_animation();
                }
                self.mode = Mode::Debug;
                self.message = format!("▶ attach · {spec}");
            }
            Err(e) => self.message = e,
        }
    }
    /// List launch.json configs into message / XLC.
    pub fn dap_list_configs(&mut self) {
        let hint = self.filename.as_deref();
        let configs = crate::dap::load_launch_configs(hint);
        if configs.is_empty() {
            self.message = "No launch.json configs".into();
            self.set_message("No .vscode/launch.json found");
            return;
        }
        self.set_message("=== launch.json ===");
        for c in &configs {
            self.set_message(&format!("  {}  [{}]  {}", c.name, c.request, c.program));
        }
        self.message = format!("{} launch config(s) — :DapConfig <name>", configs.len());
    }
    pub fn dap_stop(&mut self) {
        self.dap.stop();
        self.message = "■ Debug stopped".into();
    }
    pub fn dap_step_over(&mut self) {
        self.dap.step_over();
        self.message = "→ step over".into();
    }
    pub fn dap_step_into(&mut self) {
        self.dap.step_into();
        self.message = "→ step into".into();
    }
    pub fn dap_step_out(&mut self) {
        self.dap.step_out();
        self.message = "→ step out".into();
    }
    /// After DAP poll: jump editor to stopped frame if path matches an openable file.
    pub fn dap_apply_stopped_location(&mut self) {
        if !self.dap.location_dirty {
            return;
        }
        self.dap.location_dirty = false;
        let Some(path) = self.dap.current_path.clone() else {
            return;
        };
        let Some(line) = self.dap.current_line else {
            return;
        };
        // Open / switch to file if needed
        let same = self
            .filename
            .as_ref()
            .map(|p| {
                let a = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                let b = std::fs::canonicalize(&path).unwrap_or_else(|_| PathBuf::from(&path));
                a == b
            })
            .unwrap_or(false);
        if !same && Path::new(&path).is_file() {
            self.open_new_tab(&path);
        }
        if self.buffer.line_count() == 0 {
            return;
        }
        self.buffer.cursor.row = line.min(self.buffer.line_count().saturating_sub(1));
        self.buffer.move_to_line_start();
        self.update_scroll();
    }
}
