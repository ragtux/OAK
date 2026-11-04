//! The Git tool and the History tool in the panel (M4, batch 10:
//! `docs/fleet-probe.md`, ninth pass). The Git tool is two stacked blocks:
//! the commit message field, the Commit button, the "Changed files" header
//! with its checkbox and one 26 px row per changed file (status glyph, icon,
//! name, a checkbox for the index), then under a 2 px gap "Unsynced
//! commits", the Push button, the branch chip and the commits. The History
//! tool is a "Search commits" field, a filter row of three chips and the
//! same commit rows. Both read through `weep-git`.

use crate::assets::{file_icon, icon};
use crate::input::{InputEvent, TextInput};
use crate::skin::{self, Cell, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, App, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, ScrollHandle, SharedString, Window,
};
use std::path::PathBuf;
use std::rc::Rc;
use weep_git::{Commit, Repo, Status, StatusEntry};

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
    /// The commits' parents, read for the tui skin's graph.
    parents: Parents,
    error: Option<String>,
    /// The row a click selected (Fleet's `git-tool-selected` shot).
    selected: Option<PathBuf>,
    pub filter: GitFilter,
    /// A refresh is reading on the background executor; one asked for
    /// meanwhile runs after it.
    refreshing: bool,
    refresh_again: bool,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// The tui skin's body in whole cells, as last laid out.
    tui_grid: Rc<std::cell::Cell<(usize, usize)>>,
}

impl EventEmitter<GitToolEvent> for GitTool {}

/// What the Git tool reads through `weep-git`: gathered off the UI thread,
/// as each `git` call takes seconds on a network share.
struct GitToolData {
    entries: Vec<StatusEntry>,
    branch: Option<String>,
    upstream: Option<String>,
    unsynced: usize,
    commits: Vec<Commit>,
    /// Each commit's parents, for the tui skin's graph.
    parents: Parents,
}

impl GitToolData {
    fn load(repo: &Repo) -> GitToolData {
        let commits = repo.history(None, HISTORY_LIMIT).unwrap_or_default();
        let parents = parents_of(&commits);
        let mut entries = repo.status_entries().unwrap_or_default();
        entries.sort_by(|a, b| {
            a.path
                .file_name()
                .cmp(&b.path.file_name())
                .then(a.path.cmp(&b.path))
        });
        GitToolData {
            entries,
            branch: repo.branch(),
            upstream: repo.upstream(),
            unsynced: match repo.ahead_behind() {
                Some(ab) => ab.ahead,
                None => repo.commit_count(),
            },
            commits,
            parents,
        }
    }
}

impl Focusable for GitTool {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.message.read(cx).focus_handle(cx)
    }
}

/// The commits the History tool lists at most.
const HISTORY_LIMIT: usize = 200;

