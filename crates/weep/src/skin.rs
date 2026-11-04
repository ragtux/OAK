//! WEEP's skins: `fleet`, the default and the reference, and `tui`, a spike of
//! a skin that looks like a terminal program but is drawn as a GUI. In `tui`
//! everything sits on a grid of character cells (the mono font's advance by a
//! fixed line pitch). Box-drawing is painted as geometry on the cell centres,
//! not set as glyphs, so lines join across rows at any line height.
//!
//! Fleet has no such setting, so it is WEEP's own key, `weep.skin` (live
//! reloaded, and a row in Settings > General); `--skin` or `WEEP_SKIN`
//! override it for one run.
//! The design for every element is `docs/tui-skin.md`; the shared pieces
//! (the cell, frames, boxes, rules, scroll bars, buttons) are here.

use crate::settings::AppSettings;
use crate::theme::{Metrics, Theme};
use gpui::{
    canvas, div, point, prelude::*, px, quad, size, AnyElement, App, BorderStyle, Bounds, Div,
    ElementId, Global, Hsla, Pixels, Stateful, Window,
};
use std::cell::Cell as Shared;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Skin {
    #[default]
    Fleet,
    Tui,
}

impl Global for Skin {}

impl Skin {
    pub fn parse(s: &str) -> Option<Skin> {
        match s {
            "fleet" => Some(Skin::Fleet),
            "tui" => Some(Skin::Tui),
            _ => None,
        }
    }

    pub fn current(cx: &App) -> Skin {
        cx.try_global::<Skin>().copied().unwrap_or_default()
    }
}

/// Line pitch over font size: roomier than a terminal's 1.2, which the grid
/// can afford because no box-drawing glyph has to span the gap.
pub const LINE: f32 = 1.5;

/// One character cell, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub w: f32,
    pub h: f32,
    pub font_size: f32,
}

impl Cell {
    /// The grid's cell for the current settings: the mono font's advance at
    /// `editor.fontSize` times the zoom, by `editor.lineSpacing` (else
    /// `LINE`) of it, rounded to whole pixels so rows never drift. One size
    /// for the whole window, as a terminal has: the editor's text sits on
    /// the same grid as the frames, the tree and the status bar.
    pub fn current(cx: &App) -> Cell {
        let m = cx.global::<Metrics>();
        let theme = cx.global::<Theme>();
        let settings = &cx.global::<AppSettings>().settings;
        let font_size = settings.editor_font_size() * m.zoom;
        let spacing = settings.editor_line_spacing().unwrap_or(LINE);
        let text = cx.text_system();
        let id = text.resolve_font(&gpui::font(theme.mono_font.clone()));
        let w = text
            .advance(id, px(font_size), 'W')
            .map(|s| f32::from(s.width))
            .unwrap_or(font_size * 0.6);
        Cell {
            w: w.max(1.),
            h: (font_size * spacing).round().max(1.),
            font_size,
        }
    }

    /// `n` columns, in pixels.
    pub fn cols(&self, n: usize) -> Pixels {
        px(n as f32 * self.w)
    }

    /// `n` rows, in pixels.
    pub fn rows(&self, n: usize) -> Pixels {
        px(n as f32 * self.h)
    }

    /// The columns `text` takes: one per char (no wide glyphs on the grid).
    pub fn text_cols(text: &str) -> usize {
        text.chars().count()
    }

    /// Whole cells in `extent` logical pixels, `w` or `h`. Half a pixel of
    /// slack: an area sized in exact cells may come back a hair short once
    /// layout snaps it to device pixels.
    pub fn fit(extent: Pixels, pitch: f32) -> usize {
        ((f32::from(extent) + 0.5) / pitch).floor().max(0.) as usize
    }
}

/// Asks for another frame once this one is drawn: the grid is read back
/// while painting, and gpui ignores a `refresh` made during a draw (a skin
/// switched by the settings file came up empty until something else redrew).
pub fn redraw(window: &Window) {
    window.on_next_frame(|window, _| window.refresh());
}

/// A stroke for the grid's lines: one device pixel per zoom step, at least one.
pub fn stroke(window: &Window, m: &Metrics) -> f32 {
    let scale = window.scale_factor();
    (m.zoom * scale).round().max(1.) / scale
}

