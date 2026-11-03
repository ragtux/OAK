//! A single-line text field: the text area of Fleet's `inputField`, used by
//! the find and replace bar and the popups. It owns the text, a selection and
//! the platform's text input (IME, dead keys); the container paints the frame
//! (`inputField.border.*`, the focus ring) around it, since in the find bar
//! the frame spans the buttons too.

use crate::theme::{Metrics, Theme};
use gpui::{
    actions, div, fill, point, prelude::*, px, size, App, Bounds, ClipboardItem, Context,
    CursorStyle, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, Font, FontFeatures, FontStyle, FontWeight, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ShapedLine, SharedString, Style, TextAlign,
    TextRun, UTF16Selection, Window,
};
use std::ops::Range;

actions!(
    input,
    [
        InputLeft,
        InputRight,
        InputSelectLeft,
        InputSelectRight,
        InputWordLeft,
        InputWordRight,
        InputSelectWordLeft,
        InputSelectWordRight,
        InputHome,
        InputEnd,
        InputSelectHome,
        InputSelectEnd,
        InputBackspace,
        InputDelete,
        InputDeleteWordBackward,
        InputSelectAll,
        InputCopy,
        InputCut,
        InputPaste,
        InputSubmit,
        InputSubmitBack,
        InputCancel,
        InputTab,
        InputBackTab,
    ]
);

