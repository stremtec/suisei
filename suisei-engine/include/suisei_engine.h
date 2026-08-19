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
  uint8_t debug_sign; /* 0x01 stopped here, 0x02 the frame being read */
  uint32_t caret_vcol;
  uint32_t caret_utf16;
  uint32_t sel_v0; /* UINT32_MAX = none */
  uint32_t sel_v1;
  uint32_t sel_u0;
  uint32_t sel_u1;
  char text[SUISEI_LINE_CAP];
  SuiseiSpanC spans[SUISEI_MAX_SPANS];
  /* Fold marker: 0 none, 1 an open fold starts here, 2 a closed one does.
     Appended, never inserted — the Swift decoder reads the fields above at
     hardcoded offsets. */
  uint8_t fold;
  uint8_t _fold_pad;
  uint16_t fold_lines;
} SuiseiEditorLineC;

/* SuiseiPaneC::kind — mirrors suisei_core::media::FileKind. */
#define SUISEI_PANE_TEXT 0
#define SUISEI_PANE_TERMINAL 1
#define SUISEI_PANE_IMAGE 2
#define SUISEI_PANE_PDF 3
#define SUISEI_PANE_AUDIO 4
#define SUISEI_PANE_BINARY 5
#define SUISEI_PANE_MODEL 6

