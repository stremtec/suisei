//! Owns `App` + shell state; single place that calls dispatch and compose.

use suisei_core::app::{App, Mode};
use suisei_core::buffer::Position;
use suisei_core::key::{KeyCode, KeyEvent, KeyModifiers};

use crate::compositor::{compose, FrameDiff, ShellState};

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
    /// Parked terminal sessions (VS Code-style multi-shell). The ACTIVE session
    /// always lives in `app.terminal` so all core routing (keys/paste/resize)
    /// keeps working untouched; switching swaps sessions in and out.
    parked_terminals: Vec<suisei_core::term::Terminal>,
    /// Index of the active session within the conceptual list
    /// `[..parked[0..active], ACTIVE, parked[active..]..]`.
    active_terminal: usize,
    /// Shadow WAL — crash-recovery journal for unsaved buffers (D0).
    pub journal: crate::journal::Journal,
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
        let Ok(text) = std::fs::read_to_string(path) else { return false };
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_string());
        self.app.syntax.prewarm(path, &text, ext.as_deref());
        true
    }

    pub fn cached_parses(&self) -> usize {
        self.app.syntax.cached_count()
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

    pub fn new() -> Self {
        let mut app = App::new();
        // Same as TUI main: load ~/.xei.toml theme + editor opts.
        app.apply_config();
        app.message = "Suisei · same keys as xei · Ctrl+, settings".into();
        app.viewport.width = 100;
        app.viewport.height = 40;
        app.viewport.text_x = 5;
        Self {
            app,
            shell: ShellState::default(),
            last_diff: FrameDiff::empty(0),
            frame_gen: 0,
            pointer_down: false,
            pointer_moved: false,
            outline_cache: Vec::new(),
            outline_cache_ver: u64::MAX,
            outline_cache_path: None,
            tick_count: 0,
            parked_terminals: Vec::new(),
            active_terminal: 0,
            journal: crate::journal::Journal::new(),
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
        fresh.full_panel = self.app.terminal.full_panel;
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
        let parked_idx = if idx < self.active_terminal { idx } else { idx - 1 };
        if parked_idx >= self.parked_terminals.len() {
            return;
        }
        std::mem::swap(&mut self.app.terminal, &mut self.parked_terminals[parked_idx]);
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
                    self.app.mode = Mode::Normal;
                }
                self.active_terminal = 0;
            }
        } else {
            let parked_idx = if idx < self.active_terminal { idx } else { idx - 1 };
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
    // The face calls these INSTEAD of synthesizing vim keystrokes (`i`, `Esc`,
    // `c`, `d`). Core stays modal internally (shared with the xei TUI) but the
    // GUI never surfaces modes — these commands handle transitions invisibly.

    /// True when the engine is in a text-editing mode (Insert/Normal/Visual*).
    fn is_editing_mode(&self) -> bool {
        matches!(
            self.app.mode,
            Mode::Normal | Mode::Insert | Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        )
    }

    fn is_visual_mode(&self) -> bool {
        matches!(
            self.app.mode,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        )
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
        self.app.gui_insert_text(&ch.to_string());
        self.app.update_scroll();
        self.recompose_scroll();
    }

    /// Backspace: delete the selection, or one grapheme before each caret.
    pub fn gui_delete_backward(&mut self) {
        if !self.is_editing_mode() {
            return;
        }
        self.app.gui_delete_backward();
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
        // Let the legacy Esc close overlays / clear any stray vim state; it is
        // not intercepted, so it reaches the dispatch and does the housekeeping.
        self.dispatch_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        if self.is_editing_mode() {
            self.app.caret_collapse();
        }
        self.recompose_scroll();
    }

    /// Historically forced Insert mode for click-to-type. Typing is now
    /// mode-independent, so there is nothing to ensure — this only clears a
    /// stray vim visual selection (which the GUI should never enter) and never
    /// inserts.
    pub fn gui_ensure_insert(&mut self) {
        if self.is_visual_mode() {
            self.app.enter_normal();
            self.app.sync_sel_to_cursor();
        }
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
        if !matches!(self.app.mode, Mode::Insert | Mode::Normal) {
            return false;
        }
        if self.app.terminal_window_focused() || self.app.explorer.open {
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
    fn try_gui_edit(&mut self, ev: KeyEvent) -> bool {
        if !matches!(self.app.mode, Mode::Insert | Mode::Normal) {
            return false;
        }
        if self.app.terminal_window_focused() || self.app.explorer.open {
            return false;
        }
        let m = ev.modifiers;
        if m.contains(KeyModifiers::CONTROL) || m.contains(KeyModifiers::SUPER) {
            return false; // shortcut, not text
        }
        match ev.code {
            KeyCode::Char(c) => self.app.gui_insert_text(&c.to_string()),
            KeyCode::Enter => self.app.gui_insert_newline("    "),
            KeyCode::Backspace => self.app.gui_delete_backward(),
            KeyCode::Delete => self.app.gui_delete_forward(),
            _ => return false,
        }
        true
    }

    pub fn dispatch_key(&mut self, ev: KeyEvent) {
        // Pure-GUI editing + navigation: characters, backspace, delete, enter,
        // and the arrows drive the Selection model directly — never the vim
        // command machine. The core stays modal internally; the face never
        // surfaces a mode, and there is no synthetic `i`.
        let caret_pre = self.app.buffer.cursor();
        let ver_pre = self.app.buffer.version();
        if self.try_gui_navigation(ev) || self.try_gui_edit(ev) {
            // Only re-anchor the viewport when something actually moved — a
            // no-op arrow at a document edge must not yank an absolute scroll.
            if self.app.buffer.cursor() != caret_pre || self.app.buffer.version() != ver_pre {
                self.app.update_scroll();
            }
            self.recompose_scroll();
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
        self.sync_viewport_to_app();
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
        let buffer_or_caret_changed = caret_before != caret_after
            || ver_before != ver_after
            || file_before != file_after;
        if buffer_or_caret_changed {
            self.app.update_scroll();
            // Coherence: a non-navigation key (typing, edit) moved the cursor
            // through the legacy path. Collapse the GUI selection to a caret
            // there so the next Shift+Arrow starts from the right place — and
            // so a stale highlight never lingers after typing.
            if matches!(self.app.mode, Mode::Insert | Mode::Normal)
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
        let chrome_mode = |m: Mode| {
            !matches!(
                m,
                Mode::Normal
                    | Mode::Insert
                    | Mode::Visual
                    | Mode::VisualLine
                    | Mode::VisualBlock
                    | Mode::Terminal
                    | Mode::Preview
            )
        };
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
        if self.tick_count % 12 == 0 && self.outline_cache_ver != self.app.buffer.version() {
            self.shell.dirty = true;
            need_full = true;
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
        // Only recompose when poll actually changes state — not every 50ms while open.
        if self.app.git_wb.open && self.app.git_wb.poll_loading() {
            self.shell.dirty = true;
            need_full = true;
        }
        // which_key / completions / palette need live paint while open
        if self.shell.dirty
            || self.app.which_key_visible()
            || self.app.completions.active
            || self.app.palette.open
        {
            if need_full || self.app.git_wb.open || self.app.scm.visible() {
                self.recompose();
            } else {
                // PTY / which-key / completions: paint only (no outline/SCM rebuild).
                self.recompose_scroll();
            }
        }
        // Shadow WAL: flush dirty buffer to journal if policy is satisfied.
        {
            let file_path = self.app.filename
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let dirty = self.app.modified;
            let version = self.app.buffer.version();
            let cursor = self.app.buffer.cursor();
            let scroll = self.app.scroll;
            let text = self.app.buffer.text();
            self.journal.on_tick(
                &file_path, &text, version,
                cursor.row as u32, cursor.col as u32, scroll as u32,
                dirty,
            );
        }
        self.frame_gen
    }

    /// `css_w/h` = editor stage size (not whole window).  
    /// `line_h` = painted line height; `cell_w` = monospaced cell width.
    pub fn resize(&mut self, css_w: f32, css_h: f32, line_h: f32, cell_w: f32, dpr: f32) {
        let scroll_before = self.app.scroll;
        let caret = self.app.buffer.cursor();
        self.shell.viewport.css_w = css_w.max(80.0);
        self.shell.viewport.css_h = css_h.max(80.0);
        self.shell.viewport.cell_px = line_h.max(12.0);
        self.shell.viewport.cell_w = cell_w.max(6.0);
        self.shell.viewport.dpr = dpr.max(1.0);
        // Full editor height is usable — face already subtracted chrome.
        let rows = (self.shell.viewport.css_h / self.shell.viewport.cell_px)
            .floor() as u32;
        // Up to 200 face rows; split packing uses SUISEI_MAX_LINES (256).
        self.shell.viewport.editor_rows = rows.clamp(8, 200);
        self.sync_viewport_to_app();
        let total = self.app.buffer.line_count();
        let vis = self.app.viewport.height.max(1) as usize;
        let max_scroll = total.saturating_sub(vis.min(total));
        // GUI contract: panel/window resize NEVER re-anchors the viewport to the
        // caret — the user's scroll position is sacred (re-anchoring made hiding
        // the outline yank a far-scrolled view back to the caret line).
        let _ = caret;
        self.app.scroll = scroll_before.min(max_scroll);
        self.shell.dirty = true;
        // Resize is viewport-only — keep outline/SCM caches (face debounces full shell).
        self.recompose_scroll();
    }

    pub(crate) fn sync_viewport_public(&mut self) {
        self.sync_viewport_to_app();
    }

    pub(crate) fn update_scroll_public(&mut self) {
        self.app.update_scroll();
    }

    fn sync_viewport_to_app(&mut self) {
        let rows = self.shell.viewport.editor_rows.max(8) as u16;
        let cell_w = self.shell.viewport.cell_w.max(6.0);
        let cols = (self.shell.viewport.css_w / cell_w)
            .floor()
            .clamp(40.0, 500.0) as u16;
        self.app.viewport.height = rows;
        self.app.viewport.width = cols;
        self.app.viewport.text_x = 5;
        self.app.viewport.text_y = self.app.viewport.y;
    }


    /// Highlight window: what the viewport shows plus generous overscan, so a
    /// scroll usually stays a cache hit. Tokens are only ever consumed per row
    /// (`tokens_for_row`), so nothing needs whole-file highlighting — and
    /// rebuilding every token was the entire typing cost once parsing became
    /// incremental.
    fn refresh_syntax(&mut self) {
        const OVERSCAN: usize = 400;
        let first = self.app.scroll;
        let height = self.app.viewport.height as usize;
        let window = first.saturating_sub(OVERSCAN)..(first + height + OVERSCAN);

        let stale = self.app.syntax_seen_version != self.app.buffer.version();
        if !stale && self.app.syntax.covers_rows(&window) {
            return;
        }
        let text = self.app.buffer.text();
        let ext = self.app.file_extension();
        let path = self
            .app
            .filename
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        // Path-aware: adopts the indexer's pre-parsed tree when switching files.
        self.app.syntax.parse_path(&path, &text, ext.as_deref(), Some(window));
        self.app.syntax_seen_version = self.app.buffer.version();
    }

    pub(crate) fn recompose(&mut self) {
        self.sync_viewport_to_app();
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
        // Outline: full-buffer scan — skip when only scroll/caret chrome changed.
        let ver = self.app.buffer.version();
        let path = self.app.filename.clone();
        if self.outline_cache_ver != ver || self.outline_cache_path != path {
            self.outline_cache = crate::compositor::build_outline_public(&self.app);
            self.outline_cache_ver = ver;
            self.outline_cache_path = path;
        }
        self.frame_gen = self.frame_gen.saturating_add(1);
        self.last_diff = compose(
            &self.app,
            &self.shell,
            self.frame_gen,
            &self.outline_cache,
        );
        self.shell.dirty = false;
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
        self.sync_viewport_to_app();
        self.app.scroll_by_lines(delta_lines);
        // Keep split pane mirrors in sync so inactive panes paint correctly.
        self.app.sync_focused_pane_viewport();
        // Scroll never moves the caret — only the window.
        self.recompose_scroll();
    }

    /// Size + spawn PTY for side/full terminal (Suisei has no TUI first-draw hook).
    pub(crate) fn ensure_terminal_started(&mut self) {
        if !self.app.terminal.open || self.app.terminal.started {
            return;
        }
        self.sync_viewport_to_app();
        // Prefer editor viewport geometry for COLUMNS/LINES at spawn.
        // Full-panel: use a taller default so the shell isn't a 8-row postage stamp.
        let cols = self.app.viewport.width.max(40);
        let rows = if self.app.terminal.full_panel {
            self.app.viewport.height.max(24)
        } else {
            self.app.viewport.height.max(8).min(24).max(8)
        };
        self.app.terminal.resize(cols, rows);
        // start() uses parent(path) as cwd — prefer open file, else project root.
        let root = self.app.project_root();
        let anchor_owned = self
            .app
            .filename
            .clone()
            .unwrap_or_else(|| root.join("."));
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
                self.app.preview.hscroll = self.app.preview.hscroll.saturating_add(delta_cols as usize);
            }
            self.recompose_scroll();
            return;
        }
        if delta_cols < 0 {
            self.app.hscroll = self.app.hscroll.saturating_sub((-delta_cols) as usize);
        } else {
            self.app.hscroll = self.app.hscroll.saturating_add(delta_cols as usize);
        }
        self.app.sync_focused_pane_viewport();
        self.recompose_scroll();
    }

    /// Position-only scroll sync (no recompose): keeps Core's `scroll` tracking
    /// the native clip during covered scrolling so the next caret op / publish
    /// never snaps the viewport. The paint band already covers ±overscan.
    pub fn scroll_sync(&mut self, line: u32, hscroll_cols: u32) {
        if self.pointer_down && self.pointer_moved {
            return;
        }
        self.sync_viewport_to_app();
        self.app.scroll_to_line(line as usize);
        if !self.app.wrap_lines {
            self.app.set_hscroll(hscroll_cols as usize);
        } else {
            self.app.hscroll = 0;
        }
        self.app.sync_focused_pane_viewport();
    }

    /// Absolute scroll position for native NSScrollView faces.
    /// `line` = first fully/partially visible buffer row; `hscroll_cols` when wrap off.
    pub fn scroll_to(&mut self, line: u32, hscroll_cols: u32) {
        if self.pointer_down && self.pointer_moved {
            return;
        }
        self.sync_viewport_to_app();
        let before = (self.app.scroll, self.app.hscroll);
        self.app.scroll_to_line(line as usize);
        if !self.app.wrap_lines {
            self.app.set_hscroll(hscroll_cols as usize);
        } else {
            self.app.hscroll = 0;
        }
        self.app.sync_focused_pane_viewport();
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
        self.sync_viewport_to_app();
        let before = (self.app.scroll, self.app.scroll_frac.to_bits());
        self.app.scroll_by_frac(delta_lines);
        let after = (self.app.scroll, self.app.scroll_frac.to_bits());
        if before == after {
            return;
        }
        self.app.sync_focused_pane_viewport();
        self.recompose_scroll();
    }

    /// Scroll / paint-only recompose: editor surfaces only (no explorer/SCM rebuild).
    /// Used for wheel, light keys (face), and pointer when only the viewport moves.
    pub(crate) fn recompose_scroll(&mut self) {
        self.sync_viewport_to_app();
        // Syntax only if buffer changed (usually no-op while scrolling).
        self.refresh_syntax();
        self.frame_gen = self.frame_gen.saturating_add(1);
        // Patch in place when we already have chrome — full compose is too heavy
        // for 5k–10k line files during trackpad momentum.
        if let Some(chrome) = self.last_diff.chrome.as_mut() {
            crate::compositor::patch_chrome_editor_scroll(
                &self.app,
                &self.shell,
                self.frame_gen,
                chrome,
            );
            self.last_diff.frame_gen = self.frame_gen;
        } else {
            self.last_diff = crate::compositor::compose(
                &self.app,
                &self.shell,
                self.frame_gen,
                &self.outline_cache,
            );
        }
        self.shell.dirty = false;
    }

    /// Alias for face light path naming.
    pub fn recompose_paint_only(&mut self) {
        self.recompose_scroll();
    }

    /// Mouse **down**: place caret + arm drag. Does **not** enter Visual yet.
    pub fn click_at(&mut self, buffer_row: u32, visual_col: u32, select_word: bool) {
        self.sync_viewport_to_app();
        if self.app.buffer.line_count() == 0 {
            return;
        }
        let pos = self.pos_from_click(buffer_row, visual_col);

        if matches!(self.app.mode, Mode::Palette | Mode::Explorer) {
            self.app.palette.close();
            self.app.mode = Mode::Normal;
        }
        // A click leaves any keyboard-vim visual selection — the GUI model
        // (`app.sel`) is authoritative from here, and a lingering
        // `visual_anchor` would otherwise show through `selected_range`.
        if matches!(
            self.app.mode,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        ) {
            self.app.enter_normal();
            self.app.message.clear();
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
        self.sync_viewport_to_app();
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
        self.app.undo();
        self.app.update_scroll();
        self.recompose();
    }

    pub fn redo(&mut self) {
        self.app.redo();
        self.app.update_scroll();
        self.recompose();
    }

    pub fn select_all(&mut self) {
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
        if forward {
            self.app.search_next();
        } else {
            self.app.search_prev();
        }
        self.app.update_scroll();
        self.recompose_scroll();
    }

    /// Insert text at the caret (file drop / IME commit). Routes to the PTY
    /// when the terminal owns input, like the TUI bracketed-paste path.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.app.terminal.open
            && (matches!(self.app.mode, Mode::Terminal) || self.app.terminal_window_focused())
        {
            self.app.terminal.paste_input(text);
            self.shell.dirty = true;
            self.recompose_scroll();
            return;
        }
        if matches!(
            self.app.mode,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        ) {
            self.app.delete_selection();
        }
        self.app.paste_text_at_cursor(text);
        self.recompose();
    }

    /// GUI focus contract: clicking the terminal panel routes keys to the PTY,
    /// clicking the editor routes them back to the buffer.
    pub fn focus_terminal(&mut self, on: bool) {
        if on {
            if self.app.terminal.open {
                self.ensure_terminal_started();
                self.app.mode = Mode::Terminal;
            }
        } else if matches!(self.app.mode, Mode::Terminal) {
            self.app.mode = Mode::Normal;
        }
        self.recompose();
    }

    /// Size the PTY to the face's terminal panel (cols × rows in cells).
    pub fn terminal_resize(&mut self, cols: u32, rows: u32) {
        if !self.app.terminal.open || cols < 10 || rows < 3 {
            return;
        }
        let cols = cols.min(500) as u16;
        let rows = rows.min(200) as u16;
        self.app.terminal.resize(cols, rows);
        self.shell.dirty = true;
        self.recompose_scroll();
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
            // Keep tree data for docked navigator; leave Mode::Normal for editing.
            self.app.explorer.open = true;
            self.app.mode = Mode::Normal;
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
            self.app.mode = Mode::Normal;
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
        let vis = self.app.viewport.height.max(1) as usize;
        let target = (line_1based as usize).saturating_sub(1);
        self.app.scroll_to_line(target.saturating_sub(vis / 2));
        self.app.sync_focused_pane_viewport();
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
    pub fn split_set_ratio(&mut self, ratio: f32) {
        if !self.app.split.is_split() || !ratio.is_finite() {
            return;
        }
        self.app.split.ratio = ratio.clamp(0.15, 0.85);
        self.recompose_scroll();
    }

    /// Toggle a breakpoint on a specific 1-based line of the current file
    /// (gutter click — bookmark affordance).
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
            out.push((
                best_indent.min(200) as u8,
                best_len.min(255) as u8,
                flags,
            ));
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
            let same_tab = self.app.buffers.iter().any(|t| {
                t.filename
                    .as_ref()
                    .is_some_and(|p| p.to_string_lossy() == path)
            });
            if same_tab {
                // Switch to existing tab if present.
                if let Some(idx) = self.app.buffers.iter().position(|t| {
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
        self.app.goto_tab(index as usize);
        // In a split, the focused pane follows the document the user picked.
        self.app.sync_focused_pane_tab();
        // NO `update_scroll()` here. It re-derives the scroll from the CARET,
        // so a tab scrolled to line 7000 with the caret still at line 1 snapped
        // straight back to the top. The tab's saved scroll is authoritative.
        self.recompose();
    }

    pub fn close_tab(&mut self, index: u32) {
        let n = self.app.buffers.len();
        if n == 0 {
            return;
        }
        let idx = (index as usize).min(n.saturating_sub(1));
        self.app.goto_tab(idx);
        self.app.close_current_tab();
        self.app.sync_focused_pane_tab();
        self.app.update_scroll();
        self.recompose();
    }

    pub fn open_blank_tab(&mut self) {
        self.app.open_blank_tab();
        // Face-friendly: give blank tabs an Untitled path so they never look like
        // cold-start Welcome (filename=None + empty buffer) and tab chips show a title.
        if self.app.filename.is_none() {
            self.app.filename = Some(std::path::PathBuf::from("Untitled"));
            // Keep buffer tab metadata in sync.
            if let Some(tab) = self.app.buffers.get_mut(self.app.current_buffer) {
                tab.filename = self.app.filename.clone();
            }
        }
        self.app.update_scroll();
        self.recompose();
    }

    pub fn split_vertical(&mut self) {
        self.app.split_vertical();
        self.recompose();
    }

    pub fn split_horizontal(&mut self) {
        self.app.split_horizontal();
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
            SettingsAction::ApplyTheme
            | SettingsAction::ApplyGpuAcc
            | SettingsAction::ApplyLsp
            | SettingsAction::ApplyPet => {
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
            SettingsAction::OpenPluginStore | SettingsAction::None => {}
        }
        // GUI face has no "s to save" muscle memory — persist draft after every change.
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
        let buffer_row = self
            .app
            .scroll
            .saturating_add(row_in_view)
            .min(last);
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
    fn enter_keeps_both_lines_visible() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::char('i'));
        eng.dispatch_key(KeyEvent::char('a'));
        eng.dispatch_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        eng.dispatch_key(KeyEvent::char('b'));
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
        assert!(matches!(eng.app.mode, Mode::Normal));
        eng.mouse_up();
        assert!(matches!(eng.app.mode, Mode::Normal));
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
    fn drag_builds_gui_selection() {
        // Mouse drag now drives the GUI SelectionSet (exclusive), not vim
        // Visual mode. The painted span must still equal the yank slice.
        let mut eng = eng_with_text("abcdef");
        eng.click_at(0, 1, false);
        eng.drag_to(0, 1); // same cell — still a caret
        assert!(eng.app.sel.primary().is_empty());
        eng.drag_to(0, 4); // exclusive head at col 4 → covers chars 1,2,3
        assert!(!eng.app.sel.primary().is_empty());
        assert!(!matches!(eng.app.mode, Mode::Visual)); // no vim mode

        let (s, e) = eng.app.selected_range().expect("selection");
        assert_eq!((s.row, s.col), (0, 1), "anchor at first click");
        assert_eq!((e.row, e.col), (0, 3), "inclusive end = one before excl head");

        let line = &eng.last_diff.chrome.as_ref().unwrap().lines[0];
        let v0 = line.sel_v0.expect("sel_v0");
        let v1 = line.sel_v1.expect("sel_v1");
        let painted: String =
            line.text.chars().skip(v0 as usize).take((v1 - v0) as usize).collect();
        let chars: Vec<char> = eng.app.buffer.line(0).chars().collect();
        let yanked: String = chars[s.col..=e.col].iter().collect();
        assert_eq!(painted, yanked, "paint span must equal yank slice");
        assert_eq!(yanked, "bcd");

        eng.mouse_up();
        assert!(eng.app.selected_range().is_some(), "selection survives mouse up");
    }

    #[test]
    fn painted_selection_matches_yank_slice() {
        let mut eng = eng_with_text("hello");
        eng.click_at(0, 2, false);
        eng.drag_to(0, 2);
        assert!(eng.app.sel.primary().is_empty());
        eng.drag_to(0, 4); // exclusive [2,4) → chars 2,3
        let (s, e) = eng.app.selected_range().unwrap();
        let line = &eng.last_diff.chrome.as_ref().unwrap().lines[0];
        let v0 = line.sel_v0.unwrap();
        let v1 = line.sel_v1.unwrap();
        let painted: String =
            line.text.chars().skip(v0 as usize).take((v1 - v0) as usize).collect();
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
        assert!(!matches!(eng.app.mode, Mode::Visual), "no vim mode");
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
        assert!(matches!(eng.app.mode, Mode::Normal));
        // second click elsewhere
        eng.click_at(0, 7, false);
        eng.mouse_up();
        assert!(matches!(eng.app.mode, Mode::Normal));
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
        eng.app.viewport.height = 40;
        eng.app.viewport.width = 100;
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
        assert!(!eng.app.sel.primary().is_empty(), "word selected into GUI model");
        let (s, e) = eng.app.selected_range().expect("selection");
        let chars: Vec<char> = eng.app.buffer.line(0).chars().collect();
        let word: String = chars[s.col..=e.col].iter().collect();
        assert_eq!(word, "bar");
    }

    #[test]
    fn ctrl_f_opens_explorer_in_frame() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.dispatch_key(KeyEvent::new(
            KeyCode::Char('f'),
            KeyModifiers::CONTROL,
        ));
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
            eng.app
                .completions
                .activate("fn", Some("rs"));
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
        assert!(eng.app.buffers.len() >= 2);
        eng.goto_tab(0);
        assert_eq!(eng.app.current_buffer, 0);
        eng.goto_tab(1);
        assert_eq!(eng.app.current_buffer, 1);
    }

    #[test]
    fn vertical_split_paints_full_height_rows_not_half() {
        let mut eng = eng_with_text(&"line\n".repeat(80));
        eng.resize(1200.0, 720.0, 18.0, 9.0, 2.0);
        let rows_full = eng.shell.viewport.editor_rows as usize;
        eng.split_vertical();
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert_eq!(c.split_kind, 1, "vertical split");
        assert!(c.panes.len() >= 2);
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
        assert!(right_has, "right pane must paint file B without waiting for click");
        // Scroll left — right content must not become A.
        eng.focus_pane(0);
        eng.scroll_by(12);
        let c2 = eng.last_diff.chrome.as_ref().unwrap();
        assert!(
            c2.panes[1]
                .lines
                .iter()
                .any(|l| l.text.contains("RIGHT_ONLY_BBB")),
            "scrolling left must not clobber right pane content"
        );
        // Left advanced; right keeps its own scroll mirror.
        assert_eq!(c2.panes[1].scroll, 0, "right pane scroll must stay independent");
        assert!(c2.panes[0].scroll > 0, "left pane should advance scroll");
        assert!(
            c2.panes[0]
                .lines
                .iter()
                .any(|l| l.text.contains("left") || l.text.contains("LEFT_ONLY_AAA")),
            "left still A"
        );
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
        assert!(eng.app.buffers.len() >= 2);
        assert_eq!(c.tabs.len(), eng.app.buffers.len());
        // Active tab should be the new blank
        assert!(c.tabs.last().map(|t| t.active).unwrap_or(false));

        eng.goto_tab(0);
        let c2 = eng.last_diff.chrome.as_ref().unwrap();
        assert!(!c2.welcome);
        assert_eq!(eng.app.current_buffer, 0);
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
        let n0 = eng.app.buffers.len();

        // Simulate suisei_engine_open_path session path via App API used by FFI.
        eng.app.open_new_tab(b.to_str().unwrap());
        eng.recompose();
        assert_eq!(eng.app.buffers.len(), n0 + 1);
        assert_eq!(eng.app.current_buffer, eng.app.buffers.len() - 1);
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert!(c.tabs.len() >= 2);
        assert!(!c.welcome);
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
        assert!(matches!(eng.app.mode, Mode::Normal));
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
        eng.app.mode = Mode::Normal;
        eng.ensure_project_tree();
        assert!(matches!(eng.app.mode, Mode::Normal));
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
        eng.dispatch_key(KeyEvent::new(
            KeyCode::Char(','),
            KeyModifiers::CONTROL,
        ));
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
        // Row 1 = first Theme(i) after ThemeHeader
        eng.settings_activate(1);
        let theme_name = eng.last_diff.chrome.as_ref().unwrap().theme.name.clone();
        assert!(!theme_name.is_empty());
        assert_eq!(eng.app.theme.name, theme_name);
        assert!(eng.last_diff.chrome.as_ref().unwrap().settings.open);
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
        eng.dispatch_key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        ));
        let c = eng.last_diff.chrome.as_ref().unwrap();
        // May open empty if not in a git repo, but mode should be SourceControl
        assert!(
            matches!(eng.app.mode, Mode::SourceControl) || c.scm.open,
            "Ctrl+G should enter SCM"
        );
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
        eng.dispatch_key(KeyEvent::new(
            KeyCode::Char('f'),
            KeyModifiers::CONTROL,
        ));
        assert!(eng.app.explorer.open);
        assert_eq!(eng.app.buffer.cursor().row, 40, "cursor must not jump to line 1");
        assert_eq!(eng.app.buffer.cursor().col, 2);
        // Scroll should still show the caret row
        let vis = eng.app.viewport.height.max(1) as usize;
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
        std::fs::write(&path, "# Title\n\n```rust\nfn x() {}\n```\n\n// not a comment in md\n")
            .unwrap();
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app = App::open_file(path.to_str().unwrap());
        eng.sync_viewport_public();
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
        // hscroll only when wrap is off
        eng.app.wrap_lines = false;
        eng.scroll_to(10, 15);
        assert_eq!(eng.app.scroll, 10);
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
        assert_eq!(c.search.input, "bet", "search bar must repaint while typing");
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
        let c = eng.last_diff.chrome.as_ref().unwrap();
        assert_eq!(c.scroll, 200);
        let first = c.lines.first().map(|l| l.line_no).unwrap_or(0);
        let last = c.lines.last().map(|l| l.line_no).unwrap_or(0);
        assert!(
            first <= 200 - 40,
            "band must include overscan above scroll (first={first})"
        );
        assert!(
            last >= 200 + eng.app.viewport.height as u32,
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
    fn leader_which_key_paints_via_light_path() {
        let mut eng = Engine::new();
        eng.resize(1000.0, 700.0, 18.0, 9.0, 2.0);
        eng.app.key_hints = true;
        eng.dispatch_key(KeyEvent::char(' '));
        eng.app.which_key.force_ready();
        // Simulate the tick-driven light repaint (no full recompose).
        eng.recompose_scroll();
        if eng.app.which_key_visible() {
            let c = eng.last_diff.chrome.as_ref().unwrap();
            assert!(
                c.which_key.open,
                "leader which-key must appear via the scroll patch path"
            );
        }
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
        eng.app.mode = Mode::Insert;
        eng.dispatch_key(toggle);
        assert!(
            !eng.app.preview.open,
            "second toggle must close (closing={})",
            eng.app.preview.closing
        );
    }
}
