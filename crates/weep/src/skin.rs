//! WEEP's skins: `fleet`, the default and the reference, and `tui`, a spike of
//! a skin that looks like a terminal program but is drawn as a GUI. In `tui`
//! everything sits on a grid of character cells (the mono font's advance by a
//! fixed line pitch). Box-drawing is painted as geometry on the cell centres,
//! not set as glyphs, so lines join across rows at any line height.
//!
//! Fleet has no such setting, so it is WEEP's own key, `weep.skin` (live
//! reloaded, and a row in Settings > General); `--skin` or `WEEP_SKIN`
//! override it for one run.
//! So far the panels and editor splits (frames with their tabs set in the
//! top edge), the Files tool and the status bar follow it.

use crate::theme::{Metrics, Theme};
use gpui::{
    canvas, point, prelude::*, px, quad, size, App, BorderStyle, Bounds, Global, Hsla, Pixels,
    TextRun, Window,
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
    /// The mono font's advance at the UI size (zoomed); the pitch is rounded
    /// to whole pixels so rows never drift.
    pub fn measure(window: &Window, theme: &Theme, m: &Metrics) -> Cell {
        let font_size = m.ui_font_size;
        let w = window
            .text_system()
            .shape_line(
                "W".into(),
                px(font_size),
                &[TextRun {
                    len: 1,
                    font: gpui::font(theme.mono_font.clone()),
                    color: theme.text_primary,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width;
        Cell {
            w: f32::from(w).max(1.),
            h: (font_size * LINE).round(),
            font_size,
        }
    }

    /// Whole cells in `extent` logical pixels, `w` or `h`.
    pub fn fit(extent: Pixels, pitch: f32) -> usize {
        (f32::from(extent) / pitch).floor().max(0.) as usize
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

/// A frame round `cols` x `rows` cells: a rounded rectangle through the
/// centres of the outer cells, as `╭─╮ │ ╰─╯` would sit, but drawn.
pub fn frame(
    cell: Cell,
    cols: usize,
    rows: usize,
    color: Hsla,
    m: Metrics,
) -> impl gpui::IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            if cols < 2 || rows < 2 {
                return;
            }
            let t = stroke(window, &m);
            let x = snap(window, f32::from(bounds.origin.x) + cell.w / 2.) - t / 2.;
            let y = snap(window, f32::from(bounds.origin.y) + cell.h / 2.) - t / 2.;
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
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
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
    gpui::div()
        .size_full()
        .relative()
        .font_family(theme.mono_font.clone())
        .text_size(px(cell.font_size))
        .line_height(px(cell.h))
        .child(measure)
        .child(frame(cell, cols, rows, color, m))
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
