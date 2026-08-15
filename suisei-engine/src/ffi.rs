//! C ABI for the Swift face. Fixed buffers — no pointer lifetime traps.

use std::ffi::{CStr, c_char};

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

// Panel bits for `suisei_engine_open_panels`. Mirrored in suisei_engine.h and
// asserted against it in tests/abi_layout.rs.
pub const SUISEI_PANEL_EXPLORER: u32 = 1 << 0;
pub const SUISEI_PANEL_PALETTE: u32 = 1 << 1;
pub const SUISEI_PANEL_SEARCH: u32 = 1 << 2;
pub const SUISEI_PANEL_COMPLETIONS: u32 = 1 << 3;
pub const SUISEI_PANEL_TERMINAL: u32 = 1 << 4;
pub const SUISEI_PANEL_SETTINGS: u32 = 1 << 5;
pub const SUISEI_PANEL_SCM: u32 = 1 << 6;
pub const SUISEI_PANEL_GIT_WB: u32 = 1 << 7;
pub const SUISEI_PANEL_PREVIEW: u32 = 1 << 8;
pub const SUISEI_PANEL_OUTLINE: u32 = 1 << 9;

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
    /// `suisei_core::media::FileKind` as its discriminant — what the face
    /// should draw here. Was a plain `is_terminal` bool in the same byte, and
    /// `Terminal == 1` keeps that wire value: a face that has never heard of
    /// the other kinds still reads terminals correctly.
    pub kind: u8,
    /// Pane shell content generation — bumps when the grid changes, so the
    /// face skips re-pulling a ~300 KiB snapshot it already has. Reuses the
    /// two pad bytes: no size change, so the pane stride stays put.
    pub term_gen: u16,
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
    /// Gutter counts from the caret. Was `_pad_h0` — the same trick
    /// `SuiseiPaneC::kind` played on its pad byte, so no offset moves.
    pub relative_number: u8,
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
    /// 1 when the tab is a terminal (a shell runs in it).
    pub tab_is_terminal: [u8; SUISEI_MAX_TABS],
    /// Retired: the split shape lives in the per-pane rects (`SuiseiPaneC`).
    /// Kept as pads — renamed, not removed — so every offset after this
    /// block holds.
    pub _pad_split_kind: u8,
    pub pane_count: u8,
    pub pane_focus: u8,
    pub _pad_split: u8,
    pub _pad_split_ratio: f32,
    pub panes: [SuiseiPaneC; SUISEI_MAX_PANES],
    pub visible_line_count: u32,
    pub _pad_vis: u32,
    // The packed `lines: [SuiseiEditorLineC; SUISEI_MAX_LINES]` array used to
    // sit here. It was 176,128 of this struct's 185,440 bytes — 95% — and the
    // face never decoded a byte of it: the GUI is a PULL renderer, so every
    // canvas fetches its own rows through `suisei_engine_editor_band` on draw
    // (`EngineBridge.decodeEditorLinesAndSplit` says so, and hard-codes
    // `allLines = []`). Carrying it cost a 181 KiB memset here plus another on
    // the Swift side, per refresh, twenty times a second.
    //
    // `SuiseiPaneC::line_start` / `line_count` are now always 0 and stay only
    // because the face reads the pane struct at hardcoded byte offsets.
    /// Actual per-pane document titles. Unlike `tab_titles`, this does not
    /// collapse buffer identity when a layout uses one unified strip chip.
    pub pane_titles: [[c_char; SUISEI_TITLE_CAP]; SUISEI_MAX_PANES],
}

pub const SUISEI_GH_STATE_MISSING: u8 = 0;
pub const SUISEI_GH_STATE_OUT: u8 = 1;
pub const SUISEI_GH_STATE_IN: u8 = 2;
pub const SUISEI_GH_NAME_CAP: usize = 96;
pub const SUISEI_GH_URL_CAP: usize = 256;
pub const SUISEI_GH_HOST_CAP: usize = 64;
pub const SUISEI_GH_CODE_CAP: usize = 32;
pub const SUISEI_GH_CONTRIB_DAYS: usize = 371;

#[repr(C)]
pub struct SuiseiGitHubAccount {
    pub generation: u64,
    pub state: u8,
    pub loading: u8,
    pub signing_in: u8,
    pub _pad: u8,
    pub public_repos: u32,
    pub followers: u32,
    pub following: u32,
    pub user: [c_char; SUISEI_GH_NAME_CAP],
    pub name: [c_char; SUISEI_GH_NAME_CAP],
    pub email: [c_char; SUISEI_GH_NAME_CAP],
    pub avatar_url: [c_char; SUISEI_GH_URL_CAP],
    pub bio: [c_char; SUISEI_GH_URL_CAP],
    pub company: [c_char; SUISEI_GH_NAME_CAP],
    pub location: [c_char; SUISEI_GH_NAME_CAP],
    pub html_url: [c_char; SUISEI_GH_URL_CAP],
    pub host: [c_char; SUISEI_GH_HOST_CAP],
    pub protocol: [c_char; 24],
    pub scopes: [c_char; SUISEI_GH_URL_CAP],
    pub token_source: [c_char; SUISEI_GH_HOST_CAP],
    pub device_code: [c_char; SUISEI_GH_CODE_CAP],
    pub message: [c_char; SUISEI_MSG_CAP],
    pub contrib_total: u32,
    pub contrib_days: u16,
    pub _contrib_pad: u16,
    pub contrib_levels: [u8; SUISEI_GH_CONTRIB_DAYS],
    pub contrib_start: [c_char; 12],
    pub contrib_year: u32,
    pub contrib_year_min: u32,
}

pub const SUISEI_UPDATE_NOTES_CAP: usize = 512;

#[repr(C)]
pub struct SuiseiUpdateSnapshot {
    pub generation: u64,
    pub available: u8,
    pub installing: u8,
    pub installed: u8,
    pub checking: u8,
    pub current: [c_char; 64],
    pub latest: [c_char; 64],
    pub notes: [c_char; SUISEI_UPDATE_NOTES_CAP],
}

/// `write_cstr` for a caller-owned buffer described by pointer and capacity.
///
/// # Safety
/// `dst` must be valid for writes of `cap` bytes.
unsafe fn write_cstr_raw(dst: *mut c_char, cap: usize, s: &str) {
    if dst.is_null() || cap == 0 {
        return;
    }
    let slice = unsafe { std::slice::from_raw_parts_mut(dst, cap) };
    write_cstr(slice, s);
}

