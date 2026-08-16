//  DatatipCard.swift
//  What a variable is worth, where you pointed at it.
//
//  E3 of docs/SUISEI-DEBUG-IN-EDITOR-PLAN.md, and the reason that document
//  puts the variables list in the PANEL and not in the editor: a value is not
//  spatial — until you point at the thing it belongs to. This is that pointing.
//
//  It shares the Quick Help popover's chrome and type — same presentation, same
//  fonts, same rounded card — because the app should have one idea of what "a
//  small answer beside the code" looks like. What it does NOT share is the
//  body: Quick Help renders a document, and a datatip is a name, a value and a
//  type. Rendering three words through a Markdown block parser would be a page
//  layout wearing a value's clothes.

import SwiftUI

struct DatatipCard: View {
    @ObservedObject var engine: EngineBridge
    /// The identifier the pointer was over when the request went out.
    let symbol: String

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            content
        }
        .frame(minWidth: 180, idealWidth: 260, maxWidth: 420, alignment: .leading)
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "cube.transparent")
                .font(.system(size: 10))
                .foregroundStyle(.tint)
            Text(symbol)
                .font(QuickHelpFonts.title)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 0)
            if let t = engine.datatip?.type, !t.isEmpty {
                Text(t)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }
        }
        .padding(.horizontal, 12)
        .padding(.top, 9)
        .padding(.bottom, 8)
    }

    @ViewBuilder
    private var content: some View {
        // Three states, said differently. Collapsing "waiting" into "nothing"
        // makes the card flicker shut between the ask and the reply, and
        // collapsing "nothing" into "waiting" leaves a spinner on a keyword
        // that will never have a value.
        if let tip = engine.datatip, tip.expr == symbol || tip.expr.isEmpty {
            ScrollView {
                Text(tip.value)
                    .font(.system(size: 11.5, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 9)
            }
            // A struct printed by lldb runs to many lines; past this the card
            // is a window with a tail. Same bound Quick Help draws for the
            // same reason, smaller because a value is not a document.
            .frame(maxHeight: 220)
        } else if engine.datatipPending {
            HStack(spacing: 7) {
                ProgressView().controlSize(.small)
                Text("Evaluating…")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
        } else {
            Text("No value here in this frame.")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
        }
    }
}
