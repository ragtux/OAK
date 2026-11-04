//! The Settings tool (`open-settings`): Fleet's settings pages as a tab in
//! the editor island. A 720 px column centred in the island holds the
//! search field, the scope row (Global, the workspace, Plugins), a section
//! tree on the left and the cards of controls on the right, which scroll
//! under the fixed tree. Every control writes its key to WEEP's
//! `settings.json` at once (`settings::set_value`), as Fleet's do. Measured
//! in batch 8 (`docs/fleet-probe.md`, seventh pass; `theme.rs::metrics`
//! `SETTINGS_*`). The schema of pages, keys and defaults is
//! `settings_schema.rs`.

use crate::actions::{EditKeymapFile, EditSettingsFile, PopupPageDown, PopupPageUp};
use crate::assets::icon;
use crate::input::{InputEvent, TextInput};
use crate::scrollbar::scrollbar;
use crate::search_tool::frame;
use crate::settings::{self, AppSettings};
use crate::settings_schema::{option_label, values_equal, Item, Kind, SECTIONS};
use crate::skin::{self, Cell, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    deferred, div, point, prelude::*, px, App, Bounds, BoxShadow, Context, DispatchPhase,
    ElementId, Entity, FocusHandle, Focusable, FontWeight, HighlightStyle, Hitbox, HitboxBehavior,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollHandle,
    ScrollWheelEvent, SharedString, StyledText, Window,
};
use serde_json::Value;
use std::collections::HashMap;
use std::rc::Rc;

/// A row of the section tree: a section, or one of its titled groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeRow {
    Section(usize),
    Group(usize, usize),
}

pub struct SettingsView {
    search: Entity<TextInput>,
    query: String,
    root_name: SharedString,
    selected: TreeRow,
    /// The text fields, one per key, made when first shown.
    fields: HashMap<&'static str, Entity<TextInput>>,
    /// The key of the open dropdown; its list hangs under the control.
    dropdown: Option<&'static str>,
    content_scroll: ScrollHandle,
    tree_scroll: ScrollHandle,
    focus_handle: FocusHandle,
    /// The tui skin's grid as last laid out, columns by rows of cells.
    tui_grid: Rc<std::cell::Cell<(usize, usize)>>,
    /// The lists' scroll bars on the island's frame (the tui skin).
    tui_bars: crate::skin::FrameBars,
    /// The tui skin's page and page list, scrolled by whole rows (rule 6).
    tui_scroll: skin::RowScroll,
    tui_tree_scroll: skin::RowScroll,
    /// Rows of the page and of the list, and the rows each shows, as last
    /// laid out (indexed by `TuiBar`).
    tui_extent: [(usize, usize); 2],
    /// The scroll bar being dragged.
    tui_drag: Option<TuiBar>,
}

/// The tui skin's two scroll bars: the page's, on the frame's right edge,
/// and the page list's, on the rule beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TuiBar {
    Page = 0,
    List = 1,
}

impl Focusable for SettingsView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
}

