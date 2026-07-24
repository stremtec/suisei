//! Pretty document preview (GitHub-flavored Markdown, JSON, plain).
//!
//! TUI-oriented renderer — not a browser, but aims for readable GitHub-like
//! output: GFM tables/tasks/alerts, nested lists & quotes, footnotes,
//! autolinks, images, setext headings, fenced/indented code, entities, escapes.

use std::path::Path;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Markdown,
    Json,
    Plain,
    Image,
    Csv,
    Npy,
    Audio,
}

impl PreviewKind {
    pub fn from_ext(ext: Option<&str>) -> Self {
        match ext.map(|e| e.to_lowercase()).as_deref() {
            Some("md" | "mdx" | "markdown" | "mdown") => PreviewKind::Markdown,
            Some("json" | "jsonc") => PreviewKind::Json,
            Some(e) if crate::media::is_image_ext(e) => PreviewKind::Image,
            Some(e) if crate::media::is_csv_ext(e) => PreviewKind::Csv,
            Some(e) if crate::media::is_npy_ext(e) => PreviewKind::Npy,
            Some(e) if crate::media::is_audio_ext(e) => PreviewKind::Audio,
            _ => PreviewKind::Plain,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PreviewKind::Markdown => "Markdown",
            PreviewKind::Json => "JSON",
            PreviewKind::Plain => "Plain",
            PreviewKind::Image => "Image",
            PreviewKind::Csv => "CSV",
            PreviewKind::Npy => "NumPy",
            PreviewKind::Audio => "Audio",
        }
    }

    pub fn is_media(self) -> bool {
        matches!(
            self,
            PreviewKind::Image | PreviewKind::Csv | PreviewKind::Npy | PreviewKind::Audio
        )
    }
}

#[derive(Debug, Clone)]
pub struct PreviewState {
    pub open: bool,
    pub scroll: usize,
    pub kind: Option<PreviewKind>,
    /// Cached rendered lines (rebuild on open / buffer change fingerprint)
    pub lines: Vec<PreviewLine>,
    pub source_len: usize,
    /// Animation clock — armed by open/close, started by the **first render**
    /// (see `anim_progress`), so synchronous open work can't eat the window.
    pub opened_at: Option<std::time::Instant>,
    pub anim_pending: bool,
    /// Horizontal pan for long pretty lines (wrap_lines = false).
    pub hscroll: usize,
    /// Directory of the previewed file (relative image resolution).
    pub base_dir: Option<std::path::PathBuf>,
    /// (cell_w_px, cell_h_px) for image row budgeting.
    pub cell_dims: (u32, u32),
    /// Openness endpoints for the current phase (0 = source, 1 = fully pretty).
    pub anim_from: f32,
    pub anim_to: f32,
    /// True while exit transform plays (`open` stays true until done).
    pub closing: bool,
    /// One-shot: close animation just finished.
    pub just_closed: bool,
    /// Media preview path (image / audio / npy) — not necessarily the buffer.
    pub media_path: Option<std::path::PathBuf>,
}

/// Transform-reveal animation length (ms) — open and reverse-close.
pub const PREVIEW_ANIM_MS: u64 = 420;

/// Inline image anchored to a preview line (drawn by the Kitty layer).
#[derive(Debug, Clone)]
pub struct ImageBlock {
    /// Resolved local file path.
    pub path: String,
    /// Rows reserved below the anchor for the picture.
    pub rows: u16,
    /// Cell-width budget the row count was computed for.
    pub w_cells: u16,
}

#[derive(Debug, Clone)]
pub struct PreviewLine {
    pub spans: Vec<(String, PreviewStyle)>,
    /// Set on an image anchor row (`![alt](local-file)` on its own line).
    pub image: Option<ImageBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStyle {
    Normal,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    Bold,
    Italic,
    BoldItalic,
    Strike,
    Code,
    CodeBlock,
    CodeLang,
    Link,
    Image,
    Quote,
    AlertNote,
    AlertTip,
    AlertImportant,
    AlertWarning,
    AlertCaution,
    ListBullet,
    TaskDone,
    TaskTodo,
    Hr,
    Dim,
    Footnote,
    Kbd,
    Html,
    JsonKey,
    JsonString,
    JsonNumber,
    JsonLit,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            open: false,
            scroll: 0,
            kind: None,
            lines: Vec::new(),
            source_len: 0,
            opened_at: None,
            anim_pending: false,
            hscroll: 0,
            base_dir: None,
            cell_dims: (14, 28),
            anim_from: 0.0,
            anim_to: 1.0,
            closing: false,
            just_closed: false,
            media_path: None,
        }
    }
}

impl PreviewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(&self) -> bool {
        self.open
    }

    pub fn is_animating(&self) -> bool {
        self.anim_pending
            || self.closing
            || self
                .opened_at
                .is_some_and(|t| t.elapsed().as_millis() < PREVIEW_ANIM_MS as u128)
    }

    /// Instant hide (no reverse sweep).
    pub fn close_immediate(&mut self) {
        self.open = false;
        self.closing = false;
        self.just_closed = false;
        self.scroll = 0;
        self.hscroll = 0;
        self.lines.clear();
        self.kind = None;
        self.source_len = 0;
        self.opened_at = None;
        self.anim_pending = false;
        self.anim_from = 0.0;
        self.anim_to = 0.0;
        self.media_path = None;
    }

    /// Start reverse transform (pretty → source). Keeps `open` until done.
    pub fn close(&mut self) {
        if !self.open || self.closing {
            return;
        }
        let current = self.snapshot_openness();
        self.closing = true;
        self.anim_from = current;
        self.anim_to = 0.0;
        self.anim_pending = true;
        self.opened_at = None;
    }

    pub fn open_for(&mut self, text: &str, ext: Option<&str>) {
        self.open = true;
        self.closing = false;
        self.just_closed = false;
        self.scroll = 0;
        self.kind = Some(PreviewKind::from_ext(ext));
        let from = if self.opened_at.is_some() || self.anim_pending {
            self.snapshot_openness()
        } else {
            0.0
        };
        self.anim_from = from;
        self.anim_to = 1.0;
        self.anim_pending = true;
        self.opened_at = None;
        self.rebuild(text, ext);
    }

    /// **Linear openness** 0.0..=1.0 (0 = source view, 1 = full pretty).
    pub fn anim_progress(&mut self) -> f32 {
        let v = self.tick_openness();
        if self.closing && v <= 0.001 {
            self.finish_close();
        }
        v
    }

    fn snapshot_openness(&self) -> f32 {
        if self.anim_pending {
            return self.anim_from;
        }
        let Some(t0) = self.opened_at else {
            return if self.open && !self.closing {
                1.0
            } else {
                0.0
            };
        };
        let u = (t0.elapsed().as_millis() as f32 / PREVIEW_ANIM_MS as f32).min(1.0);
        self.anim_from + (self.anim_to - self.anim_from) * u
    }

    fn tick_openness(&mut self) -> f32 {
        if self.anim_pending {
            self.anim_pending = false;
            self.opened_at = Some(std::time::Instant::now());
            return self.anim_from;
        }
        let Some(t0) = self.opened_at else {
            return if self.open && !self.closing {
                1.0
            } else {
                0.0
            };
        };
        let u = (t0.elapsed().as_millis() as f32 / PREVIEW_ANIM_MS as f32).min(1.0);
        self.anim_from + (self.anim_to - self.anim_from) * u
    }

    fn finish_close(&mut self) {
        self.open = false;
        self.closing = false;
        self.just_closed = true;
        self.scroll = 0;
        self.lines.clear();
        self.kind = None;
        self.source_len = 0;
        self.opened_at = None;
        self.anim_pending = false;
        self.anim_from = 0.0;
        self.anim_to = 0.0;
        self.media_path = None;
    }

    /// Open media / pretty file from a path (explorer or command).
    pub fn open_path(&mut self, path: &std::path::Path) -> Result<(), String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        let kind = PreviewKind::from_ext(ext.as_deref());
        self.open = true;
        self.closing = false;
        self.just_closed = false;
        self.scroll = 0;
        self.kind = Some(kind);
        self.media_path = Some(path.to_path_buf());
        self.anim_from = 0.0;
        self.anim_to = 1.0;
        self.anim_pending = true;
        self.opened_at = None;

        match kind {
            PreviewKind::Image => {
                let name = path.display().to_string();
                self.lines = vec![
                    PreviewLine {
                        spans: vec![("  Image preview".into(), PreviewStyle::H2)],
                        image: None,
                    },
                    PreviewLine {
                        spans: vec![(format!("  {name}"), PreviewStyle::Dim)],
                        image: None,
                    },
                    PreviewLine {
                        spans: vec![("".into(), PreviewStyle::Normal)],
                        image: None,
                    },
                    PreviewLine {
                        spans: vec![(
                            "  ←/→ or h/l  resize   ·   Esc close".into(),
                            PreviewStyle::Dim,
                        )],
                        image: None,
                    },
                    PreviewLine {
                        spans: vec![(
                            "  (Kitty/Ghostty + gpu_acc for full-res image)".into(),
                            PreviewStyle::Dim,
                        )],
                        image: None,
                    },
                ];
                self.source_len = 0;
                Ok(())
            }
            PreviewKind::Csv => {
                let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
                let tsv = ext.as_deref() == Some("tsv");
                self.lines = crate::media::render_csv(&text, tsv);
                self.source_len = text.len();
                Ok(())
            }
            PreviewKind::Npy => {
                self.lines = crate::media::render_npy(path)?;
                self.source_len = 0;
                Ok(())
            }
            PreviewKind::Audio => {
                self.lines = crate::media::audio_info_lines(path, false);
                self.source_len = 0;
                Ok(())
            }
            PreviewKind::Markdown | PreviewKind::Json | PreviewKind::Plain => {
                let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
                self.media_path = Some(path.to_path_buf());
                self.rebuild(&text, ext.as_deref());
                Ok(())
            }
        }
    }

    pub fn take_just_closed(&mut self) -> bool {
        if self.just_closed {
            self.just_closed = false;
            true
        } else {
            false
        }
    }

    pub fn rebuild(&mut self, text: &str, ext: Option<&str>) {
        let kind = PreviewKind::from_ext(ext);
        self.kind = Some(kind);
        self.source_len = text.len();
        self.lines = match kind {
            PreviewKind::Markdown => {
                render_markdown(text, self.base_dir.as_deref(), self.cell_dims)
            }
            PreviewKind::Json => render_json(text),
            PreviewKind::Plain => render_plain(text),
            PreviewKind::Csv => {
                let tsv = ext.map(|e| e.eq_ignore_ascii_case("tsv")).unwrap_or(false);
                crate::media::render_csv(text, tsv)
            }
            // Image / Npy / Audio need a path — keep existing lines if any
            PreviewKind::Image | PreviewKind::Npy | PreviewKind::Audio => {
                if self.lines.is_empty() {
                    render_plain(text)
                } else {
                    self.lines.clone()
                }
            }
        };
        let max_scroll = self.lines.len().saturating_sub(1);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
    }

    pub fn scroll_by(&mut self, delta: isize, page: usize) {
        let max = self.lines.len().saturating_sub(page.max(1));
        let cur = self.scroll as isize + delta;
        self.scroll = cur.clamp(0, max as isize) as usize;
    }
}

