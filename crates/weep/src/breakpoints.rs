//! Line breakpoints (M8): one store for the whole app, keyed by absolute
//! path, so a file keeps its breakpoints while closed, every view of a
//! document paints the same ones, and the Breakpoints tool and the debug
//! sessions read one list. The editor's gutter toggles them
//! (`debug/toggle-breakpoint`, a click in the gutter); a session sends a
//! file's list with `setBreakpoints` on every change and writes the
//! adapter's verdict back through [`BreakpointStore::set_verified`].
//!
//! The names below are the contract between the editor, the Breakpoints
//! tool and `debugger.rs`. Lines are 0-based.
//!
//! Breakpoints follow the edits of their file ([`Tracked`], called from
//! `Document::edit`): each sits on its line's start, so lines inserted or
//! deleted above move it, and it goes away with its line when an edit
//! deletes the line's whole text together with a line break
//! ([`follow`]); undoing that edit brings it back ([`Removed`], kept by the
//! document), and a move-line carries it with its text. A workspace keeps
//! its breakpoints and exception breakpoints across launches in
//! `layout.json` ([`persist`]).

use crate::actions::{DebugBreakpointsJumpToTool, PopupCancel};
use crate::layout::{Layout, SavedBreakpoint};
use crate::menu::{MenuEntry, MenuItem, SelectFn};
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use weep_text::{Buffer, Edit};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBreakpoint {
    pub line: usize,
    pub enabled: bool,
    pub condition: Option<String>,
    pub log_message: Option<String>,
    /// DAP's `hitCondition` (IntelliJ's pass count): the adapter stops on
    /// the hit it names. Estimate: no probe has seen Fleet's editor for it.
    pub hit_condition: Option<String>,
    /// The running session's verdict: `None` with no session or before the
    /// adapter answered, `Some(false)` when it could not bind the line.
    pub verified: Option<bool>,
}

impl LineBreakpoint {
    pub fn at(line: usize) -> LineBreakpoint {
        LineBreakpoint {
            line,
            enabled: true,
            condition: None,
            log_message: None,
            hit_condition: None,
            verified: None,
        }
    }

    /// How the gutter draws it.
    pub fn look(&self) -> Look {
        if !self.enabled {
            Look::Disabled
        } else if self.verified == Some(false) {
            Look::Unverified
        } else {
            Look::Enabled
        }
    }

    /// The gutter tooltip's title and detail (the condition, the log
    /// message and the hit count that are set, in that order). Estimate:
    /// IntelliJ's wording; no probe has hovered one.
    pub fn tooltip(&self) -> (String, Option<String>) {
        let title = match self.look() {
            Look::Enabled => "Line breakpoint",
            Look::Disabled => "Line breakpoint (disabled)",
            Look::Unverified => "Line breakpoint (not verified)",
        };
        let set = |v: &Option<String>| v.clone().filter(|s| !s.trim().is_empty());
        let parts: Vec<String> = [
            set(&self.condition).map(|c| format!("Condition: {c}")),
            set(&self.log_message).map(|l| format!("Log: {l}")),
            set(&self.hit_condition).map(|h| format!("Hit count: {h}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let detail = (!parts.is_empty()).then(|| parts.join(", "));
        (title.to_string(), detail)
    }

    /// What the Breakpoints tool's fields edit, as the user typed them
    /// (`None` for an empty field).
    pub fn properties(&self) -> Properties {
        Properties {
            enabled: self.enabled,
            condition: self.condition.clone(),
            log_message: self.log_message.clone(),
            hit_condition: self.hit_condition.clone(),
        }
    }
}

/// A line breakpoint's settings the Breakpoints tool edits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    pub enabled: bool,
    pub condition: Option<String>,
    pub log_message: Option<String>,
    pub hit_condition: Option<String>,
}

impl Properties {
    /// A field's text as a property: blank is unset.
    pub fn field(text: &str) -> Option<String> {
        (!text.trim().is_empty()).then(|| text.to_string())
    }
}

/// A breakpoint's form in the gutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// A filled circle in `icon.background.red` (tui `●`).
    Enabled,
    /// `enabled == false`: a ring in the same red (Fleet's
    /// `breakpoint-inactive`; tui `○`).
    Disabled,
    /// The running session could not bind it: a grey circle (tui `●` grey).
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BreakpointsEvent {
    /// A file's breakpoints changed (added, removed, enabled, moved).
    Changed(PathBuf),
    /// An exception breakpoint was enabled or disabled, or an adapter
    /// offered new ones ([`BreakpointStore::exception_breakpoints`]).
    ExceptionsChanged,
}

/// An exception breakpoint: a DAP `exceptionBreakpointFilters` entry the
/// adapter offered, or WEEP's own [`RUST_PANIC`] (sent as a function
/// breakpoint on `rust_panic`, which lldb-dap has no filter for). Shown in
/// the Breakpoints tool; kept per workspace in `layout.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionBreakpoint {
    /// The adapter's `filter` id (or [`RUST_PANIC`]).
    pub filter: String,
    /// The adapter's `label`, as the Breakpoints tool shows it.
    pub label: String,
    pub enabled: bool,
}

/// WEEP's exception breakpoint for Rust panics.
pub const RUST_PANIC: &str = "rust_panic";

#[derive(Default)]
pub struct BreakpointStore {
    files: BTreeMap<PathBuf, Vec<LineBreakpoint>>,
    /// The exception breakpoints known so far ([`RUST_PANIC`] first, then
    /// every adapter's filters as sessions report them).
    exceptions: Vec<ExceptionBreakpoint>,
}

impl EventEmitter<BreakpointsEvent> for BreakpointStore {}

struct GlobalBreakpoints(Entity<BreakpointStore>);

impl Global for GlobalBreakpoints {}

impl BreakpointStore {
    pub fn init(cx: &mut App) {
        let store = cx.new(|_| BreakpointStore {
            exceptions: vec![ExceptionBreakpoint {
                filter: RUST_PANIC.to_string(),
                // Estimate: codelldb's label for its Rust panic filter.
                label: "Rust: on panic".to_string(),
                enabled: false,
            }],
            ..BreakpointStore::default()
        });
        cx.set_global(GlobalBreakpoints(store));
    }

