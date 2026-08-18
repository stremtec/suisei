import AppKit
import SwiftUI

/// The key-equivalent field: click it, press the chord, it takes it.
///
/// An `NSView` and not a SwiftUI gesture, because the whole job is to receive a
/// key press **before anything else does**. `⌘S` typed into a SwiftUI control
/// reaches the menu bar first and saves the file; a recorder that only sees
/// what the menu did not want cannot record the shortcuts a user most wants to
/// change. `performKeyEquivalent` is the one place that runs ahead of the menu,
/// so it is where this lives.
///
/// It only intercepts while RECORDING. Off, it is an ordinary view and ⌘S
/// saves, which is what the rest of the settings window expects.
final class ShortcutRecorderView: NSView {
    var onChord: ((String) -> Void)?
    var onCancel: (() -> Void)?

    var isRecording = false {
        didSet {
            guard isRecording != oldValue else { return }
            if isRecording {
                window?.makeFirstResponder(self)
            } else if window?.firstResponder === self {
                window?.makeFirstResponder(nil)
            }
            needsDisplay = true
        }
    }

    override var acceptsFirstResponder: Bool { isRecording }

    /// Ahead of the menu bar, and only while armed.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard isRecording else { return false }
        return take(event)
    }

    override func keyDown(with event: NSEvent) {
        guard isRecording, take(event) else {
            super.keyDown(with: event)
            return
        }
    }

    /// Turn an event into the notation core reads, or end the recording.
    ///
    /// Escape cancels rather than binding: it is how every recorder on this
    /// platform gets out, and a user who has changed their mind must not be
    /// left holding a binding they did not want.
    private func take(_ event: NSEvent) -> Bool {
        let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        // A modifier on its own is a user mid-chord, not an answer.
        guard let raw = event.charactersIgnoringModifiers, !raw.isEmpty else {
            return true
        }
        if event.keyCode == 53, mods.isEmpty {  // Escape
            isRecording = false
            onCancel?()
            return true
        }

        var chord = ""
        if mods.contains(.control) { chord += "⌃" }
        if mods.contains(.option) { chord += "⌥" }
        if mods.contains(.shift) { chord += "⇧" }
        if mods.contains(.command) { chord += "⌘" }
        chord += Self.keyName(event)

        isRecording = false
        onChord?(chord)
        return true
    }

    /// The key, spelled the way core spells it.
    ///
    /// `charactersIgnoringModifiers` is deliberate: it is the UNSHIFTED
    /// character, which is what an AppKit key equivalent is. Reading
    /// `characters` instead would record ⇧⌘/ as ⇧⌘? and the binding would then
    /// never fire.
    private static func keyName(_ event: NSEvent) -> String {
        switch event.keyCode {
        case 48: return "⇥"
        case 36, 76: return "↩"
        case 49: return "␣"
        case 51: return "⌫"
        case 123: return "←"
        case 124: return "→"
        case 126: return "↑"
        case 125: return "↓"
        default: break
        }
        let raw = event.charactersIgnoringModifiers ?? ""
        return raw.uppercased()
    }
}

struct ShortcutRecorder: NSViewRepresentable {
    @Binding var recording: Bool
    var onChord: (String) -> Void

    func makeNSView(context: Context) -> ShortcutRecorderView {
        let v = ShortcutRecorderView()
        v.onChord = { chord in
            recording = false
            onChord(chord)
        }
        v.onCancel = { recording = false }
        return v
    }

    func updateNSView(_ view: ShortcutRecorderView, context: Context) {
        view.onChord = { chord in
            recording = false
            onChord(chord)
        }
        view.onCancel = { recording = false }
        if view.isRecording != recording { view.isRecording = recording }
    }
}

// MARK: - The menu reads the table

/// The chords the menu bar is currently drawing, as an object the `Commands`
/// tree can observe.
///
/// Its own object for the reason `MenuState` is one: `suiseiCommands` reads a
/// lot, and making the scene observe `EngineBridge` is what used to rebuild the
/// entire menu bar on every keystroke (see the d8afc22 note). This publishes
/// only when a binding actually moves, which is when the user moves it.
final class KeymapState: ObservableObject {
    @Published private(set) var chords: [String: String] = [:]

    func adopt(_ next: [String: String]) {
        if next != chords { chords = next }
    }

    /// The AppKit pair for a command, or nil to leave the item unbound.
    func equivalent(_ id: String) -> (key: KeyEquivalent, modifiers: EventModifiers)? {
        guard let chord = chords[id] else { return nil }
        return Self.parse(chord)
    }

    /// Core's notation → SwiftUI's pair.
    ///
    /// Core owns the notation and both sides read the same strings; this is the
    /// one place it becomes AppKit's types. Keep it here so there is exactly
    /// one such place.
    static func parse(_ chord: String) -> (key: KeyEquivalent, modifiers: EventModifiers)? {
        var mods: EventModifiers = []
        var rest = ""
        for ch in chord {
            switch ch {
            case "⌘": mods.insert(.command)
            case "⇧": mods.insert(.shift)
            case "⌥": mods.insert(.option)
            case "⌃": mods.insert(.control)
            default: rest.append(ch)
            }
        }
        guard !rest.isEmpty else { return nil }
        let key: KeyEquivalent
        switch rest {
        case "⇥": key = .tab
        case "↩": key = .return
        case "␣": key = .space
        case "⌫": key = .delete
        case "←": key = .leftArrow
        case "→": key = .rightArrow
        case "↑": key = .upArrow
        case "↓": key = .downArrow
        default:
            // The UNSHIFTED character, which is what a key equivalent is. Core
            // stores it that way for the same reason.
            guard let c = rest.lowercased().first else { return nil }
            key = KeyEquivalent(c)
        }
        return (key, mods)
    }
}

extension View {
    /// Bind this menu item to whatever `id` is on right now.
    ///
    /// The point of the whole feature: the Shortcuts page edits a table, and
    /// this is what reads it. A menu item still carrying a literal
    /// `.keyboardShortcut("s", modifiers: .command)` is one the page cannot
    /// change, and the page would be lying about it.
    @ViewBuilder
    func suiseiShortcut(_ id: String, _ keys: KeymapState) -> some View {
        if let k = keys.equivalent(id) {
            self.keyboardShortcut(k.key, modifiers: k.modifiers)
        } else {
            self
        }
    }
}

/// Wraps a menu's contents so they REDRAW when a binding moves.
///
/// `SuiseiApp` holds the engine as a plain `let` on purpose — `@StateObject`
/// subscribes, and a scene that observes the engine rebuilds the entire menu
/// bar on every keystroke (d8afc22). The cost of that choice is that
/// `suiseiCommands` is evaluated once, so a menu item reading the keymap would
/// read it once too and a rebind would not show until relaunch.
///
/// This is the narrow subscription that fixes it: one observer per menu, on the
/// one object that changes when a shortcut changes, and on nothing else.
struct Keyed<Content: View>: View {
    @ObservedObject private var keys: KeymapState
    @ViewBuilder private var content: () -> Content

    init(_ keys: KeymapState, @ViewBuilder content: @escaping () -> Content) {
        self.keys = keys
        self.content = content
    }

    var body: some View { content() }
}
