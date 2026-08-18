use suisei_core::app::{App, Mode};
use suisei_core::buffer::Position;

/// Face-independent viewport metrics (from Swift resize).
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub css_w: f32,
    pub css_h: f32,
    /// Line height in points (row pitch).
    pub cell_px: f32,
    /// Glyph cell width in points (column pitch).
    pub cell_w: f32,
    pub dpr: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            css_w: 1200.0,
            css_h: 800.0,
            cell_px: 18.0,
            cell_w: 9.0,
            dpr: 2.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ShellState {
    pub viewport: Viewport,
    pub dirty: bool,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            viewport: Viewport::default(),
            dirty: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TabScene {
    /// `BufferTab::id` — stable across reorders, unlike the slot index.
    pub id: u64,
    pub title: String,
    pub dirty: bool,
    pub active: bool,
    /// The layout this chip belongs to, or 0.
    ///
    /// A folded layout wears one of two shapes (`LayoutStyle`): **grouped**,
    /// where its documents keep their own chips and the face draws one rounded
    /// container around the run, or **unified**, where it is a single chip
    /// carrying the layout's name. Both are expressed here — grouped emits a
    /// chip per document sharing this id, unified emits one chip whose own id
    /// is the layout's.
    pub group: u64,
    /// This chip *is* a layout (unified style), not a document.
    pub is_layout: bool,
    /// This tab is a terminal — a shell runs in it.
    pub is_terminal: bool,
    /// The document's file was deleted on disk out from under the open buffer
    /// — the face marks the chip so editing a vanished file is not silent.
    pub deleted: bool,
}

/// Highlight span in visual-column space (tab-expanded coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanScene {
    pub start: u32,
    pub end: u32,
    /// Maps to `suisei_core::highlight::TokenKind` via `kind_to_u8`.
    pub kind: u8,
}

#[derive(Debug, Clone)]
pub struct EditorLineScene {
    pub line_no: u32,
    /// Debugger marks for this row — see [`DEBUG_STOPPED`].
    pub debug_sign: u8,
    pub text: String,
    pub is_cursor: bool,
    pub caret_vcol: u32,
    /// Caret as a UTF-16 offset into `text`. `caret_vcol` is a TERMINAL cell
    /// column (CJK counts as 2), which does not match how a GUI renderer lays
    /// glyphs out — placing the caret at `vcol * cell_width` drifted right of
    /// the real text on Hangul/Japanese lines. The face resolves this offset
    /// against the drawn line instead.
    pub caret_utf16: u32,
    /// Visual selection on this line: inclusive start, exclusive end. None = no sel.
    pub sel_v0: Option<u32>,
    pub sel_v1: Option<u32>,
    /// Same range as UTF-16 offsets into `text` — see `caret_utf16` for why the
    /// cell grid cannot be used for GUI layout.
    pub sel_u0: u32,
    pub sel_u1: u32,
    /// Syntax highlight spans (visual cols on expanded text).
    pub spans: Vec<SpanScene>,
    /// Git gutter: 0 none, 1 added, 2 modified, 3 deleted.
    /// Bit 0x80 set ⇒ soft-wrap continuation (gutter blank / ↪).
    pub git_sign: u8,
    /// Absolute visual row (0-based) for native paint Y when soft-wrap expands lines.
    pub visual_row: u32,
}

#[derive(Debug, Clone)]
pub struct PaneScene {
    pub tab_index: u32,
    /// Display title of the pane's actual buffer. This intentionally does not
    /// come from the visible tab-strip slot: unified layout chips collapse
    /// several buffer tabs into one strip item.
    pub title: String,
    /// Total lines in this pane's buffer (for face scrollbar / clamp).
    pub doc_line_count: u32,
    /// Horizontal pan for this pane (wrap off).
    pub hscroll: u32,
    pub scroll: u32,
    pub focused: bool,
    pub lines: Vec<EditorLineScene>,
    /// Buffer version this snapshot was built from (patch-path reuse key).
    pub doc_version: u64,
    /// Row budget the snapshot was built with (patch-path reuse key).
    pub band_rows: u32,
    /// What the face should put in this pane — a shell, a text editor, or one
    /// of the viewers. This used to be a `is_terminal: bool`, which is the same
    /// routing question with two of its answers missing.
    pub kind: suisei_core::media::FileKind,
    /// Normalised rect within the editor area, straight from the layout tree.
    /// The face places panes by these instead of re-deriving geometry from a
    /// kind and a ratio, which it could only ever get right for two panes.
    pub rect: suisei_core::split::Rect,
}

impl PaneScene {
    pub fn is_terminal(&self) -> bool {
        self.kind == suisei_core::media::FileKind::Terminal
    }
}

#[derive(Debug, Clone)]
pub struct ExplorerEntryScene {
    pub name: String,
    pub is_dir: bool,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct ExplorerScene {
    pub open: bool,
    pub cwd: String,
    pub entries: Vec<ExplorerEntryScene>,
}

/// Document outline row (Inspector / jump-bar symbols).
#[derive(Debug, Clone)]
pub struct OutlineItemScene {
    pub name: String,
    /// 1-based line
    pub row: u32,
    /// 0=header, 1=fn/method, 2=type, 3=other
    pub kind: u8,
    pub depth: u8,
}

#[derive(Debug, Clone)]
pub struct PaletteItemScene {
    pub label: String,
    pub detail: String,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct PaletteScene {
    pub open: bool,
    pub kind: String,
    pub query: String,
    pub items: Vec<PaletteItemScene>,
}

#[derive(Debug, Clone)]
pub struct SearchScene {
    pub open: bool,
    pub forward: bool,
    pub input: String,
    pub match_count: u32,
    pub match_index: u32,
    /// The find bar is showing its replace field, and what is in it.
    pub replace_open: bool,
    pub replace_input: String,
}

#[derive(Debug, Clone)]
pub struct CompletionsScene {
    pub open: bool,
    pub prefix: String,
    pub selected: u32,
    pub items: Vec<(String, String)>, // label, detail
}

#[derive(Debug, Clone)]
pub struct TerminalScene {
    /// Whether the docked strip (⌃T) is showing. That is the whole of what
    /// core knows about it now: the shells inside are SwiftTerm's, and their
    /// rows never cross the ABI. `full_panel`, `pane_bound` and 200 rows of
    /// truecolor SGR used to live here.
    pub open: bool,
}

/// Packed RGB for Swift face (0x00RRGGBB).
#[derive(Debug, Clone)]
pub struct ThemeScene {
    pub name: String,
    pub editor_bg: u32,
    pub fg: u32,
    pub dim: u32,
    pub current_line: u32,
    pub invisibles: u32,
    pub accent: u32,
    pub selection: u32,
    pub caret: u32,
    pub status_bg: u32,
    pub keyword: u32,
    pub string: u32,
    pub comment: u32,
    pub number: u32,
    pub type_name: u32,
    pub function: u32,
    pub macro_name: u32,
    pub namespace: u32,
    pub parameter: u32,
    pub property: u32,
    pub constant: u32,
    pub operator: u32,
    pub punctuation: u32,
    pub window_bg: u32,
    pub border: u32,
    pub panel_bg: u32,
    pub panel_border: u32,
    pub panel_sel_bg: u32,
    pub panel_sel_fg: u32,
    pub explorer_bg: u32,
    pub explorer_fg: u32,
    pub explorer_selected: u32,
    pub status_fg: u32,
    pub muted: u32,
    pub success: u32,
    pub warning: u32,
    pub error: u32,
    pub accent_fg: u32,
    pub search_bg: u32,
    pub completion_bg: u32,
    pub completion_selected: u32,
    pub completion_border: u32,
    pub terminal_bg: u32,
    pub git_add_bg: u32,
    pub git_del_bg: u32,
    pub git_hunk: u32,
    /// The 3D workbench's stage. Appended, like every token after it — the
    /// face reads this struct by field, and the ABI order is the contract.
    pub model_bg: u32,
    pub debug_stop: u32,
}

#[derive(Debug, Clone)]
pub struct SettingsRowScene {
    pub label: String,
    pub value: String,
    pub is_header: bool,
    pub selected: bool,
    /// What this row IS — `SettingRow::kind`. 0 for rows that exist only on the
    /// About page and have no setting behind them.
    ///
    /// Carried so the face can branch on the row's identity instead of
    /// pattern-matching its label. See `SettingRow::kind`.
    pub kind: u32,
    /// Which theme / which language, for the rows that are indexed.
    pub payload: u32,
    /// Native Settings destination and control semantics from Core.
    pub page: u32,
    pub control: u32,
    pub value_index: u32,
    pub advanced: bool,
    pub group: String,
    pub detail: String,
    pub options: String,
}

#[derive(Debug, Clone)]
pub struct SettingsScene {
    pub open: bool,
    pub dirty: bool,
    pub page_index: u32,
    pub selected: u32,
    pub status: String,
    pub tabs: Vec<String>,
    pub rows: Vec<SettingsRowScene>,
}

#[derive(Debug, Clone)]
pub struct ScmEntryScene {
    pub path: String,
    pub mark: String,
    pub staged: bool,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct ScmGraphRowScene {
    pub strip: String,
    pub short: String,
    pub subject: String,
    pub when: String,
    /// `HEAD -> main, origin/main`, when the commit carries any.
    pub refs: String,
    /// Lane colour index from the graph walker, so a branch keeps one hue.
    pub color: u8,
    /// On HEAD, not yet on its upstream.
    pub unpushed: bool,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct ScmScene {
    pub open: bool,
    pub branch: String,
    pub status: String,
    pub staged: Vec<ScmEntryScene>,
    pub changes: Vec<ScmEntryScene>,
    pub graph: Vec<ScmGraphRowScene>,
}

#[derive(Debug, Clone)]
pub struct GitWbChip {
    pub label: String,
    pub active: bool,
    /// 1..=9 toolbar key
    pub key: u8,
}

#[derive(Debug, Clone)]
pub struct GitWbScene {
    pub open: bool,
    /// true = JetBrains 3-col dock (Status/Log/Files), false = full-width special tab
    pub docked: bool,
    pub tab_index: u32,
    pub branch: String,
    pub message: String,
    pub chips: Vec<GitWbChip>,
    /// Docked columns (or single special column when !docked)
    pub col_changes: Vec<String>,
    pub col_log: Vec<String>,
    pub col_files: Vec<String>,
    pub special: Vec<String>,
    pub loading: bool,
}

#[derive(Debug, Clone)]
pub struct ChromeScene {
    pub mode_label: String,
    pub message: String,
    pub filename: String,
    pub breadcrumbs: String,
    pub dirty_buffer: bool,
    pub welcome: bool,
    pub explorer_open: bool,
    pub cursor_row: u32,
    pub cursor_col: u32,
    pub caret_vcol: u32,
    /// Why Core last moved the scroll (see `ScrollIntent`).
    pub scroll_intent: u8,
    pub line_count: u32,
    pub scroll: u32,
    pub pct: u32,
    /// Sub-line scroll residual (−1, 1) for smooth face paint.
    pub scroll_frac: f32,
    /// Horizontal pan in visual columns when wrap_lines is false.
    pub hscroll: u32,
    /// Soft-wrap flag (face disables trackpad H-scroll when true).
    pub wrap_lines: u8,
    /// Gutter shows distance from the caret rather than absolute numbers.
    ///
    /// `Config::relative_number` has existed on both sides of this boundary
    /// for as long as the setting has, and never crossed it: the switch wrote
    /// the config file and the gutter went on drawing absolute numbers,
    /// because nothing ever told the face.
    pub relative_number: u8,
    pub buffer_version: u64,
    pub branch: String,
    pub tabs: Vec<TabScene>,
    /// Focused-pane (or single) lines for backwards-compatible consumers.
    pub lines: Vec<EditorLineScene>,
    /// What the single pane shows when unsplit. The FFI synthesises pane 0
    /// on that path instead of walking the pane array, so it needs telling.
    pub pane0_kind: suisei_core::media::FileKind,
    /// Actual active buffer title, independent from a unified layout chip.
    pub pane0_title: String,
    pub pane_focus: u8,
    /// Empty when unsplit; when split, one entry per pane (lines packed for FFI).
    pub panes: Vec<PaneScene>,
    pub explorer: ExplorerScene,
    pub palette: PaletteScene,
    pub search: SearchScene,
    pub completions: CompletionsScene,
    pub terminal: TerminalScene,
    pub settings: SettingsScene,
    pub theme: ThemeScene,
    pub scm: ScmScene,
    pub git_wb: GitWbScene,
    /// Document structure for Xcode-like Inspector / jump bar.
    pub outline: Vec<OutlineItemScene>,
    /// Pretty preview (Markdown / JSON / …) when Mode::Preview.
    pub preview: PreviewScene,
}

#[derive(Debug, Clone, Default)]
pub struct PreviewScene {
    pub open: bool,
    pub kind: u8, // 0 none, 1 md, 2 json, 3 plain, 4 image, 5 csv, 6 npy, 7 audio
    pub scroll: u32,
    pub hscroll: u32,
    pub lines: Vec<PreviewLineScene>,
}

#[derive(Debug, Clone)]
pub struct PreviewLineScene {
    pub text: String,
    /// Dominant PreviewStyle as small enum code (see face).
    pub style: u8,
}

#[derive(Debug, Clone)]
pub struct FrameDiff {
    pub frame_gen: u64,
    pub chrome: Option<ChromeScene>,
}

impl FrameDiff {
    pub fn empty(frame_gen: u64) -> Self {
        Self {
            frame_gen,
            chrome: None,
        }
    }
}

/// Public so Engine can cache outline across scroll-only recomposes.
pub fn build_outline_public(app: &App) -> Vec<OutlineItemScene> {
    build_outline(app)
}

/// Exact-range band pull for the face renderer: rows `[start, start+rows)` of
/// the buffer shown in `pane` (0 = focused/single). The face draws by pulling —
/// no push/merge — so this is the only paint data path that matters.
pub fn build_editor_band(
    app: &App,
    pane: usize,
    start: usize,
    rows: usize,
    wrap_cols: u16,
    wide_ratio: u16,
) -> (Vec<EditorLineScene>, u32) {
    // A pane the desk does not have is not a request to be satisfied with some
    // other pane's document.
    //
    // Both arms below used to answer anyway: unsplit returned `current` for
    // EVERY index, and the split arm clamped with `pane.min(n - 1)`. That is
    // how "switch from a split to an ordinary tab" flashed the destination in
    // BOTH panes. Leaving a layout collapses the split in one engine call, but
    // the face animates the pane list over 0.22s, so the departing pane view is
    // still alive and still pulling — and it was told the new document, because
    // by then `is_split()` was false and every index answered `current`.
    //
    // Declining costs the caller nothing: `pull_band` treats an empty band as
    // "no rows here", which is the truth for a pane that no longer exists.
    let live_panes = if app.split.is_split() {
        app.split.pane_count()
    } else {
        1
    };
    if pane >= live_panes {
        return (Vec::new(), 0);
    }

    // Resolve pane → tab + focus (mirrors build_editor_surfaces).
    let current = app.current_buffer();
    let (tab, focused) = if !app.split.is_split() {
        (current, true)
    } else {
        let n = app.split.pane_count().max(1);
        let idx = pane.min(n.saturating_sub(1));
        let focus = app.split.focus_index();
        if idx == focus {
            (current, true)
        } else {
            (
                app.split
                    .panes
                    .get(idx)
                    .map(|p| app.pane_tab(p))
                    .unwrap_or(0),
                false,
            )
        }
    };
    let buf = buffer_for_tab(app, tab);
    let total = buf.line_count() as u32;
    let caret_vcol = if focused {
        let c = app.buffer.cursor();
        visual_col(app.buffer.line(c.row), drawn_caret_col(app), app.tab_width) as u32
    } else {
        0
    };
    let sel = if focused { app.selected_range() } else { None };
    let lines = build_lines_at(app, tab, start, rows, Some(caret_vcol), sel, focused, wrap_cols, wide_ratio);
    (lines, total)
}

/// Scroll-hot path: rebuild **editor surfaces only**, keep explorer/SCM/outline/theme.
/// Avoids re-walking the project tree and git workbench on every trackpad tick.
pub fn patch_chrome_editor_scroll(app: &App, frame_gen: u64, chrome: &mut ChromeScene) {
    let cursor = app.buffer.cursor();
    let line_count = app.buffer.line_count().max(1);
    let scroll = app.scroll.min(line_count.saturating_sub(1));
    let pct = if line_count <= 1 {
        100
    } else {
        ((cursor.row.saturating_mul(100)) / (line_count.saturating_sub(1))) as u32
    };
    // Match compose() welcome rules (never flip to welcome mid-scroll).
    let any_named_tab = app.filename.is_some()
        || app.tabs.buffers.iter().any(|t| {
            t.filename
                .as_ref()
                .is_some_and(|p| !p.as_os_str().is_empty())
        });
    let multi_tab = app.tabs.buffers.len() > 1;
    let project_seeded = !app.explorer.entries.is_empty();
    let welcome = !multi_tab
        && !any_named_tab
        && !project_seeded
        && !app.modified
        && app.buffer.line_count() == 1
        && app.buffer.line(0).is_empty();
    // Large overscan so AppKit Responsive Scrolling overdraw stays filled (WWDC 2013-215).
    // Cap to packed-line budget (SUISEI_MAX_LINES ≈ 256).
    let rows = (app.grid_rows().max(8) as usize)
        .saturating_mul(3)
        .saturating_add(48)
        .min(240);
    let caret_vcol = if welcome {
        0
    } else {
        visual_col(app.buffer.line(cursor.row), drawn_caret_col(app), app.tab_width) as u32
    };
    let sel = app.selected_range();
    // Patch path may reuse unfocused pane snapshots (typing hot path in splits).
    let prev_panes = std::mem::take(&mut chrome.panes);
    let (lines, pane_focus, panes) = if welcome {
        (Vec::new(), 0u8, Vec::new())
    } else {
        // Hot path (per keystroke / scroll): skip the packed lines — the face
        // pulls its own rows, so these were pure churn every frame.
        build_editor_surfaces(app, scroll, rows, caret_vcol, sel, prev_panes, false)
    };

    chrome.mode_label = mode_label(app).into();
    chrome.message = app.message.clone();
    chrome.dirty_buffer = app.modified;
    chrome.welcome = welcome;
    chrome.cursor_row = cursor.row.saturating_add(1) as u32;
    chrome.cursor_col = cursor.col.saturating_add(1) as u32;
    chrome.caret_vcol = caret_vcol;
    chrome.scroll_intent = app.scroll_intent as u8;
    chrome.line_count = line_count as u32;
    chrome.scroll = scroll as u32;
    chrome.pct = pct;
    chrome.scroll_frac = app.scroll_frac;
    chrome.hscroll = app.hscroll as u32;
    chrome.wrap_lines = u8::from(app.wrap_lines);
    chrome.relative_number = u8::from(app.relative_number);
    chrome.buffer_version = app.buffer.version();
    chrome.lines = lines;
    chrome.pane0_kind = app
        .split
        .panes
        .first()
        .map_or(Default::default(), |p| app.tab_kind(p.buffer));
    chrome.pane0_title = tab_title(app, app.current_buffer());
    chrome.pane_focus = pane_focus;
    chrome.panes = panes;
    // Terminal PTY can change while scrolling; keep it live. Skip explorer/scm/git/outline/theme.
    if chrome.terminal.open || matches!(app.mode, Mode::Terminal) {
        chrome.terminal = build_terminal(app);
    }
    // Leader (Space) sets no pending_key — gate on visibility itself, and also
    // rebuild when the scene still shows an open popup so it can close.
    if app.completions.active || !app.completions.suggestions.is_empty() || chrome.completions.open
    {
        chrome.completions = build_completions(app);
    }
    // Preview pan (hscroll) re-slices line text; open/close can also happen
    // without a mode change on the face side.
    if app.preview.open != chrome.preview.open
        || (app.preview.open && app.preview.hscroll as u32 != chrome.preview.hscroll)
    {
        chrome.preview = build_preview(app);
    }
    let _ = frame_gen;
}

pub fn compose(app: &App, frame_gen: u64, outline: &[OutlineItemScene]) -> FrameDiff {
    let filename = app
        .filename
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "[No Name]".into());

    let breadcrumbs = app
        .filename
        .as_ref()
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .filter(|s| s != "/" && s != "\\")
                .collect::<Vec<_>>()
                .join(" › ")
        })
        .unwrap_or_else(|| "welcome".into());

