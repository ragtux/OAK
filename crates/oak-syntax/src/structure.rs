//! Structure on top of a real `tree_sitter::Tree`: what the editor needs for
//! expand/shrink selection, folding, bracket matching and sticky scope lines.
//!
//! Every offset here is a BYTE offset into the source text (the editor maps
//! them to char offsets itself); rows are 0-based line indices.

use crate::Language;
use std::cmp::Reverse;
use std::ops::Range;
use tree_sitter::{InputEdit, Node, Parser, Tree};

/// A foldable region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    /// The row holding the fold's head (the opener).
    pub start_row: usize,
    /// The row holding the closer; always `> start_row`.
    pub end_row: usize,
    /// The bytes folded away: from just after the opening delimiter to just
    /// before the closing one (for a block comment: after `/*` to before `*/`;
    /// for a markdown section or TOML table: from the end of the first line to
    /// the node's end, trailing blank lines excluded).
    pub inner: Range<usize>,
}

/// A parsed document: the tree plus the parser that produced it, kept so a
/// re-parse can reuse the old tree.
pub struct SyntaxTree {
    language: Language,
    parser: Parser,
    tree: Tree,
}

impl std::fmt::Debug for SyntaxTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyntaxTree")
            .field("language", &self.language)
            .field("root", &self.tree.root_node().to_sexp())
            .finish()
    }
}

fn grammar(language: Language) -> tree_sitter::Language {
    match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Json => tree_sitter_json::LANGUAGE.into(),
        Language::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        Language::Markdown => tree_sitter_md::LANGUAGE.into(),
    }
}

const OPENERS: [&str; 3] = ["{", "[", "("];

fn closer_of(opener: &str) -> Option<&'static str> {
    Some(match opener {
        "{" => "}",
        "[" => "]",
        "(" => ")",
        "<" => ">",
        _ => return None,
    })
}

fn opener_of(closer: &str) -> Option<&'static str> {
    Some(match closer {
        "}" => "{",
        "]" => "[",
        ")" => "(",
        ">" => "<",
        _ => return None,
    })
}

/// Rust nodes whose `<` and `>` children are a bracket pair rather than
/// comparison operators.
const ANGLE_PARENTS: [&str; 4] = [
    "type_arguments",
    "type_parameters",
    "bracketed_type",
    "for_lifetimes",
];