/* One split pane: slice into the packed lines[] array. */
typedef struct SuiseiPaneC {
  uint32_t tab_index;
  uint32_t scroll;
  uint32_t line_start; /* index into lines[] */
  uint32_t line_count;
  uint8_t focused;
  /* SuiseiPaneKind — what the face should draw here. Was an is_terminal bool
     in this byte, and TERMINAL == 1 keeps that wire value. */
  uint8_t kind;
  uint16_t term_gen; /* pane shell content generation; face skips re-pull when unchanged */
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
  /* Gutter counts from the caret instead of from 1. Was _pad_h0. */
  uint8_t relative_number;
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
   Every panel snapshot below is a fixed-size struct, so the face asks this
   FIRST and skips the copy for anything closed. (The terminal's was 300 KiB
   and the reason this probe exists; it is gone, but the others still pay.) */
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

/* `wrap_cols` = columns a wrapped row may use; 0 = do not wrap. The FACE
   decides it — only the face knows a pane's width in points, the cell width,
   the gutter and what overlays the right edge. A wrapped line arrives as
   several rows sharing one line_no, continuations flagged in git_sign & 0x80. */
/* `wide_ratio` = how wide a "two-cell" glyph really paints, in hundredths of a
   narrow cell. The editor draws with real advances, not on a grid, and with the
   shipped font Hangul is 1.44 cells — budgeting it at 2 broke Korean lines a
   quarter of a pane early. It rides beside `wrap_cols` because the two are one
   fact (how this pane measures a row); as a pushed setting it had an ordering
   question, and lost it. */
uint8_t suisei_engine_editor_band(
    const SuiseiEngine *ptr, uint32_t pane, uint32_t start_row, uint32_t max_rows,
    uint16_t wrap_cols, uint16_t wide_ratio, SuiseiBandC *out);


/* Soft-wrap geometry for the same pane at the same columns. Cached per pane
   against the document version, so asking all three per frame builds nothing.
   With cols == 0 they answer as if each line were one row. */
uint32_t suisei_engine_wrap_total_rows(const SuiseiEngine *ptr, uint32_t pane,
                                       uint16_t cols, uint16_t wide_ratio);
uint32_t suisei_engine_wrap_visual_of(const SuiseiEngine *ptr, uint32_t pane,
                                      uint16_t cols, uint16_t wide_ratio,
                                      uint32_t row);
/* Buffer row in the high 32 bits, segment within it in the low 32. */
uint64_t suisei_engine_wrap_buffer_at(const SuiseiEngine *ptr, uint32_t pane,
                                      uint16_t cols, uint16_t wide_ratio,
                                      uint32_t visual_row);

void suisei_engine_split_resize(SuiseiEngine *e, uint32_t pane_a,
                                uint32_t pane_b, float delta);
void suisei_engine_toggle_breakpoint_line(SuiseiEngine *ptr, uint32_t line_1based);

/* Act on the one git change covering a line: 0 stage, 1 unstage, 2 discard.
   Addressed by line, which is what a gutter click has. Returns 0 on success;
   the message either way is on `chrome`. */
int32_t suisei_engine_apply_hunk(SuiseiEngine *ptr, uint32_t line_1based, uint8_t action);

/* The text the change on this line REPLACED, newline-joined and NUL-terminated.
   Returns the bytes required (including the NUL); pass capacity 0 to ask for
   the size. Zero means no change there, or a change that removed nothing.
   These lines are in no buffer, so this is the only way to reach them. */
uint64_t suisei_engine_hunk_removed_text(const SuiseiEngine *ptr, uint32_t line_1based,
                                         char *out, uint64_t capacity);

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
/* Same, for the document in one pane. The call above answers for the live
   document — right for the focused pane, wrong for every other one. */
uint8_t suisei_engine_minimap_for_pane(const SuiseiEngine *ptr, uint32_t idx,
                                       SuiseiMinimapC *out);
void suisei_engine_click(
    SuiseiEngine *ptr, uint32_t buffer_row, uint32_t visual_col, uint8_t select_word);
void suisei_engine_drag(SuiseiEngine *ptr, uint32_t buffer_row, uint32_t visual_col);
/* Hit-testing by UTF-16 offset (face measures with CoreText, core owns widths). */
void suisei_engine_click_utf16(
    SuiseiEngine *ptr, uint32_t buffer_row, uint32_t utf16_off, uint8_t select_word);
void suisei_engine_drag_utf16(SuiseiEngine *ptr, uint32_t buffer_row, uint32_t utf16_off);
/* Source updates: clone the tagged commit, build it here, and exchange the
   bundle at the next launch. `_apply` is the only call that can change the
   installed app, and it is one atomic rename. */
uint8_t suisei_engine_update_start(SuiseiEngine *ptr, const char *app_path);
uint32_t suisei_engine_update_blockers(const SuiseiEngine *ptr, const char *app_path,
                                       char *out, uint32_t cap);
uint8_t suisei_engine_update_phase(const SuiseiEngine *ptr);
uint32_t suisei_engine_update_detail(const SuiseiEngine *ptr, char *out, uint32_t cap);
uint32_t suisei_engine_update_fraction(const SuiseiEngine *ptr); /* 0..10000 */
uint32_t suisei_engine_update_eta(const SuiseiEngine *ptr);      /* seconds, UINT32_MAX = unknown */
uint32_t suisei_engine_update_headline(const SuiseiEngine *ptr, char *out, uint32_t cap);
uint32_t suisei_engine_update_pending(const char *current, char *out, uint32_t cap);
uint8_t suisei_engine_update_apply(const char *current, const char *app_path,
                                   char *err, uint32_t cap);

/* Column selection. The block gesture speaks VISUAL columns — a rectangle is a
   rectangle on the screen, and a tab is one character and several columns. */
void suisei_engine_block_click(SuiseiEngine *ptr, uint32_t buffer_row, uint32_t visual_col);
void suisei_engine_block_drag(SuiseiEngine *ptr, uint32_t buffer_row, uint32_t visual_col);
void suisei_engine_block_extend_rows(SuiseiEngine *ptr, int32_t delta);

/* Code folding. The row is a BUFFER row — the gutter is clicked where the eye
   is, not where the caret is. */
void suisei_engine_fold_toggle_row(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_fold_at_cursor(SuiseiEngine *ptr, uint8_t close);
void suisei_engine_fold_all(SuiseiEngine *ptr, uint8_t close);
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
/* Caret position without the chrome snapshot: 1-based row in the high 32 bits,
   visual column in the low 32. The typing fast path publishes no chrome, and
   the face still has to scroll the caret into view on every keystroke. */
uint64_t suisei_engine_caret_row_vcol(const SuiseiEngine *ptr);
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
/* ── project.suiseiprj ─────────────────────────────────────────────────────
   The file that says "this folder is a project". Carries a stable project_id
   and a display name — and deliberately no members, roles or tokens: a file in
   a repository is editable by anyone who can edit the repository, so a member
   list in it is a permission you can grant yourself with a text editor.
   Freestanding: no engine needed, and none of these open a document. */
/* Write the marker if absent; keep the existing identity if present. */
uint8_t suisei_project_mark(const char *dir);
uint8_t suisei_project_is_marked(const char *dir);
/* Project root at or ABOVE `path`; 0 when there is none. A non-zero answer
   that is not `path` itself means `path` is inside a project. */
uint8_t suisei_project_root_of(const char *path, char *out, uint32_t cap);
uint8_t suisei_project_name(const char *dir, char *out, uint32_t cap);

/* Where a shell with no pane of its own should start — the docked strip's.
   The face runs those shells, and asks for this each time it opens one. */
uint8_t suisei_engine_terminal_cwd(const SuiseiEngine *ptr, char *out, uint32_t cap);

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
  /* Appended: an older face reads the same first five fields. */
  uint8_t replace_open;
  char replace_input[256];
} SuiseiSearchSnapshot;

uint8_t suisei_engine_palette(const SuiseiEngine *ptr, SuiseiPaletteSnapshot *out);
uint8_t suisei_engine_search(const SuiseiEngine *ptr, SuiseiSearchSnapshot *out);
/* Find and replace, in the buffer on screen. */
void suisei_engine_find_set_replace_open(SuiseiEngine *ptr, uint8_t open);
void suisei_engine_find_set_replace(SuiseiEngine *ptr, const char *text);
uint8_t suisei_engine_replace_current(SuiseiEngine *ptr);
uint32_t suisei_engine_replace_all(SuiseiEngine *ptr);
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

typedef struct SuiseiCompletionsSnapshot {
  uint8_t open;
  uint32_t selected;
  uint32_t count;
  char prefix[64];
  char labels[SUISEI_MAX_COMP][SUISEI_COMP_LABEL];
  char details[SUISEI_MAX_COMP][SUISEI_COMP_LABEL];
} SuiseiCompletionsSnapshot;

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
  uint32_t current_line;
  uint32_t invisibles;
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
  /* Chrome. The palette drives the whole app, not just the text. */
  uint32_t window_bg;
  uint32_t border;
  uint32_t panel_bg;
  uint32_t panel_border;
  uint32_t panel_sel_bg;
  uint32_t panel_sel_fg;
  uint32_t explorer_bg;
  uint32_t explorer_fg;
  uint32_t explorer_selected;
  uint32_t status_fg;
  uint32_t muted;
  uint32_t success;
  uint32_t warning;
  uint32_t error;
  uint32_t accent_fg;
  uint32_t search_bg;
  uint32_t completion_bg;
  uint32_t completion_selected;
  uint32_t completion_border;
  uint32_t terminal_bg;
  uint32_t git_add_bg;
  uint32_t git_del_bg;
  uint32_t git_hunk;
  uint32_t model_bg;
  uint32_t debug_stop;
} SuiseiThemeSnapshot;