    let cursor = app.buffer.cursor();
    let line_count = app.buffer.line_count().max(1);
    let scroll = app.scroll.min(line_count.saturating_sub(1));
    let pct = if line_count <= 1 {
        100
    } else {
        ((cursor.row.saturating_mul(100)) / (line_count.saturating_sub(1))) as u32
    };

    // Welcome = cold launch only. Never re-enter after multi-tab / any named buffer /
    // project tree seed — otherwise "+ New Tab" (empty Untitled) snaps back to WelcomeView.
    //
    // Rules:
    // - `explorer.cwd` defaults to `current_dir()` (often `/` under `open`) — NEVER seed from cwd alone.
    // - `explorer.open` alone must NOT kill welcome (docked tree flag without entries is OK).
    // - Only real project tree entries, named files (incl. Untitled tab), multi-tab, or edits leave welcome.
    let any_named_tab = app.filename.is_some()
        || app.tabs.buffers.iter().any(|t| {
            t.filename
                .as_ref()
                .is_some_and(|p| !p.as_os_str().is_empty())
        });
    let multi_tab = app.tabs.buffers.len() > 1;
    let project_seeded = !app.explorer.entries.is_empty();
    let welcome = !multi_tab
        && !any_named_tab
        && !project_seeded
        && !app.modified
        && app.buffer.line_count() == 1
        && app.buffer.line(0).is_empty();

    let tabs = build_tabs(app);
    let outline = if welcome {
        Vec::new()
    } else {
        outline.to_vec()
    };
    // Match patch_chrome_editor_scroll: fat overscan for smooth NSScrollView overdraw.
    let rows = (app.grid_rows().max(8) as usize)
        .saturating_mul(3)
        .saturating_add(48)
        .min(240);
    let caret_vcol = if welcome {
        0
    } else {
        visual_col(app.buffer.line(cursor.row), drawn_caret_col(app), app.tab_width) as u32
    };
    let sel = app.selected_range();
    let (lines, pane_focus, panes) = if welcome {
        (Vec::new(), 0u8, Vec::new())
    } else {
        // Full compose keeps building lines: the TUI face renders from them and
        // the engine tests assert on them.
        build_editor_surfaces(app, scroll, rows, caret_vcol, sel, Vec::new(), true)
    };

