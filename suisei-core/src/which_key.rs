//! Which-key style chord maps — discoverable prefixes for Normal mode.
//!
//! Instant feedback after a short delay (see [`WhichKeyState::ready`]) so fast
//! typists never see a flash. Space is the leader key with nested menus.

use std::time::{Duration, Instant};

/// Delay before the chord popup appears.
pub const WHICH_KEY_DELAY_MS: u64 = 220;

/// One row in a which-key menu: key label + description.
#[derive(Debug, Clone, Copy)]
pub struct ChordHint {
    pub key: &'static str,
    pub desc: &'static str,
}

impl ChordHint {
    pub const fn new(key: &'static str, desc: &'static str) -> Self {
        Self { key, desc }
    }
}

/// Active which-key / leader session.
#[derive(Debug, Clone)]
pub struct WhichKeyState {
    /// Leader path after Space. `None` = leader inactive.
    /// `Some("")` = Space root; `Some("f")` = Space-f files menu; etc.
    pub leader: Option<String>,
    /// Title shown on the popup (e.g. `g`, `SPC f`, `Ctrl+W`).
    pub title: String,
    /// When this prefix session started (for delay).
    pub started: Option<Instant>,
}

impl Default for WhichKeyState {
    fn default() -> Self {
        Self {
            leader: None,
            title: String::new(),
            started: None,
        }
    }
}

impl WhichKeyState {
    pub fn clear(&mut self) {
        self.leader = None;
        self.title.clear();
        self.started = None;
    }

    pub fn is_leader(&self) -> bool {
        self.leader.is_some()
    }

    /// Begin a non-leader prefix (g, z, d, …). Caller also fills `pending_hints`.
    pub fn begin_prefix(&mut self, title: &str) {
        self.leader = None;
        self.title = title.to_string();
        self.started = Some(Instant::now());
    }

    /// Open Space leader at root.
    pub fn begin_leader(&mut self) {
        self.leader = Some(String::new());
        self.title = "SPC".into();
        self.started = Some(Instant::now());
    }

    /// Enter a submenu under the leader (`f`, `g`, …).
    pub fn enter_leader_sub(&mut self, key: char, title_suffix: &str) {
        self.leader = Some(key.to_string());
        self.title = format!("SPC {title_suffix}");
        self.started = Some(Instant::now());
    }

    /// True when the popup should paint (delay elapsed).
    pub fn ready(&self) -> bool {
        match self.started {
            Some(t) => t.elapsed() >= Duration::from_millis(WHICH_KEY_DELAY_MS),
            None => false,
        }
    }

    /// Force ready (for tests or immediate show).
    pub fn force_ready(&mut self) {
        self.started = Some(Instant::now() - Duration::from_millis(WHICH_KEY_DELAY_MS + 1));
    }
}

// ── Static maps ────────────────────────────────────────────────────────────

static MAP_G: &[ChordHint] = &[
    ChordHint::new("g", "go to top"),
    ChordHint::new("d", "go to definition"),
    ChordHint::new("p", "peek definition"),
    ChordHint::new("r", "references"),
    ChordHint::new("C", "call hierarchy"),
    ChordHint::new("I", "incoming calls"),
    ChordHint::new("H", "outgoing calls"),
    ChordHint::new("O", "document symbols"),
    ChordHint::new("b", "git blame panel"),
    ChordHint::new("t", "next tab"),
    ChordHint::new("T", "prev tab"),
];

static MAP_Z: &[ChordHint] = &[
    ChordHint::new("a", "toggle fold"),
    ChordHint::new("c", "close fold"),
    ChordHint::new("o", "open fold"),
    ChordHint::new("M", "close all folds"),
    ChordHint::new("R", "open all folds"),
];

static MAP_BRACKET_CLOSE: &[ChordHint] = &[
    ChordHint::new("d", "next diagnostic"),
    ChordHint::new("c", "next git change"),
];

