//! A list in the Goto popup's frame (M7): the usages of a symbol
//! (`find-usages`) and the targets of a declaration with several, each a
//! row with an icon, a text and a right-aligned `file:line`; Up and Down
//! move, Enter opens, Escape closes, a click opens. The frame and the rows
//! are the Goto panel's measurements (batch 6); that Fleet lists usages in
//! this frame is an estimate (batch 12 has not run).

use crate::actions::{PopupCancel, PopupConfirm, PopupDown, PopupPageDown, PopupPageUp, PopupUp};
use crate::assets::icon;
use crate::theme::{Metrics, Theme};
use gpui::{
    deferred, div, point, prelude::*, px, App, BoxShadow, Context, ElementId, EventEmitter,
    FocusHandle, Focusable, FontWeight, Render, ScrollHandle, SharedString, Window,
};

#[derive(Clone, Debug)]
pub struct ListRow {
    pub icon: Option<&'static str>,
    pub text: SharedString,
    /// After the text, in the secondary colour (a usage's line).
    pub detail: Option<SharedString>,
    /// Right-aligned, in the secondary colour (`file:line`).
    pub right: Option<SharedString>,
}

#[derive(Clone, Debug)]
pub enum ListPopupEvent {
    Open(usize),
    Cancel,
}

pub struct ListPopup {
    title: SharedString,
    rows: Vec<ListRow>,
    selected: usize,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<ListPopupEvent> for ListPopup {}

impl Focusable for ListPopup {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ListPopup {
    pub fn new(title: impl Into<SharedString>, rows: Vec<ListRow>, cx: &mut Context<Self>) -> Self {
        ListPopup {
            title: title.into(),
            rows,
            selected: 0,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    fn step(&mut self, delta: isize, wrap: bool, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        let next = if wrap {
            (self.selected as isize + delta).rem_euclid(len)
        } else {
            (self.selected as isize + delta).clamp(0, len - 1)
        };
        self.selected = next as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, true, cx);
    }

    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, true, cx);
    }

    fn page_up(&mut self, _: &PopupPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-10, false, cx);
    }

    fn page_down(&mut self, _: &PopupPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(10, false, cx);
    }

    fn confirm(&mut self, _: &PopupConfirm, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected < self.rows.len() {
            cx.emit(ListPopupEvent::Open(self.selected));
        }
    }

    fn cancel(&mut self, _: &PopupCancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ListPopupEvent::Cancel);
    }
}

impl Render for ListPopup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let selected = self.selected;
        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                let is_selected = ix == selected;
                div()
                    .id(ElementId::NamedInteger("list-row".into(), ix as u64))
                    .h(px(m.goto_row_pitch))
                    .flex()
                    .items_center()
                    .px(px(m.popup_row_pad_x))
                    .rounded(px(m.popup_row_radius))
                    .cursor_pointer()
                    .when(is_selected, |d| d.bg(theme.list_bg_focused))
                    .when(!is_selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = ix;
                        cx.emit(ListPopupEvent::Open(ix));
                    }))
                    .children(row.icon.map(|path| {
                        div()
                            .flex_shrink_0()
                            .mr(px(m.popup_row_icon_gap))
                            .child(icon(path, m.icon_size, theme.icon_secondary))
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(if is_selected {
                                theme.list_text_focused
                            } else {
                                theme.list_text
                            })
                            .child(row.text.clone()),
                    )
                    .children(row.detail.clone().map(|d| {
                        div()
                            .ml(px(m.popup_row_icon_gap))
                            .min_w_0()
                            .truncate()
                            .text_color(theme.list_text_secondary)
                            .child(d)
                    }))
                    .child(div().flex_1().min_w(px(m.popup_row_icon_gap)))
                    .children(row.right.clone().map(|r| {
                        div()
                            .flex_shrink_0()
                            .text_color(theme.list_text_secondary)
                            .child(r)
                    }))
            })
            .collect();
        let count: SharedString = format!("{}", self.rows.len()).into();
        deferred(
            div()
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::page_up))
                .on_action(cx.listener(Self::page_down))
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::cancel))
                .absolute()
                .top(px(m.goto_top))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .occlude()
                        .w(px(m.goto_width))
                        .max_h(px(m.goto_max_height))
                        .flex()
                        .flex_col()
                        .rounded(px(m.popup_radius))
                        .border_1()
                        .border_color(theme.popup_border)
                        .bg(theme.popup_goto_bg)
                        .shadow(vec![BoxShadow {
                            color: theme.shadow_medium,
                            offset: point(px(0.), px(m.popup_shadow_offset_y)),
                            blur_radius: px(m.popup_shadow_blur),
                            spread_radius: px(0.),
                        }])
                        .font_family(theme.ui_font.clone())
                        .text_size(px(m.ui_font_size))
                        .text_color(theme.popup_text)
                        .child(
                            div()
                                .h(px(m.goto_header_height))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .px(px(m.goto_pad))
                                .pt(px(m.goto_tab_top))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(self.title.clone())
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .font_weight(FontWeight::NORMAL)
                                        .text_color(theme.text_secondary)
                                        .child(count),
                                ),
                        )
                        .child(
                            div()
                                .id("list-rows")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .track_scroll(&self.scroll)
                                .px(px(m.goto_pad))
                                .pb(px(m.goto_bottom_pad))
                                .children(rows),
                        ),
                ),
        )
    }
}