    FrameDiff {
        frame_gen,
        chrome: Some(ChromeScene {
            mode_label: mode_label(app).into(),
            message: app.message.clone(),
            filename,
            breadcrumbs,
            dirty_buffer: app.modified,
            welcome,
            explorer_open: app.explorer.open,
            cursor_row: cursor.row.saturating_add(1) as u32,
            cursor_col: cursor.col.saturating_add(1) as u32,
            caret_vcol,
            scroll_intent: app.scroll_intent as u8,
            line_count: line_count as u32,
            scroll: scroll as u32,
            pct,
            scroll_frac: app.scroll_frac,
            hscroll: app.hscroll as u32,
            wrap_lines: u8::from(app.wrap_lines),
            relative_number: u8::from(app.relative_number),
            buffer_version: app.buffer.version(),
            branch: branch_name(app),
            tabs,
            lines,
            pane0_kind: app
                .split
                .panes
                .first()
                .map_or(Default::default(), |p| app.tab_kind(p.buffer)),
            pane0_title: tab_title(app, app.current_buffer()),
            pane_focus,
            panes,
            explorer: build_explorer(app),
            palette: build_palette(app),
            search: build_search(app),
            completions: build_completions(app),
            terminal: build_terminal(app),
            settings: build_settings(app),
            theme: build_theme(app),
            scm: build_scm(app),
            git_wb: build_git_wb(app),
            outline,
            preview: build_preview(app),
        }),
    }
}

fn build_preview(app: &App) -> PreviewScene {
    if !app.preview.open {
        return PreviewScene::default();
    }
    let kind = match app.preview.kind {
        Some(suisei_core::preview::PreviewKind::Markdown) => 1u8,
        Some(suisei_core::preview::PreviewKind::Json) => 2,
        Some(suisei_core::preview::PreviewKind::Plain) => 3,
        Some(suisei_core::preview::PreviewKind::Image) => 4,
        Some(suisei_core::preview::PreviewKind::Csv) => 5,
        Some(suisei_core::preview::PreviewKind::Npy) => 6,
        Some(suisei_core::preview::PreviewKind::Audio) => 7,
        None => 0,
    };
    // Face owns its ScrollView — always send from line 0 (ignore TUI scroll).
    // Large cap; face pulls via range/chunk FFI so ABI struct size stays modest.
    const MAX_LINES: usize = 8_000;
    const MAX_CHARS: usize = 2_000;
    let end = app.preview.lines.len().min(MAX_LINES);
    let h = app.preview.hscroll;
    let mut lines = Vec::with_capacity(end);
    for pl in app.preview.lines.iter().take(end) {
        let mut text = String::new();
        let mut style = 0u8;
        let mut best_rank = 99u8;
        let multi = pl.spans.len() > 1;
        for (s, st) in &pl.spans {
            let code = preview_style_code(*st);
            let rank = preview_style_rank(code);
            if rank < best_rank {
                best_rank = rank;
                style = code;
            }
            if multi {
                // Private-use marker byte for face AttributedString rebuild.
                if let Some(mark) = char::from_u32(0xE000 + u32::from(code)) {
                    text.push(mark);
                }
            }
            text.push_str(s);
        }
        if let Some(img) = &pl.image {
            if text.is_empty() {
                text = format!("🖼  {}", img.path);
                style = 8;
            } else if !text.contains(&img.path) {
                text.push_str(&format!("  🖼 {}", img.path));
            }
        }
        // Horizontal pan only when face has no independent h-scroll yet.
        if h > 0 && !text.is_empty() {
            let chars: Vec<char> = text.chars().collect();
            // Skip past any leading style markers when panning.
            let mut idx = h.min(chars.len());
            while idx < chars.len() {
                let c = chars[idx];
                if (0xE000..=0xE0FF).contains(&(c as u32)) {
                    idx += 1;
                    continue;
                }
                break;
            }
            if idx < chars.len() {
                text = chars[idx..].iter().collect();
            } else {
                text.clear();
            }
        }
        if text.len() > MAX_CHARS {
            let mut cut = MAX_CHARS;
            while cut > 0 && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('…');
        }
        lines.push(PreviewLineScene { text, style });
    }
    PreviewScene {
        open: true,
        kind,
        scroll: app.preview.scroll as u32,
        hscroll: app.preview.hscroll as u32,
        lines,
    }
}

fn preview_style_code(s: suisei_core::preview::PreviewStyle) -> u8 {
    use suisei_core::preview::PreviewStyle::*;
    match s {
        Normal => 0,
        H1 => 1,
        H2 => 2,
        H3 => 3,
        H4 | H5 | H6 => 4,
        Bold | BoldItalic => 5,
        Italic => 6,
        Code | CodeBlock | CodeLang | Kbd => 7,
        Link | Image | Footnote => 8,
        Quote | AlertNote | AlertTip | AlertImportant | AlertWarning | AlertCaution => 9,
        ListBullet | TaskDone | TaskTodo => 10,
        Hr | Dim | Html | Strike => 11,
        JsonKey => 12,
        JsonString => 13,
        JsonNumber | JsonLit => 14,
    }
}

/// Lower rank wins when picking a single style for a multi-span line.
fn preview_style_rank(code: u8) -> u8 {
    match code {
        1 => 0, // H1
        2 => 1,
        3 => 2,
        4 => 3,
        7 => 4,  // code
        5 => 5,  // bold
        12 => 6, // json key
        8 => 7,  // link
        9 => 8,  // quote/alert
        10 => 9,
        13 | 14 => 10,
        6 => 11,
        11 => 12,
        _ => 20,
    }
}

/// Lightweight structure scan for Inspector / jump-bar (no LSP required).
fn build_outline(app: &App) -> Vec<OutlineItemScene> {
    let ext = app
        .filename
        .as_ref()
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let n = app.buffer.line_count();
    let mut out = Vec::new();
    let max_items = 200usize;
    // Hoisted. This is a property of the FILE, and it was being recomputed per
    // line: `Lang::from_ext` is a linear scan of 29 languages, each comparing
    // against a slice of extensions, so a 5000-line file paid something like a
    // hundred thousand string comparisons for one answer that never changed.
    let code_like = suisei_core::lang::Lang::from_ext(&ext)
        .map(|l| l.scope().is_some())
        .unwrap_or(false)
        || matches!(ext.as_str(), "kt" | "kts" | "scala" | "dart" | "zig" | "ex")
        || ext.is_empty();
    for i in 0..n {
        if out.len() >= max_items {
            break;
        }
        let line = app.buffer.line(i);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Markdown / headings
        if trimmed.starts_with('#') {
            let depth = trimmed.chars().take_while(|c| *c == '#').count().min(6);
            if depth > 0 {
                let title = trimmed[depth..].trim();
                if !title.is_empty() {
                    out.push(OutlineItemScene {
                        name: title.to_string(),
                        row: (i + 1) as u32,
                        kind: 0,
                        depth: (depth.saturating_sub(1)) as u8,
                    });
                    continue;
                }
            }
        }
        // Code symbols. Gated on the language having lexical declarations at
        // all, rather than on a hand-kept list of sixteen extensions that had
        // C#, Ruby, PHP, Lua, Scala, Dart, Zig, Haskell and Elixir missing —
        // their outline panel was empty while the file highlighted fine. Data
        // and markup languages stay out: `outline_code_line` matches `fn `,
        // `class `, `def ` and friends, which mean nothing in YAML or CSS.
        if code_like {
            if let Some(item) = outline_code_line(trimmed, i) {
                out.push(item);
            }
        }
    }
    out
}

fn outline_code_line(trimmed: &str, line_idx: usize) -> Option<OutlineItemScene> {
    let row = (line_idx + 1) as u32;
    // Strip common visibility / attributes noise
    let s = trimmed
        .trim_start_matches("pub ")
        .trim_start_matches("private ")
        .trim_start_matches("public ")
        .trim_start_matches("internal ")
        .trim_start_matches("open ")
        .trim_start_matches("async ")
        .trim_start_matches("export ")
        .trim_start_matches("default ");

    let (kind, rest) = if let Some(r) = s.strip_prefix("fn ") {
        (1u8, r)
    } else if let Some(r) = s.strip_prefix("func ") {
        (1, r)
    } else if let Some(r) = s.strip_prefix("def ") {
        (1, r)
    } else if let Some(r) = s.strip_prefix("function ") {
        (1, r)
    } else if let Some(r) = s.strip_prefix("struct ") {
        (2, r)
    } else if let Some(r) = s.strip_prefix("class ") {
        (2, r)
    } else if let Some(r) = s.strip_prefix("enum ") {
        (2, r)
    } else if let Some(r) = s.strip_prefix("trait ") {
        (2, r)
    } else if let Some(r) = s.strip_prefix("interface ") {
        (2, r)
    } else if let Some(r) = s.strip_prefix("protocol ") {
        (2, r)
    } else if let Some(r) = s.strip_prefix("impl ") {
        (2, r)
    } else if let Some(r) = s.strip_prefix("type ") {
        (2, r)
    } else if let Some(r) = s.strip_prefix("mod ") {
        (3, r)
    } else if let Some(r) = s.strip_prefix("extension ") {
        (2, r)
    } else {
        return None;
    };

    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
        .collect();
    if name.is_empty() {
        // impl Trait for Type — take a short slice
        let short: String = rest.chars().take(40).collect();
        if short.is_empty() {
            return None;
        }
        return Some(OutlineItemScene {
            name: short.trim_end_matches('{').trim().to_string(),
            row,
            kind,
            depth: 0,
        });
    }
    Some(OutlineItemScene {
        name,
        row,
        kind,
        depth: 0,
    })
}

fn build_git_wb(app: &App) -> GitWbScene {
    use suisei_core::git_workbench::{GitPane, GitTab, HistoryView};

    // Face paints workbench whenever Core has it open — Mode may be Normal after UI clicks.
    let open = app.git_wb.open;
    if !open {
        return GitWbScene {
            open: false,
            docked: false,
            tab_index: 0,
            branch: String::new(),
            message: String::new(),
            chips: Vec::new(),
            col_changes: Vec::new(),
            col_log: Vec::new(),
            col_files: Vec::new(),
            special: Vec::new(),
            loading: false,
        };
    }

    let mut branch = if app.git_wb.branch.is_empty() {
        "HEAD".into()
    } else {
        app.git_wb.branch.clone()
    };
    if app.git_wb.ahead > 0 || app.git_wb.behind > 0 {
        branch.push_str(&format!(" ↑{}↓{}", app.git_wb.ahead, app.git_wb.behind));
    }

    let docked = matches!(
        app.git_wb.tab,
        GitTab::Status | GitTab::History | GitTab::Commit
    );
    let tab = app.git_wb.tab;
    let pane = app.git_wb.pane;
    // Clean chip labels for GUI
    let chips = vec![
        GitWbChip {
            label: "Status".into(),
            active: matches!(tab, GitTab::Status | GitTab::Commit) && pane != GitPane::Files,
            key: 1,
        },
        GitWbChip {
            label: "Log".into(),
            active: matches!(tab, GitTab::History),
            key: 2,
        },
        GitWbChip {
            label: "Branches".into(),
            active: tab == GitTab::Branches,
            key: 3,
        },
        GitWbChip {
            label: "Files".into(),
            active: matches!(tab, GitTab::Status | GitTab::Commit) && pane == GitPane::Files,
            key: 4,
        },
        GitWbChip {
            label: "Diff".into(),
            active: tab == GitTab::Diff,
            key: 5,
        },
        GitWbChip {
            label: "PRs".into(),
            active: tab == GitTab::PullRequests,
            key: 6,
        },
        GitWbChip {
            label: "Issues".into(),
            active: tab == GitTab::Issues,
            key: 7,
        },
        GitWbChip {
            label: "Auth".into(),
            active: tab == GitTab::Auth,
            key: 8,
        },
        GitWbChip {
            label: "Stash".into(),
            active: tab == GitTab::Stash,
            key: 9,
        },
    ];

    let message = if app.git_wb.is_loading() {
        format!(
            "{} {}",
            app.git_wb.spinner_frame(),
            app.git_wb.loading_label().unwrap_or("Loading…")
        )
    } else {
        app.git_wb
            .message
            .clone()
            .or_else(|| app.git_wb.error.clone())
            .unwrap_or_else(|| "Space stage · c commit · Enter open · Tab pane · Esc".into())
    };

    // ── Changes column ──
    let mut col_changes = Vec::new();
    let n_ch = app.git_wb.changes.len() + app.git_wb.staged.len();
    col_changes.push(format!("▾ Changes  {n_ch}"));
    if !app.git_wb.staged.is_empty() {
        col_changes.push(format!("  Staged ({})", app.git_wb.staged.len()));
        for (i, e) in app.git_wb.staged.iter().enumerate().take(40) {
            let mark = if i == app.git_wb.selected { "›" } else { " " };
            col_changes.push(format!("{mark} {} {}", e.status.letter(), e.path));
        }
    }
    col_changes.push(format!("  Local Changes ({})", app.git_wb.changes.len()));
    if app.git_wb.changes.is_empty() && app.git_wb.staged.is_empty() {
        col_changes.push("  (clean)".into());
    } else {
        let base = app.git_wb.staged.len();
        for (i, e) in app.git_wb.changes.iter().enumerate().take(50) {
            let mark = if base + i == app.git_wb.selected {
                "›"
            } else {
                " "
            };
            col_changes.push(format!("{mark} {} {}", e.status.letter(), e.path));
        }
    }
    if !app.git_wb.commit_buf.is_empty() || app.git_wb.commit_editing {
        col_changes.push("── Commit ──".into());
        col_changes.push(format!("  {}", app.git_wb.commit_buf));
    }

    // ── Log column ──
    let mut col_log = Vec::new();
    if matches!(app.git_wb.history_view, HistoryView::Graph) && !app.git_wb.history_graph.is_empty()
    {
        col_log.push("▾ Log · graph  (v list)".into());
        for (i, row) in app.git_wb.history_graph.iter().enumerate().take(60) {
            let mark = if i == app.git_wb.history_sel {
                "›"
            } else {
                " "
            };
            let strip: String = row.glyphs.iter().map(|g| g.ch()).collect();
            col_log.push(format!(
                "{mark}{strip} {} {}",
                row.short,
                row.subject.chars().take(48).collect::<String>()
            ));
        }
    } else {
        col_log.push("▾ Log · list  (v graph)".into());
        for (i, c) in app.git_wb.commits.iter().enumerate().take(60) {
            let mark = if i == app.git_wb.history_sel {
                "›"
            } else {
                " "
            };
            col_log.push(format!(
                "{mark}{}  {}",
                c.short,
                c.subject.chars().take(52).collect::<String>()
            ));
        }
    }
    if col_log.len() <= 1 {
        col_log.push("  (loading history…)".into());
    }

    // ── Files column (commit detail) ──
    let mut col_files = Vec::new();
    col_files.push("▾ Files".into());
    if let Some(ref d) = app.git_wb.commit_detail {
        col_files.push(format!(
            "  {}  {}",
            d.short,
            d.subject.chars().take(40).collect::<String>()
        ));
        for (i, f) in d.files.iter().enumerate().take(40) {
            let mark = if i == app.git_wb.commit_file_sel {
                "›"
            } else {
                " "
            };
            col_files.push(format!("{mark} {}", f.path));
        }
    } else {
        col_files.push("  (select a commit)".into());
    }

    // ── Special full-width tabs ──
    let mut special = Vec::new();
    // Docked primary modes render Diff as their detail pane rather than as a
    // peer top-level destination. Selection keeps Status/History active.
    if docked {
        if let Some(ref path) = app.git_wb.diff_path {
            special.push(format!("diff · {path}"));
            for line in app
                .git_wb
                .diff_lines
                .iter()
                .skip(app.git_wb.diff_scroll)
                .take(50)
            {
                special.push(line.text.clone());
            }
        }
    }
    match app.git_wb.tab {
        GitTab::Branches => {
            for (i, b) in app.git_wb.branches.iter().enumerate().take(50) {
                let mark = if i == app.git_wb.branch_sel {
                    "›"
                } else {
                    " "
                };
                let cur = if b.current { "*" } else { " " };
                // Preserve branch semantics across the GUI snapshot. Parsing
                // a slash in the name cannot distinguish `feature/foo` from
                // `origin/foo`, so carry an explicit local/remote marker.
                let scope = if b.remote { "R" } else { "L" };
                special.push(format!("{mark}{cur}{scope} {}", b.name));
            }
            if special.is_empty() {
                special.push("(loading branches…)".into());
            }
        }
        GitTab::Diff => {
            if let Some(ref p) = app.git_wb.diff_path {
                special.push(format!("diff · {p}"));
            }
            for dl in app
                .git_wb
                .diff_lines
                .iter()
                .skip(app.git_wb.diff_scroll)
                .take(50)
            {
                special.push(dl.text.clone());
            }
            if special.len() <= 1 {
                special.push("(no diff — select a file)".into());
            }
        }
        GitTab::PullRequests => {
            special.push(format!(
                "Pull Requests · {} · {}",
                app.git_wb.pr_state.label(),
                if app.git_wb.is_loading() {
                    "loading…"
                } else if !app.git_wb.gh_available {
                    "gh CLI missing"
                } else {
                    "gh"
                }
            ));
            if !app.git_wb.gh_available {
                special.push("Install GitHub CLI: brew install gh".into());
                special.push("Then: gh auth login".into());
            } else if app.git_wb.prs.is_empty() && app.git_wb.prs_loaded {
                if let Some(ref e) = app.git_wb.error {
                    special.push(format!("Error: {e}"));
                } else if let Some(ref m) = app.git_wb.message {
                    special.push(m.clone());
                }
                special.push("No open PRs (or not logged in)".into());
                special.push("Auth tab → gh auth login · then re-open PRs".into());
            }
            for (i, p) in app
                .git_wb
                .pr_filtered
                .iter()
                .filter_map(|&index| app.git_wb.prs.get(index))
                .enumerate()
                .take(40)
            {
                let mark = if i == app.git_wb.pr_sel { "›" } else { " " };
                let draft = if p.is_draft { " [draft]" } else { "" };
                special.push(format!(
                    "{mark}#{:<5} {}{}  · {}",
                    p.number,
                    p.title.chars().take(48).collect::<String>(),
                    draft,
                    p.head_ref
                ));
            }
            if special.len() <= 1 && app.git_wb.is_loading() {
                special.push("Fetching pull requests via gh…".into());
            }
        }
        GitTab::Issues => {
            special.push(format!(
                "Issues · {} · {}",
                app.git_wb.issue_state.label(),
                if app.git_wb.is_loading() {
                    "loading…"
                } else if !app.git_wb.gh_available {
                    "gh CLI missing"
                } else {
                    "gh"
                }
            ));
            if !app.git_wb.gh_available {
                special.push("Install GitHub CLI: brew install gh".into());
                special.push("Then: gh auth login".into());
            } else if app.git_wb.issues.is_empty() && app.git_wb.issues_loaded {
                if let Some(ref e) = app.git_wb.error {
                    special.push(format!("Error: {e}"));
                }
                special.push("No open issues (or not logged in)".into());
                special.push("Auth tab → check login · then re-open Issues".into());
            }
            for (i, iss) in app
                .git_wb
                .issue_filtered
                .iter()
                .filter_map(|&index| app.git_wb.issues.get(index))
                .enumerate()
                .take(40)
            {
                let mark = if i == app.git_wb.issue_sel {
                    "›"
                } else {
                    " "
                };
                special.push(format!(
                    "{mark}#{:<5} {}  · {}",
                    iss.number,
                    iss.title.chars().take(52).collect::<String>(),
                    iss.state
                ));
            }
            if special.len() <= 1 && app.git_wb.is_loading() {
                special.push("Fetching issues via gh…".into());
            }
        }
        GitTab::Auth => {
            special.push(format!(
                "GitHub CLI: {}",
                if app.git_wb.gh_available {
                    "installed"
                } else {
                    "not found — brew install gh"
                }
            ));
            if !app.git_wb.auth.user.is_empty() {
                special.push(format!("User: {}", app.git_wb.auth.user));
            } else if app.git_wb.gh_available {
                special.push("Not logged in — run: gh auth login".into());
            }
            if !app.git_wb.auth.detail.is_empty() {
                special.push(app.git_wb.auth.detail.clone());
            }
            if let Some(ref e) = app.git_wb.error {
                special.push(format!("Error: {e}"));
            }
        }
        GitTab::Stash => {
            for (i, s) in app.git_wb.stashes.iter().enumerate().take(40) {
                let mark = if i == app.git_wb.stash_sel {
                    "›"
                } else {
                    " "
                };
                special.push(format!("{mark}{s}"));
            }
            if special.is_empty() {
                special.push("(empty stash)".into());
            }
        }
        _ => {}
    }

    let tab_index = match app.git_wb.tab {
        GitTab::Status => 0u32,
        GitTab::History | GitTab::Commit => 1,
        GitTab::Branches => 2,
        GitTab::Diff => 4,
        GitTab::PullRequests => 5,
        GitTab::Issues => 6,
        GitTab::Auth => 7,
        GitTab::Stash => 8,
    };

    GitWbScene {
        open: true,
        docked,
        tab_index,
        branch,
        message,
        chips,
        col_changes,
        col_log,
        col_files,
        special,
        loading: app.git_wb.is_loading(),
    }
}

fn build_scm(app: &App) -> ScmScene {
    // Docked Source Control navigator keeps data without Mode::SourceControl
    // (same pattern as Project tree + Mode::Editor).
    let open = app.scm.open || app.scm.visible();
    if !open {
        return ScmScene {
            open: false,
            branch: String::new(),
            status: String::new(),
            staged: Vec::new(),
            changes: Vec::new(),
            graph: Vec::new(),
        };
    }
    let branch = if app.scm.branch.is_empty() {
        "git".into()
    } else {
        let mut b = app.scm.branch.clone();
        if app.scm.ahead > 0 || app.scm.behind > 0 {
            b.push_str(&format!(" ↑{} ↓{}", app.scm.ahead, app.scm.behind));
        }
        b
    };
    let status = app
        .scm
        .last_result
        .clone()
        .or_else(|| app.scm.error.clone())
        .unwrap_or_else(|| {
            format!(
                "{} file(s) · Ctrl+Shift+G full Git",
                app.scm.staged.len() + app.scm.changes.len()
            )
        });
    // Flattened selection index: staged first, then changes
    let mut idx = 0usize;
    let staged: Vec<ScmEntryScene> = app
        .scm
        .staged
        .iter()
        .take(40)
        .map(|e| {
            let selected = idx == app.scm.selected;
            idx += 1;
            ScmEntryScene {
                path: e.path.clone(),
                mark: e.status.letter().to_string(),
                staged: true,
                selected,
            }
        })
        .collect();
    let changes: Vec<ScmEntryScene> = app
        .scm
        .changes
        .iter()
        .take(60)
        .map(|e| {
            let selected = idx == app.scm.selected;
            idx += 1;
            ScmEntryScene {
                path: e.path.clone(),
                mark: e.status.letter().to_string(),
                staged: false,
                selected,
            }
        })
        .collect();
    let graph: Vec<ScmGraphRowScene> = app
        .scm
        .graph
        .iter()
        .enumerate()
        .take(40)
        .map(|(i, row)| {
            let strip: String = row.glyphs.iter().map(|g| g.ch()).collect();
            ScmGraphRowScene {
                strip,
                short: row.short.clone(),
                // Was truncated to 56 characters HERE, in the compositor, so
                // the face could not have shown more even with room for it.
                // Truncation is a layout decision and belongs where the width
                // is known.
                subject: row.subject.clone(),
                when: row.when.clone(),
                refs: row.refs.clone(),
                color: row.color,
                unpushed: row.unpushed,
                selected: i == app.scm.graph_selected,
            }
        })
        .collect();

    ScmScene {
        open: true,
        branch,
        status,
        staged,
        changes,
        graph,
    }
}

/// Pack theme Color → 0x00RRGGBB without depending on ratatui in this crate.
/// Pack a theme colour for the face as `0xAARRGGBB`.
///
/// This used to `format!("{c:?}")` the colour and parse the Debug string back,
/// because the theme carried `ratatui::style::Color` — a terminal type the
/// engine could not read directly. It is a field read now.
fn color_u32(c: suisei_core::theme::Rgba) -> u32 {
    c.argb()
}
fn build_theme(app: &App) -> ThemeScene {
    let t = app.theme;
    ThemeScene {
        name: t.name.into(),
        editor_bg: color_u32(t.editor_bg),
        fg: color_u32(t.fg),
        dim: color_u32(t.line_no),
        current_line: color_u32(t.current_line),
        invisibles: color_u32(t.invisibles),
        accent: color_u32(t.accent),
        selection: color_u32(t.selection_bg),
        caret: color_u32(t.cursor),
        status_bg: color_u32(t.status_bg),
        keyword: color_u32(t.keyword),
        string: color_u32(t.string),
        comment: color_u32(t.comment),
        number: color_u32(t.number),
        type_name: color_u32(t.type_name),
        function: color_u32(t.function),
        macro_name: color_u32(t.macro_name),
        namespace: color_u32(t.namespace),
        parameter: color_u32(t.parameter),
        property: color_u32(t.property),
        constant: color_u32(t.constant),
        operator: color_u32(t.operator),
        punctuation: color_u32(t.punctuation),
        window_bg: color_u32(t.bg),
        border: color_u32(t.border),
        panel_bg: color_u32(t.panel_bg),
        panel_border: color_u32(t.panel_border),
        panel_sel_bg: color_u32(t.panel_sel_bg),
        panel_sel_fg: color_u32(t.panel_sel_fg),
        explorer_bg: color_u32(t.explorer_bg),
        explorer_fg: color_u32(t.explorer_fg),
        explorer_selected: color_u32(t.explorer_selected),
        status_fg: color_u32(t.status_fg),
        muted: color_u32(t.muted),
        success: color_u32(t.success),
        warning: color_u32(t.warning),
        error: color_u32(t.error),
        accent_fg: color_u32(t.accent_fg),
        search_bg: color_u32(t.search_bg),
        completion_bg: color_u32(t.completion_bg),
        completion_selected: color_u32(t.completion_selected),
        completion_border: color_u32(t.completion_border),
        terminal_bg: color_u32(t.terminal_bg),
        git_add_bg: color_u32(t.git_add_bg),
        git_del_bg: color_u32(t.git_del_bg),
        git_hunk: color_u32(t.git_hunk),
        model_bg: color_u32(t.model_bg),
        debug_stop: color_u32(t.debug_stop),
    }
}

fn build_settings(app: &App) -> SettingsScene {
    use suisei_core::settings::{SettingRow, SettingsPage, help_entries};
    use suisei_core::theme;

    // Face Settings window paints whenever panel is open (Mode may lag UI).
    let open = app.settings.visible();
    if !open {
        return SettingsScene {
            open: false,
            dirty: false,
            page_index: 0,
            selected: 0,
            status: String::new(),
            tabs: Vec::new(),
            rows: Vec::new(),
        };
    }

    let tabs: Vec<String> = SettingsPage::all()
        .iter()
        .map(|p| p.label().into())
        .collect();
    let page_index = SettingsPage::all()
        .iter()
        .position(|p| *p == app.settings.page)
        .unwrap_or(0) as u32;

    let mut rows = Vec::new();
    match app.settings.page {
        SettingsPage::About => {
            rows.push(SettingsRowScene {
                label: "Suisei".into(),
                value: String::new(),
                is_header: true,
                selected: false,
                kind: 0,
                payload: 0,
                page: 0,
                control: 0,
                value_index: 0,
                advanced: false,
                group: String::new(),
                detail: String::new(),
                options: String::new(),
            });
            rows.push(SettingsRowScene {
                label: "Version".into(),
                value: suisei_core::settings::SettingsPanel::version_string(),
                is_header: false,
                selected: false,
                kind: 0,
                payload: 0,
                page: 0,
                control: 0,
                value_index: 0,
                advanced: false,
                group: String::new(),
                detail: String::new(),
                options: String::new(),
            });
            rows.push(SettingsRowScene {
                label: "Core".into(),
                value: "xei-core".into(),
                is_header: false,
                selected: false,
                kind: 0,
                payload: 0,
                page: 0,
                control: 0,
                value_index: 0,
                advanced: false,
                group: String::new(),
                detail: String::new(),
                options: String::new(),
            });
            rows.push(SettingsRowScene {
                label: "Theme".into(),
                value: app.theme.name.into(),
                is_header: false,
                selected: false,
                kind: 0,
                payload: 0,
                page: 0,
                control: 0,
                value_index: 0,
                advanced: false,
                group: String::new(),
                detail: String::new(),
                options: String::new(),
            });
            rows.push(SettingsRowScene {
                label: "Config".into(),
                value: "~/.xei.toml".into(),
                is_header: false,
                selected: false,
                kind: 0,
                payload: 0,
                page: 0,
                control: 0,
                value_index: 0,
                advanced: false,
                group: String::new(),
                detail: String::new(),
                options: String::new(),
            });
        }
        // Drop TUI key-chord junk from status when painting for the face.
        SettingsPage::Setting => {
            let draft = &app.settings.draft;
            let themes = theme::all_themes();
            for (i, row) in app.settings.setting_rows().into_iter().enumerate() {
                let selected = i == app.settings.selected;
                let presentation = row.presentation();
                let (label, value, is_header) = match row {
                    SettingRow::ThemeHeader => (presentation.label.into(), String::new(), true),
                    SettingRow::AppearanceMode => (
                        presentation.label.into(),
                        match draft.theme.as_str() {
                            "light" => "light",
                            "dark" => "dark",
                            _ => "automatic",
                        }
                        .into(),
                        false,
                    ),
                    SettingRow::GlassStyle => {
                        (presentation.label.into(), draft.glass_style.clone(), false)
                    }
                    SettingRow::Theme(ti) => {
                        let name = themes.get(ti).map(|t| t.name).unwrap_or("?");
                        let mark = if draft.theme.eq_ignore_ascii_case(name) {
                            "●"
                        } else {
                            " "
                        };
                        (format!("{mark} {name}"), "Enter to apply".into(), false)
                    }
                    SettingRow::HighlightColor => (
                        presentation.label.into(),
                        draft.highlight_color.clone(),
                        false,
                    ),
                    SettingRow::EditorHeader => (presentation.label.into(), String::new(), true),
                    SettingRow::TabWidth => (
                        presentation.label.into(),
                        format!("{}", draft.tab_width),
                        false,
                    ),
                    SettingRow::UpdateCheck => (
                        presentation.label.into(),
                        if draft.update_check {
                            "automatically"
                        } else {
                            "manually"
                        }
                        .into(),
                        false,
                    ),
                    SettingRow::RelativeNumber => (
                        presentation.label.into(),
                        on_off(draft.relative_number),
                        false,
                    ),
                    SettingRow::WrapLines => {
                        (presentation.label.into(), on_off(draft.wrap_lines), false)
                    }
                    SettingRow::UndoCaching => {
                        (presentation.label.into(), on_off(draft.undo_caching), false)
                    }
                    SettingRow::ClipboardSync => (
                        presentation.label.into(),
                        on_off(draft.clipboard_sync),
                        false,
                    ),
                    SettingRow::GpuAcc => (presentation.label.into(), on_off(draft.gpu_acc), false),
                    SettingRow::GpuGraphics => {
                        (presentation.label.into(), on_off(draft.gpu_graphics), false)
                    }
                    SettingRow::GpuHyperlinks => (
                        presentation.label.into(),
                        on_off(draft.gpu_hyperlinks),
                        false,
                    ),
                    SettingRow::KeyHints => {
                        (presentation.label.into(), on_off(draft.key_hints), false)
                    }
                    SettingRow::LspHeader => (presentation.label.into(), String::new(), true),
                    SettingRow::LspEnabled => {
                        (presentation.label.into(), on_off(draft.lsp_enabled), false)
                    }
                    SettingRow::LspLang(li) => {
                        let catalog = suisei_core::config::lsp_lang_catalog();
                        let (key, label, _) = catalog.get(li).copied().unwrap_or(("?", "?", "?"));
                        let state = match draft.lsp_servers.get(key).map(|s| s.as_str()) {
                            None => "default",
                            Some("") => "off",
                            Some(_) => "custom",
                        };
                        (label.into(), state.into(), false)
                    }
                    SettingRow::GitHeader => (presentation.label.into(), String::new(), true),
                    SettingRow::OpenWorkbench | SettingRow::OpenScm => {
                        (presentation.label.into(), "Enter".into(), false)
                    }
                };
                rows.push(SettingsRowScene {
                    label,
                    value,
                    is_header,
                    selected,
                    kind: row.kind(),
                    payload: row.payload(),
                    page: presentation.page.code(),
                    control: presentation.control.code(),
                    value_index: row.value_index(draft),
                    advanced: presentation.advanced,
                    group: presentation.group.into(),
                    detail: presentation.detail.into(),
                    options: presentation.options.into(),
                });
            }
        }
        SettingsPage::Extensions => {
            rows.push(SettingsRowScene {
                label: "Extensions".into(),
                value: "Enter → plugin store".into(),
                is_header: false,
                selected: true,
                kind: 0,
                payload: 0,
                page: 0,
                control: 0,
                value_index: 0,
                advanced: false,
                group: String::new(),
                detail: String::new(),
                options: String::new(),
            });
            rows.push(SettingsRowScene {
                label: "VS Code compat host".into(),
                value: "in progress".into(),
                is_header: false,
                selected: false,
                kind: 0,
                payload: 0,
                page: 0,
                control: 0,
                value_index: 0,
                advanced: false,
                group: String::new(),
                detail: String::new(),
                options: String::new(),
            });
        }
        SettingsPage::Help => {
            for (i, e) in help_entries().iter().enumerate().take(40) {
                rows.push(SettingsRowScene {
                    label: if e.is_header {
                        e.desc.into()
                    } else {
                        e.keys.into()
                    },
                    value: if e.is_header {
                        String::new()
                    } else {
                        e.desc.into()
                    },
                    is_header: e.is_header,
                    selected: i == app.settings.selected && !e.is_header,
                    kind: 0,
                    payload: 0,
                    page: 0,
                    control: 0,
                    value_index: 0,
                    advanced: false,
                    group: String::new(),
                    detail: String::new(),
                    options: String::new(),
                });
            }
        }
    }

    SettingsScene {
        open: true,
        dirty: app.settings.dirty,
        page_index,
        selected: app.settings.selected as u32,
        status: app.settings.status.clone().unwrap_or_default(),
        tabs,
        rows,
    }
}

fn on_off(v: bool) -> String {
    if v { "on".into() } else { "off".into() }
}

fn branch_name(app: &App) -> String {
    if !app.scm.branch.is_empty() {
        return app.scm.branch.clone();
    }
    if !app.git_wb.branch.is_empty() {
        return app.git_wb.branch.clone();
    }
    String::new()
}

fn build_completions(app: &App) -> CompletionsScene {
    if !app.completions.active || app.completions.suggestions.is_empty() {
        return CompletionsScene {
            open: false,
            prefix: String::new(),
            selected: 0,
            items: Vec::new(),
        };
    }
    let items = app
        .completions
        .suggestions
        .iter()
        .take(20)
        .map(|s| (s.label.clone(), s.detail.clone()))
        .collect();
    CompletionsScene {
        open: true,
        prefix: app.completions.prefix.clone(),
        selected: app.completions.selected as u32,
        items,
    }
}

fn build_terminal(app: &App) -> TerminalScene {
    TerminalScene {
        open: app.terminal.open || matches!(app.mode, Mode::Terminal),
    }
}

fn build_explorer(app: &App) -> ExplorerScene {
    // Docked Project navigator (Xcode-like) must keep the file tree painted even when
    // Mode is Normal / explorer.open is false. `open` only means keyboard-focused tree.
    let open = app.explorer.open || matches!(app.mode, Mode::Explorer);
    let entries = app
        .explorer
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| ExplorerEntryScene {
            name: e.name.clone(),
            is_dir: e.is_dir,
            selected: i == app.explorer.selected,
        })
        .collect();
    ExplorerScene {
        open,
        cwd: app.explorer.cwd.display().to_string(),
        entries,
    }
}

