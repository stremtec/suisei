//! Compositor: App + ShellState → Scene / FrameDiff.
//! Layout engines (tabs / canvas) grow here; no App mutation.

mod scene;

pub use scene::{
    BREAKPOINT_DECORATED, BREAKPOINT_DISABLED, ChromeScene, DEBUG_FRAME, DEBUG_STOPPED,
    EditorLineScene, VALUE_EXTENT, VALUE_FIRST,
    VALUE_LAST, VALUE_WRITE, FrameDiff, OutlineItemScene, PaneScene,
    ShellState, Viewport, build_editor_band, build_outline_public, build_sticky_band, compose,
    patch_chrome_editor_scroll,
};
// Caret placement, for the FFI's cheap caret pull. The same two functions the
// chrome snapshot uses, so the face cannot be told two different caret columns
// depending on which path it asked through.
pub(crate) use scene::{buffer_for_tab, drawn_caret_col, visual_col};
