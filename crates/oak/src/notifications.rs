//! Notifications: what Oak would otherwise print to stderr (a save that
//! failed, a file that could not be created) shown the way Fleet shows it,
//! as a toast (`toast.*`) in the window's corner for a few seconds and as a
//! row in the Notifications tool (`notification.*`) until cleared. Batch 7
//! measures both; the geometry here is an estimate (`theme.rs::metrics`).

use crate::assets::icon;
use crate::theme::{metrics::TOAST_MS, Metrics, Theme};
use gpui::{
    div, point, prelude::*, px, App, BoxShadow, Context, ElementId, Entity, EventEmitter,
    FontWeight, SharedString, Window,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
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
        let _ = cx;
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
                .children(toasts),
        )
    }
}

/// The Notifications tool's body: the rows, newest first, or an empty state.
impl Render for Notifications {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
