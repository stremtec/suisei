//! Minimal but stable LSP client over stdio.
//!
//! Stability goals:
//! - absolute `file://` URIs
//! - proper initialize → initialized → didOpen order
//! - document version bump on didChange
//! - route responses by request id
//! - clear diagnostics on empty publish
//! - project root detection (Cargo.toml, package.json, …)
//! - binary presence check before spawn

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use crate::config;
use crate::highlight::{self, TokenKind};

/// Semantic highlight span: (kind, start_col, end_col, row) — **char** columns.
pub type SemanticToken = (TokenKind, usize, usize, usize);

pub struct LspClient {
    stdin: Option<ChildStdin>,
    rx: Option<Receiver<RawMsg>>,
    _child: Option<Child>,
    next_id: u64,
    /// id → request kind for routing results
    pending: HashMap<u64, PendingReq>,
    doc_version: i64,
    pub diagnostics: Vec<Diagnostic>,
    pub server_running: bool,
    pub server_name: String,
    pub server_lang: String,
    pub initialized: bool,
    pending_didopen: Option<(String, String, String)>, // path, lang, escaped text
    pub pending_definition: Option<Location>,
    /// When true, next definition result feeds peek instead of jump.
    pub definition_as_peek: bool,
    pub pending_completions: Vec<CompletionItem>,
    pub pending_hover: Option<String>,
    pub pending_references: Vec<Location>,
    /// Legacy single-string edit message (status / no-op notes).
    pub pending_workspace_edit: Option<String>,
    /// Multi-file full-text edits ready to apply (path → new content).
    pub pending_edits: Vec<FileEdit>,
    pub pending_symbols: Vec<SymbolItem>,
    pub pending_code_actions: Vec<CodeActionItem>,
    /// Call hierarchy items ready for the panel (after prepare + incoming/outgoing).
    pub pending_call_hierarchy: Vec<crate::call_hierarchy::CallItem>,
    /// Direction last requested (for UI).
    pub pending_call_direction: Option<crate::call_hierarchy::CallDirection>,
    pub call_hierarchy_ready: bool,
    pub inlay_hints: Vec<InlayHint>,
    pub inlay_supported: bool,
    inlay_dirty: bool,
    pub code_lenses: Vec<CodeLens>,
    pub code_lens_supported: bool,
    code_lens_dirty: bool,
    /// Bumped per codeLens response; stale resolve replies are ignored.
    code_lens_gen: u64,
    /// Hard failure (init crash, disconnect). Status shows `LSP:err`.
    pub error: Option<String>,
    /// Soft notice (binary missing, method unsupported). Status shows dim hint.
    pub soft_error: Option<String>,
    /// Last few stderr lines from the server (debug / soft_error detail).
    pub stderr_tail: String,
    current_uri: String,
    /// URI we last didOpen (for didClose on switch).
    opened_uri: String,
    root_uri: String,
    /// Legend from initialize (token type names)
    semantic_token_types: Vec<String>,
    pub semantic_tokens_supported: bool,
    /// Decoded semantic tokens for the current document (char columns)
    pub semantic_tokens: Vec<SemanticToken>,
    /// Full document text at last semantic-token decode (for UTF-16 → char)
    semantic_doc_text: String,
    /// Re-request semantic tokens after didChange / didOpen
    semantic_dirty: bool,
    last_semantic_req_version: i64,
    /// Master switch from config.
    pub enabled: bool,
    /// Per-language command overrides (from ~/.xei.toml `lsp.*`).
    pub server_overrides: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SymbolItem {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub row: usize,
    pub col: usize,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct InlayHint {
    pub row: usize,
    pub col: usize, // char column
    pub label: String,
}

/// Virtual text above / on a line from `textDocument/codeLens`.
#[derive(Debug, Clone)]
pub struct CodeLens {
    pub row: usize,
    pub col: usize,
    pub title: String,
}

/// One file's new full text after applying a WorkspaceEdit / format / code action.
#[derive(Debug, Clone)]
pub struct FileEdit {
    pub path: String,
    pub text: String,
}

/// LSP code action / quickfix entry.
#[derive(Debug, Clone)]
pub struct CodeActionItem {
    pub title: String,
    pub kind: String,
    pub edits: Vec<FileEdit>,
    /// Optional command id (best-effort execute via workspace/executeCommand).
    pub command: Option<String>,
    pub command_args_json: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum PendingReq {
    Definition,
    Completion,
    Hover,
    References,
    Rename,
    Initialize,
    SemanticTokens,
    DocumentSymbol,
    WorkspaceSymbol,
    InlayHint,
    Formatting,
    CodeAction,
    ExecuteCommand,
    PrepareCallHierarchy,
    IncomingCalls,
    OutgoingCalls,
    CodeLens,
    /// codeLens/resolve — tagged with the generation of its codeLens response
    /// so stale resolves (after an edit re-request) are dropped.
    CodeLensResolve(u64),
}

struct RawMsg {
    id: Option<u64>,
    method: Option<String>,
    body: String,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
    pub message: String,
    pub severity: DiagnosticSeverity,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone)]
pub struct Location {
    pub path: String,
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
}

impl Default for LspClient {
    fn default() -> Self {
        Self {
            stdin: None,
            rx: None,
            _child: None,
            next_id: 1,
            pending: HashMap::new(),
            doc_version: 1,
            diagnostics: Vec::new(),
            server_running: false,
            server_name: String::new(),
            server_lang: String::new(),
            initialized: false,
            pending_didopen: None,
            pending_definition: None,
            definition_as_peek: false,
            pending_completions: Vec::new(),
            pending_hover: None,
            pending_references: Vec::new(),
            pending_workspace_edit: None,
            pending_edits: Vec::new(),
            pending_symbols: Vec::new(),
            pending_code_actions: Vec::new(),
            pending_call_hierarchy: Vec::new(),
            pending_call_direction: None,
            call_hierarchy_ready: false,
            inlay_hints: Vec::new(),
            inlay_supported: false,
            inlay_dirty: false,
            code_lenses: Vec::new(),
            code_lens_supported: true, // probe via first request
            code_lens_dirty: false,
            code_lens_gen: 0,
            error: None,
            soft_error: None,
            stderr_tail: String::new(),
            current_uri: String::new(),
            opened_uri: String::new(),
            root_uri: String::new(),
            semantic_token_types: Vec::new(),
            semantic_tokens_supported: false,
            semantic_tokens: Vec::new(),
            semantic_doc_text: String::new(),
            semantic_dirty: false,
            last_semantic_req_version: 0,
            enabled: true,
            server_overrides: HashMap::new(),
        }
    }
}

impl LspClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, cmd: &str, root: &str, file_path: &str) {
        self.start_with_text(cmd, root, file_path, None);
    }

