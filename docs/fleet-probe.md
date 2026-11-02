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
| Line-number right edge, text start (from island left) | 35.3, 62.0 | 35.3, 61.3 | same | `GUTTER_NUMBER_RIGHT` 36, `GUTTER_WIDTH` 60 (62 since batch 5: the 61.3 was glyph ink, the selection and the boxes put the origin at 62) |
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

## Fourth pass: M2 batch 5, the editor surface (2026-11-02, Windows, 1.5 device px per logical px)

Scenes `surface` and `surface2`. Ctrl+R is `run-and-debug` on Windows (the
replace bar is Ctrl+H), which gave the Run & Debug popup as a bonus for M6.

| Element | Fleet | Constant or key |
| --- | --- | --- |
| Selection | `editor.selection.focused` fill, the full 22 px line, each row its own rounded rectangle (radius about 3.3: the edge rows start 1.3 px in); a row ending exactly at the line's end (Shift+End) ends at the last character; a row the selection runs past gets one character cell more (7.5–8 measured on lines 1 and 3); an empty selected row shows a 3.33 px stub at the text start; the current-line wash stays under it | `SELECTION_RADIUS` 3.33, `SELECTION_EMPTY_LINE_WIDTH` 3.33, the stub is the space advance |
| Current line with a selection | the wash is painted; the selection paints over it | |
| Identifier under the caret | `identifier.underCaret` box on every whole-word occurrence, the caret's own included: the glyph run wide, 20.67 tall (0.67 in from the line's top and bottom), rounded (AA on the end columns, about 2 px) | `OCCURRENCE_INSET` 0.67, `OCCURRENCE_RADIUS` 2 (estimate) |
| When the boxes appear | with the caret put at 7:12 by Goto they were there; after a click at 7:15 none showed within 2 s. Estimate: when the caret touches an identifier and the view has been idle | |
| Bracket match | `editor.brace.match` box on both brackets when the caret is next to one: the character cell wide, the full line tall | |
| Indent guides | one device px of `editor.indentGuide` ink at the text origin of each tab stop before a line's indent (spread over 3 device px: `#1C1D1F #262729 #222325`, together one pixel of the key's alpha), solid although the key says `DASHED`, continuing through blank lines inside a block (line 17), absent between blocks (line 12): a blank line takes the smaller of its neighbours' indents. The caret's block is drawn over in `editor.indentGuide.current` (peak `#454648`): the innermost block that holds the caret's row or that the caret's row opens (caret on line 7 `pub struct Record {`, at its end or inside its text: lines 8–10 are current) | `INDENT_GUIDE_WIDTH` 1 device px; `analysis::current_block` |
| Caret | 2 px, `editor.caret.background` | `CARET_WIDTH` 2 |
| Scrollbar | the thumb is translucent (`scrollbar.thumb.default` is `#FFFFFF21`): the current-line wash shows through it; a 12 × 2 px mark in the caret colour sits at the caret line, at `row × line height × track / content`, where the content includes the 4-line overscroll (line 7 → 178) | `SCROLLBAR_MARK_WIDTH` 12, `SCROLLBAR_MARK_HEIGHT` 2 |
| Find bar (Ctrl+F) | a field 841.3–1258 × 88–116: 416.7 wide, 28 tall, 16 from the island's right edge, 12 below the tab row, 1.33 `inputField.focusBorder.default` inside a 2 px `inputField.focusOutline.default`; a chevron (16 box, glyph 822.7–832) 10 px to its left opens the replace row. Inside, from the left: text at +6 (placeholder "Search (Alt+Up for history)" in `inputField.hint.default`), the match counter ("3/8") right-aligned at 1093, ↑ and ↓ in 16 px boxes at 1101 and 1123, a separator at 1147.3, the toggles Aa, W and .* at 1156.7, 1179.3 and 1203.3 (pitch 22, `toggleButton.*`, no visible border when off), a separator at 1224.7, the close glyph at 1236.7–1247.3, 8 px right pad. Tab from the field focuses ↑ | `FIND_BAR_*` |
| Matches | `editor.search.results` boxes the full line tall on every match; the current match is selected in the editor (unfocused `editor.selection` while the field has focus) and scrolled into view; a 12 × 2 scrollbar mark per match in `editor.search.results.scrollbarMarkColor` | |
| Replace bar (Ctrl+H) | a second field 6 px under the first (122–150), same width, `inputField.border.default`, with "Replace" and "Replace All" text buttons at its right; the chevron turns down | |
| Gutter hover | a chevron for every foldable line in a 16 px box at 416–432 (8 px glyph: `chevronDown` open, `chevronRight` folded) and, on the hovered line, a 10 px circle at 397.3–408 in a dim breakpoint colour; the chevrons go when the pointer leaves the gutter | `FOLD_CHEVRON_X` 40 from the island edge, `BREAKPOINT_HINT_X` 21.3 |
| Folded block | the lines collapse (7 → 12); the head keeps `{` and `}` with a `{…}` mark between: `editor.foldedMark.background`, 3 characters wide, the full line tall, radius about 3, "..." in `editor.foldedMark.text` | |
| Sticky lines | the enclosing block heads of the first visible line, one row each at the top on the editor's background, a 1 device px `border` line under them and a 3 px shadow fading below; the content scrolls under | `STICKY_SHADOW` 3 (estimate) |
| Run & Debug popup (Ctrl+R, M6) | 687 × 182 at (301.3, 64.7), `popup.background`, a title, a 28 px field with the focus ring, a workspace heading, a 24 px `listItem.background.focused` row with Run and Debug buttons, a separator, "Edit Run Configurations..." | |

