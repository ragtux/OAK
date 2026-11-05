//! A shell on a PTY and its grid. Alacritty's event loop runs on its own
//! thread, reading the PTY into the grid; [`Terminal`] is the view's side:
//! it writes keys, reads a [`Snapshot`] to paint, scrolls, selects, and
//! drains Alacritty's events (title, bell, exit) into plain fields.

use crate::profile::Profile;
use alacritty_terminal::event::{Event, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{self, Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{
    ClearMode, Color as AnsiColor, CursorShape as AnsiCursorShape, CursorStyle, Handler, NamedColor,
};
use anyhow::{Context, Result};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;

/// The grid's size: cells, and the cell's pixel size the PTY reports to
/// programs that ask (`TIOCGWINSZ`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: usize,
    pub rows: usize,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Size {
    fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows.max(term::MIN_SCREEN_LINES) as u16,
            num_cols: self.cols.max(term::MIN_COLUMNS) as u16,
            cell_width: self.cell_width.max(1),
            cell_height: self.cell_height.max(1),
        }
    }

    fn term_size(self) -> TermSize {
        TermSize::new(
            self.cols.max(term::MIN_COLUMNS),
            self.rows.max(term::MIN_SCREEN_LINES),
        )
    }
}

/// The cursor as a terminal program set it (DECSCUSR), or the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
    Hidden,
}

impl From<AnsiCursorShape> for CursorShape {
    fn from(s: AnsiCursorShape) -> CursorShape {
        match s {
            AnsiCursorShape::Block => CursorShape::Block,
            AnsiCursorShape::Underline => CursorShape::Underline,
            AnsiCursorShape::Beam => CursorShape::Beam,
            AnsiCursorShape::HollowBlock => CursorShape::HollowBlock,
            AnsiCursorShape::Hidden => CursorShape::Hidden,
        }
    }
}

impl From<CursorShape> for AnsiCursorShape {
    fn from(s: CursorShape) -> AnsiCursorShape {
        match s {
            CursorShape::Block => AnsiCursorShape::Block,
            CursorShape::Underline => AnsiCursorShape::Underline,
            CursorShape::Beam => AnsiCursorShape::Beam,
            CursorShape::HollowBlock => AnsiCursorShape::HollowBlock,
            CursorShape::Hidden => AnsiCursorShape::Hidden,
        }
    }
}

/// A cell's colour: the theme's defaults and ANSI entries, the 256-colour
/// table (see [`crate::colors::indexed`]) or a true colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// `text.primary` (the terminal's default text colour).
    Foreground,
    /// The island's background.
    Background,
    /// One of the 16 ANSI colours, `terminal.ansiColors.*.ansiX`.
    Ansi(u8),
    /// Palette index 16–255.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// A cell's text attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellFlags {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    pub inverse: bool,
    pub hidden: bool,
    /// A two-column character; the cell after it is a spacer.
    pub wide: bool,
    /// The second column of a wide character: nothing to draw.
    pub spacer: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShape,
    /// The character under it spans two columns.
    pub wide: bool,
}

/// The visible grid, row-major, `cols` cells per row.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    /// The cursor when it is on screen (not scrolled away).
    pub cursor: Option<Cursor>,
    /// Lines scrolled back from the bottom.
    pub display_offset: usize,
    /// Lines of scrollback above the screen.
    pub history: usize,
    pub modes: Modes,
}

impl Snapshot {
    pub fn cell(&self, row: usize, col: usize) -> Option<&Cell> {
        self.cells.get(row * self.cols + col)
    }

    /// The text of a row, trailing spaces trimmed (tests and the tooltip).
    pub fn row_text(&self, row: usize) -> String {
        let text: String = (0..self.cols)
            .filter_map(|c| self.cell(row, c))
            .filter(|c| !c.flags.spacer)
            .map(|c| c.c)
            .collect();
        text.trim_end().to_string()
    }
}

/// The terminal modes the view acts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    pub alt_screen: bool,
    pub focus_in_out: bool,
    /// The wheel sends arrow keys to a full-screen program.
    pub alternate_scroll: bool,
    pub mouse: bool,
}

