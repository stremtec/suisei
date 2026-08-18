//! Unified Settings panel (Ctrl+,) — About · Setting · Extensions · Help.

use crate::config::{self, Config};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    /// Welcome-like About card.
    About,
    /// Appearance + editor config (theme, tab width, …).
    Setting,
    /// VSCode extensions — status, installed list, store entry point.
    Extensions,
    /// Keyboard shortcut reference.
    Help,
}

impl SettingsPage {
    pub fn label(self) -> &'static str {
        match self {
            SettingsPage::About => "About",
            SettingsPage::Setting => "Setting",
            SettingsPage::Extensions => "Extensions",
            SettingsPage::Help => "Help",
        }
    }

    pub fn all() -> &'static [SettingsPage] {
        &[
            SettingsPage::About,
            SettingsPage::Setting,
            SettingsPage::Extensions,
            SettingsPage::Help,
        ]
    }

    pub fn next(self) -> Self {
        match self {
            SettingsPage::About => SettingsPage::Setting,
            SettingsPage::Setting => SettingsPage::Extensions,
            SettingsPage::Extensions => SettingsPage::Help,
            SettingsPage::Help => SettingsPage::About,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            SettingsPage::About => SettingsPage::Help,
            SettingsPage::Setting => SettingsPage::About,
            SettingsPage::Extensions => SettingsPage::Setting,
            SettingsPage::Help => SettingsPage::Extensions,
        }
    }
}

/// One row on the Help page: key chord + description.
#[derive(Debug, Clone, Copy)]
pub struct HelpEntry {
    pub keys: &'static str,
    pub desc: &'static str,
    /// Section header when `keys` is empty.
    pub is_header: bool,
}

