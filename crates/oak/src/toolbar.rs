//! The row above the islands: Fleet's top bar. Left to right: the logo,
//! the main menu, the three panel toggles, a separator, the tools grid and
//! the collaboration button; the workspace name in the middle; the smart
//! mode, run, search, notifications and settings buttons on the right.

use crate::assets::icon;
use crate::theme::{metrics::*, Theme};
use gpui::{div, prelude::*, px, App, Div, ElementId, Hsla, SharedString, Stateful, Window};

/// A 28 px icon button with a hover wash: Fleet's ghost button.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon_path: &str,
    theme: &Theme,
    active: bool,
) -> Stateful<Div> {
    // Fleet draws an open panel's toggle no differently from a closed one.
    let _ = active;
    let color = theme.icon_primary;
    div()
        .id(id)
        .size(px(TOOLBAR_BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(theme.ghost_hovered))
        .active(|s| s.bg(theme.ghost_pressed))
        .child(icon(icon_path, ICON_SIZE, color))
}

pub fn separator(theme: &Theme) -> Div {
    div().w(px(1.)).h(px(16.)).mx(px(6.)).bg(theme.separator)
}

pub struct ToolbarState {
    pub workspace_name: SharedString,
    pub left_open: bool,
    pub bottom_open: bool,
    pub right_open: bool,
}

/// Renders the bar; the toggles call back through `on_toggle(index)`
/// with 0, 1, 2 for the left, bottom and right panel.
pub fn render(
    state: ToolbarState,
    theme: &Theme,
    on_toggle: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
) -> impl IntoElement {
    let toggle = |ix: usize, path: &str, open: bool| {
        let f = on_toggle.clone();
        icon_button(("panel-toggle", ix), path, theme, open)
            .on_click(move |_, window, cx| f(ix, window, cx))
    };
    let name_color: Hsla = theme.text_primary;
    div()
        .h(px(TOOLBAR_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .px(px(TOOLBAR_PAD_X))
        .gap(px(TOOLBAR_GAP))
        .child(
            div()
                .size(px(TOOLBAR_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("oak/logo", LOGO_SIZE, theme.icon_primary)),
        )
        .child(icon_button("menu", "expui/general/menu", theme, false))
        .child(toggle(0, "oak/panel-left", state.left_open))
        .child(toggle(1, "oak/panel-bottom", state.bottom_open))
        .child(toggle(2, "oak/panel-right", state.right_open))
        .child(separator(theme))
        .child(icon_button("tools", "oak/grid", theme, false))
        .child(icon_button("collab", "oak/user-add", theme, false))
        .child(
            div().flex_1().min_w_0().flex().justify_center().child(
                div()
                    .id("workspace-name")
                    .max_w(px(360.))
                    .px(px(8.))
                    .h(px(TOOLBAR_BUTTON))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.ghost_hovered))
                    .text_color(name_color)
                    .child(div().truncate().child(state.workspace_name)),
            ),
        )
        .child(icon_button(
            "smart-mode",
            "expui/actions/lightning",
            theme,
            false,
        ))
        .child(icon_button("run", "expui/run/run", theme, false))
        .child(icon_button("search", "expui/general/search", theme, false))
        .child(icon_button(
            "notifications",
            "expui/toolwindows/notifications",
            theme,
            false,
        ))
        .child(icon_button(
            "settings",
            "expui/general/settings",
            theme,
            false,
        ))
}