impl Modes {
    fn from(mode: TermMode) -> Modes {
        Modes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            focus_in_out: mode.contains(TermMode::FOCUS_IN_OUT),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            mouse: mode.intersects(TermMode::MOUSE_MODE),
        }
    }
}

/// What a mouse selection takes: a click, a double click, a triple click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    Simple,
    Word,
    Line,
}

/// Alacritty's listener: every event goes to the channel and wakes the view.
#[derive(Clone)]
struct Listener {
    tx: mpsc::Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(event);
        (self.wake)();
    }
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    events: mpsc::Receiver<Event>,
    config: Config,
    size: Size,
    /// The profile that started the shell.
    pub profile: Profile,
    /// The title the program set (OSC 0/2), if any.
    pub title: Option<String>,
    /// The shell's exit code once it has ended.
    pub exited: Option<i32>,
    /// Bells rung since the last [`take_bells`](Self::take_bells).
    bells: usize,
    /// Text a program asked to put on the clipboard (OSC 52).
    clipboard: Option<String>,
    /// The PTY produced output since the last [`take_output`](Self::take_output).
    output: bool,
    /// The program's process id, when the platform tells it.
    pid: Option<u32>,
}

impl Terminal {
    /// Starts `profile`'s shell in `cwd` on a PTY of `size`, keeping
    /// `scrollback` lines (`terminal.lineCapacity`). `wake` runs on
    /// Alacritty's thread whenever there is something new to draw.
    pub fn spawn(
        profile: &Profile,
        cwd: Option<&Path>,
        size: Size,
        scrollback: usize,
        cursor: CursorShape,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Terminal> {
        Self::spawn_with_env(profile, cwd, &[], size, scrollback, cursor, wake)
    }

    /// [`Terminal::spawn`] with `env` set for the program over the app's
    /// own environment (a debug adapter's `runInTerminal`).
    pub fn spawn_with_env(
        profile: &Profile,
        cwd: Option<&Path>,
        extra_env: &[(String, String)],
        size: Size,
        scrollback: usize,
        cursor: CursorShape,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Terminal> {
        let (tx, events) = mpsc::channel();
        let listener = Listener { tx, wake };
        let config = Config {
            scrolling_history: scrollback,
            default_cursor_style: CursorStyle {
                shape: cursor.into(),
                blinking: false,
            },
            ..Config::default()
        };
        let term = Term::new(config.clone(), &size.term_size(), listener.clone());
        let term = Arc::new(FairMutex::new(term));

        let mut env = HashMap::new();
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "WEEP".to_string());
        if !cfg!(windows) {
            env.insert("TERM".to_string(), "xterm-256color".to_string());
        }
        for (k, v) in extra_env {
            env.insert(k.clone(), v.clone());
        }
        let options = tty::Options {
            shell: Some(tty::Shell::new(
                profile.program.clone(),
                profile.args.clone(),
            )),
            working_directory: cwd.map(Path::to_path_buf),
            drain_on_exit: false,
            env,
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, size.window_size(), 0)
            .with_context(|| format!("starting {}", profile.program))?;
        #[cfg(unix)]
        let pid = Some(pty.child().id());
        #[cfg(windows)]
        let pid = pty.child_watcher().pid().map(|p| p.get());
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)
            .context("the terminal's event loop")?;
        let notifier = Notifier(event_loop.channel());
        let _thread = event_loop.spawn();

        Ok(Terminal {
            term,
            notifier,
            events,
            config,
            size,
            profile: profile.clone(),
            title: None,
            exited: None,
            bells: 0,
            clipboard: None,
            output: false,
            pid,
        })
    }

