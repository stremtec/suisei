import SwiftUI
import AppKit

// MARK: - Apple Liquid Glass (macOS 26)
// Balance: blur (regular) + light tint — not clear glass, not a solid slab.

enum SuiseiGlassStyle: UInt8 {
    case clear = 0
    case tinted = 1
}

// Every tint here used to be WHITE — `panel` at 0.14 in light, on the widest
// surfaces in the app. Three functions down, `welcome` says why that is wrong:
// "a heavy white wash turns the surface into frosted plastic with no visible
// warp." The widest surfaces were carrying the heaviest wash.
//
// The Git workbench is the counter-example in the same app: sixteen surfaces,
// every one a system semantic colour or a system material, and exactly one
// glass — `.clear`, untinted. That is why the two windows read as different
// materials however their base colours are matched.
//
// So the tints are gone. `Glass.regular` alone is the platform's own balance of
// blur and lightening; anything added to it is this app disagreeing with macOS
// about what glass looks like.
enum SuiseiGlass {
    /// Floating panels (explorer / XLC / settings / SCM).
    static func panel(light: Bool, style: SuiseiGlassStyle) -> Glass {
        style == .clear ? .clear : .regular
    }

    /// Tab / status strips — same family.
    static func chrome(light: Bool, style: SuiseiGlassStyle) -> Glass {
        style == .clear ? .clear : .regular
    }

    /// Secondary surfaces that should recede (Welcome recents): the SAME
    /// regular family as `panel`. This used to be `Glass.clear` — but mixing
    /// clear and regular in one window breaks Apple's "never mix the variants"
    /// rule, and clear demands a dimming layer it never had.
    static func recessed(light: Bool, style: SuiseiGlassStyle) -> Glass {
        style == .clear ? .clear : .regular
    }

    /// Welcome launch card — the lens does the work.
    static func welcome(light: Bool, style: SuiseiGlassStyle) -> Glass {
        style == .clear ? .clear : .regular
    }
}

struct GlassScrim: View {
    var lightChrome: Bool
    var body: some View {
        Color.black.opacity(lightChrome ? 0.10 : 0.36)
    }
}
