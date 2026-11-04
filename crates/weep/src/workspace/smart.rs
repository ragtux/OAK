//! Smart Mode in the workspace (M7): the language servers behind the
//! toolbar's lightning, one per language (`weep_lsp::servers`), fed the open
//! documents and asked on the editors' behalf. The servers' answers arrive
//! through a wake channel and [`Workspace::drain_servers`], as a run
//! configuration's output does; each request carries a reply closure that
//! runs with the answer.
//!
//! What runs here: `completion` and `smart-completion`,
//! `show-parameter-info`, `show-documentation`, `go-to-declaration`,
//! `go-to-type-declaration`, `find-usages`, `rename`, `quick-fixes`,
//! `apply-primary-quick-fix`, `reformat-code` and `editor.formatOnSave`,
//! `optimize-imports`, `back` and `forward`; the diagnostics as the
//! documents' problems and the semantic tokens as their highlighting
//! (`editor.semanticHighlighting`).

use super::*;
use crate::document::{DocumentEvent, Lens, Problem, SemanticSpan, Severity};
use crate::editor::smart::Candidate;
use crate::list_popup::{ListPopup, ListPopupEvent, ListRow};
use crate::menu::MenuEntry;
use crate::notifications::Level;
use crate::toolbar::SmartIndicator;
use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui::{AnyElement, Task};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::rc::Rc;
use weep_lsp::client::{config_from_settings, no_progress};
use weep_lsp::edits::LineIndex;
use weep_lsp::lsp_types as lsp;
use weep_lsp::lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
};
use weep_lsp::lsp_types::request::{
    CodeActionRequest, CodeActionResolveRequest, CodeLensRequest, CodeLensResolve,
    Completion as CompletionRequest, ExecuteCommand, Formatting, GotoDefinition,
    GotoTypeDefinition, HoverRequest, PrepareRenameRequest, References, Rename,
    ResolveCompletionItem, SemanticTokensFullRequest, SignatureHelpRequest,
};
use weep_lsp::{Client, Encoding, Event, RequestId, ResponseError};
use weep_syntax::Language;

type Reply = Box<dyn FnOnce(&mut Workspace, Result<Value, ResponseError>, &mut Context<Workspace>)>;

/// What the server was last told about a document: the version number
/// sent and the text it holds, which positions in its answers refer to.
struct Synced {
    version: i32,
    buffer_version: u64,
    text: String,
}

pub(super) struct Server {
    client: Client,
    language: Language,
    ready: bool,
    error: Option<String>,
    /// Progress in flight: token, title, message, percentage.
    progress: Vec<(String, String, Option<String>, Option<u32>)>,
    log: VecDeque<String>,
    open: HashMap<PathBuf, Synced>,
    pending: HashMap<RequestId, Reply>,
    _pump: Task<()>,
}

/// Where the caret or a char is, as the server wants it.
struct Located {
    server: usize,
    uri: lsp::Url,
    position: lsp::Position,
    version: u64,
    path: PathBuf,
}

pub(super) struct SmartMode {
    pub enabled: bool,
    servers: Vec<Server>,
    /// Languages whose server failed; not retried until the mode is toggled.
    failed: HashMap<Language, String>,
    /// `back` and `forward`: positions jumped from, and where in them we are
    /// (`history.len()` is the present).
    history: Vec<(PathBuf, usize)>,
    history_ix: usize,
    tokens_epoch: HashMap<PathBuf, u64>,
    /// The same for the code lenses (the code vision rows).
    lens_epoch: HashMap<PathBuf, u64>,
    /// Per file, the number of the last code lens answer asked for and of
    /// the newest one shown: an answer's lenses resolve one by one, so an
    /// older answer (from before the server's workspace had loaded) can end
    /// after a newer one and must not replace it.
    lens_order: HashMap<PathBuf, (u64, u64)>,
    /// The code actions behind the quick-fix menu and the problem card's rows.
    fix_actions: Vec<(usize, lsp::CodeActionOrCommand)>,
    /// The last completion answer's items (`completionItem/resolve` needs them).
    completion_items: Vec<lsp::CompletionItem>,
    completion_server: Option<usize>,
    list: Option<Entity<ListPopup>>,
    list_targets: Vec<(PathBuf, Range<usize>)>,
    /// The inline usages: the editor they are laid in and the panel.
    usages: Option<(Entity<EditorView>, Entity<crate::usages::UsagesPanel>)>,
    /// The Smart Mode card under the lightning is open, and its focus
    /// handle (made on first use) so Escape closes it.
    pub card_open: bool,
    card_focus: Option<FocusHandle>,
}

impl SmartMode {
    pub fn new(enabled: bool) -> SmartMode {
        SmartMode {
            enabled,
            servers: Vec::new(),
            failed: HashMap::new(),
            history: Vec::new(),
            history_ix: 0,
            tokens_epoch: HashMap::new(),
            lens_epoch: HashMap::new(),
            lens_order: HashMap::new(),
            fix_actions: Vec::new(),
            completion_items: Vec::new(),
            completion_server: None,
            list: None,
            list_targets: Vec::new(),
            usages: None,
            card_open: false,
            card_focus: None,
        }
    }
}

/// A code lens answer while its lenses resolve: how many are still out,
/// each lens (resolved or as it came), and whether one should be asked
/// again.
struct LensBatch {
    waiting: usize,
    lenses: Vec<Option<lsp::CodeLens>>,
    retry: bool,
}

/// An error that means "ask again later": the content changed under the
/// request (-32801) or the server cancelled it (-32802), as rust-analyzer
/// does while its workspace loads.
fn is_retry(e: &ResponseError) -> bool {
    matches!(e.code, -32801 | -32802)
}

fn severity_of(d: &lsp::Diagnostic) -> Severity {
    match d.severity {
        Some(lsp::DiagnosticSeverity::WARNING) => Severity::Warning,
        Some(lsp::DiagnosticSeverity::INFORMATION) => Severity::Info,
        Some(lsp::DiagnosticSeverity::HINT) => Severity::Weak,
        _ => Severity::Error,
    }
}

fn first_chars(strings: Vec<String>) -> Vec<char> {
    strings.iter().filter_map(|s| s.chars().next()).collect()
}

/// The file-type icon of a path (by its name).
fn icon_for(path: &Path) -> &'static str {
    file_icon(path.file_name().and_then(|n| n.to_str()).unwrap_or(""))
}

/// A path under the root as the popups print it (`src/main.rs`).
fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

impl Workspace {
    // ---- the lightning -----------------------------------------------------------

    /// The button's state and the tooltip's second line.
    pub(super) fn smart_indicator(&self) -> (SmartIndicator, SharedString) {
        if !self.smart.enabled {
            return (SmartIndicator::Off, "".into());
        }
        if let Some(s) = self.smart.servers.iter().find(|s| s.error.is_some()) {
            let e = s.error.clone().unwrap_or_default();
            return (
                SmartIndicator::Error,
                format!("{}: {e}", s.client.name()).into(),
            );
        }
        if let Some((language, e)) = self.smart.failed.iter().next() {
            return (
                SmartIndicator::Error,
                format!("{}: {e}", language.display_name()).into(),
            );
        }
        let mut details = Vec::new();
        let mut starting = false;
        for s in &self.smart.servers {
            if !s.ready {
                starting = true;
                details.push(format!("{}: starting", s.client.name()));
            } else if let Some((_, title, message, percentage)) = s.progress.last() {
                starting = true;
                let mut d = format!("{}: {title}", s.client.name());
                if let Some(m) = message {
                    d.push_str(&format!(" {m}"));
                }
                if let Some(p) = percentage {
                    d.push_str(&format!(" {p}%"));
                }
                details.push(d);
            } else {
                details.push(s.client.name().to_string());
            }
        }
        let state = if starting {
            SmartIndicator::Starting
        } else {
            SmartIndicator::On
        };
        (state, details.join("\n").into())
    }

    /// A click on the lightning: Smart Mode turns on when it was off, and
    /// the card under the button opens (thirteenth pass: Fleet enables on
    /// the click and shows "Enabling Smart Mode..." with a Disable button).
    pub(super) fn lightning_clicked(&mut self, cx: &mut Context<Self>) {
        if !self.smart.enabled {
            self.toggle_smart_mode(cx);
            self.smart.card_open = true;
        } else {
            self.smart.card_open = !self.smart.card_open;
        }
        if self.smart.card_open {
            let handle = self
                .smart
                .card_focus
                .get_or_insert_with(|| cx.focus_handle())
                .clone();
            self.focus_later(handle, cx);
        }
        cx.notify();
    }

