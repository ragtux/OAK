//! Fleet's floating debug toolbar (M8, `editor.showFloatingDebugPanel`,
//! "Show floating debug toolbar", off by default): while the active debug
//! session is not over, a strip at the top right of the active editor with
//! Fleet's eight buttons, the Debug tool's Processes row: Resume, Pause,
//! Restart, Stop, Step Over, Step Into, Step Out and a last one, each
//! dispatching its `debug/*` action (the workspace routes them to the
//! session). It has no grip: Fleet's does not move.
//!
//! The panel is the workspace's one debugger overlay, drawn over the whole
//! window: it also hosts the quick evaluate popup (`editor/debug.rs`) and
//! tracks the pointer for the value on hover while the debuggee is stopped,
//! and it hosts the attach dialog (`attach_popup.rs`).
//!
//! Batch 13 measured it (`theme.rs`, `DEBUG_PANEL_*`): 224 x 28 on the
//! editor area's top, 12 in from the island's right edge, `popup.background`
//! in a 1 px `popup.border`, no shadow; 20 px buttons 8 apart, the hovered
//! one `ghostButton.off.background.hovered`; Resume in
//! `icon.background.green`, Stop in `icon.background.red`, the rest in
//! `icon.primary`, a button that cannot act now in `text.disabled` (Pause
//! while stopped). The tooltip is the pointer's, "Step Into ⌘;". The status
//! badge moves under the panel ([`status_top_under_panel`]).
//!
//! The last button (a box with a short bar at its bottom left) was not
//! hovered in batch 13, so what it does is unknown (estimate: it shows the
//! session's output or console, the Processes row's layout toggle); it is
//! drawn as Fleet draws it and does nothing.

use crate::actions::{
    DebugPause, DebugRestart, DebugResume, DebugStepInto, DebugStepOut, DebugStepOver, DebugStop,
};
use crate::assets::icon;
use crate::attach_popup::{AttachPopup, AttachPopupEvent, OnAttach};
use crate::debugger::{DebugSession, Debugger, SessionState};
use crate::editor::debug::{stopped_session, QuickEvaluate, QuickEvaluateEvent};
use crate::editor::EditorView;
use crate::keymap::{action_title, hint_for};
use crate::settings::AppSettings;
use crate::skin::{self, Cell, Skin};
use crate::theme::{Metrics, Theme};
use crate::tooltip::Tooltip;
use gpui::{
    canvas, div, point, prelude::*, px, Action, AnyElement, App, Bounds, Context, DispatchPhase,
    Entity, EntityId, Focusable, Global, MouseMoveEvent, Pixels, Point, Render, SharedString,
    Subscription, Task, WeakEntity, Window,
};
use std::rc::Rc;
use std::time::Duration;

/// How long a value popup the pointer opened outlives the pointer leaving
/// its expression (estimate: long enough to cross into the popup).
const HOVER_CLOSE_GRACE_MS: u64 = 300;

/// One of the panel's buttons, in Fleet's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Resume,
    Pause,
    Restart,
    Stop,
    StepOver,
    StepInto,
    StepOut,
    /// The last button, unprobed: drawn, inert.
    Output,
}

impl Button {
    const ALL: [Button; 8] = [
        Button::Resume,
        Button::Pause,
        Button::Restart,
        Button::Stop,
        Button::StepOver,
        Button::StepInto,
        Button::StepOut,
        Button::Output,
    ];

