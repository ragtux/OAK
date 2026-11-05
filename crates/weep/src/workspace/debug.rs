//! Debug sessions in the workspace (M8): Fleet debugs a run configuration
//! (`.fleet/run.json`). A `cargo` row is built first with cargo's JSON
//! messages (`weep-run`'s `cargo` module picks the executable: a
//! `cargo run` row's binary, a `cargo test` row's test executable from
//! `cargo test --no-run`), its build output streamed into the session's
//! Debugger Console; then the executable is launched under lldb-dap with
//! the row's arguments, environment and directory. A `command` row's
//! program is launched directly. A stop opens the frame's file at its line
//! and shows the Debug tool in the bottom panel.
//!
//! The `debug/*` actions the workspace answers: `debug/debug-in-context`
//! (the configuration at the caret in run.json, else the last debugged),
//! resume, pause, the steps and stop on the active session, and the
//! jump-to-tool / new-tool actions of the Debug, Breakpoints and Debugger
//! Console tools. The breakpoint and watch actions belong to the editor
//! and the tools (their key contexts); `debugger/attach-to-process` opens
//! the floating debug panel's attach dialog.
//!
//! Ended sessions stay listed (the Debug tool shows the last one's console
//! and state) until a new session of the same configuration starts or the
//! last Debug tool tab closes (estimate: Fleet's rule is unprobed).

use super::*;
use crate::debugger::{DebugSession, DebugSessionEvent, Debugger, SessionState};
use crate::toolbar::DebugIndicator;
use gpui::{EntityId, Subscription};
use std::collections::HashMap;
use weep_dap::{AdapterCommand, LaunchRequest, Start};
use weep_run::cargo::DebugBuild;
use weep_run::RunConfig;

/// The workspace's side of the debugger.
#[derive(Default)]
pub(super) struct DebugState {
    /// The last configuration debugged, for `debug/debug-in-context`.
    last: Option<String>,
    /// The sessions this workspace started, followed for their stops.
    sessions: HashMap<EntityId, Subscription>,
    /// The next id for a `debug/new-tool` tab.
    next_tab: usize,
}

/// What Fleet's notification says when no adapter is installed (estimate:
/// Fleet ships its own debugger and never shows one).
fn missing_adapter_message() -> String {
    let how = if cfg!(target_os = "macos") {
        "It comes with Xcode or its command line tools (xcode-select --install)"
    } else if cfg!(windows) {
        "It comes with LLVM for Windows (winget install LLVM.LLVM)"
    } else {
        "It comes with LLDB (the lldb package of most distributions)"
    };
    format!("Debugging needs lldb-dap (or lldb-vscode) on PATH. {how}.")
}

impl Workspace {
    /// Debugs a configuration: builds a `cargo` row first, launches a
    /// `command` row's program directly.
    pub(super) fn debug_config(&mut self, config: &RunConfig, cx: &mut Context<Self>) {
        if !crate::run_popup::debuggable(&config.kind) {
            self.notify_error(
                "Cannot debug",
                format!(
                    "WEEP debugs 'cargo' and 'command' configurations, not '{}' ('{}')",
                    config.kind, config.name
                ),
                cx,
            );
            return;
        }
        let Some(adapter) = AdapterCommand::find_lldb() else {
            self.notify_error("No debug adapter found", missing_adapter_message(), cx);
            return;
        };
        let env_file = Self::settings(cx)
            .run_environment_file()
            .map(|f| self.root.join(f));
        let command = match config.command(&self.root, env_file.as_deref()) {
            Ok(c) => c,
            Err(e) => {
                self.notify_error("Cannot debug", e, cx);
                return;
            }
        };
        let build = if config.kind == "cargo" {
            match DebugBuild::from_cargo(&command) {
                Ok(b) => Some(b),
                Err(e) => {
                    self.notify_error("Cannot debug", e, cx);
                    return;
                }
            }
        } else {
            None
        };
        self.remember_run(&config.name);
        self.debug.last = Some(config.name.clone());
        let session = self.new_debug_session(config.name.clone(), cx);
        match build {
            Some(build) => {
                session.update(cx, |s, cx| s.build_then_start(build, adapter, cx));
            }
            None => {
                // A bare name is looked up on the row's PATH, else the app's.
                let path = command
                    .env
                    .iter()
                    .find(|(k, _)| k == "PATH")
                    .map(|(_, v)| std::ffi::OsString::from(v))
                    .or_else(|| std::env::var_os("PATH"));
                let start = Start::Launch(LaunchRequest {
                    program: command.program_path(path.as_deref()),
                    args: command.args.clone(),
                    cwd: command.cwd.clone(),
                    env: command.env.clone(),
                    stop_on_entry: false,
                });
                session.update(cx, |s, cx| s.start(&adapter, start, cx));
            }
        }
        self.reveal_debug_tool(cx);
    }

