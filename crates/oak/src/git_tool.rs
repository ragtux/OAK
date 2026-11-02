//! The Git tool and the History tool in the panel (M4, batch 10:
//! `docs/fleet-probe.md`, ninth pass). The Git tool is two stacked blocks:
//! the commit message field, the Commit button, the "Changed files" header
//! with its checkbox and one 26 px row per changed file (status glyph, icon,
//! name, a checkbox for the index), then under a 2 px gap "Unsynced
//! commits", the Push button, the branch chip and the commits. The History
//! tool is a "Search commits" field, a filter row of three chips and the
//! same commit rows. Both read through `oak-git`.

use crate::assets::{file_icon, icon};
use crate::input::{InputEvent, TextInput};
use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, App, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, ScrollHandle, SharedString, Window,
};
use oak_git::{Commit, Repo, Status, StatusEntry};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum GitToolEvent {
    /// Files were staged, unstaged or committed: the workspace re-reads git.
    Changed,
    /// A row was double-clicked: the Changed Files tab on the file (ninth pass).
    Diff(PathBuf),
    /// The Push button.
    Push,
    /// The filter button, at this window position: the workspace opens its menu.
    Filter(gpui::Point<gpui::Pixels>),
    /// The branch chip, at this window position: the branches menu.
    Branches(gpui::Point<gpui::Pixels>),
}

/// The filter button's choice: which changed files the list shows
/// (estimate; the ninth pass did not open the menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GitFilter {
    #[default]
    All,
    Staged,
    Unstaged,
}

impl GitFilter {
    pub const ALL: [GitFilter; 3] = [GitFilter::All, GitFilter::Staged, GitFilter::Unstaged];

    pub fn label(self) -> &'static str {
        match self {
            GitFilter::All => "All changes",
            GitFilter::Staged => "Staged only",
            GitFilter::Unstaged => "Unstaged only",
        }
    }

    fn admits(self, entry: &StatusEntry) -> bool {
        match self {
            GitFilter::All => true,
            GitFilter::Staged => entry.staged,
            GitFilter::Unstaged => !entry.staged,
        }
    }
}

pub struct GitTool {
    repo: Option<Repo>,
    pub message: Entity<TextInput>,
    entries: Vec<StatusEntry>,
    branch: Option<String>,
    /// The upstream's name, for the Push button's label.
    upstream: Option<String>,
    /// Commits not on the upstream (every commit without one).
    unsynced: usize,
    commits: Vec<Commit>,
    error: Option<String>,
    /// The row a click selected (Fleet's `git-tool-selected` shot).
    selected: Option<PathBuf>,
    pub filter: GitFilter,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<GitToolEvent> for GitTool {}

impl Focusable for GitTool {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.message.read(cx).focus_handle(cx)
    }
}

/// The commits the History tool lists at most.
const HISTORY_LIMIT: usize = 200;

