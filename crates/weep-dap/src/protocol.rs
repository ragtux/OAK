//! The protocol's JSON as WEEP's types, both ways: request arguments from
//! the API's values (0-based lines made 1-based) and responses and events
//! into [`Response`] and [`Event`] (1-based lines made 0-based). Pure
//! functions, so the mapping is tested without an adapter.

use crate::{
    Breakpoint, EvalContext, Event, LaunchRequest, OutputCategory, Response, Scope,
    SourceBreakpoint, StackFrame, Thread, Variable,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What a request was, kept by its seq until its response arrives, so the
/// answer carries the request's context (the path, the thread, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pending {
    Initialize,
    Launch,
    Attach,
    ConfigurationDone,
    Terminate,
    Disconnect,
    SetBreakpoints(PathBuf),
    Threads,
    StackTrace(i64),
    Scopes(i64),
    Variables(i64),
    Evaluate,
    /// continue, next, stepIn, stepOut, pause: nothing to keep.
    Ack,
}

/// A 1-based protocol line or column as WEEP's 0-based one.
pub fn from_protocol(n: i64) -> usize {
    (n.max(1) - 1) as usize
}

/// A 0-based line or column as the protocol's 1-based one.
pub fn to_protocol(n: usize) -> i64 {
    n as i64 + 1
}

pub(crate) fn initialize_arguments() -> Value {
    json!({
        "clientID": "weep",
        "clientName": "WEEP",
        "adapterID": "lldb-dap",
        "locale": "en-US",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsVariableType": true,
        "supportsVariablePaging": false,
        "supportsRunInTerminalRequest": false,
        "supportsMemoryReferences": false,
        "supportsProgressReporting": false,
        "supportsInvalidatedEvent": false,
        "supportsStartDebuggingRequest": false,
    })
}

/// lldb-dap's `launch` arguments: the program, its arguments, folder and
/// environment (`K=V` strings), `stopOnEntry`, and the LLDB commands to
/// run before the target is created (the Rust formatters).
pub fn launch_arguments(launch: &LaunchRequest, init_commands: &[String]) -> Value {
    let mut args = json!({
        "program": launch.program,
        "args": launch.args,
        "env": launch
            .env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>(),
        "stopOnEntry": launch.stop_on_entry,
    });
    if !launch.cwd.as_os_str().is_empty() {
        args["cwd"] = json!(launch.cwd);
    }
    if !init_commands.is_empty() {
        args["initCommands"] = json!(init_commands);
    }
    args
}

/// lldb-dap's `attach` arguments for a running process.
pub fn attach_arguments(pid: u32, init_commands: &[String]) -> Value {
    let mut args = json!({ "pid": pid });
    if !init_commands.is_empty() {
        args["initCommands"] = json!(init_commands);
    }
    args
}

pub fn set_breakpoints_arguments(path: &Path, breakpoints: &[SourceBreakpoint]) -> Value {
    let rows: Vec<Value> = breakpoints
        .iter()
        .map(|b| {
            let mut row = json!({ "line": to_protocol(b.line) });
            if let Some(c) = b.condition.as_ref().filter(|c| !c.is_empty()) {
                row["condition"] = json!(c);
            }
            if let Some(m) = b.log_message.as_ref().filter(|m| !m.is_empty()) {
                row["logMessage"] = json!(m);
            }
            row
        })
        .collect();
    json!({
        "source": {
            "path": path,
            "name": path.file_name().map(|n| n.to_string_lossy().into_owned()),
        },
        "breakpoints": rows,
        "sourceModified": false,
    })
}