fn build_palette(app: &App) -> PaletteScene {
    let open = app.palette.open || matches!(app.mode, Mode::Palette);
    if !open {
        return PaletteScene {
            open: false,
            kind: String::new(),
            query: String::new(),
            items: Vec::new(),
        };
    }
    let kind = match app.palette.kind {
        suisei_core::palette::PaletteKind::Files => "Files",
        suisei_core::palette::PaletteKind::Commands => "Commands",
        suisei_core::palette::PaletteKind::Problems => "Problems",
        suisei_core::palette::PaletteKind::Symbols => "Symbols",
        suisei_core::palette::PaletteKind::CodeActions => "Code Actions",
    };
    let mut items = Vec::new();
    for (fi, &idx) in app.palette.filtered.iter().enumerate().take(40) {
        if let Some(it) = app.palette.items.get(idx) {
            items.push(PaletteItemScene {
                label: it.label.clone(),
                detail: it.detail.clone(),
                selected: fi == app.palette.selected,
            });
        }
    }
    // If filter empty, show raw items head
    if items.is_empty() && !app.palette.items.is_empty() {
        for (i, it) in app.palette.items.iter().enumerate().take(40) {
            items.push(PaletteItemScene {
                label: it.label.clone(),
                detail: it.detail.clone(),
                selected: i == app.palette.selected,
            });
        }
    }
    PaletteScene {
        open: true,
        kind: kind.into(),
        query: app.palette.query.clone(),
        items,
    }
}

