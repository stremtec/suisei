import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// Xcode-style hierarchical Project navigator (disclosure triangles, nest, icons, filter).
struct ProjectTreeView: View {
    /// Background warm-up of the project's code files (biggest first).
    @ObservedObject var index: ProjectIndex
    /// Needed for file operations: create / rename / move / trash all report
    /// back to the core so open tabs follow the path.
    @ObservedObject var engine: EngineBridge
    let rootPath: String
    let accent: Color
    let fg: Color
    let dim: Color
    let editorBg: Color
    var onOpenFile: (String) -> Void
    var onRefresh: () -> Void

    @State private var expanded: Set<String> = []
    @State private var selectedPath: String = ""
    /// Files a live reload just touched. Observed separately for the same
    /// reason everywhere else does it: a reload must not republish the shell
    /// to reach the tree.
    @ObservedObject private var live = EngineBridge.shared.live
    @State private var filter: String = ""
    @State private var nodes: [TreeNode] = []
    @State private var gitMarks: [String: String] = [:] // rel path → M/?/A/D
    @State private var rootName: String = ""
    @FocusState private var filterFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            if rootPath.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "folder.badge.questionmark")
                        .font(.system(size: 22))
                        .foregroundStyle(dim.opacity(0.6))
                    Text("No project open")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(dim)
                    Text("File → Open… a folder")
                        .font(.system(size: 11))
                        .foregroundStyle(dim.opacity(0.7))
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                let rows = expanded.contains(rootPath)
                    ? visibleChildren(of: rootPath, depth: 1)
                    : []
                ScrollView(.vertical, showsIndicators: true) {
                    // `LazyVStack` creates rows on demand and does NOT run
                    // insertion transitions, so above this threshold expanding
                    // is deliberately instant: past a few hundred visible rows,
                    // building every one of them eagerly costs more than the
                    // animation is worth. Below it, a plain VStack animates.
                    TreeRowStack(useLazy: rows.count > 400) {
                        treeRow(
                            name: rootName.isEmpty ? (rootPath as NSString).lastPathComponent : rootName,
                            path: rootPath,
                            isDir: true,
                            depth: 0,
                            isRoot: true
                        )
                        if expanded.contains(rootPath) {
                            ForEach(rows, id: \.id) { node in
                                treeRow(
                                    name: node.name,
                                    path: node.path,
                                    isDir: node.isDir,
                                    depth: node.depth,
                                    isRoot: false
                                )
                                // Xcode's outline reveals children as ONE
                                // block sliding out from under the parent while
                                // the rows below shift down — not a per-row
                                // sprinkle. A staggered insertion also fought
                                // the enclosing `withAnimation` and LazyVStack's
                                // on-demand row creation, which is why it looked
                                // like everything appeared at once anyway.
                                .transition(
                                    .asymmetric(
                                        insertion: .move(edge: .top).combined(with: .opacity),
                                        removal: .opacity
                                    )
                                )
                            }
                        }
                    }
                    // Implicit, bound to the row count — the same mechanism the
                    // disclosure chevron uses, and the only one measured to
                    // actually run here. The explicit `withAnimation` around
                    // `expanded` never reached these rows: frame-by-frame
                    // capture showed the chevron mid-rotation while every new
                    // row was already at its final position.
                    .animation(.smooth(duration: 0.26), value: rows.count)
                    .padding(.top, 2)
                    .padding(.bottom, 6)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)

                // Filter bar: the rounded capsule alone. The bare [+] that
                // used to sit here posted New Untitled Tab — a TUI vestige;
                // that action lives in the tab strip's + menu, and two
                // pluses stacked would read as one feature.
                HStack(spacing: 5) {
                    Image(systemName: "line.3.horizontal.decrease.circle")
                        .font(.system(size: 11))
                        .foregroundStyle(dim)
                    TextField("Filter", text: $filter)
                        .textFieldStyle(.plain)
                        .font(.system(size: 11))
                        .foregroundStyle(fg)
                        .focused($filterFocused)
                    if !filter.isEmpty {
                        Button {
                            // End any active marked-text session before clearing
                            // the binding, then return focus on the next runloop.
                            filterFocused = false
                            filter = ""
                            DispatchQueue.main.async {
                                filterFocused = true
                            }
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .font(.system(size: 10))
                                .foregroundStyle(dim)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 7)
                .padding(.vertical, 4)
                .background(
                    Capsule(style: .continuous)
                        .fill(fg.opacity(0.06))
                )
                .overlay(
                    Capsule(style: .continuous)
                        .strokeBorder(fg.opacity(0.10), lineWidth: 1)
                )
                .padding(.horizontal, 8)
                .padding(.top, 6)
                // Clear the card's 12pt bottom corner radius — at 6pt the
                // capsule sat inside the curve and read as glued to the edge.
                .padding(.bottom, 11)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onAppear {
            if expanded.isEmpty, !rootPath.isEmpty {
                expanded = [rootPath]
            }
            rebuild()
        }
        // Title-row commands (the header buttons live in ContentView's
        // dockedNavigator — the state they need, `expanded` / inline rename,
        // lives here, so they arrive as notifications, the app's existing
        // cross-component command idiom).
        .onReceive(NotificationCenter.default.publisher(for: .suiseiNavNewFile)) { _ in
            newEntry(in: targetFolder, folder: false)
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiNavNewFolder)) { _ in
            newEntry(in: targetFolder, folder: true)
        }
        .onReceive(NotificationCenter.default.publisher(for: .suiseiNavCollapseAll)) { _ in
            withAnimation(.smooth(duration: 0.26)) { expanded = [rootPath] }
        }
        .onChange(of: rootPath) { _, newRoot in
            expanded = newRoot.isEmpty ? [] : [newRoot]
            rebuild()
        }
    }

    // MARK: - Row

    /// Inline rename, the way Finder and Xcode do it: the row itself becomes a
    /// text field. A sheet would have been less code, but "the tree feels
    /// thin" is exactly the impression a modal for every rename creates.
    @State private var draggingPath: String? = nil
    @State private var dropTarget: String? = nil
    @State private var renamingPath: String? = nil
    @State private var draftName: String = ""
    @FocusState private var renameFocused: Bool

    private func treeRow(
        name: String,
        path: String,
        isDir: Bool,
        depth: Int,
        isRoot: Bool
    ) -> some View {
        let isExpanded = expanded.contains(path)
        let isSelected = selectedPath == path
        let mark = gitMark(for: path)
        let indent = CGFloat(depth) * 14 + 6

        return Button {
            selectedPath = path
            if isDir {
                if expanded.contains(path) {
                    withAnimation(.smooth(duration: 0.26)) {
                        _ = expanded.remove(path)
                    }
                } else {
                    // Read the directory BEFORE the animation starts. This was
                    // inside the `withAnimation` block, so the first frame had
                    // to wait on a synchronous `contentsOfDirectory` — on any
                    // real folder that ate the whole 0.26s and the rows just
                    // appeared, which is why the expand looked instant.
                    _ = children(of: path)
                    withAnimation(.smooth(duration: 0.26)) {
                        _ = expanded.insert(path)
                    }
                }
            } else {
                onOpenFile(path)
            }
        } label: {
            HoverRow(corner: 4) {
                HStack(spacing: 3) {
                    // Disclosure
                    if isDir {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(dim.opacity(0.85))
                            .rotationEffect(.degrees(isExpanded ? 90 : 0))
                            .animation(.snappy(duration: 0.18), value: isExpanded)
                            .frame(width: 12, height: 12)
                    } else {
                        Color.clear.frame(width: 12, height: 12)
                    }

                    let isMaster = isDir && index.masterPath == path
                    Image(systemName: isMaster
                          ? "folder.fill.badge.gearshape"
                          : iconName(name: name, isDir: isDir, isRoot: isRoot))
                        .font(.system(size: 12))
                        .foregroundStyle(isMaster
                                         ? accent
                                         : iconColor(name: name, isDir: isDir, isRoot: isRoot))
                        .frame(width: 14)

                    // Index state, right against the name so it is
                    // unambiguous which file it refers to.
                    if !isDir, index.isIndexed(path) || index.didFail(path) {
                        Image(systemName: index.didFail(path)
                              ? "xmark.circle.fill" : "checkmark.circle.fill")
                            .font(.system(size: 8, weight: .bold))
                            .foregroundStyle(index.didFail(path)
                                             ? Color.secondary.opacity(0.55) : accent.opacity(0.85))
                    }

                    if renamingPath == path {
                        TextField("", text: $draftName)
                            .textFieldStyle(.plain)
                            .font(.system(size: 12))
                            .focused($renameFocused)
                            .onSubmit { commitRename(path) }
                            .onExitCommand { renamingPath = nil }
                            .onAppear {
                                draftName = name
                                renameFocused = true
                            }
                            .frame(maxWidth: 220)
                    } else if name == SuiseiProject.marker {
                        // The marker is the folder's identity card, not a file
                        // anyone opens. It reads as itself rather than as the
                        // eleventh grey row: a git status letter would be the
                        // wrong thing to colour it, so it does not take one.
                        Text(name)
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(Self.projectInk)
                            .shadow(color: Self.projectInk.opacity(0.7), radius: 4)
                            .shadow(color: Self.projectInk.opacity(0.35), radius: 9)
                            .lineLimit(1)
                    } else {
                        Text(name)
                            .fontWeight(isDir && index.masterPath == path ? .semibold : .regular)
                            .font(.system(size: 12, weight: isRoot ? .semibold : .regular))
                            .foregroundStyle(markColor(mark) ?? fg)
                            .lineLimit(1)
                    }

                    Spacer(minLength: 2)

                    if let mark, !mark.isEmpty {
                        Text(mark)
                            .font(.system(size: 10, weight: .semibold, design: .monospaced))
                            .foregroundStyle(markBadgeColor(mark))
                            .padding(.trailing, 4)
                    }
                }
                .padding(.leading, indent)
                .padding(.trailing, 8)
                .padding(.vertical, 3)
                .frame(maxWidth: .infinity, alignment: .leading)
                // Swiss-grid list selection: restrained 4pt corners. Capsules
                // are reserved for segmented controls and filters.
                .background(
                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                        .fill(
                            dropTarget == path
                                ? accent.opacity(0.25)
                                : (isSelected ? Color.primary.opacity(0.12) : Color.clear)
                        )
                )
                // Something rewrote this file while you were elsewhere.
                //
                // Under the selection rather than over it, and fading: this is
                // a notice, not a state, and it must not look like the row is
                // selected. `TimelineView` only while something is lit — one
                // left standing would drive the whole tree at frame rate.
                .background {
                    if live.files[path] != nil {
                        TimelineView(.animation) { _ in
                            let f = live.fileIntensity(path)
                            RoundedRectangle(cornerRadius: 4, style: .continuous)
                                .fill(accent.opacity(0.22 * f))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                                        .strokeBorder(accent.opacity(0.55 * f), lineWidth: 1)
                                )
                        }
                    }
                }
                .overlay(
                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                        .strokeBorder(
                            dropTarget == path ? accent : .clear, lineWidth: 1
                        )
                )
                .contentShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
            }
        }
        .buttonStyle(.plain)
        // Drag the row out as a file URL — the same payload Finder sends, so a
        // drag out of Suisei lands correctly in other apps too.
        .onDrag {
            draggingPath = path
            return NSItemProvider(contentsOf: URL(fileURLWithPath: path))
                ?? NSItemProvider()
        }
        // A folder row takes the drop itself; a FILE row hands it to the folder
        // that contains it. That is what Finder does, and it is the only way to
        // move something *out* of a folder without walking all the way up to
        // the top-level directory and dropping there — which was the whole
        // complaint. The highlight follows the real destination, so the user
        // sees the enclosing folder light up, not the file they are hovering.
        .onDrop(
            of: [UTType.fileURL],
            isTargeted: Binding(
                get: { dropTarget == path },
                set: { hit in
                    let destination = isDir ? path : (path as NSString).deletingLastPathComponent
                    if hit, canDrop(onto: destination) {
                        dropTarget = destination
                    } else if dropTarget == destination {
                        dropTarget = nil
                    }
                }
            )
        ) { providers in
            accept(providers, into: isDir ? path : (path as NSString).deletingLastPathComponent)
        }
        .padding(.horizontal, 4)
        .contextMenu {
            if isDir {
                Button("New File") { newEntry(in: path, folder: false) }
                Button("New Folder") { newEntry(in: path, folder: true) }
                Divider()
            } else {
                Button("New File") {
                    newEntry(in: (path as NSString).deletingLastPathComponent, folder: false)
                }
                Button("New Folder") {
                    newEntry(in: (path as NSString).deletingLastPathComponent, folder: true)
                }
                Divider()
            }
            if !isRoot {
                Button("Rename") {
                    draftName = name
                    renamingPath = path
                }
                // Drag-and-drop only reaches what is on screen. Moving a file
                // to a folder that is scrolled away, collapsed, or outside the
                // project needs a destination picker.
                Button("Move to…") { moveElsewhere(path) }
                Button("Move to Trash") { trash(path) }
                Divider()
            }
            if isDir {
                // Masters are a SET now, so the question is about this folder
                // rather than about "the" master.
                if index.masters.contains(where: { $0 == path }) {
                    Button("Unset Project Master Directory") { index.unsetMaster(path) }
                    Button("Re-index") { index.start() }
                } else if let refusal = index.masterRefusal(for: path) {
                    // Shown and disabled rather than hidden: a missing item is
                    // a mystery, a greyed one with the reason beside it is an
                    // answer. Nesting is the only thing refused.
                    Button("Set Project Master Directory — \(refusal)") {}
                        .disabled(true)
                } else {
                    Button("Set Project Master Directory") { index.setMaster(path) }
                }
                Divider()
                Button("Reveal in Finder") {
                    NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: path)
                }
            } else {
                Button("Open") { onOpenFile(path) }
                Button("Reveal in Finder") {
                    NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
                }
            }
            Button("Refresh Tree") {
                onRefresh()
                rebuild()
            }
        }
    }

    // MARK: - Drag and drop

    /// A folder can take the drop unless it is the source, the source's current
    /// parent (nothing would change), or inside the source's own subtree —
    /// which would detach the moved folder from the tree entirely.
    private func canDrop(onto folder: String) -> Bool {
        guard let src = draggingPath else { return true }
        return EngineBridge.moveIsSane(from: src, to: (folder as NSString).appendingPathComponent(
            (src as NSString).lastPathComponent
        ))
    }

    private func accept(_ providers: [NSItemProvider], into folder: String) -> Bool {
        dropTarget = nil
        // ⌥ copies, the macOS convention; a plain drag moves.
        let copy = NSEvent.modifierFlags.contains(.option)
        var handled = false
        for provider in providers {
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                DispatchQueue.main.async {
                    guard EngineBridge.moveIsSane(
                        from: url.path,
                        to: (folder as NSString).appendingPathComponent(url.lastPathComponent)
                    ) else { return }
                    if engine.movePath(url.path, into: folder, copy: copy) != nil {
                        Self.invalidateCache()
                        withAnimation(.smooth(duration: 0.26)) {
                            _ = expanded.insert(folder)
                        }
                        onRefresh()
                    }
                }
            }
            handled = true
        }
        draggingPath = nil
        return handled
    }

    // MARK: - File actions

    /// Create, then drop straight into inline rename — one path for "new" and
    /// "rename", and the same gesture Finder gives you.
    private func newEntry(in directory: String, folder: Bool) {
        let made = folder
            ? engine.createFolder(in: directory)
            : engine.createFile(in: directory)
        guard let made else { return }
        Self.invalidateCache()
        withAnimation(.smooth(duration: 0.26)) {
            _ = expanded.insert(directory)
        }
        onRefresh()
        selectedPath = made
        draftName = (made as NSString).lastPathComponent
        renamingPath = made
    }

    /// Where New File / New Folder land: the selected row wins (a folder
    /// itself, a file's parent); then the active document's folder when it
    /// lives under this project; then the project root.
    private var targetFolder: String {
        if !selectedPath.isEmpty {
            var isDir: ObjCBool = false
            if FileManager.default.fileExists(atPath: selectedPath, isDirectory: &isDir) {
                return isDir.boolValue
                    ? selectedPath
                    : (selectedPath as NSString).deletingLastPathComponent
            }
        }
        let active = engine.chrome.filename
        if !active.isEmpty, active.hasPrefix(rootPath + "/") {
            return (active as NSString).deletingLastPathComponent
        }
        return rootPath
    }

    private func commitRename(_ path: String) {
        let name = draftName
        renamingPath = nil
        guard let moved = engine.renamePath(path, to: name) else { return }
        Self.invalidateCache()
        onRefresh()
        selectedPath = moved
    }

    private func trash(_ path: String) {
        engine.trashPath(path)
        Self.invalidateCache()
        onRefresh()
        if selectedPath == path { selectedPath = "" }
    }

    /// Pick a destination folder and move there. The dragging path can only
    /// reach folders that happen to be visible and expanded; this reaches any
    /// of them, including out of the project entirely.
    private func moveElsewhere(_ path: String) {
        let panel = NSOpenPanel()
        engine.sizePanel(panel, remembering: "moveTo")
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Move"
        panel.message = "Move “\((path as NSString).lastPathComponent)” to:"
        // Start where the file lives, so one click up is one click away.
        panel.directoryURL = URL(
            fileURLWithPath: (path as NSString).deletingLastPathComponent
        )
        guard panel.runModal() == .OK, let destination = panel.url else { return }
        guard engine.movePath(path, into: destination.path) != nil else { return }
        Self.invalidateCache()
        // Reveal the destination if it is inside the tree, so the moved item is
        // visible where it landed rather than seeming to vanish.
        withAnimation(.smooth(duration: 0.26)) {
            _ = expanded.insert(destination.path)
        }
        onRefresh()
        if selectedPath == path { selectedPath = "" }
    }

    // MARK: - Tree model

    struct TreeNode: Identifiable {
        var id: String { path }
        let path: String
        let name: String
        let isDir: Bool
        let depth: Int
    }

    private func rebuild() {
        guard !rootPath.isEmpty else {
            nodes = []
            rootName = ""
            return
        }
        rootName = (rootPath as NSString).lastPathComponent
        if expanded.isEmpty {
            expanded = [rootPath]
        }
        // Prime root children
        _ = children(of: rootPath)
        loadGitMarks()
    }

    /// Flat list of visible descendants under `path` following `expanded` + filter.
    private func visibleChildren(of path: String, depth: Int) -> [TreeNode] {
        var out: [TreeNode] = []
        let kids = children(of: path)
        let q = filter.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        for kid in kids {
            if !q.isEmpty {
                // Show node if name matches or any descendant would match (dirs always if expanded path matches)
                if kid.isDir {
                    if kid.name.lowercased().contains(q) || dirContainsMatch(kid.path, q: q) {
                        out.append(TreeNode(path: kid.path, name: kid.name, isDir: true, depth: depth))
                        if expanded.contains(kid.path) || kid.name.lowercased().contains(q) {
                            // Auto-expand matching dirs while filtering
                            if !expanded.contains(kid.path) && kid.name.lowercased().contains(q) {
                                // keep collapsed unless already expanded; still show nested matches
                            }
                            if expanded.contains(kid.path) {
                                out.append(contentsOf: visibleChildren(of: kid.path, depth: depth + 1))
                            } else if dirContainsMatch(kid.path, q: q) {
                                // Force-show matching nested without permanent expand
                                out.append(contentsOf: matchingDescendants(kid.path, depth: depth + 1, q: q))
                            }
                        }
                    }
                } else if kid.name.lowercased().contains(q) {
                    out.append(TreeNode(path: kid.path, name: kid.name, isDir: false, depth: depth))
                }
            } else {
                out.append(TreeNode(path: kid.path, name: kid.name, isDir: kid.isDir, depth: depth))
                if kid.isDir, expanded.contains(kid.path) {
                    out.append(contentsOf: visibleChildren(of: kid.path, depth: depth + 1))
                }
            }
        }
        return out
    }

    private func matchingDescendants(_ path: String, depth: Int, q: String) -> [TreeNode] {
        var out: [TreeNode] = []
        for kid in children(of: path) {
            if kid.isDir {
                if kid.name.lowercased().contains(q) || dirContainsMatch(kid.path, q: q) {
                    out.append(TreeNode(path: kid.path, name: kid.name, isDir: true, depth: depth))
                    out.append(contentsOf: matchingDescendants(kid.path, depth: depth + 1, q: q))
                }
            } else if kid.name.lowercased().contains(q) {
                out.append(TreeNode(path: kid.path, name: kid.name, isDir: false, depth: depth))
            }
        }
        return out
    }

    /// Filtering runs synchronously on EVERY keystroke, so it must never touch
    /// the filesystem: a cold recursive scan from a root like "/" walks System
    /// and Library three levels deep and hangs the app. Only directories whose
    /// listing is already cached (i.e. the user has actually visited them) take
    /// part, so the cost is bounded by what is on screen.
    private func cachedChildren(_ path: String) -> [FSChild]? {
        Self.listingCache[path]
    }

    private func dirContainsMatch(_ path: String, q: String) -> Bool {
        guard let kids = cachedChildren(path) else { return false }
        for kid in kids {
            if kid.name.lowercased().contains(q) { return true }
            if kid.isDir, dirContainsMatchShallow(kid.path, q: q, depth: 0) { return true }
        }
        return false
    }

    private func dirContainsMatchShallow(_ path: String, q: String, depth: Int) -> Bool {
        if depth > 3 { return false }
        guard let kids = cachedChildren(path) else { return false }
        for kid in kids {
            if kid.name.lowercased().contains(q) { return true }
            if kid.isDir, dirContainsMatchShallow(kid.path, q: q, depth: depth + 1) { return true }
        }
        return false
    }

    private struct FSChild {
        let path: String
        let name: String
        let isDir: Bool
    }

    /// Directory listing cache for the session.
    private static var listingCache: [String: [FSChild]] = [:]

    private func children(of path: String) -> [FSChild] {
        if let cached = Self.listingCache[path] { return cached }
        var dirs: [FSChild] = []
        var files: [FSChild] = []
        let url = URL(fileURLWithPath: path)
        let keys: [URLResourceKey] = [.isDirectoryKey, .isHiddenKey, .nameKey]
        guard let items = try? FileManager.default.contentsOfDirectory(
            at: url,
            includingPropertiesForKeys: keys,
            options: [.skipsPackageDescendants]
        ) else {
            Self.listingCache[path] = []
            return []
        }
        for item in items {
            let name = item.lastPathComponent
            if name == ".DS_Store" || name == ".git" { continue }
            var isDir: ObjCBool = false
            FileManager.default.fileExists(atPath: item.path, isDirectory: &isDir)
            // Skip heavy build dirs at first level only? Still show them collapsed.
            let child = FSChild(path: item.path, name: name, isDir: isDir.boolValue)
            if isDir.boolValue {
                dirs.append(child)
            } else {
                files.append(child)
            }
        }
        dirs.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        // The project marker sorts to the floor, always.
        //
        // Alphabetically it lands under P, wedged between two source files it
        // has nothing to do with. It is not a file anyone opens or edits — it
        // is the folder's identity card — and the one place it can sit without
        // interrupting a scan for real files is the end of the list.
        files.sort {
            let a = $0.name == SuiseiProject.marker
            let b = $1.name == SuiseiProject.marker
            if a != b { return b }
            return $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
        }
        let all = dirs + files
        Self.listingCache[path] = all
        return all
    }

    static func invalidateCache() {
        listingCache.removeAll()
    }

    // MARK: - Git marks

    private func loadGitMarks() {
        guard !rootPath.isEmpty else {
            gitMarks = [:]
            return
        }
        // `git status` blocks for hundreds of ms on big repos — never on main.
        let root = rootPath
        DispatchQueue.global(qos: .utility).async {
            let map = Self.gitStatusMarks(root: root)
            DispatchQueue.main.async {
                guard root == rootPath else { return }
                gitMarks = map
            }
        }
    }

    private static func gitStatusMarks(root: String) -> [String: String] {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        task.arguments = ["-C", root, "status", "--porcelain", "-uall"]
        let pipe = Pipe()
        task.standardOutput = pipe
        task.standardError = Pipe()
        do {
            try task.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            task.waitUntilExit()
            guard let text = String(data: data, encoding: .utf8) else { return [:] }
            var map: [String: String] = [:]
            for line in text.split(separator: "\n") {
                guard line.count >= 3 else { continue }
                let xy = String(line.prefix(2))
                var rest = String(line.dropFirst(3))
                // rename: "R  old -> new"
                if rest.contains(" -> ") {
                    rest = rest.components(separatedBy: " -> ").last ?? rest
                }
                rest = rest.trimmingCharacters(in: .init(charactersIn: "\""))
                let mark: String
                if xy.contains("?") { mark = "?" }
                else if xy.contains("A") { mark = "A" }
                else if xy.contains("D") { mark = "D" }
                else if xy.contains("M") || xy.contains("U") { mark = "M" }
                else { mark = String(xy.trimmingCharacters(in: .whitespaces).prefix(1)) }
                if !mark.isEmpty {
                    map[rest] = mark
                }
            }
            return map
        } catch {
            return [:]
        }
    }

    private func gitMark(for path: String) -> String? {
        guard path.hasPrefix(rootPath) else { return nil }
        var rel = String(path.dropFirst(rootPath.count))
        if rel.hasPrefix("/") { rel = String(rel.dropFirst()) }
        return gitMarks[rel]
    }

    private func markColor(_ mark: String?) -> Color? {
        guard let mark else { return nil }
        switch mark {
        case "M": return Color(nsColor: .systemOrange)
        case "A": return Color(nsColor: .systemGreen)
        case "D": return Color(nsColor: .systemRed)
        case "?": return Color(nsColor: .systemBlue)
        default: return nil
        }
    }

    private func markBadgeColor(_ mark: String) -> Color {
        markColor(mark) ?? dim
    }

    // MARK: - Icons (Xcode-ish)

    private func iconName(name: String, isDir: Bool, isRoot: Bool) -> String {
        if isRoot { return "folder.fill" }
        if isDir { return "folder.fill" }
        let ext = (name as NSString).pathExtension.lowercased()
        switch ext {
        case "rs": return "chevron.left.forwardslash.chevron.right"
        case "swift": return "swift"
        case "js", "ts", "jsx", "tsx", "mjs": return "curlybraces"
        case "json", "toml", "yaml", "yml": return "doc.badge.gearshape"
        case "md", "txt", "rst": return "doc.plaintext"
        case "png", "jpg", "jpeg", "gif", "svg", "webp": return "photo"
        case "sh", "bash", "zsh": return "terminal"
        case "html", "css", "scss": return "globe"
        case "lock": return "lock.doc"
        default:
            // Filled, against the other manifests' outline: it names the
            // project the way they name a package, and it is the only one of
            // them Suisei wrote itself.
            if name == SuiseiProject.marker { return "shippingbox.fill" }
            if name == "Cargo.toml" || name == "Package.swift" || name == "package.json" {
                return "shippingbox"
            }
            if name.hasPrefix(".") { return "doc" }
            return "doc"
        }
    }

    /// The project marker's purple.
    ///
    /// A literal, not the accent: the accent is the user's and can be set to
    /// anything, including a colour every other row already uses. This row is
    /// meant to be the one that is not like the others.
    ///
    /// Two glow radii rather than one — a tight 4pt for the bloom right at the
    /// glyph edges and a wide 9pt for the halo. One radius gives either a
    /// blurred word or a faint fog, never light.
    static let projectInk = Color(nsColor: .systemPurple)

    private func iconColor(name: String, isDir: Bool, isRoot: Bool) -> Color {
        if isDir || isRoot {
            return accent
        }
        // The icon sits against the name; leaving it grey beside a glowing
        // purple word reads as a rendering fault rather than as emphasis.
        if name == SuiseiProject.marker { return Self.projectInk }
        let ext = (name as NSString).pathExtension.lowercased()
        switch ext {
        case "rs": return Color(nsColor: .systemOrange)
        case "swift": return Color(nsColor: .systemOrange)
        case "js", "mjs": return Color(nsColor: .systemYellow)
        case "ts", "tsx": return Color(nsColor: .systemBlue)
        case "md": return dim
        case "json", "toml", "yaml", "yml": return Color(nsColor: .systemGreen)
        case "png", "jpg", "jpeg", "gif", "svg": return Color(nsColor: .systemPink)
        default: return dim.opacity(0.9)
        }
    }
}

/// Chooses between an animating stack and a lazy one.
///
/// SwiftUI's lazy containers skip insertion transitions, so folder expansion
/// appeared instantly; a plain `VStack` animates but builds every row. Big
/// trees keep laziness and give up the animation — the right trade at that
/// size.
private struct TreeRowStack<Content: View>: View {
    let useLazy: Bool
    /// A CLOSURE, not a stored `Content`.
    ///
    /// As `@ViewBuilder var content: Content` the rows were built once, at this
    /// wrapper's init, and handed in already resolved — so the insertion
    /// transitions on them never joined the `withAnimation` transaction that
    /// expanding a folder starts. Measured: the disclosure chevron rotated
    /// smoothly while every new row was already at its final position in the
    /// first frame. Building inside each branch restores it.
    @ViewBuilder let content: () -> Content

    var body: some View {
        if useLazy {
            LazyVStack(alignment: .leading, spacing: 0) { content() }
        } else {
            VStack(alignment: .leading, spacing: 0) { content() }
        }
    }
}