/// Visits every node in pre-order; `f` returns whether to descend into the
/// node's children.
fn walk(root: Node<'_>, mut f: impl FnMut(Node<'_>) -> bool) {
    let mut cursor = root.walk();
    loop {
        let descend = f(cursor.node());
        if descend && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

fn byte_range(node: Node<'_>) -> Range<usize> {
    node.start_byte()..node.end_byte()
}

impl SyntaxTree {
    /// Parses `text` with the language's grammar; None when none is bundled or
    /// parsing fails.
    pub fn parse(language: Language, text: &str) -> Option<SyntaxTree> {
        let mut parser = Parser::new();
        parser.set_language(&grammar(language)).ok()?;
        let tree = parser.parse(text, None)?;
        Some(SyntaxTree {
            language,
            parser,
            tree,
        })
    }

    /// Re-parses `text` from scratch (the tree is kept as it was if parsing
    /// fails, which only happens on cancellation or a timeout).
    pub fn reparse(&mut self, text: &str) {
        if let Some(tree) = self.parser.parse(text, None) {
            self.tree = tree;
        }
    }

    /// Re-parses after one edit, reusing the unchanged parts of the old tree.
    /// `edit` describes the change in bytes and points as tree-sitter wants it;
    /// `text` is the document after the edit.
    pub fn reparse_edited(&mut self, edit: &InputEdit, text: &str) {
        self.tree.edit(edit);
        if let Some(tree) = self.parser.parse(text, Some(&self.tree)) {
            self.tree = tree;
        }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// The underlying tree, for callers that want to walk it themselves.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The byte range of the smallest NAMED node that contains `range` and is
    /// strictly larger than it, so repeated calls walk outwards: word ->
    /// expression -> statement -> block -> ... -> whole file. None once
    /// `range` is the root's range. Anonymous nodes (punctuation) do not count,
    /// and a node with the same range as `range` is skipped.
    pub fn expand(&self, range: Range<usize>) -> Option<Range<usize>> {
        let root = self.tree.root_node();
        let mut node = root.descendant_for_byte_range(range.start, range.end)?;
        loop {
            let r = byte_range(node);
            let strictly_larger = r.start <= range.start && r.end >= range.end && (r != range);
            if node.is_named() && strictly_larger {
                return Some(r);
            }
            node = node.parent()?;
        }
    }

    /// The reverse step for a selection that is exactly some node's range: the
    /// largest named child of that node that contains `byte` (the caret) or,
    /// when none does, the first named child. None when `range` is not a
    /// node's range or the node has no named children.
    pub fn shrink(&self, range: Range<usize>, byte: usize) -> Option<Range<usize>> {
        let root = self.tree.root_node();
        let mut node = root.descendant_for_byte_range(range.start, range.end)?;
        // The deepest node with exactly this range; if it is anonymous (a
        // keyword standing alone), the named ancestor with the same range.
        loop {
            let r = byte_range(node);
            if r.start < range.start || r.end > range.end {
                return None;
            }
            if r == range && node.is_named() {
                break;
            }
            node = node.parent()?;
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        let best = children
            .iter()
            .filter(|c| c.start_byte() <= byte && byte <= c.end_byte())
            .max_by_key(|c| c.end_byte() - c.start_byte())
            .or_else(|| children.first());
        best.map(|c| byte_range(*c))
    }

    /// Every foldable region, sorted by start row, then by start byte, then by
    /// length descending (outer before inner).
    ///
    /// A node is foldable when it spans more than one row and either its first
    /// child is an anonymous `{`, `[` or `(` and its last child the matching
    /// closer (rust blocks, declaration lists, struct bodies, enum variant
    /// lists, match blocks, use lists, arrays, call arguments, parameter lists;
    /// json objects and arrays; toml arrays and inline tables), or it is a
    /// language-specific region: rust `block_comment`, markdown `section`
    /// (one with a heading), toml `table` and `table_array_element`.
    pub fn folds(&self, text: &str) -> Vec<Fold> {
        let mut folds = Vec::new();
        let language = self.language;
        walk(self.tree.root_node(), |node| {
            if node.start_position().row == node.end_position().row {
                // Nothing inside a single row can span rows either.
                return false;
            }
            if let Some(fold) = bracket_fold(node).or_else(|| region_fold(language, node, text)) {
                folds.push(fold);
            }
            true
        });
        folds.sort_by_key(|f| (f.start_row, f.inner.start, Reverse(f.inner.end)));
        folds.dedup();
        folds
    }

    /// The bracket pair when `byte` is at (or right after) an anonymous
    /// bracket token `(` `)` `[` `]` `{` `}`, or `<` `>` where the grammar
    /// makes them a pair (rust type arguments): the byte offsets of the opener
    /// and the closer, found as sibling tokens inside the same parent. None
    /// otherwise; brackets inside strings and comments are plain text to the
    /// grammar, so they never match.
    pub fn matching_bracket(&self, byte: usize) -> Option<(usize, usize)> {
        let root = self.tree.root_node();
        let at = |b: usize| {
            root.descendant_for_byte_range(b, b)
                .filter(|n| !n.is_named() && is_bracket_token(*n))
        };
        let token = at(byte).or_else(|| byte.checked_sub(1).and_then(at))?;
        let kind = token.kind();
        let parent = token.parent()?;
        if matches!(kind, "<" | ">") && !ANGLE_PARENTS.contains(&parent.kind()) {
            return None;
        }
        let mut cursor = parent.walk();
        let siblings: Vec<Node> = parent.children(&mut cursor).collect();
        let index = siblings
            .iter()
            .position(|s| s.start_byte() == token.start_byte() && s.kind() == kind)?;
        if let Some(closer) = closer_of(kind) {
            let close = siblings[index + 1..]
                .iter()
                .find(|s| !s.is_named() && s.kind() == closer)?;
            Some((token.start_byte(), close.start_byte()))
        } else {
            let opener = opener_of(kind)?;
            let open = siblings[..index]
                .iter()
                .rev()
                .find(|s| !s.is_named() && s.kind() == opener)?;
            Some((open.start_byte(), token.start_byte()))
        }
    }

    /// Syntax error ranges: `ERROR` nodes and the zero-width `MISSING` tokens
    /// the parser inserted to recover.
    pub fn errors(&self) -> Vec<Range<usize>> {
        let root = self.tree.root_node();
        if !root.has_error() {
            return Vec::new();
        }
        let mut errors = Vec::new();
        walk(root, |node| {
            if node.is_error() || node.is_missing() {
                errors.push(byte_range(node));
                return false;
            }
            node.has_error()
        });
        errors
    }

    /// The syntax errors with a message each (M6): an `ERROR` node is a
    /// "Syntax error", a zero-width `MISSING` token names what the parser
    /// inserted. Byte ranges, in document order.
    pub fn problems(&self) -> Vec<SyntaxProblem> {
        let root = self.tree.root_node();
        if !root.has_error() {
            return Vec::new();
        }
        let mut problems = Vec::new();
        walk(root, |node| {
            if node.is_missing() {
                problems.push(SyntaxProblem {
                    range: byte_range(node),
                    message: format!("Missing '{}'", node.kind()),
                });
                return false;
            }
            if node.is_error() {
                problems.push(SyntaxProblem {
                    range: byte_range(node),
                    message: "Syntax error".to_string(),
                });
                return false;
            }
            node.has_error()
        });
        problems.sort_by_key(|p| (p.range.start, p.range.end));
        problems
    }
}

/// One syntax error: where (bytes) and what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxProblem {
    pub range: Range<usize>,
    pub message: String,
}

fn is_bracket_token(node: Node<'_>) -> bool {
    matches!(node.kind(), "(" | ")" | "[" | "]" | "{" | "}" | "<" | ">")
}

/// Rule (a): a node delimited by an anonymous bracket pair.
fn bracket_fold(node: Node<'_>) -> Option<Fold> {
    let first = node.child(0)?;
    let last = node.child(node.child_count() - 1)?;
    if first.is_named() || last.is_named() || first.id() == last.id() {
        return None;
    }
    if !OPENERS.contains(&first.kind()) || closer_of(first.kind())? != last.kind() {
        return None;
    }
    let start_row = node.start_position().row;
    let end_row = last.start_position().row;
    (end_row > start_row).then(|| Fold {
        start_row,
        end_row,
        inner: first.end_byte()..last.start_byte(),
    })
}

/// Rule (b): a language-specific region.
fn region_fold(language: Language, node: Node<'_>, text: &str) -> Option<Fold> {
    match (language, node.kind()) {
        (Language::Rust, "block_comment") => {
            let range = byte_range(node);
            let body = text.get(range.clone())?;
            if !body.starts_with("/*") || !body.ends_with("*/") || body.len() < 4 {
                return None;
            }
            let start_row = node.start_position().row;
            let end_row = node.end_position().row;
            (end_row > start_row).then(|| Fold {
                start_row,
                end_row,
                inner: range.start + 2..range.end - 2,
            })
        }
        (Language::Markdown, "section") => {
            let heading = node.named_child(0)?;
            if !matches!(heading.kind(), "atx_heading" | "setext_heading") {
                return None;
            }
            line_region_fold(node, text)
        }
        (Language::Toml, "table" | "table_array_element") => line_region_fold(node, text),
        _ => None,
    }
}

/// A fold from the end of the node's first line to the end of its last
/// non-blank line.
fn line_region_fold(node: Node<'_>, text: &str) -> Option<Fold> {
    let range = byte_range(node);
    let body = text.get(range.clone())?;
    let first_line_end = range.start + body.find('\n')?;
    let trimmed_end = range.start + body.trim_end().len();
    if trimmed_end <= first_line_end {
        return None;
    }
    let start_row = node.start_position().row;
    let end_row = start_row + text[range.start..trimmed_end].matches('\n').count();
    (end_row > start_row).then_some(Fold {
        start_row,
        end_row,
        inner: first_line_end..trimmed_end,
    })
}

/// The heads of the folds enclosing `row` (`start_row < row <= end_row`),
/// outermost first: what the editor shows as sticky lines when `row` is the
/// first visible row.
pub fn enclosing_heads(folds: &[Fold], row: usize) -> Vec<usize> {
    let mut heads: Vec<usize> = folds
        .iter()
        .filter(|f| f.start_row < row && row <= f.end_row)
        .map(|f| f.start_row)
        .collect();
    heads.sort_unstable();
    heads.dedup();
    heads
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: &str = "fn main() {\n    let s = \"a)\";\n    foo(1, (2, 3));\n    if s.is_empty() {\n        bar();\n    }\n}\n";

    const JSON: &str = "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": true\n}\n";

    const MARKDOWN: &str = "# One\ntext one\n\n## Two\ntext two\n";

    const TOML: &str = "[a]\nx = 1\n\n[b]\ny = 2\n";

    /// The byte range of the first occurrence of `needle` in `text`.
    fn span(text: &str, needle: &str) -> Range<usize> {
        let start = text.find(needle).unwrap();
        start..start + needle.len()
    }

    fn at(text: &str, needle: &str) -> usize {
        text.find(needle).unwrap()
    }

    fn parse(language: Language, text: &str) -> SyntaxTree {
        SyntaxTree::parse(language, text).expect("grammar parses")
    }

    #[test]
    fn parses_every_language() {
        for language in Language::all() {
            let tree = SyntaxTree::parse(language, "").unwrap();
            assert_eq!(tree.language(), language);
        }
        parse(Language::Rust, RUST);
        parse(Language::Json, JSON);
        parse(Language::Markdown, MARKDOWN);
        parse(Language::Toml, TOML);
    }

    #[test]
    fn expand_walks_outwards_to_the_root() {
        let tree = parse(Language::Rust, RUST);
        let two = at(RUST, "2");
        // The caret alone grows to the literal under it.
        let mut range = tree.expand(two..two).unwrap();
        assert_eq!(range, span(RUST, "2"));
        let block_end = RUST.rfind('}').unwrap() + 1;
        let expected = [
            span(RUST, "(2, 3)"),
            span(RUST, "(1, (2, 3))"),
            span(RUST, "foo(1, (2, 3))"),
            span(RUST, "foo(1, (2, 3));"),
            at(RUST, "{")..block_end,
            0..block_end,
        ];
        for want in expected {
            range = tree.expand(range.clone()).unwrap();
            assert_eq!(range, want);
        }
        // `fn main ... }` is the only item; the source file is the last step
        // (its range may also take in the trailing newline) and stops the walk.
        match tree.expand(range.clone()) {
            None => {}
            Some(root) => {
                assert_eq!(root.start, 0);
                assert!(root.end >= block_end);
                assert_eq!(tree.expand(root), None);
            }
        }
    }

    #[test]
    fn shrink_reverses_one_step() {
        let tree = parse(Language::Rust, RUST);
        let call = span(RUST, "foo(1, (2, 3))");
        let args = span(RUST, "(1, (2, 3))");
        let tuple = span(RUST, "(2, 3)");
        let two = at(RUST, "2");
        assert_eq!(tree.shrink(call.clone(), two), Some(args.clone()));
        assert_eq!(tree.shrink(args.clone(), two), Some(tuple.clone()));
        assert_eq!(tree.shrink(tuple.clone(), two), Some(span(RUST, "2")));
        // The caret on the callee picks the callee.
        assert_eq!(
            tree.shrink(call.clone(), at(RUST, "foo")),
            Some(span(RUST, "foo"))
        );
        // A caret outside the node falls back to the first named child.
        assert_eq!(tree.shrink(call, 0), Some(span(RUST, "foo")));
        // A leaf has nothing to shrink to; a non-node range is not a step.
        assert_eq!(tree.shrink(span(RUST, "2"), two), None);
        assert_eq!(tree.shrink(two..two + 4, two), None);
    }

    #[test]
    fn rust_folds_are_the_two_blocks() {
        let tree = parse(Language::Rust, RUST);
        let folds = tree.folds(RUST);
        let outer_close = RUST.rfind('}').unwrap();
        let inner_close = RUST[..outer_close].rfind('}').unwrap();
        assert_eq!(
            folds,
            vec![
                Fold {
                    start_row: 0,
                    end_row: 6,
                    inner: at(RUST, "{") + 1..outer_close,
                },
                Fold {
                    start_row: 3,
                    end_row: 5,
                    inner: at(RUST, "{\n        bar") + 1..inner_close,
                },
            ]
        );
    }

    #[test]
    fn rust_block_comment_and_other_bracket_lists_fold() {
        let src = "/* a\n   b */\nstruct S {\n    x: u8,\n}\nuse a::{\n    b,\n    c,\n};\n";
        let tree = parse(Language::Rust, src);
        let folds = tree.folds(src);
        assert_eq!(
            folds,
            vec![
                Fold {
                    start_row: 0,
                    end_row: 1,
                    inner: 2..at(src, "*/"),
                },
                Fold {
                    start_row: 2,
                    end_row: 4,
                    inner: at(src, "{\n    x") + 1..at(src, "}\nuse"),
                },
                Fold {
                    start_row: 5,
                    end_row: 8,
                    inner: at(src, "{\n    b") + 1..at(src, "};"),
                },
            ]
        );
    }

    #[test]
    fn json_folds_object_then_array() {
        let tree = parse(Language::Json, JSON);
        let folds = tree.folds(JSON);
        assert_eq!(
            folds,
            vec![
                Fold {
                    start_row: 0,
                    end_row: 6,
                    inner: 1..JSON.rfind('}').unwrap(),
                },
                Fold {
                    start_row: 1,
                    end_row: 4,
                    inner: at(JSON, "[") + 1..at(JSON, "]"),
                },
            ]
        );
    }

    #[test]
    fn markdown_sections_fold_from_the_heading_line() {
        let tree = parse(Language::Markdown, MARKDOWN);
        let folds = tree.folds(MARKDOWN);
        let end = MARKDOWN.trim_end().len();
        assert_eq!(
            folds,
            vec![
                Fold {
                    start_row: 0,
                    end_row: 4,
                    inner: "# One".len()..end,
                },
                Fold {
                    start_row: 3,
                    end_row: 4,
                    inner: at(MARKDOWN, "## Two") + "## Two".len()..end,
                },
            ]
        );
    }

    #[test]
    fn toml_tables_fold() {
        let tree = parse(Language::Toml, TOML);
        let folds = tree.folds(TOML);
        assert_eq!(
            folds,
            vec![
                Fold {
                    start_row: 0,
                    end_row: 1,
                    inner: 3..at(TOML, "x = 1") + 5,
                },
                Fold {
                    start_row: 3,
                    end_row: 4,
                    inner: at(TOML, "[b]") + 3..TOML.trim_end().len(),
                },
            ]
        );
    }

    #[test]
    fn matching_bracket_uses_the_grammar() {
        let tree = parse(Language::Rust, RUST);
        let open = at(RUST, "(1, (2, 3))");
        let close = open + "(1, (2, 3)".len();
        assert_eq!(tree.matching_bracket(open), Some((open, close)));
        assert_eq!(tree.matching_bracket(close), Some((open, close)));
        // Right after the closer counts too.
        assert_eq!(tree.matching_bracket(close + 1), Some((open, close)));
        let inner_open = at(RUST, "(2, 3)");
        assert_eq!(
            tree.matching_bracket(inner_open),
            Some((inner_open, inner_open + 5))
        );
        // The `)` inside the string is plain text.
        assert_eq!(tree.matching_bracket(at(RUST, "a)") + 1), None);
        // An identifier is not a bracket.
        assert_eq!(tree.matching_bracket(at(RUST, "foo")), None);
        // Braces.
        let brace = at(RUST, "{");
        assert_eq!(
            tree.matching_bracket(brace),
            Some((brace, RUST.rfind('}').unwrap()))
        );
    }

    #[test]
    fn matching_bracket_angles_only_in_type_arguments() {
        let src = "fn f() { let v: Vec<u8> = Vec::new(); let b = 1 < 2; }";
        let tree = parse(Language::Rust, src);
        let lt = at(src, "<u8>");
        assert_eq!(tree.matching_bracket(lt), Some((lt, lt + 3)));
        assert_eq!(tree.matching_bracket(lt + 3), Some((lt, lt + 3)));
        assert_eq!(tree.matching_bracket(at(src, "< 2")), None);
    }

    #[test]
    fn enclosing_heads_outer_then_inner() {
        let tree = parse(Language::Rust, RUST);
        let folds = tree.folds(RUST);
        assert_eq!(enclosing_heads(&folds, 4), vec![0, 3]);
        assert_eq!(enclosing_heads(&folds, 5), vec![0, 3]);
        assert_eq!(enclosing_heads(&folds, 6), vec![0]);
        assert_eq!(enclosing_heads(&folds, 3), vec![0]);
        assert_eq!(enclosing_heads(&folds, 0), Vec::<usize>::new());
        assert_eq!(enclosing_heads(&folds, 7), Vec::<usize>::new());
    }

    #[test]
    fn errors_only_for_broken_code() {
        assert!(parse(Language::Rust, RUST).errors().is_empty());
        assert!(parse(Language::Json, JSON).errors().is_empty());
        assert!(!parse(Language::Rust, "fn (").errors().is_empty());
        assert!(!parse(Language::Json, "{ \"a\": }").errors().is_empty());
        let problems = parse(Language::Json, "{\n  \"a\": 1,,\n  \"b\": [1, 2\n}\n").problems();
        assert!(!problems.is_empty());
        assert!(problems.iter().all(|p| !p.message.is_empty()));
        assert!(problems
            .windows(2)
            .all(|w| w[0].range.start <= w[1].range.start));
        let missing = parse(Language::Rust, "fn main() {\n    let x = 1\n}\n").problems();
        assert!(
            missing.iter().any(|p| p.message.starts_with("Missing")),
            "{missing:?}"
        );
    }

    #[test]
    fn reparse_tracks_the_text() {
        let mut tree = parse(Language::Rust, "fn a() {}\n");
        assert!(tree.errors().is_empty());
        tree.reparse("fn (");
        assert!(!tree.errors().is_empty());
        tree.reparse("fn a() {}\n");
        assert!(tree.errors().is_empty());
    }

    #[test]
    fn reparse_edited_reuses_the_tree() {
        let before = "fn a() {\n    x();\n}\n";
        let after = "fn a() {\n    xy();\n}\n";
        let mut tree = parse(Language::Rust, before);
        let edit = InputEdit {
            start_byte: 14,
            old_end_byte: 14,
            new_end_byte: 15,
            start_position: tree_sitter::Point::new(1, 5),
            old_end_position: tree_sitter::Point::new(1, 5),
            new_end_position: tree_sitter::Point::new(1, 6),
        };
        tree.reparse_edited(&edit, after);
        assert!(tree.errors().is_empty());
        assert_eq!(tree.expand(span(after, "xy")), Some(span(after, "xy()")));
    }
}
