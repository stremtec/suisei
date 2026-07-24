#!/usr/bin/env python3
"""Render Suisei masters (default/dark/mono) from Icon Composer icon.json + Assets."""
from __future__ import annotations

import json
import sys
import unicodedata
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageChops

SIZE = 1024


def parse_srgb(s: str):
    parts = [float(x) for x in s.split(":", 1)[1].split(",")]
    r, g, b = parts[:3]
    a = parts[3] if len(parts) > 3 else 1.0
    return (
        int(round(r * 255)),
        int(round(g * 255)),
        int(round(b * 255)),
        int(round(a * 255)),
    )


def pick_spec(specs, appearance: str | None):
    default = None
    for it in specs or []:
        app = it.get("appearance")
        if app is None:
            default = it.get("value")
        elif appearance and app == appearance:
            return it.get("value")
    return default


def make_bg(cfg, appearance: str) -> Image.Image:
    value = pick_spec(
        cfg.get("fill-specializations"),
        None if appearance == "default" else appearance,
    )
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 255))
    draw = ImageDraw.Draw(img)

    if appearance == "mono":
        return Image.new("RGBA", (SIZE, SIZE), (110, 110, 112, 255))

    if value and "linear-gradient" in value:
        colors = [parse_srgb(c) for c in value["linear-gradient"]]
        orient = value.get("orientation") or {}
        sx = float((orient.get("start") or {}).get("x", 0.5))
        sy = float((orient.get("start") or {}).get("y", 0.0))
        ex = float((orient.get("stop") or {}).get("x", 0.5))
        ey = float((orient.get("stop") or {}).get("y", 1.0))
        dx, dy = ex - sx, ey - sy
        length2 = dx * dx + dy * dy or 1.0
        c0, c1 = colors[0], colors[-1]
        px = img.load()
        for y in range(SIZE):
            for x in range(SIZE):
                cx = x / (SIZE - 1)
                cy = 1.0 - y / (SIZE - 1)  # Composer y-up
                t = ((cx - sx) * dx + (cy - sy) * dy) / length2
                t = 0.0 if t < 0 else 1.0 if t > 1 else t
                px[x, y] = (
                    int(c0[0] + (c1[0] - c0[0]) * t),
                    int(c0[1] + (c1[1] - c0[1]) * t),
                    int(c0[2] + (c1[2] - c0[2]) * t),
                    255,
                )
        return img

    if value and "automatic-gradient" in value:
        for y in range(SIZE):
            t = y / (SIZE - 1)
            r = int(255 - 8 * t)
            g = int(255 - 10 * t)
            b = int(255 - 14 * t)
            draw.line([(0, y), (SIZE, y)], fill=(r, g, b, 255))
        return img

    return Image.new("RGBA", (SIZE, SIZE), (245, 245, 247, 255))


def load_asset(assets: dict, name: str) -> Image.Image:
    name_n = unicodedata.normalize("NFC", name)
    path = assets.get(name_n) or assets.get(name)
    if not path:
        for k, p in assets.items():
            kn = unicodedata.normalize("NFC", k)
            if name_n in kn or name in k:
                path = p
                break
    if not path:
        raise FileNotFoundError(name)
    return Image.open(path).convert("RGBA")


def fill_silhouette(src: Image.Image, rgba) -> Image.Image:
    r, g, b, a_mul = rgba
    out = Image.new("RGBA", src.size, (r, g, b, 0))
    alpha = src.split()[-1]
    if a_mul < 255:
        alpha = alpha.point(lambda v, m=a_mul: int(v * m / 255))
    out.putalpha(alpha)
    return out


def resolve_position(layer: dict, appearance: str):
    specs = layer.get("position-specializations")
    if specs:
        chosen = None
        for it in specs:
            if it.get("idiom") == "square":
                chosen = it.get("value")
                break
        if chosen is None:
            for it in specs:
                if it.get("appearance") is None:
                    chosen = it.get("value")
                    break
        if chosen is None and appearance != "default":
            for it in specs:
                if it.get("appearance") == appearance:
                    chosen = it.get("value")
                    break
        if chosen:
            tr = chosen.get("translation-in-points") or [0, 0]
            return float(chosen.get("scale", 1.0)), (float(tr[0]), float(tr[1]))
    pos = layer.get("position") or {}
    tr = pos.get("translation-in-points") or [0, 0]
    return float(pos.get("scale", 1.0)), (float(tr[0]), float(tr[1]))


def resolve_fill(layer: dict, appearance: str):
    if "fill-specializations" in layer:
        val = pick_spec(
            layer["fill-specializations"],
            "dark" if appearance == "dark" else None,
        )
        if isinstance(val, dict) and "solid" in val:
            return parse_srgb(val["solid"])
    fill = layer.get("fill") or {}
    if "solid" in fill:
        return parse_srgb(fill["solid"])
    return (255, 255, 255, 255)


def resolve_opacity(layer: dict, appearance: str) -> float:
    specs = layer.get("opacity-specializations")
    if not specs:
        return float(layer.get("opacity", 1.0) or 1.0)
    default = 1.0
    for it in specs:
        if it.get("appearance") is None:
            default = float(it.get("value", 1.0))
        elif appearance == "dark" and it.get("appearance") == "dark":
            return float(it.get("value", 1.0))
    return default


