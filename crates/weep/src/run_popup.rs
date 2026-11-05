//! Fleet's Run & Debug popup (`run-and-debug`, Ctrl+R; batch 11, twelfth
//! pass): the Goto popup's frame with a "Run & Debug" title, a field
//! ("Run command or configuration"), a Recent section while something has
//! run, the workspace's configurations under its name with a Run button
//! and a "..." on the selected row, and "Edit Run Configurations..." under
//! a separator. Enter runs the selected row, or the typed text as a shell
//! command when it matches no configuration.
//!
//! M8: Fleet lists a configuration once and offers Run and Debug on it
//! (batch 5 caught a row with both buttons; batch 11's `command` rows had
//! Run only). Batch 13 (macOS): the selected `cargo` row shows "Run ↩" and
//! "Debug ⌥↩", the chord inside each button in `button.secondary.text.hint`;
//! Alt+Enter debugs the selected row of any type WEEP can debug (`cargo`,
//! `command`). With run.json's caret in a configuration, that one leads
//! the workspace's list (batch 13: "debug fixture" first, the caret on it,
//! no Recent section).

use crate::actions::{PopupConfirm, PopupDown, PopupUp, RunDeleteConfiguration, RunPopupDebug};
use crate::assets::icon;
use crate::input::{InputEvent, TextInput};
use crate::list_popup::{tui_cols, tui_input_row, tui_row, tui_top, TuiRow};
use crate::search_tool::frame;
use crate::skin::{self, Cell, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    deferred, div, point, prelude::*, px, App, BoxShadow, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, KeyBinding, Pixels, Point, Render, SharedString, Window,
};
use weep_run::RunConfig;

#[derive(Clone, Debug, PartialEq)]
pub enum RunPopupEvent {
    /// Run configuration `index` (into the list the popup was given).
    Run(usize),
    /// Debug configuration `index` (M8).
    Debug(usize),
    /// Run the typed text through the shell.
    Command(String),
    /// Open `.fleet/run.json`, at a configuration's line when given.
    Edit(Option<usize>),
    /// `run/delete-configuration` on a configuration.
    Delete(usize),
    /// The "..." button on a configuration, at this window position.
    More(usize, Point<Pixels>),
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Header(SharedString),
    Recent(usize),
    Config(usize),
    Separator,
    Edit,
}

impl Row {
    fn selectable(&self) -> bool {
        matches!(self, Row::Recent(_) | Row::Config(_) | Row::Edit)
    }
}

pub struct RunPopup {
    input: Entity<TextInput>,
    configs: Vec<RunConfig>,
    /// Recent runs, most recent first, as indices into `configs`.
    recent: Vec<usize>,
    /// The configuration at run.json's caret, listed first.
    context: Option<usize>,
    workspace: SharedString,
    rows: Vec<Row>,
    selected: usize,
    focus_handle: FocusHandle,
}

impl EventEmitter<RunPopupEvent> for RunPopup {}

impl Focusable for RunPopup {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.input.read(_cx).focus_handle(_cx)
    }
}

/// The popup's own key: Alt+Enter debugs the selected row (batch 13). In
/// its own context, so the workspace's Alt+Enter (`replace-one`) is not
/// reached from the popup.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "alt-enter",
        RunPopupDebug,
        Some("RunPopup"),
    )]);
}

/// The chords the Run and Debug buttons show: "↩" and "⌥↩" on macOS
/// (batch 13), the platform's spelling elsewhere (estimate).
fn button_chords() -> (String, String) {
    (
        crate::keymap::presentable("enter"),
        crate::keymap::presentable("alt-enter"),
    )
}

/// The workspace section's order: the configuration in context first, the
/// rest as run.json lists them.
fn config_order(count: usize, context: Option<usize>) -> Vec<usize> {
    let first = context.filter(|c| *c < count);
    first
        .into_iter()
        .chain((0..count).filter(|ix| Some(*ix) != first))
        .collect()
}

/// Whether WEEP can debug a configuration type (M8): a `cargo` row through
/// its build, a `command` row's program directly.
pub fn debuggable(kind: &str) -> bool {
    matches!(kind, "cargo" | "command")
}

/// Whether the selected row shows the "Debug" button (estimate: rows Fleet
/// debugs, `cargo` here; batch 11 saw `command` rows with Run alone).
fn debug_button(kind: &str) -> bool {
    kind == "cargo"
}