uint8_t suisei_engine_completions(const SuiseiEngine *ptr, SuiseiCompletionsSnapshot *out);
uint8_t suisei_engine_status_extra(const SuiseiEngine *ptr, SuiseiStatusExtra *out);
uint8_t suisei_engine_settings(const SuiseiEngine *ptr, SuiseiSettingsSnapshot *out);
uint8_t suisei_engine_theme(const SuiseiEngine *ptr, SuiseiThemeSnapshot *out);
void suisei_engine_settings_select(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_settings_activate(SuiseiEngine *ptr, uint32_t row);
void suisei_engine_settings_set_value(SuiseiEngine *ptr, uint32_t row, uint32_t value);
void suisei_engine_settings_set_highlight_color(SuiseiEngine *ptr, const char *value);
void suisei_engine_settings_goto_page(SuiseiEngine *ptr, uint32_t page);
void suisei_engine_settings_save(SuiseiEngine *ptr);

/* ── Theme colours ─────────────────────────────────────────────────────────
 * The addressable colours of a theme, and the user's edits to them. A token is
 * addressed by its INDEX in the table below, so the table's order is ABI.
 * `suisei_engine_theme_tokens` writes "key|Label" one per line, in that order;
 * read it once and keep the order rather than hard-coding a list, or an
 * appended token will silently shift what every index means.
 * Setting an empty value or "default" clears the edit and restores the
 * theme's own colour.                                                       */
uint8_t suisei_engine_theme_tokens(char *out, size_t cap);

/* Themes. `suisei_engine_theme_catalogue` writes "name|Label|isCustom" one per
 * line: built-ins in catalogue order, then the user's own. Save-as returns 0
 * and an empty `out` when the name is blank, already taken, or shadows a
 * built-in. Built-ins cannot be deleted; deleting the theme in use falls back
 * to the palette it was built on.                                            */
uint8_t suisei_engine_theme_catalogue(const SuiseiEngine *ptr, char *out, size_t cap);
uint8_t suisei_engine_selected_theme(const SuiseiEngine *ptr, char *out, size_t cap);
void suisei_engine_settings_select_theme(SuiseiEngine *ptr, const char *name);
uint8_t suisei_engine_settings_save_theme_as(SuiseiEngine *ptr, const char *name,
                                             char *out, size_t cap);
void suisei_engine_settings_delete_theme(SuiseiEngine *ptr, const char *name);
uint32_t suisei_engine_theme_override_mask(const SuiseiEngine *ptr);
void suisei_engine_settings_set_theme_token(SuiseiEngine *ptr, uint32_t index,
                                            const char *value);
void suisei_engine_settings_reset_theme_tokens(SuiseiEngine *ptr);

/* ── GitHub account (Settings) ─────────────────────────────────────────── */
/* Independent of chrome. The face probes generation and copies this only
   when it moves, so a profile refresh cannot republish the editor. */

#define SUISEI_GH_STATE_MISSING 0
#define SUISEI_GH_STATE_OUT 1
#define SUISEI_GH_STATE_IN 2

#define SUISEI_GH_NAME_CAP 96
#define SUISEI_GH_URL_CAP 256
#define SUISEI_GH_HOST_CAP 64
#define SUISEI_GH_CODE_CAP 32
#define SUISEI_GH_CONTRIB_DAYS 371

typedef struct SuiseiGitHubAccount {
  uint64_t generation;
  uint8_t state;      /* SUISEI_GH_STATE_* */
  uint8_t loading;
  uint8_t signing_in;
  uint8_t _pad;
  uint32_t public_repos;
  uint32_t followers;
  uint32_t following;
  char user[SUISEI_GH_NAME_CAP];
  char name[SUISEI_GH_NAME_CAP];
  char email[SUISEI_GH_NAME_CAP];
  char avatar_url[SUISEI_GH_URL_CAP];
  char bio[SUISEI_GH_URL_CAP];
  char company[SUISEI_GH_NAME_CAP];
  char location[SUISEI_GH_NAME_CAP];
  char html_url[SUISEI_GH_URL_CAP];
  char host[SUISEI_GH_HOST_CAP];
  char protocol[24];
  char scopes[SUISEI_GH_URL_CAP];
  char token_source[SUISEI_GH_HOST_CAP];
  char device_code[SUISEI_GH_CODE_CAP];
  char message[SUISEI_MSG_CAP];
  uint32_t contrib_total;
  uint16_t contrib_days;
  uint16_t _contrib_pad;
  uint8_t contrib_levels[SUISEI_GH_CONTRIB_DAYS];
  char contrib_start[12];
  uint32_t contrib_year;
  uint32_t contrib_year_min;
} SuiseiGitHubAccount;

uint8_t suisei_engine_github_account(SuiseiEngine *ptr, SuiseiGitHubAccount *out);
uint64_t suisei_engine_github_account_generation(const SuiseiEngine *ptr);
void suisei_engine_github_account_refresh(SuiseiEngine *ptr);
void suisei_engine_github_sign_in(SuiseiEngine *ptr);
void suisei_engine_github_sign_out(SuiseiEngine *ptr);
void suisei_engine_github_cancel_sign_in(SuiseiEngine *ptr);
void suisei_engine_github_open_profile(SuiseiEngine *ptr);
void suisei_engine_github_setup_git(SuiseiEngine *ptr);
void suisei_engine_github_set_contrib_year(SuiseiEngine *ptr, uint32_t year);
void suisei_engine_github_install_docs(SuiseiEngine *ptr);

#define SUISEI_UPDATE_NOTES_CAP 512

typedef struct SuiseiUpdateSnapshot {
  uint64_t generation;
  uint8_t available;
  uint8_t installing;
  uint8_t installed;
  uint8_t checking;
  /* How many things this page is asking the user to look at — the red dot on
     the sidebar row, the way System Settings marks an available update. A
     count rather than a flag, because the sidebar sums several sources. */
  uint32_t badge;
  /* Bytes the update working directory is holding. A source update clones the
     repository and builds it, so this is gigabytes in ~/Library/Caches where
     nobody goes looking — and an editor that quietly keeps them for a job it
     finished last week is indistinguishable from a leak. */
  uint64_t cache_bytes;
  char current[64];
  char latest[64];
  char notes[SUISEI_UPDATE_NOTES_CAP];
} SuiseiUpdateSnapshot;

uint8_t suisei_engine_update(const SuiseiEngine *ptr, SuiseiUpdateSnapshot *out);
uint64_t suisei_engine_update_generation(const SuiseiEngine *ptr);
void suisei_engine_update_check(SuiseiEngine *ptr);
void suisei_engine_update_install(SuiseiEngine *ptr);

/* Delete the update working directory; bytes freed come back in `out_freed`.

   Result: 1 cleared · 2 refused, an update is staged · 3 refused, a build is
   running · 0 bad arguments.

   **Refusing is the point.** The staged bundle lives in this directory, and the
   safety argument for source updates is that there is a working Suisei on disk
   at every instant, with the atomic swap as the only destructive step. A cache
   button that threw away the update the user is waiting to restart into would
   be the one way to break that from inside the product. */
uint8_t suisei_engine_update_clear_cache(SuiseiEngine *ptr, uint64_t *out_freed);

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
  /* The commit in parts. graph_lines above is the same data joined; the face
     could only print that verbatim, which is why the history read as a dump. */
  char graph_short[SUISEI_MAX_SCM_GRAPH][16];
  char graph_subject[SUISEI_MAX_SCM_GRAPH][160];
  char graph_when[SUISEI_MAX_SCM_GRAPH][32];
  char graph_refs[SUISEI_MAX_SCM_GRAPH][96];
  uint8_t graph_color[SUISEI_MAX_SCM_GRAPH];
  /* 1 = on HEAD and not on its upstream (Xcode's `U`). Per row, not a count:
     the walk is `git log --all`, so the first N rows of a date-ordered
     all-branches list are not the N unpushed commits. */
  uint8_t graph_unpushed[SUISEI_MAX_SCM_GRAPH];
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
  uint8_t enabled[SUISEI_MAX_BREAKPOINTS];
  char logs[SUISEI_MAX_BREAKPOINTS][96];
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
  /* Structural role per row: 0 flow · 1 rule · 2 quote · 3 code · 4 table row.

     The core used to draw a table as `┌──┬──┐` and a quote as a `│` in column
     one, padding text to a width measured in MONOSPACE CELLS. The face does not
     draw in monospace cells, so the box was crooked and the rule ragged the
     moment the font or the size differed from what the padding assumed — and
     the face had started guessing the structure back out of the text
     (`isRule`, `isTableLike`) to compensate. Now it is told.

     A row's text never contains box-drawing. Inline formatting still rides in
     the text as U+E000+style markers, because bold and links really are
     properties of the text and survive a change of font. */
  uint8_t blocks[SUISEI_MAX_PREVIEW];
  /* Per-kind payload.
       quote: nesting depth (1 = one `>`)
       code:  bit0 first row of the run, bit1 last row
       table: bit0 header row, bit1 first row, bit2 last row */
  uint8_t block_args[SUISEI_MAX_PREVIEW];
  /* Table rows: 2 bits of column alignment each, low column first
     (0 left, 1 centre, 2 right). Eight columns fit; past that the face
     left-aligns, which is the default anyway. */
  uint16_t table_aligns[SUISEI_MAX_PREVIEW];
  char lines[SUISEI_MAX_PREVIEW][SUISEI_PREVIEW_LINE];
} SuiseiPreviewSnapshot;

