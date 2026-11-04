//! The Files panel: the workspace folder as a tree, laid out the way Fleet
//! draws it. A root row carrying the folder's name, then one 26 px row per
//! entry with a chevron for a folder and a file-type icon for a file;
//! folders holding nothing but one folder collapse into a `a / b` chain
//! (`files.compactDirectories`). A click opens a file (`files.openFileMode`:
//! single or double; a single click opens a preview tab when
//! `editor.previewTab` is on), the file opened in the editor is selected here
//! (`files.selectOpenedFile`), and `files.exclude` names what stays hidden.
//!
//! M3 adds the rest of Fleet's tree: several rows selected at once (Ctrl and
//! Shift with a click or the arrows), `files.keyboardNavigation` with its
//! highlight cursor, `toggle-hidden-files` for dotfiles, dragging rows onto a
//! folder to move them (`dragAndDrop.*`), and the right-click menu, which
//! the workspace builds from the `ContextMenu` event.

use crate::actions::{
    TreeDown, TreeExtendDown, TreeExtendUp, TreeLeft, TreeOpen, TreeRight, TreeSelect, TreeUp,
};
use crate::assets::{file_icon, icon};
use crate::input::{InputEvent, TextInput};
use crate::scrollbar::scrollbar;
use crate::settings::AppSettings;
use crate::skin::{self, Cell, Skin};
use crate::theme::{Metrics, Theme};
use gpui::Entity;
use gpui::{
    div, prelude::*, px, uniform_list, App, Context, ElementId, EventEmitter, FocusHandle,
    Focusable, MouseButton, Pixels, Point, SharedString, Subscription, UniformListScrollHandle,
    Window,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use weep_git::Status;
use weep_settings::{ClickMode, KeyboardNavigation};

#[derive(Clone, Debug)]
pub enum FileTreeEvent {
    /// Show this file: in a preview tab when `preview` (a single click with
    /// `editor.previewTab`), else in a tab of its own.
    Open { path: PathBuf, preview: bool },
    /// A right click on a row (or `show-context-menu` with the tree focused).
    ContextMenu {
        is_dir: bool,
        position: Point<Pixels>,
    },
    /// A row was dropped on a folder and moved on disk.
    Moved { from: PathBuf, to: PathBuf },
    /// The inline rename (`files/rename`) finished on disk.
    Renamed { from: PathBuf, to: PathBuf },
}

/// A row being renamed in place (Fleet turns the row into a text field with
/// the name selected up to its extension).
struct Renaming {
    path: PathBuf,
    input: Entity<TextInput>,
    _subscription: Subscription,
}

/// What a dragged row carries: every selected path.
#[derive(Clone, Debug)]
pub struct DraggedPaths {
    pub paths: Vec<PathBuf>,
}

/// The ghost under the pointer while rows are dragged: the first name, and
/// how many more.
struct DragGhost {
    title: SharedString,
    icon: &'static str,
}

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        if Skin::current(cx) == Skin::Tui {
            let (glyph, color) = if self.icon == "expui/nodes/folder" {
                ("▸", theme.icon_secondary)
            } else {
                skin::file_glyph(&self.title, &theme)
            };
            return crate::workspace::tui_ghost(glyph, color, &self.title, false, &theme, &m, cx);
        }
        div()
            .h(px(m.tree_row_height - 2. * m.tree_row_inset))
            .px(px(m.popup_row_pad_x))
            .flex()
            .items_center()
            .gap(px(m.tree_icon_gap))
            .rounded(px(m.tree_row_radius))
            .border_1()
            .border_color(theme.drag_border)
            .bg(theme.popup_bg)
            .font_family(theme.ui_font.clone())
            .text_size(px(m.ui_font_size))
            .text_color(theme.list_text)
            .child(icon(self.icon, m.icon_size, theme.icon_secondary))
            .child(self.title.clone())
            .into_any_element()
    }
}

#[derive(Debug)]
struct Node {
    /// The path this row stands for: the innermost folder of a compact chain.
    path: PathBuf,
    /// The names shown, more than one for a compact chain.
    names: Vec<String>,
    is_dir: bool,
    expanded: bool,
    children: Option<Vec<Node>>,
}

/// What the tree reads from the settings when it (re)loads.
#[derive(Clone, Debug, PartialEq)]
struct Options {
    compact: bool,
    exclude: Vec<String>,
    open_mode: ClickMode,
    toggle_mode: ClickMode,
    preview: bool,
    select_opened: bool,
    navigation: KeyboardNavigation,
    /// `toggle-hidden-files`: dotfiles shown or not (not a setting; kept per tree).
    show_hidden: bool,
}

impl Options {
    fn from(settings: &weep_settings::Settings, show_hidden: bool) -> Options {
        Options {
            compact: settings.files_compact_directories(),
            exclude: settings.files_exclude(),
            open_mode: settings.files_open_file_mode(),
            toggle_mode: settings.files_toggle_folder_mode(),
            preview: settings.editor_preview_tab(),
            select_opened: settings.files_select_opened_file(),
            navigation: settings.files_keyboard_navigation(),
            show_hidden,
        }
    }

