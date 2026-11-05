//! The Debugger Console tool (M8): the active session's console (the
//! adapter's messages, the "Process finished" line; the build's and the
//! debuggee's output go to the configuration's run tab) and the lines typed
//! into it, in the terminal's mono font, with an input at the bottom whose
//! Enter sends the text to `DebugSession::evaluate_in_console` (Up and Down
//! walk the typed history).
//!
//! Fleet's console is in LLDB's "commands" mode (batch 13, debug-fleet-console
//! and -console-evaluated): its first line says so, then "Launching:
//! <program>"; a typed line is an LLDB command, a line after `?` an
//! expression. Each becomes a block ([`Block`]): the input highlighted as
//! Rust on a band of `background.secondary` after a chevron, the answer
//! under it (an expression's `{type} value` in the UI font, the type in
//! `text.secondary`; a command's output as mono lines), then the time it
//! took ("3ms", `text.tertiary` italics) and "rerun" (`link.text.default`,
//! re-sending the input as a new block) at the right, a 1 px
//! `separator.default` rule between a block and its neighbours
//! (`metrics::CONSOLE_BLOCK_*`). The lines 32.5 in from the island, wrapped
//! at its width; the input a 100 px band of `background.secondary` across
//! the island's bottom with a `text.tertiary` chevron at its left
//! (`metrics::CONSOLE_*`). The adapter's own lines are `text.primary`
//! (batch 13); stderr and errors `text.dangerous` (estimate). The list
//! sticks to its bottom while new lines arrive, unless scrolled up. Key
//! context: `DebuggerConsole`.

use crate::actions::{PopupDown, PopupUp};
use crate::assets::icon;
use crate::debug_tool::Follow;
use crate::debugger::{
    elapsed_text, ConsoleKind, ConsoleLine, DebugSession, EvalAnswer, Evaluation,
};
use crate::editor::styled_line;
use crate::git_tool::tui_list;
use crate::input::{InputEvent, TextInput};
use crate::settings::AppSettings;
use crate::skin::{self, Cell, FrameBars, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable,
    HighlightStyle, Hsla, Render, ScrollHandle, SharedString, StyledText, TextRun, Window,
};
use std::collections::HashMap;
use std::rc::Rc;
use weep_syntax::{Highlighter, Language};

/// The typed expressions, oldest first, and where Up/Down stand in them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    entries: Vec<String>,
    /// The entry shown, while walking.
    cursor: Option<usize>,
    /// What was in the field before the walk started.
    draft: String,
}

impl History {
    /// Remembers a submitted line (not twice in a row) and ends the walk.
    pub fn push(&mut self, line: &str) {
        let line = line.trim();
        if !line.is_empty() && self.entries.last().map(String::as_str) != Some(line) {
            self.entries.push(line.to_string());
        }
        self.cursor = None;
        self.draft.clear();
    }

    /// Up: the previous entry, `current` kept as the draft at the start.
    pub fn up(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let ix = match self.cursor {
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(i) => i - 1,
        };
        self.cursor = Some(ix);
        Some(self.entries[ix].clone())
    }

    /// Down: the next entry, then back to the draft.
    pub fn down(&mut self) -> Option<String> {
        let ix = self.cursor?;
        if ix + 1 < self.entries.len() {
            self.cursor = Some(ix + 1);
            Some(self.entries[ix + 1].clone())
        } else {
            self.cursor = None;
            Some(std::mem::take(&mut self.draft))
        }
    }
}

/// What the console shows, top to bottom: the adapter's lines, and a block
/// per line typed (batch 13, debug-fleet-console-evaluated), a rule between
/// a block and whatever is next to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// One row of a plain line (a line with breaks is several).
    Line(ConsoleKind, String),
    Rule,
    Block(Block),
}

/// An evaluation's block: the input on its band, the answer's rows under
/// it, then the time it took and "rerun" at the right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// Its index in `DebugSession::evaluations` (what rerun sends again).
    pub index: usize,
    /// As typed, the `?` kept.
    pub input: String,
    pub rows: Vec<BlockRow>,
    /// "3ms", once answered.
    pub elapsed: Option<String>,
}

