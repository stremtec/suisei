import Foundation
import SwiftUI

// MARK: - File-private constants (implicitly Sendable — safe to read from any
// concurrency domain without @MainActor isolation).

/// Source files only. Binaries, images, archives and media have nothing to
/// parse, and walking them would just burn IO.
private let _codeExtensions: Set<String> = [
    "swift", "rs", "c", "h", "cpp", "hpp", "cc", "cxx", "hh", "hxx", "m", "mm",
    "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "py", "pyi", "go",
    "rb", "java", "kt", "kts", "scala", "cs", "php", "sh", "bash", "zsh",
    "lua", "dart", "zig", "nim", "ex", "exs", "hs", "sql", "json", "jsonc",
    "toml", "yaml", "yml", "md",
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
    /// Master directory, once the user picks one.
    @Published private(set) var masterPath: String?
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
    private static let masterKey = "suisei.masterDirectory"

    init() {
        if let saved = UserDefaults.standard.string(forKey: Self.masterKey),
           FileManager.default.fileExists(atPath: saved)
        {
            masterPath = saved
        }
    }

    /// Stand down while the user is dragging or typing.
    func pause() { paused = true }
    func resume() { paused = false }

    func isIndexed(_ path: String) -> Bool { indexed.contains(path) }
    func didFail(_ path: String) -> Bool { failed.contains(path) }

    /// Point the index at `path` and start warming everything under it.
    func setMaster(_ path: String) {
        task?.cancel()
        masterPath = path
        UserDefaults.standard.set(path, forKey: Self.masterKey)
        indexed.removeAll()
        failed.removeAll()
        start()
    }

    func start() {
        guard let root = masterPath else { return }
        task?.cancel()
        isRunning = true
        done = 0
        total = 0

        task = Task { [weak self] in
            // Discovery and line counting are IO — keep them off the main actor
            // so the editor stays responsive while the index builds.
            let files = await Self.discover(root: root)
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

    /// Stop indexing and forget the master directory.
    func unsetMaster() {
        stop()
        masterPath = nil
        UserDefaults.standard.removeObject(forKey: Self.masterKey)
        indexed.removeAll()
        failed.removeAll()
        total = 0
        done = 0
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
    private nonisolated static func discover(root: String) async -> [Entry] {
        await Task.detached(priority: .utility) {
            collectFiles(root: root).sorted { $0.lines > $1.lines }
        }.value
    }

}
