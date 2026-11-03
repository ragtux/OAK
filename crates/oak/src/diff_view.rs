//! The Changed Files tab (M4, batch 10: `docs/fleet-probe.md`, ninth pass):
//! a header row with a tri-state checkbox and "2/4 files", then one card per
//! changed file 6 inside the island. The open card has a 40 px header in
//! `listItem.background.focused` (checkbox, icon, name in the status colour,
//! a pencil and a Revert button), a 6 px strip, then the side-by-side diff:
//! the old text with its numbers right-aligned before a 2 px separator, the
//! new numbers and text after it, 22 px rows, unchanged runs collapsed to a
//! wavy separator, added rows washed in `diff.added`. Collapsed files are
//! 42 px rows. `diff/two-files` and `diff/with-clipboard` show the same
//! view over two texts of their own.

use crate::assets::{file_icon, icon};
use crate::editor::{styled_line, TextMetrics};
use crate::git_tool::{checkbox, status_text_color, CheckState};
use crate::theme::{metrics::DIFF_CONTEXT_LINES, Metrics, Theme};
use gpui::{
    canvas, div, point, prelude::*, px, App, Context, ElementId, EventEmitter, FocusHandle,
    Focusable, Hsla, MouseButton, Path as GpuiPath, Pixels, Point, ScrollHandle, SharedString,
    StyledText, TextRun, Window,
};
use oak_git::{diff_lines, ops_from_hunks, LineOp, Repo, Status, StatusEntry};
use oak_syntax::{Highlighter, Language, Span};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum DiffViewEvent {
    /// Files were staged or unstaged: the workspace re-reads git.
    Changed,
    /// The pencil: open the file in an editor.
    Open(PathBuf),
    /// The Revert button: the workspace confirms (`git.revert.confirm`) and reverts.
    Revert(StatusEntry),
    /// A right click on a diff row: the one-row "Copy" menu (ninth pass).
    ContextMenu {
        position: Point<Pixels>,
        text: String,
    },
}

/// One file the tab compares: the repository's change, or two texts of the
/// caller's (`diff/two-files`, `diff/with-clipboard`).
#[derive(Clone, Debug)]
pub struct DiffSource {
    /// What the rows are keyed by: the path, or the title.
    pub key: String,
    pub title: String,
    /// The folder shown after the name on a collapsed row.
    pub folder: Option<String>,
    pub path: Option<PathBuf>,
    pub entry: Option<StatusEntry>,
    /// The texts; None on one side is a file that does not exist there.
    pub old: Option<String>,
    pub new: Option<String>,
}

impl DiffSource {
    /// Two texts with names, for the diff actions.
    pub fn texts(title: String, path: Option<PathBuf>, old: String, new: String) -> DiffSource {
        DiffSource {
            key: title.clone(),
            title,
            folder: None,
            path,
            entry: None,
            old: Some(old),
            new: Some(new),
        }
    }
}

/// One half of a diff row.
struct Side {
    number: usize,
    text: SharedString,
    runs: Vec<TextRun>,
}

enum DiffRow {
    /// A line on either side; `changed` washes the row.
    Pair {
        old: Option<Side>,
        new: Option<Side>,
        changed: bool,
    },
    /// An unchanged run folded away.
    Collapsed,
}

/// A changed file's two texts and git's hunks for it, read on the
/// background executor (`git show`, `git diff` and the disk take seconds
/// each on a network share).
struct Loaded {
    old: Option<String>,
    new: Option<String>,
    hunks: Option<Vec<oak_git::Hunk>>,
}

impl Loaded {
    fn read(repo: &Repo, entry: &StatusEntry) -> Loaded {
        let old = match entry.status {
            Status::Untracked | Status::Added => None,
            Status::Renamed => entry.old_path.as_deref().and_then(|p| repo.head_text(p)),
            _ => repo.head_text(&entry.path),
        };
        let new = match entry.status {
            Status::Deleted => None,
            _ => std::fs::read_to_string(&entry.path).ok(),
        };
        Loaded {
            old,
            new,
            hunks: repo.file_diff(entry).ok(),
        }
    }
}