/// What the field tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Changed,
    /// Enter (`shift` for Shift+Enter).
    Submit {
        shift: bool,
    },
    Cancel,
    /// Tab (`shift` for Shift+Tab): the owner moves the focus.
    Tab {
        shift: bool,
    },
}

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Input");
    cx.bind_keys([
        KeyBinding::new("left", InputLeft, ctx),
        KeyBinding::new("right", InputRight, ctx),
        KeyBinding::new("shift-left", InputSelectLeft, ctx),
        KeyBinding::new("shift-right", InputSelectRight, ctx),
        KeyBinding::new("home", InputHome, ctx),
        KeyBinding::new("end", InputEnd, ctx),
        KeyBinding::new("shift-home", InputSelectHome, ctx),
        KeyBinding::new("shift-end", InputSelectEnd, ctx),
        KeyBinding::new("backspace", InputBackspace, ctx),
        KeyBinding::new("shift-backspace", InputBackspace, ctx),
        KeyBinding::new("delete", InputDelete, ctx),
        KeyBinding::new("ctrl-insert", InputCopy, ctx),
        KeyBinding::new("shift-insert", InputPaste, ctx),
        KeyBinding::new("enter", InputSubmit, ctx),
        KeyBinding::new("shift-enter", InputSubmitBack, ctx),
        KeyBinding::new("escape", InputCancel, ctx),
        KeyBinding::new("tab", InputTab, ctx),
        KeyBinding::new("shift-tab", InputBackTab, ctx),
    ]);
    // The platform's own text-field chords: Option moves by words and Command
    // to the line's ends and the clipboard on macOS, Ctrl elsewhere.
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("alt-left", InputWordLeft, ctx),
            KeyBinding::new("alt-right", InputWordRight, ctx),
            KeyBinding::new("alt-shift-left", InputSelectWordLeft, ctx),
            KeyBinding::new("alt-shift-right", InputSelectWordRight, ctx),
            KeyBinding::new("cmd-left", InputHome, ctx),
            KeyBinding::new("cmd-right", InputEnd, ctx),
            KeyBinding::new("cmd-shift-left", InputSelectHome, ctx),
            KeyBinding::new("cmd-shift-right", InputSelectEnd, ctx),
            KeyBinding::new("alt-backspace", InputDeleteWordBackward, ctx),
            KeyBinding::new("cmd-a", InputSelectAll, ctx),
            KeyBinding::new("cmd-c", InputCopy, ctx),
            KeyBinding::new("cmd-x", InputCut, ctx),
            KeyBinding::new("cmd-v", InputPaste, ctx),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-left", InputWordLeft, ctx),
            KeyBinding::new("ctrl-right", InputWordRight, ctx),
            KeyBinding::new("ctrl-shift-left", InputSelectWordLeft, ctx),
            KeyBinding::new("ctrl-shift-right", InputSelectWordRight, ctx),
            KeyBinding::new("ctrl-backspace", InputDeleteWordBackward, ctx),
            KeyBinding::new("ctrl-a", InputSelectAll, ctx),
            KeyBinding::new("ctrl-c", InputCopy, ctx),
            KeyBinding::new("ctrl-x", InputCut, ctx),
            KeyBinding::new("ctrl-v", InputPaste, ctx),
        ]);
    }
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    /// Byte offsets into `content`.
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    /// Paint with the editor's monospace face (the find field) or the UI face.
    pub mono: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        TextInput {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            mono: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Replaces the text and selects all of it (what Fleet's find field does
    /// with the selection it opens on), without an event.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_string();
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    /// Replaces the text with the caret at its end and nothing selected.
    pub fn set_text_at_end(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_string();
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Selects a byte range of the text (the tree's rename selects the stem).
    pub fn select_range(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let len = self.content.len();
        self.selected_range = range.start.min(len)..range.end.min(len);
        self.selection_reversed = false;
        cx.notify();
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content[..offset]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content[offset..]
            .chars()
            .next()
            .map(|c| offset + c.len_utf8())
            .unwrap_or(self.content.len())
    }

    fn word_left(&self, offset: usize) -> usize {
        let mut i = offset;
        let chars: Vec<(usize, char)> = self.content[..offset].char_indices().collect();
        let mut k = chars.len();
        while k > 0 && chars[k - 1].1.is_whitespace() {
            k -= 1;
            i = chars[k].0;
        }
        if k == 0 {
            return 0;
        }
        let kind = is_word(chars[k - 1].1);
        while k > 0 && !chars[k - 1].1.is_whitespace() && is_word(chars[k - 1].1) == kind {
            k -= 1;
            i = chars[k].0;
        }
        i
    }

    fn word_right(&self, offset: usize) -> usize {
        let mut i = offset;
        let rest: Vec<char> = self.content[offset..].chars().collect();
        let mut k = 0;
        if k < rest.len() && !rest[k].is_whitespace() {
            let kind = is_word(rest[k]);
            while k < rest.len() && !rest[k].is_whitespace() && is_word(rest[k]) == kind {
                i += rest[k].len_utf8();
                k += 1;
            }
        }
        while k < rest.len() && rest[k].is_whitespace() {
            i += rest[k].len_utf8();
            k += 1;
        }
        i
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        let mut utf8 = 0;
        for ch in self.content.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        line.closest_index_for_x(position.x - bounds.left())
    }

    // ---- actions --------------------------------------------------------

    fn left(&mut self, _: &InputLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let to = self.previous_boundary(self.cursor_offset());
            self.move_to(to, cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }
    fn right(&mut self, _: &InputRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let to = self.next_boundary(self.cursor_offset());
            self.move_to(to, cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }
    fn select_left(&mut self, _: &InputSelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.previous_boundary(self.cursor_offset());
        self.select_to(to, cx);
    }
    fn select_right(&mut self, _: &InputSelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.next_boundary(self.cursor_offset());
        self.select_to(to, cx);
    }
    fn word_left_action(&mut self, _: &InputWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.word_left(self.cursor_offset());
        self.move_to(to, cx);
    }
    fn word_right_action(&mut self, _: &InputWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.word_right(self.cursor_offset());
        self.move_to(to, cx);
    }
    fn select_word_left(
        &mut self,
        _: &InputSelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let to = self.word_left(self.cursor_offset());
        self.select_to(to, cx);
    }
    fn select_word_right(
        &mut self,
        _: &InputSelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let to = self.word_right(self.cursor_offset());
        self.select_to(to, cx);
    }
    fn home(&mut self, _: &InputHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }
    fn end(&mut self, _: &InputEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.content.len();
        self.move_to(end, cx);
    }
    fn select_home(&mut self, _: &InputSelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }
    fn select_end(&mut self, _: &InputSelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.content.len();
        self.select_to(end, cx);
    }
    fn backspace(&mut self, _: &InputBackspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            if prev == self.cursor_offset() {
                return;
            }
            self.select_to(prev, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }
    fn delete(&mut self, _: &InputDelete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if next == self.cursor_offset() {
                return;
            }
            self.select_to(next, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }
    fn delete_word_backward(
        &mut self,
        _: &InputDeleteWordBackward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            let to = self.word_left(self.cursor_offset());
            if to == self.cursor_offset() {
                return;
            }
            self.select_to(to, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }
    fn select_all_action(&mut self, _: &InputSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all(cx);
    }
    fn copy(&mut self, _: &InputCopy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &InputCut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }
    fn paste(&mut self, _: &InputPaste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text.replace("\r\n", "\n").replace('\n', " ");
            self.replace_text_in_range(None, &text, window, cx);
        }
    }
    fn submit(&mut self, _: &InputSubmit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit { shift: false });
    }
    fn submit_back(&mut self, _: &InputSubmitBack, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit { shift: true });
    }
    fn cancel(&mut self, _: &InputCancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
    }
    fn tab(&mut self, _: &InputTab, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Tab { shift: false });
    }
    fn back_tab(&mut self, _: &InputBackTab, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Tab { shift: true });
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        self.is_selecting = true;
        let at = self.index_for_mouse_position(event.position);
        if event.click_count >= 2 {
            self.select_all(cx);
        } else if event.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            let at = self.index_for_mouse_position(event.position);
            self.select_to(at, cx);
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            self.content[..range.start].to_owned() + new_text + &self.content[range.end..];
        let at = range.start + new_text.len();
        self.selected_range = at..at;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            self.content[..range.start].to_owned() + new_text + &self.content[range.end..];
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .map(|r| r.start + range.start..r.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                bounds.left() + layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        let index = layout.index_for_x(point.x - bounds.left())?;
        Some(self.offset_to_utf16(index))
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Input")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::word_left_action))
            .on_action(cx.listener(Self::word_right_action))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::select_all_action))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::submit_back))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::back_tab))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .size_full()
            .child(InputElement { input: cx.entity() })
    }
}

