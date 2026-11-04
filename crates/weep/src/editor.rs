//! The code editor: an [`EditorView`] entity holding a cursor (one or more
//! carets) and a scroll position on a shared [`Document`], and the
//! [`EditorElement`] that lays its visible lines out and paints them,
//! Fleet-style: a gutter of right-aligned line numbers and fold chevrons, the
//! current line washed across the whole width, the caret two pixels wide, a
//! thin scrollbar on the right with marks. Two views on one document (a file
//! in two splits) edit the same buffer; each keeps its own cursor, folds and
//! scroll, which follow the other's edits.
//!
//! Buffer rows map to display rows through a [`DisplayMap`]: a folded block
//! hides its rows, a soft-wrapped line takes several. Text is painted a
//! display row at a time through gpui's shaper; only the rows in view are
//! shaped each frame, so the cost is the viewport, not the file.

pub mod smart;

use crate::actions::*;
use crate::document::{Document, DocumentEvent, Problem, Severity};
use crate::settings::AppSettings;
use crate::skin::{self, Cell, Skin};
use crate::theme::{
    metrics::{CARET_BLINK_MS, EDITOR_OVERSCROLL_LINES},
    Metrics, Theme,
};
use crate::tooltip::Tooltip;
use gpui::{
    deferred, div, fill, point, prelude::*, px, relative, size, App, Bounds, BoxShadow,
    ClipboardItem, ContentMask, Context, Corners, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font,
    FontFeatures, FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString, Style, Task, TextAlign, TextRun,
    UTF16Selection, Window,
};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use weep_settings::{CaretShape, LineNumbers, Settings};
use weep_syntax::{enclosing_heads, Fold, Language};
use weep_text::analysis;
use weep_text::{Buffer, Cursor, Point as TextPoint, Selection};

/// What the view tells the workspace: the document changed under it (so a
/// preview tab is promoted), was saved, or text went to the clipboard.
#[derive(Clone, Debug)]
pub enum EditorEvent {
    Changed,
    Saved,
    Copied(String),
    /// A right click at this window position: the workspace opens the editor menu.
    ContextMenu(Point<Pixels>),
    /// The line-status popup's Diff button: the workspace opens the Changed
    /// Files tab on this file.
    ShowDiff,
    /// A click on a run icon in `.fleet/run.json`'s gutter: the workspace
    /// runs that configuration (its index in the file).
    RunConfiguration(usize),
    /// M7: what the view asks the workspace to ask the document's language
    /// server. `Completion` after a typed char or `completion`;
    /// `SignatureHelp` after `(` or `show-parameter-info`; `Hover` for the
    /// documentation at a char offset (`show-documentation`, or the pointer
    /// resting there); `GoToDefinition` for a Ctrl+click; `ProblemFixes`
    /// when a problem's card expands; `ApplyProblemFix` on one of its rows;
    /// `ResolveCompletion` for an item's documentation; `Accepted` after a
    /// completion with the item's command, if any.
    Completion {
        explicit: bool,
    },
    SignatureHelp,
    Hover {
        offset: usize,
        mouse: bool,
    },
    GoToDefinition {
        offset: usize,
    },
    ProblemFixes(usize),
    ApplyProblemFix {
        problem: usize,
        fix: usize,
    },
    ResolveCompletion(usize),
    Accepted {
        command: Option<weep_lsp::lsp_types::Command>,
    },
}

/// The editor's sizes at the current zoom and settings: `editor.fontSize`,
/// `editor.lineSpacing` and the measured gutter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics {
    pub font_size: f32,
    pub line_height: f32,
    pub gutter_width: f32,
    pub gutter_number_right: f32,
    pub caret_width: f32,
    pub scrollbar_thumb: f32,
    pub scrollbar_thumb_min: f32,
    pub selection_radius: f32,
    pub selection_empty_line_width: f32,
    pub occurrence_inset: f32,
    pub occurrence_radius: f32,
    pub scrollbar_mark_width: f32,
    pub scrollbar_mark_height: f32,
    pub fold_chevron_x: f32,
    pub breakpoint_hint_x: f32,
    pub breakpoint_hint_size: f32,
    pub run_icon_x: f32,
    pub problem_underline: f32,
    pub problem_underline_bottom: f32,
    pub line_status_x: f32,
    pub line_status_width: f32,
    pub line_status_hover_width: f32,
    pub line_status_active_width: f32,
    pub line_status_hit: f32,
    pub line_status_popup_above: f32,
    pub line_status_popup_left: f32,
    pub line_status_popup_width: f32,
    pub line_status_popup_height: f32,
    pub blame_width: f32,
    pub blame_text_x: f32,
    pub blame_bar_x: f32,
    pub blame_bar_width: f32,
    pub icon_size: f32,
    pub folded_mark_cells: f32,
    pub folded_mark_radius: f32,
    pub sticky_shadow: f32,
    pub wrap_indent: f32,
    pub centered_columns: f32,
    /// The tui skin's cell while that skin is on: the editor then sits on
    /// its grid (`on_grid`), and this is the frame's cached skin.
    pub cell: Option<Cell>,
}

/// The tui gutter, in cells from the left: the run icon (1), the line
/// numbers (`digits`), the line status (1), the fold mark (1) and a space.
const TUI_GUTTER_EXTRA: usize = 4;
/// The tui blame column, in cells: the label and a space after it.
const TUI_BLAME_COLS: usize = 18;

/// The tui gutter's digit columns: the last line number's digits, at least
/// 3 (so a file growing past 99 lines keeps its gutter); none without
/// line numbers.
fn tui_digits(line_count: usize, numbers: bool) -> usize {
    if !numbers {
        return 0;
    }
    line_count.max(1).to_string().len().max(3)
}

/// `v` on a whole multiple of `pitch` (a row or a column).
fn snap_to(v: f32, pitch: f32) -> f32 {
    if pitch <= 0. {
        return v;
    }
    (v / pitch).round() * pitch
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A blame line's day (`2025-12-31` once `Repo::blame` swapped it in, or
/// unix seconds) as days since the epoch.
fn blame_days(date: &str) -> Option<i64> {
    if let Ok(t) = date.parse::<i64>() {
        return Some(t.div_euclid(86_400));
    }
    let mut parts = date.get(..10)?.split('-');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.parse().ok()?;
    Some(days_from_civil(y, m, d))
}

/// An age in days as the tui blame column prints it: `3d`, `5w`, `4mo`, `2y`.
fn short_age(days: i64) -> String {
    let days = days.max(0);
    if days < 14 {
        format!("{days}d")
    } else if days < 60 {
        format!("{}w", days / 7)
    } else if days < 730 {
        format!("{}mo", days / 30)
    } else {
        format!("{}y", days / 365)
    }
}

/// The tui blame column's label, `author 2d` in exactly `cols` chars: the
/// author truncated with `…` so the age always fits, padded on the right.
fn blame_cell_label(author: &str, age: &str, cols: usize) -> String {
    let age_cols = age.chars().count();
    let room = cols.saturating_sub(age_cols + 1);
    let mut name: String = author.chars().take(room).collect();
    if author.chars().count() > room && room > 0 {
        name = author.chars().take(room - 1).collect();
        name.push('…');
    }
    let pad = cols.saturating_sub(name.chars().count() + age_cols);
    format!("{name}{}{age}", " ".repeat(pad))
}

/// Today as days since the epoch (UTC; the blame ages are whole days).
fn today_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86_400) as i64)
        .unwrap_or(0)
}

/// What the tui frame's right edge paints for the view: the scroll bar in
/// rows (offset, visible, total with the overscroll) and its marks as
/// display rows over `rows` display rows, with their colours.
#[derive(Clone, Debug, Default)]
struct TuiBar {
    offset: f32,
    visible: f32,
    total: f32,
    rows: usize,
    marks: Vec<(usize, Hsla)>,
}

/// What a find bar has found in this view's document, for painting: every
/// match and the one the bar is on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Search {
    pub matches: Vec<Range<usize>>,
    pub current: Option<usize>,
}

/// The `editor.*` settings the view paints by, read once per frame.
#[derive(Clone, Debug, PartialEq)]
struct ViewOptions {
    active_line: bool,
    indent_guides: bool,
    foldings: bool,
    scrollbar_marks: bool,
    line_numbers: LineNumbers,
    caret_shape: CaretShape,
    caret_blink: bool,
    whitespace: bool,
    guides: Vec<usize>,
    show_guides: bool,
    soft_wrap: bool,
    wrap_by_guide: bool,
    sticky_lines: bool,
    sticky_max: usize,
    distraction_free: bool,
    centered: bool,
    font_weight: f32,
    font_features: Vec<String>,
    ligatures: bool,
    indenter: bool,
    camel_humps: bool,
    /// M6: `editor.renderRunInGutter`, `ff.runIconsAfterLineNumbers`,
    /// `editor.markupWithoutSmartMode`, `editor.showFileStatus`,
    /// `editor.showTooltipOnHover`.
    run_in_gutter: bool,
    run_after_numbers: bool,
    markup: bool,
    file_status: bool,
    problem_tooltip: bool,
}

impl ViewOptions {
    fn from(settings: &Settings) -> ViewOptions {
        ViewOptions {
            active_line: settings.editor_active_line_highlight_enabled(),
            indent_guides: settings.editor_show_indent_guides(),
            foldings: settings.editor_show_foldings(),
            scrollbar_marks: settings.editor_show_scrollbar_marks(),
            line_numbers: settings.editor_line_numbers(),
            caret_shape: settings.editor_caret_shape(),
            caret_blink: settings.editor_caret_blinking_enabled(),
            whitespace: settings.editor_render_whitespace(),
            guides: settings.editor_guides(),
            show_guides: settings.editor_show_visual_guides(),
            soft_wrap: settings.editor_soft_wrap_enabled(),
            wrap_by_guide: settings.editor_guide_to_soft_wrap_by(),
            sticky_lines: settings.editor_show_sticky_lines(),
            sticky_max: settings.editor_sticky_lines_max_count(),
            distraction_free: settings.editor_distraction_free_mode(),
            centered: settings.editor_center_editor_horizontally(),
            font_weight: settings.editor_font_weight(),
            font_features: settings.editor_font_features(),
            ligatures: settings.editor_ligatures(),
            indenter: settings.enable_indenter(),
            camel_humps: settings.editor_honor_camel_humps(),
            run_in_gutter: settings.editor_render_run_in_gutter(),
            run_after_numbers: settings.ff_run_icons_after_line_numbers(),
            markup: settings.editor_markup_without_smart_mode(),
            file_status: settings.editor_show_file_status(),
            problem_tooltip: settings.editor_show_tooltip_on_hover(),
        }
    }

    /// The editor's font: `editor.fontWeight`, `editor.fontFeatures` and
    /// `editor.ligatures` (off: `calt` and `liga` disabled).
    fn font_features(&self) -> FontFeatures {
        let mut tags: Vec<(String, u32)> =
            self.font_features.iter().map(|t| (t.clone(), 1)).collect();
        if !self.ligatures {
            tags.push(("calt".into(), 0));
            tags.push(("liga".into(), 0));
        }
        FontFeatures(Arc::new(tags))
    }
}

impl TextMetrics {
    pub fn current(cx: &App) -> TextMetrics {
        let m = cx.global::<Metrics>();
        let (font_size, line_height) = m.editor_text(&cx.global::<AppSettings>().settings);
        let tm = TextMetrics {
            font_size,
            line_height,
            gutter_width: m.gutter_width,
            gutter_number_right: m.gutter_number_right,
            caret_width: m.caret_width,
            scrollbar_thumb: m.scrollbar_thumb,
            scrollbar_thumb_min: m.scrollbar_thumb_min,
            selection_radius: m.selection_radius,
            selection_empty_line_width: m.selection_empty_line_width,
            occurrence_inset: m.occurrence_inset,
            occurrence_radius: m.occurrence_radius,
            scrollbar_mark_width: m.scrollbar_mark_width,
            scrollbar_mark_height: m.scrollbar_mark_height,
            fold_chevron_x: m.fold_chevron_x,
            breakpoint_hint_x: m.breakpoint_hint_x,
            breakpoint_hint_size: m.breakpoint_hint_size,
            run_icon_x: m.run_icon_x,
            problem_underline: m.problem_underline,
            problem_underline_bottom: m.problem_underline_bottom,
            line_status_x: m.line_status_x,
            line_status_width: m.line_status_width,
            line_status_hover_width: m.line_status_hover_width,
            line_status_active_width: m.line_status_active_width,
            line_status_hit: m.line_status_hit,
            line_status_popup_above: m.line_status_popup_above,
            line_status_popup_left: m.line_status_popup_left,
            line_status_popup_width: m.line_status_popup_width,
            line_status_popup_height: m.line_status_popup_height,
            blame_width: m.blame_width,
            blame_text_x: m.blame_text_x,
            blame_bar_x: m.blame_bar_x,
            blame_bar_width: m.blame_bar_width,
            icon_size: m.icon_size,
            folded_mark_cells: m.folded_mark_cells,
            folded_mark_radius: m.folded_mark_radius,
            sticky_shadow: m.sticky_shadow,
            wrap_indent: m.wrap_indent,
            centered_columns: m.centered_columns,
            cell: None,
        };
        if Skin::current(cx) == Skin::Tui {
            tm.on_grid(Cell::current(cx), 3)
        } else {
            tm
        }
    }

    /// The tui gutter for `digits` line-number columns (from the line
    /// count, `tui_digits`); the Fleet metrics are left as they are.
    pub fn with_digits(self, digits: usize) -> TextMetrics {
        match self.cell {
            Some(cell) => self.on_grid(cell, digits),
            None => self,
        }
    }

    /// The editor on the tui skin's grid: the line height is a cell's,
    /// the gutter's parts are whole cells (`TUI_GUTTER_EXTRA`), and boxes
    /// have no radius or inset. Idempotent, so it may run twice a frame.
    fn on_grid(mut self, cell: Cell, digits: usize) -> TextMetrics {
        let w = cell.w;
        let col = |n: usize| n as f32 * w;
        self.font_size = cell.font_size;
        self.line_height = cell.h;
        self.run_icon_x = 0.;
        self.breakpoint_hint_x = 0.;
        self.breakpoint_hint_size = w;
        self.gutter_number_right = col(1 + digits);
        self.line_status_x = col(1 + digits);
        self.line_status_width = w / 3.;
        self.line_status_hover_width = w;
        self.line_status_active_width = w;
        self.line_status_hit = 0.;
        self.fold_chevron_x = col(2 + digits);
        self.gutter_width = col(digits + TUI_GUTTER_EXTRA);
        self.icon_size = w;
        self.scrollbar_thumb = 0.;
        self.scrollbar_thumb_min = cell.h;
        self.selection_radius = 0.;
        self.selection_empty_line_width = w;
        self.occurrence_inset = 0.;
        self.occurrence_radius = 0.;
        self.folded_mark_cells = 1.;
        self.folded_mark_radius = 0.;
        self.sticky_shadow = 0.;
        self.wrap_indent = snap_to(self.wrap_indent, w);
        self.blame_width = col(TUI_BLAME_COLS);
        self.blame_text_x = 0.;
        self.blame_bar_x = 0.;
        self.blame_bar_width = 0.;
        self.cell = Some(cell);
        self
    }
}

// ---- the display map ---------------------------------------------------------

/// A block this view has folded: its head row keeps the text around `inner`
/// (char offsets), the rows after it up to `end_row` are hidden.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Folded {
    start_row: usize,
    end_row: usize,
    inner: Range<usize>,
}

impl Folded {
    fn from_fold(fold: &Fold, buffer: &Buffer) -> Folded {
        Folded {
            start_row: fold.start_row,
            end_row: fold.end_row,
            inner: buffer.byte_to_char(fold.inner.start)..buffer.byte_to_char(fold.inner.end),
        }
    }
}

/// Per buffer row: where it starts in display rows and how its characters
/// split into wrapped segments.
#[derive(Clone, Debug, Default)]
struct RowInfo {
    display_start: usize,
    /// The char column each segment starts at; `[0]` for an unwrapped row.
    /// Empty for a hidden row.
    segments: Vec<usize>,
}

/// Buffer rows to display rows: folded blocks collapse to their head, a
/// soft-wrapped line takes one display row per segment.
#[derive(Clone, Debug, Default)]
struct DisplayMap {
    version: Option<u64>,
    wrap_cols: Option<usize>,
    folds: Vec<Folded>,
    rows: Vec<RowInfo>,
    total: usize,
}

/// Where a line breaks when soft-wrapped at `cols` display columns: after the
/// last blank at or before the limit, else hard at the limit (`line` is the
/// row's text with tabs expanded, as the shaper sees it).
fn wrap_points(line: &str, tab_width: usize, cols: usize) -> Vec<usize> {
    let cols = cols.max(1);
    let mut starts = vec![0];
    let mut seg_start_char = 0;
    let mut column = 0;
    let mut last_blank: Option<usize> = None;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let width = if c == '\t' {
            tab_width - column % tab_width
        } else {
            1
        };
        if column + width > cols && i > seg_start_char {
            let at = match last_blank {
                Some(b) if b > seg_start_char => b + 1,
                _ => i,
            };
            starts.push(at);
            seg_start_char = at;
            column = 0;
            last_blank = None;
            i = at;
            continue;
        }
        if c == ' ' || c == '\t' {
            last_blank = Some(i);
        }
        column += width;
        i += 1;
    }
    starts
}

impl DisplayMap {
    fn rebuild(&mut self, buffer: &Buffer, wrap_cols: Option<usize>) {
        let line_count = buffer.line_count();
        let mut rows = Vec::with_capacity(line_count);
        let mut next = 0;
        let mut fold_ix = 0;
        let mut hidden_until: Option<usize> = None;
        let folds = &self.folds;
        for row in 0..line_count {
            if hidden_until.is_some_and(|end| row <= end) {
                rows.push(RowInfo {
                    display_start: next,
                    segments: Vec::new(),
                });
                continue;
            }
            hidden_until = None;
            while fold_ix < folds.len() && folds[fold_ix].start_row < row {
                fold_ix += 1;
            }
            if fold_ix < folds.len() && folds[fold_ix].start_row == row {
                hidden_until = Some(folds[fold_ix].end_row);
            }
            let segments = match wrap_cols {
                Some(cols) if hidden_until.is_none() => {
                    wrap_points(&buffer.line(row), buffer.indent.width, cols)
                }
                _ => vec![0],
            };
            next += segments.len();
            rows.push(RowInfo {
                display_start: next - segments.len(),
                segments,
            });
        }
        self.rows = rows;
        self.total = next;
        self.version = Some(buffer.version());
        self.wrap_cols = wrap_cols;
    }

    fn is_identity(&self) -> bool {
        self.folds.is_empty() && self.wrap_cols.is_none()
    }

    fn is_hidden(&self, row: usize) -> bool {
        self.rows.get(row).is_some_and(|r| r.segments.is_empty())
    }

    /// The segment of `row` holding `column`.
    fn segment_of(&self, row: usize, column: usize) -> usize {
        match self.rows.get(row) {
            Some(r) if !r.segments.is_empty() => r
                .segments
                .partition_point(|s| *s <= column)
                .saturating_sub(1),
            _ => 0,
        }
    }

    fn display_row(&self, row: usize, column: usize) -> usize {
        match self.rows.get(row) {
            Some(r) => r.display_start + self.segment_of(row, column),
            None => self.total,
        }
    }

    /// The buffer row and segment shown at display row `d`.
    fn at(&self, d: usize) -> (usize, usize) {
        if self.rows.is_empty() {
            return (0, 0);
        }
        let d = d.min(self.total.saturating_sub(1));
        let ix = self
            .rows
            .partition_point(|r| r.display_start <= d)
            .saturating_sub(1);
        // Hidden rows share the start of the row after them; back up to the
        // visible row that owns `d`.
        let mut row = ix;
        while row > 0 && self.rows[row].segments.is_empty() {
            row -= 1;
        }
        let seg =
            (d - self.rows[row].display_start).min(self.rows[row].segments.len().saturating_sub(1));
        (row, seg)
    }

    /// The char columns a segment spans.
    fn segment_range(&self, buffer: &Buffer, row: usize, seg: usize) -> Range<usize> {
        let info = &self.rows[row];
        let start = info.segments.get(seg).copied().unwrap_or(0);
        let end = info
            .segments
            .get(seg + 1)
            .copied()
            .unwrap_or_else(|| buffer.line_len(row));
        start..end
    }

    fn fold_at_head(&self, row: usize) -> Option<&Folded> {
        self.folds.iter().find(|f| f.start_row == row)
    }
}

// ---- the view ------------------------------------------------------------------

pub struct EditorView {
    pub document: Entity<Document>,
    /// This view's carets and selections on the document.
    pub cursor: Cursor,
    /// Scroll offsets in pixels; `scroll_y` is over display rows.
    scroll_y: Pixels,
    scroll_x: Pixels,
    /// The bounds the text area last painted into, for page sizes and hit tests.
    last_bounds: Bounds<Pixels>,
    /// The sizes the last frame painted with (zoom and settings).
    tm: TextMetrics,
    options: ViewOptions,
    focus_handle: FocusHandle,
    dragging: bool,
    marked: Option<Range<usize>>,
    /// The find bar's matches, when one is open on this view.
    pub search: Option<Search>,
    /// The identifier at the caret and where else it occurs, for the buffer
    /// version and word it was computed for.
    occurrences: Option<(u64, String, Vec<Range<usize>>)>,
    map: DisplayMap,
    /// The display column the caret wants when moving through shorter rows.
    goal_column: Option<usize>,
    /// `expand-selection` steps, so `shrink-selection` retraces them.
    expand_history: Vec<Selection>,
    expand_last: Option<Selection>,
    /// The pointer is over the gutter: fold chevrons show.
    gutter_hovered: bool,
    hovered_row: Option<usize>,
    /// The git line-status bar under the pointer, and the one whose popup
    /// (↓ ↑ Revert Diff ×) is open, as indices into the document's hunks.
    status_hovered: Option<usize>,
    status_popup: Option<usize>,
    /// `git/toggle-blame`: one entry per line of the file on disk, shown in
    /// a column at the island's left while set.
    blame: Option<Vec<weep_git::BlameLine>>,
    /// The caret's blink: shown, and the epoch of the running timer.
    caret_visible: bool,
    blink_epoch: u64,
    blink_task: Option<Task<()>>,
    /// Where the sticky lines were last painted, for clicks on them.
    sticky_rows: Vec<usize>,
    /// One space's measured advance (the wrap width and the guides use it).
    space_px: Pixels,
    /// A preview (the Goto panel's): no edit changes the buffer.
    pub read_only: bool,
    /// M6: the rows of `.fleet/run.json`'s configurations (row, index) for
    /// the gutter's run icons, per buffer version.
    run_rows: Option<(u64, Vec<(usize, usize)>)>,
    /// The problems widget is open; the problem whose card is expanded.
    pub problems_open: bool,
    problem_expanded: Option<usize>,
    /// The problem under the pointer, where the pointer is, and whether the
    /// hover delay has passed (`editor.showTooltipOnHover`).
    problem_hovered: Option<usize>,
    hover_position: Point<Pixels>,
    problem_tooltip_shown: bool,
    hover_epoch: u64,
    hover_task: Option<Task<()>>,
    /// M7: Smart Mode's popups (completion, parameter info, documentation)
    /// and what the workspace told the view about its server.
    smart: smart::SmartState,
    /// The tui skin: the wheel's remainder short of a whole row or column,
    /// the scroll bar and its marks for the frame's right edge, a drag on
    /// it, the problem counts the frame's badge shows (errors, warnings,
    /// all) and the pointer over the blame column.
    scroll_rest: Point<Pixels>,
    tui_bar: TuiBar,
    tui_bar_drag: bool,
    badge: (usize, usize, usize),
    blame_hovered: bool,
}

