#!/usr/bin/env python3
"""Render WEEP's logo into the Windows application icon.

    python tools/windows/make_icon.py        writes crates/weep/assets/icons/weep/weep.ico

The drawing is `crates/weep/assets/icons/weep/logo.svg` (WEEP's own, MIT): a leaf
on the 16-unit grid, its outline at 25% fill with a 1-unit stroke, and a stem
with two veins. Pillow cannot read SVG, so the paths are copied here; keep the
two in step. The leaf is painted in WEEP's dark-theme foreground (#E0E1E4, the
toolbar logo's colour) on a rounded square of the panel background (#18191B),
both measured off a screenshot of the running app.

Each size is drawn 8x larger and downsampled, so the small sizes stay crisp
rather than being shrunk from 256. Needs Pillow.
"""
import os
import sys

from PIL import Image, ImageDraw

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(REPO, "crates", "weep", "assets", "icons", "weep", "weep.ico")

SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
SUPERSAMPLE = 8
BACKGROUND = (0x18, 0x19, 0x1B, 255)
FOREGROUND = (0xE0, 0xE1, 0xE4)

# logo.svg, path 1: M8 1.5 C4.4 1.5 2 4.2 2 7.5 C2 10.3 4 13 8 14.5
#                   C12 13 14 10.3 14 7.5 C14 4.2 11.6 1.5 8 1.5 Z
LEAF = [
    ((8, 1.5), (4.4, 1.5), (2, 4.2), (2, 7.5)),
    ((2, 7.5), (2, 10.3), (4, 13), (8, 14.5)),
    ((8, 14.5), (12, 13), (14, 10.3), (14, 7.5)),
    ((14, 7.5), (14, 4.2), (11.6, 1.5), (8, 1.5)),
]
# logo.svg, path 2 (round caps): M8 5 V11.5  M8 7.5 L5.8 6  M8 9.5 L10.2 8
VEINS = [((8, 5), (8, 11.5)), ((8, 7.5), (5.8, 6)), ((8, 9.5), (10.2, 8))]


def cubic(p0, p1, p2, p3, steps=64):
    pts = []
    for i in range(steps):
        t = i / steps
        u = 1 - t
        pts.append(tuple(
            u * u * u * a + 3 * u * u * t * b + 3 * u * t * t * c + t * t * t * d
            for a, b, c, d in zip(p0, p1, p2, p3)
        ))
    return pts


def render(size):
    big = size * SUPERSAMPLE
    unit = big / 16  # one unit of the logo's 16-unit grid
    im = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((0, 0, big - 1, big - 1), radius=round(big * 0.22), fill=BACKGROUND)

    # Small icons get a heavier stroke so the outline survives the downsample.
    stroke = unit * (1.0 if size >= 32 else 1.25)

    def at(p):
        return (p[0] * unit, p[1] * unit)

    outline = [at(p) for seg in LEAF for p in cubic(*seg)]
    layer = Image.new("RGBA", im.size, (0, 0, 0, 0))
    ImageDraw.Draw(layer).polygon(outline, fill=FOREGROUND + (64,))  # fill-opacity 0.25
    im.alpha_composite(layer)
    # Stroke by stamping round dots densely along the curve: Pillow's line joints
    # leave ticks on a polyline this fine.
    r = stroke / 2
    for x, y in (at(p) for seg in LEAF for p in cubic(*seg, steps=1024)):
        d.ellipse((x - r, y - r, x + r, y + r), fill=FOREGROUND + (255,))

    for a, b in VEINS:
        a, b = at(a), at(b)
        d.line([a, b], fill=FOREGROUND + (255,), width=round(stroke))
        r = stroke / 2
        for x, y in (a, b):
            d.ellipse((x - r, y - r, x + r, y + r), fill=FOREGROUND + (255,))

    return im.resize((size, size), Image.Resampling.LANCZOS)


def main():
    images = [render(s) for s in SIZES]
    images[-1].save(OUT, format="ICO", sizes=[(s, s) for s in SIZES], append_images=images[:-1])
    print(OUT)


if __name__ == "__main__":
    sys.exit(main())