pub struct DiffView {
    repo: Option<Repo>,
    /// None for the repository's changed files; a title for a custom diff.
    custom_title: Option<String>,
    sources: Vec<DiffSource>,
    expanded: Vec<String>,
    rows: HashMap<String, Vec<DiffRow>>,
    /// The open cards' texts, by key; cleared by every refresh and read
    /// again for the cards that are open.
    loaded: HashMap<String, Loaded>,
    /// Bumped by each refresh, so a read started before it is dropped.
    load_generation: u64,
    /// A refresh is reading the changed files on the background executor;
    /// one asked for meanwhile runs after it.
    refreshing: bool,
    refresh_again: bool,
    /// `expand` asked for a file the pending refresh has not listed yet.
    pending_expand: Option<PathBuf>,
    highlighter: Highlighter,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<DiffViewEvent> for DiffView {}

impl Focusable for DiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl DiffView {
    /// The repository's changed files.
    pub fn changed_files(repo: Option<Repo>, cx: &mut Context<Self>) -> Self {
        let mut view = DiffView {
            repo,
            custom_title: None,
            sources: Vec::new(),
            expanded: Vec::new(),
            rows: HashMap::new(),
            loaded: HashMap::new(),
            load_generation: 0,
            refreshing: false,
            refresh_again: false,
            pending_expand: None,
            highlighter: Highlighter::new(),
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        };
        view.refresh(cx);
        view
    }

    /// Two texts (or more) the caller compares, every card open.
    pub fn custom(title: String, sources: Vec<DiffSource>, cx: &mut Context<Self>) -> Self {
        let mut view = DiffView {
            repo: None,
            custom_title: Some(title),
            expanded: sources.iter().map(|s| s.key.clone()).collect(),
            sources,
            rows: HashMap::new(),
            loaded: HashMap::new(),
            load_generation: 0,
            refreshing: false,
            refresh_again: false,
            pending_expand: None,
            highlighter: Highlighter::new(),
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        };
        view.rebuild(cx);
        view
    }

    pub fn is_changed_files(&self) -> bool {
        self.custom_title.is_none()
    }

    pub fn title(&self) -> String {
        self.custom_title
            .clone()
            .unwrap_or_else(|| "Changed Files".to_string())
    }

