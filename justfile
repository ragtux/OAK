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