impl EventEmitter<EditorEvent> for EditorView {}

impl Focusable for EditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn bind_keys(cx: &mut App) {
    // Fallbacks for the keys every editor has; the Fleet keymap in
    // `keymap.rs` binds the rest and may override these.
    use gpui::KeyBinding;
    let ctx = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, ctx),
        KeyBinding::new("down", MoveDown, ctx),
        KeyBinding::new("left", MoveLeft, ctx),
        KeyBinding::new("right", MoveRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("shift-home", SelectHome, ctx),
        KeyBinding::new("shift-end", SelectEnd, ctx),
        KeyBinding::new("pageup", PageUp, ctx),
        KeyBinding::new("pagedown", PageDown, ctx),
        KeyBinding::new("shift-pageup", SelectPageUp, ctx),
        KeyBinding::new("shift-pagedown", SelectPageDown, ctx),
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("shift-backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("tab", Tab, ctx),
        KeyBinding::new("shift-tab", Outdent, ctx),
        KeyBinding::new("escape", Cancel, ctx),
    ]);
}

impl EditorView {
    pub fn new(document: Entity<Document>, cx: &mut Context<Self>) -> Self {
        let cursor = Cursor::new(&document.read(cx).buffer);
        // Any view's edit repaints this one, and tells the workspace.
        cx.subscribe(&document, |_, _doc, event: &DocumentEvent, cx| {
            match event {
                DocumentEvent::Changed => cx.emit(EditorEvent::Changed),
                DocumentEvent::Saved => cx.emit(EditorEvent::Saved),
                DocumentEvent::StatusChanged => {}
            }
            cx.notify();
        })
        .detach();
        let options = ViewOptions::from(&cx.global::<AppSettings>().settings);
        EditorView {
            document,
            cursor,
            scroll_y: px(0.),
            scroll_x: px(0.),
            last_bounds: Bounds::default(),
            tm: TextMetrics::current(cx),
            options,
            focus_handle: cx.focus_handle(),
            dragging: false,
            marked: None,
            search: None,
            occurrences: None,
            map: DisplayMap::default(),
            goal_column: None,
            expand_history: Vec::new(),
            expand_last: None,
            gutter_hovered: false,
            hovered_row: None,
            status_hovered: None,
            status_popup: None,
            blame: None,
            caret_visible: true,
            blink_epoch: 0,
            blink_task: None,
            sticky_rows: Vec::new(),
            space_px: px(0.),
            read_only: false,
            run_rows: None,
            problems_open: false,
            problem_expanded: None,
            problem_hovered: None,
            hover_position: Point::default(),
            problem_tooltip_shown: false,
            hover_epoch: 0,
            hover_task: None,
            smart: smart::SmartState::default(),
            scroll_rest: Point::default(),
            tui_bar: TuiBar::default(),
            tui_bar_drag: false,
            badge: (0, 0, 0),
            blame_hovered: false,
        }
    }

    // ---- M6: run icons and problems ---------------------------------------------

    /// The rows of `.fleet/run.json`'s configurations with their index, for
    /// the gutter's run icons; empty for any other file.
    fn run_rows(&mut self, cx: &App) -> Vec<(usize, usize)> {
        let doc = self.document.read(cx);
        let is_run_file = doc.path.as_deref().is_some_and(|p| {
            p.file_name().is_some_and(|n| n == "run.json")
                && p.parent()
                    .and_then(|d| d.file_name())
                    .is_some_and(|n| n == ".fleet")
        });
        if !is_run_file {
            return Vec::new();
        }
        let version = doc.buffer.version();
        if let Some((v, rows)) = &self.run_rows {
            if *v == version {
                return rows.clone();
            }
        }
        let rows: Vec<(usize, usize)> = weep_run::config::config_lines(&doc.buffer.text())
            .into_iter()
            .enumerate()
            .filter_map(|(i, l)| l.map(|l| (l, i)))
            .collect();
        self.run_rows = Some((version, rows.clone()));
        rows
    }

    /// The document's syntax errors, while `editor.markupWithoutSmartMode`
    /// shows them.
    pub fn problems(&self, cx: &mut App) -> Vec<Problem> {
        let markup = self.options.markup;
        self.document.update(cx, |d, _| d.problems(markup).to_vec())
    }

    /// Moves the caret to the problem after (or before) it, wrapping, and
    /// opens the problems widget on it (`editor/go-to-next-problem`,
    /// `editor/go-to-previous-problem`; Fleet's tooltip calls the first
    /// "Open Problems").
    fn go_to_problem(&mut self, forward: bool, cx: &mut Context<Self>) {
        let problems = self.problems(cx);
        if problems.is_empty() {
            return;
        }
        let head = self.with_buffer(cx, |c, _| c.selection.head);
        let ix = if forward {
            problems
                .iter()
                .position(|p| p.range.start > head)
                .unwrap_or(0)
        } else {
            problems
                .iter()
                .rposition(|p| p.range.start < head)
                .unwrap_or(problems.len() - 1)
        };
        self.show_problem(ix, &problems, cx);
    }

    fn show_problem(&mut self, ix: usize, problems: &[Problem], cx: &mut Context<Self>) {
        let Some(problem) = problems.get(ix) else {
            return;
        };
        let point = self
            .document
            .read(cx)
            .buffer
            .offset_to_point(problem.range.start);
        self.goto(point.row, point.column, cx);
        self.problems_open = true;
        self.problem_expanded = Some(ix);
        cx.notify();
    }

    fn go_to_next_problem(&mut self, _: &GoToNextProblem, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_problem(true, cx);
    }

    fn go_to_previous_problem(
        &mut self,
        _: &GoToPreviousProblem,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_problem(false, cx);
    }

    /// The status badge's click: the widget opens on the problem at the
    /// caret (the first otherwise), or closes.
    pub fn toggle_problems(&mut self, cx: &mut Context<Self>) {
        if self.problems_open {
            self.problems_open = false;
        } else {
            let problems = self.problems(cx);
            if problems.is_empty() {
                return;
            }
            let head = self.with_buffer(cx, |c, _| c.selection.head);
            self.problem_expanded = problems
                .iter()
                .position(|p| p.range.start <= head && head <= p.range.end.max(p.range.start + 1));
            self.problems_open = true;
        }
        cx.notify();
    }

    /// Closes the widget; whether it was open (Escape's first job).
    pub fn close_problems(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.problems_open {
            return false;
        }
        self.problems_open = false;
        cx.notify();
        true
    }

    /// The problem under the pointer, from the row and a monospace column
    /// (an estimate on a wrapped row).
    fn problem_at(&self, position: Point<Pixels>, cx: &App) -> Option<usize> {
        let bounds = self.last_bounds;
        if !bounds.contains(&position) {
            return None;
        }
        let y = position.y - bounds.origin.y + self.scroll_y;
        let d = (y / self.line_height()).floor().max(0.) as usize;
        let (row, seg) = self.map.at(d);
        if seg > 0 {
            return None;
        }
        let x = position.x - bounds.origin.x - self.text_offset() - px(self.tm.gutter_width)
            + self.scroll_x;
        if x < px(0.) {
            return None;
        }
        let col = (x / self.space_px.max(px(1.))).floor() as usize;
        let doc = self.document.read(cx);
        if row >= doc.buffer.line_count() {
            return None;
        }
        let line_start = doc.buffer.line_to_char(row);
        let line_len = doc.buffer.line(row).chars().count();
        let offset = line_start + col.min(line_len);
        doc.cached_problems()
            .iter()
            .position(|p| p.range.start <= offset && offset < p.range.end.max(p.range.start + 1))
    }

