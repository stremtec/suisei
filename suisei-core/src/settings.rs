//! Unified Settings panel (Ctrl+,) — About · Setting · Pet · Help.

use crate::config::{self, Config};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    /// Welcome-like About card.
    About,
    /// Appearance + editor config (theme, tab width, …).
    Setting,
    /// Desktop pet GIF overlay.
    Pet,
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
            SettingsPage::Pet => "Pet",
            SettingsPage::Extensions => "Extensions",
            SettingsPage::Help => "Help",
        }
    }

    pub fn all() -> &'static [SettingsPage] {
        &[
            SettingsPage::About,
            SettingsPage::Setting,
            SettingsPage::Pet,
            SettingsPage::Extensions,
            SettingsPage::Help,
        ]
    }

    pub fn next(self) -> Self {
        match self {
            SettingsPage::About => SettingsPage::Setting,
            SettingsPage::Setting => SettingsPage::Pet,
            SettingsPage::Pet => SettingsPage::Extensions,
            SettingsPage::Extensions => SettingsPage::Help,
            SettingsPage::Help => SettingsPage::About,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            SettingsPage::About => SettingsPage::Help,
            SettingsPage::Setting => SettingsPage::About,
            SettingsPage::Pet => SettingsPage::Setting,
            SettingsPage::Extensions => SettingsPage::Pet,
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
            desc: "Settings (About / Setting / Pet / Help)",
            is_header: false,
        },
        HelpEntry {
            keys: ":screensaver / :ss",
            desc: "xeifetch splash (clock · weather · Esc exit)",
            is_header: false,
        },
        HelpEntry {
            keys: ":pet path.gif",
            desc: "Load desktop pet GIF (Kitty graphics)",
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
            keys: "~/.xei/hooks.toml",
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
            desc: "Jump About / Setting / Pet / Help",
            is_header: false,
        },
        HelpEntry {
            keys: "j k · Enter",
            desc: "Move selection · activate",
            is_header: false,
        },
        HelpEntry {
            keys: "s",
            desc: "Save ~/.xei.toml",
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
    /// Index into `config::lsp_lang_catalog()`
    LspLang(usize),
    GitHeader,
    OpenWorkbench,
    OpenScm,
    /// Pet tab rows
    PetEnabled,
    PetPath,
    PetX,
    PetY,
    PetWidth,
    /// Playback speed (slider; h/l or Enter)
    PetSpeed,
    PetReload,
}

fn setting_rows() -> Vec<SettingRow> {
    let mut rows = vec![SettingRow::ThemeHeader];
    for i in 0..theme::all_themes().len() {
        rows.push(SettingRow::Theme(i));
    }
    rows.push(SettingRow::EditorHeader);
    rows.push(SettingRow::TabWidth);
    rows.push(SettingRow::RelativeNumber);
    rows.push(SettingRow::WrapLines);
    rows.push(SettingRow::UndoCaching);
    rows.push(SettingRow::ClipboardSync);
    rows.push(SettingRow::GpuAcc);
    rows.push(SettingRow::GpuGraphics);
    rows.push(SettingRow::GpuHyperlinks);
    rows.push(SettingRow::KeyHints);
    rows.push(SettingRow::LspHeader);
    rows.push(SettingRow::LspEnabled);
    for i in 0..config::lsp_lang_catalog().len() {
        rows.push(SettingRow::LspLang(i));
    }
    rows.push(SettingRow::GitHeader);
    rows.push(SettingRow::OpenWorkbench);
    rows.push(SettingRow::OpenScm);
    rows
}

fn pet_rows() -> Vec<SettingRow> {
    vec![
        SettingRow::PetEnabled,
        SettingRow::PetPath,
        SettingRow::PetX,
        SettingRow::PetY,
        SettingRow::PetWidth,
        SettingRow::PetSpeed,
        SettingRow::PetReload,
    ]
}

/// Discrete speed stops for Enter-cycle (percent).
const PET_SPEED_STOPS: &[u16] = &[25, 50, 75, 100, 125, 150, 200, 300, 400];

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
            SettingsPage::Pet => pet_rows().len(),
            SettingsPage::Extensions => 0,
            SettingsPage::Help => help_entries().len(),
        }
    }

    pub fn pet_rows(&self) -> Vec<SettingRow> {
        pet_rows()
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
            SettingsPage::Pet => 0,
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
        match self.page {
            SettingsPage::About | SettingsPage::Help => SettingsAction::None,
            SettingsPage::Extensions => SettingsAction::OpenPluginStore,
            SettingsPage::Pet => {
                let rows = pet_rows();
                let Some(row) = rows.get(self.selected).copied() else {
                    return SettingsAction::None;
                };
                match row {
                    SettingRow::PetEnabled => {
                        self.draft.pet_enabled = !self.draft.pet_enabled;
                        self.dirty = true;
                        self.status = Some(if self.draft.pet_enabled {
                            "pet = on  (requires gpu_acc + Kitty/Ghostty)".into()
                        } else {
                            "pet = off".into()
                        });
                        SettingsAction::ApplyPet
                    }
                    SettingRow::PetPath => {
                        self.status = Some(
                            "Set path: :pet ~/path/to/pet.gif  then s to save".into(),
                        );
                        SettingsAction::None
                    }
                    SettingRow::PetX => {
                        // Bounds applied at event layer from terminal size.
                        self.draft.pet_x = self.draft.pet_x.saturating_add(2);
                        self.dirty = true;
                        self.status = Some(format!("pet_x = {}", self.draft.pet_x));
                        SettingsAction::ApplyPet
                    }
                    SettingRow::PetY => {
                        self.draft.pet_y = self.draft.pet_y.saturating_add(1);
                        self.dirty = true;
                        self.status = Some(format!("pet_y = {}", self.draft.pet_y));
                        SettingsAction::ApplyPet
                    }
                    SettingRow::PetWidth => {
                        self.draft.pet_width_cells = match self.draft.pet_width_cells {
                            4..=7 => 12,
                            8..=15 => 20,
                            _ => 8,
                        };
                        self.dirty = true;
                        self.status =
                            Some(format!("pet_width_cells = {}", self.draft.pet_width_cells));
                        SettingsAction::ApplyPet
                    }
                    SettingRow::PetSpeed => {
                        self.draft.pet_speed = next_pet_speed(self.draft.pet_speed);
                        self.dirty = true;
                        self.status = Some(format!(
                            "pet_speed = {} ({})",
                            self.draft.pet_speed,
                            crate::pet::PetState::speed_label(self.draft.pet_speed)
                        ));
                        SettingsAction::ApplyPet
                    }
                    SettingRow::PetReload => {
                        self.status = Some("Reloading pet…".into());
                        SettingsAction::ApplyPet
                    }
                    _ => SettingsAction::None,
                }
            }
            SettingsPage::Setting => {
                let rows = setting_rows();
                let Some(row) = rows.get(self.selected).copied() else {
                    return SettingsAction::None;
                };
                match row {
                    SettingRow::Theme(i) => {
                        let themes = theme::all_themes();
                        if let Some(t) = themes.get(i) {
                            self.draft.theme = t.name.to_string();
                            self.dirty = true;
                            self.status = Some(format!("Theme → {}", t.name));
                            return SettingsAction::ApplyTheme;
                        }
                    }
                    SettingRow::TabWidth => {
                        self.draft.tab_width = match self.draft.tab_width {
                            2 => 4,
                            4 => 8,
                            _ => 2,
                        };
                        self.dirty = true;
                        self.status = Some(format!("tab_width = {}", self.draft.tab_width));
                    }
                    SettingRow::RelativeNumber => {
                        self.draft.relative_number = !self.draft.relative_number;
                        self.dirty = true;
                        self.status = Some(format!(
                            "relative_number = {}",
                            self.draft.relative_number
                        ));
                    }
                    SettingRow::UndoCaching => {
                        self.draft.undo_caching = !self.draft.undo_caching;
                        self.dirty = true;
                        self.status = Some(if self.draft.undo_caching {
                            "undo_caching = true  (history survives close · ~/.xei/undo)".into()
                        } else {
                            "undo_caching = false  (history discarded on close)".into()
                        });
                    }
                    SettingRow::WrapLines => {
                        self.draft.wrap_lines = !self.draft.wrap_lines;
                        self.dirty = true;
                        self.status = Some(if self.draft.wrap_lines {
                            "wrap_lines = true  (soft-wrap long lines)".into()
                        } else {
                            "wrap_lines = false  (horizontal scroll · zh/zl pan)".into()
                        });
                    }
                    SettingRow::ClipboardSync => {
                        self.draft.clipboard_sync = !self.draft.clipboard_sync;
                        self.dirty = true;
                        self.status = Some(format!(
                            "clipboard_sync = {}",
                            self.draft.clipboard_sync
                        ));
                    }
                    SettingRow::GpuAcc => {
                        self.draft.gpu_acc = !self.draft.gpu_acc;
                        self.dirty = true;
                        self.status = Some(if self.draft.gpu_acc {
                            "gpu_acc = true  (Ghostty/Kitty enhancements on)".into()
                        } else {
                            "gpu_acc = false  (plain cell TUI)".into()
                        });
                        return SettingsAction::ApplyGpuAcc;
                    }
                    SettingRow::GpuGraphics => {
                        self.draft.gpu_graphics = !self.draft.gpu_graphics;
                        self.dirty = true;
                        self.status = Some(format!("gpu_graphics = {}", self.draft.gpu_graphics));
                    }
                    SettingRow::GpuHyperlinks => {
                        self.draft.gpu_hyperlinks = !self.draft.gpu_hyperlinks;
                        self.dirty = true;
                        self.status =
                            Some(format!("gpu_hyperlinks = {}", self.draft.gpu_hyperlinks));
                    }
                    SettingRow::KeyHints => {
                        self.draft.key_hints = !self.draft.key_hints;
                        self.dirty = true;
                        self.status = Some(format!(
                            "key_hints = {}",
                            if self.draft.key_hints { "true" } else { "false" }
                        ));
                    }
                    SettingRow::LspEnabled => {
                        self.draft.lsp_enabled = !self.draft.lsp_enabled;
                        self.dirty = true;
                        self.status = Some(format!(
                            "lsp_enabled = {}",
                            self.draft.lsp_enabled
                        ));
                        return SettingsAction::ApplyLsp;
                    }
                    SettingRow::LspLang(i) => {
                        // Cycle: default → off → default
                        let catalog = config::lsp_lang_catalog();
                        if let Some((key, _label, default_cmd)) = catalog.get(i) {
                            let cur = self.draft.lsp_servers.get(*key).cloned();
                            match cur.as_deref() {
                                None => {
                                    // was default → turn off
                                    self.draft
                                        .lsp_servers
                                        .insert((*key).to_string(), String::new());
                                    self.status = Some(format!("lsp.{key} = off"));
                                }
                                Some("") => {
                                    // was off → restore default (remove override)
                                    self.draft.lsp_servers.remove(*key);
                                    self.status =
                                        Some(format!("lsp.{key} = default ({default_cmd})"));
                                }
                                Some(_) => {
                                    // custom → off
                                    self.draft
                                        .lsp_servers
                                        .insert((*key).to_string(), String::new());
                                    self.status = Some(format!("lsp.{key} = off"));
                                }
                            }
                            self.dirty = true;
                            return SettingsAction::ApplyLsp;
                        }
                    }
                    SettingRow::OpenWorkbench => {
                        return SettingsAction::OpenWorkbench;
                    }
                    SettingRow::OpenScm => {
                        return SettingsAction::OpenScm;
                    }
                    SettingRow::ThemeHeader
                    | SettingRow::EditorHeader
                    | SettingRow::LspHeader
                    | SettingRow::GitHeader
                    | SettingRow::PetEnabled
                    | SettingRow::PetPath
                    | SettingRow::PetX
                    | SettingRow::PetY
                    | SettingRow::PetWidth
                    | SettingRow::PetSpeed
                    | SettingRow::PetReload => {}
                }
                SettingsAction::None
            }
        }
    }

    pub fn save(&mut self) {
        config::save(&self.draft);
        self.dirty = false;
        self.status = Some("Saved ~/.xei.toml".into());
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
    ApplyPet,
    OpenWorkbench,
    OpenScm,
    OpenPluginStore,
}

