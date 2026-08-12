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
            // Membership is recomputed HERE and only here. Parking is the
            // moment the arrangement stops changing, so it is the only moment a
            // snapshot of it is worth taking; while the layout is active the
            // live panes are read directly (`layout_docs`). Maintaining
            // `docs` alongside the panes by hand is what produced every defect
            // this feature has had.
            l.docs = crate::layout_tab::LayoutTab::docs_of(&panes);
            l.tree = tree;
            l.panes = panes;
        }
    }

    /// The documents a layout is showing right now.
    ///
    /// The live desk when it owns the screen, its parked snapshot otherwise.
    /// A pane split off after the fold is a member because it is a pane —
    /// there is no separate list for it to be missing from.
    pub(crate) fn layout_docs(&self, id: u64) -> Vec<BufferId> {
        if self.active_layout == Some(id) {
            return crate::layout_tab::LayoutTab::docs_of(&self.split.panes);
        }
        self.layouts
            .iter()
            .find(|l| l.id == id)
            .map(|l| l.docs.clone())
            .unwrap_or_default()
    }

    /// Whether `doc` is in `id`'s arrangement right now.
    pub(crate) fn layout_holds(&self, id: u64, doc: BufferId) -> bool {
        if doc == BufferId::default() {
            return false;
        }
        if self.active_layout == Some(id) {
            return self.split.panes.iter().any(|p| p.buffer == doc);
        }
        self.layouts
            .iter()
            .find(|l| l.id == id)
            .is_some_and(|l| l.docs.contains(&doc))
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
        let docs = crate::layout_tab::LayoutTab::docs_of(&panes);
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
        self.message = "Grouped into a layout tab · scroll up to unify · down to unfold".into();
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
    /// Keep the active layout's members contiguous in the strip.
    ///
    /// ORDER ONLY. This replaced `swap_focused_doc_in_active_layout`, which
    /// tried to keep a hand-written member list in step with the panes:
    /// on an open it looked up the displaced document in `docs` and wrote the
    /// new one over it. A pane split off after the fold was not in that list,
    /// so the lookup missed and the function returned having done nothing —
    /// the opened file never joined the layout and the group's idea of itself
    /// drifted further from the desk with every split.
    ///
    /// Membership is derived from the panes now, so nothing here can get it
    /// wrong. What is left is the strip's ORDER: the grouped shape draws one
    /// container from the first member's left edge to the last member's right,
    /// so the members have to sit together or it swallows a stranger. This is
    /// idempotent, and a missed call costs a container drawn too wide — never
    /// a wrong member list.
    pub(crate) fn regather_active_layout(&mut self) {
        let Some(id) = self.active_layout else { return };
        let docs = self.layout_docs(id);
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
        let id = self
            .layouts
            .iter()
            .map(|l| l.id)
            .find(|id| self.layout_holds(*id, doc))?;
        self.layouts.iter().find(|l| l.id == id)
    }
    /// Drop a closed document from every PARKED layout's member snapshot.
    ///
    /// The active layout needs nothing here — its members are its panes, and
    /// closing a tab retires the panes that showed it. Whether a layout is
    /// still an arrangement is a separate question, asked by
    /// [`App::dissolve_degenerate_layouts`] once the panes have settled.
    pub(crate) fn remove_doc_from_layouts(&mut self, doc: BufferId) {
        for l in &mut self.layouts {
            l.docs.retain(|d| *d != doc);
        }
    }

    /// Retire any layout that is no longer an arrangement.
    ///
    /// Fewer than two distinct documents is not an arrangement: the lone member
    /// returns to the strip as an ordinary tab. Left alive such a layout became
    /// a zombie — invisible in grouped style (no run of two to draw a container
    /// around) and unkillable in unified style (the chip's close went through
    /// the slot-clamped `close_tab` and killed the wrong document).
    ///
    /// Asked of what each layout SHOWS, so the active one is judged on its live
    /// panes. It used to be `docs.len() >= 2` against the stored snapshot, and
    /// a snapshot taken before a pane was split off is short by one: closing
    /// that pane took two down to one and dissolved a group with two panes
    /// still on screen. Call it after the panes have settled, never before —
    /// the arrangement has to be final for the question to mean anything.
    pub(crate) fn dissolve_degenerate_layouts(&mut self) {
        let doomed: Vec<u64> = self
            .layouts
            .iter()
            .map(|l| l.id)
            .filter(|id| self.layout_docs(*id).len() < 2)
            .collect();
        if doomed.is_empty() {
            return;
        }
        self.layouts.retain(|l| !doomed.contains(&l.id));
        if let Some(active) = self.active_layout {
            if doomed.contains(&active) {
                // An on-screen arrangement stays up — only its tab entry is
                // gone, exactly like an unfold.
                self.active_layout = None;
            }
        }
    }
}
