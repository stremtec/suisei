#!/usr/bin/env python3
"""Document icon for `project.suiseiprj`.

A document icon is not the app icon on a smaller canvas. macOS draws documents
as a portrait sheet with a folded top-right corner, and that silhouette is what
tells a person "this is a file" before they have read anything — Finder relies
on it at 16pt where no glyph survives. So the sheet is drawn here rather than
reusing `Suisei.icns`, and the app's mark sits on it as a badge.

The mark is taken from the Icon Composer package's own asset, so the document
and the app cannot drift apart: there is one drawing of the knot in this
repository and both read it.

    python3 scripts/render_project_icon.py [out.iconset]
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "suisei-app/Resources/Suisei.icon/Assets"

# Rendered large and downsampled: the fold and the sheet's corner radius are
# curves, and drawing them directly at 16px gives stair-steps no icon should
# have.
SUPER = 1024

# Suisei blue, the same #0B6EDE the light palette uses for its accent.
ACCENT = (11, 110, 222, 255)
SHEET = (252, 253, 254, 255)
EDGE = (203, 211, 222, 255)
FOLD = (228, 234, 242, 255)


def mark() -> Image.Image | None:
    """The knot from the app icon package, whitened for the badge."""
    for candidate in sorted(ASSETS.glob("*.png")):
        img = Image.open(candidate).convert("RGBA")
        if img.getbbox():
            return img
    return None


def render() -> Image.Image:
    canvas = Image.new("RGBA", (SUPER, SUPER), (0, 0, 0, 0))
    d = ImageDraw.Draw(canvas)

    # Portrait sheet, inset so the icon has the breathing room every macOS
    # document icon has — a page drawn edge to edge reads as a screenshot.
    left, right = int(SUPER * 0.17), int(SUPER * 0.83)
    top, bottom = int(SUPER * 0.06), int(SUPER * 0.94)
    fold = int(SUPER * 0.20)
    radius = int(SUPER * 0.035)

    body = [
        (left, top + radius),
        (left + radius, top),
        (right - fold, top),
        (right, top + fold),
        (right, bottom - radius),
        (right - radius, bottom),
        (left + radius, bottom),
        (left, bottom - radius),
    ]
    d.polygon(body, fill=SHEET, outline=EDGE, width=max(2, SUPER // 340))

    # The fold, drawn as the triangle the corner leaves behind.
    d.polygon(
        [(right - fold, top), (right, top + fold), (right - fold, top + fold)],
        fill=FOLD,
        outline=EDGE,
        width=max(2, SUPER // 400),
    )

    # A band of accent along the foot: at 16pt the glyph is gone and this is
    # the only thing left that says which app owns the file.
    band_top = int(bottom - SUPER * 0.13)
    d.polygon(
        [
            (left, band_top),
            (right, band_top),
            (right, bottom - radius),
            (right - radius, bottom),
            (left + radius, bottom),
            (left, bottom - radius),
        ],
        fill=ACCENT,
    )

    if (glyph := mark()) is not None:
        size = int(SUPER * 0.34)
        glyph = glyph.resize((size, size), Image.LANCZOS)
        tinted = Image.new("RGBA", glyph.size, ACCENT)
        tinted.putalpha(glyph.getchannel("A"))
        canvas.alpha_composite(
            tinted,
            ((SUPER - size) // 2, int(top + (band_top - top - size) * 0.46)),
        )

    return canvas


def main() -> None:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "suisei-app/Resources/SuiseiProject.iconset"
    out.mkdir(parents=True, exist_ok=True)
    art = render()

    for pt in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            px = pt * scale
            suffix = "" if scale == 1 else "@2x"
            art.resize((px, px), Image.LANCZOS).save(out / f"icon_{pt}x{pt}{suffix}.png")

    icns = out.with_suffix(".icns")
    subprocess.run(["iconutil", "-c", "icns", str(out), "-o", str(icns)], check=True)
    print(f"→ {icns.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
