//! ABI layout verification — Rust `#[repr(C)]` structs vs the hardcoded byte
//! offsets in the Swift face (`EngineBridge.decodeEditorLinesAndSplit`).
//!
//! The Swift decoder reads fields at FIXED offsets (no bridging struct import).
//! If a Rust field moves without updating Swift, the face silently misreads.
//! This test is the tripwire.
//!
//! ```text
//! cargo test -p suisei-engine --test abi_layout
//! ```

use std::mem::{offset_of, size_of};

// Re-export the FFI types from the engine crate.
use suisei_engine::ffi::{
    SuiseiChromeSnapshot, SuiseiEditorLineC, SuiseiPaneC, SuiseiSpanC,
    SuiseiTerminalSnapshot, SUISEI_LINE_CAP, SUISEI_MAX_LINES, SUISEI_MAX_PANES,
    SUISEI_MAX_SPANS, SUISEI_MAX_TABS, SUISEI_MAX_TERM_LINES, SUISEI_MODE_CAP,
    SUISEI_MSG_CAP, SUISEI_PATH_CAP, SUISEI_TERM_LINE, SUISEI_TITLE_CAP,
};

// ─── Constants ────────────────────────────────────────────────────────────────

#[test]
fn constants_match_c_header() {
    // These #defines live in suisei_engine.h and are compiled into the Swift
    // binary via -import-objc-header. They must never silently diverge.
    assert_eq!(SUISEI_MAX_TABS, 64);
    assert_eq!(SUISEI_MAX_LINES, 256);
    assert_eq!(SUISEI_MAX_SPANS, 24);
    assert_eq!(SUISEI_MAX_PANES, 4);
    assert_eq!(SUISEI_TITLE_CAP, 96);
    assert_eq!(SUISEI_LINE_CAP, 512);
    assert_eq!(SUISEI_MSG_CAP, 256);
    assert_eq!(SUISEI_PATH_CAP, 512);
    assert_eq!(SUISEI_MODE_CAP, 24);
    // Terminal snapshot. `SUISEI_TERM_LINE` is BYTES per row, not columns —
    // rows carry truecolor SGR escapes, so a colour change costs up to 19 bytes
    // on top of the character. Too small a value silently truncates output
    // mid-line, which is what the old 256 did.
    assert_eq!(SUISEI_MAX_TERM_LINES, 200);
    assert_eq!(SUISEI_TERM_LINE, 1536);
}

// ─── SuiseiSpanC ──────────────────────────────────────────────────────────────

#[test]
fn span_c_layout() {
    assert_eq!(size_of::<SuiseiSpanC>(), 6, "SuiseiSpanC size");
    // Swift reads: start(u16) @0, end(u16) @2, kind(u8) @4
    assert_eq!(offset_of!(SuiseiSpanC, start), 0);
    assert_eq!(offset_of!(SuiseiSpanC, end), 2);
    assert_eq!(offset_of!(SuiseiSpanC, kind), 4);
}

// ─── SuiseiEditorLineC ────────────────────────────────────────────────────────
//
// Swift `decodeEditorLinesAndSplit` hardcoded offsets (EngineBridge.swift):
//   0  line_no     (u32)
//   4  is_cursor   (u8)
//   5  git_sign    (u8)
//   6  span_count  (u8)
//   8  caret_vcol  (u32)
//  12  caret_utf16 (u32)
//  16  sel_v0      (u32)
//  20  sel_v1      (u32)
//  24  sel_u0      (u32)
//  28  sel_u1      (u32)
//  32  text        ([c_char; 512])
//  32+512 = 544  spans ([SuiseiSpanC; 24])

#[test]
fn editor_line_c_layout() {
    assert_eq!(offset_of!(SuiseiEditorLineC, line_no), 0);
    assert_eq!(offset_of!(SuiseiEditorLineC, is_cursor), 4);
    assert_eq!(offset_of!(SuiseiEditorLineC, git_sign), 5);
    assert_eq!(offset_of!(SuiseiEditorLineC, span_count), 6);
    assert_eq!(offset_of!(SuiseiEditorLineC, caret_vcol), 8);
    assert_eq!(offset_of!(SuiseiEditorLineC, caret_utf16), 12);
    assert_eq!(offset_of!(SuiseiEditorLineC, sel_v0), 16);
    assert_eq!(offset_of!(SuiseiEditorLineC, sel_v1), 20);
    assert_eq!(offset_of!(SuiseiEditorLineC, sel_u0), 24);
    assert_eq!(offset_of!(SuiseiEditorLineC, sel_u1), 28);
    assert_eq!(offset_of!(SuiseiEditorLineC, text), 32);
    assert_eq!(offset_of!(SuiseiEditorLineC, spans), 32 + SUISEI_LINE_CAP);

    // Total stride used by Swift's MemoryLayout<SuiseiEditorLineC>.stride
    let expected_size = 32 + SUISEI_LINE_CAP + SUISEI_MAX_SPANS * size_of::<SuiseiSpanC>();
    // repr(C) may add trailing padding to align the struct; assert at least.
    assert!(
        size_of::<SuiseiEditorLineC>() >= expected_size,
        "SuiseiEditorLineC too small: {} < {}",
        size_of::<SuiseiEditorLineC>(),
        expected_size,
    );
}

