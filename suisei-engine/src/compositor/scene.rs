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
    pub editor_rows: u32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            css_w: 1200.0,
            css_h: 800.0,
            cell_px: 18.0,
            cell_w: 9.0,
            dpr: 2.0,
            editor_rows: 40,
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
    pub title: String,
    pub dirty: bool,
    pub active: bool,
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
    pub open: bool,
    pub full_panel: bool,
    /// Split pane index showing the full terminal, or `None` = whole main area.
    pub pane_bound: Option<u32>,
    pub lines: Vec<String>,
}

/// Packed RGB for Swift face (0x00RRGGBB).
#[derive(Debug, Clone)]
pub struct ThemeScene {
    pub name: String,
    pub editor_bg: u32,
    pub fg: u32,
    pub dim: u32,
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
}

#[derive(Debug, Clone)]
pub struct SettingsRowScene {
    pub label: String,
    pub value: String,
    pub is_header: bool,
    pub selected: bool,
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
    pub buffer_version: u64,
    pub branch: String,
    pub tabs: Vec<TabScene>,
    /// Focused-pane (or single) lines for backwards-compatible consumers.
    pub lines: Vec<EditorLineScene>,
    /// 0 none, 1 vertical, 2 horizontal
    pub split_kind: u8,
    pub split_ratio: f32,
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
) -> (Vec<EditorLineScene>, u32) {
    // Resolve pane → tab + focus (mirrors build_editor_surfaces).
    let (tab, focused) = if !app.split.is_split() {
        (app.current_buffer, true)
    } else {
        let n = app.split.pane_count().max(1);
        let idx = pane.min(n.saturating_sub(1));
        let focus = app.split.focus.min(n.saturating_sub(1));
        if idx == focus {
            (app.current_buffer, true)
        } else {
            (
                app.split.panes.get(idx).map(|p| p.tab_index).unwrap_or(0),
                false,
            )
        }
    };
    let buf = buffer_for_tab(app, tab);
    let total = buf.line_count() as u32;
    let caret_vcol = if focused {
        let c = app.buffer.cursor();
        visual_col(app.buffer.line(c.row), drawn_caret_col(app)) as u32
    } else {
        0
    };
    let sel = if focused { app.selected_range() } else { None };
    let lines = build_lines_at(app, tab, start, rows, Some(caret_vcol), sel, focused);
    (lines, total)
}