    pub fn start_with_text(
        &mut self,
        cmd: &str,
        root: &str,
        file_path: &str,
        text_override: Option<&str>,
    ) {
        self.shutdown_quiet();

        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        if !command_exists(parts[0]) {
            // Soft: missing optional binary is not a hard error
            self.soft_error = Some(install_hint(parts[0]));
            self.error = None;
            self.server_running = false;
            return;
        }

        let abs_file = abs_path(file_path);
        let abs_root = abs_path(root);
        let root = find_project_root(&abs_root, &abs_file);

        let mut child = match Command::new(parts[0])
            .args(&parts[1..])
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(format!("LSP failed to start `{}`: {}", parts[0], e));
                return;
            }
        };

        let stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                self.error = Some("LSP stdin unavailable".into());
                return;
            }
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                self.error = Some("LSP stdout unavailable".into());
                return;
            }
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || read_loop(stdout, tx));

        self.stdin = Some(stdin);
        self.rx = Some(rx);
        self._child = Some(child);
        self.server_name = parts[0].to_string();
        self.current_uri = path_to_uri(&abs_file);
        self.root_uri = path_to_uri(&root);
        self.doc_version = 1;
        self.initialized = false;
        self.server_running = false; // true after initialize result
        self.error = None;
        self.soft_error = None;
        self.stderr_tail.clear();
        self.diagnostics.clear();
        self.pending.clear();
        self.opened_uri.clear();

        let id = self.alloc_id(PendingReq::Initialize);
        let pid = std::process::id();
        let folder_name = Path::new(&root)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace");
        // Build initialize carefully — a single malformed brace kills the server
        // and surfaces as sticky LSP:err (disconnected).
        let init = build_initialize_request(
            id,
            pid,
            &self.root_uri,
            folder_name,
        );
        self.send_raw(&init);

        let text = text_override
            .map(|s| s.to_string())
            .unwrap_or_else(|| std::fs::read_to_string(&abs_file).unwrap_or_default());
        let lang = lang_id(&abs_file);
        self.semantic_doc_text = text.clone();
        self.pending_didopen = Some((
            abs_file.clone(),
            lang.to_string(),
            escape_json(&text),
        ));
    }

    pub fn auto_start(&mut self, file_path: &str) {
        self.auto_start_with_text(file_path, None);
    }

    /// Apply config LSP settings (enabled + per-language commands).
    pub fn apply_config(&mut self, enabled: bool, overrides: HashMap<String, String>) {
        self.enabled = enabled;
        self.server_overrides = overrides;
        if !enabled && (self.server_running || self.stdin.is_some()) {
            self.shutdown_quiet();
            self.soft_error = Some("LSP disabled in settings".into());
        }
    }

    /// Resolve command for an extension, honoring overrides + catalog.
    pub fn resolve_server_cmd(&self, ext: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let lang = ext_to_lang_key(ext)?;
        if let Some(cmd) = self.server_overrides.get(lang) {
            if cmd.is_empty() {
                return None; // explicitly off
            }
            return Some(cmd.clone());
        }
        default_server_for_ext(ext).map(|s| s.to_string())
    }

    /// Like [`auto_start`] but opens with live buffer text (avoids stale disk).
    pub fn auto_start_with_text(&mut self, file_path: &str, text: Option<&str>) {
        let abs = abs_path(file_path);
        let ext = Path::new(&abs)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let Some(cmd) = self.resolve_server_cmd(&ext) else {
            // No server for this file type — clear paint state, keep soft notes.
            self.diagnostics.clear();
            self.semantic_tokens.clear();
            self.inlay_hints.clear();
            // Don't keep hard err from previous language when browsing md/txt
            if !self.server_running {
                self.error = None;
            }
            return;
        };

        // Same language + already running → re-open with live text
        if self.server_running && self.server_lang == ext {
            self.open_document_with_text(&abs, text);
            return;
        }

        if self.server_running || self.stdin.is_some() {
            self.shutdown_quiet();
        }

        let root = Path::new(&abs)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".into());
        self.start_with_text(&cmd, &root, &abs, text);
        self.server_lang = ext;
    }

    /// didOpen / re-open current file on an already running server (disk text).
    pub fn open_document(&mut self, file_path: &str) {
        self.open_document_with_text(file_path, None);
    }

    /// didOpen with optional live buffer contents.
    pub fn open_document_with_text(&mut self, file_path: &str, text: Option<&str>) {
        if !self.server_running {
            return;
        }
        let abs = abs_path(file_path);
        let new_uri = path_to_uri(&abs);
        // Close previous document if switching files
        if !self.opened_uri.is_empty() && self.opened_uri != new_uri {
            let close = format!(
                r#"{{"jsonrpc":"2.0","method":"textDocument/didClose","params":{{"textDocument":{{"uri":"{}"}}}}}}"#,
                escape_json(&self.opened_uri)
            );
            self.send_raw(&close);
        }
        let text = text
            .map(|s| s.to_string())
            .unwrap_or_else(|| std::fs::read_to_string(&abs).unwrap_or_default());
        let lang = lang_id(&abs);
        self.current_uri = new_uri.clone();
        self.opened_uri = new_uri;
        self.doc_version = 1;
        self.diagnostics.clear();
        self.semantic_tokens.clear();
        self.inlay_hints.clear();
        self.code_lenses.clear();
        self.semantic_doc_text = text.clone();
        let msg = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{}","languageId":"{}","version":{},"text":"{}"}}}}}}"#,
            escape_json(&self.current_uri),
            lang,
            self.doc_version,
            escape_json(&text)
        );
        self.send_raw(&msg);
        self.semantic_dirty = true;
        self.maybe_request_semantic_tokens();
        self.inlay_dirty = true;
        self.code_lens_dirty = true;
    }

    pub fn notify_change(&mut self, path: &str, text: &str) {
        if !self.server_running {
            return;
        }
        let uri = path_to_uri(&abs_path(path));
        self.current_uri = uri.clone();
        self.doc_version = self.doc_version.saturating_add(1);
        self.semantic_doc_text = text.to_string();
        let msg = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{}","version":{}}},"contentChanges":[{{"text":"{}"}}]}}}}"#,
            escape_json(&uri),
            self.doc_version,
            escape_json(text)
        );
        self.send_raw(&msg);
        self.semantic_dirty = true;
        self.maybe_request_semantic_tokens();
        self.inlay_dirty = true;
        self.code_lens_dirty = true;
    }

    /// Request full semantic tokens if the server advertises support.
    pub fn request_semantic_tokens(&mut self) {
        if !self.server_running || !self.semantic_tokens_supported {
            return;
        }
        // Avoid flooding: one in-flight request per version
        if self
            .pending
            .values()
            .any(|k| matches!(k, PendingReq::SemanticTokens))
        {
            self.semantic_dirty = true;
            return;
        }
        let id = self.alloc_id(PendingReq::SemanticTokens);
        self.last_semantic_req_version = self.doc_version;
        self.semantic_dirty = false;
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/semanticTokens/full","params":{{"textDocument":{{"uri":"{}"}}}}}}"#,
            id,
            escape_json(&self.current_uri)
        );
        self.send_raw(&msg);
    }

    fn maybe_request_semantic_tokens(&mut self) {
        if self.semantic_dirty {
            self.request_semantic_tokens();
        }
    }

    pub fn request_definition(&mut self, path: &str, row: usize, col: usize) {
        self.definition_as_peek = false;
        self.request_position(PendingReq::Definition, "textDocument/definition", path, row, col);
    }

    pub fn request_peek_definition(&mut self, path: &str, row: usize, col: usize) {
        self.definition_as_peek = true;
        self.request_position(PendingReq::Definition, "textDocument/definition", path, row, col);
    }

    pub fn request_document_symbols(&mut self, path: &str) {
        if !self.server_running {
            return;
        }
        let uri = path_to_uri(&abs_path(path));
        let id = self.alloc_id(PendingReq::DocumentSymbol);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/documentSymbol","params":{{"textDocument":{{"uri":"{}"}}}}}}"#,
            id,
            escape_json(&uri)
        );
        self.send_raw(&msg);
    }

    pub fn request_workspace_symbols(&mut self, query: &str) {
        if !self.server_running {
            return;
        }
        let id = self.alloc_id(PendingReq::WorkspaceSymbol);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"workspace/symbol","params":{{"query":"{}"}}}}"#,
            id,
            escape_json(query)
        );
        self.send_raw(&msg);
    }

    pub fn request_inlay_hints(&mut self, path: &str, end_row: usize) {
        if !self.server_running || !self.inlay_supported {
            return;
        }
        let uri = path_to_uri(&abs_path(path));
        let id = self.alloc_id(PendingReq::InlayHint);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/inlayHint","params":{{"textDocument":{{"uri":"{}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":{},"character":0}}}}}}}}"#,
            id,
            escape_json(&uri),
            end_row.saturating_add(1)
        );
        self.send_raw(&msg);
        self.inlay_dirty = false;
    }

    pub fn mark_inlay_dirty(&mut self) {
        self.inlay_dirty = true;
    }

    pub fn maybe_request_inlays(&mut self, path: &str, end_row: usize) {
        if self.inlay_dirty && self.inlay_supported && self.server_running {
            self.request_inlay_hints(path, end_row);
        }
    }

    pub fn request_code_lens(&mut self, path: &str) {
        if !self.server_running || !self.code_lens_supported {
            return;
        }
        // Coalesce
        if self
            .pending
            .values()
            .any(|k| matches!(k, PendingReq::CodeLens))
        {
            return;
        }
        let uri = path_to_uri(&abs_path(path));
        let id = self.alloc_id(PendingReq::CodeLens);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/codeLens","params":{{"textDocument":{{"uri":"{}"}}}}}}"#,
            id,
            escape_json(&uri)
        );
        self.send_raw(&msg);
        self.code_lens_dirty = false;
    }

    pub fn mark_code_lens_dirty(&mut self) {
        self.code_lens_dirty = true;
    }

    pub fn maybe_request_code_lens(&mut self, path: &str) {
        if self.code_lens_dirty && self.code_lens_supported && self.server_running {
            self.request_code_lens(path);
        }
    }

    pub fn request_completion(&mut self, path: &str, row: usize, col: usize) {
        self.request_position(PendingReq::Completion, "textDocument/completion", path, row, col);
    }

    pub fn request_hover(&mut self, path: &str, row: usize, col: usize) {
        self.request_position(PendingReq::Hover, "textDocument/hover", path, row, col);
    }

    pub fn request_references(&mut self, path: &str, row: usize, col: usize) {
        if !self.server_running {
            return;
        }
        let uri = path_to_uri(&abs_path(path));
        let col16 = self.char_col_to_utf16(row, col);
        let id = self.alloc_id(PendingReq::References);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/references","params":{{"textDocument":{{"uri":"{}"}},"position":{{"line":{},"character":{}}},"context":{{"includeDeclaration":true}}}}}}"#,
            id,
            escape_json(&uri),
            row,
            col16
        );
        self.send_raw(&msg);
    }

    pub fn request_rename(&mut self, path: &str, row: usize, col: usize, new_name: &str) {
        if !self.server_running {
            return;
        }
        let uri = path_to_uri(&abs_path(path));
        let col16 = self.char_col_to_utf16(row, col);
        let id = self.alloc_id(PendingReq::Rename);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/rename","params":{{"textDocument":{{"uri":"{}"}},"position":{{"line":{},"character":{}}},"newName":"{}"}}}}"#,
            id,
            escape_json(&uri),
            row,
            col16,
            escape_json(new_name)
        );
        self.send_raw(&msg);
    }

    /// Request document formatting; result applied via pending_edits.
    pub fn request_formatting(&mut self, path: &str) {
        if !self.server_running {
            return;
        }
        let uri = path_to_uri(&abs_path(path));
        let id = self.alloc_id(PendingReq::Formatting);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/formatting","params":{{"textDocument":{{"uri":"{}"}},"options":{{"tabSize":4,"insertSpaces":true}}}}}}"#,
            id,
            escape_json(&uri)
        );
        self.send_raw(&msg);
    }

    /// Request code actions at cursor (quick fixes). Results → pending_code_actions.
    /// Start call hierarchy: prepareCallHierarchy → incoming or outgoing calls.
    pub fn request_call_hierarchy(
        &mut self,
        path: &str,
        row: usize,
        col: usize,
        direction: crate::call_hierarchy::CallDirection,
    ) {
        if !self.server_running {
            self.soft_error = Some("LSP not running".into());
            return;
        }
        self.pending_call_hierarchy.clear();
        self.call_hierarchy_ready = false;
        self.pending_call_direction = Some(direction);
        let abs = abs_path(path);
        let uri = path_to_uri(&abs);
        let col16 = self.char_col_to_utf16(row, col);
        let id = self.alloc_id(PendingReq::PrepareCallHierarchy);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/prepareCallHierarchy","params":{{"textDocument":{{"uri":"{}"}},"position":{{"line":{},"character":{}}}}}}}"#,
            id,
            escape_json(&uri),
            row,
            col16
        );
        self.send_raw(&msg);
    }

    fn request_calls_for_item(
        &mut self,
        item_json: &str,
        direction: crate::call_hierarchy::CallDirection,
    ) {
        let (method, kind) = match direction {
            crate::call_hierarchy::CallDirection::Incoming => {
                ("callHierarchy/incomingCalls", PendingReq::IncomingCalls)
            }
            crate::call_hierarchy::CallDirection::Outgoing => {
                ("callHierarchy/outgoingCalls", PendingReq::OutgoingCalls)
            }
        };
        let id = self.alloc_id(kind);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"{}","params":{{"item":{}}}}}"#,
            id, method, item_json
        );
        self.send_raw(&msg);
    }

    /// Contiguous slice of semantic tokens on `row`. Kept row-sorted at decode
    /// time so the renderer binary-searches instead of scanning the whole file
    /// once per visible row per frame.
    pub fn semantic_tokens_for_row(&self, row: usize) -> &[SemanticToken] {
        let lo = self.semantic_tokens.partition_point(|t| t.3 < row);
        let hi = self.semantic_tokens.partition_point(|t| t.3 <= row);
        &self.semantic_tokens[lo..hi]
    }

    /// Contiguous slice of diagnostics on `row`. Kept row-sorted when published.
    pub fn diagnostics_for_row(&self, row: usize) -> &[Diagnostic] {
        let lo = self.diagnostics.partition_point(|d| d.row < row);
        let hi = self.diagnostics.partition_point(|d| d.row <= row);
        &self.diagnostics[lo..hi]
    }

    pub fn request_code_action(&mut self, path: &str, row: usize, col: usize) {
        if !self.server_running {
            return;
        }
        let uri = path_to_uri(&abs_path(path));
        let col16 = self.char_col_to_utf16(row, col);
        // Include diagnostics that touch this line for better quick-fixes
        let mut diags_json = String::from("[");
        let mut first = true;
        for d in self.diagnostics.iter().filter(|d| d.row == row) {
            if !first {
                diags_json.push(',');
            }
            first = false;
            let sev = match d.severity {
                DiagnosticSeverity::Error => 1,
                DiagnosticSeverity::Warning => 2,
                DiagnosticSeverity::Info => 3,
                DiagnosticSeverity::Hint => 4,
            };
            diags_json.push_str(&format!(
                r#"{{"range":{{"start":{{"line":{},"character":{}}},"end":{{"line":{},"character":{}}}}},"severity":{},"message":"{}"}}"#,
                d.row,
                self.char_col_to_utf16(d.row, d.col_start),
                d.row,
                self.char_col_to_utf16(d.row, d.col_end),
                sev,
                escape_json(&d.message)
            ));
        }
        diags_json.push(']');
        let id = self.alloc_id(PendingReq::CodeAction);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/codeAction","params":{{"textDocument":{{"uri":"{}"}},"range":{{"start":{{"line":{},"character":{}}},"end":{{"line":{},"character":{}}}}},"context":{{"diagnostics":{},"only":["quickfix","refactor","source"]}}}}}}"#,
            id,
            escape_json(&uri),
            row,
            col16,
            row,
            col16,
            diags_json
        );
        self.send_raw(&msg);
    }

    pub fn execute_command(&mut self, command: &str, args_json: Option<&str>) {
        if !self.server_running {
            return;
        }
        let id = self.alloc_id(PendingReq::ExecuteCommand);
        let args = args_json.unwrap_or("[]");
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"workspace/executeCommand","params":{{"command":"{}","arguments":{}}}}}"#,
            id,
            escape_json(command),
            args
        );
        self.send_raw(&msg);
    }

    fn request_position(
        &mut self,
        kind: PendingReq,
        method: &str,
        path: &str,
        row: usize,
        col: usize,
    ) {
        if !self.server_running {
            return;
        }
        let uri = path_to_uri(&abs_path(path));
        let col16 = self.char_col_to_utf16(row, col);
        let id = self.alloc_id(kind);
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"{}","params":{{"textDocument":{{"uri":"{}"}},"position":{{"line":{},"character":{}}}}}}}"#,
            id,
            method,
            escape_json(&uri),
            row,
            col16
        );
        self.send_raw(&msg);
    }

    /// Editor char column → LSP UTF-16 code units for `row`.
    fn char_col_to_utf16(&self, row: usize, col: usize) -> usize {
        let line = self
            .semantic_doc_text
            .split('\n')
            .nth(row)
            .unwrap_or("");
        char_to_utf16_col(line, col)
    }

    fn alloc_id(&mut self, kind: PendingReq) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.pending.insert(id, kind);
        id
    }

    fn send_raw(&mut self, msg: &str) {
        if let Some(ref mut stdin) = self.stdin {
            // Content-Length is **bytes**
            let bytes = msg.as_bytes();
            let header = format!("Content-Length: {}\r\n\r\n", bytes.len());
            let _ = stdin.write_all(header.as_bytes());
            let _ = stdin.write_all(bytes);
            let _ = stdin.flush();
        }
    }

    pub fn poll(&mut self) {
        let mut batch = Vec::new();
        if let Some(ref rx) = self.rx {
            loop {
                match rx.try_recv() {
                    Ok(m) => batch.push(m),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.server_running = false;
                        self.initialized = false;
                        self.error = Some("LSP server disconnected".into());
                        break;
                    }
                }
            }
        }

        for msg in batch {
            self.handle_raw(msg);
        }

        // After initialize succeeded, send initialized + didOpen
        if self.initialized {
            if let Some((path, lang, text)) = self.pending_didopen.take() {
                self.send_raw(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#);
                self.doc_version = 1;
                self.current_uri = path_to_uri(&path);
                self.opened_uri = self.current_uri.clone();
                // Prefer in-memory text we already stored; fall back to disk
                if self.semantic_doc_text.is_empty() {
                    self.semantic_doc_text = std::fs::read_to_string(&path).unwrap_or_default();
                }
                let msg = format!(
                    r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{}","languageId":"{}","version":{},"text":"{}"}}}}}}"#,
                    escape_json(&self.current_uri),
                    lang,
                    self.doc_version,
                    text // already escaped
                );
                self.send_raw(&msg);
                self.semantic_dirty = true;
                self.maybe_request_semantic_tokens();
                self.inlay_dirty = true;
                self.code_lens_dirty = true;
            }
        }

        // Retry semantic tokens if a prior request was coalesced
        if self.semantic_dirty && self.server_running {
            self.maybe_request_semantic_tokens();
        }
    }

    fn handle_raw(&mut self, msg: RawMsg) {
        // Notifications
        if let Some(method) = msg.method.as_deref() {
            if method == "textDocument/publishDiagnostics" {
                let uri = extract_str(&msg.body, "\"uri\":\"").unwrap_or_default();
                let mut diags = parse_diagnostics(&msg.body);
                // Server columns are UTF-16 code units; the editor uses chars.
                diag_cols_utf16_to_chars(&mut diags, &self.semantic_doc_text);
                // Accept if URI matches (normalized) or empty
                if uri.is_empty() || uris_match(&uri, &self.current_uri) {
                    // Keep row-sorted so `diagnostics_for_row` can binary-search.
                    diags.sort_by_key(|d| d.row);
                    self.diagnostics = diags; // empty clears
                }
                return;
            }
            // ignore other notifications
            if msg.id.is_none() {
                return;
            }
        }

        let Some(id) = msg.id else {
            return;
        };
        let Some(kind) = self.pending.remove(&id) else {
            // Unknown id — try heuristic only for initialize-like
            if msg.body.contains("\"capabilities\"") {
                self.finish_initialize(&msg.body);
            }
            return;
        };

        // Error response — only Initialize is a hard sticky status error.
        if is_jsonrpc_error(&msg.body) {
            let m = extract_str(&msg.body, "\"message\":\"")
                .unwrap_or_else(|| "request failed".into());
            match kind {
                PendingReq::Initialize => {
                    self.error = Some(format!("LSP init: {m}"));
                    self.server_running = false;
                    self.initialized = false;
                }
                PendingReq::SemanticTokens | PendingReq::InlayHint | PendingReq::CodeLens => {
                    // Optional features — demote to soft, don't red-badge
                    self.soft_error = Some(format!("LSP: {m}"));
                    if matches!(kind, PendingReq::SemanticTokens) {
                        self.semantic_tokens_supported = false;
                    }
                    if matches!(kind, PendingReq::InlayHint) {
                        self.inlay_supported = false;
                    }
                    if matches!(kind, PendingReq::CodeLens) {
                        self.code_lens_supported = false;
                        self.code_lenses.clear();
                    }
                }
                PendingReq::CodeLensResolve(_) => {
                    // Best-effort enrichment — a failed resolve is not news.
                }
                _ => {
                    self.soft_error = Some(format!("LSP: {m}"));
                }
            }
            return;
        }

        match kind {
            PendingReq::Initialize => {
                self.finish_initialize(&msg.body);
            }
            PendingReq::Definition => {
                if let Some(loc) = parse_single_location(&msg.body) {
                    self.pending_definition = Some(loc);
                } else {
                    self.soft_error = Some("No definition found".into());
                }
            }
            PendingReq::Completion => {
                self.pending_completions = parse_completions(&msg.body);
            }
            PendingReq::Hover => {
                if let Some(h) = parse_hover(&msg.body) {
                    self.pending_hover = Some(h);
                }
            }
            PendingReq::References => {
                self.pending_references = parse_locations(&msg.body);
            }
            PendingReq::Rename => {
                let edits = parse_workspace_edit_ctx(
                    &msg.body,
                    &self.current_uri,
                    &self.semantic_doc_text,
                );
                if edits.is_empty() {
                    self.pending_workspace_edit =
                        Some(parse_rename_message(&msg.body).unwrap_or_else(|| {
                            "Rename: no changes".into()
                        }));
                } else {
                    self.pending_edits = edits;
                }
            }
            PendingReq::Formatting => {
                if let Some(edit) =
                    parse_text_edits_as_full_replace(&msg.body, &self.semantic_doc_text)
                {
                    let path = uri_to_path(&self.current_uri);
                    self.pending_edits = vec![FileEdit { path, text: edit }];
                } else if msg.body.contains("\"result\":null")
                    || msg.body.contains("\"result\":[]")
                {
                    self.soft_error = Some("Format: nothing to change".into());
                }
            }
            PendingReq::CodeAction => {
                self.pending_code_actions = parse_code_actions_ctx(
                    &msg.body,
                    &self.current_uri,
                    &self.semantic_doc_text,
                );
                if self.pending_code_actions.is_empty() {
                    self.soft_error = Some("No code actions".into());
                }
            }
            PendingReq::ExecuteCommand => {
                let edits = parse_workspace_edit_ctx(
                    &msg.body,
                    &self.current_uri,
                    &self.semantic_doc_text,
                );
                if !edits.is_empty() {
                    self.pending_edits = edits;
                } else {
                    self.soft_error = Some("Command executed".into());
                }
            }
            PendingReq::SemanticTokens => {
                let data = parse_semantic_data(&msg.body);
                let lines: Vec<&str> = self.semantic_doc_text.split('\n').collect();
                let mut toks = decode_semantic_tokens(&data, &self.semantic_token_types, &lines);
                // LSP emits tokens in position order already, but guarantee the
                // row-sorted invariant `semantic_tokens_for_row` relies on.
                toks.sort_by_key(|t| t.3);
                self.semantic_tokens = toks;
                // If document changed while request was in flight, re-fetch
                if self.doc_version != self.last_semantic_req_version {
                    self.semantic_dirty = true;
                }
            }
            PendingReq::DocumentSymbol | PendingReq::WorkspaceSymbol => {
                self.pending_symbols = parse_symbols(&msg.body);
            }
            PendingReq::InlayHint => {
                let lines: Vec<&str> = self.semantic_doc_text.split('\n').collect();
                self.inlay_hints = parse_inlay_hints(&msg.body, &lines);
            }
            PendingReq::PrepareCallHierarchy => {
                let items = parse_call_hierarchy_items(&msg.body);
                if items.is_empty() {
                    self.soft_error = Some("No call hierarchy at cursor".into());
                    self.call_hierarchy_ready = true;
                    self.pending_call_hierarchy.clear();
                } else {
                    // Use first item; request incoming/outgoing
                    let dir = self
                        .pending_call_direction
                        .unwrap_or(crate::call_hierarchy::CallDirection::Incoming);
                    let raw = items[0].raw_json.clone();
                    // Surface root name immediately as a single-item fallback
                    self.pending_call_hierarchy = items;
                    self.request_calls_for_item(&raw, dir);
                }
            }
            PendingReq::IncomingCalls | PendingReq::OutgoingCalls => {
                let calls = parse_call_hierarchy_calls(&msg.body);
                self.pending_call_hierarchy = calls;
                self.call_hierarchy_ready = true;
            }
            PendingReq::CodeLens => {
                self.code_lens_gen = self.code_lens_gen.wrapping_add(1);
                let (resolved, unresolved) = parse_code_lenses(&msg.body);
                self.code_lenses = resolved;
                // Servers like rust-analyzer return lenses without a command;
                // resolve them individually (capped to avoid request storms).
                let generation = self.code_lens_gen;
                for lens in unresolved.into_iter().take(40) {
                    let id = self.alloc_id(PendingReq::CodeLensResolve(generation));
                    let req = format!(
                        r#"{{"jsonrpc":"2.0","id":{id},"method":"codeLens/resolve","params":{lens}}}"#
                    );
                    self.send_raw(&req);
                }
            }
            PendingReq::CodeLensResolve(generation) => {
                if generation == self.code_lens_gen {
                    if let Some(lens) = parse_resolved_code_lens(&msg.body) {
                        merge_code_lens(&mut self.code_lenses, lens);
                    }
                }
            }
        }
    }

    fn finish_initialize(&mut self, body: &str) {
        self.initialized = true;
        self.server_running = true;
        self.error = None;
        // Parse semantic tokens legend + support flag
        let (types, supported) = parse_semantic_legend(body);
        self.semantic_token_types = types;
        self.semantic_tokens_supported = supported && !self.semantic_token_types.is_empty();
        // Default legend if server supports full but omitted types (rare)
        if supported && self.semantic_token_types.is_empty() {
            self.semantic_token_types = default_semantic_types();
            self.semantic_tokens_supported = true;
        }
        self.inlay_supported = body.contains("inlayHintProvider") || body.contains("\"inlayHint\"");
        if self.inlay_supported {
            self.inlay_dirty = true;
        }
    }

    pub fn shutdown(&mut self) {
        if self.stdin.is_some() {
            self.send_raw(r#"{"jsonrpc":"2.0","id":999999,"method":"shutdown","params":null}"#);
            self.send_raw(r#"{"jsonrpc":"2.0","method":"exit","params":null}"#);
        }
        self.shutdown_quiet();
    }

    fn shutdown_quiet(&mut self) {
        if let Some(mut child) = self._child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.stdin = None;
        self.rx = None;
        self.server_running = false;
        self.initialized = false;
        self.pending.clear();
        self.pending_didopen = None;
        self.diagnostics.clear();
        self.semantic_tokens.clear();
        self.semantic_token_types.clear();
        self.semantic_tokens_supported = false;
        self.semantic_doc_text.clear();
        self.semantic_dirty = false;
        self.inlay_hints.clear();
        self.inlay_supported = false;
        self.inlay_dirty = false;
        self.pending_symbols.clear();
        self.pending_code_actions.clear();
        self.pending_edits.clear();
        self.definition_as_peek = false;
        self.opened_uri.clear();
        self.soft_error = None;
    }

    /// Status label for the TUI: running server, soft miss, or hard error.
    pub fn status_label(&self) -> LspStatus {
        if self.server_running {
            LspStatus::Running {
                name: self.server_name.clone(),
                diags: self.diagnostics.len(),
            }
        } else if self.error.is_some() {
            LspStatus::HardError
        } else if self.soft_error.is_some() {
            LspStatus::Soft {
                msg: self.soft_error.clone().unwrap_or_default(),
            }
        } else {
            LspStatus::Idle
        }
    }
}

#[derive(Debug, Clone)]
pub enum LspStatus {
    Idle,
    Running { name: String, diags: usize },
    Soft { msg: String },
    HardError,
}

// ── Reader thread ───────────────────────────────────────

fn read_loop(stdout: impl Read + Send + 'static, tx: mpsc::Sender<RawMsg>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_len: Option<usize> = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return,
                Err(_) => return,
                Ok(_) => {
                    let t = line.trim_end_matches(['\r', '\n']);
                    if t.is_empty() {
                        break;
                    }
                    if let Some(rest) = t
                        .strip_prefix("Content-Length:")
                        .or_else(|| t.strip_prefix("content-length:"))
                    {
                        content_len = rest.trim().parse().ok();
                    }
                }
            }
        }
        let Some(len) = content_len else {
            continue;
        };
        if len == 0 || len > 50_000_000 {
            continue;
        }
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let text = String::from_utf8_lossy(&body).into_owned();
        let id = extract_json_id(&text);
        let method = extract_str(&text, "\"method\":\"");
        if tx
            .send(RawMsg {
                id,
                method,
                body: text,
            })
            .is_err()
        {
            return;
        }
    }
}

