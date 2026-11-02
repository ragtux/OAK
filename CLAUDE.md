# Oak

Native Rust/GPUI code editor in the shape of JetBrains Fleet.

## Ground rules

- Fleet is the reference. Sizes and colours come from measurements and from Fleet's own
  theme files, never from memory; `docs/fleet-probe.md` records each value's source and
  which are still estimates. When Oak and Fleet disagree, measure again and fix the constant
  in `crates/oak/src/theme.rs` (`metrics`).
- Fleet's file formats are Oak's: themes (`oak-theme`) and keymaps (`oak-keymap`) use Fleet's
  keys and action ids verbatim, so Fleet files load unchanged. Add a key only with the string
  Fleet uses.
- Clean room on the binary: nothing from the Fleet distribution except what is openly
  licensed (the OFL fonts). Icons are JetBrains' Apache-licensed expui set or Oak's own
  drawings (`crates/oak/assets/icons/*/LICENSE.txt`).
- One text model (`oak-text`) and one highlighter (`oak-syntax`), both free of gpui and tested
  on their own.

## Layout

```
crates/oak          GPUI app (bin): main, workspace (panels, splits, tabs, status bar), toolbar (with the Windows caption), file_tree, document (one per file, shared by its views), editor (a cursor on a document), theme (Metrics = sizes x zoom), settings (globals, live reload, zoom), layout, tooltip, scrollbar, keymap, assets
crates/oak-text     Buffer (ropey) and TextEditor: selection, undo grouping, every edit command
crates/oak-syntax   tree-sitter grammars (rust, json, toml, markdown) -> Fleet textAttributes keys
crates/oak-theme    Fleet theme JSON: palette resolution, typed Color, cascading attribute lookup
crates/oak-keymap   Fleet keymap JSON: per-platform chord resolution, gpui keystroke spelling
crates/oak-settings Fleet settings.json: Oak's file over Fleet's, typed accessors with Fleet's defaults, a change watcher
docs/               fleet-probe.md (measurements and provenance), roadmap.md (milestones), probe-plan.md (method and harnesses)
tools/fleet-probe   scripts: inventory Fleet's actions/settings/theme keys, coverage vs Oak, extract themes, measure screenshots
```

## Commands

On NixOS run the gpui commands inside `nix-shell` (`shell.nix` supplies X11, Wayland and Vulkan).
On Windows the MSVC Build Tools suffice; when the repo sits on a network share, set
`CARGO_TARGET_DIR` to a local disk (the probe driver reads `OAK_EXE` for the binary).
gpui is a git dependency pinned to a Zed release commit (see `Cargo.toml`); bump it one release
at a time and run on a real GPU before committing.

```
cargo test --workspace --exclude oak      # library tests (fast)
cargo check -p oak                        # the app (pulls gpui, slow the first time)
cargo run --release -p oak -- <path>      # open a folder or a file
cargo run --release -p oak -- --theme dark_purple --zoom 1.5 <path>
cargo fmt --all && cargo clippy --workspace --all-targets
```

Settings come from `~/.config/oak/settings.json` (`%APPDATA%\oak` on Windows) over Fleet's own
file; `OAK_CONFIG_DIR` relocates Oak's folder (the probes use a scratch one). The probe driver
`tools/fleet-probe/winprobe.py` sends real input to the foreground window: run it in the
foreground only, never as a background task.

## Conventions

- Positions in `oak-text` are char offsets; the editor element maps them to display bytes per line.
- Theme tokens in `crates/oak/src/theme.rs` are named after the Fleet key they read, in the field's
  comment.
- Fleet action ids map to gpui actions in `crates/oak/src/keymap.rs`; an action Oak lacks is simply
  not bound, never renamed.
