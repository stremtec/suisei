import AppKit
import SwiftUI

struct SoftwareUpdateSnap: Equatable {
    var generation: UInt64 = 0
    var available = false
    var installing = false
    var installed = false
    var checking = false
    var current = ""
    var latest = ""
    var notes = ""

    static let empty = SoftwareUpdateSnap()
}

final class SoftwareUpdateStore: ObservableObject {
    @Published private(set) var snap = SoftwareUpdateSnap.empty
    /// Local presentation only — Core does not ship a beta channel yet.
    @AppStorage("suisei.betaUpdates") var betaUpdates = false

    func publish(_ next: SoftwareUpdateSnap) {
        if next != snap { snap = next }
    }
}

enum SuiseiBuild {
    /// `Suisei2026dev` + short git hash, baked into the bundle at package time.
    static var installedName: String {
        (Bundle.main.object(forInfoDictionaryKey: "SuiseiBuildName") as? String)
            ?? "Suisei2026dev416ad08"
    }
}

/// System Settings → Software Update.
///
/// The page used to say one thing in every state: a red "This is not a valid
/// release." under the installed build, and a footer explaining that Suisei
/// "cannot install this update" — with no update in sight. Both sentences came
/// from `UpdateState::start_install`, which is gated until Suisei publishes
/// signed snapshots. That is a fact about INSTALLING, and it was being printed
/// as a verdict on the build the user is running.
///
/// It also had no Check Now. `suisei_engine_update_check` and its bridge method
/// existed and nothing in the app called either, so the only check that ever
/// ran was the throttled one at launch.
struct SoftwareUpdatePage: View {
    @ObservedObject var store: SoftwareUpdateStore
    var automaticUpdates: SettingsRowItem?
    var onOpenAutomatic: () -> Void
    var onOpenBeta: () -> Void
    var onCheckNow: () -> Void
    var onInfo: () -> Void

    private var snap: SoftwareUpdateSnap { store.snap }
    private var automaticOn: Bool { automaticUpdates?.valueIndex != 0 }

    // Shaped after System Settings → General → Software Update, which is the
    // page a Mac user has already read. Three things there that were not here:
    //
    //  · **No tinted glyphs on the rows.** Apple's coloured rounded-square
    //    icons live in the SIDEBAR, where they are the way you find a page in a
    //    list of thirty. Inside a page the rows carry a title, a value and an
    //    ⓘ, and nothing else — the colour would be decorating something the eye
    //    is already on. Ours had a blue clock and an orange hammer, and next to
    //    the real thing they read as a toy.
    //  · **A blocking fact is a status line, not a footnote.** "Cannot be
    //    downloaded" sits under the title in red, where the thing it is about
    //    is; the fix for it gets its own row with its own button. Ours put the
    //    equivalent sentence in the section footer, below everything, where it
    //    read as a disclaimer rather than the reason the button will not help.
    //  · **The trailing affordance is ⓘ, not a chevron.** These rows explain
    //    rather than drill in, and Apple marks that difference.
    var body: some View {
        Form {
            Section {
                statusRow
                if snap.available, !snap.notes.isEmpty {
                    Text(snap.notes)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.vertical, 2)
                }
                if snap.available {
                    blockedRow
                }
            }

            Section {
                UpdateRow(
                    title: "Automatic Updates",
                    value: automaticOn ? "On" : "Off",
                    action: onOpenAutomatic
                )
                UpdateRow(
                    title: "Beta Updates",
                    value: store.betaUpdates ? "On" : "Off",
                    action: onOpenBeta
                )
                UpdateRow(title: "Installed Build", value: SuiseiBuild.installedName)
                UpdateRow(title: "Engine", value: versionLabel)
            } footer: {
                Text("Suisei checks for a newer GitHub release; it never sends anything about your files.")
            }
        }
        .formStyle(.grouped)
    }

    private var statusRow: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .interpolation(.high)
                .frame(width: 48, height: 48)

            VStack(alignment: .leading, spacing: 2) {
                Text(headline)
                    .font(.body)
                Text(subhead)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                // Under the title, in the colour of the thing it is about —
                // the same place macOS puts "다운로드할 수 없음".
                if snap.available {
                    Text("Cannot be installed from here")
                        .font(.subheadline)
                        .foregroundStyle(Color(nsColor: .systemRed))
                }
            }

            Spacer(minLength: 8)

            trailingControl
        }
        .padding(.vertical, 4)
    }

    /// Why the button above will not finish the job, and the way that does.
    private var blockedRow: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Color(nsColor: .systemYellow))
                .font(.system(size: 14))
            VStack(alignment: .leading, spacing: 2) {
                Text("Signed in-place updates are not published yet")
                    .font(.subheadline.weight(.medium))
                Text("Download this release from GitHub and replace the app.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            Button("Open Release…", action: onInfo)
        }
        .padding(.vertical, 2)
    }

    @ViewBuilder private var trailingControl: some View {
        if snap.checking {
            ProgressView()
                .controlSize(.small)
                .padding(.trailing, 4)
        } else if snap.available {
            Button("Update Now", action: onInfo)
                .buttonStyle(.borderedProminent)
        } else {
            Button("Check Now", action: onCheckNow)
        }
    }

    /// What is true right now, in the order the states can actually occur.
    private var headline: String {
        if snap.installed { return "Restart to finish updating" }
        if snap.installing { return "Installing…" }
        if snap.checking { return "Checking for Updates…" }
        if snap.available { return "Update Available" }
        return "Suisei is up to date"
    }

    private var subhead: String {
        if snap.installed { return "Quit and reopen Suisei to load the installed version." }
        if snap.checking { return SuiseiBuild.installedName }
        if snap.available {
            let version = snap.latest.isEmpty ? "A newer release" : "Version \(snap.latest)"
            return "\(version) is available — you have \(versionLabel)."
        }
        return "\(SuiseiBuild.installedName) is the latest version."
    }

    private var versionLabel: String {
        if !snap.current.isEmpty { return snap.current }
        return EngineBridge.engineVersion
    }

}