    /// Tracks the problem under the pointer; its tooltip shows after
    /// `editor.hoverTooltipDelayMs`.
    fn hover_problem(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let hovered = if self.options.problem_tooltip && self.options.markup {
            self.problem_at(position, cx)
        } else {
            None
        };
        if hovered == self.problem_hovered {
            return;
        }
        self.problem_hovered = hovered;
        self.hover_position = position;
        self.problem_tooltip_shown = false;
        self.hover_epoch += 1;
        self.hover_task = None;
        if hovered.is_none() {
            cx.notify();
            return;
        }
        let epoch = self.hover_epoch;
        let delay = cx
            .global::<AppSettings>()
            .settings
            .editor_hover_tooltip_delay_ms();
        self.hover_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.hover_epoch == epoch && this.problem_hovered.is_some() {
                    this.problem_tooltip_shown = true;
                    cx.notify();
                }
            });
        }));
    }

    /// The file status badge, the problems widget over the editor's top
    /// right, and the hovered problem's tooltip (batch 11, twelfth pass).
    fn render_problems(&mut self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let mut out = Vec::new();
        if !self.options.file_status {
            return out;
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let problems = self.problems(cx);
        let count = problems.len();
        let next_chord = crate::keymap::hint_for(cx, "editor/go-to-next-problem");
        let prev_chord = crate::keymap::hint_for(cx, "editor/go-to-previous-problem");
        let ui = |d: gpui::Div| {
            d.font_family(theme.ui_font.clone())
                .text_size(px(m.ui_font_size))
                .text_color(theme.popup_text)
        };
        // The badge or the widget's header sits on an island-coloured margin
        // that cuts the current line's wash around it (batch 11).
        let cutout = |d: gpui::Div| {
            d.absolute()
                .top(px(0.))
                .right(px(m.file_status_right - m.file_status_margin))
                .pt(px(m.file_status_top))
                .px(px(m.file_status_margin))
                .pb(px(m.file_status_margin))
                .bg(theme.island_background)
                .flex()
                .items_center()
        };
        let box_20 = |d: gpui::Stateful<gpui::Div>| {
            d.h(px(m.file_status_size))
                .min_w(px(m.file_status_size))
                .rounded(px(m.file_status_radius))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
        };
        // The tag's colours follow the worst problem: errors red, warnings
        // yellow, weak warnings and infos grey (M7; the warning and weak
        // states are estimates, batch 12 has not caught them).
        let errors = problems
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .count();
        let warnings = problems
            .iter()
            .filter(|p| p.severity == Severity::Warning)
            .count();
        let (tag_bg, tag_bg_hovered, tag_text) = if errors > 0 {
            (
                theme.problems_error_tag_bg,
                theme.problems_error_tag_bg_hovered,
                theme.problems_error_tag_text,
            )
        } else if warnings > 0 {
            (
                theme.problems_warning_tag_bg,
                theme.problems_warning_tag_bg_hovered,
                theme.problems_warning_tag_text,
            )
        } else {
            (
                theme.problems_weak_tag_bg,
                theme.problems_weak_tag_bg,
                theme.problems_weak_tag_text,
            )
        };
        let badge_count = if errors > 0 {
            errors
        } else if warnings > 0 {
            warnings
        } else {
            count
        };
        if !self.problems_open || count == 0 {
            let title = if count == 0 {
                "No problems found"
            } else {
                "Open Problems"
            };
            let badge = cutout(ui(div()))
                .child(
                    box_20(div().id("file-status"))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_problems(cx)))
                        .map(|d| {
                            if count == 0 {
                                d.bg(theme.problems_success_bg)
                                    .tooltip(Tooltip::text(title))
                                    .child(crate::assets::icon(
                                        "weep/check",
                                        m.icon_size,
                                        theme.problems_success_fg,
                                    ))
                            } else {
                                d.bg(tag_bg)
                                    .hover(move |s| s.bg(tag_bg_hovered))
                                    .px(px(m.file_status_tag_pad_x))
                                    .text_color(tag_text)
                                    .map(|d| match next_chord.clone() {
                                        Some(chord) => {
                                            d.tooltip(Tooltip::with_shortcut(title, chord))
                                        }
                                        None => d.tooltip(Tooltip::text(title)),
                                    })
                                    .child(badge_count.to_string())
                            }
                        }),
                )
                .into_any_element();
            out.push(badge);
        } else {
            let head = self.with_buffer(cx, |c, _| c.selection.head);
            let at_caret = problems
                .iter()
                .position(|p| p.range.start <= head && head <= p.range.end.max(p.range.start + 1));
            let expanded = at_caret.or(self.problem_expanded.filter(|ix| *ix < count));
            // "2 errors", "1 warning", "2 errors, 1 warning" (the mixed form
            // is an estimate); infos and weak warnings count as problems.
            let plural = |n: usize, word: &str| {
                if n == 1 {
                    format!("1 {word}")
                } else {
                    format!("{n} {word}s")
                }
            };
            let others = count - errors - warnings;
            let mut parts = Vec::new();
            if errors > 0 {
                parts.push(plural(errors, "error"));
            }
            if warnings > 0 {
                parts.push(plural(warnings, "warning"));
            }
            if others > 0 && parts.is_empty() {
                parts.push(plural(others, "problem"));
            }
            let label: SharedString = parts.join(", ").into();
            let header = cutout(ui(div()))
                .child(
                    box_20(div().id("problems-tag"))
                        .mr(px(m.problems_header_gap))
                        .px(px(m.file_status_tag_pad_x))
                        .bg(tag_bg)
                        .hover(move |s| s.bg(tag_bg_hovered))
                        .text_color(tag_text)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_problems(cx)))
                        .child(label),
                )
                .child(
                    box_20(div().id("problems-filter"))
                        .mr(px(m.problems_button_gap))
                        .hover(|s| s.bg(theme.ghost_hovered))
                        .tooltip(Tooltip::text("Filter"))
                        .child(crate::assets::icon(
                            "weep/sliders",
                            m.icon_size,
                            theme.icon_secondary,
                        )),
                )
                .child(
                    box_20(div().id("problems-close"))
                        .hover(|s| s.bg(theme.ghost_hovered))
                        .tooltip(Tooltip::with_shortcut("Close Problems", "Escape"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_problems(cx);
                        }))
                        .child(crate::assets::icon(
                            "expui/general/close",
                            m.icon_size,
                            theme.icon_secondary,
                        )),
                )
                .into_any_element();
            out.push(header);
            let card_top = m.file_status_top + m.file_status_size + m.problems_card_gap;
            let doc = self.document.read(cx);
            let mut card_height = 2. * m.problems_card_pad + 2.;
            let rows: Vec<gpui::AnyElement> = problems
                .iter()
                .enumerate()
                .map(|(ix, p)| {
                    let line: SharedString =
                        format!(":{}", doc.buffer.offset_to_point(p.range.start).row + 1).into();
                    let message: SharedString = p.message.clone().into();
                    let is_expanded = expanded == Some(ix);
                    let severity = p.severity;
                    let title_row = |text_color: Hsla| {
                        div()
                            .h(px(m.problems_row_height))
                            .flex()
                            .items_center()
                            .pl(px(m.problems_row_pad_left))
                            .pr(px(m.problems_row_pad_right))
                            .child(
                                div()
                                    .size(px(m.icon_size))
                                    .flex_shrink_0()
                                    .relative()
                                    .mr(px(m.problems_row_text_x
                                        - m.problems_row_pad_left
                                        - m.icon_size))
                                    .map(|d| match severity {
                                        // The error icon as batch 11 caught it; the
                                        // rest expui's status icons (estimates).
                                        Severity::Error => d
                                            .child(
                                                crate::assets::icon(
                                                    "weep/circle",
                                                    m.icon_size,
                                                    theme.icon_red,
                                                )
                                                .absolute()
                                                .left_0()
                                                .top_0(),
                                            )
                                            .child(
                                                crate::assets::icon(
                                                    "weep/exclamation",
                                                    m.icon_size,
                                                    theme.problems_error_tag_text,
                                                )
                                                .absolute()
                                                .left_0()
                                                .top_0(),
                                            ),
                                        Severity::Warning => d.child(
                                            crate::assets::icon(
                                                "expui/status/warning",
                                                m.icon_size,
                                                theme.icon_yellow,
                                            )
                                            .absolute()
                                            .left_0()
                                            .top_0(),
                                        ),
                                        Severity::Weak => d.child(
                                            crate::assets::icon(
                                                "expui/status/warningOutline",
                                                m.icon_size,
                                                theme.icon_secondary,
                                            )
                                            .absolute()
                                            .left_0()
                                            .top_0(),
                                        ),
                                        Severity::Info => d.child(
                                            crate::assets::icon(
                                                "expui/status/info",
                                                m.icon_size,
                                                theme.icon_blue,
                                            )
                                            .absolute()
                                            .left_0()
                                            .top_0(),
                                        ),
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(text_color)
                                    .child(message.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(theme.problems_card_text_secondary)
                                    .child(line.clone()),
                            )
                    };
                    if is_expanded {
                        // The server's quick fixes for this problem (M7), one
                        // row each under the message; "No fixes available"
                        // while there are none (batch 11's wording).
                        let fixes: Vec<String> = match &self.smart.fixes {
                            Some((for_ix, titles)) if *for_ix == ix => titles.clone(),
                            _ => Vec::new(),
                        };
                        let extra = fixes.len().saturating_sub(1) as f32 * m.smart_popup_line;
                        card_height += m.problems_expanded_height + 2. * m.problems_expanded_border
                            - 2. * m.problems_card_pad
                            + extra;
                        let (card_bg, card_border) = match severity {
                            Severity::Error => (
                                theme.problems_card_error_bg,
                                theme.problems_card_error_border,
                            ),
                            Severity::Warning => (
                                theme.problems_card_warning_bg,
                                theme.problems_card_warning_border,
                            ),
                            Severity::Weak | Severity::Info => {
                                (theme.problems_card_weak_bg, theme.problems_card_weak_border)
                            }
                        };
                        div()
                            .id(ElementId::NamedInteger("problem-row".into(), ix as u64))
                            .h(px(m.problems_expanded_height
                                + 2. * m.problems_expanded_border
                                + extra))
                            // Its border sits on the card's edge, over the card's padding.
                            .my(px(-m.problems_card_pad))
                            .border(px(m.problems_expanded_border))
                            .border_color(card_border)
                            .bg(card_bg)
                            .rounded(px(m.popup_radius))
                            .flex()
                            .flex_col()
                            .child(
                                title_row(theme.problems_card_text)
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                            .map(|d| {
                                if fixes.is_empty() {
                                    d.child(
                                        div()
                                            .pl(px(m.problems_row_text_x))
                                            .text_color(theme.problems_card_text_secondary)
                                            .child("No fixes available"),
                                    )
                                } else {
                                    d.children(fixes.into_iter().enumerate().map(|(fix, title)| {
                                        div()
                                            .id(ElementId::NamedInteger(
                                                "problem-fix".into(),
                                                (ix * 1000 + fix) as u64,
                                            ))
                                            .pl(px(m.problems_row_text_x))
                                            .pr(px(m.problems_row_pad_right))
                                            .h(px(m.smart_popup_line))
                                            .flex()
                                            .items_center()
                                            .cursor_pointer()
                                            .text_color(theme.link)
                                            .hover(|s| s.bg(theme.list_bg_hovered))
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.emit(EditorEvent::ApplyProblemFix {
                                                    problem: ix,
                                                    fix,
                                                });
                                            }))
                                            .child(SharedString::from(title))
                                    }))
                                }
                            })
                            .into_any_element()
                    } else {
                        card_height += m.problems_row_height;
                        title_row(theme.problems_card_text)
                            .id(ElementId::NamedInteger("problem-row".into(), ix as u64))
                            .rounded(px(m.popup_row_radius))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.list_bg_hovered))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let problems = this.problems(cx);
                                this.show_problem(ix, &problems, cx);
                            }))
                            .into_any_element()
                    }
                })
                .collect();
            // The card is itself the absolute element (a wrapper would sit
            // below the editor element in the flow and carry it along).
            let card = div()
                .id("problems-card")
                .font_family(theme.ui_font.clone())
                .text_size(px(m.ui_font_size))
                .text_color(theme.popup_text)
                .occlude()
                .absolute()
                .top(px(card_top))
                .right(px(m.problems_card_right))
                .w(px(m.problems_card_width))
                .child(
                    div()
                        .w_full()
                        .p(px(m.problems_card_pad))
                        .rounded(px(m.popup_radius))
                        .border_1()
                        .border_color(theme.popup_border)
                        .bg(theme.popup_bg)
                        .shadow(vec![BoxShadow {
                            color: theme.shadow_medium,
                            offset: point(px(0.), px(m.popup_shadow_offset_y)),
                            blur_radius: px(m.popup_shadow_blur),
                            spread_radius: px(0.),
                        }])
                        .flex()
                        .flex_col()
                        .children(rows),
                )
                .into_any_element();
            out.push(card);
            let hint = |label: &'static str, chord: Option<String>| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(m.problems_hint_chord_gap))
                    .child(div().text_color(theme.text_secondary).child(label))
                    .children(chord.map(|c| div().text_color(theme.text_primary).child(c)))
            };
            let hints = div()
                .absolute()
                .top(px(card_top + card_height + m.problems_hint_top))
                .right(px(m.problems_card_right + m.problems_hint_right))
                .flex()
                .items_center()
                .gap(px(m.problems_hint_gap))
                .font_family(theme.ui_font.clone())
                .text_size(px(m.problems_hint_font))
                .whitespace_nowrap()
                .child(hint("Next Problem", next_chord))
                .child(hint("Previous Problem", prev_chord))
                .into_any_element();
            out.push(hints);
        }
        if self.problem_tooltip_shown {
            if let Some(p) = self.problem_hovered.and_then(|ix| problems.get(ix)) {
                let at = self.hover_position - self.last_bounds.origin;
                let message: SharedString = p.message.clone().into();
                out.push(
                    ui(div())
                        .absolute()
                        .left(at.x)
                        .top(at.y + px(m.tooltip_offset_y))
                        .px(px(m.tooltip_pad_x))
                        .py(px(m.tooltip_pad_y))
                        .rounded(px(m.tooltip_radius))
                        .bg(theme.tooltip_bg)
                        .border_1()
                        .border_color(theme.tooltip_border)
                        .text_color(theme.tooltip_text)
                        .line_height(px(m.tooltip_line))
                        .whitespace_nowrap()
                        .child(message)
                        .into_any_element(),
                );
            }
        }
        out
    }

    /// The whole-word occurrences of the identifier under the caret (its own
    /// included), cached per buffer version and word.
    fn occurrences(&mut self, cx: &App) -> Vec<Range<usize>> {
        let doc = self.document.read(cx);
        self.cursor.sync(&doc.buffer);
        if !self.cursor.selection.is_empty() || self.cursor.has_multiple() {
            return Vec::new();
        }
        let Some(range) = analysis::identifier_at(&doc.buffer, self.cursor.selection.head) else {
            return Vec::new();
        };
        let word = doc.buffer.slice(range);
        let version = doc.buffer.version();
        match &self.occurrences {
            Some((v, w, found)) if *v == version && *w == word => found.clone(),
            _ => {
                let found = analysis::whole_word_occurrences(&doc.buffer, &word);
                self.occurrences = Some((version, word, found.clone()));
                found
            }
        }
    }

    pub fn path(&self, cx: &App) -> Option<PathBuf> {
        self.document.read(cx).path.clone()
    }

    pub fn title(&self, cx: &App) -> String {
        self.document.read(cx).title()
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        self.document.read(cx).is_dirty()
    }

    pub fn language(&self, cx: &App) -> Option<Language> {
        self.document.read(cx).language
    }

    pub fn uses_crlf(&self, cx: &App) -> bool {
        self.document.read(cx).buffer.uses_crlf()
    }

    /// The caret's row and column, after catching up with other views' edits.
    pub fn cursor_position(&mut self, cx: &App) -> TextPoint {
        let doc = self.document.read(cx);
        self.cursor.sync(&doc.buffer);
        self.cursor.cursor_point(&doc.buffer)
    }

    pub fn caret(&mut self, cx: &App) -> usize {
        self.with_buffer(cx, |c, _| c.selection.head)
    }

    pub fn selection_range(&mut self, cx: &App) -> Range<usize> {
        self.with_buffer(cx, |c, _| c.selection.range())
    }

    pub fn selected_text(&mut self, cx: &App) -> String {
        self.with_buffer(cx, |c, b| c.selected_text(b))
    }

    pub fn version(&self, cx: &App) -> u64 {
        self.document.read(cx).buffer.version()
    }

    /// `range`, taken at buffer `version`, moved past the edits since.
    pub fn adjust_range(&self, range: Range<usize>, version: u64, cx: &App) -> Range<usize> {
        let buffer = &self.document.read(cx).buffer;
        let mut range = range;
        if let Some(edits) = buffer.edits_since(version) {
            for edit in edits {
                range = edit.adjust(range.start)..edit.adjust(range.end);
            }
        }
        let len = buffer.len_chars();
        range.start.min(len)..range.end.min(len)
    }

    /// `save-file`: through the document, which trims and writes.
    pub fn save(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let caret_rows: Vec<usize> = self.with_buffer(cx, |c, b| {
            c.selections()
                .iter()
                .map(|s| b.offset_to_point(s.head).row)
                .collect()
        });
        let settings = cx.global::<AppSettings>().settings.clone();
        self.document
            .update(cx, |doc, cx| doc.save(&caret_rows, &settings, cx))
    }

    pub fn set_crlf(&mut self, crlf: bool, cx: &mut Context<Self>) {
        self.document.update(cx, |doc, cx| doc.set_crlf(crlf, cx));
    }

    /// Runs a command on this view's cursor and the shared buffer; the
    /// document tells every view, this one included, to repaint.
    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Cursor, &mut Buffer)) {
        if self.read_only {
            return;
        }
        let cursor = &mut self.cursor;
        self.document.update(cx, |doc, cx| {
            cursor.sync(&doc.buffer);
            doc.edit(cx, |buffer| f(cursor, buffer));
        });
    }

    /// Moves this view's cursor; nothing in the buffer changes.
    fn go(&mut self, cx: &App, f: impl FnOnce(&mut Cursor, &Buffer)) {
        let doc = self.document.read(cx);
        self.cursor.sync(&doc.buffer);
        f(&mut self.cursor, &doc.buffer);
    }

    fn with_buffer<R>(&mut self, cx: &App, f: impl FnOnce(&Cursor, &Buffer) -> R) -> R {
        let doc = self.document.read(cx);
        self.cursor.sync(&doc.buffer);
        f(&self.cursor, &doc.buffer)
    }

    fn line_height(&self) -> Pixels {
        px(self.tm.line_height)
    }

    fn rows_per_page(&self) -> usize {
        let h = self.last_bounds.size.height;
        ((h / self.line_height()).floor() as usize)
            .saturating_sub(1)
            .max(1)
    }

    // ---- folds and the map ----------------------------------------------------

    /// Brings the map up to date with the buffer and the wrap width.
    fn sync_map(&mut self, cx: &App) {
        let doc = self.document.read(cx);
        let buffer = &doc.buffer;
        let version = buffer.version();
        if self.map.version != Some(version) {
            // Folds move with the text: keep the ones still spanning rows.
            if let Some(old) = self.map.version {
                if let Some(edits) = buffer.edits_since(old) {
                    for fold in &mut self.map.folds {
                        for edit in &edits {
                            fold.inner = edit.adjust(fold.inner.start)..edit.adjust(fold.inner.end);
                        }
                    }
                } else {
                    self.map.folds.clear();
                }
                let len = buffer.len_chars();
                for fold in &mut self.map.folds {
                    fold.inner.start = fold.inner.start.min(len);
                    fold.inner.end = fold.inner.end.min(len);
                    fold.start_row = buffer.offset_to_point(fold.inner.start).row;
                    fold.end_row = buffer.offset_to_point(fold.inner.end).row;
                }
                self.map
                    .folds
                    .retain(|f| f.end_row > f.start_row && !f.inner.is_empty());
                self.map.folds.sort_by_key(|f| f.start_row);
                self.map.folds.dedup_by_key(|f| f.start_row);
            }
        }
        let wrap_cols = self.wrap_columns();
        if self.map.version != Some(version) || self.map.wrap_cols != wrap_cols {
            self.map.rebuild(buffer, wrap_cols);
        }
    }

    /// `editor.softWrapEnabled`: the columns a row wraps at, from the view's
    /// width (or the first `editor.guides` entry with `guideToSoftWrapBy`).
    fn wrap_columns(&self) -> Option<usize> {
        if !self.options.soft_wrap {
            return None;
        }
        if self.options.wrap_by_guide {
            if let Some(g) = self.options.guides.first() {
                return Some((*g).max(1));
            }
        }
        let space = self.space_width();
        if space <= px(0.) {
            return None;
        }
        let width = self.last_bounds.size.width
            - px(self.tm.gutter_width)
            - px(self.tm.scrollbar_thumb)
            - self.text_offset();
        Some(((width / space).floor() as usize).max(8))
    }

    /// One space's advance: measured in prepaint; before the first frame,
    /// JetBrains Mono's 0.6 em.
    fn space_width(&self) -> Pixels {
        if self.space_px > px(0.) {
            self.space_px
        } else {
            px((self.tm.font_size * 0.6).round())
        }
    }

    /// `editor.centerEditorHorizontally` / `distractionFreeMode`: the editor
    /// area (gutter and text) shifts right to sit centred when the view is
    /// wider; the blame column, when shown, pushes it right by its width.
    fn text_offset(&self) -> Pixels {
        let blame = if self.blame.is_some() {
            px(self.tm.blame_width)
        } else {
            px(0.)
        };
        if !(self.options.centered || self.options.distraction_free) {
            return blame;
        }
        let area = px(self.tm.gutter_width) + self.space_width() * self.tm.centered_columns;
        let centered = ((self.last_bounds.size.width - area) / 2.).max(px(0.));
        match self.tm.cell {
            // On the tui grid the shift is whole columns.
            Some(cell) => px((f32::from(centered) / cell.w).floor() * cell.w) + blame,
            None => centered + blame,
        }
    }

    /// The tui skin: the hunk a row's line-status cell shows. A deletion
    /// (no rows, sitting before its row) is marked on the bottom edge of the
    /// row above it, or on the first row's top when nothing is above.
    fn tui_hunk_at(&self, row: usize, cx: &App) -> Option<usize> {
        let hunks = &self.document.read(cx).hunks;
        hunks
            .iter()
            .position(|h| h.rows > 0 && hunk_covers(h, row))
            .or_else(|| hunks.iter().position(|h| tui_deletion_on(h, row)))
    }

    /// The tui skin: whether `x_in` (from the text offset) is in the
    /// line-status cell.
    fn in_status_cell(&self, x_in: Pixels) -> bool {
        self.tm.cell.is_some_and(|cell| {
            x_in >= px(self.tm.line_status_x) && x_in < px(self.tm.line_status_x + cell.w)
        })
    }

    /// `git/toggle-blame`: shows or hides the blame column.
    pub fn set_blame(&mut self, blame: Option<Vec<weep_git::BlameLine>>, cx: &mut Context<Self>) {
        self.blame = blame;
        cx.notify();
    }

    pub fn blame_shown(&self) -> bool {
        self.blame.is_some()
    }

    fn rebuild_map(&mut self, cx: &App) {
        self.map.version = None;
        self.sync_map(cx);
    }

    /// Folds the block whose head is `row`.
    fn fold_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let fold = self.document.update(cx, |doc, _| {
            doc.folds()
                .iter()
                .filter(|f| f.start_row == row)
                .max_by_key(|f| f.end_row)
                .cloned()
        });
        let Some(fold) = fold else {
            return;
        };
        let folded = Folded::from_fold(&fold, &self.document.read(cx).buffer);
        if self.map.folds.iter().any(|f| f.start_row == row) {
            return;
        }
        self.map.folds.push(folded);
        self.map.folds.sort_by_key(|f| f.start_row);
        self.rebuild_map(cx);
        // A caret inside the block moves to its head.
        let buffer_rows: Vec<usize> = self.with_buffer(cx, |c, b| {
            c.selections()
                .iter()
                .map(|s| b.offset_to_point(s.head).row)
                .collect()
        });
        if buffer_rows
            .iter()
            .any(|r| *r > fold.start_row && *r <= fold.end_row)
        {
            let at = self
                .map
                .folds
                .iter()
                .find(|f| f.start_row == row)
                .map(|f| f.inner.start);
            if let Some(at) = at {
                self.go(cx, |c, b| c.set_cursor(b, at));
            }
        }
        cx.notify();
    }

    fn unfold_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let before = self.map.folds.len();
        self.map.folds.retain(|f| f.start_row != row);
        if self.map.folds.len() != before {
            self.rebuild_map(cx);
            cx.notify();
        }
    }

    /// Unfolds every block hiding `offset` (so a caret put there shows).
    fn reveal(&mut self, offset: usize, cx: &App) {
        let row = self.document.read(cx).buffer.offset_to_point(offset).row;
        let before = self.map.folds.len();
        self.map
            .folds
            .retain(|f| !(row > f.start_row && row <= f.end_row));
        if self.map.folds.len() != before {
            self.rebuild_map(cx);
        }
    }

    fn reveal_carets(&mut self, cx: &App) {
        let heads: Vec<usize> =
            self.with_buffer(cx, |c, _| c.selections().iter().map(|s| s.head).collect());
        for head in heads {
            self.reveal(head, cx);
        }
    }

    // ---- scrolling ------------------------------------------------------------

    /// Scrolls just enough for the caret's display row to be on screen.
    fn scroll_to_cursor(&mut self, cx: &App) {
        self.sync_map(cx);
        let lh = self.line_height();
        let point = self.cursor_position(cx);
        let row = self.map.display_row(point.row, point.column) as f32;
        let top = lh * row;
        let bottom = top + lh;
        let height = self.last_bounds.size.height;
        if height <= px(0.) {
            return;
        }
        // Sticky lines cover the top rows; keep the caret below them.
        let sticky = lh * self.sticky_rows.len() as f32;
        if top < self.scroll_y + sticky {
            self.scroll_y = (top - sticky).max(px(0.));
        } else if bottom > self.scroll_y + height {
            self.scroll_y = bottom - height;
        }
    }

    /// Scrolls the caret's row to the middle of the view (a jump: goto, find).
    fn center_cursor(&mut self, cx: &App) {
        self.sync_map(cx);
        let lh = self.line_height();
        let point = self.cursor_position(cx);
        let row = self.map.display_row(point.row, point.column) as f32;
        let height = self.last_bounds.size.height;
        if height <= px(0.) {
            return;
        }
        let top = lh * row;
        if top < self.scroll_y || top + lh > self.scroll_y + height {
            let max = self.max_scroll_y(cx);
            self.scroll_y = (top - height / 2.).clamp(px(0.), max);
        }
    }

    /// The scrollable extent: the display rows plus Fleet's overscroll past the end.
    fn content_height(&self) -> Pixels {
        self.line_height() * (self.map.total as f32 + EDITOR_OVERSCROLL_LINES)
    }

    fn max_scroll_y(&mut self, cx: &App) -> Pixels {
        self.sync_map(cx);
        (self.content_height() - self.last_bounds.size.height).max(px(0.))
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.expand_last = None;
        self.reveal_carets(cx);
        self.scroll_to_cursor(cx);
        self.restart_blink(cx);
        self.smart_after_edit(cx);
        cx.notify();
    }

    fn after_move(&mut self, cx: &mut Context<Self>) {
        self.reveal_carets(cx);
        self.scroll_to_cursor(cx);
        self.restart_blink(cx);
        self.smart_after_move(cx);
        cx.notify();
    }

    /// `editor.caretBlinkingEnabled`: the caret shows for `CARET_BLINK_MS`,
    /// hides as long, while the view has the focus; any key or click shows it.
    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        self.caret_visible = true;
        self.blink_epoch += 1;
        if !self.options.caret_blink {
            self.blink_task = None;
            return;
        }
        let epoch = self.blink_epoch;
        self.blink_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(CARET_BLINK_MS))
                .await;
            let go_on = this.update(cx, |this, cx| {
                if this.blink_epoch != epoch {
                    return false;
                }
                this.caret_visible = !this.caret_visible;
                cx.notify();
                true
            });
            if !matches!(go_on, Ok(true)) {
                break;
            }
        }));
    }

    // ---- vertical movement through the map -------------------------------------

    /// Up or down by display rows (a wrapped line's segments, past folded
    /// blocks), keeping the caret's display column; `rows` display rows.
    fn move_vertical(&mut self, delta: isize, select: bool, cx: &App) {
        self.sync_map(cx);
        if self.map.is_identity() {
            self.go(cx, |c, b| match (delta.signum(), delta.unsigned_abs()) {
                (-1, 1) => c.move_up(b, select),
                (1, 1) => c.move_down(b, select),
                (-1, n) => c.move_page(b, n, true, select),
                (_, n) => c.move_page(b, n, false, select),
            });
            self.goal_column = None;
            return;
        }
        let doc = self.document.read(cx);
        let buffer = &doc.buffer;
        self.cursor.sync(buffer);
        let selections = self.cursor.selections();
        let primary = self.cursor.selection;
        let last_display = self.map.total.saturating_sub(1) as isize;
        let mut goal = self.goal_column;
        let moved: Vec<Selection> = selections
            .iter()
            .map(|s| {
                let point = buffer.offset_to_point(s.head);
                let seg = self.map.segment_of(point.row, point.column);
                let seg_start = self.map.segment_range(buffer, point.row, seg).start;
                let column_in_seg = goal.unwrap_or(point.column - seg_start);
                if goal.is_none() && *s == primary {
                    goal = Some(column_in_seg);
                }
                let d = self.map.display_row(point.row, point.column) as isize;
                let target = (d + delta).clamp(0, last_display);
                let head = if target == d {
                    if delta < 0 {
                        0
                    } else {
                        buffer.len_chars()
                    }
                } else {
                    let (row, seg) = self.map.at(target as usize);
                    let range = self.map.segment_range(buffer, row, seg);
                    let col = (range.start + column_in_seg).min(range.end);
                    buffer.point_to_offset(TextPoint::new(row, col))
                };
                Selection {
                    anchor: if select { s.anchor } else { head },
                    head,
                }
            })
            .collect();
        let primary_ix = selections.iter().position(|s| *s == primary).unwrap_or(0);
        self.cursor.set_selections(buffer, moved, primary_ix);
        self.goal_column = goal;
    }

    // ---- actions --------------------------------------------------------

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(-1, false, cx);
        self.after_move(cx);
    }
    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(1, false, cx);
        self.after_move(cx);
    }
    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_left(b, false));
        self.goal_column = None;
        self.after_move(cx);
    }
    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_right(b, false));
        self.goal_column = None;
        self.after_move(cx);
    }
    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(-1, true, cx);
        self.after_move(cx);
    }
    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(1, true, cx);
        self.after_move(cx);
    }
    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_left(b, true));
        self.goal_column = None;
        self.after_move(cx);
    }
    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_right(b, true));
        self.goal_column = None;
        self.after_move(cx);
    }
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_line_start(b, false));
        self.goal_column = None;
        self.after_move(cx);
    }
    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_line_end(b, false));
        self.goal_column = None;
        self.after_move(cx);
    }
    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_line_start(b, true));
        self.goal_column = None;
        self.after_move(cx);
    }
    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_line_end(b, true));
        self.goal_column = None;
        self.after_move(cx);
    }
    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows_per_page();
        self.move_vertical(-(rows as isize), false, cx);
        self.scroll_y = (self.scroll_y - self.line_height() * rows as f32).max(px(0.));
        self.after_move(cx);
    }
    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows_per_page();
        self.move_vertical(rows as isize, false, cx);
        let max = self.max_scroll_y(cx);
        self.scroll_y = (self.scroll_y + self.line_height() * rows as f32).min(max);
        self.after_move(cx);
    }
    fn select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows_per_page();
        self.move_vertical(-(rows as isize), true, cx);
        self.after_move(cx);
    }
    fn select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows_per_page();
        self.move_vertical(rows as isize, true, cx);
        self.after_move(cx);
    }
    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_doc_start(b, false));
        self.after_move(cx);
    }
    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_doc_end(b, false));
        self.after_move(cx);
    }
    fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_doc_start(b, true));
        self.after_move(cx);
    }
    fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_doc_end(b, true));
        self.after_move(cx);
    }
    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_word_left(b, false));
        self.after_move(cx);
    }
    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_word_right(b, false));
        self.after_move(cx);
    }
    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_word_left(b, true));
        self.after_move(cx);
    }
    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.move_word_right(b, true));
        self.after_move(cx);
    }
    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let pairs = self.language(cx).is_some_and(Language::auto_pairs);
        self.edit(cx, |c, b| c.backspace_with(b, pairs));
        self.after_edit(cx);
    }
    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.delete(b));
        self.after_edit(cx);
    }
    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |c, b| c.delete_word_backward(b));
        self.after_edit(cx);
    }
    fn delete_word_forward(
        &mut self,
        _: &DeleteWordForward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |c, b| c.delete_word_forward(b));
        self.after_edit(cx);
    }
    fn delete_line(&mut self, _: &DeleteLine, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.delete_line(b));
        self.after_edit(cx);
    }
    fn duplicate_line(&mut self, _: &DuplicateLine, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.duplicate_line(b));
        self.after_edit(cx);
    }
    fn move_line_up(&mut self, _: &MoveLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.move_line(b, true));
        self.after_edit(cx);
    }
    fn move_line_down(&mut self, _: &MoveLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.move_line(b, false));
        self.after_edit(cx);
    }
    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        let indenter = self.options.indenter;
        self.edit(cx, |c, b| c.newline_with(b, indenter));
        self.after_edit(cx);
    }
    fn newline_below(&mut self, _: &NewlineBelow, _: &mut Window, cx: &mut Context<Self>) {
        let indenter = self.options.indenter;
        self.edit(cx, |c, b| {
            c.move_line_end(b, false);
            c.newline_with(b, indenter);
        });
        self.after_edit(cx);
    }
    fn newline_above(&mut self, _: &NewlineAbove, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| {
            if c.cursor_point(b).row == 0 {
                c.set_cursor(b, 0);
                c.newline(b);
                c.move_up(b, false);
            } else {
                c.move_up(b, false);
                c.move_line_end(b, false);
                c.newline(b);
            }
        });
        self.after_edit(cx);
    }
    fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.tab(b));
        self.after_edit(cx);
    }
    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.outdent(b));
        self.after_edit(cx);
    }
    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.undo(b));
        self.after_edit(cx);
    }
    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.redo(b));
        self.after_edit(cx);
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_or_line(cx);
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        cx.emit(EditorEvent::Copied(text));
    }
    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_or_line(cx);
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        cx.emit(EditorEvent::Copied(text));
        self.edit(cx, |c, b| {
            if c.selection.is_empty() && !c.has_multiple() {
                c.delete_line(b);
            } else {
                c.delete(b);
            }
        });
        self.after_edit(cx);
    }
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste_text(&text, cx);
        }
    }
    /// Pastes `text` (`paste`, `editor/paste-from-history`): with as many
    /// carets as lines, one line each.
    pub fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.replace("\r\n", "\n");
        self.edit(cx, |c, b| {
            c.commit();
            c.paste(b, &text);
            c.commit();
        });
        self.after_edit(cx);
    }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.select_all(b));
        cx.notify();
    }
    fn comment_line(&mut self, _: &CommentLine, _: &mut Window, cx: &mut Context<Self>) {
        let Some(prefix) = self.language(cx).and_then(Language::line_comment) else {
            return;
        };
        self.edit(cx, |c, b| c.toggle_line_comment(b, prefix));
        self.after_edit(cx);
    }
    fn comment_block(&mut self, _: &CommentBlock, _: &mut Window, cx: &mut Context<Self>) {
        let Some((open, close)) = self.language(cx).and_then(Language::block_comment) else {
            return;
        };
        self.edit(cx, |c, b| c.toggle_block_comment(b, open, close));
        self.after_edit(cx);
    }
    fn join_lines(&mut self, _: &JoinLines, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| {
            let row = c.cursor_point(b).row;
            if row + 1 >= b.line_count() {
                return;
            }
            let end = b.line_to_char(row) + b.line_len(row);
            let next = b.line(row + 1);
            let lead = next.chars().take_while(|ch| ch.is_whitespace()).count();
            let glue = if next.trim().is_empty() { "" } else { " " };
            c.replace_range(b, end..end + 1 + lead, glue);
        });
        self.after_edit(cx);
    }
    fn toggle_case(&mut self, _: &ToggleCase, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.toggle_case(b));
        self.after_edit(cx);
    }
    /// Escape: back to one empty caret.
    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.close_problems(cx) {
            return;
        }
        self.go(cx, |c, b| {
            let head = c.selection.head;
            c.clear_extra();
            c.set_cursor(b, head);
        });
        self.expand_last = None;
        cx.notify();
    }

    // ---- multiple carets ------------------------------------------------------

    fn add_caret_above(&mut self, _: &AddCaretAbove, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.add_caret_above(b));
        self.after_move(cx);
    }
    fn add_caret_below(&mut self, _: &AddCaretBelow, _: &mut Window, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.add_caret_below(b));
        self.after_move(cx);
    }
    fn add_carets_to_line_ends(
        &mut self,
        _: &AddCaretsToLineEnds,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go(cx, |c, b| c.add_carets_to_ends_of_selected_lines(b));
        self.after_move(cx);
    }
    fn select_next_occurrence(
        &mut self,
        _: &SelectNextOccurrence,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go(cx, |c, b| c.select_next_occurrence(b));
        self.after_move(cx);
    }
    fn select_all_occurrences(
        &mut self,
        _: &SelectAllOccurrences,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go(cx, |c, b| c.select_all_occurrences(b));
        self.after_move(cx);
    }
    fn unselect_occurrence(
        &mut self,
        _: &UnselectOccurrence,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go(cx, |c, b| c.unselect_occurrence(b));
        self.after_move(cx);
    }

    // ---- structure: expand, shrink, brackets, folds -----------------------------

    /// `expand-selection`: the word (its camel hump first with
    /// `editor.honorCamelHumps`), then the enclosing syntax nodes outwards.
    fn expand_selection(&mut self, _: &ExpandSelection, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.with_buffer(cx, |c, _| c.selection);
        if self.expand_last != Some(before) {
            self.expand_history.clear();
        }
        let camel = self.options.camel_humps;
        let mut changed = false;
        if before.is_empty() {
            changed = self.with_cursor_mut(cx, |c, b| c.expand_to_word(b, camel));
        }
        if !changed {
            let after = self.document.update(cx, |doc, _| {
                let range = before.range();
                let start = doc.buffer.char_to_byte(range.start);
                let end = doc.buffer.char_to_byte(range.end);
                let node = doc.syntax().and_then(|t| t.expand(start..end))?;
                Some(doc.buffer.byte_to_char(node.start)..doc.buffer.byte_to_char(node.end))
            });
            if let Some(range) = after {
                self.go(cx, |c, b| c.select(b, range.start, range.end));
                changed = true;
            } else if !before.is_empty() {
                let len = self.document.read(cx).buffer.len_chars();
                if before.range() != (0..len) {
                    self.go(cx, |c, b| c.select(b, 0, len));
                    changed = true;
                }
            }
        }
        if changed {
            self.expand_history.push(before);
            self.expand_last = Some(self.with_buffer(cx, |c, _| c.selection));
            self.after_move(cx);
        }
    }

    /// `shrink-selection`: back one `expand-selection` step, else into the
    /// largest child node at the caret.
    fn shrink_selection(&mut self, _: &ShrinkSelection, _: &mut Window, cx: &mut Context<Self>) {
        let current = self.with_buffer(cx, |c, _| c.selection);
        if self.expand_last == Some(current) {
            if let Some(previous) = self.expand_history.pop() {
                self.go(cx, |c, b| c.select(b, previous.anchor, previous.head));
                self.expand_last = Some(previous);
                self.after_move(cx);
                return;
            }
        }
        self.expand_history.clear();
        if current.is_empty() {
            return;
        }
        let inner = self.document.update(cx, |doc, _| {
            let range = current.range();
            let start = doc.buffer.char_to_byte(range.start);
            let end = doc.buffer.char_to_byte(range.end);
            let caret = doc.buffer.char_to_byte(current.head);
            let node = doc.syntax().and_then(|t| t.shrink(start..end, caret))?;
            Some(doc.buffer.byte_to_char(node.start)..doc.buffer.byte_to_char(node.end))
        });
        match inner {
            Some(range) => self.go(cx, |c, b| c.select(b, range.start, range.end)),
            None => self.go(cx, |c, b| {
                let head = c.selection.head;
                c.set_cursor(b, head);
            }),
        }
        self.expand_last = Some(self.with_buffer(cx, |c, _| c.selection));
        self.after_move(cx);
    }

    /// The bracket pair at the caret: the grammar's, else the text scan's.
    fn bracket_pair(&mut self, cx: &mut Context<Self>) -> Option<(usize, usize)> {
        let caret = self.caret(cx);
        self.document.update(cx, |doc, _| {
            let byte = doc.buffer.char_to_byte(caret);
            if let Some(tree) = doc.syntax() {
                if let Some((open, close)) = tree.matching_bracket(byte) {
                    return Some((
                        doc.buffer.byte_to_char(open),
                        doc.buffer.byte_to_char(close),
                    ));
                }
                // A plain-text fallback only where the grammar has no answer
                // because the caret is not on a bracket token at all.
            }
            analysis::matching_bracket(&doc.buffer, caret)
        })
    }

    fn move_to_matching_bracket(
        &mut self,
        _: &MoveToMatchingBracket,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let caret = self.caret(cx);
        let Some((open, close)) = self.bracket_pair(cx) else {
            return;
        };
        // From the opener to after the closer, from the closer to the opener.
        let to = if caret == open || caret == open + 1 {
            close + 1
        } else {
            open
        };
        self.go(cx, |c, b| c.set_cursor(b, to));
        self.after_move(cx);
    }

    /// `editor/collapse-folding`: the innermost block around the caret.
    fn collapse_folding(&mut self, _: &CollapseFolding, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.cursor_position(cx).row;
        let head = self.document.update(cx, |doc, _| {
            doc.folds()
                .iter()
                .filter(|f| f.start_row <= row && row <= f.end_row)
                .max_by_key(|f| f.start_row)
                .map(|f| f.start_row)
        });
        if let Some(head) = head {
            self.fold_row(head, cx);
            self.scroll_to_cursor(cx);
        }
    }

    /// `editor/expand-folding`: the folded block the caret is on or in.
    fn expand_folding(&mut self, _: &ExpandFolding, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.cursor_position(cx).row;
        let head = self
            .map
            .folds
            .iter()
            .filter(|f| f.start_row <= row && row <= f.end_row)
            .max_by_key(|f| f.start_row)
            .map(|f| f.start_row);
        if let Some(head) = head {
            self.unfold_row(head, cx);
        }
    }

    fn with_cursor_mut<R>(&mut self, cx: &App, f: impl FnOnce(&mut Cursor, &Buffer) -> R) -> R {
        let doc = self.document.read(cx);
        self.cursor.sync(&doc.buffer);
        f(&mut self.cursor, &doc.buffer)
    }

    // ---- what the find bar and the workspace drive ------------------------------

    pub fn set_search(&mut self, search: Option<Search>, cx: &mut Context<Self>) {
        if self.search != search {
            self.search = search;
            cx.notify();
        }
    }

    /// `select_range` for a byte range (a symbol's name from the syntax tree).
    pub fn select_bytes(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let (start, end) = {
            let b = &self.document.read(cx).buffer;
            (b.byte_to_char(range.start), b.byte_to_char(range.end))
        };
        self.select_range(start..end, cx);
    }

    /// A right click: the caret moves there unless the click lands in the
    /// selection (estimate: what most editors do), then the menu opens.
    fn right_click(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        if let Hit::Text(offset) = self.hit(event.position, window, cx) {
            let inside = self.with_buffer(cx, |c, _| {
                c.selections().iter().any(|s| s.range().contains(&offset))
            });
            if !inside {
                self.go(cx, |c, b| c.set_cursor(b, offset));
                cx.notify();
            }
        }
        cx.emit(EditorEvent::ContextMenu(event.position));
    }

    /// Selects `range` (the caret at its end), unfolding and scrolling to it.
    pub fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.select(b, range.start, range.end));
        self.reveal_carets(cx);
        self.center_cursor(cx);
        self.restart_blink(cx);
        cx.notify();
    }

    pub fn replace_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.replace_range(b, range, text));
        self.after_edit(cx);
    }

    /// Several replacements as one undo step (`replace-all`).
    pub fn replace_ranges(&mut self, edits: Vec<(Range<usize>, String)>, cx: &mut Context<Self>) {
        self.edit(cx, |c, b| c.replace_ranges(b, edits));
        self.after_edit(cx);
    }

    /// `goto/line`: one caret at the 0-based row and column, centred.
    pub fn goto(&mut self, row: usize, column: usize, cx: &mut Context<Self>) {
        self.go(cx, |c, b| c.goto(b, row, column));
        self.reveal_carets(cx);
        self.center_cursor(cx);
        self.restart_blink(cx);
        cx.notify();
    }

    /// The selection, or the whole current line when there is none (what
    /// Fleet copies on a bare Ctrl+C). Several carets copy their selections
    /// joined by newlines.
    fn selected_or_line(&mut self, cx: &App) -> String {
        self.with_buffer(cx, |c, b| {
            let selections = c.selections();
            if selections.len() > 1 {
                return selections
                    .iter()
                    .map(|s| {
                        if s.is_empty() {
                            b.line(b.offset_to_point(s.head).row)
                        } else {
                            b.slice(s.range())
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            if c.selection.is_empty() {
                let row = c.cursor_point(b).row;
                let mut line = b.line(row);
                line.push('\n');
                line
            } else {
                c.selected_text(b)
            }
        })
    }

    // ---- mouse ----------------------------------------------------------

    /// What a window point lands on: a text offset, a fold chevron, a folded
    /// mark, or a sticky line.
    fn hit(&mut self, position: Point<Pixels>, window: &mut Window, cx: &App) -> Hit {
        self.sync_map(cx);
        let bounds = self.last_bounds;
        let lh = self.line_height();
        let y_in = position.y - bounds.origin.y;
        if y_in >= px(0.) {
            let sticky = (y_in / lh).floor() as usize;
            if sticky < self.sticky_rows.len() {
                return Hit::Sticky(self.sticky_rows[sticky]);
            }
        }
        let y = y_in + self.scroll_y;
        let d = (y / lh).floor().max(0.) as usize;
        let (row, seg) = self.map.at(d);
        if position.x - bounds.origin.x < px(self.tm.line_status_hit) {
            if let Some(ix) = self.hunk_at(row, cx) {
                return Hit::LineStatus(ix);
            }
        }
        let x_in = position.x - bounds.origin.x - self.text_offset();
        if self.in_status_cell(x_in) && seg == 0 {
            if let Some(ix) = self.tui_hunk_at(row, cx) {
                return Hit::LineStatus(ix);
            }
        }
        // The tui gutter keeps the run icon in its own first cell.
        let run_left = if self.options.run_after_numbers && self.tm.cell.is_none() {
            px(self.tm.gutter_number_right + 4.)
        } else {
            px(self.tm.run_icon_x)
        };
        if self.options.run_in_gutter
            && seg == 0
            && x_in >= run_left
            && x_in < run_left + px(self.tm.icon_size)
        {
            if let Some((_, ix)) = self.run_rows(cx).iter().find(|(r, _)| *r == row) {
                return Hit::RunIcon(*ix);
            }
        }
        let chevron_left = px(self.tm.fold_chevron_x);
        if x_in >= chevron_left && x_in < chevron_left + px(self.tm.icon_size) && seg == 0 {
            if self.map.fold_at_head(row).is_some() {
                return Hit::Chevron(row);
            }
            let foldable = self.document.update_folds(cx, row);
            if foldable {
                return Hit::Chevron(row);
            }
        }
        let buffer = &self.document.read(cx).buffer;
        let range = self.map.segment_range(buffer, row, seg);
        let mut x = x_in - px(self.tm.gutter_width) + self.scroll_x;
        if seg > 0 {
            x -= px(self.tm.wrap_indent);
        }
        let theme = cx.global::<Theme>();
        let fold = self.map.fold_at_head(row).cloned();
        let line_text = buffer.line(row);
        if let Some(fold) = fold {
            let head_col = fold.inner.start - buffer.line_to_char(row);
            let line = DisplayLine::new(&line_text[..byte_of_col(&line_text, head_col)]);
            let shaped =
                shape_display_line(&line, &[], theme, self.tm.font_size, &self.options, window);
            let mark_left = shaped.width;
            let mark_right = mark_left + self.space_width() * self.tm.folded_mark_cells;
            if x >= mark_left && x < mark_right {
                return Hit::FoldedMark(row);
            }
            if x >= mark_right {
                let tail_line = DisplayLine::new(
                    &buffer.line(fold.end_row)[byte_of_col(
                        &buffer.line(fold.end_row),
                        fold.inner.end - buffer.line_to_char(fold.end_row),
                    )..],
                );
                let tail = shape_display_line(
                    &tail_line,
                    &[],
                    theme,
                    self.tm.font_size,
                    &self.options,
                    window,
                );
                let disp = tail.closest_index_for_x(x - mark_right);
                let col = fold.inner.end - buffer.line_to_char(fold.end_row)
                    + tail_line.char_for_display_byte(disp);
                return Hit::Text(buffer.point_to_offset(TextPoint::new(fold.end_row, col)));
            }
            let disp_byte = shaped.closest_index_for_x(x.max(px(0.)));
            let column = line.char_for_display_byte(disp_byte);
            return Hit::Text(buffer.point_to_offset(TextPoint::new(row, column)));
        }
        let seg_text =
            &line_text[byte_of_col(&line_text, range.start)..byte_of_col(&line_text, range.end)];
        let line = DisplayLine::with_start_column(seg_text, range.start, buffer.indent.width);
        let shaped =
            shape_display_line(&line, &[], theme, self.tm.font_size, &self.options, window);
        let disp_byte = shaped.closest_index_for_x(x.max(px(0.)));
        let column = range.start + line.char_for_display_byte(disp_byte);
        Hit::Text(buffer.point_to_offset(TextPoint::new(row, column)))
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        let hit = self.hit(event.position, window, cx);
        let shift = event.modifiers.shift;
        let alt = event.modifiers.alt;
        let clicks = event.click_count;
        if !matches!(hit, Hit::LineStatus(_)) && self.status_popup.take().is_some() {
            cx.notify();
        }
        match hit {
            Hit::LineStatus(ix) => {
                self.status_popup = if self.status_popup == Some(ix) {
                    None
                } else {
                    Some(ix)
                };
                cx.notify();
            }
            Hit::Chevron(row) => {
                if self.map.fold_at_head(row).is_some() {
                    self.unfold_row(row, cx);
                } else {
                    self.fold_row(row, cx);
                }
            }
            Hit::FoldedMark(row) => {
                self.unfold_row(row, cx);
            }
            Hit::RunIcon(ix) => {
                cx.emit(EditorEvent::RunConfiguration(ix));
            }
            Hit::Sticky(row) => {
                // A sticky line scrolls its block head to the top.
                let d = self.map.display_row(row, 0);
                self.scroll_y = self.line_height() * d as f32;
                cx.notify();
            }
            Hit::Text(offset) => {
                // Ctrl+click (Cmd on macOS) goes to the declaration (M7).
                if event.modifiers.secondary() && clicks < 2 && self.smart.on {
                    self.go(cx, |c, b| c.set_cursor(b, offset));
                    self.restart_blink(cx);
                    cx.emit(EditorEvent::GoToDefinition { offset });
                    cx.notify();
                    return;
                }
                self.go(cx, |c, b| match clicks {
                    2 => {
                        let range = c.word_range_at(b, offset);
                        c.select(b, range.start, range.end);
                    }
                    n if n >= 3 => {
                        let row = b.offset_to_point(offset).row;
                        let range = c.line_range(b, row);
                        c.select(b, range.start, range.end);
                    }
                    _ => {
                        if shift {
                            let anchor = c.selection.anchor;
                            c.select(b, anchor, offset);
                        } else if alt {
                            // Alt+click: another caret (kept as the primary).
                            c.add_caret_at(b, offset);
                        } else {
                            c.set_cursor(b, offset);
                        }
                    }
                });
                if clicks < 2 {
                    self.dragging = true;
                }
                self.goal_column = None;
                self.expand_last = None;
                self.restart_blink(cx);
                cx.notify();
            }
        }
    }

    fn mouse_drag(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        if let Hit::Text(offset) = self.hit(event.position, window, cx) {
            self.go(cx, |c, b| {
                let anchor = c.selection.anchor;
                c.select(b, anchor, offset);
            });
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, cx: &mut Context<Self>) {
        self.dragging = false;
        cx.notify();
    }

    /// Hovering the gutter shows the fold chevrons (batch 5).
    fn mouse_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let bounds = self.last_bounds;
        let x_in = position.x - bounds.origin.x - self.text_offset();
        let in_gutter = bounds.contains(&position) && x_in < px(self.tm.gutter_width);
        let row = if bounds.contains(&position) {
            let y = position.y - bounds.origin.y + self.scroll_y;
            let d = (y / self.line_height()).floor().max(0.) as usize;
            Some(self.map.at(d).0)
        } else {
            None
        };
        let status = match row {
            Some(r) if self.in_status_cell(x_in) => self.tui_hunk_at(r, cx),
            Some(r) if position.x - bounds.origin.x < px(self.tm.line_status_hit) => {
                self.hunk_at(r, cx)
            }
            _ => None,
        };
        let blame_hovered = self.tm.cell.is_some()
            && self.blame.is_some()
            && bounds.contains(&position)
            && x_in < px(0.)
            && x_in >= -px(self.tm.blame_width);
        if in_gutter != self.gutter_hovered
            || row != self.hovered_row
            || status != self.status_hovered
            || blame_hovered != self.blame_hovered
        {
            self.gutter_hovered = in_gutter;
            self.hovered_row = row;
            self.status_hovered = status;
            self.blame_hovered = blame_hovered;
            cx.notify();
        }
        self.hover_problem(position, cx);
        self.smart_track_pointer(position, cx);
    }

    /// The hunk whose rows hold `row` (a deletion sits on its row).
    fn hunk_at(&self, row: usize, cx: &App) -> Option<usize> {
        self.document
            .read(cx)
            .hunks
            .iter()
            .position(|h| hunk_covers(h, row))
    }

    /// The line-status popup: ↓ and ↑ step through the hunks, Revert puts
    /// HEAD's lines back, Diff opens the Changed Files tab (M4, later), ×
    /// closes (ninth pass: a 198 × 35.33 popup-coloured pill 39.33 above the
    /// hunk, 6 inside the island).
    fn render_status_popup(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let ix = self.status_popup?;
        let (row, count) = {
            let doc = self.document.read(cx);
            (doc.hunks.get(ix)?.row, doc.hunks.len())
        };
        let theme = cx.global::<Theme>().clone();
        let tm = self.tm;
        let lh = self.line_height();
        let d = self.map.display_row(row, 0);
        let top = lh * d as f32 - self.scroll_y - px(tm.line_status_popup_above);
        let m = *cx.global::<Metrics>();
        let button = |id: &'static str, label: &'static str, enabled: bool| {
            div()
                .id(id)
                .h_full()
                .px(px(m.popup_row_pad_x))
                .flex()
                .items_center()
                .rounded(px(m.popup_row_radius))
                .text_color(if enabled {
                    theme.popup_text
                } else {
                    theme.text_secondary
                })
                .when(enabled, |d| {
                    d.cursor_pointer().hover(|s| s.bg(theme.list_bg_hovered))
                })
                .child(label)
        };
        let separator = || {
            div()
                .w(px(1.))
                .h(px(m.find_bar_separator_height))
                .mx(px(m.find_bar_separator_gap_left))
                .bg(theme.separator)
        };
        Some(
            deferred(
                div()
                    .id("line-status-popup")
                    .occlude()
                    .absolute()
                    .top(top)
                    .left(px(tm.line_status_popup_left))
                    .w(px(tm.line_status_popup_width))
                    .h(px(tm.line_status_popup_height))
                    .px(px(m.popup_pad))
                    .flex()
                    .items_center()
                    .rounded(px(m.popup_radius))
                    .border_1()
                    .border_color(theme.popup_border)
                    .bg(theme.popup_bg)
                    .font_family(theme.ui_font.clone())
                    .text_size(px(m.ui_font_size))
                    .shadow(vec![BoxShadow {
                        color: theme.shadow_medium,
                        offset: point(px(0.), px(m.popup_shadow_offset_y)),
                        blur_radius: px(m.popup_shadow_blur),
                        spread_radius: px(0.),
                    }])
                    .child(
                        button("status-next", "↓", ix + 1 < count).on_click(
                            cx.listener(move |this, _, _, cx| this.step_status_popup(1, cx)),
                        ),
                    )
                    .child(button("status-prev", "↑", ix > 0).on_click(
                        cx.listener(move |this, _, _, cx| this.step_status_popup(-1, cx)),
                    ))
                    .child(separator())
                    .child(
                        button("status-revert", "Revert", true).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.document.update(cx, |d, cx| d.revert_hunk(ix, cx));
                                this.status_popup = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        button("status-diff", "Diff", true)
                            .tooltip(crate::tooltip::Tooltip::text("Show Diff"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.status_popup = None;
                                cx.emit(EditorEvent::ShowDiff);
                                cx.notify();
                            })),
                    )
                    .child(separator())
                    .child(button("status-close", "×", true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.status_popup = None;
                            cx.notify();
                        },
                    ))),
            )
            .into_any_element(),
        )
    }

    fn step_status_popup(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(ix) = self.status_popup else {
            return;
        };
        let next = ix as isize + by;
        let (row, count) = {
            let doc = self.document.read(cx);
            let Some(h) = doc.hunks.get(next.max(0) as usize) else {
                return;
            };
            (h.row, doc.hunks.len())
        };
        if next < 0 || next as usize >= count {
            return;
        }
        self.status_popup = Some(next as usize);
        let offset = self.document.read(cx).buffer.line_to_char(row);
        self.reveal(offset, cx);
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(self.line_height());
        if let Some(cell) = self.tm.cell {
            // The tui grid: a trackpad's deltas gather until they make a
            // whole row (or column), and the view moves by those.
            let (rows, rest_y) = whole_steps(self.scroll_rest.y, delta.y, cell.h);
            let (cols, rest_x) = whole_steps(self.scroll_rest.x, delta.x, cell.w);
            self.scroll_rest = Point {
                x: rest_x,
                y: rest_y,
            };
            let max = self.max_scroll_y(cx);
            let max_rows = (max / px(cell.h)).ceil();
            let row = (self.scroll_y / px(cell.h)).round();
            self.scroll_y = px(cell.h) * (row - rows).clamp(0., max_rows.max(0.));
            if self.options.soft_wrap {
                self.scroll_x = px(0.);
            } else {
                let col = (self.scroll_x / px(cell.w)).round();
                self.scroll_x = px(cell.w) * (col - cols).max(0.);
            }
            cx.notify();
            return;
        }
        let max = self.max_scroll_y(cx);
        self.scroll_y = (self.scroll_y - delta.y).clamp(px(0.), max);
        if self.options.soft_wrap {
            self.scroll_x = px(0.);
        } else {
            self.scroll_x = (self.scroll_x - delta.x).max(px(0.));
        }
        cx.notify();
    }

    // ---- utf16 bridging for the platform's text input --------------------

    fn char_to_utf16(&self, offset: usize, cx: &App) -> usize {
        let rope = self.document.read(cx).buffer.rope();
        rope.char_to_utf16_cu(offset.min(rope.len_chars()))
    }

    fn utf16_to_char(&self, utf16: usize, cx: &App) -> usize {
        let rope = self.document.read(cx).buffer.rope();
        rope.utf16_cu_to_char(utf16.min(rope.len_utf16_cu()))
    }

    fn selection(&mut self, cx: &App) -> Selection {
        self.with_buffer(cx, |c, _| c.selection)
    }
}

