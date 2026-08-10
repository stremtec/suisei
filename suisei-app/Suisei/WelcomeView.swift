import SwiftUI
import AppKit
import QuartzCore
import CoreText

/// One labelled step of launch-time warmup. `label` is what the user reads
/// while `run` executes — the After Effects "Loading Paint Palette…" rhythm.
/// The work is light today; the value is the SEAM: heavy precomputation
/// (project index, LSP warmup, syntax preparse, glyph atlas) attaches here so
/// it runs during the launch splash and never stalls a keystroke later.
struct BootStage: Identifiable {
    let id = UUID()
    let label: String
    let run: () async -> Void
    init(label: String, run: @escaping () async -> Void = {}) {
        self.label = label
        self.run = run
    }
}

/// Launch card in the Adobe After Effects splash rhythm:
/// - Fixed continuous-rounded window
/// - **Left** dark control column: brand, actions, recents
/// - **Right** full-bleed space hero art (no list clutter)
/// - Top-leading dismiss (not traffic lights)
struct WelcomeView: View {
    var onCreate: () -> Void
    var onOpen: () -> Void
    var onClone: () -> Void
    var onOpenRecent: (String) -> Void
    var onClose: () -> Void
    var recents: [RecentItem]
    /// Ordered launch-time warmup steps. Each step's `label` is what the user
    /// reads while it runs (the After Effects "Loading Paint Palette…" rhythm);
    /// its `run` does the real work. The launch actions stay hidden behind this
    /// sequence, so heavy first-run work (project index, LSP warmup, git,
    /// syntax preparse) can never make the window look frozen — the card and
    /// wordmark are already on screen. Empty → only the minimum-splash floor
    /// applies (today's near-instant boot). Stages migrate into the Core boot
    /// pipeline over time; their labels stay stable here so the sequence reads
    /// the same while the work moves down.
    var bootStages: [BootStage] = []

    /// Slightly wider than the old Xcode sheet so the art panel can breathe.
    static let windowSize = NSSize(width: 860, height: 500)
    /// Control column share — art gets the majority (AE-style).
    static let controlSplit: CGFloat = 0.40
    /// Welcome is borderless, so it cuts its own corner — and it has to match a
    /// real window sitting next to it. Same source as every other surface that
    /// lines up with a window edge.
    static let cornerRadius: CGFloat = WindowChrome.windowCornerRadius

    @Environment(\.colorScheme) private var scheme

    /// Near-black control rail — solid on purpose so the art reads as the
    /// only “hero” surface (glass on both sides made the card muddy).
    private var controlBg: Color {
        scheme == .dark
            ? Color(red: 0.06, green: 0.06, blue: 0.07)
            : Color(red: 0.08, green: 0.08, blue: 0.09)
    }
    private let label = Color.white.opacity(0.92)
    private let muted = Color.white.opacity(0.48)
    private let hairline = Color.white.opacity(0.08)

    @State private var appeared = false
    /// Boot has warmed enough to reveal the launch actions. Until then the
    /// control column shows the loading sequence, not the buttons/recents.
    @State private var ready = false
    /// Label of the boot stage currently running (drives the AE-style status
    /// line). Empty between stages / once ready.
    @State private var bootLabel = ""
    @State private var expandedRecents: Set<String> = []
    @State private var artHovering = false
    /// Picked once per launch from the WelcomeHeroes pool (00…n).
    @State private var heroPick: WelcomeHeroPick = .pickForThisLaunch()