fn build_search(app: &App) -> SearchScene {
    let open = matches!(app.mode, Mode::Search);
    SearchScene {
        open,
        forward: app.search.forward,
        input: if open {
            app.search.input.clone()
        } else {
            String::new()
        },
        match_count: app.search.matches.len() as u32,
        match_index: if open && !app.search.matches.is_empty() {
            app.search.current as u32
        } else {
            0
        },
        replace_open: open && app.search.replace_open,
        replace_input: app.search.replace_input.clone(),
    }
}

fn build_tabs(app: &App) -> Vec<TabScene> {
    let mut out = Vec::with_capacity(app.tabs.buffers.len().max(1));
    // Unified layouts whose chip has already been emitted at a member's
    // position — the remaining members are skipped.
    let mut unified_done: Vec<u64> = Vec::new();
    let current = app.current_buffer();
    for (i, tab) in app.tabs.buffers.iter().enumerate() {
        let is_current = i == current;
        let is_term = tab.terminal.is_some();
        let title = tab_title(app, i);
        let dirty = if tab.kind.is_viewer() {
            false
        } else if is_current {
            app.modified
        } else {
            tab.modified
        };
        // A document folded into a layout is drawn by that layout — either as
        // part of its group, or not at all when the layout is unified.
        let group = app
            .layout_holding(tab.id)
            .map(|l| (l.id, l.style))
            .unwrap_or((0, crate::LayoutStyleAlias::Grouped));
        if group.0 != 0 && group.1 == crate::LayoutStyleAlias::Unified {
            // The unified chip sits at its FIRST member's strip position —
            // the merge animation morphs container ⇄ chip in place instead of
            // flying the chip across the whole strip. Later members vanish
            // into it.
            if !unified_done.contains(&group.0) {
                unified_done.push(group.0);
                if let Some(l) = app.layouts.iter().find(|l| l.id == group.0) {
                    out.push(TabScene {
                        id: l.id,
                        title: l.name.clone(),
                        dirty: false,
                        active: app.active_layout == Some(l.id),
                        group: l.id,
                        is_layout: true,
                        is_terminal: false,
                        deleted: false,
                    });
                }
            }
            continue;
        }
        out.push(TabScene {
            id: tab.id.0,
            title,
            dirty,
            active: is_current,
            group: group.0,
            is_layout: false,
            is_terminal: is_term,
            // Dirty vanished tabs remain recoverable even while inactive; the
            // path check keeps their warning stable across tab switches.
            deleted: if is_current {
                app.file_deleted
            } else {
                tab.file_mtime.is_some()
                    && tab
                        .filename
                        .as_ref()
                        .is_some_and(|path| std::fs::metadata(path).is_err())
            },
        });
    }
    // Grouped layouts need no chip — their documents are the chips. A unified
    // layout with no member on the strip (cannot happen while the <2-document
    // dissolve holds, but stay honest) still gets its chip, at the end.
    for l in &app.layouts {
        if l.style == crate::LayoutStyleAlias::Unified && !unified_done.contains(&l.id) {
            out.push(TabScene {
                id: l.id,
                title: l.name.clone(),
                dirty: false,
                active: app.active_layout == Some(l.id),
                group: l.id,
                is_layout: true,
                is_terminal: false,
                deleted: false,
            });
        }
    }
    if out.is_empty() {
        out.push(TabScene {
            id: 0,
            title: "[No Name]".into(),
            dirty: app.modified,
            active: true,
            group: 0,
            is_layout: false,
            is_terminal: false,
            deleted: false,
        });
    }
    out
}

