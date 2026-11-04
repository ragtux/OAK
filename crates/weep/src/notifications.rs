//! Notifications: what WEEP would otherwise print to stderr (a save that
//! failed, a file that could not be created) shown the way Fleet shows it,
//! as a toast (`toast.*`) in the window's corner for a few seconds and as a
//! row in the Notifications tool (`notification.*`) until cleared. Batch 7
//! measures both; the geometry here is an estimate (`theme.rs::metrics`).

use crate::assets::icon;
use crate::skin::{self, Cell, Skin};
use crate::theme::{metrics::TOAST_MS, Metrics, Theme};
use gpui::{
    div, point, prelude::*, px, AnyElement, App, BoxShadow, Context, ElementId, Entity,
    EventEmitter, FontWeight, SharedString, Window,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    /// Fleet's `banner.*` colours ("File has been changed on disk") are an M3
    /// follow-up; nothing raises a warning yet.
    #[allow(dead_code)]
    Warning,
    Error,
}

impl Level {
    fn icon(self) -> &'static str {
        match self {
            Level::Info => "expui/status/info",
            Level::Warning => "expui/status/warning",
            Level::Error => "expui/status/error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub id: u64,
    pub level: Level,
    pub title: SharedString,
    pub message: Option<SharedString>,
    pub read: bool,
}

#[derive(Clone, Debug)]
pub enum NotificationsEvent {
    Changed,
}

pub struct Notifications {
    /// Newest first.
    items: Vec<Notification>,
    /// The ids still showing as toasts.
    toasts: Vec<u64>,
    next_id: u64,
}

impl EventEmitter<NotificationsEvent> for Notifications {}

impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifications {
    pub fn new() -> Self {
        Notifications {
            items: Vec::new(),
            toasts: Vec::new(),
            next_id: 1,
        }
    }

    /// Adds a notification: a toast for `TOAST_MS`, a row until cleared.
    pub fn push(
        &mut self,
        level: Level,
        title: impl Into<SharedString>,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.items.insert(
            0,
            Notification {
                id,
                level,
                title: title.into(),
                message: message.map(Into::into),
                read: false,
            },
        );
        self.toasts.push(id);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(TOAST_MS))
                .await;
            let _ = this.update(cx, |this, cx| this.dismiss_toast(id, cx));
        })
        .detach();
        cx.emit(NotificationsEvent::Changed);
        cx.notify();
        id
    }

    pub fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(ix) = self.toasts.iter().position(|t| *t == id) {
            self.toasts.remove(ix);
            cx.notify();
        }
    }

    pub fn remove(&mut self, id: u64, cx: &mut Context<Self>) {
        self.items.retain(|n| n.id != id);
        self.toasts.retain(|t| *t != id);
        cx.emit(NotificationsEvent::Changed);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.items.clear();
        self.toasts.clear();
        cx.emit(NotificationsEvent::Changed);
        cx.notify();
    }

    /// Opening the tool reads everything.
    pub fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for n in &mut self.items {
            changed |= !n.read;
            n.read = true;
        }
        if changed {
            cx.emit(NotificationsEvent::Changed);
            cx.notify();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn unread(&self) -> usize {
        self.items.iter().filter(|n| !n.read).count()
    }

    fn level_color(level: Level, theme: &Theme) -> gpui::Hsla {
        match level {
            Level::Info => theme.icon_secondary,
            Level::Warning => theme.warning,
            Level::Error => theme.text_dangerous,
        }
    }

    fn render_row(
        &self,
        n: &Notification,
        theme: &Theme,
        m: &Metrics,
        entity: &Entity<Notifications>,
    ) -> impl IntoElement {
        let id = n.id;
        let entity = entity.clone();
        div()
            .id(ElementId::NamedInteger("notification".into(), id))
            .group("notification")
            .w_full()
            .p(px(m.notification_pad))
            .flex()
            .items_start()
            .gap(px(m.notification_icon_gap))
            .border_b_1()
            .border_color(theme.notification_separator)
            .bg(if n.read {
                theme.notification_bg
            } else {
                theme.notification_bg_unread
            })
            .child(
                div()
                    .h(px(m.notification_line))
                    .flex()
                    .items_center()
                    .child(icon(
                        n.level.icon(),
                        m.icon_size,
                        Self::level_color(n.level, theme),
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .line_height(px(m.notification_line))
                    .child(
                        div()
                            .text_color(theme.notification_text)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(n.title.clone()),
                    )
                    .children(n.message.clone().map(|msg| {
                        div()
                            .text_color(theme.notification_text_secondary)
                            .child(msg)
                    })),
            )
            .child(
                div()
                    .id(ElementId::NamedInteger("notification-close".into(), id))
                    .size(px(m.icon_size))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3. * m.zoom))
                    .cursor_pointer()
                    .invisible()
                    .group_hover("notification", |s| s.visible())
                    .hover(|s| s.bg(theme.ghost_hovered))
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.remove(id, cx));
                    })
                    .child(icon(
                        "expui/general/closeSmall",
                        m.icon_size,
                        theme.icon_secondary,
                    )),
            )
    }

    /// The toasts, stacked from the window's bottom-right corner, newest at
    /// the bottom.
    pub fn render_toasts(&self, theme: &Theme, m: &Metrics, cx: &App) -> Option<impl IntoElement> {
        if self.toasts.is_empty() {
            return None;
        }
        if Skin::current(cx) == Skin::Tui {
            return Some(self.render_toasts_tui(theme, m, cx));
        }
        let toasts: Vec<_> = self
            .toasts
            .iter()
            .rev()
            .filter_map(|id| self.items.iter().find(|n| n.id == *id))
            .map(|n| {
                div()
                    .w(px(m.toast_width))
                    .p(px(m.toast_pad))
                    .flex()
                    .items_start()
                    .gap(px(m.notification_icon_gap))
                    .rounded(px(m.toast_radius))
                    .border_1()
                    .border_color(theme.toast_border)
                    .bg(theme.toast_bg)
                    .text_color(theme.toast_text)
                    .shadow(vec![BoxShadow {
                        color: theme.shadow_medium,
                        offset: point(px(0.), px(m.popup_shadow_offset_y)),
                        blur_radius: px(m.popup_shadow_blur),
                        spread_radius: px(0.),
                    }])
                    .child(
                        div()
                            .h(px(m.notification_line))
                            .flex()
                            .items_center()
                            .child(icon(
                                n.level.icon(),
                                m.icon_size,
                                Self::level_color(n.level, theme),
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .line_height(px(m.notification_line))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(n.title.clone()),
                            )
                            .children(
                                n.message
                                    .clone()
                                    .map(|msg| div().text_color(theme.text_secondary).child(msg)),
                            ),
                    )
            })
            .collect();
        Some(
            div()
                .absolute()
                .right(px(m.toast_margin))
                .bottom(px(m.toast_margin + m.status_bar_height))
                .flex()
                .flex_col()
                .gap(px(m.toast_gap))
                .font_family(theme.ui_font.clone())
                .text_size(px(m.ui_font_size))
                .children(toasts)
                .into_any_element(),
        )
    }

    /// The toasts in the tui skin: boxes stacked up from above the status
    /// bar at the window's right, on whole cells: a `▌` in the level's colour
    /// before the title, the message wrapped on the grid.
    fn render_toasts_tui(&self, theme: &Theme, m: &Metrics, cx: &App) -> AnyElement {
        let cell = Cell::current(cx);
        let cols = ((m.toast_width / cell.w).round() as usize).max(16);
        let toasts: Vec<(Level, SharedString, Vec<String>)> = self
            .toasts
            .iter()
            .rev()
            .filter_map(|id| self.items.iter().find(|n| n.id == *id))
            .map(|n| {
                let lines = n
                    .message
                    .as_ref()
                    .map(|t| skin::wrap(t, cols.saturating_sub(5)))
                    .unwrap_or_default();
                (n.level, n.title.clone(), lines)
            })
            .collect();
        let theme = theme.clone();
        let m = *m;
        skin::with_window(move |window, _| {
            let viewport = window.viewport_size();
            let win_cols = Cell::fit(viewport.width, cell.w);
            let win_rows = Cell::fit(viewport.height, cell.h);
            let cols = cols.min(win_cols.saturating_sub(2)).max(6);
            let total: usize = toasts.iter().map(|(_, _, l)| 3 + l.len()).sum();
            let top = win_rows.saturating_sub(2 + total);
            let boxes = toasts.into_iter().map(|(level, title, lines)| {
                let rows = 3 + lines.len();
                let body = div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(cell.h))
                            .flex()
                            .whitespace_nowrap()
                            .child(
                                div()
                                    .w(cell.cols(2))
                                    .flex_shrink_0()
                                    .text_color(Self::tui_level_color(level, &theme))
                                    .child("▌"),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.toast_text)
                                    .child(title),
                            ),
                    )
                    .children(lines.into_iter().map(|l| {
                        div()
                            .h(px(cell.h))
                            .pl(cell.cols(2))
                            .whitespace_nowrap()
                            .text_color(theme.text_secondary)
                            .child(SharedString::from(l))
                    }))
                    .into_any_element();
                skin::boxed(
                    cell,
                    cols,
                    rows,
                    Vec::new(),
                    Vec::new(),
                    body,
                    theme.toast_bg,
                    theme.border,
                    &theme,
                    m,
                )
            });
            div()
                .absolute()
                .left(cell.cols(win_cols.saturating_sub(cols + 2)))
                .top(cell.rows(top))
                .flex()
                .flex_col()
                .children(boxes)
                .into_any_element()
        })
        .into_any_element()
    }

    /// The tui skin's colour for a level's mark (`docs/tui-skin.md`).
    fn tui_level_color(level: Level, theme: &Theme) -> gpui::Hsla {
        match level {
            Level::Info => theme.icon_blue,
            Level::Warning => theme.warning,
            Level::Error => theme.text_dangerous,
        }
    }

    /// The Notifications popup in the tui skin: a box of `cols` cells at
    /// `origin` (window coordinates, whole cells), the title set into its top
    /// edge with the settings and clear marks at the edge's right end, then a
    /// group of rows per notification (`● title`, the message wrapped)
    /// separated by rules joining the frame, as many as `max_rows` hold.
    pub fn render_popup_tui(
        &self,
        cell: Cell,
        cols: usize,
        max_rows: usize,
        entity: &Entity<Notifications>,
        theme: &Theme,
        m: &Metrics,
    ) -> gpui::Div {
        let text_cols = cols.saturating_sub(2 + 2 + 3);
        let mut groups: Vec<AnyElement> = Vec::new();
        let mut rules = Vec::new();
        let mut row = 1;
        for n in &self.items {
            let lines = n
                .message
                .as_ref()
                .map(|t| skin::wrap(t, text_cols))
                .unwrap_or_default();
            let height = 1 + lines.len();
            let rule = usize::from(!groups.is_empty());
            if row + rule + height + 1 > max_rows && !groups.is_empty() {
                break;
            }
            if rule == 1 {
                rules.push(row);
                groups.push(div().h(px(cell.h)).flex_shrink_0().into_any_element());
                row += 1;
            }
            row += height;
            let id = n.id;
            let entity = entity.clone();
            groups.push(
                div()
                    .id(ElementId::NamedInteger("notification".into(), id))
                    .group("notification")
                    .relative()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .px(px(cell.w))
                    .hover(|s| s.bg(theme.list_bg_hovered))
                    .child(
                        div()
                            .h(px(cell.h))
                            .flex()
                            .whitespace_nowrap()
                            .child(
                                div()
                                    .w(cell.cols(2))
                                    .flex_shrink_0()
                                    .text_color(Self::tui_level_color(n.level, theme))
                                    .child(if n.read { "▌" } else { "●" }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.notification_text)
                                    .child(n.title.clone()),
                            )
                            .child(
                                div()
                                    .id(ElementId::NamedInteger("notification-close".into(), id))
                                    .w(px(cell.w))
                                    .flex_shrink_0()
                                    .cursor_pointer()
                                    .text_color(theme.icon_secondary)
                                    .invisible()
                                    .group_hover("notification", |s| s.visible())
                                    .hover(|s| s.bg(theme.list_bg_focused))
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| this.remove(id, cx));
                                    })
                                    .child("×"),
                            ),
                    )
                    .children(lines.into_iter().map(|l| {
                        div()
                            .h(px(cell.h))
                            .pl(cell.cols(2))
                            .whitespace_nowrap()
                            .text_color(theme.notification_text_secondary)
                            .child(SharedString::from(l))
                    }))
                    .into_any_element(),
            );
        }
        let empty = groups.is_empty();
        let rows = if empty { 5 } else { row + 1 };
        let body = if empty {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .whitespace_nowrap()
                .text_color(theme.text_tertiary)
                .child("You don't have any unread notifications")
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .flex_col()
                .children(groups)
                .into_any_element()
        };
        let mark = |id: &'static str, glyph: &'static str, tip: &'static str| {
            div()
                .id(id)
                .w(cell.cols(3))
                .h(px(cell.h))
                .flex()
                .justify_center()
                .cursor_pointer()
                .text_color(theme.icon_primary)
                .hover(|s| s.bg(theme.list_bg_hovered))
                .tooltip(crate::tooltip::Tooltip::text(tip))
                .child(glyph)
        };
        skin::boxed(
            cell,
            cols,
            rows,
            vec![skin::title("Notifications", theme.popup_text)],
            rules,
            body,
            theme.popup_bg,
            theme.border,
            theme,
            *m,
        )
        .child(
            div()
                .absolute()
                .top_0()
                .right(cell.cols(2))
                .h(px(cell.h))
                .flex()
                .bg(theme.popup_bg)
                .child(
                    mark("notifications-settings", "≡", "Settings").on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::OpenSettings), cx)
                    }),
                )
                .child(
                    mark("notifications-clear", "×", "Clear All").on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::ClearNotifications), cx)
                    }),
                ),
        )
    }

    /// The Notifications tool's body in the tui skin: rows of whole cells, a
    /// rule between notifications.
    fn render_tui(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let entity = cx.entity();
        let base = div()
            .size_full()
            .font_family(theme.mono_font.clone())
            .text_size(px(cell.font_size))
            .line_height(px(cell.h));
        if self.items.is_empty() {
            return base
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_tertiary)
                .child("No notifications")
                .into_any_element();
        }
        let mut rows: Vec<AnyElement> = Vec::new();
        for (k, n) in self.items.iter().enumerate() {
            if k > 0 {
                rows.push(full_rule(cell, theme.notification_separator, m).into_any_element());
            }
            let id = n.id;
            let entity = entity.clone();
            rows.push(
                div()
                    .id(ElementId::NamedInteger("notification".into(), id))
                    .group("notification")
                    .flex()
                    .whitespace_nowrap()
                    .hover(|s| s.bg(theme.list_bg_hovered))
                    .child(
                        div()
                            .w(cell.cols(2))
                            .flex_shrink_0()
                            .text_color(Self::tui_level_color(n.level, &theme))
                            .child(if n.read { "▌" } else { "●" }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.notification_text)
                                    .child(n.title.clone()),
                            )
                            .children(n.message.clone().map(|msg| {
                                div()
                                    .whitespace_normal()
                                    .text_color(theme.notification_text_secondary)
                                    .child(msg)
                            })),
                    )
                    .child(
                        div()
                            .id(ElementId::NamedInteger("notification-close".into(), id))
                            .w(px(cell.w))
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_color(theme.icon_secondary)
                            .invisible()
                            .group_hover("notification", |s| s.visible())
                            .hover(|s| s.bg(theme.list_bg_focused))
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| this.remove(id, cx));
                            })
                            .child("×"),
                    )
                    .into_any_element(),
            );
        }
        base.id("notifications-list")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(rows)
            .into_any_element()
    }
}

/// A row with a line through its centre across its whole width.
fn full_rule(cell: Cell, color: gpui::Hsla, m: Metrics) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds: gpui::Bounds<gpui::Pixels>, _, window, _| {
            let t = skin::stroke(window, &m);
            let y = skin::snap(window, f32::from(bounds.origin.y) + cell.h / 2.) - t / 2.;
            window.paint_quad(gpui::fill(
                gpui::Bounds::new(
                    gpui::point(bounds.origin.x, px(y)),
                    gpui::size(bounds.size.width, px(t)),
                ),
                color,
            ));
        },
    )
    .w_full()
    .h(px(cell.h))
    .flex_shrink_0()
}

/// The Notifications tool's body: the rows, newest first, or an empty state.
impl Render for Notifications {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(cx);
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let entity = cx.entity();
        if self.items.is_empty() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_secondary)
                .child("No notifications")
                .into_any_element();
        }
        let rows: Vec<_> = self
            .items
            .iter()
            .map(|n| self.render_row(n, &theme, &m, &entity))
            .collect();
        div()
            .id("notifications-list")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(rows)
            .into_any_element()
    }
}
