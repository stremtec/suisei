import AppKit
import SwiftUI

/// Settings → Components.
///
/// Xcode's Components page, answered honestly. The measurement behind it
/// (`docs/SUISEI-COMPONENTS-PLAN.md`) moved the answer a long way from where the
/// question started: **the debugger is 0 MB.** Rust, C and C++ debug through
/// `lldb-dap`, which ships inside Xcode; Python, Go and Node through `debugpy`,
/// `dlv` and `js-debug-adapter`, which come from pip, go and npm. Hosting our
/// own copies would mean shipping a second debugger beside the one Apple
/// already updates, and taking on its CVEs. The same argument covers language
/// servers.
///
/// So the component for debugging is not a binary. It is **finding what is
/// there, and helping install what is not** — which is what this page does, and
/// it is the half of the feature that works today. Downloading a grammar into
/// the process needs a signed, notarized release first; detection needs none.
///
/// Two rules carried over from the Software Update page, which is the other
/// page in this window shaped after System Settings:
///
///  · **A blocking fact is a status line, not a footnote.** The reason nothing
///    can be downloaded sits at the top, where the thing it is about is, rather
///    than in a section footer below everything where it reads as a disclaimer.
///  · **No tinted glyphs on the rows.** Apple's coloured rounded squares belong
///    to the SIDEBAR, where they are how you find one page among thirty. Inside
///    a page every row is already the thing you are looking at.
struct ComponentsPage: View {
    // No engine. This page reports on the MACHINE, not on the document — the
    // probe takes no engine pointer, and holding one here would suggest it did.
    @State private var items: [ComponentItem] = []
    @State private var blockedReason = ""
    @State private var loading = true
    @State private var copied: String?

    private var groups: [String] { ["Debugging", "Language Servers", "Included"] }

    var body: some View {
        Form {
            Section {
                headerRow
                if !blockedReason.isEmpty {
                    blockedRow
                }
            }

            ForEach(groups, id: \.self) { group in
                let rows = items.filter { $0.group == group }
                if !rows.isEmpty {
                    Section {
                        ForEach(rows) { item in
                            ComponentRow(item: item, copied: $copied) {
                                // An install that finished changes the answer to
                                // the question this page exists to ask, so ask
                                // it again rather than assuming the command did
                                // what it said.
                                Task { await reload() }
                            }
                        }
                    } header: {
                        Text(group)
                    } footer: {
                        if let note = footer(for: group) {
                            Text(note)
                        }
                    }
                }
            }
        }
        .formStyle(.grouped)
        // Off the main thread on purpose: probing asks the machine, and one of
        // the probes starts a Python interpreter (`debugpy` ships no console
        // script, so importing it is the only honest test). On arrival, once.
        .task {
            await reload()
        }
    }

    private var headerRow: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Components")
                    .font(.body)
                Text(summary)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            if loading {
                ProgressView()
                    .controlSize(.small)
                    .padding(.trailing, 4)
            } else {
                Button("Refresh") {
                    Task { await reload() }
                }
            }
        }
        .padding(.vertical, 4)
    }

    /// Why the page has nothing to install, in the place the fact belongs.
    private var blockedRow: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Color(nsColor: .systemYellow))
                .font(.system(size: 14))
            Text(blockedReason)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.vertical, 2)
    }

    private var summary: String {
        if loading { return "Checking what this Mac has…" }
        let tools = items.filter { $0.group != "Included" }
        let have = tools.filter(\.isPresent).count
        return "\(have) of \(tools.count) external tools found on this Mac."
    }

    private func footer(for group: String) -> String? {
        switch group {
        case "Debugging":
            return "Suisei uses the debug adapter your language already has, so security updates come from the same place the language does."
        case "Language Servers":
            return "The command each language starts is set in Settings → Language Servers. This page only reports whether it is here."
        case "Included":
            return "Built into the app. Works with no network and nothing to install."
        default:
            return nil
        }
    }

    private func reload() async {
        loading = true
        // Detached and genuinely off-main: `ComponentProbe` takes no engine
        // pointer, so there is no App state to race with. Hopping back to the
        // main actor via `MainActor.run` here would have been the same work on
        // the same thread, wearing a different hat.
        let fetched = await Task.detached(priority: .userInitiated) {
            (ComponentProbe.scan(), ComponentProbe.blockedReason())
        }.value
        items = fetched.0
        blockedReason = fetched.1
        loading = false
    }
}

/// Runs a component's install line in the background.
///
/// A background job and not the terminal, because an install is a chore, not a
/// session: the user pressed a button on a settings page and wants to carry on
/// reading it while the download happens. The terminal is still one press away
/// when it is needed — see the failure row — and that is where the answer to
/// "it wants a password" or "the whole log, please" lives.
///
/// A **login** shell (`-l`), not `Process` on the binary directly. An app
/// launched from Finder inherits `/usr/bin:/bin:/usr/sbin:/sbin` and nothing
/// else — no Homebrew, no cargo, no npm prefix, none of the version managers —
/// so `brew`/`pip3`/`go` would not even be found. This is the same reasoning
/// `exec::login_shell_path` documents on the core side.
@MainActor
final class ComponentInstaller: ObservableObject {
    static let shared = ComponentInstaller()

    enum Progress: Equatable {
        case running
        case failed(String)
    }

