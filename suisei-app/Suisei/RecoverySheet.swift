import SwiftUI

/// Crash-recovery sheet: lists unsaved buffers found in the WAL journal,
/// lets the user accept (open with recovered content) or discard each.
struct RecoverySheet: View {
    @ObservedObject var engine: EngineBridge

    private let panelBg = Color(red: 0.12, green: 0.12, blue: 0.13)
    private let rowBg = Color.white.opacity(0.06)
    private let accent = Color(red: 0.42, green: 0.62, blue: 0.95)

    var body: some View {
        VStack(spacing: 0) {
            // Header
            HStack(spacing: 10) {
                Image(systemName: "lifeline.arrow.triangle.branch")
                    .symbolRenderingMode(.hierarchical)
                    .font(.system(size: 18, weight: .medium))
                    .foregroundStyle(accent)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Unsaved Changes Found")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(.white.opacity(0.95))
                    Text("Suisei recovered unsaved work from a previous session. Review and accept to restore, or discard to delete permanently.")
                        .font(.system(size: 11))
                        .foregroundStyle(.white.opacity(0.50))
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer()
            }
            .padding(.horizontal, 18)
            .padding(.top, 18)
            .padding(.bottom, 14)

            // Recovery entries list
            ScrollView {
                VStack(spacing: 6) {
                    ForEach(engine.recoveryEntries) { item in
                        HStack(spacing: 10) {
                            Image(systemName: "doc.fill")
                                .font(.system(size: 12))
                                .foregroundStyle(accent.opacity(0.70))
                            VStack(alignment: .leading, spacing: 1) {
                                Text(item.name)
                                    .font(.system(size: 12, weight: .medium))
                                    .foregroundStyle(.white.opacity(0.90))
                                Text(item.path)
                                    .font(.system(size: 10))
                                    .foregroundStyle(.white.opacity(0.35))
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }
                            Spacer()
                            Button("Discard") {
                                engine.discardRecovery(item)
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                            Button("Recover") {
                                engine.acceptRecovery(item)
                            }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                        }
                        .padding(.horizontal, 10)
                        .padding(.vertical, 8)
                        .background(rowBg, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                    }
                }
                .padding(.horizontal, 18)
            }

            // Footer
            HStack {
                Button("Discard All") {
                    engine.discardAllRecovery()
                }
                .foregroundStyle(.red.opacity(0.85))
                Spacer()
                Button("Close") {
                    engine.recoverySheetShown = false
                }
                .keyboardShortcut(.escape)
            }
            .padding(.horizontal, 18)
            .padding(.top, 10)
            .padding(.bottom, 16)
        }
        .frame(width: 460, height: 320)
        .background(panelBg, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .preferredColorScheme(.dark)
    }
}