    var body: some View {
        ZStack(alignment: .topLeading) {
            GeometryReader { geo in
                let artW = geo.size.width * (1 - Self.controlSplit)
                HStack(spacing: 0) {
                    controlColumn
                        .frame(width: geo.size.width * Self.controlSplit, height: geo.size.height)
                        .background(controlBg)

                    artPanel
                        .frame(width: artW, height: geo.size.height)
                        // Soft bleed of the control rail into the art so the
                        // seam is a dissolve, not a hard cut.
                        .overlay(alignment: .leading) {
                            LinearGradient(
                                colors: [
                                    controlBg.opacity(0.95),
                                    controlBg.opacity(0.35),
                                    .clear,
                                ],
                                startPoint: .leading,
                                endPoint: .trailing
                            )
                            .frame(width: 48)
                            .allowsHitTesting(false)
                        }
                }
            }

            Button(role: .cancel, action: onClose) {
                Image(systemName: "xmark.circle.fill")
                    .symbolRenderingMode(.hierarchical)
                    .font(.system(size: 14, weight: .regular))
                    .foregroundStyle(.white.opacity(0.55))
            }
            .buttonStyle(.plain)
            .help("Close")
            .accessibilityLabel("Close")
            .padding(.leading, 14)
            .padding(.top, 14)

            // Caption lives on the CARD, not inside the art ZStack — that
            // stack's bounds + the continuous corner clip kept eating the
            // trailing edge ("Ca…" / "NGC 33…"). Inset past the corner arc
            // with a dark pill so the credit always reads on any nebula.
            heroCaption
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
                .padding(.trailing, Self.cornerRadius + 16)
                .padding(.bottom, Self.cornerRadius + 14)
                .allowsHitTesting(false)
        }
        .frame(width: Self.windowSize.width, height: Self.windowSize.height)
        .clipShape(RoundedRectangle(cornerRadius: Self.cornerRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Self.cornerRadius, style: .continuous)
                .strokeBorder(Color.white.opacity(0.08), lineWidth: 1)
        )
        .shadow(color: .black.opacity(0.45), radius: 28, y: 14)
        .gesture(WindowDragGesture())
        .background(WelcomeWindowChrome(cornerRadius: Self.cornerRadius))
        .onAppear {
            withAnimation(.smooth(duration: 0.45)) { appeared = true }
        }
        // Run the launch warmup off the card's first frame. The window, wordmark
        // and hero are already painting; this only gates the actions/recents.
        .task {
            // The floor runs CONCURRENTLY with the stages, so total hidden time
            // is max(stage work, floor) — never their sum. It exists so an
            // instant boot still reveals with a deliberate beat, not a flash.
            async let floor: Void = Self.minimumSplash()
            for stage in bootStages {
                withAnimation(.easeInOut(duration: 0.2)) { bootLabel = stage.label }
                // max(work, dwell): the label never flashes faster than it can
                // be read, and once real work outgrows the dwell the dwell
                // vanishes into it — same principle as the overall floor.
                async let dwell: Void = Self.stageDwell()
                await stage.run()
                await dwell
            }
            await floor
            bootLabel = ""
            withAnimation(.smooth(duration: 0.55)) { ready = true }
        }
    }

    /// Minimum time the launch actions stay behind the loading state, so the
    /// reveal always reads as an intentional transition instead of a flash —
    /// even when the stages finish instantly (as they mostly do today).
    private static func minimumSplash() async {
        try? await Task.sleep(nanoseconds: 650_000_000)
    }

    /// Legibility floor for a single stage's status label.
    private static func stageDwell() async {
        try? await Task.sleep(nanoseconds: 240_000_000)
    }