    /// Reads the changed files again (the workspace calls this with every
    /// git refresh); the open cards' diffs are rebuilt from disk. Git runs
    /// on the background executor; the tab follows once it has answered.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.refreshing {
            self.refresh_again = true;
            return;
        }
        self.refreshing = true;
        cx.spawn(async move |this, cx| {
            let entries = cx
                .background_spawn(async move { repo.status_entries().unwrap_or_default() })
                .await;
            let _ = this.update(cx, |this, cx| this.apply_entries(entries, cx));
        })
        .detach();
    }

    /// The changed files a refresh read: the cards, the ones that stay
    /// open, and a read of their texts.
    fn apply_entries(&mut self, mut entries: Vec<StatusEntry>, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.refreshing = false;
        if std::mem::take(&mut self.refresh_again) {
            self.refresh(cx);
        }
        entries.sort_by(|a, b| {
            a.path
                .file_name()
                .cmp(&b.path.file_name())
                .then(a.path.cmp(&b.path))
        });
        let root = repo.root.clone();
        self.sources = entries
            .into_iter()
            .map(|entry| {
                let title = entry
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let folder = entry
                    .path
                    .parent()
                    .and_then(|p| p.strip_prefix(&root).ok())
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .filter(|p| !p.is_empty());
                DiffSource {
                    key: entry.path.to_string_lossy().to_string(),
                    title,
                    folder,
                    path: Some(entry.path.clone()),
                    entry: Some(entry),
                    old: None,
                    new: None,
                }
            })
            .collect();
        let keys: Vec<String> = self.sources.iter().map(|s| s.key.clone()).collect();
        self.expanded.retain(|k| keys.contains(k));
        if let Some(key) = self
            .pending_expand
            .take()
            .and_then(|path| self.key_of(&path))
        {
            self.expanded = vec![key];
        }
        if self.expanded.is_empty()
            && cx
                .global::<crate::settings::AppSettings>()
                .settings
                .ff_expand_git_diff_by_default()
        {
            self.expanded = keys;
        }
        // The texts are read again from disk and HEAD with every refresh.
        self.loaded.clear();
        self.load_generation = self.load_generation.wrapping_add(1);
        self.load_expanded(cx);
    }

    /// Reads the texts of the open cards that lack them on the background
    /// executor, then rebuilds the rows (at once when none is missing).
    fn load_expanded(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<(String, StatusEntry)> = self
            .sources
            .iter()
            .filter(|s| self.expanded.contains(&s.key) && !self.loaded.contains_key(&s.key))
            .filter_map(|s| Some((s.key.clone(), s.entry.clone()?)))
            .collect();
        let Some(repo) = self.repo.clone().filter(|_| !missing.is_empty()) else {
            self.rebuild(cx);
            return;
        };
        let generation = self.load_generation;
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    missing
                        .into_iter()
                        .map(|(key, entry)| (key, Loaded::read(&repo, &entry)))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.load_generation != generation {
                    return;
                }
                this.loaded.extend(loaded);
                this.rebuild(cx);
            });
        })
        .detach();
        self.rebuild(cx);
    }

    /// The card key of `path`. Paths compare by component (git's slashes
    /// against the editor's backslashes on Windows), so the key is looked
    /// up, not built.
    fn key_of(&self, path: &Path) -> Option<String> {
        self.sources
            .iter()
            .find(|s| s.path.as_deref() == Some(path))
            .map(|s| s.key.clone())
    }

    /// Opens the card for `path` (and closes the others, as Fleet's shot
    /// shows one open), scrolled into view; a file the running refresh has
    /// not listed yet opens once it has.
    pub fn expand(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match self.key_of(&path) {
            Some(key) => {
                self.expanded = vec![key];
                self.load_expanded(cx);
            }
            None if self.refreshing => self.pending_expand = Some(path),
            None => {}
        }
    }

    fn toggle_expanded(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(ix) = self.expanded.iter().position(|k| *k == key) {
            self.expanded.remove(ix);
        } else {
            self.expanded.push(key);
        }
        self.load_expanded(cx);
    }

    /// Computes the rows of every open card whose texts have been read.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.rows.clear();
        let theme = cx.global::<Theme>().clone();
        for ix in 0..self.sources.len() {
            let key = self.sources[ix].key.clone();
            if !self.expanded.contains(&key) {
                continue;
            }
            let Some((old, new, hunks)) = self.texts(ix) else {
                continue;
            };
            let language = self.sources[ix]
                .path
                .as_deref()
                .and_then(Language::from_path);
            let mut old_lines = old.as_deref().map(lines).unwrap_or_default();
            let mut new_lines = new.as_deref().map(lines).unwrap_or_default();
            let old_spans = old
                .as_deref()
                .zip(language)
                .map(|(t, l)| self.highlighter.highlight(l, t.as_bytes()))
                .unwrap_or_default();
            let new_spans = new
                .as_deref()
                .zip(language)
                .map(|(t, l)| self.highlighter.highlight(l, t.as_bytes()))
                .unwrap_or_default();
            // Git's own hunks pair the rows for a repository change (Fleet's
            // pairing); a custom diff, or hunks that do not fit, use Oak's.
            let from_git =
                hunks.and_then(|hunks| ops_from_hunks(&hunks, old_lines.len(), new_lines.len()));
            let mut ops = from_git.unwrap_or_else(|| diff_lines(&old_lines, &new_lines));
            let old_phantom = old.as_deref().and_then(phantom);
            let new_phantom = new.as_deref().and_then(phantom);
            match (old_phantom, new_phantom) {
                (Some(o), Some(n)) => {
                    old_lines.push(o);
                    new_lines.push(n);
                    ops.push(LineOp::Keep(old_lines.len() - 1, new_lines.len() - 1));
                }
                (Some(o), None) => {
                    old_lines.push(o);
                    ops.push(LineOp::Remove(old_lines.len() - 1));
                }
                (None, Some(n)) => {
                    new_lines.push(n);
                    ops.push(LineOp::Add(new_lines.len() - 1));
                }
                (None, None) => {}
            }
            let side = |lines: &[Line], spans: &[Span], row: usize| -> Side {
                let line = &lines[row];
                let local: Vec<(std::ops::Range<usize>, &'static str)> = spans
                    .iter()
                    .filter(|s| s.range.start < line.end && s.range.end > line.start)
                    .map(|s| {
                        (
                            s.range.start.max(line.start) - line.start
                                ..s.range.end.min(line.end) - line.start,
                            s.attribute,
                        )
                    })
                    .collect();
                let (text, runs) = styled_line(&line.text, &local, &theme, cx);
                Side {
                    number: row + 1,
                    text: text.into(),
                    runs,
                }
            };
            let rows = build_rows(&ops, |op| match op {
                LineOp::Keep(o, n) => (
                    Some(side(&old_lines, &old_spans, o)),
                    Some(side(&new_lines, &new_spans, n)),
                ),
                LineOp::Remove(o) => (Some(side(&old_lines, &old_spans, o)), None),
                LineOp::Add(n) => (None, Some(side(&new_lines, &new_spans, n))),
            });
            self.rows.insert(key, rows);
        }
        cx.notify();
    }

    /// The two texts of a source and git's hunks: its own texts, or what
    /// `load_expanded` read of HEAD and the disk (None until it has).
    #[allow(clippy::type_complexity)]
    fn texts(
        &self,
        ix: usize,
    ) -> Option<(Option<String>, Option<String>, Option<Vec<oak_git::Hunk>>)> {
        let source = &self.sources[ix];
        if source.old.is_some() || source.new.is_some() {
            return Some((source.old.clone(), source.new.clone(), None));
        }
        let loaded = self.loaded.get(&source.key)?;
        Some((loaded.old.clone(), loaded.new.clone(), loaded.hunks.clone()))
    }

    fn stage_toggle(&mut self, entry: StatusEntry, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let result = if entry.staged {
            repo.unstage(&[entry.path])
        } else {
            repo.stage(&[entry.path])
        };
        if let Err(e) = result {
            eprintln!("oak: {e:#}");
        }
        cx.emit(DiffViewEvent::Changed);
    }

    fn stage_toggle_all(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let entries: Vec<&StatusEntry> = self
            .sources
            .iter()
            .filter_map(|s| s.entry.as_ref())
            .collect();
        let paths: Vec<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
        let result = if entries.iter().all(|e| e.staged) {
            repo.unstage(&paths)
        } else {
            repo.stage(&paths)
        };
        if let Err(e) = result {
            eprintln!("oak: {e:#}");
        }
        cx.emit(DiffViewEvent::Changed);
    }
}