// ─── SuiseiPaneC ──────────────────────────────────────────────────────────────
//
// Swift hardcoded offsets:
//   0  tab_index     (u32)
//   4  scroll        (u32)
//   8  line_start    (u32)
//  12  line_count    (u32)
//  16  focused       (u8)
//  20  doc_line_count (u32)
//  24  hscroll       (u32)
//  28  rect_x/y/w/h  (4 x f32)

#[test]
fn pane_c_layout() {
    assert_eq!(offset_of!(SuiseiPaneC, tab_index), 0);
    assert_eq!(offset_of!(SuiseiPaneC, scroll), 4);
    assert_eq!(offset_of!(SuiseiPaneC, line_start), 8);
    assert_eq!(offset_of!(SuiseiPaneC, line_count), 12);
    assert_eq!(offset_of!(SuiseiPaneC, focused), 16);
    assert_eq!(offset_of!(SuiseiPaneC, doc_line_count), 20);
    assert_eq!(offset_of!(SuiseiPaneC, hscroll), 24);
    assert_eq!(offset_of!(SuiseiPaneC, rect_x), 28);
    assert_eq!(offset_of!(SuiseiPaneC, rect_y), 32);
    assert_eq!(offset_of!(SuiseiPaneC, rect_w), 36);
    assert_eq!(offset_of!(SuiseiPaneC, rect_h), 40);
    assert_eq!(size_of::<SuiseiPaneC>(), 44, "SuiseiPaneC stride");
}

// ─── SuiseiChromeSnapshot (key fields) ────────────────────────────────────────
//
// The Swift face reads the chrome snapshot via struct field access (not raw
// offsets) for most fields, but split/pane/lines are walked via raw memory.
// Verify the fields that feed into the raw-walk paths.

#[test]
fn chrome_snapshot_key_offsets() {
    // frame_gen must be first — Swift uses it for change detection.
    assert_eq!(offset_of!(SuiseiChromeSnapshot, frame_gen), 0);

    // Flags block
    assert_eq!(offset_of!(SuiseiChromeSnapshot, dirty_buffer) % 4, 0,
        "flags block should be u32-aligned after path fields");

    // Per-tab ids sit right after the titles, and the split metadata after
    // them. `tab_ids` is what the face uses as list identity — an index cannot
    // serve, because a reorder leaves the index list unchanged.
    let tab_titles_end = offset_of!(SuiseiChromeSnapshot, tab_titles)
        + SUISEI_MAX_TABS * SUISEI_TITLE_CAP;
    assert_eq!(offset_of!(SuiseiChromeSnapshot, tab_ids), tab_titles_end);
    let tab_ids_end = tab_titles_end + SUISEI_MAX_TABS * size_of::<u64>();
    // Layout-tab metadata sits between the ids and the split block.
    assert_eq!(offset_of!(SuiseiChromeSnapshot, tab_groups), tab_ids_end);
    let tab_groups_end = tab_ids_end + SUISEI_MAX_TABS * size_of::<u64>();
    assert_eq!(offset_of!(SuiseiChromeSnapshot, tab_is_layout), tab_groups_end);
    let tab_is_layout_end = tab_groups_end + SUISEI_MAX_TABS;
    assert_eq!(offset_of!(SuiseiChromeSnapshot, tab_is_terminal), tab_is_layout_end);

    // Panes array follows split metadata
    let split_ratio_off = offset_of!(SuiseiChromeSnapshot, split_ratio);
    assert_eq!(split_ratio_off % 4, 0, "split_ratio must be f32-aligned");
    assert_eq!(
        offset_of!(SuiseiChromeSnapshot, panes),
        split_ratio_off + 4,
        "panes immediately after split_ratio"
    );

    // Lines array is last — its offset determines the struct's total footprint.
    let panes_end = offset_of!(SuiseiChromeSnapshot, panes)
        + SUISEI_MAX_PANES * size_of::<SuiseiPaneC>();
    let vis_off = offset_of!(SuiseiChromeSnapshot, visible_line_count);
    assert!(vis_off >= panes_end, "visible_line_count after panes");
    let lines_off = offset_of!(SuiseiChromeSnapshot, lines);
    assert!(lines_off > vis_off, "lines array is the last field");
}

// ─── Compile-time guard: struct is Copy-safe for stack snapshots ──────────────

#[test]
fn chrome_snapshot_is_stack_friendly() {
    // The Swift face allocates this on the stack per refresh. If it grows past
    // ~4 MB the stack will overflow on background threads.
    let size = size_of::<SuiseiChromeSnapshot>();
    assert!(
        size < 4 * 1024 * 1024,
        "SuiseiChromeSnapshot is {} bytes — too large for safe stack allocation",
        size,
    );
    // The terminal snapshot is a second stack allocation on the same path.
    let term = size_of::<SuiseiTerminalSnapshot>();
    assert!(
        term < 1024 * 1024,
        "SuiseiTerminalSnapshot is {term} bytes — the face zero-fills this per refresh",
    );
    println!("SuiseiTerminalSnapshot: {} bytes ({:.1} KiB)", term, term as f64 / 1024.0);
    // Print for visibility in CI logs.
    println!("SuiseiChromeSnapshot: {} bytes ({:.1} KiB)", size, size as f64 / 1024.0);
    println!("SuiseiEditorLineC:    {} bytes", size_of::<SuiseiEditorLineC>());
    println!("SuiseiPaneC:          {} bytes", size_of::<SuiseiPaneC>());
}
