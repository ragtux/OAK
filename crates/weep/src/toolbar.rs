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
use crate::skin::{self, Skin};
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
    /// M8: a debug session is live (the Run & Debug button shows it).
    pub debug: Option<DebugIndicator>,
}

/// What the Run & Debug button shows while a debug session is live
/// (estimate: unprobed): the debug icon in place of the run triangle, in
/// `icon.background.green` while the debuggee runs and in the primary icon
/// colour on a `ghostButton` pressed wash while it is stopped; the tooltip
/// names the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugIndicator {
    Running,
    Paused,
}

impl DebugIndicator {
    fn tooltip(self) -> &'static str {
        match self {
            DebugIndicator::Running => "Run & Debug (debugging)",
            DebugIndicator::Paused => "Run & Debug (paused)",
        }
    }
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
    let theme = theme.clone();
    let m = *m;
    skin::with_window(move |window, cx| {
        if Skin::current(cx) == Skin::Tui {
            let cell = skin::Cell::current(cx);
            render_tui(
                state, &theme, &m, cell, window, on_toggle, on_branch, on_smart,
            )
            .into_any_element()
        } else {
            render_fleet(state, &theme, &m, on_toggle, on_branch, on_smart).into_any_element()
        }
    })
}

/// Fleet's bar (the reference).
fn render_fleet(
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
        .child(match state.debug {
            None => icon_button("run", "expui/run/run", theme, m)
                .tooltip(Tooltip::text("Run & Debug"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(RunAndDebug), cx)),
            // M8: a live debug session (estimate, see `DebugIndicator`).
            Some(debug) => div()
                .id("run")
                .size(px(m.toolbar_button))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4. * m.zoom))
                .cursor_pointer()
                .occlude()
                .when(debug == DebugIndicator::Paused, |d| {
                    d.bg(theme.ghost_pressed)
                })
                .hover(|s| s.bg(theme.ghost_hovered))
                .active(|s| s.bg(theme.ghost_pressed))
                .tooltip(Tooltip::text(debug.tooltip()))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(RunAndDebug), cx))
                .child(icon(
                    "expui/run/debug",
                    m.icon_size,
                    match debug {
                        DebugIndicator::Running => theme.icon_green,
                        DebugIndicator::Paused => theme.icon_primary,
                    },
                )),
        })
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

// ---- the tui skin ------------------------------------------------------------

/// The rows the tui bar takes: one row of cells, or as many as the
/// platform's caption needs (macOS's traffic lights sit in Fleet's 38 px).
pub fn tui_rows(cell: skin::Cell, traffic_lights: bool, caption: bool) -> usize {
    let need = if traffic_lights {
        crate::theme::metrics::TOOLBAR_HEIGHT
    } else if caption {
        28.
    } else {
        0.
    };
    ((need / cell.h).ceil() as usize).max(1)
}

/// The tui bar's height in pixels for this window (the main menu and the
/// Notifications popup open under it).
pub fn tui_height(window: &Window, cx: &App) -> Pixels {
    let cell = skin::Cell::current(cx);
    let lights = cfg!(target_os = "macos") && !window.is_fullscreen();
    cell.rows(tui_rows(cell, lights, cfg!(windows)))
}

/// A mark of `cols` cells that a hover fills, above the caption's drag area.
fn tui_cell_button(
    id: impl Into<ElementId>,
    cols: usize,
    cell: skin::Cell,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .w(cell.cols(cols))
        .h(px(cell.h))
        .flex_shrink_0()
        .flex()
        .whitespace_nowrap()
        .cursor_pointer()
        .occlude()
        .hover(|s| s.bg(theme.list_bg_hovered))
}

/// The box a pictogram is drawn in: the cells' width, a capital's height
/// centred on the row.
fn glyph_box(bounds: gpui::Bounds<Pixels>, cell: skin::Cell) -> (f32, f32, f32, f32) {
    let x = f32::from(bounds.origin.x);
    let w = f32::from(bounds.size.width);
    let mid = f32::from(bounds.origin.y) + cell.h / 2.;
    let half = cell.font_size * 0.36;
    (x, mid - half, w, 2. * half)
}

/// A pictogram's line: about a glyph's stem, on device pixels.
fn pen(window: &Window, cell: skin::Cell) -> f32 {
    skin::snap(window, cell.font_size / 11.).max(1. / window.scale_factor())
}

