//! The client against the real lldb-dap, as weep-lsp's tests run against
//! the real rust-analyzer: tiny C and Rust programs built with debug info in
//! a temporary folder, then launched (or attached to) and driven through
//! breakpoints, frames, variables, steps, pause and exit. Each test skips
//! itself (saying why) when no adapter or compiler is installed, so CI
//! without LLDB passes.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};
use weep_dap::{
    AdapterCommand, Client, EvalContext, Event, FunctionBreakpoint, LaunchRequest, OutputCategory,
    RequestId, Response, SourceBreakpoint, Start,
};

const TIMEOUT: Duration = Duration::from_secs(60);

/// 0-based lines of `C_SOURCE` the tests stop at.
const C_CALL_LINE: usize = 10;
const C_PRINT_LINE: usize = 11;

const C_SOURCE: &str = r#"#include <stdio.h>
#include <unistd.h>

int add(int a, int b) {
    int sum = a + b;
    return sum;
}

int main(int argc, char **argv) {
    int x = 41;
    int y = add(x, 1);
    printf("result %d\n", y);
    fflush(stdout);
    if (argc > 1) {
        volatile int spins = 0;
        for (;;) {
            spins++;
            usleep(1000);
        }
    }
    return y - 40;
}
"#;

/// 0-based line of `RUST_SOURCE`'s `println!`.
const RUST_PRINT_LINE: usize = 4;

const RUST_SOURCE: &str = r#"fn main() {
    let answer: i32 = 42;
    let greeting = String::from("hello");
    let numbers = vec![1, 2, 3];
    println!("{greeting} {answer} {}", numbers.len());
}
"#;

/// 0-based line of `RUST_EVAL_SOURCE`'s `println!`.
const RUST_EVAL_LINE: usize = 8;

/// The fixture's `misc/debuggee.rs` (batch 13) with a tuple, a reference
/// and an `Option` beside its `points`, `label` and `total`.
const RUST_EVAL_SOURCE: &str = r#"fn main() {
    let points = vec![Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
    let label = String::from("points");
    let pair = (7, "seven");
    let first = &points[0];
    let maybe = Some(Point { x: 5, y: 6 });
    let total = sum(&points);
    let _ = (&pair, first, &maybe);
    println!("{label}: {total}");
}

#[derive(Debug)]
struct Point {
    x: i32,
    y: i32,
}

fn sum(points: &[Point]) -> i32 {
    points.iter().map(|p| p.x * p.y).sum()
}
"#;

/// 0-based lines of `RUST_PANIC_SOURCE`: `depth`'s `return`, its recursive
/// call, `main`'s call of `depth` and `boom`'s panic.
const PANIC_RETURN_LINE: usize = 12;
const PANIC_RECURSE_LINE: usize = 14;
const PANIC_MAIN_CALL_LINE: usize = 2;
const PANIC_LINE: usize = 18;

const RUST_PANIC_SOURCE: &str = r#"fn main() {
    let v = vec![1, 2, 3];
    let n = depth(3);
    println!("n = {}", n);
    if v.len() == 3 {
        boom(n);
    }
}

fn depth(k: i32) -> i32 {
    let here = k * 10;
    if k == 0 {
        return here;
    }
    depth(k - 1) + here
}

fn boom(n: i32) {
    panic!("boom {}", n);
}
"#;

fn adapter() -> Option<AdapterCommand> {
    let found = AdapterCommand::find_lldb();
    if found.is_none() {
        eprintln!("skipped: no lldb-dap (or lldb-vscode) on this machine");
    }
    found
}

/// Builds `C_SOURCE` with `cc -g -O0`; the folder and the source path,
/// both canonical (macOS's temp folder is behind a symlink).
fn build_c() -> Option<(tempfile::TempDir, PathBuf, PathBuf)> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("probe.c");
    std::fs::write(&source, C_SOURCE).unwrap();
    let program = root.join("probe");
    let status = Command::new("cc")
        .args(["-g", "-O0", "-o"])
        .arg(&program)
        .arg(&source)
        .status();
    if !status.is_ok_and(|s| s.success()) {
        eprintln!("skipped: cc could not build the C probe");
        return None;
    }
    Some((dir, source, program))
}

fn build_rust() -> Option<(tempfile::TempDir, PathBuf, PathBuf)> {
    build_rust_from(RUST_SOURCE)
}

