//! The Search tool: `find-in-path` and `replace-in-path` in the panel.
//! Fleet's tool keeps a query with `text`, `caseSensitive`, `words`, `regex`,
//! a file `mask`, a `limit` of 100, a `replacement` and a `searchMode` of
//! FIND or REPLACE (its local storage, `docs/fleet-probe.md`); the fields
//! and toggles here are the same, drawn like the find bar's (`inputField.*`,
//! `toggleButton.*`), with the results grouped by file under the fields and
//! the matched text in `search.match.*`. The tool searches as the query
//! changes, on a background thread, with the open documents' unsaved text
//! in place of the disk's. No probe has measured this tool yet (it is not in
//! batches 6–8); the geometry is an estimate (`theme.rs::metrics`).

use crate::actions::{PopupDown, PopupUp};
use crate::assets::{file_icon, icon};
use crate::input::{InputEvent, TextInput};
use crate::settings::AppSettings;
use crate::theme::{Metrics, Theme};
use crate::tooltip::Tooltip;
use gpui::{
    div, point, prelude::*, px, App, BoxShadow, Context, Div, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, HighlightStyle, ScrollHandle, SharedString, Stateful,
    StyledText, Task, Window,
};
use oak_search::{FileMatches, GrepOptions, GrepQuery};
use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

/// Fleet's stored result limit.
const RESULT_LIMIT: usize = 100;
/// How long after the last keystroke the search starts (estimate).
const DEBOUNCE_MS: u64 = 150;

