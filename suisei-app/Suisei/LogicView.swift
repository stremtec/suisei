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

struct LogicSnap: Equatable {
    var path: String = ""
    var lang: String = ""
    /// Why the list is empty, when it is. "Nothing here" and "I could not read
    /// this" are different facts and want different reactions.
    var note: String = ""
    var live: Bool = false
    var selected: Int = 0
    var rows: [LogicRowSnap] = []

    static let empty = LogicSnap()
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
            Image(systemName: "point.topleft.down.to.point.bottomright.curvepath")
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
