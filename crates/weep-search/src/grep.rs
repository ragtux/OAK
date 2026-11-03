//! `find-in-path` and `replace-in-path`: the Search tool's query over every
//! text file in the workspace. Fleet's stored query has `text`,
//! `caseSensitive`, `words`, `regex`, a `mask` (file patterns), a `path`
//! (a folder to search under) and a `limit` of 100 (its local storage,
//! `docs/fleet-probe.md`); the same fields are here.

use crate::files::walk;
use regex::{Regex, RegexBuilder};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Files past this size are skipped unless `search_big_files` is on
/// (`ff.searchInBigUnindexedFiles`); Fleet's own threshold is not
/// documented, so this is an estimate.
pub const BIG_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GrepOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Comma-separated file patterns (`*.rs, *.toml`); empty for all.
    pub mask: String,
    /// A folder under the root to search in; empty for the whole workspace.
    pub path: String,
    /// `files.exclude`.
    pub exclude: Vec<String>,
    pub search_big_files: bool,
    /// Stop after this many matching lines (Fleet stores 100); 0 for no limit.
    pub limit: usize,
}

#[derive(Clone, Debug)]
pub struct GrepQuery {
    re: Regex,
    masks: Vec<Regex>,
    pub options: GrepOptions,
}

/// One matching line: its 0-based row, its text (without the line ending)
/// and the matched byte ranges inside that text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineMatch {
    pub row: usize,
    pub text: String,
    pub ranges: Vec<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatches {
    pub path: PathBuf,
    pub lines: Vec<LineMatch>,
}

impl FileMatches {
    pub fn count(&self) -> usize {
        self.lines.iter().map(|l| l.ranges.len()).sum()
    }
}

fn glob_to_regex(glob: &str) -> Option<Regex> {
    let mut out = String::from("^");
    for c in glob.trim().chars() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    out.push('$');
    RegexBuilder::new(&out).case_insensitive(true).build().ok()
}

impl GrepQuery {
    /// Compiles `pattern` with the options; an empty pattern or a bad regex
    /// is an error with the message the tool shows (empty for an empty pattern).
    pub fn new(pattern: &str, options: GrepOptions) -> Result<GrepQuery, String> {
        if pattern.is_empty() {
            return Err(String::new());
        }
        let mut source = if options.regex {
            pattern.to_string()
        } else {
            regex::escape(pattern)
        };
        if options.whole_word {
            source = format!("(?:^|[^\\w])({source})(?:[^\\w]|$)");
        }
        let re = RegexBuilder::new(&source)
            .case_insensitive(!options.case_sensitive)
            .build()
            .map_err(|e| {
                e.to_string()
                    .lines()
                    .last()
                    .unwrap_or("bad pattern")
                    .trim()
                    .to_string()
            })?;
        let masks = options
            .mask
            .split(',')
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .filter_map(glob_to_regex)
            .collect();
        Ok(GrepQuery { re, masks, options })
    }

    /// The matched byte ranges in one line (the capture for whole words).
    pub fn find_in_line(&self, line: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        if self.options.whole_word {
            let mut start = 0;
            while start <= line.len() {
                let Some(caps) = self.re.captures_at(line, start) else {
                    break;
                };
                let Some(g) = caps.get(1) else { break };
                if g.end() == g.start() {
                    start = g.end() + 1;
                    continue;
                }
                out.push(g.start()..g.end());
                // The lookaround consumed a neighbour; resume at the match's end.
                start = g.end();
            }
        } else {
            for m in self.re.find_iter(line) {
                if m.end() > m.start() {
                    out.push(m.start()..m.end());
                }
            }
        }
        out
    }

    /// Whether the file mask admits `name`.
    pub fn mask_allows(&self, name: &str) -> bool {
        self.masks.is_empty() || self.masks.iter().any(|m| m.is_match(name))
    }

    /// The replacement for one matched range, with `$1` groups expanded in
    /// regex mode; literal otherwise.
    pub fn replacement_for(&self, line: &str, range: Range<usize>, replacement: &str) -> String {
        if !self.options.regex {
            return replacement.to_string();
        }
        let slice = &line[range];
        let mut out = String::new();
        if let Some(caps) = self.re.captures(slice) {
            caps.expand(&brace_groups(replacement), &mut out);
            out
        } else {
            replacement.to_string()
        }
    }
}

