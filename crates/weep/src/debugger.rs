//! The debugger's app-side state (M8): one [`DebugSession`] per started
//! debug run, each owning a `weep_dap::Client`, and the [`Debugger`] that
//! lists them and knows the active one. The Debug, Breakpoints and Debugger
//! Console tools, the floating debug panel and the editor (the execution
//! point, quick evaluate) read these entities and call their commands; the
//! workspace (`workspace/debug.rs`) starts sessions from run configurations
//! and routes the `debug/*` actions here.
//!
//! A session's life: `Starting` (a `cargo` row's build streams into the
//! console first, then the adapter starts and gets every file's enabled
//! breakpoints), `Running` once the adapter reports the configuration done,
//! `Stopped` on a stop (threads, the stopped thread's stack, frame 0's
//! scopes and the first cheap scope's variables are asked in turn, and the
//! watches re-evaluated), back to `Running` on resume or a step, and
//! `Exited` when the debuggee ends or the adapter goes (`Failed` when it
//! never started). The client is drained on the foreground through a wake
//! channel, as `workspace/smart.rs` drains rust-analyzer. What needs no gpui
//! (the event and response handling) is in plain methods returning
//! [`Effects`], tested below.
//!
//! Lines are 0-based.

use crate::breakpoints::{BreakpointStore, BreakpointsEvent, LineBreakpoint};
use futures::channel::{mpsc::unbounded, oneshot};
use futures::StreamExt;
use gpui::{
    App, AppContext, Context, Entity, EntityId, EventEmitter, Global, SharedString, Subscription,
    Task,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use weep_dap::{AdapterCommand, Client, Event, RequestId, Response, SourceBreakpoint, Start};
pub use weep_dap::{EvalContext, OutputCategory, StackFrame, Thread, Variable};

/// Where the debuggee is stopped: the editor paints this line with
/// `debug.currentFrame` and the execution arrow in its gutter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionPoint {
    pub path: PathBuf,
    pub line: usize,
    /// The top frame (the line the debuggee will run next), not a caller
    /// selected in the frames list.
    pub top: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// The adapter is starting or the configuration is being sent.
    Starting,
    Running,
    Stopped {
        reason: String,
        description: Option<String>,
    },
    /// The debuggee exited (its code when the adapter gave one).
    Exited(Option<i64>),
    Failed(String),
}

impl SessionState {
    pub fn is_stopped(&self) -> bool {
        matches!(self, SessionState::Stopped { .. })
    }

    pub fn is_over(&self) -> bool {
        matches!(self, SessionState::Exited(_) | SessionState::Failed(_))
    }
}

/// One line of the Debugger Console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleLine {
    pub kind: ConsoleKind,
    pub text: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsoleKind {
    /// The debuggee's stdout.
    Stdout,
    /// The debuggee's stderr.
    Stderr,
    /// The adapter's own messages.
    Console,
    /// An expression the user typed.
    Input,
    /// An expression's value.
    Result,
    /// An expression that failed, or a session error.
    Error,
}

/// A watch expression and its value in the selected frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    pub expression: String,
    /// `None` until evaluated (or with no stopped frame).
    pub value: Option<Result<Variable, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugSessionEvent {
    /// Anything the views show changed.
    Changed,
    /// The debuggee stopped; the workspace opens the frame's file.
    Stopped,
    /// The session ended.
    Ended,
}

/// What a request was for, so its answer lands in the right place.
#[derive(Clone, Debug, PartialEq)]
enum Pending {
    /// The lines sent, in order (the adapter answers in the same order).
    SetBreakpoints(PathBuf, Vec<usize>),
    Threads,
    StackTrace(i64),
    Scopes(i64),
    Variables(i64),
    /// A watch's value in this frame.
    Watch {
        expression: String,
        frame_id: Option<i64>,
    },
    /// A line typed into the Debugger Console.
    Console(String),
    /// [`DebugSession::evaluate`]'s, answered through its task.
    Eval(String),
    /// resume, pause, a step: named for an error line.
    Command(&'static str),
}

/// A request the handling of an event or an answer asks to send.
#[derive(Clone, Debug, PartialEq)]
enum Ask {
    Threads,
    StackTrace(i64),
    Scopes(i64),
    Variables(i64),
    Watch(String, Option<i64>),
    Pause(i64),
}

/// What handling an event or an answer leaves to do with gpui at hand.
#[derive(Debug, Default, PartialEq)]
struct Effects {
    asks: Vec<Ask>,
    /// Files whose breakpoints go to the adapter (again).
    resend: Vec<PathBuf>,
    /// Files whose verdicts changed (written to the BreakpointStore).
    verdicts: Vec<PathBuf>,
    /// The frames of a fresh stop arrived: the workspace shows the line.
    stopped: bool,
    /// The session is over.
    ended: bool,
}

impl Effects {
    fn ask(ask: Ask) -> Effects {
        Effects {
            asks: vec![ask],
            ..Effects::default()
        }
    }
}

/// A file's enabled breakpoints as `setBreakpoints` sends them.
pub fn source_breakpoints(list: &[LineBreakpoint]) -> Vec<SourceBreakpoint> {
    list.iter()
        .filter(|b| b.enabled)
        .map(|b| SourceBreakpoint {
            line: b.line,
            condition: b.condition.clone().filter(|c| !c.trim().is_empty()),
            log_message: b.log_message.clone().filter(|m| !m.is_empty()),
        })
        .collect()
}

/// The verdicts for a file's enabled breakpoints (`lines`, in order) from
/// what the adapter said per line; they stop at the first line it has not
/// answered for yet (sent after, or moved), which then reads as unknown.
fn verdicts(lines: &[usize], known: &HashMap<usize, bool>) -> Vec<bool> {
    lines
        .iter()
        .map_while(|line| known.get(line).copied())
        .collect()
}

/// A canonical path the adapter gave, under one of `roots`' canonical
/// spellings, in the root's own spelling.
fn respell(path: PathBuf, roots: &[(PathBuf, PathBuf)]) -> PathBuf {
    for (canonical, spelled) in roots {
        if let Ok(rest) = path.strip_prefix(canonical) {
            return spelled.join(rest);
        }
    }
    path
}

/// Whether two spellings name one file (`/var/x` and `/private/var/x`).
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
}

/// Appends an output chunk and takes off the complete lines (an adapter
/// sends output in pieces, a line's end not always in the same piece).
fn split_output(partial: &mut String, text: &str) -> Vec<String> {
    partial.push_str(text);
    let mut lines = Vec::new();
    while let Some(ix) = partial.find('\n') {
        lines.push(partial[..ix].trim_end_matches('\r').to_string());
        partial.drain(..=ix);
    }
    lines
}

fn console_kind(category: OutputCategory) -> ConsoleKind {
    match category {
        OutputCategory::Stdout => ConsoleKind::Stdout,
        OutputCategory::Stderr => ConsoleKind::Stderr,
        OutputCategory::Console | OutputCategory::Other => ConsoleKind::Console,
    }
}