/// `v` on the device-pixel grid, so a 1 px line is never smeared over two.
pub fn snap(window: &Window, v: f32) -> f32 {
    let scale = window.scale_factor();
    (v * scale).round() / scale
}

/// Paints a frame round `cols` x `rows` cells from `origin`: a rounded
/// rectangle through the centres of the outer cells, as `╭─╮ │ ╰─╯` would
/// sit, but drawn; and a `├──┤` rule across the centre of each row in
/// `rules` (counted from the top), joining both sides.
#[allow(clippy::too_many_arguments)]
pub fn paint_frame(
    window: &mut Window,
    origin: gpui::Point<Pixels>,
    cell: Cell,
    cols: usize,
    rows: usize,
    rules: &[usize],
    color: Hsla,
    m: &Metrics,
) {
    if cols < 2 || rows < 2 {
        return;
    }
    let t = stroke(window, m);
    let x = snap(window, f32::from(origin.x) + cell.w / 2.) - t / 2.;
    let y = snap(window, f32::from(origin.y) + cell.h / 2.) - t / 2.;
    let w = snap(window, (cols - 1) as f32 * cell.w) + t;
    let h = snap(window, (rows - 1) as f32 * cell.h) + t;
    window.paint_quad(quad(
        Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
        px(cell.w * 0.75),
        gpui::transparent_black(),
        px(t),
        color,
        BorderStyle::Solid,
    ));
    for &row in rules {
        if row == 0 || row + 1 >= rows {
            continue;
        }
        let ry = snap(
            window,
            f32::from(origin.y) + row as f32 * cell.h + cell.h / 2.,
        ) - t / 2.;
        window.paint_quad(gpui::fill(
            Bounds::new(point(px(x), px(ry)), size(px(w), px(t))),
            color,
        ));
    }
}

/// A box of exactly `cols` x `rows` cells on `bg` (whole cells, the frame's
/// own included, as a terminal fills them): the frame in `color`, a rule
/// across each row of `rules`, `strip` set into the top edge from the third
/// column (a title, tabs) and `body` inside, one cell in from every side.
/// The tui skin's popups, menus, dialogs and tooltips are boxes; the caller
/// positions the returned div and adds its handlers.
#[allow(clippy::too_many_arguments)]
pub fn boxed(
    cell: Cell,
    cols: usize,
    rows: usize,
    strip: Vec<AnyElement>,
    rules: Vec<usize>,
    body: AnyElement,
    bg: Hsla,
    color: Hsla,
    theme: &Theme,
    m: Metrics,
) -> Div {
    let paint = canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            paint_frame(window, bounds.origin, cell, cols, rows, &rules, color, &m);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full();
    div()
        .relative()
        .flex_shrink_0()
        .w(cell.cols(cols))
        .h(cell.rows(rows))
        .bg(bg)
        .font_family(theme.mono_font.clone())
        .text_size(px(cell.font_size))
        .line_height(px(cell.h))
        .child(
            div()
                .absolute()
                .left(px(cell.w))
                .top(px(cell.h))
                .w(cell.cols(cols.saturating_sub(2)))
                .h(cell.rows(rows.saturating_sub(2)))
                .overflow_hidden()
                .child(body),
        )
        .child(paint)
        .when(cols > 4 && !strip.is_empty(), |d| {
            d.child(
                div()
                    .absolute()
                    .top_0()
                    .left(cell.cols(2))
                    .max_w(cell.cols(cols - 4))
                    .h(px(cell.h))
                    .overflow_hidden()
                    .flex()
                    .bg(bg)
                    .children(strip),
            )
        })
}

/// A title set into a frame's top edge: ` text ` on the box's background,
/// so the line stops a cell short of it on either side.
pub fn title(text: impl Into<gpui::SharedString>, color: Hsla) -> AnyElement {
    let text: gpui::SharedString = text.into();
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .text_color(color)
        .child(gpui::SharedString::from(format!(" {text} ")))
        .into_any_element()
}

