//! WEEP: a native Rust/GPUI code editor in the shape of JetBrains Fleet.
//!
//! `weep [path ...] [--theme <name|file>] [--zoom <level>] [--skin fleet|tui] [--size WxH]`
//! opens a folder (or a file's folder, with the file in a tab) in the
//! Fleet-style shell: the toolbar, the tool panels, the editor splits with
//! their tabs, the status bar. Settings come from `~/.config/weep/settings.json`
//! over Fleet's own file; `--theme`, `--zoom` and `--skin` override them for the run.

// A release build on Windows is a GUI program, so launching it from Explorer opens
// no console window; `attach_parent_console` keeps `--help` and the diagnostics on
// stderr when it is started from a terminal. Debug builds stay console programs.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod actions;
mod assets;
mod dialog;
mod diff_view;
mod document;
mod drag_ghost;
mod editor;
mod file_tree;
mod find;
mod git_tool;
mod goto;
mod input;
mod keymap;
mod layout;
mod list_popup;
mod menu;
mod notifications;
mod popup;
mod run_popup;
mod run_tool;
mod scrollbar;
mod search_tool;
mod settings;
mod settings_schema;
mod settings_view;
mod skin;
mod terminal;
mod theme;
mod toolbar;
mod tooltip;
mod usages;
mod workspace;

use gpui::{
    point, px, size, App, AppContext as _, Bounds, KeyBinding, Menu, MenuItem, SystemMenuType,
    TitlebarOptions, WindowBounds, WindowOptions,
};
use std::path::PathBuf;
use theme::ThemeSource;
use workspace::Workspace;

struct Args {
    paths: Vec<PathBuf>,
    theme: Option<String>,
    zoom: Option<f32>,
    skin: Option<String>,
    size: Option<(f32, f32)>,
    /// `--scratch`: open a new scratch file, as Fleet's launcher does.
    scratch: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        paths: Vec::new(),
        theme: None,
        zoom: None,
        skin: None,
        size: None,
        scratch: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--theme" => args.theme = it.next(),
            "--zoom" => args.zoom = it.next().and_then(|z| z.parse().ok()),
            "--skin" => args.skin = it.next(),
            "--scratch" => args.scratch = true,
            "--size" => {
                args.size = it.next().and_then(|s| {
                    let (w, h) = s.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                })
            }
            "-h" | "--help" => {
                println!(
                    "usage: weep [path ...] [--theme dark|dark_purple|light|<file.json>] [--zoom 0.7..2.0] [--skin fleet|tui] [--size WxH] [--scratch]"
                );
                std::process::exit(0);
            }
            _ => args.paths.push(PathBuf::from(a)),
        }
    }
    args
}

/// Opens a workspace window on `root` with `open` in tabs: the first one at
/// startup, more for `show-recent-workspaces`.
pub fn open_workspace_window(
    root: PathBuf,
    open: Vec<PathBuf>,
    scratch: bool,
    window_size: (f32, f32),
    cx: &mut App,
) {
    let (width, height) = window_size;
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("WEEP".into()),
                // Fleet draws its own caption on Windows, and on macOS its
                // toolbar is the title bar with the traffic lights at its left
                // end (docs/fleet-probe.md, batch 1 and the fourteenth pass).
                appears_transparent: cfg!(windows) || cfg!(target_os = "macos"),
                traffic_light_position: cfg!(target_os = "macos").then(|| {
                    point(
                        px(theme::metrics::TRAFFIC_LIGHT_X),
                        px(theme::metrics::TRAFFIC_LIGHT_Y),
                    )
                }),
            }),
            // KWin ties an activation token to an application through this id.
            app_id: Some("weep".into()),
            window_min_size: Some(size(px(480.), px(320.))),
            ..Default::default()
        },
        |window, cx| {
            probe_keys(window, cx);
            cx.new(|cx| Workspace::new(root, open, scratch, window, cx))
        },
    );
    if let Err(e) = result {
        eprintln!("weep: cannot open a window: {e:#}");
    }
}