/// A row of the menu a right click on a gutter run icon opens (M8,
/// estimate: Fleet's click on the icon is unprobed; the rows after
/// IntelliJ's gutter menu, "Run 'name'" and "Debug 'name'", with the
/// in-context actions' chords, which act on the configuration at
/// run.json's caret, where the click puts it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterRunRow {
    Run,
    Debug,
}

impl GutterRunRow {
    pub fn label(self, name: &str) -> String {
        let verb = match self {
            GutterRunRow::Run => "Run",
            GutterRunRow::Debug => "Debug",
        };
        if name.is_empty() {
            verb.to_string()
        } else {
            format!("{verb} '{name}'")
        }
    }

    /// The Fleet action whose chord the row shows.
    pub fn chord_action(self) -> &'static str {
        match self {
            GutterRunRow::Run => "run/run-in-context",
            GutterRunRow::Debug => "debug/debug-in-context",
        }
    }
}

/// The run icon's menu for a configuration of type `kind`: Run, and Debug
/// where WEEP can debug the row (as the popup's Alt+Enter decides).
pub fn gutter_run_rows(kind: &str) -> Vec<GutterRunRow> {
    let mut rows = vec![GutterRunRow::Run];
    if debuggable(kind) {
        rows.push(GutterRunRow::Debug);
    }
    rows
}

/// The tab's and the row's icon for a configuration type (estimates: Fleet
/// draws its own per type).
pub fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "command" | "shell" => "weep/console",
        "cargo" => "weep/file-rust",
        _ => "expui/run/run",
    }
}