/// A row with a `─` line through its centre, `cols` cells long: a
/// separator inside a body (a rule joining a frame is `boxed`'s `rules`).
pub fn rule(cell: Cell, cols: usize, color: Hsla, m: Metrics) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let t = stroke(window, &m);
            let y = snap(window, f32::from(bounds.origin.y) + cell.h / 2.) - t / 2.;
            let x = snap(window, f32::from(bounds.origin.x));
            let w = snap(window, cols as f32 * cell.w);
            window.paint_quad(gpui::fill(
                Bounds::new(point(px(x), px(y)), size(px(w), px(t))),
                color,
            ));
        },
    )
    .w(cell.cols(cols))
    .h(px(cell.h))
    .flex_shrink_0()
}

/// Paints a scroll bar in a column one cell wide (`bounds`, whole rows): a
/// rail through the column's centre and a thumb half a cell wide, both in
/// whole rows (at least one), placed by `offset` over `total - visible`
/// (any unit: rows, pixels). Nothing when everything is visible.
#[allow(clippy::too_many_arguments)]
pub fn paint_scrollbar(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    cell: Cell,
    offset: f32,
    visible: f32,
    total: f32,
    rail: Option<Hsla>,
    thumb: Hsla,
    m: &Metrics,
) {
    let rows = Cell::fit(bounds.size.height, cell.h);
    if rows == 0 || total <= visible || visible <= 0. {
        return;
    }
    let len = ((rows as f32 * visible / total).round() as usize).clamp(1, rows);
    let room = (rows - len) as f32;
    let start = ((offset / (total - visible)).clamp(0., 1.) * room).round() as usize;
    let x0 = f32::from(bounds.origin.x);
    let y0 = f32::from(bounds.origin.y);
    if let Some(rail) = rail {
        let t = stroke(window, m);
        let x = snap(window, x0 + cell.w / 2.) - t / 2.;
        window.paint_quad(gpui::fill(
            Bounds::new(
                point(px(x), px(snap(window, y0 + cell.h / 2.))),
                size(px(t), px(snap(window, (rows - 1) as f32 * cell.h))),
            ),
            rail,
        ));
    }
    let w = snap(window, cell.w / 2.);
    let inset = snap(window, cell.h * 0.15);
    let x = snap(window, x0 + (cell.w - w) / 2.);
    let y = snap(window, y0 + start as f32 * cell.h) + inset;
    let h = snap(window, len as f32 * cell.h) - 2. * inset;
    window.paint_quad(quad(
        Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
        px(w / 2.),
        thumb,
        px(0.),
        gpui::transparent_black(),
        BorderStyle::Solid,
    ));
}

/// A button in the tui skin: `[ Label ]` in whole cells. A primary button
/// is a filled block (reverse video in the accent colour); the others are
/// bracketed text that a hover fills.
pub fn button(
    id: impl Into<ElementId>,
    label: &str,
    primary: bool,
    theme: &Theme,
    cell: Cell,
) -> Stateful<Div> {
    let text = format!("[ {label} ]");
    let cols = Cell::text_cols(&text);
    let d = div()
        .id(id.into())
        .flex_shrink_0()
        .w(cell.cols(cols))
        .h(px(cell.h))
        .whitespace_nowrap()
        .cursor_pointer()
        .child(gpui::SharedString::from(text));
    if primary {
        d.bg(theme.button_primary_bg)
            .text_color(theme.button_primary_text)
            .hover(|s| s.bg(theme.button_primary_bg_hovered))
    } else {
        d.text_color(theme.button_secondary_text)
            .hover(|s| s.bg(theme.button_secondary_bg_hovered))
    }
}

/// A check box's three cells: `[x]` or `[ ]`.
pub fn checkbox(on: bool) -> &'static str {
    if on {
        "[x]"
    } else {
        "[ ]"
    }
}

/// What an island sets on its frame besides the top strip.
#[derive(Default)]
pub struct Edges {
    /// Set into the top edge at its right end, two cells in (a badge).
    pub top_end: Vec<AnyElement>,
    /// Set into the bottom edge from the third column (key hints, a summary).
    pub bottom: Vec<AnyElement>,
    /// Laid over the right edge between the corners: one cell wide and
    /// `rows - 2` high, so a scroll bar's thumb sits on the frame line.
    pub right: Option<AnyElement>,
    /// Rules across the frame (`├──┤`), counted up from the bottom edge:
    /// 2 is the centre of the second row above it (a one-row command line
    /// under the rule).
    pub rules_from_bottom: Vec<usize>,
}