    pub fn global(cx: &App) -> Entity<BreakpointStore> {
        cx.global::<GlobalBreakpoints>().0.clone()
    }

    /// The store, when the app made one (a document outside the app has none).
    pub fn try_global(cx: &App) -> Option<Entity<BreakpointStore>> {
        cx.try_global::<GlobalBreakpoints>().map(|g| g.0.clone())
    }

    /// The exception breakpoints, [`RUST_PANIC`] first.
    pub fn exception_breakpoints(&self) -> &[ExceptionBreakpoint] {
        &self.exceptions
    }

    /// The `filter` ids to send with `setExceptionBreakpoints` (or, for
    /// [`RUST_PANIC`], as a function breakpoint): the enabled ones.
    pub fn enabled_exception_filters(&self) -> Vec<String> {
        self.exceptions
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.filter.clone())
            .collect()
    }

    /// Enables or disables an exception breakpoint by its filter id.
    pub fn set_exception_enabled(&mut self, filter: &str, enabled: bool, cx: &mut Context<Self>) {
        if let Some(e) = self.exceptions.iter_mut().find(|e| e.filter == filter) {
            if e.enabled != enabled {
                e.enabled = enabled;
                cx.emit(BreakpointsEvent::ExceptionsChanged);
                cx.notify();
            }
        }
    }

    /// A session's adapter offers these filters (`filter`, `label`, the
    /// adapter's `default`): unknown ones are added with their default,
    /// known ones keep the user's choice and take the adapter's label.
    pub fn offer_exception_filters(
        &mut self,
        filters: Vec<(String, String, bool)>,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        for (filter, label, default) in filters {
            match self.exceptions.iter_mut().find(|e| e.filter == filter) {
                Some(e) => {
                    if e.label != label {
                        e.label = label;
                        changed = true;
                    }
                }
                None => {
                    self.exceptions.push(ExceptionBreakpoint {
                        filter,
                        label,
                        enabled: default,
                    });
                    changed = true;
                }
            }
        }
        if changed {
            cx.emit(BreakpointsEvent::ExceptionsChanged);
            cx.notify();
        }
    }

    /// `debug/toggle-enabled-breakpoint`: enables a disabled breakpoint and
    /// disables an enabled one; nothing without one on `line`.
    pub fn toggle_enabled(&mut self, path: &Path, line: usize, cx: &mut Context<Self>) {
        if let Some(enabled) = self.at(path, line).map(|b| b.enabled) {
            self.set_enabled(path, line, !enabled, cx);
        }
    }

    /// The file moved (`files/rename`): its breakpoints go with it.
    pub fn rename_file(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        if from == to {
            return;
        }
        if let Some(list) = self.files.remove(from).filter(|l| !l.is_empty()) {
            self.files.insert(to.to_path_buf(), list);
            cx.emit(BreakpointsEvent::Changed(from.to_path_buf()));
            cx.emit(BreakpointsEvent::Changed(to.to_path_buf()));
            cx.notify();
        }
    }

    /// Every file with breakpoints, in path order.
    pub fn files(&self) -> impl Iterator<Item = (&Path, &[LineBreakpoint])> {
        self.files
            .iter()
            .filter(|(_, b)| !b.is_empty())
            .map(|(p, b)| (p.as_path(), b.as_slice()))
    }

    /// One file's breakpoints, sorted by line.
    pub fn in_file(&self, path: &Path) -> &[LineBreakpoint] {
        self.files.get(path).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn at(&self, path: &Path, line: usize) -> Option<&LineBreakpoint> {
        self.in_file(path).iter().find(|b| b.line == line)
    }

    /// Adds a breakpoint on `line`, or removes the one there.
    pub fn toggle(&mut self, path: &Path, line: usize, cx: &mut Context<Self>) {
        toggle_line(self.files.entry(path.to_path_buf()).or_default(), line);
        cx.emit(BreakpointsEvent::Changed(path.to_path_buf()));
        cx.notify();
    }

    pub fn set_enabled(&mut self, path: &Path, line: usize, enabled: bool, cx: &mut Context<Self>) {
        if let Some(b) = self
            .files
            .get_mut(path)
            .and_then(|l| l.iter_mut().find(|b| b.line == line))
        {
            if b.enabled != enabled {
                b.enabled = enabled;
                cx.emit(BreakpointsEvent::Changed(path.to_path_buf()));
                cx.notify();
            }
        }
    }

    /// Sets the breakpoint on `line`'s enabled state, condition, log message
    /// and hit count (the Breakpoints tool's fields); emits `Changed` when
    /// one differs, so a session sends the file's list again.
    pub fn set_properties(
        &mut self,
        path: &Path,
        line: usize,
        properties: Properties,
        cx: &mut Context<Self>,
    ) {
        if let Some(b) = self
            .files
            .get_mut(path)
            .and_then(|l| l.iter_mut().find(|b| b.line == line))
        {
            if b.properties() != properties {
                b.enabled = properties.enabled;
                b.condition = properties.condition;
                b.log_message = properties.log_message;
                b.hit_condition = properties.hit_condition;
                cx.emit(BreakpointsEvent::Changed(path.to_path_buf()));
                cx.notify();
            }
        }
    }

    /// The exception breakpoints `layout.json` kept: known filters take the
    /// saved choice (and keep their label), unknown ones (an adapter of an
    /// earlier launch offered them) come back as saved.
    pub fn restore_exceptions(&mut self, saved: &[ExceptionBreakpoint], cx: &mut Context<Self>) {
        let mut changed = false;
        for s in saved {
            match self.exceptions.iter_mut().find(|e| e.filter == s.filter) {
                Some(e) => {
                    if e.enabled != s.enabled {
                        e.enabled = s.enabled;
                        changed = true;
                    }
                }
                None => {
                    self.exceptions.push(s.clone());
                    changed = true;
                }
            }
        }
        if changed {
            cx.emit(BreakpointsEvent::ExceptionsChanged);
            cx.notify();
        }
    }

    pub fn remove(&mut self, path: &Path, line: usize, cx: &mut Context<Self>) {
        if let Some(list) = self.files.get_mut(path) {
            let before = list.len();
            list.retain(|b| b.line != line);
            if list.len() != before {
                cx.emit(BreakpointsEvent::Changed(path.to_path_buf()));
                cx.notify();
            }
        }
    }

    /// Replaces a file's whole list (edits moved its lines, or a workspace's
    /// saved breakpoints were loaded).
    pub fn set_file(&mut self, path: &Path, list: Vec<LineBreakpoint>, cx: &mut Context<Self>) {
        let list = normalized(list);
        if self.in_file(path) != list.as_slice() {
            self.files.insert(path.to_path_buf(), list);
            cx.emit(BreakpointsEvent::Changed(path.to_path_buf()));
            cx.notify();
        }
    }

    /// The session's verdicts for a file's enabled breakpoints, in line
    /// order (`None` clears them: the session ended). Repaints, but emits no
    /// `Changed`, so a session does not send the list again.
    pub fn set_verified(&mut self, path: &Path, verdicts: Option<&[bool]>, cx: &mut Context<Self>) {
        if let Some(list) = self.files.get_mut(path) {
            let mut verdicts = verdicts.map(|v| v.iter());
            for b in list.iter_mut().filter(|b| b.enabled) {
                b.verified = match verdicts.as_mut() {
                    Some(v) => v.next().copied(),
                    None => None,
                };
            }
            cx.notify();
        }
    }
}

/// A row of the menu a right click on a breakpoint opens (batch 13: at the
/// pointer, "Remove Breakpoint" with ⌫, "Disable Breakpoint", a separator,
/// "View All Breakpoints").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterMenuRow {
    Remove,
    /// Disables an enabled breakpoint, enables a disabled one.
    ToggleEnabled {
        enabled: bool,
    },
    Separator,
    /// Opens the Breakpoints tool (`debug/breakpoints/jump-to-tool`).
    ViewAll,
}

