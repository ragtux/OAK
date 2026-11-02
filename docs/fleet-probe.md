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

Still estimated rather than measured after this pass: the radii of panel
tabs, editor tabs and tree rows (6, 6, 4) and the caret width (2). The
second pass below settled the tree indent, the editor tab's padding, the
scrollbar thumb and the left panel's width.

## Second pass: Windows, zoom 1.0, 1.5 device px per logical px (2026-11-01)

Fleet 1.48.261 for Windows (`Fleet-1.48.261/Fleet.exe`, the same build as
the tarball) on Windows 11, a 3840 × 2160 display at 150 %, Fleet's
`zoomLevel` unset (1.0), so **1.5 device px per logical px**: every number
below is device px ÷ 1.5. The harness is `tools/fleet-probe/winprobe.py`
(launch, place the client area at 1280 × 800 logical, drive by `SendInput`,
grab the client area) with `make-fixture.py` for the workspace,
`scenes/editor.json` for the steps and `compare.py` for the scan lines; the
same scene ran unattended against Fleet and against Oak, and the deltas are
`compare.py`'s. Shots live outside the repo in `~/oak-probe/out/`.

What differs from the KDE pass:

- **Fleet draws its own title bar on Windows.** The toolbar is the caption:
  the window buttons (minimise, maximise, close) sit at its right end, each
  46.67 logical (70 device) wide, so the right icon group ends at 1130 and
  the caption area runs 1140–1280. The toolbar also carries the window
  title: the workspace name, truncated with an ellipsis at about 270 px,
  then a branch icon and the branch name (`main`), the whole group centred
  on the window's centre (640), not between the icon groups. Rows 0–1 of
  the client area (2 device px) are the frame's accent line (`#4C4A48`),
  drawn by the system over the toolbar. gpui's Windows window also has a
  client-area title bar and draws no caption buttons, so Oak must draw
  them (M1).
- The island margins at this scale are not symmetric: left 7.3 (11 device),
  right 6.0 (9 device), between islands 8.0 (12), top 36.0 (54), bottom
  28.0 (42). Why the right margin is 2 device px narrower than the left is
  not known; Oak keeps 8 on both sides until a shot at another window size
  says more (batch 2).

Measured along `compare.py`'s scan lines, logical px:

| Element | Fleet | Oak before | Oak after | Constant |
| --- | --- | --- | --- | --- |
| Toolbar bottom / island top | 36.0 | 36.0 | 36.0 | `TOOLBAR_HEIGHT` 36 |
| Logo glyph centre | 16.7 (16 px glyph) | 20.0 | 18.0 (Oak's glyph is 20 px in the same 24 px box) | `TOOLBAR_PAD_X` 4 (was 6) |
| Toolbar button centres (menu, panels) | 50.0, 82.0, 114.0, 146.3 | 52, 84, 116, 148 | 50, 82, 114, 146 | `TOOLBAR_BUTTON` 28 + `TOOLBAR_GAP` 4 |
| Last panel button centre to separator | 22.7 | 24.3 | 22.3 | `TOOLBAR_SEPARATOR_MARGIN` 4 (was 6) |
| Tools grid, collaborator centres | 191.7, 224.0 | 196.7, 229.0 | 190.7, 223.0 | same |
| Left island width | 360.0 (7.3–368.0) | 300.0 | 360.0 | `LEFT_PANEL_WIDTH` 360 (was 300) |
| Files tab (selected) top, bottom at x=18.67 | 43.3, 70.0 | 43.3, 70.0 | same | `PANEL_TAB_HEIGHT` 28 |
| Files tab left edge, right edge at y=56 | 14.7, 56.0 | 14.7, 57.3 | same | `PANEL_TAB_PAD_X` 6 (text width differs by 1.3) |
| Tree row pitch | 26.0 | 26.0 | 26.0 | `TREE_ROW_HEIGHT` 26 |
| Selected tree row background height | 24.0 (336.0–360.0) | 26.0 | 24.0 | `TREE_ROW_INSET` 1 |
| Selected row left, right inside a 7.3–368 island | 14.0, 362.0 | 14.0, 302.0 | 14.0, 362.0 | `PANEL_PAD_X` 6 |
| First-level chevron centre, text start | 27.3, 40.0 | 26.3, 40.0 | same | `TREE_PAD_LEFT` 5 |
| Second-level chevron centre, text start | 43.3, 56.0 | – | – | `TREE_INDENT` 16 (confirmed, was an estimate) |
| Root row text start | 28.0 | 28.0 | 28.0 | `TREE_ROOT_PAD_LEFT` 14 |
| Editor tab top, bottom (border outer) | 42.0, 70.0 | 42.0, 70.0 | same | `EDITOR_TAB_HEIGHT` 28 in a 40 band |
| Editor tab border | 1.33 (2 device px) | 0.67 (1 device px) | 0.67 | not yet: gpui `border_1` is 1 device px at 1.5 |
| Editor tab width for `main.rs` | 91.3 (383.3–474.7) | 106.7 | 91.3 (382.7–474.0) | `EDITOR_TAB_PAD_LEFT` 4, `EDITOR_TAB_ICON_GAP` 5, `EDITOR_TAB_CLOSE_GAP` 2, `EDITOR_TAB_PAD_RIGHT` 2 (was 8 / 6 / 6 / 8) |
| Tab icon glyph, title, close glyph (from tab left) | 5.3–19.3, 25.3–72.7, 77.3–84.7 | 12–20, 30.7–74.7, 86–94 | 8–16, 25.3–69.3, 76.7–84.7 | title is 3 px wider in Fleet: its label is a heavier weight; Oak's icon glyph is 8 px wide against Fleet's 14 |
| Editor line pitch (current-line wash) | 22.0 (76.0–98.0) | 20.7 | 22.0 | `EDITOR_LINE_HEIGHT` 22 (was 21) |
| Line-number right edge, text start (from island left) | 35.3, 62.0 | 35.3, 61.3 | same | `GUTTER_NUMBER_RIGHT` 36, `GUTTER_WIDTH` 60 |
| Scrollbar thumb: width, right edge, top | 6.0, flush at 1274, 76.0 | 6.0, 2 px in, 76.0 | 6.0, flush, 76.0 | `SCROLLBAR_THUMB` 6, no inset |
| Scrollbar thumb length (38-line file) | 523.3 | 334.7 | 334.7 | not yet: Fleet's scroll range past the end is shorter than Oak's page |
| Island bottom / status bar height | 772.0 / 28.0 | 773.3 / 26.7 | 772.0 / 28.0 | `STATUS_BAR_HEIGHT` 28 (was 27) |
| Status bar left text start | 15.3 | 16.7 | 16.7 | `STATUS_PAD_X` 8 |
| Status bar right text end | 1268.0 | 1263.3 | 1268.0 | `STATUS_PAD_RIGHT` 4 |
| Status bar gaps between right items | 14.7, 11.3, 11.4 | 24 | 12 | `STATUS_GAP` 12 (was 24) |
| Current line wash colour | `#152639` | `#162639` | `#162639` | alpha blend rounds differently by 1 |

Colours sampled at the same points (`compare.py` samples) match exactly:
window background `#090909`, island `#18191B`, selected panel tab
`#28292B`, selected tree row `#363739`, selected tab fill `#142F4D` with
border `#0E4F98`, scrollbar thumb `#363739`.

Behaviour the shots showed that Oak does not do yet (all M1–M3 items,
listed so the next pass can check them off):

- Opening a file selects it in the Files tree (`files.selectOpenedFile`),
  and the root row loses its selection; Oak keeps the root row selected.
- The status bar reads, left: the path relative to the root with the
  symbol under the caret appended (`src / main.rs / Record`), no root
  name; right: `LF`, then `1:1`, then `UTF-8`, then `Rust`. Oak prints the
  root name first and puts `1:1` before `LF`.
- The root row's label is clipped at the island's edge with no ellipsis;
  Oak clips at the row's inset.
- A green "no problems" check sits in the editor island's top-right corner.
- File icons are per type (Rust gear, Markdown, git, EditorConfig, plain
  text lines); Oak's placeholders are letters.
- The Trust dialog: Escape leaves Smart Mode off, which is the state every
  probe wants.

## Third pass: M1 batches 1–3 (2026-11-02, Windows, 1.5 device px per logical px)

Scenes `light`, `panels`, `splitters` and `tabs` (`tools/fleet-probe/scenes/`)
ran unattended against Fleet 1.48.261 with `winprobe.py run`; the numbers
below are device px ÷ 1.5 read off the shots with the run-length and
ink-projection snippets, the colours are the shots' pixels checked against
the resolved theme files. "Estimate" marks a value the shots do not pin down.

### Batch 1: light theme, caption, maximised

- **Light theme** (`fleet_settings` `{"theme": "light"}`): window `#EEEFF0`
  (`background.primary`), islands `#FFFFFF` (`island.background`), current
  line `#E6F0FC`, scrollbar thumb `#D1D1D1`. The island edges carry **no
  border and no shadow** in either theme: the fill meets the window colour
  directly (one anti-aliased device px). `shadow.*` is for popups only.
- **Window caption (Windows).** Rows 0–1.33 are the system's accent line
  (`#4C4A48`), drawn over the toolbar in both themes and absent when
  maximised. Three caption buttons, each 46.67 wide, fill the toolbar's
  height (0–36): minimise 1139.3–1186, maximise 1186–1232.7, close
  1232.7–1280. Glyphs are 10 × 10 centred in the button (minimise is a 10 ×
  1.33 line), drawn in `icon.primary`. Hover: close `#C42B1C` with a white
  glyph (Windows' own colour, not a theme key); minimise and maximise
  `#E5E6E7` on the light toolbar, which is `ghostButton.off.background.hovered`
  (`DarkTint_6`). The right toolbar group therefore ends at 1130 and the
  toolbar's own buttons keep their 28 px size.
- **Window title in the toolbar.** Workspace name (`text.primary`, 13 px)
  truncated with an ellipsis at 270 px, 16 px gap, a 16 px branch icon, 8
  px gap, the branch name; the whole group centred on the window's centre
  (471.3–808 around 640), not between the icon groups.
- **Maximised** (2560 × 1392 logical): nothing rescales; the left island
  stays 360, the right margin 6, the toolbar 36, the status bar 26 of which
  the bottom 2 are under the screen edge.

### Batch 2: panels

| Element | Fleet | Constant |
| --- | --- | --- |
| Bottom island (window 800 tall) | 472–772, **300** tall | `BOTTOM_PANEL_HEIGHT` 300 (was 220) |
| Right island | 914–1274, **360** wide | `RIGHT_PANEL_WIDTH` 360 |
| Gap between islands | 8 (368–376, 464–472, 906–914) | `ISLAND_MARGIN` 8 |
| Splitter hit zone | the 8 px gap itself (a drag from x=364 or y=540 did nothing, from 372 and 468 it resized); no hover highlight within 800 ms | `SPLITTER_WIDTH` = gap |
| Left panel minimum | 200 (reopened at 200 after a drag to 0) | `PANEL_MIN_WIDTH` 200 |
| Dragging past the minimum | collapses the panel (left to 0, bottom to the window bottom, right to the window edge all hid it) | |
| Bottom panel after a drag to y=250 | island 254–772 (518 tall) | the gap follows the pointer |
| Narrow left panel (200) tab row | `Files Search Git ⁞ +`: tabs clip, an overflow glyph, `+` stays visible | estimate: overflow glyph is `expui/general/moreVertical` |

Empty panel (bottom and right open with no tool): a 24 px panel icon, 20
px gap, the title ("Bottom Panel", 16 px semibold, `text.primary`, line
20), 6 px gap, the description ("Open new tool or drag-n-drop tool from
other panels", 13 px `text.secondary`, line 16, wrapping), 10 px gap, a
secondary button "Open Tool ˅" 24 tall × 92 wide (`button.secondary.*`,
`#2C2D2F` fill, text 13 px, estimate: padding 10, chevron 16 with a 4 px
gap, radius 4). The group is vertically centred in the island and
left-aligned inside a box inset **15 % of the island's width** on each
side (54 in 360, 80 in 530, 134 in 898, 158.7 in 1058). When the panel has
just opened its button carries the keyboard focus: a 2 px `focusOutline`
ring (`#224271`) outside a 1 px `border.focused` (`#2A7DEB`) border.

The `+` tool chooser (`panels-fleet-plus-popup`): a popup 166 × 316.7 at
233.3–399.3 × 73.3–390, i.e. its top 2 px below the tab row and its left
edge 6 px left of the `+` button's box; 1 px `popup.border`, `popup.background`
`#252629`, a shadow about 8 px wide and slightly heavier below (estimate:
`shadow.background.medium`, blur 8, offset 0/2). Inside, 6 px padding: a
search field 24 tall (16 px magnifier at x 248, placeholder "Search" at
267.3 in `text.tertiary`), 6 px, a 1 px `popup.separator` inset 6.7 from the
popup edges, 6 px, then 28 px list rows: 16 px icon at x 248 (8 px row
padding), 4 px gap, 13 px label. The list is alphabetical: AI Assistant,
Breakpoints, Debug, Debugger Console, Docker, Empty Diff, Files, Git,
History, Preview, … and scrolls (thumb 4.67 wide, 1.33 inside the border)
because the popup's height is capped at 316.7 (estimate: a fixed cap).

`hide-active-panel` (Shift+Escape) with focus in the Files tree hid the
left island; the editor island then starts at 7.33.

### Batch 3: editor tabs

| Element | Fleet | Constant |
| --- | --- | --- |
| Tab gap | 4 (notes.txt 382–482.7, README.md from 486.7) | `EDITOR_TAB_GAP` 4 |
| Hovered tab | fill `#28292B` = `tab.background.hovered`, no border, 42–70 | |
| Selected, unfocused | fill `#28292B` = `tab.background.selected` (the preview shot, focus in the tree) | |
| Preview tab (`editor.previewTab`, single click in Files) | title in italic, otherwise a normal tab; replaced by the next single-click file | italic |
| Modified tab | title, 2 px, an 8 px box holding a 4 px `tab.text.selectedFocused` dot, then the usual 16 px close box and 2 px pad: the tab grows by 9.3 | `EDITOR_TAB_DOT` 4 in 8 |
| Tooltip on a tab (the file's full path) | appears within 700 ms at the pointer's x (431.3 for a pointer at 430), 2 px below the tab row; `tooltip.background` `#323438`, no border, radius ~4, padding 8, 13 px text on an 18 px line, wraps at 616 wide | estimate: 500 ms, max width 600 |
| Overflow | the row scrolls to keep the active tab visible; a dim 8 px `‹` at x 378–384 (box 376–392) marks hidden tabs to the left, the first tab then starts at 396 | estimate: `›` likewise at the right end |
| Drag | the dragged tab follows the pointer as a 32 px tall ghost: the tab with a 2 px `tab.focusOutline.dragAndDrop` (`#2A7DEB`) ring, radius 6, on `tab.background.hovered`; its old place stays empty at the tab's width until the drop | |
| `split-right` | a second editor island of equal width (376–821.3 and 829.3–1274, gap 8) with its own tab row, holding a copy of the current tab, which it selects and focuses | |

Status bar after the probes: `src / main.rs / Record` (path from the root,
then the symbol under the caret) on the left; `LF · 7:12 · UTF-8 · Rust` on
the right. The symbol needs Smart Mode and tree-sitter later; the order is
M1's.

### Oak after this pass

The same scenes ran against Oak (`winprobe.py run <scene> oak`, each launch
in a fresh `OAK_CONFIG_DIR`). `compare.py` on the `editor` and `light`
scenes: every island, tab-row and status-bar edge within 1 px, every colour
sample equal; the remaining differences on the scan lines are glyphs (Oak's
file monograms and logo are narrower than Fleet's icons, so edges inside
text runs differ). Read off the Oak shots with the same ink projection:

| Element | Fleet | Oak | Constant |
| --- | --- | --- | --- |
| Panel tab labels (Files, Search, Git, History, +) | 20–48.7, 66–109.3, 130.7–147.3, 168–212.7, 234–245.3 | 20.7–49.3, 67.3–109.3, 130–147.3, 168–211.3, 228.7–242 | `PANEL_TAB_PAD_X` 9, `PANEL_TAB_SELECTED_PAD_X` 6, `PANEL_TAB_PLUS_GAP` 4 (was one 6 px pad) |
| Selected editor tab box, title, close glyph | 381.3–474.7, 408.7–456, 461.3–468 | 382–475.3, 408.7–454.7, 460.7–467.3 | `EDITOR_TAB_CLOSE_GAP` 1 (was 2); the selected title is semibold; the 1.33 border lies outside the 4 / 2 padding |
| Tree row pitch | 26 | 26 | the row is 26 with the 24 px wash inside it (a margin on a `uniform_list` item was dropped) |
| Tooltip box for a pointer at (430, 56) | 431.3–1043 × 72–122.7 | 430.7–1030 × 72.7–107.3 (one line: Oak's path was shorter) | `TOOLTIP_OFFSET_Y` 16, `TOOLTIP_MAX_WIDTH` 616 |
| `+` chooser | 233.3 × 73.3, 166 wide | 230 × 70, 166 wide | `POPUP_OFFSET_X` 8 (the popup's left edge on the glyph's), `POPUP_OFFSET_Y` 2 |
| Caption buttons, hover | 46.67 each, close `#C42B1C` with a white glyph | same | `CAPTION_BUTTON_WIDTH`, `CAPTION_GLYPH` |
| Bottom island (window 800) | 472–772 under the editor only | same | `BOTTOM_PANEL_HEIGHT` 300; the bottom panel sits between the side panels |
| Right island | 914–1274 | 914–1274 | `RIGHT_PANEL_WIDTH` 360 |
| Empty panel group | icon 24, title 16/20, text 13/16, button 24 × 92.7, inset 15 % | same | `EMPTY_PANEL_*`, `BUTTON_PAD_X` 6, `BUTTON_ICON_GAP` 4, `BUTTON_CHEVRON_BOX` 12 |
| Scrollbar thumb right edge | 1274 | 1272 | not yet: Fleet's right window margin is 6, Oak keeps 8 on both sides |

Oak's side of the `tabs` scene differs in its steps (no Goto until M3):
three files come from the command line and the preview tab is clicked last,
so it ends the row rather than starting it, and a double-click's first click
replaces the preview tab, so the overflow shot has no preview tab. The
`settings-live` scene (Oak only) changed `theme`, `zoomLevel` and
`editor.fontSize` on disk while Oak ran: the window followed within a second
(toolbar 36 → 54 at zoom 1.5, line 22 → 27 at 16 px), and `zoom-out` wrote
`zoomLevel` back to the file. Fleet's zoom step is still unmeasured: Oak
uses 0.1.

Still different after this pass, for the next one: Fleet's file-type icons
are filled drawings where Oak paints monograms; Fleet's logo glyph is 16 px
in its 24 px box where Oak's is 20; the green "no problems" badge (M6); the
overflow glyph a narrow panel's tab row shows before its `+`; the right
window margin of 6; the panel tab padding model (which tab is selected
changes the row by about 1 px); and the tab tooltip's delay, which gpui
fixes at 500 ms.

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
in the tree), the light theme's rendering, and anything behind a dialog.
The Windows pass answered the zoom 1.0 question and gave the harness that
runs a scene against both editors; the rest is a scene file each (batches
2–12 in `docs/probe-plan.md`).