impl RunPopup {
    pub fn new(
        configs: Vec<RunConfig>,
        recent: Vec<usize>,
        context: Option<usize>,
        workspace: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Run command or configuration", cx);
            // The popup draws its own prompt cell in the tui skin.
            input.tui_prompt = false;
            input
        });
        cx.subscribe(&input, Self::on_input).detach();
        let mut popup = RunPopup {
            input,
            configs,
            recent,
            context,
            workspace: workspace.into(),
            rows: Vec::new(),
            selected: 0,
            focus_handle: cx.focus_handle(),
        };
        popup.refilter(cx);
        popup
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    /// Replaces the configurations (after a delete or an edit) and keeps
    /// the selection where it can.
    pub fn set_configs(
        &mut self,
        configs: Vec<RunConfig>,
        recent: Vec<usize>,
        cx: &mut Context<Self>,
    ) {
        self.configs = configs;
        self.recent = recent;
        self.refilter(cx);
    }

    fn on_input(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => self.refilter(cx),
            // Shift+Enter is no debug gesture (batch 13: Alt+Enter is).
            InputEvent::Submit { .. } => self.confirm_selected(cx),
            InputEvent::Cancel => cx.emit(RunPopupEvent::Cancel),
            _ => {}
        }
    }

    fn matches(&self, query: &str, ix: usize) -> bool {
        query.is_empty()
            || self.configs[ix]
                .name
                .to_lowercase()
                .contains(&query.to_lowercase())
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let mut rows = Vec::new();
        let recent: Vec<usize> = self
            .recent
            .iter()
            .copied()
            .filter(|ix| *ix < self.configs.len() && self.matches(&query, *ix))
            .collect();
        if !recent.is_empty() && query.is_empty() {
            rows.push(Row::Header("Recent".into()));
            rows.extend(recent.iter().map(|ix| Row::Recent(*ix)));
            rows.push(Row::Separator);
        }
        let configs: Vec<usize> = config_order(self.configs.len(), self.context)
            .into_iter()
            .filter(|ix| self.matches(&query, *ix))
            .collect();
        if !configs.is_empty() {
            rows.push(Row::Header(self.workspace.clone()));
            rows.extend(configs.iter().map(|ix| Row::Config(*ix)));
        }
        if !rows.is_empty() {
            rows.push(Row::Separator);
        }
        rows.push(Row::Edit);
        self.rows = rows;
        // The first row is selected: a Recent one when there is any (batch
        // 11: Down moved from the recent "fail" to "echo hello").
        self.selected = self.rows.iter().position(Row::selectable).unwrap_or(0);
        cx.notify();
    }

    fn selectable_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selectable())
            .map(|(i, _)| i)
            .collect()
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let indices = self.selectable_indices();
        if indices.is_empty() {
            return;
        }
        let at = indices
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0) as isize;
        let next = (at + delta).rem_euclid(indices.len() as isize) as usize;
        self.selected = indices[next];
        cx.notify();
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    fn confirm(&mut self, _: &PopupConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_selected(cx);
    }

    fn delete(&mut self, _: &RunDeleteConfiguration, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected_config() {
            cx.emit(RunPopupEvent::Delete(ix));
        }
    }

    fn selected_config(&self) -> Option<usize> {
        match self.rows.get(self.selected) {
            Some(Row::Recent(ix)) | Some(Row::Config(ix)) => Some(*ix),
            _ => None,
        }
    }

    fn debug_action(&mut self, _: &RunPopupDebug, _: &mut Window, cx: &mut Context<Self>) {
        self.debug_selected(cx);
    }

    /// Alt+Enter: the selected row under the debugger.
    fn debug_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected_config() {
            if debuggable(&self.configs[ix].kind) {
                cx.emit(RunPopupEvent::Debug(ix));
            }
        }
    }

    fn confirm_selected(&mut self, cx: &mut Context<Self>) {
        match self.rows.get(self.selected) {
            Some(Row::Recent(ix)) | Some(Row::Config(ix)) => cx.emit(RunPopupEvent::Run(*ix)),
            Some(Row::Edit) => cx.emit(RunPopupEvent::Edit(None)),
            _ => {
                let query = self.query(cx);
                if !query.trim().is_empty() {
                    cx.emit(RunPopupEvent::Command(query.trim().to_string()));
                }
            }
        }
    }

    /// A selected row's button: the label, then its chord in the hint
    /// colour (batch 13: 20 tall, 4 in on either side, one space between).
    fn row_button(
        id: &'static str,
        label: &'static str,
        chord: String,
        theme: &Theme,
        m: &Metrics,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex_shrink_0()
            .h(px(m.run_popup_button_height))
            .px(px(m.run_popup_button_pad_x))
            .flex()
            .items_center()
            .gap(px(m.run_popup_button_chord_gap))
            .rounded(px(m.button_radius))
            .bg(theme.button_secondary_bg)
            .hover(|s| s.bg(theme.button_secondary_bg_hovered))
            .text_color(theme.button_secondary_text)
            .child(label)
            .child(
                div()
                    .text_color(theme.button_secondary_text_hint)
                    .child(chord),
            )
    }

    fn render_row(
        &self,
        ix: usize,
        row: &Row,
        theme: &Theme,
        m: &Metrics,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let entity = cx.entity();
        match row {
            Row::Header(text) => div()
                .h(px(m.goto_header_height))
                .px(px(m.popup_row_pad_x))
                .flex_shrink_0()
                .flex()
                .items_center()
                .text_color(theme.text_secondary)
                .child(text.clone())
                .into_any_element(),
            Row::Separator => div()
                .mt(px(m.run_popup_separator_above))
                .mb(px(m.run_popup_separator_below))
                .h(px(1.))
                .flex_shrink_0()
                .bg(theme.popup_separator)
                .into_any_element(),
            Row::Recent(config) | Row::Config(config) => {
                let config_ix = *config;
                let recent = matches!(row, Row::Recent(_));
                let selected = ix == self.selected;
                let c = &self.configs[config_ix];
                let name: SharedString = c.name.clone().into();
                let icon_path = kind_icon(&c.kind);
                let text_color = if selected {
                    theme.list_text_focused
                } else {
                    theme.list_text
                };
                let run_entity = entity.clone();
                let debug_entity = entity.clone();
                let more_entity = entity.clone();
                let hover_entity = entity.clone();
                let with_debug = debug_button(&c.kind);
                div()
                    .h(px(m.goto_row_pitch))
                    .flex_shrink_0()
                    .child(
                        div()
                            .id(gpui::ElementId::NamedInteger("run-row".into(), ix as u64))
                            .h(px(m.goto_row_height))
                            .pl(px(m.popup_row_pad_x))
                            .flex()
                            .items_center()
                            .gap(px(m.popup_row_icon_gap))
                            .rounded(px(m.popup_row_radius))
                            .cursor_pointer()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_color(text_color)
                            .when(selected, |d| d.bg(theme.list_bg_focused))
                            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                            .on_mouse_move(move |_, _, cx| {
                                hover_entity.update(cx, |this, cx| {
                                    if this.selected != ix {
                                        this.selected = ix;
                                        cx.notify();
                                    }
                                })
                            })
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.selected = ix;
                                    cx.emit(RunPopupEvent::Run(config_ix));
                                })
                            })
                            .child(icon(icon_path, m.icon_size, theme.icon_secondary))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .when(recent, |d| d.italic())
                                    .child(name),
                            )
                            .child(div().flex_1())
                            .when(selected, |d| {
                                // Batch 13: "Run ↩" then, 4 on, "Debug ⌥↩";
                                // the last one where Run alone ended.
                                let (run_chord, debug_chord) = button_chords();
                                let after = m.run_popup_button_gap - m.popup_row_icon_gap;
                                d.child(
                                    Self::row_button("run-row-run", "Run", run_chord, theme, m)
                                        .mr(px(if with_debug {
                                            m.run_popup_buttons_gap - m.popup_row_icon_gap
                                        } else {
                                            after
                                        }))
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            run_entity.update(cx, |_, cx| {
                                                cx.emit(RunPopupEvent::Run(config_ix))
                                            })
                                        }),
                                )
                                .when(with_debug, |d| {
                                    d.child(
                                        Self::row_button(
                                            "run-row-debug",
                                            "Debug",
                                            debug_chord,
                                            theme,
                                            m,
                                        )
                                        .mr(px(after))
                                        .on_click(
                                            move |_, _, cx| {
                                                cx.stop_propagation();
                                                debug_entity.update(cx, |_, cx| {
                                                    cx.emit(RunPopupEvent::Debug(config_ix))
                                                })
                                            },
                                        ),
                                    )
                                })
                                .child(
                                    div()
                                        .id("run-row-more")
                                        .w(px(m.run_popup_more_width))
                                        .h(px(m.run_popup_button_height))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(m.button_radius))
                                        .hover(|s| s.bg(theme.ghost_hovered))
                                        .child(icon(
                                            "expui/general/moreHorizontal",
                                            m.icon_size,
                                            theme.icon_primary,
                                        ))
                                        .on_click(move |event, _, cx| {
                                            cx.stop_propagation();
                                            let at = event.position();
                                            more_entity.update(cx, |_, cx| {
                                                cx.emit(RunPopupEvent::More(config_ix, at))
                                            })
                                        }),
                                )
                            }),
                    )
                    .into_any_element()
            }
            Row::Edit => {
                let selected = ix == self.selected;
                let text_color = if selected {
                    theme.list_text_focused
                } else {
                    theme.list_text
                };
                div()
                    .h(px(m.goto_row_pitch))
                    .mt(px(m.run_popup_edit_above - m.run_popup_separator_below))
                    .flex_shrink_0()
                    .child(
                        div()
                            .id("run-row-edit")
                            .h(px(m.goto_row_height))
                            .px(px(m.popup_row_pad_x))
                            .flex()
                            .items_center()
                            .gap(px(m.popup_row_icon_gap))
                            .rounded(px(m.popup_row_radius))
                            .cursor_pointer()
                            .text_color(text_color)
                            .when(selected, |d| d.bg(theme.list_bg_focused))
                            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |_, cx| cx.emit(RunPopupEvent::Edit(None)))
                            })
                            .child(icon(
                                "expui/general/edit",
                                m.icon_size,
                                theme.icon_secondary,
                            ))
                            .child("Edit Run Configurations..."),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for RunPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(window, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let viewport = window.viewport_size();
        let margin = px(m.island_margin);
        let width = px(m.goto_width).min(viewport.width - margin * 2.);
        let left = (viewport.width - width) / 2.;
        let field_focused = self.input.read(cx).is_focused(window);
        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| self.render_row(ix, row, &theme, &m, cx))
            .collect();
        let close = cx.entity();
        deferred(
            div()
                .id("run-popup")
                .key_context("Popup RunPopup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::debug_action))
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::delete))
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |_, cx| cx.emit(RunPopupEvent::Cancel));
                })
                .occlude()
                .absolute()
                .left(left)
                .top(px(m.goto_top))
                .w(width)
                .flex()
                .flex_col()
                .rounded(px(m.popup_radius))
                .border_1()
                .border_color(theme.popup_border)
                .bg(theme.popup_goto_bg)
                .font_family(theme.ui_font.clone())
                .text_size(px(m.ui_font_size))
                .text_color(theme.popup_text)
                .overflow_hidden()
                .shadow(vec![BoxShadow {
                    color: theme.shadow_medium,
                    offset: point(px(0.), px(m.popup_shadow_offset_y)),
                    blur_radius: px(m.popup_shadow_blur),
                    spread_radius: px(0.),
                }])
                .child(
                    div()
                        .h(px(m.run_popup_title_height))
                        .pb(px(m.run_popup_title_lift))
                        .px(px(m.goto_pad + m.popup_row_pad_x - 1.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Run & Debug"),
                )
                .child(
                    div().px(px(m.goto_pad - 1.)).flex_shrink_0().flex().child(
                        frame(field_focused, window, &theme, &m)
                            .pl(px(m.goto_pad))
                            .child(div().flex_1().min_w_0().h_full().child(self.input.clone())),
                    ),
                )
                .child(
                    div()
                        .mt(px(m.run_popup_header_gap))
                        .pl(px(m.goto_pad - 1.))
                        .pr(px(m.run_popup_rows_pad_right))
                        .pb(px(m.run_popup_bottom_pad))
                        .flex()
                        .flex_col()
                        .children(rows),
                ),
        )
        .into_any_element()
    }
}

