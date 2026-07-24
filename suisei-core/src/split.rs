//! Editor pane splits (vertical / horizontal), up to [`MAX_PANES`] panes in a
//! single direction. Repeating `Ctrl+W v` / `Ctrl+W s` adds another pane next
//! to the focused one (Vim-style enough for daily use; no mixed-direction
//! trees yet).

/// Hard cap — panes get unusably narrow beyond this.
pub const MAX_PANES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplitKind {
    #[default]
    None,
    /// Side by side (left | right)
    Vertical,
    /// Stacked (top / bottom)
    Horizontal,
}

#[derive(Debug, Clone)]
pub struct Pane {
    pub tab_index: usize,
    pub scroll: usize,
    /// Horizontal pan (visual columns) when wrap_lines is off — per pane.
    pub hscroll: usize,
    /// Per-pane cursor (row, col) — Vim-style independent window cursors.
    pub cursor: (usize, usize),
}

impl Default for Pane {
    fn default() -> Self {
        Self {
            tab_index: 0,
            scroll: 0,
            hscroll: 0,
            cursor: (0, 0),
        }
    }
}

/// Outcome of a split request (drives the status message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAdd {
    Opened,
    Added,
    Full,
    /// Already split in the other direction — no mixed trees (yet).
    MixedKind,
}

#[derive(Debug, Clone)]
pub struct SplitState {
    pub kind: SplitKind,
    /// Divider position for the 2-pane case (drag-resize); ≥3 panes are equal.
    pub ratio: f32,
    /// Focused pane index.
    pub focus: usize,
    pub panes: Vec<Pane>,
    /// After `Ctrl+W` waiting for chord
    pub pending_chord: bool,
}

impl Default for SplitState {
    fn default() -> Self {
        Self {
            kind: SplitKind::None,
            ratio: 0.5,
            focus: 0,
            panes: vec![Pane::default(), Pane::default()],
            pending_chord: false,
        }
    }
}

