//! The Files panel: the workspace folder as a tree, laid out the way Fleet
//! draws it. A root row carrying the folder's name, then one 26 px row per
//! entry with a chevron for a folder and a file-type icon for a file;
//! folders holding nothing but one folder collapse into a `a / b` chain
//! (`files.compactDirectories`). A click opens a file (`files.openFileMode`:
//! single or double; a single click opens a preview tab when
//! `editor.previewTab` is on), the file opened in the editor is selected here
//! (`files.selectOpenedFile`), and `files.exclude` names what stays hidden.

use crate::actions::{TreeDown, TreeLeft, TreeOpen, TreeRight, TreeUp};
use crate::assets::{file_icon, icon};
use crate::scrollbar::scrollbar;
use crate::settings::AppSettings;
use crate::theme::{Metrics, Theme};
use gpui::{
    div, prelude::*, px, uniform_list, App, Context, ElementId, EventEmitter, FocusHandle,
    Focusable, MouseButton, SharedString, UniformListScrollHandle, Window,
};
use oak_settings::ClickMode;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum FileTreeEvent {
    /// Show this file: in a preview tab when `preview` (a single click with
    /// `editor.previewTab`), else in a tab of its own.
    Open { path: PathBuf, preview: bool },
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
}

impl Options {
    fn from(settings: &oak_settings::Settings) -> Options {
        Options {
            compact: settings.files_compact_directories(),
            exclude: settings.files_exclude(),
            open_mode: settings.files_open_file_mode(),
            toggle_mode: settings.files_toggle_folder_mode(),
            preview: settings.editor_preview_tab(),
            select_opened: settings.files_select_opened_file(),
        }
    }

    /// `files.exclude` patterns against a name: a literal name, `*.ext`, or a
    /// `**/name` prefix, which is what Fleet's defaults use.
    fn excluded(&self, name: &str) -> bool {
        self.exclude.iter().any(|pat| {
            let pat = pat.strip_prefix("**/").unwrap_or(pat);
            if let Some(suffix) = pat.strip_prefix('*') {
                name.ends_with(suffix)
            } else {
                name == pat
            }
        })
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
}

pub struct FileTree {
    root: Node,
    rows: Vec<Row>,
    selected: Option<usize>,
    options: Options,
    focus_handle: FocusHandle,
    scroll: UniformListScrollHandle,
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
        KeyBinding::new("left", TreeLeft, Some("FileTree")),
        KeyBinding::new("right", TreeRight, Some("FileTree")),
        KeyBinding::new("enter", TreeOpen, Some("FileTree")),
        KeyBinding::new("space", TreeOpen, Some("FileTree")),
    ]);
}

