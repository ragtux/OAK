---
title: Download and run
description: The Linux, macOS and Windows archives, building WEEP from source with cargo, and the weep command line.
---

# Download and run

The [latest release](https://github.com/ragtux/WEEP/releases/latest) carries one archive per
platform, each with the binary and a `.sha256`. Nothing is signed.

| Platform | Archive | Notes |
|---|---|---|
| Linux | `weep-<version>-linux-x64.tar.gz` | Built on Ubuntu 22.04, so glibc 2.35 or newer. Holds `weep`, plus `weep.desktop` and `weep.png` for `~/.local/share/applications` and `~/.local/share/icons/hicolor/256x256/apps` |
| macOS | `weep-<version>-macos-universal.zip` | A `WEEP.app` for both arches. Unnotarized: the first launch needs right-click > Open |
| Windows | `weep-<version>-windows-x64.zip` | One `weep.exe`, with a static C runtime — no Visual C++ Redistributable |

## From source

WEEP builds with stable Rust (1.85 or newer); `rust-toolchain.toml` picks it, with rustfmt
and clippy.

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
