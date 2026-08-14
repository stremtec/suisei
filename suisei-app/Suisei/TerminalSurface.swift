//  TerminalSurface.swift
//  The terminal, as SwiftTerm's own AppKit view.
//
//  Suisei's terminal used to be a Rust emulator whose cell grid was re-encoded
//  to ANSI to cross the C ABI and re-parsed on this side. Four of the five
//  things wrong with it lived at that boundary rather than in the emulator:
//  rows truncated at a byte budget, a grid drawn with proportional text
//  measurement, a 307 KB snapshot per frame, and two scrollers over one
//  content. Moving to a view that owns its own emulator removes the boundary
//  rather than improving it.
//
//  See `third_party/SwiftTerm/VENDOR.md` for what is vendored and why.

import AppKit
import SwiftTerm

/// Compile-time proof that the vendored library is linked and its AppKit view
/// is the shape the port needs. Replaced by the real surface in the next step;
/// until then this is the only thing that imports SwiftTerm, so the build
/// plumbing can be verified and reverted on its own.
enum TerminalSurfaceProbe {
    static func describe() -> String {
        let v = LocalProcessTerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 200))
        let t = v.getTerminal()
        return "SwiftTerm linked · \(t.rows)×\(t.cols) · input client: \(v is NSTextInputClient)"
    }
}