static MAP_BRACKET_OPEN: &[ChordHint] = &[
    ChordHint::new("d", "prev diagnostic"),
    ChordHint::new("c", "prev git change"),
];

static MAP_CTRL_W: &[ChordHint] = &[
    ChordHint::new("v", "vertical split"),
    ChordHint::new("s", "horizontal split"),
    ChordHint::new("w", "other pane"),
    ChordHint::new("q", "close split"),
    ChordHint::new("=", "equalize"),
    ChordHint::new("h/l", "focus left/right"),
    ChordHint::new("j/k", "focus down/up"),
    ChordHint::new("</>", "resize"),
];

static MAP_REGISTER: &[ChordHint] = &[
    ChordHint::new("a-z", "named register"),
    ChordHint::new("A-Z", "append named"),
    ChordHint::new("+/*", "system clipboard"),
    ChordHint::new("\"", "unnamed"),
];

static MAP_MARK_SET: &[ChordHint] = &[ChordHint::new("a-z", "set mark")];
static MAP_MARK_JUMP_LINE: &[ChordHint] = &[ChordHint::new("a-z", "jump to mark (line)")];
static MAP_MARK_JUMP_EXACT: &[ChordHint] = &[ChordHint::new("a-z", "jump to mark (exact)")];
static MAP_MACRO_RECORD: &[ChordHint] = &[ChordHint::new("a-z", "record macro")];
static MAP_MACRO_PLAY: &[ChordHint] = &[
    ChordHint::new("a-z", "play macro"),
    ChordHint::new("@", "repeat last"),
];

static MAP_OP_DELETE: &[ChordHint] = &[
    ChordHint::new("d", "delete line"),
    ChordHint::new("w", "word"),
    ChordHint::new("iw", "inner word"),
    ChordHint::new("$", "to end of line"),
    ChordHint::new("i\"", "in quotes"),
    ChordHint::new("ib", "in parens"),
    ChordHint::new("G", "to EOF"),
    ChordHint::new("gg", "to BOF"),
];

static MAP_OP_CHANGE: &[ChordHint] = &[
    ChordHint::new("c", "change line"),
    ChordHint::new("w", "word"),
    ChordHint::new("iw", "inner word"),
    ChordHint::new("$", "to eol"),
    ChordHint::new("i\"", "in quotes"),
    ChordHint::new("ib", "in parens"),
];

static MAP_OP_YANK: &[ChordHint] = &[
    ChordHint::new("y", "yank line"),
    ChordHint::new("w", "word"),
    ChordHint::new("iw", "inner word"),
    ChordHint::new("$", "to eol"),
    ChordHint::new("i\"", "in quotes"),
];

static MAP_TEXTOBJECT: &[ChordHint] = &[
    ChordHint::new("w", "word"),
    ChordHint::new("W", "WORD"),
    ChordHint::new("\"/'", "quotes"),
    ChordHint::new("b )", "parens"),
    ChordHint::new("B }", "braces"),
    ChordHint::new("[ ]", "brackets"),
    ChordHint::new("t", "tag (html)"),
];

static MAP_SPACE_ROOT: &[ChordHint] = &[
    ChordHint::new("f", "files…"),
    ChordHint::new("b", "buffers…"),
    ChordHint::new("g", "git…"),
    ChordHint::new("l", "lsp…"),
    ChordHint::new("d", "debug…"),
    ChordHint::new("w", "window…"),
    ChordHint::new("s", "search…"),
    ChordHint::new("c", "code…"),
    ChordHint::new("t", "toggle…"),
    ChordHint::new("h", "help / settings"),
    ChordHint::new("p", "command palette"),
    ChordHint::new("/", "find in files"),
    ChordHint::new(",", "settings"),
    ChordHint::new("e", "file explorer"),
    ChordHint::new(";", "XLC command"),
];

