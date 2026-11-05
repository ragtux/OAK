//! One open file: its buffer, language, highlight spans, syntax tree and
//! folds, shared by every editor view that shows it (a tab in each split).
//! Views keep their own cursor and scroll; edits made through any of them
//! land here, and every view observes the document to repaint.
//!
//! With Smart Mode on (M7) the language server feeds the document too: its
//! diagnostics replace the parser's syntax errors as the problems, and its
//! semantic tokens colour spans over the tree-sitter highlighting
//! (`editor.semanticHighlighting`), and its code lenses are the code vision
//! rows above declarations (`editor.showCodeVision`). All are kept with the
//! buffer version they were computed for and moved through the edits made
//! since.
//!
//! The file's breakpoints (M8) live in the app's `BreakpointStore`, not here;
//! every edit moves them with their lines. The ones an edit deleted with
//! their lines are kept here (`removed_breakpoints`), tied to this buffer's
//! undo steps, so undoing the deletion brings them back.

use crate::breakpoints::{BreakpointStore, Removed, Tracked};
use gpui::{Context, EventEmitter};
use std::ops::Range;
use std::path::PathBuf;
use weep_settings::{EditorConfig, EndOfLine, IndentStyle, Settings};
use weep_syntax::{Fold, Highlighter, Language, Span, SyntaxTree};
use weep_text::{Buffer, Indent};

#[derive(Clone, Debug)]
pub enum DocumentEvent {
    /// The text or the dirty state changed; tabs and the title bar re-read it.
    Changed,
    Saved,
    /// The git line status was read again; the editor repaints its gutter.
    StatusChanged,
}

/// What the indentation was derived from, to notice a settings change.
#[derive(Clone, Debug, PartialEq, Eq)]
struct IndentSource {
    tab_size: usize,
    style: IndentStyle,
    auto_detect: bool,
}

/// How bad a problem is: the server's severity, or an error for the
/// parser's. Each paints with its own `problem.*` attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    /// `problem.warning.weak` (LSP's hint).
    Weak,
    Info,
}

/// One problem in a document: a char range (empty for a missing token) and
/// a message; the server's diagnostic when it came from one, for the quick
/// fixes it is asked about.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub range: Range<usize>,
    pub message: String,
    pub severity: Severity,
    /// `problem.unused`: the server tagged the range as unnecessary.
    pub unused: bool,
    /// `problem.deprecated`.
    pub deprecated: bool,
    pub diagnostic: Option<weep_lsp::lsp_types::Diagnostic>,
}

/// A semantic token's span as a char range with its Fleet attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticSpan {
    pub range: Range<usize>,
    pub attribute: &'static str,
}

/// A code vision entry (Fleet's `editor.showCodeVision`): a resolved code
/// lens's title, drawn in a short row above the line holding `offset` (a
/// char offset), and the command a click on it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Lens {
    pub offset: usize,
    pub title: String,
    pub command: weep_lsp::lsp_types::Command,
}