impl GitTool {
    pub fn new(repo: Option<Repo>, cx: &mut Context<Self>) -> Self {
        // Fleet's placeholder names its secondary Enter: Ctrl on Windows and
        // Linux, Command on macOS.
        let chord = crate::keymap::chord_text("Ctrl+Enter", "cmd-return");
        let placeholder = format!("Commit message, press {chord} to commit");
        let message = cx.new(|cx| TextInput::new(placeholder, cx));
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
            parents: Parents::new(),
            error: None,
            selected: None,
            filter: GitFilter::All,
            refreshing: false,
            refresh_again: false,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            tui_grid: Rc::default(),
        };
        tool.refresh(cx);
        tool
    }

    /// Reads the status, the branch, the unsynced count and the commits
    /// again, on the background executor; the tool shows them once they
    /// have landed.
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
            let data = cx
                .background_spawn(async move { GitToolData::load(&repo) })
                .await;
            let _ = this.update(cx, |this, cx| this.apply(data, cx));
        })
        .detach();
    }

    fn apply(&mut self, data: GitToolData, cx: &mut Context<Self>) {
        self.entries = data.entries;
        self.branch = data.branch;
        self.upstream = data.upstream;
        self.unsynced = data.unsynced;
        self.commits = data.commits;
        self.parents = data.parents;
        if self
            .selected
            .as_ref()
            .is_some_and(|s| !self.entries.iter().any(|e| &e.path == s))
        {
            self.selected = None;
        }
        self.refreshing = false;
        if std::mem::take(&mut self.refresh_again) {
            self.refresh(cx);
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
    let date = weep_git::format_iso(&commit.date);
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
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(window, cx);
        }
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
            .into_any_element()
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
    /// The commits' parents, read for the tui skin's graph.
    parents: Parents,
    /// A refresh is reading on the background executor; one asked for
    /// meanwhile runs after it.
    refreshing: bool,
    refresh_again: bool,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// The tui skin's body in whole cells, as last laid out.
    tui_grid: Rc<std::cell::Cell<(usize, usize)>>,
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
            parents: Parents::new(),
            refreshing: false,
            refresh_again: false,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            tui_grid: Rc::default(),
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

    /// Reads the branch and the commits again on the background executor
    /// (a chip or file chosen meanwhile is read once these have landed).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.refreshing {
            self.refresh_again = true;
            return;
        }
        self.refreshing = true;
        let chosen_branch = self.chosen_branch.clone();
        let file = self.file.clone();
        // A file's history (`--follow`) is drawn as one lane.
        let graph = file.is_none();
        cx.spawn(async move |this, cx| {
            let (branch, commits, parents) = cx
                .background_spawn(async move {
                    let branch = repo.branch();
                    let commits = repo
                        .history_of(chosen_branch.as_deref(), file.as_deref(), HISTORY_LIMIT)
                        .unwrap_or_default();
                    let parents = if graph {
                        parents_of(&commits)
                    } else {
                        Parents::new()
                    };
                    (branch, commits, parents)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.branch = branch;
                this.commits = commits;
                this.parents = parents;
                this.refreshing = false;
                if std::mem::take(&mut this.refresh_again) {
                    this.refresh(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for HistoryTool {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(window, cx);
        }
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
            .into_any_element()
    }
}

// ---- the tui skin ------------------------------------------------------------

/// A changed file's status as one letter, as the Files tool marks it.
fn status_letter(status: Status) -> &'static str {
    match status {
        Status::Added => "A",
        Status::Untracked => "?",
        Status::Modified => "M",
        Status::Renamed => "R",
        Status::Deleted => "D",
        Status::Conflict => "U",
        Status::Ignored => "!",
    }
}

/// `text` in at most `cols` cells, cut with `…`.
fn fit_cols(text: &str, cols: usize) -> String {
    if text.chars().count() <= cols {
        return text.to_string();
    }
    if cols == 0 {
        return String::new();
    }
    let mut out: String = text.chars().take(cols - 1).collect();
    out.push('…');
    out
}

/// A commit's day as the tui rows print it, in six cells: `Nov 01`.
fn short_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = iso.split('T').next().unwrap_or(iso);
    let mut parts = date.split('-').skip(1);
    match (
        parts.next().and_then(|m| m.parse::<usize>().ok()),
        parts.next(),
    ) {
        (Some(m @ 1..=12), Some(d)) => format!("{} {d:0>2}", MONTHS[m - 1]),
        _ => fit_cols(date, 6),
    }
}

/// Each commit's parents by full id.
type Parents = std::collections::HashMap<String, Vec<String>>;

/// The graph's parents by commit id, from the history read.
fn parents_of(commits: &[Commit]) -> Parents {
    commits
        .iter()
        .map(|c| (c.id.clone(), c.parents.clone()))
        .collect()
}

/// The lanes the graph shows at most; more are clipped.
const MAX_LANES: usize = 6;

/// One commit's row of the graph, in lanes of two cells.
#[derive(Clone, Debug, Default, PartialEq)]
struct GraphRow {
    /// The commit's lane.
    col: usize,
    /// A line comes down into the commit from the row above.
    from_top: bool,
    /// The commit's first parent goes on down its lane.
    down: bool,
    /// Lanes that run straight through the row.
    pass: Vec<usize>,
    /// Lanes that end in the commit (other children of it): a diagonal
    /// from the lane's top to the commit.
    into: Vec<usize>,
    /// Lanes the commit's other parents leave on: a diagonal from the commit
    /// to the lane's bottom (a merge).
    out: Vec<usize>,
}

/// Lays `commits` (newest first) out in lanes by their parents, as `git log
/// --graph` does; a commit without known parents continues to the next row
/// (a history read without them is one lane). Returns the rows and the lane
/// count.
fn graph_rows(commits: &[&Commit], parents: &Parents) -> (Vec<GraphRow>, usize) {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let mut width = 1;
    let free = |lanes: &mut Vec<Option<String>>, skip: Option<usize>| -> usize {
        match (0..lanes.len()).find(|&l| lanes[l].is_none() && Some(l) != skip) {
            Some(l) => l,
            None => {
                lanes.push(None);
                lanes.len() - 1
            }
        }
    };
    for (ix, c) in commits.iter().enumerate() {
        let id = &c.id;
        let col = lanes
            .iter()
            .position(|l| l.as_ref() == Some(id))
            .unwrap_or_else(|| free(&mut lanes, None));
        let from_top = lanes[col].as_ref() == Some(id);
        let mut into = Vec::new();
        let mut pass = Vec::new();
        for (l, lane) in lanes.iter_mut().enumerate() {
            if l == col {
                continue;
            }
            match lane {
                Some(other) if other == id => {
                    into.push(l);
                    *lane = None;
                }
                Some(_) => pass.push(l),
                None => {}
            }
        }
        let ps: Vec<String> = match parents.get(id) {
            Some(ps) => ps.clone(),
            None => commits
                .get(ix + 1)
                .map(|next| vec![next.id.clone()])
                .unwrap_or_default(),
        };
        lanes[col] = ps.first().cloned();
        let mut out = Vec::new();
        for p in ps.iter().skip(1) {
            let l = match lanes.iter().position(|l| l.as_ref() == Some(p)) {
                Some(l) => l,
                None => {
                    let l = free(&mut lanes, Some(col));
                    lanes[l] = Some(p.clone());
                    l
                }
            };
            out.push(l);
        }
        width = width.max(lanes.len()).max(col + 1);
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        rows.push(GraphRow {
            col,
            from_top,
            down: !ps.is_empty(),
            pass,
            into,
            out,
        });
    }
    (rows, width.min(MAX_LANES))
}

/// A line from `a` to `b` `t` wide: a quad along it, so a diagonal keeps
/// the stroke's width.
fn paint_segment(
    window: &mut gpui::Window,
    a: (f32, f32),
    b: (f32, f32),
    t: f32,
    color: gpui::Hsla,
) {
    if (a.0 - b.0).abs() < 0.01 {
        let x = skin::snap(window, a.0) - t / 2.;
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        window.paint_quad(gpui::fill(
            gpui::Bounds::new(gpui::point(px(x), px(y0)), gpui::size(px(t), px(y1 - y0))),
            color,
        ));
        return;
    }
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let (nx, ny) = (-dy / len * t / 2., dx / len * t / 2.);
    let p = |x: f32, y: f32| gpui::point(px(x), px(y));
    let mut path = gpui::Path::new(p(a.0 + nx, a.1 + ny));
    path.line_to(p(b.0 + nx, b.1 + ny));
    path.line_to(p(b.0 - nx, b.1 - ny));
    path.line_to(p(a.0 - nx, a.1 - ny));
    path.line_to(p(a.0 + nx, a.1 + ny));
    window.paint_path(path, color);
}

/// The graph's cell columns for `lanes` lanes: two cells a lane.
fn graph_cols(lanes: usize) -> usize {
    2 * lanes.max(1)
}

/// A commit's row in the tui skin, `cols` cells wide: the graph (lines
/// through the cell centres, merges as diagonals, the commit's dot), then
/// `3aa57f3 subject author Nov 01` in columns.
#[allow(clippy::too_many_arguments)]
fn tui_commit_row(
    ix: usize,
    commit: &Commit,
    row: GraphRow,
    lanes: usize,
    cols: usize,
    cell: Cell,
    theme: &Theme,
    m: Metrics,
) -> gpui::AnyElement {
    let palette = [
        theme.icon_blue,
        theme.icon_green,
        theme.icon_yellow,
        theme.icon_red,
    ];
    let lane_color = move |l: usize| palette[l % palette.len()];
    let graph_w = graph_cols(lanes);
    let graph = gpui::canvas(
        |_, _, _| (),
        move |bounds: gpui::Bounds<gpui::Pixels>, _, window, _| {
            let t = skin::stroke(window, &m);
            let x0 = f32::from(bounds.origin.x);
            let top = f32::from(bounds.origin.y);
            let mid = top + cell.h / 2.;
            let bottom = top + cell.h;
            let x = |l: usize| x0 + l as f32 * 2. * cell.w + cell.w / 2.;
            let shown = |l: usize| l < lanes;
            window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                for &l in row.pass.iter().filter(|l| shown(**l)) {
                    paint_segment(window, (x(l), top), (x(l), bottom), t, lane_color(l));
                }
                for &l in &row.into {
                    paint_segment(window, (x(l), top), (x(row.col), mid), t, lane_color(l));
                }
                for &l in &row.out {
                    paint_segment(window, (x(row.col), mid), (x(l), bottom), t, lane_color(l));
                }
                let c = lane_color(row.col);
                if row.from_top {
                    paint_segment(window, (x(row.col), top), (x(row.col), mid), t, c);
                }
                if row.down {
                    paint_segment(window, (x(row.col), mid), (x(row.col), bottom), t, c);
                }
                let d = skin::snap(window, cell.w.min(cell.h) * 0.8);
                let cx0 = skin::snap(window, x(row.col) - d / 2.);
                let cy0 = skin::snap(window, mid - d / 2.);
                window.paint_quad(gpui::quad(
                    gpui::Bounds::new(gpui::point(px(cx0), px(cy0)), gpui::size(px(d), px(d))),
                    px(d / 2.),
                    c,
                    px(0.),
                    gpui::transparent_black(),
                    gpui::BorderStyle::Solid,
                ));
            });
        },
    )
    .w(cell.cols(graph_w))
    .h(px(cell.h))
    .flex_shrink_0();
    // Columns: graph, id 7, subject, author 12, date 6, a cell between
    // each; the author and then the date give way on a narrow panel.
    let author_w = if cols >= 44 + graph_w { 12 } else { 0 };
    let date_w = if cols >= 28 + graph_w { 6 } else { 0 };
    let fixed = graph_w
        + 7
        + 1
        + if author_w > 0 { author_w + 1 } else { 0 }
        + if date_w > 0 { date_w + 1 } else { 0 };
    let subject_w = cols.saturating_sub(fixed).max(1);
    let column = |text: String, w: usize, color| {
        div()
            .w(cell.cols(w))
            .ml(px(cell.w))
            .flex_shrink_0()
            .overflow_hidden()
            .text_color(color)
            .child(SharedString::from(text))
    };
    div()
        .id(ElementId::NamedInteger("tui-commit".into(), ix as u64))
        .h(px(cell.h))
        .flex_shrink_0()
        .flex()
        .whitespace_nowrap()
        .overflow_hidden()
        .hover(|s| s.bg(theme.list_bg_hovered))
        .child(graph)
        .child(
            div()
                .w(cell.cols(7))
                .flex_shrink_0()
                .overflow_hidden()
                .text_color(theme.text_secondary)
                .child(SharedString::from(commit.short_id.clone())),
        )
        .child(column(
            fit_cols(&commit.summary, subject_w),
            subject_w,
            theme.text_primary,
        ))
        .when(author_w > 0, |d| {
            d.child(column(
                fit_cols(&commit.author, author_w),
                author_w,
                theme.text_secondary,
            ))
        })
        .when(date_w > 0, |d| {
            d.child(column(
                short_date(&commit.date),
                date_w,
                theme.text_tertiary,
            ))
        })
        .into_any_element()
}

