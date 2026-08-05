//! Layout tabs — the editor's whole arrangement, folded into the tab strip.
//!
//! The gesture, as specified: whatever the editor is showing — one file, a 2-
//! or 3-way split, or the four-pane `+` — each quick upward scroll over the
//! tab strip advances one stage: loose split → grouped layout → unified
//! layout. Each downward scroll reverses one stage. Switching to another tab
//! clears the editor down to that one document, because the arrangement is
//! safely in its layout tab.
//!
//! A layout is not a separate bar to switch between. It is a tab, and it can
//! wear one of two shapes in the strip — see [`LayoutStyle`].

use crate::app::BufferId;
use crate::split::Layout;

/// How a folded layout presents itself in the tab strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutStyle {
    /// Its documents keep their own chips, drawn together inside one rounded
    /// grey container. You can still see *what* is in there.
    Grouped,
    /// One chip carrying the layout's name. Tidier; says nothing about the
    /// contents.
    Unified,
}

impl LayoutStyle {
    pub fn toggled(self) -> Self {
        match self {
            LayoutStyle::Grouped => LayoutStyle::Unified,
            LayoutStyle::Unified => LayoutStyle::Grouped,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayoutTab {
    /// Shares the tab-id space so the face can key chips on it without
    /// wondering which kind it has.
    pub id: u64,
    pub name: String,
    /// Exactly S3's tree — it already carries a `BufferId` per leaf plus each
    /// pane's scroll and cursor, so a layout tab needs no state of its own.
    /// That is the argument for building this on top of S3 rather than
    /// alongside it.
    pub tree: Layout,
    /// The panes the tree names, exactly as they were when folded — each
    /// carrying its document, scroll and cursor. Parked with the tree because
    /// they were snapshotted together; restoring is a lookup, not a rebuild.
    pub panes: Vec<crate::split::Pane>,
    /// The documents this layout folded, in visual order. Kept separately
    /// because the grouped style draws a chip per document and the tree's
    /// leaves are not ordered for reading.
    pub docs: Vec<BufferId>,
    pub style: LayoutStyle,
}

impl LayoutTab {
    /// Whether `doc` is folded into this layout.
    pub fn holds(&self, doc: BufferId) -> bool {
        self.docs.contains(&doc)
    }
}

/// Next free "layout N" name.
///
/// Lowest unused number, so a fresh fold never takes the name of a live
/// layout — recycling names is how two tabs end up both called "layout 2".
pub fn next_name(existing: &[LayoutTab]) -> String {
    for n in 1.. {
        let candidate = format!("layout {n}");
        if !existing.iter().any(|l| l.name == candidate) {
            return candidate;
        }
    }
    unreachable!("1.. is not exhaustible")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::PaneId;

    fn tab(name: &str) -> LayoutTab {
        LayoutTab {
            id: 1,
            name: name.into(),
            tree: Layout::Leaf(PaneId(1)),
            panes: Vec::new(),
            docs: Vec::new(),
            style: LayoutStyle::Grouped,
        }
    }

    #[test]
    fn names_fill_the_lowest_free_slot() {
        assert_eq!(next_name(&[]), "layout 1");
        let one = vec![tab("layout 1")];
        assert_eq!(next_name(&one), "layout 2");
        // "layout 1" was unfolded — its name is free again, and taking it is
        // right. Taking "layout 3" here would leave a gap forever.
        let two = vec![tab("layout 2")];
        assert_eq!(next_name(&two), "layout 1");
    }

    #[test]
    fn style_toggles_both_ways() {
        assert_eq!(LayoutStyle::Grouped.toggled(), LayoutStyle::Unified);
        assert_eq!(LayoutStyle::Unified.toggled(), LayoutStyle::Grouped);
    }
}
