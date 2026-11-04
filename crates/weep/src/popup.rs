//! Small prompt popups under the toolbar: a field (`goto/line`, `new-file`,
//! `files/rename`) or a list (`editor/change-line-endings`,
//! `editor/paste-from-history`). Painted from `popup.*`, `inputField.*` and
//! `listItem.*`. Fleet's Goto popup is batch 6 of the probe plan; until it
//! runs, the width, the top offset and the row height are estimates taken
//! from the Run & Debug popup batch 5 caught (687 wide at y 64.7, 24 px
//! rows, a 28 px field with the focus ring).

use crate::actions::{PopupBackspace, PopupCancel, PopupConfirm, PopupDown, PopupUp};
use crate::input::{InputEvent, TextInput};
use crate::list_popup::{tui_cols, tui_input_row, tui_row, tui_scrollbar, tui_top, TuiRow};
use crate::skin::{self, Cell, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    deferred, div, point, prelude::*, px, App, BoxShadow, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, ScrollHandle, SharedString, Window,
};
use std::path::PathBuf;

/// Estimates until batch 6 runs (`docs/fleet-probe.md`).
pub const PROMPT_WIDTH: f32 = 600.;
pub const PROMPT_TOP: f32 = 64.67;
pub const PROMPT_ROW_HEIGHT: f32 = 24.;
pub const PROMPT_FIELD_HEIGHT: f32 = 28.;
pub const PROMPT_MAX_ROWS: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// "Line:Column" into the active editor.
    GotoLine,
    /// A file name, created under `dir` and opened.
    NewFile { dir: PathBuf },
    /// A new name for `path`.
    Rename { path: PathBuf },
    /// A folder name, created under `dir` (`files/new-folder`).
    NewFolder { dir: PathBuf },
    /// LF or CRLF for the active document.
    LineEndings,
    /// One of the recent clipboard texts, pasted into the active editor.
    PasteHistory,
    /// M7: a new name for the symbol at the caret (`rename`), through the
    /// language server.
    RenameSymbol,
}

impl PromptKind {
    fn title(&self) -> &'static str {
        match self {
            PromptKind::GotoLine => "Go to Line:Column",
            PromptKind::NewFile { .. } => "New File",
            PromptKind::Rename { .. } => "Rename",
            PromptKind::NewFolder { .. } => "New Folder",
            PromptKind::LineEndings => "Line Endings",
            PromptKind::PasteHistory => "Paste from History",
            PromptKind::RenameSymbol => "Rename",
        }
    }

    fn placeholder(&self) -> &'static str {
        match self {
            PromptKind::GotoLine => "Line:Column",
            PromptKind::NewFile { .. } => "File name",
            PromptKind::Rename { .. } | PromptKind::RenameSymbol => "New name",
            PromptKind::NewFolder { .. } => "Folder name",
            _ => "",
        }
    }

    fn has_field(&self) -> bool {
        matches!(
            self,
            PromptKind::GotoLine
                | PromptKind::NewFile { .. }
                | PromptKind::Rename { .. }
                | PromptKind::NewFolder { .. }
                | PromptKind::RenameSymbol
        )
    }
}

#[derive(Clone, Debug)]
pub enum PromptEvent {
    /// Enter: the field's text (empty for a list) and the selected row.
    Confirm {
        text: String,
        index: usize,
    },
    Cancel,
}

pub struct Prompt {
    pub kind: PromptKind,
    input: Option<Entity<TextInput>>,
    /// The rows of a list prompt; a field prompt may show hints under the field.
    pub items: Vec<SharedString>,
    pub selected: usize,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    tui_scroll: RowScroll,
}

impl EventEmitter<PromptEvent> for Prompt {}

impl Focusable for Prompt {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.input {
            Some(input) => input.read(cx).focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl Prompt {
    pub fn new(
        kind: PromptKind,
        initial: &str,
        items: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = kind.has_field().then(|| {
            let placeholder = kind.placeholder();
            let input = cx.new(|cx| {
                let mut input = TextInput::new(placeholder, cx);
                // The prompt draws its own `›` cell in the tui skin.
                input.tui_prompt = false;
                input
            });
            if !initial.is_empty() {
                input.update(cx, |i, cx| i.set_text(initial, cx));
            }
            cx.subscribe(&input, Self::on_input).detach();
            input
        });
        Prompt {
            kind,
            input,
            items,
            selected: 0,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            tui_scroll: RowScroll::default(),
        }
    }

    /// Selects the field's whole text (`rename.selectOnRename`).
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        if let Some(input) = &self.input {
            input.update(cx, |i, cx| i.select_all(cx));
        }
    }

