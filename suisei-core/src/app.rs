use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, Position};
use crate::completion::Completions;
use crate::config;
use crate::explorer::Explorer;
use crate::fold::FoldState;
use crate::git::{GitBlame, GitGutter};
use crate::multi_cursor::MultiCursor;
use crate::lsp::LspClient;
use crate::git_workbench::GitWorkbench;
use crate::preview::PreviewState;
use crate::scm::ScmPanel;
use crate::session::{self, Session, SessionFile};
use crate::settings::SettingsPanel;
use crate::nav::{Jump, JumpList};
use crate::palette::{Palette, PaletteAction};
use crate::registers::Registers;
use crate::syntax::SyntaxEngine;
use crate::term::Terminal;
use crate::theme::{self, Theme, OCEAN};
use crate::undo::UndoStack;

/// What kind of GUI edit the last keystroke was, for undo coalescing. A run of
/// the same kind shares one snapshot; switching kind (or moving the caret)
/// starts a new undo group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditRun {
    None,
    Insert,
    Delete,
}

/// Which surface owns the keyboard.
///
/// This is **focus, not a modal editing state**. It used to carry vim's
/// Normal/Insert/Visual on the same axis as the panels, which meant the editor
/// lived in vim's command mode and any key the GUI failed to intercept was read
/// as a vim command. There is now one editor state and typing always types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// The text editor. Keys are handled by the Selection-model tables in
    /// `Engine::dispatch_key`, never by a command interpreter.
    #[default]
    Editor,
    Explorer,
    Terminal,
    /// Incremental find bar (⌘F) — a panel that owns typed characters.
    Search,
    Palette,
    /// Light Source Control panel (Ctrl+G) — stage / commit / graph
    SourceControl,
    /// Full Git workbench (Ctrl+Shift+G) — branch / sync / diff / stash
    GitWorkbench,
    /// Unified settings (Ctrl+,) — About / Setting / Help
    Settings,
    /// Pretty document preview (Markdown / JSON) — Ctrl+Shift+V
    Preview,
    /// Workspace find / replace (Ctrl+Shift+F)
    WorkspaceSearch,
    /// DAP debugger panel (F5 / Ctrl+Shift+D)
    Debug,
    /// LSP call hierarchy panel
    CallHierarchy,
}

/// Sampled resource usage of the xei process, filled by the frontend for the
/// `:status` line. GPU is `None` where no per-process figure is obtainable
/// (e.g. macOS without elevated tooling) and renders as `—`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcMetrics {
    /// CPU busy normalized to one core (100% = one full core, like `top`).
    pub cpu_pct: f32,
    /// Total logical cores, so the UI can also show cores-in-use (cpu%/100).
    pub cores: u32,
    pub mem_pct: f32,
    pub mem_mb: f32,
    pub gpu_pct: Option<f32>,
    /// Set once the first sample lands, so the UI can show `…` until then.
    pub sampled: bool,
}

/// Why Core last moved the scroll position.
///
/// The face used to infer this from `abs(coreLine - clipLine)`, which cannot
/// distinguish "restore a tab" (must be instant) from "jump to a symbol" (must
/// animate) — any threshold gets one of them wrong. Core states the reason;
/// the face just obeys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ScrollIntent {
    /// Core did not move it — the view is already where it belongs.
    None = 0,
    /// Restored a tab's saved position. Place it at once.
    Restore = 1,
    /// Deliberate navigation (outline, goto, search hit). Glide there.
    Navigate = 2,
    /// Kept the caret on screen while editing. Place it at once.
    Caret = 3,
}

pub struct App {
    pub running: bool,
    pub mode: Mode,
    pub buffer: Buffer,
    pub message: String,
    pub filename: Option<PathBuf>,
    pub scroll: usize,
    /// Sub-line scroll offset in (−1, 1) for smooth GUI faces (TUI ignores).
    /// Positive = viewport shifted slightly toward the next line (finger scrolled up).
    pub scroll_frac: f32,
    /// Why `scroll` last changed — consumed and cleared by the face.
    pub scroll_intent: ScrollIntent,
    pub undo_stack: UndoStack,
    /// Deprecated alias surface: prefer `registers`. Kept in sync with unnamed.
    pub yank_buffer: Option<String>,
    pub registers: Registers,
    pub jumps: JumpList,
    /// GUI selection model (P0.2). Exclusive semantics, plural selections; the
    /// primary head mirrors `buffer.cursor`. Independent of the vim
    /// `visual_anchor`/`Mode` pair above, which stays for TUI compatibility —
    /// the GUI face drives this through the semantic `caret_*` commands.
    pub sel: crate::selection::SelectionSet,
    /// Undo coalescing for the GUI edits: a run of characters (or a run of
    /// deletes) shares one undo snapshot instead of cloning the whole buffer
    /// per keystroke. Reset by any caret move — moving then typing starts a
    /// fresh group, the standard editor contract.
    pub edit_run: crate::app::EditRun,
    /// Last committed search pattern (used by n/N after leaving search mode).
    pub search_pattern: Option<String>,
    /// Live query while in Search mode (does not touch `search_pattern` until commit).
    pub search_input: String,
    pub search_matches: Vec<Position>,
    pub search_current: usize,
    /// Cursor when `/` was pressed — restored on Esc cancel.
    pub search_origin: Option<Position>,
    pub search_scroll_origin: usize,
    /// Pattern that existed before this search session (restored on cancel).
    search_pattern_backup: Option<String>,
    /// `true` = forward `/`, `false` = reverse `?`
    pub search_forward: bool,
    pub completions: Completions,
    pub modified: bool,
    pub mouse: MouseState,
    pub viewport: EditorViewport,
    pub explorer: Explorer,
    pub terminal: Terminal,
    pub explorer_width: u16,
    pub terminal_width: u16,
    pub resize_target: Option<ResizeTarget>,
    pub explorer_separator_x: u16,
    pub terminal_separator_x: u16,
    pub screen_width: u16,
    pub screen_height: u16,
    pub theme: &'static Theme,
    /// The configured theme name — `"system"` means follow macOS.
    pub theme_pref: String,
    /// Current system appearance, pushed down by the face.
    pub system_is_dark: bool,
    pub xlc_height: u16,
    pub xlc_separator_y: u16,
    pub file_mtime: Option<std::time::SystemTime>,
    pub buffers: Vec<BufferTab>,
    pub current_buffer: usize,
    pub syntax: SyntaxEngine,
    pub lsp: LspClient,
    pub debug: bool,
    /// `:status` — show live CPU/MEM/GPU of this process in the status line.
    /// The frontend samples (platform-specific) and writes into `metrics`; core
    /// only owns the toggle + the last-sampled snapshot for rendering.
    pub show_metrics: bool,
    pub metrics: ProcMetrics,
    /// Latest `:bench` results (shown in `Mode::Bench`).
    pub bench_report: Option<crate::bench::BenchReport>,
    /// Plugin store surface state (`Mode::PluginStore`).
    pub plugin_store: crate::plugin_store::PluginStore,
    /// Extensions sidebar panel (`Mode::ExtPanel`) — a left column listing
    /// loaded extensions and their runnable commands.
    pub ext_panel_open: bool,
    pub ext_panel_sel: usize,
    /// Webview display (`Mode::Webview`): HTML awaiting a headless render (set by
    /// core, consumed by the frontend), the rendered Kitty image, and its title.
    pub webview_pending_html: Option<String>,
    pub webview_title: String,
    pub webview_image: Option<crate::media::ImageAsset>,
    /// VSCode-compatible extension host sidecar (v2, feature = "extensions").
    /// Lazily spawned on first use; `None` until then / when Node is absent.
    #[cfg(feature = "extensions")]
    pub ext: Option<xei_ext_host::ExtHost>,
    /// Pending text-object modifier `i`/`a` after operator
    pub pending_to_mod: Option<char>,
    /// Tab bar hit regions for mouse (filled by UI each frame)
    pub tab_hit_regions: Vec<(u16, u16, usize)>, // x_start, x_end, tab_index
    pub tab_bar_y: u16,
    /// Screen-row → buffer-row map for the current frame (handles soft-wrap).
    /// Index 0 = `viewport.text_y`. Built in the TUI draw path.
    pub screen_row_to_buffer: Vec<usize>,
    /// For each screen row, visual-column base within that buffer line
    /// (0, text_width, 2*text_width, …). Parallel to `screen_row_to_buffer`.
    pub screen_row_visual_base: Vec<usize>,
    pub palette: Palette,
    /// Hover popup text (LSP)
    pub hover_text: Option<String>,
    /// Double-click tracking (ms-ish ticks via counter)
    pub last_click: Option<(u16, u16, std::time::Instant)>,
    pub tab_width: usize,
    pub clipboard_sync: bool,
    pub relative_number: bool,
    /// Soft-wrap long lines; false = horizontal scroll via `hscroll`.
    pub wrap_lines: bool,
    /// Persist undo history to ~/.suisei/undo on close (config `undo_caching`).
    pub undo_caching: bool,
    /// Per-feature GPU toggles under `gpu_acc`.
    pub gpu_graphics: bool,
    pub gpu_hyperlinks: bool,
    /// Horizontal pan (visual columns) when wrap_lines is off.
    pub hscroll: usize,
    /// Last buffer version handed to the syntax highlighter (render cache).
    pub syntax_seen_version: u64,
    /// Last buffer version pushed to the LSP (didChange gate).
    lsp_synced_version: u64,
    /// Git gutter signs for the current file
    pub git: GitGutter,
    /// Optional git blame overlay (`gb` toggle)
    pub blame: GitBlame,
    /// Indent-based folds (`za` / `zc` / `zo` / `zM` / `zR`)
    pub folds: FoldState,
    /// Extra carets (primary = `buffer.cursor`)
    pub multi: MultiCursor,
    /// Light Source Control side panel (Ctrl+G)
    pub scm: ScmPanel,
    /// Full Git workbench (Ctrl+Shift+G)
    pub git_wb: GitWorkbench,
    /// Settings — About / Setting / Help (Ctrl+,)
    pub settings: SettingsPanel,
    /// Pretty preview pane (Markdown / JSON / media)
    pub preview: PreviewState,
    /// Kitty image asset for PreviewKind::Image
    pub preview_image: Option<crate::media::ImageAsset>,
    /// Audio player for PreviewKind::Audio
    pub preview_audio: Option<crate::media::AudioPlayer>,
    /// Editor splits (Ctrl+W v/s)
    pub split: crate::split::SplitState,
    /// Peek definition overlay
    pub peek: crate::peek::PeekState,
    /// Workspace find/replace panel
    pub workspace_search: crate::workspace_search::WorkspaceSearch,
    /// `:screensaver` xeifetch overlay
    pub screensaver: crate::screensaver::Screensaver,
    /// Desktop pet GIF overlay
    pub pet: crate::pet::PetState,
    /// Pane hit regions filled each frame: (x, y, w, h, pane_idx)
    pub pane_hit_regions: Vec<(u16, u16, u16, u16, usize)>,
    /// Split separator for mouse drag-resize (filled by UI each frame).
    pub split_sep_hit: Option<SplitSepHit>,
    /// Git workbench Log rows: (x, y, w, h, commit_index) for right-click menus
    pub git_log_hits: Vec<(u16, u16, u16, u16, usize)>,
    /// Git toolbar chips: (x, y, w, h, key 1..=8)
    pub git_tab_hits: Vec<(u16, u16, u16, u16, u8)>,
    /// DAP panel tab hits: (x, y, w, h, pane_id 0..3)
    pub dap_tab_hits: Vec<(u16, u16, u16, u16, u8)>,
    /// DAP list row hits: (x, y, w, h, row_index)
    pub dap_row_hits: Vec<(u16, u16, u16, u16, usize)>,
    /// DAP panel body rect for mouse (x, y, w, h)
    pub dap_panel_rect: Option<(u16, u16, u16, u16)>,
    /// Terminal rect (side panel / full window / pane-bound) for wheel routing
    pub terminal_rect: Option<(u16, u16, u16, u16)>,
    /// Inline preview images wanted this frame: (path, x, y, w_cells, rows).
    pub preview_gfx: Vec<(String, u16, u16, u16, u16)>,
    /// PR review tab chips: (x, y, w, h, tab 0=Files 1=Comments 2=Body)
    pub pr_tab_hits: Vec<(u16, u16, u16, u16, u8)>,
    /// PR review list rows: (x, y, w, h, row index)
    pub pr_row_hits: Vec<(u16, u16, u16, u16, usize)>,
    /// Git docked columns: (x, y, w, h, pane_id 0=Changes 1=Log 2=Files)
    pub git_pane_hits: Vec<(u16, u16, u16, u16, u8)>,
    /// Editor right-click context menu (Insert / Normal / Visual)
    pub editor_ctx: Option<EditorContextMenu>,
    /// Show LSP inlay hints when available
    pub inlay_hints_enabled: bool,
    /// Code actions awaiting palette selection (Ctrl+.)
    pub code_action_bank: Vec<crate::lsp::CodeActionItem>,
    /// GPU-terminal enhancements (Ghostty/Kitty sync, undercurl, graphics…)
    pub gpu_acc: bool,
    /// Which-key style chord hints after prefix keys.
    pub key_hints: bool,
    /// DAP debugger client + panel state.
    pub dap: crate::dap::DapClient,
    /// Call hierarchy panel (gC / SPC l c).
    pub call_hierarchy: crate::call_hierarchy::CallHierarchyState,
    /// Interactive rebase planner.
    pub rebase: crate::rebase::RebaseState,
    /// PR review (files + comments + diff).
    pub pr_review: crate::pr_review::PrReviewState,
    /// Plugin hooks (`~/.suisei/hooks.toml`).
    pub hooks: crate::hooks::HooksConfig,
    /// Release check + self-update (welcome notice · :update).
    pub update: crate::update::UpdateState,
    /// Hook results from background threads (drained by poll_hook_messages).
    hook_msg_tx: std::sync::mpsc::Sender<String>,
    hook_msg_rx: std::sync::mpsc::Receiver<String>,
    /// Async git gutter/blame refresh (latest generation wins).
    #[allow(clippy::type_complexity)]
    git_refresh_rx: Option<
        std::sync::mpsc::Receiver<(
            u64,
            String,
            (bool, std::collections::HashMap<usize, crate::git::GitSign>),
            Option<(bool, std::collections::HashMap<usize, crate::git::BlameLine>)>,
        )>,
    >,
    git_refresh_gen: u64,
    /// Show LSP code lenses in the editor.
    pub code_lens_enabled: bool,
    /// Detected terminal capabilities (filled by TUI shell at startup).
    /// Core only stores a simple summary string so headless tests stay free of
    /// crossterm queries; detailed flags live in the TUI `term_caps` module.
    pub term_caps_summary: String,
    pub term_sync: bool,
    pub term_undercurl: bool,
    pub term_underline_color: bool,
    pub term_hyperlinks: bool,
    /// Physical pixels per cell (from the frontend probe; 0 = unknown → 14).
    pub cell_px: u32,
    pub cell_px_h: u32,
    pub term_modern: bool,
    /// Terminal speaks Kitty graphics protocol (Ghostty/Kitty/WezTerm).
    pub term_kitty_graphics: bool,
    /// Pending rename: new name input via XLC or message
    pub rename_pending: bool,
    /// Last document state pushed to the LSP via didChange (path + text hash).
    /// `sync_lsp_document` uses these to send post-edit full-text syncs exactly
    /// once per change instead of the old pre-edit push_undo notification.
    lsp_synced_path: Option<PathBuf>,
    lsp_synced_hash: u64,
}