// ── Markdown (GFM-oriented) ─────────────────────────────


/// Resolve a markdown image to a drawable local file + row budget.
fn image_block_for(url: &str, base: Option<&Path>, cell: (u32, u32)) -> Option<ImageBlock> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return None; // no network fetches — caption text only
    }
    let p = Path::new(url);
    let path = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base?.join(p)
    };
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !crate::media::is_image_ext(&ext) {
        return None;
    }
    let (iw, ih) = image::image_dimensions(&path).ok()?;
    if iw == 0 || ih == 0 {
        return None;
    }
    let (cw, ch) = (cell.0.max(4), cell.1.max(6));
    let w_cells: u16 = 56;
    let disp_w_px = w_cells as u32 * cw;
    let disp_h_px = (disp_w_px as u64 * ih as u64 / iw as u64) as u32;
    let rows = (disp_h_px.div_ceil(ch)).clamp(3, 18) as u16;
    Some(ImageBlock {
        path: path.display().to_string(),
        rows,
        w_cells,
    })
}

fn render_markdown(
    text: &str,
    base: Option<&Path>,
    cell: (u32, u32),
) -> Vec<PreviewLine> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut li = 0;
    let mut in_code = false;
    let mut code_fence = String::new(); // "```" or "~~~"
    let mut code_lang = String::new();
    let mut footnotes: Vec<(String, String)> = Vec::new();

    // Skip YAML front matter at file start
    if lines.first().map(|l| l.trim() == "---").unwrap_or(false) {
        li = 1;
        while li < lines.len() {
            if lines[li].trim() == "---" || lines[li].trim() == "..." {
                li += 1;
                break;
            }
            li += 1;
        }
        // subtle notice
        out.push(pl(vec![("  · front matter ·".into(), PreviewStyle::Dim)]));
        out.push(pl(vec![("".into(), PreviewStyle::Normal)]));
    }

    while li < lines.len() {
        let line = lines[li];

        // HTML comments
        if !in_code && line.trim_start().starts_with("<!--") {
            if line.contains("-->") {
                li += 1;
                continue;
            }
            li += 1;
            while li < lines.len() && !lines[li].contains("-->") {
                li += 1;
            }
            if li < lines.len() {
                li += 1;
            }
            continue;
        }

        // Fenced code open/close: ``` or ~~~
        if let Some((fence, rest)) = detect_fence(line) {
            if in_code {
                if fence.starts_with(&code_fence) || code_fence.starts_with(&fence[..fence.len().min(3)]) {
                    // close if same char and length >= open
                    if fence.chars().next() == code_fence.chars().next()
                        && fence.len() >= code_fence.len()
                        && rest.trim().is_empty()
                    {
                        in_code = false;
                        code_fence.clear();
                        code_lang.clear();
                        out.push(pl(vec![("└────────────────────────".into(), PreviewStyle::Dim)]));
                        li += 1;
                        continue;
                    }
                }
                // still inside code
                out.push(code_body_line(line));
                li += 1;
                continue;
            } else {
                in_code = true;
                code_fence = fence;
                code_lang = rest.trim().to_string();
                // strip info string extras (filename etc.) — first token is lang
                let lang = code_lang.split_whitespace().next().unwrap_or("");
                let header = if lang.is_empty() {
                    "┌ code".to_string()
                } else {
                    format!("┌ {lang}")
                };
                out.push(pl(vec![
                    (header, PreviewStyle::CodeLang),
                    (" ────────────────".into(), PreviewStyle::Dim),
                ]));
                li += 1;
                continue;
            }
        }
        if in_code {
            out.push(code_body_line(line));
            li += 1;
            continue;
        }

        // Footnote definition: [^id]: text
        if let Some((id, body)) = parse_footnote_def(line) {
            footnotes.push((id, body));
            li += 1;
            continue;
        }

        // Table
        if line.contains('|') && li + 1 < lines.len() && is_table_separator(lines[li + 1]) {
            let table_rows: Vec<&str> = lines[li..]
                .iter()
                .take_while(|l| l.contains('|') && !l.trim().is_empty())
                .copied()
                .collect();
            let aligns = parse_table_aligns(lines[li + 1]);
            out.extend(render_table(&table_rows, &aligns));
            li += table_rows.len();
            continue;
        }

        // Setext heading: text then === or ---
        if li + 1 < lines.len() && !line.trim().is_empty() {
            let under = lines[li + 1].trim();
            if is_setext_underline(under, '=') {
                out.extend(heading_lines(1, line.trim()));
                li += 2;
                continue;
            }
            if is_setext_underline(under, '-') {
                out.extend(heading_lines(2, line.trim()));
                li += 2;
                continue;
            }
        }

        // HR (before list/heading misparse)
        if is_hr(line) {
            out.push(pl(vec![("─".repeat(56), PreviewStyle::Hr)]));
            li += 1;
            continue;
        }

        // ATX headings #..######
        if let Some((level, title)) = parse_atx_heading(line) {
            out.extend(heading_lines(level, &title));
            li += 1;
            continue;
        }

        // Indented code block (4 spaces or tab) — not a list continuation
        if is_indented_code_start(line) && !looks_like_list(line) {
            let mut block = Vec::new();
            while li < lines.len()
                && (is_indented_code_start(lines[li])
                    || (lines[li].trim().is_empty()
                        && li + 1 < lines.len()
                        && is_indented_code_start(lines[li + 1])))
            {
                if lines[li].trim().is_empty() {
                    block.push(String::new());
                } else {
                    block.push(strip_indent_code(lines[li]));
                }
                li += 1;
            }
            out.push(pl(vec![("┌ code".into(), PreviewStyle::CodeLang)]));
            for b in block {
                out.push(code_body_line(&b));
            }
            out.push(pl(vec![("└────────────────────────".into(), PreviewStyle::Dim)]));
            continue;
        }

        // Blockquote / GitHub alert
        if line.trim_start().starts_with('>') {
            let (quote_lines, consumed) = collect_blockquote(&lines[li..]);
            out.extend(render_blockquote(&quote_lines));
            li += consumed;
            continue;
        }

        // Lists (ul / ol / task), with nesting
        if let Some(item) = parse_list_item(line) {
            let (items, consumed) = collect_list(&lines[li..], item.indent);
            out.extend(render_list(&items));
            li += consumed;
            continue;
        }

        // Empty
        if line.trim().is_empty() {
            out.push(pl(vec![("".into(), PreviewStyle::Normal)]));
            li += 1;
            continue;
        }

        // Standalone image line → real picture (Kitty layer) + caption.
        {
            let t = line.trim();
            let chars: Vec<char> = t.chars().collect();
            if chars.first() == Some(&'!') && chars.get(1) == Some(&'[') {
                if let Some((alt, url, next)) = parse_md_image(&chars, 0) {
                    if next >= chars.len() {
                        if let Some(block) = image_block_for(&url, base, cell) {
                            let cap = if alt.is_empty() {
                                format!("🖼 {url}")
                            } else {
                                format!("🖼 {alt}")
                            };
                            let mut anchor = pl(vec![(format!("  {cap}"), PreviewStyle::Image)]);
                            let rows = block.rows;
                            anchor.image = Some(block);
                            out.push(anchor);
                            for _ in 0..rows {
                                out.push(pl(vec![("".into(), PreviewStyle::Normal)]));
                            }
                            li += 1;
                            continue;
                        }
                    }
                }
            }
        }

        // Paragraph: soft-join consecutive non-blank non-special lines (GFM-ish)
        let (para, consumed) = collect_paragraph(&lines[li..]);
        let joined = para.join(" ");
        out.push(pl(inline_md(&joined)));
        li += consumed;
    }

    // Footnotes appendix
    if !footnotes.is_empty() {
        out.push(pl(vec![("".into(), PreviewStyle::Normal)]));
        out.push(pl(vec![("─".repeat(40), PreviewStyle::Hr)]));
        out.push(pl(vec![("Footnotes".into(), PreviewStyle::H4)]));
        for (id, body) in footnotes {
            let mut spans = vec![(format!("[^{id}]  "), PreviewStyle::Footnote)];
            spans.extend(inline_md(&body));
            out.push(pl(spans));
        }
    }

    if out.is_empty() {
        out.push(pl(vec![("(empty document)".into(), PreviewStyle::Dim)]));
    }
    out
}

