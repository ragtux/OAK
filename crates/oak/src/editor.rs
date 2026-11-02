//! The code editor: an [`EditorView`] entity holding a cursor and a scroll
//! position on a shared [`Document`], and the [`EditorElement`] that lays its
//! visible lines out and paints them, Fleet-style: a gutter of right-aligned
//! line numbers, the current line washed across the whole width, the caret
//! two pixels wide, a thin scrollbar on the right. Two views on one document
//! (a file in two splits) edit the same buffer; each keeps its own cursor,
//! which follows the other's edits.
//!
//! Text is painted a line at a time through gpui's shaper. Only the rows
//! in view are shaped each frame, so the cost is the viewport, not the file.

use crate::actions::*;
use crate::document::{Document, DocumentEvent};
use crate::settings::AppSettings;
use crate::theme::{metrics::EDITOR_OVERSCROLL_LINES, Metrics, Theme};
use gpui::{
    div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, ContentMask,
    Context, Corners, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font, FontFeatures, FontStyle,
    FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent,
    ShapedLine, SharedString, Style, TextAlign, TextRun, UTF16Selection, Window,
};
use oak_syntax::Language;
use oak_text::{Buffer, Cursor, Point as TextPoint, Selection};
use std::ops::Range;
use std::path::PathBuf;

/// What the view tells the workspace: the document changed under it (so a
/// preview tab is promoted) or was saved.
#[derive(Clone, Debug)]
pub enum EditorEvent {
    Changed,
    Saved,
}

/// The editor's sizes at the current zoom and settings: `editor.fontSize`,
/// `editor.lineSpacing` and the measured gutter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics {
    pub font_size: f32,
    pub line_height: f32,
    pub gutter_width: f32,
    pub gutter_number_right: f32,
    pub caret_width: f32,
    pub scrollbar_thumb: f32,
    pub scrollbar_thumb_min: f32,
}

impl TextMetrics {
    pub fn current(cx: &App) -> TextMetrics {
        let m = cx.global::<Metrics>();
        let (font_size, line_height) = m.editor_text(&cx.global::<AppSettings>().settings);
        TextMetrics {
            font_size,
            line_height,
            gutter_width: m.gutter_width,
            gutter_number_right: m.gutter_number_right,
            caret_width: m.caret_width,
            scrollbar_thumb: m.scrollbar_thumb,
            scrollbar_thumb_min: m.scrollbar_thumb_min,
        }
    }
}

pub struct EditorView {
    pub document: Entity<Document>,
    /// This view's caret and selection on the document.
    pub cursor: Cursor,
    /// Scroll offsets in pixels.
    scroll_y: Pixels,
    scroll_x: Pixels,
    /// The bounds the text area last painted into, for page sizes and hit tests.
    last_bounds: Bounds<Pixels>,
    /// The sizes the last frame painted with (zoom and settings).
    tm: TextMetrics,
    focus_handle: FocusHandle,
    dragging: bool,
    marked: Option<Range<usize>>,
}

impl EventEmitter<EditorEvent> for EditorView {}

impl Focusable for EditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn bind_keys(cx: &mut App) {
    // Fallbacks for the keys every editor has; the Fleet keymap in
    // `keymap.rs` binds the rest and may override these.
    use gpui::KeyBinding;
    let ctx = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, ctx),
        KeyBinding::new("down", MoveDown, ctx),
        KeyBinding::new("left", MoveLeft, ctx),
        KeyBinding::new("right", MoveRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("shift-home", SelectHome, ctx),
        KeyBinding::new("shift-end", SelectEnd, ctx),
        KeyBinding::new("pageup", PageUp, ctx),
        KeyBinding::new("pagedown", PageDown, ctx),
        KeyBinding::new("shift-pageup", SelectPageUp, ctx),
        KeyBinding::new("shift-pagedown", SelectPageDown, ctx),
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("shift-backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("tab", Tab, ctx),
        KeyBinding::new("shift-tab", Outdent, ctx),
        KeyBinding::new("escape", Cancel, ctx),
    ]);
}

