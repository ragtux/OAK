//! Context menus and the main menu: Fleet's popup (`popup.*`) holding a
//! column of `listItem.*` rows, each a label with its shortcut in
//! `text.secondary` at the right, separators between groups, and submenus
//! that open beside the row (`Open In ›`, the main menu's `File ›`).
//! Opened at the pointer (right click, `show-context-menu`) or under a
//! toolbar button (the main menu); a row dispatches its action to the view
//! that had the focus before the menu opened.
//!
//! Batch 7 measured these (`docs/fleet-probe.md`, sixth pass): the popup's
//! top-left corner at the pointer, 6 px padding, 28 px rows with 8 px row
//! padding, a 12 px separator block, the width following the longest row.

use crate::actions::{PopupCancel, PopupConfirm, PopupDown, PopupLeft, PopupRight, PopupUp};
use crate::assets::icon;
use crate::theme::{Metrics, Theme};
use std::rc::Rc;

use gpui::{
    deferred, div, point, prelude::*, px, Action, App, Bounds, BoxShadow, Context, ElementId,
    Entity, EventEmitter, FocusHandle, Focusable, Font, FontFeatures, FontStyle, FontWeight,
    Pixels, Point, SharedString, Size, Subscription, TextRun, Window,
};

/// What a data row runs when chosen.
pub type SelectFn = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct MenuItem {
    pub label: SharedString,
    pub shortcut: Option<SharedString>,
    pub action: Box<dyn Action>,
    pub enabled: bool,
    /// Runs instead of `action` (a row made from data, such as a branch).
    pub on_select: Option<SelectFn>,
}

pub enum MenuEntry {
    Item(MenuItem),
    Separator,
    /// A row with a chevron that opens `entries` beside it.
    Submenu {
        label: SharedString,
        entries: Vec<MenuEntry>,
    },
}

impl MenuEntry {
    /// A row for a Fleet action id: its title and chord from the keymap.
    pub fn action(id: &str, cx: &App) -> Option<MenuEntry> {
        let action = crate::keymap::action_for(id)?;
        Some(MenuEntry::Item(MenuItem {
            label: crate::keymap::action_title(id).into(),
            shortcut: crate::keymap::hint_for(cx, id).map(Into::into),
            action,
            enabled: true,
            on_select: None,
        }))
    }

    /// A row that runs a closure (the branch tag's menu lists branches).
    pub fn custom(
        label: impl Into<SharedString>,
        f: impl Fn(&mut Window, &mut App) + 'static,
    ) -> MenuEntry {
        MenuEntry::Item(MenuItem {
            label: label.into(),
            shortcut: None,
            action: Box::new(PopupCancel),
            enabled: true,
            on_select: Some(Rc::new(f)),
        })
    }

    /// A row with its own label (the id gives the action and the chord).
    pub fn labelled(label: impl Into<SharedString>, id: &str, cx: &App) -> Option<MenuEntry> {
        let mut entry = Self::action(id, cx)?;
        if let MenuEntry::Item(item) = &mut entry {
            item.label = label.into();
        }
        Some(entry)
    }

    /// A greyed row that does nothing (a feature of a later milestone).
    pub fn disabled(label: impl Into<SharedString>) -> MenuEntry {
        MenuEntry::Item(MenuItem {
            label: label.into(),
            shortcut: None,
            action: Box::new(PopupCancel),
            enabled: false,
            on_select: None,
        })
    }

    pub fn submenu(label: impl Into<SharedString>, entries: Vec<MenuEntry>) -> MenuEntry {
        MenuEntry::Submenu {
            label: label.into(),
            entries,
        }
    }

    fn clone_entry(&self) -> MenuEntry {
        match self {
            MenuEntry::Item(i) => MenuEntry::Item(MenuItem {
                label: i.label.clone(),
                shortcut: i.shortcut.clone(),
                action: i.action.boxed_clone(),
                enabled: i.enabled,
                on_select: i.on_select.clone(),
            }),
            MenuEntry::Separator => MenuEntry::Separator,
            MenuEntry::Submenu { label, entries } => MenuEntry::Submenu {
                label: label.clone(),
                entries: entries.iter().map(MenuEntry::clone_entry).collect(),
            },
        }
    }

