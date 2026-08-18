//  BuildPanel.swift
//  Build, Run, Test — and the places the compiler complained about.
//
//  feature.txt #9's other half. The debugger has always been able to build,
//  because it has to build in order to LAUNCH; what it could not do is run
//  something for the sake of its output, and when a build failed it wrote one
//  sentence into the console naming a file and a line nobody could go to.
//
//  So this panel is two lists and a transport, and the arrangement says which
//  is which: problems on the left because they are the answer, console on the
//  right because it is the evidence. A build that produced no problems shows
//  the console alone — an empty pane captioned "no problems" beside a wall of
//  output is a column of nothing where the important thing should be.
//
//  Nothing is drawn twice. The console is `ConsoleView`, the same one the
//  debugger uses, because to a reader they are one object.

import AppKit
import SwiftUI

struct BuildPanelView: View {
    @ObservedObject var engine: EngineBridge
    let accent: Color
    let fg: Color
    let dim: Color
    let separator: Color

    private var build: BuildSnap { engine.build }

    private var palette: ConsolePalette {
        ConsolePalette(
            fg: fg, dim: dim, accent: accent,
            success: engine.chrome.theme.successColor,
            warning: engine.chrome.theme.warningColor,
            danger: engine.chrome.theme.dangerColor
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            transport
            Divider().overlay(separator)
            if build.problems.isEmpty {
                console
            } else {
                HStack(spacing: 0) {
                    problemList
                        .frame(width: 340)
                    Divider().overlay(separator)
                    console
                }
            }
        }
        // No `.task` here telling core the panel is open. This view only
        // exists while it IS open, so an observer on it could say "showing"
        // and could never say the opposite — the exact bug `pushDapPanel`
        // documents. The bridge owns both inputs and does the telling.
    }

    // MARK: - Transport

    private var transport: some View {
        HStack(spacing: 6) {
            ForEach(BuildSnap.Kind.allCases, id: \.self) { kind in
                Button {
                    engine.buildRun(kind)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: kind.symbol)
                            .font(.system(size: 10, weight: .medium))
                        Text(kind.title)
                            .font(.system(size: 11, weight: .medium))
                    }
                    .foregroundStyle(build.isRunning ? dim.opacity(0.6) : fg)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .fill(Color.primary.opacity(0.06))
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(build.isRunning)
                .help("\(kind.title) · \(shortcut(kind))")
            }

            // Always enabled, like the debugger's Stop and for the same
            // reason: it is the way OUT, and the moment it is most needed is
            // the one the panel is least sure about.
            Button {
                engine.buildStop()
            } label: {
                Image(systemName: "stop.fill")
                    .font(.system(size: 10, weight: .medium))
                    .frame(width: 22, height: 20)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(build.isRunning ? fg : dim.opacity(0.5))
            .help("Stop")

            statusChip

            Spacer(minLength: 6)

            if build.dropped > 0 {
                Text("+\(build.dropped) more")
                    .font(.system(size: 9.5))
                    .foregroundStyle(dim)
                    .help("Found past the cap and not kept")
            }
            ConsoleCopyButton(lines: build.console, dim: dim)
        }
        .padding(.horizontal, 10)
        .frame(height: 30)
    }

    private func shortcut(_ kind: BuildSnap.Kind) -> String {
        switch kind {
        case .build: return "⌘B"
        case .run: return "⌃⌘R"
        case .test: return "⌘U"
        }
    }

    /// One dot and one sentence. The counts are in the sentence rather than in
    /// two badges, because "2 errors" beside "0 warnings" makes the reader
    /// compare two numbers to learn one thing.
    private var statusChip: some View {
        HStack(spacing: 5) {
            if build.isRunning {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.6)
                    .frame(width: 10, height: 10)
            } else {
                Circle()
                    .fill(stateInk)
                    .frame(width: 6, height: 6)
            }
            Text(statusText)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(build.hasRun ? fg : dim)
                .lineLimit(1)
                .truncationMode(.middle)
            if !build.took.isEmpty, !build.isRunning {
                Text(build.took)
                    .font(.system(size: 10))
                    .foregroundStyle(dim)
            }
        }
        .padding(.leading, 4)
    }

    private var statusText: String {
        if !build.hasRun { return "Nothing has run yet" }
        return build.summary.isEmpty ? build.label : build.summary
    }

    private var stateInk: Color {
        switch build.state {
        case .idle: return dim
        case .running: return engine.chrome.theme.warningColor
        case .ok: return engine.chrome.theme.successColor
        case .failed: return engine.chrome.theme.dangerColor
        }
    }

    // MARK: - Problems

    private var problemList: some View {
        VStack(alignment: .leading, spacing: 0) {
            ConsoleHeader(title: "Problems", dim: dim, separator: separator) {
                if build.problemTotal > build.problems.count {
                    Text("\(build.problems.count) of \(build.problemTotal)")
                        .font(.system(size: 9.5))
                        .foregroundStyle(dim.opacity(0.8))
                }
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(build.problems) { problem in
                        problemRow(problem)
                    }
                }
                .padding(.bottom, 6)
            }
        }
    }

    private func problemRow(_ problem: BuildProblem) -> some View {
        Button {
            engine.buildGoto(problem.id)
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                // Shape before colour: the two severities differ by glyph as
                // well as by hue, so Increase Contrast and a colour-blind
                // reader both still see which is which.
                Image(systemName: problem.isError
                      ? "exclamationmark.octagon.fill"
                      : "exclamationmark.triangle.fill")
                    .font(.system(size: 9))
                    .foregroundStyle(problem.isError
                                     ? engine.chrome.theme.dangerColor
                                     : engine.chrome.theme.warningColor)
                    .frame(width: 12)
                VStack(alignment: .leading, spacing: 1) {
                    Text(problem.message)
                        .font(.system(size: 11))
                        .foregroundStyle(fg)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                    if problem.locatable {
                        Text(problem.place)
                            .font(.system(size: 9.5, design: .monospaced))
                            .foregroundStyle(dim)
                            .lineLimit(1)
                            .truncationMode(.head)
                    } else {
                        // Said, rather than left as a row that does nothing
                        // when clicked. A link failure names no place, and
                        // silence there reads as a broken list.
                        Text("no location")
                            .font(.system(size: 9.5))
                            .foregroundStyle(dim.opacity(0.7))
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!problem.locatable)
        .help(problem.locatable ? "Go to \(problem.place)" : problem.message)
    }

    // MARK: - Console

    private var console: some View {
        VStack(alignment: .leading, spacing: 0) {
            ConsoleHeader(title: "Output", dim: dim, separator: separator) {
                if !build.label.isEmpty {
                    Text(build.label)
                        .font(.system(size: 9.5, design: .monospaced))
                        .foregroundStyle(dim.opacity(0.8))
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            ConsoleView(
                lines: build.console,
                total: build.consoleTotal,
                palette: palette,
                placeholder: "Press Build, Run or Test. What the command "
                    + "prints lands here, and anything it says about a file "
                    + "becomes a problem you can click."
            )
        }
    }
}