/// Full shortcut list shown on the Help tab.
pub fn help_entries() -> &'static [HelpEntry] {
    &[
        HelpEntry {
            keys: "",
            desc: "General",
            is_header: true,
        },
        HelpEntry {
            keys: "Ctrl+,",
            desc: "Settings (About / Setting / Extensions / Help)",
            is_header: false,
        },
        HelpEntry {
            keys: ":screensaver / :ss",
            desc: "xeifetch splash (clock · weather · Esc exit)",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+S",
            desc: "Save file",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+P / Cmd+P",
            desc: "Quick open files",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Shift+P",
            desc: "Command palette",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+F",
            desc: "Toggle file explorer",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+E",
            desc: "Toggle XLC command panel",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Shift+V",
            desc: "Pretty preview (Markdown / JSON / CSV / image / audio)",
            is_header: false,
        },
        HelpEntry {
            keys: "za / zc / zo / zM / zR",
            desc: "Toggle / close / open fold · close all / open all",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+B / gb",
            desc: "Git blame panel (slides in · flame colors)",
            is_header: false,
        },
        HelpEntry {
            keys: "Space (leader)",
            desc: "Which-key: f files · g git · l lsp · d debug · w window · …",
            is_header: false,
        },
        HelpEntry {
            keys: "F5 / Shift+F5",
            desc: "DAP start/continue · stop session",
            is_header: false,
        },
        HelpEntry {
            keys: "F9 / F10 / F11 / S-F11",
            desc: "Toggle breakpoint · step over · into · out",
            is_header: false,
        },
        HelpEntry {
            keys: "F6",
            desc: "DAP pause a running program",
            is_header: false,
        },
        HelpEntry {
            keys: ":bp if expr / :bp log msg",
            desc: "Conditional breakpoint · logpoint",
            is_header: false,
        },
        HelpEntry {
            keys: ":pr N",
            desc: "PR review surface (files · diff · comments)",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+W v/s ×N · h j k l",
            desc: "Split again for up to 4 panes · directional focus",
            is_header: false,
        },
        HelpEntry {
            keys: "zh zl zH zL",
            desc: "Pan horizontally (wrap_lines = false)",
            is_header: false,
        },
        HelpEntry {
            keys: "Wheel / PageUp·Down",
            desc: "Terminal scrollback (badge ↑N · typing snaps live)",
            is_header: false,
        },
        HelpEntry {
            keys: ":settings / SPC l a",
            desc: "Settings · code actions (legacy terminals: Ctrl+,/Ctrl+. don't exist)",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Shift+D / SPC d",
            desc: "Debug panel (stack · vars · BPs · console)",
            is_header: false,
        },
        HelpEntry {
            keys: ":dap / :bp / :launch",
            desc: "Debug panel · breakpoint · launch program",
            is_header: false,
        },
        HelpEntry {
            keys: "gC / gI / gH / :calls",
            desc: "Call hierarchy (incoming / outgoing)",
            is_header: false,
        },
        HelpEntry {
            keys: ":rebase [N] / SPC g r",
            desc: "Interactive rebase last N commits",
            is_header: false,
        },
        HelpEntry {
            keys: ":rebase-abort / :rebase-continue",
            desc: "Abort or continue in-progress rebase",
            is_header: false,
        },
        HelpEntry {
            keys: ":codelens / SPC t l",
            desc: "Toggle LSP code lens (EOL virtual text)",
            is_header: false,
        },
        HelpEntry {
            keys: "PR Enter / :pr N",
            desc: "PR review · files + diff + comments",
            is_header: false,
        },
        HelpEntry {
            keys: "~/.suisei/hooks.toml",
            desc: "Plugin hooks: on_save / on_open / on_quit",
            is_header: false,
        },
        HelpEntry {
            keys: "]c / [c",
            desc: "Next / previous git change (gutter hunk)",
            is_header: false,
        },
        HelpEntry {
            keys: "g / z / d c y / Ctrl+W",
            desc: "Prefix chords open delayed which-key popup",
            is_header: false,
        },
        HelpEntry {
            keys: "Tab (Insert)",
            desc: "Expand snippet (fn, for, if, …) or indent",
            is_header: false,
        },
        HelpEntry {
            keys: "Live reload",
            desc: "Auto-reload when file changes on disk",
            is_header: false,
        },
        HelpEntry {
            keys: "Git · 9 Stash",
            desc: "Stash list · Enter apply · d drop · p preview",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+D",
            desc: "Multi-cursor: add next word match",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Alt+j/k",
            desc: "Multi-cursor: add caret below / above",
            is_header: false,
        },
        HelpEntry {
            keys: "Esc (multi)",
            desc: "Clear extra carets (Insert: first Esc)",
            is_header: false,
        },
        HelpEntry {
            keys: "Explorer · Enter",
            desc: "Open file · images/csv/npy/audio → preview",
            is_header: false,
        },
        HelpEntry {
            keys: "Preview ←/→",
            desc: "Resize image preview",
            is_header: false,
        },
        HelpEntry {
            keys: "Preview Space",
            desc: "Play / stop audio",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Shift+F",
            desc: "Find in files (workspace search)",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Shift+O / gO",
            desc: "Document symbols (outline)",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+.",
            desc: "Code actions / quick fix",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Shift+I",
            desc: "Format document (LSP)",
            is_header: false,
        },
        HelpEntry {
            keys: "Cmd+C / V / X",
            desc: "Copy / paste / cut (system clipboard)",
            is_header: false,
        },
        HelpEntry {
            keys: "Right-click",
            desc: "Editor context menu (Insert / Normal / Visual)",
            is_header: false,
        },
        HelpEntry {
            keys: "",
            desc: "Terminal",
            is_header: true,
        },
        HelpEntry {
            keys: "Ctrl+T",
            desc: "Side terminal panel (Esc closes side term)",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Shift+T",
            desc: "Terminal window in a split pane",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Shift+W",
            desc: "Close terminal window (then y / n)",
            is_header: false,
        },
        HelpEntry {
            keys: "Esc  (term focused)",
            desc: "Sent to shell — not editor (vim/opencode exit)",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+C / D / Z …",
            desc: "When term focused: real signals to the program",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+W w",
            desc: "From terminal pane: focus the other split",
            is_header: false,
        },
        HelpEntry {
            keys: "F12",
            desc: "Quick toggle side terminal",
            is_header: false,
        },
        HelpEntry {
            keys: "",
            desc: "Splits",
            is_header: true,
        },
        HelpEntry {
            keys: "Ctrl+W v / s",
            desc: "Vertical / horizontal split",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+W w",
            desc: "Cycle focused split pane",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+W q",
            desc: "Close current split",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+W =",
            desc: "Equalize split sizes",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+W h/j/k/l",
            desc: "Focus left / down / up / right pane",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+W < / >",
            desc: "Resize split",
            is_header: false,
        },
        HelpEntry {
            keys: "Drag split edge",
            desc: "Mouse-resize panes",
            is_header: false,
        },
        HelpEntry {
            keys: "",
            desc: "Git",
            is_header: true,
        },
        HelpEntry {
            keys: "Ctrl+G",
            desc: "Light Source Control (stage / commit)",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+Shift+G",
            desc: "Full Git workbench (mini GitHub)",
            is_header: false,
        },
        HelpEntry {
            keys: "1–8  (in Git)",
            desc: "Status / Log / Branches / Files / Diff / PRs / Issues / Auth",
            is_header: false,
        },
        HelpEntry {
            keys: "Tab  (in Git)",
            desc: "Cycle Changes · Log · Files columns",
            is_header: false,
        },
        HelpEntry {
            keys: "Space / s  (Git Status)",
            desc: "Stage / unstage selected file",
            is_header: false,
        },
        HelpEntry {
            keys: "c  (Git Status)",
            desc: "Edit commit message · Enter commits",
            is_header: false,
        },
        HelpEntry {
            keys: "Enter  (Git)",
            desc: "Open diff / commit detail / PR checkout",
            is_header: false,
        },
        HelpEntry {
            keys: "Right-click commit",
            desc: "Cherry-pick / revert / copy hash / browse",
            is_header: false,
        },
        HelpEntry {
            keys: "v  (Git Log)",
            desc: "Toggle list / graph view",
            is_header: false,
        },
        HelpEntry {
            keys: "f p u  (Git)",
            desc: "Fetch / pull / push",
            is_header: false,
        },
        HelpEntry {
            keys: "r  (Git)",
            desc: "Refresh current tab",
            is_header: false,
        },
        HelpEntry {
            keys: ":gh-login",
            desc: "GitHub CLI auth (web)",
            is_header: false,
        },
        HelpEntry {
            keys: "",
            desc: "Modes & editing",
            is_header: true,
        },
        HelpEntry {
            keys: "i a A o O",
            desc: "Enter Insert mode",
            is_header: false,
        },
        HelpEntry {
            keys: "Esc",
            desc: "Back to Normal mode",
            is_header: false,
        },
        HelpEntry {
            keys: "v / V / Ctrl+V",
            desc: "Visual / Visual Line / Visual Block",
            is_header: false,
        },
        HelpEntry {
            keys: "h j k l · ←↓↑→",
            desc: "Move cursor",
            is_header: false,
        },
        HelpEntry {
            keys: "w b e · 0 $ · gg G",
            desc: "Word / line / file motions",
            is_header: false,
        },
        HelpEntry {
            keys: "d / c / y + motion",
            desc: "Delete / change / yank",
            is_header: false,
        },
        HelpEntry {
            keys: "diw ci\" dib …",
            desc: "Text objects (inner / around)",
            is_header: false,
        },
        HelpEntry {
            keys: "u / Ctrl+R",
            desc: "Undo / Redo",
            is_header: false,
        },
        HelpEntry {
            keys: ".",
            desc: "Repeat last change",
            is_header: false,
        },
        HelpEntry {
            keys: "p P",
            desc: "Paste after / before",
            is_header: false,
        },
        HelpEntry {
            keys: "x",
            desc: "Delete character",
            is_header: false,
        },
        HelpEntry {
            keys: "\"a  \"+ ",
            desc: "Registers (named / clipboard)",
            is_header: false,
        },
        HelpEntry {
            keys: "ma  'a  `a",
            desc: "Set mark / jump",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+O / Ctrl+I",
            desc: "Jumplist back / forward",
            is_header: false,
        },
        HelpEntry {
            keys: "f t ; ,",
            desc: "Find char; repeat / reverse",
            is_header: false,
        },
        HelpEntry {
            keys: "qa … q · @a · @@",
            desc: "Record / play / replay macro",
            is_header: false,
        },
        HelpEntry {
            keys: "",
            desc: "Search",
            is_header: true,
        },
        HelpEntry {
            keys: "/  ?",
            desc: "Search forward / reverse",
            is_header: false,
        },
        HelpEntry {
            keys: "n  N",
            desc: "Next / previous match",
            is_header: false,
        },
        HelpEntry {
            keys: "*  #",
            desc: "Word under cursor (fwd / back)",
            is_header: false,
        },
        HelpEntry {
            keys: "",
            desc: "LSP & diagnostics",
            is_header: true,
        },
        HelpEntry {
            keys: "gd",
            desc: "Go to definition",
            is_header: false,
        },
        HelpEntry {
            keys: "gp",
            desc: "Peek definition",
            is_header: false,
        },
        HelpEntry {
            keys: "gr",
            desc: "Find references",
            is_header: false,
        },
        HelpEntry {
            keys: "K",
            desc: "Hover documentation",
            is_header: false,
        },
        HelpEntry {
            keys: "Ctrl+A  (Insert)",
            desc: "Completions (LSP + keywords)",
            is_header: false,
        },
        HelpEntry {
            keys: "]d  [d",
            desc: "Next / prev diagnostic",
            is_header: false,
        },
        HelpEntry {
            keys: ":Rename name",
            desc: "LSP rename",
            is_header: false,
        },
        HelpEntry {
            keys: "",
            desc: "Tabs & buffers",
            is_header: true,
        },
        HelpEntry {
            keys: "gt  gT",
            desc: "Next / previous tab",
            is_header: false,
        },
        HelpEntry {
            keys: ":e <file>",
            desc: "Open file (new tab)",
            is_header: false,
        },
        HelpEntry {
            keys: ":bd",
            desc: "Close current tab",
            is_header: false,
        },
        HelpEntry {
            keys: ":w  :q  :wq",
            desc: "Save / quit / save+quit",
            is_header: false,
        },
        HelpEntry {
            keys: ":s/pat/repl/g",
            desc: "Substitute on line / range",
            is_header: false,
        },
        HelpEntry {
            keys: ":theme <name>",
            desc: "Switch theme (persists)",
            is_header: false,
        },
        HelpEntry {
            keys: ":help",
            desc: "List XLC commands",
            is_header: false,
        },
        HelpEntry {
            keys: "",
            desc: "Settings panel",
            is_header: true,
        },
        HelpEntry {
            keys: "Tab / Shift+Tab",
            desc: "Next / previous page",
            is_header: false,
        },
        HelpEntry {
            keys: "1 2 3 4",
            desc: "Jump About / Setting / Extensions / Help",
            is_header: false,
        },
        HelpEntry {
            keys: "j k · Enter",
            desc: "Move selection · activate",
            is_header: false,
        },
        HelpEntry {
            keys: "s",
            desc: "Save ~/.suisei.toml",
            is_header: false,
        },
        HelpEntry {
            keys: "Esc / q",
            desc: "Close settings",
            is_header: false,
        },
    ]
}

