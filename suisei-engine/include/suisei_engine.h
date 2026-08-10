/* Suisei engine C ABI — Swift face links against libsuisei_engine. */
#ifndef SUISEI_ENGINE_H
#define SUISEI_ENGINE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SUISEI_MAX_TABS 64
#define SUISEI_MAX_LINES 256
#define SUISEI_MAX_SPANS 24
#define SUISEI_MAX_PANES 4
#define SUISEI_TITLE_CAP 96
#define SUISEI_LINE_CAP 512
#define SUISEI_MSG_CAP 256
#define SUISEI_PATH_CAP 512
#define SUISEI_MODE_CAP 24

typedef struct SuiseiEngine SuiseiEngine;

enum {
  SUISEI_KEY_CHAR = 1,
  SUISEI_KEY_ENTER = 2,
  SUISEI_KEY_ESC = 3,
  SUISEI_KEY_BACKSPACE = 4,
  SUISEI_KEY_TAB = 5,
  SUISEI_KEY_BACKTAB = 6,
  SUISEI_KEY_DELETE = 7,
  SUISEI_KEY_LEFT = 8,
  SUISEI_KEY_RIGHT = 9,
  SUISEI_KEY_UP = 10,
  SUISEI_KEY_DOWN = 11,
  SUISEI_KEY_HOME = 12,
  SUISEI_KEY_END = 13,
  SUISEI_KEY_PAGE_UP = 14,
  SUISEI_KEY_PAGE_DOWN = 15,
  SUISEI_KEY_F = 16
};

enum {
  SUISEI_MOD_SHIFT = 1,
  SUISEI_MOD_CONTROL = 2,
  SUISEI_MOD_ALT = 4,
  SUISEI_MOD_SUPER = 8
};

typedef struct SuiseiSpanC {
  uint16_t start; /* visual col */
  uint16_t end;   /* exclusive visual col */
  uint8_t kind;   /* TokenKind code (1=kw …) */
  uint8_t _pad;
} SuiseiSpanC;

typedef struct SuiseiEditorLineC {
  uint32_t line_no;
  uint8_t is_cursor;
  uint8_t git_sign; /* 0 none, 1 add, 2 mod, 3 del */
  uint8_t span_count;
  uint8_t _pad;
  uint32_t caret_vcol;
  uint32_t caret_utf16;
  uint32_t sel_v0; /* UINT32_MAX = none */
  uint32_t sel_v1;
  uint32_t sel_u0;
  uint32_t sel_u1;
  char text[SUISEI_LINE_CAP];
  SuiseiSpanC spans[SUISEI_MAX_SPANS];
} SuiseiEditorLineC;

/* One split pane: slice into the packed lines[] array. */
typedef struct SuiseiPaneC {
  uint32_t tab_index;
  uint32_t scroll;
  uint32_t line_start; /* index into lines[] */
  uint32_t line_count;
  uint8_t focused;
  uint8_t is_terminal; /* pane runs its own shell */
  uint16_t term_gen;   /* pane shell content generation; face skips re-pull when unchanged */
  uint32_t doc_line_count; /* total lines in this pane's buffer */
  uint32_t hscroll;        /* per-pane horizontal pan (0 when wrap on) */
  /* Normalised rect within the editor area (0..1), from the layout tree. */
  float rect_x;
  float rect_y;
  float rect_w;
  float rect_h;
} SuiseiPaneC;

typedef struct SuiseiChromeSnapshot {
  uint64_t frame_gen;
  char mode_label[SUISEI_MODE_CAP];
  char message[SUISEI_MSG_CAP];
  char filename[SUISEI_PATH_CAP];
  char breadcrumbs[SUISEI_PATH_CAP];
  uint8_t dirty_buffer;
  uint8_t welcome;
  uint8_t explorer_open;
  uint8_t _pad_flags;
  uint32_t cursor_row;
  uint32_t cursor_col;
  uint32_t caret_vcol;
  uint8_t scroll_intent; /* 0 none, 1 restore, 2 navigate, 3 caret */
  uint8_t _pad_si0;
  uint8_t _pad_si1;
  uint8_t _pad_si2;
  uint32_t line_count;
  uint32_t scroll;
  uint32_t pct;
  float scroll_frac; /* sub-line residual (−1,1); smooth GUI paint */
  uint32_t hscroll;  /* visual cols; wrap_lines=0 */
  uint8_t wrap_lines;
  uint8_t _pad_h0;
  uint8_t _pad_h1;
  uint8_t _pad_h2;
  uint64_t buffer_version;
  /* TRUE tab count — may exceed SUISEI_MAX_TABS; only the first MAX are
     filled in the arrays below. Overflow = tab_count - filled. */
  uint32_t tab_count;
  uint32_t tab_active;
  uint8_t tab_dirty[SUISEI_MAX_TABS];
  char tab_titles[SUISEI_MAX_TABS][SUISEI_TITLE_CAP];
  /* Stable per-tab id — survives reorder, unlike the slot index. */
  uint64_t tab_ids[SUISEI_MAX_TABS];
  /* Layout this chip belongs to (0 = none); consecutive chips sharing a
     non-zero value are one folded layout. */
  uint64_t tab_groups[SUISEI_MAX_TABS];
  uint8_t tab_is_layout[SUISEI_MAX_TABS];
  /* 1 when the tab is a terminal (a shell runs in it). */
  uint8_t tab_is_terminal[SUISEI_MAX_TABS];
  /* Retired: the split shape lives in the per-pane rects (SuiseiPaneC).
     Kept as pads — renamed, not removed — so offsets after this hold. */
  uint8_t _pad_split_kind;
  uint8_t pane_count;
  uint8_t pane_focus;
  uint8_t _pad_split;
  float _pad_split_ratio;
  SuiseiPaneC panes[SUISEI_MAX_PANES];
  uint32_t visible_line_count;
  uint32_t _pad_vis;
  /* A packed `SuiseiEditorLineC lines[SUISEI_MAX_LINES]` used to sit here. It
     was 176,128 of this struct's 185,440 bytes and the face never decoded one
     of them: the GUI is a PULL renderer, so each canvas fetches its own rows
     through suisei_engine_editor_band() on draw. Removing it takes the struct
     to 9,312 bytes. SuiseiPaneC::line_start / line_count are consequently
     always 0; they remain only because the face reads that struct at hardcoded
     byte offsets. See docs/SUISEI-GPU-ARCHITECTURE.md §2.1. */
  /* Actual document title per pane. Unlike tab_titles, unified layout chips
     do not collapse these buffer identities. */
  char pane_titles[SUISEI_MAX_PANES][SUISEI_TITLE_CAP];
} SuiseiChromeSnapshot;

