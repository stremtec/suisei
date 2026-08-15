use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, Position};
use crate::completion::Completions;
use crate::config;
use crate::explorer::Explorer;
use crate::fold::FoldState;
use crate::git::{GitBlame, GitGutter};
use crate::git_workbench::GitWorkbench;
use crate::lsp::LspClient;
use crate::nav::{Jump, JumpList};
use crate::palette::{Palette, PaletteAction};
use crate::preview::PreviewState;
use crate::registers::Registers;
use crate::scm::ScmPanel;
use crate::selection::Selection;
use crate::session::{self, Session, SessionFile};
use crate::settings::SettingsPanel;
use crate::syntax::SyntaxEngine;
pub use crate::tabs::{BufferTab, FIRST_TAB_ID, TabStrip};
use crate::theme::{self, OCEAN, Theme};
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
    /// The find bar — committed pattern, live input, matches, origins.
    /// State + pure computation live in [`crate::search::SearchState`]; the
    /// buffer-touching orchestration is the thin wrapper below.
    pub search: crate::search::SearchState,
    pub completions: Completions,
    /// The global scope's symbols, kept between completion activations.
    ///
    /// Collecting them is the whole cost of the scope walk — 8.7 ms on a 50k
    /// line file, and the same 8.7 ms whether the caret is nested five deep or
    /// sitting at byte 0. Keyed on the syntax tree's identity, so it survives
    /// every keystroke that does not produce a new parse.
    pub scope_cache: crate::scope::GlobalScopeCache,
    pub modified: bool,
    pub mouse: MouseState,
    /// The editor stage in pixels — the single source of viewport
    /// geometry (A6). The cell grid is derived: [`App::grid_cols`] /
    /// [`App::grid_rows`].
    pub stage: Stage,
    pub explorer: Explorer,
    /// The **docked** shell strip (⌃T) — whether it is showing, and nothing
    /// else. The shells inside are the face's (SwiftTerm), as are the panes'.
    pub terminal: TerminalDock,
    /// Folded layouts, in strip order. See `layout_tab`.
    pub layouts: Vec<crate::layout_tab::LayoutTab>,
    /// The layout the editor is currently showing, if any. Switching to a
    /// document tab clears it — that is the whole point of folding.
    pub active_layout: Option<u64>,
    /// Which pane shell's close-confirm dialog is open, if any. Per-shell:
    /// the old shared dock flag let pane B answer pane A's prompt, blackholed
    /// B's keys while A's dialog was up, and `y` killed whichever shell was
    /// focused at confirm time rather than the one that asked.
    pub(crate) pane_close_confirm: Option<crate::split::TerminalId>,
    /// For a terminal opened over a split pane (⌃⇧T): the document that pane was
    /// showing before the shell took it over. Closing the terminal tab restores
    /// this document into the pane and keeps the split, instead of collapsing
    /// it. Keyed by the terminal tab's own id. Cleared when either the terminal
    /// or the remembered document closes.
    pub(crate) terminal_replaced: std::collections::HashMap<BufferId, BufferId>,
    pub explorer_width: u16,
    pub terminal_width: u16,
    pub resize_target: Option<ResizeTarget>,
    pub explorer_separator_x: u16,
    pub terminal_separator_x: u16,
    pub screen_width: u16,
    pub screen_height: u16,
    /// Resolved Light/Dark palette plus the optional semantic highlight hue.
    /// Owned because the highlight override is per-user, not a global static.
    pub theme: Theme,
    /// The configured theme name — `"system"` means follow macOS.
    pub theme_pref: String,
    /// Native floating-chrome material: `"clear"` or `"tinted"`.
    pub glass_style: String,
    /// Current system appearance, pushed down by the face.
    pub system_is_dark: bool,
    pub xlc_height: u16,
    pub xlc_separator_y: u16,
    pub file_mtime: Option<std::time::SystemTime>,
    /// The active file was deleted/moved out from under the open buffer. Set by
    /// `check_active_file_external`, cleared when the path reappears. Drives the
    /// tab's "deleted on disk" state so editing a vanished file is not silent.
    pub file_deleted: bool,
    /// Rows of the LIVE document that a live reload just replaced, and when.
    ///
    /// A reload used to be invisible: the text changed under the reader with
    /// nothing to say where. The face needs to know WHICH rows to mark, and it
    /// cannot work that out afterwards — the old text is gone by then. So the
    /// reload records it on the way through.
    ///
    /// Rows only, no kinds. "This line is not what you were looking at" is the
    /// whole of what a reader needs; splitting it into added and changed would
    /// be a diff view, and this is a notice.
    ///
    /// Cleared by `expire_live_marks` once the face has had time to show them.
    /// Only the live document: a row number means nothing for a buffer that is
    /// not on screen, and background tabs are announced on their chips instead.
    pub live_rows: std::collections::HashMap<usize, crate::LiveKind>,
    /// How many rows a removal took away, at the row that closed over them.
    ///
    /// A removal has nothing to mark — the lines are gone — so the mark points
    /// at the line that moved up into the space. The face needs the SIZE of
    /// that space to close it, and the mark alone cannot say it.
    pub live_removed: u16,
    pub live_marked_at: Option<std::time::Instant>,
    /// Bumped whenever `live_rows` or `live_files` changes, so the face can
    /// pull only when there is something new rather than every frame.
    pub live_gen: u64,
    /// Files a live reload touched recently, and when.
    ///
    /// Separate from `live_rows` because it answers a different question and
    /// for a wider set. Rows describe the LIVE document, and only that one:
    /// a row number means nothing for a buffer that is not on screen. This is
    /// per PATH, and it includes background tabs — which is the whole point,
    /// since the project tree is where a file you are not looking at can say
    /// that it moved.
    pub live_files: std::collections::HashMap<std::path::PathBuf, std::time::Instant>,
    /// The tab strip — documents in strip order + the id source (A3-2).
    pub tabs: TabStrip,
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
    /// Pending text-object modifier `i`/`a` after operator
    pub pending_to_mod: Option<char>,
    /// Tab bar hit regions for mouse (filled by UI each frame)
    pub tab_hit_regions: Vec<(u16, u16, usize)>, // x_start, x_end, tab_index
    pub tab_bar_y: u16,
    /// Screen-row → buffer-row map for the current frame (handles soft-wrap).
    /// Index 0 = the first editor content row. Built in the TUI draw path.
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
    /// Last buffer version pushed to the LSP (didChange gate).
    lsp_synced_version: u64,
    /// Git gutter signs for the current file
    pub git: GitGutter,
    /// Optional git blame overlay (`gb` toggle)
    pub blame: GitBlame,
    /// Indent-based folds (`za` / `zc` / `zo` / `zM` / `zR`)
    pub folds: FoldState,
    /// Extra carets (primary = `buffer.cursor`)
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
    /// PR review (files + comments + diff).
    /// Plugin hooks (`~/.suisei/hooks.toml`).
    pub hooks: crate::hooks::HooksConfig,
    /// Release check + self-update (welcome notice · :update).
    pub update: crate::update::UpdateState,
    /// Hook results from background threads (drained by poll_hook_messages).
    hook_msg_tx: std::sync::mpsc::Sender<String>,
    hook_msg_rx: std::sync::mpsc::Receiver<String>,
    /// Async git gutter/blame refresh (latest generation wins).
    #[allow(clippy::type_complexity)]
    /// Last seen mtime of `.git/index`, and when it was last looked at. See
    /// [`App::poll_git_index`].
    git_index_stamp: Option<std::time::SystemTime>,
    git_index_checked_at: Option<std::time::Instant>,
    #[allow(clippy::type_complexity)]
    git_refresh_rx: Option<
        std::sync::mpsc::Receiver<(
            u64,
            String,
            (
                bool,
                std::collections::HashMap<usize, crate::git::GitSign>,
                Vec<crate::git::GitHunk>,
            ),
            Option<(
                bool,
                std::collections::HashMap<usize, crate::git::BlameLine>,
            )>,
        )>,
    >,
    git_refresh_gen: u64,
    /// Show LSP code lenses in the editor.
    pub code_lens_enabled: bool,
    pub term_sync: bool,
    pub term_undercurl: bool,
    pub term_underline_color: bool,
    pub term_hyperlinks: bool,
    pub term_modern: bool,
    /// Terminal speaks Kitty graphics protocol (Ghostty/Kitty/WezTerm).
    pub term_kitty_graphics: bool,
    /// Pending rename: new name input via XLC or message
    pub rename_pending: bool,
    /// Last document state pushed to the LSP via didChange (path + text hash).
    /// `sync_lsp_document` uses these to send post-edit full-text syncs exactly
    /// once per change instead of the old pre-edit push_undo notification.
    pub(crate) lsp_synced_path: Option<PathBuf>,
    pub(crate) lsp_synced_hash: u64,
    /// Lines the server last saw — the diff base for incremental didChange
    /// (None → the next sync sends the full document).
    lsp_synced_lines: Option<Vec<String>>,
    /// The document `App`'s live fields hold — `buffer`, `scroll`, `cursor`,
    /// `undo_stack`, `filename` and friends are this document's working copy.
    ///
    /// Outside a switch it equals the focused pane's document (S2: `App` IS
    /// the focused pane); the two differ for exactly one statement during a
    /// focus change, which is the only moment [`App::save_state_to_tab`]
    /// needs to remember where the live copy came from. The active tab's
    /// POSITION is derived — see [`App::current_buffer`].
    pub(crate) live_doc: BufferId,
    /// Source of terminal ids for pane terminals. Monotonic; never reused.
    pub(crate) next_terminal_id: u32,
    /// Buffer version at the last dirty-flag re-check, so an idle document is
    /// never re-hashed. See [`App::recheck_modified`].
    dirty_checked_version: u64,
    /// A dirty latch was raised (edit path) and has not been re-derived from
    /// the text yet. The version gate alone missed the case where the latch
    /// fires without moving the version — a no-op edit right after undo/redo
    /// (where `dirty_checked_version` already equals the live version) stayed
    /// dirty forever. This forces exactly one re-check after any latch, then
    /// the version gate takes over so an idle dirty buffer is not re-hashed.
    dirty_needs_recheck: bool,
    /// Widest line the view has seen in this document, in display columns.
    ///
    /// A high-water mark, not a live maximum, and deliberately so. Rescanning
    /// the whole file would be O(file) on the typing path; letting the extent
    /// *shrink* as short lines scroll into view would resize the scroller thumb
    /// under the user's hand. So it only grows, and resets when the document
    /// does. See [`App::max_hscroll`].
    pub(crate) content_width: usize,
    /// Soft-wrap row map for the live document, rebuilt when the document or
    /// the wrap width changes. See [`crate::wrap`].
    ///
    /// A `RefCell` because every question the face asks about it arrives
    /// through a `&self` getter — the map is a cache of the buffer, not a fact
    /// about it, and making the getters `&mut` would put a write lock on the
    /// paint path for something that is pure derivation.
    pub(crate) wrap_map: std::cell::RefCell<crate::wrap::WrapMap>,
    /// Fingerprint of the text as it stands on disk (at load, and after each
    /// save). `modified` is a one-way latch — set by every edit, cleared only
    /// by a save — so undoing back to the original state left the file marked
    /// dirty forever. This is what lets `undo` put the flag back down; see
    /// [`App::refresh_modified`].
    pub(crate) saved_hash: u64,
}

/// Stable handle to an open document, issued by [`App::take_tab_id`].
///
/// Monotonic and never reused, which is the whole point: a handle to a closed
/// document resolves to `None` rather than to whichever document has since
/// moved into that slot. Anything that outlives a single call — a split pane,
/// and later a layout tree — holds one of these instead of a position in
/// `App::buffers`.
///
/// `BufferId::default()` is the never-issued id (`take_tab_id` starts at 1), so
/// a zero-valued handle is "nothing", not "the first tab".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BufferId(pub u64);

/// The docked shell strip.
///
/// This used to be a 1,775-line terminal emulator: a PTY, a cell grid, a
/// 5,000-row scrollback, an escape parser, mouse tracking, and a re-encoder
/// that turned the grid back into ANSI so it could cross the C ABI and be
/// re-parsed on the other side. All of that is SwiftTerm's now — it runs the
/// shell in the same view that draws it, on the same side of the boundary as
/// the user. What core needs to know about the strip is whether it is showing,
/// because that decides the editor's height and where the keyboard goes.
#[derive(Clone, Copy, Default)]
pub struct TerminalDock {
    pub open: bool,
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

/// The editor stage in PIXELS — the single source of viewport geometry
/// (A6). The face reports it through [`App::resize_stage`] (the one
/// production writer); everything cell-shaped is DERIVED through
/// [`App::grid_cols`] / [`App::grid_rows`]. Nothing stores cells, so
/// nothing can go stale — the old cell viewport needed a manual re-sync
/// at a dozen call sites.
#[derive(Clone, Copy, Debug)]
pub struct Stage {
    /// Stage size in points (the face already subtracted chrome).
    pub w: f32,
    pub h: f32,
    /// Painted line height in points (row pitch).
    pub cell_px: f32,
    /// Glyph cell width in points (column pitch).
    pub cell_w: f32,
    pub dpr: f32,
}

impl Default for Stage {
    fn default() -> Self {
        Self {
            w: 1200.0,
            h: 800.0,
            cell_px: 18.0,
            cell_w: 9.0,
            dpr: 2.0,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        let (hook_msg_tx, hook_msg_rx) = std::sync::mpsc::channel();
        let mut app = Self {
            running: true,
            mode: Mode::Editor,
            buffer: Buffer::new(),
            message: String::from("Suisei"),
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
            search: crate::search::SearchState::default(),
            completions: Completions::new(),
            scope_cache: crate::scope::GlobalScopeCache::default(),
            modified: false,
            mouse: MouseState::default(),
            stage: Stage::default(),
            explorer: Explorer::new(),
            terminal: TerminalDock::default(),
            layouts: Vec::new(),
            active_layout: None,
            explorer_width: 22,
            terminal_width: 30,
            resize_target: None,
            explorer_separator_x: 0,
            terminal_separator_x: 0,
            screen_width: 80,
            screen_height: 24,
            theme: OCEAN,
            theme_pref: "system".to_string(),
            glass_style: "clear".to_string(),
            system_is_dark: true,
            xlc_height: 11,
            xlc_separator_y: 0,
            file_mtime: None,
            file_deleted: false,
            live_rows: std::collections::HashMap::new(),
            live_marked_at: None,
            live_gen: 0,
            live_removed: 0,
            live_files: std::collections::HashMap::new(),
            tabs: TabStrip::new(),
            live_doc: FIRST_TAB_ID,
            syntax: SyntaxEngine::new(),
            lsp: LspClient::new(),
            debug: false,
            show_metrics: false,
            metrics: ProcMetrics::default(),
            bench_report: None,
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
            lsp_synced_version: 0,
            git: GitGutter::new(),
            blame: GitBlame::default(),
            folds: FoldState::new(),
            scm: ScmPanel::new(),
            git_wb: GitWorkbench::new(),
            settings: SettingsPanel::new(),
            preview: PreviewState::new(),
            preview_image: None,
            preview_audio: None,
            split: crate::split::SplitState::new(),
            peek: crate::peek::PeekState::new(),
            workspace_search: crate::workspace_search::WorkspaceSearch::new(),
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
            hooks: crate::hooks::HooksConfig::load(),
            update: crate::update::UpdateState::new(),
            hook_msg_tx,
            hook_msg_rx,
            git_index_stamp: None,
            git_index_checked_at: None,
            git_refresh_rx: None,
            git_refresh_gen: 0,
            code_lens_enabled: true,
            term_sync: false,
            term_undercurl: false,
            term_underline_color: false,
            term_hyperlinks: false,
            term_modern: false,
            term_kitty_graphics: false,
            rename_pending: false,
            lsp_synced_path: None,
            lsp_synced_hash: 0,
            lsp_synced_lines: None,
            next_terminal_id: 1,
            pane_close_confirm: None,
            terminal_replaced: std::collections::HashMap::new(),
            saved_hash: EMPTY_TEXT_HASH,
            dirty_checked_version: 0,
            dirty_needs_recheck: false,
            content_width: 0,
            wrap_map: std::cell::RefCell::new(crate::wrap::WrapMap::default()),
        };
        // The first pane shows the first tab — pane slots name documents
        // by id, and `BufferId::default()` names nothing.
        app.split.focused_pane_mut().buffer = FIRST_TAB_ID;
        app
    }
}

/// FNV-1a over the whole document — cheap enough per sync tick, and unlike a
/// sampled fingerprint it cannot miss an edit.
/// Display columns a line occupies: tabs advance to the next stop, wide glyphs
/// (CJK, emoji) take two cells. Matches what the editor actually paints, so a
/// scroll clamp derived from it lands on the last glyph rather than near it.
///
/// Lives in [`crate::wrap`], which measures the same thing to decide where a
/// line breaks. Two copies would let the wrap point and the horizontal extent
/// disagree about where a line ends.
use crate::wrap::display_columns as display_width;

/// `text_hash("")` — the clean fingerprint of a brand-new empty buffer. A
/// literal so `App::default()` stays a plain struct expression.
pub(crate) const EMPTY_TEXT_HASH: u64 = 0xcbf2_9ce4_8422_2325;

pub(crate) fn text_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Line diff between the synced lines and the current lines → one
/// incremental LSP content change with UTF-16 positions (the encoding
/// negotiated in initialize). None when identical.
fn lsp_changes_since(prev: &[String], cur: &[String]) -> Option<Vec<crate::lsp::LspTextChange>> {
    let (start, old_lines, new_lines) = crate::undo::diff_lines(prev, cur)?;
    let change = crate::undo::line_delta_to_change(start, old_lines, new_lines, prev);
    let end_offset = change.start + change.old.chars().count();
    let (sl, sc) = offset_to_utf16(prev, change.start);
    let (el, ec) = offset_to_utf16(prev, end_offset);
    Some(vec![crate::lsp::LspTextChange {
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
        text: change.new,
    }])
}

/// Char offset → (line, UTF-16 column) against `lines`. Offsets past the
/// end clamp to the end of the last line.
fn offset_to_utf16(lines: &[String], offset: usize) -> (usize, usize) {
    let mut remaining = offset;
    for (row, line) in lines.iter().enumerate() {
        let chars = line.chars().count();
        if remaining <= chars {
            let utf16: usize = line.chars().take(remaining).map(|c| c.len_utf16()).sum();
            return (row, utf16);
        }
        remaining -= chars + 1;
    }
    let row = lines.len().saturating_sub(1);
    let utf16 = lines
        .get(row)
        .map(|l| l.chars().map(|c| c.len_utf16()).sum())
        .unwrap_or(0);
    (row, utf16)
}

/// A NUL byte in the first 8 KiB, or bytes that are not UTF-8 at all.
///
/// The NUL test is what every editor uses and it is what catches the files
/// that matter here — images, audio, executables. The UTF-8 test catches the
/// rest, and both are deliberately conservative: a false "binary" costs the
/// user an explicit message, a false "text" costs them the file.
pub fn looks_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8 * 1024)];
    if head.contains(&0) {
        return true;
    }
    // A truncated multi-byte character at the 8 KiB boundary is not binary, so
    // judge the whole file when it is small and only the head when it is not.
    if bytes.len() <= 8 * 1024 {
        match std::str::from_utf8(bytes) {
            Ok(_) => false,
            // `error_len() == None` means the slice ran out mid-character
            // rather than containing something invalid. That is a cut, not
            // evidence — and this function must not give a different answer
            // depending on where its caller happened to cut.
            Err(e) => e.error_len().is_some(),
        }
    } else {
        false
    }
}

