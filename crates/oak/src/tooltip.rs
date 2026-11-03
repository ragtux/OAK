//! Fleet's tooltip: `tooltip.background` with 8 px padding, 13 px text on an
//! 18 px line, an optional second line (a shortcut, a path) in
//! `tooltip.text.secondary`; measured in batch 3 (`docs/fleet-probe.md`).

use crate::theme::{Metrics, Theme};
use gpui::{div, prelude::*, px, AnyView, App, BoxShadow, Context, Render, SharedString, Window};

pub struct Tooltip {
    title: SharedString,
    detail: Option<SharedString>,
    /// A chord after the title on the same line, in the secondary colour
    /// ("Open Problems Ctrl+E", batch 11).
    shortcut: Option<SharedString>,
}

impl Tooltip {
    /// A builder for `.tooltip(...)`: one line of text.
    pub fn text(title: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let title = title.into();
        move |_, cx| {
            let title = title.clone();
            cx.new(|_| Tooltip {
                title,
                detail: None,
                shortcut: None,
            })
            .into()
        }
    }

    /// One line: the action's name, then its chord dimmed (the problems
    /// widget's tooltips).
    pub fn with_shortcut(
        title: impl Into<SharedString>,
        shortcut: impl Into<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let title = title.into();
        let shortcut = shortcut.into();
        move |_, cx| {
            let (title, shortcut) = (title.clone(), shortcut.clone());
            cx.new(|_| Tooltip {
                title,
                detail: None,
                shortcut: Some(shortcut),
            })
            .into()
        }
    }

    /// Two lines: the action's name and its shortcut, Fleet's toolbar tooltip.
    pub fn with_detail(
        title: impl Into<SharedString>,
        detail: impl Into<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let title = title.into();
        let detail = detail.into();
        move |_, cx| {
            let (title, detail) = (title.clone(), detail.clone());
            cx.new(|_| Tooltip {
                title,
                detail: Some(detail),
                shortcut: None,
            })
            .into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        // gpui places the view's top-left at the pointer; Fleet's box starts 16 px
        // lower, so a transparent spacer leads.
        div()
            .flex()
            .flex_col()
            .child(div().h(px(m.tooltip_offset_y)))
            .child(
                div()
                    .max_w(px(m.tooltip_max_width))
                    .px(px(m.tooltip_pad_x))
                    .py(px(m.tooltip_pad_y))
                    .rounded(px(m.tooltip_radius))
                    .bg(theme.tooltip_bg)
                    .border_1()
                    .border_color(theme.tooltip_border)
                    .shadow(vec![BoxShadow {
                        color: theme.shadow_medium,
                        offset: gpui::point(px(0.), px(m.popup_shadow_offset_y)),
                        blur_radius: px(m.popup_shadow_blur),
                        spread_radius: px(0.),
                    }])
                    .font_family(theme.ui_font.clone())
                    .text_size(px(m.ui_font_size))
                    .line_height(px(m.tooltip_line))
                    .text_color(theme.tooltip_text)
                    .child(
                        div()
                            .line_height(px(m.tooltip_line))
                            .flex()
                            .items_center()
                            .gap(px(m.tooltip_shortcut_gap))
                            .whitespace_nowrap()
                            .child(self.title.clone())
                            .children(
                                self.shortcut.clone().map(|s| {
                                    div().text_color(theme.tooltip_text_secondary).child(s)
                                }),
                            ),
                    )
                    .children(self.detail.clone().map(|d| {
                        div()
                            .line_height(px(m.tooltip_line))
                            .text_color(theme.tooltip_text_secondary)
                            .child(d)
                    })),
            )
    }
}