fn write_cstr(dst: &mut [c_char], s: &str) {
    dst.fill(0);
    // Truncate on a char boundary: a mid-UTF-8 cut used to hand the face an
    // invalid C string, which String(cString:) rendered as replacement-char
    // garbage at the end of dense CJK / emoji lines.
    let max = dst.len().saturating_sub(1);
    let bytes = s.as_bytes();
    let mut n = bytes.len().min(max);
    while n > 0 && !s.is_char_boundary(n) {
        n -= 1;
    }
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
    let mut char_col = 0usize;
    for ch in line.chars() {
        let next = seen_u16.saturating_add(ch.len_utf16());
        if next > utf16_off as usize {
            break;
        }
        seen_u16 = next;
        char_col += 1;
    }
    // Delegate width policy to Buffer. In particular, combining marks are
    // width zero; the previous `.max(1)` invented a cell for every jamo/accent
    // and made drag-selection drift from what CoreText drew.
    buf.buffer_col_to_screen_col(row, char_col) as u32
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
pub extern "C" fn suisei_engine_prewarm_file(ptr: *mut SuiseiEngine, path: *const c_char) -> u8 {
    if ptr.is_null() || path.is_null() {
        return 0;
    }
    let Ok(p) = (unsafe { CStr::from_ptr(path) }).to_str() else {
        return 0;
    };
    unsafe { u8::from((*ptr).0.prewarm_file(p)) }
}

/// Boot pipeline: warm every language grammar on the syntax worker so the
/// first file opened highlights with no cold parser/query build.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_warm_grammars(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.warm_grammars() }
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

/// Which chrome panels are open, as a bitmask — see `SUISEI_PANEL_*`.
///
/// Exists because every panel snapshot is a fixed-size struct (the terminal's
/// is 300 KiB, the git workbench's 55 KiB, diagnostics' 49 KiB), and the face
/// used to copy all of them on every refresh and only *then* check whether the
/// panel was open, discarding the copy if it was not. Four bytes up front lets
/// it skip the copy entirely. See `docs/SUISEI-GPU-ARCHITECTURE.md` §2.
///
/// Answered straight out of the last composed frame, so it costs a few loads
/// and no allocation.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_open_panels(ptr: *const SuiseiEngine) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    let Some(c) = eng.0.last_diff.chrome.as_ref() else {
        return 0;
    };
    let bit = |on: bool, mask: u32| if on { mask } else { 0 };
    // Explorer: "is there anything to pull", NOT "does it own the keyboard".
    // The project navigator is docked — it paints its entries whenever it has
    // them, and `explorer.open` only means Core is routing keys to it. Gating
    // the pull on the focus flag would blank the tree in Normal mode.
    bit(
        c.explorer.open || c.explorer_open || !c.explorer.entries.is_empty(),
        SUISEI_PANEL_EXPLORER,
    )
        | bit(c.palette.open, SUISEI_PANEL_PALETTE)
        | bit(c.search.open, SUISEI_PANEL_SEARCH)
        | bit(c.completions.open, SUISEI_PANEL_COMPLETIONS)
        | bit(c.terminal.open, SUISEI_PANEL_TERMINAL)
        | bit(c.settings.open, SUISEI_PANEL_SETTINGS)
        | bit(c.scm.open, SUISEI_PANEL_SCM)
        | bit(c.git_wb.open, SUISEI_PANEL_GIT_WB)
        | bit(c.preview.open, SUISEI_PANEL_PREVIEW)
        // The outline feeds the docked inspector, which has no `open` flag of
        // its own — it is empty exactly when there is nothing to show, and an
        // empty pull is the cheap case anyway.
        | bit(!c.outline.is_empty(), SUISEI_PANEL_OUTLINE)
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
    unsafe {
        u8::from(matches!(
            (*ptr).0.app().mode,
            suisei_core::app::Mode::Editor
        ))
    }
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
    o.relative_number = chrome.relative_number;
    o._pad_h1 = 0;
    o._pad_h2 = 0;
    o.buffer_version = chrome.buffer_version;
    // branch packed into message suffix is avoided — use separate FFI below

    let tab_n = chrome.tabs.len().min(SUISEI_MAX_TABS);
    // The TRUE count, not the clamped one: tabs past the cap used to vanish
    // without a trace. The face clamps its own decode loop and shows "+N".
    o.tab_count = chrome.tabs.len() as u32;
    o.tab_active = chrome.tabs.iter().position(|t| t.active).unwrap_or(0) as u32;
    for (i, tab) in chrome.tabs.iter().take(tab_n).enumerate() {
        // bit 0 = dirty, bit 1 = deleted-on-disk. Packed into the existing byte
        // so the fixed C-ABI snapshot layout is untouched.
        o.tab_dirty[i] = u8::from(tab.dirty) | (u8::from(tab.deleted) << 1);
        o.tab_ids[i] = tab.id;
        o.tab_groups[i] = tab.group;
        o.tab_is_layout[i] = u8::from(tab.is_layout);
        o.tab_is_terminal[i] = u8::from(tab.is_terminal);
        write_cstr(&mut o.tab_titles[i], &tab.title);
    }

    // Split metadata. `split_kind`/`split_ratio` are gone: the shape lives in
    // the per-pane rects. The ABI bytes stay as pads so every offset after them
    // holds.
    //
    // The packed line stream is gone too — see the note on the struct. Only the
    // COUNTS are still reported, because they cost four bytes and describe the
    // scene without carrying it.
    o.pane_focus = chrome.pane_focus;
    let pane_n = chrome.panes.len().min(SUISEI_MAX_PANES);
    o.pane_count = pane_n as u8;

    if chrome.panes.is_empty() {
        // Unsplit: one synthesised pane covering the whole editor.
        let line_n = chrome.lines.len().min(SUISEI_MAX_LINES);
        o.visible_line_count = line_n as u32;
        o.pane_count = 1;
        o.panes[0] = SuiseiPaneC {
            tab_index: o.tab_active,
            scroll: chrome.scroll,
            line_start: 0,
            line_count: line_n as u32,
            focused: 1,
            kind: chrome.pane0_kind as u8,
            // Was a pane shell's content generation, so the face could skip
            // re-pulling a grid it already had. Nothing pulls a grid.
            term_gen: 0,
            doc_line_count: chrome.line_count,
            hscroll: chrome.hscroll,
            // Unsplit: the one pane is the whole editor.
            rect_x: 0.0,
            rect_y: 0.0,
            rect_w: 1.0,
            rect_h: 1.0,
        };
        write_cstr(&mut o.pane_titles[0], &chrome.pane0_title);
    } else {
        let mut packed = 0usize;
        for (pi, pane) in chrome.panes.iter().take(pane_n).enumerate() {
            let start = packed as u32;
            let take = pane
                .lines
                .len()
                .min(SUISEI_MAX_LINES.saturating_sub(packed));
            packed += take;
            o.panes[pi] = SuiseiPaneC {
                tab_index: pane.tab_index,
                scroll: pane.scroll,
                line_start: start,
                line_count: take as u32,
                focused: u8::from(pane.focused),
                // Reuses a pad byte — no size change, so the pane stride and
                // every offset after it stay put.
                kind: pane.kind as u8,
                term_gen: 0,
                rect_x: pane.rect.x,
                rect_y: pane.rect.y,
                rect_w: pane.rect.w,
                rect_h: pane.rect.h,
                doc_line_count: pane.doc_line_count,
                hscroll: pane.hscroll,
            };
            write_cstr(&mut o.pane_titles[pi], &pane.title);
        }
        o.visible_line_count = packed.min(SUISEI_MAX_LINES) as u32;
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
    let vp = engine.0.app.stage;
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
            replace_project(engine, &path_buf, s, vp);
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
        next.stage = vp;
        next.message = format!("Opened {}", s);
        if let Some(parent) = path_buf.parent() {
            next.explorer.cwd = parent.to_path_buf();
            next.explorer.refresh();
            next.explorer.open = true;
        }
        engine.0.app = next;
    }
    // The stage rode along with `next` (or never left); geometry needs no
    // re-sync — A6 made the pixel stage the single source.
    engine.0.update_scroll_public();
    engine.0.recompose();
    1
}

/// Replace the current workspace with a directory.
///
/// Return codes: 1 switched, 2 refused because at least one tab is dirty,
/// 0 invalid input. A project switch is allowed to discard clean tabs, never
/// unsaved edits. Opening an individual file still uses `open_path` and adds a
/// tab to the current workspace.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_switch_project(ptr: *mut SuiseiEngine, path: *const c_char) -> u8 {
    if ptr.is_null() || path.is_null() {
        return 0;
    }
    let cstr = unsafe { CStr::from_ptr(path) };
    let Ok(s) = cstr.to_str() else {
        return 0;
    };
    let path_buf = std::path::PathBuf::from(s);
    if !path_buf.is_dir() {
        return 0;
    }
    let engine = unsafe { &mut *ptr };
    if app_has_dirty_tabs(&engine.0.app) {
        return 2;
    }
    let vp = engine.0.app.stage;
    replace_project(engine, &path_buf, s, vp);
    engine.0.update_scroll_public();
    engine.0.recompose();
    1
}

fn replace_project(
    engine: &mut SuiseiEngine,
    path: &std::path::Path,
    display: &str,
    viewport: suisei_core::app::Stage,
) {
    // Leave Welcome (must set filename or open a file).
    let first = first_project_file(path);
    let mut next = if let Some(ref file) = first {
        suisei_core::app::App::open_file(&file.display().to_string())
    } else {
        let mut app = suisei_core::app::App::default();
        app.apply_config();
        app.filename = Some(path.join("Untitled"));
        app.message = format!("Opened folder {display}");
        app
    };
    next.stage = viewport;
    next.explorer.cwd = path.to_path_buf();
    next.explorer.refresh();
    next.explorer.open = true;
    if first.is_some() {
        next.message = format!("Opened project {display}");
    }
    engine.0.app = next;
}

