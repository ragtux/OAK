//! The window: Fleet's shell. The toolbar across the top, then the islands
//! (the left, bottom and right tool panels when open, the editor splits in
//! the middle, each with its own tab row), then the status bar. Every size
//! comes from [`Metrics`], every colour from [`Theme`]; what was measured
//! where is in `docs/fleet-probe.md`.

use crate::actions::*;
use crate::assets::{file_icon, icon};
use crate::editor::{EditorEvent, EditorView};
use crate::file_tree::{FileTree, FileTreeEvent};
use crate::layout::{Layout, PanelLayout};
use crate::scrollbar::{scrollbar, EmptyView};
use crate::settings::{self, AppSettings};
use crate::theme::{metrics::EMPTY_PANEL_INSET, Metrics, Theme};
use crate::toolbar;
use crate::tooltip::Tooltip;
use gpui::{
    canvas, deferred, div, point, prelude::*, px, App, Bounds, BoxShadow, Context, Div,
    DragMoveEvent, ElementId, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    MouseButton, Pixels, Point, Render, ScrollHandle, SharedString, Stateful, Window,
};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
            Side::Left => "oak/panel-left-24",
            Side::Bottom => "oak/panel-bottom-24",
            Side::Right => "oak/panel-right-24",
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
    const ALL: [Tool; 14] = [
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

    fn label(self) -> &'static str {
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

    fn icon(self) -> &'static str {
        match self {
            Tool::Breakpoints => "oak/breakpoint",
            Tool::BuildOutput => "expui/toolwindows/build",
            Tool::Debug => "expui/run/debug",
            Tool::DebuggerConsole => "oak/console",
            Tool::Files => "expui/nodes/folder",
            Tool::Git => "expui/toolwindows/vcs",
            Tool::History => "expui/general/history",
            Tool::Notifications => "expui/toolwindows/notifications",
            Tool::Preview => "oak/preview",
            Tool::Problems => "expui/toolwindows/problems",
            Tool::Run => "expui/run/run",
            Tool::Search => "expui/toolwindows/find",
            Tool::Snapshots => "oak/snapshot",
            Tool::Terminal => "oak/terminal",
        }
    }
}

struct Panel {
    open: bool,
    /// Width (left, right) or height (bottom), logical px at zoom 1.0.
    size: f32,
    tools: Vec<Tool>,
    active: usize,
    focus_handle: FocusHandle,
    /// The empty panel's "Open Tool" button, which takes focus when the panel opens.
    button_focus: FocusHandle,
    /// Where the `+` and the "Open Tool" button were last laid out, for the chooser.
    plus_anchor: Rc<Cell<Bounds<Pixels>>>,
    button_anchor: Rc<Cell<Bounds<Pixels>>>,
}

