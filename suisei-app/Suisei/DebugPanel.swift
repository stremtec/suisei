//  DebugPanel.swift
//  The debugger, where a debugger goes.
//
//  feature.txt #9. Core has had a complete DAP client since before this face
//  existed — 2,995 lines of launch, attach, step, evaluate, conditional
//  breakpoints and `.vscode/launch.json` — and no way to reach any of it from
//  the GUI. This is the panel, and everything in it reads one snapshot
//  (`DapSnap`) and sends one command.
//
//  Xcode's arrangement, because it is the one a debugger has: transport across
//  the top, call stack and variables side by side, console underneath. The
//  three answer different questions — where am I, what is in scope, what did
//  it say — and putting them in one list would make each of them worse.

import AppKit
import SwiftUI

struct DebugPanelView: View {
    @ObservedObject var engine: EngineBridge
    let accent: Color
    let fg: Color
    let dim: Color
    let separator: Color

    @State private var expression = ""
    @FocusState private var expressionFocused: Bool

    private var dap: DapSnap { engine.dap }

    var body: some View {
        VStack(spacing: 0) {
            transport
            Divider().overlay(separator)
            if dap.session {
                HSplit
            } else {
                idle
            }
        }
    }

    // MARK: - Transport

    private var transport: some View {
        HStack(spacing: 6) {
            // Start and continue are the same button because they are the same
            // question — "go" — and core answers it with one call that starts a
            // session or resumes the one that exists.
            transportButton(
                dap.state == .stopped || !dap.session ? "play.fill" : "play.fill",
                help: dap.session ? "Continue · F5" : "Start Debugging · F5",
                enabled: dap.state != .running
            ) { engine.dapCommand(.startOrContinue) }

            transportButton("pause.fill", help: "Pause", enabled: dap.state == .running) {
                engine.dapCommand(.pause)
            }

            Divider().frame(height: 14).overlay(separator)

            transportButton("arrow.turn.down.right", help: "Step Over · F10", enabled: stepped) {
                engine.dapCommand(.stepOver)
            }
            transportButton("arrow.down.to.line", help: "Step Into · F11", enabled: stepped) {
                engine.dapCommand(.stepInto)
            }
            transportButton("arrow.up.from.line", help: "Step Out · ⇧F11", enabled: stepped) {
                engine.dapCommand(.stepOut)
            }

            Divider().frame(height: 14).overlay(separator)

            transportButton("arrow.clockwise", help: "Restart", enabled: dap.session) {
                engine.dapCommand(.restart)
            }
            // Always enabled, unlike every other control here. Stop is the way
            // OUT, and the moments it is most needed are exactly the ones the
            // panel is confused about: a build in flight, or an error left
            // over from a launch that failed. Greying it out in those states
            // left no way to clear them — the reported "꼬였달까".
            transportButton("stop.fill", help: "Stop · ⇧F5", enabled: true) {
                engine.dapCommand(.stop)
            }

            statusChip

            Spacer(minLength: 6)

            if !dap.adapter.isEmpty {
                Text(dap.adapter)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(dim)
            }
            configurationsMenu
        }
        .padding(.horizontal, 10)
        .frame(height: 30)
    }

    /// Stepping needs a stopped session. Greyed rather than hidden: a control
    /// that comes and goes is one the user has to hunt for, and a debugger's
    /// transport is a fixed row in every tool that has one.
    private var stepped: Bool { dap.state == .stopped }

