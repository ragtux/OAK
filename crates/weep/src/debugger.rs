//! The debugger's app-side state (M8): one [`DebugSession`] per started
//! debug run, each owning a `weep_dap::Client`, and the [`Debugger`] that
//! lists them and knows the active one. The Debug, Breakpoints and Debugger
//! Console tools, the floating debug panel and the editor (the execution
//! point, quick evaluate) read these entities and call their commands; the
//! workspace (`workspace/debug.rs`) starts sessions from run configurations
//! and routes the `debug/*` actions here.
//!
//! A session's life: `Starting` (a `cargo` row's build streams into the
//! session's run tab first, then the adapter starts and gets every file's
//! enabled breakpoints), `Running` once the adapter reports the
//! configuration done, `Stopped` on a stop (threads, the stopped thread's
//! stack and frame 0's scopes are asked in turn, the scopes left collapsed
//! as Fleet shows them, batch 13, and the watches re-evaluated), back to
//! `Running` on resume or a step, and `Exited` when the debuggee ends or the
//! adapter goes (`Failed` when it never started). [`DebugSession::restart`]
//! stops it and starts the same build or launch again in the same entity.
//! The client is drained on the foreground through a wake channel, as
//! `workspace/smart.rs` drains rust-analyzer. What needs no gpui (the event
//! and response handling) is in plain methods returning [`Effects`], tested
//! below.
//!
//! Two texts (batch 13): [`DebugSession::output`] is the session's run tab
//! (Fleet puts the build in a bottom-panel tab named after the
//! configuration: the `cargo build` line, the build's output, then the
//! adapter's command line; the debuggee's stdout and stderr follow there,
//! an estimate); [`DebugSession::console`] is the Debugger Console's: the
//! adapter's own messages, typed expressions and their values, errors.
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

/// One line of a session's run tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunLine {
    pub kind: RunLineKind,
    pub text: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RunLineKind {
    /// A process's command line (the build's, the adapter's), in
    /// `text.secondary`; each after the first opens a new block.
    Command,
    Stdout,
    /// In the terminal's ANSI red (batch 13: cargo's "Finished" line).
    Stderr,
}

/// How the session was started, to start it again ([`DebugSession::restart`]).
#[derive(Clone, Debug)]
enum Origin {
    Build(weep_run::cargo::DebugBuild, AdapterCommand),
    Start(AdapterCommand, Start),
}

/// The adapter's command line as the run tab prints it.
fn adapter_line(adapter: &AdapterCommand) -> String {
    std::iter::once(adapter.program.display().to_string())
        .chain(adapter.args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One line of the Debugger Console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleLine {
    pub kind: ConsoleKind,
    pub text: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsoleKind {
    /// The debuggee's stdout (batch 13: now in the run tab, [`RunLine`];
    /// kept for the views' match).
    Stdout,
    /// The debuggee's stderr (likewise).
    Stderr,
    /// The adapter's own messages.
    Console,
    /// A line typed into the console: its block, by its index in
    /// [`DebugSession::evaluations`] (the line's text is the input).
    Evaluation(usize),
    /// A session error.
    Error,
}

/// The Debugger Console's first line (batch 13: Fleet's console is in LLDB's
/// "commands" mode; a blank line, then the launch line, follow it).
pub const CONSOLE_MODE_LINE: &str = "Console is in 'commands' mode, prefix expressions with '?'.";

/// What a line typed into the Debugger Console is (Fleet's "commands" mode,
/// batch 13): an LLDB command, or after a leading `?` an expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleInput {
    Command(String),
    Expression(String),
}

impl ConsoleInput {
    /// The line typed, trimmed; `None` for a blank line or a bare `?`.
    pub fn classify(line: &str) -> Option<ConsoleInput> {
        let line = line.trim();
        match line.strip_prefix('?') {
            Some(expression) => {
                let expression = expression.trim();
                (!expression.is_empty()).then(|| ConsoleInput::Expression(expression.into()))
            }
            None => (!line.is_empty()).then(|| ConsoleInput::Command(line.into())),
        }
    }
}

/// An evaluation's answer, as its block shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalAnswer {
    /// An expression's value, `{type} value` (the watch context's answer:
    /// the bare value with its type).
    Value {
        type_name: Option<String>,
        value: String,
    },
    /// A command's output as LLDB printed it (its own errors included).
    Output(String),
    /// The request failed.
    Error(String),
}

/// One line typed into the Debugger Console and its answer: Fleet's block
/// (the input, the answer under it, the time it took, "rerun").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    /// As typed (trimmed), the `?` kept: what the block shows and rerun sends.
    pub input: String,
    pub command: bool,
    pub started: std::time::Instant,
    /// Set with the answer.
    pub elapsed: Option<std::time::Duration>,
    pub answer: Option<EvalAnswer>,
}

