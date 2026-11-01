//! The Files panel: the workspace folder as a tree, laid out the way Fleet
//! draws it. A root row carrying the folder's name, then one 26 px row per
//! entry with a chevron for a folder and a file-type icon for a file;
//! folders holding nothing but one folder collapse into a `a / b` chain.

use crate::actions::{TreeDown, TreeLeft, TreeOpen, TreeRight, TreeUp};
use crate::assets::{file_icon, icon};
use crate::theme::{metrics::*, Theme};
use gpui::{
    div, prelude::*, px, uniform_list, App, Context, ElementId, EventEmitter, FocusHandle,
    Focusable, MouseButton, SharedString, UniformListScrollHandle, Window,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum FileTreeEvent {
    Open(PathBuf),
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

impl Node {
    fn name(&self) -> String {
        self.names.join(" / ")
    }

    fn load(&mut self) {
        if self.children.is_some() || !self.is_dir {
            return;
        }
        let mut entries = read_dir_sorted(&self.path);
        // Fleet's compact folders: a folder whose only entry is a folder is
        // shown as one row, `outer / inner`, until a branch appears.
        for child in &mut entries {
            if child.is_dir {
                loop {
                    let inner = read_dir_sorted(&child.path);
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
        self.children = Some(entries);
    }
}

fn read_dir_sorted(dir: &Path) -> Vec<Node> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<Node> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".git" {
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
        root_node.load();
        let mut tree = FileTree {
            root: root_node,
            rows: Vec::new(),
            selected: Some(0),
            focus_handle: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        };
        tree.rebuild_rows();
        tree
    }

    pub fn root_name(&self) -> String {
        self.root.name()
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
        let node = self.node_mut(&row.index);
        if node.is_dir {
            if !node.expanded {
                node.load();
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
        let node = self.node_mut(&row.index);
        if node.expanded != expanded {
            if expanded {
                node.load();
            }
            node.expanded = expanded;
            self.rebuild_rows();
            cx.notify();
        }
    }

    fn activate(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(row_ix);
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return;
        };
        if row.is_dir {
            self.toggle(row_ix, cx);
        } else {
            cx.emit(FileTreeEvent::Open(row.path));
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
        entity: &gpui::Entity<FileTree>,
    ) -> gpui::Stateful<gpui::Div> {
        let row = &self.rows[ix];
        let selected = self.selected == Some(ix);
        let is_root = row.index.is_empty();
        let indent = if is_root {
            TREE_ROOT_PAD_LEFT
        } else {
            TREE_PAD_LEFT + (row.depth - 1) as f32 * TREE_INDENT
        };
        let mut names = row.names.iter();
        let first: SharedString = names.next().cloned().unwrap_or_default().into();
        let rest: Vec<SharedString> = names.cloned().map(Into::into).collect();
        let text_color = theme.list_text;
        let id: ElementId = ElementId::NamedInteger("tree-row".into(), ix as u64);
        div()
            .id(id)
            .h(px(TREE_ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .pl(px(indent))
            .pr(px(6.))
            .rounded(px(TREE_ROW_RADIUS))
            .cursor_pointer()
            .when(selected && focused, |d| d.bg(theme.list_bg_focused))
            .when(selected && !focused, |d| d.bg(theme.list_bg_selected))
            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
            .on_mouse_down(MouseButton::Left, {
                let entity = entity.clone();
                move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.focus_handle.focus(window, cx);
                        this.activate(ix, cx);
                    });
                }
            })
            .when(!is_root, |d| {
                d.child(
                    div()
                        .size(px(ICON_SIZE))
                        .flex_shrink_0()
                        .mr(px(TREE_ICON_GAP))
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
                                d.child(icon(chevron, ICON_SIZE, theme.icon_secondary))
                            } else {
                                d.child(icon(file_icon(&first), ICON_SIZE, theme.icon_secondary))
                            }
                        }),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .min_w_0()
                    .truncate()
                    .text_color(text_color)
                    .child(first)
                    .children(rest.into_iter().flat_map(|name| {
                        [
                            div().px(px(5.)).text_color(theme.text_tertiary).child("/"),
                            div().child(name),
                        ]
                    })),
            )
    }
}

/// The panel's own tab row: Files, Search, Git, History and a "+".
fn panel_tabs(theme: &Theme) -> impl IntoElement {
    let tab = |label: &'static str, selected: bool| {
        div()
            .h(px(PANEL_TAB_HEIGHT))
            .px(px(PANEL_TAB_PAD_X))
            .flex()
            .items_center()
            .rounded(px(PANEL_TAB_RADIUS))
            .when(selected, |d| {
                d.bg(theme.tab_bg_selected)
                    .border_1()
                    .border_color(theme.tab_border_selected)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.tab_text_selected)
            })
            .when(!selected, |d| d.text_color(theme.tab_text))
            .child(label)
    };
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(PANEL_TAB_GAP))
        .pt(px(PANEL_HEADER_PAD_TOP))
        .pb(px(PANEL_HEADER_PAD_BOTTOM))
        .px(px(PANEL_PAD_X))
        .child(tab("Files", true))
        .child(tab("Search", false))
        .child(tab("Git", false))
        .child(tab("History", false))
        .child(
            div()
                .size(px(PANEL_TAB_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("expui/general/add", ICON_SIZE, theme.icon_secondary)),
        )
}

impl Render for FileTree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let focused = self.focus_handle.is_focused(window);
        let count = self.rows.len();
        let entity = cx.entity();
        let theme_for_rows = theme.clone();
        div()
            .key_context("FileTree")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::open))
            .size_full()
            .flex()
            .flex_col()
            .child(panel_tabs(&theme))
            .child(
                div().flex_1().min_h_0().px(px(PANEL_PAD_X)).child(
                    uniform_list("file-tree", count, move |range, _window, cx| {
                        let this = entity.read(cx);
                        range
                            .map(|ix| this.render_row(ix, focused, &theme_for_rows, &entity))
                            .collect()
                    })
                    .track_scroll(&self.scroll)
                    .size_full(),
                ),
            )
    }
}
