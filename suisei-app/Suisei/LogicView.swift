//  LogicView.swift
//  The control flow of a file, drawn as a spine.
//
//  `suisei_core::logic` reads the graph off the syntax tree and
//  `suisei_core::logic_view` keeps one session per file; this is the face for
//  it. Rows in, spine out — and the rules it draws by are argued in
//  docs/SUISEI-LOGIC-VIEW-PLAN.md §6a. In short:
//
//  · **Not a canvas.** Nothing else in Suisei pans, and nothing else is laid
//    out by a graph algorithm. A program's logic is mostly a vertical
//    sequence, and this app draws vertical sequences.
//  · **The spine is the app's own shape.** The git change bar and the value
//    bracket are one object — a vertical rule with caps, meaning "this run,
//    together". A branch splits it into two indented spines; a loop carries a
//    back mark; an exit ends its spine rather than rejoining.
//  · **Labels are source text**, in the editor's own monospace. A paraphrase
//    is a second thing that can disagree with the code.
//  · **Runtime is amber** — the debugger's colour here, so a lit row and the
//    editor's stopped line read as one fact.
//  · **Nothing moves that the code did not move.** Rows in source order,
//    indentation for nesting, no auto-layout: the same function must look the
//    same after an edit or the reader loses the map they built.

import AppKit
import SwiftUI

// MARK: - Model

enum LogicKind: UInt8 {
    case entry = 0, process = 1, decision = 2, loop = 3, exit = 4, opaque = 5
    init(raw: UInt8) { self = LogicKind(rawValue: raw) ?? .opaque }
}

enum LogicEdge: UInt8 {
    case next = 0, yes = 1, no = 2, back = 3
    init(raw: UInt8) { self = LogicEdge(rawValue: raw) ?? .next }
}

struct LogicRowSnap: Identifiable, Equatable {
    var id: Int
    var kind: LogicKind
    var edge: LogicEdge
    var label: String
    /// The locals this row names, at the stop. Empty unless the program is here.
    var values: String
    var depth: Int
    var startRow: UInt32
    var endRow: UInt32
    var expandable: Bool
    var expanded: Bool
    var stopped: Bool
    var enclosing: Bool
    var caller: Bool
    var breakpoint: Bool
}

/// A run of source rows the editor should mark.
///
/// Runs, not rows: a guide down a block is one object, and assembling it
/// inside a row loop is the mistake the git bar's own comment warns about.
struct LogicRun: Equatable {
    var startRow: UInt32
    var endRow: UInt32
    /// Visual column for the guide — the node's own indentation.
    var col: Int
    /// The reader is pointing at this: the accent.
    var selected: Bool
    /// The program is stopped inside this: amber, the debugger's voice.
    var runtime: Bool
    /// One arm of the branch the pointer is over: the way taken when the test
    /// holds, and the way taken when it does not.
    var arm: Bool?
}

struct LogicSnap: Equatable {
    var path: String = ""
    var lang: String = ""
    /// Why the list is empty, when it is. "Nothing here" and "I could not read
    /// this" are different facts and want different reactions.
    var note: String = ""
    var live: Bool = false
    var selected: Int = 0
    var rows: [LogicRowSnap] = []
    /// What the EDITOR draws. Empty unless something is selected.
    var runs: [LogicRun] = []

    static let empty = LogicSnap()
}

// MARK: - The rail

/// One line of the rail: a node, or a fold of several ordinary steps.
private struct LogicEntry: Identifiable {
    enum Kind {
        case row(LogicRowSnap)
        /// How many, where the first one is, and whether the caret is in there.
        case steps(Int, Int, Bool)
    }
    let id: String
    let depth: Int
    let kind: Kind
}

//
// The right-rail form, and the primary one. §6b of the plan: in a pane the
// view must reproduce the code, because the code is not on screen — and
// reproducing indented source in 240pt is hopeless, which is what the pane was
// for. Beside the editor the code IS on screen, so this carries the SHAPE and
// the editor carries the TEXT. That is the debugger's own division of labour:
// the panel holds the tree, the editor holds the marks.
//
// So: no file name, no language, no banner. Everything the rail could say
// twice is already being said one column to the left.

