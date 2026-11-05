//! The Breakpoints tool (M8): every line breakpoint of the app's
//! `BreakpointStore`, grouped by file. A file's row is its icon and its
//! path relative to the workspace root; under it one row per breakpoint,
//! `line N: <the line's text>` with a check box that enables or disables it.
//! No probe of Fleet's tool exists yet: the rows reuse the Files tree's
//! metrics and the Git tool's check box, and the empty text is an estimate.
//!
//! Keys (context `BreakpointsTool`): arrows move, Space toggles the check
//! box (`debug/toggle-enabled-breakpoint`), Delete or Backspace removes
//! (`debug/remove-breakpoint`), Enter or a double click opens the file at
//! the line through [`DebugViewEvent::Open`].

use crate::actions::{
    DebugRemoveBreakpoint, DebugToggleEnabledBreakpoint, PopupConfirm, PopupDown, PopupUp,
};
use crate::assets::{file_icon, icon};
use crate::breakpoints::{BreakpointStore, LineBreakpoint};
use crate::debug_tool::{relative_path, DebugViewEvent};
use crate::git_tool::{checkbox, tui_list, CheckState};
use crate::skin::{self, Cell, FrameBars, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, AnyElement, App, Context, ElementId, EventEmitter, FocusHandle, Focusable,
    Render, ScrollHandle, SharedString, Subscription, Window,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

/// A line's text from an open document, when the file is open (the
/// workspace sets it; lines on disk otherwise).
pub type LineSource = Rc<dyn Fn(&Path, usize, &App) -> Option<String>>;

/// One row of the tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BpRow {
    File {
        path: PathBuf,
        label: String,
    },
    Line {
        path: PathBuf,
        line: usize,
        enabled: bool,
        /// `Some(false)`: the running session could not bind it.
        verified: Option<bool>,
        text: String,
    },
}

/// A breakpoint's row text: `line 12: let x = 1;`, 1-based, the line trimmed.
pub fn breakpoint_label(line: usize, text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        format!("line {}", line + 1)
    } else {
        format!("line {}: {text}", line + 1)
    }
}

