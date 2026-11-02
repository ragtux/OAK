//! Settings in JetBrains Fleet's `settings.json` format: the same keys with the
//! same meaning, so a Fleet user's file carries over.
//!
//! Oak reads its own file (`~/.config/oak/settings.json`, `%APPDATA%\oak` on
//! Windows) on top of Fleet's (`~/.config/JetBrains/Fleet/settings.json`,
//! `%APPDATA%\JetBrains\Fleet` on Windows): a key in Oak's file wins, a key
//! only in Fleet's still applies, and a key in neither takes the default
//! Fleet's documentation or its Settings tool gives it. Fleet allows `//`
//! comments and trailing commas in the file, so the parser strips them.
//!
//! Every accessor below names the Fleet key it reads in its doc comment, and
//! `tools/fleet-probe/coverage.py` counts the quoted keys in this file.

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Where the two files live on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub oak: PathBuf,
    pub fleet: PathBuf,
}

/// The per-user configuration root: `$XDG_CONFIG_HOME`, else `~/.config`
/// (`%APPDATA%` on Windows, which is also where Fleet keeps its own folder).
pub fn config_home() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return Some(PathBuf::from(appdata));
        }
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".config")))
}

/// Oak's own configuration folder (`settings.json`, `keymap.json`, `themes/`,
/// `layout.json`): `$OAK_CONFIG_DIR` when set (the probes point it at a scratch
/// folder so a run starts from a fresh layout), else `oak` under the config home.
pub fn oak_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OAK_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    Some(config_home()?.join("oak"))
}

/// Fleet's configuration folder on this machine.
pub fn fleet_config_dir() -> Option<PathBuf> {
    Some(config_home()?.join("JetBrains").join("Fleet"))
}

impl Paths {
    pub fn for_machine() -> Option<Paths> {
        Some(Paths {
            oak: oak_config_dir()?.join("settings.json"),
            fleet: fleet_config_dir()?.join("settings.json"),
        })
    }
}

/// How the Files tree opens a file or toggles a folder (`files.openFileMode`,
/// `files.toggleFolderMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickMode {
    Single,
    Double,
}

/// The merged settings: Oak's values over Fleet's, as one JSON object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    values: Map<String, Value>,
    /// The keys Oak's own file sets (what `save` writes back).
    own: Map<String, Value>,
}

impl Settings {
    /// Parses one settings file's text.
    pub fn parse(text: &str) -> Result<Settings> {
        let values = parse_object(text)?;
        Ok(Settings {
            own: values.clone(),
            values,
        })
    }

