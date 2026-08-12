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