    pub fn text(&self, cx: &App) -> String {
        self.input
            .as_ref()
            .map(|i| i.read(cx).text().to_string())
            .unwrap_or_default()
    }

    fn on_input(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Submit { .. } => self.confirm(cx),
            InputEvent::Cancel => cx.emit(PromptEvent::Cancel),
            InputEvent::Changed | InputEvent::Tab { .. } => {}
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        cx.emit(PromptEvent::Confirm {
            text,
            index: self.selected,
        });
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = self.selected.saturating_sub(1);
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }
    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1).min(self.items.len() - 1);
            self.scroll.scroll_to_item(self.selected);
        }
        cx.notify();
    }
    fn confirm_action(&mut self, _: &PopupConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm(cx);
    }
    fn cancel(&mut self, _: &PopupCancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PromptEvent::Cancel);
    }
    fn backspace(&mut self, _: &PopupBackspace, _: &mut Window, _: &mut Context<Self>) {}
}

impl Render for Prompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(window, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let entity = cx.entity();
        let width = px(PROMPT_WIDTH * m.zoom);
        let viewport = window.viewport_size();
        let left = (viewport.width - width) / 2.;
        let field_focused = self
            .input
            .as_ref()
            .is_some_and(|i| i.read(cx).is_focused(window));
        let rows = self.items.iter().enumerate().map(|(ix, item)| {
            let selected = ix == self.selected;
            let entity = entity.clone();
            div()
                .id(ElementId::NamedInteger("prompt-row".into(), ix as u64))
                .h(px(PROMPT_ROW_HEIGHT * m.zoom))
                .px(px(m.popup_row_pad_x))
                .flex_shrink_0()
                .flex()
                .items_center()
                .rounded(px(m.popup_row_radius))
                .cursor_pointer()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(theme.popup_text)
                .when(selected, |d| {
                    d.bg(theme.list_bg_focused)
                        .text_color(theme.list_text_focused)
                })
                .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.selected = ix;
                        this.confirm(cx);
                    })
                })
                .child(item.clone())
        });
        let close = entity.clone();
        let list_height =
            px(PROMPT_ROW_HEIGHT * m.zoom) * self.items.len().min(PROMPT_MAX_ROWS) as f32;
        deferred(
            div()
                .id("prompt")
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::confirm_action))
                .on_action(cx.listener(Self::cancel))
                .on_action(cx.listener(Self::backspace))
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |_, cx| cx.emit(PromptEvent::Cancel));
                })
                .occlude()
                .absolute()
                .left(left)
                .top(px(PROMPT_TOP * m.zoom))
                .w(width)
                .flex()
                .flex_col()
                .p(px(m.popup_pad))
                .gap(px(m.popup_pad))
                .rounded(px(m.popup_radius))
                .border_1()
                .border_color(theme.popup_border)
                .bg(theme.popup_bg)
                .font_family(theme.ui_font.clone())
                .text_size(px(m.ui_font_size))
                .text_color(theme.popup_text)
                .shadow(vec![BoxShadow {
                    color: theme.shadow_medium,
                    offset: point(px(0.), px(m.popup_shadow_offset_y)),
                    blur_radius: px(m.popup_shadow_blur),
                    spread_radius: px(0.),
                }])
                .child(
                    div()
                        .px(px(m.popup_row_pad_x))
                        .text_color(theme.text_secondary)
                        .child(self.kind.title()),
                )
                .children(self.input.clone().map(|input| {
                    div()
                        .h(px(PROMPT_FIELD_HEIGHT * m.zoom))
                        .flex_shrink_0()
                        .px(px(m.find_bar_pad_left))
                        .flex()
                        .items_center()
                        .rounded(px(m.input_radius))
                        .bg(theme.input_bg)
                        .map(|d| {
                            if field_focused {
                                d.border(px(m.input_focus_border))
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
                        .child(div().flex_1().min_w_0().h_full().child(input))
                }))
                .when(!self.items.is_empty(), |d| {
                    d.child(
                        div().relative().h(list_height).child(
                            div()
                                .id("prompt-list")
                                .size_full()
                                .overflow_y_scroll()
                                .track_scroll(&self.scroll)
                                .flex()
                                .flex_col()
                                .children(rows),
                        ),
                    )
                }),
        )
        .into_any_element()
    }
}

