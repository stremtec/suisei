//! Simple vim-style macro recording / playback storage.
//!
//! Keys are stored as a portable enum; the TUI maps crossterm events → MacroKey
//! and replays by mapping back.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroKey {
    Char(char),
    Esc,
    Enter,
    Backspace,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    /// Ctrl + char (lowercase)
    Ctrl(char),
    /// Super/Cmd + char
    Super(char),
}

#[derive(Clone, Debug, Default)]
pub struct MacroBank {
    /// Named macros a-z
    slots: [Option<Vec<MacroKey>>; 26],
    /// Currently recording into this register
    pub recording: Option<char>,
    /// Keys captured while recording
    pub buffer: Vec<MacroKey>,
    /// Last played register for `@@`
    pub last_played: Option<char>,
}

impl MacroBank {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, name: char) -> bool {
        if !name.is_ascii_lowercase() {
            return false;
        }
        self.recording = Some(name);
        self.buffer.clear();
        true
    }

    pub fn stop(&mut self) {
        if let Some(name) = self.recording.take() {
            let idx = (name as u8 - b'a') as usize;
            self.slots[idx] = Some(std::mem::take(&mut self.buffer));
        }
        self.buffer.clear();
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    pub fn push(&mut self, key: MacroKey) {
        if self.recording.is_some() {
            // Don't record the final `q` that stops — caller should stop before push of stop key
            self.buffer.push(key);
        }
    }

    pub fn get(&self, name: char) -> Option<&[MacroKey]> {
        if !name.is_ascii_lowercase() {
            return None;
        }
        let idx = (name as u8 - b'a') as usize;
        self.slots[idx].as_deref()
    }

    pub fn set_last_played(&mut self, name: char) {
        self.last_played = Some(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_get() {
        let mut m = MacroBank::new();
        assert!(m.start('a'));
        m.push(MacroKey::Char('i'));
        m.push(MacroKey::Char('x'));
        m.push(MacroKey::Esc);
        m.stop();
        let keys = m.get('a').unwrap();
        assert_eq!(keys.len(), 3);
    }
}