// ── Parsing helpers ─────────────────────────────────────

fn parse_diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let Some(start) = text.find("\"diagnostics\":") else {
        return diags;
    };
    let rest = &text[start..];
    // Each diagnostic has a range
    for item in rest.split("\"range\"").skip(1) {
        let row = extract_int(item, "\"line\":").unwrap_or(0).max(0) as usize;
        // first character = start, second in end object
        let chars: Vec<i64> = {
            let mut v = Vec::new();
            let mut search = item;
            while let Some(pos) = search.find("\"character\":") {
                let after = &search[pos + "\"character\":".len()..];
                if let Some(n) = after
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .ok()
                {
                    v.push(n);
                }
                search = &search[pos + 12..];
                if v.len() >= 2 {
                    break;
                }
            }
            v
        };
        let col_start = chars.first().copied().unwrap_or(0).max(0) as usize;
        let col_end = chars
            .get(1)
            .copied()
            .unwrap_or((col_start as i64) + 1)
            .max(0) as usize;
        let msg = extract_str(item, "\"message\":\"")
            .unwrap_or_default()
            .replace('\n', " ");
        let severity = match extract_int(item, "\"severity\":") {
            Some(1) => DiagnosticSeverity::Error,
            Some(2) => DiagnosticSeverity::Warning,
            Some(3) => DiagnosticSeverity::Info,
            _ => DiagnosticSeverity::Hint,
        };
        if !msg.is_empty() {
            diags.push(Diagnostic {
                row,
                col_start,
                col_end: col_end.max(col_start + 1),
                message: msg,
                severity,
            });
        }
    }
    diags
}

