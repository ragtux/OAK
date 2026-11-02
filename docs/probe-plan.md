# Probe plan: how Fleet gets measured, and the harnesses for it

Companion to `docs/roadmap.md` (what to build) and `docs/fleet-probe.md`
(what has been measured so far and where each value came from). This note
is the method: the sources in order of trust, the harnesses to build so a
probe is a command rather than an afternoon, and the batches of
measurements each milestone needs.

## Sources, most trusted first

1. **Fleet's data files.** Themes, keymaps, icons, fonts and the plugin
   `documentation.yaml` files are in the tarball and say exactly what Fleet
   uses. `tools/fleet-probe/inventory.py` extracts them. Found so far:
   - `fleet/themes/{dark,dark_purple,light}.json`: 684 colour keys, 157
     text attribute keys.
   - `fleet/keymap/fleet.json` plus seven presets: 203 bound action ids and
     every chord per platform.
   - 63 `documentation.yaml` files: 385 action ids and 430 settings keys,
     each with a one-line description. This is Fleet's own feature list.
   - `frontend/icons/dark/`: 441 icon names, the map for choosing expui
     equivalents (the SVGs themselves stay out of Oak).
2. **Fleet's runtime state** in `~/.config/JetBrains/Fleet` and
   `~/.cache/JetBrains/Fleet`.
   - `localStorage/keymap.0.json` is Fleet's *resolved* keymap for this
     machine: 240 trigger → chord entries with Fleet's own key-code spelling
     (`Down[Arrow,Navigation]`, `I[Character,Letter]`) and presentable
     names. It is an oracle for `oak-keymap` (harness E).
   - `log/fleet.log` logs each window's logical size (1280 × 800 default),
     which pins the screenshot scale.
   - `localStorage/recent_ships.2.json` shows the launcher arguments Fleet
     accepts (`--scratch`; the jars also mention `--workspace`, `--wait`).
   - `settings.json` is the probe's control surface: `zoomLevel` and
     `theme` are read on launch.
3. **Fleet's Settings tool.** It shows the default of every one of the 430
   keys; a screenshot series of its pages is the cheapest way to get
   defaults that `documentation.yaml` does not carry (hover delay, caret
   blink, tab limit, scroll sensitivity).
4. **Screenshots** of Fleet on a fixture workspace, measured with
   `rle.py` / `compare.py`. The only source for geometry.
5. **Screen recordings** (`spectacle -R w`, frames via `ffmpeg`) for
   timings: hover delay, caret blink, animations, scroll speed.
6. **Fleet's public documentation** (jetbrains.com/help/fleet) for
   behaviour that only shows behind a dialog or a long interaction.
7. Memory: never. If a value has no source above, it is marked "estimate"
   in `fleet-probe.md` until it has one.

## Machine constraints and synthetic input

Two machines run the probes. On **Windows 11** (3840 × 2160 at 150 %,
Fleet unpacked from the Windows zip) everything is in one script,
`tools/fleet-probe/winprobe.py`, and needs no setup beyond Python with
Pillow: it finds the editor's window by process id, sets the *client* area
to the scene's logical size at the window's DPI, sends keys, text and
pointer events through `SendInput`, and grabs the client area with
`ImageGrab`. `winprobe.py run <scene> fleet|oak <fixture> <out>` is a whole
probe; `winprobe.py attach <scene> fleet <pid> <out>` plays the scene into a
Fleet window the user already has open (Fleet is a single instance, so a
second launch only hands the folder to the first). Fleet on Windows draws its own caption into the toolbar, so the
client area is the full window and the shots of both editors start at the
toolbar's top edge. Two things to know: Fleet's Trust dialog is dismissed
by Escape (Smart Mode stays off), and Oak must be built with
`CARGO_TARGET_DIR` on a local disk when the repo is on a network share
(`OAK_EXE` tells the driver where the binary is).

On **KDE Plasma 6.6 on Wayland**, NVIDIA, no `xdotool`, `ydotool` or `wtype`
installed, three routes to drive the windows exist (checked 2026-11-01):

