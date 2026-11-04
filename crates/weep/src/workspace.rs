//! The window: Fleet's shell. The toolbar across the top, then the islands
//! (the left, bottom and right tool panels when open, the editor splits in
//! the middle, each with its own tab row), then the status bar. Every size
//! comes from [`Metrics`], every colour from [`Theme`]; what was measured
//! where is in `docs/fleet-probe.md`.

use crate::actions::*;
use crate::assets::{file_icon, icon};
use crate::document::Document;
use crate::editor::{EditorEvent, EditorView};
use crate::file_tree::{FileTree, FileTreeEvent};
use crate::find::{FindBar, FindBarEvent};
use crate::layout::{Layout, PanelLayout};
use crate::popup::{Prompt, PromptEvent, PromptKind};
mod runs;
mod smart;
mod terminals;
mod workbench;
use crate::dialog::{ConfirmDialog, DialogButton};
use crate::diff_view::DiffView;
use crate::git_tool::{GitTool, HistoryTool};
use crate::goto::GotoPanel;
use crate::menu::ContextMenu;
use crate::notifications::Notifications;
use crate::run_popup::RunPopup;
use crate::run_tool::RunView;
use crate::scrollbar::{scrollbar, EmptyView};
use crate::search_tool::SearchTool;
use crate::settings::{self, AppSettings};
use crate::settings_view::SettingsView;
use crate::terminal::TerminalView;
use crate::theme::{metrics::EMPTY_PANEL_INSET, Metrics, Theme};
use crate::toolbar;
use crate::tooltip::Tooltip;
use gpui::AnyWindowHandle;
use gpui::{
    canvas, deferred, div, point, prelude::*, px, App, Bounds, BoxShadow, Context, Div,
    DragMoveEvent, ElementId, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    MouseButton, Pixels, Point, Render, ScrollHandle, SharedString, Stateful, Window,
};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use weep_search::FileIndex;
use weep_settings::AutoSave;

/// How many copied texts `editor/paste-from-history` offers.
const CLIPBOARD_HISTORY: usize = 20;

// ---- panels -------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Bottom,
    Right,
}

impl Side {
    const ALL: [Side; 3] = [Side::Left, Side::Bottom, Side::Right];

    fn index(self) -> usize {
        match self {
            Side::Left => 0,
            Side::Bottom => 1,
            Side::Right => 2,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Side::Left => "Left Panel",
            Side::Bottom => "Bottom Panel",
            Side::Right => "Right Panel",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Side::Left => "weep/panel-left-24",
            Side::Bottom => "weep/panel-bottom-24",
            Side::Right => "weep/panel-right-24",
        }
    }
}

/// The tools a panel can show, Fleet's names. Only Files has content yet; the
/// others are M3 to M8 and show their name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Breakpoints,
    BuildOutput,
    Debug,
    DebuggerConsole,
    Files,
    Git,
    History,
    Notifications,
    Preview,
    Problems,
    Run,
    Search,
    Snapshots,
    Terminal,
}

impl Tool {
    /// Alphabetical, as Fleet's chooser lists them.
    pub const ALL: [Tool; 14] = [
        Tool::Breakpoints,
        Tool::BuildOutput,
        Tool::Debug,
        Tool::DebuggerConsole,
        Tool::Files,
        Tool::Git,
        Tool::History,
        Tool::Notifications,
        Tool::Preview,
        Tool::Problems,
        Tool::Run,
        Tool::Search,
        Tool::Snapshots,
        Tool::Terminal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Breakpoints => "Breakpoints",
            Tool::BuildOutput => "Build Output",
            Tool::Debug => "Debug",
            Tool::DebuggerConsole => "Debugger Console",
            Tool::Files => "Files",
            Tool::Git => "Git",
            Tool::History => "History",
            Tool::Notifications => "Notifications",
            Tool::Preview => "Preview",
            Tool::Problems => "Problems",
            Tool::Run => "Run",
            Tool::Search => "Search",
            Tool::Snapshots => "Snapshots",
            Tool::Terminal => "Terminal",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Breakpoints => "weep/breakpoint",
            Tool::BuildOutput => "expui/toolwindows/build",
            Tool::Debug => "expui/run/debug",
            Tool::DebuggerConsole => "weep/console",
            Tool::Files => "expui/nodes/folder",
            Tool::Git => "expui/toolwindows/vcs",
            Tool::History => "expui/general/history",
            Tool::Notifications => "expui/toolwindows/notifications",
            Tool::Preview => "weep/preview",
            Tool::Problems => "expui/toolwindows/problems",
            Tool::Run => "expui/run/run",
            Tool::Search => "expui/toolwindows/find",
            Tool::Snapshots => "weep/snapshot",
            Tool::Terminal => "weep/terminal",
        }
    }
}

/// A tool's tab in a panel. Most tools are one of a kind; terminals are
/// many, told apart by `instance` (M5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolTab {
    pub tool: Tool,
    pub instance: usize,
}

impl ToolTab {
    fn single(tool: Tool) -> ToolTab {
        ToolTab { tool, instance: 0 }
    }
}

struct Panel {
    open: bool,
    /// Width (left, right) or height (bottom), logical px at zoom 1.0.
    size: f32,
    tools: Vec<ToolTab>,
    active: usize,
    focus_handle: FocusHandle,
    /// The empty panel's "Open Tool" button, which takes focus when the panel opens.
    button_focus: FocusHandle,
    /// Where the `+` and the "Open Tool" button were last laid out, for the chooser.
    plus_anchor: Rc<Cell<Bounds<Pixels>>>,
    button_anchor: Rc<Cell<Bounds<Pixels>>>,
    /// The tui skin's grid as last laid out, columns by rows of cells.
    tui_grid: Rc<Cell<(usize, usize)>>,
}

impl Panel {
    fn new(layout: PanelLayout, tools: Vec<Tool>, cx: &mut App) -> Panel {
        Panel {
            open: layout.open,
            size: layout.size,
            tools: tools.into_iter().map(ToolTab::single).collect(),
            active: 0,
            focus_handle: cx.focus_handle(),
            button_focus: cx.focus_handle(),
            plus_anchor: Rc::new(Cell::new(Bounds::default())),
            button_anchor: Rc::new(Cell::new(Bounds::default())),
            tui_grid: Rc::new(Cell::new((0, 0))),
        }
    }

    fn layout(&self) -> PanelLayout {
        PanelLayout {
            open: self.open,
            size: self.size,
        }
    }
}

/// The `+` chooser (or the empty panel's "Open Tool" list).
struct Chooser {
    side: Side,
    origin: Point<Pixels>,
    query: String,
    selected: usize,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

impl Chooser {
    fn matches(&self) -> Vec<Tool> {
        let q = self.query.to_lowercase();
        Tool::ALL
            .iter()
            .copied()
            .filter(|t| q.is_empty() || t.label().to_lowercase().contains(&q))
            .collect()
    }
}

/// The value carried while a splitter is dragged.
struct SplitterDrag {
    side: Side,
}

// ---- editor splits --------------------------------------------------------------

/// What a tab shows: a file in an editor, the Settings tool, or a diff
/// (the Changed Files tab, `diff/two-files`, `diff/with-clipboard`).
enum TabView {
    Editor(Entity<EditorView>),
    Settings(Entity<SettingsView>),
    Diff(Entity<DiffView>),
}

struct Tab {
    view: TabView,
    /// A preview tab (`editor.previewTab`): italic title, replaced by the next
    /// single-click file, promoted by an edit or a double click.
    preview: bool,
}

impl Tab {
    fn editor(&self) -> Option<&Entity<EditorView>> {
        match &self.view {
            TabView::Editor(e) => Some(e),
            TabView::Settings(_) | TabView::Diff(_) => None,
        }
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.view {
            TabView::Editor(e) => e.read(cx).focus_handle(cx),
            TabView::Settings(s) => s.read(cx).focus_handle(cx),
            TabView::Diff(d) => d.read(cx).focus_handle(cx),
        }
    }

    fn title(&self, cx: &App) -> String {
        match &self.view {
            TabView::Editor(e) => e.read(cx).title(cx),
            TabView::Settings(_) => "Settings".into(),
            TabView::Diff(d) => d.read(cx).title(),
        }
    }

    /// The tab's icon; the Changed Files tab has none (ninth pass).
    fn icon(&self, cx: &App) -> Option<&'static str> {
        match &self.view {
            TabView::Editor(e) => Some(file_icon(&e.read(cx).title(cx))),
            TabView::Settings(_) => Some("expui/general/settings"),
            TabView::Diff(_) => None,
        }
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.editor().is_some_and(|e| e.read(cx).is_dirty(cx))
    }

    fn path(&self, cx: &App) -> Option<PathBuf> {
        self.editor().and_then(|e| e.read(cx).path(cx))
    }
}

struct Pane {
    tabs: Vec<Tab>,
    active: Option<usize>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    /// The split's find and replace bar, over its active editor.
    find: Entity<FindBar>,
    /// The tui skin's grid as last laid out, columns by rows of cells.
    tui_grid: Rc<Cell<(usize, usize)>>,
}

impl Pane {
    fn new(cx: &mut Context<Workspace>) -> Pane {
        let find = cx.new(FindBar::new);
        cx.subscribe(&find, |this, _, event: &FindBarEvent, cx| match event {
            FindBarEvent::Closed => this.focus_active_editor(cx),
            FindBarEvent::Changed => cx.notify(),
        })
        .detach();
        Pane {
            tabs: Vec::new(),
            active: None,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            find,
            tui_grid: Rc::new(Cell::new((0, 0))),
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|ix| self.tabs.get(ix))
    }
}

/// The value carried while a tab is dragged.
struct DraggedTab {
    pane: usize,
    index: usize,
}

/// What the pointer drags: the tab inside Fleet's 2 px ring.
struct TabGhost {
    title: SharedString,
    icon: Option<&'static str>,
    dirty: bool,
}

impl Render for TabGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        div()
            .p(px(0.))
            .rounded(px(m.editor_tab_radius + m.editor_tab_drag_ring))
            .border(px(m.editor_tab_drag_ring))
            .border_color(theme.tab_drag_outline)
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            .child(tab_body(
                self.title.clone(),
                self.icon,
                self.dirty,
                false,
                theme.tab_bg_hovered,
                theme.tab_text,
                &theme,
                &m,
            ))
    }
}

/// A tab's content: icon, title, the modified dot, the close box.
#[allow(clippy::too_many_arguments)]
fn tab_body(
    title: SharedString,
    icon_path: Option<&'static str>,
    dirty: bool,
    preview: bool,
    bg: gpui::Hsla,
    text: gpui::Hsla,
    theme: &Theme,
    m: &Metrics,
) -> Div {
    div()
        .h(px(m.editor_tab_height))
        .pl(px(m.editor_tab_pad_left))
        .pr(px(m.editor_tab_pad_right))
        .flex()
        .items_center()
        .rounded(px(m.editor_tab_radius))
        .bg(bg)
        .text_color(text)
        .children(icon_path.map(|icon_path| {
            div()
                .size(px(m.icon_size))
                .flex_shrink_0()
                .mr(px(m.editor_tab_icon_gap))
                .child(icon(icon_path, m.icon_size, theme.icon_secondary))
        }))
        .child(
            div()
                .whitespace_nowrap()
                .when(preview, |d| d.italic())
                .child(title),
        )
        .when(dirty, |d| {
            d.child(
                div()
                    .w(px(m.editor_tab_dot_box))
                    .h(px(m.icon_size))
                    .ml(px(m.editor_tab_close_gap))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(m.editor_tab_dot)).rounded_full().bg(text)),
            )
        })
        .child(
            div()
                .size(px(m.icon_size))
                .flex_shrink_0()
                .ml(px(m.editor_tab_close_gap))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(
                    "expui/general/closeSmall",
                    m.icon_size,
                    theme.tab_close_icon,
                )),
        )
}

// ---- the workspace --------------------------------------------------------------