/// True once the user has left cold Welcome (any real buffer / tree / multi-tab).
fn app_has_editor_session(app: &suisei_core::app::App) -> bool {
    if app.tabs.buffers.len() > 1 {
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

fn app_has_dirty_tabs(app: &suisei_core::app::App) -> bool {
    app.modified || app.tabs.buffers.iter().any(|tab| tab.modified)
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
pub extern "C" fn suisei_engine_drag(ptr: *mut SuiseiEngine, buffer_row: u32, visual_col: u32) {
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

/// Current native floating-chrome material. This is intentionally a tiny
/// getter rather than another field in the large paint snapshot: changing the
/// material invalidates SwiftUI chrome, not the editor's retained scene.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_glass_style(ptr: *const SuiseiEngine) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    u8::from(unsafe { &*ptr }.0.app().glass_style == "tinted")
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

/// Where the caret is: 1-based row in the high 32 bits, visual column in the
/// low 32. Zero when there is no engine.
///
/// The same two numbers `SuiseiChromeSnapshot` carries — and that is the point.
/// The typing fast path (`suisei_engine_gui_type_char`) deliberately publishes
/// no chrome: the face's canvas is a pull renderer and repaints itself from the
/// engine, so a 180 KiB snapshot per keystroke would be pure waste. But the
/// face also has to scroll the caret into view on every keystroke, and the only
/// copy of the caret it had was the one inside that snapshot — so while the
/// user typed continuously the scroll never moved at all.
///
/// The visual column, not the buffer column: a tab is one character and several
/// cells, and this is used to place a caret on screen.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_caret_row_vcol(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    let app = unsafe { (*ptr).0.app() };
    let cursor = app.buffer.cursor();
    let row = cursor.row.saturating_add(1) as u64;
    let vcol = crate::compositor::visual_col(
        app.buffer.line(cursor.row),
        crate::compositor::drawn_caret_col(app),
        app.tab_width,
    ) as u64;
    (row << 32) | (vcol & 0xFFFF_FFFF)
}

/// Absolute UTF-16 caret offset in the focused document.
///
/// AppKit's NSTextInputClient ranges are document offsets, not line-local
/// columns. Exposing this keeps IME composition anchored when it starts in the
/// middle of a Hangul/CJK line.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_caret_utf16_offset(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    let app = unsafe { (*ptr).0.app() };
    let caret = app.buffer.cursor();
    let mut offset = 0u64;
    for row in 0..caret.row.min(app.buffer.line_count()) {
        offset = offset
            .saturating_add(app.buffer.line(row).encode_utf16().count() as u64)
            .saturating_add(1); // document newline
    }
    offset.saturating_add(
        app.buffer
            .line(caret.row)
            .chars()
            .take(caret.col)
            .map(|ch| ch.len_utf16() as u64)
            .sum::<u64>(),
    )
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

/// Replace the native GUI find field's full value.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_find_set_input(ptr: *mut SuiseiEngine, input: *const c_char) {
    if ptr.is_null() || input.is_null() {
        return;
    }
    let input = unsafe { CStr::from_ptr(input) }.to_string_lossy();
    unsafe {
        (*ptr).0.find_set_input(&input);
    }
}

/// Accept the native find field without routing a generic Return key.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_find_accept(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.find_accept();
    }
}

/// Cancel the native find field without routing a generic Escape key.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_find_cancel(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.find_cancel();
    }
}

/// Replace the native GUI palette field's full value.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_palette_set_query(ptr: *mut SuiseiEngine, query: *const c_char) {
    if ptr.is_null() || query.is_null() {
        return;
    }
    let query = unsafe { CStr::from_ptr(query) }.to_string_lossy();
    unsafe {
        (*ptr).0.palette_set_query(&query);
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
pub extern "C" fn suisei_engine_activate_layout(
    ptr: *mut SuiseiEngine,
    id: u64,
    focus_doc: u64,
) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { u8::from((*ptr).0.activate_layout(id, focus_doc)) }
}

/// Switch a layout between grouped and unified strip shapes.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_toggle_layout_style(ptr: *mut SuiseiEngine, id: u64) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { u8::from((*ptr).0.toggle_layout_style(id)) }
}

/// Layout id that currently owns the desk, or 0 when none (free split / single).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_active_layout_id(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.active_layout_id() }
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

/// Toggle the pretty document preview, without going through the key path.
///
/// The menu item used to simulate ⇧⌘V. That chord means "pretty preview" only
/// while the editor holds focus; in a terminal pane the same chord is "paste
/// the clipboard into the shell", so a menu item labelled Pretty Preview could
/// paste into a running process. A menu item names an action, not a keystroke.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_toggle_preview(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.toggle_preview();
    }
}


/// Open a full terminal TAB, or close it when one is already focused.
///
/// Same reason as above: the menu item simulated ⇧⌘T, which the terminal pane
/// handles on a different branch from the editor.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_toggle_terminal_tab(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.toggle_terminal_tab();
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
    o.selected = ex.entries.iter().position(|e| e.selected).unwrap_or(0) as u32;
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
    wrap_cols: u16,
    wide_ratio: u16,
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
    let (lines, total) =
        eng.0
            .editor_band(pane as usize, start_row as usize, rows, wrap_cols, wide_ratio);
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

/// Stage (0), unstage (1) or discard (2) the one change covering a line.
///
/// Addressed by LINE rather than by hunk index: the caller is a click in the
/// gutter, and a line is what a click has. An index would be a second way to
/// name the same change, and it would go stale the moment the file was
/// re-diffed between the click and the call.
///
/// Returns 0 on success. The message — success or failure — is on `chrome`.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_apply_hunk(
    ptr: *mut SuiseiEngine,
    line_1based: u32,
    action: u8,
) -> i32 {
    if ptr.is_null() {
        return -1;
    }
    let action = match action {
        0 => suisei_core::git::HunkAction::Stage,
        1 => suisei_core::git::HunkAction::Unstage,
        2 => suisei_core::git::HunkAction::Discard,
        _ => return -1,
    };
    unsafe { (*ptr).0.apply_gutter_hunk(line_1based, action) }
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
pub extern "C" fn suisei_engine_minimap(ptr: *const SuiseiEngine, out: *mut SuiseiMinimapC) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiMinimapC>());
    }
    let o = unsafe { &mut *out };
    let (buckets, total) = eng.0.minimap(SUISEI_MINIMAP_MAX);
    write_minimap(o, buckets, total)
}

/// The minimap of the document in pane `idx`.
///
/// `suisei_engine_minimap` answers for the live document, which is the focused
/// pane's. With a strip in every pane that made every strip the focused file.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_minimap_for_pane(
    ptr: *const SuiseiEngine,
    idx: u32,
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
    let (buckets, total) = eng.0.minimap_of_pane(idx as usize, SUISEI_MINIMAP_MAX);
    write_minimap(o, buckets, total)
}

fn write_minimap(o: &mut SuiseiMinimapC, buckets: Vec<(u8, u8, u8)>, total: u32) -> u8 {
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

/// Soft-wrap geometry for one pane's document at `cols` columns. `cols == 0`
/// is "not wrapping", and every one of these answers as if each line were one
/// row — so the face has one code path either way.
///
/// The COLUMNS are the face's number. Only it knows a pane's width in points,
/// the cell width, the gutter and what overlays the right edge; core knows what
/// a line measures. The map is cached per pane against the document version, so
/// asking three times a frame builds nothing.

/// Total visual rows — the document's height in rows, which is the scroll
/// extent once the face multiplies by its line height.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_wrap_total_rows(
    ptr: *const SuiseiEngine,
    pane: u32,
    cols: u16,
    wide_ratio: u16,
) -> u32 {
    if ptr.is_null() {
        return 1;
    }
    unsafe { &*ptr }.0.wrap_total_rows(pane as usize, cols, wide_ratio)
}

/// First visual row of a buffer row.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_wrap_visual_of(
    ptr: *const SuiseiEngine,
    pane: u32,
    cols: u16,
    wide_ratio: u16,
    row: u32,
) -> u32 {
    if ptr.is_null() {
        return row;
    }
    unsafe { &*ptr }
        .0
        .wrap_visual_of(pane as usize, cols, wide_ratio, row as usize)
}

/// Buffer row in the high 32 bits, segment within it in the low 32 — the
/// inverse of `suisei_engine_wrap_visual_of`, for turning a click or a
/// viewport top back into a line.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_wrap_buffer_at(
    ptr: *const SuiseiEngine,
    pane: u32,
    cols: u16,
    wide_ratio: u16,
    visual_row: u32,
) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    let (row, seg) =
        unsafe { &*ptr }
            .0
            .wrap_buffer_at(pane as usize, cols, wide_ratio, visual_row);
    ((row as u64) << 32) | (seg as u64 & 0xFFFF_FFFF)
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
    o.selected = p.items.iter().position(|i| i.selected).unwrap_or(0) as u32;
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

/// Activate the tab holding stable id `id` (`BufferTab::id`). Strip slots are
/// not buffer indices once a folded layout gathers or hides members, so the
/// face addresses chips by id.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_goto_tab_id(ptr: *mut SuiseiEngine, id: u64) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.goto_tab_id(id);
    }
}

/// Close the tab holding stable id `id`.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_close_tab_id(ptr: *mut SuiseiEngine, id: u64) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.close_tab_id(id);
    }
}

/// Reorder: move the tab holding `from` onto the strip position of `to`,
/// both by stable id. Returns 1 on success (refused when the move would
/// break a folded group's contiguity).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_move_tab_ids(ptr: *mut SuiseiEngine, from: u64, to: u64) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { u8::from((*ptr).0.move_tab_ids(from, to)) }
}

