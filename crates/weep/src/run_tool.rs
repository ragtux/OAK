//! The run tool (M6): one tab per started configuration in the bottom panel,
//! as batch 11 caught it: the command line in the secondary colour, the
//! process's output under it in the terminal's mono font, and a rerun
//! button at the top right (a stop button while the process runs, an
//! estimate). The process is `weep_run::Run`; its lines arrive through a
//! channel that wakes this view.

use crate::assets::icon;
use crate::settings::AppSettings;
use crate::skin::{self, Cell, Skin};
use crate::theme::{Metrics, Theme};
use crate::tooltip::Tooltip;
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use futures::StreamExt;
use gpui::{
    div, prelude::*, px, App, Context, EventEmitter, FocusHandle, Focusable, Render, ScrollHandle,
    SharedString, Task, Window,
};
use weep_run::{Command, Event, Run};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunViewEvent {
    /// The process ended with this code (`None`: a signal).
    Exited(Option<i32>),
}

pub struct RunView {
    name: String,
    /// The configuration's `type` (`command`, `cargo`, or `shell` for a
    /// typed command), for the tab's icon.
    kind: String,
    command: Command,
    run: Option<Run>,
    lines: Vec<SharedString>,
    exit: Option<Option<i32>>,
    error: Option<String>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    _pump: Option<Task<()>>,
    /// The tui skin's body in whole cells, as last laid out.
    tui_grid: std::rc::Rc<std::cell::Cell<(usize, usize)>>,
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
            tui_grid: Default::default(),
        };
        view.start(cx);
        view
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.exited())
    }

    /// The process ended with a non-zero code or could not start: the tab's
    /// icon takes the error badge.
    pub fn failed(&self) -> bool {
        self.error.is_some() || matches!(self.exit, Some(code) if code != Some(0))
    }

    /// Starts the command over in this tab (`run/rerun`, the button).
    pub fn restart(&mut self, command: Option<Command>, cx: &mut Context<Self>) {
        self.stop();
        if let Some(command) = command {
            self.command = command;
        }
        self.lines.clear();
        self.exit = None;
        self.error = None;
        self.start(cx);
    }

    pub fn stop(&mut self) {
        if let Some(run) = &self.run {
            run.kill();
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
                Event::Line(line) => self.lines.push(line.into()),
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
        let running = self.running();
        let command: SharedString = self.command.display.clone().into();
        let lines = self.lines.clone();
        let error = self.error.clone();
        let button = div()
            .id("run-button")
            .absolute()
            .top(px(m.run_tool_button_top))
            .right(px(m.run_tool_button_right))
            .size(px(m.run_tool_button))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(m.button_radius))
            .border_1()
            .border_color(theme.popup_border)
            .bg(theme.popup_bg)
            .cursor_pointer()
            .hover(|s| s.bg(theme.ghost_hovered))
            .map(|d| {
                if running {
                    d.tooltip(Tooltip::text("Stop"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.stop();
                            cx.notify();
                        }))
                        .child(icon("expui/run/stop", m.icon_size, theme.icon_primary))
                } else {
                    d.tooltip(Tooltip::text("Rerun"))
                        .on_click(cx.listener(|this, _, _, cx| this.restart(None, cx)))
                        .child(icon("expui/run/rerun", m.icon_size, theme.icon_primary))
                }
            });
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
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme.text_secondary)
                            .whitespace_nowrap()
                            .child(command),
                    )
                    .child(div().h(px(m.run_tool_output_gap)).flex_shrink_0())
                    .children(error.map(|e| {
                        div()
                            .flex_shrink_0()
                            .text_color(theme.text_dangerous)
                            .child(format!("Could not start: {e}"))
                    }))
                    .children(lines.into_iter().map(|line| {
                        div()
                            .flex_shrink_0()
                            .text_color(theme.text_primary)
                            .whitespace_nowrap()
                            .child(line)
                    })),
            )
            .child(button)
            .into_any_element()
    }
}

impl RunView {
    /// The tool in the tui skin: row 1 is the command line in the secondary
    /// colour with `[ ▶ Rerun ] [ ■ Stop ]` at its right end, a rule with
    /// the state set in (`running`, `exit 0`), then the output on the grid.
    fn render_tui(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let cols = self.tui_grid.get().0;
        let running = self.running();
        let (state, state_color) = match (running, self.exit, &self.error) {
            (true, _, _) => ("running".to_string(), theme.text_secondary),
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
                this.stop();
                cx.notify();
            }));
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
                    .child(SharedString::from(self.command.display.clone())),
            )
            .child(gap())
            .child(rerun)
            .child(gap())
            .child(stop);
        let lines = self.lines.clone();
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
            .child(
                div()
                    .id("run-output")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .children(self.error.clone().map(|e| {
                        div()
                            .flex_shrink_0()
                            .text_color(theme.text_dangerous)
                            .child(format!("Could not start: {e}"))
                    }))
                    .children(lines.into_iter().map(|line| {
                        div()
                            .h(px(cell.h))
                            .flex_shrink_0()
                            .text_color(theme.text_primary)
                            .whitespace_nowrap()
                            .child(line)
                    })),
            )
            .into_any_element()
    }
}
