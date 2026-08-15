// Rasterise SuiseiProject.svg into an .icns.
//
// Swift rather than the Python that drew the first version, because the mark is
// an SVG now and macOS reads SVG natively (`NSImage`). Every alternative wanted
// a dependency this repository does not have — PIL cannot parse SVG, and
// rsvg/cairo would be a Homebrew package standing between a checkout and a
// build.
//
// The SVG is the single source. It is monochrome and drawn with
// `currentColor`, so the ink is chosen HERE, once per appearance:
//
//   * the document icon is drawn dark on the sheet, because Finder composites
//     it over the file's own light background at every size;
//   * `currentColor` in an NSImage resolves to the current appearance's text
//     colour, so the render forces one rather than inheriting whatever the
//     build machine happens to be set to. A build must not depend on whether
//     the person running it has dark mode on.
//
//   swift scripts/render_project_icon.swift <in.svg> <out.iconset>

import AppKit
import Foundation

let args = CommandLine.arguments
guard args.count >= 3 else {
    FileHandle.standardError.write(Data("usage: render_project_icon.swift <svg> <iconset>\n".utf8))
    exit(2)
}
let svg = URL(fileURLWithPath: args[1])
let out = URL(fileURLWithPath: args[2])

guard let source = NSImage(contentsOf: svg) else {
    FileHandle.standardError.write(Data("cannot read \(svg.path)\n".utf8))
    exit(1)
}
// An SVG-backed NSImage is resolution independent; asking it to draw at a size
// re-renders the vectors rather than scaling a bitmap.
source.isTemplate = false

try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

/// Suisei blue, the accent the light palette uses. One ink for the whole mark:
/// it is a monochrome drawing and giving it two colours here would undo the
/// reason it is monochrome.
let ink = NSColor(srgbRed: 11 / 255, green: 110 / 255, blue: 222 / 255, alpha: 1)

func render(_ px: Int) -> Data? {
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: px, pixelsHigh: px,
        bitsPerSample: 8, samplesPerPixel: 4,
        hasAlpha: true, isPlanar: false,
        colorSpaceName: .calibratedRGB,
        bytesPerRow: 0, bitsPerPixel: 0
    ) else { return nil }

    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current?.imageInterpolation = .high

    let box = NSRect(x: 0, y: 0, width: CGFloat(px), height: CGFloat(px))
    source.draw(in: box)

    // Tint by keeping the drawn alpha and replacing the colour. `currentColor`
    // resolved to the build machine's label colour, which is not a thing an
    // icon should be built out of.
    ink.set()
    box.fill(using: .sourceAtop)

    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])
}

for pt in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let px = pt * scale
        guard let data = render(px) else {
            FileHandle.standardError.write(Data("render failed at \(px)px\n".utf8))
            exit(1)
        }
        let suffix = scale == 1 ? "" : "@2x"
        try data.write(to: out.appendingPathComponent("icon_\(pt)x\(pt)\(suffix).png"))
    }
}
print("→ \(out.lastPathComponent)")