impl EditorView {
    pub fn new(document: Entity<Document>, cx: &mut Context<Self>) -> Self {
        let cursor = Cursor::new(&document.read(cx).buffer);
        // Any view's edit repaints this one, and tells the workspace.
        cx.subscribe(&document, |this, _doc, event: &DocumentEvent, cx| {
            cx.emit(match event {
                DocumentEvent::Changed => EditorEvent::Changed,
                DocumentEvent::Saved => EditorEvent::Saved,
            });
            let _ = this;
            cx.notify();
        })
        .detach();
        EditorView {
            document,
            cursor,
            scroll_y: px(0.),
            scroll_x: px(0.),
            last_bounds: Bounds::default(),
            tm: TextMetrics::current(cx),
            focus_handle: cx.focus_handle(),
            dragging: false,
            marked: None,
        }
    }

    pub fn path(&self, cx: &App) -> Option<PathBuf> {
        self.document.read(cx).path.clone()
    }

    pub fn title(&self, cx: &App) -> String {
        self.document.read(cx).title()
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        self.document.read(cx).is_dirty()
    }

    pub fn language(&self, cx: &App) -> Option<Language> {
        self.document.read(cx).language
    }

    pub fn uses_crlf(&self, cx: &App) -> bool {
        self.document.read(cx).buffer.uses_crlf()
    }

    /// The caret's row and column, after catching up with other views' edits.
    pub fn cursor_position(&mut self, cx: &App) -> TextPoint {
        let doc = self.document.read(cx);
        self.cursor.sync(&doc.buffer);
        self.cursor.cursor_point(&doc.buffer)
    }

    pub fn save(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.document.update(cx, |doc, cx| doc.save(cx))
    }

