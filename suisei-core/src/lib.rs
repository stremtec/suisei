pub mod bench;
pub mod buffer;
pub mod call_hierarchy;
pub mod clipboard;
pub mod completion;
pub mod config;
pub mod dap;
pub mod dap_cmds;
pub mod edit;
pub mod exec;
pub mod explorer;
pub mod fold;
pub mod fs_atomic;
pub mod gh;
pub mod git;
pub mod git_graph;
pub mod git_ops;
pub mod git_workbench;
pub mod gui_edit;
pub mod highlight;
pub mod hooks;
pub mod lang;
pub mod layout_tab;
pub mod layouts;
pub mod lsp;
pub mod media;
pub mod multi_cursor;
pub mod nav;
pub mod palette;
pub mod panes;
pub mod peek;
pub mod preview;
pub mod project;
pub mod pump;
pub mod registers;
pub mod scm;
pub mod scope;
pub mod search;
pub mod selection;
pub mod session;
pub mod settings;
pub mod split;
pub mod syntax;
pub mod syntax_worker;
pub mod tabs;
pub mod theme;
pub mod undo;
pub mod update;
pub mod workspace_search;
pub mod wrap;

pub mod app;
pub mod dispatch;
pub mod key;
pub use app::{
    App, BufferId, BufferTab, EditorContextMenu, EditorCtxItem, Mode, MouseState, ProcMetrics,
    ResizeTarget, SplitSepHit, Stage,
};
pub use call_hierarchy::{CallDirection, CallHierarchyState, CallItem};
pub use dap::{
    Breakpoint, DapClient, DapState, DebugPane, LaunchConfig, StackFrameInfo, VarNode,
    load_launch_configs,
};
pub use edit::{Change, Delta, Edit};
pub use fold::FoldState;
pub use fs_atomic::atomic_write_file;
pub use gh::{AuthLoginSession, GhAuthInfo, GhAuthState, GhContributions, GhProfile};
pub use git::{GitBlame, GitGutter, GitSign};
pub use git_graph::{GraphGlyph, GraphRow};
pub use git_workbench::{
    GitCtxItem, GitFocus, GitLoadTarget, GitPane, GitTab, GitWorkbench, HistoryView,
};
pub use hooks::{HookEvent, HooksConfig};
pub use key::{KeyCode, KeyEvent, KeyModifiers};
pub use layout_tab::{LayoutStyle, LayoutTab};
pub use lsp::CodeLens;
pub use media::{AudioPlayer, ImageAsset, is_media_path};
pub use nav::{Jump, JumpList};
pub use palette::{Palette, PaletteAction, PaletteKind};
pub use peek::PeekState;
pub use preview::{PreviewKind, PreviewState};
pub use registers::Registers;
pub use scm::{ScmFocus, ScmPanel, ScmStatus};
pub use settings::{
    HelpEntry, SettingRow, SettingsAction, SettingsPage, SettingsPanel, help_entries,
};
pub use split::{Axis, Layout, Pane, PaneId, SplitState, TerminalId};
pub use update::UpdateState;
pub use workspace_search::{SearchHit, WorkspaceSearch};

/// What a live reload did to a band of rows.
///
/// Blue arrived, red left, and "changed" is the same lines saying something
/// else — three states because that is what a reader asks of a change they did
/// not make: did it grow, shrink, or move?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LiveKind {
    Changed = 0,
    Added = 1,
    Removed = 2,
}
