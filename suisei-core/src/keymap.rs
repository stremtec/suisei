//! What key runs which command, and how the user changes it.
//!
//! The Shortcuts page listed sixteen commands as **hand-written strings in
//! Swift** — `("Save", "⌘S")` — beside a read-only dump of core's engine
//! bindings. So the page showed what the keys were and had no way to be wrong
//! about it, because it was not connected to anything: nothing linked the row
//! "Save" to the key that actually saves. Rebinding was not a missing button,
//! it was a missing identity.
//!
//! This is that identity. A command has an id, a title, a group and a default
//! chord; a binding is the default unless the user has said otherwise. The
//! override lives in `Config` — global, not per project, because a shortcut is
//! the reader's and not a fact about the code (the same line that keeps
//! `project.suiseiprj` down to `tab_width` and the language servers).
//!
//! Scope, deliberately: these are the **menu commands**. Core's modal engine
//! bindings stay a reference list — they are a vim keymap, they compose
//! (`d`,`i`,`w`), and "what does `diw` rebind to" is a different feature with a
//! different answer.

use crate::config::Config;

/// A rebindable command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// Stable id — what the config file stores. Never shown, never translated.
    pub id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    /// The chord Suisei ships with, in the same notation the user sees.
    pub default: &'static str,
}

/// Everything a user may rebind, in the order the Shortcuts page shows it.
///
/// **This list is exactly the menu commands whose key equivalent is wired to
/// it.** Nothing else belongs here: a row the menu does not read is a promise
/// the page cannot keep, and the page it replaced was already breaking one — it
/// printed "Terminal in Editor Pane ⌃⇧T" beside a menu item that has always
/// been ⇧⌘T. Adding a command means wiring it in `SuiseiApp.swift` in the same
/// change.
///
/// ⌃T (docked shell) and ⌃⇥ (tab cycling) are deliberately absent: those keys
/// are core's, not the menu's, and rebinding them is the modal-keymap feature.
///
/// The ids are the contract with the config file: renaming one silently drops
/// that user's override, so they do not get renamed.
pub const CATALOG: &[Command] = &[
    // File
    Command { id: "new_file",        title: "New Untitled",            group: "File",    default: "⌘N" },
    Command { id: "new_project",     title: "New Project…",            group: "File",    default: "⇧⌘N" },
    Command { id: "open",            title: "Open…",                   group: "File",    default: "⌘O" },
    Command { id: "save",            title: "Save",                    group: "File",    default: "⌘S" },
    Command { id: "save_as",         title: "Save As…",                group: "File",    default: "⇧⌘S" },
    // Editing
    Command { id: "find",            title: "Find…",                   group: "Editing", default: "⌘F" },
    Command { id: "find_next",       title: "Find Next",               group: "Editing", default: "⌘G" },
    Command { id: "find_prev",       title: "Find Previous",           group: "Editing", default: "⇧⌘G" },
    Command { id: "find_project",    title: "Find in Project…",        group: "Editing", default: "⇧⌘F" },
    Command { id: "find_replace",    title: "Find and Replace…",       group: "Editing", default: "⌥⌘F" },
    Command { id: "comment",         title: "Toggle Comment",          group: "Editing", default: "⌘/" },
    Command { id: "format",          title: "Format Document",         group: "Editing", default: "⇧⌘I" },
    Command { id: "code_actions",    title: "Code Actions…",           group: "Editing", default: "⌘." },
    // Xcode's chords for the same gesture. From a bare caret they add one
    // above/below; from a rectangle they grow it by a row — which is the same
    // act, because a column of carets IS a zero-width rectangle.
    Command { id: "cursor_above",    title: "Add Cursor Above",        group: "Editing", default: "⌃⇧↑" },
    Command { id: "cursor_below",    title: "Add Cursor Below",        group: "Editing", default: "⌃⇧↓" },
    // Navigate
    Command { id: "open_file",       title: "Go to File…",             group: "Navigate", default: "⌘P" },
    Command { id: "palette",         title: "Command Palette…",        group: "Navigate", default: "⇧⌘P" },
    Command { id: "definition",      title: "Go to Definition",        group: "Navigate", default: "⌃⌘J" },
    Command { id: "references",      title: "Find All References",     group: "Navigate", default: "⇧⌘R" },
    Command { id: "rename",          title: "Rename Symbol…",          group: "Navigate", default: "⌃⌘R" },
    Command { id: "logic",           title: "Show Logic",              group: "Navigate", default: "⌃⌘L" },
    // Panels
    Command { id: "explorer",        title: "File Explorer",           group: "Panels",  default: "⌃F" },
    Command { id: "scm",             title: "Source Control",          group: "Panels",  default: "⌃G" },
    Command { id: "git_workbench",   title: "Git Workbench",           group: "Panels",  default: "⌃⇧G" },
    Command { id: "nav",             title: "Toggle Navigator",        group: "Panels",  default: "⌘0" },
    Command { id: "inspector",       title: "Toggle Inspector",        group: "Panels",  default: "⌥⌘0" },
    Command { id: "debug_area",      title: "Toggle Debug Area",       group: "Panels",  default: "⇧⌘Y" },
    Command { id: "terminal_tab",    title: "New Terminal Tab",        group: "Panels",  default: "⇧⌘T" },
    Command { id: "preview",         title: "Pretty Preview",          group: "Panels",  default: "⇧⌘V" },
    Command { id: "settings",        title: "Settings…",               group: "Panels",  default: "⌘," },
    // Run
    Command { id: "build",           title: "Build",                   group: "Run",     default: "⌘B" },
    Command { id: "run",             title: "Run",                     group: "Run",     default: "⌘R" },
    Command { id: "test",            title: "Test",                    group: "Run",     default: "⌘U" },
    Command { id: "stop_build",      title: "Stop Build",              group: "Run",     default: "⇧⌘B" },
    Command { id: "breakpoint",      title: "Toggle Breakpoint",       group: "Run",     default: "⌘\\" },
];

