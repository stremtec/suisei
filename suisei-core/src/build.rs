//! Build & Run — what the compiler said, as something the editor can point at.
//!
//! feature.txt #9 left half of this behind. `dap.rs` has been running
//! `cargo build` since the debugger landed, and when the build failed it wrote
//! ONE line into the debug panel — `first_compile_error`, the first `error:`
//! and its arrow line, joined into a sentence. That sentence names a file and a
//! line number and there is no way to go there. Twenty other errors are thrown
//! away. And there is no way at all to run `cargo test` and read the output.
//!
//! Both halves are the same job: run a command in the project, keep what it
//! said, and turn the parts of it that name a place into places the editor can
//! go. So this module has three pieces and nothing else:
//!
//! · [`plan`] — what "run" MEANS here, decided from the manifest on disk.
//! · [`Build`] — the process, its output, and its problems.
//! · The parsers — one for `cargo`'s JSON, one for the text every other
//!   compiler prints.
//!
//! **A [`Problem`] is a [`Diagnostic`] with a path on it.** That is deliberate:
//! the editor already draws diagnostics, already lists them, already steps
//! through them with `]d`. A build error that arrived as its own kind of thing
//! would need its own squiggle, its own list and its own key, and the reader
//! would have to learn which underline meant which. See `App::diagnostics`,
//! where the two sources become one sequence.
//!
//! **Parsing happens on the main thread, one line at a time.** The reader
//! threads only ferry bytes. That is what lets every test below drive the
//! parser with a string instead of a compiler.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crate::lsp::{Diagnostic, DiagnosticSeverity};

/// Lines of console kept. A `cargo build` of a large workspace prints a few
/// hundred; a test suite prints thousands. The oldest go first, because the
/// end of the output is the part that says how it went.
pub const OUTPUT_CAP: usize = 4000;

/// Problems kept. `cargo` will happily report a thousand warnings from a
/// dependency-free rebuild; nobody reads the thousandth, and every one of them
/// costs a String and a squiggle.
pub const PROBLEM_CAP: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildKind {
    Build,
    Run,
    Test,
}

impl BuildKind {
    pub fn verb(self) -> &'static str {
        match self {
            BuildKind::Build => "Build",
            BuildKind::Run => "Run",
            BuildKind::Test => "Test",
        }
    }

    /// The key a project uses to override this command in `project.suiseiprj`.
    pub fn key(self) -> &'static str {
        match self {
            BuildKind::Build => "build",
            BuildKind::Run => "run",
            BuildKind::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildState {
    /// Nothing has ever run.
    Idle,
    Running,
    Ok,
    Failed,
}

impl BuildState {
    pub fn is_running(self) -> bool {
        self == BuildState::Running
    }
}

/// One thing the compiler complained about, in one place.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// Absolute. Empty when the error names no file at all — a link failure, a
    /// bad flag. Those are still worth showing; they are just not jumpable.
    pub path: String,
    /// 0-based, like every row in this codebase.
    pub row: usize,
    /// 0-based char column. See the note on [`Build::cargo_message`] about what
    /// the compilers actually mean by "column".
    pub col: usize,
    pub col_end: usize,
    pub message: String,
    pub severity: DiagnosticSeverity,
    /// `E0425`, `TS2304`, `unused_variables` — empty when the tool has none.
    pub code: String,
}

impl Problem {
    pub fn is_error(&self) -> bool {
        self.severity == DiagnosticSeverity::Error
    }

    /// The same complaint, in the shape the editor draws.
    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            row: self.row,
            col_start: self.col,
            col_end: self.col_end.max(self.col + 1),
            message: if self.code.is_empty() {
                self.message.clone()
            } else {
                format!("{} [{}]", self.message, self.code)
            },
            severity: self.severity.clone(),
        }
    }
}

/// What to run, decided once, so the runner has no opinions of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub kind: BuildKind,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// `cargo test` — what the header says, and what the log line says.
    pub label: String,
    /// Whether stdout carries `cargo`'s JSON messages.
    pub json: bool,
}

impl Plan {
    /// The command as a person would type it, for the console's first line.
    pub fn command_line(&self) -> String {
        let mut s = self.program.clone();
        for a in &self.args {
            s.push(' ');
            s.push_str(a);
        }
        s
    }
}

