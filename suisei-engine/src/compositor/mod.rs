//! Compositor: App + ShellState → Scene / FrameDiff.
//! Layout engines (tabs / canvas) grow here; no App mutation.

mod scene;

pub use scene::{
    build_editor_band, build_outline_public, compose, patch_chrome_editor_scroll, ChromeScene,
    EditorLineScene, FrameDiff, OutlineItemScene, PaneScene, ShellState, Viewport,
};