    /// The program's process id (`None` where the platform does not say).
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Drains Alacritty's events; true when the grid may have changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Wakeup => {
                    self.output = true;
                    changed = true;
                }
                Event::CursorBlinkingChange | Event::MouseCursorDirty => changed = true,
                Event::Title(title) => {
                    self.title = Some(title);
                    changed = true;
                }
                Event::ResetTitle => {
                    self.title = None;
                    changed = true;
                }
                Event::Bell => self.bells += 1,
                Event::Exit => {
                    self.exited.get_or_insert(0);
                    changed = true;
                }
                Event::ChildExit(code) => {
                    self.exited = Some(code);
                    changed = true;
                }
                Event::PtyWrite(text) => self.notifier.notify(text.into_bytes()),
                Event::ClipboardStore(_, text) => self.clipboard = Some(text),
                Event::TextAreaSizeRequest(format) => {
                    let reply = format(self.size.window_size());
                    self.notifier.notify(reply.into_bytes());
                }
                _ => {}
            }
        }
        changed
    }

    /// Whether the shell wrote anything since the last call.
    pub fn take_output(&mut self) -> bool {
        std::mem::take(&mut self.output)
    }

    /// Bells rung since the last call (`terminal.bell`).
    pub fn take_bells(&mut self) -> usize {
        std::mem::take(&mut self.bells)
    }

    /// Text a program put on the clipboard, once.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// Writes bytes to the shell and scrolls back to the bottom, as typing does.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.term.lock().scroll_display(Scroll::Bottom);
        self.notifier.notify(bytes);
    }

    /// Pastes text: bracketed when the program asked for it, else with its
    /// newlines as carriage returns, as a terminal would.
    pub fn paste(&self, text: &str) {
        if self.modes().bracketed_paste {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.replace("\x1b[201~", "").as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            self.write(bytes);
        } else {
            self.write(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn resize(&mut self, size: Size) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.term.lock().resize(size.term_size());
        self.notifier.on_resize(size.window_size());
    }

    /// `terminal.lineCapacity` or the caret shape changed.
    pub fn set_options(&mut self, scrollback: usize, cursor: CursorShape) {
        self.config.scrolling_history = scrollback;
        self.config.default_cursor_style = CursorStyle {
            shape: cursor.into(),
            blinking: false,
        };
        self.term.lock().set_options(self.config.clone());
    }

    pub fn modes(&self) -> Modes {
        Modes::from(*self.term.lock().mode())
    }

    /// The window gained or lost the focus (programs that asked are told).
    pub fn focus_changed(&self, focused: bool) {
        if self.modes().focus_in_out {
            self.notifier.notify(if focused {
                b"\x1b[I".as_slice()
            } else {
                b"\x1b[O".as_slice()
            });
        }
    }

    pub fn scroll_lines(&self, delta: i32) {
        self.term.lock().scroll_display(Scroll::Delta(delta));
    }

    pub fn scroll_page_up(&self) {
        self.term.lock().scroll_display(Scroll::PageUp);
    }

    pub fn scroll_page_down(&self) {
        self.term.lock().scroll_display(Scroll::PageDown);
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    /// `terminal/clear`: the scrollback goes, and the shell redraws its
    /// prompt on the form feed (Ctrl+L to the line editors).
    pub fn clear(&self) {
        {
            let mut term = self.term.lock();
            term.grid_mut().clear_history();
            term.clear_screen(ClearMode::All);
            term.goto(0, 0);
        }
        self.write(b"\x0c".as_slice());
    }

    /// Starts a selection at a viewport cell; `right_half` when the press
    /// was in the cell's right half.
    pub fn selection_start(&self, row: usize, col: usize, right_half: bool, kind: SelectionKind) {
        let mut term = self.term.lock();
        let point = self.viewport_point(&term, row, col);
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        let side = if right_half { Side::Right } else { Side::Left };
        term.selection = Some(Selection::new(ty, point, side));
    }

    pub fn selection_update(&self, row: usize, col: usize, right_half: bool) {
        let mut term = self.term.lock();
        let point = self.viewport_point(&term, row, col);
        let side = if right_half { Side::Right } else { Side::Left };
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn select_all(&self) {
        let mut term = self.term.lock();
        let top = Point::new(term.topmost_line(), Column(0));
        let bottom = Point::new(term.bottommost_line(), term.last_column());
        let mut selection = Selection::new(SelectionType::Lines, top, Side::Left);
        selection.update(bottom, Side::Right);
        term.selection = Some(selection);
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    pub fn has_selection(&self) -> bool {
        self.term
            .lock()
            .selection
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }

    /// The selected text, if there is a selection.
    pub fn selection_text(&self) -> Option<String> {
        self.term
            .lock()
            .selection_to_string()
            .filter(|s| !s.is_empty())
    }

    fn viewport_point(&self, term: &Term<Listener>, row: usize, col: usize) -> Point {
        let row = row.min(term.screen_lines().saturating_sub(1));
        let col = col.min(term.columns().saturating_sub(1));
        term::viewport_to_point(term.grid().display_offset(), Point::new(row, Column(col)))
    }

    /// The visible grid as cells with resolved colours.
    pub fn snapshot(&self) -> Snapshot {
        let term = self.term.lock();
        let content = term.renderable_content();
        let cols = term.columns();
        let rows = term.screen_lines();
        let display_offset = content.display_offset;
        let colors = content.colors;
        let selection = content.selection;
        let mut cells = vec![
            Cell {
                c: ' ',
                fg: Color::Foreground,
                bg: Color::Background,
                flags: CellFlags::default(),
                selected: false,
            };
            cols * rows
        ];
        for indexed in content.display_iter {
            let Some(point) = term::point_to_viewport(display_offset, indexed.point) else {
                continue;
            };
            if point.line >= rows || point.column.0 >= cols {
                continue;
            }
            let cell = &indexed.cell;
            let flags = cell.flags;
            let mut fg = resolve(cell.fg, colors, true);
            let mut bg = resolve(cell.bg, colors, false);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            cells[point.line * cols + point.column.0] = Cell {
                c: cell.c,
                fg,
                bg,
                flags: CellFlags {
                    bold: flags.intersects(Flags::BOLD),
                    dim: flags.intersects(Flags::DIM),
                    italic: flags.intersects(Flags::ITALIC),
                    underline: flags.intersects(Flags::ALL_UNDERLINES),
                    strikeout: flags.contains(Flags::STRIKEOUT),
                    inverse: flags.contains(Flags::INVERSE),
                    hidden: flags.contains(Flags::HIDDEN),
                    wide: flags.contains(Flags::WIDE_CHAR),
                    spacer: flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
                },
                selected: selection.is_some_and(|s| s.contains(indexed.point)),
            };
        }
        let cursor = term::point_to_viewport(display_offset, content.cursor.point)
            .filter(|p| p.line < rows && p.column.0 < cols)
            .map(|p| Cursor {
                row: p.line,
                col: p.column.0,
                shape: content.cursor.shape.into(),
                wide: cells[p.line * cols + p.column.0].flags.wide,
            });
        Snapshot {
            cols,
            rows,
            cells,
            cursor,
            display_offset,
            history: term.history_size(),
            modes: Modes::from(content.mode),
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

/// A cell's colour as the theme will paint it: the program's own palette
/// entries (OSC 4) win, the 16 ANSI names map to Fleet's keys, the rest of
/// the 256 is computed, true colour passes through.
fn resolve(color: AnsiColor, colors: &term::color::Colors, foreground: bool) -> Color {
    let rgb = |rgb: alacritty_terminal::vte::ansi::Rgb| Color::Rgb(rgb.r, rgb.g, rgb.b);
    match color {
        AnsiColor::Spec(c) => rgb(c),
        AnsiColor::Indexed(i) => match colors[i as usize] {
            Some(c) => rgb(c),
            None if i < 16 => Color::Ansi(i),
            None => Color::Indexed(i),
        },
        AnsiColor::Named(name) => {
            if let Some(c) = colors[name] {
                return rgb(c);
            }
            match name {
                NamedColor::Foreground | NamedColor::BrightForeground => Color::Foreground,
                NamedColor::Background => Color::Background,
                NamedColor::Cursor => {
                    if foreground {
                        Color::Background
                    } else {
                        Color::Foreground
                    }
                }
                NamedColor::DimForeground => Color::Foreground,
                NamedColor::DimBlack => Color::Ansi(0),
                NamedColor::DimRed => Color::Ansi(1),
                NamedColor::DimGreen => Color::Ansi(2),
                NamedColor::DimYellow => Color::Ansi(3),
                NamedColor::DimBlue => Color::Ansi(4),
                NamedColor::DimMagenta => Color::Ansi(5),
                NamedColor::DimCyan => Color::Ansi(6),
                NamedColor::DimWhite => Color::Ansi(7),
                other => {
                    let index = other as usize;
                    if index < 16 {
                        Color::Ansi(index as u8)
                    } else {
                        Color::Foreground
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell this machine has for the PTY tests.
    fn shell() -> Option<Profile> {
        let all = Profile::discover();
        Profile::default_of(None, &all).cloned()
    }

    fn wait_for(terminal: &mut Terminal, mut pred: impl FnMut(&Snapshot) -> bool) -> Snapshot {
        let start = std::time::Instant::now();
        loop {
            terminal.pump();
            let snap = terminal.snapshot();
            if pred(&snap) || start.elapsed() > std::time::Duration::from_secs(20) {
                return snap;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// A debug adapter's `runInTerminal`: a program with its own
    /// environment, its pid known.
    #[cfg(unix)]
    #[test]
    fn a_command_with_its_environment_and_pid() {
        let profile = Profile::new(
            "probe",
            "/bin/sh",
            vec!["-c".into(), "echo \"weep-$WEEP_RIT_PROBE\"; sleep 5".into()],
        );
        let size = Size {
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        };
        let mut terminal = Terminal::spawn_with_env(
            &profile,
            None,
            &[("WEEP_RIT_PROBE".into(), "42".into())],
            size,
            100,
            CursorShape::Block,
            Arc::new(|| {}),
        )
        .expect("spawn");
        let pid = terminal.pid().expect("a pid");
        assert!(pid > 0);
        let snap = wait_for(&mut terminal, |s| {
            (0..s.rows).any(|r| s.row_text(r).contains("weep-42"))
        });
        assert!(
            (0..snap.rows).any(|r| snap.row_text(r).contains("weep-42")),
            "{:?}",
            (0..snap.rows).map(|r| snap.row_text(r)).collect::<Vec<_>>()
        );
        // The pid is the program's.
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|s| s.success());
        assert!(alive);
    }

    #[test]
    fn echoes_through_a_shell() {
        let Some(profile) = shell() else {
            eprintln!("no shell on this machine; skipping");
            return;
        };
        let size = Size {
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        };
        let mut terminal = Terminal::spawn(
            &profile,
            None,
            size,
            1000,
            CursorShape::Block,
            Arc::new(|| {}),
        )
        .expect("spawn");
        terminal.write(b"echo weep-probe-7\r".as_slice());
        let snap = wait_for(&mut terminal, |s| {
            (0..s.rows).any(|r| {
                let text = s.row_text(r);
                text.contains("weep-probe-7") && !text.contains("echo")
            })
        });
        assert!(
            (0..snap.rows).any(|r| {
                let text = snap.row_text(r);
                text.contains("weep-probe-7") && !text.contains("echo")
            }),
            "the shell did not echo: {:?}",
            (0..snap.rows).map(|r| snap.row_text(r)).collect::<Vec<_>>()
        );
        assert_eq!(snap.cols, 80);
        assert_eq!(snap.rows, 24);
        assert!(snap.cursor.is_some());

        terminal.select_all();
        assert!(terminal.selection_text().unwrap().contains("weep-probe-7"));
        terminal.clear_selection();
        assert!(!terminal.has_selection());

        terminal.write(b"exit\r".as_slice());
        let start = std::time::Instant::now();
        while terminal.exited.is_none() && start.elapsed() < std::time::Duration::from_secs(20) {
            terminal.pump();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(terminal.exited.is_some(), "the shell did not exit");
    }

    #[test]
    fn resolves_colours() {
        let colors = term::color::Colors::default();
        assert_eq!(
            resolve(AnsiColor::Named(NamedColor::Red), &colors, true),
            Color::Ansi(1)
        );
        assert_eq!(
            resolve(AnsiColor::Named(NamedColor::BrightBlue), &colors, true),
            Color::Ansi(12)
        );
        assert_eq!(
            resolve(AnsiColor::Named(NamedColor::Foreground), &colors, true),
            Color::Foreground
        );
        assert_eq!(
            resolve(AnsiColor::Indexed(3), &colors, true),
            Color::Ansi(3)
        );
        assert_eq!(
            resolve(AnsiColor::Indexed(200), &colors, true),
            Color::Indexed(200)
        );
        assert_eq!(
            resolve(
                AnsiColor::Spec(alacritty_terminal::vte::ansi::Rgb { r: 1, g: 2, b: 3 }),
                &colors,
                false
            ),
            Color::Rgb(1, 2, 3)
        );
    }
}