    /// Object name + short credit, drawn on the image itself — no plate.
    /// Soft shadow keeps it readable on bright nebulæ without a fill.
    private var heroCaption: some View {
        VStack(alignment: .trailing, spacing: 3) {
            Text(heroPick.title)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.white.opacity(0.94))
                .lineLimit(1)
            Text(heroPick.subtitle)
                .font(.system(size: 11, weight: .regular))
                .foregroundStyle(.white.opacity(0.70))
                .lineLimit(2)
                .multilineTextAlignment(.trailing)
        }
        .shadow(color: .black.opacity(0.75), radius: 6, y: 1)
        .shadow(color: .black.opacity(0.45), radius: 1, y: 0)
        .opacity(appeared ? 1 : 0)
        .animation(.smooth(duration: 0.4), value: appeared)
    }

    // MARK: - Left control column

    private var controlColumn: some View {
        VStack(alignment: .leading, spacing: 0) {
            Spacer().frame(height: 40)

            // Type-only lockup — Gondens (tall condensed display). DEMO only
            // ships A–Z/a–z, so the version line stays on the system face.
            VStack(alignment: .leading, spacing: 0) {
                Text("Suisei")
                    .font(brandWordmarkFont)
                    .foregroundStyle(Color.white)
                    // Condensed faces want a hair of positive tracking so
                    // stems do not fuse at display size.
                    .tracking(0.4)
                // Tall fonts overshoot the line box; clip nothing, just
                // reclaim the excess leading below the wordmark.
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                VStack(alignment: .leading, spacing: 3) {
                    Text("© 2025–2026 Stemtec. All rights reserved.")
                        .font(.system(size: 10, weight: .regular, design: .default))
                        .foregroundStyle(muted.opacity(0.90))
                    Text("Suisei 2026dev · Legal Information")
                        .font(.system(size: 10, weight: .regular, design: .default))
                        .foregroundStyle(muted.opacity(0.75))
                }
                .padding(.top, 10)
            }
            .padding(.leading, 22)
            .padding(.trailing, 20)
            .opacity(appeared ? 1 : 0)
            .offset(y: appeared ? 0 : 6)

            Spacer().frame(height: 28)

            // The actions/recents wait behind the boot sequence; the loading
            // line occupies the same region until `ready`, then the actions
            // rise into place. Both live in one ZStack so the reveal is a
            // cross-fade in a fixed footprint, not a layout jump.
            ZStack(alignment: .top) {
                bootLoadingView
                    .opacity(ready ? 0 : 1)
                    .allowsHitTesting(!ready)

                actionsAndRecents
                    .opacity(ready ? 1 : 0)
                    .offset(y: ready ? 0 : 10)
                    .allowsHitTesting(ready)
            }
            .frame(maxHeight: .infinity, alignment: .top)
            .animation(.smooth(duration: 0.55), value: ready)

            Spacer(minLength: 12)
        }
    }

    /// Launch actions + recents — revealed together once boot is `ready`.
    private var actionsAndRecents: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(spacing: 8) {
                welcomeButton(systemImage: "plus.square", title: "Create New File…", action: onCreate)
                welcomeButton(systemImage: "square.and.arrow.down.on.square", title: "Clone Git Repository…", action: onClone)
                welcomeButton(systemImage: "folder", title: "Open Existing Project…", action: onOpen)
            }
            .padding(.horizontal, 22)

            Spacer().frame(height: 22)
            Rectangle()
                .fill(hairline)
                .frame(height: 1)
                .padding(.horizontal, 22)

            // Recents live under the actions so the art panel stays pure.
            recentsSection
                .padding(.top, 14)
                .frame(maxHeight: .infinity, alignment: .top)
        }
    }

    /// The After Effects-style status line: a spinner plus the current boot
    /// stage's label, swapped with a cross-fade as stages advance.
    private var bootLoadingView: some View {
        HStack(spacing: 10) {
            ProgressView()
                .controlSize(.small)
                .tint(.white.opacity(0.55))
            Text(bootLabel.isEmpty ? "Loading…" : bootLabel)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(muted)
                .lineLimit(1)
                .contentTransition(.opacity)
                .animation(.easeInOut(duration: 0.2), value: bootLabel)
        }
        .padding(.horizontal, 22)
        .padding(.top, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Tall condensed display face (Gondens). Registered from
    /// `Resources/Fonts` via `ATSApplicationFontsPath` + a runtime register
    /// so first launch before the system font cache settles still works.
    private var brandWordmarkFont: Font {
        WelcomeFonts.registerIfNeeded()
        // PostScript name from the OTF; family name as fallback.
        if let face = NSFont(name: "GondensDEMO-Regular", size: 44)
            ?? NSFont(name: "Gondens DEMO", size: 44)
            ?? NSFont(name: "Gondens DEMO Regular", size: 44)
        {
            return Font(face)
        }
        // System stand-in: condensed black is the closest built-in tall stack.
        if let cond = NSFont(name: "AvenirNextCondensed-Heavy", size: 42)
            ?? NSFont(name: "Avenir Next Condensed Heavy", size: 42)
        {
            return Font(cond)
        }
        return .system(size: 42, weight: .black, design: .default)
    }

    // MARK: - Right art panel

    private var artPanel: some View {
        // Fill the panel with the hero, then CLIP to the panel's own frame.
        //
        // `.scaledToFill()` sizes the image's VIEW to its aspect-filled size,
        // which for a wide hero is wider than the panel. A `.background` does
        // NOT clip a child that reports its own oversized frame — and
        // `.clipped()` on that child clips to the *oversized* frame, a no-op —
        // so the overflow spilled left, over the control column. It was
        // intermittent only because each launch picks a different hero: a
        // near-square one barely overflowed, a wide one like M51 a lot.
        //
        // Clipping the CONTAINER instead bounds it for every hero: the base
        // Rectangle establishes the panel's frame, the image overlays and
        // overflows it, and the trailing `.clipped()` cuts everything back to
        // that frame.
        Rectangle()
            .fill(Color.black)
            .overlay {
                heroImage
                    .scaleEffect(artHovering ? 1.018 : (appeared ? 1.0 : 1.04))
                    .animation(.smooth(duration: 0.55), value: artHovering)
                    .animation(.smooth(duration: 1.1), value: appeared)
            }
            .overlay {
                LinearGradient(
                    colors: [.clear, .black.opacity(0.35)],
                    startPoint: .center,
                    endPoint: .bottom
                )
            }
            .clipped()
            .contentShape(Rectangle())
            .onHover { artHovering = $0 }
    }

    @ViewBuilder
    private var heroImage: some View {
        if let ns = heroPick.image {
            Image(nsImage: ns)
                .resizable()
                .interpolation(.high)
                .scaledToFill()
                .id(heroPick.id)
        } else {
            // Fallback if the asset pool is empty.
            LinearGradient(
                colors: [
                    Color(red: 0.05, green: 0.06, blue: 0.14),
                    Color(red: 0.12, green: 0.08, blue: 0.22),
                    Color.black,
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            .overlay {
                RadialGradient(
                    colors: [
                        Color(red: 0.35, green: 0.45, blue: 0.95).opacity(0.45),
                        Color(red: 0.55, green: 0.25, blue: 0.85).opacity(0.25),
                        .clear,
                    ],
                    center: .center,
                    startRadius: 10,
                    endRadius: 220
                )
            }
        }
    }

    // MARK: - Recents (compact, under actions)

    private var recentFolders: [RecentItem] { recents.filter(\.isDir) }

    private func recentFiles(in folder: RecentItem) -> [RecentItem] {
        let prefix = folder.path.hasSuffix("/") ? folder.path : folder.path + "/"
        return recents.filter { !$0.isDir && $0.path.hasPrefix(prefix) }
    }

    private var looseRecentFiles: [RecentItem] {
        let prefixes = recentFolders.map { $0.path.hasSuffix("/") ? $0.path : $0.path + "/" }
        return recents.filter { item in
            !item.isDir && !prefixes.contains { item.path.hasPrefix($0) }
        }
    }

    @ViewBuilder
    private var recentsSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Recents")
                .font(.system(size: 11, weight: .semibold, design: .default))
                .foregroundStyle(muted)
                .padding(.horizontal, 28)
                .padding(.bottom, 8)

            if recents.isEmpty {
                Text("No Recent Projects")
                    .font(.system(size: 12, weight: .regular))
                    .foregroundStyle(muted.opacity(0.75))
                    .padding(.horizontal, 28)
                    .padding(.top, 4)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 3) {
                        ForEach(recentFolders) { folder in
                            RecentFolderRow(
                                item: folder,
                                fileCount: recentFiles(in: folder).count,
                                expanded: expandedRecents.contains(folder.path),
                                label: label,
                                muted: muted,
                                onToggle: {
                                    withAnimation(.snappy(duration: 0.22)) {
                                        if expandedRecents.contains(folder.path) {
                                            expandedRecents.remove(folder.path)
                                        } else {
                                            expandedRecents.insert(folder.path)
                                        }
                                    }
                                },
                                onOpen: { onOpenRecent(folder.path) }
                            )
                            if expandedRecents.contains(folder.path) {
                                let files = recentFiles(in: folder)
                                if files.isEmpty {
                                    Text("No recent files in this project")
                                        .font(.system(size: 11))
                                        .foregroundStyle(muted.opacity(0.7))
                                        .padding(.leading, 40)
                                        .padding(.vertical, 3)
                                        .transition(.opacity)
                                } else {
                                    ForEach(files) { file in
                                        RecentRow(
                                            item: file,
                                            label: label,
                                            muted: muted,
                                            indented: true
                                        ) {
                                            onOpenRecent(file.path)
                                        }
                                        .transition(.opacity.combined(with: .move(edge: .top)))
                                    }
                                }
                            }
                        }

                        let loose = looseRecentFiles
                        if !loose.isEmpty {
                            if !recentFolders.isEmpty {
                                Text("Files")
                                    .font(.system(size: 10, weight: .semibold))
                                    .foregroundStyle(muted.opacity(0.85))
                                    .padding(.horizontal, 10)
                                    .padding(.top, 8)
                                    .padding(.bottom, 2)
                            }
                            ForEach(loose) { item in
                                RecentRow(item: item, label: label, muted: muted) {
                                    onOpenRecent(item.path)
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                    .padding(.bottom, 16)
                }
            }
        }
    }

    private func welcomeButton(
        systemImage: String,
        title: String,
        action: @escaping () -> Void
    ) -> some View {
        WelcomeActionButton(
            systemImage: systemImage,
            title: title,
            fill: Color.white.opacity(0.07),
            label: label,
            action: action
        )
    }
}

/// Capsule action with an iOS-quality hover lift + press scale.
private struct WelcomeActionButton: View {
    var systemImage: String
    var title: String
    var fill: Color
    var label: Color
    var action: () -> Void
    @State private var hovering = false
    @State private var pressed = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 11) {
                Image(systemName: systemImage)
                    .font(.system(size: 13, weight: .regular))
                    .foregroundStyle(Color.white.opacity(hovering ? 0.92 : 0.70))
                    .frame(width: 18, alignment: .center)
                Text(title)
                    .font(.system(size: 13, weight: .regular, design: .default))
                    .foregroundStyle(label)
                Spacer(minLength: 0)
                Image(systemName: "chevron.right")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(Color.white.opacity(0.45))
                    .opacity(hovering ? 1 : 0)
                    .offset(x: hovering ? 0 : -4)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                Capsule(style: .continuous)
                    .fill(Color.white.opacity(hovering ? 0.12 : 0.07))
            )
            .overlay(
                Capsule(style: .continuous)
                    .strokeBorder(Color.white.opacity(hovering ? 0.14 : 0.0), lineWidth: 1)
            )
            .scaleEffect(pressed ? 0.985 : 1)
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(.snappy(duration: 0.18), value: hovering)
        .animation(.snappy(duration: 0.12), value: pressed)
        .simultaneousGesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in pressed = true }
                .onEnded { _ in pressed = false }
        )
    }
}