    fn selectable(&self) -> bool {
        match self {
            MenuEntry::Item(i) => i.enabled,
            MenuEntry::Separator => false,
            MenuEntry::Submenu { .. } => true,
        }
    }
}

#[derive(Clone, Debug)]
pub enum MenuEvent {
    /// Escape or a click elsewhere closed this menu (a submenu's parent stays).
    Dismissed,
    /// A row ran its action: every menu in the chain closes.
    Activated,
}

pub struct ContextMenu {
    entries: Vec<MenuEntry>,
    /// Where the menu's top-left corner goes, in window coordinates; moved
    /// inside the window when it would not fit.
    position: Point<Pixels>,
    selected: Option<usize>,
    focus_handle: FocusHandle,
    /// The view to give the focus back to, and to dispatch the action on.
    return_focus: Option<FocusHandle>,
    /// The open submenu, and the row it belongs to.
    child: Option<(usize, Entity<ContextMenu>, Subscription)>,
    /// Where the last frame put the menu, for placing a submenu and for
    /// telling a click on the submenu from a click outside.
    laid: Bounds<Pixels>,
    is_submenu: bool,
}

impl EventEmitter<MenuEvent> for ContextMenu {}

impl Focusable for ContextMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ContextMenu {
    pub fn new(
        entries: Vec<MenuEntry>,
        position: Point<Pixels>,
        return_focus: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Fleet's menus start with no row selected; the first arrow key selects.
        ContextMenu {
            entries,
            position,
            selected: None,
            focus_handle: cx.focus_handle(),
            return_focus,
            child: None,
            laid: Bounds::default(),
            is_submenu: false,
        }
    }