/* A table row's cells are joined in `lines` by ASCII Unit Separator (0x1F).
   A cell boundary has to be a character that cannot occur INSIDE a cell, and a
   tab can — markdown splits rows on `|`, so a literal tab survives into the
   cell text and would have split it again. */
#define SUISEI_PREVIEW_CELL_SEP "\x1f"

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
/* Room for a keyword guide, not a tooltip. HOVER_CHARS caps at 4,000
   CHARACTERS and this is BYTES; 16 KiB covers 4,000 of up to four bytes each,
   so a Korean or Japanese doc comment arrives whole. */
#define SUISEI_HOVER_TEXT 16384

/* 1 when a language server is attached; writes its name into `out` when `out`
   and `cap` are given. Lets a surface tell "no server for this language" from
   "the server had nothing to say about this symbol". */
uint8_t suisei_engine_lsp_server(const SuiseiEngine *ptr, char *out, uint32_t cap);

void suisei_engine_request_hover(SuiseiEngine *ptr);
uint8_t suisei_engine_hover_text(const SuiseiEngine *ptr, char *out, uint32_t cap);

/* One row a live reload touched. */
#define SUISEI_LIVE_CHANGED 0
#define SUISEI_LIVE_ADDED 1
#define SUISEI_LIVE_REMOVED 2
#define SUISEI_MAX_LIVE_MARKS 4096

