//! Owns `App` + shell state; single place that calls dispatch and compose.

use suisei_core::app::{App, Mode};
use suisei_core::buffer::Position;
use suisei_core::key::{KeyCode, KeyEvent, KeyModifiers};

use crate::compositor::{FrameDiff, ShellState, compose};

/// Ticks between post-edit `textDocument/didChange` syncs. The tick is 50 ms,
/// so this coalesces a typing run into one full-text notification every 150 ms
/// — `App::sync_lsp_document` is version-gated, so an idle frame costs nothing
/// and only a changed buffer pays the O(file) join.
const LSP_SYNC_TICKS: u32 = 3;

/// Ticks between dirty-flag re-checks (~1 s at the 50 ms tick). `App::modified`
/// is exact for clean → dirty but was a latch the other way, so a buffer put
/// back to its on-disk text stayed marked dirty; this re-derives it. Costs one
/// hash, and only while dirty and only when the text moved.
const DIRTY_RECHECK_TICKS: u32 = 20;

/// Ticks between external-file checks (~1 s). Metadata polling is intentionally
/// low-frequency, and only a transition (changed/recreated/deleted) advances a
/// frame, so an idle editor still does no paint work.
const EXTERNAL_FILE_CHECK_TICKS: u32 = 20;

/// Ticks between daemon status reports. At the 50 ms tick this is ~1 s, which
/// bounds the cost of building the status (`project_root` walks the filesystem)
/// while still feeling live in the menu bar. The reporter itself then skips
/// anything unchanged — see `daemon_report::ReportGate`.
const DAEMON_REPORT_TICKS: u32 = 20;

/// One indent level, inserted by Tab and by auto-indent on Enter.
const INDENT: &str = "    ";

/// `Status::lsp_state`: 0 none · 1 starting · 2 indexing · 3 ready · 4 error.
fn lsp_state_code(lsp: &suisei_core::lsp::LspClient) -> u8 {
    if lsp.error.is_some() {
        return 4;
    }
    if lsp.server_running {
        // Handshaked, but rust-analyzer answers thinly until its indexing,
        // metadata and build-script passes close — that window is `$/progress`.
        return if lsp.is_busy() { 2 } else { 3 };
    }
    u8::from(lsp.is_starting())
}

/// `Status::dap_state`: 0 none · 1 running · 2 paused.
fn dap_state_code(state: suisei_core::dap::DapState) -> u8 {
    use suisei_core::dap::DapState;
    match state {
        DapState::Idle => 0,
        DapState::Starting | DapState::Running | DapState::Ending => 1,
        // Stopped at a breakpoint or step — a live session, just not advancing.
        DapState::Stopped => 2,
    }
}

/// One row in the Breakpoints navigator (face FFI).
#[derive(Debug, Clone)]
pub struct BreakpointRow {
    pub path: String,
    pub name: String,
    pub line_1based: u32,
    pub verified: bool,
    pub condition: String,
    pub has_log: bool,
}

pub struct Engine {
    pub app: App,
    pub shell: ShellState,
    pub last_diff: FrameDiff,
    pub frame_gen: u64,
    /// Native Source Control has an independent observation channel. Its C
    /// snapshot is large, so editor frame generations must not be used as its
    /// invalidation token.
    git_wb_signature: u64,
    git_wb_generation: u64,
    /// True after mouse-down until mouse-up (face pointer lifecycle).
    pointer_down: bool,
    /// True once the pointer moved off the down cell (enters Visual).
    pointer_moved: bool,
    /// Outline is expensive on large files — rebuild only when buffer/path changes.
    outline_cache: Vec<crate::compositor::OutlineItemScene>,
    outline_cache_ver: u64,
    outline_cache_path: Option<std::path::PathBuf>,
    /// Tick counter for low-frequency idle work (outline refresh).
    tick_count: u32,
    /// Missing paths across all open document tabs. The active document owns
    /// the close/preserve policy in `App::check_external_change`; this cache is
    /// what lets inactive tabs acquire/clear their warning glyph without
    /// forcing a full recompose every second while a file remains absent.
    missing_tab_ids: std::collections::HashSet<suisei_core::app::BufferId>,
    /// Parked terminal sessions (VS Code-style multi-shell). The ACTIVE session
    /// always lives in `app.terminal` so all core routing (keys/paste/resize)
    /// keeps working untouched; switching swaps sessions in and out.
    parked_terminals: Vec<suisei_core::term::Terminal>,
    /// Index of the active session within the conceptual list
    /// `[..parked[0..active], ACTIVE, parked[active..]..]`.
    active_terminal: usize,
    /// Shadow WAL — crash-recovery journal for unsaved buffers (D0).
    pub journal: crate::journal::Journal,
    /// Grid the face last measured for the terminal panel, in cells. The face
    /// knows the panel's real size; the editor viewport is only a stand-in for
    /// before it has reported one.
    face_terminal_grid: Option<(u16, u16)>,
    /// Per-shell content generation: bumped whenever a pane terminal's screen
    /// changes. Shipped as a u16 in `SuiseiPaneC.term_gen` so the face skips
    /// re-pulling a ~300 KiB grid it already has — pulling one per keystroke
    /// per idle terminal was pure churn.
    pane_term_gens: std::collections::HashMap<suisei_core::split::TerminalId, u64>,
    /// Pushes LSP/DAP/project state to the daemon for the menu-bar agent.
    /// `None` outside the real app — tests must not report into the developer's
    /// running daemon, so only the FFI constructor turns it on.
    reporter: Option<crate::daemon_report::Reporter>,
    /// Background syntax parser (A1-6): the keystroke path ships a text
    /// snapshot per buffer version and never waits; frames come back and are
    /// adopted at the next recompose or tick.
    syntax_worker: suisei_core::syntax_worker::SyntaxWorker,
    /// Settings account page. Independent of chrome / git workbench so a
    /// profile refresh cannot republish the editor tree.
    pub github_account: crate::github_account::GitHubAccount,
    /// Settings Software Update page. Independent of chrome.
    pub update_generation: u64,
    /// `(version, path, window)` of the outstanding parse request — one
    /// request per change, so a typing run coalesces on the worker.
    syntax_requested: Option<(u64, String, std::ops::Range<usize>)>,
    /// Buffer version of the tokens `app.syntax` currently paints.
    syntax_applied: u64,
    /// Mirror of the worker's pre-parse cache size (FFI diagnostic).
    syntax_cached: usize,
    /// The outline is behind the buffer and waiting for a pause to catch up.
    outline_dirty: bool,
    outline_built_at: Option<std::time::Instant>,
}

impl Engine {
    /// Read-only core access for the cheap FFI probes (no snapshot decode).
    pub fn app(&self) -> &App {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// Pre-parse a file for the project index. Background work — never touches
    /// the document the user is editing.
    pub fn prewarm_file(&mut self, path: &str) -> bool {
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_string());
        // Parsed on the syntax worker — the cache lives there with the trees.
        self.syntax_worker
            .request(suisei_core::syntax_worker::SyntaxRequest::Prewarm {
                path: path.to_string(),
                ext,
                text,
            });
        true
    }

    pub fn cached_parses(&self) -> usize {
        self.syntax_cached
    }

    /// Boot pipeline: build every language grammar on the syntax worker now, so
    /// the first file opened — of any language — highlights without a cold
    /// parser+query build mid-view. Non-blocking; the worker warms in the
    /// background while the launch splash is up.
    pub fn warm_grammars(&self) {
        self.syntax_worker
            .request(suisei_core::syntax_worker::SyntaxRequest::WarmGrammars);
    }

    pub fn clear_scroll_intent(&mut self) {
        self.app.scroll_intent = suisei_core::app::ScrollIntent::None;
        if let Some(c) = self.last_diff.chrome.as_mut() {
            c.scroll_intent = 0;
        }
    }

    pub fn frame_gen(&self) -> u64 {
        self.frame_gen
    }

    pub fn git_wb_generation(&self) -> u64 {
        self.git_wb_generation
    }

    pub fn new() -> Self {
        let mut app = App::new();
        // Same as TUI main: load ~/.xei.toml theme + editor opts.
        app.apply_config();
        app.message = "Suisei · same keys as xei · Ctrl+, settings".into();
        app.stage.w = 900.0; // 100 columns
        app.stage.h = 720.0; // 40 rows
        let git_wb_signature = app.git_wb.native_snapshot_signature();
        Self {
            app,
            shell: ShellState::default(),
            last_diff: FrameDiff::empty(0),
            frame_gen: 0,
            git_wb_signature,
            git_wb_generation: 0,
            pointer_down: false,
            pointer_moved: false,
            outline_cache: Vec::new(),
            outline_cache_ver: u64::MAX,
            outline_cache_path: None,
            tick_count: 0,
            missing_tab_ids: std::collections::HashSet::new(),
            parked_terminals: Vec::new(),
            active_terminal: 0,
            journal: crate::journal::Journal::new(),
            face_terminal_grid: None,
            pane_term_gens: std::collections::HashMap::new(),
            reporter: None,
            syntax_worker: suisei_core::syntax_worker::SyntaxWorker::start(),
            syntax_requested: None,
            syntax_applied: 0,
            syntax_cached: 0,
            outline_dirty: false,
            outline_built_at: None,
            github_account: crate::github_account::GitHubAccount::new(),
            update_generation: 0,
        }
    }

    /// Start pushing status to the daemon. Called once from the FFI
    /// constructor: only the real app should report, never a test run.
    pub fn start_daemon_reporting(&mut self) {
        if self.reporter.is_none() {
            self.reporter = Some(crate::daemon_report::Reporter::spawn());
        }
    }

    /// What the menu-bar agent should say about this editor. Built from live
    /// `App` state; the daemon fills in its own uptime and health.
    pub fn daemon_status(&self) -> suisei_daemon::protocol::Status {
        suisei_daemon::protocol::Status {
            lsp_sessions: u16::from(self.app.lsp.server_running),
            lsp_state: lsp_state_code(&self.app.lsp),
            dap_state: dap_state_code(self.app.dap.state),
            // Both are the daemon's own and are ignored on the way in.
            health: 0,
            uptime_secs: 0,
            project: self.daemon_project_root(),
        }
    }

    /// The project the status should name. Prefers the navigator's root (what
    /// the user actually opened) over the walked-up root of the current file,
    /// and reports nothing rather than guessing: `App::project_root` falls back
    /// to the process cwd, which for a launched `.app` is `/`.
    fn daemon_project_root(&self) -> String {
        if !self.app.explorer.entries.is_empty() {
            return self.app.explorer.cwd.display().to_string();
        }
        match self.app.filename {
            Some(_) => self.app.project_root().display().to_string(),
            None => String::new(),
        }
    }

    // ── Multi-session terminal (VS Code-style shell list) ──

    pub fn terminal_session_count(&self) -> u32 {
        (self.parked_terminals.len() + 1) as u32
    }

    pub fn terminal_active_session(&self) -> u32 {
        self.active_terminal.min(self.parked_terminals.len()) as u32
    }

    /// Park the active shell and spawn a fresh one.
    pub fn terminal_new_session(&mut self) {
        if !self.app.terminal.open {
            self.app.toggle_terminal_side();
        }
        self.ensure_terminal_started();
        let mut fresh = suisei_core::term::Terminal::new();
        fresh.open = true;
        // Where the terminal lives is the layout tree's business now, so a
        // session swap carries nothing about placement.
        std::mem::swap(&mut self.app.terminal, &mut fresh);
        // `fresh` now holds the previously active session — park it in place.
        self.parked_terminals
            .insert(self.active_terminal.min(self.parked_terminals.len()), fresh);
        self.active_terminal = self.parked_terminals.len();
        self.ensure_terminal_started();
        self.app.mode = suisei_core::app::Mode::Terminal;
        self.shell.dirty = true;
        self.recompose();
    }

    /// Swap session `idx` (conceptual list order) into `app.terminal`.
    pub fn terminal_select_session(&mut self, idx: u32) {
        let idx = idx as usize;
        let count = self.parked_terminals.len() + 1;
        if idx >= count || idx == self.active_terminal {
            return;
        }
        // Parked slot for `idx`: indices skip the active position.
        let parked_idx = if idx < self.active_terminal {
            idx
        } else {
            idx - 1
        };
        if parked_idx >= self.parked_terminals.len() {
            return;
        }
        std::mem::swap(
            &mut self.app.terminal,
            &mut self.parked_terminals[parked_idx],
        );
        self.active_terminal = idx;
        self.app.terminal.open = true;
        self.app.mode = suisei_core::app::Mode::Terminal;
        self.shell.dirty = true;
        self.recompose();
    }

    /// Close session `idx`; closing the active one promotes a neighbor.
    pub fn terminal_close_session(&mut self, idx: u32) {
        let idx = idx as usize;
        let count = self.parked_terminals.len() + 1;
        if idx >= count {
            return;
        }
        if idx == self.active_terminal {
            self.app.terminal.shutdown();
            if let Some(mut next) = self.parked_terminals.pop() {
                std::mem::swap(&mut self.app.terminal, &mut next);
                self.app.terminal.open = true;
                self.active_terminal = self.parked_terminals.len();
            } else {
                // Last shell closed → close the panel.
                self.app.terminal.open = false;
                if matches!(self.app.mode, Mode::Terminal) {
                    self.app.mode = Mode::Editor;
                }
                self.active_terminal = 0;
            }
        } else {
            let parked_idx = if idx < self.active_terminal {
                idx
            } else {
                idx - 1
            };
            if parked_idx < self.parked_terminals.len() {
                let mut t = self.parked_terminals.remove(parked_idx);
                t.shutdown();
                if idx < self.active_terminal {
                    self.active_terminal -= 1;
                }
            }
        }
        self.shell.dirty = true;
        self.recompose();
    }

    // ─── GUI semantic editing commands ────────────────────────────────────────
    //
    // Semantic edit commands the face calls directly (there is no key
    // `c`, `d`). Core stays modal internally (shared with the xei TUI) but the
    // GUI never surfaces modes — these commands handle transitions invisibly.

    /// True when an editable text document (rather than a viewer, panel or the
    /// terminal) has focus. Media panes deliberately own no hidden text input:
    /// typing over an MP3 used to mutate its empty backing buffer and put a
    /// meaningless dirty dot on a file the save path correctly refuses to
    /// write.
    fn is_editing_mode(&self) -> bool {
        matches!(self.app.mode, Mode::Editor)
            && matches!(self.app.live_tab_kind(), suisei_core::media::FileKind::Text)
    }

    /// Type a printable character at the caret(s) — GUI fast path.
    ///
    /// Delegates to the mode-independent Selection-model edit: replaces any
    /// selection, types at every caret, no synthetic vim keystrokes. Typing
    /// always types.
    pub fn gui_type_char(&mut self, ch: char) {
        if !self.is_editing_mode() {
            return;
        }
        self.editing_with_optimistic_colour(|e| e.app.gui_insert_text(&ch.to_string()));

        self.app.completion_after_typing();
        self.app.update_scroll();
        self.recompose_scroll();
    }

    /// Backspace: delete the selection, or one grapheme before each caret.
    /// Run a single-line edit and slide the painted spans to match it.
    ///
    /// Optimistic colour: the parse is async and correct, but its answer is a
    /// tick away, so a just-typed character would otherwise be painted with the
    /// PREVIOUS frame's spans — the reported "colour is one beat late".
    /// Sliding what is already on screen costs a pass over one row's spans
    /// (measured 0.22 ms / 0.57 ms per key at 1k / 4.5k lines, against 4.1 /
    /// 6.7 ms for waiting on the parse).
    ///
    /// Caret captured before the edit and the width taken from the line's own
    /// length, so auto-pair — two characters inserted, caret moved one — is
    /// still described exactly. Multi-caret edits are skipped: they are not the
    /// typing case and one row's arithmetic cannot describe them.
    fn editing_with_optimistic_colour(&mut self, edit: impl FnOnce(&mut Self)) {
        let before = (self.app.sel.len() == 1).then(|| {
            let at = self.app.sel.primary().head;
            (
                at,
                self.app.buffer.line(at.row).chars().count(),
                self.app.buffer.line_count(),
            )
        });

        edit(self);

        let Some((at, before_len, before_lines)) = before else {
            return;
        };

        // A change in LINE COUNT first, because it renumbers every token below
        // the caret and the column arithmetic underneath assumes those numbers
        // are still right. Return was not handled at all: the stale colours the
        // paint path deliberately keeps showing became stale colours on the
        // wrong rows, which is why highlighting looked like it vanished for a
        // beat on every Enter.
        let after_lines = self.app.buffer.line_count();
        match after_lines.cmp(&before_lines) {
            std::cmp::Ordering::Greater if after_lines == before_lines + 1 => {
                self.app.syntax.nudge_for_split(at.row, at.col);
                return;
            }
            std::cmp::Ordering::Less if after_lines + 1 == before_lines => {
                // Backspace at a line start joins this row onto the one above,
                // and the caret has already moved there.
                let head = self.app.sel.primary().head;
                self.app.syntax.nudge_for_join(head.row, head.col);
                return;
            }
            // A paste or a block delete moves more rows than one nudge can
            // describe. The worker's next frame is the answer; guessing would
            // paint confident nonsense until it arrives.
            std::cmp::Ordering::Greater | std::cmp::Ordering::Less => return,
            std::cmp::Ordering::Equal => {}
        }

        let after_len = self.app.buffer.line(at.row).chars().count();
        match after_len.cmp(&before_len) {
            std::cmp::Ordering::Greater => {
                self.app
                    .syntax
                    .nudge_for_insert(at.row, at.col, after_len - before_len)
            }
            std::cmp::Ordering::Less => {
                self.app
                    .syntax
                    .nudge_for_delete(at.row, at.col, before_len - after_len)
            }
            std::cmp::Ordering::Equal => {}
        }
    }

    pub fn gui_delete_backward(&mut self) {
        if !self.is_editing_mode() {
            return;
        }
        self.editing_with_optimistic_colour(|e| e.app.gui_delete_backward());
        self.app.update_scroll();
        self.recompose_scroll();
    }

    /// Forward-delete: delete the selection, or one grapheme after each caret.
    pub fn gui_delete_forward(&mut self) {
        if !self.is_editing_mode() {
            return;
        }
        self.app.gui_delete_forward();
        self.app.update_scroll();
        self.recompose_scroll();
    }

    /// Esc: collapse the selection and dismiss editor overlays. No mode dance,
    /// no synthetic `i` — editing continues immediately.
    pub fn gui_escape(&mut self) {
        // Esc reaches the dispatch so an open panel can close itself; the
        // caret collapse below is the editor's own half of the contract.
        self.dispatch_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        if self.is_editing_mode() {
            self.app.caret_collapse();
        }
        self.recompose_scroll();
    }

    /// Give the editor the keyboard back — the face calls this when the user
    /// clicks into the text.
    ///
    /// This used to force vim Insert mode, then (once typing went modeless)
    /// shrank to "clear a stray visual selection". Neither applies now: the
    /// only thing that can hold the keyboard is a panel, so releasing it is
    /// the whole job. A panel that owns typed characters is dismissed the same
    /// way its own Esc would.
    pub fn gui_focus_editor(&mut self) {
        if matches!(self.app.mode, Mode::Editor) {
            return;
        }
        match self.app.mode {
            Mode::Search => self.app.cancel_search(),
            Mode::Palette => self.app.palette.close(),
            _ => {}
        }
        self.app.mode = Mode::Editor;
        self.recompose();
    }

    /// Route a caret-navigation key straight to the Selection model, bypassing
    /// the vim command machine. Returns true when it consumed the key.
    ///
    /// Only fires when the editor itself owns the keyboard (plain editing, no
    /// chrome panel or terminal focused), so palette/explorer/search arrows are
    /// untouched. Shift extends the selection; Alt makes it a word motion; the
    /// bare arrow moves the caret and collapses any selection.
    fn try_gui_navigation(&mut self, ev: KeyEvent) -> bool {
        use suisei_core::gui_edit::Motion;
        if !self.text_editor_owns_keys() {
            return false;
        }
        let m = ev.modifiers;
        let word = m.contains(KeyModifiers::ALT);
        let to_line_or_doc = m.contains(KeyModifiers::SUPER);
        let motion = match ev.code {
            KeyCode::Left if word => Motion::WordLeft,
            KeyCode::Left if to_line_or_doc => Motion::LineStart,
            KeyCode::Left => Motion::Left,
            KeyCode::Right if word => Motion::WordRight,
            KeyCode::Right if to_line_or_doc => Motion::LineEnd,
            KeyCode::Right => Motion::Right,
            KeyCode::Up if to_line_or_doc => Motion::DocStart,
            KeyCode::Up => Motion::Up,
            KeyCode::Down if to_line_or_doc => Motion::DocEnd,
            KeyCode::Down => Motion::Down,
            KeyCode::Home => Motion::LineStart,
            KeyCode::End => Motion::LineEnd,
            KeyCode::PageUp => Motion::PageUp,
            KeyCode::PageDown => Motion::PageDown,
            _ => return false,
        };
        // A prior legacy path may have moved the cursor; make the selection's
        // primary head agree before we move it.
        if self.app.sel.primary().is_empty()
            && self.app.sel.primary().head != self.app.buffer.cursor()
        {
            self.app.sync_sel_to_cursor();
        }
        if m.contains(KeyModifiers::SHIFT) {
            self.app.caret_extend(motion);
        } else {
            self.app.caret_move(motion);
        }
        true
    }

    /// Route a text-editing key (character, backspace, delete, enter) straight
    /// to the semantic Selection-model edits — typing always types, with no
    /// mode gate and no synthetic `i`. Returns true when it consumed the key.
    ///
    /// Control/Super combos are shortcuts (copy, save, …) and fall through to
    /// the legacy dispatch; so do chrome panels and the terminal.
    /// While the completion popup is up it owns confirm and up/down — but
    /// nothing else, and only while it is up. Without this the popup could be
    /// opened and never accepted: the accept path lived in the vim insert
    /// handler, so the GUI had no way to take a suggestion at all.
    fn try_completion_keys(&mut self, ev: KeyEvent) -> bool {
        if !self.app.completions.active || !self.text_editor_owns_keys() {
            return false;
        }
        let m = ev.modifiers;
        if m.contains(KeyModifiers::CONTROL) || m.contains(KeyModifiers::SUPER) {
            return false;
        }
        match ev.code {
            KeyCode::Tab | KeyCode::Enter => self.app.completion_accept(),
            KeyCode::Down => self.app.completion_move(true),
            KeyCode::Up => self.app.completion_move(false),
            KeyCode::Esc => {
                self.app.completions.deactivate();
                true
            }
            _ => false,
        }
    }

