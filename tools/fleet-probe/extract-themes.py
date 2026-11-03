#!/usr/bin/env python3
"""Pull Fleet's theme files and icon names out of a Fleet distribution.

usage: extract-themes.py <Fleet-dir-or-tar.gz> <out-dir>

Fleet ships its themes as `fleet/themes/*.json` inside `fleet.themes-*.jar`
and its icons as `frontend/icons/**.svg` inside `fleet.frontend.resources-*.jar`,
both under `lib/app/code-cache/<hash>/`. This writes the theme files, a
`<name>.resolved.json` beside each with every palette reference replaced by
its hex value, and a list of the icon names (the SVGs themselves are Fleet's
and are not copied into WEEP; see docs/fleet-probe.md).
"""
import json, pathlib, sys, tarfile, tempfile, zipfile

src, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
if src.is_file():
    tmp = pathlib.Path(tempfile.mkdtemp())
    tarfile.open(src).extractall(tmp)
    src = next(tmp.glob("Fleet*"))

icons = []
for jar in src.rglob("*.jar"):
    z = zipfile.ZipFile(jar)
    for name in z.namelist():
        if name.startswith("fleet/themes/") and name.endswith(".json"):
            (out / pathlib.Path(name).name).write_bytes(z.read(name))
        elif name.endswith(".svg") and "frontend.resources" in jar.name:
            icons.append(name)
(out / "icon-names.txt").write_text("\n".join(sorted(icons)) + "\n")

def resolve(theme):
    pal = theme.get("palette", {})
    def res(v):
        if isinstance(v, str):
            if v.startswith("#"): return v
            for k in (v, v.replace("_", " "), v.replace(" ", "_")):
                if k in pal: return pal[k]
            return "#00000000" if v == "Transparent" else "?" + v
        if isinstance(v, dict): return {k: res(x) for k, x in v.items()}
        return v
    return {"meta": theme["meta"], "colors": {k: res(v) for k, v in theme["colors"].items()},
            "textAttributes": {k: res(v) for k, v in theme["textAttributes"].items()}, "palette": pal}

for f in sorted(out.glob("*.json")):
    if f.name.endswith(".resolved.json"): continue
    json.dump(resolve(json.load(open(f))), open(out / f"{f.stem}.resolved.json", "w"), indent=1)
    print("wrote", f.name, "and", f.stem + ".resolved.json")
print(len(icons), "icon names in icon-names.txt")