struct LogicRail: View {
    /// `chrome.filename` verbatim, which is a path OR the literal
    /// `[No Name]` for a buffer that has never been saved. A view of a file
    /// needs a file, so anything that is not an absolute path is no file.
    let rawPath: String
    let palette: ViewerPalette
    /// Where the caret is, so the rail can follow it. The editor is the
    /// authority on where the reader is; this is the view catching up.
    ///
    /// **One-based** — `chrome.cursor_row` is `cursor.row + 1`, and every row
    /// in the logic tree is zero-based, which is the off-by-one this converts
    /// once rather than at four call sites.
    let cursorRow: UInt32
    @ObservedObject private var engine = EngineBridge.shared

    private var path: String { rawPath.hasPrefix("/") ? rawPath : "" }
    private var caretLine: Int { max(0, Int(cursorRow) - 1) }
    private var snap: LogicSnap { engine.logic[path] ?? .empty }

    var body: some View {
        Group {
            if path.isEmpty {
                note("Save the file to read its logic")
            } else if snap.rows.isEmpty {
                note(snap.note.isEmpty ? "Nothing to read here" : snap.note)
            } else {
                list
            }
        }
        .onAppear {
            engine.watchLogic(path)
            engine.logicFollow(path, caretLine)
        }
        .onDisappear { engine.unwatchLogic(path) }
        .onChange(of: path) { old, new in
            engine.unwatchLogic(old)
            engine.watchLogic(new)
            engine.logicFollow(new, caretLine)
        }
        .onChange(of: cursorRow) { _, _ in engine.logicFollow(path, caretLine) }
        .contextMenu {
            Button("Open in a Pane") { engine.openLogicView() }
        }
    }

