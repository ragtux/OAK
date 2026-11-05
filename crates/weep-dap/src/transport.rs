//! The Debug Adapter Protocol's wire: JSON bodies over the adapter's stdin
//! and stdout, each framed with a `Content-Length` header (the same framing
//! as LSP, without JSON-RPC's envelope). A reader thread parses every
//! incoming message and sends it on a channel; a writer thread drains
//! outgoing frames, so the owner never blocks on a full pipe; a third
//! thread collects the adapter's stderr. `WEEP_DAP_TRACE` in the
//! environment prints every message both ways on stderr.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

/// One message from the adapter, told apart by its `type`.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// A reverse request (`runInTerminal`, `startDebugging`).
    Request {
        seq: i64,
        command: String,
        arguments: Value,
    },
    Event {
        event: String,
        body: Value,
    },
    Response {
        request_seq: i64,
        success: bool,
        command: String,
        /// The short error text (`message`), when `success` is false.
        message: Option<String>,
        body: Value,
    },
}

pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

/// A body with its header, ready for the pipe.
pub fn frame(body: &str) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Reads one framed message; `None` at the end of the stream.
pub fn read_message(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut length: Option<usize> = None;
    let mut saw_header = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if saw_header {
                break;
            }
            continue;
        }
        saw_header = true;
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse().ok();
        }
    }
    let Some(n) = length else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "a message without Content-Length",
        ));
    };
    let mut buf = vec![0; n];
    reader.read_exact(&mut buf)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// Sorts a message body into a request, an event or a response.
pub fn parse(body: &str) -> Option<Incoming> {
    let v: Value = serde_json::from_str(body).ok()?;
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let field = |key: &str| v.get(key).cloned().unwrap_or(Value::Null);
    match v.get("type").and_then(Value::as_str)? {
        "request" => Some(Incoming::Request {
            seq: v.get("seq").and_then(Value::as_i64)?,
            command: text("command")?,
            arguments: field("arguments"),
        }),
        "event" => Some(Incoming::Event {
            event: text("event")?,
            body: field("body"),
        }),
        "response" => Some(Incoming::Response {
            request_seq: v.get("request_seq").and_then(Value::as_i64)?,
            success: v.get("success").and_then(Value::as_bool).unwrap_or(false),
            command: text("command").unwrap_or_default(),
            message: text("message"),
            body: field("body"),
        }),
        _ => None,
    }
}

/// Parses the adapter's stdout until it closes; each message goes on `tx`
/// and `wake` runs after it (and once more when the pipe closes, so the
/// owner sees the end).
pub(crate) fn spawn_reader(
    source: Box<dyn Read + Send>,
    tx: Sender<Incoming>,
    wake: Wake,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let trace = std::env::var_os("WEEP_DAP_TRACE").is_some();
        let mut reader = BufReader::new(source);
        while let Ok(Some(body)) = read_message(&mut reader) {
            if trace {
                eprintln!("dap <- {body}");
            }
            if let Some(message) = parse(&body) {
                if tx.send(message).is_err() {
                    return;
                }
                wake();
            }
        }
        drop(tx);
        wake();
    })
}

/// Writes every frame it is handed to the adapter's stdin, in order.
pub(crate) fn spawn_writer(
    mut sink: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let trace = std::env::var_os("WEEP_DAP_TRACE").is_some();
        while let Ok(bytes) = rx.recv() {
            if trace {
                eprintln!("dap -> {}", String::from_utf8_lossy(&bytes));
            }
            if sink.write_all(&bytes).is_err() || sink.flush().is_err() {
                break;
            }
        }
    })
}

/// Reads the adapter's stderr a line at a time (its log).
pub(crate) fn spawn_stderr(
    source: Box<dyn Read + Send>,
    tx: Sender<String>,
    wake: Wake,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(source);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                    wake();
                }
                Err(_) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn frames_and_reads_back() {
        let one = frame(
            r#"{"seq":3,"type":"response","request_seq":1,"success":true,"command":"initialize","body":{"supportsConfigurationDoneRequest":true}}"#,
        );
        let two = frame(r#"{"seq":4,"type":"event","event":"initialized"}"#);
        let mut bytes = one;
        bytes.extend_from_slice(&two);
        let mut reader = Cursor::new(bytes);
        let first = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(
            parse(&first),
            Some(Incoming::Response {
                request_seq: 1,
                success: true,
                command: "initialize".into(),
                message: None,
                body: json!({"supportsConfigurationDoneRequest": true}),
            })
        );
        let second = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(
            parse(&second),
            Some(Incoming::Event {
                event: "initialized".into(),
                body: Value::Null,
            })
        );
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    #[test]
    fn a_reverse_request_has_a_command() {
        let body = r#"{"seq":9,"type":"request","command":"runInTerminal","arguments":{"kind":"integrated","args":["a"]}}"#;
        assert_eq!(
            parse(body),
            Some(Incoming::Request {
                seq: 9,
                command: "runInTerminal".into(),
                arguments: json!({"kind": "integrated", "args": ["a"]}),
            })
        );
    }

    #[test]
    fn a_failed_response_keeps_its_message() {
        let body = r#"{"seq":5,"type":"response","request_seq":2,"success":false,"command":"evaluate","message":"no such variable"}"#;
        match parse(body) {
            Some(Incoming::Response {
                success, message, ..
            }) => {
                assert!(!success);
                assert_eq!(message.as_deref(), Some("no such variable"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn other_headers_and_multibyte_bodies() {
        let body = r#"{"seq":1,"type":"event","event":"output","body":{"output":"héllo\n"}}"#;
        let text = format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let mut reader = Cursor::new(text.into_bytes());
        let got = read_message(&mut reader).unwrap().unwrap();
        match parse(&got) {
            Some(Incoming::Event { event, body }) => {
                assert_eq!(event, "output");
                assert_eq!(body["output"], json!("héllo\n"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn junk_is_not_a_message() {
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"seq":1,"type":"bogus"}"#), None);
        let mut reader = Cursor::new(b"X-Other: 1\r\n\r\n{}".to_vec());
        assert!(read_message(&mut reader).is_err());
    }
}
