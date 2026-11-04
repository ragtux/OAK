# The tui skin

`"weep.skin": "tui"` (or `--skin tui`) draws WEEP as a terminal program would look, while staying a
GPUI app. This file is the design: the rules every element follows, then each element's tui form.
Fleet stays the default skin and the reference. Everything here sits behind
`Skin::current(cx) == Skin::Tui` and leaves the Fleet branch untouched.

References: Cogmind (a strict grid with deliberate breaks), Dwarf Fortress and Caves of Qud (glyphs
as UI), tmux and vim (status line, command line), Neovide (a grid editor on a GPU).

## Rules

1. **One grid.** A cell is the mono font's advance by `round(editor.fontSize × zoom × spacing)`,
   where spacing is `editor.lineSpacing`, else 1.5 (`skin::Cell::current(cx)`). Everything in the
   window has a position and a size in whole cells: the editor's text, the tree's rows, frames,
   popups, buttons and the status bar. Text is the mono font at `cell.font_size`, and the line
   height is `cell.h`. The UI font is not used.
2. **Lines are geometry.** Box drawing is painted through the centres of cells (`skin::paint_frame`,
   `skin::rule`, `skin::guides`), never set as glyphs, so it joins across rows at any line height. A
   stroke is one device pixel per zoom step (`skin::stroke`), snapped to device pixels
   (`skin::snap`).
