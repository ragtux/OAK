//! What the editor paints besides the text: the identifier under the caret
//! and its other occurrences, the bracket matching the one at the caret, and
//! the indentation each line sits at. Plain text scans; tree-sitter refines
//! the brackets later.

use crate::buffer::Buffer;
use std::ops::Range;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The identifier the caret touches: the word around `offset`, or the word
/// ending right at it, as char offsets.
pub fn identifier_at(buffer: &Buffer, offset: usize) -> Option<Range<usize>> {
    let len = buffer.len_chars();
    let at = if offset < len && buffer.char_at(offset).is_some_and(is_word) {
        offset
    } else if offset > 0 && buffer.char_at(offset - 1).is_some_and(is_word) {
        offset - 1
    } else {
        return None;
    };
    let mut start = at;
    while start > 0 && buffer.char_at(start - 1).is_some_and(is_word) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < len && buffer.char_at(end).is_some_and(is_word) {
        end += 1;
    }
    Some(start..end)
}

/// Every whole-word occurrence of `word` in the text, as char ranges.
pub fn whole_word_occurrences(buffer: &Buffer, word: &str) -> Vec<Range<usize>> {
    let text = buffer.text();
    let word_chars = word.chars().count();
    let mut out = Vec::new();
    if word.is_empty() {
        return out;
    }
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = word.chars().collect();
    let mut i = 0;
    while i + word_chars <= chars.len() {
        if chars[i..i + word_chars] == needle[..] {
            let before_ok = i == 0 || !is_word(chars[i - 1]);
            let after_ok = i + word_chars == chars.len() || !is_word(chars[i + word_chars]);
            if before_ok && after_ok {
                out.push(i..i + word_chars);
                i += word_chars;
                continue;
            }
        }
        i += 1;
    }
    out
}

const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// The bracket at the caret (the character at `offset`, else the one before
/// it) and its partner, as char offsets `(opening, closing)`.
pub fn matching_bracket(buffer: &Buffer, offset: usize) -> Option<(usize, usize)> {
    let candidate = |at: usize| {
        let c = buffer.char_at(at)?;
        PAIRS
            .iter()
            .find(|(o, c2)| *o == c || *c2 == c)
            .map(|&(o, cl)| (at, o, cl, c == o))
    };
    let (at, open, close, is_open) =
        candidate(offset).or_else(|| offset.checked_sub(1).and_then(candidate))?;
    let len = buffer.len_chars();
    let mut depth = 0usize;
    if is_open {
        let mut i = at;
        while i < len {
            let c = buffer.char_at(i)?;
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    return Some((at, i));
                }
            }
            i += 1;
        }
    } else {
        let mut i = at + 1;
        while i > 0 {
            i -= 1;
            let c = buffer.char_at(i)?;
            if c == close {
                depth += 1;
            } else if c == open {
                depth -= 1;
                if depth == 0 {
                    return Some((i, at));
                }
            }
        }
    }
    None
}

/// The indentation of `line` in columns (a tab counts to the next stop of
/// `width`), or `None` for a blank line.
pub fn indent_of(line: &str, width: usize) -> Option<usize> {
    let width = width.max(1);
    let mut column = 0;
    for c in line.chars() {
        match c {
            ' ' => column += 1,
            '\t' => column += width - column % width,
            _ => return Some(column),
        }
    }
    None
}

/// The indentation a row shows guides for: its own, or for a blank row the
/// smaller of its nearest non-blank neighbours', so guides run through the
/// blank lines inside a block and stop between blocks.
pub fn effective_indent(buffer: &Buffer, row: usize) -> usize {
    let indent = |row: usize| indent_of(&buffer.line(row), buffer.indent.width);
    if let Some(i) = indent(row) {
        return i;
    }
    let above = (0..row).rev().take(500).find_map(indent).unwrap_or(0);
    let below = (row + 1..buffer.line_count())
        .take(500)
        .find_map(indent)
        .unwrap_or(0);
    above.min(below)
}

/// `effective_indent` for every row in `rows`.
pub fn guide_indents(buffer: &Buffer, rows: Range<usize>) -> Vec<usize> {
    rows.map(|row| effective_indent(buffer, row)).collect()
}

