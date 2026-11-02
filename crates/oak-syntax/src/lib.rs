//! Syntax highlighting: tree-sitter grammars and their standard capture
//! names, mapped onto the keys of Fleet's `textAttributes` table so a theme
//! colours Oak's editor the way it colours Fleet's.
//!
//! The whole document is highlighted on every call for now; incremental
//! parsing is the first thing to add once files get large.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter};

pub mod structure;
pub mod symbols;
pub use structure::{enclosing_heads, Fold, SyntaxTree};
pub use symbols::{Symbol, SymbolKind};

/// The languages Oak ships grammars for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Json,
    Toml,
    Markdown,
}

impl Language {
    pub fn from_path(path: &Path) -> Option<Language> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Language::Rust,
            "json" | "jsonc" => Language::Json,
            "toml" => Language::Toml,
            "md" | "markdown" => Language::Markdown,
            _ => return None,
        })
    }

    /// The name the status bar shows, Fleet's spelling.
    pub fn display_name(self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::Json => "JSON",
            Language::Toml => "TOML",
            Language::Markdown => "Markdown",
        }
    }

    /// The line-comment prefix `toggle_line_comment` uses.
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Language::Rust => Some("// "),
            Language::Toml => Some("# "),
            Language::Json | Language::Markdown => None,
        }
    }

    /// The block-comment delimiters, for a block-comment toggle.
    pub fn block_comment(self) -> Option<(&'static str, &'static str)> {
        match self {
            Language::Rust => Some(("/*", "*/")),
            Language::Markdown => Some(("<!--", "-->")),
            Language::Json | Language::Toml => None,
        }
    }

    /// Whether typing an opening bracket or quote inserts its closer.
    pub fn auto_pairs(self) -> bool {
        match self {
            Language::Rust | Language::Json | Language::Toml | Language::Markdown => true,
        }
    }

    pub fn all() -> [Language; 4] {
        [
            Language::Rust,
            Language::Json,
            Language::Toml,
            Language::Markdown,
        ]
    }
}

/// Tree-sitter's conventional capture names, in the order the highlighter
/// is configured with, paired with the Fleet attribute key each paints as.
/// `configure` matches a capture to the longest of these that prefixes it,
/// so `function.method.call` lands on `function.method`.
const CAPTURES: &[(&str, &str)] = &[
    ("attribute", "metadata"),
    ("boolean", "boolean"),
    ("comment", "comment"),
    ("comment.documentation", "comment.doc"),
    ("constant", "identifier.constant"),
    ("constant.builtin", "boolean"),
    ("constructor", "identifier.type"),
    ("embedded", "identifier"),
    ("escape", "string.escape"),
    ("function", "identifier.function.call"),
    ("function.builtin", "identifier.function.call"),
    ("function.macro", "identifier.function.call"),
    ("function.method", "identifier.function.call"),
    ("keyword", "keyword"),
    ("label", "identifier.constant"),
    ("number", "number"),
    ("operator", "punctuation.operator"),
    ("property", "identifier.field"),
    ("punctuation", "punctuation"),
    ("punctuation.bracket", "punctuation"),
    ("punctuation.delimiter", "punctuation"),
    ("punctuation.special", "punctuation"),
    ("string", "string"),
    ("string.special", "string"),
    ("string.special.key", "key.json"),
    ("tag", "tagName.html"),
    ("type", "identifier.type"),
    ("type.builtin", "identifier.type"),
    ("variable", "identifier.variable"),
    ("variable.builtin", "identifier.this"),
    ("variable.parameter", "identifier.parameter"),
    ("text.title", "markup.heading"),
    ("text.literal", "markup.code.block"),
    ("text.reference", "link"),
    ("text.uri", "link"),
    ("text.emphasis", "markup.italic"),
    ("text.strong", "markup.bold"),
];

/// A highlighted byte range and the Fleet attribute key it paints as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub attribute: &'static str,
}

pub struct Highlighter {
    configs: HashMap<Language, HighlightConfiguration>,
    inner: TsHighlighter,
}

impl Default for Highlighter {
    fn default() -> Self {
        Highlighter::new()
    }
}

