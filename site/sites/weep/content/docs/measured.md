---
title: How WEEP is measured
description: How WEEP's sizes and colors are taken from a running Fleet — probe scenes, screenshot comparison, Fleet's own theme files — and recorded with their provenance.
---

# How WEEP is measured

Sizes and colors come from measurements and from Fleet's own theme files, never from
memory. When the two disagree, Fleet is measured again.

## Probes

A probe is a scripted scene — open this folder, show that popup — played into both with
real input. Both windows are captured at the same zoom and `tools/fleet-probe/compare.py`
compares the screenshots: island, tab and status-bar edges within one logical pixel, colors
exact. Twelve batches so far, grouped by milestone.

## Provenance

Constants live in one table (`metrics` in `crates/weep/src/theme.rs`), scaled by zoom, and
each is recorded in
[`docs/fleet-probe.md`](https://github.com/ragtux/WEEP/blob/main/docs/fleet-probe.md) with
its source — a probe screenshot, a Fleet file — or marked an estimate. Theme tokens are
named after the Fleet key they read.

## Inventory and coverage

`inventory.py` reads the tables Fleet ships — keymap, actions, settings, theme keys, icons
— and `coverage.py` reports how much of each WEEP covers. Those are the numbers on the
[roadmap](/docs/roadmap/).

## Clean room

The probes look at Fleet; they take nothing from it. Only the openly licensed fonts
ship.
