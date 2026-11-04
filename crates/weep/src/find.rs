//! The find and replace bar: Fleet's search field in the editor island's
//! top-right corner (batch 5, `docs/fleet-probe.md`), a second field under it
//! for the replacement, the match counter, ↑ ↓, the match-case, whole-words
//! and regex toggles, and the close box. The bar belongs to a split and
//! searches the split's active editor; the matches it finds are painted by
//! the editor (`EditorView::search`), the current one is selected and
//! scrolled into view.

use crate::assets::icon;
use crate::editor::{EditorEvent, EditorView, Search};
use crate::input::{InputEvent, TextInput};
use crate::list_popup::{tui_gap, tui_input_row, tui_toggle};
use crate::settings::AppSettings;
use crate::skin::{self, Cell, Skin};
use crate::theme::{Metrics, Theme};
use crate::tooltip::Tooltip;
use gpui::{
    actions, div, point, prelude::*, px, App, BoxShadow, Context, Div, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, KeyBinding, SharedString, Stateful, Subscription, Window,
};

actions!(
    find_bar,
    [
        FindBarTab,
        FindBarBackTab,
        FindBarPress,
        FindBarEnter,
        FindBarCancel
    ]
);

/// The keys of a bar button holding the focus (the `find-tab` scene): Tab and
/// Shift+Tab walk the bar, Space presses the button, Enter does nothing and
/// Escape closes the bar.
pub fn bind_keys(cx: &mut App) {
    let ctx = Some("FindBarButton");
    cx.bind_keys([
        KeyBinding::new("tab", FindBarTab, ctx),
        KeyBinding::new("shift-tab", FindBarBackTab, ctx),
        KeyBinding::new("space", FindBarPress, ctx),
        KeyBinding::new("enter", FindBarEnter, ctx),
        KeyBinding::new("escape", FindBarCancel, ctx),
    ]);
}
use std::ops::Range;
use weep_text::search::{Query, SearchOptions};

#[derive(Clone, Debug)]
pub enum FindBarEvent {
    /// The bar closed; the editor takes the focus back.
    Closed,
    /// Opened, closed or the query changed: the pane lays the editor out
    /// again (a typed query pushes the text down).
    Changed,
}

pub struct FindBar {
    editor: Option<Entity<EditorView>>,
    editor_subscription: Option<Subscription>,
    pub find: Entity<TextInput>,
    pub replace: Entity<TextInput>,
    pub open: bool,
    /// A query was typed since the bar opened: Fleet then pushes the text
    /// down under the find row (a bar opened on a query does not).
    pub pushed: bool,
    pub replace_shown: bool,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    /// Search only in the selection the bar opened with (a multi-line one),
    /// with the buffer version it was taken at.
    in_selection: Option<(Range<usize>, u64)>,
    matches: Vec<Range<usize>>,
    current: Option<usize>,
    error: Option<String>,
    /// The caret when the bar opened: the first match at or after it is current.
    origin: usize,
    /// Held while one of the bar's buttons has the keyboard focus; `button`
    /// says which one.
    focus_handle: FocusHandle,
    button: Stop,
}

impl EventEmitter<FindBarEvent> for FindBar {}