static MAP_SPACE_D: &[ChordHint] = &[
    ChordHint::new("d", "debug panel focus"),
    ChordHint::new("s", "start / continue (F5)"),
    ChordHint::new("b", "toggle breakpoint (F9)"),
    ChordHint::new("n", "step over (F10)"),
    ChordHint::new("i", "step into (F11)"),
    ChordHint::new("o", "step out (Shift+F11)"),
    ChordHint::new("p", "pause (F6)"),
    ChordHint::new("x", "stop (Shift+F5)"),
    ChordHint::new("r", "restart"),
    ChordHint::new("c", "launch.json configs"),
    ChordHint::new("a", "attach help"),
];

static MAP_SPACE_F: &[ChordHint] = &[
    ChordHint::new("f", "quick open file"),
    ChordHint::new("e", "toggle explorer"),
    ChordHint::new("s", "save"),
    ChordHint::new("S", "save as (:w)"),
    ChordHint::new("p", "pretty preview"),
    ChordHint::new("r", "reload from disk"),
];

static MAP_SPACE_B: &[ChordHint] = &[
    ChordHint::new("n", "next tab"),
    ChordHint::new("p", "prev tab"),
    ChordHint::new("d", "close buffer"),
    ChordHint::new("b", "quick open"),
    ChordHint::new("1-9", "goto tab (if open)"),
];

static MAP_SPACE_G: &[ChordHint] = &[
    ChordHint::new("g", "git workbench"),
    ChordHint::new("s", "source control"),
    ChordHint::new("b", "blame panel"),
    ChordHint::new("r", "interactive rebase"),
    ChordHint::new("v", "PR review (selected)"),
    ChordHint::new("f", "fetch"),
    ChordHint::new("p", "pull"),
    ChordHint::new("P", "push"),
];

static MAP_SPACE_L: &[ChordHint] = &[
    ChordHint::new("d", "definition"),
    ChordHint::new("r", "references"),
    ChordHint::new("c", "call hierarchy"),
    ChordHint::new("h", "hover (K)"),
    ChordHint::new("a", "code actions"),
    ChordHint::new("f", "format document"),
    ChordHint::new("o", "outline / symbols"),
    ChordHint::new("R", "rename"),
    ChordHint::new("n", "next diagnostic"),
    ChordHint::new("p", "prev diagnostic"),
];

static MAP_SPACE_W: &[ChordHint] = &[
    ChordHint::new("v", "vertical split"),
    ChordHint::new("s", "horizontal split"),
    ChordHint::new("w", "other pane"),
    ChordHint::new("q", "close split"),
    ChordHint::new("=", "equalize"),
    ChordHint::new("t", "terminal split"),
];

static MAP_SPACE_S: &[ChordHint] = &[
    ChordHint::new("s", "search in buffer"),
    ChordHint::new("S", "search backward"),
    ChordHint::new("f", "find in files"),
    ChordHint::new("o", "document symbols"),
    ChordHint::new("w", "workspace symbols"),
];

static MAP_SPACE_C: &[ChordHint] = &[
    ChordHint::new("a", "code actions"),
    ChordHint::new("f", "format"),
    ChordHint::new("r", "rename"),
    ChordHint::new("d", "definition"),
    ChordHint::new("R", "references"),
];

static MAP_SPACE_T: &[ChordHint] = &[
    ChordHint::new("b", "blame panel"),
    ChordHint::new("e", "explorer"),
    ChordHint::new("t", "terminal side"),
    ChordHint::new("T", "terminal full"),
    ChordHint::new("i", "inlay hints"),
    ChordHint::new("l", "code lens"),
    ChordHint::new("r", "relative numbers"),
    ChordHint::new("p", "pretty preview"),
];

static MAP_SPACE_H: &[ChordHint] = &[
    ChordHint::new("h", "settings · help"),
    ChordHint::new(",", "settings"),
    ChordHint::new("k", "key hints on/off"),
    ChordHint::new("s", "screensaver"),
];

