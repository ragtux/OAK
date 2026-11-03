//! Smart Mode in the editor view (M7): the completion popup with its
//! documentation pane, the parameter info popup and the documentation
//! popup, all fed by the workspace from the document's language server
//! (`workspace/smart.rs`) and painted over the editor element like the
//! problems widget. The view asks through [`EditorEvent`]s and the
//! workspace answers through the `show_*` methods here.
//!
//! Batch 12 has not run, so every size is an estimate (`theme.rs`,
//! `COMPLETION_*` and `SMART_POPUP_*`); the colours are Fleet's keys:
//! `listItem.*` for the rows, `completion.match.text` for the matched
//! chars, `parameterInfo.match.text` for the active parameter, `popup.*`
//! for the frames.

use super::*;
use gpui::AnyElement;
use oak_lsp::lsp_types::{CompletionItem, CompletionItemKind, InsertTextFormat, SignatureHelp};
use oak_lsp::markup::Line;
use oak_lsp::TextChange;
use oak_search::fuzzy_match;

/// One completion the server offered, with its edit already in Oak's
/// char offsets (the workspace knows the server's position encoding).
#[derive(Clone, Debug)]
pub struct Candidate {
    pub item: CompletionItem,
    /// The chars the item replaces, at the version it was requested at.
    pub replace: Option<Range<usize>>,
    /// Its additional edits (an import to add), at the same version.
    pub additional: Vec<TextChange>,
}

#[derive(Clone, Debug)]
struct Filtered {
    ix: usize,
    score: i32,
    /// Matched char indices in the label, for the accent colour.
    positions: Vec<usize>,
}

pub struct CompletionPopup {
    candidates: Vec<Candidate>,
    /// Where the completed word starts, at `version`.
    start: usize,
    version: u64,
    is_incomplete: bool,
    filtered: Vec<Filtered>,
    selected: usize,
    scroll_top: usize,
    /// The resolved documentation of the selected item, and for which.
    docs: Option<Vec<Line>>,
    docs_for: Option<usize>,
    docs_shown: bool,
}

pub struct SignaturePopup {
    help: SignatureHelp,
}

pub struct HoverPopup {
    lines: Vec<Line>,
    /// The char the popup sits over.
    anchor: usize,
    /// The symbol's range, when the server gave one: the pointer may rest
    /// anywhere in it without closing the popup.
    range: Option<Range<usize>>,
    from_mouse: bool,
}

/// What the view keeps for Smart Mode.
#[derive(Default)]
pub struct SmartState {
    /// A server serves this document: the popups and the auto popup are on.
    pub on: bool,
    completion_triggers: Vec<char>,
    signature_triggers: Vec<char>,
    completion: Option<CompletionPopup>,
    signature: Option<SignaturePopup>,
    hover: Option<HoverPopup>,
    /// The quick fixes fetched for the expanded problem: its index, titles.
    pub(super) fixes: Option<(usize, Vec<String>)>,
    fixes_for: Option<usize>,
    auto_epoch: u64,
    auto_task: Option<Task<()>>,
    /// The char under the pointer, for the documentation on hover.
    pointer_offset: Option<usize>,
    pointer_epoch: u64,
    pointer_task: Option<Task<()>>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The expui icon for a completion item's kind (estimates: Fleet draws
/// its own set).
fn kind_icon(kind: Option<CompletionItemKind>) -> &'static str {
    match kind {
        Some(CompletionItemKind::FUNCTION) | Some(CompletionItemKind::CONSTRUCTOR) => {
            "expui/nodes/function"
        }
        Some(CompletionItemKind::METHOD) => "expui/nodes/method",
        Some(CompletionItemKind::FIELD) => "expui/nodes/field",
        Some(CompletionItemKind::PROPERTY) => "expui/nodes/property",
        Some(CompletionItemKind::VARIABLE) => "expui/nodes/variable",
        Some(CompletionItemKind::CLASS) | Some(CompletionItemKind::STRUCT) => "expui/nodes/class",
        Some(CompletionItemKind::INTERFACE) => "expui/nodes/interface",
        Some(CompletionItemKind::MODULE) => "expui/nodes/module",
        Some(CompletionItemKind::ENUM) => "expui/nodes/enum",
        Some(CompletionItemKind::ENUM_MEMBER) | Some(CompletionItemKind::CONSTANT) => {
            "expui/nodes/constant"
        }
        Some(CompletionItemKind::TYPE_PARAMETER) => "expui/nodes/parameter",
        Some(CompletionItemKind::FILE) | Some(CompletionItemKind::FOLDER) => "expui/nodes/folder",
        Some(CompletionItemKind::KEYWORD) => "expui/nodes/tag",
        _ => "expui/nodes/unknown",
    }
}

impl EditorView {
    // ---- what the workspace tells the view ------------------------------------

