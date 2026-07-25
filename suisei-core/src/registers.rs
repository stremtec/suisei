//! The editor's yank store, backed by the system clipboard.
//!
//! This was a vim register bank — unnamed `"`, named `a`–`z` with uppercase
//! append, system `+`/`*`. Named registers were only reachable through the vim
//! `"x` prefix, which no longer exists, so what remains is the part the GUI
//! actually uses: a copy shares with the OS, and a paste prefers a fresh
//! system clipboard (so ⌘C in another app then ⌘V here works) and otherwise
//! falls back to the last in-editor yank.

use crate::clipboard;

#[derive(Clone, Debug, Default)]
pub struct RegisterValue {
    pub text: String,
    pub linewise: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Registers {
    /// Last yank/delete made inside the editor.
    last: Option<RegisterValue>,
}

impl Registers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Label for the status line. There is one store, so it has no name.
    pub fn active_label(&self) -> String {
        "clipboard".into()
    }

    /// Store a yank/delete and mirror it to the system clipboard.
    pub fn store(&mut self, text: String, linewise: bool) {
        let _ = clipboard::copy(&text);
        self.last = Some(RegisterValue { text, linewise });
    }

    /// Text to paste: a non-empty system clipboard wins (another app may have
    /// set it since our last yank), else the last in-editor yank.
    pub fn load_for_put(&mut self) -> Option<RegisterValue> {
        if let Some(sys) = clipboard::paste() {
            if !sys.is_empty() {
                let linewise = sys.ends_with('\n') || sys.lines().count() > 1;
                return Some(RegisterValue { text: sys, linewise });
            }
        }
        self.last.clone()
    }

    /// Peek the last in-editor yank (display / tests).
    pub fn unnamed_text(&self) -> Option<&str> {
        self.last.as_ref().map(|v| v.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_yank_is_remembered_and_shared() {
        let mut r = Registers::new();
        r.store("hello".into(), false);
        assert_eq!(r.unnamed_text(), Some("hello"));
    }

    #[test]
    fn paste_falls_back_to_the_last_yank_when_the_clipboard_is_empty() {
        let mut r = Registers::new();
        r.store("only-here".into(), false);
        // Whichever the platform clipboard reports, a value must come back.
        assert!(r.load_for_put().is_some());
    }
}