impl SplitState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_split(&self) -> bool {
        self.kind != SplitKind::None && self.panes.len() >= 2
    }

    pub fn pane_count(&self) -> usize {
        if self.is_split() {
            self.panes.len()
        } else {
            1
        }
    }

    fn clamp_focus(&self) -> usize {
        self.focus.min(self.panes.len().saturating_sub(1))
    }

    pub fn focused_pane(&self) -> &Pane {
        &self.panes[self.clamp_focus()]
    }

    pub fn focused_pane_mut(&mut self) -> &mut Pane {
        let i = self.clamp_focus();
        &mut self.panes[i]
    }

    /// Open a split of the given kind over the current tab/scroll, or add
    /// another pane when already split in the same direction.
    pub fn open_split(
        &mut self,
        kind: SplitKind,
        tab: usize,
        scroll: usize,
        cursor: (usize, usize),
    ) -> SplitAdd {
        if kind == SplitKind::None {
            self.close();
            return SplitAdd::Opened;
        }
        self.pending_chord = false;
        if self.is_split() {
            if self.kind != kind {
                return SplitAdd::MixedKind;
            }
            if self.panes.len() >= MAX_PANES {
                return SplitAdd::Full;
            }
            // New pane opens next to the focused one and takes focus.
            let at = self.clamp_focus() + 1;
            self.panes.insert(
                at,
                Pane {
                    tab_index: tab,
                    scroll,
                    hscroll: 0,
                    cursor,
                },
            );
            self.focus = at;
            return SplitAdd::Added;
        }
        self.kind = kind;
        self.ratio = 0.5;
        self.focus = 0;
        self.panes = vec![
            Pane {
                tab_index: tab,
                scroll,
                hscroll: 0,
                cursor,
            },
            // Second pane starts on same tab (VS Code-ish); user can switch later.
            Pane {
                tab_index: tab,
                scroll,
                hscroll: 0,
                cursor,
            },
        ];
        SplitAdd::Opened
    }

    /// Remove the focused pane; focus lands on the neighbor. Returns the
    /// surviving pane snapshot to adopt when the split collapses to one.
    pub fn remove_focused(&mut self) -> Option<Pane> {
        if !self.is_split() {
            return None;
        }
        let idx = self.clamp_focus();
        self.panes.remove(idx);
        self.focus = idx.min(self.panes.len().saturating_sub(1));
        if self.panes.len() < 2 {
            let survivor = self.panes.first().cloned();
            self.close_keep_panes();
            return survivor;
        }
        Some(self.focused_pane().clone())
    }

    pub fn close(&mut self) {
        self.kind = SplitKind::None;
        self.focus = 0;
        self.pending_chord = false;
        self.panes = vec![Pane::default(), Pane::default()];
    }

    fn close_keep_panes(&mut self) {
        self.kind = SplitKind::None;
        self.focus = 0;
        self.pending_chord = false;
        if self.panes.is_empty() {
            self.panes = vec![Pane::default()];
        }
        while self.panes.len() < 2 {
            let last = self.panes.last().cloned().unwrap_or_default();
            self.panes.push(last);
        }
    }

    pub fn focus_other(&mut self) {
        if self.is_split() {
            self.focus = (self.clamp_focus() + 1) % self.panes.len();
        }
    }

    pub fn set_focus(&mut self, idx: usize) {
        if self.is_split() {
            self.focus = idx.min(self.panes.len() - 1);
        }
    }

    pub fn adjust_ratio(&mut self, delta: f32) {
        self.ratio = (self.ratio + delta).clamp(0.2, 0.8);
    }

    pub fn equalize(&mut self) {
        self.ratio = 0.5;
    }

    /// Keep pane tab indices valid after tab close/reorder.
    pub fn clamp_tabs(&mut self, n_tabs: usize) {
        if n_tabs == 0 {
            return;
        }
        for p in &mut self.panes {
            if p.tab_index >= n_tabs {
                p.tab_index = n_tabs - 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_splits_add_panes_up_to_cap() {
        let mut s = SplitState::new();
        assert_eq!(s.open_split(SplitKind::Vertical, 0, 0, (0, 0)), SplitAdd::Opened);
        assert_eq!(s.pane_count(), 2);
        assert_eq!(s.open_split(SplitKind::Vertical, 1, 3, (3, 0)), SplitAdd::Added);
        assert_eq!(s.pane_count(), 3);
        // New pane sits next to previous focus and takes focus.
        assert_eq!(s.focus, 1);
        assert_eq!(s.focused_pane().tab_index, 1);
        assert_eq!(s.open_split(SplitKind::Vertical, 0, 0, (0, 0)), SplitAdd::Added);
        assert_eq!(s.pane_count(), 4);
        assert_eq!(s.open_split(SplitKind::Vertical, 0, 0, (0, 0)), SplitAdd::Full);
        assert_eq!(s.open_split(SplitKind::Horizontal, 0, 0, (0, 0)), SplitAdd::MixedKind);
    }

    #[test]
    fn remove_focused_collapses_to_single() {
        let mut s = SplitState::new();
        s.open_split(SplitKind::Vertical, 0, 0, (0, 0));
        s.open_split(SplitKind::Vertical, 2, 9, (9, 0)); // 3 panes, focus=1 (tab 2)
        s.set_focus(1);
        let survivor = s.remove_focused().expect("still split");
        // Focus falls to the neighbor at the same index.
        assert!(s.is_split());
        assert_eq!(s.pane_count(), 2);
        assert_eq!(survivor.tab_index, s.focused_pane().tab_index);
        // Removing again collapses the split and yields the last survivor.
        let last = s.remove_focused().expect("survivor");
        assert!(!s.is_split());
        assert_eq!(last.tab_index, 0);
    }

    #[test]
    fn focus_cycles_all_panes() {
        let mut s = SplitState::new();
        s.open_split(SplitKind::Horizontal, 0, 0, (0, 0));
        s.open_split(SplitKind::Horizontal, 0, 0, (0, 0));
        assert_eq!(s.pane_count(), 3);
        s.set_focus(0);
        s.focus_other();
        s.focus_other();
        assert_eq!(s.focus, 2);
        s.focus_other();
        assert_eq!(s.focus, 0);
    }
}
