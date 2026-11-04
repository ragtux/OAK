---
title: Settings
description: Where WEEP's settings.json lives, how it layers over Fleet's own settings file, and how zoom works.
---

# Settings

WEEP reads `settings.json` with Fleet's keys. There are two files, read in order:

1. **WEEP's own**, `~/.config/weep/settings.json` (`%APPDATA%\weep\settings.json` on
   Windows; `$XDG_CONFIG_HOME/weep` when that is set).
2. **Fleet's**, `~/.config/JetBrains/Fleet/settings.json` (`%APPDATA%\JetBrains\Fleet`
   on Windows), if you have Fleet installed.

WEEP's file wins; a key only Fleet has still applies; anything in neither takes Fleet's
default. WEEP never writes Fleet's file, and picks up changes to either while running.

`WEEP_CONFIG_DIR` relocates WEEP's whole folder.

## The Settings tab

`open-settings` shows Fleet's Settings pages as an editor tab. Each control is bound to its
key, shows Fleet's default, marks a change and writes it to WEEP's `settings.json`. The
Plugins and AI pages and the workspace scope are not built.

## Zoom

`zoomLevel` scales every size by one factor. `zoom-in`, `zoom-out`, `reset-zoom` and
`--zoom` change it.

## What else lives in the folder

| File | What it holds |
|---|---|
| `settings.json` | Your settings, in Fleet's keys |
| `keymap.json` | Your key bindings, over Fleet's defaults |
| `keymaps/` | Keymap presets named by the `keymap` setting |
| `themes/` | Fleet-format themes |
| `layout.json` | Panel sizes, recent files and recent workspaces |