    /// `files.exclude` patterns against a name, and dotfiles when hidden.
    fn excluded(&self, name: &str) -> bool {
        if !self.show_hidden && name.starts_with('.') {
            return true;
        }
        weep_search::files::excluded_by(&self.exclude, name)
    }
}

impl Node {
    fn name(&self) -> String {
        self.names.join(" / ")
    }

    fn load(&mut self, options: &Options) {
        if self.children.is_some() || !self.is_dir {
            return;
        }
        let mut entries = read_dir_sorted(&self.path, options);
        // Fleet's compact folders: a folder whose only entry is a folder is
        // shown as one row, `outer / inner`, until a branch appears.
        if options.compact {
            for child in &mut entries {
                if child.is_dir {
                    loop {
                        let inner = read_dir_sorted(&child.path, options);
                        match inner.as_slice() {
                            [only] if only.is_dir => {
                                child.names.push(only.names[0].clone());
                                child.path = only.path.clone();
                            }
                            _ => break,
                        }
                    }
                }
            }
        }
        self.children = Some(entries);
    }
}

fn read_dir_sorted(dir: &Path, options: &Options) -> Vec<Node> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<Node> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if options.excluded(&name) {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false) || (e.path().is_dir());
            Some(Node {
                path: e.path(),
                names: vec![name],
                is_dir,
                expanded: false,
                children: None,
            })
        })
        .collect();
    nodes.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.names[0].to_lowercase().cmp(&b.names[0].to_lowercase()))
    });
    nodes
}

/// One visible row, flattened from the tree for the list.
#[derive(Clone, Debug)]
struct Row {
    path: PathBuf,
    names: Vec<String>,
    is_dir: bool,
    expanded: bool,
    depth: usize,
    /// The index path into the tree, to find the node again.
    index: Vec<usize>,
    /// For the tui skin's guides: whether this is its folder's last entry,
    /// and for each ancestor level below the root whether a line runs on.
    last: bool,
    rails: Vec<bool>,
}

pub struct FileTree {
    root: Node,
    rows: Vec<Row>,
    /// The lead selection (the row the arrows move), and every selected row.
    selected: Option<usize>,
    selection: Vec<usize>,
    /// Where a Shift+click range starts.
    anchor: Option<usize>,
    /// `files.keyboardNavigation` highlight: the cursor row when it is not
    /// the selection.
    highlight: Option<usize>,
    options: Options,
    renaming: Option<Renaming>,
    /// Git status by path (M4): names coloured and badged as Fleet's.
    statuses: HashMap<PathBuf, Status>,
    focus_handle: FocusHandle,
    scroll: UniformListScrollHandle,
    /// The tui skin's grid as last laid out, columns by rows of cells.
    tui_grid: (usize, usize),
}

impl EventEmitter<FileTreeEvent> for FileTree {}

impl Focusable for FileTree {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn bind_keys(cx: &mut App) {
    use gpui::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("up", TreeUp, Some("FileTree")),
        KeyBinding::new("down", TreeDown, Some("FileTree")),
        KeyBinding::new("shift-up", TreeExtendUp, Some("FileTree")),
        KeyBinding::new("shift-down", TreeExtendDown, Some("FileTree")),
        KeyBinding::new("left", TreeLeft, Some("FileTree")),
        KeyBinding::new("right", TreeRight, Some("FileTree")),
        KeyBinding::new("enter", TreeOpen, Some("FileTree")),
        KeyBinding::new("space", TreeSelect, Some("FileTree")),
    ]);
}

