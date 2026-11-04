---
title: The window and its tools
description: WEEP's islands, tabs and splits, and the tools built so far — Files, Goto, Search, Git, History, Terminal, Run, Settings and Smart Mode.
---

# The window and its tools

![The WEEP window on its own repository, with the Files tool, a Rust file, the Git tool and Text Search results.](/assets/img/app/workspace.png)

Fleet's window: a toolbar on top, the editor island in the middle, left, bottom and right
panels around it, each with its own tab row. Splitters resize panels and their widths
persist; `+` adds a tool. The status bar shows path, caret, line endings, encoding and
language.

## Editor tabs and splits

Tabs show the modified state, close on hover, scroll when they overflow and drag to
reorder; a single click opens a preview tab (`editor.previewTab`). `split-right` puts a
second view beside the first. Every view of a file shares one document — an edit shows in
all of them, each keeping its own carets, folds and scroll.

## Files

Compact folders, git status colors and badges, multi-select, drag and drop, hidden files on
a toggle, inline rename, Fleet's right-click menu. `files.keyboardNavigation` gives Fleet's
highlight mode.

## Goto and Text Search

`goto/everything` opens Fleet's four tabs — Goto, Actions, Tools, Text Search — with a
preview under the list: files, symbols in the current file, every bound action with its
chord, recent files. `find-in-path` opens the Text Search tab: match case, whole words,
regex, folder and file mask, Replace and Replace All through open documents and on disk. The
Search tool shows the same results as a tree.

## Git and History

Status in the tree and the gutter; the Git tool with a commit field, changed files with
staging and unsynced commits with Push; History; blame (`git/toggle-blame`); and the Changed
Files tab, git's hunks side by side with staging and Revert. The toolbar's branch tag opens
Fleet's branch menu.

## Terminal

A shell on a real PTY with Fleet's `terminal.profiles`, in tabs, with selection,
scrollback, paste and the 256-color palette.

## Run and problems

`.fleet/run.json` in Fleet's format: `command` and `cargo` rows, environment files, a
gutter run icon, and a Run tool tab per configuration with its command line, output and a
rerun button. Errors are underlined in the editor and listed in the problems widget.

## Smart Mode

The toolbar's lightning starts a language server — today rust-analyzer, on `PATH` or
through rustup. With it on: diagnostics and quick fixes, semantic highlighting, completion,
parameter info, hover documentation, go to declaration and type declaration, find usages,
back and forward, rename, optimize imports, reformat and format on save. Inlay hints, code
vision and inline rename are not built.

## Settings

`open-settings` opens Fleet's Settings pages as a tab; every row is bound to its
`settings.json` key and writes it. See [Settings](/docs/settings/).
