//! The Debugger Console tool (M8): the active session's console (the
//! adapter's messages, the "Process finished" line; the build's and the
//! debuggee's output go to the configuration's run tab) and the
//! expressions typed into it with their results, in the terminal's
//! mono font, with an input at the bottom whose Enter sends the text to
//! `DebugSession::evaluate_in_console` (Up and Down walk the typed history).
//! Its form is batch 13's (`debug-fleet-breakpoints-tool.png`, the bottom
//! panel): the lines 32.5 in from the island, wrapped at its width; the
//! input a 100 px band of `background.secondary` across the island's bottom
//! with a `text.tertiary` chevron at its left (`metrics::CONSOLE_*`). The
//! adapter's own lines are `text.primary` (batch 13); the rest are estimates
//! on Fleet's keys (stdout `text.primary`, stderr and errors
//! `text.dangerous`, results `text.secondary`, the input echoed after
//! `> `). The list sticks to its bottom while new lines arrive, unless
//! scrolled up. Key context: `DebuggerConsole`.

use crate::actions::{PopupDown, PopupUp};
use crate::assets::icon;
use crate::debug_tool::Follow;
use crate::debugger::{ConsoleKind, ConsoleLine, DebugSession};
use crate::git_tool::tui_list;
use crate::input::{InputEvent, TextInput};
use crate::settings::AppSettings;
use crate::skin::{self, Cell, FrameBars, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, AnyElement, App, Context, Entity, FocusHandle, Focusable, Hsla, Render,
    ScrollHandle, SharedString, Window,
};
use std::rc::Rc;

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

/// A console line as printed: the input after `> `.
pub fn line_text(line: &ConsoleLine) -> String {
    match line.kind {
        ConsoleKind::Input => format!("> {}", line.text),
        _ => line.text.to_string(),
    }
}

fn line_color(kind: ConsoleKind, theme: &Theme) -> Hsla {
    match kind {
        // Batch 13: the adapter's own lines ("Launching: …") in `text.primary`.
        ConsoleKind::Stdout | ConsoleKind::Input | ConsoleKind::Console => theme.text_primary,
        ConsoleKind::Stderr | ConsoleKind::Error => theme.text_dangerous,
        ConsoleKind::Result => theme.text_secondary,
    }
}

/// The console's lines split on their line breaks, one row each.
fn rows_of(console: &[ConsoleLine]) -> Vec<(ConsoleKind, String)> {
    let mut rows = Vec::new();
    for line in console {
        let text = line_text(line);
        let text = text.strip_suffix('\n').unwrap_or(&text);
        for part in text.split('\n') {
            rows.push((line.kind, part.trim_end_matches('\r').to_string()));
        }
    }
    rows
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
            self.stick = true;
            if let Some(session) = self.session() {
                session.update(cx, |s, cx| s.evaluate_in_console(text, cx));
            }
            cx.notify();
        }
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

    fn rows(&self, cx: &App) -> Vec<(ConsoleKind, String)> {
        match &self.follow.session {
            Some(s) => rows_of(&s.read(cx).console),
            None => Vec::new(),
        }
    }
}

impl Render for DebugConsole {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        let body = if Skin::current(cx) == Skin::Tui {
            self.render_tui(rows, window, cx)
        } else {
            self.render_fleet(rows, window, cx)
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
        rows: Vec<(ConsoleKind, String)>,
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
        if rows.len() != self.drawn {
            if self.stick || at_bottom {
                self.scroll.scroll_to_bottom();
            }
            self.stick = false;
            self.drawn = rows.len();
        }
        let lines: Vec<AnyElement> = rows
            .into_iter()
            .map(|(kind, text)| {
                // Batch 13: a long line wraps at the island's width.
                div()
                    .min_h(px(line_height))
                    .flex_shrink_0()
                    .text_color(line_color(kind, &theme))
                    .child(SharedString::from(text))
                    .into_any_element()
            })
            .collect();
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
                    .pl(px(m.console_pad_x))
                    .pr(px(m.console_pad_right))
                    .font_family(theme.mono_font.clone())
                    .text_size(px(font_size))
                    .line_height(px(line_height))
                    .flex()
                    .flex_col()
                    .children(lines),
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

    /// The console in the tui skin: the lines in whole rows, a rule, and the
    /// field as the last row (`› expression▏` while focused).
    fn render_tui(
        &mut self,
        rows: Vec<(ConsoleKind, String)>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.tui_bars.clear();
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let (cols, grid_rows) = self.tui_grid.get();
        let visible = grid_rows.saturating_sub(2);
        let total = rows.len();
        let max_top = total.saturating_sub(visible);
        let was_at_bottom = self.tui_scroll.top + visible >= self.drawn;
        if total != self.drawn {
            if self.stick || was_at_bottom {
                self.tui_scroll.top = max_top;
            }
            self.stick = false;
            self.drawn = total;
        }
        self.tui_scroll.top = self.tui_scroll.top.min(max_top);
        let top = self.tui_scroll.top;
        let lines: Vec<AnyElement> = rows
            .into_iter()
            .skip(top)
            .take(visible)
            .map(|(kind, text)| {
                div()
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(line_color(kind, &theme))
                    .child(SharedString::from(text))
                    .into_any_element()
            })
            .collect();
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

    #[test]
    fn lines_print_their_kind() {
        let line = |kind, text: &str| ConsoleLine {
            kind,
            text: text.to_string().into(),
        };
        assert_eq!(line_text(&line(ConsoleKind::Input, "x + 1")), "> x + 1");
        assert_eq!(line_text(&line(ConsoleKind::Result, "2")), "2");
        let rows = rows_of(&[
            line(ConsoleKind::Stdout, "one\ntwo\n"),
            line(ConsoleKind::Stderr, "err\r\n"),
        ]);
        assert_eq!(
            rows,
            vec![
                (ConsoleKind::Stdout, "one".into()),
                (ConsoleKind::Stdout, "two".into()),
                (ConsoleKind::Stderr, "err".into()),
            ]
        );
    }
}
