//! Quick evaluate in the editor (M8): `debug/quick-evaluate-expression`
//! evaluates the selection, else the expression at the caret, in the
//! active debug session's selected frame, and shows the answer in a popup
//! at the expression the way the documentation popup sits at its symbol
//! (`editor/smart.rs`): `name = value` with the type dimmed, children
//! unfolded through the session (`DebugSession::toggle_expanded`), an "Add
//! to Watches" link; Escape closes it. While the debuggee is stopped, the
//! pointer resting on an identifier for `editor.hoverTooltipDelayMs` opens
//! the same popup (`debug_panel.rs` tracks the pointer).
//!
//! The popup is a [`QuickEvaluate`] entity hosted by the floating debug
//! panel, which the workspace draws over the whole window; it reads the
//! view's geometry through the `eval_*` methods here, so the editor itself
//! carries no field for it. What can be told without gpui is in plain
//! functions: [`expression_at`] and [`eval_rows`].
//!
//! Batch 13 has not run, so every size is an estimate (`theme.rs`,
//! `QUICK_EVAL_*`, and the documentation popup's `SMART_POPUP_*`); the
//! colours are Fleet's keys: `popup.*` for the frame, `text.*` for the
//! row, `link.text.default` for the link.

use super::*;
use crate::debugger::{DebugSession, EvalContext, SessionState, Variable};
use gpui::{AnyElement, FocusHandle, Focusable, Subscription, WeakEntity};
use std::collections::{HashMap, HashSet};

// ---- the expression at the caret ---------------------------------------------------

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Words that are no expression on their own.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while",
];