/// The console's line when the debuggee exits (estimate: the run tool
/// prints no exit line, batch 11; IntelliJ's debugger console does, in
/// these words).
fn exit_line(code: i64) -> String {
    format!("Process finished with exit code {code}")
}

/// The console's cap, oldest lines dropped first (estimate: the run tool
/// keeps everything; a debuggee in a loop can print without end).
const CONSOLE_LIMIT: usize = 20_000;

pub struct DebugSession {
    /// The configuration's name, as the tab and the panel show it.
    pub name: String,
    pub state: SessionState,
    pub threads: Vec<Thread>,
    /// The thread the frames list shows (the stopped one by default).
    pub current_thread: Option<i64>,
    /// The current thread's frames, top first.
    pub frames: Vec<StackFrame>,
    /// The selected frame, an index into `frames`.
    pub current_frame: usize,
    /// The selected frame's scopes: name and variables reference.
    pub scopes: Vec<weep_dap::Scope>,
    /// Children fetched so far, by variables reference.
    pub children: HashMap<i64, Vec<Variable>>,
    /// The references shown expanded in the variables tree.
    pub expanded: HashSet<i64>,
    pub watches: Vec<Watch>,
    pub console: Vec<ConsoleLine>,
    /// Attached to a running process rather than launched (`debugger/attach-to-process`).
    pub attached: bool,
    client: Option<Client>,
    pending: HashMap<RequestId, Pending>,
    evals: HashMap<RequestId, oneshot::Sender<Result<Variable, String>>>,
    /// The lines `Client::start` sent per file; their answers come with ids
    /// this side never saw.
    initial: HashMap<PathBuf, Vec<usize>>,
    /// The adapter's verdict per file and line.
    verified: HashMap<PathBuf, HashMap<usize, bool>>,
    /// The adapter's breakpoint ids, to place its later `breakpoint` events.
    breakpoint_ids: HashMap<i64, (PathBuf, usize)>,
    /// Files to send once the configuration is done (none in practice: the
    /// client queues early requests).
    unsent: BTreeSet<PathBuf>,
    /// The workspace's root as canonical and as spelled: the adapter answers
    /// with canonical paths (`/private/var/...`), the editors and the store
    /// use the workspace's spelling.
    roots: Vec<(PathBuf, PathBuf)>,
    /// Every file whose breakpoints were sent: its canonical path to its
    /// BreakpointStore key (the document's path, not canonicalised).
    keys: HashMap<PathBuf, PathBuf>,
    /// Threads the adapter reported started (`threads` is only asked while stopped).
    started_threads: Vec<i64>,
    /// The last error a launch reported, for `Failed` when the adapter ends
    /// before the debuggee ran.
    last_error: Option<String>,
    /// Output not yet ended by a newline, per kind.
    partial: Vec<(ConsoleKind, String)>,
    /// A stop whose frames have not arrived: their arrival is the Stopped event.
    reveal: bool,
    /// [`DebugSession::finish`] ran: later events change nothing.
    ended: bool,
    /// A `cargo` row's build, before the adapter starts.
    build: Option<weep_run::Run>,
    _pump: Option<Task<()>>,
    _build_pump: Option<Task<()>>,
    _breakpoints: Option<Subscription>,
}

impl EventEmitter<DebugSessionEvent> for DebugSession {}

impl Drop for DebugSession {
    fn drop(&mut self) {
        if let Some(client) = &mut self.client {
            client.kill();
        }
        if let Some(build) = &self.build {
            build.kill();
        }
    }
}

impl DebugSession {
    pub fn new(name: String) -> DebugSession {
        DebugSession {
            name,
            state: SessionState::Starting,
            threads: Vec::new(),
            current_thread: None,
            frames: Vec::new(),
            current_frame: 0,
            scopes: Vec::new(),
            children: HashMap::new(),
            expanded: HashSet::new(),
            watches: Vec::new(),
            console: Vec::new(),
            attached: false,
            client: None,
            pending: HashMap::new(),
            evals: HashMap::new(),
            initial: HashMap::new(),
            verified: HashMap::new(),
            breakpoint_ids: HashMap::new(),
            unsent: BTreeSet::new(),
            roots: Vec::new(),
            keys: HashMap::new(),
            started_threads: Vec::new(),
            last_error: None,
            partial: Vec::new(),
            reveal: false,
            ended: false,
            build: None,
            _pump: None,
            _build_pump: None,
            _breakpoints: None,
        }
    }

    /// The selected frame's place, for the editor and the frames list.
    pub fn execution_point(&self) -> Option<ExecutionPoint> {
        let frame = self.frames.get(self.current_frame)?;
        if !self.state.is_stopped() {
            return None;
        }
        Some(ExecutionPoint {
            path: frame.path.clone()?,
            line: frame.line,
            top: self.current_frame == 0,
        })
    }

    /// The selected frame's id, for evaluations.
    pub fn frame_id(&self) -> Option<i64> {
        if !self.state.is_stopped() {
            return None;
        }
        self.frames.get(self.current_frame).map(|f| f.id)
    }

    /// The workspace's root, to give the adapter's canonical paths back in
    /// its spelling.
    pub fn set_root(&mut self, root: &Path) {
        if let Ok(canonical) = root.canonicalize() {
            if canonical != root {
                self.roots.push((canonical, root.to_path_buf()));
            }
        }
    }

    // ---- starting -----------------------------------------------------------------