/// Setting-page row kinds (theme pickers + editor toggles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingRow {
    ThemeHeader,
    /// System-following, explicitly light, or explicitly dark.
    AppearanceMode,
    /// Native floating-chrome density: clear or tinted Liquid Glass.
    GlassStyle,
    Theme(usize),
    EditorHeader,
    TabWidth,
    RelativeNumber,
    WrapLines,
    UndoCaching,
    ClipboardSync,
    GpuAcc,
    GpuGraphics,
    GpuHyperlinks,
    KeyHints,
    LspHeader,
    LspEnabled,
    FormatOnSave,
    /// Index into `config::lsp_lang_catalog()`
    LspLang(usize),
    GitHeader,
    OpenWorkbench,
    OpenScm,
    /// Semantic accent/selection hue. The value itself is a hex string.
    HighlightColor,
    UpdateCheck,
}

/// Native Settings destinations. These values cross the engine/Swift ABI;
/// append only. They are deliberately independent from the four legacy TUI
/// tabs in [`SettingsPage`].
///
/// Exactly one page owns each question. A setting that appears on two pages is
/// two answers to one question, and the user has to guess which one the app
/// believes — so a row names its page here and nowhere else decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingSurfacePage {
    /// Not presented on any native page.
    ///
    /// The row still exists, is still parsed from and written to the config
    /// file, and can still be driven by a keybinding — it simply has no place
    /// in Settings. That is the state of a switch whose feature is gone: the
    /// stored value must survive so an existing config still loads, and the
    /// switch must not, because operating it does nothing.
    None,
    General,
    /// Retired destination. Its two halves went where Xcode keeps them: the
    /// colour scheme is app appearance and lives on General, the palette is a
    /// theme and has its own page. Kept so the ABI number is never reused.
    Appearance,
    Editor,
    LanguageServers,
    SourceControl,
    SoftwareUpdate,
    Themes,
}

impl SettingSurfacePage {
    pub const fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::General => 1,
            Self::Appearance => 2,
            Self::Editor => 3,
            Self::LanguageServers => 4,
            Self::SourceControl => 5,
            Self::SoftwareUpdate => 6,
            Self::Themes => 7,
        }
    }
}

/// How a native face should edit a setting. This is presentation semantics,
/// not merely the storage type: RelativeNumber is stored as bool, but a
/// Line-number-style menu communicates the choice better than another switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingControl {
    None,
    Toggle,
    Menu,
    Segmented,
    Action,
    Color,
}

impl SettingControl {
    pub const fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Toggle => 1,
            Self::Menu => 2,
            Self::Segmented => 3,
            Self::Action => 4,
            Self::Color => 5,
        }
    }
}

/// Layout and copy for one native Settings row. Keeping this beside
/// [`SettingRow`] makes Core the source of truth for both behavior and the
/// shape needed to present it; the Swift face no longer reconstructs a form
/// from labels and a pile of booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingPresentation {
    pub page: SettingSurfacePage,
    pub group: &'static str,
    pub control: SettingControl,
    pub label: &'static str,
    pub detail: &'static str,
    /// Pipe-delimited choices for menu/segmented controls.
    pub options: &'static str,
    pub advanced: bool,
}