/// Rewrite diagnostic columns from UTF-16 code units to char indices, using
/// the last document text we synced to the server. ASCII lines short-circuit.
fn diag_cols_utf16_to_chars(diags: &mut [Diagnostic], doc: &str) {
    if diags.is_empty() || doc.is_empty() {
        return;
    }
    let lines: Vec<&str> = doc.split('\n').collect();
    for d in diags {
        let Some(line) = lines.get(d.row) else {
            continue;
        };
        if line.is_ascii() {
            continue;
        }
        d.col_start = utf16_to_char_col(line, d.col_start);
        d.col_end = utf16_to_char_col(line, d.col_end).max(d.col_start + 1);
    }
}

fn parse_single_location(text: &str) -> Option<Location> {
    let locs = parse_locations(text);
    locs.into_iter().next()
}

fn symbol_kind_name(k: i64) -> &'static str {
    match k {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        4 => "Package",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        15 => "String",
        16 => "Number",
        17 => "Boolean",
        18 => "Array",
        19 => "Object",
        20 => "Key",
        21 => "Null",
        22 => "EnumMember",
        23 => "Struct",
        24 => "Event",
        25 => "Operator",
        26 => "TypeParameter",
        _ => "Symbol",
    }
}

fn parse_symbols(text: &str) -> Vec<SymbolItem> {
    let mut out = Vec::new();
    if text.contains("\"result\":null") {
        return out;
    }
    // DocumentSymbol has "name" + nested "range"; SymbolInformation has "location"
    for chunk in text.split("\"name\":\"").skip(1) {
        let name = chunk.split('"').next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let kind_n = extract_int(chunk, "\"kind\":").unwrap_or(0);
        let kind = symbol_kind_name(kind_n).to_string();
        let path = if let Some(uri) = extract_str(chunk, "\"uri\":\"") {
            uri_to_path(&uri)
        } else {
            String::new()
        };
        // Prefer selectionRange / range start
        let row = extract_int(chunk, "\"line\":").unwrap_or(0).max(0) as usize;
        let col = extract_int(chunk, "\"character\":").unwrap_or(0).max(0) as usize;
        let detail = extract_str(chunk, "\"detail\":\"").unwrap_or_default();
        out.push(SymbolItem {
            name,
            kind,
            path,
            row,
            col,
            detail,
        });
        if out.len() >= 500 {
            break;
        }
    }
    out
}

fn parse_inlay_hints(text: &str, lines: &[&str]) -> Vec<InlayHint> {
    let mut out = Vec::new();
    if text.contains("\"result\":null") || text.contains("\"result\":[]") {
        return out;
    }
    // Each hint: position.line/character + label (string or array of parts)
    for chunk in text.split("\"position\"").skip(1) {
        let row = extract_int(chunk, "\"line\":").unwrap_or(0).max(0) as usize;
        let col_u16 = extract_int(chunk, "\"character\":").unwrap_or(0).max(0) as usize;
        let col = if let Some(line) = lines.get(row) {
            utf16_to_char_col(line, col_u16)
        } else {
            col_u16
        };
        let label = if let Some(s) = extract_str(chunk, "\"label\":\"") {
            s
        } else {
            // label as array of {value: "..."}
            let mut parts = Vec::new();
            for part in chunk.split("\"value\":\"").skip(1).take(6) {
                let v = part.split('"').next().unwrap_or("");
                if !v.is_empty() {
                    parts.push(v.to_string());
                }
            }
            parts.join("")
        };
        let label = label.trim().to_string();
        if label.is_empty() {
            continue;
        }
        out.push(InlayHint { row, col, label });
        if out.len() >= 2000 {
            break;
        }
    }
    out.sort_by_key(|h| (h.row, h.col));
    out
}

fn parse_locations(text: &str) -> Vec<Location> {
    let mut locs = Vec::new();
    // result:null
    if text.contains("\"result\":null") {
        return locs;
    }
    for chunk in text.split("\"uri\":\"").skip(1) {
        let uri = chunk.split('"').next().unwrap_or("");
        let path = uri_to_path(uri);
        if path.is_empty() {
            continue;
        }
        // Prefer start line/character inside this chunk
        let row = extract_int(chunk, "\"line\":").unwrap_or(0).max(0) as usize;
        let col = extract_int(chunk, "\"character\":").unwrap_or(0).max(0) as usize;
        locs.push(Location { path, row, col });
    }
    locs
}