    fn action_id(self) -> &'static str {
        match self {
            Button::Resume => "debug/resume",
            Button::Pause => "debug/pause",
            // Fleet's action list has no restart id: an element id only
            // (`DebugRestart` is in no keymap table, so no chord).
            Button::Restart => "debug/restart",
            Button::Stop => "debug/stop",
            Button::StepOver => "debug/step-over",
            Button::StepInto => "debug/step-into",
            Button::StepOut => "debug/step-out",
            Button::Output => "debug/output",
        }
    }

    /// The tooltip's title: the action's (Fleet's tooltip is "Step Into ⌘;").
    fn title(self) -> String {
        match self {
            // No action id to title it by (estimate: the button's name).
            Button::Restart => "Restart".into(),
            _ => action_title(self.action_id()),
        }
    }

    /// The action to dispatch; `None` for a button that does nothing yet.
    fn action(self) -> Option<Box<dyn Action>> {
        Some(match self {
            Button::Resume => Box::new(DebugResume),
            Button::Pause => Box::new(DebugPause),
            Button::Restart => Box::new(DebugRestart),
            Button::Stop => Box::new(DebugStop),
            Button::StepOver => Box::new(DebugStepOver),
            Button::StepInto => Box::new(DebugStepInto),
            Button::StepOut => Box::new(DebugStepOut),
            Button::Output => return None,
        })
    }

    /// JetBrains' expui run icons where they match Fleet's drawing, else
    /// WEEP's own (Fleet's pause is two bars, its restart turns left, its
    /// step over is a bare arc, its steps into and out bare arrows).
    fn icon(self) -> &'static str {
        match self {
            Button::Resume => "expui/run/resume",
            Button::Pause => "weep/debug-pause",
            Button::Restart => "weep/debug-restart",
            Button::Stop => "expui/run/stop",
            Button::StepOver => "weep/debug-step-over",
            Button::StepInto => "weep/arrow-down",
            Button::StepOut => "weep/arrow-up",
            Button::Output => "weep/debug-output",
        }
    }

    /// The tui skin's one-cell mark (JetBrains Mono has each).
    fn glyph(self) -> &'static str {
        match self {
            Button::Resume => "▶",
            Button::Pause => "‖",
            Button::Restart => "↩",
            Button::Stop => "■",
            Button::StepOver => "↪",
            Button::StepInto => "↓",
            Button::StepOut => "↑",
            Button::Output => "□",
        }
    }
}

/// How a button shows for the session's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    /// Pressable: its colour, a hover box, its action.
    Enabled,
    /// Cannot act now: `text.disabled`, no hover.
    Disabled,
    /// Drawn in its colour but does nothing (the unprobed last button).
    Inert,
}

/// The eight buttons and how each shows: Resume and the steps while
/// stopped, Pause while running, Restart and Stop until the session is
/// over (batch 13, stopped: Pause alone greyed).
fn buttons(state: &SessionState) -> Vec<(Button, Look)> {
    let stopped = state.is_stopped();
    let live = !state.is_over();
    let on = |b: bool| if b { Look::Enabled } else { Look::Disabled };
    Button::ALL
        .into_iter()
        .map(|button| {
            let look = match button {
                Button::Resume | Button::StepOver | Button::StepInto | Button::StepOut => {
                    on(stopped)
                }
                Button::Pause => on(matches!(state, SessionState::Running)),
                Button::Restart | Button::Stop => on(live),
                Button::Output => Look::Inert,
            };
            (button, look)
        })
        .collect()
}

/// The panel's top-left corner: its top edge `top` under `area`'s (the
/// active editor's text area), its right edge `right` in from `area`'s,
/// kept inside `window`.
fn place(
    area: Bounds<Pixels>,
    size: gpui::Size<Pixels>,
    top: Pixels,
    right: Pixels,
    window: gpui::Size<Pixels>,
) -> Point<Pixels> {
    let x = area.origin.x + area.size.width - right - size.width;
    let y = area.origin.y + top;
    point(
        x.min(window.width - size.width).max(px(0.)),
        y.min(window.height - size.height).max(px(0.)),
    )
}

/// The panel's size in the Fleet skin: the buttons and their gaps, the
/// padding and the 1 px border (batch 13: 224 x 28).
fn fleet_size(m: &Metrics) -> gpui::Size<Pixels> {
    let n = Button::ALL.len() as f32;
    gpui::size(
        px(n * m.debug_panel_button + (n - 1.) * m.debug_panel_gap + 2. * m.debug_panel_pad + 2.),
        px(m.debug_panel_button + 2. * m.debug_panel_pad + 2.),
    )
}

/// The tui panel's columns and rows: a box, three cells a button.
const TUI_COLS: usize = 2 + 3 * Button::ALL.len();
const TUI_ROWS: usize = 3;

