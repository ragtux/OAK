//! How a document indents: the width of one level and whether it is written
//! as a tab or as spaces. The settings (`editor.tabSize`, `editor.indentStyle`)
//! give the default, `.editorconfig` may override it, and
//! `editor.autoDetectIndentation` reads it off the file's own lines.

/// One indentation level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indent {
    /// Columns per level; also the width a tab character displays at.
    pub width: usize,
    /// Write a level as one tab (`editor.indentStyle: "tab"`) instead of spaces.
    pub use_tabs: bool,
}

impl Default for Indent {
    /// Fleet's default: 4 spaces.
    fn default() -> Self {
        Indent {
            width: crate::TAB_WIDTH,
            use_tabs: false,
        }
    }
}

impl Indent {
    pub fn spaces(width: usize) -> Indent {
        Indent {
            width: width.max(1),
            use_tabs: false,
        }
    }

    pub fn tabs(width: usize) -> Indent {
        Indent {
            width: width.max(1),
            use_tabs: true,
        }
    }

    /// The text of one level.
    pub fn unit(&self) -> String {
        if self.use_tabs {
            "\t".to_string()
        } else {
            " ".repeat(self.width)
        }
    }

    /// The text that indents to `columns`: whole tabs then spaces when tabs
    /// are in use, spaces otherwise.
    pub fn to_columns(&self, columns: usize) -> String {
        if self.use_tabs {
            let tabs = columns / self.width;
            let spaces = columns % self.width;
            let mut s = "\t".repeat(tabs);
            s.push_str(&" ".repeat(spaces));
            s
        } else {
            " ".repeat(columns)
        }
    }

    /// What `text` indents with, read off its lines (`editor.autoDetectIndentation`):
    /// tabs when more indented lines start with a tab than with spaces, else the
    /// most common positive difference in leading spaces between consecutive
    /// non-blank lines (2, 4 or 8 win ties, in that order). `None` when the text
    /// has no indented line to learn from.
    pub fn detect(text: &str) -> Option<Indent> {
        let mut tab_lines = 0usize;
        let mut space_lines = 0usize;
        let mut deltas = [0usize; 9];
        let mut prev_indent: Option<usize> = None;
        for line in text.lines().take(10_000) {
            if line.trim().is_empty() {
                continue;
            }
            if line.starts_with('\t') {
                tab_lines += 1;
                prev_indent = None;
                continue;
            }
            let spaces = line.chars().take_while(|c| *c == ' ').count();
            if spaces > 0 {
                space_lines += 1;
            }
            if let Some(prev) = prev_indent {
                let delta = spaces.abs_diff(prev);
                if (1..=8).contains(&delta) {
                    deltas[delta] += 1;
                }
            }
            prev_indent = Some(spaces);
        }
        if tab_lines == 0 && space_lines == 0 {
            return None;
        }
        if tab_lines > space_lines {
            return Some(Indent::tabs(crate::TAB_WIDTH));
        }
        let best = [2usize, 4, 8, 3, 1, 5, 6, 7]
            .into_iter()
            .filter(|w| deltas[*w] > 0)
            .max_by_key(|w| deltas[*w])?;
        Some(Indent::spaces(best))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_two_and_four_spaces_and_tabs() {
        assert_eq!(
            Indent::detect("fn a() {\n  x;\n  if y {\n    z;\n  }\n}\n"),
            Some(Indent::spaces(2))
        );
        assert_eq!(
            Indent::detect("fn a() {\n    x;\n}\n"),
            Some(Indent::spaces(4))
        );
        assert_eq!(
            Indent::detect("fn a() {\n\tx;\n\tif y {\n\t\tz;\n\t}\n}\n"),
            Some(Indent::tabs(4))
        );
        assert_eq!(Indent::detect("a\nb\n"), None);
    }

    #[test]
    fn indent_text() {
        assert_eq!(Indent::spaces(2).unit(), "  ");
        assert_eq!(Indent::tabs(4).to_columns(6), "\t  ");
        assert_eq!(Indent::spaces(4).to_columns(6), "      ");
    }
}