typedef struct SuiseiLiveMarkC {
  uint32_t row;
  uint8_t kind;
  uint8_t _pad;
  /* Rows a removal took away, on a REMOVED mark; 0 otherwise. */
  uint16_t removed;
} SuiseiLiveMarkC;

/* Bumped whenever the marks change, including expiry — poll this, pull only
   when it moves. The minimap needs marks for rows outside the visible band,
   which is why these are a list and not per-line bits. */
uint64_t suisei_engine_live_gen(const SuiseiEngine *ptr);
uint32_t suisei_engine_live_marks(const SuiseiEngine *ptr, SuiseiLiveMarkC *out,
                                  uint32_t cap);
/* Paths touched recently, as consecutive NUL-terminated strings. Includes
   background tabs — the tree is where a file nobody is looking at says it
   moved. */
#define SUISEI_LIVE_FILES_CAP 8192
uint32_t suisei_engine_live_files(const SuiseiEngine *ptr, char *out, uint32_t cap);

/* Absolute path of the document in a pane; 0 when it has none (untitled, or a
   shell). The non-text viewers draw from the file, not from the buffer, so
   this is how they find it. Pulled on demand — see the Rust doc comment. */
uint8_t suisei_engine_pane_path(const SuiseiEngine *ptr, uint32_t idx, char *out,
                                uint32_t cap);
/* Stable BufferTab::id shown by one pane; 0 when no document owns it. */
uint64_t suisei_engine_pane_tab_id(const SuiseiEngine *ptr, uint32_t idx);
/* True while this stable BufferTab::id still owns an open document. */
uint8_t suisei_engine_tab_id_is_open(const SuiseiEngine *ptr, uint64_t id);

/* Where a pane's shell should be working; 0 when that pane is not a terminal.
   The face forks the pane shells (SwiftTerm), so it needs this once per shell
   — including for every terminal tab a restored window brings back. */
uint8_t suisei_engine_pane_terminal_cwd(const SuiseiEngine *ptr, uint32_t idx,
                                        char *out, uint32_t cap);
/* The face reporting the OSC 0/2 title of a pane shell, keyed by BufferTab::id.
   NULL or empty clears it back to the generic "Terminal". Recomposes only when
   the string actually changed. */
void suisei_engine_set_terminal_title(SuiseiEngine *ptr, uint64_t tab_id,
                                      const char *title);

/* LSP face surfaces — same App methods the TUI dispatches (gd / format / rename / code actions). */
void suisei_engine_format_document(SuiseiEngine *ptr);
/* Comment or uncomment the lines the selection touches. */
void suisei_engine_toggle_comment(SuiseiEngine *ptr);
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

/* ── Session persistence ───────────────────────────────────────────────── */

/* Restore the previous session's files + cursors (call once at startup). */
void suisei_engine_restore_session(SuiseiEngine *ptr);
/* Persist open files + cursors for the next launch. */
void suisei_engine_save_session(const SuiseiEngine *ptr);

/* ── Shadow WAL recovery (D0) ──────────────────────────────────────────── */

/* Number of pending crash-recovery entries found on startup. */
uint32_t suisei_engine_completion_last_total_us(const SuiseiEngine *ptr);

uint32_t suisei_engine_completion_last_scope_us(const SuiseiEngine *ptr);

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

/* ── Debugger (DAP) ───────────────────────────────────────────────────────
   suisei-core's DAP client is a complete debugger and none of it crossed this
   boundary until now: the TUI drove it directly, and the Mac app could reach
   the breakpoint store and nothing else. One snapshot for what the panel
   draws, gated on a fingerprint — memsetting a hundred kilobytes to discover
   nothing changed is a cost with nothing on the other side of it. */

#define SUISEI_MAX_DAP_THREADS 16
#define SUISEI_MAX_DAP_FRAMES 48
#define SUISEI_MAX_DAP_VARS 160
#define SUISEI_MAX_DAP_CONSOLE 200
#define SUISEI_DAP_LINE 240

typedef struct SuiseiDapSnapshot {
  uint8_t state; /* 0 idle, 1 starting, 2 running, 3 stopped, 4 ending */
  uint8_t open;
  uint8_t session;
  uint8_t has_location;
  char adapter[64];
  char status[240];
  char stopped_reason[64];
  uint32_t thread_count;
  int64_t current_thread;
  int64_t thread_ids[SUISEI_MAX_DAP_THREADS];
  char thread_names[SUISEI_MAX_DAP_THREADS][64];
  uint32_t frame_count;
  uint32_t selected_frame;
  char frame_names[SUISEI_MAX_DAP_FRAMES][128];
  char frame_paths[SUISEI_MAX_DAP_FRAMES][SUISEI_PATH_CAP];
  uint32_t frame_lines[SUISEI_MAX_DAP_FRAMES]; /* 0-based */
  uint32_t var_count;
  char var_names[SUISEI_MAX_DAP_VARS][96];
  char var_values[SUISEI_MAX_DAP_VARS][160];
  char var_types[SUISEI_MAX_DAP_VARS][64];
  uint8_t var_depth[SUISEI_MAX_DAP_VARS];
  uint8_t var_expandable[SUISEI_MAX_DAP_VARS];
  uint8_t var_expanded[SUISEI_MAX_DAP_VARS];
  uint8_t var_is_scope[SUISEI_MAX_DAP_VARS];
  /* The newest SUISEI_MAX_DAP_CONSOLE lines; console_total is the true length,
     so a face can say what it is not showing. */
  uint32_t console_count;
  uint32_t console_total;
  char console[SUISEI_MAX_DAP_CONSOLE][SUISEI_DAP_LINE];
  char current_path[SUISEI_PATH_CAP];
  uint32_t current_line; /* 0-based */
  /* What each console line IS: 0 program output, 1 note, 2 adapter, 3 error,
     4 result. Appended, so every offset above is unmoved. The kind used to be
     a prefix inside the text (`[stdout] 16`), which made what the program
     printed look like the adapter talking about it. */
  uint8_t console_kinds[SUISEI_MAX_DAP_CONSOLE];
  /* How the last program ended; INT32_MIN when it did not say. */
  int32_t exit_code;
} SuiseiDapSnapshot;

