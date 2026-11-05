//! The run tool (M6): one tab per started configuration in the bottom panel,
//! as batch 11 caught it: the command line in the secondary colour, the
//! process's output under it in the terminal's mono font (stderr in the
//! terminal's ANSI red, batch 13), and a rerun button at the top right; while
//! the process runs, a stop button joins it on its left in one box (batch
//! 13). The process is `weep_run::Run`; its lines arrive through a channel
//! that wakes this view.
//!
//! M8: a debug session's tab (batch 13: Fleet shows a debugged
//! configuration's build here, under the configuration's name): the
//! session's `output` (the `cargo build` line, the build's text, the
//! adapter's command line, the debuggee's output), each command line after
//! the first opening a block; Stop stops the session, Rerun restarts it.

use crate::assets::icon;
use crate::debugger::{DebugSession, RunLineKind};
use crate::git_tool::tui_list;
use crate::settings::AppSettings;
use crate::skin::{self, Cell, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use crate::tooltip::Tooltip;
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use futures::StreamExt;
use gpui::{
    div, prelude::*, px, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla, Render,
    ScrollHandle, SharedString, Subscription, Task, Window,
};
use weep_run::{Command, Event, Run};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunViewEvent {
    /// The process ended with this code (`None`: a signal).
    Exited(Option<i32>),
    /// M8: Rerun on a debug session's tab: the workspace restarts it.
    DebugRestart(Entity<DebugSession>),
}

/// One row of the tool's text.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    kind: RunLineKind,
    text: SharedString,
}

pub struct RunView {
    name: String,
    /// The configuration's `type` (`command`, `cargo`, or `shell` for a
    /// typed command), for the tab's icon.
    kind: String,
    command: Command,
    run: Option<Run>,
    /// The output, stderr's lines marked.
    lines: Vec<(SharedString, bool)>,
    exit: Option<Option<i32>>,
    error: Option<String>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    _pump: Option<Task<()>>,
    /// M8: the debug session this tab shows, in place of a process.
    debug: Option<Entity<DebugSession>>,
    _debug_observer: Option<Subscription>,
    /// The tui skin's body in whole cells, as last laid out.
    tui_grid: std::rc::Rc<std::cell::Cell<(usize, usize)>>,
    /// The lists' scroll bars on the island's frame (the tui skin).
    tui_bars: crate::skin::FrameBars,
    /// The tui skin's first visible output row (rule 6: whole rows).
    tui_scroll: RowScroll,
}

impl EventEmitter<RunViewEvent> for RunView {}

impl Focusable for RunView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl RunView {
    pub fn new(name: String, kind: String, command: Command, cx: &mut Context<Self>) -> Self {
        let mut view = RunView {
            name,
            kind,
            command,
            run: None,
            lines: Vec::new(),
            exit: None,
            error: None,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _pump: None,
            debug: None,
            _debug_observer: None,
            tui_grid: Default::default(),
            tui_bars: Default::default(),
            tui_scroll: RowScroll::default(),
        };
        view.start(cx);
        view
    }

    /// M8: a debug session's tab.
    pub fn for_debug(
        name: String,
        kind: String,
        session: Entity<DebugSession>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = RunView {
            name,
            kind,
            command: Command::shell("", std::path::Path::new(".")),
            run: None,
            lines: Vec::new(),
            exit: None,
            error: None,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _pump: None,
            debug: None,
            _debug_observer: None,
            tui_grid: Default::default(),
            tui_bars: Default::default(),
            tui_scroll: RowScroll::default(),
        };
        view.set_debug(session, cx);
        view
    }

