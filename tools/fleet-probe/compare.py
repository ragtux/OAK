#!/usr/bin/env python3
"""Compare a Fleet shot with an Oak shot along the scan lines a scene names (harness D).

    compare.py <scene.json> <fleet.png> <oak.png> [--tolerance 1] [--markdown] [--min-run 2] [--diff out.png]

Both shots must be the same client area at the same scale (`winprobe.py` places both windows
at the scene's `size`; the scale is read from the image width divided by the scene width).

A scene file holds:

    "size": [1280, 800]                      logical client size both editors were placed at
    "scans": [                               lines to run-length encode, in logical px
      {"name": "left edge", "axis": "y", "at": 20, "from": 0, "to": 800,
       "edges": ["toolbar bottom", "island top", "panel tab top", ...]},
      {"name": "toolbar row", "axis": "x", "at": 18}
    ]
    "samples": [                             points whose colour must match a theme key
      {"name": "island", "at": [40, 300], "key": "island.background"}
    ]

For each scan the tool prints the colour transitions along the line on both sides (position in
logical px, colour before -> after), pairs them in order, and prints the delta. Transitions
shorter than --min-run logical px (anti-aliased text) are skipped. `edges` names the transitions
the scene expects, in order, so the table reads "toolbar bottom: Fleet 36.0, Oak 36.0, delta 0".
With --markdown the rows come out in the format of docs/fleet-probe.md. Exit status is 1 when
any paired delta exceeds --tolerance. --diff writes a side-by-side image with the scan lines
drawn on both shots.

Needs Pillow.
"""
import json
import os
import sys

from PIL import Image, ImageDraw


def runs_along(img, axis, at, lo, hi, scale):
    """Run-length encode a column (axis 'y': walk down at x=at) or a row (axis 'x': walk right
    at y=at). Returns [(start_logical, length_logical, (r, g, b))]."""
    px = img.load()
    w, h = img.size
    dev = round(at * scale)
    out = []
    start, end = round(lo * scale), min(round(hi * scale), h if axis == "y" else w)
    prev = None
    for i in range(start, end):
        x, y = (dev, i) if axis == "y" else (i, dev)
        if not (0 <= x < w and 0 <= y < h):
            break
        c = px[x, y][:3]
        if prev is not None and c == prev[2]:
            prev[1] += 1
        else:
            prev = [i, 1, c]
            out.append(prev)
    return [(s / scale, n / scale, c) for s, n, c in out]


def transitions(runs, min_run):
    """Positions where a run of at least min_run ends and another of at least min_run begins,
    skipping the thin runs (glyph edges) in between."""
    sig = [(s, n, c) for s, n, c in runs if n >= min_run]
    out = []
    for (s1, n1, c1), (s2, n2, c2) in zip(sig, sig[1:]):
        out.append((s2, c1, c2))
    return out


def hexc(c):
    return "#%02X%02X%02X" % c


def pair(a, b, tolerance):
    """Pair transitions in order: walk both lists, matching the closest next positions."""
    i = j = 0
    rows = []
    while i < len(a) and j < len(b):
        pa, pb = a[i][0], b[j][0]
        if abs(pa - pb) <= max(tolerance * 4, 6):
            rows.append((a[i], b[j]))
            i += 1
            j += 1
        elif pa < pb:
            rows.append((a[i], None))
            i += 1
        else:
            rows.append((None, b[j]))
            j += 1
    rows += [(t, None) for t in a[i:]] + [(None, t) for t in b[j:]]
    return rows


def main(argv):
    opts = {"--tolerance": "1", "--min-run": "2", "--diff": None}
    flags = set()
    args = []
    it = iter(argv)
    for a in it:
        if a in opts:
            opts[a] = next(it)
        elif a.startswith("--"):
            flags.add(a)
        else:
            args.append(a)
    if len(args) != 3:
        sys.exit(__doc__)
    scene = json.load(open(args[0]))
    fleet, oak = Image.open(args[1]).convert("RGB"), Image.open(args[2]).convert("RGB")
    size = scene.get("size", [1280, 800])
    scale = fleet.size[0] / size[0]
    if abs(oak.size[0] / size[0] - scale) > 0.01 or fleet.size != oak.size:
        print(f"warning: shots differ in size: fleet {fleet.size}, oak {oak.size}", file=sys.stderr)
    tol, min_run = float(opts["--tolerance"]), float(opts["--min-run"])
    markdown = "--markdown" in flags
    failed = False

    print(f"scale {scale:.3f} device px per logical px; tolerance {tol} logical px\n")
    for scan in scene.get("scans", []):
        axis, at = scan["axis"], scan["at"]
        lo, hi = scan.get("from", 0), scan.get("to", size[1] if axis == "y" else size[0])
        ta = transitions(runs_along(fleet, axis, at, lo, hi, scale), min_run)
        tb = transitions(runs_along(oak, axis, at, lo, hi, scale), min_run)
        names = scan.get("edges", [])
        where = f"{'x' if axis == 'y' else 'y'}={at}"
        if markdown:
            print(f"| {scan['name']} ({where}) | Fleet | Oak | delta |\n| --- | --- | --- | --- |")
        else:
            print(f"== {scan['name']} ({where}, {lo}..{hi})")
        for k, (fa, ob) in enumerate(pair(ta, tb, tol)):
            name = names[k] if k < len(names) else f"edge {k}"
            fpos = f"{fa[0]:.1f}" if fa else "-"
            opos = f"{ob[0]:.1f}" if ob else "-"
            delta = f"{ob[0] - fa[0]:+.1f}" if fa and ob else "?"
            fcol = f"{hexc(fa[1])}->{hexc(fa[2])}" if fa else ""
            ocol = f"{hexc(ob[1])}->{hexc(ob[2])}" if ob else ""
            bad = (fa is None) or (ob is None) or abs(ob[0] - fa[0]) > tol
            colour_bad = fa and ob and (fa[1] != ob[1] or fa[2] != ob[2])
            failed |= bad
            mark = "  <-- " + ("missing" if (fa is None or ob is None) else "off") if bad else ("  (colour)" if colour_bad else "")
            if markdown:
                print(f"| {name} | {fpos} | {opos} | {delta} |")
            else:
                print(f"  {name:<28} fleet {fpos:>7} {fcol:<18} oak {opos:>7} {ocol:<18} {delta:>6}{mark}")
        print()

    for sample in scene.get("samples", []):
        x, y = sample["at"]
        dx, dy = round(x * scale), round(y * scale)
        cf, co = fleet.getpixel((dx, dy)), oak.getpixel((dx, dy))
        same = cf == co
        failed |= not same
        print(f"sample {sample['name']:<24} ({x},{y})  fleet {hexc(cf)}  oak {hexc(co)}  {'ok' if same else '<-- differs'}"
              + (f"  key {sample['key']}" if sample.get("key") else ""))

    if opts["--diff"]:
        w, h = fleet.size
        out = Image.new("RGB", (w * 2 + 10, h), (255, 0, 255))
        out.paste(fleet, (0, 0))
        out.paste(oak, (w + 10, 0))
        d = ImageDraw.Draw(out)
        for scan in scene.get("scans", []):
            at = round(scan["at"] * scale)
            for off in (0, w + 10):
                if scan["axis"] == "y":
                    d.line([(off + at, 0), (off + at, h)], fill=(255, 0, 255))
                else:
                    d.line([(off, at), (off + w, at)], fill=(255, 0, 255))
        out.save(opts["--diff"])
        print(f"\nwrote {opts['--diff']}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main(sys.argv[1:])