fn tab_title(app: &App, tab_index: usize) -> String {
    let Some(tab) = app.tabs.buffers.get(tab_index) else {
        return "[No Name]".into();
    };
    if tab.terminal.is_some() {
        // The shell's own title (OSC 0/2) when it has reported one —
        // `make`, `vim file`, an ssh session each name their tab.
        return tab
            .terminal_title
            .clone()
            .unwrap_or_else(|| "Terminal".to_string());
    }
    let filename = if tab_index == app.current_buffer() {
        app.filename.as_ref()
    } else {
        tab.filename.as_ref()
    };
    let name = filename
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("[No Name]");
    // A Logic tab carries the SOURCE file's path, so without this the strip
    // shows two tabs called `foo.rs` and neither says which is which.
    if tab.kind == suisei_core::media::FileKind::Logic {
        return format!("{name} · Logic");
    }
    name.to_string()
}

/// Single editor or multi-pane split surfaces for the Swift face.
/// `prev_panes` (patch path) lets unchanged **unfocused** panes be moved over
/// instead of re-tokenized — the typing hot path in splits.
fn build_editor_surfaces(
    app: &App,
    scroll: usize,
    rows: usize,
    caret_vcol: u32,
    sel: Option<(Position, Position)>,
    prev_panes: Vec<PaneScene>,
    // Build the packed visible-line stream, or leave it empty. The GUI face is
    // a PULL renderer: every canvas fetches its own rows via `build_editor_band`
    // (and terminals / preview / minimap pull their own separate snapshots), so
    // NOTHING reads these lines on the native path. The TUI still needs them, so
    // full `compose()` passes `true`; the per-keystroke scroll/edit hot path
    // passes `false` and skips ~240 line clones × pane count every keystroke.
    build_lines: bool,
) -> (Vec<EditorLineScene>, u8, Vec<PaneScene>) {
    if !app.split.is_split() {
        let lines = if build_lines {
            build_visible_lines_from_buffer(
                app,
                app.current_buffer(),
                scroll,
                rows,
                Some(caret_vcol),
                sel,
                true,
            )
        } else {
            Vec::new()
        };
        return (lines, 0, Vec::new());
    }

    let n = app.split.pane_count().max(1);
    // Reserve ~2 rows for per-pane path bar chrome in the face.
    const PATH_BAR_ROWS: usize = 2;
    const MAX_PACKED_LINES: usize = 256;
    let rects = app.split.rects();
    // Row budget PER PANE, from that pane's own share of the height. This used
    // to be one number for the whole layout, picked by `kind`: full height for
    // a vertical split, rows/n for a horizontal one. A tree can have both at
    // once, so a single number cannot describe it.
    let rows_for = |i: usize| -> usize {
        let h = rects
            .get(i)
            .map(|r: &suisei_core::split::Rect| r.h)
            .unwrap_or(1.0);
        let share = ((rows as f32) * h) as usize;
        share
            .saturating_sub(PATH_BAR_ROWS)
            .max(4)
            .min(MAX_PACKED_LINES / n)
            .max(4)
    };
    let focus = app.split.focus_index();
    let mut panes = Vec::with_capacity(n);
    let mut focused_lines = Vec::new();

    // Persist active buffer into focused pane snapshot for accurate paint.
    let mut prev_panes = prev_panes;
    let current = app.current_buffer();
    for (i, pane) in app.split.panes.iter().take(n).enumerate() {
        let focused = i == focus;
        let rows_each = rows_for(i);
        let rect = rects
            .get(i)
            .copied()
            .unwrap_or(suisei_core::split::Rect::FULL);
        let (tab, pane_scroll, pane_hscroll, pane_cursor) = if focused {
            (
                current,
                scroll,
                app.hscroll,
                (app.buffer.cursor.row, app.buffer.cursor.col),
            )
        } else {
            (app.pane_tab(pane), pane.scroll, pane.hscroll, pane.cursor)
        };
        let buf = buffer_for_tab(app, tab);
        let title = tab_title(app, tab);
        let doc_line_count = buf.line_count() as u32;
        let doc_version = buf.version();
        let eff_hscroll = if app.wrap_lines {
            0
        } else {
            pane_hscroll as u32
        };
        // Unfocused + identical inputs → move the previous snapshot over.
        if !focused {
            if let Some(idx) = prev_panes.iter().position(|p| {
                !p.focused
                    && p.tab_index == tab as u32
                    && p.title == title
                    && p.scroll == pane_scroll as u32
                    && p.hscroll == eff_hscroll
                    && p.doc_version == doc_version
                    && p.band_rows == rows_each as u32
                    && p.kind == app.tab_kind(pane.buffer)
                    && p.rect == rect
                    && p.doc_line_count == doc_line_count
            }) {
                panes.push(prev_panes.swap_remove(idx));
                continue;
            }
        }
        let caret = if focused {
            Some(caret_vcol)
        } else {
            let line = buf.line(pane_cursor.0.min(buf.line_count().saturating_sub(1)));
            Some(visual_col(line, pane_cursor.1, app.tab_width) as u32)
        };
        let pane_sel = if focused { sel } else { None };
        let lines = if build_lines {
            build_visible_lines_from_buffer(
                app,
                tab,
                pane_scroll,
                rows_each,
                caret,
                pane_sel,
                focused,
            )
        } else {
            Vec::new()
        };
        if focused {
            focused_lines = lines.clone();
        }
        panes.push(PaneScene {
            tab_index: tab as u32,
            title,
            doc_line_count,
            hscroll: eff_hscroll,
            scroll: pane_scroll as u32,
            focused,
            lines,
            doc_version,
            band_rows: rows_each as u32,
            kind: app.tab_kind(pane.buffer),
            rect,
        });
    }

    (focused_lines, focus as u8, panes)
}

pub(crate) fn buffer_for_tab(app: &App, tab: usize) -> &suisei_core::buffer::Buffer {
    if tab == app.current_buffer() {
        &app.buffer
    } else if let Some(t) = app.tabs.buffers.get(tab) {
        &t.buffer
    } else {
        &app.buffer
    }
}

/// Rows composed **above** `scroll` so native scroll faces can overdraw upward
/// without re-anchoring Core scroll (Core keeps `scroll` = true first visible line;
/// re-anchoring it above the viewport made `update_scroll` yank the view on the
/// next caret move / click).
const OVERSCAN_ABOVE: usize = 48;

fn build_visible_lines_from_buffer(
    app: &App,
    tab: usize,
    scroll: usize,
    rows: usize,
    caret_vcol: Option<u32>,
    sel: Option<(Position, Position)>,
    use_live_syntax: bool,
) -> Vec<EditorLineScene> {
    // Band starts above the viewport; `scroll` itself stays the visible top.
    build_lines_at(
        app,
        tab,
        scroll.saturating_sub(OVERSCAN_ABOVE),
        rows,
        caret_vcol,
        sel,
        use_live_syntax,
        // The packed-lines path never wrapped and does not now: it feeds the
        // chrome snapshot, which the GUI does not render from.
        0,
        scene_wide_default(),
    )
}

/// Ratio for the paths that do not wrap, where it cannot matter.
pub(crate) fn scene_wide_default() -> u16 {
    suisei_core::wrap::WIDE_TWO_CELLS
}

