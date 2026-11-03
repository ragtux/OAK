//! Fleet's Run & Debug popup (`run-and-debug`, Ctrl+R; batch 11, twelfth
//! pass): the Goto popup's frame with a "Run & Debug" title, a field
//! ("Run command or configuration"), a Recent section while something has
//! run, the workspace's configurations under its name with a Run button
//! and a "..." on the selected row, and "Edit Run Configurations..." under
//! a separator. Enter runs the selected row, or the typed text as a shell
//! command when it matches no configuration.

use crate::actions::{PopupConfirm, PopupDown, PopupUp, RunDeleteConfiguration};
use crate::assets::icon;
use crate::input::{InputEvent, TextInput};
use crate::search_tool::frame;
use crate::theme::{Metrics, Theme};
use gpui::{
    deferred, div, point, prelude::*, px, App, BoxShadow, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, Pixels, Point, Render, SharedString, Window,
};
use oak_run::RunConfig;

#[derive(Clone, Debug, PartialEq)]
pub enum RunPopupEvent {
    /// Run configuration `index` (into the list the popup was given).
    Run(usize),
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

/// The tab's and the row's icon for a configuration type (estimates: Fleet
/// draws its own per type).
pub fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "command" | "shell" => "oak/console",
        "cargo" => "oak/file-rust",
        _ => "expui/run/run",
    }
}

impl RunPopup {
    pub fn new(
        configs: Vec<RunConfig>,
        recent: Vec<usize>,
        workspace: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new("Run command or configuration", cx));
        cx.subscribe(&input, Self::on_input).detach();
        let mut popup = RunPopup {
            input,
            configs,
            recent,
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
        let configs: Vec<usize> = (0..self.configs.len())
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
                let more_entity = entity.clone();
                let hover_entity = entity.clone();
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
                                d.child(
                                    div()
                                        .id("run-row-run")
                                        .w(px(m.run_popup_button_width))
                                        .h(px(m.run_popup_button_height))
                                        .mr(px(m.run_popup_button_gap - m.popup_row_icon_gap))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(m.button_radius))
                                        .bg(theme.button_secondary_bg)
                                        .hover(|s| s.bg(theme.button_secondary_bg_hovered))
                                        .text_color(theme.button_secondary_text)
                                        .child("Run")
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            run_entity.update(cx, |_, cx| {
                                                cx.emit(RunPopupEvent::Run(config_ix))
                                            })
                                        }),
                                )
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
                .key_context("Popup")
                .track_focus(&self.focus_handle)
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
                        frame(field_focused, &theme, &m)
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
    }
}
