# fleet-probe

The scripts that read Fleet's values, so a measurement can be redone when
Fleet moves. What they found on 2026-11-01 is written up in
`docs/fleet-probe.md`.

| Script | What it does |
| --- | --- |
| `extract-themes.py <Fleet dir or tar.gz> <out>` | pulls `fleet/themes/*.json` out of the distribution's jars and writes a `.resolved.json` beside each with palette names replaced by hex |
| `make-oak-themes.py <out> crates/oak/assets/themes` | writes Oak's bundled themes (the subset of keys Oak paints from) from those resolved files |
| `inventory.py <Fleet dir or tar.gz> <out>` | writes Fleet's own name lists (bound and documented action ids, settings keys, theme keys, icon names, plugin groups) and copies of its keymap presets and themes into `<out>` (`tools/fleet-probe/out/` is gitignored for this) |
| `coverage.py <out> [-v]` | reports how many of those actions, colour keys, text attributes and settings keys Oak references; `-v` lists the missing ones |
| `rle.py <image> <geometry> [min-run]` | run-length encodes one row or column of a screenshot (`1x1120+20+0` is a column at x=20), which is how the first layout pass was measured |
| `make-fixture.py <dir>` | generates the deterministic fixture workspace both editors open (one file per grammar, a 1000-line file, a 300-column line, compact-folder chain, dotfiles, a 2 MB file, a git history with modified, added, renamed and untracked files) |
| `winprobe.py <command> ...` | Windows driver: launches Fleet or Oak, places the client area at a logical size, sends keys, text and pointer events, grabs the client area; `run <scene> fleet|oak <fixture> <out>` plays a scene file unattended; `attach <scene> fleet <pid> <out>` plays it into a Fleet window that is already open (Fleet is a single instance, so `run` cannot start a second one) ||oak <fixture> <out>` plays a scene file unattended |
| `compare.py <scene> <fleet.png> <oak.png> [--diff out.png] [--markdown]` | run-length encodes the scene's scan lines on both shots, pairs the edges and prints the deltas in logical px; exit 1 when one is off by more than the tolerance |
| `scenes/*.json` | one file per scene: window size, driver steps per editor, scan lines and colour samples. `editor` is the M0 reference scene; `light`, `panels`, `splitters` and `tabs` are M1's batches 1–3; `surface` and `surface2` are M2's batch 5, `find`, `fold` and `wrap` the M2 surfaces built after it; `goto` and `popups` are M3's batches 6 and 7, `settings` its batch 8, `dialogs` the notifications, rename and delete popups, `leftovers` what M3 still needed from Fleet (the modified-value mark, the auto save values, the shortcut overlay, the History tool), `git`, `git2`, `git3` and `git4` are M4's batch 10 (each later one a correction of the one before; `docs/fleet-probe.md`, ninth pass, says which shots came from which); `settings-live` and `splits` (Oak only) change the settings file mid-run and show one document in two splits. A scene's `fleet_settings` is written to Fleet's `settings.json` for the run; `oak_args` are extra Oak arguments with `{workspace}` expanded |

A probe on Windows, start to finish (Python with Pillow; Fleet found through `FLEET_HOME` or `~/Downloads/Fleet-*`, Oak through `OAK_EXE`):

```
python make-fixture.py ~/oak-probe/probe-fixture-with-a-deliberately-long-workspace-name
python winprobe.py run scenes/editor.json fleet ~/oak-probe/probe-fixture-... ~/oak-probe/out
python winprobe.py run scenes/editor.json oak   ~/oak-probe/probe-fixture-... ~/oak-probe/out
python compare.py scenes/editor.json ~/oak-probe/out/editor-fleet-editor.png ~/oak-probe/out/editor-oak-editor.png --diff ~/oak-probe/out/editor-diff.png
```

Run the driver in the foreground only: its synthetic input goes to the window in front, and a run started in the background types into the terminal instead. Each Oak launch gets a fresh `OAK_CONFIG_DIR` under `%LOCALAPPDATA%\oak-probe`, so a run never inherits the last one's layout.

The harnesses still to build (Oak's in-app `--probe` mode, the keymap oracle, the colour lint, the KDE launchers) are described in `docs/probe-plan.md`; the measurement batches they serve are in `docs/roadmap.md`.

Screenshots were taken with KDE's `spectacle -b -n -f -o out.png` while Fleet
ran on a scratch workspace; `magick <shot> -crop WxH+X+Y +repage win.png`
cuts the window out, and `rle.py` reads the boundaries off it. Fleet's
default keymap lives in `fleet/keymap/fleet.json` inside
`fleet.plugins.keymap.frontend-*.jar` (JSON with comments and trailing
commas); the subset Oak binds is `crates/oak/assets/keymap/default.json`.