#[derive(Clone)]
pub struct BufferTab {
    pub buffer: Buffer,
    pub filename: Option<PathBuf>,
    pub scroll: usize,
    pub modified: bool,
    pub undo_stack: UndoStack,
    pub file_mtime: Option<std::time::SystemTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeTarget {
    Explorer,
    Terminal,
    Xlc,
    /// Drag the split divider between editor panes
    Split,
}

/// Hit target for the split divider (mouse drag resize).
#[derive(Clone, Copy, Debug)]
pub struct SplitSepHit {
    /// True = vertical split (left|right), divider is a column.
    pub vertical: bool,
    /// Screen x (vertical) or y (horizontal) of the divider line.
    pub pos: u16,
    /// Parent split area origin + size — used to compute ratio on drag.
    pub area_x: u16,
    pub area_y: u16,
    pub area_w: u16,
    pub area_h: u16,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MouseState {
    pub dragging: bool,
    pub drag_anchor: Option<Position>,
}

/// Right-click menu over the editor buffer.
#[derive(Debug, Clone)]
pub struct EditorContextMenu {
    pub x: u16,
    pub y: u16,
    pub sel: usize,
    pub items: Vec<EditorCtxItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCtxItem {
    Cut,
    Copy,
    Paste,
    SelectAll,
    Undo,
    Redo,
    GoToDefinition,
    FormatDocument,
    CommandPalette,
}

impl EditorCtxItem {
    pub fn label(self) -> &'static str {
        match self {
            EditorCtxItem::Cut => "Cut",
            EditorCtxItem::Copy => "Copy",
            EditorCtxItem::Paste => "Paste",
            EditorCtxItem::SelectAll => "Select All",
            EditorCtxItem::Undo => "Undo",
            EditorCtxItem::Redo => "Redo",
            EditorCtxItem::GoToDefinition => "Go to Definition",
            EditorCtxItem::FormatDocument => "Format Document",
            EditorCtxItem::CommandPalette => "Command Palette…",
        }
    }
    pub fn key_hint(self) -> &'static str {
        match self {
            EditorCtxItem::Cut => "⌘X",
            EditorCtxItem::Copy => "⌘C",
            EditorCtxItem::Paste => "⌘V",
            EditorCtxItem::SelectAll => "⌘A",
            EditorCtxItem::Undo => "u",
            EditorCtxItem::Redo => "^R",
            EditorCtxItem::GoToDefinition => "gd",
            EditorCtxItem::FormatDocument => "^⇧I",
            EditorCtxItem::CommandPalette => "⇧⌘P",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EditorViewport {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// X of first text column (after line-number gutter).
    pub text_x: u16,
    /// Y of first editor content row (same as `y` when borderless).
    pub text_y: u16,
}

impl Default for App {
    fn default() -> Self {
        let (hook_msg_tx, hook_msg_rx) = std::sync::mpsc::channel();
        Self {
            running: true,
            mode: Mode::Editor,
            buffer: Buffer::new(),
            message: String::from("Welcome to xei! i=insert :=XLC h/j/k/l=move"),
            filename: None,
            scroll: 0,
            scroll_frac: 0.0,
            scroll_intent: ScrollIntent::None,
            undo_stack: UndoStack::new(),
            yank_buffer: None,
            registers: Registers::new(),
            jumps: JumpList::new(),
            sel: crate::selection::SelectionSet::new(),
            edit_run: EditRun::None,
            search_pattern: None,
            search_input: String::new(),
            search_matches: Vec::new(),
            search_current: 0,
            search_origin: None,
            search_scroll_origin: 0,
            search_pattern_backup: None,
            search_forward: true,
            completions: Completions::new(),
            modified: false,
            mouse: MouseState::default(),
            viewport: EditorViewport::default(),
            explorer: Explorer::new(),
            terminal: Terminal::new(),
            explorer_width: 22,
            terminal_width: 30,
            resize_target: None,
            explorer_separator_x: 0,
            terminal_separator_x: 0,
            screen_width: 80,
            screen_height: 24,
            theme: &OCEAN,
            theme_pref: "system".to_string(),
            system_is_dark: true,
            xlc_height: 11,
            xlc_separator_y: 0,
            file_mtime: None,
            buffers: vec![BufferTab {
                buffer: Buffer::new(),
                filename: None,
                scroll: 0,
                modified: false,
                undo_stack: UndoStack::new(),
                file_mtime: None,
            }],
            current_buffer: 0,
            syntax: SyntaxEngine::new(),
            lsp: LspClient::new(),
            debug: false,
            show_metrics: false,
            metrics: ProcMetrics::default(),
            bench_report: None,
            plugin_store: crate::plugin_store::PluginStore::default(),
            ext_panel_open: false,
            ext_panel_sel: 0,
            webview_pending_html: None,
            webview_title: String::new(),
            webview_image: None,
            #[cfg(feature = "extensions")]
            ext: None,
            pending_to_mod: None,
            tab_hit_regions: Vec::new(),
            tab_bar_y: 0,
            screen_row_to_buffer: Vec::new(),
            screen_row_visual_base: Vec::new(),
            palette: Palette::new(),
            hover_text: None,
            last_click: None,
            tab_width: 4,
            clipboard_sync: true,
            relative_number: false,
            wrap_lines: true,
            undo_caching: false,
            gpu_graphics: true,
            gpu_hyperlinks: true,
            hscroll: 0,
            syntax_seen_version: 0,
            lsp_synced_version: 0,
            git: GitGutter::new(),
            blame: GitBlame::default(),
            folds: FoldState::new(),
            multi: MultiCursor::new(),
            scm: ScmPanel::new(),
            git_wb: GitWorkbench::new(),
            settings: SettingsPanel::new(),
            preview: PreviewState::new(),
            preview_image: None,
            preview_audio: None,
            split: crate::split::SplitState::new(),
            peek: crate::peek::PeekState::new(),
            workspace_search: crate::workspace_search::WorkspaceSearch::new(),
            screensaver: crate::screensaver::Screensaver::new(),
            pet: crate::pet::PetState::new(),
            pane_hit_regions: Vec::new(),
            split_sep_hit: None,
            git_log_hits: Vec::new(),
            git_tab_hits: Vec::new(),
            dap_tab_hits: Vec::new(),
            dap_row_hits: Vec::new(),
            dap_panel_rect: None,
            terminal_rect: None,
            preview_gfx: Vec::new(),
            pr_tab_hits: Vec::new(),
            pr_row_hits: Vec::new(),
            git_pane_hits: Vec::new(),
            editor_ctx: None,
            inlay_hints_enabled: true,
            code_action_bank: Vec::new(),
            gpu_acc: true,
            key_hints: true,
            dap: crate::dap::DapClient::new(),
            call_hierarchy: crate::call_hierarchy::CallHierarchyState::new(),
            rebase: crate::rebase::RebaseState::new(),
            pr_review: crate::pr_review::PrReviewState::new(),
            hooks: crate::hooks::HooksConfig::load(),
            update: crate::update::UpdateState::new(),
            hook_msg_tx,
            hook_msg_rx,
            git_refresh_rx: None,
            git_refresh_gen: 0,
            code_lens_enabled: true,
            term_caps_summary: String::new(),
            term_sync: false,
            term_undercurl: false,
            term_underline_color: false,
            cell_px: 0,
            cell_px_h: 0,
            term_hyperlinks: false,
            term_modern: false,
            term_kitty_graphics: false,
            rename_pending: false,
            lsp_synced_path: None,
            lsp_synced_hash: 0,
        }
    }
}

/// FNV-1a over the whole document — cheap enough per sync tick, and unlike a
/// sampled fingerprint it cannot miss an edit.
fn text_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl App {
    pub fn apply_config(&mut self) {
        let cfg = config::load();
        self.tab_width = cfg.tab_width;
        self.clipboard_sync = cfg.clipboard_sync;
        self.relative_number = cfg.relative_number;
        self.wrap_lines = cfg.wrap_lines;
        if self.wrap_lines {
            self.hscroll = 0;
        }
        self.undo_caching = cfg.undo_caching;
        self.gpu_graphics = cfg.gpu_graphics;
        self.gpu_hyperlinks = cfg.gpu_hyperlinks;
        self.gpu_acc = cfg.gpu_acc;
        self.key_hints = cfg.key_hints;
        self.lsp
            .apply_config(cfg.lsp_enabled, cfg.lsp_servers.clone());
        self.theme_pref = cfg.theme.clone();
        self.theme = theme::resolve(&cfg.theme, self.system_is_dark);
        self.apply_pet_from_config(&cfg);
    }

    pub fn apply_pet_from_config(&mut self, cfg: &config::Config) {
        self.pet.x = cfg.pet_x;
        self.pet.y = cfg.pet_y;
        let new_w = cfg.pet_width_cells.max(4);
        if new_w != self.pet.width_cells {
            self.pet.width_cells = new_w;
            self.pet.invalidate_display_cache();
        } else {
            self.pet.width_cells = new_w;
        }
        self.pet.speed = crate::pet::PetState::clamp_speed(cfg.pet_speed);
        let path = crate::pet::expand_path(&cfg.pet_path);
        let path_s = path.display().to_string();
        if !cfg.pet_path.is_empty()
            && (self.pet.path != path_s || !self.pet.has_frames())
        {
            self.pet.load_path(&path_s);
        }
        if cfg.pet_path.is_empty() {
            self.pet.path.clear();
        }
        // Pet only runs with GPU + Kitty graphics — never enable otherwise.
        // Do **not** clamp x/y here: before the first draw `screen_*` is still
        // the default 80×24, which would permanently trash a bottom-right save.
        self.pet.enabled = cfg.pet_enabled && self.pet_graphics_ok() && self.pet.has_frames();
    }

    /// Pet overlay is allowed only with gpu_acc + Kitty graphics terminal.
    pub fn pet_graphics_ok(&self) -> bool {
        self.gpu_acc && self.term_kitty_graphics
    }

    /// Max cell coords for nudging in Settings (uses live terminal size).
    pub fn pet_pos_max(&self) -> (u16, u16) {
        let w = self.screen_width.max(1);
        let h = self.screen_height.max(1);
        // Until the first real draw, report a generous max so we don't clamp
        // config values when the user opens Settings very early.
        if w <= 80 && h <= 24 && self.screen_width == 80 && self.screen_height == 24 {
            // Still might be a real 80×24 — use actual size either way.
        }
        let max_x = w.saturating_sub(self.pet.width_cells.max(1));
        let max_y = h.saturating_sub(2); // tab/status
        (max_x, max_y)
    }

    /// Paint-time position only (does not mutate saved coords).
    pub fn pet_screen_xy(&self) -> (u16, u16) {
        self.pet.screen_xy(self.screen_width, self.screen_height)
    }

    /// `:status` — toggle the live CPU/MEM/GPU readout in the status line.
    pub fn toggle_status_metrics(&mut self) {
        self.show_metrics = !self.show_metrics;
        if self.show_metrics {
            // Force an immediate sample next frame instead of showing stale data.
            self.metrics.sampled = false;
            self.message = "status: live CPU/MEM/GPU on — :status to hide".into();
        } else {
            self.message = "status: metrics off".into();
        }
    }

    /// The face reports the system appearance (and every change to it). When
    /// the user has not pinned a theme, this is what picks light vs dark —
    /// a native app should not stay light on a dark desktop.
    pub fn set_system_appearance(&mut self, is_dark: bool) {
        if self.system_is_dark == is_dark {
            return;
        }
        self.system_is_dark = is_dark;
        self.theme = crate::theme::resolve(&self.theme_pref, is_dark);
    }

    /// Status-line note. Replaces the XLC console the vim `:` command line
    /// used to dump into; the GUI shows one line, not a scrollback panel.
    pub fn set_message(&mut self, msg: &str) {
        self.message = msg.to_string();
    }

    /// Frontend hook: store the latest sampled process metrics.
    pub fn set_metrics(&mut self, m: ProcMetrics) {
        self.metrics = m;
    }


    #[cfg(feature = "extensions")]
    pub fn ext_test(&mut self) {
        if self.ext.is_none() {
            let (bootstrap, ext_dir) = xei_ext_host::spike_paths();
            let host = xei_ext_host::ExtHost::spawn("node", &bootstrap, &ext_dir);
            if let Some(err) = host.error.clone() {
                self.message = format!("ext: {err}");
                return;
            }
            self.ext = Some(host);
            if let Some(ext) = self.ext.as_mut() {
                ext.activate();
            }
        }
        if let Some(ext) = self.ext.as_mut() {
            ext.invoke_command("xei.hello");
            self.message = "ext: invoked xei.hello — awaiting reply…".into();
        }
    }

    #[cfg(not(feature = "extensions"))]
    pub fn ext_test(&mut self) {
        self.message =
            "extensions not built — rebuild with `--features extensions`".into();
    }

    /// `:extapi` — dump the running extension's `vscode.*` API-usage histogram
    /// into the XLC panel. Unimplemented calls (`✗`) are the role worklist.
    #[cfg(feature = "extensions")]
    pub fn ext_api_report(&mut self) {
        let lines = match self.ext.as_ref() {
            Some(ext) => ext.api_report(),
            None => {
                self.message = "ext: not started — run :exttest first".into();
                return;
            }
        };
        self.set_message(&format!("=== vscode.* API usage ({} distinct) ===", lines.len()));
        for l in &lines {
            self.set_message(l);
        }
        self.message = format!("ext: {} distinct vscode.* calls — see XLC panel", lines.len());
    }

    #[cfg(not(feature = "extensions"))]
    pub fn ext_api_report(&mut self) {
        self.message =
            "extensions not built — rebuild with `--features extensions`".into();
    }

    /// `:plugins` opens the store UI; `:plugins install <id>` installs directly
    /// (scriptable). Bare form defers to `open_plugin_store`.
    #[cfg(feature = "extensions")]
    pub fn ext_plugins(&mut self, arg: &str) {
        let arg = arg.trim();
        if let Some(id) = arg.strip_prefix("install").map(str::trim).filter(|s| !s.is_empty()) {
            match xei_ext_host::open_vsx_install(id) {
                Ok(e) => self.message = format!("installed {} v{} ({})", e.id, e.version, e.name),
                Err(e) => self.message = format!("install failed: {e}"),
            }
            return;
        }
        self.open_plugin_store();
    }

    #[cfg(not(feature = "extensions"))]
    pub fn ext_plugins(&mut self, _arg: &str) {
        self.message =
            "extensions not built — rebuild with `--features extensions`".into();
    }

    /// Spawn the extension host and load + activate every installed extension.
    /// Called once at startup. All extensions share one Node process.
    #[cfg(feature = "extensions")]
    pub fn load_installed_extensions(&mut self) {
        let installed = xei_ext_host::list_installed();
        if installed.is_empty() {
            return;
        }
        let bootstrap = xei_ext_host::bootstrap_path();
        let mut host = xei_ext_host::ExtHost::spawn("node", &bootstrap, "");
        if let Some(err) = host.error.clone() {
            self.message = format!("extensions disabled: {err}");
            return;
        }
        for e in &installed {
            host.load_extension(&e.id, &e.path.display().to_string());
        }
        self.message = format!("loading {} extension(s)…", installed.len());
        self.ext = Some(host);
    }

    #[cfg(not(feature = "extensions"))]
    pub fn load_installed_extensions(&mut self) {}

    /// Open the plugin store (`SPC x` / `Ctrl+Shift+X` / `:plugins`).
    #[cfg(feature = "extensions")]
    pub fn ext_panel_rows(&self) -> Vec<crate::plugin_store::ExtRow> {
        use crate::plugin_store::ExtRow;
        let mut rows = Vec::new();
        // Cached (refreshed when the panel opens / on install) — no per-frame fs.
        for inst in &self.plugin_store.installed {
            let loaded = self
                .ext
                .as_ref()
                .and_then(|e| e.loaded.iter().find(|l| l.id == inst.id));
            let failed = self
                .ext
                .as_ref()
                .and_then(|e| e.failed.iter().find(|(id, _)| *id == inst.id));
            let badge = if loaded.is_some() {
                '✓'
            } else if failed.is_some() {
                '✗'
            } else {
                '⋯'
            };
            rows.push(ExtRow {
                is_header: true,
                primary: format!("{badge} {}", inst.id),
                secondary: format!("v{}", inst.version),
                command: None,
            });
            if let Some(le) = loaded {
                for c in &le.commands {
                    let title = if c.title.is_empty() { c.command.clone() } else { c.title.clone() };
                    rows.push(ExtRow {
                        is_header: false,
                        primary: title,
                        secondary: c.command.clone(),
                        command: Some(c.command.clone()),
                    });
                }
            } else if let Some((_, err)) = failed {
                let short: String = err.chars().take(48).collect();
                rows.push(ExtRow {
                    is_header: false,
                    primary: format!("⚠ {short}"),
                    secondary: String::new(),
                    command: None,
                });
            }
        }
        rows
    }

    #[cfg(not(feature = "extensions"))]
    pub fn ext_panel_rows(&self) -> Vec<crate::plugin_store::ExtRow> {
        Vec::new()
    }



    pub fn ext_panel_move(&mut self, delta: isize) {
        let rows = self.ext_panel_rows();
        if rows.is_empty() {
            self.ext_panel_sel = 0;
            return;
        }
        let step = if delta >= 0 { 1isize } else { -1 };
        let mut cur = (self.ext_panel_sel as isize + delta).clamp(0, rows.len() as isize - 1);
        // Land on a runnable row, not a header.
        while cur >= 0 && (cur as usize) < rows.len() && rows[cur as usize].is_header {
            let next = cur + step;
            if next < 0 || next >= rows.len() as isize {
                break;
            }
            cur = next;
        }
        self.ext_panel_sel = cur.clamp(0, rows.len() as isize - 1) as usize;
    }

    /// `:webview` / ext-panel `w` — render the most recently opened extension
    /// webview and show it as a terminal image. The actual headless render
    /// happens on a frontend thread (it needs cell pixels); core just hands off
    /// the HTML and switches mode.
    #[cfg(feature = "extensions")]

    pub fn ext_panel_run(&mut self) {
        let rows = self.ext_panel_rows();
        let Some(cmd) = rows.get(self.ext_panel_sel).and_then(|r| r.command.clone()) else {
            return;
        };
        if let Some(ext) = self.ext.as_mut() {
            ext.invoke_command(&cmd);
            self.message = format!("ran: {cmd}");
        }
    }

    #[cfg(not(feature = "extensions"))]
    pub fn ext_panel_run(&mut self) {}

    /// Kick off an async Open VSX search for the current query.
    #[cfg(feature = "extensions")]
    pub fn plugin_store_search(&mut self) {
        use crate::plugin_store::{StoreItem, StoreMsg};
        let query = self.plugin_store.query.trim().to_string();
        self.plugin_store.mark_searched();
        if query.is_empty() {
            self.plugin_store.message = "empty query".into();
            return;
        }
        let installed: std::collections::HashSet<String> =
            self.plugin_store.installed.iter().map(|i| i.id.clone()).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let msg = match xei_ext_host::open_vsx_search(&query) {
                Ok(hits) => StoreMsg::Results(
                    hits.into_iter()
                        .map(|h| StoreItem {
                            installed: installed.contains(&h.id),
                            id: h.id,
                            name: h.name,
                            version: h.version,
                            description: h.description,
                            fidelity: None,
                            downloads: h.downloads,
                        })
                        .collect(),
                ),
                Err(e) => StoreMsg::Error(e),
            };
            let _ = tx.send(msg);
        });
        self.plugin_store.begin_job(rx, "searching Open VSX…");
    }

    /// Install the selected browse result (async).
    #[cfg(feature = "extensions")]
    pub fn plugin_store_install_selected(&mut self) {
        use crate::plugin_store::StoreMsg;
        let Some(item) = self.plugin_store.selected_item() else {
            return;
        };
        if self.plugin_store.tab == crate::plugin_store::StoreTab::Installed {
            self.plugin_store.message = "already installed — switch to Browse (Tab) to add more".into();
            return;
        }
        let id = item.id.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let msg = match xei_ext_host::open_vsx_install(&id) {
                Ok(e) => StoreMsg::Installed { id: e.id, version: e.version },
                Err(e) => StoreMsg::Error(e),
            };
            let _ = tx.send(msg);
        });
        self.plugin_store.begin_job(rx, &format!("installing {}…", item.id));
    }

    /// Refresh the installed list (after an install completes).
    #[cfg(feature = "extensions")]
    pub fn plugin_store_refresh_installed(&mut self) {
        use crate::plugin_store::StoreItem;
        self.plugin_store.installed = xei_ext_host::list_installed()
            .into_iter()
            .map(|e| StoreItem {
                id: e.id,
                name: e.name,
                version: e.version,
                description: String::new(),
                installed: true,
                fidelity: None,
                downloads: 0,
            })
            .collect();
    }

    /// Remove the selected installed extension.
    #[cfg(feature = "extensions")]
    pub fn plugin_store_uninstall_selected(&mut self) {
        if self.plugin_store.tab != crate::plugin_store::StoreTab::Installed {
            self.plugin_store.message = "switch to Installed (Tab) to remove".into();
            return;
        }
        let Some(item) = self.plugin_store.selected_item() else {
            return;
        };
        let id = item.id.clone();
        match xei_ext_host::uninstall(&id) {
            Ok(()) => {
                self.plugin_store_refresh_installed();
                self.plugin_store.selected = self.plugin_store.selected.saturating_sub(1);
                self.plugin_store.message = format!("removed {id}");
            }
            Err(e) => self.plugin_store.message = format!("uninstall failed: {e}"),
        }
    }

    #[cfg(not(feature = "extensions"))]
    pub fn plugin_store_search(&mut self) {}
    #[cfg(not(feature = "extensions"))]
    pub fn plugin_store_install_selected(&mut self) {}
    #[cfg(not(feature = "extensions"))]
    pub fn plugin_store_refresh_installed(&mut self) {}
    #[cfg(not(feature = "extensions"))]
    pub fn plugin_store_uninstall_selected(&mut self) {}


    pub fn new() -> Self {
        let mut app = Self::default();
        app.apply_config();
        app.dap.load_persisted_breakpoints();
        app
    }

    pub fn open_file(path: &str) -> Self {
        let pathbuf = PathBuf::from(path);
        let abs_path = if pathbuf.is_absolute() {
            pathbuf
        } else {
            env::current_dir()
                .unwrap_or_default()
                .join(&pathbuf)
        };
        let content = fs::read_to_string(&abs_path).unwrap_or_default();
        let message = format!("Opened: {}", abs_path.display());
        let buffer = Buffer::from_string(&content);
        let mut undo = UndoStack::new();
        undo.push(buffer.snapshot());
        let mtime = std::fs::metadata(&abs_path).ok().and_then(|m| m.modified().ok());
        let mut app = Self {
            buffer: buffer.clone(),
            filename: Some(abs_path.clone()),
            message,
            modified: false,
            undo_stack: undo.clone(),
            file_mtime: mtime,
            buffers: vec![BufferTab {
                buffer,
                filename: Some(abs_path.clone()),
                scroll: 0,
                modified: false,
                undo_stack: undo,
                file_mtime: mtime,
            }],
            current_buffer: 0,
            ..Self::default()
        };
        app.apply_config();
        {
            let text = app.buffer.text();
            app.undo_stack
                .attach_file(&abs_path, app.undo_caching, &text);
            app.lsp
                .auto_start_with_text(&abs_path.display().to_string(), Some(&text));
            app.lsp_synced_path = Some(abs_path.clone());
            app.lsp_synced_hash = text_hash(&text);
        }
        app.refresh_git();
        app
    }

    /// Restore tabs/cursors from `~/.suisei/session` (used when started with no file args).
    pub fn restore_session(&mut self) {
        let session = session::load();
        if session.files.is_empty() {
            return;
        }
        for (i, f) in session.files.iter().enumerate() {
            if i == 0 {
                // Replace the empty first tab
                let content = fs::read_to_string(&f.path).unwrap_or_default();
                self.buffer = Buffer::from_string(&content);
                self.filename = Some(PathBuf::from(&f.path));
                self.buffer.cursor.row = f.row.min(self.buffer.line_count().saturating_sub(1));
                let line_len = self.buffer.line(self.buffer.cursor.row).chars().count();
                self.buffer.cursor.col = f.col.min(line_len);
                self.modified = false;
                if !self.buffers.is_empty() {
                    self.buffers[0].buffer = self.buffer.clone();
                    self.buffers[0].filename = self.filename.clone();
                    self.buffers[0].modified = false;
                }
            } else {
                self.open_new_tab(&f.path);
                self.buffer.cursor.row = f.row.min(self.buffer.line_count().saturating_sub(1));
                let line_len = self.buffer.line(self.buffer.cursor.row).chars().count();
                self.buffer.cursor.col = f.col.min(line_len);
            }
        }
        let active = session.active.min(self.buffers.len().saturating_sub(1));
        if active != self.current_buffer {
            self.save_state_to_tab();
            self.current_buffer = active;
            self.restore_state_from_tab();
        }
        if let Some(ref p) = self.filename {
            let text = self.buffer.text();
            self.lsp
                .auto_start_with_text(&p.display().to_string(), Some(&text));
            self.lsp_synced_path = Some(p.clone());
            self.lsp_synced_hash = text_hash(&text);
        }
        self.refresh_git();
        self.dap.load_persisted_breakpoints();
        self.message = format!("Restored session ({} file(s))", session.files.len());
    }