/// A row of a block's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockRow {
    /// An expression's value: `{type} ` (in `text.secondary`) and the value
    /// (`text.primary`), in the UI font (batch 13).
    Value {
        type_text: Option<String>,
        value: String,
    },
    /// A line of a command's output, in the mono font as LLDB lays it out
    /// (estimate: Fleet's command output unprobed).
    Output(String),
    /// A failed request, or a line LLDB starts with `error:`, in
    /// `text.dangerous` (estimate).
    Error(String),
}

/// A text's lines without its last line break (`\r\n` taken as one).
fn lines_of(text: &str) -> impl Iterator<Item = String> + '_ {
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
}

/// An answer as its block's rows.
pub fn block_rows(answer: &EvalAnswer) -> Vec<BlockRow> {
    match answer {
        EvalAnswer::Value { type_name, value } => vec![BlockRow::Value {
            type_text: type_name
                .as_deref()
                .filter(|t| !t.is_empty())
                .map(|t| format!("{{{t}}} ")),
            value: value.clone(),
        }],
        EvalAnswer::Output(text) if text.trim().is_empty() => Vec::new(),
        EvalAnswer::Output(text) => lines_of(text)
            .map(|l| {
                if l.starts_with("error:") {
                    BlockRow::Error(l)
                } else {
                    BlockRow::Output(l)
                }
            })
            .collect(),
        EvalAnswer::Error(message) => lines_of(message).map(BlockRow::Error).collect(),
    }
}

/// The console's items from the session's lines and evaluations.
pub fn console_items(console: &[ConsoleLine], evaluations: &[Evaluation]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut after_block = false;
    for line in console {
        match line.kind {
            ConsoleKind::Evaluation(index) => {
                if !items.is_empty() {
                    items.push(Item::Rule);
                }
                let e = evaluations.get(index);
                items.push(Item::Block(Block {
                    index,
                    input: line.text.to_string(),
                    rows: e
                        .and_then(|e| e.answer.as_ref())
                        .map(block_rows)
                        .unwrap_or_default(),
                    elapsed: e.and_then(|e| e.elapsed).map(elapsed_text),
                }));
                after_block = true;
            }
            kind => {
                if std::mem::take(&mut after_block) {
                    items.push(Item::Rule);
                }
                for part in lines_of(&line.text) {
                    items.push(Item::Line(kind, part));
                }
            }
        }
    }
    items
}

/// The items on the tui skin's grid, a row each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiRow {
    Line(ConsoleKind, String),
    /// A rule across the island.
    Rule,
    /// `› input` on the band's colour.
    Input(String),
    Answer(BlockRow),
    /// The time and `rerun` at the right, for the block at this index.
    Meta(usize, String),
}

pub fn tui_rows(items: &[Item]) -> Vec<TuiRow> {
    let mut rows = Vec::new();
    for item in items {
        match item {
            Item::Line(kind, text) => rows.push(TuiRow::Line(*kind, text.clone())),
            Item::Rule => rows.push(TuiRow::Rule),
            Item::Block(b) => {
                rows.push(TuiRow::Input(b.input.clone()));
                rows.extend(b.rows.iter().cloned().map(TuiRow::Answer));
                if let Some(elapsed) = &b.elapsed {
                    rows.push(TuiRow::Meta(b.index, elapsed.clone()));
                }
            }
        }
    }
    rows
}

/// The tui skin's meta row: `3ms  rerun` ending one cell from the right
/// edge; the column where `rerun` starts.
pub fn tui_meta(elapsed: &str, cols: usize) -> (String, usize) {
    let text = format!("{elapsed}  {RERUN}");
    let width = text.chars().count();
    let pad = cols.saturating_sub(width + 1);
    (
        format!("{}{text}", " ".repeat(pad)),
        pad + elapsed.chars().count() + 2,
    )
}

/// The block's link (batch 13).
const RERUN: &str = "rerun";

fn line_color(kind: ConsoleKind, theme: &Theme) -> Hsla {
    match kind {
        // Batch 13: the adapter's own lines ("Launching: …") in `text.primary`.
        ConsoleKind::Stdout | ConsoleKind::Console | ConsoleKind::Evaluation(_) => {
            theme.text_primary
        }
        ConsoleKind::Stderr | ConsoleKind::Error => theme.text_dangerous,
    }
}