Not settled by this pass: caret blink timing (harness G), soft wrap, rendered
whitespace and the `editor.guides` ruler, which need a `fleet_settings`
scene each. Fleet's scrollbar thumb is about 2 % shorter than track² /
content (523 against Oak's 536 for 36 + 4 lines in a 694 px track) while its
marks scale by track / content; Oak keeps the simple formula.

Oak after batch 5 (the `surface` scene replayed, read off with the same
scans): the selection rows land on Fleet's to 0.67 px (438–921.3, 438–441.3,
438–680, 438–441.3, 438–446 against 445.3), the brace boxes at 578–586 and
438–446, the `Record` occurrence box at 500.7–547.3 (Fleet 500–547.3), the
guides at 438 as one device px of `#363739` with the caret block's at
`#6B6C6D` (Fleet's ink spread over three device px sums to the same), the
caret-line scrollbar mark at 178–180. The text origin moved from 60 to 62
(`GUTTER_WIDTH`): glyph ink now starts at 438.7 against Fleet's 438–439.3.

## Fifth pass: the rest of M2, built from batch 5 (2026-11-02)

No new Fleet shots: the find bar, the folds and the sticky lines were built
from the batch 5 rows above, and what those rows do not pin down is an
estimate here until the `find`, `fold` and `wrap` scenes run against Fleet.
Oak's side of the three scenes ran on 2026-11-02 (shots in
`~/oak-probe/out/{find,fold,wrap}-oak-*.png`): the find field sits at
88–116 with its right edge 16 from the island's, the `{…}` mark folds lines
7–11 of `main.rs` to one row, the 300-column line wraps at the view's edge
with the ruler at column 80, and `fn main() {` pins to the top once the
view starts inside it.

From batch 5, now constants (`theme.rs::metrics`):

| Element | Fleet (batch 5) | Constant |
| --- | --- | --- |
| Find field | 416.7 × 28, right edge 16 from the island's, top 12 below the tab row | `FIND_BAR_WIDTH` 416.67, `FIND_BAR_HEIGHT` 28, `FIND_BAR_RIGHT` 16, `FIND_BAR_TOP` 12 |
| Focus ring | 1.33 `inputField.focusBorder.default` inside 2 px `inputField.focusOutline.default` | `INPUT_FOCUS_BORDER` 4/3, `FOCUS_OUTLINE` 2 |
| Field interior | text at +6; from the right: pad 8, close box 16, 8, separator, 7, toggles Aa W .* (20 wide, 2 apart, pitch 22), 6, separator, 8, ↓ and ↑ boxes 6 apart, 8, the counter | `FIND_BAR_PAD_LEFT` 6, `FIND_BAR_PAD_RIGHT` 8, `FIND_BAR_CLOSE_GAP` 8, `FIND_BAR_SEPARATOR_GAP_RIGHT` 7, `FIND_BAR_TOGGLE` 20, `FIND_BAR_TOGGLE_GAP` 2, `FIND_BAR_SEPARATOR_GAP_LEFT` 6, `FIND_BAR_ARROW_GAP` 6, `FIND_BAR_COUNTER_GAP` 8 (the gaps are back-solved from the glyph positions 1093, 1101, 1123, 1147.3, 1156.7–1203.3, 1224.7, 1236.7, 1258) |
| Replace chevron | 16 box, glyph 10 left of the field | `FIND_BAR_CHEVRON_GAP` 5.33 (box to field) |
| Replace field | 6 under the find field, `inputField.border.default`, "Replace" and "Replace All" at its right | `FIND_BAR_ROW_GAP` 6 |
| Fold chevron | 16 box at 40 from the island's left, on hover; `chevronDown` open, `chevronRight` folded | `FOLD_CHEVRON_X` 40 |
| Folded mark | `editor.foldedMark.background`, 3 cells wide, radius about 3, "…" in `editor.foldedMark.text` | `FOLDED_MARK_CELLS` 3, `FOLDED_MARK_RADIUS` 3 |
| Sticky lines | the enclosing block heads at the top on the island's background, 1 device px `border` under them, a 3 px shadow fading below | `STICKY_SHADOW` 3 |
| Caret-line scrollbar mark | the caret colour | now `editor.currentLine.scrollbarMark` (the same value in the dark theme) |