fn detect_fence(line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    let indent = line.len() - t.len();
    if indent > 3 {
        return None; // GFM: fence indent ≤ 3
    }
    let c = t.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = t.chars().take_while(|&ch| ch == c).count();
    if n < 3 {
        return None;
    }
    // for backticks, info string must not contain backtick
    let rest = &t[n..];
    if c == '`' && rest.contains('`') {
        return None;
    }
    Some((t[..n].to_string(), rest.to_string()))
}

fn code_body_line(line: &str) -> PreviewLine {
    pl(vec![
        ("│ ".into(), PreviewStyle::Dim),
        (line.to_string(), PreviewStyle::CodeBlock),
    ])
}

fn parse_footnote_def(line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    if !t.starts_with("[^") {
        return None;
    }
    let end = t.find("]:")?;
    let id = t[2..end].to_string();
    if id.is_empty() {
        return None;
    }
    let body = t[end + 2..].trim().to_string();
    Some((id, body))
}

fn is_setext_underline(s: &str, ch: char) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().all(|c| c == ch) && t.len() >= 1
}

fn is_hr(line: &str) -> bool {
    let t: String = line
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if t.len() < 3 {
        return false;
    }
    let first = t.chars().next().unwrap();
    if first != '-' && first != '*' && first != '_' {
        return false;
    }
    t.chars().all(|c| c == first)
}

fn parse_atx_heading(line: &str) -> Option<(usize, String)> {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &t[hashes..];
    // require space or end after hashes (GFM)
    if !rest.is_empty() && !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let mut title = rest.trim().to_string();
    // strip trailing hash closer: " ##"
    while title.ends_with('#') {
        let trimmed = title.trim_end_matches('#').trim_end();
        if trimmed.len() == title.len() {
            break;
        }
        // only strip if space before trailing hashes or all hashes
        if title
            .trim_end_matches('#')
            .chars()
            .last()
            .is_none_or(|c| c.is_whitespace())
            || trimmed.is_empty()
        {
            title = trimmed.to_string();
        } else {
            break;
        }
    }
    Some((hashes, title))
}

fn heading_lines(level: usize, title: &str) -> Vec<PreviewLine> {
    let style = match level {
        1 => PreviewStyle::H1,
        2 => PreviewStyle::H2,
        3 => PreviewStyle::H3,
        4 => PreviewStyle::H4,
        5 => PreviewStyle::H5,
        _ => PreviewStyle::H6,
    };
    let mut spans = inline_md(title);
    for s in &mut spans {
        if s.1 == PreviewStyle::Normal {
            s.1 = style;
        }
    }
    // prefix with level marker for hierarchy
    let marker = match level {
        1 => "",
        2 => "",
        _ => "",
    };
    let mut out = Vec::new();
    if level == 1 {
        out.push(pl(spans));
        out.push(pl(vec![("═".repeat(title_display_width(title).max(8).min(56)), PreviewStyle::Hr)]));
    } else if level == 2 {
        out.push(pl(spans));
        out.push(pl(vec![("─".repeat(title_display_width(title).max(6).min(48)), PreviewStyle::Hr)]));
    } else {
        // h3+: slight indent by level
        let _ = marker;
        let indent = "  ".repeat(level.saturating_sub(3));
        let mut with_indent = vec![(indent, PreviewStyle::Dim)];
        with_indent.extend(spans);
        out.push(pl(with_indent));
    }
    out
}

fn title_display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    UnicodeWidthStr::width(s)
}

fn is_indented_code_start(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

fn strip_indent_code(line: &str) -> String {
    if let Some(r) = line.strip_prefix("    ") {
        r.to_string()
    } else if let Some(r) = line.strip_prefix('\t') {
        r.to_string()
    } else {
        line.to_string()
    }
}

fn looks_like_list(line: &str) -> bool {
    parse_list_item(line).is_some()
}

// ── Blockquote + GitHub alerts ──────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

fn collect_blockquote<'a>(lines: &[&'a str]) -> (Vec<&'a str>, usize) {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if !t.starts_with('>') {
            break;
        }
        out.push(lines[i]);
        i += 1;
    }
    (out, i.max(1))
}

fn strip_quote_prefix(line: &str) -> String {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix('>') {
        if rest.starts_with(' ') {
            rest[1..].to_string()
        } else {
            rest.to_string()
        }
    } else {
        line.to_string()
    }
}

fn parse_alert_marker(s: &str) -> Option<AlertKind> {
    let t = s.trim();
    let upper = t.to_ascii_uppercase();
    // [!NOTE] or [!NOTE] trailing text
    let start = upper.strip_prefix("[!")?;
    let end = start.find(']')?;
    let kind = &start[..end];
    match kind {
        "NOTE" => Some(AlertKind::Note),
        "TIP" => Some(AlertKind::Tip),
        "IMPORTANT" => Some(AlertKind::Important),
        "WARNING" => Some(AlertKind::Warning),
        "CAUTION" => Some(AlertKind::Caution),
        _ => None,
    }
}

fn alert_style(k: AlertKind) -> (PreviewStyle, &'static str) {
    match k {
        AlertKind::Note => (PreviewStyle::AlertNote, "NOTE"),
        AlertKind::Tip => (PreviewStyle::AlertTip, "TIP"),
        AlertKind::Important => (PreviewStyle::AlertImportant, "IMPORTANT"),
        AlertKind::Warning => (PreviewStyle::AlertWarning, "WARNING"),
        AlertKind::Caution => (PreviewStyle::AlertCaution, "CAUTION"),
    }
}

fn render_blockquote(raw_lines: &[&str]) -> Vec<PreviewLine> {
    let mut out = Vec::new();
    if raw_lines.is_empty() {
        return out;
    }
    let bodies: Vec<String> = raw_lines.iter().map(|l| strip_quote_prefix(l)).collect();

    // GitHub alert on first line
    let mut start = 0;
    let mut alert: Option<AlertKind> = None;
    if let Some(k) = parse_alert_marker(&bodies[0]) {
        alert = Some(k);
        start = 1;
    }

    let depth = quote_depth(raw_lines[0]);

    if let Some(k) = alert {
        let (st, label) = alert_style(k);
        let bar = "┃ ".repeat(depth.max(1));
        out.push(pl(vec![
            (bar.clone(), st),
            (format!("⚠ {label}"), st),
        ]));
        for b in bodies.iter().skip(start) {
            if b.trim().is_empty() {
                out.push(pl(vec![(bar.clone(), st)]));
            } else {
                let mut spans = vec![(bar.clone(), st)];
                let mut body = inline_md(b);
                for s in &mut body {
                    if s.1 == PreviewStyle::Normal {
                        s.1 = st;
                    }
                }
                spans.extend(body);
                out.push(pl(spans));
            }
        }
    } else {
        for b in &bodies {
            let bar = "│ ".repeat(depth.max(1));
            if b.trim().is_empty() {
                out.push(pl(vec![(bar, PreviewStyle::Quote)]));
            } else {
                let mut spans = vec![(bar, PreviewStyle::Quote)];
                let mut body = inline_md(b);
                for s in &mut body {
                    if s.1 == PreviewStyle::Normal {
                        s.1 = PreviewStyle::Quote;
                    }
                }
                spans.extend(body);
                out.push(pl(spans));
            }
        }
    }
    out
}

