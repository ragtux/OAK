//! An empty tool, as Fleet 1.48 draws one (twenty-fourth pass: the
//! Breakpoints tool without breakpoints, `rest8-fleet-bp-gutter-alt.png`,
//! and the Debug tool without a session, `rest13-fleet-after.png`): a 24 px
//! icon in `icon.primary`, the title (14.67 px semibold, `text.primary`),
//! the description wrapped in `text.tertiary` (measured (94,94,96), the
//! shot's rendering of `LightTint_35` over the island; `text.secondary`
//! would read about 135) and, when there is one, a secondary button in the
//! Debug tool's "Add Watch" form with its chord in
//! `button.secondary.text.hint`. The rows are left-aligned in a block
//! `metrics::EMPTY_TOOL_WIDTH` wide, the block centred in the tool's area
//! (the island under its tab row). Sizes are `metrics::EMPTY_TOOL_*` and
//! the empty panel's `EMPTY_PANEL_*` (`theme.rs`).
//!
//! The tui skin draws the same words as rows on the grid: the icon's glyph,
//! a blank row, the title in `text_primary`, the description in
//! `text_tertiary` wrapped on words at [`TUI_WIDTH`] cells (Fleet's lines),
//! then a blank row and `[ Run & Debug ⌘R ]` (the chord in
//! `button_secondary_text_hint`); the block left-aligned, [`TUI_WIDTH`]
//! cells wide (less on a narrow grid), centred on the tool's grid.

use crate::assets::icon;
use crate::skin::Cell;
use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, relative, AnyElement, App, ElementId, FontWeight, SharedString, Window,
};
use std::rc::Rc;

/// What a click on the button does.
pub type OnClick = Rc<dyn Fn(&mut Window, &mut App)>;

/// The button under the description.
pub struct EmptyButton {
    pub id: &'static str,
    pub label: &'static str,
    /// The chord drawn after the label (the platform's spelling).
    pub chord: Option<String>,
    pub on_click: OnClick,
}

/// One empty tool's content.
pub struct EmptyState {
    /// The Fleet skin's icon (an `assets::icon` path).
    pub icon: &'static str,
    /// The tui skin's glyph for it.
    pub glyph: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub button: Option<EmptyButton>,
}

/// The tui block's width in cells: Fleet's description lines ("Click on
/// the line numbers in", "configuration to debug your") are 28 and 27
/// chars, so the words break where Fleet's do.
pub const TUI_WIDTH: usize = 28;

/// `text` broken on spaces into lines of at most `width` chars (a longer
/// word stands alone on its line).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
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

/// The tui block on a grid of `cols` x `rows`: its width in cells and its
/// top-left cell, the block of `height` rows centred both ways.
pub fn tui_place(cols: usize, rows: usize, height: usize) -> (usize, usize, usize) {
    let width = TUI_WIDTH.min(cols.saturating_sub(2)).max(1);
    let left = cols.saturating_sub(width) / 2;
    let top = rows.saturating_sub(height) / 2;
    (width, left, top)
}

/// The button's text in the tui skin: `[ Run & Debug ⌘R ]`, as label and
/// chord.
fn tui_button_cols(label: &str, chord: Option<&str>) -> usize {
    Cell::text_cols(label) + chord.map_or(0, |c| Cell::text_cols(c) + 1) + 4
}

impl EmptyState {
    /// The Fleet skin: fills its parent and centres the block in it.
    pub fn fleet(self, theme: &Theme, m: &Metrics) -> AnyElement {
        let button = self.button.map(|b| {
            let on_click = b.on_click.clone();
            // The Debug tool's "Add Watch" form: a 24 px `button.secondary`,
            // 8 in each side; the chord one Inter space after the label.
            div()
                .id(ElementId::Name(b.id.into()))
                .mt(px(m.empty_tool_gap_text))
                .h(px(m.button_height))
                .px(px(m.debug_add_watch_pad_x))
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap(px(m.run_popup_button_chord_gap))
                .whitespace_nowrap()
                .rounded(px(m.button_radius))
                .bg(theme.button_secondary_bg)
                .text_color(theme.button_secondary_text)
                .cursor_pointer()
                .hover(|s| s.bg(theme.button_secondary_bg_hovered))
                .active(|s| s.bg(theme.button_secondary_bg_pressed))
                .on_click(move |_, window, cx| on_click(window, cx))
                .child(b.label)
                .when_some(b.chord, |d, chord| {
                    d.child(
                        div()
                            .text_color(theme.button_secondary_text_hint)
                            .child(SharedString::from(chord)),
                    )
                })
        });
        div()
            .size_full()
            .pt(px(m.empty_tool_drop))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(m.empty_tool_width))
                    .max_w(relative(1.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .text_size(px(m.ui_font_size))
                    .child(icon(self.icon, m.empty_panel_icon, theme.icon_primary))
                    .child(
                        div()
                            .mt(px(m.empty_panel_gap_icon))
                            .text_size(px(m.empty_tool_title_size))
                            .line_height(px(m.empty_panel_title_line))
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .text_color(theme.text_primary)
                            .child(self.title),
                    )
                    .child(
                        div()
                            .w_full()
                            .mt(px(m.empty_tool_gap_title))
                            .line_height(px(m.empty_panel_text_line))
                            .text_color(theme.text_tertiary)
                            .child(self.description),
                    )
                    .children(button),
            )
            .into_any_element()
    }

