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
/// thumb's width.
pub fn scrollbar(
    id: impl Into<ElementId>,
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
    let id: ElementId = id.into();
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
