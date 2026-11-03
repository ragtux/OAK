//! The Goto panel: Fleet's one popup for `goto/everything`, `goto/file`,
//! `goto/symbol`, `show-file-symbols`, `goto/action`, `goto/tool`,
//! `find-in-path` and `replace-in-path` (its Text Search tab), the recent
//! files, and `show-recent-workspaces`.
//!
//! Measured in batch 6 and 7 (`docs/fleet-probe.md`, sixth pass): a 680 px
//! popup under the toolbar with a row of tab chips (Goto, Actions, Tools,
//! Text Search) and two icon buttons at the top, the field under them, then
//! per tab: Goto has three filter buttons (Files, Symbols, Symbols in File),
//! a section header ("Recent" for an empty query, "Found" otherwise) and
//! the rows; Actions lists the actions with their shortcuts at the right;
//! Tools shows the "Active" tools, a separator, then every tool under "New";
//! Text Search adds the folder and file-mask fields, a results count and one
//! row per match with `file:line` at the right. A preview of the selected
//! file sits at the bottom of the popup (`toggle-goto-preview`, shown by
//! default as Fleet stores it). Fleet's local storage gives the view state
//! (`GotoPanelViewState`) and the recent rows (`RecentNavigationEntity`).

use crate::actions::{PopupDown, PopupPageDown, PopupPageUp, PopupUp};
use crate::assets::{file_icon, icon};
use crate::dialog::{button, ButtonKind};
use crate::document::Document;
use crate::editor::EditorView;
use crate::input::{InputEvent, TextInput};
use crate::keymap::{action_title, hint_for, is_plain_key, ACTION_IDS};
use crate::search_tool::{frame, toggle_button, Overrides};
use crate::theme::{metrics::GOTO_PREVIEW_SHARE, Metrics, Theme};
use crate::tooltip::Tooltip;
use crate::workspace::Tool;
use gpui::{
    deferred, div, point, prelude::*, px, App, BoxShadow, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, HighlightStyle, Pixels, ScrollHandle, SharedString,
    StyledText, Task, Window,
};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use weep_search::{fuzzy_match, FileIndex, FileMatches, FuzzyMatch, GrepOptions, GrepQuery};
use weep_syntax::Symbol;

/// Fleet's result limit (its Search tool stores 100).
const MAX_ROWS: usize = 100;
/// How long after the last keystroke a text search starts (estimate).
const DEBOUNCE_MS: u64 = 150;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GotoMode {
    Everything,
    Files,
    Symbols,
    /// `show-file-symbols`: the active file's outline with signatures.
    SymbolsInFile,
    Actions,
    Tools,
    /// `find-in-path` and `replace-in-path`.
    TextSearch,
    /// `show-recent-workspaces`: a popup of its own shape, titled.
    Workspaces,
}

/// The tab chips across the top, in Fleet's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Goto,
    Actions,
    Tools,
    TextSearch,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Goto, Tab::Actions, Tab::Tools, Tab::TextSearch];

    fn label(self) -> &'static str {
        match self {
            Tab::Goto => "Goto",
            Tab::Actions => "Actions",
            Tab::Tools => "Tools",
            Tab::TextSearch => "Text Search",
        }
    }

    fn mode(self) -> GotoMode {
        match self {
            Tab::Goto => GotoMode::Everything,
            Tab::Actions => GotoMode::Actions,
            Tab::Tools => GotoMode::Tools,
            Tab::TextSearch => GotoMode::TextSearch,
        }
    }
}

/// The filter buttons under the field on the Goto tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    Files,
    Symbols,
    SymbolsInFile,
}

impl Filter {
    const ALL: [Filter; 3] = [Filter::Files, Filter::Symbols, Filter::SymbolsInFile];

    fn label(self) -> &'static str {
        match self {
            Filter::Files => "Files",
            Filter::Symbols => "Symbols",
            Filter::SymbolsInFile => "Symbols in File",
        }
    }

    fn mode(self) -> GotoMode {
        match self {
            Filter::Files => GotoMode::Files,
            Filter::Symbols => GotoMode::Symbols,
            Filter::SymbolsInFile => GotoMode::SymbolsInFile,
        }
    }
}

impl GotoMode {
    fn tab(self) -> Tab {
        match self {
            GotoMode::Everything
            | GotoMode::Files
            | GotoMode::Symbols
            | GotoMode::SymbolsInFile
            | GotoMode::Workspaces => Tab::Goto,
            GotoMode::Actions => Tab::Actions,
            GotoMode::Tools => Tab::Tools,
            GotoMode::TextSearch => Tab::TextSearch,
        }
    }

    fn filter(self) -> Option<Filter> {
        match self {
            GotoMode::Files => Some(Filter::Files),
            GotoMode::Symbols => Some(Filter::Symbols),
            GotoMode::SymbolsInFile => Some(Filter::SymbolsInFile),
            _ => None,
        }
    }

    /// The field's hint, Fleet's wording where the shots show it.
    fn placeholder(self) -> &'static str {
        match self {
            GotoMode::Everything => "Search files, classes, functions, and symbols",
            GotoMode::Files => "Search files",
            GotoMode::Symbols => "Search symbols",
            GotoMode::SymbolsInFile => "Search symbols in the current file",
            GotoMode::Actions => "Search actions",
            GotoMode::Tools => "Search tools",
            GotoMode::TextSearch => "Search in files",
            GotoMode::Workspaces => "Search workspaces",
        }
    }

    /// Whether the popup keeps its full height (a preview or an outline).
    fn full_height(self) -> bool {
        matches!(
            self,
            GotoMode::Everything
                | GotoMode::Files
                | GotoMode::Symbols
                | GotoMode::SymbolsInFile
                | GotoMode::TextSearch
        )
    }

    fn has_preview(self) -> bool {
        matches!(
            self,
            GotoMode::Everything | GotoMode::Files | GotoMode::Symbols | GotoMode::TextSearch
        )
    }
}

#[derive(Clone, Debug)]
pub enum GotoTarget {
    File(PathBuf),
    /// A symbol: the file and the byte range of its name.
    Symbol {
        path: PathBuf,
        range: Range<usize>,
    },
    /// A Fleet action id.
    Action(String),
    Tool(Tool),
    Workspace(PathBuf),
    /// A text match: the file, its 0-based row, the byte range in the line.
    TextMatch {
        path: PathBuf,
        row: usize,
        range: Range<usize>,
    },
}