    /// Whether a server serves this document, and the chars after which it
    /// wants to be asked for completions and signatures.
    pub fn set_smart(
        &mut self,
        on: bool,
        completion_triggers: Vec<char>,
        signature_triggers: Vec<char>,
        cx: &mut Context<Self>,
    ) {
        self.smart.on = on;
        self.smart.completion_triggers = completion_triggers;
        self.smart.signature_triggers = signature_triggers;
        if !on {
            self.close_smart_popups(cx);
            self.smart.fixes = None;
            self.smart.fixes_for = None;
        }
        cx.notify();
    }

    /// The server's completions for the request made at buffer `version`.
    pub fn show_completions(
        &mut self,
        candidates: Vec<Candidate>,
        version: u64,
        is_incomplete: bool,
        explicit: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.smart.on {
            return;
        }
        let caret = self.caret(cx);
        // The word starts where the items' edits begin, else at the last
        // non-word char before the caret.
        let start_at_version = candidates
            .iter()
            .filter_map(|c| c.replace.as_ref().map(|r| r.start))
            .min();
        let start = match start_at_version {
            Some(s) => self.adjust_range(s..s, version, cx).start,
            None => self.with_buffer(cx, |_, b| {
                let mut s = caret;
                while s > 0 && b.char_at(s - 1).is_some_and(is_word) {
                    s -= 1;
                }
                s
            }),
        };
        if start > caret {
            return;
        }
        let keep_selected = self
            .smart
            .completion
            .as_ref()
            .and_then(|p| p.filtered.get(p.selected))
            .map(|f| p_label(&self.smart.completion.as_ref().expect("checked").candidates[f.ix]));
        self.smart.completion = Some(CompletionPopup {
            candidates,
            start,
            version,
            is_incomplete,
            filtered: Vec::new(),
            selected: 0,
            scroll_top: 0,
            docs: None,
            docs_for: None,
            docs_shown: false,
        });
        self.refilter_completions(cx);
        if let (Some(label), Some(popup)) = (keep_selected, self.smart.completion.as_mut()) {
            if let Some(ix) = popup
                .filtered
                .iter()
                .position(|f| p_label(&popup.candidates[f.ix]) == label)
            {
                popup.selected = ix;
            }
        }
        let empty = self
            .smart
            .completion
            .as_ref()
            .is_some_and(|p| p.filtered.is_empty());
        if empty && !explicit {
            self.smart.completion = None;
        }
        cx.notify();
    }

    /// The resolved documentation of a completion item, for the pane.
    pub fn set_completion_docs(&mut self, ix: usize, lines: Vec<Line>, cx: &mut Context<Self>) {
        if let Some(popup) = self.smart.completion.as_mut() {
            if popup.filtered.get(popup.selected).map(|f| f.ix) == Some(ix) {
                popup.docs = Some(lines);
                popup.docs_for = Some(ix);
                cx.notify();
            }
        }
    }

    pub fn show_signature(&mut self, help: Option<SignatureHelp>, cx: &mut Context<Self>) {
        self.smart.signature = help
            .filter(|h| !h.signatures.is_empty())
            .map(|help| SignaturePopup { help });
        cx.notify();
    }

    /// The documentation at `anchor` (`show-documentation`, or the pointer
    /// resting on a symbol); empty lines close the popup.
    pub fn show_hover(
        &mut self,
        lines: Vec<Line>,
        anchor: usize,
        range: Option<Range<usize>>,
        from_mouse: bool,
        cx: &mut Context<Self>,
    ) {
        if from_mouse {
            // The pointer moved on since the request: the answer is stale.
            let still = self
                .smart
                .pointer_offset
                .is_some_and(|p| range.as_ref().is_some_and(|r| r.contains(&p)) || p == anchor);
            if !still {
                return;
            }
        }
        self.smart.hover = (!lines.is_empty()).then_some(HoverPopup {
            lines,
            anchor,
            range,
            from_mouse,
        });
        cx.notify();
    }

    /// The quick fixes the server offers for a problem (its index).
    pub fn set_fixes(&mut self, problem: usize, titles: Vec<String>, cx: &mut Context<Self>) {
        self.smart.fixes = Some((problem, titles));
        cx.notify();
    }

    /// `show-documentation` while the completion popup is open toggles its
    /// documentation pane; `false` when no popup is open (the workspace then
    /// asks for the hover at the caret).
    pub fn toggle_completion_docs(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(popup) = self.smart.completion.as_mut() else {
            return false;
        };
        popup.docs_shown = !popup.docs_shown;
        if popup.docs_shown {
            self.request_docs(cx);
        }
        cx.notify();
        true
    }

    pub fn close_smart_popups(&mut self, cx: &mut Context<Self>) {
        self.smart.completion = None;
        self.smart.signature = None;
        self.smart.hover = None;
        cx.notify();
    }

