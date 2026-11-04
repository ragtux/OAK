//! A list in the Goto popup's frame (M7): the usages of a symbol
//! (`find-usages`) and the targets of a declaration with several, each a
//! row with an icon, a text and a right-aligned `file:line`; Up and Down
//! move, Enter opens, Escape closes, a click opens. The frame and the rows
//! are the Goto panel's measurements (batch 6); that Fleet lists usages in
//! this frame is an estimate (batch 12 has not run).

use crate::actions::{PopupCancel, PopupConfirm, PopupDown, PopupPageDown, PopupPageUp, PopupUp};
use crate::assets::icon;
use crate::skin::{self, Cell, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    deferred, div, point, prelude::*, px, AnyElement, App, BoxShadow, Context, Div, ElementId,
    EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, Hsla, Render, ScrollHandle,
    SharedString, Stateful, StyledText, Window,
};
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct ListRow {
    pub icon: Option<&'static str>,
    pub text: SharedString,
    /// After the text, in the secondary colour (a usage's line).
    pub detail: Option<SharedString>,
    /// Right-aligned, in the secondary colour (`file:line`).
    pub right: Option<SharedString>,
}

#[derive(Clone, Debug)]
pub enum ListPopupEvent {
    Open(usize),
    Cancel,
}

pub struct ListPopup {
    title: SharedString,
    rows: Vec<ListRow>,
    selected: usize,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    tui_scroll: RowScroll,
}

impl EventEmitter<ListPopupEvent> for ListPopup {}

impl Focusable for ListPopup {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ListPopup {
    pub fn new(title: impl Into<SharedString>, rows: Vec<ListRow>, cx: &mut Context<Self>) -> Self {
        ListPopup {
            title: title.into(),
            rows,
            selected: 0,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            tui_scroll: RowScroll::default(),
        }
    }

    fn step(&mut self, delta: isize, wrap: bool, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        let next = if wrap {
            (self.selected as isize + delta).rem_euclid(len)
        } else {
            (self.selected as isize + delta).clamp(0, len - 1)
        };
        self.selected = next as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, true, cx);
    }

    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, true, cx);
    }

    fn page_up(&mut self, _: &PopupPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-10, false, cx);
    }

    fn page_down(&mut self, _: &PopupPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(10, false, cx);
    }

    fn confirm(&mut self, _: &PopupConfirm, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected < self.rows.len() {
            cx.emit(ListPopupEvent::Open(self.selected));
        }
    }

    fn cancel(&mut self, _: &PopupCancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ListPopupEvent::Cancel);
    }
}

impl Render for ListPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(window, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let selected = self.selected;
        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                let is_selected = ix == selected;
                div()
                    .id(ElementId::NamedInteger("list-row".into(), ix as u64))
                    .h(px(m.goto_row_pitch))
                    .flex()
                    .items_center()
                    .px(px(m.popup_row_pad_x))
                    .rounded(px(m.popup_row_radius))
                    .cursor_pointer()
                    .when(is_selected, |d| d.bg(theme.list_bg_focused))
                    .when(!is_selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = ix;
                        cx.emit(ListPopupEvent::Open(ix));
                    }))
                    .children(row.icon.map(|path| {
                        div()
                            .flex_shrink_0()
                            .mr(px(m.popup_row_icon_gap))
                            .child(icon(path, m.icon_size, theme.icon_secondary))
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(if is_selected {
                                theme.list_text_focused
                            } else {
                                theme.list_text
                            })
                            .child(row.text.clone()),
                    )
                    .children(row.detail.clone().map(|d| {
                        div()
                            .ml(px(m.popup_row_icon_gap))
                            .min_w_0()
                            .truncate()
                            .text_color(theme.list_text_secondary)
                            .child(d)
                    }))
                    .child(div().flex_1().min_w(px(m.popup_row_icon_gap)))
                    .children(row.right.clone().map(|r| {
                        div()
                            .flex_shrink_0()
                            .text_color(theme.list_text_secondary)
                            .child(r)
                    }))
            })
            .collect();
        let count: SharedString = format!("{}", self.rows.len()).into();
        deferred(
            div()
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::page_up))
                .on_action(cx.listener(Self::page_down))
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::cancel))
                .absolute()
                .top(px(m.goto_top))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .occlude()
                        .w(px(m.goto_width))
                        .max_h(px(m.goto_max_height))
                        .flex()
                        .flex_col()
                        .rounded(px(m.popup_radius))
                        .border_1()
                        .border_color(theme.popup_border)
                        .bg(theme.popup_goto_bg)
                        .shadow(vec![BoxShadow {
                            color: theme.shadow_medium,
                            offset: point(px(0.), px(m.popup_shadow_offset_y)),
                            blur_radius: px(m.popup_shadow_blur),
                            spread_radius: px(0.),
                        }])
                        .font_family(theme.ui_font.clone())
                        .text_size(px(m.ui_font_size))
                        .text_color(theme.popup_text)
                        .child(
                            div()
                                .h(px(m.goto_header_height))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .px(px(m.goto_pad))
                                .pt(px(m.goto_tab_top))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(self.title.clone())
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .font_weight(FontWeight::NORMAL)
                                        .text_color(theme.text_secondary)
                                        .child(count),
                                ),
                        )
                        .child(
                            div()
                                .id("list-rows")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .track_scroll(&self.scroll)
                                .px(px(m.goto_pad))
                                .pb(px(m.goto_bottom_pad))
                                .children(rows),
                        ),
                ),
        )
        .into_any_element()
    }
}