/// A line of a text with its byte range in it.
struct Line {
    text: String,
    start: usize,
    end: usize,
}

impl PartialEq for Line {
    fn eq(&self, other: &Line) -> bool {
        self.text == other.text
    }
}

fn lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0;
    for raw in text.split_inclusive('\n') {
        let end = start + raw.len();
        let body = raw.strip_suffix('\n').unwrap_or(raw);
        let body = body.strip_suffix('\r').unwrap_or(body);
        out.push(Line {
            text: body.to_string(),
            start,
            end: start + body.len(),
        });
        start = end;
    }
    out
}

/// The empty last line an editor shows after a final newline; appended
/// after the diff so that both texts' phantoms pair as context (Fleet's
/// shot numbers the row after an added last line, no checkbox).
fn phantom(text: &str) -> Option<Line> {
    (text.ends_with('\n') || text.is_empty()).then(|| Line {
        text: String::new(),
        start: text.len(),
        end: text.len(),
    })
}

/// Pairs the diff's ops into side-by-side rows: a run of removes and adds
/// becomes max(removes, adds) rows; unchanged lines stay only within
/// `DIFF_CONTEXT_LINES` of a change (and the first line, which Fleet's shot
/// keeps, estimate), the rest folding into one `Collapsed` row.
fn build_rows(
    ops: &[LineOp],
    mut make: impl FnMut(LineOp) -> (Option<Side>, Option<Side>),
) -> Vec<DiffRow> {
    let n = ops.len();
    let changed: Vec<bool> = ops
        .iter()
        .map(|op| !matches!(op, LineOp::Keep(..)))
        .collect();
    let any_change = changed.iter().any(|c| *c);
    let visible = |i: usize| -> bool {
        if !any_change || i == 0 {
            return true;
        }
        let lo = i.saturating_sub(DIFF_CONTEXT_LINES);
        let hi = (i + DIFF_CONTEXT_LINES).min(n - 1);
        (lo..=hi).any(|j| changed[j])
    };
    let mut rows = Vec::new();
    let mut i = 0;
    while i < n {
        if changed[i] {
            let mut removes = Vec::new();
            let mut adds = Vec::new();
            while i < n && changed[i] {
                match ops[i] {
                    LineOp::Remove(_) => removes.push(make(ops[i]).0),
                    LineOp::Add(_) => adds.push(make(ops[i]).1),
                    LineOp::Keep(..) => {}
                }
                i += 1;
            }
            let count = removes.len().max(adds.len());
            let mut removes = removes.into_iter();
            let mut adds = adds.into_iter();
            for _ in 0..count {
                rows.push(DiffRow::Pair {
                    old: removes.next().flatten(),
                    new: adds.next().flatten(),
                    changed: true,
                });
            }
        } else if visible(i) {
            let (old, new) = make(ops[i]);
            rows.push(DiffRow::Pair {
                old,
                new,
                changed: false,
            });
            i += 1;
        } else {
            if !matches!(rows.last(), Some(DiffRow::Collapsed)) {
                rows.push(DiffRow::Collapsed);
            }
            i += 1;
        }
    }
    rows
}