SuiseiEngine *suisei_engine_new(void);
void suisei_engine_free(SuiseiEngine *ptr);
/* Engine/core version string (static, NUL-terminated). */
const char *suisei_engine_version(void);
uint8_t suisei_engine_dispatch_key(
    SuiseiEngine *ptr, uint32_t code, uint32_t ch, uint8_t f_num, uint8_t mods);
/* Returns frame_gen after tick — face should refresh only when it changes. */
/* Typing fast-path eligibility: editor owns keys (modeless) — no snapshot. */
uint8_t suisei_engine_editor_accepts_text(const SuiseiEngine *ptr);
/* Cheap probe: completion popup open? (typing fast path) */
uint8_t suisei_engine_completions_open(const SuiseiEngine *ptr);
/* Which chrome panels are open, as a bitmask of SUISEI_PANEL_*.
   Every panel snapshot below is a fixed-size struct — the terminal's is 300
   KiB — so the face asks this FIRST and skips the copy for anything closed. */
#define SUISEI_PANEL_EXPLORER (1u << 0)
#define SUISEI_PANEL_PALETTE (1u << 1)
#define SUISEI_PANEL_SEARCH (1u << 2)
#define SUISEI_PANEL_COMPLETIONS (1u << 3)
#define SUISEI_PANEL_TERMINAL (1u << 4)
#define SUISEI_PANEL_SETTINGS (1u << 5)
#define SUISEI_PANEL_SCM (1u << 6)
#define SUISEI_PANEL_GIT_WB (1u << 7)
#define SUISEI_PANEL_PREVIEW (1u << 8)
#define SUISEI_PANEL_OUTLINE (1u << 9)
uint32_t suisei_engine_open_panels(const SuiseiEngine *ptr);
/* Document width in display columns — the face's horizontal scroll extent. */
uint32_t suisei_engine_content_cols(SuiseiEngine *ptr);
/* Tab-bar reorder: move the tab at `from` to sit at `to`. 1 = order changed. */
uint8_t suisei_engine_move_tab(SuiseiEngine *ptr, uint32_t from, uint32_t to);
/* Face acted on the scroll intent — clear it. */
void suisei_engine_clear_scroll_intent(SuiseiEngine *ptr);
/* Project auto-indexing: pre-parse a file into the syntax cache. */
uint8_t suisei_engine_prewarm_file(SuiseiEngine *ptr, const char *path);
uint32_t suisei_engine_cached_parses(const SuiseiEngine *ptr);
void suisei_engine_warm_grammars(SuiseiEngine *ptr);
void suisei_engine_terminal_input(SuiseiEngine *ptr, const char *text);

uint64_t suisei_engine_tick(SuiseiEngine *ptr, uint32_t dt_ms);
uint64_t suisei_engine_frame_gen(const SuiseiEngine *ptr);
/* css_* = editor body size in points; line_h / cell_w = glyph metrics (not the same). */
void suisei_engine_resize(
    SuiseiEngine *ptr, float css_w, float css_h, float line_h, float cell_w, float dpr);
uint8_t suisei_engine_running(const SuiseiEngine *ptr);
uint8_t suisei_engine_chrome(const SuiseiEngine *ptr, SuiseiChromeSnapshot *out);
uint8_t suisei_engine_open_path(SuiseiEngine *ptr, const char *path);
/* Replace workspace with a directory: 1 switched, 2 dirty tabs blocked, 0 invalid. */
uint8_t suisei_engine_switch_project(SuiseiEngine *ptr, const char *path);
void suisei_engine_scroll(SuiseiEngine *ptr, int32_t delta_lines);
/* Fractional lines (trackpad). Positive reveals content below. */
void suisei_engine_scroll_frac(SuiseiEngine *ptr, float delta_lines);
/* Horizontal pan in columns (no-op when wrap_lines). */
void suisei_engine_scroll_h(SuiseiEngine *ptr, int32_t delta_cols);
/* Absolute first-visible line + hscroll for native NSScrollView faces. */
void suisei_engine_scroll_to(SuiseiEngine *ptr, uint32_t line, uint32_t hscroll_cols);
/* Position-only sync while the native clip scrolls (no recompose). */
void suisei_engine_scroll_sync(SuiseiEngine *ptr, uint32_t line, uint32_t hscroll_cols);

/* ── Pull renderer: exact-range paint band ─────────────────────────── */
#define SUISEI_BAND_MAX 160