impl FileTree {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let options = Options::from(&cx.global::<AppSettings>().settings);
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
            options,
            focus_handle: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        };
        tree.rebuild_rows();
        tree
    }

    pub fn root_name(&self) -> String {
        self.root.name()
    }

    /// Re-reads the settings; a change in what is shown reloads the folder.
    fn refresh_options(&mut self, cx: &App) {
        let options = Options::from(&cx.global::<AppSettings>().settings);
        if options == self.options {
            return;
        }
        let reload =
            options.compact != self.options.compact || options.exclude != self.options.exclude;
        self.options = options;
        if reload {
            let selected = self
                .selected
                .and_then(|ix| self.rows.get(ix))
                .map(|r| r.path.clone());
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
        });
        fn walk(node: &Node, depth: usize, index: &mut Vec<usize>, rows: &mut Vec<Row>) {
            let Some(children) = &node.children else {
                return;
            };
            for (i, child) in children.iter().enumerate() {
                index.push(i);
                rows.push(Row {
                    path: child.path.clone(),
                    names: child.names.clone(),
                    is_dir: child.is_dir,
                    expanded: child.expanded,
                    depth,
                    index: index.clone(),
                });
                if child.is_dir && child.expanded {
                    walk(child, depth + 1, index, rows);
                }
                index.pop();
            }
        }
        walk(&self.root, 1, &mut Vec::new(), &mut rows);
        self.rows = rows;
        if let Some(s) = self.selected {
            if s >= self.rows.len() {
                self.selected = Some(self.rows.len().saturating_sub(1));
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

    /// Enter or Space: a folder toggles, a file opens in its own tab.
    fn activate(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(row_ix);
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

    /// A mouse click, counted: what opens or toggles depends on
    /// `files.openFileMode` and `files.toggleFolderMode`.
    fn click(&mut self, row_ix: usize, count: usize, cx: &mut Context<Self>) {
        self.selected = Some(row_ix);
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

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let ix = ix.min(self.rows.len() - 1);
        self.selected = Some(ix);
        self.scroll
            .scroll_to_item(ix, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }

    /// `files.selectOpenedFile`: the editor's file becomes the selected row,
    /// its folders expanded, scrolled into view. Returns whether it is under the root.
    pub fn select_path(&mut self, path: &Path) -> bool {
        if !self.options.select_opened {
            return false;
        }
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
            self.selected = Some(ix);
            self.scroll
                .scroll_to_item(ix, gpui::ScrollStrategy::Nearest);
            true
        } else {
            false
        }
    }

    fn up(&mut self, _: &TreeUp, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.selected.unwrap_or(0).saturating_sub(1);
        self.select(ix, cx);
    }

    fn down(&mut self, _: &TreeDown, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.selected.map(|s| s + 1).unwrap_or(0);
        self.select(ix, cx);
    }

    fn left(&mut self, _: &TreeLeft, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let row = self.rows[ix].clone();
        if row.is_dir && row.expanded && !row.index.is_empty() {
            self.set_expanded(ix, false, cx);
        } else if row.depth > 0 {
            // To the parent row: the nearest row above with a smaller depth.
            if let Some(parent) = (0..ix).rev().find(|&i| self.rows[i].depth < row.depth) {
                self.select(parent, cx);
            }
        }
    }

    fn right(&mut self, _: &TreeRight, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let row = self.rows[ix].clone();
        if row.is_dir && !row.expanded {
            self.set_expanded(ix, true, cx);
        } else if row.is_dir && ix + 1 < self.rows.len() {
            self.select(ix + 1, cx);
        }
    }

    fn open(&mut self, _: &TreeOpen, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected {
            self.activate(ix, cx);
        }
    }

    fn render_row(
        &self,
        ix: usize,
        focused: bool,
        theme: &Theme,
        m: &Metrics,
        entity: &gpui::Entity<FileTree>,
    ) -> gpui::Stateful<gpui::Div> {
        let row = &self.rows[ix];
        let selected = self.selected == Some(ix);
        let is_root = row.index.is_empty();
        let indent = if is_root {
            m.tree_root_pad_left
        } else {
            m.tree_pad_left + (row.depth - 1) as f32 * m.tree_indent
        };
        let mut names = row.names.iter();
        let first: SharedString = names.next().cloned().unwrap_or_default().into();
        let rest: Vec<SharedString> = names.cloned().map(Into::into).collect();
        let text_color = if selected && focused {
            theme.list_text_focused
        } else {
            theme.list_text
        };
        let id: ElementId = ElementId::NamedInteger("tree-row".into(), ix as u64);
        // The row keeps the 26 px pitch; the background inside it is 24 tall
        // (Windows probe), so the hover and selection washes sit 1 px in.
        div()
            .id(id)
            .h(px(m.tree_row_height))
            .py(px(m.tree_row_inset))
            .w_full()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, {
                let entity = entity.clone();
                move |event, window, cx| {
                    let count = event.click_count;
                    entity.update(cx, |this, cx| {
                        this.focus_handle.focus(window, cx);
                        this.click(ix, count, cx);
                    });
                }
            })
            .child(
                div()
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
                    .child(
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
            )
    }
}

impl Render for FileTree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_options(cx);
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
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::open))
            .size_full()
            .relative()
            .child(
                div().size_full().px(px(m.panel_pad_x)).child(
                    uniform_list("file-tree", count, move |range, _window, cx| {
                        let this = entity.read(cx);
                        range
                            .map(|ix| this.render_row(ix, focused, &theme_for_rows, &m, &entity))
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
            ))
    }
}
