import SwiftUI

/// List-row hover wash shared by navigator / git / palette rows.
/// iOS-quality: quick fade in, slightly slower fade out, no layout shift.
struct HoverRow<Content: View>: View {
    var corner: CGFloat = 6
    var tint: Color = .primary
    @ViewBuilder var content: () -> Content
    @State private var hovering = false

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
    /// bounding box centre (measured at 4x) — geometrically centred, it still
    /// reads as the hover capsule being shoved right. Nudge the glyph, never
    /// the capsule.
    var opticalNudgeX: CGFloat = 0
    var action: () -> Void
    @State private var hovering = false
    @State private var pressed = false

    var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: iconSize, weight: .medium))
                .foregroundStyle(active ? accent : (hovering ? dim.opacity(1) : dim.opacity(0.85)))
                .offset(x: opticalNudgeX)
                .frame(width: 28 + (iconSize - 13) * 2, height: 24 + (iconSize - 13) * 2)
                // Near-circular hover fill (Xcode 26 capsule language).
                .background(
                    Capsule(style: .continuous)
                        .fill(Color.primary.opacity(hovering || active ? 0.07 : 0))
                )
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
    var active: Bool
    var accent: Color
    var fg: Color
    var dim: Color
    var isLight: Bool
    var action: () -> Void
    var onClose: (() -> Void)? = nil

    @State private var hovering = false

    /// Show the trailing control (dirty or close) — always reserve space when either applies.
    private var showTrailing: Bool {
        onClose != nil || dirty
    }

    @State private var pressed = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Image(systemName: "doc.text.fill")
                    .font(.system(size: 10))
                    .foregroundStyle(active ? accent : dim.opacity(0.85))
                Text(title)
                    .font(.system(size: 12, weight: active ? .semibold : .regular))
                    .foregroundStyle(active ? fg : dim)
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
            .background(
                Capsule(style: .continuous)
                    .fill(
                        active
                            ? Color.primary.opacity(isLight ? 0.10 : 0.14)
                            : (hovering ? Color.primary.opacity(isLight ? 0.06 : 0.10) : Color.clear)
                    )
            )
            .scaleEffect(pressed ? 0.96 : 1)
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(.snappy(duration: 0.16), value: hovering)
        .animation(.snappy(duration: 0.20), value: active)
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
            // Dirty dot — visible only when dirty AND not hovering.
            Circle()
                .fill(Color(nsColor: .systemOrange))
                .frame(width: 6, height: 6)
                .opacity(dirty && !hovering ? 1 : 0)
                .scaleEffect(dirty && !hovering ? 1 : 0.55)

            // Close × — visible only while hovering (replaces dirty).
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
                .opacity(hovering ? 1 : 0)
                .scaleEffect(hovering ? 1 : 0.55)
                // Keep hit target only when visible
                .allowsHitTesting(hovering)
            }
        }
        .frame(width: 14, height: 14)
        .animation(.easeInOut(duration: 0.14), value: hovering)
    }
}
