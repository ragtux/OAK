//! LSP positions against Oak's text. Oak counts columns in chars; the
//! protocol counts them in UTF-16 units unless the server negotiated UTF-8
//! or UTF-32 (`general.positionEncodings`, LSP 3.17). Every conversion
//! needs the line's text and clamps to its end, as the protocol asks.

use lsp_types::PositionEncodingKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    #[default]
    Utf16,
    Utf32,
}

impl Encoding {
    /// What the server answered in its capabilities (UTF-16 when it said nothing).
    pub fn from_kind(kind: Option<&PositionEncodingKind>) -> Encoding {
        match kind.map(|k| k.as_str()) {
            Some("utf-8") => Encoding::Utf8,
            Some("utf-32") => Encoding::Utf32,
            _ => Encoding::Utf16,
        }
    }

    /// The encodings Oak offers, best first: chars are UTF-32, UTF-8 costs
    /// one pass over the line, UTF-16 the same (and is the fallback).
    pub fn offered() -> Vec<PositionEncodingKind> {
        vec![
            PositionEncodingKind::UTF32,
            PositionEncodingKind::UTF8,
            PositionEncodingKind::UTF16,
        ]
    }

    fn units(self, c: char) -> usize {
        match self {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
            Encoding::Utf32 => 1,
        }
    }
}

/// The LSP column of the char column `column` in `line` (clamped to the
/// line's length). `line` is the row's text without its newline.
pub fn column_to_lsp(line: &str, column: usize, encoding: Encoding) -> u32 {
    line.chars()
        .take(column)
        .map(|c| encoding.units(c))
        .sum::<usize>() as u32
}

/// The char column of the LSP column `character` in `line` (clamped to
/// the line's length; a unit inside a char rounds down to the char).
pub fn column_from_lsp(line: &str, character: u32, encoding: Encoding) -> usize {
    let target = character as usize;
    let mut units = 0;
    for (ix, c) in line.chars().enumerate() {
        if units >= target {
            return ix;
        }
        units += encoding.units(c);
        if units > target {
            return ix;
        }
    }
    line.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_the_same_in_every_encoding() {
        for e in [Encoding::Utf8, Encoding::Utf16, Encoding::Utf32] {
            assert_eq!(column_to_lsp("let x = 1;", 4, e), 4);
            assert_eq!(column_from_lsp("let x = 1;", 4, e), 4);
            assert_eq!(column_from_lsp("let x = 1;", 99, e), 10);
        }
    }

    #[test]
    fn wide_chars_count_by_encoding() {
        // "é" is 2 bytes, 1 UTF-16 unit; "😀" is 4 bytes, 2 units.
        let line = "é😀x";
        assert_eq!(column_to_lsp(line, 2, Encoding::Utf8), 6);
        assert_eq!(column_to_lsp(line, 2, Encoding::Utf16), 3);
        assert_eq!(column_to_lsp(line, 2, Encoding::Utf32), 2);
        assert_eq!(column_from_lsp(line, 6, Encoding::Utf8), 2);
        assert_eq!(column_from_lsp(line, 3, Encoding::Utf16), 2);
        assert_eq!(column_from_lsp(line, 2, Encoding::Utf32), 2);
        // A unit inside the emoji rounds down to it.
        assert_eq!(column_from_lsp(line, 2, Encoding::Utf16), 1);
        assert_eq!(column_from_lsp(line, 3, Encoding::Utf8), 1);
    }

    #[test]
    fn encoding_from_the_server() {
        assert_eq!(Encoding::from_kind(None), Encoding::Utf16);
        assert_eq!(
            Encoding::from_kind(Some(&PositionEncodingKind::UTF8)),
            Encoding::Utf8
        );
        assert_eq!(
            Encoding::from_kind(Some(&PositionEncodingKind::UTF32)),
            Encoding::Utf32
        );
    }
}
