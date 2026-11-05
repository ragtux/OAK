# Roadmap: WEEP as a Fleet clone

WEEP's first goal is a faithful clone of JetBrains Fleet 1.48 as it behaves
**without Smart Mode**: the shell, the editor, Files, Search, Goto, Git,
Terminal, Run, Settings, themes and keymaps. Smart Mode features (completion,
navigation, refactorings, debugging) come after, fed by LSP and DAP servers
instead of Fleet's IntelliJ backend. Collaboration, remote workspaces, the
JetBrains account, AI and telemetry are out of scope.

"Faithful" is measured, not felt. Every milestone below ends when:

- the probe scenes for its surfaces pass `tools/fleet-probe/compare.py`
  (edges within 1 logical px of Fleet, colours exactly the theme's values),
- `tools/fleet-probe/coverage.py` shows every action, settings key and
  theme key the milestone claims,
- `docs/fleet-probe.md` records where each new constant came from.

`docs/probe-plan.md` describes the probes and harnesses this depends on.

## Where the numbers come from

Fleet's distribution describes most of its own surface as data, and the
inventory script reads it (`tools/fleet-probe/inventory.py`, output in the
gitignored `tools/fleet-probe/out/`). Fleet 1.48.261 has:

| Table | Count | WEEP covers (2026-11-03, after the first M7 pass) |
| --- | --- | --- |
| Action ids with a default chord (`fleet/keymap/fleet.json`) | 203 | 145 |
| Action ids described in plugin `documentation.yaml` files | 204 (385 with the above) | – |
| `settings.json` keys described in `documentation.yaml` files | 430 | 104 |
| Theme `colors` keys | 684 | 199 |
| Theme `textAttributes` keys | 157 | 43 |
| Icons (`frontend/icons/dark`) | 441 | mapped by hand so far |
| Keymap presets shipped | 8 (fleet, intellij, ij-classic, visual-studio, vscode, sublimeText, eclipse, emacs) | format only |

The colour keys group by UI component and are the best available map of
Fleet's widget set. Each milestone names the groups it consumes:

```
button 108   editor 43    problemsWidget 36  terminal 32   tileButton 32   inputField 30
toggleButton 30  ghostButton 28  vim 21  checkbox 20  tab 19  combobox 18  banner 16
dropdown 16  snippetProgress 16  ai 16  listItem 15  notification 14  pillButton 14  popup 12
chat 10  remote 10  snippet 10  icon 9  link 9  text 8  survey 8  tooltip 8  gitBranchTag 6
scrollbar 6  scrollToBottom 4  progressBar 4  highlight 4  shadow 4  tree 4  dragAndDrop 3
smartMode 3  search 3  statusBar 3  toast 3  background 2  border 2  completion 2  diff 2
settings 2  shortcut 2  tag 2  focusOutline island layout parameterInfo progressSpinner
progressInline separator
```

## Deliberate departures

Kept short on purpose; everything not listed here follows Fleet.

- **tree-sitter, not TextMate.** Fleet's non-Smart highlighting is TextMate
  grammars; WEEP uses tree-sitter mapped onto Fleet's `textAttributes` keys,
  which gives the same colours with better scoping and also powers
  expand/shrink selection, folding and bracket matching without a backend.
- **LSP and DAP, not the IntelliJ backend.** Smart Mode becomes "a language
  server is attached"; the lightning icon, states and colours stay Fleet's.
- **Native Markdown preview.** Fleet renders previews in an embedded
  Chromium (JCEF); WEEP paints Markdown itself.
- **Settings file.** WEEP reads `~/.config/weep/settings.json` with Fleet's
  keys, falling back to Fleet's own `~/.config/JetBrains/Fleet/settings.json`
  so an existing Fleet setup carries over.
- **No JVM, no telemetry, no account.** Nothing phones home; nothing needs a
  login.

## Milestones

Sizes: S is a day or two, M a week, L a few weeks, XL longer.

### M0 Scaffold (done, commit 883cb9e)

Toolbar, Files island, editor island with tabs, status bar, Fleet theme and
keymap formats, four gpui-free library crates, measured constants at zoom 2.0.

### M1 Shell fidelity (done, 2026-11-02)

The frame looks right in every static configuration before more surfaces
are added to it. Batches 1–3 ran against Fleet and WEEP on Windows
(`docs/fleet-probe.md`, third pass); the `editor` and `light` scenes pass
`compare.py` on every island, tab-row and status-bar edge. Left for later:
shared buffers across splits (a split-right copy is its own editor until
M2), the overflow glyph in a narrow panel's tab row, Fleet's zoom step (WEEP
steps by 0.1), the Windows caption on the other platforms (they keep the
desktop's title bar, as Fleet does), and the island shadow, which Fleet
does not draw (the `shadow.*` keys are the popups').

- `zoomLevel` support: one scale factor applied to every metric, with the
  `zoom-in`, `zoom-out`, `reset-zoom` actions; this is also what lets probes
  at zoom 2.0 be compared with WEEP at 1.0.
- Panels: left, bottom and right islands with draggable splitters, minimum
  sizes, persisted widths, `hide-active-panel`, panel tab rows with more than
  one tool, the `+` tool chooser.
- Editor tabs: hover and modified states, close button (`ff.tabCloseButtonOnTheLeft`),
  overflow scrolling, drag to reorder, `tabs/close-other-tabs`,
  `tabs/close-tabs-to-the-right`, `tabs/close-saved-tabs`, `tabs/reopen-closed-tab`,
  preview tab (`editor.previewTab`).
- Splits: `split-right`, `unsplit`, `focus-*-split`, `move-tab-to-*-split`.
- Tooltips (`tooltip.*`), focus outline (`focusOutline`), scrollbars
  (`scrollbar.*`) on every scrolling surface, separators, island shadow.
- Light theme pass and theme switching at runtime; `themes/*.json` from
  `~/.config/JetBrains/Fleet/themes` and `~/.config/weep/themes`.
- Settings: `weep-settings` crate reading Fleet's keys (start with `theme`,
  `zoomLevel`, `editor.*` display keys, `files.*`), live reload.
- Probes: batches 1–3 in `docs/probe-plan.md`.

Theme groups: `tab`, `tooltip`, `scrollbar`, `focusOutline`, `island`,
`layout`, `separator`, `shadow`, `border`, `background`, `text`, `icon`.

### M2 Editor core (built 2026-11-02; the M2 scenes run against Fleet on the Mac, fifteenth and sixteenth passes)

Everything Fleet's editor does to a plain text file. Built in one pass on
2026-11-02 against the batch 5 measurements (`docs/fleet-probe.md`, fourth
and fifth passes). What batch 5 did not measure is marked "estimate" in
`theme.rs::metrics` and in the probe record; the scenes `find`, `fold` and
`wrap` ran on both editors on the Mac (`docs/fleet-probe.md`, fifteenth to
seventeenth passes) and corrected the find bar, the replace row, the fold mark's
click, rendered spaces, the wrap rule, the find bar's Tab order and focus
ring, its current match on reopening, when it pushes the text down, the text
it hides, the status badge under it and the fields' focus border (1 px on
whole device pixels), its width, the query's font, the toggles' glyphs and the
soft wrap's width; the eighteenth pass the gutter's width against the font
size and the line count. Still open from them: the Outline and VerticalThin carets, the status badge's cut-out over wrapped
text (square corners, a shaded word), and the
sticky lines (the fixture has no Rust file longer than the window). Left for later: the smooth caret (`editor.caretSmoothAnimation`
is read, not animated), an "in selection" button on the find bar (the mode
engages by itself when the bar opens on a multi-line selection; whether
Fleet also shows a toggle is unprobed), the breakpoint hint circle in the
hovered gutter (M8), the `editor.guides` ruler colour (no theme key names
it; it uses `editor.indentGuide`), and Fleet's Goto popup geometry for
`goto/line` (batch 6).

- Done: one document per open file (`crates/weep/src/document.rs`), shared by
  every tab and split that shows it; each view keeps its own cursor, folds
  and scroll, which follow the other views' edits through the buffer's edit
  log (`weep-text`'s `Cursor::sync`). Undo and the dirty state are the
  document's. The `splits` scene (WEEP only) shows it.
- Done (batch 5, `surface` and `surface2` scenes): the editor surface
  measured and drawn as Fleet does: rounded selection rows with the newline
  cell and the empty-row stub, the current-line wash under a selection,
  `identifier.underCaret` boxes on every whole-word occurrence,
  `editor.brace.match` on the bracket pair (the grammar's pair first, the
  text scan's for plain text), indent guides with the caret block's in
  `.current`, the caret-line (`editor.currentLine.scrollbarMark`) and
  search-match scrollbar marks.
- Done: multiple carets (`editor/add-caret-above|below`,
  `editor/add-carets-to-ends-of-selected-lines`, `select-next-occurrence`,
  `select-all-occurrences`, `unselect-occurrence`, Alt+click), every edit
  at every caret in one undo step; a paste of N lines into N carets.
- Done: the find and replace bar (`crates/weep/src/find.rs`; `find`,
  `replace`, `replace-one`, `replace-all`, `next-occurrence`,
  `prev-occurrence`) at batch 5's geometry, with the match-case, words and
  regex toggles (`inputField.*`, `toggleButton.*`), the match counter, the
  search kept to a multi-line selection, `editor.useSelectionForSearch`, and
  `$1` groups in regex replacements (`weep-text`'s `search` module, on the
  `regex` crate).
- Done: `expand-selection` (the camel hump, the word, then the enclosing
  syntax nodes), `shrink-selection` (back through the same steps),
  `editor/move-caret-to-matching-bracket`, folding (`editor/expand-folding`,
  `editor/collapse-folding`, the gutter chevrons on hover, the `{…}` mark
  from `editor.foldedMark.*`, `editor.showFoldings`) and the sticky lines
  (`editor.showStickyLines`, `stickyLinesMaxCount`), all from tree-sitter
  (`weep-syntax`'s `structure` module: `SyntaxTree::expand|shrink|folds|
  matching_bracket|errors`).
- Done: rendering options from settings: soft wrap (`editor.softWrapEnabled`,
  `editor.guideToSoftWrapBy`; word-wrapped at the view's width through a
  display map of buffer rows to display rows), `editor.guides` and
  `showVisualGuides`, `showIndentGuides`, `renderWhitespace`, `lineNumbers`
  on/relative/off, `caretShape` (line, block, underline) and
  `caretBlinkingEnabled`, `activeLineHighlightEnabled`, `showScrollbarMarks`,
  `distractionFreeMode` (no tabs, the editor area centred),
  `centerEditorHorizontally`, `lineSpacing`, `fontFamily|Size|Weight|
  Features|ligatures`.
- Done: editing behaviour: the indenter (`enableIndenter`: a level after an
  opener, the closer on its own line, a typed closer re-indented),
  auto-pairs (insert, step over, wrap a selection, backspace both),
  `editor.tabSize`, `editor.indentStyle`, `editor.autoDetectIndentation`,
  `.editorconfig` (`weep-settings`'s `editorconfig` module: indent style and
  size, end of line, trim and final newline), `trimTrailingSpacesOnSave`
  (caret rows kept), `autoSave` and `autoSave.delay`,
  `editor/change-line-endings` (also a click on the status bar's LF/CRLF),
  encoding and language in the status bar, `toggle-case`, `wrap-line`,
  `comment-block`, `editor/paste-from-history` (the editor's own copy
  history), `goto/line`, the big-file confirmation
  (`localBigFileMinimumThreshold`).
- Done: scratch files (`createScratchOnNewFile`, `--scratch`, in WEEP's
  `scratches/` folder), `new-file` (a name prompt, the file created in the
  Files tree's folder), `files/rename`, `files/delete` (confirmed) on the
  tree's selection or the active tab's file.
- Probes: batch 5 (`surface`, `surface2`, run); `find`, `fold`, `wrap`
  (WEEP's side run, Fleet's side and the comparison still to do).

Theme groups: `editor` (43), `highlight`, `inputField`, `toggleButton`,
`ghostButton`, most `textAttributes`.

### M3 Workbench tools (built 2026-11-02; batches 6, 7 and 8 measured)

The tools Fleet has without a backend, each in Fleet's island. Built in one
pass on 2026-11-02 from what Fleet's own files say (its local storage holds
the Goto panel's view state and the Search tool's query model,
`docs/fleet-probe.md`, sixth pass) and from the popups batch 5 caught, then
rebuilt the same day to what the `goto` and `popups` scenes showed on Fleet:
the Goto popup is 680 wide with four tab chips (Goto, Actions, Tools, Text
Search), filter buttons under the field, 26 px rows and the preview under
the list; `find-in-path` opens its Text Search tab; menus have 28 px rows
and submenus. The dialogs, toasts and the panel's Search tool are still
estimates (Fleet's help site was retired in December 2025, so its pages are
no longer a source). The Settings tool followed from batch 8 the same day
(`docs/fleet-probe.md`, seventh pass): Fleet's pages as an editor tab, every
row bound to its key with the default read off the page, and the
Notifications popup under the bell replaced the panel tool; the same pass
added the inline rename, "Open in Split", the changed-on-disk pill, the
find bar's push-down and the gutter's breakpoint hint. Left for later:
the Settings tool's Plugins and AI pages and the
workspace scope (the ninth pass settled `autoSave`'s three values, the
modified-value mark, the shortcut overlay and the dialog-less Delete, all
built), the snapshots tool (snapshots are
written by `local-history/make-snapshot`, not yet listed), the `showShortcuts`
overlay and `inputSequenceTimeoutMilliseconds` (gpui owns chord timing),
`restoreWorkspacesOnStartup` and the empty state, a notification's actions
and the `banner.*` bands (the on-disk change is a popup-coloured pill, built),
the Search tool's path scope (Fleet's query has a
`path`; WEEP searches the whole workspace), and Goto's typed mode prefixes,
which no source shows.

- Done: the Goto panel (`crates/weep/src/goto.rs`): `goto/everything`,
  `goto/file`, `goto/symbol` and `show-file-symbols` (tree-sitter symbols,
  `weep-syntax`'s `symbols` module), `goto/action` (every action id WEEP binds,
  with its chord), `goto/tool`, `goto/recent-file` (the recent files lead the
  empty list, as Fleet's `RecentNavigationEntity` rows do),
  `show-recent-workspaces`, `toggle-goto-preview` (a read-only editor under
  the list, shown by default as Fleet stores it), `goto_panel.showShortcuts`,
  `ff.previewAndPrioritiseFileSymbolsInGotoEverywhere`; Shift+Enter opens in
  a split, Tab switches lists. The matcher and the file index are
  `weep-search` (`fuzzy`, `files`), the index walked with the `ignore` crate.
- Done: `find-in-path` and `replace-in-path` as the Goto popup's Text
  Search tab (Fleet's own place for them): the match-case, words and regex
  toggles, the folder and file-mask fields, Fleet's limit of 100, one row
  per match with the matches in `search.match.*`, open documents searched
  with their unsaved text, Replace and Replace All through the documents
  (one undo step each) and on disk for the rest,
  `ff.searchInBigUnindexedFiles`; the Search tool in the panel
  (`crates/weep/src/search_tool.rs`, `search/jump-to-tool`) shows the same
  results as a tree. The engine is `weep-search`'s `grep` module.
- Done: the Files tool: `files/new-folder`, multi-select (Ctrl and Shift),
  `files/delete` on every selected row, drag and drop onto a folder
  (`dragAndDrop.*`), `toggle-hidden-files`, `files.keyboardNavigation`
  (highlight mode with its cursor rectangle), `open-in-associated-application`,
  `files/show-in-file-manager`, `copy-path`, the right-click menu and
  `show-context-menu`.
- Done: popups and chrome (`menu.rs`, `dialog.rs`, `notifications.rs`):
  context menus for the tree, the tabs and the editor with Fleet's rows and
  submenus (`Open In ›`, `Go To ›`), the main menu with Fleet's nine
  submenus (`popup.*`, `listItem.*`), Fleet's confirm dialog for
  deletes, big files, unsaved changes and `warn.before.quitting`
  (`button.primary|secondary|dangerous.*`), toasts and the Notifications tool
  (`toast.*`, `notification.*`) fed by what used to go to stderr, the
  unread dot on the toolbar's bell.
- Done: windows: `show-recent-workspaces` opens another window on a root
  from `layout.json`; the toolbar's buttons dispatch their actions.
- Done: keymap presets: the `keymap` setting names a preset file copied to
  `~/.config/weep/keymaps/<name>.json`, merged over Fleet's defaults.
- Done: `local-history/make-snapshot` writes a copy under WEEP's
  `local-history/` folder.
- Probes: batches 6, 7 and 8 as the `goto`, `popups`, `settings` and
  `dialogs` scenes, run on both editors (the `compare.py` pass is still to
  be run on the popups and settings pairs; the confirm dialog is still to
  be caught).
- Done: the Settings tool (`open-settings`): `settings_schema.rs` holds
  Fleet's pages (sections, groups, labels, keys, control kinds, defaults);
  `settings_view.rs` draws them at the measured values and writes each
  change to WEEP's `settings.json` through `settings::set_value`.

- Goto panel: `goto/everything`, `goto/file`, `goto/symbol` (file symbols
  via tree-sitter, `show-file-symbols`), `goto/action`, `goto/tool`, recent
  files, `goto_panel.showShortcuts`, `toggle-goto-preview`.
- Search tool: `find-in-path`, `replace-in-path`, `search/replace`, result
  tree, preview, `ff.searchInBigUnindexedFiles` behaviour; a ripgrep-style
  engine over the workspace (`grep-searcher`/`ignore` crates).
- Files tool completion: `files/new-folder`, rename, delete, drag and drop
  (`dragAndDrop.*`), `toggle-hidden-files`, `files.exclude`,
  `files.compactDirectories`, `files.selectOpenedFile`, `files.openFileMode`,
  `files.toggleFolderMode`, `files.keyboardNavigation`, multi-select,
  `open-in-associated-application`, `show-context-menu`.
- Popups and chrome: context menus and the main menu (`popup.*`,
  `listItem.*`, `shortcut.*`), notifications and the notifications tool
  (`notification.*`, `toast.*`), banners (`banner.*`), confirm dialogs
  (`confirm-dialog`, `alternative-confirm`), `warn.before.quitting`.
- Windows: recent workspaces / empty state (`show-recent-workspaces`,
  `restoreWorkspacesOnStartup`, `showEmptyStateOnLastClosedWindow`),
  `toggle-full-screen`, `minimize-window`, multiple windows.
- Settings tool, the rest: the Plugins and AI pages, the modified-value
  mark (`settings.modified.indicator`), the workspace scope, `tileButton`
  and `pillButton` states, the rows whose keys the documentation does not
  name.
- Keymap presets: load any of Fleet's eight preset files from the user's
  config; `keymap` setting; `showShortcuts` overlay;
  `inputSequenceTimeoutMilliseconds` for chords.
- Local history (`local-history/make-snapshot`, snapshots tool).
- Probes: batches 6–8.

### M4 Git (built 2026-11-02; batch 10 measured)

Begun with what the shots on hand show (`docs/fleet-probe.md`, eighth
pass): the `weep-git` crate (branch and per-file status through the `git`
binary) and the Files tree's names coloured and badged by status. Batch 10
ran that evening (ninth pass) and gave the gutter's line-status bars with
their popup and the branch tag's menu, then the Git tool (status rows with
staging, commit, unsynced commits) and the History tool
(`git/file-history`). The rest followed the same night (tenth pass): the
blame column (`git/toggle-blame`), the Changed Files tab (`diff_view.rs`:
cards per changed file, git's hunks paired side by side with the file's
syntax colours, staging, Revert, the Copy menu), `diff/two-files` and
`diff/with-clipboard` on the same view, `git/push` and `git/pull` off the
main thread with `git.push.confirm`, `git/revert` with `git.revert.confirm`,
the Git tool's filter and chip menus, the History tool's chips, the Git
submenus, Fleet's two-chord git shortcuts in the keymap, and dates in the
machine's zone.

Left for later: staging single hunks (the diff's row checkbox stages the
file), "Create New Branch...", `git.enable3WayDiffView` (conflicts show as
a plain diff), and a Fleet pass over the estimates the tenth pass lists
(the filter and chip menus, the dialogs' wording, the fold's wave).

- Status colours in the tree and tabs (`vcs/*` icons, `tree.*`), line
  status in the gutter (`editor.showLineStatus`), branch tag in the toolbar
  (`gitBranchTag.*`), `git.autoFetch` and `git.fetchInterval`.
- Git tool: changes view, commit (`git/commit/*`), history
  (`git/history/*`, `git/file-history`), `git/toggle-blame`, `git/pull`,
  `git/push`, `git/revert` with `git.revert.confirm` and `git.push.confirm`.
- Diff viewer (`diff.*`, `diff/two-files`, `diff/with-clipboard`,
  `new-diff-tab-tool`, `git.enable3WayDiffView`), `ff.expandGitDiffByDefault`.
- Shell out to `git` as Fleet does (`git.discoverGitManually`); `git2` only
  if speed demands it.
- Probes: batch 10.

### M5 Terminal (built 2026-11-02; batch 9 measured)

Built in one evening (`docs/fleet-probe.md`, eleventh pass): the
`weep-terminal` crate wraps `alacritty_terminal` 0.25 (the PTY, the parser,
the grid, the scrollback and the selection are Alacritty's; ConPTY on
Windows) behind a gpui-free API of profiles, key encoding, a grid snapshot
and the 256-colour table, tested on its own including a real shell. The
app's `terminal.rs` paints the snapshot at `terminal.fontSize` x
`terminal.lineSpacing` with the 32 `terminal.ansiColors.*` keys, takes
typed text through the platform's text input and every other key through
`weep_terminal::keys`, and the workspace keeps one tab per shell in the
bottom panel (`workspace/terminals.rs`). Batch 9 ran on Fleet twice the
same evening (the `terminal` and `terminal2` scenes): the grid's inset, the
tab's icon, title, close cross and focused colours, the selection's extent,
the right-click menu and the toggle were measured or read off the shots and
the `terminal` pair passes `compare.py` on the island, tab-row and
first-row edges.

- Done: `terminal/new-tool`, `terminal/toggle` (hides the panel when its
  terminal has the focus, else shows one focused), `terminal/jump-to-tool`,
  `terminal/new-with-profile` (a menu of the profiles), `terminal/select-
  default-profile` (the Settings tool at Tools > Terminal),
  `terminal/edit-profiles`, and on a focused terminal `terminal/copy`,
  `terminal/paste` (bracketed when the program asked), `select-all-terminal`
  and `terminal/clear`; Fleet's five terminal keymap rows added to the
  bundled keymap, which is a subset of Fleet's file; the `+` chooser and
  Goto's Tools list open a terminal; `close-tab`, the tab's cross, its menu
  and the shell's exit close one.
- Done: profiles: `terminal.profiles` rows (Fleet's `{name, program,
  args}`) plus the shells found on the machine (`pwsh`, `powershell`, `cmd`;
  `$SHELL`, `bash`, `zsh`, `fish`, `sh`), `terminal.profile.default` by
  name, `launchTerminalOnOpen`.
- Done: settings: `terminal.fontFamily|fontSize|lineSpacing|fontWeight|
  fontFeatures`, `terminal.caretShape` (null is the editor's),
  `terminal.lineCapacity` (the scrollback), `terminal.copyOnSelection`,
  `terminal.enableFocusEditorOnEsc` and `enableFocusPanelsInApps`,
  `terminal.clearTerminalOnCmdK` (the Goto chord clears instead),
  `terminal.useMacOptionAsMeta`, `terminal.bell` (counted, not sounded).
- Done: the tab as batch 9 shows it: the tool icon, "Terminal", a close
  cross, the `selectedFocused` colours while the shell has the focus (every
  panel tab now takes them while its panel is focused), a tooltip with the
  profile and the folder; the selection washes only the row's text; the
  right-click menu with Fleet's rows and chords (Page Up/Down on
  Shift+PageUp/PageDown, Line Up/Down on Alt+Up/Down); the caret from the
  editor's colour; `scrollToBottom.*` on a button shown only while output
  arrives behind a scrolled-back view.
- Probes: batch 9 as the `terminal` and `terminal2` scenes, both editors;
  the driver gained a `wheel` step.

Left for later: Fleet's terminal search bar (Find, Ctrl+F), paths and URLs
painted as links, pixel scrolling (Fleet shows a part row at the bottom
after a wheel notch; the amount per notch was not pinned down), the caret's
blink (no shot caught Fleet's caret), mouse reporting to full-screen
programs, an audible bell, the tooltip on one line with an ellipsis, the
cross's hover colour, Fleet's profile chooser popup (WEEP shows a menu), the
keys without an effect in WEEP (`terminal.fontHinting`, `fontSmoothing`,
`TypeAheadLatency`, `enableTypeAheadInTerminal`, `shellIntegration`,
`outputBufferLines`, `terminal.tool`, `ff.alignTerminalByGrid`: WEEP always
aligns by the grid), and Fleet's empty editor state, which batch 9 caught in
passing (three shortcut cards over "No recent files").

### M6 Run and problems (built 2026-11-02; batch 11 measured)

Built in one night from the `run` scene (`docs/fleet-probe.md`, twelfth
pass), which ran on Fleet first: the `weep-run` crate holds Fleet's
`.fleet/run.json` as data (serde over Fleet's field names, unknown keys
kept, a scan for each row's `"type"` line) and runs a configuration as a
child process whose lines arrive through a channel; the app's `run_tool.rs`
is the tab per started configuration in the bottom panel, `run_popup.rs`
the Run & Debug popup in the Goto popup's frame, and `workspace/runs.rs`
the actions. Problems come from tree-sitter's `ERROR` and `MISSING` nodes
(`weep-syntax`'s `SyntaxTree::problems`, cached per document): the
underline, the scrollbar mark, the status badge, the widget and
`editor/go-to-next|previous-problem` read them. On the `run` pair,
`compare.py` finds the badge, the gutter's run icon and the bottom island's
edges within 0.7 of Fleet's (one inner edge of the check mark's stroke at
1.3); the popup's and the card's edges were measured by hand to the same
tolerance (`docs/fleet-probe.md`, twelfth pass). The fixture gained two `command` rows and a `cargo` row in run.json
and `misc/broken.json`, `misc/broken.rs`.

- Done: `.fleet/run.json` (`name`, `type`, `program`, `args`, `cargoArgs`,
  `workingDir`, `environment`, `environmentFile`, `dependsOn`,
  `allowParallelRun`); `command` rows run their program, `cargo` rows run
  `cargo`, other types say so in a notification; `run.environmentFile` as
  the default environment file; the Run & Debug popup (`run-and-debug`,
  the toolbar's run button): the field runs a typed line through the shell,
  Recent rows (`run.recentConfigurationsSize`, persisted in `layout.json`),
  the workspace's rows with Run and a "..." menu (Edit Configuration, Delete
  Configuration), "Edit Run Configurations..." (the file written from a
  template when absent), `run/delete-configuration` (Shift+Delete) through
  the open document or the file; `run/run-in-context` (the row at the caret
  in run.json, else the last run, else the popup), `run/rerun` (the shown
  tab's command again in the same tab), `run/stop`,
  `stop-run-configurations`; the run tab's icon per type with a badge when
  the exit code was not 0, its close cross, `close-tab`; the gutter's run
  icons (`editor.renderRunInGutter`, `ff.runIconsAfterLineNumbers`), a
  click on one running the row.
- Done: problems: the 2 px underline and the scrollbar mark from
  `problem.error`, the status badge (`editor.showFileStatus`) with Fleet's
  tooltips, the widget with its tag, sliders and close buttons, the card,
  the expanded row and the hints, `editor/go-to-next-problem` (Ctrl+E) and
  `editor/go-to-previous-problem`, Escape closing the widget,
  `editor.markupWithoutSmartMode`, `editor.showTooltipOnHover` with
  `editor.hoverTooltipDelayMs`.
- Probes: batch 11 as the `run` scene, run on both editors.

Left for later: Fleet's Build Output tool (the project-import log, nothing
without a backend), `dependsOn` and `allowParallelRun` beyond reading them
(a dependency is not started first; a parallel run gets its own tab, a
non-parallel one reuses the finished tab of the same name), the sliders
button's menu (unknown), the hovered error's tooltip against Fleet, a click
on the gutter icon against Fleet, the "..." icon's size, paths in the
output as links, and LSP diagnostics (M7).

### M7 Smart Mode via LSP (XL; first pass built 2026-11-03, batch 12 half run)

Built in one pass without Fleet, then corrected the same night from the
`smart` scene played into the user's Fleet (`docs/fleet-probe.md`,
thirteenth pass): the completion popup, its documentation pane, the
parameter info card, the Smart Mode card under the lightning and the status
bar's symbol crumbs now follow Fleet's measurements; the declaration,
usages and diagnostics surfaces stay estimates (the Fleet run derailed on
the Jump-to prompt before them). Fleet's Rust Smart Mode turned out to be
rust-analyzer as well, so WEEP and Fleet answer from the same server.
the `weep-lsp` crate speaks the Language Server Protocol to a server on its
pipes (JSON-RPC framing on two threads, the handshake, typed requests whose
answers come back with their ids, the server's own requests answered or
handed up, positions in whichever of UTF-8, UTF-16 or UTF-32 the server
negotiated, edits as char-range changes, snippets as plain text, semantic
tokens decoded and mapped onto Fleet's `textAttributes` keys, Markdown as
plain lines), tested against the real rust-analyzer (ready, hover,
definition, completion, tokens, shutdown). The app's `workspace/smart.rs`
runs one server per language (rust-analyzer, found on `PATH` or through
rustup), feeds it the open documents (full text on every edit) and asks on
the editors' behalf; `editor/smart.rs` paints the popups. Every size of the
new surfaces is an estimate until the `smart` scene runs on Fleet with Smart
Mode on (the fixture now has a `Cargo.toml` so both sides can load it).

- Done: the lightning (`smartMode.background.enabled|error`,
  `smartMode.border.loading`): off, starting (the server's progress in the
  tooltip), on, failed; its menu (Enable/Disable, Restart Language Servers;
  the choice kept per workspace in `layout.json`); the server's messages as
  notifications; `rust-analyzer.*` settings keys handed to the server as
  its configuration.
- Done: diagnostics as the documents' problems (`problem.error|warning|
  warning.weak|info`, the widget's warning and weak cards and tags, the
  badge following the worst severity, the quick fixes of the expanded
  problem as rows under its message), semantic highlighting
  (`editor.semanticHighlighting`) laid over tree-sitter's spans and moved
  through edits until the next answer.
- Done: `completion` and `smart-completion` (the popup at the word, rows
  with a kind icon, the label with the matched chars in
  `completion.match.text`, the detail right-aligned, filtered by the Goto
  matcher as you type, Enter or Tab accepting with the item's edit, its
  additional edits (imports) in the same undo step and a snippet's first
  tabstop selected, the auto popup after `completion.autoPopupDelay` on a
  word or trigger char), `show-documentation` (the popup's documentation
  pane, else the hover at the caret; also on the pointer resting on a
  symbol after `editor.hoverTooltipDelayMs`), `show-parameter-info` (the
  active parameter in `parameterInfo.match.text`, re-asked as the caret
  moves, opened by `(` and `,`).
- Done: `go-to-declaration` (also Ctrl+click), `go-to-type-declaration`,
  `find-usages` as Fleet's inline usages (the fifteenth pass: a panel laid
  between the caret's line and the next through the display map's interline
  blocks, the usages without the declaration grouped by file in its left
  third, the selected one in a read-only preview on the right with the path
  under it; `editor.interline.*`; ↑ ↓ ← → Enter and Escape, a double click
  opens; `crates/weep/src/usages.rs`),
  code vision (`editor.showCodeVision`, the fifteenth pass's "4
  implementations" over `pub struct Record`: the server's code lenses,
  resolved, as 18.5 px rows above their lines that the display map lays
  between the 22 px ones, the title in 10.2 px Inter from the line's first
  non-blank column in `editor.interline.text.default`, `.hovered` under the
  pointer; a click lists the lens's locations in the inline usages panel
  headed "Implementations", or goes straight to the only one; rust-analyzer
  is told WEEP handles `rust-analyzer.showReferences` so it sends them, and
  its runnables are left to the gutter's run icon; asked again on
  `workspace/codeLens/refresh` and after a "content modified" answer, an
  older answer never replacing a newer one),
  `back` and `forward` over the positions jumped from, `rename` (the name
  in a prompt, preselected per `rename.selectOnRename`, the server's edit
  applied across open documents and files on disk), `quick-fixes` (a menu
  at the caret), `apply-primary-quick-fix`, `optimize-imports`
  (`source.organizeImports`), `reformat-code` and `editor.formatOnSave`,
  `workspace/applyEdit` from the server.
- Menus: the editor menu's Quick Fixes..., Refactor ›, Go To ›, Reformat
  Code and Git › rows; the main menu's Code rows.
- Probes: batch 12 as the `smart` scene (written, not yet run on either
  editor).

Left for later: inlay hints (`textDocument/inlayHint`: Fleet shows
parameter names and types inside the line, which needs the display map to
widen lines), the run triangle in the gutter at `fn main` (a code lens),
the inline rename (`editor.rename.current|entries` boxes
and `rename.autoRenameOnTyping`; WEEP renames through a prompt), Fleet's
own panel for a code vision click (unprobed: WEEP's is the inline usages
panel) and the separator between two lenses on one row, `goto/line` as Fleet's "Jump to" card in the Goto frame (an M3
estimate the pass corrected on paper), `insert-snippet` (Fleet's
`snippets.json` format is unread), `complete-statement`,
`show-quick-fixes-preview`, `go-to-next|prev-history-item` (what Fleet's
history items are is unprobed), the inline completion keys (AI), incremental
`didChange` (WEEP sends the whole text), `toolchains`, servers for the other
languages, a `cargo check` run's diagnostics shown as they arrive (the
server's `checkOnSave` runs; its progress shows in the lightning's tooltip),
and Fleet's Smart Mode popup under the lightning (WEEP shows a menu).

### M8 Debugger via DAP (L; built 2026-11-04, batch 13 run 2026-11-05, the rest built 2026-11-05)

Built in one pass without Fleet, by five agents split by file ownership over
a skeleton of shared types (`docs/fleet-probe.md`, nineteenth pass, lists
every estimate). The `weep-dap` crate speaks the Debug Adapter Protocol to an
adapter on its pipes (Content-Length framing on worker threads, the
initialize / launch or attach / initialized / setBreakpoints /
configurationDone handshake, requests queued until it is done, 0-based lines
against the protocol's 1-based ones, Rust's pretty printers loaded from the
toolchain), tested against the real lldb-dap (Xcode's on macOS; `PATH`, the
versioned LLVM paths and `lldb-dap.exe` elsewhere, unverified). Line
breakpoints live in one app-wide store (`breakpoints.rs`) keyed by path; the
debug sessions in `debugger.rs`; `workspace/debug.rs` starts them from run
configurations.

- Done: breakpoints in the gutter (a click, `debug/toggle-breakpoint`,
  `toggle-enabled-breakpoint`, `remove-breakpoint`, `editor.showBreakpoints`):
  enabled, disabled and not-verified looks with tooltips, lines that follow
  edits (a deleted line takes its breakpoint), kept per workspace in
  `layout.json`; the execution point (`debug.currentFrame` and an arrow, top
  frame and caller).
- Done: sessions from `.fleet/run.json`: a `cargo` row is built with JSON
  messages (its output in the Debugger Console) and its bin or test
  executable launched (`weep_run::cargo`); a `command` row's program launched
  directly; `debug/debug-in-context`; Debug in the Run & Debug popup (a
  button on cargo rows, Shift+Enter); the toolbar's debugging state;
  `debugger/attach-to-process` (the user's processes in a Goto-framed list).
  On a stop: the threads, the stack, the first scope expanded, the watches
  re-evaluated, the frame's file opened and the Debug tool shown.
- Done: `resume`, `pause`, `step-over`, `step-into`, `force-step-into`
  (lldb's step-into), `step-out`, `stop`; the Debug tool (frames, variables,
  watches with `add-watch`, `edit-watch`, `delete-watch`), the Breakpoints
  tool, the Debugger Console (output and an evaluate field); their
  `jump-to-tool` and `new-tool` actions; the floating debug panel
  (`editor.showFloatingDebugPanel`); `quick-evaluate-expression` and the
  value on hover while stopped. Both skins.
- Done (2026-11-05, the rest of the list, three agents; `docs/fleet-probe.md`,
  twenty-third pass): conditions, log messages and hit counts (`hitCondition`)
  and exception breakpoints (the adapter's `exceptionBreakpointFilters`, WEEP's
  own "Rust: on panic" as a function breakpoint on `rust_panic`, off by
  default) in the store, the protocol and `layout.json`; a panic stop reads
  "Panic: <message>" and selects the first frame of the user's code;
  `debug/drop-frame` (Backspace/Delete in the frames list: `restartFrame`
  where the adapter has it, else LLDB's `thread return`, which returns to the
  caller after the call rather than re-running it); expanded variables kept
  across stops (by their names' path under the function); `runInTerminal`
  answered with a Terminal tab (WEEP never asks for it: Fleet has no run.json
  key, and Xcode's lldb-dap launcher cannot be attached to on macOS); undo
  brings back a deleted line's breakpoint and move-line carries breakpoints
  with their text.
- Done (2026-11-05, twenty-fourth pass, `debug-rest` scene, run on both):
  the gutter run icon as Fleet's: a click opens Run '<name>' (⇧⌘R) / Debug
  '<name>' (⌥⌘R) with the "⇧Click on icon to Run, ⌥Click to Debug" hint,
  Shift+click runs, Alt+click debugs, a right click does nothing; Alt+click
  in the editor always adds a caret (Fleet has no quick evaluate on a click,
  and `debug/quick-evaluate-expression-with-click` is not offered, so WEEP
  binds none); the Breakpoints tool lists line breakpoints only, as Fleet's
  (the properties pane and the exception group stay in the code behind
  `SHOW_BREAKPOINT_EXTRAS`), with a row menu (Remove Breakpoint ⌫, Disable
  Breakpoint); Fleet's empty tool for Breakpoints and Debug (`empty_state.rs`,
  the Debug tool's "Run & Debug ⌘R" button), the Debug tool back to it once
  every session has ended; no dimmed library frames; no tooltip over an open
  menu.
- Probes: batch 13 as the `debug` scene (twentieth to twenty-second passes)
  and batch 13b as `debug-rest` (twenty-fourth). Fleet 1.48's quick evaluate
  shows nothing for Rust (Go to Action or ⌥⌘E), so WEEP's popup stays an
  estimate.

Left for later: redo bringing back a breakpoint an undo deleted; Rust
`fn main` / test run icons in source files (Fleet shows them from Smart
Mode's runnables, with a generated configuration such as "run debuggee";
WEEP's menu exists only in run.json); adapters for other languages (none installed here, and Fleet's
run.json keys for them are unknown); `java.debugger.*` (Java, likely never).

### Later or never

Vim mode (`vim.*`, 21 keys, a plugin in Fleet too), Markdown preview and the
`markdown/*` actions (small, can slot into M3), spellchecker, Docker, SSH
and WSL workspaces, collaboration, AI, Toolbox, plugin management, surveys.

## Working order inside a milestone

1. Run `inventory.py` and `coverage.py -v` for the milestone's groups; the
   missing lists are the task list.
2. Probe first (`docs/probe-plan.md`), record the values in
   `docs/fleet-probe.md`, add a compare scene.
3. Build the surface reading only Fleet keys (the lint in the probe plan
   rejects hard-coded colours).
4. Run the scene comparison; fix the constant, not the comparison.
