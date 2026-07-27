//! C ABI for the Swift face. Fixed buffers — no pointer lifetime traps.

use std::ffi::{c_char, CStr};

use crate::bridge::key_from_ffi;
use crate::runtime::Engine;

// 64: a 16 cap silently hid tabs past the ABI window — the user opened 30
// files and the strip only ever showed the first 16 until closes revealed
// the rest.
pub const SUISEI_MAX_TABS: usize = 64;
pub const SUISEI_MAX_LINES: usize = 256;
pub const SUISEI_MAX_SPANS: usize = 24;
pub const SUISEI_MAX_PANES: usize = 4;
pub const SUISEI_TITLE_CAP: usize = 96;
pub const SUISEI_LINE_CAP: usize = 512;
pub const SUISEI_MSG_CAP: usize = 256;
pub const SUISEI_PATH_CAP: usize = 512;
pub const SUISEI_MODE_CAP: usize = 24;

pub struct SuiseiEngine(Engine);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SuiseiSpanC {
    pub start: u16,
    pub end: u16,
    pub kind: u8,
    pub _pad: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SuiseiEditorLineC {
    pub line_no: u32,
    pub is_cursor: u8,
    pub git_sign: u8,
    pub span_count: u8,
    pub _pad: u8,
    pub caret_vcol: u32,
    /// Caret as a UTF-16 offset into `text` — the GUI places the caret with
    /// real glyph advances, not the core's terminal cell grid.
    pub caret_utf16: u32,
    /// Selection visual start; u32::MAX = none
    pub sel_v0: u32,
    /// Selection visual end (exclusive-ish); u32::MAX = none
    pub sel_v1: u32,
    /// Selection as UTF-16 offsets into `text` (GUI layout).
    pub sel_u0: u32,
    pub sel_u1: u32,
    pub text: [c_char; SUISEI_LINE_CAP],
    pub spans: [SuiseiSpanC; SUISEI_MAX_SPANS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SuiseiPaneC {
    pub tab_index: u32,
    pub scroll: u32,
    pub line_start: u32,
    pub line_count: u32,
    pub focused: u8,
    /// 1 when this pane runs its own shell (see `suisei_engine_terminal_for_pane`).
    pub _pad0: u8,
    pub _pad1: u8,
    pub _pad2: u8,
    /// Total lines in this pane's buffer.
    pub doc_line_count: u32,
    /// Per-pane horizontal pan (0 when wrap on).
    pub hscroll: u32,
    /// Normalised rect within the editor area (0..1), from the layout tree.
    /// The face places panes by these; it no longer re-derives geometry from
    /// `split_kind` + `split_ratio`, which only worked for two panes.
    pub rect_x: f32,
    pub rect_y: f32,
    pub rect_w: f32,
    pub rect_h: f32,
}

#[repr(C)]
pub struct SuiseiChromeSnapshot {
    pub frame_gen: u64,
    pub mode_label: [c_char; SUISEI_MODE_CAP],
    pub message: [c_char; SUISEI_MSG_CAP],
    pub filename: [c_char; SUISEI_PATH_CAP],
    pub breadcrumbs: [c_char; SUISEI_PATH_CAP],
    pub dirty_buffer: u8,
    pub welcome: u8,
    pub explorer_open: u8,
    pub _pad_flags: u8,
    pub cursor_row: u32,
    pub cursor_col: u32,
    pub caret_vcol: u32,
    /// Why Core last moved `scroll` (see `ScrollIntent`): 0 none, 1 restore,
    /// 2 navigate, 3 caret. The face maps this to instant vs animated instead
    /// of guessing from distance.
    pub scroll_intent: u8,
    pub line_count: u32,
    pub scroll: u32,
    pub pct: u32,
    pub scroll_frac: f32,
    pub hscroll: u32,
    pub wrap_lines: u8,
    pub _pad_h0: u8,
    pub _pad_h1: u8,
    pub _pad_h2: u8,
    pub buffer_version: u64,
    pub tab_count: u32,
    pub tab_active: u32,
    pub tab_dirty: [u8; SUISEI_MAX_TABS],
    pub tab_titles: [[c_char; SUISEI_TITLE_CAP]; SUISEI_MAX_TABS],
    /// `BufferTab::id` per tab — stable across reorders, so the face can use it
    /// as list identity and actually animate a move.
    pub tab_ids: [u64; SUISEI_MAX_TABS],
    /// Layout this chip belongs to (0 = none). Consecutive chips sharing a
    /// non-zero value are one folded layout, drawn inside a single container.
    pub tab_groups: [u64; SUISEI_MAX_TABS],
    /// 1 when the chip IS a layout (unified style) rather than a document.
    pub tab_is_layout: [u8; SUISEI_MAX_TABS],
    /// 0 none, 1 vertical, 2 horizontal
    pub split_kind: u8,
    pub pane_count: u8,
    pub pane_focus: u8,
    pub _pad_split: u8,
    pub split_ratio: f32,
    pub panes: [SuiseiPaneC; SUISEI_MAX_PANES],
    pub visible_line_count: u32,
    pub _pad_vis: u32,
    pub lines: [SuiseiEditorLineC; SUISEI_MAX_LINES],
}

fn write_cstr(dst: &mut [c_char], s: &str) {
    dst.fill(0);
    let bytes = s.as_bytes();
    let n = bytes.len().min(dst.len().saturating_sub(1));
    for (i, &b) in bytes.iter().take(n).enumerate() {
        dst[i] = b as c_char;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_new() -> *mut SuiseiEngine {
    let mut engine = Engine::new();
    // Only the real app reports; a test run must not push into the developer's
    // running daemon.
    engine.start_daemon_reporting();
    engine.recompose();
    Box::into_raw(Box::new(SuiseiEngine(engine)))
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_free(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(ptr));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_dispatch_key(
    ptr: *mut SuiseiEngine,
    code: u32,
    ch: u32,
    f_num: u8,
    mods: u8,
) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    let Some(ev) = key_from_ffi(code, ch, f_num, mods) else {
        return 0;
    };
    unsafe {
        (*ptr).0.dispatch_key(ev);
    }
    1
}

/// Terminal-cell column for a UTF-16 offset on `row`.
///
/// The face hit-tests with CoreText (real glyph advances) and gets a UTF-16
/// index; the core speaks cell columns. Converting HERE keeps the East-Asian
/// width rule in exactly one place — doing it in Swift would duplicate it and
/// the two would drift.
fn vcol_for_utf16(eng: &SuiseiEngine, row: u32, utf16_off: u32) -> u32 {
    let buf = &eng.0.app().buffer;
    let row = (row as usize).min(buf.line_count().saturating_sub(1));
    let line = buf.line(row);
    let mut seen_u16 = 0usize;
    let mut vcol = 0usize;
    for ch in line.chars() {
        if seen_u16 >= utf16_off as usize {
            break;
        }
        seen_u16 += ch.len_utf16();
        vcol += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
    }
    vcol as u32
}

/// Click addressed by UTF-16 offset instead of cell column.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_click_utf16(
    ptr: *mut SuiseiEngine,
    buffer_row: u32,
    utf16_off: u32,
    select_word: u8,
) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let vcol = vcol_for_utf16(&*ptr, buffer_row, utf16_off);
        (*ptr).0.click_at(buffer_row, vcol, select_word != 0);
    }
}

/// Drag addressed by UTF-16 offset instead of cell column.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_drag_utf16(
    ptr: *mut SuiseiEngine,
    buffer_row: u32,
    utf16_off: u32,
) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let vcol = vcol_for_utf16(&*ptr, buffer_row, utf16_off);
        (*ptr).0.drag_to(buffer_row, vcol);
    }
}

/// Pre-parse a file into the syntax cache (project auto-indexing).
/// Returns 1 when the file was read and parsed.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_prewarm_file(
    ptr: *mut SuiseiEngine,
    path: *const c_char,
) -> u8 {
    if ptr.is_null() || path.is_null() {
        return 0;
    }
    let Ok(p) = (unsafe { CStr::from_ptr(path) }).to_str() else { return 0 };
    unsafe { u8::from((*ptr).0.prewarm_file(p)) }
}

/// How many files are pre-parsed right now (diagnostics).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_cached_parses(ptr: *const SuiseiEngine) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.cached_parses() as u32 }
}