    pub fn save_session(&self) {
        let mut files = Vec::new();
        for (i, tab) in self.buffers.iter().enumerate() {
            let Some(ref path) = tab.filename else {
                continue;
            };
            let (row, col) = if i == self.current_buffer {
                (self.buffer.cursor.row, self.buffer.cursor.col)
            } else {
                (tab.buffer.cursor.row, tab.buffer.cursor.col)
            };
            files.push(SessionFile {
                path: path.display().to_string(),
                row,
                col,
            });
        }
        if files.is_empty() {
            return;
        }
        let active = self
            .buffers
            .iter()
            .enumerate()
            .filter(|(_, t)| t.filename.is_some())
            .position(|(i, _)| i == self.current_buffer)
            .unwrap_or(0);
        session::save(&Session { files, active });
        let _ = self.dap.persist_breakpoints();
    }

    /// Non-blocking: `git diff` (+ `git blame` when the panel is up) run on a
    /// background thread; results land via [`App::poll_git_refresh`].
    pub fn refresh_git(&mut self) {
        if let Some(ref p) = self.filename {
            let path = p.display().to_string();
            let want_blame = self.blame.enabled || self.blame.open;
            self.git_refresh_gen = self.git_refresh_gen.wrapping_add(1);
            let generation = self.git_refresh_gen;
            let (tx, rx) = std::sync::mpsc::channel();
            self.git_refresh_rx = Some(rx);
            std::thread::spawn(move || {
                let gutter = crate::git::compute_gutter(&path);
                let blame = if want_blame {
                    Some(crate::git::compute_blame(&path))
                } else {
                    None
                };
                let _ = tx.send((generation, path, gutter, blame));
            });
        } else {
            self.git.clear();
            self.blame.clear();
            self.blame.enabled = false;
        }
        self.rebuild_folds();
    }

    /// Apply a finished background git refresh (call once per frame).
    pub fn poll_git_refresh(&mut self) -> bool {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = self.git_refresh_rx.take() else {
            return false;
        };
        match rx.try_recv() {
            Ok((generation, path, (g_avail, signs), blame)) => {
                if generation != self.git_refresh_gen {
                    return false;
                }
                self.git.path = path.clone();
                self.git.available = g_avail;
                self.git.signs = signs;
                if let Some((b_avail, lines)) = blame {
                    self.blame.path = path;
                    self.blame.available = b_avail;
                    self.blame.lines = lines;
                    if !b_avail && self.blame.open {
                        self.blame.close_panel();
                        self.blame.enabled = false;
                        self.message = "Blame unavailable (not a git file?)".into();
                    }
                }
                true
            }
            Err(TryRecvError::Empty) => {
                self.git_refresh_rx = Some(rx);
                false
            }
            Err(TryRecvError::Disconnected) => false,
        }
    }

    pub fn rebuild_folds(&mut self) {
        let lines = self.buffer.lines();
        self.folds.rebuild(&lines, self.tab_width.max(1));
    }

    /// Toggle blame side panel (`Ctrl+B` / `gb`) with slide animation.
    pub fn toggle_blame(&mut self) {
        let path = self
            .filename
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if path.is_empty() {
            self.message = "No file for blame".into();
            return;
        }
        self.message = self.blame.toggle_panel(&path);
    }

    /// Toggle DAP debug panel *focus* (Ctrl+Shift+D). Visibility and focus are
    /// separate: Esc drops focus back to the editor keeping the panel docked;
    /// `q` in the panel closes it.
    pub fn toggle_debug_panel(&mut self) {
        if self.mode == Mode::Debug {
            self.mode = Mode::Editor;
            self.message = "Debug unfocused · Ctrl+Shift+D refocus · q in panel closes".into();
        } else if self.dap.panel_open {
            self.mode = Mode::Debug;
            self.message = "Debug · F5 start · F9 bp · F10/F11 step · Esc unfocus".into();
        } else {
            self.dap.panel_open = true;
            self.dap.arm_panel_animation();
            self.mode = Mode::Debug;
            self.message = "Debug · F5 start · F9 bp · F10/F11 step · Esc unfocus".into();
        }
    }

    /// Close the debug panel entirely (`q` from the panel).
    pub fn close_debug_panel(&mut self) {
        self.dap.panel_open = false;
        if self.mode == Mode::Debug {
            self.mode = Mode::Editor;
        }
        self.message = "Debug panel closed".into();
    }

    /// `:mbb` — fresh blank tab landing on the welcome screen.
    pub fn open_blank_tab(&mut self) {
        self.save_state_to_tab();
        let buffer = Buffer::new();
        let mut undo = UndoStack::new();
        undo.push(buffer.snapshot());
        self.buffers.push(crate::BufferTab {
            buffer,
            filename: None,
            scroll: 0,
            modified: false,
            undo_stack: undo,
            file_mtime: None,
        });
        self.current_buffer = self.buffers.len() - 1;
        self.restore_state_from_tab();
        self.split.clamp_tabs(self.buffers.len());
        self.refresh_git();
        self.mode = Mode::Editor;
        self.message = "New tab · i insert · Ctrl+P files · :e <file>".into();
    }

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
            let cwd = cfg
                .cwd
                .as_ref()
                .map(PathBuf::from)
                .or_else(|| {
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
            return self
                .dap
                .attach_port(port, lang, cfg.host.as_deref());
        }
        // Heuristic fallback: program field as port or pid
        if let Some(port) = cfg.program.parse::<u16>().ok().or_else(|| {
            cfg.program
                .rsplit(':')
                .next()
                .and_then(|s| s.parse().ok())
        }) {
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
            self.set_message(&format!(
                "  {}  [{}]  {}",
                c.name, c.request, c.program
            ));
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

    /// Open call hierarchy (incoming by default).
    pub fn open_call_hierarchy(&mut self, outgoing: bool) {
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            self.message = "No file for call hierarchy".into();
            return;
        };
        if !self.lsp.server_running {
            self.message = "LSP not running".into();
            return;
        }
        let dir = if outgoing {
            crate::call_hierarchy::CallDirection::Outgoing
        } else {
            crate::call_hierarchy::CallDirection::Incoming
        };
        let c = self.buffer.cursor();
        // Word under cursor as provisional root name
        let word = {
            let w = self.word_under_cursor();
            if w.is_empty() {
                "?".into()
            } else {
                w
            }
        };
        self.sync_lsp_document();
        self.call_hierarchy.begin(&word, dir);
        self.mode = Mode::CallHierarchy;
        self.lsp
            .request_call_hierarchy(&path, c.row, c.col, dir);
        self.message = format!("Call hierarchy ({})…", dir.label());
    }

    pub fn toggle_call_direction(&mut self) {
        if !self.call_hierarchy.open {
            return;
        }
        let dir = self.call_hierarchy.direction.toggle();
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            return;
        };
        let c = self.buffer.cursor();
        let name = self.call_hierarchy.root_name.clone();
        self.call_hierarchy.begin(&name, dir);
        self.lsp
            .request_call_hierarchy(&path, c.row, c.col, dir);
    }

    /// Apply finished call hierarchy from LSP poll.
    pub fn poll_call_hierarchy(&mut self) {
        if !self.lsp.call_hierarchy_ready {
            return;
        }
        self.lsp.call_hierarchy_ready = false;
        let items = std::mem::take(&mut self.lsp.pending_call_hierarchy);
        if let Some(dir) = self.lsp.pending_call_direction {
            self.call_hierarchy.direction = dir;
        }
        if let Some(first) = items.first() {
            if self.call_hierarchy.root_name == "?" || self.call_hierarchy.root_name.is_empty() {
                self.call_hierarchy.root_name = first.name.clone();
            }
        }
        self.call_hierarchy.set_items(items);
        self.message = self.call_hierarchy.message.clone();
    }


    pub fn run_rebase_plan(&mut self) {
        match self.rebase.run() {
            Ok(msg) => {
                self.mode = Mode::Editor;
                self.message = msg;
            }
            Err(e) => self.message = e,
        }
    }



    pub fn toggle_code_lens(&mut self) {
        self.code_lens_enabled = !self.code_lens_enabled;
        self.message = if self.code_lens_enabled {
            self.lsp.mark_code_lens_dirty();
            "code lens on".into()
        } else {
            "code lens off".into()
        };
    }

    pub fn reload_hooks(&mut self) {
        self.hooks = crate::hooks::HooksConfig::load();
        self.message = format!(
            "hooks reloaded · enabled={}",
            self.hooks.enabled
        );
    }

    /// Run the hook for `event` on a background thread; results arrive via
    /// poll_hook_messages() so a slow hook never blocks the editor.
    fn fire_hook(&mut self, event: crate::hooks::HookEvent) {
        if !self.hooks.has_hook(event) {
            return;
        }
        let cfg = self.hooks.clone();
        let file = self.filename.clone();
        let tx = self.hook_msg_tx.clone();
        std::thread::spawn(move || {
            if let Some(msg) = crate::hooks::run_hooks(&cfg, event, file.as_deref()) {
                let _ = tx.send(msg);
            }
        });
    }

    /// Drain finished hook results into the status message (call per frame).
    pub fn poll_hook_messages(&mut self) {
        while let Ok(msg) = self.hook_msg_rx.try_recv() {
            self.message = msg;
        }
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

    pub fn fold_toggle(&mut self) {
        let row = self.buffer.cursor.row;
        self.rebuild_folds();
        if let Some(msg) = self.folds.toggle(row) {
            self.message = msg.into();
            if self.folds.is_hidden(self.buffer.cursor.row) {
                for r in &self.folds.ranges {
                    if self.folds.is_closed(r.start)
                        && self.buffer.cursor.row > r.start
                        && self.buffer.cursor.row <= r.end
                    {
                        self.buffer.cursor.row = r.start;
                        self.buffer.clamp_col();
                        break;
                    }
                }
            }
            self.update_scroll();
        } else {
            self.message = "No fold here".into();
        }
    }

    pub fn fold_close(&mut self) {
        self.rebuild_folds();
        if self.folds.close_at(self.buffer.cursor.row) {
            self.message = "fold closed".into();
        } else {
            self.message = "No fold here".into();
        }
    }

    pub fn fold_open(&mut self) {
        if self.folds.open_at(self.buffer.cursor.row) {
            self.message = "fold opened".into();
        } else {
            self.message = "No closed fold here".into();
        }
    }

    pub fn fold_close_all(&mut self) {
        self.rebuild_folds();
        self.folds.close_all();
        self.message = "all folds closed".into();
        self.update_scroll();
    }

    pub fn fold_open_all(&mut self) {
        self.folds.open_all();
        self.message = "all folds opened".into();
    }

    /// Toggle light Source Control panel (Ctrl+G).
    /// From Git workbench → step back to light SCM.
    pub fn toggle_scm(&mut self) {
        if self.mode == Mode::GitWorkbench {
            self.leave_git_workbench_to_scm();
            return;
        }
        if self.scm.open && self.mode == Mode::SourceControl {
            if self.scm.closing {
                let hint = self.filename.as_deref();
                self.scm.open_and_refresh(hint);
                return;
            }
            self.close_scm();
            return;
        }
        if self.palette.open {
            self.palette.close();
        }
        if self.preview.open {
            self.preview.close_immediate();
        }
        if self.git_wb.open {
            self.git_wb.close();
        }
        let hint = self.filename.as_deref();
        self.scm.open_and_refresh(hint);
        self.mode = Mode::SourceControl;
        if let Some(ref err) = self.scm.error {
            self.message = err.clone();
        } else {
            let n = self.scm.total_files();
            let branch = if self.scm.branch.is_empty() {
                "git".into()
            } else {
                self.scm.branch.clone()
            };
            self.message = format!(
                "SCM · {} · {} change(s)  ·  Ctrl+Shift+G full Git",
                branch, n
            );
        }
    }

    /// Begin slide-out close (mode flips to Normal when anim settles).
    pub fn close_scm(&mut self) {
        if !self.scm.open {
            if self.mode == Mode::SourceControl {
                self.mode = Mode::Editor;
            }
            return;
        }
        self.scm.close();
    }

    pub fn close_scm_immediate(&mut self) {
        self.scm.close_immediate();
        if matches!(self.mode, Mode::SourceControl) {
            self.mode = Mode::Editor;
        }
    }

    /// Open full Git workbench (Ctrl+Shift+G).
    pub fn open_git_workbench(&mut self) {
        let from_scm = self.mode == Mode::SourceControl || self.scm.open;
        if self.palette.open {
            self.palette.close();
        }
        if self.preview.open {
            self.preview.close_immediate();
        }
        // Keep SCM state but hide panel while in workbench
        if self.scm.open && !self.scm.closing {
            // leave scm open flag? close visual only
            self.scm.close_immediate();
        }
        // Hint = open file, else cwd (same resolution light SCM uses via find_git_root)
        let cwd = env::current_dir().ok();
        let hint = self
            .filename
            .as_deref()
            .or(cwd.as_deref());
        self.git_wb.open_at(hint, from_scm);
        self.mode = Mode::GitWorkbench;
        let b = if self.git_wb.branch.is_empty() {
            "git".into()
        } else {
            self.git_wb.branch.clone()
        };
        let root_note = self
            .git_wb
            .root
            .as_ref()
            .and_then(|r| r.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or(".");
        self.message = format!(
            "Git · {} @ {}  ·  Status ready  ·  Esc back",
            b, root_note
        );
    }

    pub fn toggle_git_workbench(&mut self) {
        if self.mode == Mode::GitWorkbench {
            self.close_git_workbench();
        } else {
            self.open_git_workbench();
        }
    }

    /// Esc / close from workbench: back to light SCM if we came from there.
    pub fn close_git_workbench(&mut self) {
        let back_to_scm = self.git_wb.from_scm;
        self.git_wb.close();
        if back_to_scm {
            let hint = self.filename.as_deref();
            self.scm.open_and_refresh(hint);
            self.mode = Mode::SourceControl;
            self.message = String::from("Source Control");
        } else {
            self.mode = Mode::Editor;
            self.message.clear();
        }
    }

    fn leave_git_workbench_to_scm(&mut self) {
        self.git_wb.from_scm = true;
        self.close_git_workbench();
    }

    /// Open unified Settings (Ctrl+,). Starts on About page.
    pub fn open_settings(&mut self) {
        if self.mode == Mode::Settings {
            self.close_settings();
            return;
        }
        if self.palette.open {
            self.palette.close();
        }
        if self.preview.open {
            self.preview.close_immediate();
        }
        if self.git_wb.open {
            self.git_wb.close();
        }
        if self.scm.open {
            self.scm.close_immediate();
        }
        self.settings.open_panel();
        self.plugin_store_refresh_installed(); // populate the Extensions page
        self.mode = Mode::Settings;
        self.message = format!(
            "Settings · {}  ·  Tab pages · Enter apply · s save · Esc",
            crate::settings::SettingsPanel::version_string()
        );
    }

    pub fn close_settings(&mut self) {
        // GUI faces often dismiss without an explicit "s" — persist if dirty.
        if self.settings.dirty {
            self.save_settings();
        }
        self.settings.close();
        self.mode = Mode::Editor;
        self.message.clear();
    }

    pub fn apply_settings_draft(&mut self) {
        let cfg = self.settings.draft.clone();
        self.tab_width = cfg.tab_width;
        self.clipboard_sync = cfg.clipboard_sync;
        self.relative_number = cfg.relative_number;
        self.wrap_lines = cfg.wrap_lines;
        if self.wrap_lines {
            self.hscroll = 0;
        }
        self.undo_caching = cfg.undo_caching;
        self.gpu_graphics = cfg.gpu_graphics;
        self.gpu_hyperlinks = cfg.gpu_hyperlinks;
        self.gpu_acc = cfg.gpu_acc;
        self.key_hints = cfg.key_hints;
        self.lsp
            .apply_config(cfg.lsp_enabled, cfg.lsp_servers.clone());
        self.theme_pref = cfg.theme.clone();
        self.theme = theme::resolve(&cfg.theme, self.system_is_dark);
        self.apply_pet_from_config(&cfg);
        // Restart LSP for current file with new server map
        self.lsp_restart_for_current();
    }







    pub fn toggle_terminal_side(&mut self) {
        if self.terminal.open && !self.terminal.full_panel {
            self.terminal.open = false;
            self.terminal.shutdown();
            self.mode = Mode::Editor;
        } else {
            // Switch from full to side, or open side
            self.terminal.full_panel = false;
            self.terminal.pane_bound = None;
            self.terminal.close_confirm = false;
            self.terminal.open = true;
            self.terminal.start(self.filename.as_ref());
            self.mode = Mode::Terminal;
        }
    }

    /// Ctrl+Shift+T — terminal as a real split *pane* (not Mode::Terminal side panel).
    /// Stays in Normal so Ctrl+W / Git / layout chords keep working.
    pub fn toggle_terminal_full(&mut self) {
        if self.terminal.open && self.terminal.full_panel {
            // Second toggle closes immediately (GUI faces + simpler TUI).
            // Confirm-only path remains for Esc / Ctrl+Shift+W.
            self.confirm_close_pane_terminal(true);
            return;
        }
        // If side terminal (Ctrl+T) was open, promote it to pane terminal without
        // leaving Mode::Terminal (which would also paint as "side" on some faces).
        let was_side = self.terminal.open && !self.terminal.full_panel;
        if was_side {
            // Keep existing PTY; just rebind as pane window.
            if matches!(self.mode, Mode::Terminal) {
                self.mode = Mode::Editor;
            }
        }
        // Open as a window: ensure a split exists so the editor stays visible.
        self.terminal.owns_split = false;
        if !self.split.is_split() {
            let tab = self.current_buffer;
            let scroll = self.scroll;
            let cur = (self.buffer.cursor.row, self.buffer.cursor.col);
            self.split
                .open_split(crate::split::SplitKind::Vertical, tab, scroll, cur);
            self.split.set_focus(1);
            self.sync_split_from_active();
            self.terminal.owns_split = true;
        }
        self.terminal.full_panel = true;
        self.terminal.pane_bound =
            Some(self.split.focus.min(self.split.panes.len().saturating_sub(1)));
        self.terminal.close_confirm = false;
        self.terminal.open = true;
        // Size + start PTY now (desktop face has no TUI first-paint start hook).
        // Half-width for vertical split is a better COLUMNS guess.
        let cols = if self.split.is_split() {
            (self.viewport.width / 2).max(40)
        } else {
            self.viewport.width.max(40)
        };
        let rows = self.viewport.height.max(24);
        self.terminal.resize(cols, rows);
        if !self.terminal.started {
            let root = self.project_root();
            let anchor = self.filename.clone().unwrap_or_else(|| root.join("."));
            self.terminal.start(Some(&anchor));
        }
        // Critical: stay in Normal — terminal is a pane, not a mode that
        // swallows layout shortcuts / opens the side Debug terminal.
        if matches!(self.mode, Mode::Terminal | Mode::Editor) {
            self.mode = Mode::Editor;
        }
        self.message = if self.terminal.started {
            "Terminal pane · keys → shell · ⌃⇧T close · ^W w other pane".into()
        } else {
            "Terminal: failed to spawn shell (PTY)".into()
        };
    }

    /// Whether the focused split pane is showing the Ctrl+Shift+T terminal.
    pub fn terminal_window_focused(&self) -> bool {
        if !self.terminal.open || !self.terminal.full_panel {
            return false;
        }
        match self.terminal.pane_bound {
            Some(i) if self.split.is_split() => {
                self.split.focus.min(self.split.panes.len().saturating_sub(1)) == i
            }
            // Unsplit but still full_panel (split closed under us)
            _ => true,
        }
    }

    pub fn request_close_pane_terminal(&mut self) {
        if !self.terminal.open || !self.terminal.full_panel {
            return;
        }
        if self.terminal.close_confirm {
            // Second Ctrl+Shift+W while confirming — cancel
            self.terminal.close_confirm = false;
            self.message = "Close cancelled".into();
            return;
        }
        self.terminal.close_confirm = true;
        self.message = "Close terminal?  [y]es  /  [n]o  ·  Ctrl+Shift+W cancel".into();
    }

    pub fn confirm_close_pane_terminal(&mut self, yes: bool) {
        self.terminal.close_confirm = false;
        if !yes {
            self.message = "Close cancelled".into();
            return;
        }
        if self.terminal.open && self.terminal.full_panel {
            // ⌃⇧T conjured this split just to host the shell — closing the
            // shell collapses it back to the single editor (leaving an empty
            // duplicate [No Name] pane behind was a GUI-face bug). A split the
            // user already had stays untouched.
            if self.terminal.owns_split && self.split.is_split() {
                self.terminal.owns_split = false;
                let bound = self
                    .terminal
                    .pane_bound
                    .unwrap_or(self.split.focus)
                    .min(self.split.panes.len().saturating_sub(1));
                self.split.set_focus(bound);
                self.close_split(); // shuts the PTY down with its pane
                if matches!(self.mode, Mode::Terminal) {
                    self.mode = Mode::Editor;
                }
                self.message = "Terminal window closed".into();
                return;
            }
            self.terminal.owns_split = false;
            self.terminal.open = false;
            self.terminal.full_panel = false;
            self.terminal.pane_bound = None;
            self.terminal.shutdown();
            if matches!(self.mode, Mode::Terminal) {
                self.mode = Mode::Editor;
            }
            self.message = "Terminal window closed".into();
        }
    }

    /// Whether progressive GPU-terminal features should run this session.
    pub fn gpu_active(&self) -> bool {
        self.gpu_acc
            && (self.term_modern
                || self.term_sync
                || self.term_underline_color
                || self.term_undercurl)
    }

    /// Install terminal caps discovered by the TUI shell.
    pub fn set_term_caps(
        &mut self,
        summary: String,
        sync: bool,
        undercurl: bool,
        underline_color: bool,
        hyperlinks: bool,
        modern: bool,
        kitty_graphics: bool,
    ) {
        self.term_caps_summary = summary;
        self.term_sync = sync;
        self.term_kitty_graphics = kitty_graphics;
        self.term_undercurl = undercurl;
        self.term_underline_color = underline_color;
        self.term_hyperlinks = hyperlinks;
        self.term_modern = modern;
    }

    pub fn save_settings(&mut self) {
        self.settings.save();
        self.apply_settings_draft();
        self.message = self
            .settings
            .status
            .clone()
            .unwrap_or_else(|| "Settings saved".into());
    }

    pub fn scm_refresh(&mut self) {
        let hint = self.filename.as_deref();
        self.scm.refresh(hint);
        self.refresh_git();
    }

    pub fn scm_commit(&mut self) {
        match self.scm.commit(false) {
            Ok(()) => {
                let summary = self
                    .scm
                    .last_result
                    .clone()
                    .unwrap_or_else(|| "Committed".into());
                self.message = format!("✓ {}", summary);
                self.refresh_git();
            }
            Err(e) => {
                self.message = e;
            }
        }
    }

    pub fn scm_stage_selected(&mut self) {
        match self.scm.stage_selected() {
            Ok(()) => {
                self.message = "Staged/unstaged".into();
                self.refresh_git();
            }
            Err(e) => self.message = e,
        }
    }

    pub fn scm_stage_all(&mut self) {
        match self.scm.stage_all() {
            Ok(()) => {
                self.message = self
                    .scm
                    .last_result
                    .clone()
                    .unwrap_or_else(|| "Staged all".into());
                self.refresh_git();
            }
            Err(e) => self.message = e,
        }
    }

    pub fn scm_open_selected_file(&mut self) {
        let Some(entry) = self.scm.entry_at(self.scm.selected).cloned() else {
            return;
        };
        let path = if let Some(ref root) = self.scm.root {
            root.join(&entry.path)
        } else {
            PathBuf::from(&entry.path)
        };
        let path_str = path.display().to_string();
        self.close_scm_immediate();
        self.open_new_tab(&path_str);
    }

    /// Toggle pretty preview for the current buffer (Markdown / JSON / media).
    pub fn toggle_preview(&mut self) {
        // Close whenever the preview is open, regardless of mode: GUI faces
        // can be back in Insert (mouse focus) while the panel is showing —
        // gating on Mode::Preview made the second toggle re-open instead.
        if self.preview.open {
            if self.preview.closing {
                let text = self.buffer.text();
                let ext = self.file_extension();
                self.preview.base_dir = self
                    .filename
                    .as_ref()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()));
                self.preview.cell_dims =
                    (self.cell_px_or_default(), self.cell_px_h_or_default());
                self.preview.open_for(&text, ext.as_deref());
                return;
            }
            self.close_preview();
            return;
        }
        if self.scm.open {
            self.close_scm_immediate();
        }
        if self.palette.open {
            self.palette.close();
        }
        // Prefer path-based media if current file is an image/csv/npy/audio
        if let Some(ref path) = self.filename.clone() {
            if crate::media::is_media_path(path) {
                match self.open_media_preview(path) {
                    Ok(()) => return,
                    Err(e) => {
                        self.message = e;
                        return;
                    }
                }
            }
        }
        let text = self.buffer.text();
        let ext = self.file_extension();
        self.clear_media_handles();
        self.preview.open_for(&text, ext.as_deref());
        self.mode = Mode::Preview;
        let kind = self
            .preview
            .kind
            .map(|k| k.label())
            .unwrap_or("Preview");
        self.message = format!("Preview · {kind} — Esc close · j/k scroll · r refresh");
    }