/// The expression at char column `col` of `line` (the caret before or just
/// after an identifier), as a char range: the identifier with the
/// qualifiers before it joined by `.`, `::` or `->` (`self.name`,
/// `std::f64::consts::PI`, `p->next`, `pair.0`). Nothing for a number, a
/// keyword, a lifetime, a call or a macro (`len(`, `println!`), and nothing
/// when a qualifier hangs off a call or an index (`f().x`): evaluating
/// those could run code, and the tail alone names something else.
pub fn expression_at(line: &str, col: usize) -> Option<Range<usize>> {
    let chars: Vec<char> = line.chars().collect();
    let at = if chars.get(col).is_some_and(|c| is_ident(*c)) {
        col
    } else if col > 0 && chars.get(col - 1).is_some_and(|c| is_ident(*c)) {
        col - 1
    } else {
        return None;
    };
    let mut start = at;
    while start > 0 && is_ident(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && is_ident(chars[end]) {
        end += 1;
    }
    if start > 0 && chars[start - 1] == '\'' {
        return None;
    }
    if matches!(chars.get(end), Some('(') | Some('!')) {
        return None;
    }
    let mut segments = 1;
    loop {
        let sep =
            if start >= 1 && chars[start - 1] == '.' && !(start >= 2 && chars[start - 2] == '.') {
                1
            } else if start >= 2 && matches!(chars[start - 2..start], [':', ':'] | ['-', '>']) {
                2
            } else {
                break;
            };
        let e = start - sep;
        let mut s = e;
        while s > 0 && is_ident(chars[s - 1]) {
            s -= 1;
        }
        if s == e {
            // `f().x`, `v[0].x`, `"a".len`: rooted in something not a name.
            return None;
        }
        start = s;
        segments += 1;
    }
    if chars[start].is_ascii_digit() {
        return None;
    }
    if segments == 1 {
        let word: String = chars[start..end].iter().collect();
        if KEYWORDS.contains(&word.as_str()) {
            return None;
        }
    }
    Some(start..end)
}

/// The text quick evaluate sends for a selection: trimmed, `None` when
/// only whitespace is selected.
pub fn selection_expression(selected: &str) -> Option<String> {
    let t = selected.trim();
    (!t.is_empty()).then(|| t.to_string())
}

// ---- the value tree ----------------------------------------------------------------

/// A row of the popup's value tree.
#[derive(Clone, Debug, PartialEq)]
pub enum EvalRow {
    Value {
        depth: usize,
        var: Variable,
        expanded: bool,
    },
    /// An expanded value whose children the adapter has not sent yet.
    Loading { depth: usize },
}

/// The rows the tree shows for `root`: the value, then the children of
/// each expanded reference (from `children`, as fetched by the session),
/// depth first. A reference already open further up is not opened again,
/// so a cyclic structure stops at its repeat.
pub fn eval_rows(
    root: &Variable,
    children: &HashMap<i64, Vec<Variable>>,
    expanded: &HashSet<i64>,
) -> Vec<EvalRow> {
    fn walk(
        var: &Variable,
        depth: usize,
        children: &HashMap<i64, Vec<Variable>>,
        expanded: &HashSet<i64>,
        path: &mut Vec<i64>,
        out: &mut Vec<EvalRow>,
    ) {
        let r = var.variables_reference;
        let open = r != 0 && expanded.contains(&r) && !path.contains(&r);
        out.push(EvalRow::Value {
            depth,
            var: var.clone(),
            expanded: open,
        });
        if !open {
            return;
        }
        match children.get(&r) {
            Some(kids) => {
                path.push(r);
                for kid in kids {
                    walk(kid, depth + 1, children, expanded, path, out);
                }
                path.pop();
            }
            None => out.push(EvalRow::Loading { depth: depth + 1 }),
        }
    }
    let mut out = Vec::new();
    walk(root, 0, children, expanded, &mut Vec::new(), &mut out);
    out
}

// ---- what the popup reads off the view ---------------------------------------------

impl EditorView {
    /// The selection, else the expression at the caret: its text and its
    /// char range.
    pub fn eval_expression_at_caret(&mut self, cx: &App) -> Option<(String, Range<usize>)> {
        let range = self.selection_range(cx);
        if !range.is_empty() {
            let text = self.selected_text(cx);
            return selection_expression(&text).map(|t| (t, range));
        }
        let caret = self.caret(cx);
        self.eval_expression_at(caret, cx)
    }

    /// The expression at a char offset (the pointer's), with its range.
    pub fn eval_expression_at(&self, offset: usize, cx: &App) -> Option<(String, Range<usize>)> {
        let buffer = &self.document.read(cx).buffer;
        let offset = offset.min(buffer.len_chars());
        let at = buffer.offset_to_point(offset);
        let line = buffer.line(at.row);
        let cols = expression_at(&line, at.column)?;
        let text: String = line
            .chars()
            .skip(cols.start)
            .take(cols.end - cols.start)
            .collect();
        let base = buffer.line_to_char(at.row);
        Some((text, base + cols.start..base + cols.end))
    }

    /// The char under a window position, `None` outside the text (as the
    /// documentation popup's pointer finds it: the row, then a monospace
    /// column).
    pub fn eval_offset_at_point(&self, position: Point<Pixels>, cx: &App) -> Option<usize> {
        let bounds = self.last_bounds;
        if !bounds.contains(&position) {
            return None;
        }
        let y = position.y - bounds.origin.y + self.scroll_y;
        let d = self.map.row_at(y);
        if self.map.is_gap(d) {
            return None;
        }
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
        if col >= doc.buffer.line(row).chars().count() {
            return None;
        }
        Some(doc.buffer.line_to_char(row) + col)
    }

    /// A char's cell in window coordinates.
    pub fn eval_char_window_bounds(
        &mut self,
        offset: usize,
        window: &mut Window,
        cx: &App,
    ) -> Bounds<Pixels> {
        self.sync_map(cx);
        let doc = self.document.read(cx);
        let offset = offset.min(doc.buffer.len_chars());
        let at = doc.buffer.offset_to_point(offset);
        let theme = cx.global::<Theme>();
        let seg = self.map.segment_of(at.row, at.column);
        let range = self.map.segment_range(&doc.buffer, at.row, seg);
        let text = doc.buffer.line(at.row);
        let seg_text = &text[byte_of_col(&text, range.start)..byte_of_col(&text, range.end)];
        let line = DisplayLine::with_start_column(seg_text, range.start, doc.buffer.indent.width);
        let shaped =
            shape_display_line(&line, &[], theme, self.tm.font_size, &self.options, window);
        let col = at.column.saturating_sub(range.start);
        let x = shaped.x_for_index(line.display_byte_for_char(col));
        let d = self.map.display_row(at.row, at.column);
        Bounds::new(
            point(
                self.last_bounds.origin.x + self.text_offset() + px(self.tm.gutter_width) + x
                    - self.scroll_x,
                self.last_bounds.origin.y + self.map.top(d) - self.scroll_y,
            ),
            size(self.space_px, self.line_height()),
        )
    }

    /// The view's element bounds in window coordinates, as last laid out.
    pub fn eval_view_bounds(&self) -> Bounds<Pixels> {
        self.last_bounds
    }

    /// What moves a popup off its expression: the buffer's version and the
    /// scroll position.
    pub fn eval_view_key(&self, cx: &App) -> (u64, Point<Pixels>) {
        (
            self.document.read(cx).buffer.version(),
            point(self.scroll_x, self.scroll_y),
        )
    }
}

// ---- the popup -----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuickEvaluateEvent {
    Close,
}