/// The rows for `files`: a file row, then its breakpoints in line order.
pub fn bp_rows<'a>(
    files: impl IntoIterator<Item = (&'a Path, &'a [LineBreakpoint])>,
    root: &Path,
    mut line_text: impl FnMut(&Path, usize) -> Option<String>,
) -> Vec<BpRow> {
    let mut rows = Vec::new();
    for (path, list) in files {
        if list.is_empty() {
            continue;
        }
        rows.push(BpRow::File {
            path: path.to_path_buf(),
            label: relative_path(path, root),
        });
        for b in list {
            let text = line_text(path, b.line).unwrap_or_default();
            rows.push(BpRow::Line {
                path: path.to_path_buf(),
                line: b.line,
                enabled: b.enabled,
                verified: b.verified,
                text: breakpoint_label(b.line, &text),
            });
        }
    }
    rows
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

    /// Line `line` of `path` as it stands on disk, the file read again when
    /// its modification time moved.
    fn disk_line(&mut self, path: &Path, line: usize) -> Option<String> {
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
        let files: Vec<(PathBuf, Vec<LineBreakpoint>)> = store
            .files()
            .map(|(p, b)| (p.to_path_buf(), b.to_vec()))
            .collect();
        let source = self.line_source.clone();
        let root = self.root.clone();
        let rows = bp_rows(
            files.iter().map(|(p, b)| (p.as_path(), b.as_slice())),
            &root,
            |path, line| {
                source
                    .as_ref()
                    .and_then(|s| s(path, line, cx))
                    .or_else(|| self.disk_line(path, line))
            },
        );
        // Lines of files no longer listed are not kept.
        self.disk.retain(|p, _| files.iter().any(|(f, _)| f == p));
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
        let line = match self.rows(cx).get(ix) {
            Some(BpRow::Line { path, line, .. }) => Some((path.clone(), *line)),
            _ => None,
        };
        if let Some((path, line)) = line {
            cx.emit(DebugViewEvent::Open {
                path,
                line,
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
        if let Some(BpRow::Line {
            path,
            line,
            enabled,
            ..
        }) = self.selected_row(cx)
        {
            set_enabled(&path, line, !enabled, cx);
        }
    }

    /// Delete: the selected breakpoint, or every one of the selected file.
    fn remove(&mut self, _: &DebugRemoveBreakpoint, _: &mut Window, cx: &mut Context<Self>) {
        let store = BreakpointStore::global(cx);
        match self.selected_row(cx) {
            Some(BpRow::Line { path, line, .. }) => {
                store.update(cx, |s, cx| s.remove(&path, line, cx));
            }
            Some(BpRow::File { path, .. }) => {
                store.update(cx, |s, cx| s.set_file(&path, Vec::new(), cx));
            }
            None => {}
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
        let children: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(ix, row)| {
                let selected = ix == self.selected;
                let click = entity.clone();
                let inner = div()
                    .size_full()
                    .pl(px(m.tree_pad_left))
                    .pr(px(m.tree_pad_left))
                    .flex()
                    .items_center()
                    .gap(px(m.tree_icon_gap))
                    .rounded(px(m.tree_row_radius))
                    .cursor_pointer()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .when(selected && focused, |d| d.bg(theme.list_bg_focused))
                    .when(selected && !focused, |d| d.bg(theme.list_bg_selected))
                    .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)));
                let inner = match row {
                    BpRow::File { label, .. } => {
                        let name = Path::new(&label)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        inner
                            .child(icon(file_icon(&name), m.icon_size, theme.icon_secondary))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.list_text)
                                    .child(SharedString::from(label)),
                            )
                    }
                    BpRow::Line {
                        path,
                        line,
                        enabled,
                        verified,
                        text,
                    } => {
                        let toggle_path = path.clone();
                        inner
                            // Estimate: a breakpoint sits one tree level in.
                            .pl(px(m.tree_pad_left + m.tree_indent))
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
                                    set_enabled(&toggle_path, line, !enabled, cx);
                                }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(if !enabled || verified == Some(false) {
                                        theme.text_tertiary
                                    } else {
                                        theme.list_text
                                    })
                                    .child(SharedString::from(text)),
                            )
                    }
                };
                div()
                    .id(ElementId::NamedInteger("bp-row".into(), ix as u64))
                    .h(px(m.tree_row_height))
                    .flex_shrink_0()
                    .py(px(m.tree_row_inset))
                    .on_click(move |e, window, cx| {
                        let count = e.click_count();
                        click.update(cx, |this, cx| this.click(ix, count, window, cx));
                    })
                    .child(inner)
                    .into_any_element()
            })
            .collect();
        div()
            .id("bp-list")
            .size_full()
            .px(px(m.panel_pad_x))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .children(children)
            .into_any_element()
    }

    /// The tool in the tui skin: a file row `◆ src/main.rs`, then its
    /// breakpoints `[x] line 12: text` one level in, on whole rows.
    fn render_tui(
        &mut self,
        rows: Vec<BpRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.tui_bars.clear();
        let theme = cx.global::<Theme>().clone();
        let cell = Cell::current(cx);
        let (cols, visible) = self.tui_grid.get();
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
        let body: AnyElement = if rows.is_empty() {
            div()
                .h(px(cell.h))
                .text_color(theme.text_tertiary)
                .child(SharedString::from(format!(" {EMPTY}")))
                .into_any_element()
        } else {
            let children: Vec<AnyElement> = rows
                .into_iter()
                .enumerate()
                .skip(top)
                .take(visible)
                .map(|(ix, row)| {
                    let selected = ix == self.selected;
                    let click = entity.clone();
                    let d = div()
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
                        });
                    match row {
                        BpRow::File { label, .. } => {
                            let (glyph, color) = skin::file_glyph(&label, &theme);
                            d.child(div().w(px(cell.w)).flex_shrink_0())
                                .child(
                                    div()
                                        .w(px(cell.w))
                                        .flex_shrink_0()
                                        .text_color(color)
                                        .child(glyph),
                                )
                                .child(div().w(px(cell.w)).flex_shrink_0())
                                .child(
                                    div()
                                        .text_color(theme.list_text)
                                        .child(skin::fit_text(&label, cols.saturating_sub(3))),
                                )
                        }
                        BpRow::Line {
                            path,
                            line,
                            enabled,
                            verified,
                            text,
                        } => d
                            .child(div().w(cell.cols(3)).flex_shrink_0())
                            .child(
                                div()
                                    .id(ElementId::NamedInteger(
                                        "bp-check".into(),
                                        (ix - top) as u64,
                                    ))
                                    .w(cell.cols(3))
                                    .flex_shrink_0()
                                    .text_color(theme.text_secondary)
                                    .hover(|s| s.bg(theme.list_bg_hovered))
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        set_enabled(&path, line, !enabled, cx);
                                    })
                                    .child(skin::checkbox(enabled)),
                            )
                            .child(div().w(px(cell.w)).flex_shrink_0())
                            .child(
                                div()
                                    .text_color(if !enabled || verified == Some(false) {
                                        theme.text_tertiary
                                    } else {
                                        theme.list_text
                                    })
                                    .child(skin::fit_text(&text, cols.saturating_sub(7))),
                            ),
                    }
                    .into_any_element()
                })
                .collect();
            tui_list(
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
            )
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
    fn labels_are_one_based_and_trimmed() {
        assert_eq!(
            breakpoint_label(0, "    let x = 1;  "),
            "line 1: let x = 1;"
        );
        assert_eq!(breakpoint_label(9, "   "), "line 10");
    }

    #[test]
    fn rows_group_by_file() {
        let a = PathBuf::from("/w/src/main.rs");
        let b = PathBuf::from("/elsewhere/lib.rs");
        let mut off = LineBreakpoint::at(4);
        off.enabled = false;
        let la = vec![LineBreakpoint::at(1), off];
        let lb = vec![LineBreakpoint::at(0)];
        let empty: Vec<LineBreakpoint> = Vec::new();
        let c = PathBuf::from("/w/none.rs");
        let files = vec![
            (a.as_path(), la.as_slice()),
            (c.as_path(), empty.as_slice()),
            (b.as_path(), lb.as_slice()),
        ];
        let rows = bp_rows(files, Path::new("/w"), |path, line| {
            (path.ends_with("main.rs")).then(|| format!("  code {line}"))
        });
        assert_eq!(
            rows,
            vec![
                BpRow::File {
                    path: a.clone(),
                    label: "src/main.rs".into()
                },
                BpRow::Line {
                    path: a.clone(),
                    line: 1,
                    enabled: true,
                    verified: None,
                    text: "line 2: code 1".into()
                },
                BpRow::Line {
                    path: a.clone(),
                    line: 4,
                    enabled: false,
                    verified: None,
                    text: "line 5: code 4".into()
                },
                BpRow::File {
                    path: b.clone(),
                    label: "/elsewhere/lib.rs".into()
                },
                BpRow::Line {
                    path: b.clone(),
                    line: 0,
                    enabled: true,
                    verified: None,
                    text: "line 1".into()
                },
            ]
        );
    }
}