/// A `[ label ▾ ]` chip in whole cells that a hover fills.
fn tui_chip(
    id: impl Into<ElementId>,
    label: &str,
    theme: &Theme,
    cell: Cell,
) -> gpui::Stateful<gpui::Div> {
    let text = format!("[ {label} ▾ ]");
    div()
        .id(id.into())
        .flex_shrink_0()
        .w(cell.cols(Cell::text_cols(&text)))
        .h(px(cell.h))
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(theme.text_secondary)
        .hover(|s| s.bg(theme.list_bg_hovered).text_color(theme.text_primary))
        .child(SharedString::from(text))
}

impl GitTool {
    /// Stages the listed files, or unstages them when all are staged.
    fn toggle_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let all_staged = self
            .entries
            .iter()
            .filter(|e| paths.contains(&e.path))
            .all(|e| e.staged);
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

    /// The tool in the tui skin: the commit message on `input_bg`, the
    /// buttons, the changed files in sections (`── [x] Changes 3 ──`,
    /// `── [ ] Unversioned 1 ──`) with `[x] M ◆ name  dir` rows, then the
    /// unsynced commits as history rows under their own rule.
    fn render_tui(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let cols = self.tui_grid.get().0;
        let entity = cx.entity();
        let can_commit = !self.message.read(cx).text().trim().is_empty()
            && self.entries.iter().any(|e| e.staged);
        let filter = self.filter;
        let gap = || div().w(px(cell.w)).flex_shrink_0();

        // The commit message, a row on the input's colour.
        let message = div().h(px(cell.h)).flex_shrink_0().flex().child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(cell.h))
                .bg(theme.input_bg)
                .child(self.message.clone()),
        );
        let commit = entity.clone();
        let push = entity.clone();
        let filter_entity = entity.clone();
        let buttons = div()
            .h(px(cell.h))
            .flex_shrink_0()
            .flex()
            .child(
                skin::button("git-commit", "Commit", can_commit, &theme, cell)
                    .when(!can_commit, |d| d.text_color(theme.text_disabled))
                    .on_click(move |_, _, cx| commit.update(cx, |this, cx| this.commit(cx))),
            )
            .child(gap())
            .child(
                skin::button(
                    "git-push",
                    &format!("Push ↑{}", self.unsynced),
                    false,
                    &theme,
                    cell,
                )
                .when(self.unsynced == 0, |d| d.text_color(theme.text_disabled))
                .on_click(move |_, _, cx| push.update(cx, |_, cx| cx.emit(GitToolEvent::Push))),
            )
            .child(div().flex_1())
            .child(
                tui_chip(
                    "git-filter",
                    match filter {
                        GitFilter::All => "all",
                        GitFilter::Staged => "staged",
                        GitFilter::Unstaged => "unstaged",
                    },
                    &theme,
                    cell,
                )
                .on_click(move |event, _, cx| {
                    let at = event.position();
                    filter_entity.update(cx, |_, cx| cx.emit(GitToolEvent::Filter(at)));
                }),
            );
        let error = self.error.clone().map(|e| {
            div()
                .h(px(cell.h))
                .flex_shrink_0()
                .pl(px(cell.w))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(theme.text_dangerous)
                .child(SharedString::from(e))
        });

        // The two sections and their rows, in one scrolled list.
        let mut list: Vec<gpui::AnyElement> = Vec::new();
        let sections = [
            ("Changes", false, "git-check-changes"),
            ("Unversioned", true, "git-check-unversioned"),
        ];
        let mut ix = 0;
        for (title, untracked, check_id) in sections {
            let entries: Vec<&StatusEntry> = self
                .entries
                .iter()
                .filter(|e| (e.status == Status::Untracked) == untracked)
                .collect();
            if entries.is_empty() {
                continue;
            }
            let staged = entries.iter().filter(|e| e.staged).count();
            let mark = match staged {
                0 => "[ ]",
                n if n == entries.len() => "[x]",
                _ => "[-]",
            };
            let paths: Vec<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
            let toggle = entity.clone();
            list.push(
                div()
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .flex()
                    .child(
                        div()
                            .id(check_id)
                            .w(cell.cols(3))
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_color(theme.text_secondary)
                            .hover(|s| s.text_color(theme.text_primary))
                            .on_click(move |_, _, cx| {
                                let paths = paths.clone();
                                toggle.update(cx, |this, cx| this.toggle_paths(paths, cx))
                            })
                            .child(mark),
                    )
                    .child(div().w(px(cell.w)).flex_shrink_0())
                    .child(skin::section(
                        cell,
                        cols.saturating_sub(4),
                        vec![skin::title(
                            format!("{title} {}", entries.len()),
                            theme.text_primary,
                        )],
                        theme.border,
                        theme.island_background,
                        m,
                    ))
                    .into_any_element(),
            );
            for e in entries.into_iter().filter(|e| filter.admits(e)) {
                list.push(self.tui_file_row(ix, e, cell, &theme, cx));
                ix += 1;
            }
        }
        if list.is_empty() {
            list.push(
                div()
                    .h(px(cell.h))
                    .pl(px(cell.w))
                    .text_color(theme.text_tertiary)
                    .child(if self.repo.is_some() {
                        "No changes"
                    } else {
                        "Not a git repository"
                    })
                    .into_any_element(),
            );
        }

        // Unsynced commits under a rule, the branch chip set in at its end.
        let branch: SharedString = self.branch.clone().unwrap_or_default().into();
        let chip_entity = entity.clone();
        let unsynced = skin::section(
            cell,
            cols,
            vec![skin::title(
                format!("Unsynced ↑{}", self.unsynced),
                theme.text_primary,
            )],
            theme.border,
            theme.island_background,
            m,
        )
        .child(
            div()
                .absolute()
                .top_0()
                .right(cell.cols(2))
                .flex()
                .bg(theme.island_background)
                .child(gap())
                .child(
                    tui_chip("git-branch-chip", &branch, &theme, cell)
                        .text_color(theme.git_branch_tag_text)
                        .on_click(move |event, _, cx| {
                            let at = event.position();
                            chip_entity.update(cx, |_, cx| cx.emit(GitToolEvent::Branches(at)));
                        }),
                )
                .child(gap()),
        );
        let listed: Vec<&Commit> = self.commits.iter().collect();
        let (graph, lanes) = graph_rows(&listed, &self.parents);
        let commits: Vec<gpui::AnyElement> = listed
            .into_iter()
            .zip(graph)
            .enumerate()
            .map(|(ix, (c, row))| tui_commit_row(ix, c, row, lanes, cols, cell, &theme, m))
            .collect();
        div()
            .key_context("GitTool")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .font_family(theme.mono_font.clone())
            .text_size(px(cell.font_size))
            .line_height(px(cell.h))
            .child(skin::measure(cell, &self.tui_grid))
            .child(message)
            .child(div().h(px(cell.h)).flex_shrink_0().child(buttons))
            .children(error)
            .child(
                div()
                    .id("git-rows")
                    .flex_1()
                    .flex_basis(px(0.))
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .children(list),
            )
            .child(unsynced)
            .child(
                div()
                    .id("git-commits")
                    .flex_1()
                    .flex_basis(px(0.))
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(commits),
            )
            .into_any_element()
    }

    /// A changed file's row: `[x] M ◆ name  dir`, the letter and the name
    /// in the git colours, the selected row filled.
    fn tui_file_row(
        &self,
        ix: usize,
        e: &StatusEntry,
        cell: Cell,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let name = e
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let dir = self
            .repo
            .as_ref()
            .and_then(|r| e.path.parent()?.strip_prefix(&r.root).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .filter(|p| !p.is_empty());
        let old: Option<SharedString> = e.old_path.as_ref().map(|p| {
            format!(
                "({})",
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            )
            .into()
        });
        let (glyph, glyph_color) = skin::file_glyph(&name, theme);
        let color = status_text_color(e.status, theme);
        let selected = self.selected.as_ref() == Some(&e.path);
        let path = e.path.clone();
        let toggle_path = e.path.clone();
        let open = cx.entity();
        let toggle = cx.entity();
        let one = |text: &'static str, color| {
            div()
                .w(px(cell.w))
                .flex_shrink_0()
                .text_color(color)
                .child(text)
        };
        div()
            .id(ElementId::NamedInteger("tui-git-row".into(), ix as u64))
            .h(px(cell.h))
            .flex_shrink_0()
            .flex()
            .whitespace_nowrap()
            .overflow_hidden()
            .cursor_pointer()
            .when(selected, |d| d.bg(theme.list_bg_selected))
            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
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
                    .id(ElementId::NamedInteger("tui-git-check".into(), ix as u64))
                    .w(cell.cols(3))
                    .flex_shrink_0()
                    .text_color(theme.text_secondary)
                    .hover(|s| s.text_color(theme.text_primary))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        toggle.update(cx, |this, cx| this.toggle(toggle_path.clone(), cx))
                    })
                    .child(skin::checkbox(e.staged)),
            )
            .child(div().w(px(cell.w)).flex_shrink_0())
            .child(one(status_letter(e.status), color))
            .child(div().w(px(cell.w)).flex_shrink_0())
            .child(one(glyph, glyph_color))
            .child(div().w(px(cell.w)).flex_shrink_0())
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(color)
                    .child(SharedString::from(name)),
            )
            .children(old.map(|o| {
                div()
                    .ml(px(cell.w))
                    .flex_shrink_0()
                    .text_color(theme.text_secondary)
                    .line_through()
                    .child(o)
            }))
            .children(dir.map(|d| {
                div()
                    .ml(cell.cols(2))
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(theme.text_tertiary)
                    .child(SharedString::from(d))
            }))
            .into_any_element()
    }
}

