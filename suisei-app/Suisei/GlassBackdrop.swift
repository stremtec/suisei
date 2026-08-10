import SwiftUI
import AppKit

// MARK: - Apple Liquid Glass (macOS 26)
// Balance: blur (regular) + light tint — not clear glass, not a solid slab.

enum SuiseiGlassStyle: UInt8 {
    case clear = 0
    case tinted = 1
}

enum SuiseiGlass {
    /// Floating panels (explorer / XLC / settings / SCM).
    /// Keep tint low so content still shows through the frost.
    static func panel(light: Bool, style: SuiseiGlassStyle) -> Glass {
        if style == .clear { return .clear }
        return Glass.regular.tint(light ? Color.white.opacity(0.14) : Color.white.opacity(0.06))
    }

    /// Tab / status strips — same family, slightly quieter.
    static func chrome(light: Bool, style: SuiseiGlassStyle) -> Glass {
        if style == .clear { return .clear }
        return Glass.regular.tint(light ? Color.white.opacity(0.10) : Color.white.opacity(0.05))
    }

    /// Secondary surfaces that should recede (Welcome recents): the SAME
    /// regular family as `panel`, just a quieter tint. This used to be
    /// `Glass.clear` — but mixing clear and regular in one window breaks
    /// Apple's "never mix the variants" rule, and clear demands a dimming
    /// layer it never had.
    static func recessed(light: Bool, style: SuiseiGlassStyle) -> Glass {
        if style == .clear { return .clear }
        return Glass.regular.tint(light ? Color.white.opacity(0.06) : Color.white.opacity(0.03))
    }

    /// Welcome launch card — stronger liquid refraction of the desktop.
    /// Minimal tint so the lens does most of the work; a heavy white wash
    /// turns the surface into frosted plastic with no visible warp.
    static func welcome(light: Bool, style: SuiseiGlassStyle) -> Glass {
        if style == .clear { return .clear }
        return Glass.regular.tint(light ? Color.white.opacity(0.04) : Color.white.opacity(0.02))
    }
}

struct GlassScrim: View {
    var lightChrome: Bool
    var body: some View {
        Color.black.opacity(lightChrome ? 0.10 : 0.36)
    }
}