fn build_rust_from(text: &str) -> Option<(tempfile::TempDir, PathBuf, PathBuf)> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("main.rs");
    std::fs::write(&source, text).unwrap();
    let program = root.join("main");
    let status = Command::new("rustc")
        .args(["-g", "-C", "opt-level=0", "-o"])
        .arg(&program)
        .arg(&source)
        .status();
    if !status.is_ok_and(|s| s.success()) {
        eprintln!("skipped: rustc could not build the Rust probe");
        return None;
    }
    Some((dir, source, program))
}

fn launch(program: &Path, args: &[&str]) -> Start {
    Start::Launch(LaunchRequest {
        program: program.to_path_buf(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: program.parent().unwrap().to_path_buf(),
        env: vec![("WEEP_DAP_TEST".into(), "1".into())],
        stop_on_entry: false,
        run_in_terminal: false,
    })
}

fn line(n: usize) -> SourceBreakpoint {
    SourceBreakpoint {
        line: n,
        ..Default::default()
    }
}

struct Session {
    client: Client,
    /// Arrived and not yet taken by a `wait`.
    events: Vec<Event>,
    /// Everything that arrived, for the failure messages.
    log: Vec<Event>,
    woken: Receiver<()>,
}

impl Session {
    fn start(
        adapter: &AdapterCommand,
        start: Start,
        breakpoints: Vec<(PathBuf, Vec<SourceBreakpoint>)>,
    ) -> Session {
        let (tx, woken) = channel();
        let client = Client::start(adapter, start, breakpoints, move || {
            let _ = tx.send(());
        })
        .expect("the adapter starts");
        Session {
            client,
            events: Vec::new(),
            log: Vec::new(),
            woken,
        }
    }

    /// Takes the first event `f` accepts (in any position among those not
    /// yet taken), waiting for more as they arrive.
    fn wait<T>(&mut self, what: &str, mut f: impl FnMut(&Event) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let fresh = self.client.poll();
            self.log.extend(fresh.iter().cloned());
            self.events.extend(fresh);
            if let Some((i, v)) = self
                .events
                .iter()
                .enumerate()
                .find_map(|(i, e)| f(e).map(|v| (i, v)))
            {
                self.events.remove(i);
                return v;
            }
            let now = Instant::now();
            if now >= deadline {
                panic!(
                    "timed out waiting for {what}; events so far: {:#?}",
                    self.log
                );
            }
            let _ = self
                .woken
                .recv_timeout((deadline - now).min(Duration::from_millis(200)));
        }
    }

    fn response(&mut self, id: RequestId) -> Result<Response, String> {
        self.wait(&format!("the response to {id:?}"), |e| match e {
            Event::Response { id: got, result } if *got == id => Some(result.clone()),
            _ => None,
        })
    }

    fn ok(&mut self, id: RequestId) -> Response {
        self.response(id)
            .unwrap_or_else(|e| panic!("request {id:?} failed: {e}; events: {:#?}", self.log))
    }

    fn frames(&mut self, thread: i64) -> Vec<weep_dap::StackFrame> {
        let id = self.client.stack_trace(thread);
        match self.ok(id) {
            Response::StackTrace { frames, .. } => frames,
            other => panic!("{other:?}"),
        }
    }

    fn value(&mut self, expression: &str, frame: i64) -> String {
        let id = self
            .client
            .evaluate(expression, Some(frame), EvalContext::Watch);
        match self.ok(id) {
            Response::Evaluate { result, .. } => result,
            other => panic!("{other:?}"),
        }
    }

    fn stopped(&mut self) -> (String, i64) {
        self.wait("a stopped event", |e| match e {
            Event::Stopped {
                reason,
                thread_id: Some(t),
                ..
            } => Some((reason.clone(), *t)),
            _ => None,
        })
    }

    fn top_frame(&mut self, thread: i64) -> weep_dap::StackFrame {
        let id = self.client.stack_trace(thread);
        match self.ok(id) {
            Response::StackTrace { thread_id, frames } => {
                assert_eq!(thread_id, thread);
                frames.into_iter().next().expect("a frame")
            }
            other => panic!("{other:?}"),
        }
    }

    fn locals(&mut self, frame: i64) -> Vec<weep_dap::Variable> {
        let id = self.client.scopes(frame);
        let scopes = match self.ok(id) {
            Response::Scopes { frame_id, scopes } => {
                assert_eq!(frame_id, frame);
                scopes
            }
            other => panic!("{other:?}"),
        };
        let locals = scopes
            .iter()
            .find(|s| s.name.to_lowercase().contains("local"))
            .unwrap_or_else(|| panic!("a Locals scope in {scopes:?}"));
        let reference = locals.variables_reference;
        let id = self.client.variables(reference);
        match self.ok(id) {
            Response::Variables {
                reference: r,
                variables,
            } => {
                assert_eq!(r, reference);
                variables
            }
            other => panic!("{other:?}"),
        }
    }

    fn first_thread(&mut self) -> i64 {
        let id = self.client.threads();
        match self.ok(id) {
            Response::Threads(threads) => threads.first().expect("a thread").id,
            other => panic!("{other:?}"),
        }
    }

    fn terminated(&mut self) {
        self.wait("terminated", |e| {
            matches!(e, Event::Terminated).then_some(())
        });
    }

    fn print_log(&self, name: &str) {
        eprintln!("{name}: every event: {:#?}", self.log);
    }
}

