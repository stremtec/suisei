//! Limited plugin hooks — run shell commands on editor events.
//!
//! Config: `~/.xei/hooks.toml`
//!
//! ```toml
//! # Placeholders: {file} {path} {dir} {ext} {event}  (shell-quoted automatically)
//! on_save = "echo saved {file}"
//! on_open = ""
//! on_quit = ""
//! enabled = true
//! ```
//!
//! Multiple commands: separate with `;;` (each runs sequentially).
//! Hooks run on a background thread (the editor never blocks); each command is
//! killed after 10s. `on_quit` is fire-and-forget so quitting stays instant.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Per-command wall-clock budget before the hook is killed.
const HOOK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    Save,
    Open,
    Quit,
}

impl HookEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            HookEvent::Save => "save",
            HookEvent::Open => "open",
            HookEvent::Quit => "quit",
        }
    }

    pub fn config_key(self) -> &'static str {
        match self {
            HookEvent::Save => "on_save",
            HookEvent::Open => "on_open",
            HookEvent::Quit => "on_quit",
        }
    }
}

#[derive(Debug, Clone)]
pub struct HooksConfig {
    pub enabled: bool,
    pub on_save: String,
    pub on_open: String,
    pub on_quit: String,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_save: String::new(),
            on_open: String::new(),
            on_quit: String::new(),
        }
    }
}

impl HooksConfig {
    pub fn config_path() -> PathBuf {
        dirs_fallback().join("hooks.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        parse_hooks_toml(&text)
    }

    pub fn for_event(&self, ev: HookEvent) -> &str {
        match ev {
            HookEvent::Save => &self.on_save,
            HookEvent::Open => &self.on_open,
            HookEvent::Quit => &self.on_quit,
        }
    }

    /// True when this event would actually run something.
    pub fn has_hook(&self, ev: HookEvent) -> bool {
        self.enabled && !self.for_event(ev).trim().is_empty()
    }
}

fn dirs_fallback() -> PathBuf {
    if let Some(h) =
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(h).join(".xei");
    }
    PathBuf::from(".xei")
}

fn parse_hooks_toml(text: &str) -> HooksConfig {
    let mut cfg = HooksConfig::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = parse_toml_value(v);
        match k {
            "enabled" => {
                cfg.enabled = matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on");
            }
            "on_save" => cfg.on_save = v,
            "on_open" => cfg.on_open = v,
            "on_quit" => cfg.on_quit = v,
            _ => {}
        }
    }
    cfg
}

/// Quoted value up to the closing quote (inline `# comment` after it ignored);
/// bare value up to the first `#`.
fn parse_toml_value(raw: &str) -> String {
    let v = raw.trim();
    for quote in ['"', '\''] {
        if let Some(rest) = v.strip_prefix(quote) {
            if let Some(end) = rest.find(quote) {
                return rest[..end].to_string();
            }
            return rest.to_string();
        }
    }
    v.split('#').next().unwrap_or("").trim().to_string()
}

/// Single-quote `s` for `sh -c` (`'` → `'\''`), so paths with spaces or quotes
/// survive placeholder substitution.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Expanded commands for `event`: (command line, cwd).
fn expand_commands(
    cfg: &HooksConfig,
    event: HookEvent,
    file: Option<&Path>,
) -> Vec<(String, PathBuf)> {
    if !cfg.has_hook(event) {
        return Vec::new();
    }
    let path = file.map(|p| p.display().to_string()).unwrap_or_default();
    let dir = file
        .and_then(|p| p.parent())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| ".".into())
        });
    let name = file
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let ext = file
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    let cwd = file
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    cfg.for_event(event)
        .split(";;")
        .filter_map(|raw| {
            let cmd = raw
                .trim()
                .replace("{file}", &sh_quote(&name))
                .replace("{path}", &sh_quote(&path))
                .replace("{dir}", &sh_quote(&dir))
                .replace("{ext}", &sh_quote(&ext))
                .replace("{event}", event.as_str());
            if cmd.is_empty() {
                None
            } else {
                Some((cmd, cwd.clone()))
            }
        })
        .collect()
}

