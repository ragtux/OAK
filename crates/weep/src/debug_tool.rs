//! The Debug tool (M8), as Fleet 1.48 shows it in the right panel (batch
//! 13, `docs/fleet-probe.md` twentieth pass): one column of three sections
//! split by thin lines, each under a semibold title.
//!
//! - **Processes**: the title row carries the session toolbar at its right
//!   (Resume, Pause, Restart, Stop, Step Over, Step Into, Step Out and a
//!   last box-shaped button whose function no probe found, drawn inert);
//!   below, one row per debug session, its configuration icon with a state
//!   dot and its name, the active session selected.
//! - **Threads & Frames**: a tree, a node per thread (`1: tid=3212558
//!   "main"`) with the current thread expanded to its frames, names only,
//!   the selected frame highlighted; every frame in the list's text colour,
//!   library code too (twenty-fourth pass: Fleet dims none). An exception or
//!   panic stop puts
//!   its words above the tree in `text.dangerous`. Backspace and Delete drop
//!   the selected frame (`debug/drop-frame`).
//! - **Variables**: an "Add Watch" button at the title's right; below, the
//!   watches (the inline field while one is typed), then the frame's scopes
//!   in the adapter's order, collapsed until opened, each variable `name =
//!   {type} value`.
//!
//! Without a session the tool is Fleet's empty tool (`empty_state.rs`,
//! twenty-fourth pass): the bug, "Debug", "Create new or run existing
//! configuration to debug your code" and a "Run & Debug" button with the
//! `run-and-debug` chord that dispatches it.
//!
//! Sizes are `metrics::DEBUG_*` (`theme.rs`); selected rows use the list's
//! colours (`listItem.background.focused` in the section with the keyboard,
//! `.selected` elsewhere). The view follows `Debugger::global`'s active
//! session and calls its commands (`select_thread`, `select_frame`,
//! `toggle_expanded`, the watches); the toolbar dispatches the `debug/*`
//! actions, which the workspace routes to the session. The workspace opens a
//! frame's file through [`DebugViewEvent::Open`] and shows the right-click
//! menu through [`DebugViewEvent::Menu`]. Keys (context `DebugTool`): Tab
//! moves between the sections, the arrows move and fold, Enter opens a frame
//! or folds a node or edits a watch.
//!
//! This file also holds what the three debugger tools share: the event, the
//! [`DebugViews`] the workspace keeps, the key bindings and [`Follow`].

use crate::actions::{
    DebugAddWatch, DebugDeleteWatch, DebugDropFrame, DebugEditWatch, DebugPause,
    DebugRemoveBreakpoint, DebugRestart, DebugResume, DebugStepInto, DebugStepOut, DebugStepOver,
    DebugStop, DebugToggleEnabledBreakpoint, PopupConfirm, PopupDown, PopupLeft, PopupRight,
    PopupTab, PopupUp, RunAndDebug,
};
use crate::assets::icon;
use crate::breakpoints_tool::BreakpointsTool;
use crate::debug_console::DebugConsole;
use crate::debugger::{DebugSession, DebugSessionEvent, Debugger, SessionState, Variable, Watch};
use crate::empty_state::{EmptyButton, EmptyState};
use crate::git_tool::tui_list;
use crate::input::{InputEvent, TextInput};
use crate::keymap::{action_title, hint_for};
use crate::menu::MenuEntry;
use crate::skin::{self, Cell, FrameBars, RowScroll, Skin};
use crate::theme::{metrics, Metrics, Theme};
use crate::tooltip::Tooltip;
use gpui::{
    div, prelude::*, px, Action, AnyElement, App, ClipboardItem, Context, Div, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, Hsla, KeyBinding,
    MouseButton, MouseDownEvent, Pixels, Point, Render, ScrollHandle, SharedString, StyledText,
    Subscription, Window,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use weep_dap::{Scope, StackFrame, Thread};

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

/// The three debugger tools a workspace shows.
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
/// active one and redraws the view whenever that session (or the list of
/// sessions) changes.
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

/// A section's title in the Fleet skin: 13 px semibold `text.primary`.
pub fn section_title(text: &'static str, theme: &Theme, m: &Metrics) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .text_size(px(m.ui_font_size))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.text_primary)
        .child(text)
}

// ---- Processes: the toolbar --------------------------------------------------

/// One of the Processes toolbar's buttons, left to right as batch 13 shows
/// them (the floating debug panel shows the same row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarButton {
    Resume,
    Pause,
    Restart,
    Stop,
    StepOver,
    StepInto,
    StepOut,
    /// The last button: a box with a bar along its bottom (an output or
    /// layout toggle by its look; the floating panel's `Output`). No probe
    /// clicked it: drawn, inert.
    Output,
}

impl ToolbarButton {
    pub const ALL: [ToolbarButton; 8] = [
        ToolbarButton::Resume,
        ToolbarButton::Pause,
        ToolbarButton::Restart,
        ToolbarButton::Stop,
        ToolbarButton::StepOver,
        ToolbarButton::StepInto,
        ToolbarButton::StepOut,
        ToolbarButton::Output,
    ];