/// Drop a layout tab by its id ("Close Tab" on a layout chip). Documents stay
/// open as loose tabs. Returns 1 on success.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_drop_layout(ptr: *mut SuiseiEngine, id: u64) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { u8::from((*ptr).0.drop_layout(id)) }
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
pub extern "C" fn suisei_engine_split_above(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.split_above();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_split_left(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.split_left();
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

#[repr(C)]
pub struct SuiseiCompletionsSnapshot {
    pub open: u8,
    pub selected: u32,
    pub count: u32,
    pub prefix: [c_char; 64],
    pub labels: [[c_char; SUISEI_COMP_LABEL]; SUISEI_MAX_COMP],
    pub details: [[c_char; SUISEI_COMP_LABEL]; SUISEI_MAX_COMP],
}

// The 300 KiB `SuiseiTerminalSnapshot` used to live here — 200 rows × 1536
// bytes of truecolor SGR, re-encoded from a cell grid on one side of the ABI
// and re-parsed on the other, pulled on every refresh while a terminal was
// open. Nothing draws from it: every terminal in the window is a SwiftTerm
// view that reads its own PTY.

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
pub const SUISEI_SETTINGS_GROUP: usize = 48;
pub const SUISEI_SETTINGS_DETAIL: usize = 192;
pub const SUISEI_SETTINGS_OPTIONS: usize = 96;
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
    /// What each row IS (`SettingRow::kind`), so the face can branch on the
    /// row's identity rather than parse its label. 0 = prose, no setting.
    pub row_kind: [u32; SUISEI_MAX_SETTINGS_ROWS],
    /// Which theme / which language, for the indexed kinds.
    pub row_payload: [u32; SUISEI_MAX_SETTINGS_ROWS],
    pub row_page: [u32; SUISEI_MAX_SETTINGS_ROWS],
    pub row_control: [u32; SUISEI_MAX_SETTINGS_ROWS],
    pub row_value_index: [u32; SUISEI_MAX_SETTINGS_ROWS],
    pub row_advanced: [u8; SUISEI_MAX_SETTINGS_ROWS],
    pub row_groups: [[c_char; SUISEI_SETTINGS_GROUP]; SUISEI_MAX_SETTINGS_ROWS],
    pub row_details: [[c_char; SUISEI_SETTINGS_DETAIL]; SUISEI_MAX_SETTINGS_ROWS],
    pub row_options: [[c_char; SUISEI_SETTINGS_OPTIONS]; SUISEI_MAX_SETTINGS_ROWS],
    pub row_labels: [[c_char; SUISEI_SETTINGS_LABEL]; SUISEI_MAX_SETTINGS_ROWS],
    pub row_values: [[c_char; SUISEI_SETTINGS_VALUE]; SUISEI_MAX_SETTINGS_ROWS],
}

#[repr(C)]
pub struct SuiseiThemeSnapshot {
    pub name: [c_char; 32],
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
        o.row_kind[i] = r.kind;
        o.row_payload[i] = r.payload;
        o.row_page[i] = r.page;
        o.row_control[i] = r.control;
        o.row_value_index[i] = r.value_index;
        o.row_advanced[i] = u8::from(r.advanced);
        write_cstr(&mut o.row_groups[i], &r.group);
        write_cstr(&mut o.row_details[i], &r.detail);
        write_cstr(&mut o.row_options[i], &r.options);
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
    o.current_line = t.current_line;
    o.invisibles = t.invisibles;
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
    o.window_bg = t.window_bg;
    o.border = t.border;
    o.panel_bg = t.panel_bg;
    o.panel_border = t.panel_border;
    o.panel_sel_bg = t.panel_sel_bg;
    o.panel_sel_fg = t.panel_sel_fg;
    o.explorer_bg = t.explorer_bg;
    o.explorer_fg = t.explorer_fg;
    o.explorer_selected = t.explorer_selected;
    o.status_fg = t.status_fg;
    o.muted = t.muted;
    o.success = t.success;
    o.warning = t.warning;
    o.error = t.error;
    o.accent_fg = t.accent_fg;
    o.search_bg = t.search_bg;
    o.completion_bg = t.completion_bg;
    o.completion_selected = t.completion_selected;
    o.completion_border = t.completion_border;
    o.terminal_bg = t.terminal_bg;
    o.git_add_bg = t.git_add_bg;
    o.git_del_bg = t.git_del_bg;
    o.git_hunk = t.git_hunk;
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
pub extern "C" fn suisei_engine_settings_set_value(ptr: *mut SuiseiEngine, row: u32, value: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.settings_set_value(row, value);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_set_highlight_color(
    ptr: *mut SuiseiEngine,
    value: *const c_char,
) {
    if ptr.is_null() || value.is_null() {
        return;
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    unsafe {
        (*ptr).0.settings_set_highlight_color(&value);
    }
}

/// The addressable theme colours, as `key|Label` one per line, in the order
/// their indices run.
///
/// One call rather than twenty: the face needs the whole table once, at build
/// time of its Themes page, and the pipe/newline shape is the one Core already
/// uses for a row's `options`. The face keeps only the mapping from key to its
/// own snapshot field — the names and the ORDER come from here, so an appended
/// token cannot silently shift what an index means.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_theme_tokens(out: *mut c_char, cap: usize) -> u8 {
    if out.is_null() || cap == 0 {
        return 0;
    }
    let table = suisei_core::theme::ThemeToken::ALL
        .iter()
        .map(|t| format!("{}|{}", t.key(), t.label()))
        .collect::<Vec<_>>()
        .join("\n");
    unsafe { write_cstr_raw(out, cap, &table) };
    1
}

/// Bit per `ThemeToken` index: set when the user has changed that colour on the
/// palette currently being edited.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_theme_override_mask(ptr: *const SuiseiEngine) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.theme_override_mask() }
}

/// Set one theme colour. An empty value or `"default"` clears the override.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_set_theme_token(
    ptr: *mut SuiseiEngine,
    index: u32,
    value: *const c_char,
) {
    if ptr.is_null() || value.is_null() {
        return;
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    unsafe {
        (*ptr).0.settings_set_theme_token(index, &value);
    }
}

/// Every choosable theme, as `name|Label|isCustom` one per line.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_theme_catalogue(
    ptr: *const SuiseiEngine,
    out: *mut c_char,
    cap: usize,
) -> u8 {
    if ptr.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let list = unsafe { (*ptr).0.theme_catalogue() };
    unsafe { write_cstr_raw(out, cap, &list) };
    1
}

/// The theme in use, as named in the config.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_selected_theme(
    ptr: *const SuiseiEngine,
    out: *mut c_char,
    cap: usize,
) -> u8 {
    if ptr.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let name = unsafe { (*ptr).0.selected_theme() };
    unsafe { write_cstr_raw(out, cap, &name) };
    1
}

/// Choose a theme by name — built-in or user-made.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_select_theme(
    ptr: *mut SuiseiEngine,
    name: *const c_char,
) {
    if ptr.is_null() || name.is_null() {
        return;
    }
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    unsafe {
        (*ptr).0.settings_select_theme(&name);
    }
}

/// Keep the current palette's edits as a theme of its own. Writes the stored
/// name into `out`; an empty result means the name was refused (blank, already
/// taken, or shadowing a built-in).
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_save_theme_as(
    ptr: *mut SuiseiEngine,
    name: *const c_char,
    out: *mut c_char,
    cap: usize,
) -> u8 {
    if ptr.is_null() || name.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    let saved = unsafe { (*ptr).0.settings_save_theme_as(&name) };
    unsafe { write_cstr_raw(out, cap, &saved) };
    u8::from(!saved.is_empty())
}

/// Delete a user-made theme. Built-ins are not deletable and are ignored.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_delete_theme(
    ptr: *mut SuiseiEngine,
    name: *const c_char,
) {
    if ptr.is_null() || name.is_null() {
        return;
    }
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    unsafe {
        (*ptr).0.settings_delete_theme(&name);
    }
}

