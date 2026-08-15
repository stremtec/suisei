import Foundation
import SwiftUI

/// Facts about a Suisei project that more than one view needs.
enum SuiseiProject {
    /// The file that says "this folder is a project".
    ///
    /// `suisei_core::project::MARKER` is the authority — core creates the file
    /// and core decides what a project is. This is the face's copy, named once
    /// so the two views that have to recognise the file are comparing against
    /// the same string rather than each against its own literal.
    static let marker = "project.suiseiprj"
}

// MARK: - File-private constants (implicitly Sendable — safe to read from any
// concurrency domain without @MainActor isolation).

/// Source files only. Binaries, images, archives and media have nothing to
/// parse, and walking them would just burn IO.
/// Kept in step with `suisei-core/src/lang.rs`, which is the authority on what
/// the parser can build a tree for. A file that is not listed here is not
/// pre-parsed, so the first time it is opened it pays a cold parse the indexer
/// could have absorbed — invisible until the file is long.
private let _codeExtensions: Set<String> = [
    "swift", "rs", "c", "h", "cpp", "hpp", "cc", "cxx", "hh", "hxx",
    "c++", "h++", "ipp", "tpp", "inl", "m", "mm",
    "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "py", "pyi", "pyw",
    "go", "rb", "rake", "gemspec", "ru", "java", "kt", "kts",
    "scala", "sc", "sbt", "cs", "csx", "php", "phtml",
    "sh", "bash", "zsh", "ksh",
    "lua", "dart", "zig", "nim", "ex", "exs", "hs", "sql", "json", "jsonc",
    "toml", "yaml", "yml", "md", "markdown",
    "html", "htm", "xhtml", "css", "xml", "xsd", "xsl", "xslt", "plist",
    "cmake",
]

/// Directories that are never worth indexing.
private let _skippedDirectories: Set<String> = [
    ".git", "node_modules", "target", "build", "dist", ".build", "DerivedData",
    "Pods", "vendor", ".venv", "venv", "__pycache__", ".next", ".cache",
]

/// A file this large is more likely generated than edited; parsing it up
/// front costs more than it saves.
private let _maxBytes = 4_000_000

/// Auto-indexer for the project's master directory.
///
/// Opening a file costs one cold tree-sitter parse (~15ms at 6k lines); edits
/// after that are incremental and cheap. So the lag on long files is the FIRST
/// touch, and the fix is to pay it up front, in the background — biggest files
/// first, because those are the ones that would otherwise stall.
@MainActor
final class ProjectIndex: ObservableObject {
    /// Files already warmed, by absolute path.
    @Published private(set) var indexed: Set<String> = []
    /// Files that could not be read (permissions, not text, vanished).
    @Published private(set) var failed: Set<String> = []
    /// Master directories, in the order they were set.
    ///
    /// Plural. It was one path for the whole app, so setting a second project
    /// silently forgot the first — and a machine has more than one project on
    /// it. What stays forbidden is NESTING: a master inside another master
    /// would index the same files twice and give one folder two roots.
    @Published private(set) var masters: [String] = []
    /// The most recently set one, for anything that still wants a single
    /// answer (the tree's "Unset" item, status text).
    var masterPath: String? { masters.last }
    @Published private(set) var isRunning = false
    @Published private(set) var total = 0
    @Published private(set) var done = 0

    private var task: Task<Void, Never>?
    /// Held while the user is interacting. Parsing runs on the main actor, so a
    /// single big file (10–30ms) lands right in the middle of a drag and starves
    /// mouse events — measured: our own drag work is 0.3ms but events arrived
    /// only every 56–77ms while indexing ran.
    private var paused = false
    /// Set by the view so warming can parse through the engine.
    weak var engine: EngineBridge?
    private static let mastersKey = "suisei.masterDirectories"
    /// The single-path key this replaced. Read once, then never written again,
    /// so an existing install keeps the project it had.
    private static let legacyMasterKey = "suisei.masterDirectory"

    init() {
        let d = UserDefaults.standard
        var saved = d.stringArray(forKey: Self.mastersKey) ?? []
        if saved.isEmpty, let legacy = d.string(forKey: Self.legacyMasterKey) {
            saved = [legacy]
        }
        masters = saved.filter { FileManager.default.fileExists(atPath: $0) }
    }

    /// Why a directory may not become a master.
    ///
    /// `nil` means it may. Both directions are refused, not just the one the
    /// user named: allowing a PARENT of an existing master would put that
    /// master inside a master a moment later, which is the same illegal shape
    /// discovered in the other order.
    func masterRefusal(for path: String) -> String? {
        let p = Self.normalised(path)
        if masters.contains(where: { Self.normalised($0) == p }) {
            return "This folder is already a project master directory."
        }
        if let outer = masters.first(where: { Self.contains($0, p) }) {
            return "Inside “\(URL(fileURLWithPath: outer).lastPathComponent)”, "
                + "which is already a project master directory."
        }
        if let inner = masters.first(where: { Self.contains(p, $0) }) {
            return "Contains “\(URL(fileURLWithPath: inner).lastPathComponent)”, "
                + "which is already a project master directory."
        }
        return nil
    }

    private static func normalised(_ p: String) -> String {
        let s = URL(fileURLWithPath: p).standardizedFileURL.path
        return s.hasSuffix("/") && s.count > 1 ? String(s.dropLast()) : s
    }

