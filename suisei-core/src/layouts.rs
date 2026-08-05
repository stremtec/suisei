//! Layout tabs — fold/unfold/activate orchestration (A3-3 extraction).
//!
//! The arrangement's element types live in [`crate::layout_tab`]; the state
//! (`App::layouts`, `App::active_layout`) stays on `App` because the scene
//! reads it directly. This file is the domain logic: folding the desk into
//! a tab, unfolding it back, swapping membership as documents come and go.

use crate::app::{App, BufferId};

impl App {
    /// Write the on-screen arrangement back into its layout tab, so switching
    /// away and back is lossless.
    pub(crate) fn park_layout(&mut self, id: u64) {
        self.park_focused_pane();
        let (tree, panes) = self.split.snapshot();
        if let Some(l) = self.layouts.iter_mut().find(|l| l.id == id) {
            l.tree = tree;
            l.panes = panes;
        }
    }
    /// Fold the current arrangement into a layout tab.
    ///
    /// Deliberately quiet: the new layout tab becomes active, so **nothing on
    /// screen changes**. The visible effect is the next tab switch, which
    /// clears the editor down to that one document while the arrangement waits
    /// in its tab. That is the point of the feature — clearing the desk in one
    /// gesture without closing anything.
    ///
    /// Refuses when there is nothing to fold: a single pane is not an
    /// arrangement, and folding it would just hide a file behind a name.
    pub fn fold_layout(&mut self) -> bool {
        if self.active_layout.is_some() || !self.split.is_split() {
            return false;
        }
        self.park_focused_pane();
        let (tree, panes) = self.split.snapshot();
        let mut docs: Vec<BufferId> = Vec::new();
        for p in &panes {
            let b = p.buffer;
            if b != BufferId::default() && !docs.contains(&b) {
                docs.push(b);
            }
        }
        // The grouped presentation is a run of document chips. Two panes that
        // show the same document still produce only one chip, so accepting
        // that split creates an active one-member "group" whose container is
        // deliberately not drawn. It looks exactly like the group vanished.
        // Refuse an arrangement the strip cannot represent honestly.
        if docs.len() < 2 {
            self.message = "A layout needs at least two different documents".into();
            return false;
        }
        // Strip order: gather the folded documents into one contiguous run, so
        // the grouped container draws around its members alone and never
        // swallows an unrelated tab that happens to sit between two of them
        // (folding panes on 1·2·3·5 while 4 sits in the strip used to pull 4
        // inside the grey round). Panes address documents by id, so this
        // changes only the order the strip draws, never what a pane shows.
        self.gather_folded_docs(&docs);
        let id = self.take_tab_id().0;
        let name = crate::layout_tab::next_name(&self.layouts);
        self.layouts.push(crate::layout_tab::LayoutTab {
            id,
            name,
            tree,
            panes,
            docs,
            style: crate::layout_tab::LayoutStyle::Grouped,
        });
        self.active_layout = Some(id);
        self.message =
            "Grouped into a layout tab · scroll up to unify · down to unfold".into();
        true
    }
    /// Reorder `buffers` so the folded documents form one contiguous run,
    /// sitting where the first of them already was. Called by [`App::fold_layout`].
    ///
    /// The grouped strip shape draws one rounded container from the first
    /// member's left edge to the last member's right edge. If an unrelated tab
    /// sits between two members, that container swallows it visually. Gathering
    /// the members first makes the container exact. Panes address documents by
    /// [`BufferId`], so this touches only the order the strip draws, never what
    /// a pane shows.
    fn gather_folded_docs(&mut self, docs: &[BufferId]) {
        let n = self.tabs.buffers.len();
        if docs.len() < 2 || n < 2 {
            return;
        }
        let Some(anchor) = self.tabs.buffers.iter().position(|t| docs.contains(&t.id)) else {
            return;
        };
        // Already one contiguous run — nothing to gather.
        if anchor + docs.len() <= n
            && self.tabs.buffers[anchor..anchor + docs.len()]
                .iter()
                .all(|t| docs.contains(&t.id))
        {
            return;
        }
        // `partition` keeps each side's relative order, so the members keep
        // their existing strip order among themselves.
        let (members, mut others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tabs.buffers)
            .into_iter()
            .partition(|t| docs.contains(&t.id));
        let at = anchor.min(others.len());
        others.splice(at..at, members);
        self.tabs.buffers = others;
        // The active tab is derived from the focused pane's document id, and
        // reordering the vector changes no id — the position follows by
        // itself. No save/restore round-trip, no index to repoint.
    }
    /// Keep the active layout's membership honest after the focused pane's
    /// document changed from `replacing` to `opened`.
    ///
    /// Opening a file replaces what the focused pane shows. If that pane was
    /// part of a folded layout, the layout's member list has to follow: the
    /// displaced document leaves, the opened one takes its place. Without this
    /// the new file appears as a loose chip outside the group while the
    /// displaced one lingers inside it, shown by no pane.
    pub(crate) fn swap_focused_doc_in_active_layout(
        &mut self,
        replacing: BufferId,
        opened: BufferId,
    ) {
        let Some(id) = self.active_layout else { return };
        if replacing == opened {
            return;
        }
        let Some(l) = self.layouts.iter_mut().find(|l| l.id == id) else {
            return;
        };
        // Already a member (opening a document another pane of this layout
        // shows) — membership is unchanged, only focus moves.
        if l.docs.contains(&opened) {
            return;
        }
        let Some(i) = l.docs.iter().position(|d| *d == replacing) else {
            return;
        };
        l.docs[i] = opened;
        // The new document sits at the END of `buffers` (just pushed), while
        // the displaced one sat inside the group's run — so the members are no
        // longer contiguous and the grey container would swallow whatever sits
        // between them. Re-gather, exactly as `fold_layout` does.
        let docs = l.docs.clone();
        self.gather_folded_docs(&docs);
    }
    /// Unfold the **active** layout: its documents return to the strip as
    /// individual tabs and the arrangement stays exactly as it is on screen.
    ///
    /// Bound to the active layout rather than the one under the pointer. A
    /// layout that detonates because the pointer was passing over it on the
    /// way to another tab is worse than no unfold at all.
    pub fn unfold_layout(&mut self) -> bool {
        let Some(id) = self.active_layout else {
            return false;
        };
        let Some(i) = self.layouts.iter().position(|l| l.id == id) else {
            self.active_layout = None;
            return false;
        };
        self.layouts.remove(i);
        self.active_layout = None;
        self.message = "Layout unfolded".into();
        true
    }
    /// Drop a layout tab by id — what "Close Tab" on a layout chip means. Its
    /// documents stay open as loose tabs and an arrangement on screen stays
    /// up; only the strip entry goes. `unfold_layout` does the same thing but
    /// is bound to the ACTIVE layout — this one names its target, so a chip
    /// can be closed while another arrangement owns the screen.
    pub fn drop_layout(&mut self, id: u64) -> bool {
        let Some(i) = self.layouts.iter().position(|l| l.id == id) else {
            return false;
        };
        self.layouts.remove(i);
        if self.active_layout == Some(id) {
            self.active_layout = None;
        }
        self.message = "Layout unfolded".into();
        true
    }
    /// Show a layout: install its tree, exactly as it was parked.
    ///
    /// `focus_doc` names the document the caller wants focused — a grouped
    /// chip click carries the document it represents, so the arrangement comes
    /// back with that pane in front rather than always the first one in the
    /// tree. `None` keeps the tree's own order (unified chip, programmatic).
    pub fn activate_layout(&mut self, id: u64, focus_doc: Option<BufferId>) -> bool {
        let Some(l) = self.layouts.iter().find(|l| l.id == id) else {
            return false;
        };
        let (tree, panes) = (l.tree.clone(), l.panes.clone());
        self.park_focused_pane();
        self.save_state_to_tab();
        // Panes carry their own document and viewport, so restoring the tree
        // and its panes restores the whole arrangement — including where each
        // pane was scrolled to.
        self.split.restore(tree, panes);
        self.active_layout = Some(id);
        if let Some(doc) = focus_doc {
            if let Some(idx) = self.split.panes.iter().position(|p| p.buffer == doc) {
                self.split.set_focus(idx);
            }
        }
        self.load_focused_pane();
        true
    }
    /// Switch a layout between its two strip shapes.
    pub fn toggle_layout_style(&mut self, id: u64) -> bool {
        if let Some(l) = self.layouts.iter_mut().find(|l| l.id == id) {
            l.style = l.style.toggled();
            self.message = match l.style {
                crate::layout_tab::LayoutStyle::Grouped => {
                    "Layout group expanded · scroll up to unify · down to unfold".into()
                }
                crate::layout_tab::LayoutStyle::Unified => {
                    "Layout unified · scroll down to show member tabs".into()
                }
            };
            return true;
        }
        false
    }
    /// Whether `doc` is folded into some layout — folded documents do not get
    /// their own chip unless their layout is drawn grouped.
    pub fn layout_holding(&self, doc: BufferId) -> Option<&crate::layout_tab::LayoutTab> {
        self.layouts.iter().find(|l| l.holds(doc))
    }
    /// Remove a closed document from any layout's membership. Called by
    /// `close_current_tab` / `close_tab_at` so the group never references a
    /// document that no longer exists.
    pub(crate) fn remove_doc_from_layouts(&mut self, doc: BufferId) {
        for l in &mut self.layouts {
            l.docs.retain(|d| *d != doc);
        }
        // A layout with fewer than two documents is no longer an arrangement:
        // its lone member (if any) returns to the strip as an ordinary tab.
        // Left alive it became a zombie — invisible in grouped style (no run
        // of two to draw a container around) and unkillable in unified style
        // (the chip's close went through the slot-clamped `close_tab` and
        // killed the wrong document).
        let before = self.layouts.len();
        self.layouts.retain(|l| l.docs.len() >= 2);
        if self.layouts.len() != before {
            if let Some(active) = self.active_layout {
                if !self.layouts.iter().any(|l| l.id == active) {
                    // An on-screen arrangement stays up — only its tab entry
                    // is gone, exactly like an unfold.
                    self.active_layout = None;
                }
            }
        }
    }
}
