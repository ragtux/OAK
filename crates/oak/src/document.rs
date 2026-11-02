//! One open file: its buffer, language and highlight spans, shared by every
//! editor view that shows it (a tab in each split). Views keep their own
//! cursor and scroll; edits made through any of them land here, and every
//! view observes the document to repaint.

use gpui::{Context, EventEmitter};
use oak_syntax::{Highlighter, Language, Span};
use oak_text::Buffer;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum DocumentEvent {
    /// The text or the dirty state changed; tabs and the title bar re-read it.
    Changed,
    Saved,
}

pub struct Document {
    pub path: Option<PathBuf>,
    pub buffer: Buffer,
    pub language: Option<Language>,
    highlighter: Highlighter,
    spans: Vec<Span>,
    spans_version: Option<u64>,
}

impl EventEmitter<DocumentEvent> for Document {}

impl Document {
    pub fn new(path: Option<PathBuf>, text: &str) -> Self {
        let language = path.as_deref().and_then(Language::from_path);
        Document {
            path,
            buffer: Buffer::new(text),
            language,
            highlighter: Highlighter::new(),
            spans: Vec::new(),
            spans_version: None,
        }
    }

    pub fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".into())
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
    }

    /// Runs an edit on the buffer and tells every view.
    pub fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Buffer)) {
        let before = self.buffer.version();
        f(&mut self.buffer);
        if self.buffer.version() != before {
            cx.emit(DocumentEvent::Changed);
            cx.notify();
        }
    }

    pub fn save(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        std::fs::write(path, self.buffer.text_for_file())?;
        self.buffer.mark_saved();
        cx.emit(DocumentEvent::Saved);
        cx.notify();
        Ok(())
    }

    /// The highlight spans for the current text, computed once per version.
    pub fn spans(&mut self) -> &[Span] {
        let version = self.buffer.version();
        if self.spans_version != Some(version) {
            self.spans = match self.language {
                Some(lang) => self
                    .highlighter
                    .highlight(lang, self.buffer.text().as_bytes()),
                None => Vec::new(),
            };
            self.spans_version = Some(version);
        }
        &self.spans
    }
}