    /// Whether `outer` is a strict ancestor of `inner`. Compared on path
    /// COMPONENTS: a `hasPrefix` check would call `/a/bc` a child of `/a/b`.
    private static func contains(_ outer: String, _ inner: String) -> Bool {
        let a = normalised(outer), b = normalised(inner)
        guard a != b else { return false }
        return b.hasPrefix(a.hasSuffix("/") ? a : a + "/")
    }

    /// Stand down while the user is dragging or typing.
    func pause() { paused = true }
    func resume() { paused = false }

    func isIndexed(_ path: String) -> Bool { indexed.contains(path) }
    func didFail(_ path: String) -> Bool { failed.contains(path) }

    /// Add `path` as a master directory and warm everything under it.
    ///
    /// Refused when it would nest — see `masterRefusal`. Marks the folder as a
    /// project on the way in, which is what puts it above loose files in
    /// Recents.
    @discardableResult
    func setMaster(_ path: String) -> String? {
        if let refusal = masterRefusal(for: path) { return refusal }
        task?.cancel()
        masters.append(Self.normalised(path))
        UserDefaults.standard.set(masters, forKey: Self.mastersKey)
        _ = path.withCString { suisei_project_mark($0) }
        indexed.removeAll()
        failed.removeAll()
        start()
        return nil
    }

    func start() {
        let roots = masters
        guard !roots.isEmpty else { return }
        task?.cancel()
        isRunning = true
        done = 0
        total = 0

        task = Task { [weak self] in
            // Discovery and line counting are IO — keep them off the main actor
            // so the editor stays responsive while the index builds.
            //
            // Every master in one list, sorted together: the biggest file in
            // the whole set is the one most worth warming first, whichever
            // project it is in. Masters cannot nest, so nothing is walked twice.
            let files = await Self.discover(roots: roots)
            guard !Task.isCancelled else { return }
            await MainActor.run { self?.total = files.count }

            for file in files {
                if Task.isCancelled { return }
                // Wait out any interaction rather than competing with it.
                while await MainActor.run(body: { self?.paused ?? false }) {
                    if Task.isCancelled { return }
                    try? await Task.sleep(nanoseconds: 80_000_000)
                }
                // Parsing happens on the main actor because the engine is not
                // Sendable — but it is one file at a time with an await between
                // each, so the editor keeps breathing rather than blocking on
                // the whole project.
                let ok = await MainActor.run { [weak self] () -> Bool in
                    self?.engine?.prewarmFile(file.path) ?? false
                }
                await MainActor.run {
                    guard let self else { return }
                    if ok { self.indexed.insert(file.path) } else { self.failed.insert(file.path) }
                    self.done += 1
                }
                // Breathe between files so the main thread is never held for
                // more than one parse at a time.
                try? await Task.sleep(nanoseconds: 8_000_000)
            }
            await MainActor.run { self?.isRunning = false }
        }
    }

    /// Forget one master directory, or all of them.
    ///
    /// The `project.suiseiprj` file stays. It is the project's identity and
    /// belongs to the repository, not to this machine's indexing preferences —
    /// deleting it here would silently change what a teammate's clone is.
    func unsetMaster(_ path: String? = nil) {
        stop()
        if let path {
            let p = Self.normalised(path)
            masters.removeAll { Self.normalised($0) == p }
        } else {
            masters.removeAll()
        }
        UserDefaults.standard.set(masters, forKey: Self.mastersKey)
        UserDefaults.standard.removeObject(forKey: Self.legacyMasterKey)
        indexed.removeAll()
        failed.removeAll()
        total = 0
        done = 0
        if !masters.isEmpty { start() }
    }

    func stop() {
        task?.cancel()
        task = nil
        isRunning = false
    }

    // MARK: - Work

    private struct Entry: Sendable {
        let path: String
        let lines: Int
    }

    /// Synchronous filesystem walk — `nonisolated` so it runs on whatever
    /// thread calls it (a detached task), with no actor-isolation warnings.
    /// The enumerator is created, iterated and discarded within this single
    /// call; no mutable state crosses a concurrency boundary.
    private nonisolated static func collectFiles(root: String) -> [Entry] {
        var out: [Entry] = []
        let fm = FileManager.default
        guard let walker = fm.enumerator(
            at: URL(fileURLWithPath: root),
            includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey],
            options: [.skipsHiddenFiles, .skipsPackageDescendants]
        ) else { return [] }

        while let obj = walker.nextObject() {
            guard let url = obj as? URL else { continue }
            if _skippedDirectories.contains(url.lastPathComponent) {
                walker.skipDescendants()
                continue
            }
            guard _codeExtensions.contains(url.pathExtension.lowercased()) else { continue }
            let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
            guard values?.isRegularFile == true,
                  let size = values?.fileSize, size <= _maxBytes
            else { continue }
            // Count newlines without decoding the whole file as a String.
            guard let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { continue }
            let lines = data.reduce(into: 1) { acc, byte in if byte == 0x0A { acc += 1 } }
            out.append(Entry(path: url.path, lines: lines))
        }
        return out
    }

    /// Code files under `root`, **most lines first** — the expensive ones get
    /// warmed while the user is still getting oriented.
    private nonisolated static func discover(roots: [String]) async -> [Entry] {
        await Task.detached(priority: .utility) {
            roots.flatMap { collectFiles(root: $0) }.sorted { $0.lines > $1.lines }
        }.value
    }

}