pub fn command(id: &str) -> Option<&'static Command> {
    CATALOG.iter().find(|c| c.id == id)
}

/// A chord, as the parts a key equivalent is made of.
///
/// `key` is stored lowercase for letters so `⌘S` and `⌘s` are one binding —
/// AppKit's key equivalent is the unshifted character, and Shift is a
/// modifier, not a different key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub key: String,
}

impl Chord {
    /// Does this chord say anything at all?
    ///
    /// A bare letter is typing, not a shortcut. **Option plus a letter is also
    /// typing** — ⌥R is ® on a Mac keyboard, and a menu that claims it takes
    /// the character away from every text field in the app. Option with a
    /// NAMED key (⌥⇥, ⌥←) composes nothing and is fine.
    pub fn is_usable(&self) -> bool {
        !self.key.is_empty()
            && (self.cmd || self.ctrl || (self.alt && self.key.chars().count() > 1))
    }
}

/// Read the notation the user sees. Order of the glyphs does not matter.
///
/// Returns `None` for an empty or modifier-only chord — "⌘" alone is a user
/// halfway through pressing something, not a binding.
pub fn parse(chord: &str) -> Option<Chord> {
    let mut c = Chord { cmd: false, shift: false, alt: false, ctrl: false, key: String::new() };
    let mut rest = String::new();
    for ch in chord.chars() {
        match ch {
            '⌘' => c.cmd = true,
            '⇧' => c.shift = true,
            '⌥' => c.alt = true,
            '⌃' => c.ctrl = true,
            ' ' => {}
            _ => rest.push(ch),
        }
    }
    if rest.is_empty() {
        return None;
    }
    c.key = normalise_key(&rest);
    Some(c)
}

/// Back to the notation, in the order macOS draws modifiers: ⌃⌥⇧⌘.
pub fn format(c: &Chord) -> String {
    let mut s = String::new();
    if c.ctrl { s.push('⌃'); }
    if c.alt { s.push('⌥'); }
    if c.shift { s.push('⇧'); }
    if c.cmd { s.push('⌘'); }
    s.push_str(&display_key(&c.key));
    s
}

/// One spelling per key, so two ways of writing the same chord compare equal.
fn normalise_key(raw: &str) -> String {
    let t = raw.trim();
    match t {
        "⇥" | "Tab" | "tab" => "tab".into(),
        "⏎" | "↩" | "Return" | "return" | "Enter" => "return".into(),
        "⎋" | "Esc" | "esc" | "Escape" => "escape".into(),
        "␣" | "Space" | "space" => "space".into(),
        "⌫" | "Delete" | "delete" => "delete".into(),
        "←" => "left".into(),
        "→" => "right".into(),
        "↑" => "up".into(),
        "↓" => "down".into(),
        _ => {
            if t.chars().count() == 1 {
                t.to_lowercase()
            } else {
                t.to_lowercase()
            }
        }
    }
}

