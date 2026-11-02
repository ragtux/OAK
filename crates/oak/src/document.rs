//! One open file: its buffer, language, highlight spans, syntax tree and
//! folds, shared by every editor view that shows it (a tab in each split).
//! Views keep their own cursor and scroll; edits made through any of them
//! land here, and every view observes the document to repaint.

use gpui::{Context, EventEmitter};
use oak_settings::{EditorConfig, EndOfLine, IndentStyle, Settings};
use oak_syntax::{Fold, Highlighter, Language, Span, SyntaxTree};
use oak_text::{Buffer, Indent};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum DocumentEvent {
    /// The text or the dirty state changed; tabs and the title bar re-read it.
    Changed,
    Saved,
}

/// What the indentation was derived from, to notice a settings change.
#[derive(Clone, Debug, PartialEq, Eq)]
struct IndentSource {
    tab_size: usize,
    style: IndentStyle,
    auto_detect: bool,
}

pub struct Document {
    pub path: Option<PathBuf>,
    pub buffer: Buffer,
    pub language: Option<Language>,
    /// The `.editorconfig` values for the path, read when the file opened.
    pub editorconfig: EditorConfig,
    highlighter: Highlighter,
    spans: Vec<Span>,
    spans_version: Option<u64>,
    tree: Option<SyntaxTree>,
    tree_version: Option<u64>,
    folds: Vec<Fold>,
    folds_version: Option<u64>,
    indent_source: Option<IndentSource>,
    /// The file's modification time when it was read or written.
    disk_modified: Option<std::time::SystemTime>,
    /// The file changed on disk while this text had unsaved edits: the editor
    /// shows Fleet's banner until Overwrite or Reload.
    pub conflict: bool,
}

