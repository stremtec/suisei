import SwiftUI
import AppKit

// MARK: - Apple Liquid Glass (macOS 26)
// Balance: blur (regular) + light tint — not clear glass, not a solid slab.

enum SuiseiGlass {
    /// Floating panels (explorer / XLC / settings / SCM).
    /// Keep tint low so content still shows through the frost.
    static func panel(light: Bool) -> Glass {
        Glass.regular.tint(light ? Color.white.opacity(0.14) : Color.white.opacity(0.06))
    }

    /// Tab / status strips — same family, slightly quieter.
    static func chrome(light: Bool) -> Glass {
        Glass.regular.tint(light ? Color.white.opacity(0.10) : Color.white.opacity(0.05))
    }
}

struct GlassScrim: View {
    var lightChrome: Bool
    var body: some View {
        Color.black.opacity(lightChrome ? 0.10 : 0.36)
    }
}
