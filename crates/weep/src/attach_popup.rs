//! Fleet's `debugger/attach-to-process` ("Opens dialog to attach the
//! debugger to the manually specified or discovered processes", M8): the
//! Goto frame titled "Attach to Process", a field ("Enter host:port or
//! search process"), a group header "LLDB" (the debugger the processes are
//! listed for), then a row per process, `pid name`, filtered by the Goto
//! matcher (`weep-search`'s fuzzy) on the name, or by a typed pid; a typed
//! pid that is not listed is offered too. Enter hands the chosen pid to the
//! caller, which starts an attach session (`weep_dap::Start::Attach`).
//!
//! Batch 13 (debug-fleet-attach, -attach-filtered) settled the wording, the
//! header, the rows (no command line, no count in the title) and that
//! every process is listed, root's and the system users' too. Fleet's
//! order follows no key the shot shows (not the pid, the name, the user,
//! the start time, CPU time or memory: 44055, 25496, 603, 11803, 1060...;
//! "zsh" left 67006, 35653, 37106, 62762, 42625), as a hash map's would;
//! WEEP keeps a stable one, newest pid first (estimate). A host:port typed
//! for a remote attach (Fleet's placeholder offers it) is not taken:
//! lldb-dap reaches a remote only through `gdb-remote` attach commands,
//! which `weep_dap::Start::Attach` does not carry.
//!
//! The processes come from `ps -A` on macOS and Linux and `tasklist` on
//! Windows; the parsing is plain functions, tested.

use crate::actions::{PopupConfirm, PopupDown, PopupPageDown, PopupPageUp, PopupUp};
use crate::input::{InputEvent, TextInput};
use crate::list_popup::{tui_cols, tui_input_row, tui_row, tui_scrollbar, tui_top, TuiRow};
use crate::search_tool::frame;
use crate::skin::{self, Cell, RowScroll, Skin};
use crate::theme::{Metrics, Theme};
use gpui::{
    deferred, div, point, prelude::*, px, AnyElement, App, BoxShadow, Context, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, Render, ScrollHandle,
    SharedString, StyledText, Task, Window,
};
use std::ops::Range;
use std::rc::Rc;
use weep_search::fuzzy_match;

/// What runs when a process is chosen: its pid and its name.
pub type OnAttach = Rc<dyn Fn(u32, String, &mut Window, &mut App)>;

/// The field's placeholder (batch 13).
const PLACEHOLDER: &str = "Enter host:port or search process";

/// The group header over the rows: the debugger they are listed for
/// (batch 13: "LLDB"; WEEP's adapter is lldb-dap).
const GROUP: &str = "LLDB";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachPopupEvent {
    Close,
}

/// A process the user may attach to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    /// The executable's name.
    pub name: String,
}

// ---- listing ---------------------------------------------------------------------

/// `ps -o pid=,<column>=` output: each line's pid and the rest, trimmed.
/// Lines without a pid are skipped.
pub fn parse_pid_lines(output: &str) -> Vec<(u32, String)> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let end = line
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(line.len());
            let pid = line[..end].parse().ok()?;
            Some((pid, line[end..].trim().to_string()))
        })
        .collect()
}

/// The processes from `ps -o pid=,comm=`: a name given as a path keeps its
/// last part; a line without a name is dropped.
pub fn parse_ps(output: &str) -> Vec<Process> {
    parse_pid_lines(output)
        .into_iter()
        .filter_map(|(pid, name)| {
            let name = name.rsplit('/').next().unwrap_or(&name).to_string();
            (!name.is_empty()).then_some(Process { pid, name })
        })
        .collect()
}

/// One CSV line's fields (`tasklist /fo csv`: every field quoted, `""` a
/// quote inside one).
fn csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    fields.push(field);
    fields
}

/// `tasklist /fo csv /nh` output: `"Image Name","PID","Session Name",
/// "Session#","Mem Usage"` rows.
pub fn parse_tasklist(output: &str) -> Vec<Process> {
    output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let f = csv_fields(line.trim());
            let pid = f.get(1)?.trim().parse().ok()?;
            let name = f.first()?.trim().to_string();
            (!name.is_empty()).then_some(Process { pid, name })
        })
        .collect()
}

fn run(program: &str, args: &[&str]) -> Result<(u32, String), String> {
    let child = std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let pid = child.id();
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok((pid, String::from_utf8_lossy(&out.stdout).into_owned()))
}