// ---- the tui skin: the frame's edges and the editor's boxes -------------------

/// A problem's mark in the tui skin's rows: a glyph and its colour.
fn severity_glyph(severity: Severity, theme: &Theme) -> (&'static str, Hsla) {
    match severity {
        Severity::Error => ("✗", theme.problem_error_underline),
        Severity::Warning => ("▲", theme.problem_warning_underline),
        Severity::Weak => ("△", theme.problem_weak_underline),
        Severity::Info => ("●", theme.problem_info_underline),
    }
}

impl EditorView {
    /// What the tui skin sets into the editor island's frame: the problems
    /// badge at the top edge's right end (` ✓ `, or ` ✗ 3  ▲ 1 `; a click
    /// toggles the problems widget, as Fleet's badge does) and the scroll
    /// bar on the right edge (the frame line is its rail; the marks are
    /// ticks on it; a click or a drag scrolls).
    pub fn tui_edges(editor: &Entity<EditorView>, cell: Cell, cx: &App) -> skin::Edges {
        let this = editor.read(cx);
        let theme = cx.global::<Theme>();
        let mut edges = skin::Edges::default();
        if this.options.file_status {
            let (errors, warnings, count) = this.badge;
            let mut parts: Vec<(String, Hsla)> = Vec::new();
            if count == 0 {
                parts.push(("✓".into(), theme.text_positive));
            } else {
                if errors > 0 {
                    parts.push((format!("✗ {errors}"), theme.problem_error_underline));
                }
                if warnings > 0 {
                    parts.push((format!("▲ {warnings}"), theme.problem_warning_underline));
                }
                if errors + warnings == 0 {
                    parts.push((format!("△ {count}"), theme.problem_weak_underline));
                }
            }
            let mut children: Vec<gpui::AnyElement> = Vec::new();
            for (i, (text, color)) in parts.into_iter().enumerate() {
                let gap = if i == 0 { " " } else { "  " };
                children.push(div().child(gap).into_any_element());
                children.push(
                    div()
                        .text_color(color)
                        .child(SharedString::from(text))
                        .into_any_element(),
                );
            }
            children.push(div().child(" ").into_any_element());
            let open = this.problems_open;
            let e = editor.clone();
            edges.top_end.push(
                div()
                    .id("tui-problems-badge")
                    .flex()
                    .flex_shrink_0()
                    .h(px(cell.h))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .when(open, |d| d.bg(theme.list_bg_selected))
                    .hover(|s| s.bg(theme.list_bg_hovered))
                    .on_click(move |_, _, cx| e.update(cx, |e, cx| e.toggle_problems(cx)))
                    .children(children)
                    .into_any_element(),
            );
        }
        let e = editor.clone();
        let bar = gpui::canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, cx| {
                let m = *cx.global::<Metrics>();
                let theme = cx.global::<Theme>().clone();
                let (bar, dragging) = {
                    let ed = e.read(cx);
                    (ed.tui_bar.clone(), ed.tui_bar_drag)
                };
                let thumb = if dragging {
                    theme.scrollbar_thumb_pressed
                } else if hitbox.is_hovered(window) {
                    theme.scrollbar_thumb_hovered
                } else {
                    theme.scrollbar_thumb
                };
                skin::paint_scrollbar(
                    window,
                    bounds,
                    cell,
                    bar.offset,
                    bar.visible,
                    bar.total,
                    None,
                    thumb,
                    &m,
                );
                // The marks: a tick across the centre of the cell their row
                // scales to.
                let rows = Cell::fit(bounds.size.height, cell.h);
                if rows > 0 && bar.total > 0. {
                    let t = px(skin::stroke(window, &m));
                    for (row, color) in &bar.marks {
                        let r = ((*row as f32 / bar.total) * rows as f32)
                            .floor()
                            .min((rows - 1) as f32);
                        let y = px(skin::snap(
                            window,
                            f32::from(bounds.origin.y) + (r + 0.5) * cell.h,
                        )) - t;
                        fill_cells(
                            window,
                            bounds.origin.x + px(cell.w * 0.2),
                            bounds.origin.x + px(cell.w * 0.8),
                            y,
                            t * 2.,
                            *color,
                        );
                    }
                }
                let _ = bar.rows;
                if bar.total <= bar.visible {
                    return;
                }
                let (down, moved, up) = (e.clone(), e.clone(), e.clone());
                let hb = hitbox.clone();
                window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble
                        && ev.button == MouseButton::Left
                        && hb.is_hovered(window)
                    {
                        down.update(cx, |ed, cx| {
                            ed.tui_bar_drag = true;
                            ed.scroll_bar_to(ev.position.y, bounds, cx);
                        });
                    }
                });
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && ev.pressed_button == Some(MouseButton::Left)
                        && moved.read(cx).tui_bar_drag
                    {
                        moved.update(cx, |ed, cx| ed.scroll_bar_to(ev.position.y, bounds, cx));
                    }
                });
                window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && ev.button == MouseButton::Left
                        && up.read(cx).tui_bar_drag
                    {
                        up.update(cx, |ed, cx| {
                            ed.tui_bar_drag = false;
                            cx.notify();
                        });
                    }
                });
            },
        )
        .size_full();
        edges.right = Some(bar.into_any_element());
        edges
    }

    /// A click or drag at window `y` on the tui scroll bar (`bounds`): the
    /// thumb centres there, on a whole row.
    fn scroll_bar_to(&mut self, y: Pixels, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        let bar = self.tui_bar.clone();
        if bounds.size.height <= px(0.) {
            return;
        }
        let frac = ((y - bounds.origin.y) / bounds.size.height).clamp(0., 1.);
        let top = frac * bar.total - bar.visible / 2.;
        let lh = self.line_height();
        let max_rows = (self.max_scroll_y(cx) / lh).ceil().max(0.);
        self.scroll_y = lh * top.round().clamp(0., max_rows);
        cx.notify();
    }

    /// The tui skin's problems: the counts for the frame's badge, the
    /// widget as a box under it (one row per problem, `✗ 12:5  message`, the
    /// one at the caret filled with its fixes under it) and the hovered
    /// problem's tooltip as a row of cells (a box when it has several lines).
    fn render_problems_tui(
        &mut self,
        cell: Cell,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let mut out = Vec::new();
        let problems = self.problems(cx);
        let count = problems.len();
        let errors = problems
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .count();
        let warnings = problems
            .iter()
            .filter(|p| p.severity == Severity::Warning)
            .count();
        if self.badge != (errors, warnings, count) {
            // The frame drew the old counts this frame.
            self.badge = (errors, warnings, count);
            skin::redraw(window);
        }
        if !self.options.file_status {
            return out;
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let w = px(cell.w);
        let body_cols = Cell::fit(self.last_bounds.size.width, cell.w);
        let body_rows = Cell::fit(self.last_bounds.size.height, cell.h);
        if self.problems_open && count > 0 && body_cols >= 16 && body_rows >= 4 {
            let head = self.with_buffer(cx, |c, _| c.selection.head);
            let at_caret = problems
                .iter()
                .position(|p| p.range.start <= head && head <= p.range.end.max(p.range.start + 1));
            let expanded = at_caret.or(self.problem_expanded.filter(|ix| *ix < count));
            let fixes: Vec<String> = match (&self.smart.fixes, expanded) {
                (Some((for_ix, titles)), Some(ix)) if *for_ix == ix => titles.clone(),
                _ => Vec::new(),
            };
            let doc = self.document.read(cx);
            let mut widest = 0;
            let mut lines: Vec<gpui::AnyElement> = Vec::new();
            for (ix, p) in problems.iter().enumerate() {
                let at = doc.buffer.offset_to_point(p.range.start);
                let loc = format!("{}:{}", at.row + 1, at.column + 1);
                let message = p.message.lines().next().unwrap_or("").to_string();
                let (glyph, color) = severity_glyph(p.severity, &theme);
                widest = widest.max(2 + loc.chars().count() + 2 + message.chars().count());
                let selected = expanded == Some(ix);
                lines.push(
                    div()
                        .id(ElementId::NamedInteger("problem-row".into(), ix as u64))
                        .flex()
                        .flex_shrink_0()
                        .h(px(cell.h))
                        .px(w)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .cursor_pointer()
                        .when(selected, |d| d.bg(theme.list_bg_selected))
                        .hover(|s| s.bg(theme.list_bg_hovered))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let problems = this.problems(cx);
                            this.show_problem(ix, &problems, cx);
                        }))
                        .child(div().w(w).flex_shrink_0().text_color(color).child(glyph))
                        .child(div().w(w).flex_shrink_0())
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(theme.text_secondary)
                                .child(SharedString::from(loc)),
                        )
                        .child(div().w(w * 2.).flex_shrink_0())
                        .child(
                            div()
                                .text_color(theme.popup_text)
                                .child(SharedString::from(message)),
                        )
                        .into_any_element(),
                );
                if selected {
                    // The server's quick fixes, one row each under it.
                    let row = |text: String, color: Hsla| {
                        div()
                            .flex_shrink_0()
                            .h(px(cell.h))
                            .pl(w * 3.)
                            .pr(w)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_color(color)
                            .child(SharedString::from(text))
                    };
                    if fixes.is_empty() {
                        lines.push(
                            row("No fixes available".into(), theme.text_tertiary)
                                .into_any_element(),
                        );
                        widest = widest.max(2 + 18);
                    }
                    for (fix, title) in fixes.iter().enumerate() {
                        widest = widest.max(4 + title.chars().count());
                        lines.push(
                            row(format!("↳ {title}"), theme.link)
                                .id(ElementId::NamedInteger(
                                    "problem-fix".into(),
                                    (ix * 1000 + fix) as u64,
                                ))
                                .cursor_pointer()
                                .hover(|s| s.bg(theme.list_bg_hovered))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(EditorEvent::ApplyProblemFix { problem: ix, fix });
                                }))
                                .into_any_element(),
                        );
                    }
                }
            }
            let rows = (lines.len() + 2).min(body_rows);
            let cols = (widest + 4).clamp(24, body_cols);
            let plural = |n: usize, word: &str| {
                if n == 1 {
                    format!("1 {word}")
                } else {
                    format!("{n} {word}s")
                }
            };
            let mut parts = Vec::new();
            if errors > 0 {
                parts.push(plural(errors, "error"));
            }
            if warnings > 0 {
                parts.push(plural(warnings, "warning"));
            }
            if parts.is_empty() {
                parts.push(plural(count, "problem"));
            }
            let hints: Vec<String> = [
                ("editor/go-to-next-problem", "next"),
                ("editor/go-to-previous-problem", "prev"),
            ]
            .iter()
            .filter_map(|(action, word)| {
                crate::keymap::hint_for(cx, action).map(|c| format!("{c} {word}"))
            })
            .collect();
            let body = div()
                .size_full()
                .flex()
                .flex_col()
                .children(lines)
                .into_any_element();
            let mut b = skin::boxed(
                cell,
                cols,
                rows,
                vec![skin::title(parts.join(", "), theme.popup_text)],
                Vec::new(),
                body,
                theme.popup_bg,
                theme.border,
                &theme,
                m,
            );
            if !hints.is_empty() && rows > 2 && cols > 6 {
                b = b.child(
                    div()
                        .absolute()
                        .top(cell.rows(rows - 1))
                        .left(cell.cols(2))
                        .max_w(cell.cols(cols - 4))
                        .h(px(cell.h))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .bg(theme.popup_bg)
                        .text_color(theme.text_tertiary)
                        .child(SharedString::from(format!(" {} ", hints.join("  ")))),
                );
            }
            out.push(
                b.id("problems-card")
                    .occlude()
                    .absolute()
                    .top_0()
                    .right_0()
                    .into_any_element(),
            );
        }
        if self.problem_tooltip_shown {
            if let Some(p) = self.problem_hovered.and_then(|ix| problems.get(ix)) {
                let at = self.hover_position - self.last_bounds.origin;
                let col = (at.x / w).floor().max(0.) as usize;
                let row = (at.y / px(cell.h)).floor().max(0.) as usize + 1;
                let lines: Vec<&str> = p.message.lines().take(12).collect();
                let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
                let tip = if lines.len() <= 1 {
                    // One line: a row of cells, a cell of margin each side.
                    let cols = longest + 2;
                    div()
                        .h(px(cell.h))
                        .w(cell.cols(cols))
                        .bg(theme.tooltip_bg)
                        .text_color(theme.tooltip_text)
                        .font_family(theme.mono_font.clone())
                        .text_size(px(cell.font_size))
                        .line_height(px(cell.h))
                        .whitespace_nowrap()
                        .child(SharedString::from(format!(" {} ", p.message.trim())))
                        .into_any_element()
                } else {
                    let cols = (longest + 4).min(body_cols.max(8));
                    let body = div()
                        .flex()
                        .flex_col()
                        .px(w)
                        .text_color(theme.tooltip_text)
                        .children(lines.iter().map(|l| {
                            div()
                                .h(px(cell.h))
                                .whitespace_nowrap()
                                .child(SharedString::from(l.to_string()))
                        }))
                        .into_any_element();
                    skin::boxed(
                        cell,
                        cols,
                        lines.len() + 2,
                        Vec::new(),
                        Vec::new(),
                        body,
                        theme.tooltip_bg,
                        theme.border,
                        &theme,
                        m,
                    )
                    .into_any_element()
                };
                let cols = if lines.len() <= 1 {
                    longest + 2
                } else {
                    longest + 4
                };
                let col = col.min(body_cols.saturating_sub(cols));
                out.push(
                    div()
                        .absolute()
                        .left(cell.cols(col))
                        .top(cell.rows(row))
                        .child(tip)
                        .into_any_element(),
                );
            }
        }
        out
    }

    /// The tui skin's line-status popup: a one-row box above the hunk (under
    /// it at the top), ` ↓  ↑   Revert  Diff   × `, its buttons filling on
    /// hover.
    fn render_status_popup_tui(
        &mut self,
        cell: Cell,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let ix = self.status_popup?;
        let (row, count) = {
            let doc = self.document.read(cx);
            (doc.hunks.get(ix)?.row, doc.hunks.len())
        };
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let d = self.map.display_row(row, 0) as isize;
        let first = (self.scroll_y / px(cell.h)).round() as isize;
        let rel = d - first;
        let top = if rel >= 3 { rel - 3 } else { rel + 1 }.max(0) as usize;
        let button = |id: &'static str, label: &'static str, enabled: bool| {
            div()
                .id(id)
                .flex_shrink_0()
                .h(px(cell.h))
                .w(cell.cols(Cell::text_cols(label)))
                .whitespace_nowrap()
                .text_color(if enabled {
                    theme.popup_text
                } else {
                    theme.text_secondary
                })
                .when(enabled, |d| {
                    d.cursor_pointer().hover(|s| s.bg(theme.list_bg_hovered))
                })
                .child(label)
        };
        let gap = |n: usize| div().flex_shrink_0().w(cell.cols(n));
        let row_el = div()
            .flex()
            .child(
                button("status-next", " ↓ ", ix + 1 < count)
                    .on_click(cx.listener(move |this, _, _, cx| this.step_status_popup(1, cx))),
            )
            .child(
                button("status-prev", " ↑ ", ix > 0)
                    .on_click(cx.listener(move |this, _, _, cx| this.step_status_popup(-1, cx))),
            )
            .child(gap(1))
            .child(
                button("status-revert", " Revert ", true).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.document.update(cx, |d, cx| d.revert_hunk(ix, cx));
                        this.status_popup = None;
                        cx.notify();
                    },
                )),
            )
            .child(
                button("status-diff", " Diff ", true).on_click(cx.listener(|this, _, _, cx| {
                    this.status_popup = None;
                    cx.emit(EditorEvent::ShowDiff);
                    cx.notify();
                })),
            )
            .child(gap(1))
            .child(
                button("status-close", " × ", true).on_click(cx.listener(|this, _, _, cx| {
                    this.status_popup = None;
                    cx.notify();
                })),
            )
            .into_any_element();
        // ↓ ↑, a gap, Revert Diff, a gap, ×: 3 + 3 + 1 + 8 + 6 + 1 + 3.
        let inner = 25;
        Some(
            deferred(
                skin::boxed(
                    cell,
                    inner + 2,
                    3,
                    Vec::new(),
                    Vec::new(),
                    row_el,
                    theme.popup_bg,
                    theme.border,
                    &theme,
                    m,
                )
                .id("line-status-popup")
                .occlude()
                .absolute()
                .top(cell.rows(top))
                .left(self.text_offset()),
            )
            .into_any_element(),
        )
    }
}