#[test]
fn c_breakpoint_frames_variables_step_and_exit() {
    let Some(adapter) = adapter() else { return };
    let Some((_dir, source, program)) = build_c() else {
        return;
    };
    let mut s = Session::start(
        &adapter,
        launch(&program, &[]),
        vec![(source.clone(), vec![line(C_CALL_LINE)])],
    );

    // The breakpoint is verified at the line it was set on.
    let (path, bps) = s.wait("the first setBreakpoints", |e| match e {
        Event::Response {
            result: Ok(Response::SetBreakpoints { path, breakpoints }),
            ..
        } => Some((path.clone(), breakpoints.clone())),
        _ => None,
    });
    assert_eq!(path, source);
    assert_eq!(bps.len(), 1);
    s.wait("running", |e| matches!(e, Event::Running).then_some(()));

    // ... and hit.
    let (reason, thread) = s.stopped();
    assert_eq!(reason, "breakpoint");
    let bps_now = s.log.iter().rev().find_map(|e| match e {
        Event::BreakpointChanged { path, breakpoint } => {
            // lldb-dap leaves the source out; the client fills it in.
            assert_eq!(path.as_deref(), Some(source.as_path()));
            Some(breakpoint.clone())
        }
        _ => None,
    });
    assert!(
        bps[0].verified || bps_now.is_some_and(|b| b.verified),
        "the breakpoint was never verified: {bps:?}"
    );

    let frame = s.top_frame(thread);
    assert!(frame.name.contains("main"), "{frame:?}");
    assert_eq!(frame.path.as_deref(), Some(source.as_path()));
    assert_eq!(frame.line, C_CALL_LINE);
    let id = s.client.stack_trace(thread);
    if let Response::StackTrace { frames, .. } = s.ok(id) {
        // dyld's `start` has no file (lldb-dap serves its disassembly).
        assert!(frames[1..].iter().all(|f| f.path.is_none()), "{frames:?}");
    }

    let locals = s.locals(frame.id);
    let x = locals
        .iter()
        .find(|v| v.name == "x")
        .unwrap_or_else(|| panic!("x in {locals:?}"));
    assert_eq!(x.value, "41");
    assert_eq!(x.type_name.as_deref(), Some("int"));

    let id = s
        .client
        .evaluate("x + 1", Some(frame.id), EvalContext::Watch);
    match s.ok(id) {
        Response::Evaluate { result, .. } => assert_eq!(result, "42"),
        other => panic!("{other:?}"),
    }
    let id = s
        .client
        .evaluate("no_such_name", Some(frame.id), EvalContext::Hover);
    assert!(s.response(id).is_err());

    // Step over moves one line.
    let id = s.client.step_over(thread);
    assert_eq!(s.ok(id), Response::Ack);
    let (reason, thread) = s.stopped();
    assert_eq!(reason, "step");
    let frame = s.top_frame(thread);
    assert_eq!(frame.line, C_PRINT_LINE);
    let locals = s.locals(frame.id);
    assert_eq!(
        locals
            .iter()
            .find(|v| v.name == "y")
            .map(|v| v.value.as_str()),
        Some("42")
    );

    // Continue to the end: the output, the exit code, the end.
    let id = s.client.resume(thread);
    assert_eq!(s.ok(id), Response::Ack);
    let out = s.wait("the program's output", |e| match e {
        Event::Output { category, text } if text.contains("result 42") => {
            assert_eq!(text, "result 42\n");
            Some(*category)
        }
        _ => None,
    });
    assert_eq!(out, OutputCategory::Stdout);
    let code = s.wait("exited", |e| match e {
        Event::Exited(code) => Some(*code),
        _ => None,
    });
    assert_eq!(code, 2);
    s.terminated();
    s.print_log("c");
    // After the end requests are answered at once, with an error.
    let id = s.client.threads();
    assert!(s.response(id).is_err());
}

