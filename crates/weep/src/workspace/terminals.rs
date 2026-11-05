//! The Terminal tool's place in the workspace (M5): one tab per shell in the
//! bottom panel, opened by `terminal/new-tool`, `terminal/toggle`,
//! `terminal/jump-to-tool`, the `+` chooser and `launchTerminalOnOpen`;
//! closed by `close-tab`, the tab's menu or the shell's exit. The profiles
//! are `terminal.profiles` plus the shells the machine has
//! (`weep_terminal::Profile::discover`), `terminal.profile.default` picking one.

use super::*;
use crate::menu::MenuEntry;
use crate::terminal::{TerminalEvent, TerminalView};
use weep_terminal::Profile;

impl Workspace {
    /// The profiles on offer: `terminal.profiles`, then the shells found on
    /// this machine that the setting did not already name.
    pub(super) fn terminal_profiles(cx: &App) -> Vec<Profile> {
        let settings = Self::settings(cx);
        let mut profiles = Profile::from_setting(settings.terminal_profiles());
        for found in Profile::discover() {
            if !profiles.iter().any(|p| p.name == found.name) {
                profiles.push(found);
            }
        }
        profiles
    }

    /// `terminal.profile.default` among the profiles, else the first.
    pub(super) fn default_terminal_profile(cx: &App) -> Option<Profile> {
        let profiles = Self::terminal_profiles(cx);
        Profile::default_of(Self::settings(cx).terminal_profile_default(), &profiles).cloned()
    }

    pub(super) fn terminal_view(&self, id: usize) -> Option<&Entity<TerminalView>> {
        self.terminals
            .iter()
            .find(|(tid, _)| *tid == id)
            .map(|(_, view)| view)
    }

    /// A panel tab's label: the tool's name, or the terminal's title.
    pub(super) fn tool_tab_label(&self, tab: ToolTab, cx: &App) -> SharedString {
        match tab.tool {
            Tool::Terminal => self
                .terminal_view(tab.instance)
                .map(|v| v.read(cx).title())
                .unwrap_or_else(|| Tool::Terminal.label().into()),
            Tool::Run => self
                .run_view(tab.instance)
                .map(|v| v.read(cx).name().to_string().into())
                .unwrap_or_else(|| Tool::Run.label().into()),
            other => other.label().into(),
        }
    }

    /// Gives a panel tab the focus: the tree, the terminal, or the island.
    pub(super) fn focus_tool_tab(
        &mut self,
        side: Side,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.panel(side).tools.get(ix).copied() else {
            return;
        };
        match tab.tool {
            Tool::Files => self.file_tree.read(cx).focus_handle(cx).focus(window, cx),
            Tool::Terminal => {
                if let Some(view) = self.terminal_view(tab.instance) {
                    let handle = view.read(cx).focus_handle(cx);
                    handle.focus(window, cx);
                }
            }
            Tool::Run => {
                if let Some(view) = self.run_view(tab.instance) {
                    let handle = view.read(cx).focus_handle(cx);
                    handle.focus(window, cx);
                }
            }
            // M8: the debugger tools take the keys themselves.
            Tool::Debug => {
                let handle = self.debug_views.debug.read(cx).focus_handle(cx);
                handle.focus(window, cx);
            }
            Tool::Breakpoints => {
                let handle = self.debug_views.breakpoints.read(cx).focus_handle(cx);
                handle.focus(window, cx);
            }
            Tool::DebuggerConsole => {
                let handle = self.debug_views.console.read(cx).focus_handle(cx);
                handle.focus(window, cx);
            }
            _ => self.panel(side).focus_handle.focus(window, cx),
        }
    }

    /// Opens a shell with `profile` (the default when none) as a new tab in
    /// the bottom panel, and gives it the focus when asked.
    pub(super) fn new_terminal(
        &mut self,
        profile: Option<Profile>,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = profile.or_else(|| Self::default_terminal_profile(cx)) else {
            self.notify_error(
                "No shell found",
                "Add one to terminal.profiles in settings.json".to_string(),
                cx,
            );
            return;
        };
        let root = self.root.clone();
        let view = cx.new(|cx| TerminalView::new(profile, root, cx));
        cx.subscribe_in(&view, window, Self::on_terminal_event)
            .detach();
        let id = self.next_terminal;
        self.next_terminal += 1;
        self.terminals.push((id, view.clone()));
        let panel = self.panel_mut(Side::Bottom);
        panel.tools.push(ToolTab {
            tool: Tool::Terminal,
            instance: id,
        });
        panel.active = panel.tools.len() - 1;
        panel.open = true;
        self.chooser = None;
        if focus {
            let handle = view.read(cx).focus_handle(cx);
            handle.focus(window, cx);
        }
        self.save_layout();
        cx.notify();
    }