pub struct Workspace {
    root: PathBuf,
    branch: Option<SharedString>,
    /// `showShortcuts`: the chord just pressed, shown in the overlay pill for
    /// a moment; the counter tells a stale timer from a fresh one.
    shortcut_hint: Option<SharedString>,
    shortcut_generation: u64,
    /// The git repository above the root, when git finds one (M4).
    repo: Option<weep_git::Repo>,
    /// Disk-poll ticks, for the slower git status refresh.
    disk_ticks: u32,
    /// A git refresh is reading on the background executor; one asked for
    /// meanwhile runs after it (`git_refresh_again`), never beside it.
    git_refreshing: bool,
    git_refresh_again: bool,
    file_tree: Entity<FileTree>,
    panes: Vec<Pane>,
    active_pane: usize,
    panels: [Panel; 3],
    /// Paths of closed tabs, most recent last, for `tabs/reopen-closed-tab`.
    closed: Vec<PathBuf>,
    /// One document per open file, shared by every tab and split showing it.
    documents: Vec<Entity<Document>>,
    /// Polls the open files' modification times once a second (Fleet's
    /// "File has been changed on disk").
    disk_poll: Option<gpui::Task<()>>,
    chooser: Option<Chooser>,
    /// The prompt popup under the toolbar, when one is open.
    prompt: Option<Entity<Prompt>>,
    /// Texts copied or cut in the editor, most recent first (`editor/paste-from-history`).
    clipboard_history: Vec<String>,
    /// Bumped on every edit; an `autoSave` timer saves only when it still matches.
    autosave_generation: u64,
    /// The tab being dragged, whose slot stays empty until the drop.
    dragging_tab: Option<(usize, usize)>,
    /// M3: the Search tool, the notifications, the popups and their state.
    search_tool: Entity<SearchTool>,
    git_tool: Entity<GitTool>,
    history_tool: Entity<HistoryTool>,
    notifications: Entity<Notifications>,
    goto: Option<Entity<GotoPanel>>,
    menu: Option<Entity<ContextMenu>>,
    dialog: Option<Entity<ConfirmDialog>>,
    dialog_callback: Option<workbench::DialogCallback>,
    /// Files shown in this workspace, most recent first (`goto/recent-file`).
    recent: Vec<PathBuf>,
    goto_preview_hidden: bool,
    /// The workspace's files for `goto/file`, rebuilt when the disk changes.
    file_index: Arc<FileIndex>,
    /// The Notifications popup under the toolbar's bell is open.
    notifications_open: bool,
    /// The terminals (M5), by the id their panel tab carries.
    terminals: Vec<(usize, Entity<TerminalView>)>,
    next_terminal: usize,
    /// The run tool's tabs (M6), by the id their panel tab carries; the
    /// Run & Debug popup while open; the recent runs' names, most recent first.
    runs: Vec<(usize, Entity<RunView>)>,
    next_run: usize,
    run_popup: Option<Entity<RunPopup>>,
    recent_runs: Vec<String>,
    /// Smart Mode (M7): the language servers, the navigation history, the
    /// usages popup.
    smart: smart::SmartMode,
    /// This workspace's window, for deferred focus changes.
    window: AnyWindowHandle,
    focus_handle: FocusHandle,
    last_title: String,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// A path as the platform spells it, whatever separators the command line used.
fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    if cfg!(windows) {
        text.replace('/', "\\")
    } else {
        text
    }
}