// ---- the tui skin ---------------------------------------------------------------

/// A list row in the tui skin (`docs/tui-skin.md`, Goto): ` ◆ name  dim/path`
/// with the matched chars coloured and a right column, in whole cells.
pub struct TuiRow {
    pub glyph: Option<(&'static str, Hsla)>,
    /// Cells before the glyph (an outline's depth).
    pub indent: usize,
    pub title: SharedString,
    /// Byte ranges of `title` in `completion.match.text`.
    pub highlights: Vec<Range<usize>>,
    pub detail: Option<SharedString>,
    pub right: Option<SharedString>,
    pub italic: bool,
}

impl TuiRow {
    pub fn text(title: impl Into<SharedString>) -> TuiRow {
        TuiRow {
            glyph: None,
            indent: 0,
            title: title.into(),
            highlights: Vec::new(),
            detail: None,
            right: None,
            italic: false,
        }
    }
}

/// The one-cell mark for an icon path (rule 3: no SVG in the tui skin):
/// `ƒ` callables, `◇` types, `▪` fields and constants, a file's glyph by
/// its type, `·` the rest.
pub fn icon_glyph(path: &str, theme: &Theme) -> (&'static str, Hsla) {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name {
        "function" | "method" => ("ƒ", theme.icon_red),
        "class" | "enum" | "interface" | "module" | "package" => ("◇", theme.icon_blue),
        "field" | "property" | "constant" | "variable" | "parameter" => ("▪", theme.icon_green),
        "file-rust" => skin::file_glyph("a.rs", theme),
        "file-toml" => skin::file_glyph("a.toml", theme),
        "file-json" => skin::file_glyph("a.json", theme),
        "file-markdown" => skin::file_glyph("a.md", theme),
        _ => ("·", theme.text_tertiary),
    }
}

