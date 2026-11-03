//! The row above the islands: Fleet's top bar. Left to right: the logo,
//! the main menu, the three panel toggles, a separator, the tools grid and
//! the collaboration button; the window title (workspace name and git
//! branch) centred on the window; the smart mode, run, search, notifications
//! and settings buttons on the right. On Windows the toolbar is also the
//! window's caption: the three caption buttons sit at its right end and the
//! empty parts drag the window (`docs/fleet-probe.md`, batch 1). On macOS
//! the toolbar is the window's title bar too: the system's traffic lights sit
//! at its left end in place of the logo and the main menu button (the main
//! menu has no button there), and the empty parts drag the window and zoom it
//! on a double click (fourteenth pass).

use crate::actions::{
    GotoEverything, GotoTool, OpenSettings, RunAndDebug, ShowMainMenu, ToggleNotifications,
};
use crate::assets::icon;
use crate::theme::{Metrics, Theme};
use crate::tooltip::Tooltip;
use gpui::{
    div, prelude::*, px, App, Div, ElementId, Hsla, MouseButton, Pixels, Point, Rgba, SharedString,
    Stateful, Window, WindowControlArea,
};
use std::cell::Cell;

thread_local! {
    /// macOS: a press on the toolbar's empty part that has not moved yet. The
    /// first move turns it into a window drag, as Zed's title bar does: a
    /// drag started on the press itself would swallow the double click.
    static MOVE_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// A 28 px icon button with a hover wash: Fleet's ghost button.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon_path: &str,
    theme: &Theme,
    m: &Metrics,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(m.toolbar_button))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4. * m.zoom))
        .cursor_pointer()
        // Above the caption's drag area, so a click is the button's.
        .occlude()
        .hover(|s| s.bg(theme.ghost_hovered))
        .active(|s| s.bg(theme.ghost_pressed))
        .child(icon(icon_path, m.icon_size, theme.icon_primary))
}

pub fn separator(theme: &Theme, m: &Metrics) -> Div {
    div()
        .w(px(1.))
        .h(px(16. * m.zoom))
        .flex_shrink_0()
        .mx(px(m.toolbar_separator_margin))
        .bg(theme.separator)
}

pub struct ToolbarState {
    pub workspace_name: SharedString,
    pub branch: Option<SharedString>,
    pub left_open: bool,
    pub bottom_open: bool,
    pub right_open: bool,
    /// Draw the caption buttons and make the bar the window's drag area.
    pub caption: bool,
    /// macOS: leave room for the traffic lights (drawn by the system) and
    /// drag the window from the bar. False in full screen, where the system
    /// hides the lights.
    pub traffic_lights: bool,
    pub maximized: bool,
    /// Shortcut hints for the toggles' tooltips, as Fleet spells them.
    pub toggle_hints: [Option<SharedString>; 3],
    /// Unread notifications: a dot on the bell (estimate: batch 7).
    pub unread_notifications: usize,
    /// M7: the lightning button's state, and whether its card is open
    /// (the button then fills with the state's colour).
    pub smart: SmartIndicator,
    pub smart_card_open: bool,
}

/// What the lightning button shows (thirteenth pass): the icon in the
/// state's colour (the primary icon colour off, `smartMode.background.enabled`
/// on or starting, `smartMode.background.error` when a server failed), and
/// while its card is open the button filled with that colour under a bright
/// icon. The tooltip reads "Smart Mode".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartIndicator {
    Off,
    Starting,
    On,
    Error,
}

/// Windows' own close-button hover colour (not a theme key: the system's).
fn caption_close_hover() -> Hsla {
    Rgba {
        r: 0xC4 as f32 / 255.,
        g: 0x2B as f32 / 255.,
        b: 0x1C as f32 / 255.,
        a: 1.,
    }
    .into()
}