    /// The card under the lightning, in the Notifications popup's frame: a
    /// title, a line of text, the Disable (or Enable) button, then the
    /// servers' errors when there are any.
    fn render_smart_card(
        &self,
        theme: &Theme,
        m: &Metrics,
        ws: Entity<Workspace>,
    ) -> Option<AnyElement> {
        if !self.smart.card_open {
            return None;
        }
        const DESCRIPTION: &str =
            "Analyzes code to provide completion, error detection, fixes, and refactorings";
        let (state, detail) = self.smart_indicator();
        let (title, text) = match state {
            SmartIndicator::Off => ("Smart Mode is off", DESCRIPTION),
            SmartIndicator::Starting => ("Enabling Smart Mode...", DESCRIPTION),
            SmartIndicator::On => ("Smart Mode is on", DESCRIPTION),
            SmartIndicator::Error => (
                "Smart Mode is partially on",
                "Some of Smart Mode functionality didn't start properly",
            ),
        };
        let errors: Vec<SharedString> = if state == SmartIndicator::Error {
            detail
                .split('\n')
                .filter(|l| !l.is_empty())
                .map(|l| SharedString::from(l.to_string()))
                .collect()
        } else {
            Vec::new()
        };
        let label: SharedString = if self.smart.enabled {
            "Disable".into()
        } else {
            "Enable".into()
        };
        let close = ws.clone();
        let escape = ws.clone();
        let focus = self.smart.card_focus.clone()?;
        let tui = TuiCard {
            title,
            text,
            errors: errors.clone(),
            label: label.clone(),
            ws: ws.clone(),
            focus: focus.clone(),
        };
        let button = crate::dialog::button(
            "smart-card-button",
            label,
            crate::dialog::ButtonKind::Secondary,
            false,
            theme,
            m,
        )
        .on_click(move |_, _, cx| {
            ws.update(cx, |this, cx| {
                this.toggle_smart_mode(cx);
                this.smart.card_open = this.smart.enabled;
                cx.notify();
            })
        });
        let fleet = Some(
            deferred(
                div()
                    .id("smart-card")
                    .occlude()
                    .absolute()
                    .right(px(m.notifications_right))
                    .top(px(m.toolbar_height + m.notifications_top_gap))
                    .w(px(m.notifications_width))
                    .flex()
                    .flex_col()
                    .rounded(px(m.popup_radius))
                    .border_1()
                    .border_color(theme.popup_border)
                    .bg(theme.popup_bg)
                    .font_family(theme.ui_font.clone())
                    .text_size(px(m.ui_font_size))
                    .text_color(theme.popup_text)
                    .shadow(vec![BoxShadow {
                        color: theme.shadow_medium,
                        offset: point(px(0.), px(m.popup_shadow_offset_y)),
                        blur_radius: px(m.popup_shadow_blur),
                        spread_radius: px(0.),
                    }])
                    .track_focus(&focus)
                    .key_context("Popup")
                    .on_action(move |_: &PopupCancel, _, cx| {
                        escape.update(cx, |this, cx| {
                            this.smart.card_open = false;
                            this.focus_active_editor(cx);
                            cx.notify();
                        })
                    })
                    .on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |this, cx| {
                            this.smart.card_open = false;
                            this.focus_active_editor(cx);
                            cx.notify();
                        })
                    })
                    .child(
                        div()
                            .px(px(m.smart_card_pad_x))
                            .pt(px(m.smart_card_pad_top))
                            .pb(px(m.smart_card_pad_bottom))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .line_height(px(m.smart_popup_line))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_color(theme.text_secondary)
                                    .line_height(px(m.smart_popup_line))
                                    .child(text),
                            )
                            .child(div().mt(px(m.smart_card_button_gap)).flex().child(button)),
                    )
                    .when(!errors.is_empty(), |d| {
                        d.child(div().h(px(1.)).flex_shrink_0().bg(theme.popup_separator))
                            .child(
                                div()
                                    .px(px(m.smart_card_pad_x))
                                    .py(px(m.smart_card_pad_top))
                                    .flex()
                                    .flex_col()
                                    .gap(px(m.smart_card_button_gap))
                                    .children(errors.into_iter().map(|e| {
                                        div()
                                            .flex()
                                            .items_start()
                                            .gap(px(m.notifications_icon_gap))
                                            .child(
                                                div()
                                                    .size(px(m.icon_size))
                                                    .flex_shrink_0()
                                                    .relative()
                                                    .child(
                                                        icon(
                                                            "weep/circle",
                                                            m.icon_size,
                                                            theme.icon_red,
                                                        )
                                                        .absolute()
                                                        .left_0()
                                                        .top_0(),
                                                    )
                                                    .child(
                                                        icon(
                                                            "weep/exclamation",
                                                            m.icon_size,
                                                            theme.problems_error_tag_text,
                                                        )
                                                        .absolute()
                                                        .left_0()
                                                        .top_0(),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .line_height(px(m.smart_popup_line))
                                                    .child(e),
                                            )
                                    })),
                            )
                    }),
            )
            .into_any_element(),
        );
        // The skin is known once the window is: the card is built there.
        let theme = theme.clone();
        let m = *m;
        let mut fleet = fleet;
        Some(
            crate::skin::with_window(move |window, cx| {
                if crate::skin::Skin::current(cx) == crate::skin::Skin::Tui {
                    tui.render(&theme, m, window, cx)
                } else {
                    fleet.take().expect("built once").into_any_element()
                }
            })
            .into_any_element(),
        )
    }

    pub(super) fn toggle_smart_mode(&mut self, cx: &mut Context<Self>) {
        self.smart.enabled = !self.smart.enabled;
        self.save_layout();
        if self.smart.enabled {
            self.smart.failed.clear();
            let docs = self.documents.clone();
            for doc in docs {
                self.smart_document_opened(&doc, cx);
            }
        } else {
            self.stop_servers(cx);
        }
        self.smart_refresh_editors(cx);
        cx.notify();
    }

    fn stop_servers(&mut self, cx: &mut Context<Self>) {
        for mut server in self.smart.servers.drain(..) {
            server.client.shutdown();
            // The exit is not awaited: the client is killed on drop if the
            // server lingers.
        }
        for doc in &self.documents {
            doc.update(cx, |d, cx| d.clear_server_state(cx));
        }
    }

    // ---- servers ------------------------------------------------------------------

    fn server_for(&self, language: Language) -> Option<usize> {
        self.smart
            .servers
            .iter()
            .position(|s| s.language == language)
    }

    /// The server for a language, started when there is none yet.
    fn ensure_server(&mut self, language: Language, cx: &mut Context<Self>) -> Option<usize> {
        if !self.smart.enabled {
            return None;
        }
        if let Some(ix) = self.server_for(language) {
            return Some(ix);
        }
        if self.smart.failed.contains_key(&language) {
            return None;
        }
        let id = weep_lsp::language_id(language.display_name());
        let command = weep_lsp::servers::for_language(id)?;
        let config = config_from_settings(&command.name, Self::settings(cx).values());
        let (tx, mut rx) = unbounded::<()>();
        let client = match Client::spawn(&command, &self.root, config, move || {
            let _ = tx.unbounded_send(());
        }) {
            Ok(c) => c,
            Err(e) => {
                self.smart.failed.insert(language, e.clone());
                self.notify_error("Smart Mode", format!("{}: {e}", command.name), cx);
                return None;
            }
        };
        let pump = cx.spawn(async move |this, cx| {
            while rx.next().await.is_some() {
                if this.update(cx, |this, cx| this.drain_servers(cx)).is_err() {
                    break;
                }
            }
        });
        self.smart.servers.push(Server {
            client,
            language,
            ready: false,
            error: None,
            progress: Vec::new(),
            log: VecDeque::new(),
            open: HashMap::new(),
            pending: HashMap::new(),
            _pump: pump,
        });
        cx.notify();
        Some(self.smart.servers.len() - 1)
    }

    /// Polls every server and acts on what arrived.
    fn drain_servers(&mut self, cx: &mut Context<Self>) {
        let mut ix = 0;
        while ix < self.smart.servers.len() {
            let events = self.smart.servers[ix].client.poll();
            let mut removed = false;
            for event in events {
                if self.server_event(ix, event, cx) {
                    removed = true;
                    break;
                }
            }
            if !removed {
                ix += 1;
            }
        }
    }

    /// Handles one event; `true` when the server at `ix` is gone.
    fn server_event(&mut self, ix: usize, event: Event, cx: &mut Context<Self>) -> bool {
        match event {
            Event::Ready => {
                self.smart.servers[ix].ready = true;
                let paths: Vec<PathBuf> = self.smart.servers[ix].open.keys().cloned().collect();
                for path in paths {
                    self.request_semantic_tokens(&path, cx);
                    self.request_code_vision(&path, cx);
                }
                self.smart_refresh_editors(cx);
                cx.notify();
            }
            Event::Failed(e) => {
                let name = self.smart.servers[ix].client.name().to_string();
                self.smart.servers[ix].error = Some(e.clone());
                self.notify_error("Smart Mode", format!("{name}: {e}"), cx);
                cx.notify();
            }
            Event::Response { id, result } => {
                if let Some(reply) = self.smart.servers[ix].pending.remove(&id) {
                    reply(self, result, cx);
                }
            }
            Event::Diagnostics(params) => self.publish_diagnostics(ix, params, cx),
            Event::Progress {
                token,
                title,
                message,
                percentage,
                done,
            } => {
                let server = &mut self.smart.servers[ix];
                server.progress.retain(|p| p.0 != token);
                if !done {
                    server.progress.push((token, title, message, percentage));
                }
                cx.notify();
            }
            Event::Message { level, text } => {
                let name = self.smart.servers[ix].client.name().to_string();
                let level = match level {
                    weep_lsp::MessageLevel::Error => Level::Error,
                    weep_lsp::MessageLevel::Warning => Level::Warning,
                    _ => Level::Info,
                };
                self.notify(level, name, Some(text), cx);
            }
            Event::Log(line) => {
                let log = &mut self.smart.servers[ix].log;
                log.push_back(line);
                while log.len() > 200 {
                    log.pop_front();
                }
            }
            Event::ApplyEdit { id, edit, .. } => {
                let enc = self.smart.servers[ix].client.encoding();
                self.apply_workspace_edit(&edit, enc, cx);
                self.smart.servers[ix]
                    .client
                    .respond_applied(id, true, None);
            }
            Event::RefreshSemanticTokens => {
                let paths: Vec<PathBuf> = self.smart.servers[ix].open.keys().cloned().collect();
                for path in paths {
                    self.request_semantic_tokens(&path, cx);
                }
            }
            Event::RefreshCodeLens => {
                let paths: Vec<PathBuf> = self.smart.servers[ix].open.keys().cloned().collect();
                for path in paths {
                    self.request_code_vision(&path, cx);
                }
            }
            Event::Exited(code) => {
                let server = self.smart.servers.remove(ix);
                let name = server.client.name().to_string();
                let message = match code {
                    Some(c) => format!("{name} exited with code {c}"),
                    None => format!("{name} stopped"),
                };
                if self.smart.enabled {
                    self.smart.failed.insert(server.language, message.clone());
                    let last: Vec<String> = server.log.iter().rev().take(3).cloned().collect();
                    let detail = if last.is_empty() {
                        message
                    } else {
                        format!("{message}\n{}", last.join("\n"))
                    };
                    self.notify_error("Smart Mode", detail, cx);
                }
                for doc in &self.documents {
                    if doc.read(cx).language == Some(server.language) {
                        doc.update(cx, |d, cx| d.clear_server_state(cx));
                    }
                }
                self.smart_refresh_editors(cx);
                cx.notify();
                return true;
            }
        }
        false
    }

    fn request<R: lsp::request::Request>(
        &mut self,
        server: usize,
        params: R::Params,
        reply: impl FnOnce(&mut Workspace, Result<Value, ResponseError>, &mut Context<Workspace>)
            + 'static,
    ) where
        R::Params: serde::Serialize,
    {
        let Some(s) = self.smart.servers.get_mut(server) else {
            return;
        };
        let id = s.client.request::<R>(params);
        s.pending.insert(id, Box::new(reply));
    }

    // ---- documents --------------------------------------------------------------------

    fn document_at(&self, path: &Path, cx: &App) -> Option<Entity<Document>> {
        self.documents
            .iter()
            .find(|d| d.read(cx).path.as_deref() == Some(path))
            .cloned()
    }

    /// An editor showing `path`, the active one when it does.
    fn editor_for_path(&self, path: &Path, cx: &App) -> Option<Entity<EditorView>> {
        if let Some(e) = self.active_editor() {
            if e.read(cx).path(cx).as_deref() == Some(path) {
                return Some(e.clone());
            }
        }
        self.panes
            .iter()
            .flat_map(|p| p.tabs.iter().filter_map(|t| t.editor()))
            .find(|e| e.read(cx).path(cx).as_deref() == Some(path))
            .cloned()
    }

    /// A document opened (or Smart Mode turned on): its server starts if
    /// needed and gets `didOpen`; the document's edits and saves follow.
    pub(super) fn smart_document_opened(&mut self, doc: &Entity<Document>, cx: &mut Context<Self>) {
        let (path, language, text, buffer_version) = {
            let d = doc.read(cx);
            let Some(path) = d.path.clone() else { return };
            let Some(language) = d.language else { return };
            (path, language, d.buffer.text(), d.buffer.version())
        };
        let Some(ix) = self.ensure_server(language, cx) else {
            return;
        };
        if self.smart.servers[ix].open.contains_key(&path) {
            return;
        }
        let Some(uri) = weep_lsp::uri(&path) else {
            return;
        };
        let server = &mut self.smart.servers[ix];
        server
            .client
            .notify::<DidOpenTextDocument>(lsp::DidOpenTextDocumentParams {
                text_document: lsp::TextDocumentItem {
                    uri,
                    language_id: weep_lsp::language_id(language.display_name()).to_string(),
                    version: 1,
                    text: text.clone(),
                },
            });
        server.open.insert(
            path.clone(),
            Synced {
                version: 1,
                buffer_version,
                text,
            },
        );
        cx.subscribe(doc, |this, doc, event: &DocumentEvent, cx| match event {
            DocumentEvent::Changed => this.smart_document_changed(&doc, cx),
            DocumentEvent::Saved => this.smart_document_saved(&doc, cx),
            DocumentEvent::StatusChanged => {}
        })
        .detach();
        if self.smart.servers[ix].ready {
            self.request_semantic_tokens(&path, cx);
            self.request_code_vision(&path, cx);
        }
    }

    fn smart_document_changed(&mut self, doc: &Entity<Document>, cx: &mut Context<Self>) {
        let (path, text, buffer_version) = {
            let d = doc.read(cx);
            let Some(path) = d.path.clone() else { return };
            (path, d.buffer.text(), d.buffer.version())
        };
        let Some(ix) = self
            .smart
            .servers
            .iter()
            .position(|s| s.open.contains_key(&path))
        else {
            return;
        };
        let server = &mut self.smart.servers[ix];
        let Some(synced) = server.open.get_mut(&path) else {
            return;
        };
        if synced.buffer_version == buffer_version {
            return;
        }
        synced.version += 1;
        synced.buffer_version = buffer_version;
        synced.text = text.clone();
        let version = synced.version;
        let Some(uri) = weep_lsp::uri(&path) else {
            return;
        };
        server
            .client
            .notify::<DidChangeTextDocument>(lsp::DidChangeTextDocumentParams {
                text_document: lsp::VersionedTextDocumentIdentifier { uri, version },
                content_changes: vec![lsp::TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text,
                }],
            });
        self.request_semantic_tokens(&path, cx);
        self.request_code_vision(&path, cx);
    }

    fn smart_document_saved(&mut self, doc: &Entity<Document>, cx: &mut Context<Self>) {
        let Some(path) = doc.read(cx).path.clone() else {
            return;
        };
        let Some(ix) = self
            .smart
            .servers
            .iter()
            .position(|s| s.open.contains_key(&path))
        else {
            return;
        };
        let Some(uri) = weep_lsp::uri(&path) else {
            return;
        };
        self.smart.servers[ix].client.notify::<DidSaveTextDocument>(
            lsp::DidSaveTextDocumentParams {
                text_document: lsp::TextDocumentIdentifier { uri },
                text: None,
            },
        );
    }

    /// The last tab on a file closed: `didClose`.
    pub(super) fn smart_document_closed(&mut self, path: &Path) {
        for server in &mut self.smart.servers {
            if server.open.remove(path).is_some() {
                if let Some(uri) = weep_lsp::uri(path) {
                    server
                        .client
                        .notify::<DidCloseTextDocument>(lsp::DidCloseTextDocumentParams {
                            text_document: lsp::TextDocumentIdentifier { uri },
                        });
                }
            }
        }
        self.smart.tokens_epoch.remove(path);
        self.smart.lens_epoch.remove(path);
        self.smart.lens_order.remove(path);
    }

    /// A new view: told whether its document has a server.
    pub(super) fn smart_editor_created(
        &mut self,
        editor: &Entity<EditorView>,
        cx: &mut Context<Self>,
    ) {
        let language = editor.read(cx).language(cx);
        let on = self.smart.enabled
            && language
                .and_then(|l| self.server_for(l))
                .is_some_and(|ix| self.smart.servers[ix].ready);
        let (ct, st) = match language.and_then(|l| self.server_for(l)) {
            Some(ix) if on => (
                first_chars(self.smart.servers[ix].client.completion_triggers()),
                first_chars(self.smart.servers[ix].client.signature_triggers()),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        editor.update(cx, |e, cx| e.set_smart(on, ct, st, cx));
    }

    fn smart_refresh_editors(&mut self, cx: &mut Context<Self>) {
        let editors: Vec<Entity<EditorView>> = self
            .panes
            .iter()
            .flat_map(|p| p.tabs.iter().filter_map(|t| t.editor().cloned()))
            .collect();
        for editor in editors {
            self.smart_editor_created(&editor, cx);
        }
    }

    fn publish_diagnostics(
        &mut self,
        ix: usize,
        params: lsp::PublishDiagnosticsParams,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = weep_lsp::path(&params.uri) else {
            return;
        };
        let Some(doc) = self.document_at(&path, cx) else {
            return;
        };
        let enc = self.smart.servers[ix].client.encoding();
        let Some(synced) = self.smart.servers[ix].open.get(&path) else {
            return;
        };
        // Positions refer to the version the server names; an older one
        // is read against the current text as an approximation.
        let buffer_version = synced.buffer_version;
        let index = LineIndex::new(&synced.text);
        let problems: Vec<Problem> = params
            .diagnostics
            .into_iter()
            .map(|d| {
                let tags = d.tags.clone().unwrap_or_default();
                Problem {
                    range: index.range(d.range, enc),
                    message: d.message.clone(),
                    severity: severity_of(&d),
                    unused: tags.contains(&lsp::DiagnosticTag::UNNECESSARY),
                    deprecated: tags.contains(&lsp::DiagnosticTag::DEPRECATED),
                    diagnostic: Some(d),
                }
            })
            .collect();
        doc.update(cx, |d, cx| d.set_diagnostics(buffer_version, problems, cx));
    }

    /// Asks for the file's semantic tokens 250 ms after the last change
    /// (`editor.semanticHighlighting`).
    fn request_semantic_tokens(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !Self::settings(cx).editor_semantic_highlighting() {
            return;
        }
        let epoch = self
            .smart
            .tokens_epoch
            .entry(path.to_path_buf())
            .or_insert(0);
        *epoch += 1;
        let epoch = *epoch;
        let path = path.to_path_buf();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.smart.tokens_epoch.get(&path) == Some(&epoch) {
                    this.send_semantic_tokens_request(&path, cx);
                }
            });
        })
        .detach();
    }

    fn send_semantic_tokens_request(&mut self, path: &Path, _cx: &mut Context<Self>) {
        let Some(ix) = self
            .smart
            .servers
            .iter()
            .position(|s| s.open.contains_key(path))
        else {
            return;
        };
        let server = &self.smart.servers[ix];
        if !server.ready || server.client.semantic_tokens_legend().is_none() {
            return;
        }
        let Some(uri) = weep_lsp::uri(path) else {
            return;
        };
        let Some(synced) = server.open.get(path) else {
            return;
        };
        let buffer_version = synced.buffer_version;
        let path = path.to_path_buf();
        self.request::<SemanticTokensFullRequest>(
            ix,
            lsp::SemanticTokensParams {
                work_done_progress_params: no_progress(),
                partial_result_params: Default::default(),
                text_document: lsp::TextDocumentIdentifier { uri },
            },
            move |this, result, cx| {
                let Ok(value) = result else { return };
                let Ok(Some(tokens)) =
                    serde_json::from_value::<Option<lsp::SemanticTokensResult>>(value)
                else {
                    return;
                };
                let data = match tokens {
                    lsp::SemanticTokensResult::Tokens(t) => t.data,
                    lsp::SemanticTokensResult::Partial(p) => p.data,
                };
                let Some(ix) = this
                    .smart
                    .servers
                    .iter()
                    .position(|s| s.open.contains_key(&path))
                else {
                    return;
                };
                let server = &this.smart.servers[ix];
                let Some(legend) = server.client.semantic_tokens_legend().cloned() else {
                    return;
                };
                let enc = server.client.encoding();
                let Some(synced) = server.open.get(&path) else {
                    return;
                };
                if synced.buffer_version != buffer_version {
                    // The text moved on; the next request is already scheduled.
                    return;
                }
                let index = LineIndex::new(&synced.text);
                let spans: Vec<SemanticSpan> = weep_lsp::semantic::decode(&data, &legend)
                    .into_iter()
                    .filter_map(|t| {
                        let attribute = weep_lsp::semantic::fleet_attribute(&t.kind, &t.modifiers)?;
                        let line = index.line(t.line as usize)?;
                        let start = index.offset(lsp::Position::new(t.line, t.start), enc);
                        let end_col = weep_lsp::column_from_lsp(line, t.start + t.len, enc);
                        let end = index.offset(lsp::Position::new(t.line, 0), enc) + end_col;
                        (end > start).then_some(SemanticSpan {
                            range: start..end,
                            attribute,
                        })
                    })
                    .collect();
                if let Some(doc) = this.document_at(&path, cx) {
                    doc.update(cx, |d, cx| d.set_semantic_spans(buffer_version, spans, cx));
                }
            },
        );
    }

    /// Asks for the file's code lenses (Fleet's code vision) 250 ms after
    /// the last change; whether they show is `editor.showCodeVision`'s, read
    /// by the views, so the setting applies at once.
    fn request_code_vision(&mut self, path: &Path, cx: &mut Context<Self>) {
        let epoch = self.smart.lens_epoch.entry(path.to_path_buf()).or_insert(0);
        *epoch += 1;
        let epoch = *epoch;
        let path = path.to_path_buf();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.smart.lens_epoch.get(&path) == Some(&epoch) {
                    this.send_code_vision_request(&path, cx);
                }
            });
        })
        .detach();
    }

    /// `textDocument/codeLens`, then `codeLens/resolve` for each lens that
    /// came without its command (rust-analyzer's implementations); the
    /// titles land on the document together, runnables left to the gutter.
    fn send_code_vision_request(&mut self, path: &Path, _cx: &mut Context<Self>) {
        let Some(ix) = self
            .smart
            .servers
            .iter()
            .position(|s| s.open.contains_key(path))
        else {
            return;
        };
        let server = &self.smart.servers[ix];
        let provides = server
            .client
            .capabilities()
            .is_some_and(|c| c.code_lens_provider.is_some());
        if !server.ready || !provides {
            return;
        }
        let Some(uri) = weep_lsp::uri(path) else {
            return;
        };
        let Some(synced) = server.open.get(path) else {
            return;
        };
        let buffer_version = synced.buffer_version;
        let path = path.to_path_buf();
        let order = self.smart.lens_order.entry(path.clone()).or_default();
        order.0 += 1;
        let seq = order.0;
        self.request::<CodeLensRequest>(
            ix,
            lsp::CodeLensParams {
                text_document: lsp::TextDocumentIdentifier { uri },
                work_done_progress_params: no_progress(),
                partial_result_params: Default::default(),
            },
            move |this, result, cx| {
                let value = match result {
                    Ok(value) => value,
                    Err(e) => {
                        // Asked while the server was still loading: again.
                        if is_retry(&e) {
                            this.request_code_vision(&path, cx);
                        }
                        return;
                    }
                };
                let lenses: Vec<lsp::CodeLens> =
                    serde_json::from_value::<Option<Vec<lsp::CodeLens>>>(value)
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(weep_lsp::lens::shows)
                        .collect();
                // Each lens resolved (or as it came); the last answer in
                // hands them all to the document.
                let unresolved = lenses.iter().filter(|l| l.command.is_none()).count();
                let slots = Rc::new(RefCell::new(LensBatch {
                    waiting: unresolved,
                    lenses: vec![None; lenses.len()],
                    retry: false,
                }));
                for (i, lens) in lenses.into_iter().enumerate() {
                    if lens.command.is_some() {
                        slots.borrow_mut().lenses[i] = Some(lens);
                        continue;
                    }
                    let slots = slots.clone();
                    let path = path.clone();
                    this.request::<CodeLensResolve>(ix, lens, move |this, result, cx| {
                        let retry = result.as_ref().is_err_and(is_retry);
                        let resolved = result
                            .ok()
                            .and_then(|v| serde_json::from_value::<lsp::CodeLens>(v).ok());
                        let done = {
                            let mut s = slots.borrow_mut();
                            s.lenses[i] = resolved;
                            s.waiting -= 1;
                            s.retry |= retry;
                            s.waiting == 0
                        };
                        if done {
                            let (all, retry) = {
                                let mut s = slots.borrow_mut();
                                (std::mem::take(&mut s.lenses), s.retry)
                            };
                            this.set_code_vision(&path, buffer_version, seq, all, cx);
                            if retry {
                                this.request_code_vision(&path, cx);
                            }
                        }
                    });
                }
                if unresolved == 0 {
                    let all = std::mem::take(&mut slots.borrow_mut().lenses);
                    this.set_code_vision(&path, buffer_version, seq, all, cx);
                }
            },
        );
    }

    /// The resolved lenses as the document's code vision, when the text is
    /// still the one they were asked about (else the next answer will do)
    /// and no newer answer (`seq`) is shown already.
    fn set_code_vision(
        &mut self,
        path: &Path,
        buffer_version: u64,
        seq: u64,
        lenses: Vec<Option<lsp::CodeLens>>,
        cx: &mut Context<Self>,
    ) {
        let Some(server) = self
            .smart
            .servers
            .iter()
            .find(|s| s.open.contains_key(path))
        else {
            return;
        };
        let enc = server.client.encoding();
        let Some(synced) = server.open.get(path) else {
            return;
        };
        if synced.buffer_version != buffer_version {
            return;
        }
        let order = self.smart.lens_order.entry(path.to_path_buf()).or_default();
        if seq < order.1 {
            return;
        }
        order.1 = seq;
        let index = LineIndex::new(&synced.text);
        let lenses: Vec<Lens> = lenses
            .into_iter()
            .flatten()
            .filter_map(|l| {
                let command = l.command?;
                (!weep_lsp::lens::is_runnable(&command)).then(|| Lens {
                    offset: index.offset(l.range.start, enc),
                    title: command.title.clone(),
                    command,
                })
            })
            .collect();
        if let Some(doc) = self.document_at(path, cx) {
            doc.update(cx, |d, cx| d.set_code_vision(buffer_version, lenses, cx));
        }
    }

    /// A click on a code vision entry: its locations in the inline usages
    /// panel under the line (one is gone to straight away); another
    /// command goes to the server when it says it runs it.
    fn code_vision_clicked(
        &mut self,
        editor: &Entity<EditorView>,
        row: usize,
        lens: &Lens,
        cx: &mut Context<Self>,
    ) {
        let Some(located) = self.locate(editor, Some(lens.offset), cx) else {
            return;
        };
        let locations = weep_lsp::lens::locations(&lens.command);
        if locations.is_empty() {
            let runs = self.smart.servers[located.server]
                .client
                .capabilities()
                .and_then(|c| c.execute_command_provider.as_ref())
                .is_some_and(|p| p.commands.contains(&lens.command.command));
            if runs {
                self.request::<ExecuteCommand>(
                    located.server,
                    lsp::ExecuteCommandParams {
                        command: lens.command.command.clone(),
                        arguments: lens.command.arguments.clone().unwrap_or_default(),
                        work_done_progress_params: no_progress(),
                    },
                    |_, _, _| {},
                );
            }
            return;
        }
        let enc = self.encoding_of(located.server);
        if let [only] = locations.as_slice() {
            if let Some(path) = weep_lsp::path(&only.uri) {
                let (range, _) = self.target_range(&path, only.range, enc, cx);
                self.record_history(cx);
                self.goto_position(path, range, cx);
            }
            return;
        }
        let files = self.usage_files(locations, enc, cx);
        let title = weep_lsp::lens::heading(&lens.title);
        self.show_usages(editor, row, title, files, cx);
    }

    /// Locations grouped by file for the inline usages panel: the active
    /// file first, then by path and position (estimate), each line trimmed
    /// as the list shows it.
    fn usage_files(
        &self,
        locations: Vec<lsp::Location>,
        enc: Encoding,
        cx: &mut Context<Self>,
    ) -> Vec<crate::usages::UsageFile> {
        let active = self.active_editor().and_then(|e| e.read(cx).path(cx));
        let mut found: Vec<(PathBuf, lsp::Range)> = locations
            .into_iter()
            .filter_map(|l| weep_lsp::path(&l.uri).map(|p| (p, l.range)))
            .collect();
        found.sort_by_key(|(path, range)| {
            (
                active.as_deref() != Some(path.as_path()),
                path.clone(),
                range.start,
            )
        });
        let mut files: Vec<crate::usages::UsageFile> = Vec::new();
        for (path, range) in found {
            let (chars, _) = self.target_range(&path, range, enc, cx);
            let line = self.line_text(&path, range.start.line as usize, cx);
            let indent = line.chars().take_while(|c| c.is_whitespace()).count();
            let text: String = line.chars().skip(indent).collect();
            let start_col = weep_lsp::position::column_from_lsp(&line, range.start.character, enc)
                .saturating_sub(indent);
            let usage = crate::usages::Usage {
                hit: start_col..start_col + chars.len(),
                range: chars,
                row: range.start.line as usize,
                text,
            };
            match files.last_mut() {
                Some(f) if f.path == path => f.usages.push(usage),
                _ => files.push(crate::usages::UsageFile {
                    dir: crate::usages::dir_of(&self.root, &path),
                    path,
                    usages: vec![usage],
                }),
            }
        }
        files
    }

    // ---- positions ---------------------------------------------------------------------

    /// The caret (or `offset`) of an editor as the server wants it, when
    /// the document has a ready server.
    fn locate(
        &self,
        editor: &Entity<EditorView>,
        offset: Option<usize>,
        cx: &mut App,
    ) -> Option<Located> {
        let (path, language) = {
            let e = editor.read(cx);
            (e.path(cx)?, e.language(cx)?)
        };
        let server = self.server_for(language)?;
        let s = &self.smart.servers[server];
        if !s.ready || !s.open.contains_key(&path) {
            return None;
        }
        let enc = s.client.encoding();
        let uri = weep_lsp::uri(&path)?;
        let (position, version) = editor.update(cx, |e, cx| {
            let offset = offset.unwrap_or_else(|| e.caret(cx));
            let doc = e.document.read(cx);
            let at = doc.buffer.offset_to_point(offset);
            let line = doc.buffer.line(at.row);
            (
                lsp::Position {
                    line: at.row as u32,
                    character: weep_lsp::column_to_lsp(&line, at.column, enc),
                },
                doc.buffer.version(),
            )
        });
        Some(Located {
            server,
            uri,
            position,
            version,
            path,
        })
    }

    /// A position in a document's current text, as a char offset.
    fn offset_in(doc: &Document, position: lsp::Position, enc: Encoding) -> usize {
        let row = position.line as usize;
        if row >= doc.buffer.line_count() {
            return doc.buffer.len_chars();
        }
        let line = doc.buffer.line(row);
        doc.buffer.line_to_char(row) + weep_lsp::column_from_lsp(&line, position.character, enc)
    }

    fn range_in(doc: &Document, range: lsp::Range, enc: Encoding) -> Range<usize> {
        let start = Self::offset_in(doc, range.start, enc);
        let end = Self::offset_in(doc, range.end, enc).max(start);
        start..end
    }

    fn encoding_of(&self, server: usize) -> Encoding {
        self.smart
            .servers
            .get(server)
            .map(|s| s.client.encoding())
            .unwrap_or_default()
    }

    // ---- the editors' events --------------------------------------------------------

    /// The M7 events of a view: each becomes a request whose answer goes
    /// back to the view.
    pub(super) fn on_smart_editor_event(
        &mut self,
        editor: &Entity<EditorView>,
        event: &EditorEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::CloseInterline => {
                if self.smart.usages.as_ref().is_some_and(|(e, _)| e == editor) {
                    self.close_usages(cx);
                }
            }
            EditorEvent::Completion { explicit } => self.request_completion(editor, *explicit, cx),
            EditorEvent::SignatureHelp => self.request_signature(editor, cx),
            EditorEvent::Hover { offset, mouse } => {
                self.request_hover(editor, Some(*offset), *mouse, cx)
            }
            EditorEvent::GoToDefinition { offset } => {
                self.goto_definition(editor, Some(*offset), false, cx)
            }
            EditorEvent::ProblemFixes(ix) => self.request_problem_fixes(editor, *ix, cx),
            EditorEvent::ApplyProblemFix { problem, fix } => {
                let action = self
                    .smart
                    .fix_actions
                    .iter()
                    .filter(|(p, _)| p == problem)
                    .nth(*fix)
                    .map(|(_, a)| a.clone());
                if let (Some(action), Some(located)) = (action, self.locate(editor, None, cx)) {
                    self.apply_action(located.server, action, cx);
                }
            }
            EditorEvent::ResolveCompletion(ix) => self.resolve_completion(editor, *ix, cx),
            EditorEvent::CodeVision { row, lens } => {
                self.code_vision_clicked(editor, *row, lens, cx)
            }
            EditorEvent::Accepted {
                command: Some(command),
            } => {
                if command.command.ends_with("triggerParameterHints") {
                    self.request_signature(editor, cx);
                } else if let Some(located) = self.locate(editor, None, cx) {
                    self.request::<ExecuteCommand>(
                        located.server,
                        lsp::ExecuteCommandParams {
                            command: command.command.clone(),
                            arguments: command.arguments.clone().unwrap_or_default(),
                            work_done_progress_params: no_progress(),
                        },
                        |_, _, _| {},
                    );
                }
            }
            _ => {}
        }
    }

    fn request_completion(
        &mut self,
        editor: &Entity<EditorView>,
        explicit: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(located) = self.locate(editor, None, cx) else {
            return;
        };
        let enc = self.encoding_of(located.server);
        // The char before the caret, when it is one of the server's triggers.
        let trigger = {
            let e = editor.read(cx);
            let doc = e.document.read(cx);
            let caret = Self::offset_in(doc, located.position, enc);
            caret.checked_sub(1).and_then(|i| doc.buffer.char_at(i))
        }
        .filter(|c| {
            self.smart.servers[located.server]
                .client
                .completion_triggers()
                .iter()
                .any(|t| t.starts_with(*c))
        });
        let context = Some(match (explicit, trigger) {
            (false, Some(c)) => lsp::CompletionContext {
                trigger_kind: lsp::CompletionTriggerKind::TRIGGER_CHARACTER,
                trigger_character: Some(c.to_string()),
            },
            _ => lsp::CompletionContext {
                trigger_kind: lsp::CompletionTriggerKind::INVOKED,
                trigger_character: None,
            },
        });
        let editor = editor.clone();
        let server = located.server;
        let path = located.path.clone();
        let version = located.version;
        self.request::<CompletionRequest>(
            server,
            lsp::CompletionParams {
                text_document_position: lsp::TextDocumentPositionParams {
                    text_document: lsp::TextDocumentIdentifier { uri: located.uri },
                    position: located.position,
                },
                work_done_progress_params: no_progress(),
                partial_result_params: Default::default(),
                context,
            },
            move |this, result, cx| {
                let Ok(value) = result else { return };
                let Ok(answer) = serde_json::from_value::<Option<lsp::CompletionResponse>>(value)
                else {
                    return;
                };
                let (items, is_incomplete) = match answer {
                    Some(lsp::CompletionResponse::Array(items)) => (items, false),
                    Some(lsp::CompletionResponse::List(list)) => (list.items, list.is_incomplete),
                    None => (Vec::new(), false),
                };
                let Some(s) = this.smart.servers.get(server) else {
                    return;
                };
                let enc = s.client.encoding();
                let Some(synced) = s.open.get(&path) else {
                    return;
                };
                if synced.buffer_version != version {
                    // Typed on since: the view asks again on its own.
                }
                let index = LineIndex::new(&synced.text);
                let candidates: Vec<Candidate> = items
                    .iter()
                    .map(|item| {
                        let replace = match &item.text_edit {
                            Some(lsp::CompletionTextEdit::Edit(e)) => {
                                Some(index.range(e.range, enc))
                            }
                            Some(lsp::CompletionTextEdit::InsertAndReplace(e)) => {
                                Some(index.range(e.replace, enc))
                            }
                            None => None,
                        };
                        let additional = item
                            .additional_text_edits
                            .as_ref()
                            .map(|edits| weep_lsp::changes(&synced.text, edits, enc))
                            .unwrap_or_default();
                        Candidate {
                            item: item.clone(),
                            replace,
                            additional,
                        }
                    })
                    .collect();
                this.smart.completion_items = items;
                this.smart.completion_server = Some(server);
                editor.update(cx, |e, cx| {
                    e.show_completions(candidates, version, is_incomplete, explicit, cx)
                });
            },
        );
    }

    fn resolve_completion(
        &mut self,
        editor: &Entity<EditorView>,
        ix: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.smart.completion_items.get(ix).cloned() else {
            return;
        };
        let Some(server) = self.smart.completion_server else {
            return;
        };
        let editor = editor.clone();
        let finish = move |item: lsp::CompletionItem,
                           editor: &Entity<EditorView>,
                           cx: &mut Context<Self>| {
            let mut md = String::new();
            if let Some(detail) = &item.detail {
                md.push_str("```\n");
                md.push_str(detail);
                md.push_str("\n```\n\n");
            }
            if let Some(doc) = &item.documentation {
                md.push_str(&weep_lsp::markup::documentation_markdown(doc));
            }
            let lines = weep_lsp::markup::lines(&md);
            editor.update(cx, |e, cx| e.set_completion_docs(ix, lines, cx));
        };
        if item.documentation.is_some() || item.data.is_none() {
            finish(item, &editor, cx);
            return;
        }
        self.request::<ResolveCompletionItem>(server, item.clone(), move |_, result, cx| {
            let resolved = result
                .ok()
                .and_then(|v| serde_json::from_value::<lsp::CompletionItem>(v).ok())
                .unwrap_or(item);
            finish(resolved, &editor, cx);
        });
    }

    fn request_signature(&mut self, editor: &Entity<EditorView>, cx: &mut Context<Self>) {
        let Some(located) = self.locate(editor, None, cx) else {
            return;
        };
        let editor = editor.clone();
        self.request::<SignatureHelpRequest>(
            located.server,
            lsp::SignatureHelpParams {
                context: None,
                text_document_position_params: lsp::TextDocumentPositionParams {
                    text_document: lsp::TextDocumentIdentifier { uri: located.uri },
                    position: located.position,
                },
                work_done_progress_params: no_progress(),
            },
            move |_, result, cx| {
                let help = result
                    .ok()
                    .and_then(|v| serde_json::from_value::<Option<lsp::SignatureHelp>>(v).ok())
                    .flatten();
                editor.update(cx, |e, cx| e.show_signature(help, cx));
            },
        );
    }

    fn request_hover(
        &mut self,
        editor: &Entity<EditorView>,
        offset: Option<usize>,
        mouse: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(located) = self.locate(editor, offset, cx) else {
            return;
        };
        let anchor = offset.unwrap_or_else(|| editor.update(cx, |e, cx| e.caret(cx)));
        let editor = editor.clone();
        let server = located.server;
        self.request::<HoverRequest>(
            server,
            lsp::HoverParams {
                text_document_position_params: lsp::TextDocumentPositionParams {
                    text_document: lsp::TextDocumentIdentifier { uri: located.uri },
                    position: located.position,
                },
                work_done_progress_params: no_progress(),
            },
            move |this, result, cx| {
                let hover = result
                    .ok()
                    .and_then(|v| serde_json::from_value::<Option<lsp::Hover>>(v).ok())
                    .flatten();
                let enc = this.encoding_of(server);
                let (lines, range) = match hover {
                    Some(h) => {
                        let lines = weep_lsp::markup::lines(&weep_lsp::markup::hover_markdown(&h));
                        let range = h.range.map(|r| {
                            let e = editor.read(cx);
                            Self::range_in(e.document.read(cx), r, enc)
                        });
                        (lines, range)
                    }
                    None => (Vec::new(), None),
                };
                editor.update(cx, |e, cx| e.show_hover(lines, anchor, range, mouse, cx));
            },
        );
    }

    // ---- navigation -----------------------------------------------------------------

    fn current_position(&self, cx: &mut App) -> Option<(PathBuf, usize)> {
        let editor = self.active_editor()?.clone();
        editor.update(cx, |e, cx| e.path(cx).map(|p| (p, e.caret(cx))))
    }

    /// Before a jump: the present becomes the newest history entry.
    fn record_history(&mut self, cx: &mut App) {
        if let Some(position) = self.current_position(cx) {
            let keep = self.smart.history_ix.min(self.smart.history.len());
            self.smart.history.truncate(keep);
            if self.smart.history.last() != Some(&position) {
                self.smart.history.push(position);
            }
            self.smart.history_ix = self.smart.history.len();
        }
    }

    fn goto_position(&mut self, path: PathBuf, range: Range<usize>, cx: &mut Context<Self>) {
        self.open_path(path, false, cx);
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |e, cx| e.select_range(range, cx));
        }
        self.focus_active_editor(cx);
    }

    pub(super) fn back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        if self.smart.history.is_empty() || self.smart.history_ix == 0 {
            return;
        }
        if self.smart.history_ix >= self.smart.history.len() {
            // Leaving the present: remember it so `forward` returns.
            if let Some(position) = self.current_position(cx) {
                if self.smart.history.last() != Some(&position) {
                    self.smart.history.push(position);
                }
            }
            self.smart.history_ix = self.smart.history.len() - 1;
            if self.smart.history_ix == 0 {
                return;
            }
        }
        self.smart.history_ix -= 1;
        let (path, offset) = self.smart.history[self.smart.history_ix].clone();
        self.goto_position(path, offset..offset, cx);
    }

    pub(super) fn forward(&mut self, _: &Forward, _: &mut Window, cx: &mut Context<Self>) {
        if self.smart.history_ix + 1 >= self.smart.history.len() {
            return;
        }
        self.smart.history_ix += 1;
        let (path, offset) = self.smart.history[self.smart.history_ix].clone();
        self.goto_position(path, offset..offset, cx);
    }

    /// `go-to-declaration` and `go-to-type-declaration`: one target opens,
    /// several list in a popup.
    fn goto_definition(
        &mut self,
        editor: &Entity<EditorView>,
        offset: Option<usize>,
        type_def: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(located) = self.locate(editor, offset, cx) else {
            return;
        };
        let params = lsp::GotoDefinitionParams {
            text_document_position_params: lsp::TextDocumentPositionParams {
                text_document: lsp::TextDocumentIdentifier { uri: located.uri },
                position: located.position,
            },
            work_done_progress_params: no_progress(),
            partial_result_params: Default::default(),
        };
        let server = located.server;
        let title = if type_def {
            "Type Declaration"
        } else {
            "Declaration"
        };
        let reply = move |this: &mut Workspace,
                          result: Result<Value, ResponseError>,
                          cx: &mut Context<Workspace>| {
            let answer = result
                .ok()
                .and_then(|v| serde_json::from_value::<Option<lsp::GotoDefinitionResponse>>(v).ok())
                .flatten();
            let targets: Vec<(lsp::Url, lsp::Range)> = match answer {
                Some(lsp::GotoDefinitionResponse::Scalar(l)) => vec![(l.uri, l.range)],
                Some(lsp::GotoDefinitionResponse::Array(ls)) => {
                    ls.into_iter().map(|l| (l.uri, l.range)).collect()
                }
                Some(lsp::GotoDefinitionResponse::Link(links)) => links
                    .into_iter()
                    .map(|l| (l.target_uri, l.target_selection_range))
                    .collect(),
                None => Vec::new(),
            };
            let enc = this.encoding_of(server);
            this.open_targets(title, targets, enc, cx);
        };
        if type_def {
            self.request::<GotoTypeDefinition>(server, params, reply);
        } else {
            self.request::<GotoDefinition>(server, params, reply);
        }
    }

    /// A target's range as char offsets: through the open document, else
    /// the file on disk.
    /// Line `row` of `path` as it stands (an open document's, else the file's).
    fn line_text(&self, path: &Path, row: usize, cx: &App) -> String {
        if let Some(doc) = self.document_at(path, cx) {
            return doc.read(cx).buffer.line(row);
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        LineIndex::new(&text).line(row).unwrap_or("").to_string()
    }

    fn target_range(
        &self,
        path: &Path,
        range: lsp::Range,
        enc: Encoding,
        cx: &App,
    ) -> (Range<usize>, String) {
        if let Some(doc) = self.document_at(path, cx) {
            let d = doc.read(cx);
            let r = Self::range_in(d, range, enc);
            let line = d.buffer.line(range.start.line as usize);
            return (r, line.trim().to_string());
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let index = LineIndex::new(&text);
        let line = index
            .line(range.start.line as usize)
            .unwrap_or("")
            .trim()
            .to_string();
        (index.range(range, enc), line)
    }

    /// Opens the one target, or lists them (M7 estimate: a popup in the
    /// Goto frame).
    fn open_targets(
        &mut self,
        title: &str,
        targets: Vec<(lsp::Url, lsp::Range)>,
        enc: Encoding,
        cx: &mut Context<Self>,
    ) {
        let targets: Vec<(PathBuf, lsp::Range)> = targets
            .into_iter()
            .filter_map(|(uri, range)| weep_lsp::path(&uri).map(|p| (p, range)))
            .collect();
        if targets.is_empty() {
            return;
        }
        if targets.len() == 1 {
            let (path, range) = targets.into_iter().next().expect("one");
            let (chars, _) = self.target_range(&path, range, enc, cx);
            self.record_history(cx);
            self.goto_position(path, chars, cx);
            return;
        }
        let rows: Vec<(ListRow, (PathBuf, Range<usize>))> = targets
            .into_iter()
            .map(|(path, range)| {
                let (chars, line) = self.target_range(&path, range, enc, cx);
                let rel = relative_display(&self.root, &path);
                (
                    ListRow {
                        icon: Some(icon_for(&path)),
                        text: line.into(),
                        detail: None,
                        right: Some(format!("{rel}:{}", range.start.line + 1).into()),
                    },
                    (path, chars),
                )
            })
            .collect();
        self.show_list(title.to_string(), rows, cx);
    }

    /// Fleet's inline usages: the panel laid under `row` in `editor`, focused.
    fn show_usages(
        &mut self,
        editor: &Entity<EditorView>,
        row: usize,
        title: String,
        files: Vec<crate::usages::UsageFile>,
        cx: &mut Context<Self>,
    ) {
        self.close_usages(cx);
        let open_document = self.open_document_fn(cx);
        let panel = cx.new(|cx| crate::usages::UsagesPanel::new(title, files, open_document, cx));
        cx.subscribe(
            &panel,
            |this, panel, event: &crate::usages::UsagesEvent, cx| {
                if this.smart.usages.as_ref().map(|(_, p)| p) != Some(&panel) {
                    return;
                }
                match event {
                    crate::usages::UsagesEvent::Close => {
                        this.close_usages(cx);
                        this.focus_active_editor(cx);
                    }
                    crate::usages::UsagesEvent::Open(path, range) => {
                        let (path, range) = (path.clone(), range.clone());
                        this.close_usages(cx);
                        this.record_history(cx);
                        this.goto_position(path, range, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
        let view: gpui::AnyView = panel.clone().into();
        editor.update(cx, |e, cx| {
            e.set_interline(Some((row, crate::usages::block_height, view)), cx)
        });
        let handle = panel.read(cx).focus_handle(cx);
        self.smart.usages = Some((editor.clone(), panel));
        self.focus_later(handle, cx);
        cx.notify();
    }

    /// Takes the inline usages out of their editor.
    pub(super) fn close_usages(&mut self, cx: &mut Context<Self>) {
        if let Some((editor, _)) = self.smart.usages.take() {
            editor.update(cx, |e, cx| e.set_interline(None, cx));
        }
    }

    fn show_list(
        &mut self,
        title: String,
        rows: Vec<(ListRow, (PathBuf, Range<usize>))>,
        cx: &mut Context<Self>,
    ) {
        let (list_rows, targets): (Vec<ListRow>, Vec<(PathBuf, Range<usize>)>) =
            rows.into_iter().unzip();
        let popup = cx.new(|cx| ListPopup::new(title, list_rows, cx));
        cx.subscribe(&popup, |this, popup, event: &ListPopupEvent, cx| {
            if this.smart.list.as_ref() != Some(&popup) {
                return;
            }
            match event {
                ListPopupEvent::Open(ix) => {
                    let target = this.smart.list_targets.get(*ix).cloned();
                    this.smart.list = None;
                    if let Some((path, range)) = target {
                        this.record_history(cx);
                        this.goto_position(path, range, cx);
                    } else {
                        this.focus_active_editor(cx);
                    }
                }
                ListPopupEvent::Cancel => {
                    this.smart.list = None;
                    this.focus_active_editor(cx);
                }
            }
            cx.notify();
        })
        .detach();
        let handle = popup.read(cx).focus_handle(cx);
        self.smart.list = Some(popup);
        self.smart.list_targets = targets;
        self.focus_later(handle, cx);
        cx.notify();
    }

    pub(super) fn smart_overlays(
        &self,
        theme: &Theme,
        m: &Metrics,
        ws: Entity<Workspace>,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(list) = &self.smart.list {
            out.push(list.clone().into_any_element());
        }
        out.extend(self.render_smart_card(theme, m, ws));
        out
    }

    // ---- edits ----------------------------------------------------------------------

    /// Applies a workspace edit: open documents through their editor in
    /// one undo step each, other files on disk.
    pub(super) fn apply_workspace_edit(
        &mut self,
        edit: &lsp::WorkspaceEdit,
        enc: Encoding,
        cx: &mut Context<Self>,
    ) {
        for file in weep_lsp::workspace_edit_files(edit) {
            let Some(path) = weep_lsp::path(&file.uri) else {
                continue;
            };
            if let Some(editor) = self.editor_for_path(&path, cx) {
                let text = editor.read(cx).document.read(cx).buffer.text();
                let changes = weep_lsp::changes(&text, &file.edits, enc);
                let edits: Vec<(Range<usize>, String)> =
                    changes.into_iter().map(|c| (c.range, c.text)).collect();
                editor.update(cx, |e, cx| e.replace_ranges(edits, cx));
            } else if let Some(doc) = self.document_at(&path, cx) {
                let text = doc.read(cx).buffer.text();
                let new_text = weep_lsp::apply(&text, &weep_lsp::changes(&text, &file.edits, enc));
                doc.update(cx, |d, cx| d.edit(cx, |b| b.set_text(&new_text)));
            } else {
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        let new_text =
                            weep_lsp::apply(&text, &weep_lsp::changes(&text, &file.edits, enc));
                        if let Err(e) = std::fs::write(&path, new_text) {
                            self.notify_error(
                                "Cannot write file",
                                format!("{}: {e}", path.display()),
                                cx,
                            );
                        }
                    }
                    Err(e) => self.notify_error(
                        "Cannot read file",
                        format!("{}: {e}", path.display()),
                        cx,
                    ),
                }
            }
        }
    }

    fn apply_action(
        &mut self,
        server: usize,
        action: lsp::CodeActionOrCommand,
        cx: &mut Context<Self>,
    ) {
        let enc = self.encoding_of(server);
        match action {
            lsp::CodeActionOrCommand::Command(command) => self.run_command(server, command),
            lsp::CodeActionOrCommand::CodeAction(action) => {
                if let Some(edit) = &action.edit {
                    self.apply_workspace_edit(edit, enc, cx);
                    if let Some(command) = action.command {
                        self.run_command(server, command);
                    }
                } else if action.data.is_some() {
                    self.request::<CodeActionResolveRequest>(
                        server,
                        action,
                        move |this, result, cx| {
                            if let Ok(resolved) = result.and_then(|v| {
                                serde_json::from_value::<lsp::CodeAction>(v)
                                    .map_err(|e| ResponseError::new(0, e.to_string()))
                            }) {
                                if let Some(edit) = &resolved.edit {
                                    this.apply_workspace_edit(edit, enc, cx);
                                }
                                if let Some(command) = resolved.command {
                                    this.run_command(server, command);
                                }
                            }
                        },
                    );
                } else if let Some(command) = action.command {
                    self.run_command(server, command);
                }
            }
        }
    }

    fn run_command(&mut self, server: usize, command: lsp::Command) {
        self.request::<ExecuteCommand>(
            server,
            lsp::ExecuteCommandParams {
                command: command.command,
                arguments: command.arguments.unwrap_or_default(),
                work_done_progress_params: no_progress(),
            },
            |_, _, _| {},
        );
    }

    /// Code actions for a range of the active editor, with the problems
    /// there as the context.
    fn request_code_actions(
        &mut self,
        editor: &Entity<EditorView>,
        range: Range<usize>,
        only: Option<Vec<lsp::CodeActionKind>>,
        reply: impl FnOnce(&mut Workspace, usize, Vec<lsp::CodeActionOrCommand>, &mut Context<Workspace>)
            + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(located) = self.locate(editor, Some(range.start), cx) else {
            return;
        };
        let enc = self.encoding_of(located.server);
        let (end, diagnostics) = editor.update(cx, |e, cx| {
            let problems = e.problems(cx);
            let doc = e.document.read(cx);
            let at = doc.buffer.offset_to_point(range.end);
            let line = doc.buffer.line(at.row);
            let end = lsp::Position {
                line: at.row as u32,
                character: weep_lsp::column_to_lsp(&line, at.column, enc),
            };
            let diagnostics: Vec<lsp::Diagnostic> = problems
                .iter()
                .filter(|p| {
                    p.range.start <= range.end && range.start <= p.range.end.max(p.range.start + 1)
                })
                .filter_map(|p| p.diagnostic.clone())
                .collect();
            (end, diagnostics)
        });
        let server = located.server;
        self.request::<CodeActionRequest>(
            server,
            lsp::CodeActionParams {
                text_document: lsp::TextDocumentIdentifier { uri: located.uri },
                range: lsp::Range {
                    start: located.position,
                    end,
                },
                context: lsp::CodeActionContext {
                    diagnostics,
                    only,
                    trigger_kind: Some(lsp::CodeActionTriggerKind::INVOKED),
                },
                work_done_progress_params: no_progress(),
                partial_result_params: Default::default(),
            },
            move |this, result, cx| {
                let actions = result
                    .ok()
                    .and_then(|v| serde_json::from_value::<Option<lsp::CodeActionResponse>>(v).ok())
                    .flatten()
                    .unwrap_or_default();
                reply(this, server, actions, cx);
            },
        );
    }

    fn request_problem_fixes(
        &mut self,
        editor: &Entity<EditorView>,
        ix: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(range) =
            editor.update(cx, |e, cx| e.problems(cx).get(ix).map(|p| p.range.clone()))
        else {
            return;
        };
        let target = editor.clone();
        self.request_code_actions(
            editor,
            range,
            Some(vec![lsp::CodeActionKind::QUICKFIX]),
            move |this, _, actions, cx| {
                let editor = target;
                this.smart.fix_actions.retain(|(p, _)| *p != ix);
                let titles: Vec<String> = actions
                    .iter()
                    .filter(|a| match a {
                        lsp::CodeActionOrCommand::CodeAction(a) => a.disabled.is_none(),
                        lsp::CodeActionOrCommand::Command(_) => true,
                    })
                    .map(|a| match a {
                        lsp::CodeActionOrCommand::CodeAction(a) => a.title.clone(),
                        lsp::CodeActionOrCommand::Command(c) => c.title.clone(),
                    })
                    .collect();
                this.smart.fix_actions.extend(
                    actions
                        .into_iter()
                        .filter(|a| match a {
                            lsp::CodeActionOrCommand::CodeAction(a) => a.disabled.is_none(),
                            lsp::CodeActionOrCommand::Command(_) => true,
                        })
                        .map(|a| (ix, a)),
                );
                editor.update(cx, |e, cx| e.set_fixes(ix, titles, cx));
            },
            cx,
        );
    }

    // ---- actions ---------------------------------------------------------------------

    pub(super) fn completion(&mut self, _: &Completion, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            self.request_completion(&editor, true, cx);
        }
    }

    /// `smart-completion` asks the same way (Fleet's type-aware list is
    /// the backend's; an estimate).
    pub(super) fn smart_completion(
        &mut self,
        _: &SmartCompletion,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.active_editor().cloned() {
            self.request_completion(&editor, true, cx);
        }
    }

    pub(super) fn show_parameter_info(
        &mut self,
        _: &ShowParameterInfo,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.active_editor().cloned() {
            self.request_signature(&editor, cx);
        }
    }

    /// With the completion popup open: its documentation pane; else the
    /// documentation of the symbol at the caret.
    pub(super) fn show_documentation(
        &mut self,
        _: &ShowDocumentation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor().cloned() else {
            return;
        };
        if editor.update(cx, |e, cx| e.toggle_completion_docs(cx)) {
            return;
        }
        self.request_hover(&editor, None, false, cx);
    }

    pub(super) fn go_to_declaration(
        &mut self,
        _: &GoToDeclaration,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.active_editor().cloned() {
            self.goto_definition(&editor, None, false, cx);
        }
    }

    pub(super) fn go_to_type_declaration(
        &mut self,
        _: &GoToTypeDeclaration,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.active_editor().cloned() {
            self.goto_definition(&editor, None, true, cx);
        }
    }

    pub(super) fn find_usages(&mut self, _: &FindUsages, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else {
            return;
        };
        let Some(located) = self.locate(&editor, None, cx) else {
            return;
        };
        let (word, caret_row) = editor.update(cx, |e, cx| {
            let caret = e.caret(cx);
            let doc = e.document.read(cx);
            let range = e.cursor.word_range_at(&doc.buffer, caret);
            (
                doc.buffer.slice(range),
                doc.buffer.offset_to_point(caret).row,
            )
        });
        let server = located.server;
        self.request::<References>(
            server,
            lsp::ReferenceParams {
                text_document_position: lsp::TextDocumentPositionParams {
                    text_document: lsp::TextDocumentIdentifier { uri: located.uri },
                    position: located.position,
                },
                work_done_progress_params: no_progress(),
                partial_result_params: Default::default(),
                // Fleet lists the usages without the declaration ("6 in 1
                // file" for a struct used six times, fifteenth pass).
                context: lsp::ReferenceContext {
                    include_declaration: false,
                },
            },
            move |this, result, cx| {
                let locations = result
                    .ok()
                    .and_then(|v| serde_json::from_value::<Option<Vec<lsp::Location>>>(v).ok())
                    .flatten()
                    .unwrap_or_default();
                let enc = this.encoding_of(server);
                let title = if word.is_empty() {
                    "Usages".to_string()
                } else {
                    format!("Usages of {word}")
                };
                if locations.is_empty() {
                    this.notify(Level::Info, title, Some("No usages found".into()), cx);
                    return;
                }
                let files = this.usage_files(locations, enc, cx);
                this.show_usages(&editor, caret_row, "Usages".into(), files, cx);
            },
        );
    }

    /// `rename` (F2 in an editor): the symbol's name in a prompt, preselected
    /// (`rename.selectOnRename`), then the server's edit everywhere.
    pub(super) fn rename_symbol(
        &mut self,
        _: &RenameSymbol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor().cloned() else {
            return;
        };
        if !self.smart.enabled {
            // Without a server, F2 is the file's rename, as in the tree.
            self.rename_file(&RenameFile, window, cx);
            return;
        }
        let Some(located) = self.locate(&editor, None, cx) else {
            self.rename_file(&RenameFile, window, cx);
            return;
        };
        let server = located.server;
        let path = located.path.clone();
        let uri = located.uri.clone();
        let position = located.position;
        self.request::<PrepareRenameRequest>(
            server,
            lsp::TextDocumentPositionParams {
                text_document: lsp::TextDocumentIdentifier { uri: uri.clone() },
                position,
            },
            move |this, result, cx| {
                let answer = result
                    .ok()
                    .and_then(|v| {
                        serde_json::from_value::<Option<lsp::PrepareRenameResponse>>(v).ok()
                    })
                    .flatten();
                let enc = this.encoding_of(server);
                let Some(editor) = this.editor_for_path(&path, cx) else {
                    return;
                };
                let current = match answer {
                    Some(lsp::PrepareRenameResponse::Range(range)) => editor.update(cx, |e, cx| {
                        let doc = e.document.read(cx);
                        doc.buffer.slice(Self::range_in(doc, range, enc))
                    }),
                    Some(lsp::PrepareRenameResponse::RangeWithPlaceholder {
                        placeholder, ..
                    }) => placeholder,
                    Some(lsp::PrepareRenameResponse::DefaultBehavior { .. }) => {
                        editor.update(cx, |e, cx| {
                            let caret = e.caret(cx);
                            let doc = e.document.read(cx);
                            let range = e.cursor.word_range_at(&doc.buffer, caret);
                            doc.buffer.slice(range)
                        })
                    }
                    None => {
                        this.notify(
                            Level::Info,
                            "Rename",
                            Some("Nothing to rename here".into()),
                            cx,
                        );
                        return;
                    }
                };
                this.open_rename_prompt(server, uri, position, current, cx);
            },
        );
    }

    fn open_rename_prompt(
        &mut self,
        server: usize,
        uri: lsp::Url,
        position: lsp::Position,
        current: String,
        cx: &mut Context<Self>,
    ) {
        let select = Self::settings(cx).rename_select_on_rename();
        let prompt = cx.new(|cx| Prompt::new(PromptKind::RenameSymbol, &current, Vec::new(), cx));
        if select {
            prompt.update(cx, |p, cx| p.select_all(cx));
        }
        cx.subscribe(&prompt, move |this, prompt, event: &PromptEvent, cx| {
            if this.prompt.as_ref() != Some(&prompt) {
                return;
            }
            match event {
                PromptEvent::Confirm { text, .. } => {
                    this.prompt = None;
                    let new_name = text.trim().to_string();
                    this.focus_active_editor(cx);
                    if new_name.is_empty() {
                        return;
                    }
                    this.request::<Rename>(
                        server,
                        lsp::RenameParams {
                            text_document_position: lsp::TextDocumentPositionParams {
                                text_document: lsp::TextDocumentIdentifier { uri: uri.clone() },
                                position,
                            },
                            new_name,
                            work_done_progress_params: no_progress(),
                        },
                        move |this, result, cx| match result {
                            Ok(v) => {
                                if let Ok(Some(edit)) =
                                    serde_json::from_value::<Option<lsp::WorkspaceEdit>>(v)
                                {
                                    let enc = this.encoding_of(server);
                                    this.apply_workspace_edit(&edit, enc, cx);
                                }
                            }
                            Err(e) => this.notify_error("Rename", e.message, cx),
                        },
                    );
                }
                PromptEvent::Cancel => {
                    this.prompt = None;
                    this.focus_active_editor(cx);
                }
            }
            cx.notify();
        })
        .detach();
        let handle = prompt.read(cx).focus_handle(cx);
        self.prompt = Some(prompt);
        self.focus_later(handle, cx);
        cx.notify();
    }

    /// `quick-fixes` (Alt+Enter): the actions at the caret as a menu there.
    pub(super) fn quick_fixes(
        &mut self,
        _: &QuickFixes,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor().cloned() else {
            return;
        };
        let range = editor.update(cx, |e, cx| e.selection_range(cx));
        let at = editor.update(cx, |e, cx| e.caret_window_point(window, cx));
        let focus = editor.read(cx).focus_handle(cx);
        self.request_code_actions(
            &editor,
            range,
            None,
            move |this, server, actions, cx| {
                if actions.is_empty() {
                    this.notify(
                        Level::Info,
                        "Quick Fixes",
                        Some("No fixes available".into()),
                        cx,
                    );
                    return;
                }
                let ws = cx.entity();
                let entries: Vec<MenuEntry> = actions
                    .into_iter()
                    .map(|action| match &action {
                        lsp::CodeActionOrCommand::CodeAction(a) if a.disabled.is_some() => {
                            MenuEntry::disabled(a.title.clone())
                        }
                        _ => {
                            let title = match &action {
                                lsp::CodeActionOrCommand::CodeAction(a) => a.title.clone(),
                                lsp::CodeActionOrCommand::Command(c) => c.title.clone(),
                            };
                            let ws = ws.clone();
                            MenuEntry::custom(title, move |_, cx| {
                                let action = action.clone();
                                ws.update(cx, |this, cx| this.apply_action(server, action, cx));
                            })
                        }
                    })
                    .collect();
                this.open_menu(entries, at, Some(focus), cx);
            },
            cx,
        );
    }

    /// `apply-primary-quick-fix`: the preferred action, else the first quick fix.
    pub(super) fn apply_primary_quick_fix(
        &mut self,
        _: &ApplyPrimaryQuickFix,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor().cloned() else {
            return;
        };
        let range = editor.update(cx, |e, cx| e.selection_range(cx));
        self.request_code_actions(
            &editor,
            range,
            None,
            |this, server, actions, cx| {
                let enabled: Vec<lsp::CodeActionOrCommand> = actions
                    .into_iter()
                    .filter(|a| match a {
                        lsp::CodeActionOrCommand::CodeAction(a) => a.disabled.is_none(),
                        lsp::CodeActionOrCommand::Command(_) => true,
                    })
                    .collect();
                let preferred = enabled.iter().position(|a| {
                    matches!(a, lsp::CodeActionOrCommand::CodeAction(a) if a.is_preferred == Some(true))
                });
                let quickfix = enabled.iter().position(|a| {
                    matches!(a, lsp::CodeActionOrCommand::CodeAction(a) if a.kind.as_ref().is_some_and(|k| k.as_str().starts_with("quickfix")))
                });
                match preferred.or(quickfix).or((!enabled.is_empty()).then_some(0)) {
                    Some(ix) => {
                        let action = enabled[ix].clone();
                        this.apply_action(server, action, cx);
                    }
                    None => this.notify(Level::Info, "Quick Fixes", Some("No fixes available".into()), cx),
                }
            },
            cx,
        );
    }

    /// `optimize-imports`: the `source.organizeImports` action.
    pub(super) fn optimize_imports(
        &mut self,
        _: &OptimizeImports,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor().cloned() else {
            return;
        };
        let range = editor.update(cx, |e, cx| e.selection_range(cx));
        self.request_code_actions(
            &editor,
            range,
            Some(vec![lsp::CodeActionKind::SOURCE_ORGANIZE_IMPORTS]),
            |this, server, actions, cx| match actions.into_iter().next() {
                Some(action) => this.apply_action(server, action, cx),
                None => this.notify(
                    Level::Info,
                    "Optimize Imports",
                    Some("Nothing to do".into()),
                    cx,
                ),
            },
            cx,
        );
    }

    /// `reformat-code`: the server's formatting of the whole file, in one
    /// undo step.
    pub(super) fn reformat_code(
        &mut self,
        _: &ReformatCode,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor().cloned() else {
            return;
        };
        self.format_editor(&editor, |_, _| {}, cx);
    }

    /// Asks the server to format the editor's file and applies the edits;
    /// `then` runs after (also when the server had nothing to say).
    fn format_editor(
        &mut self,
        editor: &Entity<EditorView>,
        then: impl FnOnce(&mut Workspace, &mut Context<Workspace>) + 'static,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(located) = self.locate(editor, None, cx) else {
            return false;
        };
        let (tab_size, insert_spaces) = {
            let indent = editor.read(cx).document.read(cx).buffer.indent;
            (indent.width as u32, !indent.use_tabs)
        };
        let editor = editor.clone();
        let server = located.server;
        self.request::<Formatting>(
            server,
            lsp::DocumentFormattingParams {
                text_document: lsp::TextDocumentIdentifier { uri: located.uri },
                options: lsp::FormattingOptions {
                    tab_size,
                    insert_spaces,
                    properties: HashMap::new(),
                    trim_trailing_whitespace: None,
                    insert_final_newline: None,
                    trim_final_newlines: None,
                },
                work_done_progress_params: no_progress(),
            },
            move |this, result, cx| {
                let edits = result
                    .ok()
                    .and_then(|v| serde_json::from_value::<Option<Vec<lsp::TextEdit>>>(v).ok())
                    .flatten()
                    .unwrap_or_default();
                if !edits.is_empty() {
                    let enc = this.encoding_of(server);
                    let text = editor.read(cx).document.read(cx).buffer.text();
                    let changes = weep_lsp::changes(&text, &edits, enc);
                    let edits: Vec<(Range<usize>, String)> =
                        changes.into_iter().map(|c| (c.range, c.text)).collect();
                    editor.update(cx, |e, cx| e.replace_ranges(edits, cx));
                }
                then(this, cx);
            },
        );
        true
    }

    /// `save-file` through Smart Mode: `editor.formatOnSave` formats first
    /// when the file has a server, then the document writes itself.
    pub(super) fn save_editor(&mut self, editor: &Entity<EditorView>, cx: &mut Context<Self>) {
        if Self::settings(cx).editor_format_on_save() {
            let e = editor.clone();
            let started = self.format_editor(editor, move |this, cx| this.write_editor(&e, cx), cx);
            if started {
                return;
            }
        }
        self.write_editor(editor, cx);
    }

    fn write_editor(&mut self, editor: &Entity<EditorView>, cx: &mut Context<Self>) {
        if let Err(e) = editor.update(cx, |e, cx| e.save(cx)) {
            self.notify_error("Save failed", format!("{e}"), cx);
        }
    }
}

/// The Smart Mode card in the tui skin (`docs/tui-skin.md`, rule 7): a box
/// under the toolbar's right end, its state set into the top edge, the
/// text wrapped on the grid, the button right-aligned, and the servers'
/// errors under a rule joining the frame.
struct TuiCard {
    title: &'static str,
    text: &'static str,
    errors: Vec<SharedString>,
    label: SharedString,
    ws: Entity<Workspace>,
    focus: FocusHandle,
}

impl TuiCard {
    fn render(self, theme: &Theme, m: Metrics, window: &mut Window, cx: &mut App) -> AnyElement {
        use crate::skin::{self, Cell};
        let cell = Cell::current(cx);
        let all_cols = Cell::fit(window.viewport_size().width, cell.w);
        let cols = ((m.notifications_width / cell.w).round() as usize)
            .clamp(24, all_cols.saturating_sub(2).max(24));
        let inner = cols - 4;
        let text = skin::wrap(self.text, inner);
        let errors: Vec<String> = self
            .errors
            .iter()
            .flat_map(|e| {
                skin::wrap(e, inner.saturating_sub(2))
                    .into_iter()
                    .enumerate()
                    .map(|(i, l)| format!("{}{l}", if i == 0 { "✗ " } else { "  " }))
            })
            .collect();
        let rule = 1 + text.len() + 2;
        let rows = rule
            + 1
            + if errors.is_empty() {
                0
            } else {
                errors.len() + 1
            };
        let ws = self.ws.clone();
        let line = |t: String, color| {
            div()
                .h(px(cell.h))
                .flex_shrink_0()
                .pl(cell.cols(1))
                .whitespace_nowrap()
                .text_color(color)
                .child(SharedString::from(t))
        };
        let body = div()
            .flex()
            .flex_col()
            .children(text.into_iter().map(|t| line(t, theme.text_secondary)))
            .child(div().h(px(cell.h)).flex_shrink_0())
            .child(
                div()
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    .pr(cell.cols(1))
                    .child(
                        skin::button("smart-card-button", &self.label, false, theme, cell)
                            .on_click(move |_, _, cx| {
                                ws.update(cx, |this, cx| {
                                    this.toggle_smart_mode(cx);
                                    this.smart.card_open = this.smart.enabled;
                                    cx.notify();
                                })
                            }),
                    ),
            )
            .when(!errors.is_empty(), |d| {
                d.child(div().h(px(cell.h)).flex_shrink_0())
                    .children(errors.into_iter().map(|e| {
                        let (mark, rest) =
                            e.split_at(e.char_indices().nth(2).map_or(e.len(), |(b, _)| b));
                        div()
                            .h(px(cell.h))
                            .flex_shrink_0()
                            .flex()
                            .pl(cell.cols(1))
                            .whitespace_nowrap()
                            .child(
                                div()
                                    .text_color(theme.text_dangerous)
                                    .child(SharedString::from(mark.to_string())),
                            )
                            .child(SharedString::from(rest.to_string()))
                    }))
            })
            .into_any_element();
        let close = self.ws.clone();
        let escape = self.ws.clone();
        let top = ((m.toolbar_height + m.notifications_top_gap) / cell.h).ceil() as usize;
        deferred(
            div()
                .id("smart-card")
                .absolute()
                .left(cell.cols(all_cols.saturating_sub(cols + 1)))
                .top(cell.rows(top))
                .track_focus(&self.focus)
                .key_context("Popup")
                .on_action(move |_: &PopupCancel, _, cx| {
                    escape.update(cx, |this, cx| {
                        this.smart.card_open = false;
                        this.focus_active_editor(cx);
                        cx.notify();
                    })
                })
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |this, cx| {
                        this.smart.card_open = false;
                        this.focus_active_editor(cx);
                        cx.notify();
                    })
                })
                .child(
                    skin::boxed(
                        cell,
                        cols,
                        rows,
                        vec![skin::title(self.title, theme.text_primary)],
                        if rows > rule + 1 {
                            vec![rule]
                        } else {
                            Vec::new()
                        },
                        body,
                        theme.popup_bg,
                        theme.border_focused,
                        theme,
                        m,
                    )
                    .occlude(),
                ),
        )
        .into_any_element()
    }
}