impl Render for DiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let font_size = TextMetrics::current(cx).font_size;
        let entity = cx.entity();
        let entries: Vec<&StatusEntry> = self
            .sources
            .iter()
            .filter_map(|s| s.entry.as_ref())
            .collect();
        let staged = entries.iter().filter(|e| e.staged).count();
        let all = match staged {
            0 => CheckState::Off,
            n if n == entries.len() => CheckState::All,
            _ => CheckState::Some,
        };
        let count: SharedString = if self.is_changed_files() {
            format!("{staged}/{} files", entries.len()).into()
        } else {
            format!(
                "{} file{}",
                self.sources.len(),
                if self.sources.len() == 1 { "" } else { "s" }
            )
            .into()
        };
        let all_entity = entity.clone();
        let header = div()
            .flex_shrink_0()
            .mt(px(m.diff_header_top))
            .h(px(m.diff_header_height))
            .pl(px(m.diff_header_check_x))
            .flex()
            .items_center()
            .gap(px(m.git_count_gap))
            .when(self.is_changed_files(), |d| {
                d.child(
                    checkbox("diff-check-all", all, &theme, &m)
                        .size(px(m.diff_checkbox))
                        .on_click(move |_, _, cx| {
                            all_entity.update(cx, |this, cx| this.stage_toggle_all(cx))
                        }),
                )
            })
            .child(div().text_color(theme.text_primary).child(count));

        let cards: Vec<gpui::AnyElement> = self
            .sources
            .iter()
            .enumerate()
            .map(|(ix, source)| {
                let open = self.expanded.contains(&source.key);
                let rows = self.rows.get(&source.key);
                self.render_card(ix, source, open, rows, font_size, &theme, &m, cx)
            })
            .collect();

        div()
            .key_context("DiffView")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            .child(header)
            .child(
                div()
                    .id("diff-cards")
                    .flex_1()
                    .min_h_0()
                    .px(px(m.diff_card_inset))
                    .pt(px(m.diff_card_gap))
                    .pb(px(m.diff_card_inset))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(m.diff_card_gap))
                    .children(cards),
            )
    }
}

