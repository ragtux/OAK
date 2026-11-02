//! Oak: a native Rust/GPUI code editor in the shape of JetBrains Fleet.
//!
//! `oak [path ...] [--theme <name|file>] [--zoom <level>] [--size WxH]`
//! opens a folder (or a file's folder, with the file in a tab) in the
//! Fleet-style shell: the toolbar, the tool panels, the editor splits with
//! their tabs, the status bar. Settings come from `~/.config/oak/settings.json`
//! over Fleet's own file; `--theme` and `--zoom` override them for the run.

mod actions;
mod assets;
mod document;
mod editor;
mod file_tree;
mod keymap;
mod layout;
mod scrollbar;
mod settings;
mod theme;
mod toolbar;
mod tooltip;
mod workspace;

use gpui::{
    px, size, App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds,
    WindowOptions,
};
use std::path::PathBuf;
use theme::ThemeSource;
use workspace::Workspace;

struct Args {
    paths: Vec<PathBuf>,
    theme: Option<String>,
    zoom: Option<f32>,
    size: Option<(f32, f32)>,
}

fn parse_args() -> Args {
    let mut args = Args {
        paths: Vec::new(),
        theme: None,
        zoom: None,
        size: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--theme" => args.theme = it.next(),
            "--zoom" => args.zoom = it.next().and_then(|z| z.parse().ok()),
            "--size" => {
                args.size = it.next().and_then(|s| {
                    let (w, h) = s.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                })
            }
            "-h" | "--help" => {
                println!(
                    "usage: oak [path ...] [--theme dark|dark_purple|light|<file.json>] [--zoom 0.7..2.0] [--size WxH]"
                );
                std::process::exit(0);
            }
            _ => args.paths.push(PathBuf::from(a)),
        }
    }
    args
}

fn main() {
    let args = parse_args();
    let overrides = settings::Overrides {
        theme: args
            .theme
            .or_else(|| std::env::var("OAK_THEME").ok())
            .map(|spec| ThemeSource::parse(&spec)),
        zoom: args.zoom,
    };
    let (root, open) = workspace::resolve_args(&args.paths);
    let (width, height) = args.size.unwrap_or((1280., 800.));

    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            settings::init(overrides, cx);
            settings::watch(cx);
            if let Err(e) = cx.text_system().add_fonts(assets::fonts()) {
                eprintln!("oak: could not register the bundled fonts: {e}");
            }
            editor::bind_keys(cx);
            file_tree::bind_keys(cx);
            let keymap = keymap::load();
            keymap::bind(&keymap, cx);
            cx.bind_keys([
                KeyBinding::new("ctrl-q", actions::Quit, None),
                KeyBinding::new("up", actions::PopupUp, Some("Popup")),
                KeyBinding::new("down", actions::PopupDown, Some("Popup")),
                KeyBinding::new("enter", actions::PopupConfirm, Some("Popup")),
                KeyBinding::new("escape", actions::PopupCancel, Some("Popup")),
                KeyBinding::new("backspace", actions::PopupBackspace, Some("Popup")),
            ]);
            let hints = [
                keymap::hint(&keymap, "toggle-left-panel"),
                keymap::hint(&keymap, "toggle-bottom-panel"),
                keymap::hint(&keymap, "toggle-right-panel"),
            ];

            let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Oak".into()),
                        // Fleet draws its own caption on Windows (docs/fleet-probe.md).
                        appears_transparent: cfg!(windows),
                        traffic_light_position: None,
                    }),
                    // KWin ties an activation token to an application through this id.
                    app_id: Some("oak".into()),
                    window_min_size: Some(size(px(480.), px(320.))),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Workspace::new(root, open, hints, window, cx)),
            )
            .expect("open the main window");
            cx.activate(true);
        });
}
