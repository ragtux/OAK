//! Text edits as Oak applies them: a `TextEdit`'s positions become a char
//! range in the document, sorted and non-overlapping, so an open document
//! takes them through its cursor in one undo step and a closed file is
//! rewritten on disk.

use crate::position::{column_from_lsp, Encoding};
use lsp_types::{DocumentChangeOperation, DocumentChanges, OneOf, TextEdit, Url, WorkspaceEdit};
use std::ops::Range;

/// One replacement in char offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChange {
    pub range: Range<usize>,
    pub text: String,
}

/// The lines of a text as char offsets, for position lookups (built once,
/// then every token or diagnostic of a response resolves in O(1)).
pub struct LineIndex<'a> {
    lines: Vec<&'a str>,
    starts: Vec<usize>,
    len: usize,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut starts = Vec::with_capacity(lines.len());
        let mut at = 0;
        for line in &lines {
            starts.push(at);
            at += line.chars().count() + 1;
        }
        LineIndex {
            lines,
            starts,
            len: text.chars().count(),
        }
    }

    /// The char offset of a position (clamped to the text).
    pub fn offset(&self, position: lsp_types::Position, encoding: Encoding) -> usize {
        let row = position.line as usize;
        if row >= self.lines.len() {
            return self.len;
        }
        let line = self.lines[row].trim_end_matches('\r');
        self.starts[row] + column_from_lsp(line, position.character, encoding)
    }

    /// A range as char offsets.
    pub fn range(&self, range: lsp_types::Range, encoding: Encoding) -> Range<usize> {
        let start = self.offset(range.start, encoding);
        let end = self.offset(range.end, encoding).max(start);
        start..end
    }

    /// The row's text (without a trailing CR), for column conversions.
    pub fn line(&self, row: usize) -> Option<&'a str> {
        self.lines.get(row).map(|l| l.trim_end_matches('\r'))
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

/// The char offset of an LSP position in `text`.
pub fn offset_of(text: &str, position: lsp_types::Position, encoding: Encoding) -> usize {
    LineIndex::new(text).offset(position, encoding)
}

/// `edits` as char-range changes in `text`, sorted by start; edits that
/// overlap an earlier one are dropped (the protocol forbids them).
pub fn changes(text: &str, edits: &[TextEdit], encoding: Encoding) -> Vec<TextChange> {
    let lines = LineIndex::new(text);
    let mut out: Vec<TextChange> = edits
        .iter()
        .map(|e| {
            let start = lines.offset(e.range.start, encoding);
            let end = lines.offset(e.range.end, encoding).max(start);
            TextChange {
                range: start..end,
                text: e.new_text.clone(),
            }
        })
        .collect();
    out.sort_by_key(|c| (c.range.start, c.range.end));
    let mut kept: Vec<TextChange> = Vec::with_capacity(out.len());
    for c in out {
        if kept.last().is_some_and(|k| c.range.start < k.range.end) {
            continue;
        }
        kept.push(c);
    }
    kept
}

/// `text` with the (sorted, non-overlapping) `changes` applied.
pub fn apply(text: &str, changes: &[TextChange]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for c in changes {
        let start = c.range.start.min(chars.len());
        let end = c.range.end.min(chars.len()).max(start);
        if start < at {
            continue;
        }
        out.extend(chars[at..start].iter());
        out.push_str(&c.text);
        at = end;
    }
    out.extend(chars[at..].iter());
    out
}

/// A file's edits out of a workspace edit.
#[derive(Clone, Debug, PartialEq)]
pub struct FileEdits {
    pub uri: Url,
    pub edits: Vec<TextEdit>,
}