/// The face consumed the scroll intent — clear it so one move is obeyed once.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_clear_scroll_intent(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.clear_scroll_intent() }
}

/// Cheap probe: is the completion popup open? The typing fast path skips the
/// full chrome pull, but completions must still appear WHILE typing, so it asks
/// this first and only pays for the popup when there is one.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_completions_open(ptr: *const SuiseiEngine) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    eng.0
        .last_diff
        .chrome
        .as_ref()
        .map_or(0, |c| u8::from(c.completions.open))
}

/// Cheap probe for the typing fast path: is the core ready to take text?
///
/// The face used to answer this by pulling the whole chrome snapshot, which
/// costs a full editor-line decode plus a SwiftUI republish. Dispatching a key
/// costs ~1µs; asking whether we may dispatch must not cost more than that.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_editor_accepts_text(ptr: *const SuiseiEngine) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    // Editing is modeless now: the typing fast path is eligible whenever the
    // editor (not a chrome panel or the terminal) owns the keys. A selection is
    // fine — the fast path replaces it. Was `mode_is_insert`, which pinned the
    // fast path to a vim Insert mode the GUI never enters.
    unsafe { u8::from(matches!((*ptr).0.app().mode, suisei_core::app::Mode::Editor)) }
}

/// Reorder the tab bar: move the tab at `from` so it sits at `to`.
///
/// Returns 1 when the order changed. Core carries every index that points into
/// the buffer list — the active tab and every split pane — because panes
/// address their document by position.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_move_tab(ptr: *mut SuiseiEngine, from: u32, to: u32) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    let engine = unsafe { &mut *ptr };
    let moved = engine.0.app_mut().move_tab(from as usize, to as usize);
    if moved {
        engine.0.recompose();
    }
    u8::from(moved)
}

/// Width of the document in display columns (tabs expanded, wide glyphs
/// counted double). The face sizes its horizontal scroll canvas from this.
///
/// Its previous "generous budget" was `max(400, hscroll + 160)` — a width that
/// **grew with the scroll position**, so every pan to the right made the
/// document wider and the pan could never reach an end.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_content_cols(ptr: *mut SuiseiEngine) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.app_mut().content_cols() as u32 }
}

/// Drain side-effects; returns current `frame_gen` (face should paint only when it changes).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_tick(ptr: *mut SuiseiEngine, dt_ms: u32) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.tick(dt_ms) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_frame_gen(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.frame_gen }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_resize(
    ptr: *mut SuiseiEngine,
    css_w: f32,
    css_h: f32,
    line_h: f32,
    cell_w: f32,
    dpr: f32,
) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.resize(css_w, css_h, line_h, cell_w, dpr);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_running(ptr: *const SuiseiEngine) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { u8::from((*ptr).0.running()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_chrome(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiChromeSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let engine = unsafe { &*ptr };
    let Some(chrome) = engine.0.last_diff.chrome.as_ref() else {
        return 0;
    };

    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiChromeSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.frame_gen = engine.0.last_diff.frame_gen;
    write_cstr(&mut o.mode_label, &chrome.mode_label);
    write_cstr(&mut o.message, &chrome.message);
    write_cstr(&mut o.filename, &chrome.filename);
    write_cstr(&mut o.breadcrumbs, &chrome.breadcrumbs);
    o.dirty_buffer = u8::from(chrome.dirty_buffer);
    o.welcome = u8::from(chrome.welcome);
    o.explorer_open = u8::from(chrome.explorer_open);
    o.cursor_row = chrome.cursor_row;
    o.cursor_col = chrome.cursor_col;
    o.caret_vcol = chrome.caret_vcol;
    o.scroll_intent = chrome.scroll_intent;
    o.line_count = chrome.line_count;
    o.scroll = chrome.scroll;
    o.pct = chrome.pct;
    o.scroll_frac = chrome.scroll_frac;
    o.hscroll = chrome.hscroll;
    o.wrap_lines = chrome.wrap_lines;
    o._pad_h0 = 0;
    o._pad_h1 = 0;
    o._pad_h2 = 0;
    o.buffer_version = chrome.buffer_version;
    // branch packed into message suffix is avoided — use separate FFI below

    let tab_n = chrome.tabs.len().min(SUISEI_MAX_TABS);
    o.tab_count = tab_n as u32;
    o.tab_active = chrome.tabs.iter().position(|t| t.active).unwrap_or(0) as u32;
    for (i, tab) in chrome.tabs.iter().take(tab_n).enumerate() {
        o.tab_dirty[i] = u8::from(tab.dirty);
        o.tab_ids[i] = tab.id;
        o.tab_groups[i] = tab.group;
        o.tab_is_layout[i] = u8::from(tab.is_layout);
        write_cstr(&mut o.tab_titles[i], &tab.title);
    }

    // Split metadata + packed per-pane lines.
    o.split_kind = chrome.split_kind;
    o.pane_focus = chrome.pane_focus;
    o.split_ratio = chrome.split_ratio;
    let pane_n = chrome.panes.len().min(SUISEI_MAX_PANES);
    o.pane_count = pane_n as u8;

    let mut packed: Vec<&crate::compositor::EditorLineScene> = Vec::new();
    if chrome.panes.is_empty() {
        // Legacy path: single lines[] stream.
        let line_n = chrome.lines.len().min(SUISEI_MAX_LINES);
        o.visible_line_count = line_n as u32;
        o.pane_count = 1;
        o.panes[0] = SuiseiPaneC {
            tab_index: o.tab_active,
            scroll: chrome.scroll,
            line_start: 0,
            line_count: line_n as u32,
            focused: 1,
            _pad0: u8::from(chrome.pane0_is_terminal),
            _pad1: 0,
            _pad2: 0,
            doc_line_count: chrome.line_count,
            hscroll: chrome.hscroll,
            // Unsplit: the one pane is the whole editor.
            rect_x: 0.0,
            rect_y: 0.0,
            rect_w: 1.0,
            rect_h: 1.0,
        };
        for (i, line) in chrome.lines.iter().take(line_n).enumerate() {
            write_editor_line(&mut o.lines[i], line);
        }
    } else {
        for (pi, pane) in chrome.panes.iter().take(pane_n).enumerate() {
            let start = packed.len() as u32;
            let take = pane.lines.len().min(SUISEI_MAX_LINES.saturating_sub(packed.len()));
            for line in pane.lines.iter().take(take) {
                packed.push(line);
            }
            o.panes[pi] = SuiseiPaneC {
                tab_index: pane.tab_index,
                scroll: pane.scroll,
                line_start: start,
                line_count: take as u32,
                focused: u8::from(pane.focused),
                // Reuses a pad byte — no size change, so the pane stride and
                // every offset after it stay put.
                _pad0: u8::from(pane.is_terminal),
                rect_x: pane.rect.x,
                rect_y: pane.rect.y,
                rect_w: pane.rect.w,
                rect_h: pane.rect.h,
                _pad1: 0,
                _pad2: 0,
                doc_line_count: pane.doc_line_count,
                hscroll: pane.hscroll,
            };
        }
        let line_n = packed.len().min(SUISEI_MAX_LINES);
        o.visible_line_count = line_n as u32;
        for (i, line) in packed.iter().take(line_n).enumerate() {
            write_editor_line(&mut o.lines[i], line);
        }
    }
    1
}

fn write_editor_line(dst: &mut SuiseiEditorLineC, line: &crate::compositor::EditorLineScene) {
    dst.line_no = line.line_no;
    dst.is_cursor = u8::from(line.is_cursor);
    dst.git_sign = line.git_sign;
    dst.caret_vcol = line.caret_vcol;
    dst.caret_utf16 = line.caret_utf16;
    dst.sel_u0 = line.sel_u0;
    dst.sel_u1 = line.sel_u1;
    dst.sel_v0 = line.sel_v0.unwrap_or(u32::MAX);
    dst.sel_v1 = line.sel_v1.unwrap_or(u32::MAX);
    write_cstr(&mut dst.text, &line.text);
    let sn = line.spans.len().min(SUISEI_MAX_SPANS);
    dst.span_count = sn as u8;
    for (j, sp) in line.spans.iter().take(sn).enumerate() {
        dst.spans[j] = SuiseiSpanC {
            start: sp.start.min(u16::MAX as u32) as u16,
            end: sp.end.min(u16::MAX as u32) as u16,
            kind: sp.kind,
            _pad: 0,
        };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_open_path(ptr: *mut SuiseiEngine, path: *const c_char) -> u8 {
    if ptr.is_null() || path.is_null() {
        return 0;
    }
    let cstr = unsafe { CStr::from_ptr(path) };
    let Ok(s) = cstr.to_str() else {
        return 0;
    };
    let engine = unsafe { &mut *ptr };
    let vp = engine.0.app.viewport;
    let path_buf = std::path::PathBuf::from(s);
    let has_session = app_has_editor_session(&engine.0.app);

    if path_buf.is_dir() {
        if has_session {
            // Keep open tabs; only re-root the project tree.
            engine.0.app.explorer.cwd = path_buf.clone();
            engine.0.app.explorer.refresh();
            engine.0.app.explorer.open = true;
            engine.0.app.message = format!("Project {}", s);
        } else {
            // Cold start: leave Welcome (must set filename or open a file).
            let first = first_project_file(&path_buf);
            let mut next = if let Some(ref file) = first {
                suisei_core::app::App::open_file(&file.display().to_string())
            } else {
                let mut a = suisei_core::app::App::default();
                a.apply_config();
                a.filename = Some(path_buf.join("Untitled"));
                a.message = format!("Opened folder {}", s);
                a
            };
            next.viewport = vp;
            next.explorer.cwd = path_buf.clone();
            next.explorer.refresh();
            next.explorer.open = true;
            if first.is_some() {
                next.message = format!("Opened project {}", s);
            }
            engine.0.app = next;
        }
    } else if has_session {
        // Multi-tab IDE path: never wipe existing buffers by replacing App.
        engine.0.app.open_new_tab(s);
        if engine.0.app.explorer.cwd.as_os_str().is_empty()
            || engine.0.app.explorer.entries.is_empty()
        {
            if let Some(parent) = path_buf.parent() {
                engine.0.app.explorer.cwd = parent.to_path_buf();
                engine.0.app.explorer.refresh();
                engine.0.app.explorer.open = true;
            }
        }
    } else {
        let mut next = suisei_core::app::App::open_file(s);
        next.viewport = vp;
        next.message = format!("Opened {}", s);
        if let Some(parent) = path_buf.parent() {
            next.explorer.cwd = parent.to_path_buf();
            next.explorer.refresh();
            next.explorer.open = true;
        }
        engine.0.app = next;
    }
    engine.0.sync_viewport_public();
    engine.0.update_scroll_public();
    engine.0.recompose();
    1
}

/// True once the user has left cold Welcome (any real buffer / tree / multi-tab).
fn app_has_editor_session(app: &suisei_core::app::App) -> bool {
    if app.buffers.len() > 1 {
        return true;
    }
    if app.filename.is_some() {
        return true;
    }
    if app.modified {
        return true;
    }
    if app.buffer.line_count() > 1 || !app.buffer.line(0).is_empty() {
        return true;
    }
    if !app.explorer.entries.is_empty() {
        return true;
    }
    // Explorer cwd defaults to the process launch dir (repo path under a terminal
    // launch, "/" under Finder) — only a cwd the user re-rooted counts as session.
    let default_cwd = std::env::current_dir().unwrap_or_default();
    if !app.explorer.cwd.as_os_str().is_empty() && app.explorer.cwd != default_cwd {
        return true;
    }
    false
}

/// Engine/core version (workspace `version`), NUL-terminated static string.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_version() -> *const c_char {
    static VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
    VERSION.as_ptr() as *const c_char
}

/// Pick a good entry file when opening a project directory (Welcome exit path).
fn first_project_file(root: &std::path::Path) -> Option<std::path::PathBuf> {
    const PREFERRED: &[&str] = &[
        "README.md",
        "README",
        "Cargo.toml",
        "Package.swift",
        "package.json",
        "main.rs",
        "lib.rs",
        "index.ts",
        "index.js",
        "main.swift",
        "App.swift",
    ];
    for name in PREFERRED {
        let p = root.join(name);
        if p.is_file() {
            return Some(p);
        }
        // Common nested locations
        let src = root.join("src").join(name);
        if src.is_file() {
            return Some(src);
        }
    }
    // Shallow scan: first non-hidden regular file
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if p.is_file() {
                files.push(p);
            }
        }
    }
    files.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .cmp(b.file_name().unwrap_or_default())
    });
    files.into_iter().next()
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_scroll(ptr: *mut SuiseiEngine, delta_lines: i32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.scroll_by(delta_lines);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_scroll_h(ptr: *mut SuiseiEngine, delta_cols: i32) {
    if ptr.is_null() || delta_cols == 0 {
        return;
    }
    unsafe {
        (*ptr).0.scroll_h_by(delta_cols);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_scroll_frac(ptr: *mut SuiseiEngine, delta_lines: f32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.scroll_by_frac(delta_lines);
    }
}

/// Absolute scroll for native NSScrollView faces.
/// `line` = first visible buffer row; `hscroll_cols` ignored when wrap is on.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_scroll_to(ptr: *mut SuiseiEngine, line: u32, hscroll_cols: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.scroll_to(line, hscroll_cols);
    }
}