impl HistoryTool {
    /// The History tool in the tui skin: the search field as a row, the
    /// chips as `[ label ▾ ]` segments, a rule with the count set in, then
    /// the commits with their graph.
    fn render_tui(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let cols = self.tui_grid.get().0;
        let entity = cx.entity();
        let query = self.search.read(cx).text().to_lowercase();
        let author = self.author.clone();
        let shown: Vec<&Commit> = self
            .commits
            .iter()
            .filter(|c| query.is_empty() || c.summary.to_lowercase().contains(&query))
            .filter(|c| author.as_ref().is_none_or(|a| &c.author == a))
            .collect();
        let n = shown.len();
        // A filtered list skips commits, so its graph is one lane.
        let filtered = !query.is_empty() || author.is_some();
        let none = Parents::new();
        let (graph, lanes) = graph_rows(&shown, if filtered { &none } else { &self.parents });
        let commits: Vec<gpui::AnyElement> = shown
            .into_iter()
            .zip(graph)
            .enumerate()
            .map(|(ix, (c, row))| tui_commit_row(ix, c, row, lanes, cols, cell, &theme, m))
            .collect();
        let scope = match &self.file {
            Some(f) => f
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            None => self
                .repo
                .as_ref()
                .and_then(|r| r.root.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_default(),
        };
        let branch = self
            .chosen_branch
            .clone()
            .or_else(|| self.branch.clone())
            .unwrap_or_default();
        let author_label = self
            .author
            .clone()
            .unwrap_or_else(|| "all users".to_string());
        let chip = |which: HistoryChip, id: &'static str, label: String| {
            let entity = entity.clone();
            tui_chip(id, &fit_cols(&label, 16), &theme, cell).on_click(move |event, _, cx| {
                let at = event.position();
                entity.update(cx, |_, cx| cx.emit(HistoryToolEvent::Chip(which, at)));
            })
        };
        let gap = || div().w(px(cell.w)).flex_shrink_0();
        let field = div().h(px(cell.h)).flex_shrink_0().flex().child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(cell.h))
                .bg(theme.input_bg)
                .child(self.search.clone()),
        );
        let chips = div()
            .h(px(cell.h))
            .flex_shrink_0()
            .flex()
            .overflow_hidden()
            .child(gap())
            .child(chip(HistoryChip::Branch, "history-branch", branch))
            .child(gap())
            .child(chip(HistoryChip::Scope, "history-scope", scope))
            .child(gap())
            .child(chip(HistoryChip::Author, "history-author", author_label));
        div()
            .key_context("HistoryTool")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .font_family(theme.mono_font.clone())
            .text_size(px(cell.font_size))
            .line_height(px(cell.h))
            .child(skin::measure(cell, &self.tui_grid))
            .child(field)
            .child(chips)
            .child(skin::section(
                cell,
                cols,
                vec![skin::title(
                    format!("{n} commit{}", if n == 1 { "" } else { "s" }),
                    theme.text_secondary,
                )],
                theme.border,
                theme.island_background,
                m,
            ))
            .child(
                div()
                    .id("history-commits")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .children(commits),
            )
            .into_any_element()
    }
}