fn parse_completions(text: &str) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for chunk in text.split("\"label\":\"").skip(1) {
        let label = chunk.split('"').next().unwrap_or("").to_string();
        if label.is_empty() {
            continue;
        }
        let detail = extract_str(chunk, "\"detail\":\"").map(|s| s.to_string());
        items.push(CompletionItem { label, detail });
        if items.len() >= 200 {
            break;
        }
    }
    items
}

fn parse_hover(text: &str) -> Option<String> {
    if text.contains("\"result\":null") {
        return None;
    }
    // extract_str already unescapes.
    if let Some(v) = extract_str(text, "\"value\":\"") {
        if !v.trim().is_empty() {
            return Some(v);
        }
    }
    extract_str(text, "\"contents\":\"").filter(|s| !s.trim().is_empty())
}

/// Extract semanticTokensProvider legend.tokenTypes from initialize result.
fn parse_semantic_legend(body: &str) -> (Vec<String>, bool) {
    let supported = body.contains("semanticTokensProvider")
        || body.contains("\"semanticTokens\"");
    // Find tokenTypes array inside semanticTokensProvider if possible
    let search_from = body
        .find("semanticTokensProvider")
        .or_else(|| body.find("\"tokenTypes\""))
        .unwrap_or(0);
    let region = &body[search_from..];
    let Some(arr_start_rel) = region.find("\"tokenTypes\"") else {
        return (if supported { default_semantic_types() } else { Vec::new() }, supported);
    };
    let after = &region[arr_start_rel..];
    let Some(bracket) = after.find('[') else {
        return (if supported { default_semantic_types() } else { Vec::new() }, supported);
    };
    let rest = &after[bracket + 1..];
    let Some(end) = rest.find(']') else {
        return (Vec::new(), supported);
    };
    let arr = &rest[..end];
    let mut types = Vec::new();
    for part in arr.split(',') {
        let t = part
            .trim()
            .trim_matches('"')
            .trim()
            .to_string();
        if !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            types.push(t);
        }
    }
    if types.is_empty() && supported {
        types = default_semantic_types();
    }
    let nonempty = !types.is_empty();
    (types, supported || nonempty)
}

fn default_semantic_types() -> Vec<String> {
    [
        "namespace",
        "type",
        "class",
        "enum",
        "interface",
        "struct",
        "typeParameter",
        "parameter",
        "variable",
        "property",
        "enumMember",
        "event",
        "function",
        "method",
        "macro",
        "keyword",
        "modifier",
        "comment",
        "string",
        "number",
        "regexp",
        "operator",
        "decorator",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Parse `"data":[n,n,...]` from semanticTokens/full result.
fn parse_semantic_data(body: &str) -> Vec<u32> {
    let Some(pos) = body.find("\"data\"") else {
        return Vec::new();
    };
    let after = &body[pos..];
    let Some(bracket) = after.find('[') else {
        return Vec::new();
    };
    let rest = &after[bracket + 1..];
    let Some(end) = rest.find(']') else {
        return Vec::new();
    };
    let arr = &rest[..end];
    let mut out = Vec::new();
    for part in arr.split(',') {
        let t = part.trim();
        if t.is_empty() {
            continue;
        }
        if let Ok(n) = t.parse::<u32>() {
            out.push(n);
        } else if let Ok(n) = t.parse::<i64>() {
            out.push(n.max(0) as u32);
        }
    }
    out
}

/// Decode LSP relative semantic tokens into char-column spans.
fn decode_semantic_tokens(
    data: &[u32],
    legend: &[String],
    lines: &[&str],
) -> Vec<SemanticToken> {
    let mut tokens = Vec::new();
    if data.len() < 5 || legend.is_empty() {
        return tokens;
    }
    let mut line: u32 = 0;
    let mut start_utf16: u32 = 0;
    let mut i = 0;
    while i + 4 < data.len() {
        let delta_line = data[i];
        let delta_start = data[i + 1];
        let length = data[i + 2];
        let token_type = data[i + 3] as usize;
        // data[i+4] = modifiers (ignored for coloring)
        i += 5;

        if delta_line > 0 {
            line = line.saturating_add(delta_line);
            start_utf16 = delta_start;
        } else {
            start_utf16 = start_utf16.saturating_add(delta_start);
        }

        let row = line as usize;
        let Some(line_text) = lines.get(row) else {
            continue;
        };
        let scol = utf16_to_char_col(line_text, start_utf16 as usize);
        let ecol = utf16_to_char_col(line_text, start_utf16 as usize + length as usize);
        if scol >= ecol {
            continue;
        }
        let type_name = legend.get(token_type).map(|s| s.as_str()).unwrap_or("");
        if type_name.is_empty() {
            continue;
        }
        let kind = highlight::from_semantic_type(type_name);
        tokens.push((kind, scol, ecol, row));
    }
    tokens.sort_by_key(|(_, st, ed, row)| (*row, ed.saturating_sub(*st), *st));
    tokens
}

/// Convert a UTF-16 code-unit column (LSP wire format) to a char index.
pub fn utf16_to_char_col(line: &str, utf16_col: usize) -> usize {
    if utf16_col == 0 {
        return 0;
    }
    let mut u16s = 0usize;
    for (i, c) in line.chars().enumerate() {
        if u16s >= utf16_col {
            return i;
        }
        u16s += c.len_utf16();
    }
    line.chars().count()
}

/// Convert a char index to UTF-16 code-unit column (LSP wire format).
pub fn char_to_utf16_col(line: &str, char_col: usize) -> usize {
    line.chars().take(char_col).map(|c| c.len_utf16()).sum()
}

/// True if body is a JSON-RPC error object (not a result that happens to
/// contain the word "error").
fn is_jsonrpc_error(body: &str) -> bool {
    // "error": { ... } at top level without a successful result
    if let Some(pos) = body.find("\"error\"") {
        // result:null with error is still an error
        let after = &body[pos..];
        if after.contains('{') {
            // Avoid matching "errorCodes" etc. inside capabilities
            if body.contains("\"result\":") {
                // Both present — error wins if result is null
                return body.contains("\"result\":null")
                    || body.find("\"error\"").unwrap_or(usize::MAX)
                        < body.find("\"result\"").unwrap_or(usize::MAX);
            }
            return true;
        }
    }
    false
}

/// Apply LSP TextEdit[] as a full document replace when possible.
fn parse_text_edits_as_full_replace(body: &str, original: &str) -> Option<String> {
    if body.contains("\"result\":null") || body.contains("\"result\":[]") {
        return None;
    }
    // Prefer a single full-range edit
    let mut edits: Vec<(usize, usize, usize, usize, String)> = Vec::new();
    for chunk in body.split("\"range\"").skip(1) {
        let lines: Vec<i64> = {
            let mut v = Vec::new();
            let mut search = chunk;
            while let Some(pos) = search.find("\"line\":") {
                let after = &search[pos + 7..];
                if let Ok(n) = after
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '-')
                    .collect::<String>()
                    .parse()
                {
                    v.push(n);
                }
                search = &search[pos + 7..];
                if v.len() >= 2 {
                    break;
                }
            }
            v
        };
        let chars: Vec<i64> = {
            let mut v = Vec::new();
            let mut search = chunk;
            while let Some(pos) = search.find("\"character\":") {
                let after = &search[pos + 12..];
                if let Ok(n) = after
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '-')
                    .collect::<String>()
                    .parse()
                {
                    v.push(n);
                }
                search = &search[pos + 12..];
                if v.len() >= 2 {
                    break;
                }
            }
            v
        };
        let new_text = extract_str(chunk, "\"newText\":\"").unwrap_or_default();
        if lines.len() >= 2 && chars.len() >= 2 {
            edits.push((
                lines[0].max(0) as usize,
                chars[0].max(0) as usize,
                lines[1].max(0) as usize,
                chars[1].max(0) as usize,
                new_text,
            ));
        }
        if edits.len() > 50 {
            break;
        }
    }
    if edits.is_empty() {
        return None;
    }
    // Single edit covering whole file → take newText
    if edits.len() == 1 {
        let (r0, c0, r1, _c1, ref t) = edits[0];
        let line_count = original.lines().count().max(1);
        if r0 == 0 && c0 == 0 && r1 + 1 >= line_count {
            return Some(t.clone());
        }
        // Or one edit that replaces everything if newText is long
        if t.lines().count() >= line_count.saturating_sub(1) && r0 == 0 {
            return Some(t.clone());
        }
    }
    // Apply edits bottom-up on lines (char cols approximate for multi-edit)
    let mut lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    edits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    for (r0, c0, r1, c1, new_t) in edits {
        let r0 = r0.min(lines.len().saturating_sub(1));
        let r1 = r1.min(lines.len().saturating_sub(1));
        if r0 == r1 {
            let line = &lines[r0];
            let chars: Vec<char> = line.chars().collect();
            let c0 = c0.min(chars.len());
            let c1 = c1.min(chars.len()).max(c0);
            let mut s: String = chars[..c0].iter().collect();
            s.push_str(&new_t);
            s.push_str(&chars[c1..].iter().collect::<String>());
            // new_t may be multi-line
            if s.contains('\n') {
                let parts: Vec<String> = s.split('\n').map(|x| x.to_string()).collect();
                lines.splice(r0..=r0, parts);
            } else {
                lines[r0] = s;
            }
        } else {
            // Multi-line replace: keep prefix of first, suffix of last
            let first: Vec<char> = lines[r0].chars().collect();
            let last: Vec<char> = lines[r1].chars().collect();
            let c0 = c0.min(first.len());
            let c1 = c1.min(last.len());
            let mut s: String = first[..c0].iter().collect();
            s.push_str(&new_t);
            s.push_str(&last[c1..].iter().collect::<String>());
            let parts: Vec<String> = s.split('\n').map(|x| x.to_string()).collect();
            lines.splice(r0..=r1, parts);
        }
    }
    let trailing = original.ends_with('\n');
    let mut out = lines.join("\n");
    if trailing && !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

fn parse_rename_message(text: &str) -> Option<String> {
    if text.contains("\"result\":null") {
        return Some("Rename: no changes".into());
    }
    let n = text.matches("\"newText\":\"").count();
    if n == 0 {
        return Some("Rename: no changes".into());
    }
    Some(format!("Rename: {n} edit(s)"))
}

/// Parse WorkspaceEdit into full-file rewrites.
pub fn parse_workspace_edit(body: &str) -> Vec<FileEdit> {
    parse_workspace_edit_ctx(body, "", "")
}

fn parse_workspace_edit_ctx(body: &str, current_uri: &str, current_text: &str) -> Vec<FileEdit> {
    if body.contains("\"result\":null") {
        return Vec::new();
    }
    let mut by_path: HashMap<String, Vec<RawTextEdit>> = HashMap::new();

    // documentChanges: TextDocumentEdit has textDocument.uri + edits
    for chunk in body.split("\"textDocument\"").skip(1) {
        let uri = extract_str(chunk, "\"uri\":\"").unwrap_or_default();
        if uri.is_empty() {
            continue;
        }
        let path = uri_to_path(&uri);
        let edits = parse_text_edit_list(chunk);
        if !edits.is_empty() {
            by_path.entry(path).or_default().extend(edits);
        }
    }

    // changes: { "file://...": [ TextEdit ] }
    if by_path.is_empty() {
        if let Some(start) = body.find("\"changes\"") {
            let rest = &body[start..];
            for chunk in rest.split("\"file:").skip(1) {
                let uri_body = chunk.split('"').next().unwrap_or("");
                let uri = format!("file:{uri_body}");
                let path = uri_to_path(&uri);
                let edits = parse_text_edit_list(chunk);
                if !edits.is_empty() {
                    by_path.entry(path).or_default().extend(edits);
                }
            }
        }
    }

    if by_path.is_empty() {
        if let Some(uri) = extract_str(body, "\"uri\":\"") {
            let path = uri_to_path(&uri);
            let edits = parse_text_edit_list(body);
            if !edits.is_empty() {
                by_path.insert(path, edits);
            }
        }
    }

    let current_path = if current_uri.is_empty() {
        String::new()
    } else {
        uri_to_path(current_uri)
    };

    let mut out = Vec::new();
    for (path, mut edits) in by_path {
        if path.is_empty() {
            continue;
        }
        let original = if !current_path.is_empty() && path == current_path && !current_text.is_empty()
        {
            current_text.to_string()
        } else {
            std::fs::read_to_string(&path).unwrap_or_default()
        };
        let text = apply_raw_text_edits(&original, &mut edits);
        out.push(FileEdit { path, text });
    }
    out
}

#[derive(Debug, Clone)]
struct RawTextEdit {
    r0: usize,
    c0: usize, // utf-16
    r1: usize,
    c1: usize, // utf-16
    new_text: String,
}

fn parse_text_edit_list(chunk: &str) -> Vec<RawTextEdit> {
    let mut edits = Vec::new();
    for range_chunk in chunk.split("\"range\"").skip(1) {
        let (r0, c0, r1, c1) = extract_range_lines_chars(range_chunk);
        let new_text = extract_str(range_chunk, "\"newText\":\"").unwrap_or_else(|| {
            // newText may appear as next field after range object
            unescape_json_string_prefix(
                range_chunk
                    .split("\"newText\":\"")
                    .nth(1)
                    .unwrap_or(""),
            )
        });
        if new_text.is_empty() && r0 == r1 && c0 == c1 {
            // pure delete still valid — keep
        }
        edits.push(RawTextEdit {
            r0,
            c0,
            r1,
            c1,
            new_text,
        });
        if edits.len() > 200 {
            break;
        }
    }
    edits
}

fn extract_range_lines_chars(chunk: &str) -> (usize, usize, usize, usize) {
    let mut lines = Vec::new();
    let mut chars = Vec::new();
    let mut search = chunk;
    while let Some(pos) = search.find("\"line\":") {
        let after = &search[pos + 7..];
        if let Ok(n) = after
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<usize>()
        {
            lines.push(n);
        }
        search = &search[pos + 7..];
        if lines.len() >= 2 {
            break;
        }
    }
    search = chunk;
    while let Some(pos) = search.find("\"character\":") {
        let after = &search[pos + 12..];
        if let Ok(n) = after
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<usize>()
        {
            chars.push(n);
        }
        search = &search[pos + 12..];
        if chars.len() >= 2 {
            break;
        }
    }
    (
        lines.first().copied().unwrap_or(0),
        chars.first().copied().unwrap_or(0),
        lines.get(1).copied().unwrap_or(0),
        chars.get(1).copied().unwrap_or(0),
    )
}

fn unescape_json_string_prefix(part: &str) -> String {
    let mut raw = String::new();
    let mut chars = part.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                match n {
                    'n' => raw.push('\n'),
                    't' => raw.push('\t'),
                    'r' => raw.push('\r'),
                    '"' => raw.push('"'),
                    '\\' => raw.push('\\'),
                    other => {
                        raw.push('\\');
                        raw.push(other);
                    }
                }
            }
        } else if c == '"' {
            break;
        } else {
            raw.push(c);
        }
    }
    raw
}

