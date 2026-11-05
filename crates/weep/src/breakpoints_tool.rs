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
//! (file by file, in line order) and a disabled or unbound breakpoint's dot
//! (the gutter's ring and grey) are estimates. A row's tooltip is the
//! line's text. Without breakpoints the tool is Fleet's empty tool
//! (`empty_state.rs`, twenty-fourth pass): a hollow ring, "Breakpoints",
//! "Click on the line numbers in Editor to toggle breakpoints". A right
//! click on a row opens Fleet's row menu (twenty-fourth pass): "Remove
//! Breakpoint" with its chord and "Disable Breakpoint" (or "Enable"), the
//! gutter menu's rows without the separator and View All.
//!
//! Hidden behind [`SHOW_BREAKPOINT_EXTRAS`] (twenty-fourth pass: Fleet's
//! tool lists only line breakpoints and shows nothing for a selected row,
//! so the user chose to hide these; the store, the protocol and layout.json
//! keep them), estimates no probe has seen in Fleet:
//! - The exception breakpoints, a group of their own after the line
//!   breakpoints: a title row "Exception Breakpoints" with the count, then a
//!   row per filter (expui's `breakpointException` icon in the breakpoint red,
//!   `breakpointExceptionDisabled` when off, the adapter's label, a check
//!   box).
//! - The selected line breakpoint's properties under the list, in the form
//!   of IntelliJ's breakpoint dialog: past the Debug tool's section splitter
//!   a title row with the breakpoint's name, an "Enabled" check box, then
//!   the Condition, Log message and Hit count fields (`input.rs`'s field in
//!   the find field's frame), each applied on Enter or when it loses focus;
//!   Escape puts the stored value back. Fleet has no action id for editing
//!   a breakpoint and its gutter menu has no such row (batch 13), so the
//!   tool is where they are edited.
//!
//! Keys (context `BreakpointsTool`, on the list): arrows move, Space toggles
//! the check box (`debug/toggle-enabled-breakpoint`), Delete or Backspace
//! removes a line breakpoint (`debug/remove-breakpoint`), Enter or a double
//! click opens the file at the line through [`DebugViewEvent::Open`]. The
//! fields sit outside that context (`BreakpointProperties`), so Space and
//! Backspace type there.

use crate::actions::{
    DebugRemoveBreakpoint, DebugToggleEnabledBreakpoint, PopupConfirm, PopupDown, PopupUp,
};
use crate::assets::icon;
use crate::breakpoints::{BreakpointStore, ExceptionBreakpoint, LineBreakpoint, Look, Properties};
use crate::debug_tool::{relative_path, section_title, DebugViewEvent};
use crate::empty_state::EmptyState;
use crate::git_tool::{checkbox, tui_list, CheckState};
use crate::input::{InputEvent, TextInput};
use crate::skin::{self, Cell, FrameBars, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use crate::tooltip::Tooltip;
use gpui::{
    div, prelude::*, px, AnyElement, App, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, MouseButton, MouseDownEvent, Pixels, Point, Render, ScrollHandle,
    SharedString, Subscription, Window,
};
use std::cell::RefCell;
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

/// One selectable row of the tool: the line breakpoints, then the
/// exception breakpoints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Line(BpRow),
    Exception(ExceptionBreakpoint),
}

impl Item {
    fn enabled(&self) -> bool {
        match self {
            Item::Line(r) => r.enabled,
            Item::Exception(e) => e.enabled,
        }
    }
}