    /// Reads Oak's file over Fleet's; a missing file is simply empty, an
    /// unreadable one is reported and skipped so the app still starts.
    pub fn load(paths: &Paths) -> Settings {
        let read = |path: &Path| -> Map<String, Value> {
            match std::fs::read_to_string(path) {
                Ok(text) => match parse_object(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("oak: {}: {e:#}; ignored", path.display());
                        Map::new()
                    }
                },
                Err(_) => Map::new(),
            }
        };
        let mut values = read(&paths.fleet);
        let own = read(&paths.oak);
        for (k, v) in &own {
            values.insert(k.clone(), v.clone());
        }
        Settings { values, own }
    }

    /// The raw value of a key, if either file sets it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Value::as_bool)
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Value::as_f64)
    }

    /// Sets a key in Oak's own file (and the merged view).
    pub fn set(&mut self, key: &str, value: Value) {
        self.values.insert(key.to_string(), value.clone());
        self.own.insert(key.to_string(), value);
    }

    /// Writes Oak's own keys to Oak's file, creating the folder.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
        }
        let text = serde_json::to_string_pretty(&Value::Object(self.own.clone()))?;
        std::fs::write(path, text + "\n").with_context(|| path.display().to_string())
    }

    // ---- the keys M1 reads ---------------------------------------------------

    /// `theme`: "Color theme to use for the whole Fleet application", by name.
    /// Fleet's default is `dark`.
    pub fn theme(&self) -> &str {
        self.str("theme").unwrap_or("dark")
    }

    /// `zoomLevel`: "Apply zoom to all Fleet panels. The value is a number
    /// between 0.7 and 2.0." Default 1.0; out-of-range values are clamped.
    pub fn zoom_level(&self) -> f32 {
        self.f64("zoomLevel")
            .map(|z| z as f32)
            .unwrap_or(1.0)
            .clamp(ZOOM_MIN, ZOOM_MAX)
    }

    /// `editor.fontSize`: "Editor font size in pixels". Fleet's default is 13
    /// (measured: JetBrains Mono at 13 px, `docs/fleet-probe.md`).
    pub fn editor_font_size(&self) -> f32 {
        self.f64("editor.fontSize")
            .map(|v| v as f32)
            .unwrap_or(13.0)
    }

    /// `editor.fontFamily`: "Editor font family". None means the bundled JetBrains Mono.
    pub fn editor_font_family(&self) -> Option<&str> {
        self.str("editor.fontFamily").filter(|s| !s.is_empty())
    }

    /// `editor.lineSpacing`: "the distance between the baselines is calculated
    /// as font size multiplied by the given number". None means Fleet's
    /// default, which the probe measured as 22 px at 13 px.
    pub fn editor_line_spacing(&self) -> Option<f32> {
        self.f64("editor.lineSpacing").map(|v| v as f32)
    }

    /// `editor.previewTab`: "Open a separate preview tab when selecting a file
    /// in the Files view". A fresh Fleet opened a preview tab, so the default is true.
    pub fn editor_preview_tab(&self) -> bool {
        self.bool("editor.previewTab").unwrap_or(true)
    }

    /// `ff.tabCloseButtonOnTheLeft`: "Show tab close button on the left on hover
    /// in place of the tab icon". A fresh Fleet shows it on the right: default false.
    pub fn tab_close_button_on_the_left(&self) -> bool {
        self.bool("ff.tabCloseButtonOnTheLeft").unwrap_or(false)
    }

    /// `files.compactDirectories`: "single child folders will be combined into a
    /// single item". A fresh Fleet shows `outer / middle / inner`: default true.
    pub fn files_compact_directories(&self) -> bool {
        self.bool("files.compactDirectories").unwrap_or(true)
    }

    /// `files.selectOpenedFile`: "Select and scroll to the file in the Files
    /// tree that is opened in the editor". A fresh Fleet does: default true.
    pub fn files_select_opened_file(&self) -> bool {
        self.bool("files.selectOpenedFile").unwrap_or(true)
    }

    /// `files.openFileMode`: "open files in the Files tree with a single- or
    /// double-click". A single click opened a file in a fresh Fleet: default single.
    pub fn files_open_file_mode(&self) -> ClickMode {
        click_mode(self.str("files.openFileMode"), ClickMode::Single)
    }

    /// `files.toggleFolderMode`: "expand folders in the Files tree with a
    /// single- or double-click". Default single (estimate: matches openFileMode).
    pub fn files_toggle_folder_mode(&self) -> ClickMode {
        click_mode(self.str("files.toggleFolderMode"), ClickMode::Single)
    }

    /// `files.exclude`: "Listed files will be hidden from the Files tree", as
    /// glob-like names. A fresh Fleet hides `.git` and nothing else that the
    /// fixture showed, so that is the default.
    pub fn files_exclude(&self) -> Vec<String> {
        match self.get("files.exclude") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => vec![".git".to_string()],
        }
    }

    /// `editor.showStatusBar`: default true.
    pub fn editor_show_status_bar(&self) -> bool {
        self.bool("editor.showStatusBar").unwrap_or(true)
    }

    /// `editor.showTabs`: default true.
    pub fn editor_show_tabs(&self) -> bool {
        self.bool("editor.showTabs").unwrap_or(true)
    }

    /// `closeEmptySplit`: "Close empty split after closing the last remaining
    /// tab". Default true (estimate).
    pub fn close_empty_split(&self) -> bool {
        self.bool("closeEmptySplit").unwrap_or(true)
    }
}

/// Fleet's documented range for `zoomLevel`.
pub const ZOOM_MIN: f32 = 0.7;
pub const ZOOM_MAX: f32 = 2.0;
/// One `zoom-in` or `zoom-out` step. Estimate: Fleet's step is not documented
/// and the probe has not measured it yet (`docs/fleet-probe.md`).
pub const ZOOM_STEP: f32 = 0.1;

fn click_mode(value: Option<&str>, default: ClickMode) -> ClickMode {
    match value.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("single") | Some("singleclick") | Some("single-click") => ClickMode::Single,
        Some("double") | Some("doubleclick") | Some("double-click") => ClickMode::Double,
        _ => default,
    }
}

fn parse_object(text: &str) -> Result<Map<String, Value>> {
    let text = strip_json5(text);
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text).context("settings JSON")? {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("settings must be a JSON object"),
    }
}