/// The current value of a key: the settings' merged value, else the schema's default.
fn current(item: &Item, cx: &App) -> Value {
    cx.global::<AppSettings>()
        .settings
        .get(item.key)
        .cloned()
        .unwrap_or_else(|| serde_json::from_str(item.default).unwrap_or(Value::Null))
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl SettingsView {
    pub fn new(root_name: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new("Search", cx));
        cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Changed = event {
                this.query = this.search.read(cx).text().to_string();
                cx.notify();
            }
        })
        .detach();
        SettingsView {
            search,
            query: String::new(),
            root_name: root_name.into(),
            selected: TreeRow::Section(0),
            fields: HashMap::new(),
            dropdown: None,
            content_scroll: ScrollHandle::new(),
            tree_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            tui_grid: Rc::new(std::cell::Cell::new((0, 0))),
            tui_bars: Default::default(),
            tui_scroll: skin::RowScroll::default(),
            tui_tree_scroll: skin::RowScroll::default(),
            tui_extent: [(0, 0); 2],
            tui_drag: None,
        }
    }

    fn matches(&self, item: &Item) -> bool {
        self.query.is_empty()
            || item
                .label
                .to_lowercase()
                .contains(&self.query.to_lowercase())
    }

    fn group_count(&self, section: usize, group: usize) -> usize {
        SECTIONS[section].groups[group]
            .items
            .iter()
            .filter(|i| self.matches(i))
            .count()
    }

    fn section_count(&self, section: usize) -> usize {
        (0..SECTIONS[section].groups.len())
            .map(|g| self.group_count(section, g))
            .sum()
    }

    fn total_count(&self) -> usize {
        (0..SECTIONS.len()).map(|s| self.section_count(s)).sum()
    }

    /// The height of one item row in a card.
    fn item_height(item: &Item, m: &Metrics) -> f32 {
        m.settings_row
            + item
                .description
                .map(|_| 2. * m.settings_description_line)
                .unwrap_or(0.)
    }

    /// Where a section (or a group inside it) starts in the content column.
    fn offset_of(&self, row: TreeRow, m: &Metrics) -> f32 {
        let mut y = 0.;
        for (s, section) in SECTIONS.iter().enumerate() {
            if self.section_count(s) == 0 {
                continue;
            }
            if row == TreeRow::Section(s) {
                return y;
            }
            y += m.settings_title_top + m.settings_title_line + m.settings_title_gap;
            y += m.settings_card_pad;
            let mut first = true;
            for (g, group) in section.groups.iter().enumerate() {
                if self.group_count(s, g) == 0 {
                    continue;
                }
                if row == TreeRow::Group(s, g) {
                    let lead = if first {
                        m.settings_card_pad + m.settings_title_gap
                    } else {
                        m.settings_group_spacer
                    };
                    return (y - lead).max(0.);
                }
                if group.title.is_some() {
                    if first {
                        first = false;
                    } else {
                        y += m.settings_group_spacer;
                    }
                    y += m.settings_group_top + m.settings_group_title_line + m.settings_group_gap;
                }
                for item in group.items.iter().filter(|i| self.matches(i)) {
                    y += Self::item_height(item, m);
                }
            }
            y += m.settings_card_pad + m.settings_card_gap;
        }
        y
    }

    /// Scrolls to the group titled `title` (`terminal/select-default-profile`
    /// lands on Tools > Terminal).
    pub fn show_group(&mut self, title: &str, cx: &mut Context<Self>) {
        for (s, section) in SECTIONS.iter().enumerate() {
            for (g, group) in section.groups.iter().enumerate() {
                if group.title == Some(title) {
                    self.select(TreeRow::Group(s, g), cx);
                    return;
                }
            }
        }
    }

    fn select(&mut self, row: TreeRow, cx: &mut Context<Self>) {
        self.selected = row;
        let m = *cx.global::<Metrics>();
        if Skin::current(cx) == Skin::Tui {
            // The page's top row becomes the section's title or the group's
            // heading (`render_tui_content` keeps it inside the page).
            let cell = Cell::current(cx);
            let width = self.tui_content_cols(cell);
            let lines = self.tui_lines(width);
            self.tui_scroll.top = lines
                .iter()
                .position(|l| match (l, row) {
                    (TuiLine::Section(a), TreeRow::Section(b)) => *a == b,
                    (TuiLine::Group(a, b, _), TreeRow::Group(c, d)) => (*a, *b) == (c, d),
                    _ => false,
                })
                .unwrap_or(0);
            cx.notify();
            return;
        }
        let y = self.offset_of(row, &m);
        self.content_scroll.set_offset(point(px(0.), px(-y)));
        cx.notify();
    }

    fn set(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        settings::set_value(key, value, cx);
        cx.notify();
    }

    fn field(&mut self, item: &Item, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some(f) = self.fields.get(item.key) {
            return f.clone();
        }
        let text = value_text(&current(item, cx));
        let key = item.key;
        let kind = item.kind;
        let input = cx.new(|cx| {
            let mut i = TextInput::new("", cx);
            i.set_text_at_end(&text, cx);
            i
        });
        cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
            if let InputEvent::Submit { .. } = event {
                let text = input.read(cx).text().trim().to_string();
                let value = match kind {
                    Kind::Number => match text.parse::<f64>() {
                        Ok(n) => serde_json::Number::from_f64(n)
                            .map(Value::Number)
                            .unwrap_or(Value::Null),
                        Err(_) => return,
                    },
                    _ => Value::String(text),
                };
                this.set(key, value, cx);
            }
        })
        .detach();
        self.fields.insert(item.key, input.clone());
        input
    }

    // ---- rendering --------------------------------------------------------------------

    fn render_tree(&self, theme: &Theme, m: &Metrics, cx: &Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        for (s, section) in SECTIONS.iter().enumerate() {
            let count = self.section_count(s);
            if count == 0 {
                continue;
            }
            let titled = section.groups.iter().any(|g| g.title.is_some());
            rows.push(self.tree_row(
                TreeRow::Section(s),
                section.title,
                titled,
                0,
                (!self.query.is_empty() && !titled).then_some(count),
                theme,
                m,
                &entity,
            ));
            if titled {
                for (g, group) in section.groups.iter().enumerate() {
                    let n = self.group_count(s, g);
                    if n == 0 {
                        continue;
                    }
                    if let Some(title) = group.title {
                        rows.push(self.tree_row(
                            TreeRow::Group(s, g),
                            title,
                            false,
                            1,
                            (!self.query.is_empty()).then_some(n),
                            theme,
                            m,
                            &entity,
                        ));
                    }
                }
            }
        }
        div()
            .id("settings-tree")
            .w(px(m.settings_tree_width))
            .flex_shrink_0()
            .h_full()
            .pt(px(m.settings_tree_top))
            .overflow_y_scroll()
            .track_scroll(&self.tree_scroll)
            .flex()
            .flex_col()
            .children(rows)
    }

    #[allow(clippy::too_many_arguments)]
    fn tree_row(
        &self,
        row: TreeRow,
        label: &'static str,
        chevron: bool,
        depth: usize,
        count: Option<usize>,
        theme: &Theme,
        m: &Metrics,
        entity: &Entity<SettingsView>,
    ) -> gpui::AnyElement {
        let selected = self.selected == row;
        let entity = entity.clone();
        let id = match row {
            TreeRow::Section(s) => ElementId::NamedInteger("settings-section".into(), s as u64),
            TreeRow::Group(s, g) => {
                ElementId::NamedInteger("settings-group".into(), (s * 100 + g) as u64)
            }
        };
        div()
            .h(px(m.settings_tree_row))
            .flex_shrink_0()
            .py(px((m.settings_tree_row - m.settings_tree_wash) / 2.))
            .child(
                div()
                    .id(id)
                    .h_full()
                    .relative()
                    .pl(px(
                        m.settings_tree_pad_x + depth as f32 * m.settings_tree_indent
                    ))
                    .pr(px(m.popup_row_pad_x))
                    .flex()
                    .items_center()
                    .gap(px(m.popup_row_icon_gap))
                    .rounded(px(m.popup_row_radius))
                    .cursor_pointer()
                    .whitespace_nowrap()
                    .text_color(theme.list_text)
                    .when(selected, |d| d.bg(theme.list_bg_selected))
                    .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.select(row, cx)))
                    .when(chevron, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(px(m.settings_tree_chevron_x))
                                .size(px(m.icon_size))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(
                                    "expui/general/chevronDown",
                                    m.icon_size,
                                    theme.icon_secondary,
                                )),
                        )
                    })
                    .child(label)
                    .children(count.map(|n| {
                        div()
                            .text_color(theme.text_secondary)
                            .child(SharedString::from(n.to_string()))
                    })),
            )
            .into_any_element()
    }

    fn highlighted(&self, text: &str, theme: &Theme) -> StyledText {
        let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
        if !self.query.is_empty() {
            let lower = text.to_lowercase();
            let q = self.query.to_lowercase();
            let mut from = 0;
            while let Some(pos) = lower[from..].find(&q) {
                let start = from + pos;
                highlights.push((
                    start..start + q.len(),
                    HighlightStyle {
                        color: Some(theme.search_match_text),
                        background_color: Some(theme.search_match_bg),
                        ..Default::default()
                    },
                ));
                from = start + q.len().max(1);
                if from >= lower.len() {
                    break;
                }
            }
        }
        StyledText::new(SharedString::from(text.to_string())).with_highlights(highlights)
    }

    fn render_item(
        &mut self,
        item: &'static Item,
        theme: &Theme,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let entity = cx.entity();
        let value = current(item, cx);
        let modified = cx.global::<AppSettings>().settings.is_own(item.key);
        let mark = modified.then(|| {
            div()
                .absolute()
                .left_0()
                .top(px((m.settings_row - m.settings_modified_height) / 2.))
                .w(px(m.settings_modified_width))
                .h(px(m.settings_modified_height))
                .bg(theme.settings_modified)
        });
        let indent = if item.indent {
            m.settings_card_pad_x
        } else {
            0.
        };
        let label = self.highlighted(item.label, theme);
        let control: gpui::AnyElement = match item.kind {
            Kind::Checkbox => {
                let on = value.as_bool().unwrap_or(false);
                let key = item.key;
                let toggle = entity.clone();
                return div()
                    .min_h(px(Self::item_height(item, m)))
                    .relative()
                    .children(mark)
                    .pl(px(m.settings_card_pad_x + indent))
                    .pr(px(m.settings_card_pad_right))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .child(
                        div()
                            .id(ElementId::Name(
                                format!("settings-check-{}", item.key).into(),
                            ))
                            .flex()
                            .items_center()
                            .gap(px(m.settings_checkbox_gap))
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                toggle.update(cx, |this, cx| this.set(key, Value::Bool(!on), cx))
                            })
                            .child(
                                div()
                                    .size(px(m.settings_checkbox))
                                    .flex_shrink_0()
                                    .rounded(px(3. * m.zoom))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .map(|d| {
                                        if on {
                                            d.bg(theme.checkbox_on_bg)
                                        } else {
                                            d.bg(theme.checkbox_off_bg)
                                                .border_1()
                                                .border_color(theme.checkbox_off_border)
                                        }
                                    })
                                    .when(on, |d| {
                                        d.child(
                                            div()
                                                .absolute()
                                                .size(px(m.settings_checkbox))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .text_color(theme.checkbox_icon)
                                                .text_size(px(11. * m.zoom))
                                                .font_weight(FontWeight::BOLD)
                                                .child("✓"),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(theme.text_primary)
                                    .child(label),
                            ),
                    )
                    .children(item.description.map(|d| {
                        div()
                            .pl(px(m.settings_checkbox + m.settings_checkbox_gap))
                            .line_height(px(m.settings_description_line))
                            .text_color(theme.text_secondary)
                            .child(d)
                    }))
                    .into_any_element();
            }
            Kind::Dropdown(options) => {
                let key = item.key;
                let open = entity.clone();
                let list = (self.dropdown == Some(key))
                    .then(|| self.render_options(item, options, &value, theme, m, cx));
                div()
                    .id(ElementId::Name(
                        format!("settings-drop-{}", item.key).into(),
                    ))
                    .flex_1()
                    .min_w_0()
                    .relative()
                    .children(list)
                    .h(px(m.settings_control_height))
                    .px(px(m.settings_control_pad_x))
                    .flex()
                    .items_center()
                    .rounded(px(m.button_radius))
                    .border_1()
                    .border_color(theme.dropdown_border)
                    .bg(theme.dropdown_bg)
                    .text_color(theme.dropdown_text)
                    .cursor_pointer()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .on_click(move |_, _, cx| {
                        open.update(cx, |this, cx| {
                            this.dropdown = match this.dropdown {
                                Some(k) if k == key => None,
                                _ => Some(key),
                            };
                            cx.notify();
                        })
                    })
                    .child(
                        div()
                            .flex_1()
                            .child(SharedString::from(option_label(options, &value))),
                    )
                    .child(icon(
                        "expui/general/chevronDown",
                        m.icon_size,
                        theme.icon_secondary,
                    ))
                    .into_any_element()
            }
            Kind::Text | Kind::Number => {
                let input = self.field(item, cx);
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(m.settings_control_height))
                    .px(px(m.settings_control_pad_x))
                    .flex()
                    .items_center()
                    .rounded(px(m.button_radius))
                    .border_1()
                    .border_color(theme.input_border)
                    .bg(theme.input_bg)
                    .child(div().flex_1().min_w_0().h_full().child(input))
                    .into_any_element()
            }
            Kind::Link => div()
                .id(ElementId::Name(
                    format!("settings-link-{}", item.key).into(),
                ))
                .text_color(theme.link)
                .cursor_pointer()
                .on_click(|_, window, cx| window.dispatch_action(Box::new(EditSettingsFile), cx))
                .child("Edit in settings.json")
                .into_any_element(),
        };
        let keymap = item.key == "keymap";
        div()
            .h(px(m.settings_row))
            .pl(px(m.settings_card_pad_x + indent))
            .pr(px(m.settings_card_pad_right))
            .flex_shrink_0()
            .relative()
            .children(mark)
            .flex()
            .items_center()
            .child(
                div()
                    .min_w(px(m.settings_label_width - indent))
                    .flex_shrink_0()
                    .pr(px(m.settings_label_gap))
                    .whitespace_nowrap()
                    .text_color(theme.text_primary)
                    .child(label),
            )
            .child(control)
            .when(keymap, |d| {
                // Fleet's pencil after the keymap: `edit-keymap-file`.
                d.child(
                    div()
                        .id("settings-edit-keymap")
                        .absolute()
                        .right(px(m.settings_label_gap))
                        .size(px(m.goto_icon_button))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(m.button_radius))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.ghost_hovered))
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(EditKeymapFile), cx)
                        })
                        .child(icon(
                            "expui/general/edit",
                            m.icon_size,
                            theme.icon_secondary,
                        )),
                )
            })
            .into_any_element()
    }

    fn render_content(
        &mut self,
        theme: &Theme,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut blocks: Vec<gpui::AnyElement> = Vec::new();
        for (s, section) in SECTIONS.iter().enumerate() {
            if self.section_count(s) == 0 {
                continue;
            }
            let mut card: Vec<gpui::AnyElement> = Vec::new();
            for (g, group) in section.groups.iter().enumerate() {
                if self.group_count(s, g) == 0 {
                    continue;
                }
                if let Some(title) = group.title {
                    if !card.is_empty() {
                        card.push(div().h(px(m.settings_group_spacer)).into_any_element());
                    }
                    card.push(
                        div()
                            .h(px(m.settings_group_title_line))
                            .mt(px(m.settings_group_top))
                            .mb(px(m.settings_group_gap))
                            .pl(px(m.settings_card_pad_x))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .text_size(px(m.settings_group_title_size))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_primary)
                            .child(title)
                            .into_any_element(),
                    );
                }
                for item in group.items.iter() {
                    if self.matches(item) {
                        card.push(self.render_item(item, theme, m, cx));
                    }
                }
            }
            blocks.push(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .mt(px(m.settings_title_top))
                            .h(px(m.settings_title_line))
                            .mb(px(m.settings_title_gap))
                            .pl(px(m.settings_title_pad_x))
                            .flex()
                            .items_center()
                            .text_size(px(m.settings_title_size))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_primary)
                            .child(section.title),
                    )
                    .child(
                        div()
                            .py(px(m.settings_card_pad))
                            .rounded(px(m.settings_card_radius))
                            .bg(theme.background_secondary)
                            .flex()
                            .flex_col()
                            .children(card),
                    )
                    .child(div().h(px(m.settings_card_gap)))
                    .into_any_element(),
            );
        }
        if blocks.is_empty() {
            blocks.push(
                div()
                    .mt(px(m.settings_title_top))
                    .text_color(theme.text_secondary)
                    .child("Nothing matches")
                    .into_any_element(),
            );
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .relative()
            .child(
                div()
                    .id("settings-content")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.content_scroll)
                    .flex()
                    .flex_col()
                    .children(blocks),
            )
            .child(scrollbar(
                "settings-scrollbar",
                &self.content_scroll,
                m.settings_scrollbar,
                theme,
                m,
            ))
    }

    fn page(&mut self, dir: f32, cx: &mut Context<Self>) {
        if Skin::current(cx) == Skin::Tui {
            // Whole rows: a page less one row.
            let (total, visible) = self.tui_extent[TuiBar::Page as usize];
            let step = visible.saturating_sub(1).max(1);
            let top = self.tui_scroll.top;
            let top = if dir > 0. {
                top + step
            } else {
                top.saturating_sub(step)
            };
            self.tui_scroll.top = top.min(total.saturating_sub(visible));
            cx.notify();
            return;
        }
        let step = (self.content_scroll.bounds().size.height - px(40.)).max(px(40.));
        let max = self.content_scroll.max_offset().y;
        let y = (self.content_scroll.offset().y - step * dir).clamp(-max, px(0.));
        self.content_scroll.set_offset(point(px(0.), y));
        cx.notify();
    }

    fn page_down(&mut self, _: &PopupPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page(1., cx);
    }

    fn page_up(&mut self, _: &PopupPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page(-1., cx);
    }

    /// The open dropdown's list, hung under its control (ninth pass).
    fn render_options(
        &self,
        item: &'static Item,
        options: &'static [(&'static str, &'static str)],
        value: &Value,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let key = item.key;
        let entity = cx.entity();
        let close = entity.clone();
        let rows = options.iter().enumerate().map(|(ix, (json, label))| {
            let chosen = serde_json::from_str::<Value>(json)
                .map(|v| values_equal(&v, value))
                .unwrap_or(false);
            let entity = entity.clone();
            let json = *json;
            div()
                .id(ElementId::NamedInteger("settings-option".into(), ix as u64))
                .h(px(m.settings_option_wash))
                .px(px(m.popup_row_pad_x))
                .flex_shrink_0()
                .flex()
                .items_center()
                .rounded(px(m.popup_row_radius))
                .cursor_pointer()
                .whitespace_nowrap()
                .text_color(theme.list_text)
                .when(chosen, |d| d.bg(theme.list_bg_focused))
                .hover(|s| s.bg(theme.list_bg_hovered))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        if let Ok(v) = serde_json::from_str::<Value>(json) {
                            this.set(key, v, cx);
                        }
                        this.dropdown = None;
                        cx.notify();
                    })
                })
                .child(*label)
        });
        deferred(
            div()
                .id("settings-dropdown")
                .occlude()
                .absolute()
                .left(px(-m.settings_options_overhang))
                .right(px(-m.settings_options_overhang))
                .top(px(m.settings_control_height + m.settings_options_gap))
                .pt(px(m.settings_options_pad))
                .pb(px(m.settings_options_pad + 4.))
                .px(px(m.settings_options_pad_x))
                .gap(px(m.settings_option_gap))
                .flex()
                .flex_col()
                .rounded(px(m.popup_radius))
                .border_1()
                .border_color(theme.popup_border)
                .bg(theme.popup_bg)
                .shadow(vec![BoxShadow {
                    color: theme.shadow_medium,
                    offset: point(px(0.), px(m.popup_shadow_offset_y)),
                    blur_radius: px(m.popup_shadow_blur),
                    spread_radius: px(0.),
                }])
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |this, cx| {
                        this.dropdown = None;
                        cx.notify();
                    })
                })
                .children(rows),
        )
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let focused = self.search.read(cx).is_focused(window);
        let count = (!self.query.is_empty()).then(|| self.total_count());
        if Skin::current(cx) == Skin::Tui {
            return self
                .render_tui(count, &theme, &m, window, cx)
                .into_any_element();
        }
        div()
            .key_context("SettingsTool")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::page_up))
            .size_full()
            .flex()
            .justify_center()
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            .text_color(theme.text_primary)
            .child(
                div()
                    .w(px(m.settings_column_width))
                    .h_full()
                    .flex()
                    .flex_col()
                    // The search field, 32 under the tab row, with the focus ring.
                    .child(
                        div()
                            .mt(px(m.settings_field_top))
                            .flex_shrink_0()
                            .flex()
                            .child(
                                frame(focused, window, &theme, &m)
                                    .h(px(m.settings_field_height))
                                    .pl(px(m.settings_control_pad_x))
                                    .gap(px(m.popup_row_icon_gap))
                                    .child(icon(
                                        "expui/general/search",
                                        m.icon_size,
                                        theme.icon_secondary,
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .h_full()
                                            .child(self.search.clone()),
                                    ),
                            ),
                    )
                    // The scope row: Global, the workspace, Plugins.
                    .child(
                        div()
                            .mt(px(m.settings_scope_gap))
                            .h(px(m.settings_scope_height))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .h(px(m.settings_scope_height))
                                    .px(px(m.settings_scope_pad_x))
                                    .flex()
                                    .items_center()
                                    .gap(px(m.settings_checkbox_gap))
                                    .rounded(px(m.button_radius))
                                    .border_1()
                                    .border_color(theme.tab_border_selected_focused)
                                    .bg(theme.tab_bg_selected_focused)
                                    .text_color(theme.tab_text_selected)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Global")
                                    .children(count.map(|n| {
                                        div()
                                            .font_weight(FontWeight::NORMAL)
                                            .text_color(theme.text_secondary)
                                            .child(SharedString::from(n.to_string()))
                                    })),
                            )
                            .child(
                                div()
                                    .ml(px(m.settings_scope_name_gap))
                                    .text_color(theme.text_secondary)
                                    .whitespace_nowrap()
                                    .child(self.root_name.clone()),
                            )
                            .child(div().flex_1())
                            .child(div().text_color(theme.link).child("Plugins")),
                    )
                    .child(
                        div()
                            .mt(px(m.settings_separator_gap))
                            .h(px(1.))
                            .flex_shrink_0()
                            .bg(theme.separator),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .gap(px(m.settings_content_gap))
                            .child(self.render_tree(&theme, &m, cx))
                            .child(self.render_content(&theme, &m, cx)),
                    ),
            )
            .into_any_element()
    }
}