impl Panel {
    fn new(layout: PanelLayout, tools: Vec<Tool>, cx: &mut App) -> Panel {
        Panel {
            open: layout.open,
            size: layout.size,
            tools,
            active: 0,
            focus_handle: cx.focus_handle(),
            button_focus: cx.focus_handle(),
            plus_anchor: Rc::new(Cell::new(Bounds::default())),
            button_anchor: Rc::new(Cell::new(Bounds::default())),
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

struct Tab {
    editor: Entity<EditorView>,
    /// A preview tab (`editor.previewTab`): italic title, replaced by the next
    /// single-click file, promoted by an edit or a double click.
    preview: bool,
}

struct Pane {
    tabs: Vec<Tab>,
    active: Option<usize>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
}

impl Pane {
    fn new(cx: &mut App) -> Pane {
        Pane {
            tabs: Vec::new(),
            active: None,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
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
    icon: &'static str,
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
    icon_path: &'static str,
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
        .child(
            div()
                .size(px(m.icon_size))
                .flex_shrink_0()
                .mr(px(m.editor_tab_icon_gap))
                .child(icon(icon_path, m.icon_size, theme.icon_secondary)),
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
    file_tree: Entity<FileTree>,
    panes: Vec<Pane>,
    active_pane: usize,
    panels: [Panel; 3],
    /// Paths of closed tabs, most recent last, for `tabs/reopen-closed-tab`.
    closed: Vec<PathBuf>,
    chooser: Option<Chooser>,
    /// The tab being dragged, whose slot stays empty until the drop.
    dragging_tab: Option<(usize, usize)>,
    toggle_hints: [Option<SharedString>; 3],
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
        toggle_hints: [Option<String>; 3],
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
        let mut this = Workspace {
            branch: git_branch(&root).map(Into::into),
            root,
            file_tree,
            panes: vec![Pane::new(cx)],
            active_pane: 0,
            panels,
            closed: Vec::new(),
            chooser: None,
            dragging_tab: None,
            toggle_hints: toggle_hints.map(|h| h.map(Into::into)),
            focus_handle: cx.focus_handle(),
            last_title: String::new(),
        };
        for path in open {
            this.open_path(path, false, cx);
        }
        if this.panes[0].active.is_some() {
            this.focus_active_editor(cx);
        } else {
            let tree = this.file_tree.clone();
            tree.read(cx).focus_handle(cx).focus(window, cx);
        }
        this
    }

    fn settings(cx: &App) -> &oak_settings::Settings {
        &cx.global::<AppSettings>().settings
    }

    fn layout(&self) -> Layout {
        Layout {
            left: self.panels[0].layout(),
            bottom: self.panels[1].layout(),
            right: self.panels[2].layout(),
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
                .position(|t| t.editor.read(cx).path.as_deref() == Some(path))
                .map(|ix| (p, ix))
        })
    }

    fn new_editor(
        &mut self,
        path: Option<PathBuf>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Entity<EditorView> {
        let editor = cx.new(|cx| EditorView::new(path, text, cx));
        cx.subscribe(&editor, |this, editor, event: &EditorEvent, cx| {
            if let EditorEvent::Changed = event {
                // An edit promotes a preview tab to a real one.
                for pane in &mut this.panes {
                    for tab in &mut pane.tabs {
                        if tab.editor == editor {
                            tab.preview = false;
                        }
                    }
                }
            }
            cx.notify()
        })
        .detach();
        editor
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
        let text = match std::fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => {
                    eprintln!("oak: {} is not UTF-8 text; not opened", path.display());
                    return;
                }
            },
            Err(e) => {
                eprintln!("oak: cannot read {}: {e}", path.display());
                return;
            }
        };
        let editor = self.new_editor(Some(path), &text, cx);
        let p = self.active_pane;
        let pane = &mut self.panes[p];
        let existing_preview = pane
            .tabs
            .iter()
            .position(|t| t.preview && !t.editor.read(cx).is_dirty());
        match (preview, existing_preview) {
            (true, Some(ix)) => {
                pane.tabs[ix].editor = editor;
                pane.active = Some(ix);
            }
            _ => {
                pane.tabs.push(Tab { editor, preview });
                pane.active = Some(pane.tabs.len() - 1);
            }
        }
        let ix = pane.active.unwrap_or(0);
        pane.scroll.scroll_to_item(ix);
        self.after_activate(cx);
    }

    /// Opens a file Oak owns (settings, keymap), creating it with `template` first.
    fn open_own_file(&mut self, path: Option<PathBuf>, template: &str, cx: &mut Context<Self>) {
        let Some(path) = path else { return };
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, template) {
                eprintln!("oak: cannot create {}: {e}", path.display());
                return;
            }
        }
        self.open_path(path, false, cx);
        self.focus_active_editor(cx);
    }

    /// After a tab became active: the tree follows (`files.selectOpenedFile`).
    fn after_activate(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.active_editor().and_then(|e| e.read(cx).path.clone()) {
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
            .map(|t| &t.editor)
    }

    fn focus_active_editor(&self, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            let handle = editor.read(cx).focus_handle(cx);
            cx.defer(move |cx| {
                let _ = cx
                    .windows()
                    .first()
                    .map(|w| w.update(cx, |_, window, cx| handle.focus(window, cx)));
            });
        }
    }

    fn focus_editor_now(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            editor.read(cx).focus_handle(cx).focus(window, cx);
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
        if let Some(path) = tab.editor.read(cx).path.clone() {
            self.closed.push(path);
        }
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
            if let Err(e) = editor.update(cx, |e, cx| e.save(cx)) {
                eprintln!("oak: save failed: {e}");
            }
        }
    }