/// One row of the list.
#[derive(Clone)]
enum Row {
    Header(SharedString),
    Separator,
    Item(GotoItem),
}

#[derive(Clone)]
pub struct GotoItem {
    pub title: SharedString,
    /// Shown after the title in `listItem.text.secondary` (a file's folder).
    pub detail: Option<SharedString>,
    /// Shown right-aligned in `text.secondary` (a symbol's file, an action's
    /// shortcut, a match's `file:line`, a workspace's path).
    pub right: Option<SharedString>,
    pub icon: Option<&'static str>,
    pub target: GotoTarget,
    score: i32,
    /// Highlighted byte ranges in `title`.
    highlights: Vec<Range<usize>>,
    /// Outline depth and whether the row holds members (Symbols in File).
    depth: usize,
    container: bool,
}

/// What the workspace hands the panel to search.
pub struct GotoSources {
    pub root: PathBuf,
    pub files: Arc<FileIndex>,
    /// Files opened before, most recent first.
    pub recent: Vec<PathBuf>,
    /// The open documents' symbols, the active file first.
    pub symbols: Vec<(PathBuf, Vec<Symbol>)>,
    pub tools: Vec<Tool>,
    /// The tools in a panel right now, in panel order (the Tools tab's "Active").
    pub active_tools: Vec<Tool>,
    pub workspaces: Vec<PathBuf>,
    pub preview_hidden: bool,
    /// `goto_panel.showShortcuts`: the Actions tab's shortcut column.
    pub show_shortcuts: bool,
    /// `ff.previewAndPrioritiseFileSymbolsInGotoEverywhere`.
    pub symbols_first: bool,
    /// The open documents' unsaved text, for the Text Search tab.
    pub overrides: Overrides,
    pub exclude: Vec<String>,
    pub search_big_files: bool,
}

#[derive(Clone, Debug)]
pub enum GotoEvent {
    /// Enter (or a click): open the target; `split` for Shift+Enter.
    Open {
        target: GotoTarget,
        split: bool,
    },
    Cancel,
    /// `toggle-goto-preview` flipped the preview; the workspace remembers it.
    PreviewToggled(bool),
    /// Text Search: replace every match of the query in the listed files.
    ReplaceAll {
        query: GrepQuery,
        replacement: String,
        files: Vec<PathBuf>,
    },
    /// Text Search: replace the selected match alone.
    ReplaceOne {
        path: PathBuf,
        row: usize,
        range: Range<usize>,
        replacement: String,
    },
}

/// Gives the panel a document to preview: an open one, or one read for the purpose.
pub type OpenDocument = Rc<dyn Fn(&Path, &mut App) -> Option<Entity<Document>>>;

/// The Text Search tab's state.
struct TextSearch {
    replace: Entity<TextInput>,
    folder: Entity<TextInput>,
    mask: Entity<TextInput>,
    replace_mode: bool,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    results: Vec<FileMatches>,
    error: Option<String>,
    searching: bool,
    generation: u64,
    pending: Option<Task<()>>,
}

pub struct GotoPanel {
    mode: GotoMode,
    input: Entity<TextInput>,
    sources: GotoSources,
    rows: Vec<Row>,
    selected: usize,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    /// The selected file shown read-only under the list.
    preview: Option<(PathBuf, Entity<EditorView>)>,
    open_document: OpenDocument,
    search: TextSearch,
}

impl EventEmitter<GotoEvent> for GotoPanel {}

impl Focusable for GotoPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// `src/main.rs` → ("main.rs", Some("src")).
fn split_rel(rel: &str) -> (String, Option<String>) {
    match rel.rsplit_once('/') {
        Some((dir, name)) => (name.to_string(), Some(dir.to_string())),
        None => (rel.to_string(), None),
    }
}

/// Char positions of a match turned into byte ranges of the title.
fn byte_ranges(title: &str, positions: &[usize]) -> Vec<Range<usize>> {
    let offsets: Vec<usize> = title
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(title.len()))
        .collect();
    positions
        .iter()
        .filter(|p| **p + 1 < offsets.len())
        .map(|&p| offsets[p]..offsets[p + 1])
        .collect()
}