    fn on_terminal_event(
        &mut self,
        view: &Entity<TerminalView>,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalEvent::Exited => {
                if let Some(id) = self
                    .terminals
                    .iter()
                    .find(|(_, v)| v == view)
                    .map(|(id, _)| *id)
                {
                    self.close_terminal(id, window, cx);
                }
            }
            TerminalEvent::FocusEditor => self.focus_editor_now(window, cx),
            TerminalEvent::ContextMenu(position) => {
                self.terminal_context_menu(view.clone(), *position, cx)
            }
        }
    }

    /// The terminal's right-click menu, as batch 9 caught it: New Terminal,
    /// New Terminal With Profile..., Find, Copy (greyed without a
    /// selection), Paste, Select All, Clear, then Page Up, Page Down, Line
    /// Up and Line Down with their chords. Fleet's two AI rows and Find
    /// (its terminal search bar) are left out.
    fn terminal_context_menu(
        &mut self,
        view: Entity<TerminalView>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let has_selection = view.read(cx).has_selection();
        let mut entries: Vec<MenuEntry> = Vec::new();
        entries.extend(MenuEntry::action("terminal/new-tool", cx));
        entries.extend(MenuEntry::action("terminal/new-with-profile", cx));
        entries.push(MenuEntry::Separator);
        if let Some(mut copy) = MenuEntry::action("terminal/copy", cx) {
            if let MenuEntry::Item(item) = &mut copy {
                item.enabled = has_selection;
            }
            entries.push(copy);
        }
        entries.extend(MenuEntry::action("terminal/paste", cx));
        entries.extend(MenuEntry::action("select-all-terminal", cx));
        entries.extend(MenuEntry::action("terminal/clear", cx));
        entries.push(MenuEntry::Separator);
        let text = crate::keymap::chord_text;
        let scroll = |label: &str, chord: &str, key: &'static str, lines: bool| {
            let view = view.clone();
            let mut entry = MenuEntry::custom(label.to_string(), move |_, cx| {
                view.update(cx, |v, cx| v.scroll_by_key(key, lines, cx));
            });
            if let MenuEntry::Item(item) = &mut entry {
                item.shortcut = Some(chord.to_string().into());
            }
            entry
        };
        entries.push(scroll(
            "Page Up",
            &text("Shift+Page Up", "shift-pageup"),
            "pageup",
            false,
        ));
        entries.push(scroll(
            "Page Down",
            &text("Shift+Page Down", "shift-pagedown"),
            "pagedown",
            false,
        ));
        entries.push(scroll("Line Up", &text("Alt+Up", "alt-up"), "up", true));
        entries.push(scroll(
            "Line Down",
            &text("Alt+Down", "alt-down"),
            "down",
            true,
        ));
        let handle = view.read(cx).focus_handle(cx);
        self.open_menu(entries, position, Some(handle), cx);
    }

    /// Closes the tab of terminal `id` and drops its shell.
    pub(super) fn close_terminal(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for side in Side::ALL {
            if let Some(ix) = self
                .panel(side)
                .tools
                .iter()
                .position(|t| t.tool == Tool::Terminal && t.instance == id)
            {
                self.close_tool_tab(side, ix, window, cx);
                return;
            }
        }
        self.terminals.retain(|(tid, _)| *tid != id);
    }

    /// Removes a panel tab; a terminal's shell goes with it. The next tab
    /// takes its place, and the editor the focus if the tab had it.
    pub(super) fn close_tool_tab(
        &mut self,
        side: Side,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let had_focus = self.panel(side).focus_handle.contains_focused(window, cx);
        let panel = self.panel_mut(side);
        if ix >= panel.tools.len() {
            return;
        }
        let tab = panel.tools.remove(ix);
        if panel.active >= panel.tools.len() {
            panel.active = panel.tools.len().saturating_sub(1);
        } else if panel.active > ix {
            panel.active -= 1;
        }
        if tab.tool == Tool::Terminal {
            self.terminals.retain(|(id, _)| *id != tab.instance);
        }
        if tab.tool == Tool::Run {
            self.drop_run(tab.instance, cx);
        }
        // M8: the last Debug tab takes the ended debug sessions with it.
        if tab.tool == Tool::Debug {
            self.debug_tab_closed(cx);
        }
        if had_focus {
            if self.panel(side).tools.is_empty() {
                self.focus_editor_now(window, cx);
            } else {
                let active = self.panel(side).active;
                self.focus_tool_tab(side, active, window, cx);
            }
        }
        self.save_layout();
        cx.notify();
    }

    /// The panel tab's right-click menu (estimate): Close.
    pub(super) fn panel_tab_menu(
        &mut self,
        side: Side,
        ix: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity();
        let entries = vec![MenuEntry::custom("Close", move |window, cx| {
            entity.update(cx, |this, cx| this.close_tool_tab(side, ix, window, cx));
        })];
        self.open_menu(entries, position, None, cx);
    }

    /// `close-tab` while a terminal has the focus closes that terminal.
    pub(super) fn close_focused_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        for side in Side::ALL {
            let panel = self.panel(side);
            if !panel.open || !panel.focus_handle.contains_focused(window, cx) {
                continue;
            }
            if let Some(tab) = panel.tools.get(panel.active).copied() {
                if matches!(tab.tool, Tool::Terminal | Tool::Run) {
                    let ix = panel.active;
                    self.close_tool_tab(side, ix, window, cx);
                    return true;
                }
            }
        }
        false
    }

    /// `terminal/new-tool`: a new shell with the default profile.
    pub(super) fn terminal_new_tool(
        &mut self,
        _: &TerminalNewTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_terminal(None, true, window, cx);
    }

    /// `terminal/jump-to-tool`: shows the last terminal (a new one when there
    /// is none) and moves the focus there.
    pub(super) fn terminal_jump_to_tool(
        &mut self,
        _: &TerminalJumpToTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = self.panel(Side::Bottom);
        let ix = panel
            .tools
            .get(panel.active)
            .filter(|t| t.tool == Tool::Terminal)
            .map(|_| panel.active)
            .or_else(|| panel.tools.iter().rposition(|t| t.tool == Tool::Terminal));
        match ix {
            Some(ix) => {
                let panel = self.panel_mut(Side::Bottom);
                panel.active = ix;
                panel.open = true;
                self.chooser = None;
                self.focus_tool_tab(Side::Bottom, ix, window, cx);
                self.save_layout();
                cx.notify();
            }
            None => self.new_terminal(None, true, window, cx),
        }
    }

    /// `terminal/toggle`: hides the bottom panel when a terminal in it has
    /// the focus, else shows one and focuses it.
    pub(super) fn terminal_toggle(
        &mut self,
        _: &TerminalToggle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = self.panel(Side::Bottom);
        let showing = panel.open
            && panel
                .tools
                .get(panel.active)
                .is_some_and(|t| t.tool == Tool::Terminal)
            && panel.focus_handle.contains_focused(window, cx);
        if showing {
            self.panel_mut(Side::Bottom).open = false;
            self.focus_editor_now(window, cx);
            self.save_layout();
            cx.notify();
        } else {
            self.terminal_jump_to_tool(&TerminalJumpToTool, window, cx);
        }
    }

    /// `terminal/new-with-profile`: a popup listing the profiles (estimate: a
    /// menu under the bottom panel's tab row, or under the toolbar).
    pub(super) fn terminal_new_with_profile(
        &mut self,
        _: &TerminalNewWithProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profiles = Self::terminal_profiles(cx);
        if profiles.is_empty() {
            self.notify_error(
                "No shell found",
                "Add one to terminal.profiles in settings.json".to_string(),
                cx,
            );
            return;
        }
        let default = Self::settings(cx)
            .terminal_profile_default()
            .map(str::to_string);
        let entity = cx.entity();
        let entries: Vec<MenuEntry> = profiles
            .into_iter()
            .map(|profile| {
                let label = if default
                    .as_deref()
                    .is_some_and(|d| d.eq_ignore_ascii_case(&profile.name))
                {
                    format!("{} (default)", profile.name)
                } else {
                    profile.name.clone()
                };
                let entity = entity.clone();
                MenuEntry::custom(label, move |window, cx| {
                    let profile = profile.clone();
                    entity.update(cx, |this, cx| {
                        this.new_terminal(Some(profile), true, window, cx)
                    });
                })
            })
            .collect();
        let m = *cx.global::<Metrics>();
        let panel = self.panel(Side::Bottom);
        let position = if panel.open {
            let anchor = panel.plus_anchor.get();
            point(anchor.origin.x, anchor.origin.y + anchor.size.height)
        } else {
            let viewport = window.viewport_size();
            point(viewport.width / 2., px(m.toolbar_height))
        };
        self.open_menu(entries, position, None, cx);
    }

    /// `terminal/select-default-profile`: the Settings tool at its Terminal group.
    pub(super) fn terminal_select_default_profile(
        &mut self,
        _: &TerminalSelectDefaultProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(&OpenSettings, window, cx);
        for pane in &self.panes {
            for tab in &pane.tabs {
                if let TabView::Settings(view) = &tab.view {
                    view.update(cx, |view, cx| view.show_group("Terminal", cx));
                }
            }
        }
    }

    /// `terminal/edit-profiles`: `settings.json`, where `terminal.profiles` lives.
    pub(super) fn terminal_edit_profiles(
        &mut self,
        _: &TerminalEditProfiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_settings_file(&EditSettingsFile, window, cx);
    }
}
