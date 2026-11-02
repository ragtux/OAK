#!/usr/bin/env python3
"""Pull Fleet's own inventories out of a distribution, for coverage checks and probe planning.

    inventory.py <Fleet-x.y.z.tar.gz | unpacked Fleet dir> <out dir>

Writes to <out dir> (plain text, one entry per line unless noted):

    plugins.txt            plugin groups found as jars (fleet.plugins.<name>.*)
    actions-bound.txt      action ids that have a chord in the default keymap (fleet/keymap/fleet.json)
    actions-documented.txt action ids described in any plugin's documentation.yaml
    actions.txt            the union
    settings.txt           settings.json keys described in any documentation.yaml
    theme-colors.txt       the `colors` keys of the dark theme
    theme-text-attributes.txt  the `textAttributes` keys of the dark theme
    icons.txt              icon paths under frontend/icons/dark/
    documentation.yaml.md  every documentation.yaml concatenated, with its jar as a heading (local reference only; Fleet's prose, do not commit)
    keymaps/*.json         every keymap preset Fleet ships, verbatim (local reference only, do not commit)
    themes/*.json          every theme Fleet ships, verbatim (local reference only, do not commit)

Only the name lists are meant to be read into the repo (see coverage.py). Uses the standard library
only: Python's zipfile reads the jars, so no `unzip` or `java` is needed.
"""
import collections
import io
import json
import os
import re
import sys
import tarfile
import zipfile


def iter_jars(src):
    """Yield (basename, bytes) for every jar in a tarball or an unpacked directory."""
    if os.path.isdir(src):
        for root, _dirs, files in os.walk(src):
            for f in files:
                if f.endswith(".jar"):
                    with open(os.path.join(root, f), "rb") as fh:
                        yield f, fh.read()
    else:
        with tarfile.open(src) as tar:
            for member in tar:
                if member.name.endswith(".jar"):
                    yield os.path.basename(member.name), tar.extractfile(member).read()


def strip_json_comments(text):
    text = re.sub(r"//[^\n]*", "", text)
    return re.sub(r",\s*([\]}])", r"\1", text)


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    src, out = sys.argv[1:]
    os.makedirs(os.path.join(out, "keymaps"), exist_ok=True)
    os.makedirs(os.path.join(out, "themes"), exist_ok=True)

    plugins = collections.Counter()
    bound, documented, settings = set(), set(), set()
    docs = []
    colors, text_attributes, icons = [], [], []

    for name, data in iter_jars(src):
        base = re.sub(r"-\d+\.\d+\.\d+.*$", "", name)
        if base.startswith("fleet.plugins."):
            plugins[base.split(".")[2]] += 1
        z = zipfile.ZipFile(io.BytesIO(data))
        for entry in z.namelist():
            if entry.endswith("documentation.yaml"):
                text = z.read(entry).decode("utf-8", "replace")
                docs.append(f"### {name} :: {entry}\n{text}\n")
                section = None
                for line in text.splitlines():
                    if re.match(r"^(actions|settings):", line):
                        section = line.split(":")[0]
                        continue
                    m = re.match(r"^  ([\w/.-]+):", line)
                    if m and section == "actions":
                        documented.add(m.group(1))
                    elif m and section == "settings":
                        settings.add(m.group(1))
            elif (entry.startswith("fleet/keymap/") or base.startswith("fleet.plugins.keymaps.")) and entry.endswith(".json") and "META-INF" not in entry:
                # fleet.json, intellij.json and visual-studio.json sit in the keymap plugin; the other
                # presets (eclipse, emacs, ij-classic, sublimeText, vscode) are each a jar with the json at its root
                raw = z.read(entry).decode("utf-8")
                with open(os.path.join(out, "keymaps", os.path.basename(entry)), "w") as fh:
                    fh.write(raw)
                if entry.endswith("fleet.json"):
                    km = json.loads(strip_json_comments(raw))
                    entries = km["keymap"] if isinstance(km, dict) else km
                    bound.update(e["action"] for e in entries if "action" in e)
            elif entry.startswith("fleet/themes/") and entry.endswith(".json"):
                raw = z.read(entry).decode("utf-8")
                with open(os.path.join(out, "themes", os.path.basename(entry)), "w") as fh:
                    fh.write(raw)
                if entry.endswith("dark.json"):
                    theme = json.loads(strip_json_comments(raw))
                    colors = sorted(theme["colors"])
                    text_attributes = sorted(theme["textAttributes"])
            elif entry.startswith("frontend/icons/dark/") and entry.endswith(".svg"):
                icons.append(entry[len("frontend/icons/dark/"):])

    def write(fname, lines):
        with open(os.path.join(out, fname), "w") as fh:
            fh.write("\n".join(lines) + "\n")

    write("plugins.txt", [f"{k} {v}" for k, v in sorted(plugins.items())])
    write("actions-bound.txt", sorted(bound))
    write("actions-documented.txt", sorted(documented))
    write("actions.txt", sorted(bound | documented))
    write("settings.txt", sorted(settings))
    write("theme-colors.txt", colors)
    write("theme-text-attributes.txt", text_attributes)
    write("icons.txt", sorted(icons))
    write("documentation.yaml.md", docs)
    print(
        f"{len(plugins)} plugin groups, {len(bound)} bound actions, {len(documented)} documented actions "
        f"({len(bound | documented)} total), {len(settings)} settings keys, {len(colors)} colour keys, "
        f"{len(text_attributes)} textAttributes keys, {len(icons)} icons -> {out}"
    )


if __name__ == "__main__":
    main()