    private var list: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(entries) { entry in
                        switch entry.kind {
                        case .row(let row):
                            LogicRailRow(
                                row: row,
                                selected: row.id == snap.selected,
                                palette: palette,
                                onTap: { engine.logicReveal(path, row.id) },
                                onToggle: { engine.logicToggle(path, row.id) },
                                onPeek: { on in engine.logicPeek(path, on ? row.id : nil) }
                            )
                            .id(row.id)
                        case .steps(let count, let first, let holdsSelection):
                            LogicStepsRow(
                                count: count,
                                depth: entry.depth,
                                selected: holdsSelection,
                                palette: palette,
                                onTap: { engine.logicReveal(path, first) }
                            )
                            .id(entry.id)
                        }
                    }
                }
                .padding(.vertical, 4)
            }
            // Following the caret is only half of following: a row selected
            // below the fold is a selection nobody can see.
            .onChange(of: snap.selected) { _, i in
                withAnimation(.easeOut(duration: 0.18)) { proxy.scrollTo(i, anchor: .center) }
            }
        }
    }

    /// What the rail actually draws, which is not one row per node.
    ///
    /// A run of ordinary steps is folded into a single quiet line. This is the
    /// change that made the rail readable: `let old_score = self.score;` is
    /// not logic — it is text, and the text is one column to the left. What is
    /// left after folding is the shape: the branches, the loops, the ways out,
    /// and the calls, which are the edges to somewhere else.
    ///
    /// A fold is never broken open for the selection. If the caret is on an
    /// ordinary step, the fold holding it is what lights up — the exact line
    /// is already under the caret, and a list that reflows as the caret moves
    /// is a list nobody can keep their place in.
    private var entries: [LogicEntry] {
        var out: [LogicEntry] = []
        var run: [LogicRowSnap] = []

        func flush() {
            guard let first = run.first else { return }
            if run.count == 1 {
                out.append(LogicEntry(id: "row-\(first.id)", depth: first.depth, kind: .row(first)))
            } else {
                let holds = run.contains { $0.id == snap.selected }
                out.append(LogicEntry(
                    id: "steps-\(first.id)",
                    depth: first.depth,
                    kind: .steps(run.count, first.id, holds)
                ))
            }
            run.removeAll()
        }

        for row in snap.rows {
            // Foldable: an ordinary step that goes nowhere and says nothing
            // about control flow. A call is an edge and stays; anything the
            // extractor could not read stays, because dropping it quietly is
            // the one thing this view must never do.
            let plain = row.kind == .process && !row.expandable
                && !row.stopped && !row.enclosing && !row.caller && !row.breakpoint
            if plain, run.last.map({ $0.depth == row.depth }) ?? true {
                run.append(row)
                continue
            }
            flush()
            out.append(LogicEntry(id: "row-\(row.id)", depth: row.depth, kind: .row(row)))
        }
        flush()
        return out
    }

    private func note(_ text: String) -> some View {
        VStack(spacing: 6) {
            Image(systemName: "smallcircle.filled.circle")
                .font(.system(size: 18))
                .foregroundStyle(.tertiary)
            // Never a bare emptiness: a language with no table, a file that
            // would not parse and a file with no functions are three facts.
            Text(text)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 10)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// One row of the rail.
///
/// The first version drew every node the same: one size, one weight, one
/// colour, a marker glyph on each, a line number on each. Forty of those is a
/// wall — "읽기 불편함" — and nothing in it tells the eye where to stop.
///
/// So the hierarchy is carried by TYPE rather than by decoration. A function
/// is the anchor and reads as one: heavier, brighter, with air above it. A
/// step is quieter than the function it is in, and the ways out of a function
/// are brighter than the steps between them. Nothing is bold that is not
/// worth stopping at.
private struct LogicRailRow: View {
    let row: LogicRowSnap
    let selected: Bool
    let palette: ViewerPalette
    let onTap: () -> Void
    let onToggle: () -> Void
    let onPeek: (Bool) -> Void

    @State private var hovering = false

    private static let step: CGFloat = 12
    private var isFunction: Bool { row.kind == .entry }

    var body: some View {
        HStack(spacing: 0) {
            rails
            // Before the shape, not after it: the row reads "on the YES side,
            // this leaves the function", and that is the order the words go in.
            arm
            marker
            Text(row.label)
                .font(.system(size: isFunction ? 11.5 : 10.5, design: .monospaced))
                .fontWeight(isFunction ? .medium : .regular)
                .foregroundStyle(labelColor)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 4)
            trailing
        }
        .padding(.horizontal, 6)
        .frame(height: isFunction ? 24 : 20)
        // Air above a function, and only there: it is what turns one list of
        // fifty lines into five groups of ten.
        .padding(.top, isFunction ? 6 : 0)
        .background(background)
        .contentShape(Rectangle())
        .onHover { over in
            hovering = over
            // Only a branch has arms to show, and asking about anything else
            // would clear the answer the reader is looking at.
            if row.kind == .decision { onPeek(over) }
        }
        .onTapGesture { onTap() }
        .help(row.label)
    }

    /// The spine, one segment per level above this row.
    ///
    /// Faint on purpose. It is there to say what belongs to what, and a rule
    /// that competes with the text it is organising has stopped organising it.
    private var rails: some View {
        HStack(spacing: 0) {
            ForEach(0..<row.depth, id: \.self) { level in
                Rectangle()
                    .fill(railColor(level))
                    .frame(width: 1)
                    .frame(width: Self.step, alignment: .leading)
            }
        }
        .frame(maxHeight: .infinity)
    }

    /// A shape for what this is — and no shape where the label already says
    /// it. A bullet in front of every ordinary step is fifty bullets.
    private var marker: some View {
        Group {
            switch row.kind {
            case .entry:
                Image(systemName: row.expanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 8, weight: .semibold))
            case .decision:
                // The flowchart's rhombus, not a version-control branch: the
                // left rail's Source Control tab owns that glyph.
                Image(systemName: "diamond").font(.system(size: 7.5, weight: .medium))
            case .loop:
                Image(systemName: "arrow.trianglehead.clockwise").font(.system(size: 8.5))
            case .exit:
                Image(systemName: "arrow.uturn.left").font(.system(size: 8.5))
            case .opaque:
                Image(systemName: "questionmark").font(.system(size: 8))
            case .process:
                // A call: an edge to somewhere else, and the only ordinary
                // step that survives the fold.
                Image(systemName: "arrow.right").font(.system(size: 8))
                    .opacity(row.expandable ? 1 : 0)
            }
        }
        .foregroundStyle(markerColor)
        .frame(width: 15, alignment: .leading)
        .contentShape(Rectangle())
        .onTapGesture { row.expandable ? onToggle() : onTap() }
    }

    /// `Y` / `N` — a letter, not a chip. A pill behind every arm is a second
    /// decoration on the one row that already has a shape and an indent.
    @ViewBuilder
    private var arm: some View {
        if row.edge == .yes || row.edge == .no {
            Text(row.edge == .yes ? "Y" : "N")
                .font(.system(size: 8.5, weight: .semibold))
                .foregroundStyle(palette.dim.opacity(0.85))
                .frame(width: 9)
                .padding(.trailing, 5)
        }
    }

    /// The line number earns its place on a function, which is a destination.
    /// On a step it is forty more grey digits, so it waits to be asked for.
    private var trailing: some View {
        HStack(spacing: 5) {
            if row.breakpoint {
                Circle().fill(LogicPaneViewer.amber).frame(width: 5, height: 5)
            }
            if row.caller {
                Image(systemName: "arrow.turn.down.right")
                    .font(.system(size: 7, weight: .semibold))
                    .foregroundStyle(LogicPaneViewer.amber.opacity(0.8))
            }
            if isFunction || selected || hovering {
                Text("\(row.startRow + 1)")
                    .font(.system(size: 9, design: .monospaced))
                    .foregroundStyle(palette.dim.opacity(isFunction ? 0.7 : 0.5))
            }
        }
    }

    @ViewBuilder
    private var background: some View {
        if row.stopped {
            // The band the editor paints across the stopped line. Two views of
            // one fact should not need translating between them.
            LogicPaneViewer.amber.opacity(0.17)
        } else if selected {
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(palette.fg.opacity(0.08))
                .padding(.horizontal, 2)
        } else if hovering {
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(palette.fg.opacity(0.04))
                .padding(.horizontal, 2)
        } else {
            Color.clear
        }
    }

    /// The rail for one level above this row. Amber where the program is
    /// inside it — the runtime path drawn as the structure it runs through
    /// rather than as a second list.
    private func railColor(_ level: Int) -> Color {
        if (row.stopped || row.enclosing) && level >= row.depth - 1 {
            return LogicPaneViewer.amber.opacity(0.5)
        }
        return palette.dim.opacity(0.18)
    }

    private var markerColor: Color {
        if row.stopped { return LogicPaneViewer.amber }
        switch row.kind {
        case .entry: return palette.dim.opacity(0.8)
        case .opaque: return palette.dim.opacity(0.5)
        case .decision, .loop: return palette.fg.opacity(0.5)
        case .exit: return palette.fg.opacity(0.45)
        case .process: return palette.dim.opacity(0.55)
        }
    }

    private var labelColor: Color {
        if row.stopped { return palette.fg }
        switch row.kind {
        // The anchor. Everything else in the list is subordinate to it.
        case .entry: return palette.fg.opacity(0.95)
        // Visibly less certain than the rest, because it is: it means "there
        // is something here I did not read".
        case .opaque: return palette.dim.opacity(0.6)
        case .decision, .loop, .exit: return palette.fg.opacity(0.72)
        case .process: return palette.fg.opacity(0.6)
        }
    }
}