/// What Build / Run / Test mean in this directory.
///
/// Manifest first, and the CURRENT FILE only when there is no manifest at all.
/// The other order is tempting — "run the thing I am looking at" — and it makes
/// the command change every time the reader clicks a different tab, which is
/// the one thing a Run button must never do. A project that wants something
/// else says so in `project.suiseiprj`, and that override arrives here as
/// `custom`.
pub fn plan(
    kind: BuildKind,
    root: &Path,
    file: Option<&Path>,
    custom: Option<&str>,
) -> Result<Plan, String> {
    if let Some(line) = custom.map(str::trim).filter(|s| !s.is_empty()) {
        let mut words = line.split_whitespace().map(str::to_string);
        let program = words.next().unwrap_or_default();
        return Ok(Plan {
            kind,
            args: words.collect(),
            cwd: root.to_path_buf(),
            label: line.to_string(),
            json: program == "cargo",
            program,
        });
    }

    let has = |name: &str| root.join(name).exists();
    let mk = |program: &str, args: &[&str], json: bool| {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut label = program.to_string();
        for a in &args {
            // The message-format flag is machinery, not something the reader
            // asked for, so it stays out of the name on screen.
            if a.starts_with("--message-format") {
                continue;
            }
            label.push(' ');
            label.push_str(a);
        }
        Ok(Plan {
            kind,
            program: program.to_string(),
            args,
            cwd: root.to_path_buf(),
            label,
            json,
        })
    };

    if has("Cargo.toml") {
        // `--message-format=json` is the whole reason build errors can become
        // diagnostics: rustc reports the span it underlines, and cargo passes
        // it through untouched. The human text comes back too, in `rendered`,
        // so the console loses nothing by asking for the machine version.
        return match kind {
            BuildKind::Build => mk("cargo", &["build", "--message-format=json"], true),
            BuildKind::Run => mk("cargo", &["run", "--message-format=json"], true),
            BuildKind::Test => mk("cargo", &["test", "--message-format=json"], true),
        };
    }
    if has("Package.swift") {
        return match kind {
            BuildKind::Build => mk("swift", &["build"], false),
            BuildKind::Run => mk("swift", &["run"], false),
            BuildKind::Test => mk("swift", &["test"], false),
        };
    }
    if has("go.mod") {
        return match kind {
            BuildKind::Build => mk("go", &["build", "./..."], false),
            BuildKind::Run => mk("go", &["run", "."], false),
            BuildKind::Test => mk("go", &["test", "./..."], false),
        };
    }
    if has("package.json") {
        return match kind {
            BuildKind::Build => mk("npm", &["run", "build"], false),
            BuildKind::Run => mk("npm", &["start"], false),
            BuildKind::Test => mk("npm", &["test"], false),
        };
    }
    if has("Makefile") || has("makefile") {
        return match kind {
            BuildKind::Build => mk("make", &[], false),
            BuildKind::Run => mk("make", &["run"], false),
            BuildKind::Test => mk("make", &["test"], false),
        };
    }
    if has("pyproject.toml") && kind == BuildKind::Test {
        return mk("python3", &["-m", "pytest"], false);
    }

    // No manifest. A lone file can still be run if its language is one that
    // runs files, and cannot be built at all.
    let ext = file
        .and_then(|f| f.extension())
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let name = file
        .and_then(|f| f.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let interpreter = match ext {
        "py" => Some("python3"),
        "js" | "mjs" | "cjs" => Some("node"),
        "rb" => Some("ruby"),
        "sh" | "bash" | "zsh" => Some("sh"),
        "lua" => Some("lua"),
        "pl" => Some("perl"),
        _ => None,
    };
    match (kind, interpreter) {
        (BuildKind::Run, Some(cmd)) => Ok(Plan {
            kind,
            program: cmd.to_string(),
            args: vec![name.to_string()],
            cwd: file
                .and_then(|f| f.parent())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.to_path_buf()),
            label: format!("{cmd} {name}"),
            json: false,
        }),
        _ => Err(format!(
            "Nothing to {} here — no Cargo.toml, package.json, go.mod, \
             Package.swift or Makefile in {}. Set a command in the project \
             settings.",
            kind.verb().to_lowercase(),
            root.display()
        )),
    }
}

enum Ev {
    Line(String),
    Eof,
}

/// A message the compiler started and has not yet said where.
///
/// `rustc`'s human output puts the sentence on one line and the place on the
/// next, so the parser has to hold the sentence until the arrow arrives.
struct Pending {
    message: String,
    severity: DiagnosticSeverity,
    code: String,
}

/// A python traceback, which names its place several lines before it says what
/// went wrong — and names it several times, innermost last.
struct Traceback {
    path: String,
    row: usize,
}

#[derive(Default)]
pub struct Build {
    /// `cargo test` — what is running, or what ran last.
    pub label: String,
    pub kind: Option<BuildKind>,
    pub cwd: PathBuf,
    pub state: BuildState,
    /// Every line, oldest first, capped at [`OUTPUT_CAP`].
    pub output: VecDeque<String>,
    pub problems: Vec<Problem>,
    /// Bumped whenever `problems` changes, so the merge into the editor's
    /// diagnostics knows when it is stale without comparing the vectors.
    pub revision: u64,
    /// How long the last run took; `None` while one is running.
    pub took: Option<Duration>,
    /// Set when the run ends, so the face can say more than "failed".
    pub exit: Option<i32>,
    /// True when the reader is looking at this rather than the debugger.
    pub open: bool,

    /// Problems past [`PROBLEM_CAP`], counted rather than kept.
    pub dropped: usize,

    started: Option<Instant>,
    child: Option<Child>,
    rx: Option<Receiver<Ev>>,
    eofs: usize,
    json: bool,
    pending: Option<Pending>,
    traceback: Option<Traceback>,
    panic_at: Option<(String, usize, usize)>,
}