pub fn map_g() -> &'static [ChordHint] {
    MAP_G
}
pub fn map_z() -> &'static [ChordHint] {
    MAP_Z
}
pub fn map_bracket_close() -> &'static [ChordHint] {
    MAP_BRACKET_CLOSE
}
pub fn map_bracket_open() -> &'static [ChordHint] {
    MAP_BRACKET_OPEN
}
pub fn map_ctrl_w() -> &'static [ChordHint] {
    MAP_CTRL_W
}
pub fn map_register() -> &'static [ChordHint] {
    MAP_REGISTER
}
pub fn map_mark_set() -> &'static [ChordHint] {
    MAP_MARK_SET
}
pub fn map_mark_jump_line() -> &'static [ChordHint] {
    MAP_MARK_JUMP_LINE
}
pub fn map_mark_jump_exact() -> &'static [ChordHint] {
    MAP_MARK_JUMP_EXACT
}
pub fn map_macro_record() -> &'static [ChordHint] {
    MAP_MACRO_RECORD
}
pub fn map_macro_play() -> &'static [ChordHint] {
    MAP_MACRO_PLAY
}
pub fn map_operator_delete() -> &'static [ChordHint] {
    MAP_OP_DELETE
}
pub fn map_operator_change() -> &'static [ChordHint] {
    MAP_OP_CHANGE
}
pub fn map_operator_yank() -> &'static [ChordHint] {
    MAP_OP_YANK
}
pub fn map_textobject() -> &'static [ChordHint] {
    MAP_TEXTOBJECT
}
pub fn map_space_root() -> &'static [ChordHint] {
    MAP_SPACE_ROOT
}
pub fn map_space_f() -> &'static [ChordHint] {
    MAP_SPACE_F
}
pub fn map_space_b() -> &'static [ChordHint] {
    MAP_SPACE_B
}
pub fn map_space_g() -> &'static [ChordHint] {
    MAP_SPACE_G
}
pub fn map_space_l() -> &'static [ChordHint] {
    MAP_SPACE_L
}
pub fn map_space_w() -> &'static [ChordHint] {
    MAP_SPACE_W
}
pub fn map_space_s() -> &'static [ChordHint] {
    MAP_SPACE_S
}
pub fn map_space_c() -> &'static [ChordHint] {
    MAP_SPACE_C
}
pub fn map_space_d() -> &'static [ChordHint] {
    MAP_SPACE_D
}
pub fn map_space_t() -> &'static [ChordHint] {
    MAP_SPACE_T
}
pub fn map_space_h() -> &'static [ChordHint] {
    MAP_SPACE_H
}

/// Convert static map → app hint vec.
pub fn as_hints(map: &[ChordHint]) -> Vec<(&'static str, &'static str)> {
    map.iter().map(|h| (h.key, h.desc)).collect()
}

/// Hints for the current leader path (`""` root, `"f"`, …).
pub fn leader_hints(path: &str) -> Vec<(&'static str, &'static str)> {
    let map = match path {
        "" => MAP_SPACE_ROOT,
        "f" => MAP_SPACE_F,
        "b" => MAP_SPACE_B,
        "g" => MAP_SPACE_G,
        "l" => MAP_SPACE_L,
        "w" => MAP_SPACE_W,
        "s" => MAP_SPACE_S,
        "c" => MAP_SPACE_C,
        "d" => MAP_SPACE_D,
        "t" => MAP_SPACE_T,
        "h" => MAP_SPACE_H,
        _ => MAP_SPACE_ROOT,
    };
    as_hints(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_gate() {
        let mut w = WhichKeyState::default();
        w.begin_leader();
        assert!(!w.ready());
        w.force_ready();
        assert!(w.ready());
    }

    #[test]
    fn leader_maps_nonempty() {
        assert!(!map_space_root().is_empty());
        assert!(!leader_hints("f").is_empty());
        assert!(!map_g().is_empty());
    }
}
