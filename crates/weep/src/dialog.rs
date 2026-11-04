//! Fleet's confirm dialog (`confirm-dialog`, `alternative-confirm`): a popup
//! centred in the window with a title, a message and a row of buttons
//! (`button.primary.*`, `button.secondary.*`, `button.dangerous.*`). Enter
//! presses the focused button (the first one at first), Escape cancels, the
//! arrows and Tab move between buttons. Used for `files/delete`, the big-file
//! question, `warn.before.quitting` and unsaved changes. Batch 7 measures it;
//! the sizes here are estimates (`theme.rs::metrics`).

use crate::actions::{PopupBackTab, PopupCancel, PopupConfirm, PopupLeft, PopupRight, PopupTab};
use crate::theme::{Metrics, Theme};
use gpui::{
    deferred, div, point, prelude::*, px, App, BoxShadow, Context, ElementId, EventEmitter,
    FocusHandle, Focusable, FontWeight, SharedString, Window,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    /// Kept for `git/revert` with `git.revert.confirm` (M4); Fleet's file Delete

    /// has no dialog (ninth pass).

    #[allow(dead_code)]
    Dangerous,
}

#[derive(Clone, Debug)]
pub struct DialogButton {
    pub label: SharedString,
    pub kind: ButtonKind,
}

impl DialogButton {
    pub fn primary(label: impl Into<SharedString>) -> DialogButton {
        DialogButton {
            label: label.into(),
            kind: ButtonKind::Primary,
        }
    }
    pub fn secondary(label: impl Into<SharedString>) -> DialogButton {
        DialogButton {
            label: label.into(),
            kind: ButtonKind::Secondary,
        }
    }
    #[allow(dead_code)]
    pub fn dangerous(label: impl Into<SharedString>) -> DialogButton {
        DialogButton {
            label: label.into(),
            kind: ButtonKind::Dangerous,
        }
    }
}

#[derive(Clone, Debug)]
pub enum DialogEvent {
    /// The index of the button pressed.
    Chosen(usize),
    Cancelled,
}

pub struct ConfirmDialog {
    title: SharedString,
    message: Option<SharedString>,
    buttons: Vec<DialogButton>,
    focused: usize,
    focus_handle: FocusHandle,
}

impl EventEmitter<DialogEvent> for ConfirmDialog {}

impl Focusable for ConfirmDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ConfirmDialog {
    pub fn new(
        title: impl Into<SharedString>,
        message: Option<SharedString>,
        buttons: Vec<DialogButton>,
        cx: &mut Context<Self>,
    ) -> Self {
        ConfirmDialog {
            title: title.into(),
            message,
            buttons,
            focused: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    fn choose(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.buttons.len() {
            cx.emit(DialogEvent::Chosen(ix));
        }
    }

    fn move_focus(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.buttons.len() as isize;
        if n > 0 {
            self.focused = ((self.focused as isize + delta).rem_euclid(n)) as usize;
            cx.notify();
        }
    }

    fn confirm(&mut self, _: &PopupConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.choose(self.focused, cx);
    }
    fn cancel(&mut self, _: &PopupCancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DialogEvent::Cancelled);
    }
    fn left(&mut self, _: &PopupLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(-1, cx);
    }
    fn right(&mut self, _: &PopupRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(1, cx);
    }
    fn tab(&mut self, _: &PopupTab, _: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(1, cx);
    }
    fn back_tab(&mut self, _: &PopupBackTab, _: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(-1, cx);
    }
}

/// A Fleet button: 24 tall, `button.<kind>.*` colours, the focus ring when
/// it holds the keyboard focus (batch 2 measured the secondary one).
pub fn button(
    id: impl Into<ElementId>,
    label: SharedString,
    kind: ButtonKind,
    focused: bool,
    theme: &Theme,
    m: &Metrics,
) -> gpui::Stateful<gpui::Div> {
    let (bg, bg_hovered, text, border) = match kind {
        ButtonKind::Primary => (
            theme.button_primary_bg,
            theme.button_primary_bg_hovered,
            theme.button_primary_text,
            gpui::transparent_black(),
        ),
        ButtonKind::Secondary => (
            theme.button_secondary_bg,
            theme.button_secondary_bg_hovered,
            theme.button_secondary_text,
            theme.button_secondary_border,
        ),
        ButtonKind::Dangerous => (
            theme.button_dangerous_bg,
            theme.button_dangerous_bg_hovered,
            theme.button_dangerous_text,
            gpui::transparent_black(),
        ),
    };
    div()
        .id(id)
        .h(px(m.button_height))
        .px(px(m.text_button_pad_x + m.button_pad_x))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(m.button_radius))
        .cursor_pointer()
        .bg(bg)
        .text_color(text)
        .border_1()
        .border_color(if focused {
            theme.border_focused
        } else {
            border
        })
        .when(focused, |d| {
            d.shadow(vec![BoxShadow {
                color: theme.focus_outline,
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(m.focus_outline),
            }])
        })
        .hover(move |s| s.bg(bg_hovered))
        .whitespace_nowrap()
        .child(label)
}

/// A button in the tui skin: `skin::button`'s `[ Label ]`, a dangerous one
/// filled with `button.dangerous.*`, and the one holding the keyboard focus
/// filled as a focused list row (when it is not filled already).
pub fn tui_button(
    id: impl Into<ElementId>,
    label: &str,
    kind: ButtonKind,
    focused: bool,
    theme: &Theme,
    cell: crate::skin::Cell,
) -> gpui::Stateful<gpui::Div> {
    let b = crate::skin::button(id, label, kind == ButtonKind::Primary, theme, cell);
    match kind {
        ButtonKind::Dangerous => {
            let hovered = theme.button_dangerous_bg_hovered;
            b.bg(theme.button_dangerous_bg)
                .text_color(theme.button_dangerous_text)
                .hover(move |s| s.bg(hovered))
        }
        ButtonKind::Secondary if focused => b
            .bg(theme.list_bg_focused)
            .text_color(theme.list_text_focused),
        _ => b,
    }
}