    /// Fleet's action id, for the tooltip's title and chord.
    pub fn action_id(self) -> Option<&'static str> {
        match self {
            ToolbarButton::Resume => Some("debug/resume"),
            ToolbarButton::Pause => Some("debug/pause"),
            ToolbarButton::Stop => Some("debug/stop"),
            ToolbarButton::StepOver => Some("debug/step-over"),
            ToolbarButton::StepInto => Some("debug/step-into"),
            ToolbarButton::StepOut => Some("debug/step-out"),
            // Fleet's action list has no restart id (tools/fleet-probe).
            ToolbarButton::Restart | ToolbarButton::Output => None,
        }
    }

    /// The tooltip: the action's title (estimate for the two without an id).
    pub fn title(self) -> String {
        match (self, self.action_id()) {
            (_, Some(id)) => action_title(id),
            (ToolbarButton::Restart, None) => "Restart".into(),
            _ => String::new(),
        }
    }

    /// The action the button dispatches; `None` draws it inert.
    pub fn action(self) -> Option<Box<dyn Action>> {
        match self {
            ToolbarButton::Resume => Some(Box::new(DebugResume)),
            ToolbarButton::Pause => Some(Box::new(DebugPause)),
            ToolbarButton::Restart => Some(Box::new(DebugRestart)),
            ToolbarButton::Stop => Some(Box::new(DebugStop)),
            ToolbarButton::StepOver => Some(Box::new(DebugStepOver)),
            ToolbarButton::StepInto => Some(Box::new(DebugStepInto)),
            ToolbarButton::StepOut => Some(Box::new(DebugStepOut)),
            ToolbarButton::Output => None,
        }
    }

    /// The floating debug panel's icons (`debug_panel.rs`): expui's resume
    /// and stop, WEEP's own drawings of batch 13's pause, restart, step
    /// over, the arrows and the last box.
    pub fn icon(self) -> &'static str {
        match self {
            ToolbarButton::Resume => "expui/run/resume",
            ToolbarButton::Pause => "weep/debug-pause",
            ToolbarButton::Restart => "weep/debug-restart",
            ToolbarButton::Stop => "expui/run/stop",
            ToolbarButton::StepOver => "weep/debug-step-over",
            ToolbarButton::StepInto => "weep/arrow-down",
            ToolbarButton::StepOut => "weep/arrow-up",
            ToolbarButton::Output => "weep/debug-output",
        }
    }

    /// The tui skin's one-cell mark, as the floating panel's (each in
    /// JetBrains Mono; the font has no circular arrow, so Restart is `↩`).
    pub fn glyph(self) -> &'static str {
        match self {
            ToolbarButton::Resume => "▶",
            ToolbarButton::Pause => "‖",
            ToolbarButton::Restart => "↩",
            ToolbarButton::Stop => "■",
            ToolbarButton::StepOver => "↪",
            ToolbarButton::StepInto => "↓",
            ToolbarButton::StepOut => "↑",
            ToolbarButton::Output => "□",
        }
    }

    /// Whether the session's state allows the button (batch 13, stopped:
    /// Pause greyed, the rest live; the other states are estimates, as the
    /// floating panel's): Resume and the steps while stopped, Pause while
    /// running, Restart and Stop until the session is over.
    pub fn enabled(self, state: &SessionState) -> bool {
        match self {
            ToolbarButton::Resume
            | ToolbarButton::StepOver
            | ToolbarButton::StepInto
            | ToolbarButton::StepOut => state.is_stopped(),
            ToolbarButton::Pause => *state == SessionState::Running,
            ToolbarButton::Stop | ToolbarButton::Restart => !state.is_over(),
            ToolbarButton::Output => true,
        }
    }

    /// Its colour when enabled: Resume `icon.background.green`, Stop
    /// `icon.background.red` (both as the gutter's run icon and breakpoint,
    /// batch 13), the rest `icon.primary`; disabled `text.disabled`.
    pub fn color(self, enabled: bool, theme: &Theme) -> Hsla {
        match self {
            _ if !enabled => theme.text_disabled,
            ToolbarButton::Resume => theme.icon_green,
            ToolbarButton::Stop => theme.icon_red,
            _ => theme.icon_primary,
        }
    }

    fn tooltip(self, cx: &App) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
        let title = self.title();
        let hint = self.action_id().and_then(|id| hint_for(cx, id));
        move |window, cx| match &hint {
            Some(h) => Tooltip::with_shortcut(title.clone(), h.clone())(window, cx),
            None => Tooltip::text(title.clone())(window, cx),
        }
    }
}

/// The colour of a session's state dot: red while stopped (batch 13),
/// green while running, red again when it ended badly
/// (`DebugSession::failed`, as the run tab's error badge), none otherwise
/// (estimates but the first).
fn state_dot(session: &DebugSession, theme: &Theme) -> Option<Hsla> {
    match session.state {
        SessionState::Stopped { .. } => Some(theme.icon_red),
        SessionState::Running => Some(theme.icon_green),
        _ if session.failed() => Some(theme.icon_red),
        _ => None,
    }
}

/// A session's icon: Fleet's shows the configuration's (a Rust gear for the
/// `cargo` row). The session does not keep its configuration's type, so a
/// launched one shows the Rust monogram (estimate) and an attached one the
/// debug icon (estimate).
fn session_icon(session: &DebugSession) -> &'static str {
    if session.attached {
        "expui/run/debug"
    } else {
        "weep/file-rust"
    }
}

// ---- Threads & Frames ---------------------------------------------------------

/// A thread node's label as Fleet prints it: `1: tid=3212558 "main"`, the
/// 1-based position, the adapter's id (lldb-dap's is the system tid) and
/// the name in quotes; lldb-dap's name for an unnamed thread (`Thread 2`)
/// is left out.
pub fn thread_label(index: usize, thread: &Thread) -> String {
    let name = thread.name.trim();
    let unnamed = name.is_empty()
        || name.strip_prefix("Thread ").is_some_and(|n| {
            let n = n.trim_start_matches('#');
            !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
        });
    if unnamed {
        format!("{}: tid={}", index + 1, thread.id)
    } else {
        format!("{}: tid={} \"{name}\"", index + 1, thread.id)
    }
}

/// One row of the Threads & Frames tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameRow {
    Thread {
        id: i64,
        label: String,
        expanded: bool,
    },
    /// A frame of the current thread, by its index in the session's frames;
    /// `depth` 0 only when the adapter listed no threads.
    Frame {
        index: usize,
        name: String,
        depth: usize,
    },
}

/// The tree flattened: a node per thread, the current one (the only one
/// whose frames the session holds) expanded unless collapsed by hand.
pub fn frame_rows(
    threads: &[Thread],
    current: Option<i64>,
    frames: &[StackFrame],
    collapsed: &HashSet<i64>,
) -> Vec<FrameRow> {
    let frame_row = |(index, f): (usize, &StackFrame), depth| FrameRow::Frame {
        index,
        name: f.name.clone(),
        depth,
    };
    let mut rows = Vec::new();
    let mut shown = false;
    for (ix, t) in threads.iter().enumerate() {
        let is_current = Some(t.id) == current;
        let expanded = is_current && !collapsed.contains(&t.id);
        rows.push(FrameRow::Thread {
            id: t.id,
            label: thread_label(ix, t),
            expanded,
        });
        if is_current {
            shown = true;
            if expanded {
                rows.extend(frames.iter().enumerate().map(|f| frame_row(f, 1)));
            }
        }
    }
    if !shown {
        rows.extend(frames.iter().enumerate().map(|f| frame_row(f, 0)));
    }
    rows
}

/// The highlighted row: the thread the keyboard or a click put the
/// selection on, else the current frame, else the current thread.
pub fn frames_selected(
    rows: &[FrameRow],
    cursor: Option<i64>,
    current_thread: Option<i64>,
    current_frame: usize,
) -> Option<usize> {
    let thread = |id| {
        rows.iter()
            .position(|r| matches!(r, FrameRow::Thread { id: t, .. } if *t == id))
    };
    cursor
        .and_then(thread)
        .or_else(|| {
            rows.iter()
                .position(|r| matches!(r, FrameRow::Frame { index, .. } if *index == current_frame))
        })
        .or_else(|| current_thread.and_then(thread))
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
/// The scopes keep the adapter's names and order: lldb-dap's "Locals",
/// "Globals", "Registers" where Fleet's own adapter (codelldb) shows
/// "Local", "Static", "Global", "Registers" (batch 13); lldb-dap's
/// "Globals" holds the statics too, so no faithful one-to-one renaming
/// exists.
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

/// A row's text and the byte ranges of its parts: a variable's
/// `name: {type} value`, a watch's `expr = {type} value` (batch 13,
/// debug-fleet-variables-expanded and -watch: the colon right after the
/// name, the `=` between spaces, one plain space before the type; the
/// colon and the `=` in the name's colour, `text.primary`, the type in
/// `text.secondary`). Both skins print this text.
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
            let separator = row_separator(&row.kind);
            match &row.value {
                Some(value) => {
                    push(&mut text, separator, Tint::Punctuation);
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
                None => push(&mut text, &format!("{separator}not available"), Tint::Dim),
            }
        }
    }
    (text, runs)
}