impl SettingRow {
    /// Stable numeric tag for the face, and the row's payload index.
    ///
    /// The GUI used to recover a row's meaning by matching its DISPLAY LABEL —
    /// `label == "LSP enabled"`, `label.hasPrefix("LSP ·")`,
    /// `label.contains("●")`, a hard-coded list of five toggle names. The type
    /// was right here and was thrown away at the FFI boundary, so every control
    /// in Settings was a string guess: renaming a label silently broke a
    /// control, and three rows (the GPU ones) were simply never listed and so
    /// could not be reached at all.
    ///
    /// These numbers are ABI. Append only; never renumber.
    pub fn kind(self) -> u32 {
        match self {
            SettingRow::ThemeHeader => 1,
            SettingRow::Theme(_) => 2,
            SettingRow::EditorHeader => 3,
            SettingRow::TabWidth => 4,
            SettingRow::RelativeNumber => 5,
            SettingRow::WrapLines => 6,
            SettingRow::UndoCaching => 7,
            SettingRow::ClipboardSync => 8,
            SettingRow::GpuAcc => 9,
            SettingRow::GpuGraphics => 10,
            SettingRow::GpuHyperlinks => 11,
            SettingRow::KeyHints => 12,
            SettingRow::LspHeader => 13,
            SettingRow::LspEnabled => 14,
            SettingRow::FormatOnSave => 14,
            SettingRow::LspLang(_) => 15,
            SettingRow::GitHeader => 16,
            SettingRow::OpenWorkbench => 17,
            SettingRow::OpenScm => 18,
            SettingRow::HighlightColor => 19,
            SettingRow::UpdateCheck => 20,
            SettingRow::AppearanceMode => 21,
            SettingRow::GlassStyle => 22,
        }
    }

    /// Which theme / which language — 0 for rows that carry no payload.
    pub fn payload(self) -> u32 {
        match self {
            SettingRow::Theme(i) | SettingRow::LspLang(i) => i as u32,
            _ => 0,
        }
    }

    pub fn presentation(self) -> SettingPresentation {
        use SettingControl::{Action, Color, Menu, None, Segmented, Toggle};
        use SettingSurfacePage::{
            Editor, General, LanguageServers, SoftwareUpdate, SourceControl, Themes,
        };

        match self {
            Self::ThemeHeader => SettingPresentation {
                page: Themes,
                group: "Theme",
                control: None,
                label: "Theme",
                detail: "",
                options: "",
                advanced: false,
            },
            // Appearance is how the APP looks, and Xcode keeps it on General
            // next to the rest of the app's behaviour. The palette is a
            // different question with its own page.
            Self::AppearanceMode => SettingPresentation {
                page: General,
                group: "Appearance",
                control: Segmented,
                label: "Color Scheme",
                detail: "Follow macOS automatically or keep Suisei light or dark.",
                options: "Automatic|Light|Dark",
                advanced: false,
            },
            Self::GlassStyle => SettingPresentation {
                page: General,
                group: "Appearance",
                control: Segmented,
                label: "Liquid Glass",
                detail: "Choose how strongly floating controls separate from editor content.",
                options: "Clear|Tinted",
                advanced: false,
            },
            Self::Theme(_) => SettingPresentation {
                page: Themes,
                group: "Theme",
                control: Menu,
                label: "Theme",
                detail: "Controls editor colors and syntax highlighting.",
                options: "",
                advanced: false,
            },
            Self::HighlightColor => SettingPresentation {
                page: Themes,
                group: "Accent",
                control: Color,
                label: "Highlight Color",
                detail: "Used for selections, focus, links, and active controls.",
                options: "",
                advanced: false,
            },
            // Display, Tab Width and the wrap/number menus were on General
            // while the Editor page held Keep Undo History and Use System
            // Clipboard — so "how the editor shows text" was answered on two
            // pages, and the one named Editor was the page that did not have
            // Line Wrapping on it.
            Self::EditorHeader => SettingPresentation {
                page: Editor,
                group: "Display",
                control: None,
                label: "Display",
                detail: "",
                options: "",
                advanced: false,
            },
            Self::TabWidth => SettingPresentation {
                page: Editor,
                group: "Display",
                control: Menu,
                label: "Tab Width",
                detail: "Number of spaces used when inserting a tab.",
                options: "2 Spaces|4 Spaces|8 Spaces",
                advanced: false,
            },
            // Owned by Software Update, which is where a person goes to ask
            // this. It was on General as well, and the two rows wrote the same
            // config key from two pages.
            Self::UpdateCheck => SettingPresentation {
                page: SoftwareUpdate,
                group: "Automatic Updates",
                control: Menu,
                label: "Check for Updates",
                detail: "Choose whether Suisei checks for a newer release at launch.",
                options: "Manually|Automatically",
                advanced: false,
            },
            Self::RelativeNumber => SettingPresentation {
                page: Editor,
                group: "Display",
                control: Menu,
                label: "Line Numbers",
                detail: "Relative numbers show the distance from the current line.",
                options: "Absolute|Relative",
                advanced: false,
            },
            Self::WrapLines => SettingPresentation {
                page: Editor,
                group: "Display",
                control: Menu,
                label: "Line Wrapping",
                detail: "Wrap long lines at the window edge or scroll horizontally.",
                options: "No Wrapping|Wrap to Window",
                advanced: false,
            },
            // App behaviour, not text display: one is about what survives
            // closing a file, the other about sharing with other Mac apps.
            // Editor is for how the editor draws text.
            Self::UndoCaching => SettingPresentation {
                page: General,
                group: "Behavior",
                control: Toggle,
                label: "Keep Undo History",
                detail: "Preserve undo history after a file is closed.",
                options: "",
                advanced: false,
            },
            Self::ClipboardSync => SettingPresentation {
                page: General,
                group: "Behavior",
                control: Toggle,
                label: "Use System Clipboard",
                detail: "Share copy and paste operations with other Mac apps.",
                options: "",
                advanced: false,
            },
            // Not presented. The which-key overlay this switched was removed
            // when Suisei stopped having leader/prefix chords, so the switch
            // reads `key_hints` out of the config, writes it back, and nothing
            // between those two points consumes it. The config key stays so an
            // existing file still parses.
            Self::KeyHints => SettingPresentation {
                page: SettingSurfacePage::None,
                group: "",
                control: None,
                label: "Show Key Hints",
                detail: "",
                options: "",
                advanced: false,
            },
            // Not presented, for the same reason and a blunter one: these three
            // describe Suisei drawing ITSELF into Ghostty or Kitty. There is no
            // terminal frontend — the workspace builds core, engine and the
            // daemon, and the Mac app is the only face. The page even said so
            // ("The native Mac editor does not require them") while listing
            // three switches that do nothing in the only editor there is.
            Self::GpuAcc => SettingPresentation {
                page: SettingSurfacePage::None,
                group: "",
                control: None,
                label: "Terminal Rendering",
                detail: "",
                options: "Compatibility|Enhanced",
                advanced: true,
            },
            Self::GpuGraphics => SettingPresentation {
                page: SettingSurfacePage::None,
                group: "",
                control: None,
                label: "Inline Terminal Graphics",
                detail: "",
                options: "",
                advanced: true,
            },
            Self::GpuHyperlinks => SettingPresentation {
                page: SettingSurfacePage::None,
                group: "",
                control: None,
                label: "Terminal Hyperlinks",
                detail: "",
                options: "",
                advanced: true,
            },
            Self::LspHeader => SettingPresentation {
                page: LanguageServers,
                group: "Language Servers",
                control: None,
                label: "Language Servers",
                detail: "",
                options: "",
                advanced: false,
            },
            Self::LspEnabled => SettingPresentation {
                page: LanguageServers,
                group: "Language Servers",
                control: Toggle,
                label: "Enable Language Servers",
                detail: "Provide completion, navigation, diagnostics, and refactoring.",
                options: "",
                advanced: false,
            },
            Self::FormatOnSave => SettingPresentation {
                page: LanguageServers,
                group: "Language Servers",
                control: Toggle,
                label: "Format on Save",
                detail: "Run the language server's formatter on ⌘S. A save is never held longer than a moment — if the server does not answer, the file is written unformatted.",
                options: "",
                advanced: false,
            },
            Self::LspLang(_) => SettingPresentation {
                page: LanguageServers,
                group: "Configured Servers",
                control: Menu,
                label: "Language Server",
                detail: "Use the built-in command, disable it, or preserve a custom command.",
                options: "Default|Off|Custom",
                advanced: false,
            },
            Self::GitHeader => SettingPresentation {
                page: SourceControl,
                group: "Source Control",
                control: None,
                label: "Source Control",
                detail: "",
                options: "",
                advanced: false,
            },
            Self::OpenWorkbench => SettingPresentation {
                page: SourceControl,
                group: "Source Control",
                control: Action,
                label: "Open Git Workbench",
                detail: "Review changes, history, branches, pull requests, and issues.",
                options: "",
                advanced: false,
            },
            Self::OpenScm => SettingPresentation {
                page: SourceControl,
                group: "Source Control",
                control: Action,
                label: "Open Changes Navigator",
                detail: "Stage files and create a commit without leaving the editor.",
                options: "",
                advanced: false,
            },
        }
    }