fn caption_button(
    id: &'static str,
    icon_path: &str,
    area: WindowControlArea,
    theme: &Theme,
    m: &Metrics,
) -> Stateful<Div> {
    let close = matches!(area, WindowControlArea::Close);
    let hover_bg = if close {
        caption_close_hover()
    } else {
        theme.ghost_hovered
    };
    let hover_fg = if close {
        gpui::white()
    } else {
        theme.icon_primary
    };
    div()
        .id(id)
        .group(id)
        .w(px(m.caption_button_width))
        .h_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .hover(|s| s.bg(hover_bg))
        .active(|s| s.bg(hover_bg.opacity(0.8)))
        .window_control_area(area)
        .child(
            icon(icon_path, m.caption_glyph, theme.icon_primary)
                .group_hover(id, move |s| s.text_color(hover_fg)),
        )
}

/// Renders the bar; the toggles call back through `on_toggle(index)`
/// with 0, 1, 2 for the left, bottom and right panel.
pub fn render(
    state: ToolbarState,
    theme: &Theme,
    m: &Metrics,
    on_toggle: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    on_branch: impl Fn(Point<Pixels>, &mut Window, &mut App) + Clone + 'static,
    on_smart: impl Fn(Point<Pixels>, &mut Window, &mut App) + Clone + 'static,
) -> impl IntoElement {
    let toggle = |ix: usize, path: &str, label: &'static str| {
        let f = on_toggle.clone();
        let hint = state.toggle_hints[ix].clone();
        icon_button(("panel-toggle", ix), path, theme, m)
            .map(|b| match hint {
                Some(h) => b.tooltip(Tooltip::with_detail(label, h)),
                None => b.tooltip(Tooltip::text(label)),
            })
            .on_click(move |_, window, cx| f(ix, window, cx))
    };
    let _ = (state.left_open, state.bottom_open, state.right_open);
    let name_color: Hsla = theme.text_primary;
    let title = div()
        .absolute()
        .left_0()
        .right_0()
        .top_0()
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .text_color(name_color)
                .child(
                    div()
                        .max_w(px(m.title_name_max_width))
                        .truncate()
                        .child(state.workspace_name.clone()),
                )
                .children(state.branch.clone().map(|branch| {
                    // Fleet: a tooltip "Current Branch: main" and a click opens the
                    // branch menu (ninth pass).
                    let on_branch = on_branch.clone();
                    div()
                        .id("branch-tag")
                        // Above the toolbar's drag area, as the buttons are.
                        .occlude()
                        .flex()
                        .items_center()
                        .ml(px(m.title_branch_gap))
                        .cursor_pointer()
                        .tooltip(Tooltip::text(format!("Current Branch: {branch}")))
                        .on_click(move |event, window, cx| on_branch(event.position(), window, cx))
                        .child(icon("weep/git-branch", m.icon_size, theme.icon_primary))
                        .child(div().ml(px(m.title_branch_icon_gap)).child(branch))
                })),
        );
    // Fleet on macOS has neither the logo nor the main menu button: the
    // panel toggles follow the traffic lights (fourteenth pass).
    let mac = cfg!(target_os = "macos");
    div()
        .id("toolbar")
        .h(px(m.toolbar_height))
        .w_full()
        .flex_shrink_0()
        .relative()
        .flex()
        .items_center()
        .pl(px(if state.traffic_lights {
            crate::theme::metrics::TRAFFIC_LIGHTS_WIDTH
        } else {
            m.toolbar_pad_x
        }))
        .when(!state.caption, |d| d.pr(px(m.toolbar_pad_x)))
        .gap(px(m.toolbar_gap))
        .when(state.caption, |d| {
            d.window_control_area(WindowControlArea::Drag)
        })
        .when(state.traffic_lights, |d| {
            d.on_mouse_down(MouseButton::Left, |_, _, _| MOVE_PENDING.set(true))
                .on_mouse_up(MouseButton::Left, |_, _, _| MOVE_PENDING.set(false))
                .on_mouse_down_out(|_, _, _| MOVE_PENDING.set(false))
                .on_mouse_move(|_, window, _| {
                    if MOVE_PENDING.replace(false) {
                        window.start_window_move();
                    }
                })
                .on_click(|event, window, _| {
                    if event.click_count() == 2 {
                        window.titlebar_double_click();
                    }
                })
        })
        .child(title)
        .when(!mac, |d| {
            d.child(
                div()
                    .size(px(m.toolbar_button))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("weep/logo", m.logo_size, theme.icon_primary)),
            )
            .child(
                icon_button("menu", "expui/general/menu", theme, m)
                    .tooltip(Tooltip::text("Main Menu"))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowMainMenu), cx)),
            )
        })
        .child(toggle(0, "weep/panel-left", "Toggle Left Panel"))
        .child(toggle(1, "weep/panel-bottom", "Toggle Bottom Panel"))
        .child(toggle(2, "weep/panel-right", "Toggle Right Panel"))
        .child(separator(theme, m))
        .child(
            icon_button("tools", "weep/grid", theme, m)
                .tooltip(Tooltip::text("Tools"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(GotoTool), cx)),
        )
        .child(
            icon_button("collab", "weep/user-add", theme, m).tooltip(Tooltip::text("Collaborate")),
        )
        .child(div().flex_1().min_w_0())
        .child({
            // The lightning (M7): a click turns Smart Mode on or opens its card.
            let on_smart = on_smart.clone();
            let open = state.smart_card_open;
            let tint = match state.smart {
                SmartIndicator::Off => theme.icon_primary,
                SmartIndicator::Starting | SmartIndicator::On => theme.smart_mode_enabled,
                SmartIndicator::Error => theme.smart_mode_error,
            };
            let fill = match state.smart {
                SmartIndicator::Off => theme.ghost_pressed,
                _ => tint,
            };
            div()
                .id("smart-mode")
                .size(px(m.toolbar_button))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(m.smart_mode_radius))
                .cursor_pointer()
                .occlude()
                .when(open, |d| d.bg(fill))
                .when(!open, |d| {
                    d.hover(|s| s.bg(theme.ghost_hovered))
                        .active(|s| s.bg(theme.ghost_pressed))
                })
                .tooltip(Tooltip::text("Smart Mode"))
                .on_click(move |event, window, cx| on_smart(event.position(), window, cx))
                .child(icon(
                    "expui/actions/lightning",
                    m.icon_size,
                    if open && state.smart != SmartIndicator::Off {
                        theme.text_bright
                    } else {
                        tint
                    },
                ))
        })
        .child(
            icon_button("run", "expui/run/run", theme, m)
                .tooltip(Tooltip::text("Run & Debug"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(RunAndDebug), cx)),
        )
        .child(
            icon_button("search", "expui/general/search", theme, m)
                .tooltip(Tooltip::text("Search Everywhere"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(GotoEverything), cx)),
        )
        .child(
            icon_button("notifications", "expui/toolwindows/notifications", theme, m)
                .tooltip(Tooltip::text("Notifications"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleNotifications), cx))
                .relative()
                .when(state.unread_notifications > 0, |b| {
                    b.child(
                        div()
                            .absolute()
                            .top(px(5. * m.zoom))
                            .right(px(5. * m.zoom))
                            .size(px(6. * m.zoom))
                            .rounded_full()
                            .bg(theme.smart_mode_enabled),
                    )
                }),
        )
        .child(
            icon_button("settings", "expui/general/settings", theme, m)
                .tooltip(Tooltip::text("Settings"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
        )
        .when(state.caption, |d| {
            d.child(div().w(px(m.toolbar_pad_x)).flex_shrink_0()).child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .gap_0()
                    .flex_shrink_0()
                    .children([
                        caption_button(
                            "caption-minimize",
                            "weep/caption-minimize",
                            WindowControlArea::Min,
                            theme,
                            m,
                        ),
                        caption_button(
                            "caption-maximize",
                            if state.maximized {
                                "weep/caption-restore"
                            } else {
                                "weep/caption-maximize"
                            },
                            WindowControlArea::Max,
                            theme,
                            m,
                        ),
                        caption_button(
                            "caption-close",
                            "weep/caption-close",
                            WindowControlArea::Close,
                            theme,
                            m,
                        ),
                    ]),
            )
        })
}
