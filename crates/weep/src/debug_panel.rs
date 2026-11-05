//! Fleet's floating debug toolbar (M8, `editor.showFloatingDebugPanel`,
//! "Show floating debug toolbar", off by default): while the active debug
//! session is not over, a small strip over the editor with Resume (Pause
//! while the debuggee runs), Step Over, Step Into, Step Out and Stop, each
//! dispatching its `debug/*` action (the workspace routes them to the
//! session), and a handle that drags it; the place it is dragged to is kept
//! for the app's life.
//!
//! The panel is the workspace's one debugger overlay, drawn over the whole
//! window: it also hosts the quick evaluate popup (`editor/debug.rs`) and
//! tracks the pointer for the value on hover while the debuggee is stopped,
//! and it hosts the attach dialog (`attach_popup.rs`).
//!
//! Batch 13 has not run, so the panel's place and sizes are estimates
//! (`theme.rs`, `DEBUG_PANEL_*`): the top centre of the active editor. The
//! colours are the floating surfaces' keys: `popup.background`,
//! `popup.border`, `shadow.medium`, the ghost buttons' washes.

use crate::actions::{
    DebugPause, DebugResume, DebugStepInto, DebugStepOut, DebugStepOver, DebugStop,
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
    canvas, div, point, prelude::*, px, Action, AnyElement, App, Bounds, BoxShadow, Context,
    CursorStyle, DispatchPhase, Entity, Focusable, MouseButton, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, SharedString, Subscription, Task, WeakEntity, Window,
};
use std::rc::Rc;
use std::time::Duration;

/// How long a value popup the pointer opened outlives the pointer leaving
/// its expression (estimate: long enough to cross into the popup).
const HOVER_CLOSE_GRACE_MS: u64 = 300;

/// One of the panel's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Resume,
    Pause,
    StepOver,
    StepInto,
    StepOut,
    Stop,
}

impl Button {
    fn action_id(self) -> &'static str {
        match self {
            Button::Resume => "debug/resume",
            Button::Pause => "debug/pause",
            Button::StepOver => "debug/step-over",
            Button::StepInto => "debug/step-into",
            Button::StepOut => "debug/step-out",
            Button::Stop => "debug/stop",
        }
    }

    fn action(self) -> Box<dyn Action> {
        match self {
            Button::Resume => Box::new(DebugResume),
            Button::Pause => Box::new(DebugPause),
            Button::StepOver => Box::new(DebugStepOver),
            Button::StepInto => Box::new(DebugStepInto),
            Button::StepOut => Box::new(DebugStepOut),
            Button::Stop => Box::new(DebugStop),
        }
    }

    /// JetBrains' expui run icons.
    fn icon(self) -> &'static str {
        match self {
            Button::Resume => "expui/run/resume",
            Button::Pause => "expui/run/pause",
            Button::StepOver => "expui/run/stepOver",
            Button::StepInto => "expui/run/stepInto",
            Button::StepOut => "expui/run/stepOut",
            Button::Stop => "expui/run/stop",
        }
    }

    /// The tui skin's one-cell mark (JetBrains Mono has each).
    fn glyph(self) -> &'static str {
        match self {
            Button::Resume => "▶",
            Button::Pause => "‖",
            Button::StepOver => "↪",
            Button::StepInto => "↧",
            Button::StepOut => "↥",
            Button::Stop => "■",
        }
    }
}

/// The buttons for a session's state, and whether each can be pressed:
/// Resume while stopped, Pause while running; the steps only while
/// stopped; Stop until the session is over.
fn buttons(state: &SessionState) -> Vec<(Button, bool)> {
    let stopped = state.is_stopped();
    let first = if stopped {
        (Button::Resume, true)
    } else {
        (Button::Pause, matches!(state, SessionState::Running))
    };
    vec![
        first,
        (Button::StepOver, stopped),
        (Button::StepInto, stopped),
        (Button::StepOut, stopped),
        (Button::Stop, !state.is_over()),
    ]
}