    /// Selected option for the native control described by `presentation()`.
    pub fn value_index(self, draft: &Config) -> u32 {
        match self {
            Self::AppearanceMode => match draft.theme.as_str() {
                "light" => 1,
                "dark" => 2,
                _ => 0,
            },
            Self::GlassStyle => u32::from(draft.glass_style == "tinted"),
            Self::TabWidth => match draft.tab_width {
                2 => 0,
                8 => 2,
                _ => 1,
            },
            Self::RelativeNumber => u32::from(draft.relative_number),
            Self::WrapLines => u32::from(draft.wrap_lines),
            Self::UndoCaching => u32::from(draft.undo_caching),
            Self::ClipboardSync => u32::from(draft.clipboard_sync),
            Self::GpuAcc => u32::from(draft.gpu_acc),
            Self::GpuGraphics => u32::from(draft.gpu_graphics),
            Self::GpuHyperlinks => u32::from(draft.gpu_hyperlinks),
            Self::KeyHints => u32::from(draft.key_hints),
            Self::LspEnabled => u32::from(draft.lsp_enabled),
            Self::FormatOnSave => u32::from(draft.format_on_save),
            Self::HighlightColor => u32::from(draft.highlight_color != "default"),
            Self::UpdateCheck => u32::from(draft.update_check),
            Self::LspLang(i) => config::lsp_lang_catalog()
                .get(i)
                .map(
                    |(key, _, _)| match draft.lsp_servers.get(*key).map(String::as_str) {
                        None => 0,
                        Some("") => 1,
                        Some(_) => 2,
                    },
                )
                .unwrap_or(0),
            _ => 0,
        }
    }
}

fn setting_rows() -> Vec<SettingRow> {
    let mut rows = vec![
        SettingRow::ThemeHeader,
        SettingRow::AppearanceMode,
        SettingRow::GlassStyle,
    ];
    for i in 0..theme::all_themes().len() {
        rows.push(SettingRow::Theme(i));
    }
    rows.push(SettingRow::HighlightColor);
    rows.push(SettingRow::EditorHeader);
    rows.push(SettingRow::RelativeNumber);
    rows.push(SettingRow::WrapLines);
    rows.push(SettingRow::TabWidth);
    rows.push(SettingRow::UpdateCheck);
    rows.push(SettingRow::UndoCaching);
    rows.push(SettingRow::ClipboardSync);
    rows.push(SettingRow::GpuAcc);
    rows.push(SettingRow::GpuGraphics);
    rows.push(SettingRow::GpuHyperlinks);
    rows.push(SettingRow::KeyHints);
    rows.push(SettingRow::LspHeader);
    rows.push(SettingRow::LspEnabled);
    rows.push(SettingRow::FormatOnSave);
    for i in 0..config::lsp_lang_catalog().len() {
        rows.push(SettingRow::LspLang(i));
    }
    rows.push(SettingRow::GitHeader);
    rows.push(SettingRow::OpenWorkbench);
    rows.push(SettingRow::OpenScm);
    rows
}