impl FindBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let find = cx.new(|cx| {
            let mut input = TextInput::new(
                format!(
                    "Search ({} for history)",
                    crate::keymap::chord_text("Alt+Up", "alt-up")
                ),
                cx,
            );
            input.mono = true;
            // The bar draws its own prompt cells (`/`, `↳`) in the tui skin.
            input.tui_prompt = false;
            input
        });
        let replace = cx.new(|cx| {
            let mut input = TextInput::new("Replace", cx);
            input.mono = true;
            // The bar draws its own prompt cells (`/`, `↳`) in the tui skin.
            input.tui_prompt = false;
            input
        });
        cx.subscribe(&find, Self::on_find_input).detach();
        cx.subscribe(&replace, Self::on_replace_input).detach();
        FindBar {
            editor: None,
            editor_subscription: None,
            find,
            replace,
            open: false,
            pushed: false,
            replace_shown: false,
            case_sensitive: false,
            whole_word: false,
            regex: false,
            in_selection: None,
            matches: Vec::new(),
            current: None,
            error: None,
            origin: 0,
            focus_handle: cx.focus_handle(),
            button: Stop::Prev,
        }
    }

    pub fn query(&self, cx: &App) -> String {
        self.find.read(cx).text().to_string()
    }

    /// Points the bar at another editor (the split's active tab changed).
    pub fn set_editor(&mut self, editor: Option<Entity<EditorView>>, cx: &mut Context<Self>) {
        if self.editor == editor {
            return;
        }
        if let Some(old) = &self.editor {
            old.update(cx, |e, cx| {
                e.set_search(None, cx);
                e.set_status_top(px(0.), cx);
            });
        }
        self.editor_subscription = editor.as_ref().map(|e| {
            cx.subscribe(e, |this, _, event: &EditorEvent, cx| {
                if let EditorEvent::Changed = event {
                    if this.open {
                        this.search(false, cx);
                    }
                }
            })
        });
        self.editor = editor;
        self.sync_status_top(cx);
        self.in_selection = None;
        if self.open {
            self.search(true, cx);
        }
        cx.notify();
    }

    /// `find` / `replace`: opens the bar (with the replace row when
    /// `with_replace`), seeds the field with the selection when
    /// `editor.useSelectionForSearch` and the selection is on one line, and
    /// focuses the field with its text selected.
    pub fn show(&mut self, with_replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        // The selection's start, not the caret: a match selected by the bar
        // stays current when it opens again (the `find-tab` scene: Fleet
        // shows 1/8 on each reopen).
        let (selected, origin, multiline) = editor.update(cx, |e, cx| {
            let text = e.selected_text(cx);
            (
                text.clone(),
                e.selection_range(cx).start,
                text.contains('\n'),
            )
        });
        // Fleet's find opens without the replace row even after replace
        // was used; an open bar keeps it.
        self.replace_shown = with_replace || (self.open && self.replace_shown);
        self.open = true;
        cx.emit(FindBarEvent::Changed);
        self.origin = origin;
        let use_selection = cx
            .global::<AppSettings>()
            .settings
            .editor_use_selection_for_search();
        if multiline {
            self.in_selection =
                Some(editor.update(cx, |e, cx| (e.selection_range(cx), e.version(cx))));
        } else {
            self.in_selection = None;
            if use_selection && !selected.is_empty() {
                self.find.update(cx, |f, cx| f.set_text(&selected, cx));
            } else {
                self.find.update(cx, |f, cx| f.select_all(cx));
            }
        }
        let field = if with_replace && !self.find.read(cx).is_empty() {
            self.replace.clone()
        } else {
            self.find.clone()
        };
        let handle = field.read(cx).focus_handle(cx);
        handle.focus(window, cx);
        self.search(true, cx);
        self.sync_status_top(cx);
        cx.notify();
    }

    /// Whether the text is pushed down under the find row: a query typed
    /// since the bar opened (a bar opened on one lies over the text).
    pub fn pushes(&self, cx: &App) -> bool {
        self.open && self.pushed && !self.find.read(cx).is_empty()
    }

    /// Puts the editor's status badge under the bar, as Fleet does: below
    /// the find row's band (`FIND_BAR_PUSH`) and the replace row, less the
    /// part the text is already pushed down by.
    fn sync_status_top(&self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let m = *cx.global::<Metrics>();
        let top = if !self.open {
            0.
        } else {
            let replace = if self.replace_shown {
                m.find_bar_row_gap + m.find_bar_height
            } else {
                0.
            };
            let pushed = if self.pushes(cx) { m.find_bar_push } else { 0. };
            m.find_bar_push + replace - pushed
        };
        editor.update(cx, |e, cx| e.set_status_top(px(top), cx));
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.pushed = false;
        self.sync_status_top(cx);
        cx.emit(FindBarEvent::Changed);
        self.in_selection = None;
        if let Some(editor) = &self.editor {
            editor.update(cx, |e, cx| e.set_search(None, cx));
        }
        cx.emit(FindBarEvent::Closed);
        cx.notify();
    }

    fn options(&self) -> SearchOptions {
        SearchOptions {
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            in_range: self.in_selection.as_ref().map(|(r, _)| r.clone()),
        }
    }

    /// Runs the query; with `reseat`, the current match becomes the first at or
    /// after the origin, otherwise the one nearest the previous current match.
    fn search(&mut self, reseat: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let query_text = self.query(cx);
        let previous_start = self
            .current
            .and_then(|i| self.matches.get(i))
            .map(|m| m.start);
        self.error = None;
        if let Some((range, version)) = &mut self.in_selection {
            // The selection moves with edits inside it; keep it on the same text.
            let (adjusted, now) = editor.update(cx, |e, cx| {
                (e.adjust_range(range.clone(), *version, cx), e.version(cx))
            });
            *range = adjusted;
            *version = now;
        }
        self.matches = match Query::new(&query_text, self.options()) {
            Ok(query) => {
                let buffer_matches = editor.update(cx, |e, cx| {
                    let doc = e.document.read(cx);
                    query.find_in(&doc.buffer)
                });
                buffer_matches
            }
            Err(message) => {
                if !message.is_empty() {
                    self.error = Some(message);
                }
                Vec::new()
            }
        };
        self.current = if self.matches.is_empty() {
            None
        } else {
            // On opening (or a new query) the first match touching or after
            // the caret is current: Fleet keeps the match the caret sits at
            // the end of (the `find-tab` scene, 1/8 after Escape put the
            // caret after the selected match). Otherwise the current match
            // stays where it was.
            let at = if reseat {
                let origin = self.origin;
                self.matches.iter().position(|m| m.end >= origin)
            } else {
                let anchor = previous_start.unwrap_or(self.origin);
                self.matches.iter().position(|m| m.start >= anchor)
            };
            Some(at.unwrap_or(0))
        };
        self.push_to_editor(reseat || previous_start.is_none(), cx);
        cx.notify();
    }

    fn push_to_editor(&mut self, select: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let search = Search {
            matches: self.matches.clone(),
            current: self.current,
        };
        let current = self.current.and_then(|i| self.matches.get(i).cloned());
        editor.update(cx, |e, cx| {
            e.set_search(Some(search), cx);
            if select {
                if let Some(range) = current {
                    e.select_range(range, cx);
                }
            }
        });
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        self.current = Some(match self.current {
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
            None => 0,
        });
        self.push_to_editor(true, cx);
        cx.notify();
    }

    /// `next-occurrence` / `prev-occurrence`: with the bar closed these still
    /// step through the last query's matches.
    pub fn next(&mut self, cx: &mut Context<Self>) {
        if !self.open && !self.matches.is_empty() {
            self.search(false, cx);
        }
        self.step(true, cx);
    }

    pub fn prev(&mut self, cx: &mut Context<Self>) {
        if !self.open && !self.matches.is_empty() {
            self.search(false, cx);
        }
        self.step(false, cx);
    }

    /// `replace-one`: replaces the current match and moves to the next.
    pub fn replace_one(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let Some(i) = self.current else {
            return;
        };
        let Some(range) = self.matches.get(i).cloned() else {
            return;
        };
        let Ok(query) = Query::new(&self.query(cx), self.options()) else {
            return;
        };
        let replacement = self.replace.read(cx).text().to_string();
        editor.update(cx, |e, cx| {
            let text = e.document.read(cx).buffer.text();
            let with = query.replacement_for(&text, range.clone(), &replacement);
            e.replace_range(range, &with, cx);
        });
        // The document's change event re-ran the search from the caret, which
        // now sits after the replacement: the next match is current.
        self.push_to_editor(true, cx);
    }

    /// `replace-all`: every match, last to first, in one undo step.
    pub fn replace_all(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let Ok(query) = Query::new(&self.query(cx), self.options()) else {
            return;
        };
        let replacement = self.replace.read(cx).text().to_string();
        let matches = self.matches.clone();
        if matches.is_empty() {
            return;
        }
        editor.update(cx, |e, cx| {
            let text = e.document.read(cx).buffer.text();
            let edits: Vec<(Range<usize>, String)> = matches
                .iter()
                .map(|m| {
                    (
                        m.clone(),
                        query.replacement_for(&text, m.clone(), &replacement),
                    )
                })
                .collect();
            e.replace_ranges(edits, cx);
        });
    }

    fn toggle(&mut self, which: Toggle, cx: &mut Context<Self>) {
        match which {
            Toggle::Case => self.case_sensitive = !self.case_sensitive,
            Toggle::Word => self.whole_word = !self.whole_word,
            Toggle::Regex => self.regex = !self.regex,
        }
        self.search(false, cx);
    }

    fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_shown = !self.replace_shown;
        if !self.replace_shown && self.replace.read(cx).is_focused(window) {
            let handle = self.find.read(cx).focus_handle(cx);
            handle.focus(window, cx);
        }
        self.sync_status_top(cx);
        cx.notify();
    }

    fn on_find_input(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => {
                self.pushed = true;
                self.search(true, cx);
                self.sync_status_top(cx);
                cx.emit(FindBarEvent::Changed);
            }
            InputEvent::Submit { shift } => self.step(!shift, cx),
            InputEvent::Cancel => self.close(cx),
            InputEvent::Tab { shift } => cx.defer_move_focus(!shift),
        }
    }

    fn on_replace_input(
        &mut self,
        _: Entity<TextInput>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {}
            InputEvent::Submit { shift: false } => self.replace_one(cx),
            InputEvent::Submit { shift: true } => self.replace_all(cx),
            InputEvent::Cancel => self.close(cx),
            InputEvent::Tab { shift } => cx.defer_move_focus(!shift),
        }
    }

    /// The bar's Tab stops in Fleet's order (the `find-tab` scene): the find
    /// field, the replace field when shown, ↑, ↓, the three toggles, the
    /// close box and the chevron, round and round; Replace and Replace All
    /// are not stops. The tui skin's bar keeps Tab between its fields.
    fn stops(&self, cx: &App) -> Vec<Stop> {
        let mut stops = vec![Stop::Find];
        if self.replace_shown {
            stops.push(Stop::Replace);
        }
        if Skin::current(cx) != Skin::Tui {
            stops.extend([
                Stop::Prev,
                Stop::Next,
                Stop::Case,
                Stop::Word,
                Stop::Regex,
                Stop::Close,
                Stop::Chevron,
            ]);
        }
        stops
    }

    fn focused_stop(&self, window: &Window, cx: &App) -> Option<Stop> {
        if self.find.read(cx).is_focused(window) {
            Some(Stop::Find)
        } else if self.replace.read(cx).is_focused(window) {
            Some(Stop::Replace)
        } else if self.focus_handle.is_focused(window) {
            Some(self.button)
        } else {
            None
        }
    }

    /// The button drawing the focus ring, if one holds the focus.
    fn focused_button(&self, window: &Window) -> Option<Stop> {
        self.focus_handle.is_focused(window).then_some(self.button)
    }

    fn move_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.stops(cx);
        let n = stops.len();
        let at = self
            .focused_stop(window, cx)
            .and_then(|s| stops.iter().position(|t| *t == s));
        let next = match at {
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
            None => 0,
        };
        match stops[next] {
            Stop::Find => self.find.read(cx).focus_handle(cx).focus(window, cx),
            Stop::Replace => self.replace.read(cx).focus_handle(cx).focus(window, cx),
            button => {
                self.button = button;
                self.focus_handle.focus(window, cx);
            }
        }
        cx.notify();
    }

    fn on_tab(&mut self, _: &FindBarTab, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(true, window, cx);
    }

    fn on_back_tab(&mut self, _: &FindBarBackTab, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(false, window, cx);
    }

    /// Space on a focused button: what a click does.
    fn on_press(&mut self, _: &FindBarPress, window: &mut Window, cx: &mut Context<Self>) {
        match self.button {
            Stop::Prev => self.step(false, cx),
            Stop::Next => self.step(true, cx),
            Stop::Case => self.toggle(Toggle::Case, cx),
            Stop::Word => self.toggle(Toggle::Word, cx),
            Stop::Regex => self.toggle(Toggle::Regex, cx),
            Stop::Close => self.close(cx),
            Stop::Chevron => self.toggle_replace(window, cx),
            Stop::Find | Stop::Replace => {}
        }
    }

    /// Fleet's Enter on a focused button neither presses it nor reaches the
    /// editor.
    fn on_enter(&mut self, _: &FindBarEnter, _: &mut Window, _: &mut Context<Self>) {}

    fn on_cancel(&mut self, _: &FindBarCancel, _: &mut Window, cx: &mut Context<Self>) {
        self.close(cx);
    }

    /// The node holding the focus for the buttons, with their keys. It is
    /// not an ancestor of the fields, so Space still types in them.
    fn focus_node(&self, cx: &mut Context<Self>) -> Div {
        div()
            .absolute()
            .track_focus(&self.focus_handle)
            .key_context("FindBarButton")
            .on_action(cx.listener(Self::on_tab))
            .on_action(cx.listener(Self::on_back_tab))
            .on_action(cx.listener(Self::on_press))
            .on_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(Self::on_cancel))
    }

    pub fn counter_text(&self) -> Option<SharedString> {
        if let Some(error) = &self.error {
            return Some(error.clone().into());
        }
        match (self.current, self.matches.len()) {
            (_, 0) => None,
            (Some(i), n) => Some(format!("{}/{}", i + 1, n).into()),
            (None, n) => Some(format!("{n}").into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Toggle {
    Case,
    Word,
    Regex,
}

/// A Tab stop of the bar: its two fields and its buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Find,
    Replace,
    Prev,
    Next,
    Case,
    Word,
    Regex,
    Close,
    Chevron,
}

/// A Tab in a field moves the focus once the field's event is handled; the
/// window is the active one, where the key was pressed.
trait DeferFocus {
    fn defer_move_focus(&mut self, forward: bool);
}

impl DeferFocus for Context<'_, FindBar> {
    fn defer_move_focus(&mut self, forward: bool) {
        let bar = self.entity();
        self.defer(move |cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| {
                    bar.update(cx, |bar, cx| bar.move_focus(forward, window, cx))
                });
            }
        });
    }
}

