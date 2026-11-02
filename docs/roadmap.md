# Roadmap: Oak as a Fleet clone

Oak's first goal is a faithful clone of JetBrains Fleet 1.48 as it behaves
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

| Table | Count | Oak covers (2026-11-02, after the M3 build) |
| --- | --- | --- |
| Action ids with a default chord (`fleet/keymap/fleet.json`) | 203 | 114 |
| Action ids described in plugin `documentation.yaml` files | 204 (385 with the above) | – |
| `settings.json` keys described in `documentation.yaml` files | 430 | 89 |
| Theme `colors` keys | 684 | 125 |
| Theme `textAttributes` keys | 157 | 32 |
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
  grammars; Oak uses tree-sitter mapped onto Fleet's `textAttributes` keys,
  which gives the same colours with better scoping and also powers
  expand/shrink selection, folding and bracket matching without a backend.
- **LSP and DAP, not the IntelliJ backend.** Smart Mode becomes "a language
  server is attached"; the lightning icon, states and colours stay Fleet's.
- **Native Markdown preview.** Fleet renders previews in an embedded
  Chromium (JCEF); Oak paints Markdown itself.
- **Settings file.** Oak reads `~/.config/oak/settings.json` with Fleet's
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
are added to it. Batches 1–3 ran against Fleet and Oak on Windows
(`docs/fleet-probe.md`, third pass); the `editor` and `light` scenes pass
`compare.py` on every island, tab-row and status-bar edge. Left for later:
shared buffers across splits (a split-right copy is its own editor until
M2), the overflow glyph in a narrow panel's tab row, Fleet's zoom step (Oak
steps by 0.1), the Windows caption on the other platforms (they keep the
desktop's title bar, as Fleet does), and the island shadow, which Fleet
does not draw (the `shadow.*` keys are the popups').

- `zoomLevel` support: one scale factor applied to every metric, with the
  `zoom-in`, `zoom-out`, `reset-zoom` actions; this is also what lets probes
  at zoom 2.0 be compared with Oak at 1.0.
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
  `~/.config/JetBrains/Fleet/themes` and `~/.config/oak/themes`.
- Settings: `oak-settings` crate reading Fleet's keys (start with `theme`,
  `zoomLevel`, `editor.*` display keys, `files.*`), live reload.
- Probes: batches 1–3 in `docs/probe-plan.md`.

Theme groups: `tab`, `tooltip`, `scrollbar`, `focusOutline`, `island`,
`layout`, `separator`, `shadow`, `border`, `background`, `text`, `icon`.

### M2 Editor core (built 2026-11-02; the M2 scenes still to run against Fleet)

Everything Fleet's editor does to a plain text file. Built in one pass on
2026-11-02 against the batch 5 measurements (`docs/fleet-probe.md`, fourth
and fifth passes). What batch 5 did not measure is marked "estimate" in
`theme.rs::metrics` and in the probe record; the scenes `find`, `fold` and
`wrap` are written for the missing measurements and have not yet been run
against Fleet, so M2's end condition (every scene passing `compare.py`) is
still open. Left for later: the smooth caret (`editor.caretSmoothAnimation`
is read, not animated), an "in selection" button on the find bar (the mode
engages by itself when the bar opens on a multi-line selection; whether
Fleet also shows a toggle is unprobed), the breakpoint hint circle in the
hovered gutter (M8), the `editor.guides` ruler colour (no theme key names
it; it uses `editor.indentGuide`), and Fleet's Goto popup geometry for
`goto/line` (batch 6).

- Done: one document per open file (`crates/oak/src/document.rs`), shared by
  every tab and split that shows it; each view keeps its own cursor, folds
  and scroll, which follow the other views' edits through the buffer's edit
  log (`oak-text`'s `Cursor::sync`). Undo and the dirty state are the
  document's. The `splits` scene (Oak only) shows it.
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
- Done: the find and replace bar (`crates/oak/src/find.rs`; `find`,
  `replace`, `replace-one`, `replace-all`, `next-occurrence`,
  `prev-occurrence`) at batch 5's geometry, with the match-case, words and
  regex toggles (`inputField.*`, `toggleButton.*`), the match counter, the
  search kept to a multi-line selection, `editor.useSelectionForSearch`, and
  `$1` groups in regex replacements (`oak-text`'s `search` module, on the
  `regex` crate).
- Done: `expand-selection` (the camel hump, the word, then the enclosing
  syntax nodes), `shrink-selection` (back through the same steps),
  `editor/move-caret-to-matching-bracket`, folding (`editor/expand-folding`,
  `editor/collapse-folding`, the gutter chevrons on hover, the `{…}` mark
  from `editor.foldedMark.*`, `editor.showFoldings`) and the sticky lines
  (`editor.showStickyLines`, `stickyLinesMaxCount`), all from tree-sitter
  (`oak-syntax`'s `structure` module: `SyntaxTree::expand|shrink|folds|
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
  `.editorconfig` (`oak-settings`'s `editorconfig` module: indent style and
  size, end of line, trim and final newline), `trimTrailingSpacesOnSave`
  (caret rows kept), `autoSave` and `autoSave.delay`,
  `editor/change-line-endings` (also a click on the status bar's LF/CRLF),
  encoding and language in the status bar, `toggle-case`, `wrap-line`,
  `comment-block`, `editor/paste-from-history` (the editor's own copy
  history), `goto/line`, the big-file confirmation
  (`localBigFileMinimumThreshold`).
- Done: scratch files (`createScratchOnNewFile`, `--scratch`, in Oak's
  `scratches/` folder), `new-file` (a name prompt, the file created in the
  Files tree's folder), `files/rename`, `files/delete` (confirmed) on the
  tree's selection or the active tab's file.
- Probes: batch 5 (`surface`, `surface2`, run); `find`, `fold`, `wrap`
  (Oak's side run, Fleet's side and the comparison still to do).

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
the Settings tool's Plugins and AI pages, the modified-value mark and the
workspace scope, the snapshots tool (snapshots are
written by `local-history/make-snapshot`, not yet listed), the `showShortcuts`
overlay and `inputSequenceTimeoutMilliseconds` (gpui owns chord timing),
`restoreWorkspacesOnStartup` and the empty state, a notification's actions
and the `banner.*` bands (the on-disk change is a popup-coloured pill, built),
the Search tool's path scope (Fleet's query has a
`path`; Oak searches the whole workspace), and Goto's typed mode prefixes,
which no source shows.

- Done: the Goto panel (`crates/oak/src/goto.rs`): `goto/everything`,
  `goto/file`, `goto/symbol` and `show-file-symbols` (tree-sitter symbols,
  `oak-syntax`'s `symbols` module), `goto/action` (every action id Oak binds,
  with its chord), `goto/tool`, `goto/recent-file` (the recent files lead the
  empty list, as Fleet's `RecentNavigationEntity` rows do),
  `show-recent-workspaces`, `toggle-goto-preview` (a read-only editor under
  the list, shown by default as Fleet stores it), `goto_panel.showShortcuts`,
  `ff.previewAndPrioritiseFileSymbolsInGotoEverywhere`; Shift+Enter opens in
  a split, Tab switches lists. The matcher and the file index are
  `oak-search` (`fuzzy`, `files`), the index walked with the `ignore` crate.
- Done: `find-in-path` and `replace-in-path` as the Goto popup's Text
  Search tab (Fleet's own place for them): the match-case, words and regex
  toggles, the folder and file-mask fields, Fleet's limit of 100, one row
  per match with the matches in `search.match.*`, open documents searched
  with their unsaved text, Replace and Replace All through the documents
  (one undo step each) and on disk for the rest,
  `ff.searchInBigUnindexedFiles`; the Search tool in the panel
  (`crates/oak/src/search_tool.rs`, `search/jump-to-tool`) shows the same
  results as a tree. The engine is `oak-search`'s `grep` module.
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
  `~/.config/oak/keymaps/<name>.json`, merged over Fleet's defaults.
- Done: `local-history/make-snapshot` writes a copy under Oak's
  `local-history/` folder.
- Probes: batches 6, 7 and 8 as the `goto`, `popups`, `settings` and
  `dialogs` scenes, run on both editors (the `compare.py` pass is still to
  be run on the popups and settings pairs; the confirm dialog is still to
  be caught).
- Done: the Settings tool (`open-settings`): `settings_schema.rs` holds
  Fleet's pages (sections, groups, labels, keys, control kinds, defaults);
  `settings_view.rs` draws them at the measured values and writes each
  change to Oak's `settings.json` through `settings::set_value`.

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

### M4 Git (M; begun 2026-11-02)

Begun with what the shots on hand show (`docs/fleet-probe.md`, eighth
pass): the `oak-git` crate (branch and per-file status through the `git`
binary) and the Files tree's names coloured and badged by status. The
rest waits for batch 10.

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

### M5 Terminal (M)

- `terminal/new-tool`, `terminal/toggle`, `terminal/clear`, `terminal/copy`,
  `terminal/paste`, `select-all-terminal`, profiles (`terminal.profiles`,
  `terminal.profile.default`, `terminal/new-with-profile`), all `terminal.*`
  font and behaviour settings, `launchTerminalOnOpen`,
  `terminal.enableFocusEditorOnEsc`.
- `alacritty_terminal` (Apache-2.0, what Zed uses) for the PTY and grid; the
  32 `terminal.*` colour keys for painting.
- Probes: batch 9.

### M6 Run and problems (M)

- `.fleet/run.json` as the run configuration format, `run/run-in-context`,
  `run/rerun`, `run-and-debug` popup (`run.recentConfigurationsSize`),
  `stop-run-configurations`, Build Output tool, run icons in the gutter
  (`editor.renderRunInGutter`, `ff.runIconsAfterLineNumbers`).
- Problems: the file-status badge and problems widget (`problemsWidget.*`,
  `editor.showFileStatus`, `editor/go-to-next|previous-problem`,
  `editor.showTooltipOnHover`), fed first by tree-sitter syntax errors
  (`editor.markupWithoutSmartMode`), then by LSP diagnostics.
- Probes: batch 11.

### M7 Smart Mode via LSP (XL)

- Language server lifecycle behind the lightning icon (`smartMode.*`,
  `smart-mode-*` icons), per-language servers (rust-analyzer first),
  `toolchains`.
- Completion popup (`completion`, `smart-completion`, `complete-statement`,
  `completion.autoPopupDelay`, `completion.*`), `show-parameter-info`
  (`parameterInfo`), `show-documentation`, semantic highlighting
  (`editor.semanticHighlighting`), `go-to-declaration`,
  `go-to-type-declaration`, `find-usages`, `rename` (`rename.*`),
  `quick-fixes`, `apply-primary-quick-fix`, `show-quick-fixes-preview`
  (`quickFixes.preview.*`), `reformat-code`, `optimize-imports`,
  `editor.formatOnSave`, code vision (`editor.showCodeVision`), navigation
  bar (`editor.showNavBar`), `insert-snippet` (`snippet.*`, Fleet's
  `snippets.json` format), inline completion key handling.
- `back`, `forward`, `go-to-next|prev-history-item` navigation history.

### M8 Debugger via DAP (L)

- `debug/*` actions (14 bound), breakpoints (`breakpoints/*` icons,
  `editor.showBreakpoints`), Debug, Breakpoints and Debugger Console tools,
  floating debug panel (`editor.showFloatingDebugPanel`),
  `debugger/attach-to-process`.

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
