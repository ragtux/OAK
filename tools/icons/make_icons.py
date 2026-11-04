#!/usr/bin/env python3
"""Render WEEP's logo into the application icons each platform wants.

    python tools/icons/make_icons.py         writes all three
    python tools/icons/make_icons.py --ico   just Windows

    crates/weep/assets/icons/weep/weep.ico    Windows: build.rs links it as the
                                              icon and version resource
    crates/weep/assets/icons/weep/weep.icns   macOS: WEEP.app's CFBundleIconFile
    crates/weep/assets/icons/weep/weep.png    Linux: the .desktop entry's icon

The drawing is `crates/weep/assets/icons/weep/logo.svg` (WEEP's own, MIT): a leaf
on the 16-unit grid, its outline at 25% fill with a 1-unit stroke, and a stem
with two veins. Pillow cannot read SVG, so the paths are copied here; keep the
two in step. The leaf is painted in WEEP's dark-theme foreground (#E0E1E4, the
toolbar logo's color) on a rounded square of the panel background (#18191B),
both measured off a screenshot of the running app.

Each size is drawn 8x larger and downsampled, so the small sizes stay crisp
rather than being shrunk from 256. Needs Pillow; `iconutil` is not needed, the
.icns is written here (its entries are PNGs).
"""
import io
import os
import struct
import sys

from PIL import Image, ImageDraw

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ICONS = os.path.join(REPO, "crates", "weep", "assets", "icons", "weep")
OUT = os.path.join(ICONS, "weep.ico")
OUT_ICNS = os.path.join(ICONS, "weep.icns")
OUT_PNG = os.path.join(ICONS, "weep.png")

SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
# The .icns entry types, each a PNG at its pixel size: the 1x sizes (icp*) and
# the Retina pairs (ic11 is 16@2x, ic14 256@2x), as Apple's icon sets carry them.
ICNS_TYPES = [
    (b"icp4", 16), (b"icp5", 32), (b"ic11", 32), (b"ic12", 64),
    (b"ic07", 128), (b"ic13", 256), (b"ic08", 256), (b"ic14", 512),
    (b"ic09", 512), (b"ic10", 1024),
]
# Linux desktop icon: one PNG, at the size the menus and docks ask for most.
PNG_SIZE = 256
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


def write_ico():
    images = [render(s) for s in SIZES]
    images[-1].save(OUT, format="ICO", sizes=[(s, s) for s in SIZES], append_images=images[:-1])
    print(OUT)


def write_icns():
    """An .icns is "icns" + total length, then type + length + payload per entry."""
    drawn = {}
    entries = []
    for kind, size in ICNS_TYPES:
        if size not in drawn:
            buf = io.BytesIO()
            render(size).save(buf, format="PNG")
            drawn[size] = buf.getvalue()
        png = drawn[size]
        entries.append(kind + struct.pack(">I", len(png) + 8) + png)
    body = b"".join(entries)
    with open(OUT_ICNS, "wb") as f:
        f.write(b"icns" + struct.pack(">I", len(body) + 8) + body)
    print(OUT_ICNS)


def write_png():
    render(PNG_SIZE).save(OUT_PNG, format="PNG")
    print(OUT_PNG)


def main():
    wanted = [a for a in sys.argv[1:] if a.startswith("--")]
    for flag in wanted:
        if flag not in ("--ico", "--icns", "--png"):
            print(f"usage: make_icons.py [--ico] [--icns] [--png]", file=sys.stderr)
            return 2
    if not wanted or "--ico" in wanted:
        write_ico()
    if not wanted or "--icns" in wanted:
        write_icns()
    if not wanted or "--png" in wanted:
        write_png()


if __name__ == "__main__":
    sys.exit(main())
