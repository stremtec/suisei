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
struct SoftwareUpdatePage: View {
    @ObservedObject var store: SoftwareUpdateStore
    var automaticUpdates: SettingsRowItem?
    var onOpenAutomatic: () -> Void
    var onOpenBeta: () -> Void
    var onInfo: () -> Void

    private var snap: SoftwareUpdateSnap { store.snap }
    private var automaticOn: Bool { automaticUpdates?.valueIndex != 0 }

    var body: some View {
        Form {
            Section {
                statusRow
            } footer: {
                Text("Suisei cannot install this update because it is not a signed Suisei release.")
            }

            Section {
                LabeledContent("Installed", value: SuiseiBuild.installedName)
            } header: {
                Text("Installed")
            }

            Section {
                settingsLink("Automatic Updates", value: automaticOn ? "On" : "Off", action: onOpenAutomatic)
                settingsLink("Beta Updates", value: store.betaUpdates ? "On" : "Off", action: onOpenBeta)
            }
        }
        .formStyle(.grouped)
    }

    private var statusRow: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .interpolation(.high)
                .frame(width: 44, height: 44)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

            VStack(alignment: .leading, spacing: 2) {
                Text(SuiseiBuild.installedName)
                    .font(.body)
                Text(versionLabel)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                Text("This is not a valid release.")
                    .font(.subheadline)
                    .foregroundStyle(.red)
            }

            Spacer(minLength: 8)

            if snap.checking {
                ProgressView().controlSize(.small)
            }

            Button(action: onInfo) {
                Image(systemName: "info.circle")
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .symbolRenderingMode(.hierarchical)
            }
            .buttonStyle(.borderless)
            .help("About this version")
        }
        .padding(.vertical, 4)
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
                Text("When this is on, Suisei looks for a GitHub Release at launch. Install is disabled until a signed Suisei snapshot exists.")
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
