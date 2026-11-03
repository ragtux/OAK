# WEEP

Native Rust/GPUI code editor in the shape of JetBrains Fleet.

## Ground rules

- Fleet is the reference. Sizes and colours come from measurements and from Fleet's own
  theme files, never from memory; `docs/fleet-probe.md` records each value's source and
  which are still estimates. When WEEP and Fleet disagree, measure again and fix the constant
  in `crates/weep/src/theme.rs` (`metrics`).
- Fleet's file formats are WEEP's: themes (`weep-theme`) and keymaps (`weep-keymap`) use Fleet's
  keys and action ids verbatim, so Fleet files load unchanged. Add a key only with the string
  Fleet uses.
- Clean room on the binary: nothing from the Fleet distribution except what is openly
  licensed (the OFL fonts). Icons are JetBrains' Apache-licensed expui set or WEEP's own
  drawings (`crates/weep/assets/icons/*/LICENSE.txt`).
- One text model (`weep-text`) and one highlighter (`weep-syntax`), both free of gpui and tested
  on their own.

## Layout

```
crates/weep          GPUI app (bin): main, workspace (panels, splits, tabs, status bar, prompts, autosave; workspace/workbench holds the M3 tools: Goto, Search, menus, dialogs, notifications, recent workspaces), toolbar (with the Windows caption), file_tree (multi-select, drag and drop, hidden files, the right-click menu), document (one per file, shared by its views: buffer, syntax tree, folds, problems, indent, save), editor (carets, folds and a display map on a document; the element paints it, with the run icons, the error underlines, the status badge and the problems widget), find (the find/replace bar), goto (the Goto panel with its preview), search_tool (find/replace in files), git_tool (the Git tool: commit field, changed files with staging, filter, Push, unsynced commits; the History tool with its chips), diff_view (the Changed Files tab: cards per changed file with git's hunks side by side, staging, Revert; `diff/two-files` and `diff/with-clipboard` reuse it), settings_schema + settings_view (Fleet's Settings pages as data and as a tab), terminal (the Terminal tool: a view per shell painting weep-terminal's grid, keys and the text input to the PTY; workspace/terminals holds its tabs, profiles, menu and actions), run_tool (the run tool: a tab per started configuration, its command line, output and rerun button) + run_popup (the Run & Debug popup; workspace/runs holds the actions, run.json editing and the recents), menu (context and main menus, the branch tag's), dialog (Fleet's confirm dialog and buttons), notifications (toasts and the Notifications popup), input (a single-line field), popup (prompts under the toolbar), theme (Metrics = sizes x zoom), settings (globals, live reload, zoom), layout (panels, recent files, recent workspaces), tooltip, scrollbar, keymap (the action table: bindings, `goto/action`, menus), assets
crates/weep-text     Buffer (ropey) with an edit log and the document's Indent, Cursor + TextEditor (one or more selections, undo grouping, every edit command, auto-pairs and the indenter), analysis (identifier occurrences, bracket match, indent guides), search (the find bar's queries, on regex), indent (detection)
crates/weep-syntax   tree-sitter grammars (rust, json, toml, markdown) -> Fleet textAttributes keys; structure (SyntaxTree: expand/shrink, folds, bracket pairs, errors and problems with messages); symbols (the file outline for goto/symbol)
crates/weep-theme    Fleet theme JSON: palette resolution, typed Color, cascading attribute lookup
crates/weep-keymap   Fleet keymap JSON: per-platform chord resolution, gpui keystroke spelling
crates/weep-settings Fleet settings.json: WEEP's file over Fleet's, typed accessors with Fleet's defaults, a change watcher; editorconfig (.editorconfig lookup); workbench (the M3 keys)
crates/weep-search   Workspace search without gpui: fuzzy (the Goto matcher), files (the file index, walked with `ignore`), grep (find/replace in files with Fleet's query fields)
crates/weep-terminal A shell on a PTY through `alacritty_terminal` (no gpui): profiles (Fleet's `terminal.profiles` rows and the machine's shells), key encoding (xterm sequences; printable keys stay with the text input), the 256-colour table, and Terminal (spawn, write, paste, resize, scroll, selection, a Snapshot of cells with resolved colours); tested with a real shell
crates/weep-run      Fleet's `.fleet/run.json` as data (RunFile, RunConfig with Fleet's field names, the `"type"` line of each row for the gutter), the Command a row resolves to (`command` and `cargo` rows, environment files, a typed shell line) and Run (a child process whose lines and exit arrive through a channel); tested with real processes
crates/weep-lsp      A language server on its pipes (no gpui), Fleet's Smart Mode (M7): transport (JSON-RPC framing on two threads), client (the handshake, typed requests answered through `poll`, the server's own requests, progress, shutdown), position (UTF-8/16/32 columns against char offsets), edits (text and workspace edits as char-range changes), snippet (an LSP snippet as plain text), semantic (tokens decoded and mapped onto Fleet's textAttributes keys), servers (rust-analyzer on PATH or through rustup), markup (Markdown as plain lines); tested against the real rust-analyzer. The app side is `workspace/smart.rs` (the servers, document sync, diagnostics, tokens, every M7 action, the navigation history, the usages popup) and `editor/smart.rs` (the completion, parameter info and documentation popups); `list_popup` is the Goto-framed list the usages and multiple declarations use
crates/weep-git      Git through the `git` binary (no gpui): branch and branches, per-file status with the index state, stage/unstage/commit, line status against HEAD, a file's diff and HEAD's text, history, blame, fetch, push, pull, revert and ahead/behind (Repo, Status, StatusEntry, Hunk, Commit, BlameLine, Branch); a Myers line diff and git's hunks as side-by-side rows (LineOp), dates as Fleet prints them; the parsers are tested without git
docs/                fleet-probe.md (measurements and provenance), roadmap.md (milestones), probe-plan.md (method and harnesses)
tools/fleet-probe    scripts: inventory Fleet's actions/settings/theme keys, coverage vs WEEP, extract themes, measure screenshots
```