Estimates introduced by this pass, the first targets for the three scenes:

- `INPUT_RADIUS` 4 (the field's corners), `FIND_BAR_SEPARATOR_HEIGHT` 16,
  `TEXT_BUTTON_PAD_X` 8, the toggle's corner radius (`BUTTON_RADIUS` 4),
  the find field's typeface (the UI face) and the counter's colour
  (`text.secondary`), the input caret at 1 px.
- Soft wrap: continuation rows start at the text origin (`WRAP_INDENT` 0),
  the wrap is at the last blank before the view's edge (less the scrollbar).
- `editor.guides`: a 1 device px line in `editor.indentGuide` (no key names
  the ruler's colour); `editor.renderWhitespace`: `·` and `→` glyphs in
  `editor.whitespaceIndicator`.
- `editor.lineNumbers: Off` keeps the gutter's width; `Relative` shows the
  caret row's own number.
- The caret blinks on a 500 ms half period (`CARET_BLINK_MS`; harness G);
  the block caret is the cell at half opacity under the glyph, the
  underline caret 2 px at the cell's bottom.
- `editor.centerEditorHorizontally` and `distractionFreeMode` centre a
  120-column editor area (`CENTERED_COLUMNS`).
- The prompt popups (`goto/line`, `new-file`, `files/rename`, line endings,
  paste history) borrow the Run & Debug popup's shape until batch 6:
  600 wide (`PROMPT_WIDTH`), top at 64.67, 24 px rows, a 28 px field.
- Settings defaults not read off Fleet's Settings tool (batch 8), marked in
  `oak-settings`: `editor.autoDetectIndentation` true, `trimTrailingSpacesOnSave`
  false, `autoSave` false and `autoSave.delay` 1000, `localBigFileMinimumThreshold`
  20, `editor.stickyLinesMaxCount` 5, `editor.caretShape` Line,
  `editor.caretSmoothAnimation` false, `editor.ligatures` true,
  `editor.honorCamelHumps` true, `editor.useSelectionForSearch` true.

## Sixth pass: M3 workbench tools, built from Fleet's state files (2026-11-02)

Fleet's help site was retired in December 2025 (every page redirects to a
blog post), so the first build of the Goto panel, the Search tool, the menus
and the dialogs came from the two sources left, Fleet's own data files
(source 1) and its runtime state (source 2), plus the popups batch 5 caught.
The same day Fleet's side of the `goto` and `popups` scenes ran (the last
subsection) and the estimates below gave way to the measured values; they
are kept as the record of what the state files alone could say.

### What Fleet's state says

Fleet's local storage on this machine (`%APPDATA%\JetBrains\Fleet\localStorage`,
per-workspace `frontend/db.1.48.0.json` and `layout-recovery.6.json`) is a
durable snapshot of its entity store, and three entities describe the M3 tools:

| Entity | Fields and values seen | Oak |
| --- | --- | --- |
| `fleet.frontend.ui.gotoPanel.GotoPanelViewState` | `isPreviewHidden` false, `isPreviewHorizontal` false, `splitRatioHorizontal` 0.33, `splitRatioVertical` 0.5, `queryHistoryMap` `{"Goto": ["main.rs"]}` | the preview shows by default, beside the list, the list taking 0.33 of the width (`GOTO_LIST_SHARE`); `gotoPreviewHidden` persisted per workspace in `layout.json` |
| `fleet.frontend.navigation.RecentNavigationEntity` | `title`, `iconKey` (`icons.file-types.rust`), `key` (a file address), `location` (document + caret offsets), `timestamp` (a counter), `reachable` | the recent files, most recent first, lead the empty Everything and Files lists; kept in `layout.json` as `recent` |
| Search tool tab (`viewTypeId` `text-search`) | `query` `{text, caseSensitive, words, regex, order 0, limit 100, rootDirectory null, path "", mask ""}`, `replacement`, `searchMode` `FIND`, `hasCloseButton` false | the same toggles, the file mask, the limit of 100 and a replace mode; `order`, `rootDirectory` and `path` are not exposed yet |

The resolved keymap (`localStorage/keymap.0.json`, 241 entries) gives the
chords' presentable spelling (`Ctrl+Alt+Down`), which the menus and the
Goto hints use (`keymap::presentable`), but no action titles: those are
derived from the ids (`keymap::action_title`), with a hand table for the
ones whose words are not English.

### Batches 6 and 7, measured (2026-11-02, Fleet running beside the probe)

Fleet's side of the `goto` and `popups` scenes ran the same day against the
window the user had open (the driver's `run` starts its own Fleet, which a
running one absorbs; `attach.py` in the session's scratchpad plays a scene's
steps into an existing window instead). The values are device px ÷ 1.5 read
off the shots with run-length scans; the colours are the shots' pixels
resolved against the dark theme. They replaced the estimates above in
`theme.rs::metrics` and the Goto panel was rebuilt to this structure.

**The Goto popup** is one popup with four tab chips, not a list with tabs
beneath the field; its preview sits under the list, not beside it
(`isPreviewHorizontal` false means the split is stacked). Ctrl+Shift+F opens
its Text Search tab, not the Search tool in the panel.