/// A 16 px ghost icon button inside the field; `focused` draws the focus
/// ring 2 outside it (`FIND_BAR_FOCUS_RING`).
fn ghost_icon(
    id: impl Into<ElementId>,
    path: &'static str,
    focused: bool,
    theme: &Theme,
    m: &Metrics,
) -> Stateful<Div> {
    let inset = (m.find_bar_focus_ring - m.icon_size) / 2.;
    div()
        .id(id)
        .relative()
        .size(px(m.icon_size))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3. * m.zoom))
        .cursor_pointer()
        .hover(|s| s.bg(theme.ghost_hovered))
        .active(|s| s.bg(theme.ghost_pressed))
        .child(icon(path, m.icon_size, theme.icon_primary))
        .when(focused, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(-inset))
                    .left(px(-inset))
                    .size(px(m.find_bar_focus_ring))
                    .rounded(px(m.button_radius))
                    .border_1()
                    .border_color(theme.ghost_focus_border),
            )
        })
}

fn separator(theme: &Theme, m: &Metrics) -> Div {
    div()
        .w(px(1.))
        .h(px(m.find_bar_separator_height))
        .flex_shrink_0()
        .bg(theme.separator)
}

/// A field's frame: `inputField.border.default`, or when `focused` the 1.33
/// focus border inside the 2 px focus outline (batch 5).
fn frame(focused: bool, window: &Window, theme: &Theme, m: &Metrics) -> Div {
    div()
        .h(px(m.find_bar_height))
        .w(px(m.find_bar_width))
        .flex_shrink_0()
        .flex()
        .items_center()
        .rounded(px(m.input_radius))
        .bg(theme.input_bg)
        .map(|d| {
            if focused {
                // gpui paints a shadow under the whole box, so a clear field
                // would show the outline inside it: fill it as what is under it.
                d.bg(theme.island_background.blend(theme.input_bg))
                    .border(crate::theme::device_line(window, m.input_focus_border))
                    .border_color(theme.input_focus_border)
                    .shadow(vec![BoxShadow {
                        color: theme.input_focus_outline,
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(m.focus_outline),
                    }])
            } else {
                d.border(px(1.)).border_color(theme.input_border)
            }
        })
}