typedef struct SuiseiBandC {
  uint32_t start_row;
  uint32_t count;
  uint32_t doc_line_count;
  uint32_t _pad;
  SuiseiEditorLineC lines[SUISEI_BAND_MAX];
} SuiseiBandC;

uint8_t suisei_engine_editor_band(
    const SuiseiEngine *ptr, uint32_t pane, uint32_t start_row, uint32_t max_rows,
    SuiseiBandC *out);

void suisei_engine_split_resize(SuiseiEngine *e, uint32_t pane_a,
                                uint32_t pane_b, float delta);
void suisei_engine_toggle_breakpoint_line(SuiseiEngine *ptr, uint32_t line_1based);

/* ── Minimap overview (downsampled) ────────────────────────────────── */
#define SUISEI_MINIMAP_MAX 2048

typedef struct SuiseiMinimapC {
  uint32_t buckets;
  uint32_t total_lines;
  uint8_t indent[SUISEI_MINIMAP_MAX];
  uint8_t len[SUISEI_MINIMAP_MAX];
  uint8_t flags[SUISEI_MINIMAP_MAX]; /* 1 = git-changed */
} SuiseiMinimapC;

uint8_t suisei_engine_minimap(const SuiseiEngine *ptr, SuiseiMinimapC *out);
void suisei_engine_click(
    SuiseiEngine *ptr, uint32_t buffer_row, uint32_t visual_col, uint8_t select_word);
void suisei_engine_drag(SuiseiEngine *ptr, uint32_t buffer_row, uint32_t visual_col);
/* Hit-testing by UTF-16 offset (face measures with CoreText, core owns widths). */
void suisei_engine_click_utf16(
    SuiseiEngine *ptr, uint32_t buffer_row, uint32_t utf16_off, uint8_t select_word);
void suisei_engine_drag_utf16(SuiseiEngine *ptr, uint32_t buffer_row, uint32_t utf16_off);
void suisei_engine_mouse_up(SuiseiEngine *ptr);
uint8_t suisei_engine_hit_test(
    const SuiseiEngine *ptr,
    float local_x, float local_y,
    float gutter_px, float cell_px, float line_height_px,
    uint32_t *out_row, uint32_t *out_col);
void suisei_engine_save(SuiseiEngine *ptr);
void suisei_engine_save_as(SuiseiEngine *ptr, const char *path);
/* GUI-editor commands (standard Mac chords). */
void suisei_engine_undo(SuiseiEngine *ptr);
void suisei_engine_redo(SuiseiEngine *ptr);
void suisei_engine_set_system_appearance(SuiseiEngine *ptr, uint8_t is_dark);
/* 0 = clear Liquid Glass, 1 = tinted Liquid Glass. */
uint8_t suisei_engine_glass_style(const SuiseiEngine *ptr);
uint32_t suisei_engine_path_moved(SuiseiEngine *ptr, const char *old_path, const char *new_path);
void suisei_engine_select_all(SuiseiEngine *ptr);

/* ── GUI semantic editing commands ──────────────────────────────────── */
/* The face calls these INSTEAD of synthesizing vim keystrokes (i/Esc/c/d).
   Mode transitions are handled internally — the GUI never sees modes. */

/* Type a printable character at the cursor. Enters Insert if needed,
   replaces active selection (Mac text-field contract). No-op when a
   panel/terminal owns input. */
void suisei_engine_gui_type_char(SuiseiEngine *ptr, uint32_t ch);
/* Absolute UTF-16 document offset used by NSTextInputClient composition. */
uint64_t suisei_engine_caret_utf16_offset(const SuiseiEngine *ptr);
/* Backspace with Mac selection semantics (deletes selection if active). */
void suisei_engine_gui_delete_backward(SuiseiEngine *ptr);
/* Forward-delete with Mac selection semantics. */
void suisei_engine_gui_delete_forward(SuiseiEngine *ptr);
/* Esc semantic: collapse overlays/selection, land in Insert. */
void suisei_engine_gui_escape(SuiseiEngine *ptr);
/* Ensure Insert mode (click-to-type, open-file-to-type). */
void suisei_engine_gui_ensure_insert(SuiseiEngine *ptr);

void suisei_engine_find_open(SuiseiEngine *ptr);
void suisei_engine_find_step(SuiseiEngine *ptr, uint8_t forward);
void suisei_engine_find_set_input(SuiseiEngine *ptr, const char *input);
void suisei_engine_find_accept(SuiseiEngine *ptr);
void suisei_engine_find_cancel(SuiseiEngine *ptr);
void suisei_engine_palette_set_query(SuiseiEngine *ptr, const char *query);
void suisei_engine_paste_text(SuiseiEngine *ptr, const char *text);
void suisei_engine_terminal_resize(SuiseiEngine *ptr, uint32_t cols, uint32_t rows);
void suisei_engine_terminal_resize_pane(SuiseiEngine *ptr, uint32_t pane, uint32_t cols,
                                        uint32_t rows);