/// The editor the panel is drawn over while it shows, for the editor's
/// status badge (`status_top_under_panel`).
#[derive(Default)]
struct PanelOver(Option<EntityId>);

impl Global for PanelOver {}

/// M8: where the editor `editor` puts its status badge while the floating
/// debug panel covers its top right: under the panel (batch 13: the badge's
/// box 8 + 6 under the panel's bottom edge, Fleet skin), or the row under
/// the panel's box (tui). `None` while the panel is not over `editor`. The
/// editor takes the larger of this and the find bar's offset.
// M8: until editor.rs's `render_problems` (agent B) reads it.
pub fn status_top_under_panel(editor: EntityId, cx: &App) -> Option<Pixels> {
    let over = cx.try_global::<PanelOver>()?.0?;
    if over != editor {
        return None;
    }
    let m = cx.global::<Metrics>();
    Some(if Skin::current(cx) == Skin::Tui {
        Cell::current(cx).rows(TUI_ROWS)
    } else {
        px(m.debug_panel_top) + fleet_size(m).height + px(m.debug_panel_status_gap)
    })
}

pub struct DebugPanel {
    /// The workspace's active editor: the panel sits over it, and its text
    /// is what the pointer evaluates.
    editor: Option<WeakEntity<EditorView>>,
    quick: Option<Entity<QuickEvaluate>>,
    attach: Option<Entity<AttachPopup>>,
    /// The value on hover: the char under the pointer and its timer.
    pointer_offset: Option<usize>,
    pointer_epoch: u64,
    pointer_task: Option<Task<()>>,
    /// A mouse popup the pointer left closes after a grace period.
    close_task: Option<Task<()>>,
    /// The session the panel shows, and its observation.
    session: Option<WeakEntity<DebugSession>>,
    session_subscription: Option<Subscription>,
    quick_subscription: Option<Subscription>,
    attach_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl DebugPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let debugger = Debugger::global(cx);
        let subscriptions = vec![
            cx.observe(&debugger, |this, _, cx| this.follow_active(cx)),
            cx.observe_global::<AppSettings>(|_, cx| cx.notify()),
        ];
        let mut panel = DebugPanel {
            editor: None,
            quick: None,
            attach: None,
            pointer_offset: None,
            pointer_epoch: 0,
            pointer_task: None,
            close_task: None,
            session: None,
            session_subscription: None,
            quick_subscription: None,
            attach_subscription: None,
            _subscriptions: subscriptions,
        };
        panel.follow_active(cx);
        panel
    }

    /// The workspace's render hook: tells the panel the active editor and
    /// hands the panel back as the child to draw (over the whole window).
    /// It also records which editor the panel covers, for that editor's
    /// status badge (the editors render after the workspace).
    pub fn over(
        panel: &Entity<DebugPanel>,
        editor: Option<&Entity<EditorView>>,
        cx: &mut App,
    ) -> Entity<DebugPanel> {
        let id = editor.map(|e| e.entity_id());
        let editor = editor.map(Entity::downgrade);
        panel.update(cx, |p, _| {
            if p.editor.as_ref().map(|e| e.entity_id()) != id {
                p.editor = editor;
                p.pointer_offset = None;
            }
        });
        let shown = panel.read(cx).shown(cx).is_some();
        let over = id.filter(|_| shown);
        if cx.try_global::<PanelOver>().map(|g| g.0) != Some(over) {
            cx.set_global(PanelOver(over));
        }
        panel.clone()
    }

    /// Keeps the panel on the active session: a new one repaints it, and
    /// any change of it does.
    fn follow_active(&mut self, cx: &mut Context<Self>) {
        let active = Debugger::global(cx).read(cx).active();
        let same = match (&active, &self.session) {
            (Some(a), Some(s)) => s.entity_id() == a.entity_id(),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.session = active.as_ref().map(Entity::downgrade);
            self.session_subscription = active
                .as_ref()
                .map(|s| cx.observe(s, |_, _, cx| cx.notify()));
            self.close_quick(cx);
        }
        cx.notify();
    }

    fn session(&self) -> Option<Entity<DebugSession>> {
        self.session.as_ref().and_then(WeakEntity::upgrade)
    }

    fn active_editor(&self) -> Option<Entity<EditorView>> {
        self.editor.as_ref().and_then(WeakEntity::upgrade)
    }

    // ---- quick evaluate ----------------------------------------------------------

    /// `debug/quick-evaluate-expression` in `editor`: the selection, else
    /// the expression at the caret, evaluated in the active session's
    /// selected frame. Nothing happens while no session is stopped.
    pub fn quick_evaluate(
        &mut self,
        editor: &Entity<EditorView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = stopped_session(cx) else {
            return;
        };
        let Some((expression, range)) = editor.update(cx, |e, cx| e.eval_expression_at_caret(cx))
        else {
            return;
        };
        self.editor = Some(editor.downgrade());
        let quick = cx.new(|cx| QuickEvaluate::new(editor, &session, expression, range, false, cx));
        quick.update(cx, |q, cx| q.take_focus(window, cx));
        self.set_quick(quick, cx);
    }

    fn set_quick(&mut self, quick: Entity<QuickEvaluate>, cx: &mut Context<Self>) {
        self.quick_subscription = Some(cx.subscribe(&quick, |this, _, event, cx| match event {
            QuickEvaluateEvent::Close => this.close_quick(cx),
        }));
        self.quick = Some(quick);
        cx.notify();
    }

    fn close_quick(&mut self, cx: &mut Context<Self>) {
        self.close_task = None;
        if self.quick.take().is_some() {
            self.quick_subscription = None;
            cx.notify();
        }
    }

    /// The pointer moved anywhere in the window: a mouse popup closes when
    /// the pointer leaves its expression, and resting on an expression in
    /// code while stopped opens one after the delay.
    fn pointer_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button.is_some() {
            return;
        }
        let Some(editor) = self.active_editor() else {
            return;
        };
        if stopped_session(cx).is_none() {
            self.pointer_offset = None;
            return;
        }
        let offset = editor.read(cx).eval_offset_at_point(event.position, cx);
        if let Some(q) = &self.quick {
            let q = q.read(cx);
            let inside = q.hovered || offset.is_some_and(|o| q.range.contains(&o));
            if !q.from_mouse || inside {
                self.close_task = None;
            } else if self.close_task.is_none() {
                // A moment's grace: the pointer crosses the gap (or the
                // next row) on its way from the expression into the popup.
                self.close_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(HOVER_CLOSE_GRACE_MS))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.close_task = None;
                        let keep = this.quick.as_ref().is_some_and(|q| {
                            let q = q.read(cx);
                            !q.from_mouse
                                || q.hovered
                                || this.pointer_offset.is_some_and(|o| q.range.contains(&o))
                        });
                        if !keep {
                            this.close_quick(cx);
                        }
                    });
                }));
            }
        }
        if offset == self.pointer_offset {
            return;
        }
        self.pointer_offset = offset;
        self.pointer_epoch += 1;
        self.pointer_task = None;
        let Some(offset) = offset else {
            return;
        };
        let delay = cx
            .global::<AppSettings>()
            .settings
            .editor_hover_tooltip_delay_ms();
        let epoch = self.pointer_epoch;
        let weak_editor = editor.downgrade();
        self.pointer_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.pointer_epoch != epoch {
                    return;
                }
                let Some(editor) = weak_editor.upgrade() else {
                    return;
                };
                this.hover_evaluate(&editor, offset, cx);
            });
        }));
    }

    /// The pointer rested on `offset`: its expression's value, unless a
    /// popup the keyboard opened is up or one already shows it.
    fn hover_evaluate(
        &mut self,
        editor: &Entity<EditorView>,
        offset: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(q) = &self.quick {
            let q = q.read(cx);
            if !q.from_mouse || q.range.contains(&offset) {
                return;
            }
        }
        let Some(session) = stopped_session(cx) else {
            return;
        };
        let Some((expression, range)) = editor.update(cx, |e, cx| e.eval_expression_at(offset, cx))
        else {
            return;
        };
        let quick = cx.new(|cx| QuickEvaluate::new(editor, &session, expression, range, true, cx));
        self.set_quick(quick, cx);
    }

    // ---- attach to process ---------------------------------------------------------

    /// `debugger/attach-to-process`: the dialog listing the user's
    /// processes; Enter on one (or on a typed pid) calls `on_attach` with
    /// the pid and the process's name, then closes.
    pub fn attach_to_process(
        &mut self,
        on_attach: impl Fn(u32, String, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on_attach: OnAttach = Rc::new(on_attach);
        let popup = cx.new(|cx| AttachPopup::new(on_attach, cx));
        let handle = popup.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.attach_subscription = Some(cx.subscribe_in(
            &popup,
            window,
            |this, _, event: &AttachPopupEvent, window, cx| match event {
                AttachPopupEvent::Close => {
                    // The keyboard goes back to the editor unless a click
                    // outside the dialog already took it elsewhere.
                    let had_focus = this
                        .attach
                        .as_ref()
                        .is_some_and(|p| p.read(cx).focus_handle(cx).contains_focused(window, cx));
                    this.attach = None;
                    this.attach_subscription = None;
                    if let (true, Some(editor)) = (had_focus, this.active_editor()) {
                        let handle = editor.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }
                    cx.notify();
                }
            },
        ));
        self.attach = Some(popup);
        cx.notify();
    }

    // ---- rendering -------------------------------------------------------------------

    fn shown(&self, cx: &App) -> Option<SessionState> {
        if !cx
            .global::<AppSettings>()
            .settings
            .editor_show_floating_debug_panel()
        {
            return None;
        }
        let session = self.session()?;
        let state = session.read(cx).state.clone();
        (!state.is_over()).then_some(state)
    }

    /// The pointer's tooltip: the action's title and its chord on one line
    /// (batch 13: "Step Into ⌘;" under the pointer).
    fn tooltip(button: Button, cx: &App) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
        let title = button.title();
        let hint = hint_for(cx, button.action_id());
        move |window, cx| match &hint {
            Some(h) => Tooltip::with_shortcut(title.clone(), h.clone())(window, cx),
            None => Tooltip::text(title.clone())(window, cx),
        }
    }

    fn render_fleet(
        &mut self,
        state: &SessionState,
        area: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let size = fleet_size(&m);
        let at = place(
            area,
            size,
            px(m.debug_panel_top),
            px(m.debug_panel_right),
            window.viewport_size(),
        );
        let row: Vec<AnyElement> = buttons(state)
            .into_iter()
            .map(|(button, look)| {
                let color = match (button, look) {
                    (_, Look::Disabled) => theme.text_disabled,
                    (Button::Resume, _) => theme.icon_green,
                    (Button::Stop, _) => theme.icon_red,
                    _ => theme.icon_primary,
                };
                let action = button.action().filter(|_| look == Look::Enabled);
                div()
                    .id(SharedString::from(button.action_id()))
                    .size(px(m.debug_panel_button))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(m.debug_panel_button_radius))
                    .child(icon(button.icon(), m.icon_size, color))
                    .when(button != Button::Output, |d| {
                        d.tooltip(Self::tooltip(button, cx))
                    })
                    .when_some(action, |d, action| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme.ghost_hovered))
                            .active(|s| s.bg(theme.ghost_pressed))
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(action.boxed_clone(), cx)
                            })
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .id("debug-panel")
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(size.width)
            .h(size.height)
            .occlude()
            .flex()
            .items_center()
            .gap(px(m.debug_panel_gap))
            .p(px(m.debug_panel_pad))
            .rounded(px(m.debug_panel_radius))
            .border_1()
            .border_color(theme.popup_border)
            .bg(theme.popup_bg)
            .children(row)
            .into_any_element()
    }

    /// The tui skin: a box of three rows on the grid at the editor's top
    /// right (one cell in from its edge), each button its mark with a cell
    /// either side, filled on hover; disabled ones in `text_disabled`.
    fn render_tui(
        &mut self,
        state: &SessionState,
        area: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let size = gpui::size(cell.cols(TUI_COLS), cell.rows(TUI_ROWS));
        let at = place(area, size, px(0.), px(cell.w), window.viewport_size());
        let (col, row) = (Cell::fit(at.x, cell.w), Cell::fit(at.y, cell.h));
        let body = div()
            .flex()
            .h(px(cell.h))
            .children(buttons(state).into_iter().map(|(button, look)| {
                let color = match (button, look) {
                    (_, Look::Disabled) => theme.text_disabled,
                    (Button::Resume, _) => theme.text_positive,
                    (Button::Stop, _) => theme.text_dangerous,
                    _ => theme.text_primary,
                };
                let action = button.action().filter(|_| look == Look::Enabled);
                div()
                    .id(SharedString::from(button.action_id()))
                    .w(cell.cols(3))
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .text_color(color)
                    .child(button.glyph())
                    .when(button != Button::Output, |d| {
                        d.tooltip(Self::tooltip(button, cx))
                    })
                    .when_some(action, |d, action| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme.list_bg_hovered))
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(action.boxed_clone(), cx)
                            })
                    })
            }))
            .into_any_element();
        div()
            .id("debug-panel")
            .absolute()
            .left(cell.cols(col))
            .top(cell.rows(row))
            .occlude()
            .child(skin::boxed(
                cell,
                TUI_COLS,
                TUI_ROWS,
                Vec::new(),
                Vec::new(),
                body,
                theme.popup_bg,
                theme.popup_border,
                &theme,
                m,
            ))
            .into_any_element()
    }
}