/// Put every colour of the current palette back to the theme's own.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_settings_reset_theme_tokens(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.settings_reset_theme_tokens();
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
pub const SUISEI_MAX_GIT_WORKTREE: usize = 160;
pub const SUISEI_MAX_GIT_HISTORY: usize = 80;
pub const SUISEI_MAX_GIT_BRANCHES: usize = 160;
pub const SUISEI_MAX_GIT_FILES: usize = 160;
pub const SUISEI_MAX_GIT_STASHES: usize = 40;
pub const SUISEI_MAX_GIT_REMOTES: usize = 24;
pub const SUISEI_GIT_PATH: usize = 320;
pub const SUISEI_GIT_SUBJECT: usize = 240;
pub const SUISEI_GIT_AUTHOR: usize = 96;
pub const SUISEI_GIT_EMAIL: usize = 160;

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
    /// The commit, in parts. `graph_lines` above is the same data joined into
    /// one pre-formatted string, and the face printed it verbatim in monospace
    /// because that is all it could do with it — Core has had `short`,
    /// `subject`, `when` and `refs` as separate fields the whole time.
    pub graph_short: [[c_char; 16]; SUISEI_MAX_SCM_GRAPH],
    pub graph_subject: [[c_char; 160]; SUISEI_MAX_SCM_GRAPH],
    pub graph_when: [[c_char; 32]; SUISEI_MAX_SCM_GRAPH],
    pub graph_refs: [[c_char; 96]; SUISEI_MAX_SCM_GRAPH],
    /// Lane colour index from the graph walker, so branches keep their hue.
    pub graph_color: [u8; SUISEI_MAX_SCM_GRAPH],
    /// 1 = on HEAD and not on its upstream. Xcode's `U`.
    ///
    /// Per row rather than a count, because the walk is `--all`: with a count
    /// the face's only rule would be "the first N", and the first N rows of a
    /// date-ordered all-branches walk are not the unpushed commits.
    pub graph_unpushed: [u8; SUISEI_MAX_SCM_GRAPH],
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
    // Structured native-window data. The legacy text columns above remain for
    // the painted/TUI workbench, but SwiftUI must not reverse-parse them.
    pub selected_change: u32,
    pub worktree_count: u32,
    pub history_count: u32,
    pub history_selected: u32,
    pub branch_count: u32,
    pub branch_selected: u32,
    pub commit_file_count: u32,
    pub commit_file_selected: u32,
    pub stash_count: u32,
    pub remote_count: u32,
    pub commit_detail_valid: u8,
    pub root_path: [c_char; SUISEI_PATH_CAP],
    pub repository_name: [c_char; SUISEI_GIT_AUTHOR],
    pub author_name: [c_char; SUISEI_GIT_AUTHOR],
    pub author_email: [c_char; SUISEI_GIT_EMAIL],
    pub worktree_staged: [u8; SUISEI_MAX_GIT_WORKTREE],
    pub worktree_status: [c_char; SUISEI_MAX_GIT_WORKTREE],
    pub worktree_paths: [[c_char; SUISEI_GIT_PATH]; SUISEI_MAX_GIT_WORKTREE],
    pub history_hashes: [[c_char; 48]; SUISEI_MAX_GIT_HISTORY],
    pub history_shorts: [[c_char; 16]; SUISEI_MAX_GIT_HISTORY],
    pub history_subjects: [[c_char; SUISEI_GIT_SUBJECT]; SUISEI_MAX_GIT_HISTORY],
    pub history_authors: [[c_char; SUISEI_GIT_AUTHOR]; SUISEI_MAX_GIT_HISTORY],
    pub history_whens: [[c_char; 64]; SUISEI_MAX_GIT_HISTORY],
    pub branch_current: [u8; SUISEI_MAX_GIT_BRANCHES],
    pub branch_remote: [u8; SUISEI_MAX_GIT_BRANCHES],
    pub branch_names: [[c_char; SUISEI_GIT_PATH]; SUISEI_MAX_GIT_BRANCHES],
    pub branch_upstreams: [[c_char; SUISEI_GIT_PATH]; SUISEI_MAX_GIT_BRANCHES],
    pub commit_file_status: [c_char; SUISEI_MAX_GIT_FILES],
    pub commit_file_insertions: [u32; SUISEI_MAX_GIT_FILES],
    pub commit_file_deletions: [u32; SUISEI_MAX_GIT_FILES],
    pub commit_file_paths: [[c_char; SUISEI_GIT_PATH]; SUISEI_MAX_GIT_FILES],
    pub detail_hash: [c_char; 48],
    pub detail_short: [c_char; 16],
    pub detail_subject: [c_char; SUISEI_GIT_SUBJECT],
    pub detail_author: [c_char; SUISEI_GIT_AUTHOR],
    pub detail_email: [c_char; SUISEI_GIT_EMAIL],
    pub detail_date: [c_char; 64],
    pub detail_body: [c_char; 512],
    pub detail_insertions: u32,
    pub detail_deletions: u32,
    pub stashes: [[c_char; SUISEI_GIT_WB_LINE]; SUISEI_MAX_GIT_STASHES],
    pub remote_names: [[c_char; SUISEI_GIT_AUTHOR]; SUISEI_MAX_GIT_REMOTES],
    pub remote_urls: [[c_char; SUISEI_GIT_PATH]; SUISEI_MAX_GIT_REMOTES],
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
        write_cstr(&mut o.graph_short[gi], &g.short);
        write_cstr(&mut o.graph_subject[gi], &g.subject);
        write_cstr(&mut o.graph_when[gi], &g.when);
        write_cstr(&mut o.graph_refs[gi], &g.refs);
        o.graph_color[gi] = g.color;
        o.graph_unpushed[gi] = u8::from(g.unpushed);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_scm_select(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.scm_select(row) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_scm_activate(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.scm_activate(row) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_scm_toggle_stage(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.scm_toggle_stage(row) }
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
    let pack =
        |dst: &mut [[c_char; SUISEI_GIT_WB_LINE]; SUISEI_MAX_GIT_COL], src: &[String]| -> u32 {
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

    let model = &eng.0.app.git_wb;
    o.selected_change = model.selected as u32;
    if let Some(root) = model.root.as_ref() {
        write_cstr(&mut o.root_path, &root.display().to_string());
        let repository = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Repository");
        write_cstr(&mut o.repository_name, repository);
    }
    write_cstr(&mut o.author_name, &model.author_name);
    write_cstr(&mut o.author_email, &model.author_email);

    for (index, entry) in model
        .staged
        .iter()
        .chain(model.changes.iter())
        .take(SUISEI_MAX_GIT_WORKTREE)
        .enumerate()
    {
        o.worktree_staged[index] = u8::from(entry.staged);
        o.worktree_status[index] = entry.status.letter() as c_char;
        write_cstr(&mut o.worktree_paths[index], &entry.path);
        o.worktree_count += 1;
    }

    o.history_selected = model.history_sel as u32;
    for (index, commit) in model
        .commits
        .iter()
        .take(SUISEI_MAX_GIT_HISTORY)
        .enumerate()
    {
        write_cstr(&mut o.history_hashes[index], &commit.hash);
        write_cstr(&mut o.history_shorts[index], &commit.short);
        write_cstr(&mut o.history_subjects[index], &commit.subject);
        write_cstr(&mut o.history_authors[index], &commit.author);
        write_cstr(&mut o.history_whens[index], &commit.when);
        o.history_count += 1;
    }

    o.branch_selected = model.branch_sel as u32;
    for (index, branch) in model
        .branches
        .iter()
        .take(SUISEI_MAX_GIT_BRANCHES)
        .enumerate()
    {
        o.branch_current[index] = u8::from(branch.current);
        o.branch_remote[index] = u8::from(branch.remote);
        write_cstr(&mut o.branch_names[index], &branch.name);
        if let Some(upstream) = branch.upstream.as_ref() {
            write_cstr(&mut o.branch_upstreams[index], upstream);
        }
        o.branch_count += 1;
    }

    o.commit_file_selected = model.commit_file_sel as u32;
    if let Some(detail) = model.commit_detail.as_ref() {
        o.commit_detail_valid = 1;
        write_cstr(&mut o.detail_hash, &detail.hash);
        write_cstr(&mut o.detail_short, &detail.short);
        write_cstr(&mut o.detail_subject, &detail.subject);
        write_cstr(&mut o.detail_author, &detail.author);
        write_cstr(&mut o.detail_email, &detail.email);
        write_cstr(&mut o.detail_date, &detail.date);
        write_cstr(&mut o.detail_body, &detail.body);
        o.detail_insertions = detail.insertions;
        o.detail_deletions = detail.deletions;
        for (index, file) in detail.files.iter().take(SUISEI_MAX_GIT_FILES).enumerate() {
            o.commit_file_status[index] = file.status as c_char;
            o.commit_file_insertions[index] = file.insertions;
            o.commit_file_deletions[index] = file.deletions;
            write_cstr(&mut o.commit_file_paths[index], &file.path);
            o.commit_file_count += 1;
        }
    }

    for (index, stash) in model
        .stashes
        .iter()
        .take(SUISEI_MAX_GIT_STASHES)
        .enumerate()
    {
        write_cstr(&mut o.stashes[index], stash);
        o.stash_count += 1;
    }
    for (index, (name, url)) in model
        .remotes
        .iter()
        .take(SUISEI_MAX_GIT_REMOTES)
        .enumerate()
    {
        write_cstr(&mut o.remote_names[index], name);
        write_cstr(&mut o.remote_urls[index], url);
        o.remote_count += 1;
    }
    1
}

/// Monotonic token for the complete Git workbench diff payload.
///
/// The regular workbench snapshot intentionally stays small because it is
/// sampled with the rest of the chrome. The GUI compares this token and only
/// pulls the complete diff when it actually changes.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_diff_generation(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.app.git_wb.diff_generation }
}

/// Monotonic invalidation token for the structured native workbench model.
/// Unlike `frame_gen`, this does not move for editor paint, terminal or LSP
/// activity, so frontends can avoid copying `SuiseiGitWbSnapshot` on those
/// unrelated frames.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_generation(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.git_wb_generation() }
}