/// Every process (batch 13: Fleet lists root's and the system users' too),
/// newest (highest pid) first, without WEEP itself, the lister and pid 0.
pub fn list_processes() -> Result<Vec<Process>, String> {
    let mut skip = vec![0, std::process::id()];
    let mut processes = if cfg!(windows) {
        let (pid, out) = run("tasklist", &["/fo", "csv", "/nh"])?;
        skip.push(pid);
        parse_tasklist(&out)
    } else {
        // macOS's `-c` gives the executable's whole name ("Google Chrome
        // Helper (Renderer)"); Linux's `comm` is the name already (and its
        // `-c` means something else).
        let args: &[&str] = if cfg!(target_os = "macos") {
            &["-Aco", "pid=,comm="]
        } else {
            &["-Ao", "pid=,comm="]
        };
        let (pid, out) = run("ps", args)?;
        skip.push(pid);
        parse_ps(&out)
    };
    processes.retain(|p| !skip.contains(&p.pid));
    processes.sort_by_key(|p| std::cmp::Reverse(p.pid));
    Ok(processes)
}

// ---- filtering ---------------------------------------------------------------------

/// A pid typed into the field (digits only, not zero).
pub fn typed_pid(query: &str) -> Option<u32> {
    let q = query.trim();
    if q.is_empty() || !q.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    q.parse().ok().filter(|p| *p != 0)
}

/// A row of the list: a listed process (its index, the matched chars of
/// its name), or a typed pid that matches none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Process { ix: usize, positions: Vec<usize> },
    Typed(u32),
}

/// The rows for `query`, best first: a typed pid matches its process
/// exactly first, then pids starting with it; otherwise the name through
/// the Goto matcher (batch 13: "zsh" marks the name alone). An empty query
/// lists everything in the given order. At most `limit` rows.
pub fn filter_processes(processes: &[Process], query: &str, limit: usize) -> Vec<Row> {
    let q = query.trim();
    if q.is_empty() {
        return (0..processes.len().min(limit))
            .map(|ix| Row::Process {
                ix,
                positions: Vec::new(),
            })
            .collect();
    }
    let mut scored: Vec<(i32, usize, Vec<usize>)> = Vec::new();
    let pid = typed_pid(q);
    for (ix, p) in processes.iter().enumerate() {
        if let Some(pid) = pid {
            let text = p.pid.to_string();
            if p.pid == pid {
                scored.push((i32::MAX, ix, Vec::new()));
                continue;
            }
            if text.starts_with(q) {
                scored.push((i32::MAX / 2 - text.len() as i32, ix, Vec::new()));
                continue;
            }
        }
        if let Some(m) = fuzzy_match(q, &p.name) {
            scored.push((m.score, ix, m.positions));
        }
    }
    // Best first; among equals the list's own order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut rows: Vec<Row> = Vec::new();
    if let Some(pid) = pid {
        if !processes.iter().any(|p| p.pid == pid) {
            rows.push(Row::Typed(pid));
        }
    }
    rows.extend(
        scored
            .into_iter()
            .map(|(_, ix, positions)| Row::Process { ix, positions }),
    );
    rows.truncate(limit);
    rows
}

/// A row's text, `pid name` (batch 13), and the char positions of the
/// name's matched chars in it.
pub fn row_text(pid: u32, name: &str, positions: &[usize]) -> (String, Vec<usize>) {
    let pid = pid.to_string();
    let shift = pid.chars().count() + 1;
    (
        format!("{pid} {name}"),
        positions.iter().map(|p| p + shift).collect(),
    )
}

/// Char indices to the byte ranges of `text` they cover, one per char.
fn char_ranges(text: &str, positions: &[usize]) -> Vec<Range<usize>> {
    text.char_indices()
        .enumerate()
        .filter(|(i, _)| positions.contains(i))
        .map(|(_, (b, c))| b..b + c.len_utf8())
        .collect()
}

/// Rows shown at most (estimate: enough for every process of a desktop
/// session; Fleet's list scrolls on past the 22 rows its frame shows).
const ROW_LIMIT: usize = 1000;

// ---- the popup ---------------------------------------------------------------------