/// Position-only sync (no recompose) — see `Engine::scroll_sync`.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_scroll_sync(ptr: *mut SuiseiEngine, line: u32, hscroll_cols: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.scroll_sync(line, hscroll_cols);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_click(
    ptr: *mut SuiseiEngine,
    buffer_row: u32,
    visual_col: u32,
    select_word: u8,
) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.click_at(buffer_row, visual_col, select_word != 0);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_drag(
    ptr: *mut SuiseiEngine,
    buffer_row: u32,
    visual_col: u32,
) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.drag_to(buffer_row, visual_col);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_mouse_up(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.mouse_up();
    }
}

/// Map editor-local pixel coords → (buffer_row, visual_col).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_hit_test(
    ptr: *const SuiseiEngine,
    local_x: f32,
    local_y: f32,
    gutter_px: f32,
    cell_px: f32,
    line_height_px: f32,
    out_row: *mut u32,
    out_col: *mut u32,
) -> u8 {
    if ptr.is_null() || out_row.is_null() || out_col.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let (r, c) = eng
        .0
        .hit_test(local_x, local_y, gutter_px, cell_px, line_height_px);
    unsafe {
        *out_row = r;
        *out_col = c;
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_save(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.save_file();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_undo(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.undo();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_redo(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.redo();
    }
}

/// Tell the engine the system appearance (1 = dark). The face calls this at
/// launch and whenever `NSApp.effectiveAppearance` changes; when the user has
/// not pinned a theme, it is what selects light vs dark.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_set_system_appearance(ptr: *mut SuiseiEngine, is_dark: u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.app_mut().set_system_appearance(is_dark != 0);
        (*ptr).0.recompose_paint_only();
    }
}

/// Tell the engine a path moved on disk, so open tabs, the active file and the
/// language server follow it. The face performs the filesystem call — it has
/// native Trash and native drag payloads — and reports the result here.
/// Returns how many buffers were repointed.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_path_moved(
    ptr: *mut SuiseiEngine,
    old: *const c_char,
    new: *const c_char,
) -> u32 {
    if ptr.is_null() || old.is_null() || new.is_null() {
        return 0;
    }
    let old = unsafe { CStr::from_ptr(old) }.to_string_lossy().to_string();
    let new = unsafe { CStr::from_ptr(new) }.to_string_lossy().to_string();
    unsafe {
        let n = (*ptr)
            .0
            .app_mut()
            .path_moved(std::path::Path::new(&old), std::path::Path::new(&new));
        (*ptr).0.recompose_paint_only();
        n as u32
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_select_all(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.select_all();
    }
}

// ─── GUI semantic editing commands ────────────────────────────────────────────
//
// The Swift face calls these instead of synthesizing vim keystrokes.
// Mode transitions are handled internally — the GUI never sees modes.

/// Type a printable character at the cursor.
///
/// Enters Insert if needed, replaces active selection (Mac text-field
/// contract). No-op when a panel/terminal owns input.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_gui_type_char(ptr: *mut SuiseiEngine, ch: u32) {
    if ptr.is_null() {
        return;
    }
    let Some(c) = char::from_u32(ch) else { return };
    unsafe {
        (*ptr).0.gui_type_char(c);
    }
}

/// Backspace with Mac selection semantics (deletes selection if active).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_gui_delete_backward(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.gui_delete_backward();
    }
}

