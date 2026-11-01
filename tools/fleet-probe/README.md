# fleet-probe

The scripts that read Fleet's values, so a measurement can be redone when
Fleet moves. What they found on 2026-11-01 is written up in
`docs/fleet-probe.md`.

| Script | What it does |
| --- | --- |
| `extract-themes.py <Fleet dir or tar.gz> <out>` | pulls `fleet/themes/*.json` out of the distribution's jars and writes a `.resolved.json` beside each with palette names replaced by hex |
| `make-oak-themes.py <out> crates/oak/assets/themes` | writes Oak's bundled themes (the subset of keys Oak paints from) from those resolved files |
| `rle.py <image> <geometry> [min-run]` | run-length encodes one row or column of a screenshot (`1x1120+20+0` is a column at x=20), which is how the layout was measured |

Screenshots were taken with KDE's `spectacle -b -n -f -o out.png` while Fleet
ran on a scratch workspace; `magick <shot> -crop WxH+X+Y +repage win.png`
cuts the window out, and `rle.py` reads the boundaries off it. Fleet's
default keymap lives in `fleet/keymap/fleet.json` inside
`fleet.plugins.keymap.frontend-*.jar` (JSON with comments and trailing
commas); the subset Oak binds is `crates/oak/assets/keymap/default.json`.
