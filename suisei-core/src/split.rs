//! Editor pane layout: a tree of splits, up to [`MAX_PANES`] leaves.
//!
//! This replaced a flat `Vec<Pane>` with one `kind` and one `ratio` for the
//! whole layout. That model could not express two things the editor needs:
//!
//! * **Mixed directions.** One `kind` meant "split this pane the other way"
//!   had to be refused outright (`SplitAdd::MixedKind`), so the four-pane `+`
//!   was unreachable. A tree gets it for free — split right, split below, then
//!   split the other column below.
//! * **More than one divider.** One `ratio` describes one boundary. With three
//!   panes there are two, and both were driven by the same number, so neither
//!   could be dragged independently.
//!
//! The tree owns structure and geometry; [`SplitState::panes`] is a flat list
//! kept in **visual order** as a derived view, so index-addressed callers (the
//! compositor, the FFI pane array, the face) keep working unchanged.

use crate::app::BufferId;

/// Hard cap — panes get unusably narrow beyond this.
pub const MAX_PANES: usize = 4;

/// Smallest share of its parent a pane may be dragged down to.
const MIN_WEIGHT: f32 = 0.12;

/// Stable pane handle. Never reused, for the reasons in [`BufferId`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Children sit side by side — a vertical divider between them.
    Col,
    /// Children stack — a horizontal divider between them.
    Row,
}

/// Stable handle to a running shell. Never reused, like [`BufferId`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TerminalId(pub u32);

/// What a pane is showing.
///
/// "This pane is a terminal" used to be spread across four fields on
/// `App.terminal` — `open`, `full_panel`, `pane_bound`, `owns_split` — which
/// nothing kept consistent, and one of which (`owns_split`) was the terminal
/// remembering *why the split existed* so it could undo it later. It is a
/// property of the pane, so it lives on the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneContent {
    Document(BufferId),
    /// A shell runs here. `id` names **which** shell: panes each get their own,
    /// so two terminal panes are two processes. They shared one `App.terminal`
    /// at first, which made every terminal pane a second view of the same
    /// session — they looked linked because they were.
    ///
    /// `restore` is the document this pane was showing before, so closing the
    /// terminal puts it back instead of leaving a blank.
    Terminal { id: TerminalId, restore: BufferId },
}

#[derive(Debug, Clone)]
pub struct Pane {
    pub id: PaneId,
    /// Addressed by stable id — **not** by position. See
    /// `SUISEI-SPLIT-PLAN.md` §1.1.
    pub content: PaneContent,
    pub scroll: usize,
    /// Horizontal pan (visual columns) when wrap_lines is off — per pane.
    pub hscroll: usize,
    /// Per-pane cursor (row, col) — independent window cursors.
    pub cursor: (usize, usize),
}

impl Pane {
    fn new(id: PaneId) -> Self {
        Self {
            id,
            content: PaneContent::Document(BufferId::default()),
            scroll: 0,
            hscroll: 0,
            cursor: (0, 0),
        }
    }

    /// The document associated with this pane — the one it shows, or the one a
    /// terminal displaced. A terminal pane still names a document so its
    /// header and its restore path have something to point at.
    pub fn buffer(&self) -> BufferId {
        match self.content {
            PaneContent::Document(id) | PaneContent::Terminal { restore: id, .. } => id,
        }
    }

    /// The shell this pane runs, if it is a terminal.
    pub fn terminal_id(&self) -> Option<TerminalId> {
        match self.content {
            PaneContent::Terminal { id, .. } => Some(id),
            PaneContent::Document(_) => None,
        }
    }

    pub fn set_buffer(&mut self, id: BufferId) {
        self.content = match self.content {
            PaneContent::Document(_) => PaneContent::Document(id),
            // Retarget what the terminal will restore, without evicting it.
            PaneContent::Terminal { id: t, .. } => PaneContent::Terminal { id: t, restore: id },
        };
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.content, PaneContent::Terminal { .. })
    }
}

#[derive(Debug, Clone)]
pub enum Layout {
    Leaf(PaneId),
    Split {
        axis: Axis,
        children: Vec<Layout>,
        /// One weight per child. Relative, not normalised — only ratios matter.
        weights: Vec<f32>,
    },
}