/// A row on this page: a title, what it says, and — when there is somewhere to
/// go — an ⓘ.
///
/// No leading glyph, deliberately. Apple's tinted rounded squares belong to the
/// SIDEBAR, where they are how you find one page among thirty; inside a page
/// every row is already the thing you are looking at, and a colour there is
/// decoration standing where information goes. `SettingsNavigationRow` keeps
/// its icons because it is used in lists that need them.
private struct UpdateRow: View {
    var title: String
    var value: String
    /// Absent = the row states a fact and does nothing.
    var action: (() -> Void)?

    var body: some View {
        if let action {
            Button(action: action) { content(navigable: true) }
                .buttonStyle(.plain)
                .accessibilityLabel(title)
                .accessibilityValue(value)
        } else {
            content(navigable: false)
        }
    }

    private func content(navigable: Bool) -> some View {
        HStack(spacing: 8) {
            Text(title).foregroundStyle(.primary)
            Spacer(minLength: 8)
            Text(value)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            if navigable {
                // ⓘ, not a chevron: these rows explain rather than drill in,
                // and System Settings marks that difference.
                Image(systemName: "info.circle")
                    .foregroundStyle(.secondary)
            }
        }
        .contentShape(Rectangle())
    }
}

struct SoftwareUpdateAutomaticPage: View {
    var automaticUpdates: SettingsRowItem?
    var onSetAutomatic: (Bool) -> Void

    private var isOn: Bool { automaticUpdates?.valueIndex != 0 }

    var body: some View {
        Form {
            Section {
                Toggle("Automatically keep Suisei up to date", isOn: Binding(
                    get: { isOn },
                    set: onSetAutomatic
                ))
            } footer: {
                Text("When this is on, Suisei asks GitHub for the latest release at launch, at most once every four hours. Check Now ignores that interval.")
            }
        }
        .formStyle(.grouped)
    }
}

struct SoftwareUpdateBetaPage: View {
    @ObservedObject var store: SoftwareUpdateStore

    var body: some View {
        Form {
            Section {
                Picker("Beta Updates", selection: $store.betaUpdates) {
                    Text("Off").tag(false)
                    Text("Suisei Developer Beta").tag(true)
                }
                .pickerStyle(.inline)
                .labelsHidden()
            } footer: {
                Text("Developer betas are not offered yet. Turning this on only records the preference.")
            }
        }
        .formStyle(.grouped)
    }
}