/// One row of `cols` cells: a cell of margin, the indent, the glyph and a
/// space, the title (cut with `…`), two cells, the detail, then the right
/// column right-aligned with a cell of margin. Selected rows are filled.
pub fn tui_row(
    id: impl Into<ElementId>,
    row: TuiRow,
    cols: usize,
    selected: bool,
    cell: Cell,
    theme: &Theme,
) -> Stateful<Div> {
    let right = row.right.map(|r| skin::fit_text(&r, cols / 2));
    let right_cols = right.as_ref().map_or(0, |r| Cell::text_cols(r) + 2);
    let lead = 1 + row.indent + if row.glyph.is_some() { 2 } else { 0 };
    let avail = cols.saturating_sub(lead + 1 + right_cols);
    let full = row.title.to_string();
    let title = skin::fit_text(&full, avail);
    let kept = if title.len() < full.len() {
        title.len() - '…'.len_utf8()
    } else {
        title.len()
    };
    let highlights: Vec<(Range<usize>, HighlightStyle)> = row
        .highlights
        .into_iter()
        .filter(|r| r.end <= kept && r.start < r.end)
        .map(|r| {
            (
                r,
                HighlightStyle {
                    color: Some(theme.completion_match_text),
                    ..Default::default()
                },
            )
        })
        .collect();
    let title_cols = Cell::text_cols(&title);
    let detail = row.detail.and_then(|d| {
        let room = avail.saturating_sub(title_cols + 2);
        (room > 1).then(|| skin::fit_text(&d, room))
    });
    let text = if selected {
        theme.list_text_focused
    } else {
        theme.list_text
    };
    div()
        .id(id.into())
        .flex_shrink_0()
        .w(cell.cols(cols))
        .h(px(cell.h))
        .flex()
        .whitespace_nowrap()
        .overflow_hidden()
        .cursor_pointer()
        .text_color(text)
        .when(selected, |d| d.bg(theme.list_bg_focused))
        .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
        .child(div().w(cell.cols(1 + row.indent)).flex_shrink_0())
        .children(
            row.glyph
                .map(|(g, c)| div().w(cell.cols(2)).flex_shrink_0().text_color(c).child(g)),
        )
        .child(
            div()
                .flex_shrink_0()
                .when(row.italic, |d| d.italic())
                .child(StyledText::new(SharedString::from(title)).with_highlights(highlights)),
        )
        .children(detail.map(|d| {
            div()
                .flex_shrink_0()
                .pl(cell.cols(2))
                .text_color(theme.list_text_secondary)
                .child(SharedString::from(d))
        }))
        .child(div().flex_1())
        .children(right.map(|r| {
            div()
                .flex_shrink_0()
                .pr(cell.cols(1))
                .text_color(theme.text_tertiary)
                .child(SharedString::from(r))
        }))
}

/// A scroll bar laid over a box's column `col`, rows `from..to` (on a frame
/// line or a vertical rule, so the thumb sits on the line).
#[allow(clippy::too_many_arguments)]
pub fn tui_scrollbar(
    cell: Cell,
    col: usize,
    from: usize,
    to: usize,
    top: usize,
    visible: usize,
    total: usize,
    theme: &Theme,
    m: Metrics,
) -> impl IntoElement {
    let thumb = theme.scrollbar_thumb;
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
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
        },
    )
    .absolute()
    .left(cell.cols(col))
    .top(cell.rows(from))
    .w(px(cell.w))
    .h(cell.rows(to.saturating_sub(from)))
}

/// Where a popup under the toolbar starts in the tui skin: the first whole
/// row at or below Fleet's offset.
pub fn tui_top(cell: Cell, m: &Metrics) -> usize {
    (m.goto_top / cell.h).ceil() as usize
}

/// The rows a popup under the toolbar may take in all, frame included.
pub fn tui_max_rows(cell: Cell, m: &Metrics, window: &Window) -> usize {
    let all = Cell::fit(window.viewport_size().height, cell.h);
    let cap = (m.goto_max_height / cell.h).round() as usize;
    cap.min(all.saturating_sub(tui_top(cell, m) + 1)).max(4)
}

/// A popup's width in columns: Fleet's `px` width in whole cells, inside
/// the window with a cell to spare on each side.
pub fn tui_cols(width: f32, cell: Cell, window: &Window) -> usize {
    let all = Cell::fit(window.viewport_size().width, cell.w);
    ((width / cell.w).round() as usize)
        .min(all.saturating_sub(2))
        .max(10)
}