/// `looks_binary` for a path that may not exist. A missing file is not binary
/// — that is a new document, and saving it is the point.
pub fn file_looks_binary(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else {
        return false;
    };
    // One byte PAST the window, deliberately.
    //
    // `looks_binary` decides whether it is allowed to judge UTF-8 by asking
    // whether it holds the whole file — and it can only answer that if a
    // larger file yields a larger slice. Reading exactly 8 KiB made every
    // file over 8 KiB look complete, so the UTF-8 test ran on a truncated
    // head, and any multi-byte character straddling byte 8192 failed it.
    // The file was then declared binary and ⌘S refused to save it, with no
    // way for the user to tell which files were affected or why: it depends
    // on whether a Hangul syllable, an emoji or a curly quote happens to
    // land on that boundary.
    //
    // `read_to_end` over `take` rather than a single `read`, because `read`
    // may return a short count on a perfectly healthy file — which produces
    // the same misclassification by a second route.
    let mut head = Vec::with_capacity(8 * 1024 + 1);
    if f.take(8 * 1024 + 1).read_to_end(&mut head).is_err() {
        return false;
    }
    looks_binary(&head)
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
        if cfg.update_check {
            self.update.start_check(env!("CARGO_PKG_VERSION"));
        }
        self.theme_pref = cfg.theme.clone();
        self.glass_style = cfg.glass_style.clone();
        self.theme = theme::effective(&cfg.theme, &cfg, self.system_is_dark);
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
        let cfg = config::load();
        self.theme = crate::theme::effective(&self.theme_pref, &cfg, is_dark);
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

    pub fn ext_test(&mut self) {
        self.message = "extensions were removed when Suisei split from the xei workspace".into();
    }

    pub fn ext_api_report(&mut self) {
        self.message = "extensions were removed when Suisei split from the xei workspace".into();
    }

    pub fn ext_plugins(&mut self, _arg: &str) {
        self.message = "extensions were removed when Suisei split from the xei workspace".into();
    }

    pub fn load_installed_extensions(&mut self) {}

    pub fn plugin_store_refresh_installed(&mut self) {}

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
            env::current_dir().unwrap_or_default().join(&pathbuf)
        };
        // `read_to_string().unwrap_or_default()` turned any file that is not
        // valid UTF-8 into an EMPTY document, silently. Opening a PNG showed a
        // blank editor, and ⌘S then wrote that blank over the PNG. Say so
        // instead, and let `save_file` refuse.
        let raw = fs::read(&abs_path).ok();
        let kind = crate::media::classify_bytes(&abs_path, raw.as_deref());
        let content = match raw {
            Some(b) if !kind.is_viewer() => String::from_utf8_lossy(&b).into_owned(),
            _ => String::new(),
        };
        let on_disk_hash = text_hash(&content);
        let message = if kind.is_viewer() {
            format!("{}: {}", kind.noun(), abs_path.display())
        } else {
            format!("Opened: {}", abs_path.display())
        };
        let buffer = Buffer::from_string(&content);
        let mut undo = UndoStack::new();
        undo.push(buffer.snapshot());
        let mtime = std::fs::metadata(&abs_path)
            .ok()
            .and_then(|m| m.modified().ok());
        let mut app = Self {
            buffer: buffer.clone(),
            filename: Some(abs_path.clone()),
            message,
            modified: false,
            // The App's own saved-hash, not just the tab's: `refresh_modified`
            // (undo/redo) compares the live text against THIS. Left at the empty
            // default, undo back to the on-disk text re-derived dirty against the
            // wrong hash — the file read modified forever after any edit+undo.
            saved_hash: on_disk_hash,
            undo_stack: undo.clone(),
            file_mtime: mtime,
            tabs: TabStrip::with_first(BufferTab {
                id: FIRST_TAB_ID,
                buffer,
                filename: Some(abs_path.clone()),
                scroll: 0,
                modified: false,
                saved_hash: on_disk_hash,
                undo_stack: undo,
                file_mtime: mtime,
                terminal: None,
                terminal_title: None,
                kind,
                terminal_cwd: None,
            }),
            live_doc: FIRST_TAB_ID,
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
        if session.items.is_empty() {
            return;
        }
        let mut first = true;
        for item in session.items.iter() {
            match item {
                session::SessionItem::File(f) => {
                    if first {
                        // Replace the empty first tab
                        let content = fs::read_to_string(&f.path).unwrap_or_default();
                        self.buffer = Buffer::from_string(&content);
                        self.filename = Some(PathBuf::from(&f.path));
                        self.buffer.cursor.row =
                            f.row.min(self.buffer.line_count().saturating_sub(1));
                        let line_len =
                            self.buffer.line(self.buffer.cursor.row).chars().count();
                        self.buffer.cursor.col = f.col.min(line_len);
                        self.mark_clean();
                        if !self.tabs.buffers.is_empty() {
                            self.tabs.buffers[0].buffer = self.buffer.clone();
                            self.tabs.buffers[0].filename = self.filename.clone();
                            self.tabs.buffers[0].modified = false;
                            self.tabs.buffers[0].saved_hash = self.saved_hash;
                        }
                    } else {
                        self.open_new_tab(&f.path);
                        self.buffer.cursor.row =
                            f.row.min(self.buffer.line_count().saturating_sub(1));
                        let line_len =
                            self.buffer.line(self.buffer.cursor.row).chars().count();
                        self.buffer.cursor.col = f.col.min(line_len);
                    }
                }
                session::SessionItem::Terminal { cwd } => {
                    // A tab, at a directory. The shell itself is spawned by
                    // whoever shows the tab — the process is not part of what
                    // a session can carry across a restart.
                    self.open_terminal_tab_at(&PathBuf::from(cwd));
                }
            }
            first = false;
        }
        let active = session
            .active
            .min(self.tabs.buffers.len().saturating_sub(1));
        if active != self.current_buffer() {
            self.save_state_to_tab();
            self.split.focused_pane_mut().buffer = self.tabs.buffers[active].id;
            self.restore_state_from_tab();
        }
        // The split, if the session had one — its focus overrides `active`.
        if let Some(ref split) = session.split {
            self.restore_split(split);
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
        let files = session
            .items
            .iter()
            .filter(|i| i.file().is_some())
            .count();
        let shells = session.items.len() - files;
        self.message = if shells == 0 {
            format!("Restored session ({files} file(s))")
        } else {
            format!("Restored session ({files} file(s), {shells} shell(s))")
        };
    }

    pub fn save_session(&self) {
        let current = self.current_buffer();
        let mut items = Vec::new();
        for (i, tab) in self.tabs.buffers.iter().enumerate() {
            if tab.terminal.is_some() {
                // A shell keeps its slot in the list even though it has no
                // file, because `pane.tab` indexes this and a gap would point
                // every pane after it at the wrong tab.
                items.push(session::SessionItem::Terminal {
                    cwd: tab
                        .terminal_cwd
                        .clone()
                        .unwrap_or_else(|| self.terminal_working_directory())
                        .display()
                        .to_string(),
                });
                continue;
            }
            let Some(ref path) = tab.filename else {
                continue;
            };
            let (row, col) = if i == current {
                (self.buffer.cursor.row, self.buffer.cursor.col)
            } else {
                (tab.buffer.cursor.row, tab.buffer.cursor.col)
            };
            items.push(session::SessionItem::File(SessionFile {
                path: path.display().to_string(),
                row,
                col,
            }));
        }
        if items.is_empty() {
            return;
        }
        let active = self
            .tabs
            .buffers
            .iter()
            .enumerate()
            .filter(|(_, t)| t.filename.is_some() || t.terminal.is_some())
            .position(|(i, _)| i == current)
            .unwrap_or(0);
        let split = self.session_split();
        session::save(&Session {
            items,
            active,
            split,
        });
        let _ = self.dap.persist_breakpoints();
    }

    /// The split layout as session tokens — only when every pane shows a
    /// SAVED file (an unsaved tab has no identity across restarts, so a
    /// split referencing one cannot survive).
    fn session_split(&self) -> Option<session::SessionSplit> {
        if !self.split.is_split() {
            return None;
        }
        // Index within the saved ITEMS, in save order. Shells count: they are
        // saved now, so a split holding one can be saved too — it used to be
        // dropped entirely, because every pane had to resolve to a file.
        let saved: Vec<BufferId> = self
            .tabs
            .buffers
            .iter()
            .filter(|t| t.filename.is_some() || t.terminal.is_some())
            .map(|t| t.id)
            .collect();
        let file_idx_of = |pid: crate::split::PaneId| -> Option<usize> {
            let pane = self.split.panes.iter().find(|p| p.id == pid)?;
            saved.iter().position(|id| *id == pane.buffer)
        };
        let tree = Self::layout_tokens(self.split.root(), &file_idx_of)?;
        let panes = self
            .split
            .panes
            .iter()
            .map(|p| session::SessionPane {
                tab: saved.iter().position(|id| *id == p.buffer).unwrap_or(0),
                scroll: p.scroll,
                row: p.cursor.0,
                col: p.cursor.1,
            })
            .collect();
        Some(session::SessionSplit {
            tree,
            focus_pane: self.split.focus_index(),
            panes,
        })
    }

    /// `T<file>` leaves, `S<C|R>:w0,w1,...:child;child` splits. None when any
    /// leaf has no saved file — then the whole split cannot persist.
    fn layout_tokens(
        layout: &crate::split::Layout,
        file_idx_of: &impl Fn(crate::split::PaneId) -> Option<usize>,
    ) -> Option<String> {
        use crate::split::{Axis, Layout};
        match layout {
            Layout::Leaf(pid) => Some(format!("T{}", file_idx_of(*pid)?)),
            Layout::Split {
                axis,
                children,
                weights,
            } => {
                let axis_ch = match axis {
                    Axis::Col => 'C',
                    Axis::Row => 'R',
                };
                let ws = weights
                    .iter()
                    .map(|w| format!("{:.3}", w))
                    .collect::<Vec<_>>()
                    .join(",");
                let cs = children
                    .iter()
                    .map(|c| Self::layout_tokens(c, file_idx_of))
                    .collect::<Option<Vec<_>>>()?;
                Some(format!("S{}:{}:{}", axis_ch, ws, cs.join(";")))
            }
        }
    }

    /// Rebuild the split from session tokens. Leaves carry saved-file
    /// indices; panes get fresh ids in leaf order and their saved viewports.
    /// The split's focus overrides the plain `active` tab index.
    fn restore_split(&mut self, s: &session::SessionSplit) {
        let Some(indexed) = Self::parse_layout_tokens(&s.tree) else {
            return;
        };
        if s.panes.is_empty() {
            return;
        }
        let mut next_id = 1u32;
        let mut panes = Vec::new();
        let mut leaf = 0usize;
        let tree = self.rebuild_tree(&indexed, s, &mut next_id, &mut panes, &mut leaf);
        if panes.is_empty() {
            return;
        }
        // Park the live document BEFORE the tree replaces the panes: after
        // the restore the active tab is derived from the new focus, and a
        // save would land on the wrong document.
        self.save_state_to_tab();
        self.split.restore(tree, panes);
        if s.focus_pane < self.split.panes.len() {
            self.split.set_focus(s.focus_pane);
        }
        self.load_focused_pane();
    }

    /// Parse `T<n>` / `S<C|R>:w,w:child;child` into a tree whose leaves carry
    /// the saved-file index as a placeholder `PaneId`.
    fn parse_layout_tokens(s: &str) -> Option<crate::split::Layout> {
        use crate::split::{Axis, Layout, PaneId};
        fn parse_at(b: &[u8], i: &mut usize) -> Option<Layout> {
            match *b.get(*i)? {
                b'T' => {
                    *i += 1;
                    let start = *i;
                    while *i < b.len() && b[*i].is_ascii_digit() {
                        *i += 1;
                    }
                    let n: u32 = std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()?;
                    Some(Layout::Leaf(PaneId(n)))
                }
                b'S' => {
                    *i += 1;
                    let axis = match b.get(*i)? {
                        b'C' => Axis::Col,
                        b'R' => Axis::Row,
                        _ => return None,
                    };
                    *i += 1;
                    if *b.get(*i)? != b':' {
                        return None;
                    }
                    *i += 1;
                    let wstart = *i;
                    while *i < b.len() && b[*i] != b':' {
                        *i += 1;
                    }
                    let wstr = std::str::from_utf8(&b[wstart..*i]).ok()?;
                    let weights: Vec<f32> = wstr
                        .split(',')
                        .map(|w| w.parse().ok())
                        .collect::<Option<_>>()?;
                    if *b.get(*i)? != b':' {
                        return None;
                    }
                    *i += 1;
                    // Exactly weights.len() children — the count is known,
                    // which is what keeps nested splits unambiguous: a
                    // greedy "parse until no ;" would swallow the OUTER
                    // split's later children into the innermost split.
                    let n = weights.len();
                    let mut children = Vec::with_capacity(n);
                    for k in 0..n {
                        children.push(parse_at(b, i)?);
                        if k + 1 < n {
                            if *b.get(*i)? != b';' {
                                return None;
                            }
                            *i += 1;
                        }
                    }
                    if children.is_empty() {
                        return None;
                    }
                    Some(Layout::Split {
                        axis,
                        children,
                        weights,
                    })
                }
                _ => None,
            }
        }
        let mut i = 0;
        let out = parse_at(s.as_bytes(), &mut i)?;
        (i == s.len()).then_some(out)
    }

    /// Assign fresh pane ids in leaf order and attach the saved viewports.
    fn rebuild_tree(
        &self,
        layout: &crate::split::Layout,
        s: &session::SessionSplit,
        next_id: &mut u32,
        panes: &mut Vec<crate::split::Pane>,
        leaf: &mut usize,
    ) -> crate::split::Layout {
        use crate::split::Layout;
        match layout {
            Layout::Leaf(crate::split::PaneId(fi)) => {
                let pid = crate::split::PaneId(*next_id);
                *next_id += 1;
                let sp = s.panes.get(*leaf).cloned().unwrap_or_default();
                *leaf += 1;
                let buffer = self
                    .tabs
                    .buffers
                    .get(*fi as usize)
                    .map(|t| t.id)
                    .unwrap_or_default();
                panes.push(crate::split::Pane {
                    id: pid,
                    buffer,
                    scroll: sp.scroll,
                    hscroll: 0,
                    cursor: (sp.row, sp.col),
                });
                Layout::Leaf(pid)
            }
            Layout::Split {
                axis,
                children,
                weights,
            } => Layout::Split {
                axis: *axis,
                children: children
                    .iter()
                    .map(|c| self.rebuild_tree(c, s, next_id, panes, leaf))
                    .collect(),
                weights: weights.clone(),
            },
        }
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

    /// Stage or discard the change on a line, then re-read the gutter.
    ///
    /// The refresh is not left to `poll_git_index`: that watches `.git/index`,
    /// which a discard never touches, so a discarded hunk would keep its bar
    /// until something else happened.
    pub fn apply_gutter_hunk(
        &mut self,
        line_1based: u32,
        action: crate::git::HunkAction,
    ) -> i32 {
        let Some(path) = self.filename.clone() else {
            self.message = "No file".into();
            return -1;
        };
        let row = (line_1based.max(1) - 1) as usize;
        match crate::git::apply_hunk(&path.display().to_string(), row, action) {
            Ok(msg) => {
                self.message = msg;
                // A discard rewrote the file underneath the buffer.
                if action == crate::git::HunkAction::Discard {
                    self.reload_from_disk();
                    // And it is gone NOW. `refresh_git` is asynchronous, so
                    // leaving this to it kept the discarded change in the
                    // gutter for a whole `git diff` — its bar still drawn, and
                    // "Show Change" still holding the rows it revealed open
                    // over text that had just become real. Long enough to see
                    // on any repository big enough to have a slow diff.
                    //
                    // Only the discarded hunk: clearing the gutter outright
                    // would blink every other bar in the file for the same
                    // interval, to fix a flicker.
                    self.git.hunks.retain(|h| !h.contains(row));
                    self.git.signs.clear();
                    crate::git::signs_from_hunks(&self.git.hunks, &mut self.git.signs);
                }
                self.refresh_git();
                0
            }
            Err(e) => {
                self.message = e;
                -1
            }
        }
    }

    /// Notice the index changing underneath us, and re-read the gutter.
    ///
    /// Staging is not something the editor owns. It happens in the SCM panel,
    /// in the git workbench, and in whatever terminal the user has open —
    /// which is why hooking each path is the wrong shape: the one that stages
    /// from outside the app can never be hooked at all, and the gutter's bar
    /// silently disagreed with `git status` until the file was next saved.
    ///
    /// `.git/index`'s mtime answers for every one of them, including the paths
    /// that do not exist yet. Cheap enough to ask a few times a second: one
    /// `stat` of one file.
    fn poll_git_index(&mut self) -> bool {
        let now = std::time::Instant::now();
        if let Some(last) = self.git_index_checked_at {
            if now.duration_since(last) < std::time::Duration::from_millis(400) {
                return false;
            }
        }
        self.git_index_checked_at = Some(now);
        let hint = self.filename.clone();
        let Some(root) = crate::git_ops::find_git_root(hint.as_deref()) else {
            return false;
        };
        let stamp = std::fs::metadata(root.join(".git").join("index"))
            .and_then(|m| m.modified())
            .ok();
        if stamp == self.git_index_stamp {
            return false;
        }
        // The FIRST observation is not a change — it is the baseline. Kicking a
        // refresh here would re-run two `git diff`s on every file open for no
        // new information.
        let known = self.git_index_stamp.is_some();
        self.git_index_stamp = stamp;
        if known {
            self.refresh_git();
        }
        known
    }

    /// Apply a finished background git refresh (call once per frame).
    pub fn poll_git_refresh(&mut self) -> bool {
        use std::sync::mpsc::TryRecvError;
        let index_moved = self.poll_git_index();
        let Some(rx) = self.git_refresh_rx.take() else {
            return index_moved;
        };
        match rx.try_recv() {
            Ok((generation, path, (g_avail, signs, hunks), blame)) => {
                if generation != self.git_refresh_gen {
                    return false;
                }
                self.git.path = path.clone();
                self.git.available = g_avail;
                self.git.signs = signs;
                self.git.hunks = hunks;
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
    /// A path on disk moved (rename, drag in the tree, or a folder move).
    ///
    /// The face owns the filesystem call — it has native Trash and native drag
    /// payloads — but the core owns every reference to that path: open tabs,
    /// the active filename, the language server's open document, and the crash
    /// journal (whose filename is a hash of the path). Without this the tab
    /// stays pointed at a file that no longer exists and the next save
    /// resurrects it at the old location.
    ///
    /// Matches by prefix so moving a folder carries every file open beneath it.
    /// Returns how many buffers were repointed.
    pub fn path_moved(&mut self, old: &Path, new: &Path) -> usize {
        let repoint = |p: &Path| -> Option<PathBuf> {
            if p == old {
                return Some(new.to_path_buf());
            }
            p.strip_prefix(old).ok().map(|rest| new.join(rest))
        };
        let mut n = 0;
        for tab in &mut self.tabs.buffers {
            if let Some(cur) = tab.filename.clone() {
                if let Some(next) = repoint(&cur) {
                    tab.filename = Some(next);
                    n += 1;
                }
            }
        }
        if let Some(cur) = self.filename.clone() {
            if let Some(next) = repoint(&cur) {
                self.filename = Some(next.clone());
                // The server has the old URI open; re-open under the new one so
                // diagnostics and definitions keep resolving.
                let text = self.buffer.text();
                self.lsp
                    .auto_start_with_text(&next.display().to_string(), Some(&text));
                self.lsp_synced_path = Some(next);
                self.lsp_synced_hash = 0;
            }
        }
        n
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
            if w.is_empty() { "?".into() } else { w }
        };
        self.sync_lsp_document();
        self.call_hierarchy.begin(&word, dir);
        self.mode = Mode::CallHierarchy;
        self.lsp.request_call_hierarchy(&path, c.row, c.col, dir);
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
        self.lsp.request_call_hierarchy(&path, c.row, c.col, dir);
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
        self.message = format!("hooks reloaded · enabled={}", self.hooks.enabled);
    }

    /// Run the hook for `event` on a background thread; results arrive via
    /// poll_hook_messages() so a slow hook never blocks the editor.
    pub(crate) fn fire_hook(&mut self, event: crate::hooks::HookEvent) {
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
        let hint = self.filename.as_deref().or(cwd.as_deref());
        self.git_wb.open_at(hint, from_scm);
        self.sync_scm_snapshot_from_git_workbench();
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
        self.message = format!("Git · {} @ {}  ·  Status ready  ·  Esc back", b, root_note);
    }

    /// The compact Source Control navigator and full Workbench are two faces
    /// of one repository state. Keep the hidden navigator's snapshot aligned
    /// at the transition boundary so it cannot retain "No repository" or an
    /// older count while Workbench already shows fresh status.
    fn sync_scm_snapshot_from_git_workbench(&mut self) {
        self.scm.root = self.git_wb.root.clone();
        self.scm.branch = self.git_wb.branch.clone();
        self.scm.ahead = self.git_wb.ahead;
        self.scm.behind = self.git_wb.behind;
        self.scm.staged = self.git_wb.staged.clone();
        self.scm.changes = self.git_wb.changes.clone();
        self.scm.selected = self
            .git_wb
            .selected
            .min(self.scm.total_files().saturating_sub(1));
        self.scm.error = self.git_wb.error.clone();
        self.scm.last_result = None;
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
        // Native Settings is an independent window. Cmd+, means "show it",
        // never "toggle it closed". This also makes duplicate SwiftUI scene
        // presentation callbacks harmless instead of racing open → close and
        // leaving a visible window backed by an empty settings snapshot.
        if self.settings.open {
            self.mode = Mode::Settings;
            return;
        }
        if self.palette.open {
            self.palette.close();
        }
        if self.preview.open {
            self.preview.close_immediate();
        }
        // Settings and the native Source Control workbench are independent
        // macOS windows. Opening one must not tear down the other's model:
        // SwiftUI can keep both scenes visible and hands keyboard ownership
        // back through the per-window focus callbacks.
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
        if cfg.update_check {
            self.update.start_check(env!("CARGO_PKG_VERSION"));
        }
        self.theme_pref = cfg.theme.clone();
        self.glass_style = cfg.glass_style.clone();
        self.theme = theme::effective(&cfg.theme, &cfg, self.system_is_dark);
        // Restart LSP for current file with new server map
        self.lsp_restart_for_current();
    }

    /// Whether a pane terminal's close-confirm dialog is open. Dispatch gates
    /// y/n/Esc on this — nothing else may read the latch directly.
    pub fn pane_close_confirm_open(&self) -> bool {
        self.pane_close_confirm.is_some()
    }

    /// Whether progressive GPU-terminal features should run this session.
    pub fn gpu_active(&self) -> bool {
        self.gpu_acc
            && (self.term_modern
                || self.term_sync
                || self.term_underline_color
                || self.term_undercurl)
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
                self.preview.cell_dims = (self.cell_px_or_default(), self.cell_px_h_or_default());
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
        let kind = self.preview.kind.map(|k| k.label()).unwrap_or("Preview");
        self.message = format!("Preview · {kind} — Esc close · j/k scroll · r refresh");
    }

    /// Open media / data preview from a filesystem path (explorer Enter).
    /// Effective pixels-per-cell for image caches.
    /// Physical pixels per cell row — derived from the stage (A6). The old
    /// field had no writer anywhere and silently "defaulted" to 14 forever;
    /// the media preview scaled against a lie.
    pub fn cell_px_or_default(&self) -> u32 {
        let px = (self.stage.cell_px * self.stage.dpr).round() as u32;
        if px >= 4 { px } else { 14 }
    }

    pub fn cell_px_h_or_default(&self) -> u32 {
        let px = (self.stage.cell_w * self.stage.dpr).round() as u32;
        if px >= 6 {
            px
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
                            spans: vec![(
                                format!("  load error: {e}"),
                                crate::preview::PreviewStyle::AlertWarning,
                            )],
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
        self.message.clear();
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

    /// Columns of the cell grid the editor works in — derived from the
    /// pixel stage (A6). Pure derivation: the sanity clamp lives in
    /// [`App::resize_stage`], the one production writer, so tests writing
    /// `stage` directly get exactly the degenerate grid they asked for.
    pub fn grid_cols(&self) -> u16 {
        (self.stage.w / self.stage.cell_w.max(1.0)).floor().max(0.0) as u16
    }

    /// Rows of the cell grid — derived from the pixel stage (A6).
    pub fn grid_rows(&self) -> u16 {
        (self.stage.h / self.stage.cell_px.max(1.0))
            .floor()
            .max(0.0) as u16
    }

    /// Cell dimensions `(rows, cols)` of the FOCUSED editor pane.
    ///
    /// `grid_rows`/`grid_cols` describe the whole stage; under a split each
    /// pane is only a fraction of it, so caret-follow that measured against the
    /// stage let the caret leave a half-size pane long before `update_scroll`
    /// reacted — the "split scroll does not follow the caret" bug, on both axes.
    /// With no split the focused rect is FULL, so this is exactly the stage grid.
    fn focused_pane_grid(&self) -> (usize, usize) {
        let rect = self
            .split
            .rects()
            .get(self.split.focus_index())
            .copied()
            .unwrap_or(crate::split::Rect::FULL);
        let cols = ((self.stage.w * rect.w) / self.stage.cell_w.max(1.0))
            .floor()
            .max(1.0) as usize;
        let rows = ((self.stage.h * rect.h) / self.stage.cell_px.max(1.0))
            .floor()
            .max(1.0) as usize;
        (rows, cols)
    }

    /// The face resized the stage — the ONE production write of geometry.
    /// Clamps here keep the derived grid sane (40..500 columns, 8..200
    /// rows), matching what the old sync imposed on the cell viewport.
    pub fn resize_stage(&mut self, w: f32, h: f32, cell_px: f32, cell_w: f32, dpr: f32) {
        let cell_px = cell_px.max(12.0);
        let cell_w = cell_w.max(6.0);
        self.stage.cell_px = cell_px;
        self.stage.cell_w = cell_w;
        self.stage.dpr = dpr.max(1.0);
        self.stage.w = w.max(80.0).max(40.0 * cell_w).min(500.0 * cell_w);
        self.stage.h = h.max(80.0).max(8.0 * cell_px).min(200.0 * cell_px);
    }

    pub fn push_undo(&mut self) {
        self.undo_stack.push(self.buffer.snapshot());
        self.modified = true;
        // Ask for one exact re-derive: the edit may have been a no-op (backspace
        // at BOF, a paste of what was already there) that latched dirty without
        // changing the text.
        self.dirty_needs_recheck = true;
        let idx = self.current_buffer();
        if idx < self.tabs.buffers.len() {
            self.tabs.buffers[idx].modified = true;
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
            // Incremental when there is a synced baseline to diff against;
            // full sync on path change / first sync / diff failure.
            let cur_lines: Vec<String> = text.split('\n').map(String::from).collect();
            let mut sent = false;
            if !path_changed {
                if let Some(prev) = self.lsp_synced_lines.as_deref() {
                    if let Some(changes) = lsp_changes_since(prev, &cur_lines) {
                        self.lsp
                            .notify_change_incremental(&path_str, &text, &changes);
                        sent = true;
                    }
                }
            }
            if !sent {
                self.lsp.notify_change(&path_str, &text);
            }
            self.lsp_synced_lines = Some(cur_lines);
            self.lsp_synced_path = Some(path);
            self.lsp_synced_hash = hash;
        }
        self.lsp_synced_version = self.buffer.version();
    }

    pub fn undo(&mut self) {
        // The stack applies the delta's inverse straight to the buffer and
        // restores the pre-edit cursor — no snapshot round trip.
        if self.undo_stack.undo(&mut self.buffer) {
            // Undo restores Buffer::cursor, but the GUI edit model keeps its
            // own SelectionSet. Leaving it at the post-edit position makes the
            // next paste/IME commit jump forward while the status bar reports
            // the restored cursor. Until history snapshots selections too,
            // collapse them to the restored primary caret.
            self.sync_sel_to_cursor();
            self.edit_run = EditRun::None;
            self.refresh_modified();
            self.message = String::from("UNDO");
        } else {
            self.message = String::from("Already at oldest change");
        }
    }

    pub fn redo(&mut self) {
        if self.undo_stack.redo(&mut self.buffer) {
            self.sync_sel_to_cursor();
            self.edit_run = EditRun::None;
            self.refresh_modified();
            self.message = String::from("REDO");
        } else {
            self.message = String::from("Already at newest change");
        }
    }

    /// The buffer now matches what is on disk — just loaded, or just saved.
    /// Records the fingerprint so a later undo back to this text can clear the
    /// dirty flag again.
    pub fn mark_clean(&mut self) {
        // A load or a save means the text underneath the high-water mark
        // changed; let it be re-measured from what is on screen.
        self.content_width = 0;
        self.saved_hash = text_hash(&self.buffer.text());
        self.modified = false;
        self.file_deleted = false;
        self.dirty_needs_recheck = false;
        // End the current insert/delete run at the save/load point. Otherwise a
        // save mid-run left `edit_run` set, so the very next keystroke coalesced
        // into the pre-save run and skipped `push_undo` — it never re-latched
        // dirty, and the file looked saved while it was being edited.
        self.edit_run = crate::app::EditRun::None;
        let idx = self.current_buffer();
        if idx < self.tabs.buffers.len() {
            self.tabs.buffers[idx].modified = false;
            self.tabs.buffers[idx].saved_hash = self.saved_hash;
        }
    }

    /// Recompute the dirty flag from the text itself.
    ///
    /// O(file), so this is deliberately **only** called from undo and redo —
    /// the two operations that can make a dirty buffer clean again. Typing
    /// keeps the cheap one-way latch in `push_undo`; a keystroke can only ever
    /// make a document dirtier, so it never needs to ask.
    fn refresh_modified(&mut self) {
        self.dirty_checked_version = self.buffer.version();
        self.dirty_needs_recheck = false;
        self.modified = text_hash(&self.buffer.text()) != self.saved_hash;
        let idx = self.current_buffer();
        if idx < self.tabs.buffers.len() {
            self.tabs.buffers[idx].modified = self.modified;
        }
    }

    /// Correct a dirty flag that latched when it should not have. Returns true
    /// when the flag actually changed.
    ///
    /// `push_undo` raises the flag on every edit and is exact in that
    /// direction — a keystroke really does make the document differ from disk.
    /// Nothing was exact the other way, so anything that touched the buffer and
    /// put it back left the file marked dirty for the rest of the session: an
    /// abandoned IME composition, a paste of text that was already there, a
    /// deletion of nothing. Users saw files "dirty without being edited".
    ///
    /// Auditing every path that can latch the flag is a losing game; this
    /// re-derives it from the text instead. The work is bounded three ways:
    /// only while the flag is up (a clean buffer cannot be made cleaner, and
    /// the latch is exact for clean → dirty), only when the text has moved
    /// since the last check, and only on the engine's ~1 s cadence. On a
    /// 60,000-line file that is one 0.24 ms hash per second of active editing.
    pub fn recheck_modified(&mut self) -> bool {
        if !self.modified {
            return false;
        }
        // Re-derive when a latch is pending (may be a no-op that never moved the
        // version) OR the text has actually moved since the last check. An idle
        // dirty buffer trips neither and is never re-hashed.
        if !self.dirty_needs_recheck && self.buffer.version() == self.dirty_checked_version {
            return false;
        }
        self.refresh_modified();
        !self.modified // the only outcome this call can produce is a clear
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
                self.buffer
                    .line(loc.row.min(self.buffer.line_count().saturating_sub(1)))
                    .trim()
                    .to_string()
            } else {
                let lines = disk.entry(loc.path.clone()).or_insert_with(|| {
                    std::fs::read_to_string(&loc.path)
                        .map(|s| s.lines().map(|l| l.to_string()).collect())
                        .unwrap_or_default()
                });
                lines
                    .get(loc.row)
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default()
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
        // Paste moves `buffer.cursor` directly, bypassing the GUI edit model.
        // Collapse `sel` to a caret there so it cannot stay pinned to the
        // pre-paste position: a Korean IME commits a syllable-plus-space run
        // ("요 ") through here between fast single-char inserts, and the next
        // `gui_insert_text` reads `sel.head`. Left stale, that head sat before
        // the pasted run, so the following syllable landed inside it —
        // "안녕하세요 안녕" typed back as "안녕하세안녕…요". Same coherence
        // contract the legacy dispatch path already enforces after typing.
        self.sync_sel_to_cursor();
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
        let target = line_1based
            .saturating_sub(1)
            .min(self.buffer.line_count().saturating_sub(1));
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
        self.search.pattern = Some(word.clone());
        self.search.forward = false;
        self.recompute_search(&word, true);
        if self.search.matches.len() > 1 {
            self.search_prev();
        } else if self.search.matches.is_empty() {
            self.message = format!("Pattern not found: {}", word);
        } else {
            self.message = format!("?{}/  1/1", word);
        }
    }

    pub fn quit(&mut self) {
        // Persist or discard undo history for every open file (undo_caching).
        self.save_state_to_tab();
        let caching = self.undo_caching;
        for tab in &mut self.tabs.buffers {
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

    /// Ctrl+D — select the next occurrence of the word under the primary
    /// head. The occurrence becomes a real selection in `sel`, so every
    /// edit path (gui_insert_text / gui_delete_*) applies to all of them
    /// at once — the parallel MultiCursor machinery is gone.
    pub fn multi_cursor_add_next(&mut self) {
        // Word at the primary selection's START (not head): add() makes the
        // newest occurrence the primary, and its head sits just PAST the
        // word — word_at there sees the following space and gives up.
        let primary = self.sel.primary();
        let anchor = primary.start();
        let Some((wstart, wend, word)) = crate::multi_cursor::word_at(&self.buffer, anchor) else {
            self.message = "No word under cursor".into();
            return;
        };
        // A bare caret grows to its word first, so every cursor covers the
        // same text (VS Code semantics — the first press selects, and the
        // caret must not stay a zero-width twin).
        if primary.is_empty() {
            self.sel.set_primary(Selection::new(wstart, wend));
        }
        // Search after the last non-empty selection's end.
        let from = self
            .sel
            .all()
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.end())
            .max()
            .unwrap_or(wend);
        let Some(pos) = crate::multi_cursor::find_next(&self.buffer, &word, from) else {
            self.message = "No more matches".into();
            return;
        };
        let word_len = word.chars().count();
        let end_pos = Position {
            row: pos.row,
            col: pos.col + word_len,
        };
        self.sel.add(Selection::new(pos, end_pos));
        self.message = format!("cursors: {}", self.sel.len());
    }

    /// Ctrl+Alt+Down — column caret below the lowest caret.
    pub fn multi_cursor_add_below(&mut self) {
        let p = self
            .sel
            .all()
            .iter()
            .map(|s| s.head)
            .max()
            .unwrap_or_else(|| self.sel.primary().head);
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
        self.sel.add(Selection::caret(np));
        self.message = format!("cursors: {}", self.sel.len());
    }

    /// Ctrl+Alt+Up — column caret above the highest caret.
    pub fn multi_cursor_add_above(&mut self) {
        let p = self
            .sel
            .all()
            .iter()
            .map(|s| s.head)
            .min()
            .unwrap_or_else(|| self.sel.primary().head);
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
        self.sel.add(Selection::caret(np));
        self.message = format!("cursors: {}", self.sel.len());
    }

    pub fn open_file_palette(&mut self) {
        let root = if !self.explorer.cwd.as_os_str().is_empty()
            && self.explorer.cwd != std::path::Path::new("/")
        {
            self.explorer.cwd.clone()
        } else {
            self.project_root()
        };
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

    /// Stable cwd policy shared by every terminal surface and every newly
    /// created shell session. Prefer the folder the user opened, then the
    /// discovered project root of the active file. A launched `.app` commonly
    /// inherits `/`; never expose that process accident as a project cwd.
    pub fn terminal_working_directory(&self) -> std::path::PathBuf {
        let explorer = self.explorer.cwd.as_path();
        if !explorer.as_os_str().is_empty() && explorer != std::path::Path::new("/") {
            return explorer.to_path_buf();
        }
        let root = self.project_root();
        if root != std::path::Path::new("/") {
            return root;
        }
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."))
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
        let fallback = if self
            .filename
            .as_ref()
            .map(|p| p.display().to_string())
            .as_deref()
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
                    self.message = String::from("Unsaved changes. Use Save or Force quit.");
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
                    let cfg = config::load();
                    self.theme = theme::effective(t.name, &cfg, self.system_is_dark);
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
        let next = rows
            .iter()
            .copied()
            .find(|r| *r > cur)
            .or_else(|| rows.first().copied());
        if let Some(row) = next {
            self.push_jump();
            self.buffer.cursor.row = row;
            self.buffer.move_to_line_start();
            self.update_scroll();
            let sign = self
                .git
                .sign_at(row)
                .map(|s| format!("{s:?}"))
                .unwrap_or_default();
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
            let sign = self
                .git
                .sign_at(row)
                .map(|s| format!("{s:?}"))
                .unwrap_or_default();
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
                self.buffer.cursor.row = cursor.row.min(self.buffer.line_count().saturating_sub(1));
                self.buffer.cursor.col = cursor.col;
                self.buffer.clamp_col();
                self.scroll = scroll.min(self.buffer.line_count().saturating_sub(1));
                self.mark_clean();
                self.record_mtime();
                self.undo_stack = UndoStack::new();
                self.undo_stack.push(self.buffer.snapshot());
                if let Some(p) = self.filename.clone() {
                    let text = self.buffer.text();
                    self.undo_stack.attach_file(&p, self.undo_caching, &text);
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

    // ---- layout tabs (J7) -----------------------------------------------

    // ── Stable-id tab operations ─────────────────────────────────────────
    // The strip's slot numbers stop being buffer indices the moment a folded
    // layout hides members (unified style) or gathers them into a run, so the
    // face addresses tabs by `BufferTab::id` and the translation lives here.

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
        let (cursor, scroll) = (self.buffer.cursor(), self.scroll);
        self.search.begin(forward, cursor, scroll);
        self.mode = Mode::Search;
        self.message = String::from("Find — Enter accept · Esc cancel · ↑↓ cycle");
    }

    /// Pattern currently used for highlighting (live input or committed).
    pub fn active_search_pattern(&self) -> Option<&str> {
        self.search.active_pattern(self.mode == Mode::Search)
    }

    /// Commit live search input as the new pattern and leave Search mode.
    pub fn commit_search(&mut self) {
        let pattern = self.search.input.clone();
        if pattern.is_empty() {
            // Empty Enter reuses previous pattern (vim-like).
            if let Some(prev) = self.search.pattern.clone() {
                self.push_jump();
                self.recompute_search(&prev, false);
                if self.search.matches.is_empty() {
                    self.message = format!("Pattern not found: {}", prev);
                } else {
                    self.search_next();
                    self.message = format!(
                        "/{}/  {}/{}",
                        prev,
                        self.search.current + 1,
                        self.search.matches.len()
                    );
                }
            } else {
                self.message = String::from("No previous search pattern");
            }
        } else {
            let accepted_cursor = self.buffer.cursor();
            let accepted_match = self.search.current;
            self.push_jump();
            self.search.pattern = Some(pattern.clone());
            self.collect_matches(&pattern);
            if self.search.matches.is_empty() {
                self.message = format!("Pattern not found: {}", pattern);
            } else {
                // Live search already moved to the match the user selected.
                // Accepting the field must keep that match, not recompute from
                // the opening cursor and visibly jump back to match #1.
                self.search.current = self
                    .search
                    .matches
                    .get(accepted_match)
                    .filter(|position| **position == accepted_cursor)
                    .map(|_| accepted_match)
                    .or_else(|| {
                        self.search
                            .matches
                            .iter()
                            .position(|position| *position == accepted_cursor)
                    })
                    .or_else(|| self.search.nearest(accepted_cursor, self.search.forward))
                    .unwrap_or(0);
                self.buffer.cursor = self.search.matches[self.search.current];
                self.scroll_intent = ScrollIntent::Navigate;
                self.update_scroll();
                let slash = if self.search.forward { '/' } else { '?' };
                self.message = format!(
                    "{}{}/  {}/{}",
                    slash,
                    pattern,
                    self.search.current + 1,
                    self.search.matches.len()
                );
            }
        }
        self.search.finish();
        self.mode = Mode::Editor;
    }

    /// Cancel search: restore cursor, restore previous committed pattern.
    pub fn cancel_search(&mut self) {
        let (origin, scroll, restored) = self.search.cancel();
        if let Some(origin) = origin {
            self.buffer.cursor = origin;
            self.scroll = scroll;
        }
        if let Some(pat) = restored {
            // Rebuild match list for n/N without moving the restored cursor.
            self.collect_matches(&pat);
            let cur = self.buffer.cursor();
            if let Some(idx) = self
                .search
                .matches
                .iter()
                .position(|p| p.row == cur.row && p.col == cur.col)
            {
                self.search.current = idx;
            }
        }
        self.mode = Mode::Editor;
        self.message = String::from("Search cancelled");
    }

    /// Update live query while typing in Search mode.
    pub fn update_search_input(&mut self) {
        let pattern = self.search.input.clone();
        if pattern.is_empty() {
            self.search.matches.clear();
            self.search.current = 0;
            if let Some(origin) = self.search.origin {
                self.buffer.cursor = origin;
                self.scroll = self.search.scroll_origin;
            }
            self.message = String::from("Search — type to filter, Enter accept, Esc cancel");
            return;
        }
        self.recompute_search(&pattern, true);
        if self.search.matches.is_empty() {
            self.message = format!("/{}/  0 matches", pattern);
        } else {
            self.message = format!(
                "/{}/  {}/{}",
                pattern,
                self.search.current + 1,
                self.search.matches.len()
            );
        }
    }

    /// Replace the GUI find field's full value. Native AppKit text input owns
    /// IME composition and selection, so the face sends the resulting string
    /// rather than pretending each physical key is a Unicode character.
    pub fn set_search_input(&mut self, input: String) {
        if self.mode != Mode::Search || self.search.input == input {
            return;
        }
        self.search.input = input;
        self.update_search_input();
    }

    /// A character typed into the find bar.
    pub fn search_type(&mut self, c: char) {
        self.search.input.push(c);
        self.update_search_input();
    }

    /// Backspace in the find bar — an empty bar cancels (the gesture that
    /// dismisses an empty find).
    pub fn search_backspace(&mut self) {
        if self.search.input.is_empty() {
            self.cancel_search();
        } else {
            self.search.input.pop();
            self.update_search_input();
        }
    }

    /// ↑↓ in the find bar: cycle the live matches without committing.
    pub fn search_cycle(&mut self, forward: bool) {
        let Some(pos) = self.search.cycle(forward) else {
            return;
        };
        self.buffer.cursor = pos;
        self.scroll_intent = ScrollIntent::Navigate;
        self.update_scroll();
        self.message = format!(
            "/{}/  {}/{}",
            self.search.input,
            self.search.current + 1,
            self.search.matches.len()
        );
    }

    pub fn search_pattern_len_chars(&self) -> usize {
        self.search.pattern_len_chars(self.mode == Mode::Search)
    }

    /// Matches on `row` plus the global index of the first one. `search_matches`
    /// is built in row order by `collect_matches`, so this binary-searches the
    /// row's slice instead of the renderer scanning every match for every
    /// character of every visible line. The base index lets callers keep
    /// comparing against `search_current` (a global index).
    pub fn search_matches_row_slice(&self, row: usize) -> (usize, &[Position]) {
        self.search.row_slice(row)
    }

    pub fn is_current_search_match(&self, row: usize, col: usize) -> bool {
        self.search.is_current_match(row, col)
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
            (
                start,
                Position::new(prev, self.buffer.line(prev).chars().count()),
            )
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

    /// Recompute matches for `pattern`. If `jump`, move cursor to nearest match
    /// in the active search direction from origin/cursor.
    pub fn recompute_search(&mut self, pattern: &str, jump: bool) {
        self.collect_matches(pattern);
        let from = self.search.origin.unwrap_or_else(|| self.buffer.cursor());
        let Some(idx) = self.search.nearest(from, self.search.forward) else {
            self.search.current = 0;
            return;
        };
        self.search.current = idx;
        if jump {
            let pos = self.search.matches[idx];
            self.buffer.cursor = pos;
            self.scroll_intent = ScrollIntent::Navigate;
            self.update_scroll();
        }
    }

    fn collect_matches(&mut self, pattern: &str) {
        self.search.matches = crate::search::SearchState::collect(self.buffer.lines(), pattern);
    }

    /// Backward-compatible alias.
    pub fn perform_search(&mut self) {
        if let Some(pat) = self.search.pattern.clone() {
            self.recompute_search(&pat, true);
        }
    }

    pub fn search_next(&mut self) {
        // `n` follows the direction used when the pattern was committed.
        if self.search.forward {
            self.search_step(true);
        } else {
            self.search_step(false);
        }
    }

    pub fn search_prev(&mut self) {
        // `N` is opposite of search direction.
        if self.search.forward {
            self.search_step(false);
        } else {
            self.search_step(true);
        }
    }

    fn search_step(&mut self, forward: bool) {
        let Some(pat) = self.search.pattern.clone() else {
            self.message = String::from("No search pattern — press / or ? first");
            return;
        };
        let cur = self.buffer.cursor();
        self.collect_matches(&pat);
        let Some((idx, wrapped)) = self.search.step(cur, forward) else {
            self.message = format!("Pattern not found: {}", pat);
            return;
        };
        self.search.current = idx;
        let pos = self.search.matches[idx];
        self.buffer.cursor = pos;
        self.scroll_intent = ScrollIntent::Navigate;
        self.update_scroll();
        let slash = if self.search.forward { '/' } else { '?' };
        self.message = if wrapped {
            if forward {
                format!(
                    "search hit BOTTOM, continuing at TOP  {}/{}",
                    idx + 1,
                    self.search.matches.len()
                )
            } else {
                format!(
                    "search hit TOP, continuing at BOTTOM  {}/{}",
                    idx + 1,
                    self.search.matches.len()
                )
            }
        } else {
            format!(
                "{}{}/  {}/{}",
                slash,
                pat,
                idx + 1,
                self.search.matches.len()
            )
        };
    }

    /// Search for the word under the cursor (`*` in vim).
    pub fn search_word_under_cursor(&mut self) {
        let word = self.word_under_cursor();
        if word.is_empty() {
            self.message = String::from("No word under cursor");
            return;
        }
        self.push_jump();
        self.search.pattern = Some(word.clone());
        self.search.forward = true;
        self.recompute_search(&word, true);
        // Advance to next occurrence after current position
        if self.search.matches.len() > 1 {
            self.search_next();
        } else if self.search.matches.is_empty() {
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
            // Refuse to write text over something that is not text.
            //
            // Two questions, and they catch different files. The stored kind
            // is what we decided at open, which is the only thing that knows a
            // JPEG is a JPEG when its first 8 KiB happen to be NUL-free UTF-8.
            // The disk re-read catches the opposite case — a text file that
            // BECAME binary while it was open. Only the first 8 KiB, so it
            // costs nothing on a large document.
            if self.live_tab_kind().is_viewer() || file_looks_binary(&path) {
                self.set_message(&format!(
                    "Refusing to save: {} is not a text file",
                    path.display()
                ));
                return;
            }
            match atomic_write_file(&path, self.buffer.text()) {
                Ok(_) => {
                    self.mark_clean();
                    let idx = self.current_buffer();
                    if idx < self.tabs.buffers.len() {
                        self.tabs.buffers[idx].filename = Some(path.clone());
                    }
                    self.record_mtime();
                    self.refresh_git();
                    self.save_session();
                    self.set_message(&format!("✓ Saved: {}", path.display()));
                    self.fire_hook(crate::hooks::HookEvent::Save);
                }
                Err(e) => {
                    self.set_message(&format!("✗ Error: {}", e));
                }
            }
        } else {
            self.set_message("Untitled — choose a location to save it");
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
        let visible = self.grid_rows().max(1) as usize;
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
        let visible = self.grid_rows().max(1) as usize;
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
        let visible = self.grid_rows().max(1) as usize;
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
        let limit = self.max_hscroll();
        self.hscroll = cols.min(limit);
    }

    /// Furthest right the view may pan. Horizontal scrolling had no clamp at
    /// all, so a trackpad pan ran off past the end of the text into empty space
    /// forever — nothing anywhere knew where the content ended. One extra
    /// column of slack so the last glyph is not flush against the edge.
    pub fn max_hscroll(&mut self) -> usize {
        let width = usize::from(self.grid_cols().max(1));
        self.content_cols().saturating_sub(width).saturating_add(1)
    }

    /// The live document's wrap map at `cols` columns, built if the cached one
    /// no longer describes the document.
    ///
    /// `cols == 0` means wrapping is off, and produces the identity map — one
    /// visual row per line — so the face has one code path either way.
    ///
    /// The columns are the FACE's number. It knows the pane width in points,
    /// the cell width, the gutter and whatever overlays the right edge; core
    /// knows what a line measures. Neither can answer alone, and this is the
    /// seam.
    pub fn wrap_map(&self, cols: u16, wide: u16) -> std::cell::Ref<'_, crate::wrap::WrapMap> {
        let version = self.buffer.version();
        let tab = self.tab_width.max(1).min(u16::MAX as usize) as u16;
        if !self.wrap_map.borrow().is_valid_for(version, cols, tab, wide) {
            *self.wrap_map.borrow_mut() =
                crate::wrap::WrapMap::build(self.buffer.lines(), version, cols, tab, wide);
        }
        self.wrap_map.borrow()
    }

    /// Width of the document in display columns, as [`App::content_width`]
    /// defines it: raised by whatever is on screen now, never lowered.
    pub fn content_cols(&mut self) -> usize {
        let first = self.scroll;
        let last = (first + usize::from(self.grid_rows().max(1))).min(self.buffer.line_count());
        let tab = self.tab_width.max(1);
        let visible = (first..last)
            .map(|row| display_width(self.buffer.line(row), tab))
            .max()
            .unwrap_or(0);
        self.content_width = self.content_width.max(visible);
        self.content_width
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
        // Follow the FOCUSED pane, not the whole stage — under a split the pane
        // is a fraction of the grid, and measuring the stage let the caret
        // leave a half-size pane before this reacted (split H/V scroll follow).
        let (pane_rows, pane_cols) = self.focused_pane_grid();
        let visible_height = pane_rows.max(1);
        // Soft-wrap-aware: viewport width minus gutter (~5 cols).
        let text_width = pane_cols.saturating_sub(5).max(1);

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

        // NOTE: deliberately does NOT write into the focused pane's slot.
        // That slot is stale until focus leaves — see `park_focused_pane`.
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
                let deleted: String = line
                    .chars()
                    .skip(start.col)
                    .take(end.col.saturating_sub(start.col) + 1)
                    .collect();
                let prefix: String = line.chars().take(start.col).collect();
                let suffix: String = line.chars().skip(end.col + 1).collect();
                self.buffer.set_line(start.row, prefix + &suffix);
                deleted_text = deleted;
            } else {
                let first_chars: Vec<char> = self.buffer.line(start.row).chars().collect();
                let last_chars: Vec<char> = self.buffer.line(end.row).chars().collect();

                deleted_text.push_str(
                    &first_chars[start.col.min(first_chars.len())..]
                        .iter()
                        .collect::<String>(),
                );
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
        self.check_active_file_external();
        self.check_background_tabs_external();
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

    /// Note the rows a reload is about to replace.
    ///
    /// Common prefix and common suffix, and everything between them is the
    /// change. Not an LCS: a real diff would find the smallest edit script,
    /// and this is not a diff view — it is a notice, and the band between the
    /// first and last line that moved is exactly the region a reader needs to
    /// look at. It also cannot degrade: the worst case is a whole-file rewrite
    /// marking the whole file, which is the truth.
    ///
    /// Rows are in the NEW buffer's numbering, because that is what will be on
    /// screen when they are drawn.
    fn mark_live_rows(&mut self, next: &str) {
        let old = self.buffer.lines();
        // `split('\n')`, not `lines()`. The buffer keeps the empty line a
        // trailing newline creates and `str::lines()` drops it, so the two
        // sides were different lengths and the common SUFFIX never lined up —
        // every reload marked one row too many, right down to the last line.
        let new: Vec<&str> = next.split('\n').collect();

        let mut head = 0usize;
        while head < old.len() && head < new.len() && old[head] == new[head] {
            head += 1;
        }
        let mut tail = 0usize;
        while head + tail < old.len()
            && head + tail < new.len()
            && old[old.len() - 1 - tail] == new[new.len() - 1 - tail]
        {
            tail += 1;
        }

        self.live_rows.clear();
        let old_mid = old.len() - tail - head;
        let new_mid = new.len() - tail - head;

        // What the band DID, by what it did to the line count. With the prefix
        // and suffix trimmed the middle is one contiguous replacement, so its
        // net effect is the honest description: longer means lines arrived,
        // shorter means lines left, equal means the same lines say something
        // else now.
        let kind = if new_mid > old_mid {
            crate::LiveKind::Added
        } else if new_mid < old_mid {
            crate::LiveKind::Removed
        } else {
            crate::LiveKind::Changed
        };

        self.live_removed = 0;
        if new_mid == 0 {
            // Nothing arrived to mark. The row that closed over the gap is
            // where the reader should look, and it is the only place a
            // removal can be pointed at.
            let row = head.min(new.len().saturating_sub(1));
            if !new.is_empty() {
                self.live_rows.insert(row, crate::LiveKind::Removed);
                self.live_removed = (old_mid - new_mid).min(u16::MAX as usize) as u16;
            }
        } else {
            for row in head..head + new_mid {
                self.live_rows.insert(row, kind);
            }
            if old_mid > new_mid {
                // Shrank without emptying: the band is still there, just
                // shorter, and the rows below still have that much to travel.
                self.live_removed = (old_mid - new_mid).min(u16::MAX as usize) as u16;
            }
        }

        self.live_gen = self.live_gen.wrapping_add(1);
        self.live_marked_at = if self.live_rows.is_empty() {
            None
        } else {
            Some(std::time::Instant::now())
        };
    }

    /// Live marks are a flash, not a state. Dropped once the face has had time
    /// to run the fade, so a row does not stay marked for the rest of the
    /// session — and so the sign byte stops carrying a bit nothing is using.
    pub fn expire_live_marks(&mut self) {
        let mut changed = false;
        if let Some(at) = self.live_marked_at {
            if at.elapsed() >= std::time::Duration::from_millis(1_600) {
                self.live_rows.clear();
                self.live_marked_at = None;
                changed = true;
            }
        }
        // The tree's mark outlives the editor's: a file you are not looking at
        // is worth pointing out for longer than a row already on screen.
        let before = self.live_files.len();
        self.live_files
            .retain(|_, at| at.elapsed() < std::time::Duration::from_millis(3_000));
        if self.live_files.len() != before {
            changed = true;
        }
        if changed {
            self.live_gen = self.live_gen.wrapping_add(1);
        }
    }

    /// True while anything is still marked — the tick uses this to know
    /// whether expiry has any work to do at all.
    pub fn has_live_marks(&self) -> bool {
        self.live_marked_at.is_some() || !self.live_files.is_empty()
    }

    /// The same live refresh, for every OTHER open document.
    ///
    /// `check_active_file_external` watches one file: the focused pane's. That
    /// was the whole of "live reload", and it is not enough now — an agent
    /// rewrites six files and only the one being stared at catches up, while a
    /// split pane beside it keeps showing text that is no longer on disk. A
    /// document that is open is a document being watched.
    ///
    /// Deliberately narrower than the active path in what it will overwrite. A
    /// background tab reloads only when it is CLEAN. Unsaved text nobody is
    /// looking at is the easiest thing in the editor to destroy silently, and
    /// the active path's "disk won" rule at least happens in front of someone.
    /// A dirty tab keeps its copy and finds out when it is focused.
    ///
    /// One `stat` per open document per poll. At 64 tabs and a poll every
    /// ~167ms that is under 400 a second, which is the cheapest thing this
    /// tick does.
    fn check_background_tabs_external(&mut self) {
        let live = self.live_doc;
        let mut reloaded = 0usize;
        for i in 0..self.tabs.buffers.len() {
            let tab = &self.tabs.buffers[i];
            // The focused document is the active path's job, terminals have no
            // file, and a viewer draws from the path rather than the buffer.
            if tab.id == live || tab.terminal.is_some() || tab.kind.is_viewer() {
                continue;
            }
            let Some(path) = tab.filename.clone() else { continue };
            let Some(prev) = tab.file_mtime else { continue };
            let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
                // Missing or unreadable. The active path owns the deletion
                // policy, including its two-poll confirmation; guessing at it
                // for a tab nobody is looking at would only race that.
                continue;
            };
            if mtime == prev {
                continue;
            }
            if tab.modified {
                // Record the new time anyway, or this fires on every poll for
                // as long as the tab stays dirty.
                self.tabs.buffers[i].file_mtime = Some(mtime);
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                self.tabs.buffers[i].file_mtime = Some(mtime);
                continue;
            };
            let tab = &mut self.tabs.buffers[i];
            let cursor = tab.buffer.cursor();
            tab.buffer = Buffer::from_string(&content);
            tab.buffer.cursor.row = cursor
                .row
                .min(tab.buffer.line_count().saturating_sub(1));
            tab.buffer.cursor.col = cursor.col;
            tab.buffer.clamp_col();
            tab.scroll = tab.scroll.min(tab.buffer.line_count().saturating_sub(1));
            tab.saved_hash = text_hash(&content);
            tab.modified = false;
            tab.file_mtime = Some(mtime);
            // A reloaded document has no history worth keeping: undoing into
            // text that is not what anyone wrote is worse than having no undo.
            tab.undo_stack = UndoStack::new();
            tab.undo_stack.push(tab.buffer.snapshot());
            self.live_files.insert(path.clone(), std::time::Instant::now());
            reloaded += 1;
        }
        if reloaded > 0 {
            self.live_gen = self.live_gen.wrapping_add(1);
            // Only the count. Naming one of six is arbitrary, and the tabs
            // themselves are where the change is visible.
            self.set_message(&if reloaded == 1 {
                "↻ Reloaded 1 file changed on disk".to_string()
            } else {
                format!("↻ Reloaded {reloaded} files changed on disk")
            });
        }
    }

    fn check_active_file_external(&mut self) {
        let Some(path) = self.filename.clone() else {
            return;
        };
        let path_s = path.display().to_string();
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                // A clean vanished file closes after two consecutive misses.
                // A dirty one survives with a Save-to-restore affordance.
                if self.file_mtime.is_some() {
                    if !self.file_deleted {
                        // Atomic saves briefly exchange directory entries; a
                        // single metadata miss must not eject a healthy tab.
                        self.file_deleted = true;
                        self.message = format!("⚠ Missing on disk — {path_s}");
                        return;
                    }
                    if !self.modified {
                        let id = self.current_buffer_id();
                        self.close_tab_id(id);
                        self.message = format!("Closed deleted file — {path_s}");
                        return;
                    }
                    self.dirty_needs_recheck = false;
                    let idx = self.current_buffer();
                    if idx < self.tabs.buffers.len() {
                        self.tabs.buffers[idx].modified = true;
                    }
                    self.message = format!("⚠ Deleted on disk · Save to restore — {path_s}");
                }
                return;
            }
        };
        // The path exists again (re-created, or was never gone) — clear the flag.
        if self.file_deleted {
            self.file_deleted = false;
            self.message = format!("File back on disk — {path_s}");
        }
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

        // Which rows this actually changes, worked out BEFORE the old text is
        // dropped — afterwards there is nothing left to compare against.
        self.mark_live_rows(&content);
        self.live_files.insert(path.clone(), std::time::Instant::now());

        self.buffer = Buffer::from_string(&content);
        // Restore cursor within new bounds
        self.buffer.cursor.row = cursor.row.min(self.buffer.line_count().saturating_sub(1));
        self.buffer.cursor.col = cursor.col;
        self.buffer.clamp_col();
        self.scroll = scroll.min(self.buffer.line_count().saturating_sub(1));
        self.mark_clean();
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
            self.lsp.clear_diagnostics();
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
                for tab in &mut self.tabs.buffers {
                    if tab
                        .filename
                        .as_ref()
                        .map(|p| p.display().to_string() == edit.path)
                        .unwrap_or(false)
                    {
                        tab.buffer = crate::buffer::Buffer::from_string(&edit.text);
                        tab.modified = false;
                        tab.saved_hash = text_hash(&edit.text);
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
}

pub use crate::fs_atomic::atomic_write_file;

#[cfg(test)]
mod tests {
    use super::*;

    /// Give the app `docs.len()` side-by-side panes, one per document.
    ///
    /// Assigning `split.panes` directly no longer works: the tree owns the
    /// structure and the vector is a view of it, so a hand-built vector has no
    /// matching leaves.
    fn panes_on(app: &mut App, docs: &[BufferId]) {
        for _ in 1..docs.len() {
            app.split.split_focused(crate::split::Axis::Col);
        }
        for (p, id) in app.split.panes.iter_mut().zip(docs) {
            p.buffer = *id;
        }
        app.split.set_focus(0);
    }

    fn app_with(text: &str) -> App {
        let mut app = App::new();
        app.buffer = Buffer::from_string(text);
        app.stage = Stage {
            w: 720.0,
            h: 432.0,
            ..Stage::default()
        }; // 80×24 cells
        app
    }

    #[test]
    fn a_rename_repoints_the_open_tab() {
        let mut app = App::new();
        app.tabs.buffers[0].filename = Some(PathBuf::from("/p/src/a.rs"));
        app.filename = Some(PathBuf::from("/p/src/a.rs"));
        let n = app.path_moved(Path::new("/p/src/a.rs"), Path::new("/p/src/b.rs"));
        assert_eq!(n, 1);
        assert_eq!(app.filename.as_deref(), Some(Path::new("/p/src/b.rs")));
    }

    /// Moving a folder has to carry every file open beneath it, or those tabs
    /// keep pointing at paths that no longer exist.
    #[test]
    fn a_folder_move_carries_the_files_open_under_it() {
        let mut app = App::new();
        // Set the live filename first: `open_blank_tab` flushes it into the
        // current tab before pushing the new one.
        app.filename = Some(PathBuf::from("/p/old/deep/a.rs"));
        app.open_blank_tab();
        app.tabs.buffers[1].filename = Some(PathBuf::from("/p/old/b.rs"));
        app.filename = Some(PathBuf::from("/p/old/deep/a.rs"));
        let n = app.path_moved(Path::new("/p/old"), Path::new("/p/new"));
        assert_eq!(n, 2);
        assert_eq!(
            app.tabs.buffers[0].filename.as_deref(),
            Some(Path::new("/p/new/deep/a.rs"))
        );
        assert_eq!(
            app.tabs.buffers[1].filename.as_deref(),
            Some(Path::new("/p/new/b.rs"))
        );
    }

    #[test]
    fn an_unrelated_path_is_left_alone() {
        let mut app = App::new();
        app.tabs.buffers[0].filename = Some(PathBuf::from("/other/a.rs"));
        assert_eq!(app.path_moved(Path::new("/p/old"), Path::new("/p/new")), 0);
        assert_eq!(
            app.tabs.buffers[0].filename.as_deref(),
            Some(Path::new("/other/a.rs"))
        );
    }

    #[test]
    fn references_result_enriches_with_preview_and_ready() {
        let mut app = app_with("fn main() {\n    let x = foo();\n    foo();\n}");
        app.filename = Some(PathBuf::from("/tmp/refs_test.rs"));
        // Simulate the async LSP answer landing.
        app.lsp.pending_references = vec![
            crate::lsp::Location {
                path: "/tmp/refs_test.rs".into(),
                row: 1,
                col: 12,
            },
            crate::lsp::Location {
                path: "/tmp/refs_test.rs".into(),
                row: 2,
                col: 4,
            },
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
            .filter(|e| e.file_name().to_string_lossy().contains("suisei-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "tmp files leaked: {leftovers:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ime_commit_text_survives_buffer_and_atomic_save() {
        let dir = std::env::temp_dir().join(format!(
            "suisei-ime-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("ime.txt");

        // EditorCanvasView resolves a visible IME composition through the
        // engine paste-text entry point immediately before Save. Exercise the
        // same Core path with precomposed Hangul, decomposed jamo, Japanese,
        // and a multi-scalar emoji so a future buffer migration cannot regress
        // to byte/scalar truncation.
        let committed = "한글 한글 日本語 👨‍👩‍👧‍👦";
        let mut app = app_with("prefix ");
        app.buffer.move_to_line_end();
        app.paste_text_at_cursor(committed);
        app.filename = Some(path.clone());
        app.save_file();

        let expected = format!("prefix {committed}");
        assert_eq!(app.buffer.text(), expected);
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert!(!app.modified);

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
            app.message.contains("Format")
                || app.message.contains("…")
                || app.message.contains("Formatting"),
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

    /// A tab-bar drag reshuffles `buffers`. Panes hold a `BufferId`, so a
    /// document keeps its pane no matter where it lands in the strip — this
    /// pins that, and that the derived active tab follows the move.
    #[test]
    fn moving_a_tab_carries_the_active_index_and_every_pane() {
        let mut app = app_with("a");
        for name in ["b", "c", "d"] {
            let tab_id = app.take_tab_id();
            app.tabs.buffers.push(BufferTab {
                id: tab_id,
                buffer: crate::buffer::Buffer::from_string(name),
                filename: Some(PathBuf::from(format!("/tmp/{name}.txt"))),
                scroll: 0,
                modified: false,
                saved_hash: EMPTY_TEXT_HASH,
                undo_stack: UndoStack::new(),
                file_mtime: None,
                terminal: None,
                terminal_title: None,
                kind: crate::media::FileKind::Text,
                terminal_cwd: None,
            });
        }
        app.save_state_to_tab(); // tab 0 mirrors the active buffer
        let names = |app: &App| -> Vec<String> {
            app.tabs
                .buffers
                .iter()
                .map(|t| t.buffer.line(0).to_string())
                .collect()
        };
        assert_eq!(names(&app), ["a", "b", "c", "d"]);

        // Three panes, one on each of the first three tabs.
        let ids: Vec<BufferId> = app.tabs.buffers.iter().take(3).map(|t| t.id).collect();
        panes_on(&mut app, &ids);
        app.goto_tab(1); // park + restore properly; a bare assignment leaves
        // `App.buffer` showing a different tab's text
        assert!(app.move_tab(0, 2));
        assert_eq!(names(&app), ["b", "c", "a", "d"]);
        assert_eq!(
            app.current_buffer(),
            0,
            "the active tab followed its document"
        );
        // The focused pane shows the live document — `goto_tab(1)` repointed
        // it at b the moment b became active — so the panes show [b, b, c].
        // The move preserves every pane's document; only positions shifted.
        let panes: Vec<usize> = app.split.panes.iter().map(|p| app.pane_tab(p)).collect();
        assert_eq!(
            panes,
            [0, 0, 1],
            "every pane still shows the file it showed"
        );
    }

    /// Closing a tab that is not the active one leaves the editor alone.
    ///
    /// The strip's close button used to `goto_tab(idx)` first, which made the
    /// doomed document active and then had to put the editor back from
    /// information it had already overwritten.
    #[test]
    fn closing_an_inactive_tab_does_not_move_the_editor() {
        let mut app = app_with("a");
        for name in ["b", "c"] {
            let tab_id = app.take_tab_id();
            app.tabs.buffers.push(BufferTab {
                id: tab_id,
                buffer: crate::buffer::Buffer::from_string(name),
                filename: Some(PathBuf::from(format!("/tmp/{name}.txt"))),
                scroll: 0,
                modified: false,
                saved_hash: EMPTY_TEXT_HASH,
                undo_stack: UndoStack::new(),
                file_mtime: None,
                terminal: None,
                terminal_title: None,
                kind: crate::media::FileKind::Text,
                terminal_cwd: None,
            });
        }
        app.save_state_to_tab();
        app.goto_tab(2); // c is active
        assert_eq!(app.buffer.line(0), "c");

        app.close_tab_at(0); // drop a
        assert_eq!(app.tabs.buffers.len(), 2);
        assert_eq!(app.buffer.line(0), "c", "the editor still shows c");
        assert_eq!(app.current_buffer(), 1, "and the index followed c's move");

        // Closing the active one still behaves like the old path.
        app.close_tab_at(1);
        assert_eq!(app.tabs.buffers.len(), 1);
        assert_eq!(app.buffer.line(0), "b");
    }

    /// Switching between members of the ACTIVE layout must keep the multi-pane
    /// desk — not park+collapse. Collapse made ⌃⇥ / chip hops look like
    /// "leaving", after which a free re-split + tab change re-armed layout save.
    #[test]
    fn goto_tab_between_active_layout_members_keeps_the_split() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids[..2]);
        assert!(app.fold_layout());
        assert_eq!(app.split.pane_count(), 2);
        assert_eq!(app.active_layout, Some(app.layouts[0].id));

        // Hop to the other member.
        app.goto_tab_id(ids[1]);
        assert_eq!(
            app.active_layout,
            Some(app.layouts[0].id),
            "still on the desk"
        );
        assert_eq!(app.split.pane_count(), 2, "must not collapse");
        assert_eq!(app.current_buffer_id(), ids[1]);

        // Outside member still leaves and clears the desk.
        app.goto_tab_id(ids[2]);
        assert_eq!(app.active_layout, None);
        assert_eq!(app.split.pane_count(), 1);
        assert_eq!(app.current_buffer_id(), ids[2]);
        assert_eq!(app.layouts.len(), 1, "arrangement parked");
    }

    /// After leaving a layout (desk cleared), a FREE multi-pane split on a
    /// single tab must survive switching to a non-member — only an active
    /// layout may collapse the desk. (Chip clicks that would re-activate a
    /// parked layout over a free split are face-side; core goto never
    /// activates.)
    #[test]
    fn free_split_after_leaving_a_layout_does_not_collapse_on_goto() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids[..2]);
        assert!(app.fold_layout());
        // Leave: park + single c.
        app.goto_tab_id(ids[2]);
        assert_eq!(app.split.pane_count(), 1);
        assert_eq!(app.active_layout, None);
        assert_eq!(app.layouts.len(), 1);

        // Free work: split on c, open b in the other pane without activating.
        app.split.split_focused(crate::split::Axis::Col);
        app.split.panes[1].buffer = ids[1];
        assert_eq!(app.split.pane_count(), 2);
        assert_eq!(app.active_layout, None);

        // Switch focused pane to a (still a parked-layout member) via goto —
        // free split must remain; only the focused pane's document changes.
        app.split.set_focus(0);
        app.goto_tab_id(ids[0]);
        assert_eq!(app.active_layout, None, "goto does not activate layouts");
        assert_eq!(app.split.pane_count(), 2, "free split survives");
        assert_eq!(app.split.panes[0].buffer, ids[0]);
        assert_eq!(app.split.panes[1].buffer, ids[1]);
    }

    /// J7's four transitions: fold, switch away, switch back, unfold.
    #[test]
    fn folding_parks_the_arrangement_and_leaving_clears_the_desk() {
        let mut app = app_with("a");
        for name in ["b", "c"] {
            let tab_id = app.take_tab_id();
            app.tabs.buffers.push(BufferTab {
                id: tab_id,
                buffer: crate::buffer::Buffer::from_string(name),
                filename: Some(PathBuf::from(format!("/tmp/{name}.txt"))),
                scroll: 0,
                modified: false,
                saved_hash: EMPTY_TEXT_HASH,
                undo_stack: UndoStack::new(),
                file_mtime: None,
                terminal: None,
                terminal_title: None,
                kind: crate::media::FileKind::Text,
                terminal_cwd: None,
            });
        }
        app.save_state_to_tab();
        let (a_id, b_id) = (app.tabs.buffers[0].id, app.tabs.buffers[1].id);
        panes_on(&mut app, &[a_id, b_id]);
        app.goto_tab(0);
        assert_eq!(app.split.pane_count(), 2);

        // 1 · Fold. Deliberately quiet — the arrangement stays on screen.
        assert!(app.fold_layout(), "two panes are an arrangement");
        assert_eq!(app.layouts.len(), 1);
        assert_eq!(
            app.split.pane_count(),
            2,
            "the fold changes nothing on screen"
        );
        let layout_id = app.layouts[0].id;
        assert_eq!(app.active_layout, Some(layout_id));
        assert_eq!(app.layout_docs(app.layouts[0].id), vec![a_id, b_id]);

        // 2 · Switch away — the desk clears.
        app.goto_tab(2); // c
        assert_eq!(app.active_layout, None);
        assert_eq!(
            app.split.pane_count(),
            1,
            "editor comes down to one document"
        );
        assert!(!app.split.is_split());
        assert_eq!(
            app.layouts.len(),
            1,
            "the arrangement is not lost, it is parked"
        );

        // 3 · Switch back — the arrangement returns.
        assert!(app.activate_layout(layout_id, None));
        assert_eq!(app.split.pane_count(), 2);
        let shown: Vec<usize> = app.split.panes.iter().map(|p| app.pane_tab(p)).collect();
        assert_eq!(shown, vec![0, 1], "both panes on the documents they had");

        // 4 · Unfold — the layout is gone, the arrangement stays put.
        assert!(app.unfold_layout());
        assert!(app.layouts.is_empty());
        assert_eq!(app.active_layout, None);
        assert_eq!(
            app.split.pane_count(),
            2,
            "unfolding is as quiet as folding"
        );
    }

    /// A single pane is not an arrangement — folding it would just hide a file
    /// behind a name.
    #[test]
    fn folding_refuses_when_there_is_nothing_to_fold() {
        let mut app = app_with("a");
        app.save_state_to_tab();
        assert!(!app.split.is_split());
        assert!(!app.fold_layout());
        assert!(app.layouts.is_empty());
    }

    #[test]
    fn folding_refuses_two_panes_showing_the_same_document() {
        let mut app = app_with("a");
        app.save_state_to_tab();
        let a_id = app.tabs.buffers[0].id;
        panes_on(&mut app, &[a_id, a_id]);

        assert!(app.split.is_split());
        assert!(!app.fold_layout());
        assert!(app.layouts.is_empty());
        assert_eq!(
            app.message,
            "A layout needs at least two different documents"
        );
    }

    #[test]
    fn a_layout_switches_between_its_two_strip_shapes() {
        let mut app = app_with("a");
        let tab_id = app.take_tab_id();
        app.tabs.buffers.push(BufferTab {
            id: tab_id,
            buffer: crate::buffer::Buffer::from_string("b"),
            filename: Some(PathBuf::from("/tmp/b.txt")),
            scroll: 0,
            modified: false,
            saved_hash: EMPTY_TEXT_HASH,
            undo_stack: UndoStack::new(),
            file_mtime: None,
            terminal: None,
            terminal_title: None,
            kind: crate::media::FileKind::Text,
            terminal_cwd: None,
        });
        app.save_state_to_tab();
        let (a_id, b_id) = (app.tabs.buffers[0].id, app.tabs.buffers[1].id);
        panes_on(&mut app, &[a_id, b_id]);
        app.goto_tab(0);
        assert!(app.fold_layout());

        let id = app.layouts[0].id;
        assert_eq!(
            app.layouts[0].style,
            crate::layout_tab::LayoutStyle::Grouped
        );
        assert!(app.toggle_layout_style(id));
        assert_eq!(
            app.layouts[0].style,
            crate::layout_tab::LayoutStyle::Unified
        );
        assert_eq!(
            app.message,
            "Layout unified · scroll down to show member tabs"
        );
        assert!(app.toggle_layout_style(id));
        assert_eq!(
            app.layouts[0].style,
            crate::layout_tab::LayoutStyle::Grouped
        );
        assert_eq!(
            app.message,
            "Layout group expanded · scroll up to unify · down to unfold"
        );
    }

    /// The other direction, and the no-ops.
    #[test]
    fn moving_a_tab_backwards_shifts_the_slots_it_passes() {
        let mut app = app_with("a");
        for name in ["b", "c"] {
            let tab_id = app.take_tab_id();
            app.tabs.buffers.push(BufferTab {
                id: tab_id,
                buffer: crate::buffer::Buffer::from_string(name),
                filename: Some(PathBuf::from(format!("/tmp/{name}.txt"))),
                scroll: 0,
                modified: false,
                saved_hash: EMPTY_TEXT_HASH,
                undo_stack: UndoStack::new(),
                file_mtime: None,
                terminal: None,
                terminal_title: None,
                kind: crate::media::FileKind::Text,
                terminal_cwd: None,
            });
        }
        app.save_state_to_tab();
        let c_id = app.tabs.buffers[2].id;
        panes_on(&mut app, &[c_id]);
        app.goto_tab(2);

        // c moves to the front: a b c → c a b
        assert!(app.move_tab(2, 0));
        let names: Vec<String> = app
            .tabs
            .buffers
            .iter()
            .map(|t| t.buffer.line(0).to_string())
            .collect();
        assert_eq!(names, ["c", "a", "b"]);
        assert_eq!(app.current_buffer(), 0);
        assert_eq!(app.pane_tab(&app.split.panes[0]), 0);

        assert!(!app.move_tab(1, 1), "a move onto itself is a no-op");
        assert!(!app.move_tab(0, 9), "out of range is refused");
        assert!(!app.move_tab(9, 0));
    }

    /// Give the app `n` named tabs (the first reuses the one `app_with` made)
    /// and return their ids in strip order.
    fn tabs_named(app: &mut App, names: &[&str]) -> Vec<BufferId> {
        app.buffer = Buffer::from_string(names[0]);
        app.tabs.buffers[0].buffer = app.buffer.clone();
        app.tabs.buffers[0].filename = Some(PathBuf::from(format!("/tmp/{}.txt", names[0])));
        for name in &names[1..] {
            let tab_id = app.take_tab_id();
            app.tabs.buffers.push(BufferTab {
                id: tab_id,
                buffer: Buffer::from_string(*name),
                filename: Some(PathBuf::from(format!("/tmp/{name}.txt"))),
                scroll: 0,
                modified: false,
                saved_hash: EMPTY_TEXT_HASH,
                undo_stack: UndoStack::new(),
                file_mtime: None,
                terminal: None,
                terminal_title: None,
                kind: crate::media::FileKind::Text,
                terminal_cwd: None,
            });
        }
        app.save_state_to_tab();
        app.tabs.buffers.iter().map(|t| t.id).collect()
    }

    /// Folding gathers its members into one contiguous run, so the grouped
    /// strip container never swallows a tab that sat between two of them.
    /// Panes on 1·2·3·5 with 4 loose in the strip: the fold pulls 5 up beside
    /// 3 and leaves 4 after the group — not inside it.
    #[test]
    fn folding_gathers_its_members_into_one_contiguous_run() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c", "d", "e"]);
        // Panes show a·b·c·e; d is loose.
        panes_on(&mut app, &[ids[0], ids[1], ids[2], ids[4]]);
        app.goto_tab(0);
        assert!(app.fold_layout());

        let names: Vec<String> = app
            .tabs
            .buffers
            .iter()
            .map(|t| t.buffer.line(0).to_string())
            .collect();
        assert_eq!(
            names,
            ["a", "b", "c", "e", "d"],
            "members gathered, d pushed after"
        );
        assert_eq!(app.current_buffer(), 0, "the active tab kept its identity");
        // The panes still show exactly what they showed — gathering only moved
        // the strip order, never a pane's document.
        let shown: Vec<usize> = app.split.panes.iter().map(|p| app.pane_tab(p)).collect();
        assert_eq!(
            shown,
            vec![0, 1, 2, 3],
            "panes follow their documents by id"
        );
    }

    /// A reorder that would break a folded group is refused — both dragging an
    /// outside tab into the group's run and dragging a member out of it.
    #[test]
    fn a_reorder_cannot_break_a_folded_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c", "d", "e"]);
        panes_on(&mut app, &[ids[0], ids[1], ids[2], ids[4]]);
        app.goto_tab(0);
        assert!(app.fold_layout());
        // Strip is now a b c e | d, with a·b·c·e grouped.
        let names = |app: &App| -> Vec<String> {
            app.tabs
                .buffers
                .iter()
                .map(|t| t.buffer.line(0).to_string())
                .collect()
        };
        assert_eq!(names(&app), ["a", "b", "c", "e", "d"]);

        // Dragging d (slot 4) into the middle of the group is refused.
        assert!(!app.move_tab(4, 1), "an outside tab cannot enter the group");
        assert_eq!(names(&app), ["a", "b", "c", "e", "d"], "strip unchanged");

        // Dragging a member (b, slot 1) out past d is refused too.
        assert!(!app.move_tab(1, 4), "a member cannot leave the group");
        assert_eq!(names(&app), ["a", "b", "c", "e", "d"]);

        // Reordering WITHIN the group is fine — the run stays contiguous.
        assert!(app.move_tab(0, 2), "members may reorder among themselves");
        assert_eq!(names(&app), ["b", "c", "a", "e", "d"]);
    }

    /// Open a new file while a folded layout is active and one pane focused:
    /// the focused pane takes the new document, which joins the group in the
    /// displaced document's place — the displaced one leaves the group rather
    /// than lingering inside it shown by no pane, and the new one does not
    /// appear as a loose chip outside.
    #[test]
    fn opening_a_file_swaps_the_focused_pane_in_and_out_of_the_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c", "d"]);
        panes_on(&mut app, &ids); // four panes, one per document
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        // Focus the 4th pane (document d). Folding is silent — the arrangement
        // stays on screen — so this is a pane click, not a tab switch.
        app.focus_pane_to(3);
        assert_eq!(app.pane_tab(&app.split.focused_pane()), 3);

        // Open a brand-new file into the focused pane.
        let dir = std::env::temp_dir().join("suisei-open-swap");
        let _ = std::fs::create_dir_all(&dir);
        let new_path = dir.join("e.txt");
        std::fs::write(&new_path, "e").unwrap();
        app.open_new_tab(new_path.to_str().unwrap());

        // The focused pane now shows the new document — its slot names the
        // live document from the moment it becomes active, and the compositor
        // reads it that way too.
        let new_id = app.current_buffer_id();
        assert_ne!(
            new_id, ids[3],
            "focused pane took the new file, not the old"
        );

        // …and the group's membership followed: d out, e in.
        let docs = app.layout_docs(layout_id);
        assert!(docs.contains(&new_id), "the new file joined the group");
        assert!(!docs.contains(&ids[3]), "the displaced file left the group");
        assert_eq!(docs.len(), 4, "membership size is unchanged");

        // The members must be contiguous in the strip, or the grey container
        // swallows whatever sits between them. e was pushed at the end of
        // buffers; gather pulls it back beside the other members.
        let strip: Vec<BufferId> = app.tabs.buffers.iter().map(|t| t.id).collect();
        let first = strip.iter().position(|id| docs.contains(id)).unwrap();
        let run = &strip[first..first + docs.len()];
        assert!(
            run.iter().all(|id| docs.contains(id)),
            "members form one contiguous run in the strip: {strip:?}"
        );
        let _ = std::fs::remove_file(&new_path);
    }

    /// Activating a layout with a named document brings THAT pane to the
    /// front, not always the first one in the tree — so clicking a grouped
    /// chip lands on the document it represents.
    #[test]
    fn activating_a_layout_focuses_the_named_document() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c", "d", "outside"]);
        panes_on(&mut app, &ids[..4]);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        // Leave through a document outside the group. Switching between group
        // members intentionally keeps the desk alive now.
        app.goto_tab_id(ids[4]);
        assert_eq!(app.active_layout, None);
        assert!(!app.split.is_split());

        // Come back asking for the 3rd document — its pane takes focus.
        assert!(app.activate_layout(layout_id, Some(ids[2])));
        assert_eq!(app.split.pane_count(), 4);
        assert_eq!(
            app.current_buffer_id(),
            ids[2],
            "the named document is focused"
        );
        assert_eq!(app.split.focus_index(), 2);

        // And with no preference, the tree's own first pane leads.
        app.goto_tab(0);
        assert!(app.activate_layout(layout_id, None));
        assert_eq!(app.split.focus_index(), 0);
    }

    /// Closing a tab that a pane is showing **removes that pane** — it does
    /// not repoint the pane at a neighbour (that produced B|B ghosts).
    ///
    /// Closing a tab that **no** pane is showing leaves the split alone.
    /// That half is the original S1 gate (index-addressed panes used to slide
    /// when an unrelated earlier tab closed).
    #[test]
    fn closing_a_tab_removes_its_pane_and_leaves_unrelated_panes() {
        let mut app = app_with("a");
        for name in ["b", "c", "d"] {
            let tab_id = app.take_tab_id();
            app.tabs.buffers.push(BufferTab {
                id: tab_id,
                buffer: crate::buffer::Buffer::from_string(name),
                filename: Some(PathBuf::from(format!("/tmp/{name}.txt"))),
                scroll: 0,
                modified: false,
                saved_hash: EMPTY_TEXT_HASH,
                undo_stack: UndoStack::new(),
                file_mtime: None,
                terminal: None,
                terminal_title: None,
                kind: crate::media::FileKind::Text,
                terminal_cwd: None,
            });
        }
        app.save_state_to_tab();
        let shown = |app: &App| -> Vec<String> {
            app.split
                .panes
                .iter()
                .map(|p| app.tabs.buffers[app.pane_tab(p)].buffer.line(0).to_string())
                .collect()
        };

        // Two panes on c and d; focus a (repoints the focused pane to a).
        let (c_id, d_id) = (app.tabs.buffers[2].id, app.tabs.buffers[3].id);
        panes_on(&mut app, &[c_id, d_id]);
        app.goto_tab(0);
        assert_eq!(shown(&app), ["a", "d"]);

        // Close a — the pane that showed a is removed; d's pane survives alone.
        app.close_current_tab();
        assert_eq!(
            app.tabs
                .buffers
                .iter()
                .map(|t| t.buffer.line(0).to_string())
                .collect::<Vec<_>>(),
            ["b", "c", "d"]
        );
        assert_eq!(app.split.pane_count(), 1, "a's pane removed, not repointed");
        assert_eq!(shown(&app), ["d"], "only d remains");

        // Unrelated close: split on b|d, close c (shown nowhere) → still b|d.
        let b_id = app
            .tabs
            .buffers
            .iter()
            .find(|t| t.buffer.line(0) == "b")
            .unwrap()
            .id;
        let d_id = app
            .tabs
            .buffers
            .iter()
            .find(|t| t.buffer.line(0) == "d")
            .unwrap()
            .id;
        let c_id = app
            .tabs
            .buffers
            .iter()
            .find(|t| t.buffer.line(0) == "c")
            .unwrap()
            .id;
        panes_on(&mut app, &[b_id, d_id]);
        assert_eq!(shown(&app), ["b", "d"]);
        app.close_tab_id(c_id);
        assert_eq!(shown(&app), ["b", "d"], "unrelated close leaves the split");
        assert_eq!(app.split.pane_count(), 2);
    }

    /// `modified` was a one-way latch: set by every edit, cleared only by a
    /// save. Undoing back to the original text left the file marked dirty
    /// forever, so the tab dot lied and closing prompted about nothing.
    /// The horizontal pan had no right-hand limit anywhere: core never clamped
    /// it, and the face sized its scroll canvas as `hscroll + 160`, so each pan
    /// widened the document and the end receded forever.
    #[test]
    fn horizontal_scroll_stops_at_the_end_of_the_text() {
        let mut app = app_with("short\na much longer line of text here\nmid");
        app.wrap_lines = false;
        app.stage.w = 90.0;
        app.stage.h = 54.0;

        let widest = "a much longer line of text here".chars().count();
        assert_eq!(app.content_cols(), widest);
        assert_eq!(app.max_hscroll(), widest - 10 + 1);

        app.set_hscroll(100_000);
        assert_eq!(
            app.hscroll,
            widest - 10 + 1,
            "panning past the text is clamped"
        );
    }

    /// Tabs advance to the next stop and CJK takes two cells — the extent has
    /// to agree with what is painted or the clamp lands short of the last glyph.
    #[test]
    fn the_scroll_extent_counts_display_columns_not_characters() {
        let mut app = app_with("\t\tab");
        app.wrap_lines = false;
        app.tab_width = 4;
        app.stage.w = 36.0;
        app.stage.h = 18.0;
        assert_eq!(app.content_cols(), 10, "two tab stops plus two letters");

        let mut cjk = app_with("한글이다");
        cjk.wrap_lines = false;
        cjk.stage.w = 18.0;
        cjk.stage.h = 18.0;
        assert_eq!(cjk.content_cols(), 8, "four wide glyphs, two cells each");
    }

    /// The extent must not shrink while the user scrolls, or the scroller thumb
    /// resizes under their hand — so it is a high-water mark within a document.
    #[test]
    fn the_scroll_extent_never_shrinks_within_a_document() {
        let mut app = app_with("a very long first line indeed\nx\ny\nz");
        app.wrap_lines = false;
        app.stage.w = 72.0;
        app.stage.h = 18.0;

        let wide = app.content_cols();
        assert_eq!(wide, "a very long first line indeed".chars().count());

        app.scroll = 2; // only the one-character lines are on screen now
        assert_eq!(app.content_cols(), wide, "still the widest line seen");
    }

    #[test]
    fn wrapped_lines_have_no_horizontal_scroll_at_all() {
        let mut app = app_with("a very long line that would otherwise pan");
        app.wrap_lines = true;
        app.stage.w = 45.0;
        app.stage.h = 18.0;
        app.set_hscroll(50);
        assert_eq!(app.hscroll, 0);
    }

    #[test]
    fn undoing_back_to_the_saved_text_clears_the_dirty_flag() {
        let mut app = app_with("hello");
        app.mark_clean();
        assert!(!app.modified);

        app.push_undo();
        app.buffer.insert_char('!');
        app.modified = true;
        assert!(app.modified, "an edit is dirty");

        app.undo();
        assert_eq!(app.buffer.line(0), "hello");
        assert!(!app.modified, "back at the saved text — not dirty any more");
        assert!(
            !app.tabs.buffers[app.current_buffer()].modified,
            "the tab's own flag has to follow, it is what the tab dot reads"
        );
    }

    /// The other direction: redo must put the flag back up. It used to set
    /// `modified = true` unconditionally, which is right by luck here and wrong
    /// as soon as a redo lands back on the saved text.
    #[test]
    fn redo_away_from_the_saved_text_is_dirty_again() {
        let mut app = app_with("hello");
        app.buffer.cursor = Position::new(0, 5);
        app.mark_clean();
        app.push_undo();
        app.buffer.insert_char('!');
        app.modified = true;
        app.undo();
        assert!(!app.modified);

        app.redo();
        assert_eq!(app.buffer.line(0), "hello!");
        assert!(app.modified, "redo moves away from disk again");
    }

    /// A partial undo is still a difference from disk.
    #[test]
    fn undoing_only_part_of_the_way_back_stays_dirty() {
        let mut app = app_with("a");
        app.buffer.cursor = Position::new(0, 1);
        app.mark_clean();
        for c in ['b', 'c'] {
            app.push_undo();
            app.buffer.insert_char(c);
            app.modified = true;
        }
        assert_eq!(app.buffer.line(0), "abc");
        app.undo();
        assert_eq!(app.buffer.line(0), "ab");
        assert!(app.modified, "one step back is still not the saved text");
    }

    /// Saving re-anchors the fingerprint: undoing past a save must go dirty,
    /// not clean, because the file on disk moved.
    #[test]
    fn saving_re_anchors_what_counts_as_clean() {
        let mut app = app_with("a");
        app.buffer.cursor = Position::new(0, 1);
        app.mark_clean();
        app.push_undo();
        app.buffer.insert_char('b');
        app.modified = true;
        app.mark_clean(); // stands in for a successful save of "ab"

        app.undo();
        assert_eq!(app.buffer.line(0), "a");
        assert!(
            app.modified,
            "the original text is no longer what is on disk"
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
        app.search.pattern = Some("hello".into());
        app.collect_matches("hello");
        // smart-case: all lowercase → case-insensitive → 3 matches
        assert_eq!(app.search.matches.len(), 3);
        assert_eq!(app.search.matches[0], Position::new(0, 0));
        assert_eq!(app.search.matches[1], Position::new(1, 0));
        assert_eq!(app.search.matches[2], Position::new(2, 0));
    }

    #[test]
    fn search_case_sensitive_when_pattern_has_upper() {
        let mut app = app_with("hello\nHELLO\nHello");
        app.collect_matches("Hello");
        assert_eq!(app.search.matches.len(), 1);
        assert_eq!(app.search.matches[0], Position::new(2, 0));
    }

    #[test]
    fn search_utf8_char_indices() {
        let mut app = app_with("안녕 hello 안녕");
        app.collect_matches("안녕");
        assert_eq!(app.search.matches.len(), 2);
        assert_eq!(app.search.matches[0].col, 0);
        // "안녕 " = 3 chars, then "hello " = 6, second at col 9
        assert_eq!(app.search.matches[1].col, 9);
    }

    #[test]
    fn enter_search_cancel_restores_cursor() {
        let mut app = app_with("abc\ndef\nghi");
        app.buffer.cursor = Position::new(1, 1);
        app.scroll = 0;
        app.enter_search();
        app.search.input = "ghi".into();
        app.update_search_input();
        assert_eq!(app.buffer.cursor.row, 2);
        app.cancel_search();
        assert_eq!(app.mode, Mode::Editor);
        assert_eq!(app.buffer.cursor, Position::new(1, 1));
        assert!(app.search.input.is_empty());
    }

    #[test]
    fn commit_search_keeps_pattern_for_n() {
        let mut app = app_with("foo bar foo");
        app.enter_search();
        app.search.input = "foo".into();
        app.update_search_input();
        app.commit_search();
        assert_eq!(app.mode, Mode::Editor);
        assert_eq!(app.search.pattern.as_deref(), Some("foo"));
        assert_eq!(app.search.matches.len(), 2);
        let first = app.buffer.cursor;
        app.search_next();
        assert_ne!(app.buffer.cursor, first);
    }

    #[test]
    fn commit_search_keeps_the_live_match_the_user_selected() {
        let mut app = app_with("foo bar foo baz foo");
        app.enter_search();
        app.set_search_input("foo".into());
        assert_eq!(app.buffer.cursor, Position::new(0, 0));

        app.search_cycle(true);
        assert_eq!(app.buffer.cursor, Position::new(0, 8));
        app.commit_search();

        assert_eq!(app.mode, Mode::Editor);
        assert_eq!(app.search.pattern.as_deref(), Some("foo"));
        assert_eq!(app.search.current, 1);
        assert_eq!(app.buffer.cursor, Position::new(0, 8));
    }

    #[test]
    fn native_find_value_accepts_composed_unicode() {
        let mut app = app_with("앞 한글 뒤 한글");
        app.enter_search();
        app.set_search_input("한글".into());

        assert_eq!(app.search.input, "한글");
        assert_eq!(app.search.matches.len(), 2);
        assert_eq!(app.buffer.cursor, Position::new(0, 2));
    }

    #[test]
    fn every_terminal_prefers_the_open_project_root() {
        let mut app = App::new();
        let root = std::env::temp_dir().join("suisei_terminal_project_root");
        let _ = std::fs::create_dir_all(root.join("src"));
        app.explorer.cwd = root.clone();
        app.filename = Some(root.join("src/main.rs"));

        assert_eq!(app.terminal_working_directory(), root);
    }

    #[test]
    fn file_palette_walks_the_project_root_and_keeps_non_source_files() {
        let root = std::env::temp_dir().join(format!(
            "suisei_palette_project_root_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname='fixture'\n").unwrap();
        std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
        std::fs::write(root.join("config/settings.json"), "{}\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let mut app = App::open_file(root.join("src/main.rs").to_str().unwrap());
        // A file-opened app has no explicitly selected explorer root. The
        // palette must still walk up to the manifest instead of stopping in src.
        app.explorer.cwd = "/".into();
        app.open_file_palette();

        let labels: Vec<&str> = app
            .palette
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect();
        assert!(labels.contains(&"Cargo.toml"), "{labels:?}");
        assert!(labels.contains(&"README.md"), "{labels:?}");
        assert!(labels.contains(&"config/settings.json"), "{labels:?}");
        assert!(labels.contains(&"src/main.rs"), "{labels:?}");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workbench_transition_replaces_the_compact_scm_snapshot() {
        use crate::scm::{ScmEntry, ScmStatus};

        let mut app = App::new();
        app.scm.branch = "stale".into();
        app.scm.last_result = Some("No local changes".into());
        app.git_wb.root = Some("/tmp/repo".into());
        app.git_wb.branch = "main".into();
        app.git_wb.ahead = 2;
        app.git_wb.staged = vec![ScmEntry {
            path: "staged.rs".into(),
            status: ScmStatus::Added,
            staged: true,
        }];
        app.git_wb.changes = vec![ScmEntry {
            path: "changed.rs".into(),
            status: ScmStatus::Modified,
            staged: false,
        }];

        app.sync_scm_snapshot_from_git_workbench();

        assert_eq!(
            app.scm.root.as_deref(),
            Some(std::path::Path::new("/tmp/repo"))
        );
        assert_eq!(app.scm.branch, "main");
        assert_eq!(app.scm.ahead, 2);
        assert_eq!(app.scm.total_files(), 2);
        assert!(app.scm.last_result.is_none());
    }

    #[test]
    fn opening_settings_keeps_the_independent_workbench_alive() {
        let mut app = App::new();
        app.git_wb.open = true;
        app.mode = Mode::GitWorkbench;

        // Exercise the actual macOS command route. The legacy dispatcher used
        // to reject Cmd+, while Git Workbench owned the mode, so SwiftUI could
        // present a Settings window backed by a closed/empty Core model.
        app.dispatch(crate::key::KeyEvent::new(
            crate::key::KeyCode::Char(','),
            crate::key::KeyModifiers::SUPER,
        ));

        assert!(app.settings.open);
        assert!(
            app.git_wb.open,
            "opening one native window must not close the other window's model"
        );

        app.open_settings();
        assert!(
            app.settings.open,
            "showing Settings twice must be idempotent"
        );
        assert!(app.git_wb.open);

        app.close_settings();
        assert!(
            app.git_wb.open,
            "closing Settings must leave Source Control open"
        );
    }

    #[test]
    fn search_jumps_to_nearest_from_origin() {
        let mut app = app_with("aa\nbb\naa\ncc\naa");
        app.buffer.cursor = Position::new(1, 0); // on "bb"
        app.enter_search();
        app.search.input = "aa".into();
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
        assert_eq!(app.tabs.buffers.len(), 2);
        app.close_current_tab();
        assert_eq!(app.tabs.buffers.len(), 1);
        assert_eq!(app.filename.as_deref(), Some(f1.as_path()));
        assert_eq!(app.buffer.line(0), "fn a() {}");
        let _ = std::fs::remove_file(&f1);
        let _ = std::fs::remove_file(&f2);
    }

    // ── Brutal layout + terminal integration tests ──────────────────────

    /// Helper: create a temp file with content, return its path.
    fn tmp_file(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("suisei-brutal");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    /// 4분할 + 그룹 상태에서 2번 pane에 ⌃⇧T: 터미널 탭이 2번 자리에
    /// 들어가고, 밀려난 문서는 그룹 밖으로, strip은 연속 유지.
    #[test]
    fn terminal_in_layout_swaps_into_the_group_at_the_focused_pane() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c", "d"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        // Focus pane 1 (document b).
        app.focus_pane_to(1);
        assert_eq!(app.current_buffer_id(), ids[1]);

        // ⌃⇧T — terminal tab replaces b in the group.
        app.toggle_terminal_full();
        let term_id = app.current_buffer_id();
        assert_ne!(term_id, ids[1], "a new terminal tab was created");
        assert!(app.is_terminal_tab(term_id));

        let docs = app.layout_docs(layout_id);
        assert!(docs.contains(&term_id), "terminal joined the group");
        assert!(!docs.contains(&ids[1]), "displaced doc left the group");
        assert_eq!(docs.len(), 4, "group size unchanged");

        // Strip must be contiguous.
        let strip: Vec<BufferId> = app.tabs.buffers.iter().map(|t| t.id).collect();
        let first = strip.iter().position(|id| docs.contains(id)).unwrap();
        let run = &strip[first..first + docs.len()];
        assert!(
            run.iter().all(|id| docs.contains(id)),
            "members contiguous: {strip:?}"
        );
    }

    /// 터미널 탭을 닫으면 그룹에서 빠지고, pane은 adopt 탭으로 repoint.
    #[test]
    fn closing_a_terminal_tab_removes_it_from_the_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        app.focus_pane_to(0);
        app.toggle_terminal_full();
        let term_id = app.current_buffer_id();
        assert!(app.is_terminal_tab(term_id));

        let docs = app.layout_docs(layout_id);
        assert!(docs.contains(&term_id));

        // Close the terminal tab.
        app.close_current_tab();
        assert!(
            !app.tabs.buffers.iter().any(|t| t.id == term_id),
            "terminal tab gone"
        );
        assert!(
            !app.tabs.buffers.iter().any(|t| t.terminal.is_some()),
            "no terminal tab left for a shell to belong to"
        );

        // The group no longer contains the terminal.
        let docs = app.layout_docs(layout_id);
        assert!(!docs.contains(&term_id), "terminal left the group on close");
    }

    /// Mark these documents as terminal tabs.
    ///
    /// No shell is started — none would be here in any case, since the
    /// processes are the face's. What is under test is core's bookkeeping: the
    /// confirm dialog's state machine, the close paths, and the titles.
    fn terminal_tabs_on(app: &mut App, docs: &[BufferId]) -> Vec<crate::split::TerminalId> {
        docs.iter()
            .map(|id| {
                let tid = crate::split::TerminalId(app.next_terminal_id);
                app.next_terminal_id += 1;
                app.tabs
                    .buffers
                    .iter_mut()
                    .find(|t| t.id == *id)
                    .unwrap()
                    .terminal = Some(tid);
                tid
            })
            .collect()
    }

    /// The close-confirm dialog belongs to the shell it was raised for. Pane B
    /// answering pane A's prompt with `y` closes A — not B, not whatever sits
    /// under the caret. The old shared dock flag did exactly that wrong.
    #[test]
    fn close_confirm_belongs_to_the_pane_that_asked() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b"]);
        panes_on(&mut app, &ids);
        let tids = terminal_tabs_on(&mut app, &ids);

        // Ask from pane 0, move focus to pane 1, answer yes.
        app.focus_pane_to(0);
        app.request_close_pane_terminal();
        assert!(app.pane_close_confirm_open());
        app.focus_pane_to(1);
        app.confirm_close_pane_terminal(true);

        // Pane 0's shell tab is gone; pane 1's is untouched.
        assert!(!app.tabs.buffers.iter().any(|t| t.terminal == Some(tids[0])));
        assert!(app.tabs.buffers.iter().any(|t| t.terminal == Some(tids[1])));
        assert!(!app.pane_close_confirm_open(), "latch cleared");
    }

    /// Any close path clears the pending confirm — a shell closed via ⌘W
    /// while its dialog is up must not leave a latch that kills the NEXT
    /// terminal on a stale `y`.
    #[test]
    fn closing_a_terminal_tab_clears_its_close_confirm() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b"]);
        panes_on(&mut app, &ids);
        let tids = terminal_tabs_on(&mut app, &ids[..1]);

        app.focus_pane_to(0);
        app.request_close_pane_terminal();
        assert!(app.pane_close_confirm_open());

        // Close via the ⌘W path, not the dialog.
        let idx = app.buffer_index(ids[0]).unwrap();
        app.close_tab_at(idx);
        assert!(!app.pane_close_confirm_open());
        assert!(!app.tabs.buffers.iter().any(|t| t.terminal == Some(tids[0])));

        // A stray confirm now does nothing.
        app.confirm_close_pane_terminal(true);
        assert!(app.tabs.buffers.iter().any(|t| t.id == ids[1]));
    }

    /// The wrap map is derived from the document, so asking twice must not
    /// build twice — and editing must not hand back the old shape.
    ///
    /// It is a `RefCell` behind a `&self` getter, which is the arrangement that
    /// makes a cache invisible to its callers. This is the test that it stays
    /// one: a map that never rebuilt would be as wrong as one that always did.
    #[test]
    fn the_wrap_map_is_rebuilt_exactly_when_it_stops_describing_the_document() {
        let mut app = app_with("");
        app.buffer = crate::buffer::Buffer::from_string("abcdefghij\nshort");
        app.tab_width = 4;

        // 10 columns: the first line is an exact fit, the second is shorter.
        assert_eq!(app.wrap_map(10, 200).total_rows(), 2);

        // Narrower: the first line now needs two rows.
        assert_eq!(app.wrap_map(5, 200).total_rows(), 2 + 1);

        // Off: one row per line, whatever they measure.
        assert_eq!(app.wrap_map(0, 200).total_rows(), 2);

        // An edit at the same width is a different document.
        app.buffer = crate::buffer::Buffer::from_string(&"x".repeat(25));
        assert_eq!(app.wrap_map(10, 200).total_rows(), 3);

        // And the same question twice is the same answer, from the cache.
        let first = app.wrap_map(10, 200).total_rows();
        assert_eq!(app.wrap_map(10, 200).total_rows(), first);
    }

    /// A pane shell's title now arrives from the face — SwiftTerm reads the
    /// escapes, not us — and lands on the tab it names.
    ///
    /// The return value is the whole reason this is not a plain setter: `zsh`
    /// re-sends its title on every prompt, so a caller that recomposed on each
    /// report would put a full chrome republish behind every command the user
    /// runs. Reporting "unchanged" is what makes that free.
    #[test]
    fn a_reported_terminal_title_lands_on_its_own_tab() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids[..2]);
        let tids = terminal_tabs_on(&mut app, &ids[..2]);

        assert!(app.set_terminal_title(ids[0], Some("vim README.md")));
        assert_eq!(app.terminal_title(ids[0]), Some("vim README.md"));
        // The other shell is untouched: two terminals in one window are two
        // processes, and one naming itself must not name the other.
        assert_eq!(app.terminal_title(ids[1]), None);

        // The same string again is not news.
        assert!(!app.set_terminal_title(ids[0], Some("vim README.md")));

        // A shell that clears its title, or reports only whitespace, goes back
        // to the generic name rather than showing an empty chip.
        assert!(app.set_terminal_title(ids[0], Some("   ")));
        assert_eq!(app.terminal_title(ids[0]), None);

        // A document tab has no shell to name, and saying so must not panic or
        // silently land the title on some other tab.
        assert!(!app.set_terminal_title(ids[2], Some("nope")));
        assert_eq!(app.terminal_title(ids[1]), None);
        let _ = tids;
    }

    /// Split layout tokens round-trip through the session format, nesting
    /// included — the serialization the split's persistence lives and dies by.
    #[test]
    fn layout_tokens_roundtrip() {
        use crate::split::{Axis, Layout, PaneId};
        let tree = Layout::Split {
            axis: Axis::Col,
            children: vec![
                Layout::Split {
                    axis: Axis::Row,
                    children: vec![Layout::Leaf(PaneId(0)), Layout::Leaf(PaneId(1))],
                    weights: vec![0.5, 0.5],
                },
                Layout::Leaf(PaneId(2)),
            ],
            weights: vec![0.6, 0.4],
        };
        let map = |pid: PaneId| Some(pid.0 as usize);
        let tokens = App::layout_tokens(&tree, &map).expect("serialize");
        assert_eq!(tokens, "SC:0.600,0.400:SR:0.500,0.500:T0;T1;T2");
        let parsed = App::parse_layout_tokens(&tokens).expect("parse");
        let again = App::layout_tokens(&parsed, &map).expect("re-serialize");
        assert_eq!(again, tokens, "structural round-trip");
        assert!(
            App::parse_layout_tokens("SC:0.5,0.5:T0").is_none(),
            "weight/child mismatch rejected"
        );
        assert!(
            App::parse_layout_tokens("SX:0.5:T0").is_none(),
            "bad axis rejected"
        );
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lsp_incremental_change_utf16_columns() {
        // 🎉 is one char but TWO UTF-16 units: the line is 10 chars but 11
        // UTF-16 units, so the range end column must be 11, not 10.
        let prev = lines(&["let 🎉 = 1;", ""]);
        let cur = lines(&["let 🎉 = 2;", ""]);
        let changes = lsp_changes_since(&prev, &cur).expect("one change");
        assert_eq!(changes.len(), 1);
        let c = &changes[0];
        assert_eq!((c.start_line, c.start_col), (0, 0));
        assert_eq!((c.end_line, c.end_col), (0, 11), "UTF-16 units, not chars");
        assert_eq!(c.text, "let 🎉 = 2;");
    }

    #[test]
    fn lsp_incremental_change_multiline_and_delete() {
        let prev = lines(&["a", "b", "c", "d"]);
        let cur = lines(&["a", "x", "y", "d"]);
        let c = &lsp_changes_since(&prev, &cur).unwrap()[0];
        // Replace lines 1..3 (b,c): range (1,0)..(2,1) (end = just past `c`),
        // replacement text without a trailing separator.
        assert_eq!((c.start_line, c.start_col), (1, 0));
        assert_eq!((c.end_line, c.end_col), (2, 1));
        assert_eq!(c.text, "x\ny");

        // Pure deletion consumes the trailing separator: (1,0)..(3,0).
        let cur2 = lines(&["a", "d"]);
        let c2 = &lsp_changes_since(&prev, &cur2).unwrap()[0];
        assert_eq!((c2.start_line, c2.start_col), (1, 0));
        assert_eq!((c2.end_line, c2.end_col), (3, 0));
        assert_eq!(c2.text, "");

        assert!(
            lsp_changes_since(&prev, &prev).is_none(),
            "identical → None"
        );
    }

    #[test]
    fn ctrl_d_selects_next_occurrences_in_sel() {
        let mut app = app_with("foo bar foo baz foo");
        app.sel = crate::selection::SelectionSet::single(Selection::caret(Position::new(0, 1)));
        app.multi_cursor_add_next();
        assert_eq!(app.sel.len(), 2, "second occurrence selected");
        app.multi_cursor_add_next();
        assert_eq!(app.sel.len(), 3, "third occurrence");
        for s in app.sel.all() {
            let (s0, e0) = s.range();
            assert_eq!(e0.col - s0.col, 3, "each selection covers `foo`");
        }
        app.multi_cursor_add_next();
        assert_eq!(app.sel.len(), 3, "no fourth match — no phantom cursor");
    }

    #[test]
    fn add_below_stacks_column_carets() {
        let mut app = app_with("aa\nbb\ncc");
        app.sel = crate::selection::SelectionSet::single(Selection::caret(Position::new(0, 1)));
        app.multi_cursor_add_below();
        app.multi_cursor_add_below();
        assert_eq!(app.sel.len(), 3);
        let heads: Vec<Position> = app.sel.all().iter().map(|s| s.head).collect();
        assert_eq!(
            heads,
            vec![
                Position::new(0, 1),
                Position::new(1, 1),
                Position::new(2, 1)
            ]
        );
        app.multi_cursor_add_below();
        assert_eq!(app.sel.len(), 3, "no line below the last — no new caret");
        app.multi_cursor_add_above();
        assert_eq!(app.sel.len(), 3, "no line above the first — no new caret");
    }

    /// A layout that loses its second document dissolves — the lone member
    /// returns to the strip as an ordinary tab. Zombies used to linger:
    /// invisible in grouped style (no run of two to draw a container around),
    /// unkillable in unified style (the chip's close hit the slot-clamped
    /// `close_tab` and killed the wrong document).
    #[test]
    fn a_layout_dissolves_when_it_drops_below_two_documents() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids[..2]);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        // Close one member via the inactive-tab path.
        let idx = app.buffer_index(ids[1]).unwrap();
        app.close_tab_at(idx);
        assert!(
            !app.layouts.iter().any(|l| l.id == layout_id),
            "layout dissolved"
        );
        assert!(
            app.tabs.buffers.iter().any(|t| t.id == ids[0]),
            "lone member survives as a loose tab"
        );
    }

    /// `drop_layout` is "Close Tab" on a layout chip: the entry goes, the
    /// documents stay, and an active arrangement merely loses its tab.
    #[test]
    fn dropping_a_layout_keeps_its_documents() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;
        assert_eq!(app.active_layout, Some(layout_id));

        assert!(app.drop_layout(layout_id));
        assert!(app.layouts.is_empty());
        assert_eq!(app.active_layout, None);
        assert_eq!(app.tabs.buffers.len(), 2, "both documents still open");
        assert!(!app.drop_layout(layout_id), "second drop is a no-op");
    }

    /// Stable-id tab ops resolve through the buffer list, so a strip whose
    /// slots no longer match buffer indices (a folded layout gathers members
    /// into a run; unified style hides them) still names the right tab.
    #[test]
    fn id_addressed_tab_ops_hit_the_named_tab() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);

        app.close_tab_id(ids[1]);
        assert!(!app.tabs.buffers.iter().any(|t| t.id == ids[1]));
        assert!(app.tabs.buffers.iter().any(|t| t.id == ids[0]));
        assert!(app.tabs.buffers.iter().any(|t| t.id == ids[2]));

        // Move c onto a's position.
        assert!(app.move_tab_ids(ids[2], ids[0]));
        let strip: Vec<BufferId> = app.tabs.buffers.iter().map(|t| t.id).collect();
        assert_eq!(strip, vec![ids[2], ids[0]]);

        // goto by id activates the named document.
        app.goto_tab_id(ids[0]);
        assert_eq!(app.current_buffer_id(), ids[0]);
    }

    /// Reported: with a layout grouped or unified, the tab strip's "+" →
    /// "New Untitled Tab" misbehaves.
    ///
    /// It pointed the focused pane at the new buffer, which made the new
    /// document part of the arrangement — and a unified layout draws ONE chip
    /// for the whole arrangement, so the tab just asked for had no chip. The
    /// button appeared to do nothing, and a member had been displaced to make
    /// room for it.
    #[test]
    fn a_new_untitled_tab_leaves_an_active_layout() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        app.open_blank_tab();
        let fresh = app.current_buffer_id();

        assert_eq!(app.active_layout, None, "the desk left the layout");
        assert!(
            !app.layout_holds(layout_id, fresh),
            "the new tab is loose, not a member with no chip"
        );
        assert!(!app.split.is_split(), "the desk cleared down to the new tab");
        assert_eq!(
            app.layout_docs(layout_id),
            vec![ids[0], ids[1]],
            "the parked arrangement is intact and can be clicked back"
        );
    }

    /// Reported: split, group, split AGAIN, focus the new pane, pick a file in
    /// the tree — it is not added and the layout tangles.
    ///
    /// The pane split off after the fold was in no member list, so the swap
    /// looked up a document it could not find and returned having done nothing.
    #[test]
    fn a_pane_split_after_folding_is_a_member_and_can_open_files() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;
        assert_eq!(app.layout_docs(layout_id).len(), 2);

        // Split again while the layout owns the desk.
        app.split_vertical();
        assert_eq!(app.split.panes.len(), 3, "three panes now");

        let path = tmp_file("split_after_fold.txt", "hello");
        app.open_new_tab(path.to_str().unwrap());
        let opened = app.current_buffer_id();

        assert!(
            app.layout_holds(layout_id, opened),
            "the file opened into the new pane joined the arrangement"
        );
        assert!(
            app.layout_docs(layout_id).contains(&opened),
            "and it is in the member list the strip draws from"
        );
    }

    /// Reported: with more than two panes grouped, closing ONE pane dissolves
    /// the group even though two panes remain.
    ///
    /// The rule was `docs.len() >= 2` against a snapshot taken at fold time. A
    /// pane split off afterwards was not in it, so the count was already short
    /// by one and closing that pane took it to one.
    #[test]
    fn closing_one_pane_of_three_keeps_the_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        app.focus_pane_to(2);
        app.close_split();

        assert!(
            app.layouts.iter().any(|l| l.id == layout_id),
            "two panes are still an arrangement"
        );
        assert_eq!(app.layout_docs(layout_id), vec![ids[0], ids[1]]);
    }

    /// The exception the report names: A, A, B — closing B leaves two panes
    /// showing ONE document, which is not an arrangement, so the group should
    /// dissolve.
    #[test]
    fn closing_the_only_second_document_dissolves_the_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b"]);
        // Three panes: a, a, b.
        panes_on(&mut app, &[ids[0], ids[0], ids[1]]);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;
        assert_eq!(app.layout_docs(layout_id), vec![ids[0], ids[1]]);

        app.focus_pane_to(2);
        app.close_split();

        assert!(
            !app.layouts.iter().any(|l| l.id == layout_id),
            "a, a is one document, which is not an arrangement"
        );
    }

    #[test]
    fn a_document_no_pane_shows_is_not_a_member() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;
        assert_eq!(app.layout_docs(layout_id), vec![ids[0], ids[1], ids[2]]);

        // Pane 0 shows a. Open b into it — b is already on pane 1, so the
        // arrangement becomes b, b, c and NOTHING shows a any more.
        app.focus_pane_to(0);
        app.open_new_tab("/tmp/b.txt");

        // This assertion used to read the other way: membership was "unchanged
        // when opening a member", because the hand-written member list bailed
        // out early on a document it already contained and left `a` in it. A
        // member no pane shows is precisely the phantom that made opening a
        // file into a later-split pane do nothing at all — the arrangement is
        // the panes, and the panes no longer include `a`.
        assert_eq!(
            app.layout_docs(layout_id),
            vec![ids[1], ids[2]],
            "the displaced document left the group with its pane"
        );
        assert!(
            app.tabs.buffers.iter().any(|t| t.id == ids[0]),
            "it is still an open tab, just a loose one"
        );
    }

    /// 터미널 탭이 있는 상태에서 fold: 터미널도 그룹 멤버로 참여.
    #[test]
    fn folding_with_a_terminal_tab_includes_it_in_the_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b"]);
        panes_on(&mut app, &ids);

        // Turn pane 0 into a terminal first.
        app.focus_pane_to(0);
        app.toggle_terminal_full();
        let term_id = app.current_buffer_id();
        assert!(app.is_terminal_tab(term_id));

        // Now fold — both panes (terminal + b) go into the group.
        assert!(app.fold_layout());
        let docs = app.layout_docs(app.layouts[0].id);
        assert!(docs.contains(&term_id), "terminal tab is a group member");
        assert!(docs.contains(&ids[1]), "document b is a group member");
        assert_eq!(docs.len(), 2);
    }

    /// 두 그룹이 있을 때 드래그가 둘 다 안 깨뜨림.
    #[test]
    fn reorder_guard_protects_both_groups_simultaneously() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c", "d", "e", "f"]);
        // Group 1: a, b. Group 2: e, f. c, d loose.
        panes_on(&mut app, &[ids[0], ids[1]]);
        assert!(app.fold_layout());
        let g1 = app.layouts[0].id;

        // Switch away, set up group 2.
        app.goto_tab(4);
        panes_on(&mut app, &[ids[4], ids[5]]);
        assert!(app.fold_layout());
        // The new layout is the last one pushed; the first is still parked.
        let g2 = app.layouts.last().unwrap().id;
        assert_ne!(g1, g2);

        // Strip: [a,b](g1) c d [e,f](g2)
        // Dragging c between a and b breaks g1.
        assert!(!app.move_tab(2, 1), "cannot split group 1");
        // Dragging d between e and f breaks g2.
        assert!(!app.move_tab(3, 4), "cannot split group 2");
        // Dragging c to the very front is fine — groups stay contiguous.
        assert!(app.move_tab(2, 0), "moving outside both groups is allowed");
    }

    /// 그룹 멤버 탭을 close_tab_at으로 닫으면 그룹에서 빠지고, 그 탭을
    /// 보여 주던 pane도 사라진다 (repoint 유령 pane이 남지 않는다).
    #[test]
    fn closing_a_group_member_by_index_removes_it_from_the_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;
        assert_eq!(app.split.pane_count(), 3);

        // Close "b" while still *in* the layout. `goto_tab` would leave the
        // layout and collapse the desk — that is a different path.
        assert_eq!(app.current_buffer_id(), ids[0], "focused on a");
        app.close_tab_at(1);
        assert!(
            !app.tabs.buffers.iter().any(|t| t.id == ids[1]),
            "b is gone"
        );
        assert_eq!(app.split.pane_count(), 2, "b's pane left the arrangement");
        assert!(
            !app.split.panes.iter().any(|p| p.buffer == ids[1]),
            "no pane still names b"
        );

        let docs = app.layout_docs(layout_id);
        assert!(!docs.contains(&ids[1]), "b left the group");
        assert_eq!(docs.len(), 2, "a and c remain");
    }

    /// 에디터 헤더 × 로 pane을 닫으면 탭은 남지만 레이아웃 그룹에서는 나간다.
    #[test]
    fn closing_a_pane_ejects_its_document_from_the_layout_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        // Focus the middle pane (b) and close it via the header path.
        app.focus_pane_to(1);
        assert_eq!(app.current_buffer_id(), ids[1]);
        app.close_split();

        assert!(
            app.tabs.buffers.iter().any(|t| t.id == ids[1]),
            "tab b stays open — header × is not a tab close"
        );
        assert_eq!(app.split.pane_count(), 2, "one pane closed");
        assert!(
            !app.split.panes.iter().any(|p| p.buffer == ids[1]),
            "no pane shows b"
        );

        let docs = app.layout_docs(layout_id);
        assert!(!docs.contains(&ids[1]), "b left the layout group");
        assert_eq!(docs.len(), 2, "a and c remain in the group");
        // The strip still has b as a loose tab outside the group.
        assert_eq!(app.layout_holding(ids[1]).map(|l| l.id), None);
    }

    /// 2-pane 그룹에서 헤더로 하나를 닫으면 그룹은 해체된다 (멤버 < 2).
    #[test]
    fn closing_a_pane_of_a_two_member_layout_dissolves_the_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        app.focus_pane_to(1);
        app.close_split();
        assert!(
            !app.layouts.iter().any(|l| l.id == layout_id),
            "layout dissolved"
        );
        assert_eq!(app.active_layout, None);
        assert_eq!(app.split.pane_count(), 1);
        assert_eq!(app.tabs.buffers.len(), 2, "both tabs still open");
    }

    /// User scenario: 2-tab split folded into a group; close one pane via header.
    /// Group must dissolve; both tabs remain open; only one pane left.
    #[test]
    fn user_scenario_two_tab_group_header_close_dissolves() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["A", "B"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        assert_eq!(app.split.pane_count(), 2);
        assert_eq!(app.layouts.len(), 1);

        app.focus_pane_to(0); // left = A
        app.close_split();

        assert_eq!(app.layouts.len(), 0, "group must dissolve");
        assert_eq!(app.active_layout, None);
        assert_eq!(app.split.pane_count(), 1, "one pane left");
        assert_eq!(
            app.tabs.buffers.len(),
            2,
            "both tabs stay (header is not tab close)"
        );
        assert!(
            !app.split.panes.iter().any(|p| p.buffer == ids[0])
                || app.current_buffer_id() == ids[1]
                || app.split.panes[0].buffer == ids[1],
            "survivor should be B; panes={:?}",
            app.split.panes.iter().map(|p| p.buffer).collect::<Vec<_>>()
        );
        assert_eq!(app.split.panes[0].buffer, ids[1], "remaining pane shows B");
    }

    /// User scenario: group A|B; close A from the tab bar → A's pane gone, not repointed to B|B.
    #[test]
    fn user_scenario_two_tab_group_tabbar_close_removes_pane() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["A", "B"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());
        assert_eq!(app.split.pane_count(), 2);

        // Close A while focused on A (current-tab path).
        app.focus_pane_to(0);
        assert_eq!(app.current_buffer_id(), ids[0]);
        app.close_tab_id(ids[0]);

        assert!(
            !app.tabs.buffers.iter().any(|t| t.id == ids[0]),
            "A tab gone"
        );
        assert_eq!(app.tabs.buffers.len(), 1);
        assert_eq!(
            app.split.pane_count(),
            1,
            "A's pane must be removed, not kept as B|B"
        );
        assert_eq!(app.split.panes[0].buffer, ids[1], "only B remains");
        assert_eq!(app.layouts.len(), 0, "group dissolved");
    }

    /// Same tab-bar close but A is not focused (inactive-tab path).
    #[test]
    fn user_scenario_two_tab_group_tabbar_close_inactive_removes_pane() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["A", "B"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());

        app.focus_pane_to(1); // focus B
        assert_eq!(app.current_buffer_id(), ids[1]);
        app.close_tab_id(ids[0]); // close A

        assert!(!app.tabs.buffers.iter().any(|t| t.id == ids[0]));
        assert_eq!(app.split.pane_count(), 1, "A's pane removed; not B|B");
        assert_eq!(app.split.panes[0].buffer, ids[1]);
        assert_eq!(app.layouts.len(), 0);
    }

    /// 레이아웃 활성화 시 터미널 탭을 focus_doc으로 지정하면 그 pane에 포커스.
    #[test]
    fn activating_a_layout_can_focus_a_terminal_tab() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "outside"]);
        panes_on(&mut app, &ids[..2]);
        app.focus_pane_to(1);
        app.toggle_terminal_full();
        let term_id = app.current_buffer_id();
        assert!(app.is_terminal_tab(term_id));

        // Fold so the arrangement is parked as a layout.
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        // Leave the layout.
        app.goto_tab_id(ids[2]);
        assert_eq!(app.active_layout, None);

        // Come back asking for the terminal tab.
        assert!(app.activate_layout(layout_id, Some(term_id)));
        assert_eq!(app.current_buffer_id(), term_id, "terminal pane is focused");
    }

    /// 터미널 탭이 그룹에 있을 때 move_tab으로 그룹 내부 재정렬은 허용.
    #[test]
    fn terminal_tab_can_reorder_within_its_group() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c"]);
        panes_on(&mut app, &ids);
        assert!(app.fold_layout());

        // Turn pane 0 into a terminal.
        app.focus_pane_to(0);
        app.toggle_terminal_full();
        let term_id = app.current_buffer_id();

        // After swap + gather: [a, b, c, term] with group run [b, c, term].
        // Swap the terminal with its adjacent group member (not a non-member).
        let term_slot = app
            .tabs
            .buffers
            .iter()
            .position(|t| t.id == term_id)
            .unwrap();
        let docs = app.layout_docs(app.layouts[0].id);
        let neighbor = if term_slot > 0 && docs.contains(&app.tabs.buffers[term_slot - 1].id) {
            term_slot - 1
        } else {
            term_slot + 1
        };
        assert!(
            docs.contains(&app.tabs.buffers[neighbor].id),
            "neighbor is a group member"
        );
        assert!(
            app.move_tab(term_slot, neighbor),
            "reorder within group is allowed"
        );
    }

    /// 5개 탭, 3개만 pane에 (1,3,5), fold → gather 후 strip 연속,
    /// 그 상태에서 3번 pane에 새 파일 열기 → swap + re-gather.
    #[test]
    fn open_into_a_sparse_pane_layout_swaps_and_regathers() {
        let mut app = app_with("a");
        let ids = tabs_named(&mut app, &["a", "b", "c", "d", "e"]);
        // Panes on a, c, e (slots 0, 2, 4).
        panes_on(&mut app, &[ids[0], ids[2], ids[4]]);
        assert!(app.fold_layout());
        let layout_id = app.layouts[0].id;

        // After gather: [a, c, e, b, d] — members contiguous.
        let docs = app.layout_docs(layout_id);
        assert_eq!(docs.len(), 3);

        // Focus pane 1 (document c) and open a new file.
        app.focus_pane_to(1);
        assert_eq!(app.current_buffer_id(), ids[2], "pane 1 shows c");
        let new_path = tmp_file("sparse_new.txt", "new");
        app.open_new_tab(new_path.to_str().unwrap());
        let new_id = app.current_buffer_id();
        assert_ne!(new_id, ids[2]);

        let docs = app.layout_docs(layout_id);
        assert!(docs.contains(&new_id), "new file in group");
        assert!(!docs.contains(&ids[2]), "c left the group");
        assert_eq!(docs.len(), 3);

        // Strip still contiguous.
        let strip: Vec<BufferId> = app.tabs.buffers.iter().map(|t| t.id).collect();
        let first = strip.iter().position(|id| docs.contains(id)).unwrap();
        let run = &strip[first..first + docs.len()];
        assert!(
            run.iter().all(|id| docs.contains(id)),
            "contiguous: {strip:?}"
        );
        let _ = std::fs::remove_file(&new_path);
    }
}