impl Render for DebugPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let area = self
            .active_editor()
            .map(|e| e.read(cx).eval_view_bounds())
            .filter(|b| b.size.width > px(0.));
        let toolbar = match (self.shown(cx), area) {
            (Some(state), Some(area)) => Some(if Skin::current(cx) == Skin::Tui {
                self.render_tui(&state, area, window, cx)
            } else {
                self.render_fleet(&state, area, window, cx)
            }),
            _ => None,
        };
        // The window's pointer while the debuggee is stopped (the value on
        // hover).
        let listen = stopped_session(cx).is_some();
        let entity = cx.entity();
        let pointer = canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                if !listen {
                    return;
                }
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        entity.update(cx, |this, cx| this.pointer_moved(e, cx));
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(pointer)
            .children(toolbar)
            .children(self.quick.clone())
            .children(self.attach.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    fn looks(state: &SessionState) -> Vec<Look> {
        buttons(state).into_iter().map(|(_, l)| l).collect()
    }

    #[test]
    fn eight_buttons_in_fleets_order() {
        let stopped = SessionState::Stopped {
            reason: "breakpoint".into(),
            description: None,
        };
        let b: Vec<Button> = buttons(&stopped).into_iter().map(|(b, _)| b).collect();
        assert_eq!(b, Button::ALL);
        use Look::*;
        // Stopped (batch 13): Pause alone greyed; the last button drawn
        // but inert.
        assert_eq!(
            looks(&stopped),
            [Enabled, Disabled, Enabled, Enabled, Enabled, Enabled, Enabled, Inert]
        );
        assert_eq!(
            looks(&SessionState::Running),
            [Disabled, Enabled, Enabled, Enabled, Disabled, Disabled, Disabled, Inert]
        );
        assert_eq!(looks(&SessionState::Starting)[1], Disabled);
        assert_eq!(looks(&SessionState::Starting)[3], Enabled);
    }

    #[test]
    fn measured_size() {
        let m = Metrics::at(1.);
        assert_eq!(fleet_size(&m), size(px(224.), px(28.)));
    }

    #[test]
    fn placed_at_the_top_right_and_kept_inside() {
        let area = Bounds::new(point(px(100.), px(50.)), size(px(400.), px(300.)));
        let panel = size(px(100.), px(30.));
        let window = size(px(800.), px(600.));
        let at = place(area, panel, px(0.), px(12.), window);
        assert_eq!(at, point(px(388.), px(50.)));
        // A narrow editor: the panel stays in the window.
        let narrow = Bounds::new(point(px(10.), px(50.)), size(px(60.), px(300.)));
        let at = place(narrow, panel, px(0.), px(12.), window);
        assert_eq!(at, point(px(0.), px(50.)));
    }
}
