//! Oak: a native Rust/GPUI code editor in the shape of JetBrains Fleet.
//!
//! `oak [path ...] [--theme <name|file>]` opens a folder (or a file's
//! folder, with the file in a tab) in the Fleet-style shell: the toolbar,
//! the Files island, the editor island with its tabs, the status bar.

mod actions;
mod assets;
mod editor;
mod file_tree;
mod keymap;
mod theme;
mod toolbar;
mod workspace;

use gpui::{
    px, size, App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds,
    WindowOptions,
};
use std::path::PathBuf;
use theme::{Theme, ThemeSource};
use workspace::Workspace;

struct Args {
    paths: Vec<PathBuf>,
    theme: Option<String>,
    size: Option<(f32, f32)>,
}

fn parse_args() -> Args {
    let mut args = Args {
        paths: Vec::new(),
        theme: None,
        size: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--theme" => args.theme = it.next(),
            "--size" => {
                args.size = it.next().and_then(|s| {
                    let (w, h) = s.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                })
            }
            "-h" | "--help" => {
                println!("usage: oak [path ...] [--theme dark|dark_purple|light|<file.json>] [--size WxH]");
                std::process::exit(0);
            }
            _ => args.paths.push(PathBuf::from(a)),
        }
    }
    args
}

fn main() {
    let args = parse_args();
    let source = match args.theme.or_else(|| std::env::var("OAK_THEME").ok()) {
        Some(spec) => ThemeSource::parse(&spec),
        None => ThemeSource::default_for_machine(),
    };
    let theme = match Theme::load(&source) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("oak: {e:#}; using the bundled dark theme");
            Theme::load(&ThemeSource::Bundled("dark".into())).expect("bundled dark theme")
        }
    };
    let (root, open) = workspace::resolve_args(&args.paths);
    let (width, height) = args.size.unwrap_or((1280., 800.));

    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            cx.set_global(theme);
            if let Err(e) = cx.text_system().add_fonts(assets::fonts()) {
                eprintln!("oak: could not register the bundled fonts: {e}");
            }
            editor::bind_keys(cx);
            file_tree::bind_keys(cx);
            keymap::bind(cx);
            cx.bind_keys([KeyBinding::new("ctrl-q", actions::Quit, None)]);

            let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Oak".into()),
                        appears_transparent: false,
                        traffic_light_position: None,
                    }),
                    // KWin ties an activation token to an application through this id.
                    app_id: Some("oak".into()),
                    window_min_size: Some(size(px(480.), px(320.))),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Workspace::new(root, open, window, cx)),
            )
            .expect("open the main window");
            cx.activate(true);
        });
}
