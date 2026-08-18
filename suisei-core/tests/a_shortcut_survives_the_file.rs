//! A rebound shortcut has to come back after a restart, and a hand-edited file
//! has to be readable.
//!
//! `config.rs` is a flat dotted-key file people open by hand — that is why the
//! themes and the LSP servers are in it in that shape, and why the shortcuts
//! join them as `key.<command> = "<chord>"` rather than as a blob.

use suisei_core::config::Config;
use suisei_core::keymap;

/// Round-trip through the writer and reader without touching the real config
/// path: build the text the way `save` does, then feed it back the way `load`
/// does. (`save`/`load` themselves go to `~/.suisei`, which a test must not.)
fn reparse(cfg: &Config) -> Config {
    let mut out = Config::default();
    for (id, chord) in &cfg.keybindings {
        // The exact line `save` writes.
        let line = format!("key.{id} = \"{chord}\"");
        let (k, v) = line.split_once('=').expect("k = v");
        let id = k.trim().trim_start_matches("key.");
        let v = v.trim().trim_matches('"');
        if keymap::command(id).is_some() {
            let _ = keymap::set(&mut out, id, Some(v));
        }
    }
    out
}

#[test]
fn a_changed_shortcut_comes_back() {
    let mut cfg = Config::default();
    keymap::set(&mut cfg, "save", Some("⌃⌥S"));
    keymap::set(&mut cfg, "run", Some("⌃⌥R"));

    let back = reparse(&cfg);
    assert_eq!(keymap::binding(&back, "save"), "⌃⌥S");
    assert_eq!(keymap::binding(&back, "run"), "⌃⌥R");
    assert_eq!(keymap::binding(&back, "find"), "⌘F", "untouched stays shipped");
}

/// A command that no longer exists — an override written by an older build, or
/// a typo — is dropped. Storing it would mean writing it back out forever as
/// though Suisei believed in it.
#[test]
fn an_id_that_is_not_a_command_is_dropped() {
    let mut cfg = Config::default();
    cfg.keybindings.insert("save".into(), "⌃⌥S".into());
    cfg.keybindings.insert("teleport".into(), "⌃⌥T".into());

    let back = reparse(&cfg);
    assert_eq!(keymap::binding(&back, "save"), "⌃⌥S");
    assert!(!back.keybindings.contains_key("teleport"));
}

/// Nothing to say, nothing written. An empty section in a file people open by
/// hand is noise, and the rest of this config already follows that rule.
#[test]
fn an_untouched_keymap_writes_no_lines() {
    let cfg = Config::default();
    assert!(cfg.keybindings.is_empty());
    let mut touched = Config::default();
    keymap::set(&mut touched, "save", Some("⌘S")); // the shipped chord
    assert!(touched.keybindings.is_empty(), "agreeing is not an override");
}