private struct RecentFolderRow: View {
    var item: RecentItem
    var fileCount: Int
    var expanded: Bool
    var label: Color
    var muted: Color
    var onToggle: () -> Void
    var onOpen: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: onOpen) {
            HStack(spacing: 10) {
                Button(action: onToggle) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(muted)
                        .frame(width: 12, height: 12)
                }
                .buttonStyle(.plain)

                Image(systemName: "folder.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(Color.white.opacity(0.55))

                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(label)
                        .lineLimit(1)
                    Text(item.subtitle)
                        .font(.system(size: 10))
                        .foregroundStyle(muted)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
                if fileCount > 0 {
                    Text("\(fileCount)")
                        .font(.system(size: 10, weight: .medium, design: .rounded))
                        .foregroundStyle(muted)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(Color.white.opacity(0.08)))
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                    .fill(hovering ? Color.white.opacity(0.08) : Color.clear)
            )
            .contentShape(RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(.easeOut(duration: 0.12), value: hovering)
    }
}

private struct RecentRow: View {
    var item: RecentItem
    var label: Color
    var muted: Color
    var indented: Bool = false
    var onOpen: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: onOpen) {
            HStack(spacing: 10) {
                Image(systemName: "doc.text")
                    .font(.system(size: 12))
                    .foregroundStyle(Color.white.opacity(0.50))
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(label)
                        .lineLimit(1)
                    Text(item.subtitle)
                        .font(.system(size: 10))
                        .foregroundStyle(muted)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .padding(.leading, indented ? 28 : 10)
            .padding(.trailing, 10)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                    .fill(hovering ? Color.white.opacity(0.08) : Color.clear)
            )
            .contentShape(RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(.easeOut(duration: 0.12), value: hovering)
    }
}

