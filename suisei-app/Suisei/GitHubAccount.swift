import AppKit
import Combine
import SwiftUI

/// Snapshot of the signed-in GitHub identity. Independent of chrome so a
/// profile refresh cannot rebuild the editor tree.
struct GitHubAccountSnap: Equatable {
    var generation: UInt64 = 0
    var state: UInt8 = 0
    var loading = false
    var signingIn = false
    var publicRepos: UInt32 = 0
    var followers: UInt32 = 0
    var following: UInt32 = 0
    var login = ""
    var name = ""
    var email = ""
    var avatarURL = ""
    var bio = ""
    var company = ""
    var location = ""
    var htmlURL = ""
    var host = ""
    var protocolName = ""
    var scopes = ""
    var tokenSource = ""
    var deviceCode = ""
    var message = ""
    var contribTotal: UInt32 = 0
    var contribLevels: [UInt8] = []
    var contribStart = ""
    /// 0 = rolling last 365 days.
    var contribYear: UInt32 = 0
    var contribYearMin: UInt32 = 0

    static let empty = GitHubAccountSnap()

    var isMissingCLI: Bool { state == SUISEI_GH_STATE_MISSING }
    var isSignedOut: Bool { state == SUISEI_GH_STATE_OUT }
    var isSignedIn: Bool { state == SUISEI_GH_STATE_IN }

    var displayName: String {
        if !name.isEmpty { return name }
        if !login.isEmpty { return login }
        return "GitHub Account"
    }

    var subtitle: String {
        if !email.isEmpty { return email }
        if !login.isEmpty { return login }
        return host.isEmpty ? "github.com" : host
    }
}

/// Own publish rate, same reason as `GitWorkbenchStore`.
final class GitHubAccountStore: ObservableObject {
    @Published private(set) var snap = GitHubAccountSnap.empty
    @Published private(set) var avatar: NSImage?

    private var avatarURL = ""
    private var avatarTask: URLSessionDataTask?

    func publish(_ next: GitHubAccountSnap) {
        guard next != snap else { return }
        snap = next
        loadAvatar(from: next.avatarURL)
    }

    func loadAvatar(from urlString: String) {
        guard urlString != avatarURL else { return }
        avatarURL = urlString
        avatarTask?.cancel()
        guard let url = URL(string: urlString), !urlString.isEmpty else {
            avatar = nil
            return
        }
        let sized = Self.retinaAvatarURL(url)
        if let cached = GitHubAvatarCache.image(for: sized) {
            avatar = cached
            return
        }
        avatarTask = URLSession.shared.dataTask(with: sized) { [weak self] data, _, error in
            guard let self else { return }
            guard let data, let image = NSImage(data: data) else {
                // Forget which URL we were loading, so the next publish or a
                // press of Refresh tries again. Without this a single dropped
                // request meant no portrait until the app was restarted: the
                // guard at the top of `loadAvatar` would keep answering "we
                // are already on that one".
                //
                // Except when WE cancelled it — a newer URL is already in
                // flight and resetting here would let this one's failure
                // undo it.
                let cancelled = (error as? URLError)?.code == .cancelled
                DispatchQueue.main.async {
                    guard !cancelled, self.avatarURL == urlString else { return }
                    self.avatarURL = ""
                }
                return
            }
            GitHubAvatarCache.store(image, for: sized)
            DispatchQueue.main.async {
                guard self.avatarURL == urlString else { return }
                self.avatar = image
            }
        }
        avatarTask?.resume()
    }

    /// GitHub's default avatar URL is ~80px. The account hero is a 104pt
    /// disc on a retina display, so ask for a 240px variant or the photo
    /// looks like a soft stamp.
    private static func retinaAvatarURL(_ url: URL) -> URL {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return url
        }
        var items = parts.queryItems ?? []
        items.removeAll { $0.name == "s" }
        items.append(URLQueryItem(name: "s", value: "240"))
        parts.queryItems = items
        return parts.url ?? url
    }
}

