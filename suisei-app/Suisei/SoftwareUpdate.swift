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
    /// Build the tagged release on this machine and stage it for next launch.
    var onUpdate: () -> Void = {}
    /// 0 idle · 1 cloning · 2 building · 3 staging · 4 ready · 5 failed.
    var buildPhase: UInt8 = 0
    var buildDetail: String = ""
    /// 0…1, and how much longer. See `update_build::BuildProgress` for where
    /// the number comes from and which parts of it are counted rather than
    /// estimated.
    var buildFraction: Double = 0
    var buildETA: Int? = nil
    var buildHeadline: String = ""

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
                if buildPhase > 0 {
                    buildRow
                } else if snap.available {
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


    /// What the build is doing, once one is running.
    ///
    /// The last line it printed, not a percentage. A source build runs for tens
    /// of minutes with no reliable notion of how far along it is, and a bar
    /// that does not move is indistinguishable from a hang — where "Compiling
    /// suisei-core" plainly is not.
    @ViewBuilder private var buildRow: some View {
        if buildPhase > 0 {
            HStack(alignment: .top, spacing: 10) {
                if buildPhase < 4 {
                    EmptyView()
                } else {
                    Image(systemName: buildPhase == 4
                          ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                        .foregroundStyle(Color(nsColor: buildPhase == 4 ? .systemGreen : .systemRed))
                }
                VStack(alignment: .leading, spacing: 4) {
                    Text(buildPhase < 4 && !buildHeadline.isEmpty ? buildHeadline : headlineForPhase)
                        .font(.subheadline.weight(.medium))
                    // A determinate bar, because the number under it is real:
                    // the engine step counts `Cargo.lock`'s packages as cargo
                    // compiles them. Where nothing can be counted the bar holds
                    // still rather than creeping on a timer — a bar that
                    // arrives before the work does is the reason people stop
                    // believing bars.
                    if buildPhase < 4 {
                        ProgressView(value: min(max(buildFraction, 0), 1))
                            .progressViewStyle(.linear)
                            .frame(maxWidth: 320)
                        Text(progressLine)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .monospacedDigit()
                    }
                    if !buildDetail.isEmpty {
                        Text(buildDetail)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(3)
                            .fixedSize(horizontal: false, vertical: true)
                            .textSelection(.enabled)
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 2)
        }
    }

    /// "34% · about 12 minutes remaining", or just the percentage while there
    /// is nothing honest to say about the time.
    private var progressLine: String {
        let pct = Int((min(max(buildFraction, 0), 1) * 100).rounded())
        guard let eta = buildETA else { return "\(pct)%" }
        if eta < 90 { return "\(pct)% · less than a minute remaining" }
        let mins = Int((Double(eta) / 60).rounded())
        return "\(pct)% · about \(mins) minute\(mins == 1 ? "" : "s") remaining"
    }

    private var headlineForPhase: String {
        switch buildPhase {
        case 1: return "Downloading the source…"
        case 2: return "Building. This takes a while — you can keep working."
        case 3: return "Almost done…"
        case 4: return "Ready. Quit and reopen Suisei to finish updating."
        case 5: return "The update did not finish. Suisei is unchanged."
        default: return ""
        }
    }

    /// Why the button above will not finish the job, and the way that does.
    private var blockedRow: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Color(nsColor: .systemYellow))
                .font(.system(size: 14))
            VStack(alignment: .leading, spacing: 2) {
                Text("Updating builds Suisei on this Mac")
                    .font(.subheadline.weight(.medium))
                Text("Suisei is not signed, so a downloaded build would ask macOS for permission every time. Building the tagged source here does not. It needs Rust and Xcode's tools, and takes a while.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
        }
        .padding(.vertical, 2)
    }

    @ViewBuilder private var trailingControl: some View {
        if snap.checking {
            ProgressView()
                .controlSize(.small)
                .padding(.trailing, 4)
        } else if snap.available {
            // "Update Now" is what this said, and it opens a web page. The row
            // directly below it says the update CANNOT be installed from here —
            // so the button was contradicting the sentence explaining it.
            //
            // It is also the only control now: the blocked row used to carry a
            // second button doing the identical thing under a different name.
            // A blocking fact is a status line, and a status line does not need
            // its own button when the action is the prominent one above it.
            Button("Update Now", action: onUpdate)
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