    fn selectable_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.selectable())
            .map(|(ix, _)| ix)
            .collect()
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.selectable_indices();
        if rows.is_empty() {
            return;
        }
        let pos = self
            .selected
            .and_then(|s| rows.iter().position(|&e| e == s));
        let next = match (pos, forward) {
            (None, true) => 0,
            (None, false) => rows.len() - 1,
            (Some(p), true) => (p + 1) % rows.len(),
            (Some(p), false) => (p + rows.len() - 1) % rows.len(),
        };
        self.selected = Some(rows[next]);
        self.close_child(cx);
        cx.notify();
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }
    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }
    fn confirm(&mut self, _: &PopupConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected {
            self.activate(ix, window, cx);
        }
    }
    fn cancel(&mut self, _: &PopupCancel, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }
    /// Right opens the selected submenu and moves the focus into it.
    fn right(&mut self, _: &PopupRight, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected {
            if matches!(self.entries.get(ix), Some(MenuEntry::Submenu { .. })) {
                self.open_child(ix, cx);
                if let Some((_, child, _)) = &self.child {
                    child.update(cx, |c, cx| {
                        c.selected = c.selectable_indices().first().copied();
                        cx.notify();
                    });
                    let handle = child.read(cx).focus_handle.clone();
                    handle.focus(window, cx);
                }
            }
        }
    }
    /// Left closes a submenu and goes back to its parent.
    fn left(&mut self, _: &PopupLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_submenu {
            self.dismiss(window, cx);
        }
    }

    fn restore_focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(handle) = &self.return_focus {
            handle.focus(window, cx);
        }
    }

    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_child(cx);
        if !self.is_submenu {
            self.restore_focus(window, cx);
        }
        cx.emit(MenuEvent::Dismissed);
    }

    /// Runs row `ix`: the focus goes back first, so an editor action lands in
    /// the editor the menu was opened on. A submenu row opens its menu.
    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.entries.get(ix) {
            Some(MenuEntry::Item(item)) if item.enabled => {
                let action = item.action.boxed_clone();
                let custom = item.on_select.clone();
                self.restore_focus(window, cx);
                match custom {
                    Some(f) => f(window, cx),
                    None => window.dispatch_action(action, cx),
                }
                cx.emit(MenuEvent::Activated);
            }
            Some(MenuEntry::Submenu { .. }) => self.open_child(ix, cx),
            _ => {}
        }
    }

    fn close_child(&mut self, cx: &mut Context<Self>) {
        if self.child.take().is_some() {
            cx.notify();
        }
    }

    /// Opens row `ix`'s submenu beside it (its top at the row's top, its
    /// left where this menu ends).
    fn open_child(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some((open_ix, _, _)) = &self.child {
            if *open_ix == ix {
                return;
            }
        }
        let Some(MenuEntry::Submenu { entries, .. }) = self.entries.get(ix) else {
            return;
        };
        let entries: Vec<MenuEntry> = entries.iter().map(MenuEntry::clone_entry).collect();
        let m = *cx.global::<Metrics>();
        let row_top = self.laid.origin.y + px(m.popup_pad) + self.offset_of(ix, &m);
        let position = point(
            self.laid.origin.x + self.laid.size.width - px(m.popup_pad),
            row_top - px(m.popup_pad),
        );
        let return_focus = self.return_focus.clone();
        let child = cx.new(|cx| {
            let mut menu = ContextMenu::new(entries, position, return_focus, cx);
            menu.is_submenu = true;
            menu
        });
        let subscription = cx.subscribe(&child, |this, _, event: &MenuEvent, cx| match event {
            MenuEvent::Activated => cx.emit(MenuEvent::Activated),
            MenuEvent::Dismissed => {
                this.child = None;
                cx.notify();
            }
        });
        self.child = Some((ix, child, subscription));
        self.selected = Some(ix);
        cx.notify();
    }

    /// The y offset of row `ix` inside the padded column.
    fn offset_of(&self, ix: usize, m: &Metrics) -> Pixels {
        let mut y = px(0.);
        for e in self.entries.iter().take(ix) {
            y += match e {
                MenuEntry::Separator => px(1. + 2. * m.menu_separator_pad),
                _ => px(m.menu_row_height),
            };
        }
        y
    }

    /// The menu's height at the current metrics, to keep it inside the window.
    fn height(&self, m: &Metrics) -> Pixels {
        px(2. * m.popup_pad) + self.offset_of(self.entries.len(), m)
    }

    /// Whether `position` falls on this menu or any open submenu.
    pub fn contains(&self, position: Point<Pixels>, cx: &App) -> bool {
        self.laid.contains(&position)
            || self
                .child
                .as_ref()
                .is_some_and(|(_, c, _)| c.read(cx).contains(position, cx))
    }

    /// The widest row: label, the gap, and the shortcut or the chevron.
    fn content_width(&self, theme: &Theme, m: &Metrics, window: &Window) -> Pixels {
        let font = Font {
            family: theme.ui_font.clone(),
            features: FontFeatures::default(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            fallbacks: None,
        };
        let measure = |text: &SharedString| -> Pixels {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: theme.text_primary,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text.clone(), px(m.ui_font_size), &[run], None)
                .width
        };
        let mut widest = px(0.);
        for e in &self.entries {
            let w = match e {
                MenuEntry::Item(i) => {
                    measure(&i.label)
                        + i.shortcut
                            .as_ref()
                            .map(|s| px(m.menu_shortcut_gap) + measure(s))
                            .unwrap_or(px(0.))
                }
                MenuEntry::Submenu { label, .. } => {
                    measure(label) + px(m.menu_shortcut_gap) + px(m.icon_size)
                }
                MenuEntry::Separator => px(0.),
            };
            widest = widest.max(w);
        }
        widest + px(2. * m.menu_row_pad_x + 2. * m.popup_pad)
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let entity = cx.entity();
        let viewport = window.viewport_size();
        let width = self
            .content_width(&theme, &m, window)
            .max(px(m.menu_min_width));
        let height = self.height(&m);
        let margin = px(m.island_margin);
        let left = self
            .position
            .x
            .min(viewport.width - width - margin)
            .max(margin);
        let top = self
            .position
            .y
            .min(viewport.height - height - margin)
            .max(margin);
        self.laid = Bounds {
            origin: point(left, top),
            size: Size { width, height },
        };
        let rows = self
            .entries
            .iter()
            .enumerate()
            .map(|(ix, entry)| match entry {
                MenuEntry::Separator => div()
                    .id(ElementId::NamedInteger("menu-sep".into(), ix as u64))
                    .my(px(m.menu_separator_pad))
                    .h(px(1.))
                    .flex_shrink_0()
                    .bg(theme.popup_separator)
                    .into_any_element(),
                MenuEntry::Item(_) | MenuEntry::Submenu { .. } => {
                    let (label, shortcut, enabled, submenu) = match entry {
                        MenuEntry::Item(i) => {
                            (i.label.clone(), i.shortcut.clone(), i.enabled, false)
                        }
                        MenuEntry::Submenu { label, .. } => (label.clone(), None, true, true),
                        MenuEntry::Separator => unreachable!(),
                    };
                    let selected = self.selected == Some(ix);
                    let entity = entity.clone();
                    div()
                        .id(ElementId::NamedInteger("menu-row".into(), ix as u64))
                        .h(px(m.menu_row_height))
                        .px(px(m.menu_row_pad_x))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(m.menu_shortcut_gap))
                        .rounded(px(m.popup_row_radius))
                        .whitespace_nowrap()
                        .text_color(if enabled {
                            theme.list_text
                        } else {
                            theme.text_disabled
                        })
                        .when(enabled, |d| {
                            d.cursor_pointer()
                                .when(selected, |d| {
                                    d.bg(theme.list_bg_focused)
                                        .text_color(theme.list_text_focused)
                                })
                                .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                                .on_mouse_move({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            if this.selected != Some(ix) {
                                                this.selected = Some(ix);
                                                cx.notify();
                                            }
                                            if submenu {
                                                this.open_child(ix, cx);
                                            } else {
                                                this.close_child(cx);
                                            }
                                        })
                                    }
                                })
                                .on_click(move |_, window, cx| {
                                    entity.update(cx, |this, cx| this.activate(ix, window, cx))
                                })
                        })
                        .child(div().flex_1().child(label))
                        .children(shortcut.map(|s| {
                            div()
                                .flex_shrink_0()
                                .text_color(if enabled {
                                    theme.text_secondary
                                } else {
                                    theme.text_disabled
                                })
                                .child(s)
                        }))
                        .when(submenu, |d| {
                            d.child(icon(
                                "expui/general/chevronRight",
                                m.icon_size,
                                theme.text_secondary,
                            ))
                        })
                        .into_any_element()
                }
            });
        let close = entity.clone();
        let child = self.child.as_ref().map(|(_, c, _)| c.clone());
        deferred(
            div()
                .id("context-menu")
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::cancel))
                .on_action(cx.listener(Self::left))
                .on_action(cx.listener(Self::right))
                .on_mouse_down_out(move |event, window, cx| {
                    close.update(cx, |this, cx| {
                        // A click on the open submenu is not a click outside.
                        if !this.contains(event.position, cx) {
                            this.dismiss(window, cx);
                        }
                    });
                })
                .occlude()
                .absolute()
                .left(left)
                .top(top)
                .w(width)
                .flex()
                .flex_col()
                .p(px(m.popup_pad))
                .rounded(px(m.popup_radius))
                .border_1()
                .border_color(theme.popup_border)
                .bg(theme.popup_bg)
                .font_family(theme.ui_font.clone())
                .text_size(px(m.ui_font_size))
                .text_color(theme.popup_text)
                .shadow(vec![BoxShadow {
                    color: theme.shadow_medium,
                    offset: point(px(0.), px(m.popup_shadow_offset_y)),
                    blur_radius: px(m.popup_shadow_blur),
                    spread_radius: px(0.),
                }])
                .children(rows)
                .children(child),
        )
    }
}