- **KWin scripting over D-Bus** works now from the sandbox: `qdbus
  org.kde.KWin /Scripting loadScript <file.js>` then `run`, and the script
  can list windows (`workspace.windowList()`, Fleet is `resourceClass
  "jetbrains-fleet"`), set `frameGeometry`, activate and close them. Output
  via `print()` lands in `journalctl --user`. This replaces cropping and
  gives both editors the same window size and position.
- **ydotool** (uinput): `/dev/uinput` exists and the module is loaded, but
  the user has no access. NixOS needs `programs.ydotool.enable = true;`,
  the user in the `ydotool` group, a rebuild and a re-login. Then
  `ydotool key`, `ydotool mousemove -a`, `ydotool click` drive any
  compositor with no dialogs. Recommended route.
- **xdg-desktop-portal RemoteDesktop** (KDE backend, version 2, keyboard
  and pointer and touch) is available without any system change. It needs
  a D-Bus client that handles the request signals (no Python D-Bus module
  is installed; a small Rust tool on `ashpd` would do), shows one
  permission dialog that a restore token then skips, and absolute pointer
  motion also needs a ScreenCast stream. Fallback if the rebuild is
  unwanted.

Other constraints:

- Screenshots: `spectacle -b -n -a -e -S -o out.png` captures the active
  window without decoration or shadow (to verify on the first run; the M0
  batch used full-screen shots and cropped). `-d <ms>` delays.
- Fleet runs at the user's zoom 2.0 on a 1.38-scale display, so 2.76 device
  px per logical px. Oak must run at the same zoom for pixel comparison
  (M1 adds `--zoom`).
- The shell sandbox cannot see GUI processes; windows are closed through
  the KWin script (`window.closeWindow()`) or the editor's own close chord.
- Until synthetic input is set up, interaction probes are semi-manual: the
  harness prints the step, the user performs it, the harness shoots on a
  delay. Roughly 70 such steps across the twelve batches, most of them in
  the Settings tool (batch 8) and editor states (batch 5); batch 1 needs
  none.

## Harnesses

Each is a script under `tools/fleet-probe/` unless noted. Build them in this
order; A–D unblock M1, E and F are an afternoon each and pay off at once.
Status (2026-11-02): A, B, D and H exist for Windows (`make-fixture.py`,
`winprobe.py`, `compare.py`, with scenes `editor`, `light`, `panels`,
`splitters`, `tabs`, `settings-live`, `splits`, `surface`, `surface2`, and
the M2 scenes `find`, `fold` and `wrap` and the M3 scenes `goto` and `popups`
shot on both editors on 2026-11-02, their `compare.py` pass still to run); C's in-app probe
mode, E, F's lint and the KDE launchers are still to do. B's settings override is the scene's
`fleet_settings` object (written to Fleet's `settings.json` for the run and
restored after); an Oak launch gets a fresh `OAK_CONFIG_DIR` so no layout or
setting carries over between runs, and the `oak-settings` step rewrites that
file mid-scene for live-reload probes.

**The driver only ever runs in the foreground.** It sends keys and clicks
with `SendInput`, which go to whatever window is in front; it refuses to send
when the launched editor does not hold the foreground, but a run started in
the background still types into the terminal that started it before the
guard can tell. Start a run, let it take the screen, and read the shots after.

### A. Fixture workspace: `make-fixture.py <dir>`

A generated, deterministic folder both editors open, so the same names and
content sit on both sides of a comparison.

- A root with a long name (truncation), a single-child chain
  (`outer/middle/inner/` for compact folders), three levels of nesting, a
  dotfile (hidden-files toggle), one file per supported language, a 1000-line
  file (scrollbar, line-number width), a 300-column line (horizontal
  scroll, soft wrap), a file with tabs, CRLF and non-ASCII, an empty file, a
  2 MB file (big-file threshold), a `.editorconfig`, a `.fleet/run.json`.
- `git init` with one commit, then a modified, an added, a renamed and an
  untracked file, so git colours and gutter marks have something to show.
- Generated, not committed (the `.git` and the big file would be noise).