impl FileTree {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let options = Options::from(&cx.global::<AppSettings>().settings, true);
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| root.display().to_string());
        let mut root_node = Node {
            path: root,
            names: vec![name],
            is_dir: true,
            expanded: true,
            children: None,
        };
        root_node.load(&options);
        let mut tree = FileTree {
            root: root_node,
            rows: Vec::new(),
            selected: Some(0),
            selection: vec![0],
            anchor: None,
            highlight: None,
            options,
            renaming: None,
            statuses: HashMap::new(),
            focus_handle: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            tui_grid: (0, 0),
        };
        tree.rebuild_rows();
        tree
    }

    /// The tree's scroll, which the tui skin's panel paints on its frame.
    pub fn tui_scroll(&self) -> gpui::ScrollHandle {
        self.scroll.0.borrow().base_handle.clone()
    }

    pub fn root_name(&self) -> String {
        self.root.name()
    }

    pub fn root_path(&self) -> &Path {
        &self.root.path
    }

    /// The selected row's path (the root's for the root row).
    pub fn selected_path(&self) -> Option<PathBuf> {
        self.selected
            .and_then(|ix| self.rows.get(ix))
            .map(|r| r.path.clone())
    }

    /// Every selected row's path, in row order.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        let mut ixs = self.selection.clone();
        ixs.sort_unstable();
        ixs.dedup();
        ixs.into_iter()
            .filter_map(|ix| self.rows.get(ix))
            .map(|r| r.path.clone())
            .collect()
    }

    /// The folder the selection stands in: the row itself when it is a folder,
    /// else its parent.
    pub fn selected_dir(&self) -> Option<PathBuf> {
        let row = self.selected.and_then(|ix| self.rows.get(ix))?;
        if row.is_dir {
            Some(row.path.clone())
        } else {
            row.path.parent().map(Path::to_path_buf)
        }
    }

    /// `toggle-hidden-files`: dotfiles in or out of the tree.
    pub fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        self.options.show_hidden = !self.options.show_hidden;
        self.reload(cx);
    }

    /// Re-reads the folders after the disk changed (a file created, renamed
    /// or deleted), keeping what was expanded and selected where it still exists.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let expanded: Vec<PathBuf> = self
            .rows
            .iter()
            .filter(|r| r.is_dir && r.expanded && !r.index.is_empty())
            .map(|r| r.path.clone())
            .collect();
        let selected = self.selected_path();
        self.root.children = None;
        self.root.load(&self.options);
        for path in expanded {
            self.expand_to(&path);
        }
        self.rebuild_rows();
        if let Some(path) = selected {
            if let Some(ix) = self.rows.iter().position(|r| r.path == path) {
                self.set_single(ix);
            }
        }
        cx.notify();
    }

    /// Expands the folders down to `path` (itself included when it is a folder).
    fn expand_to(&mut self, path: &Path) {
        let Ok(rel) = path.strip_prefix(&self.root.path) else {
            return;
        };
        let options = self.options.clone();
        let mut node = &mut self.root;
        let mut remaining: Vec<_> = rel.components().collect();
        while !remaining.is_empty() {
            node.load(&options);
            let Some(children) = node.children.as_mut() else {
                return;
            };
            let found = children.iter().position(|child| {
                child.names.len() <= remaining.len()
                    && child
                        .names
                        .iter()
                        .zip(remaining.iter())
                        .all(|(name, comp)| comp.as_os_str() == name.as_str())
            });
            let Some(i) = found else {
                return;
            };
            let n = children[i].names.len();
            remaining.drain(..n);
            node = &mut children[i];
            if node.is_dir {
                node.load(&options);
                node.expanded = true;
            }
        }
    }

    /// Re-reads the settings; a change in what is shown reloads the folder.
    fn refresh_options(&mut self, cx: &App) {
        let options = Options::from(
            &cx.global::<AppSettings>().settings,
            self.options.show_hidden,
        );
        if options == self.options {
            return;
        }
        let reload =
            options.compact != self.options.compact || options.exclude != self.options.exclude;
        self.options = options;
        if self.options.navigation == KeyboardNavigation::Select {
            self.highlight = None;
        }
        if reload {
            let selected = self.selected_path();
            self.root.children = None;
            self.root.load(&self.options);
            self.rebuild_rows();
            if let Some(path) = selected {
                self.select_path(&path);
            }
        }
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        rows.push(Row {
            path: self.root.path.clone(),
            names: self.root.names.clone(),
            is_dir: true,
            expanded: true,
            depth: 0,
            index: Vec::new(),
            last: true,
            rails: Vec::new(),
        });
        fn walk(
            node: &Node,
            depth: usize,
            index: &mut Vec<usize>,
            rails: &mut Vec<bool>,
            rows: &mut Vec<Row>,
        ) {
            let Some(children) = &node.children else {
                return;
            };
            for (i, child) in children.iter().enumerate() {
                let last = i + 1 == children.len();
                index.push(i);
                rows.push(Row {
                    path: child.path.clone(),
                    names: child.names.clone(),
                    is_dir: child.is_dir,
                    expanded: child.expanded,
                    depth,
                    index: index.clone(),
                    last,
                    rails: rails.clone(),
                });
                if child.is_dir && child.expanded {
                    rails.push(!last);
                    walk(child, depth + 1, index, rails, rows);
                    rails.pop();
                }
                index.pop();
            }
        }
        walk(&self.root, 1, &mut Vec::new(), &mut Vec::new(), &mut rows);
        self.rows = rows;
        let last = self.rows.len().saturating_sub(1);
        if let Some(s) = self.selected {
            if s > last {
                self.set_single(last);
            }
        }
        self.selection.retain(|ix| *ix <= last);
        if let Some(h) = self.highlight {
            if h > last {
                self.highlight = Some(last);
            }
        }
    }

    fn node_mut(&mut self, index: &[usize]) -> &mut Node {
        let mut node = &mut self.root;
        for &i in index {
            node = &mut node.children.as_mut().unwrap()[i];
        }
        node
    }

    // ---- selection --------------------------------------------------------------

    /// One selected row, the anchor and the highlight on it.
    fn set_single(&mut self, ix: usize) {
        self.selected = Some(ix);
        self.selection = vec![ix];
        self.anchor = Some(ix);
        self.highlight = Some(ix);
    }

    /// Ctrl+click: the row joins or leaves the selection.
    fn toggle_in_selection(&mut self, ix: usize) {
        if let Some(pos) = self.selection.iter().position(|s| *s == ix) {
            if self.selection.len() > 1 {
                self.selection.remove(pos);
                if self.selected == Some(ix) {
                    self.selected = self.selection.last().copied();
                }
            }
        } else {
            self.selection.push(ix);
            self.selected = Some(ix);
        }
        self.anchor = Some(ix);
        self.highlight = Some(ix);
    }

    /// Shift+click or Shift+arrow: everything from the anchor to `ix`.
    fn select_range_to(&mut self, ix: usize) {
        let anchor = self.anchor.or(self.selected).unwrap_or(ix);
        let (a, b) = if anchor <= ix {
            (anchor, ix)
        } else {
            (ix, anchor)
        };
        self.selection = (a..=b).collect();
        self.selected = Some(ix);
        self.anchor = Some(anchor);
        self.highlight = Some(ix);
    }

    fn is_selected(&self, ix: usize) -> bool {
        self.selection.contains(&ix)
    }

    fn toggle(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return;
        };
        if row.index.is_empty() {
            return;
        }
        let options = self.options.clone();
        let node = self.node_mut(&row.index);
        if node.is_dir {
            if !node.expanded {
                node.load(&options);
            }
            node.expanded = !node.expanded;
            self.rebuild_rows();
            cx.notify();
        }
    }

    fn set_expanded(&mut self, row_ix: usize, expanded: bool, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return;
        };
        if row.index.is_empty() || !row.is_dir {
            return;
        }
        let options = self.options.clone();
        let node = self.node_mut(&row.index);
        if node.expanded != expanded {
            if expanded {
                node.load(&options);
            }
            node.expanded = expanded;
            self.rebuild_rows();
            cx.notify();
        }
    }

    /// Enter: a folder toggles, a file opens in its own tab.
    fn activate(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        self.set_single(row_ix);
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return;
        };
        if row.is_dir {
            self.toggle(row_ix, cx);
        } else {
            cx.emit(FileTreeEvent::Open {
                path: row.path,
                preview: false,
            });
        }
        cx.notify();
    }

    /// A mouse click, counted and with its modifiers: Ctrl toggles the row in
    /// the selection, Shift extends it; otherwise what opens or toggles depends
    /// on `files.openFileMode` and `files.toggleFolderMode`.
    fn click(
        &mut self,
        row_ix: usize,
        count: usize,
        ctrl: bool,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        if ctrl {
            self.toggle_in_selection(row_ix);
            cx.notify();
            return;
        }
        if shift {
            self.select_range_to(row_ix);
            cx.notify();
            return;
        }
        self.set_single(row_ix);
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return;
        };
        if row.is_dir {
            let wanted = match self.options.toggle_mode {
                ClickMode::Single => 1,
                ClickMode::Double => 2,
            };
            if count == wanted {
                self.toggle(row_ix, cx);
            }
        } else {
            match (self.options.open_mode, count) {
                // A single click: a preview tab when the setting is on; the
                // second click of a double promotes it to a real tab.
                (ClickMode::Single, 1) => cx.emit(FileTreeEvent::Open {
                    path: row.path,
                    preview: self.options.preview,
                }),
                (ClickMode::Single, 2) | (ClickMode::Double, 2) => cx.emit(FileTreeEvent::Open {
                    path: row.path,
                    preview: false,
                }),
                _ => {}
            }
        }
        cx.notify();
    }

    /// A right click: the row joins the selection if it is not in it, then
    /// the workspace opens the menu for it.
    fn right_click(&mut self, row_ix: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.is_selected(row_ix) {
            self.set_single(row_ix);
        } else {
            self.selected = Some(row_ix);
        }
        if let Some(row) = self.rows.get(row_ix) {
            cx.emit(FileTreeEvent::ContextMenu {
                is_dir: row.is_dir,
                position,
            });
        }
        cx.notify();
    }

    /// `show-context-menu` from the keyboard: the lead row, at `position`.
    pub fn request_context_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(ix) = self.highlight.or(self.selected) {
            self.right_click(ix, position, cx);
        }
    }

    /// The arrows: in select mode they move the selection; in highlight mode
    /// only the cursor rectangle.
    fn arrow(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let ix = ix.min(self.rows.len() - 1);
        match self.options.navigation {
            KeyboardNavigation::Select => self.set_single(ix),
            KeyboardNavigation::Highlight => self.highlight = Some(ix),
        }
        self.scroll
            .scroll_to_item(ix, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }

    /// The row the keyboard stands on: the highlight, else the lead selection.
    fn cursor_row(&self) -> usize {
        self.highlight.or(self.selected).unwrap_or(0)
    }

    /// `files.selectOpenedFile`: the editor's file becomes the selected row,
    /// its folders expanded, scrolled into view. Returns whether it is under the root.
    pub fn select_path(&mut self, path: &Path) -> bool {
        if !self.options.select_opened {
            return false;
        }
        self.reveal_path(path)
    }

    /// Expands to the path and selects its row whatever `files.selectOpenedFile` says.
    pub fn reveal_path(&mut self, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(&self.root.path) else {
            return false;
        };
        let options = self.options.clone();
        // Expand along the way; a compact chain's row stands for its innermost folder.
        let mut node = &mut self.root;
        let mut remaining: Vec<_> = rel.components().collect();
        while !remaining.is_empty() {
            node.load(&options);
            let Some(children) = node.children.as_mut() else {
                return false;
            };
            let mut found = None;
            for (i, child) in children.iter().enumerate() {
                let n = child.names.len();
                if n <= remaining.len()
                    && child
                        .names
                        .iter()
                        .zip(remaining.iter())
                        .all(|(name, comp)| comp.as_os_str() == name.as_str())
                {
                    found = Some((i, n));
                    break;
                }
            }
            let Some((i, n)) = found else {
                return false;
            };
            remaining.drain(..n);
            node = &mut children[i];
            if node.is_dir && !remaining.is_empty() {
                node.load(&options);
                node.expanded = true;
            }
        }
        self.rebuild_rows();
        if let Some(ix) = self.rows.iter().position(|r| r.path == path) {
            self.set_single(ix);
            self.scroll
                .scroll_to_item(ix, gpui::ScrollStrategy::Nearest);
            true
        } else {
            false
        }
    }

    fn up(&mut self, _: &TreeUp, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.cursor_row().saturating_sub(1);
        self.arrow(ix, cx);
    }

    fn down(&mut self, _: &TreeDown, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.cursor_row() + 1;
        self.arrow(ix, cx);
    }

    fn extend_up(&mut self, _: &TreeExtendUp, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.cursor_row().saturating_sub(1);
        self.select_range_to(ix);
        self.scroll
            .scroll_to_item(ix, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }

    fn extend_down(&mut self, _: &TreeExtendDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let ix = (self.cursor_row() + 1).min(self.rows.len() - 1);
        self.select_range_to(ix);
        self.scroll
            .scroll_to_item(ix, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }

    fn left(&mut self, _: &TreeLeft, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.cursor_row();
        let Some(row) = self.rows.get(ix).cloned() else {
            return;
        };
        if row.is_dir && row.expanded && !row.index.is_empty() {
            self.set_expanded(ix, false, cx);
        } else if row.depth > 0 {
            // To the parent row: the nearest row above with a smaller depth.
            if let Some(parent) = (0..ix).rev().find(|&i| self.rows[i].depth < row.depth) {
                self.arrow(parent, cx);
            }
        }
    }

    fn right(&mut self, _: &TreeRight, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.cursor_row();
        let Some(row) = self.rows.get(ix).cloned() else {
            return;
        };
        if row.is_dir && !row.expanded {
            self.set_expanded(ix, true, cx);
        } else if row.is_dir && ix + 1 < self.rows.len() {
            self.arrow(ix + 1, cx);
        }
    }

    fn open(&mut self, _: &TreeOpen, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.cursor_row();
        self.activate(ix, cx);
    }

    /// Space: in highlight mode the highlighted row becomes the selection
    /// (Ctrl-like: it joins when others are selected); in select mode it
    /// opens, as Enter does.
    fn select_action(&mut self, _: &TreeSelect, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.cursor_row();
        match self.options.navigation {
            KeyboardNavigation::Highlight => {
                self.toggle_in_selection(ix);
                self.highlight = Some(ix);
                cx.notify();
            }
            KeyboardNavigation::Select => self.activate(ix, cx),
        }
    }

    /// The git statuses to colour by.
    pub fn set_statuses(&mut self, statuses: HashMap<PathBuf, Status>, cx: &mut Context<Self>) {
        if self.statuses != statuses {
            self.statuses = statuses;
            cx.notify();
        }
    }

    // ---- inline rename ------------------------------------------------------------

    /// `files/rename` on a row: the row becomes a text field holding the name
    /// with its stem selected. False when the path is not in this tree.
    pub fn start_rename(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.reveal_path(path) {
            return false;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let stem = if path.is_dir() {
            name.len()
        } else {
            name.rfind('.').filter(|&i| i > 0).unwrap_or(name.len())
        };
        let input = cx.new(|cx| {
            let mut input = TextInput::new("", cx);
            input.set_text(&name, cx);
            input.select_range(0..stem, cx);
            input
        });
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Submit { .. } => this.finish_rename(cx),
            InputEvent::Cancel => this.cancel_rename(cx),
            _ => {}
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.renaming = Some(Renaming {
            path: path.to_path_buf(),
            input,
            _subscription: subscription,
        });
        cx.notify();
        true
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            cx.notify();
        }
    }

    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        let name = renaming.input.read(cx).text().trim().to_string();
        let from = renaming.path;
        if name.is_empty() || name.contains(['/', '\\']) {
            cx.notify();
            return;
        }
        let to = from.with_file_name(&name);
        if to != from {
            match std::fs::rename(&from, &to) {
                Ok(()) => {
                    self.reload(cx);
                    self.reveal_path(&to);
                    self.rebuild_rows();
                    cx.emit(FileTreeEvent::Renamed { from, to });
                }
                Err(e) => eprintln!("weep: cannot rename {}: {e}", from.display()),
            }
        }
        cx.notify();
    }

    // ---- drag and drop ------------------------------------------------------------

    /// Moves the dragged paths into the folder at `row_ix` (a file row's
    /// parent folder); each move is reported so open tabs can follow.
    fn drop_onto(&mut self, row_ix: usize, paths: &[PathBuf], cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return;
        };
        let target = if row.is_dir {
            row.path.clone()
        } else {
            match row.path.parent() {
                Some(p) => p.to_path_buf(),
                None => return,
            }
        };
        let mut moved = false;
        for from in paths {
            let Some(name) = from.file_name() else {
                continue;
            };
            let to = target.join(name);
            if *from == to || to.starts_with(from) || to.exists() {
                continue;
            }
            match std::fs::rename(from, &to) {
                Ok(()) => {
                    moved = true;
                    cx.emit(FileTreeEvent::Moved {
                        from: from.clone(),
                        to,
                    });
                }
                Err(e) => eprintln!("weep: cannot move {}: {e}", from.display()),
            }
        }
        if moved {
            self.reload(cx);
            self.expand_to(&target);
            self.rebuild_rows();
            cx.notify();
        }
    }

    fn render_row(
        &self,
        ix: usize,
        focused: bool,
        theme: &Theme,
        m: &Metrics,
        entity: &gpui::Entity<FileTree>,
        tui: Option<(Cell, usize)>,
    ) -> gpui::Stateful<gpui::Div> {
        let row = &self.rows[ix];
        let selected = self.is_selected(ix);
        let highlighted = focused
            && self.options.navigation == KeyboardNavigation::Highlight
            && self.highlight == Some(ix);
        let is_root = row.index.is_empty();
        let indent = if is_root {
            m.tree_root_pad_left
        } else {
            m.tree_pad_left + (row.depth - 1) as f32 * m.tree_indent
        };
        let mut names = row.names.iter();
        let first: SharedString = names.next().cloned().unwrap_or_default().into();
        let rest: Vec<SharedString> = names.cloned().map(Into::into).collect();
        let status = self.statuses.get(&row.path).copied();
        // Fleet colours a file's name by its git status whatever the row's
        // state (eighth pass: added and untracked green, modified and renamed
        // blue; deleted and conflict are the diff colours, an estimate).
        let text_color = match status {
            Some(Status::Added) | Some(Status::Untracked) => theme.git_added_text,
            Some(Status::Modified) | Some(Status::Renamed) => theme.git_modified_text,
            Some(Status::Deleted) => theme.git_deleted_text,
            Some(Status::Conflict) => theme.git_conflict_text,
            _ if selected && focused => theme.list_text_focused,
            _ => theme.list_text,
        };
        let badge: Option<gpui::AnyElement> = status.and_then(|s| {
            let glyph = |text: &'static str, size: f32, color| {
                div()
                    .text_size(px(size))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(text)
                    .into_any_element()
            };
            let dot = |color| {
                div()
                    .size(px(m.tree_badge_dot))
                    .rounded_full()
                    .bg(color)
                    .into_any_element()
            };
            Some(match s {
                Status::Added => glyph("+", m.tree_badge_plus_font, theme.icon_green),
                Status::Untracked => glyph("?", m.tree_badge_font, theme.icon_green),
                Status::Modified | Status::Renamed => dot(theme.icon_blue),
                Status::Deleted => dot(theme.icon_red),
                Status::Conflict => dot(theme.icon_yellow),
                Status::Ignored => return None,
            })
        });
        let id: ElementId = ElementId::NamedInteger("tree-row".into(), ix as u64);
        let drag_paths = if selected {
            self.selected_paths()
        } else {
            vec![row.path.clone()]
        };
        let ghost_title: SharedString = if drag_paths.len() > 1 {
            format!("{} and {} more", first, drag_paths.len() - 1).into()
        } else {
            first.clone()
        };
        let ghost_icon = if row.is_dir {
            "expui/nodes/folder"
        } else {
            file_icon(&first)
        };
        let drag_bg = theme.drag_bg;
        let drag_border = theme.drag_border_highlight;
        let renaming = self
            .renaming
            .as_ref()
            .filter(|r| r.path == row.path)
            .map(|r| r.input.clone());
        let not_renaming = renaming.is_none();
        let body: gpui::AnyElement = match tui {
            Some((cell, cols)) => self.tui_row_body(
                ix,
                cell,
                cols,
                selected,
                focused,
                highlighted,
                text_color,
                status,
                renaming,
                theme,
                m,
            ),
            None => div()
                .h_full()
                .w_full()
                .flex()
                .items_center()
                .pl(px(indent))
                .pr(px(m.panel_pad_x))
                .rounded(px(m.tree_row_radius))
                .when(selected && focused, |d| d.bg(theme.list_bg_focused))
                .when(selected && !focused, |d| d.bg(theme.list_bg_selected))
                .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                .when(highlighted, |d| {
                    d.border(px(m.tree_highlight_border))
                        .border_color(theme.tree_focus_border)
                })
                // The renamed row: Fleet's focus ring around icon and field.
                .when(renaming.is_some(), |d| {
                    d.border(px(m.input_focus_border))
                        .border_color(theme.input_focus_border)
                })
                .when(!is_root, |d| {
                    d.child(
                        div()
                            .size(px(m.icon_size))
                            .flex_shrink_0()
                            .mr(px(m.tree_icon_gap))
                            .flex()
                            .items_center()
                            .justify_center()
                            .map(|d| {
                                if row.is_dir {
                                    let chevron = if row.expanded {
                                        "expui/general/chevronDown"
                                    } else {
                                        "expui/general/chevronRight"
                                    };
                                    d.child(icon(chevron, m.icon_size, theme.icon_secondary))
                                } else {
                                    d.child(icon(
                                        file_icon(&first),
                                        m.icon_size,
                                        theme.icon_secondary,
                                    ))
                                }
                            }),
                    )
                })
                .map(|d| match renaming {
                    Some(input) => d.child(div().flex_1().min_w_0().h_full().child(input)),
                    None => d.child(
                        div()
                            .flex()
                            .items_center()
                            .min_w_0()
                            // The root row's label runs to the island's edge and is clipped
                            // there, without an ellipsis (Windows probe).
                            .map(|d| {
                                if is_root {
                                    d.whitespace_nowrap()
                                } else {
                                    d.truncate()
                                }
                            })
                            .text_color(text_color)
                            .child(first)
                            .children(rest.into_iter().flat_map(|name| {
                                [
                                    div()
                                        .px(px(5. * m.zoom))
                                        .text_color(theme.text_tertiary)
                                        .child("/"),
                                    div().child(name),
                                ]
                            })),
                    ),
                })
                .children(badge.map(|b| {
                    div()
                        .ml_auto()
                        .size(px(m.tree_badge_box))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(b)
                }))
                .into_any_element(),
        };
        // The row keeps the 26 px pitch; the background inside it is 24 tall
        // (Windows probe), so the hover and selection washes sit 1 px in.
        div()
            .id(id)
            .h(px(tui.map_or(m.tree_row_height, |(cell, _)| cell.h)))
            .when(tui.is_none(), |d| d.py(px(m.tree_row_inset)))
            .w_full()
            .cursor_pointer()
            .when(not_renaming, |d| {
                d.on_mouse_down(MouseButton::Left, {
                    let entity = entity.clone();
                    move |event, window, cx| {
                        let count = event.click_count;
                        let ctrl = event.modifiers.control || event.modifiers.platform;
                        let shift = event.modifiers.shift;
                        entity.update(cx, |this, cx| {
                            this.cancel_rename(cx);
                            this.focus_handle.focus(window, cx);
                            this.click(ix, count, ctrl, shift, cx);
                        });
                    }
                })
            })
            .on_mouse_down(MouseButton::Right, {
                let entity = entity.clone();
                move |event, window, cx| {
                    let position = event.position;
                    entity.update(cx, |this, cx| {
                        this.focus_handle.focus(window, cx);
                        this.right_click(ix, position, cx);
                    });
                }
            })
            .when(!is_root, |d| {
                d.on_drag(DraggedPaths { paths: drag_paths }, move |_, _, _, cx| {
                    cx.new(|_| DragGhost {
                        title: ghost_title.clone(),
                        icon: ghost_icon,
                    })
                })
            })
            .drag_over::<DraggedPaths>(move |style, _, _, _| {
                // The tui skin's drop target is its row filled, no outline.
                if tui.is_some() {
                    style.bg(drag_bg)
                } else {
                    style.bg(drag_bg).border_1().border_color(drag_border)
                }
            })
            .on_drop({
                let entity = entity.clone();
                move |dragged: &DraggedPaths, _, cx| {
                    let paths = dragged.paths.clone();
                    entity.update(cx, |this, cx| this.drop_onto(ix, &paths, cx));
                }
            })
            .child(body)
    }

    /// A row in the tui skin, one cell high and `cols` cells wide: the guides,
    /// a mark cell (the folder's chevron or the file's glyph), a gap, the
    /// name, and the git status as a letter in the last column. The washes
    /// are square and run the full row, like a terminal's reverse video.
    #[allow(clippy::too_many_arguments)]
    fn tui_row_body(
        &self,
        ix: usize,
        cell: Cell,
        cols: usize,
        selected: bool,
        focused: bool,
        highlighted: bool,
        text_color: gpui::Hsla,
        status: Option<Status>,
        renaming: Option<Entity<TextInput>>,
        theme: &Theme,
        m: &Metrics,
    ) -> gpui::AnyElement {
        let row = &self.rows[ix];
        let is_root = row.index.is_empty();
        let mark = |text: &'static str, color| {
            div()
                .w(px(cell.w))
                .flex_shrink_0()
                .text_color(color)
                .child(text)
        };
        let (glyph, glyph_color) = if row.is_dir {
            (if row.expanded { "▾" } else { "▸" }, theme.icon_secondary)
        } else {
            skin::file_glyph(&row.names[0], theme)
        };
        let letter = status.and_then(|s| {
            Some(match s {
                Status::Added => ("A", theme.icon_green),
                Status::Untracked => ("?", theme.icon_green),
                Status::Modified => ("M", theme.icon_blue),
                Status::Renamed => ("R", theme.icon_blue),
                Status::Deleted => ("D", theme.icon_red),
                Status::Conflict => ("U", theme.icon_yellow),
                Status::Ignored => return None,
            })
        });
        let name: SharedString = row.names.join(" / ").into();
        div()
            .h(px(cell.h))
            .w(px(cols as f32 * cell.w))
            .flex()
            .items_center()
            .when(selected && focused, |d| d.bg(theme.list_bg_focused))
            .when(selected && !focused, |d| d.bg(theme.list_bg_selected))
            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
            // The keyboard's highlighted row: filled as hovered, not outlined.
            .when(highlighted && !selected, |d| d.bg(theme.list_bg_hovered))
            .when(!is_root, |d| {
                d.child(skin::guides(
                    cell,
                    row.rails.clone(),
                    row.last,
                    theme.text_tertiary,
                    *m,
                ))
            })
            .child(mark(glyph, glyph_color))
            .child(div().w(px(cell.w)).flex_shrink_0())
            .map(|d| match renaming {
                Some(input) => d.child(div().flex_1().min_w_0().h_full().child(input)),
                None => d.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(text_color)
                        .when(is_root, |d| d.font_weight(gpui::FontWeight::BOLD))
                        .child(name),
                ),
            })
            .children(letter.map(|(text, color)| mark(text, color)))
            .into_any_element()
    }

    /// The tree in the tui skin: rows of whole cells filling the tool's
    /// area, which the panel sizes in whole cells inside its frame.
    fn render_tui(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let focused = self.focus_handle.is_focused(window);
        let count = self.rows.len();
        let entity = cx.entity();
        let inner_cols = self.tui_grid.0;
        let measure = {
            let entity = entity.clone();
            gpui::canvas(
                move |bounds, window, cx| {
                    let grid = (
                        Cell::fit(bounds.size.width, cell.w),
                        Cell::fit(bounds.size.height, cell.h),
                    );
                    let changed = entity.update(cx, |this, _| {
                        std::mem::replace(&mut this.tui_grid, grid) != grid
                    });
                    if changed {
                        skin::redraw(window);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        };
        let theme_for_rows = theme.clone();
        div()
            .size_full()
            .relative()
            .font_family(theme.mono_font.clone())
            .text_size(px(cell.font_size))
            .line_height(px(cell.h))
            .child(measure)
            .child(
                div().size_full().child(
                    uniform_list("file-tree", count, move |range, _window, cx| {
                        let this = entity.read(cx);
                        range
                            .map(|ix| {
                                this.render_row(
                                    ix,
                                    focused,
                                    &theme_for_rows,
                                    &m,
                                    &entity,
                                    Some((cell, inner_cols)),
                                )
                            })
                            .collect()
                    })
                    .track_scroll(&self.scroll)
                    .size_full(),
                ),
                // The scroll bar is the panel frame's right edge (workspace).
            )
    }
}

impl Render for FileTree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_options(cx);
        let tui = Skin::current(cx) == Skin::Tui;
        let content = if tui {
            Some(self.render_tui(window, cx))
        } else {
            None
        };
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let focused = self.focus_handle.is_focused(window);
        let count = self.rows.len();
        let entity = cx.entity();
        let theme_for_rows = theme.clone();
        let base = self.scroll.0.borrow().base_handle.clone();
        div()
            .key_context("FileTree")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::extend_up))
            .on_action(cx.listener(Self::extend_down))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::select_action))
            .size_full()
            .relative()
            .map(|d| match content {
                Some(tui) => d.child(tui),
                None => d
                    .child(
                        div().size_full().px(px(m.panel_pad_x)).child(
                            uniform_list("file-tree", count, move |range, _window, cx| {
                                let this = entity.read(cx);
                                range
                                    .map(|ix| {
                                        this.render_row(
                                            ix,
                                            focused,
                                            &theme_for_rows,
                                            &m,
                                            &entity,
                                            None,
                                        )
                                    })
                                    .collect()
                            })
                            .track_scroll(&self.scroll)
                            .size_full(),
                        ),
                    )
                    .child(scrollbar(
                        "file-tree-scrollbar",
                        &base,
                        m.scrollbar_thumb,
                        &theme,
                        &m,
                    )),
            })
    }
}