#[cfg(test)]
mod binary_guard_tests {
    use super::*;

    fn tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "suisei-bin-guard-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&p, bytes).expect("temp write");
        p
    }

    #[test]
    fn a_png_opens_as_unreadable_rather_than_as_an_empty_document() {
        // Eight bytes of real PNG signature — the second byte alone is enough
        // to fail UTF-8, and the NUL comes later in any real file.
        let png = tmp("x.png", b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR");
        let app = App::open_file(png.to_str().unwrap());
        assert_eq!(
            app.live_tab_kind(),
            crate::media::FileKind::Image,
            "a .png is an image, not a wall of mojibake"
        );
        assert!(
            app.message.contains("Image"),
            "expected the notice to name what it is, got {:?}",
            app.message
        );
        assert!(app.buffer.text().is_empty(), "nothing to edit, so nothing shown");
        let _ = std::fs::remove_file(png);
    }

    /// The path with no extension to go on. This is the Xcode-tile case: a
    /// compiled binary, where the bytes are the only evidence.
    #[test]
    fn an_extensionless_binary_is_caught_by_its_bytes() {
        let exe = tmp("a.out", b"\xcf\xfa\xed\xfe\x0c\x00\x00\x01\x00\x00\x00\x00");
        let app = App::open_file(exe.to_str().unwrap());
        assert_eq!(app.live_tab_kind(), crate::media::FileKind::Binary);
        let _ = std::fs::remove_file(exe);
    }

    /// The opposite direction, and the one that would be felt every day: an
    /// ordinary source file must stay ordinary. A false Binary here hides a
    /// file behind a tile the user cannot type into.
    #[test]
    fn source_files_stay_text() {
        for (name, body) in [
            ("t.rs", "fn main() { println!(\"안녕 🎧\"); }\n".as_bytes()),
            ("t.md", "# 제목\n\n본문\n".as_bytes()),
            ("Makefile", b"all:\n\techo hi\n".as_slice()),
        ] {
            let p = tmp(name, body);
            let app = App::open_file(p.to_str().unwrap());
            assert_eq!(
                app.live_tab_kind(),
                crate::media::FileKind::Text,
                "{name} should be editable text"
            );
            assert!(!app.buffer.text().is_empty(), "{name} lost its contents");
            let _ = std::fs::remove_file(p);
        }
    }

    /// ⌘S went dead on some files and not others, with no visible pattern.
    ///
    /// The pattern was the 8 KiB read window. `file_looks_binary` handed
    /// `looks_binary` exactly 8192 bytes of a larger file; `looks_binary`
    /// decides whether it may judge UTF-8 by asking `len <= 8192`, read that
    /// as "this is the whole file", and validated a slice cut through the
    /// middle of a character. Any file over 8 KiB with a Hangul syllable,
    /// emoji or curly quote straddling that boundary was declared binary and
    /// refused. Which byte a character lands on is invisible, so the failure
    /// looked random.
    ///
    /// The sweep is the test: three of these offsets used to fail.
    #[test]
    fn a_multibyte_character_on_the_8kib_boundary_is_still_text() {
        for pad in 8186..8195 {
            let mut body: String = std::iter::repeat('a').take(pad).collect();
            body.push('가');
            body.push_str(&"b".repeat(500));
            let p = tmp(&format!("k{pad}.rs"), body.as_bytes());
            assert!(
                !file_looks_binary(&p),
                "a {} byte source file was called binary because a character \
                 crosses byte 8192 (pad {pad})",
                body.len()
            );
            let mut app = App::open_file(p.to_str().unwrap());
            app.save_file();
            assert!(
                app.message.starts_with('✓'),
                "⌘S refused an ordinary source file: {:?}",
                app.message
            );
            let _ = std::fs::remove_file(p);
        }
    }

    /// Edit A, click B's tab, and B's gutter showed A's bars.
    ///
    /// `self.git` describes one file, and nothing made it follow the document.
    /// `goto_tab` — the tab-chip click — was one of the eight restore paths
    /// that never called `refresh_git`, so the old hunks simply stayed, drawn
    /// against whatever rows the new document happened to have.
    ///
    /// Asserted on `signs` as well as `hunks`: the renderer reads the sign map,
    /// so clearing only the hunks would leave the bars on screen.
    #[test]
    fn switching_tabs_takes_the_previous_file_s_hunks_with_it() {
        let a = tmp("gutter_a.rs", b"fn a() {}\n");
        let b = tmp("gutter_b.rs", b"fn b() {}\n");
        let mut app = App::open_file(a.to_str().unwrap());
        app.open_new_tab(b.to_str().unwrap());

        // Stand in for "the user edited B and it grew a hunk". Building this
        // by hand rather than through git keeps the test off the filesystem's
        // repository state, which is not what is under test.
        app.git.path = b.display().to_string();
        app.git.available = true;
        app.git.hunks = vec![crate::git::GitHunk {
            start: 0,
            len: 1,
            removed: Vec::new(),
            kind: crate::git::GitSign::Modified,
            patch: String::new(),
            staged: false,
        }];
        app.git
            .signs
            .insert(0, crate::git::GitSign::Modified);

        app.goto_tab(0);
        assert_eq!(
            app.filename.as_deref(),
            Some(a.as_path()),
            "the switch itself did not happen"
        );
        assert!(
            app.git.hunks.is_empty() && app.git.signs.is_empty(),
            "the other file's change is still in the gutter: {:?}",
            app.git.hunks
        );
        for p in [a, b] {
            let _ = std::fs::remove_file(p);
        }
    }

    /// The guard still has to work — the fix widens the read window, it does
    /// not weaken the test. A binary whose NUL sits past 8 KiB is still caught
    /// by the head, and one with a NUL early is caught immediately.
    #[test]
    fn the_widened_window_still_catches_binaries() {
        let mut blob = vec![b'a'; 9000];
        blob[4000] = 0;
        let p = tmp("big.bin", &blob);
        assert!(file_looks_binary(&p));
        let _ = std::fs::remove_file(p);
    }

    /// The explorer's open path — `App::open_file` is only the launch
    /// constructor, and this is the one a user actually reaches. It had the
    /// same `read_to_string().unwrap_or_default()` and the same ⌘S after it.
    #[test]
    fn opening_a_png_in_a_tab_does_not_offer_to_destroy_it() {
        let png = tmp("z.png", b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x01\x02\x03");
        let before = std::fs::read(&png).expect("read back");
        let mut app = App::new();
        app.open_new_tab(png.to_str().unwrap());
        assert_eq!(app.live_tab_kind(), crate::media::FileKind::Image);
        app.save_file();
        assert_eq!(
            std::fs::read(&png).expect("read back"),
            before,
            "⌘S on an image pane wrote over the image"
        );
        let _ = std::fs::remove_file(png);
    }

    #[test]
    fn saving_over_a_binary_file_is_refused() {
        let png = tmp("y.png", b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR");
        let before = std::fs::read(&png).expect("read back");
        let mut app = App::open_file(png.to_str().unwrap());
        app.save_file();
        let after = std::fs::read(&png).expect("read back");
        assert_eq!(before, after, "save truncated a binary file");
        assert!(app.message.contains("Refusing to save"), "{:?}", app.message);
        let _ = std::fs::remove_file(png);
    }

    #[test]
    fn ordinary_text_is_unaffected() {
        let rs = tmp("z.rs", b"fn main() {}\n");
        let mut app = App::open_file(rs.to_str().unwrap());
        assert!(app.message.contains("Opened"), "{:?}", app.message);
        app.buffer = crate::buffer::Buffer::from_string("fn main() { }\n");
        app.save_file();
        assert_eq!(
            std::fs::read_to_string(&rs).unwrap_or_default(),
            "fn main() { }\n"
        );
        let _ = std::fs::remove_file(rs);
    }

    #[test]
    fn utf8_with_multibyte_characters_is_text() {
        assert!(!looks_binary("한글과 émoji 🎧\n".as_bytes()));
        assert!(looks_binary(b"\x00\x01\x02"));
        assert!(!file_looks_binary(std::path::Path::new("/no/such/file")));
    }
}