/// Run hook commands, waiting up to [`HOOK_TIMEOUT`] each. Returns the last
/// status line. Blocking — call from a background thread.
pub fn run_hooks(
    cfg: &HooksConfig,
    event: HookEvent,
    file: Option<&Path>,
) -> Option<String> {
    let mut last_msg = None;
    for (cmd, cwd) in expand_commands(cfg, event, file) {
        match run_with_timeout(&cmd, &cwd) {
            HookOutcome::Ok(line) => {
                if !line.is_empty() {
                    last_msg = Some(line);
                }
            }
            HookOutcome::Failed(err) => {
                last_msg = Some(format!("hook({}): {err}", event.as_str()));
            }
            HookOutcome::TimedOut => {
                last_msg = Some(format!(
                    "hook({}): timed out ({}s), killed",
                    event.as_str(),
                    HOOK_TIMEOUT.as_secs()
                ));
            }
        }
    }
    last_msg
}

/// Platform shell: `sh -c` on unix, `cmd /C` on Windows (hooks are written
/// for the platform they run on — placeholders stay POSIX-quoted).
fn shell_command(cmd: &str) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    }
}

/// Fire-and-forget: spawn commands without waiting, so quit stays instant.
/// The children keep running after the editor exits.
pub fn run_hooks_detached(cfg: &HooksConfig, event: HookEvent, file: Option<&Path>) {
    for (cmd, cwd) in expand_commands(cfg, event, file) {
        let _ = shell_command(&cmd)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

enum HookOutcome {
    /// First non-empty stdout line ("" when silent)
    Ok(String),
    Failed(String),
    TimedOut,
}

fn run_with_timeout(cmd: &str, cwd: &Path) -> HookOutcome {
    let mut child = match shell_command(cmd)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return HookOutcome::Failed(format!("spawn: {e}")),
    };
    // Drain pipes on side threads so a chatty hook can't deadlock on a full pipe.
    let stdout = child.stdout.take().map(drain_to_string);
    let stderr = child.stderr.take().map(drain_to_string);

    let deadline = Instant::now() + HOOK_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };
    let out = stdout.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    let err = stderr.map(|h| h.join().unwrap_or_default()).unwrap_or_default();

    match status {
        None => HookOutcome::TimedOut,
        Some(s) if s.success() => HookOutcome::Ok(
            out.lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string(),
        ),
        Some(_) => HookOutcome::Failed(
            err.lines()
                .chain(out.lines())
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("hook failed")
                .to_string(),
        ),
    }
}

fn drain_to_string<R: Read + Send + 'static>(mut r: R) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut s = String::new();
        let mut buf = [0u8; 4096];
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => s.push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let t = r#"
enabled = true
on_save = "echo {file}"
on_open = ""
"#;
        let c = parse_hooks_toml(t);
        assert!(c.enabled);
        assert_eq!(c.on_save, "echo {file}");
        assert!(c.on_open.is_empty());
    }

    #[test]
    fn parse_inline_comments_and_quotes() {
        let t = r#"
enabled = true # yes
on_save = "echo hi" # runs on save
on_open = 'echo # not a comment'
on_quit = echo bare # trailing
"#;
        let c = parse_hooks_toml(t);
        assert!(c.enabled);
        assert_eq!(c.on_save, "echo hi");
        assert_eq!(c.on_open, "echo # not a comment");
        assert_eq!(c.on_quit, "echo bare");
    }

    #[test]
    fn placeholders_are_shell_quoted() {
        let cfg = HooksConfig {
            enabled: true,
            on_save: "cat {path}".into(),
            ..Default::default()
        };
        let cmds = expand_commands(&cfg, HookEvent::Save, Some(Path::new("/tmp/a b'c.rs")));
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].0, r#"cat '/tmp/a b'\''c.rs'"#);
    }

    #[test]
    fn sh_quote_roundtrip() {
        assert_eq!(sh_quote("plain"), "'plain'");
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn hook_runs_and_captures_stdout() {
        let cfg = HooksConfig {
            enabled: true,
            on_save: "echo ok {event}".into(),
            ..Default::default()
        };
        let msg = run_hooks(&cfg, HookEvent::Save, Some(Path::new("/tmp/x.rs")));
        assert_eq!(msg.as_deref(), Some("ok save"));
    }

    #[test]
    fn hook_failure_reports_stderr() {
        let cfg = HooksConfig {
            enabled: true,
            on_save: "echo boom >&2; false".into(),
            ..Default::default()
        };
        let msg = run_hooks(&cfg, HookEvent::Save, None);
        assert_eq!(msg.as_deref(), Some("hook(save): boom"));
    }
}