/// The tool's rows in order: every line breakpoint, then every exception
/// breakpoint.
pub fn items(lines: Vec<BpRow>, exceptions: &[ExceptionBreakpoint]) -> Vec<Item> {
    lines
        .into_iter()
        .map(Item::Line)
        .chain(exceptions.iter().cloned().map(Item::Exception))
        .collect()
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

/// Whether the tool shows the exception breakpoints' group and the
/// selected breakpoint's properties (condition, log message, hit count).
/// Off: Fleet 1.48's tool has neither (twenty-fourth pass,
/// `rest7-fleet-bp-tool.png`), and the user chose to hide them; the code
/// stays compiled and tested so they can come back.
pub const SHOW_BREAKPOINT_EXTRAS: bool = false;

/// The tool without breakpoints, Fleet's empty tool (twenty-fourth pass,
/// `rest8-fleet-bp-gutter-alt.png`): the hollow ring (the gutter's disabled
/// breakpoint, 18 px of ink at 24 as Fleet's), the title and its hint.
fn no_breakpoints() -> EmptyState {
    EmptyState {
        icon: "weep/breakpoint",
        glyph: "○",
        title: "Breakpoints",
        description: "Click on the line numbers in Editor to toggle breakpoints",
        button: None,
    }
}

/// Estimate: the exception group's title (IntelliJ names its groups so).
const EXCEPTIONS: &str = "Exception Breakpoints";
/// Estimate: the properties' field labels and placeholders (IntelliJ's
/// dialog says "Condition", "Evaluate and log" and "Pass count"; these are
/// DAP's names for what is sent).
const FIELDS: [(&str, &str); 3] = [
    ("Condition", "Stop only when this is true"),
    ("Log message", "Log instead of stopping: {expr}"),
    ("Hit count", "Stop on hit N"),
];
/// The tui labels, in a column of [`TUI_LABEL`] cells.
const TUI_FIELDS: [&str; 3] = ["condition", "log", "hit count"];
const TUI_LABEL: usize = 11;
/// Estimate: the properties' label column and the gap between its rows,
/// in px at zoom 1 (scaled by `Metrics::zoom`).
const PROPS_LABEL: f32 = 104.;
const PROPS_GAP: f32 = 6.;
/// The tui properties: a rule, the Enabled row and three fields.
const TUI_PROPS_ROWS: usize = 5;

/// Which of the three fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Condition,
    Log,
    Hits,
}

const ALL_FIELDS: [Field; 3] = [Field::Condition, Field::Log, Field::Hits];

impl Field {
    fn ix(self) -> usize {
        match self {
            Field::Condition => 0,
            Field::Log => 1,
            Field::Hits => 2,
        }
    }

    /// The field Tab (or Shift+Tab) moves to; `None` past the ends.
    fn next(self, back: bool) -> Option<Field> {
        let ix = self.ix() as isize + if back { -1 } else { 1 };
        ALL_FIELDS.get(usize::try_from(ix).ok()?).copied()
    }
}

