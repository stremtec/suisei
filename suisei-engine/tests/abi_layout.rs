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
    SUISEI_LINE_CAP, SUISEI_MAX_LINES, SUISEI_MAX_PANES, SUISEI_MAX_SPANS, SUISEI_MAX_TABS,
    SUISEI_MODE_CAP, SUISEI_MSG_CAP, SUISEI_PATH_CAP, SUISEI_TITLE_CAP, SuiseiChromeSnapshot,
    SuiseiEditorLineC, SuiseiPaneC, SuiseiSpanC,
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
    // `SUISEI_MAX_TERM_LINES` / `SUISEI_TERM_LINE` were here — 200 rows of
    // 1536 bytes, the shape of a terminal grid re-encoded as truecolor SGR to
    // cross this boundary. Terminals do not cross it any more.
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
//  18  term_gen      (u16)
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
    assert_eq!(offset_of!(SuiseiPaneC, kind), 17);
    assert_eq!(offset_of!(SuiseiPaneC, term_gen), 18);
    assert_eq!(offset_of!(SuiseiPaneC, doc_line_count), 20);
    assert_eq!(offset_of!(SuiseiPaneC, hscroll), 24);
    assert_eq!(offset_of!(SuiseiPaneC, rect_x), 28);
    assert_eq!(offset_of!(SuiseiPaneC, rect_y), 32);
    assert_eq!(offset_of!(SuiseiPaneC, rect_w), 36);
    assert_eq!(offset_of!(SuiseiPaneC, rect_h), 40);
    assert_eq!(size_of::<SuiseiPaneC>(), 44, "SuiseiPaneC stride");
}

/// `SuiseiPaneC::kind` is a `FileKind` discriminant, and the header hard-codes
/// those numbers as `SUISEI_PANE_*`. Nothing in the compiler connects the two,
/// so reordering the enum would silently make the face draw a PDF viewer over
/// an audio file.
///
/// `TERMINAL == 1` carries extra weight: this byte was an `is_terminal` bool,
/// and keeping the value means a stale face still routes terminals right.
#[test]
fn pane_kind_wire_values() {
    use suisei_core::media::FileKind;
    assert_eq!(FileKind::Text as u8, 0);
    assert_eq!(FileKind::Terminal as u8, 1, "was u8::from(is_terminal)");
    assert_eq!(FileKind::Image as u8, 2);
    assert_eq!(FileKind::Pdf as u8, 3);
    assert_eq!(FileKind::Audio as u8, 4);
    assert_eq!(FileKind::Binary as u8, 5);
    assert_eq!(FileKind::Model as u8, 6);
    // Appended, never inserted. A face built against an older engine reads an
    // unknown kind as Text — which is only safe while the numbers below it
    // stay put.
    assert_eq!(FileKind::Logic as u8, 7);
    // The default must be the one kind that is safe to be wrong about: a pane
    // that falls back to Text shows an editor, which is recoverable. One that
    // fell back to a viewer would hide a file the user meant to edit.
    assert_eq!(FileKind::default(), FileKind::Text);
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
    assert_eq!(
        offset_of!(SuiseiChromeSnapshot, dirty_buffer) % 4,
        0,
        "flags block should be u32-aligned after path fields"
    );

    // Per-tab ids sit right after the titles, and the split metadata after
    // them. `tab_ids` is what the face uses as list identity — an index cannot
    // serve, because a reorder leaves the index list unchanged.
    let tab_titles_end =
        offset_of!(SuiseiChromeSnapshot, tab_titles) + SUISEI_MAX_TABS * SUISEI_TITLE_CAP;
    assert_eq!(offset_of!(SuiseiChromeSnapshot, tab_ids), tab_titles_end);
    let tab_ids_end = tab_titles_end + SUISEI_MAX_TABS * size_of::<u64>();
    // Layout-tab metadata sits between the ids and the split block.
    assert_eq!(offset_of!(SuiseiChromeSnapshot, tab_groups), tab_ids_end);
    let tab_groups_end = tab_ids_end + SUISEI_MAX_TABS * size_of::<u64>();
    assert_eq!(
        offset_of!(SuiseiChromeSnapshot, tab_is_layout),
        tab_groups_end
    );
    let tab_is_layout_end = tab_groups_end + SUISEI_MAX_TABS;
    assert_eq!(
        offset_of!(SuiseiChromeSnapshot, tab_is_terminal),
        tab_is_layout_end
    );

    // Panes array follows split metadata
    let split_ratio_off = offset_of!(SuiseiChromeSnapshot, _pad_split_ratio);
    assert_eq!(
        split_ratio_off % 4,
        0,
        "split_ratio pad must stay f32-aligned"
    );
    assert_eq!(
        offset_of!(SuiseiChromeSnapshot, panes),
        split_ratio_off + 4,
        "panes immediately after split_ratio"
    );

    // `pane_titles` is last. It used to sit behind a 176 KiB packed `lines`
    // array that the face never decoded; removing that array is what took this
    // struct from 185,440 bytes to 9,312. Swift reads `pane_titles` by NAME
    // (`withUnsafeBytes(of: snap.pane_titles)`), so the move is safe — this
    // assertion is here to catch anyone reintroducing a bulk payload in front
    // of it.
    let panes_end =
        offset_of!(SuiseiChromeSnapshot, panes) + SUISEI_MAX_PANES * size_of::<SuiseiPaneC>();
    let vis_off = offset_of!(SuiseiChromeSnapshot, visible_line_count);
    assert!(vis_off >= panes_end, "visible_line_count after panes");
    let titles_off = offset_of!(SuiseiChromeSnapshot, pane_titles);
    assert!(titles_off > vis_off, "pane_titles after visible_line_count");
    assert_eq!(
        titles_off + SUISEI_MAX_PANES * SUISEI_TITLE_CAP,
        size_of::<SuiseiChromeSnapshot>(),
        "pane_titles is the last field — no bulk payload may follow it"
    );
}