impl GotoPanel {
    pub fn new(
        mode: GotoMode,
        sources: GotoSources,
        initial: &str,
        open_document: OpenDocument,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new(mode.placeholder(), cx));
        if !initial.is_empty() {
            input.update(cx, |i, cx| {
                i.set_text(initial, cx);
                i.select_all(cx);
            });
        }
        cx.subscribe(&input, Self::on_input).detach();
        let replace = cx.new(|cx| TextInput::new("Replace", cx));
        let folder = cx.new(|cx| TextInput::new("In folder", cx));
        let mask = cx.new(|cx| TextInput::new("File mask: *.*", cx));
        cx.subscribe(&replace, Self::on_replace_input).detach();
        cx.subscribe(&folder, Self::on_scope_input).detach();
        cx.subscribe(&mask, Self::on_scope_input).detach();
        let mut panel = GotoPanel {
            mode,
            input,
            sources,
            rows: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            preview: None,
            open_document,
            search: TextSearch {
                replace,
                folder,
                mask,
                replace_mode: false,
                case_sensitive: false,
                whole_word: false,
                regex: false,
                results: Vec::new(),
                error: None,
                searching: false,
                generation: 0,
                pending: None,
            },
        };
        panel.refilter(cx);
        panel
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    /// Switches the list (a tab or filter click, Tab, or another `goto/*`
    /// chord while open).
    pub fn set_mode(&mut self, mode: GotoMode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            self.input
                .update(cx, |i, cx| i.set_placeholder(mode.placeholder(), cx));
            self.refilter(cx);
        }
    }

    /// `replace-in-path`: the Text Search tab with its Replace field.
    pub fn set_replace_mode(&mut self, on: bool, cx: &mut Context<Self>) {
        self.search.replace_mode = on;
        cx.notify();
    }

    pub fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        self.sources.preview_hidden = !self.sources.preview_hidden;
        self.update_preview(cx);
        cx.emit(GotoEvent::PreviewToggled(self.sources.preview_hidden));
        cx.notify();
    }

    fn on_input(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => self.refilter(cx),
            InputEvent::Submit { shift } => {
                if self.mode == GotoMode::TextSearch && self.rows.is_empty() {
                    self.search_now(cx);
                } else {
                    self.open_selected(*shift, cx);
                }
            }
            InputEvent::Cancel => cx.emit(GotoEvent::Cancel),
            InputEvent::Tab { shift } => {
                if self.mode == GotoMode::Workspaces {
                    return;
                }
                let tabs = Tab::ALL;
                let ix = tabs.iter().position(|t| *t == self.mode.tab()).unwrap_or(0);
                let next = if *shift {
                    (ix + tabs.len() - 1) % tabs.len()
                } else {
                    (ix + 1) % tabs.len()
                };
                self.set_mode(tabs[next].mode(), cx);
            }
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
            InputEvent::Cancel => cx.emit(GotoEvent::Cancel),
            InputEvent::Tab { .. } => self.focus_later(self.input.clone(), cx),
            InputEvent::Changed => {}
        }
    }

    fn on_scope_input(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => self.schedule_search(cx),
            InputEvent::Submit { .. } => self.search_now(cx),
            InputEvent::Cancel => cx.emit(GotoEvent::Cancel),
            InputEvent::Tab { .. } => self.focus_later(self.input.clone(), cx),
        }
    }

    fn focus_later(&self, input: Entity<TextInput>, cx: &mut Context<Self>) {
        cx.defer(move |cx| {
            if let Some(window) = cx.windows().first().copied() {
                let _ = window.update(cx, |_, window, cx| {
                    input.read(cx).focus_handle(cx).focus(window, cx);
                });
            }
        });
    }

    fn open_selected(&mut self, split: bool, cx: &mut Context<Self>) {
        if let Some(Row::Item(item)) = self.rows.get(self.selected) {
            cx.emit(GotoEvent::Open {
                target: item.target.clone(),
                split,
            });
        }
    }

    fn item_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, Row::Item(_)))
            .map(|(ix, _)| ix)
            .collect()
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !matches!(self.rows.get(ix), Some(Row::Item(_))) {
            return;
        }
        self.selected = ix;
        self.scroll.scroll_to_item(ix);
        self.update_preview(cx);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.item_indices();
        if items.is_empty() {
            return;
        }
        let pos = items
            .iter()
            .position(|&i| i == self.selected)
            .map(|p| p as isize)
            .unwrap_or(0);
        let next = (pos + delta).clamp(0, items.len() as isize - 1) as usize;
        self.select(items[next], cx);
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }
    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }
    fn page_up(&mut self, _: &PopupPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-10, cx);
    }
    fn page_down(&mut self, _: &PopupPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(10, cx);
    }

    // ---- the lists ---------------------------------------------------------------

    fn item(title: String, m: &FuzzyMatch, target: GotoTarget) -> GotoItem {
        let highlights = byte_ranges(&title, &m.positions);
        GotoItem {
            title: title.into(),
            detail: None,
            right: None,
            icon: None,
            target,
            score: m.score,
            highlights,
            depth: 0,
            container: false,
        }
    }

    fn file_item(&self, rel: &str, m: FuzzyMatch) -> GotoItem {
        let (name, dir) = split_rel(rel);
        let name_start = rel.chars().count() - name.chars().count();
        let positions: Vec<usize> = m
            .positions
            .iter()
            .filter(|p| **p >= name_start)
            .map(|p| p - name_start)
            .collect();
        let mut item = Self::item(
            name.clone(),
            &FuzzyMatch {
                score: m.score,
                positions,
            },
            GotoTarget::File(self.sources.files.absolute(rel)),
        );
        item.icon = Some(file_icon(&name));
        item.detail = dir.map(Into::into);
        item
    }

    fn recent_items(&self, query: &str) -> Vec<GotoItem> {
        let root = &self.sources.root;
        self.sources
            .recent
            .iter()
            .filter(|p| p.exists())
            .filter_map(|path| {
                let name = file_name(path);
                let m = fuzzy_match(query, &name)?;
                let rel = path
                    .strip_prefix(root)
                    .ok()
                    .and_then(|r| r.parent())
                    .map(|d| d.to_string_lossy().replace('\\', "/"))
                    .filter(|d| !d.is_empty());
                let mut item = Self::item(name.clone(), &m, GotoTarget::File(path.clone()));
                item.icon = Some(file_icon(&name));
                item.detail = rel.map(Into::into);
                // Recent files lead the empty list; with a query they rank with the rest.
                item.score += 10;
                Some(item)
            })
            .collect()
    }

    fn file_items(&self, query: &str, limit: usize) -> Vec<GotoItem> {
        self.sources
            .files
            .matches(query, limit)
            .into_iter()
            .map(|(rel, m)| self.file_item(rel, m))
            .collect()
    }

    fn symbol_items(&self, query: &str) -> Vec<GotoItem> {
        let mut out = Vec::new();
        for (file_ix, (path, symbols)) in self.sources.symbols.iter().enumerate() {
            let file = file_name(path);
            for s in symbols {
                let Some(m) = fuzzy_match(query, &s.name) else {
                    continue;
                };
                let mut item = Self::item(
                    s.name.clone(),
                    &m,
                    GotoTarget::Symbol {
                        path: path.clone(),
                        range: s.range.clone(),
                    },
                );
                item.icon = Some(s.kind.icon());
                item.right = Some(file.clone().into());
                // The active file's symbols come first on a tie.
                item.score -= file_ix as i32;
                out.push(item);
            }
        }
        out
    }

    /// The active file's outline: every symbol in source order with its
    /// signature, indented by depth; a query filters it.
    fn outline_items(&self, query: &str) -> Vec<GotoItem> {
        let Some((path, symbols)) = self.sources.symbols.first() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (ix, s) in symbols.iter().enumerate() {
            let Some(m) = fuzzy_match(query, &s.signature) else {
                continue;
            };
            let has_members = symbols.get(ix + 1).is_some_and(|n| n.depth > s.depth);
            let mut item = Self::item(
                s.signature.clone(),
                &m,
                GotoTarget::Symbol {
                    path: path.clone(),
                    range: s.range.clone(),
                },
            );
            item.icon = Some(s.kind.icon());
            item.depth = s.depth;
            item.container = has_members || s.kind.is_container();
            // Source order, not score order.
            item.score = -(ix as i32);
            out.push(item);
        }
        out
    }

    fn action_items(&self, query: &str, cx: &App) -> Vec<GotoItem> {
        ACTION_IDS
            .iter()
            .filter(|id| !is_plain_key(id))
            .filter_map(|id| {
                let title = action_title(id);
                let m = fuzzy_match(query, &title)?;
                let mut item = Self::item(title, &m, GotoTarget::Action(id.to_string()));
                if self.sources.show_shortcuts {
                    item.right = hint_for(cx, id).map(Into::into);
                }
                Some(item)
            })
            .collect()
    }

    fn tool_item(tool: Tool, m: &FuzzyMatch) -> GotoItem {
        let mut item = Self::item(tool.label().to_string(), m, GotoTarget::Tool(tool));
        item.icon = Some(tool.icon());
        item
    }

    fn workspace_items(&self, query: &str) -> Vec<GotoItem> {
        self.sources
            .workspaces
            .iter()
            .filter_map(|path| {
                let name = file_name(path);
                let m = fuzzy_match(query, &name)?;
                let mut item = Self::item(name, &m, GotoTarget::Workspace(path.clone()));
                item.right = Some(path.display().to_string().into());
                Some(item)
            })
            .collect()
    }

    fn sorted(mut items: Vec<GotoItem>) -> Vec<GotoItem> {
        items.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
        items.truncate(MAX_ROWS);
        items
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let q = query.trim();
        let mut rows: Vec<Row> = Vec::new();
        let push_items = |rows: &mut Vec<Row>, items: Vec<GotoItem>| {
            rows.extend(items.into_iter().map(Row::Item));
        };
        match self.mode {
            GotoMode::Everything | GotoMode::Files | GotoMode::Symbols => {
                let (files, symbols) = match self.mode {
                    GotoMode::Files => (true, false),
                    GotoMode::Symbols => (false, true),
                    _ => (true, true),
                };
                if q.is_empty() {
                    let mut items = Vec::new();
                    if symbols && self.sources.symbols_first {
                        if let Some((path, syms)) = self.sources.symbols.first() {
                            items.extend(syms.iter().map(|s| {
                                let mut item = Self::item(
                                    s.name.clone(),
                                    &FuzzyMatch {
                                        score: 0,
                                        positions: Vec::new(),
                                    },
                                    GotoTarget::Symbol {
                                        path: path.clone(),
                                        range: s.range.clone(),
                                    },
                                );
                                item.icon = Some(s.kind.icon());
                                item.right = Some(file_name(path).into());
                                item
                            }));
                        }
                    }
                    if files {
                        items.extend(self.recent_items(q));
                    }
                    if !items.is_empty() {
                        rows.push(Row::Header("Recent".into()));
                        push_items(&mut rows, items);
                    }
                } else {
                    let mut items = Vec::new();
                    if files {
                        items.extend(self.file_items(q, MAX_ROWS));
                    }
                    if symbols {
                        items.extend(self.symbol_items(q));
                    }
                    let items = Self::sorted(items);
                    if !items.is_empty() {
                        rows.push(Row::Header("Found".into()));
                        push_items(&mut rows, items);
                    }
                }
            }
            GotoMode::SymbolsInFile => {
                let mut items = self.outline_items(q);
                if !q.is_empty() {
                    items = Self::sorted(items);
                }
                push_items(&mut rows, items);
            }
            GotoMode::Actions => push_items(&mut rows, Self::sorted(self.action_items(q, cx))),
            GotoMode::Tools => {
                let active: Vec<GotoItem> = self
                    .sources
                    .active_tools
                    .iter()
                    .filter_map(|t| fuzzy_match(q, t.label()).map(|m| Self::tool_item(*t, &m)))
                    .collect();
                let all: Vec<GotoItem> = self
                    .sources
                    .tools
                    .iter()
                    .filter_map(|t| fuzzy_match(q, t.label()).map(|m| Self::tool_item(*t, &m)))
                    .collect();
                if !active.is_empty() {
                    rows.push(Row::Header("Active".into()));
                    push_items(&mut rows, active);
                    rows.push(Row::Separator);
                }
                if !all.is_empty() {
                    rows.push(Row::Header("New".into()));
                    push_items(&mut rows, all);
                }
            }
            GotoMode::Workspaces => push_items(&mut rows, Self::sorted(self.workspace_items(q))),
            GotoMode::TextSearch => {
                self.rows.clear();
                self.selected = 0;
                self.schedule_search(cx);
                cx.notify();
                return;
            }
        }
        self.rows = rows;
        self.selected = self.item_indices().first().copied().unwrap_or(0);
        self.scroll.scroll_to_item(0);
        self.update_preview(cx);
        cx.notify();
    }

    // ---- text search ------------------------------------------------------------------

    fn grep_query(&self, cx: &App) -> Result<GrepQuery, String> {
        let s = &self.search;
        GrepQuery::new(
            &self.query(cx),
            GrepOptions {
                case_sensitive: s.case_sensitive,
                whole_word: s.whole_word,
                regex: s.regex,
                mask: s.mask.read(cx).text().to_string(),
                path: s.folder.read(cx).text().to_string(),
                exclude: self.sources.exclude.clone(),
                search_big_files: self.sources.search_big_files,
                limit: MAX_ROWS,
            },
        )
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        if self.mode != GotoMode::TextSearch {
            return;
        }
        self.search.generation += 1;
        let generation = self.search.generation;
        self.search.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.search.generation == generation {
                    this.search_now(cx);
                }
            });
        }));
    }

    fn search_now(&mut self, cx: &mut Context<Self>) {
        let query = match self.grep_query(cx) {
            Ok(q) => q,
            Err(message) => {
                self.search.results.clear();
                self.search.error = (!message.is_empty()).then_some(message);
                self.search.searching = false;
                self.rows.clear();
                self.preview = None;
                cx.notify();
                return;
            }
        };
        self.search.error = None;
        self.search.searching = true;
        let root = self.sources.root.clone();
        let overrides = (self.sources.overrides)(cx);
        let generation = self.search.generation;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { query.search(&root, &overrides) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.search.generation != generation {
                    return;
                }
                this.search.results = found;
                this.search.searching = false;
                this.rows = this.match_rows();
                this.selected = 0;
                this.scroll.scroll_to_item(0);
                this.update_preview(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// One row per match, as Fleet lists them: the line without its indent,
    /// the match highlighted, `file:line` at the right.
    fn match_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for file in &self.search.results {
            let name = file_name(&file.path);
            for line in &file.lines {
                let trimmed_start = line.text.len() - line.text.trim_start().len();
                let text: String = line.text[trimmed_start..].chars().take(300).collect();
                for r in &line.ranges {
                    let Some(start) = r.start.checked_sub(trimmed_start) else {
                        continue;
                    };
                    let end = r.end.saturating_sub(trimmed_start).min(text.len());
                    let mut highlights = Vec::new();
                    if start < end && text.is_char_boundary(start) && text.is_char_boundary(end) {
                        highlights.push(start..end);
                    }
                    rows.push(Row::Item(GotoItem {
                        title: text.clone().into(),
                        detail: None,
                        right: Some(format!("{name}:{}", line.row + 1).into()),
                        icon: None,
                        target: GotoTarget::TextMatch {
                            path: file.path.clone(),
                            row: line.row,
                            range: r.clone(),
                        },
                        score: 0,
                        highlights,
                        depth: 0,
                        container: false,
                    }));
                }
            }
        }
        rows
    }

    fn toggle_search_option(&mut self, which: usize, cx: &mut Context<Self>) {
        match which {
            0 => self.search.case_sensitive = !self.search.case_sensitive,
            1 => self.search.whole_word = !self.search.whole_word,
            2 => self.search.regex = !self.search.regex,
            _ => {
                self.search.replace_mode = !self.search.replace_mode;
                cx.notify();
                return;
            }
        }
        self.search_now(cx);
    }

    fn replace_all(&mut self, cx: &mut Context<Self>) {
        let Ok(query) = self.grep_query(cx) else {
            return;
        };
        if self.search.results.is_empty() {
            return;
        }
        cx.emit(GotoEvent::ReplaceAll {
            query,
            replacement: self.search.replace.read(cx).text().to_string(),
            files: self.search.results.iter().map(|f| f.path.clone()).collect(),
        });
    }

    fn replace_one(&mut self, cx: &mut Context<Self>) {
        if let Some(Row::Item(GotoItem {
            target: GotoTarget::TextMatch { path, row, range },
            ..
        })) = self.rows.get(self.selected)
        {
            cx.emit(GotoEvent::ReplaceOne {
                path: path.clone(),
                row: *row,
                range: range.clone(),
                replacement: self.search.replace.read(cx).text().to_string(),
            });
        }
    }

    /// After a replacement the same query runs again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.mode == GotoMode::TextSearch {
            self.schedule_search(cx);
        }
    }

    fn count_text(&self, cx: &App) -> SharedString {
        if let Some(err) = &self.search.error {
            return err.clone().into();
        }
        if self.search.searching {
            return "Searching…".into();
        }
        if self.query(cx).trim().is_empty() {
            return "".into();
        }
        let results: usize = self.search.results.iter().map(FileMatches::count).sum();
        let files = self.search.results.len();
        if results == 0 {
            "No results".into()
        } else {
            format!("{results} results in {files} files").into()
        }
    }

    // ---- the preview ---------------------------------------------------------------

    fn update_preview(&mut self, cx: &mut Context<Self>) {
        if self.sources.preview_hidden || !self.mode.has_preview() {
            self.preview = None;
            return;
        }
        let Some(Row::Item(item)) = self.rows.get(self.selected) else {
            self.preview = None;
            return;
        };
        enum Where {
            Start,
            Bytes(Range<usize>),
            Line(usize, Range<usize>),
        }
        let (path, at) = match &item.target {
            GotoTarget::File(p) => (p.clone(), Where::Start),
            GotoTarget::Symbol { path, range } => (path.clone(), Where::Bytes(range.clone())),
            GotoTarget::TextMatch { path, row, range } => {
                (path.clone(), Where::Line(*row, range.clone()))
            }
            _ => {
                self.preview = None;
                return;
            }
        };
        let editor = match &self.preview {
            Some((p, e)) if *p == path => e.clone(),
            _ => {
                let Some(document) = (self.open_document)(&path, cx) else {
                    self.preview = None;
                    return;
                };
                let editor = cx.new(|cx| {
                    let mut e = EditorView::new(document, cx);
                    e.read_only = true;
                    e
                });
                self.preview = Some((path.clone(), editor.clone()));
                editor
            }
        };
        editor.update(cx, |e, cx| match at {
            Where::Start => e.goto(0, 0, cx),
            Where::Bytes(range) => e.select_bytes(range, cx),
            Where::Line(row, range) => {
                let chars = {
                    let b = &e.document.read(cx).buffer;
                    let line_start = b.line_to_char(row);
                    let line = b.line(row);
                    let count =
                        |byte: usize| line.get(..byte).map(|s| s.chars().count()).unwrap_or(0);
                    line_start + count(range.start)..line_start + count(range.end)
                };
                e.select_range(chars, cx);
            }
        });
    }

    /// The preview's breadcrumb: the path from the root, then the symbol.
    fn breadcrumb(&self) -> Vec<SharedString> {
        let Some(Row::Item(item)) = self.rows.get(self.selected) else {
            return Vec::new();
        };
        let (path, tail) = match &item.target {
            GotoTarget::File(p) => (p, None),
            GotoTarget::Symbol { path, .. } => (path, Some(item.title.clone())),
            GotoTarget::TextMatch { path, .. } => (path, None),
            _ => return Vec::new(),
        };
        let rel = path.strip_prefix(&self.sources.root).unwrap_or(path);
        let mut crumbs: Vec<SharedString> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string().into())
            .collect();
        crumbs.extend(tail);
        crumbs
    }

    // ---- rendering -------------------------------------------------------------------

    fn render_row(
        &self,
        ix: usize,
        row: &Row,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        match row {
            Row::Header(text) => div()
                .h(px(m.goto_header_height))
                .px(px(m.popup_row_pad_x))
                .flex_shrink_0()
                .flex()
                .items_center()
                .text_color(theme.text_secondary)
                .child(text.clone())
                .into_any_element(),
            Row::Separator => div()
                .mt(px(m.goto_separator_above - 2.))
                .mb(px(m.goto_separator_below))
                .h(px(1.))
                .flex_shrink_0()
                .bg(theme.popup_separator)
                .into_any_element(),
            Row::Item(item) => {
                let selected = ix == self.selected;
                let entity = cx.entity();
                let text_color = if selected {
                    theme.list_text_focused
                } else {
                    theme.list_text
                };
                let highlights: Vec<(Range<usize>, HighlightStyle)> = item
                    .highlights
                    .iter()
                    .map(|r| {
                        (
                            r.clone(),
                            HighlightStyle {
                                color: Some(theme.search_match_text),
                                background_color: Some(theme.search_match_bg),
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                let styled = StyledText::new(item.title.clone()).with_highlights(highlights);
                let indent = item.depth as f32 * m.goto_tree_indent;
                let outline = self.mode == GotoMode::SymbolsInFile;
                div()
                    .h(px(m.goto_row_pitch))
                    .flex_shrink_0()
                    .child(
                        div()
                            .id(ElementId::NamedInteger("goto-row".into(), ix as u64))
                            .h(px(m.goto_row_height))
                            .pl(px(m.popup_row_pad_x + indent))
                            .pr(px(m.popup_row_pad_x))
                            .flex()
                            .items_center()
                            .gap(px(m.popup_row_icon_gap))
                            .rounded(px(m.popup_row_radius))
                            .cursor_pointer()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_color(text_color)
                            .when(selected, |d| d.bg(theme.list_bg_focused))
                            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                            .on_mouse_move({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        if this.selected != ix {
                                            this.select(ix, cx);
                                        }
                                    })
                                }
                            })
                            .on_click(move |event, _, cx| {
                                let shift = event.modifiers().shift;
                                entity.update(cx, |this, cx| {
                                    this.selected = ix;
                                    this.open_selected(shift, cx);
                                })
                            })
                            .when(outline, |d| {
                                d.child(div().size(px(m.icon_size)).flex_shrink_0().when(
                                    item.container,
                                    |d| {
                                        d.child(icon(
                                            "expui/general/chevronDown",
                                            m.icon_size,
                                            theme.icon_secondary,
                                        ))
                                    },
                                ))
                            })
                            .children(
                                item.icon
                                    .map(|path| icon(path, m.icon_size, theme.icon_secondary)),
                            )
                            .child(div().flex_shrink_0().child(styled))
                            .children(item.detail.clone().map(|d| {
                                div()
                                    .ml(px(m.popup_row_icon_gap))
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.list_text_secondary)
                                    .child(d)
                            }))
                            .child(div().flex_1())
                            .children(item.right.clone().map(|r| {
                                div()
                                    .flex_shrink_0()
                                    .max_w(px(m.goto_width * 0.6))
                                    .truncate()
                                    .text_color(theme.text_secondary)
                                    .child(r)
                            })),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_tabs(&self, theme: &Theme, m: &Metrics, cx: &Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let current = self.mode.tab();
        let preview_hint = hint_for(cx, "toggle-goto-preview");
        let toggle = entity.clone();
        div()
            .pt(px(m.goto_tab_top - 1.))
            .px(px(m.goto_pad - 1.))
            .h(px(m.goto_tab_top - 1. + m.goto_tab_height))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(m.goto_tab_gap))
            .children(Tab::ALL.into_iter().enumerate().map(|(ix, tab)| {
                let selected = tab == current;
                let entity = entity.clone();
                div()
                    .id(ElementId::NamedInteger("goto-tab".into(), ix as u64))
                    .h(px(m.goto_tab_height))
                    .px(px(m.goto_tab_pad_x))
                    .flex()
                    .items_center()
                    .rounded(px(m.panel_tab_radius))
                    .border(px(m.editor_tab_border))
                    .cursor_pointer()
                    .whitespace_nowrap()
                    .map(|d| {
                        if selected {
                            d.bg(theme.tab_bg_selected_focused)
                                .border_color(theme.tab_border_selected_focused)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.tab_text_selected)
                        } else {
                            d.border_color(gpui::transparent_black())
                                .text_color(theme.tab_text)
                                .hover(|s| s.bg(theme.tab_bg_hovered))
                        }
                    })
                    .on_click(move |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.set_mode(tab.mode(), cx);
                            this.focus_handle(cx).focus(window, cx);
                        })
                    })
                    .child(tab.label())
            }))
            .child(div().flex_1())
            .child(
                div()
                    .id("goto-preview-toggle")
                    .size(px(m.goto_icon_button))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(m.button_radius))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.ghost_hovered))
                    .map(|d| match preview_hint {
                        Some(h) => d.tooltip(Tooltip::with_detail("Toggle Preview", h)),
                        None => d.tooltip(Tooltip::text("Toggle Preview")),
                    })
                    .on_click(move |_, _, cx| toggle.update(cx, |this, cx| this.toggle_preview(cx)))
                    .child(icon(
                        "weep/panel-bottom",
                        m.icon_size,
                        if self.sources.preview_hidden {
                            theme.icon_secondary
                        } else {
                            theme.icon_primary
                        },
                    )),
            )
            .child(
                div()
                    .id("goto-open-window")
                    .size(px(m.goto_icon_button))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(m.button_radius))
                    .tooltip(Tooltip::text("Open in Window (not yet)"))
                    .child(icon(
                        "expui/general/layout",
                        m.icon_size,
                        theme.icon_secondary,
                    )),
            )
    }

    fn render_filters(&self, theme: &Theme, m: &Metrics, cx: &Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let current = self.mode.filter();
        div()
            .mt(px(m.goto_chip_top_gap + m.focus_outline))
            .px(px(m.goto_pad - 1.))
            .h(px(m.goto_chip_height))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(m.goto_chip_gap))
            .children(Filter::ALL.into_iter().enumerate().map(|(ix, filter)| {
                let on = current == Some(filter);
                let entity = entity.clone();
                button(
                    ElementId::NamedInteger("goto-filter".into(), ix as u64),
                    filter.label().into(),
                    if on {
                        ButtonKind::Primary
                    } else {
                        ButtonKind::Secondary
                    },
                    false,
                    theme,
                    m,
                )
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        // The chip that is on turns off: back to everything.
                        let mode = if on {
                            GotoMode::Everything
                        } else {
                            filter.mode()
                        };
                        this.set_mode(mode, cx);
                        this.focus_handle(cx).focus(window, cx);
                    })
                })
            }))
    }

    fn render_field(
        &self,
        focused: bool,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let text_search = self.mode == GotoMode::TextSearch;
        let entity = cx.entity();
        let toggle = |id: &'static str, label: &'static str, on: bool, which: usize| {
            let entity = entity.clone();
            toggle_button(id, label, on, theme, m).on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.toggle_search_option(which, cx))
            })
        };
        let replace_toggle = {
            let entity = entity.clone();
            div()
                .id("goto-replace-toggle")
                .size(px(m.find_bar_toggle))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(m.button_radius))
                .cursor_pointer()
                .when(self.search.replace_mode, |d| d.bg(theme.toggle_on_bg))
                .when(!self.search.replace_mode, |d| {
                    d.hover(|s| s.bg(theme.toggle_off_bg_hovered))
                })
                .tooltip(Tooltip::text("Replace"))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.toggle_search_option(3, cx))
                })
                .child(icon(
                    "expui/actions/replace",
                    m.icon_size,
                    if self.search.replace_mode {
                        theme.toggle_on_text
                    } else {
                        theme.icon_primary
                    },
                ))
        };
        div()
            .mt(px(m.goto_tab_field_gap))
            .px(px(m.goto_pad - 1.))
            .flex_shrink_0()
            .flex()
            .child(
                frame(focused, theme, m)
                    .pl(px(m.goto_pad))
                    .child(div().flex_1().min_w_0().h_full().child(self.input.clone()))
                    .when(text_search, |d| {
                        d.child(toggle("goto-case", "Aa", self.search.case_sensitive, 0))
                            .child(div().w(px(m.find_bar_toggle_gap)).flex_shrink_0())
                            .child(toggle("goto-word", "W", self.search.whole_word, 1))
                            .child(div().w(px(m.find_bar_toggle_gap)).flex_shrink_0())
                            .child(toggle("goto-regex", ".*", self.search.regex, 2))
                            .child(div().w(px(m.find_bar_toggle_gap)).flex_shrink_0())
                            .child(replace_toggle)
                    }),
            )
    }

    fn render_search_fields(
        &self,
        window: &Window,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let mut out: Vec<gpui::AnyElement> = Vec::new();
        let s = &self.search;
        if s.replace_mode {
            let focused = s.replace.read(cx).is_focused(window);
            out.push(
                div()
                    .mt(px(m.goto_chip_top_gap + m.focus_outline))
                    .px(px(m.goto_pad - 1.))
                    .flex_shrink_0()
                    .flex()
                    .child(
                        frame(focused, theme, m)
                            .pl(px(m.goto_pad))
                            .child(div().flex_1().min_w_0().h_full().child(s.replace.clone())),
                    )
                    .into_any_element(),
            );
        }
        let folder_focused = s.folder.read(cx).is_focused(window);
        let mask_focused = s.mask.read(cx).is_focused(window);
        let gap = if s.replace_mode {
            m.goto_block_gap
        } else {
            m.goto_chip_top_gap + m.focus_outline
        };
        out.push(
            div()
                .mt(px(gap))
                .px(px(m.goto_pad - 1.))
                .flex_shrink_0()
                .flex()
                .gap(px(m.goto_block_gap))
                .child(
                    // Fleet: the folder field 492 wide beside the 164 px mask (batch 7).
                    frame(folder_focused, theme, m)
                        .flex_none()
                        .w(px(m.goto_width
                            - 2. * m.goto_pad
                            - m.search_mask_width
                            - m.goto_block_gap
                            - 2.))
                        .flex_basis(px(m.goto_width
                            - 2. * m.goto_pad
                            - m.search_mask_width
                            - m.goto_block_gap
                            - 2.))
                        .pl(px(m.goto_pad))
                        .child(div().flex_1().min_w_0().h_full().child(s.folder.clone()))
                        .child(icon(
                            "expui/nodes/folder",
                            m.icon_size,
                            theme.icon_secondary,
                        )),
                )
                .child(
                    frame(mask_focused, theme, m)
                        .flex_none()
                        .w(px(m.search_mask_width))
                        .flex_basis(px(m.search_mask_width))
                        .pl(px(m.goto_pad))
                        .child(div().flex_1().min_w_0().h_full().child(s.mask.clone())),
                )
                .into_any_element(),
        );
        out.push(
            div()
                .h(px(if self.search_idle(cx) {
                    0.
                } else {
                    m.search_count_row
                }))
                .px(px(m.goto_pad - 1. + m.popup_row_pad_x))
                .flex_shrink_0()
                .flex()
                .items_center()
                .text_color(if s.error.is_some() {
                    theme.text_dangerous
                } else {
                    theme.text_secondary
                })
                .whitespace_nowrap()
                .child(self.count_text(cx))
                .into_any_element(),
        );
        out
    }

    /// The vertical space the blocks above the list take, inside the border.
    /// With nothing typed and nothing found, Fleet's Text Search popup is
    /// just its fields (ninth pass): no count row, list or preview.
    fn search_idle(&self, cx: &App) -> bool {
        self.mode == GotoMode::TextSearch
            && self.search.results.is_empty()
            && self.input.read(cx).text().trim().is_empty()
    }

    fn list_top(&self, m: &Metrics, idle: bool) -> f32 {
        let mut y =
            m.goto_tab_top - 1. + m.goto_tab_height + m.goto_tab_field_gap + m.goto_field_height;
        match self.mode {
            GotoMode::Everything
            | GotoMode::Files
            | GotoMode::Symbols
            | GotoMode::SymbolsInFile => {
                y += m.goto_chip_top_gap + m.focus_outline + m.goto_chip_height + m.goto_block_gap;
            }
            GotoMode::TextSearch => {
                if self.search.replace_mode {
                    y += m.goto_chip_top_gap
                        + m.focus_outline
                        + m.goto_field_height
                        + m.goto_block_gap;
                } else {
                    y += m.goto_chip_top_gap + m.focus_outline;
                }
                y += m.goto_field_height + if idle { 0. } else { m.search_count_row };
            }
            GotoMode::Actions | GotoMode::Tools | GotoMode::Workspaces => {
                y += m.goto_list_field_gap + m.focus_outline;
            }
        }
        y
    }

    fn rows_height(&self, m: &Metrics) -> f32 {
        self.rows
            .iter()
            .map(|r| match r {
                Row::Header(_) => m.goto_header_height,
                Row::Separator => m.goto_separator_above - 2. + 1. + m.goto_separator_below,
                Row::Item(_) => m.goto_row_pitch,
            })
            .sum()
    }
}