/// Bytes required for all raw diff lines encoded as consecutive NUL-terminated
/// UTF-8 strings. This preserves arbitrarily long source lines; a fixed stride
/// would merely move the old 220-byte truncation to another magic number.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_diff_byte_count(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    let lines = unsafe { &(*ptr).0.app.git_wb.diff_lines };
    lines.iter().fold(0_u64, |total, line| {
        total
            .saturating_add(line.text.len() as u64)
            .saturating_add(1)
    })
}

/// The text the change on `line_1based` replaced, as one UTF-8 string with
/// embedded newlines, NUL-terminated.
///
/// Returns the byte length written, or the length REQUIRED when `capacity` is
/// too small — so a caller can size a buffer with `capacity = 0` and call
/// again. Zero means the line carries no change, or the change removed
/// nothing (a pure addition replaced no text).
///
/// The removed lines exist nowhere else the face can reach: they are not in
/// the buffer, by definition. This is the only way across.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_hunk_removed_text(
    ptr: *const SuiseiEngine,
    line_1based: u32,
    out: *mut c_char,
    capacity: u64,
) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    let row = (line_1based.max(1) - 1) as usize;
    let app = unsafe { &(*ptr).0.app };
    let Some(hunk) = app.git.hunk_at(row) else {
        return 0;
    };
    if hunk.removed.is_empty() {
        return 0;
    }
    let text = hunk.removed.join("\n");
    let required = text.len() + 1;
    let Ok(capacity) = usize::try_from(capacity) else {
        return 0;
    };
    if out.is_null() || capacity < required {
        return required as u64;
    }
    let dst = unsafe { std::slice::from_raw_parts_mut(out.cast::<u8>(), capacity) };
    dst[..text.len()].copy_from_slice(text.as_bytes());
    dst[text.len()] = 0;
    required as u64
}