// ---- the tui skin ---------------------------------------------------------------

impl Prompt {
    /// A box under the toolbar (`docs/tui-skin.md`, Prompts): the title set
    /// into the top edge, the field as a `›` row, a rule joining the frame,
    /// the rows, and the keys in the bottom edge.
    fn render_tui(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let bg = theme.popup_bg;
        let cols = tui_cols(PROMPT_WIDTH * m.zoom, cell, window);
        let total = self.items.len();
        let visible = total.min(PROMPT_MAX_ROWS);
        self.tui_scroll.follow(self.selected, total, visible);
        let top = self.tui_scroll.top;
        let field = self.input.clone();
        let field_rows = usize::from(field.is_some());
        // The rule between the field and the rows.
        let rule = field.is_some() && visible > 0;
        let rows = 2 + field_rows + usize::from(rule) + visible;
        let entity = cx.entity();
        let items: Vec<gpui::AnyElement> = self
            .items
            .iter()
            .enumerate()
            .skip(top)
            .take(visible)
            .map(|(ix, item)| {
                let entity = entity.clone();
                tui_row(
                    ElementId::NamedInteger("prompt-row".into(), ix as u64),
                    TuiRow::text(item.clone()),
                    cols - 2,
                    ix == self.selected,
                    cell,
                    &theme,
                )
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.selected = ix;
                        this.confirm(cx);
                    })
                })
                .into_any_element()
            })
            .collect();
        let focused = field
            .as_ref()
            .is_some_and(|i| i.read(cx).is_focused(window));
        let body = div()
            .size_full()
            .flex()
            .flex_col()
            .children(
                field.map(|input| tui_input_row("›", focused, input, Vec::new(), &theme, cell)),
            )
            .when(rule, |d| d.child(div().h(px(cell.h)).flex_shrink_0()))
            .child(
                div()
                    .id("prompt-list")
                    .flex()
                    .flex_col()
                    .on_scroll_wheel(cx.listener(move |this, e, _, cx| {
                        if this.tui_scroll.wheel(e, cell, total, visible) {
                            cx.notify();
                        }
                    }))
                    .children(items),
            )
            .into_any_element();
        let hints: &[(&str, &str)] = if self.kind.has_field() {
            &[("↵", "ok"), ("esc", "cancel")]
        } else {
            &[("↵", "choose"), ("esc", "cancel")]
        };
        let close = entity.clone();
        let list_top = 1 + field_rows + usize::from(rule);
        deferred(
            div()
                .id("prompt")
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::confirm_action))
                .on_action(cx.listener(Self::cancel))
                .on_action(cx.listener(Self::backspace))
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |_, cx| cx.emit(PromptEvent::Cancel));
                })
                .absolute()
                .left(skin::centred_x(window.viewport_size().width, cols, cell))
                .top(cell.rows(tui_top(cell, &m)))
                .child(
                    skin::boxed(
                        cell,
                        cols,
                        rows,
                        vec![skin::title(self.kind.title(), theme.text_primary)],
                        if rule {
                            vec![1 + field_rows]
                        } else {
                            Vec::new()
                        },
                        body,
                        bg,
                        theme.border_focused,
                        &theme,
                        m,
                    )
                    .occlude()
                    .child(skin::edge_strip(
                        cell,
                        rows - 1,
                        2,
                        cols.saturating_sub(4),
                        skin::hints(hints, &theme),
                        bg,
                    ))
                    .when(visible > 0, |d| {
                        d.child(tui_scrollbar(
                            cell,
                            cols - 1,
                            list_top,
                            rows - 1,
                            top,
                            visible,
                            total,
                            &theme,
                            m,
                        ))
                    }),
                ),
        )
        .into_any_element()
    }
}