fn apply_raw_text_edits(original: &str, edits: &mut [RawTextEdit]) -> String {
    if edits.is_empty() {
        return original.to_string();
    }
    // Convert utf-16 cols to char cols per line, apply bottom-up
    let mut lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    // Sort by end position descending
    edits.sort_by(|a, b| b.r1.cmp(&a.r1).then(b.c1.cmp(&a.c1)));
    for e in edits.iter() {
        let r0 = e.r0.min(lines.len().saturating_sub(1));
        let r1 = e.r1.min(lines.len().saturating_sub(1));
        let c0 = utf16_to_char_col(&lines[r0], e.c0);
        let c1 = utf16_to_char_col(&lines[r1], e.c1);
        if r0 == r1 {
            let chs: Vec<char> = lines[r0].chars().collect();
            let c0 = c0.min(chs.len());
            let c1 = c1.min(chs.len()).max(c0);
            let mut s: String = chs[..c0].iter().collect();
            s.push_str(&e.new_text);
            s.push_str(&chs[c1..].iter().collect::<String>());
            if s.contains('\n') {
                let parts: Vec<String> = s.split('\n').map(|x| x.to_string()).collect();
                lines.splice(r0..=r0, parts);
            } else {
                lines[r0] = s;
            }
        } else {
            let first: Vec<char> = lines[r0].chars().collect();
            let last: Vec<char> = lines[r1].chars().collect();
            let c0 = c0.min(first.len());
            let c1 = c1.min(last.len());
            let mut s: String = first[..c0].iter().collect();
            s.push_str(&e.new_text);
            s.push_str(&last[c1..].iter().collect::<String>());
            let parts: Vec<String> = s.split('\n').map(|x| x.to_string()).collect();
            lines.splice(r0..=r1, parts);
        }
    }
    let trailing = original.ends_with('\n');
    let mut out = lines.join("\n");
    if trailing && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn parse_code_actions_ctx(
    body: &str,
    current_uri: &str,
    current_text: &str,
) -> Vec<CodeActionItem> {
    let mut out = Vec::new();
    if body.contains("\"result\":null") || body.contains("\"result\":[]") {
        return out;
    }
    for chunk in body.split("\"title\":\"").skip(1) {
        let title = chunk.split('"').next().unwrap_or("").to_string();
        if title.is_empty() {
            continue;
        }
        let kind = extract_str(chunk, "\"kind\":\"").unwrap_or_default();
        let edits = if let Some(pos) = chunk.find("\"edit\"") {
            parse_workspace_edit_ctx(&chunk[pos..], current_uri, current_text)
        } else {
            Vec::new()
        };
        let (command, command_args_json) = if let Some(pos) = chunk.find("\"command\":{") {
            let sub = &chunk[pos..];
            let cmd = extract_str(sub, "\"command\":\"");
            let args = sub.find("\"arguments\":").map(|i| {
                let rest = &sub[i + 12..];
                extract_json_array(rest).unwrap_or_else(|| "[]".into())
            });
            (cmd, args)
        } else {
            let cmd = extract_str(chunk, "\"command\":\"").filter(|s| {
                // avoid matching "command" inside longer keys when it's a string id only
                !s.is_empty() && !s.contains('{')
            });
            (cmd, None)
        };
        out.push(CodeActionItem {
            title,
            kind,
            edits,
            command,
            command_args_json,
        });
        if out.len() >= 40 {
            break;
        }
    }
    out
}

fn extract_json_array(s: &str) -> Option<String> {
    let start = s.find('[')?;
    let mut depth = 0i32;
    for (i, c) in s[start..].chars().enumerate() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn install_hint(bin: &str) -> String {
    let hint = match bin {
        "rust-analyzer" => "install: rustup component add rust-analyzer",
        "pyright-langserver" | "pyright" => "install: npm i -g pyright",
        "typescript-language-server" => "install: npm i -g typescript-language-server typescript",
        "clangd" => "install: brew install llvm  (or apt install clangd)",
        "gopls" => "install: go install golang.org/x/tools/gopls@latest",
        "lua-language-server" => "install: brew install lua-language-server",
        "marksman" => "install: brew install marksman",
        "yaml-language-server" => "install: npm i -g yaml-language-server",
        "taplo" => "install: cargo install taplo-cli --locked",
        "bash-language-server" => "install: npm i -g bash-language-server",
        "zls" => "install: see https://github.com/zigtools/zls",
        "jdtls" => "install: brew install jdtls",
        _ => "install the language server or :LspStart <cmd>",
    };
    format!("LSP `{bin}` not found — {hint}")
}

/// Build a valid `initialize` request body (tested for brace-balance).
fn build_initialize_request(id: u64, pid: u32, root_uri: &str, folder_name: &str) -> String {
    let root = escape_json(root_uri);
    let folder = escape_json(folder_name);
    // Token types legend (flat array) — keep as one line for readability.
    let token_types = r#"["namespace","type","class","enum","interface","struct","typeParameter","parameter","variable","property","enumMember","event","function","method","macro","keyword","modifier","comment","string","number","regexp","operator","decorator"]"#;
    let token_mods = r#"["declaration","definition","readonly","static","deprecated","abstract","async","modification","documentation","defaultLibrary"]"#;
    format!(
        concat!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"initialize","params":{{"#,
            r#""processId":{pid},"rootUri":"{root}","#,
            r#""workspaceFolders":[{{"uri":"{root}","name":"{folder}"}}],"#,
            r#""capabilities":{{"#,
            r#""general":{{"positionEncodings":["utf-16"]}},"#,
            r#""textDocument":{{"#,
            r#""synchronization":{{"didSave":true,"dynamicRegistration":false}},"#,
            r#""publishDiagnostics":{{"relatedInformation":true}},"#,
            r#""hover":{{"contentFormat":["markdown","plaintext"]}},"#,
            r#""completion":{{"completionItem":{{"snippetSupport":false,"documentationFormat":["markdown","plaintext"]}}}},"#,
            r#""definition":{{"linkSupport":true}},"#,
            r#""references":{{}},"#,
            r#""rename":{{"prepareSupport":false}},"#,
            r#""formatting":{{}},"#,
            r#""codeAction":{{"codeActionLiteralSupport":{{"codeActionKind":{{"valueSet":["quickfix","refactor","source"]}}}}}},"#,
            r#""documentSymbol":{{"hierarchicalDocumentSymbolSupport":true}},"#,
            r#""inlayHint":{{"resolveSupport":{{"properties":["label.tooltip"]}}}},"#,
            r#""semanticTokens":{{"requests":{{"full":true}},"tokenTypes":{token_types},"tokenModifiers":{token_mods},"formats":["relative"],"overlappingTokenSupport":false,"multilineTokenSupport":true}}"#,
            r#"}},"#, // end textDocument object + comma
            r#""workspace":{{"workspaceFolders":true,"symbol":{{}},"applyEdit":true}}"#,
            // Close: capabilities, params, root object (each `}}` → one `}` in format!)
            r#"}}}}}}"#
        ),
        id = id,
        pid = pid,
        root = root,
        folder = folder,
        token_types = token_types,
        token_mods = token_mods,
    )
}

fn extract_json_id(text: &str) -> Option<u64> {
    // "id": 123 or "id":123
    let key = "\"id\":";
    let mut search = text;
    while let Some(pos) = search.find(key) {
        let after = search[pos + key.len()..].trim_start();
        // skip if this is inside nested structure wrongly — take first numeric id at top-ish
        if after.starts_with('n') {
            // null
            search = &search[pos + key.len()..];
            continue;
        }
        if let Some(n) = after
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
        {
            return Some(n);
        }
        search = &search[pos + key.len()..];
    }
    None
}

/// Extract the JSON string value following `prefix`, unescaping as we go.
/// Stops at the first **unescaped** quote (the old slice-based version
/// truncated values containing `\"`).
fn extract_str(text: &str, prefix: &str) -> Option<String> {
    let start = text.find(prefix)? + prefix.len();
    let mut out = String::new();
    let mut chars = text[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                    {
                        out.push(ch);
                    }
                }
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => break,
            },
            '"' => break,
            c => out.push(c),
        }
    }
    Some(out)
}