pub struct AttachPopup {
    input: Entity<TextInput>,
    /// `None` while `ps` runs; the message when it failed.
    processes: Option<Result<Vec<Process>, String>>,
    rows: Vec<Row>,
    selected: usize,
    scroll: ScrollHandle,
    tui_scroll: RowScroll,
    focus_handle: FocusHandle,
    on_attach: OnAttach,
    _load: Task<()>,
}

impl EventEmitter<AttachPopupEvent> for AttachPopup {}

impl Focusable for AttachPopup {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl AttachPopup {
    pub fn new(on_attach: OnAttach, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextInput::new(PLACEHOLDER, cx);
            input.tui_prompt = false;
            input
        });
        cx.subscribe(&input, Self::on_input).detach();
        let list = cx.background_executor().spawn(async { list_processes() });
        let load = cx.spawn(async move |this, cx| {
            let processes = list.await;
            let _ = this.update(cx, |this, cx| {
                this.processes = Some(processes);
                this.refilter(cx);
            });
        });
        AttachPopup {
            input,
            processes: None,
            rows: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            tui_scroll: RowScroll::default(),
            focus_handle: cx.focus_handle(),
            on_attach,
            _load: load,
        }
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    fn listed(&self) -> &[Process] {
        match &self.processes {
            Some(Ok(p)) => p,
            _ => &[],
        }
    }

    fn on_input(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => self.refilter(cx),
            InputEvent::Submit { .. } => self.choose(None, cx),
            InputEvent::Cancel => cx.emit(AttachPopupEvent::Close),
            _ => {}
        }
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.rows = filter_processes(self.listed(), &query, ROW_LIMIT);
        // A typed pid is offered even before the list arrives.
        if self.processes.is_none() {
            if let Some(pid) = typed_pid(&query) {
                self.rows = vec![Row::Typed(pid)];
            }
        }
        self.selected = 0;
        self.tui_scroll.reset();
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    fn step(&mut self, delta: isize, wrap: bool, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        let next = if wrap {
            (self.selected as isize + delta).rem_euclid(len)
        } else {
            (self.selected as isize + delta).clamp(0, len - 1)
        };
        self.selected = next as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn up(&mut self, _: &PopupUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, true, cx);
    }

    fn down(&mut self, _: &PopupDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, true, cx);
    }

    fn page_up(&mut self, _: &PopupPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-10, false, cx);
    }

    fn page_down(&mut self, _: &PopupPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(10, false, cx);
    }

    fn confirm(&mut self, _: &PopupConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.choose(None, cx);
    }

    /// Attaches to row `ix` (the selected one by default): the callback
    /// runs once the popup has closed, with the window it was in.
    fn choose(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        let ix = ix.unwrap_or(self.selected);
        let chosen = match self.rows.get(ix) {
            Some(Row::Process { ix, .. }) => {
                let p = &self.listed()[*ix];
                Some((p.pid, p.name.clone()))
            }
            Some(Row::Typed(pid)) => Some((*pid, format!("PID {pid}"))),
            None => typed_pid(&self.query(cx)).map(|pid| (pid, format!("PID {pid}"))),
        };
        let Some((pid, name)) = chosen else {
            return;
        };
        cx.emit(AttachPopupEvent::Close);
        let on_attach = self.on_attach.clone();
        let Some(window) = cx.active_window() else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| on_attach(pid, name, window, cx));
        });
    }

    fn status_line(&self) -> Option<(SharedString, bool)> {
        match &self.processes {
            None if self.rows.is_empty() => Some(("Listing processes…".into(), false)),
            Some(Err(e)) => Some((e.clone().into(), true)),
            _ if self.rows.is_empty() => Some(("Nothing found".into(), false)),
            _ => None,
        }
    }

    /// A row's text and the char positions to mark in it.
    fn row_texts(&self, row: &Row) -> (String, Vec<usize>) {
        match row {
            Row::Process { ix, positions } => {
                let p = &self.listed()[*ix];
                row_text(p.pid, &p.name, positions)
            }
            // WEEP's own row (estimate): a typed pid nobody lists.
            Row::Typed(pid) => (format!("{pid} Attach to PID {pid}"), Vec::new()),
        }
    }
}