### B. Fleet launcher: `probe-fleet.sh <scene> [zoom] [theme]` (Windows: `winprobe.py run <scene> fleet`)

Backs up `~/.config/JetBrains/Fleet/settings.json`, writes the wanted
`zoomLevel` and `theme`, launches
`/nix/store/*-jetbrains-fleet-*/bin/jetbrains-fleet <fixture>`, prints the
scene's manual steps (which file to open, which panel to show), then
captures `out/<scene>-fleet-z<zoom>-<theme>.png` after a delay and restores
settings. The fixture lives under the trusted root `~/bstore2/raga/repos`
so the Smart Mode dialog does not appear.

### C. Oak probe mode: `oak --probe <scene.json>` (in the app)

Oak does not need screenshot code of its own; it needs to reach the same
state as Fleet without a mouse. Add to `crates/oak/src/main.rs`:

- `--window-size 1280x800`, `--zoom 2.0`, `--theme` (exists).
- `--probe <scene.json>`: after the first frame, dispatch the listed
  actions and arguments in order (`open fixture/src/main.rs`,
  `goto/line 42`, `toggle-bottom-panel`, `select 10:4-12:9`, `focus files`),
  then stay open for the external screenshot. Actions are the Fleet ids
  already mapped in `keymap.rs`, so a scene file reads the same for both
  editors. Optional `--exit-after <ms>` for unattended runs.
- `probe-oak.sh <scene>` wraps it: `nix-shell --run 'cargo run --release -p
  oak -- --probe scenes/<scene>.json <fixture>'`, then the same `spectacle`
  call as B.

### D. Comparison: `compare.py <scene> <fleet.png> <oak.png>` (built)