fn extract_int(text: &str, prefix: &str) -> Option<i64> {
    text.find(prefix).and_then(|i| {
        let s = text[i + prefix.len()..].trim_start();
        s.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
    })
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn abs_path(path: &str) -> String {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        return p.display().to_string();
    }
    std::env::current_dir()
        .map(|c| c.join(p))
        .unwrap_or_else(|_| PathBuf::from(path))
        .display()
        .to_string()
}

fn path_to_uri(path: &str) -> String {
    let abs = abs_path(path);
    let mut encoded = String::from("file://");
    // Ensure leading / on unix
    #[cfg(unix)]
    {
        if !abs.starts_with('/') {
            encoded.push('/');
        }
    }
    for c in abs.chars() {
        match c {
            ' ' => encoded.push_str("%20"),
            '\\' => encoded.push('/'),
            c if c.is_ascii_alphanumeric()
                || matches!(c, '/' | ':' | '-' | '_' | '.' | '~') =>
            {
                encoded.push(c);
            }
            c => {
                for b in c.to_string().as_bytes() {
                    encoded.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    encoded
}

/// (row, col, title-if-resolved) from one lens object.
fn code_lens_fields(lens: &serde_json::Value) -> (usize, usize, Option<String>) {
    let row = lens
        .get("range")
        .and_then(|r| r.get("start"))
        .and_then(|s| s.get("line"))
        .and_then(|l| l.as_u64())
        .unwrap_or(0) as usize;
    let col = lens
        .get("range")
        .and_then(|r| r.get("start"))
        .and_then(|s| s.get("character"))
        .and_then(|c| c.as_u64())
        .unwrap_or(0) as usize;
    let title = lens
        .get("command")
        .and_then(|c| c.get("title"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string());
    (row, col, title)
}

/// Returns (resolved lenses merged per row, raw unresolved lens objects for
/// `codeLens/resolve`).
fn parse_code_lenses(body: &str) -> (Vec<CodeLens>, Vec<serde_json::Value>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return (Vec::new(), Vec::new());
    };
    let Some(arr) = v.get("result").and_then(|r| r.as_array()) else {
        return (Vec::new(), Vec::new());
    };
    let mut out = Vec::new();
    let mut unresolved = Vec::new();
    for lens in arr {
        let (row, col, title) = code_lens_fields(lens);
        match title {
            Some(title) => out.push(CodeLens { row, col, title }),
            None => unresolved.push(lens.clone()),
        }
    }
    // Merge multiple lenses on same row: "a · b"
    out.sort_by_key(|l| (l.row, l.col));
    let mut merged: Vec<CodeLens> = Vec::new();
    for lens in out {
        if let Some(last) = merged.last_mut() {
            if last.row == lens.row {
                last.title = format!("{} · {}", last.title, lens.title);
                continue;
            }
        }
        merged.push(lens);
    }
    (merged, unresolved)
}

/// Lens from a `codeLens/resolve` response (`result` is a single lens object).
fn parse_resolved_code_lens(body: &str) -> Option<CodeLens> {
    let v = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let lens = v.get("result")?;
    let (row, col, title) = code_lens_fields(lens);
    title.map(|title| CodeLens { row, col, title })
}

/// Insert keeping row order; same-row lenses join as "a · b".
fn merge_code_lens(list: &mut Vec<CodeLens>, lens: CodeLens) {
    if let Some(existing) = list.iter_mut().find(|l| l.row == lens.row) {
        existing.title = format!("{} · {}", existing.title, lens.title);
        return;
    }
    let pos = list.partition_point(|l| l.row < lens.row);
    list.insert(pos, lens);
}

fn parse_call_hierarchy_items(body: &str) -> Vec<crate::call_hierarchy::CallItem> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let result = v.get("result");
    let arr = match result {
        Some(serde_json::Value::Array(a)) => a.clone(),
        Some(obj) if obj.is_object() => vec![obj.clone()],
        _ => return Vec::new(),
    };
    arr.into_iter()
        .filter_map(|item| call_item_from_json(&item))
        .collect()
}

fn parse_call_hierarchy_calls(body: &str) -> Vec<crate::call_hierarchy::CallItem> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(arr) = v.get("result").and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for call in arr {
        // incoming: from, outgoing: to
        let item = call
            .get("from")
            .or_else(|| call.get("to"))
            .cloned()
            .unwrap_or(call.clone());
        if let Some(ci) = call_item_from_json(&item) {
            out.push(ci);
        }
    }
    out
}

fn call_item_from_json(item: &serde_json::Value) -> Option<crate::call_hierarchy::CallItem> {
    let name = item
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("?")
        .to_string();
    let detail = item
        .get("detail")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();
    let kind_n = item.get("kind").and_then(|k| k.as_u64()).unwrap_or(12);
    let kind = symbol_kind_name(kind_n as i64).to_string();
    let uri = item
        .get("uri")
        .and_then(|u| u.as_str())
        .unwrap_or("");
    let path = uri_to_path(uri);
    let range = item
        .get("selectionRange")
        .or_else(|| item.get("range"));
    let row = range
        .and_then(|r| r.get("start"))
        .and_then(|s| s.get("line"))
        .and_then(|l| l.as_u64())
        .unwrap_or(0) as usize;
    let col = range
        .and_then(|r| r.get("start"))
        .and_then(|s| s.get("character"))
        .and_then(|c| c.as_u64())
        .unwrap_or(0) as usize;
    let raw_json = item.to_string();
    Some(crate::call_hierarchy::CallItem {
        name,
        detail,
        kind,
        path,
        row,
        col,
        raw_json,
    })
}

fn uri_to_path(uri: &str) -> String {
    let rest = uri.strip_prefix("file://").unwrap_or(uri);
    // decode %20 etc. minimally
    let mut out = String::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00"),
                16,
            ) {
                out.push(v as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    // macOS sometimes has /Users — fine
    out
}

fn uris_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let pa = uri_to_path(a);
    let pb = uri_to_path(b);
    pa == pb || abs_path(&pa) == abs_path(&pb)
}

fn find_project_root(hint: &str, file: &str) -> String {
    let start = Path::new(file)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(hint));
    let markers = [
        "Cargo.toml",
        "package.json",
        "go.mod",
        "pyproject.toml",
        "setup.py",
        "compile_commands.json",
        "CMakeLists.txt",
        "Gemfile",
        ".git",
    ];
    let mut cur = start;
    for _ in 0..12 {
        for m in &markers {
            if cur.join(m).exists() {
                return cur.display().to_string();
            }
        }
        if !cur.pop() {
            break;
        }
    }
    Path::new(file)
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| hint.to_string())
}

fn command_exists(bin: &str) -> bool {
    // absolute path
    if bin.contains('/') && Path::new(bin).exists() {
        return true;
    }
    Command::new("which")
        .arg(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether a language server is known for this file's extension (defaults only).
pub fn has_server_for(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    default_server_for_ext(&ext).is_some()
}

/// Map file extension → settings language key.
pub fn ext_to_lang_key(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx" => "c",
        "go" => "go",
        "java" => "java",
        "lua" => "lua",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "mdx" => "markdown",
        "sh" | "bash" | "zsh" => "bash",
        "zig" => "zig",
        "php" => "php",
        "rb" => "ruby",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "html" | "htm" => "html",
        "css" | "scss" | "less" => "css",
        "vue" => "vue",
        "svelte" => "svelte",
        "dart" => "dart",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "scala" => "scala",
        "nim" => "nim",
        _ => return None,
    })
}

fn default_server_for_ext(ext: &str) -> Option<&'static str> {
    // Prefer catalog defaults when available
    if let Some(lang) = ext_to_lang_key(ext) {
        for (key, _, cmd) in config::lsp_lang_catalog() {
            if *key == lang {
                return Some(*cmd);
            }
        }
    }
    Some(match ext {
        "rs" => "rust-analyzer",
        "py" | "pyi" => "pyright-langserver --stdio",
        "ts" | "tsx" | "mts" | "cts" => "typescript-language-server --stdio",
        "js" | "jsx" | "mjs" | "cjs" => "typescript-language-server --stdio",
        "c" | "h" => "clangd",
        "cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx" => "clangd",
        "go" => "gopls",
        "java" => "jdtls",
        "lua" => "lua-language-server",
        "php" => "intelephense --stdio",
        "rb" => "solargraph stdio",
        "swift" => "sourcekit-lsp",
        "kt" | "kts" => "kotlin-language-server",
        "cs" => "csharp-ls",
        "html" | "htm" => "vscode-html-language-server --stdio",
        "css" | "scss" | "less" => "vscode-css-language-server --stdio",
        "json" | "jsonc" => "vscode-json-language-server --stdio",
        "yaml" | "yml" => "yaml-language-server --stdio",
        "toml" => "taplo lsp stdio",
        "md" | "mdx" => "marksman server",
        "sh" | "bash" | "zsh" => "bash-language-server start",
        "zig" => "zls",
        "nim" => "nimlsp",
        "ex" | "exs" => "elixir-ls",
        "hs" => "haskell-language-server-wrapper --lsp",
        "scala" => "metals",
        "vue" => "vue-language-server --stdio",
        "svelte" => "svelteserver --stdio",
        "dart" => "dart language-server",
        "r" | "R" => "r-languageserver",
        _ => return None,
    })
}

