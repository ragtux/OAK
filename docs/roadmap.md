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

| Table | Count | Oak covers (2026-11-02) |
| --- | --- | --- |
| Action ids with a default chord (`fleet/keymap/fleet.json`) | 203 | 77 |
| Action ids described in plugin `documentation.yaml` files | 204 (385 with the above) | – |
| `settings.json` keys described in `documentation.yaml` files | 430 | 15 |
| Theme `colors` keys | 684 | 71 |
| Theme `textAttributes` keys | 157 | 28 |
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

### M2 Editor core (L, in progress)

Everything Fleet's editor does to a plain text file.

- Done 2026-11-02: one document per open file (`crates/oak/src/document.rs`),
  shared by every tab and split that shows it; each view keeps its own
  cursor, which follows the other views' edits through the buffer's edit
  log (`oak-text`'s `Cursor::sync`). Undo and the dirty state are the
  document's. The `splits` scene (Oak only) shows it.

- Done 2026-11-02 (batch 5, `surface` and `surface2` scenes): the editor
  surface measured and drawn as Fleet does: rounded selection rows with the
  newline cell and the empty-row stub, the current-line wash under a
  selection, `identifier.underCaret` boxes on every whole-word occurrence,
  `editor.brace.match` on the bracket pair, indent guides with the caret
  block's in `.current`, the caret-line and search-match scrollbar marks
  (`oak-text`'s `analysis` module, gpui-free). The find-match boxes paint
  from `EditorView::search`, which the find bar will fill.
- Multiple carets (`editor/add-caret-above|below`,
  `editor/add-carets-to-ends-of-selected-lines`, `select-next-occurrence`,
  `select-all-occurrences`, `unselect-occurrence`).
- Find and replace bar (`find`, `replace`, `replace-one`, `replace-all`,
  `next-occurrence`, `prev-occurrence`) with the match-case, words, regex
  and in-selection toggles (`inputField.*`, `toggleButton.*`).
- `expand-selection`, `shrink-selection`, `editor/move-caret-to-matching-bracket`,
  bracket matching (`editor.brace.*`), folding (`editor/expand-folding`,
  `editor/collapse-folding`, `editor.showFoldings`), from tree-sitter.
- Rendering options from settings: soft wrap (`editor.softWrapEnabled`,
  `editor.guideToSoftWrapBy`), `editor.guides`, indent guides, whitespace,
  line numbers on/relative/off, caret shape and blink, smooth caret,
  active-line highlight, sticky lines (`editor.showStickyLines`), scrollbar
  marks, `editor.distractionFreeMode`, `editor.centerEditorHorizontally`,
  `editor.lineSpacing`, `editor.fontFamily|Size|Weight|Features|ligatures`.
- Editing behaviour: auto-indent (`enableIndenter`), auto-pairs,
  `editor.tabSize`, `editor.indentStyle`, `editor.autoDetectIndentation`,
  EditorConfig, `trimTrailingSpacesOnSave`, `autoSave` and `autoSave.delay`,
  `editor/change-line-endings`, encoding and language in the status bar,
  `toggle-case`, `wrap-line`, `comment-block`, `editor/paste-from-history`,
  `goto/line`, big-file confirmation (`localBigFileMinimumThreshold`).
- Scratch files (`createScratchOnNewFile`, `--scratch`), `new-file`,
  `files/rename`, `files/delete` from the editor.
- Probes: batch 5.

Theme groups: `editor` (43), `highlight`, `inputField`, `toggleButton`,
`ghostButton`, most `textAttributes`.

### M3 Workbench tools (L)

The tools Fleet has without a backend, each in Fleet's island.

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
- Settings tool (`open-settings`, `edit-settings-file`, `edit-keymap-file`):
  the searchable settings UI built from the 430 documented keys, using
  `tileButton`, `checkbox`, `combobox`, `dropdown`, `button` (108 keys),
  `pillButton`, `link`.
- Keymap presets: load any of Fleet's eight preset files from the user's
  config; `keymap` setting; `showShortcuts` overlay;
  `inputSequenceTimeoutMilliseconds` for chords.
- Local history (`local-history/make-snapshot`, snapshots tool).
- Probes: batches 6–8.

### M4 Git (M)

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
