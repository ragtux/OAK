//! Evaluating Rust through LLDB, made as useful as LLDB allows.
//!
//! What lldb-dap does with an `evaluate` (Xcode's lldb-dap, LLDB 2103,
//! batch 13, on the fixture's `misc/debuggee.rs` stopped at line 6):
//!
//! - `hover` reads a variable path only (`frame variable`'s "DIL legacy
//!   mode"): `total`, `points[0].x`, `&points[0]`, `*first` work; an
//!   operator, a call or a tuple field fails (`total * 2`: "expected 'eof',
//!   got: <'*' (star)>"), so does `.` through a reference (`first.x`: "is a
//!   pointer; did you mean to use '->'?").
//! - `watch` and `repl` try the same path first, then LLDB's expression
//!   evaluator, which has no Rust: it compiles the text as Objective C++.
//!   Arithmetic works (`total * 2` = 28, the value alone, no warning), `.`
//!   through a reference works, but a failure is reported as "Expression
//!   evaluation in Rust not supported. Falling back to default language. Ran
//!   expression as 'Objective C++'.\nerror: <user expression 0>:1:1: use of
//!   undeclared identifier 'total'" plus the source excerpt: the warning is
//!   a prefix of the error, never of a value. A method call never works
//!   (`points.len()`: "called object type 'unsigned long' is not a function
//!   or function pointer"), and a tuple field is misread as a float:
//!   `pair.0` answers "0" of type `double`, wrong but successful.
//! - `repl` prints as LLDB's `expression` does ("(int) $1 = 28").
//!
//! So [`crate::Client::evaluate`] goes through the variables tree for a
//! path with a tuple field, retries a failed path there (a field of an
//! `Option`'s payload, a field through a reference in `hover`), retries a
//! failed `hover` of anything else in `watch` (a selection's arithmetic),
//! and reports a failure as LLDB's one error line ([`clean_error`]).

use crate::EvalContext;

/// What an `evaluate` the owner asked for still needs to know when its
/// answer comes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EvalRequest {
    pub expression: String,
    pub frame_id: Option<i64>,
    pub context: EvalContext,
    /// A failed `hover` was already retried in `watch`.
    pub retried: bool,
}

/// An evaluation through the variables tree: the frame's scopes, then the
/// variable named by the path's first segment, then its children one
/// segment at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Walk {
    pub expression: String,
    pub context: EvalContext,
    /// The segments still to find; the first is looked up in the request
    /// this walk waits on.
    pub path: Vec<String>,
    /// Scopes not yet searched for the first segment (their variables'
    /// references), while the walk is at the root.
    pub scopes: Vec<i64>,
    /// At the root: the first segment is a variable of a scope.
    pub at_root: bool,
    /// The type of the variable whose children are being read.
    pub parent_type: Option<String>,
    /// The error to report when the walk finds nothing, when the adapter
    /// gave a better one than the walk's own.
    pub error: Option<String>,
}

impl Walk {
    pub fn new(expression: &str, context: EvalContext, path: Vec<String>) -> Walk {
        Walk {
            expression: expression.trim().to_string(),
            context,
            path,
            scopes: Vec::new(),
            at_root: true,
            parent_type: None,
            error: None,
        }
    }

    /// The error for a segment not found: the adapter's, when it gave one,
    /// else LLDB's own wording for the case.
    pub fn not_found(&self, segment: &str) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        match (&self.parent_type, self.at_root) {
            (_, true) => format!("use of undeclared identifier '{segment}'"),
            (Some(t), false) => format!("no member named '{segment}' in '{t}'"),
            (None, false) => format!("no member named '{segment}'"),
        }
    }

    /// The answer's text for the variable found: the value, or in the
    /// console's context `(type) path = value` as `frame variable` prints.
    pub fn result_text(&self, value: &str, type_name: Option<&str>) -> String {
        match (self.context, type_name) {
            (EvalContext::Repl, Some(t)) => format!("({t}) {} = {value}", self.expression),
            (EvalContext::Repl, None) => format!("{} = {value}", self.expression),
            _ => value.to_string(),
        }
    }
}

