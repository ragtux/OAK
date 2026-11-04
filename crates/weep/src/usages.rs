//! Fleet's inline usages (`find-usages`): a panel laid between the caret's
//! line and the next (`EditorView::set_interline`), the usages grouped by file
//! in its left third and the selected one's file in a read-only preview on
//! the right, with the path under it and a close button over it. Measured in
//! the fifteenth pass (`docs/fleet-probe.md`); the colours are Fleet's
//! `editor.interline.*` keys.

use crate::actions::{Cancel, MoveDown, MoveLeft, MoveRight, MoveUp, Newline};
use crate::assets::{file_icon, icon};
use crate::editor::{EditorView, Search};
use crate::goto::OpenDocument;
use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, relative, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, MouseButton, SharedString, Window,
};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// One usage: where it is in its document (chars), its row, and the line's
/// text as the list shows it (leading blanks trimmed) with the usage's char
/// columns in that text.
#[derive(Clone, Debug)]
pub struct Usage {
    pub range: Range<usize>,
    pub row: usize,
    pub text: String,
    pub hit: Range<usize>,
}

/// The usages in one file.
#[derive(Clone, Debug)]
pub struct UsageFile {
    pub path: PathBuf,
    /// The folder under the workspace root (`src`), shown dimmed after the name.
    pub dir: String,
    pub usages: Vec<Usage>,
}

pub enum UsagesEvent {
    /// Close the panel (Escape, the close button).
    Close,
    /// Go to a usage (Enter, a double click), closing the panel.
    Open(PathBuf, Range<usize>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    File(usize),
    Usage(usize, usize),
}

pub struct UsagesPanel {
    files: Vec<UsageFile>,
    collapsed: Vec<bool>,
    selected: Row,
    /// The first list row shown, so the selection stays in view.
    top: usize,
    preview: Option<(PathBuf, Entity<EditorView>)>,
    open_document: OpenDocument,
    focus_handle: FocusHandle,
}

impl EventEmitter<UsagesEvent> for UsagesPanel {}

impl Focusable for UsagesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl UsagesPanel {
    pub fn new(files: Vec<UsageFile>, open_document: OpenDocument, cx: &mut Context<Self>) -> Self {
        let mut panel = UsagesPanel {
            collapsed: vec![false; files.len()],
            files,
            selected: Row::File(0),
            top: 0,
            preview: None,
            open_document,
            focus_handle: cx.focus_handle(),
        };
        panel.show_preview(cx);
        panel
    }

    /// How many usages in all.
    pub fn count(&self) -> usize {
        self.files.iter().map(|f| f.usages.len()).sum()
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (f, file) in self.files.iter().enumerate() {
            rows.push(Row::File(f));
            if !self.collapsed[f] {
                rows.extend((0..file.usages.len()).map(|u| Row::Usage(f, u)));
            }
        }
        rows
    }

    /// The usage the preview shows: the selected one, or a file's first.
    fn previewed(&self) -> Option<(usize, usize)> {
        match self.selected {
            Row::Usage(f, u) => Some((f, u)),
            Row::File(f) => (!self.files.get(f)?.usages.is_empty()).then_some((f, 0)),
        }
    }

    fn show_preview(&mut self, cx: &mut Context<Self>) {
        let Some((f, u)) = self.previewed() else {
            self.preview = None;
            return;
        };
        let file = &self.files[f];
        let editor = match &self.preview {
            Some((p, e)) if *p == file.path => e.clone(),
            _ => {
                let Some(document) = (self.open_document)(&file.path, cx) else {
                    self.preview = None;
                    return;
                };
                let wash = {
                    let theme = cx.global::<Theme>();
                    theme.interline_match_bg.opacity(0.4)
                };
                let editor = cx.new(|cx| {
                    let mut e = EditorView::new(document, cx);
                    e.read_only = true;
                    e.bare = true;
                    e.line_wash = Some(wash);
                    e
                });
                self.preview = Some((file.path.clone(), editor.clone()));
                editor
            }
        };
        let usage = file.usages[u].clone();
        let matches: Vec<Range<usize>> = file.usages.iter().map(|x| x.range.clone()).collect();
        editor.update(cx, |e, cx| {
            let column = {
                let b = &e.document.read(cx).buffer;
                usage.range.start.saturating_sub(b.line_to_char(usage.row))
            };
            e.goto(usage.row, column, cx);
            e.set_search(
                Some(Search {
                    matches,
                    current: Some(u),
                }),
                cx,
            );
        });
    }

