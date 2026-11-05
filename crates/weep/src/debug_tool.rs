//! The Debug tool (M8): the active debug session's threads and frames on the
//! left and the variables tree on the right, as Fleet 1.48 shows them in the
//! bottom panel. No probe of Fleet's Debug tool exists yet, so the layout
//! follows Fleet's family of IDEs (IntelliJ's Frames | Variables split) and
//! every size here is an estimate, reusing the Files tree's row metrics: a
//! thread chooser above the frames when the debuggee has several threads,
//! each frame `name  file:line` with the selected one in the list's
//! selection colours (`listItem.*`), and the tree with the watches at its
//! top (Fleet's `debugger-nodes/watch.svg` names them), then the frame's
//! scopes, the first one expanded, each variable `name = {type} value`.
//!
//! The view follows `Debugger::global`'s active session and calls its
//! commands (`select_frame`, `toggle_expanded`, the watches); the
//! workspace opens a frame's file through [`DebugViewEvent::Open`] and
//! shows the right-click and thread menus through [`DebugViewEvent::Menu`].
//! The key context is `DebugTool`.
//!
//! This file also holds what the three debugger tools share: the event, the
//! [`DebugViews`] the workspace keeps, the key bindings and [`Follow`].

use crate::actions::{
    DebugAddWatch, DebugDeleteWatch, DebugEditWatch, DebugRemoveBreakpoint,
    DebugToggleEnabledBreakpoint, PopupConfirm, PopupDown, PopupLeft, PopupRight, PopupTab,
    PopupUp,
};
use crate::assets::icon;
use crate::breakpoints_tool::BreakpointsTool;
use crate::debug_console::DebugConsole;
use crate::debugger::{DebugSession, DebugSessionEvent, Debugger, SessionState, Variable, Watch};
use crate::git_tool::tui_list;
use crate::input::{InputEvent, TextInput};
use crate::menu::MenuEntry;
use crate::skin::{self, Cell, FrameBars, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, relative, AnyElement, App, ClipboardItem, Context, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, HighlightStyle, Hsla, KeyBinding, MouseButton,
    MouseDownEvent, Pixels, Point, Render, ScrollHandle, SharedString, StyledText, Subscription,
    Window,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use weep_dap::Scope;

// ---- shared by the three tools ----------------------------------------------

/// What a debugger tool asks of the workspace.
pub enum DebugViewEvent {
    /// Open `path` at the 0-based `line` (a frame, a breakpoint), giving the
    /// editor the focus when `focus` is set.
    Open {
        path: PathBuf,
        line: usize,
        focus: bool,
    },
    /// Show a menu at `position` (window coordinates): the rows are taken
    /// out of the cell once (a `MenuEntry` is not `Clone`).
    Menu {
        position: Point<Pixels>,
        entries: Rc<RefCell<Vec<MenuEntry>>>,
    },
}

impl DebugViewEvent {
    fn menu(position: Point<Pixels>, entries: Vec<MenuEntry>) -> DebugViewEvent {
        DebugViewEvent::Menu {
            position,
            entries: Rc::new(RefCell::new(entries)),
        }
    }
}

/// The three debugger tools a workspace shows in its bottom panel.
pub struct DebugViews {
    pub debug: Entity<DebugTool>,
    pub breakpoints: Entity<BreakpointsTool>,
    pub console: Entity<DebugConsole>,
}

impl DebugViews {
    pub fn new<V: 'static>(root: PathBuf, window: &mut Window, cx: &mut Context<V>) -> DebugViews {
        DebugViews {
            debug: cx.new(|cx| DebugTool::new(window, cx)),
            breakpoints: cx.new(|cx| BreakpointsTool::new(root, cx)),
            console: cx.new(|cx| DebugConsole::new(window, cx)),
        }
    }
}

/// The tools' own keys: the lists' arrows and Enter, Delete on a watch or a
/// breakpoint, Space on a breakpoint's check box. The chords Fleet gives
/// the watch actions (`debug/add-watch`, `debug/edit-watch`) come from the
/// keymap, in the `DebugTool` context.
pub fn bind_keys(cx: &mut App) {
    let debug = Some("DebugTool");
    let breakpoints = Some("BreakpointsTool");
    let console = Some("DebuggerConsole");
    cx.bind_keys([
        KeyBinding::new("up", PopupUp, debug),
        KeyBinding::new("down", PopupDown, debug),
        KeyBinding::new("left", PopupLeft, debug),
        KeyBinding::new("right", PopupRight, debug),
        KeyBinding::new("enter", PopupConfirm, debug),
        KeyBinding::new("tab", PopupTab, debug),
        KeyBinding::new("delete", DebugDeleteWatch, debug),
        KeyBinding::new("backspace", DebugDeleteWatch, debug),
        KeyBinding::new("up", PopupUp, breakpoints),
        KeyBinding::new("down", PopupDown, breakpoints),
        KeyBinding::new("enter", PopupConfirm, breakpoints),
        KeyBinding::new("space", DebugToggleEnabledBreakpoint, breakpoints),
        KeyBinding::new("delete", DebugRemoveBreakpoint, breakpoints),
        KeyBinding::new("backspace", DebugRemoveBreakpoint, breakpoints),
        KeyBinding::new("up", PopupUp, console),
        KeyBinding::new("down", PopupDown, console),
    ]);
}

/// A tool's hold on the active debug session: it follows the `Debugger`'s
/// active one and redraws the view whenever that session changes.
#[derive(Default)]
pub struct Follow {
    pub session: Option<Entity<DebugSession>>,
    session_subscriptions: Vec<Subscription>,
    _debugger: Option<Subscription>,
}

impl Follow {
    /// Starts following for the view `V` (in its constructor): `follow`
    /// finds the view's `Follow`, `switched` runs when the active session
    /// changes.
    pub fn start<V: 'static>(
        follow: fn(&mut V) -> &mut Follow,
        switched: fn(&mut V, &mut Context<V>),
        cx: &mut Context<V>,
    ) -> Follow {
        let debugger = Debugger::global(cx);
        let active = debugger.read(cx).active();
        let observer = cx.observe(&debugger, move |this: &mut V, debugger, cx| {
            let active = debugger.read(cx).active();
            if follow(this).session != active {
                let subscriptions = Self::watch(active.as_ref(), cx);
                let f = follow(this);
                f.session = active;
                f.session_subscriptions = subscriptions;
                switched(this, cx);
            }
            cx.notify();
        });
        Follow {
            session_subscriptions: Self::watch(active.as_ref(), cx),
            session: active,
            _debugger: Some(observer),
        }
    }

    fn watch<V: 'static>(
        session: Option<&Entity<DebugSession>>,
        cx: &mut Context<V>,
    ) -> Vec<Subscription> {
        match session {
            Some(s) => vec![
                cx.observe(s, |_: &mut V, _, cx| cx.notify()),
                cx.subscribe(s, |_: &mut V, _, _: &DebugSessionEvent, cx| cx.notify()),
            ],
            None => Vec::new(),
        }
    }
}