    /// Runs a command on this view's cursor and the shared buffer; the
    /// document tells every view, this one included, to repaint.
    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Cursor, &mut Buffer)) {
        let cursor = &mut self.cursor;
        self.document.update(cx, |doc, cx| {
            cursor.sync(&doc.buffer);
            doc.edit(cx, |buffer| f(cursor, buffer));
        });
    }

    /// Moves this view's cursor; nothing in the buffer changes.
    fn go(&mut self, cx: &App, f: impl FnOnce(&mut Cursor, &Buffer)) {
        let doc = self.document.read(cx);
        self.cursor.sync(&doc.buffer);
        f(&mut self.cursor, &doc.buffer);
    }

    fn with_buffer<R>(&mut self, cx: &App, f: impl FnOnce(&Cursor, &Buffer) -> R) -> R {
        let doc = self.document.read(cx);
        self.cursor.sync(&doc.buffer);
        f(&self.cursor, &doc.buffer)
    }

    fn line_height(&self) -> Pixels {
        px(self.tm.line_height)
    }

    fn rows_per_page(&self) -> usize {
        let h = self.last_bounds.size.height;
        ((h / self.line_height()).floor() as usize)
            .saturating_sub(1)
            .max(1)
    }

    /// Scrolls just enough for the caret's row to be on screen.
    fn scroll_to_cursor(&mut self, cx: &App) {
        let lh = self.line_height();
        let row = self.cursor_position(cx).row as f32;
        let top = lh * row;
        let bottom = top + lh;
        let height = self.last_bounds.size.height;
        if height <= px(0.) {
            return;
        }
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + height {
            self.scroll_y = bottom - height;
        }
    }

    /// The scrollable extent: the lines plus Fleet's overscroll past the end.
    fn content_height(&self, line_count: usize) -> Pixels {
        self.line_height() * (line_count as f32 + EDITOR_OVERSCROLL_LINES)
    }

    fn max_scroll_y(&self, cx: &App) -> Pixels {
        let lines = self.document.read(cx).buffer.line_count();
        (self.content_height(lines) - self.last_bounds.size.height).max(px(0.))
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.scroll_to_cursor(cx);
        cx.notify();
    }

    fn after_move(&mut self, cx: &mut Context<Self>) {
        self.scroll_to_cursor(cx);
        cx.notify();
    }

    // ---- actions --------------------------------------------------------

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_up(b, false));
        self.after_move(cx);
    }
    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_down(b, false));
        self.after_move(cx);
    }
    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_left(b, false));
        self.after_move(cx);
    }
    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_right(b, false));
        self.after_move(cx);
    }
    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_up(b, true));
        self.after_move(cx);
    }
    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_down(b, true));
        self.after_move(cx);
    }
    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_left(b, true));
        self.after_move(cx);
    }
    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_right(b, true));
        self.after_move(cx);
    }
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_line_start(b, false));
        self.after_move(cx);
    }
    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_line_end(b, false));
        self.after_move(cx);
    }
    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_line_start(b, true));
        self.after_move(cx);
    }
    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_line_end(b, true));
        self.after_move(cx);
    }
    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows_per_page();
        self.go(cx, |c, b| c.move_page(b, rows, true, false));
        self.scroll_y = (self.scroll_y - self.line_height() * rows as f32).max(px(0.));
        self.after_move(cx);
    }
    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows_per_page();
        self.go(cx, |c, b| c.move_page(b, rows, false, false));
        let max = self.max_scroll_y(cx);
        self.scroll_y = (self.scroll_y + self.line_height() * rows as f32).min(max);
        self.after_move(cx);
    }
    fn select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows_per_page();
        self.go(cx, |c, b| c.move_page(b, rows, true, true));
        self.after_move(cx);
    }
    fn select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows_per_page();
        self.go(cx, |c, b| c.move_page(b, rows, false, true));
        self.after_move(cx);
    }
    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_doc_start(b, false));
        self.after_move(cx);
    }
    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_doc_end(b, false));
        self.after_move(cx);
    }
    fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_doc_start(b, true));
        self.after_move(cx);
    }
    fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_doc_end(b, true));
        self.after_move(cx);
    }
    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_word_left(b, false));
        self.after_move(cx);
    }
    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_word_right(b, false));
        self.after_move(cx);
    }
    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_word_left(b, true));
        self.after_move(cx);
    }
    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_word_right(b, true));
        self.after_move(cx);
    }
    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.backspace(b));
        self.after_edit(cx);
    }
    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.delete(b));
        self.after_edit(cx);
    }
    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |c, b| c.delete_word_backward(b));
        self.after_edit(cx);
    }
    fn delete_word_forward(
        &mut self,
        _: &DeleteWordForward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |c, b| c.delete_word_forward(b));
        self.after_edit(cx);
    }
    fn delete_line(&mut self, _: &DeleteLine, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.delete_line(b));
        self.after_edit(cx);
    }
    fn duplicate_line(&mut self, _: &DuplicateLine, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.duplicate_line(b));
        self.after_edit(cx);
    }
    fn move_line_up(&mut self, _: &MoveLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.move_line(b, true));
        self.after_edit(cx);
    }
    fn move_line_down(&mut self, _: &MoveLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.move_line(b, false));
        self.after_edit(cx);
    }
    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.newline(b));
        self.after_edit(cx);
    }
    fn newline_below(&mut self, _: &NewlineBelow, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| {
            c.move_line_end(b, false);
            c.newline(b);
        });
        self.after_edit(cx);
    }
    fn newline_above(&mut self, _: &NewlineAbove, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| {
            if c.cursor_point(b).row == 0 {
                c.set_cursor(b, 0);
                c.newline(b);
                c.move_up(b, false);
            } else {
                c.move_up(b, false);
                c.move_line_end(b, false);
                c.newline(b);
            }
        });
        self.after_edit(cx);
    }
    fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.tab(b));
        self.after_edit(cx);
    }
    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.outdent(b));
        self.after_edit(cx);
    }
    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.undo(b));
        self.after_edit(cx);
    }
    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.redo(b));
        self.after_edit(cx);
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_or_line(cx);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_or_line(cx);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.edit(cx, |c, b| {
            if c.selection.is_empty() {
                c.delete_line(b);
            } else {
                c.delete(b);
            }
        });
        self.after_edit(cx);
    }
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text.replace("\r\n", "\n");
            self.edit(cx, |c, b| {
                c.commit();
                c.insert(b, &text);
                c.commit();
            });
            self.after_edit(cx);
        }
    }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.select_all(b));
        cx.notify();
    }
    fn comment_line(&mut self, _: &CommentLine, _: &mut Window, cx: &mut Context<Self>) {
        let Some(prefix) = self.language(cx).and_then(Language::line_comment) else {
            return;
        };
        self.edit(cx, |c, b| c.toggle_line_comment(b, prefix));
        self.after_edit(cx);
    }
    fn join_lines(&mut self, _: &JoinLines, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| {
            let row = c.cursor_point(b).row;
            if row + 1 >= b.line_count() {
                return;
            }
            let end = b.line_to_char(row) + b.line_len(row);
            let next = b.line(row + 1);
            let lead = next.chars().take_while(|ch| ch.is_whitespace()).count();
            let glue = if next.trim().is_empty() { "" } else { " " };
            c.replace_range(b, end..end + 1 + lead, glue);
        });
        self.after_edit(cx);
    }
    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| {
            let head = c.selection.head;
            c.set_cursor(b, head);
        });
        cx.notify();
    }

    /// The selection, or the whole current line when there is none (what
    /// Fleet copies on a bare Ctrl+C).
    fn selected_or_line(&mut self, cx: &App) -> String {
        self.with_buffer(cx, |c, b| {
            if c.selection.is_empty() {
                let row = c.cursor_point(b).row;
                let mut line = b.line(row);
                line.push('\n');
                line
            } else {
                c.selected_text(b)
            }
        })
    }

    // ---- mouse ----------------------------------------------------------

    fn offset_at(&self, position: Point<Pixels>, window: &mut Window, cx: &App) -> usize {
        let buffer = &self.document.read(cx).buffer;
        let bounds = self.last_bounds;
        let lh = self.line_height();
        let y = position.y - bounds.origin.y + self.scroll_y;
        let row = ((y / lh).floor().max(0.) as usize).min(buffer.line_count() - 1);
        let x = position.x - bounds.origin.x - px(self.tm.gutter_width) + self.scroll_x;
        let theme = cx.global::<Theme>();
        let line = DisplayLine::new(&buffer.line(row));
        let shaped = shape_display_line(&line, &[], theme, self.tm.font_size, window);
        let disp_byte = shaped.closest_index_for_x(x.max(px(0.)));
        let column = line.char_for_display_byte(disp_byte);
        buffer.point_to_offset(TextPoint::new(row, column))
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        let offset = self.offset_at(event.position, window, cx);
        let shift = event.modifiers.shift;
        let clicks = event.click_count;
        self.go(cx, |c, b| match clicks {
            2 => {
                let range = c.word_range_at(b, offset);
                c.select(b, range.start, range.end);
            }
            n if n >= 3 => {
                let row = b.offset_to_point(offset).row;
                let range = c.line_range(b, row);
                c.select(b, range.start, range.end);
            }
            _ => {
                if shift {
                    let anchor = c.selection.anchor;
                    c.select(b, anchor, offset);
                } else {
                    c.set_cursor(b, offset);
                }
            }
        });
        if clicks < 2 {
            self.dragging = true;
        }
        cx.notify();
    }

    fn mouse_drag(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        let offset = self.offset_at(event.position, window, cx);
        self.go(cx, |c, b| {
            let anchor = c.selection.anchor;
            c.select(b, anchor, offset);
        });
        cx.notify();
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, cx: &mut Context<Self>) {
        self.dragging = false;
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(self.line_height());
        let max = self.max_scroll_y(cx);
        self.scroll_y = (self.scroll_y - delta.y).clamp(px(0.), max);
        self.scroll_x = (self.scroll_x - delta.x).max(px(0.));
        cx.notify();
    }

    // ---- utf16 bridging for the platform's text input --------------------

    fn char_to_utf16(&self, offset: usize, cx: &App) -> usize {
        let rope = self.document.read(cx).buffer.rope();
        rope.char_to_utf16_cu(offset.min(rope.len_chars()))
    }

    fn utf16_to_char(&self, utf16: usize, cx: &App) -> usize {
        let rope = self.document.read(cx).buffer.rope();
        rope.utf16_cu_to_char(utf16.min(rope.len_utf16_cu()))
    }

    fn selection(&mut self, cx: &App) -> Selection {
        self.with_buffer(cx, |c, _| c.selection)
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let start = self.utf16_to_char(range.start, cx);
        let end = self.utf16_to_char(range.end, cx);
        *adjusted_range = Some(self.char_to_utf16(start, cx)..self.char_to_utf16(end, cx));
        Some(self.document.read(cx).buffer.slice(start..end))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let selection = self.selection(cx);
        let range = selection.range();
        Some(UTF16Selection {
            range: self.char_to_utf16(range.start, cx)..self.char_to_utf16(range.end, cx),
            reversed: selection.head < selection.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked
            .clone()
            .map(|r| self.char_to_utf16(r.start, cx)..self.char_to_utf16(r.end, cx))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(range) = range.or_else(|| {
            self.marked
                .clone()
                .map(|r| self.char_to_utf16(r.start, cx)..self.char_to_utf16(r.end, cx))
        }) {
            let start = self.utf16_to_char(range.start, cx);
            let end = self.utf16_to_char(range.end, cx);
            self.go(cx, |c, b| {
                if (start..end) != c.selection.range() {
                    c.select(b, start, end);
                }
            });
        }
        self.marked = None;
        self.edit(cx, |c, b| c.insert(b, text));
        self.after_edit(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = match range.or_else(|| {
            self.marked
                .clone()
                .map(|r| self.char_to_utf16(r.start, cx)..self.char_to_utf16(r.end, cx))
        }) {
            Some(range) => {
                let start = self.utf16_to_char(range.start, cx);
                let end = self.utf16_to_char(range.end, cx);
                self.go(cx, |c, b| c.select(b, start, end));
                start
            }
            None => self.selection(cx).range().start,
        };
        self.edit(cx, |c, b| c.insert(b, new_text));
        let len = new_text.chars().count();
        self.marked = Some(start..start + len);
        if let Some(sel) = new_selected_range {
            let a = start + new_text[..new_text.len().min(sel.start)].chars().count();
            let e = start + new_text[..new_text.len().min(sel.end)].chars().count();
            self.go(cx, |c, b| c.select(b, a, e));
        }
        self.after_edit(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let offset = self.utf16_to_char(range_utf16.start, cx);
        let doc = self.document.read(cx);
        let at = doc.buffer.offset_to_point(offset);
        let theme = cx.global::<Theme>();
        let line = DisplayLine::new(&doc.buffer.line(at.row));
        let shaped = shape_display_line(&line, &[], theme, self.tm.font_size, window);
        let x = shaped.x_for_index(line.display_byte_for_char(at.column));
        let lh = self.line_height();
        Some(Bounds::new(
            point(
                element_bounds.origin.x + px(self.tm.gutter_width) + x - self.scroll_x,
                element_bounds.origin.y + lh * at.row as f32 - self.scroll_y,
            ),
            size(px(self.tm.caret_width), lh),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_at(point, window, cx);
        Some(self.char_to_utf16(offset, cx))
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Editor")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::delete_word_forward))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(Self::duplicate_line))
            .on_action(cx.listener(Self::move_line_up))
            .on_action(cx.listener(Self::move_line_down))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::newline_below))
            .on_action(cx.listener(Self::newline_above))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::comment_line))
            .on_action(cx.listener(Self::join_lines))
            .on_action(cx.listener(Self::cancel))
            .size_full()
            .child(EditorElement {
                editor: cx.entity(),
            })
    }
}

// ---- line display ---------------------------------------------------------

/// A source line made ready for the shaper: tabs expanded to spaces, with
/// the maps between source chars and display bytes both ways.
struct DisplayLine {
    text: String,
    /// Display byte offset of each source char, plus one past the end.
    char_to_display: Vec<usize>,
    /// Source byte offset of each source char, plus one past the end.
    char_to_source: Vec<usize>,
}

impl DisplayLine {
    fn new(source: &str) -> Self {
        let mut text = String::with_capacity(source.len());
        let mut char_to_display = Vec::with_capacity(source.len() + 1);
        let mut char_to_source = Vec::with_capacity(source.len() + 1);
        let mut column = 0;
        for (byte, c) in source.char_indices() {
            char_to_display.push(text.len());
            char_to_source.push(byte);
            if c == '\t' {
                let width = oak_text::TAB_WIDTH - column % oak_text::TAB_WIDTH;
                text.extend(std::iter::repeat_n(' ', width));
                column += width;
            } else {
                text.push(c);
                column += 1;
            }
        }
        char_to_display.push(text.len());
        char_to_source.push(source.len());
        DisplayLine {
            text,
            char_to_display,
            char_to_source,
        }
    }

    fn display_byte_for_char(&self, column: usize) -> usize {
        self.char_to_display[column.min(self.char_to_display.len() - 1)]
    }

    fn char_for_display_byte(&self, byte: usize) -> usize {
        match self.char_to_display.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    fn display_byte_for_source_byte(&self, source_byte: usize) -> usize {
        match self.char_to_source.binary_search(&source_byte) {
            Ok(i) => self.char_to_display[i],
            Err(i) => self.char_to_display[i.min(self.char_to_display.len() - 1)],
        }
    }
}

fn editor_font(weight: FontWeight, italic: bool, theme: &Theme) -> Font {
    Font {
        family: theme.mono_font.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight,
        style: if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    }
}

/// Shapes one display line. `spans` are the highlight spans that overlap
/// the line, in source bytes relative to the line's start.
fn shape_display_line(
    line: &DisplayLine,
    spans: &[(Range<usize>, &'static str)],
    theme: &Theme,
    font_size: f32,
    window: &mut Window,
) -> ShapedLine {
    let plain = editor_font(FontWeight::NORMAL, false, theme);
    let mut runs: Vec<TextRun> = Vec::new();
    let mut cursor = 0;
    let push =
        |runs: &mut Vec<TextRun>, len: usize, font: Font, color: Hsla, underline: Option<Hsla>| {
            if len == 0 {
                return;
            }
            runs.push(TextRun {
                len,
                font,
                color,
                background_color: None,
                underline: underline.map(|color| gpui::UnderlineStyle {
                    thickness: px(1.),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: None,
            });
        };
    for (range, attribute) in spans {
        let start = line.display_byte_for_source_byte(range.start).max(cursor);
        let end = line
            .display_byte_for_source_byte(range.end)
            .min(line.text.len());
        if end <= start {
            continue;
        }
        push(
            &mut runs,
            start - cursor,
            plain.clone(),
            theme.editor_text,
            None,
        );
        let style = theme.syntax(attribute);
        let color = style.and_then(|s| s.color).unwrap_or(theme.editor_text);
        let font = match style {
            Some(s) => editor_font(s.weight, s.italic, theme),
            None => plain.clone(),
        };
        push(
            &mut runs,
            end - start,
            font,
            color,
            style.and_then(|s| s.underline),
        );
        cursor = end;
    }
    push(
        &mut runs,
        line.text.len() - cursor,
        plain,
        theme.editor_text,
        None,
    );
    if runs.is_empty() {
        runs.push(TextRun {
            len: 0,
            font: editor_font(FontWeight::NORMAL, false, theme),
            color: theme.editor_text,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    window.text_system().shape_line(
        SharedString::from(line.text.clone()),
        px(font_size),
        &runs,
        None,
    )
}

// ---- the element -------------------------------------------------------------

pub struct EditorElement {
    editor: Entity<EditorView>,
}

struct LaidOutRow {
    row: usize,
    line: DisplayLine,
    shaped: ShapedLine,
    number: ShapedLine,
}

pub struct Prepaint {
    hitbox: Hitbox,
    rows: Vec<LaidOutRow>,
    first_row: usize,
    scroll_y: Pixels,
    scroll_x: Pixels,
    cursor: TextPoint,
    selection: Range<usize>,
    focused: bool,
    line_count: usize,
    tm: TextMetrics,
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let theme = cx.global::<Theme>().clone();
        let tm = TextMetrics::current(cx);
        let lh = px(tm.line_height);
        let focused = self.editor.read(cx).focus_handle.is_focused(window);
        let (document, cursor, selection, scroll_y, scroll_x) =
            self.editor.update(cx, |editor, cx| {
                // A zoom or font change keeps the same first line in view.
                if editor.tm != tm {
                    let row = editor.scroll_y / px(editor.tm.line_height);
                    editor.tm = tm;
                    editor.scroll_y = px(tm.line_height) * row;
                }
                editor.last_bounds = bounds;
                let max = editor.max_scroll_y(cx);
                if editor.scroll_y > max {
                    editor.scroll_y = max;
                }
                let (cursor, selection) =
                    editor.with_buffer(cx, |c, b| (c.cursor_point(b), c.selection.range()));
                (
                    editor.document.clone(),
                    cursor,
                    selection,
                    editor.scroll_y,
                    editor.scroll_x,
                )
            });
        let mut rows = Vec::new();
        let (line_count, first_row) = document.update(cx, |doc, _| {
            let line_count = doc.buffer.line_count();
            let first_row = (scroll_y / lh).floor().max(0.) as usize;
            let visible = (bounds.size.height / lh).ceil() as usize + 1;
            let last_row = (first_row + visible).min(line_count);
            let number_font = editor_font(FontWeight::NORMAL, false, &theme);
            rows.reserve(last_row.saturating_sub(first_row));
            // Spans are sorted by start; walk them once for the visible window.
            let first_byte = doc.buffer.line_to_byte(first_row.min(line_count - 1));
            let spans = doc.spans().to_vec();
            let mut span_ix = spans.partition_point(|s| s.range.end <= first_byte);
            for row in first_row..last_row {
                let source = doc.buffer.line(row);
                let line_start = doc.buffer.line_to_byte(row);
                let line_end = line_start + source.len();
                let mut line_spans: Vec<(Range<usize>, &'static str)> = Vec::new();
                let mut i = span_ix;
                while i < spans.len() && spans[i].range.start < line_end {
                    let s = &spans[i];
                    if s.range.end > line_start {
                        let a = s.range.start.max(line_start) - line_start;
                        let b = s.range.end.min(line_end) - line_start;
                        if b > a {
                            line_spans.push((a..b, s.attribute));
                        }
                    }
                    if s.range.end <= line_end {
                        span_ix = i + 1;
                    }
                    i += 1;
                }
                let line = DisplayLine::new(&source);
                let shaped = shape_display_line(&line, &line_spans, &theme, tm.font_size, window);
                let color = if row == cursor.row {
                    theme.line_number_current
                } else {
                    theme.line_number
                };
                let number_text = SharedString::from((row + 1).to_string());
                let number = window.text_system().shape_line(
                    number_text.clone(),
                    px(tm.font_size),
                    &[TextRun {
                        len: number_text.len(),
                        font: number_font.clone(),
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                rows.push(LaidOutRow {
                    row,
                    line,
                    shaped,
                    number,
                });
            }
            (line_count, first_row)
        });
        Prepaint {
            hitbox,
            rows,
            first_row,
            scroll_y,
            scroll_x,
            cursor,
            selection,
            focused,
            line_count,
            tm,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = cx.global::<Theme>().clone();
        let tm = prepaint.tm;
        let lh = px(tm.line_height);
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);

        let text_x = bounds.origin.x + px(tm.gutter_width) - prepaint.scroll_x;
        let number_right = bounds.origin.x + px(tm.gutter_number_right);
        let right = bounds.origin.x + bounds.size.width;
        let buffer_rows: Vec<(usize, usize)> = {
            let document = self.editor.read(cx).document.read(cx);
            prepaint
                .rows
                .iter()
                .map(|r| {
                    let start = document.buffer.line_to_char(r.row);
                    (start, start + document.buffer.line_len(r.row))
                })
                .collect()
        };

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (laid, (line_start, line_end)) in prepaint.rows.iter().zip(buffer_rows) {
                let y = bounds.origin.y + lh * laid.row as f32 - prepaint.scroll_y;
                // The current line's wash, across gutter and text alike.
                if laid.row == prepaint.cursor.row && prepaint.selection.is_empty() {
                    let color = if prepaint.focused {
                        theme.current_line_focused
                    } else {
                        theme.current_line
                    };
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, lh)),
                        color,
                    ));
                }
                // The selection on this row.
                let sel = &prepaint.selection;
                if !sel.is_empty() && sel.start <= line_end && sel.end >= line_start {
                    let a = sel.start.max(line_start) - line_start;
                    let b = sel.end.min(line_end) - line_start;
                    let x0 = laid.shaped.x_for_index(laid.line.display_byte_for_char(a));
                    let mut x1 = laid.shaped.x_for_index(laid.line.display_byte_for_char(b));
                    if sel.end > line_end {
                        // The selection runs past the line break: show it as a stub.
                        x1 += px(tm.font_size * 0.6);
                    }
                    let color = if prepaint.focused {
                        theme.selection_focused
                    } else {
                        theme.selection
                    };
                    window.paint_quad(fill(
                        Bounds::new(point(text_x + x0, y), size((x1 - x0).max(px(0.)), lh)),
                        color,
                    ));
                }
                // Line number, right-aligned in the gutter.
                let _ = laid.number.paint(
                    point(number_right - laid.number.width, y),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                let _ = laid
                    .shaped
                    .paint(point(text_x, y), lh, TextAlign::Left, None, window, cx);
                // The caret.
                if prepaint.focused && laid.row == prepaint.cursor.row {
                    let x = laid
                        .shaped
                        .x_for_index(laid.line.display_byte_for_char(prepaint.cursor.column));
                    window.paint_quad(fill(
                        Bounds::new(point(text_x + x, y), size(px(tm.caret_width), lh)),
                        theme.editor_caret,
                    ));
                }
            }
            // The scrollbar: a thumb proportional to the view, no track. Fleet's thumb
            // starts at the text area's top and sits flush with the island's right edge;
            // the content it measures includes the overscroll past the last line.
            let total = lh * (prepaint.line_count as f32 + EDITOR_OVERSCROLL_LINES);
            if total > bounds.size.height {
                let track_h = bounds.size.height;
                let thumb_h =
                    (track_h * (bounds.size.height / total)).max(px(tm.scrollbar_thumb_min));
                let max_scroll = total - bounds.size.height;
                let thumb_y =
                    bounds.origin.y + (track_h - thumb_h) * (prepaint.scroll_y / max_scroll);
                let x = right - px(tm.scrollbar_thumb);
                window.paint_quad(
                    fill(
                        Bounds::new(point(x, thumb_y), size(px(tm.scrollbar_thumb), thumb_h)),
                        theme.scrollbar_thumb,
                    )
                    .corner_radii(Corners::all(px(tm.scrollbar_thumb / 2.))),
                );
            }
        });

        let _ = prepaint.first_row;
        let hitbox = prepaint.hitbox.clone();
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && hitbox.is_hovered(window)
                && event.button == MouseButton::Left
            {
                editor.update(cx, |editor, cx| editor.mouse_down(event, window, cx));
            }
        });
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.pressed_button == Some(MouseButton::Left) {
                editor.update(cx, |editor, cx| editor.mouse_drag(event, window, cx));
            }
        });
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                editor.update(cx, |editor, cx| editor.mouse_up(event, cx));
            }
        });
        let hitbox = prepaint.hitbox.clone();
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                editor.update(cx, |editor, cx| editor.scroll(event, cx));
            }
        });
    }
}