impl GutterMenuRow {
    pub fn label(self) -> &'static str {
        match self {
            GutterMenuRow::Remove => "Remove Breakpoint",
            GutterMenuRow::ToggleEnabled { enabled: true } => "Disable Breakpoint",
            // estimate: Fleet's row for a disabled breakpoint was not shot.
            GutterMenuRow::ToggleEnabled { enabled: false } => "Enable Breakpoint",
            GutterMenuRow::Separator => "",
            GutterMenuRow::ViewAll => "View All Breakpoints",
        }
    }
}

/// The gutter menu's rows for a breakpoint.
pub fn gutter_menu_rows(breakpoint: &LineBreakpoint) -> [GutterMenuRow; 4] {
    [
        GutterMenuRow::Remove,
        GutterMenuRow::ToggleEnabled {
            enabled: breakpoint.enabled,
        },
        GutterMenuRow::Separator,
        GutterMenuRow::ViewAll,
    ]
}

/// The rows of the menu a right click on a Breakpoints tool row opens
/// (twenty-fourth pass, `rest8-fleet-bp-row-right.png`): the gutter menu's
/// first two, no separator and no View All.
pub fn tool_menu_rows(breakpoint: &LineBreakpoint) -> [GutterMenuRow; 2] {
    let [remove, toggle, ..] = gutter_menu_rows(breakpoint);
    [remove, toggle]
}

/// The menu for the breakpoint on `line` of `path` (none without one): its
/// rows act on that breakpoint, not on the carets' lines. Fleet shows the
/// remove chord (⌫) beside "Remove Breakpoint" and no other.
pub fn gutter_menu(path: &Path, line: usize, cx: &App) -> Vec<MenuEntry> {
    menu_with(path, line, gutter_menu_rows, cx)
}

/// The Breakpoints tool's row menu for the breakpoint on `line` of `path`:
/// the gutter menu's rows without the separator and View All.
pub fn tool_menu(path: &Path, line: usize, cx: &App) -> Vec<MenuEntry> {
    menu_with(path, line, tool_menu_rows, cx)
}

fn menu_with<const N: usize>(
    path: &Path,
    line: usize,
    rows: fn(&LineBreakpoint) -> [GutterMenuRow; N],
    cx: &App,
) -> Vec<MenuEntry> {
    let Some(breakpoint) =
        BreakpointStore::try_global(cx).and_then(|s| s.read(cx).at(path, line).cloned())
    else {
        return Vec::new();
    };
    rows(&breakpoint)
        .into_iter()
        .map(|row| {
            let path = path.to_path_buf();
            let item = |shortcut: Option<String>, f: SelectFn| {
                MenuEntry::Item(MenuItem {
                    label: row.label().into(),
                    shortcut: shortcut.map(Into::into),
                    action: Box::new(PopupCancel),
                    enabled: true,
                    on_select: Some(f),
                    icon: None,
                })
            };
            match row {
                GutterMenuRow::Remove => item(
                    crate::keymap::hint_for(cx, "debug/remove-breakpoint"),
                    Rc::new(move |_, cx| {
                        let store = BreakpointStore::global(cx);
                        store.update(cx, |s, cx| s.remove(&path, line, cx));
                    }),
                ),
                GutterMenuRow::ToggleEnabled { .. } => item(
                    None,
                    Rc::new(move |_, cx| {
                        let store = BreakpointStore::global(cx);
                        store.update(cx, |s, cx| s.toggle_enabled(&path, line, cx));
                    }),
                ),
                GutterMenuRow::Separator => MenuEntry::Separator,
                GutterMenuRow::ViewAll => MenuEntry::Item(MenuItem {
                    label: row.label().into(),
                    shortcut: None,
                    action: Box::new(DebugBreakpointsJumpToTool),
                    enabled: true,
                    on_select: None,
                    icon: None,
                }),
            }
        })
        .collect()
}