/// `path` relative to `root` when it is inside it, as the tools print it.
pub fn relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// A value on one row: line breaks shown as spaces.
fn single_line(text: &str) -> String {
    if text.contains(['\n', '\r']) {
        text.split(['\n', '\r'])
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        text.to_string()
    }
}

// ---- the variables tree as rows ----------------------------------------------

/// The watch being typed inline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editing {
    /// A new watch, in a field after the last one.
    New,
    /// The watch at this index, its row turned into the field.
    Watch(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A watch, by its index in the session's `watches`.
    Watch(usize),
    /// The inline field: adding a watch (`None`) or editing one.
    WatchInput(Option<usize>),
    Scope,
    Variable,
    /// An expanded node whose children have not arrived yet.
    Loading,
}

/// One row of the variables tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: usize,
    pub kind: NodeKind,
    pub name: String,
    pub value: Option<String>,
    pub type_name: Option<String>,
    /// The value is an evaluation's error.
    pub error: bool,
    /// The DAP variables reference; 0 for a leaf.
    pub reference: i64,
    pub expanded: bool,
    /// The expression that evaluates to this node (Add to Watches).
    pub expression: Option<String>,
}

impl TreeRow {
    pub fn expandable(&self) -> bool {
        self.reference != 0
            && matches!(
                self.kind,
                NodeKind::Watch(_) | NodeKind::Scope | NodeKind::Variable
            )
    }
}

/// What the tree is built from: the session's fields.
pub struct TreeSource<'a> {
    pub watches: &'a [Watch],
    pub scopes: &'a [Scope],
    pub children: &'a HashMap<i64, Vec<Variable>>,
    pub expanded: &'a HashSet<i64>,
}

impl<'a> TreeSource<'a> {
    pub fn of(session: &'a DebugSession) -> TreeSource<'a> {
        TreeSource {
            watches: &session.watches,
            scopes: &session.scopes,
            children: &session.children,
            expanded: &session.expanded,
        }
    }
}

/// How deep the tree goes: a guard against an adapter that hands out the
/// same reference for a node and its child.
const MAX_DEPTH: usize = 64;

/// The tree flattened into rows: the watches (or the field editing one),
/// the field for a new watch, then each scope with its expanded children.
pub fn tree_rows(src: &TreeSource, editing: Option<Editing>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for (ix, watch) in src.watches.iter().enumerate() {
        if editing == Some(Editing::Watch(ix)) {
            rows.push(input_row(Some(ix)));
            continue;
        }
        let (value, type_name, error, reference) = match &watch.value {
            None => (None, None, false, 0),
            Some(Ok(v)) => (
                Some(single_line(&v.value)),
                v.type_name.clone(),
                false,
                v.variables_reference,
            ),
            Some(Err(e)) => (Some(single_line(e)), None, true, 0),
        };
        let expanded = reference != 0 && src.expanded.contains(&reference);
        rows.push(TreeRow {
            depth: 0,
            kind: NodeKind::Watch(ix),
            name: watch.expression.clone(),
            value,
            type_name,
            error,
            reference,
            expanded,
            expression: Some(watch.expression.clone()),
        });
        if expanded {
            let parent = watch_parent(&watch.expression);
            push_children(
                src,
                reference,
                1,
                Some(&parent),
                &mut vec![reference],
                &mut rows,
            );
        }
    }
    if editing == Some(Editing::New) {
        rows.push(input_row(None));
    }
    for scope in src.scopes {
        let reference = scope.variables_reference;
        let expanded = reference != 0 && src.expanded.contains(&reference);
        rows.push(TreeRow {
            depth: 0,
            kind: NodeKind::Scope,
            name: scope.name.clone(),
            value: None,
            type_name: None,
            error: false,
            reference,
            expanded,
            expression: None,
        });
        if expanded {
            push_children(src, reference, 1, None, &mut vec![reference], &mut rows);
        }
    }
    rows
}

fn input_row(editing: Option<usize>) -> TreeRow {
    TreeRow {
        depth: 0,
        kind: NodeKind::WatchInput(editing),
        name: String::new(),
        value: None,
        type_name: None,
        error: false,
        reference: 0,
        expanded: false,
        expression: None,
    }
}

fn push_children(
    src: &TreeSource,
    reference: i64,
    depth: usize,
    parent: Option<&str>,
    path: &mut Vec<i64>,
    rows: &mut Vec<TreeRow>,
) {
    let Some(children) = src.children.get(&reference) else {
        rows.push(TreeRow {
            depth,
            kind: NodeKind::Loading,
            // Estimate: IntelliJ's placeholder while a node's children load.
            name: "Collecting data…".into(),
            value: None,
            type_name: None,
            error: false,
            reference: 0,
            expanded: false,
            expression: None,
        });
        return;
    };
    for v in children {
        let expression = v
            .evaluate_name
            .clone()
            .unwrap_or_else(|| child_expression(parent, &v.name));
        let r = v.variables_reference;
        let expanded = r != 0 && src.expanded.contains(&r);
        rows.push(TreeRow {
            depth,
            kind: NodeKind::Variable,
            name: v.name.clone(),
            value: Some(single_line(&v.value)),
            type_name: v.type_name.clone(),
            error: false,
            reference: r,
            expanded,
            expression: Some(expression.clone()),
        });
        if expanded && depth < MAX_DEPTH && !path.contains(&r) {
            path.push(r);
            push_children(src, r, depth + 1, Some(&expression), path, rows);
            path.pop();
        }
    }
}

/// A watch's expression as the parent of its children's: bracketed unless
/// it is a plain path (`a.b[0]`).
fn watch_parent(expression: &str) -> String {
    let plain = expression
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '[' | ']'));
    if plain {
        expression.to_string()
    } else {
        format!("({expression})")
    }
}

/// The expression of a child named `name` (a field, `[0]` an element).
pub fn child_expression(parent: Option<&str>, name: &str) -> String {
    match parent {
        None => name.to_string(),
        Some(p) if name.starts_with('[') => format!("{p}{name}"),
        Some(p) => format!("{p}.{name}"),
    }
}

/// The parts of a row's text, for colouring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Name,
    Punctuation,
    Type,
    Value,
    Error,
    Dim,
}