/// A run of ordinary steps, folded.
///
/// It says how many and nothing else. That is the honest amount: these lines
/// do not branch, do not loop and do not leave — and if the reader wants to
/// know what they say, the text is one column to the left, which is what the
/// rail exists to leave room for.
private struct LogicStepsRow: View {
    let count: Int
    let depth: Int
    let selected: Bool
    let palette: ViewerPalette
    let onTap: () -> Void

    @State private var hovering = false

    var body: some View {
        HStack(spacing: 0) {
            ForEach(0..<depth, id: \.self) { _ in
                Rectangle()
                    .fill(palette.dim.opacity(0.18))
                    .frame(width: 1)
                    .frame(width: 12, alignment: .leading)
            }
            Color.clear.frame(width: 15)
            Text("\(count) steps")
                .font(.system(size: 9.5))
                .foregroundStyle(palette.dim.opacity(selected ? 0.85 : 0.5))
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 6)
        .frame(height: 18)
        .background(
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(palette.fg.opacity(selected ? 0.07 : (hovering ? 0.04 : 0)))
                .padding(.horizontal, 2)
        )
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .onTapGesture { onTap() }
    }
}

// MARK: - The pane

struct LogicPaneViewer: View {
    let path: String
    let palette: ViewerPalette
    @ObservedObject private var engine = EngineBridge.shared

