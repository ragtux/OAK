---
title: Build and run
description: Building WEEP from source with cargo on Linux, NixOS, Windows and macOS, and the weep command line.
---

# Build and run

No release builds yet. WEEP builds with stable Rust (1.85 or newer); `rust-toolchain.toml`
picks it, with rustfmt and clippy.

```
git clone https://github.com/ragtux/WEEP
cd WEEP
cargo run --release -p weep -- <folder-or-file>
```

The first build pulls GPUI and takes a while.

## Per platform

| Platform | What you need |
|---|---|
| Linux | A Vulkan-capable driver; X11 and Wayland both work |
| NixOS | Run inside `nix-shell`, which supplies X11, Wayland and Vulkan: `nix-shell --run 'cargo run --release -p weep -- .'` |
| Windows | The MSVC Build Tools |
| macOS | Xcode or its command line tools |

> **Tip** On a network share or volume, point `CARGO_TARGET_DIR` at a local disk.

## The command line

```
weep [path ...] [--theme dark|dark_purple|light|<file.json>] [--zoom 0.7..2.0] [--size WxH] [--scratch]
```

| Flag | Effect |
|---|---|
| `path` | A folder to open as the workspace, or files to open in it |
| `--theme` | A bundled theme by name, or any Fleet-format theme file |
| `--zoom` | The interface scale, as Fleet's `zoomLevel` |
| `--size` | The window's size in logical pixels, e.g. `1400x900` |
| `--scratch` | Open a new scratch file |

Without `--theme`, the theme comes from WEEP's settings, or Fleet's.

## Running the tests

```
cargo test --workspace --exclude weep      # the library crates, no GPU needed
cargo check -p weep                        # the app
```

The terminal, run and git crates are tested against a real shell, real processes and real
git; the LSP crate against a real rust-analyzer.