fn line(window: &mut Window, from: (f32, f32), to: (f32, f32), width: f32, color: Hsla) {
    let mut path = gpui::PathBuilder::stroke(px(width));
    path.move_to(gpui::point(px(from.0), px(from.1)));
    path.line_to(gpui::point(px(to.0), px(to.1)));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn dot(window: &mut Window, at: (f32, f32), r: f32, color: Hsla) {
    window.paint_quad(gpui::quad(
        gpui::Bounds::new(
            gpui::point(px(at.0 - r), px(at.1 - r)),
            gpui::size(px(2. * r), px(2. * r)),
        ),
        px(r),
        color,
        px(0.),
        gpui::transparent_black(),
        gpui::BorderStyle::Solid,
    ));
}

/// A panel toggle's two cells: the window as a box, the panel's part
/// (0 left, 1 bottom, 2 right) filled while it shows, else only its edge.
fn paint_toggle(which: usize, open: bool, cell: skin::Cell, color: Hsla) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let t = pen(window, cell);
            let (x, y, w, h) = glyph_box(bounds, cell);
            let inset = skin::snap(window, cell.w * 0.3);
            let (x, w) = (
                skin::snap(window, x + inset),
                skin::snap(window, w - 2. * inset),
            );
            let (y, h) = (skin::snap(window, y), skin::snap(window, h));
            let outline = gpui::Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(w), px(h)));
            window.paint_quad(gpui::quad(
                outline,
                px(t * 1.5),
                gpui::transparent_black(),
                px(t),
                color,
                gpui::BorderStyle::Solid,
            ));
            let part = match which {
                0 => (x, y, skin::snap(window, w * 0.38), h),
                1 => {
                    let ph = skin::snap(window, h * 0.42);
                    (x, y + h - ph, w, ph)
                }
                _ => {
                    let pw = skin::snap(window, w * 0.38);
                    (x + w - pw, y, pw, h)
                }
            };
            let (px0, py0, pw, ph) = part;
            if open {
                window.paint_quad(gpui::fill(
                    gpui::Bounds::new(gpui::point(px(px0), px(py0)), gpui::size(px(pw), px(ph))),
                    color,
                ));
            } else {
                let edge = match which {
                    0 => (px0 + pw - t, py0, t, ph),
                    1 => (px0, py0, pw, t),
                    _ => (px0, py0, t, ph),
                };
                window.paint_quad(gpui::fill(
                    gpui::Bounds::new(
                        gpui::point(px(edge.0), px(edge.1)),
                        gpui::size(px(edge.2), px(edge.3)),
                    ),
                    color,
                ));
            }
        },
    )
    .w(cell.cols(2))
    .h(px(cell.h))
    .flex_shrink_0()
}

/// The branch mark (the font has no `⎇`): a trunk and a branch leaving it,
/// one cell.
fn paint_branch(cell: skin::Cell, color: Hsla) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let t = pen(window, cell);
            let (x, y, w, h) = glyph_box(bounds, cell);
            let trunk = skin::snap(window, x + w * 0.3);
            let twig = skin::snap(window, x + w * 0.78);
            let r = t * 1.4;
            line(window, (trunk, y + r), (trunk, y + h - r), t, color);
            line(window, (twig, y + h * 0.3), (trunk, y + h * 0.78), t, color);
            dot(window, (trunk, y + r), r, color);
            dot(window, (trunk, y + h - r), r, color);
            dot(window, (twig, y + h * 0.3), r, color);
        },
    )
    .w(px(cell.w))
    .h(px(cell.h))
    .flex_shrink_0()
}

/// Search's mark (the font has no `⌕`): a lens and its handle, one cell.
fn paint_search(cell: skin::Cell, color: Hsla) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let t = pen(window, cell);
            let (x, y, w, h) = glyph_box(bounds, cell);
            let r = (w * 0.34).min(h * 0.34);
            let (cx0, cy0) = (x + w * 0.44, y + h * 0.4);
            window.paint_quad(gpui::quad(
                gpui::Bounds::new(
                    gpui::point(px(cx0 - r), px(cy0 - r)),
                    gpui::size(px(2. * r), px(2. * r)),
                ),
                px(r),
                gpui::transparent_black(),
                px(t),
                color,
                gpui::BorderStyle::Solid,
            ));
            let k = std::f32::consts::FRAC_1_SQRT_2 * r;
            line(
                window,
                (cx0 + k - t / 2., cy0 + k - t / 2.),
                (x + w * 0.95, y + h),
                t * 1.3,
                color,
            );
        },
    )
    .w(px(cell.w))
    .h(px(cell.h))
    .flex_shrink_0()
}

