//! EditorConfig (<https://editorconfig.org>): the `.editorconfig` files a
//! repository keeps per folder, which set indentation, line endings and
//! whitespace handling for the files their sections match. Fleet applies them
//! over `editor.tabSize` and `editor.indentStyle` ("EditorConfig may override
//! this option"), so Oak does the same.
//!
//! The parser and the glob matcher follow the EditorConfig specification
//! (file format, section globs, the `root` key and the `unset` value) and are
//! written by hand so the crate takes no new dependency.

use crate::IndentStyle;
use std::path::{Component, Path, PathBuf};

/// The line ending `end_of_line` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndOfLine {
    Lf,
    CrLf,
    Cr,
}

impl EndOfLine {
    /// `lf`, `crlf` or `cr`, in any case.
    pub fn parse(s: &str) -> Option<EndOfLine> {
        match s.trim().to_ascii_lowercase().as_str() {
            "lf" => Some(EndOfLine::Lf),
            "crlf" => Some(EndOfLine::CrLf),
            "cr" => Some(EndOfLine::Cr),
            _ => None,
        }
    }

    /// The characters that end a line.
    pub fn as_str(self) -> &'static str {
        match self {
            EndOfLine::Lf => "\n",
            EndOfLine::CrLf => "\r\n",
            EndOfLine::Cr => "\r",
        }
    }
}

/// The EditorConfig properties that apply to one file. `None` means no file
/// set the property (or one `unset` it), so the editor's own setting applies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorConfig {
    pub indent_style: Option<IndentStyle>,
    /// Columns per indent level. `indent_size = tab` resolves to `tab_width`
    /// (and stays `None` while no `tab_width` is known).
    pub indent_size: Option<usize>,
    /// Columns per tab character; defaults to `indent_size` when only that is
    /// given, as the specification says.
    pub tab_width: Option<usize>,
    pub end_of_line: Option<EndOfLine>,
    pub trim_trailing_whitespace: Option<bool>,
    pub insert_final_newline: Option<bool>,
    /// `latin1`, `utf-8`, `utf-8-bom`, `utf-16be` or `utf-16le`, lower-cased.
    pub charset: Option<String>,
}

impl EditorConfig {
    /// Walks from the file's folder up to the filesystem root, or to the first
    /// `.editorconfig` that says `root = true`, reads each `.editorconfig` and
    /// applies the sections matching the file, the nearest file last so it
    /// wins. Unreadable or malformed files are skipped. Never panics.
    pub fn for_path(path: &Path) -> EditorConfig {
        let path = absolute(path);
        let mut layers: Vec<(Parsed, String)> = Vec::new();
        let mut folder = path.parent();
        while let Some(dir) = folder {
            if let Ok(text) = std::fs::read_to_string(dir.join(".editorconfig")) {
                let parsed = Parsed::parse(&text);
                let root = parsed.root;
                layers.push((parsed, relative_to(&path, dir)));
                if root {
                    break;
                }
            }
            folder = dir.parent();
        }
        let mut config = EditorConfig::default();
        for (parsed, relative) in layers.iter().rev() {
            config.apply_parsed(parsed, relative);
        }
        config.finish();
        config
    }

    /// Parses one file's text and applies the sections whose glob matches
    /// `relative` (the file's path relative to the folder holding the
    /// `.editorconfig`, with `/` separators) onto `self`, later sections last.
    pub fn apply(&mut self, text: &str, relative: &str) {
        self.apply_parsed(&Parsed::parse(text), relative);
    }

    /// True when no property is set.
    pub fn is_empty(&self) -> bool {
        *self == EditorConfig::default()
    }