3. **Glyphs come from the mono font.** A mark is a char the mono font has (JetBrains Mono: arrows,
   geometric shapes, block elements, box drawing, `◆ ▸ ▾ ● ○ ■ □ ✓ × ⋯ ↵ ⇥ ↑ ↓ ⌕ ±`). A char that
   falls back to another font breaks the grid. Where no char fits (the toolbar's pictograms), paint a
   one- or two-cell drawing in the line colour. No SVG icons in the tui skin.
4. **State is colour and reverse video, not outlines.** Selected: the row's whole cells filled
   (`list_bg_selected`, `list_bg_focused` when focused). Hovered: `list_bg_hovered`. Primary
   buttons, the active segment and toggles that are on: filled cells. Focus moves the island's frame
   to `border_focused`. No rounded fills except the frame corners and the scroll thumb, no shadows,
   no inset/outset padding that isn't a whole cell.
5. **Colours are Fleet's theme tokens.** Every Fleet theme works unchanged: `theme.rs` tokens only,
   never a literal colour.
6. **Motion snaps.** Scrolling, resizing and dragging land on whole cells. The editor scrolls by
   rows and columns, and a split moves by a column.
7. **Boxes.** A popup, menu, dialog, toast or multi-line tooltip is a `skin::boxed`: exact cells,
   the background filling its frame cells as a terminal fills them, a title or tabs set into the
   top edge (`skin::title`), `├──┤` rules where sections meet, and key hints set into the bottom
   edge. A box sits on whole cells of the window grid where it can.
8. **Words for chords.** A shortcut is printed as the keymap spells it (`⌘P`, `Ctrl+P`), in
   `text_tertiary`, right-aligned in its row.

## Shared pieces (`crates/weep/src/skin.rs`)

| piece | what |
|---|---|
| `Cell::current(cx)` | the cell; `cols(n)`, `rows(n)`, `fit(extent, pitch)`, `text_cols(s)` |
| `framed` | an island filling its parent: frame, strip in the top edge, body one cell in |
| `boxed` | a box of exact cells with rules, strip, body and background |
| `paint_frame` | the frame and its rules, for a canvas that already has its bounds |
| `title` | ` text ` set into an edge |
| `rule` | `─` across a row, inside a body |
| `guides` | tree guides `│ ├─ └─` |
| `paint_scrollbar` | a one-cell column: rail and a thumb half a cell wide, in whole rows |
| `button` | `[ Label ]`, a filled block when primary |
| `checkbox`, `radio` | `[x]` `[ ]`, `(•)` `( )` |
| `file_glyph` | a file's glyph and colour by extension |
| `redraw` | another frame, for grids read back while painting |
| `RowScroll` | a list's first row in whole rows: wheel, reveal, reset (rule 6) |
| `FrameBars` | the scroll bars a view's lists set on its island's right frame line, dragged and paged by rows |

## Elements

### Window and islands
- The window is one grid. Islands (panels, editor splits) are `framed`, separated by one empty
  cell, so their frames sit a cell apart and their rows line up across the window. The gap
  column between two islands is the splitter: hovering it draws a `┃` (two strokes) in
  `border_focused`, and dragging moves it by whole columns.
- A panel or split that is focused has its frame in `border_focused`, the rest in `border`.
- Empty editor area (no tab open): the frame with `WEEP` drawn as a 5-row block-element banner in
  `text_tertiary`, and under it the hints as rows: `⌘P  Goto file`, `⌘N  New file` and so on.

### Toolbar (title bar)
- One row of cells, plus the rows the platform's caption needs (macOS traffic lights), with the
  content row centred. No background other than the window's.
- Left: panel toggles as two-cell drawings of the layout (a box with its left, bottom or right
  part filled when that panel shows) and the tool launcher `⠿`.
- Centre: `▣ src  ⎇ tui-skin`, the workspace name and the branch tag (the branch tag is text with
  `⎇`, and its menu is a box).
- Right: the Smart Mode mark (`ϟ`, or a painted bolt if the font lacks it, in `smart_mode_*`
  colours), `▶` run, `⌕` search, the notification mark (`●` with a count when unread) and `≡`
  settings. Each is one cell with a cell of gap, and a hover fills the cell.

### Tabs (the island's top edge)
- Set into the frame's top edge from the third column: ` ◆ name ● ` per tab. Selected and focused:
  filled with `tab_bg_selected_focused` (reverse video); selected: `tab_bg_selected`. Others are
  plain text on the frame line, with one cell of line between tabs.
- `●` dirty; `×` replaces it on hover (one cell, not two: no reserved gap when clean). Preview tabs
  are italic.
- The panel's tool tabs (Files, Search, Git, History, `+`) use the same form.

### Editor
- **Rows on the grid:** line height = `cell.h`, text at `cell.font_size`, so column `n` of a line
  sits at `text_left + n × cell.w`. Vertical scroll by whole rows; horizontal scroll by whole
  columns. The body starts one cell in from the frame.
- **Gutter, in cells, left to right:** run/breakpoint (1 cell: `▶` run icon, `●` breakpoint, `○`
  hint on hover), line numbers right-aligned (the digit count of the last line, at least 3),
  line status (1 cell), fold (1 cell: `▾` open on hover or at a fold head, `▸` folded, always
  shown when folded), then 1 cell of space before the text. Current line number in
  `line_number_current`.
- **Line status (git):** in its cell, a bar a third of a cell wide at the cell's left for added
  (`git_added_bg`) and modified lines, and a `▁`-like wedge at the row's bottom edge for deleted
  lines; hover widens it to the full cell, and the status popup is a box.
- **Current line:** the whole row of cells filled (`current_line`/`current_line_focused`), gutter
  included.
- **Caret:** `editor.caretShape` drawn on cells: bar = a stroke twice the line width at the
  cell's left edge, the full row; block = the cell filled in `editor_caret` with its char redrawn
  in the background colour; underline = the cell's bottom fifth. Blink as Fleet.
- **Selection:** whole cells, no radius; an empty selected line shows one cell; a line's
  selection runs to the end of the text plus one cell when the newline is selected.
- **Occurrences, brace match, search matches:** whole cells filled, no inset, no radius.
- **Indent guides:** a vertical line through the centre of the indent column's first cell,
  `indent_guide`, the current block's in `indent_guide_current`. Visual guides (rulers): a line on
  the column's left edge.
- **Whitespace:** `·` and `→`, as now.
- **Folded block:** `⋯` in one cell filled with `folded_mark_bg`, flanked by the kept text.
- **Problems:** an error's range underlined by a straight stroke at the bottom of its cells
  (errors `problem_error_underline`; warnings dashed, one cell on, half a cell off; weak ones
  dotted); unused code dimmed. The gutter cell left of the text shows nothing extra: the badge
  says it.
- **Problems badge:** set into the frame's top edge at the right: ` ✓ ` on success, ` ✗ 3  ▲ 1 `
  with error and warning colours, a filled cell when hovered. The problems widget opens under it as
  a box with one row per problem: `✗ 12:5  message`.
- **Scrollbar:** the frame's right edge is the rail: the thumb (half a cell wide, whole rows) sits
  on the right frame line, from the body's first row to its last. Scrollbar marks (search,
  problems, changes) are one-cell ticks on the same line in their mark colours.
- **Sticky lines:** the top rows of the body, filled with the editor background, with a `─` rule
  painted along the bottom edge of the last sticky row (no shadow).
