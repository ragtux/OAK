//! Hover and documentation text as plain lines. Servers answer in Markdown
//! (rust-analyzer always does); WEEP's popups paint lines, a code block's in
//! the editor face, so this keeps the text and drops the markers: headings,
//! emphasis, inline code ticks, fences, and a rule becomes a separator.

use lsp_types::{Documentation, Hover, HoverContents, MarkedString, MarkupContent};

/// One line of a popup: in the mono face when it came from a code block;
/// a separator for a horizontal rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub code: bool,
    pub separator: bool,
}

/// The Markdown of a hover, as the server sent it.
pub fn hover_markdown(hover: &Hover) -> String {
    match &hover.contents {
        HoverContents::Scalar(s) => marked(s),
        HoverContents::Array(items) => items.iter().map(marked).collect::<Vec<_>>().join("\n\n"),
        HoverContents::Markup(MarkupContent { value, .. }) => value.clone(),
    }
}

fn marked(s: &MarkedString) -> String {
    match s {
        MarkedString::String(s) => s.clone(),
        MarkedString::LanguageString(l) => format!("```{}\n{}\n```", l.language, l.value),
    }
}

/// The Markdown of a completion item's or signature's documentation.
pub fn documentation_markdown(doc: &Documentation) -> String {
    match doc {
        Documentation::String(s) => s.clone(),
        Documentation::MarkupContent(MarkupContent { value, .. }) => value.clone(),
    }
}

/// Markdown as lines: fenced blocks keep their text (marked `code`), a
/// `---` rule is a separator, headings lose their hashes, `**`, `*`, `_`
/// and backtick markers go, blank lines collapse to one.
pub fn lines(markdown: &str) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let mut in_code = false;
    for raw in markdown.lines() {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push(Line {
                text: line.to_string(),
                code: true,
                separator: false,
            });
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if out.last().is_some_and(|l| !l.text.is_empty() || l.code) {
                out.push(Line {
                    text: String::new(),
                    code: false,
                    separator: false,
                });
            }
            continue;
        }
        if trimmed.chars().all(|c| c == '-' || c == '*' || c == '_') && trimmed.len() >= 3 {
            // A rule; a blank line before it is redundant.
            if out.last().is_some_and(|l| l.text.is_empty() && !l.code) {
                out.pop();
            }
            out.push(Line {
                text: String::new(),
                code: false,
                separator: true,
            });
            continue;
        }
        let text = trimmed.trim_start_matches('#').trim_start();
        out.push(Line {
            text: strip_inline(text),
            code: false,
            separator: false,
        });
    }
    while out
        .last()
        .is_some_and(|l| l.text.is_empty() && !l.code && !l.separator)
    {
        out.pop();
    }
    out
}

/// Inline markers removed: backticks, `**`, `__`, single `*` and `_` around
/// words, and `[text](url)` reduced to its text.
fn strip_inline(text: &str) -> String {
    let mut s = text.replace("**", "").replace("__", "").replace('`', "");
    // Links.
    while let Some(open) = s.find("](") {
        let Some(start) = s[..open].rfind('[') else {
            break;
        };
        let Some(close) = s[open..].find(')') else {
            break;
        };
        let label = s[start + 1..open].to_string();
        s.replace_range(start..open + close + 1, &label);
    }
    // Emphasis around a word: `*word*` or `_word_`.
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if (c == '*' || c == '_') && i + 1 < chars.len() && !chars[i + 1].is_whitespace() {
            if let Some(end) = chars[i + 1..].iter().position(|&x| x == c) {
                let end = i + 1 + end;
                if end > i + 1 && !chars[end - 1].is_whitespace() {
                    out.extend(chars[i + 1..end].iter());
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_blocks_rules_and_markers() {
        let md = "```rust\nfn main()\n```\n\n---\n\n# Title\n\nSome `code` and **bold** and *it* [link](http://x).\n\n\n";
        let lines = lines(md);
        assert_eq!(
            lines,
            vec![
                Line {
                    text: "fn main()".into(),
                    code: true,
                    separator: false
                },
                Line {
                    text: String::new(),
                    code: false,
                    separator: true
                },
                Line {
                    text: "Title".into(),
                    code: false,
                    separator: false
                },
                Line {
                    text: String::new(),
                    code: false,
                    separator: false
                },
                Line {
                    text: "Some code and bold and it link.".into(),
                    code: false,
                    separator: false
                },
            ]
        );
    }

    #[test]
    fn hover_shapes() {
        let h = Hover {
            contents: HoverContents::Scalar(MarkedString::String("x".into())),
            range: None,
        };
        assert_eq!(hover_markdown(&h), "x");
        let h = Hover {
            contents: HoverContents::Array(vec![MarkedString::LanguageString(
                lsp_types::LanguageString {
                    language: "rust".into(),
                    value: "fn f()".into(),
                },
            )]),
            range: None,
        };
        assert_eq!(lines(&hover_markdown(&h))[0].text, "fn f()");
    }
}