/// Rows `[band_start, band_start+rows)` — the shared line assembler.
fn build_lines_at(
    app: &App,
    tab: usize,
    band_start: usize,
    rows: usize,
    caret_vcol: Option<u32>,
    sel: Option<(Position, Position)>,
    use_live_syntax: bool,
    // `wide_ratio`: how wide a two-cell glyph really paints, in hundredths of
    // a narrow cell. The face's measurement, riding beside the width it
    // applies to — the two are one fact ("how this pane measures a row"), and
    // a pushed value has an ordering question that a parameter does not.
    wrap_cols: u16,
    wide_ratio: u16,
) -> Vec<EditorLineScene> {
    let buf = buffer_for_tab(app, tab);
    let total = buf.line_count();
    let is_current = tab == app.current_buffer();
    let cursor_row = if is_current {
        app.buffer.cursor().row
    } else if let Some(p) = app
        .split
        .panes
        .iter()
        // Strict: a pane whose document is closed has no cursor in `tab`, and
        // `pane_tab`'s fallback to the active tab would hand over someone
        // else's row.
        .find(|p| app.buffer_index(p.buffer) == Some(tab))
    {
        p.cursor.0
    } else {
        usize::MAX
    };
    // Columns a wrapped row may use. Zero means do not wrap, which is the
    // same shape `WrapMap` uses — one rule, one sentinel.
    //
    // Derived from the whole editor's grid, which is wrong in a split: each
    // pane is narrower than the editor, so a wrapped line in a split pane
    // breaks past its own right edge. The face is the only side that knows a
    // pane's real width, and handing that number down is the next change.
    // The FACE decides how many columns fit — it knows the pane's width in
    // points, the cell width, the gutter and whatever overlays the right edge.
    // This used to be `app.grid_cols() - 5`, the whole editor's columns, so a
    // wrapped line in a split pane broke past its own right edge.
    let wrap = wrap_cols > 0;
    // Resolve breakpoint path **once** — never canonicalize per row on the scroll hot path.
    let bp_lines: Option<std::collections::HashMap<usize, (bool, bool)>> = if is_current {
        let path_str = app
            .tabs
            .buffers
            .get(tab)
            .and_then(|t| t.filename.as_ref())
            .map(|p| p.to_string_lossy().to_string());
        path_str.and_then(|p| {
            let keys = [
                p.clone(),
                std::fs::canonicalize(&p)
                    .map(|c| c.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ];
            for k in keys {
                if k.is_empty() {
                    continue;
                }
                if let Some(bps) = app.dap.breakpoints.get(&k) {
                    // The line AND what kind of breakpoint it is. A hollow
                    // chip and a marked one are different answers, and the
                    // face cannot work them out from a set of line numbers.
                    return Some(
                        bps.iter()
                            .map(|b| (b.line, (b.enabled, b.condition.is_some() || b.log_message.is_some())))
                            .collect(),
                    );
                }
            }
            None
        })
    } else {
        None
    };
    // The row the program is stopped on — resolved once, exactly like the
    // breakpoint set above and for the same reason: this is the scroll hot
    // path and it must never canonicalize per row.
    //
    // Read immutably rather than through `DapClient::current_line_for`, whose
    // doc comment names this caller but which takes `&mut self` for its canon
    // cache. The band holds `&App`. Matching the two keys here is what the
    // breakpoint block already does one screen up.
    // All of the debugger's marks are gated on its panel being on screen —
    // the band, the frame, the bracket. A reader who closed the panel is not
    // debugging, and a stop band left behind is the editor still talking about
    // a session nobody is watching.
    let debugging = app.dap.panel_open;
    let pane_path: Option<String> = if is_current {
        app.tabs
            .buffers
            .get(tab)
            .and_then(|t| t.filename.as_ref())
            .map(|p| p.to_string_lossy().to_string())
    } else {
        None
    };
    let same_file = |other: &str| -> bool {
        let Some(ref mine) = pane_path else { return false };
        mine == other
            || std::fs::canonicalize(mine).ok().map(|c| c.to_string_lossy().to_string())
                == std::fs::canonicalize(other).ok().map(|c| c.to_string_lossy().to_string())
    };
    let stopped_line: Option<usize> = app.dap.current_line.filter(|_| debugging).and_then(|line| {
        let cur = app.dap.current_path.as_deref()?;
        same_file(cur).then_some(line)
    });
    // The caret symbol's extent, resolved once per band. Rows between the
    // first and last occurrence carry the rule; the two ends cap it; writes
    // get a tick.
    //
    // An extent that runs the whole file is noise rather than information — a
    // `static` used in four hundred places would draw a rule down every screen
    // — so it is dropped past a bound. Better to say nothing than to say
    // "everywhere".
    const MAX_EXTENT_ROWS: usize = 400;
    // A SESSION, not just the panel. The bracket says where a value lives
    // while you step, so with nothing running there is nothing to step and a
    // rule drawn round every symbol the caret touches is noise. Two gates
    // rather than one because they fail differently: the panel can be closed
    // with a program still stopped, and a program can be gone with the panel
    // still up.
    let extent: Option<(usize, usize)> = if is_current
        && debugging
        && app.dap.is_session()
        && !app.lsp.highlights.is_empty()
    {
        let rows_hl = app.lsp.highlights.iter().map(|h| h.row);
        let lo = rows_hl.clone().min().unwrap_or(0);
        let hi = rows_hl.max().unwrap_or(0);
        (hi.saturating_sub(lo) <= MAX_EXTENT_ROWS).then_some((lo, hi))
    } else {
        None
    };
    // The frame being READ, marked only when it differs from the stop —
    // otherwise every stop would draw a hollow arrow under its own solid one.
    let frame_line: Option<usize> = app.dap.frame_location().and_then(|(p, line)| {
        (same_file(&p) && Some(line) != stopped_line).then_some(line)
    });
    // GUI multi-cursor: every caret in `app.sel` except the primary (the
    // primary is painted through caret_*/sel_*). Resolved once for the whole
    // band, not per row/chunk. Empty in the single-cursor case, so the hot
    // path pays nothing.
    let gui_secondaries: Vec<Position> = if is_current && app.sel.is_multi() {
        app.secondary_caret_positions()
    } else {
        Vec::new()
    };
    let diags_active = is_current && app.has_diagnostics();

    // Visual row origin for first buffer line in this window (approx: 1:1 before scroll).
    // Xcode-style bracket hint: moving across a closer points out its opener.
    // Kind 254 is a marker span; the FACE owns the ~1s flash, so the core stays
    // stateless and the timing lives with the renderer.
    //
    // Hoisted: this asks about the CARET, so it gives the same answer for every
    // row, and it used to be asked once per row of the band. 240 identical
    // whole-document scans per draw, on the main thread, before CoreText.
    let bracket_match = if caret_vcol.is_some() && use_live_syntax && is_current {
        app.buffer.matching_bracket_before_cursor()
    } else {
        None
    };

    let mut visual_row = band_start as u32;
    let mut lines = Vec::with_capacity(rows.saturating_mul(if wrap { 2 } else { 1 }));
    let mut buffer_rows_taken = 0usize;
    // Cap matches the FFI packed budget (SUISEI_MAX_LINES = 256) minus headroom;
    // must stay ≥ OVERSCAN_ABOVE + viewport rows or the visible bottom goes blank.
    while buffer_rows_taken < rows && lines.len() < 240 {
        let row = band_start + buffer_rows_taken;
        if row >= total {
            break;
        }
        buffer_rows_taken += 1;
        let raw = buf.line(row);
        let mut text = suisei_core::wrap::expand_tabs(raw, app.tab_width);
        if text.len() > 480 {
            let mut cut = 480;
            while cut > 0 && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('…');
        }
        let is_cursor_row = row == cursor_row;
        let (sel_v0, sel_v1) = if use_live_syntax && is_current {
            selection_on_line(app, row, &text, sel)
        } else {
            (None, None)
        };
        let mut full_spans = if use_live_syntax && is_current {
            syntax_spans_for_row(app, row, raw)
        } else {
            let ext = app
                .tabs
                .buffers
                .get(tab)
                .and_then(|t| t.filename.as_ref())
                .and_then(|p| p.extension())
                .and_then(|e| e.to_str())
                .map(|s| s.to_string());
            suisei_core::highlight::highlight_line(raw, ext.as_deref())
                .into_iter()
                .take(32)
                .filter_map(|(kind, start, end)| {
                    let v0 = visual_col(raw, start, app.tab_width) as u32;
                    let v1 = visual_col(raw, end, app.tab_width) as u32;
                    (v1 > v0).then_some(SpanScene {
                        start: v0,
                        end: v1,
                        kind: kind_to_u8(kind),
                    })
                })
                .collect::<Vec<_>>()
        };
        // Find overlays are renderer spans too: 248 = another match, 249 =
        // the current match. They stay in display-column coordinates like
        // syntax spans; the AppKit face resolves those columns through the
        // drawn CoreText line so CJK/emoji widths remain exact.
        // The committed pattern stays available for Cmd-G / Shift-Cmd-G, but
        // the yellow find decorations belong to the transient find panel.
        // Once Done/Return closes that panel, the editor returns to normal
        // syntax paint instead of retaining a permanent field of yellow boxes.
        if is_current && matches!(app.mode, Mode::Search) {
            if let Some(pattern) = app.active_search_pattern() {
                let pattern_len = pattern.chars().count();
                if pattern_len > 0 {
                    let (base, matches) = app.search_matches_row_slice(row);
                    for (offset, m) in matches.iter().enumerate() {
                        let start = visual_col(raw, m.col, app.tab_width) as u32;
                        let end = visual_col(raw, m.col.saturating_add(pattern_len), app.tab_width) as u32;
                        if end > start {
                            full_spans.push(SpanScene {
                                start,
                                end,
                                kind: if base + offset == app.search.current {
                                    249
                                } else {
                                    248
                                },
                            });
                        }
                    }
                }
            }
        }
        let git_sign = if is_current {
            git_sign_for_row(app, row)
        } else {
            0
        };
        let chunks = if wrap {
            suisei_core::wrap::visual_chunks(&text, wrap_cols, wide_ratio)
        } else {
            vec![(0u32, text.clone())]
        };
        let caret_abs = if is_cursor_row {
            caret_vcol.unwrap_or(0)
        } else {
            0
        };
        let chunk_count = chunks.len();
        for (ci, (base_col, chunk)) in chunks.into_iter().enumerate() {
            let wrap_cont = ci > 0;
            let is_last_chunk = ci + 1 == chunk_count;
            let end_col = base_col + visual_width_str(&chunk) as u32;
            let mut spans: Vec<SpanScene> = full_spans
                .iter()
                .filter_map(|sp| {
                    let s = sp.start.max(base_col);
                    let e = sp.end.min(end_col);
                    (e > s).then_some(SpanScene {
                        start: s - base_col,
                        end: e - base_col,
                        kind: sp.kind,
                    })
                })
                .take(32)
                .collect();
            // A selection may legitimately reach ONE past the last character —
            // that column is the newline, and it is what a blank line inside a
            // selection consists of. `selection_on_line` already says so (its
            // `v1 <= v0` fallback returns 0..1 for an empty row); clamping to
            // `end_col` threw it away, so selecting a whole file left every
            // blank line looking unselected. Same allowance the caret gets
            // just below, and for the same reason.
            let sel_limit = if is_last_chunk {
                end_col + 1
            } else {
                end_col
            };
            let (sv0, sv1) = match (sel_v0, sel_v1) {
                (Some(a), Some(b)) => {
                    let s = a.max(base_col);
                    let e = b.min(sel_limit);
                    if e > s {
                        (Some(s - base_col), Some(e - base_col))
                    } else {
                        (None, None)
                    }
                }
                _ => (None, None),
            };
            // The caret legitimately sits ONE PAST the last character — which is
            // where it lives the whole time you are typing. `caret_abs < end_col`
            // dropped it there, so the caret (and anything drawn with it, e.g.
            // in-progress IME text) vanished mid-edit. Only the last chunk may
            // claim that extra column, otherwise a caret on a soft-wrap boundary
            // would be painted twice.
            let caret_limit = if is_last_chunk {
                end_col + 1
            } else {
                end_col.max(base_col + 1)
            };
            let caret_here = is_cursor_row && caret_abs >= base_col && caret_abs < caret_limit;
            // NOTE: kind 254 carries UTF-16 offsets, not visual columns like
            // every other span — the face positions it with CoreText, so a cell
            // column would drift on any line containing CJK. Skipped on
            // soft-wrap continuations, where chunk-relative offsets don't apply.
            if let Some(m) = bracket_match {
                if m.row == row && !wrap_cont {
                    let u0: u32 = raw.chars().take(m.col).map(|c| c.len_utf16() as u32).sum();
                    let u1 = u0
                        + raw
                            .chars()
                            .nth(m.col)
                            .map(|c| c.len_utf16() as u32)
                            .unwrap_or(1);
                    spans.push(SpanScene {
                        start: u0,
                        end: u1,
                        kind: 254,
                    });
                }
            }
            // Multi-cursor extras carry UTF-16 offsets — the face positions
            // them with CoreText so an extra caret tracks CJK the same way
            // the primary does. The head is an exclusive between-character
            // column, i.e. already the drawn column. (Secondary SELECTION
            // fills are a separate kind, to add with ⌘-D.)
            for head in &gui_secondaries {
                if head.row != row {
                    continue;
                }
                let vc = visual_col(raw, head.col, app.tab_width) as u32;
                if vc >= base_col && vc < caret_limit {
                    let u = utf16_offset_for_vcol(&chunk, vc.saturating_sub(base_col));
                    spans.push(SpanScene {
                        start: u,
                        end: u.saturating_add(1),
                        kind: 250,
                    });
                }
            }
            if diags_active {
                for d in app.diagnostics_for_row(row) {
                    let v0 = visual_col(raw, d.col_start, app.tab_width) as u32;
                    let v1 = visual_col(raw, d.col_end.max(d.col_start.saturating_add(1)), app.tab_width) as u32;
                    let s = v0.max(base_col);
                    let e = v1.min(end_col.max(base_col + 1));
                    if e > s {
                        let kind = match d.severity {
                            suisei_core::lsp::DiagnosticSeverity::Error => 251,
                            suisei_core::lsp::DiagnosticSeverity::Warning => 252,
                            _ => 253,
                        };
                        spans.push(SpanScene {
                            start: s - base_col,
                            end: e - base_col,
                            kind,
                        });
                    }
                }
            }
            let mut gsign = if wrap_cont { git_sign | 0x80 } else { git_sign };
            let mut dsign = 0u8;
            if !wrap_cont {
                if let Some(ref set) = bp_lines {
                    if let Some(&(enabled, decorated)) = set.get(&row) {
                        gsign |= 0x40;
                        if !enabled {
                            dsign |= BREAKPOINT_DISABLED;
                        }
                        if decorated {
                            dsign |= BREAKPOINT_DECORATED;
                        }
                    }
                }
            }
            if !wrap_cont {
                if stopped_line == Some(row) {
                    dsign |= DEBUG_STOPPED;
                }
                if frame_line == Some(row) {
                    dsign |= DEBUG_FRAME;
                }
                if let Some((lo, hi)) = extent {
                    if row >= lo && row <= hi {
                        dsign |= VALUE_EXTENT;
                        if row == lo {
                            dsign |= VALUE_FIRST;
                        }
                        if row == hi {
                            dsign |= VALUE_LAST;
                        }
                        if app.lsp.highlights.iter().any(|h| h.row == row && h.write) {
                            dsign |= VALUE_WRITE;
                        }
                    }
                }
            }
            // Cap at the FFI limit (SUISEI_MAX_SPANS = 24), markers first:
            // kinds >= 248 are find / caret / diagnostics / bracket overlays —
            // a plain truncate let busy syntax push the caret span off the
            // line, so syntax spans yield before markers do.
            if spans.len() > 24 {
                let markers: Vec<SpanScene> =
                    spans.iter().filter(|s| s.kind >= 248).copied().collect();
                let keep = 24usize.saturating_sub(markers.len());
                let syntax: Vec<SpanScene> = spans
                    .into_iter()
                    .filter(|s| s.kind < 248)
                    .take(keep)
                    .collect();
                spans = syntax.into_iter().chain(markers).collect();
            }
            let caret_utf16 = if caret_here {
                utf16_offset_for_vcol(&chunk, caret_abs.saturating_sub(base_col))
            } else {
                0
            };
            let sel_u0 = sv0.map(|v| utf16_offset_for_vcol(&chunk, v)).unwrap_or(0);
            let sel_u1 = sv1.map(|v| utf16_offset_for_vcol(&chunk, v)).unwrap_or(0);
            lines.push(EditorLineScene {
                line_no: (row + 1) as u32,
                text: chunk,
                is_cursor: caret_here,
                caret_vcol: if caret_here {
                    caret_abs.saturating_sub(base_col)
                } else {
                    0
                },
                caret_utf16,
                sel_u0,
                sel_u1,
                sel_v0: sv0,
                sel_v1: sv1,
                spans,
                git_sign: gsign,
                debug_sign: dsign,
                visual_row,
            });
            visual_row = visual_row.saturating_add(1);
        }
    }
    lines
}


/// UTF-16 offset of the character that starts at terminal cell column `vcol`.
/// Bridges the core's cell grid to the renderer's glyph advances.
fn utf16_offset_for_vcol(s: &str, vcol: u32) -> u32 {
    let mut cells = 0usize;
    let mut utf16 = 0usize;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        // Once the requested cell boundary is reached, still absorb trailing
        // zero-width scalars belonging to the same drawn grapheme. Otherwise
        // CoreText receives an index inside decomposed Hangul/accent clusters.
        if cells >= vcol as usize && w > 0 {
            break;
        }
        cells += w;
        utf16 += ch.len_utf16();
    }
    utf16 as u32
}

