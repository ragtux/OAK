//! The Breakpoints tool (M8), as Fleet 1.48 shows it (batch 13,
//! `debug-fleet-breakpoints-tool.png`): a title row "Breakpoints" with the
//! count in `text.secondary` and a check box at its right that enables or
//! disables them all, then one row per line breakpoint of the app's
//! `BreakpointStore`: the breakpoint's dot, `debuggee.rs:7` (the file's
//! name and the 1-based line), the folder in `text.secondary` (`misc`,
//! relative to the workspace root) and the breakpoint's own check box at the
//! row's right. Sizes are `metrics::BREAKPOINTS_*` (`theme.rs`).
//!
//! Batch 13 showed one breakpoint in one file, so the order of several
//! (file by file, in line order), a disabled or unbound breakpoint's dot
//! (the gutter's ring and grey) and the empty tool's words are estimates;
//! no other group (exception breakpoints) exists in WEEP. A row's tooltip
//! is the line's text.
//!
//! Keys (context `BreakpointsTool`): arrows move, Space toggles the check
//! box (`debug/toggle-enabled-breakpoint`), Delete or Backspace removes
//! (`debug/remove-breakpoint`), Enter or a double click opens the file at
//! the line through [`DebugViewEvent::Open`].

use crate::actions::{
    DebugRemoveBreakpoint, DebugToggleEnabledBreakpoint, PopupConfirm, PopupDown, PopupUp,
};
use crate::breakpoints::{BreakpointStore, LineBreakpoint, Look};
use crate::debug_tool::{relative_path, section_title, DebugViewEvent};
use crate::git_tool::{checkbox, tui_list, CheckState};
use crate::skin::{self, Cell, FrameBars, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use crate::tooltip::Tooltip;
use gpui::{
    div, prelude::*, px, AnyElement, App, Context, ElementId, EventEmitter, FocusHandle, Focusable,
    Render, ScrollHandle, SharedString, Subscription, Window,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

/// A line's text from an open document, when the file is open (the
/// workspace sets it; lines on disk otherwise): the rows' tooltips.
pub type LineSource = Rc<dyn Fn(&Path, usize, &App) -> Option<String>>;

/// One breakpoint's row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BpRow {
    pub path: PathBuf,
    pub line: usize,
    pub enabled: bool,
    /// `Some(false)`: the running session could not bind it.
    pub verified: Option<bool>,
    /// `debuggee.rs:7`: the file's name and the 1-based line.
    pub name: String,
    /// The file's folder relative to the root (`misc`), empty at the root.
    pub folder: String,
}

impl BpRow {
    fn look(&self) -> Look {
        LineBreakpoint {
            enabled: self.enabled,
            verified: self.verified,
            ..LineBreakpoint::at(self.line)
        }
        .look()
    }
}

/// The rows for `files`: file by file, each file's breakpoints in line order.
pub fn bp_rows<'a>(
    files: impl IntoIterator<Item = (&'a Path, &'a [LineBreakpoint])>,
    root: &Path,
) -> Vec<BpRow> {
    let mut rows = Vec::new();
    for (path, list) in files {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let folder = path
            .parent()
            .map(|p| relative_path(p, root))
            .unwrap_or_default();
        let mut list = list.to_vec();
        list.sort_by_key(|b| b.line);
        for b in list {
            rows.push(BpRow {
                path: path.to_path_buf(),
                line: b.line,
                enabled: b.enabled,
                verified: b.verified,
                name: format!("{file}:{}", b.line + 1),
                folder: folder.clone(),
            });
        }
    }
    rows
}

/// The title row's check box: on when every breakpoint is, the dash when
/// some are.
pub fn all_state(rows: &[BpRow]) -> CheckState {
    let on = rows.iter().filter(|r| r.enabled).count();
    if on == 0 {
        CheckState::Off
    } else if on == rows.len() {
        CheckState::All
    } else {
        CheckState::Some
    }
}