impl Render for AttachPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Skin::current(cx) == Skin::Tui {
            return self.render_tui(window, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let viewport = window.viewport_size();
        let width = px(m.goto_width).min(viewport.width - px(2. * m.island_margin));
        let left = (viewport.width - width) / 2.;
        // Fleet's frame keeps its height while the list shrinks (batch 13:
        // 680 tall, filtered to five rows or not).
        let height = px(m.goto_max_height)
            .min(viewport.height - px(m.goto_top + m.island_margin))
            .max(px(0.));
        let field_focused = self.input.read(cx).is_focused(window);
        let entity = cx.entity();
        let rows: Vec<AnyElement> = self
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                let selected = ix == self.selected;
                let (text, positions) = self.row_texts(row);
                let highlights: Vec<(Range<usize>, HighlightStyle)> =
                    char_ranges(&text, &positions)
                        .into_iter()
                        .map(|r| {
                            (
                                r,
                                HighlightStyle {
                                    color: Some(theme.search_match_text),
                                    background_color: Some(theme.search_match_bg),
                                    ..Default::default()
                                },
                            )
                        })
                        .collect();
                let click = entity.clone();
                let hover = entity.clone();
                // A 24 px row on a 26 px pitch (batch 13: the selected box
                // 340-388 px, the rows 52 px apart), the Goto's.
                div()
                    .h(px(m.goto_row_pitch))
                    .flex_shrink_0()
                    .child(
                        div()
                            .id(ElementId::NamedInteger("attach-row".into(), ix as u64))
                            .h(px(m.goto_row_height))
                            .flex()
                            .items_center()
                            .px(px(m.popup_row_pad_x))
                            .rounded(px(m.popup_row_radius))
                            .cursor_pointer()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_color(if selected {
                                theme.list_text_focused
                            } else {
                                theme.list_text
                            })
                            .when(selected, |d| d.bg(theme.list_bg_focused))
                            .when(!selected, |d| d.hover(|s| s.bg(theme.list_bg_hovered)))
                            .on_mouse_move(move |_, _, cx| {
                                hover.update(cx, |this, cx| {
                                    if this.selected != ix {
                                        this.selected = ix;
                                        cx.notify();
                                    }
                                })
                            })
                            .on_click(move |_, _, cx| {
                                click.update(cx, |this, cx| this.choose(Some(ix), cx))
                            })
                            .child(
                                div().min_w_0().truncate().child(
                                    StyledText::new(SharedString::from(text))
                                        .with_highlights(highlights),
                                ),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        let status = self.status_line().map(|(text, error)| {
            div()
                .h(px(m.goto_row_pitch))
                .flex()
                .items_center()
                .px(px(m.popup_row_pad_x))
                .text_color(if error {
                    theme.text_dangerous
                } else {
                    theme.text_tertiary
                })
                .child(text)
        });
        // The group header over the rows (batch 13: "LLDB" in
        // `text.secondary`, smaller than the rows, its caps centred 13 under
        // the field's bottom edge plus the 8 gap).
        let header = (!self.rows.is_empty()).then(|| {
            div()
                .h(px(m.goto_header_height))
                .px(px(m.popup_row_pad_x))
                .flex_shrink_0()
                .flex()
                .items_center()
                .text_size(px(m.attach_header_font))
                .text_color(theme.text_secondary)
                .child(GROUP)
        });
        let close = entity.clone();
        deferred(
            div()
                .id("attach-popup")
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::page_up))
                .on_action(cx.listener(Self::page_down))
                .on_action(cx.listener(Self::confirm))
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |_, cx| cx.emit(AttachPopupEvent::Close));
                })
                .occlude()
                .absolute()
                .left(left)
                .top(px(m.goto_top))
                .w(width)
                .h(height)
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
                        .child("Attach to Process"),
                )
                .child(
                    div().px(px(m.goto_pad - 1.)).flex_shrink_0().flex().child(
                        frame(field_focused, window, &theme, &m)
                            .pl(px(m.goto_pad))
                            // The field is filled as the dialog under it
                            // (batch 13: the dialog's colour inside the
                            // focus frame), not as an island.
                            .when(field_focused, |d| {
                                d.bg(theme.popup_goto_bg.blend(theme.input_bg))
                            })
                            .child(div().flex_1().min_w_0().h_full().child(self.input.clone())),
                    ),
                )
                .child(
                    div()
                        .id("attach-rows")
                        .mt(px(m.run_popup_header_gap))
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .px(px(m.attach_rows_pad_x))
                        .pb(px(m.goto_bottom_pad))
                        .flex()
                        .flex_col()
                        .children(header)
                        .children(status)
                        .children(rows),
                ),
        )
        .into_any_element()
    }
}