fn visual_width_str(s: &str) -> usize {
    s.chars()
        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1))
        .sum()
}

/// Every bit of `LineScene::git_sign`, in one place.
///
/// The byte has four owners and had no map, which cost exactly what that
/// usually costs: the staged flag was put on `0x40`, which is the BREAKPOINT
/// bit, so staging a hunk drew a real breakpoint on every line of it — and the
/// face's `gitSignKind` masks with `0x3F`, so the same flag never arrived and
/// the bar never filled. One collision, both symptoms.
///
/// ```text
///   0x03  kind: 0 none, 1 added, 2 modified, 3 deleted (the face's
///         `gitSignKind` masks with this)
///   0x04  (free — the debugger's marks moved to `debug_sign`)
///   0x08  the hunk is staged        → bar is filled rather than hollow
///   0x10  first row of its hunk     → the bar caps here
///   0x20  last row of its hunk      → the bar caps here
///   0x40  breakpoint on this line   (owned by the DAP path)
///   0x80  soft-wrap continuation    (owned by the line builder)
/// ```
/// The program is stopped on this row — the top of the call stack.
///
/// In `debug_sign` and no longer in the git byte. It started there because the
/// breakpoint bit is there, and that was one bit's worth of thinking: the
/// moment the SELECTED frame needed its own mark, the git byte had none spare
/// and the debugger's two facts would have lived in two different fields.
/// `debug_sign` costs nothing — it is the line struct's reserved `_pad`.
pub const DEBUG_STOPPED: u8 = 0x01;
/// The frame the user has SELECTED in the call stack, when it is not the stop.
///
/// Drawn hollow. Solid means "execution is here"; hollow means "you are reading
/// this one" — and conflating them is what let clicking a caller convince the
/// editor that the program had moved.
pub const DEBUG_FRAME: u8 = 0x02;
/// This row is inside the caret symbol's extent — where the value lives.
pub const VALUE_EXTENT: u8 = 0x04;
/// First and last row of that extent. The rule caps here, the same way the git
/// change bar caps at its hunk's ends — that mark is already read as "this
/// run, together", and a second vocabulary for the same idea would be one more
/// thing to learn.
pub const VALUE_FIRST: u8 = 0x08;
pub const VALUE_LAST: u8 = 0x10;
/// The value MOVES on this row: `documentHighlight` called it a write.
///
/// The distinction the feature rests on. A read is where a value is used; a
/// write is where it changes, and only the write kind answers "how does this
/// value move".
pub const VALUE_WRITE: u8 = 0x20;
/// The breakpoint on this row is not armed. Drawn hollow — it is still THERE,
/// with its place, its condition and its log message; it is switched off.
pub const BREAKPOINT_DISABLED: u8 = 0x40;
/// The breakpoint on this row carries a condition or a log message.
///
/// Both go in one bit because the chip says the same thing about them: this
/// one is not the plain kind, look at it. Which of the two it is, is the
/// menu's business and the panel's.
pub const BREAKPOINT_DECORATED: u8 = 0x80;
pub const GIT_HUNK_STAGED: u8 = 0x08;
pub const GIT_HUNK_FIRST: u8 = 0x10;
pub const GIT_HUNK_LAST: u8 = 0x20;

fn git_sign_for_row(app: &App, row: usize) -> u8 {
    use suisei_core::git::GitSign;
    let kind = match app.git.sign_at(row) {
        Some(GitSign::Added) => 1,
        Some(GitSign::Modified) => 2,
        Some(GitSign::Deleted) => 3,
        None => return 0,
    };
    let Some(h) = app.git.hunk_at(row) else {
        // A sign with no hunk should not happen — signs are derived from them
        // — but a lone slice is a better failure than a missing one.
        return kind | GIT_HUNK_FIRST | GIT_HUNK_LAST;
    };
    let mut out = kind;
    if row == h.start {
        out |= GIT_HUNK_FIRST;
    }
    if row == h.end() {
        out |= GIT_HUNK_LAST;
    }
    if h.staged {
        out |= GIT_HUNK_STAGED;
    }
    out
}

/// Convert highlight tokens → visual-column spans for the face.
/// Prefers tree-sitter; falls back to Core `highlight_line` (md/swift/etc.).
fn syntax_spans_for_row(app: &App, row: usize, raw: &str) -> Vec<SpanScene> {
    let ts = app.syntax.tokens_for_row(row);
    let ext = app.file_extension();
    let spans_src: Vec<(suisei_core::highlight::TokenKind, usize, usize)> = if !ts.is_empty() {
        ts.iter()
            .map(|(k, s, e, _)| (*k, *s, *e))
            .take(32)
            .collect()
    } else {
        // Fallback tokenizer — covers markdown, swift, and langs without a ts grammar.
        suisei_core::highlight::highlight_line(raw, ext.as_deref())
            .into_iter()
            .take(32)
            .collect()
    };
    let mut out = Vec::with_capacity(spans_src.len());
    for (kind, start, end) in spans_src {
        let v0 = visual_col(raw, start, app.tab_width) as u32;
        let v1 = visual_col(raw, end, app.tab_width) as u32;
        if v1 > v0 {
            out.push(SpanScene {
                start: v0,
                end: v1,
                kind: kind_to_u8(kind),
            });
        }
    }
    out
}

fn kind_to_u8(kind: suisei_core::highlight::TokenKind) -> u8 {
    use suisei_core::highlight::TokenKind::*;
    match kind {
        Keyword | KeywordControl | KeywordImport => 1,
        String => 2,
        Comment => 3,
        Number => 4,
        TypeName => 5,
        Function | Method => 6,
        Macro | Attribute => 7,
        Namespace => 8,
        Parameter | Lifetime => 9,
        Property => 10,
        Constant => 11,
        Variable => 12,
        Operator => 13,
        Punctuation => 14,
    }
}

/// Visual selection span for one buffer row (tab-expanded coordinates).
///
/// Core `selected_range` is **inclusive** on both ends (yank uses `end.col + 1`
/// as exclusive). Paint uses exclusive `sel_v1` so the last glyph is included:
/// `sel_v1 = visual_col(end.col + 1)`.
/// Where the caret is DRAWN.
///
/// The core keeps vim's inclusive visual selection — the cursor sits ON the
/// last selected character — but a GUI puts the caret at the selection's outer
/// edge. Double-clicking a word therefore looked like it left the caret one
/// place short of the end. Shift the drawn caret past the character when the
/// cursor is the far end of a selection; the selection itself is untouched.
pub(crate) fn drawn_caret_col(app: &App) -> usize {
    let c = app.buffer.cursor();
    if let Some((start, end)) = app.selected_range() {
        if c.row == end.row && c.col == end.col && (end.row, end.col) >= (start.row, start.col) {
            let len = app.buffer.line(c.row).chars().count();
            return (c.col + 1).min(len);
        }
    }
    c.col
}

pub(crate) fn selection_on_line(
    app: &App,
    row: usize,
    expanded: &str,
    sel: Option<(Position, Position)>,
) -> (Option<u32>, Option<u32>) {
    let Some((start, end)) = sel else {
        return (None, None);
    };
    if row < start.row || row > end.row {
        return (None, None);
    }
    let raw = app.buffer.line(row);
    // Selection coordinates below are DISPLAY columns. Counting Unicode
    // scalars here clamps a CJK selection to half its real width (한 == two
    // display cells), which in turn produces the wrong UTF-16 range for
    // CoreText. Clamp against display width, then convert to UTF-16 once.
    let line_len = visual_width_str(expanded) as u32;
    let v0 = if row == start.row {
        visual_col(raw, start.col, app.tab_width) as u32
    } else {
        0
    };
    // Exclusive visual end: one past the inclusive buffer end column.
    let v1 = if row == end.row {
        visual_col(raw, end.col.saturating_add(1), app.tab_width) as u32
    } else {
        line_len
    };
    // Single-cell / zero-width: still paint one cell when start==end on a char.
    let v1 = if v1 <= v0 {
        (v0 + 1).min(line_len.saturating_add(1))
    } else {
        v1
    };
    (Some(v0), Some(v1.min(line_len.saturating_add(1))))
}

// `expand_tabs` lived here with the tab width hardcoded to 4 and
// `wrap_visual_chunks` beside it. Both are `suisei_core::wrap`'s now: the wrap
// map counts the rows a line takes and this builder produces their contents,
// and those two answers have to come from one rule or the document is a
// different height than it draws.

/// Display column a buffer column sits at, on a raw (unexpanded) line.
///
/// `tab_width` rather than a hardcoded 4: this and the expansion have to place
/// a tab at the same stop, or a syntax span on a tabbed line is highlighted
/// beside the text it describes.
pub(crate) fn visual_col(line: &str, buf_col: usize, tab_width: usize) -> usize {
    let tab = tab_width.max(1);
    let mut col = 0usize;
    for (i, ch) in line.chars().enumerate() {
        if i >= buf_col {
            break;
        }
        if ch == '\t' {
            col += tab - (col % tab);
        } else {
            col += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        }
    }
    col
}

/// Which surface owns the keyboard, as a stable name the face switches on.
///
/// This used to be the vim status badge (` NORMAL `, ` INSERT `, ` PENDING `…)
/// and the Swift side made focus decisions by substring-matching it. It is now
/// simply the focus, so the face can parse it once into a typed value.
fn mode_label(app: &App) -> &'static str {
    match app.mode {
        Mode::Editor => "EDITOR",
        Mode::Explorer => "EXPLORER",
        Mode::Terminal => "TERMINAL",
        Mode::Search => "SEARCH",
        Mode::Palette => "PALETTE",
        Mode::SourceControl => "SCM",
        Mode::GitWorkbench => "GIT",
        Mode::Settings => "SETTINGS",
        Mode::Preview => "PREVIEW",
        Mode::WorkspaceSearch => "FIND",
        Mode::Debug => "DEBUG",
        Mode::CallHierarchy => "CALLS",
    }
}

#[cfg(test)]
mod unicode_overlay_tests {
    use super::*;
    use suisei_core::buffer::Buffer;

    #[test]
    fn cjk_selection_uses_display_width_before_utf16_conversion() {
        let mut app = App::new();
        app.buffer = Buffer::from_string("a한글b");
        let span = Some((Position::new(0, 1), Position::new(0, 2)));
        assert_eq!(
            selection_on_line(&app, 0, "a한글b", span),
            (Some(1), Some(5))
        );
    }

    #[test]
    fn find_results_emit_yellow_overlay_spans_and_current_marker() {
        let mut app = App::new();
        app.buffer = Buffer::from_string("한글 한글");
        app.mode = Mode::Search;
        app.search.input = "한글".into();
        app.recompute_search("한글", false);
        app.search.current = 1;
        let lines = build_lines_at(&app, 0, 0, 1, Some(0), None, true, 0, 200);
        let kinds: Vec<u8> = lines[0]
            .spans
            .iter()
            .filter(|span| matches!(span.kind, 248 | 249))
            .map(|span| span.kind)
            .collect();
        assert_eq!(kinds, vec![248, 249]);
    }

    #[test]
    fn accepted_find_keeps_navigation_pattern_without_persistent_overlays() {
        let mut app = App::new();
        app.buffer = Buffer::from_string("suisei suisei");
        app.enter_search();
        app.set_search_input("suisei".into());
        app.commit_search();

        assert_eq!(app.mode, Mode::Editor);
        assert_eq!(app.search.pattern.as_deref(), Some("suisei"));
        assert_eq!(app.search.matches.len(), 2);

        let lines = build_lines_at(&app, 0, 0, 1, Some(0), None, true, 0, 200);
        assert!(
            lines[0]
                .spans
                .iter()
                .all(|span| !matches!(span.kind, 248 | 249)),
            "find decorations must disappear when the find panel closes"
        );
    }

    #[test]
    fn display_to_utf16_mapping_keeps_combining_cluster_whole() {
        assert_eq!(visual_width_str("e\u{301}x"), 2);
        assert_eq!(utf16_offset_for_vcol("e\u{301}x", 1), 2);
    }
}