/// What stands between a row's name and its value: a variable's colon
/// (`total: {i32} 14`), a watch's ` = ` (`total * 2 = {long long} 28`).
fn row_separator(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Watch(_) => " = ",
        _ => ": ",
    }
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

/// The words Threads & Frames shows while the debuggee is not stopped, and
/// whether they report a failure (`DebugSession::failed`); while stopped,
/// only an exception's or a panic's words (`DebugSession::stop_text`, in
/// the failure's colour), else `None` (the frames say it).
pub fn status_text(session: &DebugSession) -> Option<(String, bool)> {
    let text = match &session.state {
        // Estimate: the words IntelliJ's debugger prints while connecting.
        SessionState::Starting => "Starting…".to_string(),
        SessionState::Running => "Running".into(),
        SessionState::Stopped { .. } => return session.stop_text().map(|t| (t, true)),
        SessionState::Exited(Some(code)) => format!("Process finished with exit code {code}"),
        SessionState::Exited(None) => "Process finished".into(),
        SessionState::Failed(e) => e.clone(),
    };
    Some((text, session.failed()))
}

// ---- the view ---------------------------------------------------------------

/// The tree's indent per level in the tui skin, in cells (estimate: the
/// Files tool's guides take two cells a level).
const TUI_INDENT: usize = 2;

/// The three sections, in Tab's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Processes,
    Threads,
    Variables,
}

impl Part {
    fn next(self) -> Part {
        match self {
            Part::Processes => Part::Threads,
            Part::Threads => Part::Variables,
            Part::Variables => Part::Processes,
        }
    }
}

