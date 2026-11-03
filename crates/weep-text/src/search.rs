//! The find bar's search: every match of a query in a buffer, as char
//! ranges, with Fleet's toggles (match case, whole words, regex, in
//! selection), and the replacement text for a match.

use crate::buffer::Buffer;
use std::ops::Range;

/// The find bar's toggles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Search only inside this char range (the selection the bar opened with).
    pub in_range: Option<Range<usize>>,
}

/// A compiled query: a regex either way, so one matcher serves every mode.
#[derive(Debug)]
pub struct Query {
    re: regex::Regex,
    pub options: SearchOptions,
}

impl Query {
    /// Compiles `pattern`; an empty pattern or a bad regex is an error with
    /// the message the bar shows (empty for an empty pattern).
    pub fn new(pattern: &str, options: SearchOptions) -> Result<Query, String> {
        if pattern.is_empty() {
            return Err(String::new());
        }
        let mut source = if options.regex {
            pattern.to_string()
        } else {
            regex::escape(pattern)
        };
        if options.whole_word {
            // Not `\b`: a query starting or ending in punctuation must still
            // match when its neighbours are not word characters.
            source = format!("(?:^|[^\\w])({source})(?:[^\\w]|$)");
        }
        let re = regex::RegexBuilder::new(&source)
            .case_insensitive(!options.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| {
                let text = e.to_string();
                text.lines()
                    .last()
                    .unwrap_or("bad pattern")
                    .trim()
                    .to_string()
            })?;
        Ok(Query { re, options })
    }

    /// Every match in `text` (the buffer's LF text), as char ranges.
    pub fn find_all(&self, text: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut byte_to_char = ByteToChar::new(text);
        let mut start = 0;
        while start <= text.len() {
            let Some(caps) = self.re.captures_at(text, start) else {
                break;
            };
            let whole = caps.get(0).unwrap();
            let m = if self.options.whole_word {
                caps.get(1).unwrap()
            } else {
                whole
            };
            if whole.end() == whole.start() {
                // An empty match (regex `a*`): step over one char.
                if whole.start() >= text.len() {
                    break;
                }
                start = whole.end() + text[whole.end()..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
            out.push(byte_to_char.convert(m.start())..byte_to_char.convert(m.end()));
            // A whole-word match consumed its right neighbour; resume at the
            // word's end so an adjacent word still matches.
            start = if self.options.whole_word {
                m.end()
            } else {
                whole.end()
            };
        }
        out
    }

    /// `find_all` on the buffer, kept to `in_range` when set.
    pub fn find_in(&self, buffer: &Buffer) -> Vec<Range<usize>> {
        let text = buffer.text();
        let mut found = self.find_all(&text);
        if let Some(range) = &self.options.in_range {
            found.retain(|m| m.start >= range.start && m.end <= range.end);
        }
        found
    }

    /// The text that replaces the match `range` (char offsets): `replacement`
    /// itself, or with regex on, `replacement` with `$1`, `${name}` and `$0`
    /// expanded from the match's groups.
    pub fn replacement_for(&self, text: &str, range: Range<usize>, replacement: &str) -> String {
        if !self.options.regex {
            return replacement.to_string();
        }
        let start = char_to_byte(text, range.start);
        let end = char_to_byte(text, range.end);
        let Some(caps) = self.re.captures(&text[start..end]) else {
            return replacement.to_string();
        };
        let mut out = String::new();
        caps.expand(replacement, &mut out);
        out
    }
}

fn char_to_byte(text: &str, ch: usize) -> usize {
    text.char_indices()
        .nth(ch)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

/// Byte offset to char offset for ascending queries, in one pass.
struct ByteToChar<'a> {
    text: &'a str,
    byte: usize,
    ch: usize,
}

impl<'a> ByteToChar<'a> {
    fn new(text: &'a str) -> Self {
        ByteToChar {
            text,
            byte: 0,
            ch: 0,
        }
    }

    fn convert(&mut self, byte: usize) -> usize {
        if byte < self.byte {
            self.byte = 0;
            self.ch = 0;
        }
        self.ch += self.text[self.byte..byte].chars().count();
        self.byte = byte;
        self.ch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(text: &str, pattern: &str, options: SearchOptions) -> Vec<Range<usize>> {
        Query::new(pattern, options).unwrap().find_all(text)
    }

    #[test]
    fn plain_search_ignores_case_by_default() {
        let t = "Record record RECORD recorder";
        assert_eq!(
            find(t, "record", SearchOptions::default()),
            vec![0..6, 7..13, 14..20, 21..27]
        );
        assert_eq!(
            find(
                t,
                "record",
                SearchOptions {
                    case_sensitive: true,
                    ..Default::default()
                }
            ),
            vec![7..13, 21..27]
        );
    }

    #[test]
    fn whole_words_and_adjacent_words() {
        let t = "a aa a_a a.a";
        assert_eq!(
            find(
                t,
                "a",
                SearchOptions {
                    whole_word: true,
                    ..Default::default()
                }
            ),
            vec![0..1, 9..10, 11..12]
        );
    }

    #[test]
    fn regex_groups_and_replacement() {
        let q = Query::new(
            r"(\w+)@(\w+)",
            SearchOptions {
                regex: true,
                ..Default::default()
            },
        )
        .unwrap();
        let t = "mail me@host now";
        let m = q.find_all(t);
        assert_eq!(m, vec![5..12]);
        assert_eq!(q.replacement_for(t, 5..12, "$2 at $1"), "host at me");
        assert!(Query::new(
            "(",
            SearchOptions {
                regex: true,
                ..Default::default()
            }
        )
        .is_err());
        assert!(Query::new("", SearchOptions::default()).is_err());
    }

    #[test]
    fn char_offsets_not_bytes() {
        let t = "ééé x";
        assert_eq!(find(t, "x", SearchOptions::default()), vec![4..5]);
    }

    #[test]
    fn in_selection() {
        let b = Buffer::new("x x x x");
        let q = Query::new(
            "x",
            SearchOptions {
                in_range: Some(2..5),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(q.find_in(&b), vec![2..3, 4..5]);
    }
}