impl DiffView {
    /// One file's card: its header row, and the diff when open.
    #[allow(clippy::too_many_arguments)]
    fn render_card(
        &self,
        ix: usize,
        source: &DiffSource,
        open: bool,
        rows: Option<&Vec<DiffRow>>,
        font_size: f32,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let entity = cx.entity();
        let name: SharedString = source.title.clone().into();
        let name_color = source
            .entry
            .as_ref()
            .map(|e| status_text_color(e.status, theme))
            .unwrap_or(theme.text_primary);
        let old_name: Option<SharedString> = source
            .entry
            .as_ref()
            .and_then(|e| e.old_path.as_ref())
            .map(|p| {
                format!(
                    "({})",
                    p.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                )
                .into()
            });
        let folder: Option<SharedString> = source.folder.clone().map(Into::into);
        let key = source.key.clone();
        let toggle = entity.clone();
        let (check_x, icon_x) = if open {
            (m.diff_card_check_x, m.diff_card_icon_x)
        } else {
            (m.diff_collapsed_check_x, m.diff_collapsed_icon_x)
        };
        let mut header = div()
            .id(ElementId::NamedInteger(
                "diff-card-header".into(),
                ix as u64,
            ))
            .h(px(if open {
                m.diff_card_header
            } else {
                m.diff_collapsed_row
            }))
            .flex_shrink_0()
            .pl(px(check_x))
            .flex()
            .items_center()
            .cursor_pointer()
            .when(open, |d| {
                d.bg(theme.list_bg_focused)
                    .rounded_t(px(m.diff_card_radius))
            })
            .when(!open, |d| {
                d.border_b_1()
                    .border_color(theme.separator)
                    .hover(|s| s.bg(theme.list_bg_hovered))
            })
            .on_click(move |_, _, cx| {
                let key = key.clone();
                toggle.update(cx, |this, cx| this.toggle_expanded(key, cx));
            });
        if let Some(entry) = source.entry.clone() {
            let stage = entity.clone();
            let state = if entry.staged {
                CheckState::All
            } else {
                CheckState::Off
            };
            header = header.child(
                checkbox(
                    ElementId::NamedInteger("diff-card-check".into(), ix as u64),
                    state,
                    theme,
                    m,
                )
                .size(px(m.diff_checkbox))
                .on_click(move |_, _, cx| {
                    let entry = entry.clone();
                    stage.update(cx, |this, cx| this.stage_toggle(entry, cx));
                }),
            );
        } else {
            header = header.child(div().size(px(m.diff_checkbox)));
        }
        header = header
            .child(
                div()
                    .ml(px(icon_x - check_x - m.diff_checkbox))
                    .size(px(m.icon_size))
                    .flex_shrink_0()
                    .child(icon(file_icon(&name), m.icon_size, theme.icon_secondary)),
            )
            .child(
                div()
                    .ml(px(m.diff_card_name_x - icon_x - m.icon_size))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(m.git_old_name_gap))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(div().text_color(name_color).child(name))
                    .children(old_name.map(|o| {
                        div()
                            .text_color(theme.text_secondary)
                            .line_through()
                            .child(o)
                    }))
                    .children(folder.map(|f| div().text_color(theme.text_secondary).child(f))),
            );
        if let Some(path) = source.path.clone() {
            let open_entity = entity.clone();
            header = header.child(
                div()
                    .id(ElementId::NamedInteger("diff-edit".into(), ix as u64))
                    .size(px(m.button_height))
                    .mr(px(m.diff_pencil_gap))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(m.button_radius))
                    .hover(|s| s.bg(theme.ghost_hovered))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        let path = path.clone();
                        open_entity.update(cx, |_, cx| cx.emit(DiffViewEvent::Open(path)));
                    })
                    .child(icon(
                        "expui/general/edit",
                        m.icon_size,
                        theme.icon_secondary,
                    )),
            );
        }
        if let Some(entry) = source.entry.clone() {
            let revert = entity.clone();
            header = header.child(
                crate::dialog::button(
                    ElementId::NamedInteger("diff-revert".into(), ix as u64),
                    "Revert".into(),
                    crate::dialog::ButtonKind::Secondary,
                    false,
                    theme,
                    m,
                )
                .w(px(m.diff_revert_width))
                .mr(px(m.diff_card_inset / 6.))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    let entry = entry.clone();
                    revert.update(cx, |_, cx| cx.emit(DiffViewEvent::Revert(entry)));
                }),
            );
        } else {
            header = header.child(div().w(px(m.diff_card_inset)));
        }

        let mut card = div().flex_shrink_0().flex().flex_col().child(header);
        if open {
            card = card.child(
                div()
                    .h(px(m.diff_strip))
                    .flex_shrink_0()
                    .bg(theme.background_secondary),
            );
            let body = div()
                .flex()
                .flex_col()
                .font_family(theme.mono_font.clone())
                .text_size(px(font_size))
                .line_height(px(m.diff_row))
                .children(
                    rows.into_iter()
                        .flatten()
                        .enumerate()
                        .map(|(r, row)| self.render_row(ix, r, row, theme, m, cx)),
                );
            card = card.child(body);
        }
        card.into_any_element()
    }

    /// One row of the side-by-side diff, or the wavy fold between runs.
    fn render_row(
        &self,
        card: usize,
        r: usize,
        row: &DiffRow,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let id = ElementId::NamedInteger(format!("diff-row-{card}").into(), r as u64);
        match row {
            DiffRow::Collapsed => {
                let color = theme.separator;
                div()
                    .id(id)
                    .h(px(m.diff_row))
                    .flex_shrink_0()
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, _| paint_wave(bounds, color, window),
                        )
                        .size_full(),
                    )
                    .into_any_element()
            }
            DiffRow::Pair { old, new, changed } => {
                let wash: Option<Hsla> = if !changed {
                    None
                } else if new.is_some() && old.is_some() {
                    Some(theme.diff_modified_bg)
                } else if new.is_some() {
                    Some(theme.diff_added_bg)
                } else {
                    Some(theme.diff_deleted_bg)
                };
                let text_of = new
                    .as_ref()
                    .or(old.as_ref())
                    .map(|s| s.text.to_string())
                    .unwrap_or_default();
                let menu = cx.entity();
                let entry = self.sources[card].entry.clone();
                let stage = cx.entity();
                let number = |side: &Side| {
                    div()
                        .w(px(m.diff_number_width))
                        .flex_shrink_0()
                        .text_color(theme.line_number)
                        .font_family(theme.mono_font.clone())
                        .flex()
                        .justify_end()
                        .child(SharedString::from(side.number.to_string()))
                };
                let text = |side: &Side| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(theme.editor_text)
                        .child(StyledText::new(side.text.clone()).with_runs(side.runs.clone()))
                };
                let old_cell = div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .pl(px(m.diff_old_text_x))
                    .when_some(wash, |d, w| d.bg(w))
                    .map(|d| match old {
                        Some(side) => d
                            .child(text(side))
                            .child(number(side).mr(px(m.diff_old_number_right))),
                        None => d.child(div().flex_1()).when(*changed, |d| {
                            d.child(
                                div()
                                    .w(px(m.diff_number_width))
                                    .mr(px(m.diff_old_number_right))
                                    .flex()
                                    .justify_end()
                                    .text_color(theme.text_secondary)
                                    .child("»"),
                            )
                        }),
                    });
                let new_cell = div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .when_some(wash, |d, w| d.bg(w))
                    .map(|d| match new {
                        Some(side) => {
                            let column = div()
                                .ml(px(m.diff_new_number_right - m.diff_number_width))
                                .w(px(m.diff_number_width))
                                .flex_shrink_0()
                                .flex()
                                .justify_end()
                                .items_center()
                                .h(px(m.diff_row));
                            let column = match (changed, entry.clone()) {
                                // The staged checkbox on a changed row (it
                                // stages the whole file; hunks are not
                                // staged on their own, estimate).
                                (true, Some(entry)) => column.child(
                                    checkbox(
                                        ElementId::NamedInteger(
                                            format!("diff-hunk-{card}").into(),
                                            r as u64,
                                        ),
                                        if entry.staged {
                                            CheckState::All
                                        } else {
                                            CheckState::Off
                                        },
                                        theme,
                                        m,
                                    )
                                    .size(px(m.diff_checkbox))
                                    .on_click(
                                        move |_, _, cx| {
                                            let entry = entry.clone();
                                            stage.update(cx, |this, cx| {
                                                this.stage_toggle(entry, cx)
                                            });
                                        },
                                    ),
                                ),
                                _ => column
                                    .text_color(theme.line_number)
                                    .child(SharedString::from(side.number.to_string())),
                            };
                            d.child(column).child(
                                text(side).ml(px(m.diff_new_text_x - m.diff_new_number_right)),
                            )
                        }
                        None => d,
                    });
                div()
                    .id(id)
                    .min_h(px(m.diff_row))
                    .flex_shrink_0()
                    .flex()
                    .items_stretch()
                    .on_mouse_down(MouseButton::Right, move |event, _, cx| {
                        let position = event.position;
                        let text = text_of.clone();
                        menu.update(cx, |_, cx| {
                            cx.emit(DiffViewEvent::ContextMenu { position, text })
                        });
                    })
                    .child(old_cell)
                    .child(
                        div()
                            .w(px(m.diff_separator))
                            .flex_shrink_0()
                            .bg(theme.separator),
                    )
                    .child(new_cell)
                    .into_any_element()
            }
        }
    }
}

/// The fold between unchanged runs: a 1 px zigzag across the row's middle
/// (Fleet's wavy separator; its amplitude and period are estimates).
fn paint_wave(bounds: gpui::Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let step = px(4.);
    let amplitude = px(1.5);
    let thickness = px(1.);
    let mid = bounds.origin.y + bounds.size.height / 2.;
    let right = bounds.origin.x + bounds.size.width;
    let mut x = bounds.origin.x;
    let mut up = true;
    let mut top = Vec::new();
    while x <= right {
        top.push(point(x, if up { mid - amplitude } else { mid + amplitude }));
        x += step;
        up = !up;
    }
    if top.len() < 2 {
        return;
    }
    let mut path = GpuiPath::new(top[0]);
    for p in &top[1..] {
        path.line_to(*p);
    }
    for p in top.iter().rev() {
        path.line_to(point(p.x, p.y + thickness));
    }
    window.paint_path(path, color);
}