// MARK: - Bundled display fonts

/// Launch-time warmup steps that do real (if currently light) precomputation.
/// Each removes a cold path that would otherwise surface as a hitch during
/// editing — the whole point of paying the cost at boot.
enum Boot {
    /// Prime the editor font's cell metrics and CoreText's line cache with the
    /// ASCII set, so the first real paint of a document has no glyph-measure /
    /// shaping cold start.
    static func warmEditorGlyphs() {
        _ = EditorMetrics.cellWidth
        let sample = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 (){}[]<>"
        let attr = NSAttributedString(
            string: sample,
            attributes: [.font: EditorMetrics.monospaced(12, weight: .regular)]
        )
        _ = CTLineCreateWithAttributedString(attr)
    }

    /// Touch the recents store so its disk read is already cached when the list
    /// appears (and, later, so the reveal never waits on I/O).
    static func primeRecents() {
        _ = RecentStore.load()
    }
}

enum WelcomeFonts {
    private static var didRegister = false

    /// Register `Resources/Fonts/*` with Core Text once per process. Info.plist
    /// `ATSApplicationFontsPath` covers normal launches; this covers direct
    /// binary runs and first-frame races before ATS finishes scanning.
    static func registerIfNeeded() {
        guard !didRegister else { return }
        didRegister = true
        guard let fontsDir = Bundle.main.resourceURL?
            .appendingPathComponent("Fonts", isDirectory: true),
              let files = try? FileManager.default.contentsOfDirectory(
                at: fontsDir,
                includingPropertiesForKeys: nil
              )
        else { return }
        for url in files where ["otf", "ttf", "ttc"].contains(url.pathExtension.lowercased()) {
            var error: Unmanaged<CFError>?
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, &error)
        }
    }
}

