---
title: What WEEP is
description: WEEP is a native Rust/GPUI code editor that clones JetBrains Fleet's interface and file formats, measured against a running copy, with language servers in place of Fleet's backend.
---

# What WEEP is

A code editor in Rust on [GPUI](https://gpui.rs), Zed's GPU-accelerated toolkit. The goal
is a faithful clone of JetBrains Fleet 1.48 — the same islands, tools, sizes, colors and
file formats — in one native binary.

## Fleet, without the parts that need a backend first

First goal: Fleet **without Smart Mode** — the shell, the editor, Files, Search, Goto, Git,
Terminal, Run, Settings, themes and keymaps. Smart Mode comes after, over
[LSP](https://microsoft.github.io/language-server-protocol/) and DAP instead of Fleet's
IntelliJ backend. Fleet's Rust Smart Mode runs rust-analyzer too, so for Rust both editors
answer from the same server.

## What carries over from Fleet

- **Themes.** A Fleet theme JSON loads unchanged; the three bundled ones carry Fleet 1.48's
  colors.
- **Keymaps.** Fleet's format, action ids and defaults; Fleet presets and your overrides load
  as they are.
- **Settings.** Fleet's keys, falling back to Fleet's own `settings.json`, so a Fleet setup
  carries over.
- **Run configurations.** `.fleet/run.json`, with Fleet's field names.

## Deliberate departures

Everything not listed here follows Fleet.

| Fleet | WEEP | Why |
|---|---|---|
| TextMate grammars | tree-sitter | Same colors through Fleet's `textAttributes` keys, plus expand/shrink selection, folding and bracket matching without a backend |
| IntelliJ backend | LSP and DAP | Smart Mode becomes "a language server is attached"; the lightning, its states and colors stay Fleet's |
| Chromium (JCEF) Markdown preview | Painted natively | No embedded browser |
| JVM, account, telemetry | None | Nothing phones home and nothing needs a login |

## What WEEP is not

Out of scope: collaboration, remote workspaces, the JetBrains account, Toolbox, AI. Later
or never: Vim mode, Docker, SSH and WSL workspaces, the spellchecker, plugin management.

## Clean room

Nothing comes from the Fleet distribution but the openly licensed fonts (Inter and
JetBrains Mono, SIL OFL). Icons are JetBrains' Apache-licensed `expui` set or WEEP's own.
Sizes and colors come from screenshots of a running Fleet and from Fleet's theme files — see
[how WEEP is measured](/docs/measured/).