    /// `debugger/attach-to-process`'s answer: a session on a running process.
    fn debug_attach(&mut self, pid: u32, name: String, cx: &mut Context<Self>) {
        let Some(adapter) = AdapterCommand::find_lldb() else {
            self.notify_error("No debug adapter found", missing_adapter_message(), cx);
            return;
        };
        let label = if name.is_empty() {
            format!("pid {pid}")
        } else {
            format!("{name} ({pid})")
        };
        let session = self.new_debug_session(label, cx);
        session.update(cx, |s, cx| s.start(&adapter, Start::Attach { pid }, cx));
        self.reveal_debug_tool(cx);
    }

    /// The callback the attach dialog calls with the chosen process.
    fn debug_attach_callback(
        &self,
        cx: &mut Context<Self>,
    ) -> impl Fn(u32, String, &mut Window, &mut App) + 'static {
        let weak = cx.weak_entity();
        move |pid, name, _window, cx| {
            if let Some(this) = weak.upgrade() {
                this.update(cx, |this, cx| this.debug_attach(pid, name, cx));
            }
        }
    }

    /// A session added to the Debugger (an ended one of the same name gone)
    /// and followed for its stops.
    fn new_debug_session(&mut self, name: String, cx: &mut Context<Self>) -> Entity<DebugSession> {
        let debugger = Debugger::global(cx);
        debugger.update(cx, |d, cx| d.remove_ended(|s| s.name == name, cx));
        let root = self.root.clone();
        let session = cx.new(|_| {
            let mut s = DebugSession::new(name);
            s.set_root(&root);
            s
        });
        let subscription = cx.subscribe(&session, Self::on_debug_session_event);
        self.debug
            .sessions
            .insert(session.entity_id(), subscription);
        debugger.update(cx, |d, cx| d.add(session.clone(), cx));
        cx.notify();
        session
    }

    fn on_debug_session_event(
        &mut self,
        session: Entity<DebugSession>,
        event: &DebugSessionEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            DebugSessionEvent::Changed => cx.notify(),
            DebugSessionEvent::Stopped => {
                // The stopped session becomes the one the tools show.
                let debugger = Debugger::global(cx);
                debugger.update(cx, |d, cx| {
                    if let Some(ix) = d.sessions().iter().position(|s| *s == session) {
                        d.set_active(ix, cx);
                    }
                });
                let frame = {
                    let s = session.read(cx);
                    s.frames.get(s.current_frame).cloned()
                };
                if let Some(path) = frame.as_ref().and_then(|f| f.path.clone()) {
                    if path.is_file() {
                        let frame = frame.as_ref().expect("checked");
                        self.open_path(path, false, cx);
                        if let Some(editor) = self.active_editor().cloned() {
                            let (line, column) = (frame.line, frame.column);
                            editor.update(cx, |e, cx| e.goto(line, column, cx));
                        }
                        self.focus_active_editor(cx);
                    }
                }
                self.reveal_debug_tool(cx);
                cx.notify();
            }
            DebugSessionEvent::Ended => {
                let (name, state) = {
                    let s = session.read(cx);
                    (s.name.clone(), s.state.clone())
                };
                if let SessionState::Failed(message) = state {
                    // estimate: Fleet's wording; the console has the details.
                    self.notify_error(&format!("Debugging '{name}' failed"), message, cx);
                }
                cx.notify();
            }
        }
    }

    /// Shows the Debug tool in the bottom panel without taking the focus
    /// (estimate: Fleet shows it as the session starts and on a stop).
    fn reveal_debug_tool(&mut self, cx: &mut Context<Self>) {
        let side = Side::Bottom;
        let panel = self.panel_mut(side);
        let ix = match panel.tools.iter().position(|t| t.tool == Tool::Debug) {
            Some(ix) => ix,
            None => {
                panel.tools.push(ToolTab::single(Tool::Debug));
                panel.tools.len() - 1
            }
        };
        panel.active = ix;
        panel.open = true;
        self.save_layout();
        cx.notify();
    }

    /// A Debug tool tab closed: with none left, the ended sessions go.
    pub(super) fn debug_tab_closed(&mut self, cx: &mut Context<Self>) {
        let any_left = self
            .panels
            .iter()
            .any(|p| p.tools.iter().any(|t| t.tool == Tool::Debug));
        if any_left {
            return;
        }
        let debugger = Debugger::global(cx);
        debugger.update(cx, |d, cx| d.remove_ended(|_| true, cx));
        let alive: Vec<EntityId> = debugger
            .read(cx)
            .sessions()
            .iter()
            .map(|s| s.entity_id())
            .collect();
        self.debug.sessions.retain(|id, _| alive.contains(id));
    }

    /// The toolbar's Run & Debug button: whether a session is live, and
    /// whether one is stopped.
    pub(super) fn debug_indicator(&self, cx: &App) -> Option<DebugIndicator> {
        let debugger = Debugger::global(cx);
        let live = debugger.read(cx).live(cx);
        if live.is_empty() {
            return None;
        }
        if live.iter().any(|s| s.read(cx).state.is_stopped()) {
            Some(DebugIndicator::Paused)
        } else {
            Some(DebugIndicator::Running)
        }
    }

    /// The session the stepping actions act on: the active one when it is
    /// live, else the last live one.
    fn command_session(&self, cx: &App) -> Option<Entity<DebugSession>> {
        let debugger = Debugger::global(cx);
        let d = debugger.read(cx);
        d.active()
            .filter(|s| !s.read(cx).state.is_over())
            .or_else(|| d.live(cx).last().cloned())
    }

    fn with_session(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut DebugSession, &mut Context<DebugSession>),
    ) {
        if let Some(session) = self.command_session(cx) {
            session.update(cx, f);
        }
    }

    // ---- actions -----------------------------------------------------------------

    /// `debug/debug-in-context` (Alt+Shift+R): in run.json the configuration
    /// at the caret; elsewhere the last debugged again; with nothing to
    /// debug, the Run & Debug popup.
    pub(super) fn debug_in_context(
        &mut self,
        _: &DebugInContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.run_configuration_at_caret(cx) {
            let configs = self.run_configurations_or_notify(cx);
            if let Some(config) = configs.get(ix).cloned() {
                self.debug_config(&config, cx);
                return;
            }
        }
        if let Some(name) = self.debug.last.clone() {
            let configs = self.run_configurations_or_notify(cx);
            if let Some(config) = configs.into_iter().find(|c| c.name == name) {
                self.debug_config(&config, cx);
                return;
            }
        }
        self.open_run_popup(window, cx);
    }

    /// A gutter icon or the popup asked to debug configuration `ix`.
    pub(super) fn debug_configuration_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        let configs = self.run_configurations_or_notify(cx);
        if let Some(config) = configs.get(ix).cloned() {
            self.debug_config(&config, cx);
        }
    }

    pub(super) fn debug_resume(&mut self, _: &DebugResume, _: &mut Window, cx: &mut Context<Self>) {
        self.with_session(cx, |s, cx| s.resume(cx));
    }

    pub(super) fn debug_pause(&mut self, _: &DebugPause, _: &mut Window, cx: &mut Context<Self>) {
        self.with_session(cx, |s, cx| s.pause(cx));
    }

    pub(super) fn debug_step_over(
        &mut self,
        _: &DebugStepOver,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_session(cx, |s, cx| s.step_over(cx));
    }

    pub(super) fn debug_step_into(
        &mut self,
        _: &DebugStepInto,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_session(cx, |s, cx| s.step_into(cx));
    }

    /// `debug/force-step-into`: lldb has no "my code only" filter to force
    /// past, so it is Step Into.
    pub(super) fn debug_force_step_into(
        &mut self,
        _: &DebugForceStepInto,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_session(cx, |s, cx| s.step_into(cx));
    }

    pub(super) fn debug_step_out(
        &mut self,
        _: &DebugStepOut,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_session(cx, |s, cx| s.step_out(cx));
    }

    pub(super) fn debug_stop(&mut self, _: &DebugStop, _: &mut Window, cx: &mut Context<Self>) {
        self.with_session(cx, |s, cx| s.stop(cx));
    }

    /// `debug/quick-evaluate-expression` (bound in the Editor context, it
    /// bubbles here): the value popup at the active editor's expression.
    pub(super) fn debug_quick_evaluate(
        &mut self,
        _: &DebugQuickEvaluateExpression,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.active_editor().cloned() {
            self.debug_panel
                .update(cx, |p, cx| p.quick_evaluate(&editor, window, cx));
        }
    }

    /// `debugger/attach-to-process`: the attach dialog (agent E's, hosted
    /// by the floating debug panel).
    pub(super) fn debugger_attach_to_process(
        &mut self,
        _: &DebuggerAttachToProcess,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on_attach = self.debug_attach_callback(cx);
        self.debug_panel
            .update(cx, |p, cx| p.attach_to_process(on_attach, window, cx));
    }

    /// The jump-to-tool actions: the tool's tab (made when there is none),
    /// shown and focused.
    fn jump_to_debug_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let side = self
            .panels
            .iter()
            .position(|p| p.tools.iter().any(|t| t.tool == tool))
            .map(|ix| Side::ALL[ix])
            .unwrap_or(Side::Bottom);
        self.choose_tool(side, tool, window, cx);
    }

    /// The new-tool actions: another tab of the tool in the bottom panel
    /// (every tab of a tool shows the same view).
    fn new_debug_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        self.debug.next_tab += 1;
        let instance = self.debug.next_tab;
        let panel = self.panel_mut(Side::Bottom);
        panel.tools.push(ToolTab { tool, instance });
        panel.active = panel.tools.len() - 1;
        panel.open = true;
        self.chooser = None;
        let ix = self.panel(Side::Bottom).active;
        self.focus_tool_tab(Side::Bottom, ix, window, cx);
        self.save_layout();
        cx.notify();
    }

    pub(super) fn debug_jump_to_tool(
        &mut self,
        _: &DebugJumpToTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_debug_tool(Tool::Debug, window, cx);
    }

    pub(super) fn debug_new_tool(
        &mut self,
        _: &DebugNewTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_debug_tool(Tool::Debug, window, cx);
    }

    pub(super) fn debug_breakpoints_jump_to_tool(
        &mut self,
        _: &DebugBreakpointsJumpToTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_debug_tool(Tool::Breakpoints, window, cx);
    }

    pub(super) fn new_breakpoints_tool(
        &mut self,
        _: &NewBreakpointsTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_debug_tool(Tool::Breakpoints, window, cx);
    }

    pub(super) fn debug_console_jump_to_tool(
        &mut self,
        _: &DebugConsoleJumpToTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_debug_tool(Tool::DebuggerConsole, window, cx);
    }

    pub(super) fn debug_console_new_tool(
        &mut self,
        _: &DebugConsoleNewTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_debug_tool(Tool::DebuggerConsole, window, cx);
    }
}
