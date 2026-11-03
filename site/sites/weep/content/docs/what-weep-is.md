---
title: What WEEP is
description: WEEP is a native Rust/GPUI code editor that clones JetBrains Fleet's interface and file formats, measured against a running copy, with language servers in place of Fleet's backend.
---

# What WEEP is

WEEP is a code editor written in Rust on [GPUI](https://gpui.rs), the GPU-accelerated
interface toolkit from the Zed editor. Its goal is a faithful clone of JetBrains Fleet 1.48:
the same islands layout, the same tools, the same sizes and colours, and the same file
formats, in a single native binary.

## Fleet, without the parts that need a backend first

WEEP's first goal is Fleet as it behaves **without Smart Mode**: the shell, the editor,
Files, Search, Goto, Git, Terminal, Run, Settings, themes and keymaps. Smart Mode —
completion, navigation, refactorings, debugging — comes after, fed by
[LSP](https://microsoft.github.io/language-server-protocol/) and DAP servers instead of
Fleet's IntelliJ backend. Fleet's own Rust Smart Mode turned out to run rust-analyzer as
well, so for Rust the two editors answer from the same server.

## What carries over from Fleet

- **Themes.** A Fleet theme JSON loads unchanged; the three bundled themes are value tables
  in Fleet's format carrying the colours Fleet 1.48 renders with.
- **Keymaps.** Fleet's keymap format and action ids, with Fleet's defaults; a Fleet preset or
  your own overrides load as they are.
- **Settings.** WEEP reads Fleet's keys, and falls back to Fleet's own `settings.json`, so a
  Fleet setup carries over without copying anything.
- **Run configurations.** `.fleet/run.json`, with Fleet's field names.

## Deliberate departures

Kept short on purpose; everything not listed here follows Fleet.

| Fleet | WEEP | Why |
|---|---|---|
| TextMate grammars | tree-sitter | Same colours through Fleet's `textAttributes` keys, plus expand/shrink selection, folding and bracket matching without a backend |
| IntelliJ backend | LSP and DAP | Smart Mode becomes "a language server is attached"; the lightning, its states and colours stay Fleet's |
| Chromium (JCEF) Markdown preview | Painted natively | No embedded browser |
| JVM, account, telemetry | None | Nothing phones home and nothing needs a login |

## What WEEP is not

Collaboration, remote workspaces, the JetBrains account, Toolbox and AI are out of scope.
Vim mode, Docker, SSH and WSL workspaces, the spellchecker and plugin management are "later
or never".

## Clean room

Nothing is taken from the Fleet distribution except what is openly licensed: the fonts
(Inter and JetBrains Mono, both under the SIL Open Font License). Icons are JetBrains'
Apache-licensed IntelliJ Platform `expui` set or WEEP's own drawings. Sizes and colours are
measured from screenshots of a running Fleet and read from Fleet's theme files —
see [how WEEP is measured](/docs/measured/).