/// A row's text, `name = {type} value`, and the byte ranges of its parts.
pub fn row_text(row: &TreeRow) -> (String, Vec<(Range<usize>, Tint)>) {
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut push = |text: &mut String, s: &str, tint: Tint| {
        let start = text.len();
        text.push_str(s);
        runs.push((start..text.len(), tint));
    };
    match row.kind {
        NodeKind::Loading => push(&mut text, &row.name, Tint::Dim),
        NodeKind::Scope => push(&mut text, &row.name, Tint::Name),
        NodeKind::WatchInput(_) => {}
        NodeKind::Watch(_) | NodeKind::Variable => {
            push(&mut text, &row.name, Tint::Name);
            match &row.value {
                Some(value) => {
                    push(&mut text, " = ", Tint::Punctuation);
                    if let Some(t) = row.type_name.as_deref().filter(|t| !t.is_empty()) {
                        push(&mut text, &format!("{{{t}}} "), Tint::Type);
                    }
                    push(
                        &mut text,
                        value,
                        if row.error { Tint::Error } else { Tint::Value },
                    );
                }
                // Estimate: a watch with no stopped frame to evaluate in.
                None => push(&mut text, " = not available", Tint::Dim),
            }
        }
    }
    (text, runs)
}

/// `text` and its runs cut to `cols` cells (the tui skin): an ellipsis ends
/// a cut text in the colour of the run it cut.
pub fn clip_runs(
    text: &str,
    runs: &[(Range<usize>, Tint)],
    cols: usize,
) -> (String, Vec<(Range<usize>, Tint)>) {
    let fitted = skin::fit_text(text, cols);
    if fitted.len() == text.len() {
        return (fitted, runs.to_vec());
    }
    let len = fitted.len();
    let kept = fitted.trim_end_matches('…').len();
    let mut out = Vec::new();
    for (range, tint) in runs {
        if range.start >= kept {
            break;
        }
        let end = if range.end >= kept { len } else { range.end };
        out.push((range.start..end, *tint));
    }
    (fitted, out)
}

/// A frame's place as the list prints it: `main.rs:12`.
pub fn frame_location(frame: &weep_dap::StackFrame) -> Option<String> {
    let name = frame
        .path
        .as_ref()?
        .file_name()?
        .to_string_lossy()
        .to_string();
    Some(format!("{name}:{}", frame.line + 1))
}

/// The status row's text and whether it reports a failure; `None` while
/// the debuggee is stopped (the frames say it).
pub fn status_text(state: &SessionState) -> Option<(String, bool)> {
    match state {
        // Estimate: the words IntelliJ's debugger prints while connecting.
        SessionState::Starting => Some(("Starting…".into(), false)),
        SessionState::Running => Some(("Running".into(), false)),
        SessionState::Stopped { .. } => None,
        SessionState::Exited(Some(code)) => {
            Some((format!("Process finished with exit code {code}"), false))
        }
        SessionState::Exited(None) => Some(("Process finished".into(), false)),
        SessionState::Failed(e) => Some((e.clone(), true)),
    }
}

// ---- the view ---------------------------------------------------------------

/// The frames list's share of the tool's width. Estimate: IntelliJ's
/// Frames | Variables split puts the frames at about two fifths.
const FRAMES_SHARE: f32 = 0.4;
/// The tui skin's frames column: two fifths of the cells, at least this many
/// (estimate, as `FRAMES_SHARE`).
const TUI_FRAMES_MIN_COLS: usize = 16;
/// The tree's indent per level in the tui skin, in cells (estimate: the
/// Files tool's guides take two cells a level).
const TUI_INDENT: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Frames,
    Variables,
}

pub struct DebugTool {
    follow: Follow,
    part: Part,
    /// The selected row of the variables tree.
    selected: usize,
    editing: Option<Editing>,
    watch_input: Entity<TextInput>,
    /// The scope expanded by default for the frame last shown: (thread,
    /// frame id, reference), so a collapse by hand stays collapsed.
    auto_expanded: Option<(Option<i64>, i64, i64)>,
    reveal: bool,
    focus_handle: FocusHandle,
    frames_scroll: ScrollHandle,
    tree_scroll: ScrollHandle,
    tui_grid: Rc<std::cell::Cell<(usize, usize)>>,
    tui_bars: FrameBars,
    /// The frames list's bars: not set on the island's frame (the tree's
    /// are), so the two lists do not share one frame line.
    tui_frame_bars: FrameBars,
    tui_frames_scroll: RowScroll,
    tui_tree_scroll: RowScroll,
}

impl EventEmitter<DebugViewEvent> for DebugTool {}

impl Focusable for DebugTool {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl DebugTool {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let watch_input = cx.new(|cx| {
            // Estimate: the hint IntelliJ's watch field shows.
            let mut i = TextInput::new("Expression to watch", cx);
            i.mono = true;
            i
        });
        cx.subscribe_in(&watch_input, window, Self::on_input)
            .detach();
        let follow = Follow::start(|t: &mut DebugTool| &mut t.follow, Self::switched, cx);
        DebugTool {
            follow,
            part: Part::Frames,
            selected: 0,
            editing: None,
            watch_input,
            auto_expanded: None,
            reveal: false,
            focus_handle: cx.focus_handle(),
            frames_scroll: ScrollHandle::new(),
            tree_scroll: ScrollHandle::new(),
            tui_grid: Default::default(),
            tui_bars: Default::default(),
            tui_frame_bars: Default::default(),
            tui_frames_scroll: RowScroll::default(),
            tui_tree_scroll: RowScroll::default(),
        }
    }

    fn switched(&mut self, _cx: &mut Context<Self>) {
        self.selected = 0;
        self.editing = None;
        self.auto_expanded = None;
        self.tui_frames_scroll.reset();
        self.tui_tree_scroll.reset();
    }

    /// The scroll bars this view's lists set on its island's frame.
    pub fn tui_bars(&self) -> FrameBars {
        self.tui_bars.clone()
    }

    fn session(&self) -> Option<Entity<DebugSession>> {
        self.follow.session.clone()
    }

    /// Expands the first scope of a newly shown frame, once (Fleet's family
    /// opens "Locals"); a session that already did leaves it alone.
    fn expand_first_scope(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        let s = session.read(cx);
        if !s.state.is_stopped() {
            return;
        }
        let (Some(scope), Some(frame)) = (s.scopes.first(), s.frames.get(s.current_frame)) else {
            return;
        };
        let key = (s.current_thread, frame.id, scope.variables_reference);
        if self.auto_expanded == Some(key) || scope.variables_reference == 0 {
            return;
        }
        let already = s.expanded.contains(&scope.variables_reference);
        self.auto_expanded = Some(key);
        if !already {
            let reference = scope.variables_reference;
            cx.defer(move |cx| session.update(cx, |s, cx| s.toggle_expanded(reference, cx)));
        }
    }

