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
use crate::skin::{self, Cell, Skin};
use crate::theme::{Metrics, Theme};
use std::rc::Rc;

use gpui::{
    deferred, div, point, prelude::*, px, Action, AnyElement, App, Bounds, BoxShadow, Context,
    ElementId, Entity, EventEmitter, FocusHandle, Focusable, Font, FontFeatures, FontStyle,
    FontWeight, Hsla, Pixels, Point, SharedString, Size, Subscription, TextRun, Window,
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
    /// An icon before the label (the gutter run icon's menu: Fleet's green
    /// run and debug icons, twenty-fourth pass).
    pub icon: Option<MenuIcon>,
}

/// A row's icon: an asset (`expui/run/run`) in `color`, 16 px at the row's
/// left padding with the popup's 4 px gap to the label (twenty-fourth pass:
/// the run icon's menu, label ink 20 px after the row's padding); the tui
/// skin puts `glyph` in a cell of its own instead.
#[derive(Clone)]
pub struct MenuIcon {
    pub path: SharedString,
    pub color: Hsla,
    pub glyph: &'static str,
}

pub enum MenuEntry {
    Item(MenuItem),
    Separator,
    /// A row with a chevron that opens `entries` beside it.
    Submenu {
        label: SharedString,
        entries: Vec<MenuEntry>,
    },
    /// A line of help in `text.tertiary` closing the menu, never selected
    /// (the gutter run icon's "⇧Click on icon to Run, ⌥Click to Debug",
    /// right under the last row with no separator; twenty-fourth pass).
    Hint(SharedString),
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
            icon: None,
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
            icon: None,
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
            icon: None,
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
                icon: i.icon.clone(),
            }),
            MenuEntry::Separator => MenuEntry::Separator,
            MenuEntry::Hint(text) => MenuEntry::Hint(text.clone()),
            MenuEntry::Submenu { label, entries } => MenuEntry::Submenu {
                label: label.clone(),
                entries: entries.iter().map(MenuEntry::clone_entry).collect(),
            },
        }
    }

    fn selectable(&self) -> bool {
        match self {
            MenuEntry::Item(i) => i.enabled,
            MenuEntry::Separator | MenuEntry::Hint(_) => false,
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
        let position = if Skin::current(cx) == Skin::Tui {
            // The submenu's first row beside this row, its frame on this
            // menu's right frame column.
            let cell = Cell::current(cx);
            point(
                self.laid.origin.x + self.laid.size.width - px(cell.w),
                self.laid.origin.y + cell.rows(ix),
            )
        } else {
            let row_top = self.laid.origin.y + px(m.popup_pad) + self.offset_of(ix, &m);
            point(
                self.laid.origin.x + self.laid.size.width - px(m.popup_pad),
                row_top - px(m.popup_pad),
            )
        };
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
                MenuEntry::Hint(_) => px(m.menu_hint_height),
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
                        + i.icon
                            .as_ref()
                            .map(|_| px(m.icon_size + m.popup_row_icon_gap))
                            .unwrap_or(px(0.))
                        + i.shortcut
                            .as_ref()
                            .map(|s| px(m.menu_shortcut_gap) + measure(s))
                            .unwrap_or(px(0.))
                }
                MenuEntry::Submenu { label, .. } => {
                    measure(label) + px(m.menu_shortcut_gap) + px(m.icon_size)
                }
                MenuEntry::Hint(text) => measure(text),
                MenuEntry::Separator => px(0.),
            };
            widest = widest.max(w);
        }
        widest + px(2. * m.menu_row_pad_x + 2. * m.popup_pad)
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(window, cx);
        }
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
                // The hint sits on a row's 28 px line but its box ends
                // `menu_hint_height` down, the line running into the
                // padding (Fleet's menu ends 26.5 under the hint's top).
                MenuEntry::Hint(text) => div()
                    .id(ElementId::NamedInteger("menu-hint".into(), ix as u64))
                    .h(px(m.menu_hint_height))
                    .flex_shrink_0()
                    .child(
                        div()
                            .h(px(m.menu_row_height))
                            .px(px(m.menu_row_pad_x))
                            .flex()
                            .items_center()
                            .whitespace_nowrap()
                            .text_color(theme.text_tertiary)
                            .child(text.clone()),
                    )
                    .into_any_element(),
                MenuEntry::Item(_) | MenuEntry::Submenu { .. } => {
                    let (label, shortcut, enabled, submenu, row_icon) = match entry {
                        MenuEntry::Item(i) => (
                            i.label.clone(),
                            i.shortcut.clone(),
                            i.enabled,
                            false,
                            i.icon.clone(),
                        ),
                        MenuEntry::Submenu { label, .. } => (label.clone(), None, true, true, None),
                        MenuEntry::Separator | MenuEntry::Hint(_) => unreachable!(),
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
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .items_center()
                                .gap(px(m.popup_row_icon_gap))
                                .children(row_icon.map(|i| icon(&i.path, m.icon_size, i.color)))
                                .child(label),
                        )
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
        .into_any_element()
    }
}

