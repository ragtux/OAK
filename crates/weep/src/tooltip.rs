//! Fleet's tooltip: `tooltip.background` with 8 px padding, 13 px text on an
//! 18 px line, an optional second line (a shortcut, a path) in
//! `tooltip.text.secondary`; measured in batch 3 (`docs/fleet-probe.md`).

use crate::theme::{Metrics, Theme};
use gpui::{
    div, point, prelude::*, px, AnyView, App, BoxShadow, Context, Pixels, Point, Render,
    SharedString, Window,
};

pub struct Tooltip {
    title: SharedString,
    detail: Option<SharedString>,
    /// A chord after the title on the same line, in the secondary colour
    /// ("Open Problems Ctrl+E", batch 11).
    shortcut: Option<SharedString>,
    /// The pointer when the tooltip was asked for: gpui places the view
    /// there, and the tui skin moves its cells onto the window's grid.
    pointer: Point<Pixels>,
}

impl Tooltip {
    /// A builder for `.tooltip(...)`: one line of text.
    pub fn text(title: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let title = title.into();
        move |window, cx| {
            let title = title.clone();
            let pointer = window.mouse_position();
            cx.new(|_| Tooltip {
                title,
                detail: None,
                shortcut: None,
                pointer,
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
        move |window, cx| {
            let (title, shortcut) = (title.clone(), shortcut.clone());
            let pointer = window.mouse_position();
            cx.new(|_| Tooltip {
                title,
                detail: None,
                shortcut: Some(shortcut),
                pointer,
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
        move |window, cx| {
            let (title, detail) = (title.clone(), detail.clone());
            let pointer = window.mouse_position();
            cx.new(|_| Tooltip {
                title,
                detail: Some(detail),
                shortcut: None,
                pointer,
            })
            .into()
        }
    }
}

impl Tooltip {
    /// The tooltip in the tui skin (`docs/tui-skin.md`, Tooltips), on whole
    /// cells of the window's grid next to the pointer's target: one line is
    /// a row of cells on `tooltip.background` with a cell of margin each
    /// side and no frame; two lines are a box.
    fn render_tui(&self, window: &Window, cx: &App) -> gpui::AnyElement {
        use crate::skin::{self, Cell};
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let line = |title: SharedString, shortcut: Option<SharedString>| {
            div()
                .h(px(cell.h))
                .flex()
                .whitespace_nowrap()
                .text_color(theme.tooltip_text)
                .child(title)
                .children(shortcut.map(|s| {
                    div()
                        .pl(cell.cols(2))
                        .text_color(theme.tooltip_text_secondary)
                        .child(s)
                }))
        };
        let first = Cell::text_cols(&self.title)
            + self
                .shortcut
                .as_ref()
                .map(|s| 2 + Cell::text_cols(s))
                .unwrap_or(0);
        let (content, cols, rows) = match self.detail.clone() {
            None => (
                line(self.title.clone(), self.shortcut.clone())
                    .w(cell.cols(first + 2))
                    .px(px(cell.w))
                    .bg(theme.tooltip_bg)
                    .font_family(theme.mono_font.clone())
                    .text_size(px(cell.font_size))
                    .line_height(px(cell.h))
                    .into_any_element(),
                first + 2,
                1,
            ),
            Some(detail) => {
                let cols = first.max(Cell::text_cols(&detail)) + 4;
                let body = div()
                    .size_full()
                    .px(px(cell.w))
                    .flex()
                    .flex_col()
                    .child(line(self.title.clone(), self.shortcut.clone()))
                    .child(
                        div()
                            .h(px(cell.h))
                            .whitespace_nowrap()
                            .text_color(theme.tooltip_text_secondary)
                            .child(detail),
                    )
                    .into_any_element();
                let boxed = skin::boxed(
                    cell,
                    cols,
                    4,
                    Vec::new(),
                    Vec::new(),
                    body,
                    theme.tooltip_bg,
                    theme.tooltip_text_secondary,
                    &theme,
                    m,
                );
                (boxed.into_any_element(), cols, 4)
            }
        };
        let viewport = window.viewport_size();
        let size = gpui::size(cell.cols(cols), cell.rows(rows));
        let at = tui_place(self.pointer, size, viewport, cell, m.tooltip_offset_y);
        let origin = gpui_origin(self.pointer, size, viewport);
        // The view is the tooltip's size, so gpui's placement is known; the
        // cells are laid where they belong from there.
        div()
            .w(size.width)
            .h(size.height)
            .child(
                div()
                    .absolute()
                    .left(at.x - origin.x)
                    .top(at.y - origin.y)
                    .child(content),
            )
            .into_any_element()
    }
}

/// Where a tooltip of `size` sits in the tui skin: from the pointer's
/// column, on the first row at least `gap` below the pointer (Fleet's
/// offset), else ending on the row above the pointer's; inside the window
/// and on whole cells of its grid.
fn tui_place(
    pointer: Point<Pixels>,
    size: gpui::Size<Pixels>,
    viewport: gpui::Size<Pixels>,
    cell: crate::skin::Cell,
    gap: f32,
) -> Point<Pixels> {
    use crate::skin::Cell;
    let all_cols = Cell::fit(viewport.width, cell.w);
    let all_rows = Cell::fit(viewport.height, cell.h);
    let cols = Cell::fit(size.width, cell.w);
    let rows = Cell::fit(size.height, cell.h);
    let x = f32::from(pointer.x).max(0.);
    let y = f32::from(pointer.y).max(0.);
    let col = ((x / cell.w).floor() as usize).min(all_cols.saturating_sub(cols));
    let below = ((y + gap) / cell.h).ceil() as usize;
    let row = if below + rows <= all_rows {
        below
    } else {
        ((y / cell.h).floor() as usize).saturating_sub(rows)
    };
    point(cell.cols(col), cell.rows(row))
}

/// Where gpui puts a tooltip view of `size` asked for at `pointer`
/// (`Window::prepaint_tooltip`): a pixel past the pointer, flipped to its
/// other side where the window ends.
fn gpui_origin(
    pointer: Point<Pixels>,
    size: gpui::Size<Pixels>,
    viewport: gpui::Size<Pixels>,
) -> Point<Pixels> {
    let axis = |p: Pixels, extent: Pixels, end: Pixels| {
        let start = p + px(1.);
        if start + extent <= end {
            start
        } else if p - extent - px(1.) >= px(0.) {
            p - extent - px(1.)
        } else {
            px(0.)
        }
    };
    point(
        axis(pointer.x, size.width, viewport.width),
        axis(pointer.y, size.height, viewport.height),
    )
}

/// Set while a context menu is open: tooltips draw nothing then, as in
/// Fleet, where the row a menu was opened on shows no tooltip over the menu
/// (debug-rest replay: the Breakpoints tool row's tooltip covered its menu).
pub struct MenuShown(pub bool);

impl gpui::Global for MenuShown {}

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if cx.try_global::<MenuShown>().is_some_and(|m| m.0) {
            return div().into_any_element();
        }
        if crate::skin::Skin::current(cx) == crate::skin::Skin::Tui {
            return self.render_tui(window, cx);
        }
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
            .into_any_element()
    }
}