/// Every file a workspace edit touches with its text edits, in the order
/// the server gave them. Resource operations (create, rename, delete) are
/// left out: Oak applies text only.
pub fn workspace_edit_files(edit: &WorkspaceEdit) -> Vec<FileEdits> {
    let mut out: Vec<FileEdits> = Vec::new();
    let mut push = |uri: &Url, edits: Vec<TextEdit>| {
        if let Some(existing) = out.iter_mut().find(|f| &f.uri == uri) {
            existing.edits.extend(edits);
        } else {
            out.push(FileEdits {
                uri: uri.clone(),
                edits,
            });
        }
    };
    if let Some(changes) = &edit.document_changes {
        let edits: Vec<&lsp_types::TextDocumentEdit> = match changes {
            DocumentChanges::Edits(edits) => edits.iter().collect(),
            DocumentChanges::Operations(ops) => ops
                .iter()
                .filter_map(|op| match op {
                    DocumentChangeOperation::Edit(e) => Some(e),
                    DocumentChangeOperation::Op(_) => None,
                })
                .collect(),
        };
        for doc in edits {
            let text_edits = doc
                .edits
                .iter()
                .map(|e| match e {
                    OneOf::Left(e) => e.clone(),
                    OneOf::Right(a) => a.text_edit.clone(),
                })
                .collect();
            push(&doc.text_document.uri, text_edits);
        }
    } else if let Some(changes) = &edit.changes {
        let mut uris: Vec<&Url> = changes.keys().collect();
        uris.sort_by_key(|u| u.as_str().to_string());
        for uri in uris {
            push(uri, changes[uri].clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range as LspRange};

    fn edit(sl: u32, sc: u32, el: u32, ec: u32, text: &str) -> TextEdit {
        TextEdit {
            range: LspRange::new(Position::new(sl, sc), Position::new(el, ec)),
            new_text: text.into(),
        }
    }

    #[test]
    fn edits_become_char_ranges_and_apply_in_order() {
        let text = "fn main() {\n    let x = 1;\n}\n";
        let edits = vec![
            edit(1, 12, 1, 13, "2"),
            edit(0, 3, 0, 7, "start"),
            edit(2, 1, 2, 1, "\n"),
        ];
        let changes = changes(text, &edits, Encoding::Utf16);
        assert_eq!(changes[0].range, 3..7);
        assert_eq!(changes[1].range, 24..25);
        assert_eq!(changes[2].range, 28..28);
        assert_eq!(apply(text, &changes), "fn start() {\n    let x = 2;\n}\n\n");
    }

    #[test]
    fn positions_past_the_end_clamp() {
        let text = "ab\ncd";
        assert_eq!(offset_of(text, Position::new(9, 0), Encoding::Utf8), 5);
        assert_eq!(offset_of(text, Position::new(1, 9), Encoding::Utf8), 5);
        assert_eq!(offset_of(text, Position::new(0, 9), Encoding::Utf8), 2);
    }

    #[test]
    fn encodings_apply_to_columns() {
        let text = "é = 1";
        let e = edit(0, 4, 0, 5, "2");
        assert_eq!(
            changes(text, std::slice::from_ref(&e), Encoding::Utf8)[0].range,
            3..4
        );
        assert_eq!(changes(text, &[e], Encoding::Utf16)[0].range, 4..5);
    }

    #[test]
    fn overlapping_edits_are_dropped() {
        let text = "abcdef";
        let edits = vec![edit(0, 1, 0, 4, "X"), edit(0, 2, 0, 5, "Y")];
        let changes = changes(text, &edits, Encoding::Utf8);
        assert_eq!(changes.len(), 1);
        assert_eq!(apply(text, &changes), "aXef");
    }

    #[test]
    fn workspace_edit_flattens_both_shapes() {
        let uri = Url::parse("file:///tmp/a.rs").unwrap();
        let mut map = std::collections::HashMap::new();
        map.insert(uri.clone(), vec![edit(0, 0, 0, 1, "z")]);
        let by_changes = WorkspaceEdit {
            changes: Some(map),
            document_changes: None,
            change_annotations: None,
        };
        let files = workspace_edit_files(&by_changes);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].edits[0].new_text, "z");

        let by_docs = WorkspaceEdit {
            changes: None,
            document_changes: Some(DocumentChanges::Edits(vec![lsp_types::TextDocumentEdit {
                text_document: lsp_types::OptionalVersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: None,
                },
                edits: vec![OneOf::Left(edit(0, 0, 0, 1, "q"))],
            }])),
            change_annotations: None,
        };
        let files = workspace_edit_files(&by_docs);
        assert_eq!(files[0].uri, uri);
        assert_eq!(files[0].edits[0].new_text, "q");
    }
}