    fn rows(&self, cx: &App) -> Vec<TreeRow> {
        match &self.follow.session {
            Some(s) => tree_rows(&TreeSource::of(s.read(cx)), self.editing),
            None => Vec::new(),
        }
    }

    fn selected_row(&self, cx: &App) -> Option<TreeRow> {
        self.rows(cx).get(self.selected).cloned()
    }

    // ---- keys ----

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        let Some(session) = self.session() else {
            return;
        };
        match self.part {
            Part::Frames => {
                let s = session.read(cx);
                let n = s.frames.len();
                if n == 0 {
                    return;
                }
                let ix = if forward {
                    (s.current_frame + 1).min(n - 1)
                } else {
                    s.current_frame.saturating_sub(1)
                };
                if ix != s.current_frame {
                    session.update(cx, |s, cx| s.select_frame(ix, cx));
                    self.reveal = true;
                }
            }
            Part::Variables => {
                let n = self.rows(cx).len();
                if n == 0 {
                    return;
                }
                self.selected = if forward {
                    (self.selected + 1).min(n - 1)
                } else {
                    self.selected.saturating_sub(1)
                };
                self.reveal = true;
                cx.notify();
            }
        }
    }

    fn left(&mut self, _: &PopupLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.part != Part::Variables || self.editing.is_some() {
            return;
        }
        let rows = self.rows(cx);
        let Some(row) = rows.get(self.selected) else {
            return;
        };
        if row.expandable() && row.expanded {
            self.toggle(row.reference, cx);
        } else if row.depth > 0 {
            if let Some(parent) = rows[..self.selected]
                .iter()
                .rposition(|r| r.depth < row.depth)
            {
                self.selected = parent;
                self.reveal = true;
                cx.notify();
            }
        } else {
            self.part = Part::Frames;
            cx.notify();
        }
    }

    fn right(&mut self, _: &PopupRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        if self.part == Part::Frames {
            self.part = Part::Variables;
            cx.notify();
            return;
        }
        let rows = self.rows(cx);
        let Some(row) = rows.get(self.selected) else {
            return;
        };
        if row.expandable() && !row.expanded {
            self.toggle(row.reference, cx);
        } else if row.expanded && self.selected + 1 < rows.len() {
            self.selected += 1;
            self.reveal = true;
            cx.notify();
        }
    }

    fn tab(&mut self, _: &PopupTab, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        self.part = match self.part {
            Part::Frames => Part::Variables,
            Part::Variables => Part::Frames,
        };
        cx.notify();
    }

    fn confirm(&mut self, _: &PopupConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        match self.part {
            Part::Frames => {
                if let Some(session) = self.session() {
                    let ix = session.read(cx).current_frame;
                    self.open_frame(ix, cx);
                }
            }
            Part::Variables => {
                let Some(row) = self.selected_row(cx) else {
                    return;
                };
                if row.expandable() {
                    self.toggle(row.reference, cx);
                } else if let NodeKind::Watch(ix) = row.kind {
                    self.start_editing(Editing::Watch(ix), window, cx);
                }
            }
        }
    }

    /// Selects frame `ix` and asks the workspace to show its line.
    fn open_frame(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        if session.read(cx).current_frame != ix {
            session.update(cx, |s, cx| s.select_frame(ix, cx));
        }
        let frame = session.read(cx).frames.get(ix).cloned();
        if let Some(frame) = frame {
            if let Some(path) = frame.path {
                cx.emit(DebugViewEvent::Open {
                    path,
                    line: frame.line,
                    focus: false,
                });
            }
        }
    }

    fn toggle(&mut self, reference: i64, cx: &mut Context<Self>) {
        if let Some(session) = self.session() {
            session.update(cx, |s, cx| s.toggle_expanded(reference, cx));
        }
    }

    // ---- watches ----

    fn add_watch(&mut self, _: &DebugAddWatch, window: &mut Window, cx: &mut Context<Self>) {
        self.start_editing(Editing::New, window, cx);
    }

    fn edit_watch(&mut self, _: &DebugEditWatch, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(TreeRow {
            kind: NodeKind::Watch(ix),
            ..
        }) = self.selected_row(cx)
        {
            self.start_editing(Editing::Watch(ix), window, cx);
        }
    }

    fn delete_watch(&mut self, _: &DebugDeleteWatch, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() || self.part != Part::Variables {
            return;
        }
        if let (
            Some(TreeRow {
                kind: NodeKind::Watch(ix),
                ..
            }),
            Some(session),
        ) = (self.selected_row(cx), self.session())
        {
            session.update(cx, |s, cx| s.delete_watch(ix, cx));
        }
    }

    fn start_editing(&mut self, editing: Editing, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        let text = match editing {
            Editing::New => String::new(),
            Editing::Watch(ix) => match session.read(cx).watches.get(ix) {
                Some(w) => w.expression.clone(),
                None => return,
            },
        };
        self.editing = Some(editing);
        self.part = Part::Variables;
        self.watch_input
            .update(cx, |i, cx| i.set_text_at_end(&text, cx));
        if let Some(ix) = self
            .rows(cx)
            .iter()
            .position(|r| matches!(r.kind, NodeKind::WatchInput(_)))
        {
            self.selected = ix;
            self.reveal = true;
        }
        let handle = self.watch_input.read(cx).focus_handle(cx);
        handle.focus(window, cx);
        cx.notify();
    }

    fn on_input(
        &mut self,
        input: &Entity<TextInput>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Submit { .. } => {
                let text = input.read(cx).text().trim().to_string();
                let editing = self.editing.take();
                if let (Some(editing), Some(session)) = (editing, self.session()) {
                    session.update(cx, |s, cx| match editing {
                        Editing::New if !text.is_empty() => s.add_watch(text, cx),
                        Editing::New => {}
                        Editing::Watch(ix) if text.is_empty() => s.delete_watch(ix, cx),
                        Editing::Watch(ix) => s.edit_watch(ix, text, cx),
                    });
                }
                self.focus_handle.focus(window, cx);
                cx.notify();
            }
            InputEvent::Cancel => {
                self.editing = None;
                self.focus_handle.focus(window, cx);
                cx.notify();
            }
            InputEvent::Changed | InputEvent::Tab { .. } => {}
        }
    }

    // ---- mouse ----

    fn click_row(&mut self, ix: usize, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.part = Part::Variables;
        self.selected = ix;
        if count == 2 {
            if let Some(row) = self.rows(cx).get(ix).cloned() {
                if row.expandable() {
                    self.toggle(row.reference, cx);
                } else if let NodeKind::Watch(w) = row.kind {
                    self.start_editing(Editing::Watch(w), window, cx);
                }
            }
        }
        cx.notify();
    }

    fn click_frame(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.part = Part::Frames;
        self.open_frame(ix, cx);
        cx.notify();
    }

    /// The tree's right-click menu: Copy Value and Add to Watches on a
    /// variable, Edit and Delete on a watch, New Watch everywhere.
    fn row_menu(&mut self, ix: Option<usize>, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        let row = ix.and_then(|ix| self.rows(cx).get(ix).cloned());
        if let Some(ix) = ix {
            self.selected = ix;
            self.part = Part::Variables;
        }
        let weak = cx.weak_entity();
        let mut entries = Vec::new();
        if let Some(row) = &row {
            if let Some(value) = row.value.clone().filter(|_| !row.error) {
                entries.push(MenuEntry::custom("Copy Value", move |_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(value.clone()));
                }));
            }
            match row.kind {
                NodeKind::Variable => {
                    if let Some(expression) = row.expression.clone() {
                        let session = session.clone();
                        entries.push(MenuEntry::custom("Add to Watches", move |_, cx| {
                            session.update(cx, |s, cx| s.add_watch(expression.clone(), cx));
                        }));
                    }
                }
                NodeKind::Watch(w) => {
                    let edit = weak.clone();
                    entries.push(MenuEntry::custom("Edit Watch", move |window, cx| {
                        let _ = edit.update(cx, |this, cx| {
                            this.start_editing(Editing::Watch(w), window, cx)
                        });
                    }));
                    let session = session.clone();
                    entries.push(MenuEntry::custom("Delete Watch", move |_, cx| {
                        session.update(cx, |s, cx| s.delete_watch(w, cx));
                    }));
                }
                _ => {}
            }
            if !entries.is_empty() {
                entries.push(MenuEntry::Separator);
            }
        }
        let add = weak.clone();
        entries.push(MenuEntry::custom("New Watch…", move |window, cx| {
            let _ = add.update(cx, |this, cx| this.start_editing(Editing::New, window, cx));
        }));
        cx.emit(DebugViewEvent::menu(position, entries));
        cx.notify();
    }

    /// The thread chooser's menu: every thread, the shown one ticked.
    fn thread_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        let s = session.read(cx);
        let current = s.current_thread;
        let entries = s
            .threads
            .iter()
            .map(|t| {
                let id = t.id;
                let session = session.clone();
                let label = if Some(id) == current {
                    format!("✓ {}", t.name)
                } else {
                    format!("   {}", t.name)
                };
                MenuEntry::custom(label, move |_, cx| {
                    session.update(cx, |s, cx| s.select_thread(id, cx));
                })
            })
            .collect();
        cx.emit(DebugViewEvent::menu(position, entries));
    }
}