/// Forward-delete with Mac selection semantics.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_gui_delete_forward(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.gui_delete_forward();
    }
}

/// Esc semantic: collapse overlays/selection, land in Insert.
///
/// GUI contract: Esc never leaves the editor in Normal mode.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_gui_escape(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.gui_escape();
    }
}

/// Ensure Insert mode (click-to-type, open-file-to-type).
///
/// Collapses any active selection first, then enters Insert.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_gui_ensure_insert(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.gui_focus_editor();
    }
}

/// Open the incremental find bar (⌘F).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_find_open(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.find_open();
    }
}

/// Jump to next (`forward != 0`) / previous match.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_find_step(ptr: *mut SuiseiEngine, forward: u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.find_step(forward != 0);
    }
}

/// Insert text at the caret (or into the PTY when the terminal owns input).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_paste_text(ptr: *mut SuiseiEngine, text: *const c_char) {
    if ptr.is_null() || text.is_null() {
        return;
    }
    let cstr = unsafe { CStr::from_ptr(text) };
    let Ok(s) = cstr.to_str() else {
        return;
    };
    unsafe {
        (*ptr).0.paste_text(s);
    }
}

/// Size the PTY grid to the face terminal panel (cells).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal_resize(ptr: *mut SuiseiEngine, cols: u32, rows: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.terminal_resize(cols, rows);
    }
}

/// Route keys to the PTY (`on != 0`) or back to the editor buffer.
/// Fold the editor's arrangement into a layout tab (J7). Returns 1 on success.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_fold_layout(ptr: *mut SuiseiEngine) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { u8::from((*ptr).0.fold_layout()) }
}

/// Unfold the ACTIVE layout — bound to the tab you are in, never the one under
/// the pointer.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_unfold_layout(ptr: *mut SuiseiEngine) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { u8::from((*ptr).0.unfold_layout()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_activate_layout(ptr: *mut SuiseiEngine, id: u64) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { u8::from((*ptr).0.activate_layout(id)) }
}

/// Switch a layout between grouped and unified strip shapes.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_toggle_layout_style(ptr: *mut SuiseiEngine, id: u64) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { u8::from((*ptr).0.toggle_layout_style(id)) }
}

/// Toggle the docked terminal (⌃T) without going through the key path.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_toggle_terminal_dock(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.toggle_terminal_dock();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_focus_terminal(ptr: *mut SuiseiEngine, on: u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.focus_terminal(on != 0);
    }
}

/// Multi-session shell list (VS Code-style).
/// Scroll the terminal panel through its scrollback; positive reveals older
/// output. Nothing in the GUI could reach the scrollback before this.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal_scroll(ptr: *mut SuiseiEngine, delta_rows: i32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.terminal_scroll(delta_rows);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal_sessions(ptr: *const SuiseiEngine) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.terminal_session_count() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal_active_session(ptr: *const SuiseiEngine) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.terminal_active_session() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal_new_session(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.terminal_new_session();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal_select_session(ptr: *mut SuiseiEngine, idx: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.terminal_select_session(idx);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal_close_session(ptr: *mut SuiseiEngine, idx: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.terminal_close_session(idx);
    }
}

pub const SUISEI_MAX_EXPLORER: usize = 128;
pub const SUISEI_EXPLORER_NAME: usize = 160;
pub const SUISEI_MAX_XLC_OUT: usize = 48;
pub const SUISEI_XLC_LINE: usize = 240;
pub const SUISEI_XLC_INPUT: usize = 256;

#[repr(C)]
pub struct SuiseiExplorerSnapshot {
    pub open: u8,
    pub selected: u32,
    pub count: u32,
    pub cwd: [c_char; SUISEI_PATH_CAP],
    pub is_dir: [u8; SUISEI_MAX_EXPLORER],
    pub names: [[c_char; SUISEI_EXPLORER_NAME]; SUISEI_MAX_EXPLORER],
}


#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_explorer(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiExplorerSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let ex = &chrome.explorer;
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiExplorerSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.open = u8::from(ex.open);
    o.selected = ex
        .entries
        .iter()
        .position(|e| e.selected)
        .unwrap_or(0) as u32;
    write_cstr(&mut o.cwd, &ex.cwd);
    let n = ex.entries.len().min(SUISEI_MAX_EXPLORER);
    o.count = n as u32;
    for (i, e) in ex.entries.iter().take(n).enumerate() {
        o.is_dir[i] = u8::from(e.is_dir);
        write_cstr(&mut o.names[i], &e.name);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_explorer_activate(ptr: *mut SuiseiEngine, index: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.explorer_activate(index);
    }
}

/// Fill Project tree entries without entering Mode::Explorer (docked navigator).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_ensure_project_tree(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.ensure_project_tree();
    }
}

/// Docked Source Control: refresh without Mode::SourceControl.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_ensure_scm(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.ensure_scm_panel();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_close_scm(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.close_scm_panel();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_goto_line(ptr: *mut SuiseiEngine, line_1based: u32) {
    if ptr.is_null() || line_1based == 0 {
        return;
    }
    unsafe {
        (*ptr).0.goto_line(line_1based);
    }
}

/// Exact-range paint band (pull renderer). Rows `[start_row, start_row+max)`.
pub const SUISEI_BAND_MAX: usize = 160;

#[repr(C)]
pub struct SuiseiBandC {
    pub start_row: u32,
    pub count: u32,
    pub doc_line_count: u32,
    pub _pad: u32,
    pub lines: [SuiseiEditorLineC; SUISEI_BAND_MAX],
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_editor_band(
    ptr: *const SuiseiEngine,
    pane: u32,
    start_row: u32,
    max_rows: u32,
    out: *mut SuiseiBandC,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiBandC>());
    }
    let o = unsafe { &mut *out };
    let rows = (max_rows as usize).min(SUISEI_BAND_MAX);
    let (lines, total) = eng.0.editor_band(pane as usize, start_row as usize, rows);
    o.start_row = start_row;
    o.doc_line_count = total;
    let n = lines.len().min(SUISEI_BAND_MAX);
    o.count = n as u32;
    for (i, line) in lines.iter().take(n).enumerate() {
        write_editor_line(&mut o.lines[i], line);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_split_resize(
    ptr: *mut SuiseiEngine,
    pane_a: u32,
    pane_b: u32,
    delta: f32,
) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.split_resize(pane_a, pane_b, delta);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_toggle_breakpoint_line(ptr: *mut SuiseiEngine, line_1based: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.toggle_breakpoint_line(line_1based);
    }
}

/// Downsampled minimap overview.
pub const SUISEI_MINIMAP_MAX: usize = 2048;

#[repr(C)]
pub struct SuiseiMinimapC {
    pub buckets: u32,
    pub total_lines: u32,
    pub indent: [u8; SUISEI_MINIMAP_MAX],
    pub len: [u8; SUISEI_MINIMAP_MAX],
    pub flags: [u8; SUISEI_MINIMAP_MAX],
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_minimap(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiMinimapC,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiMinimapC>());
    }
    let o = unsafe { &mut *out };
    let (buckets, total) = eng.0.minimap(SUISEI_MINIMAP_MAX);
    o.total_lines = total;
    let n = buckets.len().min(SUISEI_MINIMAP_MAX);
    o.buckets = n as u32;
    for (i, (indent, len, flags)) in buckets.iter().take(n).enumerate() {
        o.indent[i] = *indent;
        o.len[i] = *len;
        o.flags[i] = *flags;
    }
    1
}

pub const SUISEI_MAX_OUTLINE: usize = 128;
pub const SUISEI_OUTLINE_NAME: usize = 120;

#[repr(C)]
pub struct SuiseiOutlineSnapshot {
    pub count: u32,
    pub rows: [u32; SUISEI_MAX_OUTLINE],
    pub kinds: [u8; SUISEI_MAX_OUTLINE],
    pub depths: [u8; SUISEI_MAX_OUTLINE],
    pub names: [[c_char; SUISEI_OUTLINE_NAME]; SUISEI_MAX_OUTLINE],
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_outline(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiOutlineSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiOutlineSnapshot>());
    }
    let o = unsafe { &mut *out };
    let n = chrome.outline.len().min(SUISEI_MAX_OUTLINE);
    o.count = n as u32;
    for (i, item) in chrome.outline.iter().take(n).enumerate() {
        o.rows[i] = item.row;
        o.kinds[i] = item.kind;
        o.depths[i] = item.depth;
        write_cstr(&mut o.names[i], &item.name);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_explorer_select(ptr: *mut SuiseiEngine, index: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.explorer_select(index);
    }
}