impl Highlighter {
    pub fn new() -> Self {
        let names: Vec<&str> = CAPTURES.iter().map(|(n, _)| *n).collect();
        let mut configs = HashMap::new();
        let mut add = |lang: Language, config: Result<HighlightConfiguration, _>| match config {
            Ok(mut c) => {
                c.configure(&names);
                configs.insert(lang, c);
            }
            Err(e) => eprintln!("oak-syntax: {lang:?} grammar failed to load: {e}"),
        };
        add(
            Language::Rust,
            HighlightConfiguration::new(
                tree_sitter_rust::LANGUAGE.into(),
                "rust",
                tree_sitter_rust::HIGHLIGHTS_QUERY,
                tree_sitter_rust::INJECTIONS_QUERY,
                "",
            ),
        );
        add(
            Language::Json,
            HighlightConfiguration::new(
                tree_sitter_json::LANGUAGE.into(),
                "json",
                // tree-sitter-highlight lets the last matching pattern win, and
                // the grammar's `(string) @string` comes after its key pattern;
                // restating the key pattern last gives object keys their own colour.
                &format!(
                    "{}\n(pair key: (_) @string.special.key)",
                    tree_sitter_json::HIGHLIGHTS_QUERY
                ),
                "",
                "",
            ),
        );
        add(
            Language::Toml,
            HighlightConfiguration::new(
                tree_sitter_toml_ng::LANGUAGE.into(),
                "toml",
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
                "",
                "",
            ),
        );
        add(
            Language::Markdown,
            HighlightConfiguration::new(
                tree_sitter_md::LANGUAGE.into(),
                "markdown",
                tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
                tree_sitter_md::INJECTION_QUERY_BLOCK,
                "",
            ),
        );
        Highlighter {
            configs,
            inner: TsHighlighter::new(),
        }
    }

    pub fn supports(&self, language: Language) -> bool {
        self.configs.contains_key(&language)
    }

    /// Highlights `source` in full. Nested captures resolve to the innermost.
    pub fn highlight(&mut self, language: Language, source: &[u8]) -> Vec<Span> {
        let Some(config) = self.configs.get(&language) else {
            return Vec::new();
        };
        let Ok(events) = self.inner.highlight(config, source, None, |_| None) else {
            return Vec::new();
        };
        let mut spans = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        for event in events.flatten() {
            match event {
                HighlightEvent::HighlightStart(h) => stack.push(h.0),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    if let Some((_, attribute)) = stack.last().and_then(|h| CAPTURES.get(*h)) {
                        spans.push(Span {
                            range: start..end,
                            attribute,
                        });
                    }
                }
            }
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs_for(lang: Language, src: &str, needle: &str) -> Vec<&'static str> {
        let mut h = Highlighter::new();
        let spans = h.highlight(lang, src.as_bytes());
        let at = src.find(needle).unwrap();
        spans
            .iter()
            .filter(|s| s.range.start <= at && at < s.range.end)
            .map(|s| s.attribute)
            .collect()
    }

    #[test]
    fn rust_keywords_strings_and_calls() {
        let src = "fn main() { let s = \"hi\"; println!(\"{}\", s); } // done";
        assert_eq!(attrs_for(Language::Rust, src, "fn"), ["keyword"]);
        assert_eq!(attrs_for(Language::Rust, src, "\"hi\""), ["string"]);
        assert_eq!(
            attrs_for(Language::Rust, src, "main"),
            ["identifier.function.call"]
        );
        assert_eq!(attrs_for(Language::Rust, src, "// done"), ["comment"]);
    }

    #[test]
    fn json_keys_and_values() {
        let src = r#"{ "name": "oak", "n": 3, "ok": true }"#;
        assert_eq!(attrs_for(Language::Json, src, "\"name\""), ["key.json"]);
        assert_eq!(attrs_for(Language::Json, src, "\"oak\""), ["string"]);
        assert_eq!(attrs_for(Language::Json, src, "3"), ["number"]);
        assert_eq!(attrs_for(Language::Json, src, "true"), ["boolean"]);
    }

    #[test]
    fn toml_and_markdown_load() {
        let mut h = Highlighter::new();
        assert!(!h
            .highlight(Language::Toml, b"[package]\nname = \"oak\"\n")
            .is_empty());
        assert!(!h
            .highlight(Language::Markdown, b"# Title\n\ntext\n")
            .is_empty());
    }

    #[test]
    fn language_from_path() {
        assert_eq!(
            Language::from_path(Path::new("a/b.rs")),
            Some(Language::Rust)
        );
        assert_eq!(
            Language::from_path(Path::new("Cargo.toml")),
            Some(Language::Toml)
        );
        assert_eq!(Language::from_path(Path::new("x.txt")), None);
    }
}
