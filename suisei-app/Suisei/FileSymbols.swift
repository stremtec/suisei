//  FileSymbols.swift
//  One table turning a filename into a glyph and a hue.
//
//  feature.txt ☐23. There were three: the tree's glyph, the tree's colour and
//  the tab chip — each with its own extension switch, and they had already
//  drifted. `gltf`, `glb` and `fbx` open in the model viewer and drew as plain
//  documents in the tree, because `is_model_ext` grew in core and the copies
//  in the face did not. That is this codebase's recurring shape again: a fact
//  core holds, re-derived somewhere else, wrongly.
//
//  So the KIND comes from core — `suisei_engine_classify_name`, the same
//  classifier that decides which viewer a pane gets — and this file only
//  decides what a kind looks like. The language comes from core too, as its
//  canonical extension, so `jsx`, `mjs` and `cjs` all arrive as `js` and the
//  table below has one row for them instead of three.

import SwiftUI

enum FileSymbol {
    /// A colour family, resolved by the caller against its own palette.
    ///
    /// Returned rather than a `Color` because "dim" is the view's own dim, and
    /// a hard-coded grey beside a themed row is the kind of thing that reads
    /// as a rendering fault.
    enum Hue {
        case dim
        case orange, yellow, blue, green, pink, teal, purple
    }

    struct Look {
        var symbol: String
        var hue: Hue
    }

    /// The look for a file NAME. Directories are the caller's business.
    static func look(for name: String) -> Look {
        let key = cacheKey(name)
        if let hit = cache[key] { return hit }
        let look = derive(name)
        cache[key] = look
        return look
    }

    static func symbol(for name: String) -> String { look(for: name).symbol }

    // MARK: - Derivation

    /// Files whose meaning is in the whole NAME rather than the extension.
    private static func special(_ name: String) -> Look? {
        switch name {
        case SuiseiProject.marker:
            // Filled, against the other manifests' outline: it names the
            // project the way they name a package, and it is the only one of
            // them Suisei wrote itself.
            return Look(symbol: "shippingbox.fill", hue: .purple)
        case "Cargo.toml", "Package.swift", "package.json", "go.mod",
             "pyproject.toml", "Gemfile", "CMakeLists.txt", "Makefile":
            return Look(symbol: "shippingbox", hue: .green)
        case ".gitignore", ".gitattributes", ".gitmodules":
            return Look(symbol: "arrow.triangle.branch", hue: .dim)
        case "Dockerfile", "docker-compose.yml":
            return Look(symbol: "shippingbox", hue: .blue)
        case "LICENSE", "LICENCE", "COPYING", "NOTICE":
            return Look(symbol: "scroll", hue: .dim)
        default:
            return nil
        }
    }

    private static func derive(_ name: String) -> Look {
        if let hit = special(name) { return hit }

        var langBuf = [CChar](repeating: 0, count: 32)
        let kindRaw = name.withCString { n in
            langBuf.withUnsafeMutableBufferPointer { buf in
                suisei_engine_classify_name(n, buf.baseAddress, UInt32(buf.count))
            }
        }
        let kind = PaneKind(raw: kindRaw)
        let lang = String(cString: langBuf)

        // Not text: the kind alone is the answer, and it is core's answer —
        // so the tree can never again promise a viewer that will not open.
        switch kind {
        case .image: return Look(symbol: "photo", hue: .pink)
        case .pdf: return Look(symbol: "doc.richtext", hue: .pink)
        case .audio: return Look(symbol: "waveform", hue: .purple)
        case .model: return Look(symbol: "cube", hue: .teal)
        case .binary: return Look(symbol: "doc.fill", hue: .dim)
        case .project: return Look(symbol: "shippingbox.fill", hue: .purple)
        case .text, .terminal, .logic: break
        }

        switch lang {
        case "rs": return Look(symbol: "chevron.left.forwardslash.chevron.right", hue: .orange)
        case "swift": return Look(symbol: "swift", hue: .orange)
        case "js": return Look(symbol: "curlybraces", hue: .yellow)
        case "ts", "tsx": return Look(symbol: "curlybraces", hue: .blue)
        case "py": return Look(symbol: "chevron.left.forwardslash.chevron.right", hue: .blue)
        case "go": return Look(symbol: "chevron.left.forwardslash.chevron.right", hue: .teal)
        case "c", "h", "cpp", "hpp", "m", "cs", "zig", "nim":
            return Look(symbol: "chevron.left.forwardslash.chevron.right", hue: .purple)
        case "java", "kt", "scala", "rb", "php", "lua", "dart", "ex", "hs":
            return Look(symbol: "chevron.left.forwardslash.chevron.right", hue: .orange)
        case "sh": return Look(symbol: "terminal", hue: .green)
        case "html", "css": return Look(symbol: "globe", hue: .orange)
        case "json", "toml", "yaml", "xml": return Look(symbol: "doc.badge.gearshape", hue: .green)
        case "sql": return Look(symbol: "cylinder", hue: .blue)
        case "md": return Look(symbol: "doc.plaintext", hue: .dim)
        default: break
        }

        // No language, no kind of its own. The extension still says a couple
        // of things worth saying, and then it is a document.
        switch (name as NSString).pathExtension.lowercased() {
        case "txt", "rst", "log": return Look(symbol: "doc.plaintext", hue: .dim)
        case "lock": return Look(symbol: "lock.doc", hue: .dim)
        case "zip", "gz", "tar", "xz", "7z": return Look(symbol: "doc.zipper", hue: .dim)
        default: return Look(symbol: "doc", hue: .dim)
        }
    }

    /// Two files with the same extension look the same, so the cache is keyed
    /// by extension — a project has thousands of files and twenty extensions.
    private static func cacheKey(_ name: String) -> String {
        if special(name) != nil { return name }
        let ext = (name as NSString).pathExtension.lowercased()
        return ext.isEmpty ? "\u{1}\(name)" : ext
    }

    private static var cache: [String: Look] = [:]
}