private enum GitHubAvatarCache {
    private static let folder: URL = {
        let base = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let dir = base.appendingPathComponent("Suisei/avatars", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        // Sweep what the per-process hash left behind. Those files are named
        // after a number that will never be computed again, so nothing will
        // ever read them — one per launch, for as many launches as the account
        // page has been opened. Everything this cache writes now ends in .png,
        // so anything that does not is from the old scheme.
        if let old = try? FileManager.default.contentsOfDirectory(
            at: dir, includingPropertiesForKeys: nil
        ) {
            for file in old where file.pathExtension != "png" {
                try? FileManager.default.removeItem(at: file)
            }
        }
        return dir
    }()

    /// The cache file for a URL, by a hash that is the same next launch.
    ///
    /// This was `String(url.absoluteString.hashValue)`. Swift seeds `Hasher`
    /// randomly per process, so the name changed every time the app started:
    /// the disk cache could never hit across launches, every launch re-fetched
    /// the portrait over the network, and every launch left another file
    /// behind that nothing would ever look for again. A cache that grows
    /// without bound and never answers is worse than no cache — it is the cost
    /// of one with none of the benefit.
    ///
    /// FNV-1a, spelled out, because the requirement is exactly that it not
    /// change: any hash the standard library might reseed or re-tune is the
    /// same bug written more nicely.
    private static func file(for url: URL) -> URL {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in url.absoluteString.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x100_0000_01b3
        }
        return folder.appendingPathComponent(String(format: "%016llx.png", hash))
    }

    static func image(for url: URL) -> NSImage? {
        let path = file(for: url)
        guard let data = try? Data(contentsOf: path) else { return nil }
        return NSImage(data: data)
    }

    static func store(_ image: NSImage, for url: URL) {
        // PNG, not TIFF. A 240px portrait is about 230 KB uncompressed and
        // about 20 KB as PNG, and this is a file written once and read on
        // every launch thereafter.
        guard let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:])
        else { return }
        try? png.write(to: file(for: url), options: .atomic)
    }
}

/// Circular GitHub portrait. Falls back to the mark, never to an empty hole.
///
/// System Settings' Apple Account photo is a clean disc — no hairline ring.
/// A ring made a generated avatar look like a toolbar glyph sitting on a
/// token, which is the cheap version of this surface.
struct GitHubAvatarView: View {
    var image: NSImage?
    var size: CGFloat
    var signedIn: Bool
    var lifted = false

    var body: some View {
        ZStack {
            Circle()
                .fill(Color.primary.opacity(signedIn ? 0.08 : 0.06))
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .scaledToFill()
            } else {
                Image(systemName: signedIn ? "person.fill" : "person")
                    .font(.system(size: size * 0.42, weight: .regular))
                    .foregroundStyle(.secondary)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .shadow(color: .black.opacity(lifted && image != nil ? 0.22 : 0), radius: 10, y: 3)
    }
}

// MARK: - Account pages (System Settings / Apple Account layout)

/// System Settings group fill: white cards on gray in Light, a lifted
/// slab in Dark. `primary.opacity(0.07)` is a black wash in Light, so
/// Suisei's boxes read darker than Apple's.
enum SettingsGroupFill {
    static func color(for scheme: ColorScheme) -> Color {
        switch scheme {
        case .dark:
            return Color.white.opacity(0.11)
        default:
            return Color.white
        }
    }
}

struct GitHubAccountRootPage: View {
    @ObservedObject var store: GitHubAccountStore
    @Environment(\.colorScheme) private var colorScheme
    var accent: Color
    var onOpenProfile: () -> Void
    var onOpenSecurity: () -> Void
    var onSignIn: () -> Void
    var onCancel: () -> Void
    var onSignOut: () -> Void
    var onRefresh: () -> Void
    var onInstall: () -> Void
    var onOpenGitHub: () -> Void
    var onHelp: () -> Void

    private var snap: GitHubAccountSnap { store.snap }