pub const SUISEI_MAX_PALETTE: usize = 48;
pub const SUISEI_PALETTE_LABEL: usize = 160;
pub const SUISEI_PALETTE_DETAIL: usize = 200;

#[repr(C)]
pub struct SuiseiPaletteSnapshot {
    pub open: u8,
    pub selected: u32,
    pub count: u32,
    pub kind: [c_char; 32],
    pub query: [c_char; 128],
    pub labels: [[c_char; SUISEI_PALETTE_LABEL]; SUISEI_MAX_PALETTE],
    pub details: [[c_char; SUISEI_PALETTE_DETAIL]; SUISEI_MAX_PALETTE],
}

#[repr(C)]
pub struct SuiseiSearchSnapshot {
    pub open: u8,
    pub forward: u8,
    pub match_count: u32,
    pub match_index: u32,
    pub input: [c_char; 256],
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_palette(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiPaletteSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let p = &chrome.palette;
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiPaletteSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.open = u8::from(p.open);
    write_cstr(&mut o.kind, &p.kind);
    write_cstr(&mut o.query, &p.query);
    let n = p.items.len().min(SUISEI_MAX_PALETTE);
    o.count = n as u32;
    o.selected = p
        .items
        .iter()
        .position(|i| i.selected)
        .unwrap_or(0) as u32;
    for (i, it) in p.items.iter().take(n).enumerate() {
        write_cstr(&mut o.labels[i], &it.label);
        write_cstr(&mut o.details[i], &it.detail);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_search(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiSearchSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let s = &chrome.search;
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiSearchSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.open = u8::from(s.open);
    o.forward = u8::from(s.forward);
    o.match_count = s.match_count;
    o.match_index = s.match_index;
    write_cstr(&mut o.input, &s.input);
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_goto_tab(ptr: *mut SuiseiEngine, index: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.goto_tab(index);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_close_tab(ptr: *mut SuiseiEngine, index: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.close_tab(index);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_open_blank_tab(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.open_blank_tab();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_split_vertical(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.split_vertical();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_split_horizontal(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.split_horizontal();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_focus_next_pane(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.focus_next_pane();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_focus_pane(ptr: *mut SuiseiEngine, index: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.focus_pane(index);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_close_focused_pane(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.close_focused_pane();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_palette_activate(ptr: *mut SuiseiEngine, index: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.palette_activate(index);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_palette_select(ptr: *mut SuiseiEngine, index: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.palette_select(index);
    }
}

pub const SUISEI_MAX_HINTS: usize = 24;
pub const SUISEI_HINT_KEY: usize = 16;
pub const SUISEI_HINT_DESC: usize = 48;
pub const SUISEI_MAX_COMP: usize = 20;
pub const SUISEI_COMP_LABEL: usize = 64;
/// Rows the terminal snapshot can carry. Was 120; a full-panel terminal on a
/// tall display asks for more than that, and rows past the cap simply vanished.
pub const SUISEI_MAX_TERM_LINES: usize = 200;
/// **Bytes** per terminal row — not columns. Each row is a truecolor SGR string
/// (`Terminal::visible_rows_sgr`), so one colour change costs up to 19 bytes on
/// top of the character it colours. At the old 256 a wide `ls --color` or build
/// log ran out of budget after roughly a dozen colour changes and the rest of
/// the line was dropped: the reported "terminal gets cut off".
pub const SUISEI_TERM_LINE: usize = 1536;


#[repr(C)]
pub struct SuiseiCompletionsSnapshot {
    pub open: u8,
    pub selected: u32,
    pub count: u32,
    pub prefix: [c_char; 64],
    pub labels: [[c_char; SUISEI_COMP_LABEL]; SUISEI_MAX_COMP],
    pub details: [[c_char; SUISEI_COMP_LABEL]; SUISEI_MAX_COMP],
}

#[repr(C)]
pub struct SuiseiTerminalSnapshot {
    pub open: u8,
    pub full_panel: u8,
    /// Split pane index for pane-bound full terminal; `0xFFFFFFFF` = none / whole main.
    pub pane_bound: u32,
    pub count: u32,
    /// Shell cursor within the emitted grid. Never sent before, which is why
    /// the terminal had no visible caret at all — nothing was missing in the
    /// renderer, the position simply never crossed the bridge.
    pub cursor_row: u32,
    pub cursor_col: u32,
    pub lines: [[c_char; SUISEI_TERM_LINE]; SUISEI_MAX_TERM_LINES],
}

#[repr(C)]
pub struct SuiseiStatusExtra {
    pub branch: [c_char; 64],
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_completions(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiCompletionsSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let c = &chrome.completions;
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiCompletionsSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.open = u8::from(c.open);
    o.selected = c.selected;
    write_cstr(&mut o.prefix, &c.prefix);
    let n = c.items.len().min(SUISEI_MAX_COMP);
    o.count = n as u32;
    for (i, (lab, det)) in c.items.iter().take(n).enumerate() {
        write_cstr(&mut o.labels[i], lab);
        write_cstr(&mut o.details[i], det);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiTerminalSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let t = &chrome.terminal;
    // Deliberately NOT a blanket `write_bytes(.., 0, size_of::<..>())`: this
    // struct is 300 KiB and the face pulls it on every refresh while the
    // terminal is open. Every header field is assigned below, and a row only
    // needs its first byte cleared to read as an empty C string — `write_cstr`
    // zeroes the rows it fills. That turns a 300 KiB memset into 200 stores.
    let o = unsafe { &mut *out };
    o.open = u8::from(t.open);
    o.full_panel = u8::from(t.full_panel);
    o.pane_bound = t.pane_bound.unwrap_or(u32::MAX);
    let n = t.lines.len().min(SUISEI_MAX_TERM_LINES);
    o.count = n as u32;
    for row in o.lines.iter_mut().skip(n) {
        row[0] = 0;
    }
    // Shell cursor, so the face can actually draw a caret in the terminal.
    // NOTE: cursor_position() returns (COL, ROW) — reading it as (row, col)
    // put the caret on the wrong line, which read as "no cursor at all".
    let (ccol, crow) = eng.0.app().terminal.cursor_position();
    o.cursor_row = crow as u32;
    o.cursor_col = ccol as u32;
    for (i, line) in t.lines.iter().take(n).enumerate() {
        write_cstr(&mut o.lines[i], line);
    }
    1
}

/// Rows for the shell running in a specific pane.
///
/// Pane terminals are separate processes, so there is no single "the terminal"
/// to ask for. `suisei_engine_terminal` remains the docked one.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal_for_pane(
    ptr: *const SuiseiEngine,
    pane: u32,
    out: *mut SuiseiTerminalSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(term) = eng.0.app().pane_terminal(pane as usize) else {
        return 0;
    };
    let o = unsafe { &mut *out };
    o.open = 1;
    o.full_panel = 1;
    o.pane_bound = pane;
    let lines: Vec<String> = term
        .visible_rows_sgr()
        .into_iter()
        .take(SUISEI_MAX_TERM_LINES)
        .collect();
    let n = lines.len();
    o.count = n as u32;
    for row in o.lines.iter_mut().skip(n) {
        row[0] = 0;
    }
    let (ccol, crow) = term.cursor_position();
    o.cursor_row = crow as u32;
    o.cursor_col = ccol as u32;
    for (i, line) in lines.iter().enumerate() {
        write_cstr(&mut o.lines[i], line);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_status_extra(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiStatusExtra,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiStatusExtra>());
    }
    write_cstr(&mut unsafe { &mut *out }.branch, &chrome.branch);
    1
}

pub const SUISEI_MAX_SETTINGS_ROWS: usize = 48;
pub const SUISEI_SETTINGS_LABEL: usize = 96;
pub const SUISEI_SETTINGS_VALUE: usize = 64;
pub const SUISEI_MAX_SETTINGS_TABS: usize = 8;

#[repr(C)]
pub struct SuiseiSettingsSnapshot {
    pub open: u8,
    pub dirty: u8,
    pub page_index: u32,
    pub selected: u32,
    pub tab_count: u32,
    pub row_count: u32,
    pub status: [c_char; 160],
    pub tabs: [[c_char; 24]; SUISEI_MAX_SETTINGS_TABS],
    pub row_header: [u8; SUISEI_MAX_SETTINGS_ROWS],
    pub row_selected: [u8; SUISEI_MAX_SETTINGS_ROWS],
    pub row_labels: [[c_char; SUISEI_SETTINGS_LABEL]; SUISEI_MAX_SETTINGS_ROWS],
    pub row_values: [[c_char; SUISEI_SETTINGS_VALUE]; SUISEI_MAX_SETTINGS_ROWS],
}

#[repr(C)]
pub struct SuiseiThemeSnapshot {
    pub name: [c_char; 32],
    pub editor_bg: u32,
    pub fg: u32,
    pub dim: u32,
    pub accent: u32,
    pub selection: u32,
    pub caret: u32,
    pub status_bg: u32,
    pub keyword: u32,
    pub string_col: u32,
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

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiSettingsSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let s = &chrome.settings;
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiSettingsSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.open = u8::from(s.open);
    o.dirty = u8::from(s.dirty);
    o.page_index = s.page_index;
    o.selected = s.selected;
    write_cstr(&mut o.status, &s.status);
    let tn = s.tabs.len().min(SUISEI_MAX_SETTINGS_TABS);
    o.tab_count = tn as u32;
    for (i, t) in s.tabs.iter().take(tn).enumerate() {
        write_cstr(&mut o.tabs[i], t);
    }
    let rn = s.rows.len().min(SUISEI_MAX_SETTINGS_ROWS);
    o.row_count = rn as u32;
    for (i, r) in s.rows.iter().take(rn).enumerate() {
        o.row_header[i] = u8::from(r.is_header);
        o.row_selected[i] = u8::from(r.selected);
        write_cstr(&mut o.row_labels[i], &r.label);
        write_cstr(&mut o.row_values[i], &r.value);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_theme(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiThemeSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let t = &chrome.theme;
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiThemeSnapshot>());
    }
    let o = unsafe { &mut *out };
    write_cstr(&mut o.name, &t.name);
    o.editor_bg = t.editor_bg;
    o.fg = t.fg;
    o.dim = t.dim;
    o.accent = t.accent;
    o.selection = t.selection;
    o.caret = t.caret;
    o.status_bg = t.status_bg;
    o.keyword = t.keyword;
    o.string_col = t.string;
    o.comment = t.comment;
    o.number = t.number;
    o.type_name = t.type_name;
    o.function = t.function;
    o.macro_name = t.macro_name;
    o.namespace = t.namespace;
    o.parameter = t.parameter;
    o.property = t.property;
    o.constant = t.constant;
    o.operator = t.operator;
    o.punctuation = t.punctuation;
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_select(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.settings_select(row);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_activate(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.settings_activate(row);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_goto_page(ptr: *mut SuiseiEngine, page: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.settings_goto_page(page);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_save(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.settings_save();
    }
}

pub const SUISEI_MAX_SCM: usize = 48;
pub const SUISEI_SCM_PATH: usize = 160;
pub const SUISEI_MAX_SCM_GRAPH: usize = 40;
pub const SUISEI_GRAPH_LINE: usize = 200;
pub const SUISEI_GIT_WB_LINE: usize = 220;
pub const SUISEI_MAX_GIT_CHIPS: usize = 9;
pub const SUISEI_MAX_GIT_COL: usize = 64;

#[repr(C)]
pub struct SuiseiScmSnapshot {
    pub open: u8,
    pub staged_count: u32,
    pub change_count: u32,
    pub selected: u32,
    pub graph_count: u32,
    pub branch: [c_char; 64],
    pub status: [c_char; 160],
    pub staged_flags: [u8; SUISEI_MAX_SCM],
    pub marks: [c_char; SUISEI_MAX_SCM],
    pub paths: [[c_char; SUISEI_SCM_PATH]; SUISEI_MAX_SCM],
    pub graph_selected: [u8; SUISEI_MAX_SCM_GRAPH],
    pub graph_lines: [[c_char; SUISEI_GRAPH_LINE]; SUISEI_MAX_SCM_GRAPH],
}

#[repr(C)]
pub struct SuiseiGitWbSnapshot {
    pub open: u8,
    pub docked: u8,
    pub loading: u8,
    pub tab_index: u32,
    pub chip_count: u32,
    pub changes_count: u32,
    pub log_count: u32,
    pub files_count: u32,
    pub special_count: u32,
    pub branch: [c_char; 64],
    pub message: [c_char; 160],
    pub chip_active: [u8; SUISEI_MAX_GIT_CHIPS],
    pub chip_keys: [u8; SUISEI_MAX_GIT_CHIPS],
    pub chip_labels: [[c_char; 24]; SUISEI_MAX_GIT_CHIPS],
    pub col_changes: [[c_char; SUISEI_GIT_WB_LINE]; SUISEI_MAX_GIT_COL],
    pub col_log: [[c_char; SUISEI_GIT_WB_LINE]; SUISEI_MAX_GIT_COL],
    pub col_files: [[c_char; SUISEI_GIT_WB_LINE]; SUISEI_MAX_GIT_COL],
    pub special: [[c_char; SUISEI_GIT_WB_LINE]; SUISEI_MAX_GIT_COL],
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_scm(ptr: *const SuiseiEngine, out: *mut SuiseiScmSnapshot) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let s = &chrome.scm;
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiScmSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.open = u8::from(s.open);
    write_cstr(&mut o.branch, &s.branch);
    write_cstr(&mut o.status, &s.status);
    o.selected = 0;
    let mut i = 0usize;
    for e in s.staged.iter().take(SUISEI_MAX_SCM) {
        o.staged_flags[i] = 1;
        o.marks[i] = e.mark.chars().next().unwrap_or('?') as c_char;
        write_cstr(&mut o.paths[i], &e.path);
        if e.selected {
            o.selected = i as u32;
        }
        i += 1;
    }
    o.staged_count = i as u32;
    let staged_n = i;
    for e in s.changes.iter().take(SUISEI_MAX_SCM.saturating_sub(i)) {
        o.staged_flags[i] = 0;
        o.marks[i] = e.mark.chars().next().unwrap_or('?') as c_char;
        write_cstr(&mut o.paths[i], &e.path);
        if e.selected {
            o.selected = i as u32;
        }
        i += 1;
    }
    o.change_count = (i - staged_n) as u32;
    let gn = s.graph.len().min(SUISEI_MAX_SCM_GRAPH);
    o.graph_count = gn as u32;
    for (gi, g) in s.graph.iter().take(gn).enumerate() {
        o.graph_selected[gi] = u8::from(g.selected);
        let line = format!("{} {}  {}  {}", g.strip, g.short, g.subject, g.when);
        write_cstr(&mut o.graph_lines[gi], &line);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiGitWbSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let g = &chrome.git_wb;
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiGitWbSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.open = u8::from(g.open);
    o.docked = u8::from(g.docked);
    o.loading = u8::from(g.loading);
    o.tab_index = g.tab_index;
    write_cstr(&mut o.branch, &g.branch);
    write_cstr(&mut o.message, &g.message);
    let cn = g.chips.len().min(SUISEI_MAX_GIT_CHIPS);
    o.chip_count = cn as u32;
    for (i, c) in g.chips.iter().take(cn).enumerate() {
        o.chip_active[i] = u8::from(c.active);
        o.chip_keys[i] = c.key;
        write_cstr(&mut o.chip_labels[i], &c.label);
    }
    let pack = |dst: &mut [[c_char; SUISEI_GIT_WB_LINE]; SUISEI_MAX_GIT_COL],
                src: &[String]|
     -> u32 {
        let n = src.len().min(SUISEI_MAX_GIT_COL);
        for (i, line) in src.iter().take(n).enumerate() {
            write_cstr(&mut dst[i], line);
        }
        n as u32
    };
    o.changes_count = pack(&mut o.col_changes, &g.col_changes);
    o.log_count = pack(&mut o.col_log, &g.col_log);
    o.files_count = pack(&mut o.col_files, &g.col_files);
    o.special_count = pack(&mut o.special, &g.special);
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_set_tab(ptr: *mut SuiseiEngine, key: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.git_wb_set_tab(key);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_save_as(ptr: *mut SuiseiEngine, path: *const c_char) {
    if ptr.is_null() || path.is_null() {
        return;
    }
    let cstr = unsafe { CStr::from_ptr(path) };
    let Ok(s) = cstr.to_str() else {
        return;
    };
    unsafe {
        (*ptr).0.save_as(s);
    }
}

pub const SUISEI_MAX_BREAKPOINTS: usize = 128;
pub const SUISEI_BP_NAME: usize = 96;

#[repr(C)]
pub struct SuiseiBreakpointSnapshot {
    pub count: u32,
    pub lines: [u32; SUISEI_MAX_BREAKPOINTS],
    pub verified: [u8; SUISEI_MAX_BREAKPOINTS],
    pub has_condition: [u8; SUISEI_MAX_BREAKPOINTS],
    pub has_log: [u8; SUISEI_MAX_BREAKPOINTS],
    pub paths: [[c_char; SUISEI_PATH_CAP]; SUISEI_MAX_BREAKPOINTS],
    pub names: [[c_char; SUISEI_BP_NAME]; SUISEI_MAX_BREAKPOINTS],
    pub conditions: [[c_char; 96]; SUISEI_MAX_BREAKPOINTS],
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_breakpoints(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiBreakpointSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiBreakpointSnapshot>());
    }
    let o = unsafe { &mut *out };
    let rows = eng.0.list_breakpoints();
    let n = rows.len().min(SUISEI_MAX_BREAKPOINTS);
    o.count = n as u32;
    for (i, row) in rows.iter().take(n).enumerate() {
        o.lines[i] = row.line_1based;
        o.verified[i] = if row.verified { 1 } else { 0 };
        o.has_condition[i] = if row.condition.is_empty() { 0 } else { 1 };
        o.has_log[i] = if row.has_log { 1 } else { 0 };
        write_cstr(&mut o.paths[i], &row.path);
        write_cstr(&mut o.names[i], &row.name);
        write_cstr(&mut o.conditions[i], &row.condition);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_goto_breakpoint(
    ptr: *mut SuiseiEngine,
    path: *const c_char,
    line_1based: u32,
) {
    if ptr.is_null() || path.is_null() || line_1based == 0 {
        return;
    }
    let cstr = unsafe { CStr::from_ptr(path) };
    let Ok(s) = cstr.to_str() else {
        return;
    };
    unsafe {
        (*ptr).0.goto_breakpoint(s, line_1based);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_remove_breakpoint(
    ptr: *mut SuiseiEngine,
    path: *const c_char,
    line_1based: u32,
) {
    if ptr.is_null() || path.is_null() || line_1based == 0 {
        return;
    }
    let cstr = unsafe { CStr::from_ptr(path) };
    let Ok(s) = cstr.to_str() else {
        return;
    };
    unsafe {
        (*ptr).0.remove_breakpoint(s, line_1based);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_toggle_breakpoint_cursor(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.toggle_breakpoint_cursor();
    }
}

/// One FFI chunk of preview lines (face pages through with `start`).
pub const SUISEI_MAX_PREVIEW: usize = 128;
pub const SUISEI_PREVIEW_LINE: usize = 512;

#[repr(C)]
pub struct SuiseiPreviewSnapshot {
    pub open: u8,
    pub kind: u8,
    pub scroll: u32,
    pub hscroll: u32,
    /// Total lines available in the scene (may exceed this chunk).
    pub total: u32,
    /// Number of lines filled in this snapshot (from `start`).
    pub count: u32,
    /// First line index this chunk represents.
    pub start: u32,
    pub styles: [u8; SUISEI_MAX_PREVIEW],
    pub lines: [[c_char; SUISEI_PREVIEW_LINE]; SUISEI_MAX_PREVIEW],
}

/// Load a range of pretty-preview lines starting at `start` (0-based).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_preview(
    ptr: *const SuiseiEngine,
    start: u32,
    out: *mut SuiseiPreviewSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiPreviewSnapshot>());
    }
    let o = unsafe { &mut *out };
    let Some(chrome) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let p = &chrome.preview;
    o.open = u8::from(p.open);
    o.kind = p.kind;
    o.scroll = p.scroll;
    o.hscroll = p.hscroll;
    o.total = p.lines.len() as u32;
    o.start = start;
    if !p.open {
        o.count = 0;
        return 1;
    }
    let start_i = start as usize;
    if start_i >= p.lines.len() {
        o.count = 0;
        return 1;
    }
    let n = p.lines.len().saturating_sub(start_i).min(SUISEI_MAX_PREVIEW);
    o.count = n as u32;
    for (i, line) in p.lines.iter().skip(start_i).take(n).enumerate() {
        o.styles[i] = line.style;
        write_cstr(&mut o.lines[i], &line.text);
    }
    1
}

// ---------------------------------------------------------------------------
// Issue navigator — diagnostics.
//
// The core has carried these all along; only line spans (kinds 251-253) ever
// reached the face, so the list itself was unreachable from the GUI.
// ---------------------------------------------------------------------------

pub const SUISEI_MAX_DIAGS: usize = 200;
pub const SUISEI_DIAG_MSG: usize = 240;

#[repr(C)]
pub struct SuiseiDiagnosticsSnapshot {
    pub count: u32,
    pub rows: [u32; SUISEI_MAX_DIAGS],
    pub cols: [u32; SUISEI_MAX_DIAGS],
    /// 0 error · 1 warning · 2 info · 3 hint
    pub severities: [u8; SUISEI_MAX_DIAGS],
    pub messages: [[c_char; SUISEI_DIAG_MSG]; SUISEI_MAX_DIAGS],
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_diagnostics(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiDiagnosticsSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiDiagnosticsSnapshot>());
    }
    let o = unsafe { &mut *out };
    let diags = &eng.0.app().lsp.diagnostics;
    let n = diags.len().min(SUISEI_MAX_DIAGS);
    o.count = n as u32;
    for (i, d) in diags.iter().take(n).enumerate() {
        o.rows[i] = d.row as u32;
        o.cols[i] = d.col_start as u32;
        o.severities[i] = match d.severity {
            suisei_core::lsp::DiagnosticSeverity::Error => 0,
            suisei_core::lsp::DiagnosticSeverity::Warning => 1,
            suisei_core::lsp::DiagnosticSeverity::Info => 2,
            _ => 3,
        };
        write_cstr(&mut o.messages[i], &d.message);
    }
    1
}

// ---------------------------------------------------------------------------
// Find navigator — project-wide search.
//
// Deliberately takes NO engine pointer. `search_project` is a free function, so
// keeping it engine-free lets Swift run it off the main thread; routing it
// through the engine would either block the UI (the indexer already taught us
// what that costs) or need locking the rest of the ABI does not have.
// ---------------------------------------------------------------------------

pub const SUISEI_MAX_HITS: usize = 300;
pub const SUISEI_HIT_PATH: usize = 512;
pub const SUISEI_HIT_LINE: usize = 240;

#[repr(C)]
pub struct SuiseiSearchHitsSnapshot {
    pub count: u32,
    /// Set when the result set hit `SUISEI_MAX_HITS` and was cut short.
    pub truncated: u8,
    pub rows: [u32; SUISEI_MAX_HITS],
    pub cols: [u32; SUISEI_MAX_HITS],
    pub paths: [[c_char; SUISEI_HIT_PATH]; SUISEI_MAX_HITS],
    pub lines: [[c_char; SUISEI_HIT_LINE]; SUISEI_MAX_HITS],
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_search_project(
    root: *const c_char,
    pattern: *const c_char,
    out: *mut SuiseiSearchHitsSnapshot,
) -> u8 {
    if root.is_null() || pattern.is_null() || out.is_null() {
        return 0;
    }
    let Ok(root) = (unsafe { CStr::from_ptr(root) }).to_str() else {
        return 0;
    };
    let Ok(pattern) = (unsafe { CStr::from_ptr(pattern) }).to_str() else {
        return 0;
    };
    if pattern.is_empty() {
        return 0;
    }
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiSearchHitsSnapshot>());
    }
    let o = unsafe { &mut *out };
    // One over the cap, so a full page can be told apart from an exact fit.
    let hits = suisei_core::workspace_search::search_project(
        std::path::Path::new(root),
        pattern,
        SUISEI_MAX_HITS + 1,
    );
    o.truncated = u8::from(hits.len() > SUISEI_MAX_HITS);
    let n = hits.len().min(SUISEI_MAX_HITS);
    o.count = n as u32;
    for (i, h) in hits.iter().take(n).enumerate() {
        o.rows[i] = h.row as u32;
        o.cols[i] = h.col as u32;
        write_cstr(&mut o.paths[i], &h.path.to_string_lossy());
        // `write_cstr` already caps the copy. Slicing by BYTE index first —
        // `&line[..n]` — panics the moment `n` lands inside a multi-byte
        // character, and a panic across the FFI takes the app with it.
        write_cstr(&mut o.lines[i], h.line.trim());
    }
    1
}

// ---------------------------------------------------------------------------
// Find All References — LSP textDocument/references.
//
// Asynchronous like hover: `request` posts to the server, then the face polls
// `references` until `ready` flips. Same list shape as project search (rows +
// cols + paths + a source-line preview), so the face can reuse that panel.
// ---------------------------------------------------------------------------

pub const SUISEI_MAX_REFS: usize = 500;
pub const SUISEI_REF_PATH: usize = 512;
pub const SUISEI_REF_LINE: usize = 240;

#[repr(C)]
pub struct SuiseiReferencesSnapshot {
    pub count: u32,
    /// 1 once the LSP has answered (so 0 references reads as "done", not "wait").
    pub ready: u8,
    /// Set when the result set hit `SUISEI_MAX_REFS` and was cut short.
    pub truncated: u8,
    pub _pad0: u8,
    pub _pad1: u8,
    pub rows: [u32; SUISEI_MAX_REFS],
    pub cols: [u32; SUISEI_MAX_REFS],
    pub paths: [[c_char; SUISEI_REF_PATH]; SUISEI_MAX_REFS],
    pub lines: [[c_char; SUISEI_REF_LINE]; SUISEI_MAX_REFS],
}

/// Ask the language server for all references to the symbol under the cursor.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_request_references(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.app_mut().request_references();
        (*ptr).0.recompose_paint_only();
    }
}

/// Poll the references result. `ready == 0` means the server has not answered
/// yet; the face should keep polling. Returns 1 unless the args are null.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_references(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiReferencesSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let (refs, ready) = eng.0.app.references_result();
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiReferencesSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.ready = u8::from(ready);
    o.truncated = u8::from(refs.len() > SUISEI_MAX_REFS);
    let n = refs.len().min(SUISEI_MAX_REFS);
    o.count = n as u32;
    for (i, (loc, preview)) in refs.iter().take(n).enumerate() {
        o.rows[i] = loc.row as u32;
        o.cols[i] = loc.col as u32;
        write_cstr(&mut o.paths[i], &loc.path);
        write_cstr(&mut o.lines[i], preview);
    }
    1
}

// ---------------------------------------------------------------------------
// Quick Help inspector — LSP hover.
//
// `request_hover` is asynchronous: it posts to the language server and the
// answer lands in `app.hover_text` a round trip later. Two calls rather than
// one, so the face can ask and then poll without blocking a frame on the LSP.
// ---------------------------------------------------------------------------

pub const SUISEI_HOVER_TEXT: usize = 4096;

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_request_hover(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.app_mut().request_hover();
    }
}

/// Writes at most `SUISEI_HOVER_TEXT` bytes. Returns 0 when nothing has
/// arrived yet, which is the normal state for the frame right after asking.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_hover_text(
    ptr: *const SuiseiEngine,
    out: *mut c_char,
    cap: u32,
) -> u8 {
    if ptr.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(text) = eng.0.app().hover_text.as_ref() else {
        return 0;
    };
    let dst = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    write_cstr(dst, text);
    1
}

// ---------------------------------------------------------------------------
// LSP face surfaces — thin wrappers over existing App methods.
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_format_document(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.app_mut().format_document();
        (*ptr).0.recompose_paint_only();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_goto_definition(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.app_mut().goto_definition();
        (*ptr).0.recompose_paint_only();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_rename_symbol(ptr: *mut SuiseiEngine, new_name: *const c_char) {
    if ptr.is_null() || new_name.is_null() {
        return;
    }
    let name = unsafe { CStr::from_ptr(new_name) }.to_string_lossy();
    unsafe {
        (*ptr).0.app_mut().rename_symbol(&name);
        (*ptr).0.recompose_paint_only();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_code_actions(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.app_mut().request_code_actions();
        (*ptr).0.recompose_paint_only();
    }
}

// ---------------------------------------------------------------------------
// Project Find replace (freestanding — safe off the main engine lock).
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_replace_in_file(
    path: *const c_char,
    row: u32,
    query: *const c_char,
    replace: *const c_char,
) -> u8 {
    if path.is_null() || query.is_null() || replace.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }.to_string_lossy();
    let query = unsafe { CStr::from_ptr(query) }.to_string_lossy();
    let replace = unsafe { CStr::from_ptr(replace) }.to_string_lossy();
    match suisei_core::workspace_search::replace_in_file(
        std::path::Path::new(path.as_ref()),
        row as usize,
        query.as_ref(),
        replace.as_ref(),
    ) {
        Ok(true) => 1,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_replace_all_in_file(
    path: *const c_char,
    query: *const c_char,
    replace: *const c_char,
) -> u32 {
    if path.is_null() || query.is_null() || replace.is_null() {
        return 0;
    }
    let path = unsafe { CStr::from_ptr(path) }.to_string_lossy();
    let query = unsafe { CStr::from_ptr(query) }.to_string_lossy();
    let replace = unsafe { CStr::from_ptr(replace) }.to_string_lossy();
    suisei_core::workspace_search::replace_all_in_file(
        std::path::Path::new(path.as_ref()),
        query.as_ref(),
        replace.as_ref(),
    )
    .unwrap_or(0) as u32
}

// ─── Shadow WAL recovery (D0) ─────────────────────────────────────────────────

/// Number of pending crash-recovery entries found on startup.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_recovery_count(ptr: *const SuiseiEngine) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.journal.recovery_count() as u32 }
}

/// Get the file path of recovery entry `idx`.
/// Writes a NUL-terminated UTF-8 string into `buf` (max `buf_len` bytes).
/// Returns 1 on success, 0 if idx is out of range.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_recovery_path(
    ptr: *const SuiseiEngine,
    idx: u32,
    buf: *mut c_char,
    buf_len: u32,
) -> u8 {
    if ptr.is_null() || buf.is_null() || buf_len == 0 {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(entry) = eng.0.journal.recovery_entry(idx as usize) else {
        return 0;
    };
    let bytes = entry.file_path.as_bytes();
    let n = bytes.len().min((buf_len - 1) as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    1
}

/// Accept recovery entry `idx`: open the file from disk, replace buffer with
/// the journaled text, restore cursor/scroll, mark as modified (unsaved).
/// Returns 1 on success, 0 if idx is out of range.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_recovery_accept(ptr: *mut SuiseiEngine, idx: u32) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    let eng = unsafe { &mut *ptr };
    let Some(entry) = eng.0.journal.accept_recovery(idx as usize) else {
        return 0;
    };
    // Open the file from disk first (sets up syntax, explorer, LSP, tabs).
    eng.0.app.open_new_tab(&entry.file_path);
    // Replace buffer content with the recovered (unsaved) text.
    eng.0.app.buffer = suisei_core::buffer::Buffer::from_string(&entry.text);
    eng.0.app.buffer.cursor.row = (entry.cursor_row as usize)
        .min(eng.0.app.buffer.line_count().saturating_sub(1));
    // Clamp col too (row is already clamped): a shorter recovered line would
    // otherwise leave the caret past the end and panic on the next edit/paint.
    let line_len = eng
        .0
        .app
        .buffer
        .line(eng.0.app.buffer.cursor.row)
        .chars()
        .count();
    eng.0.app.buffer.cursor.col = (entry.cursor_col as usize).min(line_len);
    eng.0.app.scroll = entry.scroll as usize;
    eng.0.app.modified = true;
    eng.0.app.message = format!("Recovered unsaved changes: {}", entry.file_path);
    eng.0.recompose();
    1
}

/// Discard recovery entry `idx` (user chose not to recover). Deletes the WAL file.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_recovery_discard(ptr: *mut SuiseiEngine, idx: u32) {
    if ptr.is_null() {
        return;
    }
    let eng = unsafe { &mut *ptr };
    eng.0.journal.discard_recovery(idx as usize);
}
