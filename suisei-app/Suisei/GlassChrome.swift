import SwiftUI

/// List-row hover wash shared by navigator / git / palette rows.
/// iOS-quality: quick fade in, slightly slower fade out, no layout shift.
struct HoverRow<Content: View>: View {
    @State private var hovering = false
    var corner: CGFloat = 6
    var tint: Color = .primary
    @ViewBuilder var content: () -> Content

    var body: some View {
        content()
            .background(
                RoundedRectangle(cornerRadius: corner, style: .continuous)
                    .fill(tint.opacity(hovering ? 0.07 : 0))
            )
            .onHover { hovering = $0 }
            .animation(
                hovering ? .easeOut(duration: 0.10) : .easeOut(duration: 0.22),
                value: hovering
            )
    }
}

/// Compact icon for unified titlebar (no floating chrome — Xcode-style flat tools).
struct ToolbarPlainIcon: View {
    var systemImage: String
    var help: String
    var active: Bool = false
    var accent: Color
    var dim: Color
    /// Icon point size — the sidebar toggle uses a larger one (it anchors the
    /// card's top row next to the traffic lights and read undersized at 13).
    var iconSize: CGFloat = 13
    /// Optical x-correction for asymmetric glyphs. `sidebar.left` carries its
    /// filled panel on the left, so its INK centroid sits ~0.7pt left of its
    /// bounding box centre (measured at 4x); `sidebar.right` is its mirror.
    ///
    /// The nudge moves the glyph AND its capsule together. Moving only the
    /// glyph — which is what this did — left the two disagreeing by exactly the
    /// nudge, and that reads as the capsule being shoved the other way: the
    /// reported "grey rounded background is slightly right" on the inspector
    /// toggle is its `-0.6`. Optically centring the pair inside its slot costs
    /// the row rhythm the same sub-point, which nothing can see.
    var opticalNudgeX: CGFloat = 0
    /// Per-symbol vertical optical correction. Positive values move the glyph
    /// and its hover capsule down while preserving the shared 28×24 hit box.
    var opticalNudgeY: CGFloat = 0
    var action: () -> Void
    @State private var hovering = false
    @State private var pressed = false

    var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: iconSize, weight: .medium))
                .foregroundStyle(active ? accent : (hovering ? dim.opacity(1) : dim.opacity(0.85)))
                // Glyphs receive optical size correction without changing
                // the shared hit box or the rhythm of neighboring actions.
                .frame(width: 28, height: 24)
                // Near-circular hover fill (Xcode 26 capsule language).
                .background(
                    Capsule(style: .continuous)
                        .fill(Color.primary.opacity(hovering || active ? 0.07 : 0))
                )
                // AFTER the capsule, so the two move as one.
                .offset(x: opticalNudgeX, y: opticalNudgeY)
                .scaleEffect(pressed ? 0.90 : 1)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
        .onHover { hovering = $0 }
        .animation(.snappy(duration: 0.15), value: hovering)
        .animation(.snappy(duration: 0.12), value: pressed)
        .simultaneousGesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in pressed = true }
                .onEnded { _ in pressed = false }
        )
    }
}

/// Document tab chip for titlebar principal area.
/// Trailing slot: dirty ● morphs into × on hover (single place — never both).
struct ToolbarTabChip: View {
    var title: String
    var dirty: Bool
    /// The file was deleted on disk — chip shows a warning glyph + struck-through
    /// title so editing a vanished path is obvious.
    var deleted: Bool = false
    var active: Bool
    var accent: Color
    var fg: Color
    var dim: Color
    var isLight: Bool
    /// This chip IS a folded layout (unified style), not a document.
    var isLayout: Bool = false
    /// Chip sits in a layout group (grouped members) or IS the layout chip.
    /// Drives the system-blue affordance + black ink so layouts read clearly.
    var inLayoutGroup: Bool = false
    /// Identity of this chip inside `pillSpace`.
    var tabId: Int
    /// Shared namespace for the active capsule, so it TRAVELS between chips
    /// instead of one fading out while another fades in.
    var pillSpace: Namespace.ID
    /// Hover comes from the strip's single hit test, not from this chip.
    var hovered: Bool = false
    var action: () -> Void
    var onClose: (() -> Void)? = nil