## Commands

On NixOS run the gpui commands inside `nix-shell` (`shell.nix` supplies X11, Wayland and Vulkan).
On Windows the MSVC Build Tools suffice; when the repo sits on a network share, set
`CARGO_TARGET_DIR` to a local disk (the probe driver reads `WEEP_EXE` for the binary).
On macOS Xcode (or its command line tools) suffices; the same applies to a repo on a network
volume (`/Volumes/...`), e.g. `CARGO_TARGET_DIR=~/Library/Caches/weep-target`.
gpui is a git dependency pinned to a Zed release commit (see `Cargo.toml`); bump it one release
at a time and run on a real GPU before committing.

```
cargo test --workspace --exclude weep      # library tests (fast)
cargo check -p weep                        # the app (pulls gpui, slow the first time)
cargo run --release -p weep -- <path>      # open a folder or a file
cargo run --release -p weep -- --theme dark_purple --zoom 1.5 <path>
cargo fmt --all && cargo clippy --workspace --all-targets
```

Settings come from `~/.config/weep/settings.json` (`%APPDATA%\weep` on Windows) over Fleet's own
file; `WEEP_CONFIG_DIR` relocates WEEP's folder (the probes use a scratch one). The probe driver
`tools/fleet-probe/winprobe.py` sends real input to the foreground window: run it in the
foreground only, never as a background task.

## Conventions

- Positions in `weep-text` are char offsets; the editor element maps them to display bytes per line.
- Theme tokens in `crates/weep/src/theme.rs` are named after the Fleet key they read, in the field's
  comment.
- Fleet action ids map to gpui actions in `crates/weep/src/keymap.rs`; an action WEEP lacks is simply
  not bound, never renamed.