    /// Open media / data preview from a filesystem path (explorer Enter).
    /// Effective pixels-per-cell for image caches.
    pub fn cell_px_or_default(&self) -> u32 {
        if self.cell_px >= 4 { self.cell_px } else { 14 }
    }

    pub fn cell_px_h_or_default(&self) -> u32 {
        if self.cell_px_h >= 6 {
            self.cell_px_h
        } else {
            self.cell_px_or_default() * 2
        }
    }

    pub fn open_media_preview(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.clear_media_handles();
        self.preview.open_path(path)?;
        let kind = self.preview.kind;
        match kind {
            Some(crate::preview::PreviewKind::Image) => {
                match crate::media::ImageAsset::load(path, self.cell_px_or_default()) {
                    Ok(img) => {
                        self.message = format!(
                            "Image · {}×{} · ←/→ resize · Esc close",
                            img.src_w, img.src_h
                        );
                        self.preview_image = Some(img);
                    }
                    Err(e) => {
                        self.preview.lines.push(crate::preview::PreviewLine {
                            spans: vec![(format!("  load error: {e}"), crate::preview::PreviewStyle::AlertWarning)],
                            image: None,
                        });
                        self.message = e;
                    }
                }
            }
            Some(crate::preview::PreviewKind::Audio) => {
                self.preview_audio = Some(crate::media::AudioPlayer::new(path.to_path_buf()));
                self.message = "Audio · Space play/stop · Esc close".into();
            }
            Some(k) => {
                self.message = format!("Preview · {} — Esc close · j/k scroll", k.label());
            }
            None => {}
        }
        self.mode = Mode::Preview;
        Ok(())
    }

    pub fn clear_media_handles(&mut self) {
        if let Some(mut a) = self.preview_audio.take() {
            a.stop();
        }
        self.preview_image = None;
    }

    /// Begin reverse transform close (mode flips when anim settles).
    pub fn close_preview(&mut self) {
        if !self.preview.open {
            self.mode = Mode::Editor;
            return;
        }
        self.clear_media_handles();
        self.preview.close();
    }

    pub fn close_preview_immediate(&mut self) {
        self.clear_media_handles();
        self.preview.close_immediate();
        self.mode = Mode::Editor;
    }

    pub fn refresh_preview_if_open(&mut self) {
        if self.preview.open && !self.preview.closing {
            let text = self.buffer.text();
            let ext = self.file_extension();
            self.preview.rebuild(&text, ext.as_deref());
        }
    }

    /// Settle modes after panel/preview close animations complete.
    pub fn settle_anims(&mut self) {
        if self.scm.take_just_closed() {
            self.mode = Mode::Editor;
        }
        if self.preview.take_just_closed() {
            self.clear_media_handles();
            self.mode = Mode::Editor;
        }
    }

    /// Breadcrumb path segments for the current file (VS Code-style).
    pub fn breadcrumbs(&self) -> Vec<String> {
        let Some(ref path) = self.filename else {
            return vec!["untitled".into()];
        };
        let mut parts: Vec<String> = Vec::new();
        for c in path.components() {
            match c {
                std::path::Component::Normal(s) => {
                    parts.push(s.to_string_lossy().into_owned());
                }
                std::path::Component::RootDir => parts.push("/".into()),
                std::path::Component::Prefix(p) => {
                    parts.push(p.as_os_str().to_string_lossy().into_owned());
                }
                _ => {}
            }
        }
        // Keep last 4 segments for readability
        if parts.len() > 4 {
            let tail: Vec<_> = parts.into_iter().rev().take(4).collect::<Vec<_>>();
            let mut v: Vec<_> = tail.into_iter().rev().collect();
            v.insert(0, "…".into());
            v
        } else if parts.is_empty() {
            vec!["untitled".into()]
        } else {
            parts
        }
    }

    pub fn file_extension(&self) -> Option<String> {
        self.filename
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
    }

    pub fn file_name(&self) -> &str {
        self.filename
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
    }

    pub fn push_undo(&mut self) {
        self.undo_stack.push(self.buffer.snapshot());
        self.modified = true;
        if self.current_buffer < self.buffers.len() {
            self.buffers[self.current_buffer].modified = true;
        }
        // didChange is sent by sync_lsp_document (post-edit); notifying here
        // would push the *pre-edit* snapshot since push_undo runs first.
    }

    /// Push the current buffer to the LSP if it differs from what the server
    /// last saw. Frontends call this from their loop (throttled) and before
    /// position-based requests, so the server always answers against the
    /// post-edit document — including plain insert-mode typing, which never
    /// produced a didChange before.
    /// Whether the language server's copy of the current document is current —
    /// i.e. whether [`Self::sync_lsp_document`] would have anything to send.
    /// False right after an edit, true again once the sync has run.
    pub fn lsp_document_synced(&self) -> bool {
        if !self.lsp.server_running {
            return true;
        }
        let Some(path) = self.filename.as_ref() else {
            return true;
        };
        if !crate::lsp::has_server_for(&path.display().to_string()) {
            return true;
        }
        self.lsp_synced_path.as_ref() == Some(path)
            && self.lsp_synced_version == self.buffer.version()
    }

    pub fn sync_lsp_document(&mut self) {
        if !self.lsp.server_running {
            return;
        }
        let Some(path) = self.filename.clone() else {
            return;
        };
        let path_str = path.display().to_string();
        if !crate::lsp::has_server_for(&path_str) {
            return;
        }
        // Version gate: skip the O(file) join + hash entirely when the text
        // hasn't mutated since the last sync (this runs every ~5 frames).
        let path_changed = self.lsp_synced_path.as_ref() != Some(&path);
        if !path_changed && self.lsp_synced_version == self.buffer.version() {
            return;
        }
        let text = self.buffer.text();
        let hash = text_hash(&text);
        if path_changed || self.lsp_synced_hash != hash {
            self.lsp.notify_change(&path_str, &text);
            self.lsp_synced_path = Some(path);
            self.lsp_synced_hash = hash;
        }
        self.lsp_synced_version = self.buffer.version();
    }

    pub fn undo(&mut self) {
        let current = self.buffer.snapshot();
        if let Some(snap) = self.undo_stack.undo(current) {
            self.buffer.restore(&snap);
            self.message = String::from("UNDO");
        } else {
            self.message = String::from("Already at oldest change");
        }
    }

    pub fn redo(&mut self) {
        let current = self.buffer.snapshot();
        if let Some(snap) = self.undo_stack.redo(current) {
            self.buffer.restore(&snap);
            self.modified = true;
            self.message = String::from("REDO");
        } else {
            self.message = String::from("Already at newest change");
        }
    }




    pub fn store_yank(&mut self, text: String, linewise: bool) {
        self.registers.store(text.clone(), linewise);
        self.yank_buffer = Some(text);
    }





    pub fn request_references(&mut self) {
        self.sync_lsp_document();
        if let Some(ref path) = self.filename.clone() {
            let c = self.buffer.cursor();
            self.lsp
                .request_references(&path.display().to_string(), c.row, c.col);
            self.message = String::from("Finding references…");
        }
    }

    /// The current "find references" result: each location with a trimmed
    /// preview of its source line, plus whether the LSP has answered yet.
    /// The preview is pulled from the open buffer when the location is in the
    /// current file (so unsaved edits show), else read best-effort from disk
    /// (deduplicated per file). `(empty, true)` means resolved with zero refs.
    pub fn references_result(&self) -> (Vec<(crate::lsp::Location, String)>, bool) {
        use std::collections::HashMap;
        let ready = self.lsp.references_ready;
        let cur = self.filename.as_ref().map(|p| p.display().to_string());
        let mut disk: HashMap<String, Vec<String>> = HashMap::new();
        let mut out = Vec::with_capacity(self.lsp.pending_references.len());
        for loc in &self.lsp.pending_references {
            let preview = if Some(&loc.path) == cur.as_ref() {
                self.buffer.line(loc.row.min(self.buffer.line_count().saturating_sub(1)))
                    .trim()
                    .to_string()
            } else {
                let lines = disk.entry(loc.path.clone()).or_insert_with(|| {
                    std::fs::read_to_string(&loc.path)
                        .map(|s| s.lines().map(|l| l.to_string()).collect())
                        .unwrap_or_default()
                });
                lines.get(loc.row).map(|l| l.trim().to_string()).unwrap_or_default()
            };
            out.push((loc.clone(), preview));
        }
        (out, ready)
    }

    pub fn request_rename(&mut self, new_name: &str) {
        if new_name.is_empty() {
            self.message = String::from("Empty name");
            return;
        }
        self.sync_lsp_document();
        if let Some(ref path) = self.filename.clone() {
            let c = self.buffer.cursor();
            self.lsp
                .request_rename(&path.display().to_string(), c.row, c.col, new_name);
            self.message = format!("Renaming to {}…", new_name);
        }
    }

    /// Copy current visual selection, or the current line in Normal mode, to
    /// the system clipboard (Cmd+C / Ctrl+C style).
    pub fn clipboard_copy(&mut self) {
        // Gate on the SELECTION, not the mode: a GUI mouse/keyboard selection
        // lives in `self.sel` and never enters vim Visual mode.
        if self.has_selection() {
            self.yank_selection();
            // yank_selection already store_yank → system
            self.message = String::from("Copied to clipboard");
            return;
        }
        // Normal / Insert: copy current line
        let line = self.buffer.line(self.buffer.cursor.row).to_string();
        let text = if line.ends_with('\n') {
            line
        } else {
            format!("{}\n", line)
        };
        self.store_yank(text, true);
        self.message = String::from("Copied line to clipboard");
    }

    /// Paste from system clipboard (Cmd+V / Ctrl+V style) into the buffer.
    pub fn clipboard_paste(&mut self) {
        // Force system clipboard path
        let Some(val) = self.registers.load_for_put() else {
            self.message = String::from("Clipboard empty");
            return;
        };
        // Paste replaces the selection and lands at the caret — the GUI
        // contract. There is no vim "put after" branch any more.
        self.gui_insert_text(&val.text.replace('\r', ""));
        self.update_scroll();
        self.message = String::from("Pasted from clipboard");
    }

    /// Insert pasted text at the cursor verbatim (no auto-indent — a bracketed
    /// paste from the outer terminal should land exactly as-is). Used by the
    /// TUI's `Event::Paste` handler in editor Insert mode.
    pub fn paste_text_at_cursor(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.push_undo();
        let clean = text.replace('\r', "");
        self.buffer.insert_str(&clean);
        self.update_scroll();
        self.sync_lsp_document();
        self.message = String::from("Pasted");
    }

    /// Select entire buffer (⌘A / context menu). Drives the Selection model —
    /// the old version set vim's `visual_anchor`, which nothing reads now.
    pub fn select_all(&mut self) {
        self.select_all_gui();
        self.completions.deactivate();
        self.message = String::from("Selected all");
    }

    /// Open editor right-click context menu at screen coords.
    pub fn open_editor_ctx(&mut self, x: u16, y: u16) {
        let mut items = vec![
            EditorCtxItem::Cut,
            EditorCtxItem::Copy,
            EditorCtxItem::Paste,
            EditorCtxItem::SelectAll,
            EditorCtxItem::Undo,
            EditorCtxItem::Redo,
        ];
        if self.filename.is_some() {
            items.push(EditorCtxItem::GoToDefinition);
            items.push(EditorCtxItem::FormatDocument);
        }
        items.push(EditorCtxItem::CommandPalette);
        self.editor_ctx = Some(EditorContextMenu {
            x,
            y,
            sel: 0,
            items,
        });
        self.message = "Menu · j/k · Enter · Esc".into();
    }

    pub fn close_editor_ctx(&mut self) {
        self.editor_ctx = None;
    }

    /// Run selected editor context-menu action.
    pub fn run_editor_ctx_action(&mut self) -> Result<String, String> {
        let menu = self
            .editor_ctx
            .clone()
            .ok_or_else(|| "No menu".to_string())?;
        let item = *menu
            .items
            .get(menu.sel)
            .ok_or_else(|| "No item".to_string())?;
        self.editor_ctx = None;
        match item {
            EditorCtxItem::Cut => {
                if self.has_selection() {
                    self.delete_selection();
                    Ok("Cut".into())
                } else {
                    self.delete_line();
                    Ok(self.message.clone())
                }
            }
            EditorCtxItem::Copy => {
                self.clipboard_copy();
                Ok(self.message.clone())
            }
            EditorCtxItem::Paste => {
                self.clipboard_paste();
                Ok(self.message.clone())
            }
            EditorCtxItem::SelectAll => {
                self.select_all();
                Ok("Select all".into())
            }
            EditorCtxItem::Undo => {
                self.undo();
                Ok(self.message.clone())
            }
            EditorCtxItem::Redo => {
                self.redo();
                Ok(self.message.clone())
            }
            EditorCtxItem::GoToDefinition => {
                let path = self
                    .filename
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .ok_or_else(|| "No file".to_string())?;
                let c = self.buffer.cursor();
                self.push_jump();
                self.sync_lsp_document();
                self.lsp.request_definition(&path, c.row, c.col);
                Ok("Requested definition…".into())
            }
            EditorCtxItem::FormatDocument => {
                self.format_document();
                Ok(self.message.clone())
            }
            EditorCtxItem::CommandPalette => {
                self.open_command_palette();
                Ok("Command palette".into())
            }
        }
    }

    /// Snapshot current location for jumplist (call before big moves).
    pub fn push_jump(&mut self) {
        self.jumps.push(Jump {
            pos: self.buffer.cursor(),
            scroll: self.scroll,
            path: self.filename.clone(),
        });
    }

    pub fn jump_back(&mut self) {
        let current = Jump {
            pos: self.buffer.cursor(),
            scroll: self.scroll,
            path: self.filename.clone(),
        };
        if let Some(j) = self.jumps.back(current) {
            self.apply_jump(j);
            self.message = String::from("Jump ←");
        } else {
            self.message = String::from("Already at oldest jump");
        }
    }

    pub fn jump_forward(&mut self) {
        if let Some(j) = self.jumps.forward() {
            self.apply_jump(j);
            self.message = String::from("Jump →");
        } else {
            self.message = String::from("Already at newest jump");
        }
    }

