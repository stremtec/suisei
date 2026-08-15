//  QuickHelpPopover.swift
//  What a symbol is, where you asked about it.
//
//  The description was already being fetched and already had somewhere to go:
//  `EngineBridge.refreshHover` asks the language server, and the inspector has
//  a Quick Help tab that prints the answer. But the only thing that ever called
//  `refreshHover` was switching TO that tab, which is why the tab's own empty
//  state read "Put the caret on a symbol, then reopen this tab" — a feature
//  explaining its own wiring to the user.
//
//  So this is not a new source of information. It is the same answer, asked for
//  by right-clicking the thing you want to know about and shown next to it,
//  which is where a question about a symbol is asked.

import AppKit
import SwiftUI

/// The card inside the popover.
///
/// Bound to the bridge rather than handed a string, because hover is a round
/// trip to another process: the popover has to appear on the click and fill in
/// when the server answers. A view that took the text as a parameter could only
/// be shown after the wait, and a third of a second of nothing happening after
/// a menu click reads as the menu item being broken.
struct QuickHelpCard: View {
    @ObservedObject var engine: EngineBridge
    /// The identifier that was right-clicked. Empty when the click was not on
    /// one — the card still opens, because "there is nothing here to describe"
    /// is an answer.
    let symbol: String

    /// How tall the rendered answer actually is. See `body(for:)`.
    @State private var measured: CGFloat = 0

    /// Past this the card scrolls. A keyword guide runs to a couple of
    /// thousand characters, and a popover that grows to hold all of it is a
    /// window wearing a tail.
    private static let maxBodyHeight: CGFloat = 460

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: "info.circle.fill")
                    .font(.system(size: 11))
                    .foregroundStyle(.tint)
                Text(symbol.isEmpty ? "Quick Help" : symbol)
                    .font(.system(size: 12, weight: .semibold, design: symbol.isEmpty ? .default : .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .padding(.top, 10)
            .padding(.bottom, 8)

            Divider()

            body(for: engine.hoverText)
        }
        // Wide enough for a worked example. A language server's answer for a
        // keyword carries code, and code that wraps every third token has
        // stopped being a sample.
        .frame(width: 460)
    }

    @ViewBuilder
    private func body(for text: String) -> some View {
        if !text.isEmpty {
            ScrollView {
                QuickHelpBody(markdown: text)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                    .background(
                        GeometryReader { geo in
                            Color.clear.preference(
                                key: QuickHelpHeightKey.self, value: geo.size.height
                            )
                        }
                    )
            }
            .onPreferenceChange(QuickHelpHeightKey.self) { measured = $0 }
            // Measured, not flexible. A `ScrollView` has no ideal height, and
            // an NSPopover sizes itself from what its content says it wants —
            // so the card collapsed to about one paragraph and the rest of the
            // answer was scrollable but invisible. It grows to the answer and
            // stops at a few hundred points, which is where a card stops being
            // a card.
            .frame(height: min(max(measured, 44), Self.maxBodyHeight))
        } else if engine.hoverPending {
            // A spinner rather than an empty box: the wait is a round trip to
            // another process and the user has no other way to know that.
            HStack(spacing: 7) {
                ProgressView().controlSize(.small)
                Text("Looking up…")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 12)
        } else {
            VStack(alignment: .leading, spacing: 3) {
                Text(emptyHeadline)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                Text(emptyDetail)
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
        }
    }

    private var emptyHeadline: String {
        if symbol.isEmpty { return "Nothing to describe here." }
        return "No description."
    }

    /// Why there is nothing, as precisely as the app can say it.
    ///
    /// This used to be one sentence for every case — "Descriptions come from
    /// the language server for this file" — which told a user who plainly has
    /// rust-analyzer running something they already knew, and blamed a
    /// component that was working. Core has always known whether a server is
    /// attached and which one; it simply never crossed the ABI, so the card
    /// could only shrug in one way.
    ///
    /// A server that is attached and answers nothing is the interesting case,
    /// and it is usually not about the symbol: rust-analyzer answers nothing
    /// for every position in a file that is not in its crate graph — a scratch
    /// file beside Cargo.toml rather than under a member's `src/`. Naming the
    /// server is what turns "no idea" into somewhere to look.
    private var emptyDetail: String {
        if symbol.isEmpty { return "Right-click a name to ask about it." }
        let server = engine.lspServerName
        if server.isEmpty {
            return "No language server is attached to this file."
        }
        return "\(server) had nothing for this position — it may not have this file in its project."
    }
}

