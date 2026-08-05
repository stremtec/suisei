import SwiftUI

/// Per-keystroke editor state, deliberately kept OFF `EngineBridge`'s
/// `@Published chrome` / `editorSplit`.
///
/// Why this exists: typing changes the caret and scroll every keystroke. When
/// those lived on `chrome`, publishing them re-evaluated the whole ContentView
/// tree — cheap unsplit (~0.04 ms) but ~20 ms once split, because SwiftUI
/// re-ran the split container's body for every pane on every key. The leaves
/// themselves (canvas `apply`, draw) were ~0.1 ms; the cost was purely the
/// tree walk (measured with `SUISEI_PERF=1`).
///
/// A SEPARATE `ObservableObject` fixes it: only the views that actually need a
/// per-keystroke update observe THIS (each pane's `EditorHost`). The split
/// container observes only the structural `editorSplit`, so it is skipped while
/// typing. A child object's changes do NOT propagate to the parent's
/// observers, which is exactly the isolation we want.
final class EditorTickStore: ObservableObject {
    /// Per-pane per-keystroke state, indexed by visual pane order (==
    /// `paneIndex`). Only what `EditorScrollView.apply` needs each keystroke:
    /// the clip position and the doc length (changes when lines are added).
    struct PaneTick: Equatable {
        var scroll: UInt32 = 0
        var hscroll: UInt32 = 0
        var docLineCount: UInt32 = 1
    }

    /// Paint generation — bumps whenever the editor content or caret moved, so
    /// each canvas repaints. This is `chrome.gen` for editor purposes, but on
    /// its own object so the bump does not drag the whole chrome tree with it.
    @Published var gen: UInt64 = 0
    /// Why Core moved the scroll (0 none / 1 restore / 2 navigate / 3 caret).
    @Published var scrollIntent: UInt8 = 0
    /// Sub-line residual for smooth momentum.
    @Published var scrollFrac: CGFloat = 0
    /// One entry per pane; unsplit is a single element.
    @Published var panes: [PaneTick] = [PaneTick()]

    /// Tick for `paneIndex`, falling back to the first pane (unsplit stores
    /// everything at index 0) then to a default.
    func tick(for paneIndex: Int) -> PaneTick {
        if paneIndex >= 0, paneIndex < panes.count { return panes[paneIndex] }
        return panes.first ?? PaneTick()
    }
}