/// Paints the field's text, selection and caret.
struct InputElement {
    input: Entity<TextInput>,
}

struct InputPrepaint {
    line: ShapedLine,
    caret: Option<Pixels>,
    selection: Option<Range<Pixels>>,
    hitbox: Hitbox,
    placeholder: bool,
}

impl IntoElement for InputElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for InputElement {
    type RequestLayoutState = ();
    type PrepaintState = InputPrepaint;

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
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
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
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let input = self.input.read(cx);
        let placeholder = input.content.is_empty();
        let text: SharedString = if placeholder {
            input.placeholder.clone()
        } else {
            input.content.clone().into()
        };
        let font = Font {
            family: if input.mono {
                theme.mono_font.clone()
            } else {
                theme.ui_font.clone()
            },
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };
        let color = if placeholder {
            theme.input_hint
        } else {
            theme.input_text
        };
        let run = TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: input.marked_range.as_ref().map(|_| gpui::UnderlineStyle {
                thickness: px(1.),
                color: Some(theme.input_text),
                wavy: false,
            }),
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(text, px(m.ui_font_size), &[run], None);
        let focused = input.focus_handle.is_focused(window);
        let (caret, selection) = if placeholder {
            (focused.then_some(px(0.)), None)
        } else {
            let range = input.selected_range.clone();
            let head = input.cursor_offset();
            let sel = (!range.is_empty())
                .then(|| line.x_for_index(range.start)..line.x_for_index(range.end));
            (focused.then(|| line.x_for_index(head)), sel)
        };
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        InputPrepaint {
            line,
            caret,
            selection,
            hitbox,
            placeholder,
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
        let m = *cx.global::<Metrics>();
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);
        let line_height = px(m.ui_font_size * 18. / 13.).round();
        let y = bounds.origin.y + (bounds.size.height - line_height) / 2.;
        if let Some(sel) = &prepaint.selection {
            window.paint_quad(fill(
                Bounds::new(
                    point(bounds.origin.x + sel.start, y),
                    size(sel.end - sel.start, line_height),
                ),
                theme.input_selection,
            ));
        }
        let _ = prepaint.line.paint(
            point(bounds.origin.x, y),
            line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
        if let Some(x) = prepaint.caret {
            window.paint_quad(fill(
                Bounds::new(
                    point(bounds.origin.x + x, y),
                    size(px(m.caret_width / 2.).max(px(1.)), line_height),
                ),
                theme.input_caret,
            ));
        }
        let line = prepaint.line.clone();
        let placeholder = prepaint.placeholder;
        self.input.update(cx, |input, _| {
            input.last_layout = (!placeholder).then_some(line);
            input.last_bounds = Some(bounds);
        });
    }
}
