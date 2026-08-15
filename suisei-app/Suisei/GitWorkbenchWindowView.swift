import SwiftUI
import AppKit

fileprivate enum DiffRowKind: Equatable {
    case metadata
    case fileHeader
    case hunk
    case addition
    case deletion
    case context
    case note
}

fileprivate struct DiffRow: Identifiable, Equatable {
    let id: Int
    let content: String
    let marker: String
    let kind: DiffRowKind
    let oldLine: Int?
    let newLine: Int?
}

/// An independent, native Source Control browser modelled after Xcode's
/// Changes / Repositories navigator rather than a Finder file browser.
struct GitWorkbenchWindowView: View {
    let engine: EngineBridge
    @ObservedObject private var store: GitWorkbenchStore
    @Environment(\.dismiss) private var dismiss

    @State private var sourceMode: SourceMode = .changes
    @State private var sidebarSelection: SidebarSelection? = .repository
    @State private var changeFilter: ChangeFilter = .all
    @State private var historyRange: HistoryRange = .all
    @State private var searchQuery = ""
    @State private var modeFrom = 0
    @State private var modeTo = 0
    @State private var modeProgress: CGFloat = 1
    /// Drag position in SLOT FRACTIONS (0 = first slot, 1 = second, …), not
    /// points. Points tied every reader of this state to the rail's current
    /// width, which is the dependency that made a sidebar collapse re-measure
    /// the whole rail on every frame.
    @State private var modeDragFraction: Double?
    @State private var modeDragOriginFraction: Double?
    @State private var modeDragCommitting = false
    @State private var commitMessage = ""
    @State private var amend = false
    @State private var expandedChangeIDs: Set<Int> = []
    @State private var expandedCommitFileIDs: Set<Int> = []
    @State private var selectedChangeID: Int?
    @State private var selectedCommitFileID: Int?
    @State private var worktreeDiffCache: [Int: [DiffRow]] = [:]
    @State private var commitDiffCache: [Int: [DiffRow]] = [:]
    /// More than one card may be expanded while Core resolves diffs in the
    /// background. Keep every outstanding identity instead of letting the
    /// latest click orphan an earlier card's result.
    @State private var pendingDiffTargets: [DiffTarget] = []
    @State private var selectedBranchID: Int?
    @State private var changesExpanded = true
    @State private var branchesExpanded = true
    @State private var remotesExpanded = true
    @State private var stashesExpanded = true
    @State private var showNewBranchDialog = false
    @State private var newBranchName = ""
    @State private var pendingBranchDeletion: GitBranchItem?
    @State private var pendingDiscard: GitWorktreeItem?

    init(engine: EngineBridge) {
        self.engine = engine
        _store = ObservedObject(wrappedValue: engine.gitWorkbenchStore)
    }

    private static let sidebarWidth: CGFloat = 294
    private static let historyWidth: CGFloat = 318

    private enum SourceMode: String, CaseIterable, Identifiable {
        case changes = "Changes"
        case repositories = "Repositories"
        var id: String { rawValue }
    }

    private enum SidebarSelection: Hashable {
        case repository
        case change(Int)
        case branch(Int)
        case stash(Int)
        case remote(Int)
    }

    private enum ChangeFilter: String, CaseIterable, Identifiable {
        case all = "All Changes"
        case unstaged = "Unstaged"
        case staged = "Staged"
        var id: String { rawValue }
    }

    private enum HistoryRange: String, CaseIterable, Identifiable {
        case all = "All"
        case day = "24h"
        case week = "7d"
        case month = "30d"
        var id: String { rawValue }
    }

    private enum DiffTarget: Equatable {
        case worktree(id: Int, path: String)
        case commit(id: Int, path: String)

        var path: String {
            switch self {
            case .worktree(_, let path), .commit(_, let path): path
            }
        }
    }

    private var model: GitWbSnap { store.snapshot }
    private var theme: ThemeSnap { store.theme }
    private var accent: Color { theme.color(theme.accent) }

    private var accentForeground: Color {
        let color = theme.accent
        let r = Double((color >> 16) & 0xFF) / 255.0
        let g = Double((color >> 8) & 0xFF) / 255.0
        let b = Double(color & 0xFF) / 255.0
        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b
        return luminance > 0.58 ? .black : .white
    }

    private var isLightTheme: Bool {
        let color = theme.editorBg
        let r = Double((color >> 16) & 0xFF)
        let g = Double((color >> 8) & 0xFF)
        let b = Double(color & 0xFF)
        return (0.299 * r + 0.587 * g + 0.114 * b) > 150
    }

    private var repositoryName: String {
        if !model.repositoryName.isEmpty { return model.repositoryName }
        let root = engine.projectRoot.isEmpty ? engine.chrome.explorer.cwd : engine.projectRoot
        let name = URL(fileURLWithPath: root).lastPathComponent
        return name.isEmpty ? "Repository" : name
    }

    private var branchName: String {
        model.branch.components(separatedBy: " ↑").first ?? model.branch
    }

    var body: some View {
        workbenchDialogs
    }