/// The quick evaluate popup: one expression's value in the session's
/// selected frame, at the expression in its editor.
pub struct QuickEvaluate {
    editor: WeakEntity<EditorView>,
    session: WeakEntity<DebugSession>,
    pub expression: String,
    /// The expression's chars in the editor's buffer.
    pub range: Range<usize>,
    /// The view as it was when the popup opened: an edit or a scroll
    /// closes it.
    view_key: (u64, Point<Pixels>),
    /// `None` while the adapter answers.
    result: Option<Result<Variable, String>>,
    /// Opened by the pointer resting on the expression, not by the action:
    /// it has no keyboard and closes when the pointer leaves.
    pub from_mouse: bool,
    /// The pointer is over the popup (a mouse popup stays open then).
    pub hovered: bool,
    selected: usize,
    focus_handle: FocusHandle,
    tui_scroll: crate::skin::RowScroll,
    _evaluate: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<QuickEvaluateEvent> for QuickEvaluate {}

impl Focusable for QuickEvaluate {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl QuickEvaluate {
    /// Evaluates `expression` (at `range` in `editor`) in `session` and
    /// shows the answer when it comes. A keyboard popup takes the focus.
    pub fn new(
        editor: &Entity<EditorView>,
        session: &Entity<DebugSession>,
        expression: String,
        range: Range<usize>,
        from_mouse: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = session.update(cx, |s, cx| {
            s.evaluate(expression.clone(), EvalContext::Hover, cx)
        });
        let evaluate = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.result = Some(result);
                cx.notify();
            });
        });
        let view_key = editor.read(cx).eval_view_key(cx);
        let focus_handle = cx.focus_handle();
        let subscriptions = vec![
            // The view moved under the popup, or the text changed.
            cx.observe(editor, |this: &mut Self, editor, cx| {
                if editor.read(cx).eval_view_key(cx) != this.view_key {
                    cx.emit(QuickEvaluateEvent::Close);
                }
            }),
            // Children arrive; the debuggee runs on.
            cx.observe(session, |_: &mut Self, session, cx| {
                if !session.read(cx).state.is_stopped() {
                    cx.emit(QuickEvaluateEvent::Close);
                }
                cx.notify();
            }),
        ];
        QuickEvaluate {
            editor: editor.downgrade(),
            session: session.downgrade(),
            expression,
            range,
            view_key,
            result: None,
            from_mouse,
            hovered: false,
            selected: 0,
            focus_handle,
            tui_scroll: crate::skin::RowScroll::default(),
            _evaluate: evaluate,
            _subscriptions: subscriptions,
        }
    }

    /// A popup the action opened takes the keyboard (Up, Down, Enter,
    /// Escape) and closes when it loses it.
    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.focus_handle.clone();
        self._subscriptions
            .push(cx.on_blur(&handle, window, |_, _, cx| {
                cx.emit(QuickEvaluateEvent::Close)
            }));
        window.focus(&handle, cx);
    }

    /// Closes the popup; one holding the keyboard hands it back to its editor.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_handle.is_focused(window) {
            if let Some(editor) = self.editor.upgrade() {
                let handle = editor.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
        }
        cx.emit(QuickEvaluateEvent::Close);
    }

    fn rows(&self, cx: &App) -> Vec<EvalRow> {
        let Some(Ok(var)) = &self.result else {
            return Vec::new();
        };
        match self.session.upgrade() {
            Some(s) => {
                let s = s.read(cx);
                eval_rows(var, &s.children, &s.expanded)
            }
            None => eval_rows(var, &HashMap::new(), &HashSet::new()),
        }
    }

    fn toggle(&mut self, reference: i64, cx: &mut Context<Self>) {
        if reference == 0 {
            return;
        }
        if let Some(session) = self.session.upgrade() {
            session.update(cx, |s, cx| s.toggle_expanded(reference, cx));
        }
        cx.notify();
    }

    fn add_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.session.upgrade() {
            let expression = self.expression.clone();
            session.update(cx, |s, cx| s.add_watch(expression, cx));
        }
        self.close(window, cx);
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = self.selected.saturating_sub(1);
        cx.notify();
    }

    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.rows(cx).len();
        if self.selected + 1 < n {
            self.selected += 1;
            cx.notify();
        }
    }

    fn confirm(&mut self, _: &PopupConfirm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(EvalRow::Value { var, .. }) = self.rows(cx).get(self.selected) {
            let r = var.variables_reference;
            self.toggle(r, cx);
        }
    }

    fn cancel(&mut self, _: &PopupCancel, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    /// The expression's first char in window coordinates.
    fn anchor(&self, window: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        let editor = self.editor.upgrade()?;
        let start = self.range.start;
        Some(editor.update(cx, |e, cx| e.eval_char_window_bounds(start, window, cx)))
    }
}