    /// Starts `adapter` on `start` with every file's enabled breakpoints,
    /// and follows the BreakpointStore from then on.
    pub fn start(&mut self, adapter: &AdapterCommand, start: Start, cx: &mut Context<Self>) {
        if self.state.is_over() {
            return;
        }
        self.attached = matches!(start, Start::Attach { .. });
        self.follow_breakpoints(cx);
        let store = BreakpointStore::global(cx);
        let breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)> = store
            .read(cx)
            .files()
            .map(|(path, list)| (path.to_path_buf(), source_breakpoints(list)))
            .filter(|(_, list)| !list.is_empty())
            .collect();
        self.initial = breakpoints
            .iter()
            .map(|(path, list)| (path.clone(), list.iter().map(|b| b.line).collect()))
            .collect();
        for (path, _) in &breakpoints {
            self.remember_key(path);
        }
        // Whatever changes from here on is sent once the configuration is done.
        self.unsent.clear();
        let (tx, mut rx) = unbounded::<()>();
        match Client::start(adapter, start, breakpoints, move || {
            let _ = tx.unbounded_send(());
        }) {
            Ok(client) => {
                self.client = Some(client);
                self._pump = Some(cx.spawn(async move |this, cx| {
                    while rx.next().await.is_some() {
                        if this.update(cx, |this, cx| this.drain(cx)).is_err() {
                            break;
                        }
                    }
                }));
                self.drain(cx);
            }
            Err(e) => self.fail(e, cx),
        }
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }

    /// Builds a `cargo` row with cargo's JSON messages, the build's text in
    /// the console, then launches the picked executable under `adapter`.
    pub fn build_then_start(
        &mut self,
        build: weep_run::cargo::DebugBuild,
        adapter: AdapterCommand,
        cx: &mut Context<Self>,
    ) {
        self.follow_breakpoints(cx);
        self.push_line(ConsoleKind::Console, build.command.display.clone());
        let (tx, mut rx) = unbounded::<()>();
        let run = match weep_run::Run::spawn(&build.command, move || {
            let _ = tx.unbounded_send(());
        }) {
            Ok(run) => run,
            Err(e) => {
                self.fail(format!("cargo could not start: {e}"), cx);
                return;
            }
        };
        self.build = Some(run);
        let mut artifacts = Vec::new();
        self._build_pump = Some(cx.spawn(async move |this, cx| {
            while rx.next().await.is_some() {
                let done = this.update(cx, |this, cx| {
                    let events = match this.build.as_mut() {
                        Some(run) => run.poll(),
                        None => return true,
                    };
                    let mut done = false;
                    for event in events {
                        match event {
                            weep_run::Event::Line(line) => {
                                match weep_run::cargo::parse_message(&line) {
                                    Some(weep_run::cargo::Message::Artifact(a)) => {
                                        artifacts.push(a)
                                    }
                                    Some(_) => {}
                                    None => this.push_line(ConsoleKind::Console, line),
                                }
                            }
                            weep_run::Event::Exited(code) => {
                                this.build = None;
                                done = true;
                                this.build_finished(code, &build, &artifacts, &adapter, cx);
                            }
                        }
                    }
                    cx.emit(DebugSessionEvent::Changed);
                    cx.notify();
                    done
                });
                if !matches!(done, Ok(false)) {
                    break;
                }
            }
        }));
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }

    fn build_finished(
        &mut self,
        code: Option<i32>,
        build: &weep_run::cargo::DebugBuild,
        artifacts: &[weep_run::cargo::Artifact],
        adapter: &AdapterCommand,
        cx: &mut Context<Self>,
    ) {
        if self.state.is_over() {
            return;
        }
        if code != Some(0) {
            let message = match code {
                Some(code) => format!("The build failed (exit code {code})"),
                None => "The build was stopped".to_string(),
            };
            self.fail(message, cx);
            return;
        }
        match weep_run::cargo::pick(artifacts, build.target, &build.selector) {
            Ok(picked) => {
                if !picked.others.is_empty() {
                    // estimate: Fleet's wording unknown; cargo would run them all.
                    self.push_line(
                        ConsoleKind::Console,
                        format!(
                            "Debugging {}; not debugged: {}",
                            picked.artifact.label(),
                            picked.others.join(", ")
                        ),
                    );
                }
                let (cwd, env) = weep_run::cargo::launch_place(build, &picked);
                let start = Start::Launch(weep_dap::LaunchRequest {
                    program: picked.executable.clone(),
                    args: build.program_args.clone(),
                    cwd,
                    env,
                    stop_on_entry: false,
                });
                self.start(adapter, start, cx);
            }
            Err(e) => self.fail(e, cx),
        }
    }

    /// The session could not start: the message in the console, the state
    /// `Failed`, and the session over.
    pub fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        if self.state.is_over() {
            return;
        }
        self.push_line(ConsoleKind::Error, message.clone());
        self.state = SessionState::Failed(message);
        let effects = self.finish();
        self.apply(effects, cx);
    }

    fn follow_breakpoints(&mut self, cx: &mut Context<Self>) {
        if self._breakpoints.is_some() {
            return;
        }
        let store = BreakpointStore::global(cx);
        self._breakpoints = Some(
            cx.subscribe(&store, |this, _, event: &BreakpointsEvent, cx| {
                let BreakpointsEvent::Changed(path) = event;
                this.breakpoints_changed(path.clone(), cx);
            }),
        );
    }

    fn breakpoints_changed(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.state.is_over() {
            return;
        }
        // No client yet (a build runs): `start` reads the store. The client
        // queues a `setBreakpoints` made before `initialized` and sends it
        // ahead of `configurationDone`.
        if self.client.is_none() {
            return;
        }
        self.send_breakpoints(&path, cx);
    }

    fn send_breakpoints(&mut self, path: &Path, cx: &mut Context<Self>) {
        let list = source_breakpoints(BreakpointStore::global(cx).read(cx).in_file(path));
        let Some(client) = self.client.as_mut() else {
            return;
        };
        let id = client.set_breakpoints(path, &list);
        let lines = list.iter().map(|b| b.line).collect();
        self.pending
            .insert(id, Pending::SetBreakpoints(path.to_path_buf(), lines));
        self.remember_key(path);
    }

    fn remember_key(&mut self, key: &Path) {
        if let Ok(canonical) = key.canonicalize() {
            self.keys.insert(canonical, key.to_path_buf());
        }
    }

    /// A path the adapter gave (canonical) as the BreakpointStore's key for
    /// that file, when breakpoints were sent for it; else in the
    /// workspace root's spelling.
    fn store_key(&self, path: PathBuf) -> PathBuf {
        if let Some(key) = self.keys.get(&path) {
            return key.clone();
        }
        if let Some(key) = path.canonicalize().ok().and_then(|c| self.keys.get(&c)) {
            return key.clone();
        }
        respell(path, &self.roots)
    }

    // ---- the client's events --------------------------------------------------------

    fn drain(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.as_mut() else {
            return;
        };
        let events = client.poll();
        if events.is_empty() {
            return;
        }
        for event in events {
            if let Event::Response { id, result } = &event {
                if let Some(tx) = self.evals.remove(id) {
                    let expression = match self.pending.remove(id) {
                        Some(Pending::Eval(e)) => e,
                        _ => String::new(),
                    };
                    let _ = tx.send(evaluation(expression, result.clone()));
                    continue;
                }
            }
            let effects = self.on_event(event);
            self.apply(effects, cx);
            if self.client.is_none() {
                break;
            }
        }
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }

    /// Sends what the effects ask and tells the store, the views and the
    /// workspace.
    fn apply(&mut self, effects: Effects, cx: &mut Context<Self>) {
        for ask in effects.asks {
            self.send(ask);
        }
        for path in effects.resend {
            self.send_breakpoints(&path, cx);
        }
        if !effects.verdicts.is_empty() {
            let store = BreakpointStore::global(cx);
            for path in effects.verdicts {
                let lines: Vec<usize> = store
                    .read(cx)
                    .in_file(&path)
                    .iter()
                    .filter(|b| b.enabled)
                    .map(|b| b.line)
                    .collect();
                let known = self.verified.get(&path).cloned().unwrap_or_default();
                let v = verdicts(&lines, &known);
                store.update(cx, |s, cx| s.set_verified(&path, Some(&v), cx));
            }
        }
        if effects.ended {
            let store = BreakpointStore::global(cx);
            let paths: Vec<PathBuf> = store
                .read(cx)
                .files()
                .map(|(p, _)| p.to_path_buf())
                .collect();
            store.update(cx, |s, cx| {
                for path in paths {
                    s.set_verified(&path, None, cx);
                }
            });
            if let Some(mut client) = self.client.take() {
                client.kill();
            }
            if let Some(build) = self.build.take() {
                build.kill();
            }
            for (_, tx) in self.evals.drain() {
                let _ = tx.send(Err("the debug session ended".into()));
            }
            self.pending.clear();
            cx.emit(DebugSessionEvent::Changed);
            cx.emit(DebugSessionEvent::Ended);
        }
        if effects.stopped {
            cx.emit(DebugSessionEvent::Stopped);
        }
        cx.notify();
    }

    fn send(&mut self, ask: Ask) {
        let Some(client) = self.client.as_mut() else {
            return;
        };
        let (id, pending) = match ask {
            Ask::Threads => (client.threads(), Pending::Threads),
            Ask::StackTrace(thread) => (client.stack_trace(thread), Pending::StackTrace(thread)),
            Ask::Scopes(frame) => (client.scopes(frame), Pending::Scopes(frame)),
            Ask::Variables(reference) => {
                (client.variables(reference), Pending::Variables(reference))
            }
            Ask::Watch(expression, frame_id) => (
                client.evaluate(&expression, frame_id, EvalContext::Watch),
                Pending::Watch {
                    expression,
                    frame_id,
                },
            ),
            Ask::Pause(thread) => (client.pause(thread), Pending::Command("pause")),
        };
        self.pending.insert(id, pending);
    }

    /// Handles one event of the client (gpui-free).
    fn on_event(&mut self, event: Event) -> Effects {
        match event {
            Event::Response { id, result } => {
                let pending = match self.pending.remove(&id) {
                    Some(p) => p,
                    // `Client::start`'s own `setBreakpoints`.
                    None => match &result {
                        Ok(Response::SetBreakpoints { path, .. }) => {
                            let key = self.store_key(path.clone());
                            let lines = self.initial.remove(&key).unwrap_or_default();
                            Pending::SetBreakpoints(key, lines)
                        }
                        _ => return Effects::default(),
                    },
                };
                self.on_response(pending, result)
            }
            Event::Running => {
                if self.state == SessionState::Starting {
                    self.state = SessionState::Running;
                }
                Effects {
                    resend: std::mem::take(&mut self.unsent).into_iter().collect(),
                    ..Effects::default()
                }
            }
            Event::Stopped {
                reason,
                thread_id,
                all_threads: _,
                description,
            } => {
                if self.state.is_over() {
                    return Effects::default();
                }
                self.state = SessionState::Stopped {
                    reason,
                    description,
                };
                let thread = thread_id.or(self.current_thread);
                self.current_thread = thread;
                self.clear_frames();
                self.reveal = true;
                let mut effects = Effects::ask(Ask::Threads);
                if let Some(thread) = thread {
                    effects.asks.push(Ask::StackTrace(thread));
                }
                effects
            }
            Event::Continued {
                thread_id,
                all_threads,
            } => {
                if all_threads || thread_id.is_none() || thread_id == self.current_thread {
                    self.set_running();
                }
                Effects::default()
            }
            Event::Output { category, text } => {
                let kind = console_kind(category);
                let partial = match self.partial.iter_mut().find(|(k, _)| *k == kind) {
                    Some((_, p)) => p,
                    None => {
                        self.partial.push((kind, String::new()));
                        &mut self.partial.last_mut().expect("just pushed").1
                    }
                };
                let lines = split_output(partial, &text);
                for line in lines {
                    self.push_line(kind, line);
                }
                Effects::default()
            }
            Event::BreakpointChanged { path, breakpoint } => {
                let known = breakpoint
                    .id
                    .and_then(|id| self.breakpoint_ids.get(&id).cloned());
                let place = match (known, path, breakpoint.line) {
                    (Some(place), _, _) => Some(place),
                    (None, Some(path), Some(line)) => {
                        let path = self.store_key(path);
                        let known = self.verified.keys().find(|k| same_file(k, &path)).cloned();
                        Some((known.unwrap_or(path), line))
                    }
                    _ => None,
                };
                match place {
                    Some((path, line)) => {
                        self.verified
                            .entry(path.clone())
                            .or_default()
                            .insert(line, breakpoint.verified);
                        Effects {
                            verdicts: vec![path],
                            ..Effects::default()
                        }
                    }
                    None => Effects::default(),
                }
            }
            Event::ThreadStarted(id) => {
                if !self.started_threads.contains(&id) {
                    self.started_threads.push(id);
                }
                if self.state.is_stopped() {
                    Effects::ask(Ask::Threads)
                } else {
                    Effects::default()
                }
            }
            Event::ThreadExited(id) => {
                self.threads.retain(|t| t.id != id);
                self.started_threads.retain(|t| *t != id);
                Effects::default()
            }
            Event::Exited(code) => {
                if !self.state.is_over() {
                    self.flush_partial();
                    self.push_line(ConsoleKind::Console, exit_line(code));
                    self.state = SessionState::Exited(Some(code));
                }
                Effects::default()
            }
            Event::Terminated => {
                if self.ended {
                    return Effects::default();
                }
                if self.state == SessionState::Starting {
                    // A failed launch: `Error("launch: ...")`, then this.
                    self.state =
                        SessionState::Failed(self.last_error.take().unwrap_or_else(|| {
                            "The debug adapter ended before the debuggee started".into()
                        }));
                } else if !self.state.is_over() {
                    self.state = SessionState::Exited(None);
                }
                self.finish()
            }
            Event::Error(message) => {
                // The adapter's stderr (`lldb-dap: ...`) is its chatter, not
                // the session's error.
                let kind = if self.client.as_ref().is_some_and(|c| {
                    message
                        .strip_prefix(c.name())
                        .is_some_and(|rest| rest.starts_with(':'))
                }) {
                    ConsoleKind::Console
                } else {
                    self.last_error = Some(message.clone());
                    ConsoleKind::Error
                };
                for line in message.lines() {
                    self.push_line(kind, line.to_string());
                }
                Effects::default()
            }
        }
    }

    /// Handles one answer (gpui-free).
    fn on_response(&mut self, pending: Pending, result: Result<Response, String>) -> Effects {
        let response = match result {
            Ok(r) => r,
            Err(message) => {
                match pending {
                    Pending::Watch {
                        expression,
                        frame_id,
                    } => {
                        if frame_id == self.frame_id() {
                            for w in self
                                .watches
                                .iter_mut()
                                .filter(|w| w.expression == expression)
                            {
                                w.value = Some(Err(message.clone()));
                            }
                        }
                    }
                    Pending::Console(_) => self.push_line(ConsoleKind::Error, message),
                    Pending::Command(name) => {
                        self.push_line(ConsoleKind::Error, format!("{name}: {message}"))
                    }
                    Pending::SetBreakpoints(path, lines) => {
                        let map = self.verified.entry(path.clone()).or_default();
                        map.clear();
                        for line in lines {
                            map.insert(line, false);
                        }
                        return Effects {
                            verdicts: vec![path],
                            ..Effects::default()
                        };
                    }
                    _ => {}
                }
                return Effects::default();
            }
        };
        match (pending, response) {
            (
                Pending::SetBreakpoints(path, lines),
                Response::SetBreakpoints { breakpoints, .. },
            ) => {
                self.breakpoint_ids.retain(|_, (p, _)| *p != path);
                let map = self.verified.entry(path.clone()).or_default();
                map.clear();
                for (line, b) in lines.iter().zip(&breakpoints) {
                    map.insert(*line, b.verified);
                    if let Some(id) = b.id {
                        self.breakpoint_ids.insert(id, (path.clone(), *line));
                    }
                }
                Effects {
                    verdicts: vec![path],
                    ..Effects::default()
                }
            }
            (Pending::Threads, Response::Threads(threads)) => {
                self.threads = threads;
                let mut effects = Effects::default();
                if self.state.is_stopped()
                    && self.frames.is_empty()
                    && self
                        .current_thread
                        .is_none_or(|t| !self.threads.iter().any(|x| x.id == t))
                {
                    // A stop that named no thread (or one gone): the first listed.
                    if let Some(t) = self.threads.first() {
                        self.current_thread = Some(t.id);
                        effects.asks.push(Ask::StackTrace(t.id));
                    }
                }
                effects
            }
            (Pending::StackTrace(_), Response::StackTrace { thread_id, frames }) => {
                if !self.state.is_stopped() || Some(thread_id) != self.current_thread {
                    return Effects::default();
                }
                self.frames = frames
                    .into_iter()
                    .map(|mut f| {
                        f.path = f.path.map(|p| self.store_key(p));
                        f
                    })
                    .collect();
                self.current_frame = 0;
                self.scopes.clear();
                let mut effects = self.frame_selected();
                if std::mem::take(&mut self.reveal) {
                    effects.stopped = true;
                }
                effects
            }
            (Pending::Scopes(_), Response::Scopes { frame_id, scopes }) => {
                if self.frame_id() != Some(frame_id) {
                    return Effects::default();
                }
                self.scopes = scopes;
                // Locals: lldb-dap lists Locals, Globals, Registers and marks
                // only Registers cheap, so the first scope, not the first cheap one.
                match self
                    .scopes
                    .iter()
                    .find(|s| s.name == "Locals" && s.variables_reference != 0)
                    .or_else(|| self.scopes.iter().find(|s| s.variables_reference != 0))
                {
                    Some(scope) => {
                        let reference = scope.variables_reference;
                        self.expanded.insert(reference);
                        if self.children.contains_key(&reference) {
                            Effects::default()
                        } else {
                            Effects::ask(Ask::Variables(reference))
                        }
                    }
                    None => Effects::default(),
                }
            }
            (
                Pending::Variables(_),
                Response::Variables {
                    reference,
                    variables,
                },
            ) => {
                if self.state.is_stopped() {
                    self.children.insert(reference, variables);
                }
                Effects::default()
            }
            (
                Pending::Watch {
                    expression,
                    frame_id,
                },
                response @ Response::Evaluate { .. },
            ) => {
                if frame_id == self.frame_id() {
                    let value = evaluation(expression.clone(), Ok(response));
                    for w in self
                        .watches
                        .iter_mut()
                        .filter(|w| w.expression == expression)
                    {
                        w.value = Some(value.clone());
                    }
                }
                Effects::default()
            }
            (Pending::Console(_), Response::Evaluate { result, .. }) => {
                self.push_line(ConsoleKind::Result, result);
                Effects::default()
            }
            _ => Effects::default(),
        }
    }

    /// After the selected frame changed: its scopes, the watches in it.
    fn frame_selected(&mut self) -> Effects {
        let mut effects = Effects::default();
        let frame_id = self.frame_id();
        if let Some(id) = frame_id {
            effects.asks.push(Ask::Scopes(id));
        }
        for w in &mut self.watches {
            w.value = None;
        }
        let mut expressions: Vec<String> = Vec::new();
        for w in &self.watches {
            if !expressions.contains(&w.expression) {
                expressions.push(w.expression.clone());
            }
        }
        if frame_id.is_some() {
            effects
                .asks
                .extend(expressions.into_iter().map(|e| Ask::Watch(e, frame_id)));
        }
        effects
    }

    fn clear_frames(&mut self) {
        self.frames.clear();
        self.current_frame = 0;
        self.scopes.clear();
        // Variables references are only good while the debuggee stays stopped.
        self.children.clear();
        self.expanded.clear();
    }

    fn set_running(&mut self) {
        if self.state.is_over() {
            return;
        }
        self.state = SessionState::Running;
        self.reveal = false;
        self.clear_frames();
        for w in &mut self.watches {
            w.value = None;
        }
    }

    /// The session is over: what is left of the output, the verdicts cleared.
    fn finish(&mut self) -> Effects {
        self.ended = true;
        self.flush_partial();
        self.clear_frames();
        self.verified.clear();
        self.breakpoint_ids.clear();
        self.reveal = false;
        for w in &mut self.watches {
            w.value = None;
        }
        Effects {
            ended: true,
            ..Effects::default()
        }
    }

    fn flush_partial(&mut self) {
        let rest: Vec<(ConsoleKind, String)> = self
            .partial
            .drain(..)
            .filter(|(_, p)| !p.is_empty())
            .collect();
        for (kind, text) in rest {
            self.push_line(kind, text);
        }
    }

    fn push_line(&mut self, kind: ConsoleKind, text: String) {
        self.console.push(ConsoleLine {
            kind,
            text: text.into(),
        });
        if self.console.len() > CONSOLE_LIMIT {
            let over = self.console.len() - CONSOLE_LIMIT;
            self.console.drain(..over);
        }
    }

    // ---- commands -------------------------------------------------------------------

    fn command(
        &mut self,
        name: &'static str,
        send: impl FnOnce(&mut Client, i64) -> RequestId,
        cx: &mut Context<Self>,
    ) {
        if !self.state.is_stopped() {
            return;
        }
        let (Some(thread), Some(client)) = (self.current_thread, self.client.as_mut()) else {
            return;
        };
        let id = send(client, thread);
        self.pending.insert(id, Pending::Command(name));
        self.set_running();
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        self.command("resume", |c, t| c.resume(t), cx);
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        if self.state != SessionState::Running || self.client.is_none() {
            return;
        }
        // `threads` fails while the debuggee runs; lldb-dap stops the whole
        // process whichever thread is named, so any known id does (the
        // reason comes back as "pause").
        let thread = self
            .current_thread
            .or_else(|| self.threads.first().map(|t| t.id))
            .or(self.started_threads.first().copied())
            .unwrap_or(0);
        self.send(Ask::Pause(thread));
        cx.notify();
    }

    pub fn step_over(&mut self, cx: &mut Context<Self>) {
        self.command("step over", |c, t| c.step_over(t), cx);
    }

    pub fn step_into(&mut self, cx: &mut Context<Self>) {
        self.command("step into", |c, t| c.step_into(t), cx);
    }

    pub fn step_out(&mut self, cx: &mut Context<Self>) {
        self.command("step out", |c, t| c.step_out(t), cx);
    }

    /// Ends the session: the build, or the debuggee through the adapter.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.state.is_over() {
            return;
        }
        if let Some(build) = &self.build {
            // Its exit fails the session ("The build was stopped").
            build.kill();
            return;
        }
        match self.client.as_mut() {
            // The adapter's `terminated` (or its closed pipes) ends the session.
            Some(client) => client.stop(),
            None => {
                self.state = SessionState::Exited(None);
                let effects = self.finish();
                self.apply(effects, cx);
            }
        }
    }

    pub fn select_thread(&mut self, thread_id: i64, cx: &mut Context<Self>) {
        if self.current_thread == Some(thread_id) && !self.frames.is_empty() {
            return;
        }
        self.current_thread = Some(thread_id);
        self.frames.clear();
        self.current_frame = 0;
        self.scopes.clear();
        if self.state.is_stopped() {
            self.send(Ask::StackTrace(thread_id));
        }
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }

    pub fn select_frame(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.frames.len() || index == self.current_frame && !self.scopes.is_empty() {
            return;
        }
        self.current_frame = index;
        self.scopes.clear();
        let effects = self.frame_selected();
        self.apply(effects, cx);
        cx.emit(DebugSessionEvent::Changed);
    }

    /// Shows or hides a variable's children, fetching them the first time.
    pub fn toggle_expanded(&mut self, reference: i64, cx: &mut Context<Self>) {
        if reference == 0 {
            return;
        }
        if !self.expanded.remove(&reference) {
            self.expanded.insert(reference);
            if !self.children.contains_key(&reference) && self.state.is_stopped() {
                self.send(Ask::Variables(reference));
            }
        }
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }

    pub fn add_watch(&mut self, expression: String, cx: &mut Context<Self>) {
        let expression = expression.trim().to_string();
        if expression.is_empty() {
            return;
        }
        self.watches.push(Watch {
            expression: expression.clone(),
            value: None,
        });
        self.evaluate_watch(expression, cx);
    }

    pub fn edit_watch(&mut self, index: usize, expression: String, cx: &mut Context<Self>) {
        let expression = expression.trim().to_string();
        if expression.is_empty() {
            self.delete_watch(index, cx);
            return;
        }
        let Some(w) = self.watches.get_mut(index) else {
            return;
        };
        w.expression = expression.clone();
        w.value = None;
        self.evaluate_watch(expression, cx);
    }

    fn evaluate_watch(&mut self, expression: String, cx: &mut Context<Self>) {
        if let Some(frame_id) = self.frame_id() {
            self.send(Ask::Watch(expression, Some(frame_id)));
        }
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }

    pub fn delete_watch(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.watches.len() {
            self.watches.remove(index);
            cx.emit(DebugSessionEvent::Changed);
            cx.notify();
        }
    }

    /// Evaluates in the selected frame; the answer's `name` is the expression.
    pub fn evaluate(
        &mut self,
        expression: String,
        context: EvalContext,
        cx: &mut Context<Self>,
    ) -> Task<Result<Variable, String>> {
        let frame_id = self.frame_id();
        let live = !self.state.is_over();
        let Some(client) = self.client.as_mut().filter(|_| live) else {
            return Task::ready(Err("no debug session".into()));
        };
        let id = client.evaluate(&expression, frame_id, context);
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, Pending::Eval(expression));
        self.evals.insert(id, tx);
        cx.background_spawn(async move {
            rx.await
                .unwrap_or_else(|_| Err("the debug session ended".into()))
        })
    }

    /// The Debugger Console's input: echoes the line, then its value.
    pub fn evaluate_in_console(&mut self, expression: String, cx: &mut Context<Self>) {
        let expression = expression.trim().to_string();
        if expression.is_empty() {
            return;
        }
        self.push_line(ConsoleKind::Input, expression.clone());
        let frame_id = self.frame_id();
        let live = !self.state.is_over();
        match self.client.as_mut().filter(|_| live) {
            Some(client) => {
                let id = client.evaluate(&expression, frame_id, EvalContext::Repl);
                self.pending.insert(id, Pending::Console(expression));
            }
            // estimate: Fleet's wording unknown.
            None => self.push_line(
                ConsoleKind::Error,
                "The debug session is not running".into(),
            ),
        }
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }
}