    fn try_gui_edit(&mut self, ev: KeyEvent) -> bool {
        if !self.text_editor_owns_keys() {
            return false;
        }
        let m = ev.modifiers;
        if m.contains(KeyModifiers::CONTROL) || m.contains(KeyModifiers::SUPER) {
            return false; // shortcut, not text
        }
        // Every edit here goes through `editing_with_optimistic_colour`, and
        // three of them did not.
        //
        // The wrapper slides the painted spans to match the edit, so what is on
        // screen stays coloured until the async parse answers. Without it the
        // tokens keep the rows and columns they had BEFORE the edit, and the
        // face draws the ones that no longer match in `colors.fg` — white, in
        // dark mode. Enter was the loudest, because it renumbers every row
        // below the caret at once: the reported flash of white on every Return.
        match ev.code {
            KeyCode::Char(c) => {
                self.editing_with_optimistic_colour(|e| {
                    e.app.gui_insert_text(&c.to_string())
                });
                self.app.completion_after_typing();
            }
            KeyCode::Enter => {
                self.editing_with_optimistic_colour(|e| {
                    e.app.gui_insert_newline(INDENT)
                });
            }
            KeyCode::Backspace => {
                self.editing_with_optimistic_colour(|e| e.app.gui_delete_backward());
                self.app.completion_after_typing();
            }
            KeyCode::Delete => {
                self.editing_with_optimistic_colour(|e| e.app.gui_delete_forward());
            }
            // Tab indents. It used to reach vim's `handle_normal`, where Tab is
            // the jumplist-forward command (`Ctrl+I`) — pressing Tab in the
            // editor jumped somewhere else in the file instead of inserting.
            KeyCode::Tab => self.app.gui_insert_text(INDENT),
            // Esc collapses a multi-caret / selection back to one caret. In the
            // editor there is no overlay left for it to close — an overlay owns
            // the keyboard while it is up.
            KeyCode::Esc => self.app.caret_collapse(),
            _ => return false,
        }
        true
    }

    /// Does the editor itself own the keyboard right now?
    ///
    /// Ownership is a *mode* question. It is deliberately NOT asked of panel
    /// visibility flags like `explorer.open`, which in the GUI only mean "the
    /// docked navigator has entries" — `suisei_engine_open_path` sets that on
    /// every project/file open, without taking focus. Gating on it once sent
    /// every keystroke to the vim command machine for a whole session.
    fn editor_owns_keys(&self) -> bool {
        matches!(self.app.mode, Mode::Editor) && !self.app.terminal_window_focused()
    }

    /// Keyboard ownership and text-editability are separate facts. A media
    /// viewer still seals ordinary keys away from the legacy Vim dispatcher,
    /// but must never offer them to the Selection-model editing table.
    fn text_editor_owns_keys(&self) -> bool {
        self.editor_owns_keys()
            && matches!(self.app.live_tab_kind(), suisei_core::media::FileKind::Text)
    }

    /// Standard text-responder chords are not application commands in a media
    /// viewer. They must be consumed here before Core's shared dispatcher sees
    /// the viewer's empty compatibility buffer (Cmd+Backspace was the clearest
    /// route to a phantom dirty flag). Real app shortcuts continue below.
    fn is_text_only_shortcut(ev: KeyEvent) -> bool {
        let command = ev.modifiers.contains(KeyModifiers::SUPER)
            || ev.modifiers.contains(KeyModifiers::CONTROL);
        command
            && matches!(
                ev.code,
                KeyCode::Char('a' | 'A' | 'c' | 'C' | 'v' | 'V' | 'x' | 'X' | 'z' | 'Z')
                    | KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Backspace
                    | KeyCode::Delete
            )
    }

    /// Keys that still belong to `App::dispatch` while the editor has focus:
    /// modifier chords are application shortcuts (Ctrl+F explorer, Ctrl+, …)
    /// and function keys drive the debugger. Everything else unmodified is the
    /// editor's own business.
    fn is_app_shortcut(ev: KeyEvent) -> bool {
        ev.modifiers.contains(KeyModifiers::CONTROL)
            || ev.modifiers.contains(KeyModifiers::SUPER)
            || matches!(ev.code, KeyCode::F(_))
    }

    pub fn dispatch_key(&mut self, ev: KeyEvent) {
        // Pure-GUI editing + navigation: characters, backspace, delete, enter,
        // and the arrows drive the Selection model directly — never the vim
        // command machine. The core stays modal internally; the face never
        // surfaces a mode, and there is no synthetic `i`.
        let caret_pre = self.app.buffer.cursor();
        let ver_pre = self.app.buffer.version();
        if self.try_completion_keys(ev) || self.try_gui_navigation(ev) || self.try_gui_edit(ev) {
            // Only re-anchor the viewport when something actually moved — a
            // no-op arrow at a document edge must not yank an absolute scroll.
            if self.app.buffer.cursor() != caret_pre || self.app.buffer.version() != ver_pre {
                self.app.update_scroll();
            }
            self.recompose_scroll();
            return;
        }
        // SEALED: while the editor owns the keyboard, the two tables above are
        // the whole contract. A key they did not take is dropped here rather
        // than falling through to `App::dispatch`, whose Normal-mode handler is
        // the vim command interpreter — that fallthrough is what turned `z`
        // into a fold prefix and Tab into a jumplist jump. Only real
        // application shortcuts still pass.
        if self.editor_owns_keys() && !Self::is_app_shortcut(ev) {
            return;
        }
        if self.editor_owns_keys()
            && self.app.live_tab_kind().is_viewer()
            && Self::is_text_only_shortcut(ev)
        {
            return;
        }
        // Hard rule: chrome-only keys (explorer / XLC / settings / SCM / git wb)
        // must not call update_scroll or clobber caret/scroll.
        let caret_before = self.app.buffer.cursor();
        let scroll_before = self.app.scroll;
        let ver_before = self.app.buffer.version();
        let file_before = self.app.filename.clone();
        let mode_before = self.app.mode;
        let explorer_before = self.app.explorer.open;
        let term_before = self.app.terminal.open;
        let scm_before = self.app.scm.visible();
        let git_wb_before = self.app.git_wb.open;
        let palette_before = self.app.palette.open;
        self.app.dispatch(ev);
        // GUI contract: no core-side close animations. `preview.close()` only
        // sets `closing` and keeps `open` until the TUI's per-frame
        // anim_progress() finishes it — a tick this face never runs, so the
        // panel stayed open forever and re-toggling re-opened it instead.
        if self.app.preview.closing {
            self.app.close_preview_immediate();
        }
        let caret_after = self.app.buffer.cursor();
        let ver_after = self.app.buffer.version();
        let file_after = self.app.filename.clone();
        let buffer_or_caret_changed =
            caret_before != caret_after || ver_before != ver_after || file_before != file_after;
        if buffer_or_caret_changed {
            self.app.update_scroll();
            // Coherence: a non-navigation key (typing, edit) moved the cursor
            // through the legacy path. Collapse the GUI selection to a caret
            // there so the next Shift+Arrow starts from the right place — and
            // so a stale highlight never lingers after typing.
            if matches!(self.app.mode, Mode::Editor)
                && self.app.sel.primary().head != self.app.buffer.cursor()
            {
                self.app.sync_sel_to_cursor();
            }
        } else {
            // Restore in case a side-effect nudged them (mode toggles).
            self.app.buffer.cursor = caret_before;
            self.app.scroll = scroll_before;
        }
        // Hot path: typing / motions without shell surface changes skip SCM/git/outline rebuild.
        // Chrome-owned modes (XLC / search / palette / explorer / settings / SCM / git …)
        // mutate scene data the scroll-patch never rebuilds — always full compose there,
        // else typed input (`:help`, palette filter, explorer j/k) paints stale.
        let chrome_mode = |m: Mode| !matches!(m, Mode::Editor | Mode::Terminal | Mode::Preview);
        let shell_surface_changed = mode_before != self.app.mode
            || chrome_mode(self.app.mode)
            || file_before != file_after
            || explorer_before != self.app.explorer.open
            || term_before != self.app.terminal.open
            || scm_before != self.app.scm.visible()
            || git_wb_before != self.app.git_wb.open
            || palette_before != self.app.palette.open
            || self.shell.dirty;
        if shell_surface_changed {
            self.recompose();
        } else {
            self.recompose_scroll();
        }
    }

    /// Drain PTY / git side-effects. Returns current `frame_gen` (bumped only when recomposed).
    pub fn tick(&mut self, _dt_ms: u32) -> u64 {
        // Drain PTY / background side-effects (same idea as TUI main loop).
        let mut need_full = false;
        // Idle outline refresh: typing keeps the light path (never rebuilds the
        // outline); catch up ~600ms after the buffer settles.
        self.tick_count = self.tick_count.wrapping_add(1);
        // Not inside the poll below: the flash lasts what it lasts, and tying
        // its end to a 20-tick boundary would quantise it to the poll.
        if self.app.has_live_marks() {
            let before = self.app.live_gen;
            self.app.expire_live_marks();
            if self.app.live_gen != before {
                self.shell.dirty = true;
            }
        }
        if self.tick_count % 12 == 0 && self.outline_cache_ver != self.app.buffer.version() {
            self.shell.dirty = true;
            need_full = true;
        }
        if self.tick_count % EXTERNAL_FILE_CHECK_TICKS == 0 {
            let before = (
                self.app.current_buffer_id(),
                self.app.buffer.version(),
                self.app.file_deleted,
                self.app.modified,
                self.app.filename.clone(),
                self.app.message.clone(),
                self.app.tabs.buffers.len(),
            );
            self.app.check_external_change();
            let after = (
                self.app.current_buffer_id(),
                self.app.buffer.version(),
                self.app.file_deleted,
                self.app.modified,
                self.app.filename.clone(),
                self.app.message.clone(),
                self.app.tabs.buffers.len(),
            );
            let missing_now: std::collections::HashSet<_> = self
                .app
                .tabs
                .buffers
                .iter()
                .filter(|tab| tab.file_mtime.is_some())
                .filter_map(|tab| {
                    tab.filename
                        .as_ref()
                        .filter(|path| std::fs::metadata(path).is_err())
                        .map(|_| tab.id)
                })
                .collect();
            if before != after || missing_now != self.missing_tab_ids {
                self.missing_tab_ids = missing_now;
                self.shell.dirty = true;
                need_full = true;
            }
        }
        // Every pane shell flows, not just the focused one — a build running in
        // one terminal pane must keep going while you type in another.
        for (tid, t) in self.app.pane_terminals.iter_mut() {
            t.poll();
            if t.take_damage() {
                self.shell.dirty = true;
                *self.pane_term_gens.entry(*tid).or_insert(0) += 1;
            }
        }
        if self.app.terminal.open || matches!(self.app.mode, Mode::Terminal) {
            // Face never paints TUI rows that call start() — boot PTY here if deferred.
            if !self.app.terminal.started {
                self.ensure_terminal_started();
            }
            self.app.terminal.poll();
            // Only repaint when the PTY actually changed the screen —
            // unconditional dirty here advanced frame_gen every 50ms, and the
            // face's resulting 20Hz SwiftUI republish made editor scrolling
            // visibly stutter whenever a terminal was open.
            if self.app.terminal.take_damage() {
                self.shell.dirty = true;
            }
        }
        // Parked shells keep flowing (background jobs, long builds).
        for t in &mut self.parked_terminals {
            if t.started {
                t.poll();
            }
        }
        if self.app.poll_git_refresh() {
            self.shell.dirty = true;
            need_full = true;
        }
        if self.poll_outline() {
            self.shell.dirty = true;
            need_full = true;
        }
        // Only recompose when poll actually changes state — not every 50ms while open.
        if self.app.git_wb.open && self.app.git_wb.poll_loading() {
            self.shell.dirty = true;
            need_full = true;
        }
        // Language services. Nothing about the LSP works without this: the
        // drain inside is what parses the `initialize` reply and sends
        // `initialized` + `didOpen`, so skipping it leaves the server spawned
        // and permanently idle. Post-edit didChange goes first (throttled, so a
        // typing run coalesces into one full-text notification), then the drain,
        // so a request issued this frame answers against the current document.
        if self.tick_count % LSP_SYNC_TICKS == 0 {
            self.app.sync_lsp_document();
        }
        let lang = self.app.poll_language_services();
        if lang.any() {
            self.shell.dirty = true;
            need_full |= lang.chrome;
        }
        // Correct a dirty flag that latched when it should not have. Cheap by
        // construction — see `App::recheck_modified`.
        if self.tick_count % DIRTY_RECHECK_TICKS == 0 && self.app.recheck_modified() {
            self.shell.dirty = true;
            need_full = true;
        }
        // Tell the daemon what we are doing. Nothing else can: the daemon owns
        // no language server, so without this push the menu-bar agent draws
        // "none" for every field forever. Never blocks the tick.
        if self.tick_count % DAEMON_REPORT_TICKS == 0 && self.reporter.is_some() {
            let status = self.daemon_status();
            if let Some(r) = self.reporter.as_mut() {
                r.offer(status);
            }
        }
        // Settings account login / profile fetch. Does not dirty chrome —
        // the face probes `github_account.generation` on its own object.
        let _ = self.github_account.poll();
        if let Some(msg) = self.app.update.poll() {
            self.app.message = msg;
            self.update_generation = self.update_generation.wrapping_add(1);
        }
        // A background parse landed — paint the fresh tokens (paint-only).
        if self.adopt_syntax_frames().is_some() {
            self.shell.dirty = true;
        }
        // completions / palette need live paint while open
        if self.shell.dirty || self.app.completions.active || self.app.palette.open {
            if need_full || self.app.scm.visible() {
                self.recompose();
            } else {
                // PTY / which-key / completions: paint only (no outline/SCM rebuild).
                self.recompose_scroll();
            }
        }
        // Shadow WAL: flush dirty buffer to journal if policy is satisfied.
        {
            // Borrow the path rather than allocating a String every tick, and
            // hand the journal a *closure* for the text: it only wants the
            // document on an actual flush (250 ms / 4 KiB, dirty only), and
            // building it eagerly taxed every tick of every session.
            let file_path = self
                .app
                .filename
                .as_ref()
                .map(|p| p.to_string_lossy())
                .unwrap_or(std::borrow::Cow::Borrowed(""));
            let dirty = self.app.modified;
            let version = self.app.buffer.version();
            let cursor = self.app.buffer.cursor();
            let scroll = self.app.scroll;
            self.journal.on_tick(
                &file_path,
                || self.app.buffer.text(),
                version,
                cursor.row as u32,
                cursor.col as u32,
                scroll as u32,
                dirty,
            );
        }
        self.frame_gen
    }

    /// `css_w/h` = editor stage size (not whole window).  
    /// `line_h` = painted line height; `cell_w` = monospaced cell width.
    ///
    /// A6: the ONE production write of viewport geometry. The core stores
    /// pixels (`app.stage`) and derives the cell grid; `shell.viewport`
    /// keeps the face's last report for the `sync_viewport_public` test
    /// seam. Nothing else writes geometry, so nothing re-syncs.
    pub fn resize(&mut self, css_w: f32, css_h: f32, line_h: f32, cell_w: f32, dpr: f32) {
        let scroll_before = self.app.scroll;
        self.shell.viewport.css_w = css_w.max(80.0);
        self.shell.viewport.css_h = css_h.max(80.0);
        self.shell.viewport.cell_px = line_h.max(12.0);
        self.shell.viewport.cell_w = cell_w.max(6.0);
        self.shell.viewport.dpr = dpr.max(1.0);
        self.app.resize_stage(css_w, css_h, line_h, cell_w, dpr);
        let total = self.app.buffer.line_count();
        let vis = self.app.grid_rows().max(1) as usize;
        let max_scroll = total.saturating_sub(vis.min(total));
        // GUI contract: panel/window resize NEVER re-anchors the viewport to the
        // caret — the user's scroll position is sacred (re-anchoring made hiding
        // the outline yank a far-scrolled view back to the caret line).
        self.app.scroll = scroll_before.min(max_scroll);
        self.shell.dirty = true;
        // Resize is viewport-only — keep outline/SCM caches (face debounces full shell).
        self.recompose_scroll();
    }

    /// Tests only: re-apply the face's last resize report after swapping
    /// `app` wholesale (production resizes through [`Engine::resize`]).
    #[cfg(test)]
    pub(crate) fn sync_viewport_public(&mut self) {
        self.sync_viewport_to_app();
    }

    pub(crate) fn update_scroll_public(&mut self) {
        self.app.update_scroll();
    }

    /// Re-apply the face's last resize report to the core. Test seam —
    /// production goes through `resize`, the only writer. Pixels in,
    /// derived cells out (A6).
    #[cfg(test)]
    fn sync_viewport_to_app(&mut self) {
        let v = self.shell.viewport;
        self.app
            .resize_stage(v.css_w, v.css_h, v.cell_px, v.cell_w, v.dpr);
    }

    /// Highlight window: what the viewport shows plus generous overscan, so a
    /// scroll usually stays a cache hit. Tokens are only ever consumed per row
    /// (`tokens_for_row`), so nothing needs whole-file highlighting.
    ///
    /// A1-6: the parse itself runs on the syntax worker — this does no
    /// tree-sitter work at all. Adopt finished frames, drop colours that
    /// belong to another document, and request a snapshot when the buffer or
    /// the window moved. While a parse is in flight the stale tokens keep
    /// painting: a column may shift for a frame or two, the same contract
    /// every async highlighter ships.
    fn refresh_syntax(&mut self) {
        self.adopt_syntax_frames();

        let path = self
            .app
            .filename
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if self.app.syntax.applied_path() != path {
            // Document switch: the old file's colours belong to other text.
            self.app.syntax.clear_tokens();
            self.syntax_applied = 0;
            self.syntax_requested = None;
        }

        const OVERSCAN: usize = 400;
        let first = self.app.scroll;
        let height = self.app.grid_rows() as usize;
        let window = first.saturating_sub(OVERSCAN)..(first + height + OVERSCAN);

        let version = self.app.buffer.version();
        let needs = self.syntax_applied != version || !self.app.syntax.covers_rows(&window);
        let pending = self
            .syntax_requested
            .as_ref()
            .map(|(v, p, w)| *v == version && *p == path && *w == window)
            .unwrap_or(false);
        if needs && !pending {
            let text = self.app.buffer.text();
            let ext = self.app.file_extension();
            // A full channel means the worker already holds a request; the
            // next recompose retries once it drains. `try_send` never blocks.
            if self
                .syntax_worker
                .request(suisei_core::syntax_worker::SyntaxRequest::Parse {
                    path: path.clone(),
                    ext,
                    text,
                    version,
                    window: window.clone(),
                })
            {
                self.syntax_requested = Some((version, path, window));
            }
        }
    }

    /// Drain finished parses and apply the ones that still match the live
    /// document. Returns the newest adopted version so callers (the tick)
    /// can mark the frame dirty and paint the fresh tokens.
    fn adopt_syntax_frames(&mut self) -> Option<u64> {
        let mut adopted = None;
        while let Ok(frame) = self.syntax_worker.frames().try_recv() {
            adopted = self.adopt_syntax_frame(frame).or(adopted);
        }
        adopted
    }

    fn adopt_syntax_frame(
        &mut self,
        frame: suisei_core::syntax_worker::SyntaxFrame,
    ) -> Option<u64> {
        use suisei_core::syntax_worker::SyntaxFrame;
        match frame {
            SyntaxFrame::Cached { count } => {
                self.syntax_cached = count;
                None
            }
            SyntaxFrame::Tokens {
                path,
                version,
                window,
                tokens,
                active,
                tree,
                text,
                ext,
                globals,
            } => {
                let current = self
                    .app
                    .filename
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                if path == current && version == self.app.buffer.version() {
                    let changed = self
                        .app
                        .syntax
                        .apply_frame(path, window, tokens, active, tree, text, ext);
                    // The worker collected the file's global scope beside the
                    // parse. Adopting it here is the whole point: completion
                    // now looks the globals up instead of walking for them,
                    // and the walk was 8.7 ms at 50k lines.
                    self.app
                        .scope_cache
                        .adopt(globals, self.app.syntax.live_tree_gen());
                    self.syntax_applied = version;
                    // Mark the frame dirty only when the tokens actually
                    // changed — an empty answer for an untitled document
                    // paints nothing and must not bump an idle tick.
                    changed.then_some(version)
                } else {
                    // Stale — the buffer moved on or the file changed; a
                    // fresher snapshot is already requested.
                    None
                }
            }
        }
    }

    /// Buffer version the painted tokens describe.
    ///
    /// Exposed so a test can assert that a keystroke returns with colours for
    /// the text it just produced, rather than the previous frame's.
    pub fn syntax_version_applied(&self) -> u64 {
        self.syntax_applied
    }