/// Adds a breakpoint on `line` to a sorted list, or removes the one there;
/// true when it added.
pub fn toggle_line(list: &mut Vec<LineBreakpoint>, line: usize) -> bool {
    match list.iter().position(|b| b.line == line) {
        Some(ix) => {
            list.remove(ix);
            false
        }
        None => {
            let ix = list.partition_point(|b| b.line < line);
            list.insert(ix, LineBreakpoint::at(line));
            true
        }
    }
}

/// A list sorted by line with one breakpoint per line: where two land on
/// one line (an edit joined their lines) the one first in the list stays.
pub fn normalized(mut list: Vec<LineBreakpoint>) -> Vec<LineBreakpoint> {
    list.sort_by_key(|b| b.line);
    list.dedup_by_key(|b| b.line);
    list
}

// ---- following the edits ---------------------------------------------------------

/// [`follow`] for several lines, as offsets only (the tests' view).
#[cfg(test)]
fn follow_edits(spans: &[Range<usize>], edits: &[Edit]) -> Vec<Option<usize>> {
    spans
        .iter()
        .map(|span| match follow(span, edits) {
            Followed::At { start, .. } => Some(start),
            Followed::Deleted { .. } => None,
        })
        .collect()
}

/// Where one line went through some edits ([`follow`]'s rule).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Followed {
    /// Its start's offset after the edits, and the text of the last
    /// replacement that held the whole line (offsets after the edits): the
    /// line may sit elsewhere in it.
    At {
        start: usize,
        replaced: Option<Range<usize>>,
    },
    /// An edit deleted it: undoing that edit puts the text back at `anchor`
    /// (moved through the edits after it), the line `distance` chars in.
    Deleted { anchor: usize, distance: usize },
}

/// Moves a breakpoint's line through `edits`. The line is given as the
/// char span of its text (without the line break) before the edits; the
/// answer is where its start lands, or that an edit deleted the line.
///
/// The rule: a line sits on its start. An insertion at or before the start
/// pushes it on (Enter at column 0 moves the breakpoint down with the
/// code), so lines inserted above move it and so does a line break typed at
/// its start. A deletion that takes the line's whole text and a line break
/// next to it (deleting the line, or a backspace on an empty one) removes
/// it; any other deletion pulls the start to the deletion's place, so a
/// line joined to the one above (backspace at its start) takes its
/// breakpoint along, merging with one already there. A replacement keeps
/// the start's distance into the replaced range, clamped to the new text (a
/// reload from disk keeps the offset); [`Tracked`] then looks for the line's
/// own text in the replacement ([`Followed::At`]'s `replaced`), so lines a
/// move-line swapped carry their breakpoints.
fn follow(span: &Range<usize>, edits: &[Edit]) -> Followed {
    let (mut s, mut t) = (span.start, span.end);
    let mut replaced: Option<Range<usize>> = None;
    for (ix, e) in edits.iter().enumerate() {
        let (o, n, m) = (e.offset, e.old_len, e.new_len);
        if n == 0 {
            if s >= o {
                s += m;
            }
            if t >= o {
                t += m;
            }
        } else {
            let end = o + n;
            if m == 0 && o <= s && t <= end && (o < s || end > t) {
                let anchor = edits[ix + 1..].iter().fold(o, |p, e| e.adjust(p));
                return Followed::Deleted {
                    anchor,
                    distance: s - o,
                };
            }
            let covered = m > 0 && o <= s && t <= end;
            let map = |p: usize| {
                if p <= o {
                    p
                } else if p >= end {
                    p - n + m
                } else {
                    o + (p - o).min(m)
                }
            };
            s = map(s);
            t = map(t);
            if covered {
                replaced = Some(o..o + m);
                t = t.max(s);
                continue;
            }
        }
        if let Some(r) = replaced.as_mut() {
            *r = e.adjust(r.start)..e.adjust(r.end);
        }
        t = t.max(s);
    }
    Followed::At { start: s, replaced }
}

/// Line `line`'s text as a char span (the last line for one past the end).
fn line_span(buffer: &Buffer, line: usize) -> Range<usize> {
    let row = line.min(buffer.line_count().saturating_sub(1));
    let start = buffer.line_to_char(row);
    start..start + buffer.line_len(row)
}

/// The row nearest `near` whose text is `text` among the rows starting in
/// `region` (char offsets of `buffer`).
fn find_line(buffer: &Buffer, region: &Range<usize>, text: &str, near: usize) -> Option<usize> {
    let len = buffer.len_chars();
    let (from, to) = (region.start.min(len), region.end.min(len));
    let first = buffer.offset_to_point(from).row;
    let last = buffer.offset_to_point(to).row;
    (first..=last)
        .filter(|&r| {
            let at = buffer.line_to_char(r);
            from <= at && (at < to || from == to) && buffer.line(r) == text
        })
        .min_by_key(|&r| r.abs_diff(near))
}

/// The breakpoints edits deleted with their lines, kept per document
/// (`Document` holds one) until the undo step that deleted them is undone,
/// which brings them back, or that step leaves the undo stack.
#[derive(Clone, Debug, Default)]
pub struct Removed {
    entries: Vec<RemovedBreakpoint>,
}

#[derive(Clone, Debug)]
struct RemovedBreakpoint {
    breakpoint: LineBreakpoint,
    /// Where the deleted text was, moved through the edits since.
    anchor: usize,
    /// The line's start past `anchor` once the text is back.
    distance: usize,
    /// The line's text, checked before it comes back.
    text: String,
    /// The undo step that deleted it (its index in the buffer's undo stack).
    step: usize,
}