fn tint_color(tint: Tint, theme: &Theme) -> Hsla {
    match tint {
        Tint::Name => theme.list_text,
        Tint::Punctuation => theme.list_text_secondary,
        Tint::Type => theme.list_text_secondary,
        Tint::Value => theme.list_text,
        Tint::Error => theme.text_dangerous,
        Tint::Dim => theme.text_tertiary,
    }
}

fn styled(text: String, runs: &[(Range<usize>, Tint)], theme: &Theme) -> StyledText {
    let highlights: Vec<(Range<usize>, HighlightStyle)> = runs
        .iter()
        .filter(|(r, _)| r.start < r.end && r.end <= text.len())
        .map(|(r, t)| {
            (
                r.clone(),
                HighlightStyle {
                    color: Some(tint_color(*t, theme)),
                    ..Default::default()
                },
            )
        })
        .collect();
    StyledText::new(text).with_highlights(highlights)
}

impl Render for DebugTool {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.expand_first_scope(cx);
        let rows = self.rows(cx);
        if self.selected >= rows.len() {
            self.selected = rows.len().saturating_sub(1);
        }
        let root = div()
            .id("debug-tool")
            .key_context("DebugTool")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::add_watch))
            .on_action(cx.listener(Self::edit_watch))
            .on_action(cx.listener(Self::delete_watch))
            .size_full()
            .relative()
            .overflow_hidden();
        if Skin::current(cx) == Skin::Tui {
            let body = self.render_tui(rows, window, cx);
            return root.child(body);
        }
        let body = self.render_fleet(rows, window, cx);
        root.child(body)
    }
}

