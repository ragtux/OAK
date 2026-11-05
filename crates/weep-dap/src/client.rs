//! One debug session: the adapter spawned on its pipes and taken through
//! the protocol's handshake. `initialize` goes out first; its answer
//! brings the adapter's capabilities and sends `launch` (or `attach`); the
//! adapter's `initialized` event then asks for the configuration: the
//! breakpoints of every file, then `configurationDone`, whose answer is
//! [`Event::Running`]. lldb-dap answers `launch` only after
//! `configurationDone`, so nothing here waits for that answer.
//!
//! Nothing blocks: a request is written by the writer thread and its
//! answer arrives on a later [`Client::poll`] as [`Event::Response`] with
//! the id the request was given; `wake` tells the owner to poll. Requests
//! made before the configuration is sent wait in a queue: breakpoints go
//! out with the configuration, the rest right after it.

use crate::eval::{self, EvalRequest, Walk};
use crate::protocol::{self, Pending};
use crate::transport::{self, Incoming, Wake};
use crate::{AdapterCommand, EvalContext, Event, RequestId, Response, SourceBreakpoint, Start};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;

#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// `initialize` sent, not answered.
    Initializing,
    /// `launch`/`attach` sent; waiting for `initialized`.
    Configuring,
    /// The configuration went out; requests go straight to the pipe.
    Configured,
    /// The adapter exited, or was killed.
    Over,
}

/// A request made before the configuration went out.
struct Queued {
    id: RequestId,
    command: &'static str,
    arguments: Value,
    pending: Pending,
}

/// One debug session: the adapter process and the protocol state.
pub struct Client {
    name: String,
    child: Child,
    out: Sender<Vec<u8>>,
    incoming: Receiver<Incoming>,
    stderr: Receiver<String>,
    wake: Wake,
    /// The seq of the next message on the wire (requests and the answers
    /// to the adapter's requests share it).
    next_seq: i64,
    /// The next id handed to the owner.
    next_id: i64,
    phase: Phase,
    start: Option<Start>,
    attach: bool,
    /// The breakpoints to send on `initialized`, with the ids their
    /// answers will carry.
    initial_breakpoints: Vec<(RequestId, PathBuf, Vec<SourceBreakpoint>)>,
    queued: Vec<Queued>,
    /// Requests on the wire by seq: the owner's id (none for the client's
    /// own) and what the answer means.
    pending: HashMap<i64, (Option<RequestId>, Pending)>,
    capabilities: Value,
    /// The file of each breakpoint id the adapter answered: its
    /// `breakpoint` events leave the source out.
    breakpoint_paths: HashMap<i64, PathBuf>,
    /// Events made here rather than read off the pipe (a request after
    /// the end, a failed handshake); they come first on the next poll.
    local: Vec<Event>,
    terminate_sent: bool,
    disconnect_sent: bool,
    terminated_reported: bool,
    running_reported: bool,
    /// A `pause` went out and no stop came yet (lldb-dap reports the stop
    /// a pause makes as an "exception", "signal SIGSTOP").
    pausing: bool,
    /// `stopOnEntry` was asked and the first stop has not come yet (the
    /// same SIGSTOP "exception" in lldb-dap).
    entry_pending: bool,
}