/// An island in the tui skin: `frame` round its whole cells (`grid`, read
/// back while painting, so a resize settles a frame later), `strip` set into
/// the top edge from the third column (tabs, a title), and `body` inside,
/// one cell in from every side and sized in whole cells.
pub fn framed(
    cell: Cell,
    grid: &Rc<Shared<(usize, usize)>>,
    strip: Vec<gpui::AnyElement>,
    body: gpui::AnyElement,
    theme: &Theme,
    m: Metrics,
    focused: bool,
) -> gpui::AnyElement {
    framed_edges(cell, grid, strip, Edges::default(), body, theme, m, focused)
}

/// `framed`, with `edges` set into the frame as well.
#[allow(clippy::too_many_arguments)]
pub fn framed_edges(
    cell: Cell,
    grid: &Rc<Shared<(usize, usize)>>,
    strip: Vec<gpui::AnyElement>,
    edges: Edges,
    body: gpui::AnyElement,
    theme: &Theme,
    m: Metrics,
    focused: bool,
) -> gpui::AnyElement {
    let (cols, rows) = grid.get();
    let measure = {
        let grid = grid.clone();
        canvas(
            move |bounds, window, _| {
                let fit = (
                    Cell::fit(bounds.size.width, cell.w),
                    Cell::fit(bounds.size.height, cell.h),
                );
                if grid.get() != fit {
                    grid.set(fit);
                    redraw(window);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    };
    let color = if focused {
        theme.border_focused
    } else {
        theme.border
    };
    let rules: Vec<usize> = edges
        .rules_from_bottom
        .iter()
        .filter_map(|k| rows.saturating_sub(1).checked_sub(*k))
        .collect();
    let paint = canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            paint_frame(window, bounds.origin, cell, cols, rows, &rules, color, &m);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full();
    let bg = theme.island_background;
    let Edges {
        top_end,
        bottom,
        right,
        ..
    } = edges;
    gpui::div()
        .size_full()
        .relative()
        .font_family(theme.mono_font.clone())
        .text_size(px(cell.font_size))
        .line_height(px(cell.h))
        .child(measure)
        .child(paint)
        .child(
            gpui::div()
                .absolute()
                .left(px(cell.w))
                .top(px(cell.h))
                .w(px(cols.saturating_sub(2) as f32 * cell.w))
                .h(px(rows.saturating_sub(2) as f32 * cell.h))
                .overflow_hidden()
                .child(body),
        )
        .when(cols > 6, |d| {
            d.child(
                gpui::div()
                    .absolute()
                    .top_0()
                    .left(px(2. * cell.w))
                    .max_w(px((cols - 4) as f32 * cell.w))
                    .overflow_hidden()
                    .flex()
                    .children(strip),
            )
        })
        .when(cols > 6 && !top_end.is_empty(), |d| {
            d.child(
                gpui::div()
                    .absolute()
                    .top_0()
                    .right(px(2. * cell.w))
                    .h(px(cell.h))
                    .flex()
                    .bg(bg)
                    .children(top_end),
            )
        })
        .when(cols > 6 && rows > 1 && !bottom.is_empty(), |d| {
            d.child(
                gpui::div()
                    .absolute()
                    .top(px((rows - 1) as f32 * cell.h))
                    .left(px(2. * cell.w))
                    .max_w(px((cols - 4) as f32 * cell.w))
                    .h(px(cell.h))
                    .overflow_hidden()
                    .flex()
                    .bg(bg)
                    .children(bottom),
            )
        })
        .when_some(right.filter(|_| cols > 1 && rows > 2), |d, right| {
            d.child(
                gpui::div()
                    .absolute()
                    .left(px((cols - 1) as f32 * cell.w))
                    .top(px(cell.h))
                    .w(px(cell.w))
                    .h(px((rows - 2) as f32 * cell.h))
                    .child(right),
            )
        })
        .into_any_element()
}

/// The tree guides of one row, `rails.len() + 1` levels of two cells each:
/// `│ ` where an ancestor has siblings below, then `├─` or `└─` for the row.
pub fn guides(
    cell: Cell,
    rails: Vec<bool>,
    last: bool,
    color: Hsla,
    m: Metrics,
) -> impl gpui::IntoElement {
    let levels = rails.len() + 1;
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let t = stroke(window, &m);
            let top = f32::from(bounds.origin.y);
            let mid = snap(window, top + cell.h / 2.) - t / 2.;
            let x0 = f32::from(bounds.origin.x);
            let scale = window.scale_factor();
            let column = |k: usize| {
                ((x0 + k as f32 * 2. * cell.w + cell.w / 2.) * scale).round() / scale - t / 2.
            };
            let line = |window: &mut Window, x: f32, y: f32, w: f32, h: f32| {
                window.paint_quad(gpui::fill(
                    Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
                    color,
                ));
            };
            for (k, rail) in rails.iter().enumerate() {
                if *rail {
                    line(window, column(k), top, t, cell.h);
                }
            }
            let x = column(levels - 1);
            let down = if last { mid + t - top } else { cell.h };
            line(window, x, top, t, down);
            let end = snap(window, x0 + levels as f32 * 2. * cell.w - cell.w * 0.35);
            line(window, x, mid, end - x, t);
        },
    )
    .w(px(levels as f32 * 2. * cell.w))
    .h(px(cell.h))
    .flex_shrink_0()
}

/// A file's mark in the tui skin: one glyph, coloured by kind, in place of
/// the icon.
pub fn file_glyph(name: &str, theme: &Theme) -> (&'static str, Hsla) {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "rs" => ("◆", theme.icon_yellow),
        "toml" => ("≡", theme.icon_blue),
        "json" | "jsonc" | "json5" => ("{", theme.icon_green),
        "md" | "markdown" => ("¶", theme.text_secondary),
        "lock" => ("■", theme.text_tertiary),
        _ => ("·", theme.text_tertiary),
    }
}

