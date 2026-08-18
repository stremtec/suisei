//  ConsolePane.swift
//  One console, for the two things in this app that print.
//
//  The debugger has one and the build has one, and to a reader they are the
//  same object: a program said things, in order, and the last one matters most.
//  Two panels drawing that two ways would be two things to learn for no
//  information gained, so both go through here.
//
//  **A line's KIND comes from core**, and this file only decides what a kind
//  looks like. That is the whole fix behind this component: the debug console
//  used to print `[stdout] 16` — the DAP protocol's word for its own pipe —
//  which made what the program printed, the one line anybody pressed Run for,
//  look exactly like the four beside it announcing in four different ways that
//  it was over. A prefix can only be read. A kind can be coloured, and the eye
//  sorts colour before it reads anything.
//
//  The palette is the caller's, because "dim" is the panel's own dim and a
//  hard-coded grey beside a themed row reads as a rendering fault — the same
//  rule `FileSymbols` follows.

import AppKit
import SwiftUI

/// One line, and what it IS.
struct ConsoleLine: Identifiable, Equatable {
    /// Mirrors core's `dap::LogKind`. The raw values cross the ABI.
    enum Kind: UInt8 {
        /// The program's own stdout/stderr. The reason the run happened.
        case program = 0
        /// Suisei narrating: "⚙ cargo build…", "$ cargo test".
        case note = 1
        /// The adapter, or the tool, talking about itself.
        case adapter = 2
        case error = 3
        /// How it ended.
        case result = 4

        init(raw: UInt8) { self = Kind(rawValue: raw) ?? .note }
    }

    let id: Int
    let text: String
    let kind: Kind
}

struct ConsolePalette {
    var fg: Color
    var dim: Color
    var accent: Color
    var success: Color
    var warning: Color
    var danger: Color
}

struct ConsoleView: View {
    let lines: [ConsoleLine]
    /// The true length. Core keeps more than the snapshot carries, and a
    /// scrollback that starts where its window does implies a run began there.
    let total: Int
    let palette: ConsolePalette
    /// Shown when there is nothing yet — a console with no explanation reads
    /// as one that is broken.
    var placeholder: String = ""

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    if total > lines.count {
                        Text("… \(total - lines.count) earlier lines")
                            .font(.system(size: 9.5))
                            .foregroundStyle(palette.dim.opacity(0.8))
                            .padding(.vertical, 2)
                    }
                    ForEach(lines) { line in
                        row(line).id(line.id)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .overlay {
                if lines.isEmpty, !placeholder.isEmpty {
                    Text(placeholder)
                        .font(.system(size: 11))
                        .foregroundStyle(palette.dim)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 20)
                }
            }
            // A console is read from the bottom, and a line that arrives off
            // screen is a line nobody sees.
            .onChange(of: lines.count) { _, _ in
                guard let last = lines.last else { return }
                withAnimation(.snappy(duration: 0.15)) {
                    proxy.scrollTo(last.id, anchor: .bottom)
                }
            }
            .onAppear {
                guard let last = lines.last else { return }
                proxy.scrollTo(last.id, anchor: .bottom)
            }
        }
    }

    /// The result line is the answer, so it is the one thing here that is not
    /// monospaced body text: it gets a rule above it and its own weight, the
    /// way the last line of a receipt does.
    @ViewBuilder
    private func row(_ line: ConsoleLine) -> some View {
        if line.kind == .result {
            VStack(alignment: .leading, spacing: 3) {
                Rectangle()
                    .fill(palette.dim.opacity(0.25))
                    .frame(height: 1)
                    .padding(.top, 3)
                Text(line.text)
                    .font(.system(size: 11, weight: .semibold, design: .monospaced))
                    .foregroundStyle(resultInk(line.text))
                    .textSelection(.enabled)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            Text(line.text)
                .font(.system(size: 10.5, design: .monospaced))
                .foregroundStyle(ink(line.kind))
                .opacity(line.kind == .adapter ? 0.75 : 1)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func ink(_ kind: ConsoleLine.Kind) -> Color {
        switch kind {
        // Full strength, and the only kind that gets it: this is what the
        // program said.
        case .program: return palette.fg
        case .note: return palette.dim
        case .adapter: return palette.dim
        case .error: return palette.danger
        case .result: return palette.fg
        }
    }

    /// Green for a zero, red for anything else — read out of the line itself,
    /// because core writes the ending and the face should not have to be told
    /// twice what "ok" means.
    private func resultInk(_ text: String) -> Color {
        let t = text.lowercased()
        if t.contains("code 0") || t.contains("· done") || t.contains("succeeded") {
            return palette.success
        }
        if t.contains("stopped") || t.contains("terminated") {
            return palette.warning
        }
        return palette.danger
    }
}

/// The header a console sits under, in the one typographic language the model
/// workbench and the debug panel already share.
struct ConsoleHeader<Trailing: View>: View {
    let title: String
    let dim: Color
    let separator: Color
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(spacing: 6) {
            Text(title.uppercased())
                .font(.system(size: 9, weight: .semibold))
                .tracking(0.5)
                .foregroundStyle(dim)
            Spacer(minLength: 0)
            trailing()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.primary.opacity(0.035))
        .overlay(alignment: .bottom) {
            Rectangle().fill(separator).frame(height: 1)
        }
    }
}

/// Copy the whole console. A log you cannot get out of the window is a log you
/// end up screenshotting.
struct ConsoleCopyButton: View {
    let lines: [ConsoleLine]
    let dim: Color
    @State private var copied = false

    var body: some View {
        Button {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(
                lines.map(\.text).joined(separator: "\n"), forType: .string
            )
            copied = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) { copied = false }
        } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc")
                .font(.system(size: 9))
                .frame(width: 16, height: 14)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(dim)
        .help("Copy the console")
        .disabled(lines.isEmpty)
    }
}