    /// The caret's top-left corner in window coordinates (the quick-fix
    /// menu opens there).
    pub fn caret_window_point(&mut self, window: &mut Window, cx: &mut App) -> Point<Pixels> {
        let caret = self.caret(cx);
        let bounds = self.char_bounds(caret, window, cx);
        self.last_bounds.origin + bounds.origin
    }

    // ---- keys ------------------------------------------------------------------

    pub(super) fn smart_move_up(&mut self, a: &MoveUp, w: &mut Window, cx: &mut Context<Self>) {
        if !self.complete_step(-1, cx) {
            self.move_up(a, w, cx);
        }
    }

    pub(super) fn smart_move_down(&mut self, a: &MoveDown, w: &mut Window, cx: &mut Context<Self>) {
        if !self.complete_step(1, cx) {
            self.move_down(a, w, cx);
        }
    }

    pub(super) fn smart_page_up(&mut self, a: &PageUp, w: &mut Window, cx: &mut Context<Self>) {
        let rows = cx.global::<Metrics>().completion_max_rows as isize;
        if !self.complete_step(-rows, cx) {
            self.page_up(a, w, cx);
        }
    }

    pub(super) fn smart_page_down(&mut self, a: &PageDown, w: &mut Window, cx: &mut Context<Self>) {
        let rows = cx.global::<Metrics>().completion_max_rows as isize;
        if !self.complete_step(rows, cx) {
            self.page_down(a, w, cx);
        }
    }

    pub(super) fn smart_newline(&mut self, a: &Newline, w: &mut Window, cx: &mut Context<Self>) {
        if !self.accept_completion(cx) {
            self.newline(a, w, cx);
        }
    }

    pub(super) fn smart_tab(&mut self, a: &Tab, w: &mut Window, cx: &mut Context<Self>) {
        if !self.accept_completion(cx) {
            self.tab(a, w, cx);
        }
    }

    /// Escape closes the completion popup first, then the others, then
    /// does what it always did.
    pub(super) fn smart_cancel(&mut self, a: &Cancel, w: &mut Window, cx: &mut Context<Self>) {
        if self.smart.completion.take().is_some() {
            cx.notify();
            return;
        }
        if self.smart.signature.is_some() || self.smart.hover.is_some() {
            self.smart.signature = None;
            self.smart.hover = None;
            cx.notify();
            return;
        }
        self.cancel(a, w, cx);
    }

    fn complete_step(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(popup) = self.smart.completion.as_mut() else {
            return false;
        };
        if popup.filtered.is_empty() {
            return false;
        }
        let len = popup.filtered.len() as isize;
        let next = if delta.abs() == 1 {
            (popup.selected as isize + delta).rem_euclid(len)
        } else {
            (popup.selected as isize + delta).clamp(0, len - 1)
        };
        popup.selected = next as usize;
        popup.docs = None;
        popup.docs_for = None;
        if popup.docs_shown {
            self.request_docs(cx);
        }
        cx.notify();
        true
    }