    private func transportButton(
        _ symbol: String, help: String, enabled: Bool, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .medium))
                .frame(width: 22, height: 20)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .foregroundStyle(enabled ? fg : dim.opacity(0.5))
        .help(help)
    }

    private var statusChip: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(stateInk)
                .frame(width: 6, height: 6)
            Text(dap.state.label)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(fg)
            if !dap.status.isEmpty {
                Text(dap.status)
                    .font(.system(size: 10))
                    .foregroundStyle(dim)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
        }
        .padding(.leading, 4)
    }

    private var stateInk: Color {
        switch dap.state {
        case .idle: return dim
        case .starting, .ending: return engine.chrome.theme.warningColor
        case .running: return engine.chrome.theme.successColor
        case .stopped: return accent
        }
    }

    /// `.vscode/launch.json`, which core has always been able to read.
    ///
    /// Absent rather than empty when there are none: a menu with nothing in it
    /// says the feature is broken, and "this project has no launch
    /// configurations" is better said by the idle screen, which has room for a
    /// sentence.
    @ViewBuilder
    private var configurationsMenu: some View {
        let configs = engine.dapConfigurations()
        if !configs.isEmpty {
            Menu {
                ForEach(configs, id: \.self) { name in
                    Button(name) { engine.dapLaunch(name) }
                }
            } label: {
                Image(systemName: "list.bullet.rectangle")
                    .font(.system(size: 11))
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .frame(width: 22)
            .help("Launch Configurations")
        }
    }

    // MARK: - Idle

    private var idle: some View {
        VStack(spacing: 7) {
            Image(systemName: "ladybug")
                .font(.system(size: 22))
                .foregroundStyle(dim)
            Text("No debug session")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(fg)
            Text(idleHint)
                .font(.system(size: 11))
                .foregroundStyle(dim)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 380)
            if !dap.console.isEmpty {
                // What the last session said, which is where the reason it
                // ended is written.
                consoleList.frame(maxHeight: 120)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(.vertical, 12)
    }

    private var idleHint: String {
        if !dap.status.isEmpty { return dap.status }
        let configs = engine.dapConfigurations()
        if configs.isEmpty {
            return "Set a breakpoint and press Start, or add a "
                + ".vscode/launch.json to choose a configuration."
        }
        return "Press Start, or pick one of \(configs.count) launch "
            + "configuration\(configs.count == 1 ? "" : "s")."
    }

    // MARK: - Live session

    /// Stack and variables side by side, console underneath.
    ///
    /// Named for the shape rather than for what is in it: the arrangement is
    /// the point, and it is the one Xcode, VS Code and every debugger before
    /// them settled on because the three panes answer different questions.
    private var HSplit: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                column("Call Stack", width: nil) { stackList }
                Divider().overlay(separator)
                column("Variables", width: nil) { variablesList }
            }
            .frame(maxHeight: .infinity)
            Divider().overlay(separator)
            consoleSection
        }
    }

    private func column<Content: View>(
        _ title: String, width: CGFloat?, @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(dim)
                .padding(.horizontal, 10)
                .padding(.top, 6)
                .padding(.bottom, 3)
            content()
        }
        .frame(maxWidth: width ?? .infinity, alignment: .leading)
    }

    private var stackList: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(dap.frames) { frame in
                    let selected = frame.id == dap.selectedFrame
                    Button {
                        engine.dapSelectFrame(frame.id)
                    } label: {
                        HStack(spacing: 6) {
                            Image(systemName: selected ? "arrowtriangle.right.fill" : "circle.fill")
                                .font(.system(size: selected ? 8 : 4))
                                .foregroundStyle(selected ? accent : dim.opacity(0.5))
                                .frame(width: 10)
                            VStack(alignment: .leading, spacing: 0) {
                                Text(frame.name)
                                    .font(.system(size: 11))
                                    .foregroundStyle(fg)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                                if !frame.path.isEmpty {
                                    // The line is 1-based where a human reads
                                    // it; core stores it 0-based.
                                    Text("\((frame.path as NSString).lastPathComponent):\(frame.line + 1)")
                                        .font(.system(size: 9.5, design: .monospaced))
                                        .foregroundStyle(dim)
                                        .lineLimit(1)
                                        .truncationMode(.head)
                                }
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 10)
                        .padding(.vertical, 3)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(selected ? accent.opacity(0.12) : .clear)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.bottom, 6)
        }
    }

    private var variablesList: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(dap.variables) { node in
                    Button {
                        guard node.expandable else { return }
                        engine.dapToggleVariable(node.id)
                    } label: {
                        HStack(spacing: 5) {
                            // The indent IS the tree. A disclosure arrow with
                            // no indent, or an indent with no arrow, each say
                            // half of what nesting means.
                            Color.clear.frame(width: CGFloat(node.depth) * 11, height: 1)
                            Image(systemName: node.expanded ? "chevron.down" : "chevron.right")
                                .font(.system(size: 8, weight: .semibold))
                                .foregroundStyle(dim)
                                .opacity(node.expandable ? 1 : 0)
                                .frame(width: 9)
                            Text(node.name)
                                .font(.system(size: 11, weight: node.isScope ? .semibold : .regular))
                                .foregroundStyle(node.isScope ? fg : accent)
                                .lineLimit(1)
                            if !node.value.isEmpty {
                                Text(node.value)
                                    .font(.system(size: 10.5, design: .monospaced))
                                    .foregroundStyle(fg)
                                    .lineLimit(1)
                                    .truncationMode(.tail)
                            }
                            if !node.type.isEmpty, !node.isScope {
                                Text(node.type)
                                    .font(.system(size: 9.5))
                                    .foregroundStyle(dim)
                                    .lineLimit(1)
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 10)
                        .padding(.vertical, 2.5)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.bottom, 6)
        }
    }

    // MARK: - Console

    private var consoleSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text("Console")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(dim)
                if dap.consoleTotal > dap.console.count {
                    // Say what is missing rather than let the scrollback imply
                    // the session started where it does.
                    Text("last \(dap.console.count) of \(dap.consoleTotal)")
                        .font(.system(size: 9.5))
                        .foregroundStyle(dim.opacity(0.8))
                }
                Spacer()
            }
            .padding(.horizontal, 10)
            .padding(.top, 6)
            .padding(.bottom, 3)

            consoleList.frame(maxHeight: 160)

            HStack(spacing: 6) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(dim)
                TextField("Evaluate in the selected frame", text: $expression)
                    .textFieldStyle(.plain)
                    .font(.system(size: 11, design: .monospaced))
                    .focused($expressionFocused)
                    .onSubmit {
                        engine.dapEvaluate(expression)
                        expression = ""
                    }
                    // Disabled while running: an expression is evaluated in a
                    // FRAME, and a running program does not have one stopped
                    // for us to ask about.
                    .disabled(dap.state != .stopped)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Color.primary.opacity(0.04))
        }
    }

    private var consoleList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    ForEach(Array(dap.console.enumerated()), id: \.offset) { i, line in
                        Text(line)
                            .font(.system(size: 10.5, design: .monospaced))
                            .foregroundStyle(fg.opacity(0.9))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .id(i)
                    }
                }
                .padding(.horizontal, 10)
            }
            // A console is read from the bottom, and a new line arriving off
            // screen is a line nobody sees.
            .onChange(of: dap.console.count) { _, n in
                guard n > 0 else { return }
                withAnimation(.snappy(duration: 0.15)) { proxy.scrollTo(n - 1, anchor: .bottom) }
            }
        }
    }
}
