//! The cursor, the selections and the edit commands over a [`Buffer`].
//!
//! A cursor holds one primary selection and any number of extra ones (the
//! secondary carets). Every movement command moves them all, every edit
//! command edits at them all inside one undo step, and after each command
//! they are kept sorted and apart: two that overlap or touch merge into one.

use crate::analysis;
use crate::buffer::{Buffer, Edit, Point, Transaction};
use crate::indent::Indent;
use crate::search::{Query, SearchOptions};
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

    /// `range` selected in this selection's direction.
    fn spanning(&self, range: Range<usize>) -> Self {
        if self.head >= self.anchor {
            Selection {
                anchor: range.start,
                head: range.end,
            }
        } else {
            Selection {
                anchor: range.end,
                head: range.start,
            }
        }
    }

    fn forward(range: Range<usize>) -> Self {
        Selection {
            anchor: range.start,
            head: range.end,
        }
    }

    fn shifted(&self, delta: isize) -> Self {
        let shift = |p: usize| (p as isize + delta).max(0) as usize;
        Selection {
            anchor: shift(self.anchor),
            head: shift(self.head),
        }
    }

    fn clamped(&self, len: usize) -> Self {
        Selection {
            anchor: self.anchor.min(len),
            head: self.head.min(len),
        }
    }

    /// Moved past one change the way a view syncs: positions after it
    /// shift, positions inside it land at its end.
    fn adjusted(&self, offset: usize, old_len: usize, new_len: usize) -> Self {
        let edit = Edit {
            version: 0,
            offset,
            old_len,
            new_len,
        };
        Selection {
            anchor: edit.adjust(self.anchor),
            head: edit.adjust(self.head),
        }
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
    } else if is_word(c) {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The bracket and quote pairs auto-pairing knows.
const PAIRS: [(char, char); 5] = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];

fn closer_of(open: char) -> Option<char> {
    PAIRS.iter().find(|(o, _)| *o == open).map(|&(_, c)| c)
}

fn opener_of(close: char) -> Option<char> {
    PAIRS
        .iter()
        .find(|(o, c)| *c == close && *o != *c)
        .map(|&(o, _)| o)
}

/// Whether the last transaction can absorb `text` typed at `carets`: plain
/// typing (no newline) whose last insert per caret ended exactly where that
/// caret now is, so a word typed in one go undoes in one go.
fn continues_typing(tx: &Transaction, carets: &[usize], text: &str) -> bool {
    if text.contains('\n') || carets.is_empty() || tx.changes.len() < carets.len() {
        return false;
    }
    // The last n changes were applied last caret first; bring each insert's
    // end into the coordinates of the text as it is now.
    let recent = &tx.changes[tx.changes.len() - carets.len()..];
    let mut ends: Vec<usize> = Vec::with_capacity(carets.len());
    for change in recent {
        if !change.removed.is_empty() || change.inserted.contains('\n') {
            return false;
        }
        let len = change.inserted.chars().count();
        for end in &mut ends {
            if *end > change.offset {
                *end += len;
            }
        }
        ends.push(change.offset + len);
    }
    ends.sort_unstable();
    let mut carets = carets.to_vec();
    carets.sort_unstable();
    ends == carets
}

// ---- text scans shared by the commands ----------------------------------