/// What a point in the view lands on.
enum Hit {
    Text(usize),
    Chevron(usize),
    FoldedMark(usize),
    Sticky(usize),
    /// A git line-status bar on the island's edge (the hunk's index).
    LineStatus(usize),
    /// A run icon in `.fleet/run.json`'s gutter (the configuration's index).
    RunIcon(usize),
}

/// Whether a hunk's rows hold `row`; a deletion sits on the row after it.
fn hunk_covers(hunk: &weep_git::Hunk, row: usize) -> bool {
    if hunk.rows == 0 {
        hunk.row == row
    } else {
        row >= hunk.row && row < hunk.row + hunk.rows
    }
}

/// A wheel delta gathered onto `rest`, in whole steps of `pitch`: the steps
/// it makes (signed, as the delta: positive scrolls back) and what is left.
/// A turn the other way drops what was gathered.
fn whole_steps(rest: Pixels, delta: Pixels, pitch: f32) -> (f32, Pixels) {
    let rest = if f32::from(rest) * f32::from(delta) < 0. {
        px(0.)
    } else {
        rest
    };
    let total = f32::from(rest + delta);
    let steps = (total / pitch.max(1.)).trunc();
    (steps, px(total - steps * pitch.max(1.)))
}

/// The tui skin: whether a deletion's mark sits on `row` (on its bottom
/// edge; on row 0's top for a deletion before the first row).
fn tui_deletion_on(hunk: &weep_git::Hunk, row: usize) -> bool {
    hunk.rows == 0 && (hunk.row == row + 1 || (hunk.row == 0 && row == 0))
}

/// Whether `row` heads a foldable block, asked of the document.
trait FoldQuery {
    fn update_folds(&self, cx: &App, row: usize) -> bool;
}

impl FoldQuery for Entity<Document> {
    fn update_folds(&self, cx: &App, row: usize) -> bool {
        // `folds()` needs `&mut Document`; without a mutable context the
        // cached folds are read when fresh, else nothing is foldable here.
        let doc = self.read(cx);
        doc.cached_folds().iter().any(|f| f.start_row == row)
    }
}