impl Render for GotoPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let viewport = window.viewport_size();
        let margin = px(m.island_margin);
        let width = px(m.goto_width).min(viewport.width - margin * 2.);
        let left = (viewport.width - width) / 2.;
        let max_height = px(m.goto_max_height).min(viewport.height - px(m.goto_top) - margin);
        let idle = self.search_idle(cx);
        let list_top = self.list_top(&m, idle);
        let rows_height = self.rows_height(&m);
        let full = self.mode.full_height() && !idle;
        let height = if full {
            max_height
        } else {
            let bottom = if idle {
                m.search_idle_bottom_pad
            } else {
                m.goto_bottom_pad
            };
            px(list_top + rows_height + bottom + 2.).min(max_height)
        };
        let show_preview = full && !self.sources.preview_hidden && self.mode.has_preview();
        let replace_buttons = self.mode == GotoMode::TextSearch && self.search.replace_mode;
        // Under the list: the preview box and, in replace mode, the buttons.
        let inner_height = f32::from(height) - 2.;
        let below = if replace_buttons {
            m.button_height + 2. * m.goto_buttons_pad
        } else if show_preview {
            m.goto_preview_bottom_pad
        } else {
            0.
        };
        let avail = (inner_height - list_top - below).max(0.);
        let preview_height = if show_preview {
            (avail - m.goto_block_gap).max(0.) * GOTO_PREVIEW_SHARE
        } else {
            0.
        };
        let list_height = if full {
            (avail - preview_height - if show_preview { m.goto_block_gap } else { 0. }).max(0.)
        } else {
            rows_height
        };
        let field_focused = self.input.read(cx).is_focused(window);
        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| self.render_row(ix, row, &theme, &m, cx))
            .collect();
        let empty = self.rows.is_empty();
        let empty_text: SharedString = match self.mode {
            GotoMode::TextSearch => "".into(),
            _ => "Nothing found".into(),
        };
        let crumbs = self.breadcrumb();
        let preview = self.preview.clone();
        let close = cx.entity();
        let entity = cx.entity();
        deferred(
            div()
                .id("goto-panel")
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::page_up))
                .on_action(cx.listener(Self::page_down))
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |_, cx| cx.emit(GotoEvent::Cancel));
                })
                .occlude()
                .absolute()
                .left(left)
                .top(px(m.goto_top))
                .w(width)
                .h(height)
                .flex()
                .flex_col()
                .rounded(px(m.popup_radius))
                .border_1()
                .border_color(theme.popup_border)
                .bg(theme.popup_goto_bg)
                .font_family(theme.ui_font.clone())
                .text_size(px(m.ui_font_size))
                .text_color(theme.popup_text)
                .overflow_hidden()
                .shadow(vec![BoxShadow {
                    color: theme.shadow_medium,
                    offset: point(px(0.), px(m.popup_shadow_offset_y)),
                    blur_radius: px(m.popup_shadow_blur),
                    spread_radius: px(0.),
                }])
                .map(|d| {
                    if self.mode == GotoMode::Workspaces {
                        d.child(
                            div()
                                .pt(px(m.goto_tab_top - 1.))
                                .px(px(m.goto_pad - 1. + m.goto_tab_pad_x))
                                .h(px(m.goto_tab_top - 1. + m.goto_tab_height))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text_bright)
                                .child("Recent Workspaces"),
                        )
                    } else {
                        d.child(self.render_tabs(&theme, &m, cx))
                    }
                })
                .child(self.render_field(field_focused, &theme, &m, cx))
                .when(
                    self.mode.tab() == Tab::Goto && self.mode != GotoMode::Workspaces,
                    |d| d.child(self.render_filters(&theme, &m, cx)),
                )
                .when(self.mode == GotoMode::TextSearch, |d| {
                    d.children(self.render_search_fields(window, &theme, &m, cx))
                })
                .child(
                    div()
                        .mt(px(match self.mode {
                            GotoMode::TextSearch => 0.,
                            GotoMode::Actions | GotoMode::Tools | GotoMode::Workspaces => {
                                m.goto_list_field_gap + m.focus_outline
                            }
                            _ => m.goto_block_gap,
                        }))
                        .px(px(m.goto_pad - 1.))
                        .h(px(list_height))
                        .flex_shrink_0()
                        .map(|d| {
                            if empty {
                                d.flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(theme.text_secondary)
                                    .child(empty_text)
                            } else {
                                d.child(
                                    div()
                                        .id("goto-list")
                                        .size_full()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.scroll)
                                        .flex()
                                        .flex_col()
                                        .children(rows),
                                )
                            }
                        }),
                )
                .when(show_preview, |d| {
                    d.child(
                        div()
                            .mt(px(m.goto_block_gap))
                            .mx(px(m.goto_pad - 1.))
                            .h(px(preview_height))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .rounded(px(m.goto_preview_radius))
                            .overflow_hidden()
                            .bg(theme.island_background)
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .relative()
                                    .children(preview.map(|(_, editor)| editor))
                                    // The field keeps the keyboard: clicks on the preview go nowhere.
                                    .child(div().absolute().inset_0().occlude()),
                            )
                            .when(!crumbs.is_empty(), |d| {
                                d.child(
                                    div()
                                        .h(px(m.goto_preview_footer))
                                        .px(px(m.goto_pad + m.popup_row_pad_x))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .text_color(theme.text_secondary)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .children(crumbs.into_iter().enumerate().flat_map(
                                            |(i, c)| {
                                                let mut parts = Vec::new();
                                                if i > 0 {
                                                    parts.push(
                                                        div()
                                                            .px(px(7. * m.zoom))
                                                            .text_color(theme.text_tertiary)
                                                            .child("/"),
                                                    );
                                                }
                                                parts.push(div().child(c));
                                                parts
                                            },
                                        )),
                                )
                            }),
                    )
                })
                .when(replace_buttons, |d| {
                    let one = entity.clone();
                    let all = entity.clone();
                    d.child(
                        div()
                            .mt(px(m.goto_buttons_pad))
                            .px(px(m.goto_pad - 1.))
                            .h(px(m.button_height))
                            .flex_shrink_0()
                            .flex()
                            .justify_end()
                            .gap(px(m.goto_button_gap))
                            .child(
                                button(
                                    "goto-replace-one",
                                    "Replace".into(),
                                    ButtonKind::Secondary,
                                    false,
                                    &theme,
                                    &m,
                                )
                                .on_click(move |_, _, cx| {
                                    one.update(cx, |this, cx| this.replace_one(cx))
                                }),
                            )
                            .child(
                                button(
                                    "goto-replace-all",
                                    "Replace All".into(),
                                    ButtonKind::Secondary,
                                    false,
                                    &theme,
                                    &m,
                                )
                                .on_click(move |_, _, cx| {
                                    all.update(cx, |this, cx| this.replace_all(cx))
                                }),
                            ),
                    )
                }),
        )
    }
}

impl GotoPanel {
    /// A pixel value for tests of the layout arithmetic.
    #[allow(dead_code)]
    fn px_of(v: f32) -> Pixels {
        px(v)
    }
}