/// Removes `//` line comments (outside strings) and trailing commas, which
/// Fleet accepts in its JSON files.
fn strip_json5(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_string = !in_string;
                out.push(c);
            }
            '\\' if in_string => {
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '/' if !in_string && chars.peek() == Some(&'/') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    let mut cleaned = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' && (i == 0 || chars[i - 1] != '\\') {
            in_string = !in_string;
        }
        if c == ',' && !in_string {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }
        cleaned.push(c);
        i += 1;
    }
    cleaned
}

/// Notices when either file changes on disk, by modification time, for a
/// caller that polls (the app asks once a second).
#[derive(Debug)]
pub struct Watcher {
    paths: Paths,
    stamps: [Option<SystemTime>; 2],
}

impl Watcher {
    pub fn new(paths: Paths) -> Watcher {
        let stamps = [stamp(&paths.fleet), stamp(&paths.oak)];
        Watcher { paths, stamps }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// True once per change: the first call after a file was written,
    /// created or removed.
    pub fn changed(&mut self) -> bool {
        let now = [stamp(&self.paths.fleet), stamp(&self.paths.oak)];
        if now != self.stamps {
            self.stamps = now;
            true
        } else {
            false
        }
    }

    /// Forgets the current state, so a write the app made itself is not
    /// reported back as a change.
    pub fn settle(&mut self) {
        self.stamps = [stamp(&self.paths.fleet), stamp(&self.paths.oak)];
    }
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_fleet() {
        let s = Settings::parse("{}").unwrap();
        assert_eq!(s.theme(), "dark");
        assert_eq!(s.zoom_level(), 1.0);
        assert_eq!(s.editor_font_size(), 13.0);
        assert!(s.editor_preview_tab());
        assert!(!s.tab_close_button_on_the_left());
        assert!(s.files_compact_directories());
        assert_eq!(s.files_open_file_mode(), ClickMode::Single);
        assert_eq!(s.files_exclude(), vec![".git".to_string()]);
    }

    #[test]
    fn reads_fleet_style_json_with_comments_and_clamps_zoom() {
        let s = Settings::parse(
            r#"{
              // a comment
              "theme": "light",
              "zoomLevel": 3.5,
              "editor.fontSize": 15,
              "files.openFileMode": "double",
              "files.exclude": ["**/.git", "*.log"],
            }"#,
        )
        .unwrap();
        assert_eq!(s.theme(), "light");
        assert_eq!(s.zoom_level(), ZOOM_MAX);
        assert_eq!(s.editor_font_size(), 15.0);
        assert_eq!(s.files_open_file_mode(), ClickMode::Double);
        assert_eq!(s.files_exclude(), vec!["**/.git", "*.log"]);
    }

    #[test]
    fn oak_file_overrides_fleet_file_and_saves_only_its_own_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            oak: dir.path().join("oak/settings.json"),
            fleet: dir.path().join("fleet/settings.json"),
        };
        std::fs::create_dir_all(paths.fleet.parent().unwrap()).unwrap();
        std::fs::write(&paths.fleet, r#"{"theme": "light", "zoomLevel": 1.5}"#).unwrap();
        let mut s = Settings::load(&paths);
        assert_eq!(s.theme(), "light");
        assert_eq!(s.zoom_level(), 1.5);

        s.set("theme", Value::String("dark_purple".into()));
        s.save(&paths.oak).unwrap();
        let again = Settings::load(&paths);
        assert_eq!(again.theme(), "dark_purple");
        assert_eq!(again.zoom_level(), 1.5, "Fleet's key still applies");
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.oak).unwrap()).unwrap();
        assert_eq!(written, serde_json::json!({"theme": "dark_purple"}));
    }

    #[test]
    fn watcher_reports_each_change_once() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            oak: dir.path().join("settings.json"),
            fleet: dir.path().join("fleet.json"),
        };
        let mut w = Watcher::new(paths.clone());
        assert!(!w.changed());
        std::fs::write(&paths.oak, "{}").unwrap();
        assert!(w.changed());
        assert!(!w.changed());
        w.settle();
        assert!(!w.changed());
    }

    #[test]
    fn a_broken_file_is_ignored_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            oak: dir.path().join("settings.json"),
            fleet: dir.path().join("missing.json"),
        };
        std::fs::write(&paths.oak, "{ not json").unwrap();
        let s = Settings::load(&paths);
        assert_eq!(s.theme(), "dark");
    }
}
