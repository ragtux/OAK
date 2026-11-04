//! Fleet's scrollbar on a scrolling list: a 6 px thumb flush with the
//! surface's right edge, no track until hovered, draggable. The editor paints
//! its own (same look) because its scrolling is not a gpui scroll handle.

use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, Context, DragMoveEvent, ElementId, Pixels, Point, Render, ScrollHandle,
    Window,
};
use std::cell::Cell;

/// The value carried while the thumb is dragged: the handle to move and where
/// in the thumb the pointer grabbed it (set when the drag starts).
struct ThumbDrag {
    /// Which scrollbar: the element id it was given.
    id: ElementId,
    handle: ScrollHandle,
    grab: Cell<Pixels>,
}

pub struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The scrollbar for `handle`, to be placed as an absolutely positioned child
/// of the scrolling element's parent (which must be `relative`). `width` is the
/// thumb's width. In the tui skin it is a column one cell wide at the right
/// edge, the thumb in whole rows (`skin::paint_scrollbar`).
pub fn scrollbar(
    id: impl Into<ElementId>,
    handle: &ScrollHandle,
    width: f32,
    theme: &Theme,
    m: &Metrics,
) -> impl IntoElement {
    let id: ElementId = id.into();
    let handle = handle.clone();
    let theme = theme.clone();
    let m = *m;
    crate::skin::with_window(move |_, cx| {
        if crate::skin::Skin::current(cx) == crate::skin::Skin::Tui {
            let cell = crate::skin::Cell::current(cx);
            tui_scrollbar(id, &handle, cell, &theme, &m).into_any_element()
        } else {
            fleet_scrollbar(id, &handle, width, &theme, &m).into_any_element()
        }
    })
}

/// The tui skin's scrollbar: a one-cell column, the thumb in whole rows,
/// dragged as Fleet's is.
fn tui_scrollbar(
    id: ElementId,
    handle: &ScrollHandle,
    cell: crate::skin::Cell,
    theme: &Theme,
    m: &Metrics,
) -> impl IntoElement {
    let bounds = handle.bounds();
    let max = handle.max_offset();
    let track = bounds.size.height;
    let visible = max.y > px(0.) && track > px(0.);
    let rows = crate::skin::Cell::fit(track, cell.h);
    let (total, seen, offset) = (
        f32::from(track + max.y),
        f32::from(track),
        f32::from(-handle.offset().y),
    );
    // As `paint_scrollbar` places it: whole rows, at least one.
    let len = if visible && rows > 0 {
        ((rows as f32 * seen / total).round() as usize).clamp(1, rows)
    } else {
        0
    };
    let room = rows.saturating_sub(len);
    let start = if visible {
        ((offset / f32::from(max.y)).clamp(0., 1.) * room as f32).round() as usize
    } else {
        0
    };
    let scrollable = cell.rows(room);
    let thumb = if visible {
        theme.scrollbar_thumb
    } else {
        gpui::transparent_black()
    };
    let m = *m;
    let this_id = id.clone();
    let drag_id = id.clone();
    div()
        .id(id)
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .w(px(cell.w))
        .when(!visible, |d| d.invisible())
        .on_drag_move(move |e: &DragMoveEvent<ThumbDrag>, _, cx| {
            let drag = e.drag(cx);
            if drag.id != this_id || scrollable <= px(0.) {
                return;
            }
            let top = e.event.position.y - e.bounds.origin.y - drag.grab.get();
            let fraction = (top / scrollable).clamp(0., 1.);
            drag.handle
                .set_offset(Point::new(px(0.), -max.y * fraction));
        })
        .child(
            gpui::canvas(
                |_, _, _| (),
                move |b, _, window, _| {
                    crate::skin::paint_scrollbar(
                        window, b, cell, offset, seen, total, None, thumb, &m,
                    );
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .child(
            div()
                .id("thumb")
                .absolute()
                .top(cell.rows(start))
                .left_0()
                .w(px(cell.w))
                .h(cell.rows(len))
                .on_drag(
                    ThumbDrag {
                        id: drag_id,
                        handle: handle.clone(),
                        grab: Cell::new(px(0.)),
                    },
                    |drag: &ThumbDrag, grab: Point<Pixels>, _window, cx| {
                        drag.grab.set(grab.y);
                        cx.new(|_| EmptyView)
                    },
                ),
        )
}

/// Fleet's scrollbar (the reference).
fn fleet_scrollbar(
    id: ElementId,
    handle: &ScrollHandle,
    width: f32,
    theme: &Theme,
    m: &Metrics,
) -> impl IntoElement {
    let bounds = handle.bounds();
    let max = handle.max_offset();
    let offset = handle.offset();
    let track = bounds.size.height;
    let content = track + max.y;
    let visible = max.y > px(0.) && track > px(0.);
    let thumb_len = if visible {
        (track * (track / content))
            .max(px(m.scrollbar_thumb_min))
            .min(track)
    } else {
        px(0.)
    };
    let scrollable = (track - thumb_len).max(px(0.));
    let fraction = if max.y > px(0.) {
        (-offset.y / max.y).clamp(0., 1.)
    } else {
        0.
    };
    let thumb_top = scrollable * fraction;
    let this_id = id.clone();
    let drag_id = id.clone();
    div()
        .id(id)
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .w(px(width))
        .when(!visible, |d| d.invisible())
        .hover(|s| s.bg(theme.scrollbar_track_hovered))
        .on_drag_move(move |e: &DragMoveEvent<ThumbDrag>, _, cx| {
            let drag = e.drag(cx);
            if drag.id != this_id || scrollable <= px(0.) {
                return;
            }
            let top = e.event.position.y - e.bounds.origin.y - drag.grab.get();
            let fraction = (top / scrollable).clamp(0., 1.);
            drag.handle
                .set_offset(Point::new(px(0.), -max.y * fraction));
        })
        .child(
            div()
                .id("thumb")
                .absolute()
                .top(thumb_top)
                .right_0()
                .w(px(width))
                .h(thumb_len)
                .rounded(px(width / 2.))
                .bg(theme.scrollbar_thumb)
                .hover(|s| s.bg(theme.scrollbar_thumb_hovered))
                .active(|s| s.bg(theme.scrollbar_thumb_pressed))
                .on_drag(
                    ThumbDrag {
                        id: drag_id,
                        handle: handle.clone(),
                        grab: Cell::new(px(0.)),
                    },
                    |drag: &ThumbDrag, grab: Point<Pixels>, _window, cx| {
                        drag.grab.set(grab.y);
                        cx.new(|_| EmptyView)
                    },
                ),
        )
}
