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
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "info.circle.fill")
                    .font(.system(size: 11))
                    .foregroundStyle(.tint)
                Text(symbol.isEmpty ? "Quick Help" : symbol)
                    .font(QuickHelpFonts.title)
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .padding(.top, 10)
            .padding(.bottom, 9)

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
                QuickHelpBody(markdown: text, titled: symbol)
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
    /// The name already shown as the card's title, if there is one.
    ///
    /// A hover answer opens with a fenced block holding the declaration, and
    /// for a keyword that declaration is the keyword: `pub` appeared as the
    /// title and again immediately below it in a code box. Passed in so the
    /// leading block can be dropped when it says nothing the title has not —
    /// and kept when it is a real signature, which is the part of a function's
    /// answer worth reading first.
    var titled: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(visibleBlocks.enumerated()), id: \.offset) { _, block in
                switch block {
                case .rule:
                    Divider()
                case .prose(let text):
                    Text(text)
                        .font(QuickHelpFonts.body)
                        // Paragraphs of running text need air between lines;
                        // 11pt on default leading is a wall.
                        .lineSpacing(2.5)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                case .code(let source):
                    // Scrolls rather than wraps: a wrapped line of code reads
                    // as a different program.
                    ScrollView(.horizontal, showsIndicators: false) {
                        Text(source)
                            .font(QuickHelpFonts.code)
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

    /// The blocks minus the leading one the title already says.
    ///
    /// Only the FIRST block, and only when it is code that reads as the title:
    /// a later fence with the same text would be a worked example, and an
    /// example that happens to be one word long is still an example. The rule
    /// that followed it goes too — a divider under nothing is a line across an
    /// empty card.
    private var visibleBlocks: [Block] {
        var blocks = Self.blocks(in: markdown)
        let name = titled.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, case .code(let first)? = blocks.first,
              first.trimmingCharacters(in: .whitespacesAndNewlines) == name
        else { return blocks }
        blocks.removeFirst()
        if case .rule? = blocks.first { blocks.removeFirst() }
        return blocks
    }

    /// Set the `code` spans inside a sentence in the editor's own face.
    ///
    /// Markdown parsing marks them but chooses no font, so a `fn` written as
    /// code in the middle of a paragraph arrived indistinguishable from the
    /// words around it — the backticks had been resolved away and nothing took
    /// their place. Ranges are collected before the edit: mutating an
    /// `AttributedString` invalidates the run iteration that produced them.
    private static func codeSpansInMono(_ input: AttributedString) -> AttributedString {
        var out = input
        let spans = out.runs.compactMap { run -> Range<AttributedString.Index>? in
            run.inlinePresentationIntent?.contains(.code) == true ? run.range : nil
        }
        for span in spans {
            out[span].font = QuickHelpFonts.inlineCode
        }
        return out
    }

    /// A line that has to start a line of its own: a list item, a heading, a
    /// quote. Everything else in a paragraph is a soft wrap. See `unwrap`.
    private static func startsItsOwnLine(_ line: Substring) -> Bool {
        let t = line.drop { $0 == " " }
        if t.hasPrefix("- ") || t.hasPrefix("* ") || t.hasPrefix("+ ") { return true }
        if t.hasPrefix("#") || t.hasPrefix(">") || t.hasPrefix("|") { return true }
        // `1. ` and friends.
        let digits = t.prefix { $0.isNumber }
        return !digits.isEmpty && t.dropFirst(digits.count).hasPrefix(". ")
    }

    /// Undo the server's hard wrap.
    ///
    /// A language server writes its markdown wrapped for a terminal —
    /// rust-analyzer's is broken at about 95 columns — and markdown says a
    /// single newline inside a paragraph is a SOFT break. Preserving them, as
    /// this did, made the text wrap twice: once at the server's column and
    /// again at the card's width, which is where "Functions are the primary
    /// way code is executed within Rust. / Function blocks, usually just /
    /// called functions, can be defined…" comes from. The ragged short lines
    /// are the whole of why it read badly.
    ///
    /// Blank lines still separate paragraphs, and a list item or heading keeps
    /// its own line — those newlines are the author's, not the wrapper's.
    private static func unwrap(_ paragraph: String) -> String {
        var out: [String] = []
        for line in paragraph.split(separator: "\n", omittingEmptySubsequences: false) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.isEmpty { continue }
            if out.isEmpty || startsItsOwnLine(line) {
                out.append(trimmed)
            } else {
                out[out.count - 1] += " " + trimmed
            }
        }
        return out.joined(separator: "\n")
    }

    /// Split an LSP hover answer into blocks.
    ///
    /// Fences first, because everything inside one is literal — a `---` or a
    /// `*` in a code sample is code, not a rule and not emphasis. Prose runs
    /// go through `AttributedString(markdown:)`, which resolves the inline
    /// spelling (links, `code`, **bold**) that the raw text was showing.
    ///
    /// One `.prose` per PARAGRAPH rather than per run, so the space between
    /// paragraphs is the layout's to set and not a blank line inside a string.
    static func blocks(in markdown: String) -> [Block] {
        var out: [Block] = []
        var prose: [Substring] = []
        var code: [Substring] = []
        var inFence = false

        func flushProse() {
            let text = prose.joined(separator: "\n")
            prose.removeAll()
            for paragraph in text.components(separatedBy: "\n\n") {
                let body = unwrap(paragraph)
                guard !body.isEmpty else { continue }
                let parsed = try? AttributedString(
                    markdown: body,
                    options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)
                )
                out.append(.prose(codeSpansInMono(parsed ?? AttributedString(body))))
            }
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

/// The card's faces.
///
/// JetBrains Mono for everything that IS code — the signature, the samples,
/// and the inline spans inside a sentence — because those have to line up and
/// because a `fn` in the middle of prose should look like the `fn` in the
/// editor behind the card.
///
/// Running prose is the system text face. It was JetBrains Mono too, and that
/// is half of why the explanation read badly: a monospace face gives every
/// letter the same width, which is what makes columns line up and what takes
/// word shapes away from a reader. It is the right tool for four lines of
/// sample and the wrong one for four paragraphs about them. Xcode's Quick
/// Help, and every documentation viewer, splits it the same way.
///
/// Milker for the name, which is the one thing on the card that is a heading
/// rather than text.
///
/// `WelcomeView` says Milker "carries A–Z/a–z only", which would have made a
/// title a patchwork the moment a name had an underscore in it. Checked
/// against the font's own character set rather than taken on trust: letters,
/// digits, `_`, and the punctuation a signature uses are all in it. So
/// `looks_binary` and `godddddd` set in one face, not two.
///
/// Resolved once and cached — `NSFont(name:)` is a lookup, and this is read
/// for every block of every card.
enum QuickHelpFonts {
    static let title: Font = {
        WelcomeFonts.registerIfNeeded()
        // PostScript name from the OTF, family name as the fallback — the same
        // pair the Welcome wordmark asks for.
        if let face = NSFont(name: "MilkerRegular", size: 22)
            ?? NSFont(name: "Milker", size: 22)
        {
            return Font(face)
        }
        return .system(size: 19, weight: .semibold, design: .rounded)
    }()

    /// Running text. Proportional — see the type comment.
    static let body = Font.system(size: 12)
    /// A `code` span inside a sentence, sized to sit on the prose's baseline
    /// without standing off it: JetBrains Mono runs large for its point size.
    static let inlineCode = Font(EditorMetrics.monospaced(11, weight: .regular))
    /// Samples, a shade smaller so a wide line fits before it has to scroll.
    static let code = Font(EditorMetrics.monospaced(10.5, weight: .regular))
}