/// The branch `.git/HEAD` names, for the window title.
fn git_branch(root: &Path) -> Option<String> {
    let head = std::fs::read_to_string(root.join(".git").join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(rest) = head.strip_prefix("ref: ") {
        return Some(rest.rsplit('/').next().unwrap_or(rest).to_string());
    }
    Some(head.chars().take(8).collect())
}

impl Workspace {
    pub fn new(
        root: PathBuf,
        open: Vec<PathBuf>,
        scratch: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let file_tree = cx.new(|cx| FileTree::new(root.clone(), cx));
        cx.subscribe(
            &file_tree,
            |this, _tree, event: &FileTreeEvent, cx| match event {
                FileTreeEvent::Open { path, preview } => {
                    this.open_path(path.clone(), *preview, cx);
                    if !preview {
                        this.focus_active_editor(cx);
                    }
                }
                other => this.on_file_tree_event(other, cx),
            },
        )
        .detach();
        let layout = Layout::load(&root);
        let panels = [
            Panel::new(
                layout.left,
                vec![Tool::Files, Tool::Search, Tool::Git, Tool::History],
                cx,
            ),
            Panel::new(layout.bottom, Vec::new(), cx),
            Panel::new(layout.right, Vec::new(), cx),
        ];
        let search_tool = Self::make_search_tool(root.clone(), cx);
        let repo = weep_git::Repo::discover(&root);
        let git_tool = cx.new(|cx| GitTool::new(repo.clone(), cx));
        let history_tool = cx.new(|cx| HistoryTool::new(repo.clone(), cx));
        let mut this = Workspace {
            branch: git_branch(&root).map(Into::into),
            repo,
            shortcut_hint: None,
            shortcut_generation: 0,
            disk_ticks: 0,
            git_refreshing: false,
            git_refresh_again: false,
            root,
            file_tree,
            panes: vec![Pane::new(cx)],
            active_pane: 0,
            panels,
            closed: Vec::new(),
            documents: Vec::new(),
            disk_poll: None,
            chooser: None,
            prompt: None,
            clipboard_history: Vec::new(),
            autosave_generation: 0,
            dragging_tab: None,
            search_tool,
            git_tool,
            history_tool,
            notifications: cx.new(|_| Notifications::new()),
            goto: None,
            menu: None,
            dialog: None,
            dialog_callback: None,
            recent: layout.recent.clone(),
            goto_preview_hidden: layout.goto_preview_hidden,
            file_index: Arc::new(FileIndex::default()),
            notifications_open: false,
            terminals: Vec::new(),
            next_terminal: 0,
            runs: Vec::new(),
            next_run: 0,
            run_popup: None,
            recent_runs: layout.recent_runs.clone(),
            smart: smart::SmartMode::new(layout.smart_mode),
            window: window.window_handle(),
            focus_handle: cx.focus_handle(),
            last_title: String::new(),
        };
        this.init_workbench(window, cx);
        // `launchTerminalOnOpen`: a shell in the bottom panel, the editor keeping the focus.
        if Self::settings(cx).launch_terminal_on_open() {
            this.new_terminal(None, false, window, cx);
        }
        this.refresh_git(cx);
        this.watch_disk(cx);
        this.watch_shortcuts(cx);
        // `autoSave: "OnFocus"`: every dirty document is written when the
        // window loses the focus.
        cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() && Self::settings(cx).auto_save() == AutoSave::OnFocus {
                this.save_dirty(cx);
            }
        })
        .detach();
        for path in open {
            this.open_path(path, false, cx);
        }
        if scratch {
            this.open_scratch(cx);
        }
        if this.panes[0].active.is_some() {
            this.focus_active_editor(cx);
        } else {
            let tree = this.file_tree.clone();
            tree.read(cx).focus_handle(cx).focus(window, cx);
        }
        this
    }

    fn settings(cx: &App) -> &weep_settings::Settings {
        &cx.global::<AppSettings>().settings
    }

    fn layout(&self) -> Layout {
        Layout {
            left: self.panels[0].layout(),
            bottom: self.panels[1].layout(),
            right: self.panels[2].layout(),
            recent: self.recent.clone(),
            goto_preview_hidden: self.goto_preview_hidden,
            recent_runs: self.recent_runs.clone(),
            smart_mode: self.smart.enabled,
        }
    }

    fn save_layout(&self) {
        self.layout().save(&self.root);
    }

    fn panel(&self, side: Side) -> &Panel {
        &self.panels[side.index()]
    }

    fn panel_mut(&mut self, side: Side) -> &mut Panel {
        &mut self.panels[side.index()]
    }

    // ---- tabs ---------------------------------------------------------------

    fn find_tab(&self, path: &Path, cx: &App) -> Option<(usize, usize)> {
        self.panes.iter().enumerate().find_map(|(p, pane)| {
            pane.tabs
                .iter()
                .position(|t| t.path(cx).as_deref() == Some(path))
                .map(|ix| (p, ix))
        })
    }

    /// The document for `path`, shared with every tab already showing it,
    /// else read from disk. `None` when the file cannot be read as text.
    fn document_for(&mut self, path: &Path, cx: &mut Context<Self>) -> Option<Entity<Document>> {
        if let Some(doc) = self
            .documents
            .iter()
            .find(|d| d.read(cx).path.as_deref() == Some(path))
        {
            return Some(doc.clone());
        }
        let text = match std::fs::read(path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => {
                    self.notify_error(
                        "Not a text file",
                        format!("{} is not UTF-8 text", path.display()),
                        cx,
                    );
                    return None;
                }
            },
            Err(e) => {
                self.notify_error("Cannot read file", format!("{}: {e}", path.display()), cx);
                return None;
            }
        };
        let doc = cx.new(|cx| {
            let settings = &cx.global::<AppSettings>().settings;
            Document::new(Some(path.to_path_buf()), &text, settings)
        });
        self.documents.push(doc.clone());
        self.smart_document_opened(&doc, cx);
        Some(doc)
    }

    /// Drops the documents no tab shows any more (their server hears `didClose`).
    fn prune_documents(&mut self, cx: &App) {
        let shown: Vec<Entity<Document>> = self
            .panes
            .iter()
            .flat_map(|p| p.tabs.iter().filter_map(|t| t.editor()))
            .map(|e| e.read(cx).document.clone())
            .collect();
        let gone: Vec<PathBuf> = self
            .documents
            .iter()
            .filter(|d| !shown.contains(d))
            .filter_map(|d| d.read(cx).path.clone())
            .collect();
        self.documents.retain(|d| shown.contains(d));
        for path in gone {
            self.smart_document_closed(&path);
        }
    }

    fn new_editor(
        &mut self,
        document: Entity<Document>,
        cx: &mut Context<Self>,
    ) -> Entity<EditorView> {
        let editor = cx.new(|cx| EditorView::new(document, cx));
        cx.subscribe(&editor, |this, editor, event: &EditorEvent, cx| {
            match event {
                EditorEvent::Changed => {
                    // An edit promotes a preview tab to a real one.
                    for pane in &mut this.panes {
                        for tab in &mut pane.tabs {
                            if tab.editor() == Some(&editor) {
                                tab.preview = false;
                            }
                        }
                    }
                    this.schedule_autosave(cx);
                }
                EditorEvent::Copied(text) => this.remember_copied(text),
                EditorEvent::ContextMenu(position) => {
                    this.editor_context_menu(&editor, *position, cx)
                }
                EditorEvent::Saved => this.refresh_git(cx),
                EditorEvent::ShowDiff => {
                    let path = editor.read(cx).path(cx);
                    this.open_changed_files(path, cx);
                }
                EditorEvent::RunConfiguration(ix) => this.run_configuration_at(*ix, cx),
                EditorEvent::Completion { .. }
                | EditorEvent::SignatureHelp
                | EditorEvent::Hover { .. }
                | EditorEvent::GoToDefinition { .. }
                | EditorEvent::ProblemFixes(_)
                | EditorEvent::ApplyProblemFix { .. }
                | EditorEvent::ResolveCompletion(_)
                | EditorEvent::Accepted { .. } => this.on_smart_editor_event(&editor, event, cx),
            }
            cx.notify()
        })
        .detach();
        self.smart_editor_created(&editor, cx);
        editor
    }

    /// `autoSave`: `autoSave.delay` ms after the last edit, every dirty
    /// document is written.
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        let settings = Self::settings(cx);
        if settings.auto_save() != AutoSave::AfterDelay {
            return;
        }
        let delay = Duration::from_millis(settings.auto_save_delay_ms());
        self.autosave_generation += 1;
        let generation = self.autosave_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                if this.autosave_generation == generation {
                    this.save_dirty(cx);
                }
            });
        })
        .detach();
    }

    fn save_dirty(&mut self, cx: &mut Context<Self>) {
        let editors: Vec<_> = self
            .panes
            .iter()
            .flat_map(|p| p.tabs.iter().filter_map(|t| t.editor().cloned()))
            .filter(|e| e.read(cx).is_dirty(cx))
            .collect();
        for editor in editors {
            if let Err(e) = editor.update(cx, |e, cx| e.save(cx)) {
                self.notify_error("Save failed", format!("{e}"), cx);
            }
        }
    }

    fn remember_copied(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clipboard_history.retain(|t| t != text);
        self.clipboard_history.insert(0, text.to_string());
        self.clipboard_history.truncate(CLIPBOARD_HISTORY);
    }

    /// `localBigFileMinimumThreshold`: a file this big asks before it opens.
    fn is_big_file(path: &Path, cx: &App) -> bool {
        let threshold = Self::settings(cx).local_big_file_minimum_threshold_mb();
        std::fs::metadata(path)
            .map(|m| m.len() as f64 >= threshold * 1024. * 1024.)
            .unwrap_or(false)
    }

    /// Opens `path` in the active split: in a preview tab (replacing the
    /// current preview) when `preview`, else in a tab of its own. An already
    /// open file is activated instead, and promoted when `preview` is false.
    pub fn open_path(&mut self, path: PathBuf, preview: bool, cx: &mut Context<Self>) {
        if let Some((p, ix)) = self.find_tab(&path, cx) {
            self.active_pane = p;
            self.panes[p].active = Some(ix);
            if !preview {
                self.panes[p].tabs[ix].preview = false;
            }
            self.panes[p].scroll.scroll_to_item(ix);
            self.after_activate(cx);
            return;
        }
        if Self::is_big_file(&path, cx) {
            // Fleet confirms before opening a file past the threshold.
            let size_mb = std::fs::metadata(&path)
                .map(|m| m.len() as f64 / 1024. / 1024.)
                .unwrap_or(0.);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            self.confirm(
                format!("{name} is {size_mb:.1} MB. Open it anyway?"),
                Some("Large files open without highlighting.".into()),
                vec![
                    DialogButton::primary("Open"),
                    DialogButton::secondary("Cancel"),
                ],
                cx,
                move |this, choice, cx| {
                    if choice == 0 {
                        this.open_path_unchecked(path, preview, cx);
                    }
                },
            );
            return;
        }
        self.open_path_unchecked(path, preview, cx);
    }

    fn open_path_unchecked(&mut self, path: PathBuf, preview: bool, cx: &mut Context<Self>) {
        let Some(document) = self.document_for(&path, cx) else {
            return;
        };
        let editor = self.new_editor(document, cx);
        let p = self.active_pane;
        let pane = &mut self.panes[p];
        let existing_preview = pane.tabs.iter().position(|t| t.preview && !t.is_dirty(cx));
        match (preview, existing_preview) {
            (true, Some(ix)) => {
                pane.tabs[ix].view = TabView::Editor(editor);
                pane.active = Some(ix);
            }
            _ => {
                pane.tabs.push(Tab {
                    view: TabView::Editor(editor),
                    preview,
                });
                pane.active = Some(pane.tabs.len() - 1);
            }
        }
        let ix = pane.active.unwrap_or(0);
        pane.scroll.scroll_to_item(ix);
        self.prune_documents(cx);
        self.after_activate(cx);
    }

    /// Opens a file WEEP owns (settings, keymap), creating it with `template` first.
    fn open_own_file(&mut self, path: Option<PathBuf>, template: &str, cx: &mut Context<Self>) {
        let Some(path) = path else { return };
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, template) {
                self.notify_error("Cannot create file", format!("{}: {e}", path.display()), cx);
                return;
            }
        }
        self.open_path(path, false, cx);
        self.focus_active_editor(cx);
    }

    /// After a tab became active: the tree follows (`files.selectOpenedFile`).
    fn after_activate(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.active_editor().and_then(|e| e.read(cx).path(cx)) {
            self.touch_recent(&path);
            self.file_tree.update(cx, |tree, cx| {
                if tree.select_path(&path) {
                    cx.notify();
                }
            });
        }
        cx.notify();
    }

    fn active_editor(&self) -> Option<&Entity<EditorView>> {
        self.panes
            .get(self.active_pane)
            .and_then(Pane::active_tab)
            .and_then(Tab::editor)
    }

    fn focus_active_editor(&self, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            let handle = editor.read(cx).focus_handle(cx);
            self.focus_later(handle, cx);
        }
    }

    fn focus_editor_now(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.panes.get(self.active_pane).and_then(Pane::active_tab) {
            tab.focus_handle(cx).focus(window, cx);
        } else {
            self.file_tree.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    fn activate_tab(
        &mut self,
        pane: usize,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if pane >= self.panes.len() || ix >= self.panes[pane].tabs.len() {
            return;
        }
        self.active_pane = pane;
        self.panes[pane].active = Some(ix);
        self.panes[pane].scroll.scroll_to_item(ix);
        self.focus_editor_now(window, cx);
        self.after_activate(cx);
    }

    fn close_tab_at(&mut self, p: usize, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if p >= self.panes.len() || ix >= self.panes[p].tabs.len() {
            return;
        }
        let tab = self.panes[p].tabs.remove(ix);
        if let Some(path) = tab.path(cx) {
            self.closed.push(path);
        }
        drop(tab);
        self.prune_documents(cx);
        let pane = &mut self.panes[p];
        pane.active = if pane.tabs.is_empty() {
            None
        } else {
            Some(
                pane.active
                    .map(|a| {
                        if a > ix {
                            a - 1
                        } else {
                            a.min(pane.tabs.len() - 1)
                        }
                    })
                    .unwrap_or(0),
            )
        };
        if pane.tabs.is_empty() && self.panes.len() > 1 && Self::settings(cx).close_empty_split() {
            self.remove_pane(p);
        }
        self.focus_editor_now(window, cx);
        self.after_activate(cx);
    }

    fn remove_pane(&mut self, p: usize) {
        self.panes.remove(p);
        self.active_pane = self.active_pane.min(self.panes.len() - 1);
        if self.active_pane > p || (self.active_pane == p && p == self.panes.len()) {
            self.active_pane = self.active_pane.saturating_sub(1).min(self.panes.len() - 1);
        }
    }

    fn move_tab(
        &mut self,
        from: (usize, usize),
        to_pane: usize,
        to_ix: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let (fp, fi) = from;
        if fp >= self.panes.len() || fi >= self.panes[fp].tabs.len() || to_pane >= self.panes.len()
        {
            return;
        }
        let tab = self.panes[fp].tabs.remove(fi);
        let pane = &mut self.panes[fp];
        pane.active = if pane.tabs.is_empty() {
            None
        } else {
            Some(
                pane.active
                    .map(|a| {
                        if a > fi {
                            a - 1
                        } else {
                            a.min(pane.tabs.len() - 1)
                        }
                    })
                    .unwrap_or(0),
            )
        };
        let mut to_ix = to_ix.unwrap_or(self.panes[to_pane].tabs.len());
        if fp == to_pane && to_ix > fi {
            to_ix -= 1;
        }
        let target = &mut self.panes[to_pane];
        let to_ix = to_ix.min(target.tabs.len());
        target.tabs.insert(to_ix, tab);
        target.active = Some(to_ix);
        self.active_pane = to_pane;
        if self.panes[fp].tabs.is_empty()
            && fp != to_pane
            && self.panes.len() > 1
            && Self::settings(cx).close_empty_split()
        {
            self.remove_pane(fp);
            if to_pane > fp {
                self.active_pane = to_pane - 1;
            }
        }
        self.after_activate(cx);
    }

    // ---- actions --------------------------------------------------------------

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            self.save_editor(&editor, cx);
        }
    }

    fn save_all(&mut self, _: &SaveAll, _: &mut Window, cx: &mut Context<Self>) {
        let editors: Vec<_> = self
            .panes
            .iter()
            .flat_map(|p| p.tabs.iter().filter_map(|t| t.editor().cloned()))
            .collect();
        for editor in editors {
            self.save_editor(&editor, cx);
        }
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_focused_terminal(window, cx) {
            return;
        }
        let p = self.active_pane;
        if let Some(ix) = self.panes[p].active {
            self.close_tab_at(p, ix, window, cx);
        }
    }

    fn close_other_tabs(
        &mut self,
        _: &CloseOtherTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = self.active_pane;
        let Some(keep) = self.panes[p].active else {
            return;
        };
        for ix in (0..self.panes[p].tabs.len()).rev() {
            if ix != keep {
                self.close_tab_at(p, ix, window, cx);
            }
        }
    }

    fn close_tabs_to_the_right(
        &mut self,
        _: &CloseTabsToTheRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = self.active_pane;
        let Some(keep) = self.panes[p].active else {
            return;
        };
        for ix in (keep + 1..self.panes[p].tabs.len()).rev() {
            self.close_tab_at(p, ix, window, cx);
        }
    }

    fn close_saved_tabs(
        &mut self,
        _: &CloseSavedTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = self.active_pane;
        for ix in (0..self.panes[p].tabs.len()).rev() {
            if !self.panes[p].tabs[ix].is_dirty(cx) {
                self.close_tab_at(p, ix, window, cx);
            }
        }
    }

    fn reopen_closed_tab(
        &mut self,
        _: &ReopenClosedTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.closed.pop() {
            self.open_path(path, false, cx);
            self.focus_editor_now(window, cx);
        }
    }

    fn close_window(&mut self, _: &CloseWindow, _: &mut Window, cx: &mut Context<Self>) {
        self.request_quit(cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.active_pane;
        if let Some(ix) = self.panes[p].active {
            let n = self.panes[p].tabs.len();
            self.activate_tab(p, (ix + 1) % n, window, cx);
        }
    }

    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.active_pane;
        if let Some(ix) = self.panes[p].active {
            let n = self.panes[p].tabs.len();
            self.activate_tab(p, (ix + n - 1) % n, window, cx);
        }
    }

    fn toggle_panel(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panel_mut(side);
        panel.open = !panel.open;
        if panel.open && panel.tools.is_empty() {
            // Fleet gives the empty panel's button the keyboard focus (batch 2).
            panel.button_focus.focus(window, cx);
        } else if !panel.open && panel.focus_handle.contains_focused(window, cx) {
            self.focus_editor_now(window, cx);
        }
        self.save_layout();
        cx.notify();
    }

    fn toggle_left(&mut self, _: &ToggleLeftPanel, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(Side::Left, window, cx);
    }
    fn toggle_bottom(
        &mut self,
        _: &ToggleBottomPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_panel(Side::Bottom, window, cx);
    }
    fn toggle_right(&mut self, _: &ToggleRightPanel, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(Side::Right, window, cx);
    }
    fn hide_all(&mut self, _: &HideAllPanels, window: &mut Window, cx: &mut Context<Self>) {
        for panel in &mut self.panels {
            panel.open = false;
        }
        self.focus_editor_now(window, cx);
        self.save_layout();
        cx.notify();
    }
    /// `hide-active-panel`: the panel holding the keyboard focus closes.
    fn hide_active(&mut self, _: &HideActivePanel, window: &mut Window, cx: &mut Context<Self>) {
        let focused = Side::ALL.into_iter().find(|s| {
            self.panel(*s).open && self.panel(*s).focus_handle.contains_focused(window, cx)
        });
        if let Some(side) = focused {
            self.panel_mut(side).open = false;
            self.focus_editor_now(window, cx);
            self.save_layout();
            cx.notify();
        }
    }
    fn focus_editor(&mut self, _: &FocusEditor, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_editor_now(window, cx);
    }
    /// `new-file`: a scratch file when `createScratchOnNewFile` is on, else a
    /// prompt for the name of a file in the Files tree's selected folder (the
    /// active file's folder, or the root).
    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        if Self::settings(cx).create_scratch_on_new_file() {
            self.open_scratch(cx);
            self.focus_editor_now(window, cx);
            return;
        }
        let dir = self.target_dir(window, cx);
        self.open_prompt(PromptKind::NewFile { dir }, "", Vec::new(), window, cx);
    }

    /// The folder a new file goes into: the Files tree's selection when the
    /// tree has the focus, else the active file's folder, else the root.
    fn target_dir(&self, window: &Window, cx: &App) -> PathBuf {
        let tree = self.file_tree.read(cx);
        if tree.focus_handle(cx).contains_focused(window, cx) {
            if let Some(dir) = tree.selected_dir() {
                return dir;
            }
        }
        self.active_editor()
            .and_then(|e| e.read(cx).path(cx))
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| self.root.clone())
    }

    /// Where scratch files live: `scratches/` in WEEP's config folder.
    fn scratch_dir() -> Option<PathBuf> {
        weep_settings::weep_config_dir().map(|d| d.join("scratches"))
    }

    /// `--scratch` and `createScratchOnNewFile`: a fresh `scratch_N.txt`.
    fn open_scratch(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = Self::scratch_dir() else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.notify_error(
                "Cannot create folder",
                format!("{}: {e}", dir.display()),
                cx,
            );
            return;
        }
        let path = (1..)
            .map(|n| dir.join(format!("scratch_{n}.txt")))
            .find(|p| !p.exists())
            .expect("a free scratch name");
        if let Err(e) = std::fs::write(&path, "") {
            self.notify_error("Cannot create file", format!("{}: {e}", path.display()), cx);
            return;
        }
        self.open_path(path, false, cx);
    }

    /// The file an action on "the current file" means: the Files tree's
    /// selected row when the tree has the focus, else the active tab's file.
    fn target_file(&self, window: &Window, cx: &App) -> Option<PathBuf> {
        let tree = self.file_tree.read(cx);
        if tree.focus_handle(cx).contains_focused(window, cx) {
            return tree.selected_path();
        }
        self.active_editor().and_then(|e| e.read(cx).path(cx))
    }

    fn open_prompt(
        &mut self,
        kind: PromptKind,
        initial: &str,
        items: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = cx.new(|cx| Prompt::new(kind, initial, items, cx));
        cx.subscribe(&prompt, |this, prompt, event: &PromptEvent, cx| {
            if this.prompt.as_ref() != Some(&prompt) {
                return;
            }
            match event {
                PromptEvent::Confirm { text, index } => {
                    let kind = prompt.read(cx).kind.clone();
                    this.prompt = None;
                    this.confirm_prompt(kind, text.clone(), *index, cx);
                }
                PromptEvent::Cancel => {
                    this.prompt = None;
                    this.focus_active_editor(cx);
                }
            }
            cx.notify();
        })
        .detach();
        prompt.read(cx).focus_handle(cx).focus(window, cx);
        self.prompt = Some(prompt);
        cx.notify();
    }

    fn confirm_prompt(
        &mut self,
        kind: PromptKind,
        text: String,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        match kind {
            // The symbol rename's prompt answers through its own subscription
            // (`workspace/smart.rs`).
            PromptKind::RenameSymbol => {}
            PromptKind::GotoLine => {
                let mut parts = text.split(':');
                let row = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
                let column = parts
                    .next()
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(1);
                if let (Some(row), Some(editor)) = (row, self.active_editor().cloned()) {
                    editor.update(cx, |e, cx| {
                        e.goto(row.saturating_sub(1), column.saturating_sub(1), cx)
                    });
                }
            }
            PromptKind::NewFile { dir } => {
                let name = text.trim();
                if !name.is_empty() {
                    let path = dir.join(name);
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if path.exists() {
                        self.notify_error("File exists", path.display().to_string(), cx);
                    } else if let Err(e) = std::fs::write(&path, "") {
                        self.notify_error(
                            "Cannot create file",
                            format!("{}: {e}", path.display()),
                            cx,
                        );
                    } else {
                        self.files_changed(cx);
                        self.open_path(path, false, cx);
                    }
                }
            }
            PromptKind::NewFolder { dir } => {
                let name = text.trim();
                if !name.is_empty() {
                    let path = dir.join(name);
                    match std::fs::create_dir_all(&path) {
                        Ok(()) => {
                            self.files_changed(cx);
                            self.file_tree.update(cx, |tree, _| {
                                tree.select_path(&path);
                            });
                        }
                        Err(e) => self.notify_error(
                            "Cannot create folder",
                            format!("{}: {e}", path.display()),
                            cx,
                        ),
                    }
                }
            }
            PromptKind::Rename { path } => {
                let name = text.trim();
                if !name.is_empty() {
                    let to = path.with_file_name(name);
                    if to != path {
                        match std::fs::rename(&path, &to) {
                            Ok(()) => self.path_renamed(&path, to, cx),
                            Err(e) => self.notify_error(
                                "Cannot rename",
                                format!("{}: {e}", path.display()),
                                cx,
                            ),
                        }
                    }
                }
            }
            PromptKind::LineEndings => {
                if let Some(editor) = self.active_editor().cloned() {
                    editor.update(cx, |e, cx| e.set_crlf(index == 1, cx));
                }
            }
            PromptKind::PasteHistory => {
                if let (Some(text), Some(editor)) = (
                    self.clipboard_history.get(index).cloned(),
                    self.active_editor().cloned(),
                ) {
                    editor.update(cx, |e, cx| e.paste_text(&text, cx));
                }
            }
        }
        self.focus_active_editor(cx);
    }

    /// After a rename on disk: the document and its tabs follow, the tree reloads.
    fn path_renamed(&mut self, from: &Path, to: PathBuf, cx: &mut Context<Self>) {
        for doc in &self.documents {
            let matches = doc.read(cx).path.as_deref() == Some(from);
            if matches {
                doc.update(cx, |d, cx| d.set_path(Some(to.clone()), cx));
            }
        }
        self.files_changed(cx);
        self.file_tree.update(cx, |tree, _| {
            tree.select_path(&to);
        });
        cx.notify();
    }

    fn goto_line(&mut self, _: &GotoLine, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_editor().is_none() {
            return;
        }
        self.open_prompt(PromptKind::GotoLine, "", Vec::new(), window, cx);
    }

    fn rename_file(&mut self, _: &RenameFile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.target_file(window, cx) else {
            return;
        };
        if self
            .file_tree
            .update(cx, |tree, cx| tree.start_rename(&path, window, cx))
        {
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.open_prompt(PromptKind::Rename { path }, &name, Vec::new(), window, cx);
    }

    /// `files/delete`: Fleet removes the files at once, no dialog (ninth
    /// pass); WEEP sends them to the system trash (every selected row when the
    /// tree has the focus) and closes their tabs.
    fn delete_file(&mut self, _: &DeleteFile, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.target_files(window, cx);
        for path in &paths {
            self.delete_path(path, cx);
        }
    }

    fn delete_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        let result = trash::delete(path).map_err(|e| std::io::Error::other(e.to_string()));
        if let Err(e) = result {
            self.notify_error("Cannot delete", format!("{}: {e}", path.display()), cx);
            return;
        }
        for p in 0..self.panes.len() {
            for ix in (0..self.panes[p].tabs.len()).rev() {
                let under = self.panes[p].tabs[ix]
                    .path(cx)
                    .is_some_and(|f| f == path || f.starts_with(path));
                if under {
                    self.panes[p].tabs.remove(ix);
                    let pane = &mut self.panes[p];
                    pane.active = if pane.tabs.is_empty() {
                        None
                    } else {
                        Some(pane.active.unwrap_or(0).min(pane.tabs.len() - 1))
                    };
                }
            }
        }
        self.closed.retain(|c| c != path);
        self.prune_documents(cx);
        self.files_changed(cx);
        self.focus_active_editor(cx);
        cx.notify();
    }

    fn change_line_endings(
        &mut self,
        _: &ChangeLineEndings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_editor().is_none() {
            return;
        }
        self.open_prompt(
            PromptKind::LineEndings,
            "",
            vec!["LF".into(), "CRLF".into()],
            window,
            cx,
        );
    }

    fn paste_from_history(
        &mut self,
        _: &PasteFromHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_editor().is_none() || self.clipboard_history.is_empty() {
            return;
        }
        let items = self
            .clipboard_history
            .iter()
            .map(|t| {
                let one_line: String = t.lines().next().unwrap_or("").chars().take(120).collect();
                if t.contains('\n') {
                    format!("{one_line} …").into()
                } else {
                    one_line.into()
                }
            })
            .collect();
        self.open_prompt(PromptKind::PasteHistory, "", items, window, cx);
    }

    // ---- find and replace ------------------------------------------------------

    fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        let bar = self.panes[self.active_pane].find.clone();
        bar.update(cx, |bar, cx| bar.show(false, window, cx));
    }
    fn replace(&mut self, _: &Replace, window: &mut Window, cx: &mut Context<Self>) {
        let bar = self.panes[self.active_pane].find.clone();
        bar.update(cx, |bar, cx| bar.show(true, window, cx));
    }
    fn replace_one(&mut self, _: &ReplaceOne, _: &mut Window, cx: &mut Context<Self>) {
        let bar = self.panes[self.active_pane].find.clone();
        bar.update(cx, |bar, cx| bar.replace_one(cx));
    }
    fn replace_all(&mut self, _: &ReplaceAll, _: &mut Window, cx: &mut Context<Self>) {
        let bar = self.panes[self.active_pane].find.clone();
        bar.update(cx, |bar, cx| bar.replace_all(cx));
    }
    fn next_occurrence(&mut self, _: &NextOccurrence, _: &mut Window, cx: &mut Context<Self>) {
        let bar = self.panes[self.active_pane].find.clone();
        bar.update(cx, |bar, cx| bar.next(cx));
    }
    fn prev_occurrence(&mut self, _: &PrevOccurrence, _: &mut Window, cx: &mut Context<Self>) {
        let bar = self.panes[self.active_pane].find.clone();
        bar.update(cx, |bar, cx| bar.prev(cx));
    }
    fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        self.request_quit(cx);
    }

    /// `split-right`: a new split to the right of the active one, holding a
    /// second view on the active tab's document (same buffer, its own caret),
    /// which takes the focus.
    fn split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.active_pane;
        let source = self.panes[p]
            .active_tab()
            .and_then(Tab::editor)
            .map(|e| e.read(cx).document.clone());
        let mut pane = Pane::new(cx);
        if let Some(document) = source {
            let editor = self.new_editor(document, cx);
            pane.tabs.push(Tab {
                view: TabView::Editor(editor),
                preview: false,
            });
            pane.active = Some(0);
        }
        self.panes.insert(p + 1, pane);
        self.active_pane = p + 1;
        self.focus_editor_now(window, cx);
        cx.notify();
    }

    /// `unsplit`: the active split's tabs join the one to its left (or right).
    fn unsplit(&mut self, _: &Unsplit, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes.len() < 2 {
            return;
        }
        let p = self.active_pane;
        let target = if p > 0 { p - 1 } else { 1 };
        let pane = self.panes.remove(p);
        let target = if target > p { target - 1 } else { target };
        let dest = &mut self.panes[target];
        let first = dest.tabs.len();
        dest.tabs.extend(pane.tabs);
        if let Some(a) = pane.active {
            dest.active = Some(first + a);
        } else if dest.active.is_none() && !dest.tabs.is_empty() {
            dest.active = Some(0);
        }
        self.active_pane = target;
        self.focus_editor_now(window, cx);
        self.after_activate(cx);
    }

    fn focus_pane(&mut self, p: usize, window: &mut Window, cx: &mut Context<Self>) {
        if p < self.panes.len() {
            self.active_pane = p;
            self.focus_editor_now(window, cx);
            self.after_activate(cx);
        }
    }
    fn focus_next_split(
        &mut self,
        _: &FocusNextSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = (self.active_pane + 1) % self.panes.len();
        self.focus_pane(p, window, cx);
    }
    fn focus_previous_split(
        &mut self,
        _: &FocusPreviousSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.panes.len();
        self.focus_pane((self.active_pane + n - 1) % n, window, cx);
    }
    fn focus_first_split(
        &mut self,
        _: &FocusFirstSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane(0, window, cx);
    }
    fn focus_last_split(
        &mut self,
        _: &FocusLastSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane(self.panes.len() - 1, window, cx);
    }

    /// Moves the active tab to another split; past the last split a new one opens.
    fn move_active_tab(&mut self, to: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.active_pane;
        let Some(ix) = self.panes[p].active else {
            return;
        };
        let to = match to {
            Some(to) => to,
            None => {
                self.panes.insert(p + 1, Pane::new(cx));
                p + 1
            }
        };
        if to == p {
            return;
        }
        self.move_tab((p, ix), to, None, cx);
        self.focus_editor_now(window, cx);
    }
    fn move_tab_next(
        &mut self,
        _: &MoveTabToNextSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = self.active_pane;
        let to = (p + 1 < self.panes.len()).then_some(p + 1);
        self.move_active_tab(to, window, cx);
    }
    fn move_tab_previous(
        &mut self,
        _: &MoveTabToPreviousSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = self.active_pane;
        if p == 0 {
            return;
        }
        self.move_active_tab(Some(p - 1), window, cx);
    }
    fn move_tab_first(
        &mut self,
        _: &MoveTabToFirstSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active_tab(Some(0), window, cx);
    }
    fn move_tab_last(
        &mut self,
        _: &MoveTabToLastSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active_tab(Some(self.panes.len() - 1), window, cx);
    }

    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        settings::zoom_in(cx);
    }
    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        settings::zoom_out(cx);
    }
    fn reset_zoom(&mut self, _: &ResetZoom, _: &mut Window, cx: &mut Context<Self>) {
        settings::reset_zoom(cx);
    }
    fn toggle_full_screen(
        &mut self,
        _: &ToggleFullScreen,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }
    fn minimize_window(&mut self, _: &MinimizeWindow, window: &mut Window, _: &mut Context<Self>) {
        window.minimize_window();
    }
    fn edit_settings_file(&mut self, _: &EditSettingsFile, _: &mut Window, cx: &mut Context<Self>) {
        let path = settings::settings_file(cx);
        self.open_own_file(path, "{\n}\n", cx);
    }
    fn edit_keymap_file(&mut self, _: &EditKeymapFile, _: &mut Window, cx: &mut Context<Self>) {
        self.open_own_file(settings::keymap_file(), "{\n  \"keymap\": [\n  ]\n}\n", cx);
    }

    // ---- splitters --------------------------------------------------------------

    /// The pointer dragging the gap beside a panel: the island's edge follows
    /// it, and a panel pulled under its minimum collapses (batch 2).
    fn splitter_moved(
        &mut self,
        side: Side,
        pointer: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let m = *cx.global::<Metrics>();
        let viewport = window.viewport_size();
        let half = m.island_margin / 2.;
        let (size, min) = match side {
            Side::Left => (
                f32::from(pointer.x) - half - m.island_margin,
                m.panel_min_width,
            ),
            Side::Right => (
                f32::from(viewport.width) - m.island_margin - (f32::from(pointer.x) + half),
                m.panel_min_width,
            ),
            Side::Bottom => (
                f32::from(viewport.height) - m.status_bar_height - (f32::from(pointer.y) + half),
                m.panel_min_height,
            ),
        };
        let panel = self.panel_mut(side);
        if size < min {
            if panel.open {
                panel.open = false;
                panel.size = min / m.zoom;
            }
        } else {
            panel.open = true;
            panel.size = size / m.zoom;
        }
        cx.notify();
    }

    // ---- the chooser ----------------------------------------------------------

    fn open_chooser(
        &mut self,
        side: Side,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let m = *cx.global::<Metrics>();
        let origin = point(
            anchor.origin.x + px(m.popup_offset_x),
            anchor.origin.y + anchor.size.height + px(m.popup_offset_y),
        );
        let chooser = Chooser {
            side,
            origin,
            query: String::new(),
            selected: 0,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        };
        chooser.focus_handle.focus(window, cx);
        self.chooser = Some(chooser);
        cx.notify();
    }

    fn close_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chooser.take().is_some() {
            self.focus_editor_now(window, cx);
            cx.notify();
        }
    }

    fn choose_tool(&mut self, side: Side, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        if tool == Tool::Terminal {
            // The last terminal, or a new shell (M5).
            self.terminal_jump_to_tool(&TerminalJumpToTool, window, cx);
            return;
        }
        let panel = self.panel_mut(side);
        let ix = match panel.tools.iter().position(|t| t.tool == tool) {
            Some(ix) => ix,
            None => {
                panel.tools.push(ToolTab::single(tool));
                panel.tools.len() - 1
            }
        };
        panel.active = ix;
        panel.open = true;
        self.chooser = None;
        self.focus_tool_tab(side, ix, window, cx);
        self.save_layout();
        cx.notify();
    }

    fn popup_up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = &mut self.chooser {
            c.selected = c.selected.saturating_sub(1);
            c.scroll.scroll_to_item(c.selected);
            cx.notify();
        }
    }
    fn popup_down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = &mut self.chooser {
            let n = c.matches().len();
            if n > 0 {
                c.selected = (c.selected + 1).min(n - 1);
                c.scroll.scroll_to_item(c.selected);
            }
            cx.notify();
        }
    }
    fn popup_confirm(&mut self, _: &PopupConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = &self.chooser {
            let side = c.side;
            if let Some(tool) = c.matches().get(c.selected).copied() {
                self.choose_tool(side, tool, window, cx);
            }
        }
    }
    fn popup_cancel(&mut self, _: &PopupCancel, window: &mut Window, cx: &mut Context<Self>) {
        self.close_chooser(window, cx);
    }
    fn popup_backspace(&mut self, _: &PopupBackspace, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = &mut self.chooser {
            c.query.pop();
            c.selected = 0;
            cx.notify();
        }
    }
    fn popup_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &mut self.chooser else { return };
        let ks = &event.keystroke;
        if ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform {
            return;
        }
        let typed = ks
            .key_char
            .clone()
            .or_else(|| (ks.key.chars().count() == 1).then(|| ks.key.clone()));
        if let Some(t) = typed {
            if t.chars().all(|ch| !ch.is_control()) {
                c.query.push_str(&t);
                c.selected = 0;
                cx.notify();
            }
        }
    }

    // ---- rendering ------------------------------------------------------------

    fn island(&self, theme: &Theme, m: &Metrics) -> Div {
        div()
            .bg(theme.island_background)
            .rounded(px(m.island_radius))
            .overflow_hidden()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tab(
        &self,
        p: usize,
        ix: usize,
        theme: &Theme,
        m: &Metrics,
        window: &Window,
        cx: &Context<Self>,
        tui: Option<crate::skin::Cell>,
    ) -> Stateful<Div> {
        let pane = &self.panes[p];
        let tab = &pane.tabs[ix];
        let selected = pane.active == Some(ix);
        let focused = selected && tab.focus_handle(cx).is_focused(window);
        let title: SharedString = tab.title(cx).into();
        let dirty = tab.is_dirty(cx);
        let preview = tab.preview;
        let icon_path = tab.icon(cx);
        let path_text: SharedString = tab
            .path(cx)
            .as_deref()
            .map(display_path)
            .unwrap_or_else(|| tab.title(cx))
            .into();
        let close_left = Self::settings(cx).tab_close_button_on_the_left();
        let entity = cx.entity();
        let ghost_title = title.clone();
        let (bg, border, text) = if focused {
            (
                theme.tab_bg_selected_focused,
                theme.tab_border_selected_focused,
                theme.tab_text_selected,
            )
        } else if selected {
            (
                theme.tab_bg_selected,
                theme.tab_border_selected,
                theme.tab_text_selected,
            )
        } else {
            (
                gpui::transparent_black(),
                gpui::transparent_black(),
                theme.tab_text,
            )
        };
        let hidden = self.dragging_tab == Some((p, ix));
        let hovered_bg = theme.tab_bg_hovered;
        let close_hovered = theme.tab_close_icon_hovered;
        let activate = entity.clone();
        let close = entity.clone();
        let drop = entity.clone();
        let middle = entity.clone();
        let right = entity.clone();
        div()
            .id(ElementId::NamedInteger(
                format!("tab-{p}").into(),
                ix as u64,
            ))
            .group("tab")
            .map(|d| match tui {
                Some(cell) => d
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .when(hidden, |d| d.opacity(0.))
                    .map(|d| {
                        if focused {
                            d.bg(theme.list_bg_focused)
                                .text_color(theme.list_text_focused)
                        } else if selected {
                            d.bg(theme.list_bg_selected).text_color(theme.text_primary)
                        } else {
                            d.bg(theme.island_background)
                                .text_color(theme.text_secondary)
                                .hover(|s| s.text_color(theme.text_primary))
                        }
                    })
                    .when(selected, |d| d.font_weight(FontWeight::BOLD)),
                None => d
                    .h(px(m.editor_tab_height))
                    .pl(px(m.editor_tab_pad_left))
                    .pr(px(m.editor_tab_pad_right))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .rounded(px(m.editor_tab_radius))
                    .border(px(m.editor_tab_border))
                    .border_color(border)
                    .bg(bg)
                    .text_color(text)
                    // The selected tab's label is semibold (Windows probe: 3 px wider).
                    .when(selected, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .cursor_pointer()
                    // The dragged tab's slot stays, empty, until the drop (batch 3).
                    .when(hidden, |d| d.opacity(0.))
                    .when(!selected, |d| {
                        d.hover(|s| {
                            s.bg(theme.tab_bg_hovered)
                                .text_color(theme.tab_text_hovered)
                        })
                    }),
            })
            .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(hovered_bg))
            .on_drop(move |dragged: &DraggedTab, _, cx| {
                let (fp, fi) = (dragged.pane, dragged.index);
                drop.update(cx, |this, cx| {
                    this.dragging_tab = None;
                    this.move_tab((fp, fi), p, Some(ix), cx);
                });
            })
            .on_drag(
                DraggedTab { pane: p, index: ix },
                move |dragged, _, _, cx| {
                    let (dp, di) = (dragged.pane, dragged.index);
                    entity.update(cx, |this, cx| {
                        this.dragging_tab = Some((dp, di));
                        cx.notify();
                    });
                    cx.new(|_| TabGhost {
                        title: ghost_title.clone(),
                        icon: icon_path,
                        dirty,
                    })
                },
            )
            .on_click(move |event, window, cx| {
                activate.update(cx, |this, cx| {
                    if event.click_count() >= 2 {
                        this.panes[p].tabs[ix].preview = false;
                    }
                    this.activate_tab(p, ix, window, cx);
                })
            })
            .on_mouse_down(MouseButton::Middle, move |_, window, cx| {
                middle.update(cx, |this, cx| this.close_tab_at(p, ix, window, cx));
            })
            .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                let position = event.position;
                right.update(cx, |this, cx| {
                    this.tab_context_menu(p, ix, position, window, cx)
                });
            })
            .tooltip(Tooltip::text(path_text))
            .map(|d| match tui {
                Some(cell) => {
                    let (glyph, glyph_color) = crate::skin::file_glyph(&title, theme);
                    let cells = |n: usize| px(n as f32 * cell.w);
                    d.child(div().w(cells(1)).flex_shrink_0())
                        .child(
                            div()
                                .w(cells(1))
                                .flex_shrink_0()
                                .when(!focused, |d| d.text_color(glyph_color))
                                .child(glyph),
                        )
                        .child(div().w(cells(1)).flex_shrink_0())
                        .child(
                            div()
                                .w(cells(title.chars().count()))
                                .flex_shrink_0()
                                .whitespace_nowrap()
                                .when(preview, |d| d.italic())
                                .child(title),
                        )
                        .child(div().w(cells(1)).flex_shrink_0())
                        // `●` while dirty, which the close cross replaces on hover.
                        .child(
                            div()
                                .id("tab-close")
                                .w(cells(2))
                                .flex_shrink_0()
                                .relative()
                                .group("tab-close")
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    close.update(cx, |this, cx| {
                                        this.close_tab_at(p, ix, window, cx)
                                    });
                                })
                                .child(if dirty {
                                    div().group_hover("tab-close", |s| s.invisible()).child("●")
                                } else {
                                    div()
                                        .invisible()
                                        .group_hover("tab", |s| s.visible())
                                        .child("×")
                                })
                                .when(dirty, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .top_0()
                                            .left_0()
                                            .invisible()
                                            .group_hover("tab-close", |s| s.visible())
                                            .child("×"),
                                    )
                                }),
                        )
                }
                None => d
                    .child(
                        div()
                            .size(px(m.icon_size))
                            .flex_shrink_0()
                            .mr(px(m.editor_tab_icon_gap))
                            .relative()
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .when(close_left, |d| d.group_hover("tab", |s| s.invisible()))
                                    .children(
                                        icon_path
                                            .map(|p| icon(p, m.icon_size, theme.icon_secondary)),
                                    ),
                            )
                            .when(close_left, |d| {
                                d.child(
                                    div()
                                        .id("tab-close-left")
                                        .group("tab-close-left")
                                        .absolute()
                                        .inset_0()
                                        .invisible()
                                        .group_hover("tab", |s| s.visible())
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .on_click({
                                            let close = close.clone();
                                            move |_, window, cx| {
                                                cx.stop_propagation();
                                                close.update(cx, |this, cx| {
                                                    this.close_tab_at(p, ix, window, cx)
                                                });
                                            }
                                        })
                                        .child(
                                            icon(
                                                "expui/general/closeSmall",
                                                m.icon_size,
                                                theme.tab_close_icon,
                                            )
                                            .group_hover("tab-close-left", move |s| {
                                                s.text_color(close_hovered)
                                            }),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .whitespace_nowrap()
                            .when(preview, |d| d.italic())
                            .child(title),
                    )
                    .when(dirty, |d| {
                        d.child(
                            div()
                                .w(px(m.editor_tab_dot_box))
                                .h(px(m.icon_size))
                                .ml(px(m.editor_tab_close_gap))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(div().size(px(m.editor_tab_dot)).rounded_full().bg(text)),
                        )
                    })
                    .when(!close_left, |d| {
                        d.child(
                            div()
                                .id("tab-close")
                                .size(px(m.icon_size))
                                .flex_shrink_0()
                                .ml(px(m.editor_tab_close_gap))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(3. * m.zoom))
                                .group("tab-close")
                                .hover(|s| s.bg(theme.ghost_hovered))
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    close.update(cx, |this, cx| {
                                        this.close_tab_at(p, ix, window, cx)
                                    });
                                })
                                .child(
                                    icon(
                                        "expui/general/closeSmall",
                                        m.icon_size,
                                        theme.tab_close_icon,
                                    )
                                    .group_hover("tab-close", move |s| s.text_color(close_hovered)),
                                ),
                        )
                    }),
            })
    }

    fn render_tab_row(
        &self,
        p: usize,
        theme: &Theme,
        m: &Metrics,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let pane = &self.panes[p];
        let tabs: Vec<_> = (0..pane.tabs.len())
            .map(|ix| self.render_tab(p, ix, theme, m, window, cx, None))
            .collect();
        let overflow = pane.scroll.max_offset().x > px(0.);
        let offset = pane.scroll.offset().x;
        let entity = cx.entity();
        let arrow = |which: &'static str, path: &'static str, delta: f32| {
            let scroll = pane.scroll.clone();
            div()
                .id(which)
                .size(px(m.editor_tab_scroll_box))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .on_click(move |_, _, _| {
                    let o = scroll.offset();
                    let max = scroll.max_offset().x;
                    scroll.set_offset(point((o.x + px(delta)).clamp(-max, px(0.)), o.y));
                })
                .child(icon(path, m.icon_size, theme.text_tertiary))
        };
        let drop_end = entity.clone();
        div()
            .h(px(m.editor_tab_bar_height))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px(px(m.panel_pad_x))
            .when(overflow, |d| {
                d.child(arrow(
                    "tabs-left",
                    "expui/general/chevronLeft",
                    120. * m.zoom,
                ))
            })
            .child(
                div()
                    .id(ElementId::NamedInteger("tabs".into(), p as u64))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&pane.scroll)
                    .flex()
                    .items_center()
                    .gap(px(m.editor_tab_gap))
                    .when(overflow, |d| d.pl(px(m.editor_tab_gap)))
                    .children(tabs)
                    .child(
                        // The empty end of the row: a drop here appends.
                        div()
                            .id("tabs-end")
                            .flex_1()
                            .min_w(px(m.editor_tab_gap))
                            .h_full()
                            .on_drop(move |dragged: &DraggedTab, _, cx| {
                                let (fp, fi) = (dragged.pane, dragged.index);
                                drop_end.update(cx, |this, cx| {
                                    this.dragging_tab = None;
                                    this.move_tab((fp, fi), p, None, cx);
                                });
                            }),
                    ),
            )
            .when(overflow && offset < px(0.), |d| d)
            .when(overflow, |d| {
                d.child(arrow(
                    "tabs-right",
                    "expui/general/chevronRight",
                    -120. * m.zoom,
                ))
            })
    }

    fn render_pane(
        &self,
        p: usize,
        theme: &Theme,
        m: &Metrics,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let pane = &self.panes[p];
        let body = match pane.active_tab() {
            Some(tab) => match &tab.view {
                TabView::Editor(editor) => {
                    let find = pane.find.read(cx);
                    let pushed = find.open && !find.query(cx).is_empty();
                    let conflict = editor.read(cx).document.read(cx).conflict;
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .child(
                            div()
                                .size_full()
                                .when(pushed, |d| d.pt(px(m.find_bar_push)))
                                .child(editor.clone()),
                        )
                        .child(pane.find.clone())
                        .when(conflict, |d| {
                            d.child(self.render_banner(editor.clone(), theme, m))
                        })
                }
                TabView::Settings(settings) => {
                    div().flex_1().min_h_0().relative().child(settings.clone())
                }
                TabView::Diff(diff) => div().flex_1().min_h_0().relative().child(diff.clone()),
            },
            None => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(m.empty_panel_gap_title))
                .child(
                    div()
                        .text_color(theme.text_primary)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("No recent files"),
                )
                .child(div().text_color(theme.text_secondary).child(
                    match crate::keymap::hint_for(cx, "new-file") {
                        // The chord as this platform's keymap spells it (Cmd+N on
                        // a Mac, Ctrl+N elsewhere).
                        Some(chord) => format!(
                            "Select a file in the Files panel, or press {chord} for a new one"
                        ),
                        None => "Select a file in the Files panel".to_string(),
                    },
                )),
        };
        let show_tabs = Self::settings(cx).editor_show_tabs()
            && !Self::settings(cx).editor_distraction_free_mode();
        if crate::skin::Skin::current(cx) == crate::skin::Skin::Tui {
            let cell = crate::skin::Cell::measure(window, theme, m);
            let strip: Vec<gpui::AnyElement> = if show_tabs {
                let drop_end = cx.entity();
                vec![div()
                    .id(ElementId::NamedInteger("tui-tabs".into(), p as u64))
                    .flex()
                    .overflow_x_scroll()
                    .track_scroll(&pane.scroll)
                    .children((0..pane.tabs.len()).flat_map(|ix| {
                        [
                            self.render_tab(p, ix, theme, m, window, cx, Some(cell))
                                .into_any_element(),
                            div().w(px(cell.w)).flex_shrink_0().into_any_element(),
                        ]
                    }))
                    .child(
                        div()
                            .id("tabs-end")
                            .w(px(2. * cell.w))
                            .h(px(cell.h))
                            .flex_shrink_0()
                            .on_drop(move |dragged: &DraggedTab, _, cx| {
                                let (fp, fi) = (dragged.pane, dragged.index);
                                drop_end.update(cx, |this, cx| {
                                    this.dragging_tab = None;
                                    this.move_tab((fp, fi), p, None, cx);
                                });
                            }),
                    )
                    .into_any_element()]
            } else {
                Vec::new()
            };
            let focused = pane.focus_handle.contains_focused(window, cx);
            return self
                .island(theme, m)
                .track_focus(&pane.focus_handle)
                .flex_1()
                .min_w_0()
                .child(crate::skin::framed(
                    cell,
                    &pane.tui_grid,
                    strip,
                    body.size_full().into_any_element(),
                    theme,
                    *m,
                    focused,
                ))
                .into_any_element();
        }
        self.island(theme, m)
            .track_focus(&pane.focus_handle)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .when(show_tabs, |d| {
                d.child(self.render_tab_row(p, theme, m, window, cx))
            })
            .child(body)
            .into_any_element()
    }

    /// A panel's tab row: its tools, then the `+` that opens the chooser.
    /// The selected tab takes the `selectedFocused` colours while its panel
    /// has the focus, as the editor tabs do (batch 9: the terminal's tab is
    /// blue while the shell is focused, grey once the editor is). A
    /// terminal's tab carries the tool icon and a close cross.
    fn render_panel_tabs(
        &self,
        side: Side,
        theme: &Theme,
        m: &Metrics,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let panel = self.panel(side);
        let entity = cx.entity();
        let panel_focused = panel.focus_handle.contains_focused(window, cx);
        let tabs = panel.tools.iter().enumerate().map(|(ix, tab)| {
            let selected = ix == panel.active;
            // `contains_focused` reads the last rendered frame; a terminal
            // focused as its panel opens is asked directly.
            let own_focus = tab.tool == Tool::Terminal
                && self
                    .terminal_view(tab.instance)
                    .is_some_and(|v| v.read(cx).focus_handle(cx).is_focused(window));
            let focused = selected && (panel_focused || own_focus);
            let entity = entity.clone();
            let menu_entity = cx.entity();
            let close_entity = cx.entity();
            let label = self.tool_tab_label(*tab, cx);
            let is_run = tab.tool == Tool::Run;
            let is_terminal = tab.tool == Tool::Terminal || is_run;
            let run_icon = is_run.then(|| {
                self.run_view(tab.instance)
                    .map(|v| {
                        let v = v.read(cx);
                        (crate::run_popup::kind_icon(v.kind()), v.failed())
                    })
                    .unwrap_or((Tool::Run.icon(), false))
            });
            let tooltip = if is_run {
                self.run_view(tab.instance).map(|v| {
                    let v = v.read(cx);
                    (label.clone(), v.kind().to_string())
                })
            } else if is_terminal {
                self.terminal_view(tab.instance).map(|v| {
                    let v = v.read(cx);
                    (
                        label.clone(),
                        format!("{} {}", v.profile().name, display_path(&self.root)),
                    )
                })
            } else {
                None
            };
            // The border is always laid out (transparent until focused), so
            // a text tab's measured padding is taken less its width.
            let border = m.editor_tab_border;
            let (pad_left, pad_right) = if is_terminal {
                (m.panel_tab_icon_pad - border, 0.)
            } else if selected {
                (
                    m.panel_tab_selected_pad_x - border,
                    m.panel_tab_selected_pad_x - border,
                )
            } else {
                (m.panel_tab_pad_x - border, m.panel_tab_pad_x - border)
            };
            let (bg, border_color, text) = if focused {
                (
                    theme.tab_bg_selected_focused,
                    theme.tab_border_selected_focused,
                    theme.tab_text_selected,
                )
            } else if selected {
                (
                    theme.tab_bg_selected,
                    theme.island_background,
                    theme.tab_text_selected,
                )
            } else {
                (
                    gpui::transparent_black(),
                    gpui::transparent_black(),
                    theme.tab_text,
                )
            };
            let close_hovered = theme.tab_close_icon_hovered;
            div()
                .id(ElementId::NamedInteger(
                    format!("panel-tab-{}", side.index()).into(),
                    ix as u64,
                ))
                .h(px(m.panel_tab_height))
                .pl(px(pad_left))
                .pr(px(pad_right))
                .when(is_terminal, |d| d.mr(px(m.panel_tab_close_margin)))
                .flex_shrink_0()
                .flex()
                .items_center()
                .rounded(px(m.panel_tab_radius))
                .border(px(border))
                .border_color(border_color)
                .bg(bg)
                .text_color(text)
                .cursor_pointer()
                .when(selected, |d| d.font_weight(FontWeight::SEMIBOLD))
                .when(!selected, |d| d.hover(|s| s.bg(theme.tab_bg_hovered)))
                .when_some(tooltip, |d, (title, detail)| {
                    d.tooltip(Tooltip::with_detail(title, detail))
                })
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.panel_mut(side).active = ix;
                        this.focus_tool_tab(side, ix, window, cx);
                        cx.notify();
                    })
                })
                .on_mouse_down(MouseButton::Right, move |event, _, cx| {
                    let position = event.position;
                    menu_entity.update(cx, |this, cx| this.panel_tab_menu(side, ix, position, cx));
                })
                .when(is_terminal, |d| {
                    let (path, failed) = run_icon.unwrap_or((Tool::Terminal.icon(), false));
                    d.child(
                        div()
                            .size(px(m.icon_size))
                            .flex_shrink_0()
                            .relative()
                            .mr(px(m.panel_tab_icon_gap))
                            .child(icon(path, m.icon_size, theme.icon_secondary))
                            .when(failed, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .right(px(-m.zoom))
                                        .bottom(px(-m.zoom))
                                        .size(px(m.run_tab_badge))
                                        .rounded_full()
                                        .bg(theme.icon_red),
                                )
                            }),
                    )
                })
                .child(div().whitespace_nowrap().child(label))
                .when(is_terminal, |d| {
                    d.child(
                        div()
                            .id("panel-tab-close")
                            .size(px(m.icon_size))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3. * m.zoom))
                            .group("panel-tab-close")
                            .hover(|s| s.bg(theme.ghost_hovered))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                close_entity.update(cx, |this, cx| {
                                    this.close_tool_tab(side, ix, window, cx)
                                });
                            })
                            .child(
                                icon(
                                    "expui/general/closeSmall",
                                    m.icon_size,
                                    theme.tab_close_icon,
                                )
                                .group_hover("panel-tab-close", move |s| {
                                    s.text_color(close_hovered)
                                }),
                            ),
                    )
                })
        });
        let anchor = panel.plus_anchor.clone();
        let anchor_read = panel.plus_anchor.clone();
        let open = entity.clone();
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(m.panel_tab_gap))
            .pt(px(m.panel_header_pad_top))
            .pb(px(m.panel_header_pad_bottom))
            .px(px(m.panel_pad_x))
            .overflow_hidden()
            .children(tabs)
            .child(
                div()
                    .id(ElementId::NamedInteger(
                        "panel-plus".into(),
                        side.index() as u64,
                    ))
                    .ml(px(m.panel_tab_plus_gap - m.panel_tab_gap))
                    .size(px(m.panel_tab_height))
                    .flex_shrink_0()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(m.panel_tab_radius))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.ghost_hovered))
                    .on_click(move |_, window, cx| {
                        let bounds = anchor_read.get();
                        open.update(cx, |this, cx| this.open_chooser(side, bounds, window, cx));
                    })
                    .child(
                        canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .child(icon("expui/general/add", m.icon_size, theme.icon_secondary)),
            )
    }

    /// Fleet's empty panel: icon, title, hint and the "Open Tool" button (batch 2).
    fn render_empty_panel(
        &self,
        side: Side,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let panel = self.panel(side);
        let entity = cx.entity();
        let anchor = panel.button_anchor.clone();
        let anchor_read = panel.button_anchor.clone();
        let ring = theme.focus_outline;
        let border_focused = theme.border_focused;
        let outline_width = m.focus_outline;
        div().size_full().flex().items_center().child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                // The content box is inset 15 % of the island's width on each side.
                .mx(gpui::relative(EMPTY_PANEL_INSET))
                .child(icon(side.icon(), m.empty_panel_icon, theme.icon_primary))
                .child(
                    div()
                        .mt(px(m.empty_panel_gap_icon))
                        .text_size(px(m.empty_panel_title_size))
                        .line_height(px(m.empty_panel_title_line))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text_primary)
                        .child(side.title()),
                )
                .child(
                    div()
                        .mt(px(m.empty_panel_gap_title))
                        .line_height(px(m.empty_panel_text_line))
                        .text_color(theme.text_secondary)
                        .child("Open new tool or drag-n-drop tool from other panels"),
                )
                .child(
                    div()
                        .id(ElementId::NamedInteger(
                            "open-tool".into(),
                            side.index() as u64,
                        ))
                        .track_focus(&panel.button_focus)
                        .mt(px(m.empty_panel_gap_text))
                        .h(px(m.button_height))
                        .px(px(m.button_pad_x))
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(m.button_icon_gap))
                        .rounded(px(m.button_radius))
                        .border_1()
                        .border_color(theme.button_secondary_border)
                        .bg(theme.button_secondary_bg)
                        .text_color(theme.button_secondary_text)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.button_secondary_bg_hovered))
                        .active(|s| s.bg(theme.button_secondary_bg_pressed))
                        .focus(move |s| {
                            s.border_color(border_focused).shadow(vec![BoxShadow {
                                color: ring,
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: px(outline_width),
                            }])
                        })
                        .on_click(move |_, window, cx| {
                            let bounds = anchor_read.get();
                            entity
                                .update(cx, |this, cx| this.open_chooser(side, bounds, window, cx));
                        })
                        .child(
                            canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                                .absolute()
                                .size_full(),
                        )
                        .child("Open Tool")
                        .child(
                            div()
                                .w(px(m.button_chevron_box))
                                .h(px(m.icon_size))
                                .flex()
                                .items_center()
                                .justify_center()
                                .overflow_hidden()
                                .child(icon(
                                    "expui/general/chevronDown",
                                    m.icon_size,
                                    theme.icon_primary,
                                )),
                        ),
                ),
        )
    }

    fn render_panel(
        &self,
        side: Side,
        theme: &Theme,
        m: &Metrics,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let panel = self.panel(side);
        let content: gpui::AnyElement = if panel.tools.is_empty() {
            self.render_empty_panel(side, theme, m, cx)
                .into_any_element()
        } else {
            let tab = panel
                .tools
                .get(panel.active)
                .copied()
                .unwrap_or(ToolTab::single(Tool::Files));
            let body: gpui::AnyElement = match tab.tool {
                Tool::Files => self.file_tree.clone().into_any_element(),
                Tool::Search => self.search_tool.clone().into_any_element(),
                Tool::Git => self.git_tool.clone().into_any_element(),
                Tool::History => self.history_tool.clone().into_any_element(),
                Tool::Notifications => self.notifications_view().into_any_element(),
                Tool::Terminal => match self.terminal_view(tab.instance) {
                    Some(view) => view.clone().into_any_element(),
                    None => div().size_full().into_any_element(),
                },
                Tool::Run => match self.run_view(tab.instance) {
                    Some(view) => view.clone().into_any_element(),
                    None => div().size_full().into_any_element(),
                },
                other => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.text_secondary)
                    .child(other.label())
                    .into_any_element(),
            };
            if crate::skin::Skin::current(cx) == crate::skin::Skin::Tui {
                self.render_tui_panel(side, body, theme, m, window, cx)
            } else {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(self.render_panel_tabs(side, theme, m, window, cx))
                    .child(div().flex_1().min_h_0().child(body))
                    .into_any_element()
            }
        };
        self.island(theme, m)
            .track_focus(&panel.focus_handle)
            .flex_shrink_0()
            .map(|d| match side {
                Side::Bottom => d.h(px(panel.size * m.zoom)).w_full(),
                _ => d.w(px(panel.size * m.zoom)).h_full(),
            })
            .child(content)
    }

    /// A panel in the tui skin: a frame round the island's whole cells, the
    /// tool tabs set into its top edge as labels (the frame's line shows
    /// between them), and the tool inside, one cell in from every side and
    /// sized in whole cells. The grid is read back while painting, so a
    /// resize settles one frame later.
    fn render_tui_panel(
        &self,
        side: Side,
        body: gpui::AnyElement,
        theme: &Theme,
        m: &Metrics,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        use crate::skin::Cell as Grid;
        let panel = self.panel(side);
        let cell = Grid::measure(window, theme, m);
        let cols = panel.tui_grid.get().0;
        let panel_focused = panel.focus_handle.contains_focused(window, cx);
        // One label: ` name ` on the island's colour, so it cuts the frame;
        // the active tab in reverse video. Widths are whole cells.
        let label = |text: String, active: bool, focused: bool| {
            let chars = text.chars().count();
            div()
                .h(px(cell.h))
                .w(px(chars as f32 * cell.w))
                .flex_shrink_0()
                .whitespace_nowrap()
                .cursor_pointer()
                .map(|d| {
                    if focused {
                        d.bg(theme.list_bg_focused)
                            .text_color(theme.list_text_focused)
                    } else if active {
                        d.bg(theme.list_bg_selected).text_color(theme.text_primary)
                    } else {
                        d.bg(theme.island_background)
                            .text_color(theme.text_secondary)
                            .hover(|s| s.text_color(theme.text_primary))
                    }
                })
                .when(active, |d| d.font_weight(FontWeight::BOLD))
                .child(text)
        };
        let mut used = 2; // the corner and one cell of line before the first tab
        let mut strip: Vec<gpui::AnyElement> = Vec::new();
        for (ix, tab) in panel.tools.iter().enumerate() {
            let active = ix == panel.active;
            let closable = matches!(tab.tool, Tool::Terminal | Tool::Run);
            let failed = tab.tool == Tool::Run
                && self
                    .run_view(tab.instance)
                    .is_some_and(|v| v.read(cx).failed());
            let mut text = format!(" {}", self.tool_tab_label(*tab, cx));
            if failed {
                text.push_str(" !");
            }
            text.push(' ');
            let width = text.chars().count() + if closable { 2 } else { 0 };
            if used + width + 1 > cols.saturating_sub(5) && ix > 0 {
                break; // no room: the rest stay behind `+`, as Fleet clips them
            }
            used += width + 1;
            let select = cx.entity();
            let menu = cx.entity();
            let close = cx.entity();
            strip.push(
                div()
                    .id(ElementId::NamedInteger(
                        format!("tui-panel-tab-{}", side.index()).into(),
                        ix as u64,
                    ))
                    .flex()
                    .flex_shrink_0()
                    .on_click(move |_, window, cx| {
                        select.update(cx, |this, cx| {
                            this.panel_mut(side).active = ix;
                            this.focus_tool_tab(side, ix, window, cx);
                            cx.notify();
                        })
                    })
                    .on_mouse_down(MouseButton::Right, move |event, _, cx| {
                        let position = event.position;
                        menu.update(cx, |this, cx| this.panel_tab_menu(side, ix, position, cx));
                    })
                    .child(
                        label(text, active, active && panel_focused)
                            .when(failed, |d| d.text_color(theme.icon_red)),
                    )
                    .when(closable, |d| {
                        d.child(
                            div()
                                .id("tui-panel-tab-close")
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    close.update(cx, |this, cx| {
                                        this.close_tool_tab(side, ix, window, cx)
                                    });
                                })
                                .child(
                                    label("× ".into(), active, active && panel_focused)
                                        .hover(|s| s.text_color(theme.icon_red)),
                                ),
                        )
                    })
                    .into_any_element(),
            );
            // A cell of the frame's line between labels.
            strip.push(div().w(px(cell.w)).flex_shrink_0().into_any_element());
        }
        let anchor = panel.plus_anchor.clone();
        let anchor_read = panel.plus_anchor.clone();
        let open = cx.entity();
        strip.push(
            div()
                .id(ElementId::NamedInteger(
                    "tui-panel-plus".into(),
                    side.index() as u64,
                ))
                .relative()
                .on_click(move |_, window, cx| {
                    let bounds = anchor_read.get();
                    open.update(cx, |this, cx| this.open_chooser(side, bounds, window, cx));
                })
                .child(
                    canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                )
                .child(label(" + ".into(), false, false))
                .into_any_element(),
        );
        crate::skin::framed(cell, &panel.tui_grid, strip, body, theme, *m, panel_focused)
            .into_any_element()
    }

    /// The 8 px gap beside a panel: the splitter (batch 2).
    fn render_splitter(&self, side: Side, m: &Metrics) -> impl IntoElement {
        div()
            .id(ElementId::NamedInteger(
                "splitter".into(),
                side.index() as u64,
            ))
            .flex_shrink_0()
            .map(|d| match side {
                Side::Bottom => d.h(px(m.island_margin)).w_full().cursor_row_resize(),
                _ => d.w(px(m.island_margin)).h_full().cursor_col_resize(),
            })
            .on_drag(SplitterDrag { side }, |_, _, _, cx| cx.new(|_| EmptyView))
    }

    fn render_chooser(
        &self,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let chooser = self.chooser.as_ref()?;
        let side = chooser.side;
        let matches = chooser.matches();
        let entity = cx.entity();
        let rows = matches.iter().enumerate().map(|(ix, tool)| {
            let tool = *tool;
            let selected = ix == chooser.selected;
            let entity = entity.clone();
            div()
                .id(ElementId::NamedInteger("chooser-row".into(), ix as u64))
                .h(px(m.popup_row_height))
                .pl(px(m.popup_row_pad_x))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(m.popup_row_icon_gap))
                .rounded(px(m.popup_row_radius))
                .cursor_pointer()
                .text_color(theme.popup_text)
                .when(selected, |d| {
                    d.bg(theme.list_bg_focused)
                        .text_color(theme.list_text_focused)
                })
                .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.choose_tool(side, tool, window, cx));
                })
                .child(icon(tool.icon(), m.icon_size, theme.icon_primary))
                .child(tool.label())
        });
        let close = entity.clone();
        let query: SharedString = chooser.query.clone().into();
        Some(deferred(
            div()
                .id("tool-chooser")
                .key_context("Popup")
                .track_focus(&chooser.focus_handle)
                .on_action(cx.listener(Self::popup_up))
                .on_action(cx.listener(Self::popup_down))
                .on_action(cx.listener(Self::popup_confirm))
                .on_action(cx.listener(Self::popup_cancel))
                .on_action(cx.listener(Self::popup_backspace))
                .on_key_down(cx.listener(Self::popup_key))
                .on_mouse_down_out(move |_, window, cx| {
                    close.update(cx, |this, cx| this.close_chooser(window, cx));
                })
                .absolute()
                .left(chooser.origin.x)
                .top(chooser.origin.y)
                .w(px(m.popup_width))
                .max_h(px(m.popup_max_height))
                .flex()
                .flex_col()
                .p(px(m.popup_pad))
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
                .child(
                    div()
                        .h(px(m.popup_field_height))
                        .flex_shrink_0()
                        .pl(px(m.popup_row_pad_x))
                        .flex()
                        .items_center()
                        .gap(px(m.popup_row_icon_gap))
                        .child(icon(
                            "expui/general/search",
                            m.icon_size,
                            theme.icon_secondary,
                        ))
                        .map(|d| {
                            if query.is_empty() {
                                d.child(div().text_color(theme.text_tertiary).child("Search"))
                            } else {
                                d.child(div().text_color(theme.popup_text).child(query.clone()))
                            }
                        })
                        .child(div().w(px(1.)).h(px(m.icon_size)).bg(theme.editor_caret)),
                )
                .child(
                    div()
                        .my(px(m.popup_pad))
                        .h(px(1.))
                        .flex_shrink_0()
                        .bg(theme.popup_separator),
                )
                .child(
                    div()
                        .relative()
                        .min_h_0()
                        .child(
                            div()
                                .id("chooser-list")
                                .w_full()
                                // As tall as its rows, up to the popup's cap, then scrolling.
                                .max_h(px(m.popup_max_height
                                    - 4. * m.popup_pad
                                    - m.popup_field_height
                                    - 1.))
                                .overflow_y_scroll()
                                .track_scroll(&chooser.scroll)
                                .flex()
                                .flex_col()
                                .children(rows),
                        )
                        .child(scrollbar(
                            "chooser-scrollbar",
                            &chooser.scroll,
                            m.popup_scrollbar_thumb,
                            theme,
                            m,
                        )),
                ),
        ))
    }

    fn render_status_bar(
        &self,
        theme: &Theme,
        m: &Metrics,
        tui: Option<crate::skin::Cell>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut crumbs: Vec<SharedString> = Vec::new();
        let mut right: Vec<SharedString> = Vec::new();
        if let Some(editor) = self.active_editor().cloned() {
            let nav_bar = Self::settings(cx).editor_show_nav_bar();
            let (path, title, crlf, caret, language, chain) = editor.update(cx, |e, cx| {
                (
                    e.path(cx),
                    e.title(cx),
                    e.uses_crlf(cx),
                    e.cursor_position(cx),
                    e.language(cx),
                    if nav_bar { e.nav_chain(cx) } else { Vec::new() },
                )
            });
            if let Some(path) = &path {
                let rel = path.strip_prefix(&self.root).unwrap_or(path.as_path());
                crumbs.extend(
                    rel.components()
                        .map(|c| c.as_os_str().to_string_lossy().to_string().into()),
                );
            } else {
                crumbs.push(title.into());
            }
            // `editor.showNavBar`: the symbols around the caret follow the path
            // (thirteenth pass: "src / main.rs / Record").
            crumbs.extend(chain.into_iter().map(SharedString::from));
            // Fleet's order: line ending, caret, encoding, language (Windows probe).
            right.push(if crlf { "CRLF".into() } else { "LF".into() });
            right.push(format!("{}:{}", caret.row + 1, caret.column + 1).into());
            // Only UTF-8 text opens (`document_for`), so the encoding is fixed.
            right.push("UTF-8".into());
            right.push(
                language
                    .map(|l| l.display_name())
                    .unwrap_or("Plain text")
                    .into(),
            );
        }
        let entity = cx.entity();
        if let Some(cell) = tui {
            return self
                .render_tui_status_bar(cell, crumbs, right, theme, m, cx)
                .into_any_element();
        }
        let sep = |theme: &Theme| {
            div()
                .px(px(7. * m.zoom))
                .text_color(theme.text_tertiary)
                .child("/")
        };
        div()
            .h(px(m.status_bar_height))
            .flex_shrink_0()
            .flex()
            .items_center()
            .pl(px(m.island_margin + m.status_pad_x))
            .pr(px(m.island_margin + m.status_pad_right))
            .text_color(theme.status_text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .children(crumbs.into_iter().enumerate().flat_map(|(i, c)| {
                        let mut parts = Vec::new();
                        if i > 0 {
                            parts.push(sep(theme));
                        }
                        parts.push(div().child(c));
                        parts
                    })),
            )
            .child(div().flex_1())
            .child(div().flex().items_center().gap(px(m.status_gap)).children(
                right.into_iter().enumerate().map(|(i, t)| {
                    // The line ending opens `editor/change-line-endings`.
                    let entity = entity.clone();
                    div()
                        .id(ElementId::NamedInteger("status".into(), i as u64))
                        .when(i == 0, |d| {
                            d.cursor_pointer().on_click(move |_, window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.change_line_endings(&ChangeLineEndings, window, cx)
                                })
                            })
                        })
                        .child(t)
                }),
            ))
            .into_any_element()
    }

    /// The status bar in the tui skin: one row of cells, as tmux draws it. The
    /// workspace's name in reverse video, the breadcrumbs, and on the right
    /// the line ending, caret, encoding and language between `│` rules. The
    /// text starts in the panels' first column of content.
    fn render_tui_status_bar(
        &self,
        cell: crate::skin::Cell,
        crumbs: Vec<SharedString>,
        right: Vec<SharedString>,
        theme: &Theme,
        m: &Metrics,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let cells = |n: usize| px(n as f32 * cell.w);
        let seg = |text: String| {
            div()
                .w(cells(text.chars().count()))
                .flex_shrink_0()
                .whitespace_nowrap()
                .child(text)
        };
        let rule = |text: &'static str| seg(text.into()).text_color(theme.text_tertiary);
        let name = format!(" {} ", self.file_tree.read(cx).root_name());
        let entity = cx.entity();
        div()
            .h(px(cell.h))
            .flex_shrink_0()
            .flex()
            .items_center()
            .font_family(theme.mono_font.clone())
            .text_size(px(cell.font_size))
            .line_height(px(cell.h))
            .pl(px(m.island_margin + cell.w))
            .pr(px(m.island_margin + cell.w))
            .text_color(theme.status_text)
            .child(
                seg(name)
                    .bg(theme.list_bg_focused)
                    .text_color(theme.list_text_focused)
                    .font_weight(FontWeight::BOLD),
            )
            .child(div().w(cells(1)).flex_shrink_0())
            .child(div().flex().min_w_0().overflow_hidden().children(
                crumbs.into_iter().enumerate().flat_map(|(i, c)| {
                    let mut parts = Vec::new();
                    if i > 0 {
                        parts.push(rule(" / "));
                    }
                    parts.push(seg(c.to_string()));
                    parts
                }),
            ))
            .child(div().flex_1())
            .children(right.into_iter().enumerate().flat_map(|(i, t)| {
                let entity = entity.clone();
                let item = div()
                    .id(ElementId::NamedInteger("status".into(), i as u64))
                    .flex_shrink_0()
                    .when(i == 0, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.text_color(theme.text_primary))
                            .on_click(move |_, window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.change_line_endings(&ChangeLineEndings, window, cx)
                                })
                            })
                    })
                    .child(seg(t.to_string()));
                let mut parts = Vec::new();
                if i > 0 {
                    parts.push(rule(" │ ").into_any_element());
                }
                parts.push(item.into_any_element());
                parts
            }))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        // The split holding the focus is the active one.
        if let Some(p) = self
            .panes
            .iter()
            .position(|pane| pane.focus_handle.contains_focused(window, cx))
        {
            self.active_pane = p;
        }
        // Each split's find bar searches the split's active editor.
        for pane in &self.panes {
            let editor = pane.active_tab().and_then(|t| t.editor().cloned());
            pane.find.update(cx, |bar, cx| bar.set_editor(editor, cx));
        }
        let root_name = self.file_tree.read(cx).root_name();
        let title = match self.active_editor() {
            Some(e) => format!("{} — {}", e.read(cx).title(cx), root_name),
            None => root_name.clone(),
        };
        if title != self.last_title {
            window.set_window_title(&title);
            self.last_title = title;
        }
        let entity = cx.entity();
        let (smart, _) = self.smart_indicator();
        let smart_card_open = self.smart.card_open;
        let toolbar = toolbar::render(
            toolbar::ToolbarState {
                workspace_name: root_name.into(),
                branch: self.branch.clone(),
                left_open: self.panels[0].open,
                bottom_open: self.panels[1].open,
                right_open: self.panels[2].open,
                caption: cfg!(windows),
                traffic_lights: cfg!(target_os = "macos") && !window.is_fullscreen(),
                maximized: window.is_maximized(),
                toggle_hints: workbench::toggle_hints(cx),
                unread_notifications: self.notifications.read(cx).unread(),
                smart,
                smart_card_open,
            },
            &theme,
            &m,
            {
                let entity = entity.clone();
                move |which, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.toggle_panel(Side::ALL[which], window, cx)
                    })
                }
            },
            {
                let entity = entity.clone();
                move |at, window, cx| {
                    entity.update(cx, |this, cx| this.show_branches(at, window, cx))
                }
            },
            move |_, _, cx| entity.update(cx, |this, cx| this.lightning_clicked(cx)),
        );
        let left = self.panels[0].open;
        let bottom = self.panels[1].open;
        let right = self.panels[2].open;
        let panes: Vec<_> = (0..self.panes.len())
            .map(|p| self.render_pane(p, &theme, &m, window, cx))
            .collect();
        // Fleet's columns: the left panel, then the editor splits with the bottom
        // panel under them, then the right panel (batch 2).
        let centre = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap(px(m.island_margin))
                    .children(panes),
            )
            .when(bottom, |d| {
                d.child(self.render_splitter(Side::Bottom, &m))
                    .child(self.render_panel(Side::Bottom, &theme, &m, window, cx))
            });
        let middle = div()
            .flex_1()
            .min_h_0()
            .flex()
            .when(left, |d| {
                d.child(self.render_panel(Side::Left, &theme, &m, window, cx))
                    .child(self.render_splitter(Side::Left, &m))
            })
            .child(centre)
            .when(right, |d| {
                d.child(self.render_splitter(Side::Right, &m))
                    .child(self.render_panel(Side::Right, &theme, &m, window, cx))
            });
        let chooser = self.render_chooser(&theme, &m, cx);
        let prompt = self.prompt.clone();
        let drop_tab = cx.entity();
        div()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_all))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::close_other_tabs))
            .on_action(cx.listener(Self::close_tabs_to_the_right))
            .on_action(cx.listener(Self::close_saved_tabs))
            .on_action(cx.listener(Self::reopen_closed_tab))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::toggle_left))
            .on_action(cx.listener(Self::toggle_bottom))
            .on_action(cx.listener(Self::toggle_right))
            .on_action(cx.listener(Self::hide_all))
            .on_action(cx.listener(Self::hide_active))
            .on_action(cx.listener(Self::focus_editor))
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::split_right))
            .on_action(cx.listener(Self::open_in_split))
            .on_action(cx.listener(Self::git_commit_jump_to_tool))
            .on_action(cx.listener(Self::git_file_history))
            .on_action(cx.listener(Self::git_history_jump_to_tool))
            .on_action(cx.listener(Self::git_toggle_blame))
            .on_action(cx.listener(Self::git_push))
            .on_action(cx.listener(Self::git_pull))
            .on_action(cx.listener(Self::git_revert))
            .on_action(cx.listener(Self::git_show_diff))
            .on_action(cx.listener(Self::new_diff_tab))
            .on_action(cx.listener(Self::terminal_new_tool))
            .on_action(cx.listener(Self::terminal_toggle))
            .on_action(cx.listener(Self::terminal_jump_to_tool))
            .on_action(cx.listener(Self::terminal_new_with_profile))
            .on_action(cx.listener(Self::terminal_select_default_profile))
            .on_action(cx.listener(Self::terminal_edit_profiles))
            .on_action(cx.listener(Self::run_and_debug))
            .on_action(cx.listener(Self::run_in_context))
            .on_action(cx.listener(Self::rerun))
            .on_action(cx.listener(Self::stop_run_configurations))
            .on_action(cx.listener(Self::run_stop))
            .on_action(cx.listener(Self::run_edit_configurations))
            .on_action(cx.listener(Self::diff_two_files))
            .on_action(cx.listener(Self::diff_with_clipboard))
            .on_action(cx.listener(Self::unsplit))
            .on_action(cx.listener(Self::focus_next_split))
            .on_action(cx.listener(Self::focus_previous_split))
            .on_action(cx.listener(Self::focus_first_split))
            .on_action(cx.listener(Self::focus_last_split))
            .on_action(cx.listener(Self::move_tab_next))
            .on_action(cx.listener(Self::move_tab_previous))
            .on_action(cx.listener(Self::move_tab_first))
            .on_action(cx.listener(Self::move_tab_last))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_action(cx.listener(Self::toggle_full_screen))
            .on_action(cx.listener(Self::minimize_window))
            .on_action(cx.listener(Self::edit_settings_file))
            .on_action(cx.listener(Self::edit_keymap_file))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::replace))
            .on_action(cx.listener(Self::replace_one))
            .on_action(cx.listener(Self::replace_all))
            .on_action(cx.listener(Self::next_occurrence))
            .on_action(cx.listener(Self::prev_occurrence))
            .on_action(cx.listener(Self::goto_line))
            .on_action(cx.listener(Self::rename_file))
            .on_action(cx.listener(Self::delete_file))
            .on_action(cx.listener(Self::change_line_endings))
            .on_action(cx.listener(Self::paste_from_history))
            .on_action(cx.listener(Self::goto_everything))
            .on_action(cx.listener(Self::goto_file))
            .on_action(cx.listener(Self::goto_symbol))
            .on_action(cx.listener(Self::goto_action))
            .on_action(cx.listener(Self::goto_tool))
            .on_action(cx.listener(Self::goto_recent_file))
            .on_action(cx.listener(Self::show_file_symbols))
            .on_action(cx.listener(Self::toggle_goto_preview))
            .on_action(cx.listener(Self::show_recent_workspaces))
            .on_action(cx.listener(Self::find_in_path))
            .on_action(cx.listener(Self::replace_in_path))
            .on_action(cx.listener(Self::search_jump_to_tool))
            .on_action(cx.listener(Self::files_jump_to_tool))
            .on_action(cx.listener(Self::new_folder))
            .on_action(cx.listener(Self::toggle_hidden_files))
            .on_action(cx.listener(Self::open_in_associated_application))
            .on_action(cx.listener(Self::show_in_file_manager))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::show_context_menu))
            .on_action(cx.listener(Self::show_main_menu))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::toggle_notifications))
            .on_action(cx.listener(Self::clear_notifications))
            .on_action(cx.listener(Self::make_snapshot))
            .on_action(cx.listener(Self::close_all_tabs))
            .on_action(cx.listener(Self::copy_relative_path))
            .on_action(cx.listener(Self::about))
            .on_action(cx.listener(Self::completion))
            .on_action(cx.listener(Self::smart_completion))
            .on_action(cx.listener(Self::show_parameter_info))
            .on_action(cx.listener(Self::show_documentation))
            .on_action(cx.listener(Self::go_to_declaration))
            .on_action(cx.listener(Self::go_to_type_declaration))
            .on_action(cx.listener(Self::find_usages))
            .on_action(cx.listener(Self::rename_symbol))
            .on_action(cx.listener(Self::quick_fixes))
            .on_action(cx.listener(Self::apply_primary_quick_fix))
            .on_action(cx.listener(Self::reformat_code))
            .on_action(cx.listener(Self::optimize_imports))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::forward))
            .on_drag_move(
                cx.listener(|this, e: &DragMoveEvent<SplitterDrag>, window, cx| {
                    let side = e.drag(cx).side;
                    this.splitter_moved(side, e.event.position, window, cx);
                }),
            )
            .on_drop(cx.listener(|this, _: &SplitterDrag, _, _| this.save_layout()))
            .on_drop(move |_: &DraggedTab, _, cx| {
                // Dropped on nothing in particular: the tab stays where it was.
                drop_tab.update(cx, |this, cx| {
                    this.dragging_tab = None;
                    cx.notify();
                });
            })
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.background_primary)
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            .text_color(theme.text_primary)
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .pl(px(m.island_margin))
                    .pr(px(m.island_margin_right))
                    .child(middle),
            )
            .when(Self::settings(cx).editor_show_status_bar(), |d| {
                d.child({
                    let tui = (crate::skin::Skin::current(cx) == crate::skin::Skin::Tui)
                        .then(|| crate::skin::Cell::measure(window, &theme, &m));
                    self.render_status_bar(&theme, &m, tui, cx)
                })
            })
            .children(chooser)
            .children(prompt)
            .children(self.render_workbench_overlays(&theme, &m, cx.entity(), cx))
    }
}

/// Splits what the command line named into the folder to open and the
/// files to show: a file opens its parent folder with the file in a tab.
pub fn resolve_args(paths: &[PathBuf]) -> (PathBuf, Vec<PathBuf>) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let absolute = |p: &Path| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    };
    let mut root: Option<PathBuf> = None;
    let mut files = Vec::new();
    for p in paths {
        let p = absolute(p);
        if p.is_dir() {
            root.get_or_insert(p);
        } else {
            if root.is_none() {
                root = p.parent().map(Path::to_path_buf);
            }
            files.push(p);
        }
    }
    (root.unwrap_or(cwd), files)
}
