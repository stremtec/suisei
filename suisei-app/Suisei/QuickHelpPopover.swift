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
        .frame(width: 360)
    }

    @ViewBuilder
    private func body(for text: String) -> some View {
        if !text.isEmpty {
            ScrollView {
                Text(text)
                    .font(.system(size: 11))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
            }
            .frame(maxHeight: 260)
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