    fn apply_parsed(&mut self, parsed: &Parsed, relative: &str) {
        let mut indent_size_is_tab = false;
        for section in &parsed.sections {
            if !section.glob.matches(relative) {
                continue;
            }
            for (key, value) in &section.pairs {
                let lower = value.to_ascii_lowercase();
                let unset = lower == "unset";
                match key.as_str() {
                    "indent_style" => {
                        self.indent_style = if unset {
                            None
                        } else {
                            IndentStyle::parse(value).or(self.indent_style)
                        }
                    }
                    "indent_size" => {
                        if unset {
                            self.indent_size = None;
                            indent_size_is_tab = false;
                        } else if lower == "tab" {
                            indent_size_is_tab = true;
                        } else if let Some(n) = positive(value) {
                            self.indent_size = Some(n);
                            indent_size_is_tab = false;
                        }
                    }
                    "tab_width" => {
                        if unset {
                            self.tab_width = None;
                        } else if let Some(n) = positive(value) {
                            self.tab_width = Some(n);
                        }
                    }
                    "end_of_line" => {
                        self.end_of_line = if unset {
                            None
                        } else {
                            EndOfLine::parse(value).or(self.end_of_line)
                        }
                    }
                    "trim_trailing_whitespace" => {
                        self.trim_trailing_whitespace = if unset {
                            None
                        } else {
                            boolean(&lower).or(self.trim_trailing_whitespace)
                        }
                    }
                    "insert_final_newline" => {
                        self.insert_final_newline = if unset {
                            None
                        } else {
                            boolean(&lower).or(self.insert_final_newline)
                        }
                    }
                    "charset" => {
                        self.charset = if unset || lower.is_empty() {
                            None
                        } else {
                            Some(lower)
                        }
                    }
                    _ => {}
                }
            }
        }
        if indent_size_is_tab {
            self.indent_size = self.tab_width;
        }
    }

    /// The specification's defaults between properties: `tab_width` follows
    /// `indent_size` when only the latter is given.
    fn finish(&mut self) {
        if self.tab_width.is_none() {
            self.tab_width = self.indent_size;
        }
    }
}

fn positive(value: &str) -> Option<usize> {
    value.trim().parse::<usize>().ok().filter(|n| *n > 0)
}

fn boolean(lower: &str) -> Option<bool> {
    match lower {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path.to_path_buf(),
    }
}

/// `file` relative to `dir` with `/` separators (the form the globs match).
fn relative_to(file: &Path, dir: &Path) -> String {
    let rest = file.strip_prefix(dir).unwrap_or(file);
    let parts: Vec<String> = rest
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts.join("/")
}

// ---- the file format -------------------------------------------------------

struct Section {
    glob: Glob,
    pairs: Vec<(String, String)>,
}

struct Parsed {
    root: bool,
    sections: Vec<Section>,
}

impl Parsed {
    fn parse(text: &str) -> Parsed {
        let mut parsed = Parsed {
            root: false,
            sections: Vec::new(),
        };
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                let Some(end) = rest.rfind(']') else {
                    continue; // malformed header
                };
                parsed.sections.push(Section {
                    glob: Glob::compile(&rest[..end]),
                    pairs: Vec::new(),
                });
                continue;
            }
            let Some((key, value)) = split_pair(line) else {
                continue; // malformed line
            };
            match parsed.sections.last_mut() {
                Some(section) => section.pairs.push((key, value)),
                // Only `root` is meaningful in the preamble.
                None if key == "root" => parsed.root = value.eq_ignore_ascii_case("true"),
                None => {}
            }
        }
        parsed
    }
}

/// `key = value` (or `key: value`); the key lower-cased, both trimmed, a
/// trailing ` ;` / ` #` comment dropped from the value.
fn split_pair(line: &str) -> Option<(String, String)> {
    let at = line.find(['=', ':'])?;
    let key = line[..at].trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    let mut value = line[at + 1..].trim();
    if let Some(cut) = value.find(" ;").or_else(|| value.find(" #")) {
        value = value[..cut].trim_end();
    }
    Some((key, value.to_string()))
}