    /// The tui skin on a grid of `cols` x `rows` cells (the caller sets the
    /// grid's font and measures it).
    pub fn tui(self, cell: Cell, cols: usize, rows: usize, theme: &Theme) -> AnyElement {
        let lines = wrap(
            self.description,
            TUI_WIDTH.min(cols.saturating_sub(2)).max(1),
        );
        let height = 3 + lines.len() + if self.button.is_some() { 2 } else { 0 };
        let (width, left, top) = tui_place(cols, rows, height);
        let row = |text: SharedString, color| {
            div()
                .h(px(cell.h))
                .w(cell.cols(width))
                .flex_shrink_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(color)
                .child(text)
        };
        let button = self.button.map(|b| {
            let on_click = b.on_click.clone();
            let n = tui_button_cols(b.label, b.chord.as_deref());
            div().mt(cell.rows(1)).flex().child(
                div()
                    .id(ElementId::Name(b.id.into()))
                    .flex()
                    .flex_shrink_0()
                    .w(cell.cols(n))
                    .h(px(cell.h))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .text_color(theme.button_secondary_text)
                    .hover(|s| s.bg(theme.button_secondary_bg_hovered))
                    .on_click(move |_, window, cx| on_click(window, cx))
                    .child(SharedString::from(format!("[ {}", b.label)))
                    .when_some(b.chord, |d, chord| {
                        d.child(
                            div()
                                .text_color(theme.button_secondary_text_hint)
                                .child(SharedString::from(format!(" {chord}"))),
                        )
                    })
                    .child(" ]"),
            )
        });
        div()
            .size_full()
            .pl(cell.cols(left))
            .pt(cell.rows(top))
            .flex()
            .flex_col()
            .child(row(self.glyph.into(), theme.icon_primary))
            .child(div().h(px(cell.h)).flex_shrink_0())
            .child(row(self.title.into(), theme.text_primary))
            .children(
                lines
                    .into_iter()
                    .map(|l| row(l.into(), theme.text_tertiary)),
            )
            .children(button)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_break_where_fleets_do() {
        assert_eq!(
            wrap(
                "Click on the line numbers in Editor to toggle breakpoints",
                TUI_WIDTH
            ),
            [
                "Click on the line numbers in",
                "Editor to toggle breakpoints"
            ]
        );
        assert_eq!(
            wrap(
                "Create new or run existing configuration to debug your code",
                TUI_WIDTH
            ),
            [
                "Create new or run existing",
                "configuration to debug your",
                "code"
            ]
        );
        // A word longer than the width stands alone.
        assert_eq!(wrap("a verylongword b", 4), ["a", "verylongword", "b"]);
        assert!(wrap("", 10).is_empty());
    }

    #[test]
    fn the_tui_block_is_centred() {
        // 40 x 30 cells, a 6-row block: 28 wide, 6 in, 12 down.
        assert_eq!(tui_place(40, 30, 6), (28, 6, 12));
        // A narrow grid: the block takes all but a cell on each side.
        assert_eq!(tui_place(20, 4, 6), (18, 1, 0));
        assert_eq!(tui_place(0, 0, 3), (1, 0, 0));
        assert_eq!(tui_button_cols("Run & Debug", Some("⌘R")), 18);
        assert_eq!(
            Cell::text_cols("[ Run & Debug ⌘R ]"),
            tui_button_cols("Run & Debug", Some("⌘R"))
        );
    }
}