impl Default for BuildState {
    fn default() -> Self {
        BuildState::Idle
    }
}

impl Build {
    /// Start a run, replacing whatever was there.
    ///
    /// The previous run is killed rather than queued behind: two builds of the
    /// same target racing over the same `target/` directory is not a thing
    /// anyone wants, and "press Run again" has to mean "run it again now".
    pub fn start(&mut self, plan: &Plan) -> Result<(), String> {
        self.stop();
        let mut cmd = crate::exec::tool(&plan.program);
        cmd.args(&plan.args)
            .current_dir(&plan.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Colour would arrive as escape bytes in the middle of every message.
        // The stripper below handles them anyway — tools forced to colour by a
        // config file exist — but not asking is cheaper than undoing.
        cmd.env("CARGO_TERM_COLOR", "never");
        cmd.env("CLICOLOR", "0");
        cmd.env("NO_COLOR", "1");

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{} not found — is it installed?", plan.program)
            } else {
                format!("{}: {e}", plan.program)
            }
        })?;

        let (tx, rx) = mpsc::channel();
        for pipe in [
            child.stdout.take().map(PipeEnd::Out),
            child.stderr.take().map(PipeEnd::Err),
        ]
        .into_iter()
        .flatten()
        {
            let tx = tx.clone();
            thread::spawn(move || {
                let mut reader: Box<dyn BufRead> = match pipe {
                    PipeEnd::Out(o) => Box::new(BufReader::new(o)),
                    PipeEnd::Err(e) => Box::new(BufReader::new(e)),
                };
                let mut raw = Vec::new();
                loop {
                    raw.clear();
                    // `read_until`, not `lines()`: a program under test can
                    // print anything, and one invalid byte must not end the
                    // output at the line it appeared on.
                    match reader.read_until(b'\n', &mut raw) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let text = String::from_utf8_lossy(&raw).to_string();
                            if tx.send(Ev::Line(text)).is_err() {
                                return;
                            }
                        }
                    }
                }
                let _ = tx.send(Ev::Eof);
            });
        }

        self.label = plan.label.clone();
        self.kind = Some(plan.kind);
        self.cwd = plan.cwd.clone();
        self.state = BuildState::Running;
        self.output.clear();
        self.problems.clear();
        self.revision = self.revision.wrapping_add(1);
        self.took = None;
        self.exit = None;
        self.started = Some(Instant::now());
        self.child = Some(child);
        self.rx = Some(rx);
        self.eofs = 0;
        self.json = plan.json;
        self.pending = None;
        self.traceback = None;
        self.panic_at = None;
        self.dropped = 0;
        self.push_output(format!("$ {}", plan.command_line()));
        Ok(())
    }

    /// Kill whatever is running. Safe to call when nothing is.
    pub fn stop(&mut self) {
        let was_running = self.state.is_running();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.rx = None;
        self.eofs = 0;
        if was_running {
            self.finish_parse();
            self.state = BuildState::Failed;
            self.took = self.started.map(|t| t.elapsed());
            self.push_output("· stopped".into());
        }
    }

    /// Drain what the process said and notice when it ends.
    ///
    /// Called from the same tick that polls the debugger. Bounded on purpose:
    /// a test suite that prints a megabyte must not hold a frame hostage, and
    /// whatever is left arrives on the next one.
    pub fn poll(&mut self) {
        const PER_TICK: usize = 400;
        let Some(rx) = self.rx.take() else {
            return;
        };
        let mut alive = true;
        for _ in 0..PER_TICK {
            match rx.try_recv() {
                Ok(Ev::Line(l)) => self.feed(&l),
                Ok(Ev::Eof) => self.eofs += 1,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    alive = false;
                    break;
                }
            }
        }
        if alive {
            self.rx = Some(rx);
        }

        // Both pipes closed is the honest end of the output. Only then is the
        // exit status worth asking for — reaping first and reading after would
        // report "failed" above lines the reader has not seen yet.
        let done = self.eofs >= 2 || !alive;
        if !done || !self.state.is_running() {
            return;
        }
        let Some(child) = self.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1);
                self.child = None;
                self.rx = None;
                self.finish_parse();
                self.exit = Some(code);
                self.took = self.started.map(|t| t.elapsed());
                self.state = if code == 0 {
                    BuildState::Ok
                } else {
                    BuildState::Failed
                };
                let took = self
                    .took
                    .map(|d| format!(" · {:.1}s", d.as_secs_f32()))
                    .unwrap_or_default();
                self.push_output(if code == 0 {
                    format!("· done{took}")
                } else {
                    format!("· exit {code}{took}")
                });
            }
            Ok(None) => {}
            Err(_) => {
                self.child = None;
                self.state = BuildState::Failed;
            }
        }
    }

    pub fn error_count(&self) -> usize {
        self.problems.iter().filter(|p| p.is_error()).count()
    }

    pub fn warning_count(&self) -> usize {
        self.problems.len() - self.error_count()
    }

    /// The one-line answer, for a status chip that has no room for two.
    pub fn summary(&self) -> String {
        match self.state {
            BuildState::Idle => String::new(),
            BuildState::Running => format!("{}…", self.label),
            _ => {
                let e = self.error_count();
                let w = self.warning_count();
                let mut s = if self.state == BuildState::Ok {
                    format!("{} succeeded", self.label)
                } else {
                    format!("{} failed", self.label)
                };
                if e > 0 {
                    s.push_str(&format!(" · {e} error{}", plural(e)));
                }
                if w > 0 {
                    s.push_str(&format!(" · {w} warning{}", plural(w)));
                }
                s
            }
        }
    }

    /// This file's problems, sorted the way the editor reads them.
    pub fn problems_in(&self, path: &str) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = self
            .problems
            .iter()
            .filter(|p| !p.path.is_empty() && crate::logic::same_file(&p.path, path))
            .map(Problem::diagnostic)
            .collect();
        out.sort_by_key(|d| (d.row, d.col_start));
        out
    }

    // ── Parsing ────────────────────────────────────────────────────────

    /// One line of output, from either pipe.
    pub fn feed(&mut self, raw: &str) {
        let line = strip_ansi(raw.trim_end_matches(['\n', '\r']));
        if self.json && line.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if v.get("reason").is_some() {
                    self.cargo_message(&v);
                    return;
                }
            }
        }
        self.push_output(line.clone());
        self.scan_text(&line);
    }

    /// Flush anything the parser was still holding.
    fn finish_parse(&mut self) {
        self.flush_pending(None);
        self.traceback = None;
        self.panic_at = None;
    }

    /// One `cargo` JSON message.
    ///
    /// Only `compiler-message` says anything; `compiler-artifact` and
    /// `build-script-executed` arrive by the hundred and are noise here. The
    /// `rendered` field is the same text `cargo` would have printed, so the
    /// console reads exactly as it does in a shell — the JSON is for the spans,
    /// not for the display.
    ///
    /// **Columns.** rustc counts them in characters, 1-based, and this codebase
    /// counts from 0 — the only conversion. It is NOT a byte offset, which is
    /// the trap `Position::col` documents elsewhere.
    fn cargo_message(&mut self, v: &serde_json::Value) {
        if v.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            return;
        }
        let Some(msg) = v.get("message") else { return };
        if let Some(rendered) = msg.get("rendered").and_then(|r| r.as_str()) {
            for l in rendered.trim_end().lines() {
                let l = strip_ansi(l);
                self.push_output(l);
            }
        }
        let severity = match msg.get("level").and_then(|l| l.as_str()) {
            Some("error") | Some("error: internal compiler error") => DiagnosticSeverity::Error,
            Some("warning") => DiagnosticSeverity::Warning,
            // `note` and `help` are always attached to an error that was
            // reported on its own; adding them would double every complaint.
            _ => return,
        };
        let text = msg
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        if text.is_empty() || is_summary(&text) {
            return;
        }
        let code = msg
            .get("code")
            .and_then(|c| c.get("code"))
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();

        let spans = msg.get("spans").and_then(|s| s.as_array());
        let span = spans.and_then(|s| {
            s.iter()
                .find(|s| s.get("is_primary").and_then(|p| p.as_bool()) == Some(true))
                .or_else(|| s.first())
        });
        let (path, row, col, col_end) = match span {
            Some(s) => {
                let file = s
                    .get("file_name")
                    .and_then(|f| f.as_str())
                    .unwrap_or_default();
                let row = s.get("line_start").and_then(|l| l.as_u64()).unwrap_or(1) as usize;
                let col = s.get("column_start").and_then(|c| c.as_u64()).unwrap_or(1) as usize;
                let end = s.get("column_end").and_then(|c| c.as_u64()).unwrap_or(0) as usize;
                let same_line =
                    s.get("line_end").and_then(|l| l.as_u64()).unwrap_or(0) as usize == row;
                (
                    self.absolute(file),
                    row.saturating_sub(1),
                    col.saturating_sub(1),
                    if same_line && end > col {
                        end.saturating_sub(1)
                    } else {
                        col
                    },
                )
            }
            None => (String::new(), 0, 0, 0),
        };
        self.push_problem(Problem {
            path,
            row,
            col,
            col_end,
            message: text,
            severity,
            code,
        });
    }

    /// The text every other compiler prints.
    ///
    /// Four shapes, and they are four because those are the ones that exist:
    /// `path:line:col: severity: message` (clang, swiftc, go, eslint),
    /// `path(line,col): error CODE: message` (tsc, MSVC), rustc's own
    /// two-line human form, and a python traceback.
    fn scan_text(&mut self, line: &str) {
        // A rust panic says where before it says what:
        //
        //     thread 'main' panicked at src/main.rs:4:5:
        //     assertion `left == right` failed
        //
        // This is the single most useful thing in a `cargo test` run and it is
        // not a compiler message at all, so nothing above would have caught it.
        if let Some((path, row, col)) = self.panic_at.take() {
            let msg = line.trim();
            if !msg.is_empty() {
                self.push_problem(Problem {
                    path,
                    row,
                    col,
                    col_end: col,
                    message: msg.to_string(),
                    severity: DiagnosticSeverity::Error,
                    code: String::new(),
                });
                return;
            }
        }
        if line.starts_with("thread '") {
            if let Some((_, at)) = line.split_once(" panicked at ") {
                if let Some((path, row, col)) = split_location(at.trim().trim_end_matches(':')) {
                    self.panic_at = Some((
                        self.absolute(&path),
                        row.saturating_sub(1),
                        col.saturating_sub(1),
                    ));
                }
                return;
            }
        }

        // Python: the place comes first and is repeated inwards, and the last
        // line — the only one at column zero — is what actually went wrong.
        if line.trim_start().starts_with("Traceback (most recent call last)") {
            self.traceback = Some(Traceback {
                path: String::new(),
                row: 0,
            });
            return;
        }
        if self.traceback.is_some() {
            if let Some((path, row)) = python_frame(line) {
                let abs = self.absolute(&path);
                self.traceback = Some(Traceback {
                    path: abs,
                    row: row.saturating_sub(1),
                });
                return;
            }
            let unindented = !line.starts_with(' ') && !line.trim().is_empty();
            if unindented {
                let tb = self.traceback.take().expect("checked");
                if !tb.path.is_empty() {
                    self.push_problem(Problem {
                        path: tb.path,
                        row: tb.row,
                        col: 0,
                        col_end: 0,
                        message: line.trim().to_string(),
                        severity: DiagnosticSeverity::Error,
                        code: String::new(),
                    });
                }
            }
            return;
        }

        // rustc's arrow, which belongs to the sentence on the line before it.
        if let Some(rest) = line.trim_start().strip_prefix("--> ") {
            if self.pending.is_some() {
                if let Some((path, row, col)) = split_location(rest.trim()) {
                    let abs = self.absolute(&path);
                    self.flush_pending(Some((abs, row.saturating_sub(1), col.saturating_sub(1))));
                }
            }
            return;
        }

        if let Some((path, row, col, severity, code, message)) = text_problem(line) {
            let abs = self.absolute(&path);
            self.flush_pending(None);
            self.push_problem(Problem {
                path: abs,
                row: row.saturating_sub(1),
                col: col.saturating_sub(1),
                col_end: col.saturating_sub(1),
                message,
                severity,
                code,
            });
            return;
        }

        // A sentence with no place yet. Whatever was pending is finished by it.
        if let Some((severity, code, message)) = bare_severity(line) {
            self.flush_pending(None);
            if !is_summary(&message) {
                self.pending = Some(Pending {
                    message,
                    severity,
                    code,
                });
            }
        }
    }

    /// Settle the held sentence, with a place if one arrived.
    fn flush_pending(&mut self, at: Option<(String, usize, usize)>) {
        let Some(p) = self.pending.take() else { return };
        let (path, row, col) = at.unwrap_or_default();
        self.push_problem(Problem {
            path,
            row,
            col,
            col_end: col,
            message: p.message,
            severity: p.severity,
            code: p.code,
        });
    }

    fn absolute(&self, file: &str) -> String {
        if file.is_empty() {
            return String::new();
        }
        let p = Path::new(file);
        if p.is_absolute() {
            file.to_string()
        } else {
            self.cwd.join(p).display().to_string()
        }
    }

    fn push_output(&mut self, line: String) {
        if self.output.len() >= OUTPUT_CAP {
            self.output.pop_front();
        }
        self.output.push_back(line);
    }

    /// One complaint, once.
    ///
    /// `cargo` reports the same warning for every target that compiles the
    /// file — a lib and its test build report each one twice — and two
    /// identical squiggles on one word is a rendering fault, not information.
    fn push_problem(&mut self, p: Problem) {
        if self.problems.len() >= PROBLEM_CAP {
            self.dropped += 1;
            return;
        }
        if self.problems.iter().any(|q| {
            q.row == p.row && q.col == p.col && q.path == p.path && q.message == p.message
        }) {
            return;
        }
        self.problems.push(p);
        self.revision = self.revision.wrapping_add(1);
    }
}