impl Render for FindBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(window, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let entity = cx.entity();
        let find_focused = self.find.read(cx).is_focused(window);
        let replace_focused = self.replace.read(cx).is_focused(window);
        let ring = self.focused_button(window);
        // Fleet keeps the find field's focus border while a button inside it
        // holds the focus; the chevron sits outside the field.
        let find_frame_focused = find_focused || ring.is_some_and(|b| b != Stop::Chevron);
        let counter = self.counter_text();
        let counter_color = if self.error.is_some() {
            theme.text_dangerous
        } else {
            theme.text_secondary
        };
        let toggle =
            |id: &'static str, label: &'static str, on: bool, which: Toggle, stop: Stop| {
                let entity = entity.clone();
                let focused = ring == Some(stop);
                div()
                    .id(id)
                    .w(px(m.find_bar_toggle))
                    .h(px(m.find_bar_toggle))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(m.button_radius))
                    .cursor_pointer()
                    .border_1()
                    .map(|d| {
                        if on {
                            d.bg(theme.toggle_on_bg)
                                .border_color(theme.toggle_on_border)
                                .text_color(theme.toggle_on_text)
                                .hover(|s| s.bg(theme.toggle_on_bg_hovered))
                        } else {
                            d.bg(theme.toggle_off_bg)
                                .border_color(gpui::transparent_black())
                                .text_color(theme.toggle_off_text)
                                .hover(|s| {
                                    s.bg(theme.toggle_off_bg_hovered)
                                        .text_color(theme.toggle_off_text_hovered)
                                })
                                .active(|s| s.bg(theme.toggle_off_bg_pressed))
                        }
                    })
                    .when(focused, |d| {
                        d.border_color(if on {
                            theme.toggle_on_focus_border
                        } else {
                            theme.toggle_off_focus_border
                        })
                    })
                    .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.toggle(which, cx)))
                    .child(label)
            };
        let chevron = {
            let entity = entity.clone();
            ghost_icon(
                "find-toggle-replace",
                if self.replace_shown {
                    "expui/general/chevronDown"
                } else {
                    "expui/general/chevronRight"
                },
                ring == Some(Stop::Chevron),
                &theme,
                &m,
            )
            .mr(px(m.find_bar_chevron_gap))
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| this.toggle_replace(window, cx))
            })
        };
        let up = {
            let entity = entity.clone();
            ghost_icon(
                "find-prev",
                "weep/arrow-up",
                ring == Some(Stop::Prev),
                &theme,
                &m,
            )
            .tooltip(Tooltip::text("Previous Occurrence"))
            .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.step(false, cx)))
        };
        let down = {
            let entity = entity.clone();
            ghost_icon(
                "find-next",
                "weep/arrow-down",
                ring == Some(Stop::Next),
                &theme,
                &m,
            )
            .tooltip(Tooltip::text("Next Occurrence"))
            .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.step(true, cx)))
        };
        let close = {
            let entity = entity.clone();
            ghost_icon(
                "find-close",
                "expui/general/close",
                ring == Some(Stop::Close),
                &theme,
                &m,
            )
            .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.close(cx)))
        };
        let find_row = div().flex().items_center().child(chevron).child(
            frame(find_frame_focused, window, &theme, &m)
                .pl(px(m.find_bar_pad_left))
                .pr(px(m.find_bar_pad_right))
                .child(div().flex_1().min_w_0().h_full().child(self.find.clone()))
                .children(counter.map(|c| {
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_color(counter_color)
                        .child(c)
                }))
                .child(div().w(px(m.find_bar_counter_gap)).flex_shrink_0())
                .child(up)
                .child(div().w(px(m.find_bar_arrow_gap)).flex_shrink_0())
                .child(down)
                .child(div().w(px(m.find_bar_close_gap)).flex_shrink_0())
                .child(separator(&theme, &m))
                .child(div().w(px(m.find_bar_separator_gap_right)).flex_shrink_0())
                .child(toggle(
                    "find-case",
                    "Aa",
                    self.case_sensitive,
                    Toggle::Case,
                    Stop::Case,
                ))
                .child(div().w(px(m.find_bar_toggle_gap)).flex_shrink_0())
                .child(toggle(
                    "find-word",
                    "W",
                    self.whole_word,
                    Toggle::Word,
                    Stop::Word,
                ))
                .child(div().w(px(m.find_bar_toggle_gap)).flex_shrink_0())
                .child(toggle(
                    "find-regex",
                    ".*",
                    self.regex,
                    Toggle::Regex,
                    Stop::Regex,
                ))
                .child(div().w(px(m.find_bar_separator_gap_left)).flex_shrink_0())
                .child(separator(&theme, &m))
                .child(div().w(px(m.find_bar_close_gap)).flex_shrink_0())
                .child(close),
        );
        let text_button =
            |id: &'static str, label: &'static str, on_click: Box<dyn Fn(&mut App)>| {
                div()
                    .id(id)
                    .h(px(m.button_height))
                    .px(px(m.find_bar_replace_button_pad_x))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .rounded(px(m.button_radius))
                    .cursor_pointer()
                    .text_color(theme.button_secondary_text)
                    .hover(|s| s.bg(theme.ghost_hovered))
                    .active(|s| s.bg(theme.ghost_pressed))
                    .on_click(move |_, _, cx| on_click(cx))
                    .child(label)
            };
        let replace_row = self.replace_shown.then(|| {
            let one = entity.clone();
            let all = entity.clone();
            div()
                .mt(px(m.find_bar_row_gap))
                .flex()
                .items_center()
                .child(
                    div()
                        .w(px(m.icon_size + m.find_bar_chevron_gap))
                        .flex_shrink_0(),
                )
                .child(
                    frame(replace_focused, window, &theme, &m)
                        .pl(px(m.find_bar_pad_left))
                        .pr(px(m.find_bar_pad_right - m.find_bar_replace_button_pad_x))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .child(self.replace.clone()),
                        )
                        .child(text_button(
                            "replace-one",
                            "Replace",
                            Box::new(move |cx| one.update(cx, |this, cx| this.replace_one(cx))),
                        ))
                        .child(text_button(
                            "replace-all",
                            "Replace All",
                            Box::new(move |cx| all.update(cx, |this, cx| this.replace_all(cx))),
                        )),
                )
        });
        div()
            .id("find-bar")
            .absolute()
            .top(px(m.find_bar_top - m.find_bar_cover_top))
            .right(px(m.find_bar_right))
            // The text under the bar is hidden, not shown through the fields.
            .pt(px(m.find_bar_cover_top))
            .pl(px(m.find_bar_cover_left))
            .pb(px(m.find_bar_cover_bottom))
            .bg(theme.island_background)
            .flex()
            .flex_col()
            .items_end()
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            .text_color(theme.text_primary)
            // The bar sits over the text; clicks on it must not reach the editor.
            .occlude()
            .child(self.focus_node(cx))
            .child(find_row)
            .children(replace_row)
            .into_any_element()
    }
}

