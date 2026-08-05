//! Compositor: App + ShellState → Scene / FrameDiff.
//! Layout engines (tabs / canvas) grow here; no App mutation.

mod scene;

pub use scene::{
    ChromeScene, EditorLineScene, FrameDiff, OutlineItemScene, PaneScene, ShellState, Viewport,
    build_editor_band, build_outline_public, compose, patch_chrome_editor_scroll,
};
