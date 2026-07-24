//! Vim-style registers: unnamed `"`, named `a`–`z`, system `+` / `*`.

use crate::clipboard;

#[derive(Clone, Debug, Default)]
pub struct RegisterValue {
    pub text: String,
    pub linewise: bool,
}

#[derive(Clone, Debug)]
pub struct Registers {
    /// Unnamed register `"` — last yank/delete.
    unnamed: Option<RegisterValue>,
    /// Named `a`–`z`.
    named: [Option<RegisterValue>; 26],
    /// Active register for next op (`None` = unnamed).
    pub active: Option<char>,
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            unnamed: None,
            named: std::array::from_fn(|_| None),
            active: None,
        }
    }
}

impl Registers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn select(&mut self, name: char) -> bool {
        let c = name;
        if c.is_ascii_lowercase() || c == '+' || c == '*' || c == '"' {
            self.active = Some(c);
            true
        } else if c.is_ascii_uppercase() {
            // Uppercase = append on write; still select lower name
            self.active = Some(c);
            true
        } else {
            false
        }
    }

    pub fn clear_active(&mut self) {
        self.active = None;
    }

    pub fn active_label(&self) -> String {
        match self.active {
            Some(c) => format!("\"{}", c),
            None => "\"".into(),
        }
    }

    /// Store yank/delete into active register.
    ///
    /// **unnamedplus-style**: unnamed (`"`) yanks always mirror to the system
    /// clipboard so Cmd+V / other apps see editor yanks. Named registers still
    /// update unnamed + clipboard (modern sharing).
    pub fn store(&mut self, text: String, linewise: bool) {
        let val = RegisterValue { text, linewise };
        let target = self.active.take().unwrap_or('"');

        match target {
            '"' => {
                // Always share with OS
                let _ = clipboard::copy(&val.text);
                self.unnamed = Some(val);
            }
            '+' | '*' => {
                let _ = clipboard::copy(&val.text);
                self.unnamed = Some(val);
            }
            c if c.is_ascii_lowercase() => {
                let idx = (c as u8 - b'a') as usize;
                self.named[idx] = Some(val.clone());
                // Named yanks also refresh unnamed + clipboard (VS Code-like share)
                let _ = clipboard::copy(&val.text);
                self.unnamed = Some(val);
            }
            c if c.is_ascii_uppercase() => {
                let idx = (c as u8 - b'A') as usize;
                let mut merged = self.named[idx].clone().unwrap_or_default();
                if merged.text.is_empty() {
                    merged = val.clone();
                } else if merged.linewise || val.linewise {
                    if !merged.text.ends_with('\n') {
                        merged.text.push('\n');
                    }
                    merged.text.push_str(val.text.trim_start_matches('\n'));
                    merged.linewise = true;
                } else {
                    merged.text.push_str(&val.text);
                }
                self.named[idx] = Some(merged.clone());
                let _ = clipboard::copy(&merged.text);
                self.unnamed = Some(merged);
            }
            _ => {
                let _ = clipboard::copy(&val.text);
                self.unnamed = Some(val);
            }
        }
    }

    /// Read text for put. Consumes active register selection.
    ///
    /// With no explicit register, prefers **system clipboard** when non-empty
    /// (so external Cmd+C then `p` works), else falls back to unnamed.
    pub fn load_for_put(&mut self) -> Option<RegisterValue> {
        let target = self.active.take().unwrap_or('"');
        match target {
            '"' => Self::load_unnamed_or_system(&self.unnamed),
            '+' | '*' => clipboard::paste().map(|text| {
                let linewise = text.ends_with('\n') || text.lines().count() > 1;
                RegisterValue { text, linewise }
            }),
            c if c.is_ascii_lowercase() || c.is_ascii_uppercase() => {
                let idx = (c.to_ascii_lowercase() as u8 - b'a') as usize;
                self.named[idx].clone()
            }
            _ => Self::load_unnamed_or_system(&self.unnamed),
        }
    }

    fn load_unnamed_or_system(unnamed: &Option<RegisterValue>) -> Option<RegisterValue> {
        // Prefer fresh system clipboard when available (external copy → editor put).
        if let Some(sys) = clipboard::paste() {
            if !sys.is_empty() {
                let linewise = sys.ends_with('\n') || sys.lines().count() > 1;
                return Some(RegisterValue {
                    text: sys,
                    linewise,
                });
            }
        }
        unnamed.clone()
    }

    /// Peek unnamed (for compatibility / display).
    pub fn unnamed_text(&self) -> Option<&str> {
        self.unnamed.as_ref().map(|v| v.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_yank_and_put() {
        let mut r = Registers::new();
        r.select('a');
        r.store("hello".into(), false);
        assert_eq!(r.unnamed_text(), Some("hello"));
        r.select('a');
        let v = r.load_for_put().unwrap();
        assert_eq!(v.text, "hello");
        assert!(!v.linewise);
    }

    #[test]
    fn append_register() {
        let mut r = Registers::new();
        r.select('a');
        r.store("foo".into(), false);
        r.select('A');
        r.store("bar".into(), false);
        r.select('a');
        assert_eq!(r.load_for_put().unwrap().text, "foobar");
    }
}