impl Removed {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A file's breakpoints as they stood before an edit, moved through it by
/// [`moved_through`].
struct Before {
    list: Vec<LineBreakpoint>,
    spans: Vec<Range<usize>>,
    /// Each line's text, to find it again inside a replacement.
    texts: Vec<String>,
    version: u64,
    /// The buffer's undo depth: one less afterwards means an undo.
    depth: usize,
}

impl Before {
    fn take(list: Vec<LineBreakpoint>, buffer: &Buffer) -> Before {
        let rows = buffer.line_count();
        Before {
            spans: list.iter().map(|b| line_span(buffer, b.line)).collect(),
            texts: list
                .iter()
                .map(|b| {
                    if b.line < rows {
                        buffer.line(b.line)
                    } else {
                        String::new()
                    }
                })
                .collect(),
            list,
            version: buffer.version(),
            depth: buffer.undo_depth(),
        }
    }
}

/// The list after the edits `buffer` made since `before`: lines moved,
/// deleted ones kept in `removed` under the step that deleted them, and,
/// when the edit was the undo of a step, the breakpoints that step deleted
/// back on their lines.
fn moved_through(before: Before, buffer: &Buffer, removed: &mut Removed) -> Vec<LineBreakpoint> {
    let depth = buffer.undo_depth();
    let Some(edits) = buffer.edits_since(before.version) else {
        // Too many edits for the log: the lines stay, within the text.
        removed.entries.clear();
        let last = buffer.line_count().saturating_sub(1);
        return before
            .list
            .into_iter()
            .map(|b| LineBreakpoint {
                line: b.line.min(last),
                ..b
            })
            .collect();
    };
    for r in &mut removed.entries {
        r.anchor = edits.iter().fold(r.anchor, |p, e| e.adjust(p));
    }
    let mut moved = Vec::with_capacity(before.list.len());
    let mut deleted = Vec::new();
    let lines = before.list.into_iter().zip(before.spans).zip(before.texts);
    for ((b, span), text) in lines {
        match follow(&span, &edits) {
            Followed::At { start, replaced } => {
                let mut row = buffer.offset_to_point(start).row;
                if let Some(region) = replaced.filter(|_| buffer.line(row) != text) {
                    row = find_line(buffer, &region, &text, row).unwrap_or(row);
                }
                moved.push(LineBreakpoint { line: row, ..b });
            }
            Followed::Deleted { anchor, distance } => deleted.push(RemovedBreakpoint {
                breakpoint: b,
                anchor,
                distance,
                text,
                step: depth.saturating_sub(1),
            }),
        }
    }
    if depth + 1 == before.depth {
        // The undo of step `depth`: what it deleted comes back where the
        // text did, if the line reads as it did.
        let len = buffer.len_chars();
        for r in removed.entries.iter().filter(|r| r.step == depth) {
            let start = r.anchor + r.distance;
            if start > len {
                continue;
            }
            let at = buffer.offset_to_point(start);
            if at.column == 0 && buffer.line(at.row) == r.text {
                moved.push(LineBreakpoint {
                    line: at.row,
                    verified: None,
                    ..r.breakpoint.clone()
                });
            }
        }
    }
    // Steps no longer on the undo stack take their breakpoints with them; a
    // line the undo itself deleted is not kept (a redo does not bring it).
    removed.entries.retain(|r| r.step < depth);
    if depth + 1 != before.depth && depth > 0 {
        removed.entries.extend(deleted);
    }
    moved
}

/// A document's breakpoints taken before an edit (`Document::edit`), to be
/// moved through it afterwards.
pub struct Tracked {
    store: Entity<BreakpointStore>,
    path: PathBuf,
    before: Before,
}

impl Tracked {
    /// The file's breakpoints and their lines' spans in `buffer`; `None`
    /// for a file without any, live or deleted (or a document with no
    /// path).
    pub fn before_edit(
        path: Option<&Path>,
        buffer: &Buffer,
        removed: &Removed,
        cx: &App,
    ) -> Option<Tracked> {
        let path = path?;
        let store = BreakpointStore::try_global(cx)?;
        let list = store.read(cx).in_file(path).to_vec();
        if list.is_empty() && removed.is_empty() {
            return None;
        }
        Some(Tracked {
            store,
            path: path.to_path_buf(),
            before: Before::take(list, buffer),
        })
    }

    /// Moves the breakpoints through the edits `buffer` made since, and
    /// gives the store the moved list when anything changed.
    pub fn after_edit(self, buffer: &Buffer, removed: &mut Removed, cx: &mut App) {
        let moved = moved_through(self.before, buffer, removed);
        let path = self.path;
        self.store
            .update(cx, |store, cx| store.set_file(&path, moved, cx));
    }
}

// ---- kept with the workspace -------------------------------------------------------

/// The store's breakpoints under `root`, as `layout.json` keeps them.
pub fn saved_under(root: &Path, store: &BreakpointStore) -> Vec<SavedBreakpoint> {
    store
        .files()
        .filter_map(|(path, list)| Some((path.strip_prefix(root).ok()?, list)))
        .flat_map(|(rel, list)| {
            list.iter().map(move |b| SavedBreakpoint {
                path: rel.to_path_buf(),
                line: b.line,
                enabled: b.enabled,
                condition: b.condition.clone(),
                log_message: b.log_message.clone(),
                hit_condition: b.hit_condition.clone(),
            })
        })
        .collect()
}

/// Saved breakpoints as the store's lists, by absolute path.
pub fn from_saved(
    root: &Path,
    saved: &[SavedBreakpoint],
) -> BTreeMap<PathBuf, Vec<LineBreakpoint>> {
    let mut files: BTreeMap<PathBuf, Vec<LineBreakpoint>> = BTreeMap::new();
    for s in saved {
        files
            .entry(root.join(&s.path))
            .or_default()
            .push(LineBreakpoint {
                line: s.line,
                enabled: s.enabled,
                condition: s.condition.clone(),
                log_message: s.log_message.clone(),
                hit_condition: s.hit_condition.clone(),
                verified: None,
            });
    }
    files
}

/// A workspace's breakpoints across launches (Fleet keeps them with the
/// workspace): loads the ones `layout.json` holds for `root` into the store
/// (a file another window already has some for keeps its own), then writes
/// them back whenever one under `root` changes. The exception breakpoints
/// are app-wide in the store, so the workspace's saved choice is loaded
/// over them and every change is written to each open workspace's entry.
/// Lives as long as `cx`'s entity (the workspace).
pub fn persist<T: 'static>(root: PathBuf, cx: &mut Context<T>) {
    let Some(store) = BreakpointStore::try_global(cx) else {
        return;
    };
    let saved = from_saved(&root, &Layout::load_breakpoints(&root));
    store.update(cx, |store, cx| {
        for (path, list) in saved {
            if store.in_file(&path).is_empty() {
                store.set_file(&path, list, cx);
            }
        }
        store.restore_exceptions(&Layout::load_exceptions(&root), cx);
    });
    cx.subscribe(
        &store,
        move |_, store, event: &BreakpointsEvent, cx| match event {
            BreakpointsEvent::Changed(path) => {
                if path.starts_with(&root) {
                    Layout::save_breakpoints(&root, &saved_under(&root, store.read(cx)));
                }
            }
            BreakpointsEvent::ExceptionsChanged => {
                Layout::save_exceptions(&root, store.read(cx).exception_breakpoints());
            }
        },
    )
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_menu_rows_follow_fleet() {
        let mut b = LineBreakpoint::at(5);
        let labels = |b: &LineBreakpoint| gutter_menu_rows(b).map(GutterMenuRow::label);
        assert_eq!(
            labels(&b),
            [
                "Remove Breakpoint",
                "Disable Breakpoint",
                "",
                "View All Breakpoints"
            ]
        );
        assert_eq!(gutter_menu_rows(&b)[2], GutterMenuRow::Separator);
        // The Breakpoints tool's rows: the first two only.
        assert_eq!(
            tool_menu_rows(&b).map(GutterMenuRow::label),
            ["Remove Breakpoint", "Disable Breakpoint"]
        );
        b.enabled = false;
        assert_eq!(labels(&b)[1], "Enable Breakpoint");
        assert_eq!(
            tool_menu_rows(&b).map(GutterMenuRow::label),
            ["Remove Breakpoint", "Enable Breakpoint"]
        );
    }