    /// Shows `session` in this tab (a debug run reusing a finished tab of
    /// the same name).
    pub fn set_debug(&mut self, session: Entity<DebugSession>, cx: &mut Context<Self>) {
        self.stop_process();
        self.run = None;
        self._pump = None;
        self.lines.clear();
        self.exit = None;
        self.error = None;
        self._debug_observer = Some(cx.observe(&session, |_, _, cx| cx.notify()));
        self.debug = Some(session);
        cx.notify();
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn running(&self, cx: &App) -> bool {
        match &self.debug {
            Some(session) => !session.read(cx).state.is_over(),
            None => self.run.as_ref().is_some_and(|r| !r.exited()),
        }
    }

    /// The process ended with a non-zero code or could not start (a debug
    /// session: [`DebugSession::failed`]): the tab's icon takes the error badge.
    pub fn failed(&self, cx: &App) -> bool {
        match &self.debug {
            Some(session) => session.read(cx).failed(),
            None => self.error.is_some() || matches!(self.exit, Some(code) if code != Some(0)),
        }
    }

    /// Starts the command over in this tab (`run/rerun`, the button); a
    /// debug session's tab asks the workspace to restart the session, or
    /// becomes a process's tab when given a command.
    pub fn restart(&mut self, command: Option<Command>, cx: &mut Context<Self>) {
        if command.is_none() {
            if let Some(session) = self.debug.clone() {
                cx.emit(RunViewEvent::DebugRestart(session));
                return;
            }
        }
        self.stop(cx);
        self.debug = None;
        self._debug_observer = None;
        if let Some(command) = command {
            self.command = command;
        }
        self.lines.clear();
        self.exit = None;
        self.error = None;
        self.start(cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        match self.debug.clone() {
            Some(session) => session.update(cx, |s, cx| s.stop(cx)),
            None => self.stop_process(),
        }
    }

    fn stop_process(&self) {
        if let Some(run) = &self.run {
            run.kill();
        }
    }

    /// The rows the tool shows: a process's command line, its start error
    /// and output; or a debug session's `output`.
    fn rows(&self, cx: &App) -> Vec<Row> {
        if let Some(session) = &self.debug {
            return session
                .read(cx)
                .output
                .iter()
                .map(|l| Row {
                    kind: l.kind,
                    text: l.text.clone(),
                })
                .collect();
        }
        let mut rows = vec![Row {
            kind: RunLineKind::Command,
            text: self.command.display.clone().into(),
        }];
        rows.extend(self.error.iter().map(|e| Row {
            kind: RunLineKind::Stderr,
            text: format!("Could not start: {e}").into(),
        }));
        rows.extend(self.lines.iter().map(|(text, err)| Row {
            kind: if *err {
                RunLineKind::Stderr
            } else {
                RunLineKind::Stdout
            },
            text: text.clone(),
        }));
        rows
    }

    /// A row's colour: command lines in `text.secondary`, stderr in the
    /// terminal's ANSI red (batch 13: cargo's "Finished" measured (209, 73,
    /// 79), `terminal.ansiColors.foreground.ansiRed` as WEEP's own shots
    /// render it), the rest in `text.primary`.
    fn row_color(kind: RunLineKind, theme: &Theme) -> Hsla {
        match kind {
            RunLineKind::Command => theme.text_secondary,
            RunLineKind::Stdout => theme.text_primary,
            RunLineKind::Stderr => theme.terminal_fg[1],
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = unbounded::<()>();
        let wake = move || {
            let _ = tx.unbounded_send(());
        };
        match Run::spawn(&self.command, wake) {
            Ok(run) => {
                self.run = Some(run);
                self._pump = Some(Self::pump(rx, cx));
            }
            Err(e) => {
                self.run = None;
                self.error = Some(e);
                self.exit = Some(None);
                cx.emit(RunViewEvent::Exited(None));
            }
        }
        cx.notify();
    }

    /// Drains the process's events whenever its threads wake the view.
    fn pump(mut rx: UnboundedReceiver<()>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while rx.next().await.is_some() {
                let go_on = this.update(cx, |view, cx| view.drain(cx));
                if !matches!(go_on, Ok(true)) {
                    break;
                }
            }
        })
    }

    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(run) = &mut self.run else {
            return false;
        };
        let events = run.poll();
        if events.is_empty() {
            return true;
        }
        let mut go_on = true;
        for event in events {
            match event {
                Event::Line(line) => self.lines.push((line.into(), false)),
                Event::Stderr(line) => self.lines.push((line.into(), true)),
                Event::Exited(code) => {
                    self.exit = Some(code);
                    cx.emit(RunViewEvent::Exited(code));
                    go_on = false;
                }
            }
        }
        cx.notify();
        go_on
    }
}