pub struct DebugTool {
    follow: Follow,
    part: Part,
    /// The selected row of the variables tree.
    selected: usize,
    /// A thread node selected in Threads & Frames (else the current frame).
    thread_cursor: Option<i64>,
    /// The current thread's node, folded by hand.
    collapsed: HashSet<i64>,
    editing: Option<Editing>,
    watch_input: Entity<TextInput>,
    reveal: bool,
    focus_handle: FocusHandle,
    frames_scroll: ScrollHandle,
    /// The frame row last drawn as selected, to scroll a new one into view.
    shown_frame: Option<usize>,
    tree_scroll: ScrollHandle,
    tui_grid: Rc<std::cell::Cell<(usize, usize)>>,
    tui_bars: FrameBars,
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
            // Batch 13 (debug-fleet-stopped): the keyboard starts in Variables,
            // the first row (`Local`) in `listItem.background.focused` and the
            // frame row only selected.
            part: Part::Variables,
            selected: 0,
            thread_cursor: None,
            collapsed: HashSet::new(),
            editing: None,
            watch_input,
            reveal: false,
            focus_handle: cx.focus_handle(),
            frames_scroll: ScrollHandle::new(),
            shown_frame: None,
            tree_scroll: ScrollHandle::new(),
            tui_grid: Default::default(),
            tui_bars: Default::default(),
            tui_frames_scroll: RowScroll::default(),
            tui_tree_scroll: RowScroll::default(),
        }
    }

    fn switched(&mut self, _cx: &mut Context<Self>) {
        // A new session starts in Variables, as the first one does.
        self.part = Part::Variables;
        self.selected = 0;
        self.thread_cursor = None;
        self.collapsed.clear();
        self.editing = None;
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

    /// The session the tool draws: none once every session has ended, so
    /// the tool goes back to its empty state as Fleet's does when the
    /// debuggee exits (twenty-fourth pass, rest13: the Processes list
    /// emptied and the empty Debug tool came back).
    fn shown_session(&self, cx: &App) -> Option<Entity<DebugSession>> {
        let session = self.session()?;
        let any_live = Debugger::global(cx)
            .read(cx)
            .sessions()
            .iter()
            .any(|s| !s.read(cx).state.is_over());
        any_live.then_some(session)
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

    /// Threads & Frames' rows and the highlighted one.
    fn frame_rows(&self, cx: &App) -> (Vec<FrameRow>, Option<usize>) {
        let Some(session) = &self.follow.session else {
            return (Vec::new(), None);
        };
        let s = session.read(cx);
        let rows = frame_rows(&s.threads, s.current_thread, &s.frames, &self.collapsed);
        let selected =
            frames_selected(&rows, self.thread_cursor, s.current_thread, s.current_frame);
        (rows, selected)
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
        let next = |ix: usize, n: usize| {
            if forward {
                (ix + 1).min(n.saturating_sub(1))
            } else {
                ix.saturating_sub(1)
            }
        };
        match self.part {
            Part::Processes => {
                let debugger = Debugger::global(cx);
                let d = debugger.read(cx);
                let n = d.sessions().len();
                let active = self
                    .session()
                    .and_then(|a| d.sessions().iter().position(|s| *s == a));
                if let Some(ix) = active {
                    let to = next(ix, n);
                    if to != ix {
                        debugger.update(cx, |d, cx| d.set_active(to, cx));
                    }
                }
            }
            Part::Threads => {
                let (rows, selected) = self.frame_rows(cx);
                if rows.is_empty() {
                    return;
                }
                let to = match selected {
                    Some(ix) => next(ix, rows.len()),
                    None => 0,
                };
                self.select_frame_row(&rows[to], cx);
                self.reveal = true;
                cx.notify();
            }
            Part::Variables => {
                let n = self.rows(cx).len();
                if n == 0 {
                    return;
                }
                self.selected = next(self.selected, n);
                self.reveal = true;
                cx.notify();
            }
        }
    }

    /// Puts the selection on a Threads & Frames row: a thread node, or a
    /// frame (which becomes the session's selected frame).
    fn select_frame_row(&mut self, row: &FrameRow, cx: &mut Context<Self>) {
        match row {
            FrameRow::Thread { id, .. } => self.thread_cursor = Some(*id),
            FrameRow::Frame { index, .. } => {
                self.thread_cursor = None;
                if let Some(session) = self.session() {
                    let index = *index;
                    if session.read(cx).current_frame != index {
                        session.update(cx, |s, cx| s.select_frame(index, cx));
                    }
                }
            }
        }
    }

    /// Folds or unfolds a thread node; unfolding another thread makes it the
    /// current one (its frames are fetched).
    fn toggle_thread(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        let current = session.read(cx).current_thread == Some(id);
        if current && !self.collapsed.contains(&id) {
            self.collapsed.insert(id);
        } else {
            self.collapsed.remove(&id);
            if !current {
                session.update(cx, |s, cx| s.select_thread(id, cx));
            }
        }
        cx.notify();
    }

    fn left(&mut self, _: &PopupLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        match self.part {
            Part::Processes => {}
            Part::Threads => {
                let (rows, selected) = self.frame_rows(cx);
                match selected.and_then(|ix| rows.get(ix)) {
                    Some(FrameRow::Thread { id, expanded, .. }) => {
                        if *expanded {
                            self.toggle_thread(*id, cx);
                        }
                    }
                    Some(FrameRow::Frame { depth: 1, .. }) => {
                        self.thread_cursor = self.session().and_then(|s| s.read(cx).current_thread);
                        self.reveal = true;
                        cx.notify();
                    }
                    _ => {}
                }
            }
            Part::Variables => {
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
                }
            }
        }
    }

    fn right(&mut self, _: &PopupRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        match self.part {
            Part::Processes => {}
            Part::Threads => {
                let (rows, selected) = self.frame_rows(cx);
                let Some(ix) = selected else {
                    return;
                };
                if let Some(FrameRow::Thread { id, expanded, .. }) = rows.get(ix) {
                    if !expanded {
                        self.toggle_thread(*id, cx);
                    } else if let Some(child @ FrameRow::Frame { .. }) = rows.get(ix + 1) {
                        let child = child.clone();
                        self.select_frame_row(&child, cx);
                        self.reveal = true;
                        cx.notify();
                    }
                }
            }
            Part::Variables => {
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
        }
    }

    fn tab(&mut self, _: &PopupTab, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        self.part = self.part.next();
        self.reveal = true;
        cx.notify();
    }

    fn confirm(&mut self, _: &PopupConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        match self.part {
            Part::Processes => {}
            Part::Threads => {
                let (rows, selected) = self.frame_rows(cx);
                match selected.and_then(|ix| rows.get(ix)) {
                    Some(FrameRow::Thread { id, .. }) => self.toggle_thread(*id, cx),
                    Some(FrameRow::Frame { index, .. }) => self.open_frame(*index, cx),
                    None => {}
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
        self.thread_cursor = None;
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

    fn delete_watch(&mut self, _: &DebugDeleteWatch, window: &mut Window, cx: &mut Context<Self>) {
        // Fleet binds Backspace and Delete to both: in the frames list
        // they drop the frame.
        if self.editing.is_none() && self.part == Part::Threads {
            window.dispatch_action(DebugDropFrame.boxed_clone(), cx);
            return;
        }
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

    /// `debug/drop-frame`: in the watches it is Delete Watch (the same keys);
    /// in Threads & Frames the workspace drops the session's selected frame
    /// (the list keeps the session's selection), saying why it cannot.
    fn drop_frame(&mut self, _: &DebugDropFrame, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        match self.part {
            Part::Variables => self.delete_watch(&DebugDeleteWatch, window, cx),
            Part::Threads => cx.propagate(),
            // The sessions' list has no frame under the keys.
            Part::Processes => {}
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

    fn click_frame_row(
        &mut self,
        row: FrameRow,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        self.part = Part::Threads;
        match row {
            FrameRow::Frame { index, .. } => self.open_frame(index, cx),
            FrameRow::Thread { id, .. } => {
                self.thread_cursor = Some(id);
                if count == 2 {
                    self.toggle_thread(id, cx);
                }
            }
        }
        cx.notify();
    }

    fn click_session(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.part = Part::Processes;
        Debugger::global(cx).update(cx, |d, cx| d.set_active(ix, cx));
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
}

/// The tool without a session (twenty-fourth pass, `rest13-fleet-after.png`):
/// Fleet's bug, "Debug", its hint and a "Run & Debug" button with the
/// `run-and-debug` chord that runs it.
fn no_session(cx: &App) -> EmptyState {
    EmptyState {
        icon: "weep/bug",
        glyph: "¤",
        title: "Debug",
        description: "Create new or run existing configuration to debug your code",
        button: Some(EmptyButton {
            id: "debug-run-and-debug",
            label: "Run & Debug",
            chord: hint_for(cx, "run-and-debug"),
            on_click: Rc::new(|window, cx| window.dispatch_action(Box::new(RunAndDebug), cx)),
        }),
    }
}

fn tint_color(tint: Tint, theme: &Theme) -> Hsla {
    match tint {
        Tint::Name => theme.list_text,
        // Batch 13: the colon and the `=` in the name's colour.
        Tint::Punctuation => theme.list_text,
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

/// A selectable row's wash in the Fleet skin: 24 tall in its 26 pitch,
/// focused or selected or hovered.
fn fleet_wash(selected: bool, focused: bool, theme: &Theme, m: &Metrics) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .rounded(px(m.tree_row_radius))
        .whitespace_nowrap()
        .overflow_hidden()
        .cursor_pointer()
        .when(selected && focused, |d| d.bg(theme.list_bg_focused))
        .when(selected && !focused, |d| d.bg(theme.list_bg_selected))
        .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
}

/// A tree row's chevron box: the chevron when `expanded` says so, else an
/// empty box keeping the column.
fn fleet_chevron(expanded: Option<bool>, theme: &Theme, m: &Metrics) -> Div {
    let d = div().size(px(m.icon_size)).flex_shrink_0();
    match expanded {
        Some(open) => d.child(icon(
            if open {
                "expui/general/chevronDown"
            } else {
                "expui/general/chevronRight"
            },
            m.icon_size,
            // Batch 13: the chevrons are `icon.primary`.
            theme.icon_primary,
        )),
        None => d,
    }
}

impl Render for DebugTool {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A restarted session (the same entity, back to Starting with its
        // frames emptied) starts the view's selections over.
        if self
            .session()
            .is_some_and(|s| s.read(cx).state == SessionState::Starting)
        {
            self.thread_cursor = None;
            self.collapsed.clear();
        }
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
            .on_action(cx.listener(Self::drop_frame))
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
        let focused = self.focus_handle.contains_focused(window, cx);
        let Some(session) = self.shown_session(cx) else {
            return no_session(cx).fleet(&theme, &m);
        };
        let processes = self.fleet_processes(&session, focused, &theme, &m, cx);
        let threads = self.fleet_threads(&session, focused, &theme, &m, cx);
        let variables = self.fleet_variables(rows, focused, &theme, &m, cx);
        self.reveal = false;
        let splitter = || {
            div()
                .flex_shrink_0()
                .h(px(m.debug_splitter))
                .mx(px(m.panel_pad_x))
                .bg(theme.background_primary)
        };
        let share = metrics::DEBUG_FRAMES_SHARE;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(processes)
            .child(splitter())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_basis(px(0.))
                            .min_h_0()
                            .overflow_hidden()
                            .map(|mut d| {
                                d.style().flex_grow = Some(share);
                                d
                            })
                            .child(threads),
                    )
                    .child(splitter())
                    .child(
                        div()
                            .flex_basis(px(0.))
                            .min_h_0()
                            .overflow_hidden()
                            .map(|mut d| {
                                d.style().flex_grow = Some(1. - share);
                                d
                            })
                            .child(variables),
                    ),
            )
            .into_any_element()
    }

    /// A section's title row, `right` at its right end.
    fn fleet_header(
        title: &'static str,
        right: Option<AnyElement>,
        theme: &Theme,
        m: &Metrics,
    ) -> Div {
        div()
            .flex_shrink_0()
            .h(px(m.debug_section_header))
            .mx(px(m.panel_pad_x))
            .pl(px(m.debug_section_pad_x))
            .flex()
            .items_center()
            .child(section_title(title, theme, m))
            .child(div().flex_1())
            .children(right)
    }

    fn fleet_processes(
        &self,
        active: &Entity<DebugSession>,
        focused: bool,
        theme: &Theme,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let state = active.read(cx).state.clone();
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(m.debug_toolbar_gap))
            .pr(px(m.debug_section_pad_x))
            .children(ToolbarButton::ALL.into_iter().map(|button| {
                let enabled = button.enabled(&state);
                let action = button.action();
                let live = enabled && action.is_some();
                div()
                    .id(SharedString::from(format!("debug-toolbar-{button:?}")))
                    .size(px(m.debug_toolbar_button))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(m.button_radius))
                    .child(icon(
                        button.icon(),
                        m.icon_size,
                        button.color(enabled, theme),
                    ))
                    .when(button != ToolbarButton::Output, |d| {
                        d.tooltip(button.tooltip(cx))
                    })
                    .when_some(action.filter(|_| live), |d, action| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme.ghost_hovered))
                            .active(|s| s.bg(theme.ghost_pressed))
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(action.boxed_clone(), cx)
                            })
                    })
                    .into_any_element()
            }));
        let debugger = Debugger::global(cx);
        let sessions: Vec<Entity<DebugSession>> = debugger.read(cx).sessions().to_vec();
        let part_focused = focused && self.part == Part::Processes;
        let rows: Vec<AnyElement> = sessions
            .iter()
            .enumerate()
            .map(|(ix, s)| {
                let selected = s == active;
                let s = s.read(cx);
                let dot = state_dot(s, theme).map(|color| {
                    div()
                        .absolute()
                        .right_0()
                        .bottom_0()
                        .size(px(m.debug_badge))
                        .rounded_full()
                        .bg(color)
                });
                let click = entity.clone();
                div()
                    .id(ElementId::NamedInteger("debug-session".into(), ix as u64))
                    .h(px(m.tree_row_height))
                    .flex_shrink_0()
                    .py(px(m.tree_row_inset))
                    .on_click(move |_, window, cx| {
                        click.update(cx, |this, cx| this.click_session(ix, window, cx));
                    })
                    .child(
                        fleet_wash(selected, part_focused, theme, m)
                            .pl(px(m.debug_row_pad))
                            .gap(px(m.debug_chevron_gap))
                            .child(
                                div()
                                    .relative()
                                    .size(px(m.icon_size))
                                    .flex_shrink_0()
                                    .child(icon(session_icon(s), m.icon_size, theme.icon_primary))
                                    .children(dot),
                            )
                            .child(
                                div()
                                    .text_color(theme.list_text)
                                    .child(SharedString::from(s.name.clone())),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .pt(px(m.debug_section_top))
            .pb(px(m.debug_processes_bottom))
            .child(Self::fleet_header(
                "Processes",
                Some(toolbar.into_any_element()),
                theme,
                m,
            ))
            .child(div().px(px(m.panel_pad_x)).flex().flex_col().children(rows))
            .into_any_element()
    }

    fn fleet_threads(
        &mut self,
        session: &Entity<DebugSession>,
        focused: bool,
        theme: &Theme,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let (rows, selected) = self.frame_rows(cx);
        let status = status_text(session.read(cx));
        let part_focused = focused && self.part == Part::Threads;
        // A new selection (a stop that picks the user's frame under the
        // library's) is brought into view wherever the focus is.
        let moved = std::mem::replace(&mut self.shown_frame, selected) != selected;
        if (self.reveal && self.part == Part::Threads) || moved {
            if let Some(ix) = selected {
                // The status row, when shown, comes first.
                self.frames_scroll
                    .scroll_to_item(ix + usize::from(status.is_some()));
            }
        }
        let text_x = m.debug_row_pad + m.icon_size + m.debug_chevron_gap;
        let status = status.map(|(text, failed)| {
            div()
                .h(px(m.tree_row_height))
                .flex_shrink_0()
                .pl(px(text_x))
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
                .into_any_element()
        });
        let list: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(ix, row)| {
                let is_selected = selected == Some(ix);
                let (depth, chevron, label) = match &row {
                    FrameRow::Thread {
                        label, expanded, ..
                    } => (0, Some(*expanded), label.clone()),
                    FrameRow::Frame { name, depth, .. } => (*depth, None, name.clone()),
                };
                let click = entity.clone();
                let fold = entity.clone();
                let thread = match &row {
                    FrameRow::Thread { id, .. } => Some(*id),
                    FrameRow::Frame { .. } => None,
                };
                let chevron = fleet_chevron(chevron, theme, m);
                let chevron = match thread {
                    Some(id) => chevron
                        .id(ElementId::NamedInteger(
                            "debug-thread-chevron".into(),
                            ix as u64,
                        ))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            fold.update(cx, |this, cx| this.toggle_thread(id, cx));
                        })
                        .into_any_element(),
                    None => chevron.into_any_element(),
                };
                div()
                    .id(ElementId::NamedInteger("debug-frame".into(), ix as u64))
                    .h(px(m.tree_row_height))
                    .flex_shrink_0()
                    .py(px(m.tree_row_inset))
                    .on_click(move |e, window, cx| {
                        let count = e.click_count();
                        let row = row.clone();
                        click.update(cx, |this, cx| this.click_frame_row(row, count, window, cx));
                    })
                    .child(
                        fleet_wash(is_selected, part_focused, theme, m)
                            .pl(px(m.debug_row_pad + depth as f32 * m.debug_indent))
                            .gap(px(m.debug_chevron_gap))
                            .child(chevron)
                            // Batch 13: a frame's name runs to the wash's
                            // edge and is cut there, no ellipsis; library
                            // frames are not dimmed (twenty-fourth pass).
                            .child(
                                div()
                                    .text_color(theme.list_text)
                                    .child(SharedString::from(label)),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(m.debug_splitter_gap + m.debug_section_top))
            .child(Self::fleet_header("Threads & Frames", None, theme, m))
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
                    .children(status)
                    .children(list),
            )
            .into_any_element()
    }

    fn fleet_variables(
        &mut self,
        rows: Vec<TreeRow>,
        focused: bool,
        theme: &Theme,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        if self.reveal && self.part == Part::Variables {
            self.tree_scroll.scroll_to_item(self.selected);
        }
        let part_focused = focused && self.part == Part::Variables;
        let tree_rows: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .map(|(ix, row)| self.fleet_tree_row(ix, row, part_focused, theme, m, cx))
            .collect();
        let add = entity.clone();
        // Batch 13: a `button.secondary` 24 tall, its label 8 in each side.
        let add_watch = div()
            .id("debug-add-watch")
            .h(px(m.button_height))
            .mr(px(m.debug_add_watch_right))
            .px(px(m.debug_add_watch_pad_x))
            .flex()
            .items_center()
            .flex_shrink_0()
            .whitespace_nowrap()
            .rounded(px(m.button_radius))
            .bg(theme.button_secondary_bg)
            .text_color(theme.button_secondary_text)
            .cursor_pointer()
            .hover(|s| s.bg(theme.button_secondary_bg_hovered))
            .active(|s| s.bg(theme.button_secondary_bg_pressed))
            .on_click(move |_, window, cx| {
                add.update(cx, |this, cx| this.start_editing(Editing::New, window, cx));
            })
            .child("Add Watch");
        let menu = entity.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(m.debug_splitter_gap + m.debug_section_top))
            .child(Self::fleet_header(
                "Variables",
                Some(add_watch.into_any_element()),
                theme,
                m,
            ))
            .child(
                div()
                    .id("debug-variables")
                    .flex_1()
                    .min_h_0()
                    .px(px(m.panel_pad_x))
                    .overflow_y_scroll()
                    .track_scroll(&self.tree_scroll)
                    .flex()
                    .flex_col()
                    .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, _, cx| {
                        let at = e.position;
                        menu.update(cx, |this, cx| this.row_menu(None, at, cx));
                    })
                    .children(tree_rows),
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
            fleet_chevron(Some(row.expanded), theme, m)
                .id(ElementId::NamedInteger("debug-chevron".into(), ix as u64))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    toggle.update(cx, |this, cx| this.toggle(reference, cx));
                })
                .into_any_element()
        } else {
            fleet_chevron(None, theme, m).into_any_element()
        };
        // Estimate: a watch is marked by WEEP's glasses (Fleet's watch rows
        // were not reached by batch 13).
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
                fleet_wash(selected, focused, theme, m)
                    .pl(px(m.debug_row_pad + row.depth as f32 * m.debug_indent))
                    .pr(px(m.debug_row_pad))
                    .gap(px(m.debug_chevron_gap))
                    .child(chevron)
                    .children(watch)
                    .child(content),
            )
            .into_any_element()
    }

    /// The tool in the tui skin, the Fleet form's column on whole rows: a
    /// section rule per section with its title set in (`── Processes ──`),
    /// the toolbar ` ▶  ‖  ↩  ■  ↪  ↓  ↑  □ ` and `[ Add Watch ]` set into
    /// the rule's right end; a session row `● name` (the dot in the state's
    /// colour); the threads `▾ 1: tid=… "main"` with their frames two cells
    /// further in; the variables tree with `▸ ▾` chevrons and `○` watches,
    /// two cells a level. Threads & Frames takes Fleet's share of the rows
    /// under Processes; each list scrolls on its own, its bar on the
    /// island's right frame line.
    fn render_tui(
        &mut self,
        rows: Vec<TreeRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.tui_bars.clear();
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
        let Some(session) = self.shown_session(cx) else {
            return frame(no_session(cx).tui(cell, cols, grid_rows, &theme));
        };
        // A section rule with its title, `right` set into its right end.
        let heading = |title: &'static str, right: Option<AnyElement>| {
            div()
                .relative()
                .flex_shrink_0()
                .child(skin::section(
                    cell,
                    cols,
                    vec![skin::title(title, theme.text_primary)],
                    theme.border,
                    theme.island_background,
                    m,
                ))
                .children(right.map(|r| {
                    div()
                        .absolute()
                        .top_0()
                        .right(cell.cols(1))
                        .h(px(cell.h))
                        .flex()
                        .bg(theme.island_background)
                        .child(r)
                }))
        };
        // ---- Processes ----
        let state = session.read(cx).state.clone();
        let toolbar = div()
            .flex()
            .children(ToolbarButton::ALL.into_iter().map(|button| {
                let enabled = button.enabled(&state);
                let action = button.action().filter(|_| enabled);
                div()
                    .id(SharedString::from(format!("debug-toolbar-{button:?}")))
                    .w(cell.cols(3))
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_color(button.color(enabled, &theme))
                    .child(SharedString::from(format!(" {} ", button.glyph())))
                    .when(button != ToolbarButton::Output, |d| {
                        d.tooltip(button.tooltip(cx))
                    })
                    .when_some(action, |d, action| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme.list_bg_hovered))
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(action.boxed_clone(), cx)
                            })
                    })
                    .into_any_element()
            }));
        let sessions: Vec<Entity<DebugSession>> = Debugger::global(cx).read(cx).sessions().to_vec();
        // Processes keeps at most a quarter of the rows (estimate).
        let session_rows = sessions.len().min((grid_rows / 4).max(1));
        let processes_focused = focused && self.part == Part::Processes;
        let active_ix = sessions.iter().position(|s| *s == session).unwrap_or(0);
        let first = (active_ix + 1).saturating_sub(session_rows);
        let session_list: Vec<AnyElement> = sessions
            .iter()
            .enumerate()
            .skip(first)
            .take(session_rows)
            .map(|(ix, s)| {
                let selected = *s == session;
                let s = s.read(cx);
                let dot = state_dot(s, &theme).unwrap_or(theme.text_tertiary);
                let click = entity.clone();
                div()
                    .id(ElementId::NamedInteger("debug-session".into(), ix as u64))
                    .h(px(cell.h))
                    .w(cell.cols(cols))
                    .flex_shrink_0()
                    .flex()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .cursor_pointer()
                    .when(selected && processes_focused, |d| {
                        d.bg(theme.list_bg_focused)
                    })
                    .when(selected && !processes_focused, |d| {
                        d.bg(theme.list_bg_selected)
                    })
                    .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .on_click(move |_, window, cx| {
                        click.update(cx, |this, cx| this.click_session(ix, window, cx));
                    })
                    .child(div().w(px(cell.w)).flex_shrink_0())
                    .child(
                        div()
                            .w(cell.cols(2))
                            .flex_shrink_0()
                            .text_color(dot)
                            .child("● "),
                    )
                    .child(
                        div()
                            .text_color(theme.list_text)
                            .child(skin::fit_text(&s.name, cols.saturating_sub(3))),
                    )
                    .into_any_element()
            })
            .collect();
        let processes_h = 1 + session_rows;
        // ---- the two lists' heights ----
        let rest = grid_rows.saturating_sub(processes_h);
        let threads_h = ((rest as f32 * metrics::DEBUG_FRAMES_SHARE).round() as usize)
            .max(2)
            .min(rest);
        let variables_h = rest - threads_h;
        let threads_visible = threads_h.saturating_sub(1);
        let tree_visible = variables_h.saturating_sub(1);
        // ---- Threads & Frames ----
        let (frame_rows, frame_selected) = self.frame_rows(cx);
        let status = status_text(session.read(cx));
        let status_rows = usize::from(status.is_some());
        let frames_total = frame_rows.len() + status_rows;
        let moved = std::mem::replace(&mut self.shown_frame, frame_selected) != frame_selected;
        if (self.part == Part::Threads && self.reveal) || moved {
            if let Some(ix) = frame_selected {
                self.tui_frames_scroll
                    .follow(ix + status_rows, frames_total, threads_visible);
            }
        }
        self.tui_frames_scroll.top = self
            .tui_frames_scroll
            .top
            .min(frames_total.saturating_sub(threads_visible));
        let frames_top = self.tui_frames_scroll.top;
        let threads_focused = focused && self.part == Part::Threads;
        let mut frame_els: Vec<AnyElement> = Vec::new();
        if let Some((text, failed)) = status {
            if frames_top == 0 {
                frame_els.push(
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
                            &format!("   {text}"),
                            cols,
                        )))
                        .into_any_element(),
                );
            }
        }
        let skip = frames_top.saturating_sub(status_rows);
        let take = threads_visible.saturating_sub(frame_els.len());
        for (ix, row) in frame_rows.into_iter().enumerate().skip(skip).take(take) {
            let selected = frame_selected == Some(ix);
            let (indent, chevron, label) = match &row {
                FrameRow::Thread {
                    label, expanded, ..
                } => (0, if *expanded { "▾" } else { "▸" }, label.clone()),
                FrameRow::Frame { name, depth, .. } => (depth * TUI_INDENT, " ", name.clone()),
            };
            let thread = match &row {
                FrameRow::Thread { id, .. } => Some(*id),
                FrameRow::Frame { .. } => None,
            };
            let lead = 1 + indent + 2;
            let click = entity.clone();
            let fold = entity.clone();
            let slot = ix - skip;
            frame_els.push(
                div()
                    .id(ElementId::NamedInteger("debug-frame".into(), slot as u64))
                    .h(px(cell.h))
                    .w(cell.cols(cols))
                    .flex_shrink_0()
                    .flex()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .cursor_pointer()
                    .when(selected && threads_focused, |d| d.bg(theme.list_bg_focused))
                    .when(selected && !threads_focused, |d| {
                        d.bg(theme.list_bg_selected)
                    })
                    .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .on_click(move |e, window, cx| {
                        let count = e.click_count();
                        let row = row.clone();
                        click.update(cx, |this, cx| this.click_frame_row(row, count, window, cx));
                    })
                    .child(div().w(cell.cols(1 + indent)).flex_shrink_0())
                    .child(
                        div()
                            .id(ElementId::NamedInteger(
                                "debug-thread-chevron".into(),
                                slot as u64,
                            ))
                            .w(cell.cols(2))
                            .flex_shrink_0()
                            .text_color(theme.icon_primary)
                            .when_some(thread, |d, id| {
                                d.on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    fold.update(cx, |this, cx| this.toggle_thread(id, cx));
                                })
                            })
                            .child(chevron),
                    )
                    .child(
                        div()
                            .text_color(theme.list_text)
                            .child(skin::fit_text(&label, cols.saturating_sub(lead))),
                    )
                    .into_any_element(),
            );
        }
        let frames_list = tui_list(
            "debug-frames",
            &entity,
            |this: &mut DebugTool| &mut this.tui_frames_scroll,
            frames_top,
            frames_total,
            cols,
            threads_visible,
            div()
                .flex()
                .flex_col()
                .children(frame_els)
                .into_any_element(),
            cell,
            &self.tui_bars,
        );
        // ---- Variables ----
        let total = rows.len();
        let selected = self.selected;
        if self.part == Part::Variables && self.reveal {
            self.tui_tree_scroll.follow(selected, total, tree_visible);
        }
        self.reveal = false;
        self.tui_tree_scroll.top = self
            .tui_tree_scroll
            .top
            .min(total.saturating_sub(tree_visible));
        let top = self.tui_tree_scroll.top;
        let tree_focused = focused && self.part == Part::Variables;
        let tree_rows: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .skip(top)
            .take(tree_visible)
            .map(|(ix, row)| {
                self.tui_tree_row(ix, ix - top, row, cols, tree_focused, cell, &theme, cx)
            })
            .collect();
        let menu = entity.clone();
        let tree = tui_list(
            "debug-variables",
            &entity,
            |this: &mut DebugTool| &mut this.tui_tree_scroll,
            top,
            total,
            cols,
            tree_visible,
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
        let add = entity.clone();
        let add_watch = skin::button("debug-add-watch", "Add Watch", false, &theme, cell)
            .on_click(move |_, window, cx| {
                add.update(cx, |this, cx| this.start_editing(Editing::New, window, cx));
            })
            .into_any_element();
        let mut column = div()
            .flex()
            .flex_col()
            .child(heading("Processes", Some(toolbar.into_any_element())))
            .children(session_list);
        if threads_h > 0 {
            column = column
                .child(heading("Threads & Frames", None))
                .child(frames_list);
        }
        if variables_h > 0 {
            column = column
                .child(heading("Variables", Some(add_watch)))
                .child(tree);
        }
        frame(column.into_any_element())
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
        let lead = 1 + indent + 2 + if watch { 2 } else { 0 };
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
            .child(div().w(cell.cols(1 + indent)).flex_shrink_0())
            .child(
                div()
                    .id(ElementId::NamedInteger("debug-chevron".into(), slot as u64))
                    .w(cell.cols(2))
                    .flex_shrink_0()
                    .text_color(theme.icon_primary)
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
            .child(content)
            .into_any_element()
    }
}