#[derive(Clone, Debug)]
pub enum SearchToolEvent {
    /// Open the file at the match: `range` is the byte range inside the line.
    OpenMatch {
        path: PathBuf,
        row: usize,
        range: Range<usize>,
        split: bool,
    },
    /// Replace every match of the current query in the listed files.
    ReplaceAll {
        query: GrepQuery,
        replacement: String,
        files: Vec<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Toggle {
    Case,
    Word,
    Regex,
}

/// One visible row of the results: a file head or a matching line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    File(usize),
    Line(usize, usize),
}

/// Supplies the open documents' current text, by path.
pub type Overrides = Rc<dyn Fn(&mut App) -> Vec<(PathBuf, String)>>;

pub struct SearchTool {
    root: PathBuf,
    pub query: Entity<TextInput>,
    pub replace: Entity<TextInput>,
    mask: Entity<TextInput>,
    pub replace_mode: bool,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    results: Vec<FileMatches>,
    collapsed: HashSet<PathBuf>,
    selected: Option<Row>,
    error: Option<String>,
    searching: bool,
    truncated: bool,
    generation: u64,
    pending: Option<Task<()>>,
    /// The open documents' current text, searched instead of the disk's.
    overrides: Overrides,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<SearchToolEvent> for SearchTool {}

impl Focusable for SearchTool {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
}

impl SearchTool {
    pub fn new(root: PathBuf, overrides: Overrides, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            let mut i = TextInput::new("Search in files", cx);
            i.mono = true;
            i
        });
        let replace = cx.new(|cx| {
            let mut i = TextInput::new("Replace", cx);
            i.mono = true;
            i
        });
        let mask = cx.new(|cx| TextInput::new("File mask: *.rs, *.toml", cx));
        cx.subscribe(&query, Self::on_query_input).detach();
        cx.subscribe(&replace, Self::on_replace_input).detach();
        cx.subscribe(&mask, Self::on_mask_input).detach();
        SearchTool {
            root,
            query,
            replace,
            mask,
            replace_mode: false,
            case_sensitive: false,
            whole_word: false,
            regex: false,
            results: Vec::new(),
            collapsed: HashSet::new(),
            selected: None,
            error: None,
            searching: false,
            truncated: false,
            generation: 0,
            pending: None,
            overrides,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    /// `find-in-path` / `replace-in-path`: seeds the query with `seed` (the
    /// editor's one-line selection) and focuses the field.
    pub fn show(
        &mut self,
        with_replace: bool,
        seed: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_mode = self.replace_mode || with_replace;
        match seed.filter(|s| !s.is_empty() && !s.contains('\n')) {
            Some(text) => self.query.update(cx, |q, cx| {
                q.set_text(&text, cx);
                q.select_all(cx);
            }),
            None => self.query.update(cx, |q, cx| q.select_all(cx)),
        }
        self.query.read(cx).focus_handle(cx).focus(window, cx);
        self.schedule_search(cx);
        cx.notify();
    }

    pub fn query_text(&self, cx: &App) -> String {
        self.query.read(cx).text().to_string()
    }

    fn options(&self, cx: &App) -> GrepOptions {
        let settings = &cx.global::<AppSettings>().settings;
        GrepOptions {
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            mask: self.mask.read(cx).text().to_string(),
            path: String::new(),
            exclude: settings.files_exclude(),
            search_big_files: settings.ff_search_in_big_unindexed_files(),
            limit: RESULT_LIMIT,
        }
    }

    fn current_query(&self, cx: &App) -> Result<GrepQuery, String> {
        GrepQuery::new(&self.query_text(cx), self.options(cx))
    }

    /// Searches once the field has been quiet for `DEBOUNCE_MS`.
    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.search_now(cx);
                }
            });
        }));
    }

    /// Runs the search on the background executor; the results land here.
    pub fn search_now(&mut self, cx: &mut Context<Self>) {
        let query = match self.current_query(cx) {
            Ok(q) => q,
            Err(message) => {
                self.results.clear();
                self.error = (!message.is_empty()).then_some(message);
                self.searching = false;
                self.truncated = false;
                self.selected = None;
                cx.notify();
                return;
            }
        };
        self.error = None;
        self.searching = true;
        let root = self.root.clone();
        let overrides = (self.overrides)(cx);
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { query.search(&root, &overrides) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.truncated = found.iter().map(|f| f.lines.len()).sum::<usize>() >= RESULT_LIMIT;
                this.results = found;
                this.searching = false;
                this.selected = this.rows().first().copied();
                this.scroll.scroll_to_item(0);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn on_query_input(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => self.schedule_search(cx),
            InputEvent::Submit { shift } => {
                if self.results.is_empty() {
                    self.generation += 1;
                    self.search_now(cx);
                } else {
                    self.open_selected(*shift, cx);
                }
            }
            InputEvent::Tab { shift: false } => self.defer_focus(
                if self.replace_mode {
                    self.replace.clone()
                } else {
                    self.mask.clone()
                },
                cx,
            ),
            InputEvent::Tab { shift: true } => self.defer_focus(self.mask.clone(), cx),
            InputEvent::Cancel => {}
        }
    }

    fn on_replace_input(
        &mut self,
        _: Entity<TextInput>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Submit { .. } => self.replace_all(cx),
            InputEvent::Tab { shift: false } => self.defer_focus(self.mask.clone(), cx),
            InputEvent::Tab { shift: true } => self.defer_focus(self.query.clone(), cx),
            _ => {}
        }
    }

    fn on_mask_input(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => self.schedule_search(cx),
            InputEvent::Submit { .. } => {
                self.generation += 1;
                self.search_now(cx);
            }
            InputEvent::Tab { shift: false } => self.defer_focus(self.query.clone(), cx),
            InputEvent::Tab { shift: true } => self.defer_focus(
                if self.replace_mode {
                    self.replace.clone()
                } else {
                    self.query.clone()
                },
                cx,
            ),
            InputEvent::Cancel => {}
        }
    }

    /// Focus moves after the current event finishes, through the window.
    fn defer_focus(&self, input: Entity<TextInput>, cx: &mut Context<Self>) {
        cx.defer(move |cx| {
            if let Some(window) = cx.windows().first().copied() {
                let _ = window.update(cx, |_, window, cx| {
                    input.read(cx).focus_handle(cx).focus(window, cx);
                });
            }
        });
    }

    fn toggle(&mut self, which: Toggle, cx: &mut Context<Self>) {
        match which {
            Toggle::Case => self.case_sensitive = !self.case_sensitive,
            Toggle::Word => self.whole_word = !self.whole_word,
            Toggle::Regex => self.regex = !self.regex,
        }
        self.generation += 1;
        self.search_now(cx);
    }

    fn toggle_replace(&mut self, cx: &mut Context<Self>) {
        self.replace_mode = !self.replace_mode;
        if self.replace_mode {
            self.defer_focus(self.replace.clone(), cx);
        }
        cx.notify();
    }

    fn replace_all(&mut self, cx: &mut Context<Self>) {
        let Ok(query) = self.current_query(cx) else {
            return;
        };
        if self.results.is_empty() {
            return;
        }
        cx.emit(SearchToolEvent::ReplaceAll {
            query,
            replacement: self.replace.read(cx).text().to_string(),
            files: self.results.iter().map(|f| f.path.clone()).collect(),
        });
    }

    /// After a replacement or a disk change: the same query again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.query_text(cx).is_empty() {
            self.generation += 1;
            self.search_now(cx);
        }
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (f, file) in self.results.iter().enumerate() {
            rows.push(Row::File(f));
            if !self.collapsed.contains(&file.path) {
                for l in 0..file.lines.len() {
                    rows.push(Row::Line(f, l));
                }
            }
        }
        rows
    }

    fn total_matches(&self) -> usize {
        self.results.iter().map(FileMatches::count).sum()
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let pos = self
            .selected
            .and_then(|s| rows.iter().position(|r| *r == s))
            .map(|p| p as isize)
            .unwrap_or(-1);
        let next = (pos + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.selected = Some(rows[next]);
        self.scroll.scroll_to_item(next);
        cx.notify();
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }
    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    fn open_selected(&mut self, split: bool, cx: &mut Context<Self>) {
        match self.selected {
            Some(Row::Line(f, l)) => {
                let file = &self.results[f];
                let line = &file.lines[l];
                cx.emit(SearchToolEvent::OpenMatch {
                    path: file.path.clone(),
                    row: line.row,
                    range: line.ranges.first().cloned().unwrap_or(0..0),
                    split,
                });
            }
            Some(Row::File(f)) => {
                let path = self.results[f].path.clone();
                if !self.collapsed.remove(&path) {
                    self.collapsed.insert(path);
                }
                cx.notify();
            }
            None => {}
        }
    }

    fn rel(&self, path: &Path) -> (String, Option<String>) {
        let rel = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        match rel.rsplit_once('/') {
            Some((dir, name)) => (name.to_string(), Some(dir.to_string())),
            None => (rel, None),
        }
    }

    fn render_file_row(
        &self,
        f: usize,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let file = &self.results[f];
        let (name, dir) = self.rel(&file.path);
        let collapsed = self.collapsed.contains(&file.path);
        let selected = self.selected == Some(Row::File(f));
        let count: SharedString = file.count().to_string().into();
        let entity = cx.entity();
        let path = file.path.clone();
        div()
            .id(ElementId::NamedInteger("search-file".into(), f as u64))
            .h(px(m.search_row_height))
            .pl(px(m.tree_pad_left))
            .pr(px(m.panel_pad_x))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(m.tree_icon_gap))
            .rounded(px(m.tree_row_radius))
            .cursor_pointer()
            .whitespace_nowrap()
            .overflow_hidden()
            .when(selected, |d| d.bg(theme.list_bg_selected))
            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.selected = Some(Row::File(f));
                    if !this.collapsed.remove(&path) {
                        this.collapsed.insert(path.clone());
                    }
                    cx.notify();
                })
            })
            .child(icon(
                if collapsed {
                    "expui/general/chevronRight"
                } else {
                    "expui/general/chevronDown"
                },
                m.icon_size,
                theme.icon_secondary,
            ))
            .child(icon(file_icon(&name), m.icon_size, theme.icon_secondary))
            .child(div().text_color(theme.list_text).child(name))
            .children(dir.map(|d| {
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.list_text_secondary)
                    .child(d)
            }))
            .child(div().flex_1())
            .child(div().text_color(theme.list_text_secondary).child(count))
    }

    fn render_line_row(
        &self,
        f: usize,
        l: usize,
        theme: &Theme,
        m: &Metrics,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let line = &self.results[f].lines[l];
        let selected = self.selected == Some(Row::Line(f, l));
        let entity = cx.entity();
        // The line without its indent; the match ranges shift with it.
        let trimmed_start = line.text.len() - line.text.trim_start().len();
        let text: String = line.text[trimmed_start..].chars().take(300).collect();
        let text_len = text.len();
        let highlights: Vec<(Range<usize>, HighlightStyle)> =
            line.ranges
                .iter()
                .filter_map(|r| {
                    let start = r.start.checked_sub(trimmed_start)?;
                    let end = r.end.checked_sub(trimmed_start)?.min(text_len);
                    (start < end && text.is_char_boundary(start) && text.is_char_boundary(end))
                        .then(|| {
                            (
                                start..end,
                                HighlightStyle {
                                    color: Some(theme.search_match_text),
                                    background_color: Some(theme.search_match_bg),
                                    ..Default::default()
                                },
                            )
                        })
                })
                .collect();
        let _ = window;
        let styled = StyledText::new(text).with_highlights(highlights);
        let number: SharedString = format!("{}", line.row + 1).into();
        div()
            .id(ElementId::NamedInteger(
                format!("search-line-{f}").into(),
                l as u64,
            ))
            .h(px(m.search_row_height))
            .pl(px(m.tree_pad_left + m.search_indent))
            .pr(px(m.panel_pad_x))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(m.tree_icon_gap))
            .rounded(px(m.tree_row_radius))
            .cursor_pointer()
            .whitespace_nowrap()
            .overflow_hidden()
            .when(selected, |d| d.bg(theme.list_bg_selected))
            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
            .on_click(move |event, _, cx| {
                let shift = event.modifiers().shift;
                entity.update(cx, |this, cx| {
                    this.selected = Some(Row::Line(f, l));
                    this.open_selected(shift, cx);
                })
            })
            .child(
                div()
                    .w(px(m.gutter_number_right - m.tree_pad_left))
                    .flex_shrink_0()
                    .text_color(theme.line_number)
                    .font_family(theme.mono_font.clone())
                    .child(number),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(theme.list_text)
                    .font_family(theme.mono_font.clone())
                    .child(styled),
            )
    }
}