/// The byte offset of char column `col` in `line`.
fn byte_of_col(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let start = self.utf16_to_char(range.start, cx);
        let end = self.utf16_to_char(range.end, cx);
        *adjusted_range = Some(self.char_to_utf16(start, cx)..self.char_to_utf16(end, cx));
        Some(self.document.read(cx).buffer.slice(start..end))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let selection = self.selection(cx);
        let range = selection.range();
        Some(UTF16Selection {
            range: self.char_to_utf16(range.start, cx)..self.char_to_utf16(range.end, cx),
            reversed: selection.head < selection.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked
            .clone()
            .map(|r| self.char_to_utf16(r.start, cx)..self.char_to_utf16(r.end, cx))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    /// Typed text. One character goes through `type_char` (auto-pairs and the
    /// indenter); anything longer (a paste through the IME, a dead-key
    /// composition) is inserted as it is.
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(range) = range.or_else(|| {
            self.marked
                .clone()
                .map(|r| self.char_to_utf16(r.start, cx)..self.char_to_utf16(r.end, cx))
        }) {
            let start = self.utf16_to_char(range.start, cx);
            let end = self.utf16_to_char(range.end, cx);
            self.go(cx, |c, b| {
                if (start..end) != c.selection.range() {
                    c.select(b, start, end);
                }
            });
        }
        self.marked = None;
        let pairs = self.language(cx).is_some_and(Language::auto_pairs);
        let indenter = self.options.indenter;
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => self.edit(cx, |cur, b| cur.type_char(b, c, pairs, indenter)),
            _ => self.edit(cx, |c, b| c.insert(b, text)),
        }
        self.after_edit(cx);
        self.smart_typed(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = match range.or_else(|| {
            self.marked
                .clone()
                .map(|r| self.char_to_utf16(r.start, cx)..self.char_to_utf16(r.end, cx))
        }) {
            Some(range) => {
                let start = self.utf16_to_char(range.start, cx);
                let end = self.utf16_to_char(range.end, cx);
                self.go(cx, |c, b| c.select(b, start, end));
                start
            }
            None => self.selection(cx).range().start,
        };
        self.edit(cx, |c, b| c.insert(b, new_text));
        let len = new_text.chars().count();
        self.marked = Some(start..start + len);
        if let Some(sel) = new_selected_range {
            let a = start + new_text[..new_text.len().min(sel.start)].chars().count();
            let e = start + new_text[..new_text.len().min(sel.end)].chars().count();
            self.go(cx, |c, b| c.select(b, a, e));
        }
        self.after_edit(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let offset = self.utf16_to_char(range_utf16.start, cx);
        self.sync_map(cx);
        let doc = self.document.read(cx);
        let at = doc.buffer.offset_to_point(offset);
        let theme = cx.global::<Theme>();
        let seg = self.map.segment_of(at.row, at.column);
        let range = self.map.segment_range(&doc.buffer, at.row, seg);
        let text = doc.buffer.line(at.row);
        let seg_text = &text[byte_of_col(&text, range.start)..byte_of_col(&text, range.end)];
        let line = DisplayLine::with_start_column(seg_text, range.start, doc.buffer.indent.width);
        let shaped =
            shape_display_line(&line, &[], theme, self.tm.font_size, &self.options, window);
        let x = shaped.x_for_index(line.display_byte_for_char(at.column - range.start));
        let lh = self.line_height();
        let d = self.map.display_row(at.row, at.column);
        Some(Bounds::new(
            point(
                element_bounds.origin.x + self.text_offset() + px(self.tm.gutter_width) + x
                    - self.scroll_x,
                element_bounds.origin.y + lh * d as f32 - self.scroll_y,
            ),
            size(px(self.tm.caret_width), lh),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        match self.hit(point, window, cx) {
            Hit::Text(offset) => Some(self.char_to_utf16(offset, cx)),
            _ => None,
        }
    }
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tui = (Skin::current(cx) == Skin::Tui).then(|| Cell::current(cx));
        let (status_popup, problems) = match tui {
            Some(cell) => (
                self.render_status_popup_tui(cell, cx),
                self.render_problems_tui(cell, window, cx),
            ),
            None => (self.render_status_popup(cx), self.render_problems(cx)),
        };
        let smart = self.render_smart(window, cx);
        // The badge, the widget and the hover tooltip come after the
        // element, so they paint over its washes.
        div()
            .relative()
            .children(status_popup)
            .key_context("Editor")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::go_to_next_problem))
            .on_action(cx.listener(Self::go_to_previous_problem))
            .on_action(cx.listener(Self::smart_move_up))
            .on_action(cx.listener(Self::smart_move_down))
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::smart_page_up))
            .on_action(cx.listener(Self::smart_page_down))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::delete_word_forward))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(Self::duplicate_line))
            .on_action(cx.listener(Self::move_line_up))
            .on_action(cx.listener(Self::move_line_down))
            .on_action(cx.listener(Self::smart_newline))
            .on_action(cx.listener(Self::newline_below))
            .on_action(cx.listener(Self::newline_above))
            .on_action(cx.listener(Self::smart_tab))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::comment_line))
            .on_action(cx.listener(Self::comment_block))
            .on_action(cx.listener(Self::join_lines))
            .on_action(cx.listener(Self::toggle_case))
            .on_action(cx.listener(Self::smart_cancel))
            .on_action(cx.listener(Self::add_caret_above))
            .on_action(cx.listener(Self::add_caret_below))
            .on_action(cx.listener(Self::add_carets_to_line_ends))
            .on_action(cx.listener(Self::select_next_occurrence))
            .on_action(cx.listener(Self::select_all_occurrences))
            .on_action(cx.listener(Self::unselect_occurrence))
            .on_action(cx.listener(Self::expand_selection))
            .on_action(cx.listener(Self::shrink_selection))
            .on_action(cx.listener(Self::move_to_matching_bracket))
            .on_action(cx.listener(Self::collapse_folding))
            .on_action(cx.listener(Self::expand_folding))
            .size_full()
            .child(EditorElement {
                editor: cx.entity(),
            })
            .children(problems)
            .children(smart)
    }
}

// ---- line display ---------------------------------------------------------

/// A source line (or a wrapped segment of one) made ready for the shaper:
/// tabs expanded to spaces, with the maps between source chars and display
/// bytes both ways.
struct DisplayLine {
    text: String,
    /// Display byte offset of each source char, plus one past the end.
    char_to_display: Vec<usize>,
    /// Source byte offset of each source char, plus one past the end.
    char_to_source: Vec<usize>,
    /// Display byte ranges of the tabs, for the whitespace glyphs.
    tabs: Vec<Range<usize>>,
}

impl DisplayLine {
    fn new(source: &str) -> Self {
        DisplayLine::with_start_column(source, 0, weep_text::TAB_WIDTH)
    }

    /// `source` starts at display column `start_column` (a wrapped segment),
    /// so its tabs expand to the right stops.
    fn with_start_column(source: &str, start_column: usize, tab_width: usize) -> Self {
        let tab_width = tab_width.max(1);
        let mut text = String::with_capacity(source.len());
        let mut char_to_display = Vec::with_capacity(source.len() + 1);
        let mut char_to_source = Vec::with_capacity(source.len() + 1);
        let mut tabs = Vec::new();
        let mut column = start_column;
        for (byte, c) in source.char_indices() {
            char_to_display.push(text.len());
            char_to_source.push(byte);
            if c == '\t' {
                let width = tab_width - column % tab_width;
                let start = text.len();
                text.extend(std::iter::repeat_n(' ', width));
                tabs.push(start..text.len());
                column += width;
            } else {
                text.push(c);
                column += 1;
            }
        }
        char_to_display.push(text.len());
        char_to_source.push(source.len());
        DisplayLine {
            text,
            char_to_display,
            char_to_source,
            tabs,
        }
    }

    fn display_byte_for_char(&self, column: usize) -> usize {
        self.char_to_display[column.min(self.char_to_display.len() - 1)]
    }