    fn apply_jump(&mut self, j: Jump) {
        // Switch buffer if path differs and is open
        if let Some(ref path) = j.path {
            if self.filename.as_ref() != Some(path) {
                let path_str = path.display().to_string();
                // Prefer existing tab without re-pushing jump
                self.open_new_tab(&path_str);
            }
        }
        self.buffer.cursor = j.pos;
        self.buffer.clamp_col();
        self.scroll = j.scroll;
        self.update_scroll();
    }








    pub fn goto_line(&mut self, line_1based: usize) {
        self.scroll_intent = ScrollIntent::Navigate;
        self.push_jump();
        let target = line_1based.saturating_sub(1).min(self.buffer.line_count().saturating_sub(1));
        self.buffer.cursor.row = target;
        self.buffer.move_to_line_start();
        self.update_scroll();
        self.message = format!("Line {}", target + 1);
    }

    pub fn search_word_under_cursor_backward(&mut self) {
        let word = self.word_under_cursor();
        if word.is_empty() {
            self.message = String::from("No word under cursor");
            return;
        }
        self.push_jump();
        self.search_pattern = Some(word.clone());
        self.search_forward = false;
        self.recompute_search(&word, true);
        if self.search_matches.len() > 1 {
            self.search_prev();
        } else if self.search_matches.is_empty() {
            self.message = format!("Pattern not found: {}", word);
        } else {
            self.message = format!("?{}/  1/1", word);
        }
    }

    pub fn quit(&mut self) {
        // Persist or discard undo history for every open file (undo_caching).
        self.save_state_to_tab();
        let caching = self.undo_caching;
        for tab in &mut self.buffers {
            if tab.filename.is_some() {
                let text = tab.buffer.text();
                tab.undo_stack.finish(caching, &text);
            }
        }
        // Detached so quitting is instant; the hook keeps running after exit.
        crate::hooks::run_hooks_detached(
            &self.hooks,
            crate::hooks::HookEvent::Quit,
            self.filename.as_deref(),
        );
        self.save_session();
        self.running = false;
    }


    /// Return the keyboard to the editor and drop transient overlays.
    /// (Was vim's "leave whatever mode you are in"; there is one editor state
    /// now, so this is just the dismiss.)
    pub fn focus_editor(&mut self) {
        self.mode = Mode::Editor;
        self.completions.deactivate();
        self.palette.close();
        self.hover_text = None;
        self.message = String::new();
    }

    pub fn clear_multi_cursors(&mut self) {
        if self.multi.is_active() {
            self.multi.clear();
            self.message = "Multi-cursor cleared".into();
        }
    }

    /// Ctrl+D — add next occurrence of word under primary cursor.
    pub fn multi_cursor_add_next(&mut self) {
        let primary = self.buffer.cursor();
        let Some((_, end, word)) = crate::multi_cursor::word_at(&self.buffer, primary) else {
            self.message = "No word under cursor".into();
            return;
        };
        // Search after the last multi-cursor (or after primary word)
        let from = self
            .multi
            .extras
            .last()
            .copied()
            .map(|p| Position {
                row: p.row,
                col: p.col + word.chars().count(),
            })
            .unwrap_or(end);
        if let Some(pos) = crate::multi_cursor::find_next(&self.buffer, &word, from) {
            self.multi.add(primary, pos);
            self.message = format!("cursors: {}", self.multi.count(primary));
        } else {
            self.message = "No more matches".into();
        }
    }

    /// Ctrl+Alt+Down / `]c` — column cursor below primary.
    pub fn multi_cursor_add_below(&mut self) {
        let p = self.buffer.cursor();
        if p.row + 1 >= self.buffer.line_count() {
            self.message = "No line below".into();
            return;
        }
        let mut np = Position {
            row: p.row + 1,
            col: p.col,
        };
        let max = self.buffer.line(np.row).chars().count();
        if np.col > max {
            np.col = max;
        }
        self.multi.add(p, np);
        self.message = format!("cursors: {}", self.multi.count(p));
    }

    pub fn multi_cursor_add_above(&mut self) {
        let p = self.buffer.cursor();
        if p.row == 0 {
            self.message = "No line above".into();
            return;
        }
        let mut np = Position {
            row: p.row - 1,
            col: p.col,
        };
        let max = self.buffer.line(np.row).chars().count();
        if np.col > max {
            np.col = max;
        }
        self.multi.add(p, np);
        self.message = format!("cursors: {}", self.multi.count(p));
    }

    /// Apply insert-mode edit at every cursor (bottom→top so offsets stay valid).
    pub fn multi_insert_char(&mut self, ch: char) {
        if !self.multi.is_active() {
            self.buffer.insert_char(ch);
            return;
        }
        let primary = self.buffer.cursor();
        let mut all = self.multi.all(primary);
        all.sort_by(|a, b| b.row.cmp(&a.row).then(b.col.cmp(&a.col)));
        let mut updated = Vec::with_capacity(all.len());
        for pos in all {
            self.buffer.cursor = pos;
            self.buffer.insert_char(ch);
            updated.push(self.buffer.cursor);
        }
        updated.sort_by(|a, b| a.row.cmp(&b.row).then(a.col.cmp(&b.col)));
        updated.dedup();
        if let Some(first) = updated.first().copied() {
            self.buffer.cursor = first;
            self.multi.set_from_all(updated);
        }
        self.multi.clamp_all(&self.buffer);
        self.modified = true;
    }

    pub fn multi_backspace(&mut self) {
        if !self.multi.is_active() {
            self.buffer.backspace();
            return;
        }
        let primary = self.buffer.cursor();
        let mut all = self.multi.all(primary);
        all.sort_by(|a, b| b.row.cmp(&a.row).then(b.col.cmp(&a.col)));
        let mut updated = Vec::with_capacity(all.len());
        for pos in all {
            self.buffer.cursor = pos;
            self.buffer.backspace();
            updated.push(self.buffer.cursor);
        }
        updated.sort_by(|a, b| a.row.cmp(&b.row).then(a.col.cmp(&b.col)));
        updated.dedup();
        if let Some(first) = updated.first().copied() {
            self.buffer.cursor = first;
            self.multi.set_from_all(updated);
        }
        self.multi.clamp_all(&self.buffer);
        self.modified = true;
    }

    pub fn multi_delete_char(&mut self) {
        if !self.multi.is_active() {
            self.buffer.delete_char_at_cursor();
            return;
        }
        let primary = self.buffer.cursor();
        let mut all = self.multi.all(primary);
        all.sort_by(|a, b| b.row.cmp(&a.row).then(b.col.cmp(&a.col)));
        let mut updated = Vec::with_capacity(all.len());
        for pos in all {
            self.buffer.cursor = pos;
            self.buffer.delete_char_at_cursor();
            updated.push(self.buffer.cursor);
        }
        updated.sort_by(|a, b| a.row.cmp(&b.row).then(a.col.cmp(&b.col)));
        updated.dedup();
        if let Some(first) = updated.first().copied() {
            self.buffer.cursor = first;
            self.multi.set_from_all(updated);
        }
        self.multi.clamp_all(&self.buffer);
        self.modified = true;
    }

    pub fn multi_move_each(&mut self, f: impl Fn(&mut crate::buffer::Buffer)) {
        if !self.multi.is_active() {
            f(&mut self.buffer);
            return;
        }
        let primary = self.buffer.cursor();
        let all = self.multi.all(primary);
        let mut updated = Vec::with_capacity(all.len());
        for pos in all {
            self.buffer.cursor = pos;
            f(&mut self.buffer);
            updated.push(self.buffer.cursor);
        }
        updated.sort_by(|a, b| a.row.cmp(&b.row).then(a.col.cmp(&b.col)));
        updated.dedup();
        if let Some(first) = updated.first().copied() {
            self.buffer.cursor = first;
            self.multi.set_from_all(updated);
        }
        self.multi.clamp_all(&self.buffer);
    }