/// A value's text after its name: `= value`, the error for a failed one.
fn value_text(var: &Variable) -> String {
    var.value.replace('\n', " ")
}

impl Render for QuickEvaluate {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(anchor) = self.anchor(window, cx) else {
            return div().into_any_element();
        };
        if crate::skin::Skin::current(cx) == crate::skin::Skin::Tui {
            return self.render_tui(anchor, window, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let viewport = window.viewport_size();
        let focused = self.focus_handle.is_focused(window);
        let rows = self.rows(cx);
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        let row_h = px(m.quick_eval_row);
        let shown = rows.len().clamp(1, m.quick_eval_max_rows as usize);
        let footer = self.result.as_ref().is_some_and(|r| r.is_ok());
        let height = row_h * shown as f32
            + px(2. * m.smart_popup_pad_y + 2.)
            + if footer { row_h + px(1.) } else { px(0.) };
        let width_cap = px(m.smart_popup_max_width).min(viewport.width - px(2. * m.island_margin));
        // Under the expression's row (estimate: IntelliJ opens its evaluate
        // popup there), above it when the window has no room below.
        let gap = px(m.completion_gap_y);
        let below = anchor.origin.y + anchor.size.height + gap;
        let y = if below + height <= viewport.height {
            below
        } else {
            (anchor.origin.y - gap - height).max(px(0.))
        };
        let x = (anchor.origin.x - px(m.smart_popup_pad_x))
            .min(viewport.width - width_cap)
            .max(px(0.));
        let entity = cx.entity();
        let body: Vec<AnyElement> = match &self.result {
            None => vec![div()
                .h(row_h)
                .flex()
                .items_center()
                .text_color(theme.text_secondary)
                .child("Collecting data…")
                .into_any_element()],
            Some(Err(message)) => vec![div()
                .h(row_h)
                .flex()
                .items_center()
                .whitespace_nowrap()
                .child(
                    div()
                        .flex_shrink_0()
                        .child(SharedString::from(self.expression.clone())),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme.text_secondary)
                        .child(" = "),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.text_dangerous)
                        .child(SharedString::from(message.replace('\n', " "))),
                )
                .into_any_element()],
            Some(Ok(_)) => rows
                .iter()
                .enumerate()
                .map(|(ix, row)| {
                    let selected = focused && ix == self.selected;
                    match row {
                        EvalRow::Loading { depth } => div()
                            .h(row_h)
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .pl(px(m.popup_row_pad_x
                                + *depth as f32 * m.quick_eval_indent
                                + m.icon_size))
                            .text_color(theme.text_secondary)
                            .child("Collecting data…")
                            .into_any_element(),
                        EvalRow::Value {
                            depth,
                            var,
                            expanded,
                        } => {
                            let reference = var.variables_reference;
                            let toggle = entity.clone();
                            div()
                                .id(ElementId::NamedInteger("eval-row".into(), ix as u64))
                                .h(row_h)
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .px(px(m.popup_row_pad_x))
                                .pl(px(m.popup_row_pad_x + *depth as f32 * m.quick_eval_indent))
                                .rounded(px(m.popup_row_radius))
                                .whitespace_nowrap()
                                .when(selected, |d| d.bg(theme.list_bg_focused))
                                .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                                .when(reference != 0, |d| d.cursor_pointer())
                                .on_click(move |_, _, cx| {
                                    toggle.update(cx, |this, cx| {
                                        this.selected = ix;
                                        this.toggle(reference, cx);
                                    })
                                })
                                .child(div().w(px(m.icon_size)).flex_shrink_0().when(
                                    reference != 0,
                                    |d| {
                                        d.child(crate::assets::icon(
                                            if *expanded {
                                                "expui/general/chevronDown"
                                            } else {
                                                "expui/general/chevronRight"
                                            },
                                            m.icon_size,
                                            theme.icon_secondary,
                                        ))
                                    },
                                ))
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .pl(px(m.popup_row_icon_gap / 2.))
                                        .child(SharedString::from(var.name.clone())),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_color(theme.text_secondary)
                                        .child(" = "),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .child(SharedString::from(value_text(var))),
                                )
                                .children(var.type_name.clone().map(|t| {
                                    div()
                                        .flex_shrink_0()
                                        .ml(px(m.popup_row_icon_gap))
                                        .text_color(theme.text_tertiary)
                                        .child(SharedString::from(t))
                                }))
                                .into_any_element()
                        }
                    }
                })
                .collect(),
        };
        let hover = entity.clone();
        let watch = entity.clone();
        let close = entity.clone();
        let popup = div()
            .id("quick-evaluate")
            .key_context("Popup")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_hover(move |hovered, _, cx| {
                let hovered = *hovered;
                hover.update(cx, |this, _| this.hovered = hovered);
            })
            .when(!self.from_mouse, |d| {
                d.on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |_, cx| cx.emit(QuickEvaluateEvent::Close))
                })
            })
            .occlude()
            .absolute()
            .left(x)
            .top(y)
            .min_w(px(m.quick_eval_min_width).min(width_cap))
            .max_w(width_cap)
            .flex()
            .flex_col()
            .font_family(theme.mono_font.clone())
            .text_size(px(m.ui_font_size))
            .text_color(theme.popup_text)
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
            .child(
                div()
                    .id("quick-evaluate-rows")
                    .max_h(row_h * shown as f32 + px(2. * m.smart_popup_pad_y))
                    .overflow_y_scroll()
                    .px(px(m.smart_popup_pad_x / 2.))
                    .py(px(m.smart_popup_pad_y))
                    .flex()
                    .flex_col()
                    .children(body),
            )
            .when(footer, |d| {
                d.child(
                    div()
                        .h(row_h)
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .px(px(m.smart_popup_pad_x))
                        .border_t_1()
                        .border_color(theme.popup_separator)
                        .font_family(theme.ui_font.clone())
                        .child(
                            div()
                                .id("quick-evaluate-watch")
                                .cursor_pointer()
                                .text_color(theme.link)
                                .hover(|s| s.underline())
                                .child("Add to Watches")
                                .on_click(move |_, window, cx| {
                                    watch.update(cx, |this, cx| this.add_watch(window, cx))
                                }),
                        ),
                )
            });
        deferred(popup).with_priority(1).into_any_element()
    }
}