/// The properties the fields hold, over the breakpoint's enabled state.
pub fn properties_from_fields(enabled: bool, texts: [&str; 3]) -> Properties {
    Properties {
        enabled,
        condition: Properties::field(texts[0]),
        log_message: Properties::field(texts[1]),
        hit_condition: Properties::field(texts[2]),
    }
}

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
    /// The selected line breakpoint's Condition, Log message and Hit count.
    fields: [Entity<TextInput>; 3],
    /// The breakpoint the fields were loaded from (and are applied to).
    editing: Option<(PathBuf, usize)>,
    /// How many of the fields hold the focus (focus and blur events come
    /// in either order, so a count, not a flag).
    focused_fields: i32,
    /// The fields' events and focus listeners, made at the first render
    /// (they need the window).
    wired: Vec<Subscription>,
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
        let observer = cx.observe(&store, |this, _, cx| {
            this.sync_fields(cx);
            cx.notify()
        });
        let fields = FIELDS.map(|(_, placeholder)| cx.new(|cx| TextInput::new(placeholder, cx)));
        let mut tool = BreakpointsTool {
            root,
            selected: 0,
            line_source: None,
            disk: HashMap::new(),
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            reveal: false,
            fields,
            editing: None,
            focused_fields: 0,
            wired: Vec::new(),
            tui_grid: Default::default(),
            tui_bars: Default::default(),
            tui_scroll: RowScroll::default(),
            _store: observer,
        };
        tool.load_fields(cx);
        tool
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

    fn items(&mut self, cx: &App) -> Vec<Item> {
        let rows = self.rows(cx);
        if !SHOW_BREAKPOINT_EXTRAS {
            return items(rows, &[]);
        }
        let store = BreakpointStore::global(cx);
        items(rows, store.read(cx).exception_breakpoints())
    }

    fn selected_item(&mut self, cx: &App) -> Option<Item> {
        let ix = self.selected;
        self.items(cx).get(ix).cloned()
    }

    /// The selected line breakpoint's file and line.
    fn selected_key(&mut self, cx: &App) -> Option<(PathBuf, usize)> {
        match self.selected_item(cx)? {
            Item::Line(r) => Some((r.path, r.line)),
            Item::Exception(_) => None,
        }
    }

    /// Selects row `ix`, applying the fields to the breakpoint they edited
    /// first and loading the new one's.
    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.apply_fields(cx);
        self.selected = ix;
        self.load_fields(cx);
        cx.notify();
    }

    /// The fields from the selected line breakpoint (empty without one).
    fn load_fields(&mut self, cx: &mut Context<Self>) {
        let key = self.selected_key(cx);
        let store = BreakpointStore::global(cx);
        let b = key
            .as_ref()
            .and_then(|(path, line)| store.read(cx).at(path, *line).cloned());
        let texts = b
            .map(|b| [b.condition, b.log_message, b.hit_condition])
            .unwrap_or_default()
            .map(Option::unwrap_or_default);
        for (field, text) in self.fields.clone().iter().zip(texts) {
            field.update(cx, |f, cx| {
                if f.text() != text {
                    f.set_text_at_end(&text, cx)
                }
            });
        }
        self.editing = key;
    }

    /// The store changed: the fields follow the selection unless one is
    /// being typed in (its breakpoint may only have moved lines).
    fn sync_fields(&mut self, cx: &mut Context<Self>) {
        let n = self.items(cx).len();
        self.selected = self.selected.min(n.saturating_sub(1));
        let key = self.selected_key(cx);
        if key != self.editing {
            if self.focused_fields > 0 && key.is_some() {
                self.editing = key;
            } else {
                self.load_fields(cx);
            }
        }
    }

    /// Writes the fields to the breakpoint they were loaded from (never
    /// while the properties are hidden).
    fn apply_fields(&mut self, cx: &mut Context<Self>) {
        if !SHOW_BREAKPOINT_EXTRAS {
            return;
        }
        let Some((path, line)) = self.editing.clone() else {
            return;
        };
        let store = BreakpointStore::global(cx);
        let Some(enabled) = store.read(cx).at(&path, line).map(|b| b.enabled) else {
            return;
        };
        let [a, b, c] = self.fields.clone();
        let properties = properties_from_fields(
            enabled,
            [a.read(cx).text(), b.read(cx).text(), c.read(cx).text()],
        );
        store.update(cx, |s, cx| s.set_properties(&path, line, properties, cx));
    }

    /// The fields' events and focus listeners, once.
    fn wire_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.wired.is_empty() {
            return;
        }
        for field in ALL_FIELDS {
            let input = self.fields[field.ix()].clone();
            let handle = input.read(cx).focus_handle(cx);
            self.wired.push(cx.subscribe_in(
                &input,
                window,
                move |this, _, event: &InputEvent, window, cx| {
                    this.on_field(field, event, window, cx)
                },
            ));
            self.wired.push(cx.on_focus(&handle, window, |this, _, _| {
                this.focused_fields += 1;
            }));
            self.wired.push(cx.on_blur(&handle, window, |this, _, cx| {
                this.focused_fields = (this.focused_fields - 1).max(0);
                this.apply_fields(cx);
            }));
        }
    }

    fn on_field(
        &mut self,
        field: Field,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {}
            InputEvent::Submit { .. } => self.apply_fields(cx),
            InputEvent::Cancel => {
                // The stored values back, the keyboard back on the list.
                self.editing = None;
                self.load_fields(cx);
                self.focus_handle.focus(window, cx);
            }
            InputEvent::Tab { shift } => match field.next(*shift) {
                Some(next) => {
                    let handle = self.fields[next.ix()].read(cx).focus_handle(cx);
                    handle.focus(window, cx);
                }
                None => self.focus_handle.focus(window, cx),
            },
        }
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.reveal = true;
        self.select(self.selected.saturating_sub(1), cx);
    }

    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.items(cx).len();
        self.reveal = true;
        self.select((self.selected + 1).min(n.saturating_sub(1)), cx);
    }

    fn confirm(&mut self, _: &PopupConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.open(self.selected, cx);
    }

    fn open(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(Item::Line(row)) = self.items(cx).get(ix).cloned() {
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
        if let Some(item) = self.selected_item(cx) {
            set_item_enabled(&item, !item.enabled(), cx);
        }
    }

    fn remove(&mut self, _: &DebugRemoveBreakpoint, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(Item::Line(row)) = self.selected_item(cx) {
            let store = BreakpointStore::global(cx);
            store.update(cx, |s, cx| s.remove(&row.path, row.line, cx));
        }
    }

    fn click(&mut self, ix: usize, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.select(ix, cx);
        if count == 2 {
            self.open(ix, cx);
        }
    }

    /// A right click on row `ix`: the row selected and Fleet's row menu at
    /// the pointer (a line breakpoint's only).
    fn row_menu(
        &mut self,
        ix: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        self.select(ix, cx);
        let Some(Item::Line(row)) = self.selected_item(cx) else {
            return;
        };
        let entries = crate::breakpoints::tool_menu(&row.path, row.line, cx);
        if entries.is_empty() {
            return;
        }
        cx.emit(DebugViewEvent::Menu {
            position,
            entries: Rc::new(RefCell::new(entries)),
        });
    }
}