    pub fn multi_newline(&mut self) {
        if !self.multi.is_active() {
            let row = self.buffer.cursor.row;
            self.buffer.insert_newline_smart("    ");
            if let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) {
                // Newline splits after `row` → shift BPs on later lines +1
                self.dap.shift_breakpoints(&path, row, 1);
            }
            return;
        }
        let primary = self.buffer.cursor();
        let mut all = self.multi.all(primary);
        all.sort_by(|a, b| b.row.cmp(&a.row).then(b.col.cmp(&a.col)));
        let mut updated = Vec::with_capacity(all.len());
        for pos in all {
            self.buffer.cursor = pos;
            self.buffer.insert_newline_smart("    ");
            updated.push(self.buffer.cursor);
        }
        updated.sort_by(|a, b| a.row.cmp(&b.row).then(a.col.cmp(&b.col)));
        updated.dedup();
        if let Some(first) = updated.first().copied() {
            self.buffer.cursor = first;
            self.multi.set_from_all(updated);
        }
        self.multi.clamp_all(&self.buffer);
        self.modified = true;
    }

    pub fn open_file_palette(&mut self) {
        let root = self
            .filename
            .as_ref()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| env::current_dir().unwrap_or_default());
        self.palette.open_files(&root);
        self.mode = Mode::Palette;
        self.message = String::from("Open file — type to filter, Enter open, Esc cancel");
    }

    pub fn open_command_palette(&mut self) {
        self.palette.open_commands();
        self.mode = Mode::Palette;
        self.message = String::from("Commands — type to filter, Enter run, Esc cancel");
    }

    pub fn open_problems_palette(&mut self) {
        self.palette.open_problems(&self.lsp.diagnostics);
        self.mode = Mode::Palette;
        self.message = format!("Problems — {} items", self.lsp.diagnostics.len());
    }

    pub fn execute_palette_selection(&mut self) {
        let action = self.palette.selected_action().cloned();
        self.palette.close();
        self.mode = Mode::Editor;
        let Some(action) = action else {
            return;
        };
        match action {
            PaletteAction::OpenFile(path) => {
                self.open_new_tab(&path.display().to_string());
            }
            PaletteAction::Goto { row, col } => {
                self.push_jump();
                self.buffer.cursor.row = row.min(self.buffer.line_count().saturating_sub(1));
                self.buffer.cursor.col = col;
                self.buffer.clamp_col();
                self.update_scroll();
                self.message = format!("Jumped to {}:{}", row + 1, col + 1);
            }
            PaletteAction::GotoFile { path, row, col } => {
                self.goto_file_location(&path.display().to_string(), row, col);
            }
            PaletteAction::CodeAction(i) => {
                self.apply_code_action(i);
            }
            PaletteAction::Command(id) => self.run_palette_command(id),
        }
    }

    /// Jump to path:line:col (opens tab if needed).
    pub fn goto_file_location(&mut self, path: &str, row: usize, col: usize) {
        self.push_jump();
        let cur = self
            .filename
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if cur != path {
            self.open_new_tab(path);
        }
        self.buffer.cursor.row = row.min(self.buffer.line_count().saturating_sub(1));
        let line = self.buffer.line(self.buffer.cursor.row);
        // col may already be char index from search; clamp only
        self.buffer.cursor.col = col.min(line.chars().count());
        self.buffer.clamp_col();
        self.update_scroll();
        self.sync_split_from_active();
        self.message = format!("→ {}:{}:{}", path, row + 1, col + 1);
    }

    pub fn project_root(&self) -> std::path::PathBuf {
        if let Some(ref f) = self.filename {
            if let Some(parent) = f.parent() {
                // walk up for Cargo.toml / package.json / .git
                let mut cur = parent.to_path_buf();
                loop {
                    if cur.join("Cargo.toml").exists()
                        || cur.join("package.json").exists()
                        || cur.join(".git").exists()
                        || cur.join("go.mod").exists()
                        || cur.join("pyproject.toml").exists()
                    {
                        return cur;
                    }
                    if !cur.pop() {
                        break;
                    }
                }
                return parent.to_path_buf();
            }
        }
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    }

    pub fn open_workspace_search(&mut self) {
        let root = self.project_root();
        self.workspace_search.open_at(root);
        self.mode = Mode::WorkspaceSearch;
        self.message = String::from("Find in files — type pattern, Enter open hit");
    }

    pub fn open_document_symbols(&mut self) {
        let path = self.filename.as_ref().map(|p| p.display().to_string());
        if let Some(path) = path {
            self.sync_lsp_document();
            self.lsp.request_document_symbols(&path);
            self.message = String::from("Loading document symbols…");
        } else {
            self.message = String::from("No file for symbols");
        }
    }

    pub fn open_workspace_symbols(&mut self) {
        if !self.lsp.server_running {
            self.message = String::from("LSP not running");
            return;
        }
        self.sync_lsp_document();
        self.lsp.request_workspace_symbols("");
        self.message = String::from("Loading workspace symbols…");
    }

    pub fn apply_pending_symbols(&mut self) {
        let symbols = std::mem::take(&mut self.lsp.pending_symbols);
        if symbols.is_empty() {
            return;
        }
        let cur_path = self
            .filename
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let items: Vec<crate::palette::PaletteItem> = symbols
            .into_iter()
            .map(|s| {
                let path = if s.path.is_empty() {
                    cur_path.clone()
                } else {
                    s.path.clone()
                };
                let detail = if s.detail.is_empty() {
                    format!("{}  L{}", s.kind, s.row + 1)
                } else {
                    format!("{}  {}  L{}", s.kind, s.detail, s.row + 1)
                };
                crate::palette::PaletteItem {
                    label: s.name,
                    detail,
                    action: PaletteAction::GotoFile {
                        path: std::path::PathBuf::from(path),
                        row: s.row,
                        col: s.col,
                    },
                }
            })
            .collect();
        self.palette.open_symbols(items);
        self.mode = Mode::Palette;
        self.message = format!("Symbols — {} items", self.palette.items.len());
    }

    pub fn request_peek_definition(&mut self) {
        let path = self.filename.as_ref().map(|p| p.display().to_string());
        if let Some(path) = path {
            self.sync_lsp_document();
            let c = self.buffer.cursor();
            self.lsp.request_peek_definition(&path, c.row, c.col);
            self.message = String::from("Peek definition…");
        }
    }

    pub fn open_peek_at(&mut self, path: &str, row: usize, col: usize) {
        let fallback = if self.filename.as_ref().map(|p| p.display().to_string()).as_deref()
            == Some(path)
        {
            Some(self.buffer.text())
        } else {
            None
        };
        self.peek.open_at(
            std::path::PathBuf::from(path),
            row,
            col,
            fallback.as_deref(),
            8,
        );
        self.message = format!(
            "Peek {} — Enter open · Esc dismiss",
            self.peek.path.display()
        );
    }

    pub fn promote_peek(&mut self) {
        if !self.peek.open {
            return;
        }
        let path = self.peek.path.display().to_string();
        let row = self.peek.target_row;
        let col = self.peek.target_col;
        self.peek.close();
        self.goto_file_location(&path, row, col);
    }

    // ── Splits ──────────────────────────────────────────

    pub fn split_vertical(&mut self) {
        self.open_split_kind(crate::split::SplitKind::Vertical, "Vertical");
    }

    pub fn split_horizontal(&mut self) {
        self.open_split_kind(crate::split::SplitKind::Horizontal, "Horizontal");
    }

    fn open_split_kind(&mut self, kind: crate::split::SplitKind, label: &str) {
        use crate::split::SplitAdd;
        self.save_state_to_tab();
        self.sync_split_from_active();
        let cur = (self.buffer.cursor.row, self.buffer.cursor.col);
        let r = self
            .split
            .open_split(kind, self.current_buffer, self.scroll, cur);
        self.message = match r {
            SplitAdd::Opened => {
                format!("{label} split · Ctrl+W w cycle · Ctrl+W q close")
            }
            SplitAdd::Added => format!(
                "Pane added ({}) · Ctrl+W w cycle · Ctrl+W q close",
                self.split.pane_count()
            ),
            SplitAdd::Full => format!("Max {} panes", crate::split::MAX_PANES),
            SplitAdd::MixedKind => {
                "Already split the other way — Ctrl+W q panes first".into()
            }
        };
    }

    /// Vim `C-w q`: close the *focused* pane; neighbors survive (the split
    /// collapses once one pane remains).
    pub fn close_split(&mut self) {
        if !self.split.is_split() {
            return;
        }
        let closed = self
            .split
            .focus
            .min(self.split.panes.len().saturating_sub(1));
        // Pane terminal dies with its pane; higher indices shift down.
        if self.terminal.open && self.terminal.full_panel {
            match self.terminal.pane_bound {
                Some(b) if b == closed => {
                    self.terminal.open = false;
                    self.terminal.full_panel = false;
                    self.terminal.pane_bound = None;
                    self.terminal.close_confirm = false;
                    self.terminal.shutdown();
                }
                Some(b) if b > closed => {
                    self.terminal.pane_bound = Some(b - 1);
                }
                _ => {}
            }
        }
        let survivor = self.split.remove_focused();
        if self.split.is_split() {
            self.apply_focused_pane();
            self.message = format!("Pane closed · {} left", self.split.pane_count());
            return;
        }
        // Collapsed to a single view: adopt the survivor snapshot.
        if self.terminal.pane_bound.is_some() {
            self.terminal.pane_bound = None; // continues as the full-main window
        }
        if let Some(p) = survivor {
            if p.tab_index != self.current_buffer && p.tab_index < self.buffers.len() {
                self.save_state_to_tab();
                self.current_buffer = p.tab_index;
                self.restore_state_from_tab();
                self.lsp_restart_for_current();
                self.refresh_git();
            }
            let max_row = self.buffer.line_count().saturating_sub(1);
            self.buffer.cursor.row = p.cursor.0.min(max_row);
            self.buffer.cursor.col = p.cursor.1;
            self.buffer.clamp_col();
            self.scroll = p.scroll;
            self.update_scroll();
        }
        self.message = String::from("Pane closed");
    }

    /// Vim `C-w h/j/k/l`: directional focus along the split axis (steps one
    /// pane per press; works for any pane count).
    pub fn focus_dir(&mut self, dir: char) {
        if !self.split.is_split() {
            return;
        }
        let vertical = self.split.kind == crate::split::SplitKind::Vertical;
        let delta: isize = match (vertical, dir) {
            (true, 'h') | (false, 'k') => -1,
            (true, 'l') | (false, 'j') => 1,
            _ => return, // off-axis — splits are single-direction
        };
        let n = self.split.panes.len() as isize;
        let cur = self.split.focus.min(self.split.panes.len().saturating_sub(1)) as isize;
        let next = (cur + delta).clamp(0, n - 1) as usize;
        if next == cur as usize {
            return;
        }
        self.sync_split_from_active();
        self.split.set_focus(next);
        self.apply_focused_pane();
        self.message = format!("Pane {}", next + 1);
    }

    /// Persist active buffer scroll into the focused pane, then switch focus.
    pub fn focus_other_pane(&mut self) {
        if !self.split.is_split() {
            return;
        }
        self.sync_split_from_active();
        self.split.focus_other();
        self.apply_focused_pane();
        self.message = format!("Pane {}", self.split.focus + 1);
    }

    pub fn focus_pane(&mut self, idx: usize) {
        if !self.split.is_split() {
            return;
        }
        self.sync_split_from_active();
        self.split.set_focus(idx);
        self.apply_focused_pane();
    }

    /// Write current scroll/tab/hscroll into focused pane slot.
    pub fn sync_split_from_active(&mut self) {
        if !self.split.is_split() {
            return;
        }
        let cur = (self.buffer.cursor.row, self.buffer.cursor.col);
        let p = self.split.focused_pane_mut();
        p.tab_index = self.current_buffer;
        p.scroll = self.scroll;
        p.hscroll = self.hscroll;
        p.cursor = cur;
    }

    /// Keep focused pane's scroll/hscroll mirrors live (call after wheel / pan).
    pub fn sync_focused_pane_viewport(&mut self) {
        if !self.split.is_split() {
            return;
        }
        let p = self.split.focused_pane_mut();
        p.scroll = self.scroll;
        p.hscroll = self.hscroll;
    }

    /// Load focused pane's tab into the active editor.
    pub fn apply_focused_pane(&mut self) {
        if !self.split.is_split() {
            return;
        }
        let pane = self.split.focused_pane().clone();
        if pane.tab_index != self.current_buffer && pane.tab_index < self.buffers.len() {
            self.save_state_to_tab();
            self.current_buffer = pane.tab_index;
            self.restore_state_from_tab();
            self.lsp_restart_for_current();
            self.refresh_git();
        }
        // Per-pane cursor (clamped — the buffer may have changed underneath).
        let max_row = self.buffer.line_count().saturating_sub(1);
        self.buffer.cursor.row = pane.cursor.0.min(max_row);
        self.buffer.cursor.col = pane.cursor.1;
        self.buffer.clamp_col();
        self.scroll = pane.scroll;
        self.hscroll = if self.wrap_lines { 0 } else { pane.hscroll };
        self.update_scroll();
    }

    /// Assign a different tab to the focused pane (e.g. after gt in a split).
    pub fn sync_focused_pane_tab(&mut self) {
        if self.split.is_split() {
            let cur = (self.buffer.cursor.row, self.buffer.cursor.col);
            let p = self.split.focused_pane_mut();
            p.tab_index = self.current_buffer;
            p.scroll = self.scroll;
            p.hscroll = self.hscroll;
            p.cursor = cur;
        }
    }

    fn run_palette_command(&mut self, id: &str) {
        match id {
            "noop" => {}
            "save" => self.save_file(),
            "wq" => {
                self.save_file();
                if !self.modified {
                    self.quit();
                }
            }
            "quit" => {
                if self.modified {
                    self.message =
                        String::from("Unsaved changes. Use Save or Force quit.");
                } else {
                    self.quit();
                }
            }
            "quit!" => self.quit(),
            "explorer" => {
                if self.explorer.open {
                    self.explorer.close();
                } else {
                    self.explorer.toggle_at(self.filename.as_ref());
                    self.mode = Mode::Explorer;
                }
            }
            "scm" => self.toggle_scm(),
            "git" | "git_workbench" => self.open_git_workbench(),
            "settings" => self.open_settings(),
            "preview" => self.toggle_preview(),
            "terminal" => self.toggle_terminal_side(),
            "terminal_full" => self.toggle_terminal_full(),
            "tab_next" => self.next_tab(),
            "tab_prev" => self.prev_tab(),
            "tab_close" => self.close_current_tab(),
            "problems" => self.open_problems_palette(),
            "files" => self.open_file_palette(),
            "workspace_find" => self.open_workspace_search(),
            "symbols" => self.open_document_symbols(),
            "workspace_symbols" => self.open_workspace_symbols(),
            "split_v" => self.split_vertical(),
            "split_h" => self.split_horizontal(),
            "split_close" => self.close_split(),
            "lsp_def" => {
                let path = self.filename.as_ref().map(|p| p.display().to_string());
                if let Some(path) = path {
                    let c = self.buffer.cursor();
                    self.push_jump();
                    self.sync_lsp_document();
                    self.lsp.request_definition(&path, c.row, c.col);
                    self.message = String::from("Requested definition…");
                }
            }
            "lsp_peek" => self.request_peek_definition(),
            "format" => self.format_document(),
            "code_action" => self.request_code_actions(),
            id if id.starts_with("theme:") => {
                let name = &id[6..];
                if let Some(t) = theme::find(name) {
                    self.theme = t;
                    config::save_theme(t.name);
                    self.message = format!("Theme: {}", t.name);
                }
            }
            _ => {
                self.message = format!("Unknown command: {}", id);
            }
        }
    }

    pub fn diag_next(&mut self) {
        if self.lsp.diagnostics.is_empty() {
            self.message = String::from("No diagnostics");
            return;
        }
        let cur = self.buffer.cursor();
        let mut diags = self.lsp.diagnostics.clone();
        diags.sort_by_key(|d| (d.row, d.col_start));
        let next = diags
            .iter()
            .find(|d| d.row > cur.row || (d.row == cur.row && d.col_start > cur.col))
            .or_else(|| diags.first());
        if let Some(d) = next {
            self.push_jump();
            self.buffer.cursor.row = d.row;
            self.buffer.cursor.col = d.col_start;
            self.buffer.clamp_col();
            self.update_scroll();
            self.message = format!("[{:?}] {}", d.severity, d.message);
        }
    }

    pub fn diag_prev(&mut self) {
        if self.lsp.diagnostics.is_empty() {
            self.message = String::from("No diagnostics");
            return;
        }
        let cur = self.buffer.cursor();
        let mut diags = self.lsp.diagnostics.clone();
        diags.sort_by_key(|d| (d.row, d.col_start));
        let prev = diags
            .iter()
            .rev()
            .find(|d| d.row < cur.row || (d.row == cur.row && d.col_start < cur.col))
            .or_else(|| diags.last());
        if let Some(d) = prev {
            self.push_jump();
            self.buffer.cursor.row = d.row;
            self.buffer.cursor.col = d.col_start;
            self.buffer.clamp_col();
            self.update_scroll();
            self.message = format!("[{:?}] {}", d.severity, d.message);
        }
    }

    /// Jump to next git gutter change (from `git diff HEAD`).
    pub fn git_change_next(&mut self) {
        self.refresh_git();
        if self.git.signs.is_empty() {
            self.message = String::from("No git changes");
            return;
        }
        let cur = self.buffer.cursor.row;
        let mut rows: Vec<usize> = self.git.signs.keys().copied().collect();
        rows.sort_unstable();
        let next = rows.iter().copied().find(|r| *r > cur).or_else(|| rows.first().copied());
        if let Some(row) = next {
            self.push_jump();
            self.buffer.cursor.row = row;
            self.buffer.move_to_line_start();
            self.update_scroll();
            let sign = self.git.sign_at(row).map(|s| format!("{s:?}")).unwrap_or_default();
            self.message = format!("Git change · L{} · {sign}", row + 1);
        }
    }

    /// Jump to previous git gutter change.
    pub fn git_change_prev(&mut self) {
        self.refresh_git();
        if self.git.signs.is_empty() {
            self.message = String::from("No git changes");
            return;
        }
        let cur = self.buffer.cursor.row;
        let mut rows: Vec<usize> = self.git.signs.keys().copied().collect();
        rows.sort_unstable();
        let prev = rows
            .iter()
            .rev()
            .copied()
            .find(|r| *r < cur)
            .or_else(|| rows.last().copied());
        if let Some(row) = prev {
            self.push_jump();
            self.buffer.cursor.row = row;
            self.buffer.move_to_line_start();
            self.update_scroll();
            let sign = self.git.sign_at(row).map(|s| format!("{s:?}")).unwrap_or_default();
            self.message = format!("Git change · L{} · {sign}", row + 1);
        }
    }

    /// Force-reload current file from disk (discards local unsaved edits).
    pub fn reload_from_disk(&mut self) {
        let Some(path) = self.filename.clone() else {
            self.message = String::from("No file to reload");
            return;
        };
        let path_s = path.display().to_string();
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let cursor = self.buffer.cursor();
                let scroll = self.scroll;
                self.buffer = Buffer::from_string(&content);
                self.buffer.cursor.row =
                    cursor.row.min(self.buffer.line_count().saturating_sub(1));
                self.buffer.cursor.col = cursor.col;
                self.buffer.clamp_col();
                self.scroll = scroll.min(self.buffer.line_count().saturating_sub(1));
                self.modified = false;
                self.record_mtime();
                self.undo_stack = UndoStack::new();
                self.undo_stack.push(self.buffer.snapshot());
                if let Some(p) = self.filename.clone() {
                    let text = self.buffer.text();
                    self.undo_stack
                        .attach_file(&p, self.undo_caching, &text);
                }
                self.rebuild_folds();
                self.refresh_git();
                self.lsp_restart_for_current();
                self.sync_lsp_document();
                self.message = format!("↻ Reloaded {path_s}");
            }
            Err(e) => {
                self.message = format!("Reload failed: {e}");
            }
        }
    }

    /// Run a simple git remote command (fetch / pull / push) from workspace root.
    /// Network git ops run in the background (workbench runner + spinner);
    /// the result lands in the status line via `poll_loading` in the frontend.
    pub fn git_remote(&mut self, action: &str) {
        use crate::git_workbench::RemoteAction;
        if self.git_wb.root.is_none() {
            let hint = self.filename.as_deref();
            self.git_wb.root = crate::git_ops::find_git_root(hint);
        }
        if self.git_wb.root.is_none() {
            self.message = String::from("Not a git repository");
            return;
        }
        let act = match action {
            "fetch" => RemoteAction::Fetch,
            "pull" => RemoteAction::Pull,
            "push" => RemoteAction::Push,
            _ => {
                self.message = format!("unknown git action: {action}");
                return;
            }
        };
        self.message = self.git_wb.remote_action(act);
    }

    pub fn toggle_relative_number(&mut self) {
        self.relative_number = !self.relative_number;
        // Persist like theme changes so `SPC t r` survives restart.
        let mut cfg = config::load();
        cfg.relative_number = self.relative_number;
        config::save(&cfg);
        self.message = if self.relative_number {
            "relative_number on (saved)".into()
        } else {
            "relative_number off (saved)".into()
        };
    }

    pub fn toggle_inlay_hints(&mut self) {
        self.inlay_hints_enabled = !self.inlay_hints_enabled;
        self.message = if self.inlay_hints_enabled {
            "inlay hints on".into()
        } else {
            "inlay hints off".into()
        };
    }


    /// Switch to tab index if it exists (0-based).
    pub fn goto_tab(&mut self, idx: usize) {
        if idx < self.buffers.len() {
            self.save_state_to_tab();
            self.current_buffer = idx;
            self.restore_state_from_tab();
            self.message = format!("Tab {}", idx + 1);
        } else {
            self.message = format!("No tab {}", idx + 1);
        }
    }

    pub fn request_hover(&mut self) {
        self.sync_lsp_document();
        if let Some(ref path) = self.filename {
            let c = self.buffer.cursor();
            self.lsp
                .request_hover(&path.display().to_string(), c.row, c.col);
            self.message = String::from("Hover…");
        }
    }

    /// Select the word under the cursor. Delegates to the Selection model.
    pub fn select_word_under_cursor(&mut self) {
        let pos = self.buffer.cursor();
        self.select_word_gui(pos);
    }





    pub fn close_xlc(&mut self) {
        self.mode = Mode::Editor;
    }

    /// Open the incremental find bar (⌘F). This is a GUI panel that owns the
    /// keyboard while it is up, not a vim mode — `Mode::Search` sits with
    /// Palette and Settings, and the `/` `?` keys that used to open it are gone.
    pub fn enter_search(&mut self) {
        self.enter_search_dir(true);
    }

    pub fn enter_search_backward(&mut self) {
        self.enter_search_dir(false);
    }

    fn enter_search_dir(&mut self, forward: bool) {
        self.completions.deactivate();
        self.search_forward = forward;
        self.search_origin = Some(self.buffer.cursor());
        self.search_scroll_origin = self.scroll;
        self.search_pattern_backup = self.search_pattern.clone();
        self.search_input.clear();
        self.mode = Mode::Search;
        self.message = String::from("Find — Enter accept · Esc cancel · ↑↓ cycle");
    }

    /// Pattern currently used for highlighting (live input or committed).
    pub fn active_search_pattern(&self) -> Option<&str> {
        if self.mode == Mode::Search {
            if self.search_input.is_empty() {
                None
            } else {
                Some(self.search_input.as_str())
            }
        } else {
            self.search_pattern.as_deref()
        }
    }


    /// Commit live search input as the new pattern and leave Search mode.
    pub fn commit_search(&mut self) {
        let pattern = self.search_input.clone();
        if pattern.is_empty() {
            // Empty Enter reuses previous pattern (vim-like).
            if let Some(prev) = self.search_pattern.clone() {
                self.push_jump();
                self.recompute_search(&prev, false);
                if self.search_matches.is_empty() {
                    self.message = format!("Pattern not found: {}", prev);
                } else {
                    self.search_next();
                    self.message = format!(
                        "/{}/  {}/{}",
                        prev,
                        self.search_current + 1,
                        self.search_matches.len()
                    );
                }
            } else {
                self.message = String::from("No previous search pattern");
            }
        } else {
            self.push_jump();
            self.search_pattern = Some(pattern.clone());
            self.recompute_search(&pattern, true);
            if self.search_matches.is_empty() {
                self.message = format!("Pattern not found: {}", pattern);
            } else {
                let slash = if self.search_forward { '/' } else { '?' };
                self.message = format!(
                    "{}{}/  {}/{}",
                    slash,
                    pattern,
                    self.search_current + 1,
                    self.search_matches.len()
                );
            }
        }
        self.search_input.clear();
        self.search_origin = None;
        self.search_pattern_backup = None;
        self.mode = Mode::Editor;
    }

    /// Cancel search: restore cursor, restore previous committed pattern.
    pub fn cancel_search(&mut self) {
        if let Some(origin) = self.search_origin.take() {
            self.buffer.cursor = origin;
            self.scroll = self.search_scroll_origin;
        }
        self.search_input.clear();
        self.search_pattern = self.search_pattern_backup.take();
        self.search_matches.clear();
        self.search_current = 0;
        if let Some(ref pat) = self.search_pattern.clone() {
            // Rebuild match list for n/N without moving the restored cursor.
            self.collect_matches(pat);
            let cur = self.buffer.cursor();
            if let Some(idx) = self
                .search_matches
                .iter()
                .position(|p| p.row == cur.row && p.col == cur.col)
            {
                self.search_current = idx;
            }
        }
        self.mode = Mode::Editor;
        self.message = String::from("Search cancelled");
    }

    /// Update live query while typing in Search mode.
    pub fn update_search_input(&mut self) {
        let pattern = self.search_input.clone();
        if pattern.is_empty() {
            self.search_matches.clear();
            self.search_current = 0;
            if let Some(origin) = self.search_origin {
                self.buffer.cursor = origin;
                self.scroll = self.search_scroll_origin;
            }
            self.message = String::from("Search — type to filter, Enter accept, Esc cancel");
            return;
        }
        self.recompute_search(&pattern, true);
        if self.search_matches.is_empty() {
            self.message = format!("/{}/  0 matches", pattern);
        } else {
            self.message = format!(
                "/{}/  {}/{}",
                pattern,
                self.search_current + 1,
                self.search_matches.len()
            );
        }
    }


    pub fn search_pattern_len_chars(&self) -> usize {
        self.active_search_pattern()
            .map(|p| p.chars().count())
            .unwrap_or(0)
    }

    /// Matches on `row` plus the global index of the first one. `search_matches`
    /// is built in row order by `collect_matches`, so this binary-searches the
    /// row's slice instead of the renderer scanning every match for every
    /// character of every visible line. The base index lets callers keep
    /// comparing against `search_current` (a global index).
    pub fn search_matches_row_slice(&self, row: usize) -> (usize, &[Position]) {
        let lo = self.search_matches.partition_point(|p| p.row < row);
        let hi = self.search_matches.partition_point(|p| p.row <= row);
        (lo, &self.search_matches[lo..hi])
    }

    pub fn is_current_search_match(&self, row: usize, col: usize) -> bool {
        self.search_matches
            .get(self.search_current)
            .map(|p| p.row == row && p.col == col)
            .unwrap_or(false)
    }

    /// Is there a selection to copy/delete — from either source?
    pub fn has_selection(&self) -> bool {
        !self.sel.primary().is_empty()
    }

    /// Convert the GUI selection's exclusive `[start, end)` span into the
    /// **inclusive** `(start, last_selected)` pair the legacy vim consumers
    /// (yank/delete/render) expect, so a single source (`self.sel`) feeds all
    /// of them. `end` points one grapheme past the last selected character;
    /// stepping back one grapheme — or, at a line start, to the end of the
    /// previous row — yields the inclusive end.
    fn exclusive_to_inclusive(&self, start: Position, end: Position) -> (Position, Position) {
        if end.col > 0 {
            let col = crate::buffer::grapheme_prev_col(self.buffer.line(end.row), end.col);
            (start, Position::new(end.row, col))
        } else if end.row > 0 {
            let prev = end.row - 1;
            (start, Position::new(prev, self.buffer.line(prev).chars().count()))
        } else {
            (start, end)
        }
    }

    /// The active selection as an **inclusive** `(start, end)` span (vim
    /// convention), preferring the GUI model when it holds a real selection.
    /// Returns `None` for a bare caret with no vim visual mode.
    pub fn selected_range(&self) -> Option<(Position, Position)> {
        let gui = self.sel.primary();
        if gui.is_empty() {
            return None;
        }
        let (s, e) = gui.range();
        Some(self.exclusive_to_inclusive(s, e))
    }

    /// Heads of every selection in `self.sel` **except the primary** — the
    /// extra carets a GUI multi-cursor paints. The primary is already drawn
    /// through the per-line `caret_*`/`sel_*` fields (via [`Self::selected_range`]),
    /// so it is deliberately excluded to avoid a double caret. Empty when the
    /// set is a single selection (the common single-cursor case).
    ///
    /// Positions are exclusive heads (between-character), which is exactly the
    /// drawn-caret column — no inclusive `+1` fix-up, unlike the vim cursor.
    pub fn secondary_caret_positions(&self) -> Vec<Position> {
        if !self.sel.is_multi() {
            return Vec::new();
        }
        let primary = self.sel.primary_index();
        self.sel
            .all()
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != primary)
            .map(|(_, s)| s.head)
            .collect()
    }


    fn open_in_place(&mut self, path: &str) {
        self.open_new_tab(path);
    }

    fn move_file(&mut self, dest: &str) {
        if let Some(ref path) = self.filename {
            let dest_path = PathBuf::from(dest);
            match fs::rename(path, &dest_path) {
                Ok(_) => {
                    self.filename = Some(dest_path);
                    self.message = format!("Moved to: {}", dest);
                    self.set_message(&format!("Moved to: {}", dest));
                }
                Err(e) => {
                    self.set_message(&format!("Error moving: {}", e));
                }
            }
        } else {
            self.set_message("No file to move.");
        }
    }

    /// Recompute matches for `pattern`. If `jump`, move cursor to nearest match
    /// in the active search direction from origin/cursor.
    pub fn recompute_search(&mut self, pattern: &str, jump: bool) {
        self.collect_matches(pattern);
        if self.search_matches.is_empty() {
            self.search_current = 0;
            return;
        }
        let from = self
            .search_origin
            .unwrap_or_else(|| self.buffer.cursor());
        let idx = if self.search_forward {
            self.search_matches
                .iter()
                .position(|p| p.row > from.row || (p.row == from.row && p.col >= from.col))
                .unwrap_or(0)
        } else {
            self.search_matches
                .iter()
                .rposition(|p| p.row < from.row || (p.row == from.row && p.col <= from.col))
                .unwrap_or(self.search_matches.len() - 1)
        };
        self.search_current = idx;
        if jump {
            let pos = self.search_matches[idx];
            self.buffer.cursor = pos;
            self.update_scroll();
        }
    }

    fn collect_matches(&mut self, pattern: &str) {
        self.search_matches.clear();
        if pattern.is_empty() {
            return;
        }
        let smart_case = !pattern.chars().any(|c| c.is_uppercase());
        let pat_lower = if smart_case {
            pattern.to_lowercase()
        } else {
            String::new()
        };

        for (row, line) in self.buffer.lines().iter().enumerate() {
            if smart_case {
                // Case-insensitive: walk char-by-char comparing lowered windows.
                let line_chars: Vec<char> = line.chars().collect();
                let pat_chars: Vec<char> = pat_lower.chars().collect();
                if pat_chars.is_empty() {
                    continue;
                }
                let plen = pat_chars.len();
                if line_chars.len() < plen {
                    continue;
                }
                let line_lower: Vec<char> = line_chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
                let mut i = 0;
                while i + plen <= line_lower.len() {
                    if line_lower[i..i + plen] == pat_chars[..] {
                        self.search_matches.push(Position::new(row, i));
                        i += 1; // overlapping allowed (vim default for most)
                    } else {
                        i += 1;
                    }
                }
            } else {
                let mut search_from = 0usize;
                while search_from <= line.len() {
                    if let Some(byte_rel) = line[search_from..].find(pattern) {
                        let byte_abs = search_from + byte_rel;
                        let col = line[..byte_abs].chars().count();
                        self.search_matches.push(Position::new(row, col));
                        search_from = byte_abs + pattern.len().max(1);
                    } else {
                        break;
                    }
                }
            }
        }
    }

    /// Backward-compatible alias.
    pub fn perform_search(&mut self) {
        if let Some(pat) = self.search_pattern.clone() {
            self.recompute_search(&pat, true);
        }
    }

    pub fn search_next(&mut self) {
        // `n` follows the direction used when the pattern was committed.
        if self.search_forward {
            self.search_step(true);
        } else {
            self.search_step(false);
        }
    }

    pub fn search_prev(&mut self) {
        // `N` is opposite of search direction.
        if self.search_forward {
            self.search_step(false);
        } else {
            self.search_step(true);
        }
    }

    fn search_step(&mut self, forward: bool) {
        if let Some(pat) = self.search_pattern.clone() {
            let cur = self.buffer.cursor();
            self.collect_matches(&pat);
            if self.search_matches.is_empty() {
                self.message = format!("Pattern not found: {}", pat);
                return;
            }
            let idx = if forward {
                self.search_matches
                    .iter()
                    .position(|p| p.row > cur.row || (p.row == cur.row && p.col > cur.col))
                    .unwrap_or(0)
            } else {
                self.search_matches
                    .iter()
                    .rposition(|p| p.row < cur.row || (p.row == cur.row && p.col < cur.col))
                    .unwrap_or(self.search_matches.len() - 1)
            };
            self.search_current = idx;
            let pos = self.search_matches[idx];
            let wrapped = if forward {
                idx == 0 && (pos.row < cur.row || (pos.row == cur.row && pos.col <= cur.col))
            } else {
                idx == self.search_matches.len() - 1
                    && (pos.row > cur.row || (pos.row == cur.row && pos.col >= cur.col))
            };
            self.buffer.cursor = pos;
            self.update_scroll();
            let slash = if self.search_forward { '/' } else { '?' };
            self.message = if wrapped {
                if forward {
                    format!(
                        "search hit BOTTOM, continuing at TOP  {}/{}",
                        idx + 1,
                        self.search_matches.len()
                    )
                } else {
                    format!(
                        "search hit TOP, continuing at BOTTOM  {}/{}",
                        idx + 1,
                        self.search_matches.len()
                    )
                }
            } else {
                format!("{}{}/  {}/{}", slash, pat, idx + 1, self.search_matches.len())
            };
        } else {
            self.message = String::from("No search pattern — press / or ? first");
        }
    }

    /// Search for the word under the cursor (`*` in vim).
    pub fn search_word_under_cursor(&mut self) {
        let word = self.word_under_cursor();
        if word.is_empty() {
            self.message = String::from("No word under cursor");
            return;
        }
        self.push_jump();
        self.search_pattern = Some(word.clone());
        self.search_forward = true;
        self.recompute_search(&word, true);
        // Advance to next occurrence after current position
        if self.search_matches.len() > 1 {
            self.search_next();
        } else if self.search_matches.is_empty() {
            self.message = format!("Pattern not found: {}", word);
        } else {
            self.message = format!("/{}/  1/1", word);
        }
    }

    fn word_under_cursor(&self) -> String {
        let line = self.buffer.line(self.buffer.cursor.row);
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            return String::new();
        }
        let mut col = self.buffer.cursor.col.min(chars.len().saturating_sub(1));
        if col < chars.len() && !(chars[col].is_alphanumeric() || chars[col] == '_') {
            // Try char before cursor
            if col > 0 && (chars[col - 1].is_alphanumeric() || chars[col - 1] == '_') {
                col -= 1;
            } else {
                return String::new();
            }
        }
        let mut start = col;
        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        let mut end = col;
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        chars[start..end].iter().collect()
    }

    pub fn save_file(&mut self) {
        if let Some(path) = self.filename.clone() {
            match atomic_write_file(&path, self.buffer.text()) {
                Ok(_) => {
                    self.modified = false;
                    if self.current_buffer < self.buffers.len() {
                        self.buffers[self.current_buffer].modified = false;
                        self.buffers[self.current_buffer].filename = Some(path.clone());
                    }
                    self.record_mtime();
                    self.refresh_git();
                    self.save_session();
                    self.message = format!("✓ Saved: {}", path.display());
                    self.set_message(&format!("✓ Saved: {}", path.display()));
                    self.fire_hook(crate::hooks::HookEvent::Save);
                }
                Err(e) => {
                    self.message = format!("✗ Error: {}", e);
                    self.set_message(&format!("✗ Error: {}", e));
                }
            }
        } else {
            self.message = String::from("No filename. Use :w <filename>");
            self.set_message("No filename. Use: w <path>");
        }
    }

    pub fn move_left(&mut self) {
        self.buffer.move_left();
    }

    pub fn move_right(&mut self) {
        self.buffer.move_right();
    }

    pub fn move_up(&mut self) {
        self.buffer.move_up();
        self.update_scroll();
    }

    pub fn move_down(&mut self) {
        self.buffer.move_down();
        self.update_scroll();
    }

    /// Apply a fractional line scroll (GUI faces). Integer `scroll` advances when
    /// the accumulator crosses a line; residual stays in `scroll_frac` ∈ (−1, 1).
    /// Positive `delta_lines` reveals content below (window moves down the file).
    pub fn scroll_by_frac(&mut self, delta_lines: f32) {
        if delta_lines == 0.0 || !delta_lines.is_finite() {
            return;
        }
        let visible = self.viewport.height.max(1) as usize;
        let total = self.buffer.line_count();
        let max_scroll = total.saturating_sub(visible.min(total));

        let mut acc = self.scroll_frac + delta_lines;
        // Normalize into whole-line steps + residual.
        if acc >= 1.0 || acc <= -1.0 {
            let whole = acc.trunc() as i32;
            if whole > 0 {
                self.scroll = (self.scroll + whole as usize).min(max_scroll);
            } else if whole < 0 {
                self.scroll = self.scroll.saturating_sub((-whole) as usize);
            }
            acc -= whole as f32;
        }
        // Clamp residual at document edges (no rubber overscroll for now).
        if self.scroll == 0 && acc < 0.0 {
            acc = 0.0;
        }
        if self.scroll >= max_scroll && acc > 0.0 {
            acc = 0.0;
        }
        self.scroll_frac = acc.clamp(-0.999, 0.999);
    }

    /// Integer scroll (TUI / PageUp). Clears sub-line residual.
    pub fn scroll_by_lines(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let visible = self.viewport.height.max(1) as usize;
        let total = self.buffer.line_count();
        let max_scroll = total.saturating_sub(visible.min(total));
        if delta < 0 {
            self.scroll = self.scroll.saturating_sub((-delta) as usize);
        } else {
            self.scroll = (self.scroll + delta as usize).min(max_scroll);
        }
        self.scroll_frac = 0.0;
    }

    /// Absolute first-visible line (native NSScrollView faces). Clears residual.
    pub fn scroll_to_line(&mut self, line: usize) {
        let visible = self.viewport.height.max(1) as usize;
        let total = self.buffer.line_count();
        let max_scroll = total.saturating_sub(visible.min(total));
        self.scroll = line.min(max_scroll);
        self.scroll_frac = 0.0;
    }

    /// Absolute horizontal pan in visual columns (wrap_lines ⇒ no-op / zero).
    pub fn set_hscroll(&mut self, cols: usize) {
        if self.wrap_lines {
            self.hscroll = 0;
            return;
        }
        self.hscroll = cols;
    }

    pub fn update_scroll(&mut self) {
        // Keeping the caret visible is a Caret move — never overrides a
        // Restore/Navigate that has not been consumed yet.
        if self.scroll_intent == ScrollIntent::None {
            self.scroll_intent = ScrollIntent::Caret;
        }
        // Caret-driven scroll snap drops fractional offset (intentional).
        self.scroll_frac = 0.0;
        let cursor_row = self.buffer.cursor.row;
        let visible_height = self.viewport.height.max(1) as usize;
        // Soft-wrap-aware: viewport width minus gutter (~5 cols).
        let text_width = self
            .viewport
            .width
            .saturating_sub(5)
            .max(1) as usize;

        let wrap = self.wrap_lines;
        let wrap_rows = |row: usize| -> usize {
            if !wrap {
                return 1;
            }
            let vis = Self::line_visual_width(&self.buffer, row);
            if vis == 0 {
                1
            } else {
                (vis + text_width - 1) / text_width
            }
        };

        if cursor_row < self.scroll {
            self.scroll = cursor_row;
        }

        // Ensure the cursor's wrap segment is on-screen.
        let screen_col = self
            .buffer
            .buffer_col_to_screen_col(cursor_row, self.buffer.cursor.col);
        let cursor_wrap = if wrap { screen_col / text_width } else { 0 };

        // Horizontal-scroll mode: pan so the cursor stays visible.
        if !wrap {
            if screen_col < self.hscroll {
                self.hscroll = screen_col;
            } else if screen_col >= self.hscroll + text_width {
                self.hscroll = screen_col + 1 - text_width;
            }
        }

        // Visual rows from scroll .. cursor_row-1, plus cursor wrap offset + 1
        let mut needed = cursor_wrap + 1;
        for r in self.scroll..cursor_row {
            needed = needed.saturating_add(wrap_rows(r));
        }
        while needed > visible_height && self.scroll < cursor_row {
            needed = needed.saturating_sub(wrap_rows(self.scroll));
            self.scroll += 1;
        }
        // Fallback: pure buffer-line window if still off (tiny viewports)
        if cursor_row < self.scroll {
            self.scroll = cursor_row;
        }

        // Keep focused split pane scroll in sync
        if self.split.is_split() {
            let p = self.split.focused_pane_mut();
            p.scroll = self.scroll;
            p.tab_index = self.current_buffer;
        }
    }

    fn line_visual_width(buffer: &crate::buffer::Buffer, row: usize) -> usize {
        let line = buffer.line(row);
        let mut vis = 0usize;
        for ch in line.chars() {
            vis += if ch == '\t' {
                4 - (vis % 4)
            } else {
                unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1)
            };
        }
        vis
    }

    pub fn delete_line(&mut self) {
        self.push_undo();
        let row = self.buffer.cursor.row;
        let deleted = self.buffer.delete_line();
        self.store_yank(format!("{}\n", deleted), true);
        if let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) {
            // Deleted the whole line at `row` → remove BP on it, shift later −1
            self.dap.shift_breakpoints(&path, row, -1);
        }
    }

    pub fn delete_word(&mut self) {
        self.push_undo();
        let deleted = self.buffer.delete_word();
        self.store_yank(deleted, false);
    }

    /// `p` — put after cursor / below line
    pub fn paste(&mut self) {
        self.paste_impl(false);
    }

    /// `P` — put before cursor / above line
    pub fn paste_before(&mut self) {
        self.paste_impl(true);
    }

    fn paste_impl(&mut self, before: bool) {
        let Some(val) = self.registers.load_for_put() else {
            // fallback yank_buffer
            if let Some(text) = self.yank_buffer.clone() {
                self.paste_text(&text, text.contains('\n'), before);
            }
            return;
        };
        self.paste_text(&val.text, val.linewise, before);
    }

    fn paste_text(&mut self, text: &str, linewise: bool, before: bool) {
        if text.is_empty() {
            return;
        }
        self.push_undo();
        if linewise {
            let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
            if before {
                let row = self.buffer.cursor.row;
                for (i, line) in lines.iter().enumerate() {
                    self.buffer.insert_line_at(row + i, line.to_string());
                }
                self.buffer.cursor.row = row;
                self.buffer.cursor.col = 0;
            } else {
                for line in lines {
                    self.buffer.paste_line_after(line);
                }
            }
        } else {
            // Charwise: `p` inserts after the cursor char, `P` before it.
            if !before && self.buffer.cursor.col < self.buffer.current_line_len() {
                self.buffer.move_right();
            }
            // Bulk insert (O(n)) instead of char-by-char (O(n²) on long lines).
            let clean = text.replace('\r', "");
            self.buffer.insert_str(&clean);
            // Vim leaves the cursor ON the last pasted character (unless the
            // paste ended on a newline, which lands the cursor at col 0).
            if !clean.is_empty() && !clean.ends_with('\n') {
                self.buffer.move_left();
            }
        }
        self.update_scroll();
        self.message = String::from("Pasted");
    }

    pub fn yank_selection(&mut self) {
        if let Some((start, end)) = self.selected_range() {
            let mut lines: Vec<String> = Vec::new();
            for row in start.row..=end.row {
                let chars: Vec<char> = self.buffer.line(row).chars().collect();
                let s = if row == start.row && row == end.row {
                    let to = (end.col + 1).min(chars.len());
                    let from = start.col.min(to);
                    chars[from..to].iter().collect()
                } else if row == start.row {
                    let from = start.col.min(chars.len());
                    chars[from..].iter().collect()
                } else if row == end.row {
                    let to = (end.col + 1).min(chars.len());
                    chars[..to].iter().collect()
                } else {
                    chars.iter().collect()
                };
                lines.push(s);
            }
            let text = lines.join("\n");
            let label = self.registers.active_label();
            // Copy leaves the selection standing — collapsing it here would
            // make ⌘C deselect, which no GUI editor does.
            self.store_yank(text, false);
            self.message = format!("Yanked → {}", label);
        }
    }

    pub fn delete_selection(&mut self) {
        if let Some((start, end)) = self.selected_range() {
            self.push_undo();
            let mut deleted_text = String::new();

            if start.row == end.row {
                let line = self.buffer.line(start.row);
                let deleted: String = line.chars().skip(start.col).take(end.col.saturating_sub(start.col) + 1).collect();
                let prefix: String = line.chars().take(start.col).collect();
                let suffix: String = line.chars().skip(end.col + 1).collect();
                self.buffer.set_line(start.row, prefix + &suffix);
                deleted_text = deleted;
            } else {
                let first_chars: Vec<char> = self.buffer.line(start.row).chars().collect();
                let last_chars: Vec<char> = self.buffer.line(end.row).chars().collect();

                deleted_text.push_str(&first_chars[start.col.min(first_chars.len())..].iter().collect::<String>());
                for row in (start.row + 1)..end.row {
                    deleted_text.push('\n');
                    deleted_text.push_str(self.buffer.line(row));
                }
                deleted_text.push('\n');
                let last_end = (end.col + 1).min(last_chars.len());
                deleted_text.push_str(&last_chars[..last_end].iter().collect::<String>());

                let first_prefix: String = first_chars.iter().take(start.col).collect();
                let last_suffix: String = last_chars.iter().skip(end.col + 1).collect();

                self.buffer.cursor.row = end.row;
                for _row in (start.row + 1..=end.row).rev() {
                    self.buffer.cursor.row = _row;
                    self.buffer.delete_line();
                }
                self.buffer.cursor.row = start.row;
                self.buffer.set_line(start.row, first_prefix + &last_suffix);
            }

            self.store_yank(deleted_text, false);
            self.buffer.cursor = Position::new(start.row, start.col);
            self.buffer.clamp_col();
            self.focus_editor();
            self.message = String::from("Deleted");
        }
    }

    pub fn record_mtime(&mut self) {
        if let Some(ref path) = self.filename {
            self.file_mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        }
    }

    /// Live-refresh: if the open file changed on disk, reload the buffer.
    /// Preserves cursor/scroll as much as possible; rebuilds folds / git / LSP.
    pub fn check_external_change(&mut self) {
        // Also refresh other open tabs' mtimes lightly — only reload the *active* buffer.
        self.check_active_file_external();
        if self.debug && !self.lsp.diagnostics.is_empty() {
            let rows: Vec<String> = self
                .lsp
                .diagnostics
                .iter()
                .map(|d| d.row.to_string())
                .collect();
            self.set_message(&format!("diag rows: {}", rows.join(",")));
        }
    }

    fn check_active_file_external(&mut self) {
        let Some(path) = self.filename.clone() else {
            return;
        };
        let path_s = path.display().to_string();
        let Ok(meta) = std::fs::metadata(&path) else {
            return;
        };
        let Ok(mtime) = meta.modified() else {
            return;
        };
        let Some(prev) = self.file_mtime else {
            // First observation — just record
            self.file_mtime = Some(mtime);
            return;
        };
        if prev == mtime {
            return;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            // File deleted or unreadable — keep buffer, warn
            self.file_mtime = Some(mtime);
            self.message = format!("⚠ File missing or unreadable: {path_s}");
            return;
        };

        let had_local_edits = self.modified;
        let cursor = self.buffer.cursor();
        let scroll = self.scroll;

        self.buffer = Buffer::from_string(&content);
        // Restore cursor within new bounds
        self.buffer.cursor.row = cursor.row.min(self.buffer.line_count().saturating_sub(1));
        self.buffer.cursor.col = cursor.col;
        self.buffer.clamp_col();
        self.scroll = scroll.min(self.buffer.line_count().saturating_sub(1));
        self.modified = false;
        self.file_mtime = Some(mtime);
        self.undo_stack = UndoStack::new();
        self.undo_stack.push(self.buffer.snapshot());
        self.rebuild_folds();
        self.refresh_git();
        self.lsp_restart_for_current();
        self.sync_lsp_document();

        self.message = if had_local_edits {
            "↻ Live reload (disk won — local unsaved edits discarded)".into()
        } else {
            "↻ Live reload".into()
        };
    }

    pub fn save_state_to_tab(&mut self) {
        if self.current_buffer < self.buffers.len() {
            let tab = &mut self.buffers[self.current_buffer];
            tab.buffer = self.buffer.clone();
            tab.filename = self.filename.clone();
            tab.scroll = self.scroll;
            tab.modified = self.modified;
            tab.undo_stack = self.undo_stack.clone();
            tab.file_mtime = self.file_mtime;
        }
    }

    pub fn restore_state_from_tab(&mut self) {
        self.scroll_intent = ScrollIntent::Restore;
        if let Some(tab) = self.buffers.get(self.current_buffer).cloned() {
            self.buffer = tab.buffer;
            self.filename = tab.filename;
            self.scroll = tab.scroll;
            self.modified = tab.modified;
            self.undo_stack = tab.undo_stack;
            self.file_mtime = tab.file_mtime;
            // GUI selection is ephemeral across tabs (interim): collapse to a
            // caret at the restored cursor. The cursor itself rides in the
            // buffer clone, so it is already correct.
            self.sel = crate::selection::SelectionSet::single(
                crate::selection::Selection::caret(self.buffer.cursor()),
            );
            self.edit_run = EditRun::None;
        }
    }

    pub fn open_new_tab(&mut self, path: &str) {
        self.save_state_to_tab();

        let pathbuf = PathBuf::from(path);
        let abs_path = if pathbuf.is_absolute() {
            pathbuf
        } else {
            env::current_dir().unwrap_or_default().join(&pathbuf)
        };

        for (i, tab) in self.buffers.iter().enumerate() {
            if tab.filename.as_ref() == Some(&abs_path) {
                self.current_buffer = i;
                self.restore_state_from_tab();
                self.lsp_restart_for_current();
                self.refresh_git();
                self.sync_focused_pane_tab();
                self.message = format!("Switched to: {}", abs_path.display());
                return;
            }
        }

        let content = fs::read_to_string(&abs_path).unwrap_or_default();
        let buffer = Buffer::from_string(&content);
        let mtime = std::fs::metadata(&abs_path).ok().and_then(|m| m.modified().ok());
        let mut undo = UndoStack::new();
        undo.push(buffer.snapshot());
        undo.attach_file(&abs_path, self.undo_caching, &content);

        self.buffers.push(BufferTab {
            buffer,
            filename: Some(abs_path.clone()),
            scroll: 0,
            modified: false,
            undo_stack: undo,
            file_mtime: mtime,
        });
        self.current_buffer = self.buffers.len() - 1;
        self.restore_state_from_tab();
        let text = self.buffer.text();
        self.lsp
            .auto_start_with_text(&abs_path.display().to_string(), Some(&text));
        self.lsp_synced_path = Some(abs_path.clone());
        self.lsp_synced_hash = text_hash(&text);
        self.refresh_git();
        self.sync_focused_pane_tab();
        self.message = format!("Opened: {}", abs_path.display());
        self.fire_hook(crate::hooks::HookEvent::Open);
    }

    pub fn next_tab(&mut self) {
        if self.buffers.len() < 2 {
            return;
        }
        self.save_state_to_tab();
        self.current_buffer = (self.current_buffer + 1) % self.buffers.len();
        self.restore_state_from_tab();
        self.lsp_restart_for_current();
        self.refresh_git();
        self.sync_focused_pane_tab();
    }

    pub fn prev_tab(&mut self) {
        if self.buffers.len() < 2 {
            return;
        }
        self.save_state_to_tab();
        if self.current_buffer == 0 {
            self.current_buffer = self.buffers.len() - 1;
        } else {
            self.current_buffer -= 1;
        }
        self.restore_state_from_tab();
        self.lsp_restart_for_current();
        self.refresh_git();
        self.sync_focused_pane_tab();
    }

    pub fn lsp_restart_for_current(&mut self) {
        if let Some(ref path) = self.filename {
            let p = path.display().to_string();
            // Always open with live buffer text so unsaved edits aren't lost.
            let text = self.buffer.text();
            self.lsp.auto_start_with_text(&p, Some(&text));
            self.lsp_synced_path = Some(path.clone());
            self.lsp_synced_hash = text_hash(&text);
        } else {
            // No file — drop per-document state so stale diagnostics from the
            // previous buffer don't paint the empty one.
            self.lsp.diagnostics.clear();
            self.lsp.semantic_tokens.clear();
            self.lsp.inlay_hints.clear();
        }
    }

    pub fn format_document(&mut self) {
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            self.message = String::from("No file to format");
            return;
        };
        if !self.lsp.server_running {
            self.message = String::from("LSP not running");
            return;
        }
        self.sync_lsp_document();
        self.lsp.request_formatting(&path);
        self.message = String::from("Formatting…");
    }

    /// Go to definition (face/FFI entry — same as Normal-mode `gd`).
    pub fn goto_definition(&mut self) {
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            self.message = String::from("No file");
            return;
        };
        if !self.lsp.server_running {
            self.message = String::from("LSP not running");
            return;
        }
        self.push_jump();
        self.sync_lsp_document();
        let c = self.buffer.cursor();
        self.lsp.request_definition(&path, c.row, c.col);
        self.message = String::from("Requested go-to-definition");
    }

    /// Rename symbol under cursor to `new_name` via LSP (face/FFI entry).
    pub fn rename_symbol(&mut self, new_name: &str) {
        let name = new_name.trim();
        if name.is_empty() {
            self.message = String::from("Rename: empty name");
            return;
        }
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            self.message = String::from("No file");
            return;
        };
        if !self.lsp.server_running {
            self.message = String::from("LSP not running");
            return;
        }
        self.sync_lsp_document();
        let c = self.buffer.cursor();
        self.lsp.request_rename(&path, c.row, c.col, name);
        self.message = format!("Renaming → {name}…");
    }

    pub fn request_code_actions(&mut self) {
        let Some(path) = self.filename.as_ref().map(|p| p.display().to_string()) else {
            self.message = String::from("No file");
            return;
        };
        if !self.lsp.server_running {
            self.message = String::from("LSP not running");
            return;
        }
        self.sync_lsp_document();
        let c = self.buffer.cursor();
        self.lsp.request_code_action(&path, c.row, c.col);
        self.message = String::from("Code actions…");
    }

    /// Apply multi-file full-text edits (rename / format / code action).
    pub fn apply_file_edits(&mut self, edits: Vec<crate::lsp::FileEdit>) {
        if edits.is_empty() {
            self.message = String::from("No edits to apply");
            return;
        }
        let n = edits.len();
        let cur_path = self
            .filename
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        for edit in edits {
            let is_current = edit.path == cur_path
                || self
                    .filename
                    .as_ref()
                    .and_then(|p| p.canonicalize().ok())
                    .and_then(|p| {
                        std::path::Path::new(&edit.path)
                            .canonicalize()
                            .ok()
                            .map(|e| e == p)
                    })
                    .unwrap_or(false);

            if is_current {
                self.push_undo();
                let row = self.buffer.cursor.row;
                let col = self.buffer.cursor.col;
                self.buffer = crate::buffer::Buffer::from_string(&edit.text);
                self.buffer.cursor.row = row.min(self.buffer.line_count().saturating_sub(1));
                self.buffer.cursor.col = col;
                self.buffer.clamp_col();
                self.modified = true;
                self.update_scroll();
                // keep LSP in sync
                self.lsp_synced_hash = 0; // force didChange
                self.sync_lsp_document();
            } else {
                // Write other files to disk and refresh if open in a tab
                if let Err(e) = crate::fs_atomic::atomic_write_file(
                    std::path::Path::new(&edit.path),
                    &edit.text,
                ) {
                    self.message = format!("Edit failed {}: {e}", edit.path);
                    continue;
                }
                // Update open tab if present
                for tab in &mut self.buffers {
                    if tab
                        .filename
                        .as_ref()
                        .map(|p| p.display().to_string() == edit.path)
                        .unwrap_or(false)
                    {
                        tab.buffer = crate::buffer::Buffer::from_string(&edit.text);
                        tab.modified = false;
                    }
                }
            }
        }
        self.message = format!("Applied {n} file edit(s)");
    }

    pub fn open_code_actions_palette(&mut self) {
        let actions = std::mem::take(&mut self.lsp.pending_code_actions);
        if actions.is_empty() {
            return;
        }
        self.code_action_bank = actions;
        let items: Vec<crate::palette::PaletteItem> = self
            .code_action_bank
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let detail = if !a.kind.is_empty() {
                    a.kind.clone()
                } else if !a.edits.is_empty() {
                    format!("{} file(s)", a.edits.len())
                } else {
                    a.command.clone().unwrap_or_default()
                };
                crate::palette::PaletteItem {
                    label: a.title.clone(),
                    detail,
                    action: crate::palette::PaletteAction::CodeAction(i),
                }
            })
            .collect();
        self.palette.open_code_actions(items);
        self.mode = Mode::Palette;
        self.message = format!("Code actions — {} items", self.code_action_bank.len());
    }

    pub fn apply_code_action(&mut self, index: usize) {
        let Some(action) = self.code_action_bank.get(index).cloned() else {
            return;
        };
        self.code_action_bank.clear();
        if !action.edits.is_empty() {
            self.apply_file_edits(action.edits);
            return;
        }
        if let Some(cmd) = action.command {
            self.lsp
                .execute_command(&cmd, action.command_args_json.as_deref());
            self.message = format!("Running {cmd}…");
            return;
        }
        self.message = String::from("Code action had no edit/command");
    }

    pub fn close_current_tab(&mut self) {
        // Persist or discard the closing buffer's history (undo_caching).
        self.save_state_to_tab();
        if let Some(tab) = self.buffers.get_mut(self.current_buffer) {
            if tab.filename.is_some() {
                let text = tab.buffer.text();
                tab.undo_stack.finish(self.undo_caching, &text);
            }
        }
        if self.buffers.len() <= 1 {
            self.lsp.shutdown();
            self.buffer = Buffer::new();
            self.filename = None;
            self.scroll = 0;
            self.modified = false;
            self.undo_stack = UndoStack::new();
            self.undo_stack.push(self.buffer.snapshot());
            self.file_mtime = None;
            self.buffers[0] = BufferTab {
                buffer: self.buffer.clone(),
                filename: None,
                scroll: 0,
                modified: false,
                undo_stack: self.undo_stack.clone(),
                file_mtime: None,
            };
            return;
        }

        self.buffers.remove(self.current_buffer);
        if self.current_buffer >= self.buffers.len() {
            self.current_buffer = self.buffers.len() - 1;
        }
        self.restore_state_from_tab();
        // Re-point the LSP at the newly current tab (same language → reuse the
        // running server; different → restart). The old unconditional shutdown
        // left the surviving tabs with no LSP at all.
        self.lsp_restart_for_current();
        self.refresh_git();
        self.message = String::from("Buffer closed");
    }
}

