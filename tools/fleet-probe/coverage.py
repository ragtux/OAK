#!/usr/bin/env python3
"""Report how much of Fleet's surface Oak covers, from inventory.py's output and Oak's sources.

    coverage.py <inventory dir> [repo root]

Reads the name lists inventory.py wrote and greps Oak for the same strings:

    actions   Fleet action ids matched in crates/oak/src/keymap.rs (the match arms)
    colors    theme `colors` keys quoted anywhere in crates/oak/src
    attrs     theme `textAttributes` keys quoted anywhere in crates/oak/src and crates/oak-syntax/src
    settings  settings keys quoted in crates/oak-settings/src, crates/oak/src/settings.rs and settings_schema.rs

Prints a one-line summary per table and, with -v, the missing names, so a milestone can be checked
off against Fleet's own lists rather than from memory.
"""
import os
import re
import sys


def read_list(path):
    with open(path) as fh:
        return [line.strip() for line in fh if line.strip()]


def quoted_strings(paths):
    found = set()
    for path in paths:
        if not os.path.exists(path):
            continue
        for root, _dirs, files in os.walk(path) if os.path.isdir(path) else [(os.path.dirname(path), [], [os.path.basename(path)])]:
            for f in files:
                if f.endswith(".rs"):
                    with open(os.path.join(root, f), encoding="utf-8") as fh:
                        found.update(re.findall(r'"([A-Za-z0-9_./-]+)"', fh.read()))
    return found


def report(label, fleet, oak, verbose):
    have = [k for k in fleet if k in oak]
    print(f"{label:9} {len(have):4}/{len(fleet):<4} ({100 * len(have) / max(1, len(fleet)):.0f}%)")
    if verbose:
        for k in fleet:
            if k not in oak:
                print(f"    missing {k}")


def main():
    args = [a for a in sys.argv[1:] if a != "-v"]
    verbose = "-v" in sys.argv
    if not args:
        sys.exit(__doc__)
    inv = args[0]
    repo = args[1] if len(args) > 1 else os.path.join(os.path.dirname(__file__), "..", "..")
    app = os.path.join(repo, "crates", "oak", "src")
    report("actions", read_list(os.path.join(inv, "actions-bound.txt")),
           quoted_strings([os.path.join(app, "keymap.rs")]), verbose)
    report("colors", read_list(os.path.join(inv, "theme-colors.txt")), quoted_strings([app]), verbose)
    report("attrs", read_list(os.path.join(inv, "theme-text-attributes.txt")),
           quoted_strings([app, os.path.join(repo, "crates", "oak-syntax", "src")]), verbose)
    report("settings", read_list(os.path.join(inv, "settings.txt")),
           quoted_strings([os.path.join(repo, "crates", "oak-settings", "src"), os.path.join(app, "settings.rs"), os.path.join(app, "settings_schema.rs")]),
           verbose)


if __name__ == "__main__":
    main()
