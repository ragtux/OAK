#!/usr/bin/env python3
"""Write WEEP's bundled themes from Fleet's resolved theme files.

usage: make-weep-themes.py <dir-with-*.resolved.json> <crates/weep/assets/themes>

Keeps the `colors` keys WEEP's shell paints from (by prefix) and the
`textAttributes` keys its highlighter maps to, as plain hex values, in
Fleet's own file format so the result loads in both editors.
"""
import json, pathlib, sys

src, dst = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
dst.mkdir(parents=True, exist_ok=True)
COLOR_PREFIXES = ("text.", "background.", "border", "focusOutline", "island.", "separator.", "icon.",
  "tab.", "listItem.", "tree.", "editor.", "scrollbar.", "statusBar.", "popup.", "tooltip.",
  "button.primary.", "button.secondary.", "ghostButton.", "toggleButton.", "inputField.",
  "search.", "shortcut.", "gitBranchTag.", "smartMode.", "shadow.", "completion.", "link.",
  "dropdown.", "notification.", "banner.", "terminal.", "highlight.", "checkbox.", "progressBar.",
  "problemsWidget.")
ATTRS = ["editor.text.scheme","editor.selection","editor.selection.focused","editor.brace.match",
  "editor.indentGuide","editor.indentGuide.current","editor.search.results","identifier.underCaret",
  "comment","comment.doc","comment.doc.tag","comment.todo","keyword","keyword.typeModifier","string",
  "string.escape","string.regexp","number","boolean","metadata","identifier","identifier.constant",
  "identifier.variable","identifier.variable.mutable","identifier.parameter","identifier.field",
  "identifier.field.static","identifier.function.declaration","identifier.function.call",
  "identifier.type","identifier.type.class","identifier.type.enum","identifier.type.struct",
  "identifier.interface","identifier.typeParameter","identifier.namedArgument","identifier.this",
  "punctuation","punctuation.operator","key.json","key.yaml","value.yaml","tagName.html",
  "attributeName.html","attributeValue.html","markup.heading","markup.bold","markup.italic",
  "markup.code.block","markup.href","link","link.hovered","problem.error","problem.warning",
  "problem.info","problem.unused","problem.deprecated","diff.added","diff.deleted","diff.modified",
  "diff.added.word","diff.deleted.word","diff.modified.word","target.match","target.match.word",
  "snippet","snippet.selected","ai.generated"]
for f in sorted(src.glob("*.resolved.json")):
    name = f.name.replace(".resolved.json", "")
    r = json.load(open(f))
    colors = {k: v for k, v in r["colors"].items() if k.startswith(COLOR_PREFIXES)}
    attrs = {}
    for k in ATTRS:
        a = r["textAttributes"].get(k)
        if a is None: continue
        strip = lambda v: v[1:] if isinstance(v, str) and v.startswith("?") else v
        attrs[k] = {ak: ({dk: strip(dv) for dk, dv in av.items()} if isinstance(av, dict) else strip(av)) for ak, av in a.items()}
    out = {"meta": {**r["meta"], "theme.source": f"values read from JetBrains Fleet's {name} theme; see docs/fleet-probe.md"},
           "colors": colors, "textAttributes": attrs}
    json.dump(out, open(dst / f"{name}.json", "w"), indent=2)
    print(name, len(colors), "colours", len(attrs), "attributes")