// ---- the tui skin ------------------------------------------------------------------

impl QuickEvaluate {
    /// The popup in the tui skin: a box on the grid under the expression's
    /// row (above it without room), a row per value: the indent, `▸`/`▾`,
    /// `name = value` and the type in `text_tertiary`; under a rule,
    /// `[ Add to Watches ]`; the keys in the bottom edge while it has the
    /// keyboard.
    fn render_tui(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::skin::{self, Cell};
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let viewport = window.viewport_size();
        let focused = self.focus_handle.is_focused(window);
        let all_cols = Cell::fit(viewport.width, cell.w);
        let all_rows = Cell::fit(viewport.height, cell.h);
        let rows = self.rows(cx);
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        let lines: Vec<(String, Option<String>, Option<i64>)> = match &self.result {
            None => vec![("Collecting data…".into(), None, None)],
            Some(Err(e)) => vec![(
                format!("{} = {}", self.expression, e.replace('\n', " ")),
                None,
                None,
            )],
            Some(Ok(_)) => rows
                .iter()
                .map(|row| match row {
                    EvalRow::Loading { depth } => (
                        format!("{}  Collecting data…", "  ".repeat(*depth)),
                        None,
                        None,
                    ),
                    EvalRow::Value {
                        depth,
                        var,
                        expanded,
                    } => {
                        let mark = match (var.variables_reference != 0, expanded) {
                            (false, _) => " ",
                            (true, true) => "▾",
                            (true, false) => "▸",
                        };
                        (
                            format!(
                                "{}{} {} = {}",
                                "  ".repeat(*depth),
                                mark,
                                var.name,
                                value_text(var)
                            ),
                            var.type_name.clone(),
                            Some(var.variables_reference),
                        )
                    }
                })
                .collect(),
        };
        let footer = self.result.as_ref().is_some_and(|r| r.is_ok());
        let widest = lines
            .iter()
            .map(|(t, ty, _)| Cell::text_cols(t) + ty.as_ref().map_or(0, |t| 2 + t.chars().count()))
            .max()
            .unwrap_or(0);
        let cap = ((m.smart_popup_max_width / cell.w).round() as usize).max(24);
        let cols = (widest + 4).clamp(24, cap).min(all_cols.max(8));
        let inner = cols - 2;
        let max_shown = (m.quick_eval_max_rows as usize).min(all_rows.saturating_sub(6).max(1));
        let visible = lines.len().clamp(1, max_shown);
        self.tui_scroll.follow(self.selected, lines.len(), visible);
        let top = self.tui_scroll.top;
        let box_rows = visible + 2 + if footer { 2 } else { 0 };
        let entity = cx.entity();
        let body_rows: Vec<AnyElement> = lines
            .iter()
            .enumerate()
            .skip(top)
            .take(visible)
            .map(|(ix, (text, ty, reference))| {
                let selected = focused && ix == self.selected;
                let reference = reference.unwrap_or(0);
                let toggle = entity.clone();
                let ty_cols = ty.as_ref().map_or(0, |t| 2 + t.chars().count());
                let text_room = inner.saturating_sub(1 + ty_cols.min(inner / 3));
                div()
                    .id(ElementId::NamedInteger("eval-row".into(), ix as u64))
                    .w(cell.cols(inner))
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .flex()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .pl(cell.cols(1))
                    .text_color(if selected {
                        theme.list_text_focused
                    } else {
                        theme.popup_text
                    })
                    .when(selected, |d| d.bg(theme.list_bg_focused))
                    .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .when(reference != 0, |d| d.cursor_pointer())
                    .on_click(move |_, _, cx| {
                        toggle.update(cx, |this, cx| {
                            this.selected = ix;
                            this.toggle(reference, cx);
                        })
                    })
                    .child(SharedString::from(skin::fit_text(text, text_room)))
                    .child(div().flex_1())
                    .children(ty.clone().map(|t| {
                        div()
                            .flex_shrink_0()
                            .pr(cell.cols(1))
                            .text_color(theme.text_tertiary)
                            .child(SharedString::from(skin::fit_text(&t, inner / 3)))
                    }))
                    .into_any_element()
            })
            .collect();
        let watch = entity.clone();
        let body = div()
            .size_full()
            .flex()
            .flex_col()
            .on_scroll_wheel(cx.listener({
                let total = lines.len();
                move |this, e, _, cx| {
                    if this.tui_scroll.wheel(e, cell, total, visible) {
                        cx.notify();
                    }
                }
            }))
            .children(body_rows)
            .when(footer, |d| {
                d.child(div().h(px(cell.h)).flex_shrink_0()).child(
                    div().h(px(cell.h)).flex().pl(cell.cols(1)).child(
                        skin::button(
                            "quick-evaluate-watch",
                            "Add to Watches",
                            false,
                            &theme,
                            cell,
                        )
                        .on_click(move |_, window, cx| {
                            watch.update(cx, |this, cx| this.add_watch(window, cx))
                        }),
                    ),
                )
            })
            .into_any_element();
        let rules = if footer {
            vec![visible + 1]
        } else {
            Vec::new()
        };
        let bg = theme.popup_bg;
        let frame = if focused {
            theme.border_focused
        } else {
            theme.popup_border
        };
        let mut boxed = skin::boxed(
            cell,
            cols,
            box_rows,
            Vec::new(),
            rules,
            body,
            bg,
            frame,
            &theme,
            m,
        )
        .occlude();
        if focused {
            boxed = boxed.child(skin::edge_strip(
                cell,
                box_rows - 1,
                2,
                cols.saturating_sub(4),
                skin::hints(&[("↵", "expand"), ("esc", "close")], &theme),
                bg,
            ));
        }
        if lines.len() > visible {
            boxed = boxed.child(crate::list_popup::tui_scrollbar(
                cell,
                cols - 1,
                1,
                1 + visible,
                top,
                visible,
                lines.len(),
                &theme,
                m,
            ));
        }
        // On the grid: the row under the expression's, else above it.
        let anchor_col = Cell::fit(anchor.origin.x, cell.w);
        let anchor_row = Cell::fit(anchor.origin.y, cell.h);
        let col = anchor_col
            .saturating_sub(2)
            .min(all_cols.saturating_sub(cols));
        let row = if anchor_row + 1 + box_rows <= all_rows {
            anchor_row + 1
        } else {
            anchor_row.saturating_sub(box_rows)
        };
        let hover = entity.clone();
        let close = entity;
        deferred(
            div()
                .id("quick-evaluate")
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::cancel))
                .on_hover(move |hovered, _, cx| {
                    let hovered = *hovered;
                    hover.update(cx, |this, _| this.hovered = hovered);
                })
                .when(!self.from_mouse, |d| {
                    d.on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |_, cx| cx.emit(QuickEvaluateEvent::Close))
                    })
                })
                .absolute()
                .left(cell.cols(col))
                .top(cell.rows(row))
                .child(boxed),
        )
        .with_priority(1)
        .into_any_element()
    }
}