/// `suisei_engine_open_panels` actually tracks panel state.
///
/// The face uses this to decide whether to pay for a panel snapshot at all —
/// the terminal's is 300 KiB. A mask stuck at zero would silently stop the
/// terminal from ever painting; a mask stuck at one would restore the waste it
/// exists to remove. Both are worth a test.
#[test]
fn open_panels_mask_follows_the_terminal_dock() {
    use suisei_engine::ffi::{
        SUISEI_PANEL_TERMINAL, suisei_engine_free, suisei_engine_new, suisei_engine_open_panels,
        suisei_engine_toggle_terminal_dock,
    };

    let e = suisei_engine_new();
    assert_eq!(
        suisei_engine_open_panels(e) & SUISEI_PANEL_TERMINAL,
        0,
        "terminal dock starts closed"
    );
    suisei_engine_toggle_terminal_dock(e);
    assert_ne!(
        suisei_engine_open_panels(e) & SUISEI_PANEL_TERMINAL,
        0,
        "opening the dock must set the bit — otherwise the face never pulls \
         the terminal snapshot and the shell paints nothing"
    );
    suisei_engine_toggle_terminal_dock(e);
    assert_eq!(
        suisei_engine_open_panels(e) & SUISEI_PANEL_TERMINAL,
        0,
        "closing the dock must clear the bit"
    );
    suisei_engine_free(e);
}

