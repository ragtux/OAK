//! Run configurations in the workspace (M6): `.fleet/run.json` read through
//! `weep-run`, the Run & Debug popup (`run-and-debug`), `run/run-in-context`
//! (the configuration at the caret in run.json, else the last run),
//! `run/rerun`, `stop-run-configurations` and `run/stop`, the gutter's run
//! icons, "Edit Run Configurations..." and `run/delete-configuration`; one
//! run tool tab per started configuration in the bottom panel. The popup's
//! Debug button and Alt+Enter hand a row to `debug.rs` (M8), whose session
//! shows its build and output in a run tab of its own (batch 13).

use super::*;
use crate::debugger::DebugSession;
use crate::menu::MenuEntry;
use crate::run_popup::{GutterRunRow, RunPopup, RunPopupEvent};
use crate::run_tool::{RunView, RunViewEvent};
use weep_run::{Command, RunConfig, RunFile};

impl Workspace {
    pub(super) fn run_file_path(&self) -> PathBuf {
        self.root.join(".fleet").join("run.json")
    }

    /// The configurations: the open document's text when run.json is open
    /// (unsaved edits count), else the file's. An absent file is an empty list.
    pub(super) fn run_configurations(&self, cx: &App) -> Result<Vec<RunConfig>, String> {
        let path = self.run_file_path();
        let open = self
            .documents
            .iter()
            .find(|d| d.read(cx).path.as_deref() == Some(path.as_path()))
            .map(|d| d.read(cx).buffer.text());
        let text = match open {
            Some(text) => text,
            None => match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(e) => return Err(e.to_string()),
            },
        };
        RunFile::parse(&text).map(|f| f.configurations)
    }

    pub(super) fn run_configurations_or_notify(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<RunConfig> {
        match self.run_configurations(cx) {
            Ok(configs) => configs,
            Err(e) => {
                self.notify_error("run.json could not be read", e, cx);
                Vec::new()
            }
        }
    }

    pub(super) fn run_view(&self, id: usize) -> Option<&Entity<RunView>> {
        self.runs
            .iter()
            .find(|(rid, _)| *rid == id)
            .map(|(_, view)| view)
    }

    /// The recent runs as indices into `configs`, most recent first.
    fn recent_run_indices(&self, configs: &[RunConfig], cx: &App) -> Vec<usize> {
        let limit = Self::settings(cx).run_recent_configurations_size();
        self.recent_runs
            .iter()
            .filter_map(|name| configs.iter().position(|c| &c.name == name))
            .take(limit)
            .collect()
    }

    pub(super) fn remember_run(&mut self, name: &str) {
        self.recent_runs.retain(|n| n != name);
        self.recent_runs.insert(0, name.to_string());
        self.recent_runs.truncate(20);
        self.save_layout();
    }

    /// Runs a configuration: its command in a run tab (the tab of an earlier
    /// run with the same name, when that is not running and the row does not
    /// allow parallel runs).
    pub(super) fn run_config(&mut self, config: &RunConfig, cx: &mut Context<Self>) {
        let env_file = Self::settings(cx)
            .run_environment_file()
            .map(|f| self.root.join(f));
        match config.command(&self.root, env_file.as_deref()) {
            Ok(command) => {
                self.remember_run(&config.name);
                self.start_run(
                    command,
                    config.name.clone(),
                    config.kind.clone(),
                    config.allow_parallel_run,
                    cx,
                );
            }
            Err(e) => self.notify_error("Cannot run", e, cx),
        }
    }

    fn start_run(
        &mut self,
        command: Command,
        name: String,
        kind: String,
        parallel: bool,
        cx: &mut Context<Self>,
    ) {
        if !parallel {
            if let Some((id, view)) = self.idle_run_tab(&name, cx) {
                view.update(cx, |v, cx| v.restart(Some(command), cx));
                self.show_run_tab(id, cx);
                return;
            }
        }
        let view = cx.new(|cx| RunView::new(name, kind, command, cx));
        self.add_run_tab(view, cx);
    }

    /// A run tab of this name whose process (or debug session) has ended.
    fn idle_run_tab(&self, name: &str, cx: &App) -> Option<(usize, Entity<RunView>)> {
        self.runs
            .iter()
            .find(|(_, v)| v.read(cx).name() == name && !v.read(cx).running(cx))
            .map(|(id, v)| (*id, v.clone()))
    }

    /// M8: a debug session's run tab (batch 13: Fleet shows the build and
    /// the adapter's command line in a bottom-panel tab named after the
    /// configuration), reusing an idle tab of the same name; shown and
    /// focused as a run's is.
    pub(super) fn start_debug_run_tab(
        &mut self,
        session: Entity<DebugSession>,
        name: String,
        kind: String,
        cx: &mut Context<Self>,
    ) {
        if let Some((id, view)) = self.idle_run_tab(&name, cx) {
            view.update(cx, |v, cx| v.set_debug(session, cx));
            self.show_run_tab(id, cx);
            return;
        }
        let view = cx.new(|cx| RunView::for_debug(name, kind, session, cx));
        self.add_run_tab(view, cx);
    }

    fn add_run_tab(&mut self, view: Entity<RunView>, cx: &mut Context<Self>) {
        cx.subscribe(&view, |this, _, event: &RunViewEvent, cx| match event {
            RunViewEvent::Exited(_) => cx.notify(),
            RunViewEvent::DebugRestart(session) => this.restart_debug_session(session.clone(), cx),
        })
        .detach();
        let id = self.next_run;
        self.next_run += 1;
        self.runs.push((id, view));
        let panel = self.panel_mut(Side::Bottom);
        panel.tools.push(ToolTab {
            tool: Tool::Run,
            instance: id,
        });
        panel.active = panel.tools.len() - 1;
        panel.open = true;
        self.chooser = None;
        self.show_run_tab(id, cx);
    }

    /// Shows run tab `id` in the bottom panel and gives it the focus (batch
    /// 11: the tab turns blue as the run starts).
    fn show_run_tab(&mut self, id: usize, cx: &mut Context<Self>) {
        let panel = self.panel_mut(Side::Bottom);
        if let Some(ix) = panel
            .tools
            .iter()
            .position(|t| t.tool == Tool::Run && t.instance == id)
        {
            panel.active = ix;
            panel.open = true;
            self.chooser = None;
            if let Some(view) = self.run_view(id) {
                let handle = view.read(cx).focus_handle(cx);
                self.focus_later(handle, cx);
            }
        }
        self.save_layout();
        cx.notify();
    }

    /// The run tab the bottom panel shows, when it is one.
    fn active_run(&self, cx: &App) -> Option<Entity<RunView>> {
        let panel = self.panel(Side::Bottom);
        let tab = panel.tools.get(panel.active)?;
        if tab.tool != Tool::Run {
            return None;
        }
        let _ = cx;
        self.run_view(tab.instance).cloned()
    }

    /// A gutter run icon was clicked: configuration `ix` of run.json.
    pub(super) fn run_configuration_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        let configs = self.run_configurations_or_notify(cx);
        if let Some(config) = configs.get(ix).cloned() {
            self.run_config(&config, cx);
        }
    }

    /// M8: a right click on a gutter run icon: a menu with Run and, where
    /// WEEP can debug the row, Debug on configuration `ix`
    /// (`run_popup::gutter_run_rows`); Debug starts the session as the
    /// popup's Debug does.
    pub(super) fn run_icon_menu(
        &mut self,
        editor: &Entity<EditorView>,
        ix: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let configs = self.run_configurations_or_notify(cx);
        let Some(config) = configs.get(ix) else {
            return;
        };
        let entity = cx.entity();
        let entries = crate::run_popup::gutter_run_rows(&config.kind)
            .into_iter()
            .map(|row| {
                let entity = entity.clone();
                let mut entry = MenuEntry::custom(row.label(&config.name), move |_, cx| {
                    entity.update(cx, |this, cx| match row {
                        GutterRunRow::Run => this.run_configuration_at(ix, cx),
                        GutterRunRow::Debug => this.debug_configuration_at(ix, cx),
                    });
                });
                if let MenuEntry::Item(item) = &mut entry {
                    item.shortcut = crate::keymap::hint_for(cx, row.chord_action()).map(Into::into);
                }
                entry
            })
            .collect();
        let focus = editor.read(cx).focus_handle(cx);
        self.open_menu(entries, position, Some(focus), cx);
    }

    // ---- actions -----------------------------------------------------------------

    /// `run-and-debug` (Ctrl+R): the Run & Debug popup.
    pub(super) fn run_and_debug(
        &mut self,
        _: &RunAndDebug,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_run_popup(window, cx);
    }

    pub(super) fn open_run_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(popup) = self.run_popup.clone() {
            popup.read(cx).focus_handle(cx).focus(window, cx);
            return;
        }
        // The configuration at run.json's caret first (batch 13: Fleet
        // listed "debug fixture" first with the caret on it).
        let context = self.run_configuration_at_caret(cx);
        let configs = self.run_configurations_or_notify(cx);
        let recent = self.recent_run_indices(&configs, cx);
        let name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let popup = cx.new(|cx| RunPopup::new(configs, recent, context, name, cx));
        cx.subscribe_in(&popup, window, Self::on_run_popup_event)
            .detach();
        popup.read(cx).focus_handle(cx).focus(window, cx);
        self.run_popup = Some(popup);
        cx.notify();
    }

    fn close_run_popup(&mut self, cx: &mut Context<Self>) {
        self.run_popup = None;
        self.focus_active_editor(cx);
        cx.notify();
    }

    fn on_run_popup_event(
        &mut self,
        popup: &Entity<RunPopup>,
        event: &RunPopupEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.run_popup.as_ref() != Some(popup) {
            return;
        }
        match event {
            RunPopupEvent::Run(ix) => {
                self.close_run_popup(cx);
                self.run_configuration_at(*ix, cx);
            }
            RunPopupEvent::Debug(ix) => {
                self.close_run_popup(cx);
                self.debug_configuration_at(*ix, cx);
            }
            RunPopupEvent::Command(text) => {
                self.close_run_popup(cx);
                let command = Command::shell(text, &self.root);
                self.start_run(command, text.clone(), "shell".into(), true, cx);
            }
            RunPopupEvent::Edit(ix) => {
                self.close_run_popup(cx);
                self.edit_run_configurations(*ix, cx);
            }
            RunPopupEvent::Delete(ix) => {
                self.delete_run_configuration(*ix, cx);
                let configs = self.run_configurations_or_notify(cx);
                let recent = self.recent_run_indices(&configs, cx);
                popup.update(cx, |p, cx| p.set_configs(configs, recent, cx));
            }
            RunPopupEvent::More(ix, at) => {
                let ix = *ix;
                let entity = cx.entity();
                let edit_entity = entity.clone();
                let entries = vec![
                    MenuEntry::custom("Edit Configuration", move |_, cx| {
                        edit_entity.update(cx, |this, cx| {
                            this.close_run_popup(cx);
                            this.edit_run_configurations(Some(ix), cx);
                        });
                    }),
                    MenuEntry::custom("Delete Configuration", move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.delete_run_configuration(ix, cx);
                            if let Some(popup) = this.run_popup.clone() {
                                let configs = this.run_configurations_or_notify(cx);
                                let recent = this.recent_run_indices(&configs, cx);
                                popup.update(cx, |p, cx| p.set_configs(configs, recent, cx));
                            }
                        });
                    }),
                ];
                let focus = popup.read(cx).focus_handle(cx);
                self.open_menu(entries, *at, Some(focus), cx);
            }
            RunPopupEvent::Cancel => self.close_run_popup(cx),
        }
    }

    /// "Edit Run Configurations...": `.fleet/run.json` in a tab, written
    /// with Fleet's template when there is none; the caret on a
    /// configuration's line when one is named.
    pub(super) fn edit_run_configurations(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        let path = self.run_file_path();
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, weep_run::config::TEMPLATE) {
                self.notify_error("Cannot create run.json", e.to_string(), cx);
                return;
            }
            self.files_changed(cx);
        }
        let line = ix.and_then(|ix| self.run_configurations(cx).ok()?.get(ix)?.line);
        self.open_path(path, false, cx);
        if let Some(line) = line {
            if let Some(editor) = self.active_editor().cloned() {
                editor.update(cx, |e, cx| e.goto(line, 0, cx));
            }
        }
        self.focus_active_editor(cx);
    }

    pub(super) fn run_edit_configurations(
        &mut self,
        _: &RunEditConfigurations,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_run_configurations(None, cx);
    }

    /// `run/delete-configuration`: the row goes from run.json, through the
    /// open document when there is one (one undo step), else on disk.
    fn delete_run_configuration(&mut self, ix: usize, cx: &mut Context<Self>) {
        let path = self.run_file_path();
        let open = self
            .documents
            .iter()
            .find(|d| d.read(cx).path.as_deref() == Some(path.as_path()))
            .cloned();
        let text = match &open {
            Some(doc) => doc.read(cx).buffer.text(),
            None => match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    self.notify_error("run.json could not be read", e.to_string(), cx);
                    return;
                }
            },
        };
        let file = match RunFile::parse(&text) {
            Ok(f) => f,
            Err(e) => {
                self.notify_error("run.json could not be read", e, cx);
                return;
            }
        };
        if ix >= file.configurations.len() {
            return;
        }
        let new_text = file.without(ix).to_text();
        match open {
            Some(doc) => doc.update(cx, |d, cx| d.edit(cx, |b| b.set_text(&new_text))),
            None => {
                if let Err(e) = std::fs::write(&path, new_text) {
                    self.notify_error("Cannot write run.json", e.to_string(), cx);
                }
            }
        }
    }

    /// `run/run-in-context` (Ctrl+Shift+R): in run.json the configuration at
    /// the caret; elsewhere the last run again; with nothing to run, the popup.
    pub(super) fn run_in_context(
        &mut self,
        _: &RunInContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.run_configuration_at_caret(cx) {
            let configs = self.run_configurations_or_notify(cx);
            if let Some(config) = configs.get(ix).cloned() {
                self.run_config(&config, cx);
                return;
            }
        }
        if let Some(name) = self.recent_runs.first().cloned() {
            let configs = self.run_configurations_or_notify(cx);
            if let Some(config) = configs.into_iter().find(|c| c.name == name) {
                self.run_config(&config, cx);
                return;
            }
        }
        self.open_run_popup(window, cx);
    }

    /// With run.json in the active editor, the configuration the caret is
    /// in (the last one starting at or above its row; the first when the
    /// caret is above them all). `run/run-in-context` and
    /// `debug/debug-in-context` share it.
    pub(super) fn run_configuration_at_caret(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let run_path = self.run_file_path();
        let editor = self.active_editor()?.clone();
        if editor.read(cx).path(cx) != Some(run_path) {
            return None;
        }
        let row = editor.update(cx, |e, cx| {
            let head = e.cursor.selection.head;
            e.document.read(cx).buffer.offset_to_point(head).row
        });
        let configs = self.run_configurations_or_notify(cx);
        configs
            .iter()
            .rposition(|c| c.line.is_some_and(|l| l <= row))
            .or(if configs.is_empty() { None } else { Some(0) })
    }

    /// `run/rerun` (Ctrl+Alt+R): the shown run tab's command again in the
    /// same tab (batch 11), else the last run.
    pub(super) fn rerun(&mut self, _: &Rerun, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.active_run(cx) {
            view.update(cx, |v, cx| v.restart(None, cx));
            cx.notify();
            return;
        }
        if let Some((id, view)) = self.runs.last().cloned() {
            view.update(cx, |v, cx| v.restart(None, cx));
            self.show_run_tab(id, cx);
            return;
        }
        self.run_in_context(&RunInContext, window, cx);
    }

    /// `stop-run-configurations`: every running process.
    pub(super) fn stop_run_configurations(
        &mut self,
        _: &StopRunConfigurations,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (_, view) in &self.runs {
            view.update(cx, |v, cx| v.stop(cx));
        }
        cx.notify();
    }

    /// `run/stop`: the shown run tab's process.
    pub(super) fn run_stop(&mut self, _: &RunStop, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.active_run(cx) {
            view.update(cx, |v, cx| v.stop(cx));
            cx.notify();
        }
    }

    /// Closing a run tab stops its process.
    pub(super) fn drop_run(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(view) = self.run_view(id).cloned() {
            view.update(cx, |v, cx| v.stop(cx));
        }
        self.runs.retain(|(rid, _)| *rid != id);
    }
}