// ---- globs -----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    /// One literal character.
    Lit(char),
    /// `*`: any run of characters except `/`.
    Star,
    /// `**`: any run of characters, `/` included.
    DoubleStar,
    /// `**/`: zero or more whole path segments.
    Dirs,
    /// `?`: one character other than `/`.
    One,
    /// `[abc]`, `[a-z]`, `[!abc]`.
    Set {
        negate: bool,
        chars: Vec<char>,
        ranges: Vec<(char, char)>,
    },
    /// `{a,b,c}`: one of the alternatives.
    Alt(Vec<Vec<Tok>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Glob {
    toks: Vec<Tok>,
}

impl Glob {
    /// A pattern without a `/` matches the file name anywhere under the
    /// folder (`**/pattern`); one with a `/` is relative to the folder, a
    /// leading `/` meaning that folder itself.
    fn compile(pattern: &str) -> Glob {
        let pattern = pattern.trim();
        let anchored = pattern.contains('/');
        let body = if anchored {
            pattern.strip_prefix('/').unwrap_or(pattern)
        } else {
            pattern
        };
        let chars: Vec<char> = body.chars().collect();
        let (mut toks, _) = compile_seq(&chars, 0, false);
        if !anchored {
            toks.insert(0, Tok::Dirs);
        }
        Glob { toks }
    }

    fn matches(&self, path: &str) -> bool {
        let chars: Vec<char> = path.chars().collect();
        match_toks(&self.toks, &chars)
    }
}

/// Compiles `chars[from..]` until the end, or until an unbalanced `,` or `}`
/// when `in_braces`; returns the tokens and the index it stopped at.
fn compile_seq(chars: &[char], from: usize, in_braces: bool) -> (Vec<Tok>, usize) {
    let mut toks = Vec::new();
    let mut i = from;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() => {
                toks.push(Tok::Lit(chars[i + 1]));
                i += 2;
            }
            '*' => {
                if chars.get(i + 1) == Some(&'*') {
                    if chars.get(i + 2) == Some(&'/') {
                        toks.push(Tok::Dirs);
                        i += 3;
                    } else {
                        toks.push(Tok::DoubleStar);
                        i += 2;
                    }
                } else {
                    toks.push(Tok::Star);
                    i += 1;
                }
            }
            '?' => {
                toks.push(Tok::One);
                i += 1;
            }
            '[' => match compile_set(chars, i) {
                Some((tok, next)) => {
                    toks.push(tok);
                    i = next;
                }
                None => {
                    toks.push(Tok::Lit('['));
                    i += 1;
                }
            },
            '{' => match compile_alt(chars, i) {
                Some((tok, next)) => {
                    toks.push(tok);
                    i = next;
                }
                None => {
                    toks.push(Tok::Lit('{'));
                    i += 1;
                }
            },
            ',' | '}' if in_braces => break,
            _ => {
                toks.push(Tok::Lit(c));
                i += 1;
            }
        }
    }
    (toks, i)
}

/// `[...]` at `at`; None when it never closes (then `[` is literal).
fn compile_set(chars: &[char], at: usize) -> Option<(Tok, usize)> {
    let mut i = at + 1;
    let negate = chars.get(i) == Some(&'!');
    if negate {
        i += 1;
    }
    let mut set_chars = Vec::new();
    let mut ranges = Vec::new();
    let mut first = true;
    while i < chars.len() {
        let c = chars[i];
        if c == ']' && !first {
            return Some((
                Tok::Set {
                    negate,
                    chars: set_chars,
                    ranges,
                },
                i + 1,
            ));
        }
        first = false;
        let lit = if c == '\\' && i + 1 < chars.len() {
            i += 1;
            chars[i]
        } else {
            c
        };
        if chars.get(i + 1) == Some(&'-') && i + 2 < chars.len() && chars[i + 2] != ']' {
            let hi = chars[i + 2];
            ranges.push((lit.min(hi), lit.max(hi)));
            i += 3;
        } else {
            set_chars.push(lit);
            i += 1;
        }
    }
    None
}

/// `{a,b,c}` at `at`; None when it never closes or holds no comma (then `{`
/// is literal, which also covers `{1..3}` ranges and `{single}`).
fn compile_alt(chars: &[char], at: usize) -> Option<(Tok, usize)> {
    let mut alts = Vec::new();
    let mut i = at + 1;
    loop {
        let (toks, next) = compile_seq(chars, i, true);
        alts.push(toks);
        match chars.get(next) {
            Some(',') => i = next + 1,
            Some('}') => {
                if alts.len() < 2 {
                    return None;
                }
                return Some((Tok::Alt(alts), next + 1));
            }
            _ => return None,
        }
    }
}