| Element | Fleet | Constant or key |
| --- | --- | --- |
| Popup | 300.67–980.67 × 64.67–743.33 with a preview (680 wide, centred; 678.67 tall in an 800 window); Actions, Tools and Recent Workspaces end 2 px under their last row; `popup.background` (`popup.goto.background` has the same value), 1 px `popup.border` | `GOTO_WIDTH` 680, `GOTO_TOP` 64.67, `GOTO_MAX_HEIGHT` 678.67, `GOTO_BOTTOM_PAD` 2 |
| Tab chips | Goto · Actions · Tools · Text Search at 70–98 (28 tall), the first at x 308 (8 in), text 8 in, 6 between chips; the selected one on `tab.background.selectedFocused` inside `tab.border.selectedFocused`; two 24 px icon buttons at the right end (the preview toggle, open in a window) | `GOTO_TAB_TOP` 5.33, `GOTO_TAB_HEIGHT` 28, `GOTO_TAB_PAD_X` 8, `GOTO_TAB_GAP` 6, `GOTO_ICON_BUTTON` 24 |
| Field | 308.67–972.67 × 108–136 (28 tall, 8 in, 10 under the chips) with the 2 px `inputField.focusOutline.default` ring outside it (106–138); text at +8; no magnifier; "Search files, classes, functions, and symbols" | `GOTO_TAB_FIELD_GAP` 10, `GOTO_FIELD_HEIGHT` 28 |
| Filter buttons (Goto tab) | Files · Symbols · Symbols in File at 144–168 (24 tall), 6 under the ring, 10.67 apart: `button.secondary.*` off, `button.primary.*` on | `GOTO_CHIP_HEIGHT` 24, `GOTO_CHIP_TOP_GAP` 6, `GOTO_CHIP_GAP` 10.67 |
| Section header | "Recent" (empty query), "Found", "Active", "New" in `text.secondary`, a 26 px row 8 under the buttons | `GOTO_HEADER_HEIGHT` 26, `GOTO_BLOCK_GAP` 8 |
| Rows | 26 px pitch, a 24 px wash of `listItem.background.focused` (#194474) on the selected row, 8 px row padding, a 16 px icon then 4 to the text; with nothing between the ring and the list (Actions, Recent Workspaces) the first wash starts 10 under the ring; a file's folder follows its name in `listItem.text.secondary`; a symbol's file, an action's shortcut, a match's `file:line` and a workspace's path are right-aligned in `text.secondary`; Actions and Text Search rows carry no icon | `GOTO_ROW_PITCH` 26, `GOTO_ROW_HEIGHT` 24, `GOTO_LIST_FIELD_GAP` 10 |
| Matched letters | a box of `search.match.background` (#9F680C80) behind them, the text in `search.match.text` | |
| Everything | files and symbols only (Actions and Tools are their own tabs); "Recent" holds the recent files, the first selected | |
| Tools | "Active" (the tools in the panels), 8 px, a 1 px `popup.separator`, 4 px, "New" and every tool with its icon | `GOTO_SEPARATOR_ABOVE` 8, `GOTO_SEPARATOR_BELOW` 4 |
| Preview | a box on `island.background` 8 in from the sides, 462–736.67 under the Goto list (0.465 of the height left under the list's top), 6.67 above the popup's edge, holding the editor and a `src / main.rs / Record` breadcrumb at its bottom; `toggle-goto-preview` (Alt+2) removes it and the list runs to the bottom | `GOTO_PREVIEW_SHARE` 0.465, `GOTO_PREVIEW_BOTTOM_PAD` 6.67 |
| Symbols in File | an outline with chevrons: `Record`, then `id: u32`, `new(id: u32, name: &str) → Self` indented about 24; no header, no preview, the popup still full height | `GOTO_TREE_INDENT` 24 (estimate), `Symbol::signature` |
| Recent Workspaces (Ctrl+Alt+W) | the same popup with a bold "Recent Workspaces" title in the tab row's place, "Search workspaces", rows with the path right-aligned | |
| Text Search (Ctrl+Shift+F) | the fourth tab: the field holds Aa · W · .* and a replace toggle at its right; 6 under the ring an "In folder" field 492 wide with a folder icon and a "File mask: *.*" field 164 wide, 8 apart, 28 tall; a 28 px row "8 results in 1 files"; one 26 px row per match with the line (indent stripped) and `main.rs:5` at the right; the preview below. Ctrl+Shift+H adds a Replace field 8 under the ring and "Replace · Replace All" secondary buttons (24 tall, 8.67 apart) 11.33 above the popup's bottom | `SEARCH_MASK_WIDTH` 164, `SEARCH_COUNT_ROW` 28, `GOTO_BUTTONS_PAD` 11.33, `GOTO_BUTTON_GAP` 8.67 |

**Menus.**

| Element | Fleet | Constant |
| --- | --- | --- |
| Position | the top-left corner at the pointer (a right click at (100, 139) gave a menu at 100.67 × 138.67); the main menu at 36.67 × 36.67, its left edge on the menu button's box, 2 under the toolbar | the menu button's x, `TOOLBAR_HEIGHT` + `POPUP_OFFSET_Y` |
| Rows | 28 tall; labels 14.67 in (6 px popup padding, 8 px row padding); shortcuts in `text.secondary` right-aligned 14.67 from the edge; a `›` in `text.secondary` on a submenu row | `MENU_ROW_HEIGHT` 28, `MENU_ROW_PAD_X` 8, `POPUP_PAD` 6 |
| Separator | a 12 px block: the 1 px `popup.separator` with 5.5 above and below | `MENU_SEPARATOR_PAD` 5.5 |
| Width | the longest row: the tree's menu 230.67, the editor's 232, a tab's 170.67, the main menu 139 | measured from the labels at render; `MENU_MIN_WIDTH` 140 |
| Tree menu | New File..., New Folder..., sep, Find in Files..., Open In ›, sep, Rename... F2, Delete... Shift+Delete, sep, Cut, Copy, Copy Path, Copy Relative Path, Paste, sep, Git › | Oak: the same, with Cut, Copy, Paste and Git greyed (the file clipboard and M4); the seventh pass added Open in Split and dropped Find in Files from a file's menu |
| Tab menu | Close Ctrl+F4, Close Other Tabs, Close All Tabs, Close Saved Tabs, sep, Split Right Ctrl+\, Split Down, Pin, sep, Open In › | Oak: without Split Down and Pin |
| Editor menu | Quick Fixes... Alt+Enter, sep, AI Actions ›, Refactor ›, Go To ›, Reformat Code Alt+Shift+F, sep, Git ›, sep, Cut, Copy, Paste, sep, Plugins › (greyed) | Oak: Go To ›, sep, Cut, Copy, Paste (the rest are M4 and M7) |
| Main menu | File, Edit, View, Goto, Code, Run, Git, Window, Help, every row a submenu | Oak: the nine, Run and Git holding a greyed row |

`compare.py` on the `goto` pairs after the rebuild: the popup's top, the
field, the selected row and the popup's fill within 0.7 px; the preview box
first landed 12.7 px low because its share had been taken of the whole
height under the list's top, where Fleet halves what is left after the 8 px
gap (`splitRatioVertical` 0.5): `GOTO_PREVIEW_SHARE` is now 0.5 of that, which puts Oak's box top at 459.3
against Fleet's 462 (Fleet's own preview is 1.7 short of an exact half).

The M2 scenes `find`, `fold` and `wrap` ran against Fleet the same way. What
they add to the fifth pass, for the M2 follow-up: the find field's edges sit
2 px left of Oak's because Fleet's right window margin is 6 (the M1
leftover); with the field open on a file scrolled to its top and a query
typed, Fleet's text moved down 52 px (line 1 from 87 to 139) while the
empty field and the replace form left it in place, a behaviour Oak does not
have yet; the hovered gutter line carries a red breakpoint hint (#55262F at
398–408.7) and the fold chevron's hover wash reads #2A2C2F where Oak paints
#53565A (`editor.foldIndicator.background.hovered` resolves differently or
Oak draws it over a lighter base); every foldable line shows its chevron
while the gutter is hovered, as Oak's does. The `wrap` scene (run with its
`fleet_settings` through `attach`) confirms `WRAP_INDENT` 0: the
continuation rows of the 300-column line start at the text origin, the
current-line wash covers all three rows of the wrapped line, and the
`editor.guides` ruler stands at column 80 (x 1061.3, Oak 1062.7).