// ---- the tui skin ---------------------------------------------------------------

impl FindBar {
    /// The rows the bar takes at the bottom of the editor's island in the
    /// tui skin, its rule included: 0 closed, 2, or 3 with the replace row.
    /// The rule is the second of them counted up from the frame's bottom
    /// edge plus the replace row (`Edges::rules_from_bottom`).
    pub fn tui_rows(&self) -> usize {
        if self.open {
            2 + usize::from(self.replace_shown)
        } else {
            0
        }
    }

    /// The command line (`docs/tui-skin.md`, Find / replace bar): a rule
    /// across the island, then `/ query  [Aa] [W] [.*] [↳]  3/17  ↑ ↓ ×`
    /// and, with replace, `↳ text  [ Replace ] [ All ]`; at the bottom of
    /// the editor's body, which the bar is laid over.
    fn render_tui(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let entity = cx.entity();
        let find_focused = self.find.read(cx).is_focused(window);
        let replace_focused = self.replace.read(cx).is_focused(window);
        let empty_query = self.find.read(cx).is_empty();
        let (counter, counter_color) = match self.counter_text() {
            Some(c) if self.error.is_some() => (c, theme.text_dangerous),
            Some(c) => (c, theme.text_secondary),
            None if !empty_query => ("0/0".into(), theme.text_dangerous),
            None => ("".into(), theme.text_secondary),
        };
        let counter = skin::fit_text(&counter, 24);
        let toggle = |label: &'static str, on: bool, which: Option<Toggle>, ix: u64| {
            let entity = entity.clone();
            tui_toggle(
                ElementId::NamedInteger("find-option".into(), ix),
                label,
                on,
                &theme,
                cell,
            )
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| match which {
                    Some(which) => this.toggle(which, cx),
                    None => this.toggle_replace(window, cx),
                })
            })
            .into_any_element()
        };
        let mark =
            |id: &'static str, glyph: &'static str, f: fn(&mut FindBar, &mut Context<FindBar>)| {
                let entity = entity.clone();
                div()
                    .id(id)
                    .w(cell.cols(1))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .text_color(theme.text_secondary)
                    .hover(|s| s.bg(theme.list_bg_hovered).text_color(theme.text_primary))
                    .on_click(move |_, _, cx| entity.update(cx, f))
                    .child(glyph)
                    .into_any_element()
            };
        let find_row = tui_input_row(
            "/",
            find_focused,
            self.find.clone(),
            vec![
                tui_gap(cell, 1),
                toggle("Aa", self.case_sensitive, Some(Toggle::Case), 0),
                tui_gap(cell, 1),
                toggle("W", self.whole_word, Some(Toggle::Word), 1),
                tui_gap(cell, 1),
                toggle(".*", self.regex, Some(Toggle::Regex), 2),
                tui_gap(cell, 1),
                toggle("↳", self.replace_shown, None, 3),
                tui_gap(cell, 2),
                div()
                    .flex_shrink_0()
                    .w(cell.cols(Cell::text_cols(&counter)))
                    .whitespace_nowrap()
                    .text_color(counter_color)
                    .child(SharedString::from(counter))
                    .into_any_element(),
                tui_gap(cell, 2),
                mark("find-prev", "↑", |this, cx| this.step(false, cx)),
                tui_gap(cell, 1),
                mark("find-next", "↓", |this, cx| this.step(true, cx)),
                tui_gap(cell, 1),
                mark("find-close", "×", |this, cx| this.close(cx)),
                tui_gap(cell, 1),
            ],
            &theme,
            cell,
        );
        let replace_row = self.replace_shown.then(|| {
            let one = entity.clone();
            let all = entity.clone();
            tui_input_row(
                "↳",
                replace_focused,
                self.replace.clone(),
                vec![
                    tui_gap(cell, 1),
                    skin::button("replace-one", "Replace", false, &theme, cell)
                        .on_click(move |_, _, cx| one.update(cx, |this, cx| this.replace_one(cx)))
                        .into_any_element(),
                    tui_gap(cell, 1),
                    skin::button("replace-all", "All", false, &theme, cell)
                        .on_click(move |_, _, cx| all.update(cx, |this, cx| this.replace_all(cx)))
                        .into_any_element(),
                    tui_gap(cell, 1),
                ],
                &theme,
                cell,
            )
        });
        // The island's frame is focused while one of the bar's fields is.
        let line = if find_focused || replace_focused {
            theme.border_focused
        } else {
            theme.border
        };
        let rule = gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let t = skin::stroke(window, &m);
                let y = skin::snap(window, f32::from(bounds.origin.y) + cell.h / 2.) - t / 2.;
                window.paint_quad(gpui::fill(
                    gpui::Bounds::new(
                        point(bounds.origin.x, px(y)),
                        gpui::size(bounds.size.width, px(t)),
                    ),
                    line,
                ));
            },
        )
        .w_full()
        .h(px(cell.h))
        .flex_shrink_0();
        div()
            .id("find-bar")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(cell.rows(self.tui_rows()))
            .flex()
            .flex_col()
            .bg(theme.island_background)
            .font_family(theme.mono_font.clone())
            .text_size(px(cell.font_size))
            .line_height(px(cell.h))
            .text_color(theme.text_primary)
            // The bar sits over the text; clicks on it must not reach the editor.
            .occlude()
            .child(self.focus_node(cx))
            .child(rule)
            .child(find_row)
            .children(replace_row)
            .into_any_element()
    }
}