/// The offset of the previous word boundary before `from`.
fn word_left_from(buffer: &Buffer, from: usize) -> usize {
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
fn word_right_from(buffer: &Buffer, from: usize) -> usize {
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

/// The run of same-class characters around `offset`, for double-click.
fn word_range_at(buffer: &Buffer, offset: usize) -> Range<usize> {
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

/// Row `row` with its line break.
fn line_range(buffer: &Buffer, row: usize) -> Range<usize> {
    let start = buffer.line_to_char(row);
    let end = if row + 1 < buffer.line_count() {
        buffer.line_to_char(row + 1)
    } else {
        buffer.len_chars()
    };
    start..end
}

fn line_end(buffer: &Buffer, row: usize) -> usize {
    buffer.line_to_char(row) + buffer.line_len(row)
}

/// The first and last row a selection touches. With `trim_end`, a selection
/// ending at column zero of a later row does not count that row (it holds
/// none of its text), the way line commands see it.
fn rows_of(buffer: &Buffer, selection: Selection, trim_end: bool) -> (usize, usize) {
    let range = selection.range();
    let first = buffer.offset_to_point(range.start).row;
    let end = buffer.offset_to_point(range.end);
    let last = if trim_end && end.row > first && end.column == 0 {
        end.row - 1
    } else {
        end.row
    };
    (first, last)
}

/// Every row any of `selections` touches, ascending, each once.
fn row_set(buffer: &Buffer, selections: &[Selection]) -> Vec<usize> {
    let mut rows: Vec<usize> = selections
        .iter()
        .flat_map(|s| {
            let (first, last) = rows_of(buffer, *s, true);
            first..=last
        })
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// The row blocks of `selections` (first and last row each), merged where
/// they overlap, and where they are adjacent too when `join_adjacent`.
fn row_blocks(
    buffer: &Buffer,
    selections: &[Selection],
    join_adjacent: bool,
) -> Vec<(usize, usize)> {
    let mut blocks: Vec<(usize, usize)> = selections
        .iter()
        .map(|s| rows_of(buffer, *s, false))
        .collect();
    blocks.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(blocks.len());
    for (first, last) in blocks {
        if let Some(prev) = merged.last_mut() {
            let gap = if join_adjacent { 1 } else { 0 };
            if first <= prev.1 + gap {
                prev.1 = prev.1.max(last);
                continue;
            }
        }
        merged.push((first, last));
    }
    merged
}

/// The display column of char `column` in `line`, tabs expanded to `width`.
fn display_column(line: &str, column: usize, width: usize) -> usize {
    let width = width.max(1);
    let mut col = 0;
    for c in line.chars().take(column) {
        col += if c == '\t' { width - col % width } else { 1 };
    }
    col
}

/// The unmatched `open` before `offset` that a `close` typed there would
/// pair with.
fn unmatched_opener_before(
    buffer: &Buffer,
    offset: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = offset.min(buffer.len_chars());
    while i > 0 {
        i -= 1;
        let c = buffer.char_at(i)?;
        if c == close {
            depth += 1;
        } else if c == open {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
        }
    }
    None
}

/// The camel humps and snake parts of an identifier, as char ranges into
/// it: `fooBar` -> `foo`, `Bar`; `foo_bar` -> `foo`, `bar`; `HTTPServer` ->
/// `HTTP`, `Server`.
fn humps(chars: &[char]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            if let Some(s) = start.take() {
                out.push(s..i);
            }
            continue;
        }
        let Some(s) = start else {
            start = Some(i);
            continue;
        };
        let prev = chars[i - 1];
        let boundary = c.is_uppercase()
            && (!prev.is_uppercase() || chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
        if boundary && prev != '_' {
            out.push(s..i);
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(s..chars.len());
    }
    out
}

/// Indents (`on`) or outdents every row in `rows` by one level of `indent`,
/// keeping `selections` on their text.
fn indent_rows(
    buffer: &mut Buffer,
    tx: &mut Transaction,
    rows: &[usize],
    selections: &mut [Selection],
    indent: Indent,
    on: bool,
) {
    for &row in rows {
        let line_start = buffer.line_to_char(row);
        if on {
            let unit = indent.unit();
            let width = unit.chars().count();
            buffer.replace_in(tx, line_start..line_start, &unit);
            for sel in selections.iter_mut() {
                for pos in [&mut sel.anchor, &mut sel.head] {
                    if *pos >= line_start {
                        *pos += width;
                    }
                }
            }
        } else {
            let line = buffer.line(row);
            let removed = if line.starts_with('\t') {
                1
            } else {
                line.chars()
                    .take(indent.width)
                    .take_while(|c| *c == ' ')
                    .count()
            };
            if removed == 0 {
                continue;
            }
            buffer.replace_in(tx, line_start..line_start + removed, "");
            for sel in selections.iter_mut() {
                for pos in [&mut sel.anchor, &mut sel.head] {
                    if *pos >= line_start + removed {
                        *pos -= removed;
                    } else if *pos > line_start {
                        *pos = line_start;
                    }
                }
            }
        }
    }
}

/// What a typed character does at one selection with auto-pairs and the
/// indenter on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TypeAction {
    Plain,
    /// The closer is already there: just move past it.
    StepOver,
    /// Insert the character and its closer, caret between them.
    Pair(char),
    /// Put the character and its closer around the selected text.
    Wrap(char),
    /// A closer on a blank line: re-indent to its opener's line first.
    Reindent(char),
}

fn type_action(
    buffer: &Buffer,
    selection: Selection,
    c: char,
    pairs: bool,
    indenter: bool,
) -> TypeAction {
    let range = selection.range();
    let prev = range.start.checked_sub(1).and_then(|i| buffer.char_at(i));
    let next = buffer.char_at(range.end);
    if pairs {
        if let Some(close) = closer_of(c).filter(|close| *close != c) {
            if !range.is_empty() {
                return TypeAction::Wrap(close);
            }
            return if next.is_some_and(is_word) {
                TypeAction::Plain
            } else {
                TypeAction::Pair(close)
            };
        }
        if c == '"' || c == '\'' {
            if !range.is_empty() {
                return TypeAction::Wrap(c);
            }
            if next == Some(c) {
                return TypeAction::StepOver;
            }
            let blocked = prev.is_some_and(is_word)
                || next.is_some_and(|n| is_word(n) || n == '"' || n == '\'');
            return if blocked {
                TypeAction::Plain
            } else {
                TypeAction::Pair(c)
            };
        }
        if opener_of(c).is_some() && range.is_empty() && next == Some(c) {
            return TypeAction::StepOver;
        }
    }
    if indenter && range.is_empty() {
        if let Some(open) = opener_of(c) {
            let row = buffer.offset_to_point(range.start).row;
            let before = buffer.slice(buffer.line_to_char(row)..range.start);
            if before.chars().all(|c| c == ' ' || c == '\t')
                && unmatched_opener_before(buffer, range.start, open, c).is_some()
            {
                return TypeAction::Reindent(open);
            }
        }
    }
    TypeAction::Plain
}

/// What `select_next_occurrence` looks for: the primary's text, matched as
/// a whole word when it came from expanding the caret to its word.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OccurrenceQuery {
    text: String,
    whole_word: bool,
}

/// A secondary caret: its selection and, like the primary, the column it
/// wants when moving vertically.
#[derive(Clone, Copy, Debug, Default)]
struct Extra {
    selection: Selection,
    goal: Option<usize>,
}

/// Every selection with its identity while a command works on them all:
/// `rank` is the order it was added in, the primary being `usize::MAX`.
#[derive(Clone, Copy, Debug)]
struct Item {
    selection: Selection,
    goal: Option<usize>,
    rank: usize,
}

const PRIMARY: usize = usize::MAX;

/// The per-view half of an editor: where the carets and selections are, and
/// the typing group for undo. Every command takes the shared [`Buffer`].
#[derive(Clone, Debug, Default)]
pub struct Cursor {
    /// The primary selection: the one most recently added or placed.
    pub selection: Selection,
    /// The secondary selections, oldest first.
    extra: Vec<Extra>,
    /// The column the caret wants when moving vertically through shorter lines.
    goal_column: Option<usize>,
    /// Whether the next typed text may join the last undo step.
    grouping: bool,
    /// The buffer version this cursor's offsets are valid for (`sync`).
    seen_version: u64,
    /// What the occurrence commands search for, once a word was expanded.
    occurrence: Option<OccurrenceQuery>,
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

    // ---- the set of selections ------------------------------------------

    /// Every selection, the primary included, in text order.
    pub fn selections(&self) -> Vec<Selection> {
        self.sorted_items()
            .into_iter()
            .map(|i| i.selection)
            .collect()
    }

    pub fn has_multiple(&self) -> bool {
        !self.extra.is_empty()
    }

    /// Back to the primary selection alone.
    pub fn clear_extra(&mut self) {
        self.extra.clear();
    }

    fn items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = self
            .extra
            .iter()
            .enumerate()
            .map(|(rank, e)| Item {
                selection: e.selection,
                goal: e.goal,
                rank,
            })
            .collect();
        items.push(Item {
            selection: self.selection,
            goal: self.goal_column,
            rank: PRIMARY,
        });
        items
    }

    fn sorted_items(&self) -> Vec<Item> {
        let mut items = self.items();
        items.sort_by_key(|i| {
            let r = i.selection.range();
            (r.start, r.end, i.rank)
        });
        items
    }

    /// Puts `items` back as the primary and the extras in the order they
    /// were added.
    fn restore(&mut self, mut items: Vec<Item>) {
        items.sort_by_key(|i| i.rank);
        let primary = items
            .pop()
            .expect("the primary selection is always present");
        debug_assert_eq!(primary.rank, PRIMARY);
        self.selection = primary.selection;
        self.goal_column = primary.goal;
        self.extra = items
            .into_iter()
            .map(|i| Extra {
                selection: i.selection,
                goal: i.goal,
            })
            .collect();
    }

    /// The selections as the transactions store them: the primary first.
    fn pairs(&self) -> Vec<(usize, usize)> {
        std::iter::once(self.selection.pair())
            .chain(self.extra.iter().map(|e| e.selection.pair()))
            .collect()
    }

    fn set_pairs(&mut self, pairs: &[(usize, usize)]) {
        let mut pairs = pairs.iter().copied().map(Selection::from_pair);
        if let Some(primary) = pairs.next() {
            self.selection = primary;
            self.goal_column = None;
            self.extra = pairs
                .map(|selection| Extra {
                    selection,
                    goal: None,
                })
                .collect();
        }
    }

    /// Keeps the invariant: selections sorted and apart. Two that overlap
    /// or touch become one, taking the direction and identity of the one
    /// added later, so the primary stays the primary.
    fn normalize(&mut self) {
        if self.extra.is_empty() {
            return;
        }
        let items = self.sorted_items();
        let mut merged: Vec<Item> = Vec::with_capacity(items.len());
        for item in items {
            if let Some(last) = merged.last_mut() {
                let lr = last.selection.range();
                let ir = item.selection.range();
                if ir.start <= lr.end {
                    let newer = if item.rank > last.rank { item } else { *last };
                    last.selection = newer.selection.spanning(lr.start..lr.end.max(ir.end));
                    last.goal = newer.goal;
                    last.rank = newer.rank;
                    continue;
                }
            }
            merged.push(item);
        }
        self.restore(merged);
    }

    fn clamp(&mut self, len: usize) {
        self.selection = self.selection.clamped(len);
        for extra in &mut self.extra {
            extra.selection = extra.selection.clamped(len);
        }
    }

    /// Makes `selection` the primary; the old primary joins the extras.
    fn push_primary(&mut self, selection: Selection, goal: Option<usize>) {
        self.extra.push(Extra {
            selection: self.selection,
            goal: self.goal_column,
        });
        self.selection = selection;
        self.goal_column = goal;
        self.commit();
        self.normalize();
    }

    /// Moves every selection past edits other cursors made to the buffer
    /// since this one last looked, so they stay on the same text. Call
    /// before reading or acting on the selection.
    pub fn sync(&mut self, buffer: &Buffer) {
        if self.seen_version == buffer.version() {
            return;
        }
        if let Some(edits) = buffer.edits_since(self.seen_version) {
            for edit in edits {
                let adjust = |s: Selection| s.adjusted(edit.offset, edit.old_len, edit.new_len);
                self.selection = adjust(self.selection);
                for extra in &mut self.extra {
                    extra.selection = adjust(extra.selection);
                }
            }
        }
        self.clamp(buffer.len_chars());
        self.seen_version = buffer.version();
        self.normalize();
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

    /// Moves every selection's head to where `target` says, given the
    /// selection and its goal column; `target` returns the new head and
    /// the goal column to keep.
    fn move_each(
        &mut self,
        buffer: &Buffer,
        select: bool,
        target: impl Fn(&Buffer, Selection, Option<usize>) -> (usize, Option<usize>),
    ) {
        let len = buffer.len_chars();
        let moved = |sel: Selection, head: usize| Selection {
            anchor: if select { sel.anchor } else { head },
            head,
        };
        let (head, goal) = target(buffer, self.selection, self.goal_column);
        self.selection = moved(self.selection, head.min(len));
        self.goal_column = goal;
        for extra in &mut self.extra {
            let (head, goal) = target(buffer, extra.selection, extra.goal);
            extra.selection = moved(extra.selection, head.min(len));
            extra.goal = goal;
        }
        self.commit();
        self.normalize();
    }

    /// A single caret at `offset`.
    pub fn set_cursor(&mut self, buffer: &Buffer, offset: usize) {
        self.extra.clear();
        self.set_head(buffer, offset, false);
        self.goal_column = None;
    }

    /// A single selection from `anchor` to `head`.
    pub fn select(&mut self, buffer: &Buffer, anchor: usize, head: usize) {
        self.extra.clear();
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

    /// A single caret at `row`, `column` (zero-based, clamped to the text).
    pub fn goto(&mut self, buffer: &Buffer, row: usize, column: usize) {
        let offset = buffer.point_to_offset(Point::new(row, column));
        self.set_cursor(buffer, offset);
    }

    pub fn move_left(&mut self, buffer: &Buffer, select: bool) {
        self.move_each(buffer, select, |_, sel, _| {
            let head = if !select && !sel.is_empty() {
                sel.range().start
            } else {
                sel.head.saturating_sub(1)
            };
            (head, None)
        });
    }

    pub fn move_right(&mut self, buffer: &Buffer, select: bool) {
        self.move_each(buffer, select, |_, sel, _| {
            let head = if !select && !sel.is_empty() {
                sel.range().end
            } else {
                sel.head + 1
            };
            (head, None)
        });
    }

    fn move_rows(&mut self, buffer: &Buffer, delta: isize, select: bool) {
        self.move_each(buffer, select, |buffer, sel, goal| {
            let point = buffer.offset_to_point(sel.head);
            let column = goal.unwrap_or(point.column);
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
            (head, Some(column))
        });
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
        self.move_each(buffer, select, |buffer, sel, _| {
            let point = buffer.offset_to_point(sel.head);
            let line = buffer.line(point.row);
            let indent = line.chars().take_while(|c| c.is_whitespace()).count();
            let column = if point.column == indent { 0 } else { indent };
            (buffer.point_to_offset(Point::new(point.row, column)), None)
        });
    }

    pub fn move_line_end(&mut self, buffer: &Buffer, select: bool) {
        self.move_each(buffer, select, |buffer, sel, _| {
            let row = buffer.offset_to_point(sel.head).row;
            (line_end(buffer, row), None)
        });
    }

    /// To the start of the text, as one caret again.
    pub fn move_doc_start(&mut self, buffer: &Buffer, select: bool) {
        self.extra.clear();
        self.set_head(buffer, 0, select);
        self.goal_column = None;
    }

    pub fn move_doc_end(&mut self, buffer: &Buffer, select: bool) {
        self.extra.clear();
        self.set_head(buffer, buffer.len_chars(), select);
        self.goal_column = None;
    }

    /// The offset of the previous word boundary before `from`.
    pub fn word_left_from(&self, buffer: &Buffer, from: usize) -> usize {
        word_left_from(buffer, from)
    }

    /// The offset of the next word boundary after `from`.
    pub fn word_right_from(&self, buffer: &Buffer, from: usize) -> usize {
        word_right_from(buffer, from)
    }

    pub fn move_word_left(&mut self, buffer: &Buffer, select: bool) {
        self.move_each(buffer, select, |buffer, sel, _| {
            (word_left_from(buffer, sel.head), None)
        });
    }

    pub fn move_word_right(&mut self, buffer: &Buffer, select: bool) {
        self.move_each(buffer, select, |buffer, sel, _| {
            (word_right_from(buffer, sel.head), None)
        });
    }

    /// The word around `offset`, for double-click selection.
    pub fn word_range_at(&self, buffer: &Buffer, offset: usize) -> Range<usize> {
        word_range_at(buffer, offset)
    }

    pub fn line_range(&self, buffer: &Buffer, row: usize) -> Range<usize> {
        line_range(buffer, row)
    }

    // ---- adding and removing carets -------------------------------------

    fn add_caret_rows(&mut self, buffer: &Buffer, delta: isize) {
        let items = self.sorted_items();
        let edge = if delta < 0 {
            items.first()
        } else {
            items.last()
        }
        .copied()
        .expect("the primary selection is always present");
        let point = buffer.offset_to_point(edge.selection.head);
        let row = point.row as isize + delta;
        if row < 0 || row >= buffer.line_count() as isize {
            return;
        }
        let column = edge.goal.unwrap_or(point.column);
        let at = buffer.point_to_offset(Point::new(row as usize, column));
        self.push_primary(Selection::caret(at), Some(column));
    }

    /// A new caret on the row above the topmost selection, at the column it
    /// aims for, which the new caret keeps so repeated calls climb straight.
    pub fn add_caret_above(&mut self, buffer: &Buffer) {
        self.add_caret_rows(buffer, -1);
    }

    pub fn add_caret_below(&mut self, buffer: &Buffer) {
        self.add_caret_rows(buffer, 1);
    }

    /// Replaces every selection: `selections[primary]` becomes the primary,
    /// the rest the extras, clamped to the text and merged where they meet.
    /// For a view that computes the targets itself (vertical movement
    /// through wrapped and folded rows).
    pub fn set_selections(&mut self, buffer: &Buffer, selections: Vec<Selection>, primary: usize) {
        if selections.is_empty() {
            return;
        }
        let len = buffer.len_chars();
        let primary = primary.min(selections.len() - 1);
        self.extra.clear();
        for (i, selection) in selections.into_iter().enumerate() {
            if i == primary {
                self.selection = selection.clamped(len);
            } else {
                self.extra.push(Extra {
                    selection: selection.clamped(len),
                    goal: None,
                });
            }
        }
        self.goal_column = None;
        self.commit();
        self.normalize();
    }

    /// Alt+click: a new empty caret at `offset` as the primary; on a caret
    /// already there, that caret goes instead (while another remains).
    pub fn add_caret_at(&mut self, buffer: &Buffer, offset: usize) {
        let offset = offset.min(buffer.len_chars());
        let caret = Selection::caret(offset);
        if self.has_multiple() {
            if self.selection == caret {
                self.unselect_occurrence(buffer);
                return;
            }
            if let Some(i) = self.extra.iter().position(|e| e.selection == caret) {
                self.extra.remove(i);
                self.commit();
                return;
            }
        }
        self.push_primary(caret, None);
    }

    /// A caret at the end of every line the primary selection touches, in
    /// place of the selection (Fleet's "add carets to ends of selected lines").
    pub fn add_carets_to_ends_of_selected_lines(&mut self, buffer: &Buffer) {
        let (first, last) = rows_of(buffer, self.selection, true);
        let mut ends: Vec<usize> = (first..=last).map(|row| line_end(buffer, row)).collect();
        let primary = ends.pop().expect("a selection touches a row");
        self.extra.extend(ends.into_iter().map(|at| Extra {
            selection: Selection::caret(at),
            goal: None,
        }));
        self.selection = Selection::caret(primary);
        self.goal_column = None;
        self.commit();
        self.normalize();
    }

    /// Selects the identifier at the caret and makes it the occurrence
    /// query. Whether there was one.
    fn select_word_at_caret(&mut self, buffer: &Buffer) -> bool {
        let Some(range) = analysis::identifier_at(buffer, self.selection.head) else {
            return false;
        };
        self.occurrence = Some(OccurrenceQuery {
            text: buffer.slice(range.clone()),
            whole_word: true,
        });
        self.selection = Selection::forward(range);
        self.goal_column = None;
        self.commit();
        self.normalize();
        true
    }

    /// Every occurrence of the primary's text, matched as a whole word when
    /// the text is the word the caret was expanded to.
    fn occurrences(&mut self, buffer: &Buffer) -> Vec<Range<usize>> {
        let text = self.selected_text(buffer);
        if text.is_empty() {
            return Vec::new();
        }
        let whole_word = self
            .occurrence
            .as_ref()
            .is_some_and(|q| q.text == text && q.whole_word);
        self.occurrence = Some(OccurrenceQuery {
            text: text.clone(),
            whole_word,
        });
        let options = SearchOptions {
            case_sensitive: true,
            whole_word,
            ..Default::default()
        };
        Query::new(&text, options).map_or_else(|_| Vec::new(), |q| q.find_in(buffer))
    }

    /// With a bare caret, selects its word; otherwise adds the next
    /// occurrence of the primary's text after the last selection, wrapping
    /// round the text, as the new primary.
    pub fn select_next_occurrence(&mut self, buffer: &Buffer) {
        if self.selection.is_empty() {
            self.select_word_at_caret(buffer);
            return;
        }
        let found = self.occurrences(buffer);
        let selected = self.selections();
        let last_end = selected.last().map_or(0, |s| s.range().end);
        let is_selected = |r: &Range<usize>| selected.iter().any(|s| s.range() == *r);
        let next = found
            .iter()
            .filter(|r| r.start >= last_end)
            .chain(found.iter())
            .find(|r| !is_selected(r))
            .cloned();
        if let Some(range) = next {
            self.push_primary(Selection::forward(range), None);
        }
    }

    /// Selects every occurrence of the primary's text (its word, for a bare
    /// caret); the primary is the one at or after where the caret was.
    pub fn select_all_occurrences(&mut self, buffer: &Buffer) {
        if self.selection.is_empty() && !self.select_word_at_caret(buffer) {
            return;
        }
        let found = self.occurrences(buffer);
        if found.is_empty() {
            return;
        }
        let origin = self.selection.range().start;
        let primary = found.iter().position(|r| r.start >= origin).unwrap_or(0);
        for (i, range) in found.iter().enumerate() {
            if i != primary {
                self.extra.push(Extra {
                    selection: Selection::forward(range.clone()),
                    goal: None,
                });
            }
        }
        self.selection = Selection::forward(found[primary].clone());
        self.goal_column = None;
        self.commit();
        self.normalize();
    }

    /// Drops the selection added last; the one before it is the primary again.
    pub fn unselect_occurrence(&mut self, _buffer: &Buffer) {
        if let Some(previous) = self.extra.pop() {
            self.selection = previous.selection;
            self.goal_column = previous.goal;
            self.commit();
        }
    }

    /// Selects the word around the caret: with `camel_humps`, first the hump
    /// (`fooBar` -> `foo` or `Bar`, `foo_bar` -> `foo` or `bar`) and on the
    /// next call the whole identifier. Primary only. False when nothing
    /// changed, so the caller can go on to a syntax node.
    pub fn expand_to_word(&mut self, buffer: &Buffer, camel_humps: bool) -> bool {
        let sel = self.selection;
        let range = sel.range();
        let Some(ident) = analysis::identifier_at(buffer, range.start) else {
            return false;
        };
        let target = if sel.is_empty() {
            let hump = camel_humps.then(|| {
                let chars: Vec<char> = buffer.slice(ident.clone()).chars().collect();
                let at = sel.head - ident.start;
                let parts = humps(&chars);
                let part = parts
                    .iter()
                    .find(|p| p.start <= at && at < p.end)
                    .or_else(|| parts.iter().rev().find(|p| p.end <= at))
                    .or_else(|| parts.first())
                    .cloned()?;
                Some(ident.start + part.start..ident.start + part.end)
            });
            hump.flatten().unwrap_or(ident)
        } else if ident.start <= range.start && range.end <= ident.end && range != ident {
            ident
        } else {
            return false;
        };
        if target == range {
            return false;
        }
        self.selection = Selection::forward(target);
        self.goal_column = None;
        self.commit();
        self.normalize();
        true
    }

    // ---- editing --------------------------------------------------------

    /// Runs `edit` once per selection, last in the text first so the
    /// earlier offsets hold, as one undo step. `edit` gets the selection's
    /// index in text order and returns where it ends up; the results of
    /// later selections are shifted by what the earlier edits added or
    /// removed. With `join`, the edits go into the last transaction
    /// instead of a new one (typing that continues a word).
    fn run_edit(
        &mut self,
        buffer: &mut Buffer,
        join: bool,
        mut edit: impl FnMut(&mut Buffer, &mut Transaction, usize, Selection) -> Selection,
    ) {
        let mut tx = if join {
            std::mem::take(
                buffer
                    .last_transaction_mut()
                    .expect("typing joins an existing transaction"),
            )
        } else {
            Transaction {
                before: self.pairs(),
                ..Default::default()
            }
        };
        let mut items = self.sorted_items();
        for i in (0..items.len()).rev() {
            let len_before = buffer.len_chars() as isize;
            let after = edit(buffer, &mut tx, i, items[i].selection);
            let delta = buffer.len_chars() as isize - len_before;
            for later in &mut items[i + 1..] {
                later.selection = later.selection.shifted(delta);
            }
            items[i].selection = after;
            items[i].goal = None;
        }
        let len = buffer.len_chars();
        for item in &mut items {
            item.selection = item.selection.clamped(len);
        }
        self.restore(items);
        self.finish(buffer, tx, join);
    }

    /// Runs `edit` once with every selection in hand (the primary first),
    /// for the line commands that work on the rows the selections share;
    /// `edit` moves the selections itself.
    fn run_lines(
        &mut self,
        buffer: &mut Buffer,
        edit: impl FnOnce(&mut Buffer, &mut Transaction, &mut [Selection]),
    ) {
        let mut tx = Transaction {
            before: self.pairs(),
            ..Default::default()
        };
        let mut selections: Vec<Selection> = std::iter::once(self.selection)
            .chain(self.extra.iter().map(|e| e.selection))
            .collect();
        edit(buffer, &mut tx, &mut selections);
        let len = buffer.len_chars();
        let pairs: Vec<(usize, usize)> = selections.iter().map(|s| s.clamped(len).pair()).collect();
        self.set_pairs(&pairs);
        self.finish(buffer, tx, false);
    }

    /// Runs `edit` at the primary selection only, as one undo step; the
    /// extras move past its changes the way a view syncs.
    fn transact(
        &mut self,
        buffer: &mut Buffer,
        edit: impl FnOnce(&mut Buffer, &mut Transaction, Selection) -> Selection,
    ) {
        let mut tx = Transaction {
            before: self.pairs(),
            ..Default::default()
        };
        let after = edit(buffer, &mut tx, self.selection);
        for change in &tx.changes {
            let old_len = change.removed.chars().count();
            let new_len = change.inserted.chars().count();
            for extra in &mut self.extra {
                extra.selection = extra.selection.adjusted(change.offset, old_len, new_len);
            }
        }
        let len = buffer.len_chars();
        self.selection = after.clamped(len);
        self.goal_column = None;
        for extra in &mut self.extra {
            extra.goal = None;
        }
        self.clamp(len);
        self.finish(buffer, tx, false);
    }

    /// The end of every edit: the invariant, the transaction's selections
    /// after, and the typing group closed.
    fn finish(&mut self, buffer: &mut Buffer, mut tx: Transaction, joined: bool) {
        self.normalize();
        tx.after = self.pairs();
        if joined {
            *buffer
                .last_transaction_mut()
                .expect("typing joins an existing transaction") = tx;
        } else {
            buffer.push_transaction(tx);
        }
        self.grouping = false;
        self.settle(buffer);
    }

    /// Types `text` over every selection. Consecutive typing joins one undo step.
    pub fn insert(&mut self, buffer: &mut Buffer, text: &str) {
        let selections = self.selections();
        let carets: Vec<usize> = selections.iter().map(|s| s.head).collect();
        let join = self.grouping
            && selections.iter().all(Selection::is_empty)
            && buffer
                .last_transaction_mut()
                .is_some_and(|tx| continues_typing(tx, &carets, text));
        let len = text.chars().count();
        self.run_edit(buffer, join, |buffer, tx, _, sel| {
            let range = sel.range();
            buffer.replace_in(tx, range.clone(), text);
            Selection::caret(range.start + len)
        });
        self.grouping = true;
    }

    /// Replaces `range` (char offsets) with `text`, leaving the primary caret
    /// after it.
    pub fn replace_range(&mut self, buffer: &mut Buffer, range: Range<usize>, text: &str) {
        let after = Selection::caret(range.start + text.chars().count());
        self.transact(buffer, |buffer, tx, _| {
            buffer.replace_in(tx, range, text);
            after
        });
    }

    /// Several replacements that do not overlap (char ranges, in any order)
    /// as one undo step (replace-all); afterwards a single caret after the
    /// last replacement in text order.
    pub fn replace_ranges(&mut self, buffer: &mut Buffer, mut edits: Vec<(Range<usize>, String)>) {
        if edits.is_empty() {
            return;
        }
        let mut tx = Transaction {
            before: self.pairs(),
            ..Default::default()
        };
        // Last in the text first, so the earlier offsets hold.
        edits.sort_by_key(|(range, _)| (range.start, range.end));
        let mut caret = 0usize;
        for (i, (range, text)) in edits.iter().enumerate().rev() {
            let range = range.start.min(buffer.len_chars())..range.end.min(buffer.len_chars());
            let new_len = text.chars().count();
            if i + 1 == edits.len() {
                caret = range.start + new_len;
            } else {
                caret = (caret as isize + new_len as isize - range.len() as isize).max(0) as usize;
            }
            buffer.replace_in(&mut tx, range, text);
        }
        self.extra.clear();
        self.selection = Selection::caret(caret.min(buffer.len_chars()));
        self.goal_column = None;
        self.finish(buffer, tx, false);
    }

    pub fn backspace(&mut self, buffer: &mut Buffer) {
        self.backspace_with(buffer, false);
    }

    /// Backspace; with `pairs`, between the two halves of an auto-pair
    /// (`()`, `[]`, `{}`, `""`, `''`) both go.
    pub fn backspace_with(&mut self, buffer: &mut Buffer, pairs: bool) {
        self.run_edit(buffer, false, |buffer, tx, _, sel| {
            let range = sel.range();
            if !range.is_empty() {
                buffer.replace_in(tx, range.clone(), "");
                return Selection::caret(range.start);
            }
            let at = range.start;
            if at == 0 {
                return sel;
            }
            let prev = buffer.char_at(at - 1);
            let next = buffer.char_at(at);
            if pairs && prev.is_some_and(|p| closer_of(p).is_some() && closer_of(p) == next) {
                buffer.replace_in(tx, at - 1..at + 1, "");
                return Selection::caret(at - 1);
            }
            // Inside leading whitespace, back up to the previous tab stop.
            let tab = buffer.indent.width.max(1);
            let point = buffer.offset_to_point(at);
            let line = buffer.line(point.row);
            let indent = line.chars().take_while(|c| *c == ' ').count();
            let width = if point.column > 0 && point.column <= indent {
                let back = point.column % tab;
                if back == 0 {
                    tab
                } else {
                    back
                }
            } else {
                1
            };
            buffer.replace_in(tx, at - width..at, "");
            Selection::caret(at - width)
        });
    }

    pub fn delete(&mut self, buffer: &mut Buffer) {
        self.run_edit(buffer, false, |buffer, tx, _, sel| {
            let range = sel.range();
            if range.is_empty() {
                if range.start >= buffer.len_chars() {
                    return sel;
                }
                buffer.replace_in(tx, range.start..range.start + 1, "");
            } else {
                buffer.replace_in(tx, range.clone(), "");
            }
            Selection::caret(range.start)
        });
    }

    pub fn delete_word_backward(&mut self, buffer: &mut Buffer) {
        self.run_edit(buffer, false, |buffer, tx, _, sel| {
            let range = sel.range();
            let start = if range.is_empty() {
                word_left_from(buffer, range.start)
            } else {
                range.start
            };
            if start < range.end {
                buffer.replace_in(tx, start..range.end, "");
            }
            Selection::caret(start)
        });
    }

    pub fn delete_word_forward(&mut self, buffer: &mut Buffer) {
        self.run_edit(buffer, false, |buffer, tx, _, sel| {
            let range = sel.range();
            let end = if range.is_empty() {
                word_right_from(buffer, range.start)
            } else {
                range.end
            };
            if end > range.start {
                buffer.replace_in(tx, range.start..end, "");
            }
            Selection::caret(range.start)
        });
    }

    /// A line break that keeps the current line's indentation.
    pub fn newline(&mut self, buffer: &mut Buffer) {
        self.newline_with(buffer, false);
    }

    /// A line break keeping the line's indentation; with `indenter`, after
    /// an opening bracket the new line goes one level deeper, and a closer
    /// right after the caret moves to a line of its own at the old indent.
    pub fn newline_with(&mut self, buffer: &mut Buffer, indenter: bool) {
        self.run_edit(buffer, false, |buffer, tx, _, sel| {
            let range = sel.range();
            let point = buffer.offset_to_point(range.start);
            let line = buffer.line(point.row);
            let indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .take(point.column)
                .collect();
            let prev = range.start.checked_sub(1).and_then(|i| buffer.char_at(i));
            let next = buffer.char_at(range.end);
            let opener = indenter && prev.is_some_and(|c| matches!(c, '{' | '(' | '['));
            let (text, caret) = if opener {
                let unit = buffer.indent.unit();
                let caret = range.start + 1 + indent.chars().count() + unit.chars().count();
                let closes = prev.and_then(closer_of) == next;
                let text = if closes {
                    format!("\n{indent}{unit}\n{indent}")
                } else {
                    format!("\n{indent}{unit}")
                };
                (text, caret)
            } else {
                let text = format!("\n{indent}");
                let caret = range.start + text.chars().count();
                (text, caret)
            };
            buffer.replace_in(tx, range, &text);
            Selection::caret(caret)
        });
    }

    /// What a typed character does with auto-pairs and the indenter on
    /// (Fleet's `enableIndenter`): openers insert their pair or wrap the
    /// selection, a closer already there is stepped over, a closer typed on
    /// a blank line first re-indents it to its opener's line. Anything else
    /// is plain typing.
    pub fn type_char(&mut self, buffer: &mut Buffer, c: char, pairs: bool, indenter: bool) {
        let special = self
            .items()
            .iter()
            .any(|i| type_action(buffer, i.selection, c, pairs, indenter) != TypeAction::Plain);
        if !special {
            return self.insert(buffer, &c.to_string());
        }
        self.run_edit(buffer, false, |buffer, tx, _, sel| {
            let range = sel.range();
            match type_action(buffer, sel, c, pairs, indenter) {
                TypeAction::Plain => {
                    buffer.replace_in(tx, range.clone(), &c.to_string());
                    Selection::caret(range.start + 1)
                }
                TypeAction::StepOver => Selection::caret(range.start + 1),
                TypeAction::Pair(close) => {
                    buffer.replace_in(tx, range.clone(), &format!("{c}{close}"));
                    Selection::caret(range.start + 1)
                }
                TypeAction::Wrap(close) => {
                    let text = buffer.slice(range.clone());
                    let len = text.chars().count();
                    buffer.replace_in(tx, range.clone(), &format!("{c}{text}{close}"));
                    sel.spanning(range.start + 1..range.start + 1 + len)
                }
                TypeAction::Reindent(open) => {
                    let opener = unmatched_opener_before(buffer, range.start, open, c)
                        .expect("the action was chosen because an opener exists");
                    let row = buffer.offset_to_point(opener).row;
                    let columns =
                        analysis::indent_of(&buffer.line(row), buffer.indent.width).unwrap_or(0);
                    let indent = buffer.indent.to_columns(columns);
                    let line_start = buffer.line_to_char(buffer.offset_to_point(range.start).row);
                    buffer.replace_in(tx, line_start..range.end, &format!("{indent}{c}"));
                    Selection::caret(line_start + indent.chars().count() + 1)
                }
            }
        });
    }

    /// Tab: to the next tab stop at every caret, or when a selection spans
    /// rows, one more level on every selected line.
    pub fn tab(&mut self, buffer: &mut Buffer) {
        let indent = buffer.indent;
        let spans_rows = self.items().iter().any(|i| {
            let (first, last) = rows_of(buffer, i.selection, false);
            first != last
        });
        if spans_rows {
            return self.run_lines(buffer, |buffer, tx, sels| {
                let rows = row_set(buffer, sels);
                indent_rows(buffer, tx, &rows, sels, indent, true);
            });
        }
        self.run_edit(buffer, false, |buffer, tx, _, sel| {
            let range = sel.range();
            let text = if indent.use_tabs {
                "\t".to_string()
            } else {
                let point = buffer.offset_to_point(range.start);
                let column = display_column(&buffer.line(point.row), point.column, indent.width);
                " ".repeat(indent.width.max(1) - column % indent.width.max(1))
            };
            buffer.replace_in(tx, range.clone(), &text);
            Selection::caret(range.start + text.chars().count())
        });
    }

    /// One level less on every line a selection touches: a leading tab, or
    /// up to the indent width in spaces.
    pub fn outdent(&mut self, buffer: &mut Buffer) {
        let indent = buffer.indent;
        self.run_lines(buffer, |buffer, tx, sels| {
            let rows = row_set(buffer, sels);
            indent_rows(buffer, tx, &rows, sels, indent, false);
        });
    }

    /// Deletes every line a selection touches; each caret lands on the line
    /// that takes its first row's place, at its column.
    pub fn delete_line(&mut self, buffer: &mut Buffer) {
        self.run_lines(buffer, |buffer, tx, sels| {
            let blocks = row_blocks(buffer, sels, false);
            let targets: Vec<Point> = sels
                .iter()
                .map(|s| buffer.offset_to_point(s.range().start))
                .collect();
            for &(first, last) in blocks.iter().rev() {
                let start = line_range(buffer, first).start;
                let end = line_range(buffer, last).end;
                buffer.replace_in(tx, start..end, "");
            }
            let last_row = buffer.line_count().saturating_sub(1);
            for (sel, target) in sels.iter_mut().zip(targets) {
                let removed_above: usize = blocks
                    .iter()
                    .filter(|b| b.1 < target.row)
                    .map(|b| b.1 - b.0 + 1)
                    .sum();
                let block_first = blocks
                    .iter()
                    .find(|b| b.0 <= target.row && target.row <= b.1)
                    .map_or(target.row, |b| b.0);
                let row = (block_first - removed_above).min(last_row);
                *sel = Selection::caret(buffer.point_to_offset(Point::new(row, target.column)));
            }
        });
    }

    /// Copies the lines every selection touches below themselves; the
    /// selections move onto the copies.
    pub fn duplicate_line(&mut self, buffer: &mut Buffer) {
        self.run_lines(buffer, |buffer, tx, sels| {
            let blocks = row_blocks(buffer, sels, false);
            for &(first, last) in blocks.iter().rev() {
                let start = line_range(buffer, first).start;
                let end = line_range(buffer, last).end;
                let mut text = buffer.slice(start..end);
                if !text.ends_with('\n') {
                    text.insert(0, '\n');
                }
                let len = text.chars().count();
                buffer.replace_in(tx, end..end, &text);
                for sel in sels.iter_mut() {
                    for pos in [&mut sel.anchor, &mut sel.head] {
                        if *pos >= start {
                            *pos += len;
                        }
                    }
                }
            }
        });
    }

    /// Moves the lines every selection touches one row up or down, adjacent
    /// blocks together. Nothing moves when a block is already at the edge.
    pub fn move_line(&mut self, buffer: &mut Buffer, up: bool) {
        let blocks = row_blocks(buffer, &self.selections(), true);
        let line_count = buffer.line_count();
        let blocked = blocks
            .iter()
            .any(|&(first, last)| (up && first == 0) || (!up && last + 1 >= line_count));
        if blocked {
            return;
        }
        self.run_lines(buffer, |buffer, tx, sels| {
            // Up: top block first, so the ones below keep their offsets
            // until their turn; down: bottom block first.
            let order: Vec<(usize, usize)> = if up {
                blocks
            } else {
                blocks.into_iter().rev().collect()
            };
            for (first, last) in order {
                let block = line_range(buffer, first).start..line_range(buffer, last).end;
                let other_row = if up { first - 1 } else { last + 1 };
                let other = line_range(buffer, other_row);
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
                buffer.replace_in(tx, start..end, &swapped);
                for sel in sels.iter_mut() {
                    for pos in [&mut sel.anchor, &mut sel.head] {
                        if block.start <= *pos && *pos <= block.end {
                            *pos = (*pos as isize + shift).max(0) as usize;
                        }
                    }
                }
            }
        });
    }

    /// Toggles `prefix` (for example `// `) on every line the selections
    /// touch: off when all of them carry it, on otherwise, at the
    /// shallowest indent.
    pub fn toggle_line_comment(&mut self, buffer: &mut Buffer, prefix: &str) {
        self.run_lines(buffer, |buffer, tx, sels| {
            let rows = row_set(buffer, sels);
            let lines: Vec<String> = rows.iter().map(|&r| buffer.line(r)).collect();
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
            for (&row, line) in rows.iter().zip(&lines) {
                let line_start = buffer.line_to_char(row);
                if all_commented {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let lead = line.chars().take_while(|c| c.is_whitespace()).count();
                    let rest: String = line.chars().skip(lead).collect();
                    let width = if rest.starts_with(prefix) {
                        prefix.chars().count()
                    } else {
                        prefix.trim_end().chars().count()
                    };
                    let at = line_start + lead;
                    buffer.replace_in(tx, at..at + width, "");
                    for sel in sels.iter_mut() {
                        for pos in [&mut sel.anchor, &mut sel.head] {
                            if *pos >= at + width {
                                *pos -= width;
                            } else if *pos > at {
                                *pos = at;
                            }
                        }
                    }
                } else {
                    let at = line_start + indent.min(line.chars().count());
                    buffer.replace_in(tx, at..at, prefix);
                    let width = prefix.chars().count();
                    for sel in sels.iter_mut() {
                        for pos in [&mut sel.anchor, &mut sel.head] {
                            if *pos >= at {
                                *pos += width;
                            }
                        }
                    }
                }
            }
        });
    }

    /// Wraps every selection in `open`/`close` (`/* text */`), or takes
    /// them off again when the selection already carries them; a bare caret
    /// does the same to its whole line.
    pub fn toggle_block_comment(&mut self, buffer: &mut Buffer, open: &str, close: &str) {
        self.run_edit(buffer, false, |buffer, tx, _, sel| {
            let range = if sel.is_empty() {
                let row = buffer.offset_to_point(sel.head).row;
                buffer.line_to_char(row)..line_end(buffer, row)
            } else {
                sel.range()
            };
            let text = buffer.slice(range.clone());
            let trimmed = text.trim();
            let lead = text.chars().take_while(|c| c.is_whitespace()).count();
            let trail = text.chars().count() - lead - trimmed.chars().count();
            let margin = open.chars().count() + 1;
            let wrapped = trimmed.len() >= open.len() + close.len()
                && trimmed.starts_with(open)
                && trimmed.ends_with(close);
            let inner = if wrapped {
                let inner = &trimmed[open.len()..trimmed.len() - close.len()];
                let inner = inner.strip_prefix(' ').unwrap_or(inner);
                inner.strip_suffix(' ').unwrap_or(inner).to_string()
            } else {
                format!("{open} {trimmed} {close}")
            };
            let new = format!(
                "{}{inner}{}",
                &text[..text.char_indices().nth(lead).map_or(text.len(), |(b, _)| b)],
                &text[text.len()
                    - text
                        .chars()
                        .rev()
                        .take(trail)
                        .map(char::len_utf8)
                        .sum::<usize>()..]
            );
            let new_len = new.chars().count();
            buffer.replace_in(tx, range.clone(), &new);
            if !sel.is_empty() {
                return sel.spanning(range.start..range.start + new_len);
            }
            let rel = sel.head - range.start;
            let rel = if rel < lead {
                rel
            } else if wrapped {
                rel.saturating_sub(margin).max(lead)
            } else {
                rel + margin
            };
            Selection::caret(range.start + rel.min(new_len))
        });
    }

    /// Every selection's text to lower case when it is all upper case, to
    /// upper case otherwise (Fleet's `toggle-case`); a bare caret toggles
    /// its word.
    pub fn toggle_case(&mut self, buffer: &mut Buffer) {
        self.run_edit(buffer, false, |buffer, tx, _, sel| {
            let range = if sel.is_empty() {
                match analysis::identifier_at(buffer, sel.head) {
                    Some(range) => range,
                    None => return sel,
                }
            } else {
                sel.range()
            };
            let text = buffer.slice(range.clone());
            let new = if text.chars().any(char::is_lowercase) {
                text.to_uppercase()
            } else {
                text.to_lowercase()
            };
            if new == text {
                return sel;
            }
            let new_len = new.chars().count();
            buffer.replace_in(tx, range.clone(), &new);
            if sel.is_empty() {
                Selection::caret(sel.head.min(range.start + new_len))
            } else {
                sel.spanning(range.start..range.start + new_len)
            }
        });
    }

    /// Pastes `text` at every selection; when there are as many carets as
    /// `text` has lines, each caret gets its own line.
    pub fn paste(&mut self, buffer: &mut Buffer, text: &str) {
        let carets = self.extra.len() + 1;
        let lines: Vec<&str> = text
            .strip_suffix('\n')
            .unwrap_or(text)
            .split('\n')
            .collect();
        let per_caret = carets > 1 && lines.len() == carets;
        self.run_edit(buffer, false, |buffer, tx, i, sel| {
            let piece = if per_caret { lines[i] } else { text };
            let range = sel.range();
            buffer.replace_in(tx, range.clone(), piece);
            Selection::caret(range.start + piece.chars().count())
        });
    }

    pub fn undo(&mut self, buffer: &mut Buffer) {
        if let Some(selections) = buffer.undo() {
            self.restore_after_history(buffer, &selections);
        }
        self.settle(buffer);
        self.commit();
    }

    pub fn redo(&mut self, buffer: &mut Buffer) {
        if let Some(selections) = buffer.redo() {
            self.restore_after_history(buffer, &selections);
        }
        self.settle(buffer);
        self.commit();
    }

    /// Puts the carets where a transaction recorded them; a step that
    /// recorded none (a trim on save) leaves this cursor's own, moved past
    /// the change.
    fn restore_after_history(&mut self, buffer: &Buffer, selections: &[(usize, usize)]) {
        if selections.is_empty() {
            self.sync(buffer);
        } else {
            self.set_pairs(selections);
        }
        self.clamp(buffer.len_chars());
        self.normalize();
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
    ($( $name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)? ;)*) => {
        $( pub fn $name(&mut self $(, $arg: $ty)*) $(-> $ret)? {
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

    pub fn selections(&self) -> Vec<Selection> {
        self.cursor.selections()
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
        goto(row: usize, column: usize);
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
        set_selections(selections: Vec<Selection>, primary: usize);
        add_caret_at(offset: usize);
        add_caret_above();
        add_caret_below();
        add_carets_to_ends_of_selected_lines();
        select_next_occurrence();
        select_all_occurrences();
        unselect_occurrence();
        expand_to_word(camel_humps: bool) -> bool;
        insert(text: &str);
        replace_range(range: Range<usize>, text: &str);
        replace_ranges(edits: Vec<(Range<usize>, String)>);
        backspace();
        backspace_with(pairs: bool);
        delete();
        delete_word_backward();
        delete_word_forward();
        newline();
        newline_with(indenter: bool);
        type_char(c: char, pairs: bool, indenter: bool);
        tab();
        outdent();
        delete_line();
        duplicate_line();
        move_line(up: bool);
        toggle_line_comment(prefix: &str);
        toggle_block_comment(open: &str, close: &str);
        toggle_case();
        paste(text: &str);
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

    /// An editor with a caret at every offset in `carets`, the last one primary.
    fn multi(text: &str, carets: &[usize]) -> TextEditor {
        let mut e = TextEditor::new(text);
        e.set_cursor(carets[0]);
        for &at in &carets[1..] {
            e.cursor.push_primary(Selection::caret(at), None);
        }
        e
    }

    fn carets(e: &TextEditor) -> Vec<usize> {
        e.selections().iter().map(|s| s.head).collect()
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
    fn backspace_tab_stop_follows_the_indent_width() {
        let mut e = ed("      x", 6);
        e.buffer.set_indent(Indent::spaces(2));
        e.backspace();
        assert_eq!(e.text(), "    x");
        e.backspace();
        assert_eq!(e.text(), "  x");
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

    // ---- indentation settings -------------------------------------------

    #[test]
    fn tab_with_tabs_inserts_a_tab_and_indents_with_tabs() {
        let mut e = ed("a\nb", 0);
        e.buffer.set_indent(Indent::tabs(4));
        e.tab();
        assert_eq!(e.text(), "\ta\nb");
        assert_eq!(e.cursor(), 1);
        e.select_all();
        e.tab();
        assert_eq!(e.text(), "\t\ta\n\tb");
        e.outdent();
        assert_eq!(e.text(), "\ta\nb");
        // Outdent takes up to a level of spaces too.
        let mut s = TextEditor::new("      x");
        s.buffer.set_indent(Indent::tabs(4));
        s.select_all();
        s.outdent();
        assert_eq!(s.text(), "  x");
    }

    #[test]
    fn tab_with_spaces_lands_on_the_next_stop_of_the_width() {
        let mut e = ed("ab", 2);
        e.buffer.set_indent(Indent::spaces(2));
        e.tab();
        assert_eq!(e.text(), "ab  ");
        e.set_cursor(1);
        e.tab();
        assert_eq!(e.text(), "a b  ");
        // A tab already on the line counts to the next stop.
        let mut t = ed("\tx", 1);
        t.tab();
        assert_eq!(t.text(), "\t    x");
    }

    #[test]
    fn indenter_newline_between_braces() {
        let mut e = ed("  f() {}", 7);
        e.newline_with(true);
        assert_eq!(e.text(), "  f() {\n      \n  }");
        assert_eq!(e.cursor_point(), Point::new(1, 6));
        // After an opener with nothing to close, just one level deeper.
        let mut o = ed("x = [", 5);
        o.buffer.set_indent(Indent::tabs(4));
        o.newline_with(true);
        assert_eq!(o.text(), "x = [\n\t");
        // Without the indenter the line keeps its indent only.
        let mut p = ed("  {}", 3);
        p.newline_with(false);
        assert_eq!(p.text(), "  {\n  }");
    }

    #[test]
    fn indenter_reindents_a_closing_brace() {
        let mut e = ed("  if x {\n    y;\n        ", 24);
        e.type_char('}', true, true);
        assert_eq!(e.text(), "  if x {\n    y;\n  }");
        assert_eq!(e.cursor_point(), Point::new(2, 3));
        // Not on a line that already holds text before the caret.
        let mut t = ed("{\n  y ", 6);
        t.type_char('}', false, true);
        assert_eq!(t.text(), "{\n  y }");
        // No opener to match: plain typing.
        let mut n = ed("    ", 4);
        n.type_char('}', false, true);
        assert_eq!(n.text(), "    }");
    }

    // ---- auto-pairs -----------------------------------------------------

    #[test]
    fn auto_pairs_insert_step_over_wrap_and_delete_together() {
        let mut e = ed("", 0);
        e.type_char('(', true, false);
        assert_eq!(e.text(), "()");
        assert_eq!(e.cursor(), 1);
        e.type_char('"', true, false);
        assert_eq!(e.text(), "(\"\")");
        assert_eq!(e.cursor(), 2);
        e.type_char('"', true, false);
        assert_eq!(e.text(), "(\"\")", "stepped over the closing quote");
        assert_eq!(e.cursor(), 3);
        e.type_char(')', true, false);
        assert_eq!(e.text(), "(\"\")", "stepped over the closing bracket");
        assert_eq!(e.cursor(), 4);
        // Backspace between the halves takes both.
        e.set_cursor(2);
        e.backspace_with(true);
        assert_eq!(e.text(), "()");
        e.backspace_with(true);
        assert_eq!(e.text(), "");
        let mut b = ed("()", 1);
        b.backspace_with(false);
        assert_eq!(b.text(), ")", "without pairs, one char");
        // Wrapping a selection keeps it selected inside.
        let mut w = TextEditor::new("ab cd");
        w.select(0, 2);
        w.type_char('[', true, false);
        assert_eq!(w.text(), "[ab] cd");
        assert_eq!(w.selection(), Selection { anchor: 1, head: 3 });
        // No pair before a word, no quote pair after one.
        let mut n = ed("x", 0);
        n.type_char('(', true, false);
        assert_eq!(n.text(), "(x");
        let mut q = ed("x", 1);
        q.type_char('\'', true, false);
        assert_eq!(q.text(), "x'");
        // Plain characters keep grouping as typing.
        let mut g = ed("", 0);
        g.type_char('a', true, true);
        g.type_char('b', true, true);
        g.undo();
        assert_eq!(g.text(), "");
    }

    // ---- multiple carets ------------------------------------------------

    #[test]
    fn multi_caret_typing_backspace_and_newline_are_one_undo_step_each() {
        let mut e = multi("a\nb\nc", &[1, 3, 5]);
        e.insert("x");
        e.insert("y");
        assert_eq!(e.text(), "axy\nbxy\ncxy");
        assert_eq!(carets(&e), vec![3, 7, 11]);
        e.backspace();
        assert_eq!(e.text(), "ax\nbx\ncx");
        assert_eq!(carets(&e), vec![2, 5, 8]);
        e.newline();
        assert_eq!(e.text(), "ax\n\nbx\n\ncx\n");
        assert_eq!(carets(&e), vec![3, 7, 11]);
        e.undo();
        assert_eq!(e.text(), "ax\nbx\ncx");
        assert_eq!(carets(&e), vec![2, 5, 8], "undo restores every caret");
        e.undo();
        assert_eq!(e.text(), "axy\nbxy\ncxy");
        e.undo();
        assert_eq!(e.text(), "a\nb\nc", "the grouped typing undoes in one go");
        assert_eq!(carets(&e), vec![1, 3, 5]);
        e.redo();
        assert_eq!(e.text(), "axy\nbxy\ncxy");
        assert_eq!(carets(&e), vec![3, 7, 11]);
        assert!(e.cursor.has_multiple());
        e.cursor.clear_extra();
        assert_eq!(e.selections().len(), 1);
    }

    #[test]
    fn carets_that_meet_merge() {
        let mut e = multi("ab", &[0, 1]);
        e.move_right(false);
        assert_eq!(carets(&e), vec![1, 2]);
        e.move_right(false);
        assert_eq!(carets(&e), vec![2], "both reached the end and became one");
        assert!(!e.cursor.has_multiple());
        let mut d = multi("abc", &[1, 2]);
        d.delete();
        assert_eq!(d.text(), "a");
        assert_eq!(carets(&d), vec![1]);
    }

    #[test]
    fn add_caret_above_and_below_keep_the_goal_column() {
        let mut e = ed("abcdef\nx\nabcdef\nabc", 5);
        e.add_caret_below();
        assert_eq!(carets(&e), vec![5, 8]);
        e.add_caret_below();
        assert_eq!(
            carets(&e),
            vec![5, 8, 14],
            "back at column 5 past the short line"
        );
        e.add_caret_below();
        assert_eq!(carets(&e), vec![5, 8, 14, 19]);
        e.add_caret_below();
        assert_eq!(carets(&e), vec![5, 8, 14, 19], "nothing below the last row");
        assert_eq!(e.cursor(), 19, "the new caret is the primary");
        let mut u = ed("abcdef\nx\nabcdef", 14);
        u.add_caret_above();
        u.add_caret_above();
        assert_eq!(carets(&u), vec![5, 8, 14]);
        assert_eq!(u.cursor(), 5);
        u.insert("!");
        assert_eq!(u.text(), "abcde!f\nx!\nabcde!f");
    }

    #[test]
    fn carets_to_ends_of_selected_lines() {
        let mut e = TextEditor::new("ab\ncde\nf\ng");
        e.select(1, 7); // "b\ncde\n": the last row is touched at column 0 only
        e.add_carets_to_ends_of_selected_lines();
        assert_eq!(carets(&e), vec![2, 6]);
        assert!(e.selections().iter().all(Selection::is_empty));
        e.insert(";");
        assert_eq!(e.text(), "ab;\ncde;\nf\ng");
        let mut one = ed("ab\ncd", 0);
        one.add_carets_to_ends_of_selected_lines();
        assert_eq!(carets(&one), vec![2]);
    }

    #[test]
    fn select_next_occurrence_expands_then_wraps() {
        let mut e = ed("foo bar foo foobar foo", 9);
        e.select_next_occurrence();
        assert_eq!(
            e.selection(),
            Selection {
                anchor: 8,
                head: 11
            }
        );
        e.select_next_occurrence();
        assert_eq!(
            e.selections(),
            vec![Selection::forward(8..11), Selection::forward(19..22)]
        );
        e.select_next_occurrence();
        assert_eq!(
            e.selections(),
            vec![
                Selection::forward(0..3),
                Selection::forward(8..11),
                Selection::forward(19..22)
            ],
            "wrapped round, skipping foobar: whole words from a caret"
        );
        assert_eq!(
            e.selection(),
            Selection::forward(0..3),
            "the newest is the primary"
        );
        e.select_next_occurrence();
        assert_eq!(e.selections().len(), 3, "all selected: nothing to add");
        e.insert("x");
        assert_eq!(e.text(), "x bar x foobar x");
        // A selection made by hand matches anywhere, case-sensitively.
        let mut h = TextEditor::new("ab AB cab");
        h.select(0, 2);
        h.select_next_occurrence();
        assert_eq!(
            h.selections(),
            vec![Selection::forward(0..2), Selection::forward(7..9)]
        );
    }

    #[test]
    fn select_all_occurrences_and_unselect() {
        let mut e = ed("a b a b a", 4);
        e.select_all_occurrences();
        assert_eq!(
            e.selections(),
            vec![
                Selection::forward(0..1),
                Selection::forward(4..5),
                Selection::forward(8..9)
            ]
        );
        assert_eq!(
            e.selection(),
            Selection::forward(4..5),
            "the primary is the one at the caret"
        );
        let mut n = ed("a b a b a", 0);
        n.select_next_occurrence();
        n.select_next_occurrence();
        n.select_next_occurrence();
        assert_eq!(n.selections().len(), 3);
        n.unselect_occurrence();
        assert_eq!(
            n.selections(),
            vec![Selection::forward(0..1), Selection::forward(4..5)]
        );
        assert_eq!(n.selection(), Selection::forward(4..5));
        n.unselect_occurrence();
        n.unselect_occurrence();
        assert_eq!(
            n.selections(),
            vec![Selection::forward(0..1)],
            "the last one stays"
        );
    }

    #[test]
    fn expand_to_word_by_humps_then_the_identifier() {
        let mut e = ed("let fooBar_baz = 1;", 8); // inside "Bar"
        assert!(e.expand_to_word(true));
        assert_eq!(e.selected_text(), "Bar");
        assert!(e.expand_to_word(true));
        assert_eq!(e.selected_text(), "fooBar_baz");
        assert!(!e.expand_to_word(true), "nothing more to expand to");
        let mut s = ed("let fooBar_baz = 1;", 12);
        assert!(s.expand_to_word(true));
        assert_eq!(s.selected_text(), "baz");
        let mut w = ed("let fooBar_baz = 1;", 5);
        assert!(w.expand_to_word(false));
        assert_eq!(w.selected_text(), "fooBar_baz");
        let mut none = ed("a + b", 2);
        assert!(!none.expand_to_word(true));
        assert_eq!(
            humps(&"HTTPServer2x".chars().collect::<Vec<_>>()),
            vec![0..4, 4..12]
        );
    }

    #[test]
    fn toggle_case_on_selections_and_the_word_at_a_caret() {
        let mut e = TextEditor::new("Hello world");
        e.select_all();
        e.toggle_case();
        assert_eq!(e.text(), "HELLO WORLD");
        assert_eq!(e.selection(), Selection::forward(0..11));
        e.toggle_case();
        assert_eq!(e.text(), "hello world");
        let mut w = multi("ab cd", &[1, 4]);
        w.toggle_case();
        assert_eq!(w.text(), "AB CD");
        assert_eq!(carets(&w), vec![1, 4]);
        w.undo();
        assert_eq!(w.text(), "ab cd");
    }

    #[test]
    fn block_comment_on_and_off() {
        let mut e = TextEditor::new("let x;");
        e.select(4, 5);
        e.toggle_block_comment("/*", "*/");
        assert_eq!(e.text(), "let /* x */;");
        assert_eq!(e.selection(), Selection::forward(4..11));
        e.toggle_block_comment("/*", "*/");
        assert_eq!(e.text(), "let x;");
        assert_eq!(e.selection(), Selection::forward(4..5));
        let mut l = ed("  a;", 3);
        l.toggle_block_comment("/*", "*/");
        assert_eq!(l.text(), "  /* a; */");
        assert_eq!(l.cursor(), 6);
        l.toggle_block_comment("/*", "*/");
        assert_eq!(l.text(), "  a;");
        assert_eq!(l.cursor(), 3);
        let mut m = multi("a\nb", &[0, 2]);
        m.toggle_block_comment("<!--", "-->");
        assert_eq!(m.text(), "<!-- a -->\n<!-- b -->");
        m.undo();
        assert_eq!(m.text(), "a\nb");
    }

    #[test]
    fn paste_gives_each_caret_its_line() {
        let mut e = multi("a\nb\nc", &[1, 3, 5]);
        e.paste("1\n2\n3\n");
        assert_eq!(e.text(), "a1\nb2\nc3");
        e.undo();
        assert_eq!(e.text(), "a\nb\nc");
        e.paste("1\n2");
        assert_eq!(
            e.text(),
            "a1\n2\nb1\n2\nc1\n2",
            "a line count that differs goes everywhere"
        );
        let mut one = ed("", 0);
        one.paste("x\n");
        assert_eq!(one.text(), "x\n");
    }

    #[test]
    fn line_commands_with_multiple_carets() {
        let mut d = multi("a\nb\nc\nd", &[0, 4]);
        d.duplicate_line();
        assert_eq!(d.text(), "a\na\nb\nc\nc\nd");
        assert_eq!(carets(&d), vec![2, 8]);
        d.delete_line();
        assert_eq!(d.text(), "a\nb\nc\nd");
        assert_eq!(
            carets(&d),
            vec![2, 6],
            "on the rows that took the deleted rows' place"
        );
        let mut same = multi("a\nb", &[0, 1]);
        same.delete_line();
        assert_eq!(same.text(), "b");
        assert_eq!(
            carets(&same),
            vec![0, 1],
            "two carets on one row delete it once, each keeping its column"
        );
        let mut m = multi("a\nb\nc\nd", &[0, 4]);
        m.move_line(false);
        assert_eq!(m.text(), "b\na\nd\nc");
        assert_eq!(carets(&m), vec![2, 6]);
        let mut adjacent = multi("a\nb\nc", &[0, 2]);
        adjacent.move_line(false);
        assert_eq!(adjacent.text(), "c\na\nb", "adjacent lines move as a block");
        let mut c = multi("x\n  y\n  z", &[0, 8]);
        c.toggle_line_comment("# ");
        assert_eq!(
            c.text(),
            "# x\n  y\n#   z",
            "at the shallowest indent of the rows"
        );
        assert_eq!(carets(&c), vec![2, 12]);
        c.toggle_line_comment("# ");
        assert_eq!(c.text(), "x\n  y\n  z");
        assert_eq!(carets(&c), vec![0, 8]);
        let mut t = multi("a\nb", &[0, 2]);
        t.tab();
        assert_eq!(t.text(), "    a\n    b");
        t.outdent();
        assert_eq!(t.text(), "a\nb");
    }

    #[test]
    fn movement_moves_every_caret_and_a_click_drops_the_extras() {
        let mut e = multi("abc\ndef", &[0, 4]);
        e.move_line_end(true);
        assert_eq!(
            e.selections(),
            vec![Selection::forward(0..3), Selection::forward(4..7)]
        );
        e.move_word_left(false);
        assert_eq!(carets(&e), vec![0, 4]);
        e.move_down(false);
        assert_eq!(carets(&e), vec![4, 7]);
        e.set_cursor(1);
        assert_eq!(carets(&e), vec![1]);
        let mut g = multi("abc\ndef", &[0, 4]);
        g.move_doc_end(false);
        assert_eq!(carets(&g), vec![7]);
        let mut p = multi("abc\ndef", &[0, 4]);
        p.goto(1, 99);
        assert_eq!(carets(&p), vec![7]);
    }

    #[test]
    fn set_selections_replaces_them_all() {
        let mut e = multi("abcdef", &[0, 3]);
        e.set_selections(
            vec![
                Selection::forward(4..99),
                Selection::caret(1),
                Selection::caret(1),
            ],
            1,
        );
        assert_eq!(
            e.selections(),
            vec![Selection::caret(1), Selection::forward(4..6)]
        );
        assert_eq!(
            e.selection(),
            Selection::caret(1),
            "the chosen one is the primary"
        );
        e.insert("x");
        assert_eq!(e.text(), "axbcdx");
        let mut g = ed("", 0);
        g.insert("q");
        g.set_selections(vec![Selection::caret(1)], 0);
        g.insert("r");
        g.undo();
        assert_eq!(g.text(), "q", "a new typing group after set_selections");
    }

    #[test]
    fn alt_click_adds_and_removes_carets() {
        let mut e = ed("abcdef", 0);
        e.add_caret_at(3);
        e.add_caret_at(5);
        assert_eq!(carets(&e), vec![0, 3, 5]);
        assert_eq!(e.cursor(), 5);
        e.add_caret_at(3);
        assert_eq!(carets(&e), vec![0, 5], "a click on a caret removes it");
        e.add_caret_at(5);
        assert_eq!(
            carets(&e),
            vec![0],
            "even the primary; the previous one takes over"
        );
        e.add_caret_at(0);
        assert_eq!(carets(&e), vec![0], "the last caret stays");
    }

    #[test]
    fn replace_ranges_is_one_undo_step() {
        let mut e = multi("a-b-c-d", &[0, 7]);
        e.replace_ranges(vec![
            (5..6, "::".to_string()),
            (1..2, "::".to_string()),
            (3..4, String::new()),
        ]);
        assert_eq!(e.text(), "a::bc::d");
        assert_eq!(
            e.selections(),
            vec![Selection::caret(7)],
            "one caret after the last replacement"
        );
        e.undo();
        assert_eq!(e.text(), "a-b-c-d");
        assert_eq!(carets(&e), vec![0, 7]);
        e.redo();
        assert_eq!(e.text(), "a::bc::d");
        assert_eq!(carets(&e), vec![7]);
    }

    #[test]
    fn trim_on_save_then_undo_through_a_cursor() {
        let mut e = multi("a  \nb  ", &[1, 5]);
        let rows: Vec<usize> = e
            .selections()
            .iter()
            .map(|s| e.buffer.offset_to_point(s.head).row)
            .collect();
        assert!(e.buffer.trim_trailing_whitespace(&rows[..1]));
        assert_eq!(e.text(), "a  \nb");
        e.cursor.sync(&e.buffer);
        assert_eq!(carets(&e), vec![1, 5]);
        e.undo();
        assert_eq!(e.text(), "a  \nb  ");
        assert_eq!(
            carets(&e),
            vec![1, 5],
            "a step without carets keeps this cursor's"
        );
    }

    #[test]
    fn extras_follow_another_cursors_edit() {
        let mut buffer = Buffer::new("one two three");
        let mut a = Cursor::new(&buffer);
        let mut b = Cursor::new(&buffer);
        b.set_cursor(&buffer, 4);
        b.push_primary(Selection::forward(8..13), None);
        a.set_cursor(&buffer, 0);
        a.insert(&mut buffer, "zero ");
        b.sync(&buffer);
        assert_eq!(
            b.selections(),
            vec![Selection::caret(9), Selection::forward(13..18)]
        );
        assert_eq!(b.selected_text(&buffer), "three");
        // A deletion across both pulls them together into one.
        a.replace_range(&mut buffer, 8..15, "");
        b.sync(&buffer);
        assert_eq!(b.selections(), vec![Selection::forward(8..11)]);
        assert!(!b.has_multiple());
    }
}