    /// The new line of each of `lines` (rows of `before`) after `edits` turned
    /// `before` into `after`; `None` for a deleted line.
    fn follow_lines(
        before: &Buffer,
        lines: &[usize],
        edits: &[Edit],
        after: &Buffer,
    ) -> Vec<Option<usize>> {
        let spans: Vec<Range<usize>> = lines.iter().map(|&l| line_span(before, l)).collect();
        follow_edits(&spans, edits)
            .into_iter()
            .map(|o| o.map(|o| after.offset_to_point(o).row))
            .collect()
    }

    fn edit(offset: usize, old_len: usize, new_len: usize) -> Edit {
        Edit {
            version: 0,
            offset,
            old_len,
            new_len,
        }
    }

    /// The lines of `before`'s rows `lines` after one edit replacing
    /// `range` (char offsets) with `text`.
    fn after_one(
        before: &str,
        lines: &[usize],
        range: Range<usize>,
        text: &str,
    ) -> Vec<Option<usize>> {
        let mut chars: Vec<char> = before.chars().collect();
        chars.splice(range.clone(), text.chars());
        let after: String = chars.into_iter().collect();
        follow_lines(
            &Buffer::new(before),
            lines,
            &[edit(range.start, range.len(), text.chars().count())],
            &Buffer::new(&after),
        )
    }

    const TEXT: &str = "zero\none\ntwo\nthree\n";

    #[test]
    fn lines_inserted_above_move_a_breakpoint_down() {
        // A line break typed at the end of line 0.
        assert_eq!(after_one(TEXT, &[2], 4..4, "\n"), vec![Some(3)]);
        // Two lines pasted at the top.
        assert_eq!(after_one(TEXT, &[2], 0..0, "a\nb\n"), vec![Some(4)]);
    }

    #[test]
    fn enter_at_the_start_of_its_line_moves_it_with_the_code() {
        // `two` starts at 9.
        assert_eq!(after_one(TEXT, &[2], 9..9, "\n"), vec![Some(3)]);
    }

    #[test]
    fn edits_inside_or_below_leave_it() {
        assert_eq!(after_one(TEXT, &[2], 10..10, "x\ny"), vec![Some(2)]);
        assert_eq!(after_one(TEXT, &[2], 13..19, ""), vec![Some(2)]);
        // Typing at its start keeps it on its line.
        assert_eq!(after_one(TEXT, &[2], 9..9, "x"), vec![Some(2)]);
    }

    #[test]
    fn lines_deleted_above_move_it_up() {
        // `one\n` deleted.
        assert_eq!(after_one(TEXT, &[2], 5..9, ""), vec![Some(1)]);
    }

    #[test]
    fn deleting_its_line_removes_it() {
        // `two\n` (the line and the break after it).
        assert_eq!(after_one(TEXT, &[2], 9..13, ""), vec![None]);
        // `\ntwo` (the break before it and the line).
        assert_eq!(after_one(TEXT, &[2], 8..12, ""), vec![None]);
        // Several lines with it.
        assert_eq!(
            after_one(TEXT, &[1, 2, 3], 5..13, ""),
            vec![None, None, Some(1)]
        );
        // A backspace on an empty line.
        assert_eq!(after_one("a\n\nb\n", &[1], 1..2, ""), vec![None]);
    }

    #[test]
    fn emptying_its_line_keeps_it() {
        assert_eq!(after_one(TEXT, &[2], 9..12, ""), vec![Some(2)]);
    }

    #[test]
    fn a_line_joined_to_the_one_above_takes_it_along() {
        // Backspace at the start of `two`.
        assert_eq!(after_one(TEXT, &[2], 8..9, ""), vec![Some(1)]);
        // A selection from inside `one` into `two`, typed over.
        assert_eq!(after_one(TEXT, &[2], 6..10, "x"), vec![Some(1)]);
    }

    #[test]
    fn a_whole_text_replacement_keeps_the_offset() {
        let after = "zero\none\ntwo\nthree!\n";
        let lines = follow_lines(
            &Buffer::new(TEXT),
            &[2, 3],
            &[edit(0, TEXT.chars().count(), after.chars().count())],
            &Buffer::new(after),
        );
        assert_eq!(lines, vec![Some(2), Some(3)]);
    }