    var body: some View {
        // No Form on this page. Grouped Form either boxes the hero, paints a
        // second background under the groups, or — with `fixedSize` — crawls
        // into the titlebar. One ScrollView, one window background, groups
        // drawn as rounded cards with a primary wash so they stay visible.
        ScrollView {
            VStack(spacing: 20) {
                hero
                if snap.isSignedIn, !snap.contribLevels.isEmpty {
                    GitHubContributionCard(
                        total: snap.contribTotal,
                        levels: snap.contribLevels,
                        start: snap.contribStart,
                        year: snap.contribYear,
                        yearMin: snap.contribYearMin,
                        accent: accent,
                        onSelectYear: { EngineBridge.shared.setGitHubContribYear($0) }
                    )
                }
                if snap.isSignedIn {
                    accountGroup {
                        accountRow("person.crop.circle", "Profile", snap.login, action: onOpenProfile)
                        groupDivider
                        accountRow("lock", "Sign-In & Security", snap.host.isEmpty ? "github.com" : snap.host, action: onOpenSecurity)
                    }
                    accountGroup {
                        accountRow("folder", "Repositories", count(snap.publicRepos), action: onOpenGitHub)
                        groupDivider
                        accountRow("person.2", "Followers", count(snap.followers), action: onOpenGitHub)
                        if !snap.company.isEmpty {
                            groupDivider
                            accountRow("building.2", "Company", snap.company, action: onOpenProfile)
                        }
                        groupDivider
                        accountRow("arrow.up.right.square", "Open on GitHub", snap.login, action: onOpenGitHub)
                    }
                } else {
                    signedOutGroup
                }
                footer
            }
            .padding(.horizontal, 20)
            .padding(.top, 20)
            .padding(.bottom, 24)
            .frame(maxWidth: .infinity)
        }
    }

    private var hero: some View {
        VStack(spacing: 12) {
            ZStack(alignment: .bottomTrailing) {
                GitHubAvatarView(
                    image: store.avatar,
                    size: 104,
                    signedIn: snap.isSignedIn,
                    lifted: true
                )
                if snap.loading || snap.signingIn {
                    ProgressView()
                        .controlSize(.small)
                        .padding(5)
                        .background(.bar, in: Circle())
                        .offset(x: 2, y: 2)
                }
            }
            VStack(spacing: 4) {
                Text(snap.isSignedIn ? snap.displayName : "GitHub Account")
                    .font(.system(size: 22, weight: .semibold))
                if snap.isSignedIn {
                    Text(snap.subtitle)
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                } else if snap.signingIn, !snap.deviceCode.isEmpty {
                    Text("Code \(snap.deviceCode)")
                        .font(.system(size: 13, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                } else {
                    Text(signedOutCaption)
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 320)
                }
            }
        }
        .frame(maxWidth: .infinity)
    }

    private var signedOutCaption: String {
        if snap.isMissingCLI {
            return "Install the GitHub CLI to sign in."
        }
        if snap.signingIn {
            return "Finish signing in with the browser."
        }
        return "Sign in to browse, review, and push with your GitHub identity."
    }

    @ViewBuilder
    private var signedOutGroup: some View {
        VStack(alignment: .leading, spacing: 8) {
            accountGroup {
                if snap.isMissingCLI {
                    accountAction("Install GitHub CLI…", action: onInstall)
                    groupDivider
                    accountAction("Refresh", action: onRefresh)
                } else if snap.signingIn {
                    if !snap.deviceCode.isEmpty {
                        HStack {
                            Text("One-time code")
                            Spacer()
                            Text(snap.deviceCode).font(.body.monospaced())
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        groupDivider
                    }
                    accountAction("Cancel Sign In", action: onCancel)
                } else {
                    accountAction("Sign In with Browser…", action: onSignIn)
                    groupDivider
                    accountAction("Refresh Status", action: onRefresh)
                }
            }
            Text("Suisei uses the GitHub CLI already on this Mac. Credentials stay in the system keychain; the editor never stores a token of its own.")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 4)
        }
    }

    private var groupDivider: some View {
        Divider().padding(.leading, 44)
    }

    private func accountGroup<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        VStack(spacing: 0, content: content)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(SettingsGroupFill.color(for: colorScheme))
            )
    }

    private func accountAction(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var footer: some View {
        HStack(alignment: .center) {
            if snap.isSignedIn {
                Button("Sign Out…", action: onSignOut)
            }
            Spacer(minLength: 0)
            Button(action: onHelp) {
                Image(systemName: "questionmark.circle")
                    .font(.system(size: 16, weight: .regular))
                    .foregroundStyle(.secondary)
                    .symbolRenderingMode(.hierarchical)
            }
            .buttonStyle(.plain)
            .help("GitHub authentication help")
        }
    }

    private func accountRow(
        _ symbol: String,
        _ title: String,
        _ value: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: symbol)
                    .font(.system(size: 16, weight: .regular))
                    .foregroundStyle(.secondary)
                    .symbolRenderingMode(.hierarchical)
                    .frame(width: 22, alignment: .center)
                Text(title)
                    .foregroundStyle(.primary)
                Spacer(minLength: 8)
                if !value.isEmpty {
                    Text(value)
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Image(systemName: "chevron.forward")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityValue(value)
    }

    private func count(_ n: UInt32) -> String {
        n.formatted()
    }
}

struct GitHubAccountProfilePage: View {
    @ObservedObject var store: GitHubAccountStore
    var onOpenGitHub: () -> Void