/// The bar in the tui skin: one row of cells (more where the caption needs
/// them, the content row centred), the marks as glyphs of the mono font or
/// small drawings, each on whole cells of the window's grid.
#[allow(clippy::too_many_arguments)]
fn render_tui(
    state: ToolbarState,
    theme: &Theme,
    m: &Metrics,
    cell: skin::Cell,
    window: &Window,
    on_toggle: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    on_branch: impl Fn(Point<Pixels>, &mut Window, &mut App) + Clone + 'static,
    on_smart: impl Fn(Point<Pixels>, &mut Window, &mut App) + Clone + 'static,
) -> impl IntoElement {
    let rows = tui_rows(cell, state.traffic_lights, state.caption);
    let height = cell.rows(rows);
    let row_top = px(((rows - 1) as f32 * cell.h / 2.).floor());
    let window_cols = skin::Cell::fit(window.viewport_size().width, cell.w);
    let mac = cfg!(target_os = "macos");
    let mark = theme.icon_primary;

    // Left: the main menu (not on macOS), the panel toggles, the tools.
    let lead = if state.traffic_lights {
        (crate::theme::metrics::TRAFFIC_LIGHTS_WIDTH / cell.w).ceil() as usize
    } else {
        1
    };
    let opens = [state.left_open, state.bottom_open, state.right_open];
    let labels = [
        "Toggle Left Panel",
        "Toggle Bottom Panel",
        "Toggle Right Panel",
    ];
    let toggles = (0..3).map(|ix| {
        let f = on_toggle.clone();
        let hint = state.toggle_hints[ix].clone();
        let label = labels[ix];
        tui_cell_button(("panel-toggle", ix), 2, cell, theme)
            .map(|b| match hint {
                Some(h) => b.tooltip(Tooltip::with_detail(label, h)),
                None => b.tooltip(Tooltip::text(label)),
            })
            .on_click(move |_, window, cx| f(ix, window, cx))
            .child(paint_toggle(ix, opens[ix], cell, mark))
    });
    let left = div()
        .absolute()
        .top_0()
        .left(cell.cols(lead))
        .h(px(cell.h))
        .flex()
        .gap(px(cell.w))
        .when(!mac, |d| {
            d.child(
                tui_cell_button("menu", 1, cell, theme)
                    .text_color(mark)
                    .tooltip(Tooltip::text("Main Menu"))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowMainMenu), cx))
                    .child("⋮"),
            )
        })
        .children(toggles)
        .child(
            tui_cell_button("tools", 1, cell, theme)
                .text_color(mark)
                .tooltip(Tooltip::text("Tools"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(GotoTool), cx))
                .child("∷"),
        );
    let left_end = lead + if mac { 0 } else { 2 } + 3 * 3 + 1;

    // Right: smart mode, run, search, notifications, settings; then the
    // caption buttons on Windows.
    let caption_cols = if state.caption { 3 * 3 } else { 0 };
    let unread = state.unread_notifications;
    let bell = if unread > 0 {
        format!("●{unread}")
    } else {
        "○".to_string()
    };
    let bell_cols = skin::Cell::text_cols(&bell);
    let right_cols = 4 + bell_cols + 4;
    let right_start = window_cols
        .saturating_sub(caption_cols + 1 + right_cols)
        .max(left_end + 1);
    let smart = {
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
        tui_cell_button("smart-mode", 1, cell, theme)
            .text_color(if open && state.smart != SmartIndicator::Off {
                theme.text_bright
            } else {
                tint
            })
            .when(open, |d| d.bg(fill))
            .tooltip(Tooltip::text("Smart Mode"))
            .on_click(move |event, window, cx| on_smart(event.position(), window, cx))
            .child("⚡")
    };
    let right = div()
        .absolute()
        .top_0()
        .left(cell.cols(right_start))
        .h(px(cell.h))
        .flex()
        .gap(px(cell.w))
        .child(smart)
        .child(
            tui_cell_button("run", 1, cell, theme)
                // M8: a live debug session turns the mark green, `‖` while
                // stopped (estimate).
                .text_color(match state.debug {
                    Some(DebugIndicator::Running) => theme.icon_green,
                    _ => mark,
                })
                .when(state.debug == Some(DebugIndicator::Paused), |d| {
                    d.bg(theme.ghost_pressed)
                })
                .tooltip(Tooltip::text(
                    state
                        .debug
                        .map(DebugIndicator::tooltip)
                        .unwrap_or("Run & Debug"),
                ))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(RunAndDebug), cx))
                .child(match state.debug {
                    Some(DebugIndicator::Paused) => "‖",
                    _ => "▶",
                }),
        )
        .child(
            tui_cell_button("search", 1, cell, theme)
                .tooltip(Tooltip::text("Search Everywhere"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(GotoEverything), cx))
                .child(paint_search(cell, mark)),
        )
        .child(
            tui_cell_button("notifications", bell_cols, cell, theme)
                .text_color(if unread > 0 {
                    theme.smart_mode_enabled
                } else {
                    mark
                })
                .tooltip(Tooltip::text("Notifications"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleNotifications), cx))
                .child(bell),
        )
        .child(
            tui_cell_button("settings", 1, cell, theme)
                .text_color(mark)
                .tooltip(Tooltip::text("Settings"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx))
                .child("≡"),
        );

    // Centre: the workspace and the branch tag, on whole cells.
    let max_name = ((m.title_name_max_width / cell.w) as usize).max(4);
    let name: String = {
        let n = state.workspace_name.to_string();
        if n.chars().count() > max_name {
            let mut s: String = n.chars().take(max_name - 1).collect();
            s.push('…');
            s
        } else {
            n
        }
    };
    let branch = state.branch.clone();
    let title_cols = 2
        + skin::Cell::text_cols(&name)
        + branch
            .as_ref()
            .map(|b| 2 + 2 + skin::Cell::text_cols(b))
            .unwrap_or(0);
    let title_start = (window_cols.saturating_sub(title_cols) / 2).max(left_end + 1);
    let title = div()
        .absolute()
        .top_0()
        .left(cell.cols(title_start))
        .h(px(cell.h))
        .flex()
        .whitespace_nowrap()
        .text_color(theme.text_primary)
        .child(div().w(cell.cols(2)).text_color(mark).child("⊡"))
        .child(div().child(SharedString::from(name)))
        .children(branch.map(|branch| {
            let on_branch = on_branch.clone();
            let cols = 2 + skin::Cell::text_cols(&branch);
            div().flex().child(div().w(cell.cols(2))).child(
                tui_cell_button("branch-tag", cols, cell, theme)
                    .tooltip(Tooltip::text(format!("Current Branch: {branch}")))
                    .on_click(move |event, window, cx| on_branch(event.position(), window, cx))
                    .child(paint_branch(cell, mark))
                    .child(div().w(px(cell.w)))
                    .child(branch),
            )
        }));

    let captions = state.caption.then(|| {
        let maximized = state.maximized;
        let button = |id: &'static str, glyph: &'static str, area: WindowControlArea| {
            let close = matches!(area, WindowControlArea::Close);
            div()
                .id(id)
                .w(cell.cols(3))
                .h(px(cell.h))
                .flex_shrink_0()
                .flex()
                .justify_center()
                .occlude()
                .text_color(theme.icon_primary)
                .hover(move |s| {
                    if close {
                        s.bg(caption_close_hover()).text_color(gpui::white())
                    } else {
                        s.bg(theme.list_bg_hovered)
                    }
                })
                .window_control_area(area)
                .child(glyph)
        };
        div()
            .absolute()
            .top_0()
            .left(cell.cols(window_cols.saturating_sub(caption_cols)))
            .h(px(cell.h))
            .flex()
            .child(button("caption-minimize", "–", WindowControlArea::Min))
            .child(button(
                "caption-maximize",
                if maximized { "◫" } else { "□" },
                WindowControlArea::Max,
            ))
            .child(button("caption-close", "×", WindowControlArea::Close))
    });

    div()
        .id("toolbar")
        .h(height)
        .w_full()
        .flex_shrink_0()
        .relative()
        .font_family(theme.mono_font.clone())
        .text_size(px(cell.font_size))
        .line_height(px(cell.h))
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
        .child(
            div()
                .absolute()
                .left_0()
                .top(row_top)
                .w_full()
                .h(px(cell.h))
                .child(title)
                .child(left)
                .child(right)
                .children(captions),
        )
}
