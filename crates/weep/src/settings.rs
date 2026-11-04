//! The settings in the app: the merged [`weep_settings::Settings`] as a global,
//! the [`Metrics`] and [`Theme`] globals derived from it, a once-a-second
//! poll of both files for live reload, and the `zoom-*` actions, which Fleet
//! persists as `zoomLevel` in the settings file. The [`Skin`] global follows
//! `weep.skin`, WEEP's own key.

use crate::skin::Skin;
use crate::theme::{Metrics, Theme, ThemeSource};
use gpui::{App, Global};
use std::time::Duration;
use weep_settings::{Paths, Settings, Watcher, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP};

/// What the command line fixed for this run, overriding the files.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub theme: Option<ThemeSource>,
    pub zoom: Option<f32>,
    pub skin: Option<Skin>,
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

    pub fn skin(&self) -> Skin {
        self.overrides
            .skin
            .unwrap_or_else(|| Skin::parse(self.settings.weep_skin()).unwrap_or_default())
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
    cx.set_global(app.skin());
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
            eprintln!("weep: {e:#}; using the bundled dark theme");
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
    let skin = cx.global::<AppSettings>().skin();
    if Skin::current(cx) != skin {
        cx.set_global(skin);
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

/// The Settings tool wrote a value: into WEEP's file, then applied.
pub fn set_value(key: &str, value: serde_json::Value, cx: &mut App) {
    {
        let app = cx.global_mut::<AppSettings>();
        if key == "zoomLevel" {
            app.overrides.zoom = None;
        }
        if key == "theme" {
            app.overrides.theme = None;
        }
        if key == "weep.skin" {
            app.overrides.skin = None;
        }
        app.settings.set(key, value);
        if let Some(paths) = &app.paths {
            if let Err(e) = app.settings.save(&paths.weep) {
                eprintln!("weep: cannot save {}: {e:#}", paths.weep.display());
            }
        }
        if let Some(w) = app.watcher.as_mut() {
            w.settle();
        }
    }
    apply(cx);
}

/// `zoom-in`, `zoom-out`, `reset-zoom`: one step of `zoomLevel`, written to
/// WEEP's settings file as Fleet writes it to its own.
pub fn set_zoom(zoom: f32, cx: &mut App) {
    let zoom = (zoom.clamp(ZOOM_MIN, ZOOM_MAX) * 100.).round() / 100.;
    {
        let app = cx.global_mut::<AppSettings>();
        app.overrides.zoom = None;
        app.settings.set("zoomLevel", serde_json::json!(zoom));
        if let Some(paths) = &app.paths {
            if let Err(e) = app.settings.save(&paths.weep) {
                eprintln!("weep: cannot save {}: {e:#}", paths.weep.display());
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

/// The path of WEEP's own settings file, for `edit-settings-file`.
pub fn settings_file(cx: &App) -> Option<std::path::PathBuf> {
    cx.global::<AppSettings>()
        .paths
        .as_ref()
        .map(|p| p.weep.clone())
}

/// The path of WEEP's own keymap file, for `edit-keymap-file`.
pub fn keymap_file() -> Option<std::path::PathBuf> {
    weep_settings::weep_config_dir().map(|d| d.join("keymap.json"))
}