impl GitTool {
    pub fn new(repo: Option<Repo>, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| TextInput::new("Commit message, press Ctrl+Enter to commit", cx));
        cx.subscribe(&message, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Submit { .. } = event {
                this.commit(cx);
            }
        })
        .detach();
        let mut tool = GitTool {
            repo,
            message,
            entries: Vec::new(),
            branch: None,
            upstream: None,
            unsynced: 0,
            commits: Vec::new(),
            error: None,
            selected: None,
            filter: GitFilter::All,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        };
        tool.refresh(cx);
        tool
    }

    /// Reads the status, the branch, the unsynced count and the commits again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.entries = repo.status_entries().unwrap_or_default();
        self.entries.sort_by(|a, b| {
            a.path
                .file_name()
                .cmp(&b.path.file_name())
                .then(a.path.cmp(&b.path))
        });
        self.branch = repo.branch();
        self.upstream = repo.upstream();
        self.unsynced = match repo.ahead_behind() {
            Some(ab) => ab.ahead,
            None => repo.commit_count(),
        };
        self.commits = repo.history(None, HISTORY_LIMIT).unwrap_or_default();
        if self
            .selected
            .as_ref()
            .is_some_and(|s| !self.entries.iter().any(|e| &e.path == s))
        {
            self.selected = None;
        }
        cx.notify();
    }

    pub fn set_filter(&mut self, filter: GitFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        cx.notify();
    }

    /// The Push button's label: "Push 1 commit to origin/main", or "to a
    /// new remote branch" without an upstream (the ninth pass; the upstream
    /// wording is an estimate).
    pub fn push_label(&self) -> String {
        let target = match &self.upstream {
            Some(u) => u.clone(),
            None => "a new remote branch".to_string(),
        };
        format!(
            "Push {} commit{} to {target}",
            self.unsynced,
            if self.unsynced == 1 { "" } else { "s" }
        )
    }

    /// Shows what a push or pull reported, or failed with.
    pub fn report(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        self.error = result.err();
        cx.notify();
    }

    fn fail(&mut self, e: anyhow::Error, cx: &mut Context<Self>) {
        self.error = Some(format!("{e:#}"));
        cx.notify();
    }

    fn toggle(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let Some(entry) = self.entries.iter().find(|e| e.path == path) else {
            return;
        };
        let result = if entry.staged {
            repo.unstage(&[path])
        } else {
            repo.stage(&[path])
        };
        match result {
            Ok(()) => cx.emit(GitToolEvent::Changed),
            Err(e) => self.fail(e, cx),
        }
    }

    fn toggle_all(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let all_staged = self.entries.iter().all(|e| e.staged);
        let paths: Vec<PathBuf> = self.entries.iter().map(|e| e.path.clone()).collect();
        let result = if all_staged {
            repo.unstage(&paths)
        } else {
            repo.stage(&paths)
        };
        match result {
            Ok(()) => cx.emit(GitToolEvent::Changed),
            Err(e) => self.fail(e, cx),
        }
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let message = self.message.read(cx).text().trim().to_string();
        if message.is_empty() || !self.entries.iter().any(|e| e.staged) {
            return;
        }
        match repo.commit(&message) {
            Ok(()) => {
                self.message.update(cx, |m, cx| m.set_text_at_end("", cx));
                self.error = None;
                cx.emit(GitToolEvent::Changed);
            }
            Err(e) => self.fail(e, cx),
        }
    }
}

/// The glyph and colour Fleet puts before a changed file's name.
fn status_glyph(status: Status, theme: &Theme) -> (&'static str, gpui::Hsla) {
    match status {
        Status::Added => ("+", theme.icon_green),
        Status::Untracked => ("?", theme.icon_green),
        Status::Modified | Status::Renamed => ("●", theme.icon_blue),
        Status::Deleted => ("−", theme.icon_red),
        Status::Conflict => ("!", theme.icon_yellow),
        Status::Ignored => ("", theme.text_secondary),
    }
}

pub(crate) fn status_text_color(status: Status, theme: &Theme) -> gpui::Hsla {
    match status {
        Status::Added | Status::Untracked => theme.git_added_text,
        Status::Modified | Status::Renamed => theme.git_modified_text,
        Status::Deleted => theme.git_deleted_text,
        Status::Conflict => theme.git_conflict_text,
        Status::Ignored => theme.text_secondary,
    }
}