/// A mouse step of `WEEP_KEYS` as the events a real pointer would send.
fn probe_mouse(step: &str) -> Option<Vec<gpui::PlatformInput>> {
    use gpui::{
        point, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
        ScrollDelta, ScrollWheelEvent,
    };
    let (kind, args) = step.split_once(':')?;
    let n: Vec<f32> = args
        .split(';')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    let at = |i: usize| point(px(n[i]), px(n[i + 1]));
    let hover = |p, pressed_button| {
        PlatformInput::MouseMove(MouseMoveEvent {
            position: p,
            pressed_button,
            modifiers: Modifiers::default(),
        })
    };
    let press = |p, button| {
        PlatformInput::MouseDown(MouseDownEvent {
            button,
            position: p,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        })
    };
    let release = |p, button| {
        PlatformInput::MouseUp(MouseUpEvent {
            button,
            position: p,
            modifiers: Modifiers::default(),
            click_count: 1,
        })
    };
    Some(match (kind, n.len()) {
        ("move", 2) => vec![hover(at(0), None)],
        ("click", 2) => vec![
            hover(at(0), None),
            press(at(0), MouseButton::Left),
            release(at(0), MouseButton::Left),
        ],
        ("rclick", 2) => vec![
            hover(at(0), None),
            press(at(0), MouseButton::Right),
            release(at(0), MouseButton::Right),
        ],
        ("scroll", 3) => vec![
            hover(at(0), None),
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: at(0),
                delta: ScrollDelta::Pixels(point(px(0.), px(n[2]))),
                modifiers: Modifiers::default(),
                touch_phase: Default::default(),
            }),
        ],
        ("drag", 4) | ("hold", 4) => {
            let mut events = vec![hover(at(0), None), press(at(0), MouseButton::Left)];
            for i in 1..=8 {
                let t = i as f32 / 8.;
                let p = point(px(n[0] + (n[2] - n[0]) * t), px(n[1] + (n[3] - n[1]) * t));
                events.push(hover(p, Some(MouseButton::Left)));
            }
            if kind == "drag" {
                events.push(release(at(2), MouseButton::Left));
            }
            events
        }
        _ => return None,
    })
}

/// `WEEP_KEYS`: keystrokes sent to a new window once it is up, for the
/// screenshot probes on a machine where no input can be scripted. Comma
/// separated, in gpui's spelling (`cmd-shift-p`, `down`, `escape`; `comma`
/// stands for `,` as in `cmd-comma`); `type:text` types text, `wait:ms`
/// pauses (each step waits 300 ms). Mouse steps take window coordinates in
/// logical pixels, split by `;`: `move:x;y` hovers, `click:x;y`,
/// `rclick:x;y`, `scroll:x;y;dy` turns the wheel by `dy` pixels (negative
/// scrolls down) and `drag:x1;y1;x2;y2` presses, moves in steps and releases
/// (`hold:` the same without the release, to see what is dragged).
fn probe_keys(window: &mut gpui::Window, cx: &mut App) {
    let Ok(keys) = std::env::var("WEEP_KEYS") else {
        return;
    };
    window
        .spawn(cx, async move |cx| {
            let executor = cx.background_executor().clone();
            let pause = |ms| executor.timer(std::time::Duration::from_millis(ms));
            pause(1500).await;
            for step in keys.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                if let Some(ms) = step.strip_prefix("wait:") {
                    pause(ms.parse().unwrap_or(500)).await;
                    continue;
                }
                if let Some(events) = probe_mouse(step) {
                    for event in events {
                        let _ = cx.update(|window, cx| window.dispatch_event(event, cx));
                        pause(30).await;
                    }
                    pause(300).await;
                    continue;
                }
                let step = step.replace("-comma", "-,");
                let step = if step == "comma" { "," } else { step.as_str() };
                let strokes: Vec<String> = match step.strip_prefix("type:") {
                    Some(text) => text
                        .chars()
                        .map(|c| match c {
                            ' ' => "space".to_string(),
                            c => c.to_string(),
                        })
                        .collect(),
                    None => vec![step.to_string()],
                };
                for stroke in strokes {
                    match gpui::Keystroke::parse(&stroke) {
                        Ok(k) => {
                            let _ = cx.update(|window, cx| window.dispatch_keystroke(k, cx));
                        }
                        Err(e) => eprintln!("weep: WEEP_KEYS: {stroke}: {e}"),
                    }
                }
                pause(300).await;
            }
        })
        .detach();
}

