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
        if engine.isReservedWord(symbol) { return "“\(symbol)” is a keyword." }
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
        // A KEYWORD is the third case, and it was being reported as the second.
        //
        // Measured against pyright on the file this was reported from: hover
        // `argparse` and it returns a full module doc; hover the `import`
        // beside it and it returns null. Both are correct — a language server
        // describes names, not syntax — but the card said the server "had
        // nothing for this position — it may not have this file in its
        // project", which blames a project that is fine and a server that is
        // working, for a question that has no answer. "이거 왜 안뜸…? 임포트는
        // 떠야지 적어도."
        if engine.isReservedWord(symbol) {
            return "\(server) describes names, not syntax. Try the name beside it."
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
/// Blocks, not a markdown engine: headings, paragraphs, list items, fenced
/// code and rules are what a hover answer is made of, and the rest of the
/// spelling (links, `code`, **bold**) is inline and belongs to
/// `AttributedString(markdown:)`.
struct QuickHelpBody: View {
    let markdown: String
    /// The name the card is about.
    ///
    /// Two jobs. The leading fenced block is dropped when it says nothing this
    /// has not — a hover answer opens with the declaration, and for a keyword
    /// the declaration IS the keyword, so `pub` was the title and then `pub`
    /// again in a box under it. And every other mention of it, in prose or in
    /// a sample, is marked: the answer is about this word, and in four
    /// paragraphs of prose about functions the reader should be able to find
    /// `fn` without reading for it.
    var titled: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            ForEach(Array(visibleBlocks.enumerated()), id: \.offset) { _, block in
                switch block {
                case .rule:
                    Divider()

                case .heading(let level, let text):
                    Text(text)
                        .font(QuickHelpFonts.heading(level))
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.top, 2)

                case .prose(let text):
                    Text(text)
                        .font(QuickHelpFonts.body)
                        // Running text needs air between lines; a monospaced
                        // face on default leading is a wall.
                        .lineSpacing(2.5)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)

                case .bullet(let text):
                    // A hanging indent, so a wrapped item stays inside its own
                    // bullet rather than starting again at the margin.
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text("•").font(QuickHelpFonts.body).foregroundStyle(.secondary)
                        Text(text)
                            .font(QuickHelpFonts.body)
                            .lineSpacing(2.5)
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(.leading, 2)

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
        case heading(level: Int, AttributedString)
        case prose(AttributedString)
        case bullet(AttributedString)
        case code(AttributedString)
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
        var blocks = Self.blocks(in: markdown, naming: titled)
        let name = titled.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, case .code(let first)? = blocks.first,
              String(first.characters).trimmingCharacters(in: .whitespacesAndNewlines) == name
        else { return blocks }
        blocks.removeFirst()
        if case .rule? = blocks.first { blocks.removeFirst() }
        return blocks
    }

    // MARK: - Parsing

    /// `#`…`######` and the text after it, or nil.
    private static func headingParts(_ line: String) -> (Int, String)? {
        let hashes = line.prefix { $0 == "#" }
        guard (1...6).contains(hashes.count) else { return nil }
        let rest = line.dropFirst(hashes.count)
        guard rest.first == " " else { return nil }
        return (hashes.count, rest.trimmingCharacters(in: .whitespaces))
    }

    /// The text of a list item, or nil. Ordered and unordered both: what the
    /// marker was does not survive into a bulleted row.
    private static func bulletBody(_ line: String) -> String? {
        for marker in ["- ", "* ", "+ "] where line.hasPrefix(marker) {
            return String(line.dropFirst(marker.count))
        }
        let digits = line.prefix { $0.isNumber }
        if !digits.isEmpty, line.dropFirst(digits.count).hasPrefix(". ") {
            return String(line.dropFirst(digits.count + 2))
        }
        return nil
    }

    /// Split an LSP hover answer into blocks.
    ///
    /// Fences first, because everything inside one is literal — a `#` or a `*`
    /// in a code sample is code, not a heading and not emphasis.
    ///
    /// Lines inside a paragraph are JOINED. A language server writes its
    /// markdown wrapped for a terminal (rust-analyzer's is broken at about 95
    /// columns) and markdown says a single newline inside a paragraph is a
    /// soft break; keeping them made the text wrap twice, once at the server's
    /// column and again at the card's width. A heading or a list item starts
    /// its own line because that newline is the author's, not the wrapper's.
    static func blocks(in markdown: String, naming symbol: String = "") -> [Block] {
        var out: [Block] = []
        var prose: [String] = []
        var code: [String] = []
        var inFence = false

        func inline(_ text: String) -> AttributedString {
            let parsed = (try? AttributedString(
                markdown: text,
                options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)
            )) ?? AttributedString(text)
            return marked(symbol, in: codeSpansInMono(parsed))
        }

        func flushProse() {
            let lines = prose
            prose.removeAll()
            var paragraph: [String] = []
            func endParagraph() {
                let body = paragraph.joined(separator: " ")
                paragraph.removeAll()
                if !body.isEmpty { out.append(.prose(inline(body))) }
            }
            var i = 0
            while i < lines.count {
                let line = lines[i].trimmingCharacters(in: .whitespaces)
                if line.isEmpty {
                    endParagraph()
                    i += 1
                } else if let (level, text) = headingParts(line) {
                    endParagraph()
                    out.append(.heading(level: level, inline(text)))
                    i += 1
                } else if let item = bulletBody(line) {
                    endParagraph()
                    // A wrapped item belongs to the item, not to the next one.
                    var body = item
                    var j = i + 1
                    while j < lines.count {
                        let next = lines[j].trimmingCharacters(in: .whitespaces)
                        if next.isEmpty || headingParts(next) != nil || bulletBody(next) != nil {
                            break
                        }
                        body += " " + next
                        j += 1
                    }
                    out.append(.bullet(inline(body)))
                    i = j
                } else {
                    paragraph.append(line)
                    i += 1
                }
            }
            endParagraph()
        }

        for raw in markdown.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = String(raw)
            if line.trimmingCharacters(in: .whitespaces).hasPrefix("```") {
                if inFence {
                    let source = code.joined(separator: "\n")
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                    if !source.isEmpty {
                        out.append(.code(marked(symbol, in: AttributedString(source))))
                    }
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
            out.append(.code(marked(symbol, in: AttributedString(code.joined(separator: "\n")))))
        }
        flushProse()
        return out
    }

    // MARK: - Inline styling

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

    /// Mark every mention of the word the card is about.
    ///
    /// The editor's bracket-match yellow, which is already what Suisei means by
    /// "this one, here" — the same fill and the same black ink as the flash on
    /// a matching brace and the chip behind a breakpoint. One yellow in both
    /// themes, for the reason `bracketYellow` gives.
    ///
    /// Whole words only. Marking every `fn` inside `fn_name` would light up the
    /// sample rather than point into it, and the point is that the eye lands on
    /// the thing being explained.
    private static func marked(_ symbol: String, in input: AttributedString) -> AttributedString {
        let name = symbol.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return input }
        var out = input
        let text = String(out.characters)
        var ranges: [Range<AttributedString.Index>] = []
        var from = text.startIndex
        while let found = text.range(of: name, range: from..<text.endIndex) {
            from = found.upperBound
            let before = found.lowerBound == text.startIndex
                ? nil : text[text.index(before: found.lowerBound)]
            let after = found.upperBound == text.endIndex ? nil : text[found.upperBound]
            func isWord(_ c: Character?) -> Bool {
                guard let c else { return false }
                return c.isLetter || c.isNumber || c == "_"
            }
            guard !isWord(before), !isWord(after) else { continue }
            if let lower = AttributedString.Index(found.lowerBound, within: out),
               let upper = AttributedString.Index(found.upperBound, within: out)
            {
                ranges.append(lower..<upper)
            }
        }
        for range in ranges {
            out[range].backgroundColor = QuickHelpFonts.markFill
            out[range].foregroundColor = .black
        }
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
/// JetBrains Mono throughout, which is what was asked for and what the editor
/// behind the card is set in. It was moved to the system face once, on the
/// theory that a monospaced paragraph is hard to read — but the actual
/// complaint was the double wrap (see `blocks(in:naming:)`), and with the
/// paragraphs unwrapped and given line spacing the mono reads fine and belongs
/// here.
///
/// Milker for the name, which is the one thing on the card that is a heading
/// rather than text. `WelcomeView` says Milker "carries A–Z/a–z only", which
/// would have made any name with an underscore a patchwork of two faces;
/// checked against the font's own character set instead — letters, digits, `_`
/// and signature punctuation are all in it, so `looks_binary` sets in one
/// face.
///
/// Resolved once and cached: `NSFont(name:)` is a lookup, and these are read
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

    /// Running text.
    static let body = Font(EditorMetrics.monospaced(11, weight: .regular))
    /// Samples, a shade smaller so a wide line fits before it has to scroll.
    static let code = Font(EditorMetrics.monospaced(10.5, weight: .regular))
    /// A `code` span inside a sentence — the same size as the prose it sits in.
    static let inlineCode = Font(EditorMetrics.monospaced(11, weight: .medium))

    /// `#` through `######`. Three steps, because a hover answer never nests
    /// deeper than that in practice and a heading that is the same size as the
    /// paragraph under it is not a heading.
    static func heading(_ level: Int) -> Font {
        switch level {
        case 1: return Font(EditorMetrics.monospaced(14, weight: .bold))
        case 2: return Font(EditorMetrics.monospaced(12.5, weight: .bold))
        default: return Font(EditorMetrics.monospaced(11.5, weight: .semibold))
        }
    }

    /// The editor's bracket-match yellow. See `QuickHelpBody.marked(_:in:)`.
    static let markFill = Color(EditorCanvasView.bracketYellow)
}