    fn select(&mut self, row: Row, cx: &mut Context<Self>) {
        self.selected = row;
        self.show_preview(cx);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.rows();
        let at = rows.iter().position(|r| *r == self.selected).unwrap_or(0) as isize;
        let next = (at + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.select(rows[next], cx);
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        match self.selected {
            Row::File(f) => {
                self.collapsed[f] = !self.collapsed[f];
                cx.notify();
            }
            Row::Usage(f, u) => {
                let file = &self.files[f];
                cx.emit(UsagesEvent::Open(
                    file.path.clone(),
                    file.usages[u].range.clone(),
                ));
            }
        }
    }

    /// Left: fold the file and select it.
    fn collapse(&mut self, cx: &mut Context<Self>) {
        let (Row::File(f) | Row::Usage(f, _)) = self.selected;
        self.collapsed[f] = true;
        self.select(Row::File(f), cx);
    }

    /// Right: unfold the selected file.
    fn expand(&mut self, cx: &mut Context<Self>) {
        if let Row::File(f) = self.selected {
            self.collapsed[f] = false;
            cx.notify();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_row(
        &self,
        ix: usize,
        row: Row,
        focused: bool,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let selected = row == self.selected;
        let entity = cx.entity();
        let base = div()
            .id(("usage-row", ix))
            .h(px(m.usages_row_height))
            .flex_shrink_0()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .rounded(px(4. * m.zoom))
            .when(selected, |d| {
                d.bg(if focused {
                    theme.list_bg_focused
                } else {
                    theme.list_bg_selected
                })
            })
            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
            .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                let handle = entity.read(cx).focus_handle.clone();
                window.focus(&handle, cx);
                entity.update(cx, |this, cx| {
                    this.select(row, cx);
                    if e.click_count >= 2 {
                        this.open(cx);
                    }
                });
            });
        match row {
            Row::File(f) => {
                let file = &self.files[f];
                let name = file
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let chevron = if self.collapsed[f] {
                    "expui/general/chevronRight"
                } else {
                    "expui/general/chevronDown"
                };
                base.pl(px(6. * m.zoom))
                    .pr(px(4. * m.zoom))
                    .gap(px(4. * m.zoom))
                    .child(icon(chevron, m.icon_size, theme.icon_secondary))
                    .child(icon(file_icon(&name), m.icon_size, theme.icon_primary))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .flex()
                            .gap(px(4. * m.zoom))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text_primary)
                                    .child(SharedString::from(name)),
                            )
                            .child(
                                div()
                                    .text_color(theme.text_tertiary)
                                    .child(SharedString::from(file.dir.clone())),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .min_w(px(16. * m.zoom))
                            .h(px(16. * m.zoom))
                            .px(px(4. * m.zoom))
                            .rounded(px(8. * m.zoom))
                            .bg(theme.search_counter_bg)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(m.ui_font_size - 2. * m.zoom))
                            .text_color(theme.text_primary)
                            .child(SharedString::from(file.usages.len().to_string())),
                    )
                    .into_any_element()
            }
            Row::Usage(f, u) => {
                let usage = &self.files[f].usages[u];
                let chars: Vec<char> = usage.text.chars().collect();
                // A usage ending past the row's width (about 40 chars of the UI font)
                // starts the row 17 chars before it, so the word shows: Fleet kept
                // a match ending at 39 and shifted one ending at 41 (fifteenth pass).
                let skip = if usage.hit.end > 40 {
                    usage.hit.start.saturating_sub(17)
                } else {
                    0
                };
                let part = |r: Range<usize>| -> SharedString {
                    chars[r.start.max(skip).min(chars.len())..r.end.max(skip).min(chars.len())]
                        .iter()
                        .collect::<String>()
                        .into()
                };
                let hit = usage.hit.start.min(chars.len())..usage.hit.end.min(chars.len());
                base.pl(px(m.usages_item_indent))
                    .text_color(theme.text_primary)
                    .child(div().child(part(0..hit.start)))
                    .child(
                        div()
                            .rounded(px(2. * m.zoom))
                            .bg(theme.interline_match_bg)
                            .text_color(theme.interline_match_text)
                            .child(part(hit.clone())),
                    )
                    .child(div().child(part(hit.end..chars.len())))
                    .into_any_element()
            }
        }
    }
}