/// Alias used in tests.
#[cfg(test)]
fn server_for_ext(ext: &str) -> Option<&'static str> {
    default_server_for_ext(ext)
}

fn lang_id(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "go" => "go",
        "c" => "c",
        "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "java" => "java",
        "lua" => "lua",
        "php" => "php",
        "rb" => "ruby",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "mdx" => "markdown",
        "sh" | "bash" | "zsh" => "shellscript",
        "zig" => "zig",
        "vue" => "vue",
        "svelte" => "svelte",
        "dart" => "dart",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "scala" => "scala",
        "nim" => "nim",
        _ => "plaintext",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_lens_split_resolved_unresolved() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":[
            {"range":{"start":{"line":3,"character":0},"end":{"line":3,"character":2}},
             "command":{"title":"2 references","command":"x"}},
            {"range":{"start":{"line":3,"character":4},"end":{"line":3,"character":6}},
             "command":{"title":"run test","command":"y"}},
            {"range":{"start":{"line":9,"character":0},"end":{"line":9,"character":1}},
             "data":{"kind":"references"}}
        ]}"#;
        let (resolved, unresolved) = parse_code_lenses(body);
        // Same-row lenses merge; the command-less lens is queued for resolve.
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].row, 3);
        assert_eq!(resolved[0].title, "2 references · run test");
        assert_eq!(unresolved.len(), 1);
        assert_eq!(
            unresolved[0]["range"]["start"]["line"].as_u64(),
            Some(9)
        );
    }

    #[test]
    fn code_lens_resolve_response_parses_and_merges() {
        let body = r#"{"jsonrpc":"2.0","id":7,"result":
            {"range":{"start":{"line":9,"character":0},"end":{"line":9,"character":1}},
             "command":{"title":"5 references","command":"x"}}}"#;
        let lens = parse_resolved_code_lens(body).unwrap();
        assert_eq!(lens.row, 9);
        assert_eq!(lens.title, "5 references");

        let mut list = vec![CodeLens {
            row: 3,
            col: 0,
            title: "run test".into(),
        }];
        merge_code_lens(&mut list, lens);
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].row, 9);
        // Same-row merge appends with the separator.
        merge_code_lens(
            &mut list,
            CodeLens {
                row: 3,
                col: 5,
                title: "debug".into(),
            },
        );
        assert_eq!(list[0].title, "run test · debug");
    }

    #[test]
    fn code_lens_resolve_without_command_is_dropped() {
        let body = r#"{"jsonrpc":"2.0","id":7,"result":
            {"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":1}}}}"#;
        assert!(parse_resolved_code_lens(body).is_none());
    }

    #[test]
    fn escape_and_uri() {
        let e = escape_json("a\"b\nc");
        assert!(e.contains("\\\""));
        assert!(e.contains("\\n"));
        let u = path_to_uri("/tmp/foo bar.rs");
        assert!(u.starts_with("file://"));
        assert!(u.contains("%20"));
    }

    #[test]
    fn parse_diag_empty_array() {
        let body = r#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///a.rs","diagnostics":[]}}"#;
        let d = parse_diagnostics(body);
        assert!(d.is_empty());
    }

    #[test]
    fn server_map_known() {
        assert!(server_for_ext("rs").is_some());
        assert!(server_for_ext("zig").is_some());
        assert!(server_for_ext("xyz").is_none());
    }

    #[test]
    fn content_length_bytes() {
        let msg = "{\"a\":\"한글\"}";
        assert_ne!(msg.len(), msg.chars().count());
        // header must use byte len
        assert_eq!(msg.as_bytes().len(), msg.len());
    }

    #[test]
    fn decode_semantic_relative_tokens() {
        // legend index: 0=namespace, ... 12=function (default legend)
        let legend = default_semantic_types();
        let function_idx = legend.iter().position(|t| t == "function").unwrap() as u32;
        let keyword_idx = legend.iter().position(|t| t == "keyword").unwrap() as u32;
        // line 0: "fn main" — keyword at 0 len 2, function at 3 len 4
        // data: [dLine, dStart, len, type, mods]
        let data = vec![
            0, 0, 2, keyword_idx, 0, // "fn"
            0, 3, 4, function_idx, 0, // "main" (delta start 3 from 0)
        ];
        let lines = ["fn main() {}"];
        let toks = decode_semantic_tokens(&data, &legend, &lines);
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].1, 0);
        assert_eq!(toks[0].2, 2);
        assert!(matches!(toks[0].0, TokenKind::Keyword));
        assert_eq!(toks[1].1, 3);
        assert_eq!(toks[1].2, 7);
        assert!(matches!(toks[1].0, TokenKind::Function));
    }

    #[test]
    fn workspace_edit_changes_map() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"changes":{"file:///tmp/xei_we_test.rs":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":5}},"newText":"hello"}]}}}"#;
        // create file
        let path = "/tmp/xei_we_test.rs";
        let _ = std::fs::write(path, "world\n");
        let edits = parse_workspace_edit(body);
        assert!(!edits.is_empty(), "expected edits");
        assert!(edits[0].text.contains("hello"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn install_hint_mentions_binary() {
        let h = install_hint("rust-analyzer");
        assert!(h.contains("rust-analyzer"));
        assert!(h.contains("install"));
    }

    #[test]
    fn initialize_request_is_valid_json() {
        let s = build_initialize_request(1, 42, "file:///tmp/xei-proj", "xei");
        // brace balance
        let mut bal = 0i32;
        for c in s.chars() {
            match c {
                '{' => bal += 1,
                '}' => bal -= 1,
                _ => {}
            }
            assert!(bal >= 0, "negative brace balance in {s}");
        }
        assert_eq!(bal, 0, "unbalanced braces: {s}");
        // must contain core fields
        assert!(s.contains("\"method\":\"initialize\""));
        assert!(s.contains("\"processId\":42"));
        assert!(s.contains("semanticTokens"));
        assert!(s.contains("workspaceFolders"));
    }

    #[test]
    fn initialize_survives_rust_analyzer() {
        if !command_exists("rust-analyzer") {
            return;
        }
        // Smoke: spawn RA, send our init, expect a result (not disconnect).
        use std::io::{Read, Write};
        use std::process::{Command, Stdio};
        use std::time::Duration;
        let mut child = Command::new("rust-analyzer")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rust-analyzer");
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let body = build_initialize_request(1, std::process::id(), "file:///tmp", "tmp");
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        stdin.write_all(header.as_bytes()).unwrap();
        stdin.write_all(body.as_bytes()).unwrap();
        stdin.flush().unwrap();
        // read one message with timeout-ish
        let start = std::time::Instant::now();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(n) = stdout.read(&mut tmp) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    // got headers; try to find content-length and full body
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = std::str::from_utf8(&buf[..pos]).unwrap_or("");
                        let mut len = 0usize;
                        for line in header.lines() {
                            if let Some(rest) = line
                                .to_ascii_lowercase()
                                .strip_prefix("content-length:")
                            {
                                len = rest.trim().parse().unwrap_or(0);
                            }
                        }
                        let body_start = pos + 4;
                        if buf.len() >= body_start + len {
                            let body = std::str::from_utf8(&buf[body_start..body_start + len])
                                .unwrap_or("");
                            assert!(
                                body.contains("\"result\"") || body.contains("capabilities"),
                                "unexpected RA response: {body}"
                            );
                            assert!(
                                !is_jsonrpc_error(body),
                                "init should not be error: {body}"
                            );
                            let _ = child.kill();
                            return;
                        }
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let _ = child.kill();
        panic!("timed out waiting for rust-analyzer initialize response");
    }

    #[test]
    fn char_utf16_roundtrip() {
        let line = "a𝕏b"; // 𝕏 is 2 utf-16 units
        assert_eq!(char_to_utf16_col(line, 0), 0);
        assert_eq!(char_to_utf16_col(line, 1), 1);
        assert_eq!(char_to_utf16_col(line, 2), 3); // after 𝕏
        assert_eq!(utf16_to_char_col(line, 3), 2);
    }

    #[test]
    fn jsonrpc_error_detect() {
        assert!(is_jsonrpc_error(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}"#
        ));
        assert!(!is_jsonrpc_error(
            r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"errorCodes":[]}}}"#
        ));
    }

    #[test]
    fn utf16_cjk_column() {
        let line = "a한b"; // a=1, 한=1 utf16? actually 한 is one BMP char = 1 utf16 unit
        assert_eq!(utf16_to_char_col(line, 0), 0);
        assert_eq!(utf16_to_char_col(line, 1), 1);
        assert_eq!(utf16_to_char_col(line, 2), 2);
        assert_eq!(utf16_to_char_col(line, 3), 3);
    }

    #[test]
    fn extract_str_handles_escaped_quotes() {
        let body = r#"{"message":"expected \"foo\", found bar\nhere"}"#;
        let m = extract_str(body, "\"message\":\"").unwrap();
        assert_eq!(m, "expected \"foo\", found bar\nhere");
    }

    #[test]
    fn diag_cols_convert_utf16_surrogate_pairs() {
        // '𝕏' (U+1D54F) is 2 UTF-16 units but 1 char.
        let doc = "𝕏ab";
        let mut diags = vec![Diagnostic {
            row: 0,
            col_start: 2, // UTF-16 col of 'a'
            col_end: 3,
            message: "m".into(),
            severity: DiagnosticSeverity::Error,
        }];
        diag_cols_utf16_to_chars(&mut diags, doc);
        assert_eq!(diags[0].col_start, 1); // char index of 'a'
        assert_eq!(diags[0].col_end, 2);
    }

    #[test]
    fn parse_semantic_data_array() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"data":[0,0,2,15,0,0,3,4,12,0]}}"#;
        let d = parse_semantic_data(body);
        assert_eq!(d, vec![0, 0, 2, 15, 0, 0, 3, 4, 12, 0]);
    }
}