/// The block whose indent guide is the caret's: the innermost run of rows
/// indented past a tab stop that holds `caret_row` or starts right under it
/// (the row that opens a block owns it, as Fleet shows). The guide's column
/// and the rows of the run.
pub fn current_block(buffer: &Buffer, caret_row: usize) -> Option<(usize, Range<usize>)> {
    const REACH: usize = 5000;
    let line_count = buffer.line_count();
    if caret_row >= line_count {
        return None;
    }
    let own = effective_indent(buffer, caret_row);
    let below = if caret_row + 1 < line_count {
        effective_indent(buffer, caret_row + 1)
    } else {
        0
    };
    let (depth, anchor) = if below > own {
        (below, caret_row + 1)
    } else {
        (own, caret_row)
    };
    if depth == 0 {
        return None;
    }
    let width = buffer.indent.width.max(1);
    let col = (depth - 1) / width * width;
    let mut start = anchor;
    while start > 0 && anchor - start < REACH && effective_indent(buffer, start - 1) > col {
        start -= 1;
    }
    let mut end = anchor + 1;
    while end < line_count && end - anchor < REACH && effective_indent(buffer, end) > col {
        end += 1;
    }
    Some((col, start..end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_and_occurrences() {
        let b = Buffer::new("let record = Record::new(record_id); record");
        assert_eq!(identifier_at(&b, 4), Some(4..10));
        assert_eq!(
            identifier_at(&b, 10),
            Some(4..10),
            "the caret right after a word"
        );
        assert_eq!(identifier_at(&b, 11), None);
        let occ = whole_word_occurrences(&b, "record");
        assert_eq!(
            occ,
            vec![4..10, 37..43],
            "whole words only: not Record, not record_id"
        );
    }

    #[test]
    fn brackets_match_by_depth() {
        let b = Buffer::new("f(a, (b), c) {}");
        assert_eq!(matching_bracket(&b, 1), Some((1, 11)));
        assert_eq!(
            matching_bracket(&b, 12),
            Some((1, 11)),
            "the caret after the closer"
        );
        assert_eq!(matching_bracket(&b, 5), Some((5, 7)));
        assert_eq!(matching_bracket(&b, 14), Some((13, 14)));
        assert_eq!(matching_bracket(&b, 3), None);
    }

    #[test]
    fn guides_cross_blank_lines_inside_a_block() {
        let b = Buffer::new("fn a() {\n    x;\n\n    y;\n}\n\nfn b() {}");
        assert_eq!(guide_indents(&b, 0..7), vec![0, 4, 4, 4, 0, 0, 0]);
        assert_eq!(indent_of("\tx", 4), Some(4));
        assert_eq!(indent_of("\tx", 2), Some(2));
        assert_eq!(indent_of(" \t x", 4), Some(5));
        assert_eq!(indent_of("   ", 4), None);
    }

    #[test]
    fn guides_and_blocks_follow_the_buffer_indent_width() {
        let mut b = Buffer::new("fn a() {\n  if x {\n    y;\n  }\n}\n");
        b.set_indent(crate::Indent::spaces(2));
        assert_eq!(guide_indents(&b, 0..5), vec![0, 2, 4, 2, 0]);
        assert_eq!(current_block(&b, 2), Some((2, 2..3)));
        assert_eq!(current_block(&b, 1), Some((2, 2..3)));
        assert_eq!(current_block(&b, 3), Some((0, 1..4)));
        let mut t = Buffer::new("a {\n\tb;\n}\n");
        t.set_indent(crate::Indent::tabs(8));
        assert_eq!(guide_indents(&t, 0..3), vec![0, 8, 0]);
        assert_eq!(current_block(&t, 1), Some((0, 1..2)));
    }

    #[test]
    fn the_caret_block_is_the_one_its_row_is_in_or_opens() {
        let b = Buffer::new("fn a() {\n    if x {\n        y;\n    }\n    z;\n}\n\nfn b() {}");
        assert_eq!(
            current_block(&b, 0),
            Some((0, 1..5)),
            "the head row owns its block"
        );
        assert_eq!(current_block(&b, 4), Some((0, 1..5)));
        assert_eq!(
            current_block(&b, 1),
            Some((4, 2..3)),
            "an inner head owns the inner block"
        );
        assert_eq!(current_block(&b, 2), Some((4, 2..3)));
        assert_eq!(
            current_block(&b, 3),
            Some((0, 1..5)),
            "the closing row is back in the outer"
        );
        assert_eq!(current_block(&b, 6), None);
        assert_eq!(current_block(&b, 7), None);
    }
}
