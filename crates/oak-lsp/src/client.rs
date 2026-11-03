//! One language server: spawned on its pipes, taken through `initialize`
//! and `initialized`, asked typed requests whose answers come back through
//! [`Client::poll`] with the id the request was given, and told the
//! documents' changes. The server's own requests are answered here when
//! the answer is Oak's business (progress tokens, capability registration,
//! `workspace/configuration` from the settings) and handed up otherwise
//! (`workspace/applyEdit`).
//!
//! Nothing blocks: a request is written to the pipe by the writer thread
//! and its response arrives on the next poll after the reader thread saw
//! it; `wake` tells the owner to poll.

use crate::position::Encoding;
use crate::transport::{self, Incoming, Wake};
use lsp_types::notification::Notification;
use lsp_types::request::Request;
use lsp_types::{
    ApplyWorkspaceEditParams, ClientCapabilities, ClientInfo, CodeActionCapabilityResolveSupport,
    CodeActionClientCapabilities, CodeActionKindLiteralSupport, CodeActionLiteralSupport,
    CompletionClientCapabilities, CompletionItemCapability, CompletionItemCapabilityResolveSupport,
    ConfigurationParams, GeneralClientCapabilities, HoverClientCapabilities, InitializeParams,
    InitializeResult, InitializedParams, LogMessageParams, MarkupKind, MessageType,
    ParameterInformationSettings, ProgressParams, ProgressParamsValue,
    PublishDiagnosticsClientCapabilities, PublishDiagnosticsParams, RenameClientCapabilities,
    SemanticTokenModifier, SemanticTokenType, SemanticTokensClientCapabilities,
    SemanticTokensClientCapabilitiesRequests, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensServerCapabilities, SemanticTokensWorkspaceClientCapabilities,
    ServerCapabilities, ShowMessageParams, SignatureHelpClientCapabilities,
    SignatureInformationSettings, TextDocumentClientCapabilities,
    TextDocumentSyncClientCapabilities, TokenFormat, Url, WindowClientCapabilities,
    WorkDoneProgress, WorkDoneProgressParams, WorkspaceClientCapabilities, WorkspaceEdit,
    WorkspaceEditClientCapabilities, WorkspaceFolder,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// The id of a request Oak sent; its response carries it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub i64);

/// A JSON-RPC error answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

impl ResponseError {
    pub fn new(code: i64, message: impl Into<String>) -> ResponseError {
        ResponseError {
            code,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageLevel {
    Error,
    Warning,
    Info,
    Log,
}

impl MessageLevel {
    fn from(typ: MessageType) -> MessageLevel {
        match typ {
            MessageType::ERROR => MessageLevel::Error,
            MessageType::WARNING => MessageLevel::Warning,
            MessageType::INFO => MessageLevel::Info,
            _ => MessageLevel::Log,
        }
    }
}

/// What the server sent, in order.
#[derive(Clone, Debug)]
pub enum Event {
    /// `initialize` answered and `initialized` sent: requests go through.
    Ready,
    /// `initialize` failed; the server is of no use.
    Failed(String),
    Response {
        id: RequestId,
        result: Result<Value, ResponseError>,
    },
    Diagnostics(PublishDiagnosticsParams),
    /// `$/progress`: the title arrives with the first report of a token,
    /// `done` with the last.
    Progress {
        token: String,
        title: String,
        message: Option<String>,
        percentage: Option<u32>,
        done: bool,
    },
    /// `window/showMessage`.
    Message {
        level: MessageLevel,
        text: String,
    },
    /// `window/logMessage` and the server's stderr.
    Log(String),
    /// `workspace/applyEdit`: the owner applies it and answers with
    /// [`Client::respond_applied`].
    ApplyEdit {
        id: Value,
        label: Option<String>,
        edit: WorkspaceEdit,
    },
    /// `workspace/semanticTokens/refresh`: every open document asks again.
    RefreshSemanticTokens,
    /// `workspace/codeLens/refresh`.
    RefreshCodeLens,
    /// The server's stdout closed; its exit code when it had exited.
    Exited(Option<i32>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Starting,
    Ready,
    Failed,
    Exited,
}

pub struct Client {
    name: String,
    child: Child,
    out: Sender<Vec<u8>>,
    incoming: Receiver<Incoming>,
    stderr: Receiver<String>,
    next_id: i64,
    init_id: RequestId,
    shutdown_id: Option<RequestId>,
    state: State,
    /// Frames to send once `initialized` went out.
    queued: Vec<Vec<u8>>,
    capabilities: Option<ServerCapabilities>,
    encoding: Encoding,
    /// The settings subtree the server's `workspace/configuration` reads
    /// (Fleet's `rust-analyzer.*` keys, nested).
    config: Value,
    /// Progress titles by token, for the reports that carry none.
    progress: HashMap<String, String>,
    exit_reported: bool,
}

impl Client {
    /// Starts `command` for the workspace at `root` and sends `initialize`;
    /// `config` is the server's settings subtree (also its
    /// `initializationOptions`). `wake` runs on a worker thread whenever
    /// something arrived, so the owner knows to [`Client::poll`].
    pub fn spawn(
        command: &crate::ServerCommand,
        root: &Path,
        config: Value,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Client, String> {
        let wake: Wake = Arc::new(wake);
        let mut cmd = std::process::Command::new(&command.program);
        cmd.args(&command.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(crate::servers::CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", command.program.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let (out_tx, out_rx) = channel::<Vec<u8>>();
        let (in_tx, in_rx) = channel::<Incoming>();
        let (err_tx, err_rx) = channel::<String>();
        transport::spawn_writer(Box::new(stdin), out_rx);
        transport::spawn_reader(Box::new(stdout), in_tx, wake.clone());
        transport::spawn_stderr(Box::new(stderr), err_tx, wake);
        let mut client = Client {
            name: command.name.clone(),
            child,
            out: out_tx,
            incoming: in_rx,
            stderr: err_rx,
            next_id: 1,
            init_id: RequestId(0),
            shutdown_id: None,
            state: State::Starting,
            queued: Vec::new(),
            capabilities: None,
            encoding: Encoding::Utf16,
            config,
            progress: HashMap::new(),
            exit_reported: false,
        };
        let root_uri = crate::uri(root);
        // `rootUri` is deprecated in favour of the folders, but rust-analyzer
        // still reads it when the folders are absent elsewhere; both are sent.
        #[allow(deprecated)]
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: root_uri.clone(),
            initialization_options: Some(client.config.clone()),
            capabilities: client_capabilities(),
            workspace_folders: root_uri.map(|uri| {
                vec![WorkspaceFolder {
                    name: root
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    uri,
                }]
            }),
            client_info: Some(ClientInfo {
                name: "Oak".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            ..Default::default()
        };
        let id = client.allocate();
        client.init_id = id;
        client.send_now(&request_body(
            id,
            lsp_types::request::Initialize::METHOD,
            &params,
        ));
        Ok(client)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_ready(&self) -> bool {
        self.state == State::Ready
    }

    pub fn has_exited(&self) -> bool {
        self.state == State::Exited
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    pub fn capabilities(&self) -> Option<&ServerCapabilities> {
        self.capabilities.as_ref()
    }

    /// The semantic tokens legend, when the server provides tokens.
    pub fn semantic_tokens_legend(&self) -> Option<&SemanticTokensLegend> {
        match self
            .capabilities
            .as_ref()?
            .semantic_tokens_provider
            .as_ref()?
        {
            SemanticTokensServerCapabilities::SemanticTokensOptions(o) => Some(&o.legend),
            SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(o) => {
                Some(&o.semantic_tokens_options.legend)
            }
        }
    }

    /// The characters after which the server wants a completion request
    /// without a prefix (`.` and `:` for rust-analyzer).
    pub fn completion_triggers(&self) -> Vec<String> {
        self.capabilities
            .as_ref()
            .and_then(|c| c.completion_provider.as_ref())
            .and_then(|c| c.trigger_characters.clone())
            .unwrap_or_default()
    }

    /// The characters after which the server wants a signature help request.
    pub fn signature_triggers(&self) -> Vec<String> {
        self.capabilities
            .as_ref()
            .and_then(|c| c.signature_help_provider.as_ref())
            .and_then(|c| c.trigger_characters.clone())
            .unwrap_or_default()
    }

    fn allocate(&mut self) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        id
    }

    fn send_now(&self, body: &str) {
        let _ = self.out.send(transport::frame(body));
    }

    fn send(&mut self, body: String) {
        match self.state {
            State::Ready => self.send_now(&body),
            State::Starting => self.queued.push(transport::frame(&body)),
            State::Failed | State::Exited => {}
        }
    }

    /// Sends a request; its answer comes back as [`Event::Response`] with
    /// the id. Before the server is ready the request waits in a queue.
    pub fn request<R: Request>(&mut self, params: R::Params) -> RequestId
    where
        R::Params: Serialize,
    {
        let id = self.allocate();
        self.send(request_body(id, R::METHOD, &params));
        id
    }

    pub fn notify<N: Notification>(&mut self, params: N::Params)
    where
        N::Params: Serialize,
    {
        self.send(notification_body(N::METHOD, &params));
    }

    /// Answers one of the server's requests ([`Event::ApplyEdit`]).
    pub fn respond(&mut self, id: Value, result: Result<Value, ResponseError>) {
        let body = match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": e.code, "message": e.message}})
            }
        };
        self.send_now(&body.to_string());
    }

    /// Answers `workspace/applyEdit`.
    pub fn respond_applied(&mut self, id: Value, applied: bool, reason: Option<String>) {
        self.respond(id, Ok(json!({"applied": applied, "failureReason": reason})));
    }

    /// Asks the server to shut down; `exit` follows its answer (or a kill
    /// when it never answers: the owner keeps polling and may [`Client::kill`]).
    pub fn shutdown(&mut self) {
        if self.state != State::Ready {
            self.kill();
            return;
        }
        let id = self.allocate();
        self.shutdown_id = Some(id);
        self.send_now(&request_body(id, lsp_types::request::Shutdown::METHOD, &()));
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.state = State::Exited;
    }

    /// Everything that arrived since the last poll.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(line) = self.stderr.try_recv() {
            events.push(Event::Log(line));
        }
        let mut closed = false;
        loop {
            match self.incoming.try_recv() {
                Ok(message) => self.handle(message, &mut events),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        if closed && !self.exit_reported {
            self.exit_reported = true;
            let code = match self.child.try_wait() {
                Ok(Some(status)) => status.code(),
                _ => self.child.wait().ok().and_then(|s| s.code()),
            };
            if self.state == State::Starting {
                events.push(Event::Failed(format!(
                    "{} exited before it was initialized",
                    self.name
                )));
            }
            self.state = State::Exited;
            events.push(Event::Exited(code));
        }
        events
    }

    fn handle(&mut self, message: Incoming, events: &mut Vec<Event>) {
        match message {
            Incoming::Response { id, result, error } => {
                let Some(id) = id.as_i64().map(RequestId) else {
                    return;
                };
                if id == self.init_id {
                    self.finish_initialize(result, error, events);
                } else if Some(id) == self.shutdown_id {
                    self.send_now(&notification_body(
                        lsp_types::notification::Exit::METHOD,
                        &(),
                    ));
                    self.shutdown_id = None;
                } else {
                    let result = match error {
                        Some(e) => Err(ResponseError {
                            code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
                            message: e
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        }),
                        None => Ok(result.unwrap_or(Value::Null)),
                    };
                    events.push(Event::Response { id, result });
                }
            }
            Incoming::Notification { method, params } => self.notification(&method, params, events),
            Incoming::Request { id, method, params } => {
                self.server_request(id, &method, params, events)
            }
        }
    }

    fn finish_initialize(
        &mut self,
        result: Option<Value>,
        error: Option<Value>,
        events: &mut Vec<Event>,
    ) {
        if let Some(e) = error {
            self.state = State::Failed;
            let message = e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("initialize failed")
                .to_string();
            events.push(Event::Failed(message));
            return;
        }
        let parsed: Result<InitializeResult, _> =
            serde_json::from_value(result.unwrap_or(Value::Null));
        match parsed {
            Ok(init) => {
                self.encoding = Encoding::from_kind(init.capabilities.position_encoding.as_ref());
                self.capabilities = Some(init.capabilities);
                self.state = State::Ready;
                self.send_now(&notification_body(
                    lsp_types::notification::Initialized::METHOD,
                    &InitializedParams {},
                ));
                for frame in std::mem::take(&mut self.queued) {
                    let _ = self.out.send(frame);
                }
                events.push(Event::Ready);
            }
            Err(e) => {
                self.state = State::Failed;
                events.push(Event::Failed(format!("initialize result: {e}")));
            }
        }
    }

    fn notification(&mut self, method: &str, params: Value, events: &mut Vec<Event>) {
        match method {
            "textDocument/publishDiagnostics" => {
                if let Ok(p) = serde_json::from_value::<PublishDiagnosticsParams>(params) {
                    events.push(Event::Diagnostics(p));
                }
            }
            "$/progress" => {
                let Ok(p) = serde_json::from_value::<ProgressParams>(params) else {
                    return;
                };
                let token = match p.token {
                    lsp_types::NumberOrString::Number(n) => n.to_string(),
                    lsp_types::NumberOrString::String(s) => s,
                };
                let ProgressParamsValue::WorkDone(progress) = p.value;
                let (title, message, percentage, done) = match progress {
                    WorkDoneProgress::Begin(b) => {
                        self.progress.insert(token.clone(), b.title.clone());
                        (b.title, b.message, b.percentage, false)
                    }
                    WorkDoneProgress::Report(r) => (
                        self.progress.get(&token).cloned().unwrap_or_default(),
                        r.message,
                        r.percentage,
                        false,
                    ),
                    WorkDoneProgress::End(e) => (
                        self.progress.remove(&token).unwrap_or_default(),
                        e.message,
                        None,
                        true,
                    ),
                };
                events.push(Event::Progress {
                    token,
                    title,
                    message,
                    percentage,
                    done,
                });
            }
            "window/showMessage" => {
                if let Ok(p) = serde_json::from_value::<ShowMessageParams>(params) {
                    events.push(Event::Message {
                        level: MessageLevel::from(p.typ),
                        text: p.message,
                    });
                }
            }
            "window/logMessage" => {
                if let Ok(p) = serde_json::from_value::<LogMessageParams>(params) {
                    events.push(Event::Log(p.message));
                }
            }
            _ => {}
        }
    }

    fn server_request(&mut self, id: Value, method: &str, params: Value, events: &mut Vec<Event>) {
        match method {
            "window/workDoneProgress/create"
            | "client/registerCapability"
            | "client/unregisterCapability"
            | "window/showMessageRequest"
            | "workspace/inlayHint/refresh"
            | "workspace/diagnostic/refresh" => self.respond(id, Ok(Value::Null)),
            "workspace/semanticTokens/refresh" => {
                self.respond(id, Ok(Value::Null));
                events.push(Event::RefreshSemanticTokens);
            }
            "workspace/codeLens/refresh" => {
                self.respond(id, Ok(Value::Null));
                events.push(Event::RefreshCodeLens);
            }
            "workspace/configuration" => {
                let items = serde_json::from_value::<ConfigurationParams>(params)
                    .map(|p| p.items)
                    .unwrap_or_default();
                let answer: Vec<Value> = items
                    .iter()
                    .map(|item| self.configuration(item.section.as_deref()))
                    .collect();
                self.respond(id, Ok(Value::Array(answer)));
            }
            "workspace/applyEdit" => {
                match serde_json::from_value::<ApplyWorkspaceEditParams>(params) {
                    Ok(p) => events.push(Event::ApplyEdit {
                        id,
                        label: p.label,
                        edit: p.edit,
                    }),
                    Err(e) => self.respond(id, Err(ResponseError::new(-32602, e.to_string()))),
                }
            }
            "workspace/workspaceFolders" => {
                self.respond(id, Ok(Value::Null));
            }
            _ => self.respond(
                id,
                Err(ResponseError::new(
                    -32601,
                    format!("{method} is not handled"),
                )),
            ),
        }
    }

    /// The settings subtree a `workspace/configuration` item asks for: the
    /// whole config for the server's own section (or none), else the
    /// dotted path into it.
    fn configuration(&self, section: Option<&str>) -> Value {
        let Some(section) = section else {
            return self.config.clone();
        };
        if section == self.name {
            return self.config.clone();
        }
        let mut at = &self.config;
        for part in section.split('.') {
            match at.get(part) {
                Some(v) => at = v,
                None => return Value::Null,
            }
        }
        at.clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.state != State::Exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn request_body(id: RequestId, method: &str, params: &impl Serialize) -> String {
    json!({"jsonrpc": "2.0", "id": id.0, "method": method, "params": params}).to_string()
}

fn notification_body(method: &str, params: &impl Serialize) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

/// The settings for one server out of Fleet's flat keys: every
/// `<prefix>.a.b` becomes `{"a": {"b": value}}` (what rust-analyzer reads
/// as its `initializationOptions` and `workspace/configuration`).
pub fn config_from_settings(prefix: &str, settings: &serde_json::Map<String, Value>) -> Value {
    let mut root = serde_json::Map::new();
    let dotted = format!("{prefix}.");
    for (key, value) in settings {
        let Some(rest) = key.strip_prefix(&dotted) else {
            continue;
        };
        let parts: Vec<&str> = rest.split('.').collect();
        let mut node = &mut root;
        for (i, part) in parts.iter().enumerate() {
            if i + 1 == parts.len() {
                node.insert(part.to_string(), value.clone());
            } else {
                let entry = node
                    .entry(part.to_string())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !entry.is_object() {
                    *entry = Value::Object(serde_json::Map::new());
                }
                node = entry.as_object_mut().expect("an object");
            }
        }
    }
    Value::Object(root)
}

/// What Oak can do with a server's answers, as the handshake declares it.
pub fn client_capabilities() -> ClientCapabilities {
    let formats = vec![MarkupKind::Markdown, MarkupKind::PlainText];
    ClientCapabilities {
        workspace: Some(WorkspaceClientCapabilities {
            apply_edit: Some(true),
            workspace_edit: Some(WorkspaceEditClientCapabilities {
                document_changes: Some(true),
                ..Default::default()
            }),
            configuration: Some(true),
            workspace_folders: Some(true),
            semantic_tokens: Some(SemanticTokensWorkspaceClientCapabilities {
                refresh_support: Some(true),
            }),
            ..Default::default()
        }),
        text_document: Some(TextDocumentClientCapabilities {
            synchronization: Some(TextDocumentSyncClientCapabilities {
                did_save: Some(true),
                ..Default::default()
            }),
            completion: Some(CompletionClientCapabilities {
                completion_item: Some(CompletionItemCapability {
                    snippet_support: Some(true),
                    documentation_format: Some(formats.clone()),
                    deprecated_support: Some(true),
                    preselect_support: Some(true),
                    insert_replace_support: Some(true),
                    resolve_support: Some(CompletionItemCapabilityResolveSupport {
                        properties: vec![
                            "documentation".into(),
                            "detail".into(),
                            "additionalTextEdits".into(),
                        ],
                    }),
                    label_details_support: Some(true),
                    ..Default::default()
                }),
                context_support: Some(true),
                ..Default::default()
            }),
            hover: Some(HoverClientCapabilities {
                content_format: Some(formats.clone()),
                ..Default::default()
            }),
            signature_help: Some(SignatureHelpClientCapabilities {
                signature_information: Some(SignatureInformationSettings {
                    documentation_format: Some(formats),
                    parameter_information: Some(ParameterInformationSettings {
                        label_offset_support: Some(true),
                    }),
                    active_parameter_support: Some(true),
                }),
                context_support: Some(true),
                ..Default::default()
            }),
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                related_information: Some(true),
                version_support: Some(true),
                ..Default::default()
            }),
            rename: Some(RenameClientCapabilities {
                prepare_support: Some(true),
                ..Default::default()
            }),
            code_action: Some(CodeActionClientCapabilities {
                code_action_literal_support: Some(CodeActionLiteralSupport {
                    code_action_kind: CodeActionKindLiteralSupport {
                        value_set: vec![
                            "".into(),
                            "quickfix".into(),
                            "refactor".into(),
                            "refactor.extract".into(),
                            "refactor.inline".into(),
                            "refactor.rewrite".into(),
                            "source".into(),
                            "source.organizeImports".into(),
                        ],
                    },
                }),
                is_preferred_support: Some(true),
                data_support: Some(true),
                resolve_support: Some(CodeActionCapabilityResolveSupport {
                    properties: vec!["edit".into()],
                }),
                ..Default::default()
            }),
            semantic_tokens: Some(SemanticTokensClientCapabilities {
                requests: SemanticTokensClientCapabilitiesRequests {
                    range: Some(false),
                    full: Some(SemanticTokensFullOptions::Bool(true)),
                },
                token_types: vec![
                    SemanticTokenType::NAMESPACE,
                    SemanticTokenType::TYPE,
                    SemanticTokenType::CLASS,
                    SemanticTokenType::ENUM,
                    SemanticTokenType::INTERFACE,
                    SemanticTokenType::STRUCT,
                    SemanticTokenType::TYPE_PARAMETER,
                    SemanticTokenType::PARAMETER,
                    SemanticTokenType::VARIABLE,
                    SemanticTokenType::PROPERTY,
                    SemanticTokenType::ENUM_MEMBER,
                    SemanticTokenType::FUNCTION,
                    SemanticTokenType::METHOD,
                    SemanticTokenType::MACRO,
                    SemanticTokenType::KEYWORD,
                    SemanticTokenType::COMMENT,
                    SemanticTokenType::STRING,
                    SemanticTokenType::NUMBER,
                    SemanticTokenType::OPERATOR,
                    SemanticTokenType::DECORATOR,
                ],
                token_modifiers: vec![
                    SemanticTokenModifier::DECLARATION,
                    SemanticTokenModifier::DEFINITION,
                    SemanticTokenModifier::READONLY,
                    SemanticTokenModifier::STATIC,
                    SemanticTokenModifier::DEPRECATED,
                    SemanticTokenModifier::ABSTRACT,
                    SemanticTokenModifier::ASYNC,
                    SemanticTokenModifier::MODIFICATION,
                    SemanticTokenModifier::DOCUMENTATION,
                    SemanticTokenModifier::DEFAULT_LIBRARY,
                ],
                formats: vec![TokenFormat::RELATIVE],
                overlapping_token_support: Some(false),
                multiline_token_support: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        }),
        window: Some(WindowClientCapabilities {
            work_done_progress: Some(true),
            ..Default::default()
        }),
        general: Some(GeneralClientCapabilities {
            position_encodings: Some(Encoding::offered()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Positions a request needs, built from Oak's row and LSP column.
pub fn text_position(uri: Url, line: u32, character: u32) -> lsp_types::TextDocumentPositionParams {
    lsp_types::TextDocumentPositionParams {
        text_document: lsp_types::TextDocumentIdentifier { uri },
        position: lsp_types::Position { line, character },
    }
}

pub fn no_progress() -> WorkDoneProgressParams {
    WorkDoneProgressParams::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::notification::{DidChangeTextDocument, DidOpenTextDocument};
    use lsp_types::request::{Completion, GotoDefinition, HoverRequest, SemanticTokensFullRequest};
    use lsp_types::{
        CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
        DidOpenTextDocumentParams, GotoDefinitionParams, GotoDefinitionResponse, Hover,
        HoverParams, PartialResultParams, SemanticTokensParams, SemanticTokensResult,
        TextDocumentContentChangeEvent, TextDocumentItem, VersionedTextDocumentIdentifier,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    #[test]
    fn settings_nest_by_dots() {
        let mut settings = serde_json::Map::new();
        settings.insert("rust-analyzer.completion.limit".into(), json!(20));
        settings.insert("rust-analyzer.imports.prefix".into(), json!("crate"));
        settings.insert("editor.fontSize".into(), json!(13));
        let config = config_from_settings("rust-analyzer", &settings);
        assert_eq!(
            config,
            json!({"completion": {"limit": 20}, "imports": {"prefix": "crate"}})
        );
    }

    #[test]
    fn capabilities_serialize() {
        let v = serde_json::to_value(client_capabilities()).unwrap();
        assert_eq!(v["general"]["positionEncodings"][0], json!("utf-32"));
        assert_eq!(
            v["textDocument"]["completion"]["completionItem"]["snippetSupport"],
            json!(true)
        );
    }

    /// A tiny Cargo project for rust-analyzer.
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn helper(x: u32) -> u32 {\n    x + 1\n}\n\nfn main() {\n    let total = helper(41);\n    println!(\"{total}\");\n}\n",
        )
        .unwrap();
        dir
    }

    struct Harness {
        client: Client,
        events: Vec<Event>,
    }

    impl Harness {
        fn pump(&mut self) {
            self.events.extend(self.client.poll());
        }

        /// Polls until `found` picks something out of the events, within `secs`.
        fn wait<T>(&mut self, secs: u64, mut found: impl FnMut(&Event) -> Option<T>) -> Option<T> {
            let deadline = Instant::now() + Duration::from_secs(secs);
            let mut seen = 0;
            while Instant::now() < deadline {
                self.pump();
                for e in &self.events[seen..] {
                    if let Some(t) = found(e) {
                        return Some(t);
                    }
                }
                seen = self.events.len();
                if self.client.has_exited() {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            None
        }

        fn response<T: serde::de::DeserializeOwned>(
            &mut self,
            id: RequestId,
            secs: u64,
        ) -> Result<T, ResponseError> {
            let result = self
                .wait(secs, |e| match e {
                    Event::Response { id: got, result } if *got == id => Some(result.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no answer to request {id:?} in {secs} s"));
            match result {
                Ok(v) => match serde_json::from_value(v.clone()) {
                    Ok(t) => Ok(t),
                    Err(e) => panic!("cannot read the answer {v}: {e}"),
                },
                Err(e) => Err(e),
            }
        }

        /// Asks until the server has an answer: while its workspace loads it
        /// answers `null` or "content modified" (-32801) and must be asked again.
        fn ask<R: Request, T: serde::de::DeserializeOwned>(
            &mut self,
            secs: u64,
            params: impl Fn() -> R::Params,
        ) -> Option<T>
        where
            R::Params: Serialize,
        {
            let deadline = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < deadline {
                let id = self.client.request::<R>(params());
                match self.response::<Option<T>>(id, 60) {
                    Ok(Some(t)) => return Some(t),
                    Ok(None) => {}
                    Err(e) if e.code == -32801 => {}
                    Err(e) => panic!("{} answered an error: {e:?}", self.client.name()),
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            None
        }
    }

    #[test]
    fn rust_analyzer_round_trip() {
        let Some(command) = crate::servers::rust_analyzer() else {
            eprintln!("rust-analyzer is not installed; skipping");
            return;
        };
        let dir = project();
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        // No `cargo check` on save: the test needs the server's own analysis only.
        let config = json!({"checkOnSave": false, "cachePriming": {"enable": false}});
        let client = Client::spawn(&command, dir.path(), config, move || {
            w.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        let mut h = Harness {
            client,
            events: Vec::new(),
        };
        assert!(
            h.wait(60, |e| matches!(e, Event::Ready).then_some(()))
                .is_some(),
            "the server did not become ready: {:?}",
            h.events
        );
        assert!(wakes.load(Ordering::SeqCst) >= 1);
        // rust-analyzer takes the first encoding it supports from the offered list.
        assert_ne!(h.client.encoding(), Encoding::Utf16);
        assert!(h.client.semantic_tokens_legend().is_some());
        assert!(h.client.completion_triggers().contains(&".".to_string()));

        let path = dir.path().join("src/main.rs");
        let uri = crate::uri(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        h.client
            .notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "rust".into(),
                    version: 1,
                    text: text.clone(),
                },
            });
        // The workspace loads in the background, reported as progress
        // (Fetching, Roots Scanned, Indexing...); the answers below need it.
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut in_flight: std::collections::HashSet<String> = Default::default();
        let mut begun = false;
        let mut seen = 0;
        while Instant::now() < deadline {
            h.pump();
            for e in &h.events[seen..] {
                if let Event::Progress { token, done, .. } = e {
                    begun = true;
                    if *done {
                        in_flight.remove(token);
                    } else {
                        in_flight.insert(token.clone());
                    }
                }
            }
            seen = h.events.len();
            if begun && in_flight.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(begun, "no progress was reported while loading");
        // The report after the last `Indexing` end still settles for a moment.
        std::thread::sleep(Duration::from_millis(500));

        let hover: Option<Hover> = h.ask::<HoverRequest, _>(120, || HoverParams {
            text_document_position_params: text_position(uri.clone(), 5, 17),
            work_done_progress_params: no_progress(),
        });
        let hover = hover.expect("a hover over `helper`");
        assert!(crate::markup::hover_markdown(&hover).contains("fn helper"));

        let def: Option<GotoDefinitionResponse> =
            h.ask::<GotoDefinition, _>(120, || GotoDefinitionParams {
                text_document_position_params: text_position(uri.clone(), 5, 17),
                work_done_progress_params: no_progress(),
                partial_result_params: PartialResultParams::default(),
            });
        match def.expect("a definition") {
            GotoDefinitionResponse::Link(links) => {
                assert_eq!(links[0].target_selection_range.start.line, 0);
            }
            GotoDefinitionResponse::Scalar(l) => assert_eq!(l.range.start.line, 0),
            GotoDefinitionResponse::Array(ls) => assert_eq!(ls[0].range.start.line, 0),
        }

        // Type `total.` on a new line and complete after the dot.
        let changed = text.replace("    println!", "    total.\n    println!");
        h.client
            .notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: 2,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: changed,
                }],
            });
        let completion: Option<CompletionResponse> =
            h.ask::<Completion, _>(120, || CompletionParams {
                text_document_position: text_position(uri.clone(), 6, 10),
                work_done_progress_params: no_progress(),
                partial_result_params: PartialResultParams::default(),
                context: None,
            });
        let items = match completion.expect("completions") {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        };
        assert!(
            items.iter().any(|i| i.label.starts_with("pow")),
            "u32 methods expected, got {:?}",
            items.iter().map(|i| &i.label).take(10).collect::<Vec<_>>()
        );

        let tokens: Option<SemanticTokensResult> =
            h.ask::<SemanticTokensFullRequest, _>(120, || SemanticTokensParams {
                work_done_progress_params: no_progress(),
                partial_result_params: PartialResultParams::default(),
                text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
            });
        let data = match tokens.expect("semantic tokens") {
            SemanticTokensResult::Tokens(t) => t.data,
            SemanticTokensResult::Partial(p) => p.data,
        };
        let legend = h.client.semantic_tokens_legend().unwrap().clone();
        let decoded = crate::semantic::decode(&data, &legend);
        assert!(decoded.iter().any(|t| t.line == 0
            && t.kind == "function"
            && t.modifiers.iter().any(|m| m == "declaration")));

        h.client.shutdown();
        let exited = h.wait(20, |e| matches!(e, Event::Exited(_)).then_some(()));
        assert!(exited.is_some(), "the server did not exit after shutdown");
    }
}