/// So the two pipes can share one reader body without a generic.
enum PipeEnd {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Sentences that count other sentences.
///
/// "aborting due to 3 previous errors" is what `first_compile_error` used to
/// show — an accurate summary that names nothing. As a problem in a list it
/// would be a row you cannot click sitting above the three you can.
fn is_summary(msg: &str) -> bool {
    const HEADS: [&str; 6] = [
        "aborting due to",
        "could not compile",
        "For more information about",
        "Some errors have detailed",
        "build failed",
        "test failed",
    ];
    let m = msg.trim();
    HEADS.iter().any(|h| m.starts_with(h))
        || (m.ends_with("warning emitted") || m.ends_with("warnings emitted"))
        || (m.ends_with("warning generated") || m.ends_with("warnings generated"))
}

/// `error[E0425]: message`, with nowhere yet.
fn bare_severity(line: &str) -> Option<(DiagnosticSeverity, String, String)> {
    let t = line.trim_start();
    let (head, rest) = t.split_once(": ")?;
    let (word, code) = match head.split_once('[') {
        Some((w, c)) => (w, c.trim_end_matches(']').to_string()),
        None => (head, String::new()),
    };
    let severity = match word {
        "error" | "fatal error" => DiagnosticSeverity::Error,
        "warning" => DiagnosticSeverity::Warning,
        _ => return None,
    };
    let msg = rest.trim();
    if msg.is_empty() {
        return None;
    }
    Some((severity, code, msg.to_string()))
}

/// `path:line:col: severity: message`, and `path(line,col): error CODE: msg`.
///
/// Returns 1-based line and column, as the tools print them.
#[allow(clippy::type_complexity)]
fn text_problem(
    line: &str,
) -> Option<(String, usize, usize, DiagnosticSeverity, String, String)> {
    let t = line.trim_end();
    // tsc / MSVC first: its `(` cannot be confused with anything below.
    if let Some((path, row, col, rest)) = paren_location(t) {
        let (severity, code, message) = severity_prefix(rest)?;
        return Some((path, row, col, severity, code, message));
    }
    let (path, row, col, rest) = split_location_prefix(t)?;
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    let (severity, code, message) = severity_prefix(rest).unwrap_or_else(|| {
        // `go build` prints no severity at all. A compiler that bothered to
        // print a file and a line is not making conversation.
        (DiagnosticSeverity::Error, String::new(), rest.to_string())
    });
    Some((path, row, col, severity, code, message))
}

/// `error TS2304: msg`, `warning: msg`, `error[E0425]: msg` → the parts.
fn severity_prefix(rest: &str) -> Option<(DiagnosticSeverity, String, String)> {
    let (head, tail) = rest.split_once(": ")?;
    let head = head.trim();
    let (word, mut code) = match head.split_once('[') {
        Some((w, c)) => (w.trim(), c.trim_end_matches(']').to_string()),
        None => (head, String::new()),
    };
    // `error TS2304` — the tool's code, said with a space instead of brackets.
    let (word, code) = match word.split_once(' ') {
        Some((w, c)) => {
            code = c.trim().to_string();
            (w, code)
        }
        None => (word, code),
    };
    let severity = match word {
        "error" | "fatal error" => DiagnosticSeverity::Error,
        "warning" => DiagnosticSeverity::Warning,
        "note" | "info" | "help" => DiagnosticSeverity::Info,
        _ => return None,
    };
    let msg = tail.trim();
    if msg.is_empty() {
        return None;
    }
    Some((severity, code, msg.to_string()))
}

/// `path(12,5): rest`
fn paren_location(line: &str) -> Option<(String, usize, usize, &str)> {
    let open = line.find('(')?;
    let close = line[open..].find(')')? + open;
    let inner = &line[open + 1..close];
    let rest = line[close + 1..].strip_prefix(':')?;
    let (row, col) = inner.split_once(',')?;
    let row: usize = row.trim().parse().ok()?;
    let col: usize = col.trim().parse().ok()?;
    let path = line[..open].trim();
    if path.is_empty() {
        return None;
    }
    Some((path.to_string(), row, col, rest))
}

/// `path:12:5: rest` and `path:12: rest` → the parts, 1-based.
fn split_location_prefix(line: &str) -> Option<(String, usize, usize, &str)> {
    // A path can contain a colon, so the first colon is not necessarily the
    // one: try each in turn and take the first that leaves a well-formed
    // remainder behind it.
    let mut from = 0;
    while let Some(rel) = line[from..].find(':') {
        let at = from + rel;
        if let Some(parsed) = parse_after(line, at) {
            return Some(parsed);
        }
        from = at + 1;
        if from >= line.len() {
            break;
        }
    }
    None
}

fn parse_after(line: &str, colon: usize) -> Option<(String, usize, usize, &str)> {
    let path = line[..colon].trim();
    if path.is_empty() || path.contains(' ') {
        // A message that happens to hold `word:12:` is not a location. Paths
        // with spaces exist and are lost here on purpose: guessing wrong puts
        // a squiggle on an innocent line.
        return None;
    }
    // `12:34:56: server started` is a log with a clock in front of it, not a
    // file called 12. A filename made of nothing but digits and colons is not
    // a filename anyone has.
    if path.chars().all(|c| c.is_ascii_digit() || c == ':') {
        return None;
    }
    let rest = &line[colon + 1..];
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let row: usize = rest[..digits].parse().ok()?;
    let after = &rest[digits..];
    // `path:12:5: msg`
    if let Some(tail) = after.strip_prefix(':') {
        let cd = tail.len() - tail.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if cd > 0 {
            let col: usize = tail[..cd].parse().ok()?;
            let tail = &tail[cd..];
            if let Some(msg) = tail.strip_prefix(':') {
                return Some((path.to_string(), row, col, msg));
            }
            // `--> path:12:5` with nothing after it is handled by the caller.
            if tail.is_empty() {
                return Some((path.to_string(), row, col, ""));
            }
            return None;
        }
        // `path:12: msg`
        return Some((path.to_string(), row, 1, tail));
    }
    None
}

/// `path:line:col` on its own — rustc's arrow line.
pub fn split_location(s: &str) -> Option<(String, usize, usize)> {
    let (path, row, col, rest) = split_location_prefix(s)?;
    if !rest.trim().is_empty() {
        return None;
    }
    Some((path, row, col))
}

/// `  File "x.py", line 12, in f`
fn python_frame(line: &str) -> Option<(String, usize)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("File \"")?;
    let (path, rest) = rest.split_once('"')?;
    let rest = rest.strip_prefix(", line ")?;
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let row: usize = rest[..digits].parse().ok()?;
    Some((path.to_string(), row))
}