impl ConfirmDialog {
    /// The dialog in the tui skin: a box centred on the window's grid, the
    /// title set into its top edge, the message wrapped on the grid, a blank
    /// row and the buttons right-aligned.
    fn render_tui(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        use crate::skin::{self, Cell};
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let entity = cx.entity();
        let viewport = window.viewport_size();
        let win_cols = Cell::fit(viewport.width, cell.w);
        let win_rows = Cell::fit(viewport.height, cell.h);
        let labels: Vec<String> = self.buttons.iter().map(|b| b.label.to_string()).collect();
        let buttons_cols: usize = labels.iter().map(|l| Cell::text_cols(l) + 4).sum::<usize>()
            + labels.len().saturating_sub(1);
        let title_cols = Cell::text_cols(&self.title) + 2;
        let message_cols = self
            .message
            .as_ref()
            .map(|t| t.lines().map(Cell::text_cols).max().unwrap_or(0))
            .unwrap_or(0);
        // Inner: one cell of margin each side of the text and the buttons.
        let want = (buttons_cols.max(title_cols + 2).max(message_cols) + 2).max(40);
        let cols = (want.min(64) + 2).min(win_cols.saturating_sub(2)).max(8);
        let text_cols = cols.saturating_sub(4).max(1);
        let lines: Vec<String> = self
            .message
            .as_ref()
            .map(|t| skin::wrap(t, text_cols))
            .unwrap_or_default();
        // Frame, blank, message, blank, buttons, frame.
        let rows = 2 + 1 + lines.len() + usize::from(!lines.is_empty()) + 1 + 1;
        let left = cell.cols(win_cols.saturating_sub(cols) / 2);
        let top = cell.rows(win_rows.saturating_sub(rows) / 2);
        let buttons = self.buttons.iter().enumerate().map(|(ix, b)| {
            let entity = entity.clone();
            tui_button(
                ElementId::NamedInteger("dialog-button".into(), ix as u64),
                &b.label,
                b.kind,
                ix == self.focused,
                &theme,
                cell,
            )
            .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.choose(ix, cx)))
        });
        let body = div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h(px(cell.h)).flex_shrink_0())
            .children(lines.into_iter().map(|l| {
                div()
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .pl(px(cell.w))
                    .whitespace_nowrap()
                    .text_color(theme.text_secondary)
                    .child(SharedString::from(l))
            }))
            .when(self.message.is_some(), |d| {
                d.child(div().h(px(cell.h)).flex_shrink_0())
            })
            .child(
                div()
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .pr(px(cell.w))
                    .flex()
                    .justify_end()
                    .gap(px(cell.w))
                    .children(buttons),
            )
            .into_any_element();
        let frame = skin::boxed(
            cell,
            cols,
            rows,
            vec![skin::title(self.title.clone(), theme.popup_text)],
            Vec::new(),
            body,
            theme.popup_bg,
            theme.border_focused,
            &theme,
            m,
        )
        .absolute()
        .left(left)
        .top(top)
        .text_color(theme.popup_text);
        deferred(
            div()
                .id("confirm-dialog")
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::cancel))
                .on_action(cx.listener(Self::left))
                .on_action(cx.listener(Self::right))
                .on_action(cx.listener(Self::tab))
                .on_action(cx.listener(Self::back_tab))
                .occlude()
                .absolute()
                .inset_0()
                .child(frame),
        )
        .into_any_element()
    }
}

impl Render for ConfirmDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::skin::Skin::current(cx) == crate::skin::Skin::Tui {
            return self.render_tui(window, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let entity = cx.entity();
        let buttons = self.buttons.iter().enumerate().map(|(ix, b)| {
            let entity = entity.clone();
            button(
                ElementId::NamedInteger("dialog-button".into(), ix as u64),
                b.label.clone(),
                b.kind,
                ix == self.focused,
                &theme,
                &m,
            )
            .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.choose(ix, cx)))
        });
        deferred(
            div()
                .id("confirm-dialog")
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::cancel))
                .on_action(cx.listener(Self::left))
                .on_action(cx.listener(Self::right))
                .on_action(cx.listener(Self::tab))
                .on_action(cx.listener(Self::back_tab))
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .font_family(theme.ui_font.clone())
                .text_size(px(m.ui_font_size))
                .text_color(theme.popup_text)
                .child(
                    div()
                        .w(px(m.dialog_width))
                        .p(px(m.dialog_pad))
                        .flex()
                        .flex_col()
                        .rounded(px(m.popup_radius))
                        .border_1()
                        .border_color(theme.popup_border)
                        .bg(theme.popup_bg)
                        .shadow(vec![BoxShadow {
                            color: theme.shadow_medium,
                            offset: point(px(0.), px(m.popup_shadow_offset_y)),
                            blur_radius: px(m.popup_shadow_blur),
                            spread_radius: px(0.),
                        }])
                        .child(
                            div()
                                .line_height(px(m.dialog_title_line))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(self.title.clone()),
                        )
                        .children(self.message.clone().map(|message| {
                            div()
                                .mt(px(m.dialog_title_gap))
                                .text_color(theme.text_secondary)
                                .child(message)
                        }))
                        .child(
                            div()
                                .mt(px(m.dialog_buttons_gap))
                                .flex()
                                .justify_end()
                                .gap(px(m.dialog_button_gap))
                                .children(buttons),
                        ),
                ),
        )
        .into_any_element()
    }
}
