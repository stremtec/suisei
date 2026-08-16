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
    var onNewProject: () -> Void
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

    /// Three columns now, so the width is the sum of them rather than a ratio.
    ///
    /// Recents used to sit UNDER the four action buttons in a 344pt rail, which
    /// left it roughly 200pt tall and one line wide — a list of paths is the
    /// one thing on this window that needs width, and it had the least. The
    /// rail and the art keep their exact previous widths; the window grew
    /// horizontally by the new column and by nothing else, so nothing that was
    /// laid out before has moved.
    static let controlWidth: CGFloat = 344
    static let recentsWidth: CGFloat = 316
    static let artWidth: CGFloat = 516
    /// The wordmark's band, and the two columns beneath it.
    static let leftWidth: CGFloat = controlWidth + recentsWidth
    static let windowSize = NSSize(width: leftWidth + artWidth, height: 500)
    /// Welcome is borderless, so it cuts its own corner — and it has to match a
    /// real window sitting next to it. Same source as every other surface that
    /// lines up with a window edge.
    static let cornerRadius: CGFloat = WindowChrome.windowCornerRadius

    @Environment(\.colorScheme) private var scheme

    private var ink: WelcomeInk { .of(scheme) }
    /// Solid control rail on purpose, so the art reads as the only “hero”
    /// surface (glass on both sides made the card muddy).
    private var controlBg: Color { ink.rail }
    private var label: Color { ink.label }
    private var muted: Color { ink.muted }
    private var hairline: Color { ink.hairline }

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
                HStack(spacing: 0) {
                    // The wordmark spans BOTH left columns and the two columns
                    // start under it. Splitting the rail put the mark in a
                    // 344pt slot with the buttons directly beneath, and left
                    // the bottom half of both columns empty — the window had
                    // grown a third column without giving anything a reason to
                    // reach the floor.
                    VStack(spacing: 0) {
                        brandHeader

                        HStack(spacing: 0) {
                            actionsColumn
                                .frame(width: Self.controlWidth)

                            recentsColumn
                                .frame(width: Self.recentsWidth)
                                .overlay(alignment: .leading) { columnDivider }
                        }
                        .frame(maxHeight: .infinity, alignment: .top)
                    }
                    .frame(width: Self.leftWidth, height: geo.size.height)
                    .background(controlBg)

                    artPanel
                        .frame(
                            width: max(0, geo.size.width - Self.leftWidth),
                            height: geo.size.height
                        )
                        // A hairline, not a dissolve.
                        //
                        // This was a 48pt bleed of the rail into the art. On a
                        // near-black rail it read as the photograph fading in;
                        // on a light one it is a white veil smeared over the
                        // left eighth of a nebula, which is a smudge and not a
                        // transition. A panel meeting an image is an edge —
                        // every app that puts one beside the other draws it as
                        // one — and an edge does not have to be told which
                        // appearance it is in.
                        .overlay(alignment: .leading) {
                            Rectangle()
                                .fill(ink.hairline)
                                .frame(width: 1)
                                .allowsHitTesting(false)
                        }
                }
            }

            Button(role: .cancel, action: onClose) {
                Image(systemName: "xmark.circle.fill")
                    .symbolRenderingMode(.hierarchical)
                    .font(.system(size: 14, weight: .regular))
                    .foregroundStyle(ink.muted)
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
                .strokeBorder(ink.hairline, lineWidth: 1)
        )
        .shadow(color: .black.opacity(scheme == .dark ? 0.45 : 0.22), radius: 28, y: 14)
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

    /// The wordmark band across the top of both left columns.
    ///
    /// Type-only lockup — Milker. The version and legal lines stay on the
    /// system face because they are running text, not a mark; Milker's own
    /// coverage is wider than this comment used to claim (letters, digits,
    /// `_` and signature punctuation are all in it, checked against the
    /// font's character set).
    private var brandHeader: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Suisei")
                .font(brandWordmarkFont)
                .foregroundStyle(ink.brand)
                .lineLimit(1)
            VStack(alignment: .leading, spacing: 3) {
                Text("© 2025–2026 Stemtec. All rights reserved.")
                    .font(.system(size: 10, weight: .regular))
                    .foregroundStyle(muted.opacity(0.90))
                Text("Suisei 2026dev · Legal Information")
                    .font(.system(size: 10, weight: .regular))
                    .foregroundStyle(muted.opacity(0.75))
            }
            .padding(.top, 8)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.leading, 26)
        .padding(.trailing, 20)
        .padding(.top, 30)
        .padding(.bottom, 16)
        .opacity(appeared ? 1 : 0)
        .offset(y: appeared ? 0 : 6)
    }

    /// The launch actions, centred in what is left below the wordmark.
    ///
    /// Two positions were tried and both left one large hole. Directly under
    /// the wordmark emptied the bottom of the column; on the floor emptied its
    /// middle. Four buttons simply do not fill 320pt, so the honest move is to
    /// stop pretending they do and split the leftover in two — a gap above and
    /// a gap below, each about half the size, which reads as breathing room
    /// instead of as a place where something is missing.
    private var actionsColumn: some View {
        // The actions wait behind the boot sequence; the loading line occupies
        // the same region until `ready`, then the actions rise into place. Both
        // live in one ZStack so the reveal is a cross-fade in a fixed
        // footprint, not a layout jump.
        ZStack {
            bootLoadingView
                .opacity(ready ? 0 : 1)
                .allowsHitTesting(!ready)

            launchActions
                .opacity(ready ? 1 : 0)
                .offset(y: ready ? 0 : 10)
                .allowsHitTesting(ready)
        }
        .frame(maxHeight: .infinity)
        .padding(.bottom, 26)
        .animation(.smooth(duration: 0.55), value: ready)
    }

    /// A short rule between the two columns.
    ///
    /// The full-height version ran from the wordmark to the floor: the
    /// strongest line in the window, drawn across its emptiest part, cutting
    /// one panel in half. A card was worse in the other direction — it made
    /// recents a separate object sitting on the rail when it is part of it.
    ///
    /// A short rule says only what needs saying. It fades out at both ends
    /// rather than stopping dead, because a 1pt line with hard terminals reads
    /// as a line that was clipped rather than one that was drawn that length.
    private var columnDivider: some View {
        LinearGradient(
            colors: [.clear, hairline, hairline, .clear],
            startPoint: .top,
            endPoint: .bottom
        )
        .frame(width: 1, height: 150)
        .allowsHitTesting(false)
    }

    /// The recents column, revealed on the same beat as the actions.
    ///
    /// It reads `ready` itself rather than being handed it, because it no
    /// longer shares a ZStack with the boot line — the loading state belongs to
    /// the actions column, and a second spinner here would say the same thing
    /// twice.
    private var recentsColumn: some View {
        recentsSection
            .padding(.top, 4)
            .padding(.bottom, 26)
            .frame(maxHeight: .infinity, alignment: .top)
            .opacity(ready ? 1 : 0)
            .offset(y: ready ? 0 : 10)
            .allowsHitTesting(ready)
            .animation(.smooth(duration: 0.55), value: ready)
    }

    /// Launch actions — revealed once boot is `ready`.
    ///
    /// No `maxHeight: .infinity` here. The stack sizes to its four buttons and
    /// the column centres it; filling the height and top-aligning inside would
    /// pin them back under the wordmark, which is what this layout change was
    /// undoing. It read as a working build and looked untouched — the kind of
    /// modifier that survives a rewrite because nothing fails.
    private var launchActions: some View {
        VStack(spacing: 8) {
            // A project first: it is the thing this window is for, and until
            // now the only way to get one was to open a folder that already
            // existed somewhere else.
            welcomeButton(systemImage: "shippingbox", title: "Create New Project…", action: onNewProject)
            welcomeButton(systemImage: "plus.square", title: "Create New File…", action: onCreate)
            welcomeButton(systemImage: "square.and.arrow.down.on.square", title: "Clone Git Repository…", action: onClone)
            welcomeButton(systemImage: "folder", title: "Open Existing Project…", action: onOpen)
        }
        .padding(.horizontal, 22)
    }

    /// The After Effects-style status line: a spinner plus the current boot
    /// stage's label, swapped with a cross-fade as stages advance.
    private var bootLoadingView: some View {
        HStack(spacing: 10) {
            ProgressView()
                .controlSize(.small)
                .tint(ink.muted)
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

    /// The wordmark face — **Milker**, registered from `Resources/Fonts` via
    /// `ATSApplicationFontsPath` plus a runtime register, so a first launch
    /// before the system font cache settles still works.
    ///
    /// It replaces Gondens, and the reason is measurable rather than a matter
    /// of taste. Gondens at 44pt reports a line height of **103.4pt** — its box
    /// is 2.35× its point size — so drawing a 44pt wordmark reserved 103pt of
    /// column and the layout below it had to be nudged back up. Milker at 44pt
    /// reports 42.5. That single number is what "the font is vertically long"
    /// was, and it is why the workarounds around this call site (a
    /// `minimumScaleFactor`, a note about reclaiming excess leading) are gone
    /// with it.
    ///
    /// Milker is also wider — 155pt against Gondens' 113 at the same size.
    ///
    /// The mark now spans BOTH left columns rather than one 344pt rail, so it
    /// has ~614pt to work in. Measured: 104pt sets "Suisei" at 366pt wide and
    /// 101pt tall, which fills the band without reaching its trailing edge —
    /// the point is a header with presence, not a word jammed edge to edge.
    private static let wordmarkSize: CGFloat = 104

    private var brandWordmarkFont: Font {
        WelcomeFonts.registerIfNeeded()
        // PostScript name from the OTF, family name as the fallback.
        if let face = NSFont(name: "MilkerRegular", size: Self.wordmarkSize)
            ?? NSFont(name: "Milker", size: Self.wordmarkSize)
        {
            return Font(face)
        }
        return .system(size: Self.wordmarkSize, weight: .heavy, design: .rounded)
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

    /// Folders, projects first.
    ///
    /// A marked folder is the thing you came here to open; an unmarked one is
    /// somewhere you happened to be. Ordering by that rather than by recency
    /// alone is the whole point of the marker existing.
    private var recentFolders: [RecentItem] {
        let dirs = recents.filter(\.isDir)
        return dirs.filter(\.isProject) + dirs.filter { !$0.isProject }
    }

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
                .padding(.horizontal, 22)
                .padding(.bottom, 8)

            if recents.isEmpty {
                Text("No Recent Projects")
                    .font(.system(size: 12, weight: .regular))
                    .foregroundStyle(muted.opacity(0.75))
                    .padding(.horizontal, 22)
                    .padding(.top, 4)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 3) {
                        ForEach(recentFolders) { folder in
                            RecentFolderRow(
                                item: folder,
                                fileCount: recentFiles(in: folder).count,
                                expanded: expandedRecents.contains(folder.path),
                                ink: ink,
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
                                            ink: ink,
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
                                RecentRow(item: item, ink: ink) {
                                    onOpenRecent(item.path)
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                    .padding(.bottom, 8)
                }
            }
        }
    }

    private func welcomeButton(
        systemImage: String,
        title: String,
        action: @escaping () -> Void
    ) -> some View {
        WelcomeActionButton(systemImage: systemImage, title: title, action: action)
    }
}

/// A launch action, on the system's own button material.
///
/// This was a hand-drawn capsule: `Color.white.opacity(0.07)`, lifted to `0.12`
/// on hover, a 1pt white border faded in with it, and a 0.985 press scale
/// driven by a `DragGesture` — four hand-tuned numbers imitating what
/// `.buttonStyle(.bordered)` already is. The imitation cannot follow the system:
/// it does not change with Increase Contrast or Reduce Transparency, it draws
/// no focus ring for keyboard navigation, and its press state came from a drag
/// gesture rather than from the button, so it stayed pressed if the pointer
/// left while held.
///
/// The rail follows the system appearance, and so do the controls on it. It
/// used to be a fixed near-black with the scheme forced dark on top, which is
/// why a Mac in Light mode still opened to a black window — the fix is one
/// palette that moves, not two that disagree.
private struct WelcomeActionButton: View {
    var systemImage: String
    var title: String
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: systemImage)
                    .font(.system(size: 13, weight: .regular))
                    .frame(width: 18, alignment: .center)
                Text(title)
                    .font(.system(size: 13))
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 3)
        }
        .buttonStyle(.bordered)
        .controlSize(.large)
    }
}