fn block_row_color(row: &BlockRow, theme: &Theme) -> Hsla {
    match row {
        BlockRow::Error(_) => theme.text_dangerous,
        _ => theme.text_primary,
    }
}

/// A block's value row: the type in `text.secondary`, the value in
/// `text.primary` (batch 13: peaks 133 and 220 on the island).
fn value_text(type_text: Option<&str>, value: &str, theme: &Theme) -> StyledText {
    let type_len = type_text.map_or(0, str::len);
    let text = format!("{}{value}", type_text.unwrap_or(""));
    let mut highlights = vec![(
        type_len..text.len(),
        HighlightStyle {
            color: Some(theme.text_primary),
            ..Default::default()
        },
    )];
    if type_len > 0 {
        highlights.insert(
            0,
            (
                0..type_len,
                HighlightStyle {
                    color: Some(theme.text_secondary),
                    ..Default::default()
                },
            ),
        );
    }
    StyledText::new(text).with_highlights(highlights)
}

/// Estimate: the field's hint.
const HINT: &str = "Evaluate expression";

pub struct DebugConsole {
    follow: Follow,
    input: Entity<TextInput>,
    history: History,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// The rows last drawn, to tell new output from a redraw.
    drawn: usize,
    /// The list follows new output (it was at its bottom).
    stick: bool,
    highlighter: Highlighter,
    /// Each block's input highlighted, by the input's text, in the theme
    /// named by `code_theme`.
    code: HashMap<String, (SharedString, Vec<TextRun>)>,
    code_theme: String,
    tui_grid: Rc<std::cell::Cell<(usize, usize)>>,
    tui_bars: FrameBars,
    tui_scroll: RowScroll,
}

/// The tool's focus is its field's: the keys typed go to the expression.
impl Focusable for DebugConsole {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl DebugConsole {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut i = TextInput::new(HINT, cx);
            i.mono = true;
            i
        });
        cx.subscribe_in(&input, window, Self::on_input).detach();
        let follow = Follow::start(|c: &mut DebugConsole| &mut c.follow, Self::switched, cx);
        DebugConsole {
            follow,
            input,
            history: History::default(),
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            drawn: 0,
            stick: true,
            highlighter: Highlighter::new(),
            code: HashMap::new(),
            code_theme: String::new(),
            tui_grid: Default::default(),
            tui_bars: Default::default(),
            tui_scroll: RowScroll::default(),
        }
    }

    fn switched(&mut self, _cx: &mut Context<Self>) {
        self.drawn = 0;
        self.stick = true;
        self.tui_scroll.reset();
    }

    /// The scroll bars this view's lists set on its island's frame.
    pub fn tui_bars(&self) -> FrameBars {
        self.tui_bars.clone()
    }

    fn session(&self) -> Option<Entity<DebugSession>> {
        self.follow.session.clone()
    }

    fn on_input(
        &mut self,
        input: &Entity<TextInput>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::Submit { .. } = event {
            let text = input.read(cx).text().trim().to_string();
            if text.is_empty() {
                return;
            }
            self.history.push(&text);
            input.update(cx, |i, cx| i.set_text_at_end("", cx));
            self.send(text, cx);
        }
    }

    fn send(&mut self, text: String, cx: &mut Context<Self>) {
        self.stick = true;
        if let Some(session) = self.session() {
            session.update(cx, |s, cx| s.evaluate_in_console(text, cx));
        }
        cx.notify();
    }

    /// A block's "rerun": its input again, as a new block at the bottom.
    fn rerun(&mut self, index: usize, cx: &mut Context<Self>) {
        self.stick = true;
        if let Some(session) = self.session() {
            session.update(cx, |s, cx| s.rerun_evaluation(index, cx));
        }
        cx.notify();
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        let current = self.input.read(cx).text().to_string();
        if let Some(text) = self.history.up(&current) {
            self.input.update(cx, |i, cx| i.set_text_at_end(&text, cx));
        }
    }

    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.history.down() {
            self.input.update(cx, |i, cx| i.set_text_at_end(&text, cx));
        }
    }

    fn items(&self, cx: &App) -> Vec<Item> {
        match &self.follow.session {
            Some(s) => {
                let s = s.read(cx);
                console_items(&s.console, &s.evaluations)
            }
            None => Vec::new(),
        }
    }

    /// A block's input highlighted as Rust (batch 13: Fleet highlights the
    /// input as code, its `?` included), in the editor's font and colours.
    fn code(&mut self, input: &str, theme: &Theme, cx: &App) -> (SharedString, Vec<TextRun>) {
        if self.code_theme != theme.name || self.code.len() > 512 {
            self.code.clear();
            self.code_theme = theme.name.clone();
        }
        if let Some(found) = self.code.get(input) {
            return found.clone();
        }
        let spans: Vec<(std::ops::Range<usize>, &'static str)> = self
            .highlighter
            .highlight(Language::Rust, input.as_bytes())
            .into_iter()
            .map(|s| (s.range, s.attribute))
            .collect();
        let (text, runs) = styled_line(input, &spans, theme, cx);
        let made = (SharedString::from(text), runs);
        self.code.insert(input.to_string(), made.clone());
        made
    }

    /// Keeps the list at its bottom when rows were added and it was there
    /// (or a line was just sent).
    fn follow_rows(&mut self, count: usize, at_bottom: bool) -> bool {
        let moved = count != self.drawn && (self.stick || at_bottom);
        if count != self.drawn {
            self.stick = false;
            self.drawn = count;
        }
        moved
    }
}