    private var snap: LogicSnap { engine.logic[path] ?? .empty }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider().opacity(0.28)
            if snap.rows.isEmpty {
                emptyState
            } else {
                rows
            }
        }
        .background(palette.bg)
        .onAppear { engine.watchLogic(path) }
        .onDisappear { engine.unwatchLogic(path) }
    }

    // The workbench's word for "this is a section": uppercase, small, tracked.
    private var header: some View {
        HStack(spacing: 8) {
            Text("LOGIC")
                .font(.system(size: 9, weight: .semibold))
                .tracking(1.1)
                .foregroundStyle(palette.dim)
            Text((path as NSString).lastPathComponent)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(palette.fg)
                .lineLimit(1)
                .truncationMode(.middle)
            if snap.live {
                // The debugger's word, in the debugger's colour, so a lit row
                // here and the stopped line in the editor are one fact.
                Text("STOPPED HERE")
                    .font(.system(size: 9, weight: .semibold))
                    .tracking(0.8)
                    .foregroundStyle(Self.amber)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Self.amber.opacity(0.14), in: Capsule())
            }
            Spacer(minLength: 0)
            if !snap.lang.isEmpty {
                Text(snap.lang)
                    .font(.system(size: 10))
                    .foregroundStyle(palette.dim)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    private var emptyState: some View {
        VStack(spacing: 6) {
            Spacer()
            Image(systemName: "smallcircle.filled.circle")
                .font(.system(size: 22, weight: .light))
                .foregroundStyle(palette.dim.opacity(0.7))
            // The note, never a bare emptiness: a language with no table, a
            // file that would not parse and a file with no functions in it are
            // three different facts.
            Text(snap.note.isEmpty ? "Nothing to read here" : snap.note)
                .font(.system(size: 11))
                .foregroundStyle(palette.dim)
            Spacer()
        }
        .frame(maxWidth: .infinity)
    }

    private var rows: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(snap.rows) { row in
                    LogicRowView(
                        row: row,
                        selected: row.id == snap.selected,
                        palette: palette,
                        onToggle: { engine.logicToggle(path, row.id) },
                        onReveal: { engine.logicReveal(path, row.id) }
                    )
                }
            }
            .padding(.vertical, 6)
        }
    }

    /// One family per subject. Git owns green and red in the gutter, the
    /// accent belongs to the user, and the debugger speaks amber — the stop
    /// band, the breakpoint chip, the datatip and the inline values all
    /// already do.
    static let amber = Color(red: 1.0, green: 0.72, blue: 0.16)
}

// MARK: - One row

private struct LogicRowView: View {
    let row: LogicRowSnap
    let selected: Bool
    let palette: ViewerPalette
    let onToggle: () -> Void
    let onReveal: () -> Void

    @State private var hovering = false

    private var indent: CGFloat { CGFloat(row.depth) * 18 }

    var body: some View {
        HStack(spacing: 0) {
            Color.clear.frame(width: indent)
            spine
            marker
            label
            Spacer(minLength: 8)
            trailing
        }
        .padding(.horizontal, 12)
        .frame(height: 24)
        .background(background)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        // One click selects and takes you to the code — the pairing is the
        // whole interaction, and asking for a second click to see the line a
        // row IS would be a step in the way of it.
        .onTapGesture { onReveal() }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(kindName) \(row.label)")
        .accessibilityAddTraits(row.expandable ? .isButton : [])
    }

    /// The vertical rule this whole view is built on: the git bar's shape and
    /// the value bracket's, saying the same thing they say — these belong to
    /// one run.
    private var spine: some View {
        ZStack(alignment: .center) {
            Rectangle()
                .fill(spineColor)
                .frame(width: 1.5)
            if row.kind == .loop {
                // A loop goes round. The mark is the back edge, and it is the
                // one place the spine is allowed to say something twice.
                Image(systemName: "arrow.trianglehead.counterclockwise")
                    .font(.system(size: 8, weight: .semibold))
                    .foregroundStyle(spineColor)
                    .background(Circle().fill(palette.bg).frame(width: 13, height: 13))
            }
        }
        .frame(width: 14, height: 24)
    }

