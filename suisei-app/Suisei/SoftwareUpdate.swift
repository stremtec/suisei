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
            } footer: {
                // Only where it is true: when there IS something to install.
                if snap.available {
                    Text("Suisei does not publish signed in-place updates yet. Download this release from GitHub and replace the app.")
                }
            }

            Section {
                settingsLink(
                    "Automatic Updates",
                    value: automaticOn ? "On" : "Off",
                    action: onOpenAutomatic
                )
                settingsLink(
                    "Beta Updates",
                    value: store.betaUpdates ? "On" : "Off",
                    action: onOpenBeta
                )
            }

            Section {
                LabeledContent("Build", value: SuiseiBuild.installedName)
                LabeledContent("Engine", value: versionLabel)
            } footer: {
                Text("Suisei checks for a newer GitHub release; it never sends anything about your files.")
            }
        }
        .formStyle(.grouped)
    }

    private var statusRow: some View {
        HStack(alignment: .center, spacing: 12) {
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
            }

            Spacer(minLength: 8)

            trailingControl
        }
        .padding(.vertical, 4)
    }

    @ViewBuilder private var trailingControl: some View {
        if snap.checking {
            ProgressView()
                .controlSize(.small)
                .padding(.trailing, 4)
        } else if snap.available {
            Button("Release Notes…", action: onInfo)
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

    private func settingsLink(_ title: String, value: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack {
                Text(title)
                    .foregroundStyle(.primary)
                Spacer()
                Text(value)
                    .foregroundStyle(.secondary)
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
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