/// An element built once the window and the app are at hand: for a render
/// function that is given only the theme and the metrics (the toolbar, the
/// generic scroll bar) and must ask `Skin::current` or `Cell::current`.
pub fn with_window(
    build: impl FnOnce(&mut Window, &mut App) -> AnyElement + 'static,
) -> WithWindow {
    WithWindow(Box::new(build))
}

type Build = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;

#[derive(gpui::IntoElement)]
pub struct WithWindow(Build);

impl gpui::RenderOnce for WithWindow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        (self.0)(window, cx)
    }
}

/// `text` broken into lines of at most `cols` cells, at spaces where it can
/// (a word longer than a line is cut); the text's own newlines are kept.
pub fn wrap(text: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split(' ') {
            let mut word: Vec<char> = word.chars().collect();
            loop {
                let have = line.chars().count();
                let gap = usize::from(have > 0);
                if have + gap + word.len() <= cols {
                    if gap == 1 {
                        line.push(' ');
                    }
                    line.extend(word.iter());
                    break;
                }
                if have > 0 {
                    out.push(std::mem::take(&mut line));
                    continue;
                }
                let rest = word.split_off(cols);
                out.push(word.iter().collect());
                word = rest;
            }
        }
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// `v` rounded down onto the grid's `pitch` (a cell's `w` or `h`).
pub fn floor_to(v: Pixels, pitch: f32) -> Pixels {
    px((f32::from(v) / pitch).floor() * pitch)
}

// ---- popups: the boxes over the editor (Goto, lists, prompts, Smart Mode) ----

/// Paints a vertical rule through the centre of column `col` of a box at
/// `origin`, from the centre of row `from` to the centre of row `to`: it
/// joins a rule or a frame edge at both ends (`┬ │ ┴`).
#[allow(clippy::too_many_arguments)]
pub fn paint_vrule(
    window: &mut Window,
    origin: gpui::Point<Pixels>,
    cell: Cell,
    col: usize,
    from: usize,
    to: usize,
    color: Hsla,
    m: &Metrics,
) {
    if to <= from {
        return;
    }
    let t = stroke(window, m);
    let x = snap(
        window,
        f32::from(origin.x) + col as f32 * cell.w + cell.w / 2.,
    ) - t / 2.;
    let y = snap(
        window,
        f32::from(origin.y) + from as f32 * cell.h + cell.h / 2.,
    ) - t / 2.;
    let h = snap(window, (to - from) as f32 * cell.h) + t;
    window.paint_quad(gpui::fill(
        Bounds::new(point(px(x), px(y)), size(px(t), px(h))),
        color,
    ));
}

/// A strip set into row `row` of a box from column `col` (a title on a
/// rule, key hints in the bottom edge): absolute, on `bg` so the line stops
/// a cell short of it; at most `max_cols` cells wide. Add it to the box.
pub fn edge_strip(
    cell: Cell,
    row: usize,
    col: usize,
    max_cols: usize,
    children: Vec<AnyElement>,
    bg: Hsla,
) -> Div {
    div()
        .absolute()
        .top(cell.rows(row))
        .left(cell.cols(col))
        .max_w(cell.cols(max_cols))
        .h(px(cell.h))
        .overflow_hidden()
        .flex()
        .bg(bg)
        .children(children)
}

/// A strip set into row `row` of a box, ending two cells before its right
/// edge (a count, a toggle at the top edge's right end).
pub fn edge_strip_end(cell: Cell, row: usize, children: Vec<AnyElement>, bg: Hsla) -> Div {
    div()
        .absolute()
        .top(cell.rows(row))
        .right(cell.cols(2))
        .h(px(cell.h))
        .flex()
        .bg(bg)
        .children(children)
}

/// Key hints for a box's bottom edge: ` ↵ open  esc close `, each key in
/// `text_secondary` and its word in `text_tertiary`.
pub fn hints(pairs: &[(&str, &str)], theme: &Theme) -> Vec<AnyElement> {
    let mut out = Vec::new();
    for (i, (key, word)) in pairs.iter().enumerate() {
        let lead = if i == 0 { " " } else { "  " };
        out.push(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_color(theme.text_secondary)
                .child(gpui::SharedString::from(format!("{lead}{key}")))
                .into_any_element(),
        );
        let tail = if i + 1 == pairs.len() { " " } else { "" };
        out.push(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_color(theme.text_tertiary)
                .child(gpui::SharedString::from(format!(" {word}{tail}")))
                .into_any_element(),
        );
    }
    out
}

/// `text` cut to `cols` cells, ending in `…` when cut.
pub fn fit_text(text: &str, cols: usize) -> String {
    let n = text.chars().count();
    if n <= cols {
        return text.to_string();
    }
    if cols == 0 {
        return String::new();
    }
    let mut s: String = text.chars().take(cols - 1).collect();
    s.push('…');
    s
}

/// A box's left edge on the window's grid of columns: centred.
pub fn centred_x(viewport_width: Pixels, cols: usize, cell: Cell) -> Pixels {
    let all = Cell::fit(viewport_width, cell.w);
    cell.cols(all.saturating_sub(cols) / 2)
}

/// The first visible row of a list drawn in whole rows (rule 6): a wheel
/// moves it by rows, and the selection is revealed when it moves.
#[derive(Clone, Copy, Debug, Default)]
pub struct RowScroll {
    pub top: usize,
    /// Pixels the wheel moved short of a whole row.
    acc: f32,
    /// The row last revealed: a selection that moved is brought into view
    /// once, so the wheel can scroll it out of view afterwards.
    revealed: Option<usize>,
}

impl RowScroll {
    /// Brings `row` into the `visible` rows when it is not the one last
    /// revealed, and keeps `top` inside `total`.
    pub fn follow(&mut self, row: usize, total: usize, visible: usize) {
        if self.revealed != Some(row) {
            self.revealed = Some(row);
            if row < self.top {
                self.top = row;
            } else if visible > 0 && row >= self.top + visible {
                self.top = row + 1 - visible;
            }
        }
        self.top = self.top.min(total.saturating_sub(visible));
    }

    /// Back to the first row (a new list).
    pub fn reset(&mut self) {
        *self = RowScroll::default();
    }

    /// A wheel event over the list: whole rows, true when `top` moved.
    pub fn wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        cell: Cell,
        total: usize,
        visible: usize,
    ) -> bool {
        let dy = f32::from(event.delta.pixel_delta(px(cell.h)).y);
        self.acc -= dy;
        let rows = (self.acc / cell.h).trunc();
        if rows == 0. {
            return false;
        }
        self.acc -= rows * cell.h;
        let max = total.saturating_sub(visible) as isize;
        let top = (self.top as isize + rows as isize).clamp(0, max.max(0)) as usize;
        let moved = top != self.top;
        self.top = top;
        moved
    }
}