    /// What kind of step this is, as a SHAPE first — Increase Contrast and a
    /// colour-blind reader both get the same answer from it.
    private var marker: some View {
        Group {
            switch row.kind {
            case .entry:
                Image(systemName: row.expanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
            case .decision:
                Image(systemName: "diamond")
                    .font(.system(size: 8, weight: .semibold))
            case .loop:
                Image(systemName: "repeat")
                    .font(.system(size: 8, weight: .semibold))
            case .exit:
                Image(systemName: "arrow.turn.down.right")
                    .font(.system(size: 8, weight: .semibold))
            case .opaque:
                Image(systemName: "ellipsis")
                    .font(.system(size: 8, weight: .semibold))
            case .process:
                Circle().frame(width: 3, height: 3)
            }
        }
        .foregroundStyle(markerColor)
        .frame(width: 16)
        .contentShape(Rectangle())
        .onTapGesture { row.expandable ? onToggle() : onReveal() }
    }

    private var label: some View {
        HStack(spacing: 6) {
            if row.edge == .yes || row.edge == .no {
                // An arm has to read as an arm rather than as the next
                // statement — it is the one thing a flat list of source lines
                // cannot say, and the reason the graph's edges are carried
                // across at all.
                Text(row.edge == .yes ? "YES" : "NO")
                    .font(.system(size: 8, weight: .bold))
                    .tracking(0.6)
                    .foregroundStyle(palette.dim)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 1)
                    .background(palette.dim.opacity(0.12), in: RoundedRectangle(cornerRadius: 3))
            }
            // Source text in the source's own face. A paraphrase would be a
            // second thing that can disagree with the code, and the code is
            // what the reader is here to understand.
            Text(row.label)
                .font(.custom("JetBrains Mono", size: 11.5).monospaced())
                .foregroundStyle(labelColor)
                .lineLimit(1)
                .truncationMode(.tail)
        }
    }

    @ViewBuilder
    private var trailing: some View {
        HStack(spacing: 8) {
            if !row.values.isEmpty {
                // The value at this row, at the stop. The editor draws exactly
                // this at the end of the line, and it is the same read of the
                // same frame.
                Text(row.values)
                    .font(.custom("JetBrains Mono", size: 10).monospaced())
                    .foregroundStyle(LogicPaneViewer.amber)
                    .lineLimit(1)
            }
            if row.breakpoint {
                Circle()
                    .fill(LogicPaneViewer.amber)
                    .frame(width: 6, height: 6)
            }
            if row.caller {
                // The way in: exact, off the call stack, not inferred.
                Image(systemName: "arrow.turn.down.right")
                    .font(.system(size: 8, weight: .semibold))
                    .foregroundStyle(LogicPaneViewer.amber.opacity(0.8))
            }
            if hovering && row.expandable && row.kind != .entry {
                Image(systemName: row.expanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 8, weight: .semibold))
                    .foregroundStyle(palette.dim)
                    .onTapGesture { onToggle() }
            }
        }
    }

    @ViewBuilder
    private var background: some View {
        if row.stopped {
            // The same band the editor paints across the stopped line. Two
            // views of one fact should not need translating between them.
            LogicPaneViewer.amber.opacity(0.17)
        } else if selected {
            palette.fg.opacity(0.07)
        } else if hovering {
            palette.fg.opacity(0.04)
        } else {
            Color.clear
        }
    }

    private var spineColor: Color {
        if row.stopped || row.enclosing { return LogicPaneViewer.amber.opacity(0.75) }
        return palette.dim.opacity(0.35)
    }

    private var markerColor: Color {
        if row.stopped { return LogicPaneViewer.amber }
        switch row.kind {
        case .opaque: return palette.dim.opacity(0.6)
        case .decision, .loop, .exit: return palette.fg.opacity(0.75)
        default: return palette.dim
        }
    }

    private var labelColor: Color {
        if row.stopped { return palette.fg }
        // Opaque is visibly less certain than the rest, because it is: it
        // means "there is something here I did not read".
        if row.kind == .opaque { return palette.dim.opacity(0.75) }
        if row.kind == .entry { return palette.fg }
        return palette.fg.opacity(0.86)
    }

    private var kindName: String {
        switch row.kind {
        case .entry: return "function"
        case .process: return "step"
        case .decision: return "branch"
        case .loop: return "loop"
        case .exit: return "exit"
        case .opaque: return "unread"
        }
    }
}
