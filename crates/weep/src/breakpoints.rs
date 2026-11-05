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
//! ([`follow_edits`]). A workspace keeps its breakpoints across launches in
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

    /// The gutter tooltip's title and detail (the condition or the log
    /// message). Estimate: IntelliJ's wording; no probe has hovered one.
    pub fn tooltip(&self) -> (String, Option<String>) {
        let title = match self.look() {
            Look::Enabled => "Line breakpoint",
            Look::Disabled => "Line breakpoint (disabled)",
            Look::Unverified => "Line breakpoint (not verified)",
        };
        let detail = match (&self.condition, &self.log_message) {
            (Some(c), _) if !c.is_empty() => Some(format!("Condition: {c}")),
            (_, Some(l)) if !l.is_empty() => Some(format!("Log: {l}")),
            _ => None,
        };
        (title.to_string(), detail)
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
}

#[derive(Default)]
pub struct BreakpointStore {
    files: BTreeMap<PathBuf, Vec<LineBreakpoint>>,
}

impl EventEmitter<BreakpointsEvent> for BreakpointStore {}

struct GlobalBreakpoints(Entity<BreakpointStore>);

impl Global for GlobalBreakpoints {}

impl BreakpointStore {
    pub fn init(cx: &mut App) {
        let store = cx.new(|_| BreakpointStore::default());
        cx.set_global(GlobalBreakpoints(store));
    }

    pub fn global(cx: &App) -> Entity<BreakpointStore> {
        cx.global::<GlobalBreakpoints>().0.clone()
    }

    /// The store, when the app made one (a document outside the app has none).
    pub fn try_global(cx: &App) -> Option<Entity<BreakpointStore>> {
        cx.try_global::<GlobalBreakpoints>().map(|g| g.0.clone())
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

/// The menu for the breakpoint on `line` of `path` (none without one): its
/// rows act on that breakpoint, not on the carets' lines. Fleet shows the
/// remove chord (⌫) beside "Remove Breakpoint" and no other.
pub fn gutter_menu(path: &Path, line: usize, cx: &App) -> Vec<MenuEntry> {
    let Some(breakpoint) =
        BreakpointStore::try_global(cx).and_then(|s| s.read(cx).at(path, line).cloned())
    else {
        return Vec::new();
    };
    gutter_menu_rows(&breakpoint)
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

/// Moves breakpoint lines through `edits`. Each line is given as the char
/// span of its text (without the line break) before the edits; the answer
/// is the char offset its start lands on, or `None` when an edit deleted
/// the line.
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
/// reload from disk keeps the offset).
pub fn follow_edits(spans: &[Range<usize>], edits: &[Edit]) -> Vec<Option<usize>> {
    spans
        .iter()
        .map(|span| {
            let (mut s, mut t) = (span.start, span.end);
            for e in edits {
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
                        return None;
                    }
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
                }
                t = t.max(s);
            }
            Some(s)
        })
        .collect()
}

/// Line `line`'s text as a char span (the last line for one past the end).
fn line_span(buffer: &Buffer, line: usize) -> Range<usize> {
    let row = line.min(buffer.line_count().saturating_sub(1));
    let start = buffer.line_to_char(row);
    start..start + buffer.line_len(row)
}

/// A document's breakpoints taken before an edit (`Document::edit`), to be
/// moved through it afterwards.
pub struct Tracked {
    store: Entity<BreakpointStore>,
    path: PathBuf,
    list: Vec<LineBreakpoint>,
    spans: Vec<Range<usize>>,
    version: u64,
}

impl Tracked {
    /// The file's breakpoints and their lines' spans in `buffer`; `None`
    /// for a file without any (or a document with no path).
    pub fn before_edit(path: Option<&Path>, buffer: &Buffer, cx: &App) -> Option<Tracked> {
        let path = path?;
        let store = BreakpointStore::try_global(cx)?;
        let list = store.read(cx).in_file(path).to_vec();
        if list.is_empty() {
            return None;
        }
        let spans = list.iter().map(|b| line_span(buffer, b.line)).collect();
        Some(Tracked {
            store,
            path: path.to_path_buf(),
            list,
            spans,
            version: buffer.version(),
        })
    }

    /// Moves the breakpoints through the edits `buffer` made since, and
    /// gives the store the moved list when anything changed.
    pub fn after_edit(self, buffer: &Buffer, cx: &mut App) {
        let last = buffer.line_count().saturating_sub(1);
        let moved: Vec<LineBreakpoint> = match buffer.edits_since(self.version) {
            Some(edits) => self
                .list
                .into_iter()
                .zip(follow_edits(&self.spans, &edits))
                .filter_map(|(b, at)| {
                    at.map(|o| LineBreakpoint {
                        line: buffer.offset_to_point(o).row,
                        ..b
                    })
                })
                .collect(),
            // Too many edits for the log: the lines stay, within the text.
            None => self
                .list
                .into_iter()
                .map(|b| LineBreakpoint {
                    line: b.line.min(last),
                    ..b
                })
                .collect(),
        };
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
                verified: None,
            });
    }
    files
}

/// A workspace's breakpoints across launches (Fleet keeps them with the
/// workspace): loads the ones `layout.json` holds for `root` into the store
/// (a file another window already has some for keeps its own), then writes
/// them back whenever one under `root` changes. Lives as long as `cx`'s
/// entity (the workspace).
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
    });
    cx.subscribe(&store, move |_, store, event: &BreakpointsEvent, cx| {
        let BreakpointsEvent::Changed(path) = event;
        if path.starts_with(&root) {
            Layout::save_breakpoints(&root, &saved_under(&root, store.read(cx)));
        }
    })
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
        b.enabled = false;
        assert_eq!(labels(&b)[1], "Enable Breakpoint");
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
            },
            SavedBreakpoint {
                path: PathBuf::from("src/main.rs"),
                line: 2,
                enabled: true,
                condition: None,
                log_message: None,
            },
        ];
        let files = from_saved(root, &saved);
        let list = normalized(files[Path::new("/w/src/main.rs")].clone());
        assert_eq!(list.iter().map(|b| b.line).collect::<Vec<_>>(), vec![2, 9]);
        assert!(!list[1].enabled);
        assert_eq!(list[1].condition.as_deref(), Some("n == 2"));
    }
}