/// macOS: the menu bar holds the application menu alone, as Fleet's does
/// (fourteenth pass: Fleet's bar shows only "Fleet"; its main menu stays
/// behind `show-main-menu`). The rows are the system's usual ones, an
/// estimate until Fleet's own menu is captured. A workspace answers About,
/// Settings and Quit (Quit asks about unsaved files); the app answers Quit
/// itself when no window is open, and the Hide rows always.
fn mac_app_menu(cx: &mut App) {
    cx.on_action(|_: &actions::Quit, cx| cx.quit());
    cx.on_action(|_: &actions::HideApp, cx| cx.hide());
    cx.on_action(|_: &actions::HideOtherApps, cx| cx.hide_other_apps());
    cx.on_action(|_: &actions::ShowAllApps, cx| cx.unhide_other_apps());
    // The menu shows each row's chord from these bindings.
    cx.bind_keys([
        KeyBinding::new("cmd-q", actions::Quit, None),
        KeyBinding::new("cmd-h", actions::HideApp, None),
        KeyBinding::new("alt-cmd-h", actions::HideOtherApps, None),
    ]);
    cx.set_menus(vec![Menu::new("WEEP").items([
        MenuItem::action("About WEEP", actions::About),
        MenuItem::separator(),
        MenuItem::action("Settings…", actions::OpenSettings),
        MenuItem::separator(),
        MenuItem::os_submenu("Services", SystemMenuType::Services),
        MenuItem::separator(),
        MenuItem::action("Hide WEEP", actions::HideApp),
        MenuItem::action("Hide Others", actions::HideOtherApps),
        MenuItem::action("Show All", actions::ShowAllApps),
        MenuItem::separator(),
        MenuItem::action("Quit WEEP", actions::Quit),
    ])]);
}

/// Writes stdout and stderr to the terminal WEEP was started from, if any. A GUI
/// program gets no console of its own; streams already redirected to a file or a
/// pipe keep their handles. From Explorer there is no parent console and this
/// does nothing.
#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: AttachConsole takes no pointers; failure (no parent console) is harmless.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn main() {
    #[cfg(all(windows, not(debug_assertions)))]
    attach_parent_console();
    let args = parse_args();
    let overrides = settings::Overrides {
        theme: args
            .theme
            .or_else(|| std::env::var("WEEP_THEME").ok())
            .map(|spec| ThemeSource::parse(&spec)),
        zoom: args.zoom,
        skin: args
            .skin
            .or_else(|| std::env::var("WEEP_SKIN").ok())
            .map(|s| {
                skin::Skin::parse(&s).unwrap_or_else(|| {
                    eprintln!("weep: unknown skin {s:?}; using fleet");
                    skin::Skin::Fleet
                })
            }),
    };
    let (root, open) = workspace::resolve_args(&args.paths);
    let scratch = args.scratch;
    let window_size = args.size.unwrap_or((1280., 800.));

    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            settings::init(overrides, cx);
            settings::watch(cx);
            if let Err(e) = cx.text_system().add_fonts(assets::fonts()) {
                eprintln!("weep: could not register the bundled fonts: {e}");
            }
            editor::bind_keys(cx);
            file_tree::bind_keys(cx);
            input::bind_keys(cx);
            find::bind_keys(cx);
            let preset = cx
                .global::<settings::AppSettings>()
                .settings
                .keymap()
                .to_string();
            let keymap = keymap::load(&preset);
            keymap::bind(&keymap, cx);
            cx.set_global(keymap::Hints::from_keymap(&keymap));
            cx.bind_keys([
                KeyBinding::new("ctrl-q", actions::Quit, None),
                KeyBinding::new("up", actions::PopupUp, Some("Popup")),
                KeyBinding::new("down", actions::PopupDown, Some("Popup")),
                KeyBinding::new("left", actions::PopupLeft, Some("Popup")),
                KeyBinding::new("right", actions::PopupRight, Some("Popup")),
                KeyBinding::new("pageup", actions::PopupPageUp, Some("Popup")),
                KeyBinding::new("pagedown", actions::PopupPageDown, Some("Popup")),
                KeyBinding::new("pageup", actions::PopupPageUp, Some("SettingsTool")),
                KeyBinding::new("pagedown", actions::PopupPageDown, Some("SettingsTool")),
                KeyBinding::new("tab", actions::PopupTab, Some("Popup")),
                KeyBinding::new("shift-tab", actions::PopupBackTab, Some("Popup")),
                KeyBinding::new("enter", actions::PopupConfirm, Some("Popup")),
                KeyBinding::new("escape", actions::PopupCancel, Some("Popup")),
                KeyBinding::new("backspace", actions::PopupBackspace, Some("Popup")),
                KeyBinding::new("up", actions::PopupUp, Some("SearchTool")),
                KeyBinding::new("down", actions::PopupDown, Some("SearchTool")),
            ]);
            if cfg!(target_os = "macos") {
                mac_app_menu(cx);
            }
            open_workspace_window(root, open, scratch, window_size, cx);
            cx.activate(true);
        });
}
