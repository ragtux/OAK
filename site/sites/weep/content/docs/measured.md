---
title: How WEEP is measured
description: How WEEP's sizes and colours are taken from a running Fleet — probe scenes, screenshot comparison, Fleet's own theme files — and recorded with their provenance.
---

# How WEEP is measured

WEEP's rule is that sizes and colours come from measurements and from Fleet's own theme
files, never from memory. When WEEP and Fleet disagree, Fleet is measured again and the
constant is fixed.

## Probes

A probe is a scripted scene — open this folder, open that file, show this popup — played
into Fleet and into WEEP with real input. Both windows are captured at the same zoom, and
`tools/fleet-probe/compare.py` compares the screenshots: the edges of islands, tab rows and
status bars must sit within one logical pixel of Fleet's, and colours must be the theme's
exact values. Scenes are grouped in batches, one or more per milestone; twelve batches have
been written so far.

## Provenance

Every constant lives in one table in the app (`metrics` in `crates/weep/src/theme.rs`),
scaled by the zoom level, and every value is recorded in
[`docs/fleet-probe.md`](https://github.com/ragtux/WEEP/blob/main/docs/fleet-probe.md) with
where it came from — a probe screenshot, one of Fleet's files — or marked as an estimate
still to be measured. Theme tokens in the code are named after the Fleet key they read.

## Inventory and coverage

`tools/fleet-probe/inventory.py` reads the tables Fleet ships — the default keymap, the
action and settings documentation, the theme keys, the icons — and
`tools/fleet-probe/coverage.py` reports how much of each WEEP covers. Those are the numbers
on the [roadmap](/docs/roadmap/).

## Clean room

The probes look at Fleet; they do not take from it. Nothing from the Fleet distribution
ships in WEEP except the openly licensed fonts.