/// The panel's top-left corner: the top centre of `area` (the active
/// editor), moved by `offset`, kept inside `window`.
fn place(
    area: Bounds<Pixels>,
    size: gpui::Size<Pixels>,
    top: Pixels,
    offset: Point<Pixels>,
    window: gpui::Size<Pixels>,
) -> Point<Pixels> {
    let x = area.origin.x + (area.size.width - size.width) / 2. + offset.x;
    let y = area.origin.y + top + offset.y;
    point(
        x.min(window.width - size.width).max(px(0.)),
        y.min(window.height - size.height).max(px(0.)),
    )
}

pub struct DebugPanel {
    /// The workspace's active editor: the panel sits over it, and its text
    /// is what the pointer evaluates.
    editor: Option<WeakEntity<EditorView>>,
    /// Where the handle dragged the panel, from its default place.
    offset: Point<Pixels>,
    /// A drag in progress: the pointer and the offset at the press.
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
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
            offset: point(px(0.), px(0.)),
            drag: None,
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
    pub fn over(
        panel: &Entity<DebugPanel>,
        editor: Option<&Entity<EditorView>>,
        cx: &mut App,
    ) -> Entity<DebugPanel> {
        let editor = editor.map(Entity::downgrade);
        panel.update(cx, |p, _| {
            if p.editor.as_ref().map(|e| e.entity_id()) != editor.as_ref().map(|e| e.entity_id()) {
                p.editor = editor;
                p.pointer_offset = None;
            }
        });
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

    /// The pointer moved anywhere in the window: a drag of the panel goes
    /// on, a mouse popup closes when the pointer leaves its expression, and
    /// resting on an expression while stopped opens one after the delay.
    fn pointer_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some((start, from)) = self.drag {
            if event.pressed_button == Some(MouseButton::Left) {
                let mut offset = from + (event.position - start);
                if Skin::current(cx) == Skin::Tui {
                    // Rule 6: a drag lands on whole cells.
                    let cell = Cell::current(cx);
                    offset = point(
                        px((f32::from(offset.x) / cell.w).round() * cell.w),
                        px((f32::from(offset.y) / cell.h).round() * cell.h),
                    );
                }
                if offset != self.offset {
                    self.offset = offset;
                    cx.notify();
                }
                return;
            }
            self.drag = None;
        }
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
        let Some((expression, range)) = editor.read(cx).eval_expression_at(offset, cx) else {
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

    fn start_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.drag = Some((position, self.offset));
        cx.notify();
    }

    fn tooltip(button: Button, cx: &App) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
        let id = button.action_id();
        let title = action_title(id);
        let hint = hint_for(cx, id);
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
        let buttons = buttons(state);
        let n = buttons.len() as f32;
        let size = gpui::size(
            px(2. * m.debug_panel_pad
                + m.debug_panel_handle
                + n * m.debug_panel_button
                + n * m.debug_panel_gap
                + 2.),
            px(2. * m.debug_panel_pad + m.debug_panel_button + 2.),
        );
        let at = place(
            area,
            size,
            px(m.debug_panel_top),
            self.offset,
            window.viewport_size(),
        );
        let entity = cx.entity();
        let row: Vec<AnyElement> = buttons
            .into_iter()
            .map(|(button, enabled)| {
                let color = match button {
                    Button::Resume => theme.icon_green,
                    Button::Stop => theme.icon_red,
                    _ => theme.icon_primary,
                };
                div()
                    .id(SharedString::from(button.action_id()))
                    .size(px(m.debug_panel_button))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4. * m.zoom))
                    .child(icon(
                        button.icon(),
                        m.icon_size,
                        if enabled { color } else { theme.text_disabled },
                    ))
                    .tooltip(Self::tooltip(button, cx))
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme.ghost_hovered))
                            .active(|s| s.bg(theme.ghost_pressed))
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(button.action(), cx)
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
            .occlude()
            .flex()
            .items_center()
            .gap(px(m.debug_panel_gap))
            .p(px(m.debug_panel_pad))
            .rounded(px(m.popup_radius))
            .border_1()
            .border_color(theme.popup_border)
            .bg(theme.popup_bg)
            .shadow(vec![BoxShadow {
                color: theme.shadow_medium,
                offset: point(px(0.), px(m.popup_shadow_offset_y)),
                blur_radius: px(m.popup_shadow_blur),
                spread_radius: px(0.),
            }])
            .child(
                div()
                    .id("debug-panel-handle")
                    .w(px(m.debug_panel_handle))
                    .h(px(m.debug_panel_button))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(CursorStyle::OpenHand)
                    .child(icon(
                        "expui/general/moreVertical",
                        m.debug_panel_handle,
                        theme.icon_secondary,
                    ))
                    .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                        cx.stop_propagation();
                        entity.update(cx, |this, cx| this.start_drag(e.position, cx));
                    }),
            )
            .children(row)
            .into_any_element()
    }

    /// The tui skin: a box of three rows on the grid, `⋮` for the handle,
    /// then each button as its mark with a cell either side, filled on
    /// hover; disabled ones in `text_disabled`.
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
        let buttons = buttons(state);
        let cols = 2 + 3 + 3 * buttons.len();
        let rows = 3;
        let size = gpui::size(cell.cols(cols), cell.rows(rows));
        let at = place(area, size, px(0.), self.offset, window.viewport_size());
        let (col, row) = (Cell::fit(at.x, cell.w), Cell::fit(at.y, cell.h));
        let entity = cx.entity();
        let body = div()
            .flex()
            .h(px(cell.h))
            .child(
                div()
                    .id("debug-panel-handle")
                    .w(cell.cols(3))
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .cursor(CursorStyle::OpenHand)
                    .text_color(theme.text_tertiary)
                    .child("⋮")
                    .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                        cx.stop_propagation();
                        entity.update(cx, |this, cx| this.start_drag(e.position, cx));
                    }),
            )
            .children(buttons.into_iter().map(|(button, enabled)| {
                let color = match button {
                    Button::Resume => theme.text_positive,
                    Button::Stop => theme.text_dangerous,
                    _ => theme.text_primary,
                };
                div()
                    .id(SharedString::from(button.action_id()))
                    .w(cell.cols(3))
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .text_color(if enabled { color } else { theme.text_disabled })
                    .child(button.glyph())
                    .tooltip(Self::tooltip(button, cx))
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme.list_bg_hovered))
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(button.action(), cx)
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
                cols,
                rows,
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
        // The window's pointer, while a drag goes on or the debuggee is
        // stopped (the value on hover).
        let listen = self.drag.is_some() || stopped_session(cx).is_some();
        let entity = cx.entity();
        let pointer = canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                if !listen {
                    return;
                }
                let moved = entity.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        moved.update(cx, |this, cx| this.pointer_moved(e, cx));
                    }
                });
                let released = entity.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && e.button == MouseButton::Left {
                        released.update(cx, |this, cx| {
                            if this.drag.take().is_some() {
                                cx.notify();
                            }
                        });
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

    #[test]
    fn buttons_follow_the_state() {
        let stopped = SessionState::Stopped {
            reason: "breakpoint".into(),
            description: None,
        };
        let b = buttons(&stopped);
        assert_eq!(b[0], (Button::Resume, true));
        assert!(b[1..].iter().all(|(_, on)| *on));
        let b = buttons(&SessionState::Running);
        assert_eq!(b[0], (Button::Pause, true));
        assert_eq!(b[1], (Button::StepOver, false));
        assert_eq!(b[4], (Button::Stop, true));
        let b = buttons(&SessionState::Starting);
        assert_eq!(b[0], (Button::Pause, false));
        assert_eq!(b[4], (Button::Stop, true));
    }

    #[test]
    fn placed_at_the_top_centre_and_kept_inside() {
        let area = Bounds::new(point(px(100.), px(50.)), size(px(400.), px(300.)));
        let panel = size(px(100.), px(30.));
        let window = size(px(800.), px(600.));
        let at = place(area, panel, px(8.), point(px(0.), px(0.)), window);
        assert_eq!(at, point(px(250.), px(58.)));
        let at = place(area, panel, px(8.), point(px(1000.), px(-100.)), window);
        assert_eq!(at, point(px(700.), px(0.)));
    }
}