// MARK: - Welcome hero rotation (one image per launch)

/// One slot in the WelcomeHeroes pool. Files live in
/// `Resources/WelcomeHeroes/00.jpg` … `09.jpg` (plus optional captions).
struct WelcomeHeroPick: Identifiable, Equatable {
    var id: String
    var title: String
    var subtitle: String
    /// Loaded once at pick time so SwiftUI does not re-read the disk.
    var image: NSImage?

    static func == (lhs: WelcomeHeroPick, rhs: WelcomeHeroPick) -> Bool {
        lhs.id == rhs.id
    }

    /// Captions keyed by zero-padded file stem (`00` …). Unknown stems get a
    /// generic deep-sky label.
    private static let captions: [String: (String, String)] = [
        "00": ("Orion Nebula", "M42 · deep-sky photograph"),
        "01": ("Westerlund 2", "Young star cluster · Hubble"),
        "02": ("Carina Nebula", "NGC 3372 · star-forming region · Hubble"),
        "03": ("Lagoon Nebula", "M8 · emission nebula · Hubble"),
        "04": ("Cosmic Cliffs", "Carina · JWST NIRCam"),
        "05": ("Whirlpool Galaxy", "M51 · interacting spiral · Hubble"),
        "06": ("Orion Nebula", "M42 · Hubble ACS"),
        "07": ("Butterfly Nebula", "NGC 6302 · planetary nebula · Hubble"),
        "08": ("Horsehead Nebula", "Barnard 33 · dark nebula · Hubble"),
        "09": ("Galaxy field", "Distant galaxies · Hubble"),
    ]

    /// Prefer a different hero than last launch when the pool has 2+.
    static func pickForThisLaunch() -> WelcomeHeroPick {
        let urls = heroURLs()
        guard !urls.isEmpty else {
            // Legacy single-file fallback.
            if let u = Bundle.main.url(forResource: "WelcomeHero", withExtension: "jpg")
                ?? Bundle.main.url(forResource: "WelcomeHero", withExtension: "png"),
               let img = NSImage(contentsOf: u)
            {
                return WelcomeHeroPick(
                    id: "WelcomeHero",
                    title: "Orion Nebula",
                    subtitle: "M42 · deep sky",
                    image: img
                )
            }
            return WelcomeHeroPick(id: "fallback", title: "Suisei", subtitle: "Deep space", image: nil)
        }

        let key = "suisei.welcome.hero.lastId"
        let last = UserDefaults.standard.string(forKey: key)
        var pool = urls
        if urls.count > 1, let last {
            pool = urls.filter { $0.deletingPathExtension().lastPathComponent != last }
            if pool.isEmpty { pool = urls }
        }
        let chosen = pool.randomElement() ?? urls[0]
        let stem = chosen.deletingPathExtension().lastPathComponent
        UserDefaults.standard.set(stem, forKey: key)
        let cap = captions[stem] ?? ("Deep sky", "Welcome art")
        return WelcomeHeroPick(
            id: stem,
            title: cap.0,
            subtitle: cap.1,
            image: NSImage(contentsOf: chosen)
        )
    }