fn modified_time(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

impl EventEmitter<DocumentEvent> for Document {}

impl Document {
    pub fn new(path: Option<PathBuf>, text: &str, settings: &Settings) -> Self {
        let language = path.as_deref().and_then(Language::from_path);
        let editorconfig = path
            .as_deref()
            .map(EditorConfig::for_path)
            .unwrap_or_default();
        let mut doc = Document {
            path,
            buffer: Buffer::new(text),
            language,
            editorconfig,
            highlighter: Highlighter::new(),
            spans: Vec::new(),
            spans_version: None,
            tree: None,
            tree_version: None,
            folds: Vec::new(),
            folds_version: None,
            indent_source: None,
            disk_modified: None,
            conflict: false,
        };
        doc.disk_modified = doc.path.as_deref().and_then(modified_time);
        doc.refresh_indent(settings);
        doc
    }

    /// The file on disk is newer (or older) than what was read or written.
    pub fn disk_changed(&self) -> bool {
        match (&self.path, self.disk_modified) {
            (Some(path), Some(then)) => modified_time(path).is_some_and(|now| now != then),
            (Some(path), None) => modified_time(path).is_some(),
            (None, _) => false,
        }
    }

    /// The file changed under the document: a clean one follows the disk, an
    /// edited one raises the banner.
    pub fn note_disk_change(&mut self, cx: &mut Context<Self>) {
        if self.is_dirty() {
            self.conflict = true;
            self.disk_modified = self.path.as_deref().and_then(modified_time);
            cx.notify();
        } else if let Err(e) = self.reload_from_disk(cx) {
            eprintln!("oak: cannot reload {}: {e:#}", self.title());
        }
    }

    /// Replaces the text with the file's (Reload).
    pub fn reload_from_disk(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let text = std::fs::read_to_string(&path)?;
        self.edit(cx, |buffer| buffer.set_text(&text));
        self.buffer.mark_saved();
        self.disk_modified = modified_time(&path);
        self.conflict = false;
        cx.emit(DocumentEvent::Saved);
        cx.notify();
        Ok(())
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

    /// The file moved (`files/rename`): the language follows the new name.
    pub fn set_path(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        self.language = path.as_deref().and_then(Language::from_path);
        self.editorconfig = path
            .as_deref()
            .map(EditorConfig::for_path)
            .unwrap_or_default();
        self.disk_modified = path.as_deref().and_then(modified_time);
        self.path = path;
        self.spans_version = None;
        self.tree = None;
        self.tree_version = None;
        self.folds_version = None;
        cx.emit(DocumentEvent::Changed);
        cx.notify();
    }

    /// The indentation the buffer edits with: `editor.tabSize` and
    /// `editor.indentStyle`, overridden by `.editorconfig`, overridden by what
    /// the file itself uses when `editor.autoDetectIndentation` is on.
    pub fn refresh_indent(&mut self, settings: &Settings) {
        let source = IndentSource {
            tab_size: settings.editor_tab_size(),
            style: settings.editor_indent_style(),
            auto_detect: settings.editor_auto_detect_indentation(),
        };
        if self.indent_source.as_ref() == Some(&source) {
            return;
        }
        let mut indent = match source.style {
            IndentStyle::Tabs => Indent::tabs(source.tab_size),
            IndentStyle::Spaces => Indent::spaces(source.tab_size),
        };
        let ec = &self.editorconfig;
        if let Some(width) = ec.indent_size.or(ec.tab_width) {
            indent.width = width.max(1);
        }
        if let Some(style) = ec.indent_style {
            indent.use_tabs = style == IndentStyle::Tabs;
        }
        if source.auto_detect {
            if let Some(detected) = Indent::detect(&self.buffer.text()) {
                indent = detected;
                if detected.use_tabs {
                    indent.width = ec.tab_width.unwrap_or(source.tab_size).max(1);
                }
            }
        }
        self.buffer.set_indent(indent);
        self.indent_source = Some(source);
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

    /// `editor/change-line-endings`: the file is written back with the other
    /// ending; the text in memory stays LF.
    pub fn set_crlf(&mut self, crlf: bool, cx: &mut Context<Self>) {
        if self.buffer.uses_crlf() != crlf {
            self.buffer.set_crlf(crlf);
            cx.emit(DocumentEvent::Changed);
            cx.notify();
        }
    }

    /// Writes the file. `trimTrailingSpacesOnSave` (and `.editorconfig`'s
    /// `trim_trailing_whitespace`) strips line ends first, except on the rows
    /// holding carets; `insert_final_newline` adds the trailing newline.
    pub fn save(
        &mut self,
        caret_rows: &[usize],
        settings: &Settings,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let trim = self
            .editorconfig
            .trim_trailing_whitespace
            .unwrap_or_else(|| settings.trim_trailing_spaces_on_save());
        let final_newline = self.editorconfig.insert_final_newline.unwrap_or(false);
        if let Some(eol) = self.editorconfig.end_of_line {
            self.buffer.set_crlf(eol == EndOfLine::CrLf);
        }
        self.edit(cx, |buffer| {
            if trim {
                buffer.trim_trailing_whitespace(caret_rows);
            }
            if final_newline {
                buffer.ensure_final_newline();
            }
        });
        std::fs::write(&path, self.buffer.text_for_file())?;
        self.buffer.mark_saved();
        self.disk_modified = modified_time(&path);
        self.conflict = false;
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

    /// The syntax tree for the current text (none for plain text).
    pub fn syntax(&mut self) -> Option<&SyntaxTree> {
        let version = self.buffer.version();
        if self.tree_version != Some(version) {
            let text = self.buffer.text();
            match (&mut self.tree, self.language) {
                (Some(tree), Some(lang)) if tree.language() == lang => tree.reparse(&text),
                (slot, Some(lang)) => *slot = SyntaxTree::parse(lang, &text),
                (slot, None) => *slot = None,
            }
            self.tree_version = Some(version);
        }
        self.tree.as_ref()
    }

    /// The folds as last computed (`folds` refreshes them each frame).
    pub fn cached_folds(&self) -> &[Fold] {
        &self.folds
    }

    /// The foldable regions of the current text, from the syntax tree.
    pub fn folds(&mut self) -> &[Fold] {
        let version = self.buffer.version();
        if self.folds_version != Some(version) {
            let text = self.buffer.text();
            self.folds = match self.syntax() {
                Some(tree) => tree.folds(&text),
                None => Vec::new(),
            };
            self.folds_version = Some(version);
        }
        &self.folds
    }
}
