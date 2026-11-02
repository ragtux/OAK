//! A rope-backed text buffer with a linear undo history.

use ropey::Rope;
use std::collections::VecDeque;
use std::ops::Range;

/// A row and a column, both zero-based; the column counts chars, not bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub row: usize,
    pub column: usize,
}

impl Point {
    pub fn new(row: usize, column: usize) -> Self {
        Point { row, column }
    }
}

/// One replacement: `removed` was taken out at `offset` and `inserted` put in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub offset: usize,
    pub removed: String,
    pub inserted: String,
}

/// An undo step: the changes applied in order, with the cursor/selection to
/// restore on either side. Stored as char offsets of the anchor and head.
#[derive(Clone, Debug, Default)]
pub(crate) struct Transaction {
    pub changes: Vec<Change>,
    pub before: (usize, usize),
    pub after: (usize, usize),
}

/// One applied change as positions only, for a view that holds offsets into
/// the buffer (its selection) and must move them after someone else's edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    /// The buffer's version once this edit was applied.
    pub version: u64,
    pub offset: usize,
    pub old_len: usize,
    pub new_len: usize,
}

impl Edit {
    /// Where `pos` lands after this edit: shifted when it lay after the
    /// replaced range, pulled to the replacement's end when it lay inside.
    pub fn adjust(&self, pos: usize) -> usize {
        if pos <= self.offset {
            pos
        } else if pos >= self.offset + self.old_len {
            pos - self.old_len + self.new_len
        } else {
            self.offset + self.new_len
        }
    }
}

/// How many recent edits a buffer remembers for views that catch up late.
const EDIT_LOG: usize = 512;

/// The text, its undo and redo stacks, and whether it differs from what was
/// last saved. One buffer may be shown by several views (the splits), each
/// with its own cursor, so nothing here belongs to a view.
#[derive(Clone, Debug)]
pub struct Buffer {
    rope: Rope,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Bumped on every change; `saved_version` is where the file on disk is.
    version: u64,
    saved_version: u64,
    /// The last `EDIT_LOG` changes, oldest first, for `edits_since`.
    edits: VecDeque<Edit>,
    /// The line ending the file came with, written back on save.
    crlf: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::new("")
    }
}

impl Buffer {
    /// A buffer holding `text`. CRLF endings are normalised to LF in memory
    /// and remembered, so a Windows file is written back as it came.
    pub fn new(text: &str) -> Self {
        let crlf = text.contains("\r\n");
        let normalised;
        let text = if crlf {
            normalised = text.replace("\r\n", "\n");
            normalised.as_str()
        } else {
            text
        };
        Buffer {
            rope: Rope::from_str(text),
            undo: Vec::new(),
            redo: Vec::new(),
            version: 0,
            saved_version: 0,
            edits: VecDeque::new(),
            crlf,
        }
    }