    fn char_for_display_byte(&self, byte: usize) -> usize {
        match self.char_to_display.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    fn display_byte_for_source_byte(&self, source_byte: usize) -> usize {
        match self.char_to_source.binary_search(&source_byte) {
            Ok(i) => self.char_to_display[i],
            Err(i) => self.char_to_display[i.min(self.char_to_display.len() - 1)],
        }
    }
}

fn editor_font(weight: FontWeight, italic: bool, theme: &Theme, options: &ViewOptions) -> Font {
    Font {
        family: theme.mono_font.clone(),
        features: options.font_features(),
        fallbacks: None,
        weight,
        style: if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    }
}

/// A source line as a view outside the editor element draws it (the Changed
/// Files tab): tabs expanded, with the highlight runs for `spans` (in source
/// bytes relative to the line's start).
pub fn styled_line(
    source: &str,
    spans: &[(Range<usize>, &'static str)],
    theme: &Theme,
    cx: &App,
) -> (String, Vec<TextRun>) {
    let options = ViewOptions::from(&cx.global::<AppSettings>().settings);
    let line = DisplayLine::new(source);
    let runs = line_runs(&line, spans, theme, &options);
    (line.text, runs)
}

/// Shapes one display line. `spans` are the highlight spans that overlap
/// the line, in source bytes relative to the line's start.
fn shape_display_line(
    line: &DisplayLine,
    spans: &[(Range<usize>, &'static str)],
    theme: &Theme,
    font_size: f32,
    options: &ViewOptions,
    window: &mut Window,
) -> ShapedLine {
    let runs = line_runs(line, spans, theme, options);
    window.text_system().shape_line(
        SharedString::from(line.text.clone()),
        px(font_size),
        &runs,
        None,
    )
}

/// The text runs of a display line: the plain editor font, with each
/// highlight span's colour, weight, style and underline.
fn line_runs(
    line: &DisplayLine,
    spans: &[(Range<usize>, &'static str)],
    theme: &Theme,
    options: &ViewOptions,
) -> Vec<TextRun> {
    let base_weight = FontWeight(options.font_weight);
    let plain = editor_font(base_weight, false, theme, options);
    let mut runs: Vec<TextRun> = Vec::new();
    let mut cursor = 0;
    let push =
        |runs: &mut Vec<TextRun>, len: usize, font: Font, color: Hsla, underline: Option<Hsla>| {
            if len == 0 {
                return;
            }
            runs.push(TextRun {
                len,
                font,
                color,
                background_color: None,
                underline: underline.map(|color| gpui::UnderlineStyle {
                    thickness: px(1.),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: None,
            });
        };
    for (range, attribute) in spans {
        let start = line.display_byte_for_source_byte(range.start).max(cursor);
        let end = line
            .display_byte_for_source_byte(range.end)
            .min(line.text.len());
        if end <= start {
            continue;
        }
        push(
            &mut runs,
            start - cursor,
            plain.clone(),
            theme.editor_text,
            None,
        );
        let style = theme.syntax(attribute);
        let color = style.and_then(|s| s.color).unwrap_or(theme.editor_text);
        let font = match style {
            Some(s) => {
                let weight = if s.weight == FontWeight::NORMAL {
                    base_weight
                } else {
                    s.weight
                };
                editor_font(weight, s.italic, theme, options)
            }
            None => plain.clone(),
        };
        push(
            &mut runs,
            end - start,
            font,
            color,
            style.and_then(|s| s.underline),
        );
        cursor = end;
    }
    push(
        &mut runs,
        line.text.len() - cursor,
        plain.clone(),
        theme.editor_text,
        None,
    );
    if runs.is_empty() {
        runs.push(TextRun {
            len: 0,
            font: plain,
            color: theme.editor_text,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    runs
}

/// A glyph painted on its own (the whitespace dots and arrows, the folded mark).
fn shape_glyph(
    text: &str,
    color: Hsla,
    theme: &Theme,
    font_size: f32,
    options: &ViewOptions,
    window: &mut Window,
) -> ShapedLine {
    let text = SharedString::from(text.to_string());
    window.text_system().shape_line(
        text.clone(),
        px(font_size),
        &[TextRun {
            len: text.len(),
            font: editor_font(FontWeight(options.font_weight), false, theme, options),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    )
}

// ---- the element -------------------------------------------------------------

pub struct EditorElement {
    editor: Entity<EditorView>,
}

/// The highlight spans overlapping one row, in source bytes from its start.
type LineSpans = Vec<(Range<usize>, &'static str)>;

/// A highlighted stretch of a row, in display bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoxKind {
    Occurrence,
    Bracket,
    SearchMatch,
    /// A problem's underline (`problem.error`, `.warning`, `.warning.weak`, `.info`).
    Problem(Severity),
}

/// The folded mark on a head row: the display byte it sits after, and the
/// tail of the block's last row shaped after it.
struct FoldedMarkLayout {
    at: usize,
    mark: ShapedLine,
    tail: ShapedLine,
}

struct LaidOutRow {
    row: usize,
    /// The chars of the row this display row shows.
    cols: Range<usize>,
    line: DisplayLine,
    shaped: ShapedLine,
    number: Option<ShapedLine>,
    boxes: Vec<(Range<usize>, BoxKind)>,
    /// Indent-guide columns, and whether each is the caret block's.
    guides: Vec<(usize, bool)>,
    /// A fold chevron: open, or folded.
    chevron: Option<bool>,
    /// The row carries a run icon (`.fleet/run.json`, M6).
    run_icon: bool,
    folded: Option<FoldedMarkLayout>,
    /// Display byte offsets of the spaces, for the whitespace dots.
    spaces: Vec<usize>,
    continuation: bool,
    /// The blame column's bar colour for this row and, on a chunk's first
    /// row, its label.
    blame: Option<BlameLayout>,
}

struct BlameLayout {
    label: Option<ShapedLine>,
    color: Hsla,
}

/// One sticky line: a block head pinned to the top.
struct StickyLayout {
    row: usize,
    shaped: ShapedLine,
    number: Option<ShapedLine>,
}

pub struct Prepaint {
    hitbox: Hitbox,
    rows: Vec<LaidOutRow>,
    scroll_y: Pixels,
    scroll_x: Pixels,
    /// Every selection, its head's point first.
    selections: Vec<(Selection, TextPoint)>,
    cursor: TextPoint,
    focused: bool,
    caret_visible: bool,
    total_rows: usize,
    tm: TextMetrics,
    options: ViewOptions,
    /// One space's advance, for guide columns past a line's end.
    space_width: Pixels,
    /// Rows that get a scrollbar mark: search matches, and the problems'.
    mark_rows: Vec<usize>,
    caret_mark_rows: Vec<usize>,
    problem_mark_rows: Vec<(usize, Severity)>,
    text_offset: Pixels,
    sticky: Vec<StickyLayout>,
    space_glyph: Option<ShapedLine>,
    tab_glyph: Option<ShapedLine>,
    hovered_row: Option<usize>,
    gutter_hovered: bool,
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let theme = cx.global::<Theme>().clone();
        let options = ViewOptions::from(&cx.global::<AppSettings>().settings);
        let mut tm = TextMetrics::current(cx);
        if tm.cell.is_some() {
            // The tui gutter is sized by the line count's digits.
            let lines = self.editor.read(cx).document.read(cx).buffer.line_count();
            tm = tm.with_digits(tui_digits(lines, options.line_numbers != LineNumbers::Off));
        }
        let settings = cx.global::<AppSettings>().settings.clone();
        let lh = px(tm.line_height);
        let focused = self.editor.read(cx).focus_handle.is_focused(window);
        let space_width = match tm.cell {
            // A column of the tui grid is the cell, exactly.
            Some(cell) => px(cell.w),
            None => {
                shape_display_line(
                    &DisplayLine::new(" "),
                    &[],
                    &theme,
                    tm.font_size,
                    &options,
                    window,
                )
                .width
            }
        };
        let (
            document,
            selections,
            cursor,
            scroll_y,
            scroll_x,
            occurrences,
            search,
            folds,
            caret_visible,
            text_offset,
            hovered,
            gutter_hovered,
        ) = self.editor.update(cx, |editor, cx| {
            // A zoom or font change keeps the same first line in view.
            if editor.tm != tm {
                let row = editor.scroll_y / px(editor.tm.line_height);
                editor.tm = tm;
                editor.scroll_y = px(tm.line_height) * row;
            }
            editor.space_px = space_width;
            if editor.options != options {
                let blink_changed = editor.options.caret_blink != options.caret_blink;
                editor.options = options.clone();
                if blink_changed {
                    editor.restart_blink(cx);
                }
            }
            editor.last_bounds = bounds;
            editor
                .document
                .update(cx, |doc, _| doc.refresh_indent(&settings));
            editor.sync_map(cx);
            let max = editor.max_scroll_y(cx);
            if editor.scroll_y > max {
                editor.scroll_y = max;
            }
            if options.soft_wrap {
                editor.scroll_x = px(0.);
            }
            if let Some(cell) = tm.cell {
                // The tui grid scrolls by whole rows and columns, whatever
                // moved it (the caret, a jump, a page).
                let max_rows = (max / px(cell.h)).ceil();
                let row = (editor.scroll_y / px(cell.h)).round().min(max_rows);
                editor.scroll_y = px(cell.h) * row.max(0.);
                editor.scroll_x = px(snap_to(f32::from(editor.scroll_x), cell.w));
            }
            let (cursor, selections) = editor.with_buffer(cx, |c, b| {
                (
                    c.cursor_point(b),
                    c.selections()
                        .into_iter()
                        .map(|s| (s, b.offset_to_point(s.head)))
                        .collect::<Vec<_>>(),
                )
            });
            let occurrences = editor.occurrences(cx);
            (
                editor.document.clone(),
                selections,
                cursor,
                editor.scroll_y,
                editor.scroll_x,
                occurrences,
                editor.search.clone(),
                editor.map.clone(),
                editor.caret_visible,
                editor.text_offset(),
                editor.hovered_row,
                editor.gutter_hovered,
            )
        });
        let run_rows: Vec<(usize, usize)> = self.editor.update(cx, |e, cx| e.run_rows(cx));
        let map = folds;
        let mut rows = Vec::new();
        let mut mark_rows = Vec::new();
        let mut problem_mark_rows = Vec::new();
        let mut sticky = Vec::new();
        let total_rows = map.total;
        // The blame column (ninth pass): 13 px UI text in the secondary colour.
        let blame_lines = self.editor.read(cx).blame.clone();
        let today = today_days();
        let ui_font_size = cx.global::<Metrics>().ui_font_size;
        let ui_font = Font {
            family: theme.ui_font.clone(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };
        let shape_blame = |text: String, window: &mut Window| {
            let text = SharedString::from(text);
            window.text_system().shape_line(
                text.clone(),
                px(ui_font_size),
                &[TextRun {
                    len: text.len(),
                    font: ui_font.clone(),
                    color: theme.text_secondary,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
        };
        let number_font = editor_font(FontWeight(options.font_weight), false, &theme, &options);
        let shape_number = |text: String, color: Hsla, window: &mut Window| {
            let text = SharedString::from(text);
            window.text_system().shape_line(
                text.clone(),
                px(tm.font_size),
                &[TextRun {
                    len: text.len(),
                    font: number_font.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
        };
        let space_glyph = options.whitespace.then(|| {
            shape_glyph(
                "·",
                theme.whitespace_indicator,
                &theme,
                tm.font_size,
                &options,
                window,
            )
        });
        let tab_glyph = options.whitespace.then(|| {
            shape_glyph(
                "→",
                theme.whitespace_indicator,
                &theme,
                tm.font_size,
                &options,
                window,
            )
        });
        document.update(cx, |doc, _| {
            let line_count = doc.buffer.line_count();
            let first_display = (scroll_y / lh).floor().max(0.) as usize;
            let visible = (bounds.size.height / lh).ceil() as usize + 1;
            let last_display = (first_display + visible).min(total_rows);
            let (first_row, _) = map.at(first_display);
            let doc_folds: Vec<Fold> = doc.folds().to_vec();
            // Spans are sorted by start; walk them once for the visible window.
            let first_byte = doc.buffer.line_to_byte(first_row.min(line_count - 1));
            let spans = doc.spans().to_vec();
            let mut span_ix = spans.partition_point(|s| s.range.end <= first_byte);
            // The boxes: occurrences, the bracket pair and the find matches, as
            // char ranges in the whole buffer.
            let caret = doc.buffer.point_to_offset(cursor);
            let mut boxes: Vec<(Range<usize>, BoxKind)> = occurrences
                .iter()
                .map(|r| (r.clone(), BoxKind::Occurrence))
                .collect();
            if selections.len() == 1 && selections[0].0.is_empty() {
                let byte = doc.buffer.char_to_byte(caret);
                let pair = doc
                    .syntax()
                    .and_then(|t| t.matching_bracket(byte))
                    .map(|(o, c)| (doc.buffer.byte_to_char(o), doc.buffer.byte_to_char(c)))
                    .or_else(|| analysis::matching_bracket(&doc.buffer, caret));
                if let Some((open, close)) = pair {
                    boxes.push((open..open + 1, BoxKind::Bracket));
                    boxes.push((close..close + 1, BoxKind::Bracket));
                }
            }
            if let Some(search) = &search {
                for m in &search.matches {
                    boxes.push((m.clone(), BoxKind::SearchMatch));
                    let p = doc.buffer.offset_to_point(m.start);
                    mark_rows.push(map.display_row(p.row, p.column));
                }
            }
            mark_rows.sort_unstable();
            mark_rows.dedup();
            // The problems (M6, M7): a 2 px underline under each, one cell
            // wide when the parser's missing token has no width, and a mark
            // on the scrollbar; the server's diagnostics once it published,
            // else the parser's errors while `editor.markupWithoutSmartMode`.
            for p in doc.problems(options.markup).to_vec() {
                let range = if p.range.is_empty() {
                    p.range.start..p.range.start + 1
                } else {
                    p.range.clone()
                };
                boxes.push((range, BoxKind::Problem(p.severity)));
                let pt = doc.buffer.offset_to_point(p.range.start);
                problem_mark_rows.push((map.display_row(pt.row, pt.column), p.severity));
            }
            // The worst severity of a row marks it.
            problem_mark_rows.sort_unstable();
            problem_mark_rows.dedup_by_key(|(row, _)| *row);
            let current_block = analysis::current_block(&doc.buffer, cursor.row);
            let tab_width = doc.buffer.indent.width;
            let mut shaped_rows: Vec<(usize, LineSpans)> = Vec::new();
            let mut d = first_display;
            let mut row_cache: Option<(usize, String)> = None;
            while d < last_display {
                let (row, seg) = map.at(d);
                let source = match &row_cache {
                    Some((r, s)) if *r == row => s.clone(),
                    _ => {
                        let s = doc.buffer.line(row);
                        row_cache = Some((row, s.clone()));
                        s
                    }
                };
                let line_start = doc.buffer.line_to_byte(row);
                let line_end = line_start + source.len();
                let line_spans: Vec<(Range<usize>, &'static str)> = match shaped_rows.last() {
                    Some((r, s)) if *r == row => s.clone(),
                    _ => {
                        let mut line_spans = Vec::new();
                        let mut i = span_ix;
                        while i < spans.len() && spans[i].range.start < line_end {
                            let s = &spans[i];
                            if s.range.end > line_start {
                                let a = s.range.start.max(line_start) - line_start;
                                let b = s.range.end.min(line_end) - line_start;
                                if b > a {
                                    line_spans.push((a..b, s.attribute));
                                }
                            }
                            if s.range.end <= line_end {
                                span_ix = i + 1;
                            }
                            i += 1;
                        }
                        shaped_rows.push((row, line_spans.clone()));
                        line_spans
                    }
                };
                let cols = map.segment_range(&doc.buffer, row, seg);
                let fold = map.fold_at_head(row).cloned();
                let char_start = doc.buffer.line_to_char(row);
                // A folded head shows its text up to the fold, then the mark.
                let (seg_cols, folded) = match &fold {
                    Some(f) => {
                        let head_col = (f.inner.start - char_start).min(cols.end);
                        (cols.start..head_col, Some(f))
                    }
                    None => (cols.clone(), None),
                };
                let seg_source = &source
                    [byte_of_col(&source, seg_cols.start)..byte_of_col(&source, seg_cols.end)];
                let seg_byte_start = byte_of_col(&source, seg_cols.start);
                let seg_spans: Vec<(Range<usize>, &'static str)> = line_spans
                    .iter()
                    .filter(|(r, _)| {
                        r.start < seg_byte_start + seg_source.len() && r.end > seg_byte_start
                    })
                    .map(|(r, a)| {
                        (
                            r.start.max(seg_byte_start) - seg_byte_start
                                ..r.end.min(seg_byte_start + seg_source.len()) - seg_byte_start,
                            *a,
                        )
                    })
                    .collect();
                let line = DisplayLine::with_start_column(seg_source, seg_cols.start, tab_width);
                let shaped =
                    shape_display_line(&line, &seg_spans, &theme, tm.font_size, &options, window);
                let folded_layout = folded.map(|f| {
                    let mark = shape_glyph(
                        if tm.cell.is_some() { "⋯" } else { "…" },
                        theme.folded_mark_text,
                        &theme,
                        tm.font_size,
                        &options,
                        window,
                    );
                    let end_source = doc.buffer.line(f.end_row);
                    let end_line_start = doc.buffer.line_to_char(f.end_row);
                    let tail_col = f
                        .inner
                        .end
                        .saturating_sub(end_line_start)
                        .min(end_source.chars().count());
                    let tail_text = &end_source[byte_of_col(&end_source, tail_col)..];
                    let tail_line = DisplayLine::new(tail_text);
                    let tail =
                        shape_display_line(&tail_line, &[], &theme, tm.font_size, &options, window);
                    FoldedMarkLayout {
                        at: line.text.len(),
                        mark,
                        tail,
                    }
                });
                let run_index = if seg == 0 && options.run_in_gutter {
                    run_rows.iter().find(|(r, _)| *r == row).map(|(_, i)| *i)
                } else {
                    None
                };
                let show_number = seg == 0
                    && options.line_numbers != LineNumbers::Off
                    && (run_index.is_none() || options.run_after_numbers || tm.cell.is_some());
                let number = show_number.then(|| {
                    let color = if row == cursor.row {
                        theme.line_number_current
                    } else {
                        theme.line_number
                    };
                    let text = match options.line_numbers {
                        LineNumbers::Relative if row != cursor.row => {
                            row.abs_diff(cursor.row).to_string()
                        }
                        _ => (row + 1).to_string(),
                    };
                    shape_number(text, color, window)
                });
                let blame = blame_lines.as_ref().and_then(|lines| {
                    let line = lines.get(row)?;
                    let color = if line.uncommitted {
                        theme.git_added_bg
                    } else {
                        theme.git_blame_bg
                    };
                    // A chunk is a run of rows from one commit; its first row
                    // carries the label.
                    let head = seg == 0
                        && (row == 0
                            || lines.get(row - 1).is_none_or(|prev| {
                                prev.short_id != line.short_id
                                    || prev.uncommitted != line.uncommitted
                            }));
                    let label = match tm.cell {
                        // The tui column: `author 2d` in its cells, mono.
                        Some(_) if head && !line.uncommitted => {
                            let age = blame_days(&line.date)
                                .map(|d| short_age(today - d))
                                .unwrap_or_default();
                            let text = blame_cell_label(&line.author, &age, TUI_BLAME_COLS - 1);
                            Some(shape_number(text, theme.text_secondary, window))
                        }
                        Some(_) => None,
                        None if head => line.label().map(|text| shape_blame(text, window)),
                        None => None,
                    };
                    Some(BlameLayout { label, color })
                });
                let char_end = char_start + seg_cols.end;
                let seg_char_start = char_start + seg_cols.start;
                let last_seg = seg_cols.end == cols.end && fold.is_none();
                let row_boxes = boxes
                    .iter()
                    .filter(|(r, kind)| {
                        let at_end =
                            matches!(kind, BoxKind::Problem(_)) && last_seg && r.start == char_end;
                        (r.start < char_end || at_end) && r.end > seg_char_start
                    })
                    .map(|(r, kind)| {
                        let a = r.start.clamp(seg_char_start, char_end) - seg_char_start;
                        let b = r.end.min(char_end) - seg_char_start;
                        (
                            line.display_byte_for_char(a)..line.display_byte_for_char(b),
                            *kind,
                        )
                    })
                    .collect();
                let guides = if options.indent_guides && seg == 0 {
                    let indent = analysis::effective_indent(&doc.buffer, row);
                    (0..indent)
                        .step_by(tab_width.max(1))
                        .map(|col| {
                            let current = current_block
                                .as_ref()
                                .is_some_and(|(c, rows)| *c == col && rows.contains(&row));
                            (col, current)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let chevron = if options.foldings && seg == 0 {
                    if fold.is_some() {
                        Some(true)
                    } else if gutter_hovered && doc_folds.iter().any(|f| f.start_row == row) {
                        Some(false)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let spaces = if options.whitespace {
                    line.text
                        .char_indices()
                        .filter(|(b, c)| *c == ' ' && !line.tabs.iter().any(|t| t.contains(b)))
                        .map(|(b, _)| b)
                        .collect()
                } else {
                    Vec::new()
                };
                rows.push(LaidOutRow {
                    row,
                    cols: seg_cols,
                    line,
                    shaped,
                    number,
                    boxes: row_boxes,
                    guides,
                    chevron,
                    run_icon: run_index.is_some(),
                    folded: folded_layout,
                    spaces,
                    continuation: seg > 0,
                    blame,
                });
                d += 1;
            }
            // Sticky lines: the heads of the blocks enclosing the first visible row.
            if options.sticky_lines && first_display > 0 {
                let heads = enclosing_heads(&doc_folds, first_row);
                let heads: Vec<usize> = heads
                    .into_iter()
                    .filter(|h| !map.is_hidden(*h))
                    .take(options.sticky_max)
                    .collect();
                for head in heads {
                    let source = doc.buffer.line(head);
                    let line = DisplayLine::with_start_column(&source, 0, tab_width);
                    let line_start = doc.buffer.line_to_byte(head);
                    let line_end = line_start + source.len();
                    let head_spans: Vec<(Range<usize>, &'static str)> = spans
                        .iter()
                        .filter(|s| s.range.start < line_end && s.range.end > line_start)
                        .map(|s| {
                            (
                                s.range.start.max(line_start) - line_start
                                    ..s.range.end.min(line_end) - line_start,
                                s.attribute,
                            )
                        })
                        .collect();
                    let shaped = shape_display_line(
                        &line,
                        &head_spans,
                        &theme,
                        tm.font_size,
                        &options,
                        window,
                    );
                    let number = (options.line_numbers != LineNumbers::Off)
                        .then(|| shape_number((head + 1).to_string(), theme.line_number, window));
                    sticky.push(StickyLayout {
                        row: head,
                        shaped,
                        number,
                    });
                }
            }
        });
        let caret_mark_rows: Vec<usize> = selections
            .iter()
            .map(|(_, p)| map.display_row(p.row, p.column))
            .collect();
        self.editor.update(cx, |editor, _| {
            editor.sticky_rows = sticky.iter().map(|s| s.row).collect();
            if tm.cell.is_some() {
                // What the tui frame's right edge paints: the bar in rows and
                // the marks Fleet's scrollbar carries.
                let mut marks = Vec::new();
                if options.scrollbar_marks {
                    marks.extend(mark_rows.iter().map(|r| (*r, theme.search_result_mark)));
                    marks.extend(
                        caret_mark_rows
                            .iter()
                            .map(|r| (*r, theme.current_line_mark)),
                    );
                    marks.extend(problem_mark_rows.iter().map(|(r, severity)| {
                        let color = match severity {
                            Severity::Error => theme.problem_error_mark,
                            Severity::Warning => theme.problem_warning_mark,
                            Severity::Weak => theme.problem_weak_mark,
                            Severity::Info => theme.problem_info_underline,
                        };
                        (*r, color)
                    }));
                }
                editor.tui_bar = TuiBar {
                    offset: scroll_y / lh,
                    visible: bounds.size.height / lh,
                    total: total_rows as f32 + EDITOR_OVERSCROLL_LINES,
                    rows: total_rows,
                    marks,
                };
            }
        });
        Prepaint {
            hitbox,
            rows,
            scroll_y,
            scroll_x,
            selections,
            cursor,
            focused,
            caret_visible,
            total_rows,
            tm,
            options,
            space_width,
            mark_rows,
            caret_mark_rows,
            problem_mark_rows,
            text_offset,
            sticky,
            space_glyph,
            tab_glyph,
            hovered_row: hovered,
            gutter_hovered,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = cx.global::<Theme>().clone();
        let tm = prepaint.tm;
        let options = prepaint.options.clone();
        let lh = px(tm.line_height);
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);

        let origin_x = bounds.origin.x + prepaint.text_offset;
        let text_x = origin_x + px(tm.gutter_width) - prepaint.scroll_x;
        let number_right = origin_x + px(tm.gutter_number_right);
        let right = bounds.origin.x + bounds.size.width;
        let buffer_rows: Vec<(usize, usize)> = {
            let document = self.editor.read(cx).document.read(cx);
            prepaint
                .rows
                .iter()
                .map(|r| {
                    let start = document.buffer.line_to_char(r.row);
                    (start + r.cols.start, start + r.cols.end)
                })
                .collect()
        };
        let caret_rows: Vec<usize> = prepaint.selections.iter().map(|(_, p)| p.row).collect();
        let device_px = px(1. / window.scale_factor());
        let sticky_count = prepaint.sticky.len();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(cell) = tm.cell {
                paint_tui(
                    &self.editor,
                    cell,
                    bounds,
                    prepaint,
                    &buffer_rows,
                    window,
                    cx,
                );
                return;
            }
            // `editor.guides`: the rulers, behind everything.
            if options.show_guides {
                for col in &options.guides {
                    let x = text_x + prepaint.space_width * *col as f32;
                    window.paint_quad(fill(
                        Bounds::new(
                            point(x, bounds.origin.y),
                            size(device_px, bounds.size.height),
                        ),
                        theme.indent_guide,
                    ));
                }
            }
            let (hunks, status_popup, status_hovered) = {
                let e = self.editor.read(cx);
                (
                    e.document.read(cx).hunks.clone(),
                    e.status_popup,
                    e.status_hovered,
                )
            };
            let first_display = (prepaint.scroll_y / lh).floor().max(0.) as usize;
            for (display_row, (laid, (line_start, line_end))) in
                (first_display..).zip(prepaint.rows.iter().zip(buffer_rows))
            {
                let y = bounds.origin.y + lh * display_row as f32 - prepaint.scroll_y;
                let row_text_x = if laid.continuation {
                    text_x + px(tm.wrap_indent)
                } else {
                    text_x
                };
                // The current line's wash, across gutter and text alike, under
                // everything else on the row (it stays under a selection).
                if options.active_line && caret_rows.contains(&laid.row) {
                    let color = if prepaint.focused {
                        theme.current_line_focused
                    } else {
                        theme.current_line
                    };
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, lh)),
                        color,
                    ));
                }
                // The git line status (ninth pass): a bar on the island's edge for
                // each changed run, wider under the pointer and with its popup
                // open; the open hunk's rows wash across the text.
                for (ix, hunk) in hunks.iter().enumerate() {
                    if !hunk_covers(hunk, laid.row) {
                        continue;
                    }
                    if status_popup == Some(ix) {
                        let left =
                            bounds.origin.x + px(tm.line_status_x + tm.line_status_active_width);
                        window.paint_quad(fill(
                            Bounds::new(point(left, y), size(right - left, lh)),
                            theme.git_hunk_wash,
                        ));
                    }
                    let width = if status_popup == Some(ix) {
                        tm.line_status_active_width
                    } else if status_hovered == Some(ix) {
                        tm.line_status_hover_width
                    } else {
                        tm.line_status_width
                    };
                    let color = match hunk.kind {
                        weep_git::HunkKind::Added => theme.git_added_bg,
                        weep_git::HunkKind::Modified => theme.git_modified_bg,
                        weep_git::HunkKind::Deleted => theme.git_deleted_bg,
                    };
                    // A deletion has no rows: a short mark on the boundary (estimate).
                    let (top, height) = if hunk.rows == 0 {
                        (y - px(1.33), px(2.67))
                    } else {
                        (y, lh)
                    };
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.origin.x + px(tm.line_status_x), top),
                            size(px(width), height),
                        ),
                        color,
                    ));
                }
                // Occurrences, the bracket pair and the find matches.
                for (range, kind) in &laid.boxes {
                    let x0 = laid.shaped.x_for_index(range.start);
                    let x1 = laid.shaped.x_for_index(range.end);
                    if let BoxKind::Problem(severity) = kind {
                        let x1 = if x1 <= x0 {
                            x0 + prepaint.space_width
                        } else {
                            x1
                        };
                        let (color, thickness) = match severity {
                            Severity::Error => {
                                (theme.problem_error_underline, tm.problem_underline)
                            }
                            Severity::Warning => {
                                (theme.problem_warning_underline, tm.problem_underline)
                            }
                            Severity::Weak => (theme.problem_weak_underline, tm.problem_underline),
                            // `problem.info` is 1 px thick in Fleet's theme.
                            Severity::Info => {
                                (theme.problem_info_underline, tm.problem_underline / 2.)
                            }
                        };
                        let thickness = px(thickness);
                        let top = y + lh - thickness - px(tm.problem_underline_bottom);
                        window.paint_quad(fill(
                            Bounds::new(point(row_text_x + x0, top), size(x1 - x0, thickness)),
                            color,
                        ));
                        continue;
                    }
                    let (color, inset, radius) = match kind {
                        BoxKind::Occurrence => (
                            theme.occurrence_bg,
                            px(tm.occurrence_inset),
                            px(tm.occurrence_radius),
                        ),
                        BoxKind::Bracket => (theme.brace_match_bg, px(0.), px(0.)),
                        BoxKind::SearchMatch => (theme.search_result_bg, px(0.), px(0.)),
                        BoxKind::Problem(_) => unreachable!(),
                    };
                    window.paint_quad(
                        fill(
                            Bounds::new(
                                point(row_text_x + x0, y + inset),
                                size((x1 - x0).max(px(0.)), lh - inset * 2.),
                            ),
                            color,
                        )
                        .corner_radii(Corners::all(radius)),
                    );
                }
                // The selections on this row: each its own rounded rectangle
                // ending at the row's last character; an empty row shows a short stub.
                for (sel, _) in &prepaint.selections {
                    let sel = sel.range();
                    if sel.is_empty() || sel.start > line_end || sel.end < line_start {
                        continue;
                    }
                    let a = sel.start.max(line_start) - line_start;
                    let b = sel.end.min(line_end) - line_start;
                    let x0 = laid.shaped.x_for_index(laid.line.display_byte_for_char(a));
                    let mut x1 = laid.shaped.x_for_index(laid.line.display_byte_for_char(b));
                    if sel.end > line_end && laid.folded.is_none() {
                        // The selection runs past the line break: one character
                        // cell more, or a short stub on an empty row.
                        x1 += if line_end == line_start {
                            px(tm.selection_empty_line_width)
                        } else {
                            prepaint.space_width
                        };
                    }
                    let color = if prepaint.focused {
                        theme.selection_focused
                    } else {
                        theme.selection
                    };
                    if x1 > x0 {
                        window.paint_quad(
                            fill(
                                Bounds::new(point(row_text_x + x0, y), size(x1 - x0, lh)),
                                color,
                            )
                            .corner_radii(Corners::all(px(tm.selection_radius))),
                        );
                    }
                }
                // Indent guides: one device pixel at each tab stop before the
                // indent; the caret block's guide gets its colour painted over.
                for (col, current) in &laid.guides {
                    let x = text_x + prepaint.space_width * *col as f32;
                    let guide = Bounds::new(point(x, y), size(device_px, lh));
                    window.paint_quad(fill(guide, theme.indent_guide));
                    if *current {
                        window.paint_quad(fill(guide, theme.indent_guide_current));
                    }
                }
                // A block caret sits under the glyph.
                if prepaint.focused
                    && prepaint.caret_visible
                    && options.caret_shape == CaretShape::Block
                {
                    for (_, p) in &prepaint.selections {
                        let on_row = p.row == laid.row
                            && (laid.cols.contains(&p.column)
                                || (p.column == laid.cols.end
                                    && !next_segment_owns(prepaint, laid, p.column)));
                        if on_row {
                            let x = laid.shaped.x_for_index(
                                laid.line.display_byte_for_char(p.column - laid.cols.start),
                            );
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(row_text_x + x, y),
                                    size(prepaint.space_width, lh),
                                ),
                                theme.editor_caret.opacity(0.5),
                            ));
                        }
                    }
                }
                // The blame column (ninth pass): the chunk's bar on every row,
                // its date and author on the first.
                if let Some(blame) = &laid.blame {
                    let column_x = origin_x - px(tm.blame_width);
                    window.paint_quad(fill(
                        Bounds::new(
                            point(column_x + px(tm.blame_bar_x), y),
                            size(px(tm.blame_bar_width), lh),
                        ),
                        blame.color,
                    ));
                    if let Some(label) = &blame.label {
                        let _ = label.paint(
                            point(column_x + px(tm.blame_text_x), y),
                            lh,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
                // Line number, right-aligned in the gutter.
                if let Some(number) = &laid.number {
                    let _ = number.paint(
                        point(number_right - number.width, y),
                        lh,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                // The run icon (M6, batch 11): `icon.background.green` in a 16 px
                // box in place of the number, or after it (`ff.runIconsAfterLineNumbers`).
                if laid.run_icon {
                    let x = if options.run_after_numbers {
                        number_right + px(4.)
                    } else {
                        origin_x + px(tm.run_icon_x)
                    };
                    let icon_bounds = Bounds::new(
                        point(x, y + (lh - px(tm.icon_size)) / 2.),
                        size(px(tm.icon_size), px(tm.icon_size)),
                    );
                    let _ = window.paint_svg(
                        icon_bounds,
                        "icons/weep/run-gutter.svg".into(),
                        None,
                        gpui::TransformationMatrix::unit(),
                        theme.icon_green,
                        cx,
                    );
                }
                // The hovered row shows the breakpoint hint: a filled circle at
                // 21.3 (batch 5; the `fold` scene showed no wash under the chevron).
                if prepaint.gutter_hovered && prepaint.hovered_row == Some(laid.row) {
                    let d = px(tm.breakpoint_hint_size);
                    let hint = Bounds::new(
                        point(origin_x + px(tm.breakpoint_hint_x), y + (lh - d) / 2.),
                        size(d, d),
                    );
                    window.paint_quad(
                        fill(hint, theme.breakpoint_hint).corner_radii(Corners::all(d / 2.)),
                    );
                }
                // The fold chevron in its 16 px box (batch 5).
                if let Some(folded) = laid.chevron {
                    let box_origin = point(
                        origin_x + px(tm.fold_chevron_x),
                        y + (lh - px(tm.icon_size)) / 2.,
                    );
                    let chevron_bounds =
                        Bounds::new(box_origin, size(px(tm.icon_size), px(tm.icon_size)));
                    let path: SharedString = if folded {
                        "icons/expui/general/chevronRight.svg".into()
                    } else {
                        "icons/expui/general/chevronDown.svg".into()
                    };
                    let _ = window.paint_svg(
                        chevron_bounds,
                        path,
                        None,
                        gpui::TransformationMatrix::unit(),
                        theme.fold_icon,
                        cx,
                    );
                }
                // Whitespace glyphs under the text.
                if let Some(dot) = &prepaint.space_glyph {
                    for b in &laid.spaces {
                        let x = laid.shaped.x_for_index(*b);
                        let _ = dot.paint(
                            point(row_text_x + x, y),
                            lh,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
                if let Some(arrow) = &prepaint.tab_glyph {
                    for t in &laid.line.tabs {
                        let x = laid.shaped.x_for_index(t.start);
                        let _ = arrow.paint(
                            point(row_text_x + x, y),
                            lh,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
                let _ =
                    laid.shaped
                        .paint(point(row_text_x, y), lh, TextAlign::Left, None, window, cx);
                // The folded mark and the block's last row after it.
                if let Some(folded) = &laid.folded {
                    let x = row_text_x + laid.shaped.x_for_index(folded.at);
                    let w = prepaint.space_width * tm.folded_mark_cells;
                    window.paint_quad(
                        fill(Bounds::new(point(x, y), size(w, lh)), theme.folded_mark_bg)
                            .corner_radii(Corners::all(px(tm.folded_mark_radius))),
                    );
                    let _ = folded.mark.paint(
                        point(x + (w - folded.mark.width) / 2., y),
                        lh,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    let _ =
                        folded
                            .tail
                            .paint(point(x + w, y), lh, TextAlign::Left, None, window, cx);
                }
                // The carets.
                if prepaint.focused && prepaint.caret_visible {
                    for (_, p) in &prepaint.selections {
                        let on_row = p.row == laid.row
                            && (laid.cols.contains(&p.column)
                                || (p.column == laid.cols.end
                                    && !next_segment_owns(prepaint, laid, p.column)));
                        if !on_row {
                            continue;
                        }
                        let x = laid.shaped.x_for_index(
                            laid.line.display_byte_for_char(p.column - laid.cols.start),
                        );
                        match options.caret_shape {
                            CaretShape::Line => window.paint_quad(fill(
                                Bounds::new(point(row_text_x + x, y), size(px(tm.caret_width), lh)),
                                theme.editor_caret,
                            )),
                            CaretShape::Underline => window.paint_quad(fill(
                                Bounds::new(
                                    point(row_text_x + x, y + lh - px(tm.caret_width)),
                                    size(prepaint.space_width, px(tm.caret_width)),
                                ),
                                theme.editor_caret,
                            )),
                            CaretShape::Block => {}
                        }
                    }
                }
            }
            // Sticky lines over the top rows: the island's background, the row,
            // a 1 device px border under the last and a shadow fading below.
            if sticky_count > 0 {
                let height = lh * sticky_count as f32;
                window.paint_quad(fill(
                    Bounds::new(bounds.origin, size(bounds.size.width, height)),
                    theme.island_background,
                ));
                for (i, s) in prepaint.sticky.iter().enumerate() {
                    let y = bounds.origin.y + lh * i as f32;
                    if let Some(number) = &s.number {
                        let _ = number.paint(
                            point(number_right - number.width, y),
                            lh,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                    let _ = s
                        .shaped
                        .paint(point(text_x, y), lh, TextAlign::Left, None, window, cx);
                }
                let line_y = bounds.origin.y + height;
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.origin.x, line_y),
                        size(bounds.size.width, device_px),
                    ),
                    theme.border,
                ));
                let steps = (tm.sticky_shadow / f32::from(device_px)).round().max(1.) as usize;
                for i in 0..steps {
                    let alpha = 0.25 * (1. - i as f32 / steps as f32);
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.origin.x, line_y + device_px * (i as f32 + 1.)),
                            size(bounds.size.width, device_px),
                        ),
                        theme.shadow_medium.opacity(alpha),
                    ));
                }
            }
            // The scrollbar: a thumb proportional to the view, no track. Fleet's thumb
            // starts at the text area's top and sits flush with the island's right edge;
            // the content it measures includes the overscroll past the last line.
            let total = lh * (prepaint.total_rows as f32 + EDITOR_OVERSCROLL_LINES);
            let track_h = bounds.size.height;
            if total > track_h {
                let thumb_h = (track_h * (track_h / total)).max(px(tm.scrollbar_thumb_min));
                let max_scroll = total - track_h;
                let thumb_y =
                    bounds.origin.y + (track_h - thumb_h) * (prepaint.scroll_y / max_scroll);
                let x = right - px(tm.scrollbar_thumb);
                window.paint_quad(
                    fill(
                        Bounds::new(point(x, thumb_y), size(px(tm.scrollbar_thumb), thumb_h)),
                        theme.scrollbar_thumb,
                    )
                    .corner_radii(Corners::all(px(tm.scrollbar_thumb / 2.))),
                );
            }
            // Marks on the scrollbar: a row's place is its top scaled by track over
            // content (batch 5: line 7 at 178 in a 696 px track over 41 + 4 lines).
            if options.scrollbar_marks {
                let mark = |window: &mut Window, row: usize, color: Hsla| {
                    let scale = if total > track_h { track_h / total } else { 1. };
                    let y = bounds.origin.y + lh * row as f32 * scale;
                    window.paint_quad(fill(
                        Bounds::new(
                            point(right - px(tm.scrollbar_mark_width), y),
                            size(px(tm.scrollbar_mark_width), px(tm.scrollbar_mark_height)),
                        ),
                        color,
                    ));
                };
                for row in &prepaint.mark_rows {
                    mark(window, *row, theme.search_result_mark);
                }
                for row in &prepaint.caret_mark_rows {
                    mark(window, *row, theme.current_line_mark);
                }
                for (row, severity) in &prepaint.problem_mark_rows {
                    let color = match severity {
                        Severity::Error => theme.problem_error_mark,
                        Severity::Warning => theme.problem_warning_mark,
                        Severity::Weak => theme.problem_weak_mark,
                        Severity::Info => theme.problem_info_underline,
                    };
                    mark(window, *row, color);
                }
            }
        });

        let _ = prepaint.cursor;
        let hitbox = prepaint.hitbox.clone();
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && hitbox.is_hovered(window)
                && event.button == MouseButton::Left
            {
                editor.update(cx, |editor, cx| editor.mouse_down(event, window, cx));
            } else if phase == DispatchPhase::Bubble
                && hitbox.is_hovered(window)
                && event.button == MouseButton::Right
            {
                editor.update(cx, |editor, cx| editor.right_click(event, window, cx));
            }
        });
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if event.pressed_button == Some(MouseButton::Left) {
                editor.update(cx, |editor, cx| editor.mouse_drag(event, window, cx));
            } else {
                editor.update(cx, |editor, cx| editor.mouse_hover(event.position, cx));
            }
        });
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                editor.update(cx, |editor, cx| editor.mouse_up(event, cx));
            }
        });
        let hitbox = prepaint.hitbox.clone();
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                editor.update(cx, |editor, cx| editor.scroll(event, cx));
            }
        });
    }
}

/// Fills whole cells from `x0` to `x1`, `h` high from `y`, with both edges
/// on device pixels so neighbouring rows meet without a seam.
fn fill_cells(window: &mut Window, x0: Pixels, x1: Pixels, y: Pixels, h: Pixels, color: Hsla) {
    let sn = |window: &Window, v: Pixels| px(skin::snap(window, f32::from(v)));
    let (a, b) = (sn(window, x0), sn(window, x1));
    let (top, bottom) = (sn(window, y), sn(window, y + h));
    if b > a && bottom > top {
        window.paint_quad(fill(
            Bounds::from_corners(point(a, top), point(b, bottom)),
            color,
        ));
    }
}

/// A problem's underline in the tui skin: a stroke along the bottom of its
/// cells, straight for errors, dashed for warnings (a cell on, half off)
/// and dotted for weak warnings and infos.
#[allow(clippy::too_many_arguments)]
fn paint_underline(
    window: &mut Window,
    x0: Pixels,
    x1: Pixels,
    y: Pixels,
    cell: Cell,
    t: Pixels,
    severity: Severity,
    color: Hsla,
) {
    let (on, period) = match severity {
        Severity::Error => (x1 - x0, x1 - x0),
        Severity::Warning => (px(cell.w), px(cell.w * 1.5)),
        Severity::Weak | Severity::Info => (t * 2., t * 4.),
    };
    if period <= px(0.) {
        return;
    }
    let mut x = x0;
    while x < x1 {
        fill_cells(window, x, (x + on).min(x1), y, t, color);
        x += period;
    }
}

/// The tui skin's paint: the Fleet branch's layers on the cell grid. The
/// gutter is whole cells (`▶`/`○`, the numbers, the line-status cell, `▾ ▸`
/// or `↪`), every wash and box fills whole cells without radius or inset,
/// the guides run through cell centres, the carets are drawn on cells, and
/// the sticky lines end in a rule instead of a shadow. The scroll bar and
/// its marks are the frame's right edge (`EditorView::tui_edges`).
fn paint_tui(
    editor: &Entity<EditorView>,
    cell: Cell,
    bounds: Bounds<Pixels>,
    prepaint: &Prepaint,
    buffer_rows: &[(usize, usize)],
    window: &mut Window,
    cx: &mut App,
) {
    let theme = cx.global::<Theme>().clone();
    let m = *cx.global::<Metrics>();
    let tm = prepaint.tm;
    let options = &prepaint.options;
    let lh = px(cell.h);
    let w = px(cell.w);
    let t = px(skin::stroke(window, &m));
    let sn = |window: &Window, v: Pixels| px(skin::snap(window, f32::from(v)));
    let origin_x = bounds.origin.x + prepaint.text_offset;
    let text_left = origin_x + px(tm.gutter_width);
    let text_x = text_left - prepaint.scroll_x;
    let right = bounds.origin.x + bounds.size.width;
    let number_right = origin_x + px(tm.gutter_number_right);
    let status_x = origin_x + px(tm.line_status_x);
    let fold_x = origin_x + px(tm.fold_chevron_x);
    // Text scrolled sideways never runs under the gutter.
    let text_mask = ContentMask {
        bounds: Bounds::from_corners(point(text_left, bounds.origin.y), bounds.bottom_right()),
    };
    let glyph = |text: &str, color: Hsla, window: &mut Window| {
        shape_glyph(text, color, &theme, cell.font_size, options, window)
    };
    let run_glyph = glyph("▶", theme.icon_green, window);
    let hint_glyph = glyph("○", theme.breakpoint_hint, window);
    let open_glyph = glyph("▾", theme.fold_icon, window);
    let folded_glyph = glyph("▸", theme.fold_icon, window);
    let wrap_glyph = glyph("↪", theme.text_tertiary, window);
    let in_cell = |g: &ShapedLine, x: Pixels| x + ((w - g.width) / 2.).max(px(0.));
    let (hunks, status_popup, status_hovered, commit_rows) = {
        let e = editor.read(cx);
        let hovered = e
            .blame
            .as_ref()
            .filter(|_| e.blame_hovered)
            .and_then(|lines| lines.get(e.hovered_row?))
            .filter(|l| !l.uncommitted)
            .map(|l| l.short_id.clone());
        let commit_rows: Vec<bool> = prepaint
            .rows
            .iter()
            .map(|r| {
                hovered.as_ref().is_some_and(|id| {
                    e.blame
                        .as_ref()
                        .and_then(|lines| lines.get(r.row))
                        .is_some_and(|l| l.short_id == *id)
                })
            })
            .collect();
        (
            e.document.read(cx).hunks.clone(),
            e.status_popup,
            e.status_hovered,
            commit_rows,
        )
    };
    let caret_rows: Vec<usize> = prepaint.selections.iter().map(|(_, p)| p.row).collect();
    let caret_on = |laid: &LaidOutRow, column: usize, row: usize| {
        row == laid.row
            && (laid.cols.contains(&column)
                || (column == laid.cols.end && !next_segment_owns(prepaint, laid, column)))
    };
    let show_caret = prepaint.focused && prepaint.caret_visible;
    // `editor.guides`: the rulers on their columns' left edges.
    if options.show_guides {
        window.with_content_mask(Some(text_mask), |window| {
            for col in &options.guides {
                let x = sn(window, text_x + w * *col as f32);
                fill_cells(
                    window,
                    x,
                    x + t,
                    bounds.origin.y,
                    bounds.size.height,
                    theme.indent_guide,
                );
            }
        });
    }
    let first_display = (prepaint.scroll_y / lh).floor().max(0.) as usize;
    for (i, (laid, (line_start, line_end))) in prepaint
        .rows
        .iter()
        .zip(buffer_rows.iter().copied())
        .enumerate()
    {
        let y = bounds.origin.y + lh * (first_display + i) as f32 - prepaint.scroll_y;
        let row_text_x = if laid.continuation {
            text_x + px(tm.wrap_indent)
        } else {
            text_x
        };
        // The current line: the row's whole cells, gutter included.
        if options.active_line && caret_rows.contains(&laid.row) {
            let color = if prepaint.focused {
                theme.current_line_focused
            } else {
                theme.current_line
            };
            fill_cells(window, bounds.origin.x, right, y, lh, color);
        }
        // The open hunk's rows wash across the text.
        if let Some(hunk) = status_popup.and_then(|ix| hunks.get(ix)) {
            if hunk.rows > 0 && hunk_covers(hunk, laid.row) {
                fill_cells(window, text_left, right, y, lh, theme.git_hunk_wash);
            }
        }
        window.with_content_mask(Some(text_mask), |window| {
            // Occurrences, the bracket pair and the find matches.
            for (range, kind) in &laid.boxes {
                let color = match kind {
                    BoxKind::Occurrence => theme.occurrence_bg,
                    BoxKind::Bracket => theme.brace_match_bg,
                    BoxKind::SearchMatch => theme.search_result_bg,
                    BoxKind::Problem(_) => continue,
                };
                let x0 = laid.shaped.x_for_index(range.start);
                let x1 = laid.shaped.x_for_index(range.end);
                fill_cells(window, row_text_x + x0, row_text_x + x1, y, lh, color);
            }
            // The selections: whole cells to the last char, one cell more
            // when the line break is selected (the only cell of an empty row).
            for (sel, _) in &prepaint.selections {
                let sel = sel.range();
                if sel.is_empty() || sel.start > line_end || sel.end < line_start {
                    continue;
                }
                let a = sel.start.max(line_start) - line_start;
                let b = sel.end.min(line_end) - line_start;
                let x0 = laid.shaped.x_for_index(laid.line.display_byte_for_char(a));
                let mut x1 = laid.shaped.x_for_index(laid.line.display_byte_for_char(b));
                if sel.end > line_end && laid.folded.is_none() {
                    x1 += w;
                }
                let color = if prepaint.focused {
                    theme.selection_focused
                } else {
                    theme.selection
                };
                fill_cells(window, row_text_x + x0, row_text_x + x1, y, lh, color);
            }
            // Indent guides through the centre of each level's first cell.
            for (col, current) in &laid.guides {
                let x = sn(window, text_x + w * *col as f32 + w / 2.) - t / 2.;
                let color = if *current {
                    theme.indent_guide_current
                } else {
                    theme.indent_guide
                };
                fill_cells(window, x, x + t, y, lh, color);
            }
        });
        // The blame column: the hovered commit's rows filled, the label on
        // each chunk's first row.
        if let Some(blame) = &laid.blame {
            let column_x = origin_x - px(tm.blame_width);
            if commit_rows.get(i).copied().unwrap_or(false) {
                fill_cells(window, column_x, origin_x - w, y, lh, theme.git_blame_bg);
            }
            if let Some(label) = &blame.label {
                let _ = label.paint(point(column_x, y), lh, TextAlign::Left, None, window, cx);
            }
        }
        if let Some(number) = &laid.number {
            let _ = number.paint(
                point(number_right - number.width, y),
                lh,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        // The first cell: the run icon, else the breakpoint hint on hover.
        if laid.run_icon {
            let _ = run_glyph.paint(
                point(in_cell(&run_glyph, origin_x), y),
                lh,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        } else if !laid.continuation
            && prepaint.gutter_hovered
            && prepaint.hovered_row == Some(laid.row)
        {
            let _ = hint_glyph.paint(
                point(in_cell(&hint_glyph, origin_x), y),
                lh,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        // The line-status cell: a third of it for added and modified rows
        // (all of it under the pointer or with its popup open), a wedge on
        // the bottom edge where lines were deleted.
        for (ix, hunk) in hunks.iter().enumerate() {
            let color = match hunk.kind {
                weep_git::HunkKind::Added => theme.git_added_bg,
                weep_git::HunkKind::Modified => theme.git_modified_bg,
                weep_git::HunkKind::Deleted => theme.git_deleted_bg,
            };
            let wide = status_popup == Some(ix) || status_hovered == Some(ix);
            if hunk.rows > 0 && hunk_covers(hunk, laid.row) {
                let width = if wide { w } else { w / 3. };
                fill_cells(window, status_x, status_x + width, y, lh, color);
            } else if tui_deletion_on(hunk, laid.row) && !laid.continuation {
                let h = sn(window, if wide { lh / 4. } else { lh / 8. }).max(t * 2.);
                let top = if hunk.row == 0 { y } else { y + lh - h };
                fill_cells(window, status_x, status_x + w, top, h, color);
            }
        }
        // The fold cell: `▾` open (on hover), `▸` folded; `↪` on a wrapped
        // line's continuation rows.
        let fold_mark = match laid.chevron {
            Some(true) => Some(&folded_glyph),
            Some(false) => Some(&open_glyph),
            None if laid.continuation => Some(&wrap_glyph),
            None => None,
        };
        if let Some(g) = fold_mark {
            let _ = g.paint(
                point(in_cell(g, fold_x), y),
                lh,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        window.with_content_mask(Some(text_mask), |window| {
            if let Some(dot) = &prepaint.space_glyph {
                for b in &laid.spaces {
                    let x = laid.shaped.x_for_index(*b);
                    let _ = dot.paint(
                        point(row_text_x + x, y),
                        lh,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
            if let Some(arrow) = &prepaint.tab_glyph {
                for tab in &laid.line.tabs {
                    let x = laid.shaped.x_for_index(tab.start);
                    let _ = arrow.paint(
                        point(row_text_x + x, y),
                        lh,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
            let _ = laid
                .shaped
                .paint(point(row_text_x, y), lh, TextAlign::Left, None, window, cx);
            // The folded block: `⋯` in one filled cell, then the block's tail.
            if let Some(folded) = &laid.folded {
                let x = row_text_x + laid.shaped.x_for_index(folded.at);
                fill_cells(window, x, x + w, y, lh, theme.folded_mark_bg);
                let _ = folded.mark.paint(
                    point(in_cell(&folded.mark, x), y),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                let _ = folded
                    .tail
                    .paint(point(x + w, y), lh, TextAlign::Left, None, window, cx);
            }
            // Problems: a stroke along the bottom of their cells.
            for (range, kind) in &laid.boxes {
                let BoxKind::Problem(severity) = kind else {
                    continue;
                };
                let x0 = laid.shaped.x_for_index(range.start);
                let x1 = laid.shaped.x_for_index(range.end);
                let x1 = if x1 <= x0 { x0 + w } else { x1 };
                let color = match severity {
                    Severity::Error => theme.problem_error_underline,
                    Severity::Warning => theme.problem_warning_underline,
                    Severity::Weak => theme.problem_weak_underline,
                    Severity::Info => theme.problem_info_underline,
                };
                let line_y = sn(window, y + lh) - t;
                paint_underline(
                    window,
                    row_text_x + x0,
                    row_text_x + x1,
                    line_y,
                    cell,
                    t,
                    *severity,
                    color,
                );
            }
            // The carets, on cells: a bar of two strokes at the cell's left
            // edge, the cell filled with its char redrawn in the background
            // colour, or the cell's bottom fifth.
            if !show_caret {
                return;
            }
            for (_, p) in &prepaint.selections {
                if !caret_on(laid, p.column, p.row) {
                    continue;
                }
                let byte = laid.line.display_byte_for_char(p.column - laid.cols.start);
                let x = row_text_x + laid.shaped.x_for_index(byte);
                match options.caret_shape {
                    CaretShape::Line => {
                        let x = sn(window, x);
                        fill_cells(window, x, x + t * 2., y, lh, theme.editor_caret);
                    }
                    CaretShape::Underline => {
                        let h = sn(window, lh / 5.).max(t);
                        fill_cells(window, x, x + w, y + lh - h, h, theme.editor_caret);
                    }
                    CaretShape::Block => {
                        fill_cells(window, x, x + w, y, lh, theme.editor_caret);
                        let ch = laid.line.text[byte.min(laid.line.text.len())..]
                            .chars()
                            .next()
                            .filter(|c| !c.is_whitespace() && laid.folded.is_none());
                        if let Some(ch) = ch {
                            let g = shape_glyph(
                                &ch.to_string(),
                                theme.island_background,
                                &theme,
                                cell.font_size,
                                options,
                                window,
                            );
                            let _ = g.paint(point(x, y), lh, TextAlign::Left, None, window, cx);
                        }
                    }
                }
            }
        });
    }
    // Sticky lines: the top rows on the editor's background, a rule along
    // the last one's bottom edge, no shadow.
    let sticky = prepaint.sticky.len();
    if sticky > 0 {
        let height = lh * sticky as f32;
        fill_cells(
            window,
            bounds.origin.x,
            right,
            bounds.origin.y,
            height,
            theme.island_background,
        );
        for (i, s) in prepaint.sticky.iter().enumerate() {
            let y = bounds.origin.y + lh * i as f32;
            if let Some(number) = &s.number {
                let _ = number.paint(
                    point(number_right - number.width, y),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            window.with_content_mask(Some(text_mask), |window| {
                let _ = s
                    .shaped
                    .paint(point(text_x, y), lh, TextAlign::Left, None, window, cx);
            });
        }
        let line_y = sn(window, bounds.origin.y + height) - t;
        fill_cells(window, bounds.origin.x, right, line_y, t, theme.border);
    }
}

/// A caret at the boundary between two wrapped segments belongs to the
/// later one (the start of the next display row), as Fleet shows it.
fn next_segment_owns(prepaint: &Prepaint, laid: &LaidOutRow, column: usize) -> bool {
    prepaint
        .rows
        .iter()
        .any(|r| r.row == laid.row && r.continuation && r.cols.start == column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_gutter_digits() {
        assert_eq!(tui_digits(0, true), 3);
        assert_eq!(tui_digits(99, true), 3);
        assert_eq!(tui_digits(999, true), 3);
        assert_eq!(tui_digits(1000, true), 4);
        assert_eq!(tui_digits(123_456, true), 6);
        assert_eq!(tui_digits(5000, false), 0);
    }

    #[test]
    fn snapping_lands_on_whole_steps() {
        assert_eq!(snap_to(0., 20.), 0.);
        assert_eq!(snap_to(29., 20.), 20.);
        assert_eq!(snap_to(31., 20.), 40.);
        assert_eq!(snap_to(7., 0.), 7.);
    }

    #[test]
    fn wheel_deltas_gather_into_rows() {
        // A trackpad's small deltas make a row once they add up to one.
        let (steps, rest) = whole_steps(px(0.), px(8.), 20.);
        assert_eq!((steps, rest), (0., px(8.)));
        let (steps, rest) = whole_steps(rest, px(15.), 20.);
        assert_eq!((steps, rest), (1., px(3.)));
        // Backwards drops what was gathered the other way.
        let (steps, rest) = whole_steps(px(15.), px(-45.), 20.);
        assert_eq!((steps, rest), (-2., px(-5.)));
        // A mouse wheel's lines are whole rows already.
        assert_eq!(whole_steps(px(0.), px(-60.), 20.), (-3., px(0.)));
    }

    #[test]
    fn blame_labels_fit_their_cells() {
        assert_eq!(blame_cell_label("ragtux", "2d", 12), "ragtux    2d");
        assert_eq!(
            blame_cell_label("Somebody Longname", "3mo", 12),
            "Somebod… 3mo"
        );
        assert_eq!(blame_cell_label("", "1y", 4), "  1y");
        for label in [
            blame_cell_label("a", "5w", 17),
            blame_cell_label("A Very Long Author Name Indeed", "10y", 17),
        ] {
            assert_eq!(label.chars().count(), 17);
        }
    }

    #[test]
    fn blame_ages() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(
            blame_days("2025-12-31"),
            Some(days_from_civil(2025, 12, 31))
        );
        assert_eq!(blame_days("86400"), Some(1));
        assert_eq!(blame_days("soon"), None);
        assert_eq!(short_age(0), "0d");
        assert_eq!(short_age(13), "13d");
        assert_eq!(short_age(21), "3w");
        assert_eq!(short_age(90), "3mo");
        assert_eq!(short_age(800), "2y");
    }

    #[test]
    fn deletions_mark_the_row_above() {
        let deletion = |row| weep_git::Hunk {
            row,
            rows: 0,
            kind: weep_git::HunkKind::Deleted,
            old: vec!["gone".into()],
        };
        assert!(tui_deletion_on(&deletion(5), 4));
        assert!(!tui_deletion_on(&deletion(5), 5));
        assert!(tui_deletion_on(&deletion(0), 0));
    }
}
