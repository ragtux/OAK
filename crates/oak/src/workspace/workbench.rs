//! The M3 workbench on the workspace: the Goto panel (with Fleet's Text
//! Search tab), the Search tool, the Files tool's menu and multi-select
//! actions, context menus and the main menu, confirm dialogs, notifications,
//! local-history snapshots, recent workspaces and the quit confirmation. A
//! child module so it reaches the workspace's private state; `workspace.rs`
//! keeps the shell (panels, splits, tabs, the status bar) and M2's editor
//! actions.

use super::*;
use crate::actions::{OpenInSplit, SplitRight};
use crate::dialog::{ConfirmDialog, DialogButton, DialogEvent};
use crate::editor::EditorView;
use crate::goto::{GotoEvent, GotoMode, GotoPanel, GotoSources, GotoTarget};
use crate::keymap::{action_for, hint_for, is_editor_action};
use crate::menu::{ContextMenu, MenuEntry, MenuEvent};
use crate::notifications::{Level, Notifications};
use crate::search_tool::{SearchTool, SearchToolEvent};
use crate::settings_view::SettingsView;
use gpui::{deferred, Action, BoxShadow, FontWeight};
use gpui::{AnyElement, ClipboardItem, FocusHandle};
use oak_search::{FileIndex, GrepQuery};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

/// What runs when a confirm dialog's button is pressed: the button's index.
pub(super) type DialogCallback = Box<dyn FnOnce(&mut Workspace, usize, &mut Context<Workspace>)>;

/// The chords for the three panel toggles' tooltips.
pub(super) fn toggle_hints(cx: &App) -> [Option<SharedString>; 3] {
    [
        hint_for(cx, "toggle-left-panel").map(Into::into),
        hint_for(cx, "toggle-bottom-panel").map(Into::into),
        hint_for(cx, "toggle-right-panel").map(Into::into),
    ]
}

/// Which panel a tool opens in when Goto or the toolbar asks for it:
/// Fleet's fresh window puts Files, Search, Git and History on the left;
/// the run-time tools below; the rest on the right (estimate).
fn tool_side(tool: Tool) -> Side {
    match tool {
        Tool::Files | Tool::Search | Tool::Git | Tool::History | Tool::Snapshots => Side::Left,
        Tool::Terminal
        | Tool::Run
        | Tool::Debug
        | Tool::DebuggerConsole
        | Tool::BuildOutput
        | Tool::Problems
        | Tool::Breakpoints => Side::Bottom,
        Tool::Notifications | Tool::Preview => Side::Right,
    }
}

impl Workspace {
    // ---- setup ----------------------------------------------------------------------

    /// The open documents' unsaved text, read through the workspace when a
    /// search runs (the Search tool's and the Goto panel's Text Search tab).
    fn overrides(cx: &mut Context<Self>) -> crate::search_tool::Overrides {
        let ws = cx.entity();
        Rc::new(move |cx| {
            ws.read(cx)
                .documents
                .iter()
                .filter_map(|d| {
                    let d = d.read(cx);
                    Some((d.path.clone()?, d.buffer.text()))
                })
                .collect()
        })
    }

    /// The Search tool in the panel (`search/jump-to-tool`).
    pub(super) fn make_search_tool(root: PathBuf, cx: &mut Context<Self>) -> Entity<SearchTool> {
        let overrides = Self::overrides(cx);
        cx.new(|cx| SearchTool::new(root, overrides, cx))
    }

    /// Wires the M3 entities once the workspace exists.
    pub(super) fn init_workbench(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search_tool = self.search_tool.clone();
        cx.subscribe_in(&search_tool, window, Self::on_search_tool_event)
            .detach();
        let notifications = self.notifications.clone();
        cx.observe(&notifications, |_, _, cx| cx.notify()).detach();
        self.reindex(cx);
    }

    /// Rebuilds the file list for `goto/file` on the background executor.
    pub(super) fn reindex(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let exclude = Self::settings(cx).files_exclude();
        cx.spawn(async move |this, cx| {
            let index = cx
                .background_spawn(async move { FileIndex::build(&root, &exclude) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.file_index = Arc::new(index);
                cx.notify();
            });
        })
        .detach();
    }

    /// After a file was created, renamed, moved or deleted: the tree, the
    /// file index, the Search tool and an open Text Search follow.
    pub(super) fn files_changed(&mut self, cx: &mut Context<Self>) {
        self.file_tree.update(cx, |tree, cx| tree.reload(cx));
        self.refresh_git(cx);
        self.reindex(cx);
        self.search_tool.update(cx, |tool, cx| tool.refresh(cx));
        if let Some(goto) = self.goto.clone() {
            goto.update(cx, |g, cx| g.refresh(cx));
        }
    }

    pub(super) fn notify(
        &mut self,
        level: Level,
        title: impl Into<SharedString>,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.notifications
            .update(cx, |n, cx| n.push(level, title, message, cx));
    }

    pub(super) fn notify_error(&mut self, title: &str, message: String, cx: &mut Context<Self>) {
        eprintln!("oak: {title}: {message}");
        self.notify(Level::Error, title.to_string(), Some(message), cx);
    }

    /// Remembers `path` as the most recently shown file.
    pub(super) fn touch_recent(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(50);
    }