/// Copy the complete diff into `out` as consecutive NUL-terminated UTF-8
/// strings. Returns zero when the supplied buffer is too small, so callers
/// never observe a partial final line.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_diff_copy(
    ptr: *const SuiseiEngine,
    out: *mut c_char,
    capacity: u64,
) -> u64 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let Ok(capacity) = usize::try_from(capacity) else {
        return 0;
    };
    let lines = unsafe { &(*ptr).0.app.git_wb.diff_lines };
    let Some(required) = lines.iter().try_fold(0_usize, |total, line| {
        total.checked_add(line.text.len() + 1)
    }) else {
        return 0;
    };
    if capacity < required {
        return 0;
    }

    let dst = unsafe { std::slice::from_raw_parts_mut(out.cast::<u8>(), capacity) };
    let mut offset = 0_usize;
    for line in lines {
        let bytes = line.text.as_bytes();
        dst[offset..offset + bytes.len()].copy_from_slice(bytes);
        offset += bytes.len();
        dst[offset] = 0;
        offset += 1;
    }
    offset as u64
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
pub extern "C" fn suisei_engine_git_wb_select_change(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_select_change(row) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_select_history(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_select_history(row) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_select_commit_file(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_select_commit_file(row) }
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

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_select_special(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.git_wb_select_special(row);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_select_branch_history(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_select_branch_history(row) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_refresh_window(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_refresh_window() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_toggle_stage(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_toggle_stage(row) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_stage_all(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_stage_all() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_unstage_all(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_unstage_all() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_commit(
    ptr: *mut SuiseiEngine,
    message: *const c_char,
    amend: u8,
) {
    if ptr.is_null() || message.is_null() {
        return;
    }
    let Ok(message) = unsafe { CStr::from_ptr(message) }.to_str() else {
        return;
    };
    unsafe { (*ptr).0.git_wb_commit(message, amend != 0) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_stash(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_stash() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_discard_change(ptr: *mut SuiseiEngine, row: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_discard_change(row) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_open_window(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_open_window() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_focus_window(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_focus_window() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_close_window(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_close_window() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_checkout_selected_branch(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_checkout_selected_branch() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_create_branch(ptr: *mut SuiseiEngine, name: *const c_char) {
    if ptr.is_null() || name.is_null() {
        return;
    }
    let Ok(name) = unsafe { CStr::from_ptr(name) }.to_str() else {
        return;
    };
    unsafe { (*ptr).0.git_wb_create_branch(name) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_git_wb_delete_selected_branch(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.git_wb_delete_selected_branch() }
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
    let n = p
        .lines
        .len()
        .saturating_sub(start_i)
        .min(SUISEI_MAX_PREVIEW);
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

/// Fingerprint of the current diagnostic set — 0 when there are none.
///
/// `suisei_engine_diagnostics` memsets a 48.6 KiB struct and the face then
/// builds a `String` per entry. Diagnostics change when a language server
/// answers, not on the 20 Hz tick, so the face compares this first and only
/// pays for the snapshot when it moves. Hashes positions, severities AND
/// message text, so a same-position message rewrite is not missed.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_diagnostics_fingerprint(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    use std::hash::{Hash, Hasher};
    let eng = unsafe { &*ptr };
    let diags = &eng.0.app().lsp.diagnostics;
    if diags.is_empty() {
        return 0;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    diags.len().hash(&mut h);
    for d in diags.iter().take(SUISEI_MAX_DIAGS) {
        d.row.hash(&mut h);
        d.col_start.hash(&mut h);
        std::mem::discriminant(&d.severity).hash(&mut h);
        d.message.hash(&mut h);
    }
    // 0 is reserved for "no diagnostics" — never collide with it.
    h.finish() | 1
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

/// One row a live reload touched. 8 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SuiseiLiveMarkC {
    pub row: u32,
    /// `suisei_core::LiveKind` — 0 changed, 1 added, 2 removed.
    pub kind: u8,
    pub _pad: u8,
    /// Rows this removal took away, on a `Removed` mark; 0 otherwise. The mark
    /// says WHERE the lines were, and only this says how many, which is what
    /// the closing gap has to be the size of.
    pub removed: u16,
}

/// Bumped whenever the live-reload marks change, including when they expire.
/// The face polls this — one `u64` read — and pulls the list only when it has
/// actually moved.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_live_gen(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { &*ptr }.0.app().live_gen
}

/// The rows a live reload just replaced, with what it did to them.
///
/// A pull rather than per-row bits in the line array, because the MINIMAP has
/// to show changes that are off screen and the line array only carries the
/// visible band. One list serving both surfaces beats a bit for the canvas and
/// a list for the minimap — that is the same fact with two owners, and they
/// would disagree at exactly the moment the marks expire.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_live_marks(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiLiveMarkC,
    cap: u32,
) -> u32 {
    if ptr.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let app = unsafe { &*ptr }.0.app();
    let dst = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    let mut n = 0usize;
    for (&row, &kind) in app.live_rows.iter() {
        if n >= dst.len() {
            break;
        }
        dst[n] = SuiseiLiveMarkC {
            row: row as u32,
            kind: kind as u8,
            _pad: 0,
            removed: if kind == suisei_core::LiveKind::Removed {
                app.live_removed
            } else {
                0
            },
        };
        n += 1;
    }
    n as u32
}

/// Paths a live reload touched recently, as consecutive NUL-terminated UTF-8
/// strings. Returns the count written, or 0 if `out` cannot hold them all.
///
/// Per PATH, and including background tabs — which is the point. The row
/// marks describe the live document only, and the project tree is where a
/// file nobody is looking at can say that it moved.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_live_files(
    ptr: *const SuiseiEngine,
    out: *mut c_char,
    cap: u32,
) -> u32 {
    if ptr.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let app = unsafe { &*ptr }.0.app();
    if app.live_files.is_empty() {
        return 0;
    }
    let dst = unsafe { std::slice::from_raw_parts_mut(out.cast::<u8>(), cap as usize) };
    let mut at = 0usize;
    let mut n = 0u32;
    for path in app.live_files.keys() {
        let bytes = path.as_os_str().as_encoded_bytes();
        if at + bytes.len() + 1 > dst.len() {
            break;
        }
        dst[at..at + bytes.len()].copy_from_slice(bytes);
        dst[at + bytes.len()] = 0;
        at += bytes.len() + 1;
        n += 1;
    }
    n
}

/// Absolute path of the document in pane `idx`. Returns 0 when that pane has
/// no file — an untitled document, or a shell.
///
/// A pull rather than a field on the chrome snapshot, on the same reasoning as
/// `suisei_engine_pane_terminal_cwd`: only the non-text viewers need it, they
/// need it once when they appear, and four more 512-byte arrays in a struct
/// that is rebuilt every frame would be paid for by every frame that has no
/// viewer in it — which is nearly all of them.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_pane_path(
    ptr: *const SuiseiEngine,
    idx: u32,
    out: *mut c_char,
    cap: u32,
) -> u8 {
    if ptr.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let app = unsafe { &*ptr }.0.app();
    let Some(pane) = app.split.panes.get(idx as usize) else {
        return 0;
    };
    let Some(path) = app
        .tabs
        .buffers
        .iter()
        .find(|t| t.id == pane.buffer)
        .and_then(|t| t.filename.as_ref())
    else {
        return 0;
    };
    let dst = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    write_cstr(dst, &path.display().to_string());
    1
}

/// Stable `BufferTab::id` for the document shown by one pane.
///
/// Kept as a cheap pull beside `suisei_engine_pane_path`: only viewer panes
/// need it and carrying another u64 through every chrome snapshot would charge
/// the ordinary text-editor path for a viewer lifetime hook.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_pane_tab_id(ptr: *const SuiseiEngine, idx: u32) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { &*ptr }
        .0
        .app()
        .split
        .panes
        .get(idx as usize)
        .map(|pane| pane.buffer.0)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_tab_id_is_open(ptr: *const SuiseiEngine, id: u64) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    u8::from(
        id != 0
            && unsafe { &*ptr }
                .0
                .app()
                .tabs
                .buffers
                .iter()
                .any(|tab| tab.id.0 == id),
    )
}

/// Where the shell in pane `idx` should be working. Returns 0 when that pane
/// is not a terminal.
///
/// The face forks the pane shells now, so it is the one that needs this — at
/// the moment it makes the session, and again for every terminal tab a window
/// restores. The directory is the whole of what survives a restart (see
/// `BufferTab::terminal_cwd`), so a restored tab that lands in the right place
/// is the difference between useful and merely present.
///
/// A pull beside `suisei_engine_pane_path` rather than a snapshot field, for
/// the same reason: it is asked once per shell, and a 512-byte array per pane
/// in a struct rebuilt every frame would be paid for by every frame that has
/// no terminal in it.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_pane_terminal_cwd(
    ptr: *const SuiseiEngine,
    idx: u32,
    out: *mut c_char,
    cap: u32,
) -> u8 {
    if ptr.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let app = unsafe { &*ptr }.0.app();
    let Some(pane) = app.split.panes.get(idx as usize) else {
        return 0;
    };
    let Some(cwd) = app
        .tabs
        .buffers
        .iter()
        .find(|t| t.id == pane.buffer)
        .filter(|t| t.terminal.is_some())
        .and_then(|t| t.terminal_cwd.as_ref())
    else {
        return 0;
    };
    let dst = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    write_cstr(dst, &cwd.display().to_string());
    1
}

/// Mark a directory as a project — write `project.suiseiprj` if it has none.
///
/// Idempotent: a folder that is already a project keeps the identity it has.
/// Returns 1 when the folder is a project afterwards, 0 when the file could not
/// be written (read-only volume, no permission).
///
/// Freestanding — no engine. The face calls it from "New Project" and from
/// "Set Project Master Directory", neither of which needs a document open.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_project_mark(dir: *const c_char) -> u8 {
    let Some(dir) = cstr_path(dir) else { return 0 };
    u8::from(suisei_core::project::ensure(&dir).is_ok())
}

/// Whether this exact directory carries the marker.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_project_is_marked(dir: *const c_char) -> u8 {
    let Some(dir) = cstr_path(dir) else { return 0 };
    u8::from(suisei_core::project::is_project(&dir))
}

/// The project root at or above `path`, written to `out`. 0 when there is none.
///
/// Walks up, so a file deep inside a project answers with the project. This is
/// what the face asks before letting a folder become a master directory: a
/// non-zero answer that is not the folder itself means it is inside one.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_project_root_of(
    path: *const c_char,
    out: *mut c_char,
    cap: u32,
) -> u8 {
    if out.is_null() || cap == 0 {
        return 0;
    }
    let Some(path) = cstr_path(path) else { return 0 };
    let Some(root) = suisei_core::project::find_root(&path) else {
        return 0;
    };
    let dst = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    write_cstr(dst, &root.display().to_string());
    1
}

/// A project's display name, or its identity, from its marker.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_project_name(
    dir: *const c_char,
    out: *mut c_char,
    cap: u32,
) -> u8 {
    if out.is_null() || cap == 0 {
        return 0;
    }
    let Some(dir) = cstr_path(dir) else { return 0 };
    let Some(p) = suisei_core::project::read(&dir) else {
        return 0;
    };
    let dst = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    write_cstr(dst, &p.name);
    1
}

fn cstr_path(p: *const c_char) -> Option<std::path::PathBuf> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    if s.is_empty() {
        return None;
    }
    Some(std::path::PathBuf::from(s))
}

/// Where a shell with no pane of its own should start — the docked strip's.
///
/// Core's answer to "which directory is this window about": the explorer's
/// current directory, else the project root, else `$HOME`. Every shell in the
/// window has always used this policy; a pane's is frozen at spawn time into
/// `BufferTab::terminal_cwd` so a restore can reproduce it, and the dock's is
/// asked for fresh each time a session is opened.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_terminal_cwd(
    ptr: *const SuiseiEngine,
    out: *mut c_char,
    cap: u32,
) -> u8 {
    if ptr.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let cwd = unsafe { &*ptr }.0.app().terminal_working_directory();
    let dst = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    write_cstr(dst, &cwd.display().to_string());
    1
}

/// The face reporting the title its pane shell announced (OSC 0/2). A null or
/// empty `title` clears it back to the generic "Terminal".
///
/// Push rather than pull because it is rare and unpredictable: a shell sends a
/// title when it feels like it, and polling every terminal tab every frame to
/// find out would cost more than the fact is worth. Addressed by
/// `BufferTab::id` — the face keys its shells by tab and has no name for our
/// terminal ids.
///
/// Recomposes only on a real change. `zsh` re-sends its title on every prompt,
/// and rebuilding the chrome for a string that did not move would put a
/// full republish behind every command the user runs.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_set_terminal_title(
    ptr: *mut SuiseiEngine,
    tab_id: u64,
    title: *const c_char,
) {
    if ptr.is_null() || tab_id == 0 {
        return;
    }
    let owned;
    let title = if title.is_null() {
        None
    } else {
        owned = unsafe { CStr::from_ptr(title) }.to_string_lossy().into_owned();
        Some(owned.as_str())
    };
    unsafe {
        if (*ptr)
            .0
            .app_mut()
            .set_terminal_title(suisei_core::BufferId(tab_id), title)
        {
            (*ptr).0.recompose_paint_only();
        }
    }
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

/// Forward a mouse event to a terminal's inner app (xterm tracking).
/// `pane` = 0xFFFF targets the dock. Returns 1 when the shell consumed the
/// event (the face should then NOT also act on it — e.g. wheel scrollback).
/// button: 0 left, 1 middle, 2 right, 64 wheel-up, 65 wheel-down.
/// x/y: 1-based cell coordinates.
/// Restore the previous session's files + cursors (call once at startup).
/// Landing named buffers flips the welcome rule, so Welcome yields to the
/// restored editor.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_restore_session(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        (*ptr).0.restore_session();
    }
}

/// Persist open files + cursors for the next launch.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_save_session(ptr: *const SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    let eng = unsafe { &*ptr };
    eng.0.save_session();
}

fn github_state_code(state: suisei_core::GhAuthState) -> u8 {
    match state {
        suisei_core::GhAuthState::NotInstalled => SUISEI_GH_STATE_MISSING,
        suisei_core::GhAuthState::LoggedOut => SUISEI_GH_STATE_OUT,
        suisei_core::GhAuthState::LoggedIn => SUISEI_GH_STATE_IN,
    }
}

/// Settings account page. First pull starts a background probe so the face
/// does not block on `gh api user`.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_account(
    ptr: *mut SuiseiEngine,
    out: *mut SuiseiGitHubAccount,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &mut *ptr };
    eng.0.github_account.ensure_loaded();
    let acc = &eng.0.github_account;
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiGitHubAccount>());
    }
    let o = unsafe { &mut *out };
    o.generation = acc.generation;
    o.state = github_state_code(acc.info.state);
    o.loading = u8::from(acc.loading);
    o.signing_in = u8::from(acc.signing_in());
    o.public_repos = acc.profile.public_repos;
    o.followers = acc.profile.followers;
    o.following = acc.profile.following;
    write_cstr(&mut o.user, &acc.info.user);
    if o.user[0] == 0 {
        write_cstr(&mut o.user, &acc.profile.login);
    }
    write_cstr(&mut o.name, acc.profile.display_name());
    write_cstr(&mut o.email, &acc.profile.email);
    write_cstr(&mut o.avatar_url, &acc.profile.avatar_url);
    write_cstr(&mut o.bio, &acc.profile.bio);
    write_cstr(&mut o.company, &acc.profile.company);
    write_cstr(&mut o.location, &acc.profile.location);
    write_cstr(&mut o.html_url, &acc.profile.html_url);
    write_cstr(&mut o.host, &acc.info.host);
    write_cstr(&mut o.protocol, &acc.info.protocol);
    write_cstr(&mut o.scopes, &acc.info.scopes);
    write_cstr(&mut o.token_source, &acc.info.token_source);
    write_cstr(&mut o.device_code, acc.device_code());
    write_cstr(&mut o.message, &acc.message);
    o.contrib_total = acc.contributions.total;
    let n = acc.contributions.levels.len().min(SUISEI_GH_CONTRIB_DAYS);
    o.contrib_days = n as u16;
    o.contrib_levels[..n].copy_from_slice(&acc.contributions.levels[..n]);
    write_cstr(&mut o.contrib_start, &acc.contributions.start);
    o.contrib_year = acc.contrib_year;
    o.contrib_year_min = acc.contrib_year_min;
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_account_generation(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.github_account.generation }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_account_refresh(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.github_account.refresh() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_sign_in(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.github_account.sign_in() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_sign_out(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.github_account.sign_out() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_cancel_sign_in(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.github_account.cancel_sign_in() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_open_profile(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.github_account.open_profile() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_setup_git(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.github_account.setup_git() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_set_contrib_year(ptr: *mut SuiseiEngine, year: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.github_account.set_contrib_year(year) }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_github_install_docs(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    unsafe { (*ptr).0.github_account.open_install_docs() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_update(
    ptr: *const SuiseiEngine,
    out: *mut SuiseiUpdateSnapshot,
) -> u8 {
    if ptr.is_null() || out.is_null() {
        return 0;
    }
    let eng = unsafe { &*ptr };
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, size_of::<SuiseiUpdateSnapshot>());
    }
    let o = unsafe { &mut *out };
    o.generation = eng.0.update_generation;
    o.available = u8::from(eng.0.app.update.latest.is_some());
    o.installing = u8::from(eng.0.app.update.installing);
    o.installed = u8::from(eng.0.app.update.installed);
    o.checking = u8::from(eng.0.app.update.is_checking());
    write_cstr(&mut o.current, env!("CARGO_PKG_VERSION"));
    if let Some(latest) = eng.0.app.update.latest.as_deref() {
        write_cstr(&mut o.latest, latest);
    }
    write_cstr(&mut o.notes, &eng.0.app.update.notes);
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_update_generation(ptr: *const SuiseiEngine) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.update_generation }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_update_check(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    let eng = unsafe { &mut *ptr };
    eng.0.app.update.check_now(env!("CARGO_PKG_VERSION"));
    eng.0.update_generation = eng.0.update_generation.wrapping_add(1);
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_update_install(ptr: *mut SuiseiEngine) {
    if ptr.is_null() {
        return;
    }
    let eng = unsafe { &mut *ptr };
    let msg = eng.0.app.update.start_install();
    if !msg.is_empty() {
        eng.0.app.message = msg;
    }
    eng.0.update_generation = eng.0.update_generation.wrapping_add(1);
}

/// Microseconds the last completion pass took, and how much of that was the
/// lexical-visibility walk.
///
/// Diagnostic only. The popup was the last thing the user could still feel,
/// and the Swift side measured its publish at 0.044 ms — so the cost was over
/// here, unmeasured, and the app's perf log had no way to show a Rust number
/// beside its own. These two let it.
#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_completion_last_total_us(ptr: *const SuiseiEngine) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.app.completions.last_total_us }
}

#[unsafe(no_mangle)]
pub extern "C" fn suisei_engine_completion_last_scope_us(ptr: *const SuiseiEngine) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).0.app.completions.last_scope_us }
}

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
    eng.0.app.buffer.cursor.row =
        (entry.cursor_row as usize).min(eng.0.app.buffer.line_count().saturating_sub(1));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Truncation must land on a char boundary: a mid-UTF-8 cut hands the
    /// face an invalid C string, which String(cString:) renders as
    /// replacement-char garbage at the end of dense CJK/emoji lines.
    #[test]
    fn write_cstr_truncates_on_char_boundaries() {
        // 6 bytes total → 5 usable + NUL. "가" is 3 bytes: exactly one fits;
        // a naive byte cap would take 5 bytes = one char + two stray bytes.
        let mut dst = [0 as c_char; 6];
        write_cstr(&mut dst, "가나다");
        let bytes: &[u8] = unsafe { std::slice::from_raw_parts(dst.as_ptr() as *const u8, 6) };
        assert_eq!(&bytes[..3], "가".as_bytes(), "first char intact");
        assert_eq!(bytes[3], 0, "NUL right after the last full char");
        assert_eq!(bytes[4], 0, "no stray partial bytes");
    }

    /// ASCII fills to the brim with the NUL in the last slot.
    #[test]
    fn write_cstr_fills_ascii_exactly() {
        let mut dst = [0 as c_char; 4];
        write_cstr(&mut dst, "abcdef");
        let bytes: &[u8] = unsafe { std::slice::from_raw_parts(dst.as_ptr() as *const u8, 4) };
        assert_eq!(&bytes[..3], b"abc");
        assert_eq!(bytes[3], 0);
    }

    #[test]
    fn git_diff_copy_preserves_all_lines_and_long_utf8_content() {
        use suisei_core::git_ops::{DiffLine, DiffLineKind};

        let mut engine = Box::new(SuiseiEngine(Engine::new()));
        let long_line = format!("+{}", "긴줄🙂".repeat(180));
        engine.0.app.git_wb.diff_lines = vec![
            DiffLine::new(DiffLineKind::Header, "diff --git a/a b/a"),
            DiffLine::new(DiffLineKind::Add, long_line.clone()),
            DiffLine::new(DiffLineKind::Context, " final line"),
        ];

        let required = suisei_engine_git_wb_diff_byte_count(&*engine);
        assert!(required > SUISEI_GIT_WB_LINE as u64);
        let mut bytes = vec![0 as c_char; required as usize];
        let copied = suisei_engine_git_wb_diff_copy(&*engine, bytes.as_mut_ptr(), required);
        assert_eq!(copied, required);

        let raw = bytes.into_iter().map(|byte| byte as u8).collect::<Vec<_>>();
        let decoded = raw
            .split(|byte| *byte == 0)
            .filter(|line| !line.is_empty())
            .map(|line| String::from_utf8(line.to_vec()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[1], long_line);
        assert_eq!(decoded[2], " final line");
    }

    #[test]
    fn absolute_utf16_caret_offset_counts_cjk_surrogates_and_newlines() {
        let mut engine = Box::new(SuiseiEngine(Engine::new()));
        engine.0.app.buffer = suisei_core::buffer::Buffer::from_string("a한🙂\n글b");
        engine.0.app.buffer.cursor = suisei_core::buffer::Position::new(1, 1);
        // row 0: a(1)+한(1)+🙂(2)+newline(1), then 글(1)
        assert_eq!(suisei_engine_caret_utf16_offset(&*engine), 6);
    }

    #[test]
    fn utf16_hit_test_does_not_invent_cells_for_combining_marks() {
        let mut engine = SuiseiEngine(Engine::new());
        engine.0.app.buffer = suisei_core::buffer::Buffer::from_string("e\u{301}x");
        assert_eq!(vcol_for_utf16(&engine, 0, 2), 1);
        assert_eq!(vcol_for_utf16(&engine, 0, 3), 2);
    }

    #[test]
    fn project_switch_refuses_dirty_tabs_then_replaces_clean_workspace() {
        let root =
            std::env::temp_dir().join(format!("suisei_switch_project_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("README.md"), "# next\n").unwrap();
        let c_path = std::ffi::CString::new(root.display().to_string()).unwrap();

        let mut engine = Box::new(SuiseiEngine(Engine::new()));
        engine.0.app.filename = Some("/tmp/old.rs".into());
        engine.0.app.modified = true;
        assert_eq!(
            suisei_engine_switch_project(&mut *engine, c_path.as_ptr()),
            2,
            "dirty workspace must not be replaced"
        );
        assert_eq!(
            engine.0.app.filename.as_deref(),
            Some(std::path::Path::new("/tmp/old.rs"))
        );

        engine.0.app.modified = false;
        for tab in &mut engine.0.app.tabs.buffers {
            tab.modified = false;
        }
        assert_eq!(
            suisei_engine_switch_project(&mut *engine, c_path.as_ptr()),
            1
        );
        assert_eq!(engine.0.app.explorer.cwd, root);
        assert_eq!(engine.0.app.tabs.buffers.len(), 1);
        assert!(
            engine
                .0
                .app
                .filename
                .as_ref()
                .is_some_and(|path| path.ends_with("README.md"))
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
