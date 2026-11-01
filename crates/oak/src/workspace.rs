//! The window: Fleet's shell. The toolbar across the top, then the islands
//! (the Files panel on the left, the editor with its tab row in the middle,
//! a bottom panel and a right panel when open), then the status bar.

use crate::actions::*;
use crate::assets::{file_icon, icon};
use crate::editor::{EditorEvent, EditorView};
use crate::file_tree::{FileTree, FileTreeEvent};
use crate::theme::{metrics::*, Theme};
use crate::toolbar;
use gpui::{
    div, prelude::*, px, App, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    SharedString, Window,
};
use std::path::{Path, PathBuf};

pub struct Workspace {
    root: PathBuf,
    file_tree: Entity<FileTree>,
    editors: Vec<Entity<EditorView>>,
    active: Option<usize>,
    left_open: bool,
    bottom_open: bool,
    right_open: bool,
    focus_handle: FocusHandle,
    last_title: String,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Workspace {
    pub fn new(
        root: PathBuf,
        open: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let file_tree = cx.new(|cx| FileTree::new(root.clone(), cx));
        cx.subscribe(
            &file_tree,
            |this, _tree, event: &FileTreeEvent, cx| match event {
                FileTreeEvent::Open(path) => this.open_path(path.clone(), cx),
            },
        )
        .detach();
        let mut this = Workspace {
            root,
            file_tree,
            editors: Vec::new(),
            active: None,
            left_open: true,
            bottom_open: false,
            right_open: false,
            focus_handle: cx.focus_handle(),
            last_title: String::new(),
        };
        for path in open {
            this.open_path(path, cx);
        }
        if this.active.is_some() {
            this.focus_active_editor(window, cx);
        } else {
            let tree = this.file_tree.clone();
            tree.read(cx).focus_handle(cx).focus(window, cx);
        }
        this
    }

    fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(ix) = self
            .editors
            .iter()
            .position(|e| e.read(cx).path.as_deref() == Some(&path))
        {
            self.active = Some(ix);
            cx.notify();
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
        let editor = cx.new(|cx| EditorView::new(Some(path), &text, cx));
        cx.subscribe(&editor, |_this, _editor, _event: &EditorEvent, cx| {
            cx.notify()
        })
        .detach();
        self.editors.push(editor);
        self.active = Some(self.editors.len() - 1);
        cx.notify();
    }

    fn active_editor(&self) -> Option<&Entity<EditorView>> {
        self.active.and_then(|ix| self.editors.get(ix))
    }

    fn focus_active_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            let handle = editor.read(cx).focus_handle(cx);
            handle.focus(window, cx);
        }
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.editors.len() {
            return;
        }
        self.editors.remove(ix);
        self.active = if self.editors.is_empty() {
            None
        } else {
            Some(
                self.active
                    .map(|a| {
                        if a > ix {
                            a - 1
                        } else {
                            a.min(self.editors.len() - 1)
                        }
                    })
                    .unwrap_or(0),
            )
        };
        if self.active.is_some() {
            self.focus_active_editor(window, cx);
        } else {
            self.file_tree.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    // ---- actions --------------------------------------------------------

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            if let Err(e) = editor.update(cx, |e, cx| e.save(cx)) {
                eprintln!("oak: save failed: {e}");
            }
        }
    }

    fn save_all(&mut self, _: &SaveAll, _: &mut Window, cx: &mut Context<Self>) {
        for editor in self.editors.clone() {
            if let Err(e) = editor.update(cx, |e, cx| e.save(cx)) {
                eprintln!("oak: save failed: {e}");
            }
        }
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.close_tab_at(ix, window, cx);
        }
    }

    fn close_window(&mut self, _: &CloseWindow, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.active = Some((ix + 1) % self.editors.len());
            self.focus_active_editor(window, cx);
            cx.notify();
        }
    }

    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.active = Some((ix + self.editors.len() - 1) % self.editors.len());
            self.focus_active_editor(window, cx);
            cx.notify();
        }
    }

    fn toggle_panel(&mut self, which: usize, cx: &mut Context<Self>) {
        match which {
            0 => self.left_open = !self.left_open,
            1 => self.bottom_open = !self.bottom_open,
            _ => self.right_open = !self.right_open,
        }
        cx.notify();
    }

    fn toggle_left(&mut self, _: &ToggleLeftPanel, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(0, cx);
    }
    fn toggle_bottom(&mut self, _: &ToggleBottomPanel, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(1, cx);
    }
    fn toggle_right(&mut self, _: &ToggleRightPanel, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(2, cx);
    }
    fn hide_all(&mut self, _: &HideAllPanels, _: &mut Window, cx: &mut Context<Self>) {
        self.left_open = false;
        self.bottom_open = false;
        self.right_open = false;
        cx.notify();
    }
    fn focus_editor(&mut self, _: &FocusEditor, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_active_editor(window, cx);
    }
    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.new(|cx| EditorView::new(None, "", cx));
        cx.subscribe(&editor, |_this, _editor, _event: &EditorEvent, cx| {
            cx.notify()
        })
        .detach();
        self.editors.push(editor);
        self.active = Some(self.editors.len() - 1);
        self.focus_active_editor(window, cx);
        cx.notify();
    }
    fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    // ---- rendering ------------------------------------------------------

    fn island(&self, theme: &Theme) -> gpui::Div {
        div()
            .bg(theme.island_background)
            .rounded(px(ISLAND_RADIUS))
            .overflow_hidden()
    }

    fn render_tabs(&self, theme: &Theme, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let tabs = self.editors.iter().enumerate().map(|(ix, editor)| {
            let e = editor.read(cx);
            let selected = self.active == Some(ix);
            let focused = selected && e.focus_handle(cx).is_focused(window);
            let title: SharedString = e.title().into();
            let dirty = e.is_dirty();
            let icon_path = file_icon(&e.title());
            let activate = entity.clone();
            let close = entity.clone();
            div()
                .id(ElementId::NamedInteger("tab".into(), ix as u64))
                .h(px(EDITOR_TAB_HEIGHT))
                .px(px(EDITOR_TAB_PAD_X))
                .flex()
                .items_center()
                .gap(px(EDITOR_TAB_ICON_GAP))
                .rounded(px(EDITOR_TAB_RADIUS))
                .border_1()
                .cursor_pointer()
                .map(|d| {
                    if focused {
                        d.bg(theme.tab_bg_selected_focused)
                            .border_color(theme.tab_border_selected_focused)
                            .text_color(theme.tab_text_selected)
                    } else if selected {
                        d.bg(theme.tab_bg_selected)
                            .border_color(theme.tab_border_selected)
                            .text_color(theme.tab_text_selected)
                    } else {
                        d.border_color(gpui::transparent_black())
                            .text_color(theme.tab_text)
                            .hover(|s| s.bg(theme.tab_bg_hovered))
                    }
                })
                .on_click(move |_, window, cx| {
                    activate.update(cx, |this, cx| {
                        this.active = Some(ix);
                        this.focus_active_editor(window, cx);
                        cx.notify();
                    })
                })
                .child(icon(icon_path, ICON_SIZE, theme.icon_secondary))
                .child(div().child(title))
                .child(
                    div()
                        .id(ElementId::NamedInteger("tab-close".into(), ix as u64))
                        .size(px(ICON_SIZE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .text_color(theme.tab_close_icon)
                        .hover(|s| {
                            s.text_color(theme.tab_close_icon_hovered)
                                .bg(theme.ghost_hovered)
                        })
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            close.update(cx, |this, cx| this.close_tab_at(ix, window, cx));
                        })
                        .map(|d| {
                            if dirty {
                                d.child(div().size(px(6.)).rounded_full().bg(theme.text_secondary))
                            } else {
                                d.child(
                                    icon(
                                        "expui/general/closeSmall",
                                        ICON_SIZE,
                                        gpui::transparent_black(),
                                    )
                                    .text_color(theme.tab_close_icon),
                                )
                            }
                        }),
                )
        });
        div()
            .h(px(EDITOR_TAB_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px(px(PANEL_PAD_X))
            .gap(px(EDITOR_TAB_GAP))
            .overflow_hidden()
            .children(tabs)
    }

    fn render_editor_island(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let body = match self.active_editor() {
            Some(editor) => div().flex_1().min_h_0().child(editor.clone()),
            None => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
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
        self.island(theme)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_tabs(theme, window, cx))
            .child(body)
    }

    fn render_status_bar(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement {
        let root_name: SharedString = self.file_tree.read(cx).root_name().into();
        let mut crumbs: Vec<SharedString> = vec![root_name];
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
            let p = e.cursor_position();
            right.push(format!("{}:{}", p.row + 1, p.column + 1).into());
            right.push(if e.text.buffer.uses_crlf() {
                "CRLF".into()
            } else {
                "LF".into()
            });
            right.push("UTF-8".into());
            right.push(
                e.language
                    .map(|l| l.display_name())
                    .unwrap_or("Plain text")
                    .into(),
            );
        }
        let sep = |theme: &Theme| div().px(px(7.)).text_color(theme.text_tertiary).child("/");
        div()
            .h(px(STATUS_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px(px(ISLAND_MARGIN + STATUS_PAD_X))
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
                    .gap(px(STATUS_GAP))
                    .children(right.into_iter().map(|t| div().child(t))),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
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
                left_open: self.left_open,
                bottom_open: self.bottom_open,
                right_open: self.right_open,
            },
            &theme,
            move |which, _window, cx| entity.update(cx, |this, cx| this.toggle_panel(which, cx)),
        );
        let left = self.left_open.then(|| {
            self.island(&theme)
                .w(px(LEFT_PANEL_WIDTH))
                .flex_shrink_0()
                .child(self.file_tree.clone())
        });
        let right = self.right_open.then(|| {
            self.island(&theme)
                .w(px(LEFT_PANEL_WIDTH))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_secondary)
                .child("Right panel")
        });
        let bottom = self.bottom_open.then(|| {
            self.island(&theme)
                .h(px(BOTTOM_PANEL_HEIGHT))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_secondary)
                .child("Terminal")
        });
        div()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_all))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::toggle_left))
            .on_action(cx.listener(Self::toggle_bottom))
            .on_action(cx.listener(Self::toggle_right))
            .on_action(cx.listener(Self::hide_all))
            .on_action(cx.listener(Self::focus_editor))
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::quit))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background_primary)
            .font_family(theme.ui_font.clone())
            .text_size(px(UI_FONT_SIZE))
            .text_color(theme.text_primary)
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .px(px(ISLAND_MARGIN))
                    .gap(px(ISLAND_MARGIN))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .gap(px(ISLAND_MARGIN))
                            .children(left)
                            .child(self.render_editor_island(&theme, window, cx))
                            .children(right),
                    )
                    .children(bottom),
            )
            .child(self.render_status_bar(&theme, cx))
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