fn quote_depth(line: &str) -> usize {
    let mut d = 0;
    let mut chars = line.trim_start().chars().peekable();
    while chars.peek() == Some(&'>') {
        d += 1;
        chars.next();
        if chars.peek() == Some(&' ') {
            chars.next();
        }
    }
    d.max(1)
}

// ── Lists ───────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ListItem {
    indent: usize,
    ordered: bool,
    number: u32,
    task: Option<bool>, // Some(checked)
    text: String,
}

fn indent_width(line: &str) -> usize {
    let mut w = 0;
    for c in line.chars() {
        match c {
            ' ' => w += 1,
            '\t' => w += 4,
            _ => break,
        }
    }
    w
}

fn parse_list_item(line: &str) -> Option<ListItem> {
    let indent = indent_width(line);
    // only reasonable nest
    if indent > 40 {
        return None;
    }
    let byte_indent = {
        let mut b = 0;
        for c in line.chars() {
            if c == ' ' || c == '\t' {
                b += c.len_utf8();
            } else {
                break;
            }
        }
        b
    };
    let rest = &line[byte_indent..];

    // unordered: - * +
    if let Some(r) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        let (task, text) = parse_task_prefix(r);
        return Some(ListItem {
            indent,
            ordered: false,
            number: 0,
            task,
            text,
        });
    }
    // ordered: 1. or 1)
    if let Some(pos) = rest.find(". ").or_else(|| rest.find(") ")) {
        let num = &rest[..pos];
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            let sep_len = 2;
            let r = &rest[pos + sep_len..];
            let (task, text) = parse_task_prefix(r);
            let number = num.parse().unwrap_or(1);
            return Some(ListItem {
                indent,
                ordered: true,
                number,
                task,
                text,
            });
        }
    }
    None
}

fn parse_task_prefix(s: &str) -> (Option<bool>, String) {
    let t = s.trim_start();
    if let Some(r) = t.strip_prefix("[x] ").or_else(|| t.strip_prefix("[X] ")) {
        return (Some(true), r.to_string());
    }
    if let Some(r) = t.strip_prefix("[ ] ") {
        return (Some(false), r.to_string());
    }
    (None, s.to_string())
}

fn collect_list(lines: &[&str], base_indent: usize) -> (Vec<ListItem>, usize) {
    let mut items = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            // blank: peek if next is still a list at same-or-deeper indent
            if i + 1 < lines.len() {
                if let Some(n) = parse_list_item(lines[i + 1]) {
                    if n.indent >= base_indent {
                        i += 1;
                        continue;
                    }
                }
            }
            break;
        }
        if let Some(item) = parse_list_item(line) {
            if item.indent < base_indent {
                break;
            }
            items.push(item);
            i += 1;
            // continuation lines (indented more, not a new item)
            while i < lines.len() {
                let cont = lines[i];
                if cont.trim().is_empty() {
                    break;
                }
                if parse_list_item(cont).is_some() {
                    break;
                }
                let iw = indent_width(cont);
                if iw > items.last().map(|x| x.indent).unwrap_or(0) {
                    if let Some(last) = items.last_mut() {
                        last.text.push(' ');
                        last.text.push_str(cont.trim());
                    }
                    i += 1;
                } else {
                    break;
                }
            }
            continue;
        }
        break;
    }
    (items, i.max(1))
}

fn render_list(items: &[ListItem]) -> Vec<PreviewLine> {
    let mut out = Vec::new();
    let min_indent = items.iter().map(|i| i.indent).min().unwrap_or(0);
    for it in items {
        let level = (it.indent.saturating_sub(min_indent) / 2).min(6);
        let pad = "  ".repeat(level);
        let mut spans = vec![(pad, PreviewStyle::Dim)];
        match it.task {
            Some(true) => {
                spans.push(("☑ ".into(), PreviewStyle::TaskDone));
            }
            Some(false) => {
                spans.push(("☐ ".into(), PreviewStyle::TaskTodo));
            }
            None if it.ordered => {
                spans.push((format!("{}. ", it.number), PreviewStyle::ListBullet));
            }
            None => {
                let bullet = match level % 3 {
                    0 => "• ",
                    1 => "◦ ",
                    _ => "▪ ",
                };
                spans.push((bullet.into(), PreviewStyle::ListBullet));
            }
        }
        let mut body = inline_md(&it.text);
        if it.task == Some(true) {
            for s in &mut body {
                if s.1 == PreviewStyle::Normal {
                    s.1 = PreviewStyle::Strike;
                }
            }
        }
        spans.extend(body);
        out.push(pl(spans));
    }
    out
}

// ── Paragraph soft-join ─────────────────────────────────

fn is_block_start(line: &str) -> bool {
    if line.trim().is_empty() {
        return true;
    }
    if detect_fence(line).is_some() {
        return true;
    }
    if is_hr(line) {
        return true;
    }
    if parse_atx_heading(line).is_some() {
        return true;
    }
    if line.trim_start().starts_with('>') {
        return true;
    }
    if parse_list_item(line).is_some() {
        return true;
    }
    if line.contains('|') {
        return true;
    }
    if is_indented_code_start(line) {
        return true;
    }
    if parse_footnote_def(line).is_some() {
        return true;
    }
    false
}

fn collect_paragraph<'a>(lines: &[&'a str]) -> (Vec<&'a str>, usize) {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if i > 0 && is_block_start(line) {
            break;
        }
        if line.trim().is_empty() {
            break;
        }
        // hard break: two trailing spaces → keep as separate visual line later?
        out.push(line.trim_end());
        i += 1;
        // setext underline would be consumed by earlier path; stop if next is underline
        if i < lines.len() {
            let u = lines[i].trim();
            if is_setext_underline(u, '=') || is_setext_underline(u, '-') {
                break;
            }
        }
    }
    (out, i.max(1))
}

// ── Tables ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColAlign {
    Left,
    Center,
    Right,
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || !t.contains('|') || !t.contains('-') {
        return false;
    }
    t.split('|')
        .filter(|s| !s.trim().is_empty())
        .all(|seg| {
            let s = seg.trim();
            !s.is_empty()
                && s.chars().all(|c| c == '-' || c == ':' || c == ' ')
                && s.contains('-')
        })
}

fn parse_table_aligns(sep: &str) -> Vec<ColAlign> {
    split_table_row(sep)
        .into_iter()
        .map(|cell| {
            let s = cell.trim();
            let left = s.starts_with(':');
            let right = s.ends_with(':');
            match (left, right) {
                (true, true) => ColAlign::Center,
                (false, true) => ColAlign::Right,
                _ => ColAlign::Left,
            }
        })
        .collect()
}

fn render_table(rows: &[&str], aligns: &[ColAlign]) -> Vec<PreviewLine> {
    use unicode_width::UnicodeWidthStr;
    let mut out = Vec::new();
    if rows.is_empty() {
        return out;
    }

    let cells_all: Vec<Vec<String>> = rows
        .iter()
        .filter(|r| !is_table_separator(r))
        .map(|r| split_table_row(r))
        .collect();
    if cells_all.is_empty() {
        return out;
    }

    let n_cols = cells_all.iter().map(|r| r.len()).max().unwrap_or(1);
    // measure raw text width (without markdown markers ideally — use stripped approx)
    let mut col_widths = vec![3usize; n_cols];
    for row in &cells_all {
        for (i, cell) in row.iter().enumerate() {
            let plain = strip_md_for_width(cell.trim());
            let w = UnicodeWidthStr::width(plain.as_str());
            if i < col_widths.len() && w > col_widths[i] {
                col_widths[i] = w;
            }
        }
    }

    let is_header = rows.len() >= 2;
    out.push(pl(vec![(make_table_border(&col_widths, '┌', '┬', '┐'), PreviewStyle::Hr)]));

    for (row_i, row) in cells_all.iter().enumerate() {
        let mut spans = vec![("│".into(), PreviewStyle::Hr)];
        for i in 0..n_cols {
            let w = col_widths[i];
            let raw = row.get(i).map(|s| s.trim()).unwrap_or("");
            let align = aligns.get(i).copied().unwrap_or(ColAlign::Left);
            let cell_spans = if row_i == 0 && is_header {
                let mut s = inline_md(raw);
                for p in &mut s {
                    if p.1 == PreviewStyle::Normal {
                        p.1 = PreviewStyle::Bold;
                    }
                }
                s
            } else {
                inline_md(raw)
            };
            let content_w: usize = cell_spans
                .iter()
                .map(|(t, _)| UnicodeWidthStr::width(t.as_str()))
                .sum();
            let pad = w.saturating_sub(content_w);
            let (left_pad, right_pad) = match align {
                ColAlign::Left => (0, pad),
                ColAlign::Right => (pad, 0),
                ColAlign::Center => (pad / 2, pad - pad / 2),
            };
            spans.push((" ".into(), PreviewStyle::Normal));
            if left_pad > 0 {
                spans.push((" ".repeat(left_pad), PreviewStyle::Normal));
            }
            spans.extend(cell_spans);
            if right_pad > 0 {
                spans.push((" ".repeat(right_pad), PreviewStyle::Normal));
            }
            spans.push((" │".into(), PreviewStyle::Hr));
        }
        out.push(pl(spans));
        if row_i == 0 && is_header {
            out.push(pl(vec![(
                make_table_border(&col_widths, '├', '┼', '┤'),
                PreviewStyle::Hr,
            )]));
        }
    }
    out.push(pl(vec![(make_table_border(&col_widths, '└', '┴', '┘'), PreviewStyle::Hr)]));
    out
}