uint64_t suisei_engine_dap_fingerprint(const SuiseiEngine *ptr);
uint8_t suisei_engine_dap(const SuiseiEngine *ptr, SuiseiDapSnapshot *out);
uint32_t suisei_engine_dap_configs(const SuiseiEngine *ptr, char *out_names,
                                   uint32_t name_cap, uint32_t max);
/* 0 start/continue, 1 pause, 2 step over, 3 step into, 4 step out, 5 stop,
   6 restart, 7 clear breakpoints. */
void suisei_engine_dap_command(SuiseiEngine *ptr, uint32_t verb);
void suisei_engine_dap_launch(SuiseiEngine *ptr, const char *name);
void suisei_engine_dap_attach(SuiseiEngine *ptr, const char *spec);
void suisei_engine_dap_evaluate(SuiseiEngine *ptr, const char *expr);

/* Hover datatip. `request` asks; `dap_datatip` returns 1 when filled, 2 while
   a request is in flight, 0 when there is nothing. */
#define SUISEI_MAX_INLINE_VALUES 64
#define SUISEI_INLINE_VALUE_CAP 128

/* Inline values for the visible rows — `x = 5` at the end of a line.
   Its own call, not a band field: empty except while stopped. */
typedef struct SuiseiInlineValueSnapshot {
  uint32_t count;
  uint32_t rows[SUISEI_MAX_INLINE_VALUES]; /* 0-based */
  char texts[SUISEI_MAX_INLINE_VALUES][SUISEI_INLINE_VALUE_CAP];
} SuiseiInlineValueSnapshot;

uint8_t suisei_engine_inline_values(const SuiseiEngine *e, uint32_t first_row,
                                    uint32_t row_count,
                                    SuiseiInlineValueSnapshot *out);

/* Breakpoint properties. Lines are 1-based; an empty string clears. */
void suisei_engine_dap_set_condition(SuiseiEngine *e, const char *path,
                                     uint32_t line_1based, const char *condition);
void suisei_engine_dap_set_log_message(SuiseiEngine *e, const char *path,
                                       uint32_t line_1based, const char *message);
void suisei_engine_dap_toggle_breakpoint_enabled(SuiseiEngine *e, const char *path,
                                                 uint32_t line_1based);

/* Change a value while stopped. `index` is a Variables-tree row. */
void suisei_engine_dap_set_variable(SuiseiEngine *e, uint32_t index, const char *value);
uint8_t suisei_engine_dap_can_set_variable(const SuiseiEngine *e);

/* Watchpoints — stop when a value changes. `watch` toggles. */
void suisei_engine_dap_watch(SuiseiEngine *e, const char *name);
uint8_t suisei_engine_dap_can_watch(const SuiseiEngine *e);
uint8_t suisei_engine_dap_is_watched(const SuiseiEngine *e, const char *name);

void suisei_engine_dap_request_datatip(SuiseiEngine *e, const char *expr);
uint8_t suisei_engine_dap_datatip(const SuiseiEngine *e, char *out_expr,
                                  char *out_value, char *out_type,
                                  uint32_t cap);
void suisei_engine_dap_select_frame(SuiseiEngine *ptr, uint32_t index);
void suisei_engine_dap_toggle_var(SuiseiEngine *ptr, uint32_t index);
void suisei_engine_dap_set_panel(SuiseiEngine *ptr, uint8_t open);

/* ── Logic View ────────────────────────────────────────────────────────────
   The control flow of one file, as far as it has been opened. Every call
   names its path: a Logic pane is usually NOT the pane the keyboard is in,
   which is the whole point of it. */

#define SUISEI_MAX_LOGIC_ROWS 320
#define SUISEI_LOGIC_LABEL 192
#define SUISEI_LOGIC_VALUE 96

/* Per-row flags. */
#define SUISEI_LOGIC_EXPANDABLE 1
#define SUISEI_LOGIC_EXPANDED 2
#define SUISEI_LOGIC_ENCLOSING 4
#define SUISEI_LOGIC_CALLER 8
#define SUISEI_LOGIC_STOPPED 16
#define SUISEI_LOGIC_BREAKPOINT 32

/* Editor marks. */
#define SUISEI_MAX_LOGIC_RUNS 16
#define SUISEI_LOGIC_RUN_SELECTED 1
#define SUISEI_LOGIC_RUN_RUNTIME 2
#define SUISEI_LOGIC_RUN_ARM_YES 4
#define SUISEI_LOGIC_RUN_ARM_NO 8

