# What was read off Fleet, and how

Oak's shell is written to measurements of JetBrains Fleet 1.48.261 on this
machine (KDE Plasma on Wayland, NVIDIA RTX 3060), taken on 2026-11-01. This
note records where each number and colour came from, so the next pass can
redo it against a newer Fleet instead of trusting this one.

## Sources

Fleet is closed source, but its distribution is a plain tarball
(`Fleet-1.48.261.tar.gz`, also unpacked under the Nix store as
`jetbrains-fleet`) and most of what the shell needs is in it as data:

| What | Where in the distribution | How Oak uses it |
| --- | --- | --- |
| Themes: `dark`, `dark_purple`, `light` | `fleet/themes/*.json` in `lib/app/code-cache/*/fleet.themes-1.48.261.jar` | the file format is Oak's theme format (`oak-theme`); the values of the keys Oak paints from are the bundled themes in `crates/oak/assets/themes` |
| Default keymap | `fleet/keymap/fleet.json` in `fleet.plugins.keymap.frontend-1.48.261.jar` | the file format is Oak's keymap format (`oak-keymap`); the chords for the actions Oak has are `crates/oak/assets/keymap/default.json` |
| Fonts | `lib/app/fonts/Inter`, `lib/app/fonts/JetBrainsMono` (both SIL OFL) | bundled as `crates/oak/assets/fonts` (Inter's static faces come from the Inter release, JetBrains Mono from the Fleet tarball with its OFL.txt) |
| Icons | 934 SVGs under `frontend/icons/{dark,light}/...` in `fleet.frontend.resources-1.48.261.jar` | **not copied**: their names and geometry (16 px, 1 px strokes, `#E0E1E4` on dark) told which open-source set to use; Oak draws from JetBrains' Apache-licensed `expui` set plus its own drawings |
| The user's settings | `~/.config/JetBrains/Fleet/settings.json` (`"theme"`, `"zoomLevel"`) | Oak defaults to the theme named there, so it opens looking like the user's Fleet |

The theme file has four tables: `meta`, `palette` (named colours), `colors`
(about 684 UI keys such as `tab.background.selectedFocused`, each a hex
colour or a palette name, `#RRGGBBAA` with alpha last) and
`textAttributes` (157 editor and syntax keys such as `keyword`,
`editor.selection.focused`, `problem.error`, each an object with
`foregroundColor`, `backgroundColor`, `fontWeight`, `fontStyle`,
`textDecoration`). `tools/fleet-probe/extract-themes.py` pulls them out and
resolves the palette.

## Layout, measured from screenshots

Fleet was started on a scratch workspace (`jetbrains-fleet <dir>`), the
screen captured with `spectacle -b -n -f`, the window cropped with
ImageMagick and its pixel rows and columns run-length encoded with
`tools/fleet-probe/rle.py`. The shot is at **2.76 device pixels per logical
pixel** (the display's 1.38 scale times Fleet's `zoomLevel` 2.0 from the
user's settings), established from the window's logged logical size
(1280 × 800, `fleet.frontend.ui.root.RootView` in `fleet.log`) against its
rendered width and confirmed by the 14-unit hamburger icon rendering 38 px
wide and Inter's x-height at 13 px rendering 20 px.

| Element | Measured (device px) | Logical | Oak constant |
| --- | --- | --- | --- |
| Toolbar (window content top to island top) | 98 | 35.5 | `TOOLBAR_HEIGHT` 36, content centred |
| Toolbar icon buttons, centre to centre | 88 | 32 | `TOOLBAR_BUTTON` 28 + `TOOLBAR_GAP` 4 |
| Toolbar icons | 14-unit glyph at 38 px | 16 px icon | `ICON_SIZE` 16 |
| Window edge to island, island to island | 22, 24 | 8, 8.5 | `ISLAND_MARGIN` 8 |
| Island corner | ~22 | ~8 | `ISLAND_RADIUS` 8 |
| Panel tab ("Files"), height incl. 1 px border | 78 | 28.3 | `PANEL_TAB_HEIGHT` 28 |
| Island top to panel tab top | 18 | 6.5 | `PANEL_HEADER_PAD_TOP` 6 |
| Panel tab bottom to first tree row | 14 | 5 | `PANEL_HEADER_PAD_BOTTOM` 5 |
| Tree row pitch (glyph top to glyph top) | 72 | 26.1 | `TREE_ROW_HEIGHT` 26 |
| Selected root row background | 68 | 24.6 | painted full row height (estimate) |
| Root row text from island edge | 56 | 20 | `TREE_ROOT_PAD_LEFT` 14 inside a 6 px inset |
| First-level chevron box from island edge | ~30 | ~11 | `TREE_PAD_LEFT` 5 + 6 px inset |
| First-level text from island edge | 88 | 32 | chevron 16 + `TREE_ICON_GAP` 5 |
| Editor tab bar (island top to current line) | 108 | 39 | `EDITOR_TAB_BAR_HEIGHT` 40 |
| Editor tab height incl. border | 80 | 28.6 | `EDITOR_TAB_HEIGHT` 28 |
| Editor line height (current-line wash) | 60 | 21.4 | `EDITOR_LINE_HEIGHT` 21 (13 px × 1.6) |
| Line numbers, right edge from island edge | 102 | 37 | `GUTTER_NUMBER_RIGHT` 36 |
| Text start from island edge | 170 | 61 | `GUTTER_WIDTH` 60 |
| Island bottom to window bottom (status bar) | 76 | 27.5 | `STATUS_BAR_HEIGHT` 27, text centred |
| UI font | Inter, x-height 20 px | 13 px | `UI_FONT_SIZE` 13 |
| Editor font | JetBrains Mono | 13 px | `EDITOR_FONT_SIZE` 13 |

Still estimated rather than measured: the indent of a second-level tree
row (`TREE_INDENT` 16), the radii of panel tabs, editor tabs and tree rows
(6, 6, 4), the editor tab's horizontal padding and gap (8, 4), the
scrollbar's thumb width (6 in a 10 px track), the caret width (2), and the
left panel's default width (300; the probed window had 354 at 631 wide).

## What the shots showed

- On KDE, Fleet keeps the desktop's own title bar; the first row Fleet draws
  is the toolbar. Oak does the same (server-side decorations).
- Toolbar, left to right: logo (24 px), menu, panel-left, panel-bottom,
  panel-right, a 1 px separator, the tools grid, "add collaborator", the
  workspace name centred, then smart mode (lightning), run, search,
  notifications, settings.
- The Files island: a tab row "Files · Search · Git · History · +", a root
  row with the folder's name on a selected-row background, then rows with a
  chevron for folders and a file-type icon for files. A folder holding one
  folder shows as `outer / workflows` with the slash in the tertiary text
  colour (Fleet's compact folders).
- The editor island: tabs with a file-type icon, the name and a close
  cross; the selected tab has the `tab.*.selectedFocused` background and
  border. Line numbers right-aligned in a 60 px gutter, the current line
  washed across gutter and text, a 2 px caret, a thin scrollbar at the
  right edge, and a green "no problems" badge in the top-right corner.
- The status bar: the file's path as `a / b / c` breadcrumbs on the left,
  then `line:col`, encoding and language on the right.

## What this does not settle

Interaction details (hover timings, drag thresholds, what double-click does
in the tree), the light theme's rendering, how Fleet lays the window out
at zoom 1.0, and anything behind a dialog. The probe batch answered the
static questions; the rest wants a running Fleet beside a running Oak.