uint8_t suisei_engine_fold_layout(SuiseiEngine *e);
uint8_t suisei_engine_unfold_layout(SuiseiEngine *e);
uint8_t suisei_engine_activate_layout(SuiseiEngine *e, uint64_t id, uint64_t focus_doc);
uint8_t suisei_engine_toggle_layout_style(SuiseiEngine *e, uint64_t id);
/* Non-zero when a layout currently owns the desk (`App::active_layout`). */
uint64_t suisei_engine_active_layout_id(const SuiseiEngine *e);
void suisei_engine_toggle_terminal_dock(SuiseiEngine *e);
/* Pretty document preview. Direct, because ⇧⌘V means "paste" in a terminal. */
void suisei_engine_toggle_preview(SuiseiEngine *e);
/* Full terminal TAB (second call closes it). Direct, for the same reason. */
void suisei_engine_toggle_terminal_tab(SuiseiEngine *e);
void suisei_engine_focus_terminal(SuiseiEngine *ptr, uint8_t on);
/* Multi-session shells (active session lives in Core; parked ones keep running). */
uint32_t suisei_engine_terminal_sessions(const SuiseiEngine *ptr);
uint32_t suisei_engine_terminal_active_session(const SuiseiEngine *ptr);
void suisei_engine_terminal_new_session(SuiseiEngine *ptr);
void suisei_engine_terminal_select_session(SuiseiEngine *ptr, uint32_t idx);
void suisei_engine_terminal_close_session(SuiseiEngine *ptr, uint32_t idx);

#define SUISEI_MAX_EXPLORER 128
#define SUISEI_EXPLORER_NAME 160
#define SUISEI_MAX_XLC_OUT 48
#define SUISEI_XLC_LINE 240
#define SUISEI_XLC_INPUT 256

typedef struct SuiseiExplorerSnapshot {
  uint8_t open;
  uint32_t selected;
  uint32_t count;
  char cwd[SUISEI_PATH_CAP];
  uint8_t is_dir[SUISEI_MAX_EXPLORER];
  char names[SUISEI_MAX_EXPLORER][SUISEI_EXPLORER_NAME];
} SuiseiExplorerSnapshot;


uint8_t suisei_engine_explorer(const SuiseiEngine *ptr, SuiseiExplorerSnapshot *out);
void suisei_engine_explorer_activate(SuiseiEngine *ptr, uint32_t index);
void suisei_engine_explorer_select(SuiseiEngine *ptr, uint32_t index);
/* Docked Project nav: refresh tree without Mode::Explorer (editor keeps Normal keys). */
void suisei_engine_ensure_project_tree(SuiseiEngine *ptr);
/* Docked SCM nav: refresh without Mode::SourceControl. */
void suisei_engine_ensure_scm(SuiseiEngine *ptr);
void suisei_engine_close_scm(SuiseiEngine *ptr);
/* Jump caret to 1-based line (outline / jump bar). */
void suisei_engine_goto_line(SuiseiEngine *ptr, uint32_t line_1based);

#define SUISEI_MAX_OUTLINE 128
#define SUISEI_OUTLINE_NAME 120

typedef struct SuiseiOutlineSnapshot {
  uint32_t count;
  uint32_t rows[SUISEI_MAX_OUTLINE];
  uint8_t kinds[SUISEI_MAX_OUTLINE];
  uint8_t depths[SUISEI_MAX_OUTLINE];
  char names[SUISEI_MAX_OUTLINE][SUISEI_OUTLINE_NAME];
} SuiseiOutlineSnapshot;

uint8_t suisei_engine_outline(const SuiseiEngine *ptr, SuiseiOutlineSnapshot *out);

#define SUISEI_MAX_PALETTE 48
#define SUISEI_PALETTE_LABEL 160
#define SUISEI_PALETTE_DETAIL 200

typedef struct SuiseiPaletteSnapshot {
  uint8_t open;
  uint32_t selected;
  uint32_t count;
  char kind[32];
  char query[128];
  char labels[SUISEI_MAX_PALETTE][SUISEI_PALETTE_LABEL];
  char details[SUISEI_MAX_PALETTE][SUISEI_PALETTE_DETAIL];
} SuiseiPaletteSnapshot;

typedef struct SuiseiSearchSnapshot {
  uint8_t open;
  uint8_t forward;
  uint32_t match_count;
  uint32_t match_index;
  char input[256];
} SuiseiSearchSnapshot;

uint8_t suisei_engine_palette(const SuiseiEngine *ptr, SuiseiPaletteSnapshot *out);
uint8_t suisei_engine_search(const SuiseiEngine *ptr, SuiseiSearchSnapshot *out);
void suisei_engine_goto_tab(SuiseiEngine *ptr, uint32_t index);
void suisei_engine_close_tab(SuiseiEngine *ptr, uint32_t index);
void suisei_engine_open_blank_tab(SuiseiEngine *ptr);
/* Stable-id tab ops: strip slots are not buffer indices once a folded layout
   gathers or hides members, so the face addresses chips by BufferTab::id. */
void suisei_engine_goto_tab_id(SuiseiEngine *ptr, uint64_t id);
void suisei_engine_close_tab_id(SuiseiEngine *ptr, uint64_t id);
uint8_t suisei_engine_move_tab_ids(SuiseiEngine *ptr, uint64_t from, uint64_t to);
uint8_t suisei_engine_drop_layout(SuiseiEngine *ptr, uint64_t id);
void suisei_engine_split_vertical(SuiseiEngine *ptr);
void suisei_engine_split_horizontal(SuiseiEngine *ptr);
void suisei_engine_split_above(SuiseiEngine *ptr);
void suisei_engine_split_left(SuiseiEngine *ptr);
void suisei_engine_focus_next_pane(SuiseiEngine *ptr);
void suisei_engine_focus_pane(SuiseiEngine *ptr, uint32_t index);
void suisei_engine_close_focused_pane(SuiseiEngine *ptr);
void suisei_engine_palette_activate(SuiseiEngine *ptr, uint32_t index);
void suisei_engine_palette_select(SuiseiEngine *ptr, uint32_t index);

