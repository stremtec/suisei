//! Editor pane splits (vertical / horizontal), up to [`MAX_PANES`] panes in a
//! single direction. Repeating `Ctrl+W v` / `Ctrl+W s` adds another pane next
//! to the focused one (Vim-style enough for daily use; no mixed-direction
//! trees yet).

use crate::app::BufferId;

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
    /// The document this pane shows, by stable id — **not** by position.
    ///
    /// Positions move: closing a tab or dragging one along the strip shifts
    /// every slot after it, and a pane holding a position is then pointing at
    /// whatever slid underneath it. That was `tab_index`, and it is the bug
    /// class described in `SUISEI-SPLIT-PLAN.md` §1.1. An id that never comes
    /// back means a stale reference fails to resolve — visible and repairable
    /// — instead of silently naming the wrong file.
    pub buffer: BufferId,
    pub scroll: usize,
    /// Horizontal pan (visual columns) when wrap_lines is off — per pane.
    pub hscroll: usize,
    /// Per-pane cursor (row, col) — Vim-style independent window cursors.
    pub cursor: (usize, usize),
}

impl Default for Pane {
    fn default() -> Self {
        Self {
            // `BufferId::default()` is the never-issued id, so a default pane
            // resolves to nothing and falls back to the active document.
            buffer: BufferId::default(),
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
        tab: BufferId,
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
                    buffer: tab,
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
                buffer: tab,
                scroll,
                hscroll: 0,
                cursor,
            },
            // Second pane starts on same tab (VS Code-ish); user can switch later.
            Pane {
                buffer: tab,
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

    /// Repoint every pane showing `gone` at `adopt`, for when a document is
    /// closed out from under a split.
    ///
    /// This is the *whole* repair surface now. `clamp_tabs` used to sit here
    /// and clamp out-of-range indices after any tab change, which is why
    /// closing tab 0 of four quietly slid every pane one document to the left:
    /// the indices stayed in range, so there was nothing to clamp, and each
    /// pane went on pointing at a slot that now held a different file. Ids do
    /// not have an in-range-but-wrong state — either the document is still
    /// open or it is not.
    pub fn repoint(&mut self, gone: BufferId, adopt: BufferId) {
        for p in &mut self.panes {
            if p.buffer == gone {
                p.buffer = adopt;
                // The old scroll and cursor were coordinates in a document that
                // is no longer there; carrying them over lands the caret at an
                // arbitrary row of an unrelated file.
                p.scroll = 0;
                p.hscroll = 0;
                p.cursor = (0, 0);
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
        assert_eq!(s.open_split(SplitKind::Vertical, BufferId(0), 0, (0, 0)), SplitAdd::Opened);
        assert_eq!(s.pane_count(), 2);
        assert_eq!(s.open_split(SplitKind::Vertical, BufferId(1), 3, (3, 0)), SplitAdd::Added);
        assert_eq!(s.pane_count(), 3);
        // New pane sits next to previous focus and takes focus.
        assert_eq!(s.focus, 1);
        assert_eq!(s.focused_pane().buffer, BufferId(1));
        assert_eq!(s.open_split(SplitKind::Vertical, BufferId(0), 0, (0, 0)), SplitAdd::Added);
        assert_eq!(s.pane_count(), 4);
        assert_eq!(s.open_split(SplitKind::Vertical, BufferId(0), 0, (0, 0)), SplitAdd::Full);
        assert_eq!(s.open_split(SplitKind::Horizontal, BufferId(0), 0, (0, 0)), SplitAdd::MixedKind);
    }

    #[test]
    fn remove_focused_collapses_to_single() {
        let mut s = SplitState::new();
        s.open_split(SplitKind::Vertical, BufferId(0), 0, (0, 0));
        s.open_split(SplitKind::Vertical, BufferId(2), 9, (9, 0)); // 3 panes, focus=1 (tab 2)
        s.set_focus(1);
        let survivor = s.remove_focused().expect("still split");
        // Focus falls to the neighbor at the same index.
        assert!(s.is_split());
        assert_eq!(s.pane_count(), 2);
        assert_eq!(survivor.buffer, s.focused_pane().buffer);
        // Removing again collapses the split and yields the last survivor.
        let last = s.remove_focused().expect("survivor");
        assert!(!s.is_split());
        assert_eq!(last.buffer, BufferId(0));
    }

    #[test]
    fn focus_cycles_all_panes() {
        let mut s = SplitState::new();
        s.open_split(SplitKind::Horizontal, BufferId(0), 0, (0, 0));
        s.open_split(SplitKind::Horizontal, BufferId(0), 0, (0, 0));
        assert_eq!(s.pane_count(), 3);
        s.set_focus(0);
        s.focus_other();
        s.focus_other();
        assert_eq!(s.focus, 2);
        s.focus_other();
        assert_eq!(s.focus, 0);
    }
}