Still estimated after this pass: the preview box's radius (6) and its
breadcrumb line (24); the outline's indent (24); the confirm dialog (twice the
scene's click on a tree row left the keyboard in Fleet's editor, so
Shift+Delete cut a line there instead, undone each time; Fleet's tree takes
the focus on some other event, to be found before the dialog can be shot);
toasts and the Notifications tool, which no probe step triggers; the Search
tool in the panel, which no chord opens. The Fleet shots carried over to Oak
with no change to the bundled themes: every colour read off them resolved to
a key already in use.

## Seventh pass: batch 8, the Settings tool, and the dialogs scene (2026-11-02)

The `settings` and `dialogs` scenes were played into the running Fleet
(`winprobe.py attach`, which now applies a scene's `fleet_settings`) with
the fixture open. Device px ÷ 1.5, read with run-length scans; colours are
the shots' pixels resolved against the dark theme.

### The Settings tool

Fleet opens Settings as an editor tab ("Settings", the gear icon) in the
active split, not as a dialog. Inside the tab a 720 px column is centred in
the island (464.7–1184.7 in a 1280 window with the left panel open), and
everything below lives in it:

| element | measured |
|---|---|
| search field | 28 high (ring rows 114–116 and 144–146, `#224271` = `textField.focusOutline`), top 32 under the tab row, full column width; the magnifier at the left, "Search" placeholder |
| scope row | 18 under the field: a 28 px "Global" chip (57.33 wide, 8.67 padding, white semibold text, filled `#0870E440` = `tab.background.selectedFocused` with the tab's focused border), the workspace name 16 after it in secondary text, "Plugins" as a `link.text` at the column's right |
| separator | 1 px `separator.default`, 8.67 under the scope row |
| section tree | 200 wide at the column's left; first row 30 under the separator; rows 34 with a 32 px `listItem.background.selected` wash (`#363739` in the shot); labels 35.33 in, a chevron at 15.33 for sections with groups; children 16.67 further in. Sections: General, Editor (Font, Appearance, Code, Tabs, Caret, New File), Tools (Gradle, Files, Run and Debug, Git, Terminal, Docker), Application (Appearance, Status Bar, Window), then AI |
| content column | 26 right of the tree, 494.67 wide (690.67–1185.33); the section title 20 px semibold (glyph box 14.67 against Oak's 17.33 at 24) on a 32 line, 24 under the separator; 12 to the card; a 6 px scrollbar at 1179.33–1185.33 once scrolled |
| card | `background.secondary` (`#252629`, `#242528` at the shot's gradient edge), radius 8 (estimate), 9.33 px top padding (first control 17.33 under the card's top; the bottom padding is taken as the same, estimate), 494.67 wide; the next section's title 20.67 under the card |
| row | 40 pitch; the label 26.67 in (`#E0E1E4`); the control 24 high ending at 1145.33 (40 before the card's right edge) and starting 108 after the label column or 10 after a longer label ("Show whitespace" ends 826.67, its dropdown starts 836.67); dropdowns are `dropdown.background`/`dropdown.border` (`#38393B` fill in the shot) ending in a chevron, text and number fields `inputField.*` (transparent, 1 px border); the keymap row has a pencil (`edit-keymap-file`) 10 from the card's right edge |
| group title | 15 px semibold (F stem 10.67 against Oak's 11.33 at 16) on a 32 line, 8.67 under the card's padding or a 16 px spacer after the previous group's last row, 1.33 above its first row (page 3: card top to first control centre 71.33, row centre to title centre 60.67, title centre to next row centre 37.33) |
| checkbox | 16 × 16, `checkbox.on.background` (`#0870E4`) with a white tick or `checkbox.off.*`; the label 8 after; rows 40 pitch plus 17 per description line (page 2: 40, 57.33, 74 for none, one, two lines) |
| modified value | `settings.modified.indicator` (`#CDA261` seen as a 6.67 px mark at the row's left when a value differs from Fleet's default) |
| search | typing filters every page: the tree shows a count badge per section, the Global button shows the total, matches are highlighted with `search.match.*` |

The defaults read off the pages, now in `settings_schema.rs` (one row per
page item, with the key `documentation.yaml` gives it) and corrected in
`oak-settings`: `autoSave` is "On focus change" (true), `trimTrailingSpacesOnSave`
true, `createScratchOnNewFile` true, `editor.caretSmoothAnimation` true,
`editor.honorCamelHumps` false, `files.toggleFolderMode` "Double click",
`editor.lineSpacing` 1.7, `editor.tabLimit` 10, `editor.hoverTooltipDelayMs`
500, `git.fetchInterval` 15, the terminal font JetBrains Mono 13 at 1.7
with the caret "Same as in the editor", `terminal.profile.default` `pwsh`.
Rows whose key the documentation does not name (the AI page, inlay hints,
the scrollbar toggles, shell integration, the collaboration username) are
not in the schema.

The first Oak shot against Fleet's (`compare.py` on the `settings` pair):
the card's controls land within 0.7 px; Oak's column was 8 px high (its
pane body starts under a shorter tab strip, so the field's offset is 40 in
Oak for Fleet's 32), its card padding 16 for Fleet's 9.33, its titles 24
and 16 for Fleet's 20 and 15, and Page Down did not scroll. After the fixes
the field bottom, scope row, separator, tree selection, card edges and the
Editor card's controls pair within 0.7 px; the General card's controls sit
1.3–2 px low (the 9.33 px card padding rounds up at 1.5 device px per px).

Oak's Settings tool (`settings_view.rs`) is the same tab: the column, field,
scope row, tree and cards at these values, dropdowns as a popup list,
checkboxes and number/text fields writing the key to Oak's `settings.json`
at once through `settings::set_value`, "Edit in settings.json" links for the
list-valued keys, and the search with per-section counts and highlights.
Not built: the Plugins page, the modified-value mark (Oak has no record of
which value the user set), the AI page, the workspace scope (Oak writes the
global file only).

### The dialogs scene

- **Notifications** is a popup under the bell, not a panel tool: 318.67
  wide, right edge 6.67 from the window's, top 0.67 under the toolbar
  (y 36.67–173.33 for the empty state), `popup.background` with the
  popup's border and shadow; a 32 px header ("Notifications" 10.67 in,
  two 24 px icon buttons at the right: settings and clear), a 1 px
  separator, then the rows or a 105.33 px "You don't have any unread
  notifications". Oak's `notifications/toggle` now opens this popup;
  the panel's Notifications tool is gone from the right island.
- **Rename** is inline in the tree: the row turns into a text field with
  the name selected up to the extension, the focus ring (`inputField.focusBorder`)
  around icon and field; no dialog. Oak now does the same
  (`FileTree::start_rename`; the prompt stays for a file the tree cannot show).
- **The confirm dialog** (Delete) was not captured: the Delete row in the
  tree's context menu did not open it under the driver. The scene now
  walks the menu with the keyboard; it needs another run.
- **"File has been changed on disk"** appeared when the fixture was
  restored under Fleet with `main.rs` edited: not a `banner.*` band but a
  pill in the popup colours (`#252629`, border, shadow) centred over the
  editor, 374.67 × 34.67 with its top 4.67 under the tab strip; the text 12
  in, 12 to the 24 px `button.primary` buttons Overwrite (77.33 wide) and
  Reload (59.33), 12.67 apart, 11.33 of padding after. Oak polls its open
  files once a second: a clean document follows the disk, an edited one
  shows this pill (`Workspace::render_banner`). Driven by hand (edit, append
  to the file on disk, wait a poll): Oak's pill lands at 638–1011.3 × 81.3–114.7
  against Fleet's 638–1012 × 80.7–115.3, the buttons at 86–110 as Fleet's,
  12.67 apart (the 1 px transparent border of Oak's shared button had made
  each 2.67 wider; the pill drops it).
- **`autoSave`** reads "On focus change" on Fleet's General page, so its
  values are not only true and false; Oak keeps `autoSave: true` as a save
  `autoSave.delay` ms after the last edit until the other values are read
  (the probe writes `autoSave: false` into its config folder when a test
  needs a dirty document to stay dirty).
- **The tree menu** for a file: New File..., New Folder..., sep, Open in
  Split, Open In ›, sep, Rename... F2, Delete... Shift+Delete, sep, Cut,
  Copy, Copy Path, Copy Relative Path, Paste, sep, Git ›. No "Find in
  Files..." (that is the folder menu's) and no hidden-files toggle. Oak's
  menu now matches, with Cut, Copy, Paste and Git greyed until M4 and
  "Open in Split" as Oak's own `open-in-split` (Fleet gives the row no id).

### M2 follow-ups seen in this pass

- Fleet's find bar pushes the text down 52 px only once a query is typed;
  an empty field and the replace form do not move it. Oak now does the
  same (`FIND_BAR_PUSH`, the pane pads the editor while the query is set).
- Fold hover: Fleet draws a plain chevron (no wash box) and fills the
  breakpoint hint as a red circle (`#50252D` over the island, Fleet's red
  at about 28 %; no theme key names it). Oak now draws the circle and no
  wash.
- The editor island's right margin is 6, the other three 8; Oak now uses
  `ISLAND_MARGIN_RIGHT` for the column (the status bar still spans the
  full 8).
- Wrap: `WRAP_INDENT` 0 confirmed; the ruler at column 80 is at x 1061.3
  in Fleet and 1062.7 in Oak.

## Eighth pass: M4 begun from the shots on hand (2026-11-02)

The fixture is a git repository with one file in each state (`README.md`
modified, `src/added.rs` staged new, `src/old.rs` renamed to
`src/renamed.rs`, `notes.txt` untracked), so every Fleet shot of the Files
tree already shows how Fleet colours git status. Read off
`settings-fleet-settings.png` and `tabs-fleet-tabs.png`:

| element | measured |
|---|---|
| added, untracked | the name in `editor.gitDiff.text.added` (`#69B090`) |
| modified, renamed | the name in `editor.gitDiff.text.modified` (`#71A3EF`) |
| unchanged | `#E0E1E4`, selected or not; folders are never coloured (`src` holds the added file and stays white) |
| badge | at the row's right end, centred 14.3 from the row's edge: a `+` (8.67 × 8) for added, a `?` (6 × 9.33) for untracked, both `icon.background.green` (`#169068`); a 6.67 px dot in `icon.background.blue` (`#2A7DEB`) for modified and renamed |
| tabs | titles stay `tab.text.*` whatever the status (`notes.txt`, `README.md` open beside `main.rs`) |
| branch | the toolbar's branch tag is the M1 title, unchanged |

Oak's `oak-git` crate shells out to `git` as Fleet does (`rev-parse
--show-toplevel`, `rev-parse --abbrev-ref HEAD`, `status --porcelain=v1
-z --untracked-files=all`), the tree colours names and draws the badges
from these keys (deleted and conflict rows use the matching
`editor.gitDiff.text.*` and `icon.background.red/yellow`, an estimate until
batch 10 shows them; the badge glyph is a 13 px semibold `+`/`?`, also an
estimate), and the status is read again on open, after a save or a file
operation and every fifth disk poll. Still batch 10's: the gutter's line
status (`editor.showLineStatus`), the Git tool, diffs and blame.

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