- A scene file (`scenes/<name>.json`) lists the driver steps per editor
  (`steps.fleet`, `steps.oak`, in `winprobe.py`'s vocabulary), extra
  launch arguments (`oak_args`, with `{workspace}` expanded), and **scan
  lines**: columns and rows to run-length encode, each with a name and the
  edges expected along it (toolbar bottom, island top, first tab, status
  bar top). This generalises `rle.py`.
- Output: a table per scan line (edge name, Fleet px, Oak px, both in
  logical px, delta), a side-by-side image with the scan lines drawn
  (`--diff`), and a non-zero exit if any delta exceeds the tolerance (1
  logical px by default). `--markdown` prints rows in the format of
  `fleet-probe.md`'s table so a passing probe can be pasted into the record.
  Runs shorter than `--min-run` (2 logical px) are skipped, so glyph stems
  do not count as edges; scan lines should still be placed through fills
  and borders rather than through text.
- Colour checks: for named sample points, read the pixel on both sides and
  the theme key it should equal; this catches a wash or a border drawn from
  the wrong key.

### E. Keymap oracle: test in `oak-keymap`

Resolve `fleet.json` for `linux` with `oak-keymap`, load
`~/.config/JetBrains/Fleet/localStorage/keymap.0.json`, and assert the
trigger → chord table matches (translating Fleet's `keyCode` spelling to
gpui's). Ignored when the file is absent so CI stays green; run locally it
proves platform resolution, chord precedence and key naming against Fleet's
own output. Also yields the key-code vocabulary for `oak-keymap`'s
keystroke spelling.

### F. Coverage and lint

- `coverage.py` (exists): actions, colour keys, text attributes and settings
  keys Oak references versus Fleet's lists; `-v` prints what is missing.
- `lint-colors.sh`: fails if `crates/oak/src` contains a literal colour
  (`rgb(0x`, `rgba(0x`, `hsla(`, `#` + hex) outside `theme.rs`'s fallback
  table, so every painted colour traces to a Fleet key.
- `icon-map.json`: Fleet icon name → Oak asset path, maintained by hand;
  a check lists Fleet icons the roadmap's current milestone needs that have
  no mapping.

### H. Input driver: `winprobe.py` (Windows, built); `drive.sh` and `kwin.js` (KDE)

On Windows one script does the window placement and the input, with the
same step vocabulary the scene files use (`key`, `type`, `move`, `click`,
`drag`, `hover`, `wait`, `shot`, `pixel`), one command per step from a
shell or the whole scene with `run`. On KDE:

`kwin.js` helpers loaded through `qdbus`: find the Fleet or Oak window by
resource class, move it to a fixed position and size (1280 × 800 logical),
activate it, close it. `drive.sh` wraps `ydotool` with a tiny step
vocabulary shared with scene files: `key ctrl+shift+f`, `type "query"`,
`move x y` (window-relative logical coordinates, converted with the scale
read from KWin), `click`, `drag x1 y1 x2 y2`, `hover x y ms`, `wait ms`,
`shot name`, plus `mousedown` / `mouseup` for a shot mid-drag and `maximize`
/ `restore`. With this every batch below runs unattended; a scene becomes
one JSON file executed against Fleet (through the driver) and against Oak
(through `--probe`, or the same driver for mouse-only states).

### G. Timing probes: `record-fleet.sh <scene>`

`spectacle -R w` records the Fleet window; `ffmpeg -vf
"select=gt(scene\,0.001),showinfo"` lists frames where pixels changed, and
the gaps between them give hover delay, blink period and animation length.
Semi-manual like B. Used for the few values the Settings tool does not
show.

## Probe batches, in roadmap order

Each batch produces rows in `fleet-probe.md`, constants in
`theme.rs::metrics`, and a scene file. Values marked "estimate" in
`fleet-probe.md` are the first targets.

| # | Batch | For | What to measure |
| --- | --- | --- | --- |
| 1 | Zoom and light | M1 | Repeat the M0 table at zoom 1.0 to confirm linear scaling; light theme shot of the same scene; window at 1280 × 800 and maximised. |
| 2 | Panels | M1 | Default left panel width (currently 300, estimate), splitter width and hit zone, minimum sizes, bottom and right panel default sizes, panel tab row with Files · Search · Git · History and the `+`, tab padding and gap, island shadow and border. |
| 3 | Editor tabs | M1 | Tab padding, icon gap, close-button geometry and hover, modified dot, selected and hover colours, radius, overflow behaviour, preview tab styling, drag feedback. |
| 4 | Tree | M1/M3 | Second and third level indent (16, estimate), chevron box, row radius (4, estimate), hover vs selected vs focused vs unfocused colours, compact folder slash, file icon size and gap, git status colours per state, hidden files. |
| 5 | Editor surface | M2 | Caret width and blink, selection corners, occurrence highlight, bracket match box, find bar height and field geometry, toggle buttons, soft-wrap indicator, indent guides, folding gutter and markers, line-status marks, scrollbar marks, sticky lines, whitespace glyphs, `editor.guides` line. |
| 6 | Goto panel | M3 | Size, position, field, list item height, icon column, shortcut hints, preview split, empty state. |
| 7 | Popups | M3 | Context menu item height, padding, separators, shortcut column, submenu arrow; main menu; tooltip padding and delay; notifications, toasts, banners; confirm dialog layout and buttons. |
| 8 | Settings tool | M3 | Page layout, search field, section headers, `tileButton`, `checkbox`, `combobox`, `dropdown`, every `button` variant and state, `pillButton`, `link`; and read every default value off the pages. |
| 9 | Terminal | M5 | Padding, font metrics, caret, tab title, scroll-to-bottom button, selection colour. |
| 10 | Git and diff | M4 | Changes list rows, commit field, history table, blame column, diff colours and gutter, side-by-side split, 3-way view. | Scene: `scenes/git.json` (written 2026-11-02, Fleet side not yet run). |
| 11 | Run and problems | M6 | Run gutter icons, run popup, build output tool, problems widget colours and layout, file status badge states. |
| 12 | Smart Mode | M7 | Completion popup rows and documentation pane, parameter info, quick-fix menu, code vision line, lightning states. |

## What the probes cannot settle

Behaviour that depends on Fleet's backend (semantic colours, completion
ranking), on JetBrains services, or on interactions that need a mouse the
harness cannot drive. For those, the order is: Fleet documentation, then
the Settings tool's description text, then a stated assumption in
`fleet-probe.md`.