    /// Block until the worker's parse of the live document lands.
    ///
    /// Tests only — the app never waits; it paints stale and moves on. Public
    /// rather than `#[cfg(test)]` so integration tests can drive the real
    /// worker path; that path is exactly where scope-aware completion was
    /// broken while every in-crate test passed.
    pub fn flush_syntax(&mut self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            self.refresh_syntax();
            if self.syntax_applied == self.app.buffer.version() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "syntax worker did not catch up to the live document"
            );
            if let Ok(frame) = self
                .syntax_worker
                .frames()
                .recv_timeout(std::time::Duration::from_millis(20))
            {
                self.adopt_syntax_frame(frame);
            }
        }
    }

    pub(crate) fn recompose(&mut self) {
        // Same as TUI: re-parse syntax only when buffer text version changes.
        self.refresh_syntax();
        // Lazy SCM graph + git workbench tab data (may shell out once).
        if self.app.scm.visible() {
            self.app.scm.ensure_graph();
        }
        if self.app.git_wb.open {
            let _ = self.app.git_wb.poll_loading();
            self.app.git_wb.ensure_tab_data();
        }
        let git_wb_signature = self.app.git_wb.native_snapshot_signature();
        if git_wb_signature != self.git_wb_signature {
            self.git_wb_signature = git_wb_signature;
            self.git_wb_generation = self.git_wb_generation.wrapping_add(1);
        }
        // Outline: a full-buffer scan, and it ran on every version bump — so
        // every keystroke walked all 5000 lines of a 5000-line file. It is a
        // PANEL: it has never needed to be correct within one keystroke of the
        // edit that changed it.
        //
        // Marked dirty here and rebuilt on a quiet moment (below, and in the
        // tick), so a typing burst pays for it once instead of once per key.
        // A document switch is immediate: that outline is about another file.
        let ver = self.app.buffer.version();
        let path = self.app.filename.clone();
        if self.outline_cache_path != path {
            self.outline_cache_path = path;
            self.outline_dirty = true;
            self.rebuild_outline(ver);
        } else if self.outline_cache_ver != ver {
            self.outline_dirty = true;
            self.rebuild_outline_if_settled(ver);
        }
        self.frame_gen = self.frame_gen.saturating_add(1);
        self.last_diff = compose(&self.app, self.frame_gen, &self.outline_cache);
        self.shell.dirty = false;
    }

    /// How long a typing burst has to pause before the outline is rebuilt.
    /// Below what anyone notices in a panel; far above one keystroke.
    const OUTLINE_SETTLE: std::time::Duration = std::time::Duration::from_millis(140);

    fn rebuild_outline(&mut self, ver: u64) {
        self.outline_cache = crate::compositor::build_outline_public(&self.app);
        self.outline_cache_ver = ver;
        self.outline_dirty = false;
        self.outline_built_at = Some(std::time::Instant::now());
    }

    fn rebuild_outline_if_settled(&mut self, ver: u64) {
        let quiet = self
            .outline_built_at
            .map(|t| t.elapsed() >= Self::OUTLINE_SETTLE)
            .unwrap_or(true);
        if quiet {
            self.rebuild_outline(ver);
        }
    }

    /// Catch up an outline left dirty by a burst that has since stopped.
    /// Without this the panel would hold the pre-burst scan until the next
    /// edit happened to arrive after the settle window.
    fn poll_outline(&mut self) -> bool {
        if !self.outline_dirty {
            return false;
        }
        let quiet = self
            .outline_built_at
            .map(|t| t.elapsed() >= Self::OUTLINE_SETTLE)
            .unwrap_or(true);
        if !quiet {
            return false;
        }
        self.rebuild_outline(self.app.buffer.version());
        true
    }

    pub fn running(&self) -> bool {
        self.app.running
    }

    pub fn scroll_by(&mut self, delta_lines: i32) {
        // Don't steal scroll while selecting
        if self.pointer_down && self.pointer_moved {
            return;
        }
        if delta_lines == 0 {
            return;
        }
        self.app.scroll_by_lines(delta_lines);
        // Keep split pane mirrors in sync so inactive panes paint correctly.
        // Scroll never moves the caret — only the window.
        self.recompose_scroll();
    }

    /// Size + spawn PTY for side/full terminal (Suisei has no TUI first-draw hook).
    pub(crate) fn ensure_terminal_started(&mut self) {
        if !self.app.terminal.open || self.app.terminal.started {
            return;
        }
        // Spawn at the size the PANEL will actually be. This used to size the
        // PTY from the editor viewport and then let the face's own measurement
        // shrink it a moment later — and shrinking a grid pushes its top rows
        // into scrollback, so the shell's greeting had already scrolled away
        // before the first paint. The editor viewport is only the fallback for
        // the case where the face has not measured yet.
        let (cols, rows) = match self.face_terminal_grid {
            Some(grid) => grid,
            None => {
                let cols = self.app.grid_cols().max(40);
                let rows = if !self.app.pane_terminals.is_empty() {
                    self.app.grid_rows().max(24)
                } else {
                    self.app.grid_rows().max(8).min(24).max(8)
                };
                (cols, rows)
            }
        };
        self.app.terminal.resize(cols, rows);
        // `Terminal::start` consumes an anchor and starts in its parent.
        // Every shell uses the same explicit project-root policy.
        let anchor_owned = self
            .app
            .terminal_working_directory()
            .join(".suisei-terminal");
        self.app.terminal.start(Some(&anchor_owned));
        if !self.app.terminal.started {
            self.app.message = "Terminal: failed to spawn shell (PTY)".into();
        }
        self.shell.dirty = true;
    }

    /// Horizontal pan when wrap_lines is off (trackpad / zh·zl).
    pub fn scroll_h_by(&mut self, delta_cols: i32) {
        if self.app.wrap_lines || delta_cols == 0 {
            return;
        }
        if self.app.preview.open {
            if delta_cols < 0 {
                self.app.preview.hscroll = self
                    .app
                    .preview
                    .hscroll
                    .saturating_sub((-delta_cols) as usize);
            } else {
                self.app.preview.hscroll =
                    self.app.preview.hscroll.saturating_add(delta_cols as usize);
            }
            self.recompose_scroll();
            return;
        }
        // Clamped by `set_hscroll`: without a right-hand limit a trackpad pan
        // ran off past the end of the text into empty space, forever.
        let next = if delta_cols < 0 {
            self.app.hscroll.saturating_sub((-delta_cols) as usize)
        } else {
            self.app.hscroll.saturating_add(delta_cols as usize)
        };
        self.app.set_hscroll(next);
        self.recompose_scroll();
    }

    /// Position-only scroll sync (no recompose): keeps Core's `scroll` tracking
    /// the native clip during covered scrolling so the next caret op / publish
    /// never snaps the viewport. The paint band already covers ±overscan.
    pub fn scroll_sync(&mut self, line: u32, hscroll_cols: u32) {
        if self.pointer_down && self.pointer_moved {
            return;
        }
        self.app.scroll_to_line(line as usize);
        if !self.app.wrap_lines {
            self.app.set_hscroll(hscroll_cols as usize);
        } else {
            self.app.hscroll = 0;
        }
    }

    /// Absolute scroll position for native NSScrollView faces.
    /// `line` = first fully/partially visible buffer row; `hscroll_cols` when wrap off.
    pub fn scroll_to(&mut self, line: u32, hscroll_cols: u32) {
        if self.pointer_down && self.pointer_moved {
            return;
        }
        let before = (self.app.scroll, self.app.hscroll);
        self.app.scroll_to_line(line as usize);
        if !self.app.wrap_lines {
            self.app.set_hscroll(hscroll_cols as usize);
        } else {
            self.app.hscroll = 0;
        }
        let after = (self.app.scroll, self.app.hscroll);
        if before == after {
            return;
        }
        self.recompose_scroll();
    }

    /// Fractional line scroll for trackpads / smooth faces.
    pub fn scroll_by_frac(&mut self, delta_lines: f32) {
        if self.pointer_down && self.pointer_moved {
            return;
        }
        if delta_lines == 0.0 || !delta_lines.is_finite() {
            return;
        }
        // Pretty preview owns its own vertical scroll while open.
        if self.app.preview.open {
            if delta_lines > 0.0 {
                let step = delta_lines.ceil() as isize;
                self.app.preview.scroll_by(step, step.max(1) as usize);
            } else {
                let step = (-delta_lines).ceil() as isize;
                self.app.preview.scroll_by(-step, step.max(1) as usize);
            }
            self.recompose_scroll();
            return;
        }
        let before = (self.app.scroll, self.app.scroll_frac.to_bits());
        self.app.scroll_by_frac(delta_lines);
        let after = (self.app.scroll, self.app.scroll_frac.to_bits());
        if before == after {
            return;
        }
        self.recompose_scroll();
    }

    /// Scroll / paint-only recompose: editor surfaces only (no explorer/SCM rebuild).
    /// Used for wheel, light keys (face), and pointer when only the viewport moves.
    pub(crate) fn recompose_scroll(&mut self) {
        // Syntax only if buffer changed (usually no-op while scrolling).
        self.refresh_syntax();
        self.frame_gen = self.frame_gen.saturating_add(1);
        // Patch in place when we already have chrome — full compose is too heavy
        // for 5k–10k line files during trackpad momentum.
        if let Some(chrome) = self.last_diff.chrome.as_mut() {
            crate::compositor::patch_chrome_editor_scroll(&self.app, self.frame_gen, chrome);
            self.last_diff.frame_gen = self.frame_gen;
        } else {
            self.last_diff =
                crate::compositor::compose(&self.app, self.frame_gen, &self.outline_cache);
        }
        self.shell.dirty = false;
    }

    /// Alias for face light path naming.
    pub fn recompose_paint_only(&mut self) {
        self.recompose_scroll();
    }

    /// Mouse **down**: place caret + arm drag. Does **not** enter Visual yet.
    pub fn click_at(&mut self, buffer_row: u32, visual_col: u32, select_word: bool) {
        if self.app.buffer.line_count() == 0 {
            return;
        }
        let pos = self.pos_from_click(buffer_row, visual_col);

        if matches!(self.app.mode, Mode::Palette | Mode::Explorer) {
            self.app.palette.close();
            self.app.mode = Mode::Editor;
        }
        if select_word {
            // Double-click: select the word into the GUI model.
            self.app.select_word_gui(pos);
            self.app.mouse.dragging = false;
            self.app.mouse.drag_anchor = None;
            self.pointer_down = false;
            self.pointer_moved = false;
        } else {
            // Down: place a caret (collapses any selection). The anchor for a
            // subsequent drag is this cell.
            self.app.caret_place(pos);
            self.app.mouse.dragging = true;
            self.app.mouse.drag_anchor = Some(pos);
            self.pointer_down = true;
            self.pointer_moved = false;
        }

        self.app.hover_text = None;
        self.app.update_scroll();
        // Pointer motion is paint-hot; skip SCM/git shell rebuilds.
        self.recompose_scroll();
    }

    /// Mouse **move** while down: once off the down cell, enter Visual and extend.
    pub fn drag_to(&mut self, buffer_row: u32, visual_col: u32) {
        if !self.pointer_down {
            // Face skipped down — treat as down then move.
            self.click_at(buffer_row, visual_col, false);
            return;
        }

        let pos = self.pos_from_click(buffer_row, visual_col);
        let anchor = self.app.mouse.drag_anchor.unwrap_or(pos);

        if pos.row != anchor.row || pos.col != anchor.col {
            self.pointer_moved = true;
        }

        if !self.pointer_moved {
            // Still on the down cell — a caret, no selection yet.
            self.app.caret_place(pos);
            self.recompose_scroll();
            return;
        }

        // Moved: extend the GUI selection from the down cell (its anchor) to the
        // pointer. `caret_drag_to` keeps the anchor `caret_place` set on down.
        let _ = anchor;
        self.app.completions.deactivate();
        self.app.caret_drag_to(pos);
        self.app.mouse.dragging = true;
        self.app.update_scroll();
        self.recompose_scroll();
    }

    /// Mouse **up**: end pointer lifecycle; keep Visual if we moved.
    pub fn mouse_up(&mut self) {
        self.app.mouse.dragging = false;
        // Keep drag_anchor only while selecting? xei clears it; selection uses visual_anchor.
        self.app.mouse.drag_anchor = None;
        self.pointer_down = false;
        // pointer_moved left as-is for tests; clear for next gesture
        let _ = self.pointer_moved;
        self.pointer_moved = false;
        self.recompose_scroll();
    }

    pub fn save_file(&mut self) {
        self.app.save_file();
        // File is now durable — remove the shadow WAL entry.
        if let Some(ref path) = self.app.filename {
            self.journal.on_saved(&path.to_string_lossy());
        }
        self.recompose();
    }

    // ── GUI-editor commands (face menu / standard Mac chords) ──

    pub fn undo(&mut self) {
        if !self.text_editor_owns_keys() {
            return;
        }
        self.app.undo();
        self.app.update_scroll();
        self.recompose();
    }

    pub fn redo(&mut self) {
        if !self.text_editor_owns_keys() {
            return;
        }
        self.app.redo();
        self.app.update_scroll();
        self.recompose();
    }

    pub fn select_all(&mut self) {
        if !self.text_editor_owns_keys() {
            return;
        }
        self.app.select_all();
        self.recompose_scroll();
    }

    /// Open the incremental find bar (core Search mode drives the scene).
    pub fn find_open(&mut self) {
        self.app.enter_search();
        self.recompose();
    }

    /// Step matches while (or after) find: true = next, false = previous.
    pub fn find_step(&mut self, forward: bool) {
        if matches!(self.app.mode, Mode::Search) {
            self.app.search_cycle(forward);
        } else if forward {
            self.app.search_next();
        } else {
            self.app.search_prev();
        }
        self.app.update_scroll();
        // The match index is chrome state, not only editor scroll. A light
        // scroll patch leaves the Find counter stale even when the caret moved.
        self.recompose();
    }

    pub fn find_set_input(&mut self, input: &str) {
        self.app.set_search_input(input.to_string());
        self.recompose();
    }

    /// Accept the native find field before AppKit gives focus back to the
    /// editor. Keeping this semantic avoids a raw Return being reinterpreted
    /// as a document newline if first-responder changes during dismissal.
    pub fn find_accept(&mut self) {
        if !matches!(self.app.mode, Mode::Search) {
            return;
        }
        self.app.commit_search();
        self.app.update_scroll();
        self.recompose();
    }

    /// Dismiss the native find field and restore its opening cursor/scroll.
    pub fn find_cancel(&mut self) {
        if !matches!(self.app.mode, Mode::Search) {
            return;
        }
        self.app.cancel_search();
        self.recompose();
    }

    pub fn palette_set_query(&mut self, query: &str) {
        if !self.app.palette.open {
            return;
        }
        self.app.palette.set_query(query.to_string());
        self.recompose();
    }

    /// Insert text at the caret (file drop / IME commit). Routes to the PTY
    /// when the terminal owns input, like the TUI bracketed-paste path.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // The docked shell owns the paste only while it has the keyboard…
        if matches!(self.app.mode, Mode::Terminal) && self.app.terminal.open {
            self.app.terminal.paste_input(text);
            self.shell.dirty = true;
            self.recompose_scroll();
            return;
        }
        // …otherwise a focused pane shell takes it. This used to fall through
        // to the editor (or the dock) — IME commits and drops landed in the
        // terminal tab's hidden buffer while the shell had the keyboard.
        if self.app.terminal_window_focused() {
            if let Some(t) = self.app.focused_pane_terminal_mut() {
                t.paste_input(text);
                self.shell.dirty = true;
                self.recompose_scroll();
                return;
            }
        }
        if self.text_editor_owns_keys() {
            // IME commits and programmatic paste must use the same exclusive
            // Selection model as clicks, drags and ordinary typing. The old
            // cursor-only paste path inserted at `buffer.cursor`, then tried to
            // repair `sel` afterwards; mid-line Korean input could therefore
            // land at a stale position or fail to replace the active range.
            self.app.gui_insert_text(&text.replace('\r', ""));
            self.app.update_scroll();
            self.recompose_scroll();
            return;
        }
        // A viewer has no editable backing document. In particular, an IME
        // commit and AppKit Paste both arrive here rather than `dispatch_key`;
        // letting either fall through used to dirty an audio/image/PDF tab's
        // intentionally empty buffer.
        if matches!(self.app.mode, Mode::Editor) && self.app.live_tab_kind().is_viewer() {
            return;
        }
        self.app.paste_text_at_cursor(text);
        self.recompose();
    }

    /// Raw keyboard text into the FOCUSED terminal's PTY as UTF-8 bytes — the
    /// path the terminal input view uses for IME-committed Hangul/CJK and for
    /// ordinary typed characters. Unlike `paste_text` this is NOT wrapped in a
    /// bracketed-paste envelope: it is keystrokes, not a paste, so the shell and
    /// its TUIs must see it as typed input. No-op when no terminal has focus.
    pub fn terminal_input(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if matches!(self.app.mode, Mode::Terminal) && self.app.terminal.open {
            self.app.terminal.write_input(text.as_bytes());
            self.shell.dirty = true;
            self.recompose_scroll();
            return;
        }
        if self.app.terminal_window_focused() {
            if let Some(t) = self.app.focused_pane_terminal_mut() {
                t.write_input(text.as_bytes());
                self.shell.dirty = true;
                self.recompose_scroll();
            }
        }
    }

    /// GUI focus contract: clicking the terminal panel routes keys to the PTY,
    /// clicking the editor routes them back to the buffer.
    // ---- layout tabs (J7) -------------------------------------------------

    /// Fold the current arrangement into a layout tab.
    pub fn fold_layout(&mut self) -> bool {
        let ok = self.app.fold_layout();
        if ok {
            self.recompose();
        }
        ok
    }

    /// Unfold the active layout.
    pub fn unfold_layout(&mut self) -> bool {
        let ok = self.app.unfold_layout();
        if ok {
            self.recompose();
        }
        ok
    }

    pub fn activate_layout(&mut self, id: u64, focus_doc: u64) -> bool {
        // 0 is the never-issued buffer id — "no preference", keep the tree's
        // own focus order. A grouped chip click passes its document so the
        // arrangement comes back with that pane in front.
        let doc = (focus_doc != 0).then(|| suisei_core::BufferId(focus_doc));
        let ok = self.app.activate_layout(id, doc);
        if ok {
            // NO `update_scroll()` — the same rule `goto_tab_id` states.
            //
            // `App::activate_layout` restores the parked tree AND its panes,
            // and a pane carries its own viewport, so the arrangement comes
            // back looking at what it was looking at. `update_scroll` then
            // zeroed `scroll_frac` and recomputed the offset from the caret
            // row, which for a pane scrolled well down its document means the
            // viewport snaps to the top: measured, a pane parked at line 180
            // came back at line 0.
            //
            // Both positions reach the face — the panes are installed at the
            // restored offset and immediately moved to the derived one. That
            // is the "switching to a layout tab makes the editor shake".
            self.recompose();
        }
        ok
    }

    pub fn toggle_layout_style(&mut self, id: u64) -> bool {
        let ok = self.app.toggle_layout_style(id);
        if ok {
            self.recompose();
        }
        ok
    }

    /// Layout that currently owns the desk, or 0 when the desk is free.
    pub fn active_layout_id(&self) -> u64 {
        self.app.active_layout.unwrap_or(0)
    }

    /// Toggle the **docked** terminal (⌃T), directly.
    ///
    /// The face had no way to ask for this: its button synthesised a ⌃T
    /// keystroke instead. That works only for as long as nothing else claims
    /// the key — and a focused terminal pane does, quite correctly, since ⌃T
    /// is a readline binding. So the button silently stopped working. A
    /// control has to call the thing it names.
    pub fn toggle_terminal_dock(&mut self) {
        self.app.toggle_terminal_side();
        if self.app.terminal.open {
            self.ensure_terminal_started();
        }
        self.shell.dirty = true;
        self.recompose();
    }

    /// Pretty document preview, for a control that names it.
    ///
    /// Same defect as the dock button above, one step worse. The menu item
    /// simulated ⇧⌘V, and that chord is "pretty preview" only while the editor
    /// holds focus — a focused terminal pane claims it, quite correctly, as
    /// "paste the clipboard into the shell". So a menu item labelled Pretty
    /// Preview could paste into a running process.
    pub fn toggle_preview(&mut self) {
        self.app.toggle_preview();
        self.shell.dirty = true;
        self.recompose();
    }

    /// Open a terminal TAB, or close it when one is already focused.
    ///
    /// `App::toggle_terminal_full` spawns and starts its own shell, so there is
    /// no `ensure_terminal_started` here — that call belongs to the docked
    /// terminal, which shares one session.
    pub fn toggle_terminal_tab(&mut self) {
        self.app.toggle_terminal_full();
        self.shell.dirty = true;
        self.recompose();
    }

    pub fn focus_terminal(&mut self, on: bool) {
        if on {
            if self.app.terminal.open {
                self.ensure_terminal_started();
                self.app.mode = Mode::Terminal;
            }
        } else if matches!(self.app.mode, Mode::Terminal) {
            self.app.mode = Mode::Editor;
        }
        self.recompose();
    }

    /// Size the PTY to the face's terminal panel (cols × rows in cells).
    pub fn terminal_resize(&mut self, cols: u32, rows: u32) {
        if cols < 10 || rows < 3 {
            return;
        }
        let cols = cols.min(500) as u16;
        let rows = rows.min(200) as u16;
        // Remembered even when the panel is not open yet, so the PTY can be
        // spawned at the right size instead of being resized into scrollback
        // right after it prints its greeting.
        self.face_terminal_grid = Some((cols, rows));
        if !self.app.terminal.open {
            return;
        }
        self.app.terminal.resize(cols, rows);
        self.shell.dirty = true;
        self.recompose_scroll();
    }

    /// Scroll the terminal panel through its scrollback. Positive reveals
    /// older output. The GUI had no path to this at all — core kept a 5,000-row
    /// scrollback and a `scroll_offset`, and nothing on this side of the ABI
    /// ever moved or read either.
    pub fn terminal_scroll(&mut self, delta_rows: i32) {
        if !self.app.terminal.open || delta_rows == 0 {
            return;
        }
        if delta_rows > 0 {
            self.app.terminal.scroll_up(delta_rows as usize);
        } else {
            self.app.terminal.scroll_down((-delta_rows) as usize);
        }
        self.shell.dirty = true;
        self.recompose();
    }

    /// Size a PANE terminal's PTY to the face's measured grid (cells). Pane
    /// shells used to get no resize after spawn — they started at a viewport
    /// guess, so output wrapped at the wrong column forever, divider drags and
    /// window resizes never reflowed them, and vim/htop drew garbled.
    pub fn terminal_resize_pane(&mut self, pane: u32, cols: u32, rows: u32) {
        if cols < 10 || rows < 3 {
            return;
        }
        let cols = cols.min(500) as u16;
        let rows = rows.min(200) as u16;
        let Some(t) = self.app.pane_terminal_mut(pane as usize) else {
            return;
        };
        t.resize(cols, rows);
        self.shell.dirty = true;
        self.recompose_scroll();
    }

    /// Scroll a pane terminal through its scrollback — the pane twin of
    /// `terminal_scroll`. Positive reveals older output.
    pub fn terminal_scroll_pane(&mut self, pane: u32, delta_rows: i32) {
        if delta_rows == 0 {
            return;
        }
        let Some(t) = self.app.pane_terminal_mut(pane as usize) else {
            return;
        };
        if delta_rows > 0 {
            t.scroll_up(delta_rows as usize);
        } else {
            t.scroll_down((-delta_rows) as usize);
        }
        self.shell.dirty = true;
        self.recompose();
    }

    /// Wrapping u16 generation of a pane shell's content, for the face to skip
    /// re-pulling a grid it already has. 0 while the shell has produced
    /// nothing.
    pub fn pane_term_gen(&self, pane: usize) -> u16 {
        let Some(buf_id) = self.app.split.panes.get(pane).map(|p| p.buffer) else {
            return 0;
        };
        let Some(tid) = self
            .app
            .tabs
            .buffers
            .iter()
            .find(|t| t.id == buf_id)
            .and_then(|t| t.terminal)
        else {
            return 0;
        };
        self.pane_term_gens.get(&tid).copied().unwrap_or(0) as u16
    }

    pub fn save_as(&mut self, path: &str) {
        self.app.filename = Some(std::path::PathBuf::from(path));
        self.app.save_file();
        self.journal.on_saved(path);
        self.recompose();
    }

    /// Explorer row activate (Enter / double-click): Core `select_current` + open tab.
    /// Docked Project navigator keeps the tree after open (Xcode-like); Mode returns to Normal
    /// so the editor receives keys, but `explorer.entries` stay populated.
    pub fn explorer_activate(&mut self, index: u32) {
        if self.app.explorer.entries.is_empty() {
            return;
        }
        if (index as usize) < self.app.explorer.entries.len() {
            self.app.explorer.selected = index as usize;
        }
        if let Some(path) = self.app.explorer.select_current() {
            let path_str = path.display().to_string();
            self.app.open_new_tab(&path_str);
            // Keep tree data for docked navigator; leave Mode::Editor for editing.
            self.app.explorer.open = true;
            self.app.mode = Mode::Editor;
            self.app.message = format!("Opened {}", path_str);
        }
        // dir navigation refreshes entries in place
        self.recompose();
    }

    /// Ensure Project tree has entries without stealing keyboard focus into Mode::Explorer.
    pub fn ensure_project_tree(&mut self) {
        if self.app.explorer.entries.is_empty() {
            if let Some(ref f) = self.app.filename {
                if f.is_dir() {
                    self.app.explorer.cwd = f.clone();
                } else if let Some(parent) = f.parent() {
                    self.app.explorer.cwd = parent.to_path_buf();
                }
            }
            self.app.explorer.refresh();
        }
        // Mark open for face/TUI flags, but do not switch mode.
        self.app.explorer.open = true;
        self.recompose();
    }

    /// Docked SCM navigator: load status/graph without keeping Mode::SourceControl.
    pub fn ensure_scm_panel(&mut self) {
        let hint = self.app.filename.as_deref();
        if !self.app.scm.open {
            self.app.scm.open_and_refresh(hint);
        } else {
            self.app.scm.refresh_status(hint);
            self.app.scm.ensure_graph();
        }
        if matches!(self.app.mode, Mode::SourceControl) {
            self.app.mode = Mode::Editor;
        }
        self.recompose();
    }

    pub fn close_scm_panel(&mut self) {
        self.app.close_scm_immediate();
        self.recompose();
    }

    /// Jump caret to 1-based line (outline / jump bar) — centers the viewport
    /// (Xcode behavior) instead of pinning the target to the top edge.
    pub fn goto_line(&mut self, line_1based: u32) {
        self.app.goto_line(line_1based as usize);
        let vis = self.app.grid_rows().max(1) as usize;
        let target = (line_1based as usize).saturating_sub(1);
        self.app.scroll_to_line(target.saturating_sub(vis / 2));
        self.recompose();
    }

    /// Exact-range paint pull for the face renderer (see compositor::build_editor_band).
    pub fn editor_band(
        &self,
        pane: usize,
        start: usize,
        rows: usize,
    ) -> (Vec<crate::compositor::EditorLineScene>, u32) {
        crate::compositor::build_editor_band(&self.app, pane, start, rows)
    }

    /// Live split-divider drag from the face.
    ///
    /// Takes the two panes the divider sits between and how far it moved, as a
    /// fraction of the whole editor along that axis. This replaced
    /// `split_set_ratio(f32)`, which could only ever address one divider —
    /// with three panes there are two, and both were driven by that one
    /// number.
    pub fn split_resize(&mut self, a: u32, b: u32, delta: f32) {
        if !self.app.split.is_split() || !delta.is_finite() {
            return;
        }
        if self.app.split.resize_between(a as usize, b as usize, delta) {
            // The pane rects live in the chrome snapshot, but the scroll
            // patch path rebuilds the editor surfaces (rects included) far
            // more cheaply than a full recompose — a divider drag fires this
            // on every pointer move, so the full rebuild re-tokenized every
            // pane per pixel.
            self.recompose_scroll();
        }
    }

    /// Forward a face mouse event to a terminal's inner app when it
    /// requested tracking (vim/htop/tmux). `pane == 0xFFFF` targets the
    /// dock. Returns true when the shell consumed the event — the face
    /// should not also act on it (e.g. wheel → scrollback).
    pub fn terminal_mouse(
        &mut self,
        pane: u32,
        button: u8,
        x: u16,
        y: u16,
        pressed: bool,
        motion: bool,
    ) -> bool {
        let term = if pane == 0xFFFF {
            if !self.app.terminal.open {
                return false;
            }
            &mut self.app.terminal
        } else {
            match self.app.pane_terminal_mut(pane as usize) {
                Some(t) => t,
                None => return false,
            }
        };
        if !term.wants_mouse() {
            return false;
        }
        term.mouse_report(button, x, y, pressed, motion);
        true
    }

    /// Restore the previous session's files + cursors, if a session was
    /// saved. Landing named buffers flips the welcome rule, so Welcome
    /// yields to the restored editor.
    pub fn restore_session(&mut self) {
        self.app.restore_session();
        self.recompose();
    }

    /// Persist open files + cursors for the next launch (core writes
    /// `~/.suisei/session` atomically).
    pub fn save_session(&self) {
        self.app.save_session();
    }

    /// Toggle a breakpoint on a specific 1-based line of the current file
    /// (gutter click — bookmark affordance).
    /// Stage or discard the change on a line, and repaint.
    pub fn apply_gutter_hunk(
        &mut self,
        line_1based: u32,
        action: suisei_core::git::HunkAction,
    ) -> i32 {
        let rc = self.app.apply_gutter_hunk(line_1based, action);
        // The bar's fill, the text itself after a discard, and the message all
        // change here; none of them are on an input path that would repaint.
        self.shell.dirty = true;
        rc
    }

    pub fn toggle_breakpoint_line(&mut self, line_1based: u32) {
        if line_1based == 0 {
            return;
        }
        let Some(path) = self.app.filename.clone() else {
            return;
        };
        let path_str = path.to_string_lossy().to_string();
        let _ = self
            .app
            .dap
            .toggle_breakpoint(&path_str, (line_1based - 1) as usize);
        self.recompose_scroll();
    }

    /// Downsampled document overview for the minimap strip.
    /// Buckets ≤ `max_buckets`; each = (indent_cols, len_cols, flags) of the
    /// longest line in the bucket. flags: 1 = git-changed in bucket.
    pub fn minimap(&self, max_buckets: usize) -> (Vec<(u8, u8, u8)>, u32) {
        let total = self.app.buffer.line_count();
        if total == 0 || max_buckets == 0 {
            return (Vec::new(), 0);
        }
        let bucket = total.div_ceil(max_buckets).max(1);
        let mut out = Vec::with_capacity(total.div_ceil(bucket));
        let mut i = 0usize;
        while i < total {
            let end = (i + bucket).min(total);
            let mut best_len = 0usize;
            let mut best_indent = 0usize;
            let mut flags = 0u8;
            for row in i..end {
                let line = self.app.buffer.line(row);
                let trimmed = line.trim_start();
                let len = line.chars().count();
                if len > best_len {
                    best_len = len;
                    best_indent = line.len() - trimmed.len();
                }
                if flags == 0 && self.app.git.sign_at(row).is_some() {
                    flags = 1;
                }
            }
            out.push((best_indent.min(200) as u8, best_len.min(255) as u8, flags));
            i = end;
        }
        (out, total as u32)
    }

    /// Flattened breakpoint list for the navigator panel (path, 1-based line, flags).
    pub fn list_breakpoints(&self) -> Vec<BreakpointRow> {
        let mut rows = Vec::new();
        let mut keys: Vec<_> = self.app.dap.breakpoints.keys().cloned().collect();
        keys.sort();
        for path in keys {
            let Some(list) = self.app.dap.breakpoints.get(&path) else {
                continue;
            };
            let name = std::path::Path::new(&path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&path)
                .to_string();
            for bp in list {
                rows.push(BreakpointRow {
                    path: path.clone(),
                    name: name.clone(),
                    line_1based: (bp.line + 1) as u32,
                    verified: bp.verified,
                    condition: bp.condition.clone().unwrap_or_default(),
                    has_log: bp.log_message.as_ref().is_some_and(|s| !s.is_empty()),
                });
            }
        }
        rows
    }

    /// Open file (if needed) and jump to breakpoint line.
    pub fn goto_breakpoint(&mut self, path: &str, line_1based: u32) {
        if line_1based == 0 {
            return;
        }
        let need_open = self
            .app
            .filename
            .as_ref()
            .map(|p| p.to_string_lossy() != path)
            .unwrap_or(true);
        if need_open {
            let same_tab = self.app.tabs.buffers.iter().any(|t| {
                t.filename
                    .as_ref()
                    .is_some_and(|p| p.to_string_lossy() == path)
            });
            if same_tab {
                // Switch to existing tab if present.
                if let Some(idx) = self.app.tabs.buffers.iter().position(|t| {
                    t.filename
                        .as_ref()
                        .is_some_and(|p| p.to_string_lossy() == path)
                }) {
                    self.app.goto_tab(idx);
                } else {
                    self.app.open_new_tab(path);
                }
            } else {
                self.app.open_new_tab(path);
            }
        }
        self.app.goto_line(line_1based as usize);
        self.recompose();
    }

    pub fn remove_breakpoint(&mut self, path: &str, line_1based: u32) {
        if line_1based == 0 {
            return;
        }
        let line0 = (line_1based - 1) as usize;
        let has = self
            .app
            .dap
            .breakpoints
            .get(path)
            .is_some_and(|list| list.iter().any(|b| b.line == line0));
        if has {
            // toggle_breakpoint removes when already set.
            let _ = self.app.dap.toggle_breakpoint(path, line0);
            self.recompose();
        }
    }

    pub fn toggle_breakpoint_cursor(&mut self) {
        self.app.dap_toggle_breakpoint();
        self.recompose();
    }

    pub fn explorer_select(&mut self, index: u32) {
        if (index as usize) < self.app.explorer.entries.len() {
            self.app.explorer.selected = index as usize;
            self.recompose();
        }
    }

    pub fn goto_tab(&mut self, index: u32) {
        // In a split the focused pane follows the document the user picked —
        // and that is automatic now: `App` IS the focused pane's state, so
        // changing the active document changes what that pane shows. The old
        // `sync_focused_pane_tab()` here was writing the same fact twice.
        self.app.goto_tab(index as usize);
        // NO `update_scroll()` here. It re-derives the scroll from the CARET,
        // so a tab scrolled to line 7000 with the caret still at line 1 snapped
        // straight back to the top. The tab's saved scroll is authoritative.
        self.recompose();
    }

    pub fn close_tab(&mut self, index: u32) {
        let n = self.app.tabs.buffers.len();
        if n == 0 {
            return;
        }
        let idx = (index as usize).min(n.saturating_sub(1));
        // Closes that tab in place. This used to be
        // `goto_tab(idx); close_current_tab(); <put the editor back>`, where
        // the last step could only guess, because making the doomed tab active
        // had already destroyed what it was trying to restore.
        self.app.close_tab_at(idx);
        self.app.update_scroll();
        self.recompose();
    }

    pub fn open_blank_tab(&mut self) {
        self.app.open_blank_tab();
        // A blank tab has NO path, and must not be given one.
        //
        // It used to get the bare relative `"Untitled"`, which saves against
        // the process working directory — `/` for an `.app` launched from
        // Finder. Anchoring it at the project root instead was no better: a
        // session rooted at `/` produced `/Untitled.txt`, and `/` is a
        // read-only filesystem, so Save simply failed. A buffer that has never
        // been written has no location yet; the face asks for one on save.
        self.app.update_scroll();
        self.recompose();
    }

    // ---- Stable-id tab operations ---------------------------------------
    // Strip slots stop being buffer indices the moment a folded layout
    // gathers its members into a run (grouped) or hides them behind one chip
    // (unified): every slot after the group names a different document than
    // the same buffer index. The face therefore addresses chips by
    // `BufferTab::id` (and layout chips by their layout id), and these
    // entries translate at the boundary.

    pub fn goto_tab_id(&mut self, id: u64) {
        self.app.goto_tab_id(suisei_core::BufferId(id));
        // NO `update_scroll()` — same as `goto_tab`: the tab's saved scroll
        // is authoritative; the caret-derived one snaps long scrolls to top.
        self.recompose();
    }

    pub fn close_tab_id(&mut self, id: u64) {
        self.app.close_tab_id(suisei_core::BufferId(id));
        self.app.update_scroll();
        self.recompose();
    }

    pub fn move_tab_ids(&mut self, from: u64, to: u64) -> bool {
        let ok = self
            .app
            .move_tab_ids(suisei_core::BufferId(from), suisei_core::BufferId(to));
        if ok {
            self.recompose();
        }
        ok
    }

    /// "Close Tab" on a layout chip: the entry goes, its documents stay open
    /// as loose tabs. Names its target (unlike `unfold_layout`, which is
    /// bound to the active layout), so a chip can be closed while another
    /// arrangement owns the screen.
    pub fn drop_layout(&mut self, id: u64) -> bool {
        let ok = self.app.drop_layout(id);
        if ok {
            self.recompose();
        }
        ok
    }

    pub fn split_vertical(&mut self) {
        self.app.split_vertical();
        self.recompose();
    }

    pub fn split_horizontal(&mut self) {
        self.app.split_horizontal();
        self.recompose();
    }

    pub fn split_above(&mut self) {
        self.app.split_above();
        self.recompose();
    }

    pub fn split_left(&mut self) {
        self.app.split_left();
        self.recompose();
    }

    pub fn focus_next_pane(&mut self) {
        self.app.focus_other_pane();
        self.recompose();
    }

    pub fn focus_pane(&mut self, index: u32) {
        self.app.focus_pane(index as usize);
        self.recompose();
    }

    pub fn close_focused_pane(&mut self) {
        self.app.close_split();
        self.recompose();
    }

    pub fn palette_activate(&mut self, filtered_index: u32) {
        if !self.app.palette.open {
            return;
        }
        self.app.palette.selected = filtered_index as usize;
        self.app.execute_palette_selection();
        self.recompose();
    }

    pub fn palette_select(&mut self, filtered_index: u32) {
        if self.app.palette.open {
            self.app.palette.selected = filtered_index as usize;
            self.recompose();
        }
    }

    pub fn settings_select(&mut self, row: u32) {
        if !self.app.settings.visible() {
            return;
        }
        let n = self.app.settings.page_item_count().max(1);
        self.app.settings.selected = (row as usize).min(n.saturating_sub(1));
        // About page has synthetic rows — keep selection in bounds of composed list
        self.recompose();
    }

    pub fn settings_activate(&mut self, row: u32) {
        use suisei_core::settings::SettingsAction;
        if !self.app.settings.visible() {
            return;
        }
        self.settings_select(row);
        match self.app.settings.activate() {
            SettingsAction::ApplyTheme | SettingsAction::ApplyGpuAcc | SettingsAction::ApplyLsp => {
                self.app.apply_settings_draft();
            }
            SettingsAction::OpenWorkbench => {
                self.app.close_settings();
                self.app.toggle_git_workbench();
            }
            SettingsAction::OpenScm => {
                self.app.close_settings();
                self.app.toggle_scm();
            }
            SettingsAction::None => {}
        }
        // GUI face has no "s to save" muscle memory — persist draft after every change.
        if self.app.settings.dirty {
            self.app.save_settings();
        }
        self.recompose();
    }

    /// Set an option-bearing Settings row to an explicit value. Native menus
    /// and segmented controls must not emulate selection by repeatedly
    /// activating a cyclic TUI row.
    pub fn settings_set_value(&mut self, row: u32, value: u32) {
        use suisei_core::settings::SettingsAction;
        if !self.app.settings.visible() {
            return;
        }
        self.settings_select(row);
        match self.app.settings.set_value(value) {
            SettingsAction::ApplyTheme | SettingsAction::ApplyGpuAcc | SettingsAction::ApplyLsp => {
                self.app.apply_settings_draft()
            }
            SettingsAction::OpenWorkbench => {
                self.app.close_settings();
                self.app.toggle_git_workbench();
            }
            SettingsAction::OpenScm => {
                self.app.close_settings();
                self.app.toggle_scm();
            }
            SettingsAction::None => {}
        }
        if self.app.settings.dirty {
            self.app.save_settings();
        }
        self.recompose();
    }

    /// Apply the arbitrary colour carried by the native macOS color well.
    pub fn settings_set_highlight_color(&mut self, value: &str) {
        use suisei_core::settings::SettingsAction;
        if !self.app.settings.visible() {
            return;
        }
        if self.app.settings.set_highlight_color(value) == SettingsAction::ApplyTheme {
            self.app.apply_settings_draft();
        }
        if self.app.settings.dirty {
            self.app.save_settings();
        }
        self.recompose();
    }

    /// Explicit save (also used when Settings window closes).
    pub fn settings_save(&mut self) {
        // Always persist draft when panel open (covers dirty races + face Save).
        if self.app.settings.visible() || self.app.settings.dirty {
            self.app.save_settings();
            self.recompose();
        }
    }

    /// `key` is xei toolbar chip 1..=9 (same as keyboard 1–9 in TUI).
    pub fn git_wb_set_tab(&mut self, key: u32) {
        use suisei_core::git_workbench::{GitPane, GitTab};
        if !self.app.git_wb.open {
            return;
        }
        // Keep workbench mode so tick/recompose keep loading PR/Issues.
        self.app.mode = Mode::GitWorkbench;
        match key {
            1 => {
                self.app.git_wb.tab = GitTab::Status;
                self.app.git_wb.pane = GitPane::Changes;
            }
            2 => {
                self.app.git_wb.tab = GitTab::History;
                self.app.git_wb.pane = GitPane::Log;
            }
            3 => {
                self.app.git_wb.tab = GitTab::Branches;
            }
            4 => {
                self.app.git_wb.tab = GitTab::Status;
                self.app.git_wb.pane = GitPane::Files;
            }
            5 => self.app.git_wb.tab = GitTab::Diff,
            6 => self.app.git_wb.tab = GitTab::PullRequests,
            7 => self.app.git_wb.tab = GitTab::Issues,
            8 => self.app.git_wb.tab = GitTab::Auth,
            9 => self.app.git_wb.tab = GitTab::Stash,
            _ => {}
        }
        if matches!(self.app.git_wb.tab, GitTab::History | GitTab::Commit)
            && self.app.git_wb.commits.is_empty()
        {
            self.app.git_wb.history_loaded = false;
        }
        if matches!(self.app.git_wb.tab, GitTab::Branches) && self.app.git_wb.branches.is_empty() {
            self.app.git_wb.branches_loaded = false;
        }
        // Allow re-fetch if previous load left empty after error / no-auth.
        if matches!(self.app.git_wb.tab, GitTab::PullRequests)
            && self.app.git_wb.prs.is_empty()
            && self.app.git_wb.prs_loaded
            && self.app.git_wb.error.is_some()
        {
            self.app.git_wb.prs_loaded = false;
        }
        if matches!(self.app.git_wb.tab, GitTab::Issues)
            && self.app.git_wb.issues.is_empty()
            && self.app.git_wb.issues_loaded
            && self.app.git_wb.error.is_some()
        {
            self.app.git_wb.issues_loaded = false;
        }
        // Kick background load immediately (don't wait for next recompose-only path).
        self.app.git_wb.ensure_tab_data();
        self.recompose();
    }

    pub fn git_wb_select_change(&mut self, row: u32) {
        match self.app.git_wb.request_change_preview(row as usize) {
            Ok(()) => {
                self.app.message = self.app.git_wb.message.clone().unwrap_or_default();
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_select_history(&mut self, row: u32) {
        match self.app.git_wb.request_history_preview(row as usize) {
            Ok(()) => {
                self.app.message = self.app.git_wb.message.clone().unwrap_or_default();
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_select_commit_file(&mut self, row: u32) {
        match self.app.git_wb.request_commit_file_preview(row as usize) {
            Ok(()) => {
                self.app.message = self.app.git_wb.message.clone().unwrap_or_default();
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_select_special(&mut self, row: u32) {
        self.app.git_wb.select_special_row(row as usize);
        self.recompose();
    }

    pub fn git_wb_select_branch_history(&mut self, row: u32) {
        match self.app.git_wb.select_branch_history(row as usize) {
            Ok(()) => {
                let branch = self
                    .app
                    .git_wb
                    .branches
                    .get(self.app.git_wb.branch_sel)
                    .map(|branch| branch.name.as_str())
                    .unwrap_or("branch");
                self.app.message = format!("History · {branch}");
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_refresh_window(&mut self) {
        self.app.git_wb.refresh_native_window();
        self.app.message = self
            .app
            .git_wb
            .message
            .clone()
            .unwrap_or_else(|| "Source Control refreshed".into());
        self.recompose();
    }

    pub fn git_wb_toggle_stage(&mut self, row: u32) {
        let count = self.app.git_wb.total_files();
        if count == 0 {
            return;
        }
        let selected = (row as usize).min(count - 1);
        let path = self
            .app
            .git_wb
            .entry_at(selected)
            .map(|entry| entry.path.clone());
        self.app.git_wb.selected = selected;
        match self.app.git_wb.stage_selected() {
            Ok(()) => {
                if let Some(path) = path {
                    if let Some(index) = self
                        .app
                        .git_wb
                        .staged
                        .iter()
                        .chain(self.app.git_wb.changes.iter())
                        .position(|entry| entry.path == path)
                    {
                        let _ = self.app.git_wb.request_change_preview(index);
                    }
                }
                self.app.message = self.app.git_wb.message.clone().unwrap_or_default();
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_stage_all(&mut self) {
        match self.app.git_wb.stage_all() {
            Ok(()) => {
                self.app.message = self.app.git_wb.message.clone().unwrap_or_default();
                if self.app.git_wb.total_files() > 0 {
                    let _ = self.app.git_wb.request_change_preview(0);
                }
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_unstage_all(&mut self) {
        match self.app.git_wb.unstage_all() {
            Ok(()) => {
                self.app.message = self.app.git_wb.message.clone().unwrap_or_default();
                if self.app.git_wb.total_files() > 0 {
                    let _ = self.app.git_wb.request_change_preview(0);
                }
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_commit(&mut self, message: &str, amend: bool) {
        match self.app.git_wb.commit_with_message(message, amend) {
            Ok(()) => {
                self.app.message = self.app.git_wb.message.clone().unwrap_or_default();
                self.app.refresh_git();
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_stash(&mut self) {
        match self.app.git_wb.stash() {
            Ok(()) => {
                self.app.message = self.app.git_wb.message.clone().unwrap_or_default();
                self.app.refresh_git();
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_discard_change(&mut self, row: u32) {
        let count = self.app.git_wb.total_files();
        if count == 0 {
            return;
        }
        self.app.git_wb.selected = (row as usize).min(count - 1);
        let result = self
            .app
            .git_wb
            .begin_discard_selected()
            .and_then(|()| self.app.git_wb.confirm_discard());
        match result {
            Ok(()) => {
                self.app.message = self.app.git_wb.message.clone().unwrap_or_default();
                self.app.refresh_git();
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    /// Open the model that backs the native Source Control window.
    ///
    /// This is deliberately not a toggle: a window can become key, resign key,
    /// and be ordered front repeatedly without its model being torn down.
    pub fn git_wb_open_window(&mut self) {
        if !self.app.git_wb.open {
            self.app.open_git_workbench();
        } else {
            self.app.mode = Mode::GitWorkbench;
            self.app.git_wb.ensure_tab_data();
        }
        self.recompose();
    }

    pub fn git_wb_focus_window(&mut self) {
        if self.app.git_wb.open {
            self.app.mode = Mode::GitWorkbench;
            self.app.git_wb.ensure_tab_data();
            self.recompose();
        }
    }

    pub fn git_wb_close_window(&mut self) {
        if self.app.git_wb.open {
            self.app.close_git_workbench();
            self.recompose();
        }
    }

    pub fn git_wb_checkout_selected_branch(&mut self) {
        match self.app.git_wb.checkout_selected_branch() {
            Ok(()) => {
                self.app.message = self
                    .app
                    .git_wb
                    .message
                    .clone()
                    .unwrap_or_else(|| "Checked out".into());
                self.app.refresh_git();
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn git_wb_create_branch(&mut self, name: &str) {
        self.app.git_wb.begin_new_branch();
        self.app.git_wb.input_buf = name.trim().to_string();
        match self.app.git_wb.submit_input() {
            Ok(()) => {
                self.app.message = self
                    .app
                    .git_wb
                    .message
                    .clone()
                    .unwrap_or_else(|| "Branch created".into());
                self.app.refresh_git();
            }
            Err(error) => {
                self.app.git_wb.cancel_input();
                self.app.message = error;
            }
        }
        self.recompose();
    }

    pub fn git_wb_delete_selected_branch(&mut self) {
        match self.app.git_wb.delete_selected_branch() {
            Ok(()) => {
                self.app.message = self
                    .app
                    .git_wb
                    .message
                    .clone()
                    .unwrap_or_else(|| "Branch deleted".into());
                self.app.refresh_git();
            }
            Err(error) => self.app.message = error,
        }
        self.recompose();
    }

    pub fn settings_goto_page(&mut self, page: u32) {
        use suisei_core::settings::SettingsPage;
        if !self.app.settings.visible() {
            return;
        }
        let pages = SettingsPage::all();
        if let Some(p) = pages.get(page as usize).copied() {
            // Walk pages via next until match (keeps panel invariants)
            let mut guard = 0;
            while self.app.settings.page != p && guard < 8 {
                self.app.settings.next_page();
                guard += 1;
            }
            self.recompose();
        }
    }

    pub fn scm_select(&mut self, row: u32) {
        let count = self.app.scm.total_files();
        if count == 0 {
            return;
        }
        self.app.scm.selected = (row as usize).min(count - 1);
        self.app.scm.focus = suisei_core::scm::ScmFocus::Changes;
        self.recompose();
    }

    pub fn scm_activate(&mut self, row: u32) {
        self.scm_select(row);
        self.app.scm_open_selected_file();
        self.recompose();
    }

    pub fn scm_toggle_stage(&mut self, row: u32) {
        self.scm_select(row);
        self.app.scm_stage_selected();
        self.recompose();
    }

    pub fn pos_from_click(&self, buffer_row: u32, visual_col: u32) -> Position {
        let last = self.app.buffer.line_count().saturating_sub(1);
        let row = (buffer_row as usize).min(last);
        let col = self
            .app
            .buffer
            .screen_col_to_buffer_col(row, visual_col as usize);
        let max_col = self.app.buffer.line(row).chars().count();
        Position::new(row, col.min(max_col))
    }

    /// Map face editor local coords → buffer row/col using current scroll + geometry.
    pub fn hit_test(
        &self,
        local_x: f32,
        local_y: f32,
        gutter_px: f32,
        cell_px: f32,
        line_height_px: f32,
    ) -> (u32, u32) {
        let lh = line_height_px.max(1.0);
        let cell = cell_px.max(1.0);
        // Match face paint: content offset by -scroll_frac * lineHeight.
        let y_adj = (local_y / lh) + self.app.scroll_frac as f32;
        let row_in_view = y_adj.floor().max(0.0) as usize;
        let last = self.app.buffer.line_count().saturating_sub(1);
        let buffer_row = self.app.scroll.saturating_add(row_in_view).min(last);
        let text_x = (local_x - gutter_px).max(0.0);
        let visual_col = (text_x / cell).floor().max(0.0) as u32
            + if self.app.wrap_lines {
                0
            } else {
                self.app.hscroll as u32
            };
        (buffer_row as u32, visual_col)
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suisei_core::key::{KeyCode, KeyEvent, KeyModifiers};

    fn eng_with_text(s: &str) -> Engine {
        // Pure GUI: typing just types — no synthetic `i`/`Esc`. Each key routes
        // through the Selection-model edits.
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        for ch in s.chars() {
            if ch == '\n' {
                eng.dispatch_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            } else {
                eng.dispatch_key(KeyEvent::char(ch));
            }
        }
        eng
    }

    #[test]
    fn typing_a_char_updates_chrome() {
        // Pure GUI: a character just types — no `i` to enter a mode first.
        let mut eng = Engine::new();
        eng.recompose();
        eng.dispatch_key(KeyEvent::char('a'));
        assert!(!eng.last_diff.chrome.as_ref().unwrap().welcome);
        assert_eq!(eng.app.buffer.text(), "a");
    }

    #[test]
    fn ime_space_commit_between_typed_syllables_keeps_order() {
        // A Korean 2-set IME does not commit the composing syllable and the
        // following space as two events. Pressing space turns the marked text
        // into "요 " and commits both scalars at once — which the face routes
        // through `paste_text`, while every other syllable arrives as a fast
        // single-scalar `dispatch_key`. Paste advanced `buffer.cursor` without
        // collapsing `sel`, so the next `gui_insert_text` read the stale
        // pre-paste head and inserted the syllable *inside* the pasted run:
        // "안녕하세요 안녕하세요 " came back as "안녕하세안녕하세요 요". Interleave
        // the two paths exactly as the IME drives them.
        let mut eng = Engine::new();
        eng.recompose();
        for ch in "안녕하세".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        eng.paste_text("요 ");
        for ch in "안녕하세".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        eng.paste_text("요 ");
        assert_eq!(eng.app.buffer.text(), "안녕하세요 안녕하세요 ");
        // `sel` must track the real caret, or the next keystroke desyncs again.
        assert_eq!(eng.app.sel.primary().head, eng.app.buffer.cursor());
    }

    #[test]
    fn ime_commit_in_the_middle_uses_gui_selection_position() {
        let mut eng = Engine::new();
        eng.app.buffer = suisei_core::buffer::Buffer::from_string("앞뒤");
        eng.app.caret_place(Position::new(0, 1));
        eng.paste_text("한글");
        assert_eq!(eng.app.buffer.text(), "앞한글뒤");
        assert_eq!(eng.app.sel.primary().head, Position::new(0, 3));
    }

    #[test]
    fn gui_fast_character_path_keeps_bracket_and_quote_pairs() {
        let mut eng = Engine::new();
        eng.gui_type_char('(');
        eng.gui_type_char('"');
        assert_eq!(eng.app.buffer.text(), "(\"\")");
        assert_eq!(eng.app.sel.primary().head, Position::new(0, 2));
    }

    #[test]
    fn enter_keeps_both_lines_visible() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::char('i'));
        eng.dispatch_key(KeyEvent::char('a'));
        eng.dispatch_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        eng.dispatch_key(KeyEvent::char('b'));
        // The per-keystroke hot path no longer packs the line stream (the GUI
        // pulls its own rows); force a full compose to assert on the built lines.
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(c.line_count >= 2);
        assert!(c.lines.len() >= 2);
        assert_eq!(c.lines[0].line_no, 1);
    }

    #[test]
    fn click_places_caret_without_entering_visual() {
        let mut eng = eng_with_text("hello");
        eng.click_at(0, 2, false);
        assert_eq!(eng.app.buffer.cursor.col, 2);
        assert!(matches!(eng.app.mode, Mode::Editor));
        eng.mouse_up();
        assert!(matches!(eng.app.mode, Mode::Editor));
        assert!(eng.app.selected_range().is_none());
    }

    #[test]
    fn click_moves_cursor() {
        let mut eng = eng_with_text("hello");
        eng.click_at(0, 1, false);
        assert_eq!(eng.app.buffer.cursor.col, 1);
        assert!(eng.app.sel.primary().is_empty());
        eng.mouse_up();
        assert!(eng.app.sel.primary().is_empty());
    }

    #[test]
    fn secondary_caret_composes_a_kind_250_span() {
        // GUI multi-cursor render contract: every caret except the primary is
        // emitted as a kind-250 span on its row; the primary is NOT a span (it
        // rides the dedicated caret_* fields). Guards the app.sel → compositor
        // wiring end to end.
        use suisei_core::buffer::Position;
        let mut eng = eng_with_text("hello\nworld");
        // caret_add makes the *added* caret primary, so place the intended
        // secondary first, then add the primary elsewhere.
        eng.app.caret_place(Position::new(1, 2)); // secondary on line 2 ("world")
        eng.app.caret_add(Position::new(0, 0)); // added → primary on line 1
        let (lines, _) = eng.editor_band(0, 0, 20);

        let line2 = lines.iter().find(|l| l.line_no == 2).expect("row 2");
        let carets: Vec<u32> = line2
            .spans
            .iter()
            .filter(|s| s.kind == 250)
            .map(|s| s.start)
            .collect();
        assert_eq!(
            carets,
            vec![2],
            "secondary caret at UTF-16 col 2 of 'world'"
        );

        // The primary (line 1) must not be duplicated as a kind-250 span.
        let line1 = lines.iter().find(|l| l.line_no == 1).expect("row 1");
        assert!(
            !line1.spans.iter().any(|s| s.kind == 250),
            "primary caret must not also be a kind-250 span"
        );
    }

    #[test]
    fn secondary_caret_offset_is_utf16_not_cell_grid() {
        // The reason the span carries a UTF-16 offset instead of a cell column:
        // on a CJK line the two diverge. Caret after 3 wide glyphs sits at cell
        // column 6 but UTF-16 offset 3 — the face measures glyph 3 with CoreText.
        use suisei_core::buffer::Position;
        let mut eng = eng_with_text("가나다x\nabc");
        eng.app.caret_place(Position::new(0, 3)); // secondary after 가나다 (3 wide glyphs)
        eng.app.caret_add(Position::new(1, 0)); // added → primary on "abc"
        let (lines, _) = eng.editor_band(0, 0, 20);
        let cjk = lines.iter().find(|l| l.line_no == 1).expect("row 1");
        let start = cjk
            .spans
            .iter()
            .find(|s| s.kind == 250)
            .map(|s| s.start)
            .expect("secondary caret span");
        assert_eq!(start, 3, "UTF-16 offset (3), not the cell column (6)");
    }

    #[test]
    fn drag_builds_gui_selection() {
        // Mouse drag now drives the GUI SelectionSet (exclusive), not vim
        // Visual mode. The painted span must still equal the yank slice.
        let mut eng = eng_with_text("abcdef");
        eng.click_at(0, 1, false);
        eng.drag_to(0, 1); // same cell — still a caret
        assert!(eng.app.sel.primary().is_empty());
        eng.drag_to(0, 4); // exclusive head at col 4 → covers chars 1,2,3
        assert!(!eng.app.sel.primary().is_empty());
        assert!(matches!(eng.app.mode, Mode::Editor)); // selection never changes focus

        let (s, e) = eng.app.selected_range().expect("selection");
        assert_eq!((s.row, s.col), (0, 1), "anchor at first click");
        assert_eq!(
            (e.row, e.col),
            (0, 3),
            "inclusive end = one before excl head"
        );

        eng.recompose(); // lines are built on full compose, not the hot path
        let line = &eng.last_diff.chrome.as_ref().unwrap().lines[0];
        let v0 = line.sel_v0.expect("sel_v0");
        let v1 = line.sel_v1.expect("sel_v1");
        let painted: String = line
            .text
            .chars()
            .skip(v0 as usize)
            .take((v1 - v0) as usize)
            .collect();
        let chars: Vec<char> = eng.app.buffer.line(0).chars().collect();
        let yanked: String = chars[s.col..=e.col].iter().collect();
        assert_eq!(painted, yanked, "paint span must equal yank slice");
        assert_eq!(yanked, "bcd");

        eng.mouse_up();
        assert!(
            eng.app.selected_range().is_some(),
            "selection survives mouse up"
        );
    }

    #[test]
    fn painted_selection_matches_yank_slice() {
        let mut eng = eng_with_text("hello");
        eng.click_at(0, 2, false);
        eng.drag_to(0, 2);
        assert!(eng.app.sel.primary().is_empty());
        eng.drag_to(0, 4); // exclusive [2,4) → chars 2,3
        let (s, e) = eng.app.selected_range().unwrap();
        eng.recompose(); // lines are built on full compose, not the hot path
        let line = &eng.last_diff.chrome.as_ref().unwrap().lines[0];
        let v0 = line.sel_v0.unwrap();
        let v1 = line.sel_v1.unwrap();
        let painted: String = line
            .text
            .chars()
            .skip(v0 as usize)
            .take((v1 - v0) as usize)
            .collect();
        let chars: Vec<char> = eng.app.buffer.line(0).chars().collect();
        let yanked: String = chars[s.col..=e.col].iter().collect();
        assert_eq!(painted, yanked);
        assert_eq!(yanked, "ll");
    }

    #[test]
    fn drag_multiline_selection() {
        let mut eng = eng_with_text("aa\nbb\ncc");
        eng.click_at(0, 0, false);
        eng.drag_to(2, 1);
        assert!(!eng.app.sel.primary().is_empty());
        let sel = eng.app.selected_range().unwrap();
        assert_eq!(sel.0.row, 0);
        assert_eq!(sel.1.row, 2);
        eng.mouse_up();
        assert!(eng.app.selected_range().is_some());
    }

    #[test]
    fn plain_click_collapses_a_mouse_selection() {
        let mut eng = eng_with_text("abcdef");
        eng.click_at(0, 0, false);
        eng.drag_to(0, 5);
        eng.mouse_up();
        assert!(eng.app.selected_range().is_some(), "drag made a selection");
        // A fresh plain click collapses it to a caret (GUI convention).
        eng.click_at(0, 2, false);
        assert!(eng.app.selected_range().is_none());
        assert!(eng.app.sel.primary().is_empty());
    }

    #[test]
    fn shift_arrow_extends_selection_via_keyboard() {
        let mut eng = eng_with_text("hello world");
        eng.click_at(0, 0, false);
        eng.mouse_up();
        eng.dispatch_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        eng.dispatch_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        assert!(!eng.app.sel.primary().is_empty());
        assert!(
            matches!(eng.app.mode, Mode::Editor),
            "selection never changes focus"
        );
        let (s, e) = eng.app.selected_range().unwrap();
        assert_eq!((s.row, s.col), (0, 0));
        assert_eq!((e.row, e.col), (0, 1)); // exclusive head at 2 → inclusive 1 ("he")
    }

    #[test]
    fn plain_arrow_collapses_selection_to_edge() {
        let mut eng = eng_with_text("hello");
        eng.click_at(0, 0, false);
        eng.mouse_up();
        eng.dispatch_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        eng.dispatch_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        assert!(!eng.app.sel.primary().is_empty());
        // Plain Right collapses to the far edge without moving further.
        eng.dispatch_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(eng.app.sel.primary().is_empty());
        assert_eq!(eng.app.buffer.cursor.col, 2);
    }

    #[test]
    fn legacy_cursor_move_collapses_gui_selection() {
        // Coherence guard for the de-vim transition: any legacy path that
        // moves the cursor on its own (here a vim `l`, which the GUI nav
        // interceptor does NOT claim) collapses the GUI selection so it never
        // lingers stale. Once typing is fully de-moded this also covers
        // type-over.
        let mut eng = eng_with_text("hello"); // Normal mode
        eng.click_at(0, 0, false);
        eng.mouse_up();
        eng.dispatch_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        eng.dispatch_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        assert!(!eng.app.sel.primary().is_empty());
        eng.dispatch_key(KeyEvent::char('l')); // vim right — moves the cursor
        assert!(eng.app.sel.primary().is_empty(), "selection collapsed");
        assert_eq!(eng.app.sel.primary().head, eng.app.buffer.cursor());
    }

    #[test]
    fn shift_alt_right_selects_a_word() {
        let mut eng = eng_with_text("foo bar");
        eng.click_at(0, 0, false);
        eng.mouse_up();
        eng.dispatch_key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::SHIFT.union(KeyModifiers::ALT),
        ));
        assert!(!eng.app.sel.primary().is_empty());
        let (s, _e) = eng.app.selected_range().unwrap();
        assert_eq!((s.row, s.col), (0, 0));
        assert!(eng.app.sel.primary().head.col >= 3, "past 'foo'");
    }

    #[test]
    fn pure_click_does_not_leave_sticky_visual() {
        let mut eng = eng_with_text("hello world");
        eng.click_at(0, 3, false);
        eng.mouse_up();
        assert!(matches!(eng.app.mode, Mode::Editor));
        // second click elsewhere
        eng.click_at(0, 7, false);
        eng.mouse_up();
        assert!(matches!(eng.app.mode, Mode::Editor));
        assert!(eng.app.selected_range().is_none());
    }

    #[test]
    fn scroll_by_moves_window() {
        let mut eng = eng_with_text(&"line\n".repeat(50));
        eng.app.scroll = 0;
        eng.scroll_by(5);
        assert_eq!(eng.app.scroll, 5);
        eng.scroll_by(-2);
        assert_eq!(eng.app.scroll, 3);
    }

    #[test]
    fn hit_test_maps_geometry() {
        let eng = eng_with_text("hello");
        // gutter 48, cell 8, line height 17
        let (row, col) = eng.hit_test(48.0 + 16.0, 0.0, 48.0, 8.0, 17.0);
        assert_eq!(row, 0);
        assert_eq!(col, 2);
        let (row2, _) = eng.hit_test(50.0, 17.0 * 2.0 + 1.0, 48.0, 8.0, 17.0);
        // single-line buffer: y past end clamps to last row
        assert_eq!(row2, 0);
        let eng2 = eng_with_text("a\nb\nc\nd");
        let (row3, _) = eng2.hit_test(50.0, 17.0 * 2.0 + 1.0, 48.0, 8.0, 17.0);
        assert_eq!(row3, 2);
    }

    #[test]
    fn cmd_s_saves_via_dispatch() {
        let dir = std::env::temp_dir().join("suisei_save_test2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("t.txt");
        let _ = std::fs::write(&path, "");
        let mut eng = Engine::new();
        eng.app = App::open_file(path.to_str().unwrap());
        eng.app.stage.h = 720.0; // 40 rows
        eng.app.stage.w = 900.0; // 100 cols
        eng.dispatch_key(KeyEvent::char('i'));
        eng.dispatch_key(KeyEvent::char('z'));
        eng.dispatch_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        eng.dispatch_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::SUPER));
        assert!(!eng.app.modified);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains('z'));
    }

    #[test]
    fn double_click_word_selects() {
        let mut eng = eng_with_text("foo bar baz");
        // place on 'b' of bar — visual col depends on expand; "foo " = 4
        eng.click_at(0, 4, true);
        assert!(
            !eng.app.sel.primary().is_empty(),
            "word selected into GUI model"
        );
        let (s, e) = eng.app.selected_range().expect("selection");
        let chars: Vec<char> = eng.app.buffer.line(0).chars().collect();
        let word: String = chars[s.col..=e.col].iter().collect();
        assert_eq!(word, "bar");
    }

    #[test]
    fn ctrl_f_opens_explorer_in_frame() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(c.explorer.open, "Ctrl+F must open explorer via Core");
        assert!(
            !c.explorer.entries.is_empty() || c.explorer.cwd.len() > 0,
            "explorer should have cwd"
        );
        assert!(matches!(eng.app.mode, Mode::Explorer));
    }

    #[test]
    fn insert_triggers_completions_scene() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::char('i'));
        // type a common prefix that keywords might match
        for ch in "fn".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        eng.recompose();
        // Completions may or may not activate depending on buffer/ext; call activate path
        if !eng.app.completions.active {
            eng.app.completions.activate("fn", Some("rs"));
            eng.recompose();
        }
        let c = eng.last_diff.chrome.as_ref().unwrap();
        if eng.app.completions.active {
            assert!(c.completions.open);
            assert!(!c.completions.items.is_empty());
        }
    }

    #[test]
    fn ctrl_t_opens_terminal_scene() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        eng.recompose();
        // toggle may open side terminal
        let open = eng.app.terminal.open || matches!(eng.app.mode, Mode::Terminal);
        let c = eng.last_diff.chrome.as_ref().unwrap();
        if open {
            assert!(c.terminal.open);
        }
    }

    #[test]
    fn ctrl_p_opens_palette_in_frame() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        // Cmd+P file palette (SUPER+p)
        eng.dispatch_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::SUPER));
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(c.palette.open, "Cmd+P must open palette");
        assert!(matches!(eng.app.mode, Mode::Palette));
    }

    #[test]
    fn goto_tab_switches_buffer() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app.open_blank_tab();
        eng.recompose();
        assert!(eng.app.tabs.buffers.len() >= 2);
        eng.goto_tab(0);
        assert_eq!(eng.app.current_buffer(), 0);
        eng.goto_tab(1);
        assert_eq!(eng.app.current_buffer(), 1);
    }

    #[test]
    fn vertical_split_paints_full_height_rows_not_half() {
        let mut eng = eng_with_text(&"line\n".repeat(80));
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        let rows_full = eng.app.grid_rows() as usize;
        eng.split_vertical();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(c.panes.len() >= 2, "vertical split paints two panes");
        // Each side-by-side pane must keep ~full height (minus path bar), not rows/2.
        let per = c.panes[0].lines.len();
        assert!(
            per >= rows_full.saturating_sub(4).max(8) / 2 + 4,
            "pane rows {per} too small vs viewport {rows_full} (half-paint regression)"
        );
        // Stronger: must be more than half of full rows (old bug was rows/n).
        assert!(
            per > rows_full / 2,
            "expected full-height pane paint, got {per} of {rows_full}"
        );
    }

    #[test]
    fn split_panes_show_different_tabs_independently() {
        let dir = std::env::temp_dir().join("suisei_split_tabs");
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("left.txt");
        let b = dir.join("right.txt");
        std::fs::write(&a, &format!("LEFT_ONLY_AAA\n{}", "left\n".repeat(80))).unwrap();
        std::fs::write(&b, &format!("RIGHT_ONLY_BBB\n{}", "right\n".repeat(80))).unwrap();
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(a.to_str().unwrap());
        eng.recompose();
        eng.split_vertical();
        // Focus right pane and open B.
        eng.focus_pane(1);
        eng.app.open_new_tab(b.to_str().unwrap());
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert_eq!(c.panes.len(), 2);
        assert_ne!(
            c.panes[0].tab_index, c.panes[1].tab_index,
            "panes must track different tabs after open on focused pane"
        );
        let left_has = c.panes[0]
            .lines
            .iter()
            .any(|l| l.text.contains("LEFT_ONLY_AAA"));
        let right_has = c.panes[1]
            .lines
            .iter()
            .any(|l| l.text.contains("RIGHT_ONLY_BBB"));
        assert!(left_has, "left pane must still paint file A");
        assert!(
            right_has,
            "right pane must paint file B without waiting for click"
        );
        // Scroll left — right content must not become A.
        eng.focus_pane(0);
        eng.scroll_by(12);
        eng.recompose(); // lines are built on full compose, not the scroll hot path
        let c2 = eng.last_diff.chrome.as_ref().unwrap();
        assert!(
            c2.panes[1]
                .lines
                .iter()
                .any(|l| l.text.contains("RIGHT_ONLY_BBB")),
            "scrolling left must not clobber right pane content"
        );
        // Left advanced; right keeps its own scroll mirror.
        assert_eq!(
            c2.panes[1].scroll, 0,
            "right pane scroll must stay independent"
        );
        assert!(c2.panes[0].scroll > 0, "left pane should advance scroll");
        assert!(
            c2.panes[0]
                .lines
                .iter()
                .any(|l| l.text.contains("left") || l.text.contains("LEFT_ONLY_AAA")),
            "left still A"
        );
    }

    /// The §S1 gate from `SUISEI-SPLIT-PLAN.md`, driven through the real paint
    /// path: with two panes on two files, closing an *unrelated* third tab
    /// leaves both panes painting what they were painting.
    ///
    /// Two separate mechanisms used to break this and they cancelled out in no
    /// useful way. Panes addressed documents by position, so removing an
    /// earlier tab slid every pane one file along; and `close_tab` finished by
    /// pushing the newly active document into the focused pane. Both are gone
    /// — panes hold a `BufferId` and the editor follows the pane.
    #[test]
    fn closing_a_tab_leaves_split_panes_painting_their_own_files() {
        let dir = std::env::temp_dir().join("suisei_close_tab_panes");
        let _ = std::fs::create_dir_all(&dir);
        let (a, b, c) = (dir.join("a.txt"), dir.join("b.txt"), dir.join("c.txt"));
        std::fs::write(&a, format!("DOC_AAA\n{}", "a\n".repeat(40))).unwrap();
        std::fs::write(&b, format!("DOC_BBB\n{}", "b\n".repeat(40))).unwrap();
        std::fs::write(&c, format!("DOC_CCC\n{}", "c\n".repeat(40))).unwrap();

        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(a.to_str().unwrap());
        eng.recompose();
        eng.split_vertical();
        eng.focus_pane(0);
        eng.app.open_new_tab(b.to_str().unwrap());
        eng.focus_pane(1);
        eng.app.open_new_tab(c.to_str().unwrap());
        eng.recompose();

        let paints = |eng: &Engine, pane: usize, mark: &str| -> bool {
            eng.last_diff.chrome.as_ref().unwrap().panes[pane]
                .lines
                .iter()
                .any(|l| l.text.contains(mark))
        };
        assert_eq!(eng.app.tabs.buffers.len(), 3);
        assert!(paints(&eng, 0, "DOC_BBB"), "left pane starts on B");
        assert!(paints(&eng, 1, "DOC_CCC"), "right pane starts on C");

        // Close tab A — the one file no pane is showing.
        eng.close_tab(0);
        assert_eq!(eng.app.tabs.buffers.len(), 2);
        assert!(paints(&eng, 0, "DOC_BBB"), "left pane must still paint B");
        assert!(paints(&eng, 1, "DOC_CCC"), "right pane must still paint C");
    }

    /// The §S2 gate: a pane's viewport survives focus leaving and coming back.
    ///
    /// Two things had to be true and neither was. The focused pane's scroll
    /// had to be parked somewhere before focus moved (it was written to the
    /// slot from ~20 scattered call sites, any one of which could be missed),
    /// and restoring it had to not immediately undo itself — the old
    /// `apply_focused_pane` finished with `update_scroll()`, which re-derives
    /// scroll from the caret and therefore threw away any scroll the wheel had
    /// produced, since the wheel does not move the caret.
    #[test]
    fn a_pane_keeps_its_viewport_across_focus_changes() {
        let dir = std::env::temp_dir().join("suisei_pane_viewport");
        let _ = std::fs::create_dir_all(&dir);
        let long = dir.join("long.txt");
        let body: String = (0..400).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&long, &body).unwrap();

        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(long.to_str().unwrap());
        eng.recompose();
        eng.split_vertical();

        // Scroll pane 0 with the wheel — the caret stays at the top.
        eng.focus_pane(0);
        eng.scroll_by(40);
        let parked = eng.app.scroll;
        assert!(parked > 0, "pane 0 should have scrolled, got {parked}");
        assert_eq!(
            eng.app.buffer.cursor().row,
            0,
            "the wheel must not move the caret"
        );

        eng.focus_pane(1);
        assert_eq!(eng.app.scroll, 0, "pane 1 has its own viewport");

        eng.focus_pane(0);
        assert_eq!(eng.app.scroll, parked, "pane 0 came back to where it was");

        // And again, to catch a park that only works the first time.
        eng.focus_pane(1);
        eng.focus_pane(0);
        assert_eq!(
            eng.app.scroll, parked,
            "still there after a second round trip"
        );
    }

    /// J6 as specified: ⌃⇧T turns the **focused pane** into a terminal and
    /// leaves the document it displaced reachable from the tab bar. No split
    /// is created and none is destroyed.
    ///
    /// What used to happen instead: a *new split* was conjured to host the
    /// shell, the document was never displaced, and `owns_split` was recorded
    /// so the conjured split could be collapsed again later.
    #[test]
    fn terminal_takes_over_the_focused_pane_and_leaves_the_file_reachable() {
        let dir = std::env::temp_dir().join("suisei_j6_pane_terminal");
        let _ = std::fs::create_dir_all(&dir);
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "DOC_AAA\n").unwrap();
        std::fs::write(&b, "DOC_BBB\n").unwrap();

        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(a.to_str().unwrap());
        eng.recompose();
        eng.split_vertical();
        eng.focus_pane(1);
        eng.app.open_new_tab(b.to_str().unwrap());
        eng.recompose();

        let panes_before = eng.app.split.pane_count();
        let tabs_before = eng.app.tabs.buffers.len();

        eng.app.toggle_terminal_full();
        assert_eq!(
            eng.app.split.pane_count(),
            panes_before,
            "no split is created — the terminal is a tab in the focused pane"
        );
        assert_eq!(
            eng.app.tabs.buffers.len(),
            tabs_before + 1,
            "a terminal tab was added"
        );
        assert_eq!(eng.app.pane_terminals.len(), 1, "one shell, for that tab");
        assert!(
            eng.app.terminal_window_focused(),
            "focused pane shows the terminal tab"
        );

        // Toggling again closes the terminal tab.
        eng.app.toggle_terminal_full();
        assert_eq!(
            eng.app.tabs.buffers.len(),
            tabs_before,
            "terminal tab removed"
        );
        assert_eq!(
            eng.app.split.pane_count(),
            panes_before,
            "still no split churn"
        );
        assert!(eng.app.pane_terminals.is_empty(), "its shell ended with it");
    }

    /// Closing a pane terminal restores the exact document the pane showed
    /// before ⌃⇧T, and keeps the split — not just "a" document, and not a
    /// collapsed view.
    #[test]
    fn closing_a_pane_terminal_restores_the_previous_tab_and_keeps_the_split() {
        let dir = std::env::temp_dir().join("suisei_term_restore");
        let _ = std::fs::create_dir_all(&dir);
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "DOC_AAA\n").unwrap();
        std::fs::write(&b, "DOC_BBB\n").unwrap();

        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(a.to_str().unwrap());
        eng.recompose();
        eng.split_vertical();
        eng.focus_pane(1);
        eng.app.open_new_tab(b.to_str().unwrap());
        eng.recompose();
        let panes_before = eng.app.split.pane_count();
        assert!(
            eng.app.buffer.text().contains("DOC_BBB"),
            "pane 1 shows b.txt"
        );

        // ⌃⇧T over pane 1 → terminal tab takes the pane.
        eng.app.toggle_terminal_full();
        assert!(
            eng.app.terminal_window_focused(),
            "pane 1 is now a terminal"
        );
        assert!(
            !eng.app.buffer.text().contains("DOC_BBB"),
            "b.txt displaced"
        );

        // Close the terminal tab → split kept, b.txt restored into the pane.
        eng.app.toggle_terminal_full();
        assert_eq!(eng.app.split.pane_count(), panes_before, "split is kept");
        assert!(
            !eng.app.terminal_window_focused(),
            "pane is a document again"
        );
        assert!(
            eng.app.buffer.text().contains("DOC_BBB"),
            "the pane's pre-terminal document (b.txt) is restored, not collapsed away"
        );
    }

    /// Each terminal pane is its OWN process.
    ///
    /// One `App.terminal` served every terminal pane at first, so a second
    /// pane was a second view of the same session — they mirrored each other —
    /// and converting a second pane moved the shell instead of starting one.
    #[test]
    fn two_terminal_panes_are_two_shells() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.recompose();
        eng.split_vertical();

        eng.focus_pane(0);
        eng.app.toggle_terminal_full();
        eng.focus_pane(1);
        eng.app.toggle_terminal_full();

        assert_eq!(eng.app.split.pane_count(), 2, "still two panes");
        assert_eq!(eng.app.pane_terminals.len(), 2, "two live processes");
        let ids: Vec<_> = eng.app.pane_terminals.keys().collect();
        assert_ne!(ids[0], ids[1], "and they are different shells");

        // Closing one leaves the other's shell alone.
        eng.focus_pane(1);
        eng.app.toggle_terminal_full();
        assert_eq!(eng.app.pane_terminals.len(), 1, "one shell ended");
        assert!(
            eng.app.is_terminal_tab(eng.app.split.panes[0].buffer),
            "pane 0 kept its shell"
        );
    }

    /// A pane terminal's PTY must take its size from the face's measurement —
    /// before `terminal_resize_pane` existed, pane shells kept their spawn
    /// guess forever and vim/htop drew garbled.
    #[test]
    fn pane_terminal_resize_reaches_the_pane_pty() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.recompose();
        eng.split_vertical();
        eng.focus_pane(0);
        eng.app.toggle_terminal_full();
        eng.focus_pane(1);
        eng.app.toggle_terminal_full();

        // Resize pane 1's shell; pane 0's must not move.
        let before = eng.app.pane_terminal(0).map(|t| (t.cols(), t.rows_count()));
        eng.terminal_resize_pane(1, 111, 33);
        let p1 = eng.app.pane_terminal(1).expect("pane 1 runs a shell");
        assert_eq!(
            (p1.cols(), p1.rows_count()),
            (111, 33),
            "pane 1 PTY resized"
        );
        let p0 = eng.app.pane_terminal(0).expect("pane 0 runs a shell");
        assert_eq!(
            Some((p0.cols(), p0.rows_count())),
            before,
            "pane 0 untouched"
        );

        // Out-of-range pane is a no-op, not a panic.
        eng.terminal_resize_pane(7, 80, 24);
    }

    /// Dock controls must not touch pane shells. They are separate processes,
    /// and before the per-pane entries existed, a resize or scroll aimed at
    /// the dock was the only resize a pane ever saw — by accident, through
    /// the face misreporting pane geometry into the dock.
    #[test]
    fn dock_resize_and_scroll_leave_pane_terminals_alone() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.split_vertical();
        eng.focus_pane(0);
        eng.app.toggle_terminal_full();
        let (cols0, rows0) = eng
            .app
            .pane_terminal(0)
            .map(|t| (t.cols(), t.rows_count()))
            .expect("pane shell running");

        // Open the dock and work its controls.
        eng.app.toggle_terminal_side();
        eng.terminal_resize(133, 44);
        eng.terminal_scroll(7);

        assert_eq!(eng.app.terminal.cols(), 133, "dock did resize");
        let p = eng.app.pane_terminal(0).expect("pane shell still there");
        assert_eq!(
            (p.cols(), p.rows_count()),
            (cols0, rows0),
            "pane grid untouched"
        );
        assert_eq!(p.scroll(), 0, "pane scroll untouched");
    }

    /// The chip of a terminal tab carries the shell's own OSC title once the
    /// shell reports one — every tab used to read "Terminal".
    #[test]
    fn terminal_tab_title_comes_from_the_shell() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.recompose();
        eng.app.toggle_terminal_full();
        {
            let term = eng
                .app
                .pane_terminals
                .values_mut()
                .next()
                .expect("pane shell");
            term.title = Some("make [42]".into());
        }
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        let titles: Vec<&str> = c.tabs.iter().map(|t| t.title.as_str()).collect();
        assert!(
            titles.contains(&"make [42]"),
            "shell title on the chip: {titles:?}"
        );
    }

    /// build_tabs chip tagging: folded docs share a group id (grouped), the
    /// unified style collapses them to one is_layout chip whose id IS the
    /// layout id, and that chip sits at the run's anchor — not the strip end.
    #[test]
    fn build_tags_grouped_and_unified_chips() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.recompose();
        eng.app.open_blank_tab();
        eng.split_vertical();
        // A split copies the focused pane — point the two panes at two
        // different tabs so the fold has two documents to group.
        let (id0, id1) = (eng.app.tabs.buffers[0].id, eng.app.tabs.buffers[1].id);
        eng.app.tabs.buffers[0].filename = Some("/tmp/Alpha.rs".into());
        eng.app.tabs.buffers[1].filename = Some("/tmp/Beta.rs".into());
        eng.app.filename = eng.app.tabs.buffers[eng.app.current_buffer()]
            .filename
            .clone();
        eng.app.split.panes[0].buffer = id0;
        eng.app.split.panes[1].buffer = id1;
        assert!(eng.app.fold_layout());
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert_eq!(c.tabs.len(), 2, "two folded docs, two chips");
        let g = c.tabs[0].group;
        assert_ne!(g, 0, "folded chips carry a group");
        assert_eq!(c.tabs[1].group, g, "both chips share it");
        assert!(
            c.tabs.iter().all(|t| !t.is_layout),
            "grouped: no layout chip"
        );

        eng.app.toggle_layout_style(g);
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert_eq!(c.tabs.len(), 1, "unified collapses to one chip");
        assert!(c.tabs[0].is_layout);
        assert_eq!(c.tabs[0].id, g, "unified chip id is the layout id");
        let pane_titles: Vec<&str> = c.panes.iter().map(|pane| pane.title.as_str()).collect();
        assert_eq!(
            pane_titles,
            vec!["Alpha.rs", "Beta.rs"],
            "pane headers keep document identity behind the unified layout chip"
        );

        // A new loose doc lands AFTER the unified chip, because the chip
        // holds the run's anchor position (the first member's slot).
        eng.app.open_blank_tab();
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert_eq!(c.tabs.len(), 2);
        assert!(c.tabs[0].is_layout, "unified chip keeps the anchor slot");
        assert!(!c.tabs[1].is_layout);
    }

    /// Pane scrollback is per pane — scrolling one shell must not move the
    /// other's view.
    #[test]
    fn pane_terminal_scroll_moves_only_that_pane() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.recompose();
        eng.split_vertical();
        eng.focus_pane(0);
        eng.app.toggle_terminal_full();
        eng.focus_pane(1);
        eng.app.toggle_terminal_full();

        // Give pane 0 some scrollback: feed output through the emulator.
        // (No PTY round-trip needed — scroll_up bounds at the scrollback len,
        // so push rows by writing lines through a resize-induced reflow is
        // overkill; scroll math is what's under test.)
        eng.terminal_scroll_pane(0, 5);
        // No scrollback yet → offset stays 0, but the call must not panic and
        // must leave pane 1 alone either way.
        assert_eq!(eng.app.pane_terminal(1).map(|t| t.scroll()), Some(0));
        eng.terminal_scroll_pane(0, -3);
        assert_eq!(eng.app.pane_terminal(0).map(|t| t.scroll()), Some(0));
    }

    /// The unified chip sits at its first member's strip position — the merge
    /// animation morphs container ⇄ chip in place, so the chip may not jump
    /// to the strip's end (where the loose documents' slots would also stop
    /// being buffer indices).
    #[test]
    fn unified_chip_sits_at_its_first_members_position() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app.open_blank_tab();
        eng.app.open_blank_tab();
        eng.app.open_blank_tab();
        let ids: Vec<u64> = eng.app.tabs.buffers.iter().map(|t| t.id.0).collect();
        // Split shows docs 0 and 1; docs 2 and 3 stay loose after the run.
        eng.split_vertical();
        eng.focus_pane(0);
        eng.goto_tab_id(ids[0]);
        eng.focus_pane(1);
        eng.goto_tab_id(ids[1]);
        assert!(eng.fold_layout());
        let layout_id = eng.app.layouts[0].id;
        assert!(eng.toggle_layout_style(layout_id), "grouped → unified");

        eng.recompose();
        let chrome = eng.last_diff.chrome.as_ref().expect("composed");
        let chips: Vec<(u64, bool, u64)> = chrome
            .tabs
            .iter()
            .map(|t| (t.id, t.is_layout, t.group))
            .collect();
        assert_eq!(chips.len(), 3, "chip + two loose docs: {chips:?}");
        assert_eq!(
            chips[0],
            (layout_id, true, layout_id),
            "chip at the run's anchor"
        );
        assert_eq!(chips[1].0, ids[2], "loose doc keeps its order");
        assert_eq!(chips[2].0, ids[3]);
    }

    /// Face path: 2-tab split folded; close left pane via header (engine API).
    /// Group dissolves; both tabs stay; one pane remains on B.
    #[test]
    fn face_path_header_close_dissolves_two_tab_group() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app.open_blank_tab(); // second tab
        let ids: Vec<u64> = eng.app.tabs.buffers.iter().map(|t| t.id.0).collect();
        assert_eq!(ids.len(), 2);
        eng.split_vertical();
        eng.focus_pane(0);
        eng.goto_tab_id(ids[0]);
        eng.focus_pane(1);
        eng.goto_tab_id(ids[1]);
        assert!(eng.fold_layout());
        assert_eq!(eng.app.split.pane_count(), 2);
        assert_eq!(eng.app.layouts.len(), 1);

        eng.focus_pane(0);
        eng.close_focused_pane(); // == face closeFocusedPane

        assert!(eng.app.layouts.is_empty(), "group dissolved");
        assert_eq!(eng.app.active_layout, None);
        assert_eq!(eng.app.split.pane_count(), 1, "not still split");
        assert_eq!(eng.app.tabs.buffers.len(), 2, "tabs stay open");
        assert_eq!(
            eng.app.split.panes[0].buffer.0, ids[1],
            "survivor is B, not a repointed ghost"
        );
    }

    /// Face path: group A|B; close A from tab bar by stable id.
    /// A's pane must vanish — not leave B|B.
    #[test]
    fn face_path_tabbar_close_removes_member_pane() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app.open_blank_tab();
        let ids: Vec<u64> = eng.app.tabs.buffers.iter().map(|t| t.id.0).collect();
        eng.split_vertical();
        eng.focus_pane(0);
        eng.goto_tab_id(ids[0]);
        eng.focus_pane(1);
        eng.goto_tab_id(ids[1]);
        assert!(eng.fold_layout());

        eng.focus_pane(0);
        eng.close_tab_id(ids[0]); // == face closeTabId

        assert!(!eng.app.tabs.buffers.iter().any(|t| t.id.0 == ids[0]));
        assert_eq!(eng.app.split.pane_count(), 1, "must not stay B|B split");
        assert_eq!(eng.app.split.panes[0].buffer.0, ids[1]);
        assert!(eng.app.layouts.is_empty());
    }

    /// Id-addressed tab ops hit the named tab even while a folded group makes
    /// strip slots diverge from buffer indices — the slot-clamped close they
    /// replace killed the wrong document here.
    #[test]
    fn id_addressed_tab_ops_survive_a_folded_group() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app.open_blank_tab();
        eng.app.open_blank_tab();
        let ids: Vec<u64> = eng.app.tabs.buffers.iter().map(|t| t.id.0).collect();
        eng.split_vertical();
        eng.focus_pane(0);
        eng.goto_tab_id(ids[0]);
        eng.focus_pane(1);
        eng.goto_tab_id(ids[1]);
        assert!(eng.fold_layout());

        // Close the trailing loose tab by id.
        eng.close_tab_id(ids[2]);
        assert!(!eng.app.tabs.buffers.iter().any(|t| t.id.0 == ids[2]));
        assert!(eng.app.tabs.buffers.iter().any(|t| t.id.0 == ids[0]));

        // Drop the layout by id; both documents survive as loose tabs.
        let layout_id = eng.app.layouts[0].id;
        assert!(eng.drop_layout(layout_id));
        assert!(eng.app.layouts.is_empty());
        assert_eq!(
            eng.app.tabs.buffers.len(),
            2,
            "documents outlive their layout"
        );
    }

    /// Esc closes the file palette, all the way through `gui_escape`.
    ///
    /// Written to settle where an "Esc does not close the palette" report
    /// lived. It is not here: core routes Esc to `handle_palette`, the engine
    /// front-end does not intercept it, and the palette shuts. Anything that
    /// still looks stuck is above this line, in key delivery.
    #[test]
    fn esc_closes_the_file_palette() {
        let mut eng = Engine::new();
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        eng.app.open_file_palette();
        assert!(eng.app.palette.open, "palette should be open");
        eng.gui_escape();
        assert!(!eng.app.palette.open, "Esc must close the palette");
        assert!(matches!(eng.app.mode, Mode::Editor));
    }

    #[test]
    fn blank_tab_after_file_does_not_reenter_welcome() {
        let dir = std::env::temp_dir().join("suisei_blank_tab_welcome");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("a.txt");
        std::fs::write(&path, "hello\nworld\n").unwrap();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(path.to_str().unwrap());
        eng.sync_viewport_public();
        eng.recompose();
        assert!(!eng.last_diff.chrome.as_ref().unwrap().welcome);

        eng.open_blank_tab();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(
            !c.welcome,
            "new blank tab must stay in editor shell, not Welcome"
        );
        assert!(eng.app.tabs.buffers.len() >= 2);
        assert_eq!(c.tabs.len(), eng.app.tabs.buffers.len());
        // Active tab should be the new blank
        assert!(c.tabs.last().map(|t| t.active).unwrap_or(false));

        eng.goto_tab(0);
        let c2 = eng.last_diff.chrome.as_ref().unwrap();
        assert!(!c2.welcome);
        assert_eq!(eng.app.current_buffer(), 0);
        // Original file content still painted
        assert!(
            c2.lines.iter().any(|l| l.text.contains("hello")),
            "switching back must restore first buffer"
        );
    }

    #[test]
    fn open_path_file_in_session_adds_tab_not_wipe() {
        let dir = std::env::temp_dir().join("suisei_open_path_tabs");
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "AAA\n").unwrap();
        std::fs::write(&b, "BBB\n").unwrap();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(a.to_str().unwrap());
        eng.recompose();
        let n0 = eng.app.tabs.buffers.len();

        // Simulate suisei_engine_open_path session path via App API used by FFI.
        eng.app.open_new_tab(b.to_str().unwrap());
        eng.recompose();
        assert_eq!(eng.app.tabs.buffers.len(), n0 + 1);
        assert_eq!(eng.app.current_buffer(), eng.app.tabs.buffers.len() - 1);
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(c.tabs.len() >= 2);
        assert!(!c.welcome);
    }

    /// Viewer tabs carry an empty compatibility buffer because the rest of the
    /// editor addresses every open document through the same tab structure.
    /// That buffer is implementation detail, not something typing, IME commits
    /// or standard text chords may mutate.
    #[test]
    fn audio_viewer_input_never_marks_the_file_dirty() {
        let dir = std::env::temp_dir().join("suisei_audio_viewer_read_only");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("track.mp3");
        std::fs::write(&path, b"ID3").unwrap();

        let mut eng = Engine::new();
        eng.app = App::open_file(path.to_str().unwrap());
        assert_eq!(eng.app.live_tab_kind(), suisei_core::media::FileKind::Audio);
        let before_text = eng.app.buffer.text();
        let before_version = eng.app.buffer.version();

        eng.dispatch_key(KeyEvent::char('x'));
        eng.dispatch_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        eng.dispatch_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::SUPER));
        eng.gui_type_char('y');
        eng.gui_delete_backward();
        eng.paste_text("한글 IME commit");

        assert_eq!(eng.app.buffer.text(), before_text);
        assert_eq!(eng.app.buffer.version(), before_version);
        assert!(!eng.app.modified, "read-only audio viewer must stay clean");
        assert!(
            eng.app.tabs.buffers.iter().all(|tab| !tab.modified),
            "the tab-strip dirty dot must stay clear too"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// The compact Now Playing identity reopens its source through a
    /// standardised URL. That spelling can differ from the path originally
    /// supplied by the project tree without naming a different file.
    #[test]
    fn reopening_audio_by_an_equivalent_path_reuses_its_tab() {
        let dir = std::env::temp_dir().join("suisei_audio_tab_identity");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("track.mp3");
        std::fs::write(&path, b"ID3").unwrap();

        let lexical_path = dir.join(".").join("track.mp3");
        let mut eng = Engine::new();
        eng.app = App::open_file(lexical_path.to_str().unwrap());
        let original_id = eng.app.current_buffer_id();

        eng.app.open_new_tab(path.to_str().unwrap());

        assert_eq!(eng.app.tabs.buffers.len(), 1, "must not duplicate the tab");
        assert_eq!(eng.app.current_buffer_id(), original_id);
        let _ = std::fs::remove_file(&path);
    }

    /// Settings account state is engine-owned and starts empty. A cancel
    /// with no session is a no-op, and a refresh without `gh` still produces
    /// a snapshot the face can draw.
    #[test]
    fn github_account_starts_empty_and_cancel_is_safe() {
        let mut eng = Engine::new();
        assert_eq!(eng.github_account.generation, 0);
        assert!(!eng.github_account.signing_in());
        assert!(eng.github_account.profile.login.is_empty());
        eng.github_account.cancel_sign_in();
        assert!(!eng.github_account.signing_in());
        eng.github_account.ensure_loaded();
        assert!(eng.github_account.generation > 0);
    }

    #[test]
    fn explorer_activate_opens_file_when_present() {
        let dir = std::env::temp_dir().join("suisei_expl_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sample.txt");
        std::fs::write(&path, "hi\n").unwrap();
        let mut eng = Engine::new();
        eng.app.explorer.cwd = dir.clone();
        eng.app.explorer.open = true;
        eng.app.explorer.refresh();
        eng.app.mode = Mode::Explorer;
        eng.recompose();
        let idx = eng
            .app
            .explorer
            .entries
            .iter()
            .position(|e| e.name == "sample.txt")
            .expect("sample.txt in explorer");
        eng.explorer_activate(idx as u32);
        assert!(matches!(eng.app.mode, Mode::Editor));
        // Docked navigator keeps tree open after file activate.
        assert!(eng.app.explorer.open);
        assert!(
            !eng.app.explorer.entries.is_empty(),
            "file tree must remain after open"
        );
        assert!(
            eng.app
                .filename
                .as_ref()
                .map(|p| p.ends_with("sample.txt"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn open_path_dir_leaves_welcome() {
        let dir = std::env::temp_dir().join("suisei_open_dir_welcome");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("README.md"), "# hi\n").unwrap();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.recompose();
        assert!(eng.last_diff.chrome.as_ref().unwrap().welcome);
        // Simulate open_path dir via same logic as FFI
        eng.app = App::open_file(dir.join("README.md").to_str().unwrap());
        eng.app.explorer.cwd = dir.clone();
        eng.app.explorer.refresh();
        eng.app.explorer.open = true;
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(!c.welcome, "opening a project file must leave welcome");
        assert!(!c.outline.is_empty() || c.filename.contains("README"));
    }

    #[test]
    fn ensure_project_tree_fills_entries_without_mode_explorer() {
        let dir = std::env::temp_dir().join("suisei_proj_tree");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("a.txt");
        std::fs::write(&path, "x\n").unwrap();
        let mut eng = Engine::new();
        eng.app = App::open_file(path.to_str().unwrap());
        eng.app.explorer.entries.clear();
        eng.app.explorer.open = false;
        eng.app.mode = Mode::Editor;
        eng.ensure_project_tree();
        assert!(matches!(eng.app.mode, Mode::Editor));
        assert!(eng.app.explorer.open);
        assert!(
            !eng.app.explorer.entries.is_empty(),
            "ensure_project_tree should list files"
        );
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(
            !c.explorer.entries.is_empty(),
            "scene must paint explorer entries even in Normal"
        );
    }

    #[test]
    fn ctrl_comma_opens_settings_scene() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::CONTROL));
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(c.settings.open, "Ctrl+, must open settings");
        assert!(matches!(eng.app.mode, Mode::Settings));
        assert!(!c.settings.tabs.is_empty());
    }

    #[test]
    fn settings_theme_apply_updates_theme_scene() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app.open_settings();
        eng.app.settings.next_page(); // About → Setting
        assert_eq!(
            eng.app.settings.page,
            suisei_core::settings::SettingsPage::Setting
        );
        let theme_row = eng
            .app
            .settings
            .setting_rows()
            .iter()
            .position(|row| matches!(row, suisei_core::settings::SettingRow::Theme(_)))
            .expect("theme row") as u32;
        eng.settings_activate(theme_row);
        let theme_name = eng.last_diff.chrome.as_ref().unwrap().theme.name.clone();
        assert!(!theme_name.is_empty());
        assert_eq!(eng.app.theme.name, theme_name);
        assert!(eng.last_diff.chrome.as_ref().unwrap().settings.open);
    }

    /// The property the editor must hold: with it focused, **no** key can put
    /// the core into a vim state. Fires every printable ASCII plus the
    /// non-character keys and asserts the mode never leaves Normal/Insert, no
    /// vim pending state accumulates, and the leader/which-key never opens.
    ///
    /// This is what catches the regression class — reintroducing the
    /// `explorer.open` gate makes it fail on `/` (`Mode::Search`). The `return`
    /// seal in `dispatch_key` is the belt to this test's braces: today the two
    /// GUI tables happen to cover every bare key, so the seal changes no
    /// behaviour; it exists so that adding a key to `handle_normal`, or
    /// dropping one from the tables, cannot silently reopen the path.
    #[test]
    fn no_bare_key_can_reach_the_vim_machine() {
        let mut eng = eng_with_text("alpha beta\ngamma\n");
        eng.app.explorer.open = true; // the state that used to open the hole
        let mut keys: Vec<KeyEvent> = (0x20u8..0x7f).map(|b| KeyEvent::char(b as char)).collect();
        for code in [
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Insert,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
        ] {
            keys.push(KeyEvent::new(code, KeyModifiers::NONE));
        }
        for ev in keys {
            eng.dispatch_key(ev);
            assert!(
                matches!(eng.app.mode, Mode::Editor),
                "{:?} left the editor in {:?}",
                ev.code,
                eng.app.mode
            );
        }
    }

    /// Tab used to land in `handle_normal`, where it is the jumplist-forward
    /// command — it moved the caret somewhere else instead of indenting.
    #[test]
    fn tab_indents_instead_of_jumping() {
        let mut eng = eng_with_text("fn main() {}");
        eng.dispatch_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(eng.app.buffer.line(0), "fn main() {}    ");
    }

    #[test]
    fn page_keys_move_a_screenful_through_the_selection_model() {
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app.buffer = suisei_core::buffer::Buffer::from_string(&text);
        eng.app.sync_sel_to_cursor();
        eng.dispatch_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        let down = eng.app.buffer.cursor().row;
        assert!(down > 1, "PageDown must travel a screenful, got row {down}");
        assert_eq!(
            eng.app.sel.primary().head.row,
            down,
            "the Selection model must be the one that moved"
        );
        eng.dispatch_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(eng.app.buffer.cursor().row, 0, "PageUp returns to the top");
    }

    /// `explorer.open` means the docked Project navigator has entries — every
    /// project/file open sets it (`suisei_engine_open_path`), explicitly
    /// *without* taking keyboard focus. Gating the edit path on it therefore
    /// broke typing for the whole session the moment a project was opened:
    /// every key fell through to the vim command machine instead.
    #[test]
    fn typing_works_with_the_project_navigator_docked() {
        let mut eng = eng_with_text("fn main() {}");
        eng.app.explorer.open = true; // docked, not keyboard-focused
        eng.dispatch_key(KeyEvent::char('z'));
        assert!(
            eng.app.buffer.line(0).ends_with('z'),
            "a bare key must type, not run a vim command: {:?}",
            eng.app.buffer.line(0)
        );
    }

    /// Same gate, navigation side. A bare arrow is a poor probe — the vim
    /// machine moves the caret too — so use Alt+Left, which is word-left only
    /// on the GUI path.
    #[test]
    fn word_motion_works_with_the_project_navigator_docked() {
        let mut eng = eng_with_text("alpha beta");
        eng.app.explorer.open = true;
        eng.dispatch_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(
            eng.app.buffer.cursor().col,
            6,
            "Alt+Left must jump to the start of `beta` via the Selection model"
        );
    }

    /// A new tab used to be given the bare relative path `"Untitled"`, which
    /// saves against the process working directory — `/` for an `.app` launched
    /// from Finder. Anchoring it in the project is the difference between
    /// "Save" landing in the project and landing at the filesystem root.
    /// A blank tab must carry NO path. The relative `"Untitled"` saved against
    /// the process CWD (`/` under Finder), and anchoring it at the project root
    /// was worse: a session rooted at `/` gave `/Untitled.txt` on a read-only
    /// filesystem, so Save failed outright. No path means the face asks.
    #[test]
    fn a_blank_tab_has_no_path_to_save_to() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app.explorer.cwd = std::path::PathBuf::from("/");
        eng.open_blank_tab();
        assert!(
            eng.app.filename.is_none(),
            "a never-saved buffer must not claim a location: {:?}",
            eng.app.filename
        );
    }

    /// Autocomplete never opened from typing: the only trigger was a `Ctrl+A`
    /// chord, because the typing trigger lived in the vim insert handler the
    /// GUI never reached.
    #[test]
    fn typing_an_identifier_opens_the_completion_popup() {
        let mut eng = eng_with_text("");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_completion.rs"));
        assert!(!eng.app.completions.active);
        for ch in "st".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        assert!(
            eng.app.completions.active,
            "typing an identifier prefix must open the popup"
        );
    }

    #[test]
    fn punctuation_dismisses_the_popup() {
        let mut eng = eng_with_text("");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_completion.rs"));
        for ch in "st".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        assert!(eng.app.completions.active);
        eng.dispatch_key(KeyEvent::char('('));
        assert!(
            !eng.app.completions.active,
            "a non-identifier char must close it"
        );
    }

    /// And it must be acceptable — the popup used to be un-confirmable.
    #[test]
    fn tab_accepts_the_selected_suggestion() {
        let mut eng = eng_with_text("");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_completion.rs"));
        for ch in "st".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        let want = eng
            .app
            .completions
            .selected_suggestion()
            .map(|s| s.insert_text.clone())
            .expect("a suggestion");
        eng.dispatch_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(eng.app.buffer.line(0), want, "the prefix must be replaced");
        assert!(!eng.app.completions.active, "accepting closes the popup");
    }

    /// Tab must still indent when nothing is open.
    #[test]
    fn tab_still_indents_with_no_popup() {
        let mut eng = eng_with_text("x");
        eng.dispatch_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(eng.app.buffer.line(0), "x    ");
    }

    /// The tokenizer classifies fourteen kinds and the theme has a colour for
    /// each, but the face only ever painted the first six — macros, namespaces,
    /// properties, constants, operators and punctuation came out as body text.
    /// Guards that the kinds past `function` still reach the scene.
    #[test]
    fn syntax_kinds_past_function_reach_the_face() {
        let dir = std::env::temp_dir().join("suisei_kind_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("kinds.rs");
        std::fs::write(
            &path,
            "#[derive(Clone)]\nstruct S { field: u32 }\nfn f() { println!(\"x\"); }\n",
        )
        .unwrap();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(path.to_str().unwrap());
        eng.sync_viewport_public();
        eng.recompose();
        eng.flush_syntax();
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        let kinds: std::collections::HashSet<u8> = c
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.kind))
            .collect();
        assert!(
            kinds.iter().any(|k| (7..=14).contains(k)),
            "no span kind past `function` reached the scene: {kinds:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// The tick is the GUI's only pump for the language services — without
    // ── Daemon status reporting ──────────────────────────────────────────────

    /// A buffer put back to its on-disk text used to stay marked dirty for the
    /// rest of the session — `modified` only ever went up. Users read that as
    /// "dirty without being edited", because the edit that latched it was
    /// something they never saw land (an abandoned composition, a paste of what
    /// was already there).
    #[test]
    fn the_tick_clears_a_dirty_flag_that_should_not_be_up() {
        let mut eng = eng_with_text("hello");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_recheck.rs"));
        eng.app.mark_clean();

        // An edit and its exact reversal, without going through undo — the
        // latch has no way to know the text came back.
        eng.app.push_undo();
        eng.app.buffer.insert_char('!');
        eng.app.buffer.backspace();
        assert!(
            eng.app.modified,
            "the latch is up and nothing has corrected it"
        );

        for _ in 0..DIRTY_RECHECK_TICKS {
            eng.tick(50);
        }
        assert!(
            !eng.app.modified,
            "the tick must re-derive it from the text"
        );
    }

    // ── DIRTY FLAG regressions ─────────────────────────────────────────
    /// Moving the caret changes nothing on disk, so it must never dirty.
    #[test]
    fn caret_move_does_not_dirty() {
        let mut eng = eng_with_text("hello world");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_dirty_caret.rs"));
        eng.app.mark_clean();
        eng.dispatch_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        eng.dispatch_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        eng.dispatch_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(!eng.app.modified, "caret move must NOT dirty");
    }

    /// Undo back to the saved text clears dirty. Also exercises the
    /// `mark_clean` edit-run reset: no caret move is needed to make the first
    /// keystroke latch.
    #[test]
    fn undo_back_to_saved_clears_dirty() {
        let mut eng = eng_with_text("hello");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_dirty_undo.rs"));
        eng.app.mark_clean();
        eng.dispatch_key(KeyEvent::char('x'));
        eng.dispatch_key(KeyEvent::char('y'));
        assert!(eng.app.modified, "typing dirties");
        eng.app.undo();
        assert_eq!(eng.app.buffer.text(), "hello", "undo restores saved text");
        assert!(!eng.app.modified, "undo back to saved must be CLEAN");
    }

    /// A file loaded with "alpha" as its undo baseline: typing then undoing all
    /// the way returns to the on-disk text and clears dirty.
    #[test]
    fn undo_all_to_load_baseline_clears_dirty() {
        let mut eng = eng_with_text("alpha");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_dirty_undo2.rs"));
        eng.app.undo_stack = suisei_core::undo::UndoStack::new();
        eng.app.undo_stack.push(eng.app.buffer.snapshot());
        eng.app.mark_clean();
        for ch in " beta".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        assert!(eng.app.modified);
        for _ in 0..10 {
            eng.app.undo();
        }
        assert_eq!(
            eng.app.buffer.text(),
            "alpha",
            "undo-all restores saved text"
        );
        assert!(!eng.app.modified, "undo-all back to saved must be CLEAN");
    }

    /// `mark_clean` (a save) ends the insert run, so the next keystroke — with
    /// no caret move between — re-latches dirty instead of coalescing into the
    /// pre-save run and looking saved while it is edited.
    #[test]
    fn typing_immediately_after_save_redirties() {
        let mut eng = eng_with_text("hello");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_dirty_postsave.rs"));
        eng.app.mark_clean();
        eng.dispatch_key(KeyEvent::char('z'));
        assert!(eng.app.modified, "typing after save must mark dirty");
    }

    /// A no-op edit (backspace at the start of the file) may latch dirty
    /// without moving the buffer version — right after undo/redo the version
    /// gate would then skip forever. The pending-recheck flag must still clear.
    #[test]
    fn no_op_edit_does_not_leave_file_dirty() {
        let mut eng = eng_with_text("hello");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_dirty_noop.rs"));
        eng.app.mark_clean();
        for _ in 0..8 {
            eng.dispatch_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        }
        // Backspace at column 0 of row 0 deletes nothing.
        eng.dispatch_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        for _ in 0..DIRTY_RECHECK_TICKS {
            eng.tick(50);
        }
        assert_eq!(eng.app.buffer.text(), "hello");
        assert!(
            !eng.app.modified,
            "a no-op edit must not leave the file dirty"
        );
    }

    /// An external write reaches the screen without anyone asking.
    ///
    /// The claim under "live appear": edit the open file from outside — an
    /// agent, a formatter, another editor — and the pane shows it. Driven
    /// through `tick`, not by calling the check directly, because the tick is
    /// what the app actually runs and the interval is part of the promise.
    #[test]
    fn an_external_write_appears_without_being_asked() {
        let dir = std::env::temp_dir().join(format!("suisei_live_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("live.txt");
        std::fs::write(&f, "before\n").unwrap();
        let mut eng = Engine::new();
        eng.app = App::open_file(f.to_str().unwrap());
        eng.recompose();
        assert_eq!(eng.app.buffer.text(), "before\n");

        // Something else writes the file. `mtime` has to actually differ, and
        // a filesystem with coarse timestamps would otherwise make this pass
        // or fail on timing rather than on behaviour.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&f, "after\n").unwrap();

        for _ in 0..=EXTERNAL_FILE_CHECK_TICKS {
            eng.tick(8);
        }

        assert_eq!(
            eng.app.buffer.text(),
            "after\n",
            "the pane did not pick up a write it did not make"
        );
        assert!(!eng.app.modified, "a reloaded file is not a dirty one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A reload says WHERE it happened, not just that it happened.
    ///
    /// The face cannot work this out afterwards — by the time it draws, the
    /// old text is gone — so the marks are recorded on the way through and
    /// carried in the sign byte's spare bit.
    #[test]
    fn a_live_reload_marks_the_rows_it_replaced() {
        let dir = std::env::temp_dir().join(format!("suisei_live_marks_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("m.txt");
        std::fs::write(&f, "one\ntwo\nthree\nfour\n").unwrap();
        let mut eng = Engine::new();
        eng.app = App::open_file(f.to_str().unwrap());
        eng.recompose();

        std::thread::sleep(std::time::Duration::from_millis(20));
        // Only the middle moves; the first and last lines are untouched.
        std::fs::write(&f, "one\nTWO\nTHREE\nfour\n").unwrap();
        for _ in 0..=EXTERNAL_FILE_CHECK_TICKS {
            eng.tick(8);
        }

        let marked = &eng.app.live_rows;
        assert!(
            marked.contains_key(&1) && marked.contains_key(&2),
            "changed rows: {marked:?}"
        );
        assert!(
            !marked.contains_key(&0) && !marked.contains_key(&3),
            "the common prefix and suffix are not the change: {marked:?}"
        );
        // Same count in, same count out: the lines were replaced, not added.
        assert_eq!(
            marked.get(&1).copied(),
            Some(suisei_core::LiveKind::Changed),
            "a same-length replacement is not an addition"
        );
    }

    /// The marks are a flash. They expire on their own, or a row stays lit for
    /// the rest of the session and the sign byte keeps a bit nothing uses.
    #[test]
    fn live_marks_expire() {
        let dir = std::env::temp_dir().join(format!("suisei_live_exp_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("e.txt");
        std::fs::write(&f, "a\n").unwrap();
        let mut app = App::open_file(f.to_str().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&f, "b\n").unwrap();
        app.check_external_change();
        assert!(!app.live_rows.is_empty(), "the reload marked nothing");

        app.live_marked_at = Some(
            std::time::Instant::now() - std::time::Duration::from_secs(5)
        );
        app.expire_live_marks();
        assert!(app.live_rows.is_empty(), "marks outlived their flash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The project tree needs a per-FILE signal, and it has to cover the tabs
    /// the row marks cannot speak for.
    #[test]
    fn a_background_reload_is_announced_by_path() {
        let dir = std::env::temp_dir().join(format!("suisei_live_path_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "a1\n").unwrap();
        std::fs::write(&b, "b1\n").unwrap();

        let mut eng = Engine::new();
        eng.app = App::open_file(a.to_str().unwrap());
        eng.app.open_new_tab(b.to_str().unwrap());
        eng.recompose();

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&a, "a2\n").unwrap();
        for _ in 0..=EXTERNAL_FILE_CHECK_TICKS {
            eng.tick(8);
        }

        assert!(
            eng.app.live_files.keys().any(|p| p.ends_with("a.txt")),
            "a background reload said nothing the tree could show: {:?}",
            eng.app.live_files
        );
        // And `live_rows` stays quiet for it — those describe the live
        // document, and a row number means nothing for a buffer off screen.
        assert!(
            eng.app.live_rows.is_empty(),
            "a background reload marked rows of the wrong document"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file open in another tab catches up too.
    ///
    /// The half that did not exist: an agent rewrites several files, and only
    /// the focused one used to notice. A split pane beside it kept showing
    /// text that was no longer on disk.
    #[test]
    fn an_unfocused_tab_picks_up_an_external_write() {
        let dir = std::env::temp_dir().join(format!("suisei_live_bg_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "a-before\n").unwrap();
        std::fs::write(&b, "b-before\n").unwrap();

        let mut eng = Engine::new();
        eng.app = App::open_file(a.to_str().unwrap());
        eng.app.open_new_tab(b.to_str().unwrap());
        // Focus is on b; a is the background tab.
        eng.recompose();

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&a, "a-after\n").unwrap();

        for _ in 0..=EXTERNAL_FILE_CHECK_TICKS {
            eng.tick(8);
        }

        let tab_a = eng
            .app
            .tabs
            .buffers
            .iter()
            .find(|t| t.filename.as_deref() == Some(a.as_path()))
            .expect("a is still open");
        assert_eq!(
            tab_a.buffer.text(),
            "a-after\n",
            "an unfocused tab is still an open document and must follow the disk"
        );
        assert!(!tab_a.modified, "a reloaded tab is not a dirty one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unsaved text in a tab nobody is looking at is the easiest thing in the
    /// editor to destroy silently, so the disk does NOT win there.
    #[test]
    fn an_unfocused_dirty_tab_keeps_its_edits() {
        let dir = std::env::temp_dir().join(format!("suisei_live_dirty_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "a-before\n").unwrap();
        std::fs::write(&b, "b-before\n").unwrap();

        let mut eng = Engine::new();
        eng.app = App::open_file(a.to_str().unwrap());
        eng.app.gui_insert_text("!");
        assert!(eng.app.modified);
        eng.app.open_new_tab(b.to_str().unwrap());
        eng.recompose();

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&a, "a-after\n").unwrap();

        for _ in 0..=EXTERNAL_FILE_CHECK_TICKS {
            eng.tick(8);
        }

        let tab_a = eng
            .app
            .tabs
            .buffers
            .iter()
            .find(|t| t.filename.as_deref() == Some(a.as_path()))
            .expect("a is still open");
        assert!(
            tab_a.buffer.text().contains('!'),
            "unsaved text in a background tab was overwritten by the disk"
        );
        assert!(tab_a.modified, "and it is still dirty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A clean file deleted on disk closes after a confirming second poll:
    /// there are no private edits to preserve, but an atomic-save swap can
    /// cause one transient metadata miss.
    #[test]
    fn deleting_a_clean_open_file_closes_it() {
        let dir = std::env::temp_dir().join(format!("suisei_del_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("gone.txt");
        std::fs::write(&f, "hello\n").unwrap();
        let mut app = suisei_core::app::App::open_file(f.to_str().unwrap());
        app.check_external_change();
        assert!(!app.file_deleted, "present file is not flagged");
        std::fs::remove_file(&f).unwrap();
        app.check_external_change();
        assert!(
            app.file_deleted,
            "first miss marks but does not race an atomic save"
        );
        app.check_external_change();
        assert!(app.filename.is_none(), "deleted clean document was closed");
        assert_eq!(app.buffer.text(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unsaved text is the only surviving copy, so a dirty vanished file stays
    /// open with a deleted marker and Save can recreate it.
    #[test]
    fn deleting_a_dirty_open_file_marks_it_for_restore() {
        let dir = std::env::temp_dir().join(format!("suisei_dirty_del_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("gone.txt");
        std::fs::write(&f, "hello\n").unwrap();
        let mut app = suisei_core::app::App::open_file(f.to_str().unwrap());
        app.gui_insert_text("!");
        std::fs::remove_file(&f).unwrap();
        app.check_external_change();
        assert!(app.file_deleted, "dirty deletion is visibly flagged");
        assert!(app.modified, "unsaved text remains dirty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn idle_tick_surfaces_a_dirty_file_deleted_on_disk() {
        let dir =
            std::env::temp_dir().join(format!("suisei_tick_dirty_del_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("gone.txt");
        std::fs::write(&f, "hello\n").unwrap();
        let mut eng = Engine::new();
        eng.app = App::open_file(f.to_str().unwrap());
        eng.recompose();
        eng.app.gui_insert_text("!");
        std::fs::remove_file(&f).unwrap();
        let before = eng.frame_gen;

        for _ in 0..EXTERNAL_FILE_CHECK_TICKS {
            eng.tick(50);
        }

        assert!(eng.app.file_deleted);
        assert!(eng.app.modified);
        assert!(eng.frame_gen > before, "warning must reach the face");
        assert!(eng.last_diff.chrome.as_ref().unwrap().tabs[0].deleted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn idle_tick_closes_a_clean_file_after_two_missing_polls() {
        let dir =
            std::env::temp_dir().join(format!("suisei_tick_clean_del_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("gone.txt");
        std::fs::write(&f, "hello\n").unwrap();
        let mut eng = Engine::new();
        eng.app = App::open_file(f.to_str().unwrap());
        eng.recompose();
        std::fs::remove_file(&f).unwrap();

        for _ in 0..(EXTERNAL_FILE_CHECK_TICKS * 2) {
            eng.tick(50);
        }

        assert!(eng.app.filename.is_none());
        assert_eq!(eng.app.buffer.text(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The REAL path: `App::open_file` (not a hand-set baseline). Edit, undo
    /// all — must clear dirty. Regression for `open_file` leaving the App's
    /// `saved_hash` at the empty-hash default while only the tab got the real
    /// one, so undo re-derived dirty against the wrong hash forever.
    #[test]
    fn open_file_edit_undo_all_clears_dirty() {
        let dir = std::env::temp_dir().join("suisei_openfile_dirty");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("doc.txt");
        std::fs::write(&f, "line1\nline2\nline3\n").unwrap();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(f.to_str().unwrap());
        eng.recompose();
        assert!(!eng.app.modified, "opens clean");
        eng.app.buffer.cursor = suisei_core::buffer::Position::new(1, 2);
        eng.app.sync_sel_to_cursor();
        for ch in "XYZ".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        eng.dispatch_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(eng.app.modified, "edit dirties");
        for _ in 0..10 {
            eng.app.undo();
        }
        assert_eq!(eng.app.buffer.text(), "line1\nline2\nline3\n");
        assert!(
            !eng.app.modified,
            "undo-all clears dirty via the real open path"
        );
    }

    /// The exact live repro: click mid-line, type a run, press Enter (splitting
    /// the line), then undo everything. Must restore the byte-identical file and
    /// clear dirty — the earlier tests never inserted a newline.
    #[test]
    fn undo_all_after_midline_insert_and_newline_clears_dirty() {
        let mut eng = eng_with_text("line1\nline2\nline3");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_dirty_nl.rs"));
        eng.app.undo_stack = suisei_core::undo::UndoStack::new();
        eng.app.undo_stack.push(eng.app.buffer.snapshot());
        eng.app.mark_clean();
        eng.app.buffer.cursor = suisei_core::buffer::Position::new(1, 2);
        eng.app.sync_sel_to_cursor();
        for ch in "XYZ".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        eng.dispatch_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(eng.app.modified, "edit dirties");
        assert_ne!(eng.app.buffer.text(), "line1\nline2\nline3");
        for _ in 0..10 {
            eng.app.undo();
        }
        assert_eq!(
            eng.app.buffer.text(),
            "line1\nline2\nline3",
            "undo-all must restore the byte-identical file"
        );
        assert!(!eng.app.modified, "and clear dirty");
    }

    /// The correction must not undo itself: a genuinely edited buffer stays
    /// dirty however many times the tick looks at it.
    #[test]
    fn the_tick_leaves_a_genuinely_edited_buffer_dirty() {
        let mut eng = eng_with_text("hello");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_recheck2.rs"));
        eng.app.mark_clean();
        eng.app.push_undo();
        eng.app.buffer.insert_char('!');

        for _ in 0..(DIRTY_RECHECK_TICKS * 3) {
            eng.tick(50);
        }
        assert!(eng.app.modified);
    }

    /// The menu bar showed `LSP none · DAP none · Project none` for every
    /// session because nothing ever built this. Each field must follow real
    /// `App` state.
    #[test]
    fn daemon_status_follows_the_language_server() {
        let mut eng = eng_with_text("fn main() {}");
        assert_eq!(eng.daemon_status().lsp_state, 0, "no server → none");
        assert_eq!(eng.daemon_status().lsp_sessions, 0);

        eng.app.lsp.server_running = true;
        let s = eng.daemon_status();
        assert_eq!(s.lsp_state, 3, "handshaked and idle → ready");
        assert_eq!(s.lsp_sessions, 1);

        eng.app.lsp.error = Some("boom".into());
        assert_eq!(eng.daemon_status().lsp_state, 4, "a hard failure → error");
    }

    /// "Indexing" is the state a user actually waits on, and it was
    /// unobservable: the client never asked for `$/progress`.
    #[test]
    fn daemon_status_reports_indexing_while_progress_is_open() {
        let mut eng = eng_with_text("fn main() {}");
        eng.app.lsp.server_running = true;
        assert_eq!(eng.daemon_status().lsp_state, 3);

        eng.app
            .lsp
            .set_progress_open_for_test("rustAnalyzer/Indexing", true);
        assert_eq!(eng.daemon_status().lsp_state, 2, "open progress → indexing");

        eng.app
            .lsp
            .set_progress_open_for_test("rustAnalyzer/Indexing", false);
        assert_eq!(eng.daemon_status().lsp_state, 3, "closed progress → ready");
    }

    #[test]
    fn daemon_status_follows_the_debugger() {
        use suisei_core::dap::DapState;
        let mut eng = eng_with_text("x");
        assert_eq!(eng.daemon_status().dap_state, 0);
        eng.app.dap.state = DapState::Running;
        assert_eq!(eng.daemon_status().dap_state, 1);
        eng.app.dap.state = DapState::Stopped;
        assert_eq!(
            eng.daemon_status().dap_state,
            2,
            "stopped at a breakpoint → paused"
        );
    }

    /// The navigator root is what the user opened; the walked-up root of the
    /// current file is the fallback. Neither may degrade into the process cwd,
    /// which for a launched `.app` is `/`.
    #[test]
    fn daemon_status_names_the_open_project_and_never_guesses() {
        let mut eng = Engine::new();
        assert!(
            eng.daemon_status().project.is_empty(),
            "an empty editor must report no project, not `/`"
        );

        let dir = std::env::temp_dir().join("suisei_daemon_root");
        let _ = std::fs::create_dir_all(&dir);
        eng.app.explorer.cwd = dir.clone();
        eng.app
            .explorer
            .entries
            .push(suisei_core::explorer::ExplorerEntry {
                name: "a.txt".into(),
                path: dir.join("a.txt"),
                is_dir: false,
            });
        assert_eq!(eng.daemon_status().project, dir.display().to_string());
    }

    /// this call the LSP never finishes its handshake and no result is ever
    /// applied. A queued hover answer standing in for "a reply arrived".
    #[test]
    fn tick_pumps_the_language_services() {
        let mut eng = eng_with_text("hello");
        eng.app.lsp.pending_hover = Some("fn main()".to_string());
        assert!(eng.app.hover_text.is_none());
        eng.tick(50);
        assert_eq!(
            eng.app.hover_text.as_deref(),
            Some("fn main()"),
            "tick must drain the LSP and apply its results"
        );
    }

    /// The GUI edit path never notified the server itself, so without this the
    /// document the LSP answers against is whatever it saw at didOpen.
    #[test]
    fn tick_syncs_the_document_to_the_language_server() {
        let mut eng = eng_with_text("fn main() {}");
        eng.app.filename = Some(std::path::PathBuf::from("/tmp/suisei_tick_sync.rs"));
        eng.app.lsp.server_running = true; // stands in for a live server
        eng.dispatch_key(KeyEvent::char('x'));
        assert!(
            !eng.app.lsp_document_synced(),
            "an edit must leave the server's copy stale"
        );
        // The tick also runs the shadow WAL, which writes to the developer's
        // real `~/.suisei/journal` and would then surface in the app's recovery
        // sheet. The sync is version-gated, not `modified`-gated, so clearing
        // the dirty flag keeps this test out of that directory.
        eng.app.modified = false;
        for _ in 0..LSP_SYNC_TICKS {
            eng.tick(50);
        }
        assert!(
            eng.app.lsp_document_synced(),
            "the tick must send the post-edit didChange"
        );
    }

    #[test]
    fn idle_tick_does_not_bump_frame_gen() {
        let mut eng = eng_with_text("hello");
        let before = eng.frame_gen;
        let after = eng.tick(50);
        assert_eq!(after, before, "idle tick must not recompose");
        assert_eq!(eng.frame_gen, before);
    }

    #[test]
    fn rust_file_gets_syntax_spans() {
        let dir = std::env::temp_dir().join("suisei_syntax_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sample.rs");
        std::fs::write(
            &path,
            "fn main() {\n    let x = 42;\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(path.to_str().unwrap());
        eng.sync_viewport_public();
        eng.recompose();
        eng.flush_syntax();
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(!c.welcome);
        assert!(!c.lines.is_empty());
        let has_spans = c.lines.iter().any(|l| !l.spans.is_empty());
        assert!(
            has_spans,
            "Rust source should produce highlight spans (got {} lines, all empty spans)",
            c.lines.len()
        );
        // Keyword span kind = 1 on a line with `fn` or `let`
        let has_keyword = c
            .lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| s.kind == 1);
        assert!(has_keyword, "expected at least one keyword span");
    }

    #[test]
    fn ctrl_g_opens_scm_scene() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let c = eng.last_diff.chrome.as_ref().unwrap();
        // May open empty if not in a git repo, but mode should be SourceControl
        assert!(
            matches!(eng.app.mode, Mode::SourceControl) || c.scm.open,
            "Ctrl+G should enter SCM"
        );
    }

    #[test]
    fn scm_mouse_selection_uses_flattened_row_and_clamps() {
        use suisei_core::scm::{ScmEntry, ScmFocus, ScmStatus};

        let mut eng = Engine::new();
        eng.app.scm.staged = vec![ScmEntry {
            path: "staged.rs".into(),
            status: ScmStatus::Modified,
            staged: true,
        }];
        eng.app.scm.changes = vec![ScmEntry {
            path: "changed.rs".into(),
            status: ScmStatus::Modified,
            staged: false,
        }];

        eng.scm_select(1);
        assert_eq!(eng.app.scm.selected, 1);
        assert_eq!(eng.app.scm.focus, ScmFocus::Changes);

        eng.scm_select(999);
        assert_eq!(eng.app.scm.selected, 1, "face row must clamp safely");
    }

    #[test]
    fn ctrl_shift_g_opens_git_workbench() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert!(
            matches!(eng.app.mode, Mode::GitWorkbench) || eng.app.git_wb.open,
            "Ctrl+Shift+G should open git workbench"
        );
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(c.git_wb.open || matches!(eng.app.mode, Mode::GitWorkbench));
    }

    #[test]
    fn scroll_does_not_move_caret() {
        let mut eng = eng_with_text(&"line\n".repeat(100));
        eng.app.buffer.cursor = Position::new(50, 1);
        eng.app.scroll = 40;
        eng.recompose();
        eng.scroll_by(5);
        assert_eq!(eng.app.buffer.cursor().row, 50);
        assert_eq!(eng.app.scroll, 45);
    }

    #[test]
    fn explorer_toggle_preserves_cursor_and_scroll() {
        let mut eng = eng_with_text(&"line\n".repeat(80));
        eng.app.buffer.cursor = Position::new(40, 2);
        eng.app.scroll = 30;
        eng.recompose();
        eng.dispatch_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert!(eng.app.explorer.open);
        assert_eq!(
            eng.app.buffer.cursor().row,
            40,
            "cursor must not jump to line 1"
        );
        assert_eq!(eng.app.buffer.cursor().col, 2);
        // Scroll should still show the caret row
        let vis = eng.app.grid_rows().max(1) as usize;
        let row = eng.app.buffer.cursor().row;
        assert!(
            row >= eng.app.scroll && row < eng.app.scroll + vis,
            "scroll={} should keep row {} visible (vis={})",
            eng.app.scroll,
            row,
            vis
        );
    }

    #[test]
    fn markdown_gets_fallback_syntax_spans() {
        let dir = std::env::temp_dir().join("suisei_md_syntax");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("note.md");
        std::fs::write(
            &path,
            "# Title\n\n```rust\nfn x() {}\n```\n\n// not a comment in md\n",
        )
        .unwrap();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(path.to_str().unwrap());
        eng.sync_viewport_public();
        eng.recompose();
        eng.flush_syntax();
        eng.recompose();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        let has_spans = c.lines.iter().any(|l| !l.spans.is_empty());
        assert!(
            has_spans,
            "markdown should get highlight_line fallback spans"
        );
    }

    #[test]
    fn scroll_by_zero_fractional_safe() {
        let mut eng = eng_with_text(&"line\n".repeat(30));
        eng.scroll_by(0);
        // scroll_by(0) still recomposes today — just ensure no panic / clamp ok
        assert!(eng.app.scroll <= 30);
    }

    #[test]
    fn scroll_to_sets_absolute_first_line() {
        let mut eng = eng_with_text(&"row\n".repeat(100));
        eng.resize(800.0, 400.0, 18.0, 9.0, 2.0);
        eng.scroll_to(40, 0);
        assert_eq!(eng.app.scroll, 40);
        eng.scroll_to(0, 0);
        assert_eq!(eng.app.scroll, 0);
        // hscroll only when wrap is off — and only as far as the text goes.
        // These rows are 3 columns wide in an ~88-column viewport, so there is
        // nothing to pan to: the clamp pins it at the single column of slack.
        eng.app.wrap_lines = false;
        eng.scroll_to(10, 15);
        assert_eq!(eng.app.scroll, 10);
        assert_eq!(
            eng.app.hscroll, 1,
            "no content to the right of a 3-column row"
        );

        // Give it a line worth panning across and the request goes through.
        eng.app.buffer = suisei_core::buffer::Buffer::from_string(&format!(
            "{}\n{}",
            "x".repeat(400),
            "row\n".repeat(99)
        ));
        eng.app.content_cols(); // re-measure now that a wide line is on screen
        eng.scroll_to(0, 15);
        assert_eq!(eng.app.hscroll, 15);

        eng.app.wrap_lines = true;
        eng.scroll_to(10, 99);
        assert_eq!(eng.app.hscroll, 0);
    }

    #[test]
    fn palette_query_typing_updates_scene() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::SUPER));
        assert!(eng.last_diff.chrome.as_ref().unwrap().palette.open);
        for ch in "ma".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert_eq!(
            c.palette.query, "ma",
            "palette query must repaint while typing (light path regression)"
        );
    }

    #[test]
    fn search_input_typing_updates_scene() {
        let mut eng = eng_with_text("alpha beta gamma");
        eng.find_open(); // GUI trigger (Cmd+F), not the vim '/'
        for ch in "bet".chars() {
            eng.dispatch_key(KeyEvent::char(ch));
        }
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(c.search.open);
        assert_eq!(
            c.search.input, "bet",
            "search bar must repaint while typing"
        );
    }

    #[test]
    fn find_step_cycles_the_live_native_query() {
        let mut eng = eng_with_text("한글 alpha 한글");
        eng.find_open();
        eng.find_set_input("한글");
        assert_eq!(eng.app.buffer.cursor(), Position::new(0, 0));

        eng.find_step(true);
        assert_eq!(eng.app.buffer.cursor(), Position::new(0, 9));
        let search = &eng.last_diff.chrome.as_ref().unwrap().search;
        assert_eq!(search.input, "한글");
        assert_eq!(search.match_index, 1);
    }

    #[test]
    fn native_find_accept_closes_without_editing_the_document() {
        let mut eng = eng_with_text("suisei alpha suisei");
        eng.find_open();
        eng.find_set_input("suisei");
        let before = eng.app.buffer.text();

        eng.find_accept();

        assert_eq!(eng.app.mode, Mode::Editor);
        assert_eq!(eng.app.buffer.text(), before);
        assert_eq!(eng.app.search.pattern.as_deref(), Some("suisei"));
        assert!(!eng.last_diff.chrome.as_ref().unwrap().search.open);
    }

    #[test]
    fn native_find_cancel_restores_origin_without_editing_the_document() {
        let mut eng = eng_with_text("suisei alpha suisei");
        let origin = eng.app.buffer.cursor();
        eng.find_open();
        eng.find_set_input("suisei");
        let before = eng.app.buffer.text();

        eng.find_cancel();

        assert_eq!(eng.app.mode, Mode::Editor);
        assert_eq!(eng.app.buffer.text(), before);
        assert_eq!(eng.app.buffer.cursor(), origin);
        assert!(!eng.last_diff.chrome.as_ref().unwrap().search.open);
    }

    #[test]
    fn explorer_keyboard_selection_updates_scene() {
        let dir = std::env::temp_dir().join("suisei_expl_kb_nav");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app.explorer.cwd = dir;
        eng.app.explorer.refresh();
        eng.app.explorer.open = true;
        eng.app.explorer.selected = 0;
        eng.app.mode = Mode::Explorer;
        eng.recompose();
        let sel_before = eng
            .last_diff
            .chrome
            .as_ref()
            .unwrap()
            .explorer
            .entries
            .iter()
            .position(|e| e.selected);
        eng.dispatch_key(KeyEvent::char('j'));
        let sel_after = eng
            .last_diff
            .chrome
            .as_ref()
            .unwrap()
            .explorer
            .entries
            .iter()
            .position(|e| e.selected);
        assert_ne!(
            sel_before, sel_after,
            "explorer j/k must repaint selection (light path regression)"
        );
    }

    #[test]
    fn compose_band_extends_above_scroll_but_scroll_stays_true_top() {
        let mut eng = eng_with_text(&"row\n".repeat(400));
        eng.resize(800.0, 400.0, 18.0, 9.0, 2.0);
        eng.scroll_to(200, 0);
        assert_eq!(eng.app.scroll, 200, "core scroll must stay the visible top");
        eng.recompose(); // lines are built on full compose, not the scroll hot path
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert_eq!(c.scroll, 200);
        let first = c.lines.first().map(|l| l.line_no).unwrap_or(0);
        let last = c.lines.last().map(|l| l.line_no).unwrap_or(0);
        assert!(
            first <= 200 - 40,
            "band must include overscan above scroll (first={first})"
        );
        assert!(
            last >= 200 + eng.app.grid_rows() as u32,
            "band must still cover the viewport below (last={last})"
        );
        // Caret move after absolute scroll must not re-anchor the window upward.
        // Down at the last line is a no-op → scroll must stay put.
        eng.dispatch_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(
            eng.app.scroll <= 201,
            "caret move must not yank scroll (got {})",
            eng.app.scroll
        );
    }

    #[test]
    fn scroll_sync_tracks_position_without_recompose() {
        let mut eng = eng_with_text(&"row\n".repeat(300));
        eng.resize(800.0, 400.0, 18.0, 9.0, 2.0);
        let gen_before = eng.frame_gen;
        eng.scroll_sync(120, 0);
        assert_eq!(eng.app.scroll, 120, "sync must move core scroll");
        assert_eq!(
            eng.frame_gen, gen_before,
            "sync must not recompose (hot covered-scroll path)"
        );
        // Caret op after sync must not yank the viewport backwards.
        eng.app.buffer.cursor = Position::new(125, 0);
        eng.app.sync_sel_to_cursor();
        eng.dispatch_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(
            eng.app.scroll >= 118 && eng.app.scroll <= 127,
            "scroll {} should stay near the synced viewport",
            eng.app.scroll
        );
    }

    #[test]
    fn scroll_by_frac_accumulates_into_integer_scroll() {
        let mut eng = eng_with_text(&"line\n".repeat(80));
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        // Caret at end may already have scrolled — pin to top for a clean frac test.
        eng.app.scroll = 0;
        eng.app.scroll_frac = 0.0;
        eng.scroll_by_frac(0.4);
        assert_eq!(eng.app.scroll, 0);
        assert!((eng.app.scroll_frac - 0.4).abs() < 0.001);
        eng.scroll_by_frac(0.7); // total 1.1 → scroll 1, frac ~0.1
        assert_eq!(eng.app.scroll, 1);
        assert!((eng.app.scroll_frac - 0.1).abs() < 0.05);
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!((c.scroll_frac - eng.app.scroll_frac).abs() < 0.001);
    }

    /// Second ⌘⇧V must CLOSE the preview even though this face never runs the
    /// TUI close animation tick (regression: `closing` kept `open` true forever
    /// and the toggle re-opened instead).
    #[test]
    fn preview_toggle_closes_without_anim_tick() {
        let mut eng = eng_with_text("# title\n\nbody\n");
        let toggle = KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::SUPER | KeyModifiers::SHIFT,
        );
        eng.dispatch_key(toggle.clone());
        assert!(eng.app.preview.open, "first toggle opens the preview");
        // GUI reality: focus clicks can drop the app back into Insert while
        // the panel is still showing — the close must not depend on the mode.
        eng.app.mode = Mode::Editor;
        eng.dispatch_key(toggle);
        assert!(
            !eng.app.preview.open,
            "second toggle must close (closing={})",
            eng.app.preview.closing
        );
        assert!(
            !eng.app.message.contains("Preview"),
            "closed preview must not leave stale status text"
        );
    }

    #[test]
    fn git_workbench_generation_ignores_unrelated_editor_recomposes() {
        let mut eng = eng_with_text("fn main() {}\n");
        let closed_generation = eng.git_wb_generation();

        eng.recompose();
        assert_eq!(
            eng.git_wb_generation(),
            closed_generation,
            "an unchanged editor frame must not invalidate Source Control"
        );

        eng.app.git_wb.open = true;
        eng.recompose();
        let open_generation = eng.git_wb_generation();
        assert_ne!(open_generation, closed_generation);

        eng.app.message = "LSP finished indexing".into();
        eng.recompose();
        assert_eq!(
            eng.git_wb_generation(),
            open_generation,
            "editor/LSP chrome must stay outside the workbench generation"
        );

        eng.app.git_wb.branch = "performance-work".into();
        eng.recompose();
        assert_ne!(eng.git_wb_generation(), open_generation);
    }
}