/// The time an evaluation took, as its block prints it (batch 13: "3ms";
/// estimate: seconds with a decimal from one second on).
pub fn elapsed_text(elapsed: std::time::Duration) -> String {
    let ms = elapsed.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", elapsed.as_secs_f64())
    }
}

/// Whether an adapter console line is lldb-dap's echo of the commands WEEP
/// gave the launch (`Running initCommands:`, then `(lldb) command script
/// import …` per command): Fleet's console shows none of its own set-up.
/// Their output (an error a command printed) stays.
pub fn is_init_echo(line: &str) -> bool {
    let line = line.trim_end();
    line.starts_with("(lldb) ")
        || line
            .strip_prefix("Running ")
            .is_some_and(|rest| rest.ends_with("Commands:") && !rest.contains(' '))
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
    /// A line typed into the Debugger Console, by its evaluation's index.
    Console(usize),
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

/// The debuggee's own output goes to the run tab (estimate: Fleet's run tab
/// carries the adapter's process; its Debugger Console showed only the
/// adapter's messages, batch 13).
fn output_kind(category: OutputCategory) -> Option<RunLineKind> {
    match category {
        OutputCategory::Stdout => Some(RunLineKind::Stdout),
        OutputCategory::Stderr => Some(RunLineKind::Stderr),
        OutputCategory::Console | OutputCategory::Other => None,
    }
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
    /// The lines typed into the console, in order ([`ConsoleKind::Evaluation`]).
    pub evaluations: Vec<Evaluation>,
    /// The session's run tab: command lines, the build's and the
    /// debuggee's output.
    pub output: Vec<RunLine>,
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
    /// The run tab's stdout and stderr not yet ended by a newline.
    output_partial: [String; 2],
    /// A stop whose frames have not arrived: their arrival is the Stopped event.
    reveal: bool,
    /// The adapter answered `threads` once. lldb-dap's first answer is the
    /// list it took at launch, before the debuggee named its threads
    /// (`Thread 1` where every later answer says `main`, checked against
    /// lldb-dap from Xcode on macOS): the first answer asks again, so the
    /// first stop shows the name as Fleet does (batch 13).
    threads_answered: bool,
    /// [`DebugSession::finish`] ran: later events change nothing.
    ended: bool,
    /// A `cargo` row's build, before the adapter starts.
    build: Option<weep_run::Run>,
    /// What [`DebugSession::restart`] starts again.
    origin: Option<Origin>,
    /// A restart waits for the stop to end the session.
    restart_pending: bool,
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
            evaluations: Vec::new(),
            output: Vec::new(),
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
            output_partial: [String::new(), String::new()],
            reveal: false,
            threads_answered: false,
            ended: false,
            build: None,
            origin: None,
            restart_pending: false,
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
        if self.origin.is_none() {
            self.origin = Some(Origin::Start(adapter.clone(), start.clone()));
        }
        self.push_output(RunLineKind::Command, adapter_line(adapter));
        // M8 console: the Debugger Console's first lines.
        self.console_intro(&start);
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
    /// the run tab, then launches the picked executable under `adapter`.
    pub fn build_then_start(
        &mut self,
        build: weep_run::cargo::DebugBuild,
        adapter: AdapterCommand,
        cx: &mut Context<Self>,
    ) {
        self.origin = Some(Origin::Build(build.clone(), adapter.clone()));
        self.follow_breakpoints(cx);
        self.push_output(RunLineKind::Command, build.command.display.clone());
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
                                    None => this.push_output(RunLineKind::Stdout, line),
                                }
                            }
                            weep_run::Event::Stderr(line) => {
                                this.push_output(RunLineKind::Stderr, line)
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
                    self.push_output(
                        RunLineKind::Stdout,
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

    /// The session could not start: the message in the console and the run
    /// tab (estimate: Fleet's run tab for a failed start unprobed), the
    /// state `Failed`, and the session over.
    pub fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        if self.state.is_over() {
            return;
        }
        self.push_line(ConsoleKind::Error, message.clone());
        self.push_output(RunLineKind::Stderr, message.clone());
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
            if std::mem::take(&mut self.restart_pending) {
                // A restart's stop: the same start again, no Ended; on the
                // next turn, out of the pump that delivered the end.
                cx.spawn(async move |this, cx| {
                    let _ = this.update(cx, |this, cx| this.relaunch(cx));
                })
                .detach();
                cx.notify();
                return;
            }
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
                if let Some(kind) = output_kind(category) {
                    let partial =
                        &mut self.output_partial[usize::from(kind == RunLineKind::Stderr)];
                    for line in split_output(partial, &text) {
                        self.push_output(kind, line);
                    }
                    return Effects::default();
                }
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
                    if kind == ConsoleKind::Console && is_init_echo(&line) {
                        continue;
                    }
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
                    Pending::Console(ix) => self.answer(ix, EvalAnswer::Error(message)),
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
                // lldb-dap's first answer predates the threads' names.
                if !std::mem::replace(&mut self.threads_answered, true) && self.state.is_stopped() {
                    effects.asks.push(Ask::Threads);
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
                // Collapsed, nothing fetched until one is expanded (batch 13:
                // Fleet shows Local, Static, Global, Registers closed on a stop).
                self.scopes = scopes;
                Effects::default()
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
            (
                Pending::Console(ix),
                Response::Evaluate {
                    result, type_name, ..
                },
            ) => {
                let command = self.evaluations.get(ix).is_some_and(|e| e.command);
                let answer = if command {
                    EvalAnswer::Output(result)
                } else {
                    EvalAnswer::Value {
                        type_name: type_name.filter(|t| !t.is_empty()),
                        value: result,
                    }
                };
                self.answer(ix, answer);
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
        for (ix, kind) in [RunLineKind::Stdout, RunLineKind::Stderr]
            .into_iter()
            .enumerate()
        {
            let rest = std::mem::take(&mut self.output_partial[ix]);
            if !rest.is_empty() {
                self.push_output(kind, rest);
            }
        }
        let rest: Vec<(ConsoleKind, String)> = self
            .partial
            .drain(..)
            .filter(|(_, p)| !p.is_empty())
            .collect();
        for (kind, text) in rest {
            self.push_line(kind, text);
        }
    }

    fn push_output(&mut self, kind: RunLineKind, text: String) {
        self.output.push(RunLine {
            kind,
            text: text.into(),
        });
        if self.output.len() > CONSOLE_LIMIT {
            let over = self.output.len() - CONSOLE_LIMIT;
            self.output.drain(..over);
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

    /// Stops the session and starts the same build or launch again in this
    /// entity (Fleet's Restart, between Pause and Stop; batch 13). An ended
    /// session starts again at once.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        if self.origin.is_none() {
            return;
        }
        if self.state.is_over() {
            self.relaunch(cx);
            return;
        }
        self.restart_pending = true;
        self.stop(cx);
    }

    fn relaunch(&mut self, cx: &mut Context<Self>) {
        let Some(origin) = self.origin.clone() else {
            return;
        };
        self.reset();
        match origin {
            Origin::Build(build, adapter) => self.build_then_start(build, adapter, cx),
            Origin::Start(adapter, start) => self.start(&adapter, start, cx),
        }
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }

    /// Back to a fresh session before a restart: the watches and the
    /// breakpoint subscription stay, the texts start over (estimate: as the
    /// run tool's Rerun clears its output).
    fn reset(&mut self) {
        if let Some(mut client) = self.client.take() {
            client.kill();
        }
        if let Some(build) = self.build.take() {
            build.kill();
        }
        self.state = SessionState::Starting;
        self.threads.clear();
        self.current_thread = None;
        self.clear_frames();
        for w in &mut self.watches {
            w.value = None;
        }
        self.console.clear();
        self.evaluations.clear();
        self.output.clear();
        self.pending.clear();
        for (_, tx) in self.evals.drain() {
            let _ = tx.send(Err("the debug session restarted".into()));
        }
        self.initial.clear();
        self.verified.clear();
        self.breakpoint_ids.clear();
        self.unsent.clear();
        self.started_threads.clear();
        self.last_error = None;
        self.partial.clear();
        self.output_partial = [String::new(), String::new()];
        self.reveal = false;
        self.threads_answered = false;
        self.ended = false;
        self.restart_pending = false;
        self._pump = None;
        self._build_pump = None;
    }

    /// The session ended badly: it never started, the debuggee exited with
    /// a non-zero code, or it ended without one (stopped): the run tab's
    /// icon takes the error badge (batch 13: after Stop).
    pub fn failed(&self) -> bool {
        match self.state {
            SessionState::Failed(_) => true,
            SessionState::Exited(code) => code != Some(0),
            _ => false,
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

    /// The Debugger Console's input (Fleet's "commands" mode, batch 13): a
    /// line is an LLDB command ([`weep_dap::Client::command`]), a line after
    /// a leading `?` an expression evaluated as a watch (the bare value with
    /// its type). Each becomes a block, answered in place.
    pub fn evaluate_in_console(&mut self, line: String, cx: &mut Context<Self>) {
        self.send_console_line(&line);
        cx.emit(DebugSessionEvent::Changed);
        cx.notify();
    }

    /// The block's "rerun": its input again, as a new block.
    pub fn rerun_evaluation(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(input) = self.evaluations.get(ix).map(|e| e.input.clone()) {
            self.evaluate_in_console(input, cx);
        }
    }

    /// [`DebugSession::evaluate_in_console`] without gpui: the block, then
    /// the request (or the error when no adapter runs).
    fn send_console_line(&mut self, line: &str) -> Option<usize> {
        let input = ConsoleInput::classify(line)?;
        let ix = self.evaluations.len();
        self.evaluations.push(Evaluation {
            input: line.trim().to_string(),
            command: matches!(input, ConsoleInput::Command(_)),
            started: std::time::Instant::now(),
            elapsed: None,
            answer: None,
        });
        self.push_line(ConsoleKind::Evaluation(ix), line.trim().to_string());
        let frame_id = self.frame_id();
        let live = !self.state.is_over();
        match self.client.as_mut().filter(|_| live) {
            Some(client) => {
                let id = match &input {
                    ConsoleInput::Command(command) => client.command(command, frame_id),
                    ConsoleInput::Expression(e) => client.evaluate(e, frame_id, EvalContext::Watch),
                };
                self.pending.insert(id, Pending::Console(ix));
            }
            // estimate: Fleet's wording unknown.
            None => self.answer(
                ix,
                EvalAnswer::Error("The debug session is not running".into()),
            ),
        }
        Some(ix)
    }

    /// An evaluation's answer arrived: it and the time it took.
    fn answer(&mut self, ix: usize, answer: EvalAnswer) {
        if let Some(e) = self.evaluations.get_mut(ix) {
            e.elapsed = Some(e.started.elapsed());
            e.answer = Some(answer);
        }
    }

    /// The console's first lines (batch 13, debug-fleet-console): the mode
    /// line, a blank line, then "Launching: <program>" for a launch
    /// (estimate: an attach gets the mode line alone).
    fn console_intro(&mut self, start: &Start) {
        for line in intro_lines(start) {
            self.push_line(ConsoleKind::Console, line);
        }
    }
}

/// [`DebugSession::console_intro`]'s lines.
fn intro_lines(start: &Start) -> Vec<String> {
    let mut lines = vec![CONSOLE_MODE_LINE.to_string()];
    if let Start::Launch(launch) = start {
        lines.push(String::new());
        lines.push(format!("Launching: {}", launch.program.display()));
    }
    lines
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
        assert!(s.output.is_empty());
        s.on_event(Event::Output {
            category: OutputCategory::Stdout,
            text: "lo\r\nworld\n!".into(),
        });
        s.on_event(Event::Output {
            category: OutputCategory::Stderr,
            text: "bad\n".into(),
        });
        s.on_event(Event::Output {
            category: OutputCategory::Console,
            text: "Launched process 1\n".into(),
        });
        // The debuggee's stdout and stderr in the run tab, the adapter's
        // messages in the Debugger Console (batch 13).
        let texts: Vec<_> = s
            .output
            .iter()
            .map(|l| (l.kind, l.text.to_string()))
            .collect();
        assert_eq!(
            texts,
            vec![
                (RunLineKind::Stdout, "hello".into()),
                (RunLineKind::Stdout, "world".into()),
                (RunLineKind::Stderr, "bad".into()),
            ]
        );
        assert_eq!(
            s.console,
            vec![ConsoleLine {
                kind: ConsoleKind::Console,
                text: "Launched process 1".into()
            }]
        );
        s.on_event(Event::Exited(0));
        let e = s.on_event(Event::Terminated);
        assert!(e.ended);
        assert_eq!(s.state, SessionState::Exited(Some(0)));
        assert!(!s.failed());
        assert_eq!(s.output.last().unwrap().text.as_ref(), "!");
        let texts: Vec<_> = s.console.iter().map(|l| l.text.to_string()).collect();
        assert_eq!(&texts[1..], &["Process finished with exit code 0"]);
    }

    #[test]
    fn the_end_decides_the_error_badge() {
        let mut s = DebugSession::new("x".into());
        assert!(!s.failed());
        s.on_event(Event::Exited(9));
        s.on_event(Event::Terminated);
        assert!(s.failed(), "a non-zero exit (Stop kills the debuggee)");
        let mut s = DebugSession::new("x".into());
        running(&mut s);
        s.on_event(Event::Terminated);
        assert!(s.failed(), "ended without an exit code");
    }

    #[test]
    fn a_reset_starts_the_session_over_but_keeps_the_watches() {
        let mut s = DebugSession::new("x".into());
        s.watches.push(Watch {
            expression: "a".into(),
            value: Some(Err("no".into())),
        });
        s.push_output(RunLineKind::Command, "cargo build".into());
        s.push_line(ConsoleKind::Console, "hi".into());
        running(&mut s);
        s.on_event(Event::Exited(1));
        s.on_event(Event::Terminated);
        assert!(s.state.is_over() && s.ended);
        s.reset();
        assert_eq!(s.state, SessionState::Starting);
        assert!(!s.ended && s.output.is_empty() && s.console.is_empty());
        assert_eq!(s.watches.len(), 1);
        assert_eq!(s.watches[0].value, None);
        // Events count again after the reset.
        running(&mut s);
        assert_eq!(s.state, SessionState::Running);
    }

    #[test]
    fn the_adapters_command_line() {
        let adapter = AdapterCommand {
            name: "lldb-dap".into(),
            program: PathBuf::from("/usr/bin/lldb-dap"),
            args: vec!["--x".into()],
        };
        assert_eq!(adapter_line(&adapter), "/usr/bin/lldb-dap --x");
    }

    #[test]
    fn a_stop_asks_threads_frames_scopes_and_watches() {
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
        // The session's first answer asks once more (lldb-dap names the
        // threads from its second answer on).
        assert_eq!(e.asks, vec![Ask::Threads]);
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
        // The scopes arrive collapsed and nothing is fetched (batch 13).
        assert_eq!(e, Effects::default());
        assert_eq!(s.scopes.len(), 2);
        assert!(s.expanded.is_empty());
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
        assert_eq!(e.asks, vec![Ask::StackTrace(2), Ask::Threads]);
        assert_eq!(s.current_thread, Some(2));
    }

    /// lldb-dap's first `threads` answer is the list it took at launch:
    /// `Thread 1` where the next answer says `main` (checked against the
    /// real adapter). The session asks again once, so the first stop shows
    /// the name; later stops and later answers ask nothing more.
    #[test]
    fn the_first_threads_answer_is_asked_again_for_the_names() {
        let thread = |name: &str| {
            Ok(Response::Threads(vec![Thread {
                id: 7,
                name: name.into(),
            }]))
        };
        let mut s = DebugSession::new("x".into());
        running(&mut s);
        let e = s.on_event(stopped(Some(7)));
        assert_eq!(e.asks, vec![Ask::Threads, Ask::StackTrace(7)]);
        let e = s.on_response(Pending::Threads, thread("Thread 1"));
        assert_eq!(e.asks, vec![Ask::Threads]);
        let e = s.on_response(
            Pending::StackTrace(7),
            Ok(Response::StackTrace {
                thread_id: 7,
                frames: vec![frame(70, 5)],
            }),
        );
        assert!(e.stopped);
        let e = s.on_response(Pending::Threads, thread("main"));
        assert!(e.asks.is_empty());
        assert_eq!(s.threads[0].name, "main");
        // The next stop asks once, as before.
        s.on_event(Event::Continued {
            thread_id: Some(7),
            all_threads: true,
        });
        let e = s.on_event(stopped(Some(7)));
        assert_eq!(e.asks, vec![Ask::Threads, Ask::StackTrace(7)]);
        let e = s.on_response(Pending::Threads, thread("main"));
        assert!(e.asks.is_empty());
        // A restart's new adapter starts the count over.
        s.reset();
        running(&mut s);
        s.on_event(stopped(Some(7)));
        let e = s.on_response(Pending::Threads, thread("Thread 1"));
        assert_eq!(e.asks, vec![Ask::Threads]);
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
    fn console_input_is_a_command_unless_it_starts_with_a_question_mark() {
        use ConsoleInput::{Command, Expression};
        assert_eq!(
            ConsoleInput::classify(" bt 2 "),
            Some(Command("bt 2".into()))
        );
        assert_eq!(
            ConsoleInput::classify("total"),
            Some(Command("total".into()))
        );
        assert_eq!(
            ConsoleInput::classify("?total * 2"),
            Some(Expression("total * 2".into()))
        );
        assert_eq!(
            ConsoleInput::classify(" ? a.b "),
            Some(Expression("a.b".into()))
        );
        assert_eq!(ConsoleInput::classify("? "), None);
        assert_eq!(ConsoleInput::classify(""), None);
    }

    #[test]
    fn the_console_hides_the_launch_commands_echo() {
        assert!(is_init_echo("Running initCommands:"));
        assert!(is_init_echo("Running preRunCommands:"));
        assert!(is_init_echo(
            "(lldb) command script import \"/x/lib/rustlib/etc/lldb_lookup.py\""
        ));
        assert!(is_init_echo(
            "(lldb) settings append target.source-map /rustc/abc \"/x\""
        ));
        assert!(!is_init_echo(
            "error: Error reading commands from file lldb_commands - file not found."
        ));
        assert!(!is_init_echo("Running the program:"));
        assert!(!is_init_echo(
            "Process 9 exited with status = 0 (0x00000000)"
        ));
        let mut s = DebugSession::new("x".into());
        s.on_event(Event::Output {
            category: OutputCategory::Console,
            text: "Running initCommands:\n(lldb) command script import \"/a.py\"\n".into(),
        });
        s.on_event(Event::Output {
            category: OutputCategory::Console,
            text: "error: it failed\n".into(),
        });
        let texts: Vec<_> = s.console.iter().map(|l| l.text.to_string()).collect();
        assert_eq!(texts, vec!["error: it failed"]);
    }

    #[test]
    fn the_console_starts_with_the_mode_and_the_program() {
        let launch = Start::Launch(weep_dap::LaunchRequest {
            program: PathBuf::from("/w/target/debug/debuggee"),
            args: Vec::new(),
            cwd: PathBuf::from("/w"),
            env: Vec::new(),
            stop_on_entry: false,
        });
        assert_eq!(
            intro_lines(&launch),
            vec![
                CONSOLE_MODE_LINE.to_string(),
                String::new(),
                "Launching: /w/target/debug/debuggee".into()
            ]
        );
        assert_eq!(
            CONSOLE_MODE_LINE,
            "Console is in 'commands' mode, prefix expressions with '?'."
        );
        assert_eq!(elapsed_text(std::time::Duration::from_micros(3400)), "3ms");
        assert_eq!(elapsed_text(std::time::Duration::from_millis(1250)), "1.2s");
    }

    #[test]
    fn console_answers_and_errors() {
        let mut s = DebugSession::new("x".into());
        // No adapter: each line is a block answered with the error at once.
        assert_eq!(s.send_console_line("  "), None);
        assert_eq!(s.send_console_line("?"), None);
        assert_eq!(s.send_console_line("?total * 2 "), Some(0));
        assert_eq!(s.send_console_line("frame variable"), Some(1));
        assert_eq!(s.send_console_line("?x"), Some(2));
        let e = &s.evaluations[0];
        assert_eq!((e.input.as_str(), e.command), ("?total * 2", false));
        assert!(e.elapsed.is_some());
        assert!(matches!(e.answer, Some(EvalAnswer::Error(_))));
        assert!(s.evaluations[1].command);
        // The answers, by the block they belong to: an expression's value
        // with its type, a command's output, a failure.
        s.evaluations[0].answer = None;
        s.on_response(
            Pending::Console(0),
            Ok(Response::Evaluate {
                result: "28".into(),
                type_name: Some("int".into()),
                variables_reference: 0,
            }),
        );
        s.on_response(
            Pending::Console(1),
            Ok(Response::Evaluate {
                result: "(int) total = 14\n".into(),
                type_name: None,
                variables_reference: 0,
            }),
        );
        s.on_response(Pending::Console(2), Err("no x".into()));
        assert_eq!(
            s.evaluations[0].answer,
            Some(EvalAnswer::Value {
                type_name: Some("int".into()),
                value: "28".into()
            })
        );
        assert_eq!(
            s.evaluations[1].answer,
            Some(EvalAnswer::Output("(int) total = 14\n".into()))
        );
        assert_eq!(
            s.evaluations[2].answer,
            Some(EvalAnswer::Error("no x".into()))
        );
        s.on_event(Event::Error("adapter said\nthings".into()));
        let kinds: Vec<_> = s
            .console
            .iter()
            .map(|l| (l.kind, l.text.to_string()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (ConsoleKind::Evaluation(0), "?total * 2".into()),
                (ConsoleKind::Evaluation(1), "frame variable".into()),
                (ConsoleKind::Evaluation(2), "?x".into()),
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