    /// Replaces the whole text (a reload from disk), as one undo step.
    pub fn set_text(&mut self, text: &str) {
        let len = self.rope.len_chars();
        let mut tx = Transaction {
            before: (0, 0),
            after: (0, 0),
            ..Default::default()
        };
        self.replace_in(&mut tx, 0..len, text);
        self.push_transaction(tx);
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// The whole text with the file's own line endings, for writing out.
    pub fn text_for_file(&self) -> String {
        let text = self.rope.to_string();
        if self.crlf {
            text.replace('\n', "\r\n")
        } else {
            text
        }
    }

    /// The whole text with LF endings.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn uses_crlf(&self) -> bool {
        self.crlf
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// The number of rows, counting the empty row after a trailing newline.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Row `row`'s text without its line break.
    pub fn line(&self, row: usize) -> String {
        let line = self.rope.line(row);
        let len = self.line_len(row);
        line.slice(..len).to_string()
    }

    /// The chars in row `row` before its line break.
    pub fn line_len(&self, row: usize) -> usize {
        let line = self.rope.line(row);
        let mut len = line.len_chars();
        if len > 0 && line.char(len - 1) == '\n' {
            len -= 1;
        }
        len
    }

    pub fn line_to_char(&self, row: usize) -> usize {
        self.rope.line_to_char(row)
    }

    pub fn line_to_byte(&self, row: usize) -> usize {
        self.rope.line_to_byte(row)
    }

    pub fn char_to_byte(&self, offset: usize) -> usize {
        self.rope.char_to_byte(offset)
    }

    pub fn byte_to_char(&self, byte: usize) -> usize {
        self.rope.byte_to_char(byte)
    }

    pub fn char_at(&self, offset: usize) -> Option<char> {
        (offset < self.rope.len_chars()).then(|| self.rope.char(offset))
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        self.rope.slice(range).to_string()
    }

    pub fn offset_to_point(&self, offset: usize) -> Point {
        let offset = offset.min(self.rope.len_chars());
        let row = self.rope.char_to_line(offset);
        Point::new(row, offset - self.rope.line_to_char(row))
    }

    /// The offset of `point`, with the row clamped to the text and the
    /// column clamped to the row's length.
    pub fn point_to_offset(&self, point: Point) -> usize {
        let row = point.row.min(self.line_count().saturating_sub(1));
        let column = point.column.min(self.line_len(row));
        self.rope.line_to_char(row) + column
    }

    pub fn is_dirty(&self) -> bool {
        self.version != self.saved_version
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// The edits applied after `version`, oldest first, or `None` when
    /// `version` is older than the log remembers (the caller then clamps).
    pub fn edits_since(&self, version: u64) -> Option<Vec<Edit>> {
        if version >= self.version {
            return Some(Vec::new());
        }
        let first = self.edits.front()?;
        if first.version > version + 1 {
            return None;
        }
        Some(
            self.edits
                .iter()
                .filter(|e| e.version > version)
                .copied()
                .collect(),
        )
    }

    /// Applies one change to the rope, without touching history.
    fn apply(&mut self, change: &Change) {
        let old_len = change.removed.chars().count();
        if !change.removed.is_empty() {
            self.rope.remove(change.offset..change.offset + old_len);
        }
        if !change.inserted.is_empty() {
            self.rope.insert(change.offset, &change.inserted);
        }
        self.version += 1;
        if self.edits.len() == EDIT_LOG {
            self.edits.pop_front();
        }
        self.edits.push_back(Edit {
            version: self.version,
            offset: change.offset,
            old_len,
            new_len: change.inserted.chars().count(),
        });
    }

    /// Replaces `range` with `text`, recording the change into `tx`.
    pub(crate) fn replace_in(&mut self, tx: &mut Transaction, range: Range<usize>, text: &str) {
        let removed = self.rope.slice(range.clone()).to_string();
        let change = Change {
            offset: range.start,
            removed,
            inserted: text.to_string(),
        };
        self.apply(&change);
        tx.changes.push(change);
    }

    pub(crate) fn push_transaction(&mut self, tx: Transaction) {
        if tx.changes.is_empty() {
            return;
        }
        self.undo.push(tx);
        self.redo.clear();
    }

    /// The last transaction, for grouping consecutive typing into it.
    pub(crate) fn last_transaction_mut(&mut self) -> Option<&mut Transaction> {
        self.undo.last_mut()
    }

    /// Reverts the last transaction; returns the selection to restore.
    pub(crate) fn undo(&mut self) -> Option<(usize, usize)> {
        let tx = self.undo.pop()?;
        for change in tx.changes.iter().rev() {
            let inverse = Change {
                offset: change.offset,
                removed: change.inserted.clone(),
                inserted: change.removed.clone(),
            };
            self.apply(&inverse);
        }
        let before = tx.before;
        self.redo.push(tx);
        Some(before)
    }

    pub(crate) fn redo(&mut self) -> Option<(usize, usize)> {
        let tx = self.redo.pop()?;
        for change in &tx.changes {
            self.apply(change);
        }
        let after = tx.after;
        self.undo.push(tx);
        Some(after)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_points() {
        let b = Buffer::new("ab\ncde\n");
        assert_eq!(b.line_count(), 3);
        assert_eq!(b.line(0), "ab");
        assert_eq!(b.line(1), "cde");
        assert_eq!(b.line(2), "");
        assert_eq!(b.line_len(1), 3);
        assert_eq!(b.offset_to_point(4), Point::new(1, 1));
        assert_eq!(b.point_to_offset(Point::new(1, 99)), 6);
        assert_eq!(b.point_to_offset(Point::new(99, 0)), 7);
    }

    #[test]
    fn edits_since_reports_each_change_once() {
        let mut b = Buffer::new("abc");
        let v0 = b.version();
        let mut tx = Transaction::default();
        b.replace_in(&mut tx, 1..2, "XY");
        b.replace_in(&mut tx, 0..0, "-");
        b.push_transaction(tx);
        let edits = b.edits_since(v0).unwrap();
        assert_eq!(edits.len(), 2);
        assert_eq!(
            (edits[0].offset, edits[0].old_len, edits[0].new_len),
            (1, 1, 2)
        );
        assert_eq!(
            (edits[1].offset, edits[1].old_len, edits[1].new_len),
            (0, 0, 1)
        );
        assert!(b.edits_since(b.version()).unwrap().is_empty());
        // A position after the replaced range shifts, one inside lands after it.
        assert_eq!(edits[0].adjust(3), 4);
        assert_eq!(edits[0].adjust(2), 3);
        assert_eq!(edits[0].adjust(1), 1);
        // Undo logs the inverse, so a view catching up sees both.
        b.undo();
        assert_eq!(b.edits_since(v0).unwrap().len(), 4);
        assert_eq!(b.text(), "abc");
    }

    #[test]
    fn crlf_round_trips() {
        let b = Buffer::new("a\r\nb\r\n");
        assert!(b.uses_crlf());
        assert_eq!(b.text(), "a\nb\n");
        assert_eq!(b.text_for_file(), "a\r\nb\r\n");
    }
}