/// The panel bits are distinct powers of two.
///
/// They are `#define`d separately in `suisei_engine.h`; a collision here would
/// make two panels share a bit and one of them would go dark.
#[test]
fn panel_bits_are_distinct() {
    use suisei_engine::ffi::{
        SUISEI_PANEL_COMPLETIONS, SUISEI_PANEL_EXPLORER, SUISEI_PANEL_GIT_WB, SUISEI_PANEL_OUTLINE,
        SUISEI_PANEL_PALETTE, SUISEI_PANEL_PREVIEW, SUISEI_PANEL_SCM, SUISEI_PANEL_SEARCH,
        SUISEI_PANEL_SETTINGS, SUISEI_PANEL_TERMINAL,
    };
    let bits = [
        SUISEI_PANEL_EXPLORER,
        SUISEI_PANEL_PALETTE,
        SUISEI_PANEL_SEARCH,
        SUISEI_PANEL_COMPLETIONS,
        SUISEI_PANEL_TERMINAL,
        SUISEI_PANEL_SETTINGS,
        SUISEI_PANEL_SCM,
        SUISEI_PANEL_GIT_WB,
        SUISEI_PANEL_PREVIEW,
        SUISEI_PANEL_OUTLINE,
    ];
    let mut seen = 0u32;
    for b in bits {
        assert_eq!(b.count_ones(), 1, "{b:#x} is not a single bit");
        assert_eq!(seen & b, 0, "{b:#x} collides with an earlier panel bit");
        seen |= b;
    }
}

/// The chrome snapshot carries no per-line payload.
///
/// The GUI is a pull renderer: rows come from `suisei_engine_editor_band`, one
/// band per pane per paint. A packed `lines[]` here is dead weight that gets
/// memset on both sides of the ABI, twenty times a second — see
/// `docs/SUISEI-GPU-ARCHITECTURE.md` §2.1. This is the tripwire.
#[test]
fn chrome_snapshot_carries_no_line_payload() {
    let size = size_of::<SuiseiChromeSnapshot>();
    assert!(
        size < 16 * 1024,
        "chrome snapshot is {size} bytes — something bulk crept back in; \
         per-line data belongs in the band FFI, not here"
    );
    // The tab arrays dominate what is left (64 × 96 titles + 3 × 64 × 8).
    assert!(
        size > SUISEI_MAX_TABS * SUISEI_TITLE_CAP,
        "chrome snapshot lost the tab arrays"
    );
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
    // A 300 KiB `SuiseiTerminalSnapshot` used to be a second stack allocation
    // on this same path, pulled on every refresh while a terminal was open.
    // Print for visibility in CI logs.
    println!(
        "SuiseiChromeSnapshot: {} bytes ({:.1} KiB)",
        size,
        size as f64 / 1024.0
    );
    println!(
        "SuiseiEditorLineC:    {} bytes",
        size_of::<SuiseiEditorLineC>()
    );
    println!("SuiseiPaneC:          {} bytes", size_of::<SuiseiPaneC>());
}

// ─── SuiseiLogicSnapshot ──────────────────────────────────────────────────────
//
// Swift imports this struct from the header rather than reading offsets, which
// moves the risk rather than removing it: a field added on the Rust side and
// not in `suisei_engine.h` is a silent misread of everything after it. These
// numbers are the header's declaration, computed by hand — if they stop
// matching, one of the two files moved without the other.

#[test]
fn logic_snapshot_layout() {
    use suisei_engine::ffi::{
        SUISEI_LOGIC_LABEL, SUISEI_LOGIC_VALUE, SUISEI_MAX_LOGIC_ROWS, SuiseiLogicSnapshot,
    };
    assert_eq!(SUISEI_MAX_LOGIC_ROWS, 320);
    assert_eq!(SUISEI_LOGIC_LABEL, 192);
    assert_eq!(SUISEI_LOGIC_VALUE, 96);

    assert_eq!(offset_of!(SuiseiLogicSnapshot, ok), 0);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, live), 1);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, path), 4);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, note), 4 + SUISEI_PATH_CAP);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, lang), 676);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, row_count), 708);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, selected), 712);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, labels), 716);
    assert_eq!(
        offset_of!(SuiseiLogicSnapshot, values),
        716 + SUISEI_MAX_LOGIC_ROWS * SUISEI_LOGIC_LABEL
    );
    assert_eq!(offset_of!(SuiseiLogicSnapshot, kinds), 92876);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, depths), 93196);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, edges), 93516);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, flags), 93836);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, start_rows), 94156);
    assert_eq!(offset_of!(SuiseiLogicSnapshot, end_rows), 95436);
    assert_eq!(size_of::<SuiseiLogicSnapshot>(), 96716);
}