/// The folder of `path` under `root` as the list prints it (`src`), empty at the root.
pub fn dir_of(root: &Path, path: &Path) -> String {
    path.parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

impl Render for UsagesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let focused = self.focus_handle.contains_focused(window, cx);
        let count = self.count();
        let files = self.files.len();
        let summary = format!(
            " {count} in {files} file{}",
            if files == 1 { "" } else { "s" }
        );
        // The list rows that fit under the header, scrolled so the selection shows.
        let rows = self.rows();
        let fit =
            (((m.usages_height - m.usages_rows_top) / m.usages_row_pitch).floor() as usize).max(1);
        let at = rows.iter().position(|r| *r == self.selected).unwrap_or(0);
        if at < self.top {
            self.top = at;
        } else if at >= self.top + fit {
            self.top = at + 1 - fit;
        }
        self.top = self.top.min(rows.len().saturating_sub(fit));
        let list: Vec<gpui::AnyElement> = rows
            .iter()
            .enumerate()
            .skip(self.top)
            .take(fit)
            .map(|(ix, row)| self.render_row(ix, *row, focused, &theme, &m, cx))
            .collect();
        let path_parts: Vec<String> = self
            .previewed()
            .map(|(f, _)| {
                let file = &self.files[f];
                let mut parts: Vec<String> = file
                    .dir
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                if let Some(name) = file.path.file_name() {
                    parts.push(name.to_string_lossy().into_owned());
                }
                parts
            })
            .unwrap_or_default();
        let mut footer = div()
            .h(px(m.usages_footer_height))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6. * m.zoom))
            .pl(px(m.usages_pad_x))
            .text_color(theme.text_secondary);
        for (i, part) in path_parts.into_iter().enumerate() {
            if i > 0 {
                footer = footer.child(div().text_color(theme.text_tertiary).child("/"));
            }
            footer = footer.child(SharedString::from(part));
        }
        let close_entity = cx.entity();
        let close = div()
            .id("usages-close")
            .absolute()
            .top(px(m.usages_close_top))
            .right(px(m.usages_close_right))
            .size(px(m.usages_close_size))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4. * m.zoom))
            .bg(theme.interline_bg)
            .cursor_pointer()
            .hover(|s| s.bg(theme.ghost_hovered))
            .on_click(move |_, _, cx| close_entity.update(cx, |_, cx| cx.emit(UsagesEvent::Close)))
            .child(icon(
                "expui/general/closeSmall",
                m.icon_size,
                theme.icon_primary,
            ));
        let preview = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pt(px(m.usages_preview_top))
            .pr(px(m.usages_preview_right))
            .pb(px(m.usages_preview_bottom))
            .child(
                div()
                    .size_full()
                    .relative()
                    .overflow_hidden()
                    .rounded(px(m.usages_radius))
                    .bg(theme.interline_preview_bg)
                    .border_1()
                    .border_color(theme.interline_preview_border)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .children(self.preview.as_ref().map(|(_, e)| e.clone())),
                    )
                    .child(footer)
                    .child(close),
            );
        div()
            .id("usages")
            .key_context("Usages")
            .track_focus(&self.focus_handle)
            // The panel lies inside the editor, whose bindings for these
            // keys would move its caret: the panel takes the same actions first.
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| this.collapse(cx)))
            .on_action(cx.listener(|this, _: &MoveRight, _, cx| this.expand(cx)))
            .on_action(cx.listener(|this, _: &Newline, _, cx| this.open(cx)))
            .on_action(cx.listener(|_, _: &Cancel, _, cx| cx.emit(UsagesEvent::Close)))
            .size_full()
            .pt(px(m.usages_gap_top))
            .pb(px(m.usages_gap_bottom))
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            .child(
                div()
                    .occlude()
                    .size_full()
                    .rounded(px(m.usages_radius))
                    .bg(theme.interline_bg)
                    .flex()
                    .child(
                        div()
                            .w(relative(1. / 3.))
                            .flex_shrink_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .h(px(m.usages_rows_top))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .pl(px(m.usages_pad_x))
                                    .child(div().text_color(theme.text_primary).child("Usages"))
                                    .child(
                                        div()
                                            .text_color(theme.interline_text)
                                            .child(SharedString::from(summary)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_hidden()
                                    .pl(px(m.usages_row_inset_left))
                                    .pr(px(m.usages_row_inset_right))
                                    .flex()
                                    .flex_col()
                                    .gap(px(m.usages_row_pitch - m.usages_row_height))
                                    .children(list),
                            ),
                    )
                    .child(preview),
            )
    }
}
