//! An LSP snippet (`InsertTextFormat::SNIPPET`) as the plain text WEEP
//! inserts: tabstops and variables removed, placeholders replaced by their
//! default text, choices by their first choice, escapes resolved. WEEP has
//! no snippet mode yet, so the caret lands on the first tabstop (`$1`, with
//! its placeholder selected), else on `$0`, else at the end.

use std::ops::Range;

/// The plain text and the char range the caret or selection takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plain {
    pub text: String,
    pub select: Option<Range<usize>>,
}

pub fn plain(snippet: &str) -> Plain {
    let mut out = String::new();
    let mut stops: Vec<(u32, Range<usize>)> = Vec::new();
    let chars: Vec<char> = snippet.chars().collect();
    let mut i = 0;
    let mut out_len = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            out_len += 1;
            i += 2;
            continue;
        }
        if c == '$' && i + 1 < chars.len() {
            if chars[i + 1].is_ascii_digit() {
                // `$1`
                let mut j = i + 1;
                let mut n = 0u32;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    n = n * 10 + chars[j].to_digit(10).unwrap_or(0);
                    j += 1;
                }
                stops.push((n, out_len..out_len));
                i = j;
                continue;
            }
            if chars[i + 1] == '{' {
                if let Some(close) = matching_brace(&chars, i + 1) {
                    let inner: String = chars[i + 2..close].iter().collect();
                    let (name, rest) = match inner.find([':', '|']) {
                        Some(ix) => (&inner[..ix], &inner[ix..]),
                        None => (inner.as_str(), ""),
                    };
                    let stop = name.parse::<u32>().ok();
                    let default = if let Some(choices) = rest.strip_prefix('|') {
                        choices
                            .trim_end_matches('|')
                            .split(',')
                            .next()
                            .unwrap_or("")
                            .to_string()
                    } else if let Some(text) = rest.strip_prefix(':') {
                        // A placeholder may nest another snippet.
                        plain(text).text
                    } else if stop.is_none() {
                        // A variable without a default inserts nothing.
                        String::new()
                    } else {
                        String::new()
                    };
                    let start = out_len;
                    let len = default.chars().count();
                    out.push_str(&default);
                    out_len += len;
                    if let Some(n) = stop {
                        stops.push((n, start..start + len));
                    }
                    i = close + 1;
                    continue;
                }
            }
            if chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_' {
                // `$TM_FILENAME`: a variable WEEP cannot fill.
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        out.push(c);
        out_len += 1;
        i += 1;
    }
    let select = stops
        .iter()
        .filter(|(n, _)| *n > 0)
        .min_by_key(|(n, _)| *n)
        .or_else(|| stops.iter().find(|(n, _)| *n == 0))
        .map(|(_, r)| r.clone());
    Plain { text: out, select }
}

fn matching_brace(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabstops_and_placeholders() {
        let p = plain("println!(\"$1\")$0");
        assert_eq!(p.text, "println!(\"\")");
        assert_eq!(p.select, Some(10..10));
        let p = plain("fn ${1:name}($2) {\n    $0\n}");
        assert_eq!(p.text, "fn name() {\n    \n}");
        assert_eq!(p.select, Some(3..7));
    }

    #[test]
    fn final_stop_when_no_other() {
        let p = plain("foo($0)");
        assert_eq!(p.text, "foo()");
        assert_eq!(p.select, Some(4..4));
        assert_eq!(plain("plain").select, None);
    }

    #[test]
    fn choices_variables_and_escapes() {
        assert_eq!(plain("${1|a,b,c|}").text, "a");
        assert_eq!(plain("$TM_FILENAME.rs").text, ".rs");
        assert_eq!(plain("${TM_FILENAME:x}").text, "x");
        assert_eq!(plain("\\$1 \\\\ \\}").text, "$1 \\ }");
        assert_eq!(plain("${1:${2:inner}}").text, "inner");
    }
}