/// Whether a child's name is the segment: the name itself, or for a tuple
/// field `0` the older formatters' `__0`.
pub(crate) fn child_matches(name: &str, segment: &str) -> bool {
    name == segment
        || (segment.bytes().all(|b| b.is_ascii_digit()) && name.strip_prefix("__") == Some(segment))
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A Rust place expression made of a variable and fields, tuple fields and
/// constant indices (`points[0].x`, `pair.0`, `self.items[2]`, `p->x`), as
/// the names the variables tree shows on the way down: `["points", "[0]",
/// "x"]`. `None` for anything else (an operator, a call, a deref, a path
/// with `::`).
pub fn rust_path(expression: &str) -> Option<Vec<String>> {
    let s = expression.trim();
    let mut chars = s.char_indices().peekable();
    let mut out = Vec::new();
    let ident = |chars: &mut std::iter::Peekable<std::str::CharIndices>| -> Option<String> {
        let mut name = String::new();
        match chars.peek() {
            Some(&(_, c)) if is_ident_start(c) => {}
            _ => return None,
        }
        while let Some(&(_, c)) = chars.peek() {
            if !is_ident(c) {
                break;
            }
            name.push(c);
            chars.next();
        }
        Some(name)
    };
    out.push(ident(&mut chars)?);
    while let Some((_, c)) = chars.next() {
        match c {
            '.' | '-' => {
                if c == '-' && chars.next().map(|(_, c)| c) != Some('>') {
                    return None;
                }
                match chars.peek() {
                    Some(&(_, d)) if d.is_ascii_digit() => {
                        let mut n = String::new();
                        while let Some(&(_, d)) = chars.peek() {
                            if !d.is_ascii_digit() {
                                break;
                            }
                            n.push(d);
                            chars.next();
                        }
                        out.push(n);
                    }
                    _ => out.push(ident(&mut chars)?),
                }
            }
            '[' => {
                let mut n = String::new();
                for (_, d) in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    n.push(d);
                }
                let n = n.trim();
                if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                out.push(format!("[{}]", n.parse::<u64>().ok()?));
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Whether LLDB's expression evaluator misreads the path: a tuple field
/// (`pair.0`) parses as a float in Objective C++ and answers a wrong value.
pub(crate) fn has_tuple_field(path: &[String]) -> bool {
    path.iter()
        .skip(1)
        .any(|s| s.bytes().all(|b| b.is_ascii_digit()))
}

/// The prefix LLDB puts on every failure of the expression evaluator when
/// the frame's language is Rust (or another language it cannot compile).
fn is_fallback_warning(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("Expression evaluation in ") && line.contains("Falling back to")
}

/// An evaluation's error as one line: LLDB's "Falling back to default
/// language" warning dropped, then the first line left with its
/// `error: <user expression 0>:1:7: ` prefix dropped (the source excerpt
/// after it goes too). "use of undeclared identifier 'total'" for
/// lldb-dap's six lines on `total` in a frame without it.
pub fn clean_error(message: &str) -> String {
    let line = message
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !is_fallback_warning(l));
    let Some(mut line) = line else {
        return message.trim().to_string();
    };
    for prefix in ["error:", "warning:"] {
        if let Some(rest) = line.strip_prefix(prefix) {
            line = rest.trim_start();
        }
    }
    // `<user expression 12>:1:7: ` (the position in a text of one line).
    if line.starts_with("<user expression") {
        if let Some(close) = line.find('>') {
            let rest = &line[close + 1..];
            let mut parts = rest.splitn(4, ':');
            let (empty, row, col) = (parts.next(), parts.next(), parts.next());
            let numeric =
                |p: Option<&str>| p.is_some_and(|p| p.bytes().all(|b| b.is_ascii_digit()));
            if empty == Some("") && numeric(row) && numeric(col) {
                if let Some(rest) = parts.next() {
                    line = rest.trim_start();
                }
            }
        }
    }
    line.to_string()
}

/// A value's text with LLDB's fallback warning dropped, should it ever
/// come before a value (batch 13 saw it only before errors).
pub(crate) fn clean_result(result: String) -> String {
    match result.split_once('\n') {
        Some((first, rest)) if is_fallback_warning(first) => rest.to_string(),
        _ => result,
    }
}

/// Whether the adapter's `evaluateName` for a variable is an expression
/// that names it. lldb-dap gives the synthetic children of Rust's
/// formatters names that name nothing (`[0]->[0]` for a `Vec`'s element,
/// `*([0])` for a `String`'s byte, `0` for a tuple's field); without one
/// the owner builds the path from the parent's and the child's name.
pub(crate) fn usable_evaluate_name(name: &str) -> bool {
    let name = name.trim();
    name.chars().next().is_some_and(is_ident_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> Option<Vec<String>> {
        rust_path(s)
    }

    fn segments(s: &[&str]) -> Option<Vec<String>> {
        Some(s.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn rust_paths() {
        assert_eq!(path("total"), segments(&["total"]));
        assert_eq!(path(" points[0].x "), segments(&["points", "[0]", "x"]));
        assert_eq!(path("pair.0"), segments(&["pair", "0"]));
        assert_eq!(path("a.b.1[ 2 ]"), segments(&["a", "b", "1", "[2]"]));
        assert_eq!(path("first->x"), segments(&["first", "x"]));
        assert_eq!(path("self.items"), segments(&["self", "items"]));
        assert_eq!(path("total * 2"), None);
        assert_eq!(path("points.len()"), None);
        assert_eq!(path("&points[0]"), None);
        assert_eq!(path("*first"), None);
        assert_eq!(path("std::f64::MAX"), None);
        assert_eq!(path("points[i]"), None);
        assert_eq!(path("0"), None);
        assert_eq!(path("a."), None);
        assert_eq!(path("a-b"), None);
        assert!(has_tuple_field(&rust_path("pair.0").unwrap()));
        assert!(!has_tuple_field(&rust_path("points[0].x").unwrap()));
    }

    #[test]
    fn errors_are_one_line() {
        // lldb-dap's answers, batch 13.
        let watch = "Expression evaluation in Rust not supported. Falling back to default language. Ran expression as 'Objective C++'.\nerror: <user expression 0>:1:1: use of undeclared identifier 'total'\n    1 | total * 2\n      | ^~~~~\n";
        assert_eq!(clean_error(watch), "use of undeclared identifier 'total'");
        let hover = "<user expression 0>:1:7: expected 'eof', got: <'*' (star)>\n   1 | total * 2\n     | ^";
        assert_eq!(clean_error(hover), "expected 'eof', got: <'*' (star)>");
        let call = "Expression evaluation in Rust not supported. Falling back to default language. Ran expression as 'Objective C++'.\nerror: <user expression 2>:1:11: called object type 'unsigned long' is not a function or function pointer\n    1 | points.len()\n      | ~~~~~~~~~~^\n";
        assert_eq!(
            clean_error(call),
            "called object type 'unsigned long' is not a function or function pointer"
        );
        assert_eq!(clean_error("evaluate failed"), "evaluate failed");
        assert_eq!(clean_error("  "), "");
        assert_eq!(
            clean_result("Expression evaluation in Rust not supported. Falling back to default language.\n28".into()),
            "28"
        );
        assert_eq!(clean_result("(int) $1 = 28".into()), "(int) $1 = 28");
    }

    #[test]
    fn walk_errors_and_results() {
        let mut w = Walk::new("maybe.x", EvalContext::Repl, vec!["x".into()]);
        assert_eq!(w.not_found("maybe"), "use of undeclared identifier 'maybe'");
        w.at_root = false;
        w.parent_type = Some("core::option::Option<Point>".into());
        assert_eq!(
            w.not_found("x"),
            "no member named 'x' in 'core::option::Option<Point>'"
        );
        assert_eq!(w.result_text("1", Some("int")), "(int) maybe.x = 1");
        w.context = EvalContext::Watch;
        assert_eq!(w.result_text("1", Some("int")), "1");
        assert!(child_matches("0", "0"));
        assert!(child_matches("__0", "0"));
        assert!(!child_matches("__x", "x"));
    }

    #[test]
    fn evaluate_names() {
        assert!(usable_evaluate_name("points"));
        assert!(usable_evaluate_name("first->x"));
        assert!(!usable_evaluate_name("[0]->[0]"));
        assert!(!usable_evaluate_name("*([0])"));
        assert!(!usable_evaluate_name("0"));
    }
}
