#!/usr/bin/env python3
"""Generate the fixture workspace both editors open for a probe (harness A in docs/probe-plan.md).

    make-fixture.py <dir>

Deterministic: the same files, bytes and git history every time, so a screenshot taken today
and one taken against a newer Fleet show the same names in the same places. <dir> is created
(or emptied first). A long directory name exercises root-row truncation, so a name such as
`probe-fixture-with-a-deliberately-long-workspace-name` is a good choice.

What it writes, and which probe it serves:

    outer/middle/inner/leaf.txt    single-child chain: Fleet's compact folders (`outer / middle / inner`)
    src/lib/util/helpers.rs        three levels of nesting: second and third level indent
    .hidden, .editorconfig,        dotfiles: the hidden-files toggle
    .fleet/run.json                Fleet's run configurations (M6)
    src/main.rs, data/config.json, data/settings.toml, README.md
                                   one file per supported grammar: syntax colours
    long/thousand.txt              1000 lines: scrollbar thumb, line-number gutter width
    long/wide.txt                  a 300-column line: horizontal scroll, soft wrap
    misc/tabs_crlf_unicode.txt     tabs, CRLF, non-ASCII: whitespace glyphs, line endings, width
    misc/empty.txt                 empty buffer
    big/big.log                    2 MB: the big-file threshold (localBigFileMinimumThreshold)
    git: one commit, then README.md modified, src/added.rs added (staged), src/old.rs renamed
    to src/renamed.rs, notes.txt untracked: git colours in the tree and gutter marks

Needs `git` on PATH. Standard library only.
"""
import os
import shutil
import stat
import subprocess
import sys

MAIN_RS = """//! Fixture: one of each syntax element the highlighter maps.

use std::collections::HashMap;

/// A small record.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub id: u32,
    pub name: String,
    pub tags: Vec<&'static str>,
}

impl Record {
    pub fn new(id: u32, name: &str) -> Self {
        Record { id, name: name.to_string(), tags: vec!["alpha", "beta"] }
    }

    fn score(&self) -> f64 {
        let base = self.id as f64 * 1.5; // a comment after code
        if self.name.is_empty() { 0.0 } else { base + 0x1F as f64 }
    }
}

fn main() {
    let mut index: HashMap<u32, Record> = HashMap::new();
    for i in 0..10 {
        index.insert(i, Record::new(i, &format!("record-{i}")));
    }
    let total: f64 = index.values().map(Record::score).sum();
    println!("total = {total:.2}, count = {}", index.len());
    match index.get(&3) {
        Some(r) if r.tags.contains(&"alpha") => println!("{r:?}"),
        Some(_) => unreachable!(),
        None => eprintln!("missing"),
    }
}
"""

HELPERS_RS = """pub fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    v.max(lo).min(hi)
}

#[cfg(test)]
mod tests {
    use super::clamp;

    #[test]
    fn clamps() {
        assert_eq!(clamp(5, 0, 3), 3);
    }
}
"""

CONFIG_JSON = """{
  "name": "fixture",
  "version": 3,
  "enabled": true,
  "ratio": 0.75,
  "tags": ["alpha", "beta", null],
  "nested": {
    "depth": 2,
    "items": [{"id": 1}, {"id": 2}]
  }
}
"""

SETTINGS_TOML = """# Fixture settings
title = "fixture"

[editor]
tab_size = 4
soft_wrap = false
font = { family = "JetBrains Mono", size = 13.0 }

[[servers]]
name = "alpha"
port = 8080

[[servers]]
name = "beta"
port = 8081
"""

README_MD = """# Fixture workspace

A generated workspace that both editors open for a probe. Nothing here is
hand-written after generation; run `make-fixture.py` again to reset it.

## Contents

- `src/` - Rust sources, three levels deep
- `data/` - one JSON and one TOML file
- `long/` - a 1000-line file and a 300-column line

| File | Purpose |
| --- | --- |
| `misc/empty.txt` | empty buffer |
| `big/big.log` | 2 MB, above the big-file threshold |

```rust
fn main() { println!("fenced"); }
```

*Emphasis*, **strong**, `code` and a [link](https://example.invalid).
"""

EDITORCONFIG = """root = true

[*]
indent_style = space
indent_size = 4
end_of_line = lf
charset = utf-8

[*.md]
trim_trailing_whitespace = false
"""

RUN_JSON = """{
  "configurations": [
    {
      "type": "cargo",
      "name": "run fixture",
      "cargoArgs": ["run"]
    }
  ]
}
"""

GIT_ENV = {
    "GIT_AUTHOR_NAME": "Probe",
    "GIT_AUTHOR_EMAIL": "probe@example.invalid",
    "GIT_COMMITTER_NAME": "Probe",
    "GIT_COMMITTER_EMAIL": "probe@example.invalid",
    "GIT_AUTHOR_DATE": "2026-01-01T00:00:00Z",
    "GIT_COMMITTER_DATE": "2026-01-01T00:00:00Z",
}


def write(root, rel, data, newline="\n"):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline=newline) as fh:
        fh.write(data)


def git(root, *args):
    env = dict(os.environ, **GIT_ENV)
    subprocess.run(["git", "-C", root, *args], check=True, env=env,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def _force_remove(func, path, _exc):
    os.chmod(path, stat.S_IWRITE)
    func(path)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    root = os.path.abspath(sys.argv[1])
    if os.path.isdir(root):
        shutil.rmtree(root, onexc=_force_remove)
    os.makedirs(root)

    write(root, "src/main.rs", MAIN_RS)
    write(root, "src/old.rs", "pub fn old() {}\n")
    write(root, "src/lib/mod.rs", "pub mod util;\n")
    write(root, "src/lib/util/helpers.rs", HELPERS_RS)
    write(root, "src/lib/util/mod.rs", "pub mod helpers;\n")
    write(root, "outer/middle/inner/leaf.txt", "leaf\n")
    write(root, "data/config.json", CONFIG_JSON)
    write(root, "data/settings.toml", SETTINGS_TOML)
    write(root, "README.md", README_MD)
    write(root, ".hidden", "hidden\n")
    write(root, ".editorconfig", EDITORCONFIG)
    write(root, ".fleet/run.json", RUN_JSON)
    write(root, ".gitignore", "big/\n")
    write(root, "long/thousand.txt",
          "".join(f"line {i:4d}: the quick brown fox jumps over the lazy dog\n" for i in range(1, 1001)))
    wide = "".join(f"col{i:03d} " for i in range(1, 44))[:300]
    write(root, "long/wide.txt", f"short line\n{wide}\nshort line\n")
    write(root, "misc/tabs_crlf_unicode.txt",
          "tab\there\r\n\tindented with a tab\r\nnaïve café — résumé\r\n"
          "日本語のテキスト\r\nemoji \U0001f980 crab\r\ntrailing spaces   \r\n",
          newline="")
    write(root, "misc/empty.txt", "")
    line = "2026-01-01T00:00:00Z INFO fixture: a log line with some padding to reach a round length ..\n"
    write(root, "big/big.log", line * (2 * 1024 * 1024 // len(line) + 1))

    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.name", "Probe")
    git(root, "config", "user.email", "probe@example.invalid")
    git(root, "config", "core.autocrlf", "false")
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", "fixture")

    write(root, "README.md", README_MD + "\nModified after the commit.\n")
    write(root, "src/added.rs", "pub fn added() {}\n")
    git(root, "add", "src/added.rs")
    git(root, "mv", "src/old.rs", "src/renamed.rs")
    write(root, "notes.txt", "untracked\n")
    print(root)


if __name__ == "__main__":
    main()