fn display_key(key: &str) -> String {
    match key {
        "tab" => "⇥".into(),
        "return" => "↩".into(),
        "escape" => "⎋".into(),
        "space" => "␣".into(),
        "delete" => "⌫".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => k.to_uppercase(),
    }
}

/// The chord in force for `id` — the user's, or the one Suisei ships with.
///
/// An override that no longer parses is ignored rather than obeyed: a
/// hand-edited config with a typo in it should leave the command working, not
/// leave it unreachable.
pub fn binding(cfg: &Config, id: &str) -> String {
    if let Some(over) = cfg.keybindings.get(id)
        && let Some(c) = parse(over)
        && c.is_usable()
    {
        return format(&c);
    }
    command(id).map(|c| c.default.to_string()).unwrap_or_default()
}

/// True when `id` is not on its shipped chord.
pub fn is_customised(cfg: &Config, id: &str) -> bool {
    let Some(cmd) = command(id) else { return false };
    binding(cfg, id) != cmd.default
}

/// Which OTHER command already answers to this chord, if any.
///
/// Asked before a change is stored, because two menu items with one key
/// equivalent is a state AppKit resolves by picking one — silently, and not
/// necessarily the one you just set.
pub fn conflict(cfg: &Config, id: &str, chord: &str) -> Option<&'static Command> {
    let Some(want) = parse(chord) else { return None };
    let want = format(&want);
    CATALOG
        .iter()
        .find(|c| c.id != id && binding(cfg, c.id) == want)
}

/// Give `id` a new chord. `None` puts it back on the shipped one.
///
/// Returns false when the chord could not be a shortcut at all — a bare letter,
/// or nothing. The caller keeps the old binding and says so; storing an
/// unusable chord would make the command unreachable and look like a save that
/// worked.
pub fn set(cfg: &mut Config, id: &str, chord: Option<&str>) -> bool {
    let Some(cmd) = command(id) else { return false };
    let Some(chord) = chord else {
        cfg.keybindings.remove(id);
        return true;
    };
    let Some(parsed) = parse(chord) else { return false };
    if !parsed.is_usable() {
        return false;
    }
    let text = format(&parsed);
    // Back on the default is not an override — it is the absence of one, and
    // the config file should not carry a line saying "the same as shipped".
    if text == cmd.default {
        cfg.keybindings.remove(id);
    } else {
        cfg.keybindings.insert(id.to_string(), text);
    }
    true
}