pub fn evaluate_arguments(expression: &str, frame_id: Option<i64>, context: EvalContext) -> Value {
    let mut args = json!({
        "expression": expression,
        "context": match context {
            EvalContext::Watch => "watch",
            EvalContext::Repl => "repl",
            EvalContext::Hover => "hover",
        },
    });
    if let Some(id) = frame_id {
        args["frameId"] = json!(id);
    }
    args
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn int_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

/// The `source.path` of a frame or breakpoint, when it names a file: a
/// source the adapter serves itself (`sourceReference` > 0, e.g. lldb-dap's
/// disassembly of `/usr/lib/dyld`start`) has no file behind its path.
fn source_path(v: &Value) -> Option<PathBuf> {
    let source = v.get("source")?;
    if source
        .get("sourceReference")
        .and_then(Value::as_i64)
        .is_some_and(|r| r > 0)
    {
        return None;
    }
    source
        .get("path")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

pub fn breakpoint(v: &Value) -> Breakpoint {
    Breakpoint {
        id: int_field(v, "id"),
        verified: v.get("verified").and_then(Value::as_bool).unwrap_or(false),
        line: int_field(v, "line").map(from_protocol),
        message: str_field(v, "message"),
    }
}

pub fn thread(v: &Value) -> Option<Thread> {
    Some(Thread {
        id: int_field(v, "id")?,
        name: str_field(v, "name").unwrap_or_default(),
    })
}

pub fn stack_frame(v: &Value) -> Option<StackFrame> {
    Some(StackFrame {
        id: int_field(v, "id")?,
        name: str_field(v, "name").unwrap_or_default(),
        path: source_path(v),
        line: from_protocol(int_field(v, "line").unwrap_or(0)),
        column: from_protocol(int_field(v, "column").unwrap_or(0)),
    })
}

pub fn scope(v: &Value) -> Option<Scope> {
    Some(Scope {
        name: str_field(v, "name")?,
        variables_reference: int_field(v, "variablesReference").unwrap_or(0),
        expensive: v.get("expensive").and_then(Value::as_bool).unwrap_or(false),
    })
}

pub fn variable(v: &Value) -> Option<Variable> {
    Some(Variable {
        name: str_field(v, "name")?,
        value: str_field(v, "value").unwrap_or_default(),
        type_name: str_field(v, "type").filter(|t| !t.is_empty()),
        variables_reference: int_field(v, "variablesReference").unwrap_or(0),
        evaluate_name: str_field(v, "evaluateName").filter(|n| !n.is_empty()),
    })
}

fn list<T>(body: &Value, key: &str, item: impl Fn(&Value) -> Option<T>) -> Vec<T> {
    body.get(key)
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(item).collect())
        .unwrap_or_default()
}

/// A failed response's text: the formatted `body.error` when the adapter
/// gave one, else its `message`, else the command.
pub fn error_text(command: &str, message: Option<&str>, body: &Value) -> String {
    let detail = body
        .get("error")
        .and_then(|e| e.get("format"))
        .and_then(Value::as_str);
    match (detail, message) {
        (Some(d), _) if !d.is_empty() => d.to_string(),
        (_, Some(m)) if !m.is_empty() => m.to_string(),
        _ => format!("{command} failed"),
    }
}

/// A response to one of the owner's requests as the API's [`Response`].
pub(crate) fn response(pending: &Pending, body: &Value) -> Response {
    match pending {
        Pending::SetBreakpoints(path) => Response::SetBreakpoints {
            path: path.clone(),
            breakpoints: body
                .get("breakpoints")
                .and_then(Value::as_array)
                .map(|rows| rows.iter().map(breakpoint).collect())
                .unwrap_or_default(),
        },
        Pending::Threads => Response::Threads(list(body, "threads", thread)),
        Pending::StackTrace(thread_id) => Response::StackTrace {
            thread_id: *thread_id,
            frames: list(body, "stackFrames", stack_frame),
        },
        Pending::Scopes(frame_id) => Response::Scopes {
            frame_id: *frame_id,
            scopes: list(body, "scopes", scope),
        },
        Pending::Variables(reference) => Response::Variables {
            reference: *reference,
            variables: list(body, "variables", variable),
        },
        Pending::Evaluate => Response::Evaluate {
            result: str_field(body, "result").unwrap_or_default(),
            type_name: str_field(body, "type").filter(|t| !t.is_empty()),
            variables_reference: int_field(body, "variablesReference").unwrap_or(0),
        },
        _ => Response::Ack,
    }
}

pub fn output_category(category: Option<&str>) -> Option<OutputCategory> {
    match category {
        // The protocol's default is the console.
        None | Some("console") | Some("important") => Some(OutputCategory::Console),
        Some("stdout") => Some(OutputCategory::Stdout),
        Some("stderr") => Some(OutputCategory::Stderr),
        // Telemetry is the adapter's business, not the user's.
        Some("telemetry") => None,
        Some(_) => Some(OutputCategory::Other),
    }
}

/// An adapter event as the API's [`Event`]; `None` for the ones the API
/// has no use for (and `initialized`, which the client handles itself).
pub fn event(name: &str, body: &Value) -> Option<Event> {
    match name {
        "stopped" => Some(Event::Stopped {
            reason: str_field(body, "reason").unwrap_or_default(),
            thread_id: int_field(body, "threadId"),
            all_threads: body
                .get("allThreadsStopped")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            description: str_field(body, "description").or_else(|| str_field(body, "text")),
        }),
        "continued" => Some(Event::Continued {
            thread_id: int_field(body, "threadId"),
            all_threads: body
                .get("allThreadsContinued")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }),
        "output" => {
            let category = output_category(body.get("category").and_then(Value::as_str))?;
            // lldb-dap runs the debuggee on a pty: its lines end in CRLF.
            let text = str_field(body, "output")?.replace("\r\n", "\n");
            Some(Event::Output { category, text })
        }
        "breakpoint" => {
            let bp = body.get("breakpoint")?;
            Some(Event::BreakpointChanged {
                path: source_path(bp),
                breakpoint: breakpoint(bp),
            })
        }
        "thread" => {
            let id = int_field(body, "threadId")?;
            match body.get("reason").and_then(Value::as_str)? {
                "started" => Some(Event::ThreadStarted(id)),
                "exited" => Some(Event::ThreadExited(id)),
                _ => None,
            }
        }
        "exited" => Some(Event::Exited(int_field(body, "exitCode").unwrap_or(0))),
        "terminated" => Some(Event::Terminated),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_convert_both_ways() {
        assert_eq!(from_protocol(1), 0);
        assert_eq!(from_protocol(12), 11);
        // A frame without a source says line 0: kept at the top, not wrapped.
        assert_eq!(from_protocol(0), 0);
        assert_eq!(to_protocol(0), 1);
        assert_eq!(to_protocol(41), 42);
    }

    #[test]
    fn breakpoints_go_out_one_based() {
        let args = set_breakpoints_arguments(
            Path::new("/src/main.c"),
            &[
                SourceBreakpoint {
                    line: 4,
                    ..Default::default()
                },
                SourceBreakpoint {
                    line: 9,
                    condition: Some("i == 3".into()),
                    log_message: Some(String::new()),
                },
            ],
        );
        assert_eq!(args["source"]["path"], json!("/src/main.c"));
        assert_eq!(args["source"]["name"], json!("main.c"));
        assert_eq!(args["breakpoints"][0], json!({"line": 5}));
        assert_eq!(
            args["breakpoints"][1],
            json!({"line": 10, "condition": "i == 3"})
        );
    }

    #[test]
    fn breakpoints_come_back_zero_based_with_their_path() {
        let body = json!({"breakpoints": [
            {"id": 1, "verified": true, "line": 5},
            {"id": 2, "verified": false, "message": "no code"}
        ]});
        let r = response(&Pending::SetBreakpoints("/src/main.c".into()), &body);
        assert_eq!(
            r,
            Response::SetBreakpoints {
                path: "/src/main.c".into(),
                breakpoints: vec![
                    Breakpoint {
                        id: Some(1),
                        verified: true,
                        line: Some(4),
                        message: None
                    },
                    Breakpoint {
                        id: Some(2),
                        verified: false,
                        line: None,
                        message: Some("no code".into())
                    },
                ]
            }
        );
    }

    #[test]
    fn frames_scopes_variables_keep_their_context() {
        let body = json!({"stackFrames": [
            {"id": 7, "name": "main", "line": 6, "column": 5, "source": {"path": "/src/main.c", "name": "main.c"}},
            {"id": 8, "name": "start", "line": 0, "column": 0, "source": {"name": "start", "path": "/usr/lib/dyld`start", "sourceReference": 3}}
        ], "totalFrames": 2});
        let Response::StackTrace { thread_id, frames } = response(&Pending::StackTrace(3), &body)
        else {
            panic!()
        };
        assert_eq!(thread_id, 3);
        assert_eq!(frames[0].path.as_deref(), Some(Path::new("/src/main.c")));
        assert_eq!((frames[0].line, frames[0].column), (5, 4));
        assert_eq!(frames[1].path, None);

        let body =
            json!({"scopes": [{"name": "Locals", "variablesReference": 11, "expensive": false}]});
        assert_eq!(
            response(&Pending::Scopes(7), &body),
            Response::Scopes {
                frame_id: 7,
                scopes: vec![Scope {
                    name: "Locals".into(),
                    variables_reference: 11,
                    expensive: false
                }]
            }
        );

        let body = json!({"variables": [{"name": "x", "value": "42", "type": "int", "variablesReference": 0}]});
        assert_eq!(
            response(&Pending::Variables(11), &body),
            Response::Variables {
                reference: 11,
                variables: vec![Variable {
                    name: "x".into(),
                    value: "42".into(),
                    type_name: Some("int".into()),
                    variables_reference: 0,
                    evaluate_name: None
                }]
            }
        );

        let body = json!({"result": "43", "type": "int", "variablesReference": 0});
        assert_eq!(
            response(&Pending::Evaluate, &body),
            Response::Evaluate {
                result: "43".into(),
                type_name: Some("int".into()),
                variables_reference: 0
            }
        );
        assert_eq!(response(&Pending::Ack, &Value::Null), Response::Ack);
    }

    #[test]
    fn events_map() {
        assert_eq!(
            event(
                "stopped",
                &json!({"reason": "breakpoint", "threadId": 5, "allThreadsStopped": true, "description": "breakpoint 1.1"})
            ),
            Some(Event::Stopped {
                reason: "breakpoint".into(),
                thread_id: Some(5),
                all_threads: true,
                description: Some("breakpoint 1.1".into())
            })
        );
        assert_eq!(
            event("continued", &json!({"threadId": 5})),
            Some(Event::Continued {
                thread_id: Some(5),
                all_threads: true
            })
        );
        assert_eq!(
            event("output", &json!({"category": "stdout", "output": "hi\r\n"})),
            Some(Event::Output {
                category: OutputCategory::Stdout,
                text: "hi\n".into()
            })
        );
        assert_eq!(
            event("output", &json!({"output": "note\n"})),
            Some(Event::Output {
                category: OutputCategory::Console,
                text: "note\n".into()
            })
        );
        assert_eq!(
            event("output", &json!({"category": "telemetry", "output": "x"})),
            None
        );
        assert_eq!(
            event(
                "breakpoint",
                &json!({"reason": "changed", "breakpoint": {"id": 1, "verified": true, "line": 3, "source": {"path": "/a.rs"}}})
            ),
            Some(Event::BreakpointChanged {
                path: Some("/a.rs".into()),
                breakpoint: Breakpoint {
                    id: Some(1),
                    verified: true,
                    line: Some(2),
                    message: None
                }
            })
        );
        assert_eq!(
            event("thread", &json!({"reason": "started", "threadId": 2})),
            Some(Event::ThreadStarted(2))
        );
        assert_eq!(
            event("thread", &json!({"reason": "exited", "threadId": 2})),
            Some(Event::ThreadExited(2))
        );
        assert_eq!(
            event("exited", &json!({"exitCode": 3})),
            Some(Event::Exited(3))
        );
        assert_eq!(event("terminated", &Value::Null), Some(Event::Terminated));
        assert_eq!(event("process", &json!({"name": "a"})), None);
    }

    #[test]
    fn launch_arguments_are_lldb_dap_s() {
        let launch = LaunchRequest {
            program: "/bin/prog".into(),
            args: vec!["-v".into()],
            cwd: "/work".into(),
            env: vec![("RUST_LOG".into(), "debug".into())],
            stop_on_entry: true,
        };
        let args = launch_arguments(&launch, &["command script import x.py".into()]);
        assert_eq!(
            args,
            json!({
                "program": "/bin/prog",
                "args": ["-v"],
                "cwd": "/work",
                "env": ["RUST_LOG=debug"],
                "stopOnEntry": true,
                "initCommands": ["command script import x.py"],
            })
        );
        let args = attach_arguments(42, &[]);
        assert_eq!(args, json!({"pid": 42}));
        let args = evaluate_arguments("x + 1", Some(7), EvalContext::Hover);
        assert_eq!(
            args,
            json!({"expression": "x + 1", "frameId": 7, "context": "hover"})
        );
    }

    #[test]
    fn errors_prefer_the_formatted_text() {
        assert_eq!(
            error_text(
                "evaluate",
                Some("error"),
                &json!({"error": {"id": 1, "format": "use of undeclared identifier 'y'"}})
            ),
            "use of undeclared identifier 'y'"
        );
        assert_eq!(
            error_text("launch", Some("no such file"), &Value::Null),
            "no such file"
        );
        assert_eq!(error_text("next", None, &Value::Null), "next failed");
    }
}
