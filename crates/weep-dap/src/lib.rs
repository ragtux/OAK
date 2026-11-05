//! A debug adapter client, free of gpui: Fleet's debugger (`docs/roadmap.md`,
//! M8) with the IntelliJ backend replaced by an adapter speaking the Debug
//! Adapter Protocol (lldb-dap first). The app owns one [`Client`] per debug
//! session; answers and events arrive through [`Client::poll`] as
//! [`Event`]s, woken the way `weep-lsp`'s client wakes its owner.
//!
//! Modules: `transport` (the `Content-Length` framing on worker threads),
//! `protocol` (the JSON both ways, 0-based lines to 1-based and back),
//! `client` (the session: handshake, requests, events), `adapters` (where
//! lldb-dap is, and the Rust formatters' `initCommands`).
//!
//! Lines and columns in this API are 0-based (WEEP's); the client converts
//! to the protocol's 1-based ones (`linesStartAt1`).

mod adapters;
mod client;
pub mod protocol;
pub mod transport;

pub use adapters::{find_on_path, rust_init_commands, rust_init_commands_for};
pub use client::Client;

use std::path::PathBuf;

/// A request's id, echoed by its [`Event::Response`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub i64);

/// Where an adapter is and how to start it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterCommand {
    /// Shown in messages, e.g. "lldb-dap".
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// What to start under the debugger (`launch`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchRequest {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub stop_on_entry: bool,
}

/// How a session starts: a new process, or one already running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    Launch(LaunchRequest),
    Attach { pid: u32 },
}

/// A line breakpoint as sent with `setBreakpoints`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceBreakpoint {
    pub line: usize,
    pub condition: Option<String>,
    pub log_message: Option<String>,
}

/// A breakpoint as the adapter reports it back (same order as sent).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Breakpoint {
    pub id: Option<i64>,
    pub verified: bool,
    pub line: Option<usize>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    /// The source file, when the frame has one on disk.
    pub path: Option<PathBuf>,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub name: String,
    pub variables_reference: i64,
    pub expensive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    /// Non-zero when the variable has children ([`Client::variables`]).
    pub variables_reference: i64,
    /// The adapter's expression for this variable (DAP's `evaluateName`),
    /// what "Add to Watches" adds when it is given.
    pub evaluate_name: Option<String>,
}

/// Where an `evaluate` comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalContext {
    Watch,
    Repl,
    Hover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputCategory {
    Console,
    Stdout,
    Stderr,
    Other,
}

/// A request's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    /// A request with no body worth keeping (continue, next, pause, ...).
    Ack,
    SetBreakpoints {
        path: PathBuf,
        breakpoints: Vec<Breakpoint>,
    },
    Threads(Vec<Thread>),
    StackTrace {
        thread_id: i64,
        frames: Vec<StackFrame>,
    },
    Scopes {
        frame_id: i64,
        scopes: Vec<Scope>,
    },
    Variables {
        reference: i64,
        variables: Vec<Variable>,
    },
    Evaluate {
        result: String,
        type_name: Option<String>,
        variables_reference: i64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Response {
        id: RequestId,
        result: Result<Response, String>,
    },
    /// The configuration is done (breakpoints sent, `configurationDone`
    /// answered): the debuggee runs.
    Running,
    Stopped {
        reason: String,
        thread_id: Option<i64>,
        all_threads: bool,
        description: Option<String>,
    },
    Continued {
        thread_id: Option<i64>,
        all_threads: bool,
    },
    Output {
        category: OutputCategory,
        text: String,
    },
    /// The adapter changed a breakpoint by itself (verified it later, moved it).
    BreakpointChanged {
        path: Option<PathBuf>,
        breakpoint: Breakpoint,
    },
    ThreadStarted(i64),
    ThreadExited(i64),
    /// The debuggee exited with this code.
    Exited(i64),
    /// The session is over (the adapter said so, or its pipes closed).
    Terminated,
    /// A message for the user (the adapter's stderr, a failed launch).
    Error(String),
}