    private static func heroURLs() -> [URL] {
        // Prefer subdirectory WelcomeHeroes/ in the app bundle.
        if let dir = Bundle.main.resourceURL?
            .appendingPathComponent("WelcomeHeroes", isDirectory: true),
           let files = try? FileManager.default.contentsOfDirectory(
            at: dir,
            includingPropertiesForKeys: nil
           )
        {
            let imgs = files
                .filter { ["jpg", "jpeg", "png"].contains($0.pathExtension.lowercased()) }
                .sorted { $0.lastPathComponent < $1.lastPathComponent }
            if !imgs.isEmpty { return imgs }
        }
        // Flat names WelcomeHero-00.jpg … WelcomeHero-09.jpg
        var flat: [URL] = []
        for i in 0..<16 {
            let name = String(format: "WelcomeHero-%02d", i)
            if let u = Bundle.main.url(forResource: name, withExtension: "jpg")
                ?? Bundle.main.url(forResource: name, withExtension: "png")
            {
                flat.append(u)
            }
        }
        return flat
    }
}

struct RecentItem: Identifiable, Equatable, Codable {
    var id: String { path }
    var path: String
    var title: String
    var subtitle: String
    var isDir: Bool
}

enum RecentStore {
    private static let key = "suisei.recents.v1"

    static func load() -> [RecentItem] {
        guard let data = UserDefaults.standard.data(forKey: key),
              let items = try? JSONDecoder().decode([RecentItem].self, from: data)
        else { return [] }
        return items
    }

    static func push(path: String) {
        var items = load()
        items.removeAll { $0.path == path }
        let url = URL(fileURLWithPath: path)
        let isDir = (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
        let title = url.lastPathComponent
        let parent = url.deletingLastPathComponent().path
        items.insert(
            RecentItem(path: path, title: title, subtitle: parent, isDir: isDir),
            at: 0
        )
        if items.count > 12 { items = Array(items.prefix(12)) }
        if let data = try? JSONEncoder().encode(items) {
            UserDefaults.standard.set(data, forKey: key)
        }
    }
}

// MARK: - Plain-window AppKit chrome (rounded + draggable)

/// Shared AppKit apply path for the Welcome plain window.
enum WelcomeChromeApplier {
    static func apply(to window: NSWindow, cornerRadius: CGFloat) {
        // 1) Transparent host so continuous corners show the desktop, not a square slab.
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = true
        window.isMovable = true
        window.isMovableByWindowBackground = true

        // 2) Continuous corner mask on the window content view (the exterior shape).
        if let cv = window.contentView {
            cv.wantsLayer = true
            cv.layer?.cornerRadius = cornerRadius
            cv.layer?.cornerCurve = .continuous
            cv.layer?.masksToBounds = true
            cv.layer?.backgroundColor = NSColor.clear.cgColor

            if let frame = cv.superview {
                frame.wantsLayer = true
                frame.layer?.cornerRadius = cornerRadius
                frame.layer?.cornerCurve = .continuous
                frame.layer?.masksToBounds = true
            }
        }

        if let host = window.contentView?.subviews.first {
            host.wantsLayer = true
            host.layer?.cornerRadius = cornerRadius
            host.layer?.cornerCurve = .continuous
            host.layer?.masksToBounds = true
        }

        // 3) Fixed compact size — prevent content-size stretch.
        let size = WelcomeView.windowSize
        let content = window.contentLayoutRect.size
        if abs(content.width - size.width) > 0.5 || abs(content.height - size.height) > 0.5 {
            window.setContentSize(size)
        }
        var frameSize = size
        let deltaW = window.frame.width - window.contentLayoutRect.width
        let deltaH = window.frame.height - window.contentLayoutRect.height
        frameSize.width += max(0, deltaW)
        frameSize.height += max(0, deltaH)
        window.minSize = frameSize
        window.maxSize = frameSize

        // 4) Shadow must recompute after radius/opaque changes or it stays rectangular.
        window.invalidateShadow()
    }