#[test]
fn pause_a_looping_program_then_stop() {
    let Some(adapter) = adapter() else { return };
    let Some((_dir, _source, program)) = build_c() else {
        return;
    };
    let mut s = Session::start(&adapter, launch(&program, &["loop"]), Vec::new());
    // Asked before the handshake finished: queued, then answered.
    let early = s.client.threads();
    s.wait("running", |e| matches!(e, Event::Running).then_some(()));
    let thread = match s.ok(early) {
        Response::Threads(threads) => threads.first().expect("a thread").id,
        other => panic!("{other:?}"),
    };
    s.wait("the program's output", |e| match e {
        Event::Output { text, .. } if text.contains("result 42") => Some(()),
        _ => None,
    });
    // lldb-dap answers `threads` only while the process is stopped.
    let id = s.client.threads();
    assert!(s.response(id).is_err());
    let id = s.client.pause(thread);
    assert_eq!(s.ok(id), Response::Ack);
    let (reason, _) = s.stopped();
    assert_eq!(reason, "pause");
    s.client.stop();
    s.terminated();
    s.print_log("pause");
}

#[test]
fn stop_on_entry() {
    let Some(adapter) = adapter() else { return };
    let Some((_dir, _source, program)) = build_c() else {
        return;
    };
    let mut start = launch(&program, &[]);
    if let Start::Launch(l) = &mut start {
        l.stop_on_entry = true;
    }
    let mut s = Session::start(&adapter, start, Vec::new());
    let (reason, _) = s.stopped();
    assert_eq!(reason, "entry");
    // The session ran before it stopped, whatever order lldb-dap answers in.
    let running = s.log.iter().position(|e| matches!(e, Event::Running));
    let stopped = s
        .log
        .iter()
        .position(|e| matches!(e, Event::Stopped { .. }));
    assert!(running.is_some() && running < stopped, "{:#?}", s.log);
    s.client.stop();
    s.terminated();
    s.print_log("entry");
}