/// `d` with a border `w` wide on every side.
fn ring(mut d: gpui::Div, w: gpui::Pixels) -> gpui::Div {
    let w: gpui::AbsoluteLength = w.into();
    let b = &mut d.style().border_widths;
    b.top = Some(w);
    b.right = Some(w);
    b.bottom = Some(w);
    b.left = Some(w);
    d
}

/// Estimate: the words of an empty tool.
const EMPTY: &str = "No breakpoints";

pub struct BreakpointsTool {
    root: PathBuf,
    selected: usize,
    /// Open documents' lines, set by the workspace.
    pub line_source: Option<LineSource>,
    /// Files read from disk for their lines, with the time they were read.
    disk: HashMap<PathBuf, (Option<SystemTime>, Vec<String>)>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    reveal: bool,
    tui_grid: Rc<std::cell::Cell<(usize, usize)>>,
    tui_bars: FrameBars,
    tui_scroll: RowScroll,
    _store: Subscription,
}

impl EventEmitter<DebugViewEvent> for BreakpointsTool {}

impl Focusable for BreakpointsTool {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BreakpointsTool {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let store = BreakpointStore::global(cx);
        let observer = cx.observe(&store, |_, _, cx| cx.notify());
        BreakpointsTool {
            root,
            selected: 0,
            line_source: None,
            disk: HashMap::new(),
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            reveal: false,
            tui_grid: Default::default(),
            tui_bars: Default::default(),
            tui_scroll: RowScroll::default(),
            _store: observer,
        }
    }

    /// The scroll bars this view's lists set on its island's frame.
    pub fn tui_bars(&self) -> FrameBars {
        self.tui_bars.clone()
    }

    /// Line `line` of `path` from its open document, else as it stands on
    /// disk (the file read again when its modification time moved).
    fn line_text(&mut self, path: &Path, line: usize, cx: &App) -> Option<String> {
        if let Some(text) = self.line_source.as_ref().and_then(|s| s(path, line, cx)) {
            return Some(text);
        }
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let fresh = self.disk.get(path).is_some_and(|(t, _)| *t == mtime);
        if !fresh {
            let lines = std::fs::read_to_string(path)
                .map(|t| t.lines().map(str::to_string).collect())
                .unwrap_or_default();
            self.disk.insert(path.to_path_buf(), (mtime, lines));
        }
        self.disk.get(path)?.1.get(line).cloned()
    }

    fn rows(&mut self, cx: &App) -> Vec<BpRow> {
        let store = BreakpointStore::global(cx);
        let store = store.read(cx);
        let rows = bp_rows(store.files(), &self.root);
        // Lines of files no longer listed are not kept.
        self.disk.retain(|p, _| rows.iter().any(|r| &r.path == p));
        rows
    }

    fn selected_row(&mut self, cx: &App) -> Option<BpRow> {
        let ix = self.selected;
        self.rows(cx).get(ix).cloned()
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = self.selected.saturating_sub(1);
        self.reveal = true;
        cx.notify();
    }

    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.rows(cx).len();
        self.selected = (self.selected + 1).min(n.saturating_sub(1));
        self.reveal = true;
        cx.notify();
    }

    fn confirm(&mut self, _: &PopupConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.open(self.selected, cx);
    }

    fn open(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows(cx).get(ix).cloned() {
            cx.emit(DebugViewEvent::Open {
                path: row.path,
                line: row.line,
                focus: true,
            });
        }
    }

    fn toggle_enabled(
        &mut self,
        _: &DebugToggleEnabledBreakpoint,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.selected_row(cx) {
            set_enabled(&row.path, row.line, !row.enabled, cx);
        }
    }

    fn remove(&mut self, _: &DebugRemoveBreakpoint, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.selected_row(cx) {
            let store = BreakpointStore::global(cx);
            store.update(cx, |s, cx| s.remove(&row.path, row.line, cx));
        }
    }

    fn click(&mut self, ix: usize, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.selected = ix;
        if count == 2 {
            self.open(ix, cx);
        }
        cx.notify();
    }
}