impl Render for DebugConsole {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.items(cx);
        let body = if Skin::current(cx) == Skin::Tui {
            self.render_tui(items, window, cx)
        } else {
            self.render_fleet(items, window, cx)
        };
        let input = self.input.clone();
        div()
            .id("debugger-console")
            .key_context("DebuggerConsole")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            // A click on the output puts the caret in the field.
            .on_click(move |_, window, cx| {
                let handle = input.read(cx).focus_handle(cx);
                handle.focus(window, cx);
            })
            .size_full()
            .relative()
            .overflow_hidden()
            .child(body)
    }
}

impl DebugConsole {
    fn render_fleet(
        &mut self,
        items: Vec<Item>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let (font_size, line_height) = m.terminal_text(&cx.global::<AppSettings>().settings);
        // At the bottom (or nothing to scroll) before this frame's rows:
        // new output keeps the list there.
        let offset = self.scroll.offset().y;
        let max = self.scroll.max_offset().y;
        let at_bottom = f32::from(max) <= 0. || f32::from(offset) <= -f32::from(max) + 1.;
        if self.follow_rows(tui_rows(&items).len(), at_bottom) {
            self.scroll.scroll_to_bottom();
        }
        let mut children: Vec<AnyElement> = Vec::new();
        for item in items {
            match item {
                Item::Line(kind, text) => {
                    // Batch 13: a long line wraps at the island's width.
                    children.push(
                        div()
                            .min_h(px(line_height))
                            .flex_shrink_0()
                            .pl(px(m.console_pad_x))
                            .pr(px(m.console_pad_right))
                            .text_color(line_color(kind, &theme))
                            .child(SharedString::from(text))
                            .into_any_element(),
                    );
                }
                Item::Rule => children.push(
                    div()
                        .flex_shrink_0()
                        .h(px(m.console_block_rule))
                        .bg(theme.separator)
                        .into_any_element(),
                ),
                Item::Block(block) => {
                    children.push(self.fleet_block(block, font_size, line_height, &theme, m, cx))
                }
            }
        }
        let prompt_top = m.console_prompt_centre - m.icon_size / 2.;
        let field_top = m.console_prompt_centre - line_height / 2.;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("console-output")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .font_family(theme.mono_font.clone())
                    .text_size(px(font_size))
                    .line_height(px(line_height))
                    .flex()
                    .flex_col()
                    .children(children),
            )
            // The input: a band of `background.secondary` across the
            // island's bottom, a chevron, the field on its first line.
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(m.console_input_height))
                    .relative()
                    .bg(theme.background_secondary)
                    .child(
                        div()
                            .absolute()
                            .left(px(m.console_prompt_x))
                            .top(px(prompt_top))
                            .child(icon(
                                "expui/general/chevronRight",
                                m.icon_size,
                                theme.text_tertiary,
                            )),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(m.console_pad_x))
                            .right(px(m.console_pad_right))
                            .top(px(field_top))
                            .h(px(line_height))
                            .text_size(px(font_size))
                            .child(self.input.clone()),
                    ),
            )
            .into_any_element()
    }

    /// An evaluation's block (`metrics::CONSOLE_BLOCK_*`): the input on a
    /// rounded band of `background.secondary` after the field's chevron,
    /// the answer under it, then the time in `text.tertiary` italics and
    /// "rerun" in `link.text.default` at the right.
    fn fleet_block(
        &mut self,
        block: Block,
        font_size: f32,
        line_height: f32,
        theme: &Theme,
        m: Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (code, runs) = self.code(&block.input, theme, cx);
        let band = m.console_block_input;
        let band_el = div()
            .flex_shrink_0()
            .h(px(band))
            .relative()
            .rounded(px(m.console_block_radius))
            .bg(theme.background_secondary)
            .child(
                div()
                    .absolute()
                    .left(px(m.console_prompt_x))
                    .top(px((band - m.icon_size) / 2.))
                    .child(icon(
                        "expui/general/chevronRight",
                        m.icon_size,
                        theme.text_tertiary,
                    )),
            )
            .child(
                div()
                    .absolute()
                    .left(px(m.console_block_code_x))
                    .right(px(m.console_pad_right))
                    .top(px((band - line_height) / 2.))
                    .h(px(line_height))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(font_size))
                    .child(StyledText::new(code).with_runs(runs)),
            );
        let rows: Vec<AnyElement> = block
            .rows
            .iter()
            .map(|row| match row {
                BlockRow::Value { type_text, value } => div()
                    .min_h(px(m.console_block_row))
                    .font_family(theme.ui_font.clone())
                    .text_size(px(m.ui_font_size))
                    .line_height(px(m.console_block_row))
                    .child(value_text(type_text.as_deref(), value, theme))
                    .into_any_element(),
                BlockRow::Output(text) | BlockRow::Error(text) => div()
                    .min_h(px(line_height))
                    .text_color(block_row_color(row, theme))
                    .child(SharedString::from(text.clone()))
                    .into_any_element(),
            })
            .collect();
        let mut el = div().flex_shrink_0().flex().flex_col().child(band_el);
        if !rows.is_empty() {
            el = el.child(
                div()
                    .pt(px(m.console_block_result_top))
                    .pl(px(m.console_pad_x))
                    .pr(px(m.console_pad_right))
                    .flex()
                    .flex_col()
                    .children(rows),
            );
        }
        if let Some(elapsed) = block.elapsed {
            let index = block.index;
            el = el.child(
                div()
                    .h(px(m.console_block_meta))
                    .flex()
                    .justify_end()
                    .gap(px(m.console_block_meta_gap))
                    .pr(px(m.console_block_meta_right))
                    .font_family(theme.ui_font.clone())
                    .text_size(px(m.ui_font_size))
                    .line_height(px(m.console_block_row))
                    .child(
                        div()
                            .italic()
                            .text_color(theme.text_tertiary)
                            .child(SharedString::from(elapsed)),
                    )
                    .child(
                        div()
                            .id(ElementId::NamedInteger(
                                "console-rerun".into(),
                                index as u64,
                            ))
                            .cursor_pointer()
                            .text_color(theme.link)
                            .child(RERUN)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.rerun(index, cx);
                            })),
                    ),
            );
        }
        el.into_any_element()
    }

    /// The console in the tui skin: the lines in whole rows, a block as a
    /// rule, `› input` on the band's colour, the answer two cells in, then
    /// `3ms  rerun` at the right; a rule, and the field as the last row
    /// (`› expression▏` while focused).
    fn render_tui(
        &mut self,
        items: Vec<Item>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.tui_bars.clear();
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let (cols, grid_rows) = self.tui_grid.get();
        let visible = grid_rows.saturating_sub(2);
        let rows = tui_rows(&items);
        let total = rows.len();
        let max_top = total.saturating_sub(visible);
        let was_at_bottom = self.tui_scroll.top + visible >= self.drawn;
        if self.follow_rows(total, was_at_bottom) {
            self.tui_scroll.top = max_top;
        }
        self.tui_scroll.top = self.tui_scroll.top.min(max_top);
        let top = self.tui_scroll.top;
        let mut lines: Vec<AnyElement> = Vec::new();
        for row in rows.into_iter().skip(top).take(visible) {
            let line = div()
                .h(px(cell.h))
                .flex_shrink_0()
                .whitespace_nowrap()
                .overflow_hidden();
            let el = match row {
                TuiRow::Line(kind, text) => line
                    .text_color(line_color(kind, &theme))
                    .child(SharedString::from(skin::fit_text(&text, cols)))
                    .into_any_element(),
                TuiRow::Rule => line
                    .child(skin::rule(cell, cols, theme.separator, m))
                    .into_any_element(),
                TuiRow::Input(input) => {
                    // The runs are in the mono font, the grid's.
                    let (code, runs) = self.code(&input, &theme, cx);
                    line.w(cell.cols(cols))
                        .flex()
                        .bg(theme.background_secondary)
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(cell.cols(2))
                                .text_color(theme.text_tertiary)
                                .child("›"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(StyledText::new(code).with_runs(runs)),
                        )
                        .into_any_element()
                }
                TuiRow::Answer(answer) => {
                    let room = cols.saturating_sub(2);
                    let content = match &answer {
                        BlockRow::Value { type_text, value } => {
                            let type_text = type_text.as_deref().unwrap_or("");
                            let full = format!("{type_text}{value}");
                            let fitted = skin::fit_text(&full, room);
                            let cut = type_text.len().min(fitted.len());
                            let (t, v) = fitted.split_at(cut);
                            value_text(Some(t).filter(|t| !t.is_empty()), v, &theme)
                                .into_any_element()
                        }
                        BlockRow::Output(text) | BlockRow::Error(text) => div()
                            .text_color(block_row_color(&answer, &theme))
                            .child(SharedString::from(skin::fit_text(text, room)))
                            .into_any_element(),
                    };
                    line.pl(cell.cols(2)).child(content).into_any_element()
                }
                TuiRow::Meta(index, elapsed) => {
                    let (text, _) = tui_meta(&elapsed, cols);
                    let (lead, _) = text.split_at(text.len() - RERUN.len());
                    line.flex()
                        .child(
                            div()
                                .text_color(theme.text_tertiary)
                                .child(SharedString::from(lead.to_string())),
                        )
                        .child(
                            div()
                                .id(ElementId::NamedInteger(
                                    "console-rerun".into(),
                                    index as u64,
                                ))
                                .w(cell.cols(RERUN.len()))
                                .cursor_pointer()
                                .text_color(theme.link)
                                .child(RERUN)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.rerun(index, cx);
                                })),
                        )
                        .into_any_element()
                }
            };
            lines.push(el);
        }
        let output = tui_list(
            "console-output",
            &cx.entity(),
            |this: &mut DebugConsole| &mut this.tui_scroll,
            top,
            total,
            cols,
            visible,
            div().flex().flex_col().children(lines).into_any_element(),
            cell,
            &self.tui_bars,
        );
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
            .child(output)
            .child(skin::section(
                cell,
                cols,
                Vec::new(),
                theme.border,
                theme.island_background,
                m,
            ))
            .child(
                div()
                    .h(px(cell.h))
                    .w(cell.cols(cols))
                    .flex_shrink_0()
                    .bg(theme.input_bg)
                    .child(self.input.clone()),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_walks_back_and_returns_to_the_draft() {
        let mut h = History::default();
        assert_eq!(h.up("x"), None);
        h.push("a");
        h.push("b");
        h.push("b");
        h.push("  ");
        assert_eq!(h.up("dra"), Some("b".into()));
        assert_eq!(h.up("ignored"), Some("a".into()));
        assert_eq!(h.up(""), None);
        assert_eq!(h.down(), Some("b".into()));
        assert_eq!(h.down(), Some("dra".into()));
        assert_eq!(h.down(), None);
        h.push("c");
        assert_eq!(h.up(""), Some("c".into()));
    }

    fn line(kind: ConsoleKind, text: &str) -> ConsoleLine {
        ConsoleLine {
            kind,
            text: text.to_string().into(),
        }
    }

    fn evaluation(input: &str, answer: Option<EvalAnswer>) -> Evaluation {
        Evaluation {
            input: input.into(),
            command: !input.starts_with('?'),
            started: std::time::Instant::now(),
            elapsed: answer.as_ref().map(|_| std::time::Duration::from_millis(3)),
            answer,
        }
    }

    #[test]
    fn answers_make_rows() {
        assert_eq!(
            block_rows(&EvalAnswer::Value {
                type_name: Some("int".into()),
                value: "28".into()
            }),
            vec![BlockRow::Value {
                type_text: Some("{int} ".into()),
                value: "28".into()
            }]
        );
        assert_eq!(
            block_rows(&EvalAnswer::Value {
                type_name: None,
                value: "x".into()
            }),
            vec![BlockRow::Value {
                type_text: None,
                value: "x".into()
            }]
        );
        assert_eq!(
            block_rows(&EvalAnswer::Output("a\r\n  b\nerror: no\n".into())),
            vec![
                BlockRow::Output("a".into()),
                BlockRow::Output("  b".into()),
                BlockRow::Error("error: no".into()),
            ]
        );
        assert!(block_rows(&EvalAnswer::Output(String::new())).is_empty());
        assert_eq!(
            block_rows(&EvalAnswer::Error("bad".into())),
            vec![BlockRow::Error("bad".into())]
        );
    }

    #[test]
    fn blocks_sit_between_rules() {
        let console = [
            line(ConsoleKind::Console, "Console is in 'commands' mode"),
            line(ConsoleKind::Console, ""),
            line(ConsoleKind::Console, "Launching: /p\n"),
            line(ConsoleKind::Evaluation(0), "?total * 2"),
            line(ConsoleKind::Evaluation(1), "bt"),
            line(ConsoleKind::Console, "Process finished with exit code 0"),
        ];
        let evaluations = [
            evaluation(
                "?total * 2",
                Some(EvalAnswer::Value {
                    type_name: Some("int".into()),
                    value: "28".into(),
                }),
            ),
            evaluation("bt", None),
        ];
        let items = console_items(&console, &evaluations);
        let block0 = Block {
            index: 0,
            input: "?total * 2".into(),
            rows: vec![BlockRow::Value {
                type_text: Some("{int} ".into()),
                value: "28".into(),
            }],
            elapsed: Some("3ms".into()),
        };
        let block1 = Block {
            index: 1,
            input: "bt".into(),
            rows: Vec::new(),
            elapsed: None,
        };
        assert_eq!(
            items,
            vec![
                Item::Line(ConsoleKind::Console, "Console is in 'commands' mode".into()),
                Item::Line(ConsoleKind::Console, "".into()),
                Item::Line(ConsoleKind::Console, "Launching: /p".into()),
                Item::Rule,
                Item::Block(block0),
                Item::Rule,
                Item::Block(block1),
                Item::Rule,
                Item::Line(
                    ConsoleKind::Console,
                    "Process finished with exit code 0".into()
                ),
            ]
        );
        // A first block has no rule above it.
        let items = console_items(&console[3..4], &evaluations);
        assert!(matches!(items[0], Item::Block(_)));
        // The grid: a row per line, the block's input, answer and time.
        let rows = tui_rows(&console_items(&console[2..4], &evaluations));
        assert_eq!(
            rows,
            vec![
                TuiRow::Line(ConsoleKind::Console, "Launching: /p".into()),
                TuiRow::Rule,
                TuiRow::Input("?total * 2".into()),
                TuiRow::Answer(BlockRow::Value {
                    type_text: Some("{int} ".into()),
                    value: "28".into()
                }),
                TuiRow::Meta(0, "3ms".into()),
            ]
        );
        assert_eq!(tui_meta("3ms", 20), ("         3ms  rerun".into(), 14));
    }

    /// The input is highlighted as Rust, `?` and all: the literal keeps its
    /// colour as in the editor (Fleet's `2` coloured, batch 13).
    #[test]
    fn the_input_highlights_as_rust() {
        let spans = Highlighter::new().highlight(Language::Rust, b"?total * 2");
        assert!(spans.iter().any(|s| s.range == (9..10)), "{spans:?}");
        assert!(spans.iter().all(|s| s.range.start > 0), "{spans:?}");
    }
}
