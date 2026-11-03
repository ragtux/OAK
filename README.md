# WEEP

A native code editor in Rust on [GPUI](https://gpui.rs), built to look and
feel like [JetBrains Fleet](https://www.jetbrains.com/fleet/): the islands
layout, Fleet's theme and keymap file formats, its fonts, its colours and
its sizes, read off a running copy rather than guessed at.

WEEP is an independent project and is not affiliated with JetBrains.

## Status

A scaffold. The window opens a folder in the Fleet shell: the toolbar, the
Files island (a tree with compact folders and keyboard navigation), the
editor island with tabs, and the status bar. The editor opens, edits,
saves, undoes and highlights Rust, JSON, TOML and Markdown through
tree-sitter, painted in the colours of Fleet's own theme files. There is no
language intelligence, no search, no terminal and no settings UI yet.

## Build and run

```
nix-shell --run 'cargo run --release -p weep -- <folder-or-file>'   # NixOS: the shell supplies X11, Wayland and Vulkan
cargo run --release -p weep -- <folder-or-file>                     # elsewhere
cargo test --workspace --exclude weep                               # the library crates, no GPU needed
```

`weep --theme dark|dark_purple|light` picks a bundled theme; `--theme
path.json` loads any Fleet-format theme, including one from
`~/.config/JetBrains/Fleet/themes`. With no `--theme`, WEEP uses the theme
named in Fleet's own `settings.json` when there is one.

Keys follow Fleet's defaults (`crates/weep/assets/keymap/default.json`, in
Fleet's keymap format); `~/.config/weep/keymap.json` in the same format
overrides them.

## Layout

```
crates/weep          the GPUI app: toolbar, file tree, editor, tabs, status bar, theme and keymap loading
crates/weep-text     rope buffer, cursor and selection, undo, edit commands (no gpui)
crates/weep-syntax   tree-sitter highlighting mapped to Fleet's textAttributes keys (no gpui)
crates/weep-theme    Fleet-format theme files: parsing and palette resolution (no gpui)
crates/weep-keymap   Fleet-format keymap files and their gpui keystroke spelling (no gpui)
docs/fleet-probe.md  what was measured on Fleet, where each value came from, and what is still estimated
docs/roadmap.md      the milestones to a faithful Fleet clone, and what each one covers
docs/probe-plan.md   how Fleet is measured and the harnesses that make a probe repeatable
tools/fleet-probe    the scripts those measurements were made with
```

## Licences

WEEP is MIT. It bundles Inter and JetBrains Mono (SIL OFL) and icons from
JetBrains' IntelliJ Platform `expui` set (Apache 2.0, header in every file);
WEEP's own icons are MIT. The bundled themes are value tables in Fleet's file
format carrying the colours Fleet 1.48 renders with; see
`docs/fleet-probe.md`.
