//! What the pointer drags, on the tui skin's grid (`docs/tui-skin.md`, Drag
//! and drop): the dragged tab's or file's ghost is a one-row box that
//! follows the pointer in whole cells. gpui paints a drag's view at the
//! pointer less where the drag started inside its element; this view wraps
//! the ghost and moves it from there onto the window's grid. In the Fleet
//! skin it is the ghost itself, unmoved.

use crate::skin::{Cell, Skin};
use gpui::{div, prelude::*, px, AnyView, App, Context, Entity, Pixels, Point, Render, Window};

pub struct SnappedGhost {
    ghost: AnyView,
    /// The pointer's offset inside the dragged element when the drag began.
    cursor_offset: Point<Pixels>,
}

/// The ghost for an `on_drag` constructor, given the cursor offset gpui
/// hands it.
pub fn snapped(
    cursor_offset: Point<Pixels>,
    ghost: impl Into<AnyView>,
    cx: &mut App,
) -> Entity<SnappedGhost> {
    let ghost = ghost.into();
    cx.new(|_| SnappedGhost {
        ghost,
        cursor_offset,
    })
}

impl Render for SnappedGhost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Skin::current(cx) != Skin::Tui {
            return self.ghost.clone().into_any_element();
        }
        let cell = Cell::current(cx);
        // Where gpui puts this view, and the cell nearest it; the box's
        // body row lands on the dragged element's row, so its frame starts a
        // row up and a column left.
        let origin = window.mouse_position() - self.cursor_offset;
        let snap = |v: Pixels, pitch: f32| (f32::from(v) / pitch).round() * pitch;
        let x = (snap(origin.x, cell.w) - cell.w).max(0.);
        let y = (snap(origin.y, cell.h) - cell.h).max(0.);
        div()
            .child(
                div()
                    .absolute()
                    .left(px(x) - origin.x)
                    .top(px(y) - origin.y)
                    .child(self.ghost.clone()),
            )
            .into_any_element()
    }
}
