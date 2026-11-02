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

mod editorconfig;
mod git;
mod workbench;
pub use editorconfig::{EditorConfig, EndOfLine};
pub use workbench::KeyboardNavigation;

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

/// When an edited file is written by itself (`autoSave`): Fleet's General
/// page offers Off, On focus change and After delay and writes `"Off"`,
/// `"OnFocus"` and `"AfterDelay"` (ninth pass). Default `OnFocus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSave {
    Off,
    OnFocus,
    AfterDelay,
}

/// Whether an indent is made of spaces or a tab (`editor.indentStyle`, and
/// EditorConfig's `indent_style`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentStyle {
    Spaces,
    Tabs,
}

impl IndentStyle {
    /// Reads Fleet's spelling (`spaces`/`tabs`) or EditorConfig's (`space`/`tab`),
    /// in any case.
    pub fn parse(s: &str) -> Option<IndentStyle> {
        match s.trim().to_ascii_lowercase().as_str() {
            "space" | "spaces" => Some(IndentStyle::Spaces),
            "tab" | "tabs" => Some(IndentStyle::Tabs),
            _ => None,
        }
    }
}

/// How the gutter numbers lines (`editor.lineNumbers`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineNumbers {
    On,
    Off,
    Relative,
}

impl LineNumbers {
    pub fn parse(s: &str) -> Option<LineNumbers> {
        match s.trim().to_ascii_lowercase().as_str() {
            "on" => Some(LineNumbers::On),
            "off" => Some(LineNumbers::Off),
            "relative" => Some(LineNumbers::Relative),
            _ => None,
        }
    }
}

/// The caret's drawing (`editor.caretShape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretShape {
    Line,
    Block,
    Underline,
}

impl CaretShape {
    /// Fleet's names, plus `bar` and `vertical` for the line caret.
    pub fn parse(s: &str) -> Option<CaretShape> {
        match s.trim().to_ascii_lowercase().as_str() {
            "line" | "bar" | "vertical" => Some(CaretShape::Line),
            "block" => Some(CaretShape::Block),
            "underline" | "underscore" => Some(CaretShape::Underline),
            _ => None,
        }
    }
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

    /// A non-negative whole number (a JSON number or a numeric string).
    pub fn usize(&self, key: &str) -> Option<usize> {
        self.get(key).and_then(as_usize)
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
        click_mode(self.str("files.toggleFolderMode"), ClickMode::Double)
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

    // ---- the keys M2 reads ---------------------------------------------------

    /// `editor.tabSize`: "The number of spaces corresponding to a tab". Fleet's
    /// default is 4; zero is not a tab size and falls back to it.
    pub fn editor_tab_size(&self) -> usize {
        self.usize("editor.tabSize").filter(|n| *n > 0).unwrap_or(4)
    }

    /// `editor.indentStyle`: "Indent using spaces or tabs. EditorConfig may
    /// override this option." Fleet's default is spaces.
    pub fn editor_indent_style(&self) -> IndentStyle {
        self.str("editor.indentStyle")
            .and_then(IndentStyle::parse)
            .unwrap_or(IndentStyle::Spaces)
    }

    /// `editor.autoDetectIndentation`: "Indent type and size is detected by the
    /// file content. Overrides tab size and indentation style settings.
    /// EditorConfig may override the detected settings." Default true (estimate).
    pub fn editor_auto_detect_indentation(&self) -> bool {
        self.bool("editor.autoDetectIndentation").unwrap_or(true)
    }

    /// `enableIndenter`: "Automatically indent code fragments on editing, for
    /// example, after using a line break, closing a brace, inserting a snippet,
    /// and so on". Fleet's default is true.
    pub fn enable_indenter(&self) -> bool {
        self.bool("enableIndenter").unwrap_or(true)
    }

    /// `trimTrailingSpacesOnSave`: "Remove whitespace at the end of the lines
    /// when the file is saved. The lines that currently have carets on them are
    /// not trimmed." Default false (estimate).
    pub fn trim_trailing_spaces_on_save(&self) -> bool {
        self.bool("trimTrailingSpacesOnSave").unwrap_or(true)
    }

    /// `autoSave`: "Automatically save the edited file": `Off`, `OnFocus` or
    /// `AfterDelay` (Fleet's spellings; a bool from an older Oak file still
    /// reads as on or off). Default `OnFocus`.
    pub fn auto_save(&self) -> AutoSave {
        match self.get("autoSave") {
            Some(Value::Bool(false)) => AutoSave::Off,
            Some(Value::Bool(true)) => AutoSave::OnFocus,
            Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
                "off" | "false" => AutoSave::Off,
                "afterdelay" | "after_delay" | "delay" => AutoSave::AfterDelay,
                _ => AutoSave::OnFocus,
            },
            _ => AutoSave::OnFocus,
        }
    }