    private var configuredContent: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            workspace
        }
        .navigationSplitViewStyle(.balanced)
        .navigationTitle("Source Control")
        .toolbarTitleDisplayMode(.inline)
        .toolbar { sourceControlToolbar }
        // The minimum is the WINDOW's, not this view's — see `minContentSize`
        // below. Expressed here as `frame(minWidth: 1040)` it was a width the
        // content could demand, so opening the sidebar asked for 288 + 1040:
        // the split view grew to 1328, centred itself with 144pt hanging off
        // each side for the whole animation, and snapped back in the final two
        // frames. `SUISEI_DIAG=sidebar` shows it exactly:
        //
        //     pane0=[x-144 w288] pane1=[x-144 w1328]
        //     pane0=[x0    w288] pane1=[x0    w1328]
        //     pane0=[x0    w288] pane1=[x0    w1040]
        //
        // Closing never did this — it shrinks the pane in place, x pinned at 0
        // — which is why only opening popped.
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .preferredColorScheme(isLightTheme ? .light : .dark)
        .tint(accent)
        .accentColor(accent)
        .background(theme.windowBg.ignoresSafeArea())
        .background(
            ThemedWindowChrome(
                background: .windowBackgroundColor,
                light: isLightTheme,
                identifier: WindowChrome.gitWorkbenchIdentifier,
                opaque: true,
                // 294pt source list + 318pt history + a useful 420pt diff
                // column. Below this the workbench is not a smaller layout; it
                // is a clipped three-column one.
                minContentSize: NSSize(width: 1040, height: 560)
            )
        )
    }

    private var lifecycleContent: some View {
        configuredContent
        .onAppear {
            engine.openGitWorkbenchWindow()
            synchronizeSourceMode()
            if let selected = model.worktree.first(where: \.selected) ?? model.worktree.first {
                selectedChangeID = selected.id
                expandedChangeIDs.insert(selected.id)
                sidebarSelection = .change(selected.id)
                requestWorktreeDiff(selected)
            }
        }
        .onChange(of: sourceMode) { _, mode in
            searchQuery = ""
            animateModeSelection(to: mode)
            synchronizeSourceMode()
        }
        .onChange(of: sidebarSelection) { _, selection in
            handleSidebarSelection(selection)
        }
        .onChange(of: model.worktree) { _, rows in
            guard !rows.isEmpty else {
                expandedChangeIDs.removeAll()
                selectedChangeID = nil
                worktreeDiffCache.removeAll()
                return
            }
            let validIDs = Set(rows.map(\.id))
            expandedChangeIDs.formIntersection(validIDs)
            worktreeDiffCache = worktreeDiffCache.filter { validIDs.contains($0.key) }
            if !rows.contains(where: { $0.id == selectedChangeID }) {
                let row = rows.first(where: \.selected) ?? rows[0]
                selectedChangeID = row.id
                expandedChangeIDs.insert(row.id)
                requestWorktreeDiff(row)
            }
        }
        .onChange(of: model.special) { _, _ in
            cachePendingDiffIfReady()
        }
        .onChange(of: model.branches) { _, branches in
            guard sourceMode == .repositories else { return }
            guard let branch = branches.first(where: { $0.id == selectedBranchID })
                    ?? branches.first(where: \.current)
                    ?? branches.first else {
                selectedBranchID = nil
                sidebarSelection = .repository
                return
            }
            selectedBranchID = branch.id
            sidebarSelection = .branch(branch.id)
            engine.gitWbSelectBranchHistory(branch.id)
        }
        .onChange(of: model.open) { _, open in
            if open {
                // A Window scene preserves SwiftUI state while Core correctly
                // reopens on Status. Republish the remembered Changes /
                // Repositories destination or the shell can say Repositories
                // while Core never starts its branch/history load.
                synchronizeSourceMode()
            } else {
                dismiss()
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { note in
            guard let window = note.object as? NSWindow,
                  window.identifier == WindowChrome.gitWorkbenchIdentifier else { return }
            engine.closeGitWorkbenchWindow()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { note in
            handleWindowBecameKey(note.object)
        }
    }

    private var workbenchDialogs: some View {
        lifecycleContent
        .alert("New Branch", isPresented: $showNewBranchDialog) {
            TextField("Branch name", text: $newBranchName)
            Button("Cancel", role: .cancel) {}
            Button("Create") {
                engine.gitWbCreateBranch(newBranchName)
                newBranchName = ""
            }
            .disabled(newBranchName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        } message: {
            Text("Create and check out a branch from the current HEAD.")
        }
        .confirmationDialog(
            "Discard changes to “\(pendingDiscard?.path ?? "")”?",
            isPresented: Binding(
                get: { pendingDiscard != nil },
                set: { if !$0 { pendingDiscard = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Discard Changes", role: .destructive) {
                if let row = pendingDiscard { engine.gitWbDiscardChange(row.id) }
                pendingDiscard = nil
            }
            Button("Cancel", role: .cancel) { pendingDiscard = nil }
        } message: {
            Text(pendingDiscard?.status == "?"
                 ? "The untracked item will be removed. This cannot be undone."
                 : "The working copy will be restored from HEAD. This cannot be undone.")
        }
        .confirmationDialog(
            "Delete “\(pendingBranchDeletion?.name ?? "")”?",
            isPresented: Binding(
                get: { pendingBranchDeletion != nil },
                set: { if !$0 { pendingBranchDeletion = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Delete Branch", role: .destructive) {
                if let branch = pendingBranchDeletion {
                    engine.gitWbSetTab(3)
                    engine.gitWbSelectSpecial(branch.id)
                    engine.gitWbDeleteSelectedBranch()
                }
                pendingBranchDeletion = nil
            }
            Button("Cancel", role: .cancel) { pendingBranchDeletion = nil }
        } message: {
            Text("Only the selected local branch will be deleted.")
        }
    }

    private func handleWindowBecameKey(_ object: Any?) {
        guard let window = object as? NSWindow,
              window.identifier == WindowChrome.gitWorkbenchIdentifier else { return }
        engine.focusGitWorkbenchWindow()
    }

    /// Keep the persistent SwiftUI scene and the disposable Core panel on the
    /// same top-level destination. `onChange(sourceMode)` is insufficient on a
    /// reopen because the scene remembers its value while Core starts fresh.
    private func synchronizeSourceMode() {
        switch sourceMode {
        case .changes:
            engine.gitWbSetTab(1)
            sidebarSelection = selectedChangeID.map(SidebarSelection.change) ?? .repository
        case .repositories:
            engine.gitWbSetTab(3)
            sidebarSelection = selectedBranchID.map(SidebarSelection.branch) ?? .repository
        }
    }

    @ToolbarContentBuilder
    private var sourceControlToolbar: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            Button(action: openSelectedFile) {
                Image(systemName: "square.and.pencil")
            }
            .disabled(!selectedFileExists)
            .help("Open in Editor")
        }

        ToolbarItem(placement: .primaryAction) {
            Button(action: revealSelectedFile) {
                Image(systemName: "folder")
            }
            .disabled(!selectedFileExists)
            .help("Show in Finder")
        }

        ToolbarItem(placement: .primaryAction) {
            Button(action: copySelectedFilePath) {
                Image(systemName: "doc.on.doc")
            }
            .disabled(selectedFilePath == nil)
            .help("Copy Path")
        }

        ToolbarItem(placement: .primaryAction) {
            Button {
                engine.gitWbRefreshWindow()
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .help("Refresh Source Control")
        }

        ToolbarItem(placement: .primaryAction) {
            sourceControlActionsMenu
        }

        ToolbarItem(placement: .primaryAction) {
            GitWorkbenchToolbarSearchField(
                text: $searchQuery,
                prompt: sourceMode == .changes ? "Filter Changes" : "Search Commits"
            )
            .frame(width: 280, height: 30)
        }
    }

    private var sourceControlActionsMenu: some View {
        Menu {
            Button("New Branch…", systemImage: "plus") {
                newBranchName = ""
                showNewBranchDialog = true
            }
            Divider()
            Button("Stash All Changes", systemImage: "tray.and.arrow.down") {
                engine.gitWbStash()
            }
            .disabled(model.worktree.isEmpty)
            Button("Stage All Changes", systemImage: "plus.rectangle.on.folder") {
                engine.gitWbStageAll()
            }
            .disabled(model.worktree.allSatisfy(\.staged))
            Button("Unstage All Changes", systemImage: "minus.rectangle") {
                engine.gitWbUnstageAll()
            }
            .disabled(!model.worktree.contains(where: \.staged))
        } label: {
            Image(systemName: "ellipsis")
        }
        .menuIndicator(.hidden)
        .menuStyle(.borderlessButton)
        .fixedSize()
        .help("Source Control Actions")
    }

    /// The List owns the column; the mode rail is a bar pinned above it.
    ///
    /// This was `VStack { rail; Divider; List }`, and `SUISEI_DIAG=sidebar`
    /// showed what that costs. Opening the sidebar produced, in one millisecond
    /// and one layout transaction:
    ///
    ///     w=280.0 y=52.0 safeTop=44.0 row1=103.0
    ///     w=280.0 y=44.0 safeTop=44.0 row1=103.0
    ///     w=280.0 y=52.0 safeTop=44.0 row1=103.0
    ///
    /// The width never moved. `safeTop` never moved. The first row never moved.
    /// Only the container's origin, by exactly the 8pt between it and the
    /// titlebar — computed one way, then the other, then back. Rendered inside
    /// an animation, that correction is the pop.
    ///
    /// The cause is ownership. A non-scrolling rail stacked above the List
    /// leaves two candidates for who the column's top safe area belongs to, and
    /// SwiftUI settled it differently on consecutive passes. `safeAreaBar`
    /// removes the question: the List is the column, and the rail is a bar
    /// inside its safe area. It is what the Settings sidebar already does, and
    /// the reason its search field does not do this.
    ///
    /// The `Divider` goes with it. `scrollEdgeEffectStyle(.soft)` is the native
    /// version of that line and it does not leave a seam.
    private var sidebar: some View {
        List(selection: $sidebarSelection) {
            if sourceMode == .changes {
                changesOutline
            } else {
                repositoriesOutline
            }
        }
        .listStyle(.sidebar)
        .scrollContentBackground(.hidden)
        .contentMargins(.top, 4, for: .scrollContent)
        // Every row is one line. `lineLimit` propagates through the
        // environment, so setting it here means a row added later cannot
        // reintroduce wrapping by omission — and a two-line label in a column
        // whose width animates is a height change waiting to happen. A macOS
        // source list truncates; it does not wrap.
        .lineLimit(1)
        .scrollEdgeEffectStyle(.soft, for: .top)
        .safeAreaBar(edge: .top, spacing: 0) {
            sourceModeRail
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
        }
        .navigationSplitViewColumnWidth(
            min: 280,
            ideal: Self.sidebarWidth,
            max: 350
        )
        // `SUISEI_DIAG=sidebar`. Samples Core Animation's presentation layers
        // on the display link, because the collapse is an AppKit animation and
        // a SwiftUI `GeometryReader` never sees its intermediate frames.
        .background(SidebarPresentationTrace())
    }

    /// Same explicit-geometry pill used by the editor navigator: one authority
    /// owns placement, hit testing and drag travel, so the highlight cannot
    /// lag behind the selected destination.
    private var sourceModeRail: some View {
        GeometryReader { geometry in
            let modes = SourceMode.allCases
            let slot = geometry.size.width / CGFloat(modes.count)
            ZStack(alignment: .leading) {
                TravellingPill(
                    progress: modeProgress,
                    from: CGFloat(modeDragFraction ?? modeDragOriginFraction
                                  ?? Double(modeFrom)) * slot,
                    to: CGFloat(modeDragFraction ?? Double(modeTo)) * slot,
                    width: slot
                )
                .fill(accent)
                // The pill FOLLOWS a width change; it does not animate to it.
                //
                // `TravellingPill` animates all four of its values, which is
                // right where the slots resize on a discrete toggle — the
                // editor's navigator rail, which it was written for. Here the
                // container is a NavigationSplitView sidebar, and collapsing it
                // sweeps the width continuously. Every frame handed the Shape a
                // new `from`/`to`/`width` and it began a fresh interpolation
                // toward them: an animation chasing an animation, which is what
                // the pill rubber-banding behind the collapsing column was.
                //
                // Scoped to `slot`, so a mode switch — driven by `modeProgress`
                // under `withAnimation` — still travels.
                .animation(nil, value: slot)

                HStack(spacing: 0) {
                    ForEach(Array(modes.enumerated()), id: \.element.id) { index, mode in
                        Button {
                            sourceMode = mode
                        } label: {
                            HStack(spacing: 6) {
                                Image(systemName: mode == .changes
                                      ? "arrow.triangle.2.circlepath"
                                      : "shippingbox")
                                Text(mode.rawValue)
                            }
                            .font(.system(size: 11.5, weight: .semibold))
                            .foregroundStyle(modeRailForeground(index: index))
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        // Equal division by layout rather than by `slot`. Two
                        // buttons in an HStack split the width evenly on their
                        // own, and asking for `slot` made every label's body
                        // depend on the geometry — so a width sweep invalidated
                        // and re-measured both of them on every frame of the
                        // collapse, for a result identical to what the stack
                        // was already going to do.
                        .frame(maxWidth: .infinity)
                    }
                }
            }
            .contentShape(Capsule(style: .continuous))
            .simultaneousGesture(
                DragGesture(minimumDistance: 3, coordinateSpace: .local)
                    .onChanged { value in
                        guard slot > 0 else { return }
                        let points = min(
                            max(value.location.x - slot / 2, 0),
                            geometry.size.width - slot
                        )
                        modeDragFraction = Double(points / slot)
                    }
                    .onEnded { value in
                        let index = min(
                            modes.count - 1,
                            max(0, Int(value.location.x / slot))
                        )
                        modeDragOriginFraction = modeDragFraction
                        modeDragFraction = nil
                        modeDragCommitting = true
                        sourceMode = modes[index]
                        modeDragCommitting = false
                        modeFrom = index
                        modeTo = index
                        modeProgress = 0
                        withAnimation(.smooth(duration: 0.24)) {
                            modeProgress = 1
                        } completion: {
                            modeDragOriginFraction = nil
                        }
                    }
            )
        }
        .frame(height: 28)
        .padding(2)
        .background {
            Capsule(style: .continuous)
                .fill(Color.primary.opacity(isLightTheme ? 0.045 : 0.08))
                .overlay {
                    Capsule(style: .continuous)
                        .stroke(theme.separator.opacity(0.65), lineWidth: 1)
                }
        }
    }

    private func animateModeSelection(to mode: SourceMode) {
        guard !modeDragCommitting,
              let destination = SourceMode.allCases.firstIndex(of: mode),
              destination != modeTo else { return }
        modeFrom = modeTo
        modeTo = destination
        modeProgress = 0
        withAnimation(.smooth(duration: 0.24)) {
            modeProgress = 1
        } completion: {
            modeDragOriginFraction = nil
        }
    }

    /// How selected this label reads, 0…1.
    ///
    /// The drag case measures overlap in FRACTIONS of a slot rather than in
    /// points, so the function no longer needs the slot width at all — which is
    /// what let the labels stop depending on the container's geometry.
    /// `modeDragFraction` is the drag position expressed the same way.
    private func modeRailForeground(index: Int) -> Color {
        let selected: Double
        if let drag = modeDragFraction {
            let lo = max(drag, Double(index))
            let hi = min(drag + 1, Double(index + 1))
            selected = max(0, hi - lo)
        } else if modeTo == modeFrom {
            selected = modeTo == index ? 1 : 0
        } else {
            let span = Double(modeTo - modeFrom)
            let center = (Double(index) - Double(modeFrom)) / span
            let reach = 1 / abs(span)
            selected = max(0, 1 - abs(Double(modeProgress) - center) / reach)
        }
        return Color.primary.mix(with: accentForeground, by: selected)
    }

    @ViewBuilder
    private var changesOutline: some View {
        repositoryOutlineRow(subtitle: branchName)
            .tag(SidebarSelection.repository)

        DisclosureGroup(isExpanded: $changesExpanded) {
            ForEach(filteredSidebarChanges) { row in
                sourceFileRow(row)
                    .tag(SidebarSelection.change(row.id))
                    .contextMenu { worktreeContextMenu(row) }
            }
        } label: {
            HStack(spacing: 7) {
                Image(systemName: "arrow.triangle.2.circlepath")
                Text("Uncommitted Changes")
                Spacer(minLength: 4)
                Text("\(model.worktree.count)")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
            }
            .font(.system(size: 12.5, weight: .semibold))
            // Native DisclosureGroup reserves the chevron outside its label.
            // Without a small optical inset our first symbol almost touches it.
            .padding(.leading, 5)
        }
    }

    @ViewBuilder
    private var repositoriesOutline: some View {
        repositoryOutlineRow(subtitle: model.rootPath)
            .tag(SidebarSelection.repository)

        DisclosureGroup(isExpanded: $branchesExpanded) {
            if model.loading && model.branches.isEmpty {
                Label("Loading Branches…", systemImage: "arrow.clockwise")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.secondary)
            } else {
                ForEach(filteredLocalBranches) { branch in
                    branchOutlineRow(branch)
                }
            }
        } label: {
            outlineGroupLabel("Branches", symbol: "arrow.triangle.branch", count: localBranches.count)
        }

        Label("Recent Locations", systemImage: "clock")
            .font(.system(size: 12.5, weight: .semibold))
            .foregroundStyle(.secondary)
        Label("Tags", systemImage: "tag")
            .font(.system(size: 12.5, weight: .semibold))
            .foregroundStyle(.secondary)

        DisclosureGroup(isExpanded: $stashesExpanded) {
            ForEach(Array(model.stashes.enumerated()), id: \.offset) { index, stash in
                Label(stash, systemImage: "archivebox")
                    .font(.system(size: 12, weight: .semibold))
                    .lineLimit(1)
                    .tag(SidebarSelection.stash(index))
            }
        } label: {
            outlineGroupLabel("Stashed Changes", symbol: "tray.full", count: model.stashes.count)
        }

        DisclosureGroup(isExpanded: $remotesExpanded) {
            ForEach(model.remotes) { remote in
                DisclosureGroup {
                    ForEach(filteredRemoteBranches(remote.name)) { branch in
                        branchOutlineRow(branch)
                    }
                } label: {
                    Label(remote.name, systemImage: "network")
                        .font(.system(size: 12.5, weight: .semibold))
                }
                .tag(SidebarSelection.remote(remote.id))
            }
        } label: {
            outlineGroupLabel("Remotes", symbol: "externaldrive.connected.to.line.below", count: model.remotes.count)
        }
    }

    /// Explicit icon geometry avoids Label centring the repository glyph
    /// between a two-line title and subtitle. The box now shares the title's
    /// optical baseline and the same icon column as the outline below it.
    private func repositoryOutlineRow(subtitle: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "shippingbox.fill")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(theme.color(theme.accent))
                .frame(width: 17, height: 17)
                .padding(.top, 1)

            VStack(alignment: .leading, spacing: 1) {
                Text(repositoryName)
                    .font(.system(size: 13, weight: .semibold))
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.system(size: 10.5, weight: .regular))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }

            Spacer(minLength: 0)
        }
        .contentShape(Rectangle())
    }

    private func outlineGroupLabel(_ title: String, symbol: String, count: Int) -> some View {
        HStack(spacing: 7) {
            Image(systemName: symbol)
            Text(title)
            Spacer(minLength: 4)
            if count > 0 {
                Text("\(count)")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
            }
        }
        .font(.system(size: 12.5, weight: .semibold))
    }

    private func sourceFileRow(_ row: GitWorktreeItem) -> some View {
        HStack(spacing: 7) {
            Image(systemName: fileSymbol(row.path))
                .symbolRenderingMode(.monochrome)
                .foregroundStyle(sidebarSelection == .change(row.id) ? Color.white : fileTint(row.path))
                .frame(width: 16)

            VStack(alignment: .leading, spacing: 1) {
                Text(URL(fileURLWithPath: row.path).lastPathComponent)
                    .lineLimit(1)
                let parent = (row.path as NSString).deletingLastPathComponent
                if !parent.isEmpty, parent != "." {
                    Text(parent)
                        .font(.system(size: 9.5, weight: .regular))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.head)
                }
            }

            Spacer(minLength: 5)
            statusBadge(row.status, selected: sidebarSelection == .change(row.id))
        }
        .font(.system(size: 12.5, weight: .semibold))
        .frame(minHeight: 28)
    }

    private func branchOutlineRow(_ branch: GitBranchItem) -> some View {
        let selected = sidebarSelection == .branch(branch.id)
        return Button {
            selectBranch(branch)
        } label: {
            HStack(spacing: 7) {
                Image(systemName: branch.current ? "checkmark.circle.fill" : "arrow.triangle.branch")
                    .foregroundStyle(selected || branch.current ? accent : Color.secondary)
                    .frame(width: 16)
                Text(branch.name)
                    .font(.system(size: 12.5, weight: .semibold))
                    .foregroundStyle(selected ? accent : Color.primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 4)
                if branch.current {
                    Text("Current")
                        .font(.system(size: 9.5, weight: .medium))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 6)
            .frame(maxWidth: .infinity, minHeight: 28, alignment: .leading)
            .background {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(selected ? Color.primary.opacity(isLightTheme ? 0.09 : 0.12) : Color.clear)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button("Show History") { selectBranch(branch) }
            Button("Check Out") {
                engine.gitWbSetTab(3)
                engine.gitWbSelectSpecial(branch.id)
                engine.gitWbCheckoutSelectedBranch()
            }
            .disabled(branch.current)
            if !branch.remote {
                Divider()
                Button("Delete Branch", role: .destructive) {
                    pendingBranchDeletion = branch
                }
                .disabled(branch.current)
            }
        }
    }

    @ViewBuilder
    private var workspace: some View {
        if model.rootPath.isEmpty {
            ContentUnavailableView(
                "No Repository",
                systemImage: "shippingbox",
                description: Text("Open a file or folder inside a Git repository.")
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(nsColor: .textBackgroundColor))
        } else if sourceMode == .changes {
            changesWorkspace
        } else {
            repositoriesWorkspace
        }
    }

    private var changesWorkspace: some View {
        VStack(spacing: 0) {
            identityAndCommitRow
            Divider()
            commitMessageEditor
            Divider()
            changesFilterBar
            Divider()

            if model.worktree.isEmpty {
                ContentUnavailableView(
                    "No Local Changes",
                    systemImage: "checkmark.circle",
                    description: Text("The working tree is clean.")
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollViewReader { proxy in
                    ScrollView(.vertical) {
                        LazyVStack(spacing: 10) {
                            ForEach(filteredWorkspaceChanges) { row in
                                worktreeDiffCard(row)
                                    .id("change-\(row.id)")
                            }
                        }
                        .padding(10)
                    }
                    .onChange(of: selectedChangeID) { _, id in
                        guard let id else { return }
                        withAnimation(.easeOut(duration: 0.14)) {
                            proxy.scrollTo("change-\(id)", anchor: .top)
                        }
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .textBackgroundColor))
    }

    private var identityAndCommitRow: some View {
        HStack(spacing: 10) {
            Image(systemName: "person.crop.circle.fill")
                .font(.system(size: 29))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(theme.color(theme.accent))

            VStack(alignment: .leading, spacing: 1) {
                Text(model.authorName.isEmpty ? "Git Author" : model.authorName)
                    .font(.system(size: 12.5, weight: .semibold))
                Text(model.authorEmail.isEmpty ? "Configure user.email in Git" : model.authorEmail)
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
            }

            Spacer(minLength: 12)

            Toggle("Amend", isOn: $amend)
                .toggleStyle(.switch)
                .controlSize(.mini)
                .font(.system(size: 11.5))

            Button {
                guard canCommit else { return }
                engine.gitWbCommit(message: commitMessage, amend: amend)
                commitMessage = ""
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "checkmark")
                    Text("Commit")
                }
                .font(.system(size: 11.5, weight: .semibold))
                .foregroundStyle(canCommit ? Color.white : Color.secondary)
                .padding(.horizontal, 11)
                .frame(height: 24)
                .background {
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(canCommit
                              ? theme.color(theme.accent)
                              : Color(nsColor: .controlColor))
                }
                .overlay {
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .stroke(theme.separator.opacity(canCommit ? 0 : 0.55), lineWidth: 1)
                }
            }
            .buttonStyle(.plain)
            .fixedSize()
            .accessibilityLabel("Commit")
            .accessibilityValue(canCommit ? "Ready" : "Enter a commit message")
        }
        .padding(.horizontal, 12)
        .frame(minHeight: 53)
        .background(theme.windowBg)
    }

    private var commitMessageEditor: some View {
        ZStack(alignment: .topLeading) {
            if commitMessage.isEmpty {
                Text("Commit message")
                    .font(.system(size: 12.5))
                    .foregroundStyle(.tertiary)
                    .padding(.leading, 15)
                    .padding(.top, 10)
                    .allowsHitTesting(false)
            }
            TextEditor(text: $commitMessage)
                .font(.system(size: 12.5))
                .scrollContentBackground(.hidden)
                .padding(.horizontal, 10)
                // TextEditor owns an AppKit text-container inset of its own.
                // Give its top edge the same optical inset as the placeholder;
                // symmetric padding left the insertion caret visibly higher.
                .padding(.top, 10)
                .padding(.bottom, 2)
        }
        .frame(height: 48)
        .background(Color(nsColor: .textBackgroundColor))
    }

    private var changesFilterBar: some View {
        HStack(spacing: 10) {
            changesFilterRail

            Spacer()

            Button(model.worktree.contains(where: \.staged) ? "Stage All" : "Stage All") {
                engine.gitWbStageAll()
            }
            .buttonStyle(.plain)
            .font(.system(size: 11.5, weight: .semibold))
            .foregroundStyle(theme.color(theme.accent))
            .disabled(model.worktree.allSatisfy(\.staged))
        }
        .padding(.horizontal, 12)
        .frame(height: 34)
        .background(theme.windowBg)
    }

    /// Scope controls are a floating glass lens, not three AppKit segmented
    /// cells. The selected label keeps the accent while the surface behind it
    /// remains translucent in both clear and tinted Liquid Glass modes.
    private var changesFilterRail: some View {
        GeometryReader { geometry in
            let filters = ChangeFilter.allCases
            let slot = geometry.size.width / CGFloat(filters.count)
            let selected = filters.firstIndex(of: changeFilter) ?? 0

            ZStack(alignment: .leading) {
                liquidSelectionPill(width: slot)
                    .offset(x: CGFloat(selected) * slot)
                    .animation(.snappy(duration: 0.18, extraBounce: 0), value: changeFilter)

                HStack(spacing: 0) {
                    ForEach(Array(filters.enumerated()), id: \.element.id) { index, filter in
                        Button {
                            changeFilter = filter
                        } label: {
                            Text(filter.rawValue)
                                .font(.system(size: 10.5, weight: .semibold))
                                .foregroundStyle(index == selected ? accent : Color.secondary)
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .frame(width: slot)
                    }
                }
            }
        }
        .frame(width: 260, height: 24)
        .padding(2)
        .background {
            Capsule(style: .continuous)
                .fill(Color.primary.opacity(isLightTheme ? 0.045 : 0.08))
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Changes Filter")
    }

    private func worktreeDiffCard(_ row: GitWorktreeItem) -> some View {
        let expanded = expandedChangeIDs.contains(row.id)
        return VStack(spacing: 0) {
            diffCardHeader(
                path: row.path,
                status: row.status,
                staged: row.staged,
                insertions: nil,
                deletions: nil,
                expanded: expanded
            ) {
                toggleWorktreeCard(row)
            } actions: {
                Menu {
                    Button(row.staged ? "Unstage" : "Stage") {
                        engine.gitWbToggleStage(row.id)
                    }
                    Button("Open in Editor") { openFile(row.path) }
                    Button("Show in Finder") { revealFile(row.path) }
                    Divider()
                    Button("Discard Changes…", role: .destructive) {
                        pendingDiscard = row
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }

            if expanded {
                Divider()
                if let rows = worktreeDiffRows(for: row) {
                    diffCanvas(
                        rows: rows,
                        historical: false,
                        untracked: row.status == "?"
                    )
                } else {
                    ProgressView()
                        .controlSize(.small)
                        .frame(maxWidth: .infinity, minHeight: 54)
                }
            }
        }
        .animation(.easeOut(duration: 0.14), value: expanded)
        .background(theme.panelSurface)
        .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 7, style: .continuous)
                .stroke(theme.separator.opacity(0.7), lineWidth: 1)
        }
        .contextMenu { worktreeContextMenu(row) }
    }

    @ViewBuilder
    private func worktreeContextMenu(_ row: GitWorktreeItem) -> some View {
        Button("Open in Editor") { openFile(row.path) }
        Button("Show in Finder") { revealFile(row.path) }
        Divider()
        Button(row.staged ? "Unstage" : "Stage") { engine.gitWbToggleStage(row.id) }
        Button("Discard Changes…", role: .destructive) { pendingDiscard = row }
    }

    private var repositoriesWorkspace: some View {
        GeometryReader { geometry in
            HSplitView {
                historyMaster
                    .frame(minWidth: 270, idealWidth: Self.historyWidth, maxWidth: 410)
                    .frame(maxHeight: .infinity)

                repositoryDetailWorkspace
                    .frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .textBackgroundColor))
    }

    @ViewBuilder
    private var repositoryDetailWorkspace: some View {
        if model.loading && model.history.isEmpty {
            VStack(spacing: 10) {
                ProgressView().controlSize(.small)
                Text("Loading History…")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if filteredHistory.isEmpty {
            ContentUnavailableView(
                model.history.isEmpty ? "No Commits Yet" : "No Commits in \(historyRange.rawValue)",
                systemImage: "clock.arrow.circlepath",
                description: Text(
                    model.history.isEmpty
                        ? "The selected branch has no commit history."
                        : "Choose a broader history range or clear the search."
                )
            )
        } else {
            commitDetailWorkspace
        }
    }

    private var historyMaster: some View {
        VStack(spacing: 0) {
            historyHeader

            Divider()

            if model.loading && model.history.isEmpty {
                Color.clear
            } else if filteredHistory.isEmpty {
                Color.clear
            } else {
                List(selection: historySelection) {
                    ForEach(filteredHistory) { commit in
                        historyRow(commit)
                            .tag(commit.id)
                    }
                }
                .listStyle(.inset)
                .contentMargins(.top, 0, for: .scrollContent)
            }
        }
        .background(theme.windowBg)
    }

    private var historyHeader: some View {
        HStack(spacing: 8) {
            Text("History")
                .font(.system(size: 11.5, weight: .semibold))
            Spacer(minLength: 8)
            historyRangeRail
        }
        .padding(.horizontal, 9)
        .frame(height: 34)
        .background(.bar)
    }

    /// Compact Xcode-style scope rail. It keeps the four history ranges in one
    /// quiet capsule and moves one accent pill instead of drawing four little
    /// segmented-control cells.
    private var historyRangeRail: some View {
        GeometryReader { geometry in
            let ranges = HistoryRange.allCases
            let slot = geometry.size.width / CGFloat(ranges.count)
            let selected = ranges.firstIndex(of: historyRange) ?? 0

            ZStack(alignment: .leading) {
                liquidSelectionPill(width: slot)
                    .offset(x: CGFloat(selected) * slot)
                    .animation(.snappy(duration: 0.18, extraBounce: 0), value: historyRange)

                HStack(spacing: 0) {
                    ForEach(Array(ranges.enumerated()), id: \.element.id) { index, range in
                        Button {
                            historyRange = range
                        } label: {
                            Text(range.rawValue)
                                .font(.system(size: 10.5, weight: .semibold))
                                .foregroundStyle(index == selected ? accent : Color.secondary)
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .frame(width: slot)
                    }
                }
            }
        }
        .frame(width: 142, height: 24)
        .padding(2)
        .background {
            Capsule(style: .continuous)
                .fill(Color.primary.opacity(isLightTheme ? 0.045 : 0.08))
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("History Range")
    }

    private func liquidSelectionPill(width: CGFloat) -> some View {
        let shape = Capsule(style: .continuous)
        return shape
            .fill(.clear)
            .frame(width: width, height: 24)
            // Match the clear, refractive thumb used by native sliders. A
            // coloured fill reads as a segmented-control selection instead.
            .glassEffect(.clear.interactive(), in: shape)
    }

    private func historyRow(_ commit: GitHistoryItem) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "person.crop.circle.fill")
                .font(.system(size: 18))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(theme.color(theme.accent))
                .padding(.top, 1)

            VStack(alignment: .leading, spacing: 2) {
                Text(commit.subject)
                    .font(.system(size: 12, weight: .semibold))
                    .lineLimit(2)
                HStack(spacing: 5) {
                    Text(commit.author)
                    Text(commit.shortHash)
                        .font(.system(size: 10, design: .monospaced))
                }
                .font(.system(size: 10.5))
                .foregroundStyle(.secondary)
            }

            Spacer(minLength: 5)
            Text(commit.when)
                .font(.system(size: 9.5))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .padding(.vertical, 3)
    }

    private var historySelection: Binding<Int?> {
        Binding(
            get: {
                guard model.commitDetail != nil else { return nil }
                return model.history.first(where: \.selected)?.id
            },
            set: { id in
                guard let id else { return }
                expandedCommitFileIDs.removeAll()
                selectedCommitFileID = nil
                commitDiffCache.removeAll()
                engine.gitWbSelectHistory(id)
            }
        )
    }

    @ViewBuilder
    private var commitDetailWorkspace: some View {
        if let detail = model.commitDetail {
            ScrollView(.vertical) {
                LazyVStack(spacing: 10) {
                    commitHeader(detail)
                    ForEach(model.commitFiles) { file in
                        commitDiffCard(file)
                    }
                }
                .padding(10)
            }
        } else {
            ContentUnavailableView(
                "Select a Commit",
                systemImage: "point.3.connected.trianglepath.dotted",
                description: Text("Its message and file changes will appear here.")
            )
        }
    }

    private func commitHeader(_ detail: GitCommitDetailSnap) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(alignment: .top, spacing: 9) {
                Image(systemName: "person.crop.circle.fill")
                    .font(.system(size: 28))
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(theme.color(theme.accent))
                VStack(alignment: .leading, spacing: 2) {
                    Text(detail.author)
                        .font(.system(size: 12.5, weight: .semibold))
                    Text(detail.email)
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                    Text(detail.date)
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Text(detail.shortHash)
                    .font(.system(size: 10.5, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            Text(detail.subject)
                .font(.system(size: 14, weight: .semibold))
                .textSelection(.enabled)
            if !detail.body.isEmpty {
                Text(detail.body)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            HStack(spacing: 10) {
                Label("\(model.commitFiles.count) files", systemImage: "doc.on.doc")
                Text("+\(detail.insertions)").foregroundStyle(.green)
                Text("−\(detail.deletions)").foregroundStyle(.red)
            }
            .font(.system(size: 10.5, weight: .medium))
            .foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.panelSurface)
        .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 7, style: .continuous)
                .stroke(theme.separator.opacity(0.7), lineWidth: 1)
        }
    }

    private func commitDiffCard(_ file: GitCommitFileItem) -> some View {
        let expanded = expandedCommitFileIDs.contains(file.id)
        return VStack(spacing: 0) {
            diffCardHeader(
                path: file.path,
                status: file.status,
                staged: false,
                insertions: file.insertions,
                deletions: file.deletions,
                expanded: expanded
            ) {
                if expanded {
                    withAnimation(.easeOut(duration: 0.14)) {
                        _ = expandedCommitFileIDs.remove(file.id)
                    }
                } else {
                    withAnimation(.easeOut(duration: 0.14)) {
                        _ = expandedCommitFileIDs.insert(file.id)
                    }
                    selectedCommitFileID = file.id
                    requestCommitDiff(file)
                }
            } actions: {
                Menu {
                    Button("Open in Editor") { openFile(file.path) }
                    Button("Show in Finder") { revealFile(file.path) }
                    Button("Copy Path") { copyPath(file.path) }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }

            if expanded {
                Divider()
                if let rows = commitFileDiffRows(for: file) {
                    diffCanvas(
                        rows: rows,
                        historical: true,
                        untracked: false
                    )
                } else {
                    ProgressView()
                        .controlSize(.small)
                        .frame(maxWidth: .infinity, minHeight: 54)
                }
            }
        }
        .animation(.easeOut(duration: 0.14), value: expanded)
        .background(theme.panelSurface)
        .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 7, style: .continuous)
                .stroke(theme.separator.opacity(0.7), lineWidth: 1)
        }
    }

    private func diffCardHeader<Actions: View>(
        path: String,
        status: String,
        staged: Bool,
        insertions: Int?,
        deletions: Int?,
        expanded: Bool,
        toggle: @escaping () -> Void,
        @ViewBuilder actions: () -> Actions
    ) -> some View {
        HStack(spacing: 8) {
            Button(action: toggle) {
                Image(systemName: expanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 12)
            }
            .buttonStyle(.plain)

            Image(systemName: fileSymbol(path))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(fileTint(path))
                .frame(width: 17)

            Button(action: toggle) {
                HStack(spacing: 5) {
                    Text(URL(fileURLWithPath: path).lastPathComponent)
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.primary)
                    let parent = (path as NSString).deletingLastPathComponent
                    if !parent.isEmpty, parent != "." {
                        Text(parent)
                            .font(.system(size: 10.5))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.head)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Spacer(minLength: 8)

            if let insertions, insertions > 0 {
                Text("+\(insertions)")
                    .foregroundStyle(.green)
            }
            if let deletions, deletions > 0 {
                Text("−\(deletions)")
                    .foregroundStyle(.red)
            }
            if staged {
                Text("Staged")
                    .foregroundStyle(.secondary)
            }
            statusBadge(status, selected: false)
            actions()
        }
        .font(.system(size: 10.5, weight: .medium))
        .padding(.horizontal, 10)
        .frame(minHeight: 31)
        .background(theme.panelSurface)
        .contentShape(Rectangle())
    }

    private func diffCanvas(rows: [DiffRow], historical: Bool, untracked: Bool) -> some View {
        let visibleRows = displayDiffRows(rows)
        return GitDiffTableView(
            rows: visibleRows,
            historical: historical,
            untracked: untracked,
            light: isLightTheme,
            accent: theme.accent
        )
        // AppKit virtualizes/reuses the visible rows. Small diffs keep their
        // natural height; large files become one native scroll surface instead
        // of creating thousands of SwiftUI/Accessibility nodes or artificial
        // 60-line pages.
        .frame(height: min(440, diffCanvasHeight(visibleRows)))
        .background(Color(nsColor: .textBackgroundColor))
    }

    private var localBranches: [GitBranchItem] { model.branches.filter { !$0.remote } }

    private var canCommit: Bool {
        !commitMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || amend
    }

    private var filteredLocalBranches: [GitBranchItem] {
        localBranches.filter { matchesSidebarFilter($0.name) }
    }

    private func filteredRemoteBranches(_ remote: String) -> [GitBranchItem] {
        model.branches.filter {
            $0.remote && ($0.name.hasPrefix(remote + "/") || model.remotes.count == 1) && matchesSidebarFilter($0.name)
        }
    }

    private var filteredSidebarChanges: [GitWorktreeItem] {
        model.worktree.filter { matchesSidebarFilter($0.path) }
    }

    private var filteredWorkspaceChanges: [GitWorktreeItem] {
        model.worktree.filter { row in
            let statusMatches: Bool = switch changeFilter {
            case .all: true
            case .unstaged: !row.staged
            case .staged: row.staged
            }
            return statusMatches && matchesSearch(row.path)
        }
    }

    private var filteredHistory: [GitHistoryItem] {
        model.history.filter { commit in
            let query = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
            let queryMatches = query.isEmpty
                || commit.subject.localizedCaseInsensitiveContains(query)
                || commit.author.localizedCaseInsensitiveContains(query)
                || commit.shortHash.localizedCaseInsensitiveContains(query)
            return queryMatches && matchesHistoryRange(commit.when)
        }
    }

    private func matchesSidebarFilter(_ value: String) -> Bool {
        matchesSearch(value)
    }

    private func matchesSearch(_ value: String) -> Bool {
        let query = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        return query.isEmpty || value.localizedCaseInsensitiveContains(query)
    }

    private func matchesHistoryRange(_ when: String) -> Bool {
        switch historyRange {
        case .all: return true
        case .day:
            return when.contains("second") || when.contains("minute") || when.contains("hour")
        case .week:
            if when.contains("second") || when.contains("minute") || when.contains("hour") { return true }
            return relativeCount(when, unit: "day") <= 7
        case .month:
            if when.contains("second") || when.contains("minute") || when.contains("hour") { return true }
            if when.contains("day") { return relativeCount(when, unit: "day") <= 30 }
            return relativeCount(when, unit: "week") <= 4
        }
    }

    private func relativeCount(_ text: String, unit: String) -> Int {
        guard text.contains(unit) else { return .max }
        return Int(text.split(separator: " ").first ?? "") ?? 1
    }

    private func handleSidebarSelection(_ selection: SidebarSelection?) {
        guard let selection else { return }
        switch selection {
        case .repository:
            if sourceMode == .changes { engine.gitWbSetTab(1) }
        case .change(let id):
            guard let row = model.worktree.first(where: { $0.id == id }) else { return }
            selectedChangeID = id
            expandedChangeIDs.insert(id)
            if worktreeDiffCache[id] == nil, !diffTitleMatches(row.path) {
                requestWorktreeDiff(row)
            }
        case .branch(let id):
            guard sourceMode == .repositories else { return }
            selectedBranchID = id
            engine.gitWbSelectBranchHistory(id)
        case .stash(let id):
            engine.gitWbSetTab(9)
            engine.gitWbSelectSpecial(id)
        case .remote:
            break
        }
    }

    private func selectBranch(_ branch: GitBranchItem) {
        sourceMode = .repositories
        selectedBranchID = branch.id
        sidebarSelection = .branch(branch.id)
        engine.gitWbSelectBranchHistory(branch.id)
    }

    private func toggleWorktreeCard(_ row: GitWorktreeItem) {
        if expandedChangeIDs.contains(row.id) {
            withAnimation(.easeOut(duration: 0.14)) {
                _ = expandedChangeIDs.remove(row.id)
            }
        } else {
            withAnimation(.easeOut(duration: 0.14)) {
                _ = expandedChangeIDs.insert(row.id)
            }
            selectedChangeID = row.id
            if sidebarSelection == .change(row.id) {
                requestWorktreeDiff(row)
            } else {
                // Selection's onChange owns the request so a card click does
                // not synchronously ask Core for the same diff twice.
                sidebarSelection = .change(row.id)
            }
        }
    }

    private func requestWorktreeDiff(_ row: GitWorktreeItem) {
        pendingDiffTargets.removeAll { target in
            if case .worktree(let id, _) = target { return id == row.id }
            return false
        }
        pendingDiffTargets.append(.worktree(id: row.id, path: row.path))
        engine.gitWbSelectChange(row.id)
        // EngineBridge republishes synchronously today, while a future
        // background diff may arrive on a later tick. Cover both contracts.
        DispatchQueue.main.async { cachePendingDiffIfReady() }
    }

    private func requestCommitDiff(_ file: GitCommitFileItem) {
        pendingDiffTargets.removeAll { target in
            if case .commit(let id, _) = target { return id == file.id }
            return false
        }
        pendingDiffTargets.append(.commit(id: file.id, path: file.path))
        engine.gitWbSelectCommitFile(file.id)
        DispatchQueue.main.async { cachePendingDiffIfReady() }
    }

    private func cachePendingDiffIfReady() {
        guard let index = pendingDiffTargets.lastIndex(where: { diffTitleMatches($0.path) }) else {
            return
        }
        let target = pendingDiffTargets.remove(at: index)
        let rows = makeDiffRows(diffLines)
        switch target {
        case .worktree(let id, _): worktreeDiffCache[id] = rows
        case .commit(let id, _): commitDiffCache[id] = rows
        }
    }

    private func worktreeDiffRows(for row: GitWorktreeItem) -> [DiffRow]? {
        if let cached = worktreeDiffCache[row.id] { return cached }
        return diffTitleMatches(row.path) ? makeDiffRows(diffLines) : nil
    }

    private func commitFileDiffRows(for file: GitCommitFileItem) -> [DiffRow]? {
        if let cached = commitDiffCache[file.id] { return cached }
        return diffTitleMatches(file.path) ? makeDiffRows(diffLines) : nil
    }

    private func diffTitleMatches(_ path: String) -> Bool {
        guard let diffTitle else { return false }
        if diffTitle == path { return true }
        return URL(fileURLWithPath: diffTitle).lastPathComponent
            == URL(fileURLWithPath: path).lastPathComponent
    }

    private var diffTitle: String? {
        model.special.first(where: { $0.hasPrefix("diff ·") })
            .map { String($0.dropFirst("diff ·".count)).trimmingCharacters(in: .whitespaces) }
    }

    private var diffLines: [String] {
        model.special.filter { !$0.hasPrefix("diff ·") }
    }

    private func makeDiffRows(_ lines: [String]) -> [DiffRow] {
        var oldLine = 0
        var newLine = 0
        return lines.enumerated().map { index, line in
            if line.hasPrefix("@@") {
                let fields = line.split(separator: " ")
                if fields.count >= 3 {
                    oldLine = diffStartLine(fields[1])
                    newLine = diffStartLine(fields[2])
                }
                return DiffRow(id: index, content: line, marker: "", kind: .hunk, oldLine: nil, newLine: nil)
            }
            if line.hasPrefix("+") && !line.hasPrefix("+++") {
                if newLine == 0 { newLine = 1 }
                defer { newLine += 1 }
                return DiffRow(id: index, content: String(line.dropFirst()), marker: "+", kind: .addition, oldLine: nil, newLine: newLine)
            }
            if line.hasPrefix("-") && !line.hasPrefix("---") {
                if oldLine == 0 { oldLine = 1 }
                defer { oldLine += 1 }
                return DiffRow(id: index, content: String(line.dropFirst()), marker: "−", kind: .deletion, oldLine: oldLine, newLine: nil)
            }
            if line.hasPrefix(" ") {
                if oldLine == 0 { oldLine = 1 }
                if newLine == 0 { newLine = 1 }
                defer { oldLine += 1; newLine += 1 }
                return DiffRow(id: index, content: String(line.dropFirst()), marker: "", kind: .context, oldLine: oldLine, newLine: newLine)
            }
            let kind: DiffRowKind
            if line.hasPrefix("---") || line.hasPrefix("+++") {
                kind = .fileHeader
            } else if line.hasPrefix("\\ No newline") {
                kind = .note
            } else {
                kind = .metadata
            }
            return DiffRow(id: index, content: line, marker: "", kind: kind, oldLine: nil, newLine: nil)
        }
    }

    private func displayDiffRows(_ rows: [DiffRow]) -> [DiffRow] {
        let sourceRows = rows.filter { row in
            switch row.kind {
            case .metadata, .fileHeader: false
            default: true
            }
        }
        return sourceRows.isEmpty ? rows : sourceRows
    }

    private func diffCanvasHeight(_ rows: [DiffRow]) -> CGFloat {
        max(48, rows.reduce(0) { $0 + diffRowHeight($1.kind) })
    }

    private var selectedRelativePath: String? {
        if sourceMode == .changes,
           let id = selectedChangeID,
           let row = model.worktree.first(where: { $0.id == id }) {
            return row.path
        }
        if sourceMode == .repositories,
           let id = selectedCommitFileID,
           let row = model.commitFiles.first(where: { $0.id == id }) {
            return row.path
        }
        return diffTitle
    }

    private var selectedFilePath: String? {
        guard let relative = selectedRelativePath, !relative.isEmpty else { return nil }
        if (relative as NSString).isAbsolutePath { return (relative as NSString).standardizingPath }
        let root = model.rootPath.isEmpty
            ? (engine.projectRoot.isEmpty ? engine.chrome.explorer.cwd : engine.projectRoot)
            : model.rootPath
        guard !root.isEmpty else { return nil }
        return ((root as NSString).appendingPathComponent(relative) as NSString).standardizingPath
    }

    private var selectedFileExists: Bool {
        guard let path = selectedFilePath else { return false }
        return FileManager.default.fileExists(atPath: path)
    }

    private func openSelectedFile() {
        guard let path = selectedFilePath, selectedFileExists else { return }
        _ = engine.openPath(path)
    }

    private func revealSelectedFile() {
        guard let path = selectedFilePath, selectedFileExists else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    private func copySelectedFilePath() {
        guard let path = selectedFilePath else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(path, forType: .string)
    }

    private func absolutePath(_ relative: String) -> String {
        if (relative as NSString).isAbsolutePath { return relative }
        return (model.rootPath as NSString).appendingPathComponent(relative)
    }

    private func openFile(_ relative: String) {
        let path = absolutePath(relative)
        guard FileManager.default.fileExists(atPath: path) else { return }
        _ = engine.openPath(path)
    }

    private func revealFile(_ relative: String) {
        let path = absolutePath(relative)
        guard FileManager.default.fileExists(atPath: path) else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    private func copyPath(_ relative: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(absolutePath(relative), forType: .string)
    }

    private func statusBadge(_ status: String, selected: Bool) -> some View {
        Text(status == "?" ? "U" : status)
            .font(.system(size: 9.5, weight: .semibold, design: .rounded))
            .foregroundStyle(selected ? Color.white : statusColor(status))
            .frame(minWidth: 14)
            .help(statusTitle(status))
    }

    private func fileSymbol(_ path: String) -> String {
        switch URL(fileURLWithPath: path).pathExtension.lowercased() {
        case "swift", "rs", "c", "h", "cpp", "hpp", "m", "mm": "curlybraces"
        case "sh", "zsh", "bash", "fish": "terminal"
        case "md", "markdown", "txt", "rst": "doc.richtext"
        case "png", "jpg", "jpeg", "gif", "heic", "svg": "photo"
        case "json", "toml", "yaml", "yml", "plist": "list.bullet.rectangle"
        default: "doc.text"
        }
    }

    private func fileTint(_ path: String) -> Color {
        switch URL(fileURLWithPath: path).pathExtension.lowercased() {
        case "swift": .orange
        case "rs": .brown
        case "md", "markdown": .blue
        case "json", "toml", "yaml", "yml": .purple
        case "png", "jpg", "jpeg", "gif", "heic", "svg": .pink
        default: theme.color(theme.accent)
        }
    }

    private func statusColor(_ status: String) -> Color {
        switch status {
        case "M": .orange
        case "A": .green
        case "D": .red
        case "R": .blue
        case "?": .orange
        case "U": .purple
        default: .secondary
        }
    }

    private func statusTitle(_ status: String) -> String {
        switch status {
        case "M": "Modified"
        case "A": "Added"
        case "D": "Deleted"
        case "R": "Renamed"
        case "?": "Untracked"
        case "U": "Unmerged"
        default: "Changed"
        }
    }

    private func diffStartLine(_ field: Substring) -> Int {
        Int(field.dropFirst().split(separator: ",", maxSplits: 1)[0]) ?? 0
    }

    private func diffRowHeight(_ kind: DiffRowKind) -> CGFloat {
        switch kind {
        case .metadata, .fileHeader, .note: 21
        case .hunk: 24
        default: 20
        }
    }

}

/// Virtualized native diff surface. `NSTableView` creates views only for the
/// visible rows and reuses them while scrolling, so a 10,000-line diff has the
/// same live view count as a 30-line diff. The previous SwiftUI `ForEach`
/// appeared lazy, but accessibility/text selection still materialized a large
/// subtree and forced the UI into artificial 60-line pages.
private struct GitDiffTableView: NSViewRepresentable {
    let rows: [DiffRow]
    let historical: Bool
    let untracked: Bool
    let light: Bool
    let accent: UInt32

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = LayoutAwareScrollView()
        scroll.drawsBackground = true
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        scroll.borderType = .noBorder
        scroll.scrollerStyle = .overlay

        let table = NSTableView(frame: .zero)
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("diff"))
        column.minWidth = 240
        table.addTableColumn(column)
        table.headerView = nil
        table.intercellSpacing = .zero
        table.rowSizeStyle = .small
        table.columnAutoresizingStyle = .noColumnAutoresizing
        table.selectionHighlightStyle = .none
        table.allowsEmptySelection = true
        table.allowsMultipleSelection = false
        table.focusRingType = .none
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        scroll.documentView = table

        context.coordinator.table = table
        context.coordinator.column = column
        scroll.onLayout = { [weak coordinator = context.coordinator, weak scroll] in
            guard let coordinator, let scroll else { return }
            coordinator.updateColumnWidth(in: scroll)
        }
        context.coordinator.update(from: self, in: scroll)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.update(from: self, in: scroll)
    }

    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        weak var table: NSTableView?
        weak var column: NSTableColumn?
        private var rows: [DiffRow] = []
        private var historical = false
        private var untracked = false
        private var light = false
        private var accent = NSColor.controlAccentColor
        private var measuredContentWidth: CGFloat = 420

        func update(from parent: GitDiffTableView, in scroll: NSScrollView) {
            let nextAccent = Self.color(parent.accent)
            let dataChanged = rows != parent.rows
                || historical != parent.historical
                || untracked != parent.untracked
                || light != parent.light
                || accent != nextAccent

            rows = parent.rows
            historical = parent.historical
            untracked = parent.untracked
            light = parent.light
            accent = nextAccent
            scroll.backgroundColor = .textBackgroundColor
            table?.backgroundColor = .textBackgroundColor

            if dataChanged {
                let longest = rows.lazy.map { $0.content.utf16.count }.max() ?? 0
                // Monospaced 11pt text is ~6.7pt/cell. Cap only the backing
                // view extent, not the data, to avoid pathological generated
                // one-line files allocating a million-point AppKit surface.
                measuredContentWidth = min(32_768, max(420, 116 + CGFloat(longest) * 6.9))
                table?.reloadData()
            }
            updateColumnWidth(in: scroll)
        }

        func updateColumnWidth(in scroll: NSScrollView) {
            guard let table, let column else { return }
            let desired = max(scroll.contentSize.width, measuredContentWidth)
            guard abs(column.width - desired) > 0.5 else { return }
            column.width = desired
            var frame = table.frame
            frame.size.width = desired
            table.frame = frame
        }

        func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

        func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
            guard rows.indices.contains(row) else { return 20 }
            switch rows[row].kind {
            case .metadata, .fileHeader, .note: return 21
            case .hunk: return 24
            default: return 20
            }
        }

        func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }

        func tableView(
            _ tableView: NSTableView,
            viewFor tableColumn: NSTableColumn?,
            row index: Int
        ) -> NSView? {
            guard rows.indices.contains(index) else { return nil }
            let identifier = NSUserInterfaceItemIdentifier("GitDiffCell")
            let cell = tableView.makeView(withIdentifier: identifier, owner: nil) as? GitDiffCell
                ?? GitDiffCell(identifier: identifier)
            cell.configure(
                row: rows[index],
                historical: historical,
                untracked: untracked,
                light: light,
                accent: accent
            )
            return cell
        }

        private static func color(_ packed: UInt32) -> NSColor {
            let rawAlpha = CGFloat((packed >> 24) & 0xFF) / 255
            let alpha = rawAlpha == 0 ? 1 : rawAlpha
            return NSColor(
                calibratedRed: CGFloat((packed >> 16) & 0xFF) / 255,
                green: CGFloat((packed >> 8) & 0xFF) / 255,
                blue: CGFloat(packed & 0xFF) / 255,
                alpha: alpha
            )
        }
    }
}

private final class LayoutAwareScrollView: NSScrollView {
    var onLayout: (() -> Void)?

    override func layout() {
        super.layout()
        onLayout?()
    }
}

private final class GitDiffCell: NSTableCellView {
    private let gutter = NSTextField(labelWithString: "")
    private let bar = NSView()
    private let marker = NSTextField(labelWithString: "")
    private let source = NSTextField(labelWithString: "")

    init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(frame: .zero)
        self.identifier = identifier
        wantsLayer = true

        gutter.alignment = .right
        gutter.font = .monospacedSystemFont(ofSize: 10, weight: .regular)
        gutter.textColor = .tertiaryLabelColor
        gutter.drawsBackground = true
        gutter.isBezeled = false

        marker.alignment = .center
        marker.font = .monospacedSystemFont(ofSize: 11, weight: .semibold)

        source.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        source.isEditable = false
        source.isSelectable = true
        source.isBordered = false
        source.drawsBackground = false
        source.lineBreakMode = .byClipping
        source.maximumNumberOfLines = 1
        source.cell?.wraps = false
        source.cell?.usesSingleLineMode = true
        source.focusRingType = .none

        for view in [gutter, bar, marker, source] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        bar.wantsLayer = true
        NSLayoutConstraint.activate([
            gutter.leadingAnchor.constraint(equalTo: leadingAnchor),
            gutter.topAnchor.constraint(equalTo: topAnchor),
            gutter.bottomAnchor.constraint(equalTo: bottomAnchor),
            gutter.widthAnchor.constraint(equalToConstant: 76),

            bar.leadingAnchor.constraint(equalTo: gutter.trailingAnchor),
            bar.topAnchor.constraint(equalTo: topAnchor),
            bar.bottomAnchor.constraint(equalTo: bottomAnchor),
            bar.widthAnchor.constraint(equalToConstant: 3),

            marker.leadingAnchor.constraint(equalTo: bar.trailingAnchor),
            marker.topAnchor.constraint(equalTo: topAnchor),
            marker.bottomAnchor.constraint(equalTo: bottomAnchor),
            marker.widthAnchor.constraint(equalToConstant: 20),

            source.leadingAnchor.constraint(equalTo: marker.trailingAnchor),
            source.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            source.centerYAnchor.constraint(equalTo: centerYAnchor)
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(
        row: DiffRow,
        historical: Bool,
        untracked: Bool,
        light: Bool,
        accent: NSColor
    ) {
        let old = row.oldLine.map(String.init) ?? ""
        let new = row.newLine.map(String.init) ?? ""
        gutter.stringValue = String(format: "%4@ %4@  ", old, new)
        gutter.backgroundColor = NSColor.labelColor.withAlphaComponent(light ? 0.018 : 0.035)
        marker.stringValue = row.marker
        source.stringValue = row.content

        let green = NSColor.systemGreen
        let red = NSColor.systemRed
        let background: NSColor
        if !historical || untracked {
            switch row.kind {
            case .addition, .deletion:
                background = NSColor.labelColor.withAlphaComponent(light ? 0.025 : 0.045)
            case .hunk:
                background = accent.withAlphaComponent(light ? 0.08 : 0.13)
            default:
                background = .clear
            }
        } else {
            switch row.kind {
            case .addition: background = green.withAlphaComponent(light ? 0.11 : 0.16)
            case .deletion: background = red.withAlphaComponent(light ? 0.09 : 0.14)
            case .hunk: background = accent.withAlphaComponent(light ? 0.08 : 0.13)
            default: background = .clear
            }
        }
        layer?.backgroundColor = background.cgColor

        switch row.kind {
        case .addition:
            bar.layer?.backgroundColor = (historical ? green : accent).cgColor
            marker.textColor = historical ? green : .secondaryLabelColor
            source.textColor = historical ? green : .labelColor
        case .deletion:
            bar.layer?.backgroundColor = (historical ? red : accent).cgColor
            marker.textColor = historical ? red : .secondaryLabelColor
            source.textColor = historical ? red : .labelColor
        case .hunk:
            bar.layer?.backgroundColor = accent.withAlphaComponent(0.8).cgColor
            marker.textColor = accent
            source.textColor = accent
            source.font = .monospacedSystemFont(ofSize: 10.5, weight: .semibold)
        case .metadata, .fileHeader, .note:
            bar.layer?.backgroundColor = NSColor.clear.cgColor
            marker.textColor = .secondaryLabelColor
            source.textColor = .secondaryLabelColor
            source.font = .monospacedSystemFont(ofSize: 10.5, weight: .regular)
        case .context:
            bar.layer?.backgroundColor = NSColor.clear.cgColor
            marker.textColor = .secondaryLabelColor
            source.textColor = .labelColor
        }
        if row.kind != .hunk && row.kind != .metadata
            && row.kind != .fileHeader && row.kind != .note {
            source.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        }
    }
}

/// AppKit owns the native toolbar search bezel, clear button and focus ring.
/// Keeping it inside the principal toolbar cluster avoids `.searchable`
/// inserting a flexible spacer and marooning the field at the window edge.
private struct GitWorkbenchToolbarSearchField: NSViewRepresentable {
    @Binding var text: String
    var prompt: String

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSSearchField {
        let field = NSSearchField(frame: .zero)
        field.delegate = context.coordinator
        field.placeholderString = prompt
        field.sendsSearchStringImmediately = true
        field.sendsWholeSearchString = false
        field.controlSize = .regular
        field.font = .systemFont(ofSize: 12)
        return field
    }

    func updateNSView(_ field: NSSearchField, context: Context) {
        context.coordinator.parent = self
        field.placeholderString = prompt
        if field.stringValue != text {
            field.stringValue = text
        }
    }

    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var parent: GitWorkbenchToolbarSearchField

        init(_ parent: GitWorkbenchToolbarSearchField) {
            self.parent = parent
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSSearchField else { return }
            parent.text = field.stringValue
        }
    }
}