#define SUISEI_MAX_HINTS 24
#define SUISEI_HINT_KEY 16
#define SUISEI_HINT_DESC 48
#define SUISEI_MAX_COMP 20
#define SUISEI_COMP_LABEL 64
#define SUISEI_MAX_TERM_LINES 200
/* BYTES per row, not columns: rows carry truecolor SGR escapes. */
#define SUISEI_TERM_LINE 1536


typedef struct SuiseiCompletionsSnapshot {
  uint8_t open;
  uint32_t selected;
  uint32_t count;
  char prefix[64];
  char labels[SUISEI_MAX_COMP][SUISEI_COMP_LABEL];
  char details[SUISEI_MAX_COMP][SUISEI_COMP_LABEL];
} SuiseiCompletionsSnapshot;

typedef struct SuiseiTerminalSnapshot {
  uint8_t open;
  uint8_t full_panel;
  /* Split pane index for pane-bound full terminal; 0xFFFFFFFF = none / whole main. */
  uint32_t pane_bound;
  uint32_t count;
  uint32_t cursor_row; /* shell cursor within the emitted grid */
  uint32_t cursor_col;
  char lines[SUISEI_MAX_TERM_LINES][SUISEI_TERM_LINE];
} SuiseiTerminalSnapshot;

typedef struct SuiseiStatusExtra {
  char branch[64];
} SuiseiStatusExtra;

#define SUISEI_MAX_SETTINGS_ROWS 48
#define SUISEI_SETTINGS_LABEL 96
#define SUISEI_SETTINGS_VALUE 64
#define SUISEI_SETTINGS_GROUP 48
#define SUISEI_SETTINGS_DETAIL 192
#define SUISEI_SETTINGS_OPTIONS 96
#define SUISEI_MAX_SETTINGS_TABS 8

typedef struct SuiseiSettingsSnapshot {
  uint8_t open;
  uint8_t dirty;
  uint32_t page_index;
  uint32_t selected;
  uint32_t tab_count;
  uint32_t row_count;
  char status[160];
  char tabs[SUISEI_MAX_SETTINGS_TABS][24];
  uint8_t row_header[SUISEI_MAX_SETTINGS_ROWS];
  uint8_t row_selected[SUISEI_MAX_SETTINGS_ROWS];
  /* What each row IS (SettingRow::kind) — the face branches on this instead of
     matching display labels. 0 = prose row with no setting behind it. */
  uint32_t row_kind[SUISEI_MAX_SETTINGS_ROWS];
  /* Which theme / which language, for the indexed kinds. */
  uint32_t row_payload[SUISEI_MAX_SETTINGS_ROWS];
  /* Native Settings layout and control semantics supplied by Core. */
  uint32_t row_page[SUISEI_MAX_SETTINGS_ROWS];
  uint32_t row_control[SUISEI_MAX_SETTINGS_ROWS];
  uint32_t row_value_index[SUISEI_MAX_SETTINGS_ROWS];
  uint8_t row_advanced[SUISEI_MAX_SETTINGS_ROWS];
  char row_groups[SUISEI_MAX_SETTINGS_ROWS][SUISEI_SETTINGS_GROUP];
  char row_details[SUISEI_MAX_SETTINGS_ROWS][SUISEI_SETTINGS_DETAIL];
  char row_options[SUISEI_MAX_SETTINGS_ROWS][SUISEI_SETTINGS_OPTIONS];
  char row_labels[SUISEI_MAX_SETTINGS_ROWS][SUISEI_SETTINGS_LABEL];
  char row_values[SUISEI_MAX_SETTINGS_ROWS][SUISEI_SETTINGS_VALUE];
} SuiseiSettingsSnapshot;

/* Packed 0x00RRGGBB theme colors for live face paint */
typedef struct SuiseiThemeSnapshot {
  char name[32];
  uint32_t editor_bg;
  uint32_t fg;
  uint32_t dim;
  uint32_t accent;
  uint32_t selection;
  uint32_t caret;
  uint32_t status_bg;
  uint32_t keyword;
  uint32_t string_col;
  uint32_t comment;
  uint32_t number;
  uint32_t type_name;
  uint32_t function;
  uint32_t macro_name;
  uint32_t namespace;
  uint32_t parameter;
  uint32_t property;
  uint32_t constant;
  uint32_t operator;
  uint32_t punctuation;
} SuiseiThemeSnapshot;

uint8_t suisei_engine_completions(const SuiseiEngine *ptr, SuiseiCompletionsSnapshot *out);
uint8_t suisei_engine_terminal(const SuiseiEngine *ptr, SuiseiTerminalSnapshot *out);
uint8_t suisei_engine_terminal_for_pane(const SuiseiEngine *e, uint32_t pane,
                                        SuiseiTerminalSnapshot *out);