    private var snap: GitHubAccountSnap { store.snap }

    var body: some View {
        Form {
            Section {
                HStack(spacing: 14) {
                    GitHubAvatarView(image: store.avatar, size: 52, signedIn: true)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(snap.displayName).fontWeight(.medium)
                        Text(snap.login).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.vertical, 4)
            }
            Section {
                LabeledContent("Name", value: blank(snap.name))
                LabeledContent("Username", value: blank(snap.login))
                LabeledContent("Email", value: blank(snap.email))
                if !snap.bio.isEmpty {
                    LabeledContent("Bio", value: snap.bio)
                }
                if !snap.company.isEmpty {
                    LabeledContent("Company", value: snap.company)
                }
                if !snap.location.isEmpty {
                    LabeledContent("Location", value: snap.location)
                }
            } footer: {
                Text("These facts come from GitHub. Change them on github.com.")
            }
            Section {
                Button("Open Profile on GitHub", action: onOpenGitHub)
            }
        }
        .formStyle(.grouped)
    }

    private func blank(_ value: String) -> String {
        value.isEmpty ? "—" : value
    }
}

struct GitHubAccountSecurityPage: View {
    @ObservedObject var store: GitHubAccountStore
    var onSetupGit: () -> Void
    var onRefresh: () -> Void

    private var snap: GitHubAccountSnap { store.snap }

    var body: some View {
        Form {
            Section {
                LabeledContent("Host", value: blank(snap.host, fallback: "github.com"))
                LabeledContent("Git protocol", value: blank(snap.protocolName, fallback: "https"))
                LabeledContent("Token source", value: blank(snap.tokenSource))
            } header: {
                Text("Authentication")
            } footer: {
                Text("The GitHub CLI stores the token in the macOS keychain. Suisei never copies it.")
            }
            Section {
                LabeledContent("Scopes", value: blank(snap.scopes))
            } footer: {
                Text("Scopes decide what Suisei can do on your behalf — repositories, organisations, gists.")
            }
            Section {
                Button("Configure Git Credentials", action: onSetupGit)
                Button("Refresh Status", action: onRefresh)
            }
        }
        .formStyle(.grouped)
    }

    private func blank(_ value: String, fallback: String = "—") -> String {
        value.isEmpty ? fallback : value
    }
}

/// Past-year contribution grid. GitHub's page is a green spreadsheet;
/// this is the System Settings reading of the same facts: one grouped
/// card, caption typography, and the window accent instead of grass.
struct GitHubContributionCard: View {
    @Environment(\.colorScheme) private var colorScheme
    var total: UInt32
    var levels: [UInt8]
    var start: String
    var year: UInt32
    var yearMin: UInt32
    var accent: Color
    var onSelectYear: (UInt32) -> Void

    private let rows = 7
    private var weeks: Int { max(1, (levels.count + rows - 1) / rows) }
    private var currentYear: UInt32 {
        UInt32(Calendar.current.component(.year, from: Date()))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Text("\(total.formatted()) contributions")
                    .font(.system(size: 13, weight: .medium))
                Spacer(minLength: 8)
                yearMenu
            }
            GitHubContributionGrid(levels: levels, start: start, accent: accent)
            HStack(spacing: 4) {
                Spacer(minLength: 0)
                Text("Less")
                ForEach(0..<5, id: \.self) { level in
                    RoundedRectangle(cornerRadius: 2, style: .continuous)
                        .fill(swatch(UInt8(level)))
                        .frame(width: 8, height: 8)
                }
                Text("More")
            }
            .font(.system(size: 10))
            .foregroundStyle(.secondary)
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(SettingsGroupFill.color(for: colorScheme))
        )
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(total.formatted()) contributions")
    }

    /// Native macOS pop-up menu (checkmark + system list), not a custom sheet.
    private var yearMenu: some View {
        let earliest = yearMin == 0 ? max(2008, currentYear - 10) : min(yearMin, currentYear)
        return Picker("Year", selection: Binding(
            get: { year },
            set: { onSelectYear($0) }
        )) {
            Text("Past year").tag(UInt32(0))
            ForEach(Array(stride(from: Int(currentYear), through: Int(earliest), by: -1)), id: \.self) { y in
                Text(String(y)).tag(UInt32(y))
            }
        }
        .pickerStyle(.menu)
        .labelsHidden()
        .fixedSize()
        .controlSize(.small)
    }

    private func swatch(_ level: UInt8) -> Color {
        switch level {
        case 1: return accent.opacity(0.28)
        case 2: return accent.opacity(0.48)
        case 3: return accent.opacity(0.72)
        case 4: return accent
        default: return Color.primary.opacity(0.06)
        }
    }
}