// ---- the tui skin -----------------------------------------------------------------------

/// One row of the page in the tui skin.
#[derive(Clone, Debug)]
enum TuiLine {
    /// A section's title, bold.
    Section(usize),
    /// A group's heading: a rule with the title set in.
    Group(usize, usize, &'static str),
    Item(&'static Item),
    /// A line of an item's description, under its label.
    Description(String, bool),
    Blank,
}

/// The page list's width in cells.
const TUI_TREE_COLS: usize = 24;
/// A dropdown's or a field's width in cells.
const TUI_VALUE_COLS: usize = 24;

/// `text` wrapped on words into lines of at most `width` chars.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// `text` cut to `width` chars, the last one an ellipsis when cut.
fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

impl SettingsView {
    /// The page list's columns for a view `cols` wide.
    fn tui_tree_cols(cols: usize) -> usize {
        TUI_TREE_COLS.min(cols / 3)
    }

    /// The page's columns: right of the list, its rule and a blank column,
    /// and one cell in from the frame.
    fn tui_content_cols(&self, _cell: Cell) -> usize {
        let cols = self.tui_grid.get().0;
        cols.saturating_sub(Self::tui_tree_cols(cols) + 3)
    }

    /// The rows the page and the list show: under the search row and its
    /// rule.
    fn tui_visible(&self) -> usize {
        self.tui_grid.get().1.saturating_sub(2)
    }

    /// The page as rows: each section's title, its groups' headings, the
    /// items and their descriptions wrapped to `width`.
    fn tui_lines(&self, width: usize) -> Vec<TuiLine> {
        let mut lines = Vec::new();
        for (s, section) in SECTIONS.iter().enumerate() {
            if self.section_count(s) == 0 {
                continue;
            }
            if !lines.is_empty() {
                lines.push(TuiLine::Blank);
            }
            lines.push(TuiLine::Section(s));
            for (g, group) in section.groups.iter().enumerate() {
                if self.group_count(s, g) == 0 {
                    continue;
                }
                if let Some(title) = group.title {
                    lines.push(TuiLine::Blank);
                    lines.push(TuiLine::Group(s, g, title));
                }
                for item in group.items.iter().filter(|i| self.matches(i)) {
                    lines.push(TuiLine::Item(item));
                    if let Some(d) = item.description {
                        let indent = 2 + if item.indent { 2 } else { 0 };
                        for line in wrap(d, width.saturating_sub(indent + 1).max(8)) {
                            lines.push(TuiLine::Description(line, item.indent));
                        }
                    }
                }
            }
        }
        lines
    }

    /// The scroll of the page (`TuiBar::Page`) or of the list.
    fn tui_rows_mut(&mut self, bar: TuiBar) -> &mut skin::RowScroll {
        match bar {
            TuiBar::Page => &mut self.tui_scroll,
            TuiBar::List => &mut self.tui_tree_scroll,
        }
    }

    /// A click or a drag at window `y` on a scroll bar (`bounds`): the
    /// thumb centres there, on a whole row.
    fn tui_bar_to(
        &mut self,
        bar: TuiBar,
        y: Pixels,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if bounds.size.height <= px(0.) {
            return;
        }
        let (total, visible) = self.tui_extent[bar as usize];
        let frac = ((y - bounds.origin.y) / bounds.size.height).clamp(0., 1.);
        let top = (frac * total as f32 - visible as f32 / 2.).round().max(0.) as usize;
        self.tui_rows_mut(bar).top = top.min(total.saturating_sub(visible));
        cx.notify();
    }

    /// What the tab sets on its island's frame in the tui skin: the rule
    /// under the search row (`├──┤`), the rule between the page list and
    /// the page from it to the bottom edge (`┬ │ ┴`), and the page's
    /// scroll bar on the right edge.
    pub fn tui_edges(&self, cell: Cell, theme: &Theme, m: Metrics) -> skin::Edges {
        let cols = self.tui_grid.get().0;
        skin::Edges {
            rules_from_top: vec![2],
            vrules: vec![(Self::tui_tree_cols(cols) + 1, 2)],
            right: Some(self.tui_bars.edge(cell, theme.scrollbar_thumb, m)),
            ..Default::default()
        }
    }

    fn render_tui(
        &mut self,
        count: Option<usize>,
        theme: &Theme,
        m: &Metrics,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.tui_bars.clear();
        let cell = Cell::current(cx);
        let cols = self.tui_grid.get().0;
        let visible = self.tui_visible();
        let tree_cols = Self::tui_tree_cols(cols);
        let width = self.tui_content_cols(cell);
        let measure = skin::measure(cell, &self.tui_grid);
        let tree = self.render_tui_tree(tree_cols, visible, cell, theme, cx);
        let content = self.render_tui_content(width, visible, cell, theme, m, cx);
        // Row 0: one cell in, the search field (its first cell the mark:
        // `⌕`, or the input's `›` while focused), then the scope: `Global`
        // filled, the workspace, Plugins, ending one cell in.
        let global = match count {
            Some(n) => format!(" Global {n} "),
            None => " Global ".to_string(),
        };
        let root = format!(" {} ", fit(&self.root_name, 20));
        let plugins = " Plugins";
        let scope_cols =
            Cell::text_cols(&global) + Cell::text_cols(&root) + Cell::text_cols(plugins);
        let field_cols = cols.saturating_sub(scope_cols + 3);
        let searching = self.search.read(cx).is_focused(window);
        let search = div()
            .h(px(cell.h))
            .w(cell.cols(cols))
            .flex()
            .whitespace_nowrap()
            .child(div().w(cell.cols(1)).flex_shrink_0())
            .child(
                div()
                    .relative()
                    .w(cell.cols(field_cols))
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .child(self.search.clone())
                    .when(!searching, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .w(cell.cols(1))
                                .text_color(theme.text_tertiary)
                                .child("⌕"),
                        )
                    }),
            )
            .child(div().w(cell.cols(1)).flex_shrink_0())
            .child(
                div()
                    .flex_shrink_0()
                    .bg(theme.tab_bg_selected_focused)
                    .text_color(theme.tab_text_selected)
                    .font_weight(FontWeight::BOLD)
                    .child(SharedString::from(global)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(theme.text_secondary)
                    .child(SharedString::from(root)),
            )
            .child(div().flex_shrink_0().text_color(theme.link).child(plugins));
        // The page's scroll bar is the frame's right edge (`tui_edges`).
        let page_bar = {
            let (total, page_rows) = self.tui_extent[TuiBar::Page as usize];
            let entity = cx.entity();
            div()
                .absolute()
                .left_0()
                .top(cell.rows(2))
                .w(cell.cols(1))
                .h(cell.rows(visible))
                .child(
                    self.tui_bars
                        .add(self.tui_scroll.top, page_rows, total, move |to, cx| {
                            entity.update(cx, |this, cx| {
                                this.tui_scroll.top = to;
                                cx.notify();
                            })
                        }),
                )
        };
        let list_bar = self
            .tui_bar(TuiBar::List, cell, cx)
            .absolute()
            .left(cell.cols(tree_cols))
            .top(cell.rows(2))
            .w(cell.cols(1))
            .h(cell.rows(visible));
        div()
            .key_context("SettingsTool")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::page_up))
            .size_full()
            .relative()
            .font_family(theme.mono_font.clone())
            .text_size(px(cell.font_size))
            .line_height(px(cell.h))
            .text_color(theme.text_primary)
            .child(measure)
            .child(div().absolute().top_0().left_0().child(search))
            .child(
                div()
                    .id("settings-tree-tui")
                    .absolute()
                    .left_0()
                    .top(cell.rows(2))
                    .w(cell.cols(tree_cols))
                    .h(cell.rows(visible))
                    .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                        let (total, visible) = this.tui_extent[TuiBar::List as usize];
                        if this.tui_tree_scroll.wheel(ev, cell, total, visible) {
                            cx.notify();
                        }
                    }))
                    .child(tree),
            )
            .child(list_bar)
            .child(
                div()
                    .id("settings-page-tui")
                    .absolute()
                    .left(cell.cols(tree_cols + 2))
                    .top(cell.rows(2))
                    .w(cell.cols(width))
                    .h(cell.rows(visible))
                    .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                        let (total, visible) = this.tui_extent[TuiBar::Page as usize];
                        if this.tui_scroll.wheel(ev, cell, total, visible) {
                            cx.notify();
                        }
                    }))
                    .child(content),
            )
            .child(page_bar)
    }

    /// A scroll bar in a one-cell column: the thumb in whole rows on the
    /// line the column carries (the frame's right edge for the page, the
    /// list's rule for the list); a click or a drag scrolls.
    fn tui_bar(&self, bar: TuiBar, cell: Cell, cx: &Context<Self>) -> gpui::Canvas<Hitbox> {
        let entity = cx.entity();
        gpui::canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, cx| {
                let m = *cx.global::<Metrics>();
                let theme = cx.global::<Theme>().clone();
                let (top, (total, visible), dragging) = {
                    let this = entity.read(cx);
                    let top = match bar {
                        TuiBar::Page => this.tui_scroll.top,
                        TuiBar::List => this.tui_tree_scroll.top,
                    };
                    (
                        top,
                        this.tui_extent[bar as usize],
                        this.tui_drag == Some(bar),
                    )
                };
                if total <= visible {
                    return;
                }
                let thumb = if dragging {
                    theme.scrollbar_thumb_pressed
                } else if hitbox.is_hovered(window) {
                    theme.scrollbar_thumb_hovered
                } else {
                    theme.scrollbar_thumb
                };
                skin::paint_scrollbar(
                    window,
                    bounds,
                    cell,
                    top as f32,
                    visible as f32,
                    total as f32,
                    None,
                    thumb,
                    &m,
                );
                let (down, moved, up) = (entity.clone(), entity.clone(), entity.clone());
                window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble
                        && ev.button == MouseButton::Left
                        && hitbox.is_hovered(window)
                    {
                        down.update(cx, |this, cx| {
                            this.tui_drag = Some(bar);
                            this.tui_bar_to(bar, ev.position.y, bounds, cx);
                        });
                    }
                });
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && ev.pressed_button == Some(MouseButton::Left)
                        && moved.read(cx).tui_drag == Some(bar)
                    {
                        moved.update(cx, |this, cx| {
                            this.tui_bar_to(bar, ev.position.y, bounds, cx)
                        });
                    }
                });
                window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && ev.button == MouseButton::Left
                        && up.read(cx).tui_drag == Some(bar)
                    {
                        up.update(cx, |this, cx| {
                            this.tui_drag = None;
                            cx.notify();
                        });
                    }
                });
            },
        )
    }

    /// The page list: a row per section and titled group, the selected one
    /// filled; `visible` rows from the list's scroll.
    fn render_tui_tree(
        &mut self,
        cols: usize,
        visible: usize,
        cell: Cell,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut push = |row: TreeRow, text: String, count: Option<usize>| {
            let selected = self.selected == row;
            let entity = entity.clone();
            let id = match row {
                TreeRow::Section(s) => ElementId::NamedInteger("settings-section".into(), s as u64),
                TreeRow::Group(s, g) => {
                    ElementId::NamedInteger("settings-group".into(), (s * 100 + g) as u64)
                }
            };
            let count = count.map(|n| format!(" {n}")).unwrap_or_default();
            let room = cols.saturating_sub(count.chars().count() + 1);
            rows.push(
                div()
                    .id(id)
                    .h(px(cell.h))
                    .w(cell.cols(cols))
                    .flex_shrink_0()
                    .flex()
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .text_color(theme.list_text)
                    .when(selected, |d| {
                        d.bg(theme.list_bg_selected).font_weight(FontWeight::BOLD)
                    })
                    .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.select(row, cx)))
                    .child(SharedString::from(fit(&text, room)))
                    .child(
                        div()
                            .text_color(theme.text_secondary)
                            .child(SharedString::from(count)),
                    )
                    .into_any_element(),
            );
        };
        for (s, section) in SECTIONS.iter().enumerate() {
            let count = self.section_count(s);
            if count == 0 {
                continue;
            }
            let titled = section.groups.iter().any(|g| g.title.is_some());
            let mark = if titled { "▾" } else { " " };
            push(
                TreeRow::Section(s),
                format!(" {mark} {}", section.title),
                (!self.query.is_empty() && !titled).then_some(count),
            );
            if titled {
                for (g, group) in section.groups.iter().enumerate() {
                    let n = self.group_count(s, g);
                    if let (Some(title), true) = (group.title, n > 0) {
                        push(
                            TreeRow::Group(s, g),
                            format!("     {title}"),
                            (!self.query.is_empty()).then_some(n),
                        );
                    }
                }
            }
        }
        let total = rows.len();
        self.tui_extent[TuiBar::List as usize] = (total, visible);
        let scroll = &mut self.tui_tree_scroll;
        scroll.top = scroll.top.min(total.saturating_sub(visible));
        let top = scroll.top;
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .children(rows.into_iter().skip(top).take(visible))
    }

    /// The page: the rows of `tui_lines`, `width` cells wide, `visible` of
    /// them from the page's scroll.
    fn render_tui_content(
        &mut self,
        width: usize,
        visible: usize,
        cell: Cell,
        theme: &Theme,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let lines = self.tui_lines(width);
        let total = lines.len();
        self.tui_extent[TuiBar::Page as usize] = (total, visible);
        self.tui_scroll.top = self.tui_scroll.top.min(total.saturating_sub(visible));
        let top = self.tui_scroll.top;
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let row = || {
            div()
                .h(px(cell.h))
                .w(cell.cols(width))
                .flex()
                .flex_shrink_0()
                .whitespace_nowrap()
        };
        for (i, line) in lines.into_iter().enumerate().skip(top).take(visible) {
            rows.push(match line {
                TuiLine::Blank => row().into_any_element(),
                TuiLine::Section(s) => row()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.text_primary)
                    .child(SECTIONS[s].title)
                    .into_any_element(),
                TuiLine::Group(_, _, title) => skin::section(
                    cell,
                    width,
                    vec![div()
                        .flex_shrink_0()
                        .text_color(theme.text_primary)
                        .font_weight(FontWeight::BOLD)
                        .child(SharedString::from(format!(" {title} ")))
                        .into_any_element()],
                    theme.border,
                    theme.island_background,
                    *m,
                )
                .into_any_element(),
                TuiLine::Description(text, indent) => row()
                    .pl(cell.cols(2 + if indent { 2 } else { 0 }))
                    .text_color(theme.text_secondary)
                    .child(text)
                    .into_any_element(),
                TuiLine::Item(item) => {
                    // The rows a dropdown's box has under and over its row.
                    let at = i - top;
                    let room = (visible.saturating_sub(at + 1), at + 2);
                    self.render_tui_item(item, width, room, cell, theme, m, cx)
                }
            });
        }
        if rows.is_empty() {
            rows.push(
                row()
                    .text_color(theme.text_tertiary)
                    .child("Nothing matches")
                    .into_any_element(),
            );
        }
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .children(rows)
    }

    /// A setting as a row: `● Label ........ value`, the mark when the key is
    /// set in WEEP's file, the leader dots in `text_tertiary`, the value
    /// right-aligned (`[x]`, `[ dark      ▾ ]`, a field). `room` is the rows
    /// under and over the row, for a dropdown's box.
    #[allow(clippy::too_many_arguments)]
    fn render_tui_item(
        &mut self,
        item: &'static Item,
        width: usize,
        room: (usize, usize),
        cell: Cell,
        theme: &Theme,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let entity = cx.entity();
        let value = current(item, cx);
        let modified = cx.global::<AppSettings>().settings.is_own(item.key);
        let indent = if item.indent { 2 } else { 0 };
        let key = item.key;
        let value_cols = TUI_VALUE_COLS.min(width / 2).max(8);
        let keymap = key == "keymap";
        let (control, control_cols): (gpui::AnyElement, usize) = match item.kind {
            Kind::Checkbox => {
                let on = value.as_bool().unwrap_or(false);
                (
                    div()
                        .text_color(if on {
                            theme.text_primary
                        } else {
                            theme.text_secondary
                        })
                        .child(skin::checkbox(on))
                        .into_any_element(),
                    3,
                )
            }
            Kind::Dropdown(options) => {
                let cols = if keymap {
                    value_cols.saturating_sub(9).max(8)
                } else {
                    value_cols
                };
                let text = fit(&option_label(options, &value), cols - 6);
                let pad = cols - 6 - text.chars().count();
                let open = entity.clone();
                let list = (self.dropdown == Some(key)).then(|| {
                    self.render_tui_options(item, options, &value, cols, room, cell, theme, m, cx)
                });
                (
                    div()
                        .id(ElementId::Name(format!("settings-drop-{key}").into()))
                        .relative()
                        .w(cell.cols(cols))
                        .flex_shrink_0()
                        .cursor_pointer()
                        .text_color(theme.dropdown_text)
                        .hover(|s| s.bg(theme.list_bg_hovered))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            open.update(cx, |this, cx| {
                                this.dropdown = match this.dropdown {
                                    Some(k) if k == key => None,
                                    _ => Some(key),
                                };
                                cx.notify();
                            })
                        })
                        .child(SharedString::from(format!(
                            "[ {text}{} ▾ ]",
                            " ".repeat(pad)
                        )))
                        .children(list)
                        .into_any_element(),
                    cols,
                )
            }
            Kind::Text | Kind::Number => {
                let input = self.field(item, cx);
                // `[›value       ]`: the field between brackets, on
                // `input_bg`; its first cell is the input's `›` mark.
                (
                    div()
                        .w(cell.cols(value_cols))
                        .h(px(cell.h))
                        .flex()
                        .flex_shrink_0()
                        .text_color(theme.text_secondary)
                        .child(div().w(cell.cols(1)).flex_shrink_0().child("["))
                        .child(
                            div()
                                .w(cell.cols(value_cols - 2))
                                .h(px(cell.h))
                                .flex_shrink_0()
                                .child(input),
                        )
                        .child(div().w(cell.cols(1)).flex_shrink_0().child("]"))
                        .into_any_element(),
                    value_cols,
                )
            }
            Kind::Link => (
                div()
                    .id(ElementId::Name(format!("settings-link-{key}").into()))
                    .text_color(theme.link)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.list_bg_hovered))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(EditSettingsFile), cx)
                    })
                    .child("Edit in settings.json")
                    .into_any_element(),
                21,
            ),
        };
        let extra = if keymap { 9 } else { 0 };
        let label_cols = Cell::text_cols(item.label);
        let room = width.saturating_sub(2 + indent + 2 + control_cols + extra);
        let label_cols = label_cols.min(room);
        let dots = room - label_cols;
        let label: gpui::AnyElement = if label_cols < Cell::text_cols(item.label) {
            SharedString::from(fit(item.label, label_cols)).into_any_element()
        } else {
            self.highlighted(item.label, theme).into_any_element()
        };
        let toggle = entity.clone();
        let checkbox = matches!(item.kind, Kind::Checkbox);
        let on = value.as_bool().unwrap_or(false);
        div()
            .id(ElementId::Name(format!("settings-row-{key}").into()))
            .h(px(cell.h))
            .w(cell.cols(width))
            .flex()
            .flex_shrink_0()
            .whitespace_nowrap()
            .when(checkbox, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(theme.list_bg_hovered))
                    .on_click(move |_, _, cx| {
                        toggle.update(cx, |this, cx| this.set(key, Value::Bool(!on), cx))
                    })
            })
            .child(
                div()
                    .w(cell.cols(2 + indent))
                    .flex_shrink_0()
                    .text_color(theme.settings_modified)
                    .when(modified, |d| d.child("●")),
            )
            .child(
                div()
                    .w(cell.cols(label_cols))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .text_color(theme.text_primary)
                    .child(label),
            )
            .child(
                div()
                    .w(cell.cols(dots + 2))
                    .flex_shrink_0()
                    .text_color(theme.text_tertiary)
                    .child(SharedString::from(if dots > 0 {
                        format!(" {} ", ".".repeat(dots))
                    } else {
                        String::new()
                    })),
            )
            .child(control)
            .when(keymap, |d| {
                d.child(div().w(cell.cols(1)).flex_shrink_0()).child(
                    skin::button("settings-edit-keymap", "Edit", false, theme, cell).on_click(
                        |_, window, cx| window.dispatch_action(Box::new(EditKeymapFile), cx),
                    ),
                )
            })
            .into_any_element()
    }

    /// The open dropdown's options as a box under its control (over it
    /// when the page has no room below and more above), the chosen one
    /// filled.
    #[allow(clippy::too_many_arguments)]
    fn render_tui_options(
        &self,
        item: &'static Item,
        options: &'static [(&'static str, &'static str)],
        value: &Value,
        cols: usize,
        room: (usize, usize),
        cell: Cell,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let key = item.key;
        let entity = cx.entity();
        let close = entity.clone();
        let rows = options.iter().enumerate().map(|(ix, (json, label))| {
            let chosen = serde_json::from_str::<Value>(json)
                .map(|v| values_equal(&v, value))
                .unwrap_or(false);
            let entity = entity.clone();
            let json = *json;
            div()
                .id(ElementId::NamedInteger("settings-option".into(), ix as u64))
                .h(px(cell.h))
                .flex_shrink_0()
                .pl(cell.cols(1))
                .cursor_pointer()
                .whitespace_nowrap()
                .text_color(theme.popup_text)
                .when(chosen, |d| {
                    d.bg(theme.list_bg_focused)
                        .text_color(theme.list_text_focused)
                })
                .when(!chosen, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |this, cx| {
                        if let Ok(v) = serde_json::from_str::<Value>(json) {
                            this.set(key, v, cx);
                        }
                        this.dropdown = None;
                        cx.notify();
                    })
                })
                .child(SharedString::from(fit(label, cols.saturating_sub(4))))
        });
        let body = div().flex().flex_col().children(rows).into_any_element();
        let height = options.len() + 2;
        let (below, above) = room;
        let top = if height > below && above > below {
            -cell.rows(height)
        } else {
            px(cell.h)
        };
        deferred(
            div()
                .id("settings-dropdown")
                .occlude()
                .absolute()
                .top(top)
                .left_0()
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |this, cx| {
                        this.dropdown = None;
                        cx.notify();
                    })
                })
                .child(skin::boxed(
                    cell,
                    cols,
                    height,
                    Vec::new(),
                    Vec::new(),
                    body,
                    theme.popup_bg,
                    theme.popup_border,
                    theme,
                    *m,
                )),
        )
    }
}