impl Render for RunView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(cx);
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let (font_size, line_height) = m.terminal_text(&cx.global::<AppSettings>().settings);
        let running = self.running(cx);
        let rows = self.rows(cx);
        let cell = |id: &'static str| {
            div()
                .id(id)
                .size(px(m.run_tool_button))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(m.button_radius))
                .cursor_pointer()
                .hover(|s| s.bg(theme.ghost_hovered))
        };
        // Batch 13: while the process runs, Stop (`icon.background.red`) and
        // Rerun share one 56 x 28 box at batch 11's place; Rerun alone after.
        let buttons = div()
            .absolute()
            .top(px(m.run_tool_button_top))
            .right(px(m.run_tool_button_right))
            .h(px(m.run_tool_button))
            .flex()
            .rounded(px(m.button_radius))
            .border_1()
            .border_color(theme.popup_border)
            .bg(theme.popup_bg)
            .when(running, |d| {
                d.child(
                    cell("run-stop")
                        .tooltip(Tooltip::text("Stop"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.stop(cx);
                            cx.notify();
                        }))
                        .child(icon("expui/run/stop", m.icon_size, theme.icon_red)),
                )
            })
            .child(
                cell("run-button")
                    .tooltip(Tooltip::text("Rerun"))
                    .on_click(cx.listener(|this, _, _, cx| this.restart(None, cx)))
                    .child(icon("expui/run/rerun", m.icon_size, theme.icon_primary)),
            );
        let mut body: Vec<gpui::AnyElement> = Vec::new();
        let mut after_command = false;
        for (ix, row) in rows.into_iter().enumerate() {
            let command = row.kind == RunLineKind::Command;
            if command && ix > 0 {
                // A new block (batch 13: the adapter's command line 54 under
                // the build's last row).
                body.push(
                    div()
                        .h(px(m.run_tool_block_gap))
                        .flex_shrink_0()
                        .into_any_element(),
                );
            } else if !command && after_command {
                body.push(
                    div()
                        .h(px(m.run_tool_output_gap))
                        .flex_shrink_0()
                        .into_any_element(),
                );
            }
            after_command = command;
            body.push(
                div()
                    .flex_shrink_0()
                    .text_color(Self::row_color(row.kind, &theme))
                    .whitespace_nowrap()
                    .child(row.text)
                    .into_any_element(),
            );
        }
        div()
            .id("run-tool")
            .key_context("RunTool")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .overflow_hidden()
            .font_family(theme.mono_font.clone())
            .text_size(px(font_size))
            .line_height(px(line_height))
            .child(
                div()
                    .id("run-output")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(m.run_tool_pad_x))
                    .pt(px(m.run_tool_command_top))
                    .flex()
                    .flex_col()
                    .children(body),
            )
            .child(buttons)
            .into_any_element()
    }
}

impl RunView {
    /// The tool in the tui skin: row 1 is the command line in the secondary
    /// colour with `[ ▶ Rerun ] [ ■ Stop ]` at its right end, a rule with
    /// the state set in (`running`, `exit 0`), then the output on the grid.
    /// The scroll bars this view's lists set on its island's frame.
    pub fn tui_bars(&self) -> crate::skin::FrameBars {
        self.tui_bars.clone()
    }