impl DebugTool {
    fn render_fleet(
        &mut self,
        rows: Vec<TreeRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let entity = cx.entity();
        let focused = self.focus_handle.contains_focused(window, cx);
        let Some(session) = self.session() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_secondary)
                // Estimate: the empty tool's words.
                .child("No debug session")
                .into_any_element();
        };
        let s = session.read(cx);
        let status = status_text(&s.state).map(|(text, failed)| {
            div()
                .h(px(m.tree_row_height))
                .flex_shrink_0()
                .px(px(m.panel_pad_x + m.tree_pad_left))
                .flex()
                .items_center()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(if failed {
                    theme.text_dangerous
                } else {
                    theme.text_secondary
                })
                .child(SharedString::from(text))
        });
        // The thread chooser: the shown thread's name and a chevron, when
        // there is more than one to choose.
        let chooser = (s.threads.len() > 1).then(|| {
            let name = s
                .threads
                .iter()
                .find(|t| Some(t.id) == s.current_thread)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            let open = entity.clone();
            div()
                .id("debug-thread")
                .h(px(m.tree_row_height))
                .flex_shrink_0()
                .mx(px(m.panel_pad_x))
                .px(px(m.tree_pad_left))
                .flex()
                .items_center()
                .gap(px(m.tree_icon_gap))
                .rounded(px(m.tree_row_radius))
                .cursor_pointer()
                .hover(|s| s.bg(theme.list_bg_hovered))
                .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _, cx| {
                    let at = e.position;
                    open.update(cx, |this, cx| this.thread_menu(at, cx));
                })
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.list_text)
                        .child(SharedString::from(name)),
                )
                .child(icon(
                    "expui/general/chevronDown",
                    m.icon_size,
                    theme.icon_secondary,
                ))
        });
        let frames_focused = focused && self.part == Part::Frames;
        let current = s.current_frame;
        let frame_rows: Vec<AnyElement> = s
            .frames
            .iter()
            .enumerate()
            .map(|(ix, frame)| {
                let selected = ix == current && s.state.is_stopped();
                let location = frame_location(frame);
                let click = entity.clone();
                let mut text = frame.name.clone();
                let name_len = text.len();
                if let Some(l) = &location {
                    text.push_str("  ");
                    text.push_str(l);
                }
                let runs = vec![
                    (0..name_len, Tint::Name),
                    (
                        name_len..text.len(),
                        if location.is_some() {
                            Tint::Type
                        } else {
                            Tint::Dim
                        },
                    ),
                ];
                div()
                    .id(ElementId::NamedInteger("debug-frame".into(), ix as u64))
                    .h(px(m.tree_row_height))
                    .flex_shrink_0()
                    .py(px(m.tree_row_inset))
                    .child(
                        div()
                            .size_full()
                            .px(px(m.tree_pad_left))
                            .flex()
                            .items_center()
                            .rounded(px(m.tree_row_radius))
                            .cursor_pointer()
                            .when(selected && frames_focused, |d| d.bg(theme.list_bg_focused))
                            .when(selected && !frames_focused, |d| {
                                d.bg(theme.list_bg_selected)
                            })
                            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .when(location.is_none(), |d| d.text_color(theme.text_tertiary))
                                    .child(styled(text, &runs, &theme)),
                            ),
                    )
                    .on_click(move |_, window, cx| {
                        click.update(cx, |this, cx| this.click_frame(ix, window, cx));
                    })
                    .into_any_element()
            })
            .collect();
        if self.reveal {
            self.reveal = false;
            match self.part {
                Part::Frames => self.frames_scroll.scroll_to_item(current),
                Part::Variables => self.tree_scroll.scroll_to_item(self.selected),
            }
        }
        let tree_focused = focused && self.part == Part::Variables;
        let tree_rows: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .map(|(ix, row)| self.fleet_tree_row(ix, row, tree_focused, &theme, &m, cx))
            .collect();
        let menu = entity.clone();
        let frames = div()
            .w(relative(FRAMES_SHARE))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .children(chooser)
            .child(
                div()
                    .id("debug-frames")
                    .flex_1()
                    .min_h_0()
                    .px(px(m.panel_pad_x))
                    .overflow_y_scroll()
                    .track_scroll(&self.frames_scroll)
                    .flex()
                    .flex_col()
                    .children(frame_rows),
            );
        let tree = div()
            .id("debug-variables")
            .flex_1()
            .min_w_0()
            .h_full()
            .px(px(m.panel_pad_x))
            .overflow_y_scroll()
            .track_scroll(&self.tree_scroll)
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, _, cx| {
                let at = e.position;
                menu.update(cx, |this, cx| this.row_menu(None, at, cx));
            })
            .children(tree_rows);
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(status)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(frames)
                    // Estimate: a 1 px `border` line between the two lists.
                    .child(div().w(px(1.)).h_full().flex_shrink_0().bg(theme.border))
                    .child(tree),
            )
            .into_any_element()
    }

    fn fleet_tree_row(
        &self,
        ix: usize,
        row: &TreeRow,
        focused: bool,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let selected = ix == self.selected;
        let chevron = if row.expandable() {
            let toggle = entity.clone();
            let reference = row.reference;
            div()
                .id(ElementId::NamedInteger("debug-chevron".into(), ix as u64))
                .size(px(m.icon_size))
                .flex_shrink_0()
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    toggle.update(cx, |this, cx| this.toggle(reference, cx));
                })
                .child(icon(
                    if row.expanded {
                        "expui/general/chevronDown"
                    } else {
                        "expui/general/chevronRight"
                    },
                    m.icon_size,
                    theme.icon_secondary,
                ))
                .into_any_element()
        } else {
            div()
                .size(px(m.icon_size))
                .flex_shrink_0()
                .into_any_element()
        };
        let watch = matches!(row.kind, NodeKind::Watch(_) | NodeKind::WatchInput(_))
            .then(|| icon("weep/watch", m.icon_size, theme.icon_secondary));
        let content = if matches!(row.kind, NodeKind::WatchInput(_)) {
            div()
                .flex_1()
                .min_w_0()
                .h(px(m.tree_row_height - 2. * m.tree_row_inset))
                .child(self.watch_input.clone())
                .into_any_element()
        } else {
            let (text, runs) = row_text(row);
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(styled(text, &runs, theme))
                .into_any_element()
        };
        let click = entity.clone();
        let menu = entity.clone();
        div()
            .id(ElementId::NamedInteger("debug-node".into(), ix as u64))
            .h(px(m.tree_row_height))
            .flex_shrink_0()
            .py(px(m.tree_row_inset))
            .on_click(move |e, window, cx| {
                let count = e.click_count();
                click.update(cx, |this, cx| this.click_row(ix, count, window, cx));
            })
            .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                let at = e.position;
                menu.update(cx, |this, cx| this.row_menu(Some(ix), at, cx));
            })
            .child(
                div()
                    .size_full()
                    .pl(px(m.tree_pad_left + row.depth as f32 * m.tree_indent))
                    .pr(px(m.tree_pad_left))
                    .flex()
                    .items_center()
                    .gap(px(m.tree_icon_gap))
                    .rounded(px(m.tree_row_radius))
                    .cursor_pointer()
                    .when(selected && focused, |d| d.bg(theme.list_bg_focused))
                    .when(selected && !focused, |d| d.bg(theme.list_bg_selected))
                    .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .child(chevron)
                    .children(watch)
                    .child(content),
            )
            .into_any_element()
    }

    /// The tool in the tui skin: a status row when the debuggee runs or
    /// ended, then the frames (the thread chooser `[ main ▾ ]` above them)
    /// and the variables tree side by side, split by a `│` rule. The tree
    /// is `▸ ▾` chevrons and `○` watches on whole cells, two cells a level.
    fn render_tui(
        &mut self,
        rows: Vec<TreeRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.tui_bars.clear();
        self.tui_frame_bars.clear();
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let (cols, grid_rows) = self.tui_grid.get();
        let entity = cx.entity();
        let focused = self.focus_handle.contains_focused(window, cx);
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
                .child(skin::measure(cell, &self.tui_grid))
                .child(child)
                .into_any_element()
        };
        let Some(session) = self.session() else {
            return frame(
                div()
                    .h(px(cell.h))
                    .text_color(theme.text_tertiary)
                    .child(" No debug session")
                    .into_any_element(),
            );
        };
        let s = session.read(cx);
        let status = status_text(&s.state);
        let body_rows = grid_rows.saturating_sub(usize::from(status.is_some()));
        let left = (cols * 2 / 5).max(TUI_FRAMES_MIN_COLS).min(cols);
        let right = cols.saturating_sub(left + 1);
        // The frames column: the chooser, then the frames in whole rows.
        let chooser = (s.threads.len() > 1).then(|| {
            let name = s
                .threads
                .iter()
                .find(|t| Some(t.id) == s.current_thread)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            let open = entity.clone();
            let label = skin::fit_text(&format!("{name} ▾"), left.saturating_sub(4));
            skin::button("debug-thread", &label, false, &theme, cell)
                .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _, cx| {
                    let at = e.position;
                    open.update(cx, |this, cx| this.thread_menu(at, cx));
                })
                .into_any_element()
        });
        let frames_visible = body_rows.saturating_sub(usize::from(chooser.is_some()));
        let current = s.current_frame;
        let frames_total = s.frames.len();
        if self.part == Part::Frames && self.reveal {
            self.tui_frames_scroll.top = self.tui_frames_scroll.top.min(current);
            if current >= self.tui_frames_scroll.top + frames_visible.max(1) {
                self.tui_frames_scroll.top = current + 1 - frames_visible.max(1);
            }
        }
        self.tui_frames_scroll.top = self
            .tui_frames_scroll
            .top
            .min(frames_total.saturating_sub(frames_visible));
        let frames_top = self.tui_frames_scroll.top;
        let frames_focused = focused && self.part == Part::Frames;
        let frame_rows: Vec<AnyElement> = s
            .frames
            .iter()
            .enumerate()
            .skip(frames_top)
            .take(frames_visible)
            .map(|(ix, f)| {
                let selected = ix == current && s.state.is_stopped();
                let location = frame_location(f);
                let mut text = format!(" {}", f.name);
                let name_len = text.len();
                if let Some(l) = &location {
                    text.push_str("  ");
                    text.push_str(l);
                }
                let runs = vec![
                    (
                        0..name_len,
                        if location.is_some() {
                            Tint::Name
                        } else {
                            Tint::Dim
                        },
                    ),
                    (name_len..text.len(), Tint::Type),
                ];
                let (text, runs) = clip_runs(&text, &runs, left);
                let click = entity.clone();
                div()
                    .id(ElementId::NamedInteger(
                        "debug-frame".into(),
                        (ix - frames_top) as u64,
                    ))
                    .h(px(cell.h))
                    .w(cell.cols(left))
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .cursor_pointer()
                    .when(selected && frames_focused, |d| d.bg(theme.list_bg_focused))
                    .when(selected && !frames_focused, |d| {
                        d.bg(theme.list_bg_selected)
                    })
                    .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .on_click(move |_, window, cx| {
                        click.update(cx, |this, cx| this.click_frame(ix, window, cx));
                    })
                    .child(styled(text, &runs, &theme))
                    .into_any_element()
            })
            .collect();
        let frames_list = tui_list(
            "debug-frames",
            &entity,
            |this: &mut DebugTool| &mut this.tui_frames_scroll,
            frames_top,
            frames_total,
            left,
            frames_visible,
            div()
                .flex()
                .flex_col()
                .children(frame_rows)
                .into_any_element(),
            cell,
            &self.tui_frame_bars,
        );
        // The tree in whole rows, its bar on the island's right frame line.
        let total = rows.len();
        let selected = self.selected;
        if self.part == Part::Variables && self.reveal {
            self.tui_tree_scroll.follow(selected, total, body_rows);
        }
        self.reveal = false;
        self.tui_tree_scroll.top = self
            .tui_tree_scroll
            .top
            .min(total.saturating_sub(body_rows));
        let top = self.tui_tree_scroll.top;
        let tree_focused = focused && self.part == Part::Variables;
        let tree_rows: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .skip(top)
            .take(body_rows)
            .map(|(ix, row)| {
                self.tui_tree_row(ix, ix - top, row, right, tree_focused, cell, &theme, cx)
            })
            .collect();
        let menu = entity.clone();
        let tree = tui_list(
            "debug-variables",
            &entity,
            |this: &mut DebugTool| &mut this.tui_tree_scroll,
            top,
            total,
            right,
            body_rows,
            div()
                .flex()
                .flex_col()
                .children(tree_rows)
                .into_any_element(),
            cell,
            &self.tui_bars,
        )
        .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, _, cx| {
            let at = e.position;
            menu.update(cx, |this, cx| this.row_menu(None, at, cx));
        });
        let status = status.map(|(text, failed)| {
            div()
                .h(px(cell.h))
                .flex_shrink_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(if failed {
                    theme.text_dangerous
                } else {
                    theme.text_secondary
                })
                .child(SharedString::from(skin::fit_text(
                    &format!(" {text}"),
                    cols,
                )))
        });
        frame(
            div()
                .flex()
                .flex_col()
                .children(status)
                .child(
                    div()
                        .flex()
                        .child(
                            div()
                                .w(cell.cols(left))
                                .flex_shrink_0()
                                .flex()
                                .flex_col()
                                .children(chooser)
                                .child(frames_list),
                        )
                        .child(skin::vrule(cell, body_rows, theme.border, m))
                        .child(tree),
                )
                .into_any_element(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn tui_tree_row(
        &self,
        ix: usize,
        slot: usize,
        row: &TreeRow,
        cols: usize,
        focused: bool,
        cell: Cell,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let selected = ix == self.selected;
        let indent = row.depth * TUI_INDENT;
        let chevron = if row.expandable() {
            if row.expanded {
                "▾"
            } else {
                "▸"
            }
        } else {
            " "
        };
        let watch = matches!(row.kind, NodeKind::Watch(_) | NodeKind::WatchInput(_));
        let lead = indent + 1 + if watch { 2 } else { 0 } + 1;
        let room = cols.saturating_sub(lead);
        let content = if matches!(row.kind, NodeKind::WatchInput(_)) {
            div()
                .flex_1()
                .min_w_0()
                .h(px(cell.h))
                .bg(theme.input_bg)
                .child(self.watch_input.clone())
                .into_any_element()
        } else {
            let (text, runs) = row_text(row);
            let (text, runs) = clip_runs(&text, &runs, room);
            div()
                .flex_shrink_0()
                .child(styled(text, &runs, theme))
                .into_any_element()
        };
        let click = entity.clone();
        let menu = entity.clone();
        let toggle = entity.clone();
        let reference = row.reference;
        let expandable = row.expandable();
        div()
            .id(ElementId::NamedInteger("debug-node".into(), slot as u64))
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
                click.update(cx, |this, cx| this.click_row(ix, count, window, cx));
            })
            .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                let at = e.position;
                menu.update(cx, |this, cx| this.row_menu(Some(ix), at, cx));
            })
            .child(div().w(cell.cols(indent)).flex_shrink_0())
            .child(
                div()
                    .id(ElementId::NamedInteger("debug-chevron".into(), slot as u64))
                    .w(px(cell.w))
                    .flex_shrink_0()
                    .text_color(theme.icon_secondary)
                    .when(expandable, |d| {
                        d.on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            toggle.update(cx, |this, cx| this.toggle(reference, cx));
                        })
                    })
                    .child(chevron),
            )
            .when(watch, |d| {
                d.child(
                    div()
                        .w(cell.cols(2))
                        .flex_shrink_0()
                        .text_color(theme.icon_secondary)
                        .child("○ "),
                )
            })
            .child(div().w(px(cell.w)).flex_shrink_0())
            .child(content)
            .into_any_element()
    }
}