    fn save_all(&mut self, _: &SaveAll, _: &mut Window, cx: &mut Context<Self>) {
        let editors: Vec<_> = self
            .panes
            .iter()
            .flat_map(|p| p.tabs.iter().map(|t| t.editor.clone()))
            .collect();
        for editor in editors {
            if let Err(e) = editor.update(cx, |e, cx| e.save(cx)) {
                eprintln!("oak: save failed: {e}");
            }
        }
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
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
            if !self.panes[p].tabs[ix].editor.read(cx).is_dirty() {
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
        cx.quit();
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
    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.new_editor(None, "", cx);
        let p = self.active_pane;
        let pane = &mut self.panes[p];
        pane.tabs.push(Tab {
            editor,
            preview: false,
        });
        pane.active = Some(pane.tabs.len() - 1);
        self.focus_editor_now(window, cx);
        cx.notify();
    }
    fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    /// `split-right`: a new split to the right of the active one, holding a
    /// copy of the active tab (its own editor on the same file until buffers
    /// are shared, M2), which takes the focus.
    fn split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.active_pane;
        let source = self.panes[p]
            .active_tab()
            .map(|t| t.editor.read(cx).path.clone());
        let mut pane = Pane::new(cx);
        if let Some(path) = source {
            let editor = match &path {
                Some(path) => match std::fs::read_to_string(path) {
                    Ok(text) => self.new_editor(Some(path.clone()), &text, cx),
                    Err(_) => self.new_editor(None, "", cx),
                },
                None => self.new_editor(None, "", cx),
            };
            pane.tabs.push(Tab {
                editor,
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
        let panel = self.panel_mut(side);
        let ix = match panel.tools.iter().position(|t| *t == tool) {
            Some(ix) => ix,
            None => {
                panel.tools.push(tool);
                panel.tools.len() - 1
            }
        };
        panel.active = ix;
        panel.open = true;
        self.chooser = None;
        if tool == Tool::Files {
            self.file_tree.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            self.panel(side).focus_handle.focus(window, cx);
        }
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

    fn render_tab(
        &self,
        p: usize,
        ix: usize,
        theme: &Theme,
        m: &Metrics,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let pane = &self.panes[p];
        let tab = &pane.tabs[ix];
        let e = tab.editor.read(cx);
        let selected = pane.active == Some(ix);
        let focused = selected && e.focus_handle(cx).is_focused(window);
        let title: SharedString = e.title().into();
        let dirty = e.is_dirty();
        let preview = tab.preview;
        let icon_path = file_icon(&e.title());
        let path_text: SharedString = e
            .path
            .as_ref()
            .map(|p| display_path(p))
            .unwrap_or_else(|| "Untitled".into())
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
        div()
            .id(ElementId::NamedInteger(
                format!("tab-{p}").into(),
                ix as u64,
            ))
            .group("tab")
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
            .tooltip(Tooltip::text(path_text))
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
                            .child(icon(icon_path, m.icon_size, theme.icon_secondary)),
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
                            close.update(cx, |this, cx| this.close_tab_at(p, ix, window, cx));
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
            .map(|ix| self.render_tab(p, ix, theme, m, window, cx))
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
            Some(tab) => div().flex_1().min_h_0().child(tab.editor.clone()),
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
                .child(
                    div()
                        .text_color(theme.text_secondary)
                        .child("Select a file in the Files panel, or press Ctrl+N for a new one"),
                ),
        };
        self.island(theme, m)
            .track_focus(&pane.focus_handle)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .when(Self::settings(cx).editor_show_tabs(), |d| {
                d.child(self.render_tab_row(p, theme, m, window, cx))
            })
            .child(body)
    }

    /// A panel's tab row: its tools, then the `+` that opens the chooser.
    fn render_panel_tabs(
        &self,
        side: Side,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let panel = self.panel(side);
        let entity = cx.entity();
        let tabs = panel.tools.iter().enumerate().map(|(ix, tool)| {
            let selected = ix == panel.active;
            let entity = entity.clone();
            div()
                .id(ElementId::NamedInteger(
                    format!("panel-tab-{}", side.index()).into(),
                    ix as u64,
                ))
                .h(px(m.panel_tab_height))
                .px(px(if selected {
                    m.panel_tab_selected_pad_x
                } else {
                    m.panel_tab_pad_x
                }))
                .flex_shrink_0()
                .flex()
                .items_center()
                .rounded(px(m.panel_tab_radius))
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(theme.tab_bg_selected)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.tab_text_selected)
                })
                .when(!selected, |d| {
                    d.text_color(theme.tab_text)
                        .hover(|s| s.bg(theme.tab_bg_hovered))
                })
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        let panel = this.panel_mut(side);
                        panel.active = ix;
                        if panel.tools.get(ix) == Some(&Tool::Files) {
                            this.file_tree.read(cx).focus_handle(cx).focus(window, cx);
                        }
                        cx.notify();
                    })
                })
                .child(tool.label())
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
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let panel = self.panel(side);
        let content: gpui::AnyElement = if panel.tools.is_empty() {
            self.render_empty_panel(side, theme, m, cx)
                .into_any_element()
        } else {
            let tool = panel
                .tools
                .get(panel.active)
                .copied()
                .unwrap_or(Tool::Files);
            let body: gpui::AnyElement = match tool {
                Tool::Files => self.file_tree.clone().into_any_element(),
                other => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.text_secondary)
                    .child(other.label())
                    .into_any_element(),
            };
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.render_panel_tabs(side, theme, m, cx))
                .child(div().flex_1().min_h_0().child(body))
                .into_any_element()
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
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let mut crumbs: Vec<SharedString> = Vec::new();
        let mut right: Vec<SharedString> = Vec::new();
        if let Some(editor) = self.active_editor() {
            let e = editor.read(cx);
            if let Some(path) = &e.path {
                let rel = path.strip_prefix(&self.root).unwrap_or(path.as_path());
                crumbs.extend(
                    rel.components()
                        .map(|c| c.as_os_str().to_string_lossy().to_string().into()),
                );
            } else {
                crumbs.push(e.title().into());
            }
            // Fleet's order: line ending, caret, encoding, language (Windows probe).
            right.push(if e.text.buffer.uses_crlf() {
                "CRLF".into()
            } else {
                "LF".into()
            });
            let p = e.cursor_position();
            right.push(format!("{}:{}", p.row + 1, p.column + 1).into());
            right.push("UTF-8".into());
            right.push(
                e.language
                    .map(|l| l.display_name())
                    .unwrap_or("Plain text")
                    .into(),
            );
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(m.status_gap))
                    .children(right.into_iter().map(|t| div().child(t))),
            )
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
        let root_name = self.file_tree.read(cx).root_name();
        let title = match self.active_editor() {
            Some(e) => format!("{} — {}", e.read(cx).title(), root_name),
            None => root_name.clone(),
        };
        if title != self.last_title {
            window.set_window_title(&title);
            self.last_title = title;
        }
        let entity = cx.entity();
        let toolbar = toolbar::render(
            toolbar::ToolbarState {
                workspace_name: root_name.into(),
                branch: self.branch.clone(),
                left_open: self.panels[0].open,
                bottom_open: self.panels[1].open,
                right_open: self.panels[2].open,
                caption: cfg!(windows),
                maximized: window.is_maximized(),
                toggle_hints: self.toggle_hints.clone(),
            },
            &theme,
            &m,
            move |which, window, cx| {
                entity.update(cx, |this, cx| {
                    this.toggle_panel(Side::ALL[which], window, cx)
                })
            },
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
                    .child(self.render_panel(Side::Bottom, &theme, &m, cx))
            });
        let middle = div()
            .flex_1()
            .min_h_0()
            .flex()
            .when(left, |d| {
                d.child(self.render_panel(Side::Left, &theme, &m, cx))
                    .child(self.render_splitter(Side::Left, &m))
            })
            .child(centre)
            .when(right, |d| {
                d.child(self.render_splitter(Side::Right, &m))
                    .child(self.render_panel(Side::Right, &theme, &m, cx))
            });
        let chooser = self.render_chooser(&theme, &m, cx);
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
                    .px(px(m.island_margin))
                    .child(middle),
            )
            .when(Self::settings(cx).editor_show_status_bar(), |d| {
                d.child(self.render_status_bar(&theme, &m, cx))
            })
            .children(chooser)
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