/// `$1y` → `${1}y`: the regex crate reads `$1y` as a group named `1y`, while
/// Fleet's replacement field means group 1 followed by a `y`.
fn brace_groups(replacement: &str) -> String {
    let mut out = String::with_capacity(replacement.len() + 4);
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            out.push_str("${");
            out.extend(&chars[i + 1..j]);
            out.push('}');
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

impl GrepQuery {
    /// Searches `text` (one file's contents); rows are 0-based.
    pub fn search_text(&self, text: &str) -> Vec<LineMatch> {
        let mut out = Vec::new();
        for (row, raw) in text.split('\n').enumerate() {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            let ranges = self.find_in_line(line);
            if !ranges.is_empty() {
                out.push(LineMatch {
                    row,
                    text: line.to_string(),
                    ranges,
                });
            }
        }
        out
    }

    /// Searches every text file under the root (or the options' `path`),
    /// honouring `.gitignore`, `files.exclude` and the mask, skipping binary
    /// files (a NUL in the first 8 KB) and, unless asked, big ones. Files are
    /// visited in path order; the result holds at most `limit` matching lines.
    /// `overrides` supplies the current text of files open in the editor,
    /// whose unsaved contents must be searched instead of the disk's.
    pub fn search(&self, root: &Path, overrides: &[(PathBuf, String)]) -> Vec<FileMatches> {
        let start = if self.options.path.trim().is_empty() {
            root.to_path_buf()
        } else {
            root.join(self.options.path.trim())
        };
        let mut paths: Vec<PathBuf> = walk(&start, &self.options.exclude)
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .map(|e| e.into_path())
            .collect();
        paths.sort();
        let mut out = Vec::new();
        let mut lines_found = 0;
        let limit = if self.options.limit == 0 {
            usize::MAX
        } else {
            self.options.limit
        };
        for path in paths {
            if lines_found >= limit {
                break;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if !self.mask_allows(&name) {
                continue;
            }
            let text = match overrides.iter().find(|(p, _)| *p == path) {
                Some((_, t)) => t.clone(),
                None => {
                    if !self.options.search_big_files {
                        if let Ok(meta) = std::fs::metadata(&path) {
                            if meta.len() > BIG_FILE_BYTES {
                                continue;
                            }
                        }
                    }
                    let Ok(bytes) = std::fs::read(&path) else {
                        continue;
                    };
                    if bytes.iter().take(8192).any(|b| *b == 0) {
                        continue;
                    }
                    match String::from_utf8(bytes) {
                        Ok(t) => t,
                        Err(_) => continue,
                    }
                }
            };
            let mut lines = self.search_text(&text);
            if lines.is_empty() {
                continue;
            }
            if lines_found + lines.len() > limit {
                lines.truncate(limit - lines_found);
            }
            lines_found += lines.len();
            out.push(FileMatches { path, lines });
        }
        out
    }

    /// The text with every match replaced (regex groups expanded), or `None`
    /// when nothing matched.
    pub fn replace_in_text(&self, text: &str, replacement: &str) -> Option<String> {
        let mut changed = false;
        let mut out = String::with_capacity(text.len());
        let mut first = true;
        for raw in text.split('\n') {
            if !first {
                out.push('\n');
            }
            first = false;
            let (line, cr) = match raw.strip_suffix('\r') {
                Some(l) => (l, true),
                None => (raw, false),
            };
            let ranges = self.find_in_line(line);
            if ranges.is_empty() {
                out.push_str(raw);
                continue;
            }
            changed = true;
            let mut last = 0;
            for r in ranges {
                out.push_str(&line[last..r.start]);
                out.push_str(&self.replacement_for(line, r.clone(), replacement));
                last = r.end;
            }
            out.push_str(&line[last..]);
            if cr {
                out.push('\r');
            }
        }
        changed.then_some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn q(p: &str, f: impl FnOnce(&mut GrepOptions)) -> GrepQuery {
        let mut o = GrepOptions::default();
        f(&mut o);
        GrepQuery::new(p, o).unwrap()
    }

    #[test]
    fn lines_and_ranges() {
        let g = q("rec", |_| {});
        let m = g.search_text("Record\nno\nrec rec\r\n");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].row, 0);
        assert_eq!(m[0].ranges, vec![0..3]);
        assert_eq!(m[1].row, 2);
        assert_eq!(m[1].text, "rec rec");
        assert_eq!(m[1].ranges, vec![0..3, 4..7]);
    }

    #[test]
    fn toggles() {
        assert!(q("Rec", |o| o.case_sensitive = true)
            .search_text("record")
            .is_empty());
        let w = q("rec", |o| o.whole_word = true);
        assert!(w.search_text("record").is_empty());
        assert_eq!(w.search_text("a rec. rec")[0].ranges, vec![2..5, 7..10]);
        let r = q("r(e)c", |o| o.regex = true);
        assert_eq!(r.search_text("rec")[0].ranges, vec![0..3]);
        assert_eq!(r.replacement_for("rec", 0..3, "x$1y"), "xey");
        assert!(GrepQuery::new(
            "(",
            GrepOptions {
                regex: true,
                ..Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn mask_and_replace() {
        let g = q("a", |o| o.mask = "*.rs, Cargo.*".into());
        assert!(g.mask_allows("main.rs"));
        assert!(g.mask_allows("Cargo.toml"));
        assert!(!g.mask_allows("notes.txt"));
        let g = q("foo", |_| {});
        assert_eq!(
            g.replace_in_text("foo bar\r\nfoo\n", "baz").as_deref(),
            Some("baz bar\r\nbaz\n")
        );
        assert_eq!(g.replace_in_text("bar", "baz"), None);
    }

    #[test]
    fn walks_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "needle one\nhay\nneedle two\n").unwrap();
        fs::write(root.join("b.txt"), "needle three\n").unwrap();
        fs::write(root.join("bin.dat"), b"need\0le").unwrap();
        let g = q("needle", |o| o.limit = 100);
        let found = g.search(root, &[]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path, root.join("b.txt"));
        assert_eq!(found[1].lines.len(), 2);
        let g = q("needle", |o| o.mask = "*.rs".into());
        assert_eq!(g.search(root, &[]).len(), 1);
        let g = q("needle", |o| o.limit = 1);
        let found = g.search(root, &[]);
        assert_eq!(found.iter().map(|f| f.lines.len()).sum::<usize>(), 1);
        // An open file's unsaved text replaces the disk's.
        let g = q("fresh", |_| {});
        let found = g.search(root, &[(root.join("b.txt"), "fresh\n".into())]);
        assert_eq!(found.len(), 1);
        let g = q("needle", |o| o.path = "src".into());
        assert_eq!(g.search(root, &[]).len(), 1);
    }
}