/// The welcome rail's colours.
///
/// The left column is a solid panel with the hero art beside it, and it was a
/// near-black in BOTH appearances with white ink on it — so a Mac in Light
/// mode opened to a black window with white text, which is the one surface in
/// the app that never followed the system.
///
/// The art does not follow. A nebula is a photograph: its caption is white
/// because the image behind it is dark, and lightening the plate under it
/// would only make the credit unreadable.
struct WelcomeInk {
    var rail: Color
    /// The wordmark, which carries more weight than body text.
    var brand: Color
    var label: Color
    var muted: Color
    var hairline: Color
    /// Row hover, and the file-count chip.
    var hover: Color
    var chip: Color
    var icon: Color
    var iconStrong: Color

    static func of(_ scheme: ColorScheme) -> WelcomeInk {
        scheme == .dark
            ? WelcomeInk(
                rail: Color(red: 0.06, green: 0.06, blue: 0.07),
                brand: .white,
                label: .white.opacity(0.92),
                muted: .white.opacity(0.48),
                hairline: .white.opacity(0.08),
                hover: .white.opacity(0.08),
                chip: .white.opacity(0.08),
                icon: .white.opacity(0.52),
                iconStrong: .white.opacity(0.85)
            )
            : WelcomeInk(
                // Not pure white: the card sits over the desktop with a
                // shadow, and a #FFF panel beside a photograph reads as a
                // hole rather than as a surface.
                rail: Color(red: 0.965, green: 0.965, blue: 0.972),
                brand: Color(white: 0.09),
                label: .black.opacity(0.88),
                muted: .black.opacity(0.52),
                hairline: .black.opacity(0.12),
                hover: .black.opacity(0.06),
                chip: .black.opacity(0.07),
                icon: .black.opacity(0.55),
                iconStrong: .black.opacity(0.82)
            )
    }
}

