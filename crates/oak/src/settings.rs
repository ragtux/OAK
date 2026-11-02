//! The settings in the app: the merged [`oak_settings::Settings`] as a global,
//! the [`Metrics`] and [`Theme`] globals derived from it, a once-a-second
//! poll of both files for live reload, and the `zoom-*` actions, which Fleet
//! persists as `zoomLevel` in the settings file.

use crate::theme::{Metrics, Theme, ThemeSource};
use gpui::{App, Global};
use oak_settings::{Paths, Settings, Watcher, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP};
use std::time::Duration;

/// What the command line fixed for this run, overriding the files.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub theme: Option<ThemeSource>,
    pub zoom: Option<f32>,
}

pub struct AppSettings {
    pub settings: Settings,
    pub paths: Option<Paths>,
    pub overrides: Overrides,
    watcher: Option<Watcher>,
    theme_name: String,
}

impl Global for AppSettings {}

impl AppSettings {
    pub fn zoom(&self) -> f32 {
        self.overrides
            .zoom
            .unwrap_or_else(|| self.settings.zoom_level())
    }

    fn theme_source(&self) -> ThemeSource {
        self.overrides
            .theme
            .clone()
            .unwrap_or_else(|| ThemeSource::parse(self.settings.theme()))
    }
}

/// Loads the files, sets the `AppSettings`, `Metrics` and `Theme` globals.
pub fn init(overrides: Overrides, cx: &mut App) {
    let paths = Paths::for_machine();
    let settings = paths.as_ref().map(Settings::load).unwrap_or_default();
    let watcher = paths.clone().map(Watcher::new);
    let app = AppSettings {
        settings,
        paths,
        overrides,
        watcher,
        theme_name: String::new(),
    };
    let theme = load_theme(&app.theme_source());
    let theme_name = app.theme_source_name();
    cx.set_global(Metrics::at(app.zoom()));
    cx.set_global(theme);
    let mut app = app;
    app.theme_name = theme_name;
    cx.set_global(app);
}

impl AppSettings {
    fn theme_source_name(&self) -> String {
        format!("{:?}", self.theme_source())
    }
}

fn load_theme(source: &ThemeSource) -> Theme {
    match Theme::load(source) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("oak: {e:#}; using the bundled dark theme");
            Theme::load(&ThemeSource::Bundled("dark".into())).expect("bundled dark theme")
        }
    }
}

/// Re-reads the files and applies what changed: the theme, the zoom, the
/// editor font. Every window is redrawn.
pub fn reload(cx: &mut App) {
    let paths = cx.global::<AppSettings>().paths.clone();
    if let Some(paths) = paths {
        let settings = Settings::load(&paths);
        cx.global_mut::<AppSettings>().settings = settings;
    }
    apply(cx);
}

/// Derives the globals from the current settings and redraws.
pub fn apply(cx: &mut App) {
    let (source, name, zoom, font) = {
        let app = cx.global::<AppSettings>();
        (
            app.theme_source(),
            app.theme_source_name(),
            app.zoom(),
            app.settings.editor_font_family().map(str::to_string),
        )
    };
    if cx.global::<AppSettings>().theme_name != name {
        let theme = load_theme(&source);
        cx.set_global(theme);
        cx.global_mut::<AppSettings>().theme_name = name;
    }
    let mono: gpui::SharedString = font
        .map(Into::into)
        .unwrap_or_else(|| crate::assets::MONO_FONT.into());
    if cx.global::<Theme>().mono_font != mono {
        cx.global_mut::<Theme>().mono_font = mono;
    }
    let metrics = Metrics::at(zoom);
    if *cx.global::<Metrics>() != metrics {
        cx.set_global(metrics);
    }
    cx.refresh_windows();
}

/// Polls both settings files once a second; a change reloads them.
pub fn watch(cx: &mut App) {
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        let changed = cx.update(|cx| {
            cx.global_mut::<AppSettings>()
                .watcher
                .as_mut()
                .is_some_and(Watcher::changed)
        });
        if changed {
            cx.update(reload);
        }
    })
    .detach();
}

/// `zoom-in`, `zoom-out`, `reset-zoom`: one step of `zoomLevel`, written to
/// Oak's settings file as Fleet writes it to its own.
pub fn set_zoom(zoom: f32, cx: &mut App) {
    let zoom = (zoom.clamp(ZOOM_MIN, ZOOM_MAX) * 100.).round() / 100.;
    {
        let app = cx.global_mut::<AppSettings>();
        app.overrides.zoom = None;
        app.settings.set("zoomLevel", serde_json::json!(zoom));
        if let Some(paths) = &app.paths {
            if let Err(e) = app.settings.save(&paths.oak) {
                eprintln!("oak: cannot save {}: {e:#}", paths.oak.display());
            }
        }
        if let Some(w) = app.watcher.as_mut() {
            w.settle();
        }
    }
    apply(cx);
}

pub fn zoom_in(cx: &mut App) {
    let z = cx.global::<AppSettings>().zoom();
    set_zoom(z + ZOOM_STEP, cx);
}

pub fn zoom_out(cx: &mut App) {
    let z = cx.global::<AppSettings>().zoom();
    set_zoom(z - ZOOM_STEP, cx);
}

pub fn reset_zoom(cx: &mut App) {
    set_zoom(1.0, cx);
}

/// The path of Oak's own settings file, for `edit-settings-file`.
pub fn settings_file(cx: &App) -> Option<std::path::PathBuf> {
    cx.global::<AppSettings>()
        .paths
        .as_ref()
        .map(|p| p.oak.clone())
}

/// The path of Oak's own keymap file, for `edit-keymap-file`.
pub fn keymap_file() -> Option<std::path::PathBuf> {
    oak_settings::oak_config_dir().map(|d| d.join("keymap.json"))
}