fn set_enabled(path: &Path, line: usize, enabled: bool, cx: &mut App) {
    let store = BreakpointStore::global(cx);
    store.update(cx, |s, cx| s.set_enabled(path, line, enabled, cx));
}

fn set_item_enabled(item: &Item, enabled: bool, cx: &mut App) {
    match item {
        Item::Line(r) => set_enabled(&r.path, r.line, enabled, cx),
        Item::Exception(e) => {
            let store = BreakpointStore::global(cx);
            store.update(cx, |s, cx| s.set_exception_enabled(&e.filter, enabled, cx));
        }
    }
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

fn check_state(on: bool) -> CheckState {
    if on {
        CheckState::All
    } else {
        CheckState::Off
    }
}

impl Render for BreakpointsTool {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.wire_fields(window, cx);
        let items = self.items(cx);
        if self.selected >= items.len() {
            self.selected = items.len().saturating_sub(1);
        }
        let tui = Skin::current(cx) == Skin::Tui;
        let body = if tui {
            self.render_tui(items, window, cx)
        } else {
            self.render_fleet(items, window, cx)
        };
        div()
            .id("breakpoints-tool")
            .size_full()
            .relative()
            .overflow_hidden()
            .child(body)
    }
}

impl BreakpointsTool {
    /// The list's key context and focus, around the rows only (not the
    /// properties' fields).
    fn list_keys(&self, d: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        d.key_context("BreakpointsTool")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::toggle_enabled))
            .on_action(cx.listener(Self::remove))
    }

    fn render_fleet(
        &mut self,
        items: Vec<Item>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        if items.is_empty() {
            let empty = no_breakpoints().fleet(&theme, &m);
            return self
                .list_keys(div().size_full().child(empty), cx)
                .into_any_element();
        }
        let focused = self.focus_handle.is_focused(window);
        let entity = cx.entity();
        let rows: Vec<BpRow> = items
            .iter()
            .filter_map(|i| match i {
                Item::Line(r) => Some(r.clone()),
                Item::Exception(_) => None,
            })
            .collect();
        let n_lines = rows.len();
        let n_exceptions = items.len() - n_lines;
        let tooltips: Vec<Option<String>> = rows
            .iter()
            .map(|r| {
                self.line_text(&r.path, r.line, cx)
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
            })
            .collect();
        if self.reveal {
            self.reveal = false;
            // The exception group's title is a child of the list too.
            let child = self.selected + usize::from(self.selected >= n_lines);
            self.scroll.scroll_to_item(child);
        }
        let all = all_state(&rows);
        let all_rows = rows.clone();
        let title = |text: &'static str, count: usize| {
            div()
                .flex_shrink_0()
                .h(px(m.breakpoints_header))
                .pl(px(m.debug_section_pad_x))
                .pr(px(m.git_checkbox_right))
                .flex()
                .items_center()
                .child(section_title(text, &theme, &m))
                .child(
                    div()
                        .ml(px(m.breakpoints_count_gap))
                        .text_color(theme.text_secondary)
                        .child(SharedString::from(count.to_string())),
                )
                .child(div().flex_1())
        };
        let header = title("Breakpoints", n_lines)
            .mt(px(m.breakpoints_top))
            .mx(px(m.panel_pad_x))
            .when(n_lines > 0, |d| {
                d.child(
                    checkbox("bp-check-all", all, &theme, &m).on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        set_all(&all_rows, cx);
                    }),
                )
            });
        let selected_ix = self.selected;
        // A row: the wash, the 16 px box holding `mark`, the text, then
        // the row's check box at the right.
        let row = |ix: usize,
                   id: &'static str,
                   mark: AnyElement,
                   text: Vec<AnyElement>,
                   item: Item| {
            let selected = ix == selected_ix;
            let click = entity.clone();
            let menu = entity.clone();
            let on = item.enabled();
            div()
                .id(ElementId::NamedInteger(id.into(), ix as u64))
                .h(px(m.tree_row_height))
                .flex_shrink_0()
                .py(px(m.tree_row_inset))
                .on_click(move |e, window, cx| {
                    let count = e.click_count();
                    click.update(cx, |this, cx| this.click(ix, count, window, cx));
                })
                .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                    let at = e.position;
                    menu.update(cx, |this, cx| this.row_menu(ix, at, window, cx));
                })
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
                        // The mark centred in the chevron's 16 px box.
                        .child(
                            div()
                                .size(px(m.icon_size))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(mark),
                        )
                        .children(text)
                        .child(
                            checkbox(
                                ElementId::NamedInteger(format!("{id}-check").into(), ix as u64),
                                check_state(on),
                                &theme,
                                &m,
                            )
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                set_item_enabled(&item, !on, cx);
                            }),
                        ),
                )
        };
        let mut children: Vec<AnyElement> = Vec::new();
        for (ix, (item, tooltip)) in items
            .into_iter()
            .zip(
                tooltips
                    .into_iter()
                    .map(Some)
                    .chain(std::iter::repeat(None)),
            )
            .enumerate()
        {
            if ix == n_lines && n_exceptions > 0 {
                children.push(
                    title(EXCEPTIONS, n_exceptions)
                        .mt(px(m.breakpoints_top))
                        .into_any_element(),
                );
            }
            let el = match &item {
                Item::Line(r) => {
                    let d = px(m.breakpoints_dot);
                    let dot = div().size(d).rounded_full().map(|dot| match r.look() {
                        Look::Enabled => dot.bg(theme.breakpoint),
                        Look::Unverified => dot.bg(theme.breakpoint_unverified),
                        // Estimate: the gutter's ring.
                        Look::Disabled => ring(dot, d * 0.15).border_color(theme.breakpoint),
                    });
                    let text = vec![
                        div()
                            .flex_shrink_0()
                            .text_color(theme.list_text)
                            .child(SharedString::from(r.name.clone()))
                            .into_any_element(),
                        div()
                            .ml(px(m.breakpoints_folder_gap - m.debug_chevron_gap))
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_color(theme.list_text_secondary)
                            .child(SharedString::from(r.folder.clone()))
                            .into_any_element(),
                    ];
                    row(ix, "bp-row", dot.into_any_element(), text, item.clone())
                        .when_some(tooltip.flatten(), |d, text| d.tooltip(Tooltip::text(text)))
                        .into_any_element()
                }
                Item::Exception(e) => {
                    // expui's exception breakpoint, its outline when off.
                    let glyph = if e.enabled {
                        "expui/breakpoints/breakpointException"
                    } else {
                        "expui/breakpoints/breakpointExceptionDisabled"
                    };
                    let text = vec![div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_color(theme.list_text)
                        .child(SharedString::from(e.label.clone()))
                        .into_any_element()];
                    let mark = icon(glyph, m.icon_size, theme.breakpoint).into_any_element();
                    row(ix, "bp-exc-row", mark, text, item.clone())
                        .tooltip(Tooltip::text(e.filter.clone()))
                        .into_any_element()
                }
            };
            children.push(el);
        }
        let props = self.fleet_properties(&theme, &m, window, cx);
        let list = self.list_keys(
            div()
                .flex_1()
                .min_h_0()
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
                ),
            cx,
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(list)
            .children(props)
            .into_any_element()
    }

    /// The breakpoint the fields edit, with its row's name.
    fn edited(&self, cx: &App) -> Option<(PathBuf, LineBreakpoint, String)> {
        let (path, line) = self.editing.clone()?;
        let b = BreakpointStore::global(cx)
            .read(cx)
            .at(&path, line)
            .cloned()?;
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let name = format!("{file}:{}", line + 1);
        Some((path, b, name))
    }

    /// The selected line breakpoint's properties under the list (estimate:
    /// IntelliJ's dialog in the Debug tool's section form): the splitter,
    /// a title row with its name, "Enabled", then the three fields.
    fn fleet_properties(
        &mut self,
        theme: &Theme,
        m: &Metrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !SHOW_BREAKPOINT_EXTRAS {
            return None;
        }
        let (path, b, name) = self.edited(cx)?;
        let (line, enabled) = (b.line, b.enabled);
        let gap = px(PROPS_GAP * m.zoom);
        let fields = ALL_FIELDS.map(|f| {
            let input = self.fields[f.ix()].clone();
            let focused = input.read(cx).is_focused(window);
            div()
                .flex_shrink_0()
                .mt(gap)
                .flex()
                .items_center()
                .child(
                    div()
                        .w(px(PROPS_LABEL * m.zoom))
                        .flex_shrink_0()
                        .pl(px(m.debug_row_pad))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_color(theme.text_secondary)
                        .child(FIELDS[f.ix()].0),
                )
                .child(crate::search_tool::frame(focused, window, theme, m).child(input))
        });
        Some(
            div()
                .flex_shrink_0()
                .key_context("BreakpointProperties")
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_shrink_0()
                        .h(px(m.debug_splitter))
                        .mx(px(m.panel_pad_x))
                        .bg(theme.background_primary),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .px(px(m.panel_pad_x))
                        .pt(px(m.debug_splitter_gap))
                        .pb(gap)
                        .child(
                            div()
                                .h(px(m.breakpoints_header))
                                .pl(px(m.debug_section_pad_x))
                                .flex()
                                .items_center()
                                .whitespace_nowrap()
                                .text_size(px(m.ui_font_size))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text_primary)
                                .child(SharedString::from(name)),
                        )
                        .child(
                            div()
                                .id("bp-prop-enabled")
                                .h(px(m.tree_row_height))
                                .pl(px(m.debug_row_pad))
                                .flex()
                                .items_center()
                                .gap(px(m.debug_chevron_gap))
                                .cursor_pointer()
                                .on_click(move |_, _, cx| set_enabled(&path, line, !enabled, cx))
                                .child(checkbox(
                                    "bp-prop-enabled-box",
                                    check_state(enabled),
                                    theme,
                                    m,
                                ))
                                .child(div().text_color(theme.list_text).child("Enabled")),
                        )
                        .children(fields),
                )
                .into_any_element(),
        )
    }

    /// The tool in the tui skin: the title row ` Breakpoints 1` with `[x]`
    /// (`[-]` when some are off) at its right end, then a row per
    /// breakpoint ` ● debuggee.rs:7 misc` (`○` disabled, `●` in
    /// `breakpoint_unverified` when unbound) with its `[x]` at the right;
    /// then ` Exception Breakpoints 1` and a row per filter ` ◆ Rust: on
    /// panic` (`◇` off) with its box. A selected line breakpoint's
    /// properties take the last rows: a section rule with its name set in,
    /// ` [x] Enabled`, and the fields ` condition`, ` log`, ` hit count`
    /// on `input_bg`.
    fn render_tui(
        &mut self,
        items: Vec<Item>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.tui_bars.clear();
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let (cols, grid_rows) = self.tui_grid.get();
        // The grid's font and its measure, around the list or the empty tool.
        let grid = self.tui_grid.clone();
        let frame = |child: AnyElement| {
            div()
                .size_full()
                .relative()
                .flex()
                .flex_col()
                .overflow_hidden()
                .font_family(theme.mono_font.clone())
                .text_size(px(cell.font_size))
                .line_height(px(cell.h))
                .child(skin::measure(cell, &grid))
                .child(child)
        };
        if items.is_empty() {
            let empty = no_breakpoints().tui(cell, cols, grid_rows, &theme);
            return self.list_keys(frame(empty), cx).into_any_element();
        }
        let props = self.tui_properties(cell, cols, &theme, &m, window, cx);
        let props_rows = if props.is_some() { TUI_PROPS_ROWS } else { 0 };
        let visible = grid_rows.saturating_sub(1 + props_rows);
        let focused = self.focus_handle.is_focused(window);
        let entity = cx.entity();
        let n_lines = items.iter().filter(|i| matches!(i, Item::Line(_))).count();
        let rows: Vec<BpRow> = items
            .iter()
            .filter_map(|i| match i {
                Item::Line(r) => Some(r.clone()),
                Item::Exception(_) => None,
            })
            .collect();
        // The list's rows: the items, the exception group's title among
        // them.
        enum Shown {
            Item(usize, Item),
            Exceptions(usize),
        }
        let n_exceptions = items.len() - n_lines;
        let mut shown: Vec<Shown> = Vec::new();
        for (ix, item) in items.into_iter().enumerate() {
            if ix == n_lines && n_exceptions > 0 {
                shown.push(Shown::Exceptions(n_exceptions));
            }
            shown.push(Shown::Item(ix, item));
        }
        let total = shown.len();
        if self.reveal {
            self.reveal = false;
            let at = shown
                .iter()
                .position(|s| matches!(s, Shown::Item(ix, _) if *ix == self.selected));
            crate::file_tree::reveal_rows(&mut self.tui_scroll, at, total, visible);
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
        let gap = || div().w(px(cell.w)).flex_shrink_0();
        let title_row = |text: &'static str, count: usize| {
            div()
                .h(px(cell.h))
                .w(cell.cols(cols))
                .flex_shrink_0()
                .flex()
                .whitespace_nowrap()
                .overflow_hidden()
                .child(gap())
                .child(
                    div()
                        .text_color(theme.text_primary)
                        .child(SharedString::from(format!("{text} "))),
                )
                .child(
                    div()
                        .text_color(theme.text_secondary)
                        .child(SharedString::from(count.to_string())),
                )
                .child(div().flex_1())
        };
        let all = all_state(&rows);
        let all_rows = rows.clone();
        let header = title_row("Breakpoints", n_lines).when(n_lines > 0, |d| {
            d.child(
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
            .child(gap())
        });
        let children: Vec<AnyElement> = shown
            .into_iter()
            .enumerate()
            .skip(top)
            .take(visible)
            .map(|(at, s)| {
                let (ix, item) = match s {
                    Shown::Exceptions(n) => return title_row(EXCEPTIONS, n).into_any_element(),
                    Shown::Item(ix, item) => (ix, item),
                };
                let selected = ix == self.selected;
                let click = entity.clone();
                let menu = entity.clone();
                let (glyph, color, name, folder) = match &item {
                    Item::Line(r) => {
                        let (glyph, color) = match r.look() {
                            Look::Enabled => ("●", theme.breakpoint),
                            Look::Unverified => ("●", theme.breakpoint_unverified),
                            Look::Disabled => ("○", theme.breakpoint),
                        };
                        (glyph, color, r.name.clone(), r.folder.clone())
                    }
                    Item::Exception(e) => {
                        let glyph = if e.enabled { "◆" } else { "◇" };
                        (glyph, theme.breakpoint, e.label.clone(), String::new())
                    }
                };
                let room = cols.saturating_sub(3 + 5);
                let name = skin::fit_text(&name, room);
                let folder_room = room.saturating_sub(Cell::text_cols(&name) + 1);
                let folder = skin::fit_text(&folder, folder_room);
                let on = item.enabled();
                div()
                    .id(ElementId::NamedInteger("bp-row".into(), (at - top) as u64))
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
                    .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                        let at = e.position;
                        menu.update(cx, |this, cx| this.row_menu(ix, at, window, cx));
                    })
                    .child(gap())
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
                            ElementId::NamedInteger("bp-check".into(), (at - top) as u64),
                            skin::checkbox(on),
                        )
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            set_item_enabled(&item, !on, cx);
                        }),
                    )
                    .child(gap())
                    .into_any_element()
            })
            .collect();
        let list = self.list_keys(
            div().flex().flex_col().child(header).child(tui_list(
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
            )),
            cx,
        );
        frame(list.into_any_element())
            .child(div().flex_1())
            .children(props)
            .into_any_element()
    }

    /// The tui properties ([`TUI_PROPS_ROWS`] rows): `── debuggee.rs:7 ──`,
    /// ` [x] Enabled`, then a row per field, its label in `text_tertiary`
    /// (`text_primary` while it has the keyboard) and the field on
    /// `input_bg` to one cell from the right.
    fn tui_properties(
        &mut self,
        cell: Cell,
        cols: usize,
        theme: &Theme,
        m: &Metrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !SHOW_BREAKPOINT_EXTRAS {
            return None;
        }
        let (path, b, name) = self.edited(cx)?;
        let (line, enabled) = (b.line, b.enabled);
        let gap = || div().w(px(cell.w)).flex_shrink_0();
        let fields = ALL_FIELDS.map(|f| {
            let input = self.fields[f.ix()].clone();
            let focused = input.read(cx).is_focused(window);
            div()
                .h(px(cell.h))
                .w(cell.cols(cols))
                .flex_shrink_0()
                .flex()
                .whitespace_nowrap()
                .child(
                    div()
                        .w(cell.cols(TUI_LABEL))
                        .flex_shrink_0()
                        .overflow_hidden()
                        .text_color(if focused {
                            theme.text_primary
                        } else {
                            theme.text_tertiary
                        })
                        .child(SharedString::from(format!(" {}", TUI_FIELDS[f.ix()]))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(px(cell.h))
                        .bg(theme.input_bg)
                        .child(input),
                )
                .child(gap())
        });
        Some(
            div()
                .flex_shrink_0()
                .key_context("BreakpointProperties")
                .flex()
                .flex_col()
                .child(skin::section(
                    cell,
                    cols,
                    vec![skin::title(name, theme.text_primary)],
                    theme.border,
                    theme.island_background,
                    *m,
                ))
                .child(
                    div()
                        .id("bp-prop-enabled")
                        .h(px(cell.h))
                        .w(cell.cols(cols))
                        .flex_shrink_0()
                        .flex()
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.list_bg_hovered))
                        .on_click(move |_, _, cx| set_enabled(&path, line, !enabled, cx))
                        .child(gap())
                        .child(
                            div()
                                .text_color(theme.text_secondary)
                                .child(skin::checkbox(enabled)),
                        )
                        .child(div().text_color(theme.list_text).child(" Enabled")),
                )
                .children(fields)
                .into_any_element(),
        )
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

    #[test]
    fn exception_breakpoints_follow_the_line_ones() {
        let a = PathBuf::from("/w/main.rs");
        let la = vec![LineBreakpoint::at(1)];
        let rows = bp_rows(vec![(a.as_path(), la.as_slice())], Path::new("/w"));
        let exceptions = vec![
            ExceptionBreakpoint {
                filter: "rust_panic".into(),
                label: "Rust: on panic".into(),
                enabled: false,
            },
            ExceptionBreakpoint {
                filter: "cpp_throw".into(),
                label: "C++: on throw".into(),
                enabled: true,
            },
        ];
        let list = items(rows, &exceptions);
        assert_eq!(list.len(), 3);
        assert!(matches!(&list[0], Item::Line(r) if r.name == "main.rs:2"));
        assert!(matches!(&list[1], Item::Exception(e) if e.filter == "rust_panic"));
        assert!(!list[1].enabled());
        assert!(list[2].enabled());
        assert!(items(Vec::new(), &exceptions)
            .iter()
            .all(|i| matches!(i, Item::Exception(_))));
    }

    #[test]
    fn fields_become_properties() {
        let p = properties_from_fields(true, ["i > 3", "  ", "2"]);
        assert_eq!(
            p,
            Properties {
                enabled: true,
                condition: Some("i > 3".into()),
                log_message: None,
                hit_condition: Some("2".into()),
            }
        );
        assert_eq!(Field::Condition.next(false), Some(Field::Log));
        assert_eq!(Field::Hits.next(false), None);
        assert_eq!(Field::Condition.next(true), None);
        assert_eq!(Field::Hits.next(true), Some(Field::Log));
    }
}