// ---- the tui skin ---------------------------------------------------------------

/// A line of the tui popup under the field: a rule joining the frame (with
/// a section's name set in) or a row.
enum TuiLine {
    Rule(Option<SharedString>),
    Row(usize),
}

impl RunPopup {
    /// A configuration's one-cell mark: `$` a shell command, the Rust
    /// glyph for cargo, `▶` the rest.
    fn tui_glyph(kind: &str, theme: &Theme) -> (&'static str, gpui::Hsla) {
        match kind {
            "command" | "shell" => ("$", theme.text_secondary),
            "cargo" => skin::file_glyph("a.rs", theme),
            _ => ("▶", theme.text_positive),
        }
    }

    fn render_tui(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let bg = theme.popup_goto_bg;
        let cols = tui_cols(m.goto_width, cell, window);
        let inner = cols - 2;
        let mut lines: Vec<TuiLine> = Vec::new();
        for (ix, row) in self.rows.iter().enumerate() {
            match row {
                Row::Header(t) => match lines.last_mut() {
                    Some(TuiLine::Rule(title @ None)) => *title = Some(t.clone()),
                    _ => lines.push(TuiLine::Rule(Some(t.clone()))),
                },
                Row::Separator => {
                    if !matches!(lines.last(), Some(TuiLine::Rule(_))) {
                        lines.push(TuiLine::Rule(None));
                    }
                }
                _ => {
                    if lines.is_empty() {
                        lines.push(TuiLine::Rule(None));
                    }
                    lines.push(TuiLine::Row(ix));
                }
            }
        }
        let rows = 3 + lines.len();
        let mut rules = Vec::new();
        let mut titles = Vec::new();
        let mut body_rows: Vec<gpui::AnyElement> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let box_row = 2 + i;
            match line {
                TuiLine::Rule(title) => {
                    rules.push(box_row);
                    if let Some(t) = title {
                        titles.push((box_row, t.clone()));
                    }
                    body_rows.push(div().h(px(cell.h)).flex_shrink_0().into_any_element());
                }
                TuiLine::Row(ix) => {
                    body_rows.push(self.render_tui_row(*ix, inner, &theme, cell, cx))
                }
            }
        }
        let field_focused = self.input.read(cx).is_focused(window);
        let body = div()
            .size_full()
            .flex()
            .flex_col()
            .child(tui_input_row(
                "›",
                field_focused,
                self.input.clone(),
                Vec::new(),
                &theme,
                cell,
            ))
            .children(body_rows)
            .into_any_element();
        let close = cx.entity();
        let chords = button_chords();
        let mut boxed = skin::boxed(
            cell,
            cols,
            rows,
            vec![skin::title("Run & Debug", theme.text_primary)],
            rules,
            body,
            bg,
            theme.border_focused,
            &theme,
            m,
        )
        .occlude()
        .child(skin::edge_strip(
            cell,
            rows - 1,
            2,
            cols.saturating_sub(4),
            skin::hints(
                &[
                    (chords.0.as_str(), "run"),
                    (chords.1.as_str(), "debug"),
                    ("esc", "close"),
                ],
                &theme,
            ),
            bg,
        ));
        for (row, t) in titles {
            boxed = boxed.child(skin::edge_strip(
                cell,
                row,
                2,
                cols.saturating_sub(4),
                vec![skin::title(t, theme.text_tertiary)],
                bg,
            ));
        }
        deferred(
            div()
                .id("run-popup")
                .key_context("Popup RunPopup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::debug_action))
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::delete))
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |_, cx| cx.emit(RunPopupEvent::Cancel));
                })
                .absolute()
                .left(skin::centred_x(window.viewport_size().width, cols, cell))
                .top(cell.rows(tui_top(cell, &m)))
                .child(boxed),
        )
        .into_any_element()
    }

    /// A configuration's row: its mark and name (recent ones italic), and on
    /// the selected row `[ Run ] ⋯` at the right; or the edit row.
    fn render_tui_row(
        &self,
        ix: usize,
        cols: usize,
        theme: &Theme,
        cell: Cell,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let selected = ix == self.selected;
        let entity = cx.entity();
        let hover = entity.clone();
        match &self.rows[ix] {
            Row::Recent(config) | Row::Config(config) => {
                let config_ix = *config;
                let c = &self.configs[config_ix];
                let mut row = TuiRow::text(skin::fit_text(&c.name, cols.saturating_sub(32)));
                row.glyph = Some(Self::tui_glyph(&c.kind, theme));
                row.italic = matches!(self.rows[ix], Row::Recent(_));
                let run = entity.clone();
                let debug = entity.clone();
                let more = entity.clone();
                let with_debug = debug_button(&c.kind);
                tui_row(
                    gpui::ElementId::NamedInteger("run-row".into(), ix as u64),
                    row,
                    cols,
                    selected,
                    cell,
                    theme,
                )
                .on_mouse_move(move |_, _, cx| {
                    hover.update(cx, |this, cx| {
                        if this.selected != ix {
                            this.selected = ix;
                            cx.notify();
                        }
                    })
                })
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.selected = ix;
                        cx.emit(RunPopupEvent::Run(config_ix));
                    })
                })
                .when(selected, |d| {
                    // `[ Run ↩ ] [ Debug ⌥↩ ] ⋯` as Fleet's row (batch 13).
                    let (run_chord, debug_chord) = button_chords();
                    let run_label = format!("Run {run_chord}");
                    let debug_label = format!("Debug {debug_chord}");
                    d.child(
                        skin::button("run-row-run", &run_label, false, theme, cell).on_click(
                            move |_, _, cx| {
                                cx.stop_propagation();
                                run.update(cx, |_, cx| cx.emit(RunPopupEvent::Run(config_ix)))
                            },
                        ),
                    )
                    .child(div().w(cell.cols(1)).flex_shrink_0())
                    .when(with_debug, |d| {
                        d.child(
                            skin::button("run-row-debug", &debug_label, false, theme, cell)
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    debug.update(cx, |_, cx| {
                                        cx.emit(RunPopupEvent::Debug(config_ix))
                                    })
                                }),
                        )
                        .child(div().w(cell.cols(1)).flex_shrink_0())
                    })
                    .child(
                        div()
                            .id("run-row-more")
                            .w(cell.cols(1))
                            .flex_shrink_0()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.ghost_hovered))
                            .child("⋯")
                            .on_click(move |event, _, cx| {
                                cx.stop_propagation();
                                let at = event.position();
                                more.update(cx, |_, cx| cx.emit(RunPopupEvent::More(config_ix, at)))
                            }),
                    )
                    .child(div().w(cell.cols(1)).flex_shrink_0())
                })
                .into_any_element()
            }
            _ => {
                let mut row = TuiRow::text("Edit Run Configurations...");
                row.glyph = Some(("≡", theme.text_secondary));
                tui_row("run-row-edit", row, cols, selected, cell, theme)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |_, cx| cx.emit(RunPopupEvent::Edit(None)))
                    })
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_in_context_leads() {
        // Batch 13: run.json lists echo hello, fail, run fixture, debug
        // fixture; the caret on the last, Fleet listed it first.
        assert_eq!(config_order(4, Some(3)), vec![3, 0, 1, 2]);
        assert_eq!(config_order(4, None), vec![0, 1, 2, 3]);
        assert_eq!(config_order(2, Some(7)), vec![0, 1]);
    }

    #[test]
    fn debug_buttons_and_gesture_targets() {
        assert!(debug_button("cargo") && !debug_button("command"));
        assert!(debuggable("command") && !debuggable("docker"));
    }

    #[test]
    fn the_run_icon_menu() {
        use GutterRunRow::*;
        assert_eq!(gutter_run_rows("cargo"), vec![Run, Debug]);
        assert_eq!(gutter_run_rows("command"), vec![Run, Debug]);
        assert_eq!(gutter_run_rows("gradle"), vec![Run]);
        assert_eq!(Run.label("debug fixture"), "Run 'debug fixture'");
        assert_eq!(Debug.label("debug fixture"), "Debug 'debug fixture'");
        assert_eq!(Debug.label(""), "Debug");
        assert_eq!(Run.chord_action(), "run/run-in-context");
        assert_eq!(Debug.chord_action(), "debug/debug-in-context");
    }
}