    #[test]
    fn edits_apply_in_order() {
        // Two carets: a line inserted at the top, then line `one` deleted
        // (offsets of the text after the first edit).
        let lines = follow_lines(
            &Buffer::new(TEXT),
            &[2],
            &[edit(0, 0, 2), edit(7, 4, 0)],
            &Buffer::new("x\nzero\ntwo\nthree\n"),
        );
        assert_eq!(lines, vec![Some(2)]);
    }

    /// Where a breakpoint on `line` of `text` goes when `f` edits it the
    /// way the editor's commands do.
    fn through(
        text: &str,
        line: usize,
        f: impl FnOnce(&mut weep_text::Cursor, &mut Buffer),
    ) -> Option<usize> {
        let mut buffer = Buffer::new(text);
        let mut cursor = weep_text::Cursor::new(&buffer);
        let span = line_span(&buffer, line);
        let version = buffer.version();
        f(&mut cursor, &mut buffer);
        let edits = buffer.edits_since(version).unwrap();
        follow_edits(&[span], &edits)[0].map(|o| buffer.offset_to_point(o).row)
    }

    #[test]
    fn the_editor_commands_move_it_as_expected() {
        let text = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n";
        // `editor/delete-line` on it, and on the line above.
        assert_eq!(
            through(text, 2, |c, b| {
                c.goto(b, 2, 4);
                c.delete_line(b)
            }),
            None
        );
        assert_eq!(
            through(text, 2, |c, b| {
                c.goto(b, 1, 4);
                c.delete_line(b)
            }),
            Some(1)
        );
        // Enter at its first column, with the indenter.
        assert_eq!(
            through(text, 2, |c, b| {
                c.goto(b, 2, 0);
                c.newline_with(b, true)
            }),
            Some(3)
        );
        // Enter at the end of the line above.
        assert_eq!(
            through(text, 2, |c, b| {
                c.goto(b, 1, 14);
                c.newline_with(b, true)
            }),
            Some(3)
        );
        // Backspace at its first column joins it to the line above.
        assert_eq!(
            through(text, 2, |c, b| {
                c.goto(b, 2, 0);
                c.backspace(b)
            }),
            Some(1)
        );
        // Typing inside it.
        assert_eq!(
            through(text, 2, |c, b| {
                c.goto(b, 2, 8);
                c.insert(b, "mut ")
            }),
            Some(2)
        );
        // A line duplicated above it.
        assert_eq!(
            through(text, 2, |c, b| {
                c.goto(b, 1, 0);
                c.duplicate_line(b)
            }),
            Some(3)
        );
    }

    /// A document as `Document::edit` drives it: each step is one edit
    /// call, the breakpoints moved through it with the document's removed
    /// list.
    struct Doc {
        buffer: Buffer,
        cursor: weep_text::Cursor,
        list: Vec<LineBreakpoint>,
        removed: Removed,
    }

    impl Doc {
        fn new(text: &str, lines: &[usize]) -> Doc {
            let buffer = Buffer::new(text);
            let cursor = weep_text::Cursor::new(&buffer);
            let list = lines.iter().map(|&l| LineBreakpoint::at(l)).collect();
            Doc {
                buffer,
                cursor,
                list,
                removed: Removed::default(),
            }
        }

        fn edit(&mut self, f: impl FnOnce(&mut weep_text::Cursor, &mut Buffer)) {
            let before = Before::take(self.list.clone(), &self.buffer);
            f(&mut self.cursor, &mut self.buffer);
            self.list = normalized(moved_through(before, &self.buffer, &mut self.removed));
        }

        fn lines(&self) -> Vec<usize> {
            self.list.iter().map(|b| b.line).collect()
        }
    }

    const CODE: &str = "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n";

    #[test]
    fn move_line_carries_the_breakpoints_with_their_text() {
        let mut d = Doc::new(CODE, &[1, 2]);
        d.list[1].condition = Some("b > 1".into());
        // `let b` up over `let a`: both breakpoints follow their text.
        d.edit(|c, b| {
            c.goto(b, 2, 4);
            c.move_line(b, true)
        });
        assert_eq!(d.buffer.line(1), "    let b = 2;");
        assert_eq!(d.lines(), vec![1, 2]);
        assert_eq!(d.list[0].condition.as_deref(), Some("b > 1"));
        // Down twice: past `let a`, then past `let c`.
        d.edit(|c, b| c.move_line(b, false));
        d.edit(|c, b| c.move_line(b, false));
        assert_eq!(d.buffer.line(3), "    let b = 2;");
        assert_eq!(d.lines(), vec![1, 3]);
        assert_eq!(d.list[1].condition.as_deref(), Some("b > 1"));
        // Undo puts it back a row.
        d.edit(|c, b| c.undo(b));
        assert_eq!(d.lines(), vec![1, 2]);
        assert_eq!(d.list[1].condition.as_deref(), Some("b > 1"));
    }

    #[test]
    fn move_line_of_a_block_carries_each_line() {
        let mut d = Doc::new(CODE, &[1, 2, 3]);
        d.list[2].enabled = false;
        // `let b` and `let c` selected, moved up over `let a`.
        d.edit(|c, b| {
            let start = b.line_to_char(2);
            let end = b.line_to_char(3) + 3;
            c.select(b, start, end);
            c.move_line(b, true)
        });
        assert_eq!(d.buffer.line(3), "    let a = 1;");
        assert_eq!(d.lines(), vec![1, 2, 3]);
        // `let c`'s disabled one went to row 2, `let a`'s to row 3.
        assert!(!d.list[1].enabled);
        assert!(d.list[2].enabled);
    }