/// A 20 px toggle button, as the find bar's.
pub fn toggle_button(
    id: &'static str,
    label: &'static str,
    on: bool,
    theme: &Theme,
    m: &Metrics,
) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(m.find_bar_toggle))
        .h(px(m.find_bar_toggle))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(m.button_radius))
        .cursor_pointer()
        .border_1()
        .map(|d| {
            if on {
                d.bg(theme.toggle_on_bg)
                    .border_color(theme.toggle_on_border)
                    .text_color(theme.toggle_on_text)
                    .hover(|s| s.bg(theme.toggle_on_bg_hovered))
            } else {
                d.bg(theme.toggle_off_bg)
                    .border_color(gpui::transparent_black())
                    .text_color(theme.toggle_off_text)
                    .hover(|s| {
                        s.bg(theme.toggle_off_bg_hovered)
                            .text_color(theme.toggle_off_text_hovered)
                    })
                    .active(|s| s.bg(theme.toggle_off_bg_pressed))
            }
        })
        .child(label)
}

/// A field's frame with the focus ring (batch 5's find field).
pub fn frame(focused: bool, theme: &Theme, m: &Metrics) -> Div {
    div()
        .h(px(m.search_field_height))
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .pl(px(m.find_bar_pad_left))
        .pr(px(m.find_bar_pad_right))
        .rounded(px(m.input_radius))
        .bg(theme.input_bg)
        .map(|d| {
            if focused {
                d.border(px(m.input_focus_border))
                    .border_color(theme.input_focus_border)
                    .shadow(vec![BoxShadow {
                        color: theme.input_focus_outline,
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(m.focus_outline),
                    }])
            } else {
                d.border(px(1.)).border_color(theme.input_border)
            }
        })
}

