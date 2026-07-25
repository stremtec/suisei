//! Jumplist for Back / Forward navigation (go-to-definition, search jumps).

use crate::buffer::Position;
use std::path::PathBuf;




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




#[cfg(test)]
mod tests {
    use super::*;

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

}