private struct GitHubContributionGrid: View {
    var levels: [UInt8]
    var start: String
    var accent: Color

    private let rows = 7
    private var weeks: Int { max(1, (levels.count + rows - 1) / rows) }

    @State private var gridHeight: CGFloat = 90

    var body: some View {
        GeometryReader { geo in
            let layout = Self.layout(width: geo.size.width, weeks: weeks)
            Canvas { context, size in
                drawMonths(context, width: size.width, cell: layout.cell, gap: layout.gap)
                let gridTop = layout.labelH + 3
                for week in 0..<weeks {
                    for day in 0..<rows {
                        let index = week * rows + day
                        let rect = CGRect(
                            x: CGFloat(week) * (layout.cell + layout.gap),
                            y: gridTop + CGFloat(day) * (layout.cell + layout.gap),
                            width: layout.cell,
                            height: layout.cell
                        )
                        let path = RoundedRectangle(cornerRadius: 2, style: .continuous).path(in: rect)
                        let level = index < levels.count ? levels[index] : 0
                        context.fill(path, with: .color(swatch(level)))
                    }
                }
            }
            .onAppear { gridHeight = Self.height(for: layout) }
            .onChange(of: geo.size.width) { _, width in
                gridHeight = Self.height(for: Self.layout(width: width, weeks: weeks))
            }
        }
        .frame(height: gridHeight)
        .accessibilityHidden(true)
    }

    private static func height(for layout: (cell: CGFloat, gap: CGFloat, labelH: CGFloat)) -> CGFloat {
        layout.labelH + 3 + layout.cell * 7 + layout.gap * 6
    }

    /// Floor the cell so `weeks * cell + (weeks-1) * gap` cannot exceed the
    /// canvas. A fractional cell was drawing the last August column past the
    /// clip. Month labels that would hang off the right are skipped.
    private static func layout(width: CGFloat, weeks: Int) -> (cell: CGFloat, gap: CGFloat, labelH: CGFloat) {
        let gap: CGFloat = 2
        let weeks = max(weeks, 1)
        let raw = (width - 1 - gap * CGFloat(weeks - 1)) / CGFloat(weeks)
        return (max(3, floor(raw)), gap, 13)
    }

    private func drawMonths(_ context: GraphicsContext, width: CGFloat, cell: CGFloat, gap: CGFloat) {
        guard let origin = Self.parseDay(start) else { return }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.locale = .current
        formatter.dateFormat = "MMM"

        var lastMonth = 0
        for week in 0..<weeks {
            guard let date = calendar.date(byAdding: .day, value: week * rows, to: origin) else {
                continue
            }
            let month = calendar.component(.month, from: date)
            if month == lastMonth { continue }
            lastMonth = month
            let x = CGFloat(week) * (cell + gap)
            if x + 22 > width { continue }
            let label = Text(formatter.string(from: date))
                .font(.system(size: 10))
                .foregroundColor(.secondary)
            context.draw(label, at: CGPoint(x: x, y: 0), anchor: .topLeading)
        }
    }

    private func swatch(_ level: UInt8) -> Color {
        switch level {
        case 1: return accent.opacity(0.28)
        case 2: return accent.opacity(0.48)
        case 3: return accent.opacity(0.72)
        case 4: return accent
        default: return Color.primary.opacity(0.06)
        }
    }

    private static func parseDay(_ raw: String) -> Date? {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.date(from: raw)
    }
}