    /// Show the trailing control (dirty or close) — always reserve space when either applies.
    private var showTrailing: Bool {
        onClose != nil || dirty
    }

    @State private var pressed = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Image(systemName: deleted
                    ? "exclamationmark.triangle.fill"
                    : (isLayout ? "square.on.square" : "doc.text.fill"))
                    .font(.system(size: 10))
                    .foregroundStyle(deleted
                        ? Color(nsColor: .systemOrange)
                        : (active ? accent : dim.opacity(0.85)))
                Text(title)
                    .font(.system(size: 12, weight: active ? .semibold : .regular))
                    .foregroundStyle(active ? fg : dim)
                    .strikethrough(deleted, color: Color(nsColor: .systemOrange))
                    .lineLimit(1)

                if showTrailing {
                    trailingSlot
                }
            }
            .padding(.horizontal, 10)
            // Match ToolbarPlainIcon's 24pt box. The chip used to size itself
            // from its text (~22pt), so the tab sat on a different rhythm from
            // the icons beside it in the same row — small enough to look like a
            // mistake rather than a choice.
            .frame(height: 24)
            .background {
                // The hover fill belongs to THIS chip. The ACTIVE fill does
                // not live here at all — the strip draws a single capsule that
                // follows whichever chip is active.
                ZStack {
                    // The system-blue layout shape is drawn once by the strip,
                    // keyed to the layout group, and persists while grouped
                    // members are replaced by this unified chip. Drawing a
                    // second fill here made the destination pop in before the
                    // persistent shape had finished collapsing.
                    if isLayout {
                        Color.clear
                            .background(
                                AnimationTraceProbe(key: "layout-chip-\(tabId)")
                            )
                    }
                    if hovered, !active {
                        Capsule(style: .continuous)
                            .fill(
                                inLayoutGroup
                                    ? Color.black.opacity(0.08)
                                    : Color.primary.opacity(isLight ? 0.06 : 0.10)
                            )
                    }
                    // Invisible anchor the strip's capsule matches against.
                    Color.clear
                        .matchedGeometryEffect(id: tabId, in: pillSpace, isSource: true)
                }
            }
            .scaleEffect(pressed ? 0.96 : 1)
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
        .animation(.snappy(duration: 0.16), value: hovered)
        // NOT `value: active` — the travelling capsule is animated by the
        // strip, which is the only view that contains both the chip it leaves
        // and the chip it arrives at. Animating it here as well gave the shared
        // geometry two drivers.

        .animation(.snappy(duration: 0.12), value: pressed)
        .simultaneousGesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in pressed = true }
                .onEnded { _ in pressed = false }
        )
    }

    /// Fixed-size slot: dirty ● ⇄ × close. Hover (or only-close) swaps with a short morph.
    @ViewBuilder
    private var trailingSlot: some View {
        ZStack {
            // Dirty dot — visible only when dirty AND not hovered.
            Circle()
                .fill(Color(nsColor: .systemOrange))
                .frame(width: 6, height: 6)
                .opacity(dirty && !hovered ? 1 : 0)
                .scaleEffect(dirty && !hovered ? 1 : 0.55)

            // Close × — visible only while hovered (replaces dirty).
            if let onClose {
                Button {
                    onClose()
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(dim.opacity(0.90))
                        .frame(width: 14, height: 14)
                        .background(
                            Circle()
                                .fill(Color.primary.opacity(isLight ? 0.10 : 0.16))
                        )
                }
                .buttonStyle(.plain)
                .help("Close tab")
                .opacity(hovered ? 1 : 0)
                .scaleEffect(hovered ? 1 : 0.55)
                // Keep hit target only when visible
                .allowsHitTesting(hovered)
            }
        }
        .frame(width: 14, height: 14)
        .animation(.easeInOut(duration: 0.14), value: hovered)
    }
}