    /// Best-effort find of the Welcome plain window (never the editor shell).
    static func applyToWelcomeWindows(cornerRadius: CGFloat = WelcomeView.cornerRadius) {
        for window in NSApp.windows {
            let id = window.identifier?.rawValue ?? ""
            let titledWelcome = window.title == "Welcome"
            let idWelcome = id == "welcome"
            let plainSmall = !window.styleMask.contains(.titled)
                && window.isVisible
                && window.frame.width <= WelcomeView.windowSize.width + 40
                && window.frame.height <= WelcomeView.windowSize.height + 40
            if titledWelcome || idWelcome || plainSmall {
                apply(to: window, cornerRadius: cornerRadius)
            }
        }
    }
}

/// `.windowStyle(.plain)` strips system chrome: square exterior, no background-drag.
/// Re-apply continuous corners, clear opacity, and movable background the AppKit way.
private struct WelcomeWindowChrome: NSViewRepresentable {
    var cornerRadius: CGFloat

    func makeNSView(context: Context) -> WelcomeChromeProbe {
        let v = WelcomeChromeProbe(cornerRadius: cornerRadius)
        v.frame = .zero
        return v
    }

    func updateNSView(_ nsView: WelcomeChromeProbe, context: Context) {
        nsView.cornerRadius = cornerRadius
        nsView.applyChrome()
    }
}

/// Probe that configures its host `NSWindow` once attached (and on layout).
final class WelcomeChromeProbe: NSView {
    var cornerRadius: CGFloat

    init(cornerRadius: CGFloat) {
        self.cornerRadius = cornerRadius
        super.init(frame: .zero)
        isHidden = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var mouseDownCanMoveWindow: Bool { true }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        applyChrome()
    }

    override func viewDidMoveToSuperview() {
        super.viewDidMoveToSuperview()
        applyChrome()
    }

    override func layout() {
        super.layout()
        applyChrome()
    }

    func applyChrome() {
        guard let window else { return }
        WelcomeChromeApplier.apply(to: window, cornerRadius: cornerRadius)
    }

    override func mouseDown(with event: NSEvent) {
        window?.performDrag(with: event)
    }
}

// MARK: - Window sizing (welcome compact → editor expanded)

enum SuiseiWindowLayout {
    static let welcomeSize = WelcomeView.windowSize
    static let editorSize = NSSize(width: 1280, height: 820)
    static let editorMinSize = NSSize(width: 900, height: 560)

    /// Resize + center the **editor** window after leaving Welcome.
    /// Welcome chrome is owned by SwiftUI scene modifiers (not AppKit button hiding).
    static func apply(welcome: Bool, animate: Bool = true) {
        // Prefer the editor WindowGroup window when leaving welcome.
        let window = NSApp.windows.first(where: {
            $0.isVisible && $0.title != "Settings" && $0.title != "Welcome"
        })
            ?? NSApp.windows.first(where: { $0.isVisible || $0.isMainWindow })
            ?? NSApp.mainWindow
            ?? NSApp.windows.first
        guard let window else { return }

        if welcome {
            // Welcome Window scene sizes itself via .windowResizability(.contentSize).
            return
        }

        let target = editorSize
        window.minSize = editorMinSize
        window.maxSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )
        window.isMovableByWindowBackground = false
        window.isOpaque = true
        window.titlebarAppearsTransparent = true

        // Already editor-sized (SwiftUI defaultSize) → re-framing here is what
        // made freshly opened windows visibly snap sideways. Leave it alone.
        if abs(window.frame.width - target.width) < 80,
           abs(window.frame.height - target.height) < 80
        {
            return
        }

        let screen = window.screen ?? NSScreen.main
        let visible = screen?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1400, height: 900)
        var frame = window.frame
        frame.size = target
        frame.origin.x = visible.midX - target.width / 2
        frame.origin.y = visible.midY - target.height / 2
        frame.origin.x = max(visible.minX + 20, min(frame.origin.x, visible.maxX - target.width - 20))
        frame.origin.y = max(visible.minY + 20, min(frame.origin.y, visible.maxY - target.height - 20))

        if animate {
            NSAnimationContext.runAnimationGroup { ctx in
                ctx.duration = 0.28
                ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                window.animator().setFrame(frame, display: true)
            }
        } else {
            window.setFrame(frame, display: true)
        }
    }
}