#[derive(Debug, Clone)]
pub struct SettingsPanel {
    pub open: bool,
    pub page: SettingsPage,
    /// Row selection within the current page (for toggles/lists).
    pub selected: usize,
    /// Working copy of config while the panel is open.
    pub draft: Config,
    pub dirty: bool,
    pub status: Option<String>,
}

impl Default for SettingsPanel {
    fn default() -> Self {
        Self {
            open: false,
            page: SettingsPage::About,
            selected: 0,
            draft: Config::default(),
            dirty: false,
            status: None,
        }
    }
}

impl SettingsPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_panel(&mut self) {
        self.open = true;
        self.page = SettingsPage::About;
        self.selected = 0;
        self.draft = config::load();
        self.dirty = false;
        self.status = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.status = None;
    }

    pub fn visible(&self) -> bool {
        self.open
    }

    pub fn setting_rows(&self) -> Vec<SettingRow> {
        setting_rows()
    }

    pub fn page_item_count(&self) -> usize {
        match self.page {
            SettingsPage::About => 0,
            SettingsPage::Setting => setting_rows().len(),
            SettingsPage::Extensions => 0,
            SettingsPage::Help => help_entries().len(),
        }
    }

    /// Skip non-selectable header rows when moving selection on Setting page.
    pub fn move_sel(&mut self, delta: isize) {
        let n = self.page_item_count();
        if n == 0 {
            self.selected = 0;
            return;
        }
        if self.page == SettingsPage::Setting {
            let rows = setting_rows();
            let is_header = |r: &SettingRow| {
                matches!(
                    r,
                    SettingRow::ThemeHeader
                        | SettingRow::EditorHeader
                        | SettingRow::LspHeader
                        | SettingRow::GitHeader
                )
            };
            let mut cur = self.selected as isize;
            let step = if delta >= 0 { 1 } else { -1 };
            let mut left = delta.abs().max(1);
            // If already on a header (shouldn't), step off first.
            if cur >= 0 && (cur as usize) < rows.len() && is_header(&rows[cur as usize]) {
                left = left.max(1);
            }
            while left > 0 {
                let next = cur + step;
                if next < 0 || next >= rows.len() as isize {
                    // Stay on last valid non-header
                    break;
                }
                cur = next;
                if !is_header(&rows[cur as usize]) {
                    left -= 1;
                }
            }
            // Ensure we never rest on a header
            if (cur as usize) < rows.len() && is_header(&rows[cur as usize]) {
                if let Some((i, _)) = rows.iter().enumerate().find(|(_, r)| !is_header(r)) {
                    cur = i as isize;
                }
            }
            self.selected = cur as usize;
            return;
        }
        // Help: skip section headers so selection lands on real shortcuts
        if self.page == SettingsPage::Help {
            let entries = help_entries();
            let mut cur = self.selected as isize;
            let step = if delta >= 0 { 1 } else { -1 };
            let mut left = delta.abs().max(1);
            while left > 0 {
                let next = cur + step;
                if next < 0 || next >= entries.len() as isize {
                    break;
                }
                cur = next;
                if !entries[cur as usize].is_header {
                    left -= 1;
                }
            }
            if (cur as usize) < entries.len() && entries[cur as usize].is_header {
                if let Some((i, _)) = entries.iter().enumerate().find(|e| !e.1.is_header) {
                    cur = i as isize;
                }
            }
            self.selected = cur as usize;
            return;
        }
        let cur = self.selected as isize + delta;
        self.selected = cur.clamp(0, (n - 1) as isize) as usize;
    }

    pub fn next_page(&mut self) {
        self.page = self.page.next();
        self.selected = self.default_selected_for_page();
        self.status = None;
    }

    pub fn prev_page(&mut self) {
        self.page = self.page.prev();
        self.selected = self.default_selected_for_page();
        self.status = None;
    }

    fn default_selected_for_page(&self) -> usize {
        match self.page {
            SettingsPage::Setting => 1, // first theme row
            SettingsPage::Help => {
                // First real shortcut (skip "General" header)
                help_entries()
                    .iter()
                    .position(|e| !e.is_header)
                    .unwrap_or(0)
            }
            SettingsPage::About | SettingsPage::Extensions => 0,
        }
    }

    /// Activate / toggle the selected row. Returns optional UI action.
    pub fn activate(&mut self) -> SettingsAction {
        if self.page != SettingsPage::Setting {
            return SettingsAction::None;
        }
        let Some(row) = setting_rows().get(self.selected).copied() else {
            return SettingsAction::None;
        };

        let current = row.value_index(&self.draft);
        let next = match row.presentation().control {
            SettingControl::Toggle => u32::from(current == 0),
            SettingControl::Segmented => {
                let count = row
                    .presentation()
                    .options
                    .split('|')
                    .filter(|option| !option.is_empty())
                    .count()
                    .max(1) as u32;
                (current + 1) % count
            }
            SettingControl::Menu => match row {
                SettingRow::Theme(_) => current,
                SettingRow::LspLang(_) => {
                    if current == 0 {
                        1
                    } else {
                        0
                    }
                }
                _ => u32::from(current == 0),
            },
            SettingControl::Action => 0,
            SettingControl::Color => return SettingsAction::None,
            SettingControl::None => return SettingsAction::None,
        };
        self.apply_row_value(row, next)
    }

    /// Set the selected row to an explicit native-control option. Unlike the
    /// legacy `activate` cycle, this lets a face choose Tab Width 8 directly
    /// from 2 without landing on 4 first.
    pub fn set_value(&mut self, option: u32) -> SettingsAction {
        if self.page != SettingsPage::Setting {
            return SettingsAction::None;
        }
        let Some(row) = setting_rows().get(self.selected).copied() else {
            return SettingsAction::None;
        };
        self.apply_row_value(row, option)
    }

    /// Set — or clear — one colour of the palette currently being shown.
    ///
    /// `value` empty or `"default"` removes the override, which is how a token
    /// goes back to what the theme author chose. `palette` is the RESOLVED
    /// theme name, so the caller has already decided whether `system` means
    /// light or dark; this layer must not guess that.
    ///
    /// Removing the last override for a palette drops the whole entry, so the
    /// config file does not accumulate empty tables for themes you tried once.
    pub fn set_theme_token(
        &mut self,
        palette: &str,
        token: crate::theme::ThemeToken,
        value: &str,
    ) -> SettingsAction {
        // Taken verbatim: this IS the key `theme::override_target` returned.
        // It used to be lowercased here, which was right for built-ins (whose
        // names are lowercase) and silently wrong for a user-made theme called
        // "Midnight" — the edit went into a second table called "midnight"
        // that nothing ever read.
        let palette = palette.trim().to_string();
        if palette.is_empty() {
            return SettingsAction::None;
        }
        let value = value.trim();
        let clearing = value.is_empty() || value.eq_ignore_ascii_case("default");

        let normalized = if clearing {
            None
        } else {
            let hex = value.strip_prefix('#').unwrap_or(value);
            if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return SettingsAction::None;
            }
            Some(format!("#{}", hex.to_ascii_uppercase()))
        };

        let current = self
            .draft
            .theme_overrides
            .get(&palette)
            .and_then(|t| t.get(token.key()))
            .cloned();
        if current == normalized {
            return SettingsAction::None;
        }

        match normalized {
            Some(hex) => {
                self.draft
                    .theme_overrides
                    .entry(palette)
                    .or_default()
                    .insert(token.key().to_string(), hex.clone());
                self.status = Some(format!("{} = {hex}", token.label()));
            }
            None => {
                if let Some(tokens) = self.draft.theme_overrides.get_mut(&palette) {
                    tokens.remove(token.key());
                    if tokens.is_empty() {
                        self.draft.theme_overrides.remove(&palette);
                    }
                }
                self.status = Some(format!("{} → theme default", token.label()));
            }
        }
        self.dirty = true;
        SettingsAction::ApplyTheme
    }

    /// Keep the current palette's edits as a theme of its own.
    ///
    /// Whether the edits move or are copied depends on what you saved FROM, and
    /// the rule is the one that leaves you with what you meant:
    ///
    /// * from a **built-in** the edits MOVE, so the shipped palette goes back
    ///   to how its author made it. Leaving them behind would hand you two
    ///   identical themes and no way to see the original again.
    /// * from a **user-made** theme they are COPIED, because that theme is
    ///   yours and saving a second version must not empty the first.
    ///
    /// `from_palette` is the override key currently in use — what
    /// [`crate::theme::override_target`] returned — not a display name.
    ///
    /// Returns the stored name, or `None` if the name is blank, would shadow a
    /// built-in, or is already taken.
    pub fn save_theme_as(&mut self, name: &str, from_palette: &str) -> Option<String> {
        let name = name.trim();
        let from = from_palette.trim();
        if name.is_empty() || crate::theme::find(name).is_some() {
            return None;
        }
        // Case-insensitive, because two themes differing only in capitalisation
        // are two themes nobody can tell apart in a menu.
        if self
            .draft
            .custom_themes
            .keys()
            .any(|k| k.eq_ignore_ascii_case(name))
        {
            return None;
        }

        // A base is always a built-in. Saving one custom theme from another
        // would build a chain, and deleting a link in the middle would orphan
        // everything after it — so the chain is flattened to its root here.
        let derived = self.draft.custom_themes.get(from).cloned();
        let base = derived.clone().unwrap_or_else(|| from.to_string());
        if crate::theme::find(&base).is_none() {
            return None;
        }

        let edits = if derived.is_some() {
            self.draft.theme_overrides.get(from).cloned().unwrap_or_default()
        } else {
            self.draft.theme_overrides.remove(from).unwrap_or_default()
        };
        if !edits.is_empty() {
            self.draft.theme_overrides.insert(name.to_string(), edits);
        }
        self.draft.custom_themes.insert(name.to_string(), base);
        self.draft.theme = name.to_string();
        self.status = Some(format!("Saved theme “{name}”"));
        self.dirty = true;
        Some(name.to_string())
    }

    /// Remove a user-made theme and its colours.
    ///
    /// If it is the theme in use, fall back to the palette it was built on —
    /// leaving `theme` pointing at a name nothing resolves would silently drop
    /// the editor to light/dark on the next launch.
    pub fn delete_custom_theme(&mut self, name: &str) -> SettingsAction {
        let name = name.trim();
        let Some(base) = self.draft.custom_themes.remove(name) else {
            return SettingsAction::None;
        };
        self.draft.theme_overrides.remove(name);
        if self.draft.theme == name {
            self.draft.theme = base;
        }
        self.status = Some(format!("Deleted theme “{name}”"));
        self.dirty = true;
        SettingsAction::ApplyTheme
    }

    /// Choose any theme by name — built-in or user-made.
    pub fn select_theme(&mut self, name: &str) -> SettingsAction {
        let name = name.trim();
        let known = crate::theme::find(name).is_some()
            || self.draft.custom_themes.contains_key(name)
            || matches!(name, "system");
        if !known || self.draft.theme == name {
            return SettingsAction::None;
        }
        self.draft.theme = name.to_string();
        self.status = Some(format!("Theme → {name}"));
        self.dirty = true;
        SettingsAction::ApplyTheme
    }

    /// Drop every edit made to one palette.
    pub fn reset_theme_tokens(&mut self, palette: &str) -> SettingsAction {
        let palette = palette.trim().to_string();
        if self.draft.theme_overrides.remove(&palette).is_none() {
            return SettingsAction::None;
        }
        self.status = Some(format!("{palette} → theme defaults"));
        self.dirty = true;
        SettingsAction::ApplyTheme
    }

    /// How many colours the user has changed on this palette.
    pub fn theme_override_count(&self, palette: &str) -> usize {
        self.draft
            .theme_overrides
            .get(palette.trim())
            .map_or(0, std::collections::BTreeMap::len)
    }

    /// Set the arbitrary sRGB value carried by the native color well.
    pub fn set_highlight_color(&mut self, value: &str) -> SettingsAction {
        let value = value.trim();
        let hex = value.strip_prefix('#').unwrap_or(value);
        let normalized = if value.eq_ignore_ascii_case("default") {
            "default".to_string()
        } else if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            format!("#{}", hex.to_ascii_uppercase())
        } else {
            return SettingsAction::None;
        };
        if self.draft.highlight_color == normalized {
            return SettingsAction::None;
        }
        self.draft.highlight_color = normalized.clone();
        self.status = Some(format!("highlight_color = {normalized}"));
        self.dirty = true;
        SettingsAction::ApplyTheme
    }

    fn apply_row_value(&mut self, row: SettingRow, option: u32) -> SettingsAction {
        let action = match row {
            SettingRow::AppearanceMode => {
                let Some(mode) = ["system", "light", "dark"].get(option as usize) else {
                    return SettingsAction::None;
                };
                self.draft.theme = (*mode).to_string();
                self.status = Some(format!("appearance = {mode}"));
                SettingsAction::ApplyTheme
            }
            SettingRow::GlassStyle => {
                let Some(style) = ["clear", "tinted"].get(option as usize) else {
                    return SettingsAction::None;
                };
                self.draft.glass_style = (*style).to_string();
                self.status = Some(format!("glass_style = {style}"));
                SettingsAction::None
            }
            SettingRow::Theme(i) => {
                let Some(t) = theme::all_themes().get(i) else {
                    return SettingsAction::None;
                };
                self.draft.theme = t.name.to_string();
                self.status = Some(format!("Theme → {}", t.name));
                SettingsAction::ApplyTheme
            }
            SettingRow::TabWidth => {
                let Some(width) = [2, 4, 8].get(option as usize).copied() else {
                    return SettingsAction::None;
                };
                self.draft.tab_width = width;
                self.status = Some(format!("tab_width = {width}"));
                SettingsAction::None
            }
            SettingRow::UpdateCheck => {
                self.draft.update_check = option != 0;
                self.status = Some(if self.draft.update_check {
                    "update_check = true  (check at launch)".into()
                } else {
                    "update_check = false  (manual checks only)".into()
                });
                SettingsAction::None
            }
            SettingRow::RelativeNumber => {
                self.draft.relative_number = option != 0;
                self.status = Some(format!("relative_number = {}", self.draft.relative_number));
                SettingsAction::None
            }
            SettingRow::WrapLines => {
                self.draft.wrap_lines = option != 0;
                self.status = Some(if self.draft.wrap_lines {
                    "wrap_lines = true  (soft-wrap long lines)".into()
                } else {
                    "wrap_lines = false  (horizontal scroll · zh/zl pan)".into()
                });
                SettingsAction::None
            }
            SettingRow::UndoCaching => {
                self.draft.undo_caching = option != 0;
                self.status = Some(if self.draft.undo_caching {
                    "undo_caching = true  (history survives close · ~/.suisei/undo)".into()
                } else {
                    "undo_caching = false  (history discarded on close)".into()
                });
                SettingsAction::None
            }
            SettingRow::ClipboardSync => {
                self.draft.clipboard_sync = option != 0;
                self.status = Some(format!("clipboard_sync = {}", self.draft.clipboard_sync));
                SettingsAction::None
            }
            SettingRow::GpuAcc => {
                self.draft.gpu_acc = option != 0;
                self.status = Some(if self.draft.gpu_acc {
                    "gpu_acc = true  (Ghostty/Kitty enhancements on)".into()
                } else {
                    "gpu_acc = false  (plain cell TUI)".into()
                });
                SettingsAction::ApplyGpuAcc
            }
            SettingRow::GpuGraphics => {
                self.draft.gpu_graphics = option != 0;
                self.status = Some(format!("gpu_graphics = {}", self.draft.gpu_graphics));
                SettingsAction::ApplyGpuAcc
            }
            SettingRow::GpuHyperlinks => {
                self.draft.gpu_hyperlinks = option != 0;
                self.status = Some(format!("gpu_hyperlinks = {}", self.draft.gpu_hyperlinks));
                SettingsAction::ApplyGpuAcc
            }
            SettingRow::KeyHints => {
                self.draft.key_hints = option != 0;
                self.status = Some(format!("key_hints = {}", self.draft.key_hints));
                SettingsAction::None
            }
            SettingRow::LspEnabled => {
                self.draft.lsp_enabled = option != 0;
                self.status = Some(format!("lsp_enabled = {}", self.draft.lsp_enabled));
                SettingsAction::ApplyLsp
            }
            SettingRow::FormatOnSave => {
                self.draft.format_on_save = option != 0;
                self.status = Some(format!("format_on_save = {}", self.draft.format_on_save));
                SettingsAction::None
            }
            SettingRow::LspLang(i) => {
                let Some((key, _label, default_cmd)) = config::lsp_lang_catalog().get(i) else {
                    return SettingsAction::None;
                };
                match option {
                    0 => {
                        self.draft.lsp_servers.remove(*key);
                        self.status = Some(format!("lsp.{key} = default ({default_cmd})"));
                    }
                    1 => {
                        self.draft
                            .lsp_servers
                            .insert((*key).to_string(), String::new());
                        self.status = Some(format!("lsp.{key} = off"));
                    }
                    // "Custom" reports an existing override but cannot invent
                    // a command. Editing custom commands gets a dedicated text
                    // field in the future.
                    2 if self
                        .draft
                        .lsp_servers
                        .get(*key)
                        .is_some_and(|v| !v.is_empty()) =>
                    {
                        return SettingsAction::None;
                    }
                    _ => return SettingsAction::None,
                }
                SettingsAction::ApplyLsp
            }
            SettingRow::OpenWorkbench => return SettingsAction::OpenWorkbench,
            SettingRow::OpenScm => return SettingsAction::OpenScm,
            SettingRow::HighlightColor => return SettingsAction::None,
            SettingRow::ThemeHeader
            | SettingRow::EditorHeader
            | SettingRow::LspHeader
            | SettingRow::GitHeader => return SettingsAction::None,
        };

        self.dirty = true;
        action
    }

    pub fn save(&mut self) {
        config::save(&self.draft);
        self.dirty = false;
        self.status = Some("Saved ~/.suisei.toml".into());
    }

    pub fn version_string() -> String {
        format!("xei {}", env!("CARGO_PKG_VERSION"))
    }
}

/// Side-effect requested by settings activation (handled in app/event layer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    ApplyTheme,
    ApplyGpuAcc,
    ApplyLsp,
    OpenWorkbench,
    OpenScm,
}