    /// Focus moves after the current update, through this workspace's window.
    pub(super) fn focus_later(&self, handle: FocusHandle, cx: &mut Context<Self>) {
        let window = self.window;
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| handle.focus(window, cx));
        });
    }

    /// The files an action on "the current file" means: every selected row
    /// when the tree has the focus (the root row excepted), else the active tab's file.
    pub(super) fn target_files(&self, window: &Window, cx: &App) -> Vec<PathBuf> {
        let tree = self.file_tree.read(cx);
        if tree.focus_handle(cx).contains_focused(window, cx) {
            let root = tree.root_path().to_path_buf();
            return tree
                .selected_paths()
                .into_iter()
                .filter(|p| *p != root)
                .collect();
        }
        self.active_editor()
            .and_then(|e| e.read(cx).path(cx))
            .into_iter()
            .collect()
    }

    // ---- dialogs ----------------------------------------------------------------------

    /// Fleet's confirm dialog; `on` runs with the index of the button pressed.
    pub(super) fn confirm(
        &mut self,
        title: String,
        message: Option<SharedString>,
        buttons: Vec<DialogButton>,
        cx: &mut Context<Self>,
        on: impl FnOnce(&mut Workspace, usize, &mut Context<Workspace>) + 'static,
    ) {
        let dialog = cx.new(|cx| ConfirmDialog::new(title, message, buttons, cx));
        cx.subscribe(&dialog, |this, dialog, event: &DialogEvent, cx| {
            if this.dialog.as_ref() != Some(&dialog) {
                return;
            }
            this.dialog = None;
            let callback = this.dialog_callback.take();
            if let (DialogEvent::Chosen(ix), Some(callback)) = (event, callback) {
                callback(this, *ix, cx);
            }
            this.focus_active_editor(cx);
            cx.notify();
        })
        .detach();
        let handle = dialog.read(cx).focus_handle(cx);
        self.dialog = Some(dialog);
        self.dialog_callback = Some(Box::new(on));
        self.focus_later(handle, cx);
        cx.notify();
    }

    /// `quit` and `close-window`: unsaved changes ask first; so does
    /// `warn.before.quitting`.
    pub(super) fn request_quit(&mut self, cx: &mut Context<Self>) {
        let dirty = self.documents.iter().any(|d| d.read(cx).is_dirty());
        if dirty {
            self.confirm(
                "Save changes before closing?".into(),
                Some("Some files have unsaved changes.".into()),
                vec![
                    DialogButton::primary("Save All"),
                    DialogButton::secondary("Don't Save"),
                    DialogButton::secondary("Cancel"),
                ],
                cx,
                |this, choice, cx| match choice {
                    0 => {
                        this.save_dirty(cx);
                        cx.quit();
                    }
                    1 => cx.quit(),
                    _ => {}
                },
            );
        } else if Self::settings(cx).warn_before_quitting() {
            self.confirm(
                "Quit Oak?".into(),
                None,
                vec![
                    DialogButton::primary("Quit"),
                    DialogButton::secondary("Cancel"),
                ],
                cx,
                |_, choice, cx| {
                    if choice == 0 {
                        cx.quit();
                    }
                },
            );
        } else {
            cx.quit();
        }
    }

    // ---- menus -------------------------------------------------------------------------

    pub(super) fn open_menu(
        &mut self,
        entries: Vec<MenuEntry>,
        position: Point<Pixels>,
        return_focus: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) {
        if entries.is_empty() {
            return;
        }
        let menu = cx.new(|cx| ContextMenu::new(entries, position, return_focus, cx));
        cx.subscribe(&menu, |this, menu, _: &MenuEvent, cx| {
            // Activated or dismissed: the menu goes either way.
            if this.menu.as_ref() == Some(&menu) {
                this.menu = None;
                cx.notify();
            }
        })
        .detach();
        let handle = menu.read(cx).focus_handle(cx);
        self.menu = Some(menu);
        self.focus_later(handle, cx);
        cx.notify();
    }

    /// Rows from Fleet action ids; `"-"` is a separator, `"label...=id"`
    /// a row with its own label (Fleet writes an ellipsis on rows that ask).
    fn entries(specs: &[&str], cx: &App) -> Vec<MenuEntry> {
        specs
            .iter()
            .filter_map(|spec| {
                if *spec == "-" {
                    Some(MenuEntry::Separator)
                } else if let Some((label, id)) = spec.split_once('=') {
                    MenuEntry::labelled(label.to_string(), id, cx)
                } else {
                    MenuEntry::action(spec, cx)
                }
            })
            .collect()
    }

    /// `Open In ›`, as Fleet's tree and tab menus carry it.
    fn open_in_submenu(cx: &App) -> MenuEntry {
        MenuEntry::submenu(
            "Open In",
            Self::entries(
                &[
                    "Associated Application=open-in-associated-application",
                    "File Manager=files/show-in-file-manager",
                ],
                cx,
            ),
        )
    }

    /// The Files tree's menu (batch 7): New File..., New Folder..., Find in
    /// Files..., Open In ›, Rename..., Delete..., Copy Path, Copy Relative
    /// Path. Fleet's Cut, Copy, Paste and Git › rows wait for the file
    /// clipboard and M4.
    fn file_menu(&self, is_dir: bool, cx: &App) -> Vec<MenuEntry> {
        let mut entries = Self::entries(
            &[
                "New File...=new-file",
                "New Folder...=files/new-folder",
                "-",
            ],
            cx,
        );
        if is_dir {
            entries.extend(Self::entries(&["Find in Files...=find-in-path"], cx));
        }
        entries.extend(MenuEntry::labelled("Open in Split", "open-in-split", cx));
        entries.push(Self::open_in_submenu(cx));
        entries.extend(Self::entries(
            &["-", "Rename...=files/rename", "Delete...=files/delete", "-"],
            cx,
        ));
        // Fleet's file clipboard rows (Cut, Copy, Paste) and Git arrive with M4.
        entries.push(MenuEntry::disabled("Cut"));
        entries.push(MenuEntry::disabled("Copy"));
        entries.extend(Self::entries(&["copy-path", "copy-relative-path"], cx));
        entries.push(MenuEntry::disabled("Paste"));
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::submenu(
            "Git",
            vec![MenuEntry::disabled("Git arrives with M4")],
        ));
        entries
    }

    /// The tree menu's "Open in Split" (a row without an action id of its
    /// own in Fleet): a split to the right showing the file.
    pub(super) fn open_in_split(
        &mut self,
        _: &OpenInSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.target_file(window, cx) else {
            return;
        };
        if path.is_dir() {
            return;
        }
        self.split_right(&SplitRight, window, cx);
        self.open_path(path, false, cx);
    }

    /// `showShortcuts` (Fleet: "Make the shortcuts that you are using at the
    /// moment appear on the screen"): every chord with a modifier or a
    /// function key shows in the pill at the window's bottom for a moment.
    /// gpui runs the key bindings before any element listener and a bound
    /// keystroke never reaches them, so the interceptor that runs before the
    /// bindings is the hook (estimate: Fleet shows the chords that ran an
    /// action).
    pub(super) fn watch_shortcuts(&mut self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.intercept_keystrokes(move |event, _, cx| {
            let k = &event.keystroke;
            let chord = k.modifiers.control || k.modifiers.alt || k.modifiers.platform;
            let function =
                k.key.len() > 1 && k.key.starts_with('f') && k.key[1..].parse::<u8>().is_ok();
            if !(chord || function) || !Self::settings(cx).show_shortcuts() {
                return;
            }
            let text = crate::keymap::presentable(&k.to_string());
            let _ = weak.update(cx, |this, cx| this.show_shortcut(text.into(), cx));
        })
        .detach();
    }

    fn show_shortcut(&mut self, text: SharedString, cx: &mut Context<Self>) {
        self.shortcut_hint = Some(text);
        self.shortcut_generation = self.shortcut_generation.wrapping_add(1);
        let generation = self.shortcut_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.shortcut_generation == generation {
                    this.shortcut_hint = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn render_shortcut_overlay(&self, theme: &Theme, m: &Metrics) -> Option<AnyElement> {
        let text = self.shortcut_hint.clone()?;
        Some(
            deferred(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(m.shortcut_overlay_bottom))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .h(px(m.shortcut_overlay_height))
                            .px(px(m.shortcut_overlay_pad_x))
                            .flex()
                            .items_center()
                            .rounded(px(m.shortcut_overlay_radius))
                            .bg(theme.shortcut_overlay_bg)
                            .font_family(theme.ui_font.clone())
                            .text_size(px(m.shortcut_overlay_font))
                            .text_color(theme.shortcut_overlay_text)
                            .child(text),
                    ),
            )
            .with_priority(100)
            .into_any_element(),
        )
    }

    /// Reads the branch and every file's git status again: on open, after
    /// a save or a file operation, and every fifth disk poll.
    pub(super) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let branch = repo.branch().map(SharedString::from);
        if branch != self.branch {
            self.branch = branch;
            cx.notify();
        }
        match repo.status() {
            Ok(statuses) => self
                .file_tree
                .update(cx, |tree, cx| tree.set_statuses(statuses, cx)),
            Err(e) => eprintln!("oak: {e:#}"),
        }
        // The open files' line status (`editor.showLineStatus`).
        let show = Self::settings(cx).editor_show_line_status();
        for doc in self.documents.clone() {
            let Some(path) = doc.read(cx).path.clone() else {
                continue;
            };
            let hunks = if show {
                repo.line_status(&path).unwrap_or_default()
            } else {
                Vec::new()
            };
            doc.update(cx, |d, cx| d.set_hunks(hunks, cx));
        }
    }

    /// The branch tag's menu (ninth pass): "Create New Branch..." (later),
    /// then every local branch, the current one with a dot and its counts
    /// against the upstream; a row checks its branch out.
    pub(super) fn show_branches(
        &mut self,
        at: Point<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let m = *cx.global::<Metrics>();
        let mut entries = vec![MenuEntry::disabled("Create New Branch...")];
        match repo.branches() {
            Ok(branches) => {
                for b in branches {
                    let label = match (b.current, b.upstream) {
                        (true, Some(u)) => format!("● {} ↑{} ↓{}", b.name, u.ahead, u.behind),
                        (true, None) => format!("● {}", b.name),
                        (false, _) => format!("   {}", b.name),
                    };
                    let repo = repo.clone();
                    let name = b.name.clone();
                    let weak = cx.weak_entity();
                    entries.push(MenuEntry::custom(label, move |_, cx| {
                        if let Err(e) = repo.checkout(&name) {
                            eprintln!("oak: {e:#}");
                        }
                        let _ = weak.update(cx, |this, cx| {
                            this.refresh_git(cx);
                            this.files_changed(cx);
                        });
                    }));
                }
            }
            Err(e) => eprintln!("oak: {e:#}"),
        }
        let position = point(
            at.x - px(m.branch_menu_offset_x),
            px(m.toolbar_height + m.branch_menu_gap),
        );
        self.open_menu(entries, position, None, cx);
    }

    /// `git.autoFetch`: a `git fetch` every `git.fetchInterval` minutes, off
    /// the main thread, counted on the disk poll's seconds.
    fn auto_fetch(&mut self, cx: &mut Context<Self>) {
        let settings = Self::settings(cx);
        if !settings.git_auto_fetch() {
            return;
        }
        let every = (settings.git_fetch_interval_min() * 60.).round().max(1.) as u32;
        if self.disk_ticks % every != 0 {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        cx.background_spawn(async move {
            if let Err(e) = repo.fetch() {
                eprintln!("oak: {e:#}");
            }
        })
        .detach();
    }

    /// Starts the once-a-second look at the open files' modification times.
    pub(super) fn watch_disk(&mut self, cx: &mut Context<Self>) {
        self.disk_poll = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(1))
                .await;
            if this.update(cx, |ws, cx| ws.check_disk(cx)).is_err() {
                break;
            }
        }));
    }

    /// A file changed under an open document: reloaded when the document is
    /// clean, else the editor shows Fleet's banner with Overwrite and Reload.
    fn check_disk(&mut self, cx: &mut Context<Self>) {
        self.disk_ticks = self.disk_ticks.wrapping_add(1);
        if self.disk_ticks % 5 == 0 {
            self.refresh_git(cx);
        }
        self.auto_fetch(cx);
        let mut changed = false;
        for doc in self.documents.clone() {
            if doc.read(cx).disk_changed() {
                doc.update(cx, |d, cx| d.note_disk_change(cx));
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// "File has been changed on disk": the pill over the editor, Overwrite
    /// saving the document's text, Reload taking the disk's.
    pub(super) fn render_banner(
        &self,
        editor: Entity<EditorView>,
        theme: &Theme,
        m: &Metrics,
    ) -> impl IntoElement {
        let overwrite = editor.clone();
        let reload = editor;
        let button = |id: &'static str, label: &'static str| {
            crate::dialog::button(
                id,
                label.into(),
                crate::dialog::ButtonKind::Primary,
                false,
                theme,
                m,
            )
            .h(px(m.banner_button_height))
            .border_0()
            .px(px(m.banner_button_pad_x))
        };
        div()
            .absolute()
            .top(px(m.banner_top))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .h(px(m.banner_height))
                    .pl(px(m.banner_pad_left))
                    .pr(px(m.banner_pad_right))
                    .flex()
                    .items_center()
                    .gap(px(m.banner_gap))
                    .rounded(px(m.popup_radius))
                    .border_1()
                    .border_color(theme.popup_border)
                    .bg(theme.popup_bg)
                    .font_family(theme.ui_font.clone())
                    .text_size(px(m.ui_font_size))
                    .text_color(theme.text_primary)
                    .shadow(vec![BoxShadow {
                        color: theme.shadow_medium,
                        offset: point(px(0.), px(m.popup_shadow_offset_y)),
                        blur_radius: px(m.popup_shadow_blur),
                        spread_radius: px(0.),
                    }])
                    .child("File has been changed on disk")
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(m.banner_button_gap))
                            .child(button("banner-overwrite", "Overwrite").on_click(
                                move |_, _, cx| {
                                    overwrite.update(cx, |e, cx| {
                                        if let Err(err) = e.save(cx) {
                                            eprintln!("oak: cannot save: {err:#}");
                                        }
                                    });
                                },
                            ))
                            .child(button("banner-reload", "Reload").on_click(move |_, _, cx| {
                                reload.update(cx, |e, cx| {
                                    e.document.update(cx, |d, cx| {
                                        if let Err(err) = d.reload_from_disk(cx) {
                                            eprintln!("oak: cannot reload: {err:#}");
                                        }
                                    });
                                });
                            })),
                    ),
            )
    }

    /// A tab's menu (batch 7): Close, Close Other Tabs, Close All Tabs, Close
    /// Saved Tabs, Split Right, Open In ›. Fleet's Split Down and Pin wait
    /// for vertical splits and pinned tabs.
    fn tab_menu(cx: &App) -> Vec<MenuEntry> {
        let mut entries = Self::entries(
            &[
                "Close=close-tab",
                "tabs/close-other-tabs",
                "tabs/close-all-tabs",
                "tabs/close-saved-tabs",
                "-",
                "split-right",
                "-",
            ],
            cx,
        );
        entries.push(Self::open_in_submenu(cx));
        entries
    }

    /// The editor's menu (batch 7): Go To ›, Cut, Copy, Paste. Fleet's Quick
    /// Fixes, Refactor, Reformat Code and Git rows are M4 and M7.
    fn editor_menu(cx: &App) -> Vec<MenuEntry> {
        let mut entries = vec![MenuEntry::submenu(
            "Go To",
            Self::entries(
                &[
                    "goto/line",
                    "goto/file",
                    "goto/symbol",
                    "show-file-symbols",
                    "-",
                    "goto/everything",
                ],
                cx,
            ),
        )];
        entries.extend(Self::entries(&["-", "cut", "copy", "paste"], cx));
        entries
    }

    /// The main menu (batch 7): Fleet's nine submenus. Run and Git hold a
    /// greyed row until M4 and M6.
    fn main_menu(cx: &App) -> Vec<MenuEntry> {
        vec![
            MenuEntry::submenu(
                "File",
                Self::entries(
                    &[
                        "New File...=new-file",
                        "New Folder...=files/new-folder",
                        "-",
                        "save-file",
                        "save-all",
                        "-",
                        "close-tab",
                        "close-window",
                        "-",
                        "open-settings",
                        "edit-settings-file",
                        "edit-keymap-file",
                        "-",
                        "Recent Workspaces=show-recent-workspaces",
                        "-",
                        "quit",
                    ],
                    cx,
                ),
            ),
            MenuEntry::submenu(
                "Edit",
                Self::entries(
                    &[
                        "undo",
                        "redo",
                        "-",
                        "cut",
                        "copy",
                        "paste",
                        "editor/paste-from-history",
                        "-",
                        "find",
                        "replace",
                        "find-in-path",
                        "replace-in-path",
                        "-",
                        "select-all",
                        "expand-selection",
                        "shrink-selection",
                        "-",
                        "editor/change-line-endings",
                    ],
                    cx,
                ),
            ),
            MenuEntry::submenu(
                "View",
                Self::entries(
                    &[
                        "toggle-left-panel",
                        "toggle-bottom-panel",
                        "toggle-right-panel",
                        "hide-all-panels",
                        "-",
                        "split-right",
                        "unsplit",
                        "-",
                        "zoom-in",
                        "zoom-out",
                        "reset-zoom",
                        "-",
                        "toggle-full-screen",
                        "toggle-hidden-files",
                    ],
                    cx,
                ),
            ),
            MenuEntry::submenu(
                "Goto",
                Self::entries(
                    &[
                        "goto/everything",
                        "goto/file",
                        "goto/symbol",
                        "goto/action",
                        "goto/tool",
                        "-",
                        "goto/line",
                        "show-file-symbols",
                        "-",
                        "goto/go-to-next-tab",
                        "goto/go-to-previous-tab",
                    ],
                    cx,
                ),
            ),
            MenuEntry::submenu(
                "Code",
                Self::entries(
                    &[
                        "comment-line",
                        "comment-block",
                        "-",
                        "expand-selection",
                        "shrink-selection",
                        "editor/move-caret-to-matching-bracket",
                        "-",
                        "toggle-case",
                        "join-lines",
                        "editor/duplicate-line",
                        "editor/delete-line",
                        "editor/expand-folding",
                        "editor/collapse-folding",
                    ],
                    cx,
                ),
            ),
            MenuEntry::submenu(
                "Run",
                vec![MenuEntry::disabled("Run configurations arrive with M6")],
            ),
            MenuEntry::submenu("Git", vec![MenuEntry::disabled("Git arrives with M4")]),
            MenuEntry::submenu(
                "Window",
                Self::entries(
                    &[
                        "minimize-window",
                        "toggle-full-screen",
                        "-",
                        "Recent Workspaces=show-recent-workspaces",
                        "notifications/toggle",
                    ],
                    cx,
                ),
            ),
            MenuEntry::submenu(
                "Help",
                Self::entries(&["edit-keymap-file", "-", "about"], cx),
            ),
        ]
    }

    /// The tree's events beyond opening a file: its menu, and rows moved by drag and drop.
    pub(super) fn on_file_tree_event(&mut self, event: &FileTreeEvent, cx: &mut Context<Self>) {
        match event {
            FileTreeEvent::ContextMenu { is_dir, position } => {
                let entries = self.file_menu(*is_dir, cx);
                let focus = self.file_tree.read(cx).focus_handle(cx);
                self.open_menu(entries, *position, Some(focus), cx);
            }
            FileTreeEvent::Moved { from, to } => self.path_renamed(from, to.clone(), cx),
            FileTreeEvent::Renamed { from, to } => {
                self.path_renamed(from, to.clone(), cx);
                let handle = self.file_tree.read(cx).focus_handle(cx);
                self.focus_later(handle, cx);
            }
            FileTreeEvent::Open { .. } => {}
        }
    }

    /// A right click on a tab: the tab becomes active, then its menu opens.
    pub(super) fn tab_context_menu(
        &mut self,
        p: usize,
        ix: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_tab(p, ix, window, cx);
        let focus = self.active_editor().map(|e| e.read(cx).focus_handle(cx));
        let entries = Self::tab_menu(cx);
        self.open_menu(entries, position, focus, cx);
    }

    /// A right click in an editor.
    pub(super) fn editor_context_menu(
        &mut self,
        editor: &Entity<EditorView>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let focus = editor.read(cx).focus_handle(cx);
        let entries = Self::editor_menu(cx);
        self.open_menu(entries, position, Some(focus), cx);
    }

    // ---- the Goto panel -------------------------------------------------------------------

    fn goto_sources(&mut self, cx: &mut Context<Self>) -> GotoSources {
        let (show_shortcuts, symbols_first, exclude, search_big_files) = {
            let s = Self::settings(cx);
            (
                s.goto_panel_show_shortcuts(),
                s.ff_preview_file_symbols_in_goto(),
                s.files_exclude(),
                s.ff_search_in_big_unindexed_files(),
            )
        };
        let active = self.active_editor().map(|e| e.read(cx).document.clone());
        let mut docs: Vec<Entity<Document>> = self.documents.clone();
        if let Some(active) = &active {
            docs.retain(|d| d != active);
            docs.insert(0, active.clone());
        }
        let symbols = docs
            .iter()
            .filter_map(|d| {
                d.update(cx, |d, _| {
                    let path = d.path.clone()?;
                    let text = d.buffer.text();
                    let symbols = d.syntax().map(|t| t.symbols(&text)).unwrap_or_default();
                    Some((path, symbols))
                })
            })
            .collect();
        let active_tools: Vec<Tool> = self
            .panels
            .iter()
            .flat_map(|p| p.tools.iter().copied())
            .collect();
        GotoSources {
            root: self.root.clone(),
            files: self.file_index.clone(),
            recent: self.recent.clone(),
            symbols,
            tools: Tool::ALL.to_vec(),
            active_tools,
            workspaces: Layout::recent_workspaces(),
            preview_hidden: self.goto_preview_hidden,
            show_shortcuts,
            symbols_first,
            overrides: Self::overrides(cx),
            exclude,
            search_big_files,
        }
    }

    /// Opens the Goto panel in `mode`, or switches the open one to it. The
    /// field starts with the editor's one-line selection when
    /// `editor.useSelectionForSearch` is on.
    pub(super) fn open_goto(
        &mut self,
        mode: GotoMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(goto) = self.goto.clone() {
            goto.update(cx, |g, cx| g.set_mode(mode, cx));
            goto.read(cx).focus_handle(cx).focus(window, cx);
            return;
        }
        let use_selection = Self::settings(cx).editor_use_selection_for_search();
        let threshold = Self::settings(cx).local_big_file_minimum_threshold_mb();
        let initial = if use_selection && mode != GotoMode::Workspaces {
            self.active_editor()
                .cloned()
                .map(|e| e.update(cx, |e, cx| e.selected_text(cx)))
                .filter(|t| !t.is_empty() && !t.contains('\n'))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let sources = self.goto_sources(cx);
        // The preview reads an open document, else the file (never a big one);
        // a fresh document stays the panel's own and goes with it.
        let documents = self.documents.clone();
        let open_document: crate::goto::OpenDocument = Rc::new(move |path, cx| {
            if let Some(d) = documents
                .iter()
                .find(|d| d.read(cx).path.as_deref() == Some(path))
            {
                return Some(d.clone());
            }
            let size = std::fs::metadata(path).ok()?.len() as f64 / 1024. / 1024.;
            if size >= threshold {
                return None;
            }
            let text = String::from_utf8(std::fs::read(path).ok()?).ok()?;
            Some(cx.new(|cx| {
                let settings = &cx.global::<AppSettings>().settings;
                Document::new(Some(path.to_path_buf()), &text, settings)
            }))
        });
        let goto = cx.new(|cx| GotoPanel::new(mode, sources, &initial, open_document, cx));
        cx.subscribe_in(&goto, window, Self::on_goto_event).detach();
        goto.read(cx).focus_handle(cx).focus(window, cx);
        self.goto = Some(goto);
        cx.notify();
    }

    /// `find-in-path` / `replace-in-path`: the Text Search tab.
    fn open_text_search(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.open_goto(GotoMode::TextSearch, window, cx);
        if replace {
            if let Some(goto) = self.goto.clone() {
                goto.update(cx, |g, cx| g.set_replace_mode(true, cx));
            }
        }
    }

    fn on_goto_event(
        &mut self,
        goto: &Entity<GotoPanel>,
        event: &GotoEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.goto.as_ref() != Some(goto) {
            return;
        }
        match event {
            GotoEvent::Open { target, split } => {
                self.goto = None;
                self.open_goto_target(target.clone(), *split, window, cx);
            }
            GotoEvent::Cancel => {
                self.goto = None;
                self.focus_editor_now(window, cx);
            }
            GotoEvent::PreviewToggled(hidden) => {
                self.goto_preview_hidden = *hidden;
                self.save_layout();
            }
            GotoEvent::ReplaceAll {
                query,
                replacement,
                files,
            } => self.apply_replace_all(query, replacement, files, cx),
            GotoEvent::ReplaceOne {
                path,
                row,
                range,
                replacement,
            } => {
                self.open_path(path.clone(), false, cx);
                if let Some(editor) = self.active_editor().cloned() {
                    let (row, range, replacement) = (*row, range.clone(), replacement.clone());
                    editor.update(cx, |e, cx| {
                        let chars = Self::line_bytes_to_chars(e, row, range, cx);
                        e.replace_range(chars, &replacement, cx);
                    });
                }
                self.files_changed(cx);
                goto.read(cx).focus_handle(cx).focus(window, cx);
            }
        }
        cx.notify();
    }

    /// A byte range inside a line, as char offsets into the document.
    fn line_bytes_to_chars(
        e: &EditorView,
        row: usize,
        range: Range<usize>,
        cx: &App,
    ) -> Range<usize> {
        let b = &e.document.read(cx).buffer;
        let line_start = b.line_to_char(row);
        let line = b.line(row);
        let count = |byte: usize| line.get(..byte).map(|s| s.chars().count()).unwrap_or(0);
        line_start + count(range.start)..line_start + count(range.end)
    }

    fn open_goto_target(
        &mut self,
        target: GotoTarget,
        split: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            GotoTarget::File(path) => {
                if split {
                    self.split_right(&SplitRight, window, cx);
                }
                self.open_path(path, false, cx);
                self.focus_editor_now(window, cx);
            }
            GotoTarget::Symbol { path, range } => {
                if split {
                    self.split_right(&SplitRight, window, cx);
                }
                self.open_path(path, false, cx);
                if let Some(editor) = self.active_editor().cloned() {
                    editor.update(cx, |e, cx| e.select_bytes(range, cx));
                }
                self.focus_editor_now(window, cx);
            }
            GotoTarget::TextMatch { path, row, range } => {
                if split {
                    self.split_right(&SplitRight, window, cx);
                }
                self.open_path(path, false, cx);
                if let Some(editor) = self.active_editor().cloned() {
                    editor.update(cx, |e, cx| {
                        let chars = Self::line_bytes_to_chars(e, row, range, cx);
                        e.select_range(chars, cx);
                    });
                }
                self.focus_editor_now(window, cx);
            }
            GotoTarget::Action(id) => {
                self.focus_editor_now(window, cx);
                if let Some(action) = action_for(&id) {
                    if is_editor_action(&id) && self.active_editor().is_none() {
                        return;
                    }
                    window.dispatch_action(action, cx);
                }
            }
            GotoTarget::Tool(Tool::Notifications) => {
                self.focus_editor_now(window, cx);
                if !self.notifications_open {
                    self.toggle_notifications(&ToggleNotifications, window, cx);
                }
            }
            GotoTarget::Tool(tool) => self.choose_tool(tool_side(tool), tool, window, cx),
            GotoTarget::Workspace(root) => {
                self.focus_editor_now(window, cx);
                crate::open_workspace_window(root, Vec::new(), false, (1280., 800.), cx);
            }
        }
    }

    // ---- actions ------------------------------------------------------------------------

    pub(super) fn goto_everything(
        &mut self,
        _: &GotoEverything,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_goto(GotoMode::Everything, window, cx);
    }
    pub(super) fn goto_file(&mut self, _: &GotoFile, window: &mut Window, cx: &mut Context<Self>) {
        self.open_goto(GotoMode::Files, window, cx);
    }
    pub(super) fn goto_symbol(
        &mut self,
        _: &GotoSymbol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_goto(GotoMode::Symbols, window, cx);
    }
    pub(super) fn goto_action(
        &mut self,
        _: &GotoAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_goto(GotoMode::Actions, window, cx);
    }
    pub(super) fn goto_tool(&mut self, _: &GotoTool, window: &mut Window, cx: &mut Context<Self>) {
        self.open_goto(GotoMode::Tools, window, cx);
    }
    pub(super) fn goto_recent_file(
        &mut self,
        _: &GotoRecentFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_goto(GotoMode::Files, window, cx);
    }
    /// `show-file-symbols`: the Symbols in File list for the active file.
    pub(super) fn show_file_symbols(
        &mut self,
        _: &ShowFileSymbols,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_editor().is_none() {
            return;
        }
        self.open_goto(GotoMode::SymbolsInFile, window, cx);
    }
    pub(super) fn toggle_goto_preview(
        &mut self,
        _: &ToggleGotoPreview,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.goto.clone() {
            Some(goto) => goto.update(cx, |g, cx| g.toggle_preview(cx)),
            None => {
                self.goto_preview_hidden = !self.goto_preview_hidden;
                self.save_layout();
            }
        }
    }
    pub(super) fn show_recent_workspaces(
        &mut self,
        _: &ShowRecentWorkspaces,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_goto(GotoMode::Workspaces, window, cx);
    }
    pub(super) fn find_in_path(
        &mut self,
        _: &FindInPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_text_search(false, window, cx);
    }
    pub(super) fn replace_in_path(
        &mut self,
        _: &ReplaceInPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_text_search(true, window, cx);
    }
    /// `search/jump-to-tool`: the Search tool in its panel with the field focused.
    pub(super) fn search_jump_to_tool(
        &mut self,
        _: &SearchJumpToTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_tool(Side::Left, Tool::Search, window, cx);
        self.search_tool
            .update(cx, |tool, cx| tool.show(false, None, window, cx));
    }
    pub(super) fn files_jump_to_tool(
        &mut self,
        _: &FilesJumpToTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_tool(Side::Left, Tool::Files, window, cx);
    }

    pub(super) fn new_folder(
        &mut self,
        _: &NewFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = self.target_dir(window, cx);
        self.open_prompt(PromptKind::NewFolder { dir }, "", Vec::new(), window, cx);
    }
    pub(super) fn toggle_hidden_files(
        &mut self,
        _: &ToggleHiddenFiles,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_tree.update(cx, |tree, cx| tree.toggle_hidden(cx));
    }
    pub(super) fn open_in_associated_application(
        &mut self,
        _: &OpenInAssociatedApplication,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for path in self.target_files(window, cx) {
            cx.open_with_system(&path);
        }
    }
    pub(super) fn show_in_file_manager(
        &mut self,
        _: &ShowInFileManager,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.target_files(window, cx).first() {
            cx.reveal_path(path);
        }
    }
    pub(super) fn copy_path(&mut self, _: &CopyPath, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.target_files(window, cx);
        if paths.is_empty() {
            return;
        }
        let text = paths
            .iter()
            .map(|p| display_path(p))
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
    pub(super) fn copy_relative_path(
        &mut self,
        _: &CopyRelativePath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.root.clone();
        let paths = self.target_files(window, cx);
        if paths.is_empty() {
            return;
        }
        let text = paths
            .iter()
            .map(|p| display_path(p.strip_prefix(&root).unwrap_or(p)))
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
    /// `tabs/close-all-tabs`: every tab of the active split.
    pub(super) fn close_all_tabs(
        &mut self,
        _: &CloseAllTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = self.active_pane;
        for ix in (0..self.panes[p].tabs.len()).rev() {
            if p < self.panes.len() {
                self.close_tab_at(p, ix, window, cx);
            }
        }
    }
    pub(super) fn about(&mut self, _: &About, _: &mut Window, cx: &mut Context<Self>) {
        self.notify(
            Level::Info,
            format!("Oak {}", env!("CARGO_PKG_VERSION")),
            Some("A native code editor in the shape of JetBrains Fleet.".into()),
            cx,
        );
    }

    /// `show-context-menu` from the keyboard: the focused view's menu at the pointer.
    pub(super) fn show_context_menu(
        &mut self,
        _: &ShowContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = window.mouse_position();
        let tree = self.file_tree.clone();
        if tree.read(cx).focus_handle(cx).contains_focused(window, cx) {
            tree.update(cx, |tree, cx| tree.request_context_menu(position, cx));
        } else if let Some(editor) = self.active_editor().cloned() {
            self.editor_context_menu(&editor, position, cx);
        }
    }
    /// The main menu, under the toolbar's menu button (batch 7: its left edge
    /// on the button's, 2 px below the toolbar).
    pub(super) fn show_main_menu(
        &mut self,
        _: &ShowMainMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let m = *cx.global::<Metrics>();
        let position = point(
            px(m.toolbar_pad_x + m.toolbar_button + m.toolbar_gap),
            px(m.toolbar_height + m.popup_offset_y),
        );
        let focus = window.focused(cx);
        let entries = Self::main_menu(cx);
        self.open_menu(entries, position, focus, cx);
    }
    /// `open-settings`: the Settings tool as a tab in the active split (one
    /// per workspace; an open one is activated).
    pub(super) fn open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for p in 0..self.panes.len() {
            if let Some(ix) = self.panes[p]
                .tabs
                .iter()
                .position(|t| matches!(t.view, TabView::Settings(_)))
            {
                self.activate_tab(p, ix, window, cx);
                return;
            }
        }
        let root_name = self.file_tree.read(cx).root_name();
        let view = cx.new(|cx| SettingsView::new(root_name, cx));
        let p = self.active_pane;
        let pane = &mut self.panes[p];
        pane.tabs.push(Tab {
            view: TabView::Settings(view),
            preview: false,
        });
        pane.active = Some(pane.tabs.len() - 1);
        pane.scroll.scroll_to_item(pane.tabs.len() - 1);
        self.focus_editor_now(window, cx);
        self.after_activate(cx);
    }
    /// `notifications/toggle`: Fleet's Notifications popup under the toolbar's
    /// bell (batch 7); opening it reads everything.
    pub(super) fn toggle_notifications(
        &mut self,
        _: &ToggleNotifications,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notifications_open = !self.notifications_open;
        if self.notifications_open {
            self.notifications.update(cx, |n, cx| n.mark_all_read(cx));
        }
        cx.notify();
    }

    /// The Notifications popup: a 32 px header (title, two icon buttons), a
    /// separator, then the rows or the empty state.
    fn render_notifications_popup(&self, theme: &Theme, m: &Metrics, cx: &App) -> AnyElement {
        let empty = self.notifications.read(cx).is_empty();
        let icon_button =
            |id: &'static str, path: &'static str, action: fn() -> Box<dyn Action>| {
                div()
                    .id(id)
                    .size(px(m.goto_icon_button))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(m.button_radius))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.ghost_hovered))
                    .on_click(move |_, window, cx| window.dispatch_action(action(), cx))
                    .child(icon(path, m.icon_size, theme.icon_primary))
            };
        let header = div()
            .h(px(m.notifications_header - 1.))
            .flex_shrink_0()
            .pl(px(m.notifications_pad_x))
            .pr(px(m.notifications_icon_gap))
            .flex()
            .items_center()
            .gap(px(m.notifications_icon_gap))
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Notifications"),
            )
            .child(icon_button(
                "notifications-settings",
                "expui/general/settings",
                || Box::new(OpenSettings),
            ))
            .child(icon_button(
                "notifications-clear",
                "expui/general/delete",
                || Box::new(ClearNotifications),
            ));
        let body: AnyElement = if empty {
            div()
                .h(px(m.notifications_empty_height))
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_secondary)
                .child("You don't have any unread notifications")
                .into_any_element()
        } else {
            div()
                .max_h(px(m.goto_max_height / 2.))
                .child(self.notifications.clone())
                .into_any_element()
        };
        deferred(
            div()
                .id("notifications-popup")
                .occlude()
                .absolute()
                .right(px(m.notifications_right))
                .top(px(m.toolbar_height + m.notifications_top_gap))
                .w(px(m.notifications_width))
                .flex()
                .flex_col()
                .rounded(px(m.popup_radius))
                .border_1()
                .border_color(theme.popup_border)
                .bg(theme.popup_bg)
                .font_family(theme.ui_font.clone())
                .text_size(px(m.ui_font_size))
                .text_color(theme.popup_text)
                .shadow(vec![BoxShadow {
                    color: theme.shadow_medium,
                    offset: point(px(0.), px(m.popup_shadow_offset_y)),
                    blur_radius: px(m.popup_shadow_blur),
                    spread_radius: px(0.),
                }])
                .on_mouse_down_out(|_, window, cx| {
                    window.dispatch_action(Box::new(ToggleNotifications), cx)
                })
                .child(header)
                .child(div().h(px(1.)).flex_shrink_0().bg(theme.popup_separator))
                .child(body),
        )
        .into_any_element()
    }
    pub(super) fn clear_notifications(
        &mut self,
        _: &ClearNotifications,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notifications.update(cx, |n, cx| n.clear(cx));
    }

    /// `local-history/make-snapshot`: a copy of the active file under Oak's
    /// `local-history/` folder, named by the time.
    pub(super) fn make_snapshot(
        &mut self,
        _: &MakeSnapshot,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor().cloned() else {
            return;
        };
        let Some(path) = editor.read(cx).path(cx) else {
            return;
        };
        let Some(dir) = oak_settings::oak_config_dir() else {
            return;
        };
        let flat: String = path
            .display()
            .to_string()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let folder = dir.join("local-history").join(flat);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let target = folder.join(format!("{stamp}-{name}"));
        let text = editor.read(cx).document.read(cx).buffer.text_for_file();
        let result = std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(&target, text));
        match result {
            Ok(()) => self.notify(
                Level::Info,
                "Snapshot saved",
                Some(target.display().to_string()),
                cx,
            ),
            Err(e) => self.notify_error(
                "Cannot save snapshot",
                format!("{}: {e}", target.display()),
                cx,
            ),
        }
    }

    // ---- the Search tool's events ------------------------------------------------------

    fn on_search_tool_event(
        &mut self,
        _: &Entity<SearchTool>,
        event: &SearchToolEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SearchToolEvent::OpenMatch {
                path,
                row,
                range,
                split,
            } => {
                self.open_goto_target(
                    GotoTarget::TextMatch {
                        path: path.clone(),
                        row: *row,
                        range: range.clone(),
                    },
                    *split,
                    window,
                    cx,
                );
            }
            SearchToolEvent::ReplaceAll {
                query,
                replacement,
                files,
            } => self.apply_replace_all(query, replacement, files, cx),
        }
    }

    /// `replace-in-path`: open documents are edited (one undo step each),
    /// the rest rewritten on disk.
    fn apply_replace_all(
        &mut self,
        query: &GrepQuery,
        replacement: &str,
        files: &[PathBuf],
        cx: &mut Context<Self>,
    ) {
        let mut files_changed = 0;
        let mut count = 0;
        for path in files {
            let view = self
                .panes
                .iter()
                .flat_map(|p| p.tabs.iter().filter_map(|t| t.editor().cloned()))
                .find(|e| e.read(cx).path(cx).as_deref() == Some(path.as_path()));
            if let Some(editor) = view {
                let edits: Vec<(Range<usize>, String)> = {
                    let b = &editor.read(cx).document.read(cx).buffer;
                    let text = b.text();
                    query
                        .search_text(&text)
                        .iter()
                        .flat_map(|l| {
                            let start = b.line_to_char(l.row);
                            l.ranges
                                .iter()
                                .map(|r| {
                                    let s = l.text[..r.start].chars().count();
                                    let e = l.text[..r.end].chars().count();
                                    (
                                        start + s..start + e,
                                        query.replacement_for(&l.text, r.clone(), replacement),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect()
                };
                if edits.is_empty() {
                    continue;
                }
                count += edits.len();
                files_changed += 1;
                editor.update(cx, |e, cx| e.replace_ranges(edits, cx));
            } else {
                let Ok(text) = std::fs::read_to_string(path) else {
                    continue;
                };
                let Some(new_text) = query.replace_in_text(&text, replacement) else {
                    continue;
                };
                count += query
                    .search_text(&text)
                    .iter()
                    .map(|l| l.ranges.len())
                    .sum::<usize>();
                files_changed += 1;
                if let Err(e) = std::fs::write(path, new_text) {
                    self.notify_error("Cannot write", format!("{}: {e}", path.display()), cx);
                }
            }
        }
        self.files_changed(cx);
        self.notify(
            Level::Info,
            "Replace in Files",
            Some(format!(
                "Replaced {count} occurrence{} in {files_changed} file{}",
                if count == 1 { "" } else { "s" },
                if files_changed == 1 { "" } else { "s" }
            )),
            cx,
        );
    }

    // ---- rendering -----------------------------------------------------------------------

    /// The popups over the window: the Goto panel, a menu, a dialog, the toasts.
    pub(super) fn render_workbench_overlays(
        &self,
        theme: &Theme,
        m: &Metrics,
        cx: &App,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(goto) = &self.goto {
            out.push(goto.clone().into_any_element());
        }
        if let Some(menu) = &self.menu {
            out.push(menu.clone().into_any_element());
        }
        if let Some(dialog) = &self.dialog {
            out.push(dialog.clone().into_any_element());
        }
        if let Some(toasts) = self.notifications.read(cx).render_toasts(theme, m, cx) {
            out.push(toasts.into_any_element());
        }
        if self.notifications_open {
            out.push(self.render_notifications_popup(theme, m, cx));
        }
        out.extend(self.render_shortcut_overlay(theme, m));
        out
    }

    /// `Notifications` is the tool's own view; the panel shows it directly.
    pub(super) fn notifications_view(&self) -> Entity<Notifications> {
        self.notifications.clone()
    }
}
