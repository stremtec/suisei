// Re-export the extension host so the TUI frontend can reach its off-thread
// helpers (e.g. webview headless render) without a direct dependency.
#[cfg(feature = "extensions")]
pub use xei_ext_host;

pub mod bench;
pub mod buffer;
pub mod clipboard;
pub mod call_hierarchy;
pub mod completion;
pub mod config;
pub mod dap;
pub mod hooks;
pub mod explorer;
pub mod fold;
pub mod gh;
pub mod git;
pub mod snippets;
pub mod git_graph;
pub mod git_ops;
pub mod git_workbench;
pub mod highlight;
pub mod lsp;
pub mod macros;
pub mod multi_cursor;
pub mod nav;
pub mod ops;
pub mod selection;
pub mod palette;
pub mod media;
pub mod peek;
pub mod plugin_store;
pub mod preview;
pub mod pr_review;
pub mod rebase;
pub mod registers;
pub mod scm;
pub mod session;
pub mod settings;
pub mod screensaver;
pub mod pet;
pub mod split;
pub mod substitute;
pub mod syntax;
pub mod term;
pub mod undo;
pub mod update;
pub mod theme;
pub mod which_key;
pub mod workspace_search;
pub mod xlc;
pub mod fs_atomic;

pub mod app;
pub mod dispatch;
pub mod key;
pub use fs_atomic::atomic_write_file;
pub use app::{
    App, BufferTab, EditorContextMenu, EditorCtxItem, EditorViewport, Mode, MouseState,
    ProcMetrics, ResizeTarget, SplitSepHit, set_cursor_esc,
};
pub use key::{KeyCode, KeyEvent, KeyModifiers};
pub use macros::{MacroBank, MacroKey};
pub use multi_cursor::MultiCursor;
pub use nav::{FindKind, Jump, JumpList, LastFind, Marks};
pub use ops::{LastChange, Motion, Operator, TextObject};
pub use palette::{Palette, PaletteAction, PaletteKind};
pub use peek::PeekState;
pub use plugin_store::{ExtRow, PluginStore, StoreItem, StoreMsg, StoreTab};
pub use registers::Registers;
pub use fold::FoldState;
pub use git::{GitBlame, GitGutter, GitSign};
pub use git_graph::{GraphGlyph, GraphRow};
pub use gh::{AuthLoginSession, GhAuthInfo, GhAuthState};
pub use git_workbench::{
    GitCtxItem, GitFocus, GitLoadTarget, GitPane, GitTab, GitWorkbench, HistoryView,
};
pub use media::{is_media_path, AudioPlayer, ImageAsset};
pub use preview::{PreviewKind, PreviewState};
pub use scm::{ScmFocus, ScmPanel, ScmStatus};
pub use settings::{
    help_entries, HelpEntry, SettingRow, SettingsAction, SettingsPage, SettingsPanel,
};
pub use screensaver::{Screensaver, WeatherInfo};
pub use pet::PetState;
pub use split::{Pane, SplitKind, SplitState};
pub use substitute::SubstituteCmd;
pub use which_key::{ChordHint, WhichKeyState};
pub use workspace_search::{SearchHit, WorkspaceSearch};
pub use call_hierarchy::{CallDirection, CallHierarchyState, CallItem};
pub use dap::{
    load_launch_configs, Breakpoint, DapClient, DapState, DebugPane, LaunchConfig, StackFrameInfo,
    VarNode,
};
pub use hooks::{HookEvent, HooksConfig};
pub use update::UpdateState;
pub use pr_review::{PrReviewFocus, PrReviewState};
pub use rebase::{RebaseAction, RebaseState};
pub use lsp::CodeLens;