/// A session with threads, frames, scopes, children and watches, for the
/// tests of the debugger tools (no adapter behind it): stopped in `app::run`
/// on thread 1 ("main", with lldb-dap's unnamed "Thread 2" beside it), the
/// scopes lldb-dap sends, collapsed but for Locals (opened by hand).
#[cfg(test)]
pub(crate) fn fake_session() -> DebugSession {
    let var = |name: &str, value: &str, ty: Option<&str>, reference: i64| Variable {
        name: name.into(),
        value: value.into(),
        type_name: ty.map(Into::into),
        variables_reference: reference,
        evaluate_name: None,
    };
    let mut s = DebugSession::new("debug fixture".into());
    s.state = SessionState::Stopped {
        reason: "breakpoint".into(),
        description: None,
    };
    s.threads = vec![
        Thread {
            id: 3212558,
            name: "main".into(),
        },
        Thread {
            id: 3212600,
            name: "Thread 2".into(),
        },
    ];
    s.current_thread = Some(3212558);
    s.frames = vec![
        StackFrame {
            id: 100,
            name: "app::run".into(),
            path: Some(PathBuf::from("/w/src/main.rs")),
            line: 11,
            column: 4,
            presentation_hint: None,
        },
        StackFrame {
            id: 101,
            name: "std::rt::lang_start".into(),
            path: None,
            line: 0,
            column: 0,
            presentation_hint: None,
        },
    ];
    s.scopes = vec![
        Scope {
            name: "Locals".into(),
            variables_reference: 1,
            expensive: false,
        },
        Scope {
            name: "Globals".into(),
            variables_reference: 3,
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
                (0, "Globals".into()),
                (0, "Registers".into()),
            ]
        );
        assert!(rows[2].expanded && rows[2].expandable());
        assert!(rows[4].expandable() && !rows[4].expanded);
        assert!(rows[5].expandable() && !rows[5].expanded);
        assert!(!rows[3].expandable());
        assert!(rows[1].error);
        assert_eq!(rows[4].expression.as_deref(), Some("point"));
    }

    #[test]
    fn scopes_start_collapsed() {
        let mut s = fake_session();
        s.expanded.clear();
        let rows = tree_rows(&TreeSource::of(&s), None);
        assert_eq!(
            names(&rows)[2..],
            [
                (0, "Locals".into()),
                (0, "Globals".into()),
                (0, "Registers".into()),
            ]
        );
        assert!(rows[2..].iter().all(|r| r.expandable() && !r.expanded));
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
                (0, "Globals".into()),
                (0, "Registers".into()),
                (1, "Collecting data…".into()),
            ]
        );
        assert_eq!(rows[5].expression.as_deref(), Some("point.x"));
        assert_eq!(rows[9].kind, NodeKind::Loading);
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
        assert_eq!(rows.len(), 7);
    }

    #[test]
    fn row_text_dims_the_type() {
        let s = fake_session();
        let rows = tree_rows(&TreeSource::of(&s), None);
        // A variable: the colon right after the name (batch 13).
        let (text, runs) = row_text(&rows[3]);
        assert_eq!(text, "count: {i32} 3");
        assert_eq!(runs[1], (5..7, Tint::Punctuation));
        assert_eq!(runs[2], (7..13, Tint::Type));
        // A watch keeps ` = `.
        let (text, runs) = row_text(&rows[0]);
        assert!(text.starts_with(&format!("{} = ", rows[0].name)), "{text}");
        assert_eq!(runs[1].1, Tint::Punctuation);
        let (text, runs) = row_text(&rows[1]);
        assert_eq!(text, "nope = undeclared identifier");
        assert_eq!(runs.last().unwrap().1, Tint::Error);
        let (clipped, runs) = clip_runs("count: {i32} 3", &row_text(&rows[3]).1, 9);
        assert_eq!(clipped, "count: {…");
        assert_eq!(runs.last().unwrap(), &(7..clipped.len(), Tint::Type));
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
    fn threads_print_as_fleet() {
        let s = fake_session();
        assert_eq!(thread_label(0, &s.threads[0]), "1: tid=3212558 \"main\"");
        assert_eq!(thread_label(1, &s.threads[1]), "2: tid=3212600");
        let unnamed = Thread {
            id: 7,
            name: String::new(),
        };
        assert_eq!(thread_label(2, &unnamed), "3: tid=7");
        let odd = Thread {
            id: 8,
            name: "Thread pool".into(),
        };
        assert_eq!(thread_label(3, &odd), "4: tid=8 \"Thread pool\"");
    }

    #[test]
    fn the_current_thread_holds_its_frames() {
        let s = fake_session();
        let mut collapsed = HashSet::new();
        let rows = frame_rows(&s.threads, s.current_thread, &s.frames, &collapsed);
        assert_eq!(
            rows,
            vec![
                FrameRow::Thread {
                    id: 3212558,
                    label: "1: tid=3212558 \"main\"".into(),
                    expanded: true,
                },
                FrameRow::Frame {
                    index: 0,
                    name: "app::run".into(),
                    depth: 1,
                },
                // Library code (no file) is a row like any other.
                FrameRow::Frame {
                    index: 1,
                    name: "std::rt::lang_start".into(),
                    depth: 1,
                },
                FrameRow::Thread {
                    id: 3212600,
                    label: "2: tid=3212600".into(),
                    expanded: false,
                },
            ]
        );
        // The current frame is highlighted, unless a thread node is.
        assert_eq!(frames_selected(&rows, None, s.current_thread, 1), Some(2));
        assert_eq!(
            frames_selected(&rows, Some(3212600), s.current_thread, 1),
            Some(3)
        );
        // Folded by hand: the node alone, and it takes the highlight.
        collapsed.insert(3212558);
        let rows = frame_rows(&s.threads, s.current_thread, &s.frames, &collapsed);
        assert_eq!(rows.len(), 2);
        assert_eq!(frames_selected(&rows, None, s.current_thread, 0), Some(0));
        // No threads listed: the frames alone, at the left.
        let rows = frame_rows(&[], None, &s.frames, &HashSet::new());
        assert!(matches!(rows[0], FrameRow::Frame { depth: 0, .. }));
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn the_toolbar_follows_the_state() {
        use ToolbarButton::*;
        let stopped = fake_session().state.clone();
        let live: Vec<ToolbarButton> = ToolbarButton::ALL
            .into_iter()
            .filter(|b| b.enabled(&stopped))
            .collect();
        // Batch 13 (stopped): only Pause greyed.
        assert_eq!(
            live,
            vec![Resume, Restart, Stop, StepOver, StepInto, StepOut, Output]
        );
        assert!(Pause.enabled(&SessionState::Running));
        assert!(!StepOver.enabled(&SessionState::Running));
        assert!(!Stop.enabled(&SessionState::Exited(Some(0))));
        assert_eq!(Resume.action_id(), Some("debug/resume"));
        assert!(Output.action().is_none());
        assert!(!Restart.enabled(&SessionState::Exited(None)));
        assert!(Restart.action().is_some());
        assert_eq!(StepInto.title(), "Step Into");
    }

    #[test]
    fn status() {
        let mut s = fake_session();
        assert_eq!(status_text(&s), None);
        s.state = SessionState::Exited(Some(1));
        assert_eq!(
            status_text(&s),
            Some(("Process finished with exit code 1".into(), true))
        );
        s.state = SessionState::Exited(Some(0));
        assert_eq!(
            status_text(&s),
            Some(("Process finished with exit code 0".into(), false))
        );
        s.state = SessionState::Failed("no lldb-dap".into());
        assert_eq!(status_text(&s), Some(("no lldb-dap".into(), true)));
        // An exception or panic stop says what it stopped on.
        s.state = SessionState::Stopped {
            reason: "exception".into(),
            description: Some("signal SIGABRT".into()),
        };
        assert_eq!(
            status_text(&s),
            Some(("Exception: signal SIGABRT".into(), true))
        );
        s.stopped_on_panic = true;
        s.state = SessionState::Stopped {
            reason: "exception".into(),
            description: Some("boom 60".into()),
        };
        assert_eq!(status_text(&s), Some(("Panic: boom 60".into(), true)));
        s.state = SessionState::Stopped {
            reason: "exception".into(),
            description: Some(crate::debugger::PANIC_DESCRIPTION.into()),
        };
        assert_eq!(status_text(&s), Some(("Panic".into(), true)));
        assert_eq!(single_line("a\nb"), "a b");
    }
}