    /// The selected completion goes in: its edit (or its text over the
    /// typed prefix) and its additional edits in one undo step, a snippet
    /// as plain text with its first tabstop selected.
    fn accept_completion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(popup) = self.smart.completion.take() else {
            return false;
        };
        let Some(f) = popup.filtered.get(popup.selected) else {
            cx.notify();
            return true;
        };
        let candidate = popup.candidates[f.ix].clone();
        let item = &candidate.item;
        let caret = self.caret(cx);
        let range = match &candidate.replace {
            Some(r) => self.adjust_range(r.clone(), popup.version, cx),
            None => {
                self.adjust_range(popup.start..popup.start, popup.version, cx)
                    .start..caret
            }
        };
        let range = range.start
            ..range
                .end
                .max(caret)
                .min(self.with_buffer(cx, |_, b| b.len_chars()));
        let raw = match &item.text_edit {
            Some(oak_lsp::lsp_types::CompletionTextEdit::Edit(e)) => e.new_text.clone(),
            Some(oak_lsp::lsp_types::CompletionTextEdit::InsertAndReplace(e)) => e.new_text.clone(),
            None => item
                .insert_text
                .clone()
                .unwrap_or_else(|| item.label.clone()),
        };
        let (text, select) = if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
            let plain = oak_lsp::snippet::plain(&raw);
            (plain.text, plain.select)
        } else {
            (raw, None)
        };
        let mut edits: Vec<(Range<usize>, String)> = vec![(range.clone(), text.clone())];
        // Chars inserted before the word by the additional edits move it.
        let mut shift: isize = 0;
        for change in &candidate.additional {
            let r = self.adjust_range(change.range.clone(), popup.version, cx);
            if r.end <= range.start {
                shift += change.text.chars().count() as isize - (r.end - r.start) as isize;
            }
            edits.push((r, change.text.clone()));
        }
        self.replace_ranges(edits, cx);
        let base = (range.start as isize + shift).max(0) as usize;
        let (s, e) = match select {
            Some(sel) => (base + sel.start, base + sel.end),
            None => {
                let end = base + text.chars().count();
                (end, end)
            }
        };
        self.go(cx, |c, b| c.select(b, s, e));
        self.after_move(cx);
        cx.emit(EditorEvent::Accepted {
            command: item.command.clone(),
        });
        cx.notify();
        true
    }

    // ---- typing and moving --------------------------------------------------------

    /// After a typed char: the open popup refilters (a trigger char asks
    /// again), else a word or trigger char schedules the auto popup
    /// (`completion.autoPopupDelay`); a signature trigger asks for the
    /// parameter info.
    pub(super) fn smart_typed(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.smart.on {
            return;
        }
        let Some(c) = text.chars().last() else { return };
        if self.smart.signature_triggers.contains(&c) {
            cx.emit(EditorEvent::SignatureHelp);
        }
        if self.smart.completion.is_some() {
            if self.smart.completion_triggers.contains(&c) {
                cx.emit(EditorEvent::Completion { explicit: false });
            } else if !is_word(c) {
                self.smart.completion = None;
                cx.notify();
            }
            return;
        }
        if is_word(c) || self.smart.completion_triggers.contains(&c) {
            self.schedule_auto_completion(cx);
        }
    }

    fn schedule_auto_completion(&mut self, cx: &mut Context<Self>) {
        self.smart.auto_epoch += 1;
        let epoch = self.smart.auto_epoch;
        let delay = cx
            .global::<AppSettings>()
            .settings
            .completion_auto_popup_delay_ms();
        self.smart.auto_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.smart.auto_epoch == epoch
                    && this.smart.on
                    && this.smart.completion.is_none()
                {
                    cx.emit(EditorEvent::Completion { explicit: false });
                }
            });
        }));
    }

    pub(super) fn smart_after_edit(&mut self, cx: &mut Context<Self>) {
        self.smart.hover = None;
        if self.smart.completion.is_some() {
            self.refilter_completions(cx);
            let gone = self
                .smart
                .completion
                .as_ref()
                .is_some_and(|p| p.filtered.is_empty() && !p.is_incomplete);
            let incomplete = self
                .smart
                .completion
                .as_ref()
                .is_some_and(|p| p.is_incomplete);
            if gone {
                self.smart.completion = None;
            } else if incomplete {
                cx.emit(EditorEvent::Completion { explicit: false });
            }
        }
        if self.smart.signature.is_some() {
            cx.emit(EditorEvent::SignatureHelp);
        }
    }

    pub(super) fn smart_after_move(&mut self, cx: &mut Context<Self>) {
        if self.smart.hover.as_ref().is_some_and(|h| !h.from_mouse) {
            self.smart.hover = None;
        }
        if let Some(popup) = &self.smart.completion {
            let start = self
                .adjust_range(popup.start..popup.start, popup.version, cx)
                .start;
            let caret = self.caret(cx);
            let in_word = caret >= start
                && self.with_buffer(cx, |_, b| {
                    (start..caret).all(|i| b.char_at(i).is_some_and(is_word))
                });
            if !in_word {
                self.smart.completion = None;
            }
        }
        if self.smart.signature.is_some() {
            cx.emit(EditorEvent::SignatureHelp);
        }
    }

    /// Matches the typed prefix against the items (the Goto matcher), best
    /// first; the server's order breaks ties through `sortText`.
    fn refilter_completions(&mut self, cx: &mut Context<Self>) {
        let caret = self.caret(cx);
        let Some(popup) = self.smart.completion.as_mut() else {
            return;
        };
        let start = {
            let version = popup.version;
            let s = popup.start;
            let buffer = &self.document.read(cx).buffer;
            let mut s2 = s;
            if let Some(edits) = buffer.edits_since(version) {
                for e in edits {
                    s2 = e.adjust(s2);
                }
            }
            s2
        };
        let prefix: String = self.document.read(cx).buffer.slice(start.min(caret)..caret);
        let mut filtered: Vec<Filtered> = popup
            .candidates
            .iter()
            .enumerate()
            .filter_map(|(ix, c)| {
                let text = c.item.filter_text.as_deref().unwrap_or(&c.item.label);
                let m = fuzzy_match(&prefix, text)?;
                let positions = if text == c.item.label {
                    m.positions
                } else {
                    Vec::new()
                };
                Some(Filtered {
                    ix,
                    score: m.score,
                    positions,
                })
            })
            .collect();
        let candidates = &popup.candidates;
        filtered.sort_by(|a, b| {
            let ca = &candidates[a.ix].item;
            let cb = &candidates[b.ix].item;
            b.score
                .cmp(&a.score)
                .then_with(|| {
                    cb.preselect
                        .unwrap_or(false)
                        .cmp(&ca.preselect.unwrap_or(false))
                })
                .then_with(|| {
                    ca.sort_text
                        .as_deref()
                        .unwrap_or(&ca.label)
                        .cmp(cb.sort_text.as_deref().unwrap_or(&cb.label))
                })
        });
        popup.filtered = filtered;
        popup.selected = 0;
        popup.scroll_top = 0;
        popup.docs = None;
        popup.docs_for = None;
        if popup.docs_shown {
            self.request_docs(cx);
        }
    }

    fn request_docs(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = &self.smart.completion {
            if let Some(f) = popup.filtered.get(popup.selected) {
                if popup.docs_for != Some(f.ix) {
                    cx.emit(EditorEvent::ResolveCompletion(f.ix));
                }
            }
        }
    }

    /// The pointer rests on a char: after `editor.hoverTooltipDelayMs` the
    /// documentation there is asked for (when no problem claims the spot).
    pub(super) fn smart_track_pointer(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.smart.on || !self.options.problem_tooltip {
            return;
        }
        let offset = self.offset_at_point(position, cx);
        if offset == self.smart.pointer_offset {
            return;
        }
        self.smart.pointer_offset = offset;
        self.smart.pointer_epoch += 1;
        self.smart.pointer_task = None;
        // A mouse popup stays while the pointer is within its symbol.
        if let Some(h) = &self.smart.hover {
            let inside = h.from_mouse
                && offset.is_some_and(|o| {
                    h.range.as_ref().is_some_and(|r| r.contains(&o)) || o == h.anchor
                });
            if h.from_mouse && !inside {
                self.smart.hover = None;
                cx.notify();
            }
        }
        let Some(offset) = offset else { return };
        let epoch = self.smart.pointer_epoch;
        let delay = cx
            .global::<AppSettings>()
            .settings
            .editor_hover_tooltip_delay_ms();
        self.smart.pointer_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.smart.pointer_epoch == epoch
                    && this.problem_hovered.is_none()
                    && this.smart.completion.is_none()
                {
                    cx.emit(EditorEvent::Hover {
                        offset,
                        mouse: true,
                    });
                }
            });
        }));
    }

    /// The char under a window position, from the row and a monospace
    /// column (an estimate on a wrapped row), `None` outside the text.
    fn offset_at_point(&self, position: Point<Pixels>, cx: &App) -> Option<usize> {
        let bounds = self.last_bounds;
        if !bounds.contains(&position) {
            return None;
        }
        let y = position.y - bounds.origin.y + self.scroll_y;
        let d = (y / self.line_height()).floor().max(0.) as usize;
        let (row, seg) = self.map.at(d);
        if seg > 0 {
            return None;
        }
        let x = position.x - bounds.origin.x - self.text_offset() - px(self.tm.gutter_width)
            + self.scroll_x;
        if x < px(0.) {
            return None;
        }
        let col = (x / self.space_px.max(px(1.))).floor() as usize;
        let doc = self.document.read(cx);
        if row >= doc.buffer.line_count() {
            return None;
        }
        let line_len = doc.buffer.line(row).chars().count();
        if col >= line_len {
            return None;
        }
        Some(doc.buffer.line_to_char(row) + col)
    }

    // ---- geometry ----------------------------------------------------------------

    /// A char's cell, relative to the editor element's origin.
    fn char_bounds(&mut self, offset: usize, window: &mut Window, cx: &App) -> Bounds<Pixels> {
        self.sync_map(cx);
        let doc = self.document.read(cx);
        let offset = offset.min(doc.buffer.len_chars());
        let at = doc.buffer.offset_to_point(offset);
        let theme = cx.global::<Theme>();
        let seg = self.map.segment_of(at.row, at.column);
        let range = self.map.segment_range(&doc.buffer, at.row, seg);
        let text = doc.buffer.line(at.row);
        let seg_text = &text[byte_of_col(&text, range.start)..byte_of_col(&text, range.end)];
        let line = DisplayLine::with_start_column(seg_text, range.start, doc.buffer.indent.width);
        let shaped =
            shape_display_line(&line, &[], theme, self.tm.font_size, &self.options, window);
        let col = at.column.saturating_sub(range.start);
        let x = shaped.x_for_index(line.display_byte_for_char(col));
        let lh = self.line_height();
        let d = self.map.display_row(at.row, at.column);
        Bounds::new(
            point(
                self.text_offset() + px(self.tm.gutter_width) + x - self.scroll_x,
                lh * d as f32 - self.scroll_y,
            ),
            size(self.space_px, lh),
        )
    }

    /// The problem whose card is expanded, as the widget decides it.
    fn expanded_problem(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        if !self.problems_open {
            return None;
        }
        let problems = self.problems(cx);
        let head = self.with_buffer(cx, |c, _| c.selection.head);
        let at_caret = problems
            .iter()
            .position(|p| p.range.start <= head && head <= p.range.end.max(p.range.start + 1));
        at_caret.or(self.problem_expanded.filter(|ix| *ix < problems.len()))
    }

    // ---- rendering ---------------------------------------------------------------

    pub(super) fn render_smart(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if !self.smart.on {
            return out;
        }
        // The expanded problem's fixes are fetched once per problem.
        let expanded = self.expanded_problem(cx);
        if expanded != self.smart.fixes_for {
            self.smart.fixes_for = expanded;
            self.smart.fixes = None;
            if let Some(ix) = expanded {
                cx.emit(EditorEvent::ProblemFixes(ix));
            }
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let bounds = self.last_bounds;
        let lh = self.line_height();
        if let Some(popup) = &self.smart.completion {
            if !popup.filtered.is_empty() {
                out.extend(self.render_completion(&theme, &m, bounds, lh, window, cx));
            }
        }
        if self.smart.signature.is_some() {
            out.extend(self.render_signature(&theme, &m, bounds, lh, window, cx));
        }
        if self.smart.hover.is_some() {
            out.extend(self.render_hover(&theme, &m, bounds, lh, window, cx));
        }
        out
    }

    /// A popup frame in `popup.*` colours at an element-relative position.
    fn frame(theme: &Theme, m: &Metrics, at: Point<Pixels>) -> gpui::Div {
        div()
            .absolute()
            .left(at.x)
            .top(at.y)
            .occlude()
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            .text_color(theme.popup_text)
            .rounded(px(m.popup_radius))
            .border_1()
            .border_color(theme.popup_border)
            .bg(theme.popup_bg)
            .shadow(vec![BoxShadow {
                color: theme.shadow_medium,
                offset: point(px(0.), px(m.popup_shadow_offset_y)),
                blur_radius: px(m.popup_shadow_blur),
                spread_radius: px(0.),
            }])
    }

    /// Above the anchor's row when there is room, else under it.
    fn place_above(
        anchor: Bounds<Pixels>,
        height: Pixels,
        element: Bounds<Pixels>,
        lh: Pixels,
        gap: Pixels,
    ) -> Pixels {
        let above = anchor.origin.y - gap - height;
        if above >= px(0.) {
            above
        } else {
            (anchor.origin.y + lh + gap).min((element.size.height - height).max(px(0.)))
        }
    }

    fn render_completion(
        &mut self,
        theme: &Theme,
        m: &Metrics,
        bounds: Bounds<Pixels>,
        lh: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let start = {
            let p = self.smart.completion.as_ref().expect("open");
            self.adjust_range(p.start..p.start, p.version, cx).start
        };
        let anchor = self.char_bounds(start, window, cx);
        let popup = self.smart.completion.as_mut().expect("open");
        let max_rows = m.completion_max_rows as usize;
        if popup.selected < popup.scroll_top {
            popup.scroll_top = popup.selected;
        } else if popup.selected >= popup.scroll_top + max_rows {
            popup.scroll_top = popup.selected + 1 - max_rows;
        }
        let shown = popup.filtered.len().min(max_rows);
        let height = px(shown as f32 * m.completion_row + 2. * m.completion_pad);
        let width = px(m.completion_width).min(bounds.size.width);
        let gap = px(m.completion_gap_y);
        let mut y = anchor.origin.y + lh + gap;
        if y + height > bounds.size.height && anchor.origin.y - gap - height >= px(0.) {
            y = anchor.origin.y - gap - height;
        }
        let mut x = anchor.origin.x.min(bounds.size.width - width).max(px(0.));
        // The documentation pane sits beside the popup on the right, else on
        // the left; when neither side has room, the popup moves left.
        let docs_width = px(m.completion_docs_width);
        let docs_gap = px(m.completion_docs_gap);
        let docs_x = if !popup.docs_shown {
            None
        } else if x + width + docs_gap + docs_width <= bounds.size.width {
            Some(x + width + docs_gap)
        } else if x - docs_gap - docs_width >= px(0.) {
            Some(x - docs_gap - docs_width)
        } else {
            x = (bounds.size.width - width - docs_gap - docs_width).max(px(0.));
            Some(x + width + docs_gap)
        };
        let selected = popup.selected;
        let rows: Vec<AnyElement> = popup
            .filtered
            .iter()
            .enumerate()
            .skip(popup.scroll_top)
            .take(max_rows)
            .map(|(row_ix, f)| {
                let item = &popup.candidates[f.ix].item;
                let is_selected = row_ix == selected;
                let label_detail = item.label_details.as_ref().and_then(|d| d.detail.clone());
                let description = item
                    .label_details
                    .as_ref()
                    .and_then(|d| d.description.clone())
                    .or_else(|| item.detail.clone());
                let label_chars: Vec<char> = item.label.chars().collect();
                // Runs of matched and unmatched chars.
                let mut runs: Vec<(String, bool)> = Vec::new();
                for (i, c) in label_chars.iter().enumerate() {
                    let matched = f.positions.contains(&i);
                    match runs.last_mut() {
                        Some((text, m)) if *m == matched => text.push(*c),
                        _ => runs.push((c.to_string(), matched)),
                    }
                }
                let text_color = if is_selected {
                    theme.list_text_focused
                } else {
                    theme.list_text
                };
                let deprecated = item.deprecated.unwrap_or(false)
                    || item.tags.as_ref().is_some_and(|t| {
                        t.contains(&oak_lsp::lsp_types::CompletionItemTag::DEPRECATED)
                    });
                div()
                    .id(ElementId::NamedInteger(
                        "completion-row".into(),
                        row_ix as u64,
                    ))
                    .h(px(m.completion_row))
                    .w_full()
                    .flex()
                    .items_center()
                    .pl(px(m.completion_icon_x))
                    .pr(px(m.completion_pad_right))
                    .rounded(px(m.popup_row_radius))
                    .cursor_pointer()
                    .when(is_selected, |d| d.bg(theme.list_bg_focused))
                    .when(!is_selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(p) = this.smart.completion.as_mut() {
                            p.selected = row_ix;
                        }
                        this.accept_completion(cx);
                    }))
                    .child(div().flex_shrink_0().mr(px(m.completion_icon_gap)).child(
                        crate::assets::icon(
                            kind_icon(item.kind),
                            m.icon_size,
                            theme.icon_secondary,
                        ),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(text_color)
                            .when(deprecated, |d| d.line_through())
                            .children(runs.into_iter().map(|(text, matched)| {
                                div()
                                    .when(matched, |d| {
                                        d.text_color(theme.completion_match_text)
                                            .bg(theme.completion_match_bg)
                                    })
                                    .child(SharedString::from(text))
                            }))
                            .children(label_detail.map(|d| {
                                div()
                                    .text_color(theme.list_text_secondary)
                                    .child(SharedString::from(d))
                            })),
                    )
                    .child(div().flex_1().min_w(px(m.completion_icon_gap)))
                    .children(description.map(|d| {
                        div()
                            .flex_shrink_0()
                            .max_w(px(m.completion_width * 0.45))
                            .truncate()
                            .text_color(theme.list_text_secondary)
                            .child(SharedString::from(d))
                    }))
                    .into_any_element()
            })
            .collect();
        let mut out = vec![Self::frame(theme, m, point(x, y))
            .w(width)
            .h(height)
            .p(px(m.completion_pad))
            .flex()
            .flex_col()
            .children(rows)
            .into_any_element()];
        if popup.docs_shown {
            let lines = popup.docs.clone().unwrap_or_else(|| {
                vec![Line {
                    text: "Loading documentation...".into(),
                    code: false,
                    separator: false,
                }]
            });
            let docs_x = docs_x.unwrap_or(x);
            // The pane fits above the editor's bottom edge.
            let docs_y = y.min((bounds.size.height - px(m.smart_popup_max_height)).max(px(0.)));
            out.push(
                Self::frame(theme, m, point(docs_x, docs_y))
                    .w(docs_width)
                    .max_h(px(m.smart_popup_max_height))
                    .overflow_hidden()
                    .px(px(m.smart_popup_pad_x))
                    .py(px(m.smart_popup_pad_y))
                    .flex()
                    .flex_col()
                    .children(Self::markup_lines(&lines, theme, m))
                    .into_any_element(),
            );
        }
        out
    }

    /// Markdown lines as elements: code in the editor face, a rule as a
    /// separator, prose wrapped.
    fn markup_lines(lines: &[Line], theme: &Theme, m: &Metrics) -> Vec<AnyElement> {
        lines
            .iter()
            .map(|line| {
                if line.separator {
                    div()
                        .h(px(1.))
                        .my(px(m.smart_popup_pad_y / 2.))
                        .bg(theme.popup_separator)
                        .into_any_element()
                } else if line.code {
                    div()
                        .font_family(theme.mono_font.clone())
                        .line_height(px(m.smart_popup_line))
                        .whitespace_nowrap()
                        .child(SharedString::from(if line.text.is_empty() {
                            " ".to_string()
                        } else {
                            line.text.clone()
                        }))
                        .into_any_element()
                } else {
                    div()
                        .line_height(px(m.smart_popup_line))
                        .child(SharedString::from(if line.text.is_empty() {
                            " ".to_string()
                        } else {
                            line.text.clone()
                        }))
                        .into_any_element()
                }
            })
            .collect()
    }

    fn render_signature(
        &mut self,
        theme: &Theme,
        m: &Metrics,
        bounds: Bounds<Pixels>,
        lh: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let caret = self.caret(cx);
        let anchor = self.char_bounds(caret, window, cx);
        let help = &self.smart.signature.as_ref().expect("open").help;
        let active_sig = help.active_signature.unwrap_or(0) as usize;
        let lines: Vec<AnyElement> = help
            .signatures
            .iter()
            .enumerate()
            .map(|(ix, sig)| {
                let is_active = ix == active_sig;
                let active_param = sig
                    .active_parameter
                    .or(help.active_parameter)
                    .map(|p| p as usize);
                // The active parameter's char range in the label.
                let label_chars: Vec<char> = sig.label.chars().collect();
                let range = active_param
                    .and_then(|p| sig.parameters.as_ref()?.get(p).cloned())
                    .and_then(|param| match param.label {
                        oak_lsp::lsp_types::ParameterLabel::Simple(s) => {
                            let s_chars: Vec<char> = s.chars().collect();
                            label_chars
                                .windows(s_chars.len().max(1))
                                .position(|w| w == s_chars.as_slice())
                                .map(|start| start..start + s_chars.len())
                        }
                        oak_lsp::lsp_types::ParameterLabel::LabelOffsets([s, e]) => {
                            // UTF-16 offsets into the label.
                            let to_char = |u: u32| {
                                let mut units = 0u32;
                                for (i, c) in label_chars.iter().enumerate() {
                                    if units >= u {
                                        return i;
                                    }
                                    units += c.len_utf16() as u32;
                                }
                                label_chars.len()
                            };
                            Some(to_char(s)..to_char(e))
                        }
                    })
                    .filter(|r| is_active && r.start < r.end && r.end <= label_chars.len());
                let (before, inside, after) = match &range {
                    Some(r) => (
                        label_chars[..r.start].iter().collect::<String>(),
                        label_chars[r.clone()].iter().collect::<String>(),
                        label_chars[r.end..].iter().collect::<String>(),
                    ),
                    None => (sig.label.clone(), String::new(), String::new()),
                };
                div()
                    .flex()
                    .items_center()
                    .line_height(px(m.smart_popup_line))
                    .font_family(theme.mono_font.clone())
                    .whitespace_nowrap()
                    .text_color(if is_active {
                        theme.popup_text
                    } else {
                        theme.text_secondary
                    })
                    .child(SharedString::from(before))
                    .when(!inside.is_empty(), |d| {
                        d.child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.parameter_info_match_text)
                                .child(SharedString::from(inside)),
                        )
                    })
                    .child(SharedString::from(after))
                    .into_any_element()
            })
            .collect();
        let height = px(lines.len() as f32 * m.smart_popup_line + 2. * m.smart_popup_pad_y + 2.);
        let y = Self::place_above(anchor, height, bounds, lh, px(m.completion_gap_y));
        let x = anchor.origin.x.max(px(0.));
        vec![Self::frame(theme, m, point(x, y))
            .max_w(px(m.smart_popup_max_width).min(bounds.size.width - x))
            .px(px(m.smart_popup_pad_x))
            .py(px(m.smart_popup_pad_y))
            .flex()
            .flex_col()
            .children(lines)
            .into_any_element()]
    }

    fn render_hover(
        &mut self,
        theme: &Theme,
        m: &Metrics,
        bounds: Bounds<Pixels>,
        lh: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (anchor_offset, lines) = {
            let h = self.smart.hover.as_ref().expect("open");
            (h.anchor, h.lines.clone())
        };
        let anchor = self.char_bounds(anchor_offset, window, cx);
        let shown = lines.len().min(
            ((m.smart_popup_max_height - 2. * m.smart_popup_pad_y) / m.smart_popup_line) as usize,
        );
        let height = px(shown as f32 * m.smart_popup_line + 2. * m.smart_popup_pad_y + 2.);
        let y = Self::place_above(anchor, height, bounds, lh, px(m.completion_gap_y));
        let x = anchor.origin.x.max(px(0.));
        vec![Self::frame(theme, m, point(x, y))
            .max_w(px(m.smart_popup_max_width).min(bounds.size.width - x))
            .max_h(px(m.smart_popup_max_height))
            .overflow_hidden()
            .px(px(m.smart_popup_pad_x))
            .py(px(m.smart_popup_pad_y))
            .flex()
            .flex_col()
            .children(Self::markup_lines(&lines, theme, m))
            .into_any_element()]
    }
}

fn p_label(c: &Candidate) -> String {
    c.item.label.clone()
}
