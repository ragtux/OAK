---
title: Roadmap
description: WEEP's milestones toward a faithful Fleet clone — the shell, the editor, the workbench tools, Git, Terminal, Run, Smart Mode and the debugger — and where each stands.
---

# Roadmap

A milestone ends when its probe scenes pass the comparison against Fleet, the coverage
script shows every action, setting and theme key it claims, and each new constant has its
source recorded. Full plan:
[`docs/roadmap.md`](https://github.com/ragtux/WEEP/blob/main/docs/roadmap.md).

| Milestone | Covers | Status |
|---|---|---|
| M0 Scaffold | Toolbar, Files, editor with tabs, status bar, Fleet's theme and keymap formats | Done |
| M1 Shell fidelity | Zoom, panels and splitters, tab behavior, splits, tooltips, scrollbars, live settings, the light theme | Done |
| M2 Editor core | Multiple carets, find and replace, folding, soft wrap, the indenter, `.editorconfig`, autosave | Built; three scenes still to run on Fleet |
| M3 Workbench tools | Goto, Text Search, Files tool, menus, dialogs, notifications, Settings, keymap presets | Built |
| M4 Git | Status, gutter, Git and History tools, blame, Changed Files, push, pull, revert | Built |
| M5 Terminal | A PTY per tab, Fleet's profiles, keys, selection, colors | Built |
| M6 Run and problems | `.fleet/run.json`, the Run tool, the problems widget | Built |
| M7 Smart Mode via LSP | Diagnostics, completion, navigation, rename, quick fixes, formatting | First pass built |
| M8 Debugger via DAP | Breakpoints, the debug tools, the floating debug panel | Not started |

## Coverage against Fleet 1.48.261

Fleet describes most of its surface as data; a script counts it. As of 2026-11-03:

| Table | Fleet | WEEP covers |
|---|---|---|
| Action ids with a default chord | 203 | 145 |
| `settings.json` keys | 430 | 104 |
| Theme `colors` keys | 684 | 199 |
| Theme `textAttributes` keys | 157 | 43 |

## Later or never

Vim mode, the spellchecker, Docker, SSH and WSL workspaces, collaboration, AI, Toolbox,
plugin management and surveys.