/// Fleet's 14.67 px checkbox in the Git tool's rows.
pub(crate) fn checkbox(
    id: impl Into<ElementId>,
    state: CheckState,
    theme: &Theme,
    m: &Metrics,
) -> gpui::Stateful<gpui::Div> {
    let on = state != CheckState::Off;
    div()
        .id(id)
        .size(px(m.git_checkbox))
        .flex_shrink_0()
        .rounded(px(3. * m.zoom))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .map(|d| {
            if on {
                d.bg(theme.checkbox_on_bg)
            } else {
                d.bg(theme.checkbox_off_bg)
                    .border_1()
                    .border_color(theme.checkbox_off_border)
            }
        })
        .when(on, |d| {
            d.text_color(theme.checkbox_icon)
                .text_size(px(10. * m.zoom))
                .font_weight(FontWeight::BOLD)
                .child(if state == CheckState::Some {
                    "–"
                } else {
                    "✓"
                })
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckState {
    Off,
    Some,
    All,
}

/// One commit as both tools list it: a dot, the summary, then the author
/// and date in secondary text (ninth pass: 40 px blocks, "Probe, 18:00 Dec
/// 31, 2025").
fn commit_row(ix: usize, commit: &Commit, theme: &Theme, m: &Metrics) -> impl IntoElement {
    let date = oak_git::format_iso(&commit.date);
    div()
        .id(ElementId::NamedInteger("commit".into(), ix as u64))
        .h(px(m.git_commit_row))
        .flex_shrink_0()
        .relative()
        .pl(px(m.git_commit_text_x))
        .flex()
        .flex_col()
        .justify_center()
        .child(
            div()
                .absolute()
                .left(px(m.git_commit_dot_x))
                .top(px((m.git_commit_line - m.git_commit_dot) / 2.))
                .size(px(m.git_commit_dot))
                .rounded_full()
                .bg(theme.text_tertiary),
        )
        .child(
            div()
                .h(px(m.git_commit_line))
                .flex()
                .items_center()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(theme.text_primary)
                .child(SharedString::from(commit.summary.clone())),
        )
        .child(
            div()
                .h(px(m.git_commit_line))
                .flex()
                .items_center()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(theme.text_secondary)
                .child(SharedString::from(format!("{}, {date}", commit.author))),
        )
}

impl Render for GitTool {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let entity = cx.entity();
        let focused = self.message.read(cx).is_focused(window);
        let can_commit = !self.message.read(cx).text().trim().is_empty()
            && self.entries.iter().any(|e| e.staged);
        let staged = self.entries.iter().filter(|e| e.staged).count();
        let all = match staged {
            0 => CheckState::Off,
            n if n == self.entries.len() => CheckState::All,
            _ => CheckState::Some,
        };
        let filter = self.filter;
        let selected = self.selected.clone();
        let rows: Vec<_> = self
            .entries
            .iter()
            .filter(|e| filter.admits(e))
            .enumerate()
            .map(|(ix, e)| {
                let name: SharedString = e
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
                    .into();
                let old: Option<SharedString> = e.old_path.as_ref().map(|p| {
                    format!(
                        "({})",
                        p.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default()
                    )
                    .into()
                });
                let (glyph, glyph_color) = status_glyph(e.status, &theme);
                let path = e.path.clone();
                let toggle_path = e.path.clone();
                let open = entity.clone();
                let toggle = entity.clone();
                let is_selected = selected.as_ref() == Some(&e.path);
                div()
                    .id(ElementId::NamedInteger("git-row".into(), ix as u64))
                    .h(px(m.git_row))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .rounded(px(m.popup_row_radius))
                    .when(is_selected, |d| d.bg(theme.list_bg_selected))
                    .when(!is_selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                    .on_click(move |event, _, cx| {
                        open.update(cx, |this, cx| {
                            this.selected = Some(path.clone());
                            if event.click_count() == 2 {
                                cx.emit(GitToolEvent::Diff(path.clone()));
                            }
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .w(px(m.git_glyph_x))
                            .flex_shrink_0()
                            .flex()
                            .justify_end()
                            .text_color(glyph_color)
                            .child(glyph),
                    )
                    .child(
                        div()
                            .ml(px(m.git_icon_x - m.git_glyph_x - m.icon_size / 2.))
                            .size(px(m.icon_size))
                            .flex_shrink_0()
                            .child(icon(file_icon(&name), m.icon_size, theme.icon_secondary)),
                    )
                    .child(
                        div()
                            .ml(px(m.git_name_x - m.git_icon_x - m.icon_size / 2.))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(m.git_old_name_gap))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_color(status_text_color(e.status, &theme))
                            .child(name.clone())
                            .children(old.map(|o| {
                                div()
                                    .text_color(theme.text_secondary)
                                    .line_through()
                                    .child(o)
                            })),
                    )
                    .child(
                        div().pr(px(m.git_checkbox_right)).child(
                            checkbox(
                                ElementId::NamedInteger("git-check".into(), ix as u64),
                                if e.staged {
                                    CheckState::All
                                } else {
                                    CheckState::Off
                                },
                                &theme,
                                &m,
                            )
                            .on_click(move |_, _, cx| {
                                toggle.update(cx, |this, cx| this.toggle(toggle_path.clone(), cx))
                            }),
                        ),
                    )
            })
            .collect();
        let commits: Vec<_> = self
            .commits
            .iter()
            .enumerate()
            .map(|(ix, c)| commit_row(ix, c, &theme, &m))
            .collect();
        let count: SharedString = self.entries.len().to_string().into();
        let branch: SharedString = self.branch.clone().unwrap_or_default().into();
        let push_label: SharedString = self.push_label().into();
        let commit_entity = entity.clone();
        let all_entity = entity.clone();
        let filter_entity = entity.clone();
        let push_entity = entity.clone();
        let chip_entity = entity.clone();
        div()
            .key_context("GitTool")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            // The commit field, 24 tall, 6 inside the island.
            .child(
                div()
                    .flex_shrink_0()
                    .mt(px(m.git_field_top))
                    .px(px(m.panel_pad_x))
                    .child(
                        crate::search_tool::frame(focused, &theme, &m)
                            .h(px(m.git_field_height))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .child(self.message.clone()),
                            ),
                    ),
            )
            // Commit and the filter button.
            .child(
                div()
                    .flex_shrink_0()
                    .mt(px(m.git_button_gap))
                    .px(px(m.panel_pad_x))
                    .flex()
                    .items_center()
                    .gap(px(m.git_button_gap))
                    .child(
                        crate::dialog::button(
                            "git-commit",
                            "Commit".into(),
                            crate::dialog::ButtonKind::Secondary,
                            false,
                            &theme,
                            &m,
                        )
                        .when(!can_commit, |d| d.text_color(theme.text_secondary))
                        .on_click(move |_, _, cx| {
                            commit_entity.update(cx, |this, cx| this.commit(cx))
                        }),
                    )
                    .child(
                        div()
                            .id("git-filter")
                            .size(px(m.button_height))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(m.button_radius))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.ghost_hovered))
                            .when(filter != GitFilter::All, |d| d.bg(theme.ghost_hovered))
                            .on_click(move |event, _, cx| {
                                let at = event.position();
                                filter_entity.update(cx, |_, cx| cx.emit(GitToolEvent::Filter(at)));
                            })
                            .child(icon(
                                "expui/general/filter",
                                m.icon_size,
                                theme.icon_primary,
                            )),
                    ),
            )
            .children(self.error.clone().map(|e| {
                div()
                    .px(px(m.panel_pad_x))
                    .mt(px(m.git_button_gap))
                    .text_color(theme.text_dangerous)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(SharedString::from(e))
            }))
            // "Changed files" header: a 24 px washed row.
            .child(
                div()
                    .flex_shrink_0()
                    .mt(px(m.git_header_gap))
                    .mx(px(m.panel_pad_x))
                    .h(px(m.git_header_height))
                    .pl(px(m.git_header_pad_x))
                    .flex()
                    .items_center()
                    .rounded(px(m.popup_row_radius))
                    .bg(theme.list_bg_selected)
                    .child(div().text_color(theme.text_primary).child("Changed files"))
                    .child(
                        div()
                            .ml(px(m.git_count_gap))
                            .text_color(theme.text_secondary)
                            .child(count),
                    )
                    .child(div().flex_1())
                    .child(div().pr(px(m.git_checkbox_right)).child(
                        checkbox("git-check-all", all, &theme, &m).on_click(move |_, _, cx| {
                            all_entity.update(cx, |this, cx| this.toggle_all(cx))
                        }),
                    )),
            )
            .child(
                div()
                    .id("git-rows")
                    .flex_1()
                    .flex_basis(px(0.))
                    .min_h_0()
                    .mt(px(m.git_rows_gap))
                    .px(px(m.panel_pad_x))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            // The island's second block, under a 2 px gap in the window's colour.
            .child(
                div()
                    .h(px(m.git_split_gap))
                    .flex_shrink_0()
                    .bg(theme.background_primary),
            )
            .child(
                div()
                    .flex_1()
                    // Fleet halves the island: the lower block starts with the
                    // upper chrome's height so the two flex shares come out equal.
                    .flex_basis(px(m.git_field_top
                        + m.git_field_height
                        + m.git_button_gap
                        + m.button_height
                        + m.git_header_gap
                        + m.git_header_height
                        + m.git_rows_gap))
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_shrink_0()
                            .mt(px(m.git_unsynced_top))
                            .px(px(m.panel_pad_x + m.git_lower_x))
                            .h(px(m.git_header_height))
                            .flex()
                            .items_center()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_primary)
                            .child(SharedString::from(format!(
                                "Unsynced commits ↑{}",
                                self.unsynced
                            ))),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .mt(px(m.git_push_gap))
                            .ml(px(m.panel_pad_x + m.git_lower_x))
                            .flex()
                            .child(
                                crate::dialog::button(
                                    "git-push",
                                    push_label,
                                    crate::dialog::ButtonKind::Secondary,
                                    false,
                                    &theme,
                                    &m,
                                )
                                .h(px(m.git_push_height))
                                .text_color(theme.text_secondary)
                                .on_click(move |_, _, cx| {
                                    push_entity.update(cx, |_, cx| cx.emit(GitToolEvent::Push))
                                }),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .mt(px(m.git_chip_gap))
                            .ml(px(m.panel_pad_x + m.git_chip_x))
                            .flex()
                            .child(
                                div()
                                    .id("git-branch-chip")
                                    .h(px(m.git_chip_height))
                                    .px(px(m.git_chip_pad_x))
                                    .flex()
                                    .items_center()
                                    .gap(px(m.git_chip_pad_x / 4.))
                                    .rounded(px(m.button_radius))
                                    .border_1()
                                    .border_color(theme.git_branch_tag_border)
                                    .text_color(theme.git_branch_tag_text)
                                    .cursor_pointer()
                                    .on_click(move |event, _, cx| {
                                        let at = event.position();
                                        chip_entity.update(cx, |_, cx| {
                                            cx.emit(GitToolEvent::Branches(at))
                                        });
                                    })
                                    .child(branch)
                                    .child(icon(
                                        "expui/general/chevronDown",
                                        m.icon_size * 0.75,
                                        theme.git_branch_tag_text,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .id("git-commits")
                            .flex_1()
                            .min_h_0()
                            .mt(px(m.git_commits_gap))
                            .px(px(m.panel_pad_x))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .children(commits),
                    ),
            )
    }
}

// ---- the History tool --------------------------------------------------------

/// Which of the History tool's three filter chips was clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryChip {
    Branch,
    Scope,
    Author,
}

#[derive(Clone, Debug)]
pub enum HistoryToolEvent {
    /// A chip was clicked at this window position: the workspace opens its menu.
    Chip(HistoryChip, gpui::Point<gpui::Pixels>),
}

pub struct HistoryTool {
    repo: Option<Repo>,
    pub search: Entity<TextInput>,
    /// `git/file-history`: only this file's commits.
    file: Option<PathBuf>,
    /// The checked-out branch, and the one the branch chip chose instead.
    branch: Option<String>,
    chosen_branch: Option<String>,
    /// The author chip's choice; None is "all users".
    author: Option<String>,
    commits: Vec<Commit>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<HistoryToolEvent> for HistoryTool {}

impl Focusable for HistoryTool {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
}

impl HistoryTool {
    pub fn new(repo: Option<Repo>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new("Search commits", cx));
        cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if let InputEvent::Changed = event {
                cx.notify();
            }
        })
        .detach();
        let mut tool = HistoryTool {
            repo,
            search,
            file: None,
            branch: None,
            chosen_branch: None,
            author: None,
            commits: Vec::new(),
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        };
        tool.refresh(cx);
        tool
    }

    pub fn set_file(&mut self, file: Option<PathBuf>, cx: &mut Context<Self>) {
        self.file = file;
        self.refresh(cx);
    }

    /// The branch chip's choice; None goes back to the checked-out branch.
    pub fn set_branch(&mut self, branch: Option<String>, cx: &mut Context<Self>) {
        self.chosen_branch = branch;
        self.refresh(cx);
    }

    /// The author chip's choice; None is every author.
    pub fn set_author(&mut self, author: Option<String>, cx: &mut Context<Self>) {
        self.author = author;
        cx.notify();
    }

    /// Every author among the listed commits, for the chip's menu.
    pub fn authors(&self) -> Vec<String> {
        let mut authors: Vec<String> = self.commits.iter().map(|c| c.author.clone()).collect();
        authors.sort();
        authors.dedup();
        authors
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.branch = repo.branch();
        self.commits = repo
            .history_of(
                self.chosen_branch.as_deref(),
                self.file.as_deref(),
                HISTORY_LIMIT,
            )
            .unwrap_or_default();
        cx.notify();
    }
}

impl Render for HistoryTool {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let entity = cx.entity();
        let focused = self.search.read(cx).is_focused(window);
        let query = self.search.read(cx).text().to_lowercase();
        let author = self.author.clone();
        let commits: Vec<_> = self
            .commits
            .iter()
            .filter(|c| query.is_empty() || c.summary.to_lowercase().contains(&query))
            .filter(|c| author.as_ref().is_none_or(|a| &c.author == a))
            .enumerate()
            .map(|(ix, c)| commit_row(ix, c, &theme, &m))
            .collect();
        let scope: SharedString = match &self.file {
            Some(f) => f
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
                .into(),
            None => self
                .repo
                .as_ref()
                .and_then(|r| r.root.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_default()
                .into(),
        };
        let chip = |which: HistoryChip, id: &'static str, label: SharedString| {
            let entity = entity.clone();
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(m.git_chip_pad_x / 2.))
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(theme.text_secondary)
                .cursor_pointer()
                .on_click(move |event, _, cx| {
                    let at = event.position();
                    entity.update(cx, |_, cx| cx.emit(HistoryToolEvent::Chip(which, at)));
                })
                .child(label)
                .child(icon(
                    "expui/general/chevronDown",
                    m.icon_size,
                    theme.icon_secondary,
                ))
        };
        let branch_label: SharedString = self
            .chosen_branch
            .clone()
            .or_else(|| self.branch.clone())
            .unwrap_or_default()
            .into();
        let author_label: SharedString = self
            .author
            .clone()
            .unwrap_or_else(|| "all users".to_string())
            .into();
        div()
            .key_context("HistoryTool")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            .child(
                div()
                    .flex_shrink_0()
                    .mt(px(m.git_field_top))
                    .px(px(m.panel_pad_x))
                    .child(
                        crate::search_tool::frame(focused, &theme, &m)
                            .h(px(m.git_field_height))
                            .gap(px(m.popup_row_icon_gap))
                            .child(icon(
                                "expui/general/search",
                                m.icon_size,
                                theme.icon_secondary,
                            ))
                            .child(div().flex_1().min_w_0().h_full().child(self.search.clone())),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .mt(px(m.history_filter_gap))
                    .h(px(m.git_header_height))
                    .px(px(m.panel_pad_x + m.git_header_pad_x))
                    .flex()
                    .items_center()
                    .gap(px(m.history_chip_gap))
                    .child(chip(HistoryChip::Branch, "history-branch", branch_label))
                    .child(div().max_w(px(m.history_scope_max_width)).child(chip(
                        HistoryChip::Scope,
                        "history-scope",
                        scope,
                    )))
                    .child(chip(HistoryChip::Author, "history-author", author_label)),
            )
            .child(
                div()
                    .id("history-commits")
                    .flex_1()
                    .min_h_0()
                    .mt(px(m.history_rows_gap))
                    .px(px(m.panel_pad_x))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .children(commits),
            )
    }
}