/// An `evaluate` answer as the variable the views show.
fn evaluation(expression: String, result: Result<Response, String>) -> Result<Variable, String> {
    match result? {
        Response::Evaluate {
            result,
            type_name,
            variables_reference,
        } => Ok(Variable {
            evaluate_name: Some(expression.clone()),
            name: expression,
            value: result,
            type_name,
            variables_reference,
        }),
        other => Err(format!("unexpected answer: {other:?}")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebuggerEvent {
    /// A session started or ended, or the active one changed.
    SessionsChanged,
}

/// Every debug session of the app, and the one the tools show. It
/// re-notifies on every change of a session, so observing the Debugger is
/// enough to repaint with the execution point.
#[derive(Default)]
pub struct Debugger {
    sessions: Vec<Entity<DebugSession>>,
    active: Option<usize>,
    observers: HashMap<EntityId, Subscription>,
}

impl EventEmitter<DebuggerEvent> for Debugger {}

struct GlobalDebugger(Entity<Debugger>);

impl Global for GlobalDebugger {}

impl Debugger {
    pub fn init(cx: &mut App) {
        let debugger = cx.new(|_| Debugger::default());
        cx.set_global(GlobalDebugger(debugger));
    }

    pub fn global(cx: &App) -> Entity<Debugger> {
        cx.global::<GlobalDebugger>().0.clone()
    }

    pub fn sessions(&self) -> &[Entity<DebugSession>] {
        &self.sessions
    }

    pub fn active(&self) -> Option<Entity<DebugSession>> {
        self.active.and_then(|ix| self.sessions.get(ix)).cloned()
    }

    pub fn set_active(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.sessions.len() && self.active != Some(index) {
            self.active = Some(index);
            cx.emit(DebuggerEvent::SessionsChanged);
            cx.notify();
        }
    }

    /// Adds a started session and makes it the active one.
    pub fn add(&mut self, session: Entity<DebugSession>, cx: &mut Context<Self>) {
        self.observers.insert(
            session.entity_id(),
            cx.observe(&session, |_, _, cx| cx.notify()),
        );
        self.sessions.push(session);
        self.active = Some(self.sessions.len() - 1);
        cx.emit(DebuggerEvent::SessionsChanged);
        cx.notify();
    }

    pub fn remove(&mut self, session: &Entity<DebugSession>, cx: &mut Context<Self>) {
        if let Some(ix) = self.sessions.iter().position(|s| s == session) {
            self.observers.remove(&session.entity_id());
            self.sessions.remove(ix);
            self.active = match self.active {
                _ if self.sessions.is_empty() => None,
                Some(a) if a > ix => Some(a - 1),
                Some(a) => Some(a.min(self.sessions.len() - 1)),
                None => None,
            };
            cx.emit(DebuggerEvent::SessionsChanged);
            cx.notify();
        }
    }

    /// Drops the sessions that are over and match `which` (the Debug tool
    /// closed, or a new session of the same configuration started).
    pub fn remove_ended(&mut self, which: impl Fn(&DebugSession) -> bool, cx: &mut Context<Self>) {
        let gone: Vec<Entity<DebugSession>> = self
            .sessions
            .iter()
            .filter(|s| {
                let s = s.read(cx);
                s.state.is_over() && which(s)
            })
            .cloned()
            .collect();
        for session in gone {
            self.remove(&session, cx);
        }
    }

    /// The sessions still running (or starting, or stopped).
    pub fn live(&self, cx: &App) -> Vec<Entity<DebugSession>> {
        self.sessions
            .iter()
            .filter(|s| !s.read(cx).state.is_over())
            .cloned()
            .collect()
    }

    /// The active session's execution point, for the editors.
    pub fn execution_point(&self, cx: &App) -> Option<ExecutionPoint> {
        self.active()?.read(cx).execution_point()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use weep_dap::{Breakpoint, Scope};

    fn frame(id: i64, line: usize) -> StackFrame {
        StackFrame {
            id,
            name: format!("f{id}"),
            path: Some(PathBuf::from("/w/src/main.rs")),
            line,
            column: 0,
        }
    }

    fn stopped(thread: Option<i64>) -> Event {
        Event::Stopped {
            reason: "breakpoint".into(),
            thread_id: thread,
            all_threads: true,
            description: None,
        }
    }

    fn running(s: &mut DebugSession) {
        s.on_event(Event::Running);
    }

    #[test]
    fn breakpoints_convert_enabled_only() {
        let mut off = LineBreakpoint::at(3);
        off.enabled = false;
        let mut cond = LineBreakpoint::at(7);
        cond.condition = Some("x > 1".into());
        cond.log_message = Some(String::new());
        let list = vec![LineBreakpoint::at(1), off, cond];
        assert_eq!(
            source_breakpoints(&list),
            vec![
                SourceBreakpoint {
                    line: 1,
                    condition: None,
                    log_message: None
                },
                SourceBreakpoint {
                    line: 7,
                    condition: Some("x > 1".into()),
                    log_message: None
                },
            ]
        );
    }

    #[test]
    fn verdicts_follow_the_lines_answered() {
        let known: HashMap<usize, bool> = [(1, true), (7, false)].into_iter().collect();
        assert_eq!(verdicts(&[1, 7], &known), vec![true, false]);
        // A line added since: it and the rest stay unknown.
        assert_eq!(verdicts(&[1, 4, 7], &known), vec![true]);
    }

    #[test]
    fn output_is_split_into_lines_across_chunks() {
        let mut s = DebugSession::new("x".into());
        s.on_event(Event::Output {
            category: OutputCategory::Stdout,
            text: "hel".into(),
        });
        assert!(s.console.is_empty());
        s.on_event(Event::Output {
            category: OutputCategory::Stdout,
            text: "lo\r\nworld\n!".into(),
        });
        s.on_event(Event::Output {
            category: OutputCategory::Stderr,
            text: "bad\n".into(),
        });
        let texts: Vec<_> = s
            .console
            .iter()
            .map(|l| (l.kind, l.text.to_string()))
            .collect();
        assert_eq!(
            texts,
            vec![
                (ConsoleKind::Stdout, "hello".into()),
                (ConsoleKind::Stdout, "world".into()),
                (ConsoleKind::Stderr, "bad".into()),
            ]
        );
        s.on_event(Event::Exited(0));
        let e = s.on_event(Event::Terminated);
        assert!(e.ended);
        assert_eq!(s.state, SessionState::Exited(Some(0)));
        let texts: Vec<_> = s.console.iter().map(|l| l.text.to_string()).collect();
        assert_eq!(&texts[3..], &["!", "Process finished with exit code 0"]);
    }

    #[test]
    fn a_stop_asks_threads_frames_scopes_variables_and_watches() {
        let mut s = DebugSession::new("x".into());
        assert_eq!(s.state, SessionState::Starting);
        running(&mut s);
        assert_eq!(s.state, SessionState::Running);
        s.watches.push(Watch {
            expression: "a + 1".into(),
            value: None,
        });
        let e = s.on_event(stopped(Some(5)));
        assert!(s.state.is_stopped());
        assert_eq!(s.current_thread, Some(5));
        assert_eq!(e.asks, vec![Ask::Threads, Ask::StackTrace(5)]);
        assert!(!e.stopped);
        let e = s.on_response(
            Pending::Threads,
            Ok(Response::Threads(vec![Thread {
                id: 5,
                name: "main".into(),
            }])),
        );
        assert!(e.asks.is_empty());
        let e = s.on_response(
            Pending::StackTrace(5),
            Ok(Response::StackTrace {
                thread_id: 5,
                frames: vec![frame(100, 9), frame(101, 3)],
            }),
        );
        assert!(e.stopped, "the frames of a fresh stop reveal it");
        assert_eq!(
            e.asks,
            vec![Ask::Scopes(100), Ask::Watch("a + 1".into(), Some(100))]
        );
        assert_eq!(
            s.execution_point(),
            Some(ExecutionPoint {
                path: "/w/src/main.rs".into(),
                line: 9,
                top: true
            })
        );
        let e = s.on_response(
            Pending::Scopes(100),
            Ok(Response::Scopes {
                frame_id: 100,
                scopes: vec![
                    Scope {
                        name: "Registers".into(),
                        variables_reference: 7,
                        expensive: true,
                    },
                    Scope {
                        name: "Locals".into(),
                        variables_reference: 8,
                        expensive: false,
                    },
                ],
            }),
        );
        assert_eq!(e.asks, vec![Ask::Variables(8)]);
        assert!(s.expanded.contains(&8));
        s.on_response(
            Pending::Variables(8),
            Ok(Response::Variables {
                reference: 8,
                variables: vec![Variable {
                    name: "a".into(),
                    value: "1".into(),
                    type_name: Some("i32".into()),
                    variables_reference: 0,
                    evaluate_name: None,
                }],
            }),
        );
        assert_eq!(s.children[&8].len(), 1);
        s.on_response(
            Pending::Watch {
                expression: "a + 1".into(),
                frame_id: Some(100),
            },
            Ok(Response::Evaluate {
                result: "2".into(),
                type_name: None,
                variables_reference: 0,
            }),
        );
        assert_eq!(
            s.watches[0].value.as_ref().unwrap().as_ref().unwrap().value,
            "2"
        );
        // A watch answer for a frame no longer selected is dropped.
        s.current_frame = 1;
        s.on_response(
            Pending::Watch {
                expression: "a + 1".into(),
                frame_id: Some(100),
            },
            Err("gone".into()),
        );
        assert!(s.watches[0].value.as_ref().unwrap().is_ok());
        assert_eq!(
            s.execution_point().map(|p| (p.line, p.top)),
            Some((3, false))
        );
    }

    #[test]
    fn a_stop_without_a_thread_takes_the_first_listed() {
        let mut s = DebugSession::new("x".into());
        running(&mut s);
        let e = s.on_event(stopped(None));
        assert_eq!(e.asks, vec![Ask::Threads]);
        let e = s.on_response(
            Pending::Threads,
            Ok(Response::Threads(vec![
                Thread {
                    id: 2,
                    name: "a".into(),
                },
                Thread {
                    id: 3,
                    name: "b".into(),
                },
            ])),
        );
        assert_eq!(e.asks, vec![Ask::StackTrace(2)]);
        assert_eq!(s.current_thread, Some(2));
    }

    #[test]
    fn running_again_clears_the_stop() {
        let mut s = DebugSession::new("x".into());
        running(&mut s);
        s.on_event(stopped(Some(1)));
        s.on_response(
            Pending::StackTrace(1),
            Ok(Response::StackTrace {
                thread_id: 1,
                frames: vec![frame(10, 2)],
            }),
        );
        s.children.insert(4, Vec::new());
        s.on_event(Event::Continued {
            thread_id: Some(1),
            all_threads: false,
        });
        assert_eq!(s.state, SessionState::Running);
        assert!(s.frames.is_empty() && s.children.is_empty() && s.scopes.is_empty());
        assert_eq!(s.execution_point(), None);
        // A late stack trace after the resume is ignored.
        let e = s.on_response(
            Pending::StackTrace(1),
            Ok(Response::StackTrace {
                thread_id: 1,
                frames: vec![frame(10, 2)],
            }),
        );
        assert_eq!(e, Effects::default());
        assert!(s.frames.is_empty());
    }

    #[test]
    fn breakpoint_answers_become_verdicts() {
        let mut s = DebugSession::new("x".into());
        let path = PathBuf::from("/w/src/main.rs");
        s.initial.insert(path.clone(), vec![1, 7]);
        // `Client::start`'s own request: an id this side never saw.
        let e = s.on_event(Event::Response {
            id: RequestId(42),
            result: Ok(Response::SetBreakpoints {
                path: path.clone(),
                breakpoints: vec![
                    Breakpoint {
                        id: Some(1),
                        verified: true,
                        line: Some(1),
                        message: None,
                    },
                    Breakpoint {
                        id: Some(2),
                        verified: false,
                        line: None,
                        message: Some("no code".into()),
                    },
                ],
            }),
        });
        assert_eq!(e.verdicts, vec![path.clone()]);
        assert_eq!(verdicts(&[1, 7], &s.verified[&path]), vec![true, false]);
        // The adapter binds the second one later.
        let e = s.on_event(Event::BreakpointChanged {
            path: None,
            breakpoint: Breakpoint {
                id: Some(2),
                verified: true,
                line: Some(8),
                message: None,
            },
        });
        assert_eq!(e.verdicts, vec![path.clone()]);
        assert_eq!(verdicts(&[1, 7], &s.verified[&path]), vec![true, true]);
        // A failed request leaves every line it sent unverified.
        s.on_response(
            Pending::SetBreakpoints(path.clone(), vec![3]),
            Err("x".into()),
        );
        assert_eq!(verdicts(&[3], &s.verified[&path]), vec![false]);
        // The end clears them.
        let e = s.on_event(Event::Terminated);
        assert!(e.ended);
        assert!(s.verified.is_empty());
    }

    #[test]
    fn changes_before_the_configuration_are_sent_after_it() {
        let mut s = DebugSession::new("x".into());
        s.unsent.insert(PathBuf::from("/a.rs"));
        let e = s.on_event(Event::Running);
        assert_eq!(e.resend, vec![PathBuf::from("/a.rs")]);
        assert!(s.unsent.is_empty());
    }

    #[test]
    fn console_answers_and_errors() {
        let mut s = DebugSession::new("x".into());
        s.on_response(
            Pending::Console("a".into()),
            Ok(Response::Evaluate {
                result: "1".into(),
                type_name: None,
                variables_reference: 0,
            }),
        );
        s.on_response(Pending::Console("b".into()), Err("no b".into()));
        s.on_event(Event::Error("adapter said\nthings".into()));
        let kinds: Vec<_> = s
            .console
            .iter()
            .map(|l| (l.kind, l.text.to_string()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (ConsoleKind::Result, "1".into()),
                (ConsoleKind::Error, "no b".into()),
                (ConsoleKind::Error, "adapter said".into()),
                (ConsoleKind::Error, "things".into()),
            ]
        );
        assert_eq!(
            evaluation(
                "e".into(),
                Ok(Response::Evaluate {
                    result: "3".into(),
                    type_name: Some("u8".into()),
                    variables_reference: 5
                })
            )
            .unwrap(),
            Variable {
                name: "e".into(),
                value: "3".into(),
                type_name: Some("u8".into()),
                variables_reference: 5,
                evaluate_name: Some("e".into())
            }
        );
    }

    #[test]
    fn canonical_paths_come_back_in_the_workspaces_spelling() {
        let roots = vec![(PathBuf::from("/private/var/w"), PathBuf::from("/var/w"))];
        assert_eq!(
            respell(PathBuf::from("/private/var/w/src/main.rs"), &roots),
            PathBuf::from("/var/w/src/main.rs")
        );
        assert_eq!(
            respell(PathBuf::from("/usr/lib/x.c"), &roots),
            PathBuf::from("/usr/lib/x.c")
        );
        let dir = std::env::temp_dir().join(format!("weep-debugger-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.rs");
        std::fs::write(&file, "").unwrap();
        assert!(same_file(&file, &file.canonicalize().unwrap()));
        assert!(!same_file(&file, &dir.join("b.rs")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adapter_paths_map_back_to_store_keys() {
        let dir = std::env::temp_dir().join(format!("weep-debugger-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, "").unwrap();
        let mut s = DebugSession::new("x".into());
        s.remember_key(&file);
        let canonical = file.canonicalize().unwrap();
        assert_eq!(s.store_key(canonical.clone()), file);
        // The initial answer, by its canonical path, lands on the key.
        s.initial.insert(file.clone(), vec![0]);
        let e = s.on_event(Event::Response {
            id: RequestId(9),
            result: Ok(Response::SetBreakpoints {
                path: canonical,
                breakpoints: vec![Breakpoint {
                    id: None,
                    verified: true,
                    line: Some(0),
                    message: None,
                }],
            }),
        });
        assert_eq!(e.verdicts, vec![file.clone()]);
        assert_eq!(s.verified[&file].get(&0), Some(&true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_launch_fails_the_session() {
        let mut s = DebugSession::new("x".into());
        s.on_event(Event::Error("launch: no such file".into()));
        let e = s.on_event(Event::Terminated);
        assert!(e.ended);
        assert_eq!(s.state, SessionState::Failed("launch: no such file".into()));
        // A second Terminated changes nothing.
        assert_eq!(s.on_event(Event::Terminated), Effects::default());
    }

    #[test]
    fn a_stop_after_the_end_is_ignored() {
        let mut s = DebugSession::new("x".into());
        running(&mut s);
        s.on_event(Event::Terminated);
        assert_eq!(s.state, SessionState::Exited(None));
        let e = s.on_event(stopped(Some(1)));
        assert_eq!(e, Effects::default());
        assert!(s.state.is_over());
    }
}