- **Soft wrap:** continuation rows show `↪` in the fold cell's column, and the wrap indent is
  whole cells.
- **Blame:** a column of cells left of the gutter: `author 2d` (truncated with `…`, a fixed width
  in cells), in `text_secondary`, the hovered commit's rows filled with `git_blame_bg`.
- **Run icons and status popups, the line-status popup:** boxes.
- **Inlay hints:** in their own cells, in `text_tertiary`, with no pill background.
- **Code vision:** a whole row of cells above the declaration, the lens titles (`4 implementations`)
  in the line's columns from its first non-blank one, a cell between two, in
  `editor.interline.text.default` (`.hovered` under the pointer); the row scrolls like a line.

### Find / replace bar
- A command line at the bottom of the editor's island: a rule across the frame (`├──┤`) and one
  row under it (two with replace): `/ query▏          [Aa] [W] [.*]  3/17  ↑ ↓ ×`. The replace row
  reads `↳ text▏   [ Replace ] [ All ]`. Option toggles are filled when on. No match: the
  counter is `0/0` in `text_dangerous`.

### Goto (and every list popup: usages, declarations, recent files, run popup)
- A box centred under the toolbar on whole cells, ~60% of the window's columns. Its top edge
  holds the tabs (`Goto  Actions  Tools  Text Search`, the active one filled) and the layout
  toggles at its right end as text marks.
- Row 1: the query as an input: `› query▏`. Row 2: the scope chips as text segments `Files
  Symbols  Symbols in File`, the active one filled. Then a rule.
- List rows: `◆ name  dim/path`, the matched chars in `completion_match_text`, the selected row
  filled. A right column with the secondary text or the chord.
- The preview, when on, is a second box region on the right, split from the list by a vertical
  rule joining the frame (`┬ │ ┴`), showing the file's lines with line numbers on the grid.
- The bottom edge carries key hints: ` ↵ open  ⇥ preview  esc close `. Empty: `Nothing found`
  centred in `text_tertiary`.

### Completion, parameter info, documentation
- Completion: a box at the caret's cell, one row below it (above when there's no room), no
  title. Rows: kind glyph (1 cell: `ƒ` function, `◇` struct/type, `▪` field, `·` other), the
  label with matched chars in `completion_match_text`, the detail right-aligned in
  `text_tertiary`. The selected row is filled.
- Documentation: a box beside the completion box (sharing its frame column when they touch),
  the Markdown as plain wrapped lines on the grid.
- Parameter info: a one-row box above the caret, the current parameter in
  `parameter_info_match_text`, bold.