// ---- the tui skin ------------------------------------------------------------------

impl AttachPopup {
    /// The dialog in the tui skin: a Goto box under the toolbar of the
    /// Fleet frame's height, the title in its top edge, the field on row 1,
    /// a rule, the `LLDB` header in `text_secondary`, then ` pid name` rows
    /// with the matched chars coloured; ` ↵ attach  esc close ` in the
    /// bottom edge.
    fn render_tui(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let m = *cx.global::<Metrics>();
        let cell = Cell::current(cx);
        let bg = theme.popup_goto_bg;
        let cols = tui_cols(m.goto_width, cell, window);
        let inner = cols - 2;
        let max_rows = crate::list_popup::tui_max_rows(cell, &m, window);
        // Field, rule, header: the rows' room is the rest of the fixed box.
        let visible = max_rows.saturating_sub(5).max(1);
        self.tui_scroll
            .follow(self.selected, self.rows.len(), visible);
        let top = self.tui_scroll.top;
        let entity = cx.entity();
        let mut body_rows: Vec<AnyElement> = Vec::new();
        if let Some((text, error)) = self.status_line() {
            body_rows.push(
                div()
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .pl(cell.cols(1))
                    .text_color(if error {
                        theme.text_dangerous
                    } else {
                        theme.text_tertiary
                    })
                    .child(SharedString::from(skin::fit_text(&text, inner - 2)))
                    .into_any_element(),
            );
        } else {
            body_rows.push(
                div()
                    .h(px(cell.h))
                    .flex_shrink_0()
                    .pl(cell.cols(1))
                    .text_color(theme.text_secondary)
                    .child(GROUP)
                    .into_any_element(),
            );
        }
        for (ix, row) in self.rows.iter().enumerate().skip(top).take(visible) {
            let (text, positions) = self.row_texts(row);
            let mut r = TuiRow::text(text.clone());
            r.highlights = char_ranges(&text, &positions);
            let click = entity.clone();
            body_rows.push(
                tui_row(
                    ElementId::NamedInteger("attach-row".into(), ix as u64),
                    r,
                    inner,
                    ix == self.selected,
                    cell,
                    &theme,
                )
                .on_click(move |_, _, cx| click.update(cx, |this, cx| this.choose(Some(ix), cx)))
                .into_any_element(),
            );
        }
        let field_focused = self.input.read(cx).is_focused(window);
        // The box keeps its height as the list shrinks, as Fleet's frame.
        let rows = 5 + visible;
        let n = self.rows.len();
        let body = div()
            .size_full()
            .flex()
            .flex_col()
            .on_scroll_wheel(cx.listener(move |this, e, _, cx| {
                if this.tui_scroll.wheel(e, cell, n, visible) {
                    cx.notify();
                }
            }))
            .child(tui_input_row(
                "›",
                field_focused,
                self.input.clone(),
                Vec::new(),
                &theme,
                cell,
            ))
            .child(div().h(px(cell.h)).flex_shrink_0())
            .children(body_rows)
            .into_any_element();
        let mut boxed = skin::boxed(
            cell,
            cols,
            rows,
            vec![skin::title("Attach to Process", theme.text_primary)],
            vec![2],
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
            skin::hints(&[("↵", "attach"), ("esc", "close")], &theme),
            bg,
        ));
        if n > visible {
            boxed = boxed.child(tui_scrollbar(
                cell,
                cols - 1,
                4,
                rows - 1,
                top,
                visible,
                n,
                &theme,
                m,
            ));
        }
        let close = entity;
        deferred(
            div()
                .id("attach-popup")
                .key_context("Popup")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::up))
                .on_action(cx.listener(Self::down))
                .on_action(cx.listener(Self::page_up))
                .on_action(cx.listener(Self::page_down))
                .on_action(cx.listener(Self::confirm))
                .on_mouse_down_out(move |_, _, cx| {
                    close.update(cx, |_, cx| cx.emit(AttachPopupEvent::Close));
                })
                .absolute()
                .left(skin::centred_x(window.viewport_size().width, cols, cell))
                .top(cell.rows(tui_top(cell, &m)))
                .child(boxed),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_names() {
        let p = parse_ps("    1 launchd\n  351 logd\n 4242 Google Chrome Helper (Renderer)\n");
        assert_eq!(p.len(), 3);
        assert_eq!(
            p[2],
            Process {
                pid: 4242,
                name: "Google Chrome Helper (Renderer)".into(),
            }
        );
        // Linux's `comm` can be a path's last part only; a path keeps its name.
        let p = parse_ps("  7 /usr/bin/python3\n  8 \n");
        assert_eq!(
            p,
            vec![Process {
                pid: 7,
                name: "python3".into()
            }]
        );
        assert!(parse_pid_lines("garbage\n\n  12\n")
            .iter()
            .all(|(pid, _)| *pid == 12));
    }

    #[test]
    fn tasklist_csv() {
        let out = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\
                   \"weep.exe\",\"10456\",\"Console\",\"1\",\"152,344 K\"\r\n\
                   \"odd \"\"name\"\".exe\",\"77\",\"Console\",\"1\",\"1 K\"\r\n\r\n";
        let p = parse_tasklist(out);
        assert_eq!(p.len(), 3);
        assert_eq!(p[1].pid, 10456);
        assert_eq!(p[1].name, "weep.exe");
        assert_eq!(p[2].name, "odd \"name\".exe");
        assert!(
            parse_tasklist("INFO: No tasks are running which match the specified criteria.")
                .is_empty()
        );
    }

    fn sample() -> Vec<Process> {
        vec![
            Process {
                pid: 4242,
                name: "probe-fixture".into(),
            },
            Process {
                pid: 4200,
                name: "zsh".into(),
            },
            Process {
                pid: 42,
                name: "python3".into(),
            },
        ]
    }

    #[test]
    fn pids_and_names_filter() {
        let p = sample();
        assert_eq!(filter_processes(&p, "", 10).len(), 3);
        assert_eq!(filter_processes(&p, "", 2).len(), 2);
        // The exact pid first, then pids starting with it.
        let rows = filter_processes(&p, "42", 10);
        assert_eq!(
            rows[0],
            Row::Process {
                ix: 2,
                positions: Vec::new()
            }
        );
        assert!(matches!(rows[1], Row::Process { ix: 0, .. }));
        assert!(matches!(rows[2], Row::Process { ix: 1, .. }));
        // A pid nobody has is offered as typed.
        let rows = filter_processes(&p, "99", 10);
        assert_eq!(rows, vec![Row::Typed(99)]);
        // Names through the fuzzy matcher, the name alone.
        let rows = filter_processes(&p, "fix", 10);
        assert!(matches!(&rows[0], Row::Process { ix: 0, positions } if positions.len() == 3));
        let rows = filter_processes(&p, "zsh", 10);
        assert_eq!(
            rows,
            vec![Row::Process {
                ix: 1,
                positions: vec![0, 1, 2]
            }]
        );
        assert!(filter_processes(&p, "nothing-like-it", 10).is_empty());
        // A host:port matches no name: nothing (no remote attach).
        assert!(filter_processes(&p, "localhost:1234", 10).is_empty());
    }

    #[test]
    fn rows_read_pid_then_name() {
        // batch 13: "67006 zsh", the name's match marked.
        let (text, marks) = row_text(67006, "zsh", &[0, 1, 2]);
        assert_eq!(text, "67006 zsh");
        assert_eq!(marks, vec![6, 7, 8]);
        assert_eq!(char_ranges(&text, &marks), vec![6..7, 7..8, 8..9]);
    }

    #[test]
    fn typed_pids() {
        assert_eq!(typed_pid(" 123 "), Some(123));
        assert_eq!(typed_pid("0"), None);
        assert_eq!(typed_pid("12a"), None);
        assert_eq!(typed_pid(""), None);
        assert_eq!(typed_pid("99999999999"), None);
    }

    #[test]
    fn highlights_are_byte_ranges() {
        assert_eq!(char_ranges("aéb", &[1, 2]), vec![1..3, 3..4]);
    }

    #[test]
    #[cfg(unix)]
    fn lists_every_users_processes() {
        let p = list_processes().expect("ps runs");
        assert!(!p.is_empty());
        assert!(p.iter().all(|p| p.pid != std::process::id()));
        assert!(p.windows(2).all(|w| w[0].pid > w[1].pid));
        // launchd (macOS) or init (Linux), root's: listed too.
        assert!(p.iter().any(|p| p.pid == 1));
    }
}