fn strip_md_for_width(s: &str) -> String {
    // rough: remove common markers for measurement
    let mut out = s.to_string();
    for m in ["**", "__", "~~", "*", "_", "`"] {
        out = out.replace(m, "");
    }
    out
}

fn make_table_border(col_widths: &[usize], left: char, mid: char, right: char) -> String {
    let mut s = String::from(left);
    for (i, w) in col_widths.iter().enumerate() {
        if i > 0 {
            s.push(mid);
        }
        s.push_str(&"─".repeat(w + 2));
    }
    s.push(right);
    s
}

fn split_table_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let inner = t.trim_start_matches('|').trim_end_matches('|');
    // don't split on \| 
    let mut cells = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '|' {
            cur.push('|');
            i += 2;
            continue;
        }
        if chars[i] == '|' {
            cells.push(cur.trim().to_string());
            cur.clear();
            i += 1;
            continue;
        }
        cur.push(chars[i]);
        i += 1;
    }
    cells.push(cur.trim().to_string());
    cells
}

// ── Inline markdown ─────────────────────────────────────

/// GFM-oriented inline parse. Unmatched markers stay literal.
fn inline_md(s: &str) -> Vec<(String, PreviewStyle)> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<(String, PreviewStyle)> = Vec::new();
    let mut i = 0;
    let mut buf = String::new();

    let flush = |buf: &mut String, out: &mut Vec<(String, PreviewStyle)>| {
        if !buf.is_empty() {
            let decoded = decode_entities(buf);
            out.push((decoded, PreviewStyle::Normal));
            buf.clear();
        }
    };

    while i < chars.len() {
        // escape
        if chars[i] == '\\' && i + 1 < chars.len() {
            let n = chars[i + 1];
            if "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".contains(n) {
                buf.push(n);
                i += 2;
                continue;
            }
        }

        // code span
        if chars[i] == '`' {
            let open_n = count_run(&chars, i, '`');
            if open_n >= 1 && open_n <= 3 {
                if let Some((inner, next)) = take_code_span(&chars, i, open_n) {
                    flush(&mut buf, &mut out);
                    out.push((inner, PreviewStyle::Code));
                    i = next;
                    continue;
                }
            }
            buf.push(chars[i]);
            i += 1;
            continue;
        }

        // HTML <kbd>...</kbd>
        if chars[i] == '<' {
            if let Some((kind, inner, next)) = take_simple_html(&chars, i) {
                flush(&mut buf, &mut out);
                match kind {
                    "kbd" => out.push((inner, PreviewStyle::Kbd)),
                    "br" => out.push(("\n".into(), PreviewStyle::Normal)),
                    "em" | "i" => out.push((inner, PreviewStyle::Italic)),
                    "strong" | "b" => out.push((inner, PreviewStyle::Bold)),
                    "code" => out.push((inner, PreviewStyle::Code)),
                    "del" | "s" => out.push((inner, PreviewStyle::Strike)),
                    _ => out.push((inner, PreviewStyle::Html)),
                }
                i = next;
                continue;
            }
            // autolink <https://...> or <email@x>
            if let Some((url, next)) = take_angle_autolink(&chars, i) {
                flush(&mut buf, &mut out);
                out.push((url, PreviewStyle::Link));
                i = next;
                continue;
            }
        }

        // image ![alt](url) or link [text](url) or footnote [^id]
        if chars[i] == '!' && chars.get(i + 1) == Some(&'[') {
            if let Some((alt, url, next)) = parse_md_image(&chars, i) {
                flush(&mut buf, &mut out);
                let label = if alt.is_empty() {
                    format!("🖼 {url}")
                } else {
                    format!("🖼 {alt}")
                };
                out.push((label, PreviewStyle::Image));
                i = next;
                continue;
            }
        }
        if chars[i] == '[' {
            // footnote ref [^id]
            if chars.get(i + 1) == Some(&'^') {
                if let Some((id, next)) = take_footnote_ref(&chars, i) {
                    flush(&mut buf, &mut out);
                    out.push((format!("[^{id}]"), PreviewStyle::Footnote));
                    i = next;
                    continue;
                }
            }
            if let Some((label, _url, next)) = parse_md_link(&chars, i) {
                flush(&mut buf, &mut out);
                // nested inline in label
                let mut inner = inline_md(&label);
                for s in &mut inner {
                    if s.1 == PreviewStyle::Normal {
                        s.1 = PreviewStyle::Link;
                    }
                }
                if inner.is_empty() {
                    out.push(("↗".into(), PreviewStyle::Link));
                } else {
                    // append link marker to last
                    if let Some(last) = inner.last_mut() {
                        last.0.push_str(" ↗");
                    }
                    out.extend(inner);
                }
                i = next;
                continue;
            }
        }

        // bare URL autolink
        if (chars[i] == 'h' || chars[i] == 'H') && looks_like_url_start(&chars, i) {
            if let Some((url, next)) = take_bare_url(&chars, i) {
                flush(&mut buf, &mut out);
                out.push((url, PreviewStyle::Link));
                i = next;
                continue;
            }
        }

        // strikethrough
        if chars[i] == '~' && chars.get(i + 1) == Some(&'~') {
            if let Some((inner, next)) = take_delimited(&chars, i, "~~", "~~") {
                if !inner.is_empty() {
                    flush(&mut buf, &mut out);
                    // allow nested inline inside strike
                    let mut nested = inline_md(&inner);
                    for s in &mut nested {
                        if s.1 == PreviewStyle::Normal {
                            s.1 = PreviewStyle::Strike;
                        }
                    }
                    out.extend(nested);
                    i = next;
                    continue;
                }
            }
        }

        // highlight ==text== (GFM-ish extras used by some tools)
        if chars[i] == '=' && chars.get(i + 1) == Some(&'=') {
            if let Some((inner, next)) = take_delimited(&chars, i, "==", "==") {
                if !inner.is_empty() {
                    flush(&mut buf, &mut out);
                    out.push((inner, PreviewStyle::AlertTip)); // soft highlight color
                    i = next;
                    continue;
                }
            }
        }

        // *** bold italic *** or ___
        if (chars[i] == '*' || chars[i] == '_')
            && chars.get(i + 1) == Some(&chars[i])
            && chars.get(i + 2) == Some(&chars[i])
        {
            let mark = chars[i];
            let open = format!("{mark}{mark}{mark}");
            if let Some((inner, next)) = take_delimited(&chars, i, &open, &open) {
                if !inner.is_empty() {
                    flush(&mut buf, &mut out);
                    out.push((inner, PreviewStyle::BoldItalic));
                    i = next;
                    continue;
                }
            }
        }

        // ** bold ** or __
        if (chars[i] == '*' || chars[i] == '_') && chars.get(i + 1) == Some(&chars[i]) {
            let mark = chars[i];
            if chars.get(i + 2) != Some(&mark) {
                let open = format!("{mark}{mark}");
                if let Some((inner, next)) = take_delimited(&chars, i, &open, &open) {
                    if !inner.is_empty() {
                        flush(&mut buf, &mut out);
                        let mut nested = inline_md(&inner);
                        for s in &mut nested {
                            if s.1 == PreviewStyle::Normal {
                                s.1 = PreviewStyle::Bold;
                            } else if s.1 == PreviewStyle::Italic {
                                s.1 = PreviewStyle::BoldItalic;
                            }
                        }
                        out.extend(nested);
                        i = next;
                        continue;
                    }
                }
            }
        }

        // * italic * or _
        if chars[i] == '*' || chars[i] == '_' {
            let mark = chars[i];
            let left_ok = mark == '*' || i == 0 || !chars[i - 1].is_alphanumeric();
            if left_ok {
                if let Some((inner, next)) = take_single_emphasis(&chars, i, mark) {
                    if !inner.is_empty() {
                        flush(&mut buf, &mut out);
                        let mut nested = inline_md(&inner);
                        for s in &mut nested {
                            if s.1 == PreviewStyle::Normal {
                                s.1 = PreviewStyle::Italic;
                            } else if s.1 == PreviewStyle::Bold {
                                s.1 = PreviewStyle::BoldItalic;
                            }
                        }
                        out.extend(nested);
                        i = next;
                        continue;
                    }
                }
            }
        }

        buf.push(chars[i]);
        i += 1;
    }
    flush(&mut buf, &mut out);
    if out.is_empty() {
        out.push((String::new(), PreviewStyle::Normal));
    }
    out
}