/// The session quick evaluate asks: the active one, while it is stopped.
pub fn stopped_session(cx: &App) -> Option<Entity<DebugSession>> {
    let session = crate::debugger::Debugger::global(cx).read(cx).active()?;
    matches!(session.read(cx).state, SessionState::Stopped { .. }).then_some(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: &str, col: usize) -> Option<String> {
        expression_at(line, col).map(|r| line.chars().skip(r.start).take(r.len()).collect())
    }

    #[test]
    fn identifiers_and_qualifiers() {
        let line = "    let total: f64 = index.values().map(Record::score).sum();";
        assert_eq!(at(line, 9), Some("total".into()));
        // Just after the word counts as on it.
        assert_eq!(at(line, 13), Some("total".into()));
        assert_eq!(at(line, 21), Some("index".into()));
        // `values(` is a call; `Record::score` is a path.
        assert_eq!(at(line, 28), None);
        assert_eq!(at(line, 50), Some("Record::score".into()));
        assert_eq!(at(line, 40), Some("Record".into()));
        // A qualifier off a call is not evaluated.
        assert_eq!(at("f().x", 4), None);
        assert_eq!(at("v[0].x", 5), None);
    }

    #[test]
    fn fields_paths_and_pointers() {
        assert_eq!(at("self.name.is_empty()", 7), Some("self.name".into()));
        assert_eq!(at("self.name.is_empty()", 2), Some("self".into()));
        assert_eq!(
            at("x = std::f64::consts::PI;", 23),
            Some("std::f64::consts::PI".into())
        );
        assert_eq!(at("n = p->next->value;", 15), Some("p->next->value".into()));
        assert_eq!(at("let (a, b) = pair.0;", 18), Some("pair.0".into()));
        assert_eq!(at("r.tags.contains(&\"alpha\")", 3), Some("r.tags".into()));
    }

    #[test]
    fn nothing_to_evaluate() {
        assert_eq!(at("    let x = 1;", 0), None);
        assert_eq!(at("    let x = 1;", 5), None); // `let`
        assert_eq!(at("x = 0x1F + 1.5;", 6), None);
        assert_eq!(at("x = 1.5;", 6), None);
        assert_eq!(at("tags: Vec<&'static str>", 14), None);
        assert_eq!(at("println!(\"{r:?}\")", 3), None);
        assert_eq!(at("a + b", 2), None);
        assert_eq!(at("", 0), None);
        assert_eq!(at("abc", 9), None);
    }

    #[test]
    fn ranges_are_not_qualifiers() {
        assert_eq!(at("for i in 0..count {", 13), Some("count".into()));
    }

    #[test]
    fn selections_are_trimmed() {
        assert_eq!(selection_expression("  a + b \n"), Some("a + b".into()));
        assert_eq!(selection_expression(" \n "), None);
    }

    fn var(name: &str, reference: i64) -> Variable {
        Variable {
            name: name.into(),
            value: format!("{name}-value"),
            type_name: None,
            variables_reference: reference,
            evaluate_name: None,
        }
    }

    #[test]
    fn value_tree_rows() {
        let root = var("index", 1);
        let mut children = HashMap::new();
        children.insert(1, vec![var("[0]", 2), var("len", 0)]);
        let mut expanded = HashSet::new();
        // Closed: the value alone.
        assert_eq!(eval_rows(&root, &children, &expanded).len(), 1);
        expanded.insert(1);
        expanded.insert(2);
        let rows = eval_rows(&root, &children, &expanded);
        // index, [0] (expanded, not fetched yet: loading), len.
        assert_eq!(rows.len(), 4);
        assert!(matches!(
            &rows[0],
            EvalRow::Value {
                depth: 0,
                expanded: true,
                ..
            }
        ));
        assert!(matches!(
            &rows[1],
            EvalRow::Value {
                depth: 1,
                expanded: true,
                ..
            }
        ));
        assert_eq!(rows[2], EvalRow::Loading { depth: 2 });
        assert!(matches!(
            &rows[3],
            EvalRow::Value {
                depth: 1,
                expanded: false,
                ..
            }
        ));
    }

    #[test]
    fn value_tree_stops_at_a_cycle() {
        let root = var("node", 1);
        let mut children = HashMap::new();
        children.insert(1, vec![var("next", 1)]);
        let expanded: HashSet<i64> = [1].into_iter().collect();
        let rows = eval_rows(&root, &children, &expanded);
        assert_eq!(rows.len(), 2);
        assert!(matches!(
            &rows[1],
            EvalRow::Value {
                depth: 1,
                expanded: false,
                ..
            }
        ));
    }
}
