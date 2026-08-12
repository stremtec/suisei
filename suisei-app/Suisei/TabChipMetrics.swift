import AppKit
import CoreGraphics

/// How wide a tab chip is, derived from what `ToolbarTabChip` actually draws.
///
/// This is the one piece of `TabStripLayout` that needs a font, which is why
/// the computed-layout work lives face-side rather than in the engine.
///
/// The chip is:
///
///     HStack(spacing: 5) { icon; Text(title); trailing? }
///         .padding(.horizontal, 10)
///         .frame(height: 24)
///
/// so its width is `20 + icon + 5 + text (+ 5 + trailing)`. Two details are
/// easy to get wrong and both change the answer by enough to mis-aim a click:
///
/// - the ACTIVE chip's title is `.semibold`, so it is wider than the same
///   title inactive — the strip's widths depend on which tab is selected;
/// - `showTrailing` is `onClose != nil || dirty`, and the strip always passes
///   an `onClose`, so every chip in it reserves the trailing slot.
enum TabChipMetrics {
    // The box itself is `TabChipBox`, next to the hit test that reads it. This
    // file's job is the one part that needs a font: how wide the text inside
    // the box is. Restating the padding here is what let the chip, the width
    // and the close rect drift apart.

    /// Measured once per (symbol, size); SF Symbol widths vary per glyph.
    nonisolated(unsafe) private static var symbolWidths: [String: CGFloat] = [:]
    /// Measured once per (title, weight).
    nonisolated(unsafe) private static var titleWidths: [String: CGFloat] = [:]

    static func symbolWidth(_ name: String, size: CGFloat = 10) -> CGFloat {
        let key = "\(name)|\(size)"
        if let w = symbolWidths[key] { return w }
        let cfg = NSImage.SymbolConfiguration(pointSize: size, weight: .regular)
        let w = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(cfg)?
            .size.width ?? size
        symbolWidths[key] = w
        return w
    }

    static func titleWidth(_ title: String, active: Bool) -> CGFloat {
        let key = "\(active ? "b" : "r")|\(title)"
        if let w = titleWidths[key] { return w }
        let font = NSFont.systemFont(ofSize: 12, weight: active ? .semibold : .regular)
        let w = (title as NSString)
            .size(withAttributes: [.font: font])
            .width
        titleWidths[key] = w
        return w
    }

    /// Icon the chip shows, matching `ToolbarTabChip`'s own choice.
    static func symbolName(isLayout: Bool, deleted: Bool) -> String {
        if deleted { return "exclamationmark.triangle.fill" }
        return isLayout ? "square.on.square" : "doc.text.fill"
    }

    /// Total chip width.
    ///
    /// `showsTrailing` is true for every chip the tab strip builds; it is a
    /// parameter only so the arithmetic stays honest if that changes.
    static func width(
        title: String,
        active: Bool,
        isLayout: Bool,
        deleted: Bool,
        showsTrailing: Bool = true
    ) -> CGFloat {
        let icon = symbolWidth(symbolName(isLayout: isLayout, deleted: deleted))
        let text = titleWidth(title, active: active)
        var w = TabChipBox.horizontalPadding * 2
            + icon + TabChipBox.interItemGap + text
        if showsTrailing {
            w += TabChipBox.interItemGap + TabChipBox.trailingSlotWidth
        }
        // Rounded UP, per chip, because that is what SwiftUI does.
        //
        // This comment used to say the opposite — that rounding compounds, so
        // leave it fractional. Measured against a hosted `HStack(spacing: 4)`
        // of real chips, the reasoning was backwards: SwiftUI ceils every
        // child to a whole point, so `sum(ceil(wᵢ))` matches its layout
        // exactly at 1, 2, 5 and 10 chips while `sum(wᵢ)` falls behind by
        // ~0.36pt per chip — 3.6pt by the tenth tab, and growing. NOT
        // rounding is the thing that compounds.
        //
        // This matters more than it used to: `tabSlot` now computes chip
        // positions from these widths instead of measuring each chip, so a
        // systematic bias here aims every click and hover slightly wrong, and
        // worse the further right you go.
        return w.rounded(.up)
    }

    /// Drop the caches when the system font or appearance changes underneath
    /// them. Cheap to rebuild; wrong to keep.
    static func invalidate() {
        symbolWidths.removeAll(keepingCapacity: true)
        titleWidths.removeAll(keepingCapacity: true)
    }
}