fn count_run(chars: &[char], start: usize, c: char) -> usize {
    let mut n = 0;
    while start + n < chars.len() && chars[start + n] == c {
        n += 1;
    }
    n
}

fn take_code_span(chars: &[char], start: usize, open_n: usize) -> Option<(String, usize)> {
    let mut i = start + open_n;
    let mut inner = String::new();
    while i < chars.len() {
        if chars[i] == '`' {
            let close_n = count_run(chars, i, '`');
            if close_n == open_n {
                let trimmed = if inner.len() >= 2
                    && inner.starts_with(' ')
                    && inner.ends_with(' ')
                    && !inner[1..inner.len() - 1].trim().is_empty()
                {
                    inner[1..inner.len() - 1].to_string()
                } else {
                    inner
                };
                return Some((trimmed, i + close_n));
            }
            for _ in 0..close_n {
                inner.push('`');
            }
            i += close_n;
            continue;
        }
        if chars[i] == '\n' {
            return None;
        }
        inner.push(chars[i]);
        i += 1;
    }
    None
}

fn take_delimited(chars: &[char], start: usize, open: &str, close: &str) -> Option<(String, usize)> {
    let open_chars: Vec<char> = open.chars().collect();
    let close_chars: Vec<char> = close.chars().collect();
    if open_chars.is_empty() || !chars[start..].starts_with(&open_chars) {
        return None;
    }
    let mut i = start + open_chars.len();
    let mut inner = String::new();
    while i + close_chars.len() <= chars.len() {
        if chars[i..].starts_with(&close_chars) {
            return Some((inner, i + close_chars.len()));
        }
        if chars[i] == '\n' {
            return None;
        }
        inner.push(chars[i]);
        i += 1;
    }
    None
}

fn take_single_emphasis(chars: &[char], start: usize, mark: char) -> Option<(String, usize)> {
    if chars.get(start + 1) == Some(&mark) {
        return None;
    }
    // opening content shouldn't be whitespace (GFM)
    if chars.get(start + 1).is_some_and(|c| c.is_whitespace()) {
        return None;
    }
    let mut i = start + 1;
    let mut inner = String::new();
    while i < chars.len() {
        if chars[i] == mark {
            if chars.get(i + 1) == Some(&mark) {
                return None;
            }
            if mark == '_' {
                let after = chars.get(i + 1);
                if after.is_some_and(|c| c.is_alphanumeric()) {
                    inner.push(chars[i]);
                    i += 1;
                    continue;
                }
            }
            // closing content shouldn't end with whitespace
            if inner.ends_with(|c: char| c.is_whitespace()) || inner.is_empty() {
                return None;
            }
            return Some((inner, i + 1));
        }
        if chars[i] == '\n' {
            return None;
        }
        inner.push(chars[i]);
        i += 1;
    }
    None
}

fn parse_md_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    if chars.get(start) != Some(&'[') {
        return None;
    }
    let mut i = start + 1;
    let mut label = String::new();
    let mut depth = 1;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            label.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if chars[i] == '[' {
            depth += 1;
            label.push('[');
            i += 1;
            continue;
        }
        if chars[i] == ']' {
            depth -= 1;
            if depth == 0 {
                break;
            }
            label.push(']');
            i += 1;
            continue;
        }
        if chars[i] == '\n' {
            return None;
        }
        label.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() || chars.get(i) != Some(&']') {
        return None;
    }
    i += 1; // ]
    if chars.get(i) != Some(&'(') {
        // reference link [text][id] — show label only
        if chars.get(i) == Some(&'[') {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j < chars.len() {
                return Some((label, String::new(), j + 1));
            }
        }
        return None;
    }
    i += 1; // (
    let mut url = String::new();
    // optional <url>
    let angle = chars.get(i) == Some(&'<');
    if angle {
        i += 1;
    }
    while i < chars.len() {
        if angle && chars[i] == '>' {
            i += 1;
            break;
        }
        if !angle && (chars[i] == ')' || chars[i].is_whitespace()) {
            break;
        }
        if chars[i] == '\\' && i + 1 < chars.len() {
            url.push(chars[i + 1]);
            i += 2;
            continue;
        }
        url.push(chars[i]);
        i += 1;
    }
    // skip title "..." or '...'
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if matches!(chars.get(i), Some('"' | '\'')) {
        let q = chars[i];
        i += 1;
        while i < chars.len() && chars[i] != q {
            i += 1;
        }
        if i < chars.len() {
            i += 1;
        }
    }
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if chars.get(i) != Some(&')') {
        return None;
    }
    Some((label, url, i + 1))
}

fn parse_md_image(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    if chars.get(start) != Some(&'!') {
        return None;
    }
    let (alt, url, next) = parse_md_link(chars, start + 1)?;
    Some((alt, url, next))
}

fn take_footnote_ref(chars: &[char], start: usize) -> Option<(String, usize)> {
    // [^id]
    if chars.get(start) != Some(&'[') || chars.get(start + 1) != Some(&'^') {
        return None;
    }
    let mut i = start + 2;
    let mut id = String::new();
    while i < chars.len() && chars[i] != ']' {
        if chars[i].is_whitespace() {
            return None;
        }
        id.push(chars[i]);
        i += 1;
    }
    if id.is_empty() || chars.get(i) != Some(&']') {
        return None;
    }
    // definition is [^id]: — not a ref
    if chars.get(i + 1) == Some(&':') {
        return None;
    }
    Some((id, i + 1))
}

fn take_angle_autolink(chars: &[char], start: usize) -> Option<(String, usize)> {
    if chars.get(start) != Some(&'<') {
        return None;
    }
    let mut i = start + 1;
    let mut url = String::new();
    while i < chars.len() && chars[i] != '>' {
        if chars[i].is_whitespace() || chars[i] == '<' {
            return None;
        }
        url.push(chars[i]);
        i += 1;
    }
    if chars.get(i) != Some(&'>') || url.is_empty() {
        return None;
    }
    // must look like url or email
    let ok = url.contains("://")
        || url.contains('@')
        || url.starts_with("www.");
    if !ok {
        return None;
    }
    Some((url, i + 1))
}

fn looks_like_url_start(chars: &[char], i: usize) -> bool {
    let s: String = chars[i..].iter().take(8).collect();
    let lower = s.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

fn take_bare_url(chars: &[char], start: usize) -> Option<(String, usize)> {
    if !looks_like_url_start(chars, start) {
        return None;
    }
    let mut i = start;
    let mut url = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || "<>\"'`".contains(c) {
            break;
        }
        // trailing punctuation often not part of URL
        url.push(c);
        i += 1;
    }
    // strip trailing .,;:!?)]
    while url
        .chars()
        .last()
        .is_some_and(|c| ".,;:!?)]}".contains(c))
    {
        url.pop();
        i -= 1;
    }
    if url.len() < 8 {
        return None;
    }
    Some((url, i))
}