/* Scroll the terminal panel through its scrollback; positive = older output. */
void suisei_engine_terminal_scroll(SuiseiEngine *ptr, int32_t delta_rows);
void suisei_engine_terminal_scroll_pane(SuiseiEngine *ptr, uint32_t pane, int32_t delta_rows);
uint8_t suisei_engine_status_extra(const SuiseiEngine *ptr, SuiseiStatusExtra *out);
uint8_t suisei_engine_settings(const SuiseiEngine *ptr, SuiseiSettingsSnapshot *out);
uint8_t suisei_engine_theme(const SuiseiEngine *ptr, SuiseiThemeSnapshot *out);
void suisei_engine_settings_select(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_settings_activate(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_settings_set_value(SuiseiEngine *ptr, uint32_t row, uint32_t value);
void suisei_engine_settings_set_highlight_color(SuiseiEngine *ptr, const char *value);
void suisei_engine_settings_goto_page(SuiseiEngine *ptr, uint32_t page);
void suisei_engine_settings_save(SuiseiEngine *ptr);

#define SUISEI_MAX_SCM 48
#define SUISEI_SCM_PATH 160
#define SUISEI_MAX_SCM_GRAPH 40
#define SUISEI_GRAPH_LINE 200
#define SUISEI_MAX_GIT_WB_LINES 64
#define SUISEI_GIT_WB_LINE 220
#define SUISEI_MAX_GIT_TABS 8

typedef struct SuiseiScmSnapshot {
  uint8_t open;
  uint32_t staged_count;
  uint32_t change_count;
  uint32_t selected;
  uint32_t graph_count;
  char branch[64];
  char status[160];
  uint8_t staged_flags[SUISEI_MAX_SCM]; /* 1=staged list entries */
  char marks[SUISEI_MAX_SCM];
  char paths[SUISEI_MAX_SCM][SUISEI_SCM_PATH];
  /* graph rows */
  uint8_t graph_selected[SUISEI_MAX_SCM_GRAPH];
  char graph_lines[SUISEI_MAX_SCM_GRAPH][SUISEI_GRAPH_LINE];
  /* staged packed first [0..staged_count), then changes */
} SuiseiScmSnapshot;

#define SUISEI_MAX_GIT_CHIPS 9
#define SUISEI_MAX_GIT_COL 64
#define SUISEI_MAX_GIT_WORKTREE 160
#define SUISEI_MAX_GIT_HISTORY 80
#define SUISEI_MAX_GIT_BRANCHES 160
#define SUISEI_MAX_GIT_FILES 160
#define SUISEI_MAX_GIT_STASHES 40
#define SUISEI_MAX_GIT_REMOTES 24
#define SUISEI_GIT_PATH 320
#define SUISEI_GIT_SUBJECT 240
#define SUISEI_GIT_AUTHOR 96
#define SUISEI_GIT_EMAIL 160

typedef struct SuiseiGitWbSnapshot {
  uint8_t open;
  uint8_t docked;
  uint8_t loading;
  uint32_t tab_index;
  uint32_t chip_count;
  uint32_t changes_count;
  uint32_t log_count;
  uint32_t files_count;
  uint32_t special_count;
  char branch[64];
  char message[160];
  uint8_t chip_active[SUISEI_MAX_GIT_CHIPS];
  uint8_t chip_keys[SUISEI_MAX_GIT_CHIPS];
  char chip_labels[SUISEI_MAX_GIT_CHIPS][24];
  char col_changes[SUISEI_MAX_GIT_COL][SUISEI_GIT_WB_LINE];
  char col_log[SUISEI_MAX_GIT_COL][SUISEI_GIT_WB_LINE];
  char col_files[SUISEI_MAX_GIT_COL][SUISEI_GIT_WB_LINE];
  char special[SUISEI_MAX_GIT_COL][SUISEI_GIT_WB_LINE];
  uint32_t selected_change;
  uint32_t worktree_count;
  uint32_t history_count;
  uint32_t history_selected;
  uint32_t branch_count;
  uint32_t branch_selected;
  uint32_t commit_file_count;
  uint32_t commit_file_selected;
  uint32_t stash_count;
  uint32_t remote_count;
  uint8_t commit_detail_valid;
  char root_path[SUISEI_PATH_CAP];
  char repository_name[SUISEI_GIT_AUTHOR];
  char author_name[SUISEI_GIT_AUTHOR];
  char author_email[SUISEI_GIT_EMAIL];
  uint8_t worktree_staged[SUISEI_MAX_GIT_WORKTREE];
  char worktree_status[SUISEI_MAX_GIT_WORKTREE];
  char worktree_paths[SUISEI_MAX_GIT_WORKTREE][SUISEI_GIT_PATH];
  char history_hashes[SUISEI_MAX_GIT_HISTORY][48];
  char history_shorts[SUISEI_MAX_GIT_HISTORY][16];
  char history_subjects[SUISEI_MAX_GIT_HISTORY][SUISEI_GIT_SUBJECT];
  char history_authors[SUISEI_MAX_GIT_HISTORY][SUISEI_GIT_AUTHOR];
  char history_whens[SUISEI_MAX_GIT_HISTORY][64];
  uint8_t branch_current[SUISEI_MAX_GIT_BRANCHES];
  uint8_t branch_remote[SUISEI_MAX_GIT_BRANCHES];
  char branch_names[SUISEI_MAX_GIT_BRANCHES][SUISEI_GIT_PATH];
  char branch_upstreams[SUISEI_MAX_GIT_BRANCHES][SUISEI_GIT_PATH];
  char commit_file_status[SUISEI_MAX_GIT_FILES];
  uint32_t commit_file_insertions[SUISEI_MAX_GIT_FILES];
  uint32_t commit_file_deletions[SUISEI_MAX_GIT_FILES];
  char commit_file_paths[SUISEI_MAX_GIT_FILES][SUISEI_GIT_PATH];
  char detail_hash[48];
  char detail_short[16];
  char detail_subject[SUISEI_GIT_SUBJECT];
  char detail_author[SUISEI_GIT_AUTHOR];
  char detail_email[SUISEI_GIT_EMAIL];
  char detail_date[64];
  char detail_body[512];
  uint32_t detail_insertions;
  uint32_t detail_deletions;
  char stashes[SUISEI_MAX_GIT_STASHES][SUISEI_GIT_WB_LINE];
  char remote_names[SUISEI_MAX_GIT_REMOTES][SUISEI_GIT_AUTHOR];
  char remote_urls[SUISEI_MAX_GIT_REMOTES][SUISEI_GIT_PATH];
} SuiseiGitWbSnapshot;

uint8_t suisei_engine_scm(const SuiseiEngine *ptr, SuiseiScmSnapshot *out);
void suisei_engine_scm_select(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_scm_activate(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_scm_toggle_stage(SuiseiEngine *ptr, uint32_t row);
uint8_t suisei_engine_git_wb(const SuiseiEngine *ptr, SuiseiGitWbSnapshot *out);
uint64_t suisei_engine_git_wb_generation(const SuiseiEngine *ptr);
uint64_t suisei_engine_git_wb_diff_generation(const SuiseiEngine *ptr);
uint64_t suisei_engine_git_wb_diff_byte_count(const SuiseiEngine *ptr);
uint64_t suisei_engine_git_wb_diff_copy(const SuiseiEngine *ptr, char *out,
                                        uint64_t capacity);
/* key 1..=9 maps to xei toolbar chips */
void suisei_engine_git_wb_set_tab(SuiseiEngine *ptr, uint32_t key);
void suisei_engine_git_wb_select_change(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_git_wb_select_history(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_git_wb_select_commit_file(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_git_wb_select_special(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_git_wb_select_branch_history(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_git_wb_refresh_window(SuiseiEngine *ptr);
void suisei_engine_git_wb_toggle_stage(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_git_wb_stage_all(SuiseiEngine *ptr);
void suisei_engine_git_wb_unstage_all(SuiseiEngine *ptr);
void suisei_engine_git_wb_commit(SuiseiEngine *ptr, const char *message,
                                 uint8_t amend);
void suisei_engine_git_wb_stash(SuiseiEngine *ptr);
void suisei_engine_git_wb_discard_change(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_git_wb_open_window(SuiseiEngine *ptr);
void suisei_engine_git_wb_focus_window(SuiseiEngine *ptr);
void suisei_engine_git_wb_close_window(SuiseiEngine *ptr);
void suisei_engine_git_wb_checkout_selected_branch(SuiseiEngine *ptr);
void suisei_engine_git_wb_create_branch(SuiseiEngine *ptr, const char *name);
void suisei_engine_git_wb_delete_selected_branch(SuiseiEngine *ptr);

/* Breakpoints navigator (replaces Find in the icon rail). */
#define SUISEI_MAX_BREAKPOINTS 128
#define SUISEI_BP_NAME 96

typedef struct SuiseiBreakpointSnapshot {
  uint32_t count;
  uint32_t lines[SUISEI_MAX_BREAKPOINTS]; /* 1-based */
  uint8_t verified[SUISEI_MAX_BREAKPOINTS];
  uint8_t has_condition[SUISEI_MAX_BREAKPOINTS];
  uint8_t has_log[SUISEI_MAX_BREAKPOINTS];
  char paths[SUISEI_MAX_BREAKPOINTS][SUISEI_PATH_CAP];
  char names[SUISEI_MAX_BREAKPOINTS][SUISEI_BP_NAME]; /* file basename */
  char conditions[SUISEI_MAX_BREAKPOINTS][96];
} SuiseiBreakpointSnapshot;

uint8_t suisei_engine_breakpoints(const SuiseiEngine *ptr, SuiseiBreakpointSnapshot *out);
/* Open path + jump to 1-based line. */
void suisei_engine_goto_breakpoint(SuiseiEngine *ptr, const char *path, uint32_t line_1based);
/* Remove BP at path + 1-based line (no-op if missing). */
void suisei_engine_remove_breakpoint(SuiseiEngine *ptr, const char *path, uint32_t line_1based);
/* Toggle BP on current cursor file/line (F9). */
void suisei_engine_toggle_breakpoint_cursor(SuiseiEngine *ptr);

/* Pretty preview (Ctrl+Shift+V / :preview) — face pages with start offset. */
#define SUISEI_MAX_PREVIEW 128
#define SUISEI_PREVIEW_LINE 512

typedef struct SuiseiPreviewSnapshot {
  uint8_t open;
  uint8_t kind; /* 0 none · 1 md · 2 json · 3 plain · 4 image · 5 csv · 6 npy · 7 audio */
  uint32_t scroll;
  uint32_t hscroll;
  uint32_t total; /* full document line count in scene */
  uint32_t count; /* lines filled in this chunk */
  uint32_t start; /* first line index of this chunk */
  uint8_t styles[SUISEI_MAX_PREVIEW];
  char lines[SUISEI_MAX_PREVIEW][SUISEI_PREVIEW_LINE];
} SuiseiPreviewSnapshot;

/* Load preview lines starting at `start` (0-based). Loop until start+count >= total. */
uint8_t suisei_engine_preview(const SuiseiEngine *ptr, uint32_t start, SuiseiPreviewSnapshot *out);

/* Issue navigator — diagnostics the core has always had but never exposed. */
#define SUISEI_MAX_DIAGS 200
#define SUISEI_DIAG_MSG 240

typedef struct SuiseiDiagnosticsSnapshot {
  uint32_t count;
  uint32_t rows[SUISEI_MAX_DIAGS];
  uint32_t cols[SUISEI_MAX_DIAGS];
  /* 0 error / 1 warning / 2 info / 3 hint */
  uint8_t severities[SUISEI_MAX_DIAGS];
  char messages[SUISEI_MAX_DIAGS][SUISEI_DIAG_MSG];
} SuiseiDiagnosticsSnapshot;

uint8_t suisei_engine_diagnostics(const SuiseiEngine *ptr,
                                  SuiseiDiagnosticsSnapshot *out);
/* Fingerprint of the diagnostic set (0 = none). The snapshot above is 48.6 KiB
   and diagnostics change when a language server answers, not on the tick — so
   compare this first and only pull when it moves. */
uint64_t suisei_engine_diagnostics_fingerprint(const SuiseiEngine *ptr);

/* Find navigator. Takes NO engine pointer on purpose — safe to call off the
   main thread, which is what keeps a project-wide grep from freezing the UI. */
#define SUISEI_MAX_HITS 300
#define SUISEI_HIT_PATH 512
#define SUISEI_HIT_LINE 240

typedef struct SuiseiSearchHitsSnapshot {
  uint32_t count;
  uint8_t truncated;
  uint32_t rows[SUISEI_MAX_HITS];
  uint32_t cols[SUISEI_MAX_HITS];
  char paths[SUISEI_MAX_HITS][SUISEI_HIT_PATH];
  char lines[SUISEI_MAX_HITS][SUISEI_HIT_LINE];
} SuiseiSearchHitsSnapshot;

uint8_t suisei_engine_search_project(const char *root, const char *pattern,
                                     SuiseiSearchHitsSnapshot *out);

/* Find All References — LSP textDocument/references. Asynchronous like hover:
   request, then poll `references` until `ready` != 0 (0 refs then reads as
   done, not still-waiting). Same list shape as project search. */
#define SUISEI_MAX_REFS 500
#define SUISEI_REF_PATH 512
#define SUISEI_REF_LINE 240

typedef struct SuiseiReferencesSnapshot {
  uint32_t count;
  uint8_t ready;
  uint8_t truncated;
  uint8_t _pad0;
  uint8_t _pad1;
  uint32_t rows[SUISEI_MAX_REFS];
  uint32_t cols[SUISEI_MAX_REFS];
  char paths[SUISEI_MAX_REFS][SUISEI_REF_PATH];
  char lines[SUISEI_MAX_REFS][SUISEI_REF_LINE];
} SuiseiReferencesSnapshot;

void suisei_engine_request_references(SuiseiEngine *ptr);
uint8_t suisei_engine_references(const SuiseiEngine *ptr,
                                 SuiseiReferencesSnapshot *out);

/* Quick Help inspector — LSP hover. Asynchronous: request, then poll. */
#define SUISEI_HOVER_TEXT 4096

void suisei_engine_request_hover(SuiseiEngine *ptr);
uint8_t suisei_engine_hover_text(const SuiseiEngine *ptr, char *out, uint32_t cap);

/* LSP face surfaces — same App methods the TUI dispatches (gd / format / rename / code actions). */
void suisei_engine_format_document(SuiseiEngine *ptr);
void suisei_engine_goto_definition(SuiseiEngine *ptr);
void suisei_engine_rename_symbol(SuiseiEngine *ptr, const char *new_name);
void suisei_engine_code_actions(SuiseiEngine *ptr);

/* Project Find replace — freestanding (no engine lock); uses atomic writes. */
/* Returns 1 if a replacement was made, 0 if no match / error. */
uint8_t suisei_engine_replace_in_file(const char *path, uint32_t row,
                                      const char *query, const char *replace);
/* Returns number of replacements written. */
uint32_t suisei_engine_replace_all_in_file(const char *path, const char *query,
                                          const char *replace);

/* Forward a mouse event to a terminal's inner app (xterm tracking).
   pane = 0xFFFF targets the dock. Returns 1 when consumed (the face should
   NOT also act — e.g. wheel scrollback). button: 0 left, 1 middle, 2 right,
   64 wheel-up, 65 wheel-down. x/y: 1-based cells. */
uint8_t suisei_engine_terminal_mouse(SuiseiEngine *ptr, uint32_t pane, uint8_t button,
                                     uint16_t x, uint16_t y, uint8_t pressed,
                                     uint8_t motion);

/* ── Session persistence ───────────────────────────────────────────────── */

/* Restore the previous session's files + cursors (call once at startup). */
void suisei_engine_restore_session(SuiseiEngine *ptr);
/* Persist open files + cursors for the next launch. */
void suisei_engine_save_session(const SuiseiEngine *ptr);

/* ── Shadow WAL recovery (D0) ──────────────────────────────────────────── */

/* Number of pending crash-recovery entries found on startup. */
uint32_t suisei_engine_recovery_count(const SuiseiEngine *ptr);
/* Get the file path of recovery entry idx. Writes NUL-terminated UTF-8
   into buf (max buf_len bytes). Returns 1 on success, 0 if out of range. */
uint8_t suisei_engine_recovery_path(const SuiseiEngine *ptr, uint32_t idx,
                                    char *buf, uint32_t buf_len);
/* Accept recovery entry idx: open file from disk, replace buffer with
   journaled text, restore cursor/scroll, mark as modified (unsaved).
   Returns 1 on success, 0 if out of range. */
uint8_t suisei_engine_recovery_accept(SuiseiEngine *ptr, uint32_t idx);
/* Discard recovery entry idx (user chose not to recover). Deletes the WAL file. */
void suisei_engine_recovery_discard(SuiseiEngine *ptr, uint32_t idx);

#ifdef __cplusplus
}
#endif

#endif /* SUISEI_ENGINE_H */
