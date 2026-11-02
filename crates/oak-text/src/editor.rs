//! The cursor, the selection and the edit commands over a [`Buffer`].

use crate::buffer::{Buffer, Point, Transaction};
use crate::TAB_WIDTH;
use std::ops::Range;

/// A selection as char offsets: `anchor` is where it started, `head` is
/// where the caret is. Equal when nothing is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Self {
        Selection {
            anchor: at,
            head: at,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn pair(&self) -> (usize, usize) {
        (self.anchor, self.head)
    }

    fn from_pair((anchor, head): (usize, usize)) -> Self {
        Selection { anchor, head }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharClass {
    Space,
    Word,
    Punct,
}

fn class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

/// Whether the last transaction can absorb another typed `text` at `offset`:
/// plain typing (no newline) that continues exactly where the last insert
/// ended, so a word typed in one go undoes in one go.
fn continues_typing(tx: &Transaction, offset: usize, text: &str) -> bool {
    if text.contains('\n') {
        return false;
    }
    let Some(last) = tx.changes.last() else {
        return false;
    };
    last.removed.is_empty()
        && !last.inserted.contains('\n')
        && last.offset + last.inserted.chars().count() == offset
}

/// The per-view half of an editor: where the caret and selection are, and
/// the typing group for undo. Every command takes the shared [`Buffer`].

#[derive(Clone, Debug, Default)]
pub struct Cursor {
    pub selection: Selection,
    /// The column the caret wants when moving vertically through shorter lines.
    goal_column: Option<usize>,
    /// Whether the next typed text may join the last undo step.
    grouping: bool,
    /// The buffer version this cursor's offsets are valid for (`sync`).
    seen_version: u64,
}

impl Cursor {
    pub fn new(buffer: &Buffer) -> Self {
        Cursor {
            seen_version: buffer.version(),
            ..Default::default()
        }
    }

    pub fn cursor(&self) -> usize {
        self.selection.head
    }

    pub fn cursor_point(&self, buffer: &Buffer) -> Point {
        buffer.offset_to_point(self.selection.head)
    }

    pub fn selected_text(&self, buffer: &Buffer) -> String {
        buffer.slice(self.selection.range())
    }

    /// Moves the selection past edits other cursors made to the buffer since
    /// this one last looked, so it stays on the same text. Call before
    /// reading or acting on the selection.
    pub fn sync(&mut self, buffer: &Buffer) {
        if self.seen_version == buffer.version() {
            return;
        }
        if let Some(edits) = buffer.edits_since(self.seen_version) {
            for edit in edits {
                self.selection.anchor = edit.adjust(self.selection.anchor);
                self.selection.head = edit.adjust(self.selection.head);
            }
        }
        let len = buffer.len_chars();
        self.selection.anchor = self.selection.anchor.min(len);
        self.selection.head = self.selection.head.min(len);
        self.seen_version = buffer.version();
    }

    fn settle(&mut self, buffer: &Buffer) {
        self.seen_version = buffer.version();
    }

    /// Ends the current typing group: the next insert starts a new undo step.
    pub fn commit(&mut self) {
        self.grouping = false;
    }

    // ---- movement -------------------------------------------------------

    fn set_head(&mut self, buffer: &Buffer, head: usize, select: bool) {
        let head = head.min(buffer.len_chars());
        self.selection.head = head;
        if !select {
            self.selection.anchor = head;
        }
        self.commit();
    }

    pub fn set_cursor(&mut self, buffer: &Buffer, offset: usize) {
        self.set_head(buffer, offset, false);
        self.goal_column = None;
    }

    pub fn select(&mut self, buffer: &Buffer, anchor: usize, head: usize) {
        let len = buffer.len_chars();
        self.selection = Selection {
            anchor: anchor.min(len),
            head: head.min(len),
        };
        self.goal_column = None;
        self.commit();
    }

    pub fn select_all(&mut self, buffer: &Buffer) {
        self.select(buffer, 0, buffer.len_chars());
    }

    pub fn move_left(&mut self, buffer: &Buffer, select: bool) {
        let range = self.selection.range();
        let head = if !select && !self.selection.is_empty() {
            range.start
        } else {
            self.selection.head.saturating_sub(1)
        };
        self.set_head(buffer, head, select);
        self.goal_column = None;
    }

    pub fn move_right(&mut self, buffer: &Buffer, select: bool) {
        let range = self.selection.range();
        let head = if !select && !self.selection.is_empty() {
            range.end
        } else {
            self.selection.head + 1
        };
        self.set_head(buffer, head, select);
        self.goal_column = None;
    }

    fn move_rows(&mut self, buffer: &Buffer, delta: isize, select: bool) {
        let point = buffer.offset_to_point(self.selection.head);
        let column = self.goal_column.unwrap_or(point.column);
        let last = buffer.line_count().saturating_sub(1) as isize;
        let row = (point.row as isize + delta).clamp(0, last) as usize;
        let head = if row == point.row {
            // Top or bottom of the text: go to its very start or end.
            if delta < 0 {
                0
            } else {
                buffer.len_chars()
            }
        } else {
            buffer.point_to_offset(Point::new(row, column))
        };
        self.set_head(buffer, head, select);
        self.goal_column = Some(column);
    }

    pub fn move_up(&mut self, buffer: &Buffer, select: bool) {
        self.move_rows(buffer, -1, select);
    }

    pub fn move_down(&mut self, buffer: &Buffer, select: bool) {
        self.move_rows(buffer, 1, select);
    }

    pub fn move_page(&mut self, buffer: &Buffer, rows: usize, up: bool, select: bool) {
        let delta = rows.max(1) as isize;
        self.move_rows(buffer, if up { -delta } else { delta }, select);
    }

    /// Home the way Fleet does it: to the first non-blank character, and from
    /// there to column zero.
    pub fn move_line_start(&mut self, buffer: &Buffer, select: bool) {
        let point = buffer.offset_to_point(self.selection.head);
        let line = buffer.line(point.row);
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        let column = if point.column == indent { 0 } else { indent };
        let head = buffer.point_to_offset(Point::new(point.row, column));
        self.set_head(buffer, head, select);
        self.goal_column = None;
    }

    pub fn move_line_end(&mut self, buffer: &Buffer, select: bool) {
        let point = buffer.offset_to_point(self.selection.head);
        let head = buffer.line_to_char(point.row) + buffer.line_len(point.row);
        self.set_head(buffer, head, select);
        self.goal_column = None;
    }

    pub fn move_doc_start(&mut self, buffer: &Buffer, select: bool) {
        self.set_head(buffer, 0, select);
        self.goal_column = None;
    }

    pub fn move_doc_end(&mut self, buffer: &Buffer, select: bool) {
        self.set_head(buffer, buffer.len_chars(), select);
        self.goal_column = None;
    }

    /// The offset of the previous word boundary before `from`.
    pub fn word_left_from(&self, buffer: &Buffer, from: usize) -> usize {
        let mut i = from;
        while i > 0 && class(buffer.char_at(i - 1).unwrap()) == CharClass::Space {
            if buffer.char_at(i - 1) == Some('\n') && i != from {
                return i;
            }
            i -= 1;
        }
        if i == 0 {
            return 0;
        }
        let kind = class(buffer.char_at(i - 1).unwrap());
        while i > 0 && class(buffer.char_at(i - 1).unwrap()) == kind {
            i -= 1;
        }
        i
    }

    /// The offset of the next word boundary after `from`.
    pub fn word_right_from(&self, buffer: &Buffer, from: usize) -> usize {
        let len = buffer.len_chars();
        let mut i = from;
        if i >= len {
            return len;
        }
        let kind = class(buffer.char_at(i).unwrap());
        if kind != CharClass::Space {
            while i < len && class(buffer.char_at(i).unwrap()) == kind {
                i += 1;
            }
        }
        while i < len && class(buffer.char_at(i).unwrap()) == CharClass::Space {
            if buffer.char_at(i) == Some('\n') && i != from {
                return i;
            }
            i += 1;
        }
        i
    }

    pub fn move_word_left(&mut self, buffer: &Buffer, select: bool) {
        let head = self.word_left_from(buffer, self.selection.head);
        self.set_head(buffer, head, select);
        self.goal_column = None;
    }

    pub fn move_word_right(&mut self, buffer: &Buffer, select: bool) {
        let head = self.word_right_from(buffer, self.selection.head);
        self.set_head(buffer, head, select);
        self.goal_column = None;
    }

    /// The word around `offset`, for double-click selection.
    pub fn word_range_at(&self, buffer: &Buffer, offset: usize) -> Range<usize> {
        let len = buffer.len_chars();
        if len == 0 {
            return 0..0;
        }
        let at = offset.min(len - 1);
        let kind = class(buffer.char_at(at).unwrap());
        let mut start = at;
        while start > 0 && class(buffer.char_at(start - 1).unwrap()) == kind {
            start -= 1;
        }
        let mut end = at + 1;
        while end < len && class(buffer.char_at(end).unwrap()) == kind {
            end += 1;
        }
        start..end
    }

    pub fn line_range(&self, buffer: &Buffer, row: usize) -> Range<usize> {
        let start = buffer.line_to_char(row);
        let end = if row + 1 < buffer.line_count() {
            buffer.line_to_char(row + 1)
        } else {
            buffer.len_chars()
        };
        start..end
    }

    // ---- editing --------------------------------------------------------

    /// Runs `edit` as one undo step, with the selection before and after.
    fn transact(
        &mut self,
        buffer: &mut Buffer,
        edit: impl FnOnce(&mut Buffer, &mut Transaction, Selection) -> Selection,
    ) {
        let mut tx = Transaction {
            before: self.selection.pair(),
            ..Default::default()
        };
        let after = edit(buffer, &mut tx, self.selection);
        tx.after = after.pair();
        self.selection = after;
        self.goal_column = None;
        buffer.push_transaction(tx);
        self.grouping = false;
        self.settle(buffer);
    }

    /// Types `text` over the selection. Consecutive typing joins one undo step.
    pub fn insert(&mut self, buffer: &mut Buffer, text: &str) {
        let range = self.selection.range();
        let joined = self.grouping
            && range.is_empty()
            && buffer
                .last_transaction_mut()
                .is_some_and(|tx| continues_typing(tx, range.start, text));
        let after = Selection::caret(range.start + text.chars().count());
        if joined {
            let mut tx = std::mem::take(buffer.last_transaction_mut().unwrap());
            buffer.replace_in(&mut tx, range, text);
            tx.after = after.pair();
            *buffer.last_transaction_mut().unwrap() = tx;
            self.selection = after;
            self.goal_column = None;
            self.settle(buffer);
        } else {
            self.transact(buffer, |buffer, tx, _| {
                buffer.replace_in(tx, range, text);
                after
            });
            self.grouping = true;
        }
    }

    /// Replaces `range` (char offsets) with `text`, leaving the caret after it.
    pub fn replace_range(&mut self, buffer: &mut Buffer, range: Range<usize>, text: &str) {
        let after = Selection::caret(range.start + text.chars().count());
        self.transact(buffer, |buffer, tx, _| {
            buffer.replace_in(tx, range, text);
            after
        });
    }

    pub fn backspace(&mut self, buffer: &mut Buffer) {
        let range = self.selection.range();
        if range.is_empty() {
            if range.start == 0 {
                return;
            }
            let point = buffer.offset_to_point(range.start);
            // Inside leading whitespace, back up to the previous tab stop.
            let line = buffer.line(point.row);
            let indent = line.chars().take_while(|c| *c == ' ').count();
            let width = if point.column > 0 && point.column <= indent {
                let back = point.column % TAB_WIDTH;
                if back == 0 {
                    TAB_WIDTH
                } else {
                    back
                }
            } else {
                1
            };
            self.replace_range(buffer, range.start - width..range.start, "");
        } else {
            self.replace_range(buffer, range, "");
        }
    }

    pub fn delete(&mut self, buffer: &mut Buffer) {
        let range = self.selection.range();
        if range.is_empty() {
            if range.start >= buffer.len_chars() {
                return;
            }
            self.replace_range(buffer, range.start..range.start + 1, "");
        } else {
            self.replace_range(buffer, range, "");
        }
    }

    pub fn delete_word_backward(&mut self, buffer: &mut Buffer) {
        let range = self.selection.range();
        if !range.is_empty() {
            return self.replace_range(buffer, range, "");
        }
        let start = self.word_left_from(buffer, range.start);
        if start < range.start {
            self.replace_range(buffer, start..range.start, "");
        }
    }

    pub fn delete_word_forward(&mut self, buffer: &mut Buffer) {
        let range = self.selection.range();
        if !range.is_empty() {
            return self.replace_range(buffer, range, "");
        }
        let end = self.word_right_from(buffer, range.start);
        if end > range.start {
            self.replace_range(buffer, range.start..end, "");
        }
    }

    /// A line break that keeps the current line's indentation.
    pub fn newline(&mut self, buffer: &mut Buffer) {
        let range = self.selection.range();
        let point = buffer.offset_to_point(range.start);
        let line = buffer.line(point.row);
        let indent: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let indent = indent.chars().take(point.column).collect::<String>();
        let text = format!("\n{indent}");
        self.replace_range(buffer, range, &text);
    }

    /// Tab: spaces to the next tab stop, or an indent of every selected line.
    pub fn tab(&mut self, buffer: &mut Buffer) {
        let range = self.selection.range();
        let start = buffer.offset_to_point(range.start);
        let end = buffer.offset_to_point(range.end);
        if start.row != end.row {
            return self.indent_lines(buffer, start.row, end.row, true);
        }
        let width = TAB_WIDTH - start.column % TAB_WIDTH;
        self.replace_range(buffer, range, &" ".repeat(width));
    }

    pub fn outdent(&mut self, buffer: &mut Buffer) {
        let range = self.selection.range();
        let start = buffer.offset_to_point(range.start).row;
        let end = buffer.offset_to_point(range.end).row;
        self.indent_lines(buffer, start, end, false);
    }

    fn indent_lines(&mut self, buffer: &mut Buffer, first: usize, last: usize, indent: bool) {
        self.transact(buffer, |buffer, tx, selection| {
            let mut anchor = selection.anchor;
            let mut head = selection.head;
            for row in first..=last {
                let line_start = buffer.line_to_char(row);
                let line = buffer.line(row);
                let change: isize = if indent {
                    buffer.replace_in(tx, line_start..line_start, &" ".repeat(TAB_WIDTH));
                    TAB_WIDTH as isize
                } else {
                    let spaces = line
                        .chars()
                        .take(TAB_WIDTH)
                        .take_while(|c| *c == ' ')
                        .count();
                    buffer.replace_in(tx, line_start..line_start + spaces, "");
                    -(spaces as isize)
                };
                for pos in [&mut anchor, &mut head] {
                    if *pos > line_start || (*pos == line_start && indent && row == first) {
                        *pos = (*pos as isize + change).max(line_start as isize) as usize;
                    }
                }
            }
            Selection { anchor, head }
        });
    }

    pub fn delete_line(&mut self, buffer: &mut Buffer) {
        let range = self.selection.range();
        let first = buffer.offset_to_point(range.start).row;
        let last = buffer.offset_to_point(range.end).row;
        let start = self.line_range(buffer, first).start;
        let end = self.line_range(buffer, last).end;
        let column = buffer.offset_to_point(range.start).column;
        self.transact(buffer, |buffer, tx, _| {
            buffer.replace_in(tx, start..end, "");
            let row = first.min(buffer.line_count().saturating_sub(1));
            Selection::caret(buffer.point_to_offset(Point::new(row, column)))
        });
    }

    pub fn duplicate_line(&mut self, buffer: &mut Buffer) {
        let range = self.selection.range();
        let first = buffer.offset_to_point(range.start).row;
        let last = buffer.offset_to_point(range.end).row;
        let start = self.line_range(buffer, first).start;
        let end = self.line_range(buffer, last).end;
        let mut text = buffer.slice(start..end);
        if !text.ends_with('\n') {
            text.insert(0, '\n');
        }
        let len = text.chars().count();
        self.transact(buffer, |buffer, tx, selection| {
            buffer.replace_in(tx, end..end, &text);
            Selection {
                anchor: selection.anchor + len,
                head: selection.head + len,
            }
        });
    }

    pub fn move_line(&mut self, buffer: &mut Buffer, up: bool) {
        let range = self.selection.range();
        let first = buffer.offset_to_point(range.start).row;
        let last = buffer.offset_to_point(range.end).row;
        let line_count = buffer.line_count();
        if (up && first == 0) || (!up && last + 1 >= line_count) {
            return;
        }
        let block = self.line_range(buffer, first).start..self.line_range(buffer, last).end;
        let other_row = if up { first - 1 } else { last + 1 };
        let other = self.line_range(buffer, other_row);
        let mut block_text = buffer.slice(block.clone());
        let mut other_text = buffer.slice(other.clone());
        // Keep every line terminated while swapping, then restore the end.
        let block_had_nl = block_text.ends_with('\n');
        let other_had_nl = other_text.ends_with('\n');
        if !block_had_nl {
            block_text.push('\n');
        }
        if !other_had_nl {
            other_text.push('\n');
        }
        let (start, end) = if up {
            (other.start, block.end)
        } else {
            (block.start, other.end)
        };
        let mut swapped = if up {
            format!("{block_text}{other_text}")
        } else {
            format!("{other_text}{block_text}")
        };
        if !(block_had_nl && other_had_nl) && swapped.ends_with('\n') {
            swapped.pop();
        }
        let shift = other_text.chars().count() as isize * if up { -1 } else { 1 };
        self.transact(buffer, |buffer, tx, selection| {
            buffer.replace_in(tx, start..end, &swapped);
            Selection {
                anchor: (selection.anchor as isize + shift) as usize,
                head: (selection.head as isize + shift) as usize,
            }
        });
    }

    /// Toggles `prefix` (for example `// `) on every line of the selection:
    /// off when all of them carry it, on otherwise, at the shallowest indent.
    pub fn toggle_line_comment(&mut self, buffer: &mut Buffer, prefix: &str) {
        let range = self.selection.range();
        let first = buffer.offset_to_point(range.start).row;
        let mut last = buffer.offset_to_point(range.end).row;
        if last > first && buffer.offset_to_point(range.end).column == 0 {
            last -= 1;
        }
        let lines: Vec<String> = (first..=last).map(|r| buffer.line(r)).collect();
        let all_commented = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .all(|l| l.trim_start().starts_with(prefix.trim_end()));
        let indent = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
            .min()
            .unwrap_or(0);
        self.transact(buffer, |buffer, tx, selection| {
            let mut anchor = selection.anchor;
            let mut head = selection.head;
            for (i, line) in lines.iter().enumerate() {
                let row = first + i;
                let line_start = buffer.line_to_char(row);
                if all_commented {
                    let lead = line.chars().take_while(|c| c.is_whitespace()).count();
                    if line.trim().is_empty() {
                        continue;
                    }
                    let rest: String = line.chars().skip(lead).collect();
                    let width = if rest.starts_with(prefix) {
                        prefix.chars().count()
                    } else {
                        prefix.trim_end().chars().count()
                    };
                    let at = line_start + lead;
                    buffer.replace_in(tx, at..at + width, "");
                    for pos in [&mut anchor, &mut head] {
                        if *pos >= at + width {
                            *pos -= width;
                        } else if *pos > at {
                            *pos = at;
                        }
                    }
                } else {
                    let at = line_start + indent.min(line.chars().count());
                    buffer.replace_in(tx, at..at, prefix);
                    let width = prefix.chars().count();
                    for pos in [&mut anchor, &mut head] {
                        if *pos >= at {
                            *pos += width;
                        }
                    }
                }
            }
            Selection { anchor, head }
        });
    }

    pub fn undo(&mut self, buffer: &mut Buffer) {
        if let Some(selection) = buffer.undo() {
            self.selection = Selection::from_pair(selection);
            self.clamp_selection(buffer);
        }
        self.settle(buffer);
        self.commit();
    }

    pub fn redo(&mut self, buffer: &mut Buffer) {
        if let Some(selection) = buffer.redo() {
            self.selection = Selection::from_pair(selection);
            self.clamp_selection(buffer);
        }
        self.settle(buffer);
        self.commit();
    }

    fn clamp_selection(&mut self, buffer: &mut Buffer) {
        let len = buffer.len_chars();
        self.selection.anchor = self.selection.anchor.min(len);
        self.selection.head = self.selection.head.min(len);
    }
}

/// One buffer with one cursor: the single-view editor, and what the tests
/// drive. Every command forwards to the cursor with the buffer.
#[derive(Clone, Debug, Default)]
pub struct TextEditor {
    pub buffer: Buffer,
    pub cursor: Cursor,
}

macro_rules! forward {
    ($( $name:ident ( $($arg:ident : $ty:ty),* ) ;)*) => {
        $( pub fn $name(&mut self $(, $arg: $ty)*) {
            self.cursor.$name(&mut self.buffer $(, $arg)*)
        } )*
    };
}

impl TextEditor {
    pub fn new(text: &str) -> Self {
        let buffer = Buffer::new(text);
        let cursor = Cursor::new(&buffer);
        TextEditor { buffer, cursor }
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn selection(&self) -> Selection {
        self.cursor.selection
    }

    pub fn cursor(&self) -> usize {
        self.cursor.cursor()
    }

    pub fn cursor_point(&self) -> Point {
        self.cursor.cursor_point(&self.buffer)
    }

    pub fn selected_text(&self) -> String {
        self.cursor.selected_text(&self.buffer)
    }

    pub fn commit(&mut self) {
        self.cursor.commit();
    }

    pub fn word_range_at(&self, offset: usize) -> Range<usize> {
        self.cursor.word_range_at(&self.buffer, offset)
    }

    pub fn line_range(&self, row: usize) -> Range<usize> {
        self.cursor.line_range(&self.buffer, row)
    }

    forward! {
        set_cursor(offset: usize);
        select(anchor: usize, head: usize);
        select_all();
        move_left(select: bool);
        move_right(select: bool);
        move_up(select: bool);
        move_down(select: bool);
        move_page(rows: usize, up: bool, select: bool);
        move_line_start(select: bool);
        move_line_end(select: bool);
        move_doc_start(select: bool);
        move_doc_end(select: bool);
        move_word_left(select: bool);
        move_word_right(select: bool);
        insert(text: &str);
        replace_range(range: Range<usize>, text: &str);
        backspace();
        delete();
        delete_word_backward();
        delete_word_forward();
        newline();
        tab();
        outdent();
        delete_line();
        duplicate_line();
        move_line(up: bool);
        toggle_line_comment(prefix: &str);
        undo();
        redo();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_cursors_on_one_buffer_follow_each_others_edits() {
        let mut buffer = Buffer::new("hello world");
        let mut a = Cursor::new(&buffer);
        let mut b = Cursor::new(&buffer);
        b.select(&buffer, 6, 11); // "world"
        a.set_cursor(&buffer, 0);
        a.insert(&mut buffer, "well, ");
        assert_eq!(buffer.text(), "well, hello world");
        b.sync(&buffer);
        assert_eq!(b.selection.range(), 12..17);
        assert_eq!(b.selected_text(&buffer), "world");
        // An edit that shortens text inside b's selection shrinks it to match.
        a.replace_range(&mut buffer, 12..15, "W");
        b.sync(&buffer);
        assert_eq!(b.selected_text(&buffer), "Wld");
        // b undoes a's last step; the restored caret is a's, clamped to the text.
        b.undo(&mut buffer);
        assert_eq!(buffer.text(), "well, hello world");
        a.sync(&buffer);
        assert!(a.selection.head <= buffer.len_chars());
    }

    fn ed(text: &str, cursor: usize) -> TextEditor {
        let mut e = TextEditor::new(text);
        e.set_cursor(cursor);
        e
    }

    #[test]
    fn typing_groups_into_one_undo_step() {
        let mut e = TextEditor::new("");
        for c in ["h", "e", "y"] {
            e.insert(c);
        }
        assert_eq!(e.text(), "hey");
        e.undo();
        assert_eq!(e.text(), "");
        e.redo();
        assert_eq!(e.text(), "hey");
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn a_move_breaks_the_typing_group() {
        let mut e = TextEditor::new("");
        e.insert("a");
        e.move_left(false);
        e.move_right(false);
        e.insert("b");
        e.undo();
        assert_eq!(e.text(), "a");
    }

    #[test]
    fn newline_keeps_indent() {
        let mut e = ed("    foo", 7);
        e.newline();
        assert_eq!(e.text(), "    foo\n    ");
        assert_eq!(e.cursor_point(), Point::new(1, 4));
    }

    #[test]
    fn backspace_in_indent_goes_to_tab_stop() {
        let mut e = ed("      x", 6);
        e.backspace();
        assert_eq!(e.text(), "    x");
        e.backspace();
        assert_eq!(e.text(), "x");
    }

    #[test]
    fn smart_home_toggles() {
        let mut e = ed("  ab", 4);
        e.move_line_start(false);
        assert_eq!(e.cursor(), 2);
        e.move_line_start(false);
        assert_eq!(e.cursor(), 0);
        e.move_line_start(false);
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn vertical_moves_keep_goal_column() {
        let mut e = ed("abcdef\nx\nabcdef", 5);
        e.move_down(false);
        assert_eq!(e.cursor_point(), Point::new(1, 1));
        e.move_down(false);
        assert_eq!(e.cursor_point(), Point::new(2, 5));
    }

    #[test]
    fn word_moves() {
        let mut e = ed("foo_bar  baz()", 0);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 9);
        e.move_word_right(false);
        assert_eq!(e.cursor(), 12);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 9);
        e.move_word_left(false);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn delete_and_duplicate_line() {
        let mut e = ed("a\nb\nc", 2);
        e.duplicate_line();
        assert_eq!(e.text(), "a\nb\nb\nc");
        assert_eq!(e.cursor_point(), Point::new(2, 0));
        e.delete_line();
        assert_eq!(e.text(), "a\nb\nc");
        assert_eq!(e.cursor_point(), Point::new(2, 0));
    }

    #[test]
    fn move_line_up_and_down() {
        let mut e = ed("a\nb\nc", 2);
        e.move_line(true);
        assert_eq!(e.text(), "b\na\nc");
        assert_eq!(e.cursor_point(), Point::new(0, 0));
        e.move_line(false);
        e.move_line(false);
        assert_eq!(e.text(), "a\nc\nb");
        assert_eq!(e.cursor_point(), Point::new(2, 0));
    }

    #[test]
    fn toggle_comment() {
        let mut e = TextEditor::new("  a\n  b");
        e.select_all();
        e.toggle_line_comment("// ");
        assert_eq!(e.text(), "  // a\n  // b");
        e.toggle_line_comment("// ");
        assert_eq!(e.text(), "  a\n  b");
    }

    #[test]
    fn tab_and_outdent_selection() {
        let mut e = TextEditor::new("a\nb");
        e.select_all();
        e.tab();
        assert_eq!(e.text(), "    a\n    b");
        e.outdent();
        assert_eq!(e.text(), "a\nb");
    }

    #[test]
    fn selection_replaced_by_typing() {
        let mut e = TextEditor::new("hello world");
        e.select(0, 5);
        e.insert("bye");
        assert_eq!(e.text(), "bye world");
        e.undo();
        assert_eq!(e.text(), "hello world");
        assert_eq!(e.selection(), Selection { anchor: 0, head: 5 });
    }
}