fn set_enabled(path: &Path, line: usize, enabled: bool, cx: &mut App) {
    let store = BreakpointStore::global(cx);
    store.update(cx, |s, cx| s.set_enabled(path, line, enabled, cx));
}

/// The title's check box: enables every breakpoint unless all are on, then
/// disables them all.
fn set_all(rows: &[BpRow], cx: &mut App) {
    let enable = all_state(rows) != CheckState::All;
    for r in rows {
        if r.enabled != enable {
            set_enabled(&r.path, r.line, enable, cx);
        }
    }
}

impl Render for BreakpointsTool {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        if self.selected >= rows.len() {
            self.selected = rows.len().saturating_sub(1);
        }
        let tui = Skin::current(cx) == Skin::Tui;
        let body = if tui {
            self.render_tui(rows, window, cx)
        } else {
            self.render_fleet(rows, window, cx)
        };
        div()
            .id("breakpoints-tool")
            .key_context("BreakpointsTool")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::toggle_enabled))
            .on_action(cx.listener(Self::remove))
            .size_full()
            .relative()
            .overflow_hidden()
            .child(body)
    }
}

impl BreakpointsTool {
    fn render_fleet(
        &mut self,
        rows: Vec<BpRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        if rows.is_empty() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_secondary)
                .child(EMPTY)
                .into_any_element();
        }
        let focused = self.focus_handle.is_focused(window);
        let entity = cx.entity();
        if self.reveal {
            self.reveal = false;
            self.scroll.scroll_to_item(self.selected);
        }
        let all = all_state(&rows);
        let all_rows = rows.clone();
        let header = div()
            .flex_shrink_0()
            .mt(px(m.breakpoints_top))
            .h(px(m.breakpoints_header))
            .mx(px(m.panel_pad_x))
            .pl(px(m.debug_section_pad_x))
            .pr(px(m.git_checkbox_right))
            .flex()
            .items_center()
            .child(section_title("Breakpoints", &theme, &m))
            .child(
                div()
                    .ml(px(m.breakpoints_count_gap))
                    .text_color(theme.text_secondary)
                    .child(SharedString::from(rows.len().to_string())),
            )
            .child(div().flex_1())
            .child(
                checkbox("bp-check-all", all, &theme, &m).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    set_all(&all_rows, cx);
                }),
            );
        let children: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(ix, row)| {
                let selected = ix == self.selected;
                let click = entity.clone();
                let d = px(m.breakpoints_dot);
                let dot = div().size(d).rounded_full().map(|dot| match row.look() {
                    Look::Enabled => dot.bg(theme.breakpoint),
                    Look::Unverified => dot.bg(theme.breakpoint_unverified),
                    // Estimate: the gutter's ring.
                    Look::Disabled => ring(dot, d * 0.15).border_color(theme.breakpoint),
                });
                let tooltip = self
                    .line_text(&row.path, row.line, cx)
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty());
                let (path, line, enabled) = (row.path.clone(), row.line, row.enabled);
                div()
                    .id(ElementId::NamedInteger("bp-row".into(), ix as u64))
                    .h(px(m.tree_row_height))
                    .flex_shrink_0()
                    .py(px(m.tree_row_inset))
                    .on_click(move |e, window, cx| {
                        let count = e.click_count();
                        click.update(cx, |this, cx| this.click(ix, count, window, cx));
                    })
                    .when_some(tooltip, |d, text| d.tooltip(Tooltip::text(text)))
                    .child(
                        div()
                            .size_full()
                            .pl(px(m.debug_row_pad))
                            .pr(px(m.git_checkbox_right))
                            .flex()
                            .items_center()
                            .gap(px(m.debug_chevron_gap))
                            .rounded(px(m.tree_row_radius))
                            .cursor_pointer()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .when(selected && focused, |d| d.bg(theme.list_bg_focused))
                            .when(selected && !focused, |d| d.bg(theme.list_bg_selected))
                            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                            // The dot centred in the chevron's 16 px box.
                            .child(
                                div()
                                    .size(px(m.icon_size))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(dot),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(theme.list_text)
                                    .child(SharedString::from(row.name)),
                            )
                            .child(
                                div()
                                    .ml(px(m.breakpoints_folder_gap - m.debug_chevron_gap))
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .text_color(theme.list_text_secondary)
                                    .child(SharedString::from(row.folder)),
                            )
                            .child(
                                checkbox(
                                    ElementId::NamedInteger("bp-check".into(), ix as u64),
                                    if enabled {
                                        CheckState::All
                                    } else {
                                        CheckState::Off
                                    },
                                    &theme,
                                    &m,
                                )
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    set_enabled(&path, line, !enabled, cx);
                                }),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("bp-list")
                    .flex_1()
                    .min_h_0()
                    .px(px(m.panel_pad_x))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .children(children),
            )
            .into_any_element()
    }

    /// The tool in the tui skin: the title row ` Breakpoints 1` with `[x]`
    /// (`[-]` when some are off) at its right end, then a row per
    /// breakpoint ` ● debuggee.rs:7 misc` (`○` disabled, `●` in
    /// `breakpoint_unverified` when unbound) with its `[x]` at the right.
    fn render_tui(
        &mut self,
        rows: Vec<BpRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.tui_bars.clear();
        let theme = cx.global::<Theme>().clone();
        let cell = Cell::current(cx);
        let (cols, grid_rows) = self.tui_grid.get();
        let visible = grid_rows.saturating_sub(1);
        let focused = self.focus_handle.is_focused(window);
        let entity = cx.entity();
        let total = rows.len();
        if self.reveal {
            self.reveal = false;
            crate::file_tree::reveal_rows(
                &mut self.tui_scroll,
                Some(self.selected),
                total,
                visible,
            );
        }
        self.tui_scroll.top = self.tui_scroll.top.min(total.saturating_sub(visible));
        let top = self.tui_scroll.top;
        // A check box's three cells, clickable.
        let check = |id: ElementId, text: &'static str| {
            div()
                .id(id)
                .w(cell.cols(3))
                .flex_shrink_0()
                .text_color(theme.text_secondary)
                .cursor_pointer()
                .hover(|s| s.bg(theme.list_bg_hovered))
                .child(text)
        };
        let body: AnyElement = if rows.is_empty() {
            div()
                .h(px(cell.h))
                .text_color(theme.text_tertiary)
                .child(SharedString::from(format!(" {EMPTY}")))
                .into_any_element()
        } else {
            let all = all_state(&rows);
            let all_rows = rows.clone();
            let count = rows.len().to_string();
            let header = div()
                .h(px(cell.h))
                .w(cell.cols(cols))
                .flex_shrink_0()
                .flex()
                .whitespace_nowrap()
                .overflow_hidden()
                .child(div().w(px(cell.w)).flex_shrink_0())
                .child(div().text_color(theme.text_primary).child("Breakpoints "))
                .child(
                    div()
                        .text_color(theme.text_secondary)
                        .child(SharedString::from(count)),
                )
                .child(div().flex_1())
                .child(
                    check(
                        "bp-check-all".into(),
                        match all {
                            CheckState::All => "[x]",
                            CheckState::Some => "[-]",
                            CheckState::Off => "[ ]",
                        },
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        set_all(&all_rows, cx);
                    }),
                )
                .child(div().w(px(cell.w)).flex_shrink_0());
            let children: Vec<AnyElement> = rows
                .into_iter()
                .enumerate()
                .skip(top)
                .take(visible)
                .map(|(ix, row)| {
                    let selected = ix == self.selected;
                    let click = entity.clone();
                    let (glyph, color) = match row.look() {
                        Look::Enabled => ("●", theme.breakpoint),
                        Look::Unverified => ("●", theme.breakpoint_unverified),
                        Look::Disabled => ("○", theme.breakpoint),
                    };
                    let room = cols.saturating_sub(3 + 5);
                    let name = skin::fit_text(&row.name, room);
                    let folder_room = room.saturating_sub(Cell::text_cols(&name) + 1);
                    let folder = skin::fit_text(&row.folder, folder_room);
                    let (path, line, enabled) = (row.path.clone(), row.line, row.enabled);
                    div()
                        .id(ElementId::NamedInteger("bp-row".into(), (ix - top) as u64))
                        .h(px(cell.h))
                        .w(cell.cols(cols))
                        .flex_shrink_0()
                        .flex()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .cursor_pointer()
                        .when(selected && focused, |d| d.bg(theme.list_bg_focused))
                        .when(selected && !focused, |d| d.bg(theme.list_bg_selected))
                        .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                        .on_click(move |e, window, cx| {
                            let count = e.click_count();
                            click.update(cx, |this, cx| this.click(ix, count, window, cx));
                        })
                        .child(div().w(px(cell.w)).flex_shrink_0())
                        .child(
                            div()
                                .w(cell.cols(2))
                                .flex_shrink_0()
                                .text_color(color)
                                .child(glyph),
                        )
                        .child(
                            div()
                                .text_color(theme.list_text)
                                .child(SharedString::from(name)),
                        )
                        .when(!folder.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_color(theme.list_text_secondary)
                                    .child(SharedString::from(format!(" {folder}"))),
                            )
                        })
                        .child(div().flex_1())
                        .child(
                            check(
                                ElementId::NamedInteger("bp-check".into(), (ix - top) as u64),
                                skin::checkbox(enabled),
                            )
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                set_enabled(&path, line, !enabled, cx);
                            }),
                        )
                        .child(div().w(px(cell.w)).flex_shrink_0())
                        .into_any_element()
                })
                .collect();
            div()
                .flex()
                .flex_col()
                .child(header)
                .child(tui_list(
                    "bp-list",
                    &entity,
                    |this: &mut BreakpointsTool| &mut this.tui_scroll,
                    top,
                    total,
                    cols,
                    visible,
                    div()
                        .flex()
                        .flex_col()
                        .children(children)
                        .into_any_element(),
                    cell,
                    &self.tui_bars,
                ))
                .into_any_element()
        };
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font_family(theme.mono_font.clone())
            .text_size(px(cell.font_size))
            .line_height(px(cell.h))
            .child(skin::measure(cell, &self.tui_grid))
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_name_the_file_and_line_with_the_folder() {
        let a = PathBuf::from("/w/misc/debuggee.rs");
        let b = PathBuf::from("/elsewhere/lib.rs");
        let c = PathBuf::from("/w/main.rs");
        let mut off = LineBreakpoint::at(4);
        off.enabled = false;
        let la = vec![off, LineBreakpoint::at(6)];
        let lb = vec![LineBreakpoint::at(0)];
        let lc = vec![LineBreakpoint::at(2)];
        let empty: Vec<LineBreakpoint> = Vec::new();
        let none = PathBuf::from("/w/none.rs");
        let files = vec![
            (a.as_path(), la.as_slice()),
            (none.as_path(), empty.as_slice()),
            (b.as_path(), lb.as_slice()),
            (c.as_path(), lc.as_slice()),
        ];
        let rows = bp_rows(files, Path::new("/w"));
        let short: Vec<(&str, &str, bool)> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.folder.as_str(), r.enabled))
            .collect();
        assert_eq!(
            short,
            vec![
                ("debuggee.rs:5", "misc", false),
                ("debuggee.rs:7", "misc", true),
                ("lib.rs:1", "/elsewhere", true),
                ("main.rs:3", "", true),
            ]
        );
        assert_eq!(rows[1].path, a);
        assert_eq!(rows[1].line, 6);
        assert_eq!(rows[0].look(), Look::Disabled);
        assert!(all_state(&rows) == CheckState::Some);
        assert!(all_state(&rows[1..]) == CheckState::All);
        assert!(all_state(&rows[..1]) == CheckState::Off);
    }
}