#[test]
fn attach_to_a_running_process() {
    let Some(adapter) = adapter() else { return };
    let Some((_dir, _source, program)) = build_c() else {
        return;
    };
    let mut child = Command::new(&program)
        .arg("loop")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let mut s = Session::start(&adapter, Start::Attach { pid: child.id() }, Vec::new());
    let outcome = s.wait("running or an error", |e| match e {
        Event::Running => Some(Ok(())),
        Event::Error(m) if !m.starts_with("lldb-dap:") => Some(Err(m.clone())),
        _ => None,
    });
    if let Err(e) = outcome {
        eprintln!("skipped: the OS refused the attach: {e}");
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    // lldb-dap stops the process it attached to; it may already say so.
    let thread = s.first_thread();
    let already = s.log.iter().any(|e| matches!(e, Event::Stopped { .. }));
    if !already {
        let id = s.client.pause(thread);
        let _ = s.response(id);
    }
    let (reason, _) = s.stopped();
    assert_eq!(reason, "pause");
    s.client.stop();
    s.terminated();
    s.print_log("attach");
    // A detach leaves the process running.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the attached process died"
    );
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn rust_breakpoint_and_formatted_values() {
    let Some(adapter) = adapter() else { return };
    let Some((_dir, source, program)) = build_rust() else {
        return;
    };
    let mut s = Session::start(&adapter, launch(&program, &[]), Vec::new());
    // Set after the start, before `initialized`: queued into the configuration.
    let id = s.client.set_breakpoints(&source, &[line(RUST_PRINT_LINE)]);
    match s.ok(id) {
        Response::SetBreakpoints { path, breakpoints } => {
            assert_eq!(path, source);
            assert_eq!(breakpoints.len(), 1);
        }
        other => panic!("{other:?}"),
    }
    let (reason, thread) = s.stopped();
    assert_eq!(reason, "breakpoint");
    let frame = s.top_frame(thread);
    eprintln!("rust frame: {frame:?}");
    assert!(frame.name.contains("main"), "{frame:?}");
    assert_eq!(frame.path.as_deref(), Some(source.as_path()));
    assert_eq!(frame.line, RUST_PRINT_LINE);
    let locals = s.locals(frame.id);
    eprintln!("rust locals: {locals:#?}");
    let answer = locals.iter().find(|v| v.name == "answer").expect("answer");
    assert_eq!(answer.value, "42");
    let greeting = locals
        .iter()
        .find(|v| v.name == "greeting")
        .expect("greeting");
    if weep_dap::rust_init_commands().is_empty() {
        eprintln!(
            "no Rust formatters in this toolchain: greeting = {}",
            greeting.value
        );
    } else {
        assert!(greeting.value.contains("hello"), "{greeting:?}");
    }
    let id = s
        .client
        .evaluate("answer", Some(frame.id), EvalContext::Watch);
    match s.ok(id) {
        Response::Evaluate { result, .. } => assert_eq!(result, "42"),
        other => panic!("{other:?}"),
    }
    // The console's context answers as LLDB's `expression` prints.
    let id = s
        .client
        .evaluate("answer", Some(frame.id), EvalContext::Repl);
    match s.ok(id) {
        Response::Evaluate { result, .. } => assert!(result.contains("= 42"), "{result}"),
        other => panic!("{other:?}"),
    }
    // `println!` puts code at two places on its line (breakpoint 1.1 and
    // 1.2): cleared, so the program runs to its end.
    let id = s.client.set_breakpoints(&source, &[]);
    match s.ok(id) {
        Response::SetBreakpoints { breakpoints, .. } => assert!(breakpoints.is_empty()),
        other => panic!("{other:?}"),
    }
    let id = s.client.resume(thread);
    s.ok(id);
    s.wait("the program's output", |e| match e {
        Event::Output {
            category: OutputCategory::Stdout,
            text,
        } if text.contains("hello 42 3") => Some(()),
        _ => None,
    });
    let code = s.wait("exited", |e| match e {
        Event::Exited(code) => Some(*code),
        _ => None,
    });
    assert_eq!(code, 0);
    s.terminated();
    s.print_log("rust");
}

#[test]
fn rust_expressions_through_lldb() {
    let Some(adapter) = adapter() else { return };
    let Some((_dir, source, program)) = build_rust_from(RUST_EVAL_SOURCE) else {
        return;
    };
    let formatters = !weep_dap::rust_init_commands().is_empty();
    let mut s = Session::start(
        &adapter,
        launch(&program, &[]),
        vec![(source.clone(), vec![line(RUST_EVAL_LINE)])],
    );
    let (reason, thread) = s.stopped();
    assert_eq!(reason, "breakpoint");
    let frame = s.top_frame(thread);
    assert_eq!(frame.line, RUST_EVAL_LINE);
    let mut eval = |expression: &str, context: EvalContext| {
        let id = s.client.evaluate(expression, Some(frame.id), context);
        let result = s.response(id).map(|r| match r {
            Response::Evaluate {
                result,
                type_name,
                variables_reference,
            } => (result, type_name, variables_reference),
            other => panic!("{other:?}"),
        });
        eprintln!("{expression} ({context:?}): {result:?}");
        result
    };
    use EvalContext::{Hover, Repl, Watch};

    // LLDB's evaluator (Objective C++) does arithmetic; hover, a variable
    // path only, is retried as a watch.
    assert_eq!(eval("total * 2", Watch).unwrap().0, "28");
    assert_eq!(eval("total * 2", Hover).unwrap().0, "28");
    assert!(eval("total * 2", Repl).unwrap().0.ends_with("= 28"));
    for context in [Watch, Hover] {
        let (value, type_name, _) = eval("total", context).unwrap();
        assert_eq!((value.as_str(), type_name.as_deref()), ("14", Some("int")));
        assert_eq!(eval("points[0].x", context).unwrap().0, "1");
        // `.` through a reference: hover's path refuses it, the tree has it.
        assert_eq!(eval("first.x", context).unwrap().0, "1");
        // A tuple field: the evaluator would answer 0 (a float `.0`).
        let (value, type_name, _) = eval("pair.0", context).unwrap();
        assert_eq!((value.as_str(), type_name.as_deref()), ("7", Some("int")));
        let (value, _, children) = eval("maybe.0", context).unwrap();
        assert!(children > 0, "maybe.0 = {value}");
        let (value, _, children) = eval("&points[0]", context).unwrap();
        assert!(value.starts_with("0x"), "{value}");
        assert!(children > 0);
        if formatters {
            assert_eq!(eval("label", context).unwrap().0, "\"points\"");
            assert!(value.ends_with("{x:1, y:2}"), "{value}");
        }
    }
    assert_eq!(eval("pair.0", Repl).unwrap().0, "(int) pair.0 = 7");

    // Failures are LLDB's one error line, the fallback warning dropped.
    let one_line = |e: &str| !e.contains('\n') && !e.contains("Falling back") && !e.is_empty();
    for context in [Watch, Hover, Repl] {
        let e = eval("points.len()", context).unwrap_err();
        assert!(one_line(&e), "{e}");
        let e = eval("nope * 2", context).unwrap_err();
        assert_eq!(e, "use of undeclared identifier 'nope'");
        let e = eval("maybe.x", context).unwrap_err();
        assert!(one_line(&e) && e.contains('x'), "{e}");
        let e = eval("pair.5", context).unwrap_err();
        assert!(one_line(&e), "{e}");
    }

    // lldb-dap's evaluateName of a Vec's element ("[0]->[0]") is dropped.
    let locals = s.locals(frame.id);
    let points = locals.iter().find(|v| v.name == "points").expect("points");
    assert_eq!(points.evaluate_name.as_deref(), Some("points"));
    let id = s.client.variables(points.variables_reference);
    if let Response::Variables { variables, .. } = s.ok(id) {
        assert!(
            variables.iter().all(|v| v.evaluate_name.is_none()),
            "{variables:?}"
        );
    }
    s.client.stop();
    s.terminated();
}

/// The Debugger Console's "commands" mode: a line is an LLDB command (even
/// one lldb-dap's `auto` mode would evaluate as an expression), its output
/// the answer; a mistyped command answers LLDB's error as output.
#[test]
fn console_commands_through_lldb() {
    let Some(adapter) = adapter() else { return };
    let Some((_dir, source, program)) = build_rust_from(RUST_EVAL_SOURCE) else {
        return;
    };
    let mut s = Session::start(
        &adapter,
        launch(&program, &[]),
        vec![(source.clone(), vec![line(RUST_EVAL_LINE)])],
    );
    let (reason, thread) = s.stopped();
    assert_eq!(reason, "breakpoint");
    let frame = s.top_frame(thread);
    let mut command = |line: &str| {
        let id = s.client.command(line, Some(frame.id));
        let answer = s.response(id).map(|r| match r {
            Response::Evaluate {
                result, type_name, ..
            } => (result, type_name),
            other => panic!("{other:?}"),
        });
        eprintln!("{line}: {answer:?}");
        answer
    };
    let (out, type_name) = command("frame variable total").unwrap();
    assert_eq!(out.trim_end(), "(int) total = 14");
    assert_eq!(type_name, None);
    let (out, _) = command("p total * 2").unwrap();
    assert!(out.trim_end().ends_with("28"), "{out}");
    // In `auto` mode this would be the expression `total`.
    let (out, _) = command("total").unwrap();
    assert!(out.contains("not a valid command"), "{out}");
    let (out, _) = command("bt 1").unwrap();
    assert!(out.contains("frame #0"), "{out}");
    s.client.stop();
    s.terminated();
}

#[test]
fn a_launch_that_fails_says_why_and_ends() {
    let Some(adapter) = adapter() else { return };
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-program");
    let mut s = Session::start(&adapter, launch(&missing, &[]), Vec::new());
    let early = s.client.threads();
    let message = s.wait("an error", |e| match e {
        Event::Error(m) if m.starts_with("launch:") => Some(m.clone()),
        _ => None,
    });
    eprintln!("failed launch: {message}");
    s.terminated();
    assert!(s.response(early).is_err());
}

#[cfg(unix)]
#[test]
fn kill_ends_the_session_at_once() {
    let Some(adapter) = adapter() else { return };
    let Some((_dir, _source, program)) = build_c() else {
        return;
    };
    let mut s = Session::start(&adapter, launch(&program, &["loop"]), Vec::new());
    s.wait("running", |e| matches!(e, Event::Running).then_some(()));
    let pid = s.client.adapter_pid();
    s.client.kill();
    s.terminated();
    assert!(s.client.is_over());
    // The adapter is gone (`kill -0` fails on a reaped pid).
    let alive = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|st| st.success());
    assert!(!alive, "the adapter {pid} still runs");
}

