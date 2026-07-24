//! Marks, jumplist, and last f/t/T for `;` / `,` repeat.

use crate::buffer::Position;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Mark {
    pub pos: Position,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct Marks {
    /// File-local marks `a`–`z`.
    slots: [Option<Mark>; 26],
}

impl Marks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, name: char, pos: Position, path: Option<PathBuf>) -> bool {
        if !name.is_ascii_lowercase() {
            return false;
        }
        let idx = (name as u8 - b'a') as usize;
        self.slots[idx] = Some(Mark { pos, path });
        true
    }

    pub fn get(&self, name: char) -> Option<&Mark> {
        if !name.is_ascii_lowercase() {
            return None;
        }
        let idx = (name as u8 - b'a') as usize;
        self.slots[idx].as_ref()
    }
}

#[derive(Clone, Debug)]
pub struct Jump {
    pub pos: Position,
    pub scroll: usize,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct JumpList {
    entries: Vec<Jump>,
    /// Index of "current" position in list; jumps navigate relative to this.
    index: usize,
}

impl Default for JumpList {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            index: 0,
        }
    }
}

impl JumpList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a jump origin (call *before* moving).
    pub fn push(&mut self, jump: Jump) {
        // Drop forward history when branching
        if self.index + 1 < self.entries.len() {
            self.entries.truncate(self.index + 1);
        }
        // Avoid duplicate consecutive
        if let Some(last) = self.entries.last() {
            if last.pos == jump.pos && last.path == jump.path {
                return;
            }
        }
        self.entries.push(jump);
        // Cap size
        const MAX: usize = 100;
        if self.entries.len() > MAX {
            let drop = self.entries.len() - MAX;
            self.entries.drain(0..drop);
        }
        self.index = self.entries.len().saturating_sub(1);
    }

    /// Move back. Returns the jump to restore, after pushing `current` if needed.
    pub fn back(&mut self, current: Jump) -> Option<Jump> {
        if self.entries.is_empty() {
            return None;
        }
        // If we're at the tip, save current as the "now" entry
        if self.index + 1 >= self.entries.len() {
            if self.entries.last().map(|j| j.pos != current.pos).unwrap_or(true) {
                self.entries.push(current);
                self.index = self.entries.len() - 1;
            }
        }
        if self.index == 0 {
            return None;
        }
        self.index -= 1;
        self.entries.get(self.index).cloned()
    }

    pub fn forward(&mut self) -> Option<Jump> {
        if self.index + 1 >= self.entries.len() {
            return None;
        }
        self.index += 1;
        self.entries.get(self.index).cloned()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindKind {
    Find,  // f
    Till,  // t
}

#[derive(Clone, Copy, Debug)]
pub struct LastFind {
    pub ch: char,
    pub kind: FindKind,
    /// true = original was forward (f/t), false = F/T
    pub forward: bool,
}

impl LastFind {
    pub fn repeat(&self, reverse: bool) -> (FindKind, bool, char) {
        let forward = if reverse {
            !self.forward
        } else {
            self.forward
        };
        (self.kind, forward, self.ch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_roundtrip() {
        let mut m = Marks::new();
        m.set('a', Position::new(3, 2), None);
        let got = m.get('a').unwrap();
        assert_eq!(got.pos, Position::new(3, 2));
    }

    #[test]
    fn jumplist_back_forward() {
        let mut jl = JumpList::new();
        jl.push(Jump {
            pos: Position::new(0, 0),
            scroll: 0,
            path: None,
        });
        jl.push(Jump {
            pos: Position::new(10, 0),
            scroll: 5,
            path: None,
        });
        let cur = Jump {
            pos: Position::new(20, 0),
            scroll: 10,
            path: None,
        };
        let back = jl.back(cur).unwrap();
        assert_eq!(back.pos.row, 10);
        let fwd = jl.forward().unwrap();
        assert_eq!(fwd.pos.row, 20);
    }

    #[test]
    fn last_find_reverse() {
        let lf = LastFind {
            ch: 'x',
            kind: FindKind::Find,
            forward: true,
        };
        let (k, fwd, ch) = lf.repeat(true);
        assert_eq!(k, FindKind::Find);
        assert!(!fwd);
        assert_eq!(ch, 'x');
    }
}