    /// Keyed by component id. A finished-and-successful install leaves NO entry:
    /// the row's own probe says "Installed", and two sources for one fact is
    /// how they come to disagree.
    @Published private(set) var progress: [String: Progress] = [:]

    private init() {}

    func install(_ id: String, command: String, onFinish: @escaping () -> Void) {
        guard !command.isEmpty, progress[id] != .running else { return }
        progress[id] = .running
        Task.detached(priority: .utility) {
            let failure = Self.run(command)
            await MainActor.run {
                if let failure {
                    self.progress[id] = .failed(failure)
                } else {
                    self.progress.removeValue(forKey: id)
                }
                onFinish()
            }
        }
    }

    /// Nil on success; the useful end of the output on failure.
    private nonisolated static func run(_ command: String) -> String? {
        let shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        let proc = Process()
        proc.executableURL = URL(
            fileURLWithPath: FileManager.default.isExecutableFile(atPath: shell)
                ? shell : "/bin/zsh"
        )
        proc.arguments = ["-l", "-c", command]
        let pipe = Pipe()
        proc.standardOutput = pipe
        proc.standardError = pipe
        // No tty: anything that insists on one (a `sudo` password) fails here
        // rather than hanging forever waiting on a prompt nobody can see.
        proc.standardInput = FileHandle.nullDevice
        do {
            try proc.run()
        } catch {
            return error.localizedDescription
        }
        // Drained BEFORE the wait. A pipe has a finite buffer, and an installer
        // that prints more than it holds would block writing while we blocked
        // waiting — the classic deadlock, and `npm i -g` prints plenty.
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        proc.waitUntilExit()
        guard proc.terminationStatus != 0 else { return nil }
        let text = String(decoding: data, as: UTF8.self)
        let lines = text
            .split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        let tail = lines.suffix(2).joined(separator: " · ")
        if tail.isEmpty { return "Exit code \(proc.terminationStatus)." }
        return tail.count > 240 ? String(tail.prefix(240)) + "…" : tail
    }
}

/// One component: what it is, whether it is here, and the line that gets it.
private struct ComponentRow: View {
    var item: ComponentItem
    @Binding var copied: String?
    var onInstalled: () -> Void
    @ObservedObject private var installer = ComponentInstaller.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(item.title)
                    .foregroundStyle(.primary)
                Spacer(minLength: 8)
                statusLabel
            }
            if !item.detail.isEmpty {
                Text(item.detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // WHERE it was found, because which copy answered is the useful
            // half of "installed" — a developer Mac routinely has three
            // `clangd`s and a stale one first.
            if case .present(let path) = item.state, !path.isEmpty {
                Text(path)
                    .font(.caption.monospaced())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
            }
            if item.state == .missing, !item.install.isEmpty {
                installRow
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(item.title)
        .accessibilityValue(statusText)
    }

    @ViewBuilder private var statusLabel: some View {
        switch item.state {
        case .bundled:
            Text("Included")
                .foregroundStyle(.secondary)
        case .present:
            HStack(spacing: 4) {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(Color(nsColor: .systemGreen))
                Text("Installed")
                    .foregroundStyle(.secondary)
            }
        case .missing:
            Text("Not Installed")
                .foregroundStyle(Color(nsColor: .secondaryLabelColor))
        }
    }

    private var statusText: String {
        switch item.state {
        case .bundled: return "Included"
        case .present(let p): return "Installed at \(p)"
        case .missing: return "Not installed"
        }
    }

    /// The command, a button that runs it, and a button that copies it.
    ///
    /// **Install runs it in the docked terminal**, not through a pipe with a
    /// spinner over it. These lines install software globally with the user's
    /// own toolchain — `pip3 install`, `go install`, `npm i -g` — and any of
    /// them can ask for a password, refuse under PEP 668, or print a conflict
    /// only a human can settle. A terminal is where all three are visible and
    /// answerable, and the transcript is still there afterwards. It is one
    /// press either way; what it is not is hidden.
    ///
    /// Copy stays, because reading a command before running it is a legitimate
    /// thing to want, and because the shell you trust may not be this one.
    private var installRow: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(item.install)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 8)
                if installer.progress[item.id] == .running {
                    ProgressView()
                        .controlSize(.small)
                    Text("Installing…")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Button("Install") {
                        ComponentInstaller.shared.install(
                            item.id, command: item.install, onFinish: onInstalled
                        )
                    }
                    .controlSize(.small)
                    .help("Download and install this in the background")
                }
                Button(copied == item.id ? "Copied" : "Copy") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(item.install, forType: .string)
                    copied = item.id
                }
                .controlSize(.small)
            }
            // What the installer actually said, and a way to watch it say more.
            // A background job that fails silently is worse than one that was
            // never offered; and the reasons these fail — a password, a version
            // conflict, PEP 668 — are exactly the ones a person has to see in
            // full and answer, which is what the terminal is for.
            if case .failed(let why)? = installer.progress[item.id] {
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(Color(nsColor: .systemYellow))
                    Text(why)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                    Spacer(minLength: 8)
                    Button("Open in Terminal") {
                        EngineBridge.shared.runInDockTerminal(item.install)
                    }
                    .controlSize(.small)
                }
            }
        }
        .padding(.top, 1)
    }
}
