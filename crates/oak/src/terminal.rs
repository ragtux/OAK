//! The Terminal tool (M5): one shell per tab in the bottom panel, painted
//! from `oak-terminal`'s grid at `terminal.fontSize` × `terminal.lineSpacing`
//! with the theme's `terminal.ansiColors.*` keys, and Fleet's actions on it
//! (`terminal/copy`, `terminal/paste`, `select-all-terminal`, `terminal/clear`).
//! Batch 9 (`docs/probe-plan.md`) has not run against Fleet: the grid's
//! inset, the caret, the selection colour and the scroll-to-bottom button are
//! estimates, marked in `theme.rs::metrics`.

use crate::actions::{
    GotoEverything, SelectAllTerminal, TerminalClear, TerminalCopy, TerminalPaste,
};
use crate::assets::icon;
use crate::settings::AppSettings;
use crate::theme::{Metrics, Theme};
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use futures::StreamExt;
use gpui::{
    div, fill, linear_color_stop, linear_gradient, point, prelude::*, px, size, App, Bounds,
    ClipboardItem, ContentMask, Context, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, EventEmitter, FocusHandle, Focusable, Font, FontFeatures,
    FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, StrikethroughStyle,
    Style, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use oak_settings::CaretShape;
use oak_terminal::keys::{self, KeyModes};
use oak_terminal::{Color, CursorShape, Profile, SelectionKind, Size, Snapshot, Terminal};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum TerminalEvent {
    /// The shell ended: the tab closes.
    Exited,
    /// Esc with `terminal.enableFocusEditorOnEsc`: the editor takes the focus.
    FocusEditor,
    /// A right click, at this window position: the workspace opens the menu.
    ContextMenu(Point<Pixels>),
}

pub struct TerminalView {
    terminal: Option<Terminal>,
    /// Why there is no shell, when the spawn failed.
    error: Option<String>,
    profile: Profile,
    focus_handle: FocusHandle,
    /// The grid's origin and cell size at the last paint, for the mouse.
    origin: Point<Pixels>,
    cell: (Pixels, Pixels),
    dragging: bool,
    /// The display is scrolled back.
    scrolled_back: bool,
    /// Output arrived while scrolled back: the scroll-to-bottom button shows
    /// (estimate: Fleet showed none while scrolled back and idle).
    fresh_output: bool,
    was_focused: bool,
    /// The options last given to the shell, to notice a settings change.
    options: (usize, CursorShape),
    /// The shell's exit is reported once.
    exit_reported: bool,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The caret the settings ask for: `terminal.caretShape`, else the editor's.
fn caret_shape(settings: &oak_settings::Settings) -> CursorShape {
    match settings
        .terminal_caret_shape()
        .unwrap_or_else(|| settings.editor_caret_shape())
    {
        CaretShape::Line => CursorShape::Beam,
        CaretShape::Block => CursorShape::Block,
        CaretShape::Underline => CursorShape::Underline,
    }
}

impl TerminalView {
    pub fn new(profile: Profile, cwd: PathBuf, cx: &mut Context<Self>) -> Self {
        let settings = &cx.global::<AppSettings>().settings;
        let options = (settings.terminal_line_capacity(), caret_shape(settings));
        let (tx, rx) = unbounded::<()>();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = tx.unbounded_send(());
        });
        // A first size before the element has measured anything; the first
        // frame resizes it to the island.
        let size = Size {
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 22,
        };
        let (terminal, error) =
            match Terminal::spawn(&profile, Some(&cwd), size, options.0, options.1, wake) {
                Ok(t) => (Some(t), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            };
        if let Some(e) = &error {
            eprintln!("oak: terminal: {e}");
        }
        let view = TerminalView {
            terminal,
            error,
            profile,
            focus_handle: cx.focus_handle(),
            origin: point(px(0.), px(0.)),
            cell: (px(8.), px(22.)),
            dragging: false,
            scrolled_back: false,
            fresh_output: false,
            was_focused: false,
            options,
            exit_reported: false,
        };
        view.listen(rx, cx);
        view
    }

    /// Redraws whenever Alacritty's thread has something new.
    fn listen(&self, mut rx: UnboundedReceiver<()>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while rx.next().await.is_some() {
                if this.update(cx, |view, cx| view.pump(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        let Some(terminal) = self.terminal.as_mut() else {
            return;
        };
        if terminal.pump() {
            cx.notify();
        }
        if terminal.take_output() && self.scrolled_back {
            self.fresh_output = true;
        }
        if let Some(text) = terminal.take_clipboard() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        terminal.take_bells();
        if terminal.exited.is_some() && !self.exit_reported {
            self.exit_reported = true;
            cx.emit(TerminalEvent::Exited);
        }
    }

    /// The tab's title: "Terminal", every tab alike (batch 9); the profile
    /// and the folder are the tab's tooltip.
    pub fn title(&self) -> SharedString {
        "Terminal".into()
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    fn snapshot(&self) -> Option<Snapshot> {
        self.terminal.as_ref().map(Terminal::snapshot)
    }

    /// The element laid the grid out: the shell follows the island's size.
    fn laid_out(
        &mut self,
        origin: Point<Pixels>,
        cell: (Pixels, Pixels),
        cols: usize,
        rows: usize,
        cx: &mut Context<Self>,
    ) {
        self.origin = origin;
        self.cell = cell;
        let settings = &cx.global::<AppSettings>().settings;
        let options = (settings.terminal_line_capacity(), caret_shape(settings));
        if let Some(terminal) = self.terminal.as_mut() {
            if options != self.options {
                self.options = options;
                terminal.set_options(options.0, options.1);
            }
            terminal.resize(Size {
                cols: cols.max(2),
                rows: rows.max(1),
                cell_width: f32::from(cell.0).round().max(1.) as u16,
                cell_height: f32::from(cell.1).round().max(1.) as u16,
            });
        }
    }

    fn set_scrolled_back(&mut self, scrolled: bool, cx: &mut Context<Self>) {
        if self.scrolled_back != scrolled {
            self.scrolled_back = scrolled;
            if !scrolled {
                self.fresh_output = false;
            }
            cx.notify();
        }
    }

    /// A menu row scrolls as the key would.
    pub fn scroll_by_key(&mut self, key: &str, lines: bool, cx: &mut Context<Self>) {
        if self.scroll_key(key, lines) {
            cx.notify();
        }
    }

    /// Alt+Up and Alt+Down scroll a line, Shift+PageUp and Shift+PageDown a
    /// page (Fleet's terminal menu lists these four).
    fn scroll_key(&self, key: &str, lines: bool) -> bool {
        let Some(terminal) = &self.terminal else {
            return false;
        };
        match (key, lines) {
            ("up", true) => terminal.scroll_lines(1),
            ("down", true) => terminal.scroll_lines(-1),
            ("pageup", false) => terminal.scroll_page_up(),
            ("pagedown", false) => terminal.scroll_page_down(),
            _ => return false,
        }
        true
    }

    /// The cell under a window position, and whether the point is in its right half.
    fn cell_at(&self, position: Point<Pixels>) -> (usize, usize, bool) {
        let x = (f32::from(position.x - self.origin.x) / f32::from(self.cell.0)).max(0.);
        let y = (f32::from(position.y - self.origin.y) / f32::from(self.cell.1)).max(0.);
        (y as usize, x as usize, x.fract() > 0.5)
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            window.focus(&self.focus_handle, cx);
        }
        match event.button {
            MouseButton::Left => {
                let Some(terminal) = &self.terminal else {
                    return;
                };
                let (row, col, right) = self.cell_at(event.position);
                let kind = match event.click_count {
                    1 => SelectionKind::Simple,
                    2 => SelectionKind::Word,
                    _ => SelectionKind::Line,
                };
                terminal.selection_start(row, col, right, kind);
                self.dragging = true;
                cx.notify();
            }
            MouseButton::Right => cx.emit(TerminalEvent::ContextMenu(event.position)),
            _ => {}
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.dragging || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some(terminal) = &self.terminal {
            let (row, col, right) = self.cell_at(event.position);
            terminal.selection_update(row, col, right);
            cx.notify();
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left || !self.dragging {
            return;
        }
        self.dragging = false;
        let copy = cx
            .global::<AppSettings>()
            .settings
            .terminal_copy_on_selection();
        if let Some(terminal) = &self.terminal {
            if copy {
                if let Some(text) = terminal.selection_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            if !terminal.has_selection() {
                terminal.clear_selection();
            }
        }
        cx.notify();
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = &self.terminal else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Lines(l) => l.y,
            ScrollDelta::Pixels(p) => f32::from(p.y) / f32::from(self.cell.1),
        };
        let lines = if lines.abs() < 1. {
            lines.signum()
        } else {
            lines.round()
        } as i32;
        if lines == 0 {
            return;
        }
        let modes = terminal.modes();
        if modes.alt_screen && modes.alternate_scroll {
            // A full-screen program gets the wheel as arrow keys, as xterm sends it.
            let key: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
            let bytes: Vec<u8> = key
                .iter()
                .copied()
                .cycle()
                .take(key.len() * lines.unsigned_abs() as usize)
                .collect();
            terminal.write(bytes);
        } else {
            terminal.scroll_lines(lines);
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = &self.terminal else {
            return;
        };
        let ks = &event.keystroke;
        let settings = &cx.global::<AppSettings>().settings;
        let modes = terminal.modes();
        let plain = !ks.modifiers.control && !ks.modifiers.alt && !ks.modifiers.platform;
        if ks.key == "escape"
            && plain
            && settings.terminal_focus_editor_on_esc()
            && (!modes.alt_screen || settings.terminal_focus_panels_in_apps())
        {
            cx.emit(TerminalEvent::FocusEditor);
            cx.stop_propagation();
            return;
        }
        if !modes.alt_screen {
            let page = ks.modifiers.shift && plain;
            let line = ks.modifiers.alt && !ks.modifiers.control && !ks.modifiers.shift;
            if (page || line) && self.scroll_key(&ks.key, line) {
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        let mods = keys::Modifiers {
            shift: ks.modifiers.shift,
            control: ks.modifiers.control,
            alt: ks.modifiers.alt,
        };
        let key_modes = KeyModes {
            app_cursor: modes.app_cursor,
            alt_is_meta: !cfg!(target_os = "macos") || settings.terminal_use_mac_option_as_meta(),
        };
        if let Some(bytes) = keys::encode(&ks.key, ks.key_char.as_deref(), mods, key_modes) {
            terminal.clear_selection();
            terminal.write(bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// `terminal.clearTerminalOnCmdK`: the Goto chord clears the terminal instead.
    fn goto_everything(&mut self, _: &GotoEverything, window: &mut Window, cx: &mut Context<Self>) {
        if cx
            .global::<AppSettings>()
            .settings
            .terminal_clear_on_cmd_k()
        {
            self.clear(&TerminalClear, window, cx);
        } else {
            cx.propagate();
        }
    }

    pub fn clear(&mut self, _: &TerminalClear, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.clear();
            cx.notify();
        }
    }

    pub fn copy(&mut self, _: &TerminalCopy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.as_ref().and_then(Terminal::selection_text) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn paste(&mut self, _: &TerminalPaste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if let Some(terminal) = &self.terminal {
            terminal.clear_selection();
            terminal.paste(&text);
            cx.notify();
        }
    }

    pub fn select_all(&mut self, _: &SelectAllTerminal, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.select_all();
            cx.notify();
        }
    }

    pub fn has_selection(&self) -> bool {
        self.terminal.as_ref().is_some_and(Terminal::has_selection)
    }

    fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.scroll_to_bottom();
            cx.notify();
        }
    }
}

/// Typed text reaches the shell through the platform's text input (IME and
/// dead keys included); the terminal has no text of its own to report.
impl gpui::EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Control characters come through `key_down`; a text input only brings
        // what a key would print.
        let text: String = new_text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() {
            return;
        }
        if let Some(terminal) = &self.terminal {
            terminal.clear_selection();
            terminal.write(text.into_bytes());
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cursor = self.snapshot()?.cursor?;
        Some(Bounds::new(
            point(
                self.origin.x + self.cell.0 * cursor.col as f32,
                self.origin.y + self.cell.1 * cursor.row as f32,
            ),
            size(self.cell.0, self.cell.1),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let focused = self.focus_handle.is_focused(window);
        if focused != self.was_focused {
            self.was_focused = focused;
            if let Some(terminal) = &self.terminal {
                terminal.focus_changed(focused);
            }
        }
        let entity = cx.entity();
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::goto_everything))
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .relative()
            .overflow_hidden()
            .child(TerminalElement { view: cx.entity() })
            .when_some(self.error.clone(), |d, error| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .p(px(m.terminal_pad_x))
                        .text_color(theme.text_dangerous)
                        .child(format!("The shell could not start: {error}")),
                )
            })
            .when(self.scrolled_back && self.fresh_output, |d| {
                let size = px(m.scroll_to_bottom_size);
                let [top, bottom] = theme.scroll_to_bottom;
                let [top_h, bottom_h] = theme.scroll_to_bottom_hovered;
                d.child(
                    div()
                        .id("scroll-to-bottom")
                        .absolute()
                        .right(px(m.scroll_to_bottom_margin))
                        .bottom(px(m.scroll_to_bottom_margin))
                        .size(size)
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(top, 0.),
                            linear_color_stop(bottom, 1.),
                        ))
                        .hover(move |s| {
                            s.bg(linear_gradient(
                                180.,
                                linear_color_stop(top_h, 0.),
                                linear_color_stop(bottom_h, 1.),
                            ))
                        })
                        .on_click(move |_, window, cx| {
                            entity.update(cx, |view, cx| {
                                view.scroll_to_bottom(cx);
                                window.focus(&view.focus_handle, cx);
                            })
                        })
                        .child(icon(
                            "expui/general/chevronDown",
                            m.icon_size,
                            theme.icon_primary,
                        )),
                )
            })
    }
}

/// One shaped run of a row: its first column and the shaped text.
struct RunLayout {
    col: usize,
    line: ShapedLine,
}

struct RowLayout {
    runs: Vec<RunLayout>,
    /// Background washes: column range and colour.
    backgrounds: Vec<(Range<usize>, Hsla)>,
}

pub struct Prepaint {
    hitbox: Hitbox,
    rows: Vec<RowLayout>,
    cursor: Option<(Bounds<Pixels>, CursorShape, Option<ShapedLine>)>,
    origin: Point<Pixels>,
    cell: (Pixels, Pixels),
    focused: bool,
}

/// Paints the grid: backgrounds, the selection, the text and the caret.
struct TerminalElement {
    view: Entity<TerminalView>,
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

/// A cell's colour as the theme paints it.
fn resolve(color: Color, theme: &Theme, foreground: bool) -> Hsla {
    match color {
        Color::Foreground => theme.text_primary,
        Color::Background => theme.island_background,
        Color::Ansi(i) => {
            if foreground {
                theme.terminal_fg[(i as usize) % 16]
            } else {
                theme.terminal_bg[(i as usize) % 16]
            }
        }
        Color::Indexed(i) => {
            let (r, g, b) = oak_terminal::colors::indexed(i).unwrap_or((128, 128, 128));
            gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into()
        }
        Color::Rgb(r, g, b) => gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into(),
    }
}

/// The style a run of cells shares.
#[derive(Clone, Copy, PartialEq)]
struct RunStyle {
    color: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strikeout: bool,
}

fn terminal_font(
    theme: &Theme,
    settings: &oak_settings::Settings,
    bold: bool,
    italic: bool,
) -> Font {
    let family: SharedString = settings
        .terminal_font_family()
        .map(|f| f.to_string().into())
        .unwrap_or_else(|| theme.mono_font.clone());
    let weight = settings.terminal_font_weight();
    let tags: Vec<(String, u32)> = settings
        .terminal_font_features()
        .into_iter()
        .map(|t| (t, 1))
        .collect();
    Font {
        family,
        features: FontFeatures(Arc::new(tags)),
        fallbacks: None,
        weight: FontWeight(if bold {
            (weight + 300.).min(900.)
        } else {
            weight
        }),
        style: if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let settings = cx.global::<AppSettings>().settings.clone();
        let (font_size, line_height) = m.terminal_text(&settings);
        let font_size = px(font_size);
        let line_height = px(line_height);
        let plain = terminal_font(&theme, &settings, false, false);
        let cell_width = window
            .text_system()
            .shape_line(
                "W".into(),
                font_size,
                &[TextRun {
                    len: 1,
                    font: plain.clone(),
                    color: theme.text_primary,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width
            .max(px(1.));
        let origin = point(
            bounds.origin.x + px(m.terminal_pad_x),
            bounds.origin.y + px(m.terminal_pad_top),
        );
        let inner_w = f32::from(bounds.size.width) - 2. * m.terminal_pad_x;
        let inner_h = f32::from(bounds.size.height) - m.terminal_pad_top - m.terminal_pad_bottom;
        let cols = (inner_w / f32::from(cell_width)).floor().max(2.) as usize;
        let rows = (inner_h / f32::from(line_height)).floor().max(1.) as usize;
        let focused = self.view.read(cx).focus_handle.is_focused(window);
        self.view.update(cx, |view, cx| {
            view.laid_out(origin, (cell_width, line_height), cols, rows, cx);
        });
        let Some(snapshot) = self.view.read(cx).snapshot() else {
            return Prepaint {
                hitbox,
                rows: Vec::new(),
                cursor: None,
                origin,
                cell: (cell_width, line_height),
                focused,
            };
        };
        self.view.update(cx, |view, cx| {
            view.set_scrolled_back(snapshot.display_offset > 0, cx);
        });

        let selection = if focused {
            theme.selection_focused
        } else {
            theme.selection
        };
        let mut rows_out = Vec::with_capacity(snapshot.rows);
        for row in 0..snapshot.rows.min(rows) {
            let text_end = (0..snapshot.cols)
                .rev()
                .find(|c| {
                    snapshot
                        .cell(row, *c)
                        .is_some_and(|cell| cell.c != ' ' || cell.flags.spacer)
                })
                .map(|c| c + 1)
                .unwrap_or(0);
            let mut runs: Vec<RunLayout> = Vec::new();
            let mut backgrounds: Vec<(Range<usize>, Hsla)> = Vec::new();
            let mut text = String::new();
            let mut run_col = 0;
            let mut style: Option<RunStyle> = None;
            let mut bg_run: Option<(usize, Hsla)> = None;
            for col in 0..snapshot.cols.min(cols) {
                let Some(cell) = snapshot.cell(row, col) else {
                    break;
                };
                // Backgrounds: a run per colour, the selection over them.
                let bg = if cell.selected && col < text_end {
                    Some(selection)
                } else if cell.bg != Color::Background {
                    Some(resolve(cell.bg, &theme, false))
                } else {
                    None
                };
                match (bg_run, bg) {
                    (Some((start, colour)), Some(b)) if colour == b => {
                        let _ = start;
                    }
                    (Some((start, colour)), other) => {
                        backgrounds.push((start..col, colour));
                        bg_run = other.map(|b| (col, b));
                    }
                    (None, Some(b)) => bg_run = Some((col, b)),
                    (None, None) => {}
                }
                // Text: a run per style; spacers and hidden cells draw nothing.
                let mut colour = resolve(cell.fg, &theme, true);
                if cell.flags.dim {
                    colour.a *= 0.6;
                }
                let cell_style = RunStyle {
                    color: colour,
                    bold: cell.flags.bold,
                    italic: cell.flags.italic,
                    underline: cell.flags.underline,
                    strikeout: cell.flags.strikeout,
                };
                let glyph = if cell.flags.spacer || cell.flags.hidden {
                    None
                } else {
                    Some(cell.c)
                };
                if style != Some(cell_style) || cell.flags.wide || glyph.is_none() {
                    shape_run(
                        &mut runs, &mut text, style, run_col, &theme, &settings, font_size, window,
                    );
                    style = Some(cell_style);
                    run_col = col;
                }
                match glyph {
                    Some(c) => text.push(c),
                    None => {
                        // The run ends here; the next glyph starts a new one.
                        shape_run(
                            &mut runs, &mut text, style, run_col, &theme, &settings, font_size,
                            window,
                        );
                        style = None;
                    }
                }
                if cell.flags.wide {
                    shape_run(
                        &mut runs, &mut text, style, run_col, &theme, &settings, font_size, window,
                    );
                    style = None;
                }
            }
            shape_run(
                &mut runs, &mut text, style, run_col, &theme, &settings, font_size, window,
            );
            if let Some((start, colour)) = bg_run {
                backgrounds.push((start..snapshot.cols.min(cols), colour));
            }
            rows_out.push(RowLayout { runs, backgrounds });
        }

        let cursor = snapshot
            .cursor
            .filter(|c| c.shape != CursorShape::Hidden && c.row < rows && c.col < cols)
            .map(|c| {
                let width = if c.wide { cell_width * 2. } else { cell_width };
                let cell_bounds = Bounds::new(
                    point(
                        origin.x + cell_width * c.col as f32,
                        origin.y + line_height * c.row as f32,
                    ),
                    size(width, line_height),
                );
                // A block caret shows its character in the island's colour.
                let glyph = snapshot
                    .cell(c.row, c.col)
                    .filter(|cell| c.shape == CursorShape::Block && focused && cell.c != ' ')
                    .map(|cell| {
                        let font =
                            terminal_font(&theme, &settings, cell.flags.bold, cell.flags.italic);
                        let s = cell.c.to_string();
                        window.text_system().shape_line(
                            SharedString::from(s.clone()),
                            font_size,
                            &[TextRun {
                                len: s.len(),
                                font,
                                color: theme.island_background,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        )
                    });
                (cell_bounds, c.shape, glyph)
            });

        Prepaint {
            hitbox,
            rows: rows_out,
            cursor,
            origin,
            cell: (cell_width, line_height),
            focused,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let focus_handle = self.view.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);
        let (cell_width, line_height) = prepaint.cell;
        let origin = prepaint.origin;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (row, layout) in prepaint.rows.iter().enumerate() {
                let y = origin.y + line_height * row as f32;
                for (cols, colour) in &layout.backgrounds {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(origin.x + cell_width * cols.start as f32, y),
                            size(cell_width * (cols.end - cols.start) as f32, line_height),
                        ),
                        *colour,
                    ));
                }
            }
            if let Some((cell_bounds, shape, _)) = &prepaint.cursor {
                let caret = theme.editor_caret;
                let thin = px(m.caret_width.max(1.));
                if !prepaint.focused {
                    // An unfocused terminal shows a hollow caret (estimate).
                    outline(window, *cell_bounds, caret, px(1.));
                } else {
                    match shape {
                        CursorShape::Block => window.paint_quad(fill(*cell_bounds, caret)),
                        CursorShape::HollowBlock => outline(window, *cell_bounds, caret, px(1.)),
                        CursorShape::Beam => window.paint_quad(fill(
                            Bounds::new(cell_bounds.origin, size(thin, cell_bounds.size.height)),
                            caret,
                        )),
                        CursorShape::Underline => window.paint_quad(fill(
                            Bounds::new(
                                point(
                                    cell_bounds.origin.x,
                                    cell_bounds.origin.y + cell_bounds.size.height - thin,
                                ),
                                size(cell_bounds.size.width, thin),
                            ),
                            caret,
                        )),
                        CursorShape::Hidden => {}
                    }
                }
            }
            for (row, layout) in prepaint.rows.iter().enumerate() {
                let y = origin.y + line_height * row as f32;
                for run in &layout.runs {
                    let _ = run.line.paint(
                        point(origin.x + cell_width * run.col as f32, y),
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
            if let Some((cell_bounds, CursorShape::Block, Some(glyph))) = &prepaint.cursor {
                if prepaint.focused {
                    let _ = glyph.paint(
                        cell_bounds.origin,
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
        });

        let hitbox = prepaint.hitbox.clone();
        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                view.update(cx, |v, cx| v.mouse_down(event, window, cx));
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                view.update(cx, |v, cx| v.mouse_move(event, window, cx));
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                view.update(cx, |v, cx| v.mouse_up(event, window, cx));
            }
        });
        let hitbox = prepaint.hitbox.clone();
        let view = self.view.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                view.update(cx, |v, cx| v.scroll_wheel(event, window, cx));
                cx.stop_propagation();
            }
        });
    }
}

/// Shapes the pending run and starts the next one.
#[allow(clippy::too_many_arguments)]
fn shape_run(
    runs: &mut Vec<RunLayout>,
    text: &mut String,
    style: Option<RunStyle>,
    col: usize,
    theme: &Theme,
    settings: &oak_settings::Settings,
    font_size: Pixels,
    window: &mut Window,
) {
    let Some(s) = style else {
        text.clear();
        return;
    };
    if text.trim().is_empty() && !s.underline && !s.strikeout {
        text.clear();
        return;
    }
    let shaped: String = std::mem::take(text);
    let font = terminal_font(theme, settings, s.bold, s.italic);
    let line = window.text_system().shape_line(
        SharedString::from(shaped.clone()),
        font_size,
        &[TextRun {
            len: shaped.len(),
            font,
            color: s.color,
            background_color: None,
            underline: s.underline.then(|| UnderlineStyle {
                thickness: px(1.),
                color: Some(s.color),
                wavy: false,
            }),
            strikethrough: s.strikeout.then(|| StrikethroughStyle {
                thickness: px(1.),
                color: Some(s.color),
            }),
        }],
        None,
    );
    runs.push(RunLayout { col, line });
}

/// A 1 px frame inside `bounds`.
fn outline(window: &mut Window, bounds: Bounds<Pixels>, colour: Hsla, width: Pixels) {
    let Bounds { origin, size: sz } = bounds;
    window.paint_quad(fill(Bounds::new(origin, size(sz.width, width)), colour));
    window.paint_quad(fill(
        Bounds::new(
            point(origin.x, origin.y + sz.height - width),
            size(sz.width, width),
        ),
        colour,
    ));
    window.paint_quad(fill(Bounds::new(origin, size(width, sz.height)), colour));
    window.paint_quad(fill(
        Bounds::new(
            point(origin.x + sz.width - width, origin.y),
            size(width, sz.height),
        ),
        colour,
    ));
}