/// A language server's answer, rendered.
///
/// It arrives as **markdown**, and it was being printed with a plain `Text`.
/// For a symbol that is one line of signature nobody noticed; for a keyword it
/// is a guide, and a guide shown as raw markup is not a guide — the user saw
/// literal ```` ```rust ```` fences, a bare `---`, and `[impl](https://…)`
/// where a link should be. rust-analyzer's answer for `fn` is 1,834 characters
/// of exactly that: what a function is, where one may be written, and four
/// worked examples.
///
/// Three kinds of block, because three is what a hover answer contains.
/// Anything cleverer would be a markdown engine, and the one thing this has to
/// get right is that code looks like code.
struct QuickHelpBody: View {
    let markdown: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(Self.blocks(in: markdown).enumerated()), id: \.offset) { _, block in
                switch block {
                case .rule:
                    Divider()
                case .prose(let text):
                    Text(text)
                        .font(.system(size: 11))
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                case .code(let source):
                    // Scrolls rather than wraps: a wrapped line of code reads
                    // as a different program.
                    ScrollView(.horizontal, showsIndicators: false) {
                        Text(source)
                            .font(.system(size: 10.5, design: .monospaced))
                            .textSelection(.enabled)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                    }
                    .background(
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .fill(Color.primary.opacity(0.055))
                    )
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    enum Block {
        case prose(AttributedString)
        case code(String)
        case rule
    }

    /// Split an LSP hover answer into blocks.
    ///
    /// Fences first, because everything inside one is literal — a `---` or a
    /// `*` in a code sample is code, not a rule and not emphasis. Prose runs
    /// go through `AttributedString(markdown:)`, which resolves the inline
    /// spelling (links, `code`, **bold**) that the raw text was showing.
    /// `.inlineOnlyPreservingWhitespace` because paragraph parsing would
    /// collapse the line breaks the server put there on purpose.
    static func blocks(in markdown: String) -> [Block] {
        var out: [Block] = []
        var prose: [Substring] = []
        var code: [Substring] = []
        var inFence = false

        func flushProse() {
            let text = prose.joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            prose.removeAll()
            guard !text.isEmpty else { return }
            let parsed = try? AttributedString(
                markdown: text,
                options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)
            )
            out.append(.prose(parsed ?? AttributedString(text)))
        }

        for line in markdown.split(separator: "\n", omittingEmptySubsequences: false) {
            if line.trimmingCharacters(in: .whitespaces).hasPrefix("```") {
                if inFence {
                    let source = code.joined(separator: "\n")
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                    if !source.isEmpty { out.append(.code(source)) }
                    code.removeAll()
                } else {
                    flushProse()
                }
                inFence.toggle()
                continue
            }
            if inFence {
                code.append(line)
                continue
            }
            let bare = line.trimmingCharacters(in: .whitespaces)
            if bare == "---" || bare == "***" || bare == "___" {
                flushProse()
                out.append(.rule)
                continue
            }
            prose.append(line)
        }
        // An unterminated fence is still code — the cap that bounds hover text
        // can land in the middle of one.
        if inFence, !code.isEmpty {
            out.append(.code(code.joined(separator: "\n")))
        }
        flushProse()
        return out
    }
}

/// The rendered answer's height, so the card can be as tall as its content.
private struct QuickHelpHeightKey: PreferenceKey {
    static var defaultValue: CGFloat { 0 }
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}