impl Render for SearchTool {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let entity = cx.entity();
        let query_focused = self.query.read(cx).is_focused(window);
        let replace_focused = self.replace.read(cx).is_focused(window);
        let mask_focused = self.mask.read(cx).is_focused(window);
        let toggle = |id, label, on, which| {
            let entity = entity.clone();
            toggle_button(id, label, on, &theme, &m)
                .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.toggle(which, cx)))
        };
        let chevron = {
            let entity = entity.clone();
            div()
                .id("search-toggle-replace")
                .size(px(m.icon_size))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3. * m.zoom))
                .cursor_pointer()
                .hover(|s| s.bg(theme.ghost_hovered))
                .tooltip(Tooltip::text("Replace"))
                .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.toggle_replace(cx)))
                .child(icon(
                    if self.replace_mode {
                        "expui/general/chevronDown"
                    } else {
                        "expui/general/chevronRight"
                    },
                    m.icon_size,
                    theme.icon_primary,
                ))
        };
        let counter: SharedString = if let Some(err) = &self.error {
            err.clone().into()
        } else if self.searching {
            "Searching…".into()
        } else if self.query_text(cx).is_empty() {
            "".into()
        } else {
            let total = self.total_matches();
            let files = self.results.len();
            match (total, self.truncated) {
                (0, _) => "No matches".into(),
                (_, true) => format!("First {RESULT_LIMIT} matches in {files} files").into(),
                (n, false) => format!(
                    "{n} match{} in {files} file{}",
                    if n == 1 { "" } else { "es" },
                    if files == 1 { "" } else { "s" }
                )
                .into(),
            }
        };
        let counter_color = if self.error.is_some() {
            theme.text_dangerous
        } else {
            theme.text_secondary
        };
        let query_row = div()
            .flex()
            .items_center()
            .gap(px(m.find_bar_chevron_gap))
            .child(chevron)
            .child(
                frame(query_focused, &theme, &m)
                    .child(div().flex_1().min_w_0().h_full().child(self.query.clone()))
                    .child(div().w(px(m.find_bar_separator_gap_left)).flex_shrink_0())
                    .child(toggle(
                        "search-case",
                        "Aa",
                        self.case_sensitive,
                        Toggle::Case,
                    ))
                    .child(div().w(px(m.find_bar_toggle_gap)).flex_shrink_0())
                    .child(toggle("search-word", "W", self.whole_word, Toggle::Word))
                    .child(div().w(px(m.find_bar_toggle_gap)).flex_shrink_0())
                    .child(toggle("search-regex", ".*", self.regex, Toggle::Regex)),
            );
        let replace_row = self.replace_mode.then(|| {
            let all = entity.clone();
            div()
                .flex()
                .items_center()
                .gap(px(m.find_bar_chevron_gap))
                .child(div().size(px(m.icon_size)).flex_shrink_0())
                .child(
                    frame(replace_focused, &theme, &m)
                        .pr(px(m.find_bar_pad_right - m.text_button_pad_x / 2.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .child(self.replace.clone()),
                        )
                        .child(
                            div()
                                .id("search-replace-all")
                                .h(px(m.button_height))
                                .px(px(m.text_button_pad_x))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .rounded(px(m.button_radius))
                                .cursor_pointer()
                                .text_color(theme.button_secondary_text)
                                .hover(|s| s.bg(theme.ghost_hovered))
                                .active(|s| s.bg(theme.ghost_pressed))
                                .on_click(move |_, _, cx| {
                                    all.update(cx, |this, cx| this.replace_all(cx))
                                })
                                .child("Replace All"),
                        ),
                )
        });
        let mask_row = div()
            .flex()
            .items_center()
            .gap(px(m.find_bar_chevron_gap))
            .child(div().size(px(m.icon_size)).flex_shrink_0())
            .child(
                frame(mask_focused, &theme, &m)
                    .child(div().flex_1().min_w_0().h_full().child(self.mask.clone())),
            );
        let rows: Vec<_> = self
            .rows()
            .into_iter()
            .map(|row| match row {
                Row::File(f) => self.render_file_row(f, &theme, &m, cx),
                Row::Line(f, l) => self.render_line_row(f, l, &theme, &m, window, cx),
            })
            .collect();
        div()
            .key_context("SearchTool")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(m.panel_pad_x))
                    .pt(px(m.search_gap))
                    .flex()
                    .flex_col()
                    .gap(px(m.search_gap))
                    .child(query_row)
                    .children(replace_row)
                    .child(mask_row)
                    .child(
                        div()
                            .h(px(m.notification_line))
                            .pl(px(m.icon_size
                                + m.find_bar_chevron_gap
                                + m.find_bar_pad_left))
                            .flex()
                            .items_center()
                            .text_color(counter_color)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(counter),
                    ),
            )
            .child(
                div()
                    .id("search-results")
                    .flex_1()
                    .min_h_0()
                    .px(px(m.panel_pad_x))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .when(
                self.results.is_empty() && self.error.is_none() && !self.searching,
                |d| {
                    let _ = FontWeight::NORMAL;
                    d
                },
            )
    }
}