    fn render_tui(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        self.tui_bars.clear();
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let cols = self.tui_grid.get().0;
        let running = self.running(cx);
        let debug_state = self.debug.as_ref().map(|s| s.read(cx).state.clone());
        let (state, state_color) = match (running, self.exit, &self.error) {
            (true, _, _) => ("running".to_string(), theme.text_secondary),
            // M8: a debug session's end.
            (false, _, _) if debug_state.is_some() => match debug_state {
                Some(crate::debugger::SessionState::Exited(Some(0))) => {
                    ("exit 0".to_string(), theme.text_positive)
                }
                Some(crate::debugger::SessionState::Exited(Some(code))) => {
                    (format!("exit {code}"), theme.text_dangerous)
                }
                Some(crate::debugger::SessionState::Failed(_)) => {
                    ("failed".to_string(), theme.text_dangerous)
                }
                _ => ("stopped".to_string(), theme.text_dangerous),
            },
            (_, _, Some(_)) => ("could not start".to_string(), theme.text_dangerous),
            (_, Some(Some(0)), _) => ("exit 0".to_string(), theme.text_positive),
            (_, Some(Some(code)), _) => (format!("exit {code}"), theme.text_dangerous),
            (_, Some(None), _) => ("killed".to_string(), theme.text_dangerous),
            (false, None, None) => (String::new(), theme.text_secondary),
        };
        let gap = || div().w(px(cell.w)).flex_shrink_0();
        let rerun = skin::button("run-rerun", "▶ Rerun", false, &theme, cell)
            .when(running, |d| d.text_color(theme.text_disabled))
            .on_click(cx.listener(|this, _, _, cx| this.restart(None, cx)));
        let stop = skin::button("run-stop", "■ Stop", false, &theme, cell)
            .when(!running, |d| d.text_color(theme.text_disabled))
            .on_click(cx.listener(|this, _, _, cx| {
                this.stop(cx);
                cx.notify();
            }));
        // The first command line heads the tool; the rest of the rows (a
        // debug session's further command lines among them) run under the rule.
        let mut rows_all = self.rows(cx);
        let command_text = if rows_all
            .first()
            .is_some_and(|r| r.kind == RunLineKind::Command)
        {
            rows_all.remove(0).text
        } else {
            SharedString::default()
        };
        let command = div()
            .h(px(cell.h))
            .flex_shrink_0()
            .flex()
            .whitespace_nowrap()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(theme.text_secondary)
                    .child(command_text),
            )
            .child(gap())
            .child(rerun)
            .child(gap())
            .child(stop);
        // The output in whole rows under the command line and the rule.
        let visible = self.tui_grid.get().1.saturating_sub(2);
        let total = rows_all.len();
        self.tui_scroll.top = self.tui_scroll.top.min(total.saturating_sub(visible));
        let top = self.tui_scroll.top;
        let rows: Vec<gpui::AnyElement> = rows_all
            .into_iter()
            .map(|row| {
                div()
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .text_color(Self::row_color(row.kind, &theme))
                    .whitespace_nowrap()
                    .child(row.text)
                    .into_any_element()
            })
            .skip(top)
            .take(visible)
            .collect();
        let output = tui_list(
            "run-output",
            &cx.entity(),
            |this: &mut RunView| &mut this.tui_scroll,
            top,
            total,
            cols,
            visible,
            div().flex().flex_col().children(rows).into_any_element(),
            cell,
            &self.tui_bars,
        );
        div()
            .id("run-tool")
            .key_context("RunTool")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font_family(theme.mono_font.clone())
            .text_size(px(cell.font_size))
            .line_height(px(cell.h))
            .child(skin::measure(cell, &self.tui_grid))
            .child(command)
            .child(skin::section(
                cell,
                cols,
                if state.is_empty() {
                    Vec::new()
                } else {
                    vec![skin::title(state, state_color)]
                },
                theme.border,
                theme.island_background,
                m,
            ))
            .child(output)
            .into_any_element()
    }
}