/// A pane's share of the editor area, normalised to 0..1.
///
/// Geometry is computed here rather than in the face because the face cannot
/// see the tree, and every attempt to have it re-derive the layout from a
/// couple of scalars is what produced the three-pane clipping bug.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const FULL: Rect = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
}

/// Outcome of a split request (drives the status message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAdd {
    Opened,
    Added,
    Full,
}

#[derive(Debug, Clone)]
pub struct SplitState {
    root: Layout,
    /// Every pane, in **visual order** (left→right, top→bottom). Derived from
    /// the tree by [`SplitState::resync_order`] after every structural change.
    pub panes: Vec<Pane>,
    focus: PaneId,
    /// After `Ctrl+W`, waiting for the chord.
    pub pending_chord: bool,
    next_id: u32,
    next_terminal_id: u32,
}

impl Default for SplitState {
    fn default() -> Self {
        let first = PaneId(1);
        Self {
            root: Layout::Leaf(first),
            panes: vec![Pane::new(first)],
            focus: first,
            pending_chord: false,
            next_id: 2,
            next_terminal_id: 1,
        }
    }
}

impl SplitState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn root(&self) -> &Layout {
        &self.root
    }

    pub fn is_split(&self) -> bool {
        self.panes.len() >= 2
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len().max(1)
    }

    // ---- focus -----------------------------------------------------------

    pub fn focus_id(&self) -> PaneId {
        self.focus
    }

    /// Focused pane's position in visual order — the index the compositor, the
    /// FFI and the face all speak.
    pub fn focus_index(&self) -> usize {
        self.panes
            .iter()
            .position(|p| p.id == self.focus)
            .unwrap_or(0)
    }

    pub fn focused_pane(&self) -> &Pane {
        let i = self.focus_index();
        &self.panes[i]
    }

    pub fn focused_pane_mut(&mut self) -> &mut Pane {
        let i = self.focus_index();
        &mut self.panes[i]
    }

    pub fn set_focus(&mut self, idx: usize) {
        if let Some(p) = self.panes.get(idx.min(self.panes.len().saturating_sub(1))) {
            self.focus = p.id;
        }
    }

    pub fn focus_other(&mut self) {
        if self.panes.len() < 2 {
            return;
        }
        let next = (self.focus_index() + 1) % self.panes.len();
        self.set_focus(next);
    }

    // ---- geometry --------------------------------------------------------

    /// Normalised rect per pane, in visual order — same order as `panes`.
    pub fn rects(&self) -> Vec<Rect> {
        let mut out = Vec::with_capacity(self.panes.len());
        Self::rects_into(&self.root, Rect::FULL, &mut out);
        out.into_iter().map(|(_, r)| r).collect()
    }

    fn rects_into(node: &Layout, r: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match node {
            Layout::Leaf(id) => out.push((*id, r)),
            Layout::Split { axis, children, weights } => {
                let total: f32 = weights.iter().sum();
                let mut off = 0.0f32;
                for (i, child) in children.iter().enumerate() {
                    let f = if total > 0.0 {
                        weights.get(i).copied().unwrap_or(0.0) / total
                    } else {
                        1.0 / children.len() as f32
                    };
                    let sub = match axis {
                        Axis::Col => Rect { x: r.x + off * r.w, y: r.y, w: f * r.w, h: r.h },
                        Axis::Row => Rect { x: r.x, y: r.y + off * r.h, w: r.w, h: f * r.h },
                    };
                    Self::rects_into(child, sub, out);
                    off += f;
                }
            }
        }
    }

    // ---- structure -------------------------------------------------------

    /// Split the focused pane along `axis`, putting the new pane after it.
    ///
    /// When the focused leaf already sits in a split of the same axis, the new
    /// pane joins that split as a sibling rather than nesting — so three
    /// "split right"s give three columns, not a column beside a column beside
    /// a column. Splitting the *other* way nests, which is what makes the
    /// four-pane `+` reachable.
    pub fn split_focused(&mut self, axis: Axis) -> SplitAdd {
        self.pending_chord = false;
        if self.panes.len() >= MAX_PANES {
            return SplitAdd::Full;
        }
        let was_split = self.is_split();
        let id = self.take_id();
        let target = self.focus;
        Self::insert_beside(&mut self.root, target, id, axis);

        let mut pane = Pane::new(id);
        if let Some(src) = self.panes.iter().find(|p| p.id == target) {
            // The new pane starts on the same document and viewport, VS Code
            // style; the user retargets it from there.
            pane.content = src.content;
            pane.scroll = src.scroll;
            pane.hscroll = src.hscroll;
            pane.cursor = src.cursor;
        }
        self.panes.push(pane);
        self.focus = id;
        self.resync_order();
        if was_split { SplitAdd::Added } else { SplitAdd::Opened }
    }

    /// Returns true if it found `target` and inserted next to it.
    fn insert_beside(node: &mut Layout, target: PaneId, fresh: PaneId, axis: Axis) -> bool {
        match node {
            Layout::Leaf(id) if *id == target => {
                // A bare leaf (or one whose parent runs the other way): wrap it.
                *node = Layout::Split {
                    axis,
                    children: vec![Layout::Leaf(target), Layout::Leaf(fresh)],
                    weights: vec![0.5, 0.5],
                };
                true
            }
            Layout::Leaf(_) => false,
            Layout::Split { axis: node_axis, children, weights } => {
                // Same axis and the target is a direct child → join as sibling,
                // taking half of the target's share.
                if *node_axis == axis {
                    if let Some(i) = children
                        .iter()
                        .position(|c| matches!(c, Layout::Leaf(id) if *id == target))
                    {
                        children.insert(i + 1, Layout::Leaf(fresh));
                        // Equal shares among the siblings, vim-style. Halving
                        // only the focused pane instead gives 1/2, 1/4, 1/8,
                        // 1/8 for four "split right"s, which is not what
                        // anyone means by splitting four ways.
                        weights.insert(i + 1, 0.0);
                        let n = children.len() as f32;
                        for w in weights.iter_mut() {
                            *w = 1.0 / n;
                        }
                        return true;
                    }
                }
                for child in children.iter_mut() {
                    if Self::insert_beside(child, target, fresh, axis) {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Remove the focused pane; focus lands on its neighbour.
    ///
    /// Returns the **survivor** that now has focus — the caller adopts its
    /// viewport when the split collapses to a single view.
    pub fn remove_focused(&mut self) -> Option<Pane> {
        if self.panes.len() < 2 {
            return None;
        }
        let gone = self.focus;
        let at = self.focus_index();
        Self::remove_leaf(&mut self.root, gone);
        if let Some(i) = self.panes.iter().position(|p| p.id == gone) {
            self.panes.remove(i);
        }
        self.resync_order();
        // Neighbour at the same visual slot, else the new last pane.
        let next = at.min(self.panes.len().saturating_sub(1));
        self.set_focus(next);
        self.panes.get(next).cloned()
    }

    /// Returns true when `node` itself should be dropped by the caller.
    fn remove_leaf(node: &mut Layout, gone: PaneId) -> bool {
        let left = match node {
            Layout::Leaf(id) => return *id == gone,
            Layout::Split { children, weights, .. } => {
                let mut hit = None;
                for (i, child) in children.iter_mut().enumerate() {
                    if Self::remove_leaf(child, gone) {
                        hit = Some(i);
                        break;
                    }
                }
                if let Some(i) = hit {
                    let freed = weights.remove(i);
                    children.remove(i);
                    // Hand the space to the siblings so the layout stays full.
                    if !weights.is_empty() {
                        let share = freed / weights.len() as f32;
                        for w in weights.iter_mut() {
                            *w += share;
                        }
                    }
                }
                children.len()
            }
        };
        // A split with one child is not a split any more — hoist the survivor,
        // or the tree keeps a node that draws a divider against nothing.
        if left == 1 {
            if let Layout::Split { children, .. } = node {
                let only = children.remove(0);
                *node = only;
            }
            return false;
        }
        left == 0
    }

    /// Collapse to a single pane, keeping the focused one's contents.
    pub fn close(&mut self) {
        let keep = self
            .panes
            .iter()
            .find(|p| p.id == self.focus)
            .cloned()
            .unwrap_or_else(|| Pane::new(self.focus));
        self.root = Layout::Leaf(keep.id);
        self.focus = keep.id;
        self.panes = vec![keep];
        self.pending_chord = false;
    }

    // ---- sizing ----------------------------------------------------------

    /// Drag the divider between the panes at visual indices `a` and `b` by
    /// `delta` — a fraction of the **whole editor** along the split's axis,
    /// which is what the face can measure.
    ///
    /// Returns false when those two panes do not share a divider.
    pub fn resize_between(&mut self, a: usize, b: usize, delta: f32) -> bool {
        let (Some(&ida), Some(&idb)) = (
            self.panes.get(a).map(|p| &p.id),
            self.panes.get(b).map(|p| &p.id),
        ) else {
            return false;
        };
        // The parent's extent along its axis scales `delta` from screen units
        // into that node's local weight space.
        let mut located = Vec::new();
        Self::rects_into(&self.root, Rect::FULL, &mut located);
        let extent = |id: PaneId, axis: Axis| -> f32 {
            located
                .iter()
                .find(|(pid, _)| *pid == id)
                .map(|(_, r)| match axis {
                    Axis::Col => r.w,
                    Axis::Row => r.h,
                })
                .unwrap_or(1.0)
        };
        Self::resize_in(&mut self.root, ida, idb, delta, &extent)
    }

    fn resize_in(
        node: &mut Layout,
        a: PaneId,
        b: PaneId,
        delta: f32,
        extent: &dyn Fn(PaneId, Axis) -> f32,
    ) -> bool {
        let Layout::Split { axis, children, weights } = node else {
            return false;
        };
        let axis = *axis;
        let ia = children.iter().position(|c| Self::contains(c, a));
        let ib = children.iter().position(|c| Self::contains(c, b));
        if let (Some(ia), Some(ib)) = (ia, ib) {
            if ia + 1 == ib {
                // `delta` is a fraction of the whole editor; the pair occupies
                // `span` of it, and shares `weights[ia] + weights[ib]`.
                let span = extent(a, axis) + extent(b, axis);
                if span <= 0.0 {
                    return false;
                }
                let pair = weights[ia] + weights[ib];
                let shift = (delta / span) * pair;
                let lo = pair * MIN_WEIGHT;
                let na = (weights[ia] + shift).clamp(lo, pair - lo);
                weights[ib] = pair - na;
                weights[ia] = na;
                return true;
            }
            if ia == ib {
                return Self::resize_in(&mut children[ia], a, b, delta, extent);
            }
            return false;
        }
        for child in children.iter_mut() {
            if Self::resize_in(child, a, b, delta, extent) {
                return true;
            }
        }
        false
    }

    fn contains(node: &Layout, id: PaneId) -> bool {
        match node {
            Layout::Leaf(l) => *l == id,
            Layout::Split { children, .. } => children.iter().any(|c| Self::contains(c, id)),
        }
    }

    /// Reset every divider to equal shares.
    pub fn equalize(&mut self) {
        Self::equalize_in(&mut self.root);
    }

    fn equalize_in(node: &mut Layout) {
        if let Layout::Split { children, weights, .. } = node {
            let n = children.len().max(1) as f32;
            for w in weights.iter_mut() {
                *w = 1.0 / n;
            }
            for c in children.iter_mut() {
                Self::equalize_in(c);
            }
        }
    }

    /// `Ctrl+W >` / `<` — nudge the focused pane's boundary with its next
    /// sibling, or its previous one when it is last.
    pub fn adjust_focused(&mut self, delta: f32) {
        let i = self.focus_index();
        if i + 1 < self.panes.len() && self.resize_between(i, i + 1, delta) {
            return;
        }
        if i > 0 {
            self.resize_between(i - 1, i, -delta);
        }
    }

    // ---- documents -------------------------------------------------------

    /// Repoint every pane showing `gone` at `adopt`, for when a document is
    /// closed out from under a split.
    pub fn repoint(&mut self, gone: BufferId, adopt: BufferId) {
        for p in &mut self.panes {
            if p.buffer() == gone {
                p.set_buffer(adopt);
                // Coordinates in a document that is no longer there.
                p.scroll = 0;
                p.hscroll = 0;
                p.cursor = (0, 0);
            }
        }
    }

    // ---- terminal placement ---------------------------------------------

    /// Visual index of the pane running the terminal, if any.
    ///
    /// This replaces `terminal.pane_bound` — and, with it, the hand-written
    /// patch-up (`Some(b - 1)`) that shifted the index when a lower pane
    /// closed. There is nothing to shift: the terminal is a property of a
    /// pane, and a pane that closes takes it along.
    pub fn terminal_pane(&self) -> Option<usize> {
        self.panes.iter().position(|p| p.is_terminal())
    }

    /// Every pane running a shell, in visual order.
    pub fn terminal_panes(&self) -> Vec<(usize, TerminalId)> {
        self.panes
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.terminal_id().map(|t| (i, t)))
            .collect()
    }

    /// Turn the pane at `idx` into a terminal with a fresh shell id,
    /// remembering the document it displaces. Returns `(shell, displaced)`.
    pub fn make_terminal(&mut self, idx: usize) -> Option<(TerminalId, BufferId)> {
        if self.panes.get(idx)?.is_terminal() {
            return None;
        }
        let id = TerminalId(self.next_terminal_id);
        self.next_terminal_id = self.next_terminal_id.wrapping_add(1);
        let p = self.panes.get_mut(idx)?;
        let restore = p.buffer();
        p.content = PaneContent::Terminal { id, restore };
        p.scroll = 0;
        p.hscroll = 0;
        p.cursor = (0, 0);
        Some((id, restore))
    }

    /// Put the pane at `idx` back to the document its terminal displaced.
    /// Returns `(shell that ended, document restored)`.
    pub fn clear_terminal_at(&mut self, idx: usize) -> Option<(TerminalId, BufferId)> {
        let p = self.panes.get_mut(idx)?;
        let id = p.terminal_id()?;
        let restore = p.buffer();
        p.content = PaneContent::Document(restore);
        p.scroll = 0;
        p.hscroll = 0;
        p.cursor = (0, 0);
        Some((id, restore))
    }

    // ---- internals -------------------------------------------------------

    fn take_id(&mut self) -> PaneId {
        let id = PaneId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Sort `panes` into the tree's visual order. The tree is the authority;
    /// the vector is a view of it that index-addressed callers can use.
    fn resync_order(&mut self) {
        let mut order = Vec::with_capacity(self.panes.len());
        Self::walk(&self.root, &mut order);
        self.panes.sort_by_key(|p| {
            order.iter().position(|id| *id == p.id).unwrap_or(usize::MAX)
        });
        self.panes.retain(|p| order.contains(&p.id));
        if !self.panes.iter().any(|p| p.id == self.focus) {
            if let Some(first) = self.panes.first() {
                self.focus = first.id;
            }
        }
    }

    fn walk(node: &Layout, out: &mut Vec<PaneId>) {
        match node {
            Layout::Leaf(id) => out.push(*id),
            Layout::Split { children, .. } => {
                for c in children {
                    Self::walk(c, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(s: &SplitState) -> Vec<u32> {
        s.panes.iter().map(|p| p.id.0).collect()
    }

    #[test]
    fn repeated_same_axis_splits_stay_flat_up_to_the_cap() {
        let mut s = SplitState::new();
        assert_eq!(s.split_focused(Axis::Col), SplitAdd::Opened);
        assert_eq!(s.pane_count(), 2);
        assert_eq!(s.split_focused(Axis::Col), SplitAdd::Added);
        assert_eq!(s.split_focused(Axis::Col), SplitAdd::Added);
        assert_eq!(s.pane_count(), 4);
        assert_eq!(s.split_focused(Axis::Col), SplitAdd::Full);

        // Four columns, equal, in visual order and spanning the full width.
        let r = s.rects();
        assert_eq!(r.len(), 4);
        for w in r.iter().map(|x| x.w) {
            assert!((w - 0.25).abs() < 1e-4, "expected quarters, got {w}");
        }
        assert!((r[0].x - 0.0).abs() < 1e-4);
        assert!((r[3].x + r[3].w - 1.0).abs() < 1e-4);
        for rect in &r {
            assert!((rect.h - 1.0).abs() < 1e-4, "columns keep full height");
        }
    }

    /// The whole point of the tree: a 2x2 `+`, which one `kind` could not
    /// express at all.
    #[test]
    fn split_right_then_below_twice_gives_the_four_pane_plus() {
        let mut s = SplitState::new();
        s.split_focused(Axis::Col); // A | B, focus B
        s.split_focused(Axis::Row); // A | (B / C), focus C
        s.set_focus(0); // back to A
        s.split_focused(Axis::Row); // (A / D) | (B / C)
        assert_eq!(s.pane_count(), 4);

        let r = s.rects();
        // Two columns of two, every pane a quarter of the area.
        for rect in &r {
            assert!((rect.w - 0.5).abs() < 1e-4, "half width, got {}", rect.w);
            assert!((rect.h - 0.5).abs() < 1e-4, "half height, got {}", rect.h);
        }
        let corners: Vec<(f32, f32)> = r.iter().map(|x| (x.x, x.y)).collect();
        for want in [(0.0, 0.0), (0.0, 0.5), (0.5, 0.0), (0.5, 0.5)] {
            assert!(
                corners.iter().any(|c| (c.0 - want.0).abs() < 1e-4
                    && (c.1 - want.1).abs() < 1e-4),
                "missing quadrant {want:?} in {corners:?}"
            );
        }
    }

    #[test]
    fn dividers_move_independently_with_three_panes() {
        let mut s = SplitState::new();
        s.split_focused(Axis::Col);
        s.split_focused(Axis::Col);
        assert_eq!(s.pane_count(), 3);

        // Drag only the first divider. The third pane must not move.
        let before = s.rects();
        assert!(s.resize_between(0, 1, 0.10));
        let after = s.rects();
        assert!(after[0].w > before[0].w + 0.05, "pane 0 grew");
        assert!(after[1].w < before[1].w - 0.05, "pane 1 gave the space");
        assert!(
            (after[2].w - before[2].w).abs() < 1e-4,
            "pane 2 is behind a different divider and must not move"
        );
        // And the layout still tiles the full width.
        let total: f32 = after.iter().map(|r| r.w).sum();
        assert!((total - 1.0).abs() < 1e-4, "panes must tile, got {total}");
    }

    #[test]
    fn a_divider_cannot_be_dragged_past_the_minimum() {
        let mut s = SplitState::new();
        s.split_focused(Axis::Col);
        s.resize_between(0, 1, -5.0);
        let r = s.rects();
        assert!(r[0].w >= MIN_WEIGHT - 1e-4, "pane 0 kept a floor: {}", r[0].w);
        assert!(r[1].w <= 1.0 - MIN_WEIGHT + 1e-4);
    }

    #[test]
    fn closing_a_pane_gives_its_space_to_the_survivors() {
        let mut s = SplitState::new();
        s.split_focused(Axis::Col);
        s.split_focused(Axis::Col);
        let all = ids(&s);
        s.set_focus(1);
        s.remove_focused();
        assert_eq!(s.pane_count(), 2);
        assert_eq!(ids(&s), vec![all[0], all[2]], "the right pane survived");
        let total: f32 = s.rects().iter().map(|r| r.w).sum();
        assert!((total - 1.0).abs() < 1e-4, "no gap left behind: {total}");
    }

    #[test]
    fn removing_the_nested_pane_collapses_its_split() {
        let mut s = SplitState::new();
        s.split_focused(Axis::Col); // A | B
        s.split_focused(Axis::Row); // A | (B / C), focus C
        assert_eq!(s.pane_count(), 3);
        s.remove_focused(); // drop C
        assert_eq!(s.pane_count(), 2);
        // B should be full height again — the Row split is gone, not left
        // wrapping a single child.
        let r = s.rects();
        for rect in &r {
            assert!((rect.h - 1.0).abs() < 1e-4, "expected full height, got {}", rect.h);
        }
    }

    #[test]
    fn focus_cycles_all_panes_in_visual_order() {
        let mut s = SplitState::new();
        s.split_focused(Axis::Col);
        s.split_focused(Axis::Col);
        s.set_focus(0);
        s.focus_other();
        assert_eq!(s.focus_index(), 1);
        s.focus_other();
        assert_eq!(s.focus_index(), 2);
        s.focus_other();
        assert_eq!(s.focus_index(), 0);
    }

    #[test]
    fn equalize_resets_every_divider() {
        let mut s = SplitState::new();
        s.split_focused(Axis::Col);
        s.split_focused(Axis::Col);
        s.resize_between(0, 1, 0.15);
        s.equalize();
        for r in s.rects() {
            assert!((r.w - 1.0 / 3.0).abs() < 1e-4, "got {}", r.w);
        }
    }
}