typedef struct SuiseiLogicSnapshot {
  uint8_t ok;
  /* Stopped in THIS file: the runtime flags mean something. */
  uint8_t live;
  uint8_t _pad[2];
  char path[SUISEI_PATH_CAP];
  /* Why the list is empty, when it is. */
  char note[160];
  char lang[32];
  uint32_t row_count;
  uint32_t selected;
  char labels[SUISEI_MAX_LOGIC_ROWS][SUISEI_LOGIC_LABEL];
  char values[SUISEI_MAX_LOGIC_ROWS][SUISEI_LOGIC_VALUE];
  /* 0 entry, 1 process, 2 decision, 3 loop, 4 exit, 5 opaque */
  uint8_t kinds[SUISEI_MAX_LOGIC_ROWS];
  uint8_t depths[SUISEI_MAX_LOGIC_ROWS];
  /* 0 next, 1 yes, 2 no, 3 back */
  uint8_t edges[SUISEI_MAX_LOGIC_ROWS];
  uint8_t flags[SUISEI_MAX_LOGIC_ROWS];
  uint32_t start_rows[SUISEI_MAX_LOGIC_ROWS];
  uint32_t end_rows[SUISEI_MAX_LOGIC_ROWS];

  /* What the EDITOR draws: runs, not rows — a guide down a block is a run and
     the face clips it to the band it is painting. */
  uint32_t run_count;
  uint32_t run_start[SUISEI_MAX_LOGIC_RUNS];
  uint32_t run_end[SUISEI_MAX_LOGIC_RUNS];
  /* Visual column for the guide: the node's own indentation. */
  uint16_t run_col[SUISEI_MAX_LOGIC_RUNS];
  uint8_t run_flags[SUISEI_MAX_LOGIC_RUNS];
} SuiseiLogicSnapshot;

uint64_t suisei_engine_logic_fingerprint(const SuiseiEngine *ptr, const char *path);
uint8_t suisei_engine_logic(SuiseiEngine *ptr, const char *path,
                            SuiseiLogicSnapshot *out);
void suisei_engine_logic_toggle(SuiseiEngine *ptr, const char *path, uint32_t index);
void suisei_engine_logic_select(SuiseiEngine *ptr, const char *path, uint32_t index);
void suisei_engine_logic_reveal(SuiseiEngine *ptr, const char *path, uint32_t index);
/* The rail catching up with the caret. Returns 1 when something moved. */
uint8_t suisei_engine_logic_follow(SuiseiEngine *ptr, const char *path, uint32_t line);
/* Ask what a branch's two arms are. UINT32_MAX clears it. */
void suisei_engine_logic_peek(SuiseiEngine *ptr, const char *path, uint32_t index);
void suisei_engine_logic_open(SuiseiEngine *ptr);
uint8_t suisei_engine_logic_available(const SuiseiEngine *ptr);

/* ── The project marker ────────────────────────────────────────────────────
   `project.suiseiprj` opens as a screen rather than as raw JSON. A VIEWER
   pane, so ⌘S cannot write an empty buffer over a file the team shares: the
   pane asks core to write the project, and core owns the format. */

#define SUISEI_MAX_PROJECT_LSP 24

typedef struct SuiseiProjectSnapshot {
  uint8_t ok;
  /* Zero is a legal indent width and a wrong answer for "not set". */
  uint8_t has_tab_width;
  uint8_t _pad[2];
  uint32_t schema;
  uint32_t tab_width;
  char root[SUISEI_PATH_CAP];
  char name[128];
  char project_id[96];
  uint32_t lsp_count;
  char lsp_langs[SUISEI_MAX_PROJECT_LSP][32];
  char lsp_cmds[SUISEI_MAX_PROJECT_LSP][192];
  /* What Build / Run / Test mean here, in that order; "" = the manifest on
     disk decides. Appended, so every offset above is where it was. */
  char commands[3][192];
} SuiseiProjectSnapshot;

/* Open a file as TEXT whatever kind it is — the escape hatch under a viewer. */
void suisei_engine_open_as_text(SuiseiEngine *ptr, const char *path);

uint8_t suisei_engine_project(const SuiseiEngine *ptr, SuiseiProjectSnapshot *out);
void suisei_engine_project_set_name(SuiseiEngine *ptr, const char *name);
/* 0 clears it back to "inherit the global setting". */
void suisei_engine_project_set_tab_width(SuiseiEngine *ptr, uint32_t width);
/* An empty command removes the entry. */
void suisei_engine_project_set_lsp(SuiseiEngine *ptr, const char *lang, const char *cmd);
/* 0 build, 1 run, 2 test; an empty command hands it back to the manifest. */
void suisei_engine_project_set_command(SuiseiEngine *ptr, uint32_t which, const char *cmd);

/* What kind of file a NAME is, and which language — no disk, no engine.
   `out_lang` gets the language's canonical extension (`js` for `jsx`/`mjs`),
   or "". Returns the FileKind discriminant. */
uint8_t suisei_engine_classify_name(const char *name, char *out_lang, uint32_t cap);

/* ── Accessibility ─────────────────────────────────────────────────────────
   The canvas draws its text itself, so AppKit cannot describe it. A text area
   answers in CHARACTER OFFSETS over the whole document, and the document is
   here — a mirror in the face would be one more thing to hold in step with
   every keystroke, and a screen reader reading stale lines is worse than one
   reading nothing. Asked at human pace, so an O(lines) walk is the right
   trade against a cache that has to be invalidated correctly. */