impl ContextMenu {
    /// The menu in the tui skin: a box of whole cells on the window's grid,
    /// a row per entry (`  Label      ⌘C `), separators as rules joining the
    /// frame, `▸` closing a submenu row, the selected row filled.
    fn render_tui(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let entity = cx.entity();
        let focused = self.focus_handle.is_focused(window);
        let inner = self
            .entries
            .iter()
            .map(|e| match e {
                MenuEntry::Item(i) => {
                    2 + Cell::text_cols(&i.label)
                        + i.icon.as_ref().map(|_| 2).unwrap_or(0)
                        + i.shortcut
                            .as_ref()
                            .map(|s| 3 + Cell::text_cols(s))
                            .unwrap_or(0)
                }
                MenuEntry::Submenu { label, .. } => 2 + Cell::text_cols(label) + 3,
                MenuEntry::Hint(text) => 2 + Cell::text_cols(text),
                MenuEntry::Separator => 0,
            })
            .max()
            .unwrap_or(0)
            .max(12);
        let viewport = window.viewport_size();
        let max_cols = Cell::fit(viewport.width, cell.w).max(4);
        let max_rows = Cell::fit(viewport.height, cell.h).max(3);
        let cols = (inner + 2).min(max_cols);
        let rows = (self.entries.len() + 2).min(max_rows);
        let (width, height) = (cell.cols(cols), cell.rows(rows));
        let left = skin::floor_to(
            self.position.x.min(viewport.width - width).max(px(0.)),
            cell.w,
        );
        let top = skin::floor_to(
            self.position.y.min(viewport.height - height).max(px(0.)),
            cell.h,
        );
        self.laid = Bounds {
            origin: point(left, top),
            size: Size { width, height },
        };
        let rules: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, MenuEntry::Separator))
            .map(|(ix, _)| ix + 1)
            .collect();
        let rows_el: Vec<AnyElement> = self
            .entries
            .iter()
            .enumerate()
            .map(|(ix, entry)| {
                let (label, shortcut, enabled, submenu, row_icon) = match entry {
                    MenuEntry::Separator => {
                        return div().h(px(cell.h)).flex_shrink_0().into_any_element();
                    }
                    // A dim row of its own, never selected.
                    MenuEntry::Hint(text) => {
                        return div()
                            .h(px(cell.h))
                            .w_full()
                            .flex_shrink_0()
                            .px(px(cell.w))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_color(theme.text_tertiary)
                            .child(text.clone())
                            .into_any_element();
                    }
                    MenuEntry::Item(i) => (
                        i.label.clone(),
                        i.shortcut.clone(),
                        i.enabled,
                        false,
                        i.icon.clone(),
                    ),
                    MenuEntry::Submenu { label, .. } => (label.clone(), None, true, true, None),
                };
                let selected = self.selected == Some(ix);
                let entity = entity.clone();
                div()
                    .id(ElementId::NamedInteger("menu-row".into(), ix as u64))
                    .h(px(cell.h))
                    .w_full()
                    .flex_shrink_0()
                    .flex()
                    .px(px(cell.w))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(if enabled {
                        theme.list_text
                    } else {
                        theme.text_disabled
                    })
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .when(selected && focused, |d| {
                                d.bg(theme.list_bg_focused)
                                    .text_color(theme.list_text_focused)
                            })
                            .when(selected && !focused, |d| d.bg(theme.list_bg_selected))
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
                    // An icon is a glyph in its cell, then a blank cell.
                    .children(row_icon.map(|i| {
                        div()
                            .flex_shrink_0()
                            .w(cell.cols(2))
                            .text_color(i.color)
                            .child(i.glyph)
                    }))
                    .child(div().flex_1().overflow_hidden().child(label))
                    .children(shortcut.map(|s| {
                        div()
                            .flex_shrink_0()
                            .pl(cell.cols(2))
                            .text_color(if enabled {
                                theme.text_tertiary
                            } else {
                                theme.text_disabled
                            })
                            .child(s)
                    }))
                    .when(submenu, |d| {
                        d.child(
                            div()
                                .flex_shrink_0()
                                .pl(cell.cols(2))
                                .text_color(theme.text_secondary)
                                .child("▸"),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        let body = div()
            .size_full()
            .flex()
            .flex_col()
            .children(rows_el)
            .into_any_element();
        let close = entity.clone();
        let child = self.child.as_ref().map(|(_, c, _)| c.clone());
        let frame = skin::boxed(
            cell,
            cols,
            rows,
            Vec::new(),
            rules,
            body,
            theme.popup_bg,
            if focused {
                theme.border_focused
            } else {
                theme.border
            },
            &theme,
            m,
        )
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
                if !this.contains(event.position, cx) {
                    this.dismiss(window, cx);
                }
            });
        })
        .occlude()
        .absolute()
        .left(left)
        .top(top)
        .text_color(theme.popup_text);
        // The box in window coordinates, and an open submenu beside it.
        deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_0()
                .child(frame)
                .children(child),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_closes_a_menu_unselected() {
        // The run icon's menu: two rows with icons, then the hint, which
        // keyboard and pointer pass over.
        let row = |label: &str| {
            let mut entry = MenuEntry::custom(label.to_string(), |_, _| {});
            if let MenuEntry::Item(item) = &mut entry {
                item.icon = Some(MenuIcon {
                    path: "weep/run-gutter".into(),
                    color: gpui::black(),
                    glyph: "▶",
                });
            }
            entry
        };
        let entries = [
            row("Run 'debug fixture'"),
            row("Debug 'debug fixture'"),
            MenuEntry::Hint("⇧Click on icon to Run, ⌥Click to Debug".into()),
        ];
        let selectable: Vec<bool> = entries.iter().map(MenuEntry::selectable).collect();
        assert_eq!(selectable, [true, true, false]);
        let copy = entries[2].clone_entry();
        assert!(matches!(copy, MenuEntry::Hint(t) if t.starts_with("⇧Click")));
        let copy = entries[0].clone_entry();
        assert!(
            matches!(copy, MenuEntry::Item(i) if i.icon.as_ref().is_some_and(|i| i.glyph == "▶"))
        );
    }
}
