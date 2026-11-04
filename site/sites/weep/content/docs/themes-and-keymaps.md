---
title: Themes and keymaps
description: WEEP's bundled themes, loading any Fleet-format theme, and Fleet-format keymaps, presets and overrides.
---

# Themes and keymaps

## Themes

Three bundled themes in Fleet's format, carrying the colors Fleet 1.48 renders with:
`dark`, `dark_purple` and `light`.

```
weep --theme dark_purple .
weep --theme ~/.config/JetBrains/Fleet/themes/my-theme.json .
```

Without `--theme`, the `theme` setting picks one — WEEP's, else Fleet's. Themes in
`~/.config/weep/themes` and in Fleet's own folder are found by name, and switching applies
at once.

A Fleet theme is a palette plus two tables: `colors` (the interface — `tab.*`, `button.*`,
`editor.*`) and `textAttributes` (syntax). WEEP resolves the palette and cascades lookups as
Fleet does, reading keys only by Fleet's strings, so a Fleet theme needs no changes.

## Keymaps

Fleet's keymap format and action ids, defaulting to Fleet's chords per platform. Two ways
to change them:

- **Overrides.** `~/.config/weep/keymap.json`, merged over the defaults.
- **Presets.** The `keymap` setting names a file in `~/.config/weep/keymaps/` — Fleet ships
  `intellij`, `vscode`, `sublimeText`, `eclipse`, `emacs` and others — merged over Fleet's
  defaults.

An action WEEP lacks is simply not bound, never renamed. `goto/action` lists every action it
has, with its chord.
