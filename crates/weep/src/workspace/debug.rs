//! Debug sessions in the workspace (M8): Fleet debugs a run configuration
//! (`.fleet/run.json`). A `cargo` row is built first with cargo's JSON
//! messages (`weep-run`'s `cargo` module picks the executable: a
//! `cargo run` row's binary, a `cargo test` row's test executable from
//! `cargo test --no-run`), its build output streamed into the session's run
//! tab (batch 13: a bottom-panel run tool tab named after the
//! configuration, with the adapter's command line after the build); then
//! the executable is launched under lldb-dap with the row's arguments,
//! environment and directory. A `command` row's program is launched
//! directly. The Debug tool opens in the right panel as the session starts
//! (batch 13), wherever the user put it otherwise; a session's first stop
//! opens the frame's file at its line and focuses the Debug tool (batch 13:
//! its tab selectedFocused on the stop).
//!
//! The `debug/*` actions the workspace answers: `debug/debug-in-context`
//! (the configuration at the caret in run.json, else the last debugged),
//! resume, pause, the steps, restart (`DebugRestart`, WEEP's own: Fleet's
//! Restart button has no known id) and stop on the active session, and the
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
use gpui::{EntityId, Subscription};
use std::collections::{HashMap, HashSet};
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
    /// Sessions whose first stop has focused the Debug tool.
    stopped_once: HashSet<EntityId>,
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
        self.start_debug_run_tab(
            session.clone(),
            config.name.clone(),
            config.kind.clone(),
            cx,
        );
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
        let session = self.new_debug_session(label.clone(), cx);
        self.start_debug_run_tab(session.clone(), label, "attach".into(), cx);
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
        self.follow_debug_session(&session, cx);
        session
    }

    /// The session listed by the Debugger and followed for its stops (again,
    /// after the Debug tool's close dropped it, for a restart from its run tab).
    fn follow_debug_session(&mut self, session: &Entity<DebugSession>, cx: &mut Context<Self>) {
        if let std::collections::hash_map::Entry::Vacant(e) =
            self.debug.sessions.entry(session.entity_id())
        {
            e.insert(cx.subscribe(session, Self::on_debug_session_event));
        }
        let debugger = Debugger::global(cx);
        if !debugger.read(cx).sessions().contains(session) {
            debugger.update(cx, |d, cx| d.add(session.clone(), cx));
        }
        cx.notify();
    }

    /// Restarts a session (its run tab's Rerun, `DebugRestart`): stopped and
    /// started again from the same build or launch, the tools on it.
    pub(super) fn restart_debug_session(
        &mut self,
        session: Entity<DebugSession>,
        cx: &mut Context<Self>,
    ) {
        self.follow_debug_session(&session, cx);
        let debugger = Debugger::global(cx);
        debugger.update(cx, |d, cx| {
            if let Some(ix) = d.sessions().iter().position(|s| *s == session) {
                d.set_active(ix, cx);
            }
        });
        // Its next stop focuses the Debug tool again.
        self.debug.stopped_once.remove(&session.entity_id());
        session.update(cx, |s, cx| s.restart(cx));
        self.reveal_debug_tool(cx);
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
                let first = self.debug.stopped_once.insert(session.entity_id());
                if let Some(path) = frame.as_ref().and_then(|f| f.path.clone()) {
                    if path.is_file() {
                        let frame = frame.as_ref().expect("checked");
                        self.open_path(path, false, cx);
                        if let Some(editor) = self.active_editor().cloned() {
                            // Estimate (batch 13, one shot): Fleet's caret at
                            // the stopped line's first column (6:1), not the
                            // frame's.
                            let line = frame.line;
                            editor.update(cx, |e, cx| e.goto(line, 0, cx));
                        }
                    }
                }
                // Batch 13: on the first stop the Debug tool's tab is the
                // focused one; otherwise the focus stays where it was (the
                // editor unfocused under its wash; estimates from one shot).
                if let Some((side, ix)) = self.reveal_debug_tool(cx) {
                    if first {
                        self.focus_tool_tab_later(side, ix, cx);
                    }
                }
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

    /// Shows the Debug tool without taking the focus: in the panel holding
    /// one, else in the right panel (batch 13; Fleet shows it as the
    /// session starts and on a stop). Returns where it is.
    fn reveal_debug_tool(&mut self, cx: &mut Context<Self>) -> Option<(Side, usize)> {
        let side = self
            .panels
            .iter()
            .position(|p| p.tools.iter().any(|t| t.tool == Tool::Debug))
            .map(|ix| Side::ALL[ix])
            .unwrap_or_else(|| workbench::tool_side(Tool::Debug));
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
        Some((side, ix))
    }

    /// Focuses the Debug tool once the frame is drawn (a session's event
    /// has no window at hand).
    fn focus_tool_tab_later(&mut self, side: Side, ix: usize, cx: &mut Context<Self>) {
        if self.panel(side).tools.get(ix).map(|t| t.tool) != Some(Tool::Debug) {
            return;
        }
        let handle = self.debug_views.debug.read(cx).focus_handle(cx);
        self.focus_later(handle, cx);
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
        self.debug.stopped_once.retain(|id| alive.contains(id));
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

    /// `DebugRestart` (the Processes row's and the floating panel's Restart,
    /// batch 13): the live session the steps act on, else the active one
    /// even when it has ended.
    pub(super) fn debug_restart(
        &mut self,
        _: &DebugRestart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self
            .command_session(cx)
            .or_else(|| Debugger::global(cx).read(cx).active());
        if let Some(session) = session {
            self.restart_debug_session(session, cx);
        }
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
        // `choose_tool` keeps a tab where the user put it.
        self.choose_tool(workbench::tool_side(tool), tool, window, cx);
    }

    /// The new-tool actions: another tab of the tool in its default panel
    /// (the right one for Debug and Breakpoints, batch 13; every tab of a
    /// tool shows the same view).
    fn new_debug_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        self.debug.next_tab += 1;
        let instance = self.debug.next_tab;
        let side = workbench::tool_side(tool);
        let panel = self.panel_mut(side);
        panel.tools.push(ToolTab { tool, instance });
        panel.active = panel.tools.len() - 1;
        panel.open = true;
        self.chooser = None;
        let ix = self.panel(side).active;
        self.focus_tool_tab(side, ix, window, cx);
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