/// Escape sequences out, text through.
///
/// Not a terminal emulator: this drops CSI and OSC sequences and keeps
/// everything else, which is all a log line needs. The console this feeds is a
/// list of strings, not a screen — `TerminalSurface` is where the emulator is.
pub fn strip_ansi(s: &str) -> String {
    if !s.contains('\u{1b}') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                // CSI: parameters, then one final byte in @..~
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC: ends at BEL or ST.
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_in(dir: &str) -> Build {
        Build {
            cwd: PathBuf::from(dir),
            json: true,
            ..Build::default()
        }
    }

    #[test]
    fn a_cargo_error_becomes_a_place() {
        let mut b = build_in("/proj");
        b.feed(
            r#"{"reason":"compiler-message","message":{"level":"error","message":"cannot find value `x` in this scope","code":{"code":"E0425"},"rendered":"error[E0425]: cannot find value `x`\n --> src/main.rs:3:5\n","spans":[{"file_name":"src/main.rs","line_start":3,"line_end":3,"column_start":5,"column_end":6,"is_primary":true}]}}"#,
        );
        assert_eq!(b.problems.len(), 1);
        let p = &b.problems[0];
        assert_eq!(p.path, "/proj/src/main.rs");
        assert_eq!((p.row, p.col, p.col_end), (2, 4, 5), "0-based");
        assert_eq!(p.code, "E0425");
        assert!(p.is_error());
        // The console shows the text a shell would have shown, not the JSON.
        assert!(b.output.iter().any(|l| l.contains("cannot find value")));
        assert!(!b.output.iter().any(|l| l.starts_with('{')), "no raw JSON");
    }

    #[test]
    fn the_same_warning_from_two_targets_is_one_squiggle() {
        let mut b = build_in("/proj");
        let msg = r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `y`","code":{"code":"unused_variables"},"spans":[{"file_name":"src/lib.rs","line_start":9,"line_end":9,"column_start":9,"column_end":10,"is_primary":true}]}}"#;
        b.feed(msg);
        b.feed(msg);
        assert_eq!(b.problems.len(), 1);
        assert_eq!(b.warning_count(), 1);
        assert_eq!(b.error_count(), 0);
    }

    #[test]
    fn counting_sentences_are_not_problems() {
        let mut b = build_in("/proj");
        b.feed(
            r#"{"reason":"compiler-message","message":{"level":"error","message":"aborting due to 2 previous errors","spans":[]}}"#,
        );
        b.feed(r#"{"reason":"compiler-artifact","target":{"name":"x"}}"#);
        assert!(b.problems.is_empty());
    }

    #[test]
    fn an_error_with_no_span_is_kept_but_points_nowhere() {
        let mut b = build_in("/proj");
        b.feed(
            r#"{"reason":"compiler-message","message":{"level":"error","message":"linking with `cc` failed","spans":[]}}"#,
        );
        assert_eq!(b.problems.len(), 1);
        assert_eq!(b.problems[0].path, "", "nothing to jump to, and it says so");
    }

    #[test]
    fn clang_and_go_and_tsc_are_all_read() {
        let mut b = Build {
            cwd: PathBuf::from("/w"),
            ..Build::default()
        };
        b.feed("src/a.c:12:7: error: use of undeclared identifier 'q'");
        b.feed("main.go:4:2: undefined: fmt.Printl");
        b.feed("src/app.ts(31,9): error TS2304: Cannot find name 'foo'.");
        assert_eq!(b.problems.len(), 3);
        assert_eq!(
            (b.problems[0].path.as_str(), b.problems[0].row, b.problems[0].col),
            ("/w/src/a.c", 11, 6)
        );
        assert_eq!(b.problems[0].message, "use of undeclared identifier 'q'");
        assert_eq!(b.problems[1].message, "undefined: fmt.Printl", "no severity word");
        assert!(b.problems[1].is_error(), "a file and a line is not chat");
        assert_eq!(b.problems[2].code, "TS2304");
        assert_eq!(b.problems[2].row, 30);
    }

    /// rustc's human output, which puts the sentence and the place on two
    /// lines. This is what `rustc` prints for a single file — the path the
    /// debugger takes for a loose `.rs`.
    #[test]
    fn the_arrow_line_belongs_to_the_sentence_above_it() {
        let mut b = Build {
            cwd: PathBuf::from("/w"),
            ..Build::default()
        };
        for l in [
            "error[E0308]: mismatched types",
            " --> test.rs:7:19",
            "  |",
            "7 |     let x: u8 = \"s\";",
            "  |            --   ^^^ expected `u8`",
            "",
            "error: aborting due to 1 previous error",
        ] {
            b.feed(l);
        }
        b.finish_parse();
        assert_eq!(b.problems.len(), 1, "the summary is not a second problem");
        let p = &b.problems[0];
        assert_eq!(p.message, "mismatched types");
        assert_eq!(p.code, "E0308");
        assert_eq!((p.path.as_str(), p.row, p.col), ("/w/test.rs", 6, 18));
    }

    #[test]
    fn a_python_traceback_points_at_the_innermost_frame() {
        let mut b = Build {
            cwd: PathBuf::from("/w"),
            ..Build::default()
        };
        for l in [
            "Traceback (most recent call last):",
            "  File \"main.py\", line 10, in <module>",
            "    go()",
            "  File \"/w/lib/thing.py\", line 3, in go",
            "    return 1 / 0",
            "ZeroDivisionError: division by zero",
        ] {
            b.feed(l);
        }
        assert_eq!(b.problems.len(), 1);
        let p = &b.problems[0];
        assert_eq!(p.path, "/w/lib/thing.py", "where it actually broke");
        assert_eq!(p.row, 2);
        assert_eq!(p.message, "ZeroDivisionError: division by zero");
    }

    /// Ordinary prose that happens to contain a colon and a number is not a
    /// compiler error, and a squiggle on an innocent line is worse than a
    /// missed one.
    #[test]
    fn prose_is_not_mistaken_for_a_location() {
        let mut b = Build::default();
        for l in [
            "running 12 tests",
            "test result: ok. 12 passed; 0 failed",
            "note: run with `RUST_BACKTRACE=1`",
            "Compiling suisei-core v0.1.0 (/Users/a/suisei)",
            "12:34:56 server listening on 8080",
            "   Finished dev [unoptimized] target(s) in 0.42s",
        ] {
            b.feed(l);
        }
        assert!(b.problems.is_empty(), "{:?}", b.problems);
    }

    /// The most useful line in a failing test run, and it is not a compiler
    /// message at all: the panic says WHERE on one line and WHAT on the next.
    #[test]
    fn a_panic_is_a_place_you_can_go_to() {
        let mut b = Build {
            cwd: PathBuf::from("/w"),
            ..Build::default()
        };
        for l in [
            "running 1 test",
            "thread 'tests::adds' panicked at src/lib.rs:31:9:",
            "assertion `left == right` failed",
            "note: run with `RUST_BACKTRACE=1` to display a backtrace",
        ] {
            b.feed(l);
        }
        assert_eq!(b.problems.len(), 1, "{:?}", b.problems);
        let p = &b.problems[0];
        assert_eq!((p.path.as_str(), p.row, p.col), ("/w/src/lib.rs", 30, 8));
        assert_eq!(p.message, "assertion `left == right` failed");
    }

    #[test]
    fn colour_never_reaches_the_console() {
        assert_eq!(strip_ansi("\u{1b}[1;31merror\u{1b}[0m: bad"), "error: bad");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}plain"), "plain");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
    }

    #[test]
    fn the_console_forgets_the_oldest_line_first() {
        let mut b = Build::default();
        for i in 0..OUTPUT_CAP + 10 {
            b.feed(&format!("line {i}"));
        }
        assert_eq!(b.output.len(), OUTPUT_CAP);
        assert_eq!(b.output.back().unwrap(), &format!("line {}", OUTPUT_CAP + 9));
    }

    #[test]
    fn a_cargo_project_builds_with_cargo_and_a_lone_script_runs_itself() {
        let d = std::env::temp_dir().join("suisei_build_plan");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();

        let lone = d.join("s.py");
        let p = plan(BuildKind::Run, &d, Some(&lone), None).expect("a script runs");
        assert_eq!(p.program, "python3");
        assert_eq!(p.args, vec!["s.py".to_string()]);
        assert!(plan(BuildKind::Build, &d, Some(&lone), None).is_err(), "nothing to build");

        std::fs::write(d.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let p = plan(BuildKind::Test, &d, Some(&lone), None).expect("a project tests");
        assert_eq!(p.program, "cargo");
        assert!(p.json, "so the errors have spans");
        assert_eq!(p.label, "cargo test", "the flag is machinery");

        // And a project that says otherwise is obeyed.
        let p = plan(BuildKind::Run, &d, None, Some("just serve --port 3000")).unwrap();
        assert_eq!(p.program, "just");
        assert_eq!(p.args, ["serve", "--port", "3000"]);
        assert_eq!(p.label, "just serve --port 3000");
    }
}