/// Scroll-hot path: rebuild **editor surfaces only**, keep explorer/SCM/outline/theme.
/// Avoids re-walking the project tree and git workbench on every trackpad tick.
pub fn patch_chrome_editor_scroll(
    app: &App,
    shell: &ShellState,
    frame_gen: u64,
    chrome: &mut ChromeScene,
) {
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
        || app
            .buffers
            .iter()
            .any(|t| t.filename.as_ref().is_some_and(|p| !p.as_os_str().is_empty()));
    let multi_tab = app.buffers.len() > 1;
    let project_seeded = !app.explorer.entries.is_empty();
    let welcome = !multi_tab
        && !any_named_tab
        && !project_seeded
        && !app.modified
        && app.buffer.line_count() == 1
        && app.buffer.line(0).is_empty();
    // Large overscan so AppKit Responsive Scrolling overdraw stays filled (WWDC 2013-215).
    // Cap to packed-line budget (SUISEI_MAX_LINES ≈ 256).
    let rows = (shell.viewport.editor_rows.max(8) as usize)
        .saturating_mul(3)
        .saturating_add(48)
        .min(240);
    let caret_vcol = if welcome {
        0
    } else {
        visual_col(app.buffer.line(cursor.row), drawn_caret_col(app)) as u32
    };
    let sel = app.selected_range();
    // Patch path may reuse unfocused pane snapshots (typing hot path in splits).
    let prev_panes = std::mem::take(&mut chrome.panes);
    let (lines, split_kind, split_ratio, pane_focus, panes) = if welcome {
        (Vec::new(), 0u8, 0.5f32, 0u8, Vec::new())
    } else {
        build_editor_surfaces(app, scroll, rows, caret_vcol, sel, prev_panes)
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
    chrome.buffer_version = app.buffer.version();
    chrome.lines = lines;
    chrome.split_kind = split_kind;
    chrome.split_ratio = split_ratio;
    chrome.pane_focus = pane_focus;
    chrome.panes = panes;
    // Terminal PTY can change while scrolling; keep it live. Skip explorer/scm/git/outline/theme.
    if chrome.terminal.open || matches!(app.mode, Mode::Terminal) {
        chrome.terminal = build_terminal(app);
    }
    // Leader (Space) sets no pending_key — gate on visibility itself, and also
    // rebuild when the scene still shows an open popup so it can close.
    if app.completions.active
        || !app.completions.suggestions.is_empty()
        || chrome.completions.open
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

pub fn compose(
    app: &App,
    shell: &ShellState,
    frame_gen: u64,
    outline: &[OutlineItemScene],
) -> FrameDiff {
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
        || app
            .buffers
            .iter()
            .any(|t| t.filename.as_ref().is_some_and(|p| !p.as_os_str().is_empty()));
    let multi_tab = app.buffers.len() > 1;
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
    let rows = (shell.viewport.editor_rows.max(8) as usize)
        .saturating_mul(3)
        .saturating_add(48)
        .min(240);
    let caret_vcol = if welcome {
        0
    } else {
        visual_col(app.buffer.line(cursor.row), drawn_caret_col(app)) as u32
    };
    let sel = app.selected_range();
    let (lines, split_kind, split_ratio, pane_focus, panes) = if welcome {
        (Vec::new(), 0u8, 0.5f32, 0u8, Vec::new())
    } else {
        build_editor_surfaces(app, scroll, rows, caret_vcol, sel, Vec::new())
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
            buffer_version: app.buffer.version(),
            branch: branch_name(app),
            tabs,
            lines,
            split_kind,
            split_ratio,
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
        1 => 0,  // H1
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
        // Rust / Swift / general code symbols
        if matches!(ext.as_str(), "rs" | "swift" | "go" | "ts" | "tsx" | "js" | "jsx" | "py" | "c" | "h" | "cpp" | "hpp" | "java" | "kt" | "m" | "mm" | "")
            || ext.is_empty()
        {
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
            .unwrap_or_else(|| {
                "Space stage · c commit · Enter open · Tab pane · Esc".into()
            })
    };

    // ── Changes column ──
    let mut col_changes = Vec::new();
    let n_ch = app.git_wb.changes.len() + app.git_wb.staged.len();
    col_changes.push(format!("▾ Changes  {n_ch}"));
    if !app.git_wb.staged.is_empty() {
        col_changes.push(format!("  Staged ({})", app.git_wb.staged.len()));
        for e in app.git_wb.staged.iter().take(40) {
            col_changes.push(format!("  {} {}", e.status.letter(), e.path));
        }
    }
    col_changes.push(format!("  Local Changes ({})", app.git_wb.changes.len()));
    if app.git_wb.changes.is_empty() && app.git_wb.staged.is_empty() {
        col_changes.push("  (clean)".into());
    } else {
        for e in app.git_wb.changes.iter().take(50) {
            col_changes.push(format!("  {} {}", e.status.letter(), e.path));
        }
    }
    if !app.git_wb.commit_buf.is_empty() || app.git_wb.commit_editing {
        col_changes.push("── Commit ──".into());
        col_changes.push(format!("  {}", app.git_wb.commit_buf));
    }

    // ── Log column ──
    let mut col_log = Vec::new();
    if matches!(app.git_wb.history_view, HistoryView::Graph)
        && !app.git_wb.history_graph.is_empty()
    {
        col_log.push("▾ Log · graph  (v list)".into());
        for (i, row) in app.git_wb.history_graph.iter().enumerate().take(60) {
            let mark = if i == app.git_wb.history_sel { "›" } else { " " };
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
            let mark = if i == app.git_wb.history_sel { "›" } else { " " };
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
        col_files.push(format!("  {}  {}", d.short, d.subject.chars().take(40).collect::<String>()));
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
    match app.git_wb.tab {
        GitTab::Branches => {
            for (i, b) in app.git_wb.branches.iter().enumerate().take(50) {
                let mark = if i == app.git_wb.branch_sel { "›" } else { " " };
                let cur = if b.current { "*" } else { " " };
                special.push(format!("{mark}{cur} {}", b.name));
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
            for (i, p) in app.git_wb.prs.iter().enumerate().take(40) {
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
            for (i, iss) in app.git_wb.issues.iter().enumerate().take(40) {
                let mark = if i == app.git_wb.issue_sel { "›" } else { " " };
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
                let mark = if i == app.git_wb.stash_sel { "›" } else { " " };
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
                subject: row.subject.chars().take(56).collect(),
                when: row.when.clone(),
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
    }
}

fn build_settings(app: &App) -> SettingsScene {
    use suisei_core::settings::{help_entries, SettingRow, SettingsPage};
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

    let tabs: Vec<String> = SettingsPage::all().iter().map(|p| p.label().into()).collect();
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
            });
            rows.push(SettingsRowScene {
                label: "Version".into(),
                value: suisei_core::settings::SettingsPanel::version_string(),
                is_header: false,
                selected: false,
            });
            rows.push(SettingsRowScene {
                label: "Core".into(),
                value: "xei-core".into(),
                is_header: false,
                selected: false,
            });
            rows.push(SettingsRowScene {
                label: "Theme".into(),
                value: app.theme.name.into(),
                is_header: false,
                selected: false,
            });
            rows.push(SettingsRowScene {
                label: "Config".into(),
                value: "~/.xei.toml".into(),
                is_header: false,
                selected: false,
            });
        }
        // Drop TUI key-chord junk from status when painting for the face.

        SettingsPage::Setting => {
            let draft = &app.settings.draft;
            let themes = theme::all_themes();
            for (i, row) in app.settings.setting_rows().into_iter().enumerate() {
                let selected = i == app.settings.selected;
                let (label, value, is_header) = match row {
                    SettingRow::ThemeHeader => ("Theme".into(), String::new(), true),
                    SettingRow::Theme(ti) => {
                        let name = themes.get(ti).map(|t| t.name).unwrap_or("?");
                        let mark = if draft.theme.eq_ignore_ascii_case(name) {
                            "●"
                        } else {
                            " "
                        };
                        (format!("{mark} {name}"), "Enter to apply".into(), false)
                    }
                    SettingRow::EditorHeader => ("Editor".into(), String::new(), true),
                    SettingRow::TabWidth => ("Tab width".into(), format!("{}", draft.tab_width), false),
                    SettingRow::RelativeNumber => {
                        ("Relative number".into(), on_off(draft.relative_number), false)
                    }
                    SettingRow::WrapLines => ("Wrap lines".into(), on_off(draft.wrap_lines), false),
                    SettingRow::UndoCaching => {
                        ("Undo caching".into(), on_off(draft.undo_caching), false)
                    }
                    SettingRow::ClipboardSync => {
                        ("Clipboard sync".into(), on_off(draft.clipboard_sync), false)
                    }
                    SettingRow::GpuAcc => ("GPU accel (TUI)".into(), on_off(draft.gpu_acc), false),
                    SettingRow::GpuGraphics => {
                        ("GPU graphics".into(), on_off(draft.gpu_graphics), false)
                    }
                    SettingRow::GpuHyperlinks => {
                        ("GPU hyperlinks".into(), on_off(draft.gpu_hyperlinks), false)
                    }
                    SettingRow::KeyHints => ("Key hints".into(), on_off(draft.key_hints), false),
                    SettingRow::LspHeader => ("LSP".into(), String::new(), true),
                    SettingRow::LspEnabled => {
                        ("LSP enabled".into(), on_off(draft.lsp_enabled), false)
                    }
                    SettingRow::LspLang(li) => {
                        let catalog = suisei_core::config::lsp_lang_catalog();
                        let (key, label, _) = catalog
                            .get(li)
                            .copied()
                            .unwrap_or(("?", "?", "?"));
                        let state = match draft.lsp_servers.get(key).map(|s| s.as_str()) {
                            None => "default",
                            Some("") => "off",
                            Some(_) => "custom",
                        };
                        (format!("LSP · {label}"), state.into(), false)
                    }
                    SettingRow::GitHeader => ("Git".into(), String::new(), true),
                    SettingRow::OpenWorkbench => {
                        ("Open Git workbench".into(), "Enter".into(), false)
                    }
                    SettingRow::OpenScm => ("Open SCM panel".into(), "Enter".into(), false),
                    _ => continue,
                };
                rows.push(SettingsRowScene {
                    label,
                    value,
                    is_header,
                    selected,
                });
            }
        }
        SettingsPage::Pet => {
            let draft = &app.settings.draft;
            for (i, row) in app.settings.pet_rows().into_iter().enumerate() {
                let selected = i == app.settings.selected;
                let (label, value) = match row {
                    SettingRow::PetEnabled => ("Enabled".into(), on_off(draft.pet_enabled)),
                    SettingRow::PetPath => (
                        "Path".into(),
                        if draft.pet_path.is_empty() {
                            "(none · :pet file.gif)".into()
                        } else {
                            draft.pet_path.clone()
                        },
                    ),
                    SettingRow::PetX => ("X".into(), format!("{}", draft.pet_x)),
                    SettingRow::PetY => ("Y".into(), format!("{}", draft.pet_y)),
                    SettingRow::PetWidth => {
                        ("Width cells".into(), format!("{}", draft.pet_width_cells))
                    }
                    SettingRow::PetSpeed => ("Speed %".into(), format!("{}", draft.pet_speed)),
                    SettingRow::PetReload => ("Reload pet".into(), "Enter".into()),
                    _ => continue,
                };
                rows.push(SettingsRowScene {
                    label,
                    value,
                    is_header: false,
                    selected,
                });
            }
        }
        SettingsPage::Extensions => {
            rows.push(SettingsRowScene {
                label: "Extensions".into(),
                value: "Enter → plugin store".into(),
                is_header: false,
                selected: true,
            });
            rows.push(SettingsRowScene {
                label: "VS Code compat host".into(),
                value: "in progress".into(),
                is_header: false,
                selected: false,
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
    if v {
        "on".into()
    } else {
        "off".into()
    }
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
    if !app.terminal.open && !matches!(app.mode, Mode::Terminal) {
        return TerminalScene {
            open: false,
            full_panel: false,
            pane_bound: None,
            lines: Vec::new(),
        };
    }
    // Truecolor SGR lines from Core PTY cells (see Terminal::visible_rows_sgr).
    let max_rows = if app.terminal.full_panel { 120 } else { 48 };
    let mut lines: Vec<String> = app
        .terminal
        .visible_rows_sgr()
        .into_iter()
        .take(max_rows)
        .collect();
    if lines.is_empty() {
        lines.push(" ".into());
    }
    let pane_bound = if app.terminal.full_panel {
        app.terminal
            .pane_bound
            .map(|i| i as u32)
            .or(Some(app.split.focus.min(app.split.panes.len().saturating_sub(1)) as u32))
    } else {
        None
    };
    TerminalScene {
        open: true,
        full_panel: app.terminal.full_panel,
        pane_bound,
        lines,
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
        forward: app.search_forward,
        input: if open {
            app.search_input.clone()
        } else {
            String::new()
        },
        match_count: app.search_matches.len() as u32,
        match_index: if open && !app.search_matches.is_empty() {
            app.search_current as u32
        } else {
            0
        },
    }
}

fn build_tabs(app: &App) -> Vec<TabScene> {
    let mut out = Vec::with_capacity(app.buffers.len().max(1));
    for (i, tab) in app.buffers.iter().enumerate() {
        let is_current = i == app.current_buffer;
        let filename = if is_current {
            app.filename.as_ref()
        } else {
            tab.filename.as_ref()
        };
        let title = filename
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("[No Name]")
            .to_string();
        let dirty = if is_current {
            app.modified
        } else {
            tab.modified
        };
        out.push(TabScene {
            title,
            dirty,
            active: is_current,
        });
    }
    if out.is_empty() {
        out.push(TabScene {
            title: "[No Name]".into(),
            dirty: app.modified,
            active: true,
        });
    }
    out
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
) -> (Vec<EditorLineScene>, u8, f32, u8, Vec<PaneScene>) {
    use suisei_core::split::SplitKind;

    if !app.split.is_split() {
        let lines = build_visible_lines_from_buffer(
            app,
            app.current_buffer,
            scroll,
            rows,
            Some(caret_vcol),
            sel,
            true,
        );
        return (lines, 0, 0.5, 0, Vec::new());
    }

    let kind = match app.split.kind {
        SplitKind::Vertical => 1u8,
        SplitKind::Horizontal => 2u8,
        SplitKind::None => 0u8,
    };
    let n = app.split.pane_count().max(1);
    // Side-by-side (Vertical): each pane keeps full height — do NOT divide rows by n
    // (that was the "only half the editor paints" bug).
    // Stacked (Horizontal): share height across panes.
    // Reserve ~2 rows for per-pane path bar chrome in the face.
    const PATH_BAR_ROWS: usize = 2;
    const MAX_PACKED_LINES: usize = 256;
    let rows_each = match app.split.kind {
        SplitKind::Vertical | SplitKind::None => rows.saturating_sub(PATH_BAR_ROWS).max(8),
        SplitKind::Horizontal => {
            let share = (rows / n).max(4);
            share.saturating_sub(PATH_BAR_ROWS).max(4)
        }
    };
    // Cap so n panes fit the packed lines[] ABI budget.
    let rows_each = rows_each.min(MAX_PACKED_LINES / n).max(4);
    let focus = app.split.focus.min(n.saturating_sub(1));
    let mut panes = Vec::with_capacity(n);
    let mut focused_lines = Vec::new();

    // Persist active buffer into focused pane snapshot for accurate paint.
    let mut prev_panes = prev_panes;
    for (i, pane) in app.split.panes.iter().take(n).enumerate() {
        let focused = i == focus;
        let (tab, pane_scroll, pane_hscroll, pane_cursor) = if focused {
            (
                app.current_buffer,
                scroll,
                app.hscroll,
                (app.buffer.cursor.row, app.buffer.cursor.col),
            )
        } else {
            (pane.tab_index, pane.scroll, pane.hscroll, pane.cursor)
        };
        let buf = buffer_for_tab(app, tab);
        let doc_line_count = buf.line_count() as u32;
        let doc_version = buf.version();
        let eff_hscroll = if app.wrap_lines { 0 } else { pane_hscroll as u32 };
        // Unfocused + identical inputs → move the previous snapshot over.
        if !focused {
            if let Some(idx) = prev_panes.iter().position(|p| {
                !p.focused
                    && p.tab_index == tab as u32
                    && p.scroll == pane_scroll as u32
                    && p.hscroll == eff_hscroll
                    && p.doc_version == doc_version
                    && p.band_rows == rows_each as u32
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
            Some(visual_col(line, pane_cursor.1) as u32)
        };
        let pane_sel = if focused { sel } else { None };
        let lines = build_visible_lines_from_buffer(
            app,
            tab,
            pane_scroll,
            rows_each,
            caret,
            pane_sel,
            focused,
        );
        if focused {
            focused_lines = lines.clone();
        }
        panes.push(PaneScene {
            tab_index: tab as u32,
            doc_line_count,
            hscroll: eff_hscroll,
            scroll: pane_scroll as u32,
            focused,
            lines,
            doc_version,
            band_rows: rows_each as u32,
        });
    }

    (
        focused_lines,
        kind,
        app.split.ratio.clamp(0.2, 0.8),
        focus as u8,
        panes,
    )
}

fn buffer_for_tab(app: &App, tab: usize) -> &suisei_core::buffer::Buffer {
    if tab == app.current_buffer {
        &app.buffer
    } else if let Some(t) = app.buffers.get(tab) {
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
    )
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
) -> Vec<EditorLineScene> {
    let buf = buffer_for_tab(app, tab);
    let total = buf.line_count();
    let cursor_row = if tab == app.current_buffer {
        app.buffer.cursor().row
    } else if let Some(p) = app
        .split
        .panes
        .iter()
        .find(|p| p.tab_index == tab)
    {
        p.cursor.0
    } else {
        usize::MAX
    };
    let wrap = app.wrap_lines;
    let text_width = if wrap {
        app.viewport.width.saturating_sub(5).max(20) as usize
    } else {
        usize::MAX
    };
    // Resolve breakpoint path **once** — never canonicalize per row on the scroll hot path.
    let bp_lines: Option<std::collections::HashSet<usize>> = if tab == app.current_buffer {
        let path_str = app
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
                    return Some(bps.iter().map(|b| b.line).collect());
                }
            }
            None
        })
    } else {
        None
    };
    let multi_active = tab == app.current_buffer && app.multi.is_active();
    // GUI multi-cursor: every caret in `app.sel` except the primary (the primary
    // is painted through caret_*/sel_*). Resolved once for the whole band, not
    // per row/chunk. Empty in the single-cursor case, so the hot path pays
    // nothing. Distinct from the dormant vim `app.multi` above.
    let gui_secondaries: Vec<Position> = if tab == app.current_buffer && app.sel.is_multi() {
        app.secondary_caret_positions()
    } else {
        Vec::new()
    };
    let diags_active = tab == app.current_buffer && !app.lsp.diagnostics.is_empty();

    // Visual row origin for first buffer line in this window (approx: 1:1 before scroll).
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
        let mut text = expand_tabs(raw);
        if text.len() > 480 {
            let mut cut = 480;
            while cut > 0 && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('…');
        }
        let is_cursor_row = row == cursor_row;
        // Xcode-style bracket hint: moving across a closer points out its
        // opener. Kind 254 is a marker span; the FACE owns the ~1s flash, so
        // the core stays stateless and the timing lives with the renderer.
        let bracket_match = if caret_vcol.is_some() && use_live_syntax && tab == app.current_buffer {
            app.buffer.matching_bracket_before_cursor()
        } else {
            None
        };
        let (sel_v0, sel_v1) = if use_live_syntax && tab == app.current_buffer {
            selection_on_line(app, row, &text, sel)
        } else {
            (None, None)
        };
        let full_spans = if use_live_syntax && tab == app.current_buffer {
            syntax_spans_for_row(app, row, raw)
        } else {
            let ext = app
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
                    let v0 = visual_col(raw, start) as u32;
                    let v1 = visual_col(raw, end) as u32;
                    (v1 > v0).then_some(SpanScene {
                        start: v0,
                        end: v1,
                        kind: kind_to_u8(kind),
                    })
                })
                .collect::<Vec<_>>()
        };
        let git_sign = if tab == app.current_buffer {
            git_sign_for_row(app, row)
        } else {
            0
        };
        let chunks = if wrap {
            wrap_visual_chunks(&text, text_width)
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
            let (sv0, sv1) = match (sel_v0, sel_v1) {
                (Some(a), Some(b)) => {
                    let s = a.max(base_col);
                    let e = b.min(end_col);
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
                        + raw.chars().nth(m.col).map(|c| c.len_utf16() as u32).unwrap_or(1);
                    spans.push(SpanScene { start: u0, end: u1, kind: 254 });
                }
            }
            if multi_active {
                for p in &app.multi.extras {
                    if p.row != row {
                        continue;
                    }
                    let vc = visual_col(raw, p.col) as u32;
                    if vc >= base_col && vc < end_col.max(base_col + 1) {
                        let rel = vc.saturating_sub(base_col);
                        spans.push(SpanScene {
                            start: rel,
                            end: rel.saturating_add(1),
                            kind: 250,
                        });
                    }
                }
            }
            // GUI multi-cursor extras. Unlike kind-250 above (cell columns), these
            // carry UTF-16 offsets — the face positions them with CoreText so an
            // extra caret tracks CJK the same way the primary does. The head is an
            // exclusive between-character column, i.e. already the drawn column.
            // (Secondary SELECTION fills are a separate kind, to add with ⌘-D.)
            for head in &gui_secondaries {
                if head.row != row {
                    continue;
                }
                let vc = visual_col(raw, head.col) as u32;
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
                for d in app.lsp.diagnostics_for_row(row) {
                    let v0 = visual_col(raw, d.col_start) as u32;
                    let v1 = visual_col(raw, d.col_end.max(d.col_start.saturating_add(1))) as u32;
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
            if !wrap_cont {
                if let Some(ref set) = bp_lines {
                    if set.contains(&row) {
                        gsign |= 0x40;
                    }
                }
            }
            if spans.len() > 32 {
                spans.truncate(32);
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
                visual_row,
            });
            visual_row = visual_row.saturating_add(1);
        }
    }
    lines
}

/// Split expanded (tab-expanded) text into visual-column chunks for soft-wrap.
fn wrap_visual_chunks(text: &str, width: usize) -> Vec<(u32, String)> {
    if width == 0 || width == usize::MAX {
        return vec![(0, text.to_string())];
    }
    let mut out = Vec::new();
    let mut col: u32 = 0;
    let mut seg_start_col: u32 = 0;
    let mut seg = String::new();
    let mut seg_w = 0usize;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
        if seg_w > 0 && seg_w + w > width {
            out.push((seg_start_col, seg));
            seg = String::new();
            seg_start_col = col;
            seg_w = 0;
        }
        seg.push(ch);
        seg_w += w;
        col += w as u32;
    }
    if seg.is_empty() && out.is_empty() {
        out.push((0, String::new()));
    } else if !seg.is_empty() || out.is_empty() {
        out.push((seg_start_col, seg));
    }
    out
}

/// UTF-16 offset of the character that starts at terminal cell column `vcol`.
/// Bridges the core's cell grid to the renderer's glyph advances.
fn utf16_offset_for_vcol(s: &str, vcol: u32) -> u32 {
    let mut cells = 0usize;
    let mut utf16 = 0usize;
    for ch in s.chars() {
        if cells >= vcol as usize {
            break;
        }
        cells += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
        utf16 += ch.len_utf16();
    }
    utf16 as u32
}

fn visual_width_str(s: &str) -> usize {
    s.chars()
        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1).max(1))
        .sum()
}

fn git_sign_for_row(app: &App, row: usize) -> u8 {
    use suisei_core::git::GitSign;
    match app.git.sign_at(row) {
        Some(GitSign::Added) => 1,
        Some(GitSign::Modified) => 2,
        Some(GitSign::Deleted) => 3,
        None => 0,
    }
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
        let v0 = visual_col(raw, start) as u32;
        let v1 = visual_col(raw, end) as u32;
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
fn drawn_caret_col(app: &App) -> usize {
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
    let line_len = expanded.chars().count() as u32;
    let v0 = if row == start.row {
        visual_col(raw, start.col) as u32
    } else {
        0
    };
    // Exclusive visual end: one past the inclusive buffer end column.
    let v1 = if row == end.row {
        visual_col(raw, end.col.saturating_add(1)) as u32
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

fn expand_tabs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0usize;
    for ch in s.chars() {
        if ch == '\t' {
            let n = 4 - (col % 4);
            for _ in 0..n {
                out.push(' ');
            }
            col += n;
        } else {
            out.push(ch);
            col += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        }
    }
    out
}

fn visual_col(line: &str, buf_col: usize) -> usize {
    let mut col = 0usize;
    for (i, ch) in line.chars().enumerate() {
        if i >= buf_col {
            break;
        }
        if ch == '\t' {
            col += 4 - (col % 4);
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
