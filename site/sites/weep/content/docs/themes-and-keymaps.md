---
title: Themes and keymaps
description: WEEP's bundled themes, loading any Fleet-format theme, and Fleet-format keymaps, presets and overrides.
---

# Themes and keymaps

## Themes

WEEP bundles three themes, in Fleet's theme format, carrying the colours Fleet 1.48 renders
with: `dark`, `dark_purple` and `light`.

```
weep --theme dark_purple .
weep --theme ~/.config/JetBrains/Fleet/themes/my-theme.json .
```

Without `--theme`, the `theme` setting picks one — WEEP's, or Fleet's when WEEP's file does
not set it. Themes in `~/.config/weep/themes` and in Fleet's own
`~/.config/JetBrains/Fleet/themes` are found by name, and switching themes applies at once.

A Fleet theme is a palette plus two tables: `colors` (the interface, by component —
`tab.*`, `button.*`, `editor.*`) and `textAttributes` (the syntax colours). WEEP resolves
the palette and cascades attribute lookups the way Fleet does, and reads keys only by the
strings Fleet uses, so a theme written for Fleet needs no changes.

## Keymaps

Key bindings use Fleet's keymap format and Fleet's action ids. The defaults are Fleet's own
chords, per platform. Two ways to change them:

- **Overrides.** `~/.config/weep/keymap.json`, in the same format, is merged over the
  defaults.
- **Presets.** The `keymap` setting names a preset file in `~/.config/weep/keymaps/` —
  Fleet ships `intellij`, `vscode`, `sublimeText`, `eclipse`, `emacs` and others in this
  format — merged over Fleet's defaults.

An action WEEP does not implement is simply not bound; it is never renamed or mapped to
something else. `goto/action` lists every action WEEP has, with its chord.