fn take_simple_html(chars: &[char], start: usize) -> Option<(&'static str, String, usize)> {
    if chars.get(start) != Some(&'<') {
        return None;
    }
    // self-closing <br> <br/>
    let rest: String = chars[start..].iter().take(8).collect();
    let lower = rest.to_ascii_lowercase();
    if lower.starts_with("<br>") || lower.starts_with("<br/>") || lower.starts_with("<br />") {
        let n = if lower.starts_with("<br />") {
            6
        } else if lower.starts_with("<br/>") {
            5
        } else {
            4
        };
        return Some(("br", String::new(), start + n));
    }
    // <tag>inner</tag>
    let mut i = start + 1;
    let mut tag = String::new();
    while i < chars.len() && chars[i].is_ascii_alphabetic() {
        tag.push(chars[i]);
        i += 1;
    }
    if tag.is_empty() || chars.get(i) != Some(&'>') {
        return None;
    }
    i += 1;
    let tag_l = tag.to_ascii_lowercase();
    let kind: &'static str = match tag_l.as_str() {
        "kbd" => "kbd",
        "em" => "em",
        "i" => "i",
        "strong" => "strong",
        "b" => "b",
        "code" => "code",
        "del" => "del",
        "s" => "s",
        "sub" | "sup" | "mark" | "span" => "html",
        _ => return None,
    };
    let close = format!("</{tag_l}>");
    let close_chars: Vec<char> = close.chars().collect();
    let mut inner = String::new();
    while i + close_chars.len() <= chars.len() {
        if chars[i..].starts_with(&close_chars)
            || chars[i..]
                .iter()
                .take(close_chars.len())
                .map(|c| c.to_ascii_lowercase())
                .eq(close_chars.iter().copied())
        {
            // case-insensitive match length
            let mut j = 0;
            let mut ok = true;
            while j < close_chars.len() {
                if chars[i + j].to_ascii_lowercase() != close_chars[j] {
                    ok = false;
                    break;
                }
                j += 1;
            }
            if ok {
                return Some((kind, decode_entities(&inner), i + close_chars.len()));
            }
        }
        if chars[i] == '<' {
            // nested not supported
            return None;
        }
        inner.push(chars[i]);
        i += 1;
    }
    None
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' {
            if let Some((decoded, next)) = take_entity(&chars, i) {
                out.push_str(&decoded);
                i = next;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn take_entity(chars: &[char], start: usize) -> Option<(String, usize)> {
    if chars.get(start) != Some(&'&') {
        return None;
    }
    // &#123; or &#x1F;
    if chars.get(start + 1) == Some(&'#') {
        let hex = chars.get(start + 2) == Some(&'x') || chars.get(start + 2) == Some(&'X');
        let mut i = start + if hex { 3 } else { 2 };
        let mut num = String::new();
        while i < chars.len() && chars[i] != ';' && num.len() < 8 {
            let c = chars[i];
            if hex && c.is_ascii_hexdigit() || !hex && c.is_ascii_digit() {
                num.push(c);
                i += 1;
            } else {
                break;
            }
        }
        if chars.get(i) != Some(&';') || num.is_empty() {
            return None;
        }
        let cp = if hex {
            u32::from_str_radix(&num, 16).ok()?
        } else {
            num.parse().ok()?
        };
        let ch = char::from_u32(cp)?;
        return Some((ch.to_string(), i + 1));
    }
    // named
    let mut i = start + 1;
    let mut name = String::new();
    while i < chars.len() && chars[i].is_ascii_alphanumeric() && name.len() < 16 {
        name.push(chars[i]);
        i += 1;
    }
    if chars.get(i) != Some(&';') {
        return None;
    }
    let decoded = match name.as_str() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" | "39" => "'",
        "nbsp" => " ",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "bull" => "•",
        "middot" => "·",
        "laquo" => "«",
        "raquo" => "»",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "times" => "×",
        "divide" => "÷",
        "ne" => "≠",
        "le" => "≤",
        "ge" => "≥",
        "rarr" => "→",
        "larr" => "←",
        "uarr" => "↑",
        "darr" => "↓",
        "check" => "✓",
        "cross" => "✗",
        _ => return None,
    };
    Some((decoded.to_string(), i + 1))
}

// ── JSON ────────────────────────────────────────────────

fn render_json(text: &str) -> Vec<PreviewLine> {
    let trimmed = text.trim();
    let pretty = pretty_json_heuristic(trimmed);
    let mut out = Vec::new();
    for line in pretty.lines() {
        out.push(pl(colorize_json_line(line)));
    }
    if out.is_empty() {
        out.push(pl(vec![("(empty)".into(), PreviewStyle::Dim)]));
    }
    out
}

fn pretty_json_heuristic(s: &str) -> String {
    let mut out = String::new();
    let mut indent = 0i32;
    let mut in_str = false;
    let mut escape = false;
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                indent += 1;
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] != '}' && bytes[j] != ']' {
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent as usize));
                }
            }
            '}' | ']' => {
                indent = (indent - 1).max(0);
                out.push('\n');
                out.push_str(&"  ".repeat(indent as usize));
                out.push(c);
            }
            ',' => {
                out.push(c);
                out.push('\n');
                out.push_str(&"  ".repeat(indent as usize));
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_whitespace() {
                    j += 1;
                }
                i = j;
                continue;
            }
            ':' => {
                out.push(c);
                out.push(' ');
            }
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
        i += 1;
    }
    if out.is_empty() {
        s.to_string()
    } else {
        out
    }
}

fn colorize_json_line(line: &str) -> Vec<(String, PreviewStyle)> {
    let t = line.trim_start();
    let indent = &line[..line.len() - t.len()];
    let mut spans = vec![(indent.to_string(), PreviewStyle::Normal)];
    if let Some(rest) = t.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let key = format!("\"{}\"", &rest[..end]);
            let after = &rest[end + 1..];
            spans.push((key, PreviewStyle::JsonKey));
            if let Some(colon) = after.find(':') {
                spans.push((after[..=colon].to_string(), PreviewStyle::Dim));
                let val = after[colon + 1..].trim();
                spans.push((val.to_string(), json_val_style(val)));
            } else {
                spans.push((after.to_string(), PreviewStyle::JsonString));
            }
            return spans;
        }
    }
    spans.push((t.to_string(), json_val_style(t)));
    spans
}

fn json_val_style(v: &str) -> PreviewStyle {
    let v = v.trim().trim_end_matches(',');
    if v.starts_with('"') {
        PreviewStyle::JsonString
    } else if v == "true" || v == "false" || v == "null" {
        PreviewStyle::JsonLit
    } else if v.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '-') {
        PreviewStyle::JsonNumber
    } else {
        PreviewStyle::Normal
    }
}

// ── Plain ───────────────────────────────────────────────

fn render_plain(text: &str) -> Vec<PreviewLine> {
    text.lines()
        .map(|l| pl(vec![(l.to_string(), PreviewStyle::Normal)]))
        .collect()
}

fn pl(spans: Vec<(String, PreviewStyle)>) -> PreviewLine {
    PreviewLine { spans, image: None }
}

// ── Style → ratatui ─────────────────────────────────────

/// Blend two RGB colors (falls back to `a` for named colors).
fn mix(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => Color::Rgb(
            (ar as f32 + (br as f32 - ar as f32) * t) as u8,
            (ag as f32 + (bg as f32 - ag as f32) * t) as u8,
            (ab as f32 + (bb as f32 - ab as f32) * t) as u8,
        ),
        _ => a,
    }
}

/// Theme-derived styling: headings grade from accent toward fg, semantic
/// colors come from the active theme so light themes stay readable.
pub fn to_ratatui_style(s: PreviewStyle, theme: &crate::theme::Theme) -> Style {
    let heading = |level: u8| {
        let t = (level.saturating_sub(1)) as f32 * 0.13;
        Style::default()
            .fg(mix(theme.accent, theme.fg, t))
            .add_modifier(Modifier::BOLD)
    };
    match s {
        PreviewStyle::Normal => Style::default().fg(theme.fg),
        PreviewStyle::H1 => heading(1),
        PreviewStyle::H2 => heading(2),
        PreviewStyle::H3 => heading(3),
        PreviewStyle::H4 => heading(4),
        PreviewStyle::H5 => heading(5),
        PreviewStyle::H6 => heading(6),
        PreviewStyle::Bold => Style::default()
            .fg(theme.fg)
            .add_modifier(Modifier::BOLD),
        PreviewStyle::Italic => Style::default()
            .fg(theme.fg)
            .add_modifier(Modifier::ITALIC),
        PreviewStyle::BoldItalic => Style::default()
            .fg(theme.fg)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        PreviewStyle::Strike => Style::default()
            .fg(theme.muted)
            .add_modifier(Modifier::CROSSED_OUT),
        PreviewStyle::Code => Style::default().fg(theme.number).bg(theme.panel_bg),
        PreviewStyle::CodeBlock => Style::default().fg(theme.string).bg(theme.panel_bg),
        PreviewStyle::CodeLang => Style::default()
            .fg(theme.namespace)
            .add_modifier(Modifier::BOLD),
        PreviewStyle::Link => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::UNDERLINED),
        PreviewStyle::Image => Style::default()
            .fg(theme.macro_name)
            .add_modifier(Modifier::ITALIC),
        PreviewStyle::Quote => Style::default().fg(theme.comment),
        PreviewStyle::AlertNote => Style::default().fg(theme.accent),
        PreviewStyle::AlertTip => Style::default().fg(theme.success),
        PreviewStyle::AlertImportant => Style::default()
            .fg(theme.macro_name)
            .add_modifier(Modifier::BOLD),
        PreviewStyle::AlertWarning => Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
        PreviewStyle::AlertCaution => Style::default()
            .fg(theme.error)
            .add_modifier(Modifier::BOLD),
        PreviewStyle::ListBullet => Style::default().fg(theme.string),
        PreviewStyle::TaskDone => Style::default().fg(theme.success),
        PreviewStyle::TaskTodo => Style::default().fg(theme.muted),
        PreviewStyle::Hr => Style::default().fg(theme.border),
        PreviewStyle::Dim => Style::default().fg(theme.muted),
        PreviewStyle::Footnote => Style::default()
            .fg(theme.namespace)
            .add_modifier(Modifier::ITALIC),
        PreviewStyle::Kbd => Style::default()
            .fg(theme.fg)
            .bg(theme.selection_bg)
            .add_modifier(Modifier::BOLD),
        PreviewStyle::Html => Style::default().fg(theme.comment),
        PreviewStyle::JsonKey => Style::default().fg(theme.type_name),
        PreviewStyle::JsonString => Style::default().fg(theme.string),
        PreviewStyle::JsonNumber => Style::default().fg(theme.number),
        PreviewStyle::JsonLit => Style::default().fg(theme.macro_name),
    }
}