private struct RecentFolderRow: View {
    var item: RecentItem
    var fileCount: Int
    var expanded: Bool
    var ink: WelcomeInk
    var onToggle: () -> Void
    var onOpen: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: onOpen) {
            HStack(spacing: 10) {
                Button(action: onToggle) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(ink.muted)
                        .frame(width: 12, height: 12)
                }
                .buttonStyle(.plain)

                // A project reads as one at a glance. Same slot, same size —
                // only the glyph and its weight change, so the column stays a
                // column.
                Image(systemName: item.isProject ? "shippingbox.fill" : "folder.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(item.isProject ? ink.iconStrong : ink.icon)

                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(ink.label)
                        .lineLimit(1)
                    Text(item.subtitle)
                        .font(.system(size: 10))
                        .foregroundStyle(ink.muted)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
                if fileCount > 0 {
                    Text("\(fileCount)")
                        .font(.system(size: 10, weight: .medium, design: .rounded))
                        .foregroundStyle(ink.muted)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(ink.chip))
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                    .fill(hovering ? ink.hover : Color.clear)
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
    var ink: WelcomeInk
    var indented: Bool = false
    var onOpen: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: onOpen) {
            HStack(spacing: 10) {
                Image(systemName: "doc.text")
                    .font(.system(size: 12))
                    .foregroundStyle(ink.icon)
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title)
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(ink.label)
                        .lineLimit(1)
                    Text(item.subtitle)
                        .font(.system(size: 10))
                        .foregroundStyle(ink.muted)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .padding(.leading, indented ? 28 : 10)
            .padding(.trailing, 10)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                    .fill(hovering ? ink.hover : Color.clear)
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
    /// This folder carries `project.suiseiprj`.
    ///
    /// Decoded as `false` for entries written before the marker existed, which
    /// is what `Codable` does with a missing key given a default — and the
    /// right answer, because those entries were never checked.
    var isProject: Bool = false
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
        let isProject = isDir && path.withCString { suisei_project_is_marked($0) != 0 }
        items.insert(
            RecentItem(
                path: path, title: title, subtitle: parent,
                isDir: isDir, isProject: isProject
            ),
            at: 0
        )
        // Projects are not evicted by files. The cap used to be a flat twelve,
        // so opening a dozen files buried the three folders you actually work
        // in — the entries most worth keeping were the ones most easily pushed
        // out, because you open a project once and its files all day.
        var projects = items.filter(\.isProject)
        var rest = items.filter { !$0.isProject }
        if projects.count > 8 { projects = Array(projects.prefix(8)) }
        if rest.count > 12 { rest = Array(rest.prefix(12)) }
        items = projects + rest
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