/// WEEP's Rust panic breakpoint: a function breakpoint on `rust_panic`
/// (lldb-dap binds `__rustc::rust_panic` by its base name) set before the
/// configuration is done, with the adapter's exception filters (an unknown
/// one left out); the stop names the breakpoint it hit, and the frames
/// below it reach the panicking function.
#[test]
fn a_panic_stops_on_the_rust_panic_breakpoint() {
    let Some(adapter) = adapter() else { return };
    let Some((dir, source, program)) = build_rust_from(RUST_PANIC_SOURCE) else {
        return;
    };
    let mut s = Session::start(&adapter, launch(&program, &[]), Vec::new());
    let function = s.client.set_function_breakpoints(&[FunctionBreakpoint {
        name: "rust_panic".into(),
        condition: None,
    }]);
    let exceptions = s
        .client
        .set_exception_breakpoints(&["cpp_throw".into(), "no_such_filter".into()]);
    let filters = s.wait("the exception filters", |e| match e {
        Event::ExceptionFilters(f) => Some(f.clone()),
        _ => None,
    });
    eprintln!("exception filters: {filters:?}");
    assert!(filters
        .iter()
        .any(|f| f.filter == "cpp_throw" && !f.label.is_empty()));
    let ids = match s.ok(function) {
        Response::SetFunctionBreakpoints(bps) => {
            assert_eq!(bps.len(), 1);
            assert!(bps[0].verified, "{bps:?}");
            bps.iter().filter_map(|b| b.id).collect::<Vec<_>>()
        }
        other => panic!("{other:?}"),
    };
    assert!(matches!(
        s.ok(exceptions),
        Response::SetExceptionBreakpoints(_)
    ));
    let (reason, hit, thread) = s.wait("the panic's stop", |e| match e {
        Event::Stopped {
            reason,
            hit_breakpoint_ids,
            thread_id: Some(t),
            ..
        } => Some((reason.clone(), hit_breakpoint_ids.clone(), *t)),
        _ => None,
    });
    assert_eq!(reason, "breakpoint");
    assert!(hit.iter().any(|h| ids.contains(h)), "{hit:?} of {ids:?}");
    let frames = s.frames(thread);
    assert!(frames[0].name.contains("rust_panic"), "{:?}", frames[0]);
    let boom = frames
        .iter()
        .find(|f| f.path.as_deref() == Some(source.as_path()))
        .expect("the panicking function's frame");
    assert!(boom.name.contains("boom"), "{boom:?}");
    assert_eq!(boom.line, PANIC_LINE);
    // The library's frames: none under the probe's folder, some dimmed by
    // the adapter (the frames it has only disassembly of).
    let root = dir.path().canonicalize().unwrap();
    let library = frames
        .iter()
        .filter(|f| !f.path.as_ref().is_some_and(|p| p.starts_with(&root)))
        .count();
    assert!(library > frames.len() / 2, "{frames:#?}");
    assert!(frames.iter().any(|f| f.is_subtle()), "{frames:#?}");
    // lldb-dap answers exceptionInfo for any stop.
    let id = s.client.exception_info(thread);
    match s.response(id) {
        Ok(Response::ExceptionInfo(info)) => eprintln!("exception info: {info:?}"),
        other => panic!("{other:?}"),
    }
    // The panic's message reached stderr before the stop's report.
    let text: String = s
        .log
        .iter()
        .filter_map(|e| match e {
            Event::Output { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    eprintln!("panic output: {text:?}");
    s.client.stop();
    s.terminated();
}

/// `debug/drop-frame` through LLDB: `frame select` and `thread return` pop
/// the selected frame and those above it; the caller is then the top frame,
/// stopped at its call. lldb-dap has no `restartFrame`.
#[test]
fn drop_frame_returns_to_the_caller() {
    let Some(adapter) = adapter() else { return };
    let Some((_dir, source, program)) = build_rust_from(RUST_PANIC_SOURCE) else {
        return;
    };
    let mut s = Session::start(
        &adapter,
        launch(&program, &[]),
        vec![(source.clone(), vec![line(PANIC_RETURN_LINE)])],
    );
    let (reason, thread) = s.stopped();
    assert_eq!(reason, "breakpoint");
    assert!(!s.client.supports("supportsRestartFrame"));
    assert!(s.client.can_drop_frame());
    // depth(0) .. depth(3), main.
    let before = s.frames(thread);
    assert_eq!(s.value("k", before[0].id), "0");
    assert_eq!(before[1].line, PANIC_RECURSE_LINE);
    // Drop depth(0), depth(1) and depth(2): depth(3) is on top.
    let id = s.client.drop_frame(2, before[2].id);
    assert_eq!(s.ok(id), Response::Ack);
    let after = s.frames(thread);
    assert_eq!(after.len(), before.len() - 3, "{after:#?}");
    assert_eq!(after[0].line, PANIC_RECURSE_LINE);
    assert_eq!(s.value("k", after[0].id), "3");
    // The top frame alone: main, at its call of depth.
    let id = s.client.drop_frame(0, after[0].id);
    assert_eq!(s.ok(id), Response::Ack);
    let top = s.top_frame(thread);
    assert!(top.name.contains("main"), "{top:?}");
    assert_eq!(top.line, PANIC_MAIN_CALL_LINE);
    // The debuggee goes on from there (the breakpoint cleared).
    let id = s.client.set_breakpoints(&source, &[]);
    s.ok(id);
    let id = s.client.resume(thread);
    s.ok(id);
    s.wait("exited", |e| matches!(e, Event::Exited(_)).then_some(()));
    s.terminated();
}

/// An ad-hoc signed copy of an Apple-signed lldb-dap (macOS): the
/// `runInTerminal` launcher is lldb-dap itself, waiting for the adapter to
/// attach before it starts the debuggee, and macOS refuses an attach to
/// Xcode's binary ("Not allowed to attach to process"). The copy finds
/// LLDB.framework through `DYLD_FRAMEWORK_PATH`.
#[cfg(target_os = "macos")]
fn attachable_launcher(program: &str, dir: &Path) -> Option<(PathBuf, Vec<(String, String)>)> {
    let program = Path::new(program).canonicalize().ok()?;
    let developer = program.ancestors().find(|p| p.ends_with("Developer"))?;
    let frameworks = developer.parent()?.join("SharedFrameworks");
    let copy = dir.join("lldb-dap-launcher");
    std::fs::copy(&program, &copy).ok()?;
    let signed = Command::new("codesign")
        .args(["-s", "-", "-f"])
        .arg(&copy)
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    signed.then(|| {
        (
            copy,
            vec![(
                "DYLD_FRAMEWORK_PATH".into(),
                frameworks.display().to_string(),
            )],
        )
    })
}

/// `runInTerminal`: with `run_in_terminal` set lldb-dap asks the client to
/// start its launcher (`lldb-dap --comm-file … --launch-target <program>
/// <args>`) in a terminal; the test starts it itself (no terminal) and
/// answers with its pid; the adapter attaches to it and the breakpoint is
/// hit as with a plain launch.
#[test]
fn run_in_terminal_is_asked_of_the_client() {
    let Some(adapter) = adapter() else { return };
    let Some((dir, source, program)) = build_rust_from(RUST_PANIC_SOURCE) else {
        return;
    };
    let mut start = launch(&program, &["one two"]);
    if let Start::Launch(l) = &mut start {
        l.run_in_terminal = true;
    }
    let mut s = Session::start(
        &adapter,
        start,
        vec![(source.clone(), vec![line(PANIC_RETURN_LINE)])],
    );
    let request = s.wait("the runInTerminal request", |e| match e {
        Event::RunInTerminal(r) => Some(r.clone()),
        _ => None,
    });
    eprintln!("runInTerminal: {request:#?}");
    assert_eq!(request.kind, "integrated");
    assert_eq!(request.cwd, program.parent().unwrap());
    let target = request
        .args
        .iter()
        .position(|a| a == "--launch-target")
        .expect("the launcher's --launch-target");
    assert_eq!(Path::new(&request.args[target + 1]), program.as_path());
    assert_eq!(request.args[target + 2], "one two");
    assert!(request
        .env
        .iter()
        .any(|(k, v)| k == "WEEP_DAP_TEST" && v.as_deref() == Some("1")));

    #[cfg(target_os = "macos")]
    let (launcher, extra_env) = attachable_launcher(&request.args[0], dir.path())
        .unwrap_or_else(|| (PathBuf::from(&request.args[0]), Vec::new()));
    #[cfg(not(target_os = "macos"))]
    let (launcher, extra_env): (PathBuf, Vec<(String, String)>) =
        (PathBuf::from(&request.args[0]), Vec::new());
    let mut cmd = Command::new(&launcher);
    cmd.args(&request.args[1..])
        .current_dir(&request.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null());
    for (k, v) in &request.env {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    cmd.envs(extra_env);
    let mut child = cmd.spawn().expect("the launcher starts");
    s.client.answer_run_in_terminal(request.id, Ok(child.id()));

    let outcome = s.wait("a stop or a failed launch", |e| match e {
        Event::Stopped {
            thread_id: Some(t), ..
        } => Some(Ok(*t)),
        Event::Error(m) if m.starts_with("launch:") => Some(Err(m.clone())),
        _ => None,
    });
    match outcome {
        Ok(thread) => {
            let frame = s.top_frame(thread);
            assert_eq!(frame.line, PANIC_RETURN_LINE);
            assert_eq!(frame.path.as_deref(), Some(source.as_path()));
            s.client.stop();
            s.terminated();
        }
        Err(e) => {
            // The adapter took the answer and tried the attach.
            assert!(e.contains("attach"), "{e}");
            eprintln!("skipped the debuggee: {e}");
            s.terminated();
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}
