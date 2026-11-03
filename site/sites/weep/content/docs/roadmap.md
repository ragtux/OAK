---
title: Roadmap
description: WEEP's milestones toward a faithful Fleet clone — the shell, the editor, the workbench tools, Git, Terminal, Run, Smart Mode and the debugger — and where each stands.
---

# Roadmap

WEEP is built in milestones. Each ends when its probe scenes pass the comparison against
Fleet (edges within one logical pixel, colours exactly the theme's), the coverage script
shows every action, settings key and theme key it claims, and the probe record says where
each new constant came from. The full plan, with every item, is
[`docs/roadmap.md`](https://github.com/ragtux/WEEP/blob/main/docs/roadmap.md) in the
repository.

| Milestone | Covers | Status |
|---|---|---|
| M0 Scaffold | Toolbar, Files, editor with tabs, status bar, Fleet's theme and keymap formats | Done |
| M1 Shell fidelity | Zoom, panels and splitters, tab behaviour, splits, tooltips, scrollbars, live settings, the light theme | Done |
| M2 Editor core | Multiple carets, find and replace, folding, soft wrap, the indenter, `.editorconfig`, autosave | Built; three scenes still to run on Fleet |
| M3 Workbench tools | Goto, Text Search, Files tool, menus, dialogs, notifications, Settings, keymap presets | Built |
| M4 Git | Status, gutter, Git and History tools, blame, Changed Files, push, pull, revert | Built |
| M5 Terminal | A PTY per tab, Fleet's profiles, keys, selection, colours | Built |
| M6 Run and problems | `.fleet/run.json`, the Run tool, the problems widget | Built |
| M7 Smart Mode via LSP | Diagnostics, completion, navigation, rename, quick fixes, formatting | First pass built |
| M8 Debugger via DAP | Breakpoints, the debug tools, the floating debug panel | Not started |

## Coverage against Fleet 1.48.261

Fleet's distribution describes most of its own surface as data, and an inventory script
counts it. As of 2026-11-03:

| Table | Fleet | WEEP covers |
|---|---|---|
| Action ids with a default chord | 203 | 145 |
| `settings.json` keys | 430 | 104 |
| Theme `colors` keys | 684 | 199 |
| Theme `textAttributes` keys | 157 | 43 |

## Later or never

Vim mode, the spellchecker, Docker, SSH and WSL workspaces, collaboration, AI, Toolbox,
plugin management and surveys.