fn match_toks(toks: &[Tok], s: &[char]) -> bool {
    let Some((first, rest)) = toks.split_first() else {
        return s.is_empty();
    };
    match first {
        Tok::Lit(c) => s.first() == Some(c) && match_toks(rest, &s[1..]),
        Tok::One => s.first().is_some_and(|c| *c != '/') && match_toks(rest, &s[1..]),
        Tok::Star => {
            let mut i = 0;
            loop {
                if match_toks(rest, &s[i..]) {
                    return true;
                }
                if i >= s.len() || s[i] == '/' {
                    return false;
                }
                i += 1;
            }
        }
        Tok::DoubleStar => (0..=s.len()).any(|i| match_toks(rest, &s[i..])),
        Tok::Dirs => {
            if match_toks(rest, s) {
                return true;
            }
            (0..s.len()).any(|i| s[i] == '/' && match_toks(rest, &s[i + 1..]))
        }
        Tok::Set {
            negate,
            chars,
            ranges,
        } => {
            let Some(&c) = s.first() else {
                return false;
            };
            let hit = chars.contains(&c) || ranges.iter().any(|(lo, hi)| *lo <= c && c <= *hi);
            hit != *negate && match_toks(rest, &s[1..])
        }
        Tok::Alt(alts) => alts.iter().any(|alt| {
            let mut joined = alt.clone();
            joined.extend_from_slice(rest);
            match_toks(&joined, s)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glob(pattern: &str, path: &str) -> bool {
        Glob::compile(pattern).matches(path)
    }

    #[test]
    fn globs_follow_the_specification() {
        assert!(glob("*", "a.rs"));
        assert!(glob("*", "a/b.rs"), "no slash: matches anywhere below");
        assert!(glob("*.rs", "a.rs"));
        assert!(glob("*.rs", "a/b.rs"));
        assert!(!glob("*.rs", "a.toml"));
        assert!(glob("src/*.rs", "src/a.rs"));
        assert!(!glob("src/*.rs", "src/x/a.rs"), "* stops at /");
        assert!(
            !glob("src/*.rs", "lib/src/a.rs"),
            "a slash anchors to the folder"
        );
        assert!(glob("/Makefile", "Makefile"));
        assert!(!glob("/Makefile", "sub/Makefile"));
        assert!(glob("src/**/*.rs", "src/x/y/a.rs"));
        assert!(glob("src/**/*.rs", "src/a.rs"), "**/ matches zero segments");
        assert!(glob("**/test_*.rs", "test_a.rs"));
        assert!(glob("**/test_*.rs", "a/b/test_a.rs"));
        assert!(!glob("**/test_*.rs", "a/b/atest_a.rs"));
        assert!(glob("{*.md,*.txt}", "README.md"));
        assert!(glob("{*.md,*.txt}", "docs/notes.txt"));
        assert!(!glob("{*.md,*.txt}", "a.rs"));
        assert!(glob("*.{js,ts}", "a.ts"));
        assert!(glob("{package.json,.prettierrc}", ".prettierrc"));
        assert!(glob("[!.]*", "a.rs"));
        assert!(!glob("[!.]*", ".hidden"));
        assert!(glob("[abc].rs", "b.rs"));
        assert!(!glob("[abc].rs", "d.rs"));
        assert!(glob("[a-c].rs", "c.rs"));
        assert!(glob("?.rs", "a.rs"));
        assert!(!glob("?.rs", "ab.rs"));
        assert!(glob("a\\*b", "a*b"));
        assert!(!glob("a\\*b", "axb"));
        assert!(glob("{single}", "{single}"), "no comma: a literal");
        assert!(glob("a[", "a["), "unclosed set: a literal");
    }

    #[test]
    fn later_sections_win_and_values_parse() {
        let mut c = EditorConfig::default();
        c.apply(
            "root = true\n\
             [*]\n\
             indent_style = space\n\
             indent_size = 2\n\
             end_of_line = lf\n\
             charset = UTF-8\n\
             trim_trailing_whitespace = true\n\
             insert_final_newline = true\n\
             \n\
             ; a comment\n\
             # another\n\
             [*.rs]\n\
             indent_size = 4\n\
             INDENT_STYLE = Tab\n",
            "src/main.rs",
        );
        assert_eq!(c.indent_style, Some(IndentStyle::Tabs));
        assert_eq!(c.indent_size, Some(4));
        assert_eq!(c.end_of_line, Some(EndOfLine::Lf));
        assert_eq!(c.charset.as_deref(), Some("utf-8"));
        assert_eq!(c.trim_trailing_whitespace, Some(true));
        assert_eq!(c.insert_final_newline, Some(true));
        assert!(!c.is_empty());

        let mut c = EditorConfig::default();
        c.apply("[*.rs]\nindent_size = 4\n", "notes.md");
        assert!(c.is_empty(), "no section matched");
    }

    #[test]
    fn indent_size_tab_resolves_to_tab_width() {
        let mut c = EditorConfig::default();
        c.apply("[*]\nindent_size = tab\ntab_width = 8\n", "a.rs");
        assert_eq!(c.indent_size, Some(8));
        assert_eq!(c.tab_width, Some(8));

        let mut c = EditorConfig::default();
        c.apply("[*]\ntab_width = 3\n[*.rs]\nindent_size = tab\n", "a.rs");
        assert_eq!(c.indent_size, Some(3), "tab_width from an earlier section");

        let mut c = EditorConfig::default();
        c.apply("[*]\nindent_size = tab\n", "a.rs");
        assert_eq!(
            c.indent_size, None,
            "no tab_width known: the editor's applies"
        );
    }

    #[test]
    fn unset_clears_and_malformed_lines_are_ignored() {
        let mut c = EditorConfig::default();
        c.apply(
            "[*]\n\
             indent_size = 2\n\
             end_of_line = crlf\n\
             this line has no equals sign\n\
             = novalue\n\
             [unclosed\n\
             indent_size = not a number\n\
             end_of_line = unset\n\
             tab_width = 0\n",
            "a.rs",
        );
        assert_eq!(c.indent_size, Some(2), "a bad value leaves the old one");
        assert_eq!(c.end_of_line, None);
        assert_eq!(c.tab_width, None, "zero is not a width");
    }

    #[test]
    fn nearer_files_win_and_root_stops_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().join("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(
            dir.path().join(".editorconfig"),
            "[*]\nindent_size = 8\ncharset = latin1\n",
        )
        .unwrap();
        std::fs::write(
            outer.join(".editorconfig"),
            "[*]\nindent_size = 2\nend_of_line = crlf\n[inner/*.rs]\nindent_style = tab\n",
        )
        .unwrap();
        std::fs::write(
            inner.join(".editorconfig"),
            "[*.rs]\nindent_size = 4\ntrim_trailing_whitespace = true\n",
        )
        .unwrap();

        let c = EditorConfig::for_path(&inner.join("main.rs"));
        assert_eq!(c.indent_size, Some(4), "the nearest file wins");
        assert_eq!(c.tab_width, Some(4), "tab_width follows indent_size");
        assert_eq!(
            c.indent_style,
            Some(IndentStyle::Tabs),
            "a path glob from the outer file"
        );
        assert_eq!(c.end_of_line, Some(EndOfLine::CrLf));
        assert_eq!(
            c.charset.as_deref(),
            Some("latin1"),
            "the farthest file still applies"
        );
        assert_eq!(c.trim_trailing_whitespace, Some(true));

        let c = EditorConfig::for_path(&inner.join("notes.md"));
        assert_eq!(c.indent_size, Some(2));
        assert_eq!(c.indent_style, None);

        std::fs::write(
            outer.join(".editorconfig"),
            "root = true\n[*]\nindent_size = 2\n",
        )
        .unwrap();
        let c = EditorConfig::for_path(&inner.join("main.rs"));
        assert_eq!(c.indent_size, Some(4));
        assert_eq!(c.charset, None, "root = true stopped the walk");

        let c = EditorConfig::for_path(&dir.path().join("nothing/here/x.rs"));
        assert_eq!(
            c.charset.as_deref(),
            Some("latin1"),
            "missing folders are fine"
        );
    }

    #[test]
    fn a_malformed_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".editorconfig"), b"\xff\xfe not utf-8 [*").unwrap();
        assert!(EditorConfig::for_path(&dir.path().join("a.rs")).is_empty());
    }
}
