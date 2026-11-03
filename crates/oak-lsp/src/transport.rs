//! JSON-RPC 2.0 over a server's stdin and stdout, each message framed with
//! a `Content-Length` header as the protocol says. A reader thread parses
//! every incoming message and sends it on a channel; a writer thread drains
//! outgoing frames, so the owner never blocks on a full pipe.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

/// One message from the server, told apart by its fields: a request has a
/// method and an id, a notification a method alone, a response an id alone.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: Value,
        result: Option<Value>,
        error: Option<Value>,
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

/// Sorts a message body into a request, a notification or a response.
pub fn parse(body: &str) -> Option<Incoming> {
    let v: Value = serde_json::from_str(body).ok()?;
    let method = v.get("method").and_then(Value::as_str).map(str::to_string);
    let id = v.get("id").cloned().filter(|id| !id.is_null());
    let params = || v.get("params").cloned().unwrap_or(Value::Null);
    match (method, id) {
        (Some(method), Some(id)) => Some(Incoming::Request {
            id,
            method,
            params: params(),
        }),
        (Some(method), None) => Some(Incoming::Notification {
            method,
            params: params(),
        }),
        (None, Some(id)) => Some(Incoming::Response {
            id,
            result: v.get("result").cloned(),
            error: v.get("error").cloned(),
        }),
        (None, None) => None,
    }
}

/// Parses the server's stdout until it closes; each message goes on `tx`
/// and `wake` runs after it.
pub(crate) fn spawn_reader(
    source: Box<dyn Read + Send>,
    tx: Sender<Incoming>,
    wake: Wake,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(source);
        while let Ok(Some(body)) = read_message(&mut reader) {
            if let Some(message) = parse(&body) {
                if tx.send(message).is_err() {
                    break;
                }
                wake();
            }
        }
    })
}

/// Writes every frame it is handed to the server's stdin, in order.
pub(crate) fn spawn_writer(
    mut sink: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(bytes) = rx.recv() {
            if sink.write_all(&bytes).is_err() || sink.flush().is_err() {
                break;
            }
        }
    })
}

/// Reads the server's stderr a line at a time (its log).
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
        let one = frame(r#"{"jsonrpc":"2.0","id":1,"result":null}"#);
        let two = frame(r#"{"jsonrpc":"2.0","method":"$/progress","params":{"token":"t"}}"#);
        let mut bytes = one.clone();
        bytes.extend_from_slice(&two);
        let mut reader = Cursor::new(bytes);
        let first = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(
            parse(&first),
            Some(Incoming::Response {
                id: json!(1),
                result: Some(Value::Null),
                error: None
            })
        );
        let second = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(
            parse(&second),
            Some(Incoming::Notification {
                method: "$/progress".into(),
                params: json!({"token": "t"})
            })
        );
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    #[test]
    fn a_request_has_an_id_and_a_method() {
        let body = r#"{"jsonrpc":"2.0","id":"abc","method":"workspace/configuration","params":{"items":[]}}"#;
        assert_eq!(
            parse(body),
            Some(Incoming::Request {
                id: json!("abc"),
                method: "workspace/configuration".into(),
                params: json!({"items": []})
            })
        );
    }

    #[test]
    fn other_headers_are_skipped() {
        let body = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"no"}}"#;
        let text = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let mut reader = Cursor::new(text.into_bytes());
        let got = read_message(&mut reader).unwrap().unwrap();
        match parse(&got) {
            Some(Incoming::Response { id, error, .. }) => {
                assert_eq!(id, json!(2));
                assert_eq!(error.unwrap()["code"], json!(-32601));
            }
            other => panic!("{other:?}"),
        }
    }
}