/// A session with threads, frames, scopes, children and watches, for the
/// tests of the debugger tools (no adapter behind it).
#[cfg(test)]
pub(crate) fn fake_session() -> DebugSession {
    use weep_dap::{StackFrame, Thread};
    let var = |name: &str, value: &str, ty: Option<&str>, reference: i64| Variable {
        name: name.into(),
        value: value.into(),
        type_name: ty.map(Into::into),
        variables_reference: reference,
        evaluate_name: None,
    };
    let mut s = DebugSession::new("app".into());
    s.state = SessionState::Stopped {
        reason: "breakpoint".into(),
        description: None,
    };
    s.threads = vec![
        Thread {
            id: 1,
            name: "main".into(),
        },
        Thread {
            id: 2,
            name: "worker".into(),
        },
    ];
    s.current_thread = Some(1);
    s.frames = vec![
        StackFrame {
            id: 100,
            name: "app::run".into(),
            path: Some(PathBuf::from("/w/src/main.rs")),
            line: 11,
            column: 4,
        },
        StackFrame {
            id: 101,
            name: "std::rt::lang_start".into(),
            path: None,
            line: 0,
            column: 0,
        },
    ];
    s.scopes = vec![
        Scope {
            name: "Locals".into(),
            variables_reference: 1,
            expensive: false,
        },
        Scope {
            name: "Registers".into(),
            variables_reference: 2,
            expensive: true,
        },
    ];
    s.children.insert(
        1,
        vec![
            var("count", "3", Some("i32"), 0),
            var("point", "{x:1, y:2}", Some("Point"), 10),
        ],
    );
    s.children.insert(
        10,
        vec![var("x", "1", Some("i32"), 0), var("y", "2", Some("i32"), 0)],
    );
    s.expanded.insert(1);
    s.watches = vec![
        Watch {
            expression: "count + 1".into(),
            value: Some(Ok(var("count + 1", "4", Some("i32"), 0))),
        },
        Watch {
            expression: "nope".into(),
            value: Some(Err("undeclared identifier".into())),
        },
    ];
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(rows: &[TreeRow]) -> Vec<(usize, String)> {
        rows.iter().map(|r| (r.depth, r.name.clone())).collect()
    }

    #[test]
    fn flattens_watches_then_scopes_with_expanded_children() {
        let s = fake_session();
        let rows = tree_rows(&TreeSource::of(&s), None);
        assert_eq!(
            names(&rows),
            vec![
                (0, "count + 1".into()),
                (0, "nope".into()),
                (0, "Locals".into()),
                (1, "count".into()),
                (1, "point".into()),
                (0, "Registers".into()),
            ]
        );
        assert!(rows[2].expanded && rows[2].expandable());
        assert!(rows[4].expandable() && !rows[4].expanded);
        assert!(!rows[3].expandable());
        assert!(rows[1].error);
        assert_eq!(rows[4].expression.as_deref(), Some("point"));
    }

    #[test]
    fn expanding_shows_children_and_loading() {
        let mut s = fake_session();
        s.expanded.insert(10);
        s.expanded.insert(2);
        let rows = tree_rows(&TreeSource::of(&s), None);
        assert_eq!(
            names(&rows)[3..],
            [
                (1, "count".into()),
                (1, "point".into()),
                (2, "x".into()),
                (2, "y".into()),
                (0, "Registers".into()),
                (1, "Collecting data…".into()),
            ]
        );
        assert_eq!(rows[5].expression.as_deref(), Some("point.x"));
        assert_eq!(rows[8].kind, NodeKind::Loading);
    }

    #[test]
    fn a_cycle_of_references_stops() {
        let mut s = fake_session();
        s.children.insert(
            10,
            vec![Variable {
                name: "me".into(),
                value: "{...}".into(),
                type_name: None,
                variables_reference: 10,
                evaluate_name: None,
            }],
        );
        s.expanded.insert(10);
        let rows = tree_rows(&TreeSource::of(&s), None);
        assert_eq!(rows.iter().filter(|r| r.name == "me").count(), 1);
    }

    #[test]
    fn editing_puts_the_field_in_the_tree() {
        let s = fake_session();
        let rows = tree_rows(&TreeSource::of(&s), Some(Editing::New));
        assert_eq!(rows[2].kind, NodeKind::WatchInput(None));
        let rows = tree_rows(&TreeSource::of(&s), Some(Editing::Watch(1)));
        assert_eq!(rows[1].kind, NodeKind::WatchInput(Some(1)));
        assert_eq!(rows.len(), 6);
    }

    #[test]
    fn row_text_dims_the_type() {
        let s = fake_session();
        let rows = tree_rows(&TreeSource::of(&s), None);
        let (text, runs) = row_text(&rows[3]);
        assert_eq!(text, "count = {i32} 3");
        assert_eq!(runs[2], (8..14, Tint::Type));
        let (text, runs) = row_text(&rows[1]);
        assert_eq!(text, "nope = undeclared identifier");
        assert_eq!(runs.last().unwrap().1, Tint::Error);
        let (clipped, runs) = clip_runs("count = {i32} 3", &row_text(&rows[3]).1, 10);
        assert_eq!(clipped, "count = {…");
        assert_eq!(runs.last().unwrap(), &(8..clipped.len(), Tint::Type));
    }

    #[test]
    fn child_expressions() {
        assert_eq!(child_expression(None, "a"), "a");
        assert_eq!(child_expression(Some("a"), "b"), "a.b");
        assert_eq!(child_expression(Some("v"), "[0]"), "v[0]");
        assert_eq!(watch_parent("a + b"), "(a + b)");
        assert_eq!(watch_parent("a.b[1]"), "a.b[1]");
    }

    #[test]
    fn frames_and_status() {
        let s = fake_session();
        assert_eq!(frame_location(&s.frames[0]).as_deref(), Some("main.rs:12"));
        assert_eq!(frame_location(&s.frames[1]), None);
        assert_eq!(status_text(&s.state), None);
        assert_eq!(
            status_text(&SessionState::Exited(Some(1))),
            Some(("Process finished with exit code 1".into(), false))
        );
        assert_eq!(
            status_text(&SessionState::Failed("no lldb-dap".into())),
            Some(("no lldb-dap".into(), true))
        );
        assert_eq!(single_line("a\nb"), "a b");
    }
}