pub fn preview_line_to_ratatui(line: &PreviewLine, theme: &crate::theme::Theme) -> Line<'static> {
    if line.spans.is_empty() {
        return Line::from(Span::raw(""));
    }
    let spans: Vec<Span<'static>> = line
        .spans
        .iter()
        .map(|(t, st)| Span::styled(t.clone(), to_ratatui_style(*st, theme)))
        .collect();
    Line::from(spans)
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn render_markdown_t(text: &str) -> Vec<PreviewLine> {
        render_markdown(text, None, (14, 28))
    }

    fn has_style(lines: &[PreviewLine], text: &str, style: PreviewStyle) -> bool {
        lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|(t, s)| t.contains(text) && *s == style)
        })
    }

    #[test]
    fn md_heading_and_bold() {
        let lines = render_markdown_t("# Title\n\nHello **world** and `code`\n");
        assert!(has_style(&lines, "Title", PreviewStyle::H1));
        assert!(has_style(&lines, "world", PreviewStyle::Bold));
        assert!(has_style(&lines, "code", PreviewStyle::Code));
    }

    #[test]
    fn md_list() {
        let lines = render_markdown_t("- one\n- two\n");
        assert!(lines.len() >= 2);
        assert!(lines[0]
            .spans
            .iter()
            .any(|(_, s)| *s == PreviewStyle::ListBullet));
    }

    #[test]
    fn json_pretty() {
        let lines = render_json(r#"{"a":1,"b":"x"}"#);
        assert!(lines.len() > 1);
    }

    #[test]
    fn kind_from_ext() {
        assert_eq!(PreviewKind::from_ext(Some("md")), PreviewKind::Markdown);
        assert_eq!(PreviewKind::from_ext(Some("json")), PreviewKind::Json);
    }

    #[test]
    fn md_inline_quotes_and_apostrophe_stay_literal() {
        let spans = inline_md("it's fine, empty '' ok");
        let joined: String = spans.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, "it's fine, empty '' ok");
        assert!(spans.iter().all(|(_, s)| *s == PreviewStyle::Normal));
    }

    #[test]
    fn md_inline_underscore_italic_and_bold() {
        let spans = inline_md("say _hi_ and __bye__");
        assert!(spans
            .iter()
            .any(|(t, s)| t == "hi" && *s == PreviewStyle::Italic));
        assert!(spans
            .iter()
            .any(|(t, s)| t == "bye" && *s == PreviewStyle::Bold));
        let spans2 = inline_md("snake_case");
        let joined: String = spans2.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, "snake_case");
    }

    #[test]
    fn md_inline_strike_and_unmatched_star() {
        let spans = inline_md("~~gone~~ and lone * star");
        assert!(spans
            .iter()
            .any(|(t, s)| t == "gone" && *s == PreviewStyle::Strike));
        let joined: String = spans.iter().map(|(t, _)| t.as_str()).collect();
        assert!(joined.contains("lone * star"));
    }

    #[test]
    fn md_code_with_inner_backticks() {
        let spans = inline_md("use `` `x` `` please");
        assert!(spans
            .iter()
            .any(|(t, s)| t.contains("`x`") && *s == PreviewStyle::Code));
    }

    #[test]
    fn md_table_renders() {
        let md = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 25 |\n";
        let lines = render_markdown_t(md);
        assert!(lines.len() >= 5, "got {} lines", lines.len());
        assert!(has_style(&lines, "Name", PreviewStyle::Bold));
        assert!(lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|(t, _)| t.contains('┌') || t.contains('├') || t.contains('└'))
        }));
    }

    #[test]
    fn md_task_list_and_nested() {
        let md = "- [x] done\n- [ ] todo\n  - nested\n";
        let lines = render_markdown_t(md);
        assert!(has_style(&lines, "done", PreviewStyle::Strike) || has_style(&lines, "☑", PreviewStyle::TaskDone));
        assert!(lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|(t, s)| t.contains("☐") && *s == PreviewStyle::TaskTodo)
        }));
        assert!(has_style(&lines, "nested", PreviewStyle::Normal) || lines.iter().any(|l| {
            l.spans.iter().any(|(t, _)| t.contains("nested"))
        }));
    }

    #[test]
    fn md_alert_and_blockquote() {
        let md = "> [!WARNING]\n> be careful\n\n> plain quote\n";
        let lines = render_markdown_t(md);
        assert!(has_style(&lines, "WARNING", PreviewStyle::AlertWarning));
        assert!(lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|(t, s)| t.contains("plain") && *s == PreviewStyle::Quote)
        }));
    }

    #[test]
    fn md_autolink_image_entity_escape() {
        let spans = inline_md(r#"see https://example.com and ![logo](a.png) &amp; \*star\*"#);
        assert!(spans.iter().any(|(t, s)| t.contains("example.com") && *s == PreviewStyle::Link));
        assert!(spans.iter().any(|(_, s)| *s == PreviewStyle::Image));
        let joined: String = spans.iter().map(|(t, _)| t.as_str()).collect();
        assert!(joined.contains('&'));
        assert!(joined.contains("*star*"));
    }

    #[test]
    fn md_footnote() {
        let md = "Hello[^1]\n\n[^1]: World note\n";
        let lines = render_markdown_t(md);
        assert!(has_style(&lines, "[^1]", PreviewStyle::Footnote));
        assert!(lines.iter().any(|l| l.spans.iter().any(|(t, _)| t.contains("World"))));
    }

    #[test]
    fn md_setext_and_h6_and_fence_tilde() {
        let md = "Main\n====\n\nSub\n----\n\n###### tiny\n\n~~~rs\nfn x(){}\n~~~\n";
        let lines = render_markdown_t(md);
        assert!(has_style(&lines, "Main", PreviewStyle::H1));
        assert!(has_style(&lines, "Sub", PreviewStyle::H2));
        assert!(has_style(&lines, "tiny", PreviewStyle::H6));
        assert!(has_style(&lines, "rs", PreviewStyle::CodeLang) || has_style(&lines, "fn x", PreviewStyle::CodeBlock));
    }

    #[test]
    fn md_kbd_and_bold_italic() {
        let spans = inline_md("press <kbd>Ctrl</kbd> and ***both***");
        assert!(spans.iter().any(|(t, s)| t == "Ctrl" && *s == PreviewStyle::Kbd));
        assert!(spans
            .iter()
            .any(|(t, s)| t == "both" && *s == PreviewStyle::BoldItalic));
    }

    #[test]
    fn anim_progress_lifecycle() {
        let mut p = PreviewState::new();
        assert!((p.anim_progress() - 1.0).abs() < f32::EPSILON || p.anim_progress() == 0.0);
        p.open_for("# t", Some("md"));
        assert_eq!(p.anim_progress(), 0.0);
        let a = p.anim_progress();
        assert!((0.0..=1.0).contains(&a));
        p.opened_at =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(PREVIEW_ANIM_MS * 4));
        p.anim_from = 0.0;
        p.anim_to = 1.0;
        assert!((p.anim_progress() - 1.0).abs() < 0.01);
        p.close();
        assert!(p.closing);
        assert_eq!(p.anim_progress(), p.anim_from);
        assert!(p.open);
        p.opened_at =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(PREVIEW_ANIM_MS * 4));
        p.anim_from = 1.0;
        p.anim_to = 0.0;
        let _ = p.anim_progress();
        assert!(!p.open);
        assert!(p.take_just_closed());
    }

    #[test]
    fn is_table_sep_detects() {
        assert!(is_table_separator("|---|---|"));
        assert!(is_table_separator("|:--:|---:|"));
        assert!(!is_table_separator("| Name | Age |"));
        assert!(!is_table_separator("hello"));
    }

    #[test]
    fn md_front_matter_skipped() {
        let md = "---\ntitle: x\n---\n# Hi\n";
        let lines = render_markdown_t(md);
        assert!(has_style(&lines, "Hi", PreviewStyle::H1));
        assert!(!lines.iter().any(|l| l.spans.iter().any(|(t, _)| t.contains("title:"))));
    }
}