def resolve_blend(layer: dict, appearance: str):
    specs = layer.get("blend-mode-specializations")
    if specs:
        for it in specs:
            if appearance == "mono" and it.get("appearance") == "tinted":
                return it.get("value")
        for it in specs:
            if it.get("appearance") is None:
                return it.get("value")
    return layer.get("blend-mode")


def glass_tint(glyph: Image.Image) -> Image.Image:
    w, h = glyph.size
    highlight = Image.new("L", (w, h), 0)
    d = ImageDraw.Draw(highlight)
    for y in range(h):
        v = int(max(0, 36 * (1.0 - y / (h * 0.55))))
        d.line([(0, y), (w, y)], fill=v)
    a = glyph.split()[-1]
    highlight = ImageChops.multiply(highlight, a)
    white = Image.new("RGBA", (w, h), (255, 255, 255, 0))
    white.putalpha(highlight)
    return Image.alpha_composite(glyph, white)


def place(
    canvas: Image.Image,
    glyph: Image.Image,
    scale: float,
    translation,
    opacity: float,
    blend: str | None,
    shadow_opacity: float,
) -> Image.Image:
    w, h = glyph.size
    nw = max(1, int(round(w * scale)))
    nh = max(1, int(round(h * scale)))
    g = glyph.resize((nw, nh), Image.Resampling.LANCZOS)
    if opacity < 0.999:
        a = g.split()[-1].point(lambda v, o=opacity: int(v * o))
        g.putalpha(a)

    tx, ty = translation
    ox = int(round(SIZE / 2 + tx - nw / 2))
    oy = int(round(SIZE / 2 - ty - nh / 2))

    layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    layer.paste(g, (ox, oy), g)

    if shadow_opacity > 0 and blend not in ("plus-lighter",):
        alpha = layer.split()[-1]
        sh = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
        sh.putalpha(alpha.point(lambda v, s=shadow_opacity: int(v * s * 0.5)))
        sh = sh.filter(ImageFilter.GaussianBlur(radius=20))
        shifted = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
        shifted.paste(sh, (0, 10), sh)
        canvas = Image.alpha_composite(canvas, shifted)

    if blend == "plus-lighter":
        # Vectorized-ish: convert and use screen-like add via numpy if available, else pixel loop
        try:
            import numpy as np

            base = np.array(canvas, dtype=np.uint16)
            lay = np.array(layer, dtype=np.uint16)
            a = lay[..., 3:4]
            # premultiplied contribution
            contrib = (lay[..., :3] * a) // 255
            rgb = np.clip(base[..., :3] + contrib, 0, 255).astype(np.uint8)
            alpha = np.maximum(base[..., 3], lay[..., 3]).astype(np.uint8)
            out = np.dstack([rgb, alpha])
            return Image.fromarray(out, "RGBA")
        except Exception:
            return Image.alpha_composite(canvas, layer)

    return Image.alpha_composite(canvas, layer)


def collect_layers(cfg):
    layers = []
    for group in cfg.get("groups") or []:
        gshadow = float((group.get("shadow") or {}).get("opacity", 0.5) or 0.5)
        for layer in group.get("layers") or []:
            if layer.get("hidden") or not layer.get("image-name"):
                continue
            layer = dict(layer)
            layer["_group_shadow"] = gshadow
            layers.append(layer)
    return layers


def render(icon_dir: Path, appearance: str) -> Image.Image:
    cfg = json.loads((icon_dir / "icon.json").read_text())
    assets = {}
    for p in (icon_dir / "Assets").glob("*.png"):
        assets[unicodedata.normalize("NFC", p.name)] = p
        assets[p.name] = p

    canvas = make_bg(cfg, appearance)
    for layer in collect_layers(cfg):
        scale, translation = resolve_position(layer, appearance)
        if appearance == "mono":
            fill = (255, 255, 255, 255)
            # Primary blue-fill layer full; watermark soft
            if layer.get("fill") and "solid" in (layer.get("fill") or {}):
                opacity = 1.0
            else:
                opacity = 0.22
            blend = None
            glass = False
        else:
            fill = resolve_fill(layer, appearance)
            opacity = resolve_opacity(layer, appearance)
            blend = resolve_blend(layer, appearance)
            glass = bool(layer.get("glass"))

        src = load_asset(assets, layer["image-name"])
        glyph = fill_silhouette(src, fill)
        if glass and appearance != "mono":
            glyph = glass_tint(glyph)
        canvas = place(
            canvas,
            glyph,
            scale,
            translation,
            opacity,
            blend,
            float(layer.get("_group_shadow", 0.5)),
        )

    base = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 255))
    return Image.alpha_composite(base, canvas).convert("RGB")


def main():
    icon_dir = Path(sys.argv[1] if len(sys.argv) > 1 else "suisei-app/Resources/Suisei.icon")
    out_dir = Path(sys.argv[2] if len(sys.argv) > 2 else "suisei-app/Resources")
    out_dir.mkdir(parents=True, exist_ok=True)
    mapping = {
        "default": "Suisei.png",
        "dark": "Suisei-dark.png",
        "mono": "Suisei-mono.png",
    }
    for app, name in mapping.items():
        im = render(icon_dir, app)
        path = out_dir / name
        im.save(path, "PNG", optimize=True)
        print(f"  {name} {im.size}")


if __name__ == "__main__":
    main()
