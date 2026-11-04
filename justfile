# WEEP task runner. `just` lists the recipes.
# On NixOS run the gpui recipes inside `nix-shell`: `nix-shell --run 'just run .'`.

default:
    @just --list --unsorted

# Library tests only: every crate but the gpui app (fast, no GPU)
test:
    cargo test --workspace --exclude weep

# Type-check the app crate (pulls gpui; slow the first time)
check:
    cargo check -p weep

# Open a folder (or a file) in WEEP
run path=".":
    cargo run --release -p weep -- {{path}}

# Open with a specific theme: a bundled name (dark, dark_purple, light) or a Fleet-format JSON file
run-theme theme path=".":
    cargo run --release -p weep -- --theme {{theme}} {{path}}

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets

# Build weep.cc into site/dist/weep (needs node; on macOS without it: nix-shell -p nodejs --run 'just site')
site:
    cd site && npm install --silent && node build.mjs weep

# Build weep.cc and serve it on http://localhost:4321/
site-serve:
    cd site && npm install --silent && node build.mjs weep --serve

# Re-cut the site's copy of the README's screenshots from docs/img/app (1600 px wide)
site-media:
    for f in docs/img/app/*.png; do \
        if command -v sips >/dev/null; then sips -Z 1600 "$f" --out site/sites/weep/assets/img/app/ >/dev/null; \
        else cp -f "$f" site/sites/weep/assets/img/app/; fi; \
    done
    @echo "site/sites/weep/assets/img/app refreshed from docs/img/app"

# Linux: release build, checks and dist/weep-<version>-linux-<arch>.tar.gz
package-linux:
    tools/linux/package.sh

# macOS: release build of both arches, WEEP.app and dist/weep-<version>-macos-universal.zip
package-macos:
    tools/macos/package.sh

# Windows: release build, checks and dist/weep-<version>-windows-<arch>.zip
package-windows:
    pwsh -NoProfile -File tools/windows/package.ps1

# Re-render the application icons from the logo (needs Pillow)
icons:
    python3 tools/icons/make_icons.py