impl Client {
    /// Starts `adapter`, sends `initialize`, then `launch`/`attach`; on the
    /// adapter's `initialized` event sends `breakpoints` (per file) and
    /// `configurationDone`, then reports [`Event::Running`]. `wake` runs on
    /// a worker thread whenever something arrived, so the owner knows to
    /// [`Client::poll`].
    ///
    /// The answers to these first `setBreakpoints` arrive as
    /// [`Response::SetBreakpoints`] with ids the owner never saw: match
    /// them by their path. Rust's formatters
    /// ([`crate::rust_init_commands`]) go with the `launch`/`attach`.
    pub fn start(
        adapter: &AdapterCommand,
        start: Start,
        breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Client, String> {
        let wake: Wake = Arc::new(wake);
        let mut cmd = std::process::Command::new(&adapter.program);
        cmd.args(&adapter.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Start::Launch(launch) = &start {
            if launch.cwd.is_dir() {
                cmd.current_dir(&launch.cwd);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", adapter.program.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let (out_tx, out_rx) = channel::<Vec<u8>>();
        let (in_tx, in_rx) = channel::<Incoming>();
        let (err_tx, err_rx) = channel::<String>();
        transport::spawn_writer(Box::new(stdin), out_rx);
        transport::spawn_reader(Box::new(stdout), in_tx, wake.clone());
        transport::spawn_stderr(Box::new(stderr), err_tx, wake.clone());
        let attach = matches!(start, Start::Attach { .. });
        let entry_pending = matches!(&start, Start::Launch(l) if l.stop_on_entry);
        let mut client = Client {
            name: adapter.name.clone(),
            child,
            out: out_tx,
            incoming: in_rx,
            stderr: err_rx,
            wake,
            next_seq: 1,
            next_id: 1,
            phase: Phase::Initializing,
            start: Some(start),
            attach,
            initial_breakpoints: Vec::new(),
            queued: Vec::new(),
            pending: HashMap::new(),
            capabilities: Value::Null,
            breakpoint_paths: HashMap::new(),
            local: Vec::new(),
            terminate_sent: false,
            disconnect_sent: false,
            terminated_reported: false,
            running_reported: false,
            pausing: false,
            entry_pending,
        };
        client.initial_breakpoints = breakpoints
            .into_iter()
            .map(|(path, rows)| (client.allocate(), path, rows))
            .collect();
        client.write(
            None,
            "initialize",
            protocol::initialize_arguments(),
            Pending::Initialize,
        );
        Ok(client)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The adapter's answer to `initialize` (`supportsConditionalBreakpoints`,
    /// `exceptionBreakpointFilters`, ...); null until it arrived.
    pub fn capabilities(&self) -> &Value {
        &self.capabilities
    }

    /// Whether the adapter declared the `supports<Name>` capability, e.g.
    /// `supports("supportsTerminateRequest")`.
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities
            .get(capability)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Whether the session attached to a running process (rather than
    /// launching one).
    pub fn is_attach(&self) -> bool {
        self.attach
    }

    /// Whether the session is over (the adapter exited or was killed).
    pub fn is_over(&self) -> bool {
        self.phase == Phase::Over
    }

    /// The adapter's process id.
    pub fn adapter_pid(&self) -> u32 {
        self.child.id()
    }

    fn allocate(&mut self) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Puts a request on the wire now.
    fn write(&mut self, id: Option<RequestId>, command: &str, arguments: Value, pending: Pending) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let body = json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        });
        self.pending.insert(seq, (id, pending));
        let _ = self.out.send(transport::frame(&body.to_string()));
    }

    /// One of the owner's requests: on the wire once the configuration
    /// went out, queued before, answered with an error after the end.
    fn request(&mut self, command: &'static str, arguments: Value, pending: Pending) -> RequestId {
        let id = self.allocate();
        match self.phase {
            Phase::Configured => self.write(Some(id), command, arguments, pending),
            Phase::Initializing | Phase::Configuring => self.queued.push(Queued {
                id,
                command,
                arguments,
                pending,
            }),
            Phase::Over => {
                self.local.push(Event::Response {
                    id,
                    result: Err(format!("{}: the debug session is over", self.name)),
                });
                (self.wake)();
            }
        }
        id
    }

    /// Answers one of the adapter's requests.
    fn respond(&mut self, request_seq: i64, command: &str, result: Result<Value, String>) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let body = match result {
            Ok(body) => json!({
                "seq": seq, "type": "response", "request_seq": request_seq,
                "success": true, "command": command, "body": body,
            }),
            Err(message) => json!({
                "seq": seq, "type": "response", "request_seq": request_seq,
                "success": false, "command": command, "message": message,
            }),
        };
        let _ = self.out.send(transport::frame(&body.to_string()));
    }

    /// Everything that arrived since the last call; never blocks.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = std::mem::take(&mut self.local);
        while let Ok(line) = self.stderr.try_recv() {
            if !line.trim().is_empty() {
                events.push(Event::Error(format!("{}: {line}", self.name)));
            }
        }
        let mut closed = false;
        loop {
            match self.incoming.try_recv() {
                Ok(message) => self.handle(message, &mut events),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        if closed && self.phase != Phase::Over {
            // The adapter's stdout closed: it is gone, whatever it said.
            let _ = self.child.wait();
            self.phase = Phase::Over;
            self.fail_queued(&mut events);
            self.report_terminated(&mut events);
        }
        events
    }

    fn report_running(&mut self, events: &mut Vec<Event>) {
        if !self.running_reported {
            self.running_reported = true;
            events.push(Event::Running);
        }
    }

    fn report_terminated(&mut self, events: &mut Vec<Event>) {
        if !self.terminated_reported {
            self.terminated_reported = true;
            events.push(Event::Terminated);
        }
    }

    /// Answers every request that will now never go out.
    fn fail_queued(&mut self, events: &mut Vec<Event>) {
        for q in std::mem::take(&mut self.queued) {
            events.push(Event::Response {
                id: q.id,
                result: Err(format!("{}: the debug session is over", self.name)),
            });
        }
        for (id, _) in std::mem::take(&mut self.pending).into_values() {
            if let Some(id) = id {
                events.push(Event::Response {
                    id,
                    result: Err(format!("{}: the debug session is over", self.name)),
                });
            }
        }
    }

    fn handle(&mut self, message: Incoming, events: &mut Vec<Event>) {
        match message {
            Incoming::Response {
                request_seq,
                success,
                command,
                message,
                body,
            } => {
                let Some((id, pending)) = self.pending.remove(&request_seq) else {
                    return;
                };
                let error =
                    (!success).then(|| protocol::error_text(&command, message.as_deref(), &body));
                self.answered(id, pending, error, &body, events);
            }
            Incoming::Event { event, body } => self.event(&event, body, events),
            Incoming::Request { seq, command, .. } => {
                // runInTerminal is declined in `initialize`; startDebugging
                // (child sessions) is not WEEP's yet.
                let message = format!("WEEP does not support the {command} request");
                self.respond(seq, &command, Err(message));
            }
        }
    }

    fn answered(
        &mut self,
        id: Option<RequestId>,
        pending: Pending,
        error: Option<String>,
        body: &Value,
        events: &mut Vec<Event>,
    ) {
        match pending {
            Pending::Initialize => match error {
                None => {
                    self.capabilities = body.clone();
                    self.phase = Phase::Configuring;
                    let init = crate::rust_init_commands();
                    match self.start.take() {
                        Some(Start::Launch(launch)) => self.write(
                            None,
                            "launch",
                            protocol::launch_arguments(&launch, &init),
                            Pending::Launch,
                        ),
                        Some(Start::Attach { pid }) => self.write(
                            None,
                            "attach",
                            protocol::attach_arguments(pid, &init),
                            Pending::Attach,
                        ),
                        None => {}
                    }
                }
                Some(e) => self.give_up(format!("{}: initialize: {e}", self.name), events),
            },
            Pending::Launch | Pending::Attach => {
                if let Some(e) = error {
                    let what = if pending == Pending::Launch {
                        "launch"
                    } else {
                        "attach"
                    };
                    self.give_up(format!("{what}: {e}"), events);
                }
            }
            Pending::ConfigurationDone => match error {
                None => self.report_running(events),
                Some(e) => events.push(Event::Error(format!("configurationDone: {e}"))),
            },
            Pending::Terminate => {
                // The adapter ends the debuggee and says `terminated`; the
                // adapter itself goes with `disconnect`, sent here in case
                // that event never comes.
                self.disconnect();
            }
            Pending::Disconnect => {}
            Pending::Evaluate(request) => {
                if let Some(id) = id {
                    self.evaluated(id, request, error, body, events);
                }
            }
            Pending::WalkScopes(walk) => {
                if let Some(id) = id {
                    self.walked(id, walk, true, error, body, events);
                }
            }
            Pending::WalkVariables(walk) => {
                if let Some(id) = id {
                    self.walked(id, walk, false, error, body, events);
                }
            }
            pending => {
                if let Some(id) = id {
                    let result = match error {
                        None => Ok(protocol::response(&pending, body)),
                        Some(e) => Err(e),
                    };
                    if let Ok(Response::SetBreakpoints { path, breakpoints }) = &result {
                        for id in breakpoints.iter().filter_map(|b| b.id) {
                            self.breakpoint_paths.insert(id, path.clone());
                        }
                    }
                    events.push(Event::Response { id, result });
                }
            }
        }
    }

    /// The session cannot go on: says why, ends the adapter.
    fn give_up(&mut self, message: String, events: &mut Vec<Event>) {
        events.push(Event::Error(message));
        self.kill_now();
        self.fail_queued(events);
        self.report_terminated(events);
    }

    fn event(&mut self, name: &str, body: Value, events: &mut Vec<Event>) {
        match name {
            "initialized" => self.configure(events),
            "capabilities" => {
                if let (Some(new), Some(old)) = (
                    body.get("capabilities").and_then(Value::as_object),
                    self.capabilities.as_object_mut(),
                ) {
                    for (k, v) in new {
                        old.insert(k.clone(), v.clone());
                    }
                }
            }
            "terminated" => {
                self.report_terminated(events);
                self.disconnect();
            }
            _ => match protocol::event(name, &body) {
                Some(Event::BreakpointChanged {
                    path: None,
                    breakpoint,
                }) => {
                    let path = breakpoint
                        .id
                        .and_then(|id| self.breakpoint_paths.get(&id).cloned());
                    events.push(Event::BreakpointChanged { path, breakpoint });
                }
                Some(Event::Stopped {
                    reason,
                    thread_id,
                    all_threads,
                    description,
                }) => {
                    // lldb-dap may stop (on entry, at once) before it
                    // answers `configurationDone`: the session ran first.
                    self.report_running(events);
                    let sigstop = reason == "exception"
                        && description
                            .as_deref()
                            .is_some_and(|d| d.contains("SIGSTOP"));
                    let reason = if sigstop && self.entry_pending {
                        "entry".to_string()
                    } else if sigstop && self.pausing {
                        "pause".to_string()
                    } else {
                        reason
                    };
                    self.entry_pending = false;
                    self.pausing = false;
                    events.push(Event::Stopped {
                        reason,
                        thread_id,
                        all_threads,
                        description,
                    });
                }
                Some(e @ Event::Exited(_)) => {
                    self.report_running(events);
                    events.push(e);
                }
                Some(e) => events.push(e),
                None => {}
            },
        }
    }

    /// `initialized`: the breakpoints, the owner's queued breakpoints, then
    /// `configurationDone` and the rest of the queue.
    fn configure(&mut self, events: &mut Vec<Event>) {
        if self.phase != Phase::Configuring {
            return;
        }
        for (id, path, rows) in std::mem::take(&mut self.initial_breakpoints) {
            let args = protocol::set_breakpoints_arguments(&path, &rows);
            self.write(
                Some(id),
                "setBreakpoints",
                args,
                Pending::SetBreakpoints(path),
            );
        }
        let (configuration, rest): (Vec<Queued>, Vec<Queued>) = std::mem::take(&mut self.queued)
            .into_iter()
            .partition(|q| matches!(q.pending, Pending::SetBreakpoints(_)));
        for q in configuration {
            self.write(Some(q.id), q.command, q.arguments, q.pending);
        }
        if self.supports("supportsConfigurationDoneRequest") {
            self.write(
                None,
                "configurationDone",
                json!({}),
                Pending::ConfigurationDone,
            );
        } else {
            self.report_running(events);
        }
        self.phase = Phase::Configured;
        for q in rest {
            self.write(Some(q.id), q.command, q.arguments, q.pending);
        }
    }

    /// Replaces the breakpoints of one file (an empty list clears them).
    pub fn set_breakpoints(&mut self, path: &Path, breakpoints: &[SourceBreakpoint]) -> RequestId {
        let args = protocol::set_breakpoints_arguments(path, breakpoints);
        self.request(
            "setBreakpoints",
            args,
            Pending::SetBreakpoints(path.to_path_buf()),
        )
    }

    pub fn threads(&mut self) -> RequestId {
        self.request("threads", json!({}), Pending::Threads)
    }

    /// Every frame of the thread, innermost first.
    pub fn stack_trace(&mut self, thread_id: i64) -> RequestId {
        self.request(
            "stackTrace",
            json!({"threadId": thread_id, "startFrame": 0}),
            Pending::StackTrace(thread_id),
        )
    }

    pub fn scopes(&mut self, frame_id: i64) -> RequestId {
        self.request(
            "scopes",
            json!({"frameId": frame_id}),
            Pending::Scopes(frame_id),
        )
    }

    pub fn variables(&mut self, reference: i64) -> RequestId {
        self.request(
            "variables",
            json!({"variablesReference": reference}),
            Pending::Variables(reference),
        )
    }

    /// Evaluates `expression` in the frame, answered by
    /// [`Response::Evaluate`] (or one line of error). Rust through LLDB
    /// (see `eval.rs`): a path with a tuple field (`pair.0`, which LLDB's
    /// evaluator misreads) is looked up in the variables tree; a path the
    /// adapter cannot evaluate is retried there; a failed `hover` of
    /// anything else is retried in `watch` (the expression evaluator, for
    /// a selection's `total * 2`); a failure is LLDB's one error line.
    pub fn evaluate(
        &mut self,
        expression: &str,
        frame_id: Option<i64>,
        context: EvalContext,
    ) -> RequestId {
        if let (Some(path), Some(frame_id)) = (eval::rust_path(expression), frame_id) {
            if eval::has_tuple_field(&path) {
                let walk = Walk::new(expression, context, path);
                return self.request(
                    "scopes",
                    json!({"frameId": frame_id}),
                    Pending::WalkScopes(walk),
                );
            }
        }
        let args = protocol::evaluate_arguments(expression, frame_id, context);
        let request = EvalRequest {
            expression: expression.to_string(),
            frame_id,
            context,
            retried: false,
        };
        self.request("evaluate", args, Pending::Evaluate(request))
    }

    /// The answer to an `evaluate`: the owner's, or a fallback's request.
    fn evaluated(
        &mut self,
        id: RequestId,
        request: EvalRequest,
        error: Option<String>,
        body: &Value,
        events: &mut Vec<Event>,
    ) {
        let Some(error) = error else {
            let response = protocol::response(&Pending::Evaluate(request), body);
            events.push(Event::Response {
                id,
                result: Ok(response),
            });
            return;
        };
        let cleaned = eval::clean_error(&error);
        if self.phase == Phase::Configured {
            if let (Some(path), Some(frame_id)) =
                (eval::rust_path(&request.expression), request.frame_id)
            {
                let mut walk = Walk::new(&request.expression, request.context, path);
                walk.error = Some(cleaned);
                self.write(
                    Some(id),
                    "scopes",
                    json!({"frameId": frame_id}),
                    Pending::WalkScopes(walk),
                );
                return;
            }
            if request.context == EvalContext::Hover && !request.retried {
                let args = protocol::evaluate_arguments(
                    &request.expression,
                    request.frame_id,
                    EvalContext::Watch,
                );
                let request = EvalRequest {
                    retried: true,
                    ..request
                };
                self.write(Some(id), "evaluate", args, Pending::Evaluate(request));
                return;
            }
        }
        events.push(Event::Response {
            id,
            result: Err(cleaned),
        });
    }

    /// One step of an evaluation through the variables tree: the frame's
    /// scopes, a scope's variables, or a variable's children.
    fn walked(
        &mut self,
        id: RequestId,
        mut walk: Walk,
        scopes: bool,
        error: Option<String>,
        body: &Value,
        events: &mut Vec<Event>,
    ) {
        let fail = |walk: &Walk, segment: &str, events: &mut Vec<Event>| {
            events.push(Event::Response {
                id,
                result: Err(walk.not_found(segment)),
            });
        };
        let segment = walk.path.first().cloned().unwrap_or_default();
        if let Some(e) = error {
            let message = walk.error.clone().unwrap_or_else(|| eval::clean_error(&e));
            events.push(Event::Response {
                id,
                result: Err(message),
            });
            return;
        }
        if scopes {
            // The frame's scopes, registers aside: searched in order.
            walk.scopes = body
                .get("scopes")
                .and_then(Value::as_array)
                .map(|rows| rows.iter().filter_map(protocol::scope).collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s.variables_reference > 0 && !s.name.contains("Register"))
                .map(|s| s.variables_reference)
                .collect();
            return self.walk_next_scope(id, walk, events);
        }
        let variables: Vec<crate::Variable> = body
            .get("variables")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(protocol::variable).collect())
            .unwrap_or_default();
        let Some(found) = variables
            .into_iter()
            .find(|v| eval::child_matches(&v.name, &segment))
        else {
            if walk.at_root && !walk.scopes.is_empty() {
                return self.walk_next_scope(id, walk, events);
            }
            return fail(&walk, &segment, events);
        };
        walk.path.remove(0);
        if walk.path.is_empty() {
            events.push(Event::Response {
                id,
                result: Ok(Response::Evaluate {
                    result: walk.result_text(&found.value, found.type_name.as_deref()),
                    type_name: found.type_name,
                    variables_reference: found.variables_reference,
                }),
            });
            return;
        }
        walk.at_root = false;
        walk.scopes.clear();
        walk.parent_type = found.type_name.clone();
        if found.variables_reference <= 0 || self.phase != Phase::Configured {
            let next = walk.path[0].clone();
            return fail(&walk, &next, events);
        }
        self.write(
            Some(id),
            "variables",
            json!({"variablesReference": found.variables_reference}),
            Pending::WalkVariables(walk),
        );
    }

    /// Reads the next scope's variables for the walk's first segment.
    fn walk_next_scope(&mut self, id: RequestId, mut walk: Walk, events: &mut Vec<Event>) {
        if walk.scopes.is_empty() || self.phase != Phase::Configured {
            let segment = walk.path.first().cloned().unwrap_or_default();
            events.push(Event::Response {
                id,
                result: Err(walk.not_found(&segment)),
            });
            return;
        }
        let reference = walk.scopes.remove(0);
        self.write(
            Some(id),
            "variables",
            json!({"variablesReference": reference}),
            Pending::WalkVariables(walk),
        );
    }

    /// `continue`. The answer ([`Response::Ack`]) means the debuggee runs
    /// again: the protocol sends no `continued` for it (lldb-dap does, but
    /// other adapters need not). So for the steps.
    pub fn resume(&mut self, thread_id: i64) -> RequestId {
        self.request("continue", json!({"threadId": thread_id}), Pending::Ack)
    }

    /// `pause`: a `stopped` event with reason "pause" follows (lldb-dap's
    /// "exception" for the SIGSTOP is reported as "pause"; so is the first
    /// stop of a `stopOnEntry` launch, as "entry").
    pub fn pause(&mut self, thread_id: i64) -> RequestId {
        self.pausing = true;
        self.request("pause", json!({"threadId": thread_id}), Pending::Ack)
    }

    /// `next`: a `stopped` event with reason "step" follows.
    pub fn step_over(&mut self, thread_id: i64) -> RequestId {
        self.request("next", json!({"threadId": thread_id}), Pending::Ack)
    }

    pub fn step_into(&mut self, thread_id: i64) -> RequestId {
        self.request("stepIn", json!({"threadId": thread_id}), Pending::Ack)
    }

    pub fn step_out(&mut self, thread_id: i64) -> RequestId {
        self.request("stepOut", json!({"threadId": thread_id}), Pending::Ack)
    }

    /// Ends the session: `terminate` the debuggee when the adapter can
    /// (a launch), else `disconnect` (ending a launched debuggee, leaving
    /// an attached one running); `disconnect` follows `terminate`, and
    /// [`Event::Terminated`] arrives when the adapter is gone. A second
    /// call while the first is under way kills the adapter.
    pub fn stop(&mut self) {
        match self.phase {
            Phase::Over => {}
            Phase::Initializing => {
                self.kill();
            }
            _ if self.terminate_sent || self.disconnect_sent => self.kill(),
            Phase::Configuring | Phase::Configured => {
                if !self.attach && self.supports("supportsTerminateRequest") {
                    self.terminate_sent = true;
                    self.write(None, "terminate", json!({}), Pending::Terminate);
                } else {
                    self.disconnect();
                }
            }
        }
    }

    fn disconnect(&mut self) {
        if self.disconnect_sent || self.phase == Phase::Over {
            return;
        }
        self.disconnect_sent = true;
        self.write(
            None,
            "disconnect",
            json!({"restart": false, "terminateDebuggee": !self.attach}),
            Pending::Disconnect,
        );
    }

    fn kill_now(&mut self) {
        if self.phase != Phase::Over {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.phase = Phase::Over;
        }
    }

    /// Kills the adapter without asking (the window closes);
    /// [`Event::Terminated`] comes on the next poll.
    pub fn kill(&mut self) {
        if self.phase == Phase::Over {
            return;
        }
        self.kill_now();
        let mut events = Vec::new();
        self.fail_queued(&mut events);
        self.report_terminated(&mut events);
        self.local.extend(events);
        (self.wake)();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.kill_now();
    }
}

#[allow(dead_code)]
fn _assert_send() {
    fn is_send<T: Send>() {}
    is_send::<Client>();
}