impl ListPopup {
    fn render_tui(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let cols = tui_cols(m.goto_width, cell, window);
        let total = self.rows.len();
        let visible = total
            .min(tui_max_rows(cell, &m, window).saturating_sub(2))
            .max(1);
        self.tui_scroll.follow(self.selected, total, visible);
        let top = self.tui_scroll.top;
        let selected = self.selected;
        let rows: Vec<AnyElement> = self
            .rows
            .iter()
            .enumerate()
            .skip(top)
            .take(visible)
            .map(|(ix, row)| {
                let mut r = TuiRow::text(row.text.clone());
                r.glyph = row.icon.map(|p| icon_glyph(p, &theme));
                r.detail = row.detail.clone();
                r.right = row.right.clone();
                tui_row(
                    ElementId::NamedInteger("list-row".into(), ix as u64),
                    r,
                    cols - 2,
                    ix == selected,
                    cell,
                    &theme,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = ix;
                    cx.emit(ListPopupEvent::Open(ix));
                }))
                .into_any_element()
            })
            .collect();
        let box_rows = visible + 2;
        let bg = theme.popup_goto_bg;
        let body = div()
            .id("list-rows")
            .size_full()
            .flex()
            .flex_col()
            .on_scroll_wheel(cx.listener(move |this, e, _, cx| {
                if this.tui_scroll.wheel(e, cell, total, visible) {
                    cx.notify();
                }
            }))
            .children(rows)
            .into_any_element();
        let count = SharedString::from(format!(" {total} "));
        deferred(
            div()
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::page_up))
                .on_action(cx.listener(Self::page_down))
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::cancel))
                .absolute()
                .top(cell.rows(tui_top(cell, &m)))
                .left(skin::centred_x(window.viewport_size().width, cols, cell))
                .child(
                    skin::boxed(
                        cell,
                        cols,
                        box_rows,
                        vec![skin::title(self.title.clone(), theme.text_primary)],
                        Vec::new(),
                        body,
                        bg,
                        theme.border_focused,
                        &theme,
                        m,
                    )
                    .occlude()
                    .child(skin::edge_strip_end(
                        cell,
                        0,
                        vec![div()
                            .text_color(theme.text_tertiary)
                            .child(count)
                            .into_any_element()],
                        bg,
                    ))
                    .child(skin::edge_strip(
                        cell,
                        box_rows - 1,
                        2,
                        cols.saturating_sub(4),
                        skin::hints(&[("↵", "open"), ("esc", "close")], &theme),
                        bg,
                    ))
                    .child(tui_scrollbar(
                        cell,
                        cols - 1,
                        1,
                        box_rows - 1,
                        top,
                        visible,
                        total,
                        &theme,
                        m,
                    )),
                ),
        )
        .into_any_element()
    }
}

/// An input row of the tui skin: a cell of margin, a prompt cell (`›`,
/// `/`, `↳`, in the list rows' glyph column; `inputField.focusBorder`
/// while the field has the keyboard) and a space, then the field on `input_bg` to the end of the row with `trailing` (toggles, a
/// counter) after it.
pub fn tui_input_row(
    prompt: &'static str,
    focused: bool,
    input: impl IntoElement,
    trailing: Vec<AnyElement>,
    theme: &Theme,
    cell: Cell,
) -> Div {
    div()
        .h(px(cell.h))
        .w_full()
        .flex_shrink_0()
        .flex()
        .child(
            div()
                .w(cell.cols(3))
                .flex_shrink_0()
                .pl(cell.cols(1))
                .text_color(if focused {
                    theme.input_focus_border
                } else {
                    theme.text_tertiary
                })
                .child(prompt),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .bg(theme.input_bg)
                .child(div().flex_1().min_w_0().h_full().child(input))
                .children(trailing),
        )
}

/// An option toggle of the tui skin: `[Aa]`, filled when on.
pub fn tui_toggle(
    id: impl Into<ElementId>,
    label: &str,
    on: bool,
    theme: &Theme,
    cell: Cell,
) -> Stateful<Div> {
    let text = format!("[{label}]");
    div()
        .id(id.into())
        .flex_shrink_0()
        .w(cell.cols(Cell::text_cols(&text)))
        .h(px(cell.h))
        .whitespace_nowrap()
        .cursor_pointer()
        .map(|d| {
            if on {
                d.bg(theme.toggle_on_bg)
                    .text_color(theme.toggle_on_text)
                    .hover(|s| s.bg(theme.toggle_on_bg_hovered))
            } else {
                d.text_color(theme.toggle_off_text)
                    .hover(|s| s.bg(theme.toggle_off_bg_hovered))
            }
        })
        .child(SharedString::from(text))
}

/// A cell of space in a row.
pub fn tui_gap(cell: Cell, n: usize) -> AnyElement {
    div().w(cell.cols(n)).flex_shrink_0().into_any_element()
}

/// A section heading inside a list body: `─ title ──────`, a rule with the
/// title set in a cell from its start.
pub fn tui_heading(
    title: SharedString,
    cols: usize,
    bg: Hsla,
    theme: &Theme,
    cell: Cell,
    m: Metrics,
) -> AnyElement {
    div()
        .relative()
        .flex_shrink_0()
        .w(cell.cols(cols))
        .h(px(cell.h))
        .child(skin::rule(cell, cols, theme.border, m))
        .child(
            div()
                .absolute()
                .top_0()
                .left(cell.cols(1))
                .bg(bg)
                .text_color(theme.text_tertiary)
                .whitespace_nowrap()
                .child(SharedString::from(format!(" {title} "))),
        )
        .into_any_element()
}