pub use crate::fs_atomic::atomic_write_file;

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(text: &str) -> App {
        let mut app = App::new();
        app.buffer = Buffer::from_string(text);
        app.viewport = EditorViewport {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
            text_x: 5,
            text_y: 0,
        };
        app
    }

    #[test]
    fn references_result_enriches_with_preview_and_ready() {
        let mut app = app_with("fn main() {\n    let x = foo();\n    foo();\n}");
        app.filename = Some(PathBuf::from("/tmp/refs_test.rs"));
        // Simulate the async LSP answer landing.
        app.lsp.pending_references = vec![
            crate::lsp::Location { path: "/tmp/refs_test.rs".into(), row: 1, col: 12 },
            crate::lsp::Location { path: "/tmp/refs_test.rs".into(), row: 2, col: 4 },
        ];
        app.lsp.references_ready = true;

        let (refs, ready) = app.references_result();
        assert!(ready);
        assert_eq!(refs.len(), 2);
        // Preview is the TRIMMED source line from the open buffer (unsaved-safe).
        assert_eq!(refs[0].1, "let x = foo();");
        assert_eq!(refs[1].1, "foo();");
        assert_eq!(refs[0].0.row, 1);
        assert_eq!(refs[1].0.col, 4);
    }

    #[test]
    fn references_result_reports_not_ready_before_answer() {
        let mut app = app_with("x");
        app.filename = Some(PathBuf::from("/tmp/x.rs"));
        app.lsp.references_ready = false;
        app.lsp.pending_references.clear();
        let (refs, ready) = app.references_result();
        assert!(!ready, "must read as pending until the server answers");
        assert!(refs.is_empty());
    }

    #[test]
    fn save_file_is_atomic_and_matches_buffer() {
        let dir = std::env::temp_dir().join(format!(
            "suisei-atomic-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("doc.txt");
        fs::write(&path, "OLD CONTENT THAT MUST NOT BE LEFT HALF-WRITTEN").unwrap();

        let mut app = app_with("hello atomic world\nline2");
        app.filename = Some(path.clone());
        app.modified = true;
        app.save_file();

        let on_disk = fs::read_to_string(&path).expect("read saved file");
        assert_eq!(on_disk, app.buffer.text());
        assert_eq!(on_disk, "hello atomic world\nline2");
        assert!(!app.modified);

        // No temp siblings left behind.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains("suisei-tmp")
            })
            .collect();
        assert!(leftovers.is_empty(), "tmp files leaked: {leftovers:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_file_replaces_existing() {
        let dir = std::env::temp_dir().join(format!(
            "suisei-atomic-fn-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("f.txt");
        fs::write(&path, "before").unwrap();
        atomic_write_file(&path, "after full content").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "after full content");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_file_edits_writes_other_files_atomically() {
        let dir = std::env::temp_dir().join(format!(
            "suisei-apply-edits-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::create_dir_all(&dir);
        let other = dir.join("other.rs");
        fs::write(&other, "old other").unwrap();
        let mut app = app_with("current");
        app.filename = Some(dir.join("current.rs"));
        app.apply_file_edits(vec![crate::lsp::FileEdit {
            path: other.display().to_string(),
            text: "new other content".into(),
        }]);
        assert_eq!(fs::read_to_string(&other).unwrap(), "new other content");
        // Current buffer untouched.
        assert_eq!(app.buffer.line(0), "current");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lsp_face_entry_points_set_messages_without_server() {
        // Face/FFI call these real App methods; without a server they must not panic
        // and must report LSP-not-running (not silently no-op into undefined state).
        let mut app = app_with("fn main() {}");
        app.filename = Some(std::path::PathBuf::from("/tmp/suisei-lsp-face-test.rs"));
        app.lsp.server_running = false;
        app.format_document();
        assert!(app.message.to_lowercase().contains("lsp"));
        app.goto_definition();
        assert!(app.message.to_lowercase().contains("lsp"));
        app.rename_symbol("foo");
        assert!(app.message.to_lowercase().contains("lsp"));
        app.request_code_actions();
        assert!(app.message.to_lowercase().contains("lsp"));
        // With a filename but server "running", request paths should arm messages.
        app.lsp.server_running = true;
        app.format_document();
        assert!(
            app.message.contains("Format") || app.message.contains("…") || app.message.contains("Formatting"),
            "format should request: {}",
            app.message
        );
        app.goto_definition();
        assert!(
            app.message.to_lowercase().contains("definition")
                || app.message.contains("…")
                || app.message.contains("Requested"),
            "definition: {}",
            app.message
        );
        app.rename_symbol("Bar");
        assert!(
            app.message.contains("Bar") || app.message.to_lowercase().contains("renam"),
            "rename: {}",
            app.message
        );
        app.request_code_actions();
        assert!(
            app.message.to_lowercase().contains("code") || app.message.contains("…"),
            "code actions: {}",
            app.message
        );
    }

    #[test]
    fn undo_restores_simple_insert() {
        let mut app = app_with("ab");
        app.mode = Mode::Editor;
        app.buffer.cursor = Position::new(0, 2);
        app.push_undo();
        app.buffer.insert_char('X');
        assert_eq!(app.buffer.line(0), "abX");
        app.undo();
        assert_eq!(
            app.buffer.line(0),
            "ab",
            "undo must restore pre-insert text"
        );
    }

    #[test]
    fn hscroll_follows_cursor_when_wrap_off() {
        let long = "x".repeat(300);
        let mut app = app_with(&long);
        app.wrap_lines = false;
        // viewport width 80 − 5 gutter = 75 text cols
        app.buffer.cursor.col = 200;
        app.update_scroll();
        assert_eq!(app.hscroll, 200 + 1 - 75);
        // Moving back left pulls the pan window back.
        app.buffer.cursor.col = 10;
        app.update_scroll();
        assert_eq!(app.hscroll, 10);
        // Wrap mode never pans.
        app.wrap_lines = true;
        app.hscroll = 0;
        app.buffer.cursor.col = 250;
        app.update_scroll();
        assert_eq!(app.hscroll, 0);
    }

    #[test]
    fn split_panes_keep_independent_cursors() {
        let text = vec!["word here"; 50].join("\n");
        let mut app = app_with(&text);
        app.buffer.cursor.row = 10;
        app.buffer.cursor.col = 3;
        app.split_vertical();
        // Pane 1: move somewhere else.
        app.focus_other_pane();
        app.buffer.cursor.row = 40;
        app.buffer.cursor.col = 7;
        // Back to pane 0 — its cursor must be restored.
        app.focus_other_pane();
        assert_eq!((app.buffer.cursor.row, app.buffer.cursor.col), (10, 3));
        // And pane 1 kept its own.
        app.focus_other_pane();
        assert_eq!((app.buffer.cursor.row, app.buffer.cursor.col), (40, 7));
    }

    #[test]
    fn close_split_keeps_the_other_pane() {
        let text = vec!["line"; 100].join("\n");
        let mut app = app_with(&text);
        app.buffer.cursor.row = 8;
        app.split_vertical();
        app.focus_pane(1);
        // Pane 0 (the unfocused one) sits at scroll 5.
        app.split.panes[0].scroll = 5;
        app.close_split();
        assert!(!app.split.is_split());
        // Vim C-w q: the focused pane closes; the *other* view survives.
        assert_eq!(app.scroll, 5);
    }

    #[test]
    fn search_finds_all_matches_char_safe() {
        let mut app = app_with("hello\nhello world\nHELLO");
        app.search_pattern = Some("hello".into());
        app.collect_matches("hello");
        // smart-case: all lowercase → case-insensitive → 3 matches
        assert_eq!(app.search_matches.len(), 3);
        assert_eq!(app.search_matches[0], Position::new(0, 0));
        assert_eq!(app.search_matches[1], Position::new(1, 0));
        assert_eq!(app.search_matches[2], Position::new(2, 0));
    }

    #[test]
    fn search_case_sensitive_when_pattern_has_upper() {
        let mut app = app_with("hello\nHELLO\nHello");
        app.collect_matches("Hello");
        assert_eq!(app.search_matches.len(), 1);
        assert_eq!(app.search_matches[0], Position::new(2, 0));
    }

    #[test]
    fn search_utf8_char_indices() {
        let mut app = app_with("안녕 hello 안녕");
        app.collect_matches("안녕");
        assert_eq!(app.search_matches.len(), 2);
        assert_eq!(app.search_matches[0].col, 0);
        // "안녕 " = 3 chars, then "hello " = 6, second at col 9
        assert_eq!(app.search_matches[1].col, 9);
    }

    #[test]
    fn enter_search_cancel_restores_cursor() {
        let mut app = app_with("abc\ndef\nghi");
        app.buffer.cursor = Position::new(1, 1);
        app.scroll = 0;
        app.enter_search();
        app.search_input = "ghi".into();
        app.update_search_input();
        assert_eq!(app.buffer.cursor.row, 2);
        app.cancel_search();
        assert_eq!(app.mode, Mode::Editor);
        assert_eq!(app.buffer.cursor, Position::new(1, 1));
        assert!(app.search_input.is_empty());
    }

    #[test]
    fn commit_search_keeps_pattern_for_n() {
        let mut app = app_with("foo bar foo");
        app.enter_search();
        app.search_input = "foo".into();
        app.update_search_input();
        app.commit_search();
        assert_eq!(app.mode, Mode::Editor);
        assert_eq!(app.search_pattern.as_deref(), Some("foo"));
        assert_eq!(app.search_matches.len(), 2);
        let first = app.buffer.cursor;
        app.search_next();
        assert_ne!(app.buffer.cursor, first);
    }

    #[test]
    fn search_jumps_to_nearest_from_origin() {
        let mut app = app_with("aa\nbb\naa\ncc\naa");
        app.buffer.cursor = Position::new(1, 0); // on "bb"
        app.enter_search();
        app.search_input = "aa".into();
        app.update_search_input();
        // nearest at-or-after origin (row 1) is row 2
        assert_eq!(app.buffer.cursor.row, 2);
    }

    #[test]
    fn close_tab_keeps_remaining_tab_state() {
        let dir = std::env::temp_dir();
        let f1 = dir.join("xei_test_close_a.rs");
        let f2 = dir.join("xei_test_close_b.rs");
        let _ = std::fs::write(&f1, "fn a() {}");
        let _ = std::fs::write(&f2, "fn b() {}");
        let mut app = App::open_file(f1.to_str().unwrap());
        app.open_new_tab(f2.to_str().unwrap());
        assert_eq!(app.buffers.len(), 2);
        app.close_current_tab();
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(app.filename.as_deref(), Some(f1.as_path()));
        assert_eq!(app.buffer.line(0), "fn a() {}");
        let _ = std::fs::remove_file(&f1);
        let _ = std::fs::remove_file(&f2);
    }
}

#[cfg(all(test, feature = "extensions"))]
mod ext_tests {
    use super::*;

    #[test]
    fn webview_html_renders_and_loads_as_kitty_image() {
        if !xei_ext_host::webview::available() {
            eprintln!("skipping: no headless browser");
            return;
        }
        let html = "<!doctype html><body style='margin:0;background:#7c3aed;width:100vw;height:100vh'></body>";
        let png = xei_ext_host::webview::render_html(html, 320, 200).expect("render");
        // The exact display path the frontend uses: PNG → ImageAsset with a
        // populated Kitty cache, ready for place_rgba_rect_b64.
        let img = crate::media::ImageAsset::load(&png, 14).expect("image load");
        assert!(img.cached_w > 0 && img.cached_h > 0, "no cache built");
        assert!(!img.cached_b64.is_empty(), "no base64 payload");
    }

    #[test]
    fn exttest_command_drives_a_message_from_a_real_extension() {
        let mut app = App::new();
        app.ext_test(); // spawn host + activate + invoke xei.hello
        if app.ext.is_none() {
            eprintln!("skipping: node unavailable ({})", app.message);
            return;
        }
        let start = std::time::Instant::now();
        let mut text = None;
        while start.elapsed().as_millis() < 5000 {
            if let Some(ext) = app.ext.as_mut() {
                ext.poll();
                if let Some(m) = ext.pending_messages.first() {
                    text = Some(m.text.clone());
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(text.as_deref().unwrap_or("").contains("Hello"), "got: {text:?}");
    }
}