    #[test]
    fn undo_brings_back_a_deleted_lines_breakpoint() {
        let mut d = Doc::new(CODE, &[2, 3]);
        d.list[0].log_message = Some("b = {b}".into());
        d.edit(|c, b| {
            c.goto(b, 2, 4);
            c.delete_line(b)
        });
        assert_eq!(d.lines(), vec![2]);
        d.edit(|c, b| c.undo(b));
        assert_eq!(d.lines(), vec![2, 3]);
        assert_eq!(d.list[0].log_message.as_deref(), Some("b = {b}"));
        assert!(d.removed.is_empty());
        // Redo deletes it again, a second undo brings it again.
        d.edit(|c, b| c.redo(b));
        assert_eq!(d.lines(), vec![2]);
        d.edit(|c, b| c.undo(b));
        assert_eq!(d.lines(), vec![2, 3]);
    }

    #[test]
    fn undo_restores_after_later_steps_are_undone_too() {
        let mut d = Doc::new(CODE, &[2]);
        // Several lines deleted at once, then a line typed at the top.
        d.edit(|c, b| {
            let start = b.line_to_char(1);
            let end = b.line_to_char(4);
            c.select(b, start, end);
            c.backspace(b)
        });
        assert!(d.lines().is_empty());
        d.edit(|c, b| {
            c.goto(b, 0, 0);
            c.insert(b, "// x\n")
        });
        d.edit(|c, b| c.undo(b));
        assert!(
            d.lines().is_empty(),
            "undoing the typing alone brings nothing"
        );
        d.edit(|c, b| c.undo(b));
        assert_eq!(d.buffer.text(), CODE);
        assert_eq!(d.lines(), vec![2]);
    }

    #[test]
    fn a_backspace_on_an_empty_line_comes_back_on_undo() {
        let mut d = Doc::new("a\n\nb\n", &[1]);
        d.edit(|c, b| {
            c.goto(b, 1, 0);
            c.backspace(b)
        });
        assert!(d.lines().is_empty());
        d.edit(|c, b| c.undo(b));
        assert_eq!(d.lines(), vec![1]);
    }

    #[test]
    fn a_new_step_after_the_undo_forgets_the_deleted_ones() {
        let mut d = Doc::new(CODE, &[2]);
        d.edit(|c, b| {
            c.goto(b, 2, 4);
            c.delete_line(b)
        });
        assert!(!d.removed.is_empty());
        d.edit(|c, b| c.undo(b));
        // Deleted again, then something else typed and that undone: the
        // delete is still on the stack, so its breakpoint waits.
        d.edit(|c, b| c.delete_line(b));
        d.edit(|c, b| c.insert(b, "x"));
        d.edit(|c, b| c.undo(b));
        assert!(d.lines().is_empty());
        assert!(!d.removed.is_empty());
        // A different edit replaces the delete's redo: nothing to bring back.
        d.edit(|c, b| c.undo(b));
        assert_eq!(d.lines(), vec![2]);
        d.edit(|c, b| c.insert(b, "y"));
        assert!(d.removed.is_empty());
    }

    #[test]
    fn a_reload_keeps_the_breakpoint_on_its_text() {
        let mut d = Doc::new(CODE, &[2]);
        let reloaded = format!("// header\n{CODE}");
        d.edit(|_, b| b.set_text(&reloaded));
        assert_eq!(d.lines(), vec![3]);
    }

    #[test]
    fn toggling_keeps_the_list_sorted() {
        let mut list = Vec::new();
        assert!(toggle_line(&mut list, 5));
        assert!(toggle_line(&mut list, 1));
        assert!(toggle_line(&mut list, 3));
        let lines: Vec<usize> = list.iter().map(|b| b.line).collect();
        assert_eq!(lines, vec![1, 3, 5]);
        assert!(!toggle_line(&mut list, 3));
        let lines: Vec<usize> = list.iter().map(|b| b.line).collect();
        assert_eq!(lines, vec![1, 5]);
    }

    #[test]
    fn breakpoints_merged_onto_one_line_keep_the_first() {
        let mut a = LineBreakpoint::at(4);
        a.condition = Some("x > 1".into());
        let list = normalized(vec![
            LineBreakpoint::at(7),
            a.clone(),
            LineBreakpoint::at(4),
        ]);
        assert_eq!(list, vec![a, LineBreakpoint::at(7)]);
    }

    #[test]
    fn looks_and_tooltips() {
        let mut b = LineBreakpoint::at(0);
        assert_eq!(b.look(), Look::Enabled);
        b.verified = Some(false);
        assert_eq!(b.look(), Look::Unverified);
        b.enabled = false;
        assert_eq!(b.look(), Look::Disabled);
        b.log_message = Some("hit".into());
        assert_eq!(b.tooltip().1.as_deref(), Some("Log: hit"));
        b.condition = Some("n > 2".into());
        b.hit_condition = Some("3".into());
        assert_eq!(
            b.tooltip().1.as_deref(),
            Some("Condition: n > 2, Log: hit, Hit count: 3")
        );
        b.condition = Some("  ".into());
        b.log_message = None;
        assert_eq!(b.tooltip().1.as_deref(), Some("Hit count: 3"));
    }

    #[test]
    fn saved_breakpoints_come_back_under_the_root() {
        let root = Path::new("/w");
        let saved = vec![
            SavedBreakpoint {
                path: PathBuf::from("src/main.rs"),
                line: 9,
                enabled: false,
                condition: Some("n == 2".into()),
                log_message: None,
                hit_condition: Some("2".into()),
            },
            SavedBreakpoint {
                path: PathBuf::from("src/main.rs"),
                line: 2,
                enabled: true,
                condition: None,
                log_message: None,
                hit_condition: None,
            },
        ];
        let files = from_saved(root, &saved);
        let list = normalized(files[Path::new("/w/src/main.rs")].clone());
        assert_eq!(list.iter().map(|b| b.line).collect::<Vec<_>>(), vec![2, 9]);
        assert!(!list[1].enabled);
        assert_eq!(list[1].condition.as_deref(), Some("n == 2"));
        assert_eq!(list[1].hit_condition.as_deref(), Some("2"));
        // And back, the hit count with them.
        let mut store = BreakpointStore::default();
        store.files.insert(PathBuf::from("/w/src/main.rs"), list);
        let again = saved_under(root, &store);
        assert_eq!(again[1].hit_condition.as_deref(), Some("2"));
    }
}