pub struct Document {
    pub path: Option<PathBuf>,
    pub buffer: Buffer,
    pub language: Option<Language>,
    /// The `.editorconfig` values for the path, read when the file opened.
    pub editorconfig: EditorConfig,
    highlighter: Highlighter,
    /// The tree-sitter spans for `spans_version`.
    syntax_spans: Vec<Span>,
    syntax_spans_version: Option<u64>,
    /// The merged spans (syntax under semantic) for `spans_key`.
    spans: Vec<Span>,
    spans_key: Option<(u64, u64)>,
    tree: Option<SyntaxTree>,
    tree_version: Option<u64>,
    folds: Vec<Fold>,
    folds_version: Option<u64>,
    /// The parser's syntax errors (M6), as char ranges with a message, per version.
    syntax_problems: Vec<Problem>,
    syntax_problems_version: Option<u64>,
    /// The server's diagnostics (M7), for the version they were received
    /// at; `None` while no server has published for this file.
    diagnostics: Option<(u64, Vec<Problem>)>,
    /// The server's semantic tokens (M7) for the version they were
    /// computed at, and a counter bumped whenever they are replaced.
    semantic: Option<(u64, Vec<SemanticSpan>)>,
    semantic_generation: u64,
    /// The server's code lenses (M7) for the version they were computed at,
    /// sorted by offset.
    code_vision: Option<(u64, Vec<Lens>)>,
    indent_source: Option<IndentSource>,
    /// The file's modification time when it was read or written.
    disk_modified: Option<std::time::SystemTime>,
    /// The file changed on disk while this text had unsaved edits: the editor
    /// shows Fleet's banner until Overwrite or Reload.
    pub conflict: bool,
    /// The changed runs against HEAD (`editor.showLineStatus`), read with the
    /// git status; against the saved text, not the unsaved edits (estimate:
    /// Fleet diffs the buffer).
    pub hunks: Vec<weep_git::Hunk>,
    /// Breakpoints edits deleted with their lines, until their undo step is
    /// undone or leaves the stack (M8, `breakpoints::Removed`).
    removed_breakpoints: Removed,
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
            syntax_spans: Vec::new(),
            syntax_spans_version: None,
            spans: Vec::new(),
            spans_key: None,
            tree: None,
            tree_version: None,
            folds: Vec::new(),
            folds_version: None,
            syntax_problems: Vec::new(),
            syntax_problems_version: None,
            diagnostics: None,
            semantic: None,
            semantic_generation: 0,
            code_vision: None,
            indent_source: None,
            disk_modified: None,
            conflict: false,
            hunks: Vec::new(),
            removed_breakpoints: Removed::default(),
        };
        doc.disk_modified = doc.path.as_deref().and_then(modified_time);
        doc.refresh_indent(settings);
        doc
    }

    pub fn set_hunks(&mut self, hunks: Vec<weep_git::Hunk>, cx: &mut Context<Self>) {
        if self.hunks != hunks {
            self.hunks = hunks;
            cx.emit(DocumentEvent::StatusChanged);
            cx.notify();
        }
    }

    /// Revert on the line-status popup: HEAD's lines take the hunk's place
    /// (the whole text is replaced in one edit, an approximation).
    pub fn revert_hunk(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(hunk) = self.hunks.get(ix).cloned() else {
            return;
        };
        let text = self.buffer.text();
        let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        let end = (hunk.row + hunk.rows).min(lines.len());
        let row = hunk.row.min(lines.len());
        lines.splice(row..end, hunk.old.iter().cloned());
        let new_text = lines.join("\n");
        self.edit(cx, |buffer| buffer.set_text(&new_text));
        let shift = hunk.old.len() as isize - hunk.rows as isize;
        self.hunks.remove(ix);
        for h in self.hunks.iter_mut().filter(|h| h.row > hunk.row) {
            h.row = (h.row as isize + shift).max(0) as usize;
        }
        cx.emit(DocumentEvent::StatusChanged);
        cx.notify();
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
            eprintln!("weep: cannot reload {}: {e:#}", self.title());
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
        // M8: the breakpoints go with the file.
        if let (Some(from), Some(to), Some(store)) = (
            self.path.clone(),
            path.clone(),
            BreakpointStore::try_global(cx),
        ) {
            store.update(cx, |store, cx| store.rename_file(&from, &to, cx));
        }
        self.language = path.as_deref().and_then(Language::from_path);
        self.editorconfig = path
            .as_deref()
            .map(EditorConfig::for_path)
            .unwrap_or_default();
        self.disk_modified = path.as_deref().and_then(modified_time);
        self.path = path;
        self.syntax_spans_version = None;
        self.spans_key = None;
        self.tree = None;
        self.tree_version = None;
        self.folds_version = None;
        self.diagnostics = None;
        self.semantic = None;
        self.code_vision = None;
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

    /// Runs an edit on the buffer and tells every view. The file's
    /// breakpoints (M8) follow the edit's lines (`breakpoints::Tracked`).
    pub fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Buffer)) {
        let before = self.buffer.version();
        let breakpoints = Tracked::before_edit(
            self.path.as_deref(),
            &self.buffer,
            &self.removed_breakpoints,
            cx,
        );
        f(&mut self.buffer);
        if self.buffer.version() != before {
            if let Some(tracked) = breakpoints {
                tracked.after_edit(&self.buffer, &mut self.removed_breakpoints, cx);
            }
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

    /// The tree-sitter spans for the current text, computed once per version.
    fn syntax_spans(&mut self) -> &[Span] {
        let version = self.buffer.version();
        if self.syntax_spans_version != Some(version) {
            self.syntax_spans = match self.language {
                Some(lang) => self
                    .highlighter
                    .highlight(lang, self.buffer.text().as_bytes()),
                None => Vec::new(),
            };
            self.syntax_spans_version = Some(version);
        }
        &self.syntax_spans
    }

    /// The highlight spans for the current text: tree-sitter's, with the
    /// server's semantic tokens painted over them where there are any.
    pub fn spans(&mut self) -> &[Span] {
        let version = self.buffer.version();
        let key = (version, self.semantic_generation);
        if self.spans_key != Some(key) {
            self.catch_up_semantic();
            let semantic: Vec<(Range<usize>, &'static str)> = self
                .semantic
                .as_ref()
                .map(|(_, spans)| {
                    spans
                        .iter()
                        .map(|s| {
                            (
                                self.buffer.char_to_byte(s.range.start)
                                    ..self.buffer.char_to_byte(s.range.end),
                                s.attribute,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let syntax = self.syntax_spans().to_vec();
            self.spans = merge_spans(syntax, semantic);
            self.spans_key = Some(key);
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

    /// The parser's syntax errors of the current text
    /// (`editor.markupWithoutSmartMode`): tree-sitter's `ERROR` and `MISSING`
    /// nodes, as char ranges in document order with a message each.
    fn syntax_problems(&mut self) -> &[Problem] {
        let version = self.buffer.version();
        if self.syntax_problems_version != Some(version) {
            let found: Vec<weep_syntax::SyntaxProblem> = self
                .syntax()
                .map(|tree| tree.problems())
                .unwrap_or_default();
            let buffer = &self.buffer;
            self.syntax_problems = found
                .into_iter()
                .map(|p| Problem {
                    range: buffer.byte_to_char(p.range.start)..buffer.byte_to_char(p.range.end),
                    message: p.message,
                    severity: Severity::Error,
                    unused: false,
                    deprecated: false,
                    diagnostic: None,
                })
                .collect();
            self.syntax_problems_version = Some(version);
        }
        &self.syntax_problems
    }

    /// The problems the editor shows: the server's diagnostics once it has
    /// published for this file, else the parser's errors when `markup`
    /// (`editor.markupWithoutSmartMode`) asks for them. In document order.
    pub fn problems(&mut self, markup: bool) -> &[Problem] {
        if self.diagnostics.is_some() {
            self.catch_up_diagnostics();
            return &self.diagnostics.as_ref().expect("checked").1;
        }
        if markup {
            self.syntax_problems()
        } else {
            &[]
        }
    }

    /// The problems as last computed (`problems` refreshes them).
    pub fn cached_problems(&self) -> &[Problem] {
        match &self.diagnostics {
            Some((_, problems)) => problems,
            None => &self.syntax_problems,
        }
    }

    /// The server published for this file: `problems` are in char offsets
    /// of buffer `version` (the version the text was sent at).
    pub fn set_diagnostics(
        &mut self,
        version: u64,
        mut problems: Vec<Problem>,
        cx: &mut Context<Self>,
    ) {
        problems.sort_by_key(|p| (p.range.start, p.range.end));
        self.diagnostics = Some((version, problems));
        cx.notify();
    }

    /// Smart Mode went off (or the server left): back to the parser's errors.
    pub fn clear_server_state(&mut self, cx: &mut Context<Self>) {
        let had =
            self.diagnostics.is_some() || self.semantic.is_some() || self.code_vision.is_some();
        self.diagnostics = None;
        self.semantic = None;
        self.code_vision = None;
        self.semantic_generation += 1;
        if had {
            cx.notify();
        }
    }

    /// The server's semantic tokens for buffer `version`, as char ranges.
    pub fn set_semantic_spans(
        &mut self,
        version: u64,
        mut spans: Vec<SemanticSpan>,
        cx: &mut Context<Self>,
    ) {
        spans.sort_by_key(|s| (s.range.start, s.range.end));
        self.semantic = Some((version, spans));
        self.semantic_generation += 1;
        cx.notify();
    }

    /// The server's code lenses for buffer `version` (the code vision rows).
    pub fn set_code_vision(&mut self, version: u64, mut lenses: Vec<Lens>, cx: &mut Context<Self>) {
        lenses.sort_by_key(|l| l.offset);
        if self.code_vision.as_ref().map(|(_, l)| l) == Some(&lenses) {
            self.code_vision = Some((version, lenses));
            return;
        }
        self.code_vision = Some((version, lenses));
        cx.notify();
    }

    /// The code vision rows of the current text: each buffer row with a
    /// lens and its lenses in order, moved through the edits made since the
    /// server's answer (none when the log no longer reaches back to it).
    pub fn code_vision(&self) -> Vec<(usize, Vec<Lens>)> {
        let Some((at, lenses)) = &self.code_vision else {
            return Vec::new();
        };
        let Some(edits) = self.buffer.edits_since(*at) else {
            return Vec::new();
        };
        let len = self.buffer.len_chars();
        let mut rows: Vec<(usize, Vec<Lens>)> = Vec::new();
        for lens in lenses {
            let offset = edits.iter().fold(lens.offset, |o, e| e.adjust(o)).min(len);
            let row = self.buffer.offset_to_point(offset).row;
            let moved = Lens {
                offset,
                ..lens.clone()
            };
            match rows.last_mut() {
                Some((r, l)) if *r == row => l.push(moved),
                _ => rows.push((row, vec![moved])),
            }
        }
        rows.sort_by_key(|(r, _)| *r);
        rows.dedup_by(|b, a| {
            if a.0 == b.0 {
                a.1.append(&mut b.1);
                true
            } else {
                false
            }
        });
        rows
    }

    /// Moves the diagnostics' ranges through the edits since their version.
    fn catch_up_diagnostics(&mut self) {
        let version = self.buffer.version();
        let Some((at, problems)) = &mut self.diagnostics else {
            return;
        };
        if *at == version {
            return;
        }
        match self.buffer.edits_since(*at) {
            Some(edits) => {
                for p in problems.iter_mut() {
                    for e in &edits {
                        p.range = e.adjust(p.range.start)..e.adjust(p.range.end);
                    }
                    if p.range.end < p.range.start {
                        p.range.end = p.range.start;
                    }
                }
            }
            None => {
                // Too far behind the log: the ranges would be guesses.
                problems.clear();
            }
        }
        *at = version;
    }

    /// Moves the semantic spans through the edits since their version; a
    /// span an edit touched is dropped (its token may have changed).
    fn catch_up_semantic(&mut self) {
        let version = self.buffer.version();
        let Some((at, spans)) = &mut self.semantic else {
            return;
        };
        if *at == version {
            return;
        }
        match self.buffer.edits_since(*at) {
            Some(edits) => {
                spans.retain_mut(|s| {
                    for e in &edits {
                        let inside = e.offset > s.range.start && e.offset < s.range.end;
                        let removed = e.offset..e.offset + e.old_len;
                        let overlaps = removed.start < s.range.end && removed.end > s.range.start;
                        if inside || overlaps {
                            return false;
                        }
                        if e.old_len == 0 && e.offset == s.range.start {
                            // Typed just before the token: it moves whole.
                            s.range = s.range.start + e.new_len..s.range.end + e.new_len;
                        } else {
                            s.range = e.adjust(s.range.start)..e.adjust(s.range.end);
                        }
                    }
                    s.range.start < s.range.end
                });
            }
            None => spans.clear(),
        }
        *at = version;
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

/// Lays `semantic` byte spans over `syntax` ones: the parts of a syntax
/// span under a semantic span give way to it. Both inputs are sorted and
/// non-overlapping; the output is too.
fn merge_spans(syntax: Vec<Span>, semantic: Vec<(Range<usize>, &'static str)>) -> Vec<Span> {
    if semantic.is_empty() {
        return syntax;
    }
    let mut out: Vec<Span> = Vec::with_capacity(syntax.len() + semantic.len());
    let mut sem_ix = 0;
    for span in syntax {
        let mut start = span.range.start;
        let end = span.range.end;
        // Skip semantic spans wholly before this one.
        while sem_ix < semantic.len() && semantic[sem_ix].0.end <= start {
            sem_ix += 1;
        }
        let mut k = sem_ix;
        while k < semantic.len() && semantic[k].0.start < end {
            let (sr, _) = &semantic[k];
            if sr.start > start {
                out.push(Span {
                    range: start..sr.start,
                    attribute: span.attribute,
                });
            }
            start = start.max(sr.end);
            k += 1;
        }
        if start < end {
            out.push(Span {
                range: start..end,
                attribute: span.attribute,
            });
        }
    }
    out.extend(
        semantic
            .into_iter()
            .map(|(range, attribute)| Span { range, attribute }),
    );
    out.sort_by_key(|s| (s.range.start, s.range.end));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(range: Range<usize>, attribute: &'static str) -> Span {
        Span { range, attribute }
    }

    #[test]
    fn semantic_spans_cut_the_syntax_ones() {
        let syntax = vec![span(0..10, "keyword"), span(12..20, "identifier")];
        let semantic = vec![
            (2..4, "identifier.function.call"),
            (14..22, "identifier.type"),
        ];
        let merged = merge_spans(syntax, semantic);
        assert_eq!(
            merged,
            vec![
                span(0..2, "keyword"),
                span(2..4, "identifier.function.call"),
                span(4..10, "keyword"),
                span(12..14, "identifier"),
                span(14..22, "identifier.type"),
            ]
        );
    }

    #[test]
    fn a_semantic_span_covering_a_syntax_span_replaces_it() {
        let merged = merge_spans(vec![span(3..6, "a")], vec![(0..10, "b")]);
        assert_eq!(merged, vec![span(0..10, "b")]);
    }
}