    /// Whether Oak's own file sets the key (the Settings tool's modified mark).
    pub fn is_own(&self, key: &str) -> bool {
        self.own.contains_key(key)
    }

    /// `autoSave.delay`: "Automatically save the edited file after a specified
    /// number of milliseconds since the last modification". Default 1000 (estimate).
    pub fn auto_save_delay_ms(&self) -> u64 {
        self.f64("autoSave.delay")
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v as u64)
            .unwrap_or(1000)
    }

    /// `createScratchOnNewFile`: "Always create a temporary file on invoking
    /// the New File action". Fleet's default is false.
    pub fn create_scratch_on_new_file(&self) -> bool {
        self.bool("createScratchOnNewFile").unwrap_or(true)
    }

    /// `localBigFileMinimumThreshold`: "Open local files of this size (in MB)
    /// and larger only after an additional confirmation". Default 20 MB (estimate).
    pub fn local_big_file_minimum_threshold_mb(&self) -> f64 {
        self.f64("localBigFileMinimumThreshold")
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(20.0)
    }

    /// `editor.softWrapEnabled`: "Enable soft wraps in the editor. When soft
    /// wraps are on, the content of the lines that exceed the editor width will
    /// be wrapped onto the next line without actually inserting line breaks in
    /// the file." Fleet's default is false.
    pub fn editor_soft_wrap_enabled(&self) -> bool {
        self.bool("editor.softWrapEnabled").unwrap_or(false)
    }

    /// `editor.guideToSoftWrapBy`: "Soft-wrap text based on the guides
    /// specified in the editor.guides setting". Fleet's default is false.
    pub fn editor_guide_to_soft_wrap_by(&self) -> bool {
        self.bool("editor.guideToSoftWrapBy").unwrap_or(false)
    }

    /// `editor.guides`: "Visual guides (rulers) in the editor. For example,
    /// `[80, 100]` will add two guides at characters 80 and 100 relative to the
    /// beginning of the line." A single integer is accepted too. Fleet's
    /// default is no guides.
    pub fn editor_guides(&self) -> Vec<usize> {
        match self.get("editor.guides") {
            Some(Value::Array(items)) => items.iter().filter_map(as_usize).collect(),
            Some(v) => as_usize(v).into_iter().collect(),
            None => Vec::new(),
        }
    }

    /// `editor.showVisualGuides`: "Show visual guides in the editor". Fleet's
    /// default is true.
    pub fn editor_show_visual_guides(&self) -> bool {
        self.bool("editor.showVisualGuides").unwrap_or(true)
    }

    /// `editor.showIndentGuides`: "Show indent guides in the editor". A fresh
    /// Fleet draws them (`docs/fleet-probe.md`): default true.
    pub fn editor_show_indent_guides(&self) -> bool {
        self.bool("editor.showIndentGuides").unwrap_or(true)
    }

    /// `editor.showFoldings`: "Show folding areas on the gutter". Fleet's
    /// default is true.
    pub fn editor_show_foldings(&self) -> bool {
        self.bool("editor.showFoldings").unwrap_or(true)
    }

    /// `editor.showScrollbarMarks`: "Show colored stripes on the vertical
    /// scrollbar that indicate warnings, errors, TODOs, and so on". Fleet's
    /// default is true.
    pub fn editor_show_scrollbar_marks(&self) -> bool {
        self.bool("editor.showScrollbarMarks").unwrap_or(true)
    }

    /// `editor.showStickyLines`: "Show current scope on the top of the editor".
    /// A fresh Fleet showed sticky lines (`docs/fleet-probe.md`, batch 5):
    /// default true.
    pub fn editor_show_sticky_lines(&self) -> bool {
        self.bool("editor.showStickyLines").unwrap_or(true)
    }

    /// `editor.stickyLinesMaxCount`: "Maximum number of sticky lines to show."
    /// Default 5 (estimate).
    pub fn editor_sticky_lines_max_count(&self) -> usize {
        self.usize("editor.stickyLinesMaxCount").unwrap_or(5)
    }

    /// `editor.renderWhitespace`: "Show spaces and tabs in the editor". A
    /// boolean, or a string: `none`/`off` (or empty) means off, any other
    /// value on. Fleet's default is false.
    pub fn editor_render_whitespace(&self) -> bool {
        match self.get("editor.renderWhitespace") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => {
                let s = s.trim();
                !(s.is_empty()
                    || s.eq_ignore_ascii_case("none")
                    || s.eq_ignore_ascii_case("off")
                    || s.eq_ignore_ascii_case("false"))
            }
            _ => false,
        }
    }

    /// `editor.lineNumbers`: "Show line numbers in the gutter. Use "On" to
    /// render line numbers as an absolute value. Use "Off" to not show any line
    /// numbers. Use "Relative" to render line numbers as a distance in lines to
    /// caret position". Fleet's default is On.
    pub fn editor_line_numbers(&self) -> LineNumbers {
        self.str("editor.lineNumbers")
            .and_then(LineNumbers::parse)
            .unwrap_or(LineNumbers::On)
    }

    /// `editor.caretShape`: "Caret shape": line, block or underline (`bar` and
    /// `vertical` also mean line). Default line (estimate).
    pub fn editor_caret_shape(&self) -> CaretShape {
        self.str("editor.caretShape")
            .and_then(CaretShape::parse)
            .unwrap_or(CaretShape::Line)
    }

    /// `editor.caretBlinkingEnabled`: "Enable caret blinking in the editor".
    /// Fleet's default is true.
    pub fn editor_caret_blinking_enabled(&self) -> bool {
        self.bool("editor.caretBlinkingEnabled").unwrap_or(true)
    }

    /// `editor.caretSmoothAnimation`: "Enable cursor smooth caret animation".
    /// Default false (estimate).
    pub fn editor_caret_smooth_animation(&self) -> bool {
        self.bool("editor.caretSmoothAnimation").unwrap_or(true)
    }

    /// `editor.activeLineHighlightEnabled`: "Highlight active line". Fleet's
    /// default is true.
    pub fn editor_active_line_highlight_enabled(&self) -> bool {
        self.bool("editor.activeLineHighlightEnabled")
            .unwrap_or(true)
    }

    /// `editor.distractionFreeMode`: "Hide editor tabs and move the code closer
    /// to the editor center". Fleet's default is false.
    pub fn editor_distraction_free_mode(&self) -> bool {
        self.bool("editor.distractionFreeMode").unwrap_or(false)
    }

    /// `editor.centerEditorHorizontally`: "Center editor area horizontally if
    /// viewport is wider than editor area". Fleet's default is false.
    pub fn editor_center_editor_horizontally(&self) -> bool {
        self.bool("editor.centerEditorHorizontally")
            .unwrap_or(false)
    }

    /// `editor.fontWeight`: "Editor text weight which will be used if no other
    /// weight is given by highlighting. For example, a value of 500 will raise
    /// the default font weight to medium, which is slightly heavier than normal
    /// (regular)." Default 400, regular.
    pub fn editor_font_weight(&self) -> f32 {
        self.f64("editor.fontWeight")
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|v| v as f32)
            .unwrap_or(400.0)
    }

    /// `editor.fontFeatures`: "OpenType features that should be activated in
    /// the editor text. Use this to enable stylistic sets (ss01-ss20),
    /// character variants (cv01-cv99), slashed zero (zero), fractions (frac),
    /// and more." An array of feature tags, one tag (or several separated by
    /// commas or spaces), or an object `{ "ss01": true }` whose true keys are
    /// the features. Default none.
    pub fn editor_font_features(&self) -> Vec<String> {
        let tags = |s: &str| -> Vec<String> {
            s.split(|c: char| c == ',' || c.is_whitespace())
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect()
        };
        match self.get("editor.fontFeatures") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .flat_map(tags)
                .collect(),
            Some(Value::String(s)) => tags(s),
            Some(Value::Object(map)) => map
                .iter()
                .filter(|(_, v)| v.as_bool() == Some(true))
                .map(|(k, _)| k.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `editor.ligatures`: "Enable font ligatures". Default true (estimate).
    pub fn editor_ligatures(&self) -> bool {
        self.bool("editor.ligatures").unwrap_or(true)
    }

    /// `editor.honorCamelHumps`: "Expand/shrink selection in identifiers and
    /// literals respecting words written in camel or snake case". Default true
    /// (estimate).
    pub fn editor_honor_camel_humps(&self) -> bool {
        self.bool("editor.honorCamelHumps").unwrap_or(false)
    }

    /// `editor.useSelectionForSearch`: "Use selected text as initial query for
    /// Goto dialogs and editor search". Default true (estimate).
    pub fn editor_use_selection_for_search(&self) -> bool {
        self.bool("editor.useSelectionForSearch").unwrap_or(true)
    }

    /// `editor.showFileDetails`: "Show file details in the bottom part of the
    /// editor". Default true (estimate).
    pub fn editor_show_file_details(&self) -> bool {
        self.bool("editor.showFileDetails").unwrap_or(true)
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

/// A non-negative whole number, written as a JSON number or a numeric string.
fn as_usize(v: &Value) -> Option<usize> {
    match v {
        Value::Number(n) => n.as_u64().map(|n| n as usize).or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite() && *f >= 0.0 && f.fract() == 0.0)
                .map(|f| f as usize)
        }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
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
        assert_eq!(s.files_toggle_folder_mode(), ClickMode::Double);
        assert_eq!(s.files_exclude(), vec![".git".to_string()]);
        assert_eq!(s.editor_tab_size(), 4);
        assert_eq!(s.editor_indent_style(), IndentStyle::Spaces);
        assert!(s.editor_auto_detect_indentation());
        assert!(s.enable_indenter());
        // Batch 8 read these off Fleet's Settings pages (2026-11-02).
        assert!(s.trim_trailing_spaces_on_save());
        assert_eq!(s.auto_save(), AutoSave::OnFocus);
        assert!(!s.is_own("autoSave"));
        let s = Settings::parse(r#"{"autoSave": "AfterDelay"}"#).unwrap();
        assert_eq!(s.auto_save(), AutoSave::AfterDelay);
        assert!(s.is_own("autoSave"));
        let s = Settings::parse(r#"{"autoSave": "Off"}"#).unwrap();
        assert_eq!(s.auto_save(), AutoSave::Off);
        let s = Settings::parse(r#"{"autoSave": false}"#).unwrap();
        assert_eq!(s.auto_save(), AutoSave::Off);
        assert_eq!(s.auto_save_delay_ms(), 1000);
        assert!(s.create_scratch_on_new_file());
        assert_eq!(s.local_big_file_minimum_threshold_mb(), 20.0);
        assert!(!s.editor_soft_wrap_enabled());
        assert!(!s.editor_guide_to_soft_wrap_by());
        assert!(s.editor_guides().is_empty());
        assert!(s.editor_show_visual_guides());
        assert!(s.editor_show_indent_guides());
        assert!(s.editor_show_foldings());
        assert!(s.editor_show_scrollbar_marks());
        assert!(s.editor_show_sticky_lines());
        assert_eq!(s.editor_sticky_lines_max_count(), 5);
        assert!(!s.editor_render_whitespace());
        assert_eq!(s.editor_line_numbers(), LineNumbers::On);
        assert_eq!(s.editor_caret_shape(), CaretShape::Line);
        assert!(s.editor_caret_blinking_enabled());
        assert!(s.editor_caret_smooth_animation());
        assert!(s.editor_active_line_highlight_enabled());
        assert!(!s.editor_distraction_free_mode());
        assert!(!s.editor_center_editor_horizontally());
        assert_eq!(s.editor_font_weight(), 400.0);
        assert!(s.editor_font_features().is_empty());
        assert!(s.editor_ligatures());
        assert!(!s.editor_honor_camel_humps());
        assert!(s.editor_use_selection_for_search());
        assert!(s.editor_show_file_details());
    }

    #[test]
    fn m2_keys_accept_fleet_spellings_and_shapes() {
        let s = Settings::parse(
            r#"{
              "editor.tabSize": 2,
              "editor.indentStyle": "Tabs",
              "editor.lineNumbers": "relative",
              "editor.caretShape": "BLOCK",
              "editor.renderWhitespace": "all",
              "editor.guides": [80, 100],
              "editor.fontFeatures": ["ss01", "zero"],
              "editor.fontWeight": 500,
              "autoSave.delay": 250,
            }"#,
        )
        .unwrap();
        assert_eq!(s.editor_tab_size(), 2);
        assert_eq!(s.editor_indent_style(), IndentStyle::Tabs);
        assert_eq!(s.editor_line_numbers(), LineNumbers::Relative);
        assert_eq!(s.editor_caret_shape(), CaretShape::Block);
        assert!(s.editor_render_whitespace());
        assert_eq!(s.editor_guides(), vec![80, 100]);
        assert_eq!(s.editor_font_features(), vec!["ss01", "zero"]);
        assert_eq!(s.editor_font_weight(), 500.0);
        assert_eq!(s.auto_save_delay_ms(), 250);

        let s = Settings::parse(
            r#"{
              "editor.tabSize": 0,
              "editor.indentStyle": "space",
              "editor.lineNumbers": "Off",
              "editor.caretShape": "bar",
              "editor.renderWhitespace": "none",
              "editor.guides": 120,
              "editor.fontFeatures": { "ss01": true, "cv02": false, "frac": true }
            }"#,
        )
        .unwrap();
        assert_eq!(s.editor_tab_size(), 4, "zero is not a tab size");
        assert_eq!(s.editor_indent_style(), IndentStyle::Spaces);
        assert_eq!(s.editor_line_numbers(), LineNumbers::Off);
        assert_eq!(s.editor_caret_shape(), CaretShape::Line);
        assert!(!s.editor_render_whitespace());
        assert_eq!(s.editor_guides(), vec![120]);
        let mut features = s.editor_font_features();
        features.sort();
        assert_eq!(features, vec!["frac", "ss01"]);

        let s = Settings::parse(
            r#"{
              "editor.caretShape": "underline",
              "editor.renderWhitespace": true,
              "editor.fontFeatures": "ss02",
              "editor.lineNumbers": "nonsense"
            }"#,
        )
        .unwrap();
        assert_eq!(s.editor_caret_shape(), CaretShape::Underline);
        assert!(s.editor_render_whitespace());
        assert_eq!(s.editor_font_features(), vec!["ss02"]);
        assert_eq!(
            s.editor_line_numbers(),
            LineNumbers::On,
            "unknown falls back"
        );
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