/// Every command back to its shipped chord.
pub fn reset_all(cfg: &mut Config) {
    cfg.keybindings.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn a_command_with_no_override_is_on_its_shipped_chord() {
        let c = cfg();
        assert_eq!(binding(&c, "save"), "⌘S");
        assert!(!is_customised(&c, "save"));
    }

    #[test]
    fn setting_and_clearing_a_binding() {
        let mut c = cfg();
        assert!(set(&mut c, "save", Some("⌃⌥S")));
        assert_eq!(binding(&c, "save"), "⌃⌥S");
        assert!(is_customised(&c, "save"));

        assert!(set(&mut c, "save", None));
        assert_eq!(binding(&c, "save"), "⌘S");
        assert!(c.keybindings.is_empty(), "and no line left in the file");
    }

    /// Setting a command back to the chord it already ships with is the absence
    /// of an override, not an override that happens to agree. Otherwise the
    /// config accumulates lines that say nothing and "Reset" has work to do
    /// where nothing was changed.
    #[test]
    fn setting_the_shipped_chord_records_nothing() {
        let mut c = cfg();
        assert!(set(&mut c, "save", Some("⌘S")));
        assert!(c.keybindings.is_empty());
        assert!(!is_customised(&c, "save"));
    }

    /// Modifier order is presentation. ⌘⇧P and ⇧⌘P are one chord, and the
    /// stored form is the one macOS draws.
    #[test]
    fn modifier_order_does_not_make_a_different_chord() {
        let mut a = cfg();
        let mut b = cfg();
        set(&mut a, "palette", Some("⌘⇧P"));
        set(&mut b, "palette", Some("⇧⌘P"));
        assert_eq!(binding(&a, "palette"), binding(&b, "palette"));
        assert_eq!(binding(&a, "palette"), "⇧⌘P");
    }

    /// Case is presentation too — AppKit's key equivalent is the unshifted
    /// character, and Shift is a modifier.
    #[test]
    fn case_is_not_part_of_the_chord() {
        let mut c = cfg();
        set(&mut c, "save", Some("⌃s"));
        assert_eq!(binding(&c, "save"), "⌃S");
    }

    /// Option and a letter is a character, not a chord. Claiming ⌥R for a menu
    /// takes ® away from every text field in the app. Option with a NAMED key
    /// composes nothing and is allowed.
    #[test]
    fn option_plus_a_letter_is_typing() {
        let mut c = cfg();
        assert!(!set(&mut c, "run", Some("⌥R")));
        assert!(set(&mut c, "run", Some("⌥⇥")), "⌥ and a named key is fine");
        assert_eq!(binding(&c, "run"), "⌥⇥");
    }

    /// A chord a menu cannot hold is refused, and the old one survives.
    #[test]
    fn a_bare_letter_is_not_a_shortcut() {
        let mut c = cfg();
        assert!(!set(&mut c, "save", Some("S")), "typing is not a shortcut");
        assert!(!set(&mut c, "save", Some("⇧S")), "nor is a capital letter");
        assert!(!set(&mut c, "save", Some("⌘")), "nor is a modifier alone");
        assert!(!set(&mut c, "save", Some("")));
        assert_eq!(binding(&c, "save"), "⌘S", "the command still works");
    }

    /// Two menu items on one key is a state AppKit resolves by picking one,
    /// silently, and not necessarily the one you just set. So it is reported
    /// before it happens.
    #[test]
    fn a_chord_another_command_holds_is_reported() {
        let c = cfg();
        let clash = conflict(&c, "save", "⌘P").expect("Open File has ⌘P");
        assert_eq!(clash.id, "open_file");

        assert!(conflict(&c, "save", "⌃⌥⇧S").is_none(), "nobody holds this");
        assert!(
            conflict(&c, "save", "⌘S").is_none(),
            "a command does not conflict with itself"
        );
    }

    /// Conflicts are asked of what is IN FORCE, not of the shipped table — the
    /// chord you are about to take may have been moved onto this command by an
    /// earlier edit.
    #[test]
    fn conflicts_follow_the_users_own_changes() {
        let mut c = cfg();
        set(&mut c, "find", Some("⌃⌥F"));
        assert!(conflict(&c, "save", "⌘F").is_none(), "Find left ⌘F");
        assert_eq!(
            conflict(&c, "save", "⌃⌥F").map(|x| x.id),
            Some("find"),
            "and took ⌃⌥F with it"
        );
    }

    /// A typo in a hand-edited config leaves the command working. The file is
    /// meant to be opened by hand, so it will be, and an unreachable Save is a
    /// worse answer than an ignored line.
    #[test]
    fn an_unreadable_override_falls_back_to_the_default() {
        let mut c = cfg();
        c.keybindings.insert("save".into(), "⌘".into());
        assert_eq!(binding(&c, "save"), "⌘S");
        c.keybindings.insert("save".into(), String::new());
        assert_eq!(binding(&c, "save"), "⌘S");
    }

    #[test]
    fn reset_puts_everything_back() {
        let mut c = cfg();
        set(&mut c, "save", Some("⌃⌥S"));
        set(&mut c, "run", Some("⌃⌥R"));
        reset_all(&mut c);
        assert_eq!(binding(&c, "save"), "⌘S");
        assert_eq!(binding(&c, "run"), "⌘R");
    }

    /// The ids are the contract with the config file, and the catalog is what
    /// the page enumerates — a duplicate id would shadow a command in both.
    #[test]
    fn the_catalog_is_well_formed() {
        let mut ids: Vec<&str> = CATALOG.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate command id");

        for c in CATALOG {
            let parsed = parse(c.default).unwrap_or_else(|| panic!("{}: {}", c.id, c.default));
            assert!(parsed.is_usable(), "{} ships with an unusable chord", c.id);
            assert_eq!(
                format(&parsed),
                c.default,
                "{} is written in a form that does not round-trip",
                c.id
            );
        }
    }

    /// Nothing ships on top of anything else.
    #[test]
    fn no_two_commands_ship_with_the_same_chord() {
        let c = cfg();
        for cmd in CATALOG {
            assert!(
                conflict(&c, cmd.id, cmd.default).is_none(),
                "{} ships on a chord another command also holds",
                cmd.id
            );
        }
    }
}
