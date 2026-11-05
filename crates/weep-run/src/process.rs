//! A running configuration: the child process, its output a line at a time
//! (stdout and stderr told apart: the run tool paints stderr red, batch 13),
//! and its exit. Two threads read stdout and stderr, a third waits for the
//! exit; each sends on a channel and wakes the owner.

use crate::config::Command;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// What the process sent: a line of stdout or of stderr (without its
/// newline), or its exit code (`None` when a signal ended it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Line(String),
    Stderr(String),
    Exited(Option<i32>),
}

type Wake = Arc<dyn Fn() + Send + Sync>;

/// A started process: poll it for its lines and its exit, kill it to stop.
pub struct Run {
    pid: u32,
    rx: Receiver<Event>,
    exited: bool,
}

impl Run {
    /// Starts `command`; `wake` runs on a worker thread whenever a line or
    /// the exit arrives, so the owner knows to [`Run::poll`].
    pub fn spawn(
        command: &Command,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Run, String> {
        let wake: Wake = Arc::new(wake);
        let mut cmd = std::process::Command::new(&command.program);
        cmd.args(&command.args)
            .current_dir(&command.cwd)
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let pid = child.id();
        let (tx, rx) = channel();
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            readers.push(reader(Box::new(out), false, tx.clone(), wake.clone()));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(reader(Box::new(err), true, tx.clone(), wake.clone()));
        }
        // The exit is reported after the readers drained the pipes, so no
        // line arrives after `Exited`.
        std::thread::spawn(move || {
            for r in readers {
                let _ = r.join();
            }
            let code = child.wait().ok().and_then(|s| s.code());
            let _ = tx.send(Event::Exited(code));
            wake();
        });
        Ok(Run {
            pid,
            rx,
            exited: false,
        })
    }

    /// Everything the process sent since the last poll.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            if matches!(event, Event::Exited(_)) {
                self.exited = true;
            }
            out.push(event);
        }
        out
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Stops the process and everything it started (`taskkill /T` takes
    /// the tree on Windows; `kill -TERM` elsewhere).
    pub fn kill(&self) {
        if self.exited {
            return;
        }
        let pid = self.pid.to_string();
        #[cfg(windows)]
        let mut cmd = {
            use std::os::windows::process::CommandExt;
            let mut c = std::process::Command::new("taskkill");
            c.args(["/PID", &pid, "/T", "/F"]);
            c.creation_flags(CREATE_NO_WINDOW);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = std::process::Command::new("kill");
            c.args(["-TERM", &pid]);
            c
        };
        let _ = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn reader(
    source: Box<dyn std::io::Read + Send>,
    stderr: bool,
    tx: Sender<Event>,
    wake: Wake,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(source);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    while buf.last().is_some_and(|c| *c == b'\n' || *c == b'\r') {
                        buf.pop();
                    }
                    let line = String::from_utf8_lossy(&buf).into_owned();
                    let event = if stderr {
                        Event::Stderr(line)
                    } else {
                        Event::Line(line)
                    };
                    if tx.send(event).is_err() {
                        break;
                    }
                    wake();
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn drain(run: &mut Run) -> Vec<Event> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut events = Vec::new();
        while Instant::now() < deadline {
            events.extend(run.poll());
            if run.exited() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        events
    }

    #[test]
    fn echo_then_exit_zero() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let command = Command::shell("echo hello from the fixture", Path::new("."));
        let mut run = Run::spawn(&command, move || {
            w.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        let events = drain(&mut run);
        assert_eq!(
            events,
            vec![
                Event::Line("hello from the fixture".into()),
                Event::Exited(Some(0))
            ]
        );
        assert!(wakes.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn exit_code_is_reported() {
        let command = Command::shell("exit 3", Path::new("."));
        let mut run = Run::spawn(&command, || {}).unwrap();
        let events = drain(&mut run);
        assert_eq!(events.last(), Some(&Event::Exited(Some(3))));
    }

    #[test]
    fn stderr_lines_are_told_apart() {
        let command = Command::shell("echo to-err 1>&2", Path::new("."));
        let mut run = Run::spawn(&command, || {}).unwrap();
        let events = drain(&mut run);
        assert_eq!(
            events,
            vec![Event::Stderr("to-err".into()), Event::Exited(Some(0))]
        );
    }

    #[test]
    fn a_missing_program_is_an_error() {
        let command = Command {
            program: "weep-no-such-program".into(),
            args: Vec::new(),
            cwd: Path::new(".").to_path_buf(),
            env: Vec::new(),
            display: String::new(),
        };
        assert!(Run::spawn(&command, || {}).is_err());
    }
}