fn next_pet_speed(cur: u16) -> u16 {
    for (i, &s) in PET_SPEED_STOPS.iter().enumerate() {
        if cur < s {
            return s;
        }
        if cur == s {
            return PET_SPEED_STOPS
                .get(i + 1)
                .copied()
                .unwrap_or(PET_SPEED_STOPS[0]);
        }
    }
    100
}

fn prev_pet_speed(cur: u16) -> u16 {
    let mut prev = *PET_SPEED_STOPS.last().unwrap_or(&100);
    for &s in PET_SPEED_STOPS {
        if cur <= s {
            return prev;
        }
        prev = s;
    }
    prev
}

impl SettingsPanel {
    /// Nudge pet position / speed with h/l or ←/→.
    /// `max_x` / `max_y` should be terminal size (so pet can reach the edges).
    pub fn nudge_pet(&mut self, dir: i16, max_x: u16, max_y: u16) -> SettingsAction {
        if self.page != SettingsPage::Pet {
            return SettingsAction::None;
        }
        let rows = pet_rows();
        let Some(row) = rows.get(self.selected).copied() else {
            return SettingsAction::None;
        };
        match row {
            SettingRow::PetX => {
                if dir < 0 {
                    self.draft.pet_x = self.draft.pet_x.saturating_sub(1);
                } else {
                    self.draft.pet_x = self.draft.pet_x.saturating_add(1).min(max_x);
                }
                self.dirty = true;
                self.status = Some(format!("pet_x = {}  (0..{max_x})", self.draft.pet_x));
                SettingsAction::ApplyPet
            }
            SettingRow::PetY => {
                if dir < 0 {
                    self.draft.pet_y = self.draft.pet_y.saturating_sub(1);
                } else {
                    self.draft.pet_y = self.draft.pet_y.saturating_add(1).min(max_y);
                }
                self.dirty = true;
                self.status = Some(format!("pet_y = {}  (0..{max_y})", self.draft.pet_y));
                SettingsAction::ApplyPet
            }
            SettingRow::PetWidth => {
                if dir < 0 {
                    self.draft.pet_width_cells =
                        self.draft.pet_width_cells.saturating_sub(1).max(4);
                } else {
                    self.draft.pet_width_cells =
                        self.draft.pet_width_cells.saturating_add(1).min(40);
                }
                self.dirty = true;
                self.status = Some(format!("pet_width_cells = {}", self.draft.pet_width_cells));
                SettingsAction::ApplyPet
            }
            SettingRow::PetSpeed => {
                self.draft.pet_speed = if dir < 0 {
                    prev_pet_speed(self.draft.pet_speed)
                } else {
                    next_pet_speed(self.draft.pet_speed)
                };
                self.dirty = true;
                self.status = Some(format!(
                    "pet_speed = {} ({})",
                    self.draft.pet_speed,
                    crate::pet::PetState::speed_label(self.draft.pet_speed)
                ));
                SettingsAction::ApplyPet
            }
            _ => SettingsAction::None,
        }
    }
}