### Inline usages (find usages)
- A box filling the interline block between the caret's line and the next: whole rows of the
  grid (the block's height is re-rounded to the tui row on a skin switch or a zoom), across the
  editor's columns. ` Usages 6 in 1 file ` set into the top edge, ` × ` at its right end.
- The list in the left third, a rule joining the box (`┬ │ ┴`) to the preview on the right
  (the read-only editor on the grid). A file row `▾ ◆ main.rs  src        6`; a usage row
  indented four cells, the match filled with `editor.interline.match.background`, a row too
  long for its cells dropping just enough of its start for the match to fit. The selected row
  filled.
- The bottom edge: ` ↵ open  esc close ` at the left, the path at the right.

### Menus (context, main, branch)
- A box with no title. Rows: `  Label            ⌘C ` with one cell of margin, the shortcut
  right-aligned in `text_tertiary`; disabled rows in `text_disabled`; separators are rules that
  join the frame; a submenu row ends in `▸`; a checked row starts with `✓`. The hovered or
  selected row is filled.

### Dialogs
- A box centred in the window, the title set into the top edge, the message as wrapped lines,
  then a blank row and the buttons right-aligned: `[ Cancel ] [ Don't Save ] [ Save ]`, the
  primary one filled; a dangerous one filled with `button_dangerous_bg`.

### Notifications
- A toast is a box at the window's bottom right above the status bar, with a coloured `▌` cell at
  the left of its title row (info `icon_blue`, warning `warning`, error `text_dangerous`),
  the text wrapped and its actions as buttons. The Notifications popup is a box under the
  toolbar's mark: one row group per notification, separated by rules, unread ones marked `●`.

### Tooltips
- One line: a row of cells on `tooltip_bg`, one cell of margin each side, no frame. Several
  lines: a box. They sit on whole cells next to their target.

### Inputs
- A single-line input is a row of cells on `input_bg`: the text, a bar caret on a cell edge
  (the block caret when `editor.caretShape` is block), the selection in whole cells, the hint in
  `input_hint`. Focused: the cell before the text shows `›` in `input_focus_border`.

### Status bar
- One row of cells, tmux-like (as built): segments of text separated by `│`, the workspace
  segment filled, the language, encoding, line ending and position on the right; a segment
  hovered is filled.

### Files tool
- As built: rows of whole cells, `├─ └─` guides, `▸ ▾` folders, a glyph per file type, git letters
  in the last column. Drag targets fill their row in `drag_bg`; a drop line is a rule.

### Search tool (find in files)
- The query, replace and file mask as input rows, each with its label in a fixed column
  (`find`, `repl`, `mask`), and the option toggles `[Aa] [W] [.*]` filled when on. A rule.
- Results as a tree with the Files tool's guides: a file row `◆ name  dim/dir  (3)`, a match row
  `  12│ text with the match filled`. The summary (`17 matches in 4 files`) is set into the
  panel frame's bottom edge or as the last row.

### Git tool
- The commit message as a multi-row input region on `input_bg` (whole rows), then a row:
  `[x] Amend      [ Commit ] [ Push ↑2 ]`.
- Sections are rules with a title set in: `── Changes 3 ─────` and `── Unversioned 1 ───`. A
  file row: `[x] M ◆ name  dim/dir`, the letter in the git colours. Unsynced commits as
  history rows.
- History tool: a graph column painted as geometry (`│` rails, `●` commits, merges as diagonals
  through cell centres), then `3aa57f3  subject…  author  2h` columns. Chips (branch, author
  filters) are `[ label ▾ ]` segments.

### Changed Files (diff view)
- One box per changed file: the file's name and stats set into its top edge (` ◆ name +12 -3 `),
  and `[ Stage ] [ Revert ]` at its right end. Inside, two columns of rows split by a vertical
  rule joining the box (`┬ │ ┴`), each with its own line-number gutter in cells; added rows filled
  with `diff_added_bg`, deleted with `diff_deleted_bg`, changed with `diff_modified_bg`; hunks
  separated by a `⋯ 12 lines ⋯` row.

### Terminal tool
- Already a cell grid: its cell becomes `skin::Cell` (the terminal's own font size and spacing
  are ignored in the tui skin so it shares the grid), inside the panel frame; terminal tabs in the
  frame's top edge; the cursor drawn as the editor's caret.

### Run tool
- A tab per run in the panel's top edge (` ▶ name ` running in `text_positive`, ` ■ name ` when
  done; the exit code set into the bottom edge). Row 1: the command line in `text_secondary`,
  then a rule, then the output on the grid; `[ ↻ Rerun ] [ ■ Stop ]` right-aligned on row 1.

### Settings
- A tab with two regions split by a vertical rule: the page list on the left (rows, the
  selected one filled) and the page on the right. A setting is a row: `Label ........  value`,
  the leader dots in `text_tertiary`, the value right-aligned: check boxes `[x]`, dropdowns
  `[ dark          ▾ ]` (opening a menu box), numbers and text as inputs; a modified setting
  has `●` in `settings_modified` before its label. Group headings are rules with a title set in.

### Prompts (popup under the toolbar), recent workspaces
- Boxes, as Goto.

### Drag and drop
- The dragged tab or file is a one-row box following the pointer, snapped to cells; the drop
  target's frame turns `drag_border_highlight`; a split target fills its half in `drag_bg`.

## Checking it

Screenshots of the real window are the test. Where no input can be scripted (macOS without
accessibility permission), `WEEP_KEYS` drives a new window from inside: comma-separated steps of
gpui keystrokes (`cmd-p`, `comma` for `,`), `type:text`, `wait:ms`, and mouse steps in logical
window pixels split by `;`: `move:x;y`, `click:x;y`, `rclick:x;y`, `scroll:x;y;dy` (negative
scrolls down), `drag:x1;y1;x2;y2` and `hold:x1;y1;x2;y2` (a drag left pressed, for the ghost).
Run with `--skin tui` and a scratch `WEEP_CONFIG_DIR`, and capture only WEEP's window. Synthetic
input lands only while the window is active, so run one probe at a time.

Open points:
- The documentation box beside the completion box draws its own line next to the completion
  box's frame instead of sharing one column.
- A live window resize is clamped in code (whole rows kept) but not yet seen on screen.

