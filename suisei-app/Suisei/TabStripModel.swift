import CoreGraphics
import Foundation

/// What the tab strip is a view of.
///
/// See `docs/SUISEI-TAB-STRIP-BEHAVIOUR.md`. The rules in short:
///
/// - the strip is an ordered list of ENTRIES, and an entry is a document or a
///   layout;
/// - a layout's style (grouped / merged) changes how it DRAWS, never whether it
///   exists, so the entry list is invariant under a style toggle;
/// - selection is always the focused document — a merged layout chip carries
///   the highlight as a proxy for the member it hides;
/// - merged is opaque: members are not hit-testable while merged;
/// - hover and click resolve through the same lookup, so they cannot disagree.
///
/// The previous structure had none of these fixed, and had grown a branch per
/// guess. Chips vanished from the list on merge, which churned identity and
/// stranded measurements; `TabScene.id` carried buffer ids and layout ids in one
/// field; hover and click used different authorities and selected different
/// tabs.
enum TabEntry: Equatable {
    case document(Doc)
    case layout(Layout)

    struct Doc: Equatable {
        let stableId: UInt64
        /// Slot index — what every engine call takes.
        let slot: Int
        let title: String
        let dirty: Bool
        let deleted: Bool
        let isTerminal: Bool
    }

    struct Layout: Equatable {
        let id: UInt64
        /// Slot of the chip the engine addresses this layout by. For a merged
        /// layout that is its own chip; for a grouped one, its first member.
        let slot: Int
        let name: String
        let merged: Bool
        /// Always populated, in both styles — that is what keeps the entry list
        /// invariant. While merged they are simply not drawn or hit-tested.
        let members: [Doc]
    }

    /// Chips this entry contributes, left to right.
    var chips: [Doc] {
        switch self {
        case .document(let d):
            return [d]
        case .layout(let l):
            // Merged is OPAQUE: one chip standing for the arrangement, with the
            // members deliberately absent from the drawn and hit-testable set.
            return l.merged
                ? [Doc(stableId: l.id, slot: l.slot, title: l.name,
                       dirty: false, deleted: false, isTerminal: false)]
                : l.members
        }
    }

    var group: UInt64 {
        if case .layout(let l) = self { return l.id }
        return 0
    }
}

/// The entry list, and the questions the strip asks of it.
struct TabStripModel: Equatable {
    let entries: [TabEntry]

    /// Build from what the engine already publishes.
    ///
    /// No ABI change is needed: a grouped layout arrives as a run of tabs
    /// sharing a non-zero `group`, and a merged one as a single tab with
    /// `isLayout`. Both collapse to ONE `.layout` entry, which is exactly why
    /// the entry count survives a style toggle even though the tab count does
    /// not.
    init(tabs: [TabItem]) {
        var out: [TabEntry] = []
        var i = 0
        while i < tabs.count {
            let t = tabs[i]
            if t.isLayout {
                // Merged: one chip, its own id. Members are not on the strip,
                // so the entry records none — see `membersUnavailableWhileMerged`.
                out.append(.layout(.init(
                    id: t.stableId, slot: t.id, name: t.title,
                    merged: true, members: []
                )))
                i += 1
                continue
            }
            if t.group != 0 {
                // Grouped: consume the whole run that shares this group.
                let g = t.group
                var members: [TabEntry.Doc] = []
                while i < tabs.count, tabs[i].group == g, !tabs[i].isLayout {
                    members.append(Self.doc(tabs[i]))
                    i += 1
                }
                out.append(.layout(.init(
                    id: g, slot: members.first?.slot ?? t.id,
                    name: t.title, merged: false, members: members
                )))
                continue
            }
            out.append(.document(Self.doc(t)))
            i += 1
        }
        entries = out
    }

    private static func doc(_ t: TabItem) -> TabEntry.Doc {
        TabEntry.Doc(
            stableId: t.stableId, slot: t.id, title: t.title,
            dirty: t.dirty, deleted: t.deleted, isTerminal: t.isTerminal
        )
    }

    /// Every chip, left to right — what gets laid out and hit-tested.
    var chips: [TabEntry.Doc] { entries.flatMap(\.chips) }

    /// The entry a chip belongs to.
    func entry(forChip stableId: UInt64) -> TabEntry? {
        entries.first { $0.chips.contains { $0.stableId == stableId } }
    }

    /// Which entry carries the highlight, given the focused document.
    ///
    /// Selection is always the focused document (spec §2). A merged layout
    /// hides its member, so its chip stands in — a defined proxy, not a second
    /// selection concept.
    func highlightedChip(focusedDocument: UInt64, activeLayout: UInt64?) -> UInt64? {
        for entry in entries {
            switch entry {
            case .document(let d):
                if d.stableId == focusedDocument { return d.stableId }
            case .layout(let l):
                if l.merged {
                    // Its members are not listed while merged, so the layout
                    // being active is what tells us the focus is inside it.
                    if activeLayout == l.id { return l.id }
                } else if let m = l.members.first(where: { $0.stableId == focusedDocument }) {
                    return m.stableId
                }
            }
        }
        return nil
    }

    /// What a click on `stableId` means (spec §3).
    enum ClickAction: Equatable {
        /// Focus a document, leaving any active layout.
        case focusDocument(UInt64)
        /// Focus a member without disturbing the arrangement.
        case focusMember(UInt64, inLayout: UInt64)
        /// Restore a whole arrangement.
        case activateLayout(UInt64)
    }

    func click(chip stableId: UInt64) -> ClickAction? {
        guard let entry = entry(forChip: stableId) else { return nil }
        switch entry {
        case .document(let d):
            return .focusDocument(d.stableId)
        case .layout(let l):
            return l.merged
                ? .activateLayout(l.id)
                : .focusMember(stableId, inLayout: l.id)
        }
    }

    /// What a close (✕) on `stableId` means (spec §5).
    enum CloseAction: Equatable {
        /// Close one document.
        case closeDocument(UInt64)
        /// Drop a layout; its documents stay open as ordinary entries.
        case dropLayout(UInt64)
    }

    func close(chip stableId: UInt64) -> CloseAction? {
        guard let entry = entry(forChip: stableId) else { return nil }
        switch entry {
        case .document(let d):
            return .closeDocument(d.stableId)
        case .layout(let l):
            // Closing the merged chip drops the ARRANGEMENT; closing a member
            // closes that document. Same gesture, and in both cases it removes
            // the thing clicked rather than the things inside it.
            return l.merged ? .dropLayout(l.id) : .closeDocument(stableId)
        }
    }
}