// ---- the tools' pieces (Search, Git, History, Changed Files, Run) ----------

/// Measures its parent in whole cells into `grid` while painting (laid out
/// absolutely over it): for a body that lays out by columns. A change asks
/// for another frame, as `framed` does.
pub fn measure(cell: Cell, grid: &Rc<Shared<(usize, usize)>>) -> impl IntoElement {
    let grid = grid.clone();
    canvas(
        move |bounds, window, _| {
            let fit = (
                Cell::fit(bounds.size.width, cell.w),
                Cell::fit(bounds.size.height, cell.h),
            );
            if grid.get() != fit {
                grid.set(fit);
                redraw(window);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Paints a vertical line through the centre of the cell column whose left
/// edge is `x`, from `y0` to `y1` (a `│` joining across rows).
#[allow(clippy::too_many_arguments)]
pub fn paint_vline(
    window: &mut Window,
    x: f32,
    y0: f32,
    y1: f32,
    cell: Cell,
    color: Hsla,
    m: &Metrics,
) {
    let t = stroke(window, m);
    let x = snap(window, x + cell.w / 2.) - t / 2.;
    let y0 = snap(window, y0);
    let y1 = snap(window, y1);
    if y1 > y0 {
        window.paint_quad(gpui::fill(
            Bounds::new(point(px(x), px(y0)), size(px(t), px(y1 - y0))),
            color,
        ));
    }
}

/// One cell column `rows` high with a line through its centre: a `│`
/// separator that joins the rows above and below it.
pub fn vrule(cell: Cell, rows: usize, color: Hsla, m: Metrics) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let y = f32::from(bounds.origin.y);
            paint_vline(
                window,
                f32::from(bounds.origin.x),
                y,
                y + rows as f32 * cell.h,
                cell,
                color,
                &m,
            );
        },
    )
    .w(px(cell.w))
    .h(cell.rows(rows))
    .flex_shrink_0()
}

/// A section heading inside a body: a rule `cols` cells long with `strip`
/// set in from the third column on `bg` (`── Changes 3 ──────`).
pub fn section(
    cell: Cell,
    cols: usize,
    strip: Vec<AnyElement>,
    color: Hsla,
    bg: Hsla,
    m: Metrics,
) -> Div {
    div()
        .relative()
        .flex_shrink_0()
        .w(cell.cols(cols))
        .h(px(cell.h))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(rule(cell, cols, color, m)),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(cell.cols(2))
                .max_w(cell.cols(cols.saturating_sub(4)))
                .h(px(cell.h))
                .overflow_hidden()
                .whitespace_nowrap()
                .flex()
                .bg(bg)
                .children(strip),
        )
}

/// An option toggle in whole cells, `[Aa]`: filled when on (reverse video),
/// a hover fills it when off.
pub fn toggle(
    id: impl Into<ElementId>,
    label: &str,
    on: bool,
    theme: &Theme,
    cell: Cell,
) -> Stateful<Div> {
    let text = format!("[{label}]");
    let d = div()
        .id(id.into())
        .flex_shrink_0()
        .w(cell.cols(Cell::text_cols(&text)))
        .h(px(cell.h))
        .whitespace_nowrap()
        .cursor_pointer()
        .child(gpui::SharedString::from(text));
    if on {
        d.bg(theme.toggle_on_bg)
            .text_color(theme.toggle_on_text)
            .hover(|s| s.bg(theme.toggle_on_bg_hovered))
    } else {
        d.text_color(theme.toggle_off_text)
            .hover(|s| s.bg(theme.toggle_off_bg_hovered))
    }
}