uint32_t suisei_engine_ax_line_count(const SuiseiEngine *ptr);
uint64_t suisei_engine_ax_char_count(const SuiseiEngine *ptr);
/* Returns the line's length in CHARACTERS (not bytes, not what fit in cap). */
uint32_t suisei_engine_ax_line(const SuiseiEngine *ptr, uint32_t row, char *out, uint32_t cap);
uint64_t suisei_engine_ax_offset_of_row(const SuiseiEngine *ptr, uint32_t row);
uint32_t suisei_engine_ax_row_of_offset(const SuiseiEngine *ptr, uint64_t offset);
void suisei_engine_ax_selection(const SuiseiEngine *ptr, uint64_t *out_start, uint64_t *out_len);
void suisei_engine_ax_set_selection(SuiseiEngine *ptr, uint64_t start, uint64_t len);

/* ── Build & Run ───────────────────────────────────────────────────────────
   feature.txt #9's other half. The debugger builds in order to LAUNCH; this
   runs a command because the output is the point — and turns the lines that
   name a place into diagnostics, which is why a compile error can now be
   jumped to instead of read.

   One pull gated on a fingerprint, like the debugger's snapshot: the newest
   console lines with the true total beside them, so the face can say what it
   is not showing. A problem carries no path — a row is clicked by INDEX and
   core opens the file, because only core knows whether it is already in a
   tab. */

#define SUISEI_MAX_BUILD_CONSOLE 300
#define SUISEI_BUILD_LINE 200
#define SUISEI_MAX_BUILD_PROBLEMS 64

typedef struct SuiseiBuildSnapshot {
  uint8_t state; /* 0 idle, 1 running, 2 ok, 3 failed */
  uint8_t kind;  /* 0 build, 1 run, 2 test; 255 = nothing has run */
  uint8_t open;
  uint8_t _pad;
  int32_t exit; /* INT32_MIN while it is still running */
  uint32_t took_ms;
  uint32_t errors;
  uint32_t warnings;
  uint32_t dropped; /* found past the cap, and therefore not kept */
  char label[96];
  char summary[240];
  uint32_t console_count;
  uint32_t console_total;
  char console[SUISEI_MAX_BUILD_CONSOLE][SUISEI_BUILD_LINE];
  /* What each line IS — the debugger's vocabulary, because it is one console
     to the reader: 0 program, 1 note, 2 adapter, 3 error, 4 result. */
  uint8_t console_kinds[SUISEI_MAX_BUILD_CONSOLE];
  uint32_t problem_count;
  uint32_t problem_total;
  uint32_t problem_rows[SUISEI_MAX_BUILD_PROBLEMS]; /* 0-based */
  uint32_t problem_cols[SUISEI_MAX_BUILD_PROBLEMS]; /* 0-based, chars */
  uint8_t problem_severities[SUISEI_MAX_BUILD_PROBLEMS]; /* 0 error 1 warn 2 info */
  uint8_t problem_locatable[SUISEI_MAX_BUILD_PROBLEMS]; /* 0 = names no place */
  char problem_files[SUISEI_MAX_BUILD_PROBLEMS][64];
  char problem_messages[SUISEI_MAX_BUILD_PROBLEMS][SUISEI_BUILD_LINE];
} SuiseiBuildSnapshot;

uint64_t suisei_engine_build_fingerprint(const SuiseiEngine *ptr);
uint8_t suisei_engine_build(const SuiseiEngine *ptr, SuiseiBuildSnapshot *out);
/* 0 build, 1 run, 2 test. */
void suisei_engine_build_run(SuiseiEngine *ptr, uint32_t kind);
void suisei_engine_build_stop(SuiseiEngine *ptr);
void suisei_engine_build_goto(SuiseiEngine *ptr, uint32_t index);
void suisei_engine_build_set_open(SuiseiEngine *ptr, uint8_t open);

/* ── Shortcuts ─────────────────────────────────────────────────────────────
 * Core owns the chord notation on both sides, so the face never parses "⇧⌘P"
 * and the two cannot drift over what it means. */
#define SUISEI_KEY_CAP 32

typedef struct SuiseiKeyBindingC {
  char id[64];                        /* stable, what the config file stores */
  char title[SUISEI_TITLE_CAP];
  char group[32];
  char chord[SUISEI_KEY_CAP];         /* in force now */
  char default_chord[SUISEI_KEY_CAP]; /* what it ships with */
  uint8_t customised;                 /* 1 = chord != default_chord */
  uint8_t _pad[7];
} SuiseiKeyBindingC;

/* Settings → Components. Detection only: what the machine has, and the line
   that installs what it does not. Downloading needs a signed release. */
typedef struct {
  char id[64];
  char title[96];
  char group[32];
  char detail[320];
  char install[192];
  char path[320];
  uint8_t state; /* 0 missing · 1 present · 2 bundled */
  uint8_t _pad[7];
} SuiseiComponentC;

/* No engine pointer: probing touches no App state, which is what makes it safe
   to call from a background thread. */
uint32_t suisei_engine_components_refresh(void);
uint8_t suisei_engine_components_row(uint32_t index, SuiseiComponentC *out);
uint32_t suisei_engine_components_blocked_reason(char *out, uint32_t cap);

uint32_t suisei_engine_keymap_count(void);
uint8_t suisei_engine_keymap_row(const SuiseiEngine *ptr, uint32_t index,
                                 SuiseiKeyBindingC *out);
/* Title of the OTHER command already on `chord`, or "" — asked before setting. */
uint8_t suisei_engine_keymap_conflict(const SuiseiEngine *ptr, const char *id,
                                      const char *chord, char *out_title,
                                      uint32_t cap);
/* NULL or empty chord = back to the shipped one. 0 = not a usable shortcut. */
uint8_t suisei_engine_keymap_set(SuiseiEngine *ptr, const char *id,
                                 const char *chord);
void suisei_engine_keymap_reset_all(SuiseiEngine *ptr);
